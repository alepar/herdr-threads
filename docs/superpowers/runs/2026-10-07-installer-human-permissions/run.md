# super-auto run — 2026-10-07-installer-human-permissions

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: roast-design
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

- top-split · auto · ht-uwd.1 LEAF, ht-uwd.2 LEAF, ht-uwd.3 PROMOTE, ht-uwd.4 LEAF, ht-uwd.5 PROMOTE, ht-uwd.6 LEAF, ht-uwd.7 PROMOTE, ht-uwd.8 PROMOTE, ht-uwd.9 LEAF, ht-uwd.10 LEAF, ht-uwd.11 LEAF, ht-uwd.12 LEAF

- coverage-round-1 · auto C1 applied ht-uwd.11, C2 narrowed ht-uwd.10, C3 amended ht-uwd.8.2/ht-uwd.7.2; canonical requirements:
  R1 Ordinary root reads and communication writes remain available while person/operator actions require immediate human, independent of output formatting or globals.
  R2 Root inferred Human and legacy/operator aliases refuse before identity, intent, accountable effects, completed presentation or cleanup.
  R3 Retry classification uses original frozen semantics/claim/scope and retains exact bytes/digest/keys and completed agent historical replay.
  R4 Independently owned permission lifecycle uses explicit missing-component consent and never silently grants through hook ownership.
  R5 Claude positive ordinary rules migrate exact historical owned broad/retired grants with durable interruption recovery and preserved foreign/preexisting/stronger policy.
  R6 Codex exact executable union allow plus immediate human prompt operates under active CODEX_HOME/rules, preserving stronger policy and sandbox/network modes.
  R7 Validated bare/canonical/link/alias and pinned forms cover installed paths without foreign PATH adoption or ambiguous native path wildcard widening.
  R8 Setup/status/unsetup/installer/requested doctor share permission component semantics and truthful diagnostics.
  R9 Human ready/retry/continuation guidance preserves strict wire compatibility and honest A2/A3/A4 provenance, shared owner seams remain coordinated.
  R10 Early isolated configuration smoke and focused causal regression tests cover grammar, both backends, quoting/compounds, ownership refusal and immutable replay without native models/real config/shared host/full suite.
  requirements: 10 · mapped: 10 · unmapped: 0

- coverage-round-2 · auto · no new findings; C1/C2/C3 closed; requirements: 10 · mapped: 10 · unmapped: 0; findings: 3 → 0; novel: 0/0 (0%); widening: no

roastDesignRound: 2

roast-design: 2026-10-07-installer-human-permissions-roast-design-1.md, 2026-10-07-installer-human-permissions-roast-design-2.md

stepBackDesign-round-1: redesign — applied: destination-manifest absence → exact historical-owned state with native-scope-bounded narrowing (dissolves 1)

graph-pass: depth 10→10 · width 1.8→1.8 · applied 1 · parked 8

parked:
- graph-pass · graph-change · "change: ht-uwd.8.2 <- ht-uwd.7 · repoint · ht-uwd.8.2 <- ht-uwd.7.2: permission reconciliation and consent · safe no · Acceptance explicitly consumes ht-uwd.7 lifecycle and consent; its lifecycle requirement must be verified through ht-uwd.7.2."
- graph-pass · graph-change · "change: ht-uwd.8.1 <- ht-uwd.7 · repoint · ht-uwd.8.1 <- ht-uwd.7.1: explicit permission lifecycle API · safe no · Removed wait on ht-uwd.7.2 involves shared src/cli/installer.rs and an explicit consent reference."
- graph-pass · graph-change · "change: ht-uwd.7.1 <- ht-uwd.5 · repoint · ht-uwd.7.1 <- ht-uwd.5.3.2: independent Claude lifecycle incorporating renderer and historical transfer · safe no · The dependent explicitly consumes ht-uwd.5 backend and historical transfer outputs."
- graph-pass · graph-change · "change: ht-uwd.5.3 <- ht-uwd.5.2 · narrow · ht-uwd.5.3.1 <- ht-uwd.5.2: legacy ownership evidence · safe no · Existing producer edge suffices structurally, but the epic declares shared src/harness/setup.rs and historical ownership consumption."
- graph-pass · graph-change · "change: ht-uwd.5.3 <- ht-uwd.5.1 · narrow · ht-uwd.5.3.1 <- ht-uwd.5.1: Claude renderer and representability contract · safe no · The lifecycle child and renderer share src/harness/permissions/claude.rs; renderer consumption is explicit."
- graph-pass · graph-change · "change: ht-uwd.8 <- ht-uwd.7 · narrow · ht-uwd.8.1 <- ht-uwd.7.1: permission lifecycle API; ht-uwd.8.2 <- ht-uwd.7.2: permission reconciliation and consent · safe no · Removed waits include shared src/cli/installer.rs and declared lifecycle/consent consumption."
- graph-pass · graph-change · "change: ht-uwd.7 <- ht-uwd.5 · narrow · ht-uwd.7.1 <- ht-uwd.5.3.2: independent Claude lifecycle · safe no · ht-uwd.7.2 and ht-uwd.5.3.2 share tests/installer_integrations.rs, and the epic explicitly consumes Claude backend outputs."
- graph-pass · graph-change · "change: ht-uwd.3 <- ht-uwd.1 · narrow · ht-uwd.3.1 <- ht-uwd.1: InvocationActor route; ht-uwd.3.2 <- ht-uwd.1: InvocationActor route and command catalog · safe no · Both leaf edges already exist, but declared src/cli/mod.rs overlap and explicit route consumption prevent the strict safe classification."
