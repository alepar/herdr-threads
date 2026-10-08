# super-auto run — 2026-10-07-installer-human-permissions

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: design
codeMechanism: ordinary-subagents

idea: we basically need to make sure that all harnesses are permissioned to run arbitrary Bash(herdr-threads *) commands, except the ones that pose as human
spec: 2026-10-07-installer-human-permissions-design.md
branch: installer-permissions
base: main

approvals:
- design-review · approved — user approved the concrete namespace/permission rundown, then requested fully autonomous super-auto with both roasts on.
- workspace · human — preserve existing installer-permissions branch/worktree from c3b8f3f0; explicit original task naming overrides super-auto branch convention.
- merge · human — original task authorizes reviewed main-only merge/push after clean/current local+remote preflight and coordinator mutation window; no stash/force/tags/releases.
- validation · human — no worker full suite; focused tests and required clippy/default/fmt/diff/UUID cleanup. Coordinator owns exact integrated suite/release.
- retention · human — retain worktree/evidence until independent landing confirmation; coordinator owns finished-tab closure.
