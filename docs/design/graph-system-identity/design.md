## Goal

Let herdr-graph publish durable notifications as a programmatic participant and maintain required system-channel invitations through herdr-threads. System authority belongs to one live Unix-domain connection; agents must explicitly accept invitations and retain sole authority over their own receipts.

Status: user-approved top-level design for the scoped addition to existing epic `ht-4is`; no implementation authorized by this document. Prepared 2026-09-28. Existing implementation worktree inspected at `546ade1`. The cooperative caller amendment remains normative. This document extends the existing design rather than restarting its review or implementation history.

## Why this extension exists

Threads already identifies native participants. What it lacks is a participant representing a background service, independent of a Herdr pane and native conversation. Graph needs that identity to author rename/state-path notices, create system channels and invite clones. A durable author ID preserves attribution across reconnects; the accepted socket connection supplies temporary authority to act as that author. Neither an author label nor a claimed role in an ordinary request grants system authority.

Graph team/seat/clone structure, collective subscriptions, naming, activation and retirement policy remain graph concerns. A threads native participant corresponds to a graph clone. This extension introduces no thread groups, graph hierarchy or hcom dependency.

## 1. Registration and authority

- One reserved durable programmatic author per threads instance, displayed distinctly from native participants and the daemon's built-in events. Initial caller is herdr-graph. This release does not support a registry of independent service accounts.
- Explicit `system register` on the existing private UDS establishes a persistent service connection. Validate protocol, expected instance and kernel peer UID using existing transport checks. Registration is a cooperative claim, not proof that the process is graph.
- Registration returns durable author ID, daemon boot ID and a connection generation for diagnostics. A server-private connection handle grants authority; copying returned IDs into another request cannot grant it. No reusable service credential.
- Only one registration is active. Competing registration is rejected with an inspectable busy result; no silent takeover. After EOF, shutdown or protocol failure, revoke the connection and release the slot. Reconnecting must explicitly register again. Daemon restart drops all live registrations while preserving durable author/history.
- Treat connection revocation and system mutation decisions as ordered operations. A owns the shared connection-authority gate contract: after acquiring the database transaction/write lock, C acquires a short service-authority guard, checks boot/generation and retains the guard through commit/rollback. B revokes under the same gate without acquiring the database lock. This fixes the lock order (database then authority, never the reverse), makes an in-progress decided transaction finish before revocation takes effect, and prevents a check/commit race. No host call or request preparation holds either guard; staging reacquires authority for final publication. Recheck current boot/connection generation at the deciding transaction. Work not yet decided after revocation must reject; a committed operation remains committed even if its response is lost. Late cleanup from an old connection cannot release its successor's slot.
- An idle service connection does not expire under the ordinary five-second request timeout. Retain bounded individual requests and frames, one in-flight service request at a time, and existing ordinary client behavior. No heartbeat scheduler or model wake is introduced.
- A hung but still connected service retains its slot. Explicit owner/operator recovery disconnects the observed boot/generation only; a stale recovery request cannot evict a successor. Record the recovery event. This uses the existing cooperative same-UID operator model, not an assertion of human presence.
- Ordinary native callers cannot submit service operations, choose the reserved author or request required invitations. Same-UID code can deliberately register when the slot is free or invoke operator recovery; document that limit and reserve these commands for programmatic clients.

## 2. Service operations

Provide a typed programmatic client over the registered connection for: ensure a service-managed thread; invite/reinvite native participants with ordinary or required membership; publish system notifications; inspect membership/status; update a managed thread topic; release a required constraint; and archive/reopen a service-managed thread. Exact CLI spelling may follow existing conventions, but these capabilities and boundaries are fixed.

Service management is durable and distinct from author membership. A system author is not a fake pane-bound seat, is not an agent receipt recipient, and does not require native check-in, prompt delivery or retirement. Native discussion and receipts remain ordinary native operations even inside a service-managed thread.

Ordinary service invitations may target any existing thread, including a populated thread, without masquerading as a joined native participant. Required invitations are allowed only on service-managed threads. New managed threads are public/discoverable. Do not silently convert an existing ordinary thread into a managed one: a requested thread-ID collision with an ordinary or differently owned thread returns incompatible ownership without mutation. A fresh thread ID may use the same topic as another thread; topic equality creates no error or collision-metadata obligation. Graph owns any additional naming policy. Thread identity and idempotency, not topic uniqueness, determine reuse.

Only the registered service may archive/reopen or change the topic of a managed thread. Native members may read, discuss, accept and ACK, but cannot archive a system channel as an indirect way to disable it. Ordinary threads retain their existing controls. Operator orphan invitations remain ordinary and cannot downgrade a required constraint.

Releasing a constraint makes membership voluntary; it does not remove the native participant or settle receipts. This supports teardown without a permanent lock. Graph's whole-seat leave still uses notifications telling other clones to leave individually. No new service operation forcibly accepts, ACKs, retires native participants or transfers graph roles.

## 3. Required invitation state machine

A required invitation is **pending until explicit acceptance by the current native agent**, using the existing cooperative caller, binding and receipt machinery. User rationale: acceptance confirms the agent is active and has taken up the invitation. It is historical evidence at that moment, not a perpetual liveness guarantee and not evidence that feature work is complete.

- Absent/left + required invite -> pending required invitation.
- Pending ordinary + required invite -> same pending invitation with required constraint, preserving original timing/history.
- Already joined voluntarily + required invite -> create a pending requirement confirmation without removing existing voluntary membership. Until explicit acceptance of that requirement, the participant can still leave; the required invitation stays pending. Do not silently treat an earlier ordinary acceptance as acceptance of a new unleaveable obligation.
- Explicit acceptance of required invitation -> joined with required constraint; audit the native actor and the requirement episode accepted.
- Joined required + voluntary leave -> typed `membership_required` error with owning service/thread and recovery guidance; no membership or receipt changes.
- Repeat required invite/accept -> existing state returned idempotently, no deadline reset or duplicate acceptance record.
- Service releases requirement -> ordinary membership if joined; cancel the pending requirement if unaccepted, preserving any independently existing ordinary invitation/membership. Record release explicitly.
- Native participant retirement ends effective membership under the existing retirement contract, including required membership. Preserve receipt/overdue history and show terminal status. Required membership cannot revive a retired native participant.

Pending required invitations stay visible through normal invitation discovery/check-in and use existing invitation deadlines/warnings. Deadline passage does not auto-accept, remove or continually recreate them. The read API distinguishes pending requirement, accepted requirement, voluntary membership, released requirement and retirement.

Membership belongs to the durable threads participant, not each successive native conversation. After a native session replacement on the same participant, the required membership persists; ordinary check-in exposes it to the new occupant. Graph must not interpret the predecessor's acceptance as proof that the new occupant is active. Requiring a new invitation on every native session is outside this extension.

## 4. Notifications and receipts

Service notifications are durable `system_notify` events with explicit programmatic author attribution, distinct from built-in timeout/receipt events. They require no message ACK, matching existing system-event semantics. Info notices are discoverable at read/check-in without an independent wake; actionable warn notices use existing bounded, coalesced native wake handling. The programmatic sender never becomes a native wake target.

Snapshot the notification audience from joined native members plus pending required invitees at publication, deduplicated. That lets a pending invitee discover actionable system notices before acceptance; public history is already readable without membership. Use existing bounded logical publication/materialization machinery rather than an unbounded recipient loop in a writer transaction. No receipt obligations are created by this audience selection. New memberships do not retroactively change the snapshot; history remains discoverable.

Native messages in these channels retain normal recipient snapshots and explicit ACK rules. Neither service registration, successful socket writes, history reads, invitation acceptance nor system notifications manufacture ACKs. Graph must separately reconcile the desired effect of a notification, such as other clones leaving an ordinary thread; publication is not proof they acted.

## 5. Durability, replay and observability

Persist service author identity, managed-thread ownership, required invitation/acceptance/release episodes and service event attribution with existing database migrations. Old databases migrate through the normal versioned path; incompatibility fails visibly without rewriting history. Keep native participant identifiers unchanged.

Mutations use caller-stable operation keys and payload checks, scoped to the durable author and instance. After response loss, reconnect/register before exact-key retry; return the original committed result/provenance. Reusing a key with different payload rejects. Connection IDs are authority fences, not durable idempotency namespaces. Registration itself is connection-local and never resurrected by replay.

Expose registration status/boot/generation, author kind, management owner and required membership state in bounded API/CLI output, history and diagnostics. Do not display a service as an active native agent. Service client retains exact pending request intent across an ambiguous outcome; no automatic mutation retry or fake success. No secret is emitted because there is no bearer credential.

## 6. Approved implementation slices

These are direct additions to `ht-4is`, not a new epic. Existing `ht-4is.12` remains the terminal integration sweep. The caller owns implementation handoff to herdr-threads/main; this design run will not execute implementation or close that epic.

| Key | Deliverable | owns: | consumes: | Files hint | New blocking deps |
|---|---|---|---|---|---|
| A | Compiled programmatic participant contract | Service request/result types, author kind, connection authority interface, required-membership states and typed errors | Existing cooperative protocol and store/transport ports | src/protocol/*, src/ports.rs | none |
| B | Persistent service registration and fencing | Registered UDS lifecycle, server-private authority construction/revocation, bounded request loop, generation-specific operator disconnect | A's authority and wire contracts | src/daemon/*, transport authority wiring | A |
| C | Service-managed threads and required invitations | Durable author/ownership/schema, invitation state machine, service notification publication and query projections | A's service actor and operation contracts; existing store publication and receipt primitives | src/store/* | A |
| D | Programmatic client and production integration (promoted subtree) | Persistent client/retry intent, dispatch wiring, bounded display and docs; extension end-to-end acceptance tests | A/B/C contracts and working implementations | src/client/*, src/service/*, src/app.rs, src/cli/*, docs, tests | A; runtime B/C edges belong only to integration leaves |

Fresh promotion review: A is a LEAF; B, C and D are PROMOTE. Nested designs and two coverage rounds are complete; see run.md. D separates its client/intent work (consumes A) from runtime integration (consumes B/C), avoiding an unnecessary whole-subtree gate.

A lands only the compilable seam and contract-level tests, not runtime behavior. B and C test their own behavior against the seam independently. D integrates the service client and validates actual transport/store behavior; it does not claim to implement graph. Existing sweep consumes every added leaf with explicit dependency reasons. Existing native harness validation remains required; retain earlier blockers and open findings.

## 7. Acceptance evidence

A: exhaustive typed requests/results distinguish native/service actors; neither a serialized actor nor returned connection generation is a credential. Document the existing one-shot client compatibility and schema transition.

B: real UDS tests cover simultaneous claims (one winner), independent instances, idle persistence, per-request timeout, malformed/oversized frames, EOF/reconnect, daemon restart, generation-targeted recovery and delayed old cleanup. Pause a request around revocation to prove predecision rejection versus durable postdecision completion. Verify ordinary clients/health still progress with a connected idle service.

C: real SQLite tests exercise every transition in section 3, including joined-voluntary upgrade, repeated invite timing, release before/after acceptance, leave races, retired recipients, managed archive/topic controls and ordinary thread behavior. System notifications have the right author/audience, bounded warning delivery and zero ACK obligations. Test migration, payload mismatch, commit-then-response-loss exact-key replay, and audience larger than one materialization quantum.

D: one real daemon/client flow registers graph, creates public system channel, invites pending native recipient, shows pending status, receives explicit native acceptance, refuses leave, publishes attributable system notice, disconnects/reconnects and recovers an ambiguous mutation without duplication. Exercise voluntary invitation to an existing populated ordinary thread and rejection of a required invite there. Verify malformed ordinary requests cannot claim service authority, stale recovery cannot evict a successor, release restores leave, and native message ACK behavior is unchanged. Run Codex and Claude explicit acceptance/leave refusal through existing native validation harnesses; evidence is model-owned acceptance plus stored actor/event records, not prompt submission or bead closure.

## 8. Approval record

Core registration and initial explicit acceptance reflect user decisions. User approved these details with the top split on 2026-09-28: requiring fresh acceptance when upgrading an already-joined voluntary member; service-only managed-channel lifecycle controls; system notifications retaining existing no-ACK semantics; explicit generation-targeted recovery; and the four-slice split above.

## Follow-on and limits

Graph's implementation, collective subscription state, event observers, clone loading, graph instance rules and automatic reconciliation are separate work. Cooperative service registration does not resist malicious same-user code. A live but hung connection needs explicit recovery. No agent is kept periodically awake by creating a service identity. Thread acceptance and ACKs are distinct from task completion and user acceptance.

## Nested designs

- [Connection lifecycle](connection-design.md)
- [Store and membership](store-design.md)
- [Client and integration](client-integration-design.md)
