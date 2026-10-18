# super-auto run — 2026-10-17-handoff-profile-and-skill-guidance

flags: planOneShot=true skipPlanRoast=true skipCodeRoast=false autonomous=true
phase: design
codeMechanism: ordinary-subagents
runTag: handoff-profile-and-skill-guidance-20261018T024112
context: Context: codex · planner=475K/auto · implementer=475K/auto · reviewer=475K/auto · finalReview=475K/auto · assembler=475K/auto

epic: ht-1urq
spec: 2026-10-17-handoff-profile-and-skill-guidance-design.md

idea: Improve shipped herdr-threads skill to prefer names in user-facing communication; use p0 when a recipient may be waiting, blocked, or idle and needing to react now; request ACK when sender depends on receipt and proactively work around missing ACKs (inspect via herdr skill, do work, or hand off fresh). Add handoff profile directory selection (CODEX_HOME, CLAUDE_CONFIG_DIR, adapter equivalents) and working directory for launch modes, validated existing directories and frozen for retry, with tests, help/docs/changelog and matrix coverage. Fully autonomous, design roast off, code roast on; hand green integration branch to main for merge/release, proposing minor 2.1.0. Do not merge/tag.
branch: integration/handoff-profile-and-skill-guidance
base: main

graph-pass: depth 5→5 · width 2.0→2.0 · applied 0 · parked 0

approvals:
- top-split · auto · ht-1urq.1 LEAF, ht-1urq.2 LEAF, ht-1urq.3 LEAF, ht-1urq.4 LEAF, ht-1urq.5 LEAF, ht-1urq.6 LEAF, ht-1urq.7 LEAF, ht-1urq.8 LEAF, ht-1urq.9 LEAF
- coverage-round-1 · canonical R-list: R1 Known names in user-facing prose, exact IDs in commands/disambiguation/receipt evidence.; R2 Use p0 when recipient may be waiting, blocked, or idle and needing to react now; preserve routine p1 and wake defaults.; R3 Request ACK and actionable response when dependent on receipt; recover proactively by pane inspection, self-work or fresh handoff without polling/spam, preserving ACK-only-receipt semantics.; R4 Public new-tab profile directory selection maps Codex CODEX_HOME and Claude CLAUDE_CONFIG_DIR without credential logging.; R5 Hermes HERMES_HOME plus native named-profile resolution work; agy unsupported explicit directory and cwd behavior are honest.; R6 Explicit profile/cwd paths must be existing absolute directories canonicalized once with symlink/encoding/control/size handling.; R7 The frozen selected profile env/cwd reach real tab creation and all derived-launch preflight/config inspection consistently, preserving guarded executable/PATH behavior.; R8 Automatic and explicit-thread retries retain exact selected directories/env/cwd/argv despite caller changes and refuse disappeared directories before new effects.; R9 Additive daemon capability admission, canonical context agreement and historical context-free identity/replay compatibility remain exact.; R10 Pane/existing profile/cwd options refuse before effects; ordinary no-option and plain launch behavior remain unchanged.; R11 Concise shipped skill plus full reference/help/README/goldens and changelog are aligned, byte growth recorded within 1 KiB skill target, propose 2.1.0 without tagging/bumping.; R12 Focused tests and executable public configuration smoke/release matrix prove actual child env/cwd with owned fixtures and cleanup, without real config/credentials or shared server changes. · requirements: 12 · mapped: 12 · unmapped: 0 · auto GAP compact ownership clarification → ht-1urq.2–.9 (pane/existing refusal, focused tests, guarded PATH, credential diagnostics/evidence, skill bytes, release boundary, fixture cleanup); no scope expansion
- design-pause · human account-switch pause · completed root Mode B brainstorm/spec commit, fresh key-abstractions/sync, nine-leaf decomposition, fresh promotion COMPLETE, auto top split, two valid coverage-round-1 reviews and ownership clarification edits ht-1urq.2–.9 · pending coverage round-1 requirements-tally/result finalization, round 2 (required because round 1 changed compact declarations), integration-sweep leaf and graph pass; design roast off; caller-owned handoff not reached · all four design subagents completed; no implementation started

- coverage-round-2 · requirements: 12 · mapped: 12 · unmapped: 0 · auto no findings; C1–C7 closed; two valid fresh reviews; loop exits at fixed cap
- integration-sweep · auto · ht-1urq.10 depends on all nine implementation/documentation/configuration-smoke leaves; terminal scoped integration net, caller retains root acceptance
- design-ready · auto · coverage two rounds complete, 12/12 mapped, no open findings; design roast OFF, zero reports and zero rounds; graph all five candidates kept, no parked changes; caller-owned handoff
