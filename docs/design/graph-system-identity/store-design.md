> **Superseded (2026-10, ht-p03.2 / B4):** the adversarial verification layer described here was removed; see "Cooperative reality (2026-10)". Where this document says "native" acceptance, or mentions caller proof or verification evidence, read it as the cooperative claim: acceptance and ACKs are the top-level agent's cooperative claim, recorded as `cooperative_top_level`; see "Cooperative reality (2026-10)" in the [seat identity design](../herdr-threads/2026-09-27-herdr-threads--seat-identity-design.md) and [root design](../herdr-threads/2026-09-27-herdr-threads-design.md).

## Goal

Persist attributable programmatic notifications and service-managed required invitations without replacing native explicit acceptance or ACK semantics.

Parent: [approved amendment](design.md). Bead: ht-4is.31. Earlier sibling: [connection](connection-design.md). Consumes ht-4is.29; store tests may use its fake authority gate without waiting for transport implementation.

## Schema and actor contract

Add a versioned durable programmatic author record keyed by instance/reserved identity; it is not a native seat row. Store immutable author kind/ID with authored system events, preserving existing native and built-in event attribution. Threads gain nullable managed owner. Required membership uses independent requirement episodes: ID, thread, native participant, issuer, pending/accepted/released/retired state, invitation linkage and explicit acceptance provenance/time. Keep the existing voluntary membership and required episode independently representable so a joined voluntary member can have a pending requirement.

Enforce at most one effective requirement per thread/participant with database constraints. Audit upgrades, acceptance, release and retirement. An already pending ordinary invitation can be linked to the requirement without resetting its deadline; accepting it must acknowledge its current required semantics. Address acceptance with the expected invitation/requirement revision, so an old ordinary acceptance racing an upgrade cannot silently consent to unleaveable membership. Reject stale acceptance with the current required state for reread and explicit retry. Root API contract owns this revision field and native client exposure; never infer new acceptance from an old idempotency key.

A previously joined voluntary member receives a new requirement confirmation and its own normal invitation timing anchor. Leaving before accepting preserves the pending requirement. Required acceptance joins if needed; release cancels only the requirement, retaining any independent ordinary invitation/membership. Required members cannot voluntarily leave. Native retirement terminates effective membership and requirement, using existing bounded retirement processing and warning settlement. Session replacement alone does not create a new participant or reset requirement acceptance.

Schema migration preserves existing identities/history, initializes ordinary threads with no managed owner, and uses existing migration/version/corruption handling. Query projections, membership revision checks, pending-invitation checks and recipient snapshots must read effective required state. Large retirement or publication workloads use the existing resumable bounded mechanisms.

## Control operations

Ensure managed thread uses a stable requested thread ID/operation key. Existing same-ID/same-owner returns its recorded result; incompatible ownership rejects. Topic names are display/search data, never uniqueness authority. Service can ordinary-invite into any ordinary populated or orphan thread; required invitations only target its managed threads. Native operators cannot downgrade required state. Only owner service may set topic/archive/reopen managed threads. Native discussion, reads and explicit receipts retain existing behavior; archiving does not erase pending requirements or their warnings. Releasing requirements does not force native leave or settle native message obligations.

All service mutations validate the active actor with the shared DB-then-authority guard through commit. Operation records are scoped to instance/durable author and stable key/payload; after reconnect exact-key replay returns the committed original actor/result without a new event. Lookup/replay still requires current connection authority. Requests never ACK or accept on behalf of native agents.

## Notifications and views

Programmatic system notifications support existing info/warn severity, authored by service and distinguishable from daemon-generated events. Use joined natives plus pending required invitees as the deciding-time audience; deduplicate and freeze the audience with existing staged logical publication. It produces notification attention, never message receipt rows. Retired recipients are excluded by effective-state checks. Archive permits these system events just as it permits built-in events. Info does not independently wake; warn uses existing bounded/coalesced safe native delivery. The author is not a wake or ACK recipient. Service cannot forge a built-in receipt/deadline event kind.

Extend bounded directory/participant/history/pending/status output with owner/author kind/required episode and pending versus accepted status. Pending requirement confirmations must be returned even for currently joined voluntary members. Preserve exact pagination/output bounds and durable native operation journals.

## Deliverables

C1 owns migrated schema, author/owner/requirement structures and compiled store helper boundaries; consumes ht-4is.29. Files: store schema/effective/shared helpers. Tests migrate old data, enforce constraints and preserve original attribution. Land only substrate unblocking C2/C3, not control implementations.

C2 owns managed-thread controls, required invite/accept/release/leave/retirement transitions, decision replay and required status queries; consumes C1. Files: store control/retirement/invitation queries. Exercise every root state transition, stale ordinary accept versus upgrade, repeated acceptance and preserved timing, release with independent voluntary state, archive/topic permissions, retired targets and decision-time races.

C3 owns programmatic event publication, effective audience snapshots, author display/query and notification attention; consumes C1. Files: store event/publication/read projections. Exercise info/warn, joined+pending-required dedupe, retired exclusion, zero receipt rows, archive behavior, multi-quantum audience, migration attribution, exact-key response-loss replay and mixed native/system history. A fake authority gate and seeded requirement substrate suffice; no dependency on C2 control handlers.
