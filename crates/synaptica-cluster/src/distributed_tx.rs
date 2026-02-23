use crate::partition::PartitionId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Unique identifier for a distributed transaction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DistributedTxId(pub String);

/// States a distributed transaction passes through during 2PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistributedTxState {
    Initiated,
    Preparing,
    Prepared,
    Committing,
    Committed,
    Aborting,
    Aborted,
}

/// Errors that can occur during distributed transaction coordination.
#[derive(Debug, Error)]
pub enum DistributedTxError {
    #[error("participant failed: {0:?}")]
    ParticipantFailed(PartitionId),

    #[error("timeout waiting for participant response")]
    Timeout,

    #[error("transaction already committed")]
    AlreadyCommitted,

    #[error("transaction already aborted")]
    AlreadyAborted,

    #[error("coordinator failed")]
    CoordinatorFailed,
}

// ---------------------------------------------------------------------------
// ParticipantHandle – abstract interface to partition participants
// ---------------------------------------------------------------------------

/// Abstract interface for communicating with a partition participant in 2PC.
pub trait ParticipantHandle: Send + Sync {
    /// Phase 1: ask the participant to prepare. Returns `true` if it votes YES.
    fn prepare(&self, tx_id: &DistributedTxId) -> Result<bool, DistributedTxError>;

    /// Phase 2 (commit path): tell the participant to commit.
    fn commit(&self, tx_id: &DistributedTxId) -> Result<(), DistributedTxError>;

    /// Phase 2 (abort path): tell the participant to abort.
    fn abort(&self, tx_id: &DistributedTxId) -> Result<(), DistributedTxError>;
}

// ---------------------------------------------------------------------------
// DistributedTxLog – WAL for 2PC decisions
// ---------------------------------------------------------------------------

/// Entry types persisted in the 2PC decision log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogEntry {
    Prepare {
        tx_id: DistributedTxId,
        participants: Vec<PartitionId>,
    },
    Commit {
        tx_id: DistributedTxId,
    },
    Abort {
        tx_id: DistributedTxId,
    },
}

/// Write-ahead log for 2PC decisions.
///
/// Ensures crash recovery: if the coordinator crashes after prepare but before
/// commit/abort, we can determine the outcome on recovery.
#[derive(Debug)]
pub struct DistributedTxLog {
    entries: Vec<LogEntry>,
    wal_file: Option<std::fs::File>,
}

impl Default for DistributedTxLog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            wal_file: None,
        }
    }
}

impl DistributedTxLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a persistent WAL backed by a file.
    pub fn with_path(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            entries: Vec::new(),
            wal_file: Some(file),
        })
    }

    /// Recover entries from a WAL file.
    pub fn recover_from_file(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let contents = std::fs::read_to_string(&path)?;
        let mut entries = Vec::new();
        for line in contents.lines() {
            if let Ok(entry) = serde_json::from_str::<LogEntry>(line) {
                entries.push(entry);
            }
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            entries,
            wal_file: Some(file),
        })
    }

    fn persist_entry(&mut self, entry: &LogEntry) {
        if let Some(ref mut file) = self.wal_file {
            if let Ok(json) = serde_json::to_string(entry) {
                let _ = writeln!(file, "{}", json);
                let _ = file.flush();
            }
        }
    }

    /// Record that we are entering the prepare phase for `tx_id`.
    pub fn log_prepare(&mut self, tx_id: &DistributedTxId, participants: &[PartitionId]) {
        let entry = LogEntry::Prepare {
            tx_id: tx_id.clone(),
            participants: participants.to_vec(),
        };
        self.entries.push(entry.clone());
        self.persist_entry(&entry);
    }

    /// Record the commit decision for `tx_id`.
    pub fn log_commit(&mut self, tx_id: &DistributedTxId) {
        let entry = LogEntry::Commit {
            tx_id: tx_id.clone(),
        };
        self.entries.push(entry.clone());
        self.persist_entry(&entry);
    }

    /// Record the abort decision for `tx_id`.
    pub fn log_abort(&mut self, tx_id: &DistributedTxId) {
        let entry = LogEntry::Abort {
            tx_id: tx_id.clone(),
        };
        self.entries.push(entry.clone());
        self.persist_entry(&entry);
    }

    /// Return transaction IDs that have a prepare record but no commit/abort
    /// (i.e. in-doubt transactions that need resolution after a crash).
    pub fn in_doubt_transactions(&self) -> Vec<DistributedTxId> {
        let mut prepared: HashMap<DistributedTxId, bool> = HashMap::new();
        for entry in &self.entries {
            match entry {
                LogEntry::Prepare { tx_id, .. } => {
                    prepared.entry(tx_id.clone()).or_insert(true);
                }
                LogEntry::Commit { tx_id } | LogEntry::Abort { tx_id } => {
                    prepared.insert(tx_id.clone(), false);
                }
            }
        }
        prepared
            .into_iter()
            .filter_map(|(id, in_doubt)| if in_doubt { Some(id) } else { None })
            .collect()
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }
}

// ---------------------------------------------------------------------------
// TwoPhaseCoordinator
// ---------------------------------------------------------------------------

/// Internal bookkeeping for a single distributed transaction.
struct TxRecord {
    state: DistributedTxState,
    participants: Vec<PartitionId>,
}

/// Coordinator that drives the two-phase commit protocol across partitions.
pub struct TwoPhaseCoordinator {
    next_id: Mutex<u64>,
    transactions: Mutex<HashMap<DistributedTxId, TxRecord>>,
    log: Mutex<DistributedTxLog>,
    participants: HashMap<PartitionId, Arc<dyn ParticipantHandle>>,
}

impl TwoPhaseCoordinator {
    pub fn new(participants: HashMap<PartitionId, Arc<dyn ParticipantHandle>>) -> Self {
        Self {
            next_id: Mutex::new(1),
            transactions: Mutex::new(HashMap::new()),
            log: Mutex::new(DistributedTxLog::new()),
            participants,
        }
    }

    /// Start a new distributed transaction and return its unique ID.
    pub fn begin_distributed_tx(&self) -> DistributedTxId {
        let mut next = self.next_id.lock().unwrap();
        let id = DistributedTxId(format!("dtx-{}", *next));
        *next += 1;

        self.transactions.lock().unwrap().insert(
            id.clone(),
            TxRecord {
                state: DistributedTxState::Initiated,
                participants: Vec::new(),
            },
        );
        id
    }

    /// Phase 1 – ask every participant to prepare.
    ///
    /// Returns `Ok(true)` if all participants voted YES, `Ok(false)` if any
    /// voted NO (the caller should then call `abort`).
    pub fn prepare(
        &self,
        tx_id: &DistributedTxId,
        participant_ids: Vec<PartitionId>,
    ) -> Result<bool, DistributedTxError> {
        // Update state to Preparing
        {
            let mut txns = self.transactions.lock().unwrap();
            let record = txns
                .get_mut(tx_id)
                .ok_or(DistributedTxError::CoordinatorFailed)?;
            match record.state {
                DistributedTxState::Initiated => {}
                DistributedTxState::Committed => return Err(DistributedTxError::AlreadyCommitted),
                DistributedTxState::Aborted => return Err(DistributedTxError::AlreadyAborted),
                _ => return Err(DistributedTxError::CoordinatorFailed),
            }
            record.state = DistributedTxState::Preparing;
            record.participants = participant_ids.clone();
        }

        // WAL: record prepare intent *before* contacting participants
        self.log
            .lock()
            .unwrap()
            .log_prepare(tx_id, &participant_ids);

        // Contact each participant
        let mut all_yes = true;
        for pid in &participant_ids {
            let handle = self
                .participants
                .get(pid)
                .ok_or_else(|| DistributedTxError::ParticipantFailed(pid.clone()))?;

            match handle.prepare(tx_id) {
                Ok(true) => {}
                Ok(false) => {
                    all_yes = false;
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        // Transition to Prepared (or stay for abort path)
        {
            let mut txns = self.transactions.lock().unwrap();
            let record = txns.get_mut(tx_id).unwrap();
            record.state = if all_yes {
                DistributedTxState::Prepared
            } else {
                DistributedTxState::Preparing
            };
        }

        Ok(all_yes)
    }

    /// Phase 2 (commit) – tell all prepared participants to commit.
    pub fn commit(&self, tx_id: &DistributedTxId) -> Result<(), DistributedTxError> {
        let participant_ids: Vec<PartitionId>;
        {
            let mut txns = self.transactions.lock().unwrap();
            let record = txns
                .get_mut(tx_id)
                .ok_or(DistributedTxError::CoordinatorFailed)?;
            match record.state {
                DistributedTxState::Prepared => {}
                DistributedTxState::Committed => return Err(DistributedTxError::AlreadyCommitted),
                DistributedTxState::Aborted => return Err(DistributedTxError::AlreadyAborted),
                _ => return Err(DistributedTxError::CoordinatorFailed),
            }
            record.state = DistributedTxState::Committing;
            participant_ids = record.participants.clone();
        }

        // WAL: record commit decision *before* telling participants
        self.log.lock().unwrap().log_commit(tx_id);

        for pid in &participant_ids {
            let handle = self
                .participants
                .get(pid)
                .ok_or_else(|| DistributedTxError::ParticipantFailed(pid.clone()))?;
            handle.commit(tx_id)?;
        }

        // Transition to Committed
        {
            let mut txns = self.transactions.lock().unwrap();
            let record = txns.get_mut(tx_id).unwrap();
            record.state = DistributedTxState::Committed;
        }
        Ok(())
    }

    /// Abort – tell all participants to roll back.
    pub fn abort(&self, tx_id: &DistributedTxId) -> Result<(), DistributedTxError> {
        let participant_ids: Vec<PartitionId>;
        {
            let mut txns = self.transactions.lock().unwrap();
            let record = txns
                .get_mut(tx_id)
                .ok_or(DistributedTxError::CoordinatorFailed)?;
            match record.state {
                DistributedTxState::Committed => return Err(DistributedTxError::AlreadyCommitted),
                DistributedTxState::Aborted => return Err(DistributedTxError::AlreadyAborted),
                _ => {}
            }
            record.state = DistributedTxState::Aborting;
            participant_ids = record.participants.clone();
        }

        // WAL: record abort decision
        self.log.lock().unwrap().log_abort(tx_id);

        for pid in &participant_ids {
            if let Some(handle) = self.participants.get(pid) {
                // Best-effort: we still try remaining participants even if one fails.
                let _ = handle.abort(tx_id);
            }
        }

        // Transition to Aborted
        {
            let mut txns = self.transactions.lock().unwrap();
            let record = txns.get_mut(tx_id).unwrap();
            record.state = DistributedTxState::Aborted;
        }
        Ok(())
    }

    /// Recover in-doubt transactions after a coordinator crash.
    ///
    /// Returns the IDs of transactions that were prepared but neither
    /// committed nor aborted.
    pub fn recover(&self) -> Result<Vec<DistributedTxId>, DistributedTxError> {
        let log = self.log.lock().unwrap();
        Ok(log.in_doubt_transactions())
    }

    /// Retrieve the current state of a distributed transaction.
    pub fn state(&self, tx_id: &DistributedTxId) -> Option<DistributedTxState> {
        self.transactions
            .lock()
            .unwrap()
            .get(tx_id)
            .map(|r| r.state.clone())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Mock participant that always votes YES.
    struct YesParticipant {
        committed: AtomicBool,
        aborted: AtomicBool,
    }

    impl YesParticipant {
        fn new() -> Self {
            Self {
                committed: AtomicBool::new(false),
                aborted: AtomicBool::new(false),
            }
        }
    }

    impl ParticipantHandle for YesParticipant {
        fn prepare(&self, _tx_id: &DistributedTxId) -> Result<bool, DistributedTxError> {
            Ok(true)
        }
        fn commit(&self, _tx_id: &DistributedTxId) -> Result<(), DistributedTxError> {
            self.committed.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn abort(&self, _tx_id: &DistributedTxId) -> Result<(), DistributedTxError> {
            self.aborted.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Mock participant that always votes NO.
    struct NoParticipant;

    impl ParticipantHandle for NoParticipant {
        fn prepare(&self, _tx_id: &DistributedTxId) -> Result<bool, DistributedTxError> {
            Ok(false)
        }
        fn commit(&self, _tx_id: &DistributedTxId) -> Result<(), DistributedTxError> {
            Ok(())
        }
        fn abort(&self, _tx_id: &DistributedTxId) -> Result<(), DistributedTxError> {
            Ok(())
        }
    }

    fn make_pid(name: &str) -> PartitionId {
        PartitionId(name.to_string())
    }

    #[test]
    fn test_successful_2pc_commit() {
        let p1 = Arc::new(YesParticipant::new());
        let p2 = Arc::new(YesParticipant::new());

        let mut handles: HashMap<PartitionId, Arc<dyn ParticipantHandle>> = HashMap::new();
        handles.insert(make_pid("p1"), p1.clone());
        handles.insert(make_pid("p2"), p2.clone());

        let coord = TwoPhaseCoordinator::new(handles);
        let tx = coord.begin_distributed_tx();

        // Phase 1
        let all_yes = coord
            .prepare(&tx, vec![make_pid("p1"), make_pid("p2")])
            .unwrap();
        assert!(all_yes);
        assert_eq!(coord.state(&tx), Some(DistributedTxState::Prepared));

        // Phase 2
        coord.commit(&tx).unwrap();
        assert_eq!(coord.state(&tx), Some(DistributedTxState::Committed));
        assert!(p1.committed.load(Ordering::SeqCst));
        assert!(p2.committed.load(Ordering::SeqCst));
    }

    #[test]
    fn test_abort_when_participant_votes_no() {
        let p1 = Arc::new(YesParticipant::new());
        let p_no: Arc<dyn ParticipantHandle> = Arc::new(NoParticipant);

        let mut handles: HashMap<PartitionId, Arc<dyn ParticipantHandle>> = HashMap::new();
        handles.insert(make_pid("p1"), p1.clone());
        handles.insert(make_pid("p_no"), p_no);

        let coord = TwoPhaseCoordinator::new(handles);
        let tx = coord.begin_distributed_tx();

        // Phase 1 – at least one NO vote
        let all_yes = coord
            .prepare(&tx, vec![make_pid("p1"), make_pid("p_no")])
            .unwrap();
        assert!(!all_yes);

        // Coordinator decides to abort
        coord.abort(&tx).unwrap();
        assert_eq!(coord.state(&tx), Some(DistributedTxState::Aborted));
        assert!(p1.aborted.load(Ordering::SeqCst));
    }

    #[test]
    fn test_state_transitions() {
        let p1: Arc<dyn ParticipantHandle> = Arc::new(YesParticipant::new());
        let mut handles: HashMap<PartitionId, Arc<dyn ParticipantHandle>> = HashMap::new();
        handles.insert(make_pid("p1"), p1);

        let coord = TwoPhaseCoordinator::new(handles);
        let tx = coord.begin_distributed_tx();

        assert_eq!(coord.state(&tx), Some(DistributedTxState::Initiated));

        coord.prepare(&tx, vec![make_pid("p1")]).unwrap();
        assert_eq!(coord.state(&tx), Some(DistributedTxState::Prepared));

        coord.commit(&tx).unwrap();
        assert_eq!(coord.state(&tx), Some(DistributedTxState::Committed));

        // Double-commit is an error
        let err = coord.commit(&tx);
        assert!(err.is_err());
    }

    #[test]
    fn test_recover_in_doubt_transactions() {
        let p1: Arc<dyn ParticipantHandle> = Arc::new(YesParticipant::new());
        let mut handles: HashMap<PartitionId, Arc<dyn ParticipantHandle>> = HashMap::new();
        handles.insert(make_pid("p1"), p1);

        let coord = TwoPhaseCoordinator::new(handles);

        // tx1: prepared but not committed/aborted (simulates crash)
        let tx1 = coord.begin_distributed_tx();
        coord.prepare(&tx1, vec![make_pid("p1")]).unwrap();

        // tx2: fully committed
        let tx2 = coord.begin_distributed_tx();
        coord.prepare(&tx2, vec![make_pid("p1")]).unwrap();
        coord.commit(&tx2).unwrap();

        let in_doubt = coord.recover().unwrap();
        assert_eq!(in_doubt.len(), 1);
        assert!(in_doubt.contains(&tx1));
    }

    #[test]
    fn test_wal_file_persistence() {
        let dir = std::env::temp_dir().join("synaptica_wal_test_persist");
        let _ = std::fs::create_dir_all(&dir);
        let wal_path = dir.join("test.wal");
        let _ = std::fs::remove_file(&wal_path);

        let tx_id = DistributedTxId("tx-persist-1".to_string());
        let participants = vec![PartitionId("p1".to_string())];

        {
            let mut log = DistributedTxLog::with_path(&wal_path).unwrap();
            log.log_prepare(&tx_id, &participants);
            log.log_commit(&tx_id);
        }

        let recovered = DistributedTxLog::recover_from_file(&wal_path).unwrap();
        assert_eq!(recovered.entries().len(), 2);
        assert!(recovered.in_doubt_transactions().is_empty());

        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_wal_recovery_in_doubt() {
        let dir = std::env::temp_dir().join("synaptica_wal_test_indoubt");
        let _ = std::fs::create_dir_all(&dir);
        let wal_path = dir.join("test.wal");
        let _ = std::fs::remove_file(&wal_path);

        let tx_id = DistributedTxId("tx-indoubt-1".to_string());
        let participants = vec![PartitionId("p1".to_string())];

        {
            let mut log = DistributedTxLog::with_path(&wal_path).unwrap();
            log.log_prepare(&tx_id, &participants);
            // No commit or abort — simulates crash
        }

        let recovered = DistributedTxLog::recover_from_file(&wal_path).unwrap();
        let in_doubt = recovered.in_doubt_transactions();
        assert_eq!(in_doubt.len(), 1);
        assert!(in_doubt.contains(&tx_id));

        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_dir(&dir);
    }
}
