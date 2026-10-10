# agy native gate

`scripts/agy-native-gate` drives a genuine agy TUI in an isolated named Herdr session against a scripted fake Gemini
server (`scripts/agy-fake-gemini`). It needs no model and no key. It is an opt-in developer gate and never runs in CI.

## Prerequisites

- macOS (it uses `/usr/bin/sandbox-exec` and `cp -c`) with `/usr/bin/python3` (3.9) or `HT_AGY_PYTHON`.
- `HT_AGY_BIN`: an absolute path to a genuine agy executable. Without it the gate exits 2.
  `scripts/agy-sandbox/fetch-agy --dest DIR` can download and verify one (below).
- Herdr 0.9.1 or newer on `PATH` (the first version that detects agy).
- A Rust toolchain, unless you pass `--herdr-threads-bin`: the gate builds
  `cargo build --locked --all-features --bin herdr-threads` (the daemon's owner watch is test-support only).

## Running

```
HT_AGY_BIN=/path/to/agy scripts/agy-native-gate --list
HT_AGY_BIN=/path/to/agy scripts/agy-native-gate --flow boot
HT_AGY_BIN=/path/to/agy scripts/agy-native-gate            # every discovered flow
```

Options: `--flow NAME` (repeatable), `--list` (names and `requires`), `--herdr-threads-bin PATH`, `--report PATH`
(JSON; default `target/agy-gate/report-<run id>.json`), `--keep-on-fail` (keeps a failing world's files but still
stops every process), `--timeout-scale F` (multiplies every wait).

Each flow runs sequentially in a fresh world with no fail-fast. A world is one root under `/private/tmp/ih.XXXXXX`:
a private Herdr session, a herdr-threads daemon, the fake server, a `cp -c` clone of `HT_AGY_BIN` behind an
`env -i` + `sandbox-exec` wrapper (loopback-only network, no read access to the real `~/.gemini`, `~/.claude`,
`~/.codex`, `~/.aisw`, `~/.hermes`, writes only under the root) and an isolated HOME. `world.ht(...)` refuses any
environment whose HOME is outside the root, so a flow can never touch the real home.

### Running against genuine agy 1.3.2

- `setup` and `unsetup` run through the `human` namespace, as the operator of the gate's isolated world: since
  herdr-threads v0.6.0 an agent-actor call of an escalating command must start with that word, and `World.ht`
  always puts `--state-dir` first.
- `World.start_agy` walks agy's first-run wizard (color scheme, Terms of Service) in each fresh HOME, then waits for
  the composer footer. It never answers a trust prompt (the `allow-rewrite` flow owns that one). A pane agy reaches
  without `start_agy` (`launch`) walks the wizard itself.
- Every pane shell finds the sandbox `agy` wrapper first (`<root>/home/.zshenv`), since `launch` and `handoff`
  type `agy` by name into a new pane.
- Scripted `run_command` calls carry all five properties the genuine declaration requires (`CommandLine`, `Cwd`,
  `WaitMsBeforeAsync`, `toolSummary`, `toolAction`), with `Cwd` bound to the world's `ws`; `invoke_subagent` carries
  `Subagents`, `toolSummary` and `toolAction`. The `permission` flow's denied command ends in `touch`, because agy
  runs `; true` unprompted.
- The `subagent` flow enrolls the parent with a first turn (agy fires no enrolling hook at startup) and expects one
  more parent request after the child reports.
- Acceptance: `scripts/agy-native-gate --flow boot --flow setup --flow permission --flow allow-rewrite --flow unsetup
  --flow launch --flow handoff --flow enroll --flow subagent --flow clear --flow resume`.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Every selected flow passed and the leak check is clean |
| 1 | A flow failed (a failed check or an exception) |
| 2 | Environment refusal: no `HT_AGY_BIN`, `CI`/`GITHUB_ACTIONS` set, not darwin, Herdr missing or older than 0.9.1, unknown `--flow`, build failed |
| 3 | The leak check failed although every flow passed (the pids are in the report and on stdout) |

## Never in CI

The gate, `fetch-agy` and the live smoke refuse under `CI` or `GITHUB_ACTIONS` (exit 2; unit-tested). CI runs only
the offline unit tests: `python3 -m unittest discover -s scripts/agy-sandbox -p 'test_*.py'` and the same for
`scripts/agy-gate`. They need no agy, Herdr, network or key.

## Leak-check policy

The gate exports `HT_TEST_OWNER=<gate pid>` and `HT_LEAK_RUN_ID=<uuid>`; both reach every child, including through
the wrapper's `env -i` allowlist. Everything is started through `agyenv.spawn_owned` (own session, group recorded
as `/tmp/ih-reaper.<owner>/g.<pgid>`) or inside Herdr panes, so the `isolated-herdr.sh` reaper stops it even if the
gate is SIGKILLed. Teardown order: agy panes (`/exit`, 5 s bound), the backend server, the daemon (bounded stop),
then `ih_teardown`. After all flows the gate runs `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"
--grace 15`; a non-zero result is exit 3.

## fetch-agy

```
scripts/agy-sandbox/fetch-agy --dest DIR [--version V] [--manifest FILE] [--archive FILE]
```

Mirrors the official installer: the manifest comes from the auto-updater service (or a saved `--manifest`), its URL
must start with `https://storage.googleapis.com/antigravity-public/antigravity-cli/`, redirects to other hosts are
refused, the archive sha512 must match the manifest and `agy-versions.json` when the version is pinned there, and
`antigravity` is extracted to `DIR/<version>/agy`. `--version` refuses a manifest of another version. It never runs
`agy install`. `--archive` verifies a local tar.gz instead of downloading.

## Flows

Flows are modules `scripts/agy-gate/flows/*.py` exporting `FLOW = Flow(name, requires, run)`, discovered by glob.
`requires` lists the beads that must have landed; `--list` prints it.

| Flow | Bead | What it proves |
|---|---|---|
| `boot` | ht-poh.6.2 | The infrastructure: Herdr detects agent `agy`, one main request reaches the fake server, `BOOT-OK` is on the pane, the sandbox denies the real `~/.gemini` and a non-loopback connect with EPERM, zero leaks after teardown |
| `setup` | ht-poh.6.4 | `setup agy`, plugin files, enable/disable, allow entry ownership |
| `permission` | ht-poh.6.4 | `command(herdr-threads)` auto-approves; a compound command blocks |
| `allow-rewrite` | ht-poh.6.4 | Whether agy's trust prompt drops the allow member, and re-setup repairs it |
| `unsetup` | ht-poh.6.4 | Owned files removed; pre-existing members and other keys kept |
| `launch` | ht-poh.6.4 | `herdr-threads launch --kind agy` and refused argv |
| `enroll` | ht-poh.6.5 | Seat binding, native session, startup check-in |
| `subagent` | ht-poh.6.5 | Child conversation: no presence lock, no new seat |
| `clear` | ht-poh.6.5 | `/clear` rotates the generation |
| `resume` | ht-poh.6.5 | `--conversation` and `-c` fire no `SessionStart` |
| `delivery` | ht-poh.6.6 | Transcript-confirmed settlement, ACK without a permission prompt |
| `downgrade` | ht-poh.6.6 | An unsupported injected step aborts the turn; the downgrade and recovery |
| `wake` | ht-poh.6.7 | Idle wake, busy and draft cases, `wake-on all` |

Only `boot` exists in this change; each other row is added by its leaf bead as its own module.
