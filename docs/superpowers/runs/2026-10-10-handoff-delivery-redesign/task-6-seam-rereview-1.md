Spec CLEAN
Quality CLEAN
I1 ADDRESSED
I2 ADDRESSED
I3 ADDRESSED

Scoped static rereview: 3f9167bd90cf19ddbd0b75f1282674628f607e43..f48857d73b18c76b8ee35b0f8cab5589a6f4ea5c. No new Critical/Important/Minor findings. The previous overall CLEAN withdrawal remains recorded in task-6-seam-review.md.

- I1: src/store/schema.rs:3825 restores Option<i64>, exact Lazy→NULL classification and unchanged Option tuple propagation. The downstream deadline-none branch now suppresses retrospective overdue events for Lazy, including direct settlement/retirement paths. Added actual accountable late acceptance and bounded retirement regressions assert terminal provenance/state and no overdue condition/event/native/mod origin wake (tests/store/control.rs:11025,11078,11151).
- I2: src/store/schema.rs:4073 restores exact canonical origin resolution, frozen invitation origin instance/seat INSERT and original inviter warning-recipient/wake-work projection after the existing transition enqueue. Independent source/probe consumers from the prior package remain unchanged. Original inviter attribution no longer depends on current membership or informational frontier.
- I3: src/store/schema.rs:3637,3695 restores condition_id selection and invitation-only passive original-inviter clear-recipient insertion after existing transition enqueue. Clear creates no added wake work; existing cutoff/high-water snapshot handling remains untouched.
- Independent original→corrected preservation audit: compared a04d1a41..19afa478 against eac57640..f48857d7. All approved runtime schema added/deleted lines exactly equal this correction's24insertions/8deletions: six hunks at U3/seven at U0. All original schema migration/version groups were separately accounted for: constants33/34/latest34, fresh1..29 plus inherited30/31 FK wrappers and32 backfill, supported18..34, then additive33/34 transactions/audits. Both renamed audit function bodies match approved originals after only expected name/version substitutions. Both receipt/handoff FK-wrapper function bodies exactly match integration BASE. Thus the correction adds the approved invitation runtime changes without replacing inherited receipt/backfill logic.
- All five overlap mapping checks independently repeated: TRUST-POLICY.md, src/store/effective.rs and tests/store/schema.rs original added/deleted lines exactly match corrected integration deltas; schema runtime/audit checks above; the active invitation capability regression body matches the original exactly, and prior reviewed dynamic expected inventory composition retains inherited receipt capability behavior. No new migration/schema/catalog fixture changes occur in this fix; previously reviewed immutable SQL/catalog evidence stands.
- Mandatory test audit: git diff --stat 3f9167bd..f48857d7 -- tests shows one file/161insertions/0deletions. Capped400-line preview contains the entire test diff. Read complete13435-byte fix package and updated report; no remaining omitted fix hunks, weakened assertions, skipped/ignored tests or raised budgets. Both new tests invoke production store paths through the established canonical bound fixture; retirement asserts completion and actual retired state instead of only an internal predicate.
- Author evidence records two actual failures on broken3f9167bd before restoring production: direct Lazy gateway and original inviter projection. Correctedf48857d7 is explicitly uncompiled/unrun at this review. No Cargo/runtime actions were performed or claimed by this reviewer. Fresh focused43-test selection/run and exact-tip gates remain coordinator work. This static approval resolves I1/I2/I3, not final runtime readiness or downstream shared-driver fairness.

Spec CLEAN
Quality CLEAN
I1 ADDRESSED
I2 ADDRESSED
I3 ADDRESSED
