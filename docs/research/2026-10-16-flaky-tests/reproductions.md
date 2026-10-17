# Reproduction evidence

Checkout base: 82786da05; branch fix/v2-flakes. Controlled reproductions deliberately choose an allowed scheduler ordering; they do not identify the exact historical scheduler trace.

## Hook wake

Original clear unread test passed unchanged once locally. Kept new SB2 pane `agent_status=working` from before its SessionStart until the hook completed, then returned it to idle. Original test still required native prompt after the mark and failed after 21.67s: `timed out waiting for a native wake prompt`, with the same `last_outcome=unsafe` family as hosted CI. The complete hook body had already been delivered and its receipt ACKed, so no idle wake was needed. Temporary reproduction became a permanent short-hook ACK test that checks no later wake. The separate native-wake test now uses a body exceeding the hook envelope and requires a still-pending receipt.

## Watch close

Added `presentation_watch_lifecycle_close_wins_over_inflight_refusal` before changing production code. Fake client returns either typed Pull SessionMismatch or Confirm StaleBinding; real watch consumer logic sees pending Attention then Close with BindingChanged or Replaced. Before fix: assertion failed `left: 1`, `right: 0`. After bounded terminal-frame drain: regression plus existing presentation-watch unit filters passed 12/12. Existing failed/stale confirmation test still checks no truncation success hint. This reproduces a product protocol ordering defect in the current V2 path, not loss of canonical mail. The older ht-7ykh report came from v1.2.0; its exact schedule and legacy-path root cause were not reconstructed, so it remains a historical open flake rather than a claim that this V2 fix proves that old failure resolved. JS consumer drops local queue on closing, uses backoff [1,2,5,10,30] for generic error, marks session suspect for binding_changed, and stops on replaced exit3. Frequency and replacement ping-pong remain unmeasured/unreproduced. Separate product bead ht-x0b9; coordinator approved non-blocking 2.0.1 patch candidacy.

## Lifecycle startup

Before fix, baseline two crashed_* tests passed 10/10 stress iterations. Temporary `sleep 6` added to the isolated lifecycle launcher reproduced both original errors: HostUnavailable five-second readiness timeout, and `before_cleanup=None` instead of `Some((false,false,false))`. After using the already-existing LIVENESS guard in those convergence waits and fixture gate, both passed under the same delay in 6.68s. Temporary sleep removed from final source. The tests still verify old metadata retirement, exclusive owner lease, node identity continuity and new boot identity; only host-dependent liveness guards changed. Final repeated focused tests exercise normal startup.

## Restore host

New deterministic `starting_node_after_store_only_peer_attaches_its_host` starts the isolated node with no remembered host, stops fixture tracking, then starts a store-only daemon contender before calling Cluster::start_node. Before fix: herdr_hosts count Some("0") vs expected Some("1") at tests/multinode/config_restore.rs:64 (1.25s). After explicit public host-scoped request in start_node, the host attaches before the fixture releases new peer admission. Historical log had store-only startup, equal vectors and 12 parked dependencies. Code tracing: own seat creation maps HostUnavailable to WaitFor::Host, seats require herdr_hosts, and existing host-attachment apply retry already handles release. No replay algorithm modification justified.

## Commands and transient captures

Focused normal repetitions: `nice cargo nextest run --locked --all-features -E 'test(mod_delivery::clear_) | test(config_restore::) | test(daemon::lifecycle::tests::crashed_) | test(presentation_watch_lifecycle_close)' --stress-count 5`.

Final result: 5/5 stress iterations passed, ten tests each, 37.925s. Final two iterations overlapped full-suite startup. Temporary captures during this session: /tmp/ht-clear-red.log, /tmp/ht-watch-red.log, /tmp/ht-lifecycle-red.log, /tmp/ht-lifecycle-delayed-green.log, /tmp/ht-watch-green.log, /tmp/ht-flakes-focused-green.log. These scratch logs are not canonical long-term artifact locations; this document preserves the meaningful interventions and observed outputs.

## Follow-up: remote read before replication (ht-41yy)

Hosted run 39003998464, job 117312603907, tree 82786da05 failed the immediate remote inbox assertion in `sweep::created_thread_invites_unseen_seats_and_delivery_follows_the_contract` (tests/multinode/sweep.rs:125 before fix). Source node send completion proves its own commit, not recipient replication. Diagnostic `cluster.partition(0, 1)` immediately before the p0 send reproduced the exact assertion failure in 3.41s. After adding a wait for B's applied message row and healing immediately before that wait, the same test passed through reconnect in 7.38s. Temporary partition/heal removed from the final patch.

The require-ACK case in the same test waited only for node A's pending receipt, which exists at its own send commit. It now additionally waits for B's pending receipt before ACKing on B. Both waits use existing bounded fixture condition helpers; inbox/ACK/wake assertions and timeout constants remain unchanged. This is a test precondition issue, with no product defect established. The final follow-up validation is recorded in validation.md; it does not imply the earlier full-suite run covered a later test edit.

## Follow-up: pair creation overlaps an audience fence (ht-2b3p)

Hosted manual run 39004001353, job 117312615474, tree 82786da05 returned `send preparation snapshot changed` from C's direct send in `config_relay::pair_auto_join_facts_relay_through_a_node_with_no_pair_seat`. This is an intended conservative refusal, not evidence of a product regression: the fact-kinds design section16 expressly retains membership, lifecycle, eligibility and config fences; src/store/messages.rs rejects stale captured revisions before publication. Existing `send_fence::membership_lifecycle_eligibility_config_bumps_still_conflict` passed 1/1. The direct-send recover hint alone is not proof that any arbitrary retry will resolve a conflict; this fix does not depend on recovery behavior.

The original relay test passed 20/20 repetitions under four owned `nice -n19 yes` CPU loops; all four PIDs were stopped and reaped and the run-scoped leak check passed. The exact hosted CLI conflict was not reproduced. Incoming A pair/auto-join facts changing C's preparation fence is consistent with source and the error, but the exact historical revision/interleaving is not established.

A stronger fixture precondition was then tested: before C opens independently it must still have no pair thread. Temporarily allowing convergence after A's send deterministically failed that assertion, Some("1") vs Some("0"), in1.75s. This reproduces the missing independent-open prerequisite rather than claiming to reproduce the exact CLI refusal.

Final fixture waits for initial convergence and C-B linked state, cuts C-B, awaits its backoff state, opens A and C while isolated, asserts C has not learned A's pair before C opens, then heals C-B. This follows the existing crossed-pair fixture's causal pattern. All original canonical ID, joined-once, no invitation, message, relay authorship, no-wake and convergence assertions remain. No production code, timeout changes, retries or generic error suppression were added. Independent review found no must-fix issue and confirmed the cut kills existing pumps and blocks new ones.
