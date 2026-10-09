# friction log — 2026-10-09-claude-mod-inbound-delivery
- [2026-10-09T07:04:13Z] super-design parallelism pass judged inline (single candidate edge) instead of dispatching graph-pass-prompt.md; rationale recorded in coverage-ledger g1
- [2026-10-09T07:04:52Z] super-code pre-flight: testPaths covers tests/** and the mod dir; Rust #[cfg(test)] inline tests in src/ are not covered by the Test-changes check
- [2026-10-09T10:11:25Z] super-roast PR round 1 left the integration worktree detached at the reviewed SHA (a scout/judge ran git checkout <sha> in the caller's worktree); run-record commits landed detached and the fix-loop coordinator stopped integration-blocked; reattached by fast-forwarding the branch and relaunched
