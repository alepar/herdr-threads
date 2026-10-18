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
