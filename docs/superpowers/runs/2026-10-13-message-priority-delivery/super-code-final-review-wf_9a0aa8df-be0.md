**Verdict: not ready.** This is super-auto/message-priority-delivery @ ea502589 (fork 998d7d34, 132 commits, 261 files, +37.3k/−4.7k). The three blockers from the previous final review (wf_a121fe45-a39) are fixed: ht-yha.44 has merged, ht-yha.52 has merged, and config5 now passes. Three things still block it:
- Two readers that landed before ht-yha.44 still read the seat-wide `state` column, which contradicts the per-session rule.
- The new per-binding view table grows without bound for sessions that never end. TRUST-POLICY says this costs nothing outside that session, and that is wrong.
- main has moved 43 commits since the fork, and a trial merge conflicts.

There is still no full-suite run of this branch.

Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-998d7d34..ea502589.diff`

## Step 1: my own view of the branch against the spec

**What I ran on ea502589 (all passed):**
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`
- `scripts/check-default-features`
- `scripts/check-harness-neutral`
- `cargo fmt --check`
- `session_bindings_smoke::config5_resume_within_a_page_and_past_it` and `mod_delivery::stall_with_only_a_p0_delivery_row_hands_over_to_the_native_wake`
- 65 focused tests in combined and integration, filtered on binding_lazy, deadline_window, lazy_schema, lazy_attention, wake_on_wake, wake:: and priority
- `check-no-leaked-processes`: clean after each run

**S1. Two readers still use `lazy_recipients.state`, which ht-yha.44 says nothing reads.**
- ht-yha.44 says no delivery seek reads `state` (`TRUST-POLICY.md:~805-808`). Two readers that landed earlier still do:
  - `src/store/wake.rs:599`, `LAZY_FRONTIER_UNREAD_SQL` (from ht-yha.45), has `r.state='pending'`.
  - `src/store/seats.rs:5212`, `STALL_ROWS_HEAD` (from ht-yha.46), has `r.state='pending'`.
- Under the new rule, a binding disconnected by an end event or as superseded "owes nothing", so a sibling's display sets `state='displayed'`. The binding keeps its own view and is "offered its rows again when it reconnects".
- Failure case: a Claude session exits and is later resumed with `--resume` (its binding is disconnected by the end event, then reconnected). Before the resume, a sibling displays a p0 row.
  - `wake_frontier` reads the resumed binding's view and reserves a wake.
  - `wake_owed_to_binding` then finds `state<>'pending'` and refuses it.
  - So a sibling's display decides whether this session is woken, which goes against the user's 2026-10-14 decision. The mod stall rule also ignores that row.
- The fix is small: drop the `state` test and let `binding_lazy_unread_sql!` decide, or read `binding_pending`.

**S2. `binding_pending` grows without bound, and TRUST-POLICY understates the cost.**
- The `binding_pending_publish` trigger (`migrations/0047_message_priority.sql:98`) inserts one row for every open binding of the recipient seat on every publish, including disconnected ones.
- Bindings are ended only by exclusivity or retirement. Subagent stops, end events and superseded sessions only disconnect. So those view rows are written on every publish and never deleted.
- The live store (v46, read-only query) has 39 open bindings, of which 8 are connected and 18 are subagents. One seat has 22 open bindings with 1 connected, so each delivery to that seat would write 22 rows plus two index entries each.
- `TRUST-POLICY.md:808` says "a process-exited binding that never returns keeps its own view until it ends; it costs only itself". In practice it costs every sender's publish transaction plus unbounded storage.
- Fix: keep views only for connected bindings (and process-exit ones, if wanted), delete a view on disconnect, and reseed it with `seed_binding_pending` on reconnect (`src/store/bindings.rs:556`). This gives the same view without the growth.
- Alternative: get the user to accept the growth explicitly and correct the policy text.

**S3. main has moved on.**
- main is 43 commits past 998d7d34 (test-infrastructure speed work, `store/verified.rs`, handoff automatic validation caching, the schema-template seeding in `connection.rs`).
- `git merge-tree` reports conflicts in `src/cli/direct_send.rs`, `tests/cli/wake_on.rs` and `tests/store/wake_on_wake.rs`, plus about 50 overlapping files that auto-merge.
- main adds no migration, so v47 does not collide. Even so, the branch has never been built or tested against current main.

**S4. Smaller observations.**
- The repair path for a pre-amendment v47 store (`src/store/schema.rs:~855-878`) does not run `verify_message_priority_schema` after applying the tail.
- `pending_summary` and `seat_lazy_pending` (`src/store/lazy_delivery/inbox.rs:408-436`) have no callers.
- Several hot-path walks still scan the seat's full delivery-row history with a correlated predicate instead of reading `binding_pending`:
  - `binding_lazy_unread` (mod fingerprint)
  - `binding_lazy_window`
  - `binding_has_unread_lazy`
  - `attention::pending_lazy`

  This cost class is not new: at the base, lazy was the default send, so these rows already covered most traffic.
- The v1 `InboxBatch` path can still be reached with a non-`ib2:` cursor (`src/cli/mod.rs:2089`) and walks `ReceiptScanScope::Seat` history.
- `register.js:595` still matches `kind === 'lazy'`.

## Step 2: ledger triage

**Recurring cluster: `[doc-drift]` ×8 across 6 tasks (ht-yha.13, .8, .11, .10, .16, .53).**
- The class is a planning defect: doc and model-visible text sat outside every task's file list, so no task could fix it.
- The live instance is the spec. Row D10 (line 29: "capability-gated in both directions") and lines 188, 195, 227, 241, 249, 253 and 275 still describe old-daemon fallbacks that line 148 (protocol 8, mixed versions refused) and ht-yha.53 removed.
- The spec's Post-Implementation Notes section is empty. Neither protocol 8 nor the per-binding views (ht-yha.44) is recorded there.
- This is a cheap docs-only fix. The other instances were resolved in the earlier review.

**Recurring cluster: `[missing-edge-case-test]` ×3 (ht-yha.5.1, .15, .41).** Deferred OK, except that the mod's re-evaluation of `wakes` after T is raised (D2 on the Channel path) is still untested.

**Parked and blocked items, all resolved:**
- ht-yha.33 and ht-yha.42: settled by the user's decisions (protocol 8; tests rewritten).
- ht-yha.44 BLOCKED on config5: fixed by ht-yha.52. config5 passes unmodified at ea502589; I ran it.
- Task 8 (ht-yha.5.3) fix-pass BLOCKED: report paperwork only.
- No `BLOCKED-AUTH` lines.

**`landed≠completed` (40 vs 36):** these are the same four tasks recorded only as landed (ht-yha.1, .14, .5.1, .47). All are closed in the tracker, and all 62 ht-yha beads are closed. The gap is explained.

**Misgraded minors (they name failing or possibly flaky tests), must be checked in the sweep:**
- doctor_reports (ht-yha.44)
- 5 hermes hook tests "under load" (ht-yha.53)
- `startup_enrollment_hermes_steady_turns_make_no_resolver_call` (ht-yha.50)
- the mod_delivery stall test (ht-yha.46; passed in my isolated run)
- ht-3wm and ht-kk8 from earlier

Each needs a run at the base before anyone calls it a flake.

**ht-yha.45 `[empty-test-filter]`:** its integration check matched 0 tests, and S1 is a seam in exactly that code. The sweep has to cover `lazy_attention` and `tests/store/wake.rs`.

**`[unrun-default-features-check]` (ht-yha.44, .52):** resolved. I ran it and it passed.

**Other minors in the ledger:** deferred OK; they are listed at the end.

## Must fix before landing
1. Remove the `r.state='pending'` test from `LAZY_FRONTIER_UNREAD_SQL` (`src/store/wake.rs:599`) and `STALL_ROWS_HEAD` (`src/store/seats.rs:5212`). Add a test: a binding resumed after an end event wakes on a p0 row in its view that a sibling displayed while it was away.
2. Stop the `binding_pending` growth for disconnected bindings that never end: drop the view on disconnect and reseed on reconnect. Alternatively, get explicit user acceptance and correct `TRUST-POLICY.md:808`, whose "costs only itself" claim is false.
3. Merge current main (43 commits, 3 conflicting files), then run the full suite with `HT_LEAK_RUN_ID` and `check-no-leaked-processes` on the merged HEAD. Compare every failing or named-flaky test against the base before classing it as a flake.
4. Docs only: amend spec row D10 and lines 188, 195, 227, 241, 249, 253 and 275 for protocol 8, and add Post-Implementation Notes for protocol 8 and the per-binding views.

## Untested scope
- **No full-suite run exists for ea502589.** The last sweep was at e8ecb681, with 3 failures. ht-yha.43 through .53 merged after it, including ht-yha.44's rewrite of the delivery store core (new table, 7 triggers, an amended v47, dropped indexes). Treat the branch as untested. The caller's sweep is the first measurement.
- The branch has not been tested merged with current main.
- The pre-release wake e2e (real Claude and Codex agents) has not run against this 1.0.0 candidate. It is required before tagging.
- No migration test runs v46 to v47 on a realistic store with many open, disconnected bindings. The live store, with 39 open bindings, would be a good copy to test on.
- The pre-amendment v47 repair path has no schema verification after repair.
- D2 on the Channel path is untested: no test re-evaluates a queued mod item's `wakes` after T is raised.
- Search at 256–342 bytes on a p0 message has not been tested against the real daemon.
- ht-yha.45's integration coverage was never shown (the empty test filter).

## Deferred OK
- History scans in the hot-path walks (`binding_lazy_unread`, `binding_lazy_window`, `binding_has_unread_lazy`, `pending_lazy`). The cost class existed at the base; moving them onto `binding_pending` is a follow-up.
- Dead `pending_summary` and `seat_lazy_pending`.
- The v1 `InboxBatch` path reachable through a non-`ib2:` cursor, and the `ReceiptScanScope::Seat` callers (`queries.rs:5526`, `5886`): ht-yha.52's residual-history-walk.
- The `kind === 'lazy'` branch in `register.js`, and `register.js` sorting the whole queue.
- `tests/support/lazy_proxy.rs:99` still strips `send.priority_v1`.
- The cursor triggers cost O(view size) per advance.
- The hand-simulated stale `binding_pending` rows in the work-limited test (`tests/store/lazy_inbox.rs:1703`).
- File-list deviations (ht-yha.44, .47, .53).
- Missing TDD red-phase evidence, and stale re-runs after rebase.
- `annotate`'s doc comment now sits on `annotation_len` (`src/cli/output.rs` ~423).
- Human-only explicit recipients recorded `delivery_mode='ordinary'` (audit only, as the spec says).
- The `operator_orphan_invite` NULL confirmation, and the `LEGACY_PAIR_MARKER` test-only branch.
- Duplicate mixed-order coverage, partial pair-matrix coverage, and the real clock in `priority_sweep`.