## Goal

A new-tab command freezes exact intent and performs at-most-one native creation per canonical attempt, then existing guarded launch and terminal replay.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.4. Autonomous Mode B.

## Decisions

Inherit the root spec exact grammar, immutable identities, authority and failure contracts verbatim; no independent namespace or retry policy. Split only independently reviewable artifacts with actual consumed boundaries.

### New-tab grammar and immutable publication

Implement strict target/channel grammar and additive BootstrapPlan publication.
owns: frozen bootstrap publication and normalized new-tab grammar;
consumes: topology boundary contract.
Files: src/cli/commands.rs (new-tab arguments), src/cli/topology_handoff.rs (publication), src/cli/mod.rs.
Acceptance: actual CLI parse-conflict RED/GREEN; absolute existing cwd, livecaller workspace vs focus, exact state/socket/options/child keys; no manual nativecreate or effects during parse; historical pane grammar unchanged.

### At-most-one native creation and exact seat attachment

Implement operation-lock/reserve/local-progress/native-call/evidence/guarded-resolution path.
owns: BootstrapPlan live resume and native submission progress;
consumes: frozen bootstrap publication, canonical bootstrap state transitions, canonical bootstrap attachment and typed CreatedTab/submission outcomes.
Files: src/cli/topology_handoff.rs (live coordinator), src/cli/retry.rs.
Acceptance: RED/GREEN successful create, lost reserve/host reply/save, crash/concurrentcopiedjournal, local/canonical recovery, incarnation/restorehold exactrecipient guards and deterministic resolution; never close user tab or auto adopt label; reports retain exact IDs/attempts.

### Downstream launch and terminal bootstrap replay

Compose exact existing Handoff downstream with persistent possible-start fence and bootstrap completion/report cleanup.
owns: bootstrap downstream completion and absorbing terminal replay;
consumes: BootstrapPlan live resume and canonical bootstrap attachment.
Files: src/cli/topology_handoff.rs (downstream/terminal), src/cli/retry.rs, src/cli/handoff.rs (existing compound helper only).
Acceptance: RED/GREEN no duplicate create/invite/message/launch on failed flush/removal/restart/archive/binding change; frozen terminal report required; completed replay only presents/removes owned intent, wrongnamespace refuses; launch possible-start preserved.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
