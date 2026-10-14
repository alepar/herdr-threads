**Verdict: not ready.** No full-suite run exists for HEAD 970ed3f5 (the deferred sweep is the caller's). The last sweep, at dae11184, had 3 failures. Two have since been fixed (ht-yha.40, ht-yha.41) and pass at HEAD. The third, `wire_compat::cli_against_an_old_fixture_daemon_never_sends_full_bodies`, still fails at HEAD: I re-ran it (tests/integration/wire_compat.rs:448).

Checks I ran at HEAD: `scripts/check-harness-neutral` passed (0 violations). `cargo clippy --locked --all-targets --all-features -D warnings` finished with no warnings. The three sweep-failing tests: 2 pass, wire_compat fails.

## Step 1: my own review against the spec

**F1, Must-fix. Rows nobody will read stay in the delivery-row index forever, and once there are about 50–100 of them, a reader's first page comes back empty.** This is still open from the previous final review (wf_ecf6a4ce, finding A). No commit after 30721043 touches it.
- **What changed.** On main, a top-level completion marked a row displayed seat-wide right away. On this branch, `MARK_DISPLAYED_SQL` (src/store/lazy_delivery/completion.rs:70) only does so when no open top-level binding still owes it. A binding still owes it if it is connected, or disconnected because its process exited (`seat_owes_display_sql!`, src/store/lazy_delivery/mod.rs:129).
- **Where stale rows come from:**
  - A binding whose process exited without an end event never returns, so every later row for that seat stays `state='pending'` permanently. It takes one crash or kill in a pane.
  - The cursor-only display paths (`AdvanceCursors`: `body` reads, the cursor-only advances from subagent streams) never run `MARK_DISPLAYED_SQL`. Each `body` read of a truncated row leaves that row pending.
- **Why it bites.** `seek_pending` (src/store/lazy_delivery/inbox.rs:78) applies the binding filter after the seek, and every rejected row costs one of the page's `CANDIDATE_LIMIT` = 100 visits. The deadline window runs first, and 2 visits are held back for warnings.
- **What breaks.** Once more stale rows than that sit ahead of new ones in (priority, decision_seq) order, a fresh page uses up its work and returns no items with a continuation. Default p1 rows are hit first.
  - `hook_inbox::read_page` (src/cli/hook_inbox.rs:62) reads one pushed first page and shows nothing when it is empty, which breaks delivery contract rule 3 for that seat.
  - A p0 wake can arrive with nothing delivered in the turn.
  - Mod/pull drains still get there, but walk every stale row on every drain.
  - `has_wake_rows` and the wake seeks scan the stale rows on every probe.
- **Repro.** A process-exited sibling binding plus a live one; send 120 p1 rows; the live binding displays them through the text inbox; send one more. The next hook page is empty.
- **Accepted limit is not a sign-off.** TRUST-POLICY.md says "the ordered seeks skip them within their visit bound" (around line 800), which understates this. A fix pass (ht-yha.38) wrote that limit, not the user.
- **Fix.** Either take a row out of the pending index on the cursor-only paths and once no owing binding can return, or give the seek a per-binding lower bound. Then rewrite the TRUST-POLICY text and add the three tests named in wf_ecf6a4ce §Must fix A.

**F2, Must-fix. A known failing test.** ht-yha.42 is BLOCKED, and the bead's amendment (option C: rewrite the test's identical-bytes assertion) was never applied. Its root cause: every fixture message there is `--nudge --require-ack`, so p0, so the current daemon prints `[p0] <id>` rows, as spec §Surfaces and D8 require.

**F3, Should-fix. The release-gate e2e has a check that can pass vacuously.** `markers_ok` (scripts/wake-e2e.py:208) returns true when no expected header is on screen.

**F4, Deferred OK.** A redundant `delivery_mode='ordinary'` filter remains in `mod_stall_oldest` (src/store/seats.rs:5225). It is a tautology for receipts, but it is one more decision read of the column the spec says is never read for decisions.

**F5, Deferred OK. A small Rust/JS mismatch in a legacy-only case.** For a priority-less promoted row from an old daemon, the Rust batch key gives p1 (`DeliveryKind::Lazy`), while JS `orderKey` gives p0 (its watch kind is `message`). It only affects ordering within one batch against an old daemon.

**Composed seams I checked, all OK:**
- Send staging (D9; explicit-ACK humans get a p0 row) → `note_published` decision_seq copy and threshold-gated `wake_work` → `wake_frontier` (one seek per admitted priority, connected-only) → the reserve re-check.
- `for_seat_attention` now includes `lazy`; test at tests/scheduler/dispatch.rs:5321.
- Migration 0047: trigger drop/recreate order, backfills, the priority-keyed `lazy_manifest_source`.
- Pair auto-join through `record_join`.
- Wire fields are additive, with no `deny_unknown_fields` on the item types.
- The drain sorts per page with stable ties.
- The mod's passive test follows `wakes`/`ackRequired`.

## Step 2: ledger triage

**Recurring-minor clusters:**
- **[doc-drift]**, ×3 across ht-yha.13/.8/.11 (more instances in .10, .16, .34, .39). The class is a pipeline defect: task file lists forbade editing model-visible or doc text outside the list, and no task owned a final text sweep. ht-yha.34 and ht-yha.39 have since swept most of it; the SKILL.md:17/:199 and help.rs:151 pair-invitation wording is gone. Remaining: CHANGELOG.md:46, which is a historical note. Deferred OK.
- **[missing-edge-case-test]**, ×3 across ht-yha.5.1/.15/.41. The class is combination-edge coverage gaps. Deferred OK, except the ht-yha.41 member, which comes with a product defect (next item).

**Must fix before landing:**
- ht-yha.41's "magic-number-bound" minor was misgraded; it names a product defect. `body --max-bytes N` for N in about 256–342 is refused for p0 messages: `fit_annotated_read` (src/cli/mod.rs:1778) shrinks by at least 25% and gives up below 256. The same defect existed on main for `[lazy]` rows, but it now hits every ACK and `--nudge` message. Fix it, or have the user explicitly accept it.
- ht-yha.15 "vacuous-check" (`markers_ok`) is the same as F3.
- Parked, needs the user's decision: ServiceSend keeps receipts for its whole audience (`AckAudience::Everyone`, src/store/service_send.rs:353), outside D9. Record the decision in TRUST-POLICY.md and the spec.
- Parked, needs the user's ratification: ht-yha.33 shipped option (c). An old CLI cannot follow a new daemon's format-2 `next:` cursor; this is recorded in §Compatibility and docs/agent-usage.md:173. The user has not approved it.
- ht-yha.42 (F2).

**BLOCKED-AUTH:** none recorded.

**Metrics:** "landed≠completed 30 vs 27": ht-yha.1, ht-yha.14 and ht-yha.5.1 landed with tracker closure unconfirmed. They are closed in bd now. Bead ht-cbb is still open even though its fix landed in 011c24d2 and its tests are no longer ignored; close it.

## Must fix before landing
1. F1: stale pending delivery rows (from process-exited bindings and cursor-only/`body` displays) push new rows off the first page, including hook check-ins. Fix it, rewrite the TRUST-POLICY.md A8 limit, and add the tests.
2. F2 / ht-yha.42: make `wire_compat::cli_against_an_old_fixture_daemon_never_sends_full_bodies` pass (amendment option C).
3. `markers_ok` in scripts/wake-e2e.py must fail when no expected header is visible.
4. `body --max-bytes` 256–342 refusal for p0 messages: fix, or get explicit user acceptance.
5. User decisions: ServiceSend vs D9, and ratifying ht-yha.33 option (c). Record both in TRUST-POLICY.md and the spec.
6. The caller's full-suite sweep at HEAD (with `--all-features`), plus `scripts/check-default-features` and `scripts/check-no-leaked-processes --run-id`. This also settles the flake claims made without base runs (tasks 4, 5, 12, 13).

## Untested scope
- No full suite at HEAD 970ed3f5; the only sweep was at dae11184, before ht-yha.40 and ht-yha.41.
- The pre-release wake e2e (real agents, run by a person) has not run against the T × priority matrix, and its marker check can pass vacuously. The optional Linux e2e has not run either.
- F1 has no tests: a stale binding plus more than 100 rows, `body`-read rows left in the pending index, and the cost of the wake and discovery probes.
- ht-yha.33 is tested only against a hand copy of the old decoder.
- Bare `--nudge` wake is asserted through the digest, not through scheduler candidates (Tasks 7/16).
- Pair first send is covered only at p1 (Task 16).
- The cross-surface mod fixture does not cover the passive/wake split by priority (Task 18).
- The capability sweep checks the `ADVERTISED` constant, not the running daemon's reply (Task 19).
- The `wake-on` deprecation line and the lossy-priority stderr line are not asserted by any test (Task 5).
- Paged `read --recent` with `[p0]` rows is not asserted (Task 30).

## Deferred OK
- Test-shape and evidence minors: Task 2 (lines 12/13/46/47); Task 6 (28–31); Task 3 (8/41/42); 24, 79, 87, 88, 136, 145–147, 152, 158, 160, 161, 192, 202, 226.
- Code-hygiene minors: 64–66, 74, 95, 115, 116, 135, 196, 197, 201, 208; and F4 (the `mod_stall_oldest` filter).
- The rest:
  - 71 (interim wake gap): covered by tests/store/nudge_threshold.rs, priority_sweep and priority_config_smoke.
  - 72 (delivery_mode readers): converted, except F4.
  - 80/81 (state-dependent wake seek): superseded by ht-yha.32/.38.
  - 86 (skipped receipts-capability probe): every `send.priority_v1` daemon has it.
  - 89/123 (human handoff Lazy policy): only matters against old daemons.
  - 114 (drain error path): superseded by ht-yha.31.
  - 121 (`LEGACY_PAIR_MARKER`, test builds only): remove it with the last pre-upgrade fixture.
  - 122 (operator invite stores NULL confirmation): documented.
  - 124, 96, 101, 102, 140, 141, 206, 207: fixed by ht-yha.34/.39.
  - 159, 230 (doc drift).
  - F5 (Rust/JS legacy tie-order mismatch).

Review package: /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-998d7d34..970ed3f5.diff