# Service rebind of a restart-held seat (ht-0f0r)

Status: draft. Requirements from herdr-graph: seat snSGuWQplkCA, thread dS9jx1CvnZb, messages mnSGwik5XhjI and
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
| D2 | Qualifies: OLD is local, unnamed, on a Herdr pane host (`HostKind::Herdr`), `unresolved`; the request's `host` and `target` are OLD's saved host and target (the pane it was resolved on); that target holds an unreleased C2 recovery hold; the host's first reconciliation pass of the current recovery epoch has run (`reconciliation_lags` false); no kept C1 claim names the target. |
| D3 | Refusals: an unresolved seat that is not on its saved target, or whose target holds no hold, or a non-Herdr host: `NotFound` (nothing to restore; graph's answer for "not this case"). Lag or a kept claim on the target: `ServiceBusy` (retryable; the claim or pass decides first). `replace` with an unresolved OLD: `InvalidRequest`. Authority: `require_v4_authority` unchanged (keyless: any v4 session; keyed: the key owner, `Unauthorized` otherwise). |
| D4 | Decision: exactly the operator's (`rebind_unresolved_seat`, `release_claimed_target`, `lift_baseline_hold_if_clear`) under the session's decision guard; seat history `service_rebind` with `service:<author>`, with no `service_unretire` before it. This mirrors the operator, whose rebind records `operator_rebind` alone for an unresolved seat and `operator_unretire` then `operator_rebind` for a retired one. |
| D5 | Race with a resumed session: if OLD is already `resolved` on exactly the requested host and target, the call succeeds as a no-op with the new result `SeatAlreadyResolved(seat)`, writes nothing but its journal row, and needs the same authority. Resolved anywhere else: `NotFound`. |
| D6 | After the rebind, the hold on P is released and, when no unresolved seat of that server remains, the baseline hold lifts. A pending kept claim cannot exist (D2). OLD's bindings ended at the rebind; a resumed session of OLD later checks in on a pane whose owner is OLD and enrolls as an ordinary check-in, as after `ResurrectSeat`. A session matching another seat finds P owned by OLD and is refused `TargetAlreadyOwned`, as at any owned pane. |
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
- `service::dispatch::service_resurrect`: map the no-op result.
- TRUST-POLICY.md: amendment line; C1 *Service resurrect* row extended; C2 hold-release list gains the service
  restore; `service_rebind` provenance text; the accepted limit above.
- docs/programmatic-service-client.md, CHANGELOG.

## Tests (test-first)

Store-level, in `tests/store/reversible_retirement.rs` style fixtures:

1. Qualifying keyless seat: resolves onto its target, hold released, baseline lifts when it was the last, history
   `service_rebind` with `service:<author>` and no `service_unretire`, open bindings ended.
2. Each D2 miss refuses with its D3 code and writes nothing: other target, no hold, named host, lag, kept claim,
   `replace`.
3. Keyed seat: owner succeeds; another author `Unauthorized`.
4. D5: seat resolved on that target answers `SeatAlreadyResolved` and writes no seat row or history; resolved
   elsewhere `NotFound`.
5. Replay: same operation key returns the first result.
6. Service session (tests/service): a v4 client gets `SeatAlreadyResolved` / the refusal codes over the wire.
