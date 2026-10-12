# ht-0lo.14 review

Status: CLEAN
Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`
Reviewed head: `538bfd7c43d9778aa847e1c7028563b84b396a8c`

## Spec compliance

✅ The one-file test-only change accounts explicitly for the six intentional migration45 triggers alongside the unchanged eight migration42 exclusions. Historical objects retain exact name/SQL equality; the additions have six independent literal names, events, tables and full expected definitions, including missing/exhausted fence guards and increment. Unexpected objects still fail equality. No production, public wire, cap, history, design or foreign-scope change appears.

✅ No existing assertion is deleted, skipped or weakened. The corrected comparison is justified by the current bead's explicit test-defect requirement. Current `bd show ht-0lo.14 --json` and `bd comments ht-0lo.14` contain no amendment or unresolved conflict with Task 22.

✅ A focused source read of `tests/store/effective.rs:2088–2276` checked the concrete risk that a passing selector could cover only one origin or bypass later preservation assertions. The test iterates `[28, 36]`, constructs the v36 origin from published migrations, snapshots populated rows, checks sequence/FKs/ordinal, reinitializes with clock 9999, and checks frozen cutover plus v36 invitation provenance, confirmation, deadline and due-index behavior. Those fixtures and downstream assertions are unchanged by this diff.

No ⚠️ items. Combined build/project gates and serial integration remain parent-owned and pending under the dispatch contract; this verdict covers this task's scope and evidence only.

## Issues

Critical: none.
Important: none.
Minor: none.

## Test evidence

No tests, Cargo, Herdr or tracker mutations were run by this reviewer.

The report's parent stage2 command is:

```sh
nice cargo nextest run --locked --all-features --lib --test service --no-fail-fast -E 'test(=store::queries::tests::query_work_q4::q4_private_fresh_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_private_resumed_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_public_and_origin_warning_sources_remain_enabled) | test(=store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers) | test(=store::queries::tests::query_work_q4::q4_actual_continued_tail_checks_scope_once_and_charges_retirement) | test(=store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once) | test(=store::effective::tests::receipt_send_time_upgrade_failure_rolls_back_constraint_and_cutover_then_retries) | test(=presentation_owner_budget_controls::presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_pull_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_live_owner_waiters_serialize_and_succeed_after_release) | test(=presentation_owner_budget_controls::presentation_poisoned_owner_recovers_for_pull_and_confirm)'
```

Read `/private/tmp/ht-roast1-rust-stage2-corrected-741984b9/execution.json` and `run.log`: populated preservation PASS (0.466s), rollback/retry PASS (0.868s). Both v28/v36 iterations and downstream assertions run inside the passing preservation test body. The batch is explicitly 13 tests, 9 passed / 4 failed, exit 100; all four failures are separately scoped owner-waiter causal controls for the other parent task, not ignored migration failures or an overall green batch. The five source-query tests also passed. Leak check for run ID `ea337674-33a7-4cd0-9b47-18f05aa0b118` exited 0.

Read-only `git rev-parse HEAD HEAD:tests/store/effective.rs 046100d70eedc1076279426e64c5949464e09f48:tests/store/effective.rs` confirms reviewed head and parent validation source share blob `fc495dc2725591f995c866233e93a654ecad6c12`. The actual runtime evidence therefore covers the changed file reviewed here.

Retained first causal RED: `/private/tmp/ht-roast1-migration-preservation-control/{execution.json,run.log}` records `nice cargo nextest run --locked --all-features --lib --no-fail-fast -E 'test(=store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once)'` at `3abee5a6add69443e49322e96f9130b834686235`: 0 passed / 1 failed, exit 100, at the old catalog equality. The printed catalogs differ by exactly the six presentation triggers, with no historical removal. Later assertions and the v36 iteration were unmeasured in that RED run. Its leak check exited 0.

Test changes commands, run in the designated task worktree:

```sh
git diff --stat bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Both succeeded: one file, 58 additions / 3 deletions. The old single catalog assertion becomes exact historical equality plus exact six-trigger equality; no unrelated test changes. Base ancestor check succeeded, and the required review-package command produced a nonempty one-commit package.

## Strengths

The expectation comes from literals in the test rather than the migration implementation, preserving a falsifiable catalog oracle while reaching the existing preservation checks.
