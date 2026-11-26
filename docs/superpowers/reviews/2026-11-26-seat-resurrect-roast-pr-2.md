---
super-roast verdict: Should-fix (9 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 2 of 3
profile (assumed): Not supplied; inferred from AGENTS.md and the packets, and unchanged from iteration 1. herdr-threads is a locally installed, single-user daemon and CLI for message threads between coding agents, with a cooperative same-user trust model and no adversarial caller verification. State lives in a local SQLite store with one-way migrations, nodes replicate facts to peers, and one external consumer (herdr-graph) uses the service-session API. Blast radius is small, but store convergence defects are hard to roll back and TRUST-POLICY.md is normative, so convergence defects keep Should-fix weight.
inputs: feat/seat-resurrect@7032973e4 vs 5a96ed277 (fix commits after PR roast 1)
delta vs prior: 6 new confirmed (0 Blocking) · 1 carried (0 Blocking) · 14 resolved · 2 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, hygiene-docs) · raw 12 → deduped 8 → panel 8 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 8 · rr 0.63 · rg 0.75 · fg 0.88 · unanimous 0.63 · ground-loo 1.00 (n=5) · reproduce 6/2/0 · refute 7/1/0 · ground 8/0/0
lane-yield (found/confirmed/unique/refuted): correctness 2/2/1/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 1/1/1/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 1/1/0/0 · data-migrations 0/0/0/0 · api-contract 1/1/0/0 · observability 0/0/0/0 · testing 3/3/1/0 · test-causality-lifecycle 2/2/0/0 · test-isolation-environment 0/0/0/0 · hygiene-docs 2/2/1/0
note: review readiness arrived as coverage unknown, provenance unknown, with an empty basis, so `[review readiness incomplete]` and the compatibility `[low coverage]` are appended; stage coverage itself is complete (lowCoverage false, no dead seats), and `[converged]` is not eligible. No severity floor applies and no severity was demoted on profile grounds; severities follow the confirming seats, by majority where they split. No default route was changed. The `[fix-regression]` tag on the first entry did not arrive from the engine; it is set from the packet's `lanes` field, which includes `regression`.
prior confirmed findings (17), status at 7032973e4: resolved on packet evidence (5) — src/service/dispatch.rs:2054 (decision guard taken in the transaction and held through commit; revoked-session test), src/store/seats/publication.rs:607 (`record_expiry` marks the home row; `NOT EXISTS` guard), src/store/control/invitation.rs:903 (terminal insert guarded on `expired.is_none()`), src/protocol/authority.rs:249 (`service_resurrect_with_replace_is_idempotent_per_operation_key`), src/store/seats/publication.rs:589 (`applied_expiry_fact_marks_the_home_row_and_the_lane_does_not_repeat_it`; parked replay recorded as deferred). Resolved on the gate's own grep of head, not re-surfaced by any scout and not seat-verified (9) — TRUST-POLICY.md:2513 and :2504 (limits rewritten), src/service/dispatch.rs:591 (v4 level and v3 refusal now a protocol test, `resurrect_seat_is_v4_and_round_trips`; the live-socket end-to-end test is recorded as not covered in the spec's implementation notes, lines 201-206), src/store/materialization.rs:737 (`a_receipt_first_projected_after_a_recorded_expiry_settles_at_write`), tests/store/reversible_retirement.rs:1795 (`expiry_warns_the_sender_of_an_obligation_overdue_at_the_close`), spec:176 (deferrals recorded), src/store/seats/operator.rs:311 (`next:` deviation recorded at spec line 108), src/store/seats/operator.rs:295 (`operator_retire` with `service:<author>` defined at TRUST-POLICY.md:828), docs/programmatic-service-client.md:63 (scoped to keyed seats). Regressed, fixed incompletely (2) — TRUST-POLICY.md:2508 and src/store/seats/reversible.rs:89, both entered below. Still-open (1) — src/scheduler/deadlines.rs:420, entered below. Three of the fixes introduced new confirmed findings: invitation.rs:971 (twice) and reversible.rs:196.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] src/store/control/invitation.rs:971 — The new `expired.is_none()` guard in `insert_invitation` also skips the terminal 'retired' `memberships` row for a finally retired invitee that has a recorded expiry. A node that receives the invitation late ends with no row; a node that held it ends with a 'retired' row. (new; introduced by the fix for the prior invitation.rs:903 finding) [fix-regression] [lanes: correctness, regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: the state is chosen first by `seat_state == "retired" && !is_frozen`, which gives `recipient_retired` for a finally retired seat.
    `expired` comes from `expired_episode`, a SELECT on `obligation_expiries`; that table is never pruned (only the `INSERT OR IGNORE` at reversible.rs:308 writes it), so `expired.filter(|_| state == "recipient_retired")` stays Some.
    The insert is now guarded by `state == "recipient_retired" && prior.is_none() && expired.is_none()`, so it is skipped.
    A holder node has an 'invited' row from `place_invitee`; the retirement job's audit phase (control/retirement.rs:517) runs `UPDATE memberships SET state='retired',... WHERE ordinal=? AND state!='retired'`, so it ends 'retired'.
    At base 5a96ed277 the guard was `state == "recipient_retired" && prior.is_none()`, so the late node wrote the same row.
    The insert's own comment states the invariant: "either arrival order ends with the same `memberships` row".
    All three seats rate Should-fix: a narrow convergence regression on one row, not data loss. No seat ran the code.
  fix-shape hint: skip the terminal insert only when the state came from the expiry rather than from a finally retired seat (for example, record which branch chose `recipient_retired`).
- [Should-fix] src/store/control/invitation.rs:971 — The fix for the prior invitation.rs:903 defect has no test. No test reaches the `prior.is_none()` branch with a recorded expiry, and no test reads `invitations.retired_at`, so reverting `&& expired.is_none()` or `expired.or(seat_retired_at)` fails nothing. (new) [lanes: testing] [fix-regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `seed_seat` (tests/store/reversible_retirement.rs:116-120) inserts `memberships('t', seat, 'joined', ...)` for every fixture seat, so `prior` is always Some.
    The late-invitation tests (`an_obligation_created_before_a_recorded_expiry_settles_at_write` ~2018, `recorded_expiry_settles_whatever_the_seat_state_and_is_idempotent` ~2073, `a_late_invitation_to_an_already_joined_seat_stays_already_joined` ~2511) all use such seats.
    `Fx::bare` (line 2552) deletes memberships but is called only at lines 2250 and 2477, in the service-replace tests.
    The `retired_at` reads at lines 1305, 1646 and 1843 are on seats, `receipt_state` and `obligation_expiries`, never on `invitations`.
    The same missing test would also have caught the finally retired regression in the entry above.
  fix-shape hint: with `bare` plus the expiry setup, materialize a late invitation and assert the membership row count and the invitation's `retired_at`, once for a resurrected seat and once for a finally retired one.
- [Should-fix] src/store/seats/reversible.rs:196 — A home timed row whose episode's `seat.obligations_expired` is applied before its `seat.retired_reversible` never finalizes: every pass computes `expire=true`, `expire_obligations` returns false under the new `NOT EXISTS(obligation_expiries ...)` guard, and the new `row.finalize && expired` gate skips finalization. The row also keeps its slot in the oldest-first `LIMIT 16` window. (new; introduced by the fix for the prior publication.rs:607 finding) [lanes: correctness]
  verdict: confirmed 2-1 (reproduce REJECT at FYI, outvoted / refute ✗-survived / ground ✓)
  evidence: `record_expiry` (~304-324) inserts `obligation_expiries`, then updates `reversible_retirements ... WHERE seat_id=?1 AND cutover_at=?2`, which matches nothing when the row does not exist yet.
    `insert_fact_row` (~667) later inserts the row with `obligations_expired_at` NULL and never reads `obligation_expiries`.
    In `timed_pass`, `expired = !row.expire || outcome("expiry", ...)` is then false on every pass. Nothing else reconciles the row: `materialize_seat_retired_reversible` calls only `insert_fact_row` and `kept_unretire`.
    All three seats agree on this mechanism and that both the guard and the gate are new in this diff.
    Requirement: spec line 60 (N7) says the rule "does not depend on the order facts arrive in"; the applier's comment says "arrival order and re-issue change nothing".
    Dissent (reproduce seat): no path produces that order. Both facts come from one origin, origin logs apply as a contiguous prefix by counter (sync/mod.rs `LogHead`), both need the same seat entity, and `HELD_KINDS` lists both.
    The confirming seats address it: both concede no delivery path is shown and that normal counter-order replay authors the expiry after the retire, and still confirm on the stated order-independence requirement. The dissent is on materiality only.
    Impact if reached: the seat stays reversible; an operator or service `FinalizeRetirement` still works.
  fix-shape hint: have `insert_fact_row` seed `obligations_expired_at` from a matching `obligation_expiries` row, or have the lane treat a recorded expiry as already expired; add an expiry-fact-first test.
- [Nit] TRUST-POLICY.md:3052 — The G5 accepted-limit text still says a target-absence retirement of a keyless seat "is final", while C2 (lines 556-558) and the code make every `Closed` retirement reversible. (regressed: the prior TRUST-POLICY.md:2508 finding named this clause, then at ~:3037; e885664c8 rewrote the :2508 bullet and left it) [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: TRUST-POLICY.md:3050-3052 at head: "For a seat with a continuity key that retirement is reversible, so its service can undo it; a keyless seat's is final."
    TRUST-POLICY.md:556-558: every `Closed` retirement, target absence included, is reversible, "a keyless one on the timed rules below".
    src/store/control/retirement.rs (~172-181) calls `reversible::insert_row` and returns `RetirementStart::Reversible` for `Closed`, with no key check.
    The refute seat also found the sibling restored-pane bullet at ~:3043 still saying the seat retires "(C2), irreversibly".
    Severity: two seats rate Nit (doc-only, no runtime effect); the refute seat rates Should-fix. The majority stands.
  fix-shape hint: rewrite the :3052 clause and the :3043 "irreversibly" to point at the timed keyless rules.
- [Nit] src/store/seats/publication.rs:534; tests/store/reversible_retirement.rs:2402 — The `timed_from` replay-window fix is wired by one call site, `materialize_seat_retired_reversible` passing `m.decision.utc` to `insert_fact_row`, and no test goes through it. The tests call `insert_fact_row` directly with their own instant, so passing `payload.retired_at` there fails nothing. (regressed: the prior Nit at src/store/seats/reversible.rs:89, replay into a timed row untested, was fixed incompletely; the `timed_from` mechanism itself is new) [lanes: testing, test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `insert_fact_row` (reversible.rs:667-675) stores `timed_from = MAX(retired_at, applied_at)`; `timed_due` and `expire_obligations` read `COALESCE(timed_from, cutover_at)`.
    `insert_fact_row` is called from tests only at tests/store/reversible_retirement.rs:2375 and :2402; `git grep materialize_seat_retired_reversible` under tests/ finds nothing.
    With the mutation, `timed_from` equals `cutover_at` and a row replayed two weeks after its close expires and finalizes on the first tick, before a later `seat.unretired` in the same restore can apply.
    All three seats rate Nit: the helper logic is tested and the gap is one argument at one call site.
  fix-shape hint: write the row through `materialize_seat_retired_reversible` (or the `restored()` fixture) with the decision instant at the replay time, then run the same `timed_pass` boundary checks.
- [Nit] docs/programmatic-service-client.md:57 — The service `ResurrectSeat` `replace` contract gained two refusals (`InvalidRequest` for a NEW with any `memberships` row, `Unauthorized` for a NEW keyed to another service), and the service-client doc and the TRUST-POLICY C1 table row do not mention them. Narrowed: the TRUST-POLICY C2 bullet does state the rule. (new) [lanes: api-contract, hygiene-docs]
  verdict: confirmed 2-1, narrowed (reproduce ✓ / refute REJECT at FYI, outvoted / ground ✓)
  evidence: docs/programmatic-service-client.md:57 at head: "`replace` names a fresh seat already resolved on `target`, which retires finally in the same decision"; it lists only `Unauthorized` for a keyed OLD and `NotFound`. The diff touches that file only at line 63.
    TRUST-POLICY.md:267 (C1, Service resurrect) says only "the operator resurrect's decision, through the same fresh target observation".
    Dissent (refute seat): TRUST-POLICY.md C2 (~line 611) says "A service's `replace` must name a bare NEW (no memberships) whose key, if any, is the service's own", so the normative policy was updated. Both confirming seats accept this and confirm only the service-client doc and C1 row gap.
    The membership check matches a row in any state ('invited', 'left'), per the finding; neither confirming seat re-read operator.rs.
  fix-shape hint: add one sentence to line 57 naming the bare-NEW rule and both refusals, and a clause to the C1 row.
- [Nit] src/store/seats/operator.rs:165; tests/store/reversible_retirement.rs:2199 — No test checks that the service decision guard is held through the resurrect transaction's commit, or that a guard whose author differs from the actor is refused. Dropping the guard right after the check passes every test. (new) [lanes: testing, test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: operator.rs:155-173 takes `service_decision_guard_in`, compares `guard.author()` with `actor.service_author()`, and stores the guard in `held`; `drop(held)` at line 429 runs after `execute_idempotent_transaction` returns. Production code is correct.
    The fixture `Gate` returns a guard that holds no lock and always reports the gate's own author. `RevokedGate` (line 2090) errors at acquisition, before any write.
    `service_resurrect_authority_and_trail` builds `stranger_gate` with the same author as the actor, so the mismatch branch is never reached.
    Replacing `*held.borrow_mut() = Some(guard)` with `let _ = guard;` restores the check-then-act race against `revoke_exact` and passes every test.
  fix-shape hint: use a mutex-backed gate in the style of `GenerationGate` (src/store/service_events.rs:645) and assert a second thread's `revoke_exact` returns only after the commit; add one author-mismatch case.
- [Nit] src/ports.rs:257 — `service_decision_guard_in` builds a `ServiceWriteTransactionProof` from any `&Transaction` and ignores the argument, so the proof's doc ("Only `ServiceDecisionTransaction::begin` creates this") is stale. `mutate_operator_guarded` hand-rolls the guard lifecycle with a `RefCell` outside `ServiceDecisionTransaction`, and the new function was inserted between that struct's doc comment and the struct. (new) [lanes: simplicity-design]
  verdict: confirmed 2-1 (reproduce REJECT at FYI, outvoted / refute ✗-survived / ground ✓)
  evidence: ports.rs:256 is `pub(crate) fn service_decision_guard_in(_transaction: &Transaction<'_>, gate, connection)`; its only caller is operator.rs:166.
    The ordering is correct today only because `execute_idempotent_transaction_presented` opens with `TransactionBehavior::Immediate` (schema.rs ~1202).
    The lines "Owns both the actual SQLite write transaction and authority guard..." now document the new function; `pub struct ServiceDecisionTransaction` has no doc.
    Dissent (reproduce seat): the proof's field is private, so only ports.rs can build it; the one call site is inside an Immediate transaction; the finding claims no broken behaviour. The confirming seats agree on all three points and confirm the stale doc, the misplaced comment and the invariant moving to caller discipline. That seat also concedes the misplaced comment is real.
  fix-shape hint: move the doc comment back onto the struct, correct the proof's doc, and optionally take an Immediate-typed or `ServiceDecisionTransaction` argument instead of an unused `&Transaction`.
- [Nit] src/scheduler/deadlines.rs:420 — Remainder of the iteration 1 finding: `TEST_OBLIGATION_EXPIRY_ENV` is still set by no test, and no DeadlineDriver-level test covers the `timed_retirements` call. (still-open, see iteration 1; not punch-listed, not re-surfaced by this round's scouts) [lanes: concurrency-async, testing, observability, test-isolation-environment]
  verdict: carried from iteration 1, where it was confirmed 2-1 and narrowed (reproduce ✓ / refute REJECT at FYI / ground ✓); not re-judged by a panel this round
  evidence: this status rests on the gate's own grep of head 7032973e4, not on seat evidence.
    `TEST_OBLIGATION_EXPIRY_ENV` appears only at src/store/seats/reversible.rs:58 and :75 and src/store/seats.rs:81.
    Under tests/, `timed_retirements` appears only at tests/store/reversible_retirement.rs:1621, through the store port; no fake-port test in tests/scheduler drives it. deadlines.rs:420 is still `self.port.timed_retirements(budget)?`.
    Part of the original finding was addressed: commit 7032973e4 says a failing timed row no longer fails the deadline drive, and `a_failing_timed_row_does_not_stop_the_pass` tests that at the store level.
  fix-shape hint: set the expiry env var in one daemon-level test or remove the constant and its re-export; one DeadlineDriver test with a fake port for the call.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- none

## Unverified nits (spot-checked)
- none

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---