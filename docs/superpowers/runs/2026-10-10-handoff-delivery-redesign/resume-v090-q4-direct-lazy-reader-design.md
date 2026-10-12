# Q4: initial direct Lazy delivery and binding cursors

## Goal

Make a published Lazy body explicitly addressed to the recipient of a new immutable direct pair readable before invitation acceptance. Preserve that binding's unread body through the initial acceptance transaction, and allow actual complete text body/inbox display to advance only the displaying binding. Preserve invitation consent, immutable Lazy mode, independent ACK-required handoff semantics, and Main35's ordinary joined-thread/no-backlog rules.

**Status: architecture draft, not production authorization.** Parent supplied the clean diagnostic candidate `07fdf88d` while running its canonical diagnostic. This draft inspected source and the earlier registered-hook6 5-pass/1-fail evidence; it did not run or receive the new diagnostic. No source/test edits, Cargo, runtime, Git writes, Herdr or nested agents. The only write is this document. Design roast remains OFF; counters unchanged. This uses the caller-invoked brainstorming workflow under the user's autonomous authorization; the parent owns the next decision.

Source references are relative to the nested `upstream-composition-v090` candidate. References to other run documents use this outer run directory. The existing failing test remains authoritative until the parent has the exact predicate witness.

## Decision and limits

The requested **initial-episode** exception is feasible using existing canonical rows, without a migration, wire change, version gate, new sender-side binding fanout or binding-birth timestamps. It requires four coordinated changes, not merely a predicate edit:

1. A shared per-binding thread-eligibility predicate admits an exact published Lazy recipient in the active initial direct invitation.
2. Initial direct-recipient acceptance preserves each existing cursor and initializes missing open-binding cursors at zero rather than advancing them to the head.
3. The separate Lazy body-continuation eligibility check uses the same thread rule.
4. `AdvanceCursors` admits exact published addressed Lazy direct messages for that reader before joining; public full text `body` reads then work without acceptance.

Two limits must be explicit before implementation:

- **New bindings while the initial invitation remains pending start this newly admitted stream at zero if they have no cursor.** Thus a binding born after a sibling displayed the body may see it independently before acceptance. This does not alter Main35's seat-maximum/head seeding for joined threads. Avoiding this initial-invite replay would require additional enrollment-time baseline work or durable baseline state; an unbounded `MAX` across bindings in each candidate predicate is rejected.
- **The initial-only exception does not repair all later reinvitation cases.** A cursorless binding during episode 2+ still gets no automatic Lazy stream. This excludes newly addressed bodies in that later episode as well as older history. Existing bindings with cursors retain their current behavior. If the intended contract is automatic delivery for *every* fresh direct body during any active invitation, the initial-only approach is insufficient; see Alternatives. This limitation must not be hidden behind the phrase “no backlog.”

Acceptance itself must never be treated as body display. Existing explicitly documented lifecycle fast-forward after a joined binding resumes remains a separate accepted Lazy-mail limit; this proposal does not promise survival against that deliberate cursor advance. Existing already-accepted pairs whose cursors were previously seeded past a body cannot be safely repaired by rewinding: the retained rows do not distinguish join-based skipping from legitimate display/fast-forward for each binding.

## Grounding and actual cause to verify

The direct-thread design explicitly includes an invited target in a frozen Lazy audience without implying joining, attention or receipt (`2026-10-10-automatic-one-to-one-threads-design.md:25,49` in its run directory). The handoff redesign itself always uses an addressed ACK-required ordinary body (`2026-10-10-handoff-delivery-redesign-design.md:40`); its pre-acceptance receipt source is separate from this bug.

`src/store/control.rs:695–726` joins/initializes only the creator, then creates the recipient's episode-1 invited membership. `src/store/messages.rs:420–425,660–689` stages the direct Lazy recipient independently from ACK-oriented invited recipients. `src/store/lazy_delivery.rs:41–68` currently requires a cursor or joined membership before a modern Lazy row is unread. `src/store/queries.rs:6825–6850` scans the raw addressed row and applies that predicate before hydration. Physically retained `state='pending'` is not modern binding unread authority.

The existing actual hook test fails at Message count zero after establishing exact claims, actual Ensure/Send, passive invitation context, byte accounting, invited membership and zero receipts/wakes. The parent's diagnostic must establish exact missing cursor, modern `legacy_pending=0`, matching publication and false reader eligibility. No extra hook loop can change an unchanged eligibility predicate.

## Canonical eligibility

Factor the **thread eligibility** half from `binding_lazy_unread_sql!` so both the complete-message predicate and the continued-body path share it. Express it with explicit SQL expression parameters for binding ID, recipient seat ID and thread ID; do not require callers to add artificial `occupant_bindings`/`lazy_recipients` joins solely to obtain alias names.

The complete modern rule remains:

```text
message.sequence > COALESCE(own_cursor, 0)
AND reader_thread_eligible(binding, recipient, thread)
```

The existing pre-V34 `legacy_pending=1 AND top_level` branch remains unchanged. `reader_thread_eligible` is:

```text
own binding_cursors(binding, thread) row exists
OR exact memberships(thread, recipient) row satisfies:
    existing state='joined'
    OR initial_direct_invitation:
        state='invited'
        AND episode=1
        AND COALESCE(voluntary_state,state)='invited'
        AND exact direct_threads(thread_id) row names this recipient
        AND that pair's instance equals recipient seat.instance_id
        AND recipient seat.state != 'retired'
```

The pair lookup is by UNIQUE `direct_threads.thread_id`; seat membership is a scalar equality to either endpoint of that one row, not two fanout scans. The seat lookup is by its primary key. Membership is UNIQUE `(thread_id,seat_id)`. The existing cursor key is PRIMARY KEY `(binding_id,thread_id)` WITHOUT ROWID. These are at most two additional unique relation probes per evaluation, not a scan of pairs, invitations, bindings or history. Correlation to the published addressed row and binding seat remains outside/alongside the predicate in each existing caller.

**Why voluntary state is mandatory:** rejection updates `memberships.voluntary_state` to `absent`/`left` but can leave raw `state='invited'` and episode 1 (`control.rs:1266–1273`). `service_substrate.rs:191–260` explicitly warns that compatibility rows can remain invited after cancellation. A raw-state-only exception would expose rejected content to a fresh reader. Use the same COALESCE interpretation as effective voluntary membership.

**Why no new direct-marker column is needed for the initial case:** the pair is immutable; episode 1 identifies its never-joined recipient, while the creator begins joined. Ordinary Lazy broadcast never stages an invitee. Therefore an existing canonically published Lazy row for that initial invited pair recipient is the durable addressing fact needed here. The proposal does not infer authority from the topic, body, public ID prefix, local journal, insertion time or current caller's wishes. It does not attempt adversarial reconstruction of historical request JSON.

**Why no invitation-history query is needed in the proposed predicate:** canonical initial Ensure creates exactly episode 1; rejection clears voluntary invitation; retirement is excluded directly. Pair channels cannot be service-owned, and required invitation creation requires the calling service to own the thread (`service_controls.rs:254,307–310`), so a required-episode cancellation cannot legitimately replace the initial pair invitation. Ordinary duplicate invitations reuse its active episode; fresh reinvitation increments the episode. If implementation inspection finds another legitimate writer that violates this invariant, do not weaken the condition: add an exact episode-1 invitation/cancellation/rejection check and recount its probes, or stop for a revised design. This is a proof based on existing canonical writers, not a promise about hand-edited corrupt databases.

No seat-wide legacy display flag is read to settle modern work. No read creates a cursor. A late new binding after rejection/leave/reinvitation fails the initial condition and still has no automatic historical stream. A binding already holding a cursor retains its addressed work through the existing first branch.

## Initial acceptance transaction

Current `accept` changes membership and then calls `read_cursors::init_for_join` (`control.rs:1103–1118`). That helper upserts every open binding at `MAX(existing, thread_head)` (`read_cursors.rs:136–152`), losing a staged pre-acceptance Lazy body even if the predicate was fixed.

Before changing the selected invitation or membership, capture a canonical boolean in the same deciding transaction:

```text
selected invitation is the recipient's exact episode-1 pending invitation
AND current membership meets initial_direct_invitation above
AND this operation is the first transition into joined
```

Use a named internal cursor-initialization policy, or a dedicated helper, so this decision is passed explicitly. Do not make every `init_for_join` caller guess historical intent after membership has changed.

- Ordinary create/join/rejoin/accept-required and all noninitial transitions retain `ThreadHead` exactly.
- This initial direct accept uses `PreserveInitialDirect`: insert missing open-binding cursor rows at `0`; keep existing cursor values unchanged. It does not consult or copy another binding's maximum. A sibling that displayed everything cannot erase this binding's unread prefix.
- Use the same existing open-binding selection as `init_for_join`; disconnected-but-open bindings remain included, ended bindings remain excluded. This **replaces values in the already existing join fanout**. It adds no fanout at Ensure, send, publication, hook pull or first-contact read. The join transaction already has O(open bindings of this seat) writes; this design does not claim that inherited operation is O(1).
- Compute the preserve policy before mutations, apply it only after the existing successful transition checks, and commit invitation state, membership interval and cursor initialization atomically. An error rolls back all three. Saved accept replay remains an exact historical result and does not reseed anything.

Zero is safe here because the new pair has no unrelated recipient membership history. All receivable Lazy content before first acceptance was explicitly addressed to this recipient. Nonreceivable system events/sender-only messages remain filtered; ordinary pending obligations retain their existing path. This is an initial addressed stream, not general thread-history enrollment.

A first caller accepting an initially unbound recipient resolves/enrolls its binding before the transition, so that binding is included. If another binding enrolls after the acceptance commits, its existing Main35 `init_for_new_binding` uses the seat maximum or head as before. Do not alter `seed_preview`, lineage copying or general joined-thread enrollment in this minimal option.

## Whole bodies, continuation and completion

### Read path

There is no production command named `PeekLongBody` in the inspected source. The public inspection path is `body MESSAGE`, represented by `Command::Message(MessageQuery)`. `queries.rs:2609–2699` reads by canonical message/instance and validates body cursor length/UTF-8 offset; it does not require invitation acceptance. Keep that read-only query behavior.

The separate continued-Lazy path currently checks only cursor-or-joined at `queries.rs:6803–6821`; changing the main macro alone leaves long pre-acceptance bodies broken. Replace that membership half with the shared `reader_thread_eligible` rule. Retain the existing exact captured message, addressed recipient, manifest/publication cutoff, ordinal range and binding fence validations (`:7135–7189`). A continuation intentionally keeps its captured body identity even after concurrent completion; do not replace this with the full sequence-unread predicate and thereby apply an old offset to another message or reject an otherwise valid captured tail. No acceptance, completion or cursor write occurs while querying a chunk.

### Actual text display

`cli/mod.rs:1175–1189,1320–1368` sends `AdvanceCursors` only after the body output path succeeds, only for ordinary content whose returned chunk completes the body from offset zero or a native body continuation. `output.rs:276–306` writes and flushes before returning. JSON/machine reads and explicit nonzero-offset partial reads do not move cursors. Preserve the documented inherited limitation that a continuation begun from an explicit offset is counted whole; this repair must not claim stronger contiguous-read evidence than the CLI records.

`control.rs:2596–2661` presently resolves the correct binding, collects exact message sequences, then silently ignores every unjoined thread. Extend admission **per message**, not by broadly granting a new thread to every supplied ID:

- Keep the existing joined-thread case unchanged.
- For a nonjoined thread, admit only an exact published addressed Lazy row in an immutable pair naming the caller seat, when this binding has an existing cursor for the thread **or** the active initial-direct invitation rule holds. Require the manifest/message/thread/instance relation to agree. This supports current initial pre-acceptance display and an already-established pair stream after departure; it grants no cursor to a fresh rejected/left/reinvited reader.
- Filter nonadmitted IDs before constructing the per-thread display-sequence vectors. An ordinary/system/unaddressed/foreign-instance body cannot bootstrap a new initial direct cursor. Existing error treatment for nonexistent supplied IDs is retained.
- Use the existing `advance_through_displayed_with_change` walk for admitted sequences. It stops at the earliest unshown receivable message, cannot skip a preceding Lazy gap, and changes only this binding's cursor. Coalesce `presentation_reader::note_binding_reader_change` once for the actually changed binding, exactly as the current handler does. That helper uses the exact binding and two fixed Hook/Watch consumer points, never a seat-wide reset.

Cursor admission is based on current canonical eligibility in the deciding transaction. If acceptance commits between reading and advancing, the ordinary joined case applies and the preserved cursor can advance. If rejection wins before a cursorless reader submits its display, the narrow new path remains refused/ignored; the content was still readable through `body`, but the display does not automatically establish a post-rejection stream. Existing `CompleteInboxDelivery` has its own addressed-published completion semantics; this proposal does not redefine all terminal display behavior.

### Existing completion paths

`CompleteInboxDelivery` already checks exact Lazy mode and addressed publication, decides the canonical caller, matches the binding and calls `complete_for_binding` (`control.rs:2501–2591`). `lazy_delivery.rs:475–559` groups exact sequences, advances only the matched binding, preserves legacy bookkeeping, and performs the passive consumer invalidation when changed. It does not require joined membership. Keep this path, including its no-receipt behavior.

Own default text inbox uses durable contiguous chunk accounting in `cli/lazy_display.rs:26–123`; only complete flushed chains enter displayed/Lazy completion candidates. `cli/mod.rs:1722–1777` advances this binding before top-level Lazy settlement. Subagent own-stream `AdvanceCursors` behavior remains an implementation contract; completion/ACK remains refused for subagents. This design task itself performs none of those operations.

AGY physical `DeliveryPresentationConfirmV2` remains output bookkeeping only. It must not advance a read cursor, ACK a body or accept an invitation. Existing logical DeliveryConfirm/transcript evidence or the caller's own complete text display supplies the relevant completion. A truncated hook frame or content-free wake supplies no display authority.

## Bounds and accounting

The proposed shared predicate adds **two conservative unique relation probes**: exact pair and exact nonretired seat. It reuses the existing exact membership probe; there is no extra membership-history query. Charge both on each relevant Lazy predicate evaluation even when SQLite short-circuits. Query-plan tests must demonstrate UNIQUE/PRIMARY KEY searches for pair, membership, seat and cursor with large unrelated populations.

| Path | Existing bound/cost | Proposed accounting |
| --- | --- | --- |
| Exact Lazy due/owner replay | 1 mode + 7 eligibility + 3 hydration = 11 | 1 + 9 + 3 = **13** per attempt; charge `ordinary_due_key_probes` accordingly |
| Pending ordinary due | 13 | unchanged |
| Settled unread ordinary due | 18 | unchanged; still the maximum |
| Due/owner replay total | 3 setup + 100×18 = 1803; 100 seeks; 32 hydrations | **unchanged caps**, since Lazy13 ≤18 |
| Retained carry | 3 + 32×18 = 579; 32 seeks/hydrations | unchanged cap; Lazy branch charged13 |
| Lazy known-offer reconciliation | 1 mode + 7 eligibility = 8 | **10** per key; fits existing 256×10 =2560 |
| Fresh private ordinary source | ≤100 physical candidate visits | at most **200 additional keyed probes**, charged before filtering, including a continued Lazy body visit |
| Private ordinary continuation | existing ordinary-only 15-probe special validation | unchanged for ordinary; Lazy continuation's two new probes are charged with its one normal candidate visit |
| Initial acceptance | existing O(open bindings) cursor upsert | same selected bindings/write cardinality; fixed predicate lookup overhead; no history scan or new send fanout |

Add an internal normal-source counter, e.g. `ordinary_lazy_scope_key_probes`, returned through `QueryModePage`/`OrdinaryPresentationPage` into `PresentationWork`, incremented by 2 exactly when the normal private Lazy reader-scope query is executed. Enforce `<= 2 * ordinary` and `<= 200`; do not charge it in carry/due modes that already include the probes in their own counters. `scan` visits a candidate once before output fitting (`queries.rs:7242–7252`); binary-search sizing does not rerun SQL. A continued Lazy item consumes one visit too. One ordinary source query per fresh quantum, 100 visits, 32 blocks, 65536 physical bytes, carry155731, partial49415, discovery433 and sizing352 remain unchanged. Document the added relation work separately rather than pretending discovery433 counts SQL relations.

Update the branch comments/counters at `queries.rs:7457–7461` and `invitation_content.rs:1801–1807`, plus policy statements that currently call Lazy11 and reconciliation8. Preserve the runtime caps at `invitation_content.rs:2828–2842`. No extra receipt availability seek or hydration is needed for the initial-direct predicate.

**Inherited limits that this draft does not conceal:** the public/nonprivate Lazy candidate/window/stat readers contain unread predicates before their SQL LIMIT (`lazy_delivery.rs:73–83,141–160,360–453`). A visible-row LIMIT is not a physical-inspection bound; the additional two point probes can be paid for each physical candidate considered. `seat_unread_sql!` also examines qualifying bindings, as before. The proposed change adds no new nonunique join or binding enumeration, but cannot make those existing paths globally constant-time. Separately, `advance_through_displayed_with_change` (`read_cursors.rs:265–302`) walks thread messages to the first unshown receivable row without a row cap. Starting at zero can traverse many nonreceivable system events. This is existing completion machinery, not part of the private 100-candidate presentation envelope. If the parent requires a strict new end-to-end mutation work bound, this minimal design is **not sufficient**: the display walk needs a separately designed bounded-progress mechanism; do not bury an unbounded first-recipient seek or `MIN` join behind a one-row SQL result. Tests should include large initial system-event history to expose that inherited cost rather than label it bounded.

## Alternatives and why they are not silently selected

1. **Initial-only, no new schema (recommended for this stated narrow repair).** The design above has exact point eligibility, reuses existing cursor state and preserves generic no-backlog semantics. Costs and limits are explicit. It fixes the current registered new-pair case and initial acceptance, with independent before-acceptance binding streams.

2. **Current-invitation episode boundary, also potentially no schema.** To deliver fresh direct work during episode2+ without reviving older history, look up the exact current invitation by UNIQUE `(thread,seat,episode)` and use its canonical creation decision plus message publication decision as the boundary. Acceptance would need the exact thread sequence corresponding to that episode's canonical invitation event as the seed floor, preserving already-established own progress appropriately. This uses canonical order, not heuristic timestamps. However, it requires an audited event mapping, cancellation/rejection checks, later-episode lifecycle/replay semantics and additional probe accounting. It cannot be approximated by dropping `episode=1` or simply preserving every pre-reinvite cursor. It is a separate refinement, not implementation-authorized here.

3. **Durable directed-reader baseline/obligation state.** If the requirement includes no initial-invite replay to later bindings, preservation across every rejection/reinvite/acceptance, recovery of already-head-skipped historical rows, and strict bounded completion, existing cursor state is insufficient to establish all those distinctions with the proposed point predicate. Additive canonical state with bounded producer/reader maintenance may be warranted. Do not reuse `legacy_pending`, assign provenance retroactively, or fan out a row to every binding on send. A fresh schema and retention design would be necessary; no migration is proposed in this draft.

The parent must choose the initial-only scope/limits explicitly. This document does not grant permission to relax the original direct-address contract merely to pass one test. If the broader later-episode contract is required now, select option2 for further design rather than describing option1 as a universal fix.

## Required implementation evidence

These are prospective tests, not claimed results. Keep test assertions causal and exercise canonical/public routes; do not fabricate a recipient cursor or flip `legacy_pending` to make the hook pass.

1. **Predicate witness and actual hook:** retain the parent diagnostic RED. On the implementation, the unchanged public direct-send → registered actual AGY hook test emits invitation and exact Lazy body, preserves both full claims and actual native encoded bytes, and leaves membership/invitation/receipts/wake state unchanged. Physical confirmation alone leaves logical completion pending.
2. **Initial reader matrix:** binding predates pair; first binding enrolls after publication while initial invitation pending; two siblings; later binding while still pending after sibling display. Assert the explicit zero-baseline rule and that one sibling's completion never advances the other. After acceptance, a newly enrolled binding uses normal seat-max/head seeding.
3. **Acceptance:** message remains unread if accept occurs before any display; first body displayed but second remains unread; two siblings with different cursors remain distinct; initially unbound target is accepted by its first enrolled binding. Real accept records one participation episode and no receipt/display evidence. Ordinary thread/rejoin/episode2+ cursor-head initialization stays unchanged.
4. **Long-body public path:** real parsed `body` with enough UTF-8 content to require continuation before acceptance; prefix produces no cursor/ACK/complete; final valid text continuation moves only the intended binding after flush. JSON, machine, explicit nonzero offset, failed write/flush and wrong-bound/foreign IDs produce no new cursor. Preserve the known explicit-offset continuation limit honestly.
5. **Inbox/private continuation:** truncated initial direct Lazy inbox retains the exact ID/offset; next page succeeds before acceptance. Acceptance between chunks preserves progress. Sibling completion cannot hide the current binding's tail. Own full-body display while the hook holds partial/carry state triggers passive reader invalidation and correct next-hook retirement/revalidation without resetting another binding.
6. **Terminal controls:** real Reject leaves raw invited state but clears voluntary invitation; fresh reader sees no body. Left, retired, reinvited episode2+, ordinary nonpair invited, foreign pair endpoint and unrelated Lazy rows do not acquire a cursorless initial stream. Existing pre-leave cursor readers retain their addressed Lazy rows. Preserve binding Lazy legacy/left tests at `tests/store/binding_lazy.rs:313,348`.
7. **Display gap:** multiple addressed Lazy rows; displaying a later body cannot move over an earlier unshown body. Acceptance cannot turn that gap into displayed evidence. No new ACK actor/provenance/timer, membership interval, adoption or normal wake appears. `wake_on:all` retains its existing optional Lazy wake behavior.
8. **Plans/bounds:** actual EXPLAIN QUERY PLAN on large unrelated pair/membership/binding/message populations; 100 raw ineligible/eligible mixed candidates advance continuation and count rejected visits; lazy due13/reconciliation10 accounting; total1803/579/2560 caps unchanged; normal extra probes≤200. A continuation does not add a second fresh scan. No production/synthetic LIMIT test may claim physical boundedness for the inherited public prefiltered readers.
9. **Compatibility and scope:** no migration/wire/public capability changes, immutable historical command/replay bytes, no cursor rewind for already accepted historical pairs, frozen legacy_pending behavior, Main35 ordinary first-contact/seat-max/lineage and initial-hand-off ACK-required receipt coverage remain intact.

Suggested ownership stays narrow: `src/store/lazy_delivery.rs`, `read_cursors.rs`, `control.rs`, `queries.rs`, `invitation_content.rs`, internal `ports.rs` work accounting, normative TRUST/refinement text and focused existing store/CLI/hook modules. Any proposed change to enrollment, general receipt settlement, scheduler admission, schema or published wire is a scope decision, not a mechanical consequence.

## Post-Implementation Notes

No implementation or runtime validation performed by this author. Parent diagnostic outcome, selected scope/limits, implementation commit and actual focused verification remain to be recorded by the owning workflow.
