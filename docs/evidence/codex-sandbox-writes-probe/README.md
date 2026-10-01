# Codex 0.159.3 workspace-write write probe (ht-4is.8.20)

Live tea party take 8 (2026-10-01): a Codex guest's `herdr-threads accept` failed with a local permission error, and an earlier `ack` needed escalation. The socket allowance ([0.159.3 socket probe](../codex-1593-sandbox-probe/README.md)) lets a sandboxed command reach the daemon, but every mutation (and check-in) first writes the instance's client-side journals: the intent journal `<instance>/intents/` (a pending-operation record for recovery and `retry`, plus its `allocator.lock`, `next-ordinal` and `journal-format`) and the caller-context journal `<instance>/contexts/<sha256(seat)>/` (`context.json`, `context.lock`). Codex's `workspace-write` sandbox allows writes only under the workspace roots and tmp, so on a real install, where the state directory is `~/.local/state/herdr/plugins/herdr-threads`, those writes fail with `EPERM` and the CLI prints `herdr-threads: Operation not permitted (os error 1)`. The earlier native demos passed only because their state directories were under `/private/tmp`, which the sandbox treats as writable.

The fix: `setup codex` adds exactly those two directories to `sandbox_workspace_write.writable_roots`, as owned array members of the same allowance (manifest version 2; a version-1 allowance is upgraded on the next `setup codex`). It writes no root for the instance directory, the SQLite database, the endpoint descriptor, the owner lock, the daemon log or the setup manifests.

- Binary: `~/.local/bin/codex` resolving to `~/.codex/packages/standalone/releases/0.159.3-aarch64-apple-darwin/bin/codex`, `codex-cli 0.159.3`, sha256 `4d210f7c5a18fd0386434df23b5bdbb8c0e7257d3e8a2b30b0769c8bbe99a878` (the same binary as the socket probe).
- No model ran. Only `codex sandbox` ran, with scratch `CODEX_HOME`s: `ch-socket` holds only the three socket keys (what setup wrote before this change), and `ch-setup` holds the `config.toml` that this branch's `herdr-threads setup codex --harness-binary <that codex>` wrote. The output is in the results file. The real `~/.codex`, `~/.local`, the Herdr server and the installed daemon were never used.
- Everything lived under `$SCRATCH` (`/private/tmp/ht-sandbox-writes-scratch/run2`): `HOME`, the state directory at Herdr's default layout `$HOME/.local/state/herdr/plugins/herdr-threads`, a private fake Herdr endpoint ([fakehost.py](fakehost.py), the `ping`/`session.snapshot`/`pane.get` stand-in from `tests/integration/sweep.rs`), and a scratch daemon that the probe started unsandboxed and stopped afterwards.
- `/private/tmp` is itself writable under workspace-write. To make the scratch state directory as unwritable as a real `~/.local/state` one, every sandboxed command adds `-c sandbox_workspace_write.exclude_slash_tmp=true -c sandbox_workspace_write.exclude_tmpdir_env_var=true`, and runs with cwd `$SCRATCH/ws` (the only workspace root). B1 below confirms that the state directory was then read-only.
- Seats A and B are cooperative top-level Codex callers (`--cooperative-seat ... --cooperative-harness codex --cooperative-role top-level`) on the fake panes `w1:p1` and `w1:p2`.

Script: [run-probe.sh](run-probe.sh), with [fakehost.py](fakehost.py) and [try-write.py](try-write.py). Full output: [probe-results.txt](probe-results.txt). Paths are redacted to `$SCRATCH`, `$INSTANCE` (the instance digest), `$HERE` and `~`.

## Results

**Before** (`ch-socket`, the socket allowance alone). All of these failed with `EPERM`:

| Step | Command | Result |
| --- | --- | --- |
| B1 | `touch <instance>/intents/x` | `Operation not permitted`, rc=1 |
| B2 | A `send` | `herdr-threads: Operation not permitted (os error 1)`, rc=1 |
| B3 | B `check-in --lifecycle-event` | same, rc=1 |
| B4 | B `accept` | same, rc=1 |
| B5 | A `daemon health` | same, rc=1 (the CLI opens the intent journal even for this read) |

**After** (`ch-setup`, the `config.toml` setup now writes). First, `intents/` and `contexts/` were deleted, so the sandboxed CLI had to create its own roots. A writable root covers its own path, so the `mkdir` was allowed even though the instance directory stays read-only:

| Step | Command | Result |
| --- | --- | --- |
| A1 | A `check-in --lifecycle-event` (recreates both roots) | `checked_in`, rc=0 |
| A2 | A `send --require-ack B` (twice) | `message_sent`, rc=0 |
| A3 | B `check-in --lifecycle-event` | `checked_in`, rc=0 |
| A4 | B `ack <msg>` | `acknowledged`, rc=0 |
| A5 | B `accept` | `accepted`, rc=0 |
| A6 | B `send` (reply) | `message_sent`, rc=0 |
| A7 | B `check-in` (turn boundary) | `checked_in`, rc=0 |
| A8 | B `leave` | `left`, rc=0 |
| A9 | A `invite B` (re-invite after leave) | `invitation`, rc=0 |

**Negatives, same `ch-setup` sandbox.** Everything outside the two roots stayed read-only (`EPERM`, or SQLite's `attempt to write a readonly database`):

| Target | Attempt | Result |
| --- | --- | --- |
| `<instance>/x` | touch | denied |
| `<instance>/threads.sqlite3` | touch, open r+w, `CREATE TABLE` through sqlite3 | denied, denied, `readonly database`; an unsandboxed check afterwards found no `probe_x` table |
| `threads.sqlite3-wal`, `-shm` | open for append | denied |
| `endpoint.json`, `owner.lock`, `daemon.log` | open r+w / append | denied |
| `<instance>/client2` | mkdir | denied |
| `<state>/x`, `<state>/instances/x`, `<state>/setup/x` | touch | denied |
| symlink `contexts/esc` -> `threads.sqlite3` | create the link (allowed, it is inside a root), then open it r+w | denied: the sandbox checks the resolved target |
| `intents/journal-format` -> `<instance>/moved-out` | rename out of a root | denied |
| `$SCRATCH/x`, `$HOME/x`, `ch-setup/config.toml` | touch | denied |
| `intents/probe-ok`, `contexts/probe-ok` | touch, append (positive control) | allowed |

Control: `invite` of a seat that has already joined fails with `herdr-threads: unexpected mutation result`, both in the sandbox and unsandboxed. The probe runs that step unsandboxed to show it is not a sandbox effect. It leaves one pending intent in `intents/`, and it is a separate product issue. In the sandbox, A9 re-invites only after the leave.

## What this does not show

- Hooks: the hook path (`hook codex`, run by Codex itself) was not probed here; this probe covers the agent's own sandboxed tool commands.
- A sandboxed command cannot start the daemon (it would create `owner.lock`, the database and the socket outside the roots). The probe started the daemon unsandboxed first.
- The roots are per instance, like the socket. A sandboxed agent can write any seat's context journal under `contexts/`. That is no weaker than unsandboxed use, where the agent runs as the same user, and the daemon still verifies every claim.
