# Q1 cancellation baseline — not ready

The first private unread-body correction is V090 `5a7ce5a3`, tree `268aa3fd79a206f0de0968f8152e355afb9e22c9`. Source gates passed: fmt 7.030 s, neutral 1.735 s, strict all-target/all-feature clippy 41.278 s, default-feature check 21.308 s. Six exact continuation controls passed in 0.790 s (enumeration 20.385 s), scoped leak check 0, run ID `143ebd46-5688-4e91-9adb-82baf27fb189`. This does not clear the independent review findings.

Fresh spec and quality reviews found a final-save cancellation gap and an insufficient sibling content assertion. Review artifacts are in `.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-q1-continuation-{spec,quality}-review.md`. The old unbudgeted decision wrapper could commit private progress after cancellation. The existing SQL-abort rollback test did not cover cancellation.

Tests-only commit `89a430fb75859e7a273f8629f8fd9e1c2962be93`, tree `b75e77e0caa8341f136e3adea412a8bf21c1a61f`, preserves production byte-exact at 5a7. Parent ran two exact combined controls; 0 PASS / 2 FAIL, run 0.500 s, enumeration 4.747 s, scoped leaks 0, run ID `725027ab-d1f3-45db-85db-92fd1a6c5eda`. Actual logs: `/private/tmp/ht-C-v090-cancellation-causal-baseline/`.

`continued_body_cancellation_before_final_save_rolls_back_and_retries` reached the assertions proving its per-fixture clock fired at the old pre-save check and its actual token was cancelled. The old implementation nevertheless returned Unchanged and committed one ordinary visit with 15 continuation probes. The failed expect_err is causal evidence for the cancellation defect. A budget-aware decision correction is authorized; no unchanged retry or weakened budget.

The strengthened sibling control failed earlier, in public AdvanceCursors with malformed JSON at helper line 1233. This is a fixture failure, not a sibling-retirement causal result; it must be repaired before assessing its content assertions.

Actual local Main is now `23801c98ece1a8c437420d6cd823c5780993f386`, independently reviewed and locally merged according to fully displayed message `m4ihW7qQi` (coordination #1050). Origin remains b6dd5dd1. Main still owns the unresolved first-run lane_wiring DeadlineExceeded qualification failure; no release/install readiness is inferred. The composed candidate has not imported these latest changes yet; published migrations 1–35 remain immutable.

C remains in progress. Revised Q2 controls, final composed inventory execution, code roast and exact-tip readiness remain pending.


## Final scoped correction

Tests-only6ca651ea preserved production and repaired fixture receipt provenance JSON. Actual two-control baseline: sibling exact-content PASS; Decision-phase cancellation FAIL at expect_err after UTC/fired/token assertions. Run ID c987ce12-ae8a-4d3e-a511-41d469c86bc7; wrapper1.488s/leaks0. This is a second causal witness after the tentative save.

Minimal correction6cfb3180 changes only presentation_pull to the existing budgeted decision wrapper, passing the live call budget. Tentative writes roll back through decision acceptance; accepted storage outcomes retain existing independent commit semantics. Actual eight exact controls PASS1.730s (wrapper2.125s/list38.717s), run633b9ed6-ae76-4fdb-8254-8611e6afd0a2/leaks0. Fresh independent Spec and Quality both PASS scoped. The old fifth-clock-call witness can cancel earlier with the new callbacks; the independent Decision-phase control verifies the final boundary on both versions.

Frozen18ea source gates all pass: fmt14.617s/neutral4.125s/clippy69.133s/default22.997s. Lint exceeds the one-minute budget in this run; final controlled incremental measurement is pending, with no speed-budget waiver or ambient-load cause claim.

Main238 composed cleanly in7f1879f9, then expiry negative71bb imported separately4c0d9d37 and duplicate-parent-guard optimization342 imported9a573c3f. Current candidate9a573c3f/tree5158b885c8988b1c3ebf5b96047751a6e4949212. The optimization is not a proven fix for historical expiry failures: the duplicate-guard mutant also passed. Revised Q2 runtime, canonical schema/binding controls, complete expanded matrix, code roast and exact-tip readiness remain pending.
