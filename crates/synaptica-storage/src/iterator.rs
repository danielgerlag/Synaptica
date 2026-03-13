use crate::engine::{StorageEngine, StorageResult};
use rocksdb::IteratorMode;

/// Iterator over nodes in a graph, providing streaming access.
/// Prefix scan helper for raw key-value pairs.
pub fn prefix_scan(
    engine: &StorageEngine,
    cf_name: &str,
    prefix: &[u8],
) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
    let cf = engine.raw_db().cf_handle(cf_name).ok_or_else(|| {
        crate::engine::StorageError::Internal(format!("column family not found: {}", cf_name))
    })?;

    let iter = engine
        .raw_db()
        .iterator_cf(&cf, IteratorMode::From(prefix, rocksdb::Direction::Forward));

    let mut results = Vec::new();
    for item in iter {
        let (key, value) = item?;
        if !key.starts_with(prefix) {
            break;
        }
        results.push((key.to_vec(), value.to_vec()));
    }
    Ok(results)
}
