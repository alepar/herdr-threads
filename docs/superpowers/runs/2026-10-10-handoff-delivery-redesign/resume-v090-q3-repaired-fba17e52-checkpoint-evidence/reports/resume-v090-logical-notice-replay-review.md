# Logical Notice replay review

Verdict: **PASS — static production proposal review.** No required production revision found. Compilation and behavioral GREEN remain the parent's validation gates; neither was run by this reviewer.

Scope: current candidate `src/store/invitation_content.rs`, `resume-v090-logical-notice-replay-production-proposed.patch`, the author's `resume-v090-logical-notice-replay-proposal.md`, relevant canonical schema/writers, and TRUST-POLICY. The proposed additive service test was read for coverage, not executed. Review method: super-roast inline, restricted by the assignment's no-nested-agents rule; no independent judge panel. No production, test, Git, index, Cargo, runtime, Herdr or configuration mutation was performed. This report is the only file written.

## Causal evidence

`/private/tmp/ht-C-v090-notice-replay-diagnostic-33c74793/run.log` shows the exact warning `w0af6e0a6-63f5-813d-9e84-7a63938466f3` and thread `tgN8gyP6y` retaining published decision 8, warning offset/count 1/1, digest event `(8,1)`, recipient generation 8 and frontier 2. Physical warning rows are null before and after retry. The canonical binding ID, generation and execution persist through the allowed reconnect; retry_checkins is zero. The output comparison drops only Notice while preserving Invitation and Attention. The current physical-only branch at `invitation_content.rs:1655` directly explains this loss. A7 expressly allows bounded restreaming of a frozen outbox after its notice frontier advances.

## Exact authority predicate

The logical fallback requires all of the following, with no caller-selected discovery:

- The captured warning and thread match the canonical recipient digest at `(claim.seat, warning)`.
- Its event sequence selects a published manifest in the admitted claim's instance; the manifest thread must match the captured thread.
- The digest offset is positive, within the manifest warning count, and selects the same warning in that exact published preparation.
- The canonical warning recipient is the admitted seat and its generation equals that publication decision.

An unpublished recipe, unrelated preparation carrying the same warning ID, foreign instance/thread/seat, mismatched digest event/offset or mismatched recipient generation cannot satisfy the fallback. Publication plus exact prepared recipe supplies canonical built-in event provenance; the digest or a sparse recipe alone does not. Current membership, current condition actionability and the already advanced offer frontier are correctly absent: this is replay of an allocated historical informational event, not fresh Attention or receipt authority.

The helper's seat/instance arguments are protected by existing `binding_scope` admission (`invitation_content.rs:264`), exact input binding comparison before replay, and binding/consumer-owned load. The stored generation/execution/instance reset rule prevents a changed binding scope from inheriting an old outbox. No new binding provenance gate, sibling selection or seat continuity inference is introduced.

The physical branch is exactly the old captured `id + thread_id + kind='warn'` predicate. Its existing acceptance behavior is preserved. The logical branch adds the missing canonical representation; it neither materializes a message nor edits historical provenance.

## Bounded work

Each Notice adds at most five keyed relation probes, including the existing physical message probe:

| Relation | Bound established by existing schema |
|---|---|
| messages | Primary key `id` |
| digest_programmatic_warnings | Unique `(seat_id, warning_id)` |
| send_manifests | Explicit `send_manifests_instance_decision` unique `(instance_id, decision_seq)` index |
| prepared_unavailable_warnings | Unique `(preparation_id, warning_offset)`; exact warning ID is additionally checked |
| warning_recipients | Unique `(warning_id, seat_id)`; event generation is additionally checked |

The CROSS JOIN chain prevents a warning-ID-wide prepared-history traversal: each preceding relation supplies at most one row and the next exact unique key. There is no membership fanout, condition walk, hydration, fallback range or LIMIT-after-filter scan. `checked_masks` bounds the entire retained descriptor vector at 32 before iteration, so the conservative Notice allowance is **160 relation probes per replay**, with fewer Notices in mixed pages. This is in addition to existing separately bounded invitation hydration and relevance work, not a claim that all replay work totals 160. The normal allocator currently captures at most 16 Notices, but the safe proof uses the checked 32-descriptor limit.

`work.replay` continues counting descriptors, not SQL relations; the proposal and this review do not reinterpret it as a five-probe runtime counter. No counter ceiling is relaxed. This bound is established by source and unique-index inspection; no EXPLAIN or runtime probe measurement was performed.

## Captured token and Confirm

The patch changes only applicability in `replay`. A valid Notice retains the original frozen descriptor, slot, body, order, token and render identity. It adds no frontier settlement, logical offer, acceptance, receipt ACK, health effect or wake.

Confirm remains unchanged (`invitation_content.rs:825`): exact binding/consumer/current-or-last token admission and complete captured-control subset validation precede effects. Noncontrol slots are consumed from the original descriptor vector; omitted control stamps remain unconfirmed in their original slots. Partial retirement does not compact the vector. Completed-token handling retains only captured control stamps and masks. Adding the new lookup to Confirm would be unnecessary and conflict with the policy that explicit confirmation performs no discovery/fallback/revalidation walk.

## Validation handoff

The author's additive deterministic test exercises the relevant loss and captures full-page equality, physical-row absence, unchanged publication/digest/recipient/frontier/binding/receipt state, foreign-token refusal, partial original-slot masks, full confirmation and duplicate confirmation. The registered failed-stdout regression provides the actual writer/flush failure boundary. Run the author's tests-only RED, then production GREEN and listed controls under the parent's authorized validation process. This PASS approves the minimal proposal's logic and bounds; it does not substitute for those executions.

No schema, version gate, history rebuild, new provenance, TRUST-POLICY weakening or counter relaxation is required.
