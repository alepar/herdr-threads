# Integration sweep notes (ht-p03.51)

Root integration sweep for the remaining herdr-threads findings: the unknown-unknowns net for seams no per-seam bead
owned. Run on the integration tip plus the ht-p03.131 stack parent (owned test children, leak checker), macOS arm64,
isolated named Herdr sessions only (never the shared server), `TMPDIR` a short path (see Environment).

## Environment note: TMPDIR length

Unix socket paths are limited to 104 bytes on macOS (`SUN_LEN`). With `TMPDIR` set to `<worktree>/.tmp`
(about 170 bytes in this nested-worktree layout) 43 library tests that bind a socket under `std::env::temp_dir()` fail
with `path must be shorter than SUN_LEN`, not with a product defect (first full run: `1649 passed; 43 failed`).
`docs/release.md` already tells operators to point `TMPDIR` at a short private directory for the same reason. The
recorded runs below used `TMPDIR=/private/tmp/ht51`, a symlink to the worktree's `.tmp` (so every file lands inside the
worktree); the symlink is removed at the end of the task.

## 1. Main flows (item 1): what exercises each, with outcome

"Serial" is `cargo test --locked --all-features --test integration -- --test-threads=1` on this tip before the sweep
tests were added: `78 passed; 0 failed; 1 ignored` (the ignored one is the leak probe child). "Full" is the final
`cargo test --locked --all-targets --all-features --no-fail-fast` with no thread pin.

| Flow | Exercised by | Outcome |
|---|---|---|
| send -> commit -> wake within 100 ms, no tick wait | existing `lanes_latency::send_is_attempted_within_100ms_without_a_tick_wait`, `lanes_latency::deadline_commit_creating_a_warning_wake_is_attempted_within_100ms`; new `sweep_remaining::send_commit_wake_under_100ms_with_all_five_lanes` (three recipients, all five lanes registered first) | pass |
| idle daemon commit/wakeup bounds across the five lanes | `lane_wiring::idle_five_lanes_30s`, `lanes_latency::idle_daemon_commits_nothing_from_deadline_wake_request_for_30s`, `lane_wiring::every_lane_registers`; new `retention_runs_alongside_the_wake_idle_bound` (same bound with a 3,000-row retention backlog) | pass |
| Herdr stopped/restarted under a running daemon | `lanes_latency::herdr_stopped_bounds_wake_commits_per_seat`, `herdr_down_log` (stopped, capped summaries), `observation_outage`, `config_smoke::herdr_*` (4 states) | pass (serial and full) |
| retention keeps snapshot / work-job tables bounded, discovery flat in settled history | new `retention_keeps_tables_bounded_while_discovery_stays_flat` (6,000 settled jobs: send latency under 100 ms before and after, drained in at least 24 bounded batches, `preparation_cleanup` kept, snapshot generations <= 6); cost counters: `store::discovery_cost::wake_discovery_is_flat_in_settled_history`, `store::discovery_cost_tests::{work_discovery_is_flat_in_completed_jobs, observation_walk_is_flat_in_retired_seats_and_superseded_generations, recovery_walk_is_flat_in_retired_and_unreserved_rows}` | pass. Limit: the production discovery functions are private to the store, so the daemon-level test measures latency, not VM units; the units are the store tests |
| startup failure -> that attempt's own startup log tail | `operator_text::ensure_startup_failure_prints_remedy_and_attempt_log`; new `startup_failure_lane_failure_and_skew_reach_the_operator` phase 1 | pass |
| lane failure -> daemon.log + Health `degraded: ...` | `lane_wiring::injected_lane_failure_logs_and_degrades_then_clears`, `operator_text::degraded_lane_in_health_prints_see_log`; new `lane_failure_surfaces_through_remedy_text_within_the_health_line_budget` | pass |
| version skew both directions, stop-then-ensure remedy works | `wire_compat::skewed_protocol_version_still_yields_the_stop_then_ensure_report`, `wire_compat::old_protocol1_cli_gets_decodable_skew_replies`, `daemon_skew::*` (6); new test phase 3 | pass |
| optimistic admission for a newer harness, known_broken refusal (test recipe override), doctor --json Claude fields | `doctor_admission_seam::{listed_claude_is_listed, newer_claude_is_optimistic, known_broken_claude_is_refused, missing_claude_is_not_found}_and_the_canary_*` (stub binaries + `HT_TEST_RECIPES_JSON`), `cli::doctor::doctor_json::*` | pass |
| re-observation on binary change | `service::workers::admission_reobserve_tests::swapping_the_binary_reobserves_on_the_next_tick` and `cancelling_the_lane_kills_a_hung_harness_and_ends_the_pass` (the production `AdmissionReobserver` on the lane's Pacer; a swapped stub binary is observed on the next tick and the change is logged) | pass. No daemon-level variant was added: the in-process daemon reads the test process's own `PATH`, which cannot be swapped per test |
| canary: `--self-test`, a probe, report -> `file_issues.py --dry-run` | `bash scripts/harness-canary.sh --self-test` -> `Ran 11 tests ... OK`, exit 0; `python3 -m unittest test_seam_integration test_file_issues test_probe_contract` in `scripts/canary` -> `Ran 23 tests ... OK` (the probe runs there are `--probe` with fixtures and `--keep`; the report -> `file_issues.py --dry-run` chain is `test_file_issues`) | pass. A real-registry `--probe` needs network and npm and was not run; `canary-smoke.md` records the real-registry runs |
| installer fresh / upgrade / uninstall against an isolated Herdr; release workflow shape | `bash tests/release/install_test.sh` -> `INSTALL_TEST_PASS 195 checks (stub herdr + isolated real herdr matrix)`; `actionlint` -> exit 0; `bash tests/release/workflows_test.sh` -> `all workflow checks passed`; `rg 'uses: [^@]+@v[0-9]' .github/` -> no match (no tag pins) | pass after the fix in section 4 (before it, `install_test.sh` aborted in teardown) |
| human read/show/participants cost counts, agent-facing presentation contract | `cli::read_cost_tests::show_and_participants_use_one_connection_and_two_calls`, `cli::follow::read_cost_seam::one_human_read_session_meets_every_leaf_bound`, `cli::follow::read_cost_names::*`, `cli::human::golden_contract::*`, `token_diet` integration | pass |

Full suite on the final tree (no thread pin, `--no-fail-fast`): library `1692 passed; 16 ignored`, `contracts 57`,
`hook_entrypoint 36 (8 ignored)`, `host_adapter 24`, `integration 84 (1 ignored)` (the five new sweep tests and the
multibyte test included), `lifecycle_ux 23`, `local_endpoint 8`, `package 12`, `service 81 passed; 2 failed` (fixed, see
section 4), `setup_cli 29`, `view 20`. The two `service` failures were re-run after the fix:
`cargo test --locked --all-features --test service composition -- --test-threads=1` -> `19 passed`.

## 2. Added (cross-bucket tests, `tests/integration/sweep_remaining.rs`)

- `send_commit_wake_under_100ms_with_all_five_lanes` (B1 x B4)
- `retention_keeps_tables_bounded_while_discovery_stays_flat` (B4 x B1)
- `retention_runs_alongside_the_wake_idle_bound` (B4 x B1; 30 s)
- `lane_failure_surfaces_through_remedy_text_within_the_health_line_budget` (B2 x B3): all five lanes fail together; Health
  stays within `HEALTH_LINE_BUDGET`, folds into one `5 lanes degraded (...)` line carrying the daemon.log path, each lane
  logs once; heal returns Health to its healthy lines.
- `startup_failure_lane_failure_and_skew_reach_the_operator` (B3 x B6): a startup failure, a degraded lane and a skewed
  descriptor each print their own `remedy()` text; the skew report is not the lane pointer, and with the skew removed the
  degraded state is still reported.
- `hook_parse_detail_cut_is_the_wire_bound_for_multibyte_text` (finding 3a below).

Not added, with the covering test named in section 1: `optimistic_admission_and_known_broken_and_doctor_claude_fields`
(`doctor_admission_seam`), `binary_change_is_reobserved` (`admission_reobserve_tests`),
`capability_gated_messages_across_skew` (`wire_compat`: `cli_against_an_old_fixture_daemon_never_sends_full_bodies`,
`hook_parse_failure_under_optimistic_sends_no_report_to_an_old_daemon`, both pass), and
`startup_failure_lane_failure_and_skew_reach_the_operator`'s three classes as separate tests (they exist per class in
`operator_text` and `wire_compat`; the new test adds the interaction).

The new tests reuse the existing `lane_wiring::Session` and `lanes_latency::{Session, Scene}` fixtures; their items are
now `pub(crate)` (no behaviour change). The send-latency tests use the `lanes_latency` fixture because the
`lane_wiring` one puts a freshly checked-in plain-shell recipient on the 30 s wake ladder (the first attempt then waits
for the tick), which is the fixture's documented shape, not a daemon defect.

## 3. Unwired-value sweep (item 3): findings and dispositions

| Check | Result | Disposition |
|---|---|---|
| `const` defined but never read (317 `const [A-Z_]+:` in `src`; any with one occurrence in `src`) | `HOOK_PARSE_DETAIL_BYTES` (never read; the CLI cut at a literal 256 characters and the wire validated a literal 256 bytes); `VERSIONS_JSON_PATH` (read by `tests/harness/versions_json_guard.rs` only, a deliberate pub test-visible path) | fixed inline: constant and `bounded_hook_detail` in `protocol/commands.rs`, used by the CLI and by validation, test `hook_parse_detail_cut_is_the_wire_bound_for_multibyte_text`; `VERSIONS_JSON_PATH` kept (read) |
| every lane registered with `CommitKicks` and a `WorkerStatus` | `Lane::ALL` = deadline, wake, observation, admission-observer, retention; each registered in `run_elected` (`src/app.rs`: `register_lane` for deadline, wake and admission observer, `kicks.register` for observation and retention) and each has a `WorkerStatus` | no gap; checked at runtime by `every_lane_registers` and the new first test (`registered` and `status_has_pacer` for all five) |
| capability constants in `ADVERTISED` | `HISTORY_FULL_BODIES`, `HOOK_PARSE_FAILURE_REPORT` both advertised; `every_advertised_capability_has_a_handler` and the wire_compat tests pass | no gap |
| test-only overrides absent from a release build | `cargo build --locked --release` then `strings target/release/herdr-threads \| grep -E 'HT_TEST_\|HT_CANARY_'` -> no output (exit 1) | no gap |
| release-build warnings (default features) | `unused imports Lane, LaneSet` in `src/store/mod.rs` and `type IdleHook is never used` in `src/service/pacer.rs`: items only the `test-support` surface uses, so a default-feature `clippy -D warnings` (the CI job exists) would fail | fixed inline: both gated with `#[cfg(any(test, feature = "test-support"))]`; `cargo clippy --locked -- -D warnings` clean |
| CLI flags documented but not parsed (every `--flag` in `docs/agent-usage.md`, `install.md`, `operations.md`, `README.md` against clap and `scripts/install.sh`) | 77 distinct flags; every one is parsed by `herdr-threads` (clap), `scripts/install.sh` (`--version --setup --no-setup --no-herdr --prefix --bin-dir --force --uninstall --yes`), `scripts/demo-tea-party.sh` (`--dry-run`), or belongs to another tool the docs name (`herdr pane split --no-focus`, `codex exec --ignore-user-config`, `--dangerously-bypass-hook-trust`, `claude --setting-sources`, `cargo --locked/--release`) | no gap |
| every new `docs/operations.md` statement backed by code | Logs: daemon.log 1 MiB (`MAX_LOG_BYTES`), 30 s window (`LANE_LOG_WINDOW_MS`), startup log prune 24 h / keep 8 (`STARTUP_LOG_MAX_AGE`, `STARTUP_LOG_KEEP`), eight-hex nonce (`logs.rs`); backoff 100 ms doubling to 30 s with +-20 % jitter (`pacer.rs` `CAP_MS`, doc header); retention tick once a minute (`RETENTION_TICK`), 256 rows per transaction (`RETENTION_BATCH_ROWS`), 24 h job age and `completed_at IS NULL` immediate (`WORK_JOB_RETENTION_MS`, `retention.rs`), kept `preparation_cleanup` (`PRUNED_JOB_KINDS`), deadline safety tick 5 s; fold of more than two lanes (`LANE_LINES_BEFORE_FOLD`); remedy table (`cargo test remedy_table`); skew-tolerant `daemon stop` (`daemon_skew::remedy_round_trip_stops_the_old_daemon`) | one unbacked statement found and repaired (section 4); the rest backed as listed |

## 4. Fixed inline (each with its evidence)

1. `docs/operations.md`: the Health pointer was documented as `degraded: see <instance>/daemon.log`. The real line is
   `degraded: ` plus the `LaneDegraded` remedy (`degraded: temporary; retry the command; see <instance>/daemon.log` for a
   transient failure), and when more than two lanes are degraded Health emits only the folded summary line (which ends
   `: see <instance>/daemon.log`) with no separate `degraded:` line. Statement rewritten.
2. `tests/service/composition.rs`: two tests (`elected_service_opens_real_sqlite_and_routes_health_and_public_query_over_ipc`,
   `elected_service_reports_actual_private_settings_over_ipc`) still matched the pre-B3 text `degraded: see ` and so failed
   deterministically on the integration tip (a cross-bucket regression: B3's `remedy()` changed the pointer, the ht-p03.27
   tests were not updated). Predicates are now `starts_with("degraded: ")`. Re-run: 19 passed.
3. (a) `HOOK_PARSE_DETAIL_BYTES` was never read; the CLI cut the hook parse-failure detail at 256 characters while the wire
   validation rejects more than 256 bytes, so a multi-byte `{error:?}` was refused by the daemon and the report silently
   lost. Fixed as in section 3.
4. `src/app.rs` (test-support `LaneProbe` log sink): `writeln!` on a `File` is two writes, so five lanes failing at once
   interleaved their lines in daemon.log ("...failurelane retention: ..."). One `write_all` per line now; production
   (stderr, locked per `writeln!`) was never affected. Found by the all-lanes-fail test.
5. `scripts/lib/isolated-herdr.sh`: `ih_kill_pids` ran `kill -TERM "$@"` as a plain statement; `ih_root_pids` also lists
   its own `ps`/`grep` pipeline, which has exited by then, so under `set -e` (as `tests/release/install_test.sh` runs) the
   teardown aborted the whole script with exit 1 after the 112th check. `|| true`. Re-run: `INSTALL_TEST_PASS 195 checks`.
6. `docs/release.md`: the canary smoke record lists configuration `3b` as NOT_EXERCISED but the follow-on item did not name
   it, failing `tests/release/docs_test.sh` (`follow-on dispatch covers NOT_EXERCISED configuration: 3b`). The bullet now
   names it; `DOCS_TEST_PASS`.
7. Release/default-feature warnings fixed (section 3).

## 5. Filed

None. Observations that are not defects, recorded for the reviewer:

- The folded Health line (`N lanes degraded (a, b, c): see <log>`) spells `see <log>` itself instead of calling `remedy()`
  (the path still comes from the daemon-log accessor). Two tests pin that exact text
  (`tests/daemon/health_budget.rs`, `tests/service/worker_health.rs`); left as designed.
- Leak checker run after the final test run: `scripts/check-no-leaked-processes --run-id <this run>` lists only processes of
  other worktrees (`thread-summaries-compaction-survival` tasks and the ht-p03.26 task's isolated `herdr server`), none
  of this task's; `stop-run-processes --check` for this worktree is clean.
- Flakiness is out of this task's scope (a separate sidequest owns it). The full no-thread-pin run was made once, after
  fixes 3 to 5 and 7; everything every later edit touched (fixes 1, 2 and 6 and a test comment) was re-verified by its own
  targeted run, listed in section 4, not by a second full run.
