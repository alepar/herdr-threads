# Claude Code 2.1.286 hook capture (input shapes and output application)

- Binary: `claude --version` = `2.1.286 (Claude Code)`, `~/.local/share/claude/versions/2.1.286` (the target of `~/.local/bin/claude`), SHA-256 `75e3016e9d2570767b08e43a7467d4817a4f149232c169ca295f2c95fef21433`.
- Method (same as `claude-285-hook-capture`, same `run.sh`, `hook.py`, `hook_out.py` and settings files with only the binary path and the marker changed): a scratch git project at `<scratch>/proj` (`/private/tmp/ht-claude286-scratch`) with **project-local** capture hooks only (SessionStart; PreToolUse matcher Bash), normal user auth (no `CLAUDE_CONFIG_DIR`), `--setting-sources project,local`, model `claude-haiku-4-5-20251001`, `--max-budget-usd 0.10` per run. `run.sh` launches each run from bash with every parent `CLAUDE*`/`HERDR*` variable removed through `env -u`, and records what is left. `*.parent-leak.txt` was empty for all five runs. The hook `env-names` files are identical to the 2.1.285 ones for all four payloads. No user-global settings, hooks, permissions or credentials were changed. At cleanup, `proj/.claude/settings.local.json` was deleted. Claude wrote its own transcripts for the scratch project under `~/.claude/projects/-private-tmp-ht-claude286-scratch-proj/`. `extract.py` produced the redacted `payloads/` and `run*.json` files from the raw logs and those transcripts.
- Runs:
  - **run1** starts up and runs root Bash (`echo capture-ok`), using `hook.py`, which logs and prints nothing.
  - **run2** uses `--continue`. A general-purpose subagent runs Bash (`echo child-capture-ok`).
  - **run3**, **run3b** and **run3c** are fresh sessions using `hook_out.py`, which returns the adapter-shaped output. SessionStart returns `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"HT-SS-MARKER-<m>"}}`. Root Bash PreToolUse returns `hookEventName:"PreToolUse"`, an `updatedInput` that copies every `tool_input` key and prefixes `command` with `export HERDR_THREADS_CALLER_CONTEXT='ctx_probe-<m>';\n` (the adapter's prefix), and `additionalContext:"HT-PTU-MARKER-<m>"`. It sets no `permissionDecision`. The marker was `<m>=c286f4`.
  - run3 allowed only `Bash(echo *)` (`settings-run3.json`). run3b and run3c added `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` (`settings-run3b.json`).
  - run3 and run3b used the 2.1.285 prompt wording (run `echo "ctx=$HERDR_THREADS_CALLER_CONTEXT"` and repeat every `HT-` string). In run3b the model declined to run Bash, calling the request a possible prompt injection, so no PreToolUse fired. It still quoted `HT-SS-MARKER-c286f4`. run3c repeated run3b with a prompt that says it is the user's own scratch project and hooks, and the model ran the command. run3b is kept as evidence of SessionStart delivery only.
- Cost: **$0.1142** at list price (run1 0.0212, run2 0.0475, run3 0.0154, run3b 0.0145, run3c 0.0155). There were no discarded attempts.

## Input shapes (payloads 01–04)

Each captured payload has **the same key paths and JSON value types as the 2.1.285 fixture of the same kind**. This was checked mechanically by flattening each object to a `path → type` map and comparing the sets; the symmetric difference was empty for all four. The kinds are: 01 SessionStart startup, 02 PreToolUse Bash root, 03 SessionStart resume, and 04 PreToolUse Bash subagent (`agent_id` + `agent_type`, parent `session_id`). Every field the 2.1.283 parser requires is present. The extra keys it does not read are unchanged from 2.1.285: `permission_mode`, `prompt_id`, `tool_input.description`, and on resume `context_tokens`, `estimated_cache_write_usd`, `prompt_cache_likely_expired`, `seconds_since_last_response`. There is no `event_id`. The sets of environment variable names visible to the hook are identical to 2.1.285 for all four.

## Output application (run3/run3b/run3c, `payloads/transcript-evidence.json`)

- **SessionStart `additionalContext`: delivered.** In all three runs the transcript has a `hook_additional_context` attachment with content `["HT-SS-MARKER-c286f4"]`, and the model repeated the marker verbatim.
- **PreToolUse `additionalContext`: delivered.** In run3 and run3c the transcript has a `hook_additional_context` attachment with content `["HT-PTU-MARKER-c286f4"]`, and the model repeated it.
- **PreToolUse `updatedInput`: applied (run3c).** The model's `tool_use` input was `echo "ctx=$HERDR_THREADS_CALLER_CONTEXT"` plus a description. The tool result was `ctx=ctx_probe-c286f4` with `is_error: false`, and `permission_denials` was `[]`. The `description` key was preserved in the executed input.
- **Permission interaction (run3), unchanged from 2.1.285.** 2.1.286 permission-checks the **rewritten** command. With only `Bash(echo *)` allowed, the rewrite was denied: `This Bash command contains multiple operations. The following part requires approval: export HERDR_THREADS_CALLER_CONTEXT='ctx_probe-c286f4'` (`is_error: true`). The denial is listed in `permission_denials` with the rewritten input, and `description` was preserved there too. The model's run3 answer claimed the output was `ctx=`, but the command never executed. `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` was sufficient to allow it (run3c).

## Parser acceptance

`tests/fixtures/claude-2.1.286/01–04` are payloads 01–04, redacted: the scratch path becomes `<scratch>` and home becomes `~`. No username appears. `05-run3c-pretooluse-bash-root.returned.json` pairs the run3c root input with the output that 2.1.286 applied. The unit test `captured_claude_286_payloads_parse_and_encode_as_applied` checks three things. First, all four payloads parse under 2.1.286 (Startup, Tool/TopLevel, Resume, Tool/Subagent) and are rejected under 2.1.287. Second, the child gets `{}`. Third, the adapter's own `encode_tool_response` on the fixture 05 input produces exactly the `updatedInput` that 2.1.286 executed.

## Verdict

Compatible. The recipe `claude-hooks-2.1.283` is widened to [2.1.283, 2.1.286]; the rejection sentinels move to 2.1.287.

## Not verified

- Clear, compact and `/clear` paths.
- Interactive permission prompts for the rewritten command.
- Coexistence with user-global hooks.
- The Herdr TUI pane.
- Model receipt as a qualified, durable gate. The markers reached the model in single print-mode runs only, so `model_receipt` stays unsupported.
- Subagent output: the child path returns `{}` by design and was not exercised with output.
