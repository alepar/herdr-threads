# Service rebind of a restart-held seat (ht-0f0r)

Status: agreed with the coordinator 2026-11-26 (message mnSGeNxlZDVD); revised after design roast 1
(docs/superpowers/reviews/2026-11-26-service-restore-held-seat-roast-design-1.md). Requirements from herdr-graph: seat snSGuWQplkCA, thread dS9jx1CvnZb, messages mnSGwik5XhjI and
mnSGYFlLQ3RJ (relayed as mnSGs4KFcCzc). User (relayed): "herdr restarts need to be frictionless".

## Problem

After a new Herdr incarnation, every saved seat whose continuity is not structurally proven goes unresolved and
every unowned target in the baseline is held (TRUST-POLICY C2). A resumed session reattaches through cooperative
continuity (C1). A fresh, non-resume agent start in the held pane is refused until an operator runs
`seat rebind OLD --pane P --operator`. herdr-graph knows which seat it put in which pane and wants to put it back
before launching a fresh agent there. `ResurrectSeat` (v4) answers `NotFound`, because the seat is unresolved, not
reversibly retired.

## What the operator path already does

`seat rebind OLD --pane P --operator` on an unresolved OLD runs `store::seats::operator` (the same transaction
`ResurrectSeat` uses, through `IdentityRepair::operator_as_service`):

- a fresh target observation of P (`OperatorTargetGuard`, `observed_matches`, `consume_for_decision`);
- P must have no resolved owner (`target_free`), else the C3 refusal;
- OLD must be local and `unresolved` (or reversibly retired, which un-retires first);
- `rebind_unresolved_seat`: OLD resolves onto P with one generation step, its open bindings end
  (`supersede_ended_bindings`), its offers drop, an unavailability episode opens, the pane's key conflict row clears;
- `release_claimed_target`: P's recovery hold is released and its baseline release recorded, then
  `lift_baseline_hold_if_clear`;
- seat history `operator_rebind` with `operator:local-user:<uid>`; no binding opens, and the next lifecycle
  check-in in P enrolls onto OLD.

The service path differs only in the guard at the top of the seat check: for a service actor it demands a
reversible row (`NotFound` otherwise) and `require_v4_authority`.

## Decisions

| # | Decision |
|---|---|
| D1 | Shape: `ResurrectSeat` (v4, no new level, no new request field) also accepts an unresolved seat that qualifies under D2. The request is unchanged; the seat's state chooses the case. |
| D2 | Qualifies: OLD is local, unnamed, on a Herdr pane host (`HostKind::Herdr`), `unresolved`; the request's `host` and `target` are OLD's saved host and target (the pane it was resolved on); that target is held in the effective sense, `effective_recovery_disposition` answering `ExplicitHold` or `BaselineHeld` (an unreleased current-epoch `recovery_holds` row, or a member of the retained baseline while `baseline_hold_unclaimed` with no release; the headline case, a pane whose seat was still resolved at baseline capture, is held only the second way); the host's first reconciliation pass of the current recovery epoch has run (`reconciliation_lags` false); no kept C1 claim names the target, and none names a top-level session of OLD or carries OLD's key (`kept_claim_blocks`, the seat-keyed test restored-pane and absence retirement already use). |
| D3 | Refusals: an unresolved seat that is not on its saved target, or whose target is not held (D2), or a non-Herdr host: `NotFound` (nothing to restore; graph's answer for "not this case"). Lag, or a kept claim on the target or of OLD: `ServiceBusy` (retryable; the pass or the claim decides first). `replace` with an unresolved OLD: `InvalidRequest`. Authority: `require_v4_authority` unchanged (keyless: any v4 session; keyed: the key owner, `Unauthorized` otherwise). |
| D4 | Decision: exactly the operator's (`rebind_unresolved_seat`, `release_claimed_target`, `lift_baseline_hold_if_clear`) under the session's decision guard; seat history `service_rebind` with `service:<author>`, with no `service_unretire` before it. This mirrors the operator, whose rebind records `operator_rebind` alone for an unresolved seat and `operator_unretire` then `operator_rebind` for a retired one. |
| D5 | Race with a resumed session: if OLD is already `resolved` on exactly the requested host and target, the call succeeds as a no-op with the new result `SeatAlreadyResolved(seat)` and needs the same authority. Under lag it answers `ServiceBusy` instead: a resolved mapping may be a pre-restart one the pending pass will unresolve. Resolved anywhere else: `NotFound`. Store shape: the check closure skips the `target_free` refusal for this case; the deciding closure consumes the fresh-observation guard as for any rebind, then returns the new `CommandResult::OperatorAlreadyResolved(seat)` before its own `target_free` check, writing no seat row, history, hold or binding change. The idempotent transaction stores that result under the operation key like any other, `validate_result` accepts it for `Rebind` with the same seat, and a replay returns it. Only a service actor reaches it; the operator's rebind of a resolved seat is still refused. |
| D6 | After the rebind, the hold on P is released and, when no unresolved seat of that server remains, the baseline hold lifts. A pending kept claim cannot exist (D2). OLD's bindings ended at the rebind; a resumed session of OLD later checks in on a pane whose owner is OLD and enrolls as an ordinary check-in, as after `ResurrectSeat`. A session matching another seat finds P owned by OLD and is refused `TargetAlreadyOwned`, as at any owned pane. In-flight order (the hook read P as unowned or held, the rebind committed, then the hook's C1 claim arrived): the claim is refused `TargetAlreadyOwned`. Today the hook's arm for a pane with no seat reports that as a failure; it now treats it as the Resolved arm already does: a lifecycle event goes on to the ordinary resolver, which finds P's owner (OLD) and checks in; a turn event keeps its refusal and the next lifecycle or turn event takes the Resolved arm. A kept claim of OLD's at another pane cannot be pending at the rebind (D2). |
| D7 | Not touched: the 24 h / 7 d reversible machinery, the operator path, other unresolved seats saved at the same target (they stay unresolved and keep blocking the baseline lift until resolved or retired). |

## Trust argument

The hold exists so that ordinary resolution cannot put a fresh session onto a pane whose old seat may come back.
D2 puts the old seat itself back onto its own saved pane, on an explicit call by a service with authority over it;
the hold's purpose is met, not bypassed. The service's call is the evidence, as with `ResurrectSeat`, which
already lets the same authority bind a reversibly retired keyless seat onto *any* free pane. D2 is narrower in
target (own saved pane only, held only, Herdr only, after the pass, with no competing kept claim). The new
things are a C2 hold-release path and a C1 table row; no invariant weakens beyond what `ResurrectSeat` accepts.

Accepted limit (new, *Service restore of a held seat*): Herdr target ids are unique only within one incarnation;
after a cold restart the saved target id names whatever pane Herdr gives it. Herdr keeps pane ids across a cold
restart (graph's observation), but threads does not prove it: if Herdr reassigned the id to another pane, the
service's call seats OLD on that pane. The operator rebind carries the same limit; structural continuity paths do
not.

## Changes

- `store::seats::operator`: the service guard becomes: reversible row, else D2 (own target, hold, Herdr, no lag,
  no kept claim, no `replace`), else D5 no-op, else `NotFound`; authority unchanged. The D5 check runs before the
  `target_free` refusal.
- `protocol::service`: `ServiceResult::SeatAlreadyResolved(SeatId)`; `ResurrectSeat` doc; client
  `result_matches` arm. Service wire only (not the peer wire); a pre-3.6 daemon answers `NotFound` here as today.
- `store::operator::validate_result`: the `OperatorAlreadyResolved` arm; `protocol::commands::CommandResult` gains it.
- `service::dispatch::service_resurrect`: map `OperatorAlreadyResolved` to `SeatAlreadyResolved`.
- `cli::hook` (pane with no seat): `TargetAlreadyOwned` on the C1 claim of a lifecycle event goes to the ordinary resolver, as in the Resolved arm.
- TRUST-POLICY.md: amendment line; C1 *Service resurrect* row extended; C2 hold-release list gains the service
  restore; `service_rebind` provenance text; the accepted limit above.
- docs/programmatic-service-client.md, CHANGELOG.

## Tests (test-first)

Store-level, in `tests/store/reversible_retirement.rs` style fixtures:

1. Qualifying keyless seat, held both ways (an explicit hold row; and baseline-held only, the seat resolved at
   baseline capture and unresolved by the pass, no row): resolves onto its target, hold released, baseline lifts
   when it was the last, history `service_rebind` with `service:<author>` and no `service_unretire`, open bindings
   ended.
2. Each D2 miss refuses with its D3 code and writes nothing: other target, not held, named host, lag, kept claim on
   the target, kept claim of OLD at another pane, `replace`.
3. Keyed seat: owner succeeds; another author `Unauthorized`.
4. D5: seat resolved on that target answers `SeatAlreadyResolved` and writes no seat row or history; the same under
   lag answers `ServiceBusy`; resolved elsewhere `NotFound`.
5. Replay: the same operation key returns the first result, for the rebind and for the no-op.
6. After the rebind, a lifecycle check-in in P enrolls onto OLD (D6).
7. Hook in-flight order: the C1 claim of a lifecycle event in a pane read with no seat answered `TargetAlreadyOwned`
   goes on to the ordinary resolver and checks in onto P's owner.
8. Service session (tests/service): a v4 client gets `SeatAlreadyResolved` and the refusal codes over the wire.
   Dropped at implementation: the tests/service fixture host is unavailable, so a `ResurrectSeat` there stops at host
   routing before the store. The store results (tests 1-5), the client's `result_matches` arm and the wire round trip
   (`src/client/service.rs`, `src/protocol/service.rs`) are tested; the dispatch mapping is the three-line arm between.
