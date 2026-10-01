# Claude native caller probe

Tested on macOS with Claude Code 2.1.283 and Herdr 0.9.1 on 2026-09-27. The probe uses one owned Herdr pane, w4:p3, created from the assigned w4:p1, and an additional session settings file. Existing Claude hooks and permission rules remained enabled. The session used `HCOM_DIR=/private/tmp/ht4is-claude-probe.pyzgDs/hcom`, `HCOM_AUTO_APPROVE=0`, and `HCOM_AUTO_TRUST_WORKSPACE=0`. A non-login shell in w4:p3 printed and checked `HERDR_ENV=1`, `HERDR_WORKSPACE_ID=w4`, `HERDR_TAB_ID=w4:t1`, and `HERDR_PANE_ID=w4:p3`.

## Reproduce the native capture

Use a new pane and a new private directory; do not reuse the historical IDs below. Run `claude --version` and `herdr --version` first. The hook is [capture.py](../../tests/native/provenance/claude/capture.py). Add this session-only settings file through `claude --settings /private/tmp/<probe>/settings.json`:

```json
{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"python3 /absolute/path/to/tests/native/provenance/claude/capture.py","timeout":10}]}]}}
```

Set `CLAUDE_PROBE_LOG=/private/tmp/<probe>/events.jsonl`, `CLAUDE_PROBE_NONCE=<unique harmless nonce>`, `CLAUDE_PROBE_VERSION=2.1.283`, and `CLAUDE_PROBE_GENERATION=1` only in that pane. Set the isolated hcom variables above. Run this exact Bash tool command from the native parent, one native Agent child, the parent again, and two concurrent Agent children:

```sh
printf 'HT4IS_CONTEXT=%s\n' "$CLAUDE_PROBE_CONTEXT"
```

The hook records only allowlisted metadata and returns a PreToolUse `updatedInput` that prefixes this exact command with `export CLAUDE_PROBE_CONTEXT=<new random token>;`. It never returns a permission decision. A tool result containing the token is evidence that the same Bash invocation received the hook output. The first trial used a shell assignment without `export`; expansion yielded an empty value. It is retained as the negative control.

The observer runs `herdr pane get` and `herdr pane process-info` during the hook, before the Bash command. It retains the pane's reported session ID and the single Claude PID, omitting all process command lines. The hook's direct parent PID matched that Claude PID in the observed native rows. To inspect the host without a tool call, run:

```sh
python3 tests/native/provenance/claude/capture.py --observe <owned-pane-id>
```

To check native tool results without printing arbitrary transcript content, run:

```sh
python3 tests/native/provenance/claude/capture.py --verify-receipts \
  /private/tmp/<probe>/events.jsonl \
  /Users/<user>/.claude/projects/<owned-project-key>
```

The receiver follows the root transcript for parent calls and `subagents/agent-<agent_id>.jsonl` for child calls. It requires the exact original Bash tool input and exact printed token for the same `tool_use_id`. The private raw event log from this run is retained at `/private/tmp/ht4is-claude-probe.pyzgDs/events.jsonl`; its SHA-256 is in [manifest.json](../../tests/fixtures/callers/claude/manifest.json). Fixtures use synthetic IDs and preserve the raw log line, harmless nonce, hook/result times, native transcript kind, and field provenance.

The preturn `--observe` responses were recovered from the **original coordinator tool output**, separate from the Claude hook log. The source is `~/.codex/sessions/2026/09/27/rollout-2026-09-27T07-29-43-01a0e345-9682-7d73-82ca-8e9e7cceb2e2.jsonl`. [cases.json](../../tests/fixtures/callers/claude/cases.json) retains 14 allowlisted source rows with original line numbers, SHA-256 of each exact source line, control-event order, observer times, and hashed session/PID identities. The original stdout and a recovered copy remain private. Replay the source check locally with:

```sh
python3 tests/native/provenance/claude/capture.py --verify-transition-sources \
  ~/.codex/sessions/2026/09/27/rollout-2026-09-27T07-29-43-01a0e345-9682-7d73-82ca-8e9e7cceb2e2.jsonl \
  tests/fixtures/callers/claude/cases.json w4:p3
```

This returned `validated 14 original rollout source rows`. The extractor is `capture.py --recover-timeline <rollout-jsonl> <pane-id>`; it emits only the probe's observer and control rows. The fixture ties the resumed same-conversation observation to native hook line 10 and the preturn `/clear` observation to hook line 11. The fresh process produced a preturn observation, then was cleared before its first model turn, so it has no same-conversation hook result.

## Native observations

| Scenario | Source rows | Result |
| --- | --- | --- |
| Initial root command | Hook 1 | Hook ran; the first rewrite printed an empty context. Negative control. |
| Root → child → root | Hooks 2–4 | All three exact tool results received distinct contexts. The child alone had `agent_id` and `agent_type`. These early rows lack in-hook host observations. |
| Concurrent children | Hooks 5–6, 12–13 | Distinct child IDs and contexts. Lines 12–13 include in-hook host observations about 39–60 ms apart. |
| Root and child with host observation | Hooks 7–8 | Each hook's parent PID was 54472, matching the live Herdr Claude PID. Both hook session IDs matched the host session ID; only line 8 carried child fields. Both tool results matched their contexts. |
| `/clear` in the same process | Hooks 9, 11; rollout 495, 502, 507, 512 | The recovered observer recorded the old UUID at 14:53:56.775Z, `/clear` text and Enter, then the new UUID at 14:54:46.883Z with the same PID 60170 and host revision 8. The first later hook ran at 14:55:45.859Z and observed the new UUID at 14:55:45.871Z. |
| Exit and resume the same conversation | Hook 10; rollout 379, 400, 405, 410, 418, 423 | The recovered observer recorded the same UUID with PID 54472 before exit and PID 59469 after resume, before the first resumed turn. Hook 10's parent PID matched 59469 and its result received the context. |
| Fresh replacement before first turn | Rollout 423, 443, 448, 453, 467, 495 | Exit removed the old pane agent. Fresh launch and observer recorded a new UUID and PID 60170 at 14:53:56.775Z. That conversation was cleared before a tool call, so this is a native preturn observation without same-conversation invocation proof. |
| Native environment identity | Hooks 14–15 | `CLAUDE_CODE_SESSION_ID` matched the parent session in both root and child hooks. No allowlisted child-specific environment ID appeared; `agent_id` in hook input supplied the child distinction. Both native tool results matched. |

The per-call shape is useful for Claude: child tool hooks include `agent_id` and `agent_type`; root hooks omit them, as [Claude's hook reference](https://code.claude.com/docs/en/hooks) documents. All calls shared the parent session ID, transcript path hash, Herdr pane variables, and Claude PID. Those shared fields alone cannot distinguish a child. The host's process list plus hook parent PID provided live process correlation for this tested version. The timestamped in-hook observation preceded the matching native tool result; line 7's host observation at 14:43:42.490Z preceded its result at 14:43:43.397Z.

## Classification and limits

[cases.json](../../tests/fixtures/callers/claude/cases.json) contains 15 native cases and 11 labeled synthetic mutations. The [classifier](../../tests/native/provenance/claude/classify.py) returns `ROOT_CANDIDATE` only for the tested version and event shape, matching native session and host process PID, a confirmed same-invocation result, and a fresh in-hook observation. It returns `CHILD` when those checks pass and a nonempty `agent_id` is present. Missing, unknown, cached predecessor, stale generation, expired context, known replacement, and evidence seen only after hook return resolve to `UNSUPPORTED`. Fixture generation, freshness, expiry, and replacement flags model later service checks; the native probe did not issue a daemon permit.

No stale predecessor host RPC was observed during the controlled Claude replacement: the recovered source rows show the new process and session before the first turn. The fixture named `replacement_before_observation` remains a synthetic denial and is separate from the three native transition records. Cached predecessor PID/session cases are synthetic denials. Herdr's session and process observations are separate RPCs, so a replacement between them can yield an inconsistent pair; such a pair must be rejected by a later adapter. Replacement after observation but before a service transaction remains a residual race. The observer here does not prove atomic native-current-at-commit authority.

The per-harness capture result is **supported for this bounded native attribution and invocation transport probe**. It does not authorize thread ACKs or release the shared two-harness gate. Other Claude versions, launch modes, tool types, and missing hooks are unsupported until tested. The installed global claude-mem SessionStart hook emitted a nonblocking error; it was left unchanged.
