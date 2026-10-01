# Codex 0.158.0 live hook capture and additionalContext delivery

**Outcome: all launches succeeded on the real aisw profile, and no "workspace routing discovery failed" occurred. I captured live payloads for SessionStart startup and resume, root PreToolUse Bash, SubagentStart, and child PreToolUse Bash. Codex delivered context-only `additionalContext` to the model for both SessionStart and PreToolUse. The session rollout shows each marker as a `developer` message, and the model quoted both markers back.**

`$S` = a private scratch capture directory under `/private/tmp`. `$CODEX_HOME` = the aisw Codex profile directory. Payload files use these placeholders.

**Redaction (committed copy):** the capture's `$S` path embedded the local username and a session id, and `$CODEX_HOME` named the aisw profile. In this committed copy both are replaced by the placeholders above in `report.md` and `run{1,2,3}.sh`; the `run*.sh` scripts are therefore records, not runnable verbatim. No other file contained either value. The run1 hook `$S/capture.py` was byte-identical to `../codex-158-hook-capture/capture.py` as committed at `508c812`; that script's local default log path has since been removed (`HT_CAP_LOG` is mandatory), which does not affect run1 because `HT_CAP_LOG` was set.

## Setup
- Binary: `command codex --version` reports `codex-cli 0.158.0`. Model: `gpt-6-luna`, `model_reasoning_effort="low"`.
- Each script first runs `aisw workspace check --tool codex` (rc 0), then `HERDR_AGENT=codex command codex --no-daemon exec ...`, which mirrors the user's shell function.
- I dropped `--approve-for-me` from the function's flags, because it forces the workspace-write reviewer path. All runs used `-s read-only`.
- Hooks were passed only as `-c hooks.<Event>=[...]` overrides, together with `--ignore-user-config --dangerously-bypass-hook-trust --json --skip-git-repo-check`. I wrote no trust, hook or config entries anywhere.
- Exact commands are in `run1.sh`, `run2.sh` and `run3.sh`. The hook scripts are `$S/capture.py` (silent) and `inject.py` (logs the payload and prints the marker JSON).
- Stdin must be `/dev/null`. The first attempt (run0) hung on "Reading additional input from stdin..." and timed out after 300s. It spent no tokens.

## Runs and usage
The usage figures below are from `turn.completed`. Auth is the ChatGPT account (`tokens` auth), so Codex reports no dollar cost. The total is about 118.6k input tokens (103.4k of them cached) and 307 output tokens. Even at API list rates for a small model, that is well under $0.30.

| Run | Mode | Events captured | Usage (in / cached / out) |
|---|---|---|---|
| run1 | `--ephemeral`, capture-only; prompt: echo capture-ok, then spawn one subagent to run echo child-ok | SessionStart startup, PreToolUse root, SubagentStart, PreToolUse child | 59241 / 53248 / 136 |
| run2 | persisted session, inject hooks; prompt asks for markers before and after running echo capture-ok | SessionStart startup (marker returned) | 14752 / 11008 / 40 |
| run3 | `codex exec -s read-only -C proj resume <run2 id> ...`, inject hooks | SessionStart resume, PreToolUse root (markers returned) | 44611 / 39168 / 131 |

## Captured payloads (`payloads/`, redacted paths)
- **SessionStart startup** has the keys `session_id, transcript_path, cwd, hook_event_name, model, permission_mode, source="startup"`. With `--ephemeral`, `transcript_path` is `null`. Without it, the value is `$CODEX_HOME/sessions/.../rollout-*.jsonl`. There is no `turn_id`.
- **SessionStart resume** has the same key set, with `source="resume"` and the same `session_id` as the original session.
- **PreToolUse root** has the keys `session_id, turn_id, transcript_path, cwd, hook_event_name, model, permission_mode, tool_name="Bash", tool_input={"command":"echo capture-ok"}, tool_use_id="exec-<uuid>"`.
  - The model actually called the code-mode `exec` custom tool (JS `tools.exec_command({cmd:"echo capture-ok"})`).
  - The hook still reports `tool_name` `Bash` with the raw command, not the command wrapped as `/bin/zsh -lc`.
- **SubagentStart** has the keys `session_id` (the parent's), `turn_id` (a new child turn), `transcript_path, cwd, hook_event_name, model, permission_mode, agent_id=<uuid>, agent_type="default"`.
- **PreToolUse child** has the root key set plus `agent_id` and `agent_type`, which match SubagentStart. Its `session_id` is the parent session's, and its `turn_id` is the child turn from SubagentStart.
  - No child SessionStart fired.
  - The event stream showed only a `collab_tool_call` `wait` item, not a spawn item.
- **`permission_mode` was `bypassPermissions` in every payload, even with `-s read-only`.** Approval policy is `never` under exec, so this reflects approvals, not the sandbox. Worth noting for adapter policy.
- **Env seen by hooks** (names only, full lists in `hooks-run*.jsonl`): `CODEX_HOME`, `AISW_SHELL_HOOK`, `HERDR_AGENT`, `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`, `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH`. Codex sets no `CODEX_*` hook-specific variables.
- All payloads match the 0.158.0 embedded schemas and the adapter recipe described in `../report.md` §3–4.

## additionalContext delivery: observed
The hook stdout was exactly the adapter shape:
- SessionStart returned `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"herdr-threads-session-marker"}}`.
- PreToolUse returned the matching shape with `herdr-threads-capture-marker`.

The ground truth is the persisted rollout (excerpt in `payloads/run2-run3.rollout-model-items.json`):
- line 8: a `developer` message `herdr-threads-session-marker`. It is placed after the environment_context and before the run2 user prompt.
- line 20: on resume, SessionStart **re-injects** a fresh `developer` message `herdr-threads-session-marker` before the run3 user prompt. The marker therefore appears twice in the history.
- line 25: the tool call. line 27: a `developer` message `herdr-threads-capture-marker`. line 29: the tool output. So PreToolUse context is inserted after the call and before its output. The command still ran, which confirms that context-only output does not block.
- In run3 the model answered: "Before the tool call: `herdr-threads-session-marker` / After the tool call: `herdr-threads-capture-marker`".

Caveat: in run2 the model wrongly answered "NONE-BEFORE" even though the marker is in its context (line 8). It also refused to run echo, claiming the read-only sandbox blocked it. That is a lapse by the low-effort model, not a delivery failure. The rollout proves delivery, and run3 confirms the model can see the marker.

## Side effects in `$CODEX_HOME` (Codex's own writes; I wrote nothing)
- Created by Codex: `sessions/2026/09/29/rollout-2026-09-29T17-02-14-01a0ef9e-76a2-7830-8c1f-431f5368a539.jsonl`. This is the run2/run3 session, needed to test resume.
- Codex also created `plugins/cache/openai-curated-remote/pages/0.1.18/**`. Modified: `models_cache.json`, `cache/codex_apps_*`, `plugins/cache/.../.codex-remote-plugin-install.json`, and the sqlite dbs (state, logs, goals, memories, queue, thread_history).
- `config.toml`, `auth.json`, `session_index.jsonl` and `history.jsonl` are unchanged. No hooks or trust entries were added.
- Attribution caveat: the <profile> app-server daemon (0.159.1) and other codex sessions were running concurrently, so some sqlite and cache changes may not be from these runs.
- The rollout file can be deleted if unwanted. I left it in place, per the instruction not to touch the profile.
- `~/.codex` was not touched (`config.toml` is from Sep 28 and `hooks.json` from Sep 26).

## Cleanup
The hooks existed only as `-c` overrides. `$S/proj` contains only `README` and `.git`: there is no `.codex/` and no hooks file, so nothing needed removal. I made no changes to herdr-threads source, Git, Beads or Herdr.

## Files
`run{1,2,3}.sh`, `run{1,2,3}.events.jsonl`, `run{1,2,3}.stderr`, `run0-stdin-hang.stderr`, `hooks-run{1,2,3}.jsonl` (raw hook log, paths redacted, env names only), `inject.py`, `payloads/*.json`, `profile-before.txt` and `profile-after.txt` (mtime/size listing).
