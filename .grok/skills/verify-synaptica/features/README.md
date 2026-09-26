# Synaptica verification map

This directory is the maintained source for verifying user-facing Synaptica behavior. Read this index, then follow one feature file. The harness is the CLI against the isolated server started by `verify.ps1`, not the React UI.

## Baseline preconditions

- Launch with `powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 launch`.
- The server listens on `127.0.0.1:19090` and metrics on `127.0.0.1:19091`.
- Data is `%TEMP%\synaptica-verify\data`. Do not use `./data`.
- `verify.ps1 doctor` reports the recorded pid, listen address, and data directory.
- Drive only that pid. A second launch against the same ports is refused.

## Driving conventions

- Start from a fresh launch unless the feature file says it continues an earlier feature.
- Pipe statements to `target\debug\synaptica-cli.exe --host http://127.0.0.1:19090`.
- Keep quoted GQL unchanged. A trailing `\` is the real line continuation.
- Set `USERPROFILE` to `%TEMP%\synaptica-verify` for the CLI process so history stays off the user's profile.
- Capture the transcript and, after a mutation, the matching Prometheus lines.
- Cleanup removes the server and `%TEMP%\synaptica-verify` only. Leave `.grok/verify-evidence/`.

## Proof and skip reporting

- Record the statements and the table or mutation lines they print.
- A mutation is proved only when a later `MATCH`, `LIST GRAPHS`, or metrics scrape shows the stored effect.
- The web UI routes are not verified by a CLI transcript. Report them as not driven.
- Report an unreachable path with the command and the failed doctor check. Do not substitute another feature.

## Feature entry contract

Each feature file has an H1, one opening paragraph, then the four H2 sections `Sub-features`, `How to get to it (user POV)`, `Driving it with verify.ps1`, and `Gotchas`.

## Features

- [Query a graph](./query-graph.md) inserts two people, a `KNOWS` edge, and reads them back ordered by age.
- [Graphs](./graphs.md) creates a named graph, lists graphs, and runs a query against that graph.
- [Update and delete](./update-delete.md) changes a property and removes a node, then reads what remains.
- [Health and counts](./health.md) reads `:status` and the Prometheus node and edge totals.
