use serde::{Deserialize, Serialize};

/// Metadata for a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub term: u64,
    pub index: u64,
    pub timestamp: u64,
}

/// Placeholder snapshot manager.
#[derive(Debug, Default)]
pub struct SnapshotManager {
    last_index: u64,
}

impl SnapshotManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new snapshot. Stub: returns metadata with placeholder values.
    pub fn create_snapshot(&mut self) -> SnapshotMeta {
        self.last_index += 1;
        SnapshotMeta {
            term: 0,
            index: self.last_index,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Restore from a snapshot. Stub: records the index.
    pub fn restore_snapshot(&mut self, meta: &SnapshotMeta) {
        self.last_index = meta.index;
    }
}