super-roast verdict: Nit (2 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 2 of 3
profile (assumed): herdr-threads is an open-source Rust CLI plus local daemon (v2.4.x) that gives coding agents durable message threads across a user's own machines; peers are authenticated same-user nodes under a cooperative trust model (TRUST-POLICY A1/N6). Blast radius is low: no money or external users' data, a small user base, and rollback is an install of the previous release. The one feature-level stake is routing correctness: the change decides which seat a `send --seat X@M` lands on, and spec D7 / TRUST-POLICY N14 forbid silent local fallback or a choice between readings. Resilience and observability findings are weighed down accordingly; routing-correctness and spec-contract findings are not.
inputs: feat/remote-addresses@022e94c89 vs 6852590ec (delta)
delta vs prior: 2 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 14 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs) · raw 3 → deduped 3 → panel 2 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 3/2/2/0 · test-causality-lifecycle 0/0/0/0 · test-isolation-environment 0/0/0/0 · test-suite-cost 0/0/0/0 · hygiene-docs 0/0/0/0

Readiness note: the engine supplied no review-readiness basis (coverage unknown, provenance unknown). The verdict carries `[low coverage]` and `[review readiness incomplete]` for that reason only; all 14 scouts returned, judge completion is 100% and `coverage.lowCoverage` is false. No absence-based signal is drawn from this round. This round's scope is the delta 022e94c89 vs 6852590ec only; the round-1 fixes themselves (6852590ec) were not re-scouted here.

Prior-report tracking: none of the 14 round-1 confirmed findings was re-surfaced by this round's packets. Each is marked resolved below on fix evidence at the head, not on scout silence: the fix commit 6852590ec ("Fix the remote address review findings (PR roast round 1)") and 022e94c89, checked by grep at 022e94c89.
- resolved: src/service/sync_admin.rs:494 (transport bound) — transport.rs:556 gives `SyncCommand::ResolveAddress` its own bound; as-built note "Budgets (PR roast round 1)" records `REMOTE_ADDRESS_DAEMON_BOUND_MS` = 10 s + 5 s + 2 s and the CLI 3 s longer.
- resolved: src/cli/seat_resolution.rs:721 (`Err(_)` local fallback) — seat_resolution.rs:972 matches `ErrorCode::Unsupported | ErrorCode::UnknownWireVersion` only; as-built "Error classes" note: every other failure is passed through as a refusal.
- resolved: src/sync/coordinator.rs:620 (resolve_remote untimed send) — coordinator.rs:694-709 checks `budget.cancellation` and selects on `cancelled()`, like snapshot.
- resolved: src/sync/coordinator/remote_addr.rs:183 (saturation reported as LinkLost) — remote_addr.rs:214-215 answers `unavailable("too many remote requests in flight on the asking node")`.
- resolved: src/cli/seat_resolution.rs:723 (raw daemon errors passed through) — seat_resolution.rs:740/750 map to `remote_error::unknown_machine`; Unsupported handled at :972.
- resolved: src/cli/seat_resolution.rs:1037 (refusals all HostUnavailable) — seat_resolution.rs:882-896 split: Unavailable → host_unavailable, Refused → `invalid_request("{label} refused {location}: ...")`. This round's first confirmed finding is the missing test for that split.
- resolved: src/cli/seat_resolution.rs:781 (fabricated `w0:p0`) — seat_resolution.rs:838 prints `"-"` for an unparsable pane.
- resolved: src/service/remote_resolver.rs:107 (`.ok()?` swallow) — remote_resolver.rs:92/129 answer `seat lookup failed: {detail}`; 022e94c89 classes store read failures as Unavailable.
- resolved: src/sync/coordinator.rs:8153 (fixed 300 ms sleep) — coordinator.rs:8130-8184 `entered: Arc<AtomicUsize>` polled to 8; no `from_millis(300)` remains in the file.
- resolved: docs/superpowers/specs/2026-10-27-remote-addresses-design.md:547 (same-machine rule undocumented) — spec line 579-583 "Deferred to phase 2" records it.
- resolved: src/cli/seat_resolution.rs:787 (both_readings untested) — tests/multinode/remote_addresses.rs:161-169 asserts `2 readings` for a local label that is also a live remote address. (The new SelfNode Conflict arm added in 022e94c89 is a separate gap; see confirmed findings.)
- resolved: src/service/sync_admin.rs:538 (N14 asking-side check untested) — tests/service/sync_admin.rs:2174 `resolve_address_matches_machines_and_takes_only_the_named_nodes_seat`, :2225-2229 `wait_replicated` with an absent seat.
- resolved: src/service/remote_resolver.rs:128 (RemoteResolver untested) — remote_resolver.rs:263 `#[cfg(test)]` module covering `pool`, host-failure rule, Conflict; tests/store/remote_seats.rs:856 imports `exact_outcome, pane_seats, seat_row`.
- resolved: tests/multinode/remote_addresses.rs:48 (negative-path tests unrecorded) — the as-built "Budgets" note records timeout, late reply and the 8-request bound at the coordinator test, host failure at the resolver, and reconciles AC5 to the 20 s CLI budget. Recorded as unit-level coverage rather than the spec's e2e cases, which the finding's fix shape allowed.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Nit] src/cli/seat_resolution.rs:882 — No test checks the CLI's split between retryable `AddrOutcome::Unavailable` (host_unavailable with the `\@` hint) and permanent `AddrOutcome::Refused` (invalid_request `refused LOC: detail`), which is this commit's stated purpose; swapping or merging the two CLI arms passes every test. [lanes: testing] (new this iteration)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: seat_resolution.rs:882-896 maps Unavailable to "{label} cannot answer now: ..." on the retryable path and Refused to `ApiError::invalid_request("{label} refused {location}: ...")`. `git grep` at 022e94c89 finds "cannot answer now", "is not shared" and "refused " in no test. AddrOutcome::Unavailable/Refused appear in tests only on the producing side: coordinator.rs:8089 and :8191 (home node answers Unavailable), tests/store/remote_seats.rs:868 and :886 (exact_outcome). tests/multinode/remote_addresses.rs reaches neither a remote Refused nor a remote Unavailable at the asking CLI; its "lives on ... not alpha" case is the new local SeatElsewhere reply. The prior round's confirmed Nit (seat_resolution.rs:1037) is what this split fixes, and no test pins the fix. Severity: unanimous Nit; shipped behaviour is correct, the gap is end-to-end coverage of a few-line match.
  fix-shape hint: one multinode or service-level case per arm: a home node answering Unavailable (e.g. HT_TEST-gated resolver busy) expecting class Unavailable / exit 3 with the `\@` hint, and one answering Refused (an unpublished seat's `is not shared with other machines`) expecting invalid_request / exit 2 with the `refused` wording.

- [Nit] src/cli/seat_resolution.rs:786 — The new SelfNode guard that refuses an ambiguous split reading (`Err(RunError::Api(error)) if error.code == ErrorCode::Conflict => both_readings(.., "{location} (ambiguous)")`) has no test; deleting the arm restores the old behaviour of silently taking the whole string's local pick and passes every test. [lanes: testing] (new this iteration)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: the arm exists at seat_resolution.rs:~786-791 at 022e94c89; the commit message lists it as a fix ("an ambiguous split reading on this node is refused, not replaced by the whole string's pick"). `git grep '(ambiguous)'` over tests/ returns nothing. The only SelfNode collision test is `w1:p1@alpha` in tests/multinode/remote_addresses.rs, where both readings resolve uniquely (label w1:p7 vs plain w1:p1) and the code takes the same_resolution / both_readings path, never the Conflict arm. Without the arm the Conflict error falls to the generic `Err(_)` branch and `resolve_local(raw)`, a routing choice between readings that D7/N14 forbid. Severity: unanimous Nit; the refusal is implemented, only the regression guard is missing. The ground seat notes it did not run the tests or confirm the fixture cannot reach the arm; the reproduce seat's fixture inspection covers that.
  fix-shape hint: add a multinode case with two local panes whose location matches the left side of a `LOC@<self>` label that also names a local pane, and assert the `2 readings` refusal with the `(ambiguous)` line.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- none this iteration. The 7 rejections and the 22 spot-rejected nits of iteration 1 stand unchanged; nothing re-surfaced them.

## Unverified nits (spot-checked)
- [Nit] src/service/remote_resolver.rs:160 — The commit's stated change that the home node answers its own retired seat for `SEAT_ID@M` (seat_row dropped the old `state<>'retired'` filter) is not exercised: tests/store/remote_seats.rs::remote_resolution_answers_only_published_local_seats never retires the local seat, so restoring the filter passes every test. (spot refute: CONFIRM Nit — seat_row at 022e94c89 has no state filter and is documented as "retired included"; the only caller test covers unpublished, published, foreign and unknown; the retired case is tested only on the asking side via a different query in sync_admin::self_address. Fix: one extra assertion using the file's existing retire helper, expecting `AddrOutcome::Seat` for the retired local seat's ID.)

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
