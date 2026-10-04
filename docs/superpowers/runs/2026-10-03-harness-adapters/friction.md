# friction log — 2026-10-03-harness-adapters

- preflight · capability · Workflow absent; supported ordinary-subagents rung selected using collaboration.spawn_agent, collaboration.wait_agent and collaboration agent results. Execution uses one task chain at a time.
- preflight · user override · Reuse existing isolated harness-adapters branch/worktree; coordinator owns integrated full-suite sweep, base merge and cleanup. Do not recreate or remove the user-authorized workspace or run branch full suites.

- 2026-10-03 · super-design coverage · ordinary spawn_agent accepts text only, no input attachment. Reviewers load one immutable assembled prompt bundle (literal goals/tree/canonical requirements/precheck/ledger) then perform no further tool or source exploration; this is prompt transport only, all reviewers receive identical inputs.
