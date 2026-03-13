//! Simple inverted-index-based full-text search stored in the `PROP_INDEX` column family.
//!
//! Key layout (reuses `PROP_INDEX` CF with a distinct marker byte `0x02`):
//!   `graph_id(16) ++ 0x02 ++ index_name_hash(8) ++ token_bytes ++ 0x00 ++ node_id(16)`

use crate::cf::ColumnFamilies;
use crate::engine::{StorageError, StorageResult};
use rocksdb::{DBWithThreadMode, MultiThreaded, WriteBatch};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use synaptica_core::graph::{GraphId, NodeId};

const FT_MARKER: u8 = 0x02;
const UUID_LEN: usize = 16;
const HASH_LEN: usize = 8;

fn hash_index_name(name: &str) -> [u8; HASH_LEN] {
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    h.finish().to_be_bytes()
}

/// Tokenize text: lowercase, split on non-alphanumeric, drop tokens shorter than 2 chars.
fn tokenize(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(String::from)
        .collect()
}

/// A simple inverted-index full-text search backed by RocksDB.
pub struct FullTextIndex {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    graph_id: GraphId,
    #[allow(dead_code)]
    index_name: String,
    name_hash: [u8; HASH_LEN],
    #[allow(dead_code)]
    property_name: String,
}

impl FullTextIndex {
    pub fn new(
        db: Arc<DBWithThreadMode<MultiThreaded>>,
        graph_id: GraphId,
        index_name: String,
        property_name: String,
    ) -> Self {
        let name_hash = hash_index_name(&index_name);
        Self {
            db,
            graph_id,
            index_name: index_name,
            name_hash,
            property_name,
        }
    }

    fn cf(&self) -> StorageResult<Arc<rocksdb::BoundColumnFamily<'_>>> {
        self.db
            .cf_handle(ColumnFamilies::PROP_INDEX)
            .ok_or_else(|| {
                StorageError::Internal(format!(
                    "column family not found: {}",
                    ColumnFamilies::PROP_INDEX
                ))
            })
    }

    /// Build a key: graph_id ++ FT_MARKER ++ name_hash ++ token ++ 0x00 ++ node_id
    fn entry_key(&self, token: &str, node_id: &NodeId) -> Vec<u8> {
        let tb = token.as_bytes();
        let mut k = Vec::with_capacity(UUID_LEN + 1 + HASH_LEN + tb.len() + 1 + UUID_LEN);
        k.extend_from_slice(self.graph_id.as_bytes());
        k.push(FT_MARKER);
        k.extend_from_slice(&self.name_hash);
        k.extend_from_slice(tb);
        k.push(0x00); // separator
        k.extend_from_slice(node_id.as_bytes());
        k
    }

    /// Prefix for scanning all entries of a given token.
    fn token_prefix(&self, token: &str) -> Vec<u8> {
        let tb = token.as_bytes();
        let mut p = Vec::with_capacity(UUID_LEN + 1 + HASH_LEN + tb.len() + 1);
        p.extend_from_slice(self.graph_id.as_bytes());
        p.push(FT_MARKER);
        p.extend_from_slice(&self.name_hash);
        p.extend_from_slice(tb);
        p.push(0x00); // separator — ensures exact token match
        p
    }

    /// Prefix for scanning all entries whose token starts with `prefix`.
    fn token_scan_prefix(&self, prefix: &str) -> Vec<u8> {
        let pb = prefix.as_bytes();
        let mut p = Vec::with_capacity(UUID_LEN + 1 + HASH_LEN + pb.len());
        p.extend_from_slice(self.graph_id.as_bytes());
        p.push(FT_MARKER);
        p.extend_from_slice(&self.name_hash);
        p.extend_from_slice(pb);
        p
    }

    fn extract_node_id(key: &[u8]) -> StorageResult<NodeId> {
        if key.len() < UUID_LEN {
            return Err(StorageError::Deserialization(
                "fulltext key too short for node id".into(),
            ));
        }
        let id_bytes: [u8; 16] = key[key.len() - UUID_LEN..]
            .try_into()
            .map_err(|_| StorageError::Deserialization("invalid node id in ft index".into()))?;
        Ok(NodeId::from_bytes(id_bytes))
    }

    /// Scan a prefix and collect all matching node IDs.
    fn scan_prefix_ids(&self, prefix: &[u8]) -> StorageResult<Vec<NodeId>> {
        let cf = self.cf()?;
        let iter = self.db.prefix_iterator_cf(&cf, prefix);
        let mut ids = Vec::new();
        for item in iter {
            let (key, _) = item?;
            if !key.starts_with(prefix) {
                break;
            }
            ids.push(Self::extract_node_id(&key)?);
        }
        Ok(ids)
    }

    /// Index the given text for a node. Tokenizes and writes inverted-index entries.
    pub fn index_text(&self, node_id: &NodeId, text: &str) -> StorageResult<()> {
        let cf = self.cf()?;
        let tokens = tokenize(text);
        let mut batch = WriteBatch::default();
        for token in &tokens {
            let key = self.entry_key(token, node_id);
            batch.put_cf(&cf, &key, &[]);
        }
        self.db.write(batch)?;
        Ok(())
    }

    /// Remove inverted-index entries for the given text and node.
    pub fn remove_text(&self, node_id: &NodeId, text: &str) -> StorageResult<()> {
        let cf = self.cf()?;
        let tokens = tokenize(text);
        let mut batch = WriteBatch::default();
        for token in &tokens {
            let key = self.entry_key(token, node_id);
            batch.delete_cf(&cf, &key);
        }
        self.db.write(batch)?;
        Ok(())
    }

    /// Search for nodes containing **all** query terms (AND semantics).
    pub fn search(&self, query: &str) -> StorageResult<Vec<NodeId>> {
        let tokens: Vec<String> = tokenize(query).into_iter().collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        // Intersect posting lists for each token.
        let first_ids: HashSet<NodeId> = self
            .scan_prefix_ids(&self.token_prefix(&tokens[0]))?
            .into_iter()
            .collect();

        let mut result = first_ids;
        for token in &tokens[1..] {
            let ids: HashSet<NodeId> = self
                .scan_prefix_ids(&self.token_prefix(token))?
                .into_iter()
                .collect();
            result = result.intersection(&ids).copied().collect();
            if result.is_empty() {
                break;
            }
        }

        let mut out: Vec<NodeId> = result.into_iter().collect();
        out.sort();
        Ok(out)
    }

    /// Prefix search: find nodes containing any token that starts with `prefix`.
    pub fn search_prefix(&self, prefix: &str) -> StorageResult<Vec<NodeId>> {
        let prefix_lower = prefix.to_lowercase();
        let scan = self.token_scan_prefix(&prefix_lower);
        let mut ids: BTreeSet<NodeId> = BTreeSet::new();
        let cf = self.cf()?;
        let iter = self.db.prefix_iterator_cf(&cf, &scan);
        for item in iter {
            let (key, _) = item?;
            if !key.starts_with(&scan) {
                break;
            }
            ids.insert(Self::extract_node_id(&key)?);
        }
        Ok(ids.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{StorageConfig, StorageEngine};

    fn setup() -> (tempfile::TempDir, Arc<DBWithThreadMode<MultiThreaded>>) {
        let dir = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let db = Arc::clone(engine.raw_db());
        (dir, db)
    }

    #[test]
    fn test_index_and_search_single_term() {
        let (_dir, db) = setup();
        let gid = GraphId::new();
        let ft = FullTextIndex::new(db, gid, "ft_desc".into(), "description".into());

        let n1 = NodeId::new();
        let n2 = NodeId::new();

        ft.index_text(&n1, "The quick brown fox jumps over the lazy dog")
            .unwrap();
        ft.index_text(&n2, "A slow turtle crosses the road")
            .unwrap();

        let results = ft.search("fox").unwrap();
        assert_eq!(results, vec![n1]);

        let results = ft.search("turtle").unwrap();
        assert_eq!(results, vec![n2]);

        // Term not present
        let results = ft.search("elephant").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_multi_term_and_semantics() {
        let (_dir, db) = setup();
        let gid = GraphId::new();
        let ft = FullTextIndex::new(db, gid, "ft_bio".into(), "bio".into());

        let n1 = NodeId::new();
        let n2 = NodeId::new();
        let n3 = NodeId::new();

        ft.index_text(&n1, "Alice likes graph databases and Rust programming")
            .unwrap();
        ft.index_text(&n2, "Bob enjoys graph theory and mathematics")
            .unwrap();
        ft.index_text(&n3, "Charlie writes Rust code for web servers")
            .unwrap();

        // "graph" AND "rust" => only n1
        let mut results = ft.search("graph rust").unwrap();
        results.sort();
        assert_eq!(results, vec![n1]);

        // "graph" alone => n1 and n2
        let mut results = ft.search("graph").unwrap();
        results.sort();
        let mut expected = vec![n1, n2];
        expected.sort();
        assert_eq!(results, expected);

        // "rust" alone => n1 and n3
        let mut results = ft.search("rust").unwrap();
        results.sort();
        let mut expected = vec![n1, n3];
        expected.sort();
        assert_eq!(results, expected);
    }

    #[test]
    fn test_prefix_search() {
        let (_dir, db) = setup();
        let gid = GraphId::new();
        let ft = FullTextIndex::new(db, gid, "ft_title".into(), "title".into());

        let n1 = NodeId::new();
        let n2 = NodeId::new();

        ft.index_text(&n1, "programming in Rust").unwrap();
        ft.index_text(&n2, "professional cooking tips").unwrap();

        // Prefix "pro" should match both (programming, professional)
        let mut results = ft.search_prefix("pro").unwrap();
        results.sort();
        let mut expected = vec![n1, n2];
        expected.sort();
        assert_eq!(results, expected);

        // Prefix "prog" should match only n1 (programming)
        let results = ft.search_prefix("prog").unwrap();
        assert_eq!(results, vec![n1]);
    }

    #[test]
    fn test_remove_text() {
        let (_dir, db) = setup();
        let gid = GraphId::new();
        let ft = FullTextIndex::new(db, gid, "ft_notes".into(), "notes".into());

        let n1 = NodeId::new();
        ft.index_text(&n1, "hello world").unwrap();

        let results = ft.search("hello").unwrap();
        assert_eq!(results, vec![n1]);

        ft.remove_text(&n1, "hello world").unwrap();

        let results = ft.search("hello").unwrap();
        assert!(results.is_empty());
    }
}
