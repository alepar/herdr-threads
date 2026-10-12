# ht-0lo.21 implementation evidence

Status: IMPLEMENTED
Base: 7ea79d3d8e598b01729b61be9c0faa31a6e046f4
Head: 8efa319f47c3c50ed30416d10a1486edc4a76bba
Branch: task-ht-0lo.21
Worktree: /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.21
Stacked: false. Fresh assigned branch; no rebase per parent override.

## Implementation

Corrected only the driver's idle wake oracle: invite/1:1 uses a first human direct Lazy send, so it is passive under wake_on=nudge and wakes under all. Existing send-lazy behavior and human group-invitation wake expectation remain. Updated the static driver explanation/comment accordingly. Preserved Main's human namespace, guarded person coordinator setup, CALLER_SESSION_ENV and all registered-marker stripping unchanged.

Added five private IdleWakeOracle selftests exercising actual Recipient.case, coord, send and Run.ht with fake helper results/recipient observations: exact human --seat --lazy first-contact command; all 16 idle action/channel/wake_on combinations; rejection of unexpected direct/nudge wake; rejection of absent direct/all wake; and passive direct opening delivery still required. Group invitation command is checked to remain plain human invite without a new confirmation policy. Existing coordinator/guard/all-marker controls all pass.

## Independently traced canonical semantics before implementation

Read the current task bead with `bd --sandbox --readonly show ht-0lo.21 --json` and `bd --sandbox --readonly comments ht-0lo.21`: no comments/amendments. Read plan Design ruling and /private/tmp/handoff-delivery-main-merge-review.txt; verified against the assigned base's source rather than adopting the review as an oracle.

- scripts/wake-e2e.py Recipient.coord emits human through actual Run.ht; Recipient.send supplies --seat and --lazy for invite/1:1. Human priority does not rewrite delivery mode.
- src/cli/direct_send.rs start freezes creation_policy Some(EnsureCreationPolicy{Lazy,None}) without a Human exception. retry sends that same frozen policy and claim in EnsureDirectThread.
- src/store/control.rs ensure_direct_inner stores lazy confirmation and sets timed=false. Its Human exclusion is only RequireAck, so Human Lazy is accepted by this producer.
- src/delivery/invitation.rs InvitationContext::wakes returns false for non-required Lazy invitation.
- src/store/lazy_delivery.rs note_published inserts wake_work only for seats with wake_on='all'; has_wake_rows likewise requires all. The opening Lazy message supplies the all-mode wake work, not a nudge obligation.
- src/cli/commands.rs human parsing clears confirmation only for MutationSpec::Invite (the group route), retaining the group's legacy/timed invitation. This does not affect DirectSend's separate creation policy.

Thus no production change or fabricated receipt was needed. The private controls prove driver command/oracle behavior; they are not live daemon/native qualification.

## RED / GREEN evidence

Every test ran from the worktree above with TMPDIR=/tmp/sp-bd3546df/ht-0lo.21. Logs are retained inside its git-ignored .worktrees/task21-evidence/ directory.

RED, after adding tests and before changing scripts/wake-e2e.py:
`TMPDIR=/tmp/sp-bd3546df/ht-0lo.21 python3 -m unittest discover -s scripts -p wake_e2e_selftest.py -k IdleWakeOracle`

oracle-red.log:
```
FAIL: test_idle_matrix_preserves_group_invite_and_send_modes (case='invite/1:1', wake_on='nudge')
AssertionError: True != False
FAIL: test_lazy_direct_invitation_is_passive_under_nudge
AssertionError: True != False
FAIL: test_unexpected_direct_wake_fails
AssertionError: 'PASS' != 'FAIL'
Ran 5 tests in 0.004s
FAILED (failures=3)
```
The true/false failures expose the exact obsolete oracle. Unexpected wake wrongly PASS supplies a separate negative control. Assertions after a failing assertion were unmeasured in RED; they were executed in GREEN.

GREEN same command after the narrow fix, oracle-green.log:
```
Ran 5 tests in 0.004s
OK
```

Final entire-driver check:
`TMPDIR=/tmp/sp-bd3546df/ht-0lo.21 python3 scripts/wake-e2e.py --self-test`
Run with narrowly approved escalation for process inspection, selftests-green-unsandboxed.log:
```
Ran 47 tests in 11.872s
OK
```
Exit 0. Read all output. The existing failure-path tests intentionally print fake abort/cleanup errors and fake credential-refresh guidance; all tests pass and all credentials are FAKE_AUTH under private temporary roots. No profile or real credentials were read. `git diff --check` exits 0. Postcommit `git status --short` is empty.

The first sandboxed entire-driver attempt, selftests-green.log, completed with `Ran 47 tests in 26.031s; FAILED (failures=8)`: pgrep returned 3 / sysmond service unavailable due to sandbox process-list restrictions. The unchanged command subsequently passed with process-inspection permission; no production/selftest code was changed to bypass those checks.

## Test hygiene

Prescribed scoped stop-run-processes --check was invoked after RED, focused GREEN, sandboxed selftests and final selftests from /private/tmp (the script refuses execution from a root being scanned). Final --check ran with process-inspection permission, exited 0 and emitted no matches; leak-final.log empty. Final ownership-limited stop-run-processes also exited 0 with no matches/survived lines; cleanup-final.log empty. No task-owned process remains. One initial --check from the worktree was refused by the script's self-protection, then immediately rerun from /private/tmp; this is not a harness permission refusal.

## Files / size / scope

Changed only scripts/wake-e2e.py and scripts/wake_e2e_selftest.py. Commit trailer: Outside-adapter: scripts/wake-e2e.py, scripts/wake_e2e_selftest.py.

New runtime model-visible strings: 0 bytes. No delivery/wake framing, prompt, marker or repeated boilerplate changed. Static driver Expected paragraph is 232 -> 260 UTF-8 bytes (+28), read as script documentation, not injected each turn. No queue/body caps apply to this oracle-only change.

Toolchain provenance: /opt/homebrew/bin/python3 is the standard-library runner; no project Python environment/install step exists. Actual driver module resolved inside the assigned worktree at scripts/wake-e2e.py; fake selftest imports that file directly.

## Assumptions and concerns

Applied current bead, frozen brief, design ruling and parent overrides together; no conflicting amendment. Required Cargo, harness-neutral and default-feature gates remain the parent's responsibility. No Cargo, real daemon, Herdr, authenticated native agent, profile/plugin qualification, tracker write, real-config write, Main change, push or stash was performed. This report does not claim release/runtime clearance. Authenticated release matrix is still required before tagging under project policy.
