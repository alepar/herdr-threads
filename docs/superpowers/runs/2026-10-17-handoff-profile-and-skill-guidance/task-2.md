Expose new-tab --profile-dir and validate explicit profile/cwd paths before any recipient resolution, durable publication or native effects.

Spec: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/2026-10-17-handoff-profile-and-skill-guidance-design.md
Files-touched: src/cli/commands.rs, src/cli/handoff_delivery.rs, src/cli/topology_runtime.rs (request construction only), tests/cli/topology_handoff.rs, tests/cli/handoff.rs
owns: public handoff directory grammar and path normalization into LaunchDirectoryContext capture inputs.
consumes: optional launch-context transport fields and LaunchPolicy explicit-directory seam.
blocked-by ht-1urq.1: consumes boundary contract
owns: pane/existing directory-option refusal before effects and focused parser/no-option/plain-launch regression tests.

## Acceptance
- [ ] Add --profile-dir PATH for new-tab launch; existing and pane launch reject explicit profile/cwd with concise new-tab guidance before effects; plain launch and no-option grammar preserve behavior.
- [ ] Validate absolute existing directories, reject files/missing/non-UTF-8/control/over-bound inputs and canonicalize once; symlink/spaces/quotes cases retain the intended canonical directory.
- [ ] Pass normalized request data without adapter-name branches or native named-profile confusion; parser fixtures prove invalid requests cause zero thread/tab/seat/native effects.
- [ ] Run focused parser/handoff tests, fmt, locked all-feature clippy and harness-neutral check.
