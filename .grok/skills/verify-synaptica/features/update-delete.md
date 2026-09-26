# Update and delete

A user changes a stored property and deletes a node, then reads the graph to see what remains.

## Sub-features

- `update-set` changes Alice's age to 31 and returns the new value.
- `update-delete` removes Bob.
- `update-read` shows Alice at 31 and does not show Bob.

## How to get to it (user POV)

- Type `SET` and `DETACH DELETE` in the CLI, then a `MATCH`.
- The web UI query editor can run the same statements. This harness does not drive that editor.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` has printed `doctor ok`.
- Query-graph has already inserted Alice (30) and Bob (25). Run `drive-query` first on this same launch.

- **Set age.** Pipe `MATCH (n:Person) WHERE n.name = 'Alice' SET n.age = 31 RETURN n.name, n.age` to `target\debug\synaptica-cli.exe --host http://127.0.0.1:19090`. The table contains `Alice` and `31`, and stdout contains `Properties set: 1`.
- **Delete Bob.** Pipe `MATCH (n:Person) WHERE n.name = 'Bob' DETACH DELETE n`. Stdout contains `Nodes deleted: 1`.
- **Read what remains.** Pipe `MATCH (n:Person) RETURN n.name, n.age ORDER BY n.name`. The only data row is `Alice` and `31`.
- **Proof.** Save the three transcripts under `.grok/verify-evidence/update-delete/`. A `SET` line without the follow-up `MATCH` is not proof.

## Gotchas

- `SET n.age = 31` without `RETURN` still changes the node. The following `MATCH` is the check.
- `DETACH DELETE` also removes edges that touched Bob. A later edge read can be empty for that reason.
- Deleting from `default` does not delete nodes on another graph.
