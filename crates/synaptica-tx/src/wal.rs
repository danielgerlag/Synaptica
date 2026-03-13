// Write-ahead log integration.
// RocksDB provides its own WAL. This module adds transaction-level
// commit markers for crash recovery of multi-key transactions.

// In the current implementation, we leverage RocksDB's native WAL
// through WriteBatch atomicity. Multi-partition transaction recovery
// will be added in Phase 8 (distributed transactions).
