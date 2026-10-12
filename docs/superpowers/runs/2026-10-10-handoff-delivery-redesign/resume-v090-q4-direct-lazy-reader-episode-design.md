# Q4 refinement: direct Lazy readers across invitation episodes

Architecture proposal, 2026-10-11. Supersedes the initial-only recommendation in `resume-v090-q4-direct-lazy-reader-design.md`. No production authorization, implementation or verification claim. Source inspected at candidate `07fdf88da984869eb2908c25f9ff20caf253dc28`, tree `063aa3709e46e1ed2a21214f763c079d10e86785`. Parent subsequently reports current candidate `3dd448ef`; this author has not independently revalidated that tip. The WS `resume-v090-q4-spec-assessment.md` and `resume-v090-q4-quality-assessment.md` refinements are incorporated below. All source paths refer to nested `upstream-composition-v090`. Only this design document is written; no source/test, Cargo, Git, Herdr or runtime actions. Design roast remains OFF; counters unchanged.

## Decision

Use the **current canonical invitation episode** to admit newly addressed Lazy direct work for a cursorless binding. The necessary state already exists: immutable pair identity, exact current membership episode, invitation creation decision, and its immutable timeline event. No schema change, binding birth timestamp, version gate, send-time binding fanout or autoaccept is required.

The initial-only option is insufficient for the settled direct-send contract. The upstream automatic one-to-one design explicitly includes an invited direct target in the frozen Lazy audience without implying joining or receipt (`2026-10-10-automatic-one-to-one-threads-design.md:25,49`); this also applies after an explicit, legitimate reinvitation. The root handoff design still uses an ACK-required ordinary body; this repair must not convert handoff to Lazy or broaden ordinary invitation audiences.

The new rule has two distinct responsibilities: eligibility uses the invitation's **global creation decision**; cursor bootstrap uses the actual invitation event's **thread sequence**. These are different numbers. Existing cursor owners retain their established stream, including older unread work. Only cursorless readers use an episode baseline. Generic Main35 joined-thread enrollment and no-backlog semantics remain unchanged.

## Established failure

Parent's `/private/tmp/ht-C-v090-direct-lazy-eligibility-diagnostic-07fdf88d/execution.json` records the exact selected actual registered AGY test, exit 100 in 2.038 seconds, and scoped leak check exit 0. `run.log:24` records both pre/post canonical snapshots: exact connected binding; cursor absent; membership invited episode 1; invitation pending Lazy, creation decision 4; modern Lazy recipient pending with `legacy_pending=0`; exact manifest and ordinary Lazy message at thread sequence 3, publication decision 6. Reader gate is `past_cursor=1`, `has_cursor=0`, `joined=0`, `binding_lazy_unread=0` before and after. The actual native hook emits one invitation, zero bodies, 327 encoded bytes, `more=false`; physical confirmation changes no membership or body completion.

This proves a canonical reader exclusion, not an unpublished body, timing race, missing actual hook, or missing second page. Captured `lazy_recipients.state='pending'` is not the modern binding unread predicate. The latter comes from `lazy_delivery.rs:41–68`. Repeating an unchanged hook or accepting first is not the fix.

## Canonical episode authority

For an exact published Lazy recipient/message/binding tuple, define `active_direct_episode(thread, seat, instance)` from bounded point relations:

1. Existing UNIQUE membership `(thread_id,seat_id)` has `state='invited'` and `COALESCE(voluntary_state,state)='invited'`.
2. UNIQUE `direct_threads.thread_id` names this seat as one of its two endpoints and matches the instance.
3. Exact seat is in that instance and not retired.
4. UNIQUE invitation `(thread_id,seat_id,membership.episode)` is pending; return its ID, episode and `created_decision_seq`.

No episode number restriction. No search for the latest invitation. No scan of invitations, seats, pairs, intervals or bindings. The relevant keys are in `migrations/0001_initial.sql:273–296`, `0029_direct_threads.sql`, and `0032_session_bindings.sql`.

**Terminal-state proof:** do not test raw invited state alone. Rejection records a side-table row and restores voluntary state to absent/left, while raw invited can survive (`control.rs:1259–1273`). All legitimate native, operator and ordinary service invitation writers reuse an existing active pending invitation and create a new episode only otherwise (`control.rs:2127–2144,2275–2295`; `service_controls.rs:355–421`). Consequently, for an immutable pair, the exact current pending invitation and voluntary invited marker identify the active ordinary invitation. Required invitations need service ownership (`service_controls.rs:307`), and immutable pairs reject service ownership (`control.rs`, `guard_pair_service_owner`; `service_controls.rs:254`). The only cancellation insertion is required-episode release (`service_controls.rs:511`), which cannot canonically apply to a pair. Retirement is checked by seat directly because membership retirement can lag.

Thus rejection/cancellation history scans or extra side-table probes are unnecessary for this narrow pair predicate. This is a proof about canonical writers, not permission to ignore discovered legitimate counterexamples. Tests must cover every writer and terminal transition. If another writer invalidates this proof, add exact terminal point checks and redesign probe budgeting before implementation; do not silently rely on a stale voluntary marker.

The published row itself proves addressing. Ordinary Lazy thread sends do not capture invitees; explicit direct composition does. Validate the existing row/message/manifest/thread/instance/binding relationship, not body text, thread names or a reconstructed client marker.

## Reader rule

Factor a parameterized scope predicate taking validated message sequence and publication decision along with binding, seat, thread and instance. Its modern unread rule is:

```text
message.sequence > COALESCE(own_cursor, 0)
AND (
    own_cursor exists
    OR membership.state = joined
    OR (active_direct_episode exists
        AND message.publication_decision > invitation.created_decision_seq)
)
```

The legacy top-level `legacy_pending` branch remains unchanged. Existing cursor and joined branches retain their existing interpretation. Only the cursorless, unjoined branch adds this exception. A later-episode fresh reader excludes old published rows; a reader with a retained cursor keeps old unread work and must not skip it merely because a new invitation exists.

Publication is allocated in `messages.rs:833–865`; message and manifest share its decision and base sequence. Preparation captures membership/timeline revisions; publication verifies them again in the deciding transaction (`messages.rs:954–1013`). An old prepared audience cannot simply publish through an intervening reinvitation. Creation/publication ordering is canonical transactional order, not wall-clock inference.

Apply the same parameterized scope rule to all current macro consumers: fresh/private Lazy scan, public Lazy readers, due/owner replay, retained carry, completion-state queries and known-offer reconciliation. The separate Lazy continuation check at `queries.rs:6803–6821` must also use it, with the captured message's decision. Continuation uses scope eligibility, not the full sequence-unread comparison: another completion must not accidentally reinterpret an old offset as a new message or invalidate a captured tail solely because a cursor advanced. Retain exact identity, binding, publication cutoff and ordinal checks at `queries.rs:7135–7189`.

## Exact event-to-sequence mapping

All three legitimate pair invitation producers append an immutable source-invitation timeline event in the same transaction:

| Producer | UNIQUE `messages.event_key` | Source |
| --- | --- | --- |
| Initial Ensure / native Invite | `invite:<invitation-id>` | `control.rs:735–745,2177–2187` |
| Operator orphan Invite | `operator_invite:<invitation-id>` | `control.rs:2322–2334` |
| Ordinary ServiceInvite | `service_invite:<invitation-id>:ordinary` | `service_controls.rs:452–474` |

`schema.rs:3590–3758` shows that ordinary append-event allocates its own decision and consumes the actual next thread sequence. **Never equate the invitation creation decision to the event decision or add one heuristically.** The diagnostic's invitation decision 4 and body decision 6 illustrate the distinction; it does not independently record the intervening event row.

The bootstrap helper performs at most three exact event-key probes, validates each returned row's instance, thread, kind and `source_invitation_id`, and requires its decision to follow invitation creation. Select the smallest sequence among the valid results in memory. This fixed-size minimum is not an unbounded SQL aggregate. Current Ordinary ServiceInvite returns early when reusing an active invitation (`service_controls.rs:374–383`), before appending any event; it does not create a later event in that case. All three keys are needed because each producer can create the invitation itself. Earliest-of-three conservatively handles multiple recognized retained event rows; that robustness fixture is distinct from a public duplicate-invitation test.

No matching event, or a conflicting identity for an exact key, is a canonical integrity error; do not fall back to zero, thread head, a timestamp, an unindexed `source_invitation_id` scan, or a guessed decision-to-sequence conversion. The existing immutable event key and FK are sufficient without a migration. Add regression coverage proving duplicate service invocation after a native invitation and body publication returns the original invitation without moving its floor.

Call the selected thread sequence `episode_floor`. It excludes timeline history at or before the invitation event, while every legitimately published body for that new invitation follows it. Event probes are needed only when initializing a cursor, never in each hook candidate predicate.

## Cursor lifecycle and acceptance

| Reader state | Baseline and behavior |
| --- | --- |
| Existing own cursor, any later episode | Keep that exact cursor. Its old unread addressed prefix remains authoritative. |
| No cursor, active direct invitation episode N | Read-only eligibility admits only publications after this invitation's creation decision. Do not create a cursor on peek or hook rendering. |
| New binding while that invitation is pending | Same episode rule. Its independent stream may include work a sibling displayed; no binding-birth heuristic or sibling settlement hides it. |
| Reject/left/retired, no cursor | No new automatic direct Lazy stream. A fresh binding cannot reopen old work. |
| First or later direct acceptance | Preserve existing cursors; initialize missing open-binding cursors at this exact invitation's `episode_floor`, not the head. |
| New binding after acceptance | Existing Main35 seat-maximum/head seeding applies. |
| Generic thread join/rejoin/accept | Existing `init_for_join` head policy applies. |

Acceptance must capture the selected active direct invitation and its validated event floor **before** changing membership, in the same deciding transaction. Pass an explicit internal initialization policy to `read_cursors::init_for_join` or a dedicated helper. `control.rs:1090–1118` currently marks accepted, updates membership and calls the head-seeding helper; `read_cursors.rs:136–152` raises all open cursors to head. Replace this only for the exact active pair invitation acceptance: `INSERT ... episode_floor ... ON CONFLICT DO NOTHING`. Never `MAX(existing, episode_floor)`: an existing older cursor is an established stream, not a missing baseline.

This reuses the current acceptance fanout over open bindings, including disconnected open bindings and excluding ended ones. It adds no new fanout at Ensure, send, publication, enrollment or hook read. Preserve canonical join intervals and actor provenance. Acceptance initializes unread scope; it records no body display or ACK.

**Required same-transaction reader invalidation:** after the direct acceptance membership/cursor changes, call `presentation_reader::note_binding_reader_change` exactly once for **each affected open binding in this existing acceptance fanout**, including bindings whose existing cursors were preserved unchanged. Here the reader change is the canonical invited-to-joined eligibility transition, not necessarily a cursor-value change. Settled ordinary direct messages can become newly joined-readable even when the cursor is unchanged. An invitation/membership warning trigger or a bounded attention view with zero ordinary work does not make a caught-up private `ordinary_done` reader rescan; neither substitutes for this explicit invalidation. General Join and unrelated acceptance behavior remain unchanged.

Reuse the existing helper's exact canonical binding/seat scope and its two fixed Hook/Watch consumer points (`presentation_reader.rs:59–147`). It checks the current binding's generation/execution against the consumer or Mod scope and updates only matching existing consumer rows. It must not enumerate consumer history, select a sibling by seat, manufacture absent consumer rows, or broaden current-session authority. The helper increments only `ordinary_passive_generation` and `mod_version`; preserve carry, output token, retained partial body, parked progress and all other presentation payload/state. Subsequent ordinary revalidation uses the existing bounded reader machinery rather than destructive resets.

Invitation acceptance, membership/interval changes, missing-cursor seeds, and all per-binding private invalidations belong to one deciding transaction. Checked counter overflow, invalid counter types, canonical scope errors, cursor errors or any later failure propagate and roll back **all** these writes, including private increments already applied for earlier bindings. Accept replay returns its stored result with no reseeding or repeated invalidation. Count this additional work explicitly as described below; it is not hidden in presentation caps.

Already-joined accept replays do not reseed; rejected/terminal selections do not qualify. Direct creator rejoin through plain Join and all generic Join paths keep their established head policy. An explicit invitation to either immutable endpoint, including the original creator, uses the same episode rule when that endpoint is unjoined.

**Mixed delivery modes:** the cursor is shared by ordinary and Lazy messages. Preserving an episode-floor cursor on direct acceptance can expose already-settled addressed ordinary direct messages above that floor through the established joined-reader stream, as well as Lazy bodies. Pending ACK-required direct messages already have their independent obligation source. This is an explicit consequence of the narrow direct-accept preservation policy, not an assertion that every ordinary direct acceptance history case stays unchanged. Document and test it; unrelated ordinary nonpair joins remain at head. Suppressing only the ordinary part while preserving Lazy would need a different cursor model and is not proposed.

## Actual full display and mutation seams

The cursor walk (`read_cursors.rs:255–305`) presently treats a missing cursor as zero. In episode 2+, an undisplayed older Lazy row can block advancement for a freshly displayed body that was eligible only in the current episode. Therefore a predicate-only or acceptance-only fix is incomplete.

Add one shared cursor-baseline policy at the completion walk boundary, used by **both** `AdvanceCursors` and `CompleteInboxDelivery`/`complete_displayed_for_binding`. Resolve the exact binding's existing cursor first. If it exists, keep it unchanged as the starting point. If missing and the supplied completed IDs include at least one exact published, addressed Lazy pair body eligible under the current active invitation, resolve `episode_floor` and walk from that **virtual** baseline. Persist only resulting actual progress beyond the virtual floor; do not perform an intermediate cursor insert. Do not bootstrap because an arbitrary system/unaddressed/foreign message was supplied. Filter new-path admission per ID before constructing display sequence vectors.

The new floor is reader scope, not evidence that older bodies were displayed. It skips only pre-episode history that the new reader was never eligible to receive automatically. The walk must still stop at the earliest unshown receivable row **after** the floor. Displaying the second new body cannot skip the first. Preserve receipt ownership checks and canonical prepared-recipient addressing. If a same-episode gap prevents all progress beyond the virtual floor, leave the cursor absent and report no change. Coalesce `presentation_reader::note_binding_reader_change` once for actual persisted progress by the binding. No sibling cursor moves. Acceptance separately performs its explicit missing-cursor initialization.

`AdvanceCursors` currently ignores unjoined threads (`control.rs:2596–2664`). Its additional per-message admission is exact published addressed Lazy pair work with either an existing own cursor or the active episode condition. The existing joined branch remains unchanged. `CompleteInboxDelivery` already validates Lazy addressing and matches the canonical binding (`control.rs:2501–2594`) but its helper calls the same zero-start walk (`lazy_delivery.rs:475–559`); apply the same baseline there. Do not use historical seat-level `state='displayed'` as modern reader completion. Preserve legacy bookkeeping and notification touches.

If rejection wins before a cursorless display mutation, the new current-episode baseline is unavailable. Existing cursor owners retain their old stream. If acceptance wins first, its preserved/seeded cursor supplies the baseline. If a later invitation wins before a delayed earlier-body completion, that older message cannot borrow the newer episode floor; exact message publication eligibility matters. Preserve existing explicit historical published-ID completion semantics: where CompleteInboxDelivery already admits that display, its existing missing-cursor zero baseline/gap walk remains available, rather than silently refusing or relabeling the operation. AdvanceCursors retains its prior refusal for an unjoined historical ID that fails the new admission rule. The new policy grants a virtual episode floor only for exact currently eligible displayed content; it does not globally narrow existing completion authority.

The public inspection command is `body MESSAGE` (`Command::Message(MessageQuery)`), not a production symbol named PeekLongBody. The query reads without acceptance (`queries.rs:2609–2699`). Full successful text output flush precedes `AdvanceCursors` (`cli/mod.rs:1175–1189,1320–1368`; `output.rs:276–306`). JSON/machine reads, partial explicit-offset output and failed writes/flushes create no cursor. Preserve the already documented limitation that a native continuation begun from an explicit offset may be treated whole; do not claim a stronger full-prefix guarantee.

Own text inbox's contiguous chunk journal (`cli/lazy_display.rs:26–123`) emits completion only for complete flushed chains; `cli/mod.rs:1722–1777` advances the binding and then performs permitted Lazy completion. Truncated hook presentation and physical `DeliveryPresentationConfirmV2` remain output bookkeeping, with no ACK, acceptance or cursor advancement. Existing logical display/transcript evidence retains its separate semantics.

## Bounded query accounting

The shared predicate adds **three** conservative unique relation probes to an existing evaluation: exact pair, seat, and current invitation. It reuses the existing membership probe. Count all three even when SQLite short-circuits. No event lookup occurs here. The following accounting preserves the current 1803/579/2560 caps and 100 candidate limit:

| Path | Proposed charged bound |
| --- | --- |
| Exact Lazy due/replay | 1 mode + (7 existing eligibility + 3 new) + 3 hydration = **14** |
| Other ordinary due/replay | Existing 13 pending / 18 settled maximum unchanged |
| Due total | 3 setup + 100×18 = **1803**, unchanged; 100 seeks, 32 hydrations |
| Carry total | 3 + 32×18 = **579**, unchanged |
| Known Lazy reconciliation | **10** per key after canonical message-row reuse below; 256×10 = **2560** |
| Fresh private Lazy scan / Lazy continuation | At most **300 additional unique probes** over at most 100 physically visited candidates |
| Cursorless bootstrap / acceptance floor | Fixed three event-key probes, plus fixed exact episode authority lookups; outside presentation accounting |
| Direct acceptance reader invalidation for N affected open bindings | **N helper calls** within existing acceptance fanout: at most **4N indexed relation reads** (binding + seat + Hook + Watch) and **2N exact consumer updates**, in addition to existing cursor writes; no history scan |

**Required reconciliation refactor:** `invitation_content.rs:1793–1810` already performs an exact message-mode lookup, then repeats a message join inside its seven-relation Lazy eligibility query. Extend the first row to carry canonical thread, sequence and publication decision, validate it once, and pass those scalar values into the shared predicate. The second query then needs six original probes: Lazy recipient, manifest, binding, two cursor points and membership. Add pair, seat and invitation = nine; the first message lookup makes ten. Manifest/message/thread/instance identity comparisons remain explicit against the first canonical row. Reuse happens within the same read snapshot; a client-provided scalar is not an acceptable substitute. This removes an actual redundant relation access, not just a counter increment.

**Implementation/test seam:** expose the actual first-message and second Lazy-reconciliation SQL constants to the relevant store tests at crate-internal visibility, and make the production shared scope predicate scalarized rather than relying on a mandatory `m` table alias. Use explicit named scalar inputs for message ID, instance, thread, sequence and publication decision, plus seat and binding. The first statement keeps exact ID/instance/ordinary-kind selection; the second binds those results and verifies `r.message_id`, `r.thread_id`, `sm.preparation_id`, `sm.message_id`, `sm.thread_id`, `sm.instance_id`, `sm.decision_seq`, and binding-seat agreement. Missing/inconsistent authority retains known capacity rather than declaring the offer terminal. Sequence comes from the first canonical message point, not the manifest or caller. Current `insert_publication` uses the same `base` for body sequence and manifest base_sequence (`messages.rs:859–866`), and the Lazy manifest trigger requires zero warnings; nevertheless this optimization does not need a new `sm.base_sequence = captured_sequence` admission condition or an unsupported historical shape assumption. Exact canonical message identity and matching publication decision supply the needed relation. Query-plan/counter controls must execute these production constants and predicate, not copied illustrative SQL. No hidden second message lookup or fabricated alias join is permitted.

Keep the ordinary reconciliation branch's current maximum ten. Verify the physical SQL with EXPLAIN before claiming the cap proof. If the selected implementation cannot remove the redundant relation access, it must use an explicitly bounded reconciliation key budget with resumable progress and charge eleven; it must not raise the cap or report eleven as ten. Such a batching fallback needs a separate progress design and is not the preferred implementation.

Add an internal fresh-source scope-probe counter, charged +3 each time a normal/private Lazy scope check runs, including continuation. Enforce `<=3 * physically_visited_candidates` and `<=300`; carry/due are charged in their own counters, not twice. Rejected candidates count before filtering. Do not rerun SQL during encoded-size binary search. Output sizes, 32-item page, ordinary continuation's existing 15-probe rule, and one-source-query-per-quantum behavior remain unchanged. Update all comments, tests and normative arithmetic to match physical access.

The event floor helper does at most three point reads regardless of history size. Direct acceptance already visits N open bindings; the added invalidation is a fixed envelope per binding, not a claim that the whole acceptance is constant work. Each helper performs three read statements touching four unique relations and up to two point updates. The two consumer points are charged even when absent or stale and require no retained-history scan. This extra mutation work does not consume or change the private presentation 1803/579/2560 caps. No new fanout occurs at publication or reading. The completion walk itself remains an inherited unbounded sequential walk through messages after its starting sequence; this design does not claim a new end-to-end mutation bound. Likewise public Lazy readers place eligibility before LIMIT and can inspect many physical candidates (`lazy_delivery.rs:73–83,141–160,360–453`), and seat-level unread can enumerate existing connected bindings. No new fanout or unbounded join is added, but existing cost is not magically bounded by a visible-row limit. Strictly bounding those inherited paths would require a separate progress/state design.

## Scope, limits and required documentation

Update TRUST's Main35 reader/no-backlog wording (`TRUST-POLICY.md:795–803`) in the same implementation change: cursorless active direct invitees start at their canonical current invitation boundary; accepting preserves that newly admitted unread work. Preserve A8 immutable addressing and the distinction between addressing, presentation, display and participation (`:736–754`). Update the accepted direct-composition refinement, not merely the failing test.

The rule intentionally allows a newly created binding during a pending episode to see that episode's addressed work even if a sibling displayed it. Preventing that replay would require enrollment-time baseline state/work and is outside the no-fanout proposal. Existing cursors retain older work across leave/reinvite; current-episode exclusion only applies where no reader cursor exists. Existing explicit joined-binding lifecycle fast-forward remains a documented Lazy limit. Historical bodies already skipped by prior head initialization are not safely recoverable by cursor rewind: existing rows do not distinguish legitimate display/fast-forward from the old bug.

No new canonical state is needed for **fresh episode-2 delivery and preservation through its acceptance**. New state would be needed for stronger distinctions such as immutable enrollment-time replay exclusion or safe repair of already-skipped historical cursors. Do not promise either under this proposal.

## Prospective verification

These are required future checks, not results from this design lane.

1. Keep the exact actual AGY RED predicate witness. GREEN must emit invitation plus exact Lazy body before acceptance, preserve claims/bytes/mode and leave invitation, cursor, receipts and wakes untouched after physical confirmation.
2. Exercise episode 1 and episode 2+ through canonical commands. Episode 2 has an old undisplayed addressed Lazy body, no own cursor, a legitimate reinvitation and new body: fresh hook includes only the new eligible body; full display advances from the episode floor; accept-before-display preserves the new body.
3. Repeat with a retained old cursor: old unread prefix remains visible and blocks skipping to later display. No `MAX(cursor,floor)`. Sibling cursors remain independent. If an earlier new body blocks progress from a virtual floor, verify the cursor remains absent and no invalidation is fabricated. Test mixed ordinary/Lazy direct content across acceptance and unchanged ordinary nonpair head initialization.
4. Exercise native, operator orphan, ordinary service invitation and duplicate service invitation after a native invite plus published body. The duplicate returns the original invitation and creates no later event. Separately test earliest fixed-key selection with a deliberately constructed multiple-event robustness fixture; absence/conflicting identity errors rather than guessing. Prove required pair invitations/ownership are rejected and terminal voluntary markers exclude rejected cursorless readers.
5. Enroll bindings before invitation, after publication while pending, after a sibling displayed while pending, disconnected-open before accept, ended before accept, and new after accept. Verify the exact table above and unchanged generic seat-max/head/lineage/no-backlog tests.
6. Cover long UTF-8 body through public text body and own inbox continuation before acceptance, acceptance between chunks, rejection/reinvitation before delayed completion, concurrent sibling display, JSON/partial/failed flush, and wrong-addressed IDs. Both AdvanceCursors and CompleteInboxDelivery must use the same baseline; no prefix-only cursor advance or autoaccept.
7. Verify preparation invalidation across membership/timeline changes. Retained pre-episode publication cannot enter the new cursorless episode by replaying a frozen request.
8. Inspect actual query plans with many unrelated pairs, episodes, messages and bindings. Prove exact-key access and no history scan in the new predicate/floor. Exercise 100 rejected/mixed raw candidates, due Lazy14, reconciliation Lazy10, continuation scope probes and unchanged 1803/579/2560 limits. Confirm canonical scalar reuse preserves all exact identity checks.
9. Preserve legacy_pending, generic left/no-cursor tests, ordinary ACK-required handoff delivery and immutable replay bytes. No new receipt provenance, adoption, lifecycle timestamp inference, binding fanout, schema, wire shape or version gate.
10. **Accept-only actual reader control:** prime both Hook and Watch private consumers to quiet/caught-up `ordinary_done` before acceptance, with mixed current-episode Lazy and already-settled addressed ordinary direct content. Include one binding with a retained cursor that acceptance must leave unchanged and a cursorless sibling that receives the episode seed. Perform acceptance as the only intervening semantic change: no new send, body completion, artificial consumer reset or extra publication. Drain actual framed Hook/Watch output through existing bounded progress and prove the now-eligible ordinary body and still-unread Lazy work are delivered for the correct reader. Confirm one checked private generation/version increment per matching consumer per affected open binding, absent/stale consumers remain untouched, and carry/token/partial/parked payloads survive the acceptance transaction. Test disconnected open and ended bindings, idempotent accept replay, and overflow/error in a later binding after an earlier tentative update; rollback must restore invitation, membership/intervals, every cursor and every private row atomically.

Implementation can remain within the store reader/predicate, completion/acceptance policy, private query/reconciliation accounting, normative documents and focused tests. This is a coherent design candidate for the parent's next step, not evidence that source is fixed.
