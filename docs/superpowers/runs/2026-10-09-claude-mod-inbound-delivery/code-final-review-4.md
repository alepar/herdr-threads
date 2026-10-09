**Verdict for the ht-j16 epic review: not ready to land**

The branch's code mostly matches the spec, and every earlier blocker is fixed. Three things stop it landing: it no longer merges cleanly with `main`, two accepted limits are missing from TRUST-POLICY, and nobody has run the full suite or the live stress test. The caller has not yet run the full-suite sweep, so this branch has no full-suite measurement and should not be treated as tested.

## Step 1: my own review against the spec

1. **The branch conflicts with `main`. This is the main blocker.**
   - The fork point is `08cea709`, and `main` (`3d10e3bb`) is 205 commits ahead.
   - `git merge-tree --write-tree main HEAD` shows conflicts in 11 files, 21 hunks in total: `src/app.rs`, `src/cli/{commands,doctor,hook,installer,launch,setup}.rs`, `src/daemon/harness_states.rs`, `tests/cli/hook.rs`, `tests/harness/bridge.rs` and `tests/setup_cli.rs`.
   - On `main`, `src/cli/setup.rs` was largely rewritten (about 2,400 lines changed; the commits move setup into Claude/Codex setup backends). `src/harness/setup.rs` gained about 990 lines and `hook.rs` about 1,070.
   - So the mod install (D8: `CLAUDE_CODE_PLUGIN_DIRS`, the manifest, the managed-policy check, `--hooks-only`, `setup-status`) and the hook digest omission (D7) have to be re-done on the new code, not just merged.
   - `src/scheduler` is unchanged on `main`, so the wake suppression in `WakeRunner` should carry over. `src/notification/dispatch.rs` did change and needs checking.
   - None of the earlier final reviews mentions this.

2. **TRUST-POLICY.md is missing two accepted limits.** AGENTS.md requires any new accepted limit to be recorded there in the same commit. Both are only in the design doc's Post-Implementation Notes:
   - **Stall clock reset:** every new registration resets `last_ack_or_registration` (`register()` in `src/service/mod_channels.rs` creates a fresh `Entry`). A mod whose `watch` keeps reconnecting can therefore keep native wake off with no time limit. This contradicts the stall bound TRUST-POLICY now states.
   - **Daemon restart:** after a restart, a message can reach the agent twice, once through native wake and once through the mod.
   - The cheaper code fix for the first is to keep the stall clock and the last attention push when the same seat re-registers under the same generation.

3. **The mod keeps delivering its queue after the daemon has handed the seat back to native wake.** `onChildExit` in `register.js` never clears or parks `S.queue`, and `pump()` / `onToolCall` never check whether the channel is connected.
   - After `Close{stalled}`, `disabled` or a 3-exit, the mod still submits or attaches queued messages while the native ladder may also deliver them. D7 explicitly rejected having two delivery paths.
   - The acks come back retryable, so nothing is lost; the cost is duplicate delivery.

4. **Minor:** the mod's queue is not pruned when a message is settled some other way (for example `inbox` during a post-abort hold), so it can submit messages that are already settled.

5. **Minor:** a nested `claude` session started inside the pane inherits `HERDR_PANE_ID`. Its mod then retries `watch` with `session_mismatch` every 30 seconds for that session's whole life. D11 assumes it exits 3, which only happens with no pane id.

6. **Checked and correct:**
   - receipt settlement under `cooperative_mod_delivery`, including the truncation check against the stored body;
   - the resume re-ack from the previous generation;
   - per-seat liveness across the wake path, pokes and the hook digest;
   - rebind and reconnect grace;
   - separate admission for watch connections;
   - capability gating;
   - TRUST-POLICY rows A3, A4, A5 and A8.

7. **Mod JS tests:** I ran `scripts/test-claude-mod` at `77590941`: 44 passed, 0 failed, and `claude plugin validate --strict` passed. Nothing in nextest or CI runs these tests.

## Step 2: ledger triage

- **Metrics:** 17 merges and 17 completions, ledger check ok. There are no `parked`, `BLOCKED-AUTH`, `Recurring minor` or `Recurring blocker` lines.
- **A cluster the ledger did not flag:** reports with no captured failing (red) run before the fix.
  - It appears 7 times across 7 tasks: T1, T2, T4, T10 (ht-j16.17), T13 (ht-j16.20), T15 (ht-j16.22) and T17 (ht-j16.24). T12 (ht-j16.19) adds "coincidental-red-tests".
  - It went unflagged because each task used a different tag for the same thing, so the cluster detector never matched them. That is a defect in the pipeline.
  - The class is that implementers do not capture red-phase test evidence. As a result, the regression tests for fixes .17, .19, .20, .22 and .24 are not shown to fail on the old code.
- **T2 [unexplained-test-anomaly] and T4 [test-summary-swallowed]:** graded minor, but tests may be silently not running. The branch adds no stdio redirection; the existing `dup2` in daemon tests is the likely cause. Treat as must-verify.
- **T8 [workaround-masks-bug]:** resolved. At HEAD, `clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick` asserts no prompt without marking the pane as working.
- **T7 [unreachable-status-field] and [best-effort-guess]:** resolved by ht-j16.22.
- **T18 [leak-check-gap]:** a foreign `claude` process was seen. The caller's sweep has to include the leak check.

## Must fix before landing

1. Rebase or merge onto current `main`, resolve the 21 hunks, and re-implement the D8 mod install and the D7 hook digest omission in `main`'s new setup backend and hook code. Then re-review those seams.
2. Record both accepted limits from item 2 in TRUST-POLICY.md, or fix the stall-clock reset in code and record only the daemon-restart window.
3. After the rebase, run the full nextest suite with `HT_LEAK_RUN_ID`, then `scripts/check-no-leaked-processes`, and check the 5-minute budget (the new `mod-delivery` group uses fixed sleeps and a 64-watch admission test).
4. During that run, check that the number of `--lib` and `cli::hook` tests that actually ran equals `cargo nextest list`. Treat any gap as skipped tests.
5. Re-run `scripts/test-claude-mod` on the post-rebase commit. It must report results, not `skipped:`.
6. Run ht-j16.9, the live stress test, on the final post-rebase commit, or record its documented read-only fallback and the live gap. D10 requires it, and it is still open behind the ht-j16.8 gate.

## Untested scope

- The full suite has never been run on this branch, and the branch has never been built merged with current `main`.
- The live D10 scenarios have no evidence: Esc takeover, queued prompt, Stop-hook continuation, permission dialog, `/clear`, `/resume`, reload mid-turn, UserPromptSubmit drop.
- Real Claude Code behaviour is untested:
  - whether `stream.return()` actually kills the `watch` child;
  - whether `$.state` survives a real reload;
  - the real ordering of `session.end` against the SessionStart hook;
  - the daemon-restart double-delivery window.
- CI never runs the mod JS tests, and the script exits 0 with `skipped:` when `claude` is missing or too old.
- The fix-bead regression tests listed above have never been shown to fail before the fix.
- No coverage was lost to permission refusals.

## Deferred OK (file follow-up beads)

- Item 3 (the mod should drop or park its queue after a stalled, disabled or 3-exit close) and item 4 (prune queue entries settled by another path).
- Item 5 (nested sessions retrying `watch`).
- Earlier final-review items F2 (batch cap) and F4 (cost of the read every pass makes, to be measured in the sweep).
- T2: the mod channel worker does not show in Health (a dead worker stops stall handover); the non-atomic register, which heals on the next pass; the trait location.
- T3: the read before authorization, and the lint allow.
- T5: the style nit, and transport coverage that used only a fake server (now covered end to end).
- T6: context-delivery timing (already an accepted limit), the weak API probe, the constant stress seed, the unchanged `.d.ts`.
- T7: `doctor --fix` and the installer now install the mod.
- T8: fixed sleeps and the tolerant exit assertion, unless the sweep breaks the budget.
- T12: an older daemon is read as retryable rather than unsupported.
- T14: the scope deviation and the new `live` field.
- T16 and T17: line wrap, fixture text, import placement.
- T10: the default `seat_live` drift, which affects test fakes only.
- `rec.attentionVersions` is unused, and `rec.delivered` and `emitted` are unbounded.

Files reviewed:
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/.superpowers/sdd/ht-j16-plan/review-08cea709..77590941.diff`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/service/mod_channels.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/integrations/claude/mod/hooks/register.js`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/TRUST-POLICY.md`