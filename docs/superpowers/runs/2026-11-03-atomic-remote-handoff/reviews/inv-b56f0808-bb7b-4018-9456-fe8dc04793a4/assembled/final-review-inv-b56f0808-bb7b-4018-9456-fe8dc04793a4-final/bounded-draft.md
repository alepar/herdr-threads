# Final review (bounded): ht-8lem atomic remote handoff

Invocation inv-b56f0808-bb7b-4018-9456-fe8dc04793a4 · branch super-auto/atomic-remote-handoff · base 1e620fdb570ab6e8a254cee334fca7a484d79354 · head ce1d8177124b3463b37706693ec09cb2e677efc8 (both verified with `git rev-parse --verify <sha>^{commit}`; the caller checkout was at head ce1d81771 with a clean status when the review started).

Route: bounded. review-inputs.json says status unavailable, sourceAvailable true, wholeRangeReady false, 174 changed paths, 0 omitted, 10 provenance gaps. I read the full changed-path inventory (diffstat.txt, 174 files, +22764/-502). I built no whole-range package. This report does not stand in for the whole-epic review, which is still outstanding.

## Scope inspected

This pass covered the deadline-ownership seam (N5), the sender ledger and the code-roast round 1 fix loop (ht-8lem.37 to .41, 9dafdced6..ce1d81771). Paths and hunks are listed in bounded-coverage.json: the 0015 migration, src/store/receipts.rs, src/store/schema.rs, src/store/handoff_obligations.rs, src/store/peer_handoff.rs, src/store/messages.rs, src/scheduler/deadlines.rs, the handoff_refusal hunks of src/service/peer_launcher.rs and the migration_0015 test hunk. I also read part of src/service/peer_handoff.rs (the IN_FLIGHT slot, poll, settle and the re-send pass) and the fix-loop diffs of tests/multinode/handoff_atomic.rs and tests/daemon/peer_handoff_decide.rs. Neither file is claimed in coverage.

## Step 1: own view (diff against spec and roast)

- Roast Blocking 1 (receipt overdue dead after link) looks fixed. One `deadline_owner_sql!` fragment, COALESCE(deadline_node, origin_node), now drives PHYSICAL_DUE_SELECTION, the receipt_due_sparse repair insert, both halves of the scan_extension_lapses UNION and obligation_authored_here. Migration 0015 recreates both receipt_due_sparse triggers with the same predicate. materialize_message writes deadline_node in the same INSERT as the message row, ahead of any receipt row, so the AFTER INSERT trigger sees it. No receipt due filter on origin_node is left under src/ (git grep). The invitation filter correctly stays on invitations.origin_node.
- Roast Blocking 2 (migration_0015 test) looks fixed. The fixture now inserts the thread row and uses distinct sequences, and the 31-char deadline_node insert asserts "CHECK constraint failed", so the check is no longer vacuous.
- Roast Should-fix (retired author accepted) is fixed. handoff_refusal reads state='retired' and refuses with "the sender seat is retired here" (31 bytes) after the home check and before the profile-dir check. A daemon test pins the order. A replicated remote seat's retirement does set state='retired' (seats/publication.rs), so the check fires for remote rows.
- Roast Should-fix (link-then-never-ACKed untested) is addressed by linked_then_overdue_on_the_sender_only. It asserts the row linked before its deadline, exactly one overdue:receipt on node 0, no overdue:handoff and nothing on node 1. overdue_while_partitioned no longer fakes Launched with raw SQL.
- Roast Nit (re-send race, "too many launch requests in flight") is addressed by the process-wide IN_FLIGHT slot, but the fix adds a budget overrun (see Must fix). Requester::poll computes `wait = min(remaining, SEND_WAIT_MS)`, can spend up to `wait` in wait_slot, then sends with the same full `wait`. A CLI poll that meets a re-send holding the slot (target slow or unreachable, up to SEND_WAIT_MS = 10 s) can therefore run for about 2x wait plus REPLICA_WAIT_MS, past its own budget deadline. The spec's "the CLI poll says still pending" outcome then depends on whether the client's call timeout fires first. The fix is cheap: recompute the wait from the budget after the slot wait. wait_slot also reads wall-clock Instant, not the injected Clock (ledger minor, ht-8lem.41).
- Ledger state machine (handoff_obligations.rs): the guarded transitions match spec section 2.3 (sent->launched, sent|launched->linked, sent|launched->expired, sent->failed). Expiry runs the due step first, so no deadline-bearing row expires without its overdue. TTL < RETENTION_MS is asserted at compile time. Failed{staged:Some} keeps the row open in settle, so the deferred minor about the sender half (ht-8lem.18) is resolved.
- Target staging (store/peer_handoff.rs): one transaction, idempotent on request id. It refuses a non-running row, a different seat and a retired or foreign target seat, and it authors only the recipient's join (N4). The idempotent early return comes before the seat and gate checks (ledger minors ht-8lem.7). Single-transaction staging makes the "first stage without a gate, repeat with one" order unreachable from the current callers, so that is deferred.
- Due-pass starvation (roast Nit, cluster B dropped): due_rows keeps selecting launched rows that have a known seat and no held pair thread, ordered by deadline with LIMIT 16. Sixteen such rows starve later ones until expiry. It is still unfixed and needs an unusual replication state, so it is deferred with the cluster-B remedy on record.

## Step 2: ledger triage

- Recurring minor [missing-evidence] (two clusters: ht-8lem.1/.7/.3 and ht-8lem.40/.37/.41) is process-class. Fix-loop test claims (for example ".40: 3 runs, 2 passed each") rest on unretained /tmp logs, so the focused-test evidence for the fix loop is prose only. I treat this as untested scope, not as a product defect.
- ht-8lem.14 [edge-case-untested] handoff_warning_of is graded must-fix because it names a missed-overdue product path. A handoff overdue:handoff warning fires before link, and its ("receipt", seat) condition is later cleared by the recipient's ACK of another pair message. When the linked handoff message's receipt then goes due, record_overdue_inner finds no active condition, picks up the old cleared warning through handoff_warning_of and inserts nothing, so the sender is never told the handed-off task itself went unACKed. Confirm with a store test. Then either record a fresh overdue when the reused warning's condition is closed, or accept it as a limit in TRUST-POLICY.
- ht-8lem.15 parked finding (recipient-ACK clearing not asserted at multinode level; fail_after_staging output discarded): its root cause, the origin_node due filter, is fixed by .37. ACK clearing is covered at store/service level (ack_clears_handoff_warning, tests/service/peer_handoff.rs). Deferred OK.
- ht-8lem.15 [slow-test] overdue_while_partitioned takes 76 s of real-time waits. With the full suite unmeasured, that time counts against the 5-minute budget and could push past it. Untested scope until a suite run measures it.
- ht-8lem.1 [unmeasured-flake] codex_schema_tests older_version_with_matching_schemas_is_refused_without_fingerprinting flaked once. It is unrelated to the epic and belongs to the flake-hunt track.
- The other minors (line-wrap, doc drift, dead RemoteLaunch.then path, unused `_now`, hand-built Failed answer, duplicated settle mapping, missing dispatch-level tests, string-size drift, test seam in product file, duplicated trigger SQL pinned by a test) are maintainability or coverage nits. Deferred OK.
- Roast punch-list nits: plain-launch request-key rekey v3->v4 orphans a 2.x pending launch across an upgrade, the SKILL.md:64/212 launch prompt text is stale, the re-send snapshot error is not logged, there is no log for a seat known without a pair, and the branch carries ~124 duplicated no-op commits with non-compiling intermediates. The commit history needs a squash merge or history cleanup before landing on main, because bisect is broken otherwise.
- BLOCKED-AUTH lines: none in the ledger. Metrics: the ledger-check reports landed≠completed (22 vs 5) and read-ledger transport gaps, so the ledger is incomplete. I do not read absence of findings there as clean.

## Unreviewed (bounded route)

- Every changed path and hunk not listed in bounded-coverage.json is unreviewed. That includes src/service/peer_handoff.rs (claimed only in part), src/sync/handoff_block.rs, src/sync/frame.rs, src/sync/coordinator.rs and coordinator/remote_launch.rs, src/sync/facts/*, src/cli/peer_launch.rs, src/cli/peer_handoff_launch.rs, src/cli/handoff.rs, src/cli/topology_handoff.rs, src/cli/commands.rs and cli/mod.rs, src/protocol/*, src/store/attention.rs, src/store/control*, src/store/peer_requests.rs, src/store/materialize.rs, src/app.rs, src/ports.rs, the remaining src/service/peer_launcher.rs hunks (worker, gate_argv, launch_args, tests), TRUST-POLICY.md, integrations/skill/SKILL.md, scripts/check-wire-compat, every golden fixture and all test files except the migration_0015 hunk.
- Provenance gaps (from review-inputs.json): run.md@ce1d817 lines 40 and 68 have a malformed collection record. 2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 (inv-8759cc82) and 58 (inv-b6e4e8fd), at base and head, hold an invalid failed assembly archive. 2026-11-03-atomic-remote-handoff/run.md line 31 (inv-632e6c0e) has an incomplete claimed collection, an incomplete claimed final report, an invalid failed assembly archive (missing final-review-inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb.md) and an incomplete assembled bundle (digest.md).

## Verdict
not ready (bounded review only; no full-suite run on this head; one fix-loop budget overrun and one missed-overdue edge to settle)

## Must fix before landing
- src/service/peer_handoff.rs Requester::poll: after wait_slot, recompute the send wait from the remaining budget, so a poll that met an in-flight re-send does not run to about 2x SEND_WAIT_MS plus REPLICA_WAIT_MS, past its own deadline (introduced by ht-8lem.41).
- src/store/schema.rs handoff_warning_of (ht-8lem.14 minor, regraded): a pre-link overdue:handoff warning whose receipt condition was later cleared by an ACK of another pair message is reused for the linked handoff message's receipt, so no new overdue is recorded. Add a store test, then fix it or record it as an accepted limit in TRUST-POLICY.
- Run the unfiltered full suite on ce1d81771 with HT_LEAK_RUN_ID and scripts/check-no-leaked-processes --run-id. The caller deferred it, so no full-suite measurement of this branch exists and the branch is untested as a whole.
- Land through a squash or a cleaned history: the branch carries ~124 duplicated no-op commits and non-compiling intermediate commits (roast nit src/cli/handoff.rs:840), which break bisect on main.

## Untested scope
- Whole-range final review: this pass is bounded. Everything not in bounded-coverage.json is unreviewed (see Unreviewed above), and the 10 provenance gaps in review-inputs.json stay open.
- Full suite, the leak check and the 5-minute suite budget are unmeasured on this head. overdue_while_partitioned alone takes about 76 s of real-time waits.
- The fix-loop focused test runs (ht-8lem.37, .40, .41) are recorded only in prose, with logs in unretained /tmp scratch (recurring missing-evidence).
- No dispatch-level tests exist for PeerHandoffStage and PeerRequestView routing (ht-8lem.7, ht-8lem.8), and there is no CLI-level test of run_display_inbox alerts (ht-8lem.21).
- The hc handoff worker branch (allocate_gate_seat -> stage_handoff -> gate) has no in-process test (ht-8lem.9).
- Recipient-ACK clearing of a handoff warning is asserted at store/service level only, not at multinode level (ht-8lem.15 parked).

## Deferred OK
- Due-pass starvation by launched rows with a known seat and no held pair thread (roast nit handoff_obligations.rs:503, cluster B dropped); the remedy is recorded in run.md.
- No log or alert when a seat is known but its pair thread or seat row never replicates (roast nit handoff_obligations.rs:339).
- Re-send snapshot errors dropped without a daemon.log line (roast nit peer_handoff.rs:606).
- Plain remote launch request-key tag v3->v4 orphans a 2.x launch left pending across the upgrade (roast nit peer_launch.rs:291); acceptable for a 3.0.0 major.
- Stale SKILL.md launch-prompt text at lines 64 and 212 (roast nit); worth a follow-up because the skill is model-visible.
- wait_slot uses wall-clock Instant instead of the injected Clock (ht-8lem.41 minor).
- Idempotent staging early return skips the gate and seat checks (ht-8lem.7 minors); unreachable from the current single-transaction callers.
- The receipt_due_sparse triggers restate the deadline-owner fragment literally, pinned by deadline_owner_triggers_use_the_fragment (ht-8lem.37 minor).
- Dead RemoteLaunch.then, AfterHandoff and journaled record hook paths in src/cli/peer_launch.rs (ht-8lem.10 minor).
- TRUST-POLICY N4/N5/N15 changed with no dated Amended entry (roast nit); hygiene only.
- Unused `_now` in mark_overdue, the hand-built Failed answer, duplicated settle mapping, line-wrap, doc drift, string-size drift and the test seam in peer_handoff_launch.rs (task minors).
- Codex schema test flake older_version_with_matching_schemas_is_refused_without_fingerprinting (ht-8lem.1 minor); unrelated, belongs to the flake hunt.
