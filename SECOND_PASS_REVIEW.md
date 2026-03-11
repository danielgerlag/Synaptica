# Synaptica — Second-Pass Code Review

> **Date**: 2025-07-16  
> **Scope**: All 8 crates, full source read  
> **Focus**: Bugs missed by first-pass review, issues introduced by the 6 recent fixes

---

## Summary

**15 issues found** — 3 High, 8 Medium, 4 Low.  
Two first-pass fixes (self-loop dedup, parking_lot migration) were verified as correct.

---

## HIGH Severity

### H-1. Parser: Parenthesized expressions have no recursion depth guard — stack overflow

**File**: `crates/synaptica-gql/src/parser.rs:1035–1039`

`parse_primary` handles `Token::LParen` by calling `self.parse_expression()` with no `enter_depth()`/`exit_depth()`. The call path `parse_expression → parse_or_expr → … → parse_primary` re-enters itself for each `(…)`, so the input `((((…))))` nested ~5000 levels deep will overflow the stack and crash the server.

The parser correctly depth-guards `parse_unary_expr` (line 983) and `parse_not_expr`, but not this path.

```rust
// Current (line 1035–1039):
Token::LParen => {
    self.advance();
    let expr = self.parse_expression()?;
    self.expect(&Token::RParen)?;
    Ok(expr)
}
```

**Fix**: Add depth tracking:
```rust
Token::LParen => {
    self.advance();
    self.enter_depth()?;
    let expr = self.parse_expression()?;
    self.exit_depth();
    self.expect(&Token::RParen)?;
    Ok(expr)
}
```

Same issue applies to:
- **EXISTS subquery** (line 1087–1094): `EXISTS { MATCH … WHERE EXISTS { … } }` recurses through `parse_statement → parse_expression → parse_primary` without depth tracking.
- **CASE expressions** (line 1157–1180): `CASE WHEN CASE WHEN … END END` recurses via `parse_expression()` calls with no depth guard.
- **List/map literals** (lines 1041–1084): `[[[…]]]` and `{a: {b: …}}` recurse without depth tracking.

---

### H-2. `exec_expand` follows undirected edges back to the starting node

**File**: `crates/synaptica-exec/src/engine.rs:264–267`

For `Direction::Undirected`, the code collects both outgoing and incoming edges (lines 249–260), deduplicating self-loops. But the target-node resolution always uses `edge.target` for both `Outgoing` and `Undirected`:

```rust
let target_id = match direction {
    Direction::Incoming => edge.source,
    _ => edge.target,       // ← Undirected falls here
};
```

For incoming edges added to the undirected set, `edge.source` is the neighbor and `edge.target` is the current node. Using `edge.target` navigates back to the same node instead of the neighbor, making undirected graph traversal return wrong results for edges that were only in the incoming set.

**Fix**:
```rust
let target_id = match direction {
    Direction::Incoming => edge.source,
    Direction::Outgoing => edge.target,
    Direction::Undirected => {
        if edge.source == source_id { edge.target } else { edge.source }
    }
};
```

---

### H-3. `encode_label` panics on user-controlled input > 65535 bytes

**File**: `crates/synaptica-storage/src/encoding.rs:19–23`

```rust
fn encode_label(label: &str, buf: &mut Vec<u8>) {
    let bytes = label.as_bytes();
    assert!(bytes.len() <= u16::MAX as usize, …);
    …
}
```

Labels come from user GQL queries (e.g., `CREATE (:VeryLongLabel…)`). A label exceeding 65535 bytes will `panic!` and crash the server. This should return a `Result` or `StorageError` instead.

**Fix**: Change `assert!` to a proper error return:
```rust
fn encode_label(label: &str, buf: &mut Vec<u8>) -> Result<(), StorageError> {
    let bytes = label.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(StorageError::InvalidData(
            format!("label exceeds maximum length of {} bytes", u16::MAX)
        ));
    }
    …
}
```
Update all callers (`encode_node_key`, `encode_node_label_key`, `encode_edge_adj_key`, etc.) to propagate the error.

---

## MEDIUM Severity

### M-1. Planner silently drops ORDER BY and LIMIT on MATCH statements

**File**: `crates/synaptica-gql/src/planner.rs:229–236`

The parser correctly parses `order_by` and `limit_offset` on MATCH statements (parser.rs lines 182–188, stored in the `MatchStatement` AST node), but `plan_statement` for `GqlStatement::Match` only reads the `WHERE` clause and ignores them entirely:

```rust
GqlStatement::Match(m) => {
    // … processes patterns and where_clause …
    // m.order_by — ignored
    // m.limit_offset — ignored
    Ok(plan)
}
```

A query like `MATCH (n:Person) ORDER BY n.age LIMIT 10 RETURN n` silently returns all results unordered.

**Fix**: Add `Sort` and `Limit` plan nodes in the MATCH planning arm, same as done for RETURN (lines 255–265).

---

### M-2. `ResultSet::add_record` uses `debug_assert` — no validation in release builds

**File**: `crates/synaptica-exec/src/result.rs:40–47`

```rust
pub fn add_record(&mut self, values: Vec<Value>) {
    debug_assert_eq!(values.len(), self.columns.len(), …);
    self.records.push(Record { columns: self.columns.clone(), values });
}
```

In release builds, `debug_assert_eq!` is a no-op. If a record with the wrong number of values is added, `Record::get()` (line 21: `&self.values[i]`) will panic with index-out-of-bounds when accessing a column whose index exceeds `values.len()`.

**Fix**: Use a real assertion or return `Result`:
```rust
if values.len() != self.columns.len() {
    return Err(ExecError::Internal(format!(
        "record has {} values but {} columns", values.len(), self.columns.len()
    )));
}
```

---

### M-3. `exec_join` produces a full cartesian product — incorrect for pattern matching

**File**: `crates/synaptica-exec/src/engine.rs:482–498`

The join implementation is an unconditional cross-product of all left × right records:

```rust
for l in &left.records {
    for r in &right.records {
        let mut values = l.values.clone();
        values.extend(r.values.clone());
        rs.add_record(values);
    }
}
```

For GQL patterns like `MATCH (a)-[:KNOWS]->(b), (b)-[:LIKES]->(c)`, the second clause should join on the shared variable `b`, but the current implementation produces `|left| × |right|` results with no predicate filtering. Two scans of N nodes each produce N² results instead of at most N.

**Fix**: Implement equi-join by identifying shared column names between left and right, then filtering on equality.

---

### M-4. WAL `persist_entry` silently swallows write and sync failures

**File**: `crates/synaptica-cluster/src/distributed_tx.rs:138–144`

```rust
fn persist_entry(&mut self, entry: &LogEntry) {
    if let Some(ref mut file) = self.wal_file {
        if let Ok(json) = serde_json::to_string(entry) {
            let _ = writeln!(file, "{}", json);      // write error ignored
            let _ = file.sync_data();                 // sync error ignored
        }
    }
}
```

For a 2PC coordinator, WAL durability is critical. If the write or sync fails (disk full, I/O error) but the coordinator proceeds to send prepare/commit messages, crash recovery will have an incomplete log. This can lead to stuck "in-doubt" transactions that can never be resolved.

**Fix**: Return `Result` and propagate errors to callers (`log_prepare`, `log_commit`, `log_abort`):
```rust
fn persist_entry(&mut self, entry: &LogEntry) -> Result<(), DistributedTxError> {
    if let Some(ref mut file) = self.wal_file {
        let json = serde_json::to_string(entry)
            .map_err(|e| DistributedTxError::CoordinatorFailed(e.to_string()))?;
        writeln!(file, "{}", json)
            .map_err(|e| DistributedTxError::CoordinatorFailed(e.to_string()))?;
        file.sync_data()
            .map_err(|e| DistributedTxError::CoordinatorFailed(e.to_string()))?;
    }
    Ok(())
}
```

---

### M-5. `hash_index_name` uses `DefaultHasher` — non-cryptographic collision risk

**File**: `crates/synaptica-storage/src/index.rs:42–46`

```rust
fn hash_index_name(name: &str) -> [u8; HASH_LEN] {
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    h.finish().to_be_bytes()
}
```

Index names are hashed into an 8-byte prefix that forms the column-family key namespace. Two different index names hashing to the same u64 would silently merge their index data, causing query corruption. `DefaultHasher` is SipHash which is decent, but:
1. The hash is NOT salted/randomized per instance (same hash across process restarts).
2. 8 bytes gives ~birthday-bound collision at ~4 billion indexes — unlikely but the failure mode is silent data corruption.

**Fix**: Use the full index name as the key prefix (length-prefixed), or store an index registry that detects collisions. Alternatively, use a 16-byte hash (e.g., blake3 or SipHash128) to make collisions vanishingly unlikely.

---

### M-6. Tombstone value `[0x00]` is an unsafe sentinel — no API guard

**File**: `crates/synaptica-storage/src/mvcc.rs:117, 148–162`

```rust
const TOMBSTONE: &[u8] = &[0x00];
```

`put_at` accepts any `&[u8]` value and stores it directly. If any caller passes `&[0x00]` as a value, `get_at` and `prefix_scan_at` will treat it as a deletion (tombstone), silently "deleting" the record. Current callers use bincode-serialized structs (always >1 byte) or empty slices, so no active code path triggers this. But the API is a trap for future callers.

**Fix**: Either validate in `put_at`:
```rust
if value == TOMBSTONE {
    return Err(StorageError::InvalidData("value collides with tombstone marker".into()));
}
```
Or use an out-of-band tombstone mechanism (separate CF, or length-distinguished marker like empty `&[]`).

---

### M-7. Static file server reads only 4096 bytes of HTTP request

**File**: `crates/synaptica-server/src/main.rs:154`

```rust
let mut buf = [0u8; 4096];
let n = match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await { … };
```

HTTP requests with large headers (e.g., large cookies, many headers, long Authorization tokens) can exceed 4096 bytes. The truncation may cut the request line itself, causing path extraction to fail and return a 404 for a valid request.

**Fix**: Use a larger buffer (8192 or 16384 is standard for HTTP servers) or use an incremental read loop that finds the end of the request line:
```rust
let mut buf = [0u8; 8192];
```

---

### M-8. Fulltext entry key uses `0x00` separator inside variable-length token

**File**: `crates/synaptica-storage/src/fulltext.rs:70–78`

```rust
k.extend_from_slice(tb);    // token bytes (variable length)
k.push(0x00);               // separator
k.extend_from_slice(node_id.as_bytes());  // 16 bytes UUID
```

The key format `graph_id ++ FT_MARKER ++ name_hash ++ token_bytes ++ 0x00 ++ node_id` uses `0x00` as a separator between the variable-length token and the fixed-length node_id. If a token ever contains `0x00`, the key becomes ambiguous — prefix scans in `search()` (which use `token_prefix`) would not match correctly.

In practice, `tokenize()` splits on non-alphanumeric characters and lowercases, so tokens won't contain NUL bytes. But the encoding is fragile by design.

**Fix**: Since node_id is always exactly 16 bytes (UUID), the separator is unnecessary. Extract the node_id from the last 16 bytes of the key instead:
```rust
let node_id_bytes = &key[key.len() - UUID_LEN..];
```

---

## LOW Severity

### L-1. `Duration::Display` outputs "P" for zero duration (ISO 8601 violation)

**File**: `crates/synaptica-core/src/types.rs:186–206`

When `months=0, days=0, nanos=0`, the format produces just `"P"` with no duration components. ISO 8601 requires at least one component; the canonical zero duration is `"PT0S"`.

**Fix**: Add a zero check at the end:
```rust
if self.months == 0 && self.days == 0 && self.nanos == 0 {
    write!(f, "PT0S")?;
} else {
    // existing formatting logic
}
```

---

### L-2. TimestampOracle key stored in default CF without namespace isolation

**File**: `crates/synaptica-storage/src/mvcc.rs:31`

```rust
const TS_ORACLE_KEY: &[u8] = b"__ts_oracle_counter__";
```

The timestamp oracle uses `db.put()` (default CF, no versioning suffix), while MVCC data in the same default CF uses versioned keys. A hypothetical user key whose prefix matches this 21-byte ASCII string could collide. In practice, MVCC keys are UUID-based binary, so collision is extremely unlikely — but the lack of namespace separation is a design smell.

**Fix**: Use a dedicated system CF, or prefix with a non-UTF8 byte marker (e.g., `0xFF`) that can't appear in normal key encodings.

---

### L-3. `NaN` sorts equal to any value in `cmp_values`

**File**: `crates/synaptica-exec/src/engine.rs` (sort comparison logic)

The value comparison for `ORDER BY` sorting uses:
```rust
(Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
```

`NaN.partial_cmp(anything)` returns `None`, which becomes `Ordering::Equal`. This means `NaN` sorts as equal to every other float, producing unpredictable sort order. SQL/GQL convention is to sort NaN values to the end (NULLS LAST equivalent).

**Fix**: Replace `unwrap_or(Ordering::Equal)` with a total ordering:
```rust
x.partial_cmp(y).unwrap_or_else(|| {
    match (x.is_nan(), y.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,  // NaN sorts last
        (false, true) => Ordering::Less,
        _ => unreachable!(),
    }
})
```

---

### L-4. Parser allows unbounded UNION/INTERSECT/EXCEPT nesting

**File**: `crates/synaptica-gql/src/parser.rs` (`maybe_parse_composite`)

`maybe_parse_composite` recursively calls `parse_statement()` to parse the right-hand side of UNION/INTERSECT/EXCEPT, and the result can itself be a composite. With 10000+ chained UNION clauses, this creates deep right-recursive call chains that could overflow the stack. This is a lower-risk variant of H-1 since UNION queries of that depth are unusual.

---

## Verification of First-Pass Fixes

| Fix | Status | Notes |
|-----|--------|-------|
| `GraphId::from_name()` | ✅ Correct | Deterministic UUID v5 from name string |
| TimestampOracle persistence | ✅ Correct | Persists to RocksDB, loads on startup |
| Tombstone change | ✅ Correct | `[0x00]` doesn't collide with bincode-serialized structs |
| `delete_node` cascade | ✅ Correct | `HashSet` dedup at line 189 prevents self-loop double-delete |
| String index encoding | ✅ Correct | NUL-escape in `encode_value_comparable` handles `\x00` bytes |
| `parking_lot::Mutex` replacement | ✅ Correct | `distributed_tx.rs` also migrated; no `.unwrap()` on lock |

All 6 first-pass fixes are properly implemented and introduce no new issues.

---

## Issue Tally by Crate

| Crate | Issues |
|-------|--------|
| `synaptica-gql` (parser/planner) | H-1, M-1, L-4 |
| `synaptica-exec` (engine/result) | H-2, M-2, M-3, L-3 |
| `synaptica-storage` (encoding/mvcc/index/fulltext) | H-3, M-5, M-6, M-8, L-2 |
| `synaptica-cluster` (distributed_tx) | M-4 |
| `synaptica-server` (main) | M-7 |
| `synaptica-core` (types) | L-1 |
