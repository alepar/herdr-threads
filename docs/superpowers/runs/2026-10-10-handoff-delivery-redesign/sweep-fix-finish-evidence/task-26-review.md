# ht-0lo.18 light review

- Status: CLEAN
- Base: `dc15706ead701e609ce530c69dd170ecb6ad123c`
- Reviewed author HEAD: `04cc1778ac3d710da3afbd10f3aa2543bcf75676`

## Spec compliance

✅ Current `bd --sandbox --readonly show ht-0lo.18 --json` and `bd --sandbox --readonly comments ht-0lo.18` were read before the frozen brief; no comments exist. The current explicit requirement selects decomposed A3+16+11=30/B3+18+14=35 and permits only the named test plus the stale TRUST Lazy11→14 clause. The frozen Task26 section and brief agree; no requirement conflict was found.

✅ The package contains one commit and exactly those two paths. The named recovery test retains both bindings, exact total equality, seek1, presence checks, no-ACK-candidate check, ACK attribution to replay-A and replay-B cursor0. New stage-local exact equalities make compensating accounting errors detectable. The TRUST edit changes only11→14; ordinary18, due1803, carry579 and production remain unchanged.

✅ This is a justified oracle correction, not an expected value copied from implementation alone: the current task expressly selects these totals, and the already-existing normative direct-invitation clause at TRUST-POLICY.md:1613–1623 separately selects three indexed episode points and Lazy14. Existing independent Q4 SQL-plan/VM controls exercise canonical relation points and retained-history scaling; they were not changed here.

No ⚠ items.

## Issues

- Critical: none.
- Important: none.
- Minor: [validation-warning] `/private/tmp/ht-sweep-combined-gates-0b04/clippy.log:1` and `default.log:1` contain `nice: setpriority: Operation not permitted`. Commands nevertheless exit0; these checks passed without the requested scheduling priority. No source correction is needed; retain this environment fact with validation evidence.

## Test evidence

The implementer report accurately describes source-only work and prints the proposed targeted command:

```sh
nice cargo nextest run --locked --all-features --lib --no-fail-fast -E 'test(=store::attention::tests::presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement) | test(=store::attention::tests::presentation_capacity_due_exact_keys_have_bounded_actual_sql_plans_and_vm) | test(=store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers) | test(=store::queries::tests::query_work_q4::q4_episode_private_and_reconciliation_points_have_constant_sqlite_work) | test(=store::queries::tests::query_work_q4::q4_scalar_agreement_and_strict_retirement_use_exact_scope)'
```

Per the runtime override, its earlier pending wording is supplemented by actual parent evidence, rather than treated as missing execution. Read `/private/tmp/ht-sweep-fix-focused-corrected-f746/execution.json` and `run.log`: actual argv is `nice cargo nextest run --locked --all-features --lib --no-fail-fast --success-output immediate -E <the recorded exact 26-test expression>`, including every test above. Summary:26 run/26 passed/4917 skipped, exit0,13.506s. The named recovery case passed in0.244s and therefore reached both bindings and the retained final canonical assertions. Scoped leak check `scripts/check-no-leaked-processes --run-id aa07d15e-4108-41cd-8b65-f16cde5c0b5e` exits0.

Validation HEAD `0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2`, tree `29f7d42d742fa09a56bf64f9de1648420e2565e3`, carries byte-identical changed-file blobs to the reviewed author HEAD, verified with `git rev-parse HEAD:tests/store/attention.rs HEAD:TRUST-POLICY.md 0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2:tests/store/attention.rs 0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2:TRUST-POLICY.md`: attention `522d463e77e90c47cc1e96bc787b366ddb73abe8`, TRUST `04a91c7c4781717ad0a6f0a54dbb107ded4ad38f` for each revision.

Read `/private/tmp/ht-sweep-combined-gates-0b04/execution.json` and the textual lint/default/neutral logs. All required gates exit0: `cargo fmt -- --check`4.499s; `scripts/check-harness-neutral`1.316s; `nice cargo clippy --locked --all-targets --all-features -- -D warnings`22.406s; `nice scripts/check-default-features`8.878s. No Cargo or tests were run by this reviewer.

Test-change commands actually run, from the designated worktree:

```sh
git diff --stat dc15706ead701e609ce530c69dd170ecb6ad123c..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff dc15706ead701e609ce530c69dd170ecb6ad123c..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Both exit0. Only tests/store/attention.rs changes,30 additions/1 deletion. No deleted/skipped tests or weakened preservation assertions; changed expected accounting has the explicit independent selected-contract justification above.

## Focused source check

Risk checked: an accounting correction could conceal a production regression or weaken sibling-reader preservation. Read the exact-key helpers at src/store/queries.rs:7486–7593, the existing normative clause at TRUST-POLICY.md:1613–1623, existing Q4 indexed SQL/VM and actual-helper controls at tests/store/query_work_q4.rs:347–410 and566–675, and the unchanged final canonical assertions immediately after the diff. Context3, settled ordinary16/18 and Lazy11/14 agree with the selected conservative envelope, with no production hunk or changed caps. No shared boundary, authority or durability primitive is modified, so retirement/caller tracing is inapplicable to this test/prose-only change.

## Strengths

Stage-local equalities retain a strict, binding-specific falsifier for each component while keeping the original behavioral and canonical-state checks.
