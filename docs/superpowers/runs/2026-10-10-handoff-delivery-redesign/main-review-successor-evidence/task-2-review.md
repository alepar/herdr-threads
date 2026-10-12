# ht-0lo.21 light independent task review

Status: CLEAN
Base: 7ea79d3d8e598b01729b61be9c0faa31a6e046f4
Reviewed head: 8efa319f47c3c50ed30416d10a1486edc4a76bba

## Spec compliance

✅ Current `bd --sandbox --readonly show ht-0lo.21 --json` and `bd --sandbox --readonly comments ht-0lo.21` agree with the frozen Task 2 brief and design ruling; there are no comments or conflicting amendments. The diff corrects only the direct Lazy invitation wake oracle and its static explanation, with five added private selftests. Human group invitations retain their prior wake expectation. Human/person producer commands, guard setup, caller markers, busy delivery and other production code are unchanged.

Focused source check addressed the concrete risk that a human direct first-contact producer might retain legacy timed invitation semantics. `Recipient.case` → `Recipient.send` → `Recipient.coord` → `Run.ht` emits guarded `human send --seat … --lazy`; the new control exercises that actual Python chain. `src/cli/commands.rs:2351` selects DirectSend, `src/cli/mod.rs:3554` calls `direct_send::start`, and `src/cli/direct_send.rs:254` freezes Lazy creation policy, passed unchanged to Ensure at line 391. `src/store/control.rs:665` excludes Human only for RequireAck and line 678 makes Lazy untimed. `src/delivery/invitation.rs` marks ordinary Lazy invitations non-waking; `src/store/lazy_delivery.rs:891` and `:907` restrict Lazy wake work to wake_on=all. The sibling group producer remains plain human invite; `src/cli/commands.rs:1874` clears its confirmation to historical semantics. This matches the original delivery/token-efficiency contract and the design's explicit modern-direct-Lazy refinement, without a source qualification waiver.

The task's two changed-file blob IDs exactly match parent combined b1c3d9e1 (tree a71e9847fe65215b1fa7ce43ba46b1c9d241df84): wake-e2e.py 3c637db7cb36408ea957c59ba1328a8a0921b6b8; wake_e2e_selftest.py ab5acefbd5fadc3642efdc5f201a6775e254d0ae. No authenticated native qualification or release clearance is claimed. Parent compile gates and queue-task plugin results are separate integration evidence, not evidence for this driver's source correctness.

## Issues

Critical: none.
Important: none.
Minor: none.

## Test evidence

Report command: `TMPDIR=/tmp/sp-bd3546df/ht-0lo.21 python3 -m unittest discover -s scripts -p wake_e2e_selftest.py -k IdleWakeOracle`. Retained oracle-red.log shows the exact obsolete expectation and wrong acceptance of unexpected direct wake: 5 tests, 3 failures. oracle-green.log shows the same 5 tests passing after the narrow fix. Negative controls independently require unexpected nudge wake, absent all wake, and absent opening-body delivery to fail; the matrix retains group and send behavior. No existing test was deleted, skipped or loosened.

Report command: `TMPDIR=/tmp/sp-bd3546df/ht-0lo.21 python3 scripts/wake-e2e.py --self-test`. Retained selftests-green-unsandboxed.log ends with 47 tests in 11.872s, OK. The initial sandboxed attempt's 8 process-inspection failures are disclosed and retained, with the actual authorized rerun passing; the code does not bypass process checks. The final leak and cleanup logs are empty, consistent with the report's successful ownership-scoped cleanup. Evidence plausibly covers the changed driver and preserved coordinator/marker controls. No tests were run by this reviewer.

Required test-change commands were run in the assigned worktree, both successfully with empty output:

`git diff --stat 7ea79d3d8e598b01729b61be9c0faa31a6e046f4..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'`

`git diff 7ea79d3d8e598b01729b61be9c0faa31a6e046f4..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'`

Those prescribed pathspecs do not match wake_e2e_selftest.py; its entire changed hunk was reviewed in the nonempty review package (two changed files, one commit). Base ancestry check passed. Only the designated diff and this review were written; no Cargo, runtime, Herdr, source/tracker mutation or delegation occurred.
