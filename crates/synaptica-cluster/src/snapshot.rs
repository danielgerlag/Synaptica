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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_snapshot() {
        let mut mgr = SnapshotManager::new();
        let snap = mgr.create_snapshot();
        assert!(snap.index > 0);
        assert!(snap.timestamp > 0);
    }

    #[test]
    fn test_sequential_snapshots_increment_index() {
        let mut mgr = SnapshotManager::new();
        let s1 = mgr.create_snapshot();
        let s2 = mgr.create_snapshot();
        let s3 = mgr.create_snapshot();
        assert!(s2.index > s1.index);
        assert!(s3.index > s2.index);
    }

    #[test]
    fn test_restore_snapshot() {
        let mut mgr = SnapshotManager::new();
        let snap = mgr.create_snapshot();
        let saved_index = snap.index;

        let mut mgr2 = SnapshotManager::new();
        mgr2.restore_snapshot(&snap);
        let next = mgr2.create_snapshot();
        assert!(next.index > saved_index);
    }

    #[test]
    fn test_snapshot_meta_serializable() {
        let meta = SnapshotMeta {
            term: 5,
            index: 42,
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: SnapshotMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.term, meta.term);
        assert_eq!(deserialized.index, meta.index);
        assert_eq!(deserialized.timestamp, meta.timestamp);
    }
}
