# Capacity design scoped rereview — round 1

2026-10-11. **CDR-1 ADDRESSED. Spec PASS; Quality PASS for this design delta.** No new blocking defect found in the revision. This is design clearance only, not production implementation or runtime verification.

Compared `capacity-design.md` against preserved `capacity-design-round1.md` and the original CDR-1 finding. The source semantics supporting that finding remain those reviewed at immutable `finish-capacity-design` baseline `3520d8fbf2ded578b19c4da9cd981812c6ab6e2c`; no production source was changed or executed during this rereview.

The revised positive-capacity predicate is exactly `ordinary_runnable = normal_pending`. Once the captured normal walk ends with empty carry and its signal consumed, a conservative missing due ID no longer requests another quantum. A later independently admitted fresh pull may invoke the inherited retry behavior, but the completed pass itself no longer generates an infinite chain of such calls. This directly closes the reclamation-plus-missing-ID counterexample without dropping the ID, changing its offer stamp, or inventing settlement.

The revision also correctly distinguishes source-mode selection capacity from final remaining capacity. A normal allocation consuming its last free slot switches the final continuation predicate to the finite exact-key due set. Unknown carry and unfinished ordinary state alone then remain parked. A distinct due key still requests the next exact-key call. If capacity remains positive, the finite normal pass governs continuation. Replay allocates no new IDs, uses post-reconciliation capacity, and shares the same rule; ordinary debt is not restored through nonordinary outbox_more.

The added regressions cover the required boundary cases: terminal reclamation with conservative missing due IDs and no replacement; initially nonfull missing-ID state; outstanding outbox and confirmation; final normal page retaining capacity; and last-slot consumption both with and without distinct due work. Their planned assertions preserve unchanged stamps, finite fixture-derived pull bounds, same-call reclamation, one ordinary source per call, and exact parked-state retention. These are test requirements, not reported passing results.

The three parent dispositions now match the original review's conditions: bounded process-local per-consumer cache with incarnation/full-stamp merging; 1303 relation probes plus 100 availability seeks as an explicitly declared envelope requiring linked-SQLite proof; and standalone-unrenderable deferral with the topic-only-shortening retry limit explicitly accepted and required in TRUST-POLICY.md in the implementation commit. No extra query or invalidation scope is introduced by this revision.

The delta does not change cache lifecycle, canonical eligibility, source modes, SQL/schema/wire/harness boundaries, reconciliation budget, physical433/ordinary100 bounds, 32-block/65536-byte output cap, sizing352 bound, carry155731 cap, or partial49415 cap. Original implementation verification obligations remain in force.

Only this rereview artifact was written. No Cargo, formatting, lint, SQL execution, source/Git/Herdr/tracker mutation, subprocess service, or subagent was used. **COMPLETE + QUIESCENT.**
