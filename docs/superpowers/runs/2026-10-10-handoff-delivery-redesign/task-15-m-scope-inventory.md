# Task15 selected M — proposed exact additional scope (no implementation authorization)

2026-10-11. Read-only source investigation against D branch `task-ht-0lo.8.4`, base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`, restored own `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.8.4`. Existing 33 dirty paths remain unchanged. No Cargo lease or target here. This inventory requests coordinator admission; it does not authorize algorithm code. Contract: final narrow interface/budget selection and continuity append in `task-15-bounded-design-dispositions.md`, including the parent's 217-source-candidate refinement.

## Extraction choice

Use NEW `src/store/invitation_content.rs` for canonical context publication, bounded recipient progress, item counters, sizing/revisit/cleanup, and transactional invitation outbox allocation/revalidation/confirmation. Keep its SQL/invariants together for focused review; do not add unrelated store helpers or generic arbitrary-query/transaction closures to StorePort. Existing producers call a narrow transaction-local context-change helper. StorePort receives typed, binding-scoped presentation operations only; SqliteStore implements these under its existing writer/permit discipline. Canonical claim/admission checks remain service-owned and are revalidated by the transactional operation; Watch additionally holds its exact per-seat live-owner permit.

Use NEW `src/service/delivery_presentation.rs` for V2 orchestration, unchanged Hook admission reuse, exact Watch admission/lease, five-way output arbitration and invitation-only outbox handling. Legacy `delivery_pull.rs` retains historical V1 behavior and ordinary-message/notice semantics. Reuse its narrowly extracted eligibility/notice helpers rather than duplicate canonical warning policy.

Use NEW `src/delivery/invitation.rs` for pure harness-neutral invitation render/key/final-byte sizing. This is deliberately separate from SQL and emission completion. New V2 wire types belong in EXISTING `src/protocol/delivery.rs`; no extra protocol module/export is needed.

## Additional production files requested (outside the existing 33 dirty paths)

| Exact path | Narrow change |
| --- | --- |
| `TRUST-POLICY.md` | Accepted latest-after-quiescence limit, cooperative emit+flush evidence/A7, scoped presentation state, bounded cleanup debt and replay limits; no new caller provenance or receipt ACK. |
| `migrations/0040_invitation_presentation.sql` (new; prospective number only) | Latest-context/epoch, current seat/consumer/item/outbox state and exact indexes/CHECKs. Verify composed tip remains39 before assigning40; never edit1..39. |
| `src/store/schema.rs` | Register/validate new additive migration, exact schema/index constraints and canonical context helper integration. |
| `src/store/invitation_content.rs` (new) | Extracted canonical bounded algorithm described above. |
| `src/store/mod.rs` | Module wiring and narrow typed StorePort implementation using existing canonical transaction/permit path. |
| `src/store/control.rs` | Ordinary SetTopic and effective requirement/lifecycle transitions publish coherent context/epoch in their existing transactions, with checked update/sequence arithmetic; no due-driver changes. |
| `src/store/service_controls.rs` | Service SetTopic, attach/release/replacement effective requirements publish same helper; replay/no-op publishes nothing. |
| `src/protocol/delivery.rs` | Deny-unknown V2 pull/confirmation, fixed consumer, bounded tokens/prefix, optional token-bearing invitation pages; leave V1 exact shape unchanged. |
| `src/protocol/commands.rs` | New commands, claim extraction, validation and typed mutation routing; serialize overlap with B. |
| `src/protocol/capabilities.rs` | Honest `delivery.presentation_v2` advertisement only with handlers/schema active. |
| `src/service/delivery_presentation.rs` (new) | Extracted V2 service, admission and five-way output arbitration. |
| `src/service/mod.rs` | New service module export. |
| `src/service/dispatch.rs` | Typed V2 routing; preserve existing HookPull/ModWatch admission; accepted live Watch registration result. |
| `src/delivery/invitation.rs` (new) | Canonical context key, command/goal render and final framing/byte sizing without unconditional invitation8KiB clipping. |
| `src/delivery/mod.rs` | New neutral render module export. |
| `src/harness/adapter.rs` | Add neutral HookStep result/completion contract with inert/default behavior; no core harness branch. |
| `src/harness/agy/delivery.rs` | Capability-first V2 selection, historical V1 fallback and post-emit completion; remove V2 pre-output seen/token advancement. |
| `src/cli/hook.rs` | Carry neutral presentation completion in HookOutcome; commit only after successful complete stdout write+flush. Preserve existing attention/lazy completion ordering. |
| `src/daemon/control.rs` | Pass accepted Watch lease result through LocalService delegation. |
| `src/daemon/transport/watch_connection.rs` | Emit optional scoped lease in Accepted only for active V2 capability; registration/replacement cleanup keeps exact owner identity. |

No new Cargo target or dependency is requested. No `src/protocol/pagination.rs` change is planned: OrdinaryOnly is an INTERNAL query mode, not a new public cursor field or V1 wire mutation. No `src/protocol/output.rs`/`wire.rs`/`protocol/mod.rs` change is currently required: new V2 types live in the existing module and result encoding remains existing serialization. Any discovered exhaustive match or copied package/helper requirement outside this inventory must be identified concretely before edit. No changes proposed to retention, H/D/I, receipts, generic wake policy, ordinary20-candidate rewriting, or released migrations.

## Existing dirty production paths with additional M work

- `src/ports.rs`: narrowly typed presentation StorePort operations, accepted Watch lease/owner permit contract, internal ordinary-only source request. Defaults refuse unsupported capability; no generic store bypass.
- `src/protocol/results.rs`: V2 result variants and canonical invitation content key metadata; B overlap requires composed-blob review.
- `src/protocol/watch.rs`: optional32hex Accepted lease (default/skipNone), additive exact decimal context metadata; frozen message frames unchanged.
- `src/store/queries.rs`: one internal OrdinaryOnly query per quantum, exclude invitation capture AND traversal entirely; retain finite ordinary cursor/partial assembly across calls, physically visited ordinary count<=100.
- `src/store/attention.rs`, `src/store/seats.rs`: replace provisional newest1001 signature with bounded canonical recipient maintenance/change observation. No global churn, no wake fingerprint mutation.
- `src/service/mod_channels.rs`: exact Live lease and per-seat owner permit; no grace authorization; replacement fences old owner and preserves fixed Watch outbox; bounded maintenance continuation without unrelated Attention or presentation consumption.
- `src/service/delivery_pull.rs`: reuse narrow canonical admission/ordinary/notice helpers while preserving V1; canonical origin actionability independent of new outbox.
- `src/cli/watch.rs`: capability-first route, accepted lease, bounded V2 continuation even at unchanged attention, stdout line+flush then confirm; historical fallback unchanged.
- `src/delivery/assemble.rs`, `src/delivery/ledger.rs`, `src/delivery/page.rs`: adapt capable-path invitation exclusion/context identity without changing ordinary-message8KiB or legacy message ledgers. V1 remains historical, not falsely saturation-safe.
- `integrations/claude/mod/hooks/register.js`: additive canonical epoch/context identity and completeness dedupe; no downstream confirmation/model-uptake promise, no lazy idle submit or message ACK substitution.

## Exact additional test files requested and their existing registered hosts

| Test path | Existing host / purpose |
| --- | --- |
| `tests/protocol/delivery.rs` | `--lib protocol::delivery::tests`: V2 bounded validation/serde, malformed token/consumer, V1 exactbytes. |
| `tests/protocol/capabilities.rs` | `--lib protocol::capabilities::tests`: advertised capability and old-peer behavior. |
| `tests/protocol/watch.rs` | `--lib protocol::watch::tests`: Accepted old omitted exactbytes, old nested-extra decoder compatibility, lease and exact context key metadata. |
| `tests/store/invitation_content.rs` (new included unit module) | NEW `#[cfg(test)] #[path]` in `src/store/invitation_content.rs`, existing lib target: actual SQL plans/counters, transactions, finite progress/outbox/restart/cleanup and producer source assertions. No new process-global state/target. |
| `tests/store/control.rs` | Existing `--lib store::control::tests`: actual ordinary and service dispatch SetTopic/requirement attach/release/replacement/no-op replay, coherent epoch/source, rollback/exhaustion. `service_controls.rs` has no separate test host. |
| `tests/store/facade.rs` | Existing `--lib store::tests`: migration application/reopen/exact old schema rejection and canonical StorePort operation routing. `schema.rs` has no standalone included schema test module. |
| `tests/store/attention.rs` | Existing `--lib store::attention::tests`: bounded passive maintenance, unrelated decisions no output/wake/cache flood. |
| `tests/service/delivery_presentation.rs` (new included unit module) | NEW `#[cfg(test)] #[path]` in new service module, existing lib target: actual service admission, output fairness, continuation and canonical warning independence. |
| `tests/daemon/watch_connection.rs` | Existing `--lib daemon::transport::watch_connection::tests`: actual Accepted lease exposure, replacement/confirmation race fencing, disconnect/grace refusal. |
| `tests/daemon/control.rs` | Existing `--lib daemon::control::tests`: changed registration delegation contract, no claim-only bypass. |
| `tests/cli/hook.rs` | Existing `--lib cli::hook::tests`: neutral completion after full stdout flush, failed output no confirm, existing HookOutcome constructors. |
| `tests/cli/hook_neutral.rs` | Existing `--lib cli::hook::neutral_tests`: neutral HookStep completion/default behavior, no harness branch. |
| `tests/harness/agy_delivery.rs` | Existing `--lib harness::agy::delivery::tests`: actual capability fallback, pre-output state unchanged, emit completion and retry/page replay. |

Existing dirty test hosts reused/amended: `tests/service/delivery_pull.rs` (real1002/1025 regressions migrate to capable route; historical limitation remains honestly explicit), `tests/service/mod_channels.rs`, `tests/store/queries.rs`, `tests/cli/watch.rs`, `tests/delivery/extraction.rs`, `tests/protocol/lazy_inbox_v2.rs` (registered combined module), `tests/hook_entrypoint/agy.rs` (registered `hook_entrypoint`, isolated real daemon/stdout), `integrations/claude/mod/tests/delivery.test.ts`. Existing process-global hosts retain `required-features = ["test-support"]`; new unit modules do not require a Cargo manifest change. Actual fixture requirements, not blanket test-assertion updates, determine edits. No new tests run or passing claims in this inventory.

## Numeric proof/test plan before production

1. Meaningful RED in actual new command/admission/consumer path first. Existing1002 oldest-topic and1025 discovery REDs remain held; convert/prove V2 actual route rather than claim legacy fix from predicates.
2. Fresh quantum: invitation100 (40initial/40context/20revisit, cleanup<=10 inside20); ordinary<=100 ONE OrdinaryOnly query with preserved cursor/partial; notices<=17; total source visits<=217. Separately count notice settlement<=16 indexed validations and explicit fixed end/binding/token probes. Context rows PLUS expansions count before filtering. No hidden old invitation traversal or4096-page loop.
3. Replay-only<=32 descriptor revalidations and NO allocation; prove obsolete-empty token retirement cannot retire replacement. Bound32blocks/65536 FINAL framed bytes, including outer Attention/header/indent/escaping/continuation; descriptor/payload/schema arithmetic and plans match exact indexes, no temp sorts.
4. Five output classes persisted first-priority rotation; active class standalone allowance within5 successful fresh calls. Include sustained notices/message pressure and invitation near64KiB; allocation failure leaves rotation unchanged, withheld ordinary candidate retained boundedly.
5. Actual topic and same-ID requirement None→required→None, unrelated-recipient edits, interrupted expansion/highwater, finite1025+/removal/rebind, size-ineligible saturation, partial→later-full samekey, changed render shape/capacity, cleanup debt/restart. Logical perkey/binding at most1partial+1complete per fixed consumer, <=2complete total; physical replay explicitly separate.
6. Hook NoHookPull and canonical binding negatives; Watch accepted lease exact Live-only, replaced caller cannot allocate/confirm; per-seat serialization race proves new owner's fixed outbox reuse without old authority. No global registry lock held across DB/socket I/O.
7. Actual successful stdout flush→confirmation; broken pipe/failed flush/crash before emit/lost daemon reply/restart/token loss remain recoverable. No acceptance/messageACK forged. AGY no V2 pre-emission local seen/token save. Watch yielded continuation does not wait for unrelated Attention or wake idle Lazy recipients.
8. Historical V1 request/reply/Accepted omitted exactbytes, old nested-extra Watch decoder, capability probe BEFORE any V2 progress/output/request. Origin open warnings remain canonical/native actionable after full offer/sameversionUnchanged; final passive clear/no warning-only wake.

After corresponding GREEN: scoped leaks, fmt, harness-neutral, all-target/all-feature clippy and required default-feature implications under authorized lease; no routine full-suite or profile tuning. Review exact new schema, source/consumer interface and complete test-change diff before commit.
