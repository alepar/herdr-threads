# Handoff delivery redesign: handoff

## Goal

Complete bounded instructions and explicit confirmation policy freeze once into durable handoff retry across every delivery/launch path.

Parent: [root design](2026-10-10-handoff-delivery-redesign-design.md); bead ht-0lo.3.

## Decisions and boundaries

The parent spec is the normative behavioral contract for this subtree. It specifies the selected alternatives, compatibility rules and edge cases; this spec scopes ownership and execution, without adding a second interpretation. Complete bounded instructions and explicit confirmation policy freeze once into durable handoff retry across every delivery/launch path.

## Tasks

### Handoff instruction source acquisition and frozen payload

owns: positional/file/stdin source exclusivity, bounded nonempty UTF-8 65,536-byte acquisition and exact bytes frozen in all handoff intent forms; consumes: existing compound intent/body schema. Files: src/cli/handoff.rs parser and validation, src/cli/{handoff_delivery,topology_handoff}.rs saved plans and associated parser sources/tests. Preserve topic/goal limits separately; invalid sources fail before mutations; mutate/delete source after initial staging and prove retry keeps original content. Native argv contains fixed routing/inbox only.

### Handoff confirmation policy wiring and recovery documentation

owns: handoff --deadline/--require-invite-ack/--invite-deadline parsing/freeze/pass-through, modern lazy invitation plus addressed ACK-required body staging across new-tab/empty-pane/existing paths, retained child digest compatibility and source/help/skill docs; consumes: frozen handoff payload, invitation confirmation wire contract and send-time receipt semantics. Files: src/cli/{handoff,handoff_delivery,topology_handoff,mod}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and related tests/help/skill. Run isolated actual handoff per all three paths/body sources; crash/retry after stage and possible-start/unknown-create never duplicates/restages/relaunches. Older plans absent new fields replay original keyed requests, modern deadlines freeze before staging; routing exact3field fallback and no auto trust/accept/ACK.


Also owns: separating hook/setup/trust/profile readiness diagnostics from launch admission; src/cli/launch.rs prepare_launch and src/harness/launch.rs configuration checks. Missing/untrusted/wrong-profile hooks do not block staged work or otherwise structurally safe launch. Preserve empty-shell/seat/caller/native argv and owned launch-configuration fingerprint safety, and truthful native trust refusal; never auto-approve. Test missing/stale profile hooks through guarded handoff and observe durable ACK deadline warning without registration.

Policy integration also consumes canonical invitation lifecycle ht-0lo.2.2; its existing wire edge ht-0lo.2.1 remains for the frozen representation.

## Validation and failure handling

Each task runs its listed configurations before terminal integration with isolated config/daemon/host fixtures and owned test-support children. Preserve immutable keyed historical requests and terminal outcomes; new incompatible fields negotiate before durable effects. All implementation follows AGENTS.md lint/format/test and TRUST-POLICY cooperative canonical authority. No real user config edits, automatic trust or shared server management.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
