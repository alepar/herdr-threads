**Verdict: not ready.**

Branch `super-auto/message-priority-delivery` @ d5e4c65b, fork point 998d7d34 (main). Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-998d7d34..d5e4c65b.diff`

This branch has no full-suite run at HEAD. The caller runs the sweep after this review, so the branch is not yet tested. The last sweep (at dae11184, in `sweep-dae11184.log`) had 3 failures and took 329 s. That is over the 300 s budget in AGENTS.md; earlier runs' sweeps took 224–290 s.

What I ran at HEAD:
- `scripts/check-harness-neutral`: ok, 0 violations.
- `wire_compat::cli_against_an_old_fixture_daemon_never_sends_full_bodies` (the ht-yha.42 target): passes.

Only one code commit (8eed145d, a test change) landed after the previous final review (wf_4875ea42, at 970ed3f5). Its other must-fix items are still open.

## Step 1: my own review against the spec

**A. Must fix. Delivery rows that every reader has seen can stay in the pending index forever, and once there are about 100 of them a reader's first page comes back empty.** The previous review found this; I checked it again myself.
- `MARK_DISPLAYED_SQL` (`src/store/lazy_delivery/completion.rs:70`) is the only thing that sets `state='displayed'`. It runs only from `complete_displayed_for_binding`, and only for the messages in that one completion.
- The cursor-only path never runs it: `advance_cursors` (`src/store/control.rs:2821`) handles `body` reads, and also the hook's `AdvanceCursors` when the `CompleteInboxDelivery` that follows it fails (`src/cli/hook_inbox.rs:335-360`).
- A binding disconnected by process exit counts as owing the row (`seat_owes_display_sql!`, `src/store/lazy_delivery/mod.rs:128`). Rows the live sibling displayed meanwhile stay pending, even after that binding ends, because nothing marks them again.
- `seek_pending` (`src/store/lazy_delivery/inbox.rs:78`) drops these rows after the index seek, and each one costs a visit against `CANDIDATE_LIMIT`. `hook_inbox::read_page` reads only one first page (`src/cli/hook_inbox.rs:62`). So with more than about 90 stale rows ahead of new ones, the hook shows nothing, which breaks delivery contract rule 3.
- The wake seeks (`FRONTIER_SEEK_SQL` and `ANY_SEEK_SQL`, `src/store/lazy_delivery/wake.rs`) also walk every stale row on every probe.
- The accepted limit in `TRUST-POLICY.md:800` was written by a fix pass (ht-yha.38), not by the user.

**B. Must fix (new). A stalled mod channel no longer recovers for `--nudge` messages.**
- `mod_stall_oldest` (`src/store/seats.rs:5214`) counts only pending receipts. The stall rule in `src/service/mod_channels.rs:31` and `src/ports.rs:4028` requires one.
- Under D9, a bare `--nudge` send or the first p0 message of an auto-joined pair creates delivery rows and no receipt.
- A live channel suppresses native wakes (`src/service/mod_channels.rs:12-20`), and the stall close is the only way back to them.
- So a wedged but still registered mod holding a p0 row is never closed, and the seat never wakes. On main, `--nudge` created receipts for every joined agent, so this case was covered. Before this branch only opt-in `wake_on: all` lazy rows had the gap; now it is the default urgent path.
- This is the same class of seam as the `for_seat_attention` "lazy" merge fix. None of the earlier reviews caught it: they only noted the redundant `delivery_mode='ordinary'` filter.
- Fix: have the stall rule also count pending delivery rows that the seat's threshold admits. Or record it as an accepted limit with the user's sign-off.

**C. Should fix. A new client shows different markers depending on the daemon version.** Against an old daemon it still prints `[lazy]` (`src/cli/output.rs:408`) and never `[p0]`; against a new daemon it prints `[p0]` and never `[lazy]`. This only happens during an upgrade window.

**Seams I checked that compose correctly:**
- Send staging: only explicit non-human ACK recipients get receipts; a human named with `--require-ack` gets a p0 row; the unavailable-recipient warning is kept for p0.
- `note_published` copies `decision_seq` and writes `wake_work` only where `THRESHOLD_ADMITS_SQL` admits the priority; then `wake_frontier` seeks once per admitted priority, and the reserve step re-checks it.
- Migration 0047: trigger order, the backfills, and the priority-keyed `lazy_manifest_source`.
- `RECEIVABLE_WHERE` is limited to the seat's own `prepared_recipients`, so the Unread walk and the delivery rows show no duplicates.
- Pair auto-join through `record_join`, with the `first_cursor_sql!` join floor.
- The drain sorts within each page with stable ties, and the Rust `batch_key` matches the JS `orderKey` for current daemons.
- The mod's passive/wake decision follows `wakes` and `ackRequired`.
- The legacy `MutationSpec::into_command` path that drops a p > 1 priority is only reachable from tests; production sends go through `src/cli/mod.rs:3850` with the `send.priority_v1` probe.
- No `#[ignore]` was added.

## Step 2: ledger triage

**Recurring clusters**
- **`[doc-drift]` (×3 across ht-yha.13, ht-yha.8 and ht-yha.11, with more instances in .10, .16, .34 and .39).** This is a pipeline defect: each task's file list excluded the shared docs and model-visible text, and no task owned a final text sweep. ht-yha.34 and ht-yha.39 cleaned up most of it. What is left is harmless: 3 `wake_on: all` doc comments in `src` (line 159) and `CHANGELOG.md:46` (line 230). Deferred OK.
- **`[missing-edge-case-test]` (×3 across ht-yha.5.1, ht-yha.15 and ht-yha.41).** These are coverage gaps for combined edge cases. Deferred OK, except the ht-yha.41 member, which is tied to a product defect (below).
- **One more pattern, below the cluster threshold but systemic:** missing red-first or base-run evidence for TDD and flake claims in tasks 4, 5, 7, 8, 12, 13, 20 and 21 (lines 24, 73, 88, 127, 134, 136, 192, 202). The full-suite sweep has to settle these; until it runs they are unverified, not clean.

**Misgraded minors (must fix)**
- Line 267 (ht-yha.41, `[magic-number-bound]`): `body --max-bytes N` with N between 256 and about 342 is refused for every p0 message. `fit_annotated_read` (`src/cli/mod.rs:1778`) shrinks by at least 25% and gives up below 256. This is a product defect: fix it, or get the user to accept it explicitly.
- Line 110 (ht-yha.11, `[vacuous-check]`): `markers_ok` (`scripts/wake-e2e.py:208`) passes when no header is on screen. This is in the release-gate e2e.

**Blocked tasks closed by the orchestrator's own decision, not the user's** (both reports said "Decision needed from the user")
- **ht-yha.33, option (c):** the limit is accepted. An old CLI cannot follow a new daemon's format-2 `next:` cursor; it is recorded in the spec's §Compatibility and in `docs/agent-usage.md:173`. Needs the user's ratification.
- **ht-yha.42, option (C):** the test was rewritten. It is low risk and the test passes, but it still needs the user's acknowledgement.

**Deviation that needs a user decision:** line 70. ServiceSend keeps receipts for its whole audience (`AckAudience::Everyone`, `src/store/service_send.rs:353`), against D9. This is not recorded in the spec or in TRUST-POLICY.md.

**Other ledger notes**
- No `parked` lines.
- The only BLOCKED fix pass is Task 8 (line 59), and it was about missing report evidence, not code.
- No BLOCKED-AUTH lines.
- **Metrics:** "landed≠completed 31 vs 1" means the ledger is incomplete (the resume read seeded too few completions), so the absence of problem lines in it does not mean clean. ht-yha.1, .14 and .5.1 landed with tracker closure unconfirmed, though bd now shows them closed. Bead ht-cbb is still open even though its fix landed in 011c24d2.

## Must fix before landing
1. **A:** stale pending delivery rows from cursor-only displays and process-exited bindings, which empty the first page and slow the wake probes. Fix it, replace the TRUST-POLICY.md:800 limit text, and add tests: a stale sibling plus more than 100 rows, a `body`-read row, and the probe cost.
2. **B:** the mod stall rule must cover pending delivery rows the seat's threshold admits, or the user must accept the limit and it gets recorded in TRUST-POLICY.md.
3. `body --max-bytes` refusing bounds of about 256–342 for p0 messages: fix, or explicit user acceptance.
4. `markers_ok` must fail when no expected header is visible.
5. User decisions: ServiceSend vs D9 (record it in the spec and TRUST-POLICY.md), ratifying ht-yha.33 (c), and acknowledging ht-yha.42 (C).
6. The caller's full-suite sweep at HEAD with `--all-features`, plus `scripts/check-default-features` and `scripts/check-no-leaked-processes --run-id`. The 300 s wall-clock budget has to be checked too, since the last sweep took 329 s. Then close ht-cbb.

## Untested scope
- No full suite at HEAD. The only sweep is from dae11184, before ht-yha.40, .41 and .42.
- The pre-release wake e2e (real agents, the T × priority matrix) has not run, and its marker check can pass vacuously. The optional Linux e2e has not run.
- No tests exist for A, or for the stall path in B with a p0 delivery row and no receipt.
- ht-yha.33 is tested only against a hand copy of the old decoder (line 226).
- Bare `--nudge` wake is asserted through the digest, not through scheduler candidates (lines 71, 146).
- A pair's first send is covered only at p1 (line 145).
- The cross-surface mod fixture never covers the passive/wake split by priority (line 152).
- The capability sweep checks the `ADVERTISED` constant, not the running daemon's reply (line 157).
- The `wake-on` deprecation line and the lossy-priority stderr line are not asserted by any test (line 87).
- Paged `read --recent` with `[p0]` rows is not asserted (line 266).
- The flake claims without base runs (tasks 4, 5, 12 and 13) are unverified.

## Deferred OK
- **Test-shape and evidence minors:** lines 12, 13, 28–31, 46, 47, 79, 136, 147, 158, 160, 161, 283.
- **Code-hygiene minors:** lines 64–66, 74, 95, 115, 116, 135, 196, 197, 201, 208; the redundant `delivery_mode='ordinary'` filter in `mod_stall_oldest` (it becomes moot if B is fixed); and the Rust/JS tie-order mismatch for priority-less rows from old daemons only.
- **Superseded or fixed:**
  - 71: interim wake gap, now covered by tests.
  - 72: `delivery_mode` readers converted.
  - 80, 81: superseded by ht-yha.32 and .38.
  - 86: every `send.priority_v1` daemon has the receipts capability.
  - 89, 123: only matter against old daemons.
  - 114: superseded by ht-yha.31.
  - 121: `LEGACY_PAIR_MARKER` exists in test builds only; remove it with the last pre-upgrade fixture.
  - 122: documented.
  - 96, 124: passed in the dae11184 sweep.
  - 101, 102, 140, 141, 206, 207: fixed by ht-yha.34 and .39.
- **Remaining doc drift:** lines 8, 41, 42, 108, 109, 159, 230, plus C (marker vocabulary in the upgrade window).