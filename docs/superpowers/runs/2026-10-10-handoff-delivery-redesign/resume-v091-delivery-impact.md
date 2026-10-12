# v0.9.1 base drift: delivery, reader identity, and wake composition

Read-only source investigation. No source, Git, tracker, Cargo, Herdr, or runtime mutation; only this report was written. No test result is claimed. All code observations use immutable Git objects, not the worktree's midmerge files.

- Previous main: `565def869ae50e47e6da7388ea333229050eaf8d` (P).
- New main: `20e7ca73e41a9449376169ab6875c0675da02edc` (M).
- Feature: `0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2` (F), `super-auto/sweep-validation`.
- Source references below use `M:path:line` or `F:path:line`; obtain their exact contents with `git show SHA:path`. They do not refer to unstable current line numbers.

## Disposition

Absorb main's independent wake fixes, Codex turn-start delivery and compatibility surface. Preserve the feature's approved binding-owned V2 presentation and current-direct-episode reader semantics. Two behavioral adaptations are necessary even if conflict resolution compiles:

1. Main's new in-turn invitation wake accounting must inspect **pending confirming invitations**, not every discoverable pending invitation. Both the candidate calculation and the final per-composer-binding recheck need the feature's timed source.
2. Main's new generic join frontier must not replace the feature's exact direct-invitation acceptance policy. Direct acceptance preserves existing cursors and seeds missing cursors at the invitation event floor; it must not convert that stream into seat-wide legacy-pending mail or move it to the thread head.

V2 physical presentation confirmation is not a new source of `AdvanceCursors`, receipt ACK, invitation acceptance, or main's standard-hook in-turn records. Keep those evidence paths separate. This is a bounded composition recipe within the existing authorized design, not a request for a new delivery design or a reason to pause parent absorption.

## Contract read and precedence

Read the complete F root design, including confirmed roast refinements and its appended authorized upstream, AGY, Lazy-positive-delivery, aggregate-envelope, and final framing/relevance refinements:
`docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md`.
Also inspected `author-refinements.md`, the selected framing proposal, the full current-episode Q4 design and its private Lazy continuation-retirement supplement, and normative TRUST-POLICY sections covering continuity/attribution, A6/A7/A8, Q4 readers, and bounded V2 presentation. Historical proposal headers do not supersede the subsequently implemented normative F policy.

The combined result must retain:

- A2 canonical binding decisions, A4 exact lifecycle and liveness rules, and A5 role-based authority. The new wake evidence must not enroll/rebind/merge a seat or authorize a caller from a client hint.
- Send-time receipt deadlines and independently timed invitation acceptance; display, registration and launch do not settle either. Lazy invitation is positively discoverable in actual output but not timed/native attention.
- A7 independent still-open origin warnings for every exact original sender/inviter, including departed origins, after informational notice settlement. Final clear remains passive.
- A8 independent binding reader positions, Q4 current-episode admission before direct acceptance, preserved direct-accept cursors, and separate actual full-display completion.
- V2 exact binding/fixed-consumer/token/codec controls, explicit Live Watch lease/mode election, bounded replay/carry/discovery, and physical confirmation that changes neither cursor nor receipt.

Main's policy adds narrow standard-hook in-turn evidence and a generic join-frontier rule. Its blanket new policy language “every transition ... into Joined” and “first ... displays ... sibling ... gets no copy” is incompatible with the approved Q4 direct-accept exception. Merge policy semantically: state the direct-accept exception explicitly, retain generic head policy and its documented seat-pending limits, and do not leave two contradictory universal claims in TRUST.

## Overlapping functions and invariants

| Seam | Main change | Feature responsibility | Disposition |
|---|---|---|---|
| `store::wake::load_candidate`, `reason_labels`, `reserve` | Adds `receipts_served_in_turn`, `invitations_served_in_turn`, all-reachable-binding coverage, and reservation comparison | Origin warnings remain waking after notice frontiers; invitation attention is timed-only | Behavioral: retain both, use timed invitation population; never blanket-mask warnings with served flags |
| `wake::wake_owed_to_binding`, `all_offered`, `served_by_all`; `NativeWakeDispatcher` | Rechecks chosen composer binding after reservation validation; refuses already-served pending receipt/invitation-only wakes | Canonical scope/foreground selection and active-turn guards remain authoritative | Import intact topology/fences, adapt only invitation source; do not use newest-seat binding when composer selected a binding |
| `control::advance_cursors` | Top-level `in_turn` records pending receipt IDs and pending invitation IDs in one transaction | Q4 per-ID direct Lazy admission, virtual-floor cursor walk, coalesced `presentation_reader` invalidation | Behavioral merge in the same deciding transaction; retain Q4 admission and invalidation, append wake-only evidence independently of whether any cursor moved |
| `control::accept`, `read_cursors::init_for_join` | Generic join marks past-seat-maximum undisplayed Lazy rows `legacy_pending=1`, then advances all open binding cursors to head | Direct accept captures exact active invitation floor before state transition and calls `init_for_direct_accept` | Keep feature's direct branch and helper; main generic rule may remain only on the generic head-policy route |
| `lazy_delivery::binding_lazy_scope_sql!`, `binding_lazy_unread_sql!`, completion helpers | Main comment/storage interpretation now acknowledges the join-set legacy flag and historical `state` check | F adds cursor-or-joined-or-active-direct-episode scope; `state=displayed` is not modern binding unread authority | Preserve F predicates and exact canonical publication validation; qualify main comments to generic join; no new direct legacy flag assignment |
| `cli::hook::run_admitted_hook_since`; `hook_inbox::{read_page,append_presented,LazyCommit::commit}` | Gated capability, full shown invitation IDs, `in_turn` only for Tool/current mode, commit after output flush | Stable per-event hooks and separate adapter-owned V2 hook-pull path | Mostly mechanical after preserving routing/codec ownership; do not route V2 confirmation into standard-hook completion |
| `seats::{mark_seat_unresolved,rebind_unresolved_seat,register_available}`, `control::begin_retirement_fence` | Clears in-turn rows on mapping lifecycle; lifecycle check-in clears only own binding | F V2 durable rows and logical ledger have their own binding/consumer lifecycle; same-session resume keeps logical offered set | Add main cleanup only for `in_turn_offer`; never clear sibling/V2 rows or logical offer history by analogy |
| `ports::{WakeCandidate,StorePort}`, `service::workers::ScheduledStore`, `store::mod` | Wires owed-wake method/fields | Existing capable owner/replay interfaces | Mechanical propagation; retain real production delegate rather than relying on trait default `true` |
| `protocol::{AdvanceCursors,capabilities}`, call sites | Optional default-false/empty fields, validation, `inbox.in_turn_offer_v1` | Existing historical request digests and V2 capabilities | Additively compose; old serialization omits new fields; ordinary body/inbox/subagent producers default false/empty |
| `service::kicks`, `notification::DispatchState::observe`, `service::pacer::Backoff` | In-turn writes kick wake; advancing frontier retries immediately without resetting refusal count | F completion recheck hints and private presentation changes are passive | Retain independent main fixes; do not make passive V2 progress or Lazy invitations new attention |
| `harness::codex::{CONTRACT,DECLARATION,check_hooks_v1,parse_shape,decode,encode_event_context}` | Adds UserPromptSubmit as Current/Tool context and preserves native event name | Stable `hook-event` entrypoints and owned per-event setup | Import adapter-owned turn event; verify its stable generated command and exact output event name |
| Schema migration, audit, tests | Released `0036_in_turn_offer.sql` | F has unlanded redesign migrations occupying later numbers | Separate mechanical/schema lane must preserve released 1..36 bytes and migrate genuine M36 populated stores; do not edit released SQL to fit F numbering |

Other overlapping paths include direct-send JSON output, CLI/help/goldens and capability fixtures. They are not a reason to replace the feature's actual retained Ensure policy, send-time preflight, exact historical retry behavior, or binding-owned output with main's earlier assumptions.

## Concrete failure 1: all-pending invitation accounting reintroduces a wake

F `store/attention.rs:692–722` deliberately separates `pending_invitations` (discoverable) from `pending_confirming_invitations` (NULL/historical or RequireAck). The latter puts its confirmation predicate **before LIMIT**, so a large Lazy backlog cannot hide an older timed invitation. `wake_seat_attention`, F lines 1739–1768, correctly uses that timed source.

M `store/wake.rs` adds calls to `pending_invitations` in two places: `load_candidate`'s `invitations_served_in_turn` and `wake_owed_to_binding`'s final invitation branch. In M every ordinary invitation was timed; the calls therefore matched its wake population. They no longer match F.

Minimal counterexample: one binding has timed invitation T and Lazy invitation L. It was fully shown T inside its turn; L remains unseen. F attention correctly has the pending timed reason T. The unadapted main coverage check tests `{T,L}`, finds L unserved and keeps the reason. Final dispatch repeats that same wrong set and types a wake, despite the only timed invitation being served. A saturated Lazy backlog is the same error with larger cost/false conservative coverage.

Recipe: use `pending_confirming_invitations` for both new wake coverage calls, retaining the same decision-sequence snapshot, bounded source and saturation behavior. Keep `pending_invitations` for actual discoverability/output. Do not filter a limited all-pending set afterward, remove Lazy invitations from output, manufacture confirmations for L, or suppress all invitation attention. The new record writer may record a still-pending Lazy invitation harmlessly; it gains no timed wake authority merely because a row exists.

Also retain F `wake::load_candidate:205–212`: open origin warnings invalidate warning-offer suppression even when their informational page was carried. M final `wake_owed_to_binding` returns true for reasons beyond pending receipt/invitation; that correctly keeps origin-warning backstops independent. Do not simplify it into “all displayed => no wake.”

## Concrete failure 2: accepting a pair destroys a sibling's stream

M `read_cursors::init_for_join:138+` marks pending Lazy messages above the **maximum cursor of any seat binding**, then writes the thread head into every open binding. The first top-level display clears the seat-wide legacy flag. M TRUST expressly accepts sibling loss/rejoin skipping, and M `pair_rejoin_with_two_bindings_pins_read_positions` asserts it.

F intentionally repaired a different, stronger reader contract:

- `lazy_delivery.rs:47–102` admits a cursorless invited pair reader only after the exact current invitation creation decision; own cursors retain their earlier stream.
- `control.rs:1090–1137` captures the exact floor before acceptance, then calls the direct helper.
- `read_cursors.rs:155–177` uses `ON CONFLICT DO NOTHING`, retains existing cursors, and invalidates each affected open binding exactly once.
- TRUST A8 explicitly states that joined ordinary messages already settled by a sibling can consequently become readable above the retained floor; this mixed ordinary/Lazy effect is intentional.

Calling main's generic helper instead makes acceptance look like display. A sibling with lower cursor loses both its ordinary settled-but-unread stream and some Lazy work; converting the body to a seat-wide legacy flag cannot restore independent binding consumption. Copying main's global `state='displayed'` condition into modern direct unread also hides a body from an unread sibling.

Recipe: preserve the exact active-direct-accept branch and its transactional reader invalidation. Absorb main's generic `init_for_join` logic on the existing non-direct/head-policy branch (including ordinary explicit Join where F already preserves generic head policy), document this scope, and preserve modern direct predicates/continuations. Do not rewind historical cursors, infer a baseline from time or binding birth, add a second cursor model, or reset private captures. Those are outside this composition.

The two new M pair acceptance/rejoin tests contain assertions for the superseded direct-accept policy. Port their real producer fixtures and useful “no loss/whole output” checks, but change incompatible expectations to the approved Q4 independent-reader outcomes, with explicit review rationale. Do not blindly delete the tests or force F to satisfy their old global-head/sibling-suppression assertions.

## Evidence types that must remain distinct

1. **Standard-hook full flushed context:** main `hook_inbox` sends `AdvanceCursors { in_turn:true, displayed, invitations }` only after context output was successfully emitted. The flag is gated by the additive capability and current/tool event. A lifecycle callback may leave the session idle and therefore records no in-turn service. This record only suppresses an otherwise owed idle wake for the exact binding; it is not ACK/acceptance.
2. **Actual reader movement:** F `control::advance_cursors:2645–2699` admits exact direct Lazy IDs and coalesces invalidation only when progress changes. Main in-turn recording cannot depend on `reader_changed`: an invited ordinary receipt or invitation may have been shown with no cursor movement. Conversely an ordinary non-in-turn cursor advance supplies no wake evidence. Subagent advances remain reader-only; actual canonical role gates the new record writer.
3. **V2 physical confirmation:** F `invitation_content::confirm:859–984` validates exact binding/consumer/token/control stamps and retires nonordinary outbox slots. It does not know that a standard native event was inside a turn. Its ordinary message IDs do not live in the nonordinary outbox. F `delivery_presentation::presentation_confirm:327–395` adds only narrowly scoped fresh Watch Attention health, with an exact Live lease. Do not append in-turn rows or invoke cursor/receipt handlers here.
4. **Logical hook/mod receipt or Lazy completion:** existing transcript/context evidence follows its declared provenance and offered ledger. Main does not change this authority. F replay explicitly retains full Offer stamps and native outlook (`delivery_presentation.rs:286–302`); do not treat owner replay as a new logical retry or native wake grant.
5. **Lifecycle reset:** M resets this binding's in-turn record on lifecycle check-in; F same-session resume preserves the logical offered ledger and binding-owned private presentation state according to their own scope. Resetting one is not authority to reset the other.

No additional adversarial caller verification, provenance kind, version gate, global presentation fanout, or timeout is necessary.

## Minimal targeted causal controls on the combined candidate

These are required prospective checks, not runtime results from this lane. Reuse existing fixtures and actual public producers; do not seed a cursor to bypass the behavior under test.

1. **Timed + Lazy invitation interaction:** one timed invitation shown in-turn plus an unshown Lazy invitation; verify both candidate and final pre-send binding check suppress the timed-only wake, leave both invitations pending, retain their policies/deadlines, and keep Lazy output discoverable. Then add a new unshown timed invitation and prove wake is owed. Include a Lazy backlog larger than the invitation window to prove the timed filter runs before LIMIT. NULL historical confirmation stays timed.
2. **Chosen binding and post-reservation race:** retain M `in_turn_service_is_decided_for_the_binding_the_composer_reaches`, `an_in_turn_offer_after_reservation_cancels_that_bindings_wake`, and recovery-hint coverage. A served foreground binding must get no keys while an unserved sibling keeps an owed wake. Reverse foreground ownership and obtain the positive wake. No ladder escalation on refused served output.
3. **Origin warning independence:** timed invitation/receipt was shown in-turn, deadline still passes, origin has left and its informational notice frontier is already settled. The canonical original inviter/sender still has waking open warning attention; debtor display does not settle it. Late accept/ACK yields one passive clear. Reuse the feature's multi-origin controls rather than inventing new origin inference.
4. **Actual Codex producer:** exercise `hook-event codex UserPromptSubmit` with proper native payload and a pending body/invitation. Assert exact `hookEventName`, body in output, postflush in-turn records and no later duplicate native wake. SessionStart/non-in-turn output, child event, failed write/flush and a daemon lacking IN_TURN_OFFER are negative controls. Verify equivalent setup emits stable per-event bytes and preserves foreign/drifted groups.
5. **Direct accept independent readers:** retain F `q4_initial_accept_preserves_each_bindings_unread_body_and_display_gap`, `q4_direct_lazy_completion_is_independent_for_two_initial_bindings`, and episode-two tests in `tests/store/direct_lazy_readers.rs`. Mixed opening ordinary/Lazy messages must survive for each eligible own reader; acceptance preserves existing cursor and seeds missing at exact event floor, not head. Generic nonpair join retains main's frontier behavior. Confirm direct acceptance does not set modern rows' legacy flag as a substitute for reader identity.
6. **Accept-only private invalidation:** F `presentation_q4_accept_only_reopens_quiet_watch_and_hook_with_preserved_reader` primes actual consumers quiet before acceptance, then uses acceptance as the sole semantic change. Retain fanout/rollback/carry/full-offer-window controls in `q4_accept_controls.rs`; counter overflow rolls back invitation, membership, cursor and private updates together.
7. **Shared AdvanceCursors seam:** an exact current-episode Lazy full display advances only that binding and invalidates its matching consumers once; an in-turn ordinary pending receipt can record wake evidence without creating a cursor. Wrong-addressed/old-episode/partial output cannot borrow a new invitation floor. A subagent/non-in-turn advance writes no in-turn records. Historical request bytes still omit false/empty additions and replay unchanged.
8. **V2 no new evidence:** physical Hook/Watch confirmation, owner replay, refused/stale/foreign token and replaced/grace Watch owner create no in-turn rows, reader movement, receipt ACK or acceptance. Keep actual preaccept body/failed-output tests in `q4_body_controls.rs`, and the exact private modern-tail retirement/public refusal/legacy-negative/rollback tests in `q4_retirement_refusal_controls.rs`. Main's generic legacy marker must not make a modern direct episode silently bypass Q4 retirement eligibility.
9. **Refusal backoff:** retain M `new_attention_retries_a_refusal_backed_off_seat_at_once_and_keeps_the_backoff`; genuinely new timed/receipt/origin attention becomes immediately retryable, while passive Lazy invitation/context/replay progress does not become a new native frontier.
10. **Upgrade/lifecycle:** genuine populated M36 -> combined migration/reopen preserves released in-turn rows and binding IDs; ordinary Current check-in keeps them, own lifecycle resets only its rows, unresolve/rebind/retire clears appropriately. The migration/schema owner should execute rollback and exact released-SQL identity controls alongside the feature migration tests.

Changes in these seams require targeted source-level and actual producer tests plus the required combined fmt, clippy, harness-neutral, and premerge default-feature gates. Parent retains final PR roast and subsequent combined exact-tip verification; neither old-main evidence nor feature-only evidence establishes the new combined result. Do not run the routine full suite merely to substitute for these causal controls; the project restriction remains in force.

## Scope limits

This investigation does not claim the current midmerge source is broken or repaired; it identifies conflicts from immutable P/M/F behavior. It does not assess the full migration renumber implementation, installer fixture details, release driver completeness, or every unrelated main delta. The parent can absorb the bounded adaptations autonomously, then require fresh combined review/evidence. No additional user approval is required by any finding above.
