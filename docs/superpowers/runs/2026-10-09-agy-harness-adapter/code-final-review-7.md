**Verdict: not ready.** I found no Critical or Important defect in the code. The branch is not ready because the evidence it needs to land does not exist yet: no full-suite run at this SHA, no final-SHA native gate or live smoke, and one flaky test this branch added is still open. HEAD is `54fb8d0a`. No full-suite measurement of this branch exists; the caller runs that sweep after this review, so the branch is not tested as a whole.

Checks I ran at HEAD, all passing:
- `cargo clippy --all-targets --all-features -D warnings` is clean, and `cargo fmt --check` is clean.
- `scripts/check-harness-neutral` reports 0 violations, 85 allowlisted, and no `to-migrate` entries.
- Focused lib tests filtered on `agy delivery frame wake_on`: 370 passed. `check-no-leaked-processes` was clean afterwards.
- `scripts/test-claude-mod`: 95 passed, including the R17 frame goldens.
- Every Python suite under `scripts/agy-*` and `scripts/tests` passes.

## Step 1: my own review

**Every blocker from the previous final review (invocation 6) is resolved on the branch:**
- **R17 compact framing has landed (ht-poh.41).** The new header is `[herdr-threads] new messages:`. The "untrusted" wording is gone from the frame, the mod, the hook check-ins (`peer_data:`) and the agy skill tail. The SKILL "Trust model" section and the AGENTS.md "Token-efficient communication" rule are present.
- **Confirmation after a tool call on genuine agy is covered.** ht-poh.45 added a genuine capture, `tests/fixtures/agy/delivery/transcript-genuine-post-tool-call.jsonl` (`SYSTEM_SDK`/`SYSTEM_MESSAGE`), and the wake gate now asserts settlement.
- **DC-3 is resolved.** The ActiveTurn poke branch and the `poke_during_turn` capability were removed (ht-poh.42).
- **Two parked decisions now have the human's ruling, recorded in the spec's post-implementation notes:** `unsetup agy` keeps the v0.6 settings backups (ht-poh.43), and the "check-in unavailable" notice prints when the daemon is down (ht-poh.47).
- **Smaller fixes landed:** probation now ends after one turn (ht-poh.50), the relay reserves the worst-case cost (ht-poh.51), and the attention-block command is built from `marker_prefix` (ht-poh.49).

**Cross-task seams I checked and found correct:**
- The delivery cycle works end to end. A pull records ids in the offered ledger. The scheduler gate holds a wake while a re-offer is not yet due, and sends a wake only when the next pull would deliver something. An id offered 3 times is parked for `inbox`, so the wake falls back to the marker that names `inbox`. Otherwise the wake types `wake up`, the `PreInvocation` hook pulls, and confirm settles at `Stop`.
- On a binding-generation rotation, the hook state and the daemon ledger both discard the old offers.
- The R17 header change does not break transcript confirmation, because the scan keys only on per-item header lines.

**New findings:**
- **F1. A content-free wake has no fallback guidance.** The SKILL never explains a bare `herdr-threads: wake up`. If the wake's turn gets no injection, the model sees a prompt with no content and no instruction, and the wake ladder repeats it. That happens when:
  - the pull is refused or busy;
  - the 1.5 s hook budget runs out;
  - the agy plugin is disabled.

  R18 does not require this guidance; it is a one-line SKILL addition.
- **F2. On resume, delivery can wait a turn.** The resume `invocationNum == 0` `PreInvocation` takes the lifecycle path, not the hook step. A resumed turn that ends after one model call gets its bodies only on the next turn or wake. This is consistent with the design.
- **F3. The Stop-time abort check does not ask whether our hook injected.** Any `failed to inject steps` error at `Stop` downgrades the agy identity, even when another plugin caused it. This fails safe (downgrade to `userMessage`).
- **F4. ht-poh.46 raised a wait instead of fixing the cause.** It raised the `handoff_topology_cli` WAIT from 15 s to 60 s because CLI and helper exits took up to 20.7 s under load. The AGENTS.md speed-budget rule says to fix slow tests rather than raise budgets. The ledger says main has the same weakness.
- **F5. One CI gap:** `scripts/tests/test_outside_adapter_report.py` is not run in CI.

## Step 2: ledger triage

- **Recurring `[missing-edge-case-test]` ×4, across tasks ht-poh.2.1, 4.3, 5 and 50.** The class is: behaviour that only a real environment shows is checked with synthetic fixtures (interruption paths, legacy records, real process parents). The post-tool-call instance is now fixed. The legacy record without `probation_at` is still untested, but the code handles it (`is_none_or` re-downgrades).
- **Recurring `[tdd-order]` ×3.** This is a pipeline defect, and the rate is higher than the count. The same smell appears under `no-red-run`, `tdd-red-skipped` and `process-note` labels. It does not block landing.
- **BLOCKED-AUTH, Task 39 (ht-poh.13).** The re-dispatch ht-poh.41 restored this coverage once the human ran the goldens generator.
- **No parked completion lines** (Metrics: parked 0).
- **Three minors were misgraded or are now resolved:**
  - ht-poh.41 `[unrun-ts-tests]` and `[unrun-checks]`: I ran these checks and they pass, so these are resolved.
  - ht-poh.4.4 and ht-poh.11: `golden_surfaces::handoff_bootstrap_plans` is still flaky (ht-01s open). That file is new on this branch, so the flake is branch-introduced and not root-caused. ht-poh.46's "Done when" (20 green runs, or shown pre-existing with a bead) is not met for it. This is must-fix.
  - ht-poh.42 and ht-poh.46: two flakes were never classified. `cooperative_overdue_warning_wakes_idle_native_agent_exactly_once` lives in `tests/scheduler/dispatch.rs`, which this branch edited (ActiveTurn removal, hook-pull wake gate), so calling it "unrelated" is not established. `pending_public_retry_from_different_valid_caller_is_silent` has no bead. Both are must-classify.

## Must fix before landing

1. **Your full-suite sweep** (`cargo nextest run --all-targets --all-features`) plus `check-no-leaked-processes`, with the 300 s budget checked; the last measured run took 282 s.
2. **ht-01s:** root-cause and fix `golden_surfaces::handoff_bootstrap_plans`, for example one fixture per (kind, mode).
3. **Classify the two unclassified flakes** under sweep load against main, and file beads.
4. **ht-poh.6.9:** the full native gate (every flow, including delivery, downgrade and wake) and `agy-live-smoke --live` at the final SHA, with records committed. This is the first time the R17 frame and the R18 wake text run together on genuine agy. Before running it, make `live_smoke.execute` survive a `World.__enter__` failure. Right now that failure skips the leak check and writes no evidence.
5. **The R13 outside-adapter report** generated at the final SHA (phase 6). 25 non-merge commits lack an `Outside-adapter:` trailer; the report lists them, and the diff stays authoritative.
6. **Planned code roast round 2** (`handoff-continuation.md`), before the fix loop exits.

Cheap to bundle: F1 (SKILL guidance for a bare `wake up`), and F5 (add the report tool's test to CI).

## Untested scope

- **No full-suite measurement at `54fb8d0a`.**
- **Never exercised on genuine agy at the final SHA:** the R17 frame (the genuine captures predate it), the R18 wake together with injection, and the live smoke with a real model.
- **Paths never exercised for real:**
  - the `proc_pidpath` identity with a real agy parent;
  - the 24 h probation retry;
  - a legacy probation record without `probation_at`;
  - the Linux `/proc/<ppid>/exe` path.
- **Single live passes only:** the wake flow (215-385 s); phase (g) never waits for the agent row to clear.
- **Fixed by a raised timeout, not a root cause:** the `handoff_topology_cli` flakes (ht-vrg, ht-2w9).
- **BLOCKED-AUTH:** none left uncovered.

## Deferred OK

These can land as is or as follow-ups:
- F2 (delivery waits a turn after resume) and F3 (abort-marker attribution).
- In the plugin setup, the plugin is installed before the allow entry (a re-run resumes).
- The `doctor --debug` per-harness "none" lines change; document it.
- The temp `.db` files that `tests/store/wake_on.rs` never removes.
- The fake upstream's `HEADER_RE` does not match quoted thread names (dry-run only).
- The stale column name `wake_work.last_lazy_ordinal`.
- The `run_admitted_hook` input asymmetry.
- Relay and fake-server edge cases, and gate assertion looseness.
- The `downgrade.json` read-modify-write race.
- The replayed confirm answering `not_offered`.
- The composer live-region heuristic (it fails safe to `Unreadable`).
- v1 `HookParseFailure` cannot report agy (frozen wire).
- Doc and comment wrapping nits.

Files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-f3431b31..54fb8d0a.diff
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/harness/agy/delivery.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/service/delivery_pull.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/scheduler/mod.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/integrations/skill/SKILL.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/tests/golden_surfaces.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/scripts/agy-gate/live_smoke.py
