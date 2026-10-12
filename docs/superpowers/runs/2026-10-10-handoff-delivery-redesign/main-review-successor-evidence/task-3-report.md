# ht-0lo.22 implementation evidence

Status: IMPLEMENTED
Base: 7ea79d3d8e598b01729b61be9c0faa31a6e046f4
Head: 045b1b65204b1c8bbb6957f7848e9b4a874e7be1
Branch: task-ht-0lo.22
Worktree: /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.22
Stacked: false

## Implementation

Changed only docs/agent-usage.md and CHANGELOG.md. Qualified generic join/nonpair acceptance's head and seat-pending behavior; documented active direct acceptance's preserved own cursors, missing open-binding event-floor seeding, independent sibling stream, and normal fresh-binding-after-acceptance seeding. No runtime behavior, policy or design changed.

## Independent grounding and expected oracle

Before editing, read current bead via `bd --sandbox --readonly show ht-0lo.22 --json` and `bd --sandbox --readonly comments ht-0lo.22` (no comments), the frozen brief/design ruling, and /private/tmp/handoff-delivery-main-merge-review.txt. The wording must distinguish two actual canonical paths:

- src/store/control.rs:1097 captures the exact direct episode's floor before membership changes; :1136 chooses direct initialization only for that path, otherwise generic join initialization. Accepted replay returns before either initializer.
- src/store/lazy_delivery.rs:252 validates direct pair identity and the exact active invitation, then resolves its real timeline event via episode_floor; this is not a guessed head baseline.
- src/store/read_cursors.rs:145 marks undisplayed lazy recipients beyond the seat maximum as legacy pending and raises open bindings to the head. :175 inserts missing direct cursors at the floor with ON CONFLICT DO NOTHING, preserving established cursors. :90 seeds a fresh joined binding from the highest seat cursor, otherwise head.
- TRUST-POLICY.md:851–904 explicitly separates own-stream direct readers from frozen legacy pending, states acceptance is not display, preserves sibling unread delivery, and retains the fresh-binding-after-acceptance rule. This agrees with the inspected implementation rather than merely repeating review text.

The pre-edit assertion that every acceptance starts all sessions at the head and a first reader settles every session was false for the active direct route. The edited text confines that settlement claim to generic seat-pending mail and preserves the independent direct stream. A binding born while the invitation is pending is distinguished from one born after acceptance.

## Verification

Documentation-only task: no tests, Cargo commands, runtime probes, Herdr, authenticated agents or real config access, as directed. Parent owns compile/merge and runtime qualifications.

Final commands run in the task worktree:

```text
git diff HEAD^ HEAD --check
[exit 0; empty output]
git status --short
[exit 0; empty output]
git rev-parse HEAD
045b1b65204b1c8bbb6957f7848e9b4a874e7be1
```

Self-review of the complete two-file diff matched the current bead and design ruling. Commit has truthful trailer: `Outside-adapter: docs/agent-usage.md, CHANGELOG.md`. No rebase after commit, per parent override.

## Model-visible size

Static documentation only; no per-message or per-turn framing, repeated boilerplate, queue policy or append bounds changed. UTF-8 file bytes measured before/after: docs/agent-usage.md 83,770 → 84,319 (+549); CHANGELOG.md 29,888 → 30,136 (+248); total +797. The added explanation lives in the static guide/changelog.

## Test hygiene

None: no tests or background processes started; no daemon/server/helpers to stop.

## Assumptions and concerns

Current bead, frozen brief and parent design ruling agree; no conflicting amendments. Parent overrides assign all build/runtime gates to the coordinator and forbid delegation; no child agents used. Fresh task worktree created from the stated integration branch; toolchain setup/provenance is inapplicable because this task edits only Markdown and invokes no runner/package.

No runtime clearance or final integration signoff is claimed. No concerns in the documentation scope.
