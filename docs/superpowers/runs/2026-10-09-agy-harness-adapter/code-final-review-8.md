**Verdict: not ready.** I found no Critical defect. One Important seam needs a fix or a recorded accepted limit. The branch has never had a full-suite run: no full-suite measurement of `super-auto/agy-harness-adapter` exists yet, and the caller runs that sweep after this review, so the branch is not tested as a whole. A flaky test the branch added is still open.

HEAD is `417c534f`; the fork point is `7c40c83c` (main).

Checks I ran at HEAD, all passing:
- `cargo fmt --check` and `cargo clippy --locked --all-targets --all-features -D warnings` are clean.
- `scripts/check-harness-neutral`: 0 violations, 85 allowlisted (49 frozen-wire, 2 history, 34 registration, 0 to-migrate).
- Focused lib tests filtered on `delivery_pull wake_on reject agy`: 398 passed. `check-no-leaked-processes` was clean afterwards.

Final-SHA evidence on the branch:
- The native gate ran at `c2f51b51`, which is the final code (later commits are evidence only). All 14 flows passed on genuine agy 1.3.2 with Herdr 0.9.1, so the R17 frame and the R18 `wake up` ran together for the first time.
- The live smoke passed at `4d2ec663`: 7 requests, $0.022. It covered wake, injection, ACK and reply, with `cooperative_hook_delivery` settlement.

## Step 1: my own review

**Seams I checked and found correct:**
- **Truncation** is enforced twice. The hook tracks only `page.offered`, and the store refuses truncated ids (`a_truncated_id_is_listed_truncated_and_refused_on_confirm`).
- **Pull and confirm** are decided against the canonical binding. On a generation rotation, both the hook state and the daemon ledger drop their old offers.
- **Wake routing:**
  - The scheduler holds a hook-pull wake while a re-offer is not yet due.
  - The wake types the `MARKER` text when ids are parked for `inbox`; otherwise it types `wake up`.
  - Claude's channel suppression uses `idle_wake == Channel`.
  - `wake_on: all` re-reads the lazy frontier before it reserves a wake.
- **Bootstrap V2:** V1 plans stay byte-identical (`composed_argv` and `startup_input` are skipped when absent). The journal accepts version 1 or 2 only when it matches the payload. `handoff --new-tab --kind agy` is fixed (`85549582`) and covered.
- **Role witnesses** fail toward child, and the conversation-state file is never used as lifecycle authority.
- **CI** runs `check-harness-neutral` with its tests, plus every `scripts/agy-*` Python suite.

**New findings:**
- **S1 (Important). An agy session never sees informational notices between lifecycle check-ins.** This includes main v0.6.1's invitation-rejection notices, which this branch absorbed.
  - The cause:
    - The agy `PreInvocation` and `Stop` hooks only run the delivery step (`src/cli/hook.rs:3113`); they never run a check-in.
    - `DeliveryPull` skips `InformationalNotice` items (`src/delivery/page.rs:90`).
    - The pull's fingerprint counts only invitations and waking warnings (`src/store/seats.rs:4640`).
  - So rejection reasons, and other seats' overdue or cleared transitions, reach an agy session only at `SessionStart`, after `/clear`, or when the agent runs `inbox` itself.
  - Claude still shows these notices at every tool-boundary check-in. The absorbed SKILL text and TRUST-POLICY A7 promise delivery "at their next hook/check-in", which is wrong for agy. Neither TRUST-POLICY nor the agy README records this as a limit.
  - Two fixes: carry notices in the pull (or raise the attention block while notices are pending), or record an accepted limit in A7, `integrations/agy/README.md` and the SKILL. AGENTS.md requires an accepted limit to be written down.
- **S2. The R13 report counts main's own changes as agy work.** `outside-adapter-report.md` measures `f3431b31..1eaf635b` (the original base). That range includes the absorbed main v0.6.1/v0.6.2 changes (CHANGELOG, Cargo, canary, invitation-rejection docs and store edits). They are counted as outside-adapter changes: 711 files instead of 676, about +1.4k lines. It also has no "before" count from ht-poh.1's initial to-migrate inventory, so the "allowlist delta" is measured against zero.
- **S3. Still no fallback for an empty wake.** Nothing in the SKILL tells a model what to do with a bare `herdr-threads: wake up` that arrives with no injected items (the pull was refused or busy, or the 1.5 s budget ran out). Prior review F1 is still open. This is a one-line SKILL addition.
- **S4. Downgrade misses can collide across conversations.** `DowngradeRecord.misses` stores per-conversation turn numbers but is shared per executable identity. Misses on turn 1 of two different conversations count as one, which delays a downgrade (`src/harness/agy/delivery.rs`, `stop`). This fails safe.

## Step 2: ledger triage

**Recurring clusters, triaged as classes:**
- **`[missing-edge-case-test]` ×4 (ht-poh.2.1, 4.3, 5, 50).** The class: behaviour that only a real agy shows is checked only with synthetic fixtures. The final-SHA gate now covers most of it. Still uncovered: the legacy probation record without `probation_at` (the code handles it), the 24 h probation, the real `proc_pidpath` parent, and Linux `/proc`. Not blocking.
- **`[tdd-order]` ×3, plus the same smell as `no-red-run`, `tdd-red-skipped` and `process-note`.** A pipeline defect: implementers don't capture a failing run first. Not blocking.
- **`[doc-drift]` ×3 (ht-poh.7, 4.8, 6.9).** Docs and notes describe state that differs from what shipped. The live instances are the wrong agy sha512 prefix in the 6.9 orchestrator note and the R13 range ending at `1eaf635b` (S2). Fix both in one doc pass.

**Minors that were misgraded, so must-fix:**
- **ht-01s (open): `golden_surfaces::handoff_bootstrap_plans` is flaky.** The file is new on this branch, and the flake has no root cause.
- **Two flakes are unclassified and have no bead.** I searched the tracker for both:
  - `cooperative_overdue_warning_wakes_idle_native_agent_exactly_once`, in `tests/scheduler/dispatch.rs`. This branch edited that file (ActiveTurn removal, hook-pull wake gate), so "unrelated" is not established.
  - `pending_public_retry_from_different_valid_caller_is_silent`.
- **ht-veq, ht-vrg and ht-2w9 (open) are flakes the run saw.** ht-zo4 (the flakiness side quest) is closed, so these are real regressions to classify against main, not background noise.

**BLOCKED-AUTH:** Task 39 (ht-poh.13). Its coverage was restored by ht-poh.41, and the prior review ran `scripts/test-claude-mod` (95 passed). The kept worktree `task-ht-poh.13` still holds the uncommitted partial work and should be removed.

**Parked:** none (Metrics: parked 0). The one blocked fix pass, ht-poh.28 (a product defect in the resume flow), is resolved: the resume flow passes at the final SHA.

## Must fix before landing

1. **The caller's full-suite sweep:** `cargo nextest run --locked --all-targets --all-features` plus `check-no-leaked-processes --run-id`, with the 300 s budget checked. The last measured run took 282 s.
2. **ht-01s:** root-cause and fix `handoff_bootstrap_plans`.
3. **Classify the two unclassified flakes** under suite load against main, and file beads. Re-check ht-veq, ht-vrg and ht-2w9 now that ht-zo4 is closed.
4. **S1:** deliver informational notices through the agy pull, or have the human accept the limit and record it in TRUST-POLICY A7, the agy README and the SKILL rejection text.
5. **S2:** regenerate the R13 report over `7c40c83c..HEAD`, with the before-count taken from ht-poh.1's initial allowlist.

## Untested scope

- **No full-suite measurement of this branch at `417c534f` exists.**
- **Never exercised for real:**
  - the 24 h probation retry;
  - a legacy probation record without `probation_at`;
  - the real `proc_pidpath` identity;
  - the Linux `/proc/<ppid>/exe` path;
  - `--sandbox`.
- **Informational and rejection notices on agy:** no test or gate flow covers them (S1).
- **The wake flow (215 s) has single live passes only,** and phase (g) never waits for the agent row to clear.
- **The `handoff_topology_cli` flakes were masked** by raising the wait from 15 s to 60 s (ht-poh.46), against the AGENTS.md speed-budget rule.
- **The doctor `--debug` output change** (it no longer prints `hooks.<other>: none` lines) has no test or golden.

## Deferred OK

- S3 (one SKILL line for a bare `wake up`; cheap, worth bundling) and S4 (the misses collision fails safe).
- Prior F2 (delivery waits a turn after resume) and F3 (any abort marker at `Stop` downgrades, even when another plugin caused it).
- `find_seat_for` ignores the harness in `selected_generation`. This is pre-existing behaviour, and the daemon decides.
- The stale column name `wake_work.last_lazy_ordinal`; the temp `.db` files in `tests/store/wake_on.rs`.
- The `run_admitted_hook` input asymmetry; the composer live-region heuristic (it fails safe to `Unreadable`).
- The `downgrade.json` read-modify-write race; the replayed confirm answering `not_offered`.
- Relay and fake-server edge cases (`HEADER_RE` in the dry run).
- 25 commits without an `Outside-adapter:` trailer (the diff stays authoritative).
- The foreign-leak note in the gate record (a clean follow-up was recorded).
- Doc wrapping and comment nits; v1 `HookParseFailure` cannot report agy (frozen wire).

Files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-7c40c83c..417c534f.diff
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/cli/hook.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/delivery/page.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/store/seats.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/harness/agy/delivery.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/TRUST-POLICY.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/integrations/agy/README.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/integrations/skill/SKILL.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/outside-adapter-report.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/tests/golden_surfaces.rs
