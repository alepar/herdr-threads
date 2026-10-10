# Integration sweep: agy harness adapter (ht-poh.11)

Root sweep of the agy adapter epic on the integrated branch (`super-auto/agy-harness-adapter` with the
unmerged stack parents ht-poh.35 and ht-poh.36 merged in, main v0.6.0 absorbed at `d342add2`).
Written 2026-10-10. Status: every flow below passes; the gaps found are listed under "Findings".

## Flows checked

| # | Flow | Evidence | Verdict |
|---|---|---|---|
| 1 | Bare agy enrollment: child vs `/clear`, resume, startup context | `tests/hook_entrypoint/agy.rs` (existing: top-level, child, clear, inv0 resume, state-file rebind, no-lock) | passes |
| 2 | HookPull pull -> inject -> transcript confirm -> settle with `cooperative_hook_delivery` | unit round trip in `tests/harness/agy_delivery.rs` (`daemon`), and new end to end `agy_pull_confirm_and_the_claude_watch_path_coexist_on_one_daemon` (real `hook agy` through `sh -c`, elected daemon, scripted transcript) | passes |
| 3 | Claude mod `PushChannel` unchanged | DP10 files show no diff against `f3431b31` and `d342add2`; DP10 suites, `golden_surfaces::`, `mod_delivery::` all pass; the new coexistence test keeps a live `watch` channel beside the agy seat | passes |
| 4 | `wake_on` all/nudge across Native and Channel seats | new `wake_on_all_wakes_a_native_hook_pull_seat_for_a_lazy_row_and_nudge_never` and `wake_on_all_promotes_a_lazy_row_on_a_live_channel_instead_of_a_native_wake` | passes |
| 5 | `check-harness-neutral` strict | `ok - 0 violations, 85 allowlisted (to-migrate 0, frozen-wire 49, history 2, registration 34), 51 entries` | passes |
| 6 | setup/unsetup agy, launch/handoff agy through bootstrap V2 | `tests/harness/agy_setup.rs`, `agy_launch.rs`, `tests/handoff_agy.rs` (`handoff_kind_agy_composes_through_bootstrap_v2_and_starts`, `..._without_the_plugin_is_refused_with_the_setup_hint`), `scripts/agy-gate` offline suites | passes |

## Wiring table

`declared` is the trait or constant, `implemented by` the adapters that override the neutral default,
`called from` the core call sites (registry forwarders omitted). Unwired means declared but never
called, or a value nobody reads.

| interface | declared | implemented by | called from | verdict |
|---|---|---|---|---|
| `delivery_policy` | `harness/adapter.rs:190` (default `None`) | claude `claude.rs:986` (PushChannel), agy `agy/mod.rs:223` (HookPull), fourth fixture | `store/receipts.rs:147`, `service/dispatch.rs:651`, `harness/bridge.rs:1224`, `delivery/mod.rs:27`, `cli/hook.rs:2115,2180,2590` | wired |
| `DeliveryPolicy::mechanism` | `adapter.rs:340` | claude, agy | `delivery/routing.rs:26,38`, `delivery/mod.rs:28`, `service/dispatch.rs:652`, `store/receipts.rs:148` | wired |
| `DeliveryPolicy::idle_wake` | `adapter.rs:345` | claude (Channel), agy (Native) | `service/dispatch.rs:693` -> `ModChannelRegistration.idle_wake` -> `service/mod_channels.rs:589` (`wake_suppressed`) -> `scheduler/mod.rs:620`; registry pairing check `registry.rs:1225` | wired |
| `DeliveryPolicy::provenance` | `adapter.rs:342` | claude (`cooperative_mod_delivery`), agy (`cooperative_hook_delivery`) | `store/receipts.rs:150` (PushChannel only); registry pairing check `registry.rs:1225-1233` | wired by validation: a HookPull settlement writes the constant `COOPERATIVE_HOOK_DELIVERY_PROVENANCE` (`store/receipts.rs:184`) and the registry refuses any other value for HookPull, so nothing can drift; no runtime read of agy's own value |
| `local_evidence` | `adapter.rs:182` (default empty) | agy `agy/mod.rs:211` -> `witness.rs:90` | `cli/hook.rs:2785` | wired |
| `hook_step` | `adapter.rs:186` (default `None`) | agy `agy/mod.rs:220` (`AgyHookStep`) | `cli/hook.rs:3113`, top-level `Observer` events only | wired |
| `settings_kind` | `adapter.rs:206` | claude `claude.rs:948`, codex `codex.rs:1336`, agy `agy/mod.rs:191` | `harness/setup.rs:2294` | wired |
| `AdapterText.setup_scope` | `adapter.rs:249` | claude, codex, hermes, agy | `cli/help.rs:220` | wired |
| `AdapterText.setup_foreground` | `adapter.rs:251` | claude, codex (agy empty by design) | `cli/help.rs:226` | wired |
| `AdapterText.setup_notes` | `adapter.rs:253` | claude (agy empty) | `cli/help.rs:232` | wired |
| `AdapterText.launch_notes` | `adapter.rs:255` | codex, hermes, agy | `cli/help.rs:249` | wired |
| `AdapterText.config_root_hint` | `adapter.rs:257` | claude, codex, agy (`HOME`) | `cli/setup.rs:465`, `cli/help.rs:184` | wired |
| `AdapterText.trust_reminder_label` | `adapter.rs:259` | codex (agy empty: no trust reminders) | `cli/setup.rs:1091` | wired |
| `AdapterText.version_history_note` | `adapter.rs:261` | claude, codex (agy empty: no runtime version) | `daemon/harness_states.rs:569` | wired |
| `agent_env_markers` | `adapter.rs:365` (metadata) | claude, codex, agy (`ANTIGRAVITY_AGENT`, person guard) | `harness/registry.rs:1294,1347` -> `cli/mod.rs:263`, `cli/output.rs:175`, `cli/picker.rs:39` | wired |
| `foreground_advice` | `adapter.rs:209` (default `None`) | claude, codex; agy none by design (agy-adapter design: hooks inherit the pane environment) | `cli/setup/foreground.rs:63` | wired; agy default is intended |
| `setup_status_extras` | `adapter.rs:213` | claude, agy `agy/setup.rs:526` | `cli/setup.rs:719` | wired |
| `setup_report_lines` | `adapter.rs:215` | claude, agy `agy/setup.rs:537` | `cli/setup.rs:1057` | wired |
| `doctor_text` | `adapter.rs:219` | claude, codex, agy `agy/doctor.rs:265` | `cli/doctor.rs:1309,1363` | wired |
| `doctor_summary_claims` | `adapter.rs:223` | codex (agy `&[]` by default) | `cli/doctor.rs:1397` | wired; agy default is intended |
| `version_syntax` | `adapter.rs:226` | claude, codex; agy `None` by design | `harness/contract.rs:131` | wired; agy default is intended (agy-adapter design line 113) |
| `transcript_version` | `adapter.rs:229` | claude, codex; agy `None` by design | `harness/attribution.rs:100` | wired; agy default is intended |
| `wake_on` (column, `WakeOn`) | `migrations/0029_seat_wake_on.sql`, `protocol/commands.rs` | set by `SetWakeOn` (`store/seats.rs:4729`) | read at `store/seats.rs:4706` by `store/wake.rs:482` (reservation recheck), `store/attention.rs:1067,1149` (wake examination), `store/queries.rs:6100` (promotion `wakes`), `store/messages.rs:743` and `store/lazy_delivery.rs` (publication reason bit), `delivery/assemble.rs:142` | wired |
| `seat.wake_on_v1` | `protocol/capabilities.rs:32` | daemon advertises | `cli/wake_on.rs:60` (gate) | wired |
| `delivery.pull_v1` (`DELIVERY_PULL`) | `protocol/capabilities.rs:48` | daemon advertises | none in the hook: by design the hook step never probes capabilities (an older daemon fails to decode and the hook prints nothing, delivery-policy design) | advertisement only, deliberate |
| bootstrap plan V2 | `harness/launch.rs:429-466` | any launch-capable kind (agy via `AgyLaunch`) | `cli/handoff.rs:1037,1582-1620`, `cli/topology_handoff.rs:195,278` | wired; agy exercised by `tests/handoff_agy.rs` |

No unwired interface found. The two "by design" rows are the only values without a runtime reader.

## Findings

1. **Lifecycle hooks with the daemon down do not print nothing.** The orchestrator update asked for tests that agy
   `SessionStart` and the inv0 resume/clear lifecycles "print nothing and exit 0 with the daemon stopped". The
   design's end-to-end bullet ("the daemon being down prints nothing and exits 0") and R4's output discipline
   ("on daemon unavailability it prints nothing") describe the routine `hook_step` events, which already have
   that test (`with_the_daemon_down_a_routine_hook_prints_nothing_and_exits_zero`). Lifecycle events go through
   the core check-in, which ensures a daemon within the 5 s lifecycle budget and, when the check-in is refused,
   prints the locked-shape notice `herdr-threads: check-in unavailable (...)` for every harness, Claude and Codex
   included. Making agy silent there would be a core special case for one harness. The new test
   `with_the_daemon_down_the_lifecycle_hooks_exit_zero_and_enroll_nothing` pins what holds for agy, for a top-level
   `SessionStart`, a `/clear` `SessionStart`, and the fresh and resume inv0 turn starts, each run with the production
   (unscaled) budgets: exit 0 inside the 5 s lifecycle bound (plus slack), at most the one locked-shape
   `injectSteps` step, that step is the unavailable notice (never the startup context), and no binding is
   enrolled. Not changed; recorded for the human gate.
2. **`SessionStart` never settles an agy lazy row as `cooperative_hook_context`.** agy declares `HookPull`, so
   `delivery::routing::hook_inbox_page` is false for it at every boundary and a hook-pull seat's lazy rows go
   only through `DeliveryPull`/`DeliveryConfirm` (also enforced in the store: `complete_inbox_delivery` with
   `via=hook_context` is refused for a hook-pull harness, `store/control.rs`). The new test
   `agy_lazy_row_settles_only_through_the_transcript_confirm_never_hook_context` runs the real hook against a
   pending lazy row: the `/clear` `SessionStart` shows no inbox block and leaves the row `pending`, the turn-start
   pull injects the body (still `pending`), and only the transcript-confirmed `Stop` makes it `displayed`.
3. **No double delivery between main v0.6's lazy hook delivery and HookPull** (ht-poh.17's model): the same test
   shows exactly one delivery path for the row; `tests/cli/hook_pull_lazy.rs` pins the absence of an inbox page and
   of `CompleteInboxDelivery` from the standard hook for the fourth fixture.
4. **Docs match what landed.** README agy row, `integrations/agy/README.md`, `docs/agent-usage.md` and the agy
   SKILL tail (`AGY_SKILL_GUIDANCE`) describe hook-pull delivery, the composer idle wake and the minimal
   `herdr-threads: wake up` text; none claims R17 compact framing. One inline fix: the operator guide said an
   idle agy "receives only nudge-class messages", which omitted `wake-on all`; it now names it.
5. **Commit trailers.** `scripts/outside-adapter-report` lists many commits as missing their `Outside-adapter`
   trailer, including recent ones (`f98b7495`) whose trailer sits in its own paragraph: `git` parses trailers only
   from the final paragraph, so it must be in one block with `Co-Authored-By`. This task's commits follow that.
6. **Inline fix: `service::loop_inventory` was red on the integration branch** (also at `d2c83d0e`, this task's
   base). The adapter-seam move of the shell probe and the `--version` helper out of `src/cli/launch.rs` and
   `src/harness/codex.rs` into `src/harness/launch.rs` and `src/harness/process.rs` left four stale ALLOWLIST
   entries (still naming `codex.rs`) and four unlisted hits. The four entries now name the new files with their
   original reasons (`tests/service/loop_inventory.rs`).
7. **Flaky tests.** Three beads filed, none root-caused (not reproducible: 6 solo runs and 36 runs at 12x parallel
   load, all green): `ht-01s` (`golden_surfaces::handoff_bootstrap_plans`), `ht-vrg`
   (`handoff_topology_cli::explicit_pane_preserves_both_harness_configs`), `ht-2w9`
   (`handoff_topology_cli::final_sdd_actual_native_byte_and_line_limits_refuse_before_effects`). The handoff
   topology file bounds its polling helper at 15 s (`WAIT`, `tests/handoff_topology_cli.rs:42`) against 60 s in
   `golden_surfaces.rs`, the likeliest cause under full-suite load. The single full run added a fourth, also
   unreproducible (5 solo runs, 48 runs at 16x load): `ht-veq`
   (`topology_activation::elected_public_new_tab_completes_one_native_create_and_start_then_replays_history`,
   `StaleHostObservation: snapshot target set incomplete` in the test's own setup).

## Tests added

- `tests/integration/agy_sweep.rs` (new; a child module of `mod_delivery` so it reuses that sweep's rig, added with
  one `#[path]` line in `tests/integration/mod_delivery.rs`; run under the `mod-delivery` nextest group):
  `agy_pull_confirm_and_the_claude_watch_path_coexist_on_one_daemon` (a),
  `wake_on_all_wakes_a_native_hook_pull_seat_for_a_lazy_row_and_nudge_never` and
  `wake_on_all_promotes_a_lazy_row_on_a_live_channel_instead_of_a_native_wake` (b),
  `a_clear_rotation_discards_the_pull_ledger_and_reoffers_the_pending_message` (c).
- `tests/integration/sweep.rs`: the stand-in Herdr answers `agent.read` for an `agy` pane with the recorded empty
  composer capture, so a native wake of an agy pane is observable.
- `tests/harness/agy_delivery.rs`: `a_clear_rotation_discards_the_agy_pull_ledger` (service level: `not_offered` for
  the new generation, `stale_generation` for the old claim, fresh offer, then settle).
- `tests/hook_entrypoint/agy.rs`: the daemon-down lifecycle test and the lazy-row test above.
- `tests/service/loop_inventory.rs`: the four ALLOWLIST entries re-pointed at the moved probes (finding 6).

## Guards run

- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
- `nice scripts/check-default-features`: clean. `scripts/check-harness-neutral`: ok, 0 violations.
- DP10: `git diff f3431b31 HEAD` and `git diff d342add2 HEAD` over the DP10 file list are empty. Suites
  `watch mod_ack mod_channel mod_digest hook bridge cooperative attention agy delivery wake_on golden_surfaces::`
  plus the whole `hook_entrypoint` binary pass (combined 74, hook_entrypoint 27, integration 42, service 3).
- `scripts/agy-*` offline Python suites under python3.9 and python3.11: fake-gemini 88, gate 128, relay 92,
  sandbox 36, all OK on both.
- Full suite once, `HT_LEAK_RUN_ID=<uuid> nice cargo nextest run --locked --all-targets --all-features`: 4883 tests,
  282 s wall clock (under the 5 minute budget; `package manifest::failpoints_are_compiled_out_of_release_builds`
  alone takes 262 s). 4881 passed and 2 failed: `loop_inventory` (fixed inline, finding 6) and the `ht-veq` flake.
  `scripts/check-no-leaked-processes --run-id` found nothing.

## R13 interim numbers

`scripts/outside-adapter-report --base $(git merge-base main HEAD) --head HEAD`, run at `7348564c` (the sweep's
tests commit, before this record). Base `f3431b31` is the pre-absorption main, so the outside bucket includes main
v0.6.0's own changes absorbed at `d342add2`; the final figure is ht-poh.6.9's.

| Bucket | Files | Added | Removed |
|---|---|---|---|
| adapter | 80 | 9697 | 0 |
| outside | 488 | 39500 | 3645 |

| Category | Files | Added | Removed |
|---|---|---|---|
| trait-extension | 2 | 589 | 12 |
| reworked | 78 | 5152 | 3088 |
| callers | 20 | 1864 | 0 |
| test-infra | 74 | 11452 | 0 |
| tests | 263 | 13504 | 272 |
| docs | 36 | 4684 | 35 |
| other | 15 | 2255 | 238 |

Neutral check: 85 allowlisted (to-migrate 0, frozen-wire 49, history 2, registration 34). Allowlist delta against
base: +26 frozen-wire, +1 history, +24 registration. The report lists 70 commits without an `Outside-adapter`
trailer (the run's own design and bookkeeping commits and the trailers `git` cannot parse, finding 5).
