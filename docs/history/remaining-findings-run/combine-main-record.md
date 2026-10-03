# Combined candidate tree: main merged into remaining-findings

Branch `combine/main-into-remaining` (worktree `.worktrees/combine-main`), created from the run tip
`ea4692f9` (super-auto/remaining-herdr-threads-findings); `git merge --no-ff main` with main at `826a8804`.
Fork point `55512edb`; main brought 82 commits (B5 trust epic ht-rzi, b5 follow-ups ht-6ry / ht-6y1 /
ht-p63, ht-xoc harness version evidence spec), the run branch 509.

Merge commit: `eccfc030`. Fix commits after it are listed under "Test results". Final tested tip: `b8123d4d` (this record is committed on top of it).

Resolution rules followed (from the integration plan, run.md integrationNote-2026-10-01): main's B5
guards and ht-xoc are authoritative where they meet this branch's B4/B5-adjacent code; this branch's
B1-B4/B6-B10 work is authoritative in its own scope; where both changed one function the intents were
combined. Every hunk was resolved individually (no one-side bulk resolution). Where both sides appended
different tests at the same end-of-file spot and git interleaved the two appends into several hunks
(tests/daemon/transport.rs, tests/lifecycle_ux.rs, tests/scheduler/dispatch.rs, tests/store/wake.rs),
the file tail was rebuilt as "this branch's appended block, then main's appended block", after checking
that both blocks are pure appends after the fork point's end of file.

## Conflicts (47 files, 101 hunks)

### Code and scripts

| File | Hunks | Resolution |
| --- | --- | --- |
| `src/store/schema.rs` | 1 (+ duplicated helpers git merged cleanly) | `V10` = main's `0010_b5_trust_guards.sql`, new `V11` = `0011_cooperative_only.sql`. Fresh DB runs V1..V11 and stamps 11. Every upgrade arm gains `migrate_v10_to_v11`; new `10 =>` arm (verify v10 shape, migrate, verify); `11 => verify_existing`. `verify_existing` = `verify_existing_v10_shape` + `verify_v11_b1` (renamed from `verify_v10_b1`; error texts say v11). Removed the duplicate `verify_existing_v9_shape`/`migrate_v9_to_v10` that the textual merge produced; added `verify_existing_v10_shape` and `migrate_v10_to_v11`. |
| `migrations/0010_cooperative_only.sql` | rename | `git mv` to `0011_cooperative_only.sql`; header comment says v11 (SQL below the B1 marker unchanged, so the v11 audit is unchanged). |
| `src/daemon/lifecycle.rs` | 1 | Kept this branch's `live_skew` path in `handshake` (descriptor read before decode, stale descriptor = not running). Main's `protocol_mismatch_error` removed; main's `check_protocol` kept (the hook's descriptor-only check) but now returns `skew_error(UnknownWireVersion, ..)`, i.e. the one VersionSkew `remedy()` line. |
| `src/cli/mod.rs` | 3 | `connect`: this branch's `skew_guard` (live skew vs stale descriptor) instead of main's bare `check_protocol`. Caller derivation: this branch's `LazyConnection`/`run_caller_scoped` plus main's `RuntimeContext` threaded through `run_caller_scoped` -> `derive_caller` -> `derive_selection` (main's A4 agent-evidence read needs it). `run_caller_scoped` got an `allow(clippy::too_many_arguments)`. |
| `src/cli/doctor.rs` | 1 | This branch's `skew_error` remedy text (main's "stop it by hand" text is superseded by the skew-tolerant `daemon stop`). |
| `src/cli/me.rs` | 1 | Both arms: main's `Unauthorized` arm (discard the frozen human event, A4) followed by this branch's `Conflict` arm. |
| `src/host/native.rs` | 1 | Both: this branch's `pane_agent_state` / `send_submit_key` (Wave 28 submit verification) and main's `observe_pane_agent` (A4). |
| `src/ports.rs` | 1 | Both `HostPort` methods (as above). Also (clean-merged code) main's three new `StorePort` default bodies (`replay_continuity`, `decide_continuity`, `record_reconciliation_pass`) made required, per B4 decision 2 (no `Unsupported` defaults; `SqliteStore` implements all three). `ReconfirmStructure` doc updated for C4 carry-forward and the baseline hold lift. |
| `src/identity/reconcile.rs` | 1 | Ours: the blanket `impl ObservationStore for T: StorePort` stays deleted (B4 decision 2c); main's new trait method `record_reconciliation_pass` (clean-merged into the trait) is implemented by `ScheduledStore` in `service/workers.rs`. |
| `src/store/seats.rs` | 5 | Imports: `CARRIED_BINDING_PROVENANCES` (main) without `ReceiptRegistration` (B4). Transition validation: `MarkOccupantUnavailable` and `Reconfirm` arms dropped (B4/P10), main's `CarryForward` combined with `ReconfirmStructure` (resolved-seat check first). Proof match: `ReconfirmStructure | CarryForward | Move`. Application: main's `MarkOccupantUnavailable` application dropped. Native `register_available`/`revoke_registration` (main had only threaded `operator` and `CARRIED_BINDING_PROVENANCES` through them) dropped; the cooperative `register_available` already carries main's `operator` argument. All six `lift_baseline_hold_if_clear` call sites of main are present. |
| `src/store/mod.rs` | 1 | `seats::register_available(.., &request.command, request.operator.as_ref(), permit, budget, ..)` (no registration argument). |
| `src/store/queries.rs` | 1 | This branch's linear `fit_candidates` page fitting; main's `open_binding` field added to both the fit closure and the final `SeatInspection`. |
| `src/service/dispatch.rs` | 1 | `RegisterAvailableRequest { command, read, operator }` (no `registration`). |
| `src/service/workers.rs` | 2 | Combined: main's `pass_complete` refusal tracking and durable `record_reconciliation_pass` (only after a refusal-free pass, C2) plus this branch's Health `record_reconcile_page` (success-only `last_reconciliation_at`, B3) and `begin_reconcile_pass`; continuation tuple carries the refused flag. Main's `record_reconciled` call dropped (superseded by the success-only rule). |
| `src/notification/dispatch.rs` | 1 | `mut target` (main) with this branch's `Refused(RefusalCause::Unsafe)` for a missing safe target; main's new "cooperative reservation without bound harness" refusal also returns `Refused(RefusalCause::Unsafe)` (a pre-send refusal must not climb the ladder, pacer D5). |
| `src/protocol/ids.rs` | 1 | Main's (more precise) entropy comment. |
| `scripts/install.sh` | 1 (+ clean-merged code) | Header describes the combined order. Code: one "stop the old daemon (upgrade)" block before the swap: Herdr's stop action with the still-installed old package (main ht-6ry), else the old executable's `daemon stop` (this branch, Herdr down / `--no-herdr`, or the action failing because an old executable lacks `internal json-field`), else main's pid warning. The branch's separate Herdr-down `daemon stop` inside the swap block was folded into that block. The truthful final status (next_steps/finish) is unchanged. |
| `AGENTS.md` | add/add | One file: "Development rules" (this branch) then "Trust policy" (main). |

Compile-time fixes in clean-merged code, made in the merge commit: `ErrorCode::DaemonBootChanged`
(main) got a class (`Transient`), an `ALL` entry in declaration order, a constructor
(`daemon_boot_changed`) and a row in `tests/protocol/error_class.rs`. Clippy on the combined tree:
`Reattach::Done` boxed (`large_enum_variant`, main's hook code with this branch's larger `CheckedIn`),
the A4 human-check-in guard in `seats.rs` collapsed into one let-chain (`collapsible_if`).

### Docs

| File | Resolution |
| --- | --- |
| `docs/superpowers/specs/INDEX.md` | All rows of both sides. |
| `docs/design/herdr-threads/2026-09-27-herdr-threads--seat-identity-design.md` | Main's `seat retire` / `seat rebind --replace` bullets, then this branch's superseded note (extended: the operator actions remain current, TRUST-POLICY.md normative). |
| `docs/operations.md` | This branch's logs/remedy section and remedy table, then main's protocol-2 paragraph rewritten for the combined behaviour (descriptor check, VersionSkew remedy, skew-tolerant stop, installer order). |
| `docs/install.md` (clean merge, edited) | Upgrade/skew paragraphs reconciled with the combined installer order and the skew-tolerant `daemon stop`. |
| design docs (clean merge, edited) | `herdr-threads-design.md`, `--store-design.md`, `src/store/retention.rs` comment: "schema v10 (0010_cooperative_only)" -> v11 (0011). |

### Tests

| File | Hunks | Resolution |
| --- | --- | --- |
| `tests/cli/commands.rs`, `tests/cli/hook.rs`, `tests/cli/launch.rs`, `tests/harness/launch.rs`, `tests/integration.rs`, `tests/integration/sweep.rs` (2nd), `tests/service/worker_health.rs`, `tests/store/queries.rs`, `tests/store/schema.rs` | 1-2 each | Both sides' appended tests/modules kept. |
| `tests/contracts.rs`, `tests/integration/{follow,latency,token_diet,sweep}.rs`, `tests/lifecycle_ux.rs` (13), `tests/hook_entrypoint.rs` (6), `tests/setup_cli.rs` (2), `tests/service/{composition,retirement_health}.rs` (3+1) | 30 | This branch's `scrubbed_command` (B9 isolation) over main's `Command::new(..).envs([test_owner_env()])` (ht-6y1). Combined by making `test_support::isolation::scrub_env` set `TEST_OWNER_PID_ENV` after scrubbing (the one sanctioned `HERDR_` variable), so every scrubbed child carries the owner pid; the isolation test allows that key. |
| `tests/integration/operator_ux.rs` | 1 | Main's `run_with_env` split with `scrubbed_command`. |
| `tests/daemon/transport.rs` | 6 | Tail rebuilt (branch's serve-loop/drain tests, then main's expected-boot tests); main's `BootCheckService` got the unserved-route macro. Malformed-frame list: only the forged-`operator_actor` frame at `PROTOCOL_VERSION` (a foreign version now gets the decodable skew reply). |
| `tests/lifecycle_ux.rs` | 2 (interleaved) | Tail rebuilt: branch's startup-log tests, then main's owner-watch / guard tests. |
| `tests/scheduler/dispatch.rs` | 5 (interleaved) | Tail rebuilt; main's `CooperativeRecordingHost` lost `subscribe_lifecycle` (B4) and gained `pane_agent_state`/`send_submit_key`; its no-harness assertion is `Refused(RefusalCause::Unsafe)`. |
| `tests/store/wake.rs` | 5 (interleaved) | Tail rebuilt (branch's recovery-walk/refusal tests, then main's bound-harness tests). |
| `tests/identity/reconcile.rs` | 1 | Ours: the native-registration block (B4-deleted; main only added an argument). |
| `tests/store/cooperative_checkin.rs` | 3 | `None` operator argument; two native-only tests stay deleted (B4 list). |
| `tests/store/control.rs` | 4 | The 515-line native-registration block stays deleted (B4 list); three calls get the `None` operator argument. |
| `tests/cli/cooperative.rs` | 3 | `derive_caller(.., &runtime, &paths, &no_connection(), &clock)`; main's A4 tests kept, `ApiError` built with `ApiError::unsupported`, skew text assertion moved to the remedy line. |
| `tests/release/install_test.sh` | 2 | This branch's structure (no `HT_RELEASE_TEST_HERDR` mode; isolated-Herdr matrix) plus main's signal traps, stop-failure injection, action-versions recording and section 3b (now unconditional, stub Herdr). Main's watchdog rewritten for the matrix: isolated roots are appended to `$root/ih-roots` and the watchdog kills their servers and daemons once the script is gone. |

Clean-merged tests adjusted to compile against the combined API: `tests/store/receipts.rs` (no
`fence` argument, W5-1), `tests/cli/{read_cost,read_cost_seam,read_cost_names}.rs` (runtime context,
`open_binding`), `tests/identity/reconcile.rs` (`bound_epoch`), `tests/store/retention.rs`
(`operator: None`), `tests/service/worker_health.rs` (main's marker tests drive the observation lane's
Pacer), `tests/cli/{hook,launch}.rs` and `tests/hook_entrypoint.rs` (fakes implement the branch's
`call_with_output` / submit-verification methods, `ApiError::new`, no `subscribe_lifecycle`).

## Design reconciliations

**Schema.** Main's B5 migration keeps v10 (it already shipped on main). The run's cooperative-only
migration (B4: no table dropped; B1: partial indexes + `work_jobs.completed_at`) becomes v11. Both
paths are covered: `tests/store/schema.rs` bumps every "latest version" assertion to 11, renames the
branch's v10 tests to v11, and adds `main_v10_store_upgrades_to_v11_with_both_migrations` (a populated
main-v10 store keeps its B5 marker, kind and diagnostic rows and gains exactly the B1 objects, second
start a no-op) and `fresh_and_main_v10_stores_share_the_v11_shape`. Accepted limit: a store created by
the unreleased run branch at its own v10 (cooperative-only, no B5 columns) is refused as incompatible
by the combined build; no release ever carried that numbering.

**Protocol and skew.** One behaviour: `PROTOCOL_VERSION = 2` (main). Every client reads the published
descriptor's protocol before sending anything (main ht-rzi.23 and B3 Decision 3 agree). The CLI and
`ensure` use `live_skew` (a stale descriptor whose owner is gone reads as "not running"; a live one
gives `unknown_wire_version`); the native hook uses the descriptor-only `check_protocol` (fail fast,
`daemon protocol: UnknownWireVersion`). The message is always the single `remedy(VersionSkew)` line:
"daemon is version X (protocol N), CLI is Y (protocol M): run `herdr-threads daemon stop` then
`herdr-threads daemon ensure`" — main's "stop with the older executable, else kill pid by hand" text is
retired because B3's skew-tolerant `daemon stop` (no wire Stop; lock + descriptor confirmed, SIGTERM)
makes the remedy work from the newer CLI. The daemon still answers a foreign-version request with a
decodable skew reply. Tests now use the real release pair, protocol 1 vs 2: `tests/daemon/skew.rs`
(`OLD_PROTOCOL = 1`, a guard test pins `PROTOCOL_VERSION == 2`), `tests/integration/wire_compat.rs`
(skewed descriptor at `PROTOCOL_VERSION - 1`; the frozen `tests/fixtures/old_wire/*` frames are
protocol 1, so `old_protocol1_cli_gets_decodable_skew_replies` now asserts the decodable skew reply
with only protocol-1 keys instead of a successful health/history exchange), `tests/daemon/transport.rs`,
`tests/daemon/control.rs`, `tests/daemon/lifecycle.rs`, `tests/cli/{hook,cooperative}.rs`.

**B4 vs B5.** TRUST-POLICY.md on main lists P10 (EmptyShell) and W5-1 (decision fence) as "remove"
(C5, A2), matching the run's B4 deletion. Main had only threaded new arguments through the deleted
native paths (`operator`, `CARRIED_BINDING_PROVENANCES`, `bound_epoch`, a `None` registration), so the
deletions stand; main's live behaviour (C4 `CarryForward`, baseline hold lift, C1 continuity,
A4 guards, retire/replace) sits on the kept cooperative path. Post-merge acceptance rg
`allocate_seat|revoke_registration|CallerVerifier|MutationPermit::new|decision_fence|ProvenEmptyShell|proven_empty_shell_bridge|MarkOccupantUnavailable`
over `src/` and `tests/`: no hits. Main's three new `StorePort` default bodies were made required
(B4 decision 2).

**Installer.** Order on upgrade: stop with the old executable before replacing files (main ht-6ry),
Herdr action first, the old executable's `daemon stop` as fallback (this branch's Herdr-down case),
pid warning if a daemon survives; then the swap, registration, `ensure`, setup, and the branch's
single truthful final status line and exit code. Mid-run warnings from ht-6ry (pid + manual kill)
are kept.

**Test-daemon leaks.** Main's ht-6y1 owner watch and this branch's B9 scrubbed environments are
combined in `scrub_env` (see Tests above).

## Test results

All commands run in `.worktrees/combine-main`. The full suite is
`nice cargo test --locked --all-features --no-fail-fast -- --test-threads=1 < /dev/null` (stdin from
`/dev/null` as in CI: with an open terminal/pipe stdin a pre-existing hook test leaves a thread holding
the stdin lock and `harness::bridge_tests::cooperative_typed_rejection_...` blocks in `read_body`; this
code is unchanged since the fork point and unrelated to the merge).

### Final: `b8123d4d`

`cargo fmt --check` ok; `nice cargo check --locked --all-targets --all-features` ok;
`nice cargo clippy --locked --all-targets --all-features -- -D warnings` ok; B4 acceptance rg: no hits.

| Target | Result |
| --- | --- |
| lib (unit) | 1685 passed, 0 failed, 16 ignored |
| bin (main.rs) | 0 tests |
| contracts | 57 passed |
| hook_entrypoint | 36 passed, 8 ignored |
| host_adapter | 24 passed, 1 ignored |
| integration | 73 passed |
| lifecycle_ux | 22 passed |
| local_endpoint | 8 passed |
| package | 12 passed, 1 ignored |
| service | 83 passed |
| setup_cli | 29 passed |
| view | 20 passed |
| doctests | 1 passed |

Exit 0, no failures.

`tests/release/install_test.sh target/debug/herdr-threads` (stub Herdr + isolated real-Herdr matrix;
never the shared server): every check passes except the two "Herdr not on PATH" matrix cases,
`upgrade_herdr_absent: old daemon stopped` and `uninstall_herdr_absent: daemon stopped`. Both fail
identically on the run tip `ea4692f9` alone (verified by running the same script there), so they are
pre-existing, not merge-caused: with `herdr` absent the old executable's `daemon stop` cannot find the
daemon a named-session Herdr started. With those two cases removed from the case list (temporary copy
of the script, not committed) the combined tree reports `INSTALL_TEST_PASS 179 checks (stub herdr +
isolated real herdr matrix)`.

### How we got there

| SHA | Run | Outcome and fix |
| --- | --- | --- |
| `eccfc030` (merge) | full suite | lib 658 failed, hook_entrypoint 32, integration 48, lifecycle_ux 7, service 61: one cause, `verify_query_connection` still pinned schema 10 (both sides had bumped the literal to 10). Fixed in `2a4b4dc4` with `LATEST_VERSION = 11`. Also `continuity_refusal_table_is_pinned` (main's pinned table vs this branch's `StaleRequirementAcceptance` deterministic rejection). |
| `2a4b4dc4` | full suite | lib 5 failed, integration 1, service 1-2: v11 stamp in `legacy_v1_startup...`; main's receipts/wake tests assumed main's fixtures (adapted to this branch's setup and B1 wake discovery); `canary_payloads` emitted flags for the codex top-level `resume` form main now refuses; loop inventory needed reasons for main's test-only sleeps (`0dfc9498`). `resolution::real_ipc_exact_resolution_replay_after_retirement_and_hold_...` (deterministic) and `operator::elected_operator_rebind_and_fresh_...` (flaky) failed on the combined tree but passed on both the run tip and main: fixed in `3758f54e` (marker write under the lane lock; the resolution test keeps an unresolved seat so main's hold lift does not clear the hold it asserts). `latency::history_and_health_stay_fast_under_a_party_with_a_slow_host` failed once; it passed on the run tip and main and 2/2 on the combined tree after `3758f54e`. |
| `3758f54e` | full suite | only `trust_policy::expected_boot_and_carry_forward_...` failed (flaky ~50% on the combined tree, 6/6 green on main): its `wait_reconciled` could see the old daemon's marker; fixed in `b8123d4d` (6/6 and the whole trust_policy module 3/3 green). |
| `b8123d4d` | full suite | all green (table above). |


Commits on `combine/main-into-remaining` after the run tip:

- `eccfc030` merge
- `2a4b4dc4` `LATEST_VERSION` / continuity refusal table fix
- `0dfc9498` merged-test adaptations
- `3758f54e` reconciliation marker under the observation lane lock (+ resolution test premise)
- `b8123d4d` trust_policy test race
- this record

## Unresolved / follow-ups

- Installer with `herdr` absent from `PATH` cannot stop a daemon a named-session Herdr started
  (pre-existing on the run tip; 2 matrix checks).
- A pre-existing test leaves a stdin-draining thread that blocks later stdin reads when the suite runs
  with a live stdin; run the suite with `< /dev/null` (CI already does).
- A store created by the unreleased run branch at its own v10 numbering is refused by the combined
  build (expected: no release carried it).
- Doc drift outside the conflict set was fixed only where it named the renumbered migration or the
  skew behaviour (operations, install, design docs); the run's own spec text still says "v10" for B1/B4
  as written history.
