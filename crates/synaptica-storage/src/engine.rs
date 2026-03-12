use crate::cf::ColumnFamilies;
use crate::encoding;
use rocksdb::{
    checkpoint::Checkpoint, BoundColumnFamily, DBWithThreadMode, MultiThreaded, Options, WriteBatch,
};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use synaptica_core::graph::{Edge, GraphId, GraphMeta, Label, Node};
use synaptica_core::types::Value;
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

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("backup error: {0}")]
    Backup(String),
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

    /// Load a TimestampOracle from the persisted counter in this database.
    pub fn load_timestamp_oracle(&self) -> crate::mvcc::TimestampOracle {
        crate::mvcc::TimestampOracle::load_from_db(&self.db)
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

    /// List all graphs by scanning the GRAPH_META column family.
    pub fn list_graphs(&self) -> StorageResult<Vec<GraphMeta>> {
        let cf = self.cf(ColumnFamilies::GRAPH_META)?;
        let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
        let mut graphs = Vec::new();
        for item in iter {
            let (_key, value) = item?;
            if let Ok(meta) = encoding::deserialize_value::<GraphMeta>(&value) {
                graphs.push(meta);
            }
        }
        Ok(graphs)
    }

    /// Delete a graph and all its data across all column families.
    pub fn delete_graph(&self, graph_id: &GraphId) -> StorageResult<()> {
        // Delete all data with graph_id prefix from data column families
        let data_cfs = [
            ColumnFamilies::NODES,
            ColumnFamilies::EDGES,
            ColumnFamilies::ADJ_OUT,
            ColumnFamilies::ADJ_IN,
            ColumnFamilies::NODE_LABELS,
            ColumnFamilies::EDGE_LABELS,
            ColumnFamilies::PROP_INDEX,
        ];
        let prefix = graph_id.as_bytes().to_vec();
        for cf_name in &data_cfs {
            let cf = self.cf(cf_name)?;
            let mut batch = WriteBatch::default();
            let iter = self.db.prefix_iterator_cf(&cf, &prefix);
            for item in iter {
                let (key, _) = item?;
                if key.starts_with(&prefix) {
                    batch.delete_cf(&cf, &key);
                } else {
                    break;
                }
            }
            if batch.len() > 0 {
                self.db.write(batch)?;
            }
        }
        // Delete graph metadata entry
        let meta_cf = self.cf(ColumnFamilies::GRAPH_META)?;
        let meta_key = encoding::encode_graph_meta_key(graph_id);
        self.db.delete_cf(&meta_cf, &meta_key)?;
        Ok(())
    }

    // --- Backup & Export Operations ---

    /// Create a RocksDB checkpoint (point-in-time snapshot) at the given path.
    pub fn create_backup(&self, backup_path: &Path) -> StorageResult<()> {
        let checkpoint = Checkpoint::new(&*self.db)?;
        checkpoint.create_checkpoint(backup_path)?;
        Ok(())
    }

    /// Scan all edges in a graph (unlimited).
    pub fn scan_edges(&self, graph_id: &GraphId) -> StorageResult<Vec<Edge>> {
        let cf = self.cf(ColumnFamilies::EDGES)?;
        let prefix = encoding::encode_edge_prefix(graph_id);
        let iter = self.db.prefix_iterator_cf(&cf, &prefix);

        let mut edges = Vec::new();
        for item in iter {
            let (key, value) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            let edge: Edge =
                encoding::deserialize_value(&value).map_err(StorageError::Deserialization)?;
            edges.push(edge);
        }
        Ok(edges)
    }

    /// Export a graph as GQL INSERT statements to a writer.
    /// Nodes are exported first, then edges. Node IDs are preserved via
    /// a synthetic `_id` property so edges can reference them on import.
    pub fn export_graph<W: Write>(
        &self,
        graph_id: &GraphId,
        writer: &mut W,
    ) -> StorageResult<usize> {
        let mut count = 0;

        // Export nodes with a synthetic _id property for edge matching
        let nodes = self.scan_nodes(graph_id)?;
        for node in &nodes {
            let labels: String = node
                .labels
                .iter()
                .map(|l| format!(":{}", l.0))
                .collect::<String>();
            let mut props = node.properties.clone();
            props.insert("_id".to_string(), Value::String(node.id.0.to_string()));
            let props_str = format_properties_for_export(&props);
            writeln!(writer, "INSERT ({} {})", labels, props_str)?;
            count += 1;
        }

        // Export edges using MATCH on the _id property
        let edges = self.scan_edges(graph_id)?;
        for edge in &edges {
            let props_str = if edge.properties.is_empty() {
                String::new()
            } else {
                format!(" {}", format_properties_for_export(&edge.properties))
            };
            writeln!(
                writer,
                "MATCH (a {{_id: '{src}'}}), (b {{_id: '{tgt}'}}) INSERT (a)-[:{label}{props}]->(b)",
                src = edge.source.0,
                tgt = edge.target.0,
                label = edge.label.0,
                props = props_str,
            )?;
            count += 1;
        }

        Ok(count)
    }

    // --- Node Operations ---

    /// Insert or update a node.
    pub fn put_node(&self, node: &Node) -> StorageResult<()> {
        let mut batch = WriteBatch::default();
        let nodes_cf = self.cf(ColumnFamilies::NODES)?;
        let node_labels_cf = self.cf(ColumnFamilies::NODE_LABELS)?;

        // Clean up old label indexes if node already exists with different labels
        let key = encoding::encode_node_key(&node.graph_id, &node.id);
        if let Some(old_data) = self.db.get_cf(&nodes_cf, &key)? {
            let old_node: Node = encoding::deserialize_value(&old_data)
                .map_err(StorageError::Deserialization)?;
            for old_label in &old_node.labels {
                if !node.labels.contains(old_label) {
                    let label_key = encoding::encode_node_label_key(&node.graph_id, old_label, &node.id);
                    batch.delete_cf(&node_labels_cf, &label_key);
                }
            }
        }

        // Store node data
        let value =
            encoding::serialize_value(node).map_err(StorageError::Serialization)?;
        batch.put_cf(&nodes_cf, &key, &value);

        // Index current labels
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

    /// Delete a node, its label indexes, and all connected edges.
    pub fn delete_node(&self, graph_id: &GraphId, node_id: &synaptica_core::graph::NodeId) -> StorageResult<()> {
        // First get the node to know its labels
        let node = self.get_node(graph_id, node_id)?;

        // Collect connected edges to delete (deduplicate for self-loops)
        let outgoing = self.get_outgoing_edges(graph_id, node_id, None)?;
        let incoming = self.get_incoming_edges(graph_id, node_id, None)?;

        let mut seen = std::collections::HashSet::new();
        for edge in outgoing.iter().chain(incoming.iter()) {
            if seen.insert(edge.id.clone()) {
                self.delete_edge(graph_id, &edge.id)?;
            }
        }

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

    /// Scan nodes with a limit on the number of results returned.
    pub fn scan_nodes_limit(&self, graph_id: &GraphId, limit: usize) -> StorageResult<Vec<Node>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
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
            if nodes.len() >= limit {
                break;
            }
        }
        Ok(nodes)
    }

    /// Scan edges with a limit on the number of results returned.
    pub fn scan_edges_limit(&self, graph_id: &GraphId, limit: usize) -> StorageResult<Vec<Edge>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let cf = self.cf(ColumnFamilies::EDGES)?;
        let prefix = encoding::encode_edge_prefix(graph_id);
        let iter = self.db.prefix_iterator_cf(&cf, &prefix);

        let mut edges = Vec::new();
        for item in iter {
            let (key, value) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            let edge: Edge =
                encoding::deserialize_value(&value).map_err(StorageError::Deserialization)?;
            edges.push(edge);
            if edges.len() >= limit {
                break;
            }
        }
        Ok(edges)
    }

    // --- Edge Operations ---

    /// Insert or update an edge with adjacency indexes.
    pub fn put_edge(&self, edge: &Edge) -> StorageResult<()> {
        let mut batch = WriteBatch::default();
        let edges_cf = self.cf(ColumnFamilies::EDGES)?;
        let adj_out_cf = self.cf(ColumnFamilies::ADJ_OUT)?;
        let adj_in_cf = self.cf(ColumnFamilies::ADJ_IN)?;
        let edge_labels_cf = self.cf(ColumnFamilies::EDGE_LABELS)?;

        // Clean up old indexes if edge already exists with different source/target/label
        let key = encoding::encode_edge_key(&edge.graph_id, &edge.id);
        if let Some(old_data) = self.db.get_cf(&edges_cf, &key)? {
            let old_edge: Edge = encoding::deserialize_value(&old_data)
                .map_err(StorageError::Deserialization)?;
            if old_edge.source != edge.source || old_edge.target != edge.target || old_edge.label != edge.label {
                let old_adj_out = encoding::encode_adj_out_key(&edge.graph_id, &old_edge.source, &old_edge.label, &edge.id);
                batch.delete_cf(&adj_out_cf, &old_adj_out);
                let old_adj_in = encoding::encode_adj_in_key(&edge.graph_id, &old_edge.target, &old_edge.label, &edge.id);
                batch.delete_cf(&adj_in_cf, &old_adj_in);
                let old_label_key = encoding::encode_edge_label_key(&edge.graph_id, &old_edge.label, &edge.id);
                batch.delete_cf(&edge_labels_cf, &old_label_key);
            }
        }

        // Store edge data
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

/// Format a properties map as a GQL property literal `{key: value, ...}`.
fn format_properties_for_export(props: &std::collections::BTreeMap<String, Value>) -> String {
    if props.is_empty() {
        return String::new();
    }
    let pairs: Vec<String> = props
        .iter()
        .map(|(k, v)| format!("{}: {}", k, v))
        .collect();
    format!("{{{}}}", pairs.join(", "))
}

fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
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

    #[test]
    fn test_bulk_node_insert_1000() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        for i in 0..1000 {
            let mut node = Node::new(graph_id);
            node.add_label("Bulk");
            node.set_property("index", Value::Integer(i));
            engine.put_node(&node).unwrap();
        }

        let all = engine.scan_nodes(&graph_id).unwrap();
        assert_eq!(all.len(), 1000);
    }

    #[test]
    fn test_node_update_properties() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let mut node = Node::new(graph_id);
        node.add_label("Person");
        node.set_property("name", Value::String("Alice".into()));
        engine.put_node(&node).unwrap();

        // Update properties
        node.set_property("name", Value::String("Bob".into()));
        node.set_property("age", Value::Integer(25));
        engine.put_node(&node).unwrap();

        let fetched = engine.get_node(&graph_id, &node.id).unwrap();
        assert_eq!(
            fetched.get_property("name"),
            Some(&Value::String("Bob".into()))
        );
        assert_eq!(fetched.get_property("age"), Some(&Value::Integer(25)));
    }

    #[test]
    fn test_node_with_many_properties() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let mut node = Node::new(graph_id);
        node.set_property("str_val", Value::String("hello".into()));
        node.set_property("int_val", Value::Integer(42));
        node.set_property("float_val", Value::Float(3.14));
        node.set_property("bool_true", Value::Bool(true));
        node.set_property("bool_false", Value::Bool(false));
        node.set_property("null_val", Value::Null);
        node.set_property(
            "list_val",
            Value::List(vec![
                Value::Integer(1),
                Value::String("two".into()),
                Value::Bool(false),
            ]),
        );
        let mut map = BTreeMap::new();
        map.insert("nested_key".to_string(), Value::Integer(99));
        node.set_property("map_val", Value::Map(map));
        node.set_property("bytes_val", Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        node.set_property("neg_int", Value::Integer(-1000));
        node.set_property("large_int", Value::Integer(i64::MAX));
        node.set_property("small_int", Value::Integer(i64::MIN));
        node.set_property("zero_float", Value::Float(0.0));
        node.set_property("neg_float", Value::Float(-273.15));
        node.set_property("empty_str", Value::String(String::new()));
        node.set_property("empty_list", Value::List(vec![]));
        node.set_property("empty_map", Value::Map(BTreeMap::new()));
        node.set_property("empty_bytes", Value::Bytes(vec![]));
        node.set_property(
            "nested_list",
            Value::List(vec![Value::List(vec![Value::Integer(1)])]),
        );
        let mut nested_map = BTreeMap::new();
        nested_map.insert("inner".to_string(), Value::Map(BTreeMap::new()));
        node.set_property("nested_map", Value::Map(nested_map));

        engine.put_node(&node).unwrap();
        let fetched = engine.get_node(&graph_id, &node.id).unwrap();
        assert_eq!(fetched.properties.len(), 20);
        assert_eq!(fetched, node);
    }

    #[test]
    fn test_unicode_property_values() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let mut node = Node::new(graph_id);
        node.set_property("japanese", Value::String("日本語".into()));
        node.set_property("emoji", Value::String("émoji 🎉".into()));
        node.set_property("spanish", Value::String("Ñoño".into()));

        engine.put_node(&node).unwrap();
        let fetched = engine.get_node(&graph_id, &node.id).unwrap();
        assert_eq!(
            fetched.get_property("japanese"),
            Some(&Value::String("日本語".into()))
        );
        assert_eq!(
            fetched.get_property("emoji"),
            Some(&Value::String("émoji 🎉".into()))
        );
        assert_eq!(
            fetched.get_property("spanish"),
            Some(&Value::String("Ñoño".into()))
        );
    }

    #[test]
    fn test_null_property_value() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let mut node = Node::new(graph_id);
        node.set_property("empty", Value::Null);
        engine.put_node(&node).unwrap();

        let fetched = engine.get_node(&graph_id, &node.id).unwrap();
        assert_eq!(fetched.get_property("empty"), Some(&Value::Null));
        assert!(fetched.get_property("empty").unwrap().is_null());
    }

    #[test]
    fn test_multiple_graphs_isolation() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();

        let graph1 = GraphId::new();
        let graph2 = GraphId::new();

        for i in 0..5 {
            let mut n = Node::new(graph1);
            n.set_property("idx", Value::Integer(i));
            engine.put_node(&n).unwrap();
        }
        for i in 0..3 {
            let mut n = Node::new(graph2);
            n.set_property("idx", Value::Integer(i));
            engine.put_node(&n).unwrap();
        }

        let g1_nodes = engine.scan_nodes(&graph1).unwrap();
        let g2_nodes = engine.scan_nodes(&graph2).unwrap();
        assert_eq!(g1_nodes.len(), 5);
        assert_eq!(g2_nodes.len(), 3);

        for n in &g1_nodes {
            assert_eq!(n.graph_id, graph1);
        }
        for n in &g2_nodes {
            assert_eq!(n.graph_id, graph2);
        }
    }

    #[test]
    fn test_delete_node_with_edges() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let node_a = Node::new(graph_id);
        let node_b = Node::new(graph_id);
        engine.put_node(&node_a).unwrap();
        engine.put_node(&node_b).unwrap();

        let edge = Edge::new(graph_id, node_a.id, node_b.id, "KNOWS");
        engine.put_edge(&edge).unwrap();

        // Delete source node — connected edges are also cleaned up
        engine.delete_node(&graph_id, &node_a.id).unwrap();
        assert!(engine.get_node(&graph_id, &node_a.id).is_err());

        // Edge should be gone too
        assert!(engine.get_edge(&graph_id, &edge.id).is_err());

        // node_b's incoming edges should be empty
        let incoming = engine.get_incoming_edges(&graph_id, &node_b.id, None).unwrap();
        assert!(incoming.is_empty());
    }

    #[test]
    fn test_scan_empty_graph() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let nodes = engine.scan_nodes(&graph_id).unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_multiple_labels_on_node() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let mut node = Node::new(graph_id);
        node.add_label("Person");
        node.add_label("Employee");
        node.add_label("Manager");
        engine.put_node(&node).unwrap();

        let persons = engine
            .scan_nodes_by_label(&graph_id, &Label::new("Person"))
            .unwrap();
        let employees = engine
            .scan_nodes_by_label(&graph_id, &Label::new("Employee"))
            .unwrap();
        let managers = engine
            .scan_nodes_by_label(&graph_id, &Label::new("Manager"))
            .unwrap();

        assert_eq!(persons.len(), 1);
        assert_eq!(employees.len(), 1);
        assert_eq!(managers.len(), 1);
        assert_eq!(persons[0].id, node.id);
        assert_eq!(employees[0].id, node.id);
        assert_eq!(managers[0].id, node.id);
    }

    #[test]
    fn test_edge_with_properties() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let node_a = Node::new(graph_id);
        let node_b = Node::new(graph_id);
        engine.put_node(&node_a).unwrap();
        engine.put_node(&node_b).unwrap();

        let mut edge = Edge::new(graph_id, node_a.id, node_b.id, "WORKS_WITH");
        edge.set_property("since", Value::Integer(2020));
        edge.set_property("department", Value::String("Engineering".into()));
        edge.set_property("active", Value::Bool(true));
        edge.set_property("score", Value::Float(0.95));
        edge.set_property(
            "tags",
            Value::List(vec![
                Value::String("team".into()),
                Value::String("collab".into()),
            ]),
        );

        engine.put_edge(&edge).unwrap();
        let fetched = engine.get_edge(&graph_id, &edge.id).unwrap();
        assert_eq!(fetched, edge);
        assert_eq!(fetched.get_property("since"), Some(&Value::Integer(2020)));
        assert_eq!(fetched.get_property("active"), Some(&Value::Bool(true)));
        assert_eq!(fetched.get_property("score"), Some(&Value::Float(0.95)));
    }

    #[test]
    fn test_adjacency_scan_with_many_edges() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let hub = Node::new(graph_id);
        engine.put_node(&hub).unwrap();

        for _ in 0..50 {
            let target = Node::new(graph_id);
            engine.put_node(&target).unwrap();
            let edge = Edge::new(graph_id, hub.id, target.id, "CONNECTS");
            engine.put_edge(&edge).unwrap();
        }

        let outgoing = engine
            .get_outgoing_edges(&graph_id, &hub.id, None)
            .unwrap();
        assert_eq!(outgoing.len(), 50);
    }

    #[test]
    fn test_bidirectional_edges() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let node_a = Node::new(graph_id);
        let node_b = Node::new(graph_id);
        engine.put_node(&node_a).unwrap();
        engine.put_node(&node_b).unwrap();

        let edge_ab = Edge::new(graph_id, node_a.id, node_b.id, "FOLLOWS");
        let edge_ba = Edge::new(graph_id, node_b.id, node_a.id, "FOLLOWS");
        engine.put_edge(&edge_ab).unwrap();
        engine.put_edge(&edge_ba).unwrap();

        let a_out = engine
            .get_outgoing_edges(&graph_id, &node_a.id, None)
            .unwrap();
        assert_eq!(a_out.len(), 1);
        assert_eq!(a_out[0].id, edge_ab.id);

        let a_in = engine
            .get_incoming_edges(&graph_id, &node_a.id, None)
            .unwrap();
        assert_eq!(a_in.len(), 1);
        assert_eq!(a_in[0].id, edge_ba.id);

        let b_out = engine
            .get_outgoing_edges(&graph_id, &node_b.id, None)
            .unwrap();
        assert_eq!(b_out.len(), 1);
        assert_eq!(b_out[0].id, edge_ba.id);

        let b_in = engine
            .get_incoming_edges(&graph_id, &node_b.id, None)
            .unwrap();
        assert_eq!(b_in.len(), 1);
        assert_eq!(b_in[0].id, edge_ab.id);
    }

    #[test]
    fn test_put_node_label_update_cleans_old_indexes() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let mut node = Node::new(graph_id);
        node.add_label("Person");
        engine.put_node(&node).unwrap();

        let persons = engine.scan_nodes_by_label(&graph_id, &Label::new("Person")).unwrap();
        assert_eq!(persons.len(), 1);

        // Update labels: remove "Person", add "Employee"
        node.labels.clear();
        node.add_label("Employee");
        engine.put_node(&node).unwrap();

        let persons = engine.scan_nodes_by_label(&graph_id, &Label::new("Person")).unwrap();
        assert!(persons.is_empty(), "stale Person label index should be cleaned up");

        let employees = engine.scan_nodes_by_label(&graph_id, &Label::new("Employee")).unwrap();
        assert_eq!(employees.len(), 1);
        assert_eq!(employees[0].id, node.id);
    }

    #[test]
    fn test_put_edge_source_target_update_cleans_old_indexes() {
        let dir = temp_dir();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let graph_id = GraphId::new();

        let node_a = Node::new(graph_id);
        let node_b = Node::new(graph_id);
        let node_c = Node::new(graph_id);
        engine.put_node(&node_a).unwrap();
        engine.put_node(&node_b).unwrap();
        engine.put_node(&node_c).unwrap();

        // Create edge A → B
        let mut edge = Edge::new(graph_id, node_a.id, node_b.id, "KNOWS");
        engine.put_edge(&edge).unwrap();

        assert_eq!(engine.get_outgoing_edges(&graph_id, &node_a.id, None).unwrap().len(), 1);
        assert_eq!(engine.get_incoming_edges(&graph_id, &node_b.id, None).unwrap().len(), 1);

        // Update edge to A → C
        edge.target = node_c.id;
        engine.put_edge(&edge).unwrap();

        let b_incoming = engine.get_incoming_edges(&graph_id, &node_b.id, None).unwrap();
        assert!(b_incoming.is_empty(), "stale incoming index on B should be cleaned up");

        let c_incoming = engine.get_incoming_edges(&graph_id, &node_c.id, None).unwrap();
        assert_eq!(c_incoming.len(), 1);
        assert_eq!(c_incoming[0].id, edge.id);
    }
}