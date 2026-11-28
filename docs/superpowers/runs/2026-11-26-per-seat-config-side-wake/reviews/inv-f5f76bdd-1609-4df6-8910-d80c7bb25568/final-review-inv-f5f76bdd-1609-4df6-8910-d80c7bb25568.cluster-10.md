# Final review: cluster 10 (root-files), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). The caller checkout was at HEAD c3e9ab0be with a clean `git status --short` when the review ran. Inputs: slices 000001-000005 (AGENTS.md, CHANGELOG.md, README.md, RELEASE.md, TRUST-POLICY.md), digest.md, the root spec `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` at head (it has a `## Requirement baseline`: B1-B10, S1).

## Scope and method

This cluster holds documentation only. I checked each claim the five files make against the head source it names (read with `git show <head>:<path>`). I did not build or run anything. For the baseline rows I checked that these documents describe each row accurately and completely, and that the code they cite exists at head. Tracing each behaviour end to end from the CLI or daemon entry point belongs to the code clusters and the seam reviewer.

## Claims checked against head source

- AGENTS.md seat-config paragraph, byte sizes. `SEAT_CONFIG_REFUSAL` is 54 bytes, `GET_NEEDS_SEAT` 62, `HUMAN_NEEDS_SEAT` 64, `SUBAGENT_REFUSAL` 63 and `CONFIG_SEAT_FLAG_REFUSAL` 77. `LIST_TRAILER` is 103. The `programmatic_wake` list line is 155 bytes and the `nudge_threshold` line 116. `human_tier_refusal("programmatic_wake", "set programmatic_wake on")` is 89. All of these match the stated sizes.
- The output formats in AGENTS.md (`<key>: <value>[ (default)]` and `<key>: <value> (changed|unchanged)`) match `render_entry` and `render_write` in `src/cli/config_seat.rs`. That `list` is static (no daemon) matches `run_list`.
- AGENTS.md /side routing paragraph. `auxiliary_session()` in `src/harness/codex.rs` checks SessionStart, UserPromptSubmit, PreToolUse and Stop, top-level only. `wake_process_root` names a root only if exactly one binding is neither concurrent nor auxiliary and every other binding is auxiliary. `CENSUS_LIMIT = 64` is in `src/store/wake_intents/selection.rs`, and a lone concurrent or auxiliary binding is excluded there. TRUST-POLICY and the design amendment say the same things.
- TRUST-POLICY ask-rule table:
  - Claude: `render()` asks on `<spelling> human` for both spellings. Its test pins `human config seat set|unset` as ask and `config seat set|get` as allow.
  - Codex: the new `permission_codex_prompts_on_human_config_seat_for_both_spellings` test covers both spellings.
  - agy: `ALLOW_RULE = "command(herdr-threads)"` with `no_permission_rules`.
  - opencode: `ALLOW_KEYS = ["herdr-threads *", "ht *"]`.
  - Pi: the module doc says settings and permission configuration are untouched.
  - Hermes: setup has no permission configuration.
- TRUST-POLICY authority rows:
  - `GetSeatConfig { seat, key? }` carries no claim, as the row says.
  - The agent-tier write refuses subagents, a `--seat` naming another seat, human-tier keys, remote seats (`require_local_seat_setting`, checked before the transaction) and read-only bindings. The read-only check exists on both backings (`is_readonly`, and `write_threshold`'s `refuse_readonly`).
  - A finally retired seat gets `seat retired: <seat>` (`src/store/seat_config.rs`), while a reversibly retired seat is still accepted.
- CHANGELOG and AGENTS: doctor's obsolete-setting line and its `programmatic wake: on for <seat>, ...` line exist (`programmatic_wake_text` in `src/cli/doctor.rs`). A changed table-backed value calls `wake::note_new_attention` in the same transaction.
- RELEASE.md and AGENTS: `scripts/release-qualify` selects queue proof only with `--queue-proof` ("no operator setting is read") and rejects the flag off Linux and macOS. In `scripts/wake-e2e.py`, `Recipient.enable_queue_wake` runs `ht human config seat set --seat <seat> programmatic_wake on` for Codex recipients after the standing turn and before the queue bootstrap and the first send.
- Stale wording: none of the five files still presents the node `programmatic_wake` key as active, or says queue selection requires exactly one connected top-level binding.

## Requirement baseline as documented

- B1 (seat level replaces node level): documented consistently in AGENTS, CHANGELOG, RELEASE and TRUST-POLICY ("Bounded native-only queue degradation"). The daemon read path belongs to the code clusters.
- B2 with S1 (closed typed registry): TRUST-POLICY "Seat config" describes the closed registry with no synonyms, which matches `src/protocol/seat_config.rs` `REGISTRY`.
- B3 and B4 (read/write CLI, list with descriptions): README table row, AGENTS paragraph and `render_list`. Each list line carries values, default, tier and description.
- B5 (prose instruction works): the skill line (outside this cluster) and the list trailer name both spellings. AGENTS records this.
- B6 and B7 (tiers; nudge threshold writable without the human path): the TRUST-POLICY authority rows and A1 invocation spelling. The Claude and Codex permission tests show that agent `config seat set` is allowed and `human config seat` asks.
- B8 (/side routing spikes): TRUST-POLICY "Queue wake target with `/side` sessions present" cites the spike doc's S1-S4 and records the accepted limits.
- B9 (no native typing into a side chat): the AGENTS paragraph on `CODEX_PLACEHOLDERS` and the TRUST-POLICY busy-sibling paragraph.
- B10 (brainstorm decisions):
  - "Remove, no migration": CHANGELOG says users must set the key on each seat.
  - "Own seat; human may name any": the A5 exception row.
  - "Key over existing column": TRUST-POLICY column-backed key.
- No row is contradicted or left undocumented in these files. Whether each row is delivered end to end is for the code clusters and the seam reviewer.

## Findings

- [nit] AGENTS.md:147 and RELEASE.md:44 say the driver enables `programmatic_wake` on "each owned Codex recipient seat". `TmuxRecipient.start` (scripts/wake-e2e.py:3897) never calls `enable_queue_wake`, so a tmux-named Codex seat stays native under `--queue-proof`. Before this run it was queue-enabled node-wide. This is the existing deferred minor from Task 6. The docs state it more broadly than the code does.
- [nit] AGENTS.md:122 lists the seat-config fixed refusal lines with their byte sizes but leaves out the new operator refusal `seat retired: <seat>` (14 bytes plus the seat id). The design names it as the one new model-visible line. TRUST-POLICY "Seat config" also says only that final retirement deletes the rows, not that later operator writes are refused.
- [nit] CHANGELOG.md `## Unreleased` lists only the per-seat queue wake change (ht-z41e.4). It leaves out:
  - the new `config seat` / `human config seat` CLI, a new user-facing feature with a minor-version impact;
  - native wake deferring on a displayed `/side` composer;
  - root-session queue routing beside `/side` sessions;
  - node migrations 20-22, which RELEASE.md:7 says must be called out.

  The release's "Prepare vX.Y.Z" commit writes the changelog entry, so nothing is lost. But a partial Unreleased section may lead whoever cuts the release to under-version it.
- [nit] The TRUST-POLICY agent-tier row says the write is "decided exactly as `SetNudgeThreshold`" and also "never ... a read-only binding". The `nudge-threshold` / `wake-on` aliases keep accepting a read-only binding (`refuse_readonly` is false for them, src/store/seats/threshold.rs:120). So a read-only binding can still change `nudge_threshold` through the alias but not through `config seat set`. The table does not say this. The behaviour is deliberate in code but undocumented in the policy.

## Must fix before landing

- none

## Untested scope

- Claims in these docs about live behaviour (side-queue and side-native-defer outcomes, S2 busy-sibling drop, root re-registration receipt) were checked only against source and the spike doc; this documentation review ran no tests or live runs.

## Verdict

ready

## Must fix before landing

- none

## Untested scope

- Live-behaviour statements in AGENTS.md and TRUST-POLICY (side-queue Accepted on root, side-native-defer, S2 busy-sibling drop) were verified only against source and the spike doc, not by any run in this review.

## Deferred OK

- AGENTS.md:147 and RELEASE.md:44 say "each owned Codex recipient seat", but TmuxRecipient never enables programmatic_wake (tmux-named Codex stays native under --queue-proof); narrow the wording or enable it there.
- AGENTS.md:122 refusal-line inventory omits the new `seat retired: <seat>` operator refusal (14 bytes plus seat id), and TRUST-POLICY "Seat config" does not mention it.
- CHANGELOG.md Unreleased lists only ht-z41e.4; the config seat CLI, /side native defer, /side queue routing and node migrations 20-22 still need entries at release prep.
- TRUST-POLICY agent-tier row says "exactly as SetNudgeThreshold" plus a read-only refusal, while the nudge-threshold/wake-on aliases still accept a read-only binding; state the difference.
