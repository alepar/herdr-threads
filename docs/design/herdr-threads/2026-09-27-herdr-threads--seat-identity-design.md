# Seat identity and Herdr reconciliation

> **Superseding user direction (2026-09-28):** Top-level acceptance/ACK responsibility may rely on demonstrated cooperation and prompting. Service-side adversarial main-agent proof is no longer a release prerequisite. See [cooperative receipt direction](cooperative-receipt-user-direction.md). Explicit model-issued ACKs, actual both-harness delegation/restart/clear/resume and SQLite evidence remain required. Conflicting older caller-enforcement clauses below are historical pending coordinated contract update.


> **Coordinated contract adopted:** [Cooperative caller amendment](cooperative-caller-contract-adoption.md) is normative for CheckIn, claimed context and top-level acceptance/ACK fences. Independent scoped review159 is CLEAN; implementation and both-harness receipt demonstrations remain open. Earlier conflicting native main-agent proof clauses below are historical.

## Goal

Preserve a pane-bound role across disposable agent occupants and observed layout changes, while preventing ambiguous host recovery, stale occupants or subagent calls from settling another seat's obligations.

Parent: [root design](2026-09-27-herdr-threads-design.md). Bead: ht-4is.4. Earlier siblings: [caller attribution](2026-09-27-herdr-threads--caller-attribution-design.md), [store](2026-09-27-herdr-threads--store-design.md).

## Adopted shared-contract detail

The [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md) is normative for the exact types, schema, algorithms and ownership described below. Its adoption is a design decision, not implemented or native-tested evidence. Existing acceptance remains required. Logical publication is authoritative; bounded physical projection cannot hide committed receipt/warning obligations. Preserve F6's unresolved dissent and the BOTH-harness gate `ht-910`; shared types, fake ports and store tests cannot satisfy that gate. Formal design review counters and original task review histories are unchanged by these edits.

## Identity layers and allocation

A durable plugin seat UUID belongs to one scoped local Herdr instance. Keep a registered host namespace in plugin state, selected by the normalized local endpoint/session location; a newly registered host receives a new namespace. A socket pathname is a locator, not proof of a process incarnation or restored pane.

The seat records current public pane address, terminal ID, host-observation incarnation and binding generation. Names/tab order/cwd are display metadata and never continuity keys. Native reference retains source/harness/kind/value. A validated execution-instance identifier from the harness recipe distinguishes a restarted CLI resuming the same conversation from an old still-finishing tool call.

Observed host targets and allocated seats are separate. Snapshots observe targets and reconcile existing seats; a previously unused observed pane alone does not create a seat. An explicit ordinary `seat resolve --pane ADDRESS` allocates an unowned target only when no unresolved continuity claim or recovery hold applies. This is an explicit same-instance local discovery/service action: it may allocate an empty shell pane before native launch without recipient caller proof. It grants no membership, receipt eligibility or authority to send/ACK/accept. Repeated resolve returns the existing seat. A verified startup flow may also allocate an unclaimed target. A retired seat is never revived by either path.

Keep two separate state dimensions:

- Continuity is `resolved`, `unresolved` or terminal `retired`.
- Occupant proof is `unregistered`, `registered(execution, generation)` or `unavailable`.

A root startup/resume/clear that replaces the occupant rotates the generation once per execution instance; repeated hook delivery is idempotent. An already-observed replacement followed by its matching startup does not rotate twice. Ordinary compaction retains its execution. A known process exit marks the occupant unavailable, not the seat retired. Registration and current availability can be revoked without deleting historical provenance. Accountable native mutations require the fresh authorization flow below; local allocation and explicit administrative operations have separate narrowly typed actors.

## Host adapter and finite calls

Use the inherited Herdr binary and endpoint context through argument arrays or the documented socket protocol. Structural terminal freshness, verified host incarnation/coherent enumeration and proven current native execution are distinct evidence states; Unknown is explicit. A nullable occupant alone proves neither an empty shell nor current native execution. Return a complete observation or explicit error; permission denial, partial parse and unsupported version cannot masquerade as an empty host. Capture installed API version/protocol, tolerate additive fields and reject missing identity-critical fields. A fresh RPC response is not sufficient evidence of the current native execution when the underlying field is cached. The adapter and caller recipe must establish each field's meaning and freshness against pinned source and native tests, including same-conversation execution replacement; otherwise return `identity_unavailable` for accountable operations.

HostPort exposes snapshot, owned cancellable lifecycle subscription, current-target read, native prompt and the narrow managed native launch boundary. Subscription events are dirty/invalidation hints only; unsupported streaming leaves periodic reconciliation degraded. safe_wake_target receives the trusted resolved SeatId explicitly. Prompt/launch outcomes distinguish observed completion/submission from possible-submission OutcomeUnknown; neither grants receipt authority. Every target is explicit; never use the focused pane. Internally share:

- `HostCallContext { call_id, connection_epoch, monotonic_deadline, cancel }`.
- `HostObservation { boot_id, connection_epoch, observation_seq, started_at, completed_at, incarnation_evidence, completeness, target_or_snapshot }`.
- Typed outcomes `Completed`, `TimedOut`, `Cancelled`, `Unsupported`, `Unavailable`, `InvalidObservation`; prompts distinguish `Submitted` from `OutcomeUnknown`. Submission is not an ACK.

Whole-call monotonic limits include connection/setup/parse: current-target read 750 ms, snapshot 2 seconds, prompt 2 seconds and subscription establishment 2 seconds. Clip each to the caller's remaining absolute budget. Queue waits consume that budget and expired jobs never dispatch. Ordinary hooks retain a 1.5-second total budget; startup ensure/check-in retains five seconds total.

Use cancellable asynchronous I/O. Timeout closes the owned request connection and discards its result by call ID/epoch. A CLI implementation terminates only its owned helper and reaps it through bounded cleanup; never kill Herdr or create a forever-blocked thread per timeout. Subscription receive is long-lived but cancellable within 250 ms; silence is not health proof. Reconnect uses bounded backoff and requests reconciliation. The service's bounded admission is specified in the daemon contract.

Obtain server-incarnation evidence from supported local peer/process identity when available; missing evidence is a reported limitation. The same live terminal ID within verified unchanged host context can bridge daemon reconnect and observed moves. Events are lossy hints, never a durable lifecycle log.

## Ordered observation and reconciliation

One coordinator owns a serialized snapshot/current-target observation lane. Allocate `(boot_id, connection_epoch, observation_seq)` before dispatch and apply or discard its response through the domain owner before beginning another observation. Never await the host while holding the domain writer or a write transaction. A timeout discards the observation and advances the connection epoch; old-epoch late completions cannot mutate identity. Daemon restart creates a new boot epoch, discards all permits and reconciles before host-dependent mutations.

Lifecycle events only mark reconciliation dirty and may conservatively invalidate pending authorization permits; their payloads cannot directly create, move, replace or retire seats. An event during an active snapshot queues another fresh observation rather than installing competing state. Coalesce dirty hints and retain five-second complete snapshots even with a healthy subscription, so silent event loss still recovers. Delayed event payloads cannot overwrite a newer snapshot. Target reads for mutation/check-in/rebind/allocation/wake preflight use this same lane; an older snapshot cannot be applied after a newly accepted target read.

Only a complete internally coherent terminal enumeration in a verified unchanged host incarnation can establish absence for retirement. The adapter must distinguish such a view from partial/multi-call assembly and verify that capability from source/native evidence. If the interface cannot establish it, absence makes continuity unresolved with explicit degraded capability, never retirement. Coordinator sequence numbers order accepted observations; they do not fabricate a host-side timestamp or coherent capture.

Apply these rules with epoch/sequence and expected-generation guards:

1. Exact live terminal match in verified host context retains the seat and updates address/labels. Fresh observations repair moves whether or not their events arrived.
2. A changed validated native/execution identity in the surviving pane retains the seat, revokes registration, rotates generation and records the normal replacement notice; old receipts retain original actors.
3. Allocate only through explicit resolve/verified startup under the allocation and recovery-hold rules. Labels/addresses alone never resurrect or transfer a role.
4. Accepted authoritative same-incarnation absence retires a known terminal. A close event triggers this fresh observation rather than directly retiring its payload target. Retirement calls StorePort begin_retirement to commit a constant-size terminal fence, frozen cutover and cleanup job, then releases the observation lane. Effective memberships/obligations and wake authority are terminal immediately; StorePort's bounded worker materializes owed warnings before per-thread retirement audits. Identity never loops over the backlog or waits for cleanup. Host timestamps do not backdate the cutover.
5. Host outage, denied access, unknown incarnation or missing identity across unverified restore preserves unresolved continuity; no retirement or receipt transfer follows an error.
6. Across restore, accept only an established continuity bridge. Fresh terminal IDs plus a reused public address, or native conversation equality alone, cannot transfer a pane role. Missing evidence follows explicit repair. Routine native restart/clear in a surviving pane remains automatic when its execution evidence passes.

Cold restore may require repair. Test and document it as repair, not automatic continuity. Older/replayed observations cannot lower generation or reinstate revoked proof.

## Caller validation and authorization point

The early positive BOTH-harness recipe gate ht-4is.2.3 remains mandatory for production caller verification and adapters. The recipe establishes synchronous root/child attribution and invocation-scoped transport; no inherited boolean/global pane credential or client-supplied verified actor is accepted. Native references must describe the actual current execution, not merely a previously cached session. Read-only same-instance discovery remains available to children and callers without native proof.

Every accountable native command, including registration/checkpoint, content/membership changes and ACK/accept, follows this flow:

1. Validate the versioned native invocation evidence, root/execution identity and expiry with bounded work outside the writer. Reject child, missing, unknown and unsupported shapes.
2. Initiate a new target read after this request arrives. Startup registration, a periodic snapshot and an unexpired invocation context cannot replace this read. Match host incarnation, explicit terminal, native root and actual execution to the claim. Reconcile a detected replacement before evaluating its predecessor; mismatch returns `stale_occupant`, missing/ambiguous/hung freshness returns `identity_unavailable`, both uncommitted.
3. Mint internal nonserializable `MutationPermit { request_id, payload_digest, boot_id, epoch, observation_seq, seat, binding_generation, root, execution, observed_at, expires_at }`, bound once to this payload. Expiry is at most 250 ms after read completion and no later than invocation/request expiry.
4. Retain the observation lane until this decision applies or discards, while submitting a short transaction to the domain writer. After queue/lock/preparation waits, at StorePort's transaction decision recheck permit expiry, boot/epoch, binding generation and known invalidation. A known change before that point rejects. No host wait occurs inside the transaction. Expired permits reject without mutation; a subsequent retry retains the operation key and obtains new evidence.

The authorization observation point is the actual fresh host/native view used in step 2. For a successful operation, the caller was current at that point and passed the internal fences at the transaction decision. Record that observation identity/time with actor provenance separately from the transaction's `decision_at`. A replacement visible before the fresh read rejects even when its event was silently lost and stored generation was previously unchanged. An event/invalidation learned after the read but before decision also rejects. Without a host expected-session guard, a replacement after the read that remains unobserved can occur before decision/SQLite COMMIT; this bounded observation gap is not an atomic native-current-at-COMMIT guarantee. A permitted operation retains the original observed caller, never attributes it to the successor. If stronger physical-commit authority is required, host support is required before claiming it.

Successful idempotency replay retains the original result and provenance; it neither replays the mutation nor attributes an old receipt to a retrying successor. Ordinary compaction remains within the proven execution; replacement invalidates old contexts.

### Store eligibility and occupant-offer fences

Every accepted transition that changes any recipient's send-time eligibility uses the shared store helper to increment the single instance send_eligibility_revision in its deciding transaction: registration/proof change, replacement/generation rotation, revocation/unavailability/exit, mapping uncertainty or repair, retirement and host boot/epoch invalidation/restoration. Do not substitute a vector requiring final send-recipient enumeration. Unchanged diagnostic observations need no bump. The store also maintains scalar membership/lifecycle/timeline/configuration fences as specified by the adopted amendment.

Warning offer state is `(seat,binding_generation,execution_id,offered_through_seq)`. Replacement resets the successor frontier atomically with the binding change, never ordinary obligations or retained wake spacing. Old-generation check-in cannot update it. Delayed warning attribution compares the original event decision sequence to the matching current occupant frontier; repeated hooks within one execution do not reset again.


## Availability and safe recovery hints

Receipt availability requires resolved continuity, fresh matching top-level proof and validated check-in. Only this transaction appends a verified immutable availability anchor. Unstarted logical receipts immediately derive their earliest eligible anchor plus frozen duration; bounded workers may index timers later without changing first-start time. Retain anchors across repeated registrations/replacements; a mutable latest timestamp is insufficient. Existing running deadlines persist through outages/replacement and unavailability creates the deduplicated root warning. A new occupant gets bounded topic/stats/pending recovery without new obligations for previously ACKed history.

A generic native hint has a separate target-safety predicate. A resolved target observed as a supported native idle/done occupant can receive the fixed inbox hint while unregistered; it does not require the registration it is intended to trigger. A shell alone, unknown native identity, unresolved/held target, unavailable/working state, blocked approval/question UI or positively known human input defers. Working agents use supported hooks. Scheduler immediately rereads target/state before prompt and observes the existing optimistic composer/session race; missing atomic host guards are not invented.

After failed startup check-in, recovered service observes the idle native target and durable pending work, then sends the ordinary coalesced marker under normal backoff. Its native turn/tool hook retries proof and check-in. Only actual validated check-in advances offered generation or starts timers; only explicit model commands accept/ACK. Lost output keeps obligations retriable. Missing/incompatible hooks remain visible degraded capability. The daemon must be running again; this adds no external supervision or native PTY/permission changes.

## Administrative authority and operator repair

Explicit operator mode means the owning local OS user, not proof of a human. For each such request, the daemon compares kernel-supplied connected-peer effective UID with daemon/state owner effective UID and constructs `OperatorActor { instance, effective_uid }`. Missing/mismatched credentials reject. Wire usernames, actor objects, environment, pane names and PID cannot establish it. An agent/child/tool running as that user may deliberately use operator mode; audit is `operator:local-user:<uid>`, never native agent provenance. Mode is per request and never automatically inherited from socket access.

There are exactly three typed administrative domain actions:

- `seat rebind SEAT --pane ADDRESS --operator`: unresolved nonretired seat to an unowned freshly observed target, recording the operator's continuity assertion, old/new mappings and decision time. Fresh target/epoch/generation/hold guards apply at the decision. Refuse live-seat collision and retired-seat revival; invalidate prior invocation contexts and require new validated top-level check-in.
- `seat resolve --pane ADDRESS --new-seat --operator`: explicit fresh-role choice for an unowned held target despite uncertainty. Record the choice without changing old unresolved seats/history. Refuse an already-owned target.
- `invite THREAD --seat SEAT --operator [--deadline SECONDS]`: the store permits an ordinary invitation only if its deciding transaction finds zero joined seats. Unavailable/unresolved joined seats still count. Existing pending invite reuses its episode/deadline. This restores the usual explicit target acceptance path; it does not join, reopen, rewrite recipients or settle old receipts.

No operator ACK, accept, send, joined-seat control or checkpoint advancement exists. Missing explicit administrative mode on repair/fresh-role override returns `operator_required`. Ordinary joined-seat invitations continue to require native authority. Target rebind/fresh allocation uses the fresh observation lane and internal decision guards without requiring a native recipient (the target may be empty). Service-side allocation/repair permits are distinct from native MutationPermit and cannot authorize receipt/content actions. Corrected P11 pairs the operator command instance/target with its non-wire fresh guard and consumes against matching instance/target, boot/epoch/generation, invalidation, expiry and single-use fences; durable ownership/recovery-hold checks remain in the deciding transaction. Preserve stable operation keys and original operator provenance after response-loss replay.

## Restore holds — F6 clarification pending fresh review

F6 remains the unresolved roast-1 escalation with its original dissent. These explicit rules are the new within-scope clarification for fresh review, not a declaration that the old panel is resolved.

After an unknown/new host incarnation with unresolved saved seats, establish a coherent recovery baseline of current targets. Hold unclaimed restored targets for repair; saved addresses only suggest candidates. Without reliable narrowing, hold every unclaimed target present at that baseline, including moved/restored address mismatches and many-to-one suggestions. Until a coherent baseline is available, unresolved continuity prevents ordinary allocation of potentially restored targets. Persist baseline/hold/allocation decisions so daemon restart cannot release a hold or let a startup claim preempt repair.

Ordinary resolve and startup reject a held target with compact inspect/rebind/explicit-fresh-role guidance. Rebind may choose an unowned held target and record the operator's assertion; explicit `--new-seat --operator` may instead consume that target's hold for fresh identity. Old unresolved seats/history remain intact. A merely observed empty pane is unallocated and remains available for rebind before anyone claims a new role. New panes authoritatively shown created after a coherent current-incarnation baseline have no restore claim and follow ordinary allocation. Uncertain creation timing retains the hold.

Never merge colliding seats or use a hold as proof of lineage. Keep all-target ambiguity, holds, suggestions and operator choices inspectable with bounded continued diagnostics. This conservative choice may delay a genuinely new role until explicit operator choice; fresh review must assess the recovery tradeoff.

## Decomposition and acceptance

The host adapter owns bounded calls and observation/incarnation capability evidence. Reconciliation owns ordered observations, lazy allocation/holds and typed StorePort transitions. CallerVerifier owns recipe validation and internal native permits. Identity facade owns check-in, local resolve and administrative routing. Shared contracts expose distinct target safety, proof, peer identity and permit types; no client actor object deserializes into authority.

Tests retain rename/reorder/move, native restart/clear/resume/compaction, socket denial, retirement and isolated restore coverage. Add connected hung snapshot/target/prompt, bounded cancellation and late-epoch discard; delayed snapshot versus new target read, duplicate/stale events and silent lost replacement; unknown/incoherent snapshot cannot retire. Verify old generation never returns.

Authorization tests cover replacement before fresh read (reject), change/invalidation between read and decision (reject), and silent change after read (document bounded original-observation provenance, no atomic claim). Expire permits during writer queue/lock/preparation, reject cached native execution evidence, child/missing/stale claims and old-boot permits. Check-in alone starts only unstarted timers and never ACKs.

Allocation/repair tests cover empty-pane prelaunch resolve without native proof; snapshots allocating nothing; all-target restore ambiguity, moved addresses, many-to-one suggestions, startup before repair, new empty rebind target, persistent holds across restart, explicit fresh-role choice and occupied-target refusal. Peer-credential tests cover actual same-user connection, missing/mismatched UID, explicit child administration versus denied child ACK, orphan-invite race and no implicit membership. Native recovery tests in both harnesses fail startup through its budget, omit initial prompt, recover service with idle native target, and capture hint → real verified hook → explicit model accept/ACK → matching SQLite provenance, without manual rescue. Never restart the shared Herdr instance.

## Design revision record

2026-09-27: Integrated roast-1 F8/F10/F12/F15 host contract and related F9 authority/time/query boundaries after the cumulative-history assessment. Adopted the explicit F6 recovery-baseline clarification for fresh review; F6 remains unresolved. These are specification/acceptance changes, not completed native capability evidence.

## Shared-contract adoption record

2026-09-27: Reconciled this canonical spec with the adopted revision-4 shared contract. Detailed normative algorithms/types and preserved revision responses are in the [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md). This is specification work before the next formal design review; no source implementation or native-support completion is asserted.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### Ordinary resolve recovery clarification

[Seat resolution recovery](seat-resolution-recovery-clarification.md) makes ordinary allocating resolve carry an operation key and use a narrow private service-allocation intent. It preserves the existing guarded allocation authority and exact original-result replay; replay never revives or reallocates a retired seat. This is required implementation work, not validation evidence.

### Guarded native start

[Guarded native start disposition](guarded-native-start-disposition.md) extends availability preflight to a supported host-side shell/prompt check-and-start operation. It preserves Unknown observations and all identity/recovery fences. A finite launch-specific 30-second ceiling is clipped to caller budget; possible-start failures remain OutcomeUnknown. Native qualification is still required.
