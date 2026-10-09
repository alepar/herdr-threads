---
super-roast verdict: Nit (2 confirmed) [converged]
mode: pr        iteration: 2 of 3
profile (assumed): Local same-user cooperative production CLI/daemon with durable SQLite state and native topology creation. Persistent seat attribution, original actor, migration preservation and idempotent replay are critical; labels are discovery hints. No adversarial caller verification or privilege-escalation claim solely from the cooperative invoker controlling their own input.
inputs: super-auto/handoff-topology@45af88ff09c1cd3009c022aa43c7268ecf87d948 vs main@6ef444d4a43759bbd25ed7616321b4e68d2f0c60
delta vs prior: 1 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 5 resolved · 1 regressed (0 Blocking) · 1 punch-listed (open)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, dependency, hygiene-docs) · raw 4 → deduped 4 → panel 3 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 3 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=3) · reproduce 2/1/0 · refute 2/1/0 · ground 2/1/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 1/0/0/1 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 1/0/0/1 · api-contract 1/1/1/0 · observability 0/0/0/0 · testing 1/1/1/0 · dependency 0/0/0/0 · hygiene-docs 0/0/0/0

## Confirmed findings
- [Nit] src/cli/topology_handoff.rs:1438 — A proven typed zero-submission rearm (attempt N closed, Prepared N+1) exits with ErrorCode::InvalidRequest (exit status 2, "invalid arguments or invalid local context") although the arguments were valid and the documented remedy is to retry the same reference unchanged. (regressed: incomplete fix of iteration 1's [Should-fix] src/cli/topology_handoff.rs:1423;1398 — the pending report is now rendered, the usage-class exit status remains) [lanes: api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/cli/topology_handoff.rs:1421-1440 matches Ok(next) with state not Attached/Completed, writes the pending report, then returns super::invalid_request("bootstrap lacks canonical attachment: ... retry the original to continue").
  src/cli/exit.rs:18 maps InvalidRequest to EXIT_USAGE (2). src/cli/commands.rs:1829-1839 documents 2 as invalid arguments or invalid local context, 3 as daemon or host unavailable, 5 as outcome unknown; the table is declared stable (src/cli/exit.rs:1-2).
  The underlying outcome is a typed NotSubmitted built from ApiError::host_unavailable (tests/cli/topology_handoff.rs:904), so the work is retryable. A script branching on exit status reads a usage error.
  No test asserts the error code: the writer tests added in 84237aad only check the error is not RunError::Io. The message string appears only at line 1439.
  Severity stays Nit: the pending report with retry_argv is written first and the message states the retry remedy, so this is a misclassification, not a lost outcome.
  fix-shape hint: Return the error class that matches the typed outcome (host unavailable, 3) or a dedicated retryable code, and assert the exit status in the writer tests.

- [Nit] src/cli/topology_runtime.rs:440 — The post-publication pre-writer failure report is tested only with no observed canonical bootstrap status; the observed-state branch, the "continuation" phase label, and the silence for observed Completed/Cancelled bootstraps have no test. (new) [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/cli/topology_runtime.rs:340-475: the continuation closure records compound-filtered `observed`; the Err arm guards on observed not Completed/Cancelled and derives phase None|Prepared|PossibleCreation => "creation", else "continuation".
  write_pending_observed (src/cli/topology_handoff.rs:1712) returns invalid_request("pending bootstrap status differs") for Completed/Cancelled, so removing the guard would replace the original error.
  The only public test of this route is tests/handoff_topology_cli.rs:1713, which asserts 0 bootstrap_handoffs rows, attempt null, status_unknown true and phase "creation" (observed was None).
  No test asserts a "continuation" phase from this route. The only other write_pending_observed caller in tests (tests/cli/topology_handoff.rs:2832) passes None and "creation" directly to the writer.
  No test drives a lock, capacity or SetupEnv failure after an observed Created/Attached status. Mutations (always "creation", dropped compound filter, removed Completed/Cancelled guard) would pass the suite.
  Severity Nit: the gap is in error-path report labelling with no state mutation.
  fix-shape hint: Add runtime-level tests that fail after an observed Created/Attached status (assert phase "continuation" and the observed fields) and after an observed Completed/Cancelled status (assert the original error propagates with no report).

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [FYI] src/host/transport.rs:819 — Native tab.create is sent without a pre-write method/schema check, so every post-write refusal (including unknown_method/invalid_params) becomes OutcomeUnknown and a stuck bootstrap blocks archival until a human runs handoff recover.
  verdict: rejected (reproduce REJECT / refute REJECT / ground REJECT).
  reason: The deterministic premise fails: tab.create is a long-standing Herdr operation present on every supported release (floor 0.9.1), so no supported release lacks it. The remaining forward-compatibility case is the design's explicit rule.
  evidence: https://herdr.dev/docs/cli-reference/ lists `tab create` with --workspace, --cwd, --label, --env, --focus/--no-focus; the socket API doc lists tab.create with workspace_id/cwd/focus; herdrdev/herdr#4935 shows tab.create with cwd on 0.9.1-preview. tests/herdr-releases.tsv supports 0.9.1-0.9.3.
  docs/superpowers/runs/2026-10-07-handoff-topology/2026-10-07-handoff-topology-design.md:64: "Only proven NotSubmitted evidence may close that attempt... if the transport cannot prove it, unknown is mandatory. No guessed error class resets the boundary." Operator recovery and the archival veto while a bootstrap is live are designed behaviour.
  The exact socket schema per release was not fetched by any seat, so the floor claim rests on documentation rather than a schema diff. A pre-write schema gate like request_guarded_start (src/host/transport.rs:574-583) remains a hardening idea only.

## Unverified nits (spot-checked)
- [FYI] src/harness/launch.rs:266 — The frozen-V1 fix copies the whole Codex acceptance grammar into harness::launch::v1 although compose_bootstrap_v1_argv only returns the caller argv unchanged or refuses, re-coupling future Codex grammar changes to Root bootstrap launches through a nonexistent plan version 2.
  verdict: unverified nit; one refute-seat spot check REJECTed at FYI, not panel-strength verification.
  evidence: The spot seat read src/harness/launch.rs:253-290 and callers in src/cli/handoff.rs (~1581, ~1068). The frozen copy is the documented design ("Do not edit this to follow current launch behavior: it is a compatibility record"); divergence fails closed with "a new bootstrap plan version is required" and a tripwire test. The proposed `compose_native_argv == request.argv` guard would let the V1 definition float with the current grammar, the drift the iteration 1 finding at attachment.rs:627 asked to remove. Treated as a duplication preference, not a defect.

## Escalations (need human)
- none

## Prior-report tracking (iteration 1)
- [Blocking] src/cli/topology_runtime.rs:534 — recovery rearm on invalidated assertion: resolved (not re-surfaced by any lane).
- [Should-fix] src/cli/topology_handoff.rs:1423; src/cli/topology_handoff.rs:1398 — zero-submission rearm bypasses the recovery report: regressed (fixed incompletely; the report is now rendered, the exit status 2 residue is confirmed above at Nit).
- [Should-fix] src/cli/handoff_delivery.rs:761 — existing-peer delivery omits partial report: resolved.
- [Should-fix] src/store/topology_handoff/attachment.rs:627 — retained reports validated against current composition: resolved (frozen V1 contract in 57881273; the spot-checked duplication nit above concerns the shape of that fix).
- [Should-fix] src/archival_legacy.rs:712 — per-child directory lookup amplification: punch-listed (open).
- [Should-fix] src/cli/topology_handoff.rs:1678 — pending reports emit a nonexistent `pending` subcommand: resolved (f10c3df7).
- [Should-fix] tests/cli/topology_handoff.rs:3307 — zero-submission tests stop below the writer: resolved (writer tests added in 84237aad).
- Previously rejected (standing): src/store/schema.rs:529 schema 27 forward migration; src/cli/topology_handoff.rs:530 dropped native cause. Neither re-surfaced.
---
