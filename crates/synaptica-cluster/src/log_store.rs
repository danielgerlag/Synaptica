use std::fmt::Debug;
use std::ops::RangeBounds;
use std::sync::Arc;

use openraft::storage::LogState;
use openraft::storage::RaftLogReader;
use openraft::BasicNode;
use openraft::Entry;
use openraft::LogId;
use openraft::OptionalSend;
use openraft::RaftLogId;
use openraft::RaftStorage;
use openraft::RaftTypeConfig;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::Vote;
use tokio::sync::RwLock;

use crate::raft::{NodeId, RaftResponse, TypeConfig};
use crate::state_machine::StateMachineApplier;
use synaptica_storage::engine::{StorageEngine, StorageResult};

const VOTE_KEY: &[u8] = b"raft_vote";
const LAST_PURGED_KEY: &[u8] = b"raft_last_purged";
const COMMITTED_KEY: &[u8] = b"raft_committed";
const LAST_APPLIED_KEY: &[u8] = b"raft_last_applied";
const LAST_MEMBERSHIP_KEY: &[u8] = b"raft_last_membership";

/// RocksDB-backed Raft log and state store.
///
/// Stores log entries in RAFT_LOG column family (key = big-endian u64 index).
/// Stores metadata (vote, purged marker, applied state, membership) in RAFT_META.
pub struct RocksLogStore {
    storage: Arc<StorageEngine>,
    applier: Option<Arc<StateMachineApplier>>,
    current_snapshot: RwLock<Option<StoredSnapshot>>,
}

#[derive(Debug, Clone)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta<NodeId, BasicNode>,
    pub data: Vec<u8>,
}

impl RocksLogStore {
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self {
            storage,
            applier: None,
            current_snapshot: RwLock::new(None),
        }
    }

    /// Create a log store with a state machine applier that executes committed mutations.
    pub fn with_applier(storage: Arc<StorageEngine>, applier: Arc<StateMachineApplier>) -> Self {
        Self {
            storage,
            applier: Some(applier),
            current_snapshot: RwLock::new(None),
        }
    }

    fn index_to_key(index: u64) -> [u8; 8] {
        index.to_be_bytes()
    }

    // Synchronous helpers that don't hold iterator state across await points.
    fn meta_get(&self, key: &[u8]) -> StorageResult<Option<Vec<u8>>> {
        self.storage.raft_meta_get(key)
    }

    fn meta_put(&self, key: &[u8], value: &[u8]) -> StorageResult<()> {
        self.storage.raft_meta_put(key, value)
    }

    fn meta_delete(&self, key: &[u8]) -> StorageResult<()> {
        self.storage.raft_meta_delete(key)
    }

    fn log_put(&self, key: &[u8], value: &[u8]) -> StorageResult<()> {
        self.storage.raft_log_put(key, value)
    }

    fn log_delete(&self, key: &[u8]) -> StorageResult<()> {
        self.storage.raft_log_delete(key)
    }

    fn read_log_entries_sync(
        &self,
        start: u64,
        end: Option<u64>,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<NodeId>> {
        let start_key = Self::index_to_key(start);
        let scanned = self
            .storage
            .raft_log_scan_from(&start_key)
            .map_err(|e| StorageIOError::read_logs(&e))?;

        let mut entries = Vec::new();
        for (key, value) in scanned {
            if key.len() != 8 {
                continue;
            }
            let index = u64::from_be_bytes(key[..8].try_into().unwrap());
            if let Some(end_idx) = end {
                if index >= end_idx {
                    break;
                }
            }
            let entry: Entry<TypeConfig> =
                bincode::deserialize(&value).map_err(|e| StorageIOError::read_logs(&*e))?;
            entries.push(entry);
        }
        Ok(entries)
    }

    fn last_log_id_sync(&self) -> Result<Option<LogId<NodeId>>, StorageError<NodeId>> {
        match self
            .storage
            .raft_log_last()
            .map_err(|e| StorageIOError::read_logs(&e))?
        {
            Some(value) => {
                let entry: Entry<TypeConfig> =
                    bincode::deserialize(&value).map_err(|e| StorageIOError::read_logs(&*e))?;
                Ok(Some(*entry.get_log_id()))
            }
            None => Ok(None),
        }
    }

    fn delete_logs_from_sync(&self, from_index: u64) -> Result<(), StorageError<NodeId>> {
        let start_key = Self::index_to_key(from_index);
        let keys: Vec<Vec<u8>> = self
            .storage
            .raft_log_scan_from(&start_key)
            .map_err(|e| StorageIOError::read_logs(&e))?
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        for key in keys {
            self.log_delete(&key)
                .map_err(|e| StorageIOError::write_logs(&e))?;
        }
        Ok(())
    }

    fn delete_logs_upto_sync(&self, upto_index: u64) -> Result<(), StorageError<NodeId>> {
        let start_key = Self::index_to_key(0);
        let end_key = Self::index_to_key(upto_index.saturating_add(1));
        let keys: Vec<Vec<u8>> = self
            .storage
            .raft_log_scan_from(&start_key)
            .map_err(|e| StorageIOError::read_logs(&e))?
            .into_iter()
            .filter_map(|(key, _)| (key[..] < end_key[..]).then_some(key))
            .collect();
        for key in keys {
            self.log_delete(&key)
                .map_err(|e| StorageIOError::write_logs(&e))?;
        }
        Ok(())
    }
}

impl RaftLogReader<TypeConfig> for Arc<RocksLogStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<NodeId>> {
        let start = match range.start_bound() {
            std::ops::Bound::Included(&s) => s,
            std::ops::Bound::Excluded(&s) => s + 1,
            std::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            std::ops::Bound::Included(&e) => Some(e + 1),
            std::ops::Bound::Excluded(&e) => Some(e),
            std::ops::Bound::Unbounded => None,
        };
        self.read_log_entries_sync(start, end)
    }
}

impl RaftStorage<TypeConfig> for Arc<RocksLogStore> {
    type LogReader = Arc<RocksLogStore>;

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<NodeId>> {
        let last_purged: Option<LogId<NodeId>> = self
            .meta_get(LAST_PURGED_KEY)
            .map_err(|e| StorageIOError::read(&e))?
            .map(|v| bincode::deserialize(&v))
            .transpose()
            .map_err(|e| StorageIOError::read(&*e))?;

        let last_log_id = self.last_log_id_sync()?;
        let last = last_log_id.or(last_purged);

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last,
        })
    }

    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        let bytes = bincode::serialize(vote).map_err(|e| StorageIOError::write_vote(&*e))?;
        self.meta_put(VOTE_KEY, &bytes)
            .map_err(|e| StorageIOError::write_vote(&e))?;
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        match self
            .meta_get(VOTE_KEY)
            .map_err(|e| StorageIOError::read_vote(&e))?
        {
            Some(bytes) => {
                let vote: Vote<NodeId> =
                    bincode::deserialize(&bytes).map_err(|e| StorageIOError::read_vote(&*e))?;
                Ok(Some(vote))
            }
            None => Ok(None),
        }
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<NodeId>>,
    ) -> Result<(), StorageError<NodeId>> {
        match committed {
            Some(c) => {
                let bytes = bincode::serialize(&c).map_err(|e| StorageIOError::write(&*e))?;
                self.meta_put(COMMITTED_KEY, &bytes)
                    .map_err(|e| StorageIOError::write(&e))?;
            }
            None => {
                let _ = self.meta_delete(COMMITTED_KEY);
            }
        }
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<NodeId>>, StorageError<NodeId>> {
        match self
            .meta_get(COMMITTED_KEY)
            .map_err(|e| StorageIOError::read(&e))?
        {
            Some(bytes) => {
                let log_id: LogId<NodeId> =
                    bincode::deserialize(&bytes).map_err(|e| StorageIOError::read(&*e))?;
                Ok(Some(log_id))
            }
            None => Ok(None),
        }
    }

    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, BasicNode>), StorageError<NodeId>>
    {
        let last_applied: Option<LogId<NodeId>> = self
            .meta_get(LAST_APPLIED_KEY)
            .map_err(|e| StorageIOError::read_state_machine(&e))?
            .map(|v| bincode::deserialize(&v))
            .transpose()
            .map_err(|e| StorageIOError::read_state_machine(&*e))?;

        let last_membership: StoredMembership<NodeId, BasicNode> = self
            .meta_get(LAST_MEMBERSHIP_KEY)
            .map_err(|e| StorageIOError::read_state_machine(&e))?
            .map(|v| bincode::deserialize(&v))
            .transpose()
            .map_err(|e| StorageIOError::read_state_machine(&*e))?
            .unwrap_or_default();

        Ok((last_applied, last_membership))
    }

    async fn delete_conflict_logs_since(
        &mut self,
        log_id: LogId<NodeId>,
    ) -> Result<(), StorageError<NodeId>> {
        self.delete_logs_from_sync(log_id.index)
    }

    async fn purge_logs_upto(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let bytes = bincode::serialize(&log_id).map_err(|e| StorageIOError::write(&*e))?;
        self.meta_put(LAST_PURGED_KEY, &bytes)
            .map_err(|e| StorageIOError::write(&e))?;
        self.delete_logs_upto_sync(log_id.index)
    }

    async fn append_to_log<I>(&mut self, entries: I) -> Result<(), StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend,
    {
        for entry in entries {
            let key = RocksLogStore::index_to_key(entry.log_id.index);
            let value = bincode::serialize(&entry)
                .map_err(|e| StorageIOError::write_log_entry(entry.log_id, &*e))?;
            self.log_put(&key, &value)
                .map_err(|e| StorageIOError::write_log_entry(entry.log_id, &e))?;
        }
        Ok(())
    }

    async fn apply_to_state_machine(
        &mut self,
        entries: &[Entry<TypeConfig>],
    ) -> Result<Vec<RaftResponse>, StorageError<NodeId>> {
        let mut responses = Vec::with_capacity(entries.len());
        for entry in entries {
            let bytes =
                bincode::serialize(&entry.log_id).map_err(|e| StorageIOError::write(&*e))?;
            self.meta_put(LAST_APPLIED_KEY, &bytes)
                .map_err(|e| StorageIOError::write(&e))?;

            match &entry.payload {
                openraft::EntryPayload::Blank => {
                    responses.push(RaftResponse {
                        success: true,
                        error: None,
                        rows_affected: 0,
                    });
                }
                openraft::EntryPayload::Membership(mem) => {
                    let stored = StoredMembership::new(Some(entry.log_id), mem.clone());
                    let bytes =
                        bincode::serialize(&stored).map_err(|e| StorageIOError::write(&*e))?;
                    self.meta_put(LAST_MEMBERSHIP_KEY, &bytes)
                        .map_err(|e| StorageIOError::write(&e))?;
                    responses.push(RaftResponse {
                        success: true,
                        error: None,
                        rows_affected: 0,
                    });
                }
                openraft::EntryPayload::Normal(ref req) => {
                    let response = if let Some(ref applier) = self.applier {
                        applier.apply(req)
                    } else {
                        RaftResponse {
                            success: true,
                            error: None,
                            rows_affected: 0,
                        }
                    };
                    responses.push(response);
                }
            }
        }
        Ok(responses)
    }

    type SnapshotBuilder = Arc<RocksLogStore>;

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, StorageError<NodeId>> {
        Ok(Box::new(std::io::Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, BasicNode>,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<NodeId>> {
        let data = snapshot.into_inner();
        tracing::info!(snapshot_size = data.len(), "installing snapshot");

        if let Some(ref last) = meta.last_log_id {
            let bytes = bincode::serialize(last).map_err(|e| StorageIOError::write(&*e))?;
            self.meta_put(LAST_APPLIED_KEY, &bytes)
                .map_err(|e| StorageIOError::write(&e))?;
        }
        let mem_bytes =
            bincode::serialize(&meta.last_membership).map_err(|e| StorageIOError::write(&*e))?;
        self.meta_put(LAST_MEMBERSHIP_KEY, &mem_bytes)
            .map_err(|e| StorageIOError::write(&e))?;

        // Now safe to await — no cf handles live
        let mut current = self.current_snapshot.write().await;
        *current = Some(StoredSnapshot {
            meta: meta.clone(),
            data,
        });
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<openraft::storage::Snapshot<TypeConfig>>, StorageError<NodeId>> {
        let snap = self.current_snapshot.read().await;
        match &*snap {
            Some(s) => Ok(Some(openraft::storage::Snapshot {
                meta: s.meta.clone(),
                snapshot: Box::new(std::io::Cursor::new(s.data.clone())),
            })),
            None => Ok(None),
        }
    }
}

impl openraft::RaftSnapshotBuilder<TypeConfig> for Arc<RocksLogStore> {
    async fn build_snapshot(
        &mut self,
    ) -> Result<openraft::storage::Snapshot<TypeConfig>, StorageError<NodeId>> {
        // Read state synchronously — no cf handles held across await
        let last_applied: Option<LogId<NodeId>> = self
            .meta_get(LAST_APPLIED_KEY)
            .map_err(|e| StorageIOError::read_state_machine(&e))?
            .map(|v| bincode::deserialize(&v))
            .transpose()
            .map_err(|e| StorageIOError::read_state_machine(&*e))?;

        let last_membership: StoredMembership<NodeId, BasicNode> = self
            .meta_get(LAST_MEMBERSHIP_KEY)
            .map_err(|e| StorageIOError::read_state_machine(&e))?
            .map(|v| bincode::deserialize(&v))
            .transpose()
            .map_err(|e| StorageIOError::read_state_machine(&*e))?
            .unwrap_or_default();

        let snapshot_id = format!(
            "snapshot-{}-{}",
            last_applied.map_or(0, |l| l.index),
            uuid::Uuid::new_v4()
        );

        let data = Vec::new();
        let meta = SnapshotMeta {
            last_log_id: last_applied,
            last_membership,
            snapshot_id,
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        // Now safe to await
        let mut current = self.current_snapshot.write().await;
        *current = Some(snapshot);

        Ok(openraft::storage::Snapshot {
            meta,
            snapshot: Box::new(std::io::Cursor::new(data)),
        })
    }
}
