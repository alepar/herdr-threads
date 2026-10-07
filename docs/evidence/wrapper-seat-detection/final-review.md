# Final feature review

Independent agent `final_feature_review` reviewed the complete feature from base
`cbd5b706`, including the original flag removal and the subsequent user-selected scope.
It approved the implementation with no Critical or Important findings.

The user explicitly chose user-managed foreground configuration after the native Codex
Bash-tool probe, rather than automatic TUI-pane inference or an additional wrapper.
The accepted limit is recorded in TRUST-POLICY. This review does not claim automatic
seat detection or qualify unmeasured wrapper execution.

The reviewed implementation removes automatic daemon-mode flags, parses optional
`HERDR_THREADS_CODEX_OPTS` and `HERDR_THREADS_CLAUDE_OPTS` without shell expansion,
prepends them while retaining caller argument order, and freezes them in handoff state
before preflight. Retry uses the stored argv. Existing canonical seat and binding checks
remain in place.

Installer inspection is read-only and advisory. Claude recognizes user
`disableAgentView` or the equivalent configured environment flag. Codex guidance states
that persistent auto-start settings do not prevent attachment to an existing daemon;
users can choose the optional native flag only where their entrypoint supports it.
Setup and setup-status explain missing/unknown configuration and do not write these
foreground settings.

The reviewer suggested labeling the earlier blocker review as historical and adding an
isolated CLI entrypoint regression for environment-option wiring. The historical record
is now labeled accordingly. The added actual CLI regression checks both harness option
variables and malformed quoting before launch connects or writes durable state.

Final focused verification: 35 CLI launch tests, 19 handoff tests, 49 setup CLI tests
and the bounded foreground-reader unit test passed. Required all-feature clippy,
default-feature checking, formatting and diff guards passed. No full suite or release
operation is part of this feature run; the coordinator retains the integrated sweep
and next release. The frozen SHA is reported in the integration handoff.
