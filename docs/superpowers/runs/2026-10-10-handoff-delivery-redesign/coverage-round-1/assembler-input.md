# FROZEN COVERAGE ROUND 1 INPUT
Full spec prose and complete bulk tree follow. Review root plus receipts/invitations/handoff/hooks as FIVE separate passes. Subepic pass includes local epic+descendants and external dependency definitions supplied below, with root Goal as parent chain. Root includes all. No flags, previous changes or ledger. Canonical requirements apply ONLY to root. Integration sweep intentionally created AFTER loop; absence is not a finding.

# SPEC ROOT
# Handoff delivery redesign

## Goal

A sender can durably hand off complete instructions and receive a bounded-time warning when receipt or explicitly requested invitation acceptance is unconfirmed, even if the recipient never registers or becomes available. Native event hook commands remain stable across equivalent reinstalls without weakening ownership, routing, launch or receipt safeguards.

## Context and prior decisions

Baseline main 7c40c83c028d4a1a7650d6a58e92cd0bdedd3cdb. The existing CLI names/handoff and topology-handoff designs already establish thread-carried bodies, exact keyed child retries, managed-launch binding, fixed inbox startup, structural guards and administrative unknown-create recovery. Preserve those contracts. Lazy message and hook-delivery designs distinguish presentation from receipt; reuse this distinction for invitations. The user collaboratively chose send-time message deadlines and a single invitation acceptance obligation, then authorized fully autonomous implementation with both roasts. Do not resume the historical topology run or depend on unmerged automatic 1:1 thread work ht-e99.

## Options and decisions

1. Add a special launch-confirmation timer, or repair ordinary receipt deadlines. Choose ordinary receipt deadlines: all ACK-required sends have the same bounded receipt semantics and existing joined-but-unavailable seats are covered.
2. Add registration confirmation plus invitation acceptance, or one invitation obligation. Choose one obligation settled by explicit acceptance; registration alone proves neither receipt nor participation.
3. Keep random shell owner comments, use stable sidecar scripts, or dedicated CLI event entrypoints. Choose stable CLI entrypoints with manifest-owned registrations; sidecar scripts introduce another executable lifecycle and random comments alter native trust keys.
4. Unlimited bodies or normal message bounds. Choose the existing 65,536 UTF-8 byte message bound; remove only the arbitrary handoff 1,024-byte body restriction. Topic and goal retain their existing limits.

## Message deadline contract

For every new durable ACK-required ordinary send, freeze the duration using existing validation/default configuration and set its original deadline to canonical committed send time plus that duration. This applies to every recipient, including no binding, managed-launch/unregistered binding, unavailable binding and pending invitation. Availability never changes the original start/deadline, and cannot reset it on registration, accept, restart, or rebind. Lazy messages and human receipt waivers remain free of ACK obligations. Receipt still means receipt only.

The effective receipt projection, manifest preparation, sparse physical materialization, due cursors, soft-poke calculations and wake paths must agree. Keep availability separately meaningful: setting a deadline is not permission to wake, nor evidence of a live harness. Do not pretend send time is observed availability. Existing structural/native execution launch guards and wake harness/empty-composer guards remain mandatory.

Upgrade outstanding pending ACK-required messages too: preserve every previously running deadline, but give formerly dormant obligations the deadline derived from their immutable message send time and frozen duration. Already expired rows become due on the normal bounded scan. Preserve terminal states, receipt provenance and warning identities. A transactionally recorded schema/semantics marker makes old and new manifest/physical projections consistent; materialize/index due work so unavailable recipients are actually discoverable, not merely capable of returning a deadline on individual lookup. Do not rewrite immutable requests or old result digests.

Existing explicit summary catch-up extensions in TRUST-POLICY A6 remain separate and visible; this redesign removes availability-based deferral, not the policy's bounded progress-based extension. Tests distinguish original deadline from an explicitly extended effective deadline. Warning delivery to the originating seat is durable even if it later leaves the thread; joined observers may retain existing warning visibility. An overdue warning says confirmation missed its deadline, never why. Late ACK clears the existing warning through normal lifecycle without retroactively changing the deadline.

## Invitation contract

Ordinary agent invitations default to Lazy: membership is pending and discoverable through existing inbox/hook opportunities, but the invitation itself requests no timed confirmation or native wake. Add invitation `--require-ack` and optional `--deadline SECONDS`; explicit deadline implies require-ack for ergonomic compatibility, and `--lazy` conflicts with either. An ACK-required invitation starts its acceptance deadline at canonical invitation creation. Only explicit recipient acceptance settles it automatically; no separate invitation ACK command and no registration settlement. Expiry notifies its originating seat with invitation deadline missed, regardless of failure to register or accept. Rejection/cancellation/retirement retain existing truthful terminal outcomes and attributed rejection notices; lazy does not suppress explicit rejection notices.

Existing required service memberships and explicit human/operator invitation semantics remain unchanged. Existing ordinary pending invitations retain their current timed obligation; historical retry with omitted new fields retains its original interpretation. Use an additive optional delivery/confirmation field whose absence denotes legacy timed behavior; modern CLI explicitly emits Lazy for the new default. Persist requested timing and original inviter attribution independently of joined membership. Pending-invite duplicate calls return the existing invitation without silently upgrading/downgrading its deadline or waking it; changing obligation policy requires a new episode through existing disposition flow. Already joined is still AlreadyJoined, never a new obligation. Existing human mode cannot manufacture an agent ACK requirement.

Queries, inbox compact/machine/JSON, digest, mod/native wake selection and due scans all honor the requested mode. Lazy presentation is bookkeeping only. ACK-required invitation attention remains ordinary guarded delivery; accept remains an accountable recipient choice. Add schema fields/index predicate changes conservatively and negotiate protocol capability/version before any mutation requiring the new representation. Older saved intent bytes/digests are preserved and replayed unchanged; unsupported daemon refuses new semantics before writing.

## Durable handoff interface and recovery

Support exactly one instruction source: the legacy quoted positional body after `--`, `--file PATH`, or `--stdin`. Read once before any durable effect, require nonempty valid UTF-8 within 65,536 bytes, preserve exact content/newlines, bound input while reading, and freeze actual bytes into the compound intent. Retrying never rereads a file/stdin or native environment options. Keep topic/goal validation separate. All three modes (new-tab, explicit empty-pane launch, existing-recipient delivery) use the same body/deadline contract and exact keys.

Handoff always sends an ACK-required addressed instructions message, defaulting to the existing receipt-duration setting; add `--deadline SECONDS` to override that message deadline, frozen before staging. Its invitation is lazy unless the caller explicitly requests `--require-invite-ack`; add `--invite-deadline SECONDS` to imply that option. Message and invitation deadlines are independent. Stage messages while invitation is pending; sender already joined is required. Carry policy through all public request, preparation, canonical bootstrap, legacy handoff, existing delivery and retained retry representations. Omitted new fields in historical intents preserve historical child requests/digests. No restaging of completed child mutations or automatic launch after possible-start.

Native startup contains only fixed inbox instructions, exact canonical routing metadata and pinned fallback. It contains no task body, body path or arbitrary task argv. Compare all three routing fields before preferring hook commands. Existing launched agents with changing harness versions still recover instructions from durable inbox; setup diagnostics/trust refusals do not discard them. Hook/setup/profile readiness inspection is diagnostic and does not block staging or an otherwise safe guarded launch: observed hook installation/trust cannot establish delivery, and a wrong inherited profile must not veto launching a user-selected native harness. Retain exact empty-shell/seat/caller/native argv safety guards; native harness approval/configuration refusals remain truthful refusals, never automatically answered. ACK or overdue warning confirms the bounded handoff outcome, independent of advisory setup status. Launch success is not registration, acceptance, receipt or task adoption. On missed confirmation the sender inspects and recovers; trust/approval remains user consent, never automatic.

## Stable hook entrypoints and ownership

Expose `herdr-threads hook-event HARNESS EVENT` as the dedicated stable CLI surface; map each declared native event exactly to its registered codec/handler. Keep existing `hook HARNESS --event EVENT` syntax working for old registrations. Reject invalid harness/event pairs before daemon effects. A given executable spelling plus canonical state-dir/host-endpoint plus harness/event yields byte-identical shell command across reinstalls. Routing selectors remain pinned where required; different installations/routes must not silently collapse.

Remove the per-install UUID shell comment from newly generated command strings, not the manifest installation identity. Persist exact installed groups/fingerprints and installation ID in ownership records. Existing unmanaged identical groups are foreign: do not adopt, overwrite, remove or treat them as owned without the existing explicit adoption path. Conflicting same-event registrations fail safely with actionable diagnostics. Reinstall reuses its existing exact owned group, retaining position to avoid unnecessary native trust-key shifts. Migrate legacy UUID-marked or shared-event owned groups only under explicit setup/doctor-fix intent, preserving unrelated groups/keys and interrupted-update recovery. Unsetup removes only exactly owned records; drift fails safely. No automatic edit of native trust records, hooks.state, or permission grants. Update status/trust reporting to compare each event's actual stable command rather than infer one shared string. Never silently rewrite real user configurations during development.

The Claude and Codex registrations get their declared per-event entrypoints. Hermes callback transport remains its existing fixed Python bridge; do not mechanically apply native shell grammar to it. Any bundled launch/mod prefix that currently consumes hook argv continues to obtain the canonical route rather than an event command. Registry manifests/canary expectations and embedded skill/help must agree with stable generated commands. TRUST-POLICY records invitation confirmation meaning and repaired receipt timer semantics; no new receipt provenance is needed for invitation acceptance.

## Implementation tree and ownership

The root epic has four cohesive leaves before its terminal sweep:

- Receipt deadlines: owns canonical send-time/upgrade receipt semantics, due materialization and originating-seat warning routing; consumes existing preparation/projection schema. Files: src/store/{effective,materialization,receipts,messages,poke,wake,schema,queries}.rs, relevant send preparation modules and tests, TRUST-POLICY.md deadline clauses. Exercise unavailable/unbound/invited send+expiry, sparse/physical/restart, human/lazy exemptions and catch-up distinction before integration.
- Invitation lifecycle: owns invitation mode/duration/acceptance contract, protocol capability and inviter warning routing through CLI/store/query/wake; consumes existing membership lifecycle. Files: src/protocol/{commands,results,capabilities}.rs and wire negotiation, src/cli parser invitation branches, src/store/{control,invitation_due,wake,queries,schema}.rs, relevant tests, TRUST-POLICY invitation clauses. Exercise bare lazy invite, explicit timed invite, never-register expiry, register-without-accept expiry, acceptance/rejection, duplicates, legacy retry and required memberships before integration.
- Handoff payload: owns source parsing, 65,536-byte body/deadline freezing and pass-through into all handoff child requests; consumes receipt deadlines and invitation confirmation wire contract. Files: src/cli/{handoff,handoff_delivery,topology_handoff,parse_launch,mod}.rs as applicable, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs, help/embedded skill and focused CLI/recovery tests. Exercise positional/file/stdin through new-tab/empty-pane/existing delivery with isolated fake host/harness, unknown retries and routing before integration.
- Stable hooks: owns native event command surface, exact command generation, owned migration/unsetup/status/canary/launcher consistency; consumes existing registered harness codec declaration. Files: src/cli/{hook,setup,mod}.rs, src/harness/{setup,claude,codex}.rs and setup submodules, installed manifest/canary/tooling tests and docs. Exercise Claude/Codex per-event dispatch/setup repeat/migration/unsetup in isolated config; verify Hermes and mod route remain compatible before integration.

Receipt/invitation work can proceed independently; handoff consumes their landed semantics. Stable hooks has no logical dependency. Shared files serialize at execution's merge gate. The final Integration sweep depends on all four leaves, owns remaining inter-feature wiring/docs and exact-tip lint/default-feature checks plus focused end-to-end sender timeout/long inbox recovery scenarios. It adds tests for uncovered seams and fixes small gaps; broader full suite only if project constraints justify it, never routinely until ht-zo4 lands. No activation, live-config migration or merge into base belongs to this tree.

## Acceptance and validation

- Long instructions survive restart/retry unchanged; native argv excludes the body. Invalid, oversized, conflicting or unreadable sources fail before durable writes.
- New and previously dormant ACK obligations expire without recipient activity; already running deadlines do not move. Late registration/acceptance never restarts the clock. Due scans are bounded/indexed, sender receives warnings, late receipt clears normally.
- Lazy ordinary invitation produces no overdue/native wake; explicitly timed invitation warns the originator and acceptance settles automatically. No acceptance/ACK is synthesized by launch or registration.
- Equivalent reinstall emits identical per-event commands while foreign and drifted configuration survives; explicit legacy migration is recoverable and honest about native trust needing review.
- Existing bootstrap unknown-create/possible-start guarantees, human waivers, receipt provenance, required memberships, routing and hook trust boundaries continue passing their focused regression tests.
- Run cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and focused tests per change; before merge readiness also nice scripts/check-default-features. Use test-support-owned children/private Herdr sessions, stop all owned processes, and perform scoped leak checks after any full suite.

## Risks and mitigations

Backdating dormant obligations may produce an upgrade-time warning burst: bound scan work, preserve exact warning identities and explain historical send timestamps; do not silently extend them. Stable commands weaken implicit ownership hints if manifest checks are relaxed: retain exact manifest identity and conservative foreign-group detection. Additive fields may invalidate legacy digests or old peers: omit absent fields, explicitly default legacy semantics and negotiate before writes. Shared hot files reduce parallelism: execution serializes merges and reviews combined changes.

## Non-goals and follow-on

No auto-trust, recipient auto-accept, task-adoption ACK, delivery exactly-once claim, automatic 1:1 thread selection, server restart, seat repair heuristics, real user config writes, deployment or merge into main. The parent run notifies its established main-tab discussion when the integration branch is ready for review/merge.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

# SPEC --receipts
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

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

## Promotion refinement

Historical upgrade is a separate leaf ht-0lo.1.3 consuming ht-0lo.1.1. Due discovery ht-0lo.1.2 consumes both the projection and upgrade so its historical discovery scenarios are satisfiable. Handoff ht-0lo.3.2 still depends on ht-0lo.1.1 unchanged: it consumes only new-send semantics, not historical migration.

# SPEC --invitations
# Handoff delivery redesign: invitations

## Goal

Ordinary invitations are lazy unless confirmation was explicitly requested; timed invitations settle at acceptance and notify originator on expiry independently of registration.

Parent: [root design](2026-10-10-handoff-delivery-redesign-design.md); bead ht-0lo.2.

## Decisions and boundaries

The parent spec is the normative behavioral contract for this subtree. It specifies the selected alternatives, compatibility rules and edge cases; this spec scopes ownership and execution, without adding a second interpretation. Ordinary invitations are lazy unless confirmation was explicitly requested; timed invitations settle at acceptance and notify originator on expiry independently of registration.

## Tasks

### Invitation confirmation protocol and schema contract

owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.

### Invitation acceptance, due and delivery lifecycle

owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.

### Invitation CLI and presentation contract

owns: ordinary invite --lazy default, --require-ack/--deadline implication and conflicts, compact/JSON/inbox/help/embedded skill presentation; consumes: invitation confirmation protocol/store contract. Files: src/cli parser invitation branches/mod/output and help/skill resources, relevant CLI tests. Invoke actual CLI in isolated daemon/config for bare lazy, require-ack default duration, explicit deadline, conflicting flags, duplicate and historical keyed retry. Required/operator commands remain unchanged.

## Validation and failure handling

Each task runs its listed configurations before terminal integration with isolated config/daemon/host fixtures and owned test-support children. Preserve immutable keyed historical requests and terminal outcomes; new incompatible fields negotiate before durable effects. All implementation follows AGENTS.md lint/format/test and TRUST-POLICY cooperative canonical authority. No real user config edits, automatic trust or shared server management.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

# SPEC --handoff
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

# SPEC --hooks
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

# COMPLETE BULK TASK TREE

## ht-0lo (epic) Handoff delivery redesign
Deliver thread-carried complete handoff instructions with send-time receipt deadlines, lazy optional-confirmation invitations and stable native hook entrypoints. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md. User authorized autonomous implementation; integration merge remains coordinator decision.
Acceptance: 
Blocking dependencies: (none)

## ht-0lo.1 (epic) Receipt deadlines
owns: canonical send-time/upgrade receipt semantics, due materialization and originating-seat warning routing; consumes: existing preparation/projection schema. Files: src/store/{effective,materialization,receipts,messages,poke,wake,schema,queries}.rs, relevant send preparation modules and tests, TRUST-POLICY.md deadline clauses. Exercise unavailable/unbound/invited send+expiry, sparse/physical/restart, human/lazy exemptions and catch-up distinction before integration.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md
Acceptance: Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.
Blocking dependencies: (none)

## ht-0lo.1.1 (task) Canonical send-time receipt projection
owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: (none)

## ht-0lo.1.2 (task) Due discovery and originating-seat warning lifecycle
owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md
blocked-by ht-0lo.1.1: consumes send-anchored canonical receipt projection
blocked-by ht-0lo.1.3: consumes historical dormant receipt upgrade
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: ht-0lo.1.3, ht-0lo.1.1

## ht-0lo.1.3 (task) Upgrade dormant historical receipt obligations
owns: transactional upgrade/backfill of formerly dormant historical ACK obligations, sparse/physical due projection consistency and idempotent schema marker; consumes: canonical send-time receipt projection. Preserve already running deadlines, terminal states/provenance and immutable request/result digests. Files: src/store/{schema,effective,materialization}.rs and focused migration/store tests. Exercise legacy unavailable receipts upgrade/restart/replay and overdue indexed discovery, bounded warning burst.
blocked-by ht-0lo.1.1: consumes canonical send-time receipt projection
Acceptance: Legacy dormant physical/manifest ACK obligations receive send-time deadlines while existing started/terminal/waived obligations remain unchanged. Idempotent restart and focused migration tests pass.
Blocking dependencies: ht-0lo.1.1

## ht-0lo.2 (epic) Invitation lifecycle
owns: invitation mode/duration/acceptance contract, protocol capability and inviter warning routing through CLI/store/query/wake; consumes: existing membership lifecycle. Files: src/protocol/{commands,results,capabilities}.rs and wire negotiation, src/cli parser invitation branches, src/store/{control,invitation_due,wake,queries,schema}.rs, relevant tests, TRUST-POLICY invitation clauses. Exercise bare lazy invite, explicit timed invite, never-register expiry, register-without-accept expiry, acceptance/rejection, duplicates, legacy retry and required memberships before integration.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md
Acceptance: Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.
Blocking dependencies: (none)

## ht-0lo.2.1 (task) Invitation confirmation protocol and schema contract
owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: (none)

## ht-0lo.2.2 (task) Invitation acceptance, due and delivery lifecycle
owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md
blocked-by ht-0lo.2.1: consumes invitation mode schema/wire contract
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: ht-0lo.2.1

## ht-0lo.2.3 (task) Invitation CLI and presentation contract
owns: ordinary invite --lazy default, --require-ack/--deadline implication and conflicts, compact/JSON/inbox/help/embedded skill presentation; consumes: invitation confirmation protocol/store contract. Files: src/cli parser invitation branches/mod/output and help/skill resources, relevant CLI tests. Invoke actual CLI in isolated daemon/config for bare lazy, require-ack default duration, explicit deadline, conflicting flags, duplicate and historical keyed retry. Required/operator commands remain unchanged.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md
blocked-by ht-0lo.2.2: consumes invitation confirmation store behavior
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: ht-0lo.2.2

## ht-0lo.3 (epic) Handoff payload
owns: full frozen handoff payload and policy integration; consumes: send-time receipt semantics and invitation confirmation wire contract. Files: src/cli handoff paths, src/protocol/handoff.rs, src/store handoff paths and tests. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md
Acceptance: Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.
Blocking dependencies: (none)

## ht-0lo.3.1 (task) Handoff instruction source acquisition and frozen payload
owns: positional/file/stdin source exclusivity, bounded nonempty UTF-8 65,536-byte acquisition and exact bytes frozen in all handoff intent forms; consumes: existing compound intent/body schema. Files: src/cli/handoff.rs parser and validation, src/cli/{handoff_delivery,topology_handoff}.rs saved plans and associated parser sources/tests. Preserve topic/goal limits separately; invalid sources fail before mutations; mutate/delete source after initial staging and prove retry keeps original content. Native argv contains fixed routing/inbox only.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: (none)

## ht-0lo.3.2 (task) Handoff confirmation policy wiring and recovery documentation
owns: handoff --deadline/--require-invite-ack/--invite-deadline parsing/freeze/pass-through, modern lazy invitation plus addressed ACK-required body staging across new-tab/empty-pane/existing paths, retained child digest compatibility and source/help/skill docs; consumes: frozen handoff payload, invitation confirmation wire contract and send-time receipt semantics. Files: src/cli/{handoff,handoff_delivery,topology_handoff,mod}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and related tests/help/skill. Run isolated actual handoff per all three paths/body sources; crash/retry after stage and possible-start/unknown-create never duplicates/restages/relaunches. Older plans absent new fields replay original keyed requests, modern deadlines freeze before staging; routing exact3field fallback and no auto trust/accept/ACK.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md
blocked-by ht-0lo.3.1: consumes frozen handoff payload
blocked-by ht-0lo.2.1: consumes invitation confirmation wire contract
blocked-by ht-0lo.1.1: consumes send-time receipt semantics
Also owns: separating hook/setup/trust/profile readiness diagnostics from launch admission; src/cli/launch.rs prepare_launch and src/harness/launch.rs configuration checks. Missing/untrusted/wrong-profile hooks do not block staged work or otherwise structurally safe launch. Preserve empty-shell/seat/caller/native argv and owned launch-configuration fingerprint safety, and truthful native trust refusal; never auto-approve. Test missing/stale profile hooks through guarded handoff and observe durable ACK deadline warning without registration.
blocked-by ht-0lo.2.2: consumes canonical invitation lifecycle
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: ht-0lo.1.1, ht-0lo.2.2, ht-0lo.2.1, ht-0lo.3.1

## ht-0lo.4 (epic) Stable hooks
owns: native event command surface, exact command generation, owned migration/unsetup/status/canary/launcher consistency; consumes: existing registered harness codec declaration. Files: src/cli/{hook,setup,mod}.rs, src/harness/{setup,claude,codex}.rs and setup submodules, installed manifest/canary/tooling tests and docs. Exercise Claude/Codex per-event dispatch/setup repeat/migration/unsetup in isolated config; verify Hermes and mod route remain compatible before integration.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md
Acceptance: Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.
Blocking dependencies: (none)

## ht-0lo.4.1 (task) Stable native event dispatch and command generation
owns: hook-event HARNESS EVENT parser/registered-codec dispatch and stable per-event shell command grammar; consumes: existing harness event registry and canonical routing. Files: src/cli/{hook,mod,setup}.rs, src/harness/{setup,claude,codex}.rs, registry declarations/tests. Keep legacy hook syntax; validate invalid pairs before effects; command stability includes executable/route/harness/event only. Exercise declared Claude/Codex events with fixtures through dedicated entrypoints; retain Hermes callback and bundled mod route compatibility.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: (none)

## ht-0lo.4.2 (task) Owned stable registration migration and tooling
owns: manifest-recorded exact ownership/install identity, repeat setup in place, recoverable UUID/shared-command migration, safe unsetup and status/trust/canary/launcher command compatibility; consumes: stable native per-event command grammar. Files: src/harness/setup.rs and claude/codex setup modules, src/cli setup/launch consumers, canary/tooling/docs/tests. Invoke isolated Claude/Codex setup-repeat/migration/interruption/unsetup with foreign identical/drifted groups and ensure no ownership theft, no trust writes, no group shifting on equivalent reinstall. Hermes/Claude mod regressions stay passing.
Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md
blocked-by ht-0lo.4.1: consumes stable native per-event command grammar
Acceptance: Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.
Blocking dependencies: ht-0lo.4.1

# CANONICAL ROOT REQUIREMENTS
R1: New ordinary ACK-required messages have an original deadline at canonical committed send time plus frozen configured/validated duration.
R2: No recipient binding, availability, registration, acceptance, restart or rebind delays or resets the original message deadline.
R3: Lazy messages and human receipt waivers have no ACK obligation; receipt preserves its existing meaning and provenance.
R4: Canonical/effective projection, preparation, sparse/physical materialization, due cursors, soft poke and wake calculations agree without fabricating availability.
R5: Upgrade preserves running deadlines and terminal states while transactionally converting formerly dormant obligations using immutable send time and duration without rewriting request bytes/digests.
R6: Unavailable recipients have bounded indexed due discovery, including already-expired upgraded obligations.
R7: Explicit A6 catch-up extensions remain separate and visible from original deadlines.
R8: Originating-seat message warnings survive leaving the thread, make no causal claim, and clear on late ACK without deadline changes.
R9: Ordinary modern invitations default to Lazy with pending discoverability and no timed confirmation/native wake.
R10: Invitation --require-ack and --deadline request timed confirmation; deadline implies require-ack and --lazy conflicts with either.
R11: Timed invitation deadlines start at canonical creation and settle automatically only on explicit acceptance; registration is insufficient.
R12: Invitation expiry durably warns its originating seat regardless of registration or membership.
R13: Rejection/cancellation/retirement and attributed rejection notices retain truthful outcomes, including Lazy invitations.
R14: Required service memberships and human/operator invitation semantics remain unchanged; human mode cannot manufacture agent ACK obligations.
R15: Historical pending invitations/intents retain legacy timed interpretation, original bytes/digests and inviter attribution.
R16: Pending-invite duplicate calls preserve policy/deadline/wake behavior; already-joined creates no obligation.
R17: Invitation queries, compact/machine/JSON inbox, digest, mod/native wake and due scans honor requested mode.
R18: New invitation representations negotiate capability/version before mutation and refuse unsupported semantics before writes.
R19: Handoff accepts exactly one positional/file/stdin body source, bounded while reading, nonempty valid UTF-8 <=65,536 bytes, preserving exact content and separate topic/goal limits.
R20: Handoff freezes actual body bytes and native options before durable effects; retries never reread sources/environment.
R21: New-tab, empty-pane launch and existing-recipient handoff share body/deadline contracts and exact keys.
R22: Handoff always stages addressed ACK-required instructions with configurable --deadline, independent Lazy/default or explicitly timed invitation policy.
R23: Handoff policy flows through all public/preparation/bootstrap/legacy/existing/retry representations while preserving omitted historical fields and completed keyed child mutations.
R24: Handoff stages while invitation pending, requires joined sender and never auto launches after possible-start.
R25: Native startup contains fixed inbox instructions, exact canonical routing and pinned fallback, no body/path/arbitrary task argv; all three route fields are compared.
R26: Hook/setup/trust/profile readiness stays diagnostic, so staged work and structurally safe selected-harness launch proceed while structural/caller/empty-shell/native-argv guards and truthful native consent refusals remain.
R27: Handoff ACK or missed-confirmation warning is independent of launch, registration, acceptance or task adoption.
R28: Stable hook-event HARNESS EVENT dispatches registered per-event codecs, retains legacy hook syntax and rejects invalid pairs before daemon effects.
R29: Equivalent executable/route/harness/event commands are byte-identical across reinstall without collapsing distinct installations/routes.
R30: Exact installed group fingerprints and installation identity establish ownership, preserving foreign identical/unrelated groups and failing conflicts safely.
R31: Equivalent reinstall reuses exact owned group in position; explicit legacy UUID/shared-event migration is interruption recoverable.
R32: Unsetup removes only exact owned records, drift fails safely and no native trust/permission records are automatically edited.
R33: Status/trust compares actual per-event stable commands; Claude/Codex events, Hermes fixed bridge and bundled launch/mod canonical route remain compatible.
R34: Registry manifests/canaries/help/embedded skills and TRUST-POLICY agree with new command, invitation and timer semantics.
R35: Focused isolated configuration exercises and format/clippy checks cover changed behavior before terminal integration; default-feature checks establish merge readiness and owned test processes are stopped.
