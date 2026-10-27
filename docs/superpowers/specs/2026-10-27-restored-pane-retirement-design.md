# Retiring unresolved seats whose restored pane closed (ht-wpq4)

Status: design, 2026-10-27, revised after design roast 1 (`docs/superpowers/reviews/2026-10-27-restored-pane-retirement-roast-design-1.md`: grace anchored on observed closure, observed-time-only grace, limits reworded). Based on main with ht-dtx0.

## Problem

On the laptop (2026-10-27) 15 seats stayed `unresolved`/`other` for 1-8 days after their panes and tabs were
closed (all of w3S, w8:pA/pC/p1, wY:p2/p6), with their restore holds, until the user retired them by hand.

Cause (verified): `plan_page` (`src/identity/reconcile.rs`) tests
`saved.state == Unresolved && !recoverable => MarkUnresolved` before the absent-target branch, and the applier
answers `Unchanged` for a seat already unresolved. A seat that a new Herdr incarnation unresolves (C2, reason
`other`) therefore never reaches `BeginRetirement`; only resolved and `host_invalidation` seats retire when their
target disappears. The hold lift waits for "no unresolved seats remain", so one such seat keeps every hold of its
server.

The naive fix (retire when the saved address is absent) is wrong: across incarnations the saved address and
terminal ids prove nothing (C2). The recovery baseline alone is not enough either: it is retaken on every new
recovery boot/epoch (every daemon restart and upgrade), so a pane that closed before the restart is absent from
the new baseline and leaves no trace.

## Decisions

### D1. Evidence rule: restored-pane closure

An unresolved seat with reason `other` and saved target A on Herdr server H retires when the pane that the
current incarnation showed at A, while A was held for repair, is gone from a later coherent snapshot of the same
incarnation, the seat's session has not claimed it, and a grace period has passed. Concretely, in one decision
transaction against the active publication P (A2):

1. **Watch record.** A durable watch row for the seat exists with P's boot and incarnation, target A, terminal T,
   and the seat's current mapping generation. It was written by a reconciliation pass whose publication showed A
   with terminal T while A was a member of that recovery's baseline with no release (held for repair: at the
   start of that recovery a pane was there and no seat had claimed it), and no other seat's structural proof or
   binding names T in this incarnation.
2. **Closure.** P is a coherent, complete publication of the same boot and incarnation, and shows neither target A
   nor terminal T anywhere (a moved pane keeps T and is not a close, TRUST-POLICY "Moved panes").
3. **No claim.** No kept continuity claim (ht-dtx0) on H, of any age, names the harness and native session of any
   top-level binding of the seat, and no other seat's structural proof or binding names T. Expired rows are pruned
   only after a refusal-free pass, so while passes are refused an old row may be a live session's only claim
   (review of ht-wpq4: a TTL bound here made retirement fail-unsafe during a long lag).
4. **Observed grace.** The daemon has observed the closure for at least `RESTORED_PANE_GRACE_MS` (10 min) of its
   own running time, counted only while H's reconciliation marker names the current recovery boot and epoch (C1
   claims are being decided; a lagging marker reads as Watching). Each pass that finds 1-3 true but the grace not
   yet served records the absence: it adds the
   time since the previous absent pass, capped at `ABSENCE_STEP_CAP_MS` (30 s, the lane's backoff cap), to
   `absent_observed_ms`. A suspend, a clock step or daemon downtime therefore adds at most one step, never the
   gap. The first absent pass adds nothing. The grace runs from the observed closure, not from the watch record.
5. **Seat unchanged.** The seat is still local, `unresolved`/`other`, at target A and at the watch row's
   generation (rebind, unresolve and retire bump it; a check-in does not).

What counts as evidence is the structural fact in 1 and 2: a terminal observed in a coherent snapshot of one
incarnation, then absent from a later coherent snapshot of the same incarnation. That is the class of evidence
the existing pane-close retirement already uses. The saved address only chooses which pane to watch; it is never
the evidence, and an address seen in another incarnation, or never held, starts no watch. 3, 4 and 5 make sure
retirement cannot race C1: a claim decided (5) or kept (3, live for its whole hour), or an idle mod watch or a
hook retry that reclaims within minutes of the closure (4). The grace is shorter than the kept-claim TTL because a
kept claim is checked directly; the grace only has to cover claims not yet made.

Rejected:
- *Absent saved address.* That is heuristic across incarnations (C2).
- *Baseline membership only.* The baseline is lost on the next daemon restart.
- *Grace alone.* An absent claim is not evidence that a pane closed.
- *Herdr's agent field.* It may only suggest candidates (C1).

### D2. Planning and applying

- `saved_seats_page` adds `restored_pane: RestoredPane` to `SnapshotSavedSeat` for `unresolved`/`other` seats:
  `None`, `Observe` (record a watch), `Watching`, `Absent` (closure seen, grace not served) or `Closed`. One store
  function, `restored_pane_evidence`, computes it and is called by both the page read and the applier.
- `plan_page`: for an `unresolved`/`other` seat, `Observe` → `WatchRestoredPane`, `Absent` →
  `NoteRestoredPaneAbsent`, `Closed` → `RetireRestoredPane`, otherwise the current no-op `MarkUnresolved`.
- The applier re-derives the evidence inside its transaction and refuses (`Stale`) unless it still holds.
  `WatchRestoredPane` upserts the row (one per seat). `NoteRestoredPaneAbsent` advances `absent_observed_ms` and
  `absent_seen_at` (D1.4). `RetireRestoredPane` calls `begin_retirement_authored` with
  `RetireReason::Closed` (unchanged on the wire), closure boot, epoch and target from P, and closure generation
  from the watch row. It sets `retirements.closure_evidence = 'restored_pane_closed'`, deletes the watch row and
  runs `lift_baseline_hold_if_clear`.
- Only coherent publications plan this, and each publication runs a full pass, so retirement follows within one
  capture cadence after the grace ends.

### D3. Provenance

Schema delta `0009_restored_pane_retirement.sql`:

- table `restored_pane_watch(seat_id PK, host_id, host_boot, incarnation, target_id, terminal_id,
  target_generation, seat_generation, recorded_at, absent_seen_at, absent_observed_ms)`: local, never
  replicated. Stale rows (another incarnation or generation) are ignored and replaced by the next watch, and a
  retirement deletes its row;
- column `retirements.closure_evidence TEXT NULL CHECK(closure_evidence IN ('restored_pane_closed'))`. NULL
  means every existing path: same-incarnation pane close, operator, replicated.

The column is the record; `seat inspect` does not show it, because `RetirementStatus` is `deny_unknown_fields`
and an older CLI would refuse a newer daemon's inspect reply (a follow-up can add it behind a capability). The
`seat.retired` fact keeps `reason: closed`. No new wire value, no new receipt or binding provenance, no new
model- or operator-visible string.

### D4. Test override

`HT_TEST_RESTORED_PANE_GRACE_MS` (test-support builds only) overrides the grace, as
`HT_TEST_ACCEPT_CHANNEL_GRACE_MS` does.

## Acceptance criteria

1. A seat unresolved by a new incarnation whose restored pane then closes retires with
   `closure_evidence = restored_pane_closed` once the grace has passed, and its restore hold lifts when no
   unresolved seat remains on that server.
2. The same seat whose pane still exists stays unresolved, and a later C1 claim, including a kept claim decided
   after reconciliation, reattaches it.
3. A seat with a kept claim for one of its sessions, or whose session claims during the grace window, is never
   retired by this rule. The same holds for a seat whose watched pane closes before the grace ends and whose
   session then claims (C1 bumps the generation).
4. Absence within the grace does not retire; the first pass after the grace has been observed does. A gap between
   passes (suspend, daemon down) counts at most `ABSENCE_STEP_CAP_MS`.
5. The watch survives a daemon restart within the incarnation, and closure after the restart still retires.
6. A pane present only in a different incarnation, a target not in the baseline or released, and a moved pane
   (T at another target) never retire.
7. `host_invalidation` and resolved seats behave exactly as before, and operator retirement is unchanged.
8. TRUST-POLICY.md states the rule (C2 pane-close retirement) and the accepted limit in the same commit.

## Accepted limits

- **A seat whose session lives elsewhere can lose it.** The seat retires, irreversibly, when the watched pane
  closes and, during the 10 observed minutes after, no claim rebinds the seat and none for its sessions is kept.
  That includes a claim refused outright (for example the session first enrolled a fresh seat in its new pane,
  then `/resume`d: an owned target), a kept claim decided as a refusal or pruned undecided once past its TTL, and person seats, which have no C1 path.
  Such a session's next C1 claim finds no match and goes through ordinary guards.
- **The watched pane is whichever pane the current baseline held at A.** The baseline is retaken after every
  daemon restart inside one incarnation, and the watch needs only that A was in it, unclaimed. So the watched pane
  need not be one Herdr restored: when nothing was restored at A, or the restored pane was gone before the
  restart, a pane Herdr later issued at A and held by the retaken baseline is watched, and its close retires the
  seat.
- **Terminal ids.** The rule relies on a moved pane keeping its terminal id (observed on Herdr 0.9.1-0.9.5,
  "Moved panes") and on terminal ids not being reused within one incarnation (assumed, as the existing pane-close
  rule does).
- **Closures not observed are not retired.** A seat whose pane closed while no publication of that incarnation
  recorded a watch for it, for example before this release was installed or while the daemon was down, stays
  unresolved for the operator, as today. In particular, the laptop seats from before this fix need a manual
  retire.

## Proposed CHANGELOG line

- Seats left unresolved by a Herdr restart now retire on their own when the pane Herdr restored at their address
  is closed and their session has not reclaimed them within 10 minutes; their restore holds lift as before
  (ht-wpq4).
