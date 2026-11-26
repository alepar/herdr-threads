---
super-roast verdict: Should-fix (17 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 1 of 3
profile (assumed): Not supplied; inferred from AGENTS.md and the packets. herdr-threads is a locally installed, single-user daemon and CLI for message threads between coding agents, with a cooperative same-user trust model and no adversarial caller verification. State lives in a local SQLite store with one-way schema migrations, and nodes replicate facts to peers that may run other versions; one external consumer (herdr-graph) uses the service-session API. Blast radius is small (few users, no money), but store and peer-wire changes are hard to roll back and TRUST-POLICY.md is normative, so convergence defects and policy contradictions keep Should-fix weight.
inputs: feat/seat-resurrect@c190211f3 vs main@24972624a
coverage: scouts 15/15 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs, deploy-safety) · raw 104 → deduped 72 → panel 29 · spot 43 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 29 · rr 0.90 · rg 0.97 · fg 0.86 · unanimous 0.86 · ground-loo 0.96 (n=26) · reproduce 17/12/0 · refute 14/15/0 · ground 18/11/0
lane-yield (found/confirmed/unique/refuted): correctness 8/4/1/3 · security 4/1/0/3 · premortem 8/0/0/6 · simplicity-design 13/1/0/4 · hot-path-perf 4/0/0/4 · concurrency-async 4/1/0/3 · data-migrations 6/1/0/5 · api-contract 11/3/0/7 · observability 6/2/0/4 · testing 16/9/7/4 · test-causality-lifecycle 2/0/0/2 · test-isolation-environment 3/0/0/3 · test-suite-cost 2/0/0/2 · hygiene-docs 9/6/4/2 · deploy-safety 8/0/0/8
note: review readiness arrived as coverage unknown, provenance unknown, with an empty basis, so `[review readiness incomplete]` and the compatibility `[low coverage]` are appended. Stage coverage itself is complete (lowCoverage false, no dead seats). No severity floor applies and no severity was demoted on profile grounds; severities follow the confirming seats, by majority where they split.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] src/service/dispatch.rs:2054; src/service/dispatch.rs:2053 — ResurrectSeat is dispatched before the normal service-operation path, so it skips the ServiceAuthorityGate decision guard: a revoked or superseded connection generation can still commit a resurrect or a `replace` final retirement of NEW. It also skips `operation.validate()` and the per-session service journal scope. No test covers a revoked session. [lanes: correctness, security, simplicity-design, concurrency-async, api-contract, observability, testing]
  verdict: confirmed 2-1 (reproduce ✓ / refute REJECT at FYI, outvoted / ground ✓)
  evidence: dispatch.rs:2053-2054 returns `self.service_resurrect(request, connection, budget)` before `self.store.service_operation(..., gate, ...)`; `gate` is never used.
    service_controls.rs:200 answers `Unsupported` for ResurrectSeat, so the guarded path (`ServiceDecisionTransaction::begin`, service_controls.rs:57) is never reached.
    service_connection.rs ~302-316 runs the operation in `spawn_blocking` and revokes on disconnect or expiry; with no decision guard held, the revoke does not wait for the resurrect.
    The journal scope is `service-resurrect:{instance}:{author}`, not `service:{instance}:{author}`, so one operation key reused across FinalizeRetirement and ResurrectSeat is not caught as OperationPayloadMismatch.
    Dissent: the refute seat cites spec D4 and section 5 ("any registered v4 service session") and the cooperative model. Both confirming seats answer it: the spec is silent on the gate and on revocation. They accept only the skipped `ensure_reserved_author` check as spec-covered for keyless seats, so that part is outside the confirmed scope.
    No floor: the reproduce seat finds no cross-user escalation, and the commit executes the one request the client made.
  fix-shape hint: run the operator repair transaction under the gate's decision guard, or recheck the live generation inside the deciding transaction; add a revoked-session test.
- [Should-fix] src/store/seats/publication.rs:607; src/store/seats/publication.rs:590 — Applying or replaying `seat.obligations_expired` records `obligation_expiries` but never sets `reversible_retirements.obligations_expired_at`. After a restore from the own log the home node expires the same episode again with a later `expired_at` and authors a second fact. [lanes: correctness, data-migrations]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `obligations_expired_at` is written in one place only, `expire_obligations` (reversible.rs:254); all three seats confirmed this by grep.
    `materialize_seat_obligations_expired` only calls `reversible::record_expiry`.
    `timed_due` (reversible.rs:118-121) and the `expire_obligations` guard (:245) test only `obligations_expired_at IS NULL AND cutover_at+24h<=now`; neither reads `obligation_expiries`.
    Result: replay records E1, the row stays NULL, the lane re-expires at E2 > E1. `record_expiry` keeps E1 (`INSERT OR IGNORE`) while `settle_expired` runs with E2, so obligations created in (E1, E2] settle on nodes that hold them and stay pending for late copies.
    This contradicts spec section 2 ("Re-applying settles nothing new") and the late-arrival agreement.
    One seat did not trace `insert_fact_row` for the replayed row; the grep result covers it.
  fix-shape hint: set `obligations_expired_at` when the fact is applied to the home row, or have `expire_obligations` skip when an `obligation_expiries` row exists for (seat, cutover_at); settle against the stored `expired_at` so the `record_expiry` mismatch (rejected below) cannot arise.
- [Should-fix] src/store/control/invitation.rs:903 — A late-arriving voluntary invitation that `expired_episode` settles at write, sent to a seat since resurrected and with no prior membership in the thread, inserts a terminal `memberships` row with state 'retired' for a live seat. That blocks later voluntary invitations to that thread on this node, and the node diverges from nodes whose copy `settle_expired` settled. [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `expired = Voluntary && expired_episode(...).is_some()` forces `state = "recipient_retired"` whatever the seat's current state.
    The `state == "recipient_retired" && prior.is_none()` branch (~line 963) then inserts `memberships(...,'retired','retired',seat_retired_at)`; after `unretire_local` that `retired_at` is NULL.
    `place_invitee` (line 1026) treats `Some("retired")` as terminal: `(Voluntary, Some("joined" | "retired")) => {}`.
    `settle_expired` never touches memberships, so nodes that held the invitation at the expiry keep 'invited'.
    Spec section 2 says "Expiry never touches memberships" and that late arrivals are "settled at write, by the same rule".
    The receipt late-arrival path (materialization.rs:746+) uses the returned episode and writes no membership row.
  fix-shape hint: skip the terminal membership insert when the cause is `expired` rather than a finally retired seat, and use the episode from `expired_episode` for the invitation's `retired_at`.
- [Should-fix] TRUST-POLICY.md:2513; TRUST-POLICY.md:2512 — The accepted limit for a store restored from its own log still says `FinalizeRetirement` answers `Unauthorized` and only an operator can finalize a restored reversible seat. The new code lets any service session finalize it, and the 7 d window finalizes it automatically. [lanes: api-contract, hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: TRUST-POLICY.md:2513-2515 at head is an unchanged context line in the diff.
    The same commit adds "A service session may also finalize a keyless seat (`service_finalize` with `service:<author>`)" and "a row replayed from the own log, or a keyed seat whose key is gone, is timed too".
    The test was renamed `unretire_answers_unauthorized_and_finalize_succeeds_after_restore`.
    AGENTS.md: "A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit."
  fix-shape hint: rewrite the restored-store limit to say `UnretireSeat` still answers `Unauthorized`, while finalize succeeds for any service session and at 7 d.
- [Should-fix] TRUST-POLICY.md:2508 — The policy still lists the ht-ma6i.3 accepted limit "A keyless seat's close stays final". This commit makes it false and adds the replacement text without removing it, so the normative policy contradicts itself. [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: TRUST-POLICY.md:2508-2509 is untouched by the diff (base line 2471, true there).
    The same commit adds line 122 "every `Closed` retirement is reversible; a keyless one is timed", lines 556-559, and the "Seat resurrect (ht-eu3v)" limit at :2516.
    Two seats also found a second stale clause at ~:3037 in the G5 text: "a keyless seat's is final".
  fix-shape hint: delete or rewrite the :2508 bullet and the :3037 clause, or mark them superseded by ht-eu3v.
- [Should-fix] src/service/dispatch.rs:591 — The service-session ResurrectSeat path (`DomainService::service_resurrect` plus the dispatch short-circuit) has no test. The v4 level gate, the v3-session refusal, the other-instance refusal, the missing-owner-uid branch, the OperatorRebound-to-SeatResurrected mapping and the client's `result_matches` arm are unexercised. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `git grep ResurrectSeat` and `service_resurrect` over tests/ find nothing at c190211f3.
    tests/store/reversible_retirement.rs:1963 `service_resurrect_authority_and_trail` builds `OperatorActor::for_service(501, ...)` by hand and calls the store, skipping dispatch.rs:2053, protocol/service.rs:83 and client/service.rs:966.
    Spec section 10 lists "Service `ResurrectSeat`: keyless by any v4 session, keyed by owner only, v3 session refused".
    The store-level authority halves (keyless by any session, keyed by owner) are covered.
  fix-shape hint: add a service-session test that sends ResurrectSeat from a v4 session and from a v3 session, and one from another instance.
- [Should-fix] src/store/seats/publication.rs:589 — The cross-node `seat.obligations_expired` apply path (`materialize_seat_obligations_expired`, and fact authoring through `emit_if_published`) is never run by a test. The tests call `record_expiry` directly. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: the only behavioural test, tests/store/reversible_retirement.rs:2074, calls `record_expiry(&tx, ...)` twice on a live local seat.
    Other test hits are registry and name lists: tests/sync/facts_seat.rs:841, incarnations_scenarios.rs:1732, peer_wire_fixtures.rs:43-44.
    The unknown-seat branches (publication.rs:600-606), the payload's needs/creates, a peer apply and the parked-then-replayed case are untested.
    Spec section 10 line 193 lists these cross-node cases.
    Not verified by the seats: that the fixture seats are unpublished, so that `emit_if_published` is a no-op in every test.
  fix-shape hint: add sync-level tests in tests/sync/facts_seat_reversible.rs that apply the fact on a peer, after an earlier `seat.unretired`, and after a park and replay.
- [Should-fix] src/store/materialization.rs:737 — The late-arrival rule for ACK receipts (`project_receipt` settling a message created before a recorded expiry) has no test. Only the invitation half is covered. [lanes: data-migrations, testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: the new `expired_episode` lookup, the `_ if expired.is_some() => "recipient_retired"` arm and the `retired_at = Some(episode)` override are at materialization.rs ~737-775.
    `an_obligation_created_before_a_recorded_expiry_settles_at_write` (reversible_retirement.rs:2019) feeds only `InvitationCreatedPayload` through `materialize_invitation`.
    No test projects a message receipt after `record_expiry`; a wrong `decision_at` comparison or a dropped `retired_at` would pass.
  fix-shape hint: add a store test that projects an ACK-required message with `decision_at` before a recorded expiry and asserts `recipient_retired` with the episode's `retired_at`, plus the boundary one millisecond after.
- [Should-fix] tests/store/reversible_retirement.rs:1795 — No test asserts that an expired obligation's sender gets the overdue warning the spec requires. Removing the warnings from the expiry rule would fail no test. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: spec line 58: "Each settled obligation gets the overdue warning the retirement job writes, with the same warning basis."
    `settle_expired` calls `schema::record_overdue_if_pending(..., &TimeBasis::Expiry(..), decided)` and the `clear_warning_condition*` helpers.
    `expiry_settles_ordinary_obligations_and_keeps_memberships` asserts states, memberships, intervals, `obligations_expired_at` and the expiry count, and never reads `warning_message_id`, `warning_conditions` or `kind='warn'` rows.
    `TimeBasis::Expiry` appears in no test.
  fix-shape hint: assert the warning row and its Expiry basis for the settled receipt and invitation in the existing expiry test.
- [Should-fix] src/protocol/authority.rs:249 — The spec's test for a lost-response retry of ResurrectSeat with `replace` under the same operation id is missing, although the diff changes the operation scope used for service-actor idempotency. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: authority.rs:249-250 makes `operation_scope` return `service-resurrect:{instance}:{author}` for a service actor.
    Spec line 196: "`ResurrectSeat` with `replace`, retried with the same operation id (lost response): one decision."
    Every `rebind`/`rebind_as` call in tests/store/reversible_retirement.rs uses a distinct operation key; the only `--replace` test runs once, as the operator.
    The refute seat rated this Nit; it did not check whether a retry is mishandled.
  fix-shape hint: call the service resurrect with `replace` twice under one OperationId and assert one decision and one NEW retirement.
- [Should-fix] docs/superpowers/specs/2026-11-25-seat-resurrect-design.md:176 — Spec section 10 says each listed item is one named test, but several are missing and none is recorded as deferred: the ResurrectSeat `replace` retry, a v3 session refused for ResurrectSeat, `seat.obligations_expired` parked then applied on replay, and operator `--replace NEW` with an open binding on NEW. [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: the test diff has no session-level ResurrectSeat test, no same-operation-id retry and no open-binding-on-NEW test.
    The parked-replay item is covered only by adding the kind to `registry_lacking` and the HELD_KINDS lists.
    A grep of the spec for deferred or follow-up finds only D3, which is unrelated.
    Three of the four items overlap the dispatch.rs:591, authority.rs:249 and publication.rs:589 findings above. The item unique to this entry is the open-binding-on-NEW test (plain calls stay NEW's, a lifecycle check-in enrolls OLD).
    The reproduce seat rated this Nit; the other two rated it Should-fix.
  fix-shape hint: add the open-binding-on-NEW test, and for any item deliberately left out record it in the spec as deferred.
- [Nit] TRUST-POLICY.md:2504 — The accepted limit "A service that never acts (ht-ma6i.3)" still says threads runs no timer, so a reversibly retired seat stays frozen until its service or an operator acts. That is no longer true for keyless seats. [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: TRUST-POLICY.md:2504-2507 is untouched by the diff.
    The same file now says the deadline lane "expires a timed row's obligations at `cutover + 24 h` and then finalizes it at `cutover + 7 d`".
    Severity: two seats rated Nit because the text stays true for keyed seats; the ground seat rated Should-fix. The majority stands.
  fix-shape hint: narrow the limit to keyed seats, in the same edit as the :2508 and :2513 bullets.
- [Nit] src/store/seats/operator.rs:311; tests/store/reversible_retirement.rs:1949; docs/superpowers/specs/2026-11-25-seat-resurrect-design.md:108 — The spec's `next:` guidance line for `--replace NEW` with a live agent on NEW (restart the agent or run /clear) is not implemented, not tested, and the deviation is not recorded. [lanes: correctness, testing, hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: spec lines 107-108 require one `next:` line in the command's text output.
    `git grep -i "restart the agent\|/clear" -- src` at c190211f3 finds no rebind output string; the rebind text is still `Rebound seat {}.` (src/cli/human.rs:207).
    The spec's implementation notes at lines 133 and 139 cover other deviations, not this one.
    The seat state stays correct: plain calls stay NEW's and the next check-in enrolls OLD.
  fix-shape hint: print the `next:` line when NEW has an open binding and state its byte size in the commit, or record the deviation in the spec.
- [Nit] src/store/seats/operator.rs:295 — A service ResurrectSeat with `replace` records NEW's final retirement as `operator_retire` (retire reason Operator) under the label `service:<author>`, a combination the provenance table does not define. [lanes: api-contract, observability]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: the replace branch inserts kind 'operator_retire' with `actor.audit_label()` and calls `begin_retirement_authored(..., RetireReason::Operator, ...)`.
    The unretire and rebind halves are switched to `service_unretire` and `service_rebind`.
    TRUST-POLICY ~line 816: "An operator's finalization records `operator:local-user:<uid>` under `operator_retire`".
    The label column still carries `service:<author>`, so the actor is recorded honestly; only a reader keyed on kind alone is misled.
  fix-shape hint: record a service kind for the replaced seat, or define the `operator_retire` plus `service:<author>` combination in TRUST-POLICY.
- [Nit] src/scheduler/deadlines.rs:420 — No test runs the 24 h expiry through the daemon's deadline lane or the error path of `timed_retirements` in `drive_due`, and `TEST_OBLIGATION_EXPIRY_ENV` is defined but no test sets it. Narrowed from the original claim: the lane wiring itself is exercised. [lanes: concurrency-async, testing, observability, test-isolation-environment]
  verdict: confirmed 2-1, narrowed (reproduce ✓ / refute REJECT at FYI / ground ✓)
  evidence: the refute seat's evidence disproves the "wiring is unchecked" half. tests/integration/session_bindings_smoke.rs sets `TEST_KEYLESS_FINALIZE_ENV=0`, and finalization runs only in `timed_pass` reached through `drive_due` (store/mod.rs:3115, reversible.rs:180-185). The reproduce seat did not check this; the ground seat concedes the smoke test uses the timed path.
    All three seats agree on the remainder: `TEST_OBLIGATION_EXPIRY_ENV` appears only at reversible.rs:58 and seats.rs:81; the FakePort in tests/scheduler/deadlines.rs does not implement `timed_retirements` (default `Ok(0)`); nothing drives the `?` at deadlines.rs:420.
  fix-shape hint: one DeadlineDriver test with a fake port (a pass runs on a due tick, and an error from it), and either use the expiry env var in a daemon test or remove it.
- [Nit] docs/programmatic-service-client.md:63 — The "Required order" list still says "Threads has no timer" and that a vanished service leaves seats frozen until an operator acts. That is now wrong for keyless seats. [lanes: hygiene-docs]
  verdict: confirmed 2-1 (reproduce ✓ / refute REJECT at FYI, outvoted / ground ✓)
  evidence: line 63 at head is unchanged; the same commit edited line 55 to say a keyless seat "is timed: what it is owed expires at 24 hours and it finalizes at 7 days".
    Dissent: the refute seat notes the list starts with ReserveContinuityKey, so its seats are keyed and the text holds for them. Both confirming seats concede this and still find the general statement literally false.
    Not checked by any seat: that herdr-graph reads this doc.
  fix-shape hint: add one clause scoping item 3 to keyed seats.
- [Nit] src/store/seats/reversible.rs:89 — Replaying an own `seat.retired_reversible` for a keyless seat into a timed row that then expires and finalizes is untested, although the design relies on derived clocks to cover this case. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: spec line 187 lists this test.
    `timed_row_sql!` (reversible.rs:89-97) requires `ts.is_local=1` and no continuity key.
    tests/store/reversible_facts.rs `restored()` tests check state, row state and keys; none runs `timed_due` or `timed_pass`, and none asserts `is_local`.
    Two seats rated Nit; no seat found an actual defect, and one notes `retirement_fact_row` suggests the seat is local on replay.
  fix-shape hint: extend a `restored()` test to advance the clock and run the timed pass on the replayed row.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- src/scheduler/deadlines.rs:420; src/store/seats/reversible.rs:259; :122; :149 — any error from the timed-retirement step propagates out of `timed_pass` and `drive_due`, so one failing row blocks later timed rows. Rejected 2-1.
  reason: all seats agree on the mechanism. The only identified trigger is the own-history hold refusing held kinds; it is capped at 5 minutes, the per-row transaction rolls back, and the row retries once the hold lifts.
  The finding names no persistent error. The unguarded `?` matches the adjacent `due_handoffs` call. The spec sets no latency bound.
  The ground seat's CONFIRM (Should-fix) rests on the same facts and concedes the failure is bounded and self-heals, so the dissent is on materiality only.
  Both rejecting seats name an optional hardening: skip `is_own_history_hold` in `timed_pass`, as receipts.rs:788 does.
- src/store/seats/operator.rs:143 — a v4 service `ResurrectSeat { replace: Some(NEW) }` finally retires NEW with no authority or freshness check on NEW. Rejected 3-0.
  reason: spec section 5 says "NEW must be resolved on P, as for the operator". The existing operator `--replace` has the same precheck and the same final retirement. "Fresh" describes the use case. The retirement is recorded under `service:<author>`.
- src/store/seats/reversible.rs:1177 — v3 FinalizeRetirement now finalizes a keyless reversible seat with no capability gate. Rejected 3-0.
  reason: spec section 5 (lines 132-134) states it: "the store does not see the session level, so this applies at every level". Keyless seats were never reversible before, so no v3 client was refused on one.
- src/store/seats/reversible.rs:96; src/store/seats/reversible.rs:89 — the timed clocks apply retroactively at upgrade with no grace period. Rejected 3-0.
  reason: spec section 1 (lines 33-38) names this population and says such a seat "now finalizes at 7 d instead of staying frozen until an operator acts". Only keyed rows whose key was released or lost can be affected. One seat did not read the migration file or TRUST-POLICY.md.
- src/store/seats/reversible.rs:285 — `record_expiry` settles against the payload's `expired_at` even when `INSERT OR IGNORE` kept an earlier one. Rejected 3-0 as a standalone finding.
  reason: all three seats confirm the mechanism and reject on the premise that no flow produces a second expiry with a different `expired_at` for one episode.
  That premise is contradicted by the confirmed finding at src/store/seats/publication.rs:607, where three seats show the home node re-expiring at E2 > E1 after an own-log replay.
  The mismatch is therefore carried as a consequence of that confirmed finding and its fix-shape hint, not as a separate task. Once that finding is fixed the precondition is gone.
- src/store/seats/reversible.rs:149; src/store/mod.rs:3115; src/store/seats/reversible.rs:851 — `timed_pass` ignores the CallBudget it is given. Rejected 3-0.
  reason: spec section 2 makes expiry "one transaction, not quanta" and accepts the cost. The pass is capped at 16 rows, each step is its own transaction, and `due_handoffs` is budget-blind in the same way.
- src/store/control/retirement.rs:170 — a store-only daemon idle-exits 10 minutes after the last pane closes, so expiry and finalization wait for a restart. Rejected 3-0.
  reason: spec section 8: "a daemon that is down past a deadline acts on its first tick after start, never earlier". An idle-exited daemon is a down daemon, and any CLI call or hook restarts it. An open requester pane keeps the daemon up.
- src/store/seats/reversible.rs:289 — obligation expiry leaves 'invited' memberships whose invitations are `recipient_retired`. Rejected 3-0.
  reason: spec section 2: "Expiry never touches memberships". Delivery predicates also require a pending invitation (lazy_delivery/direct.rs:11, lazy_delivery/mod.rs:45 and :199). The finding names no verified consequence.
- src/store/materialization.rs:737 — late-arrival settlement compares origin-clock creation time with the home `expired_at`, so a badly skewed origin can settle an obligation for a live seat. Rejected 3-0.
  reason: spec section 2, Late arrivals, judges "against the origin's clock" and accepts that skew "can land on either side". The finding concedes the impact is limited to skewed clocks.
- src/scheduler/deadlines.rs:420 — more than 16 due timed rows are not treated as a continuation. Rejected 3-0.
  reason: the spec sets no latency or throughput requirement for timed rows. The backlog drains at 16 rows per 5 s tick against clocks of 24 h and 7 d, and the predicate is re-derived on every pass.
- src/protocol/service.rs:438 — `SeatResurrected(SeatId)` carries no generation. Rejected 3-0.
  reason: docs/programmatic-service-client.md:57 documents the reply as `SeatResurrected(seat)` and says no binding opens. `RetirementFinalized(SeatId)` is the same shape. The wire-break argument is about a hypothetical later change.
- src/store/seats/reversible.rs:258 — `seat.obligations_expired` takes the home `expired_at` with no upper bound and expiry rows are never pruned. Rejected 3-0.
  reason: `expired_at` is the home node's own clock at the expiry instant. The spec accepts skew around it. A cap against the receiver's clock would break cross-node agreement and add the adversarial check AGENTS.md rules out. One seat did not check the migration comment or the consumers.

## Unverified nits (spot-checked)
- [Nit] docs/operations.md:304 — the "Reading the repair history" table has no rows for `operator_unretire`, `window_finalize` and `service_rebind`, and still says "the key's service" for `service_unretire`/`service_finalize`. (spot: CONFIRM Nit)
- [Nit] docs/agent-usage.md:183 — says an operator can bring a closed seat back "for 7 days"; the limit applies only to keyless seats. (spot: CONFIRM Nit)
- [Nit] tests/store/reversible_retirement.rs:1831 — the spec case "a service-required invitation is acceptable after a resurrect" is untested; the test only shows it stays pending. (spot: CONFIRM Nit)
- [Nit] src/protocol/service.rs:260 — no JSON round-trip fixture for `resurrect_seat` or `SeatResurrected`. (spot: CONFIRM Nit)
- [Nit] tests/store/control.rs:6669 — `left_seq` assertion loosened from the literal 2 to equality with `retired_seq`, which the same finalize path computes. (spot: CONFIRM Nit)
- [Nit] src/store/seats/reversible.rs:11 — the module header still says "Must not: ... settle an obligation" and omits `obligation_expiries` from Owns, while the diff adds `settle_expired`, `record_expiry` and `expire_obligations` there. (spot: CONFIRM Nit)
- [Nit] src/store/seats/reversible.rs:313 — `settle_expired` repeats the retirement job's per-obligation settlement from control/retirement.rs instead of sharing it. (spot: CONFIRM Nit)
- [Nit] src/store/seats/reversible.rs:58 — `TEST_OBLIGATION_EXPIRY_ENV` is used by no test, and both test clock constants are re-exported ungated at seats.rs:81. (spot: CONFIRM Nit)
- [Nit] src/service/dispatch.rs:585 — `service_resurrect` was inserted between `operator_host`'s doc comment and `operator_host`, so the doc now heads the wrong function. (spot: CONFIRM Nit)
- [Nit] src/service/kicks.rs:276 — `obligation_expiries` is added to `KNOWN_UNMAPPED` twice with the same comment. (spot: CONFIRM Nit)
- [Nit] src/store/seats/operator.rs:305 — the `service:{author}` label format is written in three places, and `UnretireBy` only rebuilds a label the caller already holds. (spot: CONFIRM Nit)
- [Nit] src/store/seats/operator.rs:187 — `reversible::row_state(tx, seat)` is read up to three times for one seat in `mutate_operator`. (spot: CONFIRM Nit)
- [Nit] src/store/control/retirement.rs:171 — the removed `if key_of_seat(...)` guard left a bare `{ ... }` block ending in an unconditional return. (spot: CONFIRM Nit)
- [FYI] src/store/seats/reversible.rs:971 — `UnretireSeat` still answers `Unauthorized` for a keyless seat while `FinalizeRetirement` accepts it. (spot: REJECT; spec D4 and section 5 keep `UnretireSeat` key-owner-only and route keyless seats to v4 `ResurrectSeat`)
- [FYI] src/store/seats/publication.rs:534 — after a restore-and-downgrade, the old binary may hold keyless reversible rows with no timer. (spot: REJECT; the spec has no downgrade requirement and recovery is `seat retire --operator`; the seat checked spec text only, not code)
- [FYI] tests/integration/pane_close_retirement.rs:116 — `closing_a_pane_with_two_bindings_retires_its_seat` asserts the reversible row before any deadline pass has run. (spot: REJECT; clock arithmetic is pinned by store tests and the daemon wiring by session_bindings_smoke.rs:293)
- [FYI] tests/integration/pane_close_retirement.rs:235 — `a_closed_tabs_seat_resurrects_onto_a_new_pane` rebinds before any timed pass has seen the row. (spot: REJECT; with the 7 d clock the 5 s tick cannot finalize the row, and dedicated store tests own the clocks)
- [FYI] src/store/seats/reversible.rs:235 — the 24 h expiry writes no seat-history entry and no read path exposes it. (spot: REJECT; spec section 6 keeps `seat inspect` at `retirement_state: reversible` and derives the clocks from `retired_at`)
- [FYI] src/service/dispatch.rs:599 — `ResurrectSeat` reports an instance mismatch as `StaleHostObservation`, not `InstanceMismatch`. (spot: REJECT; it follows the operator path's existing code at operator.rs:102, and the spec names neither code)
- [FYI] src/store/seats/operator.rs:196 — the operator resurrect of a keyed reversible seat is untested. (spot: REJECT; the new arm reads only the row state, and the spec's test list does not ask for a keyed variant)
- [FYI] src/service/dispatch.rs:621 — `ResurrectSeat` lets a service session make the daemon attach a Herdr server named by its `HostLocator`. (spot: REJECT; `Command::ResolveSeat` already attaches a caller-supplied locator on base before any owner-uid check)
- [FYI] src/store/seats/publication.rs:602 — a local-origin `seat.obligations_expired` for an unknown seat is a silent no-op. (spot: REJECT; `retirement_fact_row` has the same local-origin pattern on base, and a seat with no local row has no obligations to settle)
- [FYI] src/scheduler/deadlines.rs:420 — the count `timed_pass` returns is discarded, so the lane has no counter in DriveOutcome or Health. (spot: REJECT; the spec requires no signal for this lane and errors still propagate)
- [FYI] src/service/workers.rs:217; src/scheduler/deadlines.rs:420 — the timed step takes a background writer turn and an unindexed scan on every deadline pass. (spot: REJECT; same shape as `due_handoffs`, small per-call cost; the seat did not verify the batch size or the missing index)
- [FYI] tests/store/control.rs:7049; tests/store/control.rs:7040 — `review_operator_rebind_new_incarnation_then_absence` hand-builds a RetirementJob. (spot: REJECT; only `job.id`, read from the store, is passed to `advance_retirement`, so the made-up fields are inert)
- [FYI] src/store/seats/reversible.rs:62 — the `HT_TEST_*` clock overrides are read from the ambient process environment and `HT_` is not scrubbed. (spot: REJECT; many other `HT_TEST_*` knobs work the same way, and a stray export would fail the boundary assertions loudly)
- [FYI] src/scheduler/deadlines.rs:420 — smoke configurations now get final retirement effects from a later deadline-lane pass, not at close. (spot: REJECT; the pass runs on essentially every 1 s smoke tick against 60 s waits)
- [FYI] src/protocol/authority.rs:218 — `OperatorActor` carries an optional service identity, so one type stands for two principals. (spot: REJECT; design preference with distinct labels and scopes and no behavioural defect shown)
- [FYI] CHANGELOG.md:1 — no CHANGELOG entry for the migration, the new fact, the v4 level or the keyless-close change. (spot: REJECT; RELEASE.md line 24 puts the entry in the "Prepare vX.Y.Z" commit)
- [FYI] src/client/service.rs:560 — `PersistentServiceClient` still registers `service_session_v3`, so `ResurrectSeat` is unreachable through it. (spot: REJECT; the diff documents "a v4 caller registers its own session", and registering v4 would refuse v3-era daemons)
- [FYI] src/store/materialization.rs:741 — `project_receipt` and `insert_invitation` add two point queries per recipient on every send. (spot: REJECT; indexed lookups needed for the late-arrival rule, and no stated budget covers them)
- [FYI] migrations/node/0019_seat_resurrect.sql:31 — `obligation_expiries` is never pruned. (spot: REJECT; at most one row per retirement episode, growing no faster than the `seats` table it references)
- [FYI] src/store/mod.rs:260 — `begin_final_retirement_for_test` gets the job by calling `begin_retirement` a second time. (spot: REJECT; test-only helper relying on the documented retried-proof branch)
- [FYI] tests/store/fact_convergence_seat.rs:286 — the begin, finalize, begin-again shim is repeated across test files. (spot: REJECT; those fixtures hold a raw Connection or StoreContext, which the SqliteStore helper does not fit)
- [FYI] src/store/seats/reversible.rs:158 — `timed_pass` builds its `decide` closure inside the loop. (spot: REJECT; stylistic, and `execute_budgeted_decision` is not a drop-in)
- [FYI] tests/integration/pane_close_retirement.rs:227 — the resurrect integration test repeats the existing test's whole prelude in a serialized group. (spot: REJECT; one Session per scenario is the file's convention and the cost is unmeasured)
- [FYI] tests/integration/pane_close_retirement.rs:233 — the resurrect test sets up a second binding it never checks. (spot: REJECT; it reuses the shared helper, and the second binding strengthens the `open_bindings == 1` check)
- [FYI] src/store/control/retirement.rs:170 — an un-updated herdr-graph now sees keyless closed seats as `retirement_state: reversible`; correct behaviour depends on graph treating them as final. External claim. (spot: REJECT; spec section 6 states this as graph's own caution, and the worst case is `Unauthorized` or a seat that finalizes at 7 d; graph's code was not inspected)
- [FYI] src/store/control/retirement.rs:170; src/sync/facts/seat.rs:574; src/sync/facts/seat.rs:253 — peers that do not know `seat.obligations_expired` park it and keep the obligations pending. (spot: REJECT; spec sections 7 and 8 list this as an accepted limit and name the registry as the capability gate)
- [FYI] src/store/control/retirement.rs:170 — every keyless close now leaves obligations pending for 24 h and delays archival for 7 d. (spot: REJECT; this is D1, listed under spec section 8 accepted limits; the seat did not confirm the branch lacks a version marker)
- [FYI] src/store/control/retirement.rs:170 — the reversible-close behaviour has no config or kill switch. (spot: REJECT; the spec asks for none, and `seat retire --operator` finalizes at once)
- [FYI] migrations/node/0019_seat_resurrect_after.sql:3; src/store/schema.rs:181 — migration 0019 is one-way, so a code rollback needs a backup restore. (spot: REJECT; same pattern as deltas 12 and 16, with the documented installer backup)
- [FYI] src/store/seats/reversible.rs:1 — one 1652-line commit bundles the migration, the new fact, the v4 level, the close-semantics change, the timers and the CLI change. (spot: REJECT; the pieces implement one feature, and the spec requires the TRUST-POLICY update in the same commit)

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---