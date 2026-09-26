# Graphs

A user creates a named graph, lists the graphs the server knows, and runs a statement against one graph without changing another.

## Sub-features

- `graphs-create` creates `demo`.
- `graphs-list` shows `default` and `demo`.
- `graphs-select` writes a node on `demo` and shows that `default` does not have it.

## How to get to it (user POV)

- In the CLI, type `CREATE GRAPH demo` and `LIST GRAPHS`, or the REPL command `:graphs`.
- Reconnect with `--graph demo`.
- In the web UI the header graph selector switches graphs. This harness does not drive that control.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` has printed `doctor ok`.
- `USERPROFILE` for the CLI process is `%TEMP%\synaptica-verify`.

- **Create and list.** Pipe `CREATE GRAPH demo` and then `LIST GRAPHS` to `target\debug\synaptica-cli.exe --host http://127.0.0.1:19090`. The list table contains `default` and `demo`.
- **Write on demo.** Pipe `INSERT (:Person {name: 'Cara', age: 40})` to `target\debug\synaptica-cli.exe --host http://127.0.0.1:19090 --graph demo`. Stdout contains `Nodes created: 1`.
- **Read both graphs.** Pipe `MATCH (n:Person) RETURN n.name` to the same CLI with `--graph demo`, then again without `--graph`. The demo transcript contains `Cara`. The default transcript does not.
- **Proof.** Save both transcripts under `.grok/verify-evidence/graphs/`.

## Gotchas

- `--graph` selects the graph for that process. A later process without `--graph` is `default` again.
- `LIST GRAPHS` does not show node counts. Use the per-graph `MATCH` as the proof that the write landed on `demo`.
- The flag is `--host`, not `--addr`.
