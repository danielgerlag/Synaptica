use std::sync::Arc;
use synaptica_storage::mvcc::MvccStore;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TxError {
    #[error("mvcc error: {0}")]
    Mvcc(#[from] synaptica_storage::mvcc::MvccError),

    #[error("write conflict: key was modified by concurrent transaction")]
    WriteConflict,

    #[error("transaction not active")]
    NotActive,

    #[error("transaction already committed")]
    AlreadyCommitted,

    #[error("transaction already rolled back")]
    AlreadyRolledBack,

    #[error("serialization error: {0}")]
    Serialization(String),
}

pub type TxResult<T> = Result<T, TxError>;

/// Transaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxState {
    Active,
    Committed,
    RolledBack,
}

/// Represents a buffered write operation within a transaction.
#[derive(Debug, Clone)]
pub struct BufferedWrite {
    pub cf_name: String,
    pub key: Vec<u8>,
    pub value: Option<Vec<u8>>, // None = delete (tombstone)
}

/// A single MVCC transaction with snapshot isolation.
/// Provides snapshot isolation with write-write conflict detection.
pub struct Transaction {
    pub id: u64,
    pub snapshot_ts: u64,
    pub state: TxState,
    write_set: Vec<BufferedWrite>,
    store: Arc<MvccStore>,
}

impl Transaction {
    /// Begin a new transaction.
    pub(crate) fn new(id: u64, snapshot_ts: u64, store: Arc<MvccStore>) -> Self {
        Self {
            id,
            snapshot_ts,
            state: TxState::Active,
            write_set: Vec::new(),
            store,
        }
    }

    /// Read a key, seeing the snapshot + any local writes (read-your-own-writes).
    pub fn get(&mut self, cf_name: &str, key: &[u8]) -> TxResult<Option<Vec<u8>>> {
        if self.state != TxState::Active {
            return Err(TxError::NotActive);
        }

        // Check write buffer first (read-your-own-writes)
        for w in self.write_set.iter().rev() {
            if w.cf_name == cf_name && w.key == key {
                return Ok(w.value.clone());
            }
        }

        // Read from MVCC store at snapshot
        Ok(self.store.get_at(cf_name, key, self.snapshot_ts)?)
    }

    /// Buffer a write (applied on commit).
    pub fn put(&mut self, cf_name: &str, key: &[u8], value: &[u8]) -> TxResult<()> {
        if self.state != TxState::Active {
            return Err(TxError::NotActive);
        }
        self.write_set.push(BufferedWrite {
            cf_name: cf_name.to_string(),
            key: key.to_vec(),
            value: Some(value.to_vec()),
        });
        Ok(())
    }

    /// Buffer a delete (applied on commit).
    pub fn delete(&mut self, cf_name: &str, key: &[u8]) -> TxResult<()> {
        if self.state != TxState::Active {
            return Err(TxError::NotActive);
        }
        self.write_set.push(BufferedWrite {
            cf_name: cf_name.to_string(),
            key: key.to_vec(),
            value: None,
        });
        Ok(())
    }

    /// Get the write set (for conflict detection).
    pub fn write_set(&self) -> &[BufferedWrite] {
        &self.write_set
    }
}
