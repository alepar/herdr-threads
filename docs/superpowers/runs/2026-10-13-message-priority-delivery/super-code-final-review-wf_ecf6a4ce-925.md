**Verdict: not ready.** No full-suite run exists for this branch (HEAD 79a20c72). The sweep is deferred to the caller, so the branch is untested as a whole. The release-gate wake e2e has not run either. The branch is also 20 commits behind `main` (v0.11.0), and merging `main` conflicts.

## Step 1: my own review against the spec

**A. Rows can stay in the `state='pending'` partial index forever. Important; confirmed by reading the code, no test covers it.**
- The ordered seeks walk only `state='pending'` rows:
  - `seek_pending` in `src/store/lazy_delivery/inbox.rs`;
  - `wake_frontier`, `has_wake_rows` and `oldest_pending_at` in `src/store/lazy_delivery/wake.rs`.
- `state` flips to `displayed` in only one place: `MARK_DISPLAYED_SQL` inside the top-level `complete_displayed_for_binding` (`src/store/lazy_delivery/completion.rs`).
- ht-yha.38 narrowed F1 from the earlier final review but did not remove it. Two paths still leave rows pending:
  1. **A process-exited binding that never comes back** (`seat_owes_display_sql!`, `src/store/lazy_delivery/mod.rs`). TRUST-POLICY says disconnection never ends a binding. agy sends no end event (`src/harness/agy/decode.rs:150`, `binding_event: None`), so every agy session exit leaves such a binding. Claude and Codex leave one after a crash or a killed pane. From then on, every new row in the seat's joined threads stays pending.
  2. **Displays that only move the cursor.** A top-level `body` read (`advance_over_body_read`, `src/cli/mod.rs:1352`) calls `AdvanceCursors` and never reaches completion. This is the normal path for a truncated message over 8 KiB, because the hook completes only messages shown whole.
- **Consequence:** every first page starts at `(-1,0,0)` (`seek_pending`) and spends one visit per stale row against `CANDIDATE_LIMIT`=100 (`src/store/queries.rs:53`). Once about 100 pile up:
  - CLI `inbox` and `hook_inbox` first pages come back with no rows and only a `next:` line;
  - delivery pull walks the whole backlog on every pull;
  - the wake-discovery probe `has_wake_rows` scans them with no budget, for every live seat on every pass.
- The new accepted limit in TRUST-POLICY.md:784 understates this ("the ordered seeks skip them within their visit bound"). A fix pass added it, not the user. This is the same root cause as F2 in the earlier review: a seat-wide `state` column decides per-binding delivery.

**B. The branch must merge `main` (v0.11.0), and that merge has a semantic seam.**
- A trial merge (`git merge-tree`) conflicts in `AGENTS.md`, `TRUST-POLICY.md` and `scripts/wake-e2e.py`.
- The seam: `main`'s ht-327 one-process composer wake adds `WakeReservation::for_seat_attention`, which exempts only `pending_invitation`, `pending_receipt`, `actionable_warning` and `soft_deadline` from the multi-target gate (`main:src/ports.rs`, `main:src/notification/dispatch.rs`).
- Under D9, a bare `--nudge` (p0, no ACK) no longer creates receipts. It wakes through the `lazy` reason instead.
- After the merge, a p0 send to a Codex seat with forked sessions in one process would be refused `Unsafe`. On `main` today the same send still wakes, because it creates a `pending_receipt`.
- Someone has to decide whether p0 delivery-row wakes count as seat attention for that path. `main`'s wake-e2e side cases also have to be folded into the new matrix.

**C. Two open decisions were closed by the pipeline, not by the user.**
- **ht-yha.33:** the bead's Done-when still requires a format-1-compatible token. The pipeline implemented option (c) instead (an old CLI cannot follow `next:`; recorded in spec §Compatibility line 151 and `docs/agent-usage.md`). The run log says "escalated" and "amended", but no user decision is recorded.
- **ServiceSend vs D9:** ServiceSend still keeps receipts for its whole audience (`AckAudience::Everyone`, `src/store/service_send.rs`). It is still parked, and neither TRUST-POLICY.md nor the spec mentions it.

**D. Minor doc drift.** `docs/agent-usage.md:79` still says a human seat named by `--require-ack` gets a "human receipt". Under D9 it gets a priority-0 delivery row.

**Checked and found consistent:**
- the v47 migration, its backfills and triggers;
- `DeliveryPriority`, `NudgeThreshold` and `DeliveryOrderKey`, including the compact codec;
- D9 send staging: the explicit-recipient dedupe, human ACK recipients getting p0 rows, the p0 unavailable-recipient warning, and the manifest trigger that allows it;
- `note_published`: the `decision_seq` copy and the wake work gated by the threshold;
- re-reading the threshold on every wake decision (D2);
- pair auto-join through `record_join`;
- the invitation policy;
- the CLI priority rules and the probe that refuses rather than downgrades;
- `batch_key` and its JS mirror, and the mod's passive/wake rule;
- the old-wire `MessageDeliveryModes` gate;
- `scripts/check-harness-neutral` passes (0 violations). No `#[ignore]` or skipped tests were added.

## Step 2: ledger triage

- **Recurring minor `[doc-drift]` (×3 across 3 tasks).** This is a pipeline defect: task file lists left out the shared docs, so drift was deferred to "an owner". ht-yha.34 and ht-yha.39 closed most instances. Left: D, `CHANGELOG.md:46` (historical, fine), and three `wake_on: all` doc comments.
- **Minors that are really defects:**
  - Task 15 `[vacuous-check]`: `markers_ok` (`scripts/wake-e2e.py:192`) still passes when no header is on screen. A release-gate check that can pass without testing anything is must-fix.
  - Task 9 `[state-dependent-wake-seek]`: the two-connected-bindings case is fixed by `MARK_DISPLAYED_SQL`, but the same class survives as A.
  - Task 7 `[interim-wake-gap]` (wake assertions weakened, "to be restored by Task 9"): only indirectly re-covered, since Task 16 `[indirect-wake-check]` reads the digest rather than the scheduler's wake candidates. Not proven restored.
- **Already resolved:** the pre-existing test failures from Tasks 2, 11 and 12 (fixed in 5170075c), F3 and Task 23's SKILL.md/help.rs drift (226b5094), and Task 14's `drain_pages` error path (superseded by 137f697e).
- **Parked and blocked items:** no `parked` lines. Task 8's BLOCKED fix pass was about missing report evidence, not code. No `BLOCKED-AUTH` lines.
- **Ledger completeness:** "landed≠completed 28 vs 25" is the three `landed (tracker closure unconfirmed)` lines for ht-yha.14, ht-yha.5.1 and ht-yha.1. The tracker shows all 48 beads closed. Read the ledger as incomplete, not as clean.

## Verdict
Not ready. A causes user-visible inbox and hook degradation on a common path (every agy restart, every long-message `body` read). B blocks landing on current `main`. C needs two user decisions. No full-suite or wake-e2e run exists.

## Must fix before landing
1. **A.** Define one cheap rule for when a row leaves the pending view. Either flip `state` (or prune the row) on the cursor-only display paths and when an owing binding can no longer return, or give the seek a per-binding lower bound so already-displayed rows cost nothing. Rewrite TRUST-POLICY.md:784 to match. Add tests:
   - a never-returning process-exited sibling plus more than 100 rows the live binding displayed: the first inbox and hook page still shows the new row;
   - a `body` read of a truncated delivery row leaves nothing in `lazy_recipients_pending_order`;
   - `has_wake_rows` stays bounded.
2. **B.** Merge `main` and resolve the conflicts in `AGENTS.md`, `TRUST-POLICY.md` and `scripts/wake-e2e.py`. Decide `for_seat_attention` for the `lazy` (p0 delivery-row) reason, and add a test for a one-process, multi-binding p0 `--nudge` wake.
3. **C.** The user ratifies ht-yha.33 option (c) or picks another scheme, and the bead's Done-when is aligned with what shipped. The user decides ServiceSend vs D9, and the decision goes into TRUST-POLICY.md and the spec.
4. **`markers_ok`.** Make it fail when no expected header is on screen.
5. **Full suite.** Run `nextest --all-features` plus `scripts/check-default-features` and `scripts/check-no-leaked-processes --run-id`, after the `main` merge. This also settles the flake claims made without base runs (tasks 4, 5, 12, 13).

## Untested scope
- No full suite at HEAD, and none after a `main` merge.
- The wake e2e (release gate: real agents, run by a person) has not run against the new T × priority matrix, and its marker check can pass vacuously.
- The optional Linux e2e has not run.
- A has no tests: stale process-exit binding plus backlog, `body`-read rows, the cost of the discovery probe.
- B: the p0 delivery-row wake on a one-process, multi-binding seat.
- Wake for a bare `--nudge` is asserted through the digest, not scheduler candidates (Tasks 7/16).
- Pair first send is covered only at p1 (Task 16).
- The mod's passive/wake split by priority is missing from the cross-surface fixture (Task 18).
- The capability sweep checks the `ADVERTISED` constant, not the running daemon's reply (Task 19).
- The ht-yha.33 limit is tested only against a hand copy of the old decoder.

## Deferred OK
- **Test-shape and evidence minors:** Task 2 (lines 12/13/46/47), Task 6 (28–31), Task 3 (8/41/42), 79, 87, 88, 145–147, 158, 160, 161, 192, 202, 226.
- **Code-hygiene minors:** 64–66, 74, 95, 115, 116, 135, 196, 197, 201, 208.
- **Others:**
  - `LEGACY_PAIR_MARKER`, the test-only branch (121): remove it with the last pre-upgrade fixture.
  - `operator_orphan_invite` storing a NULL confirmation (122): documented.
  - Human handoff keeping Lazy and the p0 legacy spelling (89/123): only matters against old daemons.
  - The receipts-capability probe skipped for priority-shaped plans (86).
  - `mod_stall_oldest` still reading `delivery_mode='ordinary'`: equivalent, because a receipt implies the ordinary mode.
  - `nudge-threshold` treating a failed probe as an old daemon (F4 in the earlier review): no value is lost.
  - Old CLIs rendering `[lazy]` for p0 non-ACK messages, through `delivery_mode` as stored.
  - `complete_addressed`, now unused (`completion.rs`): remove it, since it bypasses `MARK_DISPLAYED_SQL`.
  - `CHANGELOG.md:46` historical wording, and D.

Files:
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-3a1203e0..79a20c72.diff`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/lazy_delivery/inbox.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/lazy_delivery/mod.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/lazy_delivery/completion.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/lazy_delivery/wake.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/cli/mod.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/service_send.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/scripts/wake-e2e.py`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/TRUST-POLICY.md`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/docs/agent-usage.md`