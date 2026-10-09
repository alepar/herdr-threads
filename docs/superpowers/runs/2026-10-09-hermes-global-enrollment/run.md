# super-auto run — 2026-10-09-hermes-global-enrollment

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: fix-loop
codeMechanism: Workflow

idea: ht-4ne: make Hermes enroll globally through its hooks instead of launch-only (TRUST-POLICY.md:412-416 says Hermes enrolls only via launch --kind hermes). Reasons in docs/superpowers/runs/2026-10-03-harness-adapters/2026-10-03-harness-adapters--hermes-design.md: launch-bound exact-runtime qualification (QualifiedCallback), the guarded Herdr process hint (src/harness/hermes/launch.rs:185), and no top-level vs child signal in on_session_start. New facts: Herdr 0.9.1 lists hermes as a built-in agent kind, and Herdr's own Hermes integration (~/.hermes/plugins/herdr-agent-state/__init__.py, v5; read-only, never write ~/.hermes) reports report-agent-session --agent hermes on on_session_start, on_session_reset and pre_llm_call, so bare-Hermes detection may already be solved (unverified). First verify in an isolated named Herdr test session (scripts/lib/isolated-herdr.sh) whether a bare hermes pane shows as agent hermes, then design. The agy run (super-auto/agy-harness-adapter) is designing a generic harness-local top-level witness for telling a subagent from /clear; coordinate before duplicating it. Follow AGENTS.md (no real-config writes, no push or stash, never touch the shared Herdr server).
branch: super-auto/hermes-global-enrollment
base: main
spec: 2026-10-09-hermes-global-enrollment-design.md
epic: ht-4ne
roastDesignRound: 2
roast-design: 2026-10-09-hermes-global-enrollment-roast-design-1.md, 2026-10-09-hermes-global-enrollment-roast-design-2.md

approvals:
- top-split · auto · ht-4ne.1 LEAF, ht-4ne.2 LEAF, ht-4ne.3 LEAF, ht-4ne.4 LEAF
- coverage-round-1 · L1 flag-sweep ht-4ne.1 kept (auto) · L2 R2 applied (auto) · L3 R6 applied (auto) · L4 R7 applied (auto) · L5 R9 applied (auto) · L6 R5 rejected (auto) · L7 R10 applied (auto) · L8 bridge refresh applied (auto) · L9 real-Hermes applied in part (auto) · L10 parity merged into L2 (auto) · L11 profile-without-plugin rejected (auto) · L12-L14 seams applied (auto) · R-list: R1-R12 per coverage-round-1-requirements.md · requirements: 10 · mapped: 10 · unmapped: 2 (R6, R9)
- coverage-round-2 · L15 R4 applied (auto) · L16 pane reuse applied (auto) · L17 resume applied (auto) · L18 daemon start applied (auto) · L19 stale-bridge hint rejected (auto) · L20 seam applied (auto) · L21 inert label applied (auto) · L22 profile resolution rejected (auto) · L23 duplicate check-in applied (auto) · L24 race applied (auto) · L25 disabled plugin rejected in part (auto) · L26 R10 launched pane applied (auto) · L27 version skew rejected (auto) · L28 merged into L17 (auto) · L29 cli_session_start applied (auto) · requirements: 12 · mapped: 12 · unmapped: 0 · divergence: findings 13 → 15, novel 13/15 (87%), widening: yes (cap holds at two rounds; the design roast reviews the settled tree) · integration sweep ht-4ne.5
stepBackDesign-round-1: redesign — applied: fallback never starts the daemon → fallback keeps the existing turn-path ensure_running (dissolves 2)
roast-design exit: converged at round 2 (Should-fix (3 confirmed) [converged], no escalations, nothing beyond cap, no qualifier); punch list applied inline: r2 cross-harness context conflict → ht-4ne.4 + spec D10; r2 two Nits (Key decisions daemon sentence; D8/Follow-on side-agent limit) → spec; r2 rejected operator-retire re-enrollment → accepted-limit note in spec D10
graph-pass: depth 4→4 · width 1.2→1.2 · applied 0 · parked 0

codeBuckets:
  completed: ht-4ne.1, ht-4ne.4, ht-4ne.2, ht-4ne.3, ht-4ne.5
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: not ready (stale docs/compatibility/README.md:40 + adapter.rs:326 comment; concurrent start/first-turn test masks an UnknownOutcome failure with a retry; TRUST-POLICY says resumed session checks in as Startup but seated-pane resume checks in as current with no context)
  sweep: SWEEP DEFERRED (caller-owned)
  slowness: round 1: graph-bound — 5 open beads, depth 4, achievable width 2 vs cap 8; edge audit armed now
  worktreesKept:
  processSweep: stopped 0 · survived 0
roastCodeRound: 1
roast-code: 2026-10-09-hermes-global-enrollment-roast-pr-1.md
stepBackCode-round-1: patch — These are first-round findings. They are independent, low-severity, and mostly about docs, test isolation or test strength. None comes from a design decision that a redesign would remove, so redesigning would cost more than patching.
scopeFilter-round-1: [Should-fix] .config/nextest.toml:101 punch-list — Test-scheduling hardening against a flake; it does not change whether the goal-named enrollment behavior is correct.
scopeFilter-round-1: [Should-fix] docs/superpowers/runs/2026-10-09-hermes-global-enrollment/run.md:27 in-scope — The committed run record is tied to the goal-named stale docs (D8: docs saying Hermes is launch-only must be updated), and the same cluster has an in-scope member.
scopeFilter-round-1: [Nit] tests/cli/hook.rs:8260 punch-list — Test-strength improvement; the test cannot detect double allocation, but no incorrect behavior is shown and the cluster has no in-scope member.
scopeFilter-round-1: [Nit] docs/compatibility/README.md:40 in-scope — The page still says Hermes joins only via launch, which is the user-facing doc the D8 scope note requires updating.
cluster dropped: hermes-hook-cap-isolation — widen the startup_enrollment_hermes_.* nextest serializing override, narrow the concurrent-test retry, detect double allocation
scope-filter: 2 in-scope · 2 punch-listed
fix-beads-round-1: ht-4ne.10 (stale-prelaunch-text cluster + final-review adapter.rs:326), ht-4ne.11 (final-review untested scope: retire re-enroll, stale-hint absence, visible smoke skip); final-review retry item and TRUST-POLICY resume item stay rejected (roast r1 panels); full-suite/lint/leak item → phase 6
