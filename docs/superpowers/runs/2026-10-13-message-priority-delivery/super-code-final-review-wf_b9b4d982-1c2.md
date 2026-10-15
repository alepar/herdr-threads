**Verdict: not ready.** I found no code defect that blocks the branch, and the remaining code changes are small. It still can't land yet: no full-suite run exists at HEAD 376c0c0e, the pre-merge checks haven't been run there, and three items below need fixing first. The full-suite sweep is the caller's next step, so nothing here should be read as "tested".

## Step 1: my own review against the spec

Checked and consistent with the spec:
- **Priority rules (D3/D4, D10):** `DeliveryPriority` and `NudgeThreshold` in `src/protocol/priority.rs` match the spec. CLI flag resolution (`src/cli/send_priority.rs`) and the daemon-side refusals (`src/protocol/commands.rs:1033-1036`) both implement D3 (ACK forces p0) and "deadline needs explicit ACK recipients".
- **Old-intent retry (D10):** `resolved_priority` (`src/store/messages.rs:96`) and the digest covering `priority` only when the request carries it are both right.
- **Wake rule (D2):** `wake_frontier` and `has_wake_rows` (`src/store/lazy_delivery/wake.rs`) do one seek per connected top-level view and admitted priority, and read T at wake time.
- **Disconnects:** every liveness change goes through `src/store/bindings.rs:495/508/564`, which drops the view on disconnect and re-seeds it on reconnect. The v47 triggers cover binding end.
- **Deletes vs the new view tables:** `lazy_recipients` is deleted only for unpublished preparations (`src/store/materialization.rs:802`, `src/store/lazy_delivery/staging.rs:88`). The new `binding_pending` table only holds published rows, so these deletes can't hit its foreign key.
- **Ordering seams:**
  - The watch/hook batch sort (`batch_order`) and the mod's `batchOrder` in `register.js` agree.
  - Invitations now carry `deadline_at` and are ordered inside the batch.
  - The three invitation-ordering tests named in ht-cbb are present and no longer ignored.
- **Deadline windows (ht-yha.63):** `advance()` saves `window_last` after every emitted item, so `persist_partial` keeps the right skip key.
- **Static checks I ran at HEAD:** `scripts/check-harness-neutral` passes and `cargo fmt --check` is clean.

Seams I found myself:
1. **The wake e2e evidence is out of date.** The only passing run (114/114) is at 744ca90a. Since then the branch took the v0.11.0 main merge and ht-yha.60–.67, which rewrote the one-process wake rule (`wake_owed_to_reached` in `src/store/wake.rs`), edited the 0047 triggers, extended display marks to receipt rows (`src/store/read_cursors.rs`) and changed deadline-window resume. The new opt-in `/side` sibling check (ht-yha.62) has never run live.
2. **`migrations/0047_message_priority.sql` was edited after it had been run** (778fb055, 20a69676): one trigger was removed, `lazy_recipient_source` now checks priority, and the publish-trigger scope changed. A state dir migrated by an earlier branch build has different v47 DDL. This is harmless while v47 is unreleased, but no kept state dir should have been migrated by a pre-20a69676 build.
3. **The suite is over its speed budget.** The last two complete sweeps took 329 s (dae11184) and 378 s (9de1002a; 1 fail, 1 timeout), against the 5-minute budget in AGENTS.md. Neither covers HEAD.
4. **TRUST-POLICY.md:1105 is wrong under D9.** It still says "a direct `--nudge` target is a pending receipt from publication", but a bare `--nudge` now makes a p0 delivery row and no receipt.

## Step 2: ledger triage

**Recurring minors:**
- **[doc-drift], ×9 across 7 tasks.** This is a pipeline defect, not a code smell: task file lists forbade editing shared docs, and no task owned them until ht-yha.64. At HEAD, the model-visible cases I spot-checked are fixed (SKILL.md, help.rs, `AUTOMATIC_PENDING_GUIDANCE`, the mod README, the linux-e2e README, AGENTS.md). Two normative gaps remain: TRUST-POLICY.md:1105 (seam 4), and TRUST-POLICY doesn't record that the view seed relies on "an open binding has a cursor in every joined thread" (ht-yha.67). Stale "old-daemon mapping" lines remain only in the historical design spec.
- **[missing-edge-case-test], ×4 across 4 tasks.** These are boundary combinations reviewers noticed but nobody pinned. None is a correctness defect.
- **Pre-merge checks skipped, an unlabelled cluster.** ht-yha.44, .52 and .63 skipped `scripts/check-default-features`, and the run's merge check was only `cargo check`, with no clippy and no warnings-as-errors. Another pipeline gap.

**Parked findings:** none. The two BLOCKED decisions (ht-yha.33 and ht-yha.44) were resolved and landed.

**BLOCKED-AUTH lines:** none.

**Metrics:**
- The ledger is incomplete: landed≠completed is 53 vs 50, and lines 472-475 show one invocation's metrics UNAVAILABLE. The three "landed" re-seeds (ht-yha.1, .5.1, .47) are closed in the tracker.
- The one merge failure was the false blocker ht-xml, which is closed.

**Minors re-graded:**
- Ignored and failing tests are resolved:
  - The ignored-test fix passes (Tasks 5 and 18) left no `#[ignore]` in the diff.
  - `pair_accept_delivers_mixed_opening_mail_once`, the `q4_retirement_refusal_controls` tests and `config5` pass in sweep 9de1002a.
- Two minors name real product defects, so I graded them must-fix (below): Task 30's [magic-number-bound] and Task 50's [rare-rekey-duplicate].

## Must fix before landing
1. **Run the gates at HEAD:** the full nextest suite with `scripts/check-no-leaked-processes --run-id`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and `nice scripts/check-default-features`. None of these has evidence at 376c0c0e. Also check wall-clock against the 300 s budget, since the last two sweeps exceeded it.
2. **Fix TRUST-POLICY.md:1105** (seam 4), and add the binding-cursor invariant ht-yha.67 now relies on. TRUST-POLICY is normative and currently misstates behaviour.
3. **Task 50 [rare-rekey-duplicate]:** legacy or dormant receipts with no stored deadline can re-key between pages and be shown twice. That contradicts the 2026-10-15 spec note ("every item is shown exactly once") and there's no test. Either freeze that key or amend the note to an accepted at-least-once limit.
4. **Task 30 [magic-number-bound]:** the refit in `fit_annotated_read` (`src/cli/mod.rs`) gives up below about 342 bytes, even where 256 + 15 bytes would fit, so a p0-annotated `body`/`read` is refused at bounds 256–342. The planned clamp-to-256 fix is small. It's pre-existing for `[lazy]`, but the new marker widens it.
5. **Close ht-cbb.** It is still OPEN and labelled `blocker`, but its fix landed (Task 18's fix pass) and its tests aren't ignored.

## Untested scope
- **No full-suite run at HEAD.** The last sweep (9de1002a) predates the main merge and ht-yha.60–.67.
- **Wake e2e not re-run since 744ca90a** (seam 1). It's required before any `v*` tag and strongly advised before landing, because the wake rule changed. Its opt-in `side-nudge`/`side-draft` cases, including the ht-yha.62 sibling phase, have never run live.
- **The ht-yha.63 deadline-window rework** (frozen keys, receipt display marks) has only focused-test evidence. Its own report says check-default-features wasn't run.
- **Upgrading a real v0.11 state dir with live data** has only been exercised on fixtures. The service-send legacy-receipt resume path (`src/store/service_send.rs:357`) has no test of its own.
- **Linux Docker e2e:** not run (optional).

## Deferred OK (each should get a bead)
- **New flaky test:** `mod_delivery::stall_with_only_a_p0_delivery_row_hands_over_to_the_native_wake` failed under load in ht-yha.46, ht-yha.57 and sweep e6de8153. It needs hardening or a nextest group (ht-zo4 class).
- **Speed:** Task 50's `deadline_traversals_show_every_item_exactly_once` takes 6–36 s, and the 0047 cursor triggers cost O(view size) per advance (Task 32).
- **Task 41:** `ReceiptScanScope::Seat` callers (`inbox_batch` receipt step, `pending_receipts`) can still walk settled or unpublished history. This is a performance issue in the same class as the config5 bug, not a correctness one.
- **Task 49 [mixed-reason-semantics]:** I judge this correct, since receipts and invitations are seat-wide items, but it should be pinned by a test.
- **Small leftovers:**
  - Task 52: service sends with explicit recipients lose the `[p0]` read marker (disclosed in the commit).
  - Task 44: the seat-wide displayed mark is no longer read for any decision.
  - Task 12: the test-only `LEGACY_PAIR_MARKER` branch remains.
- **Deadline-class order is exact only within one 64-item window.** This is by design in the inbox-merge subspec and noted in the spec.
- **Process-evidence minors** (no RED run shown, flakes without a base run, garbled comments, test fixtures edited outside a task's file list): not code defects.
- **Migration 0047 edited after it ran** (seam 2): acceptable while v47 is unreleased.

Files reviewed: the package at `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-a0f93109..376c0c0e.diff`, the spec at `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery-design.md`, and the ledger at `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/progress.md`.