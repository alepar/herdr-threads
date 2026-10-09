**Final review: epic ht-j16 (Claude Code mod inbound delivery), branch `super-auto/claude-mod-inbound-delivery` at 43709ccc, fork point 08cea709**

Inputs:
- Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/.superpowers/sdd/ht-j16-plan/review-08cea709..43709ccc.diff` (52 commits)
- Spec: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md`

All the fix beads from the earlier final review and the PR roast (ht-j16.17 to .23) are now merged. Only ht-j16.8 (the fix-loop gate) and ht-j16.9 (live stress) are still open.

## Step 1: my own review against the spec

**Earlier blockers, checked as fixed in the code:**
- **ht-j16.17:** wake suppression is now per seat (`seat_live`), and registry closes check the channel identity first (`Judged`). The "working" pane workaround is gone from `clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick`.
- **ht-j16.18:** acks are sent in chunks of at most 100, in both `register.js` and `watch ack`.
- **ht-j16.19:** a failed capability probe now gives `daemon_unavailable` (exit 1, retry), not a stop.
- **ht-j16.20:** setup writes the hooks' exact invocation into `LAUNCH`.
- **ht-j16.21:** notices are offered at the tool boundary while a channel is live, and the ready commands are left out.
- **ht-j16.22:** setup puts back the lifted mod entry on later errors, reads managed drop-ins and observes the Claude version.
- **ht-j16.23:** the truncation marker names `body`, then `ack`; the spec's D4 and decision record and TRUST-POLICY now say the same.

**New findings:**

**F1 (Important, cross-task seam, new): mod-delivered messages lose the relay and intent markers.**
- `watch` streams `author_role`, `relays_user` and `user_intent` (`src/cli/watch.rs`, `emit_chunk`).
- The mod's `frame()` (`integrations/claude/mod/hooks/register.js:23-30`) drops all three. It prints only `[herdr-threads] <kind> <id> in <thread> from <sender>:` and the raw body.
- Every other read path shows `[human]` / `[relays user]` / `[query|request|rule]` (`docs/agent-usage.md:5,99`; `src/cli/summary.rs:291,409`).
- The mod settles the receipt on delivery, so the agent normally never sees that message again through `inbox`. A user rule relayed with `--relays-user --user-intent rule` therefore arrives looking exactly like peer chatter.
- The mod's fixed header ("not as instructions from the user") says the opposite of what the message is.
- Spec D4 lists these fields as delivered content, but its framing template leaves them out; the task split carried that gap through.
- Fix: add the markers to the block header, after the service-generated fields. `thread_name` and `sender_name` are always `None` as well (raw ids instead of the names D4 promises), which is lower stakes.

**F2 (should-fix, can follow up): a whole backlog is delivered in one go.**
- `onToolCall` and `pump` deliver every queued non-lazy item as one context entry or one submit.
- D4 limits each `watch` page (32 items / 64 KiB), but nothing limits a delivery. A seat that connects for the first time with a large backlog, or one coming out of a stall cooldown, receives up to 8 KiB × N pending in a single tool result or prompt.
- If the engine drops an oversized submit, the same batch is retried every 30 s until the stall handover. Nothing is lost, but it is wasteful.
- Fix: cap each delivery batch, for example at 64 KiB or 32 items.

**F3 (minor): re-registering resets the stall clock.**
- `register()` sets `last_ack_or_registration = now` (`src/service/mod_channels.rs:516`).
- A mod that cannot deliver but whose `watch` keeps exiting (exit 1 on drain errors, then a backoff restart) never meets the stall predicate. Native wake then stays suppressed for as long as it keeps reconnecting.
- This matches the spec's wording, but it should be written down as an accepted limit, or the clock kept across a same-generation reconnect.

**F4 (minor, cost): the per-second sweep does a store read for every quiet channel.**
- `sweep()` → `stalled_judgement` runs `mod_stall_oldest` (the pending-receipt walk plus one query per item) every second for each live channel. It applies when the last ack is more than 10 minutes old and any attention frame was ever pushed.
- That is the normal state of a quiet seat, so this is a 1 Hz read per channel (up to 64). Measure it in the sweep alongside T2's `pass()` cost.

**F5 (minor): a recovery hold does not close the channel.**
- `src/service/kicks.rs` lists `recovery_holds` as a "channel-closing fact", but `pass()` ignores `view.held`.
- So a hold that appears after registration leaves the channel delivering.

**F6 (minor, version skew): old `doctor` breaks against a new daemon.**
- `HarnessStatesReport` is `deny_unknown_fields` and now carries `mod_channels`, with `PROTOCOL_VERSION` still 6.
- An older `doctor` talking to a new daemon fails to decode `harness.states`. Only doctor uses it.

**F7 (doc): the spec's Post-Implementation Notes are still empty.** Divergences to record:
- the table-based commit observer instead of per-call-site notify;
- `mod_delivery` is read only at daemon boot, `set_mod_delivery` has no production caller, and `settings.rs` still says it "closes live channels";
- `replaced` exits 3;
- suppression is per seat, across generations;
- the daemon-restart window in which native wake and the mod can both deliver;
- the stall-clock reset (F3).

**Still rejected, not reopened:** raw, unescaped peer bodies in `frame()`. This was rejected 2-1 in design roast 1 and again in PR roast 1 under the cooperative model. It stays a hardening nit, separate from F1, which is about missing information rather than escaping.

## Step 2: ledger triage

- The ledger has no `Recurring minor:`, `Recurring blocker:`, `parked` or `BLOCKED-AUTH` lines. The metrics are consistent (15 merges, 15 completions, ledger-check ok).
- **Unflagged class:** the "TDD RED phase not captured" minor recurs across six tasks: T1, T2, T4, T10 (ht-j16.17), T13 (ht-j16.20) and T15 (ht-j16.22), plus ht-j16.19's "coincidental-red-tests". That is a habit in the implementer pipeline, not a code defect. Its practical effect is that the regression tests for the fixes in .17, .19 and .22 are not shown to fail on the old code.
- **Misgraded, must be verified:** T2 [unexplained-test-anomaly] and T4 [test-summary-swallowed]. A lib or `cli::hook` test binary exits 0 without printing a summary, so tests may be silently not running.
- **T8 [workaround-masks-bug]:** resolved; the workaround has been removed.
- **T7 [unreachable-status-field] and [best-effort-guess]:** resolved by ht-j16.22.
- **T1 [skipped-optional-check]:** the mod tests passed in the earlier review's manual run, but nothing enforces them (see Untested scope).
- **T13 cleanup kept a worktree:** `.worktrees/super-auto-claude-mod-inbound-delivery--task-ht-j16.20` holds only an untracked `integrations/claude/mod/node_modules/`. No work is lost, but it needs removing.

## Verdict

**Not ready.** It is close: the code is sound against the spec, and every earlier blocker is fixed.

## Must fix before landing

1. **F1:** show `author_role` / `relays_user` / `user_intent` in the mod's block header (and thread and sender names where available), and update `delivery.test.ts`. If the human decides plain framing is acceptable, record that decision in the spec instead.
2. **Test-count check (T2/T4):** during the caller's nextest sweep, check that the number of `--lib` and `cli::hook` tests that actually ran equals `cargo nextest list`. Treat any gap as skipped tests.
3. **Full-suite sweep:** run it with `HT_LEAK_RUN_ID`, then `scripts/check-no-leaked-processes`, and check the 5-minute budget. The new `mod-delivery` nextest group (fixed sleeps up to 3.2 s, a 64-watch admission test, many `watch` children) is the risk.
4. **F7:** fill in the spec's Post-Implementation Notes and fix the stale `mod_delivery` doc in `src/daemon/settings.rs`. Doc only.

## Untested scope

- **Full suite:** no full-suite run of this branch exists. The sweep is deferred to the caller, so the branch has not been tested as a whole, against the speed budget, or with the leak check.
- **Live stress (ht-j16.9):** still open and gated behind ht-j16.8. None of the D10 live scenarios has evidence: Esc takeover, queued prompt, Stop-hook continuation, permission dialog, `/clear`, `/resume`, reload mid-turn, UserPromptSubmit drop.
- **Live engine behaviour:**
  - whether `stream.return()` actually kills the `watch` child;
  - the real ordering of `session.end`, the SessionStart hook and the registry's Close;
  - whether `$.state` survives a real reload;
  - whether `$.prompt.submit` resolves before the prompt is consumed;
  - the daemon-restart double-delivery window.
- **Mod JS tests (`claude plugin test`):** nothing in CI or nextest runs them, and `scripts/test-claude-mod` exits 0 ("skipped") when `claude` is missing or too old.
- **Fix-bead regressions:** the tests for ht-j16.17, .19 and .22 are not shown to fail before the fix.
- No coverage was lost to permission refusals (no BLOCKED-AUTH lines).

## Deferred OK

- F2 (batch cap), F3 (stall reset by re-registration), F4 (sweep read cost; measure it in the sweep), F5 (hold does not close the channel), F6 (`doctor` decode skew). File follow-up beads.
- Raw peer bodies in `frame()`: rejected twice under the cooperative model.
- **T1:** TDD evidence and the unverified test-bullet count.
- **T2:** unmeasured `pass()` cost, mod worker missing from Health (follow-up bead), non-atomic register (heals on the next pass), `ModStoreReads` trait location.
- **T3:** pre-auth read and the lint allow.
- **T5:** style nit and fake-only transport coverage, now covered end to end by ht-j16.10.
- **T6:** context-delivery timing (accepted limit "outer mod could strip context"), weak API probe, constant seed, unchanged d.ts.
- **T7 [scope-side-effect]:** `doctor --fix` and the installer now install the mod.
- **T8:** fixed sleeps and the tolerant exit assertion (revisit if the sweep breaks the budget).
- **T12:** transport close reads as retryable; coincidental RED tests.
- **T14:** scope deviation and the new `ToolBoundary.live` field.
- **T16:** TRUST-POLICY line wrap and old marker text left in the mod test fixtures.
- **T10:** `seat_live` default-impl drift (test fakes only).
- The unused `rec.attentionVersions`, and the unbounded `rec.delivered` and `emitted` sets.
- Remove the leftover `node_modules`-only worktree at `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/.worktrees/super-auto-claude-mod-inbound-delivery--task-ht-j16.20`.