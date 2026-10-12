# ht-0lo.18 recovery oracle sweep fix

Status: IMPLEMENTED, source-only; parent-owned focused execution, required gates and one light review remain pending. No GREEN or completion claim is made.

Base: `dc15706ead701e609ce530c69dd170ecb6ad123c`
Head: `04cc1778ac3d710da3afbd10f3aa2543bcf75676`
Branch: `super-auto/sweep-ht-0lo-18`

## Change and scope

Corrected only `presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement` in `tests/store/attention.rs`. Expected work is now decomposed into context3, settled ordinary A16/B18 and Lazy A11/B14, with stage-local exact equalities and binding-specific failure diagnostics, followed by the retained aggregate equality: A3+16+11=30 and B3+18+14=35.

The comment explains that the old seven Lazy eligibility relations gained three canonical episode points: direct pair (`direct_threads`), exact seat (`seats`) and current invitation (`invitations`, including membership episode). The helper conservatively charges all ten even when own-cursor scope short-circuits the episode branch; the initial message key adds1 and unread hydration adds3. Neither production accounting nor the fixture changed.

The original two-binding loop, ordinary/lazy presence checks, no-ACK-candidate match, seek equality1, ACK attribution replay-A and replay-B cursor0 assertions remain intact. No canonical writes or forced private rows were added.

`TRUST-POLICY.md` has exactly one linked factual edit: the stale Lazy11 clause now says14, consistent with its already-selected direct-invitation paragraph. Ordinary pending13, maximum ordinary18, due1803 and carry579 remain unchanged. No production, schema, public/V1 bytes, bounds, deadlines, root Goal, accepted refinement or code-roast exit changes.

Files changed: `tests/store/attention.rs`, `TRUST-POLICY.md`.
Commit trailer: `Outside-adapter: tests/store/attention.rs, TRUST-POLICY.md`.
No new production model-visible strings. Added diagnostic text occurs only on test assertion failure (four short stage/total messages).

## Requirements and diagnosis

Read current `bd --sandbox --readonly show ht-0lo.18 --json` and `bd --sandbox --readonly comments ht-0lo.18`; no comments exist. Applied the current explicit amendment A30/B35 with decomposed exact charges and the single linked TRUST edit. Read the extracted brief, supplied recovery diagnosis, normative TRUST policy (especially the already-selected Q4 allowance and sibling reader/carry rules), root design Goal and accepted-refinement handoff/Q4 parent decision. Their contracts are preserved.

Read-only source inspection confirms `ordinary_due_context` charges3; ordinary exact-key settled unread costs16 or18; Lazy exact-key charges1+10 and optional3 hydration. The three episode relations use keyed canonical SQL. The existing Q4 actual-helper/SQL controls remain the independent production-work oracle.

## Verification and runtime ownership

Per dispatch runtime override, this implementer ran no Cargo, tests, plugin, runtime, daemon, target, Herdr or tracker mutation. Parent owns all runtime validation and gates. No rebase performed, as explicitly required by the override.

Source checks actually run:

- `git diff --check` exited0 before commit.
- `git show --check --oneline HEAD` printed the commit without whitespace errors.
- `git status --short` was empty after commit.
- Diff inspection shows only the named test and one TRUST sentence.

Existing actual RED read from `/private/tmp/ht-final-C-three-repro-d8427582/execution.json` and `run.log`, HEAD `d8427582d2d0bfca0ed50e5b58f29b130ee12640`, UUID `73df30b1-27c4-4d5a-a313-4fea555b0599`: the supplied three-test nextest command exited100, summary3 run/0 passed/3 failed/4939 skipped. This task's test failed in0.167s at its old total equality, left30/right27. The sibling-B branch and final canonical assertions were unmeasured in that RED. Existing final-C had the same failure. The parent evidence's scoped leak check exited0. These are baseline evidence, not new execution at this commit.

Parent focused command, NOT RUN by implementer:

```sh
nice cargo nextest run --locked --all-features --lib --no-fail-fast -E 'test(=store::attention::tests::presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement) | test(=store::attention::tests::presentation_capacity_due_exact_keys_have_bounded_actual_sql_plans_and_vm) | test(=store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers) | test(=store::queries::tests::query_work_q4::q4_episode_private_and_reconciliation_points_have_constant_sqlite_work) | test(=store::queries::tests::query_work_q4::q4_scalar_agreement_and_strict_retirement_use_exact_scope)'
```

Parent should preserve actual output, exact-source identity and scoped leak evidence, then run required format/Clippy/harness-neutral/default-feature gates before the single light review. A passing exact test must reach replay-B and final canonical checks; predicted B35 is not claimed observed here.

## Assertion falsifiers

- Context equality rejects a charged2 or4 rather than3 for either exact binding.
- Ordinary delta equality rejects A15 or B17 rather than16/18, catching omitted eligibility/hydration work even if another stage compensates.
- Lazy delta equality rejects the old A8/B11 envelope or any charge other than A11/B14, catching missing episode charges or incorrect hydration accounting.
- Aggregate equality rejects the old A27/B32 totals and any deviation from30/35; it uses explicit independent expected components, not values compared to themselves.
- Preserved ordinary/lazy presence checks reject A receiving settled/read mail or B losing its unread mail; the no-ACK match rejects a renewed ACK candidate on the settled ordinary receipt. Seek1 rejects0 or2. Final canonical assertions reject ACK attribution moving from replay-A or B's cursor advancing above0.

## Test hygiene and assumptions

No processes started, no task runtime/temp state created, no processes to stop. Runtime leak checks are parent-owned; no new leak-check result is claimed.

Initial sandboxed `git add` could not create the worktree index lock because Git administrative storage is read-only in the default sandbox. Narrow approved CLI-only `git add` and `git commit` succeeded for the two task paths/own branch. No approval-review rejection occurred, no unrelated Git ref or source epoch changed.

Assumptions: none beyond the current bead's explicit test-defect correction and parent runtime override. Concerns: new source has not yet been compiled/run; parent validation and one light review remain required before claiming the failure resolved.

## Parent runtime evidence

Actual combined validation 0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2 / tree29f7d42d742fa09a56bf64f9de1648420e2565e3: exact26 tests PASS (13.506s nextest summary, process13.737s; list/build52.117s), scoped leaks0 UUID aa07d15e-4108-41cd-8b65-f16cde5c0b5e. Exact command, selection and output: /private/tmp/ht-sweep-fix-focused-corrected-f746/execution.json and run.log. Original3 now reach full post-setup preservation assertions. Every task-changed file blob independently matches author 04cc1778ac3d710da3afbd10f3aa2543bcf75676. Combined required fmt/neutral/strictclippy/default PASS, evidence /private/tmp/ht-sweep-combined-gates-0b04/execution.json. No production change, cap/time limit adjustment or failed assertion waiver.
