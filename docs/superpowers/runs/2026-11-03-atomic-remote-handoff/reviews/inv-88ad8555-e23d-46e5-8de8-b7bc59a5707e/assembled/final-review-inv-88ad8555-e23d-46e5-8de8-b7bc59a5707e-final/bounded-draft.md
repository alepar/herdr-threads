# Final review (bounded): ht-8lem atomic remote handoff

Base 1e620fdb570a, head 9bd767327eac. Bounded captured-source route: review-inputs.json is status unavailable, sourceAvailable true, wholeRangeReady false, 204 changed paths, 0 omitted, 17 provenance gaps.

## Scope inspected

This pass covers the delta since the last bounded final review (inv-dd32bbd0 at 8d55586e9): ht-8lem.48 (cb39dfe0b, three-line seat wait in tampered_block_three_nodes), the sweep evidence at 1ba75ec1a and the ht-8lem.48 5x evidence log. It also claims 23 changed source paths (65 hunks) that no earlier bounded pass claimed: the protocol surface (capabilities, commands, results, sync_admin), the dispatch, port and worker wiring (dispatch.rs, ports.rs, workers.rs, daemon/mod.rs, daemon/control.rs, app.rs), the target staging store entry (store/topology_handoff/runtime.rs), the pair helpers (store/control.rs, control/direct.rs), the seat-alert read path (inbox_v2/mod.rs, queries.rs, cooperative.rs, output_compact.rs, assemble.rs, hook_inbox.rs, delivery_pull.rs, accept_delivery.rs) and the prompt-v2 call sites in cli/topology_handoff.rs and topology_handoff/automatic.rs. bounded-coverage.json lists them.

I also read, without claiming them in coverage: src/cli/mod.rs lines 2645-2770 (the text-inbox seat_alerts request and AdvanceCursors.alerts), handoff_obligations.rs resend_set, due_rows and mark_alerts_shown, peer_handoff.rs start_resend_task, the summary_prompt module in cli/handoff.rs, and spec sections Goal, 6, 7, 9, Acceptance and Q4.

## Step 1: own view

- ht-8lem.48 fixes the prior Must-fix "tampered_block_three_nodes misses seat waits on nodes 0 and 1". cb39dfe0b adds support::wait_seats on nodes 0 and 1 before the node-0 send, the same shape as ht-8lem.47. evidence/ht-8lem.48/handoff-atomic-5x.log shows 5 runs of the handoff_atomic group, 15/15 passing each time. The 1ba75ec1a sweep log has exactly this test as its only failure (9128 passed, 1 failed, 54 skipped, 258 s). No full suite exists at 9bd767327.
- Still unfixed at head (carried from inv-583f17bf and inv-dd32bbd0): handoff_obligations::resend_set selects every sent|launched row with no author-state filter, and no seat-retirement path touches handoff_obligations. ALERT_PENDING_SQL only suppresses alerts for a retired author. peer_launcher refuses a retired sender only once the retirement fact has replicated to the target (ht-8lem.39). A link-up re-send can still get there first and start an agent for a retired author.
- The seat-alert read path matches spec Q4. The text inbox sets seat_alerts only for a v2, binding-cursor, non-subagent, Text-format, first, thread-less page under inbox.seat_alerts_v1. The daemon adds HandoffOverdue items ahead of the merge only on that page. The CLI's AdvanceCursors carries the shown request ids, and also sends an empty advance when only alerts were shown. advance_cursors clears them only for a top_level, non-readonly binding. mark_alerts_shown filters on o.author = the claim's seat, so a binding cannot clear another seat's alerts. Hook and delivery pulls send seat_alerts false and fold the alert into counts (pending_warning_count, mod_seat_view other_pending). That is the spec's accepted consequence: one inbox call to read the alert.
- The protocol additions are wire-safe. RemoteLaunch.claim and resume_only are serde(default), and a unit test decodes the old JSON. HandoffLedgerOutcome and RemoteLaunchAnswer::HandoffLedger fall back to Unknown via serde(other). LaunchReply::Remote.handoff_message is skipped when None. HandoffOverdue uses deny_unknown_fields, but it is a local result reached only under its capability, so peer-wire rules do not apply.
- The PeerHandoffStage and PeerRequestView dispatch is served only through elected_writer(peer) under a foreground writer turn, and the request id length is validated (1..=64 bytes). The store entry prints the target audit line only on a fresh stage.
- The re-send thread joins with the incarnations worker on shutdown inside SYNC_STOP_BOUND, and its Pacer wait returns on cancellation. I see no shutdown hang.
- topology_handoff V3 plans use render_v2 (no thread id, no inbox hint) when the daemon advertises HANDOFF_SENDER_PROMPT_V2, else render_thread_free byte for byte. That is spec section 6. V2 plans freeze sender_prompt beside summary_prompt, and every launch_prompt call site now passes it.
- New nit: the HANDOFF_ATOMIC_REMOTE_V1 doc comment in src/protocol/capabilities.rs says "not advertised until its feature lands", but daemon/control.rs now advertises it unconditionally. The comment is stale.

## Step 2: ledger triage

- Recurring minor [missing-evidence] (ht-8lem.1/.7/.3 and .40/.37/.41) is process-class. ht-8lem.48 committed its evidence log, which is the opposite of that defect. Its minor [extra-file-changed] (the evidence log beside the test) is deferred OK.
- ht-8lem.15 parked (recipient ACK clearing not asserted at multinode level): deferred OK as in the earlier passes. The root cause was fixed by .37, and clearing is covered at store and service level.
- The ht-8lem.47 minor [sibling-test-same-race], which inv-dd32bbd0 regraded to must-fix, is now fixed by ht-8lem.48.
- The earlier deferred items keep their dispositions. I re-read them in inv-dd32bbd0's report but did not re-inspect their code.
- The Metrics ledger-check reads landed≠completed (29 vs 1) and read-ledger reports a malformed ledger-digest header and a failed git landing lookup. The ledger is incomplete, so I do not read absence there as clean. There are no BLOCKED-AUTH lines.
- The cleanup lines for .44 to .48 report exit 4 on uninspectable non-run pids 1950, 2008, 6582 and 976133 (systemd --user, sd-pam, sshd-session). Task worktrees and /tmp/sp-e3317cf8/ht-8lem.* were kept, which leaves caller cleanup to do.

## Unreviewed (bounded route)

- Every changed path and hunk not in bounded-coverage.json is unreviewed by this pass. That includes src/cli/commands.rs, src/cli/mod.rs (hunks), src/cli/peer_launch.rs, src/cli/launch*.rs, src/cli/help.rs, src/cli/watch.rs, src/protocol/handoff*.rs, src/protocol/authority.rs, src/scheduler/deadlines.rs, src/service/kicks.rs, src/service/mod.rs, src/store/attention.rs, src/store/receipts.rs, src/store/messages.rs, src/store/schema.rs, src/store/mod.rs, src/store/materialize.rs, src/store/peer_requests.rs, src/store/topology_handoff/automatic.rs, src/sync/*, src/test_support/multinode.rs, every test file and the docs and run artifacts. Earlier bounded passes claimed some of these at older heads; that coverage is not re-verified here.
- Provenance gaps (17): run.md@9bd767327 lines 54, 82, 85, 87 and 88 have a malformed collection record. The inv-8759cc82 and inv-b6e4e8fd failed assembly archives (2026-10-17 run.md:51 and :58, at base and head) are invalid. inv-632e6c0e (run.md:41) and inv-583f17bf (run.md:43) each have an incomplete claimed collection, an incomplete final report, missing bounded evidence and an incomplete digest.md bundle.

## Verdict
not ready (retired-author re-send unfixed; no full suite at head; whole-range review incomplete)

## Must fix before landing
- src/store/handoff_obligations.rs resend_set / src/service/peer_handoff.rs resend_pass re-send the sent|launched rows of a retired author: there is no author-state filter and no retirement hook into the ledger, so a link-up re-send can reach the target before the retirement fact and start an agent for a retired author. Filter retired authors, or fail or expire their rows at retirement, and add a test, or record an accepted limit in TRUST-POLICY (carried from inv-583f17bf and inv-dd32bbd0, still undispositioned at 9bd767327).
- Run the unfiltered full suite on 9bd767327 with HT_LEAK_RUN_ID, then scripts/check-no-leaked-processes --run-id. The caller deferred it, so this head has no full-suite measurement. The last sweep, at 1ba75ec1a, had 1 failure; ht-8lem.48 fixed it and it passed 5 focused runs, but the whole suite has not been re-measured.
- Complete a whole-range final review. This bounded pass covered 65 hunks in 23 of 204 paths, and 17 provenance gaps are open.
- Land through a squash or a cleaned history. The branch carries duplicated no-op and non-compiling intermediate commits (roast nit src/cli/handoff.rs:840), which break bisect on main.

## Untested scope
- whole-epic review incomplete; bounded inspected hunks only; all remaining hunks and provenance gaps remain unreviewed
- Full suite, leak check and temp-file check are unmeasured on 9bd767327; ht-8lem.48 has focused evidence only (5 runs of the handoff_atomic group).
- No test covers a re-send for an author who retired while the target was offline.
- No dispatch-level test of PeerHandoffStage and PeerRequestView routing (ht-8lem.7, .8), no in-process test of the hc worker handoff branch (ht-8lem.9), no coordinator test that a peer without peer.handoff.v1 receives no frame (ht-8lem.4), and no CLI test of inbox seat alerts (ht-8lem.21).
- Recipient ACK clearing of a handoff warning is asserted at store and service level only, not multinode (ht-8lem.15 parked).
- Process and worktree cleanup for ht-8lem.44 to .48 is incomplete (exit 4 on uninspectable non-run pids); the leak check has not run at this head.

## Deferred OK
- The HANDOFF_ATOMIC_REMOTE_V1 doc comment says "not advertised until its feature lands", but the daemon now advertises it unconditionally (stale comment).
- ht-8lem.48 committed its evidence log beside the named test file (minor [extra-file-changed]); this follows the repo's evidence convention.
- Seat-addressed alerts reach hooks and channels only as the generic attention count and need one inbox call (spec Q4 accepted consequence).
- wait_slot builds a per-call SystemClock Pacer instead of the injected clock (ht-8lem.44 minor).
- Earlier deferred items, dispositions unchanged: due-pass starvation (handoff_obligations.rs:503), no log for a seat known without a pair (:339), no daemon.log line for re-send snapshot errors (peer_handoff.rs:606), resend_pass aborts on one row's settle error, the plain-launch v3->v4 request-key rekey, stale SKILL.md launch-prompt text at :64 and :212, no dated Amended entry for TRUST-POLICY N4/N5/N15, duplicated deadline-owner trigger SQL, dead RemoteLaunch.then paths, and the idempotent stage early return.
- Kept task worktrees and /tmp/sp-e3317cf8/ht-8lem.* from the incomplete process cleanup; caller cleanup, not code.
