---
super-roast verdict: Should-fix (7 confirmed) [low coverage] [review readiness incomplete]
mode: design        iteration: 1 of 3
profile (assumed): herdr-threads is a Rust CLI and local daemon (SQLite store) that carries durable message threads between coding agents in one user's Herdr panes. The trust model is cooperative and same-user, with no adversarial caller verification, no external users and no money at stake. It syncs between that user's own nodes over a signed fact log and ships versioned releases, so mixed-version peers and restored stores are real but uncommon cases. The project holds strict test-first and TRUST-POLICY discipline, so spec gaps in stated invariants and in the test list are costly, while resilience against rare node-level faults is down-weighted.
inputs: docs/superpowers/specs/2026-11-25-seat-resurrect-design.md
coverage: scouts 8/8 (premortem, completeness, yagni, failure-mode, feasibility, domain:distributed-systems, domain:identity-and-trust, domain:state-machine-lifecycle) · raw 116 → deduped 55 → panel 33 · spot 22 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 33 · rr 0.82 · rg 0.61 · fg 0.42 · unanimous 0.42 · ground-loo 0.52 (n=27) · reproduce 7/26/0 · refute 1/32/0 · ground 20/13/0
lane-yield (found/confirmed/unique/refuted): premortem 18/3/0/15 · completeness 19/3/0/16 · yagni 11/1/0/10 · failure-mode 12/3/0/9 · feasibility 15/4/0/11 · domain:distributed-systems 11/3/0/8 · domain:identity-and-trust 15/2/0/13 · domain:state-machine-lifecycle 15/4/1/11

Review readiness was not supplied by the engine (coverage unknown, provenance unknown, empty basis). The `[low coverage]` and `[review readiness incomplete]` tags reflect that only. No pipeline stage was lost: 8/8 scouts returned and judge completion is 100%.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] §2 Obligation expiry (reused `invitations` phase; "pending ordinary invitations"; "requirement episodes ... stay as they are") — Expiry reuses the retirement job's invitations phase, which has no filter for service-required invitations. At 24 h it would settle them as `recipient_retired` while their requirement episodes stay pending. That breaks graph requirement (4).
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: `src/store/control/retirement.rs:445` selects `FROM invitations i WHERE i.seat_id=?1 AND i.thread_id=?2 AND i.state='pending'` and line 451 sets `state='recipient_retired'`. It has no `requirement_episodes` exclusion.
  evidence: Only the audit phase (line 511) retires `requirement_episodes`, and expiry skips it. The episode would read pending while its invitation reads `recipient_retired`.
  evidence: `src/store/control/invitation.rs:612` and `:712` already carry `NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id ...)`.
  evidence: §10 tests only that "requirement episodes stay", not that their invitation rows stay pending.
  evidence: The refute seat conceded the code premise and rejected only on the grounds that the intent is stated and the filter is an implementer detail.
  fix-shape hint: Name the ordinary-only predicate for the expiry-mode invitations phase in §2. Add a §10 test that a service-required invitation is still pending after 24 h and acceptable after a resurrect.
- [Should-fix] §2 ("When the quanta finish, the row records `obligations_expired_at`, and a published seat authors `seat.obligations_expired`"; "a resurrect in between commits first or after, never interleaved"), §7, §8 — A resurrect that commits between expiry quanta leaves the home node with part of the episode's obligations settled and no `seat.obligations_expired` authored. Peers keep those obligations pending. The divergence is permanent.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: §2 authors the fact only when the quanta finish. §4 step 4 deletes the row on resurrect, so the remaining quanta stop and the fact is never written.
  evidence: Peers receive only `seat.unretired`, which shifts their still-pending copies. Home holds the same obligations settled ("the expired obligations stay settled").
  evidence: §2 makes a late fact "a no-op when the episode is not frozen on that peer (already un-retired...)", so it cannot repair the split.
  evidence: The §7 and §8 limit ("settles them only at finalization (7 d)") does not cover this. After a resurrect no finalization happens.
  evidence: All three seats confirmed. The refute seat searched the spec for quanta, partial and interleav and found no rule that completes, rolls back or publishes a partial expiry.
  fix-shape hint: Have the resurrect transaction publish what the committed quanta settled, or make it finish or abort the expiry atomically, or make expiry a single transaction. Add a resurrect-between-quanta test.
- [Should-fix] §4 ("`--replace NEW` retires NEW finally, as today"; "No binding opens; the next agent check-in in P enrolls onto OLD"), §5 `replace` — The spec does not show that a live agent bound to NEW in P enrolls onto OLD after `--replace`. Existing policy says its plain calls keep attributing to the retired NEW.
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: `TRUST-POLICY.md` lines 858-866: a binding disconnected `seat_retired` "never reconnects on a plain call: the call is served as that binding (attribution and cursor) and its liveness is unchanged".
  evidence: Only a lifecycle check-in (startup, /clear, resume) enrolls a new binding. Until then the running agent is neither OLD nor a third seat, and the resurrect does not take effect for it.
  evidence: C3 (`TRUST-POLICY.md` line 681) says "nothing moves from NEW to OLD", so the never-merge rule is not violated. The gap is the unstated live-agent behaviour and operator step.
  evidence: §10's integration test uses a fresh pane with no prior seat. The replace-with-live-agent path is untested.
  evidence: The refute seat's objection is that `--replace` is existing, unchanged behaviour. The two confirming seats agree on that and confirm on the spec's unqualified claim and missing test. They split Should-fix and Nit. Should-fix is kept because a reopened tab with an agent already enrolled is the feature's main case.
  fix-shape hint: State in §4 and §5 that a live agent on NEW stays attributed to NEW until its next lifecycle check-in, or refuse a NEW with an open binding. Name the operator step. Add the §10 test from the finding's spike.
- [Should-fix] §1 ("The row also records its clocks: expires_at = cutover_at + 24 h ... finalizes_at = cutover_at + 7 d", set only at `begin_retirement_authored`) — A keyless reversible row recreated by own-log replay gets no clocks. It never expires and never finalizes, and it has no key owner. It stays frozen until an operator acts.
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: `src/store/seats/publication.rs:533` (`materialize_seat_retired_reversible`) calls `reversible::insert_fact_row(tx, seat, retired_at, reason)`.
  evidence: `src/store/seats/reversible.rs:233` inserts only seat_id, state, cutover_at and reason. It writes no clocks and no closure inputs.
  evidence: `TRUST-POLICY.md` line 589: "A local row replayed from the own log takes the fence's or un-retire's transition, with no key and no closure inputs."
  evidence: §2 and §3 fire only on `expires_at <= now` and `finalizes_at <= now`. NULL never matches, which contradicts D1 ("finalizes 7 days after").
  evidence: The spec has no rule for replay, and none for what a replayed `seat.obligations_expired` does.
  evidence: The refute seat's objection: `seat retire --operator` and v4 `FinalizeRetirement` can still finalize, and restore is rare. The confirming seats split Nit and Should-fix. Should-fix is kept because the stuck state is silent and unbounded, and the spec fix is one rule.
  fix-shape hint: Say that replay derives both clocks from `retired_at` for a keyless home row, and whether a replayed `seat.obligations_expired` sets `obligations_expired_at` or reruns the quanta. Otherwise list the case as an accepted limit.
- [Should-fix] §10 Tests — The test list omits the riskiest race and failure paths that the spec itself makes claims about. [lanes: premortem, completeness]
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: §10 has one test per clock, the operator and service forms, and three cross-node cases (apply, no-op after un-retire, parked).
  evidence: No test covers a resurrect between quanta, although §2 claims "never interleaved" and that the quantum "checks the row is still `reversible`".
  evidence: No test covers a restart mid-expiry, or both clocks overdue on the first tick (§8 "acts on its first tick after start").
  evidence: No test replays `ResurrectSeat` with `replace` (§5 lost-response fallback).
  evidence: No test covers `seat.obligations_expired` arriving before the episode's retire fact (§7).
  evidence: The refute seat's objection: §10 is a minimum list and these paths inherit existing mechanisms. The confirming seats split Nit and Should-fix. Should-fix is kept because the untested paths include the confirmed defect paths above and the list is labelled "written first".
  fix-shape hint: Add one named test per missing path to §10. Several fall out of the fixes to the findings above.
- [Nit] §7 ("its copies of the episode's obligations stay frozen until `seat.retired` at 7 d (accepted limit)"), §8 second bullet — On a peer that parks `seat.obligations_expired`, a resurrect after expiry leaves the obligations settled on home and pending on the peer permanently. The accepted limit states only a 7 d delay.
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: `TRUST-POLICY.md` lines 575-592: a peer applying `seat.unretired` "restores remote, shifts the peer's own copies of the seat's pending deadlines ... and deletes the row".
  evidence: No `seat.retired` follows a resurrect, so the peer's copies never settle. A later replay of the parked fact is a no-op under §2.
  evidence: The refute seat's objection: version skew is already gated by the registry, and the home node is authoritative for the seat.
  demoted: the seats split Should-fix and Nit. The profile is a same-user cooperative tool, and this needs a peer on a version without the fact kind plus a resurrect after 24 h. The minimum fix is a wording correction to the limit.
  fix-shape hint: Correct the §8 limit to say the peer never settles those obligations after a resurrect, or reuse the between-quanta fix so the resurrect publishes the settlement.
- [Nit] §2 ("it is a no-op when the episode is not frozen on that peer (already un-retired or finalized)"), §7 — `seat.obligations_expired` is order-dependent, contrary to the N7 per-episode rule. It is lost if it arrives before `seat.retired_reversible` or after `seat.unretired`. The spec gives no keep-and-replay rule.
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: `TRUST-POLICY.md` around line 584: "Per episode the outcome does not depend on arrival order or re-issue (N7)". An early `seat.unretired` is kept in the journal and replayed.
  evidence: The spec has no such rule for the new fact and does not say whether the journal-mark episode guard covers it. §10 tests only "no-op after an un-retire".
  evidence: The refute seat's objection: home authors expiry after the retire and per-origin journal order normally prevents reordering. The early-arrival case degrades to the §8 limit.
  evidence: Both confirming seats rated it Nit. The one unbounded case is unretire-then-expire on a peer.
  fix-shape hint: Keep an early expiry in the journal as `seat.unretired` is kept, or state that per-origin ordered delivery is assumed. Say whether the episode guard covers the new kind. Add a reorder test.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- §2 expiry quanta: no durable cursor, job identity or warning time basis named (2 REJECT / 1 CONFIRM Nit).
  reason: Expiry settles only what is still pending, so a restart re-scans what is left. The spec promises no cursor-exact resume. Cursor and warning basis are implementation choices. The confirming seat did not read `retirement.rs`.
- §1, §7, §8: keyless closes now author `seat.retired_reversible`, which older peers park for up to 7 days (3 REJECT).
  reason: `TRUST-POLICY.md` (~line 2479) already has the accepted limit "A node that does not know the reversible kinds (ht-ma6i.7)". §7 cites the registry gate. Only the frequency changes. A sentence in §8 is optional.
- §5 `ResurrectSeat`: no reply shape, error set or replay semantics (3 REJECT).
  reason: `operation` is the existing service `OperationId`. `src/protocol/service.rs:537` and `TRUST-POLICY.md` (lines 794, 1527, 1581) define journaled exact-operation replay, which returns the stored result before the precheck.
- §2, §3, §8: no order or interlock when expiry and finalization are due together (3 REJECT).
  reason: Finalization deletes the reversible state that each quantum checks. The finalization job runs the same settle phases. A late fact is a no-op on a finalized episode. The end state is the same in every order.
- §3: the 7 d finalize racing a resurrect has no stated re-check (3 REJECT).
  reason: §3 finalizes "exactly as `FinalizeRetirement` does". `finalize` in `src/store/seats/reversible.rs` reads the row inside one writer transaction and fails with `no_retirement` once a resurrect has deleted it.
- §6: reversible and final are distinguished only in CLI `seat inspect`, not in a service-session operation (3 REJECT).
  reason: The spec states no service-level inspect requirement. `ServiceOperation` has no seat-inspect operation. §6 says graph treats reversible-keyless as final until it adopts `ResurrectSeat`.
- §2, §7, §8: the home node never returns, so peers stay frozen indefinitely (2 REJECT / 1 CONFIRM Nit).
  reason: Seat facts are home-authored for every kind. A permanently lost home is a node-level failure outside this change. A peer-side timer would have peers decide seat state. One line in §8 is optional.
- §1, §8: wall-clock deadlines with no guard against clock steps, suspend or downtime (2 REJECT / 1 CONFIRM Nit).
  reason: D1 defines calendar windows from the close and §8 accepts first-tick action. The observed-grace rule (`TRUST-POLICY.md` line 608) guards a heuristic closure decision, not a retention deadline. A forward clock step is an operational fault under the same-user model.
- §2, §8: the `seat.obligations_expired` payload carries no obligation set or bound (2 REJECT / 1 CONFIRM Nit).
  reason: Each node applies the phases to its own copies, as `seat.retired` already does. §8 bullets 2 and 3 accept the window-fact and peer-lag cases. No spec text promises identical settled sets.
- §2, §3: deadline-lane behaviour for an unpublished seat or under the restore-window hold (2 REJECT / 1 CONFIRM Nit).
  reason: A refused tick leaves the row matching, so the next tick retries. The hold's `ServiceBusy` abort and `emit_if_published` in `src/store/control/retirement.rs` are existing behaviour that the reused path inherits.
- D5, §4 step 1: operator un-retires a keyed seat without the key owner; key fate and owner's later calls unstated (2 REJECT / 1 CONFIRM Should-fix).
  reason: Steps 3 and 4 do not touch the key. The existing continuity-key tables decide a key-presented reattach. The owner's later `UnretireSeat` gets the existing not-found and `FinalizeRetirement` gets `no_retirement`. The dissenting seat cited the same policy lines (558-559, 570) that the majority answered. A clarifying sentence and a keyed-operator test are optional.
- §9: TRUST-POLICY *Pane-close retirement* and *hc host closure* are not updated (2 REJECT / 1 CONFIRM Nit).
  reason: Those paragraphs are already imprecise for keyed seats since ht-ma6i.3. The Reversible retirement section is the controlling text and §9 updates it. A cross-reference is optional.
- §1: thread or group send behaviour for a reversibly retired member is unspecified (3 REJECT).
  reason: `TRUST-POLICY.md` lines 547-600 define it: window facts write frozen pending rows, direct sends are refused, the seat stays a live member. §1 inherits this unchanged. Expiry bounds the frozen receipts to 24 h.
- §4, §5, §7: the node on which a resurrect may run is unstated (2 REJECT / 1 CONFIRM Nit).
  reason: The spec reuses the existing `unretire_local` and `rebind_unresolved_seat` paths and changes no fact authoring. Pane P is local to the deciding daemon. The spec introduces no peer-side resurrect.
- §1, §4, §5: accepted target kinds and the cross-kind rule are unstated (2 REJECT / 1 CONFIRM Nit).
  reason: "The usual target checks" delegates to the existing rebind path. The service form takes a generic `HostLocator`, so hc seats have a route. A refused cross-kind target is safe and the seat finalizes at 7 d.
- §2: how a peer drives the expiry quanta is unstated (3 REJECT).
  reason: `materialize_seat_retired` in `src/store/seats/publication.rs` already enqueues a durable replicated job (`insert_replicated_retirement_job`) that runs the same budgeted phases and sets `retired_seq` from the fact.
- §1, §10: no schema migration or downgrade story (3 REJECT).
  reason: `src/store/schema.rs` (~line 428) refuses a store whose `user_version` is newer than the binary ("store is newer than this binary"). The new columns need a delta that bumps it, so an older binary never opens the store.
- §5, §8: any registered v4 service session can resurrect or finalize any keyless seat (3 REJECT).
  reason: §5 and §8 state and accept this under the cooperative same-user model. AGENTS.md forbids adversarial caller verification. Early finalize only brings forward the 7 d timer. Provenance is recorded in seat history.
- §5: service `replace` does not check that NEW is fresh or empty (2 REJECT / 1 CONFIRM Nit).
  reason: §5 is "the same transaction as section 4" and inherits the operator form's NEW checks, including the C3 guidance for a live NEW. "Fresh" is descriptive. A one-line clarification is optional.
- D3, §1: automatic continuity paths acting on a reversibly retired keyless seat are not checked (3 REJECT).
  reason: Process and shell continuity act only on `unresolved` seats (`TRUST-POLICY.md` lines 255-256). Line 531 says a retired seat's fingerprint row, "including a reversible seat's, is no rival". The paths key on seat state, not on the key.
- §2, §8: `receipts` and `logical_receipts` treat window facts differently under expiry (2 REJECT / 1 CONFIRM Nit).
  reason: The seats agree on the mechanism (physical phase unfenced, logical phase fenced by `retired_seq`). The asymmetry already exists in the finalization job. Every such obligation settles by 7 d, which §8 accepts. A sentence and a test are optional.
- §2: expiry "over the seat's memberships" may miss obligations outside current memberships (3 REJECT).
  reason: `src/store/control/retirement.rs` enumerates `memberships WHERE seat_id=?1 AND ordinal>?2 AND ordinal<=?3` with no state filter. The phases key on (seat_id, thread_id). Expiry has the same coverage as the final job.
- §1: a keyless `Closed` close of an unpublished seat is unspecified (3 REJECT).
  reason: The policy sentence ("an unpublished seat authors nothing, and the restore-window hold refuses both as retryable") is about authoring facts, not about refusing the close. "Exactly as a keyed one does" gives a reversible row with no fact.
- §2, §4 step 3: senders warned at 24 h are never told of a resurrect (2 REJECT / 1 CONFIRM Nit).
  reason: §2 keeps expired obligations settled on purpose and §8 accepts the notice delay. The warning was true when written. Senders can resend after the resurrect. A sender notice would be a new feature.
- §4, §6: the operator has no way to find OLD's seat id (3 REJECT).
  reason: `seat list --include-retired` exists (`src/cli/commands.rs`, `SeatSub::List`) with `--label`, `--harness`, `--node` and `--on` filters. `SeatsQuery` has a target filter that includes retired seats.
- §1, §2, §8: deferring the audit phase keeps memberships live for 7 days and delays archival (2 REJECT / 1 CONFIRM Nit).
  reason: Keeping memberships is the purpose of resurrect and follows from D1. §3 states when memberships retire and audits are written. The delay is bounded and loses nothing. One §8 bullet is optional.

## Unverified nits (spot-checked)
- [FYI] §2 "A peer applies it ... the same phases" — the spec does not say whether a peer writes its own overdue warnings, so senders may see duplicates across nodes.
  spot check: refute seat REJECT. Warnings attach to each node's own settled copy and the spec never says warnings replicate.
- [FYI] §1 `expires_at`, §2 `obligations_expired_at` — per-episode clocks and expiry marks after a resurrect and a re-close are not spelled out.
  spot check: refute seat REJECT. The rebind deletes the row, a re-close creates a new one, and the fact's `retired_at` is the episode key.
- [FYI] §1 "(keyless rows only; NULL for keyed)", §3 — timers for a keyed seat whose key was dropped, and the clocks on peer-side rows, are unstated.
  spot check: refute seat REJECT. §2, §7 and §8 keep timers on the home node, and D5 keeps keyed seats on today's contract.
- [FYI] §4 "No binding opens" — still-pending p0 obligations are not woken or delivered before an agent checks in after a resurrect.
  spot check: refute seat REJECT. This is the same state as any operator rebind of an unresolved seat, and after 24 h the p0 items are already settled.
- [FYI] §4 step 3, §2 — resurrect into threads that were archived during the window is not covered.
  spot check: refute seat REJECT. Archival is a thread-level lifecycle, and the spec promises restored membership rows, not unarchived threads.
- [FYI] §4, §5 Effect — no check that the agent who later checks in on P is the agent OLD stood for (harness kind, session).
  spot check: refute seat REJECT. This is an explicit operator or service decision, the host-kind refusal in `rebind_unresolved_seat` carries over, and AGENTS.md forbids adversarial verification.
- [FYI] §6 "Graph treats reversible-keyless as final" — graph may replace the clone while an operator later resurrects OLD, giving two live seats for one clone.
  spot check: refute seat REJECT. The C3 refusal guards an owned target, and graph's clone bookkeeping is outside this daemon-side design.
- [FYI] §2, §8 — service-required invitations and requirement episodes stay pending for 7 days, while §8 states only a 24 h sender delay.
  spot check: refute seat REJECT. Graph asked for this retention as requirement (4) and can finalize early through the D4 operation.
- [FYI] §1, §2, §3 deadline lane — no index, per-tick budget or fairness bound now that every keyless close is reversible.
  spot check: refute seat REJECT. Expiry reuses bounded quanta, finalize reuses the existing job, and the spec makes no scale claim.
- [FYI] §2 `recipient_retired` — expired obligations keep a final-retirement state value after the seat is live again.
  spot check: refute seat REJECT. The state records that the recipient was retired when the obligation expired, and liveness is read from seat state.
- [FYI] §2 — expiry writes `retired_at = cutover` on settled obligations, not the expiry time.
  spot check: refute seat REJECT. `obligations_expired_at` and the fact's `expired_at` record the expiry time.
- [FYI] §6, §2 — `seat inspect` cannot show a half-expired seat between the first quantum and `obligations_expired_at`.
  spot check: refute seat REJECT. `expires_at <= now` with no `obligations_expired_at` shows expiry is due or in progress.
- [FYI] D5, §4 step 1 — the operator resurrect of keyed seats is unrequested scope and an unwritten change to the owner-only rule.
  spot check: refute seat REJECT. D5, §4 and the §9 C3 and `operator_unretire` entries write it down, and excluding keyed seats would need an extra refusal branch.
- [FYI] §4 steps 3-4 — one resurrect records three history entries and a momentary unavailability episode.
  spot check: refute seat REJECT. The episode lives inside one writer transaction, and the history rows are intended provenance.
- [FYI] §4 Refusals "seat OLD retired finally; it can no longer be resurrected" — no stated byte size for a new model-visible string.
  spot check: refute seat REJECT. Rule (f) asks for the size in the PR or commit, not in the design spec. The string is about 55 bytes.
- [FYI] §8 third bullet — an ACK request that lands after the 24 h expiry gives its sender no warning for up to 6 more days.
  spot check: refute seat REJECT. §8 states and accepts this limit, and it is bounded by the 7 d finalization.
- [FYI] §2, §7, §8 — the new fact adds cross-node surface for an event a peer could derive from `cutover + 24 h`.
  spot check: refute seat REJECT. Home-authored facts are the existing pattern (as for the 7 d `seat.retired`), and a peer-local timer would race an un-retire.
- [FYI] §2 "its own copies of that episode's obligations" — which obligations a peer holds for a remote seat is not established.
  spot check: refute seat REJECT. The existing `seat.retired` application already runs these phases on peer rows, and an empty copy is a harmless no-op.
- [FYI] §2, §8 — the two-clock design adds a reversible-with-expired-obligations state and its rules.
  spot check: refute seat REJECT. D1 is a user decision, the row stays `reversible` with one marker, and the costs are listed in §8.
- [FYI] §5 Authority, §10 — the keyed branch of `ResurrectSeat` duplicates `UnretireSeat` with no named consumer.
  spot check: refute seat REJECT. It is the same shared transaction, `UnretireSeat` does not bind to a pane, and the cost is one test line.
- [FYI] §1 `expires_at`, `finalizes_at` — the stored columns are fixed offsets of `cutover_at` and duplicate the keyed/keyless distinction.
  spot check: refute seat REJECT. Stored deadlines suit an indexed scan, and NULL lets the scan skip keyed rows.
- [FYI] §6, D4 — `ResurrectSeat` and v4 `FinalizeRetirement` ship before graph adopts them, which freezes the wire shape.
  spot check: refute seat REJECT. D4 is an explicit decision sourced to graph, and the shape is derived from graph's stated requirements.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---