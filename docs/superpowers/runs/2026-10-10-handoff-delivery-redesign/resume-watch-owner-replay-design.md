# Watch owner replay correction

Status: scoped revision WRD-1/WRD-2; parent independent rereview precedes source authorization.
Source inspected: task worktree `super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`, HEAD `7bdb3678`.
Workflow: caller-invoked brainstorming, autonomous Mode B; design-roast OFF, implementation review ON. No Cargo, Herdr, source edits, or subordinate agents were used.

## Goal and problem

Restore the original immediate Watch reconnect behavior: each newly admitted live Watch lease can physically restream previously offered ordinary identities that remain canonically pending. Historical mod design D4/D7 and its cursor contract require this. The existing test `reconnect_within_grace_keeps_wake_suppressed_restreams_and_late_ack_settles_once` stays an immediate causal RED-to-GREEN test. Waiting 60 seconds, relaxing its assertion, or attributing this to an expected new policy is not a repair.

V2 snapshots the shared ordinary ledger and marks every not-yet-due offer blocked. Ordinary Message items are deliberately absent from the durable invitation/control outbox. Consequently replacing a Watch cannot recover a recently emitted but unacknowledged ordinary message. Clearing the ledger, recording an extra offer, or putting messages into the invitation outbox would change established authority or retry limits.

## Main challenges

Separate one transport recovery pass from logical offers, preserve the current owner fence and canonical receipt/lazy eligibility, terminate at both full and nonfull capacity, and avoid doubling ordinary discovery work. Retained normal cursor/carry/partial state and nonordinary outbox tokens must survive recovery. State must have an exact lifetime even when the ledger is evicted and recreated.

## Selected decisions

Use one process-local frozen replay worklist in the current Watch registry Entry. At its first successful admitted V2 pull, capture at most 256 existing ledger ID/full-Offer pairs. Drain that fixed worklist through the existing exact-key canonical ordinary hydrator, using the existing ordinary source slice instead of either normal discovery or logical due traversal. Physical replay neither creates nor changes a logical Offer. No durable schema, public command, serialized protocol, presentation token shape, or model-visible string changes are needed.

The first Watch registration also captures existing Hook/V1 offers: transport recovery is defined by the newly admitted owner, not by a guess that a previous Watch existed. An empty initial capture is still initialized and cannot be refilled in that lease.

## 1. Owner state and lifetime

Recommended typed state (names illustrative, semantics required):

- `Uninitialized`, or `Initialized { scope, remaining: BTreeMap<MessageId, Offer> }`.
- Scope is exact seat (Entry key), binding generation, native session, current execution, 32-hex presentation lease and the existing 16-byte ledger incarnation. Generation/native/lease may reuse immutable Entry fields instead of duplicating them.
- Offer is the existing full Copy/Eq tuple: first_at, last_at, count, truncated. Do not substitute last_at alone.
- Initialized with an empty map is terminal for this lease. Signals, time, prefix/render changes and new offers do not refill it. A replacement registration creates a new lease and Uninitialized state.
- At most one worklist per Entry, at most `MAX_WATCH_CONNECTIONS = 64` entries, at most 256 pairs per list. Payload bound per list: 256*(128 ID bytes + 16 timestamp bytes + 1 count byte + 1 truncation byte), plus at most 128 execution bytes, 16 incarnation bytes and fixed bookkeeping. Native session/lease/seat are already bounded Entry values. Rust tree/allocator overhead is additional and bounded by 256 nodes. No body, rendered text, prefix or topic is retained.
- A request has one bounded clone of this list and the already existing due snapshot. Transient request memory is bounded by admitted live owner permits, not by historical connections.
- Drop/rebind grace cannot use replay authority. State may remain inert inside its bounded Entry until expiry/replacement; removal, disabled/stopped registry or daemon restart drops it.
- Ledger LRU eviction/recreation or a changed incarnation abandons remaining work and retains Initialized-empty for the same lease. It must not capture a fresh worklist in that lease. Ordinary DP5 discovery can offer pending content afresh after ledger loss. Thus there is no special persistence claim or repeated same-lease recovery caused by LRU.
- Generation/native/execution mismatch refuses the request via canonical admission, never changes another owner's state. An impossible stored scope mismatch is abandoned/refused; never infer continuity.

Why registry instead of ledger: the live lease outlives a possible 4096-seat ledger LRU eviction. Keeping the once-initialized bit in the Entry preserves the per-lease bound without an additional tombstone cache. This also limits retained replay state to 64 owners. A durable new table, historical lease map or epoch on every Offer would be larger and unnecessary.

## 2. Neutral interfaces and exact ownership

Add neutral default-inert methods to `ModChannels` for reading a replay snapshot and applying an exact scoped delta, implemented in `ModChannelRegistry`. A mock/no-mod registry cannot grant replay. Keep the existing presentation owner permit: acquired before exact Live lease admission and held through canonical store work, ledger bookkeeping and replay progress application. Register, unregister and close already share that permit.

Each registry method briefly locks the Entry map, verifies exact current Live seat/generation/native/lease, and clones/applies bounded state; no registry or ledger mutex spans SQLite, socket I/O or another global mutex. Canonical `watch_presentation_view` supplies execution and current binding; lease possession alone never authorizes a source read.

A new internal `PresentationInput` field carries the candidate snapshot and its scope; a new `PresentationOutput` field carries (a) completed attempt ID/stamps, (b) the emitted physical replay ID/stamps, and (c) an explicit ordinary source/bookkeeping disposition. The disposition distinguishes Normal, LogicalDue, OwnerReplay and None (outbox/control-only), including whether None occurred during an active owner recovery pass. It must distinguish an empty OwnerReplay quantum from ordinary logical allocation; emitted IDs alone cannot do that. Reuse the existing ordinary due snapshot's incarnation/stamps instead of inventing a second authority source. Validate Watch consumer, canonical claim scope, valid lease, <=256 unique IDs, and exact membership/stamp agreement with the known-offer snapshot before any mutation. Accept every nonzero recorded `u8` Offer count (1..=255); only zero is invalid. Counts above three can result from supported concurrent recording and must not be refused, clamped or rewritten. Hook supplies no owner replay snapshot. These are internal ports, not serialized additions.

Build first-capture state transiently; install initialization/progress only after a successful store return, including an admitted outbox replay or Unchanged response. Store errors/refusals do not initialize or drain the worklist. A pure snapshot getter must not quietly commit progress before store success.

Prune candidates whose current ledger Offer differs or vanished whenever preparing a snapshot, and again when applying a returned delta. Never resurrect an old stamp after `record`, `remove`, `retain_pending` or ledger recreation. Lazy exact pruning at these mandatory seams avoids callbacks from every ledger mutation into the registry. A stale delta can only remove an equal captured candidate; it cannot initialize a successor lease, replace its worklist, reset a due cache or alter its ledger.

## 3. Allocation and existing outbox

Keep all existing canonical admission, bounded offer reconciliation, publication maintenance and relevance advancement.

Existing nonordinary outbox replay remains first. Its token, immutable descriptors, slot masks and original codec remain authoritative. It hydrates no ordinary replay body and allocates no logical or physical ordinary item. Its continuation includes pending owner-replay work. The current Watch writes/flushes and confirms that token, then immediately follows continuation into a fresh pull. Never confirm, retire, clear or overwrite an old outbox just to unlock ordinary recovery. Existing canonical obsolescence retirement remains permitted exactly as today.

An unconfirmed applicable outbox can delay ordinary progress until its existing confirmation/recovery prerequisite succeeds; that preexisting prerequisite is not removed. The immediate reconnect regression with ordinary-only pending content has no such prerequisite. The replacement mixed-page regression must demonstrate successful old-token replay/confirmation followed promptly by ordinary restream.

For a fresh non-control-only allocation, select exactly one ordinary mode before query work:

1. If the admitted owner worklist has remaining exact-current candidates: owner replay mode, at either positive or zero post-reconciliation capacity.
2. Otherwise use existing normal mode at positive capacity.
3. Otherwise use existing bounded logical due-key mode.

Owner replay uses rotating output class 0; other four classes retain their existing output fairness. It never performs normal ordinary discovery or logical due traversal in the same call, even when it empties its list. Normal work resumes on a later admitted continuation.

Owner replay does not reset, fetch, consume, advance or reserialize the retained ordinary walk tuple: ordinary_version, ordinary_done, ordinary_cursor, ordinary_pending/carry prefix, ordinary_partial, ordinary_skip_body, ordinary_scan_passive_generation. Independent bounded publication maintenance still advances its publication head/after/current passive generation as it already does; those are incoming signals, not captured walk progress.

Within owner class 0, traverse candidate IDs in stable order using `ordinary_due_context` and `ordinary_presentation_message_by_id`. Reuse their exact canonical mode, instance, published-head, recipient/lazy state and effective-receipt rules, including manifest-first sparse receipt semantics and fallback. Reuse the existing assembler, 8192-byte ordinary truncation and source marker_lazy reconstruction. A replay hint alone proves neither addressability nor pending state.

For each attempted key:

- Charge an ordinary visit and check CallBudget before its exact read.
- Missing, unpublished, wrong-mode, nonaddressed, terminal, unavailable or invalid-current-stamp candidates are completed attempts without output; they grant no settlement or capacity.
- Hydrate at most one whole canonical body at a time; reuse the existing transient body-copy bounds.
- Measure candidate plus current outputs with Sizer. If it fits, append its normal Message item and offered/truncated metadata and record it in the internal physical-replay subset.
- If it does not fit after other output, leave the candidate unconsumed for its future standalone class-0 opportunity.
- If it cannot fit an otherwise empty page under supported ordinary truncation, complete its replay attempt without output or logical offer. This reuses the accepted standalone-unrenderable limitation: an unchanged lease has no repeated render-probe obligation. Existing logical traversal and its render/signal invalidation remain independent; a replacement lease supplies a fresh physical recovery opportunity.
- Never add a Message descriptor to the durable nonordinary outbox.

A same-call terminal reclamation removes that candidate. A missing key retains its logical Offer unless existing canonical reclamation proves terminal; finishing a physical attempt is not evidence that a receipt vanished.

## 4. Logical versus physical bookkeeping and races

Page offered/truncated metadata continues to describe the physical page for the existing Watch consumer. The internal emitted-replay subset tells service bookkeeping exactly which IDs must bypass `SeatOffers::record`. Assert the subset belongs to emitted ordinary items and cannot overlap a logical ordinary allocation in the same call. Keep an explicit logical-offer ID set: only successfully allocated logical offered/truncated IDs belong to it. Remaining logical IDs follow the existing record path unchanged. Pass only this logical-offer set as `consumed` to logical `ordinary_runnable`; never pass the combined physical page IDs. Physical replay leaves an existing due logical episode runnable, even if its ID was physically emitted on this call.

For every replayed ID, first_at, last_at, count and truncated remain byte-for-byte unchanged. Replay may bypass the 60-second/not-due/parked gate only for the captured current owner worklist. Existing `Offer::is_due` alone enforces the logical 60-second spacing and parks counts >=3. The sequential three-offer eligibility policy does not strengthen cross-route serialization: V1 and another route can both select a due count-two stamp, then record counts three and four. Preserve this accepted CIR-1 behavior and the full nonzero recorded domain. Owner recovery also accepts and replays pending count-3, count-4 and other nonzero `u8` stamps without incrementing them. After recovery completes, a due logical retry may independently run in the same lease; the once-per-lease restriction concerns the extra transport recovery pass, not existing legitimate logical offers.

Replay progress cannot set or clear native-wake outlook, reset read_attention/whole_read, create health evidence, ACK, accept or settle lazy state. Owner attempt outcomes cannot populate or clear ordinary due deferrals; existing independent due-cache render/publication scope maintenance still runs as before.

Use the explicit disposition for post-store service bookkeeping:

| Disposition | Native outlook bookkeeping |
| --- | --- |
| OwnerReplay, including an empty/missing-key quantum or mixed nonordinary output | Preserve the current `entry.more_pending` in the post-store ledger critical section; do not assign it from page metadata. Preserve current read_attention/whole_read. |
| None for outbox replay or control-only work during an owner recovery pass | The same preservation rule applies, including a pass whose remaining keys were exhausted/pruned on this call. No ordinary logical allocation occurred. |
| Normal or LogicalDue | Apply existing legitimate logical allocation bookkeeping using logical-offer IDs; owner continuation cannot supply native outlook. |
| None with no active owner recovery | Keep the existing unrelated outbox/control-only behavior unchanged. |

Do not restore a pre-store snapshot of `more_pending`: a concurrent V1 call may legitimately change it during SQLite work. Preservation means leaving the value found under the post-store ledger lock untouched. Filtering replay IDs inside `page.more && any_logical_message` and then assigning false is explicitly incorrect. The active-recovery disposition survives exhaustion on this call so the final replay page also preserves outlook. Actual canonical terminal reclamation and independently legitimate logical work retain their existing effects.

Store success then lost socket/output is a consumed physical replay attempt for that lease, just as ordinary offer allocation precedes reply today. Repeating a successful pull in that same lease does not reset it. A replacement lease immediately gets another attempt. Failed store/RPC-sizing/budget work returns no progress delta and transaction rollback leaves retained state intact. Successful stdout/flush still does not ACK; existing independent `AckModDelivered` alone settles cooperative message/lazy claims. Its grace and predecessor policies are unchanged.

The per-seat owner permit prevents replacement during allocation and progress installation. Existing V1 routes can still change logical stamps while SQLite runs: the canonical store transaction is the eligibility linearization point, and post-store delta application must require current incarnation and exact captured stamps. A changed/removed stamp abandons the obsolete work item and cannot overwrite newer history. It is not necessary to reinterpret the completed physical output as a new logical offer. Canonical ACK after this read may race with physical emission, as in the existing cooperative at-least-once transport.

No daemon-local lock can prevent bytes from an already returned RPC being written after a subsequent replacement. Existing old-lease confirm/pull fencing remains the authority boundary; this correction grants no new stale-owner effect.

## 5. Work and finite continuation

Owner replay shares, rather than adds to, the existing exact-key ordinary budget:

| Work | Bound per successful quantum |
| --- | --- |
| Ordinary source modes | exactly one of normal / logical due / owner replay |
| Exact replay attempts | <=100, charged to work.ordinary |
| Hydrated replay candidates | <=32 |
| Exact-key relation probes / availability seeks | <=1303 / <=100, using existing counters |
| Canonical offer reconciliation | <=256 visits / <=1280 probes |
| Total fresh physical source visits | <=433 (unchanged) |
| Final page | <=32 blocks and <=65536 actual encoded Watch bytes |
| Output sizing / final RPC | <=352 sizing passes; existing one RPC pass and wrapper ceiling |
| Persistent ordinary carry / partial / cursor | 155731 / 49415 / 8192 bytes, unchanged |
| Outbox-only replay | <=32 descriptors; zero ordinary hydration |

Rename an internal due-work counter/comment to exact-key work if that improves honesty, or explicitly document that its current name now covers both mutually exclusive modes. No duplicate 100-key allowance, extra ordinary page or second 32-body array. Existing query plans and linked-SQLite work proof remain applicable; new tests must exercise replay selection through that same path.

Continuation is independent owner_pending OR existing ordinary_runnable(current remaining capacity, consumed=logical-offer IDs only) OR existing nonordinary continuation. Exclude completed attempts and stale stamps before calculating owner_pending. At zero capacity, physical replay IDs remain eligible for logical due-key continuation because their logical stamps did not change. For example, with 255 parked count-three stamps and one due count-two stamp on the final physical replay page, owner_pending becomes false but logical ordinary_runnable stays true; a following pull can make the legitimate logical third offer. Never exclude that due ID merely because it appeared in the physical page. Returning an empty Page.more is justified only by a finite remaining attempt set, independently runnable logical work, or existing bounded maintenance work.

At a stable owner, each class-0 opportunity either removes attempts, emits other output that advances rotation, or defers a candidate for its bounded next standalone opportunity. At most 256 identities are captured. All-missing work drains in three ordinary quanta (100+100+56). Every renderable candidate can be physically replayed once under continued successful calls and eventual quiescence; five-way rotation supplies standalone output opportunities. Once the list empties, full-capacity parking and nonfull completed-pass quiet behavior are exactly the existing capacity design. No idle timer, wake, global drain cap, admission retry loop or durable normal-pass reset is added.

## 6. Compatibility, policy wording and rejected alternatives

Capability probe and accepted presentation_lease still precede all V2 state/output. Legacy fallback never initializes this worklist and remains byte-compatible. No new model-visible framing: expected added model-visible bytes per message/turn are 0. No new model-visible diagnostic is required.

Update TRUST-POLICY in the implementation commit: distinguish DP5's bounded logical offers from existing Watch physical reconnect recovery; state the 64*256 owner-local memory bound, lifecycle, unchanged offer stamps and output/work limits. This reconciles preserved D4/D7 semantics, not a new receipt provenance or durable guarantee. Retain daemon-restart and downstream silent-loss limits.

Separately reconcile A7 wording with the already-approved V2 Watch shared notice frontier and correct the stale bridge comment that says notices never use a channel (locate the current wording in the actual author baseline). This is documentation/comment alignment only, with no new notice settlement rule, provenance or accepted loss limit.

Rejected: resetting shared offers on registration (breaks Hook logical retry history); merely unblocking IDs in normal traversal (cannot recover full capacity and can lose retained partial/cursor progress); adding ordinary items to invitation outbox (conflates receipt and invitation authority); only replaying due IDs (fails immediate reconnect); 60-second test waiver; collecting pending bodies or a new durable replay relation; storing per-lease history across registrations; refilling the worklist on arbitrary attention/render signals; doubling exact-key work alongside normal traversal.

## 7. Required dedicated and mutation-sensitive evidence

Every listed assertion should fail when its targeted guard/branch is removed; avoid tests that only mirror helpers.

1. Actual CLI reconnect within grace, recently offered ordinary pending item: current failure stays immediate; observe replacement connection then body before any 60-second logical retry, no native wake, late ACK settles once.
2. Initial V2 Watch after existing Hook/V1 offer: immediate same-ID body, unchanged all four Offer fields and no logical increment for recovery, including initial count 3. Reuse the real CIR-1 V1 before-record interleaving to produce count 4; a newly admitted Watch must immediately replay that pending ID, preserve its exact full stamp, and permit independent nonordinary output. Validate the recorded domain 1..=255, retain zero-count rejection, and kill a mutant that rejects count >3. Do not replace the count-four producer with a fabricated malformed-state fixture.
3. Nonfull and exactly-full 256-ledger owner replacement: blocked recent known IDs replay; carried 257th unknown ID remains parked at full capacity; no reset of logical history.
4. Repeated pulls on the same lease, including repeated Attention at unchanged version and render-prefix changes: one extra physical recovery attempt per captured ID; finite terminal response, no empty continuation spin.
5. Replay whole, >8192 truncated, lazy, wake_on:all promoted lazy, manifest-only sparse pending and physical receipt fallback through actual canonical producers; framing, pending status and markers unchanged.
6. Missing/unpublished/nonaddressed/wrong-mode/terminal keys among 256 captured entries: no unauthorized body; misses drain in <=3 key quanta; no receipt settlement or unproven capacity release.
7. Current binding/execution/native checks; replaced/foreign lease; ReconnectGrace/RebindGrace; stale old-lease pull/confirm cannot initialize, drain or change successor state.
8. Owner replacement races with a blocked canonical read using deterministic barriers: existing permit serializes replacement; after replacement the old lease is refused.
9. V1 record/remove while store read is blocked; same-generation ledger LRU recreation: full-stamp/incarnation merge guard prevents overwriting new offers; same lease never recaptures after eviction.
10. Error before transaction commit, final sizing refusal, budget exhaustion: no replay progress installed. Success then dropped RPC response: same lease does not repeat; replacement lease recovers immediately. Failed stdout/flush similarly leaves ACK/lazy pending and replacement recovery intact.
11. Unconfirmed old nonordinary outbox containing invitation/control: original token/descriptors/slot masks/codec are preserved, physically replayed and normally confirmed; ordinary restream follows continuation. No repair retires old token early. Retain `activation_v2_watch_replacement_replays_uncommitted_output` assertions.
12. Byte-for-byte normal carry/cursor/partial/captured scan tuple preserved throughout replay at both capacities, including a partial oversized message and nonempty 257th carry; after replay, ACK frees capacity and continuation resumes exactly there.
13. Existing logical `is_due` spacing/threshold and accepted cross-route concurrent-record behavior remain unchanged across physical recovery and both consumers. Dedicated mutant calls record() for replay and must fail full-stamp assertions; mutant skips owner path at slots>0 must fail nonfull test; mutant skips it at slots==0 must fail full test.
14. Actual encoded output bytes <=65536, <=32 blocks, 100 attempts/32 hydrated/1303 probes/100 seeks/433 source/352 sizing bounds; fixtures make normal and due paths available concurrently and prove only owner source runs.
15. Legacy capability-disabled fallback/probe-before-effects and serialized bytes remain unchanged; Hook does not obtain Watch replay or owner state.
16. Daemon restart: no stale lease survives, pending data stays canonical, fresh ledger normal offer recovers without resurrection of old local progress.
17. Full-ledger final-page continuation: 255 parked count-three identities and one due count-two identity arranged on the final owner replay page. After physical emission, exact stamps are unchanged, owner debt is empty, and more remains true solely for the logical due key. The following logical allocation increments only that due stamp. A mutant feeding combined physical IDs into logical consumed must fail.
18. Native outlook preservation: exercise initial more_pending=true and false, empty/missing replay quanta, mixed nonordinary output, outbox/control-only work during recovery, and the final exhausted replay call. Add a deterministic concurrent V1 change while the store call is blocked and assert the post-store current value survives. Compare more_pending, read_attention, whole_read and next_pull, allowing only actual canonical settlement/independent legitimate logical effects. A live-channel no-wake assertion alone is insufficient because registry suppression hides corrupted outlook until handover. Mutants that assign filtered-false, restore a stale snapshot, or lose the empty/recovery disposition must fail.

## Scoped review disposition

WRD-1: recorded count validation explicitly accepts 1..=255 and preserves the demonstrated concurrent count-four history; added its owner-replay/nonordinary-output and zero-rejection test requirements. WRD-2: explicit source/bookkeeping disposition distinguishes empty/recovery-only work, logical consumed IDs exclude physical replay, and current post-store native outlook is preserved in both directions. Added the final-full-page due-key and concurrent/outbox/control/empty outlook mutants. Existing independent due-cache scope maintenance remains intact. No architecture, source budget, durable schema, wire field or model-visible byte allowance changed.

Run focused tests and mandatory per-change lint/neutral check only under the parent's lane. Parent independently reviews this design before authoring; implementation receives required independent code review. This handoff itself makes no test-success claim.

