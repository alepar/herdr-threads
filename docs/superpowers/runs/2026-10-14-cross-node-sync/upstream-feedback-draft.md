# 2026-10-14-cross-node-sync: unconfirmed tracker closure is never re-checked, so every later invocation ends `closure-unconfirmed`

Plugin: 6.4.2-alepar5.1. Run: 261 beads (148 landed across 8 super-code coordinator invocations), 2 design-roast and 2 PR-roast rounds, one sweep-fix pass. 2026-10-14 to 2026-10-16.

## Defects

### 1. A landing whose tracker closure could not be confirmed stays "closure unconfirmed" in every later invocation, even after the bead is closed
- **Evidence:** progress.md repeats `Task 96 (ht-5fr.15): landed (commits 4729922…..8a4ad78…, tracker closure unconfirmed)` 18 times (lines 783, 785, 791, 812, 895 … 1014). The tracker shows ht-5fr.15 closed at 2026-10-16T03:44:54Z. Every later invocation still returned stopReason `closure-unconfirmed`. Every Finish wrote `ledger-check landed≠completed: 111 vs 110 … 129 vs 128 … 141 vs 140 — … (an incomplete Resume read seeds too few completions)`, a generic cause; the real cause is this one landing.
- **Premise to verify:** On resume, coordinator.js rebuilds `landed` from the historical `landed (…closure unconfirmed)` lines, and Finish computes `closureUnconfirmed` without re-reading the tracker. Upstream should confirm why the close check (`reconcileBucketsPrompt`) never moves the ID to `completed` once the bead is closed.
- **Suggested fix shape:** On resume and at Finish, re-check each carried unconfirmed landing against the tracker and log `complete (closure confirmed on re-entry)`. Never re-append an identical landed line. Make ledger-check name the IDs behind any gap.

### 2. A triage wait edge on a `review: <id>` bead is never released when that review bead closes, so the invocation ends `stalled`
- **Evidence:**
  - ledger 574: `Edge add: ht-5fr.3.37 <- ht-5fr.25 (triage)`.
  - ledger 576: `Task 100 (ht-5fr.3.37): waiting on ht-5fr.25`. ht-5fr.25 is `review: ht-5fr.24` and was closed at 18:21:46Z.
  - ledger 577–578: ht-5fr.24 merged.
  - run.md code-run-2: `stopReason stalled (ht-5fr.3.37 left un-redispatched after a triage wait edge)`. The task merged only after a relaunch.
- **Premise to verify:** JS readiness tracks the planner's `deps` rows. A triage edge to a review bead created mid-run is never mapped to that review's closure or to the task's merge.
- **Suggested fix shape:** Resolve a `review: X` target to X's merge event, or treat any mid-run wait target as opaque and re-check it at every landing. Re-test waiting tasks against the tracker before declaring `stalled`.

### 3. Readiness changes made mid-run are invisible until the next landing or round head
- **Evidence:**
  - friction 2026-10-14: `ht-5fr.1.2 sole in-flight task ~2h … newly-ready ht-5fr.3.1 etc. idle until a landing fires the bd-ready top-up`. Run 1's Detector line reads `top-up queries 40/40 … waiting on deps 47`.
  - friction 2026-10-15: `ht-5fr.15 (integration sweep) dispatched from planner rows mapped before ht-5fr.4.40 (and its sweep edge) existed`.
  - Mid-run edits did happen in practice: narrows the user approved, fix beads the session filed, and edges from a security review.
- **Premise to verify:** a cheap change-detection read, such as a tracker modification timestamp, can run without spending a top-up agent.
- **Suggested fix shape:** Detect tracker changes in the epic subtree on a timer or at each free dispatch slot, and re-map rows when something changed. Re-check a fan-in or sweep task's blockers just before dispatching it.

### 4. Finish metrics depend on a model reading the ledger, so a model refusal blanks every Metrics line
- **Evidence:** friction 2026-10-16: `read-ledger:finish failed twice (Sonnet 5.5 safeguards flag, reasoning_extraction; req_011Cg5yF5CHAg1aRfiWqVnR4, req_011Cg5yJkR3N2BMbjidDs6rz) → final Metrics UNAVAILABLE`. Two invocations each wrote 4 `Metrics: UNAVAILABLE (…) — the Finish ledger re-read returned null` lines.
- **Premise to verify:** the counts (Merge:, complete, fix pass, cancelled, landed) come from line prefixes, so a script or a JS file read can compute them.
- **Suggested fix shape:** Compute the Finish counts in code, and use a model only for narrative. Failing that, retry a null or refused read on another model tier before reporting UNAVAILABLE.

### 5. Recurring-minor clustering keys on free-form reviewer tags, so recurring classes are undercounted
- **Evidence:** the ledger has 403 `minor (deferred)` lines with about 339 distinct tags. The "no RED run" class appears under at least 14 tags across about 26 lines (`no-red-run` 8, `missing-red-run` 3, `no-red-evidence` 2, `missing-red-evidence` 2, `tests-after-implementation` 2, `tdd-skipped`, `skipped-red-step`, …). The detector reported only `[no-red-run] ×8` and `[missing-red-run] ×3`. The temp-path class is split the same way (`test-temp-path` 3, `temp-path-helper` 2, `scratch-temp-path` 2, …). Clustering is specified in coordinator-workflow.md:1170.
- **Premise to verify:** a small seeded vocabulary covers most minors.
- **Suggested fix shape:** Give the task reviewer a canonical class list, open to new classes only when none fits, plus the run's tags so far. Alternatively, merge synonymous tags in one pass before applying the cluster threshold.

### 6. The scope filter's "goal-named" test punch-lists defects the change itself introduced in project gates and format consumers
- **Evidence:**
  - scope-filter round 1 sent `Cargo.toml:64` to the punch list with the reason "Release license-packaging config is not a goal-named behavior". The new dependency's BSD-3-Clause licence is not in about.toml, so the release workflow's `cargo about` step fails.
  - Two scripts still on the old layout (`scripts/validate-native-demo.py:3807`, and `src/daemon/paths.rs:190` for the Hermes probe) were punch-listed by clusterOverride.
  - Roast round 2 newly confirmed `scripts/wake-e2e.py:874`: the 8-char ID regexes no longer match the change's 11-char tagged IDs. It went to the punch list as well.
  - The final review then reported these same items as blocking landing.
- **Premise to verify:** "this change breaks an existing gate or consumer" can be told apart from "an unrelated improvement".
- **Suggested fix shape:** Add a rule to scope-filter-prompt.md: a finding that this change breaks an existing build, release, CI or validation gate, or an in-repo consumer of an interface or format the change altered, is in scope even when the goal does not name that artifact.

## Run metrics
### Judge panel
- design, iteration 1, same-family (Claude), seat-differentiated panel, rung Workflow: `panels 39 · rr 0.74 · rg 0.62 · fg 0.56 · unanimous 0.46 · ground-loo 0.62 (n=29)`
- design, iteration 2, same-family (Claude): `panels 12 · rr 0.75 · rg 0.83 · fg 0.58 · unanimous 0.58 · ground-loo 0.78 (n=9)`
- PR, iteration 1, same-family (Claude): `panels 39 · rr 0.74 · rg 0.77 · fg 0.72 · unanimous 0.62 · ground-loo 0.83 (n=29)`
- PR, iteration 2, same-family (Claude): `panels 3 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=3)`
### Fix loop
- `Metrics: completions — review clean 134 · after fix pass 6 · parked 2 · re-entry closes 0 · dispatched early 55 · cancelled 5` (last invocation that had counts)
- `Metrics: fix-pass — entered 10 · FIXED 6 · BLOCKED 4`
### Merge-back
- `Metrics: merges 141 · merge-failed 0 · rebase-conflicts 16 · seam-reviews 46 (fixed 3) · check-fails 2 (fixed 2)`
- `Metrics: ledger-check landed≠completed: 141 vs 140` (see defect 1)
- Merge: lines: 16 with a rebase conflict, 46 where a seam review ran, 2 `check fail→fixed`, 0 `check fail`, 0 `→ blocker`.
- Ledger incomplete: 143 first-parent merge commits on main..branch against 141 success-path Merge: lines. Some of the extra merge commits are session-made base merges.
### Coverage
- coverage round 1: `requirements: 16 · mapped: 16 · unmapped: 0`; round 2: `requirements: 20 · mapped: 20 · unmapped: 0`.
- scope filter, PR round 1: `4 in-scope · 11 punch-listed` (cluster `join-error` dropped).
### Timing
none (the ledger has no `Profile:` lines)
### Bead graph
| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-5fr.65 | task | Sweep fix: token_diet search budget too small for node-tagged IDs | Covers: token_diet::agent_facing_machine_text_is_compact Test-defect or sweep-fix bead: it may change a test the bead names as the defect, and otherwise fixes t |
| ht-5fr.64 | task | Sweep fix: peer --stdio exits 1 on stdin EOF | Covers: peer_stdio::stdin_eof_logs_link_down Test-defect or sweep-fix bead: it may change a test the bead names as the defect, and otherwise fixes the code the  |
| ht-5fr.63 | task | Sweep fix: agy canonical_v2 page tests see no pending invitations | Covers: harness::agy::delivery::tests::daemon::canonical_v2_system_allocated_pages_fit_actual_native_codec, harness::agy::delivery::tests::daemon::canonical_v2_ |
| ht-5fr.62 | task | Sweep fix: 'journaled entity has no creating fact' StoreCorrupt in handoff and summary_flo | Covers: cli::topology_handoff::tests::live::composition_actual_new_thread_child_requires_canonical_created_thread, cli::topology_handoff::tests::live::downstrea |
| ht-5fr.61 | task | Fix (roast r1): credit grants are never dropped on a full control queue | Covers: [Should-fix] src/sync/coordinator/ingest.rs:203 ingest.rs:201-205 discards the result of entry.handle.grant(len); grant -> control.try_send into the 256 |
| ht-5fr.60 | task | Fix (roast r1): node-scoped context resolution for peer --stdio, sync/peer CLI and install | Covers: [Blocking] src/main.rs:117, [Should-fix] src/cli/mod.rs:616, [Should-fix] scripts/install.sh:383 Cluster bead: it may change every instance of its rule, |
| ht-5fr.57 | task | Fix: skill doc peers wording; restore pre-accept text-body flush coverage | Final review (run 4): - integrations/skill/SKILL.md:28 says peers are 'on this machine' while line 144 describes linked machines; make line 28 consistent (model |
| ht-5fr.56 | task | Fix: silent local-only fallbacks for journaled threads become errors | Final review (run 4) should-fix 4. |
| ht-5fr.55 | task | Fix: loop_inventory allowlist for ht-5fr.42's paused-time sleeps | Final review (run 4) must-fix 3. |
| ht-5fr.54 | task | Fix: overdue hold counts enabled dial targets that are not linked yet | Final review (run 4) must-fix 2. |
| ht-5fr.53 | task | Fix: overdue hold waits for ingested settlements to apply | Final review (run 4) must-fix 1. |
| ht-5fr.51 | task | Test: mod-channel accept-grace on an ordinary-thread invitation | ht-5fr.31 deleted 14 pair-invitation accept tests (their subject no longer exists after auto-join). |
| ht-5fr.49 | task | Test: two-node auto-ACK on public display paths (text inbox, mod/policy delivery) | Coordination with super-auto/inbox-injection-auto-ack (epic ht-794l, peer seat snbemykTY): auto-ACK on display must settle the sender-home receipt across nodes  |
| ht-5fr.46 | task | Apply and startup robustness minors | Deferred review minors: - src/store/apply.rs: self_tag_k set by a follow-up's handler is dropped (only the trigger's tag reaches ApplyProgress); follow-ups of f |
| ht-5fr.45 | task | Sync coordinator robustness minors | Deferred review minors in src/sync/coordinator.rs and neighbours: - shutdown(): handshakes.abort_all() leaves link_count incremented for in-flight handshakes (5 |
| ht-5fr.44 | task | Test: remote-seat filters in wake, liveness and reconcile must be load-bearing | ht-5fr.4.14 review: the wake, liveness and reconcile remote-seat tests pass with or without the new filters. |
| ht-5fr.43 | task | Security: message.sent apply checks author membership and recipients | Follow-up from the background security review of src/store/messages.rs (df1a1a0a7). |
| ht-5fr.42 | task | Security: unsigned Have claims must not drive evidence, restore or hold | Follow-up from the background security review of ht-5fr.6.13 (src/sync/incarnations/worker.rs). |
| ht-5fr.41 | task | Security and robustness: sync serve resource bounds | Follow-up from the background security review of ht-5fr.5.13 (src/sync/coordinator.rs, src/sync/coordinator/serve.rs). |
| ht-5fr.40 | task | Security: envelope payload parser differential | Follow-up from the background security review of ht-5fr.3.2 (src/sync/envelope.rs, commit 4a41d3e5). |
| ht-5fr.39 | task | Docs and cleanup: stale instances/ paths; stale allow(dead_code) | - docs/install.md:125,516, docs/operations.md:41,104,191 and scripts/wake-e2e.py:1041 still name the pre-sync instances/ state layout; update to the node/ layou |
| ht-5fr.38 | task | Fix: two branch test failures (capabilities hello decode; loop_inventory timed waits) | Both fail on the integration branch before and after the main merge (verified at 56e8b1e42). |
| ht-5fr.37 | task | Reconcile: multinode tests for priority, pair and invite; fact-kinds test expectations (it | Spec: docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-15-cross-node-sync--main-v1-reconcile-design.md §Tests, items 22 and 23. |
| ht-5fr.36 | task | Reconcile: journal oracle priority fields and store fact tests (items 20, 21) | Spec: docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-15-cross-node-sync--main-v1-reconcile-design.md §Tests, items 20 and 21. |
| ht-5fr.35 | task | Reconcile: drop unreachable pre-v47 compatibility code and its tests (item 19) | Spec: docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-15-cross-node-sync--main-v1-reconcile-design.md item 19 and C7. |
| ht-5fr.34 | task | Reconcile: nudge-threshold / wake-on refuse remote seats (item 18) | Spec: docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-15-cross-node-sync--main-v1-reconcile-design.md item 18. |
| ht-5fr.33 | task | Reconcile: handoff automatic pair route without invitation (item 17) | Spec: docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-15-cross-node-sync--main-v1-reconcile-design.md item 17. |
| ht-5fr.32 | task | Reconcile: auto-joined pair read floor and pending peer in the audience (items 14, 16) | Spec: docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-15-cross-node-sync--main-v1-reconcile-design.md §2.3, §2.4, items 14 and 16. |
| ht-5fr.31 | task | Reconcile: cross-node 1:1 auto-join authoring (items 11, 12, 13, 15) | Spec: docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-15-cross-node-sync--main-v1-reconcile-design.md §2 (R2) and implementation items 11, 12, 13, 15. |
| ht-5fr.4.40 | task | Fix: bind thread.register actor to the signing node | Security (background commit review of 581c83671, ht-5fr.4.38): thread.register applies under AuthorRule::Any (by design: any member's node may change topic/name |
| ht-5fr.4.38 | task | Fix: restore the actor on topic, name and archive events | Regression from main: main's set_topic wrote actor_seat into the event; ThreadRegisterPayload has no actor field and materialize_register (src/store/control/thr |
| ht-5fr.4.37 | task | Fix: a store-only daemon must restore its own seats after wipe or resync | materialize_seat_created (src/store/seats.rs ~283) needs an attached host; after a wipe herdr_hosts is empty, and peer --stdio starts the daemon store-only, so  |
| ht-5fr.5.29 | task | Fix: links promoted by traffic never ask for the peer vector (dial-each-other stall) | When two nodes dial each other at start (discovery is on by default, so this is the normal topology), one side's active link can want nothing (rx_logs=0 unsolic |
| ht-5fr.24 | task | Fix: put the sync timed waits on a Pacer (loop_inventory test) | service::loop_inventory::every_timed_wait_is_on_a_pacer_or_allowlisted_with_a_reason (tests/service/loop_inventory.rs:306) fails on the run branch: its static s |
| ht-5fr.23 | task | Fix: nextest timeout/serial group for the release failpoints test and two-daemon tests; Lo | (1) package manifest::failpoints_are_compiled_out_of_release_builds takes 274-287 s alone against the 300 s nextest limit and timed out in every full-suite run  |
| ht-5fr.22 | task | Fix: show the discovery "not installed" state and make sync verify re-hash entries | (1) Root spec §11: a dial target whose remote lacks herdr-threads is shown as "candidate: not installed"; DiscoveryStatus is built in app.rs but surfaced nowher |
| ht-5fr.2.25 | task | Fix: start archival and other host-scoped lanes when the first host attaches | The archival lane starts only if a host was attached at startup (src/app.rs ~2287, marked Interim). |
| ht-5fr.5.27 | task | Fix: dialer ChildGuard teardown must always signal the process group | ht-5fr.5.7 [teardown-skips-group-sweep]: ssh or dial_command descendants leak when the group leader has already exited (src/sync/dialer.rs ChildGuard::teardown) |
| ht-5fr.6.31 | task | Fix: salvage must not delete the resync marker when Headroom stopped a log | ht-5fr.6.14 [salvage-headroom-data-loss]: at Headroom, salvage stops a log and still deletes the resync marker, so that tail survives only in the backup — a dat |
| ht-5fr.6.30 | task | Fix: fork branch completion, low branch tips and incarnations queue backoff | (1) branch_progress (src/store/incarnations.rs ~1310) sets complete = last >= tip_counter \|\| !more, so a source that holds less than the tip completes the bra |
| ht-5fr.3.37 | task | Fix: enforce the own-history hold cap without relying on ingest kicks | OwnHistoryHold expires only inside OwnHistoryHold::active(), called only from apply_pass, which the Apply lane runs only when the apply hint is set (src/service |
| ht-5fr.18 | task | Fix: process-table discovery must only read processes owned by this user (Linux) | Security fix (background commit review of 2f1254a0, ht-5fr.7). |
| ht-5fr.3.32 | task | Fix: commit_kicks test fixture collides with the self sync_logs row seeded at journal open | store::commit_kicks_tests::each_table_kicks_exactly_its_lanes (tests/store/commit_kicks.rs ~:110, fixture INSERTs at ~:134/:138) fails on the integration branch |
| ht-5fr.3.21 | task | Fix: verify and dedupe self-log entries before AheadOfSelf in check_next | Fix: `check_next` (src/sync/chain.rs) returns `ChainStep::AheadOfSelf` for ANY entry naming this node's active self log, before the duplicate and signature chec |
| ht-5fr | epic | Cross-node herdr-threads sync | Cross-node herdr-threads sync: herdr-threads daemons on SSH-linked machines (one node per machine+OS user) share one thread space through partitions of any leng |
| ht-5fr.59 | task | review: ht-5fr.53 | Review, fix and merge of ht-5fr.53, tracked separately so ht-5fr.53's dependents can start once it is implemented. |
| ht-5fr.58 | bug | Large lazy body shown by accept then followed via inbox continuation never completes (disp | Found by ht-5fr.57 e2e (tests/hook_entrypoint.rs text_accept_of_an_ordinary_invitation_shows_a_large_body_first_chunk_and_inbox_restarts_it). |
| ht-5fr.52 | task | review: ht-5fr.32 | Review, fix and merge of ht-5fr.32, tracked separately so ht-5fr.32's dependents can start once it is implemented. |
| ht-5fr.50 | task | review: ht-5fr.31 | Review, fix and merge of ht-5fr.31, tracked separately so ht-5fr.31's dependents can start once it is implemented. |
| ht-5fr.48 | task | review: ht-5fr.36 | Review, fix and merge of ht-5fr.36, tracked separately so ht-5fr.36's dependents can start once it is implemented. |
| ht-5fr.47 | task | review: ht-5fr.34 | Review, fix and merge of ht-5fr.34, tracked separately so ht-5fr.34's dependents can start once it is implemented. |
| ht-5fr.4.42 | task | review: ht-5fr.4.40 | Review, fix and merge of ht-5fr.4.40, tracked separately so ht-5fr.4.40's dependents can start once it is implemented. |
| ht-5fr.4.41 | task | review: ht-5fr.4.37 | Review, fix and merge of ht-5fr.4.37, tracked separately so ht-5fr.4.37's dependents can start once it is implemented. |
| ht-5fr.4.39 | task | review: ht-5fr.4.38 | Review, fix and merge of ht-5fr.4.38, tracked separately so ht-5fr.4.38's dependents can start once it is implemented. |
| ht-5fr.5.30 | task | review: ht-5fr.5.29 | Review, fix and merge of ht-5fr.5.29, tracked separately so ht-5fr.5.29's dependents can start once it is implemented. |
| ht-5fr.30 | task | review: ht-5fr.11 | Review, fix and merge of ht-5fr.11, tracked separately so ht-5fr.11's dependents can start once it is implemented. |
| ht-5fr.6.34 | task | review: ht-5fr.6.11 | Review, fix and merge of ht-5fr.6.11, tracked separately so ht-5fr.6.11's dependents can start once it is implemented. |
| ht-5fr.4.36 | task | review: ht-5fr.4.18 | Review, fix and merge of ht-5fr.4.18, tracked separately so ht-5fr.4.18's dependents can start once it is implemented. |
| ht-5fr.29 | task | review: ht-5fr.12 | Review, fix and merge of ht-5fr.12, tracked separately so ht-5fr.12's dependents can start once it is implemented. |
| ht-5fr.28 | task | review: ht-5fr.13 | Review, fix and merge of ht-5fr.13, tracked separately so ht-5fr.13's dependents can start once it is implemented. |
| ht-5fr.4.35 | task | review: ht-5fr.4.13 | Review, fix and merge of ht-5fr.4.13, tracked separately so ht-5fr.4.13's dependents can start once it is implemented. |
| ht-5fr.4.34 | task | review: ht-5fr.4.12 | Review, fix and merge of ht-5fr.4.12, tracked separately so ht-5fr.4.12's dependents can start once it is implemented. |
| ht-5fr.4.33 | task | review: ht-5fr.4.10 | Review, fix and merge of ht-5fr.4.10, tracked separately so ht-5fr.4.10's dependents can start once it is implemented. |
| ht-5fr.4.32 | task | review: ht-5fr.4.16 | Review, fix and merge of ht-5fr.4.16, tracked separately so ht-5fr.4.16's dependents can start once it is implemented. |
| ht-5fr.4.31 | task | review: ht-5fr.4.11 | Review, fix and merge of ht-5fr.4.11, tracked separately so ht-5fr.4.11's dependents can start once it is implemented. |
| ht-5fr.4.30 | task | review: ht-5fr.4.9 | Review, fix and merge of ht-5fr.4.9, tracked separately so ht-5fr.4.9's dependents can start once it is implemented. |
| ht-5fr.4.29 | task | review: ht-5fr.4.14 | Review, fix and merge of ht-5fr.4.14, tracked separately so ht-5fr.4.14's dependents can start once it is implemented. |
| ht-5fr.4.28 | task | review: ht-5fr.4.8 | Review, fix and merge of ht-5fr.4.8, tracked separately so ht-5fr.4.8's dependents can start once it is implemented. |
| ht-5fr.4.27 | task | review: ht-5fr.4.7 | Review, fix and merge of ht-5fr.4.7, tracked separately so ht-5fr.4.7's dependents can start once it is implemented. |
| ht-5fr.2.26 | task | review: ht-5fr.2.25 | Review, fix and merge of ht-5fr.2.25, tracked separately so ht-5fr.2.25's dependents can start once it is implemented. |
| ht-5fr.4.26 | task | review: ht-5fr.4.5 | Review, fix and merge of ht-5fr.4.5, tracked separately so ht-5fr.4.5's dependents can start once it is implemented. |
| ht-5fr.4.25 | task | review: ht-5fr.4.17 | Review, fix and merge of ht-5fr.4.17, tracked separately so ht-5fr.4.17's dependents can start once it is implemented. |
| ht-5fr.4.24 | task | review: ht-5fr.4.6 | Review, fix and merge of ht-5fr.4.6, tracked separately so ht-5fr.4.6's dependents can start once it is implemented. |
| ht-5fr.4.23 | task | review: ht-5fr.4.4 | Review, fix and merge of ht-5fr.4.4, tracked separately so ht-5fr.4.4's dependents can start once it is implemented. |
| ht-5fr.4.22 | task | review: ht-5fr.4.2 | Review, fix and merge of ht-5fr.4.2, tracked separately so ht-5fr.4.2's dependents can start once it is implemented. |
| ht-5fr.27 | task | review: ht-5fr.23 | Review, fix and merge of ht-5fr.23, tracked separately so ht-5fr.23's dependents can start once it is implemented. |
| ht-5fr.26 | task | review: ht-5fr.22 | Review, fix and merge of ht-5fr.22, tracked separately so ht-5fr.22's dependents can start once it is implemented. |
| ht-5fr.6.33 | task | review: ht-5fr.6.30 | Review, fix and merge of ht-5fr.6.30, tracked separately so ht-5fr.6.30's dependents can start once it is implemented. |
| ht-5fr.25 | task | review: ht-5fr.24 | Review, fix and merge of ht-5fr.24, tracked separately so ht-5fr.24's dependents can start once it is implemented. |
| ht-5fr.5.28 | task | review: ht-5fr.5.27 | Review, fix and merge of ht-5fr.5.27, tracked separately so ht-5fr.5.27's dependents can start once it is implemented. |
| ht-5fr.6.32 | task | review: ht-5fr.6.31 | Review, fix and merge of ht-5fr.6.31, tracked separately so ht-5fr.6.31's dependents can start once it is implemented. |
| ht-5fr.3.38 | task | review: ht-5fr.3.37 | Review, fix and merge of ht-5fr.3.37, tracked separately so ht-5fr.3.37's dependents can start once it is implemented. |
| ht-5fr.21 | task | review: ht-5fr.18 | Review, fix and merge of ht-5fr.18, tracked separately so ht-5fr.18's dependents can start once it is implemented. |
| ht-5fr.2.24 | task | review: ht-5fr.2.10 | Review, fix and merge of ht-5fr.2.10, tracked separately so ht-5fr.2.10's dependents can start once it is implemented. |
| ht-5fr.2.23 | task | review: ht-5fr.2.12 | Review, fix and merge of ht-5fr.2.12, tracked separately so ht-5fr.2.12's dependents can start once it is implemented. |
| ht-5fr.20 | task | review: ht-5fr.14 | Review, fix and merge of ht-5fr.14, tracked separately so ht-5fr.14's dependents can start once it is implemented. |
| ht-5fr.19 | task | review: ht-5fr.8 | Review, fix and merge of ht-5fr.8, tracked separately so ht-5fr.8's dependents can start once it is implemented. |
| ht-5fr.6.29 | task | review: ht-5fr.6.14 | Review, fix and merge of ht-5fr.6.14, tracked separately so ht-5fr.6.14's dependents can start once it is implemented. |
| ht-5fr.17 | task | review: ht-5fr.7 | Review, fix and merge of ht-5fr.7, tracked separately so ht-5fr.7's dependents can start once it is implemented. |
| ht-5fr.2.22 | task | review: ht-5fr.2.9 | Review, fix and merge of ht-5fr.2.9, tracked separately so ht-5fr.2.9's dependents can start once it is implemented. |
| ht-5fr.6.28 | task | review: ht-5fr.6.15 | Review, fix and merge of ht-5fr.6.15, tracked separately so ht-5fr.6.15's dependents can start once it is implemented. |
| ht-5fr.6.27 | task | review: ht-5fr.6.10 | Review, fix and merge of ht-5fr.6.10, tracked separately so ht-5fr.6.10's dependents can start once it is implemented. |
| ht-5fr.5.26 | task | review: ht-5fr.5.11 | Review, fix and merge of ht-5fr.5.11, tracked separately so ht-5fr.5.11's dependents can start once it is implemented. |
| ht-5fr.16 | task | review: ht-5fr.10 | Review, fix and merge of ht-5fr.10, tracked separately so ht-5fr.10's dependents can start once it is implemented. |
| ht-5fr.5.25 | task | review: ht-5fr.5.10 | Review, fix and merge of ht-5fr.5.10, tracked separately so ht-5fr.5.10's dependents can start once it is implemented. |
| ht-5fr.6.26 | task | review: ht-5fr.6.13 | Review, fix and merge of ht-5fr.6.13, tracked separately so ht-5fr.6.13's dependents can start once it is implemented. |
| ht-5fr.2.21 | task | review: ht-5fr.2.8 | Review, fix and merge of ht-5fr.2.8, tracked separately so ht-5fr.2.8's dependents can start once it is implemented. |
| ht-5fr.6.25 | task | review: ht-5fr.6.12 | Review, fix and merge of ht-5fr.6.12, tracked separately so ht-5fr.6.12's dependents can start once it is implemented. |
| ht-5fr.6.24 | task | review: ht-5fr.6.7 | Review, fix and merge of ht-5fr.6.7, tracked separately so ht-5fr.6.7's dependents can start once it is implemented. |
| ht-5fr.5.24 | task | review: ht-5fr.5.6 | Review, fix and merge of ht-5fr.5.6, tracked separately so ht-5fr.5.6's dependents can start once it is implemented. |
| ht-5fr.2.20 | task | review: ht-5fr.2.6 | Review, fix and merge of ht-5fr.2.6, tracked separately so ht-5fr.2.6's dependents can start once it is implemented. |
| ht-5fr.6.23 | task | review: ht-5fr.6.8 | Review, fix and merge of ht-5fr.6.8, tracked separately so ht-5fr.6.8's dependents can start once it is implemented. |
| ht-5fr.6.22 | task | review: ht-5fr.6.6 | Review, fix and merge of ht-5fr.6.6, tracked separately so ht-5fr.6.6's dependents can start once it is implemented. |
| ht-5fr.5.23 | task | review: ht-5fr.5.13 | Review, fix and merge of ht-5fr.5.13, tracked separately so ht-5fr.5.13's dependents can start once it is implemented. |
| ht-5fr.6.21 | task | review: ht-5fr.6.5 | Review, fix and merge of ht-5fr.6.5, tracked separately so ht-5fr.6.5's dependents can start once it is implemented. |
| ht-5fr.6.20 | task | review: ht-5fr.6.9 | Review, fix and merge of ht-5fr.6.9, tracked separately so ht-5fr.6.9's dependents can start once it is implemented. |
| ht-5fr.4.21 | task | review: ht-5fr.4.15 | Review, fix and merge of ht-5fr.4.15, tracked separately so ht-5fr.4.15's dependents can start once it is implemented. |
| ht-5fr.5.22 | task | review: ht-5fr.5.12 | Review, fix and merge of ht-5fr.5.12, tracked separately so ht-5fr.5.12's dependents can start once it is implemented. |
| ht-5fr.4.20 | task | review: ht-5fr.4.3 | Review, fix and merge of ht-5fr.4.3, tracked separately so ht-5fr.4.3's dependents can start once it is implemented. |
| ht-5fr.2.19 | task | review: ht-5fr.2.7 | Review, fix and merge of ht-5fr.2.7, tracked separately so ht-5fr.2.7's dependents can start once it is implemented. |
| ht-5fr.3.36 | task | review: ht-5fr.3.16 | Review, fix and merge of ht-5fr.3.16, tracked separately so ht-5fr.3.16's dependents can start once it is implemented. |
| ht-5fr.3.35 | task | review: ht-5fr.3.21 | Review, fix and merge of ht-5fr.3.21, tracked separately so ht-5fr.3.21's dependents can start once it is implemented. |
| ht-5fr.3.34 | task | review: ht-5fr.3.32 | Review, fix and merge of ht-5fr.3.32, tracked separately so ht-5fr.3.32's dependents can start once it is implemented. |
| ht-5fr.2.18 | task | review: ht-5fr.2.5 | Review, fix and merge of ht-5fr.2.5, tracked separately so ht-5fr.2.5's dependents can start once it is implemented. |
| ht-5fr.5.21 | task | review: ht-5fr.5.7 | Review, fix and merge of ht-5fr.5.7, tracked separately so ht-5fr.5.7's dependents can start once it is implemented. |
| ht-5fr.6.19 | task | review: ht-5fr.6.2 | Review, fix and merge of ht-5fr.6.2, tracked separately so ht-5fr.6.2's dependents can start once it is implemented. |
| ht-5fr.5.20 | task | review: ht-5fr.5.8 | Review, fix and merge of ht-5fr.5.8, tracked separately so ht-5fr.5.8's dependents can start once it is implemented. |
| ht-5fr.2.17 | task | review: ht-5fr.2.2 | Review, fix and merge of ht-5fr.2.2, tracked separately so ht-5fr.2.2's dependents can start once it is implemented. |
| ht-5fr.5.19 | task | review: ht-5fr.5.5 | Review, fix and merge of ht-5fr.5.5, tracked separately so ht-5fr.5.5's dependents can start once it is implemented. |
| ht-5fr.3.33 | task | review: ht-5fr.3.13 | Review, fix and merge of ht-5fr.3.13, tracked separately so ht-5fr.3.13's dependents can start once it is implemented. |
| ht-5fr.6.18 | task | review: ht-5fr.6.3 | Review, fix and merge of ht-5fr.6.3, tracked separately so ht-5fr.6.3's dependents can start once it is implemented. |
| ht-5fr.2.16 | task | review: ht-5fr.2.4 | Review, fix and merge of ht-5fr.2.4, tracked separately so ht-5fr.2.4's dependents can start once it is implemented. |
| ht-5fr.5.18 | task | review: ht-5fr.5.4 | Review, fix and merge of ht-5fr.5.4, tracked separately so ht-5fr.5.4's dependents can start once it is implemented. |
| ht-5fr.3.31 | task | review: ht-5fr.3.14 | Review, fix and merge of ht-5fr.3.14, tracked separately so ht-5fr.3.14's dependents can start once it is implemented. |
| ht-5fr.3.30 | task | review: ht-5fr.3.10 | Review, fix and merge of ht-5fr.3.10, tracked separately so ht-5fr.3.10's dependents can start once it is implemented. |
| ht-5fr.3.29 | task | review: ht-5fr.3.9 | Review, fix and merge of ht-5fr.3.9, tracked separately so ht-5fr.3.9's dependents can start once it is implemented. |
| ht-5fr.1.25 | task | review: ht-5fr.1.13 | Review, fix and merge of ht-5fr.1.13, tracked separately so ht-5fr.1.13's dependents can start once it is implemented. |
| ht-5fr.3.28 | task | review: ht-5fr.3.8 | Review, fix and merge of ht-5fr.3.8, tracked separately so ht-5fr.3.8's dependents can start once it is implemented. |
| ht-5fr.2.15 | task | review: ht-5fr.2.3 | Review, fix and merge of ht-5fr.2.3, tracked separately so ht-5fr.2.3's dependents can start once it is implemented. |
| ht-5fr.1.24 | task | review: ht-5fr.1.8 | Review, fix and merge of ht-5fr.1.8, tracked separately so ht-5fr.1.8's dependents can start once it is implemented. |
| ht-5fr.5.17 | task | review: ht-5fr.5.3 | Review, fix and merge of ht-5fr.5.3, tracked separately so ht-5fr.5.3's dependents can start once it is implemented. |
| ht-5fr.6.17 | task | review: ht-5fr.6.4 | Review, fix and merge of ht-5fr.6.4, tracked separately so ht-5fr.6.4's dependents can start once it is implemented. |
| ht-5fr.3.27 | task | review: ht-5fr.3.15 | Review, fix and merge of ht-5fr.3.15, tracked separately so ht-5fr.3.15's dependents can start once it is implemented. |
| ht-5fr.3.26 | task | review: ht-5fr.3.12 | Review, fix and merge of ht-5fr.3.12, tracked separately so ht-5fr.3.12's dependents can start once it is implemented. |
| ht-5fr.3.25 | task | review: ht-5fr.3.7 | Review, fix and merge of ht-5fr.3.7, tracked separately so ht-5fr.3.7's dependents can start once it is implemented. |
| ht-5fr.1.23 | task | review: ht-5fr.1.9 | Review, fix and merge of ht-5fr.1.9, tracked separately so ht-5fr.1.9's dependents can start once it is implemented. |
| ht-5fr.6.16 | task | review: ht-5fr.6.1 | Review, fix and merge of ht-5fr.6.1, tracked separately so ht-5fr.6.1's dependents can start once it is implemented. |
| ht-5fr.1.22 | task | review: ht-5fr.1.11 | Review, fix and merge of ht-5fr.1.11, tracked separately so ht-5fr.1.11's dependents can start once it is implemented. |
| ht-5fr.3.24 | task | review: ht-5fr.3.6 | Review, fix and merge of ht-5fr.3.6, tracked separately so ht-5fr.3.6's dependents can start once it is implemented. |
| ht-5fr.2.14 | task | review: ht-5fr.2.11 | Review, fix and merge of ht-5fr.2.11, tracked separately so ht-5fr.2.11's dependents can start once it is implemented. |
| ht-5fr.5.16 | task | review: ht-5fr.5.2 | Review, fix and merge of ht-5fr.5.2, tracked separately so ht-5fr.5.2's dependents can start once it is implemented. |
| ht-5fr.5.15 | task | review: ht-5fr.5.9 | Review, fix and merge of ht-5fr.5.9, tracked separately so ht-5fr.5.9's dependents can start once it is implemented. |
| ht-5fr.1.21 | task | review: ht-5fr.1.12 | Review, fix and merge of ht-5fr.1.12, tracked separately so ht-5fr.1.12's dependents can start once it is implemented. |
| ht-5fr.1.20 | task | review: ht-5fr.1.10 | Review, fix and merge of ht-5fr.1.10, tracked separately so ht-5fr.1.10's dependents can start once it is implemented. |
| ht-5fr.1.19 | task | review: ht-5fr.1.7 | Review, fix and merge of ht-5fr.1.7, tracked separately so ht-5fr.1.7's dependents can start once it is implemented. |
| ht-5fr.2.13 | task | review: ht-5fr.2.1 | Review, fix and merge of ht-5fr.2.1, tracked separately so ht-5fr.2.1's dependents can start once it is implemented. |
| ht-5fr.1.18 | task | review: ht-5fr.1.6 | Review, fix and merge of ht-5fr.1.6, tracked separately so ht-5fr.1.6's dependents can start once it is implemented. |
| ht-5fr.3.23 | task | review: ht-5fr.3.11 | Review, fix and merge of ht-5fr.3.11, tracked separately so ht-5fr.3.11's dependents can start once it is implemented. |
| ht-5fr.4.19 | task | review: ht-5fr.4.1 | Review, fix and merge of ht-5fr.4.1, tracked separately so ht-5fr.4.1's dependents can start once it is implemented. |
| ht-5fr.3.22 | task | review: ht-5fr.3.4 | Review, fix and merge of ht-5fr.3.4, tracked separately so ht-5fr.3.4's dependents can start once it is implemented. |
| ht-5fr.1.17 | task | review: ht-5fr.1.5 | Review, fix and merge of ht-5fr.1.5, tracked separately so ht-5fr.1.5's dependents can start once it is implemented. |
| ht-5fr.1.16 | task | review: ht-5fr.1.3 | Review, fix and merge of ht-5fr.1.3, tracked separately so ht-5fr.1.3's dependents can start once it is implemented. |
| ht-5fr.5.14 | task | review: ht-5fr.5.1 | Review, fix and merge of ht-5fr.5.1, tracked separately so ht-5fr.5.1's dependents can start once it is implemented. |
| ht-5fr.3.20 | task | review: ht-5fr.3.3 | Review, fix and merge of ht-5fr.3.3, tracked separately so ht-5fr.3.3's dependents can start once it is implemented. |
| ht-5fr.3.19 | task | review: ht-5fr.3.5 | Review, fix and merge of ht-5fr.3.5, tracked separately so ht-5fr.3.5's dependents can start once it is implemented. |
| ht-5fr.3.18 | task | review: ht-5fr.3.2 | Review, fix and merge of ht-5fr.3.2, tracked separately so ht-5fr.3.2's dependents can start once it is implemented. |
| ht-5fr.3.17 | task | review: ht-5fr.3.1 | Review, fix and merge of ht-5fr.3.1, tracked separately so ht-5fr.3.1's dependents can start once it is implemented. |
| ht-5fr.1.15 | task | review: ht-5fr.1.2 | Review, fix and merge of ht-5fr.1.2, tracked separately so ht-5fr.1.2's dependents can start once it is implemented. |
| ht-5fr.1.14 | task | review: ht-5fr.1.4 | Review, fix and merge of ht-5fr.1.4, tracked separately so ht-5fr.1.4's dependents can start once it is implemented. |
| ht-5fr.15 | task | Integration sweep: cross-node herdr-threads sync | Root integration sweep for the cross-node sync tree. |
| ht-5fr.14 | task | Real-ssh two-node check in the Linux Docker runner | A pre-release check that two nodes link over the DEFAULT ssh transport (no sync.dial_command), which every other test replaces (spec: peer-link design Transport |
| ht-5fr.1.13 | task | Forward node-schema migrations after the baseline | Add the post-release schema evolution path so the first change after the release that ships the node store does not also have to invent it (spec: node-identity  |
| ht-5fr.1.12 | task | ID consumers: tagged and d… IDs in CLI parsing, delivery framing, Claude mod and harness a | Audit and update every ID parser, validator and test fixture outside the store so tagged public IDs <prefix><tag><8> (12 chars) and direct-thread IDs d… (11 cha |
| ht-5fr.6.15 | task | Daemon composition of the incarnations runtime (app.rs) | Compose the incarnations runtime into the daemon in place of `LogOnlyForkSink`, split from ht-5fr.6.9 at promotion review (spec §12 Composition; Key abstraction |
| ht-5fr.6.14 | task | Resync rebuild wiring: plan composition, ResyncPin in reselect, salvage reader, two-daemon | The rebuild half of `sync resync`: turning a resync plan into a pinned, salvaging restart, split from ht-5fr.6.10 at promotion review (spec §11 "After `journal: |
| ht-5fr.6.13 | task | IncarnationsWorker thread: queue, coalescing, cap, writer quanta, backoff, third-party and | The `herdr-incarnations` repair thread that turns sink jobs into writer quanta, split from ht-5fr.6.9 at promotion review (spec §5 third-party path; §6 owner de |
| ht-5fr.6.12 | task | IncarnationStore status, declared-freeze adoption and random bootstrap call site | The read side and the remaining IncarnationStore entry points that other modules call: repair status for ht-5fr.8, the I2/I3 checks for `sync verify`, adopting  |
| ht-5fr.6.11 | task | Incarnations convergence scenarios over in-process stores | End-to-end proof that forks, restores and wipes converge, with three to four in-process stores, no daemons (spec Testing "Convergence scenarios"). |
| ht-5fr.6.10 | task | Resync marker, phases, request preconditions and step-0 backup/move in app.rs | The `sync resync` request and the restart-time step 0 that backs up and moves the damaged store; the CLI and operator gate are ht-5fr.8's, and turning the plan  |
| ht-5fr.6.9 | task | IncarnationsSink front and request table | The `ForkSink` implementation's non-blocking front, the `Job` set it enqueues and the branch-request table (spec §5 third-party path; §6 owner detection; §7 pul |
| ht-5fr.6.8 | task | Coordinator fork wiring: on_have, branch frame routing and serving, outbox drain, fork ref | The coordinator half of fork handling, tested against a recording fake ForkSink (spec §4 serving of `branch_want`; §12 Coordinator additions; Key abstractions S |
| ht-5fr.6.7 | task | Owner and restore rotation, branch pulls and the clone gate in IncarnationStore | The owner's repair transactions: rotate away from a forked or restored active log, record branch tips and admit pulled branch entries, all behind the clone gate |
| ht-5fr.6.6 | task | Re-issue pass: interleaved union re-issue of frozen own logs | The persisted, kind-agnostic, idempotent job that re-issues every branch tail of a frozen own log into the active incarnation (spec §9 Re-issue job; §13 Trust p |
| ht-5fr.6.5 | task | Journal reissue and ApplyMode::Reissue with re-issue suppression and own-history attributi | Journal's second authoring entry point and the apply mode it runs under, kind-agnostic (spec §8 `reissue`; §10 Apply on the owner during re-issue; Key abstracti |
| ht-5fr.6.4 | task | IncarnationStore: fork tables, triggers, record_fork and next_incarnation | The store owner of fork repair state, with the first repair decision that turns proofs into rows (spec §2 Schema additions; §5 record_fork; §6 next_incarnation; |
| ht-5fr.6.3 | task | Journal serve_branch and log_meta on JournalPort, SqliteStore and MemoryJournal | Serving a branch that ends at a given tip, from `journal` or by a bounded backward walk through `journal_forked` (spec §4 Serving; §8 `serve_branch`; Key abstra |
| ht-5fr.6.2 | task | Fork wire seam: fork.v1 frames, branch_entries credit and the ForkSink extension | The wire and trait seam every other fork leaf builds on, landed inert (spec §4 Wire fork.v1; §12 ForkSink extension; §1 rows src/sync/frame.rs and src/sync/link |
| ht-5fr.6.1 | task | ForkProof: pure fork classification (classify_pair, classify_ahead_of_self) | The one statement of what counts as a fork, as pure functions with no store, clock or network (spec §3 Proofs; §1 row src/sync/incarnations.rs; Key abstractions |
| ht-5fr.4.18 | task | Fact kinds integration: cross-kind convergence, deadline scope and oracle workload | Cross-kind convergence that needs several kind leaves at once: retirement against sends, overdue scope across two nodes, archival with remote members, and the o |
| ht-5fr.4.17 | task | Send-fence narrowing (§16) with reader audit | Narrow the send publish fence so unrelated timeline traffic (remote messages, other local senders) no longer invalidates in-flight local preparations, after aud |
| ht-5fr.4.16 | task | message.sent remote-recipient staging and synthesized publication branch; cleanup/abandon  | Let a send reach another node: stage remote recipients on the author, synthesize the preparation from the payload on every other node, and keep cleanup away fro |
| ht-5fr.4.15 | task | Attribution: fact_attribution_staging, kicks classification, local_journal provenance in T | Local attribution staging for replicated rows, its `local_journal` fallback provenance in TRUST-POLICY, and the two-store exchange helper every convergence test |
| ht-5fr.4.14 | task | Remote seats in node-local workers and reads; archival remote-member rule | Every node-local worker selects local seats only, reads show remote seats compactly, and automatic archival defers threads with a live remote member (spec §14;  |
| ht-5fr.4.13 | task | Deadline lane scope: only the authoring node records overdue | Make every overdue path refuse obligations authored on another node, and scope the due scans to match (spec §13; root §7 "recipient nodes never emit overdue for |
| ht-5fr.4.12 | task | receipt.settled: per-receipt settlement and the batch ACK event | Author one `receipt.settled` per newly settled receipt and move the batch-level writes into the handler, so every node derives the same ACK history (spec §9; §1 |
| ht-5fr.4.11 | task | message.sent kind, author build in publish_send and the local-adopt materializer branch (+ | Author every send as `message.sent` carrying the complete audience, and publish it on the authoring node through the local-adopt branch of the one materializer  |
| ht-5fr.4.10 | task | Invitation facts and direct pairs: invitation.created/answered, ensure_direct_thread | Author invitations and answers as facts, settle crossed invites `already_joined`, and convert direct-pair creation to the deterministic `d…` thread (spec §6; §5 |
| ht-5fr.4.9 | task | Membership facts: membership.joined / membership.left and the already_joined rule | Author membership transitions as facts with the home-node ordinal for idempotence, and own the one `already_joined` settlement rule (spec §7; §6 `settle_superse |
| ht-5fr.4.8 | task | Thread facts: thread.created, thread.register and thread.direct | Author thread creation and the LWW registers as facts, including automatic archival, and land the `thread.direct` kind (spec §5; §11 event keys; §15 "The decisi |
| ht-5fr.4.7 | task | Seat retirement and remote eligibility: seat.retired and replicated retirements | Retire a published seat on every node through `seat.retired`, reuse the retirement job for replicated retirements, and let remote seats be eligible invitees and |
| ht-5fr.4.6 | task | DeliveryTail: extract the local delivery tail (behaviour-preserving) | Extract the one writer of local delivery rows for messages and invitations, behaviour-preserving for local seats, with the locality guard in its single attentio |
| ht-5fr.4.5 | task | FactRoute and seat publication: seats::ensure_published and seat.created | The one place a command turns a decided payload into rows, plus publication of local seats at enrollment so remote seats are discoverable (spec §2 authoring pat |
| ht-5fr.4.4 | task | WriteGuard and the replicated projection for converted tables | Test-support enforcement of the write-path closure plus the oracle's view of the converted tables, so each fact-kind leaf can prove its commands write replicate |
| ht-5fr.4.3 | task | Fact seams: MaterializeInput, fact payloads and the v1 kind module layout | The inert seam every fact-kind leaf builds on: the materializer input, the shared payload value types and key builders, and the six entity-grouped kind modules  |
| ht-5fr.4.2 | task | Fact-kind schema additions in the node baseline | Land the columns, CHECKs and indexes the v1 fact kinds need, inert: existing writers fill them with the single-node values so behaviour is unchanged (spec §2 `t |
| ht-5fr.4.1 | task | Split file: src/store/control.rs into per-entity submodules (move-only) | Move-only, behaviour-preserving split of `src/store/control.rs` (2.8k lines) so the parallel fact-kind leaves (schema insert sites, DeliveryTail, seat retiremen |
| ht-5fr.5.13 | task | Coordinator serve, live tail, ingest worker outcomes and Have deltas | Moving entries over routed links: Want serving, live tail, the sequential ingest worker with its outcome handling, and Have deltas on kicks (spec §9 Want servin |
| ht-5fr.5.12 | task | Coordinator routing: duplicate-link standby, Have/Want, rx reselect and failover | Routing over established links on top of the coordinator skeleton: duplicate-link standby and promotion, vector exchange (Have/ask/Want/Unwant), rx-source selec |
| ht-5fr.5.11 | task | Daemon idle exit for a store-only daemon with no links, targets or clients | Create the daemon's idle exit (none exists today) under the root's name, narrowly: only a store-only daemon with nothing to serve exits (spec §15 Idle exit). |
| ht-5fr.5.10 | task | Compose the sync runtime into the daemon: start, shutdown and two-daemon link tests | Start the coordinator, Dialer and peer intake in the elected daemon and stop them cleanly, proving two real daemons link and converge (spec §15 Composition and  |
| ht-5fr.5.9 | task | peer --stdio stateless pump | The remote side of a link: a stateless two-thread splice between ssh stdio and the node daemon socket (spec §2 `peer --stdio`: the stateless pump). |
| ht-5fr.5.8 | task | Daemon transport peer sniffing: hello sniff, PeerIntake hand-off and ConnectionGauge | Admit inbound peer connections on the existing daemon socket and count local clients for idle exit (spec §3 Transport: first-frame peer sniffing and admission). |
| ht-5fr.5.7 | task | Dialer: dial tasks, transport command, full-jitter backoff, child ownership and sync.dial_ | The only place sync child processes are spawned, owned and classified, plus the setting tests use to dial without ssh (spec §13 Runtime, DialTask loop, Transpor |
| ht-5fr.5.6 | task | Reachability gossip, contact flush and the SyncSignals round-complete signal | Wire per-node reachability and the "first sync round complete" signal into the coordinator for ht-5fr.13 (spec §12 Reachability and the round-complete signal; § |
| ht-5fr.5.5 | task | SyncCoordinator actor skeleton: CoordinatorDeps, link entry points, SyncHandle, PeerIntake | The coordinator's actor skeleton and its seams: composition deps, inbound and dialed link entry points, the SyncHandle facade and the ForkSink contract (spec §8 |
| ht-5fr.5.4 | task | Sync store: sync_dial_targets and sync_node_contact tables, DialTargets, SyncContacts/Cont | The coordinator's persistence layer and its narrow port, with the in-memory contact book whose flush it serves (spec §13 Table, Store API, SyncStorePort; §12 Co |
| ht-5fr.5.3 | task | Lane::Sync and its kick table map | Add the Sync lane to the closed lane set so journal and sync-log commits kick it (spec §11 Have deltas and Lane::Sync; §15 composition step 1 pacer arrays). |
| ht-5fr.5.2 | task | PeerLink: preamble reader, mutually signed handshake, credit window and liveness | The link runtime over any byte stream (spec §5 Handshake and mutual authentication; §6 Credit window and liveness; §7 PeerLink runtime; §4 preamble outcomes). |
| ht-5fr.5.1 | task | Peer wire frames: PeerFrame, PeerHello, CloseReason and preamble constants | The versionless peer wire vocabulary, landed inert as the seam every other ht-5fr.5 leaf builds on (spec §4 Wire: preamble, framing, PeerFrame, CloseReason; §1  |
| ht-5fr.2.12 | task | Dispatch by claim host: DomainService hosts, lazy resolve, per-host bootstrap, interim gua | Route every target-bearing command and bootstrap path to the session of the host it names, attach lazily on the request path, and delete the remaining single-ho |
| ht-5fr.2.11 | task | Seat-row host joins: rewrite queries to join herdr_hosts by seats.host_id | Rewrite every non-cooperative, non-observation query that reads the primary host to take its host from the seat row, so delivery, attention and history paths wo |
| ht-5fr.3.16 | task | ApplyContext services and own-history hold: provenance, is_local_seat, delivery_suppressed | The ApplyContext services handlers use for attribution and delivery: provenance, local-seat check and catch-up suppression, plus the own-history hold that makes |
| ht-5fr.3.15 | task | Journal open in daemon start and auto-open test constructors | Wire `journal::open` into daemon start and make the SqliteStore test constructors open the journal, so every existing suite runs against an opened journal (spec |
| ht-5fr.2.10 | task | Health and doctor per attached host | Report one Health entry per attached host and fold unreachable-host limitations (spec §10). |
| ht-5fr.2.9 | task | Archival per-host evidence: archival_hosts, HostArchivalEvidence and session samplers | Keep archival one node-wide lane while making coherence and sampling per host, so an outage on one server never resets quiet periods of threads that live on ano |
| ht-5fr.2.8 | task | Compose HostSessions into the daemon: HostAttachPlan, startup attach, stop_all, health/arc | Wire HostSessions into the elected daemon: build it from a HostAttachPlan, attach at startup, stop it with the daemon, and hand `Arc<HostSessions>` to the healt |
| ht-5fr.2.7 | task | HostSessions registry: HostSession, HostFactory/HostAttachPlan, attach, sweep and per-host | Build the multi-host registry as a service module, not yet composed into the daemon (spec §2; §3 attach sequence, concurrency, startup, sweep, no detach; §6 per |
| ht-5fr.2.6 | task | cooperative_claim: store entry check by claim host; remove StoreContext.primary_host | Replace `seats::cooperative_instance` with `cooperative_claim`, which resolves `claim.host` through HostIndex and returns the `HostId` the transaction uses, and |
| ht-5fr.2.5 | task | Wake scope per host: ScheduledStore::for_host and host-filtered wake store calls | Scope every wake query to one host so each per-host wake worker serves only its own seats (spec §7 "Wake scope"; Key abstractions "ScheduledStore / wake store c |
| ht-5fr.2.4 | task | Baseline re-key to host_id and per-host observation queries | Re-key every Herdr-scope child table to `host_id`, tighten the seats CHECK, and make every non-cooperative query take its host from the seat row or the owning w |
| ht-5fr.2.3 | task | Claim host on the wire: CallerClaim.host, target-bearing commands, fences and handoff comp | Carry the server locator on every claim and target-bearing command (spec §1 "CallerClaim", "Target-bearing commands", `Command::host_locator`, `PROTOCOL_VERSION |
| ht-5fr.2.2 | task | CommitKicks fan-out: per-lane pacer lists and unregister | Make `CommitKicks` lane slots lists of pacers so a lane kick reaches every registered pacer (spec §7 "Kick fan-out"; Key abstractions "CommitKicks"). |
| ht-5fr.2.1 | task | Host addressing seam: HostLocator, HostId, HostIndex and store::hosts | Land the host addressing types and the store's attached-host view, inert by default (spec §1 first two bullets; §4 `src/store/hosts.rs` and HostIndex; §3 attach |
| ht-5fr.3.14 | task | Journal replay oracle (test_support::journal_oracle) | Test-support harness proving "replaying a journal into an empty store reproduces the replicated tables" (spec Goal; Testing "Oracle"). |
| ht-5fr.3.13 | task | Apply lane worker and daemon wiring | Run ApplyEngine as a daemon lane: a `herdr-apply` worker kicked by `journal` commits, with startup replay, backoff and Health reporting (spec §13 Worker, includ |
| ht-5fr.3.12 | task | Lane::Apply, LaneSet u16 and kicks_for_commit | Add the Apply lane to the closed lane set, widen the bitset for ht-5fr.5's `Sync`, and make one rule decide which lanes a commit kicks (spec §13 Lane::Apply; §4 |
| ht-5fr.3.11 | task | ThreadRegisters: LWW store module for thread_registers | The only writer of `thread_registers`, with the `(HlcStamp, NodeUuid)` LWW tie-break that ht-5fr.4's `thread.created`/`thread.register` handlers will call (spec |
| ht-5fr.3.10 | task | Node kinds: node.retag and tag collision handling with post-commit identity update | Complete the two v1 node kinds: tag collision detection on genesis/retag apply, retags authored only as follow-ups by the higher node, and the post-commit tag u |
| ht-5fr.3.9 | task | MemoryJournal and the JournalPort contract suite | The second `JournalPort` implementation for ht-5fr.5's link and routing tests, proven equivalent to `SqliteStore` by one contract suite (spec §12 JournalPort an |
| ht-5fr.3.8 | task | ApplyEngine passes, replay and follow-ups; SqliteStore implements JournalPort | Complete the one apply path for ingested entries: causal readiness, parked re-attempts, replay and follow-ups, reached through `apply_pending` (spec §7 ApplyEng |
| ht-5fr.3.7 | task | Journal ingest, serve and vector | Verified batch ingest of other nodes' entries plus the committed-range read side (spec §6 Ingest; §5 serve, vector, stored_hash, store_forked, freeze; §3 Skew). |
| ht-5fr.3.6 | task | Journal open and local authoring with inline apply: SyncState, author/author_above, commit | Every local fact is authored, signed, chained and applied in the command's own transaction (spec §5 Journal: open, author; §3 SyncState/HLC resume; §7 apply_one |
| ht-5fr.3.5 | task | FactKind/FactRegistry and ApplyContext contract seam | Land the fact-kind contract and the handler context type, compilable and inert, so Journal authoring, ApplyEngine and ht-5fr.4's kinds can be written against th |
| ht-5fr.3.4 | task | Journal-core baseline DDL: sync_logs, journal, journal_forked, thread_registers and guard  | Add the journal-core tables, indexes and triggers to the node baseline and classify them for kicks (spec §4 Tables (baseline DDL), Triggers, Kicks classificatio |
| ht-5fr.3.3 | task | Ingest contract: ChainRules (src/sync/chain.rs) and JournalPort trait with outcome types | The two pure contracts both JournalPort implementations and ht-5fr.5 build on, landed inert (spec §6 decision table; §12 JournalPort; §1 rows src/sync/chain.rs  |
| ht-5fr.3.2 | task | Envelope v1 and sync value types: LogKey/LogHead/Vector, EntityKey, Deps, WireEnvelope | The signed journal-entry form and the leaf value types every other journal piece uses (spec §2 Envelope: canonical bytes, deps, wire form; §1 module layout rows |
| ht-5fr.3.1 | task | Hlc: hybrid logical clock (src/sync/hlc.rs) | Create the `herdr_threads::sync` module and its HLC (spec §3 HLC; §1 module layout row src/sync/hlc.rs). |
| ht-5fr.1.11 | task | Node layout integration suite, health hosts count and doctor store-only label | Make the store-only daemon visible and prove the node layout end to end: health `hosts: 0`, the doctor `store-only` label, the test_support hooks they need, and |
| ht-5fr.1.10 | task | Seat home-node delta, node_directory API and daemon identity start check | Apply the seats part of the node delta (home node, host, remote state), add the `store::node_directory` API, and complete the daemon start identity check (spec  |
| ht-5fr.1.9 | task | Doctor legacy report and legacy purge (LegacyInventory) | Doctor legacy report and operator-gated `herdr-threads legacy purge`, both through one read-only LegacyInventory (spec §9 Legacy instances/: doctor report and l |
| ht-5fr.1.8 | task | Delete legacy migration chain code | Delete the legacy 46-step migration chain code and the transient baseline-equivalence test (spec §5 Delete the legacy chain). |
| ht-5fr.1.7 | task | Store-only daemon and primary-host attach via HostChoice | Store-only daemon and primary-host attach through `HostChoice`: the elected daemon chooses zero or one Herdr server after the store opens, spawns host workers o |
| ht-5fr.1.6 | task | Tagged public IDs and pure direct-thread ID function | Tagged public IDs `<prefix><tag><8>` minted from `local_node.tag` in the caller's transaction, plus the pure `d…` direct-thread ID function (spec §7 Public IDs  |
| ht-5fr.1.5 | task | Node row delta: local_node, herdr_hosts, initialize(NodeSeed), primary-host accessor and q | Apply the node-row part of the node delta (`host_instances` → `local_node` + `herdr_hosts`, the `node_directory` table) to the baseline, change `initialize` to  |
| ht-5fr.1.4 | task | Squash schema into verified node baseline (migrations/node/0001) and drop upgrade-path tes | Squash today's 46-step schema into one verbatim baseline `migrations/node/0001_node_baseline.sql`, switch `schema::initialize` to it with `verify_baseline`, and |
| ht-5fr.1.3 | task | Optional claim host: RuntimeContext.require_host, daemon run argv, primary_host guard | Make the Herdr host optional on the client side and in daemon argv: `RuntimeContext.host_endpoint: Option`, `require_host()`, `daemon run --state-dir <dir> [--h |
| ht-5fr.1.2 | task | NodePaths: resolve node layout by state dir only; descriptor node_uuid | Rename `InstancePaths` → `NodePaths` and resolve every node path from the state dir only; the daemon loads `NodeIdentity` and the descriptor carries `node_uuid` |
| ht-5fr.1.1 | task | Node identity module: NodeUuid/NodeTag, NodeIdentity key custody, node.json | Deliver the new core module `src/node/` (`mod.rs`, `identity.rs`, `tag.rs`, `json.rs`, exported as `herdr_threads::node`) plus the crypto dependencies (spec §1  |
| ht-5fr.13 | task | Sync-aware deadline hold and unreachable-node marker | Spec §7 (overdue hold at daemon start and after sleep) and §8 (unreachable marker). |
| ht-5fr.12 | task | Agent and user docs for cross-node sync | Update the agent skill (integrations/skill/SKILL.md: remote seats, node tag in IDs, unreachable-node marker; keep per-message framing unchanged) and user docs ( |
| ht-5fr.11 | task | Configuration smoke: 2-node direct, 3-node relay, partition+heal, restore fork, mixed vers | Spec §13 configurations. |
| ht-5fr.10 | task | Multi-node test harness | Spec §13. |
| ht-5fr.9 | task | TRUST-POLICY amendment for cross-node sync | Spec §12. |
| ht-5fr.8 | task | Operator and status CLI for sync | Spec §11 CLI. |
| ht-5fr.7 | task | Peer discovery worker | Spec §11 discovery. |
| ht-5fr.6 | epic | Forks, restore and incarnations | Spec §10. |
| ht-5fr.5 | epic | Peer link and sync coordinator | Spec §9. |
| ht-5fr.4 | epic | Fact kinds and write-path conversion | Spec §6, §7, §8. |
| ht-5fr.3 | epic | Journal core: envelope, HLC, journal, apply engine, fact registry | Spec §4, §5. |
| ht-5fr.2 | epic | Multi-host daemon: HostSessions | Spec §3. |
| ht-5fr.1 | epic | Node identity, layout and schema baseline | Spec §1, §2, §3 (schema part), §8 (seats columns). |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-5fr.54 | ht-5fr.53 | same hold code (src/sync/signals.rs, src/scheduler/deadlines.rs). |
| ht-5fr.37 | ht-5fr.34 | consumes remote-seat threshold refusal; blocked-by ht-5fr.36: consumes oracle priority fields. |
| ht-5fr.37 | ht-5fr.36 | consumes oracle priority fields. |
| ht-5fr.37 | ht-5fr.31 | unstated |
| ht-5fr.37 | ht-5fr.32 | unstated |
| ht-5fr.33 | ht-5fr.31 | consumes resolve_direct_pair's auto-join. |
| ht-5fr.32 | ht-5fr.31 | consumes auto_join_local_pair_seats and the auto-join join path. |
| ht-5fr.4.40 | ht-5fr.4.38 | consumes ThreadRegisterPayload.actor |
| ht-5fr.3.37 | ht-5fr.25 | the loop_inventory acceptance test fails until ht-5fr.24 (Pacer fix, commit 60462da84) merges, and ht-5fr.25's close is  |
| ht-5fr.18 | ht-5fr.7 | consumes process_table::scan/remote_targets and the discovery worker |
| ht-5fr.59 | ht-5fr.53 | unstated |
| ht-5fr.52 | ht-5fr.32 | unstated |
| ht-5fr.50 | ht-5fr.31 | unstated |
| ht-5fr.48 | ht-5fr.36 | unstated |
| ht-5fr.47 | ht-5fr.34 | unstated |
| ht-5fr.4.42 | ht-5fr.4.40 | unstated |
| ht-5fr.4.41 | ht-5fr.4.37 | unstated |
| ht-5fr.4.39 | ht-5fr.4.38 | unstated |
| ht-5fr.5.30 | ht-5fr.5.29 | unstated |
| ht-5fr.30 | ht-5fr.11 | unstated |
| ht-5fr.6.34 | ht-5fr.6.11 | unstated |
| ht-5fr.4.36 | ht-5fr.4.18 | unstated |
| ht-5fr.29 | ht-5fr.12 | unstated |
| ht-5fr.28 | ht-5fr.13 | unstated |
| ht-5fr.4.35 | ht-5fr.4.13 | unstated |
| ht-5fr.4.34 | ht-5fr.4.12 | unstated |
| ht-5fr.4.33 | ht-5fr.4.10 | unstated |
| ht-5fr.4.32 | ht-5fr.4.16 | unstated |
| ht-5fr.4.31 | ht-5fr.4.11 | unstated |
| ht-5fr.4.30 | ht-5fr.4.9 | unstated |
| ht-5fr.4.29 | ht-5fr.4.14 | unstated |
| ht-5fr.4.28 | ht-5fr.4.8 | unstated |
| ht-5fr.4.27 | ht-5fr.4.7 | unstated |
| ht-5fr.2.26 | ht-5fr.2.25 | unstated |
| ht-5fr.4.26 | ht-5fr.4.5 | unstated |
| ht-5fr.4.25 | ht-5fr.4.17 | unstated |
| ht-5fr.4.24 | ht-5fr.4.6 | unstated |
| ht-5fr.4.23 | ht-5fr.4.4 | unstated |
| ht-5fr.4.22 | ht-5fr.4.2 | unstated |
| ht-5fr.27 | ht-5fr.23 | unstated |
| ht-5fr.26 | ht-5fr.22 | unstated |
| ht-5fr.6.33 | ht-5fr.6.30 | unstated |
| ht-5fr.25 | ht-5fr.24 | unstated |
| ht-5fr.5.28 | ht-5fr.5.27 | unstated |
| ht-5fr.6.32 | ht-5fr.6.31 | unstated |
| ht-5fr.3.38 | ht-5fr.3.37 | unstated |
| ht-5fr.21 | ht-5fr.18 | unstated |
| ht-5fr.2.24 | ht-5fr.2.10 | unstated |
| ht-5fr.2.23 | ht-5fr.2.12 | unstated |
| ht-5fr.20 | ht-5fr.14 | unstated |
| ht-5fr.19 | ht-5fr.8 | unstated |
| ht-5fr.6.29 | ht-5fr.6.14 | unstated |
| ht-5fr.17 | ht-5fr.7 | unstated |
| ht-5fr.2.22 | ht-5fr.2.9 | unstated |
| ht-5fr.6.28 | ht-5fr.6.15 | unstated |
| ht-5fr.6.27 | ht-5fr.6.10 | unstated |
| ht-5fr.5.26 | ht-5fr.5.11 | unstated |
| ht-5fr.16 | ht-5fr.10 | unstated |
| ht-5fr.5.25 | ht-5fr.5.10 | unstated |
| ht-5fr.6.26 | ht-5fr.6.13 | unstated |
| ht-5fr.2.21 | ht-5fr.2.8 | unstated |
| ht-5fr.6.25 | ht-5fr.6.12 | unstated |
| ht-5fr.6.24 | ht-5fr.6.7 | unstated |
| ht-5fr.5.24 | ht-5fr.5.6 | unstated |
| ht-5fr.2.20 | ht-5fr.2.6 | unstated |
| ht-5fr.6.23 | ht-5fr.6.8 | unstated |
| ht-5fr.6.22 | ht-5fr.6.6 | unstated |
| ht-5fr.5.23 | ht-5fr.5.13 | unstated |
| ht-5fr.6.21 | ht-5fr.6.5 | unstated |
| ht-5fr.6.20 | ht-5fr.6.9 | unstated |
| ht-5fr.4.21 | ht-5fr.4.15 | unstated |
| ht-5fr.5.22 | ht-5fr.5.12 | unstated |
| ht-5fr.4.20 | ht-5fr.4.3 | unstated |
| ht-5fr.2.19 | ht-5fr.2.7 | unstated |
| ht-5fr.3.36 | ht-5fr.3.16 | unstated |
| ht-5fr.3.35 | ht-5fr.3.21 | unstated |
| ht-5fr.3.34 | ht-5fr.3.32 | unstated |
| ht-5fr.2.18 | ht-5fr.2.5 | unstated |
| ht-5fr.5.21 | ht-5fr.5.7 | unstated |
| ht-5fr.6.19 | ht-5fr.6.2 | unstated |
| ht-5fr.5.20 | ht-5fr.5.8 | unstated |
| ht-5fr.2.17 | ht-5fr.2.2 | unstated |
| ht-5fr.5.19 | ht-5fr.5.5 | unstated |
| ht-5fr.3.33 | ht-5fr.3.13 | unstated |
| ht-5fr.6.18 | ht-5fr.6.3 | unstated |
| ht-5fr.2.16 | ht-5fr.2.4 | unstated |
| ht-5fr.5.18 | ht-5fr.5.4 | unstated |
| ht-5fr.3.31 | ht-5fr.3.14 | unstated |
| ht-5fr.3.30 | ht-5fr.3.10 | unstated |
| ht-5fr.3.29 | ht-5fr.3.9 | unstated |
| ht-5fr.1.25 | ht-5fr.1.13 | unstated |
| ht-5fr.3.28 | ht-5fr.3.8 | unstated |
| ht-5fr.2.15 | ht-5fr.2.3 | unstated |
| ht-5fr.1.24 | ht-5fr.1.8 | unstated |
| ht-5fr.5.17 | ht-5fr.5.3 | unstated |
| ht-5fr.6.17 | ht-5fr.6.4 | unstated |
| ht-5fr.3.27 | ht-5fr.3.15 | unstated |
| ht-5fr.3.26 | ht-5fr.3.12 | unstated |
| ht-5fr.3.25 | ht-5fr.3.7 | unstated |
| ht-5fr.1.23 | ht-5fr.1.9 | unstated |
| ht-5fr.6.16 | ht-5fr.6.1 | unstated |
| ht-5fr.1.22 | ht-5fr.1.11 | unstated |
| ht-5fr.3.24 | ht-5fr.3.6 | unstated |
| ht-5fr.2.14 | ht-5fr.2.11 | unstated |
| ht-5fr.5.16 | ht-5fr.5.2 | unstated |
| ht-5fr.5.15 | ht-5fr.5.9 | unstated |
| ht-5fr.1.21 | ht-5fr.1.12 | unstated |
| ht-5fr.1.20 | ht-5fr.1.10 | unstated |
| ht-5fr.1.19 | ht-5fr.1.7 | unstated |
| ht-5fr.2.13 | ht-5fr.2.1 | unstated |
| ht-5fr.1.18 | ht-5fr.1.6 | unstated |
| ht-5fr.3.23 | ht-5fr.3.11 | unstated |
| ht-5fr.4.19 | ht-5fr.4.1 | unstated |
| ht-5fr.3.22 | ht-5fr.3.4 | unstated |
| ht-5fr.1.17 | ht-5fr.1.5 | unstated |
| ht-5fr.1.16 | ht-5fr.1.3 | unstated |
| ht-5fr.5.14 | ht-5fr.5.1 | unstated |
| ht-5fr.3.20 | ht-5fr.3.3 | unstated |
| ht-5fr.3.19 | ht-5fr.3.5 | unstated |
| ht-5fr.3.18 | ht-5fr.3.2 | unstated |
| ht-5fr.3.17 | ht-5fr.3.1 | unstated |
| ht-5fr.1.15 | ht-5fr.1.2 | unstated |
| ht-5fr.1.14 | ht-5fr.1.4 | unstated |
| ht-5fr.15 | ht-5fr.4.18 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.9 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.23 | unstated |
| ht-5fr.15 | ht-5fr.4.7 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.2 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.13 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.13 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.3 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.32 | unstated |
| ht-5fr.15 | ht-5fr.2.9 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.8 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.9 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.10 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.16 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.5 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.37 | unstated |
| ht-5fr.15 | ht-5fr.2.2 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.1 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.27 | unstated |
| ht-5fr.15 | ht-5fr.6.3 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.12 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.10 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.14 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.4 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.3 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.6 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.12 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.12 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.7 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.7 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.10 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.37 | unstated |
| ht-5fr.15 | ht-5fr.4.40 | unstated |
| ht-5fr.15 | ht-5fr.5.2 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.8 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.22 | unstated |
| ht-5fr.15 | ht-5fr.4.11 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.13 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.7 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.2 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.1 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.12 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.7 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.8 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.3 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.18 | unstated |
| ht-5fr.15 | ht-5fr.1.3 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.11 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.4 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.7 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.12 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.1 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.14 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.24 | unstated |
| ht-5fr.15 | ht-5fr.3.2 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.9 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.15 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.4 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.13 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.6 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.6 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.8 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.8 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.9 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.13 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.4 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.1 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.9 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.10 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.9 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.5 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.2 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.6 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.8 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.5 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.1 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.12 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.14 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.13 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.4 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.10 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.7 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.11 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.14 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.5 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.16 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.31 | unstated |
| ht-5fr.15 | ht-5fr.4.38 | unstated |
| ht-5fr.15 | ht-5fr.1.10 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.4 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.15 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.15 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.11 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.8 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.30 | unstated |
| ht-5fr.15 | ht-5fr.2.25 | unstated |
| ht-5fr.15 | ht-5fr.6.11 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.1 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.29 | unstated |
| ht-5fr.15 | ht-5fr.4.10 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.6.5 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.3 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.12 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.2.5 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.4.6 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.11 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.3.21 | unstated |
| ht-5fr.15 | ht-5fr.4.17 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.1.6 | consumes all leaves (integration sweep) |
| ht-5fr.15 | ht-5fr.5.11 | consumes all leaves (integration sweep) |
| ht-5fr.14 | ht-5fr.5.7 | consumes TransportCommand::build default ssh argv, default remote command and BatchMode classification |
| ht-5fr.14 | ht-5fr.5.10 | consumes the sync runtime composed into the daemon and peer --stdio |
| ht-5fr.14 | ht-5fr.8 | consumes the peer add [--remote-command] CLI that creates the explicit ssh target |
| ht-5fr.1.13 | ht-5fr.1.8 | consumes src/store/schema.rs with the legacy migration chain deleted (shared file) |
| ht-5fr.1.13 | ht-5fr.1.4 | consumes NODE_BASELINE, verify_baseline and IncompatibleSchema |
| ht-5fr.1.12 | ht-5fr.1.6 | consumes tagged public IDs (is_public_id, public_id_parts) and direct_thread_id/is_direct_thread_id in protocol::ids |
| ht-5fr.6.15 | ht-5fr.6.10 | shared src/app.rs (resync step 0 lands first) |
| ht-5fr.6.15 | ht-5fr.5.10 | consumes coordinator composition in run_elected_impl (CoordinatorDeps.fork_sink replaced there) |
| ht-5fr.6.15 | ht-5fr.6.13 | consumes IncarnationsWorker (with IncarnationsSink::start) |
| ht-5fr.6.14 | ht-5fr.6.15 | consumes incarnations composition in src/app.rs (shared src/app.rs) |
| ht-5fr.6.14 | ht-5fr.6.8 | shared src/sync/coordinator.rs; consumes reselect with the fork wiring in place |
| ht-5fr.6.14 | ht-5fr.9 | consumes TRUST-POLICY repair section, next to which accepted limit 3 and the resync entry are added |
| ht-5fr.6.14 | ht-5fr.6.13 | consumes the IncarnationsWorker job loop that runs Job::Salvage (shared src/sync/incarnations.rs) |
| ht-5fr.6.14 | ht-5fr.6.10 | consumes ResyncMarker, ResyncPhase, ResyncPlan, resume_at_start and the resync.after_moved failpoint in src/sync/resync. |
| ht-5fr.6.13 | ht-5fr.6.7 | consumes owner_rotate, restore_rotate, upsert_branch_tip, branch_progress and due_branches |
| ht-5fr.6.13 | ht-5fr.6.6 | consumes reissue_pass |
| ht-5fr.6.13 | ht-5fr.5.6 | after clear_clone_evidence the IncarnationsWorker calls SyncHandle::signals().request_round(now); SyncSignals::request_r |
| ht-5fr.6.13 | ht-5fr.6.12 | consumes record_own_unknown_newer (clone evidence for own_unknown_newer logs) |
| ht-5fr.6.13 | ht-5fr.6.9 | consumes IncarnationsSink, Job, JobQueue, RequestTable and the shared state in src/sync/incarnations.rs |
| ht-5fr.6.12 | ht-5fr.6.4 | consumes the store::incarnations module, next_incarnation and the sync_forks/sync_branches/sync_reissues tables |
| ht-5fr.6.12 | ht-5fr.6.7 | shared src/store/incarnations.rs and tests/store/incarnations.rs; consumes restore_successor and the lineage_branch evid |
| ht-5fr.6.11 | ht-5fr.6.3 | consumes JournalPort::serve_branch on SqliteStore and MemoryJournal |
| ht-5fr.6.11 | ht-5fr.4.4 | consumes journal_oracle::replicated_projection with the converted tables |
| ht-5fr.6.11 | ht-5fr.6.8 | consumes coordinator fork routing (on_have, branch_* routing, branch_want serving, outbox drain, fork refs) that the sce |
| ht-5fr.6.11 | ht-5fr.4.12 | consumes message.sent/receipt.settled kinds for the settled-history and open-obligation assertions |
| ht-5fr.6.11 | ht-5fr.6.13 | consumes IncarnationsSink/IncarnationsWorker (sink front via ht-5fr.6.9) |
| ht-5fr.6.11 | ht-5fr.6.12 | consumes adopt_declared_freeze on node.genesis apply (D freezing from A's genesis) and the next_incarnation bootstrap (w |
| ht-5fr.6.10 | ht-5fr.5.10 | consumes SyncHandle, DialTargets and run_elected_impl with the sync runtime composed (step 0 goes before it) |
| ht-5fr.6.9 | ht-5fr.6.1 | consumes ForkProof classify_ahead_of_self for the synchronous on_fork check |
| ht-5fr.6.9 | ht-5fr.6.2 | consumes ForkSink extension, ForkRef, BranchWant, BranchReply and ForkOutbox |
| ht-5fr.6.9 | ht-5fr.5.12 | consumes ForkSink::on_clone and CloneReport |
| ht-5fr.6.9 | ht-5fr.6.3 | consumes JournalPort::log_meta (and stored_hash) for pubkey and own-head reads |
| ht-5fr.6.8 | ht-5fr.6.3 | consumes JournalPort::serve_branch on both implementations |
| ht-5fr.6.8 | ht-5fr.6.2 | consumes fork.v1 frames, branch_entries credit and the ForkSink extension |
| ht-5fr.6.8 | ht-5fr.5.13 | consumes coordinator serving path, ingest outcomes and Have deltas (branch serving and refs deltas extend them) |
| ht-5fr.6.8 | ht-5fr.5.12 | consumes coordinator Have merge and routing (on_have hooks in after the merge) |
| ht-5fr.6.7 | ht-5fr.6.4 | consumes store::incarnations module, record_fork, next_incarnation and the sync_branches table |
| ht-5fr.6.7 | ht-5fr.9 | consumes TRUST-POLICY fork quarantine amendment, next to which accepted limit 1 is added |
| ht-5fr.6.7 | ht-5fr.6.6 | shared src/store/incarnations.rs, tests/store/incarnations.rs and TRUST-POLICY.md (lands after the re-issue pass) |
| ht-5fr.6.6 | ht-5fr.6.5 | consumes journal::reissue and ApplyMode::Reissue |
| ht-5fr.6.6 | ht-5fr.6.4 | consumes store::incarnations module and the sync_reissues/sync_forks tables |
| ht-5fr.6.6 | ht-5fr.9 | consumes TRUST-POLICY fork quarantine/re-issue amendment, next to which accepted limits 2 and 4 are added |
| ht-5fr.6.5 | ht-5fr.4.15 | consumes Attribution take (and the MaterializeInput helper from ht-5fr.4.3) that is_own_history feeds |
| ht-5fr.6.5 | ht-5fr.6.3 | shared src/store/journal.rs (serve_branch/log_meta land first) |
| ht-5fr.6.5 | ht-5fr.3.16 | consumes ApplyContext::delivery_suppressed/provenance and the apply engine it extends |
| ht-5fr.6.4 | ht-5fr.3.7 | consumes store_forked and the lower-only, active-refusing freeze |
| ht-5fr.6.4 | ht-5fr.3.6 | consumes SyncState.incarnation_rng (and begin_incarnation(Predecessor) for test setup) |
| ht-5fr.6.4 | ht-5fr.6.1 | consumes Proof (ForkProof) for record_fork |
| ht-5fr.6.3 | ht-5fr.3.8 | consumes impl JournalPort for SqliteStore and journal serve/store_forked |
| ht-5fr.6.3 | ht-5fr.3.9 | consumes MemoryJournal and tests/sync/journal_port_contract.rs |
| ht-5fr.6.2 | ht-5fr.5.1 | consumes PeerFrame, Have/HaveLog in src/sync/frame.rs (extended with the fork variants) |
| ht-5fr.6.2 | ht-5fr.5.2 | consumes link.rs data queue and credit window (branch_entries joins it) |
| ht-5fr.6.2 | ht-5fr.5.5 | consumes ForkSink trait (with ForkAck) and LogOnlyForkSink in src/sync/coordinator.rs |
| ht-5fr.6.1 | ht-5fr.3.2 | consumes Envelope seal/verify, EnvelopeError and LogKey in src/sync/ |
| ht-5fr.6.1 | ht-5fr.3.6 | consumes NodeIdentity::from_seed for the classification tests |
| ht-5fr.4.18 | ht-5fr.4.12 | consumes receipt.settled (late ACK overdue) |
| ht-5fr.4.18 | ht-5fr.4.10 | consumes invitation.created/answered (remote invite, late answer overdue, invitation due scope) |
| ht-5fr.4.18 | ht-5fr.4.7 | consumes seat.retired and replicated retirement (retirement x send, archival after seat.retired) |
| ht-5fr.4.18 | ht-5fr.4.11 | consumes message.sent author build and local-adopt branch |
| ht-5fr.4.18 | ht-5fr.4.14 | consumes archival remote-member rule and remote seats in workers |
| ht-5fr.4.18 | ht-5fr.4.17 | consumes send-fence narrowing (fence across nodes) |
| ht-5fr.4.18 | ht-5fr.4.13 | consumes deadline lane scope (sender-only overdue) |
| ht-5fr.4.18 | ht-5fr.4.16 | consumes message.sent synthesized branch (sends to a remote seat, message into an archived thread) |
| ht-5fr.4.17 | ht-5fr.4.6 | consumes DeliveryTail extraction in src/store/messages.rs (shared file ordering for publish_send) |
| ht-5fr.4.16 | ht-5fr.4.7 | consumes remote recipient eligibility (eligible_delivery_recipient, walk_audience) |
| ht-5fr.4.16 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.16 | ht-5fr.4.11 | consumes MessageSent, materialize_message local branch and the insert_publication/PublicationAuthor refactor |
| ht-5fr.4.16 | ht-5fr.4.9 | consumes membership.joined for a remote seat (B joined in the convergence test) |
| ht-5fr.4.15 | ht-5fr.4.3 | consumes MaterializeInput (src/store/materialize.rs), the input `take` reads origin and provenance from |
| ht-5fr.4.15 | ht-5fr.9 | consumes TRUST-POLICY replicated_from:<node_uuid> section, next to which this leaf adds local_journal |
| ht-5fr.4.15 | ht-5fr.3.15 | consumes seeded test constructor (distinct node identities) used by test_support::fact_exchange |
| ht-5fr.4.14 | ht-5fr.4.7 | consumes seat.retired remote-row semantics (state='retired') that the archival non-retired-member rule reads |
| ht-5fr.4.14 | ht-5fr.2.9 | consumes per-host scan_members gate in src/store/archival.rs where the remote-member rule goes |
| ht-5fr.4.14 | ht-5fr.2.4 | consumes host-scoped observation/reconciliation queries the remote-seat regression tests cover |
| ht-5fr.4.14 | ht-5fr.4.8 | consumes archival thread.register authoring; shared archival.rs |
| ht-5fr.4.13 | ht-5fr.4.2 | consumes messages.origin_node / invitations.origin_node columns and invitations_due_origin index |
| ht-5fr.4.13 | ht-5fr.4.6 | consumes DeliveryTail::attention as the warning-recipient writer in src/store/materialization.rs |
| ht-5fr.4.13 | ht-5fr.4.12 | consumes materialize_settlement's late-overdue call in src/store/receipts.rs (shared file) |
| ht-5fr.4.13 | ht-5fr.4.16 | consumes the message materializer's edits to src/store/materialization.rs (project_receipt; shared file) |
| ht-5fr.4.12 | ht-5fr.3.16 | consumes the own-history hold (OwnHistoryHold in SyncState) for the foreign-first wipe-then-pull test |
| ht-5fr.4.12 | ht-5fr.4.16 | consumes message.sent applied through the synthesized branch on the receiving node (receipt.settled needs m…; cross-node |
| ht-5fr.4.12 | ht-5fr.4.15 | consumes Attribution (ack:<message>/<seat> staging and take) and test_support::fact_exchange |
| ht-5fr.4.12 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.11 | ht-5fr.4.6 | consumes DeliveryTail::message |
| ht-5fr.4.11 | ht-5fr.4.8 | consumes thread.created (message.sent needs a journaled thread) |
| ht-5fr.4.11 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.11 | ht-5fr.4.15 | consumes Attribution (send:<message> staging and take) |
| ht-5fr.4.10 | ht-5fr.4.6 | consumes DeliveryTail::invitation |
| ht-5fr.4.10 | ht-5fr.4.7 | consumes remote eligibility in invite/resolve_direct_pair |
| ht-5fr.4.10 | ht-5fr.4.1 | consumes file split |
| ht-5fr.4.10 | ht-5fr.4.15 | consumes Attribution (answer:<invitation> staging and take) and test_support::fact_exchange |
| ht-5fr.4.10 | ht-5fr.4.9 | consumes membership joined transition and settle_superseded_invitations |
| ht-5fr.4.10 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.9 | ht-5fr.4.1 | consumes file split |
| ht-5fr.4.9 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.9 | ht-5fr.4.15 | consumes Attribution (join:<thread>/<seat>/<n> staging and take) and test_support::fact_exchange |
| ht-5fr.4.9 | ht-5fr.4.8 | consumes thread.created and the converted create_thread* (membership facts need a journaled thread) |
| ht-5fr.4.8 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.8 | ht-5fr.4.1 | consumes file split |
| ht-5fr.4.8 | ht-5fr.4.5 | consumes FactRoute emit and seat publication (src/store/materialize.rs) |
| ht-5fr.4.8 | ht-5fr.3.11 | consumes ThreadRegisters upsert/current (src/store/thread_registers.rs) |
| ht-5fr.4.8 | ht-5fr.1.6 | consumes direct_thread_id function in src/protocol/ids.rs |
| ht-5fr.4.7 | ht-5fr.4.1 | consumes file split |
| ht-5fr.4.7 | ht-5fr.4.6 | consumes DeliveryTail extraction in control/invitation.rs, control/direct.rs, messages.rs (shared files) |
| ht-5fr.4.7 | ht-5fr.4.5 | consumes seat.created/seats.published and the FactRoute emit |
| ht-5fr.4.7 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.6 | ht-5fr.4.3 | consumes MaterializeInput (DeliveryTail::message signature) |
| ht-5fr.4.6 | ht-5fr.4.1 | consumes file split |
| ht-5fr.4.6 | ht-5fr.4.2 | consumes send_manifests.suppressed_seats column |
| ht-5fr.4.5 | ht-5fr.4.3 | consumes MaterializeInput/local_only, payload referenced_seats and seat_key, and the src/sync/facts/seat.rs module |
| ht-5fr.4.5 | ht-5fr.4.2 | consumes seats.published and threads.journaled columns |
| ht-5fr.4.5 | ht-5fr.3.16 | consumes OwnHistoryHold::active (the restore-window authoring hold's signal) |
| ht-5fr.4.5 | ht-5fr.4.4 | consumes WriteGuard armed for converted tables |
| ht-5fr.4.5 | ht-5fr.4.15 | consumes test_support::fact_exchange (two-store convergence in tests/store/fact_convergence_seat.rs) |
| ht-5fr.4.4 | ht-5fr.4.3 | consumes materialize::local_only scope (src/store/materialize.rs) |
| ht-5fr.4.4 | ht-5fr.4.2 | consumes new replicated columns (journaled, home_transition, origin_node, settle_batch, created_hlc/created_node) |
| ht-5fr.4.4 | ht-5fr.3.14 | consumes test_support::journal_oracle::replicated_projection |
| ht-5fr.4.3 | ht-5fr.3.16 | consumes ApplyContext services (provenance, is_local_origin, delivery_suppressed) that MaterializeInput is built from |
| ht-5fr.4.2 | ht-5fr.1.10 | consumes seats delta (`is_local`, `home_node`, `remote_kind`, state `remote`) in migrations/node/0001_node_baseline.sql |
| ht-5fr.4.2 | ht-5fr.4.1 | consumes file split |
| ht-5fr.4.1 | ht-5fr.1.5 | consumes src/store/control.rs query redirection (file order) |
| ht-5fr.5.13 | ht-5fr.5.5 | consumes SyncCoordinator actor skeleton, CoordinatorDeps and the ForkSink trait/LogOnlyForkSink in src/sync/coordinator. |
| ht-5fr.5.13 | ht-5fr.5.3 | consumes Lane::Sync (enter_lane, pacer kicks, LaneStatus failures) |
| ht-5fr.5.13 | ht-5fr.3.9 | consumes MemoryJournal (JournalPort fake with author_raw) for coordinator tests, and JournalPort/IngestOutcome/ForkEvide |
| ht-5fr.5.13 | ht-5fr.5.12 | consumes rx subscriptions, reselect, penalty/pause bookkeeping and the active-link set (entries are accepted only on a r |
| ht-5fr.5.12 | ht-5fr.3.16 | consumes OwnHistoryHold (SyncState) for the own-history Links flag |
| ht-5fr.5.12 | ht-5fr.3.9 | consumes MemoryJournal (JournalPort fake) for routing tests |
| ht-5fr.5.12 | ht-5fr.5.5 | consumes SyncCoordinator actor skeleton, CoordinatorDeps, link state and SyncHandle in src/sync/coordinator.rs |
| ht-5fr.5.12 | ht-5fr.5.2 | consumes PeerLink LinkHandle, LinkEvent and PeerInfo (role, hello nonce) for the duplicate order |
| ht-5fr.5.11 | ht-5fr.5.10 | consumes composed SyncHandle and ConnectionGauge in run_elected_impl |
| ht-5fr.5.11 | ht-5fr.5.7 | consumes SyncHandle::enabled_target_count() (the Dialer's enabled-target count) |
| ht-5fr.5.10 | ht-5fr.3.8 | consumes impl JournalPort for SqliteStore |
| ht-5fr.5.10 | ht-5fr.3.13 | consumes Apply lane worker composed in run_elected_impl |
| ht-5fr.5.10 | ht-5fr.5.8 | consumes serve_with_gate peers/gauge parameters and the hello sniff |
| ht-5fr.5.10 | ht-5fr.5.6 | consumes SyncSignals, CoordinatorDeps.sync_store and the contact flush |
| ht-5fr.5.10 | ht-5fr.2.8 | consumes HostSessions composed in run_elected_impl (composition order) |
| ht-5fr.5.10 | ht-5fr.5.12 | consumes coordinator routing (Have/Want, rx reselect, duplicate standby) that the two-daemon convergence test exercises |
| ht-5fr.5.10 | ht-5fr.5.9 | consumes peer --stdio pump (two-daemon tests dial through it) |
| ht-5fr.5.10 | ht-5fr.5.13 | consumes Want serving, live tail, the ingest worker and Have deltas that the two-daemon convergence test exercises |
| ht-5fr.5.10 | ht-5fr.5.7 | consumes Dialer inside the actor, DialConfig and the sync.dial_command setting |
| ht-5fr.5.10 | ht-5fr.3.15 | consumes journal::open call in run_elected_impl (composition point) |
| ht-5fr.5.9 | ht-5fr.5.1 | consumes PREAMBLE constant in src/sync/frame.rs |
| ht-5fr.5.9 | ht-5fr.1.7 | consumes store-only daemon start through ensure_running with an optional host hint |
| ht-5fr.5.8 | ht-5fr.5.5 | consumes PeerIntake and PeerStream in src/sync/coordinator.rs |
| ht-5fr.5.7 | ht-5fr.5.5 | consumes coordinator actor DialedStream entry point with its DialReport channel (Established, LinkEnded), CoordinatorDep |
| ht-5fr.5.7 | ht-5fr.5.4 | consumes SyncStorePort/MemorySyncStore and DialTransition |
| ht-5fr.5.6 | ht-5fr.5.12 | consumes link activation, full Have/ask handling and the wantable rule (round start and caught-up/unwantable completion) |
| ht-5fr.5.6 | ht-5fr.5.13 | consumes ingest Stored outcomes and rx progress (contacts on Stored, round completion on catch-up) |
| ht-5fr.5.6 | ht-5fr.5.4 | consumes ContactBook, SyncStorePort/MemorySyncStore and store::sync_contacts |
| ht-5fr.5.6 | ht-5fr.5.5 | consumes SyncCoordinator actor, CoordinatorDeps (with the sync_store field) and SyncHandle in src/sync/coordinator.rs |
| ht-5fr.5.5 | ht-5fr.5.2 | consumes PeerLink handshake/spawn, LinkHandle, LinkEvent and LinkError in src/sync/link.rs |
| ht-5fr.5.5 | ht-5fr.3.9 | consumes MemoryJournal (JournalPort fake) for coordinator tests, and JournalPort/IngestOutcome/ForkEvidence from ht-5fr. |
| ht-5fr.5.5 | ht-5fr.5.3 | consumes Lane::Sync (pacer in CoordinatorDeps, LaneStatus) |
| ht-5fr.5.5 | ht-5fr.5.4 | consumes SyncStorePort trait and MemorySyncStore for the CoordinatorDeps.sync_store field |
| ht-5fr.5.4 | ht-5fr.5.3 | consumes Lane::Sync for the sync_dial_targets kick mapping |
| ht-5fr.5.4 | ht-5fr.5.1 | consumes CloseReason/CloseClass (stopped_reason, DialTransition::Stopped) and SeenAge (ContactBook Seen ages) |
| ht-5fr.5.4 | ht-5fr.1.4 | consumes migrations/node/0001_node_baseline.sql and verify_baseline |
| ht-5fr.5.3 | ht-5fr.3.12 | consumes Lane::Apply, LaneSet u16 and kicks_for_commit in src/service/kicks.rs |
| ht-5fr.5.2 | ht-5fr.5.1 | consumes PeerFrame, PeerHello, CloseReason and preamble constants in src/sync/frame.rs |
| ht-5fr.5.2 | ht-5fr.1.10 | consumes NodeDirectory (store::node_directory) as the source of the known genesis pubkey per node_uuid for the handshake |
| ht-5fr.5.1 | ht-5fr.3.2 | consumes WireEnvelope and LogKey in src/sync/ (and NodeUuid/NodeTag from ht-5fr.1.1 through it) |
| ht-5fr.2.12 | ht-5fr.2.8 | consumes composed Arc<HostSessions>, fake-locator HostFactory and LaneProbe::host_pacer |
| ht-5fr.2.12 | ht-5fr.2.3 | consumes Command::host_locator and command host |
| ht-5fr.2.12 | ht-5fr.2.6 | consumes cooperative_claim and C1 continuity by command host |
| ht-5fr.2.12 | ht-5fr.1.3 | consumes interim single-host guard and EndpointDescriptor.primary_host (deleted here) |
| ht-5fr.2.12 | ht-5fr.1.11 | consumes tests/daemon/node_layout.rs interim-guard case (rewritten here) |
| ht-5fr.2.11 | ht-5fr.1.10 | consumes seats host_id/is_local columns and LOCAL_SEAT |
| ht-5fr.3.16 | ht-5fr.3.13 | OwnHistoryHold must set SyncState.apply_hint and kick Lane::Apply when the hold clears or expires; apply_hint and the Ap |
| ht-5fr.3.16 | ht-5fr.3.8 | consumes ApplyContext and the apply_pass/replay engine |
| ht-5fr.3.16 | ht-5fr.9 | consumes TRUST-POLICY amendment (replicated_from:<node_uuid> provenance) |
| ht-5fr.3.15 | ht-5fr.3.6 | consumes journal::open, OpenReport and NodeIdentity::from_seed |
| ht-5fr.2.10 | ht-5fr.2.8 | consumes Arc<HostSessions> composed into the daemon health provider |
| ht-5fr.2.10 | ht-5fr.1.11 | consumes health hosts count and doctor store-only label |
| ht-5fr.2.10 | ht-5fr.2.12 | consumes tests/daemon/multi_host.rs suite and lazy attach by claim host (Health cases added there) |
| ht-5fr.2.10 | ht-5fr.2.7 | consumes HostSessions per-session evidence, reachability and lane status accessors |
| ht-5fr.2.9 | ht-5fr.2.7 | consumes HostSessions read API attached()/get() and per-session HostReachability::archival_state |
| ht-5fr.2.9 | ht-5fr.2.4 | consumes re-keyed baseline (seat_archival/herdr_hosts host_id keys) and the archival interim HostId param |
| ht-5fr.2.9 | ht-5fr.2.8 | consumes Arc<HostSessions> composed in run_elected_impl (archival sampler routing) |
| ht-5fr.2.8 | ht-5fr.2.7 | consumes HostSessions and HostAttachPlan |
| ht-5fr.2.8 | ht-5fr.2.6 | consumes interim single-host HostIndex publish (deleted here) |
| ht-5fr.2.7 | ht-5fr.2.4 | consumes OrdinaryIdentity constructed with host_id |
| ht-5fr.2.7 | ht-5fr.1.7 | consumes single-host HostSession value and attached_host service seam |
| ht-5fr.2.7 | ht-5fr.2.2 | consumes CommitKicks register/unregister fan-out |
| ht-5fr.2.7 | ht-5fr.2.5 | consumes ScheduledStore::for_host |
| ht-5fr.2.7 | ht-5fr.2.1 | consumes HostIndex, HostLocator/HostId and StorePort::attach_host |
| ht-5fr.2.6 | ht-5fr.2.11 | consumes seat-row host joins (primary accessor left only on the cooperative path it deletes) |
| ht-5fr.2.6 | ht-5fr.9 | consumes TRUST-POLICY amendment (claim attribution by host) |
| ht-5fr.2.6 | ht-5fr.2.1 | consumes HostIndex and StoreContext::hosts() |
| ht-5fr.2.6 | ht-5fr.2.4 | consumes re-keyed recovery_holds and effective_observation(db, HostId, target) |
| ht-5fr.2.6 | ht-5fr.2.3 | consumes CallerClaim.host and target-bearing command host |
| ht-5fr.2.5 | ht-5fr.2.1 | consumes HostId type in src/ports.rs |
| ht-5fr.2.5 | ht-5fr.2.4 | consumes host_id re-keyed wake.rs joins and interim observation worker (shared src/store/wake.rs, src/service/workers.rs |
| ht-5fr.2.4 | ht-5fr.2.1 | consumes HostId type in src/ports.rs |
| ht-5fr.2.4 | ht-5fr.1.10 | consumes seats host_id/is_local columns, seats_live_target(host_id,target_id) and LOCAL_SEAT |
| ht-5fr.2.3 | ht-5fr.2.1 | consumes HostLocator type in src/protocol/ids.rs |
| ht-5fr.2.3 | ht-5fr.1.3 | consumes RuntimeContext::require_host and old hook argv host parsing |
| ht-5fr.2.2 | ht-5fr.3.12 | consumes Lane::Apply/LaneSet u16 in src/service/kicks.rs (shared slots) |
| ht-5fr.2.1 | ht-5fr.1.5 | consumes herdr_hosts table and StoreContext |
| ht-5fr.2.1 | ht-5fr.1.6 | consumes src/protocol/ids.rs and src/ports.rs (file order) |
| ht-5fr.3.14 | ht-5fr.3.8 | consumes impl JournalPort for SqliteStore and apply_pass/replay |
| ht-5fr.3.14 | ht-5fr.3.15 | consumes seeded test constructor (distinct replay identity) |
| ht-5fr.3.13 | ht-5fr.3.15 | consumes journal::open call in run_elected_impl (the lane starts right after it) |
| ht-5fr.3.13 | ht-5fr.3.12 | consumes Lane::Apply and kicks_for_commit |
| ht-5fr.3.13 | ht-5fr.3.8 | consumes SqliteStore apply_pending and replay |
| ht-5fr.3.13 | ht-5fr.3.10 | consumes post-commit retag helper |
| ht-5fr.3.12 | ht-5fr.3.4 | consumes journal table in the baseline (kick mapping) |
| ht-5fr.3.12 | ht-5fr.3.6 | consumes SyncState in src/store/connection.rs (shared file) |
| ht-5fr.3.11 | ht-5fr.3.4 | consumes thread_registers table and triggers |
| ht-5fr.3.11 | ht-5fr.3.1 | consumes HlcStamp |
| ht-5fr.3.10 | ht-5fr.1.6 | consumes public_ids::fresh minting from local_node.tag |
| ht-5fr.3.10 | ht-5fr.3.8 | consumes follow_up authoring and apply_pending |
| ht-5fr.3.10 | ht-5fr.3.15 | consumes the journal::open call site in run_elected_impl (post-open helper call point) |
| ht-5fr.3.9 | ht-5fr.3.8 | consumes impl JournalPort for SqliteStore |
| ht-5fr.3.8 | ht-5fr.3.7 | consumes ingest and freeze |
| ht-5fr.3.7 | ht-5fr.3.6 | consumes store::journal module, SyncState and author |
| ht-5fr.3.7 | ht-5fr.3.3 | consumes ChainRules and IngestOutcome/ForkEvidence |
| ht-5fr.3.6 | ht-5fr.3.4 | consumes journal-core baseline DDL |
| ht-5fr.3.6 | ht-5fr.3.5 | consumes FactKind/FactRegistry and the ApplyContext type |
| ht-5fr.3.6 | ht-5fr.1.10 | consumes NodeDirectory upsert API and the daemon start node.json step |
| ht-5fr.3.5 | ht-5fr.3.2 | consumes EntityKey, LogKey and HlcStamp types in src/sync/ |
| ht-5fr.3.4 | ht-5fr.1.5 | consumes local_node table in migrations/node/0001_node_baseline.sql |
| ht-5fr.3.3 | ht-5fr.3.2 | consumes Envelope, EnvelopeError and LogKey/LogHead/Vector |
| ht-5fr.3.2 | ht-5fr.3.1 | consumes HlcStamp |
| ht-5fr.3.2 | ht-5fr.1.1 | consumes NodeIdentity::sign and NodeUuid (src/node/) |
| ht-5fr.1.11 | ht-5fr.1.7 | consumes HostChoice, store-only daemon start and the attached_host service seam |
| ht-5fr.1.11 | ht-5fr.1.10 | consumes node.json regeneration and the IdentityMismatch start check |
| ht-5fr.1.10 | ht-5fr.1.5 | consumes local_node/herdr_hosts/node_directory tables, initialize(conn, now, &NodeSeed) and the StoreContext primary_hos |
| ht-5fr.1.9 | ht-5fr.1.11 | consumes doctor store-only label (shared src/cli/doctor.rs and tests/cli/doctor_labels.rs) |
| ht-5fr.1.9 | ht-5fr.1.2 | consumes paths::legacy_instances_dir |
| ht-5fr.1.8 | ht-5fr.1.6 | consumes stable test tree (chain deletion last per spec order) |
| ht-5fr.1.8 | ht-5fr.1.12 | consumes ID-consumer test edits in the shared test tree |
| ht-5fr.1.8 | ht-5fr.1.7 | consumes stable test tree (chain deletion last per spec order) |
| ht-5fr.1.8 | ht-5fr.1.5 | consumes initialize(conn, now, &NodeSeed) signature |
| ht-5fr.1.8 | ht-5fr.1.10 | consumes stable test tree (chain deletion last per spec order) |
| ht-5fr.1.8 | ht-5fr.1.9 | consumes stable test tree (chain deletion last per spec order) |
| ht-5fr.1.7 | ht-5fr.1.5 | consumes herdr_hosts table and StoreContext primary_host accessor |
| ht-5fr.1.7 | ht-5fr.1.3 | consumes descriptor primary_host and the optional --host-endpoint attach hint |
| ht-5fr.1.6 | ht-5fr.1.5 | consumes local_node.tag column read by fresh |
| ht-5fr.1.5 | ht-5fr.1.4 | consumes migrations/node/0001_node_baseline.sql and verify_baseline |
| ht-5fr.1.5 | ht-5fr.1.2 | consumes NodeIdentity loaded in run_elected_impl via NodePaths |
| ht-5fr.1.3 | ht-5fr.1.2 | consumes host-free NodePaths::resolve and descriptor node_uuid |
| ht-5fr.1.2 | ht-5fr.1.1 | consumes NodeUuid, NodeIdentity::load_or_create and the NodeJson reader (src/node/) |
| ht-5fr.13 | ht-5fr.4.13 | consumes Deadline lane scope change (record_overdue_if_pending deciding-node guard and origin-scoped due scans) |
| ht-5fr.13 | ht-5fr.4.14 | consumes remote seat read rendering (rows the marker line follows) |
| ht-5fr.13 | ht-5fr.5.10 | consumes composed SyncHandle (signals wired into the deadline lane in app.rs) |
| ht-5fr.13 | ht-5fr.5.6 | consumes sync round-complete signal (SyncSignals via SyncHandle::signals) and per-node reachability (contact flush into  |
| ht-5fr.12 | ht-5fr.1.9 | consumes legacy purge and the doctor legacy report |
| ht-5fr.12 | ht-5fr.7 | consumes discovery behaviour and the sync.discover setting |
| ht-5fr.12 | ht-5fr.4.14 | consumes remote seat read rendering (state remote, format and size) |
| ht-5fr.12 | ht-5fr.2.10 | consumes health and doctor output per attached host |
| ht-5fr.12 | ht-5fr.1.11 | consumes store-only doctor label and health hosts count |
| ht-5fr.12 | ht-5fr.8 | consumes peer/sync CLI commands and their wire commands |
| ht-5fr.12 | ht-5fr.13 | consumes the unreachable-node marker line format |
| ht-5fr.11 | ht-5fr.6.15 | consumes IncarnationsSink/IncarnationsWorker composed into the daemon (fork freeze, owner rotation, branch pulls and re- |
| ht-5fr.11 | ht-5fr.2.12 | consumes fully routed multi-host daemon (HostSessions composed, dispatch by claim host) |
| ht-5fr.11 | ht-5fr.13 | consumes the unreachable-node marker line in thread show/participants (partition+heal case) |
| ht-5fr.11 | ht-5fr.8 | consumes sync verify (agreement after heal), sync resync (restore-fork case) and sync clone-cleared (clone case) CLI com |
| ht-5fr.11 | ht-5fr.6.8 | consumes coordinator fork wiring (fork.v1 frames routed and served, outbox drained, fork refs in Have) |
| ht-5fr.11 | ht-5fr.10 | consumes multi-node test harness |
| ht-5fr.11 | ht-5fr.4.12 | consumes v1 fact kind modules: message.sent and receipt.settled (send/ACK) |
| ht-5fr.11 | ht-5fr.7 | consumes the discovery worker and the sync.discover setting (unconditional discovery case) |
| ht-5fr.11 | ht-5fr.4.7 | consumes v1 fact kind modules: seat.created/seat.retired and remote eligibility |
| ht-5fr.11 | ht-5fr.4.10 | consumes v1 fact kind modules: thread, membership and invitation kinds with ensure_direct_thread (crossed direct pairs) |
| ht-5fr.10 | ht-5fr.2.8 | consumes fake-locator HostFactory on the composed daemon path |
| ht-5fr.10 | ht-5fr.5.10 | consumes sync.dial_command setting and peer --stdio command, composed into a dialing daemon (setting from ht-5fr.5.7, co |
| ht-5fr.8 | ht-5fr.6.12 | consumes IncarnationStore status (store::incarnations::status, IncarnationsStatus with own_unknown_newer and clone_evide |
| ht-5fr.8 | ht-5fr.6.10 | consumes sync::resync::request (ResyncMarker, preconditions, graceful stop) behind sync resync |
| ht-5fr.8 | ht-5fr.6.15 | sync clone-cleared must signal the running incarnations worker so the held restore rotation runs; the worker is composed |
| ht-5fr.7 | ht-5fr.5.7 | consumes SyncHandle::reload_targets() (Dialer picks up discovered targets) |
| ht-5fr.7 | ht-5fr.5.4 | consumes sync_dial_targets table and store::sync_dial_targets::upsert_discovered |
| ht-5fr.7 | ht-5fr.5.10 | consumes the sync runtime composed in run_elected_impl (worker composition point, two-daemon link test setup) |
| ht-5fr.7 | ht-5fr.2.7 | consumes HostSessions any_reachable attached-host check |

## Design questions

### A. Should mergeCheck run the tests for the files a merge touched?
Per-merge mergeCheck is build-only while the sweep is deferred. A test break caused by one task therefore surfaces only in a later task's review or in the final sweep.
- In this run, a commit_kicks fixture broke when ht-5fr.3.6 and 3.7 merged, and was found later as fix bead ht-5fr.3.32. The same break fed into a false BLOCKED on ht-5fr.2.4.
- The phase-6 sweep found 8 more failures.

Both sides:
- **For:** a break is caught one merge after it happens, and reviewers stop labelling those failures pre-existing.
- **Against:** the serialized merge lane gets longer, and a test that is flaky under load (here a failpoints test near its 300 s timeout) becomes a merge failure.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### B. Should file overlap alone stop the graph pass from marking a narrow `safe yes`?
graph-pass-prompt.md:49–53 allows `safe yes` only when the two beads declare no file in common.
- In this run that parked epic narrows worth `depth 30→19, width 3.2→5.1`; all were "safe no" on shared-file overlap only. Applying them took a mid-run human approval.
- Run 1 still showed a 2 h stretch with a single task in flight.

Both sides:
- **For relaxing:** hotFileCap plus rebase and seam review already serialize same-file work, and the `owns:`, `consumes:` and `needs:` checks catch edges that carry a real artifact.
- **For keeping:** declared shared files often hide implicit interface coupling. Run 1 had 14 rebase conflicts, so the overlap was real.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps
- **Base drift when the session's merge is refused.** super-auto §Base drift defines only `baseAbsorbed: <base sha> → <merge sha>, <N> conflicted files`. Here the permission layer refused the session's merge of base (7 commits, v1.1.0). The session fell back on the generic "skip, record, report as uncovered" rule, and the sweep and final review judged a tree that is not the one that will land. Suggested: define `baseAbsorbed: refused — <base sha>, <N> commits behind (<cause>)`, give report-status a `base not absorbed` qualifier, and have phase 6 state that the sweep judged the branch without base.
- **The codeBuckets block form is not shown where phase 6 writes it.** run-state.md:40–49 names the fields with dotted keys (`codeBuckets.completed`). A session wrote `- codeBuckets.sweep:` bullet lines, and report-status (which matches only `/^codeBuckets:/`) reported `sweep: none recorded` and `final review: none recorded` without a warning. Suggested: show the literal indented block at the phase-6 write site, and have report-status warn on a `codeBuckets.` key outside the block.
- **Duplicate `## Key abstractions` in nested specs.** The nested-spec author wrote its own section, and the Key Abstractions "Apply" step inserted the designer's next to it (friction 2026-10-14). The re-entry rule would instead adopt the author's copy. Suggested: tell the nested author not to write that section, and have Apply replace an existing one.

## Already fixed — do not re-litigate
- **Evidence-only fix pass classed BLOCKED.** A fix pass that adds only evidence, with no new commit, is classed BLOCKED. It happened 4 times (ht-5fr.1.2, 1.6, 2.4, 3.37), and its stacked tasks were cancelled and redone (5 cancellations). This was already delivered upstream mid-run through the superpowers workspace agent tab. It is not fixed as of 6.4.2-alepar5.1; it is listed here so it is not filed twice. The final counts add weight: `fix-pass — entered 10 · FIXED 6 · BLOCKED 4`.

## Not established
- The coordinator's ledger-append path is fire-and-forget and can lose a line without noticing. Every ledger-derived count above is therefore a lower bound; `Metrics: ledger-check` is the one cross-check.
- Two invocations' Metrics were UNAVAILABLE (defect 4). The Fix loop and Merge-back lines come from the last invocation that produced counts.
- Defect 2, defect 6's clusterOverride pattern and the doc gaps are each observed in this one run only.
- No Profile lines were written, so there is no timing model and no what-if figures.
- The analyst ran on opus.

## Verification bar
- **Defects 1 and 2:** resume a coordinator against a ledger that holds a `landed (…closure unconfirmed)` line for a bead now closed, and against a triage wait edge to a `review:` bead. Check that stopReason is not `closure-unconfirmed` or `stalled`.
- **Defect 3:** add a dep edge and close a blocker mid-round. Check that the newly ready bead is dispatched without waiting for a landing.
- **Defect 4:** force the Finish ledger read to return null. Metrics must still be computed.
- **Defect 6:** re-run the scope filter on this run's round-1 findings. `Cargo.toml:64` and the ID-regex consumers should come back in-scope.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
