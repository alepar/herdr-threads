# super-auto run — 2026-10-07-installer-human-permissions

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: design
codeMechanism: ordinary-subagents

idea: we basically need to make sure that all harnesses are permissioned to run arbitrary Bash(herdr-threads *) commands, except the ones that pose as human
spec: 2026-10-07-installer-human-permissions-design.md
epic: ht-uwd
branch: installer-permissions
base: main

approvals:
- design-review · approved — user approved the concrete namespace/permission rundown, then requested fully autonomous super-auto with both roasts on.
- workspace · human — preserve existing installer-permissions branch/worktree from c3b8f3f0; explicit original task naming overrides super-auto branch convention.
- merge · human — original task authorizes reviewed main-only merge/push after clean/current local+remote preflight and coordinator mutation window; no stash/force/tags/releases.
- validation · human — no worker full suite; focused tests and required clippy/default/fmt/diff/UUID cleanup. Coordinator owns exact integrated suite/release.
- retention · human — retain worktree/evidence until independent landing confirmation; coordinator owns finished-tab closure.

- top-split · auto · ht-uwd.1 LEAF, ht-uwd.2 LEAF, ht-uwd.3 PROMOTE, ht-uwd.4 LEAF, ht-uwd.5 PROMOTE, ht-uwd.6 LEAF, ht-uwd.7 PROMOTE, ht-uwd.8 PROMOTE, ht-uwd.9 LEAF, ht-uwd.10 LEAF
