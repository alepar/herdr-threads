# friction log — 2026-11-03-atomic-remote-handoff
- [preflight] loaded skills 6.4.2-alepar6.6, latest 6.4.2-alepar6.7; plugin update skipped because the run's rules forbid writing ~/.aisw (the plugin cache lives there); run continues on the loaded version.
- [roast-design r1] usage limit killed 2 scouts, dedupe and reporter mid-run (weekly limit); resumed via resumeFromRunId after the reset, completed 8/8 scouts. Rung: Workflow.
- [design] round-2 punch list applied inline by a spec fixer (converged exit, no re-roast): 3 Should-fix + 4 Nits; one fix (failed ledger state) reverses round-1 C41's 'delete on Refused' detail.
- [design] parallelism pass judged inline (4 candidates, obvious artifacts) instead of dispatching graph-pass-prompt
- [code] final review produced only a bounded draft (assembled/.../bounded-draft.md; "omission provenance unavailable"); no final-review-<inv>.md was written though reviewPath was returned non-null; report-status --collect exit 3 (bundle unavailable + malformed collection record). Code roast covers the whole branch instead.
- [code] Finish process sweep exit 4: PIDs 1950 (systemd --user), 2008 (sd-pam), 6582, 976133 (sshd-session) uninspectable; not run processes; task worktrees retained for phase-7 cleanup.
- [roast-code r1] dispatch-text assembler skipped; round run directly with captured base/head (assembler: unavailable — orchestrator choice to save a dispatch; diff ~ whole branch)
- [report] report.md written by the orchestrator directly (no drafter dispatch); status line condensed from report-status, whose full output is dominated by acceptance-bookkeeping gaps
- [sweep] final suite at 4c30ec33a: 1 failure (store_only_daemon_serves_without_host_workers: handoff task logged a cancellation); fixed directly by the orchestrator (4-line filter) instead of a coordinator round
