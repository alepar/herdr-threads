**Verdict: not ready.** The code mostly matches the spec, but these block landing: one confirmed failing test that the branch itself introduced, current main has not been merged in (it has three conflicting files), and no full-suite run exists for the branch tip.

## Step 1: my own review against the spec

I built the review package (fork point 998d7d34, 137 commits, 263 files) and read the root spec, including its post-implementation notes. I focused on the newest and riskiest seam: the per-session pending views (ht-yha.44, .54, .55). They landed after the last full-suite run and rewrote the core of delivery.

- **Per-binding views fit together correctly.** One predicate (`view_held_sql!`) decides which bindings hold a view. The v47 triggers, the seed, `disconnect_with_hooks` (which drops a view on any disconnect except process exit) and `connect` (which re-seeds it) all use it. A schema test pins the triggers to it. The only liveness transitions are in `bindings.rs:495` and `:568`. Every binding-creation path runs `read_cursors::seed`, which seeds the view last.
- **Wake, stall rule and digest** read only views of connected top-level bindings, plus legacy rows. No delivery or wake decision reads `lazy_recipients.state` any more, which matches what the spec says about per-session views.
- **Gap: a process-exited binding can keep its view forever.**
  - Agent bindings that exit are never ended (TRUST-POLICY forbids ending one on heuristic evidence). Each one keeps a view, and every publish to its seat adds one row to it.
  - `end_event_session` / `disconnect` only change bindings whose liveness is still `connected`. So if the liveness worker records a process exit before the SessionEnd hook arrives, or the session crashes or is killed while its pane stays open, the view is never dropped.
  - TRUST-POLICY.md (around line 819) records this as an accepted limit. A fix pass wrote that limit; I see no explicit user acceptance of unbounded growth.
- **Merge to main not done.** Main is 43 commits ahead. A trial merge (`git merge-tree`) conflicts in `src/cli/direct_send.rs`, `tests/cli/wake_on.rs` and `tests/store/wake_on_wake.rs`. Main also changed `src/store/wake.rs`, which this branch rewrote, and binding and multi-binding reader tests. Those are seams where the merge can go wrong even without a textual conflict.
- **Skew test uses the wrong old version.** `tests/daemon/skew.rs` still pairs protocol 3 against 8. The real release pair is v0.11.0 (protocol 7) against v1.0.0 (protocol 8). The refusal is generic, so this is minor.
- **Dead fallbacks after protocol 8.** `batch_key` in `src/delivery/assemble.rs`, `register.js` `orderKey`, and two comments in `src/protocol/watch.rs` still describe "an older daemon" filling in a missing priority. Protocol 8 refuses mixed versions, so this code is unreachable. Minor.
- **Repair code for unreleased schema states.** `src/store/schema.rs` adds a `version == 47` missing-views branch and `repair_v47_view_triggers`. They only repair stores created at intermediate v47 states inside this unreleased branch, and they run an extra `sqlite_master` lookup on every open. Acceptable if the dogfood store needs them.
- **Checks I ran:**
  - `scripts/check-harness-neutral`: ok.
  - `scripts/check-default-features`: exit 0.
  - Focused `--lib` filters (binding_lazy, mod_seat_view, nudge_threshold, delivery_row_seek, priority_send, pair_auto_join, deadline_window, store::wake): all pass. The `lazy_schema` filter matched 0 tests, so it proved nothing.
  - An unfiltered `cargo test --lib` gave no summary within 590 s. That is inconclusive, so it is not evidence either way.

## Step 2: ledger triage

- **Recurring minor `[doc-drift]` (×8 across 6 tasks), class "no task owned the shared docs":** mostly fixed by ht-yha.34 and ht-yha.55. A grep of SKILL.md, help, README and agent-usage finds no lazy, initial-invitation or `wake_on: all` wording left. What remains are the dead older-daemon comments above and `tests/support/lazy_proxy.rs:99`.
- **Recurring minor `[missing-edge-case-test]` (×3 across 3 tasks), class "edge-case coverage left to later tasks":** test-only gaps. Deferred OK.
- **Misgraded minor, Task 42 (ht-yha.54) `[pre-existing-test-failure]` — must fix.** It names a failing test. I reproduced it at HEAD: `q4_retirement_refusal_controls::modern_lazy_retirement_requires_intact_captured_publication_and_recipient_authority` panics at `tests/service/q4_retirement_refusal_controls.rs:328` with `FOREIGN KEY constraint failed`. "Identical at base" means the base of ht-yha.54, so the failure came in with ht-yha.44 on this branch, and no follow-up bead exists. Production only deletes unpublished `lazy_recipients` rows, which views never hold, so this is a fixture fix (delete the `binding_pending` rows first), not a product defect.
- **Smell across many minors (flake claims with no base-run evidence, tests not written red first):** tasks 5, 12, 13, 32, 34, 38 and 40. The named flakes are hermes hook tests, `mod_delivery` stall timing, topology_handoff permits, `doctor_reports` (ht-kk8) and the q5 search refit (ht-3wm). Each failing test in the coming sweep needs a base-run comparison before anyone calls it a flake.
- **BLOCKED entries:**
  - ht-yha.33 (old-CLI cursor) and ht-yha.42 / .41 (`[p0]` annotation): both settled, by the protocol-8 decision and by triage clarification respectively.
  - ht-yha.44 config5: settled by ht-yha.52.
  - Task 8's fix-pass BLOCKED was a gap in the report, not in the code.
- **Metrics:** "landed≠completed 42 vs 38" matches landings of ht-yha.1, .14 and .5.1 recorded as "tracker closure unconfirmed". Git shows those merges exist. Bead ht-cbb is still open although its fix landed in 011c24d2.

## Verdict
**Not ready.**

## Must fix before landing
1. Fix `q4_retirement_refusal_controls::modern_lazy_retirement_requires_intact_captured_publication_and_recipient_authority`: the fixture must remove `binding_pending` rows before deleting the `lazy_recipients` row. Confirmed failing at HEAD 02307bba.
2. Merge current main (43 commits). Resolve `src/cli/direct_send.rs`, `tests/cli/wake_on.rs` and `tests/store/wake_on_wake.rs`, then re-check `src/store/wake.rs` and the binding and multi-binding reader tests after the merge.
3. Run the full suite on the merged HEAD with `HT_LEAK_RUN_ID` set, then `scripts/check-no-leaked-processes --run-id`. Compare every failure against the base run before calling it a flake (ht-3wm, ht-kk8 and the others the tasks named).
4. Get the user's explicit decision on the process-exited-view growth limit (TRUST-POLICY.md around line 819). The alternative is a bounded rule: for example, drop the view when a lifecycle-claimed sibling on the same seat takes over, or when an end event arrives after a process exit.

## Untested scope
- **No full-suite run exists for 02307bba.** The last sweeps were at dae11184 and e8ecb681; the e8ecb681 sweep had 3 failures. Everything from ht-yha.44 through ht-yha.55 landed after it, including the new `binding_pending` table, 8 triggers and an amended v47 migration. Treat the branch as untested; the caller's sweep will be its first full measurement.
- The branch has never been tested merged with current main.
- The pre-release wake e2e has never run against this 1.0.0 candidate. It is required before tagging.
- A v46→v47 migration on a realistic store with many open and disconnected bindings is untested, and so is the repair path for intermediate v47 stores.
- D2 on the channel path is untested: an item already queued in the mod keeps its `wakes` value after T is raised.
- Search at 256–342 bytes on a p0 message is not tested against the real daemon.
- The race between the liveness worker's process-exit and the SessionEnd hook is untested; it decides whether a view is retained.

## Deferred OK
- The dead older-daemon fallbacks and comments (`assemble.rs` `batch_key`, `register.js` `orderKey`, `watch.rs`), and the stale `lazy_proxy.rs` capability strip.
- The skew test using protocol 3 instead of 7.
- The repair code for intermediate v47 states.
- The cost of the cursor-advance trigger, which is O(view-in-thread).
- Subagent bindings keep a view while connected. This deviates from the ht-yha.54 bead wording but is defensible, because subagents read their own stream.
- `LEGACY_PAIR_MARKER` is compiled only into test builds.
- `--max-bytes` bound of 384 in the test, and the hardcoded `p0_messages = 4`.
- Gaps in TDD evidence and reports (tasks 4, 5, 7, 8, 12, 13, 20, 21, 33).
- The `delivery_mode='ordinary'` audit value written for human-only ServiceSend recipients.
- Leftover tracker hygiene: close ht-cbb.