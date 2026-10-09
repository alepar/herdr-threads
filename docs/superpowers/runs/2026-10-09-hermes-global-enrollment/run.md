# super-auto run — 2026-10-09-hermes-global-enrollment

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: design
codeMechanism: Workflow

idea: ht-4ne: make Hermes enroll globally through its hooks instead of launch-only (TRUST-POLICY.md:412-416 says Hermes enrolls only via launch --kind hermes). Reasons in docs/superpowers/runs/2026-10-03-harness-adapters/2026-10-03-harness-adapters--hermes-design.md: launch-bound exact-runtime qualification (QualifiedCallback), the guarded Herdr process hint (src/harness/hermes/launch.rs:185), and no top-level vs child signal in on_session_start. New facts: Herdr 0.9.1 lists hermes as a built-in agent kind, and Herdr's own Hermes integration (~/.hermes/plugins/herdr-agent-state/__init__.py, v5; read-only, never write ~/.hermes) reports report-agent-session --agent hermes on on_session_start, on_session_reset and pre_llm_call, so bare-Hermes detection may already be solved (unverified). First verify in an isolated named Herdr test session (scripts/lib/isolated-herdr.sh) whether a bare hermes pane shows as agent hermes, then design. The agy run (super-auto/agy-harness-adapter) is designing a generic harness-local top-level witness for telling a subagent from /clear; coordinate before duplicating it. Follow AGENTS.md (no real-config writes, no push or stash, never touch the shared Herdr server).
branch: super-auto/hermes-global-enrollment
base: main
spec: 2026-10-09-hermes-global-enrollment-design.md
