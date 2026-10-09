# Claude mod delivery: live stress evidence (ht-j16.9)

Status: FALLBACK. The live run in real Claude TUI sessions did not happen because no signed-in profile was
usable. The unit-level stress passed; the live gap is recorded below.

## Run

- Source SHA: `13d795f9a496ec15cb5bb88b86258a76e894718f` (head of `super-auto/claude-mod-inbound-delivery` when the
  task branch was cut, after gate ht-j16.8). Evidence is not final-SHA for any later commit.
- Claude Code: 2.1.295 (`claude --version`), macOS (Darwin 25.6.0), tmux and Python 3 stdlib only.
- Command: `python3 tests/native/claude_mod/stress.py --fallback-only` (runs `scripts/test-claude-mod`).
- Raw result: [`summary.json`](summary.json) (includes the full `claude plugin validate --strict` and
  `claude plugin test` output).

## Result

| Check | Result |
|---|---|
| `claude plugin validate --strict` | passed |
| `claude plugin test` (44 tests across `delivery.test.ts` and `stress.test.ts`, including the randomized stress of 600 schedules asserting no submit during a turn or hold and every id delivered and acked at most once) | 44 pass, 0 fail |
| Driver invariant helpers (`python3 -m unittest tests/native/claude_mod/test_stress.py`) | 21 tests pass |
| Live scenarios (14 scenarios x 20 iterations) | NOT RUN, see gap |

## Live gap

Reason: the signed-in spike profile (`.../scratchpad/modspike/profile`) was copied to a private dir and used as
`CLAUDE_CONFIG_DIR` with an isolated `HOME`; `claude -p "say ok"` answered `Not logged in - Please run /login`.
The cause was not investigated further (credential stores were deliberately not inspected); the copy simply is not signed in. The original profile was not modified and no other credential source was inspected.

Not exercised: every live scenario (`busy_context`, `idle_submit`, `lazy_append`, `queued_user_prompt`,
`stop_hook_continuation`, `esc_interrupt_hold`, `permission_dialog`, `clear_rebind`, `resume_keeps_set`, `reload`,
`reload_mid_turn`, `kill_watch_fallback_and_restream`, `user_prompt_submit_block_drop`,
`denied_tool_with_pending_context`), that is: real TUI turn/abort/permission events, the real `$.prompt.submit`
and `$.session.append` paths, the `watch` child against a real daemon, and the daemon-receipt cross-check.

The live driver (`tests/native/claude_mod/stress.py`) is written but its live path has never run: its tmux
orchestration, thread bootstrap (`thread create`/`invite` flags), receipts DB path and the Herdr pane setup
(it uses fake pane ids; an isolated Herdr session via `scripts/lib/isolated-herdr.sh` is not wired) are untested
guesses and will need a first dry run with a usable profile. The profile-side hooks some scenarios assume (a Stop
hook that blocks while `stop-armed` exists, a UserPromptSubmit hook that blocks while `block-ups` exists, both in
the working directory) are not yet installed by the driver.

## Re-running live

1. Provide a signed-in `CLAUDE_CONFIG_DIR` that stays signed in when copied, or run against a profile the operator
   authenticates inside the private tmux server.
2. `python3 tests/native/claude_mod/stress.py --profile <dir> --iterations 1` for a dry run, read every pane
   (`pane-*.txt` captures), fix the driver, then the default `--iterations 20`.
3. `scripts/check-no-leaked-processes` afterwards.

## Invariants checked per run (pure functions, unit-tested)

- at most one `delivered` and one settling `acked` (`reason: settled`) per message id per session key; a `/clear`
  or an `ack:stale_generation` refusal legitimately resets the id;
- no `submit` ledger entry while the entry's `turn` is set (open main turn), nor inside a post-abort hold;
- no ack for a truncated id;
- every sent message ends `acked` or `pending` in the daemon (never missing), and a settling ledger ack never
  disagrees with a pending daemon receipt.
