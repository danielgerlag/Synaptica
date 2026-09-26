# Health and counts

A user checks that the server is up and that the published node and edge totals match the graph they just wrote.

## Sub-features

- `health-status` prints the server status, version, and uptime.
- `health-counts` reads `synaptica_nodes_total` and `synaptica_edges_total` after a known write.

## How to get to it (user POV)

- In the CLI, type `:status`.
- Open `http://127.0.0.1:19091/` for this verification server. A normal user server uses `http://127.0.0.1:9091/`.
- The web UI Metrics page reads the same gauges through gRPC. This harness does not open that page.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` has printed `doctor ok`.
- Query-graph has run on this launch, so the store has two nodes and one edge.

- **Status.** Pipe `:status` to `target\debug\synaptica-cli.exe --host http://127.0.0.1:19090`. Stdout matches `Status: ok | Version: 0.1.0 | Uptime: ` and the uptime is not `0s` after the process has been up for at least a second.
- **Counts.** `Invoke-WebRequest http://127.0.0.1:19091/` and keep the lines `synaptica_nodes_total 2` and `synaptica_edges_total 1`.
- **Proof.** Save the `:status` transcript and those two metric lines under `.grok/verify-evidence/health/`.

## Gotchas

- `synaptica_nodes_total` stays 0 until a scrape. Doctor and this read both hit the metrics port, which refreshes the gauges.
- Port `9091` is the user's server, not this run. Use `19091`.
- Uptime is whole seconds. A status read in the first second can still print `0s`.
