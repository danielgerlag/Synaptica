# Synaptica — Cluster Mode Guide

Synaptica can run as a distributed cluster using **Raft consensus** (via [openraft](https://github.com/databendlabs/openraft)). Every write is replicated to a majority of nodes before being acknowledged, giving you strong consistency for mutations and automatic leader failover.

---

## Table of Contents

- [Overview](#overview)
- [Quick Start: 3-Node Cluster](#quick-start-3-node-cluster)
- [Architecture](#architecture)
- [Configuration Reference](#configuration-reference)
- [CLI Flags](#cli-flags)
- [Operations](#operations)
  - [Bootstrap a Cluster](#bootstrap-a-cluster)
  - [Add a Node](#add-a-node)
  - [Remove a Node](#remove-a-node)
  - [Check Cluster Status](#check-cluster-status)
- [Read and Write Routing](#read-and-write-routing)
- [Failure Scenarios](#failure-scenarios)
- [Recovery Time Characteristics](#recovery-time-characteristics)
- [Tuning](#tuning)
- [Limitations](#limitations)

---

## Overview

| Aspect | Detail |
|---|---|
| Consensus | Raft (openraft v0.9) |
| Replication | Synchronous — committed after majority ack |
| Reads | Local (eventual consistency) |
| Writes | Leader only; followers reject writes |
| Storage | RocksDB per node (each node has a full copy) |
| Inter-node transport | gRPC (tonic) on a dedicated cluster port |
| Serialization | bincode for Raft messages |

Cluster mode is activated by providing cluster configuration (via config file or CLI flags). Without cluster configuration, Synaptica runs as a standalone single-node server.

---

## Quick Start: 3-Node Cluster

The fastest way to try cluster mode locally — three nodes on `localhost` with different ports and data directories.

### Using CLI flags

Open three terminals:

```bash
# Terminal 1 — Node 1 (will bootstrap as leader)
cargo run --release --bin synaptica-server -- \
  --node-id 1 \
  --listen    127.0.0.1:9090 \
  --cluster-addr 127.0.0.1:9191 \
  --data-dir ./data/node1

# Terminal 2 — Node 2
cargo run --release --bin synaptica-server -- \
  --node-id 2 \
  --listen    127.0.0.1:9091 \
  --cluster-addr 127.0.0.1:9192 \
  --data-dir ./data/node2 \
  --peer 1=127.0.0.1:9191

# Terminal 3 — Node 3
cargo run --release --bin synaptica-server -- \
  --node-id 3 \
  --listen    127.0.0.1:9093 \
  --cluster-addr 127.0.0.1:9193 \
  --data-dir ./data/node3 \
  --peer 1=127.0.0.1:9191 \
  --peer 2=127.0.0.1:9192
```

**Node 1** starts first with no peers, so it bootstraps itself as the sole voter and becomes leader. Nodes 2 and 3 connect to the existing members.

### Using a config file

```toml
# node1.toml
listen_addr   = "127.0.0.1:9090"
data_dir      = "./data/node1"
default_graph = "default"

[cluster]
node_id      = 1
cluster_addr = "127.0.0.1:9191"
peers        = []

# node2.toml
listen_addr   = "127.0.0.1:9091"
data_dir      = "./data/node2"
default_graph = "default"

[cluster]
node_id      = 2
cluster_addr = "127.0.0.1:9192"

[[cluster.peers]]
node_id = 1
address = "127.0.0.1:9191"

# node3.toml
listen_addr   = "127.0.0.1:9093"
data_dir      = "./data/node3"
default_graph = "default"

[cluster]
node_id      = 3
cluster_addr = "127.0.0.1:9193"

[[cluster.peers]]
node_id = 1
address = "127.0.0.1:9191"

[[cluster.peers]]
node_id = 2
address = "127.0.0.1:9192"
```

Start each node with `--config`:

```bash
cargo run --release --bin synaptica-server -- --config node1.toml
cargo run --release --bin synaptica-server -- --config node2.toml
cargo run --release --bin synaptica-server -- --config node3.toml
```

### Verify the cluster

Connect the CLI to any node and run a write:

```bash
cargo run --release --bin synaptica-cli -- --host http://127.0.0.1:9090

synaptica> INSERT (:Person {name: 'Alice', age: 30})
Nodes created: 1
```

Then read from a different node:

```bash
cargo run --release --bin synaptica-cli -- --host http://127.0.0.1:9091

synaptica> MATCH (n:Person) RETURN n.name, n.age
+---------+-------+
| n.name  | n.age |
+---------+-------+
| Alice   | 30    |
+---------+-------+
```

The data was written via Raft on the leader and replicated to all followers.

---

## Architecture

```
                  ┌──────────────┐
    Clients ─────►│  Node 1      │
                  │  (Leader)    │
                  │  :9090 gRPC  │◄────── Client API (gRPC / gRPC-Web)
                  │  :9191 Raft  │◄─┐
                  └──────┬───────┘  │
                         │          │     Inter-node Raft RPCs
              ┌──────────┼──────────┤     (Vote, AppendEntries, InstallSnapshot)
              │          │          │
     ┌────────▼──┐  ┌────▼───────┐ │
     │  Node 2   │  │  Node 3    │ │
     │ (Follower) │  │ (Follower) │─┘
     │  :9091    │  │  :9093     │
     │  :9192    │  │  :9193     │
     └───────────┘  └────────────┘
```

Each node runs two gRPC servers:

| Port | Service | Purpose |
|------|---------|---------|
| Client port (`--listen`) | `SynapticaService` | Client queries, schema, health, metrics |
| Cluster port (`--cluster-addr`) | `ClusterService` | Raft protocol (Vote, AppendEntries, InstallSnapshot), write forwarding |

### Write path

1. Client sends a GQL write (`INSERT`, `SET`, `DELETE`, etc.) to any node.
2. If the node **is the leader**, it proposes the write to the Raft log.
3. The entry is replicated to followers; once a **majority acknowledges**, it is committed.
4. Each node applies the committed entry to its local RocksDB via the state machine applier.
5. The leader returns the result to the client.

If a client sends a write to a **follower**, the request is rejected with an error. Clients should retry on the leader. (The leader's identity is available via the `ClusterStatus` RPC.)

### Read path

Reads (`MATCH ... RETURN ...`) are executed **locally** on whatever node the client connects to. This means reads are fast (no consensus round-trip) but provide **eventual consistency** — a follower may lag the leader by one or two heartbeat intervals.

---

## Configuration Reference

Full `synaptica.toml` with all options:

```toml
# ─── Server ──────────────────────────────────────────────────────────
listen_addr   = "0.0.0.0:9090"       # Client gRPC listen address
data_dir      = "./data"              # RocksDB data directory
default_graph = "default"             # Default graph name
log_level     = "info"                # Logging level (trace, debug, info, warn, error)

# ─── Metrics ─────────────────────────────────────────────────────────
metrics_enabled = true                # Enable Prometheus /metrics endpoint
metrics_addr    = "0.0.0.0:9091"      # Prometheus endpoint listen address

# ─── TLS (optional) ─────────────────────────────────────────────────
[tls]
cert_path    = "/etc/synaptica/cert.pem"
key_path     = "/etc/synaptica/key.pem"
ca_cert_path = "/etc/synaptica/ca.pem"   # Optional — enables mutual TLS

# ─── Auth (optional) ────────────────────────────────────────────────
[auth]
enabled = true
tokens  = ["my-secret-token"]

# ─── Cluster (optional — enables cluster mode) ─────────────────────
[cluster]
node_id      = 1                         # Unique node ID (u64, must differ per node)
cluster_addr = "0.0.0.0:9191"            # Inter-node gRPC listen address

[[cluster.peers]]
node_id = 2
address = "10.0.0.2:9191"

[[cluster.peers]]
node_id = 3
address = "10.0.0.3:9191"
```

### Key rules

- **`node_id`** must be unique across all cluster members and is a positive integer (`u64`).
- **`cluster_addr`** is the address other nodes connect *to* for Raft RPCs — make sure it's reachable from peers.
- **Peers** are listed with their `node_id` and the `address` matching their `cluster_addr`.
- The first node (with no peers) auto-bootstraps as a single-voter cluster and becomes leader immediately.

---

## CLI Flags

All cluster settings can be provided via CLI flags, which **override** config file values:

```
synaptica-server [OPTIONS]

Options:
  --config <PATH>           Path to TOML configuration file
  --listen <ADDR>           Client gRPC listen address [default: 0.0.0.0:9090]
  --data-dir <DIR>          RocksDB storage directory [default: ./data]
  --ui-dir <PATH>           Path to UI static files (enables web UI)
  --ui-addr <ADDR>          UI server listen address [default: 0.0.0.0:8080]
  --node-id <ID>            Cluster node ID (u64)
  --cluster-addr <ADDR>     Inter-node gRPC listen address [default: 0.0.0.0:9191]
  --peer <ID=ADDR>          Peer node (repeatable), e.g. --peer 2=10.0.0.2:9191
```

**Cluster mode activation:** Cluster mode is enabled if *any* of these conditions are true:
- A `[cluster]` section exists in the config file
- `--node-id` is specified on the command line
- `--cluster-addr` is specified on the command line
- One or more `--peer` flags are provided

When mixing CLI flags and config file, CLI flags take precedence. Peer lists from both sources are merged (deduplicated by node ID).

---

## Operations

### Bootstrap a Cluster

**Step 1 — Start the first node with no peers:**

```bash
synaptica-server --node-id 1 --cluster-addr 0.0.0.0:9191 --data-dir ./data1
```

This node bootstraps a single-voter Raft group and becomes leader immediately.

**Step 2 — Start additional nodes with `--peer` pointing to existing members:**

```bash
synaptica-server --node-id 2 --cluster-addr 0.0.0.0:9192 --data-dir ./data2 \
  --peer 1=10.0.0.1:9191

synaptica-server --node-id 3 --cluster-addr 0.0.0.0:9193 --data-dir ./data3 \
  --peer 1=10.0.0.1:9191 --peer 2=10.0.0.2:9192
```

### Add a Node

To add a new node to a running cluster:

1. Start the new node with `--peer` pointing to existing members.
2. From the leader, issue an `AddLearner` request (via the cluster gRPC service) to register the new node as a non-voting learner.
3. Once the learner has caught up with the log, issue a `ChangeMembership` request to promote it to a voter.

The `AddLearner` and `ChangeMembership` RPCs are defined in the `ClusterService` (see [gRPC Services](#grpc-services)).

### Remove a Node

To remove a node from the voting set, issue a `ChangeMembership` request with the updated list of voter IDs (excluding the node to remove). The removed node will stop receiving new log entries and can be shut down.

### Check Cluster Status

Use the `ClusterStatus` RPC or the web UI to inspect:
- Current leader node ID
- This node's role (leader / follower)
- All known cluster members
- Partition count

```bash
# Via the CLI (connect to any node)
synaptica-cli --host http://127.0.0.1:9090

# Or via gRPC directly (grpcurl example)
grpcurl -plaintext 127.0.0.1:9090 synaptica.SynapticaService/ClusterStatus
```

---

## gRPC Services

### Client Service — `SynapticaService`

Exposed on the client port (`--listen`). Used by application clients, the CLI, and the web UI.

| RPC | Description |
|-----|-------------|
| `ExecuteQuery` | Execute a GQL query (read or write) |
| `Health` | Health check — returns `"ok"` and version |
| `ClusterStatus` | Get leader ID, node role, and member list |
| `ListLabels` | List node/edge labels with counts |
| `GetSchema` | Get labels and property keys |
| `ListIndexes` | List secondary indexes |
| `CreateIndex` | Create a secondary property index |
| `DropIndex` | Drop a secondary property index |
| `GetMetrics` | Get Prometheus-format metrics |

### Cluster Service — `ClusterService`

Exposed on the cluster port (`--cluster-addr`). Used for inter-node Raft communication.

| RPC | Description |
|-----|-------------|
| `Vote` | Raft leader election vote request/response |
| `AppendEntries` | Raft log replication and heartbeats |
| `InstallSnapshot` | Snapshot transfer to lagging followers |
| `ForwardWrite` | Forward a write from follower to leader |
| `AddLearner` | Add a non-voting replica to the cluster |
| `ChangeMembership` | Change the voting member set |

---

## Failure Scenarios

Synaptica handles the following failure modes automatically:

### Leader crash

When the leader goes down, followers detect the absence of heartbeats. After the election timeout expires (1.5–3 seconds with default settings), a follower calls a new election. The new leader begins accepting writes once elected.

**Requires:** A majority of nodes must still be alive. A 3-node cluster tolerates 1 failure; a 5-node cluster tolerates 2.

### Follower crash

The leader continues operating with the remaining majority. When the follower comes back online, the leader detects it on the next heartbeat and replays any missed log entries to bring it up to date.

### Network partition

If the leader is partitioned away from the majority, the majority elects a new leader. The old leader detects it has lost quorum and steps down. When the partition heals, the stale leader recognizes the new leader and becomes a follower, catching up on missed entries.

### Total cluster restart

When all nodes restart simultaneously, no state is lost — the Raft log and metadata are persisted in RocksDB. Nodes hold a new election and resume operation within a few seconds.

---

## Recovery Time Characteristics

The following measurements were taken using the in-process test harness with zero network latency. Production times will be higher depending on network RTT — expect roughly 2–10x these values on a LAN and 10–50x over WAN.

| Scenario | Median | P95 | Notes |
|---|---|---|---|
| Cold start election (3 nodes) | ~290 ms | ~580 ms | First leader elected after cluster creation |
| Cold start election (5 nodes) | ~440 ms | ~450 ms | More nodes → slightly more time |
| Re-election after leader crash | ~1.1 s | ~1.1 s | Dominated by election timeout (1.5–3 s default) |
| Write commit latency (3 nodes) | ~1.6 ms | ~2.2 ms | Steady-state, single INSERT via Raft |
| Write commit latency (5 nodes) | ~2.5 ms | ~3.2 ms | More voters → slightly higher latency |
| Follower catch-up (1 missed entry) | ~520 ms | ~520 ms | Next heartbeat triggers catch-up |
| Follower catch-up (50 entries) | ~460 ms | ~460 ms | Catch-up is heartbeat-bound, not count-bound |
| Follower catch-up (500 entries) | ~520 ms | ~1.1 s | Larger payloads may need extra round-trips |
| Write availability after crash | ~1.1 s | ~1.1 s | Time until new leader accepts first write |
| Full recovery cycle | ~2.1 s | ~2.3 s | Leader crash → new election → write → old leader rejoins |
| Stale leader step-down | ~230 ms | ~250 ms | Partitioned leader recognizes new leader |
| New learner catch-up (100 entries) | ~99 ms | ~104 ms | Log replay to a freshly added node |
| Cluster reform after total partition | ~160 ms | ~470 ms | All nodes reconnect after full partition |

**Key insights:**
- **Election timeout** is the dominant factor in failover time. Reduce `election_timeout_min/max` for faster failover at the cost of more spurious elections.
- **Follower catch-up** is bounded by heartbeat interval, not by the number of missed entries.
- **Write latency** is extremely low (~2 ms) because Raft consensus only requires majority acknowledgment.

You can reproduce these measurements by running:

```bash
cargo test --test ttr_report -- --nocapture
```

---

## Tuning

### Raft timing parameters

The default Raft timing is set in `crates/synaptica-server/src/node.rs`:

```rust
heartbeat_interval:   500,   // ms — how often the leader sends heartbeats
election_timeout_min: 1500,  // ms — minimum time before a follower calls election
election_timeout_max: 3000,  // ms — maximum election timeout (randomized per node)
```

| Parameter | Default | Effect of lowering | Effect of raising |
|---|---|---|---|
| `heartbeat_interval` | 500 ms | Faster failure detection, more network traffic | Slower detection, less traffic |
| `election_timeout_min` | 1500 ms | Faster failover, more spurious elections on slow networks | Slower failover, fewer false alarms |
| `election_timeout_max` | 3000 ms | Tighter election window | Wider spread reduces split-vote probability |

**Rules of thumb:**
- `election_timeout_min` should be at least 3× `heartbeat_interval`.
- On a LAN (< 1 ms RTT), you can safely lower to `heartbeat: 200, election: 600–1200`.
- Over WAN (50+ ms RTT), raise to `heartbeat: 1000, election: 3000–6000`.

### Cluster sizing

| Cluster size | Fault tolerance | Write latency | Recommendation |
|---|---|---|---|
| 1 node | 0 failures | Lowest | Development / testing only |
| 3 nodes | 1 failure | Low | Most production workloads |
| 5 nodes | 2 failures | Moderate | High-availability critical systems |
| 7+ nodes | 3+ failures | Higher | Rarely needed; increases commit latency |

Prefer **odd numbers** of nodes — even-numbered clusters don't improve fault tolerance (e.g., 4 nodes still only tolerates 1 failure, same as 3).

---

## Limitations

The current cluster implementation has the following known limitations:

| Area | Status | Detail |
|---|---|---|
| Write forwarding | Partial | `ForwardWrite` RPC exists but is not used automatically by the client service. Clients must send writes to the leader or handle the error and retry. |
| Range partitioning | Scaffolded | Partition and routing structures are defined but all data is currently stored in a single Raft group. Horizontal sharding is not yet functional. |
| Snapshots | Placeholder | Snapshot metadata is tracked but full graph state snapshot/restore is a stub. Catch-up is done entirely via log replay. |
| Transactions | Not implemented | `BeginTransaction`, `CommitTransaction`, `RollbackTransaction` RPCs are defined but not functional. |
| Inter-node TLS | Not implemented | Cluster gRPC traffic is unencrypted. Client-facing TLS works. |
| Dynamic config reload | Not supported | Changing Raft parameters requires a restart. |

---

## Troubleshooting

### Writes fail with "raft error" on a follower

**Cause:** You're sending writes to a follower node. Only the leader can accept writes.

**Fix:** Connect your client to the leader node. Use `ClusterStatus` to find the current leader, or implement retry logic in your client.

### Node won't join the cluster

**Cause:** The `--peer` addresses may be unreachable, or the cluster port is blocked by a firewall.

**Fix:**
1. Verify the peer addresses match the `--cluster-addr` of existing nodes (not the client `--listen` port).
2. Check that the cluster port is reachable: `curl http://<peer-addr>` (should get a gRPC error, not connection refused).
3. Check logs for `"raft initialize"` and `"cluster mode active"` messages.

### Slow re-election after leader crash

**Cause:** The election timeout (default 1.5–3 seconds) must elapse before followers call an election.

**Fix:** Lower `election_timeout_min` and `election_timeout_max` in the Raft config. Ensure `election_timeout_min >= 3 × heartbeat_interval`.

### Data not appearing on followers

**Cause:** Followers may lag by one heartbeat interval (~500 ms by default).

**Fix:** This is expected eventual consistency behavior. If you need linearizable reads, direct reads to the leader.

### High CPU usage on idle cluster

**Cause:** Heartbeats are sent every 500 ms. With many nodes, this can generate noticeable background activity.

**Fix:** Increase `heartbeat_interval` if you can tolerate slower failure detection.
