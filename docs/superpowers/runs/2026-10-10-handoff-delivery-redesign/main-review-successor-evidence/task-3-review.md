# ht-0lo.22 independent task review

Status: CLEAN
Base: 7ea79d3d8e598b01729b61be9c0faa31a6e046f4
Reviewed head: 045b1b65204b1c8bbb6957f7848e9b4a874e7be1

## Spec compliance

✅ Current bead description, acceptance requirements, frozen Task 3 and brief agree. `bd --sandbox --readonly show ht-0lo.22 --json` and `bd --sandbox --readonly comments ht-0lo.22` were read; no comments or conflicting amendments exist. The parent override limits this to one light documentation review, forbids runtime/tests/Cargo/Herdr/delegation, and assigns later merge gates to the coordinator.

✅ The commit changes only docs/agent-usage.md and CHANGELOG.md. Both distinguish generic join/rejoin and nonpair acceptance's head/seat-pending policy from active direct acceptance's retained own cursors, missing open-binding event-floor seeding and independent sibling delivery. The usage guide also distinguishes bindings born during a pending invitation from fresh bindings after acceptance. Both retain the latter's seat-maximum-or-head seeding rule.

Grounding was one focused check for the concrete risk of misstating route selection or fresh-binding semantics: src/store/control.rs:1097–1143 selects the captured active direct floor before membership changes and dispatches to the direct or generic initializer; src/store/lazy_delivery.rs:252–278 validates that exact active pair invitation and obtains its actual event floor; src/store/read_cursors.rs:90–104,145–188 implement fresh joined binding, generic join and direct acceptance rules. TRUST-POLICY.md:851–904 agrees that own-stream delivery is separate from frozen legacy pending, acceptance is not display, siblings remain independent and a binding born after acceptance retains normal seeding. No whole-branch audit was performed.

No ⚠️ items.

## Issues

- Critical: none.
- Important: none.
- Minor: none.

## Test evidence

The report records `git diff HEAD^ HEAD --check` with exit 0 and empty output, `git status --short` with exit 0 and empty output, and `git rev-parse HEAD` yielding the reviewed commit. Documentation-only scope and the explicit parent override make the absence of runtime tests appropriate. No test processes were started; cleanup is inapplicable. Parent merge gates remain outstanding and are not claimed by this review.

Test changes commands executed in the task worktree (both exited 0 with empty output):

```sh
git diff --stat 7ea79d3d8e598b01729b61be9c0faa31a6e046f4..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff 7ea79d3d8e598b01729b61be9c0faa31a6e046f4..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Test changes: none. The base ancestry check passed; review-package succeeded and contained one commit and both changed files. The diff was read in full.
