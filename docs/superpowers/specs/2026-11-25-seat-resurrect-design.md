# Resurrect a closed tab's seat (ht-eu3v)

Status: agreed with the user 2026-11-25; revised after design roast 1
(docs/superpowers/reviews/2026-11-25-seat-resurrect-roast-design-1.md). Requirements from herdr-graph: seat snSGwq67dzAh,
message mnSG0VnIFf9j.

## Problem

A keyless seat whose pane closes retires finally: the retirement job settles its pending ACK requests and
invitations as recipient-retired, warns their senders and retires its memberships. Reopening the tab gives a
fresh seat, and the old one's memberships (pair channels, topic threads) and history attribution stay with the
dead seat. Only a seat that holds a service-reserved continuity key retires reversibly (ht-ma6i.3), and only
the key owner can un-retire it.

## Decisions

| # | Decision | Source |
|---|---|---|
| D1 | A keyless `Closed` retirement is reversible, with two clocks: its obligations expire 24 h after the close, and the seat finalizes 7 days after it. `seat retire --operator` finalizes at once. | user ("hybrid"), graph (days, not minutes) |
| D2 | Resurrect is `ht human seat rebind OLD --pane P [--replace NEW] --operator`, which now also accepts a reversibly retired OLD. No new verb. | user |
| D3 | Explicit only. A resumed native session does not un-retire a seat by itself; that is a follow-up bead. | user |
| D4 | A service-session operation for graph ships in this change: un-retire and bind to a pane in one call, and finalize a keyless seat, behind `service_session_v4`. | user, graph (1) |
| D5 | Keyed seats keep today's contract: no timers, the owner un-retires and finalizes. The operator form of D2 also works on them. | design |

## 1. Retirement of a keyless seat

`begin_retirement_authored` is still the one place that chooses. A `Closed` retirement (same-incarnation close,
closure proof, restored-pane close, target absence, hc finality) of a seat with **no** continuity key now goes
reversible exactly as a keyed one does: the fence runs, a `reversible_retirements` row holds the job's inputs,
`seat.retired_reversible` is authored for a published seat.

**Timed rows.** A *timed* row is a local `reversible` row whose seat holds no continuity key. Its clocks are
derived, never stored: obligations expire at `cutover_at + 24 h` (`OBLIGATION_EXPIRY_MS`), the seat finalizes at
`cutover_at + 7 d` (`KEYLESS_FINALIZE_MS`). Deriving them means every way a local row comes to exist is covered:
the fence, a replay of the own `seat.retired_reversible` (no key, no closure inputs: TRUST-POLICY *Reversible
retirement*), and a keyed seat whose store was restored from its own log (keys are local and not in facts) or
whose owner released the key while it was reversible: such a seat now finalizes at 7 d instead of staying frozen
until an operator acts. A keyed row is never timed (D5). A peer's row is never timed: peers act only on facts.

Named seats are never `Closed` (unchanged). Operator retire and `--replace` stay final (unchanged).

While the row reads `reversible`, everything in TRUST-POLICY *Reversible retirement* holds unchanged: the seat
reads retired, sends to it are refused as retired, what it owes is frozen (skipped by the due scans), its
memberships and requirement episodes stay.

## 2. Obligation expiry (24 h)

**The rule.** An expiry of seat S at instant `expired_at` settles, as `recipient_retired`, every obligation *to* S
that is still pending and was created at or before `expired_at`:

- ordinary invitations to S: pending, not cancelled, not rejected, and **not service-required** (the existing
  `NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.created_decision_seq=
  i.created_decision_seq)` predicate of `control/invitation.rs`), so graph's required invitations stay pending
  (requirement (4));
- ACK-required receipts to S, physical and logical (the retirement job's `receipts` and `logical_receipts`
  selections).

Each settled obligation gets the overdue warning the retirement job writes, with the same warning basis. Expiry
never touches memberships, membership intervals or requirement episodes (the job's `audit` phase does not run).
The rule is a predicate, not a progress cursor, so it does not depend on the order facts arrive in (N7).

**Home node.** When the deadline lane finds a timed row with `cutover_at + 24 h <= now` and no
`obligations_expired_at`, one writer transaction re-checks that the row is still `reversible` for that episode,
sets `obligations_expired_at = now`, records `(seat, retired_at, expired_at)` in `obligation_expiries`, applies
the rule, and, for a published seat, authors `seat.obligations_expired {seat, retired_at, expired_at}`. It is
one transaction, not quanta: a resurrect commits wholly before or wholly after it, so no partly expired state
exists and no fact can be lost to an interleaving. The cost is one transaction proportional to the seat's
pending obligations to it (pending ACK requests and invitations, in practice a handful; accepted).

**Any node.** `seat.obligations_expired` (own-log replay included) records the expiry in `obligation_expiries`
and applies the rule to the node's own rows, whatever state the seat is in there (frozen, un-retired, or
finally retired with the job not yet past it). Re-applying settles nothing new, so a re-issued copy is a no-op.

**Late arrivals.** An obligation to S first written on a node (a window message or invitation from a node that had
not seen the retirement) with `created_at` at or before the latest recorded `expired_at` of S is settled at
write, by the same rule. So home and peers agree on which obligations expired, judged against the origin's clock
(an obligation created within clock skew of `expired_at` can land on either side, as for the un-retire shift).

After expiry the seat stays reversible: a resurrect still brings back its id, history and memberships; the expired
obligations stay settled (they did expire). The un-retire deadline shift applies only to what is still pending.

## 3. Finalization (7 d)

When the deadline lane finds a timed row with `cutover_at + 7 d <= now`, it finalizes it exactly as
`FinalizeRetirement` does: the deferred job is inserted with the original cutover and closure inputs,
`retired_seq` advances to this decision, the ordinary `seat.retired` is authored. The job then retires
memberships and writes audits; what expiry settled is already settled. Seat history records the decision with the
new provenance `window_finalize`.

## 4. Resurrect: operator form (D2)

`seat rebind OLD --pane P [--replace NEW] --operator` accepts an OLD that is `unresolved` (unchanged) or
`retired` with a `reversible` row (new). For the latter, one deciding transaction:

1. the precheck sees OLD retired with a `reversible` row (keyed or keyless) and the usual target checks;
2. `--replace NEW` retires NEW finally, as today;
3. OLD un-retires (`unretire_local`: unresolved, deadlines of still-pending obligations shift by their frozen time,
   new unavailability episode); seat history records `operator_unretire` with `operator:local-user:<uid>`;
   a published seat authors `seat.unretired`;
4. `rebind_unresolved_seat` places OLD on P (deletes the row), seat history records the rebind as today.

No binding opens; the next lifecycle check-in in P (a session start, `/clear`, resume, or a `launch` there)
enrolls onto OLD, as after any operator rebind.

**A live agent on NEW.** With `--replace NEW`, a running agent bound to NEW in P stays served as NEW's (now
retired) binding on its plain calls until its next lifecycle check-in (TRUST-POLICY *bindings disconnected
seat_retired never reconnect on a plain call*); nothing moves from NEW to OLD (C3). Implementation note: the planned
`next:` line in the command output is not added (no new model-visible text); `docs/operations.md` states the step
(restart the agent in P or run `/clear` there). Graph's case is a pane
with no agent (requirement (3)), so this only affects the operator fallback.

Refusals:
- OLD finally retired: `seat OLD retired finally; it can no longer be resurrected` (window closed or operator
  retire). The C3 guidance for a live NEW is unchanged.
- OLD `unretired` (a keyed seat its owner un-retired, waiting) reads `unresolved`, so the existing path applies.

The C3 refusal text for a rebind onto an owned target already names `--replace`, so it covers resurrect.

## 5. Resurrect: service form (D4)

New capability `service_session_v4`: the v3 operations plus `ResurrectSeat`.

```text
ResurrectSeat { seat, target: HostTargetId, host: HostLocator, replace: Option<SeatId>, operation }
```

- Authority: a keyed seat, the key owner (as `UnretireSeat`); a keyless seat, any registered v4 service session.
  This is the cooperative same-user model: a service that names a keyless seat answers for it; graph's evidence is
  that the seat is the recorded `threads_seat` of its clone. Recorded, not proven.
- Effect: the same transaction as section 4, with `service_unretire` (author recorded) instead of
  `operator_unretire`, and the rebind recorded as `service_rebind:<author>`. `replace` retires a fresh seat on P
  finally (graph's lost-response fallback: requirement (3)); NEW must be resolved on P, as for the operator.
- `FinalizeRetirement` also accepts a keyless reversible seat from any service session (graph's early finalize at
  purge). Implementation note: the store does not see the session level, so this applies at every level; the only
  change is that an `Unauthorized` answer for a keyless seat becomes success, and a keyless seat has no owner to protect.

## 6. Visibility

`seat inspect` already reports `retirement_state: reversible` for a reversibly retired seat and nothing for a final
one, which is what graph asked for. Implementation note: no clock fields are added, because `SeatInspection` decodes
with `deny_unknown_fields` and an older client would refuse them; the clocks are documented as `retired_at + 24 h`
and `retired_at + 7 d`. Graph treats reversible-keyless as final until its follow-up adopts `ResurrectSeat`
(graph caution).

## 7. Cross-node

No change to the existing facts. One new fact, `seat.obligations_expired` (authored by home only, applied on every
node, section 2). A node that does not know it parks it; its copies of the obligations stay pending (frozen, or
live again after a resurrect) until it upgrades and replays the parked fact, which then applies the rule; or until
`seat.retired` settles them, if the seat finalizes first. The registry is the capability gate, as for the
reversible kinds. Overdue warnings for expired obligations are written per node, as the final retirement job
writes them today.

## 8. Accepted limits (TRUST-POLICY additions)

- Senders learn of a closed keyless recipient's settled obligations up to 24 h later than today. Sends to it are
  still refused as retired at once.
- A peer that does not know `seat.obligations_expired` keeps the expired obligations pending until it upgrades
  (and replays the parked fact) or the seat finalizes. If the seat was resurrected, that is until the upgrade.
- Service-required invitations and requirement episodes of a closed keyless seat stay pending for up to 7 days
  (graph's requirement (4)); their service owns them and can release them.
- Expiry and finalization run on the home daemon's deadline lane: a daemon that is down past a deadline acts on its
  first tick after start, never earlier.
- A keyless seat's service authority is any service session (cooperative, same-user).
- A thread whose remaining members are all reversibly retired archives only after they finalize (7 d): a reversible
  seat stays a live member for archival (found in implementation).

## 9. TRUST-POLICY changes (same commit)

- *Reversible retirement*: keyless `Closed` retirements are reversible; timed rows and derived clocks; the expiry
  rule and late arrivals; timer finalization.
- C3: resurrect through `seat rebind` of a reversibly retired seat.
- Provenance table: `operator_unretire`, `window_finalize`, `service_rebind:<author>`; `service_unretire` and
  `service_finalize` now also cover keyless seats from v4 sessions.
- Decision record entry and the accepted limits above.

## 10. Tests (written first)

Each item is one named test.

- Store: keyless close goes reversible and timed; keyed close reversible and not timed.
- Expiry: after 24 h (injected clock) pending receipts and ordinary invitations settle with warnings;
  memberships, intervals and requirement episodes stay; a resurrect afterwards keeps the settled ones settled
  and the memberships effective.
- Expiry keeps a service-required invitation pending; it is acceptable after a resurrect.
- Resurrect just before and just after the expiry transaction: all or none of the obligations expired.
- A daemon down past both clocks: the first tick expires, then finalizes, in that order.
- Replay of an own `seat.retired_reversible` of a keyless seat yields a timed row that expires and finalizes.
- Late arrival: an obligation created before `expired_at` written after expiry settles at write.
- Finalize: at 7 d the job runs and the seat is final; a later rebind is refused with the closed-window error.
- Operator rebind of a reversible seat, with and without `--replace`; deadline shift; refusal on a final seat.
- Service `ResurrectSeat`: keyless by any v4 session, keyed by owner only, v3 session refused; `FinalizeRetirement`
  keyless from v4.
- Cross-node: `seat.obligations_expired` applies on a peer; applies after a `seat.unretired` that arrived first;
  re-issue is a no-op; parked by a registry without it, then applied on replay.
- Operator `--replace NEW` with an open binding on NEW: plain calls stay NEW's; a lifecycle check-in enrolls OLD.
- `ResurrectSeat` with `replace`, retried with the same operation id (lost response): one decision.
- Integration (private Herdr): close a pane, rebind the retired seat onto a new pane, an agent check-in there
  enrolls onto the old seat with its memberships.

Implementation notes (PR roast 1): the service decision runs under the session's decision guard and journals under the
session's operation scope (covered at store level: revoked session, replace retry); not covered by a named test, and
left to the existing mechanisms they reuse: `ResurrectSeat` end to end through a live v4 socket (the v4 level gate is
a protocol test), a parked `seat.obligations_expired` replayed after an upgrade across two nodes (the applier is order
independent and tested directly), and plain calls of a live agent on a replaced NEW (TRUST-POLICY binding rules,
unchanged).

Implementation notes (independent review, 2026-11-26): the expiry records the derived instant `cutover + 24 h`;
a row replayed from a fact runs its clocks from the replay (`timed_from`); a failing timed row is skipped, never
failing the deadline drive; a service `replace` needs a bare NEW it may act on; a late arrival expires only what would
otherwise be pending and gets no overdue warning on a non-authoring node (its authoring node warns at the expiry).
Residual accepted limit (6) in TRUST-POLICY: a restore whose later own entries stay unreachable past a full window.
