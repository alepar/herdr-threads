# Task9 narrow preallocation seam — exact proposal for review

Source: worker task-ht-qhz.4.2, BASE511a8446a15a82baa9f40bc351a5b928c2f2ba4f / TREE98cf1fd991ccfc0376f921eb032478374f1402a8. Same ht-qhz run. This proposal is unimplemented; original actor prerequisite remains its isolated six-CLI owner. Public execution stays Unsupported until19.

## Concrete defect and required boundary

Current OrdinaryIdentity::resolve in src/identity/repair.rs219 first replays ResolveSeat, otherwise obtains ordinary HostObservation and invokes StorePort::resolve_seat. HostObservation proves pane/terminal/incarnation but has no workspace/tab. NativeCli::observe_target in src/host/native.rs1516 discards NativePane's workspace/tab when consuming it into observation. Attachment::attach_pending validates fresh workspace/tab AFTER ordinary resolution may already allocate. A pane moved to another tab with otherwise identical structural identity can therefore allocate before the later attachment refusal. Creation evidence, labels, frozen parent tab, snapshots or a second independently observed response cannot repair this ordering.

## Exact additive wire and capability

Add protocol::handoff::ResolveBootstrapSeat { identity: BootstrapIdentity, expected_attempt: BootstrapAttempt, operation: OperationId }. The target is derived only from canonical recorded creation; no caller target/tab guess. Validate full identity and operation == identity.payload.resolve_key. Command::ResolveBootstrapSeat(Box<...>) returns unchanged CommandResult::SeatResolved. This command is distinct from old ResolveSeat; old request/digest/operation scope/results/golden bytes remain unchanged. No new SQL, migration, identity payload field, operation child key or schema/host transport change.

Add named inert constant protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1 = "handoff.bootstrap_guarded_resolution_v1". Do NOT add it to ADVERTISED in this leaf. The service command arm is explicitly Unsupported alongside other inert bootstrap commands. Only19, after the actual handler and every required producer/permit/A2/namespace guard are enabled, adds this capability to ADVERTISED and the existing every_advertised_capability_has_a_handler probe. Existing Command::Capabilities/CommandResult::Capabilities(CapabilityList) is the only discovery mechanism. The internal coordinator calls Capabilities before Begin/reserve/native/any mutation; absent name, Unsupported/older peer, wrong result or failed discovery returns Unsupported and zero mutation/native activity. Current fixture clients explicitly advertise the capability as their implemented internal guard contract; no assertion that the production build serves it.

## Task9 store-only implementation and minimum named footprint

Task9 amendment files beyond original owned CLI paths:
- src/protocol/handoff.rs: additive neutral request and validation (no original BootstrapIdentity changes).
- src/protocol/commands.rs: additive command, bounded validation, exhaustive read-only classifications as needed; old mutation/permit routes remain unchanged.
- src/protocol/capabilities.rs: constant only, no advertisement.
- src/service/dispatch.rs: inert Unsupported arm only; no activation/permit bypass.
- src/store/seats.rs: narrow bootstrap resolver reusing the unchanged ordinary allocation/result/ledger recipe.
- src/store/topology_handoff/attachment.rs: shared crate-visible validation of exact canonical creation/current attempt/fresh same-response BootstrapAttachmentGuard before any allocation; existing attachment validation consumes it as appropriate.
- tests/cli/topology_handoff.rs: compiled store-backed source tests and coordinator tests; no new target or process-global state.

Proposed store function seats::resolve_bootstrap_seat(context: &StoreContext, conn: &mut Connection, canonical: &HandoffNamespace, request: &ResolveBootstrapSeat, guard: &BootstrapAttachmentGuard, budget: &CallBudget) -> Result<SeatId, ApiError>. This is an internal producer-independent foundation, not the public handler. Canonical is explicitly selected by the real daemon in19, never inferred from request or journal. Guard is the existing sealed owned same-response type; no substitute observation/guard/result abstraction and no public constructor.

Use existing schema::execute_idempotent_transaction_presented immediate transaction. The new resolver uses EXACT old ordinary digest canonical_digest(("resolve_seat", canonical.instance, canonical_creation.root_pane)), scope service-allocation:<instance> and existing resolve_key. Extract the existing ordinary apply body into a narrow shared helper if necessary, preserving all old resolve branches/replay-before-live-check semantics. The bootstrap validate callback checks the whole canonical identity via current, current attempt equality, state Created or Attached only, exact canonical creation, original validate_live A2/membership/archive/namespace, resolve_key, same fresh guard workspace/tab/pane/terminal/incarnation and ordinary hold/admission checks BEFORE the unchanged allocation helper. The bootstrap present callback performs the same checks for exact operation replay and verifies the resolved current owner matches the saved SeatResolved result. This callback also runs on fresh writes before commit; any late refusal rolls back allocation and operation insertion together. Old ordinary presentation remains identity so old historical replay is unchanged. Guard failure must never touch seats/allocation_decisions/operations. Completed/Cancelled/stale attempt refuse this live route; completed parent presentation remains separate terminal consumer19/.4.3.

Budget/cancellation: check existing snapshot_budget at entry and all validate/apply/present boundaries. Existing schema helper holds the same deciding transaction; never perform host I/O while holding it. Guard admission/lifecycle revisions reject observations invalidated while queued. Fresh canonical status and claim guard run in the deciding transaction, including replay. No local journal authority and no PID heuristics.

## Exact19 producer/handler integration (not Task9 edits)

Indispensable future named files: src/ports.rs, src/host/native.rs, src/identity/repair.rs, src/store/mod.rs, src/service/dispatch.rs. Actual19 accountable/permit plumbing may additionally require src/protocol/authority.rs, src/protocol/commands.rs and existing service/identity accountable context files;19 must declare its own exact permit footprint before edits. This proposal does not supply a pretend permit or activate a route.

HostPort gets a distinct additive observe_bootstrap_target(target,context)->BootstrapPaneObservation boundary (default Unsupported until real NativeCli implementation, consistent with optional guards); NativeCli retains NativePane.workspace/tab from the SAME pane_witnessed response and constructs observation and scope once after existing context/epoch/boot/budget checks. No response parser/schema expansion or second pane read. OrdinaryIdentity obtains SAME current observation and existing admission/publishes through its bounded read lane/fair writer. It derives BootstrapAttachmentGuard from that same owned response plus ResolveSeat{target:canonical_creation.root_pane,operation:resolve_key}. Its new bootstrap resolution path must never call old ReplayOnly without this fresh scope: it gets current canonical creation then performs one scoped read and invokes the new store resolver. StorePort needs the additive bootstrap resolver boundary (default Unsupported until implementation; exact implementer requirements to be resolved in19), SqliteStore passes its actual selected namespace and bounded writer to the store function. Runtime service validates elected owner/expected daemon boot/current original actor/accountable permit and canonical actual InstancePaths namespace before this handler; no caller-supplied path equality shortcut. Attach must freshly obtain same qualified scope again immediately before its deciding transaction. No scope proof from frozen CreatedTab or prior resolver proof bypasses final attachment currentness.

The new internal coordinator calls ResolveBootstrapSeat after durable canonical creation, consumes SeatResolved, derives exact frozen downstream HandoffPlan/digest with canonical recipient, then calls existing AttachBootstrapHandoff. Until reviewed exact neutral/store source lands it conservatively stops at canonical Created with Unsupported; no old ResolveSeat call.

## Required compiled source-bound tests

Use real private SQLite schema27, actual begin/reserve/record_created, actual same-response guard, actual ordinary allocation writer and retained operation ledger. No model/shared host/config.

1. Moved current tab with same pane/terminal/incarnation refuses before allocation: seat count, allocation_decisions count and operations for resolve_key all ZERO; canonical creation remains unchanged. This must produce compiled RED against a deliberately scaffolded bootstrap resolver delegating existing ordinary resolver, then GREEN guarded implementation; temporary production weakening is not evidence.
2. Same-response exact workspace/tab/pane/terminal/incarnation positive allocates once, stores old ordinary digest, attaches exact recipient. Exact replay returns same recipient with zero second allocation and freshly verifies current owner.
3. Wrong workspace/tab/terminal/incarnation, missing pane/unqualified guard, restore hold, occupied conflicting owner, unresolved/retired current owner, stale admission/lifecycle, wrong resolve key/current attempt/original claim/namespace all refuse before allocation. Changed binding/membership/archive between preparation and deciding transaction refuses.
4. Actual canonical attempt/admin cancellation change between prepare and transaction refuses; already Completed/Cancelled never creates/resolves. Concurrent distinct local journals share canonical DB and one allocation/key; same-reference normal operation lock excludes known active process.
5. Late allocation/operation insert/presentation failure rolls back owned transaction even if outer caller handles refusal; replay digest mismatch preserves old operation. Budget/cancellation expired at entry or final validation/presentation produces no allocation/ledger write.
6. Capability absent/old/wrong discovery/Unsupported refuses coordinator BEFORE Begin/reserve/native/progress mutation. Positive fixture advertises only its actual wired implementation; production ADVERTISED remains unchanged and service wire Unsupported test remains passing.

Native phase compiled RED/GREEN continues separately on initial fixture scaffold. Full exact attachment success and consumer composition wait for approved neutral/store seam. Process cases use owned test_support spawn with child-local HOME/CODEX_HOME/CLAUDE_CONFIG_DIR and tagged UUID, stop/reap; no whole suite, shared server action, models, transport/schema/native trial expansion.

## Ownership / accepted limits

Requires parent exact design review BEFORE any amended protocol/store edits. Parent owns counters,19 activation, independent implementation review, exact actor successor composition and final approval. Task9 owns only the original three CLI paths plus reviewed neutral/store/inert footprint above; no speculative native/authority API implementation. Fresh read plus deciding transaction is bounded cooperative evidence, not a distributed host/daemon transaction. Product never closes topology/adopts labels/merges/moves seats or grants permission from a local file.


# Task9 preallocation design fix1 — bounded correction only

Read and accepted both findings verbatim from task-9-preallocation-design-review.md. Frozen initial input SHA256 d7ecdcde3a61857a8bf98653a6feacff0a120bcb08681d3f12eed547c97ef978 and its review remain untouched. This is a supplement to that complete proposal, not source approval or activation. Same BASE511a8446/TREE98cf1fd9, same ht-qhz run; no amended protocol/store source edit has occurred.

## Important: exact phase-aware lifecycle predicate

The bootstrap path must NOT depend on existing validate_bootstrap_creation's owner-is-none lifecycle condition. In the new path, check lifecycle explicitly for EVERY fresh-entry and retained-operation presentation, irrespective of whether the target currently has an owner. Preserve the old ordinary resolver and its historical replay semantics unchanged.

Let A be checked_host_number(guard.ordinary().admission().lifecycle_revision). Publication of the SAME current-target observation performs its known lifecycle transition, so the deciding transaction must initially see exactly L=A+1. Compute the checked addition once; integer overflow is SequenceExhausted. This is additional to exact admission sequence/invalidation/active snapshot/structural evidence/ordinary restore/ownership guards. Having an existing owner never waives L equality.

Phases and exact predicates:

| Phase | Expected lifecycle | Other required state |
|---|---|---|
| Fresh deciding validate, no current owner | A+1 | Exact active Created/Attached parent/current attempt/A2/namespace/evidence and same-response scope; ordinary allocation guards permit a new pane. |
| Fresh deciding validate, existing current owner | A+1 | Same parent/A2/scope and ordinary current-owner checks; no allocation is permitted or required. |
| Retained operation replay presentation | A+1 | Fresh SAME-response guard, exact parent/current attempt/A2/scope; replayed SeatResolved must equal current resolved owner. The saved operation is never mutated. |
| Fresh-result presentation with existing owner, this transaction allocated nothing | A+1 | Same exact guards; result equals current owner. |
| Fresh-result presentation after THIS transaction inserted a new ordinary seat | A+2 | Same exact guards; result equals that exact newly allocated current owner. Only this known allocation's lifecycle increment is accounted for. |

Implementation records a transaction-local Cell<bool> own_allocation=false, captured only by this call's schema::execute_idempotent_transaction_presented callbacks. The apply callback, after it sees no owner and insert_ordinary_allocation returns successful SeatResolved, sets own_allocation=true. The existing insertion in src/store/seats.rs2443 writes seat/allocation_decisions, eligibility transition, exactly one bump_lifecycle_revision (2460), and directory filter revision. It performs no other lifecycle bump. The presentation callback requires A+1+u64::from(own_allocation), checked for overflow, and rechecks canonical A2/namespace/current attempt/creation/current owner plus all same-response guard evidence. Replay skips apply, so own_allocation remains false: exact L=A+1 is mandatory. No counter guessed from current owner, saved seat existence, labels, PID or journal.

The validate callback compares L=A+1 BEFORE any allocation/operation INSERT. The existing ordinary allocation guards run as well, rather than using a post-allocation equality to accidentally veto the transaction's own positive allocation. The final presentation uses current_resolution_owner plus the explicit phase-aware lifecycle check; it must not call an owner-is-none preallocation check after creating a seat. A changed lifecycle invalidates replay and existing-owner fresh resolution even when all other target/proof fields remain identical.

The immediate deciding transaction serializes the lifecycle read, parent/A2 validation, owner lookup, known allocation transition, ledger insert and final presentation. No competing writer can insert an unrelated lifecycle change inside it. A source test uses a connection-local TEMP trigger or bounded clock to make an additional unexpected bump/final refusal during the own allocation path; A+3 refuses and the whole seat/allocation/operation transition rolls back. Failure before own_allocation is set also rolls back. No schema helper, old ordinary validation, namespace key or payload change is needed.

## Paired lifecycle source tests added to required packet

1. New target, SAME freshly published scope, L=A+1: fresh allocation succeeds, L=A+2 final presentation succeeds, one allocation and old exact ordinary digest are retained.
2. Existing resolved current owner, SAME freshly published scope, L=A+1: fresh-key resolution succeeds, no allocation or lifecycle increment; final presentation at A+1 succeeds.
3. Existing owner and otherwise identical proof/admission, lifecycle-only change AFTER publication BEFORE fresh deciding call (L=A+2): refusal before any new resolve-key operation, no allocation, old owner/records unchanged.
4. First succeed, then publish a fresh scope and make ONLY lifecycle bump after publication before exact retained-operation replay: refusal, zero new allocation, previous immutable operation digest/result bytes unchanged.
5. New allocation path unexpected extra bump from private transaction-local TEMP trigger/final presentation refusal: original parent/creation remains, seat/allocation_decisions/resolve-key operation absent after rollback. No test changes shared state or old ordinary semantics.

These pair with the already required moved-tab ZERO-allocation RED/GREEN, exact same-response positive, stale attempt/terminal/incarnation/original claim/namespace/admin/current owner/restore/admission/budget controls. First compile/observe real behavioral RED through scaffolds; no manufactured source weakening.

## Minor: complete declared footprint

Add src/store/topology_handoff.rs ONLY for a narrow pub(crate) re-export of the shared bootstrap-resolution validator defined in its existing private attachment module. Keep `mod attachment` private. All other initial named neutral/store/inert/CLI and future19 production ownership remains unchanged. No new source directory/file/schema/migration/transport/adapter dependency.

## Unchanged activation and capability constraints

Initial proposal's named capability constant remains inert/unadvertised in Task9; discovery must precede every coordinator mutation. Public daemon command Unsupported until actual19 guarded handler/producer/permit/namespace integration and handler probe. Actual six-CLI original-actor successor remains separate and unmodified. Parent owns fresh design rereview, amended footprint permission, implementation/consumer composition review and counters. No permission is inferred from this fix document.


## Review disposition

Initial scoped review found lifecycle phase ambiguity and missing module-root re-export. Fresh FIX1 scoped review addresses both with no new Critical or Important finding. Main approves the narrow boundary; parent admits exact neutral/store/inert footprint. This is design approval only. Compiled moved-tab zero-allocation and positive same-response, stale/replay/race/rollback/budget controls and fresh independent code/consumer composition review are required before activation. Original root run/design roast2/counters remain unchanged.
