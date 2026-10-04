# super-auto run — 2026-10-03-harness-adapters

flags: planOneShot=false skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: design
codeMechanism: ordinary-subagents

idea: Own the harness adapter architecture and Hermes feature; ideally adding a new harness is as easy as implementing the new interface. Audit Claude/Codex seams and authoritative Hermes integration docs, preview and approve the architecture, then implement and review in an isolated worktree for coordinator-owned integration into main. Approved scope includes a built-in registry in the same binary and the Hermes Python bridge, for the later 0.3.0 release.
branch: harness-adapters
base: main

approvals:
- design-review · approved — user approved broad architecture and Python bridge before invoking super-auto; detailed design proceeds autonomously with both reviews retained.

parked:
- run.md · degraded-verdict · "Final integrated full-suite sweep, main merge, worktree cleanup and 0.3.0 release belong to threads-main w4:p1 by explicit user instruction; this run supplies focused checks and a frozen merge request."
