# Phase 6 recovery-counter diagnosis

Scope: read-only diagnosis of `store::attention::tests::presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement` at supplied HEAD `d8427582`, tree `53d0c453`. No source/test patch, Git operation, Cargo/runtime invocation, tracker/Herdr mutation or delegation was performed. Only this report was written. Applied systematic-debugging instructions from cache `6.4.2-alepar5.1`.

## Finding

The failing counter oracle predates Q4's three additional direct-invitation episode points. Current production accounting agrees with the implemented SQL and the explicit current contract: lazy exact-key eligibility charges ten relation probes, then up to three hydration probes, plus the initial message point. The old test still expects seven eligibility probes. The minimum justified correction is the exact test oracle, with decomposition explaining the three added keyed relations. Do not lower production accounting or raise work caps.

The actual first failure is replay-A's `30 != 27`, after both `ordinary.is_some() == false` and `lazy.is_some() == false` have passed. Replay-B is not reached in this failing run. Its source-derived expected total is **35**, not the test's old **32**; this is a prediction requiring the parent's green run, not a claimed observed result.

There is also a directly related stale prose sentence in `TRUST-POLICY.md:1694`: it still says Lazy is bounded by 11 probes, while `:1614-1617` already specifies the Q4 Lazy14 and unchanged due1803/carry579 contract. Correcting that one stale value to14 reconciles the document without changing an invariant or allowance.

## Actual baseline evidence

- `/private/tmp/ht-final-C-d8427582/nextest-run.log:582-585`: exact failure at `tests/store/attention.rs:4175`, left30/right27, 0.207s.
- `/private/tmp/ht-final-C-three-repro-d8427582/run.log:26-45`: parent-owned focused reproduction fails again on the same assertion, 0.167s. No stochastic explanation is needed.
- Historical `/private/tmp/ht-C-v090-final-composed-657/nextest-run.log:1666`: this exact test passed in0.258s. That run's `execution.json` records actual and expected HEAD `9a573c3f2a5420ff0c3fee07a3e836d189e88a83`, tree `5158b885c8988b1c3ebf5b96047751a6e4949212` (actual head/tree appear at lines283/284,412/413,587/588). The larger historical run had other failures; this is evidence only for this individual case.
- The original author report `resume-main-v090-store-binding-author.md`, sections “Indexed probe accounting” and “Tests retained and added,” explicitly records lazy11, A27 and B32.
- The subsequent saved patch `resume-v090-q4-query-implementation.patch:174-189` changes lazy eligibility `+=7` to `+=10`, with the explanation “Existing seven relations plus the three exact episode points.” `resume-v090-q4-reader-implementation.patch:41-42` adds those episode joins. The interface artifact `resume-v090-q4-query-implementation-interface.md` states old11→14 and unchanged ordinary18/due1803/carry579. These are source/history artifacts, not substitutes for execution.

This is a causally explained composition drift from the demonstrated older passing oracle. No claim that the current failure is preexisting, flaky or harmless by history alone is made.

## Complete fixture and reached paths

`tests/store/attention.rs:34-75` creates an in-memory fully initialized store with instance `i` at decision40, resolved native seat `s` and other seat `o`. `every_source()` adds five threads and independent invitation/ordinary/warning sources. Relevant ordinary `m3` is a published manifest message in `t3`, sequence1/decision10, with prepared recipient `(p,s)`, `eligible_at_snapshot=0`. Other fixture mail belongs to other threads or recipient `o` and is excluded by exact IDs and addressing.

The target test at `:4114-4200` then adds:

- Two explicit top-level connected Codex bindings `replay-A` and `replay-B`, both of `s`, with different sessions/executions, same generation/target and non-null registration.
- Joined membership `(t3,s)` and binding cursors A50/B0.
- Sparse canonical `receipt_state(m3,s)='acked'`, attributed to `replay-A`.
- Published modern lazy message `msg-replay-lazy`, sequence50/decision30, with matching lazy recipient, preparation and manifest. It sets `legacy_pending=0`, then historical row state `displayed`.

The loop uses exact BindingId A then B. `ordinary_due_context` reads head40 and calls `current_agent` with that BindingId; `queries.rs:7117-7134` selects `b.binding_id=?2`, not the seat's newest binding. Both bindings meet current-agent conditions. The context is three charged relation points: instance, seat, exact binding.

For ordinary `m3`, canonical effective receipt is already acknowledged, so the pending/ACK branch is false. Eligibility falls back to that binding's unread query. A's cursor50 excludes sequence1; B's cursor0 includes it and yields a Message with `ack_candidate: None`. The prepared recipient, published manifest, message, joined membership and exact binding cursor remain canonical authority.

For modern lazy `msg-replay-lazy`, the SQL tests sequence50 against that exact binding's cursor. A50 is read; B0 is unread and has retained cursor scope (also joined membership). The SQL intentionally does not consult the historical `lazy_recipients.state`; `legacy_pending=0` prevents the legacy top-level exception. Therefore A sees neither message, and B must see both even though A settled the seat receipt and the lazy physical row says displayed.

The helper executes SELECTs only. Existing final assertions retain ACK attribution `replay-A` and B's cursor0. They are not reached in the red run because A's counter assertion stops the test. Keeping and reaching them in the corrected green run is required before claiming sibling preservation. No canonical settlement should be added to setup or recovery.

## Accounting decomposition

`src/store/queries.rs:7486-7573` implements the exact source. The counter is a conservative relation envelope, not a count of SQLite statements or only branches that physically execute. Short-circuited eligibility still receives the fixed ten-relation charge.

| Work component | Charged points | Explanation |
|---|---:|---|
| Context |3| `host_instances(id)` plus seat/current exact binding |
| Ordinary initial key |1| ordinary message ID, instance and publication bound |
| Effective receipt |10| existing conservative envelope, including sparse/fallback/waiver paths |
| Ordinary settled unread check |5| prepared recipient, manifest, message, membership, own cursor |
| Ordinary hydration if unread |2| message and topic thread |
| Lazy initial key |1| message ID, instance and publication bound |
| Lazy eligibility |10| four outer joins + two own-cursor points + membership + three episode points |
| Lazy hydration if unread |3| message, topic thread and seat wake preference |

The effective-receipt maximum already includes conservative branch costs; its separate availability seek is charged once per ordinary key even when it returns no row. Here `m3` is ineligible-at-send, so its indexed availability query executes and returns none; acknowledgement remains authoritative. Do not add ten to physical statement counts and assert equality with SQLite executed operations.

Derived totals:

- A: `3 + (1+10+5) + (1+10) = 3+16+11 = 30`.
- B: `3 + (1+10+5+2) + (1+10+3) = 3+18+14 = 35`.
- One `ordinary_due_seek` for each loop iteration. Lazy introduces no availability seek.

The old assertion's `3+16+8` and `3+18+11` exactly encode seven old lazy eligibility points. The difference is precisely the Q4 addition of3; neither ordinary eligibility nor hydration changed.

## Concrete indexed keys behind the lazy ten

The addressed SELECT at `queries.rs:7534-7536` expands `binding_lazy_unread_sql!` and `binding_lazy_scope_sql!` (`lazy_delivery.rs:47-102`):

1. `lazy_recipients r`: full unique `(message_id,seat_id)`; preparation+seat is an equivalent complete unique alternative.
2. `send_manifests sm`: exact preparation primary key, with equality checks for message/thread/instance/publication.
3. `messages m`: unique message ID, plus same canonical instance/thread/decision.
4. `occupant_bindings b`: unique exact binding ID, with seat agreement.
5. `binding_cursors lc`: full `(binding_id,thread_id)` primary key for sequence.
6. `binding_cursors lc`: same complete key for retained stream scope.
7. `memberships lm`: unique `(thread_id,seat_id)`.
8. `direct_threads ld`: unique thread ID, with canonical instance and endpoint membership.
9. `seats ls`: exact seat primary key, checking instance/nonretirement.
10. `invitations li`: unique `(thread_id,seat_id,episode)`, where episode is the exact membership episode; pending state and publication-after-creation predicate.

The uniqueness contracts are in migrations0001,0026,0029 and0032. There is no latest-invitation search, invitation-event history scan or unbounded seat binding search in these new points. On this fixture, own-cursor scope means episode points can short-circuit; conservative accounting deliberately still reserves them. An optimization that removes the three charges only for this fixture would misstate the helper's agreed envelope.

## Independent current controls already observed

The final-C log records PASS for:

- `store::attention::tests::presentation_capacity_due_exact_keys_have_bounded_actual_sql_plans_and_vm` (log620). It traces actual production statements, rejects full scans/sorts, caps statement VM≤300 and checks equality between retained-history32/2048 matrices. It covers both ordinary and lazy exact helpers, absent/unpublished/wrong-instance keys and waiver lookup.
- `store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers` (log831). It calls the actual exact-ID helper, asserts a returned LazyMessage, exact14, no fresh-scope double charge, and equal helper VM work at retained-history128/8192. Its invited/cursorless fixture actually exercises the added episode branch.
- `store::queries::tests::query_work_q4::q4_episode_private_and_reconciliation_points_have_constant_sqlite_work` (log833). It checks the actual composed point SQL's ten aliases and complete equality keys (`query_work_q4.rs:233-315,348-395`), no scans/sorts, canonical eligibility and invariant VM as retained history grows.
- `store::queries::tests::query_work_q4::q4_scalar_agreement_and_strict_retirement_use_exact_scope` (log766).
- `store::queries::binding_inbox_tests::bound_lazy_stream_is_per_binding` (log730), `one_bindings_display_never_hides_mail_from_the_other` (log739), and `service::delivery_pull::tests::presentation_two_live_bindings_keep_offer_stamps_replay_and_carry_independent` (log444).

These controls support the existing production accounting and binding semantics; they do not turn the unexecuted B branch of this specific failing test into a pass.

## Proposed minimum scope and parent verification

1. In this test only, replace the stale lazy8/11 branch totals with11/14, retaining ordinary16/18 and setup3. Prefer named local expected components or stage-local delta assertions (setup3, ordinary16/18, lazy11/14) and a short comment identifying pair/seat/current-invitation points. Keep equality, both binding cases, seek1, no-ACK candidate and final canonical-state assertions. A mere opaque27→30 replacement is insufficient because B also changes32→35 and the reason must remain reviewable.
2. Reconcile the stale Lazy11 sentence in `TRUST-POLICY.md:1694` with its already-correct14 stanza. No production, migration, budget or delivery-policy change is justified by this failure.
3. The single parent-owned sweep fix pass should run the exact failing selector and preserve evidence that B executes and the final ACK-binding/cursor assertions pass. The existing red and focused reproduction are sufficient red evidence; no extra baseline run is needed merely to repeat them.
4. Retain the exact-ID SQL/VM and Q4 actual-helper controls above in the parent post-change selection. Their final-C pass is useful pre-change evidence; rerunning after a test-only correction is an inexpensive confirmation relative to broadening the sweep. The Q4 retained-history tests are around16–17s each in the supplied sweep; no full suite is requested.

Exact minimal diagnostic filter: `test(=store::attention::tests::presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement)` on the library binary. Supplemental exact filters are the first four names in the control list. If the parent wants a diagnostic before editing, add per-stage assertion messages carrying binding ID and context/ordinary/lazy deltas; expected A3/16/11, B3/18/14. Production instrumentation is unnecessary given existing traced SQL controls and explicit source accounting.

Bounds remain `3+100*18=1803` for due/replay and `3+32*18=579` for carry because Lazy14 remains below Ordinary18. `invitation_content.rs:2880-2893` still enforces these values with100 seeks and32 hydrated items; no limit should be changed. Recovery remains read-only with respect to canonical receipts, lazy settlement and binding cursor progress.
