# Synaptica

A high-performance distributed graph database written in Rust, implementing the [GQL ISO standard (ISO/IEC 39075:2024)](https://www.iso.org/standard/76120.html).

## Features

- **GQL Query Language** — Full parser for the ISO/IEC 39075:2024 Graph Query Language standard
- **Multi-Graph** — Create, list, switch, and drop independent graphs within a single server
- **RocksDB Storage** — High-performance embedded storage with column-family-based graph encoding
- **MVCC Transactions** — Snapshot isolation with optimistic concurrency control
- **Distributed Architecture** — Raft consensus, range-based partitioning, two-phase commit
- **gRPC API** — Client and inter-node communication via Protocol Buffers
- **Interactive CLI** — REPL client with syntax highlighting and tabular output

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Client (gRPC)                        │
├─────────────────────────────────────────────────────────┤
│                  GQL Parser & AST                       │
├─────────────────────────────────────────────────────────┤
│           Query Planner & Optimizer                     │
├─────────────────────────────────────────────────────────┤
│              Execution Engine                           │
├──────────────────────┬──────────────────────────────────┤
│  Transaction Manager │     Distributed Layer            │
│  (MVCC / Snapshot    │  (Raft + Range Partitioning +    │
│   Isolation)         │   Cluster Management)            │
├──────────────────────┴──────────────────────────────────┤
│               Storage Engine (RocksDB)                  │
└─────────────────────────────────────────────────────────┘
```

### Crate Structure

| Crate | Description |
|---|---|
| `synaptica-core` | Graph data model — Node, Edge, Label, Property, Value types |
| `synaptica-storage` | RocksDB engine, key encoding, column families, secondary indexes |
| `synaptica-tx` | MVCC transaction manager with snapshot isolation |
| `synaptica-gql` | GQL lexer, parser, AST, semantic analysis, query planner, optimizer |
| `synaptica-exec` | Volcano-model query execution engine |
| `synaptica-cluster` | Raft consensus, range partitioning, distributed transactions |
| `synaptica-server` | gRPC server, configuration, metrics |
| `synaptica-cli` | Interactive REPL client |
| `synaptica-mcp` | MCP server for AI agent integration (stdio & remote) |

## Quick Start

### Prerequisites

- Rust 1.75+ (2021 edition)
- C++ compiler (for RocksDB compilation)
- Protocol Buffers compiler (`protoc`)

### Build

```bash
cargo build --release
```

### Run the Server

```bash
# With default configuration
cargo run --release --bin synaptica-server

# With custom config
cargo run --release --bin synaptica-server -- --config synaptica.toml

# With CLI options
cargo run --release --bin synaptica-server -- --listen 0.0.0.0:9090 --data-dir ./mydata
```

### Connect with CLI

```bash
# Connect to local server
cargo run --release --bin synaptica-cli

# Connect to remote server
cargo run --release --bin synaptica-cli -- --host http://remote-host:9090

# Output in JSON format
cargo run --release --bin synaptica-cli -- --format json
```

### Example Session

```
synaptica> INSERT (:Person {name: 'Alice', age: 30})
Nodes created: 1

synaptica> INSERT (:Person {name: 'Bob', age: 25})
Nodes created: 1

synaptica> MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) \
         > INSERT (a)-[:KNOWS {since: 2020}]->(b)
Edges created: 1

synaptica> MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name, b.name, r.since
+---------+---------+---------+
| a.name  | b.name  | r.since |
+---------+---------+---------+
| Alice   | Bob     | 2020    |
+---------+---------+---------+
1 row(s) returned

synaptica> MATCH (n:Person) WHERE n.age > 20 RETURN n.name, n.age ORDER BY n.age DESC
+---------+-------+
| n.name  | n.age |
+---------+-------+
| Alice   | 30    |
| Bob     | 25    |
+---------+-------+
2 row(s) returned
```

## Configuration

Create a `synaptica.toml` file:

```toml
listen_addr = "0.0.0.0:9090"
data_dir = "./data"
default_graph = "default"
log_level = "info"
metrics_enabled = true
metrics_addr = "0.0.0.0:9091"

[cluster]
node_id = 1
cluster_addr = "0.0.0.0:9191"

[[cluster.peers]]
node_id = 2
address = "10.0.0.2:9191"

[[cluster.peers]]
node_id = 3
address = "10.0.0.3:9191"
```

## GQL Support

Synaptica implements the ISO/IEC 39075:2024 GQL standard. Currently supported:

### Statements
- `MATCH` — Graph pattern matching with node/edge patterns
- `RETURN` — Result projection with expressions, aliases, `DISTINCT`
- `INSERT` — Create nodes and edges with labels and properties
- `SET` — Update node/edge properties
- `DELETE` / `DETACH DELETE` — Remove nodes and edges
- `REMOVE` — Remove properties and labels
- `CREATE GRAPH` / `DROP GRAPH` / `LIST GRAPHS` — Multi-graph management
- `CREATE GRAPH TYPE` — Schema definitions

### Clauses
- `WHERE` — Filter with boolean expressions
- `ORDER BY` — Sort results (ASC/DESC)
- `LIMIT` / `OFFSET` — Pagination
- `GROUP BY` / `HAVING` — Aggregation
- `AS` — Column aliases

### Expressions
- Arithmetic: `+`, `-`, `*`, `/`, `%`
- Comparison: `=`, `<>`, `<`, `>`, `<=`, `>=`
- Boolean: `AND`, `OR`, `NOT`, `XOR`
- String: `||` (concatenation), `LIKE`
- Null: `IS NULL`, `IS NOT NULL`
- Collections: `IN`, list/map literals
- Functions: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `COLLECT`
- Built-ins: `toString()`, `toInteger()`, `toFloat()`, `size()`, `keys()`, `labels()`, `type()`, `id()`
- `CASE WHEN ... THEN ... ELSE ... END`
- `EXISTS` subqueries

### Graph Patterns
- Node patterns: `(n:Label {prop: value})`
- Edge patterns: `-[r:LABEL]->`, `<-[r]-`, `-[r]-`
- Path modes: `WALK`, `TRAIL`, `SIMPLE`, `ACYCLIC`
- Variable-length paths: `-[r:KNOWS*1..5]->`
- Shortest path: `SHORTEST`, `ALL SHORTEST`

### Set Operations
- `UNION` / `UNION ALL`
- `INTERSECT`
- `EXCEPT`

## Storage Engine

Synaptica uses RocksDB with a carefully designed key layout:

| Column Family | Key Format | Purpose |
|---|---|---|
| `nodes` | `graph_id \|\| node_id` | Node data |
| `edges` | `graph_id \|\| edge_id` | Edge data |
| `adj_out` | `graph_id \|\| src_id \|\| label \|\| edge_id` | Outgoing adjacency |
| `adj_in` | `graph_id \|\| tgt_id \|\| label \|\| edge_id` | Incoming adjacency |
| `node_labels` | `graph_id \|\| label \|\| node_id` | Label → node index |
| `edge_labels` | `graph_id \|\| label \|\| edge_id` | Label → edge index |
| `prop_index` | `graph_id \|\| index_hash \|\| value \|\| id` | Property indexes |
| `graph_meta` | `graph_id` | Graph metadata |

All keys use big-endian encoding for correct lexicographic ordering, enabling efficient range scans and prefix-based iteration.

## Transaction Model

- **MVCC** — Multi-Version Concurrency Control with versioned keys
- **Snapshot Isolation** — Each transaction reads from a consistent snapshot
- **Optimistic Concurrency** — Write-write conflicts detected at commit time
- **Read-Your-Own-Writes** — Transactions see their own uncommitted changes
- **Distributed 2PC** — Two-phase commit for cross-partition transactions

## Multi-Graph Support

Synaptica supports multiple independent graphs within a single server instance. Each graph has fully isolated nodes, edges, indexes, and properties.

### GQL Statements

```sql
-- Create a new graph
CREATE GRAPH myGraph

-- Create only if it doesn't exist
CREATE GRAPH IF NOT EXISTS myGraph

-- List all graphs
LIST GRAPHS

-- Drop a graph (removes all data)
DROP GRAPH myGraph

-- Drop only if it exists
DROP GRAPH IF EXISTS myGraph
```

### CLI Usage

```bash
# Connect to a specific graph
synaptica-cli --addr http://127.0.0.1:50051 --graph myGraph

# List graphs from the REPL
:graphs
```

### Web UI

The web UI includes a graph selector dropdown in the header. You can switch between graphs and create new ones inline.

### MCP / AI Agent Integration

MCP tools accept an optional `graph` parameter:

- **`list_graphs`** — List all available graphs
- **`create_graph`** — Create a new graph by name
- **`query`** — Run a GQL query against a specific graph (use the `graph` parameter)

## Cluster Mode

Synaptica supports distributed deployment with:

- **Raft Consensus** — Leader election and log replication via `openraft`
- **Automatic Failover** — Leader crash detected in 1.5–3 s, new leader elected automatically
- **Write Replication** — All mutations committed to majority before acknowledgment
- **Local Reads** — Queries execute on any node for low-latency reads

See **[CLUSTER.md](CLUSTER.md)** for the full cluster operations guide, including setup instructions, configuration reference, failure scenarios, and recovery time benchmarks.

## AI Agent Integration (MCP)

Synaptica can be used as a tool by AI agents (Claude, Copilot, Cursor, etc.) via the [Model Context Protocol](https://modelcontextprotocol.io).

```json
{
  "mcpServers": {
    "synaptica": {
      "command": "synaptica-mcp",
      "args": ["--data-dir", "./my-graph"]
    }
  }
}
```

Tools available: `query`, `get_schema`, `list_labels`, `list_indexes`, `create_index`, `drop_index`, `health`, `cluster_status`.

See **[MCP.md](MCP.md)** for setup instructions, remote mode configuration, and the full tool reference.

## Development

```bash
# Run all tests
cargo test --workspace

# Run specific crate tests
cargo test -p synaptica-storage
cargo test -p synaptica-gql
cargo test -p synaptica-tx

# Run integration tests
cargo test --test gql_compliance_tests

# Build in release mode
cargo build --release

# Check for issues
cargo clippy --workspace
```

## License

MIT
