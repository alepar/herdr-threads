SETTLED COMMIT f5384a27


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md
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


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md
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

Existing explicit summary catch-up extensions in TRUST-POLICY A6 remain separate and visible; this redesign removes availability-based deferral, not the policy's bounded progress-based extension. Tests distinguish original deadline from an explicitly extended effective deadline. Warning delivery to the originating seat is durable even if it later leaves the thread; joined observers may retain existing warning visibility. An overdue warning says confirmation missed its deadline, never why. The original sender's open warning is waking attention under the existing composer/host/mod admission guards, even if the sender is no longer joined; clear remains passive and observer visibility is unchanged. For non-seat service authors preserve existing service reporting rather than inventing a sender seat. Update TRUST-POLICY A7 to record this narrow new waking recipient rule. Native wake can be deferred by a draft, active turn or unavailable host; the durable warning remains pending, so bounded deadline detection does not promise bounded native key submission. Late ACK clears the existing warning through normal lifecycle without retroactively changing the deadline.

## Invitation contract

Ordinary agent invitations default to Lazy: membership is pending and discoverable through existing inbox/hook opportunities, but the invitation itself requests no timed confirmation or native wake. Add invitation `--require-ack` and optional `--deadline SECONDS`; explicit deadline implies require-ack for ergonomic compatibility, and `--lazy` conflicts with either. An ACK-required invitation starts its acceptance deadline at canonical invitation creation. Only explicit recipient acceptance settles it automatically; no separate invitation ACK command and no registration settlement. Expiry notifies its originating seat with invitation deadline missed, regardless of failure to register or accept. The inviter's open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear remains passive and existing debtor/observer visibility remains. Originator attribution is frozen from the original invitation episode, never inferred from a current member or a later duplicate caller. Rejection/cancellation/retirement retain existing truthful terminal outcomes and attributed rejection notices; lazy does not suppress explicit rejection notices.

Existing required service memberships and explicit human/operator invitation semantics remain unchanged. Existing ordinary pending invitations retain their current timed obligation; historical retry with omitted new fields retains its original interpretation. Use an additive optional delivery/confirmation field whose absence denotes legacy timed behavior; modern CLI explicitly emits Lazy for the new default. Persist requested timing and original inviter attribution independently of joined membership. Pending-invite duplicate calls return the existing invitation without silently upgrading/downgrading its deadline or waking it; changing obligation policy requires a new episode through existing disposition flow. Already joined is still AlreadyJoined, never a new obligation. Existing human mode cannot manufacture an agent ACK requirement.

Queries, inbox compact/machine/JSON, digest, mod/native wake selection and due scans all honor the requested mode. Lazy presentation is bookkeeping only. ACK-required invitation attention remains ordinary guarded delivery; accept remains an accountable recipient choice. Add schema fields/index predicate changes conservatively and negotiate protocol capability/version before any mutation requiring the new representation. Older saved intent bytes/digests are preserved and replayed unchanged; unsupported daemon refuses new semantics before writing.

## Durable handoff interface and recovery

Support exactly one instruction source: the legacy quoted positional body after `--`, `--file PATH`, or `--stdin`. Read once before any durable effect, require nonempty valid UTF-8 within 65,536 bytes, preserve exact content/newlines, bound input while reading, and freeze actual bytes into the compound intent. Retrying never rereads a file/stdin or native environment options. Keep topic/goal validation separate. Reuse src/cli/input.rs normal message source validation; body validation is nonempty valid UTF-8, not bounded_text's unrelated topic/identifier trim or NUL restrictions. The source bytes and their JSON escaping are preserved exactly. All three modes (new-tab, explicit empty-pane launch, existing-recipient delivery) use the same body/deadline contract and exact keys.

Handoff always sends an ACK-required addressed instructions message, defaulting to the existing receipt-duration setting; add `--deadline SECONDS` to override that message deadline, frozen before staging. Its invitation is lazy unless the caller explicitly requests `--require-invite-ack`; add `--invite-deadline SECONDS` to imply that option. Message and invitation deadlines are independent. Stage messages while invitation is pending; sender already joined is required. Carry policy through all public request, preparation, canonical bootstrap, legacy handoff, existing delivery and retained retry representations. Omitted new fields in historical intents preserve historical child requests/digests. No restaging of completed child mutations or automatic launch after possible-start.

Raise only body-bearing retained envelope ceilings conservatively: canonical bootstrap identity and request-bearing local bootstrap progress from128KiB to512KiB; recovery from256KiB to1MiB. Keep metadata-only creation/attachment128KiB, completed2MiB and global wire1MiB unless the actual serializer's supported worst-case fixtures demonstrate a required narrowly scoped adjustment. Six JSON bytes per input byte means a65,536-byte body consumes393,216 encoded bytes per occurrence; inventory every duplicated body and nested serialized original rather than assuming one occurrence. The handoff source task owns actual encoded-bound calculation/tests across public wire, canonical identity/recovery/completion, local progress/linked/terminal/origin snapshots and replay. Test maximum ordinary text, maximum six-byte-escaped UTF-8 body and source mutations through each public handoff path; all supported body values under ordinary bounded metadata must stage and survive retry. Exact serialized capacity checks occur before durable effects; independent oversized metadata/native response limits still fail honestly without truncation or reclassification. Update TRUST-POLICY encoded-limit documentation in the implementation commit and preserve historical bytes/keys/digests. No unconditional global frame increase or unbounded reads.

Native startup contains only fixed inbox instructions, exact canonical routing metadata and pinned fallback. It contains no task body, body path or arbitrary task argv. Compare all three routing fields before preferring hook commands. Existing launched agents with changing harness versions still recover instructions from durable inbox; setup diagnostics/trust refusals do not discard them. Hook/setup/profile readiness inspection is diagnostic and does not block staging or an otherwise safe guarded launch: observed hook installation/trust cannot establish delivery, and a wrong inherited profile must not veto launching a user-selected native harness. Retain exact empty-shell/seat/caller/native argv safety guards; fingerprint-recheck any owned launch configuration actually used, but absence of owned hook configuration is not a veto and never gets a fabricated fingerprint. Unsupported environment overrides and native host-kind/grammar capabilities remain safety refusals; native harness approval/configuration refusals remain truthful refusals, never automatically answered. ACK or overdue warning confirms the bounded handoff outcome, independent of advisory setup status. Launch success is not registration, acceptance, receipt or task adoption. On missed confirmation the sender inspects and recovers; trust/approval remains user consent, never automatic.

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

Receipt/invitation work can proceed independently; handoff consumes their landed semantics. Stable hooks has no logical dependency. Shared files serialize at execution's merge gate. The final Integration sweep depends on all implementation leaves, owns remaining inter-feature wiring/docs and exact-tip lint/default-feature checks plus focused end-to-end sender timeout/long inbox recovery scenarios. It adds tests for uncovered seams and fixes small gaps; broader full suite only if project constraints justify it, never routinely until ht-zo4 lands. No activation, live-config migration or merge into base belongs to this tree.

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

## Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md
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

## Coverage refinements


Originator open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear passive; update TRUST-POLICY A7. No invented seat for service authors. Test notification pending while drafts/working/host outage defer actual native wake.

## Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md
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


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md
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

## Coverage refinements


Original inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.

## Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-tree.json
[
  {
    "id": "ht-0lo.5",
    "title": "Integration sweep: handoff delivery redesign",
    "description": "owns: remaining cross-feature integration wiring, documentation consistency and end-to-end tests no owning leaf covers; consumes: all implementation leaves. Verify long durable inbox handoff, all native delivery paths, send-time unavailable-recipient expiry, lazy/timed invitation acceptance and independent sender warnings, retained keyed retry and stable per-event owned hooks. Fix small integration gaps inline; file blockers for larger discoveries. Run focused isolated tests, cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and nice scripts/check-default-features on the final candidate. Do not run the routine full suite while ht-zo4 restriction remains. Tests own/tag/stop all children/private sessions and never alter shared server or real user configs. Files: integration tests, embedded skills/help/docs, TRUST-POLICY.md and minimal seam wiring.\nblocked-by ht-0lo.1.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.1.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.1.3: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.3: consumes all leaves (integration sweep)\nblocked-by ht-0lo.3.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.3.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.4.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.4.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.6: consumes all leaves (integration sweep)\nblocked-by ht-0lo.7: consumes all leaves (integration sweep)",
    "acceptance_criteria": "Main handoff flows complete end to end with exact frozen bodies/routing and independent receipt/invitation deadlines. Overdue originators receive durable guarded waking attention; late confirmation clears passively. All owned native configurations and legacy retries remain safe. Final format, focused tests, all-feature clippy and default-feature gates pass; owned processes stopped. (needs: ht-0lo.1.1) (needs: ht-0lo.1.2) (needs: ht-0lo.1.3) (needs: ht-0lo.2.1) (needs: ht-0lo.2.2) (needs: ht-0lo.2.3) (needs: ht-0lo.3.1) (needs: ht-0lo.3.2) (needs: ht-0lo.4.1) (needs: ht-0lo.4.2)",
    "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T23:09:38Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T23:18:42Z",
    "metadata": {
      "depth": 1,
      "order": 5
    },
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.2.3",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:48Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.2.2",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:47Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.6",
        "type": "blocks",
        "created_at": "2026-10-10T23:18:29Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.1.3",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:43Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.1.1",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:40Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.7",
        "type": "blocks",
        "created_at": "2026-10-10T23:18:38Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.3.2",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:52Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.2.1",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:45Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.4.1",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:55Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.4.2",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:58Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.3.1",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:50Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo.1.2",
        "type": "blocks",
        "created_at": "2026-10-10T23:09:41Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.5",
        "depends_on_id": "ht-0lo",
        "type": "parent-child",
        "created_at": "2026-10-10T23:09:38Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 12,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-0lo"
  },
  {
    "id": "ht-0lo.1.3",
    "title": "Upgrade dormant historical receipt obligations",
    "description": "owns: transactional upgrade/backfill of formerly dormant historical ACK obligations, sparse/physical due projection consistency and idempotent schema marker; consumes: canonical send-time receipt projection. Preserve already running deadlines, terminal states/provenance and immutable request/result digests. Files: src/store/{schema,effective,materialization}.rs and focused migration/store tests. Exercise legacy unavailable receipts upgrade/restart/replay and overdue indexed discovery, bounded warning burst.\nblocked-by ht-0lo.1.1: consumes canonical send-time receipt projection\nExercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
    "acceptance_criteria": "Legacy dormant physical/manifest ACK obligations receive send-time deadlines while existing started/terminal/waived obligations remain unchanged. Idempotent restart and focused migration tests pass. Exercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:49:48Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T23:18:48Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.1.3",
        "depends_on_id": "ht-0lo.1.1",
        "type": "blocks",
        "created_at": "2026-10-10T22:49:49Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.1.3",
        "depends_on_id": "ht-0lo.1",
        "type": "parent-child",
        "created_at": "2026-10-10T22:49:48Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-0lo.1"
  },
  {
    "id": "ht-0lo.3.2",
    "title": "Handoff confirmation policy wiring and recovery documentation",
    "description": "owns: handoff --deadline/--require-invite-ack/--invite-deadline parsing/freeze/pass-through, modern lazy invitation plus addressed ACK-required body staging across new-tab/empty-pane/existing paths, retained child digest compatibility and source/help/skill docs; consumes: frozen handoff payload, invitation confirmation wire contract and send-time receipt semantics. Files: src/cli/{handoff,handoff_delivery,topology_handoff,mod}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and related tests/help/skill. Run isolated actual handoff per all three paths/body sources; crash/retry after stage and possible-start/unknown-create never duplicates/restages/relaunches. Older plans absent new fields replay original keyed requests, modern deadlines freeze before staging; routing exact3field fallback and no auto trust/accept/ACK.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nblocked-by ht-0lo.3.1: consumes frozen handoff payload\nblocked-by ht-0lo.2.1: consumes invitation confirmation wire contract\nblocked-by ht-0lo.1.1: consumes send-time receipt semantics\nAlso owns: separating hook/setup/trust/profile readiness diagnostics from launch admission; src/cli/launch.rs prepare_launch and src/harness/launch.rs configuration checks. Missing/untrusted/wrong-profile hooks do not block staged work or otherwise structurally safe launch. Preserve empty-shell/seat/caller/native argv and owned launch-configuration fingerprint safety, and truthful native trust refusal; never auto-approve. Test missing/stale profile hooks through guarded handoff and observe durable ACK deadline warning without registration.\nblocked-by ht-0lo.2.2: consumes canonical invitation lifecycle\nblocked-by ht-0lo.1.2: consumes overdue receipt warning lifecycle\nHook readiness is advisory: keep owned launch configuration fingerprint recheck only when that configuration is actually used; absence cannot veto and no fingerprint is fabricated. Unsupported environment-overrides/native grammar/host-kind safety refusals remain.\nRequire messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Observe durable overdue warning for an unregistered recipient (needs: ht-0lo.1.2). Require messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:20Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T23:18:53Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.3.2",
        "depends_on_id": "ht-0lo.3",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:20Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.3.2",
        "depends_on_id": "ht-0lo.1.1",
        "type": "blocks",
        "created_at": "2026-10-10T22:48:30Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.3.2",
        "depends_on_id": "ht-0lo.2.2",
        "type": "blocks",
        "created_at": "2026-10-10T22:49:52Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.3.2",
        "depends_on_id": "ht-0lo.2.1",
        "type": "blocks",
        "created_at": "2026-10-10T22:48:29Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.3.2",
        "depends_on_id": "ht-0lo.1.2",
        "type": "blocks",
        "created_at": "2026-10-10T22:59:18Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.3.2",
        "depends_on_id": "ht-0lo.3.1",
        "type": "blocks",
        "created_at": "2026-10-10T22:48:28Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 5,
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-0lo.3"
  },
  {
    "id": "ht-0lo.3.1",
    "title": "Handoff instruction source acquisition and frozen payload",
    "description": "owns: positional/file/stdin source exclusivity, bounded nonempty UTF-8 65,536-byte acquisition and exact bytes frozen in all handoff intent forms; consumes: existing compound intent/body schema. Files: src/cli/handoff.rs parser and validation, src/cli/{handoff_delivery,topology_handoff}.rs saved plans and associated parser sources/tests. Preserve topic/goal limits separately; invalid sources fail before mutations; mutate/delete source after initial staging and prove retry keeps original content. Native argv contains fixed routing/inbox only.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nAlso owns: body-bearing envelope capacity calculation and limits: bootstrap identity/progress512KiB, recovery1MiB; preserve metadata-only creation/attachment128KiB, completed2MiB/globalwire1MiB unless actual serializer worst-case fixtures prove a narrowly scoped bound adjustment necessary. Files additionally src/store/topology_handoff.rs and persistence.rs, src/cli/journal.rs, TRUST-POLICY encoded-limit clauses. Reuse src/cli/input.rs; protocol body validator must follow normal nonempty UTF-8 message bytes (NUL/whitespace are not identifier validation). Inventory all repeated bodies/nested originals; actual serializer64KiB six-byte escape public wire/canonical/local retained/retry fixtures prove support under ordinary bounded metadata. Exact serialized preflight before effects; retain historical bytes/digests and no clipping.",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:19Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:59:16Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.3.1",
        "depends_on_id": "ht-0lo.3",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:19Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-0lo.3"
  },
  {
    "id": "ht-0lo.4.2",
    "title": "Owned stable registration migration and tooling",
    "description": "owns: manifest-recorded exact ownership/install identity, repeat setup in place, recoverable UUID/shared-command migration, safe unsetup and status/trust/canary/launcher command compatibility; consumes: stable native per-event command grammar. Files: src/harness/setup.rs and claude/codex setup modules, src/cli setup/launch consumers, canary/tooling/docs/tests. Invoke isolated Claude/Codex setup-repeat/migration/interruption/unsetup with foreign identical/drifted groups and ensure no ownership theft, no trust writes, no group shifting on equivalent reinstall. Hermes/Claude mod regressions stay passing.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md\nblocked-by ht-0lo.4.1: consumes stable native per-event command grammar",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:18Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:48:31Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.4.2",
        "depends_on_id": "ht-0lo.4.1",
        "type": "blocks",
        "created_at": "2026-10-10T22:48:27Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.4.2",
        "depends_on_id": "ht-0lo.4",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:18Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-0lo.4"
  },
  {
    "id": "ht-0lo.4.1",
    "title": "Stable native event dispatch and command generation",
    "description": "owns: hook-event HARNESS EVENT parser/registered-codec dispatch and stable per-event shell command grammar; consumes: existing harness event registry and canonical routing. Files: src/cli/{hook,mod,setup}.rs, src/harness/{setup,claude,codex}.rs, registry declarations/tests. Keep legacy hook syntax; validate invalid pairs before effects; command stability includes executable/route/harness/event only. Exercise declared Claude/Codex events with fixtures through dedicated entrypoints; retain Hermes callback and bundled mod route compatibility.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:17Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:48:17Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.4.1",
        "depends_on_id": "ht-0lo.4",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:17Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-0lo.4"
  },
  {
    "id": "ht-0lo.2.3",
    "title": "Invitation CLI and presentation contract",
    "description": "owns: ordinary invite --lazy default, --require-ack/--deadline implication and conflicts, compact/JSON/inbox/help/embedded skill presentation; consumes: invitation confirmation protocol/store contract. Files: src/cli parser invitation branches/mod/output and help/skill resources, relevant CLI tests. Invoke actual CLI in isolated daemon/config for bare lazy, require-ack default duration, explicit deadline, conflicting flags, duplicate and historical keyed retry. Required/operator commands remain unchanged.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.2: consumes invitation confirmation store behavior",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:15Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:48:33Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.2.3",
        "depends_on_id": "ht-0lo.2.2",
        "type": "blocks",
        "created_at": "2026-10-10T22:48:26Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.2.3",
        "depends_on_id": "ht-0lo.2",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:15Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-0lo.2"
  },
  {
    "id": "ht-0lo.2.2",
    "title": "Invitation acceptance, due and delivery lifecycle",
    "description": "owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.1: consumes invitation mode schema/wire contract\nOriginal inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:14Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T23:18:51Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.2.2",
        "depends_on_id": "ht-0lo.2.1",
        "type": "blocks",
        "created_at": "2026-10-10T22:48:25Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.2.2",
        "depends_on_id": "ht-0lo.2",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:14Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 3,
    "comment_count": 0,
    "parent": "ht-0lo.2"
  },
  {
    "id": "ht-0lo.2.1",
    "title": "Invitation confirmation protocol and schema contract",
    "description": "owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:13Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:48:13Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.2.1",
        "depends_on_id": "ht-0lo.2",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:13Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 3,
    "comment_count": 0,
    "parent": "ht-0lo.2"
  },
  {
    "id": "ht-0lo.1.2",
    "title": "Due discovery and originating-seat warning lifecycle",
    "description": "owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nblocked-by ht-0lo.1.1: consumes send-anchored canonical receipt projection\nblocked-by ht-0lo.1.3: consumes historical dormant receipt upgrade\nOriginator open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear passive; update TRUST-POLICY A7. No invented seat for service authors. Test notification pending while drafts/working/host outage defer actual native wake.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:11Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T23:18:46Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.1.2",
        "depends_on_id": "ht-0lo.1.3",
        "type": "blocks",
        "created_at": "2026-10-10T22:49:50Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.1.2",
        "depends_on_id": "ht-0lo.1.1",
        "type": "blocks",
        "created_at": "2026-10-10T22:48:24Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-0lo.1.2",
        "depends_on_id": "ht-0lo.1",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:11Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 2,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-0lo.1"
  },
  {
    "id": "ht-0lo.1.1",
    "title": "Canonical send-time receipt projection",
    "description": "owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nReceipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
    "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:48:10Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T23:18:44Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.1.1",
        "depends_on_id": "ht-0lo.1",
        "type": "parent-child",
        "created_at": "2026-10-10T22:48:10Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 4,
    "comment_count": 0,
    "parent": "ht-0lo.1"
  },
  {
    "id": "ht-0lo.4",
    "title": "Stable hooks",
    "description": "owns: native event command surface, exact command generation, owned migration/unsetup/status/canary/launcher consistency; consumes: existing registered harness codec declaration. Files: src/cli/{hook,setup,mod}.rs, src/harness/{setup,claude,codex}.rs and setup submodules, installed manifest/canary/tooling tests and docs. Exercise Claude/Codex per-event dispatch/setup repeat/migration/unsetup in isolated config; verify Hermes and mod route remain compatible before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
    "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
    "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:45:47Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:57:33Z",
    "metadata": {
      "sp_depth": 1,
      "sp_order": 3
    },
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.4",
        "depends_on_id": "ht-0lo",
        "type": "parent-child",
        "created_at": "2026-10-10T22:45:47Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-0lo"
  },
  {
    "id": "ht-0lo.3",
    "title": "Handoff payload",
    "description": "owns: full frozen handoff payload and policy integration; consumes: send-time receipt semantics and invitation confirmation wire contract. Files: src/cli handoff paths, src/protocol/handoff.rs, src/store handoff paths and tests. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md",
    "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
    "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:45:46Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:57:32Z",
    "metadata": {
      "sp_depth": 1,
      "sp_order": 4
    },
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.3",
        "depends_on_id": "ht-0lo",
        "type": "parent-child",
        "created_at": "2026-10-10T22:45:46Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-0lo"
  },
  {
    "id": "ht-0lo.2",
    "title": "Invitation lifecycle",
    "description": "owns: invitation mode/duration/acceptance contract, protocol capability and inviter warning routing through CLI/store/query/wake; consumes: existing membership lifecycle. Files: src/protocol/{commands,results,capabilities}.rs and wire negotiation, src/cli parser invitation branches, src/store/{control,invitation_due,wake,queries,schema}.rs, relevant tests, TRUST-POLICY invitation clauses. Exercise bare lazy invite, explicit timed invite, never-register expiry, register-without-accept expiry, acceptance/rejection, duplicates, legacy retry and required memberships before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
    "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
    "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:45:45Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:57:31Z",
    "metadata": {
      "sp_depth": 1,
      "sp_order": 2
    },
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.2",
        "depends_on_id": "ht-0lo",
        "type": "parent-child",
        "created_at": "2026-10-10T22:45:45Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-0lo"
  },
  {
    "id": "ht-0lo.1",
    "title": "Receipt deadlines",
    "description": "owns: canonical send-time/upgrade receipt semantics, due materialization and originating-seat warning routing; consumes: existing preparation/projection schema. Files: src/store/{effective,materialization,receipts,messages,poke,wake,schema,queries}.rs, relevant send preparation modules and tests, TRUST-POLICY.md deadline clauses. Exercise unavailable/unbound/invited send+expiry, sparse/physical/restart, human/lazy exemptions and catch-up distinction before integration.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
    "acceptance_criteria": "Implement the corresponding spec contract and exercise all configurations named in this description with isolated owned children before terminal integration. Run focused tests and mandated per-change lint.",
    "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:45:44Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:57:30Z",
    "metadata": {
      "sp_depth": 1,
      "sp_order": 1
    },
    "labels": [
      "sp:ht-0lo"
    ],
    "dependencies": [
      {
        "issue_id": "ht-0lo.1",
        "depends_on_id": "ht-0lo",
        "type": "parent-child",
        "created_at": "2026-10-10T22:45:44Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-0lo"
  },
  {
    "id": "ht-0lo",
    "title": "Handoff delivery redesign",
    "description": "Deliver thread-carried complete handoff instructions with send-time receipt deadlines, lazy optional-confirmation invitations and stable native hook entrypoints. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md. User authorized autonomous implementation; integration merge remains coordinator decision.",
    "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T22:45:27Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T22:57:33Z",
    "labels": [
      "sp:ht-0lo"
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0
  }
]


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/coverage-ledger.md
# Coverage dispositions

- C1 acceptance gap .3.2 warning observation: verified against quoted acceptance/description and complete bulk graph; applied .3.2 <- .1.2 with reason and needs citation.
- A1 author gap originator waking warning: verified TRUST-POLICY A7 baseline and parent scope authorization; applied explicit open-waking/clear-passive frozen sender/inviter routing to .1.2/.2.2, guards unchanged.
- A2 author gap escaped-body envelope capacity: verified source constants and actual JSON worst-case6byte expansion; applied conservative bounded adjustment/calculation/worstcase serialization tests to .3.1, historical envelope bytes retained.
- A3 author clarification readiness vs fingerprint safety: verified launch.rs unconditional missing-hook fingerprint veto; applied advisory-only readiness with optional owned configuration recheck in .3.2, native structural/execution guards retained.

## Round 2 disposition

Three valid independent reviews mapped all 36 requirements and found no new issues. C1 and author refinements A1–A3 verified closed; all 11 pre-sweep edges and the sole explicit needs citation passed mechanical checks. No third coverage round. Terminal integration sweep ht-0lo.5 created after coverage with all ten leaf dependencies and matching reason lines.


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/author-refinements.md
# Authorized refinements applied with round-one disposition

- A1: Originator open warnings must be waking attention. Evidence: TRUST-POLICY A7 currently wakes only the debtor and leaves other seats passive. Parent confirmed this change is necessary for the user-requested originating-agent recovery. Apply to original sender/inviter seat under current safe native/mod guards, durably even after leaving; clear remains passive. Existing debtor/observer visibility remains. Update A7 in implementation commit. Owners ht-0lo.1.2 and ht-0lo.2.2.
- A2: Handoff body growth needs matching encoded-envelope capacity. Evidence: src/store/topology_handoff.rs MAX_IDENTITY_BYTES128KiB, persistence MAX_RECOVERY_BYTES256KiB and cli/topology_handoff.rs MAX_BOOTSTRAP_PROGRESS128KiB;64KiB valid text can encode at six JSON bytes per input byte. Parent approved narrow conservative enlargement, exact serialized preflight, and worst-escape64KiB fixtures across public wire/canonical/local retry. Proposed identity/progress512KiB, recovery1MiB; metadata-only creation/attachment128KiB and completed2MiB remain. Globalwire1MiB remains unless actual serializer fixtures prove ordinary supported handoff needs more. Owner ht-0lo.3.1; update TRUST-POLICY encoded limits in implementation commit.
- A3: Hook readiness is diagnostic, while owned configuration recheck is retained only if that configuration is actually used. Evidence: cli/launch.rs currently final config_fingerprint.ok_or MissingHook unconditionally rejects missing hooks. Absence cannot remain a veto under the approved design. Preserve unsupported environment-overrides/native grammar/host-kind safety refusals and no automatic native trust. Owner ht-0lo.3.2.

These are author findings against evidence, not independent scout findings. They were excluded from the frozen round-one packet, applied alongside verified round-one results, and independently verified closed in round two.


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/coverage-changes.md
C1: amended ht-0lo.3.2 edge/acceptance needs .1.2. A1: amended ht-0lo.1.2/.2.2 waking originator warning/frozen attribution. A2: amended ht-0lo.3.1 encoded capacity calculation, bounded limits, actual serializer fixtures and normal body validation. A3: amended ht-0lo.3.2 optional owned fingerprint/advisory missing readiness. Root plus receipt/invitation/handoff specs updated; hooks subtree unchanged. R36: Supported65,536-byte body including worst JSON escaping survives public wire/canonical/local retry under ordinary bounded metadata, with exact encoded preflight and conservative documented bounds.


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/roast-fix-ledger.md
# Design roast dispositions

Round1 confirmed R1-1 and R1-4; zero escalations/unverified/qualifiers. Verified against root Goal, scheduler/deadlines.rs5s safetytick/100cap/continuation and existing protocol capability discovery. Both design fixes applied with implementation ownership/early fixtures; no source code changed.

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

Designfixbeads ht-0lo.6, ht-0lo.7 closed for their completed design-only deliverables; terminal sweep depends both, plus all10implementation leaves. Root prose stale four-leaf count corrected mechanically. No third coverage pass. Next full design roast required because data/compatibility decisions changed.


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/coverage-round-2/summary.json
{
  "round": 2,
  "final_fixed_cap_round": true,
  "valid_reviews_per_pass": {
    "ROOT": 3,
    "receipts": 3,
    "invitations": 3,
    "handoff": 3
  },
  "hooks_local": "Skipped unchanged; included full root input.",
  "degraded": false,
  "requirements_count": 36,
  "mapped_count": 36,
  "unmapped_count": 0,
  "unmapped_union": [],
  "requirements_mapping_union": {
    "R1": [
      "ht-0lo.1.1"
    ],
    "R2": [
      "ht-0lo.1.1",
      "ht-0lo.1.3"
    ],
    "R3": [
      "ht-0lo.1.1",
      "ht-0lo.1.3"
    ],
    "R4": [
      "ht-0lo.1.1",
      "ht-0lo.1.2",
      "ht-0lo.1.3"
    ],
    "R5": [
      "ht-0lo.1.3"
    ],
    "R6": [
      "ht-0lo.1.2",
      "ht-0lo.1.3"
    ],
    "R7": [
      "ht-0lo.1.1"
    ],
    "R8": [
      "ht-0lo.1.2"
    ],
    "R9": [
      "ht-0lo.2.2",
      "ht-0lo.2.3"
    ],
    "R10": [
      "ht-0lo.2.1",
      "ht-0lo.2.3"
    ],
    "R11": [
      "ht-0lo.2.1",
      "ht-0lo.2.2"
    ],
    "R12": [
      "ht-0lo.2.2"
    ],
    "R13": [
      "ht-0lo.2.2"
    ],
    "R14": [
      "ht-0lo.2.1",
      "ht-0lo.2.2",
      "ht-0lo.2.3"
    ],
    "R15": [
      "ht-0lo.2.1",
      "ht-0lo.2.2",
      "ht-0lo.2.3"
    ],
    "R16": [
      "ht-0lo.2.2",
      "ht-0lo.2.3"
    ],
    "R17": [
      "ht-0lo.2.2",
      "ht-0lo.2.3"
    ],
    "R18": [
      "ht-0lo.2.1"
    ],
    "R19": [
      "ht-0lo.3.1"
    ],
    "R20": [
      "ht-0lo.3.1",
      "ht-0lo.3.2"
    ],
    "R21": [
      "ht-0lo.3.1",
      "ht-0lo.3.2"
    ],
    "R22": [
      "ht-0lo.3.2"
    ],
    "R23": [
      "ht-0lo.3.1",
      "ht-0lo.3.2"
    ],
    "R24": [
      "ht-0lo.3.2"
    ],
    "R25": [
      "ht-0lo.3.1",
      "ht-0lo.3.2"
    ],
    "R26": [
      "ht-0lo.3.2"
    ],
    "R27": [
      "ht-0lo.1.2",
      "ht-0lo.2.2",
      "ht-0lo.3.2"
    ],
    "R28": [
      "ht-0lo.4.1"
    ],
    "R29": [
      "ht-0lo.4.1",
      "ht-0lo.4.2"
    ],
    "R30": [
      "ht-0lo.4.2"
    ],
    "R31": [
      "ht-0lo.4.2"
    ],
    "R32": [
      "ht-0lo.4.2"
    ],
    "R33": [
      "ht-0lo.4.1",
      "ht-0lo.4.2"
    ],
    "R34": [
      "ht-0lo.1.1",
      "ht-0lo.1.2",
      "ht-0lo.2.2",
      "ht-0lo.2.3",
      "ht-0lo.3.1",
      "ht-0lo.3.2",
      "ht-0lo.4.1",
      "ht-0lo.4.2"
    ],
    "R35": [
      "ht-0lo",
      "ht-0lo.1",
      "ht-0lo.1.1",
      "ht-0lo.1.2",
      "ht-0lo.1.3",
      "ht-0lo.2",
      "ht-0lo.2.1",
      "ht-0lo.2.2",
      "ht-0lo.2.3",
      "ht-0lo.3",
      "ht-0lo.3.1",
      "ht-0lo.3.2",
      "ht-0lo.4",
      "ht-0lo.4.1",
      "ht-0lo.4.2"
    ],
    "R36": [
      "ht-0lo.3.1"
    ]
  },
  "R_new": [],
  "deduped_findings": [],
  "finding_count": 0,
  "trajectory": {
    "round1": 1,
    "round2": 0,
    "novel_count": 0,
    "novel_fraction": 0,
    "novel_fraction_note": "No round2 findings; zero novel identities, convention0%;0/0 denominator empty.",
    "assessment": "Converged1to0; no widening."
  },
  "fixes": {
    "C1": "All3reviewers confirm direct warning blocker+needs citation closes original acceptance gap.",
    "A1": "All3confirm waking-originator/open+passiveclear/frozeninviter/A7guarded lifecycle.",
    "A2": "All3confirm owned encoded envelope inventory/fixtures/preflight/normalbody semantics.",
    "A3": "All3confirm advisory readiness+conditional owned fingerprint safety."
  },
  "evidence_verification": "No findings to disposition. Frozen bulk mechanical check11edges all reason-lined; sole needs citation direct blocker; no epic blockers; originalR1-R35stable. Reviewer1edgecount12is numerical slip; individual local counts/frozen tree11.",
  "mutation_scope": "Only coverage-round-2 artifacts; no design/tree/run.md/bd mutations.",
  "next": "Fixed-cap coverage loop ends; root integration sweep created by parent after loop; no round3."
}


ARTIFACT docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/coverage-round-2/summary.md
# Coverage round 2 summary

requirements: 36 · mapped: 36 · unmapped: 0

ROOT, receipts, invitations and handoff each received three valid independent no-tool reviews. Hooks local skipped because unchanged; all five specs included in ROOT. Full literal packet/template supplied before review. Coverage not degraded.

Findings: none. Trajectory: round1 1 → round2 0. Novel identities: 0; novel fraction 0% by empty-set convention (no round2 findings). Converged; no widening.

All reviewers confirm C1, A1, A2 and A3 closed with no introduced seams, edges or unsatisfiable acceptance. Mechanical frozen-input audit:11blockingedges all reason-lined, one needs citation satisfied by direct blocker, no epic blockers, original35Ridsstable. Reviewer1stated12edges; source and its own enumerated local counts total11, a numerical audit slip.

Final fixed-cap round: no round3. Parent creates/adopts root Integration sweep after loop. No spec/tree/tracker/run-state mutation performed.

## Requirements mapping union

R1 → ht-0lo.1.1
R2 → ht-0lo.1.1, ht-0lo.1.3
R3 → ht-0lo.1.1, ht-0lo.1.3
R4 → ht-0lo.1.1, ht-0lo.1.2, ht-0lo.1.3
R5 → ht-0lo.1.3
R6 → ht-0lo.1.2, ht-0lo.1.3
R7 → ht-0lo.1.1
R8 → ht-0lo.1.2
R9 → ht-0lo.2.2, ht-0lo.2.3
R10 → ht-0lo.2.1, ht-0lo.2.3
R11 → ht-0lo.2.1, ht-0lo.2.2
R12 → ht-0lo.2.2
R13 → ht-0lo.2.2
R14 → ht-0lo.2.1, ht-0lo.2.2, ht-0lo.2.3
R15 → ht-0lo.2.1, ht-0lo.2.2, ht-0lo.2.3
R16 → ht-0lo.2.2, ht-0lo.2.3
R17 → ht-0lo.2.2, ht-0lo.2.3
R18 → ht-0lo.2.1
R19 → ht-0lo.3.1
R20 → ht-0lo.3.1, ht-0lo.3.2
R21 → ht-0lo.3.1, ht-0lo.3.2
R22 → ht-0lo.3.2
R23 → ht-0lo.3.1, ht-0lo.3.2
R24 → ht-0lo.3.2
R25 → ht-0lo.3.1, ht-0lo.3.2
R26 → ht-0lo.3.2
R27 → ht-0lo.1.2, ht-0lo.2.2, ht-0lo.3.2
R28 → ht-0lo.4.1
R29 → ht-0lo.4.1, ht-0lo.4.2
R30 → ht-0lo.4.2
R31 → ht-0lo.4.2
R32 → ht-0lo.4.2
R33 → ht-0lo.4.1, ht-0lo.4.2
R34 → ht-0lo.1.1, ht-0lo.1.2, ht-0lo.2.2, ht-0lo.2.3, ht-0lo.3.1, ht-0lo.3.2, ht-0lo.4.1, ht-0lo.4.2
R35 → ht-0lo, ht-0lo.1, ht-0lo.1.1, ht-0lo.1.2, ht-0lo.1.3, ht-0lo.2, ht-0lo.2.1, ht-0lo.2.2, ht-0lo.2.3, ht-0lo.3, ht-0lo.3.1, ht-0lo.3.2, ht-0lo.4, ht-0lo.4.1, ht-0lo.4.2
R36 → ht-0lo.3.1
