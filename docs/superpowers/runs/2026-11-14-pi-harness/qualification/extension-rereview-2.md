Machine-specific command paths are redacted as `HEAVY TOOLBOX` and `<worktree>`; review verdicts are unchanged.

# Task 1 scoped re-review, round 2

Spec compliance: R1 ADDRESSED in the frozen patch. Code quality: Approved, conditional on pending final validation and committed-patch identity verification. No new Critical, Important or Minor finding in this fix diff.

Scope: base `8ba848027`, frozen uncommitted snapshot `.superpowers/sdd/implementation-plan/review-fix2-frozen.diff`, coordinator-supplied diff-payload SHA-256 `760e9475b61399330a38b645072b88097946c94b9f856a7b820c9107e305fd9b`. This is a scoped R1/new-breakage review, not a whole-branch review or completion approval.

## R1 — ADDRESSED

`integrations/pi/extension/herdr-threads.js:365` clears each recovered item's predecessor-local `waking` witness during successor-owner construction. The accepted proposal/body/custom-type identity remains intact. The successor still needs a current canonical waking replay to acquire intent, after which the existing same-owner witness suppresses unchanged reoffers. The change does not reset same-owner rejection history or widen receipt authority.

The callback regression at `integrations/pi/extension/tests/extension.test.mjs:501` exercises an accepted but unobserved urgent frame across reload. It asserts metadata-only and passive-replay silence, exactly one content-free wake on current urgent replay, repeated-reoffer silence, no duplicate native body frame, and no ACK until exact successor context observation. This directly covers R1 and protects the I1 behavior that originally exposed it.

The added actual-SDK path at `integrations/pi/extension/tests/sdk.test.mjs:115` uses cancellation after a completed boundary proposal, native `session.reload()`, repeated canonical urgent replay, exactly one successor run, one stored body frame and one eventual exact ACK. Its runtime result is still pending; these are checks present in the frozen test code, not a claim that the SDK case passed.

## Findings

- Critical: none.
- Important: none.
- Minor: none.

## Evidence and acceptance conditions

- Read the appended/prepended round2 evidence section. Targeted R1 RED reports `0 !== 1`, exit 1. The six-case focused GREEN reports six passes, zero failures, exit 0, covering R1 plus alternating urgent keys, no-progress suppression, saturation and detached-watch shutdown/reload. Production source was unchanged after this GREEN according to the report.
- The report explicitly states that the additional passive-replay assertion and new actual-SDK cancellation/reload case were added after the six-case GREEN. Both are present in the reviewed frozen diff. Their final combined callback/native-IO/SDK validation is QUEUED, not complete.
- Final acceptance remains conditional on the coordinator checking the complete combined test result, the scoped leak check for `pi-task1-fix2-20261114-1634`, and that the committed patch equals this frozen snapshot. Prior-round passing results do not substitute for those checks.
- Read the frozen diff once. No tests/builds rerun, changed-file rereads, broader source inspection, source/index mutation, subprocess/service operation, credentials, threads operation or subagents. Only this review report was written.
- Existing Rust/daemon integration and real TUI/provider qualification remain separate later-task gates.

Code quality: Approved conditionally. The repair resets the misplaced owner-local witness at the same lifecycle boundary as the other recovered execution state, while preserving native frame deduplication and context-gated receipts. Final task completion remains pending the explicit validation conditions above.
