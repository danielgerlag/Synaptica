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

    #[test]
    fn test_multiple_writes_single_tx() {
        let (_dir, tm) = setup();
        let mut tx = tm.begin();

        for i in 0..10 {
            let key = format!("key{}", i);
            let value = format!("value{}", i);
            tx.put("default", key.as_bytes(), value.as_bytes()).unwrap();
        }

        tm.commit(&mut tx).unwrap();

        let snap = tm.snapshot();
        for i in 0..10 {
            let key = format!("key{}", i);
            let expected = format!("value{}", i);
            assert_eq!(
                snap.get("default", key.as_bytes()).unwrap(),
                Some(expected.into_bytes())
            );
        }
    }

    #[test]
    fn test_read_after_write_in_tx() {
        let (_dir, tm) = setup();
        let mut tx = tm.begin();

        tx.put("default", b"key1", b"val1").unwrap();
        assert_eq!(tx.get("default", b"key1").unwrap(), Some(b"val1".to_vec()));

        tx.put("default", b"key2", b"val2").unwrap();
        assert_eq!(tx.get("default", b"key1").unwrap(), Some(b"val1".to_vec()));
        assert_eq!(tx.get("default", b"key2").unwrap(), Some(b"val2".to_vec()));
    }

    #[test]
    fn test_concurrent_readers() {
        let (_dir, tm) = setup();

        let mut tx1 = tm.begin();
        tx1.put("default", b"shared", b"data").unwrap();
        tm.commit(&mut tx1).unwrap();

        let snap2 = tm.snapshot();
        let snap3 = tm.snapshot();

        assert_eq!(
            snap2.get("default", b"shared").unwrap(),
            Some(b"data".to_vec())
        );
        assert_eq!(
            snap3.get("default", b"shared").unwrap(),
            Some(b"data".to_vec())
        );
    }

    #[test]
    fn test_sequential_transactions() {
        let (_dir, tm) = setup();

        let mut tx1 = tm.begin();
        tx1.put("default", b"key", b"v1").unwrap();
        tm.commit(&mut tx1).unwrap();

        let mut tx2 = tm.begin();
        assert_eq!(tx2.get("default", b"key").unwrap(), Some(b"v1".to_vec()));
        tx2.put("default", b"key", b"v2").unwrap();
        tm.commit(&mut tx2).unwrap();

        let mut tx3 = tm.begin();
        assert_eq!(tx3.get("default", b"key").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn test_rollback_does_not_affect_subsequent_tx() {
        let (_dir, tm) = setup();

        let mut tx1 = tm.begin();
        tx1.put("default", b"key", b"rolled_back").unwrap();
        tm.rollback(&mut tx1).unwrap();

        let mut tx2 = tm.begin();
        tx2.put("default", b"key", b"committed").unwrap();
        tm.commit(&mut tx2).unwrap();

        let mut tx3 = tm.begin();
        assert_eq!(
            tx3.get("default", b"key").unwrap(),
            Some(b"committed".to_vec())
        );
    }

    #[test]
    fn test_double_commit_error() {
        let (_dir, tm) = setup();

        let mut tx = tm.begin();
        tx.put("default", b"key", b"value").unwrap();
        tm.commit(&mut tx).unwrap();

        let result = tm.commit(&mut tx);
        assert!(matches!(result, Err(TxError::AlreadyCommitted)));
    }

    #[test]
    fn test_operations_on_committed_tx_error() {
        let (_dir, tm) = setup();

        let mut tx = tm.begin();
        tx.put("default", b"key", b"value").unwrap();
        tm.commit(&mut tx).unwrap();

        assert!(matches!(
            tx.put("default", b"key2", b"val"),
            Err(TxError::NotActive)
        ));
        assert!(matches!(
            tx.get("default", b"key"),
            Err(TxError::NotActive)
        ));
    }

    #[test]
    fn test_gc_cleans_old_writes() {
        let dir = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let ts_oracle = Arc::new(TimestampOracle::new());
        let store = Arc::new(MvccStore::new(engine.raw_db().clone(), ts_oracle.clone()));
        let tm = TransactionManager::new(store.clone(), ts_oracle);

        let mut tx1 = tm.begin();
        tx1.put("default", b"key", b"v1").unwrap();
        tm.commit(&mut tx1).unwrap();

        let mut tx2 = tm.begin();
        tx2.put("default", b"key", b"v2").unwrap();
        let ts2 = tm.commit(&mut tx2).unwrap();

        let mut tx3 = tm.begin();
        tx3.put("default", b"key", b"v3").unwrap();
        tm.commit(&mut tx3).unwrap();

        // GC versions at or below ts2 — should remove v1 but keep v2 as latest at watermark
        let removed = store.gc_before("default", b"", ts2).unwrap();
        assert!(removed >= 1);

        // Latest version still readable
        let snap = tm.snapshot();
        assert_eq!(
            snap.get("default", b"key").unwrap(),
            Some(b"v3".to_vec())
        );
    }

    #[test]
    fn test_large_value_transaction() {
        let (_dir, tm) = setup();

        let large_value = vec![0xABu8; 1024 * 1024]; // 1MB
        let mut tx = tm.begin();
        tx.put("default", b"big_key", &large_value).unwrap();
        tm.commit(&mut tx).unwrap();

        let snap = tm.snapshot();
        let result = snap.get("default", b"big_key").unwrap().unwrap();
        assert_eq!(result.len(), 1024 * 1024);
        assert_eq!(result, large_value);
    }

    #[test]
    fn test_many_concurrent_non_conflicting_txs() {
        let (_dir, tm) = setup();

        let mut txs: Vec<_> = (0..10).map(|_| tm.begin()).collect();

        for (i, tx) in txs.iter_mut().enumerate() {
            let key = format!("key_{}", i);
            let val = format!("val_{}", i);
            tx.put("default", key.as_bytes(), val.as_bytes()).unwrap();
        }

        for tx in txs.iter_mut() {
            tm.commit(tx).unwrap();
        }

        let snap = tm.snapshot();
        for i in 0..10 {
            let key = format!("key_{}", i);
            let expected = format!("val_{}", i);
            assert_eq!(
                snap.get("default", key.as_bytes()).unwrap(),
                Some(expected.into_bytes())
            );
        }
    }

    #[test]
    fn test_concurrent_commit_conflict_detected() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let engine = StorageEngine::open(dir.path(), &StorageConfig::default()).unwrap();
        let ts_oracle = Arc::new(TimestampOracle::new());
        let store = Arc::new(MvccStore::new(engine.raw_db().clone(), ts_oracle.clone()));
        let tm = Arc::new(TransactionManager::new(store, ts_oracle));

        let mut tx1 = tm.begin();
        tx1.put("default", b"shared_key", b"from_tx1").unwrap();

        let mut tx2 = tm.begin();
        tx2.put("default", b"shared_key", b"from_tx2").unwrap();

        let tm1 = tm.clone();
        let h1 = std::thread::spawn(move || tm1.commit(&mut tx1));

        let tm2 = tm.clone();
        let h2 = std::thread::spawn(move || tm2.commit(&mut tx2));

        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();

        // Exactly one must succeed and one must get WriteConflict
        let (successes, conflicts): (Vec<_>, Vec<_>) =
            [r1, r2].into_iter().partition(|r| r.is_ok());
        assert_eq!(successes.len(), 1);
        assert_eq!(conflicts.len(), 1);
        assert!(matches!(
            conflicts[0],
            Err(TxError::WriteConflict)
        ));
    }

    #[test]
    fn test_atomic_commit_all_or_nothing() {
        let (_dir, tm) = setup();
        let mut tx = tm.begin();

        tx.put("default", b"atom_k1", b"v1").unwrap();
        tx.put("default", b"atom_k2", b"v2").unwrap();
        tx.put("default", b"atom_k3", b"v3").unwrap();

        let commit_ts = tm.commit(&mut tx).unwrap();

        // All three keys must be readable at the commit timestamp
        let snap = tm.snapshot();
        assert_eq!(snap.get("default", b"atom_k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(snap.get("default", b"atom_k2").unwrap(), Some(b"v2".to_vec()));
        assert_eq!(snap.get("default", b"atom_k3").unwrap(), Some(b"v3".to_vec()));
        assert!(commit_ts > 0);
    }

    #[test]
    fn test_committed_writes_gc_automatic() {
        let (_dir, tm) = setup();

        // Commit 15000 transactions, each writing a unique key
        for i in 0..15_000u64 {
            let mut tx = tm.begin();
            let key = format!("gc_key_{}", i);
            tx.put("default", key.as_bytes(), b"val").unwrap();
            tm.commit(&mut tx).unwrap();
        }

        // Commit one more — the auto-GC should have pruned old entries
        let mut tx = tm.begin();
        tx.put("default", b"gc_final", b"done").unwrap();
        let commit_ts = tm.commit(&mut tx).unwrap();
        assert!(commit_ts > 0);

        // Verify the final value is readable (system didn't OOM or break)
        let snap = tm.snapshot();
        assert_eq!(snap.get("default", b"gc_final").unwrap(), Some(b"done".to_vec()));
    }
}
