# Q4 physical query controls — scratch proposal

2026-10-11. Proposal only while the revised episode design and production interfaces are pending. No source/test/TRUST edits, Cargo/build/runtime SQL, Git/index, Herdr or nested agents. Parent owns authorization, causal RED, the Cargo lane, architecture interfaces and test-file ownership. This document does not claim the candidate's stated HEAD/tree or any runtime outcome.

Read current production Lazy predicates, exact due lookup, reconciliation and private raw/continuation source, plus the episode architecture and Q4 spec/quality assessments. The architecture's refined accepted assumptions are three added unique pair/seat/invitation probes, same-snapshot message scalar reuse, and a separately charged fresh-source counter. Those are requirements to verify, not existing measured facts.

Source provenance read:

- `src/store/lazy_delivery.rs`: `be9775f973992ba97c7a40633563a58531727a5f9a37edc866216046b07dbce0`
- `src/store/queries.rs`: `44f092cbb44f6a11a30c93ef3066e515b5dc64cae27dd1becb1904ba49435d24`
- `src/store/invitation_content.rs`: `014b31080186266aa2d5f03db99ff5d13ac493c60bba1f79cf9262df5ebb4d62`
- `src/ports.rs`: `0dd3de6092764630a7f7648cf75630e0fe81be5a50beba3ec7db71c21bb79d79`
- RUN `resume-v090-q4-direct-lazy-reader-episode-design.md`: `916dbfd3787c6d161943ba5ecd067cdd5bd96ab1f7b3646dfb0e51cc274bd852`

## Ownership and seam proposal

Use the existing `tests/store/queries.rs` module for physical shared-predicate/scalarized-reconciliation SQL witnesses; its selector namespace is `store::queries::tests`. No new target, process-wide hook, failpoint or shared counter is needed. The parent must assign this file exclusively before any applied patch. Existing `tests/service/delivery_pull.rs` ownership should remain with the behavioral/counter author: that module proves real pull/reconciliation outputs,256-key fairness and charged counters. Floor/accept/display mutations belong to their owning tests and do not acquire inferred boundedness from these SQL witnesses.

Require the actual composed production SQL, not a copied model of it. Preferred seams are production-owned `pub(super)` constants/functions for the final fresh Lazy eligibility query, exact Lazy due query, reconciliation canonical message lookup and scalarized Lazy reconciliation query. The shared predicate should accept explicit validated thread/sequence/publication scalars; no synthetic messages alias or hidden message lookup should restore the removed relation. Existing legacy, joined and own-cursor alternatives must be exercised through the real composed expression. If architecture prefers no constants, source-extract its exact literals/macros and log the full production-source SHA256 at test execution; parent reviews the extraction against actual invocation and binds. A source hash is provenance, not a substitute for behavior.

Do not author a compiling patch against invented API names while the seam is pending. This plan is the scratch deliverable. No existing counter oracle changes are proposed before actual new-production failures.

## Proposed controls and discriminating expectations

### `presentation_q4_lazy_episode_scope_exact_keys_have_constant_sqlite_work`

Mutation caught: replacing any exact pair/seat/current-invitation key with latest/history/seat-wide scanning, probing membership twice while claiming reuse, or broadening cursorless eligibility past the current episode.

Use the complete migrated store fixture with foreign keys, all constraints and triggers. The Q3 incomplete extracted-DDL fixture failed setup on missing referenced canonical tables; do not repeat it or disable triggers to hide invalid fixture relationships. Establish the selected pair/current membership/invitation and actual message/manifest/recipient facts via canonical writers where available. Additional bulk unrelated records may use canonical-shaped fixture SQL only after its writer invariants are reviewed, with setup excluded from the measured region.

Compare **the same100 exact candidate keys** under unrelated populations128 and8192. Grow these dimensions together first, and independently if a result changes:

- unrelated immutable pairs/threads with this seat as an endpoint, so a seat-wide pair lookup cannot hide behind other seats;
- unrelated current membership/invitation pairs for this same seat;
- retained older invitation episodes on selected threads, with the exact current episode unchanged;
- unrelated bindings and messages/recipient keys.

Do not grow the selected candidate slice or its scalar values. Use 25 candidates in each of four literal semantic classes: current-episode cursorless invited pair succeeds; pre-episode body fails; foreign/nonpair unjoined invitation fails; terminal voluntary/current-invitation scope fails. These four classes fix the100-candidate workload. Separate small controls cover retained own cursors, joined readers, absent cursor plus stale episode, retired seat, mismatched endpoint/instance, and frozen legacy_pending top-level/subagent behavior. Expectations are literal booleans from the documented reader rule, not recomputed with the same SQL.

Capture `EXPLAIN QUERY PLAN` for the **actual full composed eligibility SQL**, SQLite version, source hash, `VmStep`, `FullscanStep` and result for each selected key. Assert no source-relation SCAN or temp sort, exact indexed SEARCH paths for each relation, and0 fullscan steps. Identify aliases explicitly: two separate cursor point reads are two accesses to the same relation and must be counted honestly. Pair uses UNIQUE `direct_threads.thread_id`, seat its ID key, current invitation the `(thread_id,seat_id,episode)` unique key, membership its exact pair key. Fail if invitations are selected by ordinal/latest, if membership is accessed a second time without accounting, or if an unintended source relation appears. Fixed virtual/constant sources, if architecture permits any, must be reported separately and never mislabeled indexed relation probes.

For each identical case/key, demand equal observed VM work across unrelated cardinalities; do not compare different short-circuit branches to each other. Equality applies to this linked-SQLite VM witness, not elapsed time/page-cache/disk I/O or a universal B-tree complexity theorem. A case-dependent plan may change with legitimate query redesign; review its actual physical access rather than weakening the bound silently.

### `presentation_q4_lazy_reconciliation_reuses_canonical_message_scalars`

Mutation caught: retaining `JOIN messages` in the second query but charging it as removed, losing canonical thread/decision identity when passing scalars, or treating missing/ambiguous addressing as terminal capacity evidence.

Execute the actual first canonical message query against an exact selected ID/instance/kind and capture its mode/thread/sequence/decision row. Execute the actual scalarized second query in the **same transaction/snapshot**, with the exact binding and recipient. Repeat against128/8192 unrelated populations. Do not accept caller-supplied scalars as authority and do not fetch a second message through a helper outside the measured scope.

The plan inventory must show one messages access in the first query, **zero messages accesses in the second**. The latter has six inherited accesses (recipient,manifest,binding,two cursor points,membership) plus pair,seat,current invitation: nine indexed relation accesses conservatively, even if SQLite short-circuits. Combined source-accounting charge is ten. Record actual statement VM/fullscan data independently from that conservative counter; an EXISTS result or `+=9` is insufficient proof.

Hand-derived scalar agreement matrix:

| Fixed canonical facts / input mutation | Expected observation |
|---|---|
| Exact modern published addressed body, matching canonical scalars | Returns the literal expected unread/terminal result for the chosen current-episode/cursor case. |
| Wrong requested instance or nonordinary kind in first query | No canonical message tuple; second query must not run in the production caller; offer retained. |
| Exact tuple, missing recipient or manifest | No exact addressed row; offer retained, never capacity released on a false EXISTS alone. |
| Same message ID but mismatched recipient/manifest thread | No matching canonical tuple; offer retained. |
| Manifest decision differs from captured canonical message decision | No matching canonical tuple; offer retained. |
| Foreign binding/seat or changed exact owner scope | No matching addressed reader; offer retained or existing canonical refusal, never sibling terminal proof. |
| Current-episode body with absent cursor / valid invitation | Unread, not reclaimed. |
| Retained own cursor below old body despite later episode | Unread, not reclaimed. |
| Own completed cursor | Canonically terminal for that reader; reclaimed without receipt/acceptance evidence. |

For mismatch fixtures preserve canonical schema constraints; prefer exact SQL bind perturbations or a clearly labeled integrity fixture over impossible public states. Do not assert that a public writer can create a contradiction it actually prevents. The control must distinguish SELECT-row absence from an unread boolean false: the current reconciliation code uses `unread==Some(false)` for terminal proof.

**Pending scalar detail for architecture:** the sequence is trusted because it comes from the first canonical message row in this snapshot. Does the second query additionally check Lazy manifest `base_sequence==captured sequence`, or is no sequence equality required beyond that provenance? Query-bind poisoning is meaningful only for identity checks the production contract actually requires. Do not demand a nonexistent sequence guard merely to produce a negative test; instead prove the real caller obtains and forwards the first row unchanged, and that no arbitrary scalar API or second lookup supplies it. Thread/decision/message/instance identity agreement is mandatory. Parent has been asked to settle this distinction with architecture.

### `presentation_q4_lazy_reconciliation_full_snapshot_has_bounded_physical_work`

Home: existing service/delivery_pull controls owner, coordinated through parent. This complements, rather than replaces, point-SQL controls.

Run actual presentation reconciliation with256 distinct Lazy known keys and a frozen ordinary-source mode. Include unread keys at the lexical front and completed terminal keys at the tail, with missing/foreign mismatches retained. Assert the full expected reclaimed ID set and all256 visits, including terminal suffix progress; no first196 truncation masquerading as a new cap. For256 valid Lazy keys, ten conservative charged probes per key yields2560. A mixed ordinary/Lazy fixture preserves mode-specific counts and the same cap. Actual `count_vm_units` wraps the real store call to catch hidden helper work beyond individually measured SQL; compare the same snapshot/outputs against small and large unrelated populations, excluding fixture setup and separate earlier priming. Additional passive/control/reconciliation work is listed separately, not hidden under the source slice.

Existing due/carry/owner-replay controls should retain1803/579 allowance and independent reconciliation2560. A failed old oracle earns adaptation only after the parent attributes its actual first failure to changed legitimate accounting.

### `presentation_q4_lazy_fresh_and_continuation_charge_scope_before_filter`

Home: real service/store source-mode controls owner. Keep scope work in the new architecture-approved fresh-source counter, whose exact name is still pending.

Use100 raw private Lazy candidates, all current-episode rejected or mixed with a small renderable suffix. Verify100 physical candidate visits before filtering, at most300 added pair/seat/invitation probes, and conservatively three charges for every performed shared scope evaluation. With100 evaluations the charge is300 even if no body renders. Eligible-body hydration/output remains capped32; do not confuse output count with source visits. Due/carry/reconciliation counters cannot absorb or double-charge this new work.

Add a long UTF-8 pinned body case: one actual continuation scope evaluation is charged3 in this counter and consumes the appropriate existing raw-candidate allowance; pure encoded-size fitting/binary search performs no new canonical SQL. Compare before/after own cursor movement during continuation without applying a fresh unread-sequence test to the already captured tail. Acceptance/current-episode rejection/reinvitation changes must use exact captured message/publication identity. Preserve the separate ordinary continuation allowance15 and all existing output/carry/partial bounds. Parent must verify actual continuation execution, not infer it from the fresh branch's passing plan.

## Accounting and limits to report

- Due Lazy: mode1 + eligibility(7+3) + hydration3 =14 conservative keyed probes; ordinary maximum18 unchanged. Due1803 and carry579 stay valid with setup charged separately.
- Lazy reconciliation: message1 + optimized eligibility(7−1+3)=10;2560 unchanged only if the redundant messages access is actually absent and identity remains exact. No unbounded reinvitation/history/source scan may hide behind scalar reuse.
- Fresh private/continued Lazy scope: at most3 added probes per actual checked candidate;100 candidate envelope gives300, charged separately and before output filtering. Short-circuiting does not erase conservative accounting.
- Fixed event-floor reads are not per-message scope work. If later delegated to this author, verify all three exact UNIQUE event keys and fixed-size in-memory earliest valid sequence selection in a separate control; mutation fanout/walk costs remain inherited and explicitly outside the private100-candidate bound.
- Public Lazy queries may filter before LIMIT and enumerate binding candidates; full completion can walk many messages after the baseline. These controls establish neither a100-row public/completion bound nor an end-to-end operation latency bound.
- No VM/runtime result exists for this proposal. Actual source constants, compiled selectors, causal RED, physical measurements and behavioral counters remain parent-owned future gates.

Scratch proposal is quiescent. No applied patch or source mutation accompanies it.
