# Task ht-0lo.15 review

Status: CLEAN.
Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2` (ancestor check passed).
Reviewed HEAD: `3b1113f0d1938ee065324f39d7590b250a4058a7`.

## Spec compliance

✅ Current `bd show ht-0lo.15 --json`, its empty comments, and frozen brief agree. No amendments apply. The diff stays within the three specified files and implements one budget-aware acquisition helper shared by V2 Pull/Confirm, without wire, capacity, canonical history or foreign-scope changes.

✅ The helper checks cancellation/deadline before acquisition and again after successful or poisoned acquisition. Contention waits through the cancellation token for at most the smaller of remaining time and 10 ms; exhausted waiters exit before admission. Poison recovery retains the same mutex guard.

✅ Both methods bind the returned guard in their existing method scope. It remains held through admission, canonical store calls, and successful ledger/replay or confirmation progress installation. The original permit selection remains intact.

✅ Tests use a real canonical invitation/token, observe the exact held registry owner, check bounded return before release for both cancellation and deadline on Pull/Confirm, compare canonical snapshots, and verify the token remains confirmable. Both Hook and Watch execute on GREEN. Successful contention and poison controls remain successful; replay application asserts the exact permit is held.

## Issues

Critical: none.
Important: none.
Minor: none.
⚠️ items: none.

## Focused reachability and semantics check

Risk: a shared-boundary fix could be unreachable from a real V2 command, leave a sibling route blocking, release its owner before progress installation, or use a cancellation wait with a lost wakeup.

Checked `src/service/dispatch.rs:1426`–1428: real V2 command dispatch invokes `presentation_pull` and `presentation_confirm`. Searched those two files for route calls, owner acquisition and progress installation; both acquisition sites use the helper at `src/service/delivery_presentation.rs:97` and `:341`, with no superseded blocking owner lock left in either route. Read the existing store/progress portions: Pull's store call and replay installation remain within the guard scope; Confirm's store call and attention/ledger updates do likewise. Checked `src/protocol/time.rs` cancellation and budget definitions: cancellation wakes the blocking wait with ordered flag inspection, and the deadline/cancellation check is reevaluated each iteration. No correctness issue found.

## Test evidence

No tests, Cargo, Herdr, source/Git/tracker mutations or delegation were performed by this reviewer. The designated package script generated a nonempty four-commit review package.

The report contains the exact targeted GREEN command:

```sh
nice cargo nextest run --locked --all-features --test service --no-fail-fast -E 'test(=presentation_owner_budget_controls::presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_pull_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_live_owner_waiters_serialize_and_succeed_after_release) | test(=presentation_owner_budget_controls::presentation_poisoned_owner_recovers_for_pull_and_confirm)'
```

Read parent execution JSON and run log in `/private/tmp/ht-roast1-owner-green-3b1113f0/`: six tests passed, 93 skipped, test summary 1.209 s, run exit 0, leak check exit 0. Coverage is relevant and plausible. Source inspection confirms each regression/control loops both Hook and Watch, and assertion failures would fail its test. `git diff 1bf42f80ffa2cb7ea2438fb05d12b7c26f02d57a HEAD -- src/service/delivery_presentation.rs tests/service.rs tests/service/presentation_owner_budget_controls.rs` produced no output, independently verifying all task-file blobs match the parent's validated commit.

The report's corrected causal baseline records four expected before-release timeout FAILs and two passing contention/poison controls on unchanged production; the initial E0624 is explicitly a fixture diagnostic, not causal RED. The RED Hook-first limitation is recorded rather than glossed over. GREEN covers both consumers and no-effect assertions.

Parent's updated gate evidence supersedes the report's earlier pending status: `/private/tmp/ht-roast1-four-fix-gates-1bf42f80/execution.json` records exit 0 for `cargo fmt -- --check` (4.523 s), `scripts/check-harness-neutral` (1.329 s), `nice cargo clippy --locked --all-targets --all-features -- -D warnings` (21.672 s), and `nice scripts/check-default-features` (8.777 s), all at the exact validated task-file blobs.

Test changes commands, both successful in the task worktree:

```sh
git diff --stat bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Result: two test paths, 514 added lines; new registered owner controls only. No existing test deletion, skip, loosening, or expectation alteration.

## Strengths

The production change is narrow, retains exact permit identity and guard lifetime, and is supported by causal regression evidence plus real serialization and poison controls.
