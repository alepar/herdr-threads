super-roast verdict: Should-fix (5 confirmed) [converged]
mode: pr        iteration: 2 of 3
profile (assumed): Local developer tool, pre-release: a Herdr plugin (Rust daemon + CLI) running on the operator's own machine over per-user Unix sockets. Distributed publicly via GitHub releases and a curl|bash installer that edits user-level Claude Code / Codex config. Small single-operator user base; persists real user data in local SQLite with forward migrations. Identity is cooperative, not a security boundary.
inputs: super-auto/remaining-herdr-threads-findings@ea02aa89 vs main@826a8804
delta vs prior: 2 new confirmed (0 Blocking) · 3 carried (0 Blocking) · 4 resolved · 0 regressed (0 Blocking)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, dependency, hygiene-docs) · raw 6 → deduped 4 → panel 2 · spot 2 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0

## Confirmed findings
- [Should-fix] src/scheduler/mod.rs:258; src/service/workers.rs:1208 — `drive_wakes` falls back to `SubmissionVerification::NotChecked` for every reserved attempt where the dispatcher recorded no verification (pre-send refusals, Cancelled, TimedOut, Unavailable, OutcomeUnknown), and the new `observe_drive` consumer forwards all of them to daemon.log as `wake prompt submission not checked`, so routine refusals dominate the not_checked line and its counts and hide real verifier failures. [fix-regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: scheduler/mod.rs:256-266 pushes `take_verification(seat).unwrap_or(NotChecked)` for every `Ok(Some(_))` from `try_candidate`; `try_candidate` returns `Ok(Some(outcome))` at ~588 for every reserved attempt, including Refused/Cancelled/TimedOut/Unsafe/Unavailable/OutcomeUnknown. The dispatcher inserts into `verifications` only in the `PromptOutcome::Submitted` arm (src/notification/dispatch.rs:245-249). `observe_drive` (workers.rs:1207-1215, new in de2b02db, first production consumer of `.verification`) forwards every NotChecked to `record_wake_verification`; src/daemon/logs.rs:209-226 writes `lane wakes: verification not_checked: seat X: wake prompt submission not checked`, throttled per (scope, code), so the single seat-named first line and the `repeated N times` counts are shared by refusals and genuine verifier misses. Contradicts docs/operations.md:89 ('wake prompts whose submission could not be checked'), the `WakeDriveOutcome.verification` doc (mod.rs:88, 'results of sent prompts') and the `record_wake_verification` doc ('A sent wake prompt...'). New test `observe_drive_reports_unchecked_and_unsubmitted_wakes` builds `WakeDriveOutcome` by hand and never goes through `try_candidate`. On main `observe_drive` (workers.rs:977) never read `.verification`, so the mislabel is introduced here. Diagnostic only: wake delivery and Health unaffected. Seat severities Should-fix / Nit / Should-fix.
  severity note: profile down-weights observability findings, considered. Held at Should-fix because this is the fix for the prior-round nit (dispatch.rs:91) and it replaces a silent gap with a misleading signal that contradicts the operations.md text edited in the same round; the fix is one line. This is the round's only Should-fix and is a fix-regression, not new feature work.
  fix-shape hint: push a `(seat, verification)` entry only when `take_verification` returns `Some` (drop the `unwrap_or(NotChecked)`), or skip non-Submitted outcomes in `observe_drive`; add a test that drives a refused attempt through `drive_wakes` and asserts no not_checked line.

- [Nit] src/notification/dispatch.rs:243 — The new post-send verification (250 ms settle plus up to two pane reads and a submit key, bounded only by the attempt's `lease_end`) runs inside the `run_owned_attempt` watchdog. A daemon stop during that window, or a slow advisory read that runs to `lease_end`, discards the dispatcher's `Submitted` and records `OutcomeUnknown` for a prompt that was already delivered.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: dispatch.rs:243-249 calls `verify_submission` after `PromptOutcome::Submitted` and before returning; its deadline is `min(context.budget.deadline, now+PROMPT_MILLIS)`. scheduler/mod.rs ~725-760 `run_owned_attempt`: a 5 ms watcher sets `expired` on `caller_budget.cancellation.is_cancelled() || now >= lease_end`; line ~756 `if !joined || expired || now >= lease_end { return Ok(WakeOutcome::OutcomeUnknown) }` discards the worker's `Ok(Submitted)`. On main `attempt_wake` returned right after `submit_prompt` (main dispatch.rs:181), so the window is new; the watchdog override itself is pre-existing (main scheduler:663-680), only widened by ~250 ms to 2 s. Effects: store/wake.rs:571 writes `last_outcome='outcome_unknown'`; `WakeDispatch::record_outcome` resets refusal backoff only on Submitted. Ladder step is identical for both outcomes, so no duplicate prompt. All three seats Nit. Distinct from the prior Nit on the 250 ms settle (throughput) and the now-fixed uninterruptible settle.
  fix-shape hint: capture the Submitted result before verification and let a cancelled/expired verification return NotChecked without the watchdog overriding the send outcome; or give `verify_submission` a deadline strictly before `lease_end`.

- [FYI] src/harness/codex_config.rs:318 — `foreign_network_proxy_keys` filters out the whole `features.network_proxy.unix_sockets` table, so user-owned socket allow entries are neither recorded as foreign nor warned about while setup's `network_access=true` makes them effective. (still-open, see iteration 1)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓) — iteration 1 panel; not re-raised this round
  evidence: branch at ea02aa89 still has `.filter(|(key, _)| !matches!(*key, "enabled" | "unix_sockets"))` at codex_config.rs:318 and the doc comment at :308 still describes it that way; no commit between f79901da and ea02aa89 touches this filter. Pre-existing on base: main already set `network_access=true` with no foreign-key handling at all (src/harness/codex_config.rs on main), so final severity is FYI by rule and it does not drive a fix round.
  fix-shape hint: iterate `unix_sockets` entries, skip only the daemon-socket key, report the rest as foreign dotted keys; reword "exactly one Unix socket" in setup.rs ~1699 and docs/install.md:208.

- [Nit] src/host/native.rs:1024 — Every successfully sent wake prompt blocks the serial `drive_wakes` loop for a 250 ms settle plus a `pane read` RPC (up to two of each plus a send-keys when the composer still holds the prompt); a page of 16 candidates can consume most of the 5 s drive budget. (still-open, see iteration 1; partially addressed)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓) — iteration 1 panel; not re-raised this round
  evidence: de2b02db made the settle cancellable (`wait_blocking(Duration::from_millis(COMPOSER_SETTLE_MILLIS))` at native.rs:1030), which was the second half of the iteration-1 fix-shape hint. The first half is not done: this round's dispatch.rs:243 packet shows `verify_submission` still runs inline inside `attempt_wake`, so the settle and reads still occupy the serial drive loop per sent prompt. Bounded as before: unprocessed candidates stay in `scan.pending` and the worker re-drives with a fresh budget.
  fix-shape hint: run verification after the send loop (or batch the settle across the page's sent prompts) so the drive loop is not held per prompt.

- [Nit] src/store/schema.rs:542 — `migrate_v10_to_v11` stamps `user_version` with the moving constant `LATEST_VERSION` instead of the literal `11`; after a future bump to 12, a v10 store is stamped 12 after only V11 ran, and a crash before the v12 step leaves a store that `12 => verify_existing` refuses. (still-open, see iteration 1)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓) — iteration 1 panel; not re-raised this round
  evidence: branch at ea02aa89 still reads `.and_then(|_| conn.pragma_update(None, "user_version", LATEST_VERSION))` at schema.rs:542 with `LATEST_VERSION = 11` at :43; the only other `LATEST_VERSION` stamp is the fresh-install path at :127. Latent today since the constant equals 11. Not on main.
  fix-shape hint: one-token change to the literal `11`; keep `LATEST_VERSION` only in the fresh-install `0 =>` path.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this round. No iteration-1 rejection was re-surfaced by the scouts; all eleven iteration-1 rejections stand as written there.

## Unverified nits (spot-checked)
- tests/host/wake_submission.rs:116 — The fix pass's `marker_fifth_from_bottom_reads_submitted` test says it pins `COMPOSER_TAIL_LINES = 4`, but against the new prompt-anchored parser (native.rs ~1360-1393, `rposition` on the last `>`/`›` line, then join below it) it passes for every window size 1-10; the only branch where the window matters is the `None => tail.concat()` fallback, which no fixture reaches. Follow-on to the iteration-1 nit at native.rs:54, which asked for this boundary to be pinned; the fix added the fixture but the parser change made the fixture window-insensitive, and the test comment still cites the removed "whole-line rule". The wrapped-marker-in-narrow-box risk on the fallback branch is unverified against Claude's real composer. (spot: CONFIRM Nit — test-coverage and stale-comment gap, not a functional bug; add a fixture with no prompt line in the tail, and one with a wrapped marker, and fix the comment.)
- CHANGELOG.md:13 — The bullet added in 637bd050 for the `doctor --json` Claude reshape names only the moved `installed` bool; `hooks.claude.settings` → `hooks.claude.setup.settings` and `hooks.claude.adopted` → `hooks.claude.setup.adopted` moved in the same hunk (doctor.rs:473-480) and are not listed, so a script reading the old paths gets null with no error. Follow-on to the iteration-1 doctor.rs:493 nit, which is resolved for the key it named. (spot: CONFIRM Nit — v0.1.0 unpublished, two secondary JSON keys, one-line doc edit: name all three moved keys or the whole `hooks.claude.setup` object.)

## Escalations (need human)
- none

## Prior-report tracking (iteration 1 confirmed findings)
- resolved: src/host/native.rs:1029 fenced advisory reads — `early_exit_output` (native.rs:389) and `pane_agent_state` (native.rs:1038) now call `run_unfenced` (de2b02db).
- still-open: src/harness/codex_config.rs:318 (FYI, pre-existing) — filter unchanged; listed above.
- resolved: tests/host/wake_submission.rs:7 missing fixtures — eight fixtures now cover the prompt-anchored parser including a Codex-shaped composer (per this round's wake_submission.rs:116 spot evidence); residual fallback-branch gap recorded as an unverified nit above.
- still-open (partially addressed): src/host/native.rs:1024 settle blocks drive loop — settle is cancellable; verification still inline; listed above.
- still-open: src/store/schema.rs:542 `LATEST_VERSION` stamp — unchanged; listed above.
- resolved: src/cli/doctor.rs:493 CHANGELOG bullet for `hooks.claude.installed` — added at CHANGELOG.md:13 (637bd050); sibling-key omission recorded as an unverified nit above.
- resolved: docs/install.md:188 and docs/agent-usage.md:144 stale admission contract — both now describe the listed / schema-matched / optimistic / refused ladder and the doctor PATH check; the "Until the doctor PATH check lands" sentence is gone (637bd050).