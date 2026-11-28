# Final review: cluster 13 (src-cli), epic ht-z41e

## Scope and method

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168, both checked with `git rev-parse --verify`. The review source was bound with `dispatch-text.mjs bind-review-source`, exit 0. The caller checkout was at HEAD c3e9ab0be with a clean status at review time. The code was read through `git show <head>:<path>`. No worktree was created, and no build or test was run, so test claims rest on reading the test sources at head.

Cluster files: src/cli/commands.rs, src/cli/config_seat.rs (new), src/cli/doctor.rs, src/cli/hook.rs, src/cli/mod.rs, src/cli/nudge_threshold.rs, src/cli/peer_launch.rs, src/cli/site_capability.rs (slices 000071-000078). Spec: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md (Requirement baseline B1-B10, scope change S1).

## Requirement baseline trace (CLI side)

- B3 (CLI that reads/writes): `ht config seat get|set|unset` and `ht human config seat set|unset` parse in `parse_cli_in_registry`. They route through `caller_need` -> `config_seat_need`: agent writes go through `CallerNeed::Selection` to `run_selected` and then `config_seat::run` with a claim. Reads and person writes without `--seat` go through `SeatDefault{required:false}`, where `default_seat` fills the slot, and then the unselected branch in `run_caller_scoped_deferred`. `--seat` forms need no caller. The CLI sends `GetSeatConfig`/`SetSeatConfig`/`UnsetSeatConfig` after the `seat.config_v1` gate. Delivered on the CLI side; the daemon's decisions belong to other clusters.
- B4 (list with descriptions): `config seat list` returns early in `run_parsed_in_pane`, so it needs no daemon or caller. It renders `<key> <values> default=<d> tier=<t> — <description>` from the compiled `REGISTRY`, plus a trailer line naming both write spellings. `--json` gives `{key, values, default, tier, description}`. Delivered.
- B5 (prose instruction works): an agent told to set `programmatic_wake` runs `ht config seat set programmatic_wake on`. Parse refuses it with the exact `ht human config seat set programmatic_wake on` (89 bytes). That command defaults to the pane's own seat, and the person approves it through the existing `Bash(<spelling> human *)` ask rule (src/harness/claude/permission_rules.rs). `nudge_threshold` is set directly. The skill and docs/agent-usage.md name the verbs. Delivered.
- B6/B7 (tiered writes, all keys readable, nudge threshold writable by an agent): agent writes are refused at parse time for `--seat` (CONFIG_SEAT_FLAG_REFUSAL, 77 bytes) and for a human-tier key. Subagent writes are refused in `run` (role != TopLevel) before anything is sent. Reads are open to any caller, subagents included. `nudge_threshold` is agent tier. The `human` tier is decided only by an immediate argv[1] `human` (`split_actor_os_argv`); `--human` and flags before `human` do not grant it, and `tests/cli/commands.rs::human_tier_needs_human_as_argv_one` covers that. Delivered.
- B1 (node setting replaced, doctor guidance): doctor reports `programmatic_wake_node_setting: obsolete` when settings.json sets the key, with one text line pointing at `ht human config seat set --seat <seat> programmatic_wake on`. It also prints `programmatic wake: on for <seats>` from a read-only store read. An older schema or a missing store reports nothing. The CLI side is delivered; the removal of the runtime flag is in other clusters.
- B10 ("own seat; human may name any"): the agent path sends `seat: None` plus a claim; the human path sends an explicit seat (from `--seat` via the Global-tier selector `SeatSlot::ConfigSeat`, or the located caller seat) and no claim. Delivered.
- B8/B9 (side routing, native defer): not in this cluster beyond carrying the auxiliary evidence. `drop_unadvertised_auxiliary` strips `claim.auxiliary_session` for a daemon without `seat.auxiliary_session_v1` and sends no extra round trip when the bit is off. `exact_check_in_replay` zeroes the echoed bit before comparing against `bridge::caller_claim`, which builds with false. The hook's synthetic events (`pending_event`, `watch_continuity`) and the cooperative check-ins send `false`. The Codex adapter, store and selection path are for the seam reviewer.

## Code notes

- The agent `set` of a human-tier key is refused twice: at parse time, and again by the daemon in store/seat_config.rs. The CLI refusal is a hint only, which matches A2 (the daemon decides).
- `refuse_agent_config_write` interpolates the typed VALUE unquoted into the suggested `ht human ...` command. It is harmless for the registered human key (`on|off`); a malformed value is refused later anyway.
- `LIST_TRAILER` (104 bytes) adds one model-visible line to `list` that the spec's list format does not name, and its size is not stated in the source comment (token rule f). The line is useful and small.
- The `pending_event` retry replay sends `auxiliary_session: false`. The fresh event that follows in the same hook run carries the real evidence and the bit is sticky, so the replay costs at most nothing.
- Retired-seat refusal, remote-seat refusal and operator provenance are daemon-side (store/seat_config.rs); the CLI passes refusals through unchanged (`a_daemon_refusal_passes_through_unchanged`).

## Findings

- [nit] src/cli/commands.rs:3018 the agent-path refusal pastes VALUE unquoted into the suggested `ht human config seat set KEY VALUE`; a value with spaces or shell metacharacters would produce a malformed suggestion (only reachable with a bad value for a human-tier key, which the daemon refuses anyway).
- [nit] src/cli/config_seat.rs:44 `LIST_TRAILER` is a 104-byte model-visible line beyond the spec's per-key list format, and its size is not recorded in the source comment the way the other new strings' sizes are.
- [minor] src/cli/doctor.rs:1170 the `report_captured` wiring (settings.json `programmatic_wake` -> `programmatic_wake_node_setting`, and the store read -> `programmatic_wake_seats`) has no test that drives a node through doctor; tests/cli/doctor_json.rs tests only the renderer and the store helper.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- doctor `report_captured` wiring of the obsolete node `programmatic_wake` key and the per-seat on-list is not exercised end to end (only `programmatic_wake_text` and `programmatic_wake_seats_report` are unit-tested in tests/cli/doctor_json.rs)
- `ht human config seat set` defaulting to the located pane seat is covered only through the `default_seat_for_test` seam, with no end-to-end pane test (already recorded as a Task 4 minor)
- no cargo build or test was run for this review; the coverage judgement rests on reading tests/cli/config_seat.rs and tests/cli/commands.rs at head

## Deferred OK
- unquoted VALUE in the agent-path human-tier refusal suggestion (src/cli/commands.rs:3018); cosmetic for the registered `on|off` key
- `LIST_TRAILER` size not stated in its source comment (src/cli/config_seat.rs:44)
- `pending_event` retry replay drops auxiliary evidence (src/cli/hook.rs:1719); the fresh event in the same hook run re-supplies it, and the bit is sticky
