# Handoff delivery redesign: hooks

## Goal

Equivalent reinstalls preserve stable native per-event command strings while exact ownership, conservative migration and native trust boundaries remain intact.

Parent: [root design](2026-10-10-handoff-delivery-redesign-design.md); bead ht-0lo.4.

## Decisions and boundaries

The parent spec is the normative behavioral contract for this subtree. It specifies the selected alternatives, compatibility rules and edge cases; this spec scopes ownership and execution, without adding a second interpretation. Equivalent reinstalls preserve stable native per-event command strings while exact ownership, conservative migration and native trust boundaries remain intact.

## Tasks

### Stable native event dispatch and command generation

owns: hook-event HARNESS EVENT parser/registered-codec dispatch and stable per-event shell command grammar; consumes: existing harness event registry and canonical routing. Files: src/cli/{hook,mod,setup}.rs, src/harness/{setup,claude,codex}.rs, registry declarations/tests. Keep legacy hook syntax; validate invalid pairs before effects; command stability includes executable/route/harness/event only. Exercise declared Claude/Codex events with fixtures through dedicated entrypoints; retain Hermes callback and bundled mod route compatibility.

### Owned stable registration migration and tooling

owns: manifest-recorded exact ownership/install identity, repeat setup in place, recoverable UUID/shared-command migration, safe unsetup and status/trust/canary/launcher command compatibility; consumes: stable native per-event command grammar. Files: src/harness/setup.rs and claude/codex setup modules, src/cli setup/launch consumers, canary/tooling/docs/tests. Invoke isolated Claude/Codex setup-repeat/migration/interruption/unsetup with foreign identical/drifted groups and ensure no ownership theft, no trust writes, no group shifting on equivalent reinstall. Hermes/Claude mod regressions stay passing.

## Validation and failure handling

Each task runs its listed configurations before terminal integration with isolated config/daemon/host fixtures and owned test-support children. Preserve immutable keyed historical requests and terminal outcomes; new incompatible fields negotiate before durable effects. All implementation follows AGENTS.md lint/format/test and TRUST-POLICY cooperative canonical authority. No real user config edits, automatic trust or shared server management.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
