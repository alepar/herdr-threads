# Q1 cancellation baseline — not ready

The first private unread-body correction is V090 `5a7ce5a3`, tree `268aa3fd79a206f0de0968f8152e355afb9e22c9`. Source gates passed: fmt 7.030 s, neutral 1.735 s, strict all-target/all-feature clippy 41.278 s, default-feature check 21.308 s. Six exact continuation controls passed in 0.790 s (enumeration 20.385 s), scoped leak check 0, run ID `143ebd46-5688-4e91-9adb-82baf27fb189`. This does not clear the independent review findings.

Fresh spec and quality reviews found a final-save cancellation gap and an insufficient sibling content assertion. Review artifacts are in `.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-q1-continuation-{spec,quality}-review.md`. The old unbudgeted decision wrapper could commit private progress after cancellation. The existing SQL-abort rollback test did not cover cancellation.

Tests-only commit `89a430fb75859e7a273f8629f8fd9e1c2962be93`, tree `b75e77e0caa8341f136e3adea412a8bf21c1a61f`, preserves production byte-exact at 5a7. Parent ran two exact combined controls; 0 PASS / 2 FAIL, run 0.500 s, enumeration 4.747 s, scoped leaks 0, run ID `725027ab-d1f3-45db-85db-92fd1a6c5eda`. Actual logs: `/private/tmp/ht-C-v090-cancellation-causal-baseline/`.

`continued_body_cancellation_before_final_save_rolls_back_and_retries` reached the assertions proving its per-fixture clock fired at the old pre-save check and its actual token was cancelled. The old implementation nevertheless returned Unchanged and committed one ordinary visit with 15 continuation probes. The failed expect_err is causal evidence for the cancellation defect. A budget-aware decision correction is authorized; no unchanged retry or weakened budget.

The strengthened sibling control failed earlier, in public AdvanceCursors with malformed JSON at helper line 1233. This is a fixture failure, not a sibling-retirement causal result; it must be repaired before assessing its content assertions.

Actual local Main is now `23801c98ece1a8c437420d6cd823c5780993f386`, independently reviewed and locally merged according to fully displayed message `m4ihW7qQi` (coordination #1050). Origin remains b6dd5dd1. Main still owns the unresolved first-run lane_wiring DeadlineExceeded qualification failure; no release/install readiness is inferred. The composed candidate has not imported these latest changes yet; published migrations 1–35 remain immutable.

C remains in progress. Revised Q2 controls, final composed inventory execution, code roast and exact-tip readiness remain pending.
