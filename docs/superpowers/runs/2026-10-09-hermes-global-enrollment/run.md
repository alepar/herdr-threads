# super-auto run — 2026-10-09-hermes-global-enrollment

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: roast-design
codeMechanism: Workflow

idea: ht-4ne: make Hermes enroll globally through its hooks instead of launch-only (TRUST-POLICY.md:412-416 says Hermes enrolls only via launch --kind hermes). Reasons in docs/superpowers/runs/2026-10-03-harness-adapters/2026-10-03-harness-adapters--hermes-design.md: launch-bound exact-runtime qualification (QualifiedCallback), the guarded Herdr process hint (src/harness/hermes/launch.rs:185), and no top-level vs child signal in on_session_start. New facts: Herdr 0.9.1 lists hermes as a built-in agent kind, and Herdr's own Hermes integration (~/.hermes/plugins/herdr-agent-state/__init__.py, v5; read-only, never write ~/.hermes) reports report-agent-session --agent hermes on on_session_start, on_session_reset and pre_llm_call, so bare-Hermes detection may already be solved (unverified). First verify in an isolated named Herdr test session (scripts/lib/isolated-herdr.sh) whether a bare hermes pane shows as agent hermes, then design. The agy run (super-auto/agy-harness-adapter) is designing a generic harness-local top-level witness for telling a subagent from /clear; coordinate before duplicating it. Follow AGENTS.md (no real-config writes, no push or stash, never touch the shared Herdr server).
branch: super-auto/hermes-global-enrollment
base: main
spec: 2026-10-09-hermes-global-enrollment-design.md
epic: ht-4ne
roastDesignRound: 1

approvals:
- top-split · auto · ht-4ne.1 LEAF, ht-4ne.2 LEAF, ht-4ne.3 LEAF, ht-4ne.4 LEAF
- coverage-round-1 · L1 flag-sweep ht-4ne.1 kept (auto) · L2 R2 applied (auto) · L3 R6 applied (auto) · L4 R7 applied (auto) · L5 R9 applied (auto) · L6 R5 rejected (auto) · L7 R10 applied (auto) · L8 bridge refresh applied (auto) · L9 real-Hermes applied in part (auto) · L10 parity merged into L2 (auto) · L11 profile-without-plugin rejected (auto) · L12-L14 seams applied (auto) · R-list: R1-R12 per coverage-round-1-requirements.md · requirements: 10 · mapped: 10 · unmapped: 2 (R6, R9)
- coverage-round-2 · L15 R4 applied (auto) · L16 pane reuse applied (auto) · L17 resume applied (auto) · L18 daemon start applied (auto) · L19 stale-bridge hint rejected (auto) · L20 seam applied (auto) · L21 inert label applied (auto) · L22 profile resolution rejected (auto) · L23 duplicate check-in applied (auto) · L24 race applied (auto) · L25 disabled plugin rejected in part (auto) · L26 R10 launched pane applied (auto) · L27 version skew rejected (auto) · L28 merged into L17 (auto) · L29 cli_session_start applied (auto) · requirements: 12 · mapped: 12 · unmapped: 0 · divergence: findings 13 → 15, novel 13/15 (87%), widening: yes (cap holds at two rounds; the design roast reviews the settled tree) · integration sweep ht-4ne.5
