# Claude Code 2.1.285 hook capture (input shapes and output application)

- Binary: `claude --version` = `2.1.285 (Claude Code)`, `~/.local/share/claude/versions/2.1.285`, SHA-256 `51f09bd1e021d9fa8a1864c179799bd37cb39962a937935c5cf6823398e86db4`.
- Method (same as `claude-284-hook-capture`): a scratch git project with **project-local** capture hooks only (SessionStart; PreToolUse matcher Bash), normal user auth (no `CLAUDE_CONFIG_DIR`), `--setting-sources project,local`, model `claude-haiku-4-5-20251001`, `--max-budget-usd 0.10` per run. `run.sh` launches each run from bash with every parent `CLAUDE*`/`HERDR*` variable removed through `env -u`, and records what is left. `*.parent-leak.txt` was empty for all four valid runs. The `CLAUDE*` names in the hook `env-names` files are the ones Claude itself sets for hooks, and they match the 2.1.284 set. No user-global settings, hooks or credentials were changed. At cleanup, `proj/.claude/settings.local.json` was deleted. Claude wrote its own transcripts for the scratch project under `~/.claude/projects/`.
- Runs:
  - **run1** starts up and runs root Bash (`echo capture-ok`), using `hook.py`, which logs and prints nothing.
  - **run2** uses `--continue`. A general-purpose subagent runs Bash (`echo child-capture-ok`).
  - **run3** and **run3b** are fresh sessions using `hook_out.py`, which returns the adapter-shaped output. SessionStart returns `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"HT-SS-MARKER-<m>"}}`. Root Bash PreToolUse returns `hookEventName:"PreToolUse"`, an `updatedInput` that copies every `tool_input` key and prefixes `command` with `export HERDR_THREADS_CALLER_CONTEXT='ctx_probe-<m>';\n` (the adapter's prefix), and `additionalContext:"HT-PTU-MARKER-<m>"`. It sets no `permissionDecision`. The model was asked to run `echo "ctx=$HERDR_THREADS_CALLER_CONTEXT"` and to repeat every `HT-` string in its context. The marker was `<m>=eb75e8`.
  - run3 allowed only `Bash(echo *)`. run3b added `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)`.
- Cost: the valid runs cost **$0.1027** at list price (0.0214 + 0.0512 + 0.0149 + 0.0153). An earlier, discarded attempt cost $0.0956. Its launcher was zsh, where an unquoted `$UNSET` does not word-split, so `env -u` removed nothing and the parent `HERDR*`/`CLAUDE*` env leaked into the hooks (visible in its env names). Its payloads are not used. Total spend was $0.198.

## Input shapes (payloads 01–04)

Each captured payload has **the same key set and value types as the 2.1.284 fixture of the same kind**: 01 SessionStart startup, 02 PreToolUse Bash root, 03 SessionStart resume, 04 PreToolUse Bash subagent (`agent_id` + `agent_type`, parent `session_id`). Every field the 2.1.283 parser requires is present. The extra keys it does not read are unchanged from 2.1.284: `permission_mode`, `prompt_id`, `tool_input.description`, and on resume `context_tokens`, `estimated_cache_write_usd`, `prompt_cache_likely_expired`, `seconds_since_last_response`. There is no `event_id`.

## Output application (run3/run3b, `payloads/transcript-evidence.json`)

- **SessionStart `additionalContext`: delivered.** The transcript has a `hook_additional_context` attachment with content `["HT-SS-MARKER-eb75e8"]` in both runs. The model repeated the marker verbatim.
- **PreToolUse `additionalContext`: delivered.** The transcript has a `hook_additional_context` attachment with content `["HT-PTU-MARKER-eb75e8"]` in both runs. The model repeated it.
- **PreToolUse `updatedInput`: applied (run3b).** The model's `tool_use` input was `echo "ctx=$HERDR_THREADS_CALLER_CONTEXT"` plus a description. The tool result was `ctx=ctx_probe-eb75e8` with `is_error: false`, and `permission_denials` was `[]`. The `description` key was preserved in the executed input.
- **Permission interaction (run3).** 2.1.285 permission-checks the **rewritten** command, not the command the model proposed. With only `Bash(echo *)` allowed, the rewrite was denied: `This Bash command contains multiple operations. The following part requires approval: export HERDR_THREADS_CALLER_CONTEXT='ctx_probe-eb75e8'`. The denial appears in `permission_denials` with the rewritten input. The model's run3 answer quotes the token, but it read the token from the denial text; the command never executed. When the model omitted `description`, the rewrite added none. So an unattended (print-mode) session needs an allow rule covering the export prefix. `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` was sufficient here. An interactive session would presumably prompt; that path was not exercised.

## Parser acceptance

`tests/fixtures/claude-2.1.285/01–04` are payloads 01–04, redacted (scratch path becomes `<scratch>`, home becomes `~`, username becomes `<user>`). `05-run3b-pretooluse-bash-root.returned.json` pairs the run3b root input with the output 2.1.285 applied. The unit test `captured_claude_285_payloads_parse_and_encode_as_applied` checks three things. First, all four payloads parse under 2.1.285 (Startup, Tool/TopLevel, Resume, Tool/Subagent) and are rejected under 2.1.286. Second, the child gets `{}`. Third, the adapter's own `encode_tool_response` on the fixture 05 input produces exactly the `updatedInput` that 2.1.285 executed.

## Not verified

- Clear, compact and `/clear` paths.
- Interactive permission prompts for the rewritten command.
- Coexistence with user-global hooks.
- The Herdr TUI pane.
- Model receipt as a qualified, durable gate. The markers reached the model in single print-mode runs only, so `model_receipt` stays unsupported.
- Subagent output: the child path returns `{}` by design and was not exercised with output.
