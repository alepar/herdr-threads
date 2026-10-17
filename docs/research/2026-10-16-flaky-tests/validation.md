# Verification

Base 82786da05; branch fix/v2-flakes; verified code commit 97755c280c089eda3b7e43249efabcf7c21ce31e. Independent review found no must-fix issues. No merge or push performed.

| Check | Result | Wall time |
|---|---|---|
| Focused nextest stress, ten relevant tests × five iterations | 50/50 passed; final iterations overlapped full-suite startup | 37.925s |
| Full `nice cargo nextest run --locked --all-targets --all-features` | 7,602 passed, 48 existing ignored/skipped, zero failures | 289.180s runner / 289.45s command |
| `scripts/check-no-leaked-processes --run-id` | No leaked test processes or temp files | 0.76s |
| `nice cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed | 58.65s |
| `nice cargo clippy --locked --all-targets -- -D warnings` | Passed | 54.72s |
| `nice scripts/check-default-features` | Passed | 40.93s |
| `cargo fmt -- --check` | Passed | — |
| `scripts/check-harness-neutral` | Zero violations | — |
| `git diff --check` | Passed | — |

Leak run ID: `1bda9736-c1df-478e-9097-874a32bf96ec`. The full run stayed inside the strict 300s budget. Its slowest test was the release failpoint proof (259.337s); neither test timeout nor suite budget was raised. Both clippy configurations stayed below one minute. Structured stage results: verification-results.json.

Controlled failing-first evidence is preserved in [reproductions](reproductions.md). The current V2 product watch defect is ht-x0b9. Historical ht-7ykh remains open because its older v1.2.0 path/schedule was not reconstructed. Broader static candidates remain tracked under ht-fmww.

Research checks: coverage structural validator passed (three rounds, explicit budget-exhausted stop); claim-support checker passed all fifteen factual ledger entries; report surface and package validators passed; external citation URL validator confirmed all nine external URLs accessible. Local execution evidence was checked by reading the retained record and local link validation, not counted as an independent external document. Retrieval saturation and dynamic confirmation of the broader audit candidates are not claimed.

All suite fixtures and helpers completed; run-scoped cleanup found no leaks. Compiler/lint helpers exited when their commands completed. No authenticated release wake gate was run for this non-release patch task.

## ht-41yy follow-up after the original batch

The test-only `tests/multinode/sweep.rs` follow-up was verified separately after the original batch; the earlier 7,602-test full run does not cover this later test edit. Per the project instruction to avoid routine full reruns, this narrow change used its focused regression plus required checks.

- Controlled partition RED: exact hosted inbox assertion failed (3.41s).
- Reconnect with recipient-side wait GREEN: passed (7.38s).
- Final code, temporary partition/heal removed: 20/20 nextest stress iterations passed (83.788s runner; 84.96s command).
- Unique leak run `7460272b-3fbb-4bd2-8fc1-3d4209b2ff67`: no leaked test processes or temp files (0.25s).
- All-target/all-feature clippy passed (0.34s cached incremental check); default-feature gate passed (0.12s).
- Format, harness-neutral and diff checks passed; independent review found no must-fix issues and verified atomic message/delivery materialization.
- Production code, timeouts, delivery/wake assertions and convergence oracles remain unchanged.

Structured results: sweep-followup-verification.json. The coordinator receives the new commit for the 2.0.1 batch; this is non-blocking for 2.0.0.

## ht-2b3p relay follow-up

Before changing the fixture, verified the intentionally retained audience fence with `send_fence::membership_lifecycle_eligibility_config_bumps_still_conflict` (1/1 passed). The original relay case passed20/20 under four owned low-priority CPU loops; exact hosted CLI refusal was not reproduced. Those load processes were terminated/reaped; baseline leak run `baf318d0-f155-4fc1-8c2a-0c35b7aa5651` passed cleanup. A diagnostic delay through convergence before C's open failed the added independent-open prerequisite (Some1 vs Some0,1.75s), demonstrating the fixture ordering gap rather than claiming an exact CLI-error reproduction.

Final code isolates C-B through both independent pair opens, observes linked then backoff, asserts C has no pair before its open, heals and keeps every original relay oracle. Ten stress iterations of both config_relay tests and neighboring crossed-pair test passed30/30 (32.659s runner;33.88s command). Leak run `f76dcc79-6428-4246-871e-848894905cef` passed (0.25s); all-feature/all-target clippy passed (0.33s cached incremental), default-feature gate passed (0.12s), format/neutral/diff checks passed. Independent review found no must-fix. The earlier full-suite run predates this test-only follow-up; no additional full run was performed, per the project's focused-test instruction. Structured records: relay-baseline-verification.json and relay-followup-verification.json.

Product classification was reported to the coordinator before implementation: changed audience snapshots are intentionally refused; this patch neither weakens that contract nor depends on error retries/recovery argv.

## ht-dc68 cold-host hook follow-up

Original assertion reproduced2/20. Temporary service error instrumentation reproduced3/20 (~146ms each), all with the exact current-epoch reconciliation ServiceBusy. A sandboxed attempt failed before daemon readiness due local socket restrictions and is excluded. Diagnostic removed. Initial correction passed30/30 (7.932s); final version adds no-allocation assertion and passed all11 multi_host tests ×5 =55/55 (30.795s runner;33.40s command). Scoped leak run `dd6e777e-5825-42f0-9d1c-29cf03d596b4` passed (0.25s). All-target/all-feature clippy passed25.45s; default-feature gate passed9.76s. Format, harness-neutral and diff checks passed. Independent review found no must-fix; optional no-allocation oracle adopted. Research16/16 factual claims verified; coverage/report/package validators pass.

Structured results: multi-host-followup-verification.json. No production changes, full-suite rerun or ordinary-hook retry loop. Earlier full-suite evidence predates this test-only follow-up. Classification/impact were sent before correction. Test flaw is resolved under the explicit policy; real startup enrollment usability concern remains OPEN separately as `ht-bryo`, and release acceptance belongs to the coordinator.

## ht-bryo product fix and complete release qualification

User decision supersedes the earlier nonblocking recommendation: the cold second-host startup gap blocks2.0.0. The test-only ht-dc68 correction was an intermediate step; the final product fix completes an empty saved-seat recovery pass in the canonical publication transaction. The integration oracle again requires FIRST-callback enrollment, without resume fallback. Saved-seat/hold refusal semantics and C1 hook retry rules remain intact.

Failing-first worker-free regression: `nice cargo test --locked --all-features --lib cold_empty_host_continuity_is_decidable_before_the_worker_pass` failed with exact ServiceBusy reconciliation-pending before the production edit, then passed (0.11s test,24.72s build). Preliminary qualification exposed two mistakes in the new store test itself (private HostId accessor at compile time, then retired-at fixture CHECK); both were corrected before the final green qualification. The failed focused run was stopped by SIGINT to its owned runner and scoped cleanup passed. These are not counted as passing product verification.

Final qualification on the complete candidate:

- Focused `nice cargo nextest run --locked --all-features -E 'test(cold_empty_host_continuity_) | test(multi_host::) | test(snapshot_eligibility::) | test(continuity_) | test(record_reconciliation_pass_)' --stress-count 3`:100 tests ×3 =300/300 passed,21.802s runner/26.20s command.
- Full `nice cargo nextest run --locked --all-targets --all-features`:7,604/7,604 passed,48 existing skips,220.569s runner/220.81s command, below300s budget. Existing release-build failpoint proof60.907s.
- Scoped leak run `3d86fe2d-4d66-41b6-8db8-a54d369d5e1e`:process/temp checks passed after focused0.26s and full0.53s.
- All-target/all-feature clippy26.93s and default-feature clippy26.41s passed, both with locked dependencies and warnings denied.
- `nice scripts/check-default-features`12.70s, harness-neutral1.77s and `cargo fmt --check`5.37s passed. Diff check passed.
- `scripts/herdr-release-smoke`:all5 listed Herdr releases0.9.1–0.9.5 ×5 cases =25/25 passed,398.27s; private server/daemon cleanup passed. This is the stand-in harness compatibility matrix, not the authenticated release wake e2e.
- Independent design investigation and code review found no must-fix. No new model-visible production strings, timers, waits, retries, skips or serialization were added.

Structured results:cold-host-product-verification.json. Per-release matrix:cold-host-product-release-matrix.md. The matrix header reports predecessor HEAD a3bec8c1a because the candidate diff was tested before its final commit; production/test code was committed without further edits. The main release coordinator receives this commit with the whole fix/v2-flakes batch and owns final merge/release qualification. The user changed the intended release target from the prior2.0.1 follow-up to the2.0.0 candidate.
