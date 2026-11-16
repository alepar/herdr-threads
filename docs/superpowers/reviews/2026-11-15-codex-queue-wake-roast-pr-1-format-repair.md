super-roast verdict: Should-fix (4 confirmed)
mode: pr        iteration: 1 of 3
profile (assumed): Production local same-user cooperative messaging tool. Wake mistakes cost user attention and quota; durable migration or message/receipt corruption is material. Same-user cooperative trust is explicit; adversarial caller hardening is outside this profile.
inputs: feat/codex-queue-control@6e4d55cba vs main@4dabf06ef
coverage: scouts 15/15 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, deploy-safety, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs) · raw 5 → deduped 4 → panel 4 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "Captured immutable full branch6e4d55cba and base4dabf06ef; complete git diff and repository available in dedicated detached worktree; historical scoped/full/live evidence and post-rebase mapping in docs/superpowers/runs/2026-11-14-codex-queue-wake."
independence: same-family (OpenAI Codex) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 4 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=4) · reproduce 4/0/0 · refute 4/0/0 · ground 4/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 1/1/0/0 · concurrency-async 1/1/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 1/1/1/0 · testing 1/1/1/0 · test-causality-lifecycle 1/1/1/0 · test-isolation-environment 0/0/0/0 · test-suite-cost 0/0/0/0 · hygiene-docs 0/0/0/0

The panel used static review and in-memory models. It did not rerun the full tests or reproduce these timing paths live.

## Confirmed findings

- [Should-fix] src/notification/queue_recovery.rs:280; src/notification/queue_recovery.rs:278 — Repeated pagination of fully cleaned wake-intent history can prevent daemon idle exit indefinitely. [lanes: hot-path-perf, concurrency-async]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All seats confirmed that recovery selects retained history, sets work=true on every nonfinal page, and resets the cursor after the last page. With 97 cleaned rows, four five-second pages repeat true,true,true,false. ActivityProbe consumes this flag and the 30-second IdleTimer samples can repeatedly reset its ten-minute window. Pruning occurs only on a subsequent prepare. The reproduce seat's in-memory transcription showed no idle exit over a simulated hour; this was not a live daemon reproduction.
  fix-shape hint: Separate pagination progress from pending cleanup activity so completed history cannot keep resetting idle exit.

- [Should-fix] scripts/wake-e2e.py:1944 — Queue qualification can fail a successful wake when cleanup replaces its required Add-helper witness before the sampler observes it. [lanes: test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: The 100 ms sampler is unsynchronized with Add, Accepted, native Start, and cleanup. activation.rs:65-85 permits the current helper witness to be replaced, and src/notification/queue_recovery.rs:437-443 starts List cleanup. The independently timed five-second sweep gives the Add witness no minimum lifetime. The reproduce seat executed queue_proof in memory: equivalent accepted/fulfilled evidence failed with no_correlated_accepted_with_baseline_and_add_witness when the earlier Add snapshot was omitted. No live failure frequency was established.
  fix-shape hint: Preserve immutable Add-attempt evidence or an ordered event stream that qualification can inspect after cleanup.

- [Should-fix] src/harness/codex/wake/control.rs:502 — The Add-response ownership boundary lacks negative behavioral coverage for unrelated or missing correlation and invalid returned IDs. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All seats found that control.rs:496-508 currently validates the returned ID and exact correlation correctly. tests/fixtures/codex_queue_helper.py:39 always supplies a valid ID and echoes the requested correlation. Existing failure modes do not exercise parsed negative Add responses, and List ownership tests cover a separate predicate. Accepted IDs become durable owned_ids and cleanup deletion targets. The claim that removing the equality check would escape coverage is a static inference, not an executed mutation test; no current production ownership defect was found.
  fix-shape hint: Add parsed Add-response cases with mismatched correlation, missing correlation, and invalid IDs; assert Unknown and no accepted ownership.

- [Nit] src/notification/queue.rs:228 — A failed result transaction after a possible queue effect discards the database error, leaving an unexplained unresolved intent. [lanes: observability]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: The existing accepted_effect_with_result_write_failure_is_unknown_and_never_native fault test shows Armed state, absent owned IDs, and both transport fences after result persistence fails. Err(_) if possible_effect discards the error. Same-boot Armed recovery returns successfully, so its cleanup logger supplies no cause. All seats confirm the missing diagnostic; reproduce and refute rate it Nit, while ground rates it Should-fix. The conservative Unknown behavior preserves safety.
  severity basis: Nit follows the two-seat assessment of limited troubleshooting impact in this local cooperative tool; no profile-driven demotion from their severity is applied.
  fix-shape hint: Emit a bounded diagnostic for the failed result write while preserving Unknown and both transport fences.

## Not verified (beyond panel cap)

- none

## Not verified (dedupe failed or judge lost)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

- none

## Unverified nits (spot-checked)

- none

## Escalations (need human)

- none
