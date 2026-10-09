**Review: ht-j16 (Claude Code mod inbound delivery), branch super-auto/claude-mod-inbound-delivery at 50e51077, fork point 08cea709**

Review package: /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/.superpowers/sdd/ht-j16-plan/review-08cea709..50e51077.diff
Spec: /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md

**Child bead status:** .1–.7 and .10 are merged and closed. ht-j16.8 (fix-loop gate) and ht-j16.9 (live race stress in real Claude sessions) are still open. Ledger metrics are consistent: 8 merges and 8 completions, nothing parked, no fix passes, no Recurring lines, no BLOCKED-AUTH lines.

## Step 1: my findings from the diff and the spec

**F1 (Important, confirmed; same defect as open bead ht-ows).** /clear can still trigger a native wake prompt into the pane being cleared.
- After a SessionStart(clear) check-in commits, the binding is already on the new generation. The registry entry stays Live for the old generation until the next worker pass turns it into rebind grace.
- During that gap the wake dispatcher (/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/scheduler/mod.rs, `mod_suppressed`) calls `is_live(seat, new_gen)`, which returns false. The wake lane can then type the attention marker into the pane.
- This is the exact turn-boundary race spec D7 says the rebind grace prevents.
- Pokes and the hook digest already check per seat (any registry entry, via `status()`). The wake-candidate path is the only one that checks per generation.
- Integration test `clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick` (/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/tests/integration/mod_delivery.rs:1580) avoids the race by marking the pane "working". So the assertion passes only because of that workaround.

**F2 (Important, likely but unverified).** Informational notices stop being delivered while a channel is live.
- At the PreToolUse(Bash) tool boundary, /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/harness/bridge.rs:1045 returns `ToolBoundary::default()` when `mod_channel_live` is set. That return comes before the `notices_pending` check-in that offers notices.
- `watch drain` (/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/cli/watch.rs) reads `InboxBatchV2`, whose warnings scan stops at the already-offered cutoff (`latest_warning_offset`). New notices therefore never reach the stream.
- The fingerprint (`other_pending` = invitations + warnings) does not track notices either.
- Result: a notice published while the channel is live waits until the next lifecycle check-in or until the channel is lost. Before this branch it showed at the next Bash call.
- Spec D2 names notice publication as a notify call site, and D4 lists notices under `attention`. No test covers notices with a live channel.

**F3 (Important, a seam between tasks, untested).** The mod starts the wrong binary, or the wrong instance, in non-default setups.
- `register.js` (/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/integrations/claude/mod/hooks/register.js) runs bare `herdr-threads watch` / `watch ack` from PATH, with no `--state-dir` or `--host-endpoint`.
- The settings hooks are installed with the absolute setup executable and an explicit state dir and host endpoint (`installed_argv`). hook.rs itself notes that pane agents do not inherit `HERDR_PLUGIN_STATE_DIR` and that auto-detection may not reach the hook's instance.
- Setup (ht-j16.7) writes nothing the mod could read: no `HERDR_THREADS_BIN`, no config file.
- Failure cases:
  - With a custom state dir or a second Herdr instance, `watch` resolves another instance.
  - If `herdr-threads` is not on PATH, or the PATH copy is older than the setup executable, it either fails to start or lacks `watch`.
- Any of these gives a silent, permanent exit-2/1 retry loop every 30 s. The feature never engages, even though setup-status says the mod is installed.
- The integration sweep always passes `--state-dir` and `--host-endpoint` explicitly, so the real invocation shape is never tested.

**F4 (Minor, spec gap).** `WatchMessage.thread_name` and `sender_name` are always `None`. D4 asks for thread name and sender, so the mod shows raw ids.

**F5 (Minor, divergence not recorded).**
- The `mod_delivery` setting is read only when the daemon starts. `set_mod_delivery`, `ModChannels::notify` and `record_attention_push` have no production callers.
- The D2 notify call sites became a table-commit observer.
- The `replaced` close reason exits 3, which the D3 table does not list.
- The spec's "Post-Implementation Notes" section is empty. These changes should be written there.

**F6 (Minor).**
- The `status()`-based checks (pokes, hook digest flag) ignore an expired `grace_until` until the sweep runs (about 1 s).
- The stalled-but-not-yet-closed window (up to 1 s) lets the native ladder and the open channel both deliver.
- Both windows are small.

## Step 2: ledger triage

**Must fix (misgraded as minor):**
- **T8 [workaround-masks-bug]:** this names a known product defect (ht-ows, still open), the same as F1. Must fix.

**Should fix or explicitly accept before landing:**
- **T7 [best-effort-guess]:** the cache file name for server-delivered managed settings (`remote-settings.json`) is a guess.
  - If it is wrong, a server-pushed `disableSideloadFlags` policy goes undetected and setup writes `CLAUDE_CODE_PLUGIN_DIRS`. Per spec D8, Claude Code then refuses to start.
  - Combined with T7 [scope-side-effect] (`doctor --fix` and the installer now also install the mod), users can hit this without running `setup claude`.
  - Verify the name, or skip the write when unsure.
- **T7 [unreachable-status-field]:** `claude_version_supported` is always null, so the D8 requirement to report `claude --version >= 2.1.287` is not met in practice. This is a spec requirement no task delivered.

**Deferred OK:**
- **T1:** TDD evidence missing, plugin validate skipped, test-count check unverified.
- **T2:** unmeasured digest cost per pass; non-atomic register (self-heals on the commit-observer pass); ModStoreReads location; tests written before TDD.
- **T2 [health-visibility-gap]:** acceptable for now. Note that if the worker dies, grace and stall are never swept, so the poke and digest suppression stays on indefinitely. Worth a follow-up bead.
- **T2 [unexplained-test-anomaly] and T4 [test-summary-swallowed]:** same class. A stdio-redirecting test already in the lib binary hides the `cargo test --lib` summary. This is a test-harness visibility problem and affects only single-process runs; nextest (one process per test) is unaffected. Confirm in the nextest sweep that the 113th `cli::hook` test passes.
- **T3:** pre-auth read (cooperative model); lint allow.
- **T4:** RED log missing.
- **T5:** style nit. The drain-coverage and fake-only items are now covered end to end by ht-j16.10, except notices (F2).
- **T6 [context-delivery-timing]:** this is the accepted limit "an outer mod could strip context" in TRUST-POLICY.
- **T6:** api-probe weakness, constant seed, d.ts unchanged.
- **T8:** fixed sleeps (watch the speed budget); tolerant exit assertion.

**Classes across tasks (no Recurring lines in the ledger):**
- (a) Process: tasks 1, 2 and 4 kept no RED-first TDD evidence.
- (b) Coverage stops at fakes and in-process stand-ins (T5, T6, T8). Nothing runs the real mod inside real Claude against a daemon set up by `setup claude`. F3 sits exactly in that gap.

## Verdict

Not ready.

## Must fix before landing

1. **F1 / ht-ows.** Make wake-candidate suppression count a Live or ReconnectGrace entry from an older generation as live, the same way pokes and the digest already do. Alternatively, run the registry pass synchronously after a lifecycle commit, before kicking the wake lane. Then remove the "working" workaround from `clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick` and update `tests/scheduler/mod_routing.rs::wake_reserved_when_channel_live_for_another_generation_only`.
2. **F3.** Have setup tell the mod the exact executable, state dir and host endpoint the hooks use (for example `env.HERDR_THREADS_BIN` plus a generated config the mod reads, or full argv). Add an integration case that runs the mod's exact argv with no flags.
3. **F2.** Keep offering informational notices while a channel is live: either still run the tool-boundary check-in when `notices_pending` is set, or have `watch` emit an `attention` item and fingerprint change for notices. Add a test for a notice published while the channel is live.
4. **T7 managed-policy guess.** Verify the cached file name for server-delivered managed settings, or fail safe (do not write `CLAUDE_CODE_PLUGIN_DIRS`) when it cannot be checked. This needs a decision because a false negative stops Claude from starting.

## Untested scope

- **Whole-suite run:** none exists for this branch. The full nextest sweep is deferred to the caller and has not run, so the branch is not proven against the suite or the 5-minute budget. The new integration tests include fixed sleeps of up to 3.2 s and a 64-watch admission test. Run `scripts/check-no-leaked-processes` afterwards; this suite spawns many `watch` children.
- **ht-j16.9 (live race stress in real Claude TUI sessions):** still open and gated on ht-j16.8. None of the D10 live scenarios have evidence: Esc takeover, queued prompt, Stop-hook continuation, permission dialog, /clear, /resume, reload mid-turn, UserPromptSubmit drop.
- **Mod JS tests** (`integrations/claude/mod/tests/*.test.ts`, run by `scripts/test-claude-mod`): not wired into nextest or CI, and they skip with exit 0 when `claude` is missing. No ledger evidence shows they ran on the merged head. T1 also records that `claude plugin validate` was skipped.
- **Live engine behaviour:** `stream.return()` actually stopping the watch child, the order of `session.end` vs SessionStart hook vs Close, and `$.state` surviving a real reload are all untested.
- **The mod's real invocation** (bare binary and instance auto-detection), per F3.
- **Notices with a live channel** (F2), and the `setup-status` Claude version gate (always null).
- No BLOCKED-AUTH lines, so no coverage was lost to permission refusals.

## Deferred OK

- F4 (thread and sender names), F5 (record the divergences in Post-Implementation Notes), F6 (sub-second windows).
- T1 through T6 minors as triaged above. File a follow-up bead for the mod worker's Health visibility (T2).
- T7 [scope-side-effect] (doctor and installer install the mod) is acceptable once must-fix 4 is resolved.
- T8 [fixed-sleeps] and [tolerant-assertion], to revisit if the sweep exceeds the speed budget.
- Process class (a), no TDD evidence: not a code defect.