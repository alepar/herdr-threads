**Verdict: not ready.** This branch (super-auto/message-priority-delivery @ 8f6668d8, fork 998d7d34, 124 commits, 256 files, +36.9k/−4.1k) is not ready to land: one user-approved fix was never merged, a known test still fails, and a related problem in the inbox ordering blocks every delivery row behind a seat's history. No full-suite run exists at this HEAD.

Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-998d7d34..8f6668d8.diff`

## Step 1: my own view of the branch against the spec

**S1. The per-session pending fix (ht-yha.44) was built but never merged.**
- ht-yha.44 is still open. Its code is on `task-ht-yha.44` at a1288945, which sits on 02ae39a8 and is not an ancestor of HEAD. It has not been reviewed: the bead was marked BLOCKED, then the orchestrator amended it to "implementation stands", and the run ended without merging it.
- On HEAD the stale-row problem from report item A is still live:
  - `advance_cursors` (`src/store/control.rs:2821`), used by body reads, the hook fallback and subagent streams, never runs `MARK_DISPLAYED_SQL`. Rows displayed that way stay `state='pending'` in `lazy_recipients_pending_order` forever.
  - `seat_owes_display_sql!` (`src/store/lazy_delivery/mod.rs`) lets an open binding whose process exited keep rows pending.
  - Each stale row costs a `seek_pending` visit (`src/store/lazy_delivery/inbox.rs`). The row seek gets only about 42 visits per page after the deadline window's share, so a session with more than about 40 stale rows gets empty `inbox` and hook first pages.
  - `FRONTIER_SEEK_SQL` (`src/store/lazy_delivery/wake.rs`, `ORDER BY decision_seq DESC LIMIT 1` with the unread test inside the WHERE) scans every stale row of each admitted priority inside one statement. This runs on every wake check and has no bound.
- `TRUST-POLICY.md:803-806` still lists "a process-exited binding that never returns keeps its unread rows pending" as an accepted limit. The user rejected that on 2026-10-14 ("a session's own stale backlog must never slow or empty another session's pages").

**S2. Delivery rows wait behind the seat's whole deadline-class history (wider than ht-yha.52).**
- `capture` (`src/store/queries/inbox_v2/capture.rs`) starts every deadline walk at ordinal 0 on each fresh page.
- The seat-scope receipt walk (`src/store/effective.rs:317-322`, `prepared_recipients LEFT JOIN send_manifests` and `receipts WHERE seat_id`, with no state filter) and `next_invitation` (`sources.rs:130`) both visit settled and unpublished history.
- The merge loop (`inbox_v2/mod.rs:299`) emits nothing until every class has a head or is done. A deadline window stops after 50 visits, and a second window needs 58 visits left, so each page advances 50 history rows and shows zero items.
- `tests/store/deadline_window.rs:205-235` pins this behaviour: 200 settled receipts take 5 rounds.
- The effect: on a seat with roughly 400 or more settled ACK receipts plus invitations (a coordinator, say), `herdr-threads inbox` shows an empty page with a `next:` line, even when a p0 delivery row is pending. The CLI only follows 8 empty work pages (`src/cli/mod.rs` `INBOX_DISPLAY_PAGE_READ_LIMIT`). Every hook pull that is not `Unchanged` re-walks the whole history, up to 4096 pages.
- This is a regression from base, whose round-robin still emitted lazy rows on page 1. ht-yha.52 is scoped only to unpublished preparations, which is the config5 symptom, not this whole class.
- The thread scope already reads from a pending-only projection (`digest_pending_manifest_receipts`). The seat scope needs the same, or the merge needs a history floor.

**S3. A known test fails on HEAD.** `session_bindings_smoke::config5_resume_within_a_page_and_past_it` failed at e8ecb681 and at 43dc5e66. Its owner, ht-yha.52, is open and has no implementation.

**S4. Tracker and bookkeeping gaps.**
- ht-yha.48 (review of ht-yha.47) is still open, although ht-yha.47 landed at cdcc2a68.
- `landed≠completed` (37 vs 33) is explained: the 4 tasks recorded only as landed (ht-yha.1, .14, .5.1, .47) are closed in the tracker, and the one `merge-failed` was the false ht-yha.1 blocker.
- The epic cannot close while .44, .52 and .48 are open.

**S5. Leftovers from the protocol-8 break (ht-yha.49).** Mixed versions are now refused, so the old-daemon fallbacks are dead code: `SendShape::Legacy` and its stderr notice, the `nudge-threshold` → `wake-on` fallback, the `send.priority_v1` probe, and the client-side pair-policy fallback. The commit calls their removal a "separate follow-up", but no bead tracks it. Spec line 160 ("`PROTOCOL_VERSION` does not change") and the epic's "no PROTOCOL_VERSION bump" constraint still contradict the user's decision at spec line 153.

**S6. Smaller seam observations.**
- The mod freezes each item's `wakes` when the item streams. `set_nudge_threshold` does call `note_new_attention`, but I found no test showing that a queued passive item is re-evaluated after T is raised (D2), for the Channel path.
- `register.js` `batchOrder` sorts the whole queue, which can merge separate arrivals. The spec says separate batches stay in arrival order.

## Step 2: ledger triage

**Recurring clusters**
- `[doc-drift]` ×3 across ht-yha.13, .8 and .11 (plus several single instances).
  - The class: model-visible and doc text sat outside each task's file list, so no task could fix it. That is a planning defect in the pipeline.
  - I checked the instances at HEAD and they are resolved: mod README:123, the linux-e2e case names, AGENTS.md wake-e2e, the `wake_on: all` comments, SKILL.md and help.rs pair text, and the `inbox.rs:388` comment. CHANGELOG:46 is historical.
  - Nothing left to do.
- `[missing-edge-case-test]` ×3 across ht-yha.5.1, .15 and .41.
  - The class: tests cover the happy path but not the combinations.
  - Deferred OK, except that the mod passive/wake split by priority has no cross-surface fixture. It is the mod's only threshold-dependent behaviour, so add a test for it when S6 is checked.

**Parked items and BLOCKED lines**
- ht-yha.33 and ht-yha.42: the user resolved both (protocol 8; tests rewritten).
- ht-yha.44: this is S1. The fix is implemented but unmerged and unreviewed. Must fix.
- No `BLOCKED-AUTH` lines.

**Minors I checked as resolved at HEAD**
- `service-send-deviation`: fixed by ht-yha.47.
- `magic-number-bound` (256–342): fixed by ht-yha.50.
- `vacuous-check` (markers_ok): fixed by ht-yha.51.
- The 7 failing upgrade tests at base and `pair_accept…`: fixed by ht-yha.34; they were absent from the e8ecb681 sweep.
- `interim-wake-gap`: bare `--nudge` wakes are covered by `priority_config_smoke` (p ∈ {0,1,5} × T ∈ {−1,0,3}).
- `state-dependent-wake-seek`: partly mitigated by ht-yha.32/.38; ht-yha.44 finishes it.

**Misgraded minors (they name possibly-failing tests)**
- ht-3wm (q5 `ReadBudgetExhausted`) is in the read-budget path that ht-yha.41 and .50 changed, and it was never run at base.
- The same goes for `mod_delivery` stall-timing and `startup_enrollment_hermes…`.
- Each needs a base comparison before these can be called flakes.

## Must fix before landing
1. Merge ht-yha.44: rebase a1288945 onto HEAD (it predates .45, .46, .49, .50 and .51 and touches the migration and schema audit), run its review, and replace the `TRUST-POLICY.md:803-806` accepted-limit text with the per-session rule.
2. Fix the cause of S2 and S3, not only the unpublished case: seat-scope invitation and receipt walks should seek pending, published items only, so settled or staged history costs no visits. Either widen ht-yha.52 or add a sibling bead. The fix should include a test where more than 200 settled receipts do not hide a pending p0 delivery row on the first page, and config5 should pass unmodified.
3. Run the deferred full-suite sweep with leak checks on the final HEAD. Compare ht-3wm, ht-kk8, the `mod_delivery` stall test and the hermes enrollment test against base before classing any of them as flakes.
4. Tracker: close ht-yha.48, and close ht-yha.44 and .52 when they land.

## Untested scope
- No full-suite run exists for 8f6668d8. The last one, at e8ecb681, had 3 failures, and ht-yha.43 through .51 merged after it. Treat the branch as untested.
- The pre-release wake e2e (real Claude and Codex agents) has not run against this candidate. It is required before tagging 1.0.0.
- No test re-evaluates a queued mod item's `wakes` after T is raised (D2 on the Channel path).
- ht-yha.44's code has had no task review and has never been tested on the current HEAD.
- Search at 256–342 bytes for a p0 message is not tested against the real daemon.

## Deferred OK
- Removing the dead legacy fallbacks after protocol 8, and fixing the spec line 160 wording (file a cleanup bead).
- The `LEGACY_PAIR_MARKER` test-only branch (absent from production builds).
- `operator_orphan_invite` stores NULL confirmation (documented).
- Human-only explicit recipients are recorded `delivery_mode='ordinary'` (audit only, as the spec says).
- The per-item `effective_receipt` lookup in `hydrate.rs`.
- `register.js` sorting the whole queue.
- Missing assertion on the deprecation stderr line.
- Hand-copied test codecs and fragile byte lookups.
- Missing TDD red-phase evidence.
- The real-clock test in `priority_sweep`.
- Duplicate mixed-order coverage.
- Partial pair-matrix coverage.
- The `init_for_new_binding` mark tiebreak (superseded by ht-yha.44).