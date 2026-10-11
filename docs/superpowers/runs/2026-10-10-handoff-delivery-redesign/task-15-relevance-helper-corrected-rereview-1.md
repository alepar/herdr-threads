# Corrected relevance helper: frozen cumulative rereview1

Spec CLEAN

Quality CLEAN

These are source-review verdicts for the isolated helper patch, not execution results, whole Task15 approval or integration readiness.

## Exact reviewed scope

Frozen clean helper HEAD `367da744905c1df11dbdf72d811c133e6bdf9e21`, baseline `308564b887e1d0f4af866c69a5ed40275c860b69`, in `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.1.1`. Read the complete narrow correction and new fixtures, current report/checkpoint and inherited full cumulative review against the selected `70ce105d8c621d254b4880a39c33c85731e01471d7435653979dac514fb145c1` contract. Independently compared actual git diffs with supplied artifacts:

- Full BASE→367 diff:154432 bytes, SHA256 `2f76269cef0ca8b9945f9e82c4efc00a149bed65346f65f90b5d92a4f113c6ea`, exact byte match.
- Full previously reviewed0129→367 delta:6345 bytes, SHA256 `c7ef2266bf7748e5b5b6d09a1c65622f4585eb60d4ee5ca6e626f720d5e6ca73`, exact byte match.
- Report SHA256 `6adb63206559a9a01a253906b36cb2055acd1e76fda685aa1aa91a43679e7759` matches.
- Migration41 is byte-identical to the full-reviewed0129 migration. Exactly the same three cumulative paths remain; the delta changes only the helper Membership branch and existing attention tests.

The prior full `task-15-relevance-helper-corrected-review.md` remains the full source/schema/callee/test inventory for unchanged cumulative code. Its sole blocking I1 is superseded by this rereview's resolution below; its scope limitations and execution/caller/TRUST prerequisites remain. No Cargo/runtime/SQLite VM/test execution, source application, source modification or empirical PASS claim was made here.

## I1: ADDRESSED

Membership now receives both current cutoff and current high from its existing same-transaction evaluator. The effective query bound is `min(captured_high,current_high)`. A cursor above that bound performs the existing negative authority-union transition, charges the already performed keyed work and does not issue a membership range. Equality issues the existing LIMIT1 empty range and completes negatively. A cursor below the bound can visit only an interval within both current authority and the captured finite frontier; its joined/left test uses current cutoff. Consequently first acceptance cannot use an interval that exact witness validation rejects solely for exceeding the current source high-water.

Continuation deliberately retains the original captured high and monotonically advances after; it cannot expand the captured finite walk. Subsequent current-source checks can narrow the effective range again. Negative generic direct authority proceeds to prepared alternatives through existing finish_negative; known prepared authority completes its own candidate. No invalid after>high cursor is persisted by the early-negative branch. A later source mutation remains visible through the existing fence/recapture contract and exact witness checks.

The correction adds no query, authority, table, API, hook, field, budget, source class or producer change. It can make a reserved fallback step cheaper. The scheduler still charges actual work and rotates a completed step; protected source/fallback opportunities and lending are unchanged. The same Points context and64 cap remain around preflight, reload and evaluation. Range work stays within the same100/26 meter. No normal producer decrease is asserted: this closes the retained/operator-repair/future-writer current-input gap identified by I1.

## Test delta reviewed

New `relevance_suspend_legacy_membership` reads the actual qualifying interval and persisted suspended slot8 cursor/high, rather than guessing ordinal positions. It requires Unknown while obtaining an unfinished cursor strictly before the sole qualifying interval. `presentation_relevance_suspended_membership_respects_repaired_current_high` independently sets current high to cursor-1, cursor and cursor+1; all three stay below the qualifying interval, require every subsequent result not Open and require finite Absent. These cover cursor above, equal and below effective bound. The unchanged-high control uses the same suspension, requires finite Open, verifies the exact qualifying witness ordinal and requires subsequent Open recheck. These tests use existing relevance_pull counter assertions and explicitly identify repair rather than canonical-producer mutation.

All prior eleven fixtures and assertions are byte-preserved by the exact delta. There are now thirteen source-reviewed fixtures. The new tests and corrected earlier tests are UNRUN in this review; fixture plausibility is not GREEN evidence.

## Full cumulative source/schema disposition

The original global-positive finding remains ADDRESSED: exact direct warning/job thread/event/source association or exact service publication/recipient independently establishes boolean relevance; same-ID prepared alternatives do not erase it. Positive source/membership/witness authority ignores unrelated global fence changes. Current condition, source identity, receipt/waiver and frozen recipient machinery remain exact; I1's missing current membership bound is now included.

The original logical-close omission remains ADDRESSED: shared eligibility checks physical clear and the exact retained-chain successor for every actual receipt/unavailable condition route and witness before Open. Invitation conditions receive no fabricated sweep. Sole canonical sweep insertion retains adjacent scope ranges and has no production cleanup/deletion; the bounded successor is complete under the selected normative chain invariant. Migration41 still has87 identical guarded fence hooks covering29 input tables, and private proof/cursor/output writes remain excluded. Missing/negative/MAX fence refusal and atomic rollback behavior are unchanged.

All13 indexed physical source definitions, source LIMIT-before-judgment, generic prepared LIMIT1 fallback and historical interval LIMIT1 remain unchanged. Finite captured ranges, separate persisted rotations, protected SOURCE2/2+FALLBACK3/3 reservation, full-cost50/50 shares and alternating lending still share100 rows/26 seeks, including coverage and initial13 highwaters. Typed source/fallback Points<=64 and aggregate<=6400, separate witness<=64, fixed admission/proof/fence/native metadata probes and D descriptor<=2048 accounting remain unchanged. No hidden legacy effective-warning/receipt/window or history judge was added.

The proof still has fixed13 records, fixed tagged authority/judgment, optional single witness, bounded identifiers, deny_unknown_fields and32768 byte checks in Rust/SQL. One row per retained seat/fixed consumer, seat FK cascade, consumer-delete reclaim, exact claim mismatch recapture, cancellation/transaction admission, rollback/restart and no self-invalidating proof persistence remain as fully reviewed. Projection/bootstrap pending-only rules and immutable migrations1..39 are unchanged. No additional source/schema correctness finding was identified in this frozen cumulative patch.

## Remaining mandatory composed obligations

D must prove exactly ONE advance per successful V2Pull on fresh/replay/control/no-output paths and reuse its same-transaction coherent proof. ExplicitConfirm must do NO advance/revalidate/source/fallback/coverage/nonunique relevance walk; diagnostic revalidate remains outside that authority. Descriptor keyed allowance cannot conceal ranges or a second relevance meter.

Before Task15 can be promoted: execute actual corrected churn and prepublication-close RED→GREEN, the new repair/control tests and retained fixtures; obtain actual linked SQLite upgrade/startup/query-plan/VM and work/fairness evidence; review the final composed D source/schema/interfaces and unchanged V1/capability/consumer lifecycle behavior; perform required checks. The selected source-union clarification, normative sweep-chain future-writer/operator-repair rule, accepted bootstrap/index cost and absence-delay/lifecycle limits must be recorded in TRUST in the same integration commit. This isolated helper's CLEAN does not assert those D obligations complete and does not authorize a merge.
