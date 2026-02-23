use rocksdb::{DBWithThreadMode, MultiThreaded, WriteBatch};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MvccError {
    #[error("RocksDB error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    #[error("key not found")]
    NotFound,

    #[error("column family not found: {0}")]
    CfNotFound(String),
}

pub type MvccResult<T> = Result<T, MvccError>;

/// Timestamp oracle: generates monotonically increasing timestamps.
pub struct TimestampOracle {
    counter: AtomicU64,
}

impl TimestampOracle {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    pub fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst)
    }

    pub fn current(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }
}

impl Default for TimestampOracle {
    fn default() -> Self {
        Self::new()
    }
}

/// Encodes a versioned key: original_key ++ !timestamp (inverted for newest-first ordering).
pub fn encode_versioned_key(key: &[u8], timestamp: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(key.len() + 8);
    buf.extend_from_slice(key);
    // Invert timestamp so newest versions sort first in RocksDB
    buf.extend_from_slice(&(!timestamp).to_be_bytes());
    buf
}

/// Decode the timestamp from a versioned key.
pub fn decode_versioned_timestamp(versioned_key: &[u8]) -> Option<u64> {
    if versioned_key.len() < 8 {
        return None;
    }
    let ts_bytes: [u8; 8] = versioned_key[versioned_key.len() - 8..]
        .try_into()
        .ok()?;
    Some(!u64::from_be_bytes(ts_bytes))
}

/// Extract the original key prefix from a versioned key.
pub fn extract_key_prefix(versioned_key: &[u8]) -> &[u8] {
    if versioned_key.len() >= 8 {
        &versioned_key[..versioned_key.len() - 8]
    } else {
        versioned_key
    }
}

/// Tombstone marker for deleted keys.
const TOMBSTONE: &[u8] = b"__TOMBSTONE__";

/// Check if a value is a tombstone (deletion marker).
pub fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

/// MVCC-aware storage operations on top of a RocksDB instance.
pub struct MvccStore {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    ts_oracle: Arc<TimestampOracle>,
}

impl MvccStore {
    pub fn new(
        db: Arc<DBWithThreadMode<MultiThreaded>>,
        ts_oracle: Arc<TimestampOracle>,
    ) -> Self {
        Self { db, ts_oracle }
    }

    pub fn timestamp_oracle(&self) -> &TimestampOracle {
        &self.ts_oracle
    }

    /// Write a versioned key-value pair at the given timestamp.
    pub fn put_at(
        &self,
        cf_name: &str,
        key: &[u8],
        value: &[u8],
        timestamp: u64,
    ) -> MvccResult<()> {
        let cf = self
            .db
            .cf_handle(cf_name)
            .ok_or_else(|| MvccError::CfNotFound(cf_name.to_string()))?;
        let versioned = encode_versioned_key(key, timestamp);
        self.db.put_cf(&cf, &versioned, value)?;
        Ok(())
    }

    /// Delete a key by writing a tombstone at the given timestamp.
    pub fn delete_at(
        &self,
        cf_name: &str,
        key: &[u8],
        timestamp: u64,
    ) -> MvccResult<()> {
        self.put_at(cf_name, key, TOMBSTONE, timestamp)
    }

    /// Read the latest visible version of a key at or before the given snapshot timestamp.
    pub fn get_at(
        &self,
        cf_name: &str,
        key: &[u8],
        snapshot_ts: u64,
    ) -> MvccResult<Option<Vec<u8>>> {
        let cf = self
            .db
            .cf_handle(cf_name)
            .ok_or_else(|| MvccError::CfNotFound(cf_name.to_string()))?;

        // Start scanning from the newest possible version of this key
        let scan_start = encode_versioned_key(key, u64::MAX);
        let iter = self.db.iterator_cf(
            &cf,
            rocksdb::IteratorMode::From(&scan_start, rocksdb::Direction::Forward),
        );

        for item in iter {
            let (k, v) = item?;

            // Check if this key still belongs to our prefix
            let prefix = extract_key_prefix(&k);
            if prefix != key {
                break;
            }

            let ts = decode_versioned_timestamp(&k).unwrap_or(0);
            if ts <= snapshot_ts {
                if is_tombstone(&v) {
                    return Ok(None);
                }
                return Ok(Some(v.to_vec()));
            }
        }

        Ok(None)
    }

    /// Scan all visible key-value pairs with the given prefix at a snapshot.
    pub fn prefix_scan_at(
        &self,
        cf_name: &str,
        prefix: &[u8],
        snapshot_ts: u64,
    ) -> MvccResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let cf = self
            .db
            .cf_handle(cf_name)
            .ok_or_else(|| MvccError::CfNotFound(cf_name.to_string()))?;

        let scan_start = encode_versioned_key(prefix, u64::MAX);
        let iter = self.db.iterator_cf(
            &cf,
            rocksdb::IteratorMode::From(&scan_start, rocksdb::Direction::Forward),
        );

        let mut results = Vec::new();
        let mut last_key: Option<Vec<u8>> = None;

        for item in iter {
            let (k, v) = item?;
            let key_prefix = extract_key_prefix(&k);

            // Stop if we've left our scan prefix
            if !key_prefix.starts_with(prefix) {
                break;
            }

            // Skip duplicate versions of the same key (we only want the latest visible)
            if let Some(ref lk) = last_key {
                if key_prefix == lk.as_slice() {
                    continue;
                }
            }

            let ts = decode_versioned_timestamp(&k).unwrap_or(0);
            if ts <= snapshot_ts {
                last_key = Some(key_prefix.to_vec());
                if !is_tombstone(&v) {
                    results.push((key_prefix.to_vec(), v.to_vec()));
                }
            }
        }

        Ok(results)
    }

    /// Batch write multiple versioned key-value pairs atomically.
    pub fn batch_put_at(
        &self,
        writes: &[(&str, &[u8], &[u8])], // (cf_name, key, value)
        timestamp: u64,
    ) -> MvccResult<()> {
        let mut batch = WriteBatch::default();

        for (cf_name, key, value) in writes {
            let cf = self
                .db
                .cf_handle(cf_name)
                .ok_or_else(|| MvccError::CfNotFound(cf_name.to_string()))?;
            let versioned = encode_versioned_key(key, timestamp);
            batch.put_cf(&cf, &versioned, value);
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// Garbage collect versions older than the given watermark.
    /// Only removes old versions if a newer version exists before the watermark.
    pub fn gc_before(
        &self,
        cf_name: &str,
        prefix: &[u8],
        watermark: u64,
    ) -> MvccResult<usize> {
        let cf = self
            .db
            .cf_handle(cf_name)
            .ok_or_else(|| MvccError::CfNotFound(cf_name.to_string()))?;

        let scan_start = encode_versioned_key(prefix, u64::MAX);
        let iter = self.db.iterator_cf(
            &cf,
            rocksdb::IteratorMode::From(&scan_start, rocksdb::Direction::Forward),
        );

        let mut to_delete = Vec::new();
        let mut last_key: Option<Vec<u8>> = None;
        let mut found_visible = false;

        for item in iter {
            let (k, _v) = item?;
            let key_prefix = extract_key_prefix(&k);

            if !key_prefix.starts_with(prefix) {
                break;
            }

            // New key group
            if last_key.as_deref() != Some(key_prefix) {
                last_key = Some(key_prefix.to_vec());
                found_visible = false;
            }

            let ts = decode_versioned_timestamp(&k).unwrap_or(0);

            if ts <= watermark {
                if found_visible {
                    // This is an older version — safe to GC
                    to_delete.push(k.to_vec());
                } else {
                    found_visible = true;
                }
            }
        }

        let count = to_delete.len();
        let mut batch = WriteBatch::default();
        for key in &to_delete {
            batch.delete_cf(&cf, key);
        }
        self.db.write(batch)?;

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{StorageConfig, StorageEngine};

    fn setup() -> (tempfile::TempDir, MvccStore) {
        let dir = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let ts = Arc::new(TimestampOracle::new());
        let store = MvccStore::new(engine.raw_db().clone(), ts);
        (dir, store)
    }

    #[test]
    fn test_versioned_key_encoding() {
        let key = b"test_key";
        let ts1 = 100u64;
        let ts2 = 200u64;

        let v1 = encode_versioned_key(key, ts1);
        let v2 = encode_versioned_key(key, ts2);

        // Newer timestamp should sort BEFORE older (inverted)
        assert!(v2 < v1);

        assert_eq!(decode_versioned_timestamp(&v1), Some(ts1));
        assert_eq!(decode_versioned_timestamp(&v2), Some(ts2));
        assert_eq!(extract_key_prefix(&v1), key.as_slice());
    }

    #[test]
    fn test_put_and_get_at() {
        let (_dir, store) = setup();
        let cf = "default";
        let key = b"node/123";

        store.put_at(cf, key, b"version1", 10).unwrap();
        store.put_at(cf, key, b"version2", 20).unwrap();
        store.put_at(cf, key, b"version3", 30).unwrap();

        // Read at different snapshots
        assert_eq!(store.get_at(cf, key, 5).unwrap(), None);
        assert_eq!(store.get_at(cf, key, 10).unwrap(), Some(b"version1".to_vec()));
        assert_eq!(store.get_at(cf, key, 15).unwrap(), Some(b"version1".to_vec()));
        assert_eq!(store.get_at(cf, key, 20).unwrap(), Some(b"version2".to_vec()));
        assert_eq!(store.get_at(cf, key, 25).unwrap(), Some(b"version2".to_vec()));
        assert_eq!(store.get_at(cf, key, 30).unwrap(), Some(b"version3".to_vec()));
        assert_eq!(store.get_at(cf, key, 100).unwrap(), Some(b"version3".to_vec()));
    }

    #[test]
    fn test_delete_at() {
        let (_dir, store) = setup();
        let cf = "default";
        let key = b"node/456";

        store.put_at(cf, key, b"value", 10).unwrap();
        store.delete_at(cf, key, 20).unwrap();

        assert_eq!(store.get_at(cf, key, 15).unwrap(), Some(b"value".to_vec()));
        assert_eq!(store.get_at(cf, key, 25).unwrap(), None);

        // Re-insert after delete
        store.put_at(cf, key, b"resurrected", 30).unwrap();
        assert_eq!(store.get_at(cf, key, 35).unwrap(), Some(b"resurrected".to_vec()));
    }

    #[test]
    fn test_prefix_scan_at() {
        let (_dir, store) = setup();
        let cf = "default";

        store.put_at(cf, b"graph/1/node/a", b"a_v1", 10).unwrap();
        store.put_at(cf, b"graph/1/node/a", b"a_v2", 20).unwrap();
        store.put_at(cf, b"graph/1/node/b", b"b_v1", 10).unwrap();
        store.put_at(cf, b"graph/1/node/c", b"c_v1", 15).unwrap();
        store.delete_at(cf, b"graph/1/node/c", 25).unwrap();
        store.put_at(cf, b"graph/2/node/x", b"x_v1", 10).unwrap();

        // Scan at ts=12: should see a_v1, b_v1
        let results = store.prefix_scan_at(cf, b"graph/1/node/", 12).unwrap();
        assert_eq!(results.len(), 2);

        // Scan at ts=22: should see a_v2, b_v1, c_v1
        let results = store.prefix_scan_at(cf, b"graph/1/node/", 22).unwrap();
        assert_eq!(results.len(), 3);

        // Scan at ts=30: should see a_v2, b_v1 (c deleted)
        let results = store.prefix_scan_at(cf, b"graph/1/node/", 30).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_gc_before() {
        let (_dir, store) = setup();
        let cf = "default";

        store.put_at(cf, b"key1", b"v1", 10).unwrap();
        store.put_at(cf, b"key1", b"v2", 20).unwrap();
        store.put_at(cf, b"key1", b"v3", 30).unwrap();

        // GC before ts=25 should remove v1 (ts=10) but keep v2 (ts=20) as latest visible
        let removed = store.gc_before(cf, b"key", 25).unwrap();
        assert_eq!(removed, 1);

        // v2 should still be readable
        assert_eq!(store.get_at(cf, b"key1", 20).unwrap(), Some(b"v2".to_vec()));
        // v3 still there
        assert_eq!(store.get_at(cf, b"key1", 30).unwrap(), Some(b"v3".to_vec()));
    }

    #[test]
    fn test_batch_put_at() {
        let (_dir, store) = setup();
        let cf = "default";

        let writes: Vec<(&str, &[u8], &[u8])> = vec![
            (cf, b"k1", b"val1"),
            (cf, b"k2", b"val2"),
            (cf, b"k3", b"val3"),
        ];
        store.batch_put_at(&writes, 10).unwrap();

        assert_eq!(store.get_at(cf, b"k1", 10).unwrap(), Some(b"val1".to_vec()));
        assert_eq!(store.get_at(cf, b"k2", 10).unwrap(), Some(b"val2".to_vec()));
        assert_eq!(store.get_at(cf, b"k3", 10).unwrap(), Some(b"val3".to_vec()));
    }
}