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

## Coverage refinements


Also owns: body-bearing envelope capacity calculation and limits: bootstrap identity/progress512KiB, recovery1MiB; preserve metadata-only creation/attachment128KiB, completed2MiB/globalwire1MiB unless actual serializer worst-case fixtures prove a narrowly scoped bound adjustment necessary. Files additionally src/store/topology_handoff.rs and persistence.rs, src/cli/journal.rs, TRUST-POLICY encoded-limit clauses. Reuse src/cli/input.rs; protocol body validator must follow normal nonempty UTF-8 message bytes (NUL/whitespace are not identifier validation). Inventory all repeated bodies/nested originals; actual serializer64KiB six-byte escape public wire/canonical/local retained/retry fixtures prove support under ordinary bounded metadata. Exact serialized preflight before effects; retain historical bytes/digests and no clipping.
blocked-by ht-0lo.1.2: consumes overdue receipt warning lifecycle
Hook readiness is advisory: keep owned launch configuration fingerprint recheck only when that configuration is actually used; absence cannot veto and no fingerprint is fabricated. Unsupported environment-overrides/native grammar/host-kind safety refusals remain.

## Confirmed roast refinements

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


## Authorized upstream composition refinement — supersedes automatic invitation-flag refusal

Parent fact-driven ruling: Ensure actually creates the initial invitation. Automatic handoff supports --require-invite-ack and --invite-deadline for INITIAL pair creation, independently from the always-addressed ACK-required body and frozen message deadline. Add optional frozen Ensure creation policy Lazy/RequireAck plus duration; ABSENT means historical timed behavior and preserves old bytes/digests/keys. Modern direct send explicitly requests Lazy for a NEW pair invitation. Modern automatic handoff defaults that initial invitation to Lazy and can request timed acceptance. Registration/launch is never acceptance or ACK.

Preserve canonical pair lifecycle exactly: existing joined/pending episodes are reused without policy/deadline/origin changes; rejected/left pairs refuse without reinvite. A later caller requesting different creation policy cannot override/reset an existing episode. Present the ACTUAL retained policy/outcome honestly; AlreadyJoined creates no new invitation/obligation. Do not manufacture a separate Invite child: automatic remains Ensure→Send with exact canonical attachment/publication/completion proof. Existing explicit paths retain their Invite children. Test initial Lazy/RequireAck/default-duration/explicit-duration, historical absent exact bytes, duplicate policy mismatch retaining original, AlreadyJoined, rejected/left refusal and independent message/invitation deadlines (7/11 seconds).

All automatic and explicit handoff routes acquire exactly one nonempty valid UTF-8 source <=65536 bytes once, freeze exact body/policy/durations before effects, preserve NUL/whitespace/newlines and retry without rereading. Require send-time/invitation/automatic/direct capabilities as applicable BEFORE journal/allocation/Ensure/topology/native effects. Hook/setup/trust/profile readiness is advisory; structural/caller/seat/native grammar/actually-used owned fullconfig fingerprint checks and truthful native trust refusal remain mandatory. No auto trust/accept/ACK/adoption.

Released main478ec4a9 migrations0029_direct_threads and0030_automatic_handoff remain exact. Our unlanded29..35 shift31..37; add migration38 for automatic body-bearing original/recovery capacities proven with actual serializers, never rewrite released30 SQL. Preserve historical retained rows/requests/digests, genuine main30→combined upgrade/rollback/reopen and all dependent FK relationships. Count nested body copies across child/attachment/recovery/terminal/archival and enforce exact limits before effects; no clipping/unconditional globalwire increase.

Owned composition epic ht-0lo.8 has serial leaves .8.1/.8.2/.8.3, plan ordinals12/13/14. Work remains in separate isolated candidate workspace, distinct task branches/provenance reusing one owned warm validation slot serially. No provisional or unresolved composition merges into root integration until the whole reviewed A/B/C contract passes. Root closure waits. Parent owns final PR roast and subsequent expensive/defaultfeature exact-tip readiness. Prior11 ordinals and three edge-audit budget remain unchanged.


### Ensure creation-policy capability refinement

New optional Ensure creation-policy fields require their OWN honest additive capability, provisionally direct_threads.ensure_creation_policy_v1; invitation.confirmation_v1 and old automatic/direct capability do not establish support in released v0.7.0 Ensure handlers. Task13 owns constant, canonical activation/advertisement and every modern caller preflight. Advertise only when canonical Ensure policy creation is actually active. Modern Lazy directsend remains exempt from messages.send_time_receipts_v1 but MUST require this new Ensure-policy capability BEFORE journal/pair/allocation/native effects; modern automatic policy likewise requires it before any topology/durable/native effects, in addition to applicable automatic/invitation/receipt capabilities. Explicit historical absent fields preserve historical behavior/bytes and completed inspection; unsupported modern semantics never silently fall back. Test real old-v0.7 capability peers for automatic and direct routes, INCLUDING Lazy directsend, and assert no new journal/pair/topology/native effects. Task12 supplies the actual schema/canonical activation-ready cutpoint if needed with additive38; do not invent extra DDL merely to gate a capability. This is parent-authorized autonomous bounded composition within ht-0lo.8; no new confirmation request is required.
