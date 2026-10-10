# Handoff delivery redesign: receipts

## Goal

Every pending ACK-required message has a send-anchored original deadline and is scanned for overdue warning without requiring recipient availability.

Parent: [root design](2026-10-10-handoff-delivery-redesign-design.md); bead ht-0lo.1.

## Decisions and boundaries

The parent spec is the normative behavioral contract for this subtree. It specifies the selected alternatives, compatibility rules and edge cases; this spec scopes ownership and execution, without adding a second interpretation. Every pending ACK-required message has a send-anchored original deadline and is scanned for overdue warning without requiring recipient availability.

## Tasks

### Canonical send-time receipt projection

owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.

### Due discovery and originating-seat warning lifecycle

owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.

## Validation and failure handling

Each task runs its listed configurations before terminal integration with isolated config/daemon/host fixtures and owned test-support children. Preserve immutable keyed historical requests and terminal outcomes; new incompatible fields negotiate before durable effects. All implementation follows AGENTS.md lint/format/test and TRUST-POLICY cooperative canonical authority. No real user config edits, automatic trust or shared server management.

## Promotion refinement

Historical upgrade is a separate leaf ht-0lo.1.3 consuming ht-0lo.1.1. Due discovery ht-0lo.1.2 consumes both the projection and upgrade so its historical discovery scenarios are satisfiable. Handoff ht-0lo.3.2 still depends on ht-0lo.1.1 unchanged: it consumes only new-send semantics, not historical migration.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
