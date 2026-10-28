# Harness-neutral CLI text

Task (user, via thread diV43X2hGqA): every CLI surface a model or operator reads is
harness-neutral; guidance is actionable by any harness (Claude Code, Codex, Hermes,
agy) or a plain shell. Patch release.

## Why the existing check missed it

`scripts/check-harness-neutral` strips comments before matching, so `///` doc comments
on clap-derived items, which clap renders as help, were never scanned. It matches only
harness *names*, so hook-event names (`SessionStart`) and internal design references
(`spec §1`, `C1`, `TRUST-POLICY A4`) in core string literals passed as well.

## Inventory

Source: every `--help` page (84 commands, rendered recursively from the all-features
binary), plus a scan of core string literals that read as prose (contain a space).

### Fixed: core-authored help

| # | Site | Text | Fix |
|---|------|------|-----|
| H1 | `src/cli/commands.rs:960` | `--agent-id`: "inserted by herdr-threads' Claude PreToolUse hook (spec §1) ... no behaviour change until session bindings land" (global: on all 83 subcommand pages) | `hide = true` (plumbing the harness integration inserts; agents never type it) and neutral text |
| H2 | `src/cli/commands.rs:1097` | `setup` about: "Claude: `$CLAUDE_CONFIG_DIR/settings.json` ... Codex: `$CODEX_HOME/hooks.json`" (two of four harnesses) | dropped; the per-harness scope list in the long help already comes from each adapter |
| H3 | `src/cli/commands.rs:1120` | `launch` about: "Start Codex or Claude" | "Start a registered agent harness" |
| H4 | `src/cli/commands.rs:1290` | `harness-version normalize`: examples `codex-cli 0.158.0`, `2.1.286 (Claude Code)` | neutral examples |
| H5 | `src/cli/commands.rs:1834`, `:1869` | `--harness-binary`: "default: the first `claude`/`codex` on PATH" | "default: the harness's executable on PATH" |
| H6 | `src/cli/help.rs:97`, `:102` | `launch` long help: "SessionStart hook" | "session-start hook" |

### Fixed: core-authored runtime output

| # | Site | Text | Fix |
|---|------|------|-----|
| O1 | `src/cli/launch.rs:1192` | launch report hint: "the agent's SessionStart hook shows pending ..." | "session-start hook" |
| O2 | `src/daemon/health.rs:466` | doctor/health line: "reattach when their session claims them (C1)" | design id dropped |
| O3 | `src/harness/launch.rs:742` | launch refusal: "(TRUST-POLICY A4)" | policy id dropped |
| O4 | `src/store/service_controls.rs:177` | error: "notification publication belongs to C3" | design id dropped |

### Kept (allowed by the task's rules)

- `[possible values: claude, codex, hermes, agy, ...]`, `--kind claude|codex|...`, the
  `HERDR_THREADS_<ID>_OPTS` list and the `human me init` agent-marker list: harness ids
  as data, generated from the registry.
- Per-harness blocks in `setup`/`unsetup`/`setup-status`/`launch`/`handoff` long help:
  adapter-owned fragments (`src/harness/<id>…`), each keyed by its harness.
- `setup --no-delivery-mod`, `--disable-prompt-suggestions`, `--keep-prompt-suggestions`:
  flags only `setup claude` honours; each help line is labelled `setup (claude)`.
- `contract-id --json` keys `{"claude": ID, "codex": ID}`: frozen v1 wire shape.
- `bootstrap plan V1/V2`: product format version names, not design references.
- `src/harness/launch.rs` V1 Codex bootstrap refusals: already allowlisted frozen-wire,
  printed only for a Codex launch.

## Enforcement

1. `scripts/check-harness-neutral` additionally flags, in core string literals that
   contain a space (prose, not data keys):
   - native hook-event and harness-marker names (`SessionStart`, `SessionEnd`,
     `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop`,
     `PreCompact`, `pre_llm_call`, `post_llm_call`, `PreInvocation`, `CLAUDECODE`);
   - design references: `§`, requirement ids `[ACDPR]N`, bead ids `ht-xxx`,
     `docs/superpowers`.
   Bare literals (`"SessionStart"`), identifiers and comments stay unflagged. Same
   allowlist file and categories; no new entries were needed.
2. A test (`every_help_page_is_harness_neutral`, `tests/cli/commands.rs`) renders long help for every command in `Cli::command()` and fails on a
   harness id or alias, a hook-event name or a design reference, after removing
   `[possible values: ...]`, registry-generated lists, and lines that occur verbatim in
   an adapter-owned source file. Remaining exceptions sit in a reasoned in-test table.
   This backstops doc comments, which the source scan cannot see.

## Acceptance criteria

- A1. No core-authored `--help` text names a harness, a hook event or a design
  reference; the help test passes and fails if `--agent-id`'s old text is restored.
- A2. O1–O4 strings are neutral; `check-harness-neutral` fails if any is restored.
- A3. `check-harness-neutral` self-tests (`scripts/tests/test_check_harness_neutral.py`)
  cover the new token classes and the prose/data distinction.
- A4. clippy (all features), fmt, check-harness-neutral, check-wire-compat,
  check-default-features green; golden fixtures updated where they capture changed text;
  one unfiltered full suite plus leak check.
