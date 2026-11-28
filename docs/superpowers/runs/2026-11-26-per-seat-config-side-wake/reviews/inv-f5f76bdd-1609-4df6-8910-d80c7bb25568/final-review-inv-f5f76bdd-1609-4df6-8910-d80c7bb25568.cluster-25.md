# Final review, cluster 25 (tests-cli), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout observed at HEAD c3e9ab0be with a clean status before and after the review. No worktree was created and nothing was built. I read the 18 slices (000128..000145) and the full head text of `tests/cli/config_seat.rs`. To trace the baseline rows, I also read `src/cli/config_seat.rs`, the `src/cli/commands.rs` and `src/cli/mod.rs` diffs (`caller_need`/`config_seat_need`/`derive_caller`/`default_seat`), `src/cli/nudge_threshold.rs`, the `src/cli/doctor.rs` diff and the design's Requirement baseline, Section 1, Error handling and Testing sections.

## Scope of this cluster

- `tests/cli/config_seat.rs` (new, 854 lines): a daemon-double `LocalClient` and a fixture that checks in through the real `run_selected` and then drives `config_seat::run` through `run_selected` (selected caller) and the unselected entry. It covers `list` text and JSON, `get` (own seat, `--seat` any seat, subagent read, JSON, seatless refusal), agent `set`/`unset` (tier, claim, no seat, changed/unchanged, negative value), M12 byte-equality with the `nudge-threshold` alias, subagent write refusal with nothing sent, human `set`/`unset` with `--seat` (human tier, no claim, agent-tier key on another seat), the human default seat (through a `#[cfg(test)]` seam), seatless human refusal, the capability gate on every verb but `list`, and daemon-refusal passthrough.
- `tests/cli/commands.rs`: parse-level tier and selector tests (`config_seat_verbs_parse_with_their_tier`, `an_agent_config_write_refuses_seat_and_human_tier_keys_before_anything_is_sent`, `human_tier_needs_human_as_argv_one`), plus the `ConfigSeat` slot added to the D4 slot mapping and the clap seat-argument walk.
- `tests/cli/doctor_json.rs`: one test of `programmatic_wake_seats_report` (sorted seat list from real `seat_config` rows, `off` and NULL rows excluded) and `programmatic_wake_text` (obsolete node line and seat line, empty on a default node).
- The other 15 files only add `auxiliary_session: false` to `CallerClaim`, `LifecycleEvent` or `EventMetadata` literals. That is mechanical and consistent with the new field. No assertion changed.

## Baseline trace (CLI rows)

- B3 (CLI reads/writes): production entry `parse_argv` → `Top::Config` → `CliAction::ConfigSeat` → `derive_caller` (`config_seat_need`) → `run_selected` or the unselected branch in `run_parsed_in_pane` → `config_seat::run`. Both branches are exercised by the fixture (`fixture.run` = `run_selected`, `run_unselected` = `config_seat::run(None)`, which is what the unselected production branch calls). Delivered.
- B4 (list with descriptions, values, defaults): `run_list` is reached before caller derivation in `run_parsed_in_pane` and needs no daemon. The text and JSON tests assert all five fields per key, the byte bounds and the trailer. Delivered.
- B5 (prose instruction works): the list trailer names both spellings, the agent-path refusal names the exact `ht human config seat set|unset KEY [VALUE]` command (89 bytes, asserted), and `--human` output mode does not make a human caller. The CLI part is delivered. Skill/doc discoverability belongs to another cluster.
- B6 (tiered writes, all readable): the agent-tier `programmatic_wake` write is refused at parse with nothing sent, the agent `--seat` write is refused (77 bytes), any caller and subagents may read any seat, and the human tier needs `human` as argv[1]. Delivered at the CLI layer. Daemon-side enforcement belongs to the service/store clusters.
- B7 (nudge threshold via non-human path): the agent `config seat set nudge_threshold N` sends the agent tier with a claim and no seat. Output and M12 are byte-identical to the alias (the alias `render` format matches `render_write`). Delivered.
- B1 (node setting replaced; doctor line): the doctor rendering helpers are tested here. The settings.json → `programmatic_wake_node_setting` producer in `report_captured` is not exercised by any test (see the untested-scope list). The seat list is exercised end to end by `tests/integration/seat_config_sweep.rs` (another cluster).
- B8, B9 and B10 (queue routing, side composer, write-target decisions): outside this cluster apart from the "own seat; human may name any" decision (B10), which the agent/human seat tests above cover. The rest is for the seam reviewer and the queue/host clusters.

## Observations on test quality

- The fixture runs a real check-in through `run_selected` before each case, so the agent-path claim comes from the production `wake_on::current_claim`, not a hand-built claim. This is good fidelity for a unit layer.
- The "nothing sent" assertions use `client.calls` unfiltered (including `SeatInspect`) for the subagent and capability-gate cases. That is strictly stronger than the `sent()` filter.
- `human_tier_needs_human_as_argv_one` only asserts that the refusal detail `contains("human")` for the flag-before-`human` forms. Any error mentioning "human" (for example an unknown-subcommand error) would pass. The existing actor_route refusal text would make an exact assertion easy.
- Retained evidence (`evidence/ht-z41e.3/lib-tests.log`) shows the 15 `config_seat` tests and `config_seat_verbs_parse_with_their_tier` passing at the Task 4 SHA. It does not list the two later commands.rs tests or the doctor test, and no head-SHA run of this cluster is retained. The full-suite measurement is the caller's root-acceptance row, so I did not rerun it (no duplicate qualification).

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The person's `ht human config seat set|unset` without `--seat` from inside a pane is tested only through the `default_seat_for_test` seam (tests/cli/config_seat.rs:709). The production chain `config_seat_need` → `CallerNeed::SeatDefault { required: false }` → `located_pane_seat` → `default_seat` → unselected `config_seat::run` has no end-to-end test (ledger Task 4 `seat-only-test`).
- The doctor's obsolete-node-setting line is tested only at `programmatic_wake_text` with a hand-built JSON value. The producer in `report_captured` (`daemon::settings::load(...).programmatic_wake.is_some()` → `programmatic_wake_node_setting: "obsolete"`) and the line's placement in `render_text_in`/`render_debug_text_in` run with no settings.json fixture.
- `--json` output for `config seat set|unset` (the `SeatConfigWritten` encoding) is not asserted; only `get --json` and `list --json` are.
- No retained head-SHA run of this cluster's tests: the retained log predates `an_agent_config_write_refuses_seat_and_human_tier_keys_before_anything_is_sent`, `human_tier_needs_human_as_argv_one` and `doctor_reports_seats_with_programmatic_wake_and_the_obsolete_node_setting`. The caller's full-suite row covers them.

## Deferred OK
- `human_tier_needs_human_as_argv_one` asserts `error.detail.contains("human")` for the flag-before-`human` forms. Asserting the actor_route "require immediate human namespace" text would keep it from passing on an unrelated error.
- `list_json_is_an_array_of_the_five_fields` hard-codes `rows.len() == 2`. Adding a registry key requires editing this test (intentional tripwire, but it duplicates the REGISTRY-length check the text test already makes).
