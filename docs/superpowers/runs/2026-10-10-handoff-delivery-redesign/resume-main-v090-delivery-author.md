# Main v0.9.0 delivery composition author proposal

Status: parent approved the concrete proposal and signatures; seven owned production files authored and QUIESCENT for parent compilation/review. The initial proposal preceded every source edit. Author inspected immutable Main b6dd5dd122772599a02e2cc7f76b20e5492f0af3 TRUST policy and relevant implementation, candidate D f330bdab and the unresolved merge in `.worktrees/upstream-composition-v090`. No Cargo, runtime, staging, commits, subagents or source mutations. Historical D owner-replay evidence remains historical and does not validate this architecture composition.

## Contract being composed

Main binds channels, offered ledgers, notice/warning frontiers and unread cursors to a canonical BindingId. Sibling sessions of one seat remain independent even at the same mapping generation. Claims resolve by seat/harness/native-session/agent identity; connected top-level bindings can receive delivery regardless of enrollment provenance. A seat receipt settles once, but a sibling still sees settled mail past its own cursor. A display ACK can advance only its own binding; another binding's ACK is not evidence this reader saw the message. Main's ReaderBound cursor semantics and reconnect path are normative, not optional fallback behavior.

D adds durable physical presentation by consumer, bounded ordinary carry/exact-key recovery, emitted-control confirmation, and one owner replay capture per successful live Watch lease. Physical replay changes no logical offer stamp or native outlook. It cannot coalesce independent Main bindings under a seat key. Existing per-seat mutex serialization can remain, since it only serializes decisions and does not select their owner.

## Minimal source plan and interfaces

Owned source remains the six assigned modules plus essential announced `src/service/delivery_presentation.rs`; that internal module owns all V2 Pull/Confirm admission, ledger snapshots and apply. No other extra source paths requested. Store, schema, frontend and test authors own their respective changes. Parent alone stages/commits/builds.

1. Keep Main `OfferedLedger` keyed by BindingId, `binding_mut(seat, binding_id, generation)`, 4096-entry LRU bound and 256 offer bound per binding. Integrate D invitation display bookkeeping and the two consumer due caches into BindingOffers. Bind every due snapshot to the exact BindingId, generation and random ledger incarnation; replace the obsolete snapshot native-session identity with BindingId. The entry stores its own BindingId so `due_snapshot(consumer)` stays self-contained. Full Offer stamps, including recorded count domain1..255, remain unchanged; admission ceiling3 remains a separate scheduling policy. No seat_mut compatibility shim that silently aliases sessions.

2. Add canonical `binding_id: BindingId` to WatchReplayScope and internal PresentationInput. Registry replay lookup/apply uses that binding's entry, then seat/generation/native/lease/incarnation/full-stamp comparisons. `presentation_owner_live` and `record_presentation_attention_push` gain an explicit BindingId after seat. `presentation_replay_snapshot` can use known.binding_id to address the entry, and also verifies canonical service claim scope. Store independently resolves the claim and compares input.binding_id before using any hint; the internal field grants no authority. Keep MAX_WATCH_CONNECTIONS=64, at most256 stamps per owner, single successful capture including empty, no recapture after incarnation loss, and existing failure/refusal no-progress semantics.

3. Keep Main ModChannels registry/by_seat/cooldowns and all same-binding register/close/grace/stall behavior. Retain D owner permit across admission, store decision, current-ledger update and replay apply. Per-seat weak permit cache is allowed to serialize siblings. Register/close/disconnect acquire that seat permit but mutate only the addressed binding. Never hold registry/ledger lock over store work or nest those locks. Existing lease is still obtained from the actual accepted channel ID.

4. `ModStoreReads::presentation_seat_view(&SeatId, ModBindingRef, &CallBudget)` parallels Main mod_binding_view while preserving D bounded canonical admission and zero ordinary digest counts. Watch registration and V2 claim admission use ModBindingRef::Claim; registry pass uses Id. Proposed refresh interface is `presentation_refresh(&SeatId, &BindingId, generation, native_session, &CallBudget)` (add the binding ID, retain the existing exact generation/native fences). Store refresh independently resolves that binding and canonical conditions, so no same-native sibling or latest-binding fallback can reset another consumer. Main pending unread counters cannot be reintroduced through a legacy mod digest on the V2 path.

5. Preserve Main V1 hook plain reconnect and its peer argument. V2 Hook must also share the canonical reconnect behavior without a legacy ordinary digest; this needs the existing bounded presentation view plus the same canonical reconnect decision, then a fresh bounded presentation view. Proposal: pass PeerIdentity from dispatch into V2 Pull/Confirm and the Hook admission helper; Watch remains connected-only. This point requires parent/interface confirmation before implementation because calling Main's current hook_plain_reconnect returns a full mod view and would violate D's zero ordinary count path if used unchanged. Do not silently gate V2 on cooperative_top_level provenance, or silently omit Main's legitimate same-session reconnect.

6. Assemble merges Main's FirstContact=>Pending case with D's full invitation display branch. Keep ordinary ACK candidate derived from canonical receipt state, lazy/promoted distinction, bounded bodies, codecs and control descriptors. No adapter policy changes, no new wire claim field and no binding ID supplied by an untrusted client as authority.

## Store/schema coordination

Agreed with store_binding (awaiting combined parent approval):

- seats::presentation_seat_view(db, instance, seat, binding: ModBindingRef).
- ordinary_presentation_page(db, context, instance, claim: &CallerClaim, cursor, budget): q.claim=Some(claim.clone()), hence ReaderBound cursor.
- ordinary_due_context(db, instance, seat, binding: &BindingId, work).
- ordinary_presentation_message_by_id(db, instance, seat, binding: &BindingId, id, due_context, work).

Canonical normal and exact-key eligibility is binding unread OR seat pending ACK obligation, with lazy unread scoped to binding. ACK candidate is present only for actual pending receipt. Indexed key lookup happens before filters and every attempted candidate remains charged to existing physical visit/probe budgets. Message reclamation, outbox revalidation and Confirm retirement must likewise preserve another binding's still-unread message after a sibling ACK; seat terminal receipt alone is insufficient. This requirement is substantive and must be implemented by the durable-store author, not assumed satisfied by changing primary keys.

Schema author's proposal matches required isolation: consumers and warning proofs PK(binding_id,consumer); items PK(binding_id,invitation_id,consumer); each binding_id FK to occupant_bindings, retain seat_id, binding-leading seek/cleanup indexes; consumer deletion matches OLD.binding_id+consumer. Canonical invitation/context/confirming projections remain shared by seat/thread. Main versions1..35 immutable; unpublished D migrations are renumbered by schema owner (old40/41 to44/45 after old32..39 shift). No rewriting released schema or D evidence.

Notice and warning frontier reads/settlement must use the actual binding's canonical frontier, never latest occupant. D retains post-emission exact-control physical confirmation; duplicate/foreign/stale token confirmation cannot retire another binding's descriptors or advance its native attention. Whole ordinary physical Confirm remains separate from transcript/mod ACK and cursor authority: only actual admitted display/ACK commands move canonical read cursors under Main.

## Causal proof scope required before behavior clearance

Parent-owned tests must add negative controls before production behavior changes are considered validated; no existing D PASS substitutes for these:

- Two live same-seat/same-generation bindings A/B: both channels accepted; A replacement/close/reconnect/stall changes only A. Wrong-binding lease and replay snapshot refused; B's owner worklist and ledger stamps unchanged.
- Same ordinary ID offered independently to A/B. A's physical replay consumes no logical offer count/outlook and cannot consume B's recovery; recorded count4 remains accepted. Full256 A does not block B; existing64 owners remain bounded.
- A reads/ACKs canonical message while B remains unread: B still receives it marked settled/no ACK requirement; B's existing outbox/carry and due key remain eligible until B's own display cursor passes it. A foreign confirmation/token cannot advance B or delete B's outbox.
- Same informational notice and invitation context on A/B: A confirmation/notice frontier does not hide B; B sees its own current control episode. Replacement A resumes its own durable page, not B's codec/masks/descriptors.
- Binding generation rotation, execution change and same-generation ledger recreation each fence stale progress. Registry restart discards leases, durable page remains binding-scoped; ended/subagent/unknown claims refused without enrolling on reads.
- Main V1 and V2 Hook legitimate plain reconnect remain equivalent in authority, with V2 zero ordinary digest counts and bounded probes proven. Watch never gains reconnect enrollment authority from Pull.
- Retain D original bounded owner replay, more-only continuation, no ordinary walk when full, current post-snapshot outlook, prune-all passive control and genuine logical allocation discrimination tests, adapted to canonical IDs rather than fabricated seat aliases.

Open decisions for parent: approve exact refresh/Input/snapshot signatures and essential delivery_presentation module; confirm the shared-path bounded Hook reconnect signature without adding an unbounded legacy digest. No source edits until these are resolved.


## Interface follow-up before core edits

The existing reconnect helper is already within the assigned service dispatch and store seats modules. Proposed exact adaptation: add `presentation: bool` after claim in StorePort::hook_plain_reconnect (before budget), pass it through the service helper (after peer, before budget), and add it after claim in seats::hook_plain_reconnect. V1 passes false and V2 Hook true. One unchanged canonical execute_decision/cooperative_mapping/resolve_cooperative path then selects bounded presentation_seat_view or legacy mod_binding_view. Parent confirmation requested; no additional enrollment algorithm or extra owned module is needed.

Quality owner and delivery author agreed `settle_pulled_notices` adds canonical `&BindingId` immediately after seat, retaining generation/native guards. Quality owns attention helper; delivery owns port and service call; parent owns store/mod forwarding. This removes the remaining native-only selector ambiguity.

Store-binding author identified a substantive bound adaptation that is **pending explicit design disposition**, not silently covered by the historical D budget: settled-but-unread ordinary eligibility needs five additional canonical keyed relation probes. Proposed worst case18 probes per ordinary due/replay attempt rather than13; setup3 plus100 attempts gives1803. Pending-receipt branch stays13, lazy max11;100 attempts/32 hydration/100 availability seeks stay unchanged. Parent informed. Physical ordinal seek before canonical filtering remains required so bound accounting counts inspected rows rather than only accepted rows.


## Approved source handoff (uncompiled)

Parent approved all interfaces, the bounded reconnect view mode and the explicit3+100x18=1803 keyed-probe ceiling. Main ledger bound4096 bindings x256 offers and registry global64 Watch owners x256 captured stamps remain unchanged. Negative sibling tests are being authored by independent assigned test owners; the merge cannot execute the legacy RED state yet, which is recorded rather than represented as actual evidence. No behavior clearance is claimed.

The seven owned files now have their merge markers resolved and their binding adaptation authored. The shared reconnect helper takes presentation after peer at service level and after claim at the StorePort/store levels. V2 Hook passes true, V1 false; Watch checks connected-only state. Service resolves canonical binding before its live-lease lookup. Registry close/unregister obtains the serializing seat from the exact binding entry rather than mistaking the binding map key for a seat. Replacement drains only that binding's old sink outside the registry lock. Passive context frames keep exact channel fencing. Replay apply rejects a sibling current snapshot before capture installation; same-binding incarnation loss retains the original finite-abandon behavior.

D invitation display cache is now named MAX_INVITATION_DISPLAYS_PER_BINDING; ordinary uses Main MAX_OFFERS_PER_BINDING. BindingOffers holds its own binding_id for self-contained due snapshots. Assembler keeps Main FirstContact suppression and D invitation presentation. Notice settle passes admitted binding_id. No wire shape, schema, adapter policy, migration, test file, unassigned production path or real config was edited by this author. No Git staging/commit or Cargo command was run.

Static verification: rustfmt with explicit seven files and skip_children=true succeeded; scoped git diff --check passed; no merge markers remain in these seven files; static old seat_mut/MAX_OFFERS_PER_SEAT references are absent from owned service/ledger modules. This is parse/format/source inspection only, not compilation or runtime. Store/other authors continue their disjoint work. Parent owns Git, compilation, all tests, strict clippy, default-feature/neutral gates and fresh review.

Current file SHA256 values (working bytes, not a commit claim):

| Path | SHA256 |
| --- | --- |
| src/delivery/assemble.rs | 6ae75481b5b23ad0d018d25de534e1a746125ade879aed0c7fe39228086e751d |
| src/delivery/ledger.rs | 820269f009a37f23ae98ac14a3f674f5fe9293dbe495bc0679f688bcd4e40ff0 |
| src/ports.rs | 5d4cb3d195f7171db609d54c1c1a98df5786dc2947bcd7400342aa29c11e7c51 |
| src/service/delivery_pull.rs | 16174645c500eb4adedb0a72009a3a58637634c6b0952e298fdc6f4c35363bce |
| src/service/dispatch.rs | b270b6fff247108bf145fc0e70b2484f3693c2718e37a6c41c070be2f0b3f530 |
| src/service/mod_channels.rs | b15f8d75b50536a4dafc4cb182a4f4b4299deed3f5fdd6cebd55cd6647b33485 |
| src/service/delivery_presentation.rs | 8c1c71615d7b87208f07517575be6fa60849c3700bb65760ec0d51f4e74ac2a2 |

Source work QUIESCENT. No processes were started. Runtime and cross-binding causal evidence remain pending.
