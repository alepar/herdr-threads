**Verdict: not ready.** I found no code defect that blocks landing, and every must-fix item from the previous final review (wf_b9b4d982-1c2) has landed. The branch still can't land because nothing tested it as a whole at HEAD 5b79084c: there is no full-suite run, and the wake e2e predates the latest rewrite of the wake rule.

## Step 1: my own review against the spec

**Static checks I ran at HEAD 5b79084c, all passing:**
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`
- `nice scripts/check-default-features` (exit 0)
- `scripts/check-harness-neutral` (0 violations)
- `cargo fmt --check`

The run's own merge check was only `cargo check`, so these are the first clippy and default-feature results for HEAD.

**Must-fix items from the previous review, now landed:**
- **TRUST-POLICY.md under D9 (ht-yha.70):** the old wording at line 1105 is fixed. A bare `--nudge` is now described as a p0 delivery row with no receipt. The view seed's reliance on "an open binding has a cursor in every joined thread" is now written down, along with the rule that any change breaking it must extend the seed in the same commit.
- **Order keys no longer re-key between pages (ht-yha.71):** `effective_receipt_with_key_deadline` (`src/store/effective.rs`) keys a receipt by its stored deadline, else its send-time projection. I checked every writer of a stored deadline:
  - `materialization.rs:80`
  - `materialization.rs:724` (`project_receipt`)
  - `receipts.rs:592/602` (ACK)
  - `control.rs:275` (retire)
  
  For a pending receipt with no stored deadline, each of them writes the effective pending deadline, which equals the projection. So the key stays the same when a deadline gets stored. The one exception is a manifest receipt sent before the cutover whose seat retired before the cutover, which the task report already discloses.
- **p0 marker refused at `--max-bytes` 256–342 (ht-yha.72):** this is pinned by a test; the fix itself was 932255d9.
- **ht-cbb** is closed.

**Items from the main review (main-review-b6ad43fe.md), now landed:**
- I1 → ht-yha.60, I2 → .61, I3 → .64, I5 and I6 → .63.
- **I4 (ht-yha.62, a sibling's display blocking a one-process wake):** the semantics are right. Receipts and invitations are still owed only while every reached binding is unserved, which keeps the old rule. The delivery-row (`lazy`) frontier is now owed while any reached binding still has it unread. `wake_owed_to_binding` remains the single-binding case.

**Seams I found myself:**
1. **The wake e2e evidence is out of date.** The only passing run (114/114) is at 744ca90a. Since then, ht-yha.62 rewrote the one-process wake decision (`wake_owed_to_reached`, `ReservationCheck::owes_wake` signature, dispatch). ht-yha.63/.71 changed display marks and deadline keys. The new `side-nudge` sibling phase has never run live.
2. **Speed budget.** The last complete sweep (9de1002a) took 378 s against the 300 s budget. It also had a failure that no bead covers: `cli::topology_handoff::tests::live::downstream_new_thread_create_links_parent_child_in_same_real_transaction` (FAIL, 0.58 s). It also had the release-build failpoints timeout, which ht-wur covers.
3. **The design spec's ht-yha.45 note (line ~314) is stale.** It still describes the per-binding `lazy` refusal and doesn't mention that ht-yha.62 reversed it for one-process wakes. It's a historical doc, so this is minor.

## Step 2: ledger triage

**Recurring clusters**
- **[doc-drift], ×9 across 7 tasks:** a pipeline defect. Task file lists kept shared normative docs out of scope, and no task owned them until ht-yha.64 and ht-yha.70. Both normative gaps are now closed in TRUST-POLICY.md. Only the design-spec line in seam 3 remains.
- **[missing-edge-case-test], ×5 across 5 tasks:** one systemic smell. Boundary combinations get noticed during review, but no task owns pinning them. None names a skipped test, a product defect or a data-loss path. The open instances:
  - A bare p0 send to a pending pair invitee makes no receipt and one p0 row (ht-yha.70). This pins TRUST-POLICY text that was just written.
  - The service-send legacy-receipt resume path (`service_send.rs:357`) has no test of its own.
  - The mixed receipt+invitation semantics of `none_offered` (ht-yha.62) are unpinned.
  - The TopologyHandoff route of the M5 test (ht-yha.65) is not covered.

**Parked findings:** none. The one BLOCKED fix pass was resolved earlier.

**BLOCKED-AUTH lines:** none.

**Metrics**
- landed≠completed 56 vs 53: the three extra "landed" lines are ht-yha.1, .5.1 and .47, re-seeded on every Resume. All three are CLOSED in the tracker, so this is a ledger-read artifact, not missing work.
- One invocation's metrics are UNAVAILABLE (ledger lines 472-475), so the counts are incomplete for that invocation.
- merge-failed 1 is ht-xml, which is closed. Seam reviews: 9, fixed 0.

**Minors I checked:**
- **[unverified-key-transition] (ht-yha.71):** I checked the writers. Not a defect beyond the disclosed retired-before-cutover case.
- **[order-semantics-change] (ht-yha.63):** the 2026-10-15 spec note now covers it.
- **[accepted-regression] (ht-yha.66):** disclosed in its commit.

None of the deferred minors needs to be re-graded as must-fix.

## Must fix before landing
1. **Run the full suite at 5b79084c.** Use `nice cargo nextest run --locked --all-targets --all-features` with `HT_LEAK_RUN_ID`, then `scripts/check-no-leaked-processes --run-id`. No full-suite measurement of this branch's HEAD exists; the caller's sweep is the first. Check wall-clock against the 300 s budget, since the last sweep took 378 s. Triage the topology_handoff `downstream_new_thread_create_links_parent_child_in_same_real_transaction` failure from sweep 9de1002a: it has no bead.
2. **Re-run the wake e2e at HEAD**, including the opt-in `--harness codex --cases side-nudge/group,side-draft/group`. Before landing, this run (or an explicit user deferral to the release gate) is needed because the wake rule changed after 744ca90a. AGENTS.md makes it required before any `v*` tag either way.

## Untested scope
- **No full-suite run at HEAD.** The last sweep (9de1002a) predates the main merge and ht-yha.60–.72.
- **The wake e2e** has not run since 744ca90a. The ht-yha.62 sibling phase has never run live.
- **The deadline-window rework (ht-yha.63/.71)** has only focused-test evidence.
- **Upgrading a real v0.11 state dir** has only been tested on fixtures.
- **`migrations/0047_message_priority.sql` was edited after earlier branch builds** (778fb055, 20a69676). Any state dir migrated by an earlier branch build has different v47 DDL and should be discarded. This is acceptable only because v47 is unreleased.
- **The Linux Docker e2e** was not run (optional).

## Deferred OK (file beads)
- **Unpinned edge-case tests:** the four from the [missing-edge-case-test] cluster above.
- **Doc fix:** the spec's ht-yha.45 note should say that ht-yha.62 reversed it for one-process wakes.
- **Speed:**
  - `deadline_traversals_show_every_item_exactly_once` takes 6–36 s.
  - The 0047 cursor triggers cost O(view size) per cursor advance.
- **Small test leftovers:**
  - A weak boundary case in `tests/cli/lazy_markers.rs`, and its `BoundedP0History` mock can drift from the real history fitting.
  - Duplicate `invite --lazy` checks in `tests/cli/commands.rs`.
- **Accepted by design:**
  - Service sends with explicit recipients lose the `[p0]` read marker.
  - Deadline-class order is exact only within one 64-item window.
- **Existing flake beads stay open:** ht-3wm, ht-kk8, ht-wur.

Files reviewed:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-a0f93109..5b79084c.diff
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery-design.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/progress.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/effective.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/wake.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/queries/inbox_v2/deadline_window.rs