pub mod conflict;
pub mod mvcc;
pub mod transaction;
pub mod wal;

#[cfg(test)]
mod tests {
    use crate::mvcc::TransactionManager;
    use crate::transaction::TxError;
    use synaptica_storage::engine::{StorageConfig, StorageEngine};
    use synaptica_storage::mvcc::{MvccStore, TimestampOracle};
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, TransactionManager) {
        let dir = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let ts_oracle = Arc::new(TimestampOracle::new());
        let store = Arc::new(MvccStore::new(engine.raw_db().clone(), ts_oracle.clone()));
        let tm = TransactionManager::new(store, ts_oracle);
        (dir, tm)
    }

    #[test]
    fn test_basic_transaction() {
        let (_dir, tm) = setup();
        let mut tx = tm.begin();

        tx.put("default", b"key1", b"value1").unwrap();
        tx.put("default", b"key2", b"value2").unwrap();

        // Read-your-own-writes
        assert_eq!(tx.get("default", b"key1").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(tx.get("default", b"key2").unwrap(), Some(b"value2".to_vec()));

        // Not yet visible to others
        let snap = tm.snapshot();
        assert_eq!(snap.get("default", b"key1").unwrap(), None);

        // Commit
        let commit_ts = tm.commit(&mut tx).unwrap();
        assert!(commit_ts > 0);

        // Now visible
        let snap2 = tm.snapshot();
        assert_eq!(snap2.get("default", b"key1").unwrap(), Some(b"value1".to_vec()));
    }

    #[test]
    fn test_snapshot_isolation() {
        let (_dir, tm) = setup();

        // tx1 writes key1
        let mut tx1 = tm.begin();
        tx1.put("default", b"key1", b"v1").unwrap();
        tm.commit(&mut tx1).unwrap();

        // tx2 starts (sees key1=v1)
        let mut tx2 = tm.begin();
        assert_eq!(tx2.get("default", b"key1").unwrap(), Some(b"v1".to_vec()));

        // tx3 writes key1=v2 and commits
        let mut tx3 = tm.begin();
        tx3.put("default", b"key1", b"v2").unwrap();
        tm.commit(&mut tx3).unwrap();

        // tx2 still sees v1 (snapshot isolation)
        assert_eq!(tx2.get("default", b"key1").unwrap(), Some(b"v1".to_vec()));

        // New snapshot sees v2
        let snap = tm.snapshot();
        assert_eq!(snap.get("default", b"key1").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn test_write_write_conflict() {
        let (_dir, tm) = setup();

        // tx1 writes key1
        let mut tx1 = tm.begin();
        tx1.put("default", b"key1", b"from_tx1").unwrap();

        // tx2 also writes key1
        let mut tx2 = tm.begin();
        tx2.put("default", b"key1", b"from_tx2").unwrap();

        // tx1 commits first
        tm.commit(&mut tx1).unwrap();

        // tx2 should fail with write conflict
        let result = tm.commit(&mut tx2);
        assert!(matches!(result, Err(TxError::WriteConflict)));
    }

    #[test]
    fn test_no_conflict_different_keys() {
        let (_dir, tm) = setup();

        let mut tx1 = tm.begin();
        tx1.put("default", b"key1", b"from_tx1").unwrap();

        let mut tx2 = tm.begin();
        tx2.put("default", b"key2", b"from_tx2").unwrap();

        // Both should commit successfully
        tm.commit(&mut tx1).unwrap();
        tm.commit(&mut tx2).unwrap();

        let snap = tm.snapshot();
        assert_eq!(snap.get("default", b"key1").unwrap(), Some(b"from_tx1".to_vec()));
        assert_eq!(snap.get("default", b"key2").unwrap(), Some(b"from_tx2".to_vec()));
    }

    #[test]
    fn test_rollback() {
        let (_dir, tm) = setup();

        let mut tx = tm.begin();
        tx.put("default", b"key1", b"value1").unwrap();
        tm.rollback(&mut tx).unwrap();

        // Data should not be visible
        let snap = tm.snapshot();
        assert_eq!(snap.get("default", b"key1").unwrap(), None);
    }

    #[test]
    fn test_delete_in_transaction() {
        let (_dir, tm) = setup();

        // Insert
        let mut tx1 = tm.begin();
        tx1.put("default", b"key1", b"value1").unwrap();
        tm.commit(&mut tx1).unwrap();

        // Delete
        let mut tx2 = tm.begin();
        assert_eq!(tx2.get("default", b"key1").unwrap(), Some(b"value1".to_vec()));
        tx2.delete("default", b"key1").unwrap();
        // Read-your-own-deletes
        assert_eq!(tx2.get("default", b"key1").unwrap(), None);
        tm.commit(&mut tx2).unwrap();

        // Verify deleted
        let snap = tm.snapshot();
        assert_eq!(snap.get("default", b"key1").unwrap(), None);
    }
}
