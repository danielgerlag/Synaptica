---
name: verify-synaptica
description: >
  Drive the Synaptica graph database the way a user does: an isolated gRPC server
  and the synaptica-cli table session. Use when verifying GQL inserts, matches,
  graph selection, updates, deletes, health, or Prometheus counts, or when the
  user runs /verify-synaptica.
---

# Verify Synaptica

Primary surface: the `synaptica-cli` REPL against `synaptica-server`. Users type GQL and REPL commands (`:status`, `:graphs`) and read a box table. The React UI at `http://localhost:3000` (`/query`, `/graph`, `/schema`, `/explorer`, `/cluster`, `/metrics`) is a second surface. This repo has no browser harness, so do not claim a UI check from the CLI transcript.

Verification uses debug binaries and ports `19090` / `19091` so it does not attach to a server the user already has on `9090` / `9091`. Two verification instances cannot run at once: both would bind those ports. Launch refuses when a recorded pid is still alive.

Run every command from the repository root. The harness is `powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 <command>`.

## Launch

Build once if `target\debug\synaptica-server.exe` or `target\debug\synaptica-cli.exe` is missing (`cargo build -p synaptica-server -p synaptica-cli`). The script builds a missing server or CLI binary itself.

```powershell
powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 launch
```

This writes a private config under `%TEMP%\synaptica-verify\` and starts `target\debug\synaptica-server.exe --config` that file through a one-shot scheduled task named `synaptica-verify`. The task is deleted as soon as the process is running, which keeps the server from dying when the launch shell exits. Ready means the server log contains `starting gRPC server addr=127.0.0.1:19090` (ANSI color codes in the log are ignored). The process id is `%TEMP%\synaptica-verify\state.json`. A ready excerpt is copied to `.grok/verify-evidence/query-graph/server-ready.log`. `schtasks` may warn that the task time is earlier than now; `/Run` still starts it.

The data directory is `%TEMP%\synaptica-verify\data`, not `./data`. Metrics listen on `127.0.0.1:19091` because that address is config-only (`metrics_addr`); the server has no `--metrics-addr` flag.

## Doctor

Run this before any drive, and again when a command fails.

```powershell
powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 doctor
```

Doctor exits non-zero unless all of these are true:

- `state.json` exists and its pid is a live process named `synaptica-server`.
- The server log contains this run's `data_dir` and `starting gRPC server addr=127.0.0.1:19090`.
- `127.0.0.1:19090` accepts TCP.
- `http://127.0.0.1:19091/` contains `synaptica_nodes_total`.

Do not drive a server you did not launch.

## Drive

Pipe GQL into `target\debug\synaptica-cli.exe --host http://127.0.0.1:19090`. Set `USERPROFILE` to `%TEMP%\synaptica-verify` for that process only so the REPL history file is not the user's `%USERPROFILE%\.synaptica_history`. A trailing `\` continues a statement onto the next line. Piped input does not echo the `synaptica>` prompt; the prompt appears only in an interactive terminal.

Table output hides columns whose names start with `__`, prints strings without added quotes, and ends a visible result with `N row(s) returned in Xms`. Inserts with only internal id columns print `Nodes created: N` or `Edges created: N` and no table.

The scripted happy path:

```powershell
powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 drive-query
```

That sends the Alice/Bob insert, the `KNOWS` edge, and both `MATCH` queries from the README. Other features in `features/` use the same CLI binary and host with their own statements.

## Evidence

Write proof to `.grok/verify-evidence/query-graph/` (gitignored). `drive-query` writes:

- `transcript.txt` — CLI stdout for the statements, including mutation lines and the box table.
- `metrics.txt` — the `synaptica_nodes_total` and `synaptica_edges_total` lines after those statements.
- `server-ready.log` — the ready log lines copied at launch.

A passing drive contains `Nodes created: 1`, `Edges created: 1`, both names, `2020`, `1 row(s) returned`, `2 row(s) returned`, `synaptica_nodes_total 2`, and `synaptica_edges_total 1`. The metrics scrape is the side-effect check: the table is not enough.

JSON (`--format json`) and CSV keep internal columns. Do not use them as the table proof. Do not call storage or the execution engine in-process. There is no test-only endpoint.

## Cleanup

```powershell
powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 cleanup
```

Cleanup stops the pid recorded in `state.json` when that process is still named `synaptica-server`, then deletes `%TEMP%\synaptica-verify`. It does not delete `.grok/verify-evidence/`. Never stop `synaptica-server` by process name.

## Helpers

`.grok/skills/verify-synaptica/scripts/verify.ps1` accepts `launch`, `doctor`, `drive-query`, and `cleanup`. Feature files name any extra CLI statements. Do not invent another wrapper.
