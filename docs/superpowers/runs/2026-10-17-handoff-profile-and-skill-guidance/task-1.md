Land the inert shared directory-context type, defaulted optional retained fields and adapter policy seam so later leaves compile against one contract.

Spec: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/2026-10-17-handoff-profile-and-skill-guidance-design.md
Files-touched: src/protocol/handoff/launch_directories.rs (new), src/protocol/handoff.rs, src/protocol/handoff/automatic_bootstrap.rs, src/protocol/handoff/automatic.rs, src/cli/launch.rs, src/cli/topology_handoff.rs, src/harness/adapter.rs; required constructor updates only
owns: LaunchDirectoryContext; owns: optional launch-context transport fields and LaunchPolicy explicit-directory seam.
**LaunchDirectoryContext** — new struct, the immutable directory and declared-environment selection retained across new-tab handoff and retry.
- Owns: canonical requested profile directory, bounded adapter-produced environment projection and selected cwd; shared validation of their bounds and agreement with bootstrap cwd/env.
- Hides: the difference between a newly captured selection and its exact replay. An absent context preserves historical serialization and empty-env rules.
- Must not: choose harness environment names, resolve native named profiles, hold credentials, confer seat authority or freeze configuration contents. Adapters own profile semantics; existing daemon identity validation owns authority.
- Relationships: composed into saved bootstrap and derived launch inputs; consumed by existing creation, preflight and retry orchestration. `LaunchPolicy` produces the profile projection; transport uses the frozen projection unchanged.
- Lifecycle: captured once before publication, owned by the durable plan and copied into its exact derived child. Immutable, with no process-global state or owned external resources.
- Errors: malformed or inconsistent retained data is distinct from a selected directory that has disappeared or ceased to be a directory. Structural validation is deterministic; orchestration rechecks filesystem existence before unpublished create/start boundaries without canonicalizing again.

## Acceptance
- [ ] Create LaunchDirectoryContext with deterministic bounded validation for absolute UTF-8 profile directory, cwd and environment projection; absence serializes identically to historic inputs.
- [ ] Add a responsibility comment on the type stating ownership, exclusions, protocol value role and where adapter variation goes; include the abstraction entry above.
- [ ] Add optional serde-default/skip-none fields to saved bootstrap/derived launch request contexts and a neutral default unsupported LaunchPolicy method; keep context-bearing operational admission disabled until canonical-validation leaf.
- [ ] Carry context through exact child constructors as inert data, update literal constructors and prove empty/default serialization and existing tests compile; no CLI feature activation or new effects.
