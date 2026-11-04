# Final review (bounded): epic ht-8lem, atomic remote handoff

Branch `super-auto/atomic-remote-handoff`, captured source base `1e620fdb570ab6e8a254cee334fca7a484d79354`, head `40283c9474754a2d268e3eaace1c83adefd23e44` (both verified with `git rev-parse --verify <sha>^{commit}`). Caller checkout observed at HEAD `40283c947`, working tree clean; it was not switched or modified.

This is a BOUNDED review. The review-input selection is `status: unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, 158 changed paths, 0 omitted, 5 provenance gaps. No whole-range package was built or loaded. Only the paths and hunks listed in `bounded-coverage.json` were inspected from captured Git; everything else is unreviewed. A whole-range review remains outstanding. No full-suite measurement of this branch exists (the sweep is deferred to the caller), and no tests were run by this review.

## Scope inspected

Core of the feature, read from captured Git: the signed block (`src/sync/handoff_block.rs`), the fact and its authority rule (`src/sync/facts/message.rs`, `src/sync/facts/mod.rs`, `src/store/materialize.rs`), message materialization and the deadline owner (`src/store/messages.rs`, `src/store/schema.rs`, migration 0015), the sender ledger and requester (`src/store/handoff_obligations.rs`, `src/service/peer_handoff.rs`), the target's decide checks, worker and staging (`src/service/peer_launcher.rs` up to its test hunks, `src/store/peer_handoff.rs`, `src/store/peer_requests.rs`), the wire and caps (`src/sync/frame.rs`, `src/sync/coordinator.rs`, `src/sync/coordinator/remote_launch.rs` except its last test hunk), attention/alerts (`src/store/attention.rs`), the due driver hook (`src/scheduler/deadlines.rs`), TRUST-POLICY.md, the wire-compat script, SKILL.md, and the 0015 migration test in `tests/store/schema.rs`. Design doc sections 1-5 and 2.3 and parts of `tests/multinode/handoff_atomic.rs` and `src/cli/peer_handoff_launch.rs` were read for context but are not claimed as covered.

## Findings (own view, step 1)

### Important: a linked handoff message never goes overdue on any node (spec 4.3, TRUST-POLICY N5)
- `messages.deadline_node` and `obligation_authored_here` were switched to `COALESCE(deadline_node, origin_node)`, but the receipt due scans that *select* candidates were not: `src/store/receipts.rs:1019` (`PHYSICAL_DUE_SELECTION`), `:1081` (the `receipt_due_sparse` insert) and `:1355` all still require `messages.origin_node = local_node`. `src/store/receipts.rs` is not in the changed-path inventory.
- On the sender node the handoff message's origin is the target node, so its receipt is never selected; on the target node it is selected but `obligation_authored_here` answers false (deadline_node is the sender). The ledger row stops being scanned once `linked` (`due_rows` only reads `sent|launched` and unseen `failed`).
- Failure scenario: `handoff --pane P@M --deadline 10` where the message replicates back to the sender before the deadline (the normal case), and the recipient never ACKs: no `overdue` warning is ever recorded on either node. Spec 4.3 requires the sender's node to fire it "after linking through the ordinary receipt path".
- The only multinode overdue test (`overdue_while_partitioned`) covers the overdue-before-link path; the integration task's parked finding already noted "receipt overdue scans filter on origin_node, not deadline_node" and left it unresolved.

### Important: committed test `migration_0015_handoff_obligations` fails deterministically
- `tests/store/schema.rs` (`migration_0015_handoff_obligations`, ~l.7350-7365) inserts `messages` rows with `thread_id='t'`, a thread that does not exist; the ledger records the failure as `message instance does not match thread` at tests/store/schema.rs:7365 at every task since ht-8lem.1, with attribution "gap — open, owner unassigned".
- The test was added by this branch, so it is a failing test introduced here, not a baseline flake. The `deadline_node` CHECK (31-char refusal) is therefore not actually verified.

### Important: the target accepts a handoff from a retired sender seat (spec 3.1 step 3)
- Spec 3.1 step 3 requires the author to be a seat held as remote, `home_node == from`, nonretired. `handoff_refusal` (`src/service/peer_launcher.rs`, the `SELECT home_node,is_local,name FROM seats` read) checks only `is_local` and `home_node`; there is no `state`/`retired_seq` check, and no test of it (grep finds no "retir" in `tests/daemon/peer_handoff_decide.rs`).
- The sender side compounds it: `record` checks retirement only when signing, while `resend_set` keeps replaying the frozen bytes for every `sent|launched` row for up to 6 days regardless of the author's state.
- Failure scenario: seat A signs a handoff, the target is offline; A is retired; the target links later, the re-send task replays the request, the target launches an agent and stages a task message from a retired seat. Task 11's deferred `[weak-validation]` minor is the same gap.

### Minor (own view)
- `Requester::ledger_answer` documents that a rerun showing a failed row marks its alert shown, but it marks only when `overdue_seq` is already set; a failure fetched by the re-send task and shown by a rerun before the deadline still raises the seat-addressed alert at the deadline (duplicate notification).
- `LaunchPlan::required_cap` returns one cap, so a handoff to a `NewTerminal` target checks only `peer.handoff.v1`, not also `peer.launch.terminal.v1`; production always advertises both, so only a withheld-cap node is affected.
- `resend_pass` aborts the whole pass on the first row whose settle write errors (`?` inside the loop), delaying every later row to the next 2 s tick.
- `store::peer_handoff::stage` relies on `debug_assert!(block.restates(&payload))` and on decide's earlier verification; in release builds the authoring node does not re-run the authority predicate before writing, as spec 4.2 says it does. Low risk because the payload is built from `expected_payload`.

## Ledger triage (step 2)

- Recurring minor `[missing-evidence]` (ht-8lem.1, .7, .3; also .8, .14, .20, .15): process-only; Deferred OK.
- Recurring `[unverified-baseline-claim]` / attribution gap on `migration_0015_handoff_obligations` (ht-8lem.17, .3, .4, .8, .14, .20, .10, .21): misgraded as minor. It names a failing test this branch added; must fix (see above).
- Task 16 (ht-8lem.15) parked finding: the overdue acceptance ("with the seat known: one in-thread warn on the sender, none on the target, recipient ACK clears") is only partly asserted, and the report admits the link-before-Launched path produces no in-thread warn because receipt scans filter on `origin_node`. Triaged as must fix: it is the product defect in the first finding, not only a test gap. The other parked parts (staged-message CLI print not asserted at multinode level; no multinode test of the older-third-node replay) are untested scope; head has tests named `fail_after_staging_prints_the_staged_message` and `older_third_node_fails_then_replays_after_upgrade`, which were not inspected here.
- Task 11 `[weak-validation]` sender seat check: escalated to must fix (retired sender seat, above).
- Task 14 `[starvation-unbounded-retry]`: a `launched` row whose pair thread is never held stays in `due_rows` every pass. It is reachable: `open_pair` skips a retired author, so such rows can starve later due rows under the pass limit. Fix alongside the retired-seat item; not independently blocking.
- Task 14 `[edge-case-untested]` `handoff_warning_of` reusing a cleared handoff warning: a missed-overdue path in principle, but it needs another ACK in the pair before the link. Its value depends on the receipt-scan fix above; re-check it once that lands.
- Task 10 `[stale-test-owned-by-later-task]` `tests/multinode/peer_launch_terminal.rs` failing: head shows that file changed by +231/-? lines in the range (likely ht-8lem.15's rewrite), but it was not inspected and no suite run exists. Untested scope until the caller's sweep.
- Task 16 `[weakened-test]` remote_addresses.rs assertions removed: untested scope; confirm against the spec amendment in the sweep.
- Task 16 `[slow-test]` 76 s `overdue_while_partitioned` against the 5-minute suite budget: Deferred OK per the release-not-blocked-on-suite-time rule, but track it.
- Task 5 `[hand-built-failed-answer]`, Task 7 idempotent early-return items, Task 9 `[duplicated-logic]`, Task 10 `[dead-code-left]` / `[missing-edge-guard]` / `[string-size-drift]`, Task 2 `[wire-strictness]` (`SenderPrompt` deny_unknown_fields: local-only type, check-wire-compat passed), Task 3/15/8 doc wrapping, Task 4/14 unused `now`, Task 18 formatting, Task 16 test seam: Deferred OK.
- Missing dispatch/CLI tests (Task 7, 9, 11, 12, 13, 17, 18): untested scope.
- Metrics lines are UNAVAILABLE (ledger re-read malformed), so absence of a record in the ledger is not read as clean. No BLOCKED-AUTH lines.

## Unreviewed (bounded route)

- Provenance gap: `40283c947:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:34` malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: `docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51` (inv-8759cc82-264d-4506-beaa-307c9600bb8d), invalid failed assembly archive, at base and head.
- Provenance gap: `docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58` (inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f), invalid failed assembly archive, at base and head.
- Unreviewed product code: `src/cli/peer_launch.rs` (+968), `src/cli/commands.rs`, `src/cli/handoff.rs`, `src/cli/mod.rs`, `src/cli/topology_handoff.rs`, `src/cli/peer_handoff_launch.rs` (partly read), `src/service/sync_admin.rs`, `src/protocol/sync_admin.rs`, `src/protocol/output_compact.rs`, `src/protocol/commands.rs`, `src/protocol/results.rs`, `src/protocol/capabilities.rs`, `src/store/control.rs`, `src/store/control/direct.rs`, `src/store/topology_handoff/runtime.rs`, `src/service/dispatch.rs`, `src/app.rs`, `src/daemon/*`, `src/ports.rs` and the remaining small one-line changes.
- Unreviewed hunks in inspected files: the last test hunk of `src/service/peer_launcher.rs` and of `src/sync/coordinator/remote_launch.rs`.
- Unreviewed tests: all of `tests/` except the inspected `tests/store/schema.rs` and `tests/scheduler/deadlines.rs` hunks (`tests/multinode/handoff_atomic.rs` partly read for context), including the golden handoff fixtures.
- Unreviewed docs: the design doc beyond sections 1-5 and 2.3, roast/coverage artifacts, `docs/environments.md`, `docs/superpowers/specs/INDEX.md`, `evidence/ht-8lem.4/lib-tests.log`.

## Verdict
not ready (bounded review; linked handoff messages never go overdue, a committed test fails, retired sender seats are accepted; no full-suite run)

## Must fix before landing
- Receipt due scans in src/store/receipts.rs (lines 1019, 1081, 1355) still filter on messages.origin_node, so a handoff message linked before its deadline never raises an overdue on any node (spec 4.3, N5); switch them to COALESCE(deadline_node, origin_node) and add a multinode test for the link-before-deadline, no-ACK path.
- tests/store/schema.rs migration_0015_handoff_obligations fails deterministically ("message instance does not match thread" at ~l.7365: messages inserted into nonexistent thread 't'); fix the fixture so the deadline_node CHECK is actually verified.
- handoff_refusal in src/service/peer_launcher.rs does not refuse a retired sender seat (spec 3.1 step 3), and the sender's re-send task keeps replaying a retired author's request for up to 6 days; add the retirement check (and stop re-sending, or expire, rows whose author retired) with tests.
- Whole-range review still outstanding: this bounded review covered 109 hunks in 22 of 158 paths, with three provenance gaps unresolved.

## Untested scope
- No full-suite run of this branch exists; the caller's sweep (cargo nextest run --locked --all-targets --all-features) is still owed, so the branch is not treated as tested.
- tests/multinode/peer_launch_terminal.rs was reported failing at ht-8lem.10 and later rewritten; not inspected or run here.
- Overdue acceptance "recipient ACK clears" and the link-before-Launched in-thread warning are not asserted (parked in ht-8lem.15).
- Staged-message CLI print and older-third-node replay multinode coverage were parked in ht-8lem.15; head tests exist by name but were not inspected.
- No dispatch-level tests for PeerHandoffStage / PeerRequestView routing and elected_writer authority (ht-8lem.7, .8).
- No in-process test of the hc worker handoff branch (allocate_gate_seat -> stage_handoff -> gate opens on staged Send) (ht-8lem.9).
- No coordinator-level test that a handoff plan to a peer without peer.handoff.v1 sends no frame (ht-8lem.4).
- No CLI-level test that the inbox display requests and advances seat alerts (ht-8lem.21).
- remote_addresses.rs handoff assertions were removed (ht-8lem.15); confirm against the spec amendment.
- All unreviewed paths and provenance gaps listed under "Unreviewed (bounded route)".

## Deferred OK
- Recurring missing-evidence process findings across tasks (no code impact).
- handoff_obligations due_rows starvation by launched rows without a held pair thread; fix together with the retired-author item, not blocking alone.
- handoff_warning_of reuse of a cleared handoff warning; re-check after the receipt-scan fix.
- ledger_answer marks a shown failure only when already overdue (duplicate seat alert later).
- required_cap checks only peer.handoff.v1 for a handoff to a new terminal (production advertises both caps).
- resend_pass aborts the whole pass on one row's settle error.
- stage relies on debug_assert for the restates check in release builds.
- 76 s overdue_while_partitioned against the 5-minute suite budget (track, not a release gate).
- Hand-built Failed answer, idempotent early-return order in stage, duplicated settle mapping, dead RemoteLaunch.then code, empty message= guard, string-size drift, SenderPrompt deny_unknown_fields, doc wrapping, unused now parameter, formatting nit, test seam in peer_handoff_launch.rs.
