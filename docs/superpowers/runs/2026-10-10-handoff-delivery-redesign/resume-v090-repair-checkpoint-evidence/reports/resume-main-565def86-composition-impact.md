# Main565 first-unit materialization impact

Status: read-only inspection complete; QUIESCENT. No import, source/index mutation, Cargo, test/runtime invocation, Herdr operation or release/install action.

## Frozen source and coordination facts

Parent supplied fully displayed message `mCuslDHjq #1054`: Main565 reviewed, merged and pushed; qualification run `38185603361` pending; no install. Earlier `mX3QmY3ZR #1051` identified the causal first-unit quantum repair. This subagent read/ACKed no thread.

Independent local Git observations, repeated at inspection end:

- `main` and remote-tracking `origin/main`: `565def869ae50e47e6da7388ea333229050eaf8d`.
- Composition HEAD: `94a7c539491006782497db9af4af8f2610c0806b`.
- Merge base: `23801c98ece1a8c437420d6cd823c5780993f386`.
- Composition worktree/index clean.

The matching local remote-tracking ref corroborates the parent source fact; no network query was performed. Qualification success and installation are not established. Parent's active focused run belongs to its frozen composition source and is unaffected by this read-only inspection.

## Exact change and predicted conflict

Main `23801c98..565def86` changes only:

- `src/store/materialization.rs`: calculate elapsed as before, but enforce the 5 ms quantum only when `units > 0`.
- `tests/store/materialization.rs`: add `StallClock`, whose monotonic reads advance by 10 ms, and `stall_before_first_unit_still_does_one_unit_within_call_budget`.

Production patch applicability check succeeds against composition94. Composition's separate 114-line receipt deadline backfill addition precedes `advance_work`; preserve it. The production hunk is otherwise unchanged since Main238 and needs no semantic rewrite.

The test patch applicability check fails at the common EOF append. Nonmutating `git merge-tree --trivial-merge 23801c98 94a7c539 565def86` confirms **one expected content conflict**, at the tail of `tests/store/materialization.rs` after `send_attention_completion_stamps_completed_at`:

- Ours: `unavailable_send_materializes_due_deadline_without_availability`.
- Main: `StallClock`, its `Clock` implementation, and `stall_before_first_unit_still_does_one_unit_within_call_budget`.

Smallest resolution: retain the entire existing unavailable-send test, close its function, then append Main's complete StallClock and new test. The three-way conflict shares a closing brace outside the marker span; ensure both test functions remain complete. Do not choose an entire side. Also preserve composition's two existing `(available=1200, deadline=1300)` expectations; reverting them to Main's historical deadline1500 would discard candidate A's original send-time clock semantics.

No other source path is changed by this Main commit.

## Causal behavior and limits

Before: admission samples `started`, loads the durable job, then checks the per-unit CallBudget and the 5 ms scheduling quantum. A delay/preemption between sampling and the first quantum check can exceed 5 ms despite unexpired CallBudget. No unit runs, and the existing zero-unit/no-failure branch returns `DeadlineExceeded`, which the work lane exposes as a failure and retry delay.

After: the CallBudget/cancellation check still precedes every unit; `elapsed` is still sampled exactly once each loop. While no unit has run, elapsed scheduling quantum alone cannot manufacture that error. One bounded unit can run, then the next loop yields when elapsed is at least5 ms. `DurableWorkAdmission` remains 1–16 units; successful per-unit savepoints, rollback of a failed unit, committed-prefix cursor/count checks, durable diagnostics and completed-at behavior remain unchanged.

This is a work scheduling progress repair, not relaxed CallBudget admission. A CallBudget already exhausted before a unit still prevents that unit. As before, a unit admitted before the deadline can take time to finish; neither version provides hard intra-unit interruption. The patch intentionally allows an initial unit after the cooperative 5 ms quantum elapsed. It does not promise that the entire transaction, including a preemption/job load, finishes within 5 ms.

Main's commit traces this bug to the older work loop, before session bindings, and identifies `lane_wiring::injected_lane_failure_logs_and_degrades_then_clears` first healthy assertion as the observed symptom. That provenance is a commit/source claim, not a reproduced runtime result from this inspection.

## Interaction with candidate A and D

- **D work budgets:** `invitation_content`, `presentation_relevance`, and `delivery_pull` do not call `materialization::advance_work` directly. Their source, exact-key, hydration, sizing, outbox, carry and physical-replay limits are not changed. No additional unit is admitted beyond the existing durable-work cap, and no D counter limit should be raised for this import. The shared writer can now spend one already-bounded unit on a previously delayed job rather than record false zero-work exhaustion.
- **Publication:** Send manifests and immutable publication identity are unchanged. The worker can advance existing `send_attention`, warning attribution/close and timer projection jobs sooner under preemption; it neither publishes a new send on its own nor changes manifest audience/history. The new regression uses warning attribution, not a send or ACK.
- **ACK/provenance:** No receipt ACK writer, `ack_binding_id`, observation provenance, binding cursor, offered ledger or presentation confirmation changes. Existing `project_receipt` pending-only upsert and effective receipt authority remain in place, so earlier materialization is not evidence of receipt ACK or model consumption. Keep the existing `ack_before_unavailable_warning_attribution_keeps_history_without_wake` oracle.
- **Candidate A deadlines:** Preserve the independent backfill implementation, frozen original deadline, unavailable-recipient due projection and changed1300 expectations. Main565 does not alter deadline values, restart clocks or change availability semantics.
- **Retirement:** Seat retirement uses the separate `control::advance_retirement` path and retains its own quantum, budget, atomicity and diagnostics. `Kind` in materialization has no retirement variant. Materialization's existing retired-recipient checks remain unchanged. D's ordinary logical reclamation, physical owner replay and invitation outbox retirement are likewise untouched; no binding can be inferred settled merely because materialization progressed.
- **Health:** Existing worker progress/error interpretation remains intact. A successful prefix can clear its actual work-job failure through the normal progress path; the patch does not bypass Health, suppress a recorded error or change the injected lane failure test's healthy/degraded assertions.

## Published migrations

Compared Git blobs for all35 released `migrations/0001_*`–`0035_*` files at b6, Main238, Main565 and composition94. Every map has35 files and identical blob IDs. Main565 changes no migration. No schema edit, compatibility alias or TRUST provenance change is indicated.

## Parent-owned verification selectors after an eventual merge

New exact unit selector (lib test module verified from the source):

`nice cargo test --locked --all-features --lib store::materialization::tests::stall_before_first_unit_still_does_one_unit_within_call_budget -- --exact`

The new oracle requires exactly one processed/committed unit, position1, remaining work, no error and one warning recipient despite each clock read advancing10 ms. It would distinguish the prior zero-unit error without a real-time sleep.

Existing nearby oracles to preserve/check: `expired_call_budget_returns_committed_prefix_with_error`, `first_unit_failure_is_durable_and_retried_at_same_cursor`, `ack_before_unavailable_warning_attribution_keeps_history_without_wake`, `published_manifest_warning_and_first_anchor_project_without_new_decision`, and composition's `unavailable_send_materializes_due_deadline_without_availability` (all under `store::materialization::tests`).

Observed integration symptom selector: `--test integration lane_wiring::injected_lane_failure_logs_and_degrades_then_clears -- --exact`. Its child/process/leak handling remains subject to project rules. No command in this section was run here.

Import remains parent-owned after its active lane is quiet. Existing focused evidence must keep frozen94 identity; it cannot be relabeled as verification of a later merged tree. The source fact is neither a new permission requirement nor a pause request. This report is the only file written.
