# Final review: cluster 2 (docs-root), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Inputs: slices/000006.diff (docs/agent-usage.md, +4 lines, new "Per-seat config" subsection) and slices/000008.diff (docs/install.md, "Opt in to programmatic wake" rewritten). Spec: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md at head, including its Requirement baseline (B1-B10, S1).

## Claims checked against the head source

- `config seat list` needs no daemon and prints key, values, default, tier, description: `src/cli/config_seat.rs` `run_list`/`render_list` read the compiled `REGISTRY` and return before any client call. Matches.
- `config seat get [KEY] [--seat S]` reads any local seat, `(default)` marks an unset key, and a seatless caller must pass `--seat`: `render_entry` and `GET_NEEDS_SEAT`. Matches.
- `config seat set|unset` writes an agent-tier key on the caller's own seat as its top-level binding: the agent branch requires a selection with `Role::TopLevel` (`SUBAGENT_REFUSAL` otherwise) and sends a claim, not a seat. Matches.
- `--seat` exists only on `get` and the human verbs: `src/cli/commands.rs` refuses `--seat` on an agent set/unset ("--seat is for ht human config seat set|unset; an agent sets only its own seat"). Matches.
- Human tier is asked by the harness permission policy: Claude ask rules cover `<spelling> human *` (test rows in `src/harness/claude/permission_rules.rs` include `human config seat set --seat S programmatic_wake on` -> ask, `config seat set nudge_threshold 3` -> allow); Codex has `prompt` rules for `human` (`src/harness/codex/permission_rules.rs`). install.md "Agent permissions" already documents this. Matches B6.
- Per-seat enablement with no daemon restart: `src/app.rs` drops `.with_programmatic_wake(config.programmatic_wake())`; `queue_wake_selection` (`src/store/wake_intents/selection.rs`) reads `seat_config::programmatic_wake_on` inside the selection transaction on each attempt and fails off on a read error. Matches B1 and the install.md text.
- `ht doctor` lists seats with it on and flags the obsolete node key: `src/cli/doctor.rs` `programmatic_wake_seats_report` / `programmatic_wake_text` emit `programmatic wake: on for <seats>` and the `settings.json: programmatic_wake is obsolete ...` line in both text and debug renderings; `src/daemon/settings.rs` still parses the key (`Option<bool>`). Matches.
- `ht` spelling in install.md is valid: the installer links `~/.local/bin/ht` (install.md line 23-25).

## Stale text left beside the rewritten paragraph

The paragraph directly after the rewritten one in docs/install.md (head lines 573-576) still ends: "The release driver also selects that proof when the selected install has opted in." At head that is false. `scripts/release-qualify` line 111 says "Queue proof is selected only by the explicit flag; no operator setting is read", its `--queue-proof` help says "never read from operator settings", and AGENTS.md line 147 states release-qualify "selects queue proof only with `--queue-proof`". The spec (Section 1, "Removing the node setting") removes the node-settings probe and requires "the release docs follow". There is also no per-install opt-in left to select on, since the flag is now per seat. A release agent who reads install.md would expect queue proof to be selected automatically and could run the release without `--queue-proof`, so the release would never qualify queue wake.

## Minor wording

- agent-usage.md says "A daemon that predates `seat.config_v1` refuses with one line". The refusal actually comes from the CLI's capability gate (`SEAT_CONFIG_REFUSAL`, "herdr-threads: daemon predates seat config; restart it"), so the old daemon never receives the command. The effect described is right; only the wording is off.
- agent-usage.md says `nudge_threshold` is "the same value `nudge-threshold` sets". That is true of the stored value, but the spec gives the two paths different validation: `config seat set nudge_threshold -5` is refused (strict -1..9), while `nudge-threshold -5` clamps to -1. A reader could expect identical input handling.

## Requirement baseline in this cluster

- B3/B4/B6/B7: the agent-usage subsection documents list, get, the agent-tier set/unset and the human-tier write path. Each one traces to a production CLI entry point at head (above).
- B1: install.md documents per-seat enablement and the obsolete node key, and the production path reads the seat key per attempt. The one stale sentence (above) is a documentation defect, not undelivered behaviour.
- B5 ("telling an agent in prose to set this setting works") depends on the skill text and the `list` trailer. Those live in the integrations-skill and src-cli clusters, so tracing them is the seam reviewer's job. This cluster's docs give the exact spellings an agent needs.
- B8/B9 (side-chat routing, and native wake deferring on a side chat) are documented in integrations/codex/README.md, which is another cluster. This cluster does not need to restate them.

## Verdict
not ready (docs/install.md still says release qualification selects queue proof from the install's opt-in; release-qualify now needs an explicit --queue-proof)

## Must fix before landing
- docs/install.md head line 575-576: remove or replace "The release driver also selects that proof when the selected install has opted in." with the head behaviour, as in AGENTS.md: `scripts/release-qualify` selects queue proof only with an explicit `--queue-proof` (Linux/macOS), and under it the driver sets `programmatic_wake on` per owned Codex seat in its owned instance. The sentence contradicts scripts/release-qualify line 111 and the spec's "release docs follow" requirement, and it can lead a release run to skip queue proof.

## Untested scope
- No check ties the install.md "Opt in to programmatic wake" text to scripts/release-qualify's selection rule. The stale sentence above got through for that reason. A docs/behaviour consistency check is not required by the spec; this is noted, not demanded.
- The doctor lines that install.md promises (`programmatic wake: on for ...` and the obsolete-setting line) were checked by reading the source only. I ran no doctor rendering against a store with a seat set to on (read-only review).

## Deferred OK
- docs/agent-usage.md: change "A daemon that predates `seat.config_v1` refuses with one line" to say the CLI refuses before sending (wording only; the behaviour described is correct).
- docs/agent-usage.md: note that `config seat set nudge_threshold` applies the strict -1..9 range while the `nudge-threshold` alias clamps negatives to -1, so the two paths do not handle input identically.
