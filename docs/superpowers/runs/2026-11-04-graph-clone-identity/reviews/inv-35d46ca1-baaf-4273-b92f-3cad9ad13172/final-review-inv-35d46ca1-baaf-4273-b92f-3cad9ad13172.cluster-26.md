# Final review, cluster 26 (tests-fixtures), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified with `git rev-parse --verify`. The caller checkout stayed at the head and clean throughout.

## Scope and method

The cluster is one file, `tests/fixtures/herdr-matrix/codex.js` (slice 000146, +5/-4, commit 227af9625). The change teaches the stand-in Codex `resume <id>`: it keeps the given session id, its SessionStart carries `source: resume`, and a resumed run logs to `<session>.<pid>.jsonl`.

I read the fixture at the head, its consumers (`tests/integration/herdr_release_smoke.rs`, `tests/named_seats/rig.rs`), the one caller of the new form (`seat_succession` in `tests/clone_identity_sweep.rs`), and the design's requirement baseline.

## Behaviour checks

I ran the head's fixture under a pty with a scratch `CODEX_HOME`, `HOME` and log directory, and a SessionStart hook that recorded its stdin and environment. The scratch directory has been removed and no process remained.

| argv | log file | hook `session_id` and `CODEX_SESSION_ID` | `source` |
|---|---|---|---|
| `--version` | none | no hook | none |
| (none) | `<uuid>.jsonl` | the uuid | `startup` |
| `resume abc-1`, run twice | `abc-1.<pid>.jsonl`, one file per run | `abc-1` | `resume` |
| `resume` | `<uuid>.jsonl` | a new uuid | `startup` |
| `resume --last` | `--last.<pid>.jsonl` | `--last` | `resume` |

The fresh-start path keeps its old shape. Every existing caller is therefore unaffected: no other test or script passes `resume` to this stand-in. `git grep` finds the word in neither `herdr_release_smoke.rs` nor `tests/named_seats/`. The `tests/cli/handoff.rs` and `tests/cli/launch.rs` matches build requests and do not start this fixture.

## Log-file naming

The per-pid name is required, not cosmetic. `Matrix::log_of` selects a file by the pane on its first row. A resume in another pane that appended to the old `<session>.jsonl` would leave that file's first row on the old pane, so `log_of(successor_pane)` would stay empty.

No consumer parses the file name. `Matrix::logs` reads the whole directory and sorts by the first row's `t`, and the evidence copy keeps each entry's name, so two runs of one session do not overwrite each other.

## Fidelity to real Codex

The stand-in now matches what the repository records about Codex:

- `scripts/validate-native-demo.py` line 1743 says the resumed thread keeps its id and rollout.
- `src/harness/codex.rs` line 1235 maps `source: resume` to `EventKind::Resume`.
- `scripts/validate-native-demo.sh` documents `codex resume <thread>`.

Real Codex also accepts flags and `--last` in this position, and the stand-in does not model them. Its only caller uses the plain `resume <id>` form.

## Requirement baseline

Rows B1 to B4 describe production behaviour, and this cluster holds no production path, so none of them can be delivered or missed here.

- B1: the fixture only enables the `seat_succession` flow, where a key passes to a successor and a resumed session binds there. Tracing B1 from its production entry point belongs to the seam reviewer and the sweep's cluster.
- B2 and B3: this fixture change adds nothing to either. Any trace belongs to the same reviewers.
- B4: the change touches a test stand-in only, and Herdr is unchanged.

The design's testing section asks for stand-in harnesses on an isolated Herdr. This fixture serves that, and it writes only to its log directory and to whatever hooks it is given.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `seat_succession` was not rerun by this review (the caller runs the suite once at this head, and each flow is that cluster's evidence); the fixture's resume behaviour was instead driven directly under a pty, as tabled above.
- `seat_succession` does not assert `source: resume` on the hook payload, so a stand-in that regressed to `startup` on resume might still pass it; the sweep's cluster owns that assertion, and the pty run confirms the head's value.

## Deferred OK
- `tests/fixtures/herdr-matrix/codex.js` line 20 still says every event goes to `$FAKE_CODEX_LOG_DIR/<session>.jsonl`, which is wrong for a resumed run (`<session>.<pid>.jsonl`); line 17 added the resume sentence without correcting it, and only a maintainer reading the docstring is affected.
- `tests/fixtures/herdr-matrix/codex.js` lines 34-35 take `argv[2]` as the session id without checking it, so `resume --last` runs with session `--last` and a bare `resume` silently starts fresh with `source: startup`, unlike real Codex; no caller uses either form, so handle them when one appears.
