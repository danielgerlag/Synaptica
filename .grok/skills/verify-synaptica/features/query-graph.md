# Query a graph

A user inserts people, connects them with an edge, and reads the names, the edge property, and an age ordering back as a table.

## Sub-features

- `query-insert-nodes` creates Alice and Bob.
- `query-insert-edge` creates the `KNOWS` edge between them.
- `query-read-edge` returns the two names and `since`.
- `query-order` returns people older than 20, highest age first.

## How to get to it (user POV)

- Start the server, then run `synaptica-cli` and type the statements at the `synaptica>` prompt.
- In the web UI, open `http://localhost:3000/query` and use the Run control on the query editor. This harness does not drive that page.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 launch` has printed `ready`.
- `verify.ps1 doctor` has printed `doctor ok`.
- The data directory has no `Person` nodes yet. Use a fresh launch.

- **Insert Alice and Bob, then the edge, then both reads.** Run `powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 drive-query`. Stdout contains `Nodes created: 1` twice, `Edges created: 1`, a table whose header is `a.name`, `b.name`, `r.since` with `Alice`, `Bob`, `2020`, and a second table `n.name`, `n.age` with `Alice` / `30` above `Bob` / `25`.
- **Confirm storage.** Open `.grok/verify-evidence/query-graph/metrics.txt`. It contains `synaptica_nodes_total 2` and `synaptica_edges_total 1`.
- **Proof.** Keep `.grok/verify-evidence/query-graph/transcript.txt`, `metrics.txt`, and `server-ready.log`.

## Gotchas

- Piped CLI output has no `synaptica>` prompt. The mutation lines and tables are the proof.
- A table that still shows `__node_id` is the wrong binary. Rebuild `synaptica-cli`.
- `Edges created: 1` without `synaptica_edges_total 1` is not proof.
- Do not point this run at `./data` or port `9090`.
