super-roast verdict: clean (2 nits)
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (local multi-agent messaging daemon). The change is test-only (tests/multinode); blast radius is CI signal quality, not production behaviour.
inputs: bead/ht-i3pc@8a280c9c4 vs main@dc6ef629f
coverage: unavailable — inline run, no coverage object · lanes: correctness, test-causality-lifecycle, test-isolation-environment, test-suite-cost (inline)
independence: none (inline) · rung: inline

## Confirmed findings

None.

## Rejected (with reason)

- [Should-fix] config_auto_ack case 3/4: turning "still tracks the deadline" into a wait weakens the negative check.
  Rejected: receipt_due_sparse rows for the sender are removed only when the receipt leaves pending (trigger
  receipt_due_sparse_update). If a read-only read or the partitioned display had settled the receipt, the row
  would never reach 1 again and wait_due_rows would time out, so the test still fails. The other assertions
  (is_pending on both nodes, no receipt.settled kind) still run first, unchanged.
- [Should-fix] The race hides a product defect: the deadline scan could miss a pending receipt before its projection.
  Rejected: the due row is projected by the send's own send_attention work job on the deadline lane
  (DeliveryTail::message -> materialization::send_unit -> project_receipt), which commits within the same lane's
  progress. The worst case is overdue detection delayed by projection latency, not a missed warning. This is the
  documented lazy-materialization design (`effective_receipt_of` doc: "A pending obligation need not have a
  physical receipt_state row").
- [Nit] assert_settled_once's `due_rows == 0` could race the same way.
  Rejected: it runs after wait_receipt(node 0, "acked"), which reads the physical receipt_state; the due-row
  delete trigger fires in the same transaction that makes that row acked.
- [Should-fix] config_direct: waiting for pending on node 0 could pass after the ACK.
  Rejected: the ACK (text inbox) comes after this loop, so pending can only be the pre-ACK state.

## Unverified nits (spot-checked)

- [Nit] tests/multinode/config_auto_ack.rs:105 `due_rows(node 1) == 0` is checked straight after node 1's effective
  pending, so it may run before node 1's projection exists. It still holds (the trigger never writes due rows for a
  message whose origin_node is remote) but is a weak check. Pre-existing; left as is.
- [FYI] Tests outside tests/multinode that read receipt_state right after a send were not swept. The pattern
  (effective-view wait, then an immediate physical read) is the thing to look for if another receipt flake appears.

## Escalations (need human)

None.

## Evidence

- CI run 40710373544, job 122893832371: panic at tests/multinode/config_auto_ack.rs:103, "the sender node tracks the
  pending deadline", left 0, right 1.
- A temporary 500 ms sleep before project_receipt's INSERT (reverted) reproduces that exact panic on the base test;
  the fixed tests pass twice under the same delay.
- Plain stress, nice 19: base 3x6 runs of config_auto_ack:: 18/18 pass (no natural repro); fixed 2x5 10/10 pass; leak check clean.
