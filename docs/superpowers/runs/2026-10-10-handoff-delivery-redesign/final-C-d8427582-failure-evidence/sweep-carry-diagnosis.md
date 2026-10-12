# Phase 6 sweep: Q4 nonempty carry fixture diagnosis

Scope: supplied clean HEAD `d8427582`, tree `53d0c453`; read-only investigation under systematic-debugging 6.4.2-alepar5.1. Only this report was written. No Cargo, tests, runtime helpers, source edits, Git/index, tracker or Herdr actions were performed.

## Observed failure and classification

`/private/tmp/ht-final-C-d8427582/nextest-run.log:488-490` records the exact failure at `tests/service/q4_accept_controls.rs:764`: `SETUP: actual unoffered carry at full capacity`. The test has already passed the real 256-offer assertion and obtained a real invitation page/token. It has NOT invoked `rig.accept("q4-retained-accept")` yet. Consequently this failure is not evidence that acceptance loses carry or outbox state. It is evidence that the fixture failed to create its demanded nonempty ordinary buffer.

The strongest supported diagnosis is stale fixture page/capacity geometry. Exact page-by-page causal attribution remains a hypothesis pending the small diagnostic below. There is no actual before-change run in this investigation and no claim that the failure is preexisting or flaky.

Other actual results in this same log: Q4 acceptance reopening, fanout, and rollback controls PASS (lines 450,451,463); full-pending quiet/due controls PASS (351,354); last-slot control PASS (332); private warning fresh/resumed exclusions and public/origin warning controls PASS (743,777,778). These do not prove this missing nonempty setup, but they constrain proposed changes.

## Complete fixture and production trace

1. `AcceptRig::new` (`q4_accept_controls.rs:226-412`) uses real Q3 channel registration, direct Ensure, initial Lazy seed, genuine CompleteInboxDelivery establishing an own cursor, Reject, reinvitation, a fresh Lazy, and ordinary Send followed by explicit ACK. The settled ordinary is excluded while invited; the fresh Lazy is really rendered and remains unread/uncompleted. Independent Hook and unrelated readers are drained. Registry passes establish quiet Watch/Hook headers before the test starts. Q3 channel constructor itself drains its actual Accepted/continuation path. The fixture clock is fixed.
2. The failing test (`:733-786`) sends 257 short ordinary bodies to joined thread `t`, then `drain_watch` repeatedly calls the real V2 route until `!page.more`/Unchanged. `OfferWindowFixture::pull` confirms only nonmessage presentation. No ordinary message ACK/display is performed in that drain. The retained direct Lazy also occupies a genuine existing offer. The test discards the returned map of actually rendered IDs.
3. Service input snapshots known IDs and not-yet-due blocked IDs and computes `slots = 256 - offers.len()` (`src/service/delivery_presentation.rs:201-248`). Fixed clock means known recent offers remain blocked.
4. Store reconciliation can free a slot only with canonical terminal evidence (`src/store/invitation_content.rs:1787-1901`). Ordinary ACK alone is insufficient while the binding remains unread; the predicate also checks its actual own cursor.
5. Fresh private capture is exactly one source query, limit32/max65536 (`src/store/queries.rs:7433-7484`). The cursor rotates Lazy, Unread, Invitations, Receipts, Warnings. Private mode disables Invitations and now Warnings. Pending ordinary receipts already unread are deduplicated against the Unread source (`:6972-7006`). Capture can be bounded by raw rows, bytes or candidate work. Capture position and actual offer ledger are different state.
6. `normal_mode = !owner_mode && input.new_message_slots > 0` (`invitation_content.rs:2332`). Only normal mode advances fresh capture or revalidates carry. The normal render loop removes blocked messages, emits new IDs until no slots remain, and persists only *already captured* leftover messages (`:2614-2657`, `:2728-2732`). Thus more canonical messages than available slots does NOT imply a nonempty carry buffer: capture can stop exactly at the last admitted message.
7. At zero slots `ordinary_runnable` considers only eligible known due IDs (`:2029-2043`), and source switches to LogicalDue. It intentionally neither captures fresh unknown IDs nor advertises carry-only endless continuation. Carry and parked cursor remain intact. Creating the unrelated Lazy invitation AFTER reaching capacity exercises independent passive presentation; it cannot manufacture fresh ordinary carry. This explains why the final raw invitation pull may have a token and empty carry at once.
8. Acceptance contract is explicit in `TRUST-POLICY.md:859-869`: preserve established cursors, seed missing open cursors at captured event floor, increment only current private passive/content counters once, preserve carry/token/partial/parked progress, atomic rollback, no replay invalidation. Implementation `src/store/read_cursors.rs:160-177` inserts missing cursors with DO NOTHING then calls the exact-binding helper; `src/store/presentation_reader.rs:134-157` updates ONLY `ordinary_passive_generation,mod_version`. The test snapshots every private field except these two and clones complete offer stamps; these are the correct no-weaken assertions.

## Effect of round-1 private warning exclusion, separately

The retained task21 review diff identifies production change `2b49940f`: fresh private captures set `next_manifest_warning=ordinary_only`; resumed private cursors normalize that bit true; the warning two-candidate reserve applies only to public discovery. Current source matches this flow (`queries.rs:6533,7163-7165,7296-7299`). It changes the *composition and candidate cost* of a bounded raw ordinary query, not offer capacity, carry preservation, acceptance mutation or ACK authority.

Before exclusion, a warning could consume a raw row and/or candidate work while assembling no ordinary `PendingMessage`: `src/delivery/assemble.rs:211-220` maps warnings to Attention/InformationalNotice, and `assemble_message` retains only Assembled::Item. Even a filtered warning candidate can consume work. Removing that traversal can shift which ordinary IDs land at a raw-page boundary. The rig has actual Reject history, but I have NOT established that the old private capture returned its warning to this recipient, nor measured its exact candidate cost. Do not state “one rejected warning used to occupy one row” as fact.

A consistent current geometry is: the previously offered Lazy is encountered/blocked during first capture, 31 fresh short bodies fill the other raw slots, then seven batches of32 reach 256 total known offers; two of257 new sends remain beyond the captured cursor. Empty carry then follows without a production defect. Exact counts/cursor geometry must be checked rather than inferred solely from the changed line.

## Minimal diagnostic and testable repair hypothesis

Parent's next focused diagnostic should retain the 257 Send IDs and the `drain_watch` output map, then record at the full-capacity boundary BEFORE creating the extra invitation:

- known offer IDs/count, intersection with the257 sends, direct Lazy presence, and unoffered send IDs;
- `ordinary_done`, decoded `ordinary_cursor` (source, unread/receipt positions, frozen high waters), `ordinary_pending` item IDs, and partial/skip state;
- optionally compact per-pull tuples `(offers_before, page.offered IDs/count, page.more, ordinary work, carry IDs, decoded unread position)` until the256th offer. No body dumps or broad store snapshots are needed.

Single hypothesis: the current reader fills capacity at a capture boundary and parks two genuine pending sends outside carry; one canonical fully displayed ordinary completion will free exactly one slot, allowing the existing parked capture to read both remaining short messages, emit one and retain the other at full capacity. This constructs the asserted state deliberately.

Smallest fixture-only experiment: keep the sent IDs and actual rendered map; select the earliest rendered, nontruncated ordinary send from thread `t` (NOT the invited Lazy). Use existing `rig.q.window.ack(std::slice::from_ref(&id), true, false, unique_operation)` to perform real AckModDelivered/Context after actual Watch serialization/display. The helper asserts Settled. Explicit/manual ACK (`mod_ack=false`) is not sufficient. Then boundedly drain through the real route and assert: the completed ID is reclaimed; all other original offers/stamps survive; exactly one formerly unoffered ordinary ID enters; offer count returns to256; carry contains at least one of the other genuine sent IDs, absent from offers and still canonically pending. Then create the unrelated invitation and take the raw unconfirmed page as before. Keep the existing exact acceptance comparisons unchanged.

This is a proposed experiment, not a tested patch. If the diagnostic finds different parked geometry, do not stack changes or force private state; revise the setup based on the actual cursor. Merely increasing257 to258 need not work: if capacity is reached before the next capture, any additional unpublished-to-reader tail stays outside carry too. Weakening the nonempty assertion, inserting JSON carry/offer rows, restoring private warning traversal, raising capacity, or permitting fresh capture at zero slots would miss the intended contract.

## Required controls / selector ownership

The parent sole runtime lane owns the one phase6 sweep-fix pass. Recommended focused exact library selectors (prefix `service::delivery_pull::tests::` unless specified):

- `q4_accept_controls::presentation_q4_accept_preserves_real_nonempty_carry_outbox_and_full_offer_window` — genuine nonempty unoffered carry + real outstanding outbox + full256, all retained across actual acceptance.
- Other three `q4_accept_controls::` tests — actual framed Watch/Hook output after acceptance, retained/missing cursors, one invalidation, replay/fresh-noop, disconnected/absent/stale fanout, whole-operation rollback.
- `presentation_capacity_last_slot_parks_unknown_carry_and_only_continues_for_due_keys` — correct positive-slot to zero-slot boundary (uses synthetic missing logical IDs internally; do NOT copy that setup into the canonical Q4 preservation test).
- `presentation_full_pending_carry_reaches_quiet_without_evicting_offers` and `presentation_full_pending_carry_serves_due_known_offers_without_admitting_fresh_id` — no spin, due-known liveness, parked normal-state preservation. Their helper name does not itself prove nonempty JSON carry; retain the Q4 explicit oracle.
- `presentation_watch_settled_receipts_reclaim_live_offer_capacity` — canonical display completion/reclamation route used to create the deliberate gap.
- `store::queries::tests::query_work_q4::{q4_private_fresh_excludes_warning_history,q4_private_resumed_excludes_warning_history,q4_public_and_origin_warning_sources_remain_enabled}` — preserve the round1 fix and independent public warning visibility.

After fixture establishment, strengthen rather than weaken by keeping acceptance-only before/after receipt and own-cursor snapshots if adding setup assertions: the setup display is allowed to change canonical receipt/cursor state, but acceptance itself still must not act as display/ACK. Existing other Q4 test already covers those acceptance invariants. No new production behavior or trust-policy change is justified by the observed failure.
