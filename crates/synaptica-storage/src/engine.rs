use crate::cf::ColumnFamilies;
use crate::encoding;
use rocksdb::{
    BoundColumnFamily, DBWithThreadMode, MultiThreaded, Options, WriteBatch,
};
use std::path::Path;
use std::sync::Arc;
use synaptica_core::graph::{Edge, GraphId, GraphMeta, Label, Node};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("RocksDB error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("column family not found: {0}")]
    CfNotFound(String),

    #[error("unique constraint violation: {0}")]
    UniqueViolation(String),
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Configuration for the storage engine.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub max_open_files: i32,
    pub write_buffer_size: usize,
    pub max_write_buffer_number: i32,
    pub target_file_size_base: u64,
    pub max_bytes_for_level_base: u64,
    pub bloom_filter_bits: i32,
    pub block_cache_size: usize,
    pub compression_enabled: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            max_open_files: 10_000,
            write_buffer_size: 64 * 1024 * 1024,      // 64MB
            max_write_buffer_number: 3,
            target_file_size_base: 64 * 1024 * 1024,   // 64MB
            max_bytes_for_level_base: 256 * 1024 * 1024, // 256MB
            bloom_filter_bits: 10,
            block_cache_size: 512 * 1024 * 1024,       // 512MB
            compression_enabled: true,
        }
    }
}

/// The core storage engine backed by RocksDB.
pub struct StorageEngine {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
}

impl StorageEngine {
    /// Open or create a storage engine at the given path.
    pub fn open(path: impl AsRef<Path>, config: &StorageConfig) -> StorageResult<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_max_open_files(config.max_open_files);
        opts.set_write_buffer_size(config.write_buffer_size);
        opts.set_max_write_buffer_number(config.max_write_buffer_number);
        opts.set_target_file_size_base(config.target_file_size_base);
        opts.set_max_bytes_for_level_base(config.max_bytes_for_level_base);
        opts.increase_parallelism(num_cpus());
        opts.set_allow_concurrent_memtable_write(true);
        opts.set_enable_pipelined_write(true);

        if config.compression_enabled {
            opts.set_compression_type(rocksdb::DBCompressionType::Snappy);
        }

        let cf_names = ColumnFamilies::all();
        let db = DBWithThreadMode::<MultiThreaded>::open_cf(&opts, path, cf_names)?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Get a handle to a column family.
    fn cf(&self, name: &str) -> StorageResult<Arc<BoundColumnFamily<'_>>> {
        self.db
            .cf_handle(name)
            .ok_or_else(|| StorageError::CfNotFound(name.to_string()))
    }

    /// Get the underlying RocksDB instance (for advanced usage / transactions).
    pub fn raw_db(&self) -> &Arc<DBWithThreadMode<MultiThreaded>> {
        &self.db
    }

    // --- Graph Metadata ---

    /// Create or update graph metadata.
    pub fn put_graph_meta(&self, meta: &GraphMeta) -> StorageResult<()> {
        let cf = self.cf(ColumnFamilies::GRAPH_META)?;
        let key = encoding::encode_graph_meta_key(&meta.id);
        let value =
            encoding::serialize_value(meta).map_err(StorageError::Serialization)?;
        self.db.put_cf(&cf, &key, &value)?;
        Ok(())
    }

    /// Get graph metadata by ID.
    pub fn get_graph_meta(&self, graph_id: &GraphId) -> StorageResult<GraphMeta> {
        let cf = self.cf(ColumnFamilies::GRAPH_META)?;
        let key = encoding::encode_graph_meta_key(graph_id);
        let value = self
            .db
            .get_cf(&cf, &key)?
            .ok_or_else(|| StorageError::NotFound(format!("graph {}", graph_id.0)))?;
        encoding::deserialize_value(&value).map_err(StorageError::Deserialization)
    }

    // --- Node Operations ---

    /// Insert or update a node.
    pub fn put_node(&self, node: &Node) -> StorageResult<()> {
        let mut batch = WriteBatch::default();
        let nodes_cf = self.cf(ColumnFamilies::NODES)?;
        let node_labels_cf = self.cf(ColumnFamilies::NODE_LABELS)?;

        // Store node data
        let key = encoding::encode_node_key(&node.graph_id, &node.id);
        let value =
            encoding::serialize_value(node).map_err(StorageError::Serialization)?;
        batch.put_cf(&nodes_cf, &key, &value);

        // Index labels
        for label in &node.labels {
            let label_key =
                encoding::encode_node_label_key(&node.graph_id, label, &node.id);
            batch.put_cf(&node_labels_cf, &label_key, &[]);
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// Get a node by ID.
    pub fn get_node(&self, graph_id: &GraphId, node_id: &synaptica_core::graph::NodeId) -> StorageResult<Node> {
        let cf = self.cf(ColumnFamilies::NODES)?;
        let key = encoding::encode_node_key(graph_id, node_id);
        let value = self
            .db
            .get_cf(&cf, &key)?
            .ok_or_else(|| StorageError::NotFound(format!("node {}", node_id.0)))?;
        encoding::deserialize_value(&value).map_err(StorageError::Deserialization)
    }

    /// Delete a node and its label indexes.
    pub fn delete_node(&self, graph_id: &GraphId, node_id: &synaptica_core::graph::NodeId) -> StorageResult<()> {
        // First get the node to know its labels
        let node = self.get_node(graph_id, node_id)?;

        let mut batch = WriteBatch::default();
        let nodes_cf = self.cf(ColumnFamilies::NODES)?;
        let node_labels_cf = self.cf(ColumnFamilies::NODE_LABELS)?;

        // Delete node data
        let key = encoding::encode_node_key(graph_id, node_id);
        batch.delete_cf(&nodes_cf, &key);

        // Delete label indexes
        for label in &node.labels {
            let label_key = encoding::encode_node_label_key(graph_id, label, node_id);
            batch.delete_cf(&node_labels_cf, &label_key);
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// Scan all nodes in a graph.
    pub fn scan_nodes(&self, graph_id: &GraphId) -> StorageResult<Vec<Node>> {
        let cf = self.cf(ColumnFamilies::NODES)?;
        let prefix = encoding::encode_node_prefix(graph_id);
        let iter = self.db.prefix_iterator_cf(&cf, &prefix);

        let mut nodes = Vec::new();
        for item in iter {
            let (key, value) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            let node: Node =
                encoding::deserialize_value(&value).map_err(StorageError::Deserialization)?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// Scan nodes by label.
    pub fn scan_nodes_by_label(
        &self,
        graph_id: &GraphId,
        label: &Label,
    ) -> StorageResult<Vec<Node>> {
        let label_cf = self.cf(ColumnFamilies::NODE_LABELS)?;
        let prefix = encoding::encode_node_label_prefix(graph_id, label);
        let iter = self.db.prefix_iterator_cf(&label_cf, &prefix);

        let mut nodes = Vec::new();
        for item in iter {
            let (key, _) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            // Extract node_id from the end of the key
            let node_id_offset = key.len() - 16;
            let node_id_bytes: [u8; 16] = key[node_id_offset..].try_into().map_err(|_| {
                StorageError::Deserialization("invalid node id in label index".into())
            })?;
            let node_id = synaptica_core::graph::NodeId::from_bytes(node_id_bytes);
            let node = self.get_node(graph_id, &node_id)?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    // --- Edge Operations ---

    /// Insert or update an edge with adjacency indexes.
    pub fn put_edge(&self, edge: &Edge) -> StorageResult<()> {
        let mut batch = WriteBatch::default();
        let edges_cf = self.cf(ColumnFamilies::EDGES)?;
        let adj_out_cf = self.cf(ColumnFamilies::ADJ_OUT)?;
        let adj_in_cf = self.cf(ColumnFamilies::ADJ_IN)?;
        let edge_labels_cf = self.cf(ColumnFamilies::EDGE_LABELS)?;

        // Store edge data
        let key = encoding::encode_edge_key(&edge.graph_id, &edge.id);
        let value =
            encoding::serialize_value(edge).map_err(StorageError::Serialization)?;
        batch.put_cf(&edges_cf, &key, &value);

        // Outgoing adjacency: source -> edge
        let adj_out_key = encoding::encode_adj_out_key(
            &edge.graph_id,
            &edge.source,
            &edge.label,
            &edge.id,
        );
        // Value stores target node id for fast lookup without deserializing the edge
        batch.put_cf(&adj_out_cf, &adj_out_key, edge.target.as_bytes());

        // Incoming adjacency: target -> edge
        let adj_in_key = encoding::encode_adj_in_key(
            &edge.graph_id,
            &edge.target,
            &edge.label,
            &edge.id,
        );
        batch.put_cf(&adj_in_cf, &adj_in_key, edge.source.as_bytes());

        // Edge label index
        let label_key =
            encoding::encode_edge_label_key(&edge.graph_id, &edge.label, &edge.id);
        batch.put_cf(&edge_labels_cf, &label_key, &[]);

        self.db.write(batch)?;
        Ok(())
    }

    /// Get an edge by ID.
    pub fn get_edge(
        &self,
        graph_id: &GraphId,
        edge_id: &synaptica_core::graph::EdgeId,
    ) -> StorageResult<Edge> {
        let cf = self.cf(ColumnFamilies::EDGES)?;
        let key = encoding::encode_edge_key(graph_id, edge_id);
        let value = self
            .db
            .get_cf(&cf, &key)?
            .ok_or_else(|| StorageError::NotFound(format!("edge {}", edge_id.0)))?;
        encoding::deserialize_value(&value).map_err(StorageError::Deserialization)
    }

    /// Delete an edge and its adjacency/label indexes.
    pub fn delete_edge(
        &self,
        graph_id: &GraphId,
        edge_id: &synaptica_core::graph::EdgeId,
    ) -> StorageResult<()> {
        let edge = self.get_edge(graph_id, edge_id)?;

        let mut batch = WriteBatch::default();
        let edges_cf = self.cf(ColumnFamilies::EDGES)?;
        let adj_out_cf = self.cf(ColumnFamilies::ADJ_OUT)?;
        let adj_in_cf = self.cf(ColumnFamilies::ADJ_IN)?;
        let edge_labels_cf = self.cf(ColumnFamilies::EDGE_LABELS)?;

        // Delete edge data
        let key = encoding::encode_edge_key(graph_id, edge_id);
        batch.delete_cf(&edges_cf, &key);

        // Delete adjacency indexes
        let adj_out_key =
            encoding::encode_adj_out_key(graph_id, &edge.source, &edge.label, edge_id);
        batch.delete_cf(&adj_out_cf, &adj_out_key);

        let adj_in_key =
            encoding::encode_adj_in_key(graph_id, &edge.target, &edge.label, edge_id);
        batch.delete_cf(&adj_in_cf, &adj_in_key);

        // Delete label index
        let label_key = encoding::encode_edge_label_key(graph_id, &edge.label, edge_id);
        batch.delete_cf(&edge_labels_cf, &label_key);

        self.db.write(batch)?;
        Ok(())
    }

    /// Get outgoing edges from a node, optionally filtered by label.
    pub fn get_outgoing_edges(
        &self,
        graph_id: &GraphId,
        source: &synaptica_core::graph::NodeId,
        label_filter: Option<&Label>,
    ) -> StorageResult<Vec<Edge>> {
        let adj_cf = self.cf(ColumnFamilies::ADJ_OUT)?;

        let prefix = match label_filter {
            Some(label) => encoding::encode_adj_out_label_prefix(graph_id, source, label),
            None => encoding::encode_adj_out_prefix(graph_id, source),
        };

        let iter = self.db.prefix_iterator_cf(&adj_cf, &prefix);
        let mut edges = Vec::new();

        for item in iter {
            let (key, _value) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            // Extract edge_id from end of key (last 16 bytes)
            let edge_id_offset = key.len() - 16;
            let edge_id_bytes: [u8; 16] =
                key[edge_id_offset..].try_into().map_err(|_| {
                    StorageError::Deserialization("invalid edge id in adj index".into())
                })?;
            let edge_id = synaptica_core::graph::EdgeId::from_bytes(edge_id_bytes);
            let edge = self.get_edge(graph_id, &edge_id)?;
            edges.push(edge);
        }
        Ok(edges)
    }

    /// Get incoming edges to a node, optionally filtered by label.
    pub fn get_incoming_edges(
        &self,
        graph_id: &GraphId,
        target: &synaptica_core::graph::NodeId,
        label_filter: Option<&Label>,
    ) -> StorageResult<Vec<Edge>> {
        let adj_cf = self.cf(ColumnFamilies::ADJ_IN)?;

        let prefix = match label_filter {
            Some(label) => encoding::encode_adj_in_label_prefix(graph_id, target, label),
            None => encoding::encode_adj_in_prefix(graph_id, target),
        };

        let iter = self.db.prefix_iterator_cf(&adj_cf, &prefix);
        let mut edges = Vec::new();

        for item in iter {
            let (key, _value) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            let edge_id_offset = key.len() - 16;
            let edge_id_bytes: [u8; 16] =
                key[edge_id_offset..].try_into().map_err(|_| {
                    StorageError::Deserialization("invalid edge id in adj index".into())
                })?;
            let edge_id = synaptica_core::graph::EdgeId::from_bytes(edge_id_bytes);
            let edge = self.get_edge(graph_id, &edge_id)?;
            edges.push(edge);
        }
        Ok(edges)
    }

    /// Destroy the database at the given path.
    pub fn destroy(path: impl AsRef<Path>) -> StorageResult<()> {
        let opts = Options::default();
        DBWithThreadMode::<MultiThreaded>::destroy(&opts, path)?;
        Ok(())
    }
}

fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use synaptica_core::graph::{GraphMeta, Node};
    use synaptica_core::types::Value;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_open_and_close() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default());
        assert!(engine.is_ok());
    }

    #[test]
    fn test_node_crud() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();

        let graph_id = GraphId::new();
        let mut node = Node::new(graph_id);
        node.add_label("Person");
        node.set_property("name", Value::String("Alice".into()));
        node.set_property("age", Value::Integer(30));

        // Create
        engine.put_node(&node).unwrap();

        // Read
        let fetched = engine.get_node(&graph_id, &node.id).unwrap();
        assert_eq!(fetched, node);

        // Delete
        engine.delete_node(&graph_id, &node.id).unwrap();
        assert!(engine.get_node(&graph_id, &node.id).is_err());
    }

    #[test]
    fn test_edge_crud() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();

        let graph_id = GraphId::new();
        let node_a = Node::new(graph_id);
        let node_b = Node::new(graph_id);
        engine.put_node(&node_a).unwrap();
        engine.put_node(&node_b).unwrap();

        let mut edge = Edge::new(graph_id, node_a.id, node_b.id, "KNOWS");
        edge.set_property("since", Value::Integer(2020));

        // Create
        engine.put_edge(&edge).unwrap();

        // Read
        let fetched = engine.get_edge(&graph_id, &edge.id).unwrap();
        assert_eq!(fetched, edge);

        // Adjacency
        let outgoing = engine.get_outgoing_edges(&graph_id, &node_a.id, None).unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].id, edge.id);

        let incoming = engine.get_incoming_edges(&graph_id, &node_b.id, None).unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].id, edge.id);

        // Delete
        engine.delete_edge(&graph_id, &edge.id).unwrap();
        assert!(engine.get_edge(&graph_id, &edge.id).is_err());

        let outgoing = engine.get_outgoing_edges(&graph_id, &node_a.id, None).unwrap();
        assert_eq!(outgoing.len(), 0);
    }

    #[test]
    fn test_scan_nodes_by_label() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();

        let graph_id = GraphId::new();
        let mut n1 = Node::new(graph_id);
        n1.add_label("Person");
        let mut n2 = Node::new(graph_id);
        n2.add_label("Person");
        let mut n3 = Node::new(graph_id);
        n3.add_label("Company");

        engine.put_node(&n1).unwrap();
        engine.put_node(&n2).unwrap();
        engine.put_node(&n3).unwrap();

        let persons = engine
            .scan_nodes_by_label(&graph_id, &Label::new("Person"))
            .unwrap();
        assert_eq!(persons.len(), 2);

        let companies = engine
            .scan_nodes_by_label(&graph_id, &Label::new("Company"))
            .unwrap();
        assert_eq!(companies.len(), 1);
    }

    #[test]
    fn test_graph_meta() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();

        let meta = GraphMeta {
            id: GraphId::new(),
            name: "social".to_string(),
            graph_type: Some("SocialGraph".to_string()),
        };
        engine.put_graph_meta(&meta).unwrap();

        let fetched = engine.get_graph_meta(&meta.id).unwrap();
        assert_eq!(fetched.name, "social");
    }

    #[test]
    fn test_label_filtered_adjacency() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();

        let graph_id = GraphId::new();
        let node_a = Node::new(graph_id);
        let node_b = Node::new(graph_id);
        let node_c = Node::new(graph_id);
        engine.put_node(&node_a).unwrap();
        engine.put_node(&node_b).unwrap();
        engine.put_node(&node_c).unwrap();

        let edge1 = Edge::new(graph_id, node_a.id, node_b.id, "KNOWS");
        let edge2 = Edge::new(graph_id, node_a.id, node_c.id, "WORKS_AT");
        engine.put_edge(&edge1).unwrap();
        engine.put_edge(&edge2).unwrap();

        // Filter by label
        let knows = engine
            .get_outgoing_edges(&graph_id, &node_a.id, Some(&Label::new("KNOWS")))
            .unwrap();
        assert_eq!(knows.len(), 1);

        let works = engine
            .get_outgoing_edges(&graph_id, &node_a.id, Some(&Label::new("WORKS_AT")))
            .unwrap();
        assert_eq!(works.len(), 1);

        // No filter
        let all = engine.get_outgoing_edges(&graph_id, &node_a.id, None).unwrap();
        assert_eq!(all.len(), 2);
    }
}