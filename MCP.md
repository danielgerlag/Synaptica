# Synaptica MCP — AI Agent Integration

Synaptica can be used as a tool by AI agents via the [Model Context Protocol (MCP)](https://modelcontextprotocol.io). Agents can query, create, update, and delete graph data using natural GQL queries.

## Modes

| Mode | Transport | Use Case |
|------|-----------|----------|
| **stdio** (default) | stdin/stdout | Agent spawns `synaptica-mcp` as a subprocess with an embedded database. No server needed. |
| **remote** | stdio (with gRPC backend) | Agent spawns `synaptica-mcp` which connects to a running Synaptica server. |

---

## Quick Setup

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "synaptica": {
      "command": "synaptica-mcp",
      "args": ["--data-dir", "/path/to/graph-data"]
    }
  }
}
```

### VS Code / GitHub Copilot

Add to your VS Code `settings.json`:

```json
{
  "mcp": {
    "servers": {
      "synaptica": {
        "command": "synaptica-mcp",
        "args": ["--data-dir", "/path/to/graph-data"]
      }
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json` in your project:

```json
{
  "mcpServers": {
    "synaptica": {
      "command": "synaptica-mcp",
      "args": ["--data-dir", "./graph-data"]
    }
  }
}
```

### Remote Mode (connect to existing server)

```json
{
  "mcpServers": {
    "synaptica": {
      "command": "synaptica-mcp",
      "args": ["--mode", "remote", "--server-url", "http://10.0.0.1:9090"]
    }
  }
}
```

---

## CLI Reference

```
synaptica-mcp [OPTIONS]

Options:
  --mode <MODE>              "stdio" (default) or "remote"
  --data-dir <DIR>           RocksDB data directory for local mode [default: ./data]
  --server-url <URL>         gRPC server URL for remote mode [default: http://localhost:9090]
  --graph <NAME>             Default graph name [default: default]
  -h, --help                 Print help
  -V, --version              Print version
```

---

## Available Tools

### `query`
Execute any GQL query (reads and writes).

**Parameters:**
- `query` (string, required) — The GQL query
- `graph` (string, optional) — Graph name

**Examples the agent might use:**
```
query: "INSERT (:Person {name: 'Alice', age: 30})"
query: "MATCH (n:Person) RETURN n.name, n.age"
query: "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) INSERT (a)-[:KNOWS]->(b)"
query: "MATCH (n:Person) WHERE n.age > 25 RETURN n.name ORDER BY n.age DESC"
```

### `get_schema`
Returns the graph schema — all node and edge labels with their property keys and counts.

**Parameters:**
- `graph` (string, optional)

### `list_labels`
Lists all node and edge labels with their counts.

**Parameters:**
- `graph` (string, optional)

### `list_indexes`
Lists all secondary property indexes.

**Parameters:**
- `graph` (string, optional)

### `create_index`
Creates a secondary property index for faster lookups.

**Parameters:**
- `label` (string, required) — Node label (e.g. "Person")
- `property` (string, required) — Property name (e.g. "name")
- `graph` (string, optional)

### `drop_index`
Drops a secondary property index.

**Parameters:**
- `label` (string, required)
- `property` (string, required)
- `graph` (string, optional)

### `health`
Returns database health status and version.

### `cluster_status`
Returns Raft cluster information (remote mode only).

---

## Local vs Remote Mode

| Feature | Local (stdio) | Remote |
|---------|--------------|--------|
| Setup | No server needed | Requires running Synaptica server |
| Data location | `--data-dir` on agent's machine | Server's configured storage |
| Performance | Direct RocksDB access | Network round-trip via gRPC |
| Cluster features | No | Yes (cluster_status, replicated writes) |
| Multi-agent access | Single process only | Multiple agents can share one server |
| Best for | Personal knowledge graphs, local dev | Shared team databases, production |

---

## GQL Quick Reference for Agents

The server instructions include a GQL syntax summary, but here's a concise reference:

```gql
-- Create nodes
INSERT (:Person {name: 'Alice', age: 30})

-- Create edges (MUST MATCH endpoints first)
MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'})
INSERT (a)-[:KNOWS {since: 2024}]->(b)

-- Read data
MATCH (n:Person) RETURN n.name, n.age
MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name, b.name

-- Filter and sort
MATCH (n:Person) WHERE n.age > 25 RETURN n.name ORDER BY n.age DESC LIMIT 10

-- Update
MATCH (n:Person {name: 'Alice'}) SET n.age = 31

-- Delete
MATCH (n:Person {name: 'Alice'}) DETACH DELETE n

-- Indexes
CREATE INDEX ON :Person(name)
DROP INDEX ON :Person(name)
```

---

## Building

```bash
cargo build --release -p synaptica-mcp
```

The binary is at `target/release/synaptica-mcp` (or `synaptica-mcp.exe` on Windows).

## Debugging

Set `RUST_LOG=debug` to see MCP protocol messages on stderr:

```json
{
  "mcpServers": {
    "synaptica": {
      "command": "synaptica-mcp",
      "args": ["--data-dir", "./data"],
      "env": { "RUST_LOG": "debug" }
    }
  }
}
```
