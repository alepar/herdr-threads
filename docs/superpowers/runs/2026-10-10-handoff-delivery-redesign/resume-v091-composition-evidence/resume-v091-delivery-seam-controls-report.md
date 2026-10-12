# v0.9.1 delivery seam controls — scratch patch, not executed

Prepared `resume-v091-delivery-seam-controls.patch` and corresponding full-file scratch snapshots `resume-v091-delivery-seam-controls-{integration,hooks,direct}.rs`. Only these scratch artifacts and this report were written. No source checkout edits, Git/tracker mutations, Cargo, runtime tests, Herdr calls, processes or delegation. No RED/GREEN or compilation claim. Parent owns review/import, formatting, execution, mutation checks and final combined verification.

Read the composed candidate TRUST policy before writing: A8 direct Lazy reader/current invitation episode rules, preserved direct-accept event-floor cursors, actual full-display authority, physical V2 confirmation exclusion, and the now-explicit generic Join frontier exception. This follows the parent's settled composition policy; no new delivery design is proposed. Applied test-driven-development skill guidance and `writing-good-tests.md`: tests name concrete production mistakes and use real canonical producers.

## Exact scratch bases

Read-only snapshot of the in-progress composed candidate at `ROOT/.worktrees/upstream-composition-v091`, built from immutable M `20e7ca73e41a9449376169ab6875c0675da02edc` and F `0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2`. The patch records only the intended test deltas, not unrelated schema-author edits. Baseline SHA256 values were checked again before the final patch rewrite:

| Path | Captured SHA256 |
|---|---|
| `tests/integration/lazy_attention.rs` | `a6c1d82510e18a2b2fe82a3c7b9c34cfb69b685f4e936651bb55e9ad0fc50fe3` |
| `tests/hook_entrypoint.rs` | `0ca9240da537fbc654614ee21f42899732263600aca593e9afbfc8dfccf9c95d` |
| `tests/store/direct_threads.rs` | `fb32610e7ad151b04a31f4ada4ab92b55a8761d042cef5fc915670348feb5905` |

Apply the patch rather than copying whole scratch files over a later candidate. Whole `.rs` files are review material and may become stale.

## New mixed-invitation causal test

`in_turn_timed_invitation_service_ignores_unseen_lazy_backlog_per_binding` extends the existing canonical store integration fixture. Existing fixture setup seeds host/seat structure; the second binding is enrolled by actual `register_available`, each thread/invitation is created by actual accountable `CreateThread`/`Invite`, and every display uses actual `AdvanceCursors { in_turn: true }`. No invitation/offer/cursor rows are hand-seeded to obtain the asserted behavior. The test's store has an explicit daemon boot so `reserve_wake` can produce a real reservation.

The sequence proves:

1. A RequireAck invitation at fixed committed time 1000 with 10000 ms duration has immutable pending policy/deadline 11000; both distinct live bindings are wake targets and an actual reservation contains only `pending_invitation`.
2. A displays it: aggregate wake remains owed because B did not, but `wake_owed_to_binding` is false for A and true for B. B displays it: aggregate and both exact-target answers become false.
3. One unseen Lazy invitation does not undo that suppression. A full backlog of unseen Lazy invitations likewise does not. Both candidate coverage and the final exact-target coverage method are checked, so changing only one of the two call sites back to `pending_invitations` fails.
4. The backlog exceeds the actual source window: `attention::WINDOW + 1`, currently **1002**, because the attention cap is 1000 plus one saturation row. Merely 101 Lazy invitations would not test filter-before-LIMIT. The old timed ID remains the sole nonsaturated confirming result while all-pending discovery is saturated.
5. A new historical NULL-policy invitation independently restores wake for both bindings; showing it to A again leaves only B owed. Showing it to B clears only wake eligibility. Both original timed policies/deadlines remain exact, every invitation stays pending, no accept/receipt/reader cursor appears, and exactly four in-turn records exist.

This is a canonical store boundary test of `wake_candidates`, `reserve_wake`, and `wake_owed_to_binding`. It intentionally does not claim to execute a physical native dispatcher. The existing upstream dispatcher tests cover identity/foreground selection, reservation fence ordering and no-keystroke refusal. In later stages this test reuses the retained invitation-only reservation to exercise the owed-work predicate; it does not assert that its old witness still passes the separate reservation-validation method after later mutations.

Execution cost is unmeasured. The 1002-row prefix uses roughly 2000 real canonical create/invite mutations; if too expensive in parent execution, retain the one-Lazy end-to-end case and move only the saturation boundary to an existing bounded query fixture, explicitly retaining an actual producer control. Do not call a 101-row population proof of the 1001-row attention-window boundary.

## Upstream test oracle changes, grounded independently

### Mixed opening hook fixture

Rename `pair_accept_delivers_mixed_opening_mail_once` to `pair_accept_preserves_the_mixed_opening_reader_gap`, keeping the real hook/CLI setup, exact original bodies, explicit ACK, acceptance and quiet-followup assertions. The changed expected body counts are:

| Body | Before accept: M -> composed | After accept: M -> composed |
|---|---|---|
| ordinary-acked | 1 -> 1 | **0 -> 1** |
| ordinary-pending | 1 -> 1 | 1 -> 1 |
| lazy-open | **0 -> 1** | **1 -> 0** |
| lazy-middle | **0 -> 1** | 1 -> 1 |
| lazy-last | **0 -> 1** | 1 -> 1 |

This is not a blanket “Lazy appears before accept, disappears afterward” rule. Composed `control::advance_cursors` admits preaccept direct Lazy IDs, but does not admit unjoined ordinary IDs into its cursor sequence list. Its shared cursor walk therefore passes `lazy-open` and stops at the next `ordinary-acked` row. Hook `CompleteInboxDelivery` contains the Lazy IDs, not ordinary display authority, and cannot pass that ordinary gap either. The following explicit `ack` has no display provenance. Direct acceptance preserves the existing cursor, so the now-joined reader sees the settled ordinary gap and the later Lazy bodies again. Modern Lazy unread does not use historical seat-global `state='displayed'` to suppress this binding's stream. This is already the feature's documented shared-cursor consequence.

Added independent database assertions that the exact reader cursor is at `lazy-open` before acceptance and remains there immediately after acceptance. These expose a mistaken generic-head merge directly, before later rendering can hide it. The final zero legacy marker assertion stays, with corrected rationale. Actual runtime may reveal further inherited page ordering/marker interactions; do not weaken the oracle to match output without checking this canonical cursor trace.

### Two-binding hook reaccept fixture

`pair_rejoin_with_two_bindings_pins_read_positions` keeps its original actual producer sequence and bodies. Changes:

- B sees `first-contact` after A displays it: false -> true.
- B sees unread `joined-era` after direct reaccept: false -> true.
- A sees `rejoin-open` after B displays it: false -> true.
- A still does not repeat its own already-read `joined-era`; both still receive `after-rejoin`, without repeating `rejoin-open`.
- Added exact before/after-reaccept cursor tuple equality. The test now detects any direct accept that raises either established cursor to head.

These follow the approved independent binding stream, not global receipt-style consumption. The fixture's original live/open binding setup is preserved.

### Store direct acceptance fixture

`accepting_a_pair_invitation_keeps_the_opening_messages_unread` keeps both Ordinary and Lazy modes and actual Ensure/Send/Accept operations. The missing reader cursor now equals the exact immutable `invite:<invitation-id>` timeline event sequence instead of the latest ordinary body sequence. This expectation is read from the retained event key, independently of the production floor helper. Both modes remain unread: old false for Ordinary -> true. Both modes retain zero `legacy_pending` rows: old Lazy 2 -> 0. No historical cursor is rewound or fabricated.

### Generic join out-of-order display control

Rename `rejoin_never_reshows_a_lazy_row_displayed_out_of_order` to `generic_join_never_reshows_a_lazy_row_displayed_out_of_order`. Keep its pair, actual initial acceptance, both original Lazy bodies, actual display bookkeeping, held-cursor assertion and actual Leave. Replace only reinvite/reaccept with actual plain `Join` under the caller's accountable permit; no active pending invitation exists on that path. **Keep both original final expectations unchanged:** undisplayed `held` gets seat-pending 1; out-of-order `shown` stays 0. This preserves main's generic Join regression and explicitly selects the route to which that policy still applies. Direct reaccept coverage remains in the Q4 and two-binding tests rather than asserting the generic policy on the excluded direct-accept route.

## Existing controls to select, not duplicate

- Origin warning independence: `departed_receipt_origins_survive_pull_offers_late_association_and_partial_ack` and `a_departed_inviter_remains_actionable_after_whole_deadline_warning_until_acceptance` in `tests/service/delivery_pull.rs`.
- Main final-dispatch fence/identity: `in_turn_service_is_decided_for_the_binding_the_composer_reaches`, `an_in_turn_offer_after_reservation_cancels_that_bindings_wake`, `a_binding_served_one_reason_is_still_woken_for_another`, `a_recovery_hint_wake_to_a_served_binding_is_refused` in `tests/store/wake.rs`.
- Q4 positive/rollback: `q4_initial_accept_preserves_each_bindings_unread_body_and_display_gap`, `q4_direct_lazy_completion_is_independent_for_two_initial_bindings`, episode-two direct readers; `presentation_q4_accept_only_reopens_quiet_watch_and_hook_with_preserved_reader` and `presentation_q4_accept_private_failure_rolls_back_episode_join_all_readers`.
- V2 confirmation/read/receipt separation and exact owner: existing `presentation_v2_replays_one_canonical_invitation_until_emit_confirmation`, `presentation_watch_fresh_confirmation_records_exact_owner_publication_cutoff_once`, Q4 physical preaccept body/failed-output controls and owner-replay tests. This patch adds no physical-confirmation-to-in-turn/cursor/ACK evidence bridge and changes no V2 producer.

Parent should import after checking base drift, format only the intended test deltas, compile and run these focused controls, and verify at least each of the two wrong invitation source substitutions fails the new test. Source-only reasoning does not establish its failure mode, runtime cost or GREEN result.
