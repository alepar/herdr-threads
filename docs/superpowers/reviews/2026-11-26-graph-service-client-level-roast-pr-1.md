---
super-roast verdict: Should-fix (3 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 1 of 3
profile (assumed): Production local developer tool (herdr-threads 3.x, released). The change touches a public Rust client library API that herdr-graph uses. The trust model is cooperative and same-user, with no network surface.
inputs: feat/graph-service-asks@ee50a8e13 vs main@f672dbb5d
coverage: scouts 11/11 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, api-contract, testing, test-causality-lifecycle, test-isolation-environment, hygiene-docs) · raw 28 → deduped 16 → panel 5 · spot 11 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 5 · rr 0.80 · rg 1.00 · fg 0.80 · unanimous 0.80 · ground-loo 1.00 (n=4) · reproduce 3/2/0 · refute 2/3/0 · ground 3/2/0
lane-yield (found/confirmed/unique/refuted): correctness 4/2/0/2 · security 0/0/0/0 · premortem 3/1/0/2 · simplicity-design 4/0/0/3 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · api-contract 4/2/0/2 · testing 6/1/1/5 · test-causality-lifecycle 3/0/0/3 · test-isolation-environment 1/0/0/1 · hygiene-docs 3/1/0/2

Readiness note: review readiness arrived as coverage unknown and provenance unknown with an empty basis, so the verdict carries `[review readiness incomplete]` and, for compatibility, `[low coverage]`. Stage coverage itself was complete: 11 of 11 scouts returned, no dedupe or judge seat was lost, and nothing was capped.

Severity note: the panel split Nit / Should-fix / Nit on the first finding. It is held at Should-fix because the profile is a released public client API with a downstream consumer, the wrong outcome is durable, and this diff introduces it. No finding matches a severity floor. No severity was demoted because of the profile.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] src/client/service.rs:774; src/client/service.rs:433 — The client level is held only in memory and `replay` does not check it. A pending v4 `ResurrectSeat` intent replayed by a client rebuilt at the default V3 level is refused `Unsupported` at the daemon's level gate and is durably completed as a refusal, although the first submission may have applied. [lanes: correctness, premortem, api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `new` sets `level: ServiceSessionLevel::V3` (src/client/service.rs:433); only `with_level` (line 439) changes it.
  evidence: `SavedServiceIntent` holds only `author` and `request`. `replay` (line 774) checks only `saved.author` and `expected_instance`, and the author is `reserved_author_id(&instance)` at every level.
  evidence: the daemon gate at src/daemon/transport/service_connection.rs:268 (`if level < required { ... Unsupported "operation requires service_session_v4"; continue; }`) runs before the handler, so the operation's idempotent replay is never consulted.
  evidence: in `send_saved` only UnknownOutcome and ServiceBusy keep the intent pending. `Unsupported` reaches `self.journal.complete(key, definitive_response)` and returns `pending: None`. Afterwards `record` answers "operation key already completed" and `replay` answers "operation already completed".
  evidence: introduced by this diff. The refute seat checked the base: a V4 operation there was never pending, so the mismatch could not arise.
  evidence: trigger is a restart path that rebuilds the client without `.with_level(V4)` while V4 intents are pending; `journal().pending()` then `replay` is the documented recovery path.
  fix-shape hint: in `replay` (and optionally `submit`), keep the intent pending or refuse locally when `ServiceSessionLevel::required_by(op) > self.level`; alternatively store the level in the saved intent.
- [Nit] src/client/service.rs:672; src/client/service.rs:666; src/client/service.rs:667 — `submit` journals and sends an operation above the client's own level with no local check, then stores the daemon's `Unsupported` as a durable completion. That consumes the caller-stable OperationId. The docs and CHANGELOG do not mention it. [lanes: correctness, api-contract, hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute REJECT, dissent / ground ✓)
  evidence: `grep "required_by\|self.level"` in src/client/service.rs finds only the `with_level` assignment (line 440) and the registration capability (line 568), so `submit` has no level check before `journal.record`.
  evidence: `send_saved` (about lines 876-905) completes the key for any failure other than UnknownOutcome or ServiceBusy. `ServiceIntentJournal::record` (about line 172) then refuses the key with "operation key already completed".
  evidence: dissent recorded, default stands. The refute seat says this is the existing uniform journal contract and that base f672dbb5d behaves identically for a v3 client. The reproduce seat's evidence already addresses that ("the journal's existing design, applies to every refusal") and still rates a local pre-send check as low-impact hardening.
  evidence: neither confirming seat checked whether the docs or CHANGELOG mention the consumed key.
  fix-shape hint: the same `required_by` guard as the first finding, placed before `journal.record`, returns `Unsupported` with no intent written; otherwise add one doc line saying a level refusal consumes the key.
- [FYI] tests/service/reversible_session.rs:461 — No test drives a successful `ResurrectSeat` through `PersistentServiceClient::submit`, although the docs changed in this diff say a v4 client "carries it through `submit`". The `result_matches` arm pairing `ResurrectSeat` with `SeatResurrected` and the wrong-result path are never exercised. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: the v4 integration test ends at `ErrorCode::HostUnavailable`. `register_negotiates_the_chosen_level` checks only the Register frame. `v3_pairs()` (src/client/service.rs:1909) has no `ResurrectSeat`/`SeatResurrected` pair.
  evidence: pre-existing on base: the `result_matches` arm (src/client/service.rs:966 on base, about 974 at head) and the base `v3_pairs()` are untouched by this diff. All three seats marked it pre-existing, so final severity is FYI and it does not drive a fix round.
  fix-shape hint: a mock-socket unit test that answers `SeatResurrected`, then a wrong result expecting UnknownOutcome with the key still pending; or add the pair to the `v3_pairs` table.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- tests/service/reversible_session.rs:470 — The new test asserts `pending == None` after a `HostUnavailable` refusal of `ResurrectSeat`, locking in that the client journal consumes the key.
  reason: rejected 3 of 3. The `send_saved` rule (only UnknownOutcome and ServiceBusy stay pending) is unchanged by this diff. The assertion is incidental to the test's purpose, which is the level gate. Whether `HostUnavailable` should stay pending is a client-journal design question for a separate change; the finding itself calls it a design question.
- src/client/service.rs:439; src/client/service.rs:437 — `with_level` accepts any `ServiceSessionLevel`, including V1 and V2, so a client can be downgraded below v3.
  reason: rejected 3 of 3. Bead ht-lzhi asks for a caller-chosen level with a V3 default and no fallback, so accepting the whole enum is the requested design. A level below v3 is an explicit opt-in and fails with a typed `Unsupported` at the daemon gate; nothing is corrupted. Narrowing the type is an API preference, not a requirement.

## Unverified nits (spot-checked)
- tests/service/reversible_session.rs:462; tests/service/reversible_session.rs:460 — The v4 half of `persistent_client_registers_the_chosen_level` proves the gate passed only through a downstream `HostUnavailable` that depends on dispatch resolving the host before the store checks seat state; the seeded live seat is dead setup.
  spot check: REJECT (FYI). The contrast with the v3 half still discriminates the level, and a reorder would fail the test visibly.
- tests/service/reversible_session.rs:429 — The v4 half's `HostUnavailable` comes from `HostSessions::attach_locked` refusing the non-fixture locator `/run/herdr-test.sock`, not from the fixture's `UnavailableHost`; the test comment misdescribes it and the store-side v4 authority is never exercised.
  spot check: REJECT (FYI). The mechanism is probably right but was not run; what remains is an inaccurate comment.
- docs/programmatic-service-client.md:57 — The doc drops the wrong advice that a v4 caller registers its own session but never states the one-live-service-session-per-instance limit (src/service/live_gate.rs:64-68) that made it wrong.
  spot check: REJECT (FYI). Documentation addition, not a contract gap.
- src/client/service.rs:416 — The level chosen with `with_level` cannot be read back: no `level()` getter, and `ServiceRegistration` carries no level.
  spot check: REJECT (FYI). Introspection is outside ht-lzhi; the code that builds the client knows the level.
- tests/service/reversible_session.rs:429 — The expected `HostUnavailable` uses the absolute path `/run/herdr-test.sock` and not a scratch path.
  spot check: REJECT (FYI). Fixed fake socket paths are an existing test convention. A second spot check states the outcome depends only on the fake host world's live set, not the real filesystem.
- src/client/service.rs:1854; src/client/service.rs:1853 — `register_negotiates_the_chosen_level` builds its scratch root with the module's bare `/tmp/hsc-<uuid>` helper and removes it only on the success path, so a failing assertion leaves an unledgered directory.
  spot check: REJECT (FYI). pre-existing on base: `temp()` at src/client/service.rs:1033, used by 22 tests on base; the new test copies the pattern.
- src/client/service.rs:433 — The V3 default has no stated requirement; a V4 default would have needed no new public API.
  spot check: REJECT (FYI). Bead ht-lzhi asks for a caller-selected level with no fallback, and a V4 default would break clients against pre-v3.5.0 daemons.
- src/client/service.rs:568 — Nothing tests that the chosen level survives re-registration after a dropped or invalidated session.
  spot check: REJECT (FYI). `level` is a plain field read on every `register`; no code path resets it.
- src/client/service.rs:1861 — In `register_negotiates_the_chosen_level`, a failed capability assertion in the server task surfaces as a client EOF panic because `server.await` is reached second.
  spot check: REJECT (FYI). The test still fails and the task's panic message still prints; the existing v3 register test has the same ordering.
- tests/service/reversible_session.rs:438 — Test names and the `with_level` doc comment say "negotiates" and "registers the chosen level", but the integration test holds two scenarios and there is no negotiation (no fallback).
  spot check: CONFIRM (Nit). Wording only; a single refute-seat pass, not panel-strength.
- src/client/service.rs:1852 — `register_negotiates_the_chosen_level` repeats the default-v3 register test almost line for line; no shared parameterized helper.
  spot check: REJECT (FYI). Test-only duplication that matches the module's inline style.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---