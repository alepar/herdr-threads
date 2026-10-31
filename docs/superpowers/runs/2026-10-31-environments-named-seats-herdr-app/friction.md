# friction log — 2026-10-31-environments-named-seats-herdr-app
- [2026-10-31] coverage-reviewer: inputs passed as three named files the reviewers may read (not pasted into the prompt as the template says), because Workflow scripts cannot read files and inlining 28 KB in the tool call was avoided; reviewers were told to read only those files
- [2026-10-31] coverage applier (subagent) wrote a bead description through an unquoted heredoc; backticked command names executed and a `cargo nextest` run started in the run worktree; stopped during compilation, no test/daemon/Herdr process started (verified by the orchestrator: only the pre-existing shared daemon is running)
- [2026-10-31] super-roast design round 1 · rung: Workflow
