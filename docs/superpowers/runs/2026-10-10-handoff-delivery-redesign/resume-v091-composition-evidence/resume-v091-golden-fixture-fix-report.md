# f39 setup/help golden fixture diagnosis

Source baseline: `f39da231501cb686e361ff656000e777f984735e`, tree `aea4138134a36486aed22bc98aa31d6844e64ee6`. Read-only diagnosis; no Cargo, binary invocation, production edit, configuration edit, or git mutation performed. This report is the only output. No scratch patch: the parent should capture canonical actual snapshots using its controlled selective run, then compare against the independently derived delta below. Predicted complete outputs are not substituted for runtime evidence.

## Actual failure evidence and disposition

`/private/tmp/ht-v091-focused-f39d/execution.json` identifies the head/tree and selection; `/private/tmp/ht-v091-focused-f39d/run.log:524–527`, `564–567`, and `598–601` establish:

- `golden_surfaces::help_texts`: help/setup line 116 expects legacy `hook <harness>` plus appended `--event`; actual is `hook-event <harness> <EVENT>`.
- `golden_surfaces::setup_reports`: setup/claude.text line 4 expects the shared route plus an owner UUID comment; actual is the same shared route without the comment.
- `golden_surfaces::setup_status_reports`: setup-status/claude.absent.text line 49 first differs because actual adds `registration_conflict: none` before scope.

These are stale expectations against the selected stable-hook design. The first mismatch aborts each test, so this evidence does NOT establish that later Codex/all/Hermes snapshots match. Source inspection identifies the remaining expected deltas below. The feature itself already contained stale snapshots: immutable `0b04f91...:tests/fixtures/golden/setup/claude.text` has the owner comment and no commands map, while that same feature's Claude setup producer already emits `commands`. This is not evidence to revert stable commands to Main's older snapshot convention.

## Contract and producer grounding

The original root design, `docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md:46–52`, requires deterministic per-event commands, installation UUID retained only in the manifest, exact ownership/fingerprints, foreign identical registration refusal, explicit legacy migration, and no automatic native trust/permission writes. It explicitly preserves canonical route prefixes used by launch/mod and Hermes's fixed Python bridge. Current TRUST-POLICY remains normative; this fixture refresh adds no new provenance or delivery/identity meaning.

The actual generation chain is `SetupEnv::hook_argv` (`src/cli/setup.rs:436`) → `native_entries` (`src/harness/setup.rs:233`) → `installed_event_argv` (`src/cli/hook.rs:110`) → `native_event_command` (`src/harness/setup.rs:60`) → exact owned groups. Every argument is single-quoted by `shell_command`. The event command contains no `--event` tail and no owner comment.

`shared_command` (`src/harness/setup.rs:616`) deliberately derives a common canonical legacy-shaped route from stable per-event commands. Therefore top-level `command` and `hook_argv` are NOT to be mechanically changed into a single event command. Exact installed command reporting is the separate `commands` map (`commands_json`, line 340), emitted by Claude install (`src/harness/claude/setup.rs:177–194`), Codex install (`src/harness/codex/setup.rs:846–867`), and recorded status (`src/harness/setup.rs:3488`).

`settings_status` (`src/harness/setup.rs:3477–3509`) reports `registration_conflict` from the real settings bytes, exact recorded groups, and recognized competing registrations. Missing settings returns null, not false: file read failed, so no readable config was checked. A successful fresh install has readable settings and only exact owned groups, hence false. It emits no commands map when no manifest exists. `current_command_matches` remains true after a fresh stable install because the shared route matches expected canonical routing; it remains false while absent.

## Exact expected delta allowlist

Paths below are relative to `tests/fixtures/golden/`. Expected changed files total **15**:

| Files | Expected change |
| --- | --- |
| `help/setup.txt` | Replace the two legacy grammar lines and expand the ownership paragraph exactly from `src/cli/help.rs:30–41` (verbatim block below). |
| `setup/claude.text`, `setup/claude.json` | Remove only the trailing owner comment from `command`; add four exact entries in `commands`. |
| `setup/codex.text`, `setup/codex.json` | Remove only the trailing owner comment from `command`; add six exact entries in `commands`, including Main's UserPromptSubmit. |
| `setup/all.json` | Apply those same changes inside the Claude and Codex `harnesses[].report` objects only. |
| `setup-status/claude.text`, `setup-status/claude.json` | Same command/map delta as installed Claude; add `registration_conflict: false`. |
| `setup-status/codex.text`, `setup-status/codex.json` | Same command/map delta as installed Codex; add `registration_conflict: false`. |
| `setup-status/all.json` | Same installed deltas inside the Claude and Codex nested reports only. |
| `setup-status/claude.absent.text`, `setup-status/claude.absent.json`, `setup-status/codex.absent.text`, `setup-status/codex.absent.json` | Add `registration_conflict` only: text `none`, JSON `null`. Do not invent a command/map/manifest ownership for an absent installation. |

The shared normalized `command` is exactly:

```text
'<bin>' '--state-dir' '<root>/state' '--host-endpoint' '<root>/host.sock' 'hook' 'claude'
'<bin>' '--state-dir' '<root>/state' '--host-endpoint' '<root>/host.sock' 'hook' 'codex'
```

Each exact `commands[EVENT]` value is:

```text
'<bin>' '--state-dir' '<root>/state' '--host-endpoint' '<root>/host.sock' 'hook-event' 'HARNESS' 'EVENT'
```

Substitute HARNESS with `claude` or `codex` and EVENT with the exact case-sensitive declaration key. Claude declaration/install order is `SessionStart`, `PreToolUse`, `SubagentStart`, `SubagentStop` (`src/harness/claude.rs:793`). Codex declaration/install order is `SessionStart`, `SubagentStart`, `PreToolUse`, `UserPromptSubmit`, `SubagentStop`, `SessionEnd` (`src/harness/codex.rs:270`). The existing `events` arrays retain that order. JSON object keys and text `commands.EVENT` rows are lexicographic through serde_json's map and `render_text` (`src/cli/setup.rs:1130`): Claude `PreToolUse`, `SessionStart`, `SubagentStart`, `SubagentStop`; Codex `PreToolUse`, `SessionEnd`, `SessionStart`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit`. Do not reorder the events array to match the map.

The exact new help block is:

```text
Ambiguity is refused. The installed hook command is
`<this executable> --state-dir <state> --host-endpoint <socket> hook-event <harness> <EVENT>`
for each native event (legacy hook commands keep working;
`doctor` suggests re-running setup); in any session that is not a pane of that Herdr
instance it prints nothing, runs no version probe and starts no daemon, and only sends a
short best-effort evidence note to a daemon already running (gate files under
<state>/harness/evidence).
Each harness's ownership manifest lives in <state>/setup/. Native commands use the stable
hook-event HARNESS EVENT entrypoint with canonical routing. Installation UUIDs remain in the
manifest, outside shell commands. Equivalent setup retains exact owned groups in place; unrecorded
identical stable groups are foreign and cause a safe refusal. Setup explicitly migrates recorded
legacy hooks; status only inspects them. Changed native commands may require review again.
```

The subsequent old-owner-marker adoption paragraph remains source-produced and unchanged; it refers to the existing marked legacy adoption path, not permission to adopt unrecorded stable commands. Do not generalize or rewrite it during fixture repair.

## Required unchanged truth and refresh procedure

- Preserve `hook_argv` as the existing shared route vector. Preserve explicit canonical state/host selectors and executable spelling. Native per-event commands must contain both selectors; no random installation data or native-trust hashes are appended.
- Preserve all four Claude and all six Codex events, including Codex UserPromptSubmit in `events`, installed config layer event lists, and trust keys. Main's sixth group is retained with the stable generated grammar.
- Preserve `adopted: null/none`, installed/action/created fields, no stale installation, `current_command_matches: true` after install and false when absent, Codex `duplicate_hook: false`, and the installed Codex layer classification `identical`. `observe_codex_layer` recognizes actual stable commands (`src/harness/codex/setup.rs:156–205`), so changing it to foreign/different to match a broken inspection would mask a regression.
- Preserve trust `review_required`, six `trust_recorded: false` keys at the existing `:0:0` positions after Codex install, and unknown trust when absent. `codex_trust_json` checks exact recorded groups (`src/harness/codex/setup.rs:489–557`); do not mark installed hooks trusted or verified. Preserve no permissions granted, no sandbox allowance, unknown observation/execution, and unchanged foreground settings/manual advice.
- Preserve all Hermes outputs and the all-harness text summaries (`setup/all.text`, `setup-status/all.text`). They do not print the changed command maps/conflict fields; Hermes retains its Python bridge. Preserve unrelated AGY/synthetic harness results in aggregate JSON.
- Preserve all `.status` snapshots: normal help/setup/individual-status cases remain `exit: 0` with empty stderr. **The aggregate setup-status case intentionally remains exit 2**, with the existing synthetic_fourth explicit fixture-root-unavailable stderr. Do not blanket-normalize status-all to success. `record` snapshots status/stderr separately (`tests/golden_surfaces.rs:248`); they must still be checked.
- Parent may execute its one controlled selective `HT_BLESS` run for exactly `golden_surfaces::{help_texts,setup_reports,setup_status_reports}` using existing isolated fixture/environment discipline and retained logs. No broad blessing. The test functions cover multiple files and some only `record` outputs rather than asserting success; a successful blessing by itself does not prove expected command success.
- Review the generated diff against the 15-file/field allowlist and unchanged truths above; retain the canonical generated outputs as evidence. `help_texts` also visits launch/handoff/me-init; this diagnosis does not authorize unexplained changes there. Any additional diff needs its actual cause accounted for before import. Hook stdout and its helper are owned separately; do not edit `tests/golden_surfaces.rs` or hook snapshots under this report.
- Then run the same three tests normally without `HT_BLESS` as part of the parent's combined verification. No PASS/RED/GREEN claim is made here. The only actual failures read were the captured f39 ones; this source-derived plan explains what the next controlled run should produce.

Minor docs/agentusage line159 and Main CHANGELOG generic-accept phrasing are left on the existing light-review punchlist; no edits or broadened review loop.

## Captured-diff review addendum (controlled 8d37 generation)

**Disposition: approve the concrete 16 setup/help fixture changes for import.** All 15 originally planned paths match the source-derived delta exactly. The one additional path, `tests/fixtures/golden/help/handoff.txt`, is a valid stale-golden correction grounded below. No unsafe or unexplained field changes were found in this scope. This is a fixture-diff approval, not a runtime regression GREEN claim. Ten separately changed `hook/*.stdout.status` paths remain under the hook helper owner's review and are not approved or assessed here.

Captured evidence: `/private/tmp/ht-v091-golden-capture-8d37/execution.json` records head `8d37746d98e6bd766761e5cff82018ebf579643b`, tree `147212b87330382ac3f2541457a0c5941dfffe53`, run UUID `573e1e9a-b5aa-4dd4-a03b-1aedc8a9516b`, four exact fixture-generation cases exiting 0, and scoped leak check exit 0. The evidence explicitly says `controlled actual fixture generation, not regression GREEN`. I read this evidence and the actual working-tree diff; I ran no product/test binaries. The parent still owns normal unblessed combined verification.

### The 15 planned paths

Read-only `git diff` inspection of every changed text fixture matches the expected shared-command owner-comment removal, precise four/six event maps, and absent-null/installed-false conflict fields. A recursive JSON old-versus-new comparison across all eight changed JSON fixtures (including both nested aggregate reports) found **only**:

- `command` losing its final ` # herdr-threads-owner:<uuid>` suffix;
- `commands` added with the exact declared stable per-event shell bytes;
- `registration_conflict` added as null for absent status and false for installed status.

No other JSON leaves changed: all trust, permissions, event ordering, foreground, installation/adoption, layer classification, synthetic harness, and route fields are retained. Claude maps have four keys; Codex maps have six, including UserPromptSubmit and SessionEnd. The shared prefix remains `hook HARNESS`; exact event map entries are `hook-event HARNESS EVENT`. The setup help delta is byte-for-byte the planned source block.

Read-only byte comparisons against HEAD additionally confirm every setup/setup-status `.status` snapshot is unchanged, including both `setup-status/all.{text,json}.status` files with exit 2 and the exact existing synthetic_fourth stderr. All Hermes fixtures, both aggregate text reports, `help/launch.txt`, and `help/me-init.txt` are unchanged. No status/foreign/trust refusal is being erased by these changes.

### Additional handoff help path: valid historical drift

The 55-line git-stat change (net +11 lines) is fully accounted for; it is not a new implementation or unrequested policy adjustment:

| Actual changed area | Independent grounding and meaning |
| --- | --- |
| Usage changes from required positional BODY to `<BODY|--file <FILE>|--stdin>`; individual BODY shown optional | `src/cli/commands.rs:1362` defines a required, mutually exclusive `instruction_source` group over body/file/stdin; BODY itself is Option<String>. Required choice remains enforced; this does not make instructions optional. |
| Adds `--deadline`, `--require-invite-ack`, `--invite-deadline` descriptions | Same file lines 1364–1372 declares the flags. Policy construction at 2733–2744 makes invite deadline imply timed acceptance, keeps Lazy default, and preserves independent message/invitation durations. Original root spec lines 38–40 requires these exact source/deadline choices. |
| Adds `--file` and `--stdin` descriptions | Same file lines 1410–1415 declares the public flags; line 2745 acquires the selected source via `input::read_body` before constructing the handoff action. |
| Existing options interleave in a new display order | Captured clap output reflects the added local fields alongside inherited globals and flattened selector. Names/descriptions of existing options remain unchanged. No option removal, permission bypass, or behavioral weakening appears in that reorder. |
| Epilogue replaces old 1..1024 positional-only body text with one source and 1..65536 UTF-8 bytes, frozen once | `src/cli/help.rs:141` is the exact text producer. `src/cli/input.rs:30–78` enforces exactly one bounded source and valid nonempty UTF-8; `src/cli/handoff.rs:55–63` separately validates handoff body against MAX_BODY_BYTES. Original root spec line 38 explicitly requires read-once/frozen exact 65,536-byte input. Topic/goal's separate 1,024-byte check remains separate. |
| New epilogue details addressed ACK, independent durations, Lazy invitations, no launch/registration settlement, and sender deadline warning | Exact producer `src/cli/help.rs:142`; original invitation/handoff contract at root spec lines 28–40 requires these semantics. It does not claim recipient acceptance, ACK, task adoption, or model consumption from launch. |
| Epilogue changes native-readiness statement to retain native approval/structural safety while hook readiness is diagnostic | Exact producer `src/cli/help.rs:153`; original root spec line 44 explicitly says missing/stale/wrong-profile setup readiness is advisory while selected-native approval/configuration refusal and structural safeguards remain authoritative. This does not add auto-trust or weaken ownership checks. |

Additional immutable evidence: I compared the complete current `HANDOFF_TEMPLATE` and `HandoffArgs` struct with `0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2`; both selected source blocks are byte-identical. That same frozen feature's `help/handoff.txt` still says `1..1024 UTF-8 bytes`. Thus the extra regenerated path corrects a stale expectation already present before Main composition. The first setup help mismatch had prevented the earlier unblessed `help_texts` run from ever reaching handoff. The controlled generation now reached its existing case; no new golden case or relaxed helper was needed for handoff.

### Concrete reviewed scope seal

Review HEAD: `8d37746d98e6bd766761e5cff82018ebf579643b`. SHA-256 of the exact read-only `git diff --` for `help/setup.txt`, `help/handoff.txt`, `setup/`, and `setup-status/`: `8965f63b9c33f3a0a6e0a777cdb9afa25d6a899c9e89699ae8b3109b4f70dcb1`. Scope: 16 files, 76 insertions, 37 deletions. No report approval extends to subsequent changes outside this sealed diff.
