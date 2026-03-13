//! Secondary property index support for the Synaptica graph database.
//!
//! Key layouts inside the `PROP_INDEX` column family:
//!   - Metadata: `graph_id ++ 0x00 ++ index_name_bytes`
//!   - Entry:    `graph_id ++ 0x01 ++ index_name_hash(8) ++ encoded_value(s) ++ entity_id(16)`

use crate::cf::ColumnFamilies;
use crate::engine::{StorageError, StorageResult};
use parking_lot::Mutex;
use rocksdb::{DBWithThreadMode, Direction, IteratorMode, MultiThreaded};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use synaptica_core::graph::{GraphId, Node, NodeId};
use synaptica_core::types::Value;

/// Whether the index covers nodes or edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexEntityType {
    Node,
    Edge,
}

/// Defines a secondary property index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexDefinition {
    pub name: String,
    pub graph_id: GraphId,
    pub entity_type: IndexEntityType,
    pub property_names: Vec<String>,
    pub unique: bool,
}

// ---------- internal constants ----------

const META_MARKER: u8 = 0x00;
const ENTRY_MARKER: u8 = 0x01;
const UUID_LEN: usize = 16;
const HASH_LEN: usize = 8;

fn hash_index_name(name: &str) -> [u8; HASH_LEN] {
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    h.finish().to_be_bytes()
}

/// Encode a [`Value`] into a byte-comparable binary representation.
///
/// Type tags guarantee cross-type ordering never collides.
/// Integers use sign-bit-flipped big-endian so that negative < positive.
/// Floats use IEEE-754 comparable encoding.
/// Strings use escaped encoding: 0x00 → [0x00, 0x01], end → [0x00, 0x00].
fn encode_value_comparable(value: &Value, buf: &mut Vec<u8>) {
    match value {
        Value::Null => buf.push(0x00),
        Value::Bool(b) => {
            buf.push(0x01);
            buf.push(u8::from(*b));
        }
        Value::Integer(i) => {
            buf.push(0x02);
            let bytes = i.to_be_bytes();
            buf.push(bytes[0] ^ 0x80);
            buf.extend_from_slice(&bytes[1..]);
        }
        Value::Float(f) => {
            buf.push(0x03);
            let bits = f.to_bits();
            let encoded = if *f >= 0.0 {
                bits ^ (1u64 << 63)
            } else {
                !bits
            };
            buf.extend_from_slice(&encoded.to_be_bytes());
        }
        Value::String(s) => {
            buf.push(0x04);
            // Escape NUL bytes: 0x00 → [0x00, 0x01]; end-of-string → [0x00, 0x00]
            for &byte in s.as_bytes() {
                if byte == 0x00 {
                    buf.push(0x00);
                    buf.push(0x01);
                } else {
                    buf.push(byte);
                }
            }
            buf.push(0x00);
            buf.push(0x00);
        }
        other => {
            buf.push(0xFF);
            // Use length-prefixed encoding; serialization failure → empty (logged)
            match bincode::serialize(other) {
                Ok(serialized) => {
                    buf.extend_from_slice(&(serialized.len() as u32).to_be_bytes());
                    buf.extend_from_slice(&serialized);
                }
                Err(e) => {
                    tracing::error!(error = %e, "index value serialization failed");
                    buf.extend_from_slice(&0u32.to_be_bytes());
                }
            }
        }
    }
}

// ---------- IndexManager ----------

/// Manages secondary property indexes stored in the `PROP_INDEX` column family.
pub struct IndexManager {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    unique_lock: Mutex<()>,
}

impl IndexManager {
    pub fn new(db: Arc<DBWithThreadMode<MultiThreaded>>) -> Self {
        Self {
            db,
            unique_lock: Mutex::new(()),
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

    // ---- key builders ----

    fn meta_key(graph_id: &GraphId, name: &str) -> Vec<u8> {
        let nb = name.as_bytes();
        let mut k = Vec::with_capacity(UUID_LEN + 1 + nb.len());
        k.extend_from_slice(graph_id.as_bytes());
        k.push(META_MARKER);
        k.extend_from_slice(nb);
        k
    }

    fn meta_prefix(graph_id: &GraphId) -> Vec<u8> {
        let mut p = Vec::with_capacity(UUID_LEN + 1);
        p.extend_from_slice(graph_id.as_bytes());
        p.push(META_MARKER);
        p
    }

    fn entry_key(def: &IndexDefinition, encoded_vals: &[u8], entity_id: &[u8; 16]) -> Vec<u8> {
        let nh = hash_index_name(&def.name);
        let mut k = Vec::with_capacity(UUID_LEN + 1 + HASH_LEN + encoded_vals.len() + UUID_LEN);
        k.extend_from_slice(def.graph_id.as_bytes());
        k.push(ENTRY_MARKER);
        k.extend_from_slice(&nh);
        k.extend_from_slice(encoded_vals);
        k.extend_from_slice(entity_id);
        k
    }

    fn entry_prefix(def: &IndexDefinition) -> Vec<u8> {
        let nh = hash_index_name(&def.name);
        let mut p = Vec::with_capacity(UUID_LEN + 1 + HASH_LEN);
        p.extend_from_slice(def.graph_id.as_bytes());
        p.push(ENTRY_MARKER);
        p.extend_from_slice(&nh);
        p
    }

    fn entry_value_prefix(def: &IndexDefinition, encoded_vals: &[u8]) -> Vec<u8> {
        let nh = hash_index_name(&def.name);
        let mut p = Vec::with_capacity(UUID_LEN + 1 + HASH_LEN + encoded_vals.len());
        p.extend_from_slice(def.graph_id.as_bytes());
        p.push(ENTRY_MARKER);
        p.extend_from_slice(&nh);
        p.extend_from_slice(encoded_vals);
        p
    }

    // ---- value encoding helpers ----

    fn encode_properties(def: &IndexDefinition, node: &Node) -> Vec<u8> {
        let mut buf = Vec::new();
        for name in &def.property_names {
            match node.get_property(name) {
                Some(v) => encode_value_comparable(v, &mut buf),
                None => encode_value_comparable(&Value::Null, &mut buf),
            }
        }
        buf
    }

    fn encode_values(values: &[Value]) -> Vec<u8> {
        let mut buf = Vec::new();
        for v in values {
            encode_value_comparable(v, &mut buf);
        }
        buf
    }

    // ---- public API ----

    /// Store index metadata in the PROP_INDEX column family.
    pub fn create_index(&self, def: &IndexDefinition) -> StorageResult<()> {
        let cf = self.cf()?;
        let key = Self::meta_key(&def.graph_id, &def.name);
        let value = crate::encoding::serialize_value(def).map_err(StorageError::Serialization)?;
        self.db.put_cf(&cf, &key, &value)?;
        Ok(())
    }

    /// Remove index metadata and all associated entries.
    pub fn drop_index(&self, graph_id: &GraphId, name: &str) -> StorageResult<()> {
        let cf = self.cf()?;
        self.db.delete_cf(&cf, &Self::meta_key(graph_id, name))?;

        let temp = IndexDefinition {
            name: name.to_string(),
            graph_id: *graph_id,
            entity_type: IndexEntityType::Node,
            property_names: vec![],
            unique: false,
        };
        let prefix = Self::entry_prefix(&temp);
        let mut batch = rocksdb::WriteBatch::default();
        for item in self.db.prefix_iterator_cf(&cf, &prefix) {
            let (key, _) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            batch.delete_cf(&cf, &key);
        }
        self.db.write(batch)?;
        Ok(())
    }

    /// Create an index entry for a node, enforcing uniqueness when required.
    pub fn index_node(&self, def: &IndexDefinition, node: &Node) -> StorageResult<()> {
        let cf = self.cf()?;
        let encoded = Self::encode_properties(def, node);

        if def.unique {
            let _guard = self.unique_lock.lock();
            let vp = Self::entry_value_prefix(def, &encoded);
            for item in self.db.prefix_iterator_cf(&cf, &vp) {
                let (key, _) = item?;
                if !key.starts_with(&vp) {
                    break;
                }
                if key.len() >= UUID_LEN {
                    let id_bytes: [u8; 16] = key[key.len() - UUID_LEN..]
                        .try_into()
                        .map_err(|_| StorageError::Deserialization("invalid entity id".into()))?;
                    if NodeId::from_bytes(id_bytes) != node.id {
                        return Err(StorageError::UniqueViolation(format!(
                            "index '{}': duplicate value for node {}",
                            def.name, node.id
                        )));
                    }
                }
            }
            let key = Self::entry_key(def, &encoded, node.id.as_bytes());
            self.db.put_cf(&cf, &key, &[])?;
            return Ok(());
        }

        let key = Self::entry_key(def, &encoded, node.id.as_bytes());
        self.db.put_cf(&cf, &key, &[])?;
        Ok(())
    }

    /// Remove an index entry for a node.
    pub fn unindex_node(&self, def: &IndexDefinition, node: &Node) -> StorageResult<()> {
        let cf = self.cf()?;
        let encoded = Self::encode_properties(def, node);
        let key = Self::entry_key(def, &encoded, node.id.as_bytes());
        self.db.delete_cf(&cf, &key)?;
        Ok(())
    }

    /// Exact-match lookup returning all matching node IDs.
    pub fn lookup(&self, def: &IndexDefinition, values: &[Value]) -> StorageResult<Vec<NodeId>> {
        let cf = self.cf()?;
        let prefix = Self::entry_value_prefix(def, &Self::encode_values(values));
        let mut out = Vec::new();
        for item in self.db.prefix_iterator_cf(&cf, &prefix) {
            let (key, _) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            if key.len() >= UUID_LEN {
                let id: [u8; 16] = key[key.len() - UUID_LEN..]
                    .try_into()
                    .map_err(|_| StorageError::Deserialization("invalid node id".into()))?;
                out.push(NodeId::from_bytes(id));
            }
        }
        Ok(out)
    }

    /// Range scan over a single property: returns node IDs where
    /// `start <= value < end` (inclusive start, exclusive end).
    pub fn range_scan(
        &self,
        def: &IndexDefinition,
        start: &Value,
        end: &Value,
    ) -> StorageResult<Vec<NodeId>> {
        let cf = self.cf()?;
        let idx_prefix = Self::entry_prefix(def);

        let mut start_key = idx_prefix.clone();
        start_key.extend_from_slice(&Self::encode_values(&[start.clone()]));

        let mut end_key = idx_prefix.clone();
        end_key.extend_from_slice(&Self::encode_values(&[end.clone()]));

        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&start_key, Direction::Forward));

        let mut out = Vec::new();
        for item in iter {
            let (key, _) = item?;
            if !key.starts_with(&idx_prefix) || key[..] >= *end_key {
                break;
            }
            if key.len() >= UUID_LEN {
                let id: [u8; 16] = key[key.len() - UUID_LEN..]
                    .try_into()
                    .map_err(|_| StorageError::Deserialization("invalid node id".into()))?;
                out.push(NodeId::from_bytes(id));
            }
        }
        Ok(out)
    }

    /// List all index definitions stored for a graph.
    pub fn list_indexes(&self, graph_id: &GraphId) -> StorageResult<Vec<IndexDefinition>> {
        let cf = self.cf()?;
        let prefix = Self::meta_prefix(graph_id);
        let mut out = Vec::new();
        for item in self.db.prefix_iterator_cf(&cf, &prefix) {
            let (key, value) = item?;
            if !key.starts_with(&prefix) {
                break;
            }
            let def: IndexDefinition = crate::encoding::deserialize_value(&value)
                .map_err(StorageError::Deserialization)?;
            out.push(def);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{StorageConfig, StorageEngine};

    fn setup() -> (tempfile::TempDir, IndexManager) {
        let dir = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let mgr = IndexManager::new(Arc::clone(engine.raw_db()));
        (dir, mgr)
    }

    #[test]
    fn test_single_property_lookup() {
        let (_dir, mgr) = setup();
        let gid = GraphId::new();

        let def = IndexDefinition {
            name: "idx_name".into(),
            graph_id: gid,
            entity_type: IndexEntityType::Node,
            property_names: vec!["name".into()],
            unique: false,
        };
        mgr.create_index(&def).unwrap();

        let mut n1 = Node::new(gid);
        n1.set_property("name", Value::String("Alice".into()));
        let mut n2 = Node::new(gid);
        n2.set_property("name", Value::String("Bob".into()));
        let mut n3 = Node::new(gid);
        n3.set_property("name", Value::String("Alice".into()));

        mgr.index_node(&def, &n1).unwrap();
        mgr.index_node(&def, &n2).unwrap();
        mgr.index_node(&def, &n3).unwrap();

        let mut found = mgr.lookup(&def, &[Value::String("Alice".into())]).unwrap();
        found.sort();
        let mut expected = vec![n1.id, n3.id];
        expected.sort();
        assert_eq!(found, expected);

        let found = mgr.lookup(&def, &[Value::String("Bob".into())]).unwrap();
        assert_eq!(found, vec![n2.id]);

        let found = mgr
            .lookup(&def, &[Value::String("Charlie".into())])
            .unwrap();
        assert!(found.is_empty());

        let indexes = mgr.list_indexes(&gid).unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].name, "idx_name");
    }

    #[test]
    fn test_composite_index() {
        let (_dir, mgr) = setup();
        let gid = GraphId::new();

        let def = IndexDefinition {
            name: "idx_first_last".into(),
            graph_id: gid,
            entity_type: IndexEntityType::Node,
            property_names: vec!["first".into(), "last".into()],
            unique: false,
        };
        mgr.create_index(&def).unwrap();

        let mut n1 = Node::new(gid);
        n1.set_property("first", Value::String("Alice".into()));
        n1.set_property("last", Value::String("Smith".into()));

        let mut n2 = Node::new(gid);
        n2.set_property("first", Value::String("Alice".into()));
        n2.set_property("last", Value::String("Jones".into()));

        let mut n3 = Node::new(gid);
        n3.set_property("first", Value::String("Bob".into()));
        n3.set_property("last", Value::String("Smith".into()));

        mgr.index_node(&def, &n1).unwrap();
        mgr.index_node(&def, &n2).unwrap();
        mgr.index_node(&def, &n3).unwrap();

        let found = mgr
            .lookup(
                &def,
                &[Value::String("Alice".into()), Value::String("Smith".into())],
            )
            .unwrap();
        assert_eq!(found, vec![n1.id]);

        let found = mgr
            .lookup(
                &def,
                &[Value::String("Alice".into()), Value::String("Jones".into())],
            )
            .unwrap();
        assert_eq!(found, vec![n2.id]);

        let found = mgr
            .lookup(
                &def,
                &[Value::String("Bob".into()), Value::String("Jones".into())],
            )
            .unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn test_range_scan_integer() {
        let (_dir, mgr) = setup();
        let gid = GraphId::new();

        let def = IndexDefinition {
            name: "idx_age".into(),
            graph_id: gid,
            entity_type: IndexEntityType::Node,
            property_names: vec!["age".into()],
            unique: false,
        };
        mgr.create_index(&def).unwrap();

        let ages = [10i64, 20, 25, 30, 40, 50];
        let mut nodes = Vec::new();
        for &age in &ages {
            let mut n = Node::new(gid);
            n.set_property("age", Value::Integer(age));
            mgr.index_node(&def, &n).unwrap();
            nodes.push(n);
        }

        // Range [20, 40) → ages 20, 25, 30
        let found = mgr
            .range_scan(&def, &Value::Integer(20), &Value::Integer(40))
            .unwrap();
        assert_eq!(found.len(), 3);

        let mut found_ids: Vec<_> = found.into_iter().collect();
        found_ids.sort();
        let mut expected: Vec<_> = nodes[1..4].iter().map(|n| n.id).collect();
        expected.sort();
        assert_eq!(found_ids, expected);
    }

    #[test]
    fn test_unique_constraint_violation() {
        let (_dir, mgr) = setup();
        let gid = GraphId::new();

        let def = IndexDefinition {
            name: "idx_email_uniq".into(),
            graph_id: gid,
            entity_type: IndexEntityType::Node,
            property_names: vec!["email".into()],
            unique: true,
        };
        mgr.create_index(&def).unwrap();

        let mut n1 = Node::new(gid);
        n1.set_property("email", Value::String("alice@example.com".into()));
        mgr.index_node(&def, &n1).unwrap();

        // Re-indexing the same node is allowed
        mgr.index_node(&def, &n1).unwrap();

        // A different node with the same value must fail
        let mut n2 = Node::new(gid);
        n2.set_property("email", Value::String("alice@example.com".into()));
        let result = mgr.index_node(&def, &n2);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StorageError::UniqueViolation(_)
        ));

        // A different value succeeds
        let mut n3 = Node::new(gid);
        n3.set_property("email", Value::String("bob@example.com".into()));
        mgr.index_node(&def, &n3).unwrap();
    }
}
