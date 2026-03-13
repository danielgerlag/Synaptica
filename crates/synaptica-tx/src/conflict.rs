use std::collections::HashSet;

/// Validates that a transaction has no write-write conflicts with
/// recently committed transactions.
///
/// Uses optimistic concurrency: all reads/writes proceed without locks,
/// conflicts are detected at commit time.
pub fn check_write_write_conflicts(
    tx_write_keys: &[(&str, &[u8])],
    committed_keys_since_snapshot: &[(&str, &[u8])],
) -> bool {
    let write_set: HashSet<(&str, &[u8])> = tx_write_keys.iter().copied().collect();

    for &(cf, key) in committed_keys_since_snapshot {
        if write_set.contains(&(cf, key)) {
            return true; // conflict found
        }
    }

    false
}
