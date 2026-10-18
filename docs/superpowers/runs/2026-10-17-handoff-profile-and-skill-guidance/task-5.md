Admit additive context-bearing identities canonically while preserving exact historical context-free validation and replay.

Spec: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/2026-10-17-handoff-profile-and-skill-guidance-design.md
Files-touched: src/protocol/handoff.rs, src/protocol/handoff/automatic_bootstrap.rs, src/protocol/handoff/automatic.rs, src/store/topology_handoff/*.rs as needed, src/service capability registration, tests/store/topology_handoff*.rs, tests/handoff_topology_authority.rs
owns: canonical launch-context admission, capability and exact retained identity/child/report validation.
consumes: LaunchDirectoryContext; consumes: optional launch-context transport fields.
blocked-by ht-1urq.1: consumes boundary contract
owns: focused canonical capability admission, historical identity and exact replay regression tests.

## Acceptance
- [ ] Advertise additive handoff.launch_directories_v1 capability for the complete canonical implementation and reject context-bearing operations without support before effects.
- [ ] Present context requires exact bounded cwd/env agreement; absent context keeps old empty-env rule and old serialized identity/digest bytes; reject mismatched context and derived child/report.
- [ ] Automatic and explicit-thread bootstrap retain context unchanged through canonical attachment, progress, completion and replay; malformed/unknown shapes do not weaken existing fences.
- [ ] No historical migration/identity bytes change; add meaningful identity round-trip, old-shape and forged mismatch tests; focused durability tests, fmt, locked all-feature clippy and harness-neutral pass.
