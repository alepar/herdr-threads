---
super-roast verdict: Should-fix (4 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 1 of 3
profile (assumed): Production local developer tool (herdr-threads 3.x, released), used by multiple agents and by herdr-graph. Seat identity is governed by the trust policy: a wrong seat move misattributes messages and obligations. The model is cooperative and same-user, with no network surface.
inputs: feat/graph-service-asks@483a1f8ab vs f738bcb2f
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, api-contract, observability, testing, test-causality-lifecycle, hygiene-docs, data-migrations) · raw 55 → deduped 43 → panel 14 · spot 29 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 14 · rr 0.93 · rg 0.93 · fg 1.00 · unanimous 0.93 · ground-loo 1.00 (n=13) · reproduce 3/11/0 · refute 4/10/0 · ground 4/10/0
lane-yield (found/confirmed/unique/refuted): correctness 4/1/0/3 · security 3/0/0/3 · premortem 5/0/0/5 · simplicity-design 7/0/0/5 · hot-path-perf 2/0/0/2 · concurrency-async 3/0/0/3 · api-contract 7/0/0/6 · observability 5/0/0/5 · testing 9/3/1/4 · test-causality-lifecycle 1/0/0/1 · hygiene-docs 6/2/1/3 · data-migrations 3/0/0/3

Review readiness: the engine supplied coverage "unknown", provenance "unknown" and an empty basis. The verdict therefore carries `[review readiness incomplete]` and `[low coverage]`. Stage coverage itself is intact: no scout, dedupe or judge seat was lost. All four confirmed findings are test or documentation gaps; no seat confirmed a defect in production code.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] tests/store/reversible_retirement.rs:2876 — The `replace` case in `service_restore_refusals` names another pane and is refused by the saved-target `NotFound` check first, so the D3 `InvalidRequest` branch and the `replace.is_some() && state != "unresolved"` clause are untested. [lanes: correctness, testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `author(&f, "replace", "s3", Some("s3"))` acts on seat s1, whose saved target is s1. In `service_case_of` (src/store/seats/operator.rs ~485), `saved_target != Some(target)` returns `not_found()` before the match. The test asserts only `ErrorCode::NotFound`. The `InvalidRequest` arm ("a service restores a held seat without replace", operator.rs:502) is never reached. The design (D3 line 40, test list line ~84) names `replace` with an unresolved OLD as `InvalidRequest`. The test's doc comment says it kills a restore "with `replace`". The refute seat found no other test asserting this `InvalidRequest`.
  severity note: seats voted Nit / Should-fix / Should-fix. Kept at Should-fix: the branch is new code in this diff, it is a spec-named refusal code, and the test claims coverage it does not give.
  fix-shape hint: call the helper with target "s1" and `Some("s3")` and assert `InvalidRequest`; keep or add a separate case for the resolved-seat `replace` clause.
- [Should-fix] tests/store/reversible_retirement.rs:2786 — The success test never checks that the baseline hold is released (D6, and design test plan item 1 "baseline lifts when it was the last"). In the `Held::Baseline` case none of its assertions could fail if the baseline release or lift were dropped. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `effective_recovery_disposition` (src/store/effective.rs:135) returns `Owned` as soon as a resolved seat sits on the target, before it reads any hold or baseline state. The `recovery_holds ... released_at IS NULL == 0` count is trivially 0, because `restart_held(.., Held::Baseline)` inserts no `recovery_holds` row. No assertion reads `recovery_baseline_releases` or `herdr_hosts.baseline_hold_unclaimed` (grep shows `baseline_hold_unclaimed` only in fixture setup, lines 1206 and 2742). Deleting the baseline branch of `release_claimed_target` would leave this test green. Not verified by the seats: whether the fixture reaches `lift_baseline_hold_if_clear` (the finding says `recovery_boot` stays NULL so it returns early).
  severity note: seats voted Nit / Nit / Should-fix. Set to Should-fix because the profile states production, trust-policy-governed seat identity, and this is the C2 hold release for the design's headline case. The release code itself is reused, untouched by the diff, and was not shown to be wrong.
  fix-shape hint: in the Baseline iteration assert the `recovery_baseline_releases` row and `baseline_hold_unclaimed`, and set up the fixture so the lift path actually runs.
- [Nit] src/service/dispatch.rs:637; docs/superpowers/specs/2026-11-26-service-restore-held-seat-design.md:103 — Design test plan item 8 (a v4 service session over the wire getting `SeatAlreadyResolved` and the refusal codes) was not implemented or recorded as dropped, so the dispatch arm `OperatorAlreadyResolved -> SeatAlreadyResolved` is not executed by any test. [lanes: testing, hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `git diff --stat f738bcb2f...483a1f8ab -- tests/service` is empty. `SeatAlreadyResolved` appears only in the client fake-server test (src/client/service.rs ~1655), the dispatch arm (src/service/dispatch.rs:638) and the serde roundtrip (src/protocol/service.rs:989). The store tests call `mutate_operator_as_service` directly and bypass `DomainService::service_resurrect`. The existing resurrect test in tests/service/reversible_session.rs (~436-472) reaches only a host-unreachable refusal. The spec diff changes only the Status line to "agreed". All three seats rated it Nit: the arm is a 3-line mapping and the store and client halves are tested separately.
  fix-shape hint: add one tests/service case that drives `ResurrectSeat` through dispatch to `SeatAlreadyResolved`, or record item 8 as dropped in the design.
- [Nit] TRUST-POLICY.md:272 — The TRUST-POLICY C1 *Service resurrect* row still says ResurrectSeat applies "for a reversibly retired seat only", which the new *Service restore* row right below it contradicts. [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✗ (REJECT) / refute ✗-survived / ground ✓)
  evidence: Line 272 is unchanged by the diff and keeps "for a reversibly retired seat only". Line 273 has the same verb act on an unresolved restart-held seat with no `service_unretire`. The amendment note at lines 122-123 says `ResurrectSeat` "also rebinds an unresolved seat held by a Herdr restart". The reproduce seat's dissent (the "only" scopes that row's own case; two labelled cases, each with its own provenance) is answered by the refute seat: the rows are distinct cases, but nothing amends the older row's "only". Behaviour is unambiguous from line 273; this is wording in a normative document.
  fix-shape hint: drop "only" from line 272 or add a cross-reference to the Service restore row.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- src/cli/hook.rs:2122; src/cli/hook.rs:2121 — The new hook arm sends every lifecycle `TargetAlreadyOwned` continuity refusal from an absent or held pane to the ordinary resolver without checking who the owner is.
  reason: 3/3 REJECT. Design D6 (spec line 43) specifies this in general terms: "it now treats it as the Resolved arm already does: a lifecycle event goes on to the ordinary resolver, which finds P's owner (OLD) and checks in; a turn event keeps its refusal". Test item 7 repeats it. The arm is gated on `Enroll::Lifecycle`. The resolver call that follows re-applies the daemon's canonical guards, including the key verdicts. The finding concedes the Resolved arm already behaves this way and that it did not establish the outcome is wrong under the pane-as-identity model. Residual noted by the ground seat: tests cover only the restore owner.
- src/protocol/service.rs:441 — `ServiceResult::SeatAlreadyResolved` ships on the existing `service_session_v4` level with no capability or level gate, so a strict v4 client built against v3.5.0 cannot decode the response.
  reason: 3/3 REJECT. All three seats accept the mechanism: `ServiceResult` has no serde fallback and the variant is ungated. They reject on scope. D1 says "v4, no new level, no new request field", and TRUST-POLICY records a new level as rejected. The variant is reachable only when a client sends `ResurrectSeat` for a seat already resolved on its own saved pane, which answered `NotFound` in 3.5 and which no documented 3.5 flow issues. A 3.5.0 `PersistentServiceClient` could not register v4. The finding is external and its spike (does herdr-graph decode `ServiceResult` strictly) was not run; no seat voted UNVERIFIED, so the engine route stands. Two seats call a one-line doc note about older strict clients the most this warrants.
- src/store/seats/operator.rs:220; src/store/seats/operator.rs:522 — `service_case_of` runs before `require_v4_authority`, so a non-owner gets `ServiceBusy` or `NotFound` instead of `Unauthorized`.
  reason: 3/3 REJECT. The ordering is real. D3 says "Authority: require_v4_authority unchanged" and does not require authority first. The base already returned the reversible-state `NotFound` before authority (f738bcb2f operator.rs ~257-266). AGENTS.md: "Do not add adversarial caller verification; the model is cooperative and same-user." A non-owner gets `Unauthorized` once the lag clears and nothing is written. Residual: no test of a stranger during lag.
- src/store/seats/operator.rs:490 — `service_case_of` runs the reconciliation-lag check before it branches on seat state and on `replace`.
  reason: 3/3 REJECT. The spec's Changes section orders the D2 guard as "own target, hold, Herdr, no lag, no kept claim, no replace". D3 sets no precedence between lag and `replace`. The effect is a transient `ServiceBusy` that becomes the terminal answer after the first reconciliation pass. Nothing is written.
- src/store/seats/operator.rs:206 — In the window right after a Herdr restart, before the daemon records the new host boot, a restore probably gets a definitive refusal instead of `ServiceBusy`.
  reason: 3/3 REJECT. The premise is disproved by src/identity/repair.rs. `operator_as_service` → `operator_for` → `with_observation` → `observe_once` turns the adapter's `StaleHostObservation` into `NeedsCapture`, requests an explicit lane capture and waits for it (`wait_for_capture`, `CAPTURE_WAIT_CAP` 5 s) before the store check runs. The guard therefore carries the adopted boot, and `reconciliation_lags` then answers `ServiceBusy`. The residual `StaleHostObservation` on a capture timeout is shared with base `ResurrectSeat` and every operator action, and is not introduced by this diff.
- src/store/seats/operator.rs:513; src/store/seats/operator.rs:515 — Any kept continuity claim on the target or of the seat blocks a service restore with `ServiceBusy` whatever its age.
  reason: 3/3 REJECT. D2 and D3 require it (spec lines 39-40: "a kept claim on the target or of OLD: `ServiceBusy` (retryable; the pass or the claim decides first)") and the test list includes it. The wait is bounded: `kept_continuity::due` (src/store/kept_continuity.rs:155) decides or prunes rows at the 1 h `KEPT_CLAIM_TTL_MS`. The delay is the documented fail-safe ("costs a delay, never a seat").
- src/store/seats/operator.rs:334 — A `SeatAlreadyResolved` answer is stored permanently under the operation key and replays after the seat is held again.
  reason: 3/3 REJECT. D5 says the idempotent transaction stores the result under the operation key like any other and a replay returns it; test item 5 covers replay. A new disruption is a new operation and takes a new key. The same holds for a replayed successful rebind.
- src/store/seats/operator.rs:497; src/store/seats/operator.rs:500 — `service_case_of` answers `AlreadyResolved` for any local seat resolved on its own saved Herdr pane with no hold or restart condition.
  reason: 3/3 REJECT. D5 (spec line 42) specifies exactly this and sets no hold or restart precondition for the resolved arm; tests 4 and 8 name it. The change from 3.5's `NotFound` is deliberate and documented.
- src/store/seats/operator.rs:206 — A D5 race in which the resumed session's reattach advances the pane's effective observation reaches the service as `TargetAlreadyOwned` instead of `SeatAlreadyResolved`.
  reason: 3/3 REJECT. The premise is unsupported. `observed_matches` (src/store/seats/resolution.rs:36-54) compares host boot, epoch and structural generation from host observation tables. A reattach writes the seat row and bindings; src/store/seats/cooperative.rs only reads `structural_generation` (lines 288, 382, 1272, 1288), and writes come from snapshot processing in observation.rs. The finding says it did not trace this. A real host re-observation is a genuine target change, and the stale-guard refusal is existing behaviour.
- src/store/seats/operator.rs:487 — While `reconciliation_lags` is true every restore returns `ServiceBusy`, with no bound.
  reason: 3/3 REJECT. D2, D3 and D5 choose this. `reconciliation_lags` already has the same retryable semantics on base in continuity.rs (f738bcb2f lines 605, 831, 869, 1450). The cited comment at observation.rs:1414 is about `kept_claim_blocks`, not a stuck first pass. No indefinite stall was shown, and a stuck pass would be a defect in code this diff does not touch.

## Unverified nits (spot-checked)
- [Nit] tests/store/reversible_retirement.rs:2905 — The no-op (AlreadyResolved) case is tested only with the default author on a keyless seat; no test shows a stranger on a keyed, already-resolved seat gets `Unauthorized` (D5 "needs the same authority").
  spot: CONFIRM. Production code enforces it (operator.rs ~219-222 runs `require_v4_authority` for every case); only the regression guard is missing.
- [Nit] src/store/seats/operator.rs:483 — No test covers the D2 refusals for a non-Herdr host kind or for a saved host that differs from the request's host, although design test plan item 2 lists "named host".
  spot: CONFIRM. The `saved_host != host.get() || HostKind::from_stored(&kind) != HostKind::Herdr` clause could be deleted with no test failing; later checks would likely still refuse a named seat.
- [Nit] docs/programmatic-service-client.md:57 — The doc still lists "already un-retired" under `NotFound`; a second ResurrectSeat under a new key onto the same pane now answers `SeatAlreadyResolved`.
  spot: CONFIRM. The next doc paragraph does describe `SeatAlreadyResolved`; only the earlier `NotFound` sentence is imprecise for the same-pane case.
- [Nit] docs/operations.md:273 — The operator-facing "When a hold lifts" list was not updated for the service-restore release path, so it disagrees with TRUST-POLICY C2 as amended in the same commit.
  spot: CONFIRM. The commit touched operations.md only on the `service_rebind` history row (line 306).
- [Nit] src/store/seats/operator.rs:512 — The new inline kept-claim SQL duplicates the identical query at src/store/seats/continuity.rs:726, both outside `store::kept_continuity`.
  spot: CONFIRM. Read-only one-line EXISTS; tidy-up only.
- [Nit] tests/store/reversible_retirement.rs:2823 — In `service_restore_refusals` the helper closure is named `author` but performs a service rebind and unwraps its error.
  spot: CONFIRM. The file already uses `author` for a different thing at line 145.
- [FYI] src/store/seats/operator.rs:219 — Any `service_session_v4` session can rebind any keyless, unresolved, restart-held local seat onto its saved pane; no ownership check ties the caller to the seat.
  spot: REJECT. D3 states the authority (keyless: any v4 session; keyed: the key owner), TRUST-POLICY adds the accepted limit "Service restore of a held seat (ht-0f0r)", and AGENTS.md forbids adversarial caller verification.
- [FYI] src/cli/hook.rs:2121 — The new hook arm falls through to the ordinary resolver with no diagnostic, so nothing records that the session's claimed seat was passed over.
  spot: REJECT. D6 specifies the fall-through; the service rebind is recorded in history as `service_rebind` with `service:<author>`; a diagnostic line is not a spec requirement.
- [FYI] tests/cli/hook.rs:8521 — The hook test fixes the store's answer to a C1 claim after a service restore as `TargetAlreadyOwned`, with no store-level test of that order.
  spot: REJECT. The owned-pane `TargetAlreadyOwned` contract is tested in tests/identity/kept_continuity_rules.rs:36, tests/store/continuity_key_decisions.rs:681/697/713/1130 and tests/store/shell_continuity.rs:602. No single test chains restore then claim.
- [FYI] src/store/seats/operator.rs:220 — Error precedence changed: a non-reversible seat sent to a pane another seat owns used to get `TargetAlreadyOwned` and now gets `NotFound`.
  spot: REJECT. D5 requires the case to be decided before `target_free`; both orders refuse before any write; the base order was incidental.
- [FYI] src/store/seats/operator.rs:480; src/store/seats/operator.rs:484; src/store/seats/operator.rs:487 — `service_case_of` refuses `replace` in two places with different codes (`NotFound` and `InvalidRequest`).
  spot: REJECT. The split follows D3's precedence (not on saved target → `NotFound`; `replace` on the seat's own pane → `InvalidRequest`). The doc naming no code is a documentation nit.
- [FYI] src/protocol/service.rs:254 — The public rustdoc on `ResurrectSeat` still describes only the un-retire case, though the design's Changes list names "`ResurrectSeat` doc".
  spot: REJECT. The facts hold; docs/programmatic-service-client.md carries the full restore contract and the new variant has its own rustdoc.
- [FYI] docs/programmatic-service-client.md:59 — The doc says "retry" on `ServiceBusy` with no retry-after, backoff or jitter guidance.
  spot: REJECT. The base doc uses the same wording for `UnretireSeat` and `FinalizeRetirement` (lines 51-52 on f738bcb2f); the contention claim is speculative. The seat did not verify the cited store/schema.rs lines.
- [FYI] src/store/seats/operator.rs:516 — The restore path's `ServiceBusy` refusals carry no detail about what is blocking.
  spot: REJECT. D3 defines the refusal as retryable; the spec requires no blocker identity; the blocker is not something the service can act on.
- [FYI] tests/cli/hook.rs:9318 — In the SyntheticTool iteration of `continuity_claim_beaten_by_a_restore_checks_in_on_lifecycle`, `outcome.diagnostic.is_some()` passes on any hook failure, including a budget expiry.
  spot: REJECT. A dropped `Enroll::Lifecycle` guard still fails `enrollment_counts == (0,0)` in a normal run; the masking case needs a ~20 s stall and is unreproduced. Asserting the diagnostic's code would harden the test.
- [FYI] tests/store/reversible_retirement.rs:2860 — Only the native-session branch of `kept_claim_blocks` is tested on the restore path; the continuity-key branch is not.
  spot: REJECT. `kept_claim_blocks` is existing shared logic (observation.rs:1419); its key disjunct is exercised in tests/store/restored_pane.rs (~line 951).
- [FYI] src/store/seats/operator.rs:465 — `service_case_of` gives one `NotFound` code and message for at least six refusal causes.
  spot: REJECT. D3 and D5 define the single `NotFound` ("graph's answer for 'not this case'"); causes needing different handling use `ServiceBusy`, `InvalidRequest` and `SeatAlreadyResolved`.
- [FYI] src/store/seats/operator.rs:334 — The `SeatAlreadyResolved` no-op writes no `allocation_decisions` row, so seat history holds no trace of the service's call.
  spot: REJECT. D5 (spec line 42) and test item 4 specify "writing no seat row, history, hold or binding change"; the operation journal is the specified record.
- [FYI] src/protocol/results.rs:138 — `CommandResult::OperatorAlreadyResolved` is written into `operations.result_json` with no schema bump, so a binary-only rollback fails to decode such rows.
  spot: REJECT. docs/install.md supports going back only by restoring the pre-upgrade backup; existing `CommandResult` variants were added the same way; the harm is a decode error on one status read.
- [FYI] src/store/seats/operator.rs:334 — For a keyless seat any v4 session can confirm that a seat is resolved on a given pane, and the result is persisted under its operation id.
  spot: REJECT. D5 specifies the no-op, its authority and its journaling; the caller already names seat, host and target; cooperative same-user model.
- [FYI] src/store/seats/operator.rs:224 — The already-resolved no-op decision is spread across four places in `mutate_operator_guarded`.
  spot: REJECT. D5 prescribes this store shape; the flag is computed once and each use is commented "ht-0f0r D5".
- [FYI] tests/store/reversible_retirement.rs:2797 — The `Held::Row` "hold released" check uses a single seat, so it cannot tell a per-pane release from a host-wide one.
  spot: REJECT. Test item 1 does not require a two-seat test; `lift_baseline_hold_if_clear` has its own tests at tests/store/reversible_retirement.rs:1214-1220 and tests/store/control.rs:8329.
- [FYI] docs/programmatic-service-client.md:59 — The doc says "A daemon before 3.6 answers NotFound" while the CHANGELOG entry is under "Unreleased" and Cargo.toml is 3.5.0.
  spot: REJECT. The versioning rule makes 3.6.0 the expected next tag; release prep reconciles the number.
- [FYI] src/store/queries.rs:717 — The new `OperatorAlreadyResolved` arm in `query_operation_status` has no test.
  spot: REJECT. One or-pattern alternative whose sibling arms have no per-variant tests either. The seat did not check whether generic operation-status tests exercise it.
- [FYI] src/store/seats/operator.rs:441 — The three-variant `ServiceCase` enum is read only as a bool; nothing distinguishes `Resurrect` from `Restore`.
  spot: REJECT. Literally true; no behavioural consequence.
- [FYI] src/store/seats/operator.rs:469; src/store/seats/operator.rs:461 — `service_case_of` re-reads seat state that the check and decision closures read again, and repeats a host-kind check.
  spot: REJECT. A few point reads in one transaction on a cold path; the reads cannot diverge.
- [FYI] src/cli/hook.rs:2121 — The new hook arm is separate from the adjacent `NotFound` arm although their lifecycle behaviour is the same.
  spot: REJECT. The arms differ for non-lifecycle events, so merging them would change behaviour or need an equally complex guard.
- [FYI] docs/superpowers/specs/2026-11-26-service-restore-held-seat-design.md:84 — The design names `protocol::commands::CommandResult`, but the variant is added in src/protocol/results.rs.
  spot: REJECT. A one-token slip in a prose Changes list (the seat places it at design line 68).
- [FYI] src/cli/hook.rs:2121 — A lifecycle `Reattach::Refused(TargetAlreadyOwned)` now makes one more daemon round trip.
  spot: REJECT. At most one extra round trip per occurrence, on lifecycle events only, in the restore race; the finding itself records it for completeness.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---