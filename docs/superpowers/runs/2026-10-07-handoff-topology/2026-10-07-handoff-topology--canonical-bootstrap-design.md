## Goal

Canonical exact identity/attempt transitions/atomic attachment remain durable and absorbing across concurrent replay and response loss.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.2. Autonomous Mode B.

## Decisions

Inherit the root spec exact grammar, immutable identities, authority and failure contracts verbatim; no independent namespace or retry policy. Split only independently reviewable artifacts with actual consumed boundaries.

### Schema and exact bootstrap identity

Land minimal additive0028 schema, unique immutable namespace/keys and bootstrap current-state lookup with inert command wiring.
owns: canonical bootstrap persistence and exact identity lookup;
consumes: topology boundary contract.
Files: migrations/0028_handoff_topology.sql, src/store/topology_handoff.rs, src/store/schema.rs, src/store/mod.rs.
Acceptance: actual reviewed0026/0027 predecessors absorbed before migration edit; no placeholders; migration/upgrade/tamper and exact payload/namespace mismatch RED/GREEN; historical bytes and old Handoff shapes unchanged; bounded indexed lookups.

### One-use creation attempts and operator recovery decisions

Implement canonical reserve/evidence/recovery state transitions in deciding transactions.
owns: canonical bootstrap state transitions, one-use submission authorization and administrative recovery decisions;
consumes: canonical bootstrap persistence and exact identity lookup.
Files: src/store/topology_handoff.rs (attempt transition module if split), tests/store/topology_attempts.rs.
Acceptance: concurrent/lost-reserve reply yields at most one authorization; exact same-attempt evidence replay; proven NotSubmitted vs unknown; operator created-pane/not-created replay/contradiction and late older creation conflict; live A2 checks and completed recovery cannot reactivate; meaningful RED/GREEN.

### Atomic recipient/thread attachment and absorbing completion

Implement cross-fence exact resolution/recipient/downstream attachment and bootstrap terminal state.
owns: canonical bootstrap attachment, archival protection and absorbing completion;
consumes: canonical bootstrap persistence and exact identity lookup.
Files: src/store/topology_handoff.rs (attachment module if split), src/store/control.rs, src/store/archival.rs.
Acceptance: existing-thread protected before topology; create-thread attaches in deciding create transaction to both fences; deterministic resolve key/result; contradictory attach refuses; completed historical presentation bypasses only live guards, no effects or revived protection; delayed importer terminal precedence; RED/GREEN.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
