use crate::transaction::{Transaction, TxError, TxResult, TxState};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use synaptica_storage::mvcc::{MvccStore, TimestampOracle};

/// MVCC transaction manager with optimistic concurrency control.
pub struct TransactionManager {
    store: Arc<MvccStore>,
    ts_oracle: Arc<TimestampOracle>,
    /// Tracks committed transactions and their write keys for conflict detection.
    /// Maps commit_ts → list of (cf_name, key) that were written.
    committed_writes: RwLock<HashMap<u64, Vec<(String, Vec<u8>)>>>,
    /// Watermark: oldest active transaction's snapshot timestamp.
    gc_watermark: RwLock<u64>,
}

impl TransactionManager {
    pub fn new(store: Arc<MvccStore>, ts_oracle: Arc<TimestampOracle>) -> Self {
        Self {
            store,
            ts_oracle,
            committed_writes: RwLock::new(HashMap::new()),
            gc_watermark: RwLock::new(0),
        }
    }

    /// Begin a new transaction with a snapshot at the current timestamp.
    pub fn begin(&self) -> Transaction {
        let snapshot_ts = self.ts_oracle.current();
        let tx_id = self.ts_oracle.next();
        Transaction::new(tx_id, snapshot_ts, self.store.clone())
    }

    /// Commit a transaction: validate no conflicts, then apply writes atomically.
    pub fn commit(&self, tx: &mut Transaction) -> TxResult<u64> {
        if tx.state != TxState::Active {
            return Err(TxError::AlreadyCommitted);
        }

        // Hold write lock for the entire conflict check + commit to prevent TOCTOU races
        let mut cw = self.committed_writes.write();

        // Inline conflict check: did any concurrent transaction commit writes
        // to keys in our write set since our snapshot?
        let write_keys: std::collections::HashSet<(&str, &[u8])> = tx
            .write_set()
            .iter()
            .map(|w| (w.cf_name.as_str(), w.key.as_slice()))
            .collect();

        if !write_keys.is_empty() {
            for (&commit_ts, keys) in cw.iter() {
                if commit_ts > tx.snapshot_ts {
                    for (cf, key) in keys {
                        if write_keys.contains(&(cf.as_str(), key.as_slice())) {
                            return Err(TxError::WriteConflict);
                        }
                    }
                }
            }
        }

        // Get commit timestamp
        let commit_ts = self.ts_oracle.next();

        // Apply all buffered writes atomically via batch
        let writes: Vec<(&str, &[u8], Option<&[u8]>)> = tx
            .write_set()
            .iter()
            .map(|w| (w.cf_name.as_str(), w.key.as_slice(), w.value.as_deref()))
            .collect();
        self.store.batch_write_at(&writes, commit_ts)?;

        // Record committed writes for future conflict detection
        let committed_keys: Vec<(String, Vec<u8>)> = tx
            .write_set()
            .iter()
            .map(|w| (w.cf_name.clone(), w.key.clone()))
            .collect();
        cw.insert(commit_ts, committed_keys);

        // Auto-GC: prune old entries to prevent unbounded growth
        const GC_THRESHOLD: usize = 10_000;
        if cw.len() > GC_THRESHOLD {
            let gc_watermark = commit_ts.saturating_sub(GC_THRESHOLD as u64);
            cw.retain(|&ts, _| ts > gc_watermark);
        }

        drop(cw);

        tx.state = TxState::Committed;
        Ok(commit_ts)
    }

    /// Rollback a transaction, discarding all buffered writes.
    pub fn rollback(&self, tx: &mut Transaction) -> TxResult<()> {
        if tx.state != TxState::Active {
            return Err(TxError::NotActive);
        }
        tx.state = TxState::RolledBack;
        Ok(())
    }

    /// Clean up old committed write records below the watermark.
    pub fn gc_committed_writes(&self, watermark: u64) {
        let mut cw = self.committed_writes.write();
        cw.retain(|&ts, _| ts > watermark);
        *self.gc_watermark.write() = watermark;
    }

    /// Get a read-only snapshot at the current timestamp.
    pub fn snapshot(&self) -> Snapshot {
        let ts = self.ts_oracle.current();
        Snapshot {
            ts,
            store: self.store.clone(),
        }
    }
}

/// A read-only snapshot for consistent reads without a full transaction.
pub struct Snapshot {
    pub ts: u64,
    store: Arc<MvccStore>,
}

impl Snapshot {
    pub fn get(&self, cf_name: &str, key: &[u8]) -> TxResult<Option<Vec<u8>>> {
        Ok(self.store.get_at(cf_name, key, self.ts)?)
    }

    pub fn prefix_scan(
        &self,
        cf_name: &str,
        prefix: &[u8],
    ) -> TxResult<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self.store.prefix_scan_at(cf_name, prefix, self.ts)?)
    }
}