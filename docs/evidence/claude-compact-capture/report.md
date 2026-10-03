# Claude Code 2.1.287 compact capture (SessionStart `source: compact` input and output application)

- Binary: `claude --version` = `2.1.287 (Claude Code)`, `~/.local/share/claude/versions/2.1.287` (the target of `~/.local/bin/claude`), SHA-256 `6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea`.
- Method (same as `claude-286-hook-capture`; `run.sh`, `hook.py` and `settings-capture.json` differ only in the binary path; `hook_out.py` and `settings-out.json` are new, see below): a scratch git project at `<scratch>/proj` with **project-local** hooks only, normal user auth (no `CLAUDE_CONFIG_DIR`), `--setting-sources project,local`, model `claude-haiku-4-5-20251001`, `--max-budget-usd 0.10` per run. `run.sh` launches each run from bash with every parent `CLAUDE*`/`HERDR*` variable removed through `env -u` and records what is left; `*.parent-leak.txt` was empty for all four runs (the recorded values are in `payloads/transcript-evidence.json`). No user-global settings, hooks, permissions or credentials were changed. At cleanup `proj/.claude/settings.local.json` was deleted. `extract.py` produced the redacted `payloads/` and `run*.json` from the raw hook logs and Claude's own transcript for the scratch project. The scratch dir was placed under the task worktree's `.tmp/` (so every file the run created is traceable to the task) instead of `/private/tmp/ht-claude287-compact-scratch`; it appears as `<scratch>` in every committed file.
- `hook_out.py` (SessionStart only; logs stdin and returns adapter-shaped output): `source: compact` returns `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"HT-COMPACT-MARKER-c287cp"}}`; every other source returns the same shape with `HT-SS-MARKER-c287cp`, so the two are distinguishable. The returned value per event is in `payloads/hook-out-returned.json`.
- Runs (all one session, `e3272da9-a6a2-4d99-bd8d-e552b74d87f0`, chained with `--continue`; print mode throughout, so no interactive pane was needed):
  - **run1** starts up and runs root Bash (`echo capture-ok`) with `hook.py`, which logs and prints nothing.
  - **run2** `--continue`; a general-purpose subagent runs Bash (`echo child-capture-ok`). Gives the resume and subagent payloads.
  - **run3** `--continue -p "/compact"` with `hook_out.py`. Print mode ran `/compact` (`num_turns` 0, empty `result`). The hook log has a `SessionStart` with `source: "compact"`, preceded by the `resume` that every `--continue` produces.
  - **run4** `--continue -p "List every string starting with HT- that appears in your context, verbatim."` Answer: `HT-SS-MARKER-c287cp` and `HT-COMPACT-MARKER-c287cp`.
- Cost: the `total_cost_usd` Claude printed is session-cumulative on `--continue` runs: run1 0.0243, run2 0.0656, run3 0.0761, run4 0.0928. The session total is **$0.0928**; summing the printed figures (an upper bound) gives $0.259. Both are under the $1.00 budget. No discarded attempts.

## Input shapes (payloads 01-04)

Each captured payload has **the same key paths and JSON value types as the 2.1.286 fixture of the same kind** (`shape-compare.py`, output in `shape-compare.txt`: each object flattened to a `path -> type` map; the symmetric difference is empty for all four). The kinds: 01 SessionStart startup, 02 PreToolUse Bash root, 03 SessionStart resume, 04 PreToolUse Bash subagent (`agent_id` + `agent_type`, parent `session_id`). The hook `env-names` files match the 2.1.286 ones except that the 2.1.286 files also list `CODEX_HOME` and `HCOM`; those come from the capturing shell's ambient environment, not from Claude, so they are a difference in the capture host and not in what Claude sets.

## Compact payload (`payloads/05-sessionstart-compact.json`)

Keys: `cwd`, `hook_event_name` (`SessionStart`), `model`, `prompt_id`, `session_id`, `source` (`compact`), `transcript_path`. There is no `agent_id` and no `agent_type`. The parser reads `hook_event_name`, `source`, `session_id` and the absence of `agent_id`; the other keys are not read. Against the startup payload (5 keys) compact adds `model` and `prompt_id`; against the resume payload it lacks `context_tokens`, `estimated_cache_write_usd`, `prompt_cache_likely_expired` and `seconds_since_last_response`.

## Output application after compaction (`payloads/transcript-evidence.json`)

Transcript order for the compacting run: lines 68-69 the resume `hook_success` and `hook_additional_context` (`HT-SS-MARKER-c287cp`); **line 70 `compact_boundary`**; line 71 the compact summary message; line 80 `hook_success` named `SessionStart:compact` whose stdout is the returned JSON; **line 81 `hook_additional_context` with content `["HT-COMPACT-MARKER-c287cp"]`**, after the boundary. That marker is only produced for `source: compact`, so it was produced and attached after compaction. In run4 the model repeated `HT-COMPACT-MARKER-c287cp` verbatim in a later turn. `permission_denials` is `[]` in all runs.

## Verdict

**Admitted.** All three admission clauses hold: (a) a `SessionStart` payload with `source: "compact"`, `hook_event_name`, `session_id` and no `agent_id` was captured from 2.1.287; (b) the 2.1.287 startup, resume, root-Bash and subagent-Bash payloads have the same key paths and JSON types as the 2.1.286 fixtures; (c) a SessionStart `additionalContext` returned on the compact event was attached as `hook_additional_context` after the compact boundary and the model repeated the marker. Recipe `claude-hooks-2.1.287` is added with compact supported; `claude-hooks-2.1.283` stays [2.1.283, 2.1.286] with compact unsupported (no compact evidence exists for those versions).

## Not verified

- Interactive-TUI compaction (`/compact` typed in a pane). The evidence is print mode (`--continue -p "/compact"`), observed once.
- Auto-compaction triggered by context size: only the manual `/compact` command was run.
- PreToolUse output application, `/clear`, permission prompts, user-global hook coexistence and the Herdr TUI pane (unchanged from the 2.1.286 report, not re-run).
- Model receipt as a qualified, durable gate: the marker reached the model in single print-mode runs, so `model_receipt` stays unsupported.
- Subagent compaction output: the child path returns `{}` by design.
