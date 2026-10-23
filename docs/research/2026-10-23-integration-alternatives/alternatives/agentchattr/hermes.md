# agentchattr x Hermes Agent (Nous Research)

- Upstream: https://github.com/bcurts/agentchattr, clone `scratchpad/alts/agentchattr` at HEAD `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10, `VERSION` 0.5.0). 1558 stars and 239 forks; last push 2026-09-10 (GitHub API, 2026-10-23).
- Hermes reference: `primers/hermes.md` (Hermes `main` `f3b5b980`, latest release v0.22.5), plus a few lines re-read in the Hermes clone `scratchpad/alts/hermes-agent` (https://github.com/NousResearch/hermes-agent). Paths are relative to each clone root.
- Tags: **VERIFIED** means I read it in source or docs. **INFERRED** means I reasoned it from those. **unknown** means I did not find it.

## Support status

**Not supported.** Hermes does not appear in agentchattr's code or docs, and no issue mentions it.

- VERIFIED: `grep -rni 'hermes\|nous'` over the clone finds nothing. The only hit is "asynchronous" in `agents.py:58`.
- VERIFIED: the README names the shipped harnesses: "Claude Code, Codex, Gemini CLI, GitHub Copilot CLI, Kimi, Qwen, Kilo CLI, CodeBuddy, and MiniMax — and any MCP-compatible agent can join" (`README.md:5`). `config.toml:10-94` has no `[agents.hermes]` entry. `_BUILTIN_DEFAULTS` (`wrapper.py:131-161`) covers only claude, gemini, codex, kimi and kilo.
- VERIFIED: the GitHub search `repo:bcurts/agentchattr hermes` returns 0 issues or PRs.

So any Hermes integration uses one of two generic paths. The first is the CLI wrapper (`wrapper.py`): tmux keystroke wake plus MCP pull. Most of this report covers that path. The second is the shipped API-agent wrapper (`wrapper_api.py`, `type = "api"`), which drives Hermes' OpenAI-compatible API server headlessly (see "Alternative: the API-agent path" below). The CLI-wrapper path fits Hermes worse than the mapper's note suggested, for three reasons. None of agentchattr's MCP config injection modes can deliver the per-instance bearer token to Hermes. Since v0.22.1 Hermes runs turns and owns MCP servers in a shared gateway process, so env-based tricks do not reach it. And Hermes' default busy-input mode turns a mistimed wake into an interrupt.

## Injection mechanism

### What agentchattr does generically (VERIFIED)

It is a two-step pull. No harness hook is involved.

1. **Trigger.** An @mention makes the server append a JSON line to `data/<name>_queue.jsonl` (`agents.py:32-54`): `{"sender","text","time","channel"[,"prompt"][,"job_id"]}`.
2. **Wake prompt.** The wrapper's `_queue_watcher` (`wrapper.py:473-571`) polls that file every 1 s. It reads all lines and truncates the file (`wrapper.py:483-486`), then builds one prompt:
   ```python
   prompt = f"use mcp to read #{channel} - you're mentioned, take appropriate action and respond"
   ```
   (`wrapper.py:531`; the job variant is at `:529`, and a session-engine custom `prompt` replaces it at `:526-527`). It then adds `ROLE: …` when a role is set (`:536-540`). It adds `RULES: <rules joined by "; ">` on the first trigger, when the rules epoch changes, or every N triggers (N defaults to 10, `:543-559`), and a one-time `_IDENTITY_HINT` for multi-instance agents (`:561-563`). Newlines are flattened to spaces before typing (`:564-567`).
3. **Content pull.** The model calls the `chat_read` MCP tool (`mcp_bridge.py:575-689`). This uses a per-sender, per-channel cursor. The first read returns the last `limit=20` messages and later reads return the messages after the cursor; the result is then cut with `msgs[-limit:]` and the cursor advances (`:648-663`). The payload is a JSON array of `{id,sender,text,type,time,channel[,attachments][,reply_to]}` (`_serialize_messages`, `:409-426`). It has no per-body truncation. Empty reads return escalating "do not poll" text (`:666-679`). With several instances, `[identity: … | label: …]` is prepended (`:681-688`).
4. **Framing and contract.** The behavioural rules live in the MCP server `instructions` string (`mcp_bridge.py:54-124`): "always respond in chat via chat_send", identity rules, token-aware reading, jobs, rules.
5. **Dedupe and ack.** Dedupe comes only from the cursor. There is no ACK. Once more than 20 messages arrive between reads, the older unread ones are skipped silently, because the cursor jumps to the last returned id (`mcp_bridge.py:655-663`).

### How that lands in Hermes

- **Tool surface (VERIFIED, Hermes).** Hermes reads MCP servers from `<HERMES_HOME>/config.yaml` under `mcp_servers` (`website/docs/user-guide/features/mcp.md:36`; the path is fixed at `hermes_constants.py:1257-1259`, `get_hermes_home() / "config.yaml"`). An HTTP entry takes `url` and `headers` (`website/docs/reference/mcp-config-reference.md:25-26`, `:52`). Tools are shown with an `mcp_<server>_` prefix (`mcp.md:571-582`, e.g. `mcp_github_create_issue`), so the model would see `mcp_agentchattr_chat_read` and similar. The wake text "use mcp to read #general" names no tool, so the model has to map it (INFERRED: easy for a capable model).
- **When it fires.** Content reaches a Hermes model only when the model calls the tool. That means on a woken turn, or whenever the model decides to read. agentchattr uses none of Hermes' push surfaces: `pre_llm_call`, `transform_tool_result`, `inject_message` or `session.steer` (primer §0, §2, §4). So a busy Hermes gets nothing mid-turn unless it happens to call `chat_read` (INFERRED from agentchattr having no hook code, overview §Injection).
- **The MCP `instructions` probably never reach the Hermes model (INFERRED, medium-high confidence).** Hermes keeps the `InitializeResult`, but its code inspects only `.capabilities` (`tools/mcp_tool.py:437-442`, `tools/mcp_tool_registration.py:187-204`, `tools/mcp_tool_transport.py:167`). A grep for `.instructions` across `tools/`, `agent/` and `hermes_cli/` finds only catalog or connector and coding-context uses, none on the MCP session result. So agentchattr's whole behavioural contract would be missing in Hermes: always reply with `chat_send`, use your base name, don't poll, read summaries at start. Only the tool docstrings would remain. Expected symptom: Hermes answers in its own terminal instead of the chat, which is exactly what the contract forbids (`mcp_bridge.py:76-80`).
- **Hermes' MCP description scanner** logs a warning for patterns such as `system\s*:` in tool descriptions, but never blocks (`tools/mcp_tool_schema.py:15-30`). I don't expect agentchattr's descriptions to trip it (INFERRED).
- **Size bounds.** agentchattr has no body cap. Hermes caps MCP tool results with its own tool-result limits, which this report did not examine (unknown).

### The auth blocker: no injection mode fits Hermes (VERIFIED parts, INFERRED conclusion)

The server rejects unauthenticated tool calls whose sender belongs to a configured agent family: `"Error: authenticated agent session required for '<name>'."` (`mcp_bridge.py:181-186`). Families are all `[agents.*]` keys (`registry.py:550-563`, seeded from config at `app.py:306-307`). Each wrapper run gets a fresh random token from `/api/register` (`wrapper.py:616-624`). So Hermes must present that token, which no static config can hold. The wrapper's ways of passing it (`_VALID_INJECT_MODES`, `wrapper.py:163`) all fail for Hermes:

| Mode | What it does | Why it fails for Hermes |
|---|---|---|
| `settings_file` | Merges `{"mcpServers": {...}, "security": {"folderTrust": …}}` into a JSON file (`wrapper.py:38-82`, `:234-252`) | Wrong key (`mcpServers` instead of `mcp_servers`) and a JSON writer. If it were pointed at `~/.hermes/config.yaml`, `json.loads` would fail on the YAML, `existing = {}` (`:57-61`), and the user's **whole config.yaml would be overwritten** with a JSON stub. Hermes would still find no `mcp_servers` (INFERRED). Destructive. |
| `env` | Writes a JSON settings file and exports its path in an env var (`:254-287`) | Hermes has no env var that names an MCP settings file. Its config path is always `HERMES_HOME/config.yaml` (`hermes_constants.py:1257-1259`). Also, since v0.22.1, `HERMES_*` names never travel from the client to the gateway (primer §1, `agent/session_client_env.py` `_travels`). |
| `flag` | Writes a Claude-style `{"mcpServers": …}` JSON and passes `--mcp-config <file>` (`:289-298`) | Hermes has no such flag. Its top-level options are `--profile`, `--resume`, `--continue`, `--in`, `--worktree`, `--yolo`, `--pass-session-id`, `--ignore-user-config`, `--ignore-rules`, `--tui`, `--cli`, `--dev` (`website/docs/reference/cli-commands.md:26-39`). |
| `env_content` | Puts Kilo/OpenCode JSON (`{"mcp":{…type:"remote"}}`) in an env var (`:300-310`) | Hermes has no such variable (unknown/INFERRED absent). |
| `proxy_flag` / none (the default for an unknown CLI) | Starts the per-instance identity proxy and passes its URL through a flag template (`:312-317`, `:639-661`) | The proxy binds an **OS-assigned port** (`mcp_proxy.py:84-89`, `port: int = 0`; `wrapper.py:652-657` passes no port). With no `mcp_inject`, `_apply_mcp_inject` returns no launch args (`:221-223`), so nothing tells Hermes the URL. A user cannot put a random port into config.yaml ahead of time. |

The one header-templating route on Hermes' side also fails. Hermes expands `${VAR}` in `url` and `headers` "from the active profile's secret scope (falling back to the process environment)" (`mcp-config-reference.md:79-90`). The process is the **gateway**, which serves every pane, so a token exported into the wrapper's tmux command by `env VAR=… hermes` (`wrapper_unix.py:213-222`) never reaches it (primer §1: "the turn runs in the gateway"; MCP servers are "owned by the gateway session that runs the turn", `hermes_cli/cli_agent_setup_mixin.py:649-655`). INFERRED.

**Workarounds that would work, all INFERRED and none shipped:**
1. **Manual, unauthenticated, no wrapper** (the README's "manual MCP registration" pattern, `README.md:305-325`). Add `mcp_servers: {agentchattr: {url: http://127.0.0.1:8200/mcp}}` to the Hermes profile and do **not** add `[agents.hermes]`. "hermes" is then not a family, so the model can `chat_send(sender="hermes")` unauthenticated (`mcp_bridge.py:176-190`). Cost: no wrapper means no tmux wake, no presence beyond tool-call touches, and no @mention routing to a queue (INFERRED: the router targets registered agents). Any client can spoof the name. Wake is manual: a human types "mcp read #general" (`README.md:47`).
2. **Wrapper for wake plus an unauthenticated MCP alias.** Add `[agents.hermes] command="hermes"` to get the tmux wake, but have the model send as a non-family name such as `nous`. @hermes mentions wake the pane, while replies, cursors and presence are filed under `nous`. Identity is split, and the model must be told the alias, because the MCP instructions don't reach it.
3. **One Hermes profile per agentchattr instance**, with a hand-written `.env` containing the token and `headers: {Authorization: "Bearer ${AGENTCHATTR_TOKEN}"}`. This breaks on every wrapper restart or 409 re-register, because the token rotates (`wrapper.py:780-785`, `:678-690`), and agentchattr cannot write YAML.

### Alternative: the API-agent path (VERIFIED parts, INFERRED fit)

agentchattr also ships `wrapper_api.py` (started by `macos-linux/start_api_agent.sh <name>`) for agents with `type = "api"` (`wrapper_api.py:51`; `config.toml` `[agents.minimax]` is one). It registers with `/api/register`, polls the same `<name>_queue.jsonl` every 1 s (`wrapper_api.py:306-347`), and on a trigger reads the last `context_messages` (default 20) chat messages over HTTP. It formats them as OpenAI messages (`"<sender>: <text>"`, its own messages as `assistant`, a generated system prompt with role, online agents and name, `:246-273`), POSTs them to `<base_url>/chat/completions` with `Authorization: Bearer $<api_key_env>` and a 120 s timeout (`:228-244`), and posts the reply with its own token through `/api/send` (`:216-226`, `:276-300`). No MCP and no tmux are involved, so the token problem above does not arise.

Hermes' gateway serves exactly that endpoint: `API_SERVER_ENABLED=true` and `API_SERVER_KEY=…` in `~/.hermes/.env`, then `POST http://127.0.0.1:8642/v1/chat/completions` runs a turn "with its full toolset" (`website/docs/user-guide/features/api-server.md`, Quick Start and `:163`). So `[agents.hermes] type="api" base_url="http://127.0.0.1:8642/v1" api_key_env="API_SERVER_KEY"` should give an authenticated Hermes chat participant with no code changes (INFERRED, not tried). Costs (INFERRED):
- It is a headless gateway session, not the pane's interactive session. The human never sees the turn in a TUI, and no idle/busy or draft question arises.
- Without `X-Hermes-Session-Id`, Hermes derives the session from the system prompt and first user message (`api-server.md:155-158`). agentchattr's 20-message window slides and its system prompt includes the online list, so most triggers probably land in a new Hermes session, and each replays up to 20 messages in full.
- The reply is only the final `choices[0].message.content`. Hermes cannot call `chat_read`/`chat_send` itself, and a turn longer than 120 s fails at the urllib timeout.

This path is the one agentchattr setup that gives Hermes authenticated replies. It does so by giving up the interactive pane, which is what herdr-threads exists to serve.

## Wake mechanism

### Mechanism (VERIFIED, agentchattr)

The Unix wrapper runs the CLI inside a dedicated tmux session `agentchattr-<name>` (`wrapper.py:890`, `wrapper_unix.py:244-249`) and attaches the user to it (`:255`). `inject()` (`wrapper_unix.py:64-94`) delegates to `_deliver()` (`:97-136`), which works in four steps:

```python
["tmux", "load-buffer", "-b", buffer_name, "-"]                 # text on stdin
["tmux", "paste-buffer", "-p", "-d", "-b", buffer_name, "-t", pane]  # bracketed if app enabled it
time.sleep(max(delay, len(text) * 0.001))                       # inject_delay default 0.3 s
["tmux", "send-keys", "-t", pane, "Enter"]
```

Each tmux call has a 5 s timeout (`TMUX_COMMAND_TIMEOUT = 5.0`, `:23`). A failure is printed and never retried, because "a paste may have arrived" (`:77-83`).

### Idle/busy detection and draft safety

- **None before injecting (VERIFIED).** The watcher's only condition is `has_trigger` (`wrapper.py:488-502`). It sleeps 0.5 s and types. The pane-hash activity checker (`wrapper_unix.py:159-183`) feeds only the UI status pill (`wrapper.py:842-879`). It has a Codex-only animation filter (`provider == "codex"`, `wrapper_unix.py:174-175`) and no Hermes filter, so the Hermes spinner or status bar would keep the pill "active" whenever it redraws (INFERRED).
- **Hermes busy behaviour makes this worse than on Claude (VERIFIED, Hermes).** `display.busy_input_mode` defaults to `"interrupt"` (`hermes_cli/config_defaults.py:829`; primer §9). Enter while the agent runs calls `agent.redirect()` (cancel the in-flight model request and append the text as a correction) or the interrupt queue. So a wake that lands mid-turn **interrupts or redirects Hermes' running turn** instead of queueing (INFERRED from primer §9 and the wrapper's lack of a gate). With `busy_input_mode: queue` the user could make it queue, but agentchattr never sets that.
- **Draft clobbering (INFERRED).** A human's half-typed text in the Hermes composer gets the pasted wake text appended. Enter then submits the combined line. Hermes' Ctrl+S draft stash (primer §9) could protect it, but agentchattr has no per-harness key sequence.
- **Paste handling in Hermes (VERIFIED, Hermes).** Hermes collapses pastes of 5 or more lines or 2,000 or more chars into a file reference (`hermes_cli/config_defaults.py:2506-2508`, `hermes_cli/cli_tui_mixin.py:1668-1672`) and expands them before submission (primer §9). agentchattr's newline flattening (`wrapper.py:564-567`) keeps the paste to one line. A RULES-bearing wake could still pass 2,000 chars if many rules are active (rules max 160 chars each, `mcp_bridge.py:89`). It would then show as `[Pasted text #N …]` but still be expanded and submitted (INFERRED: harmless).

### Latency (INFERRED from code)

Up to 1 s of queue poll (`wrapper.py:571`), plus a 0.5 s pre-inject sleep (`:506`), plus two loopback HTTP calls for role and rules (3 s timeouts each, `:430-451`), plus at least 0.3 s paste-to-Enter. That is about 1-2 s typical. Then the model's first turn must call `chat_read`, which adds a full model round trip before any content is read.

### Failure modes

- Coalescing: one poll's lines produce one injection, and the last line's channel, job or prompt wins (`wrapper.py:488-531`). Two mentions in different channels inside 1 s become a wake for only the last channel (VERIFIED code; INFERRED consequence).
- There is a read-then-truncate race in which a line appended between `readlines()` and `write_text("")` is lost (`wrapper.py:484-486`, INFERRED).
- Triggers queued while the wrapper was down are dropped, because the queue is cleared at start (`wrapper.py:719-721`).
- **Hermes detach semantics (INFERRED from primer §6).** `/quit`, `/exit` and closing the terminal only *detach* the thin client. The gateway session keeps running. The wrapper's restart loop (`wrapper_unix.py:267-273`, 3 s) relaunches `hermes`, which opens a **new** session unless the user put `-c` or `--resume` in the args. The old session may still be finishing a turn with nobody watching.
- **Gateway injection is invisible to the wrapper.** If anything else injects a turn (Hermes `inject_message`, background notifications), the tmux pane only shows it. agentchattr has no concept of it.
- The Hermes classic CLI and the Ink TUI (`--tui`) both accept typed input. Which composer is in the pane depends on `display.interface` (primer §9), and the paste and Enter semantics of the Ink TUI were not checked (unknown).

## Setup/installation footprint on the harness

- **Shipped: none.** There is no Hermes launcher (`macos-linux/start_*.sh` and `windows/start_*.bat` have no Hermes variant; VERIFIED by listing). The generic `start_api_agent.sh` / `.bat` would serve a user-written `type = "api"` entry.
- **For a user-written `[agents.hermes]`:**
  - agentchattr itself writes `data/provider-config/…` only for the `env` and `flag` modes, and `data/<name>_queue.jsonl` (VERIFIED, `wrapper.py:228`, `:666`). For an unknown CLI it writes nothing into Hermes config (no `mcp_inject` means no file, `wrapper.py:221-223`).
  - It creates a tmux session and strips `CLAUDECODE` from the env (`wrapper.py:723-724`).
  - The user must hand-edit `<HERMES_HOME>/config.yaml` `mcp_servers` (Hermes' own file). If they misuse `settings_file` against it, agentchattr would overwrite that file (see the table above).
  - If `mcp_inject = "env"` were chosen, `_ensure_gemini_folder_trusted` would also write `~/.gemini/trustedFolders.json` (`wrapper.py:737-738`, `:322-353`). That is an unrelated real-user file, written as a side effect of picking a mode.
- **Hermes side.** Nothing is installed: no plugin, no hooks, no shell-hook consent. Hermes may need `/reload-mcp` or a new session after config edits (`mcp.md:783-788`).

## Token cost

| Item | Estimate | Basis |
|---|---|---|
| Static tool schemas | ~850 input tokens resident every turn of every Hermes session in that profile, wake or not | README estimate, `README.md:274` (VERIFIED estimate). It applies per profile, because MCP config is per profile (INFERRED). |
| MCP `instructions` | probably 0 in Hermes (not surfaced) | INFERRED, see above. On Claude this is about 1.3k tokens of contract. |
| Wake prompt | ~20 tokens bare (`"use mcp to read #general - you're mentioned, take appropriate action and respond"`, 82 bytes). It grows with `ROLE:` and with `RULES:` (up to N × 160 chars) on the first trigger, on epoch change and every 10th trigger | `wrapper.py:531-559` (VERIFIED text, INFERRED token count) |
| Per message read | ~30 per call + ~40 per message, plus the body | `README.md:275`. It is a full model turn just to call the tool. |
| Reply | ~45 per `chat_send` | `README.md:276` |
| Quiet turn | 0 bytes injected (no hook). The ~850-token schema cost remains | INFERRED |
| Model-initiated polling | Escalating "STOP" texts limit it, and each empty read costs a tool round trip | `mcp_bridge.py:666-679` |

Compared with herdr-threads on Hermes (`pre_llm_call` context up to 4 KiB, with no extra tool call on the woken turn, primer §11), agentchattr spends an extra model round trip per wake (the `chat_read` call). It saves the per-turn hook cost, which for us is zero on quiet turns anyway.

## Limitations and bugs I can see

1. **No working authenticated path for an interactive Hermes pane** (see the table above). Every inject mode either targets a format or flag that Hermes lacks or is unreachable through the gateway. The CLI-wrapper configurations that work give up authentication or split identity. The only authenticated route is the headless API-agent path, which gives up the pane (INFERRED).
2. **`settings_file` silently overwrites any non-JSON target** (`wrapper.py:56-61`, `:81`). Pointed at Hermes' `config.yaml`, it destroys the user's config (VERIFIED code, INFERRED outcome). It is a general bug, not specific to Hermes.
3. **The wake interrupts a busy Hermes** because of `busy_input_mode: interrupt` (`config_defaults.py:829`), and agentchattr has no idle gate (`wrapper.py:488-506`). On Claude a mistimed wake queues; on Hermes it cancels in-flight work (INFERRED).
4. **The behavioural contract is lost.** The MCP `instructions` are probably not shown to Hermes models (INFERRED). So "respond in chat", identity and anti-poll rules exist only in tool descriptions.
5. **Wrong identity guidance.** `_MCP_INSTRUCTIONS` hard-codes base names for Anthropic, OpenAI, Google, Qwen and Kilo products (`mcp_bridge.py:61-68`). A Hermes model, if it saw them, would have no base name to use (VERIFIED text).
6. **The backlog skip past 20 messages** (`mcp_bridge.py:655-663`) and **coalesced multi-channel wakes** (`wrapper.py:499-500`) apply as on every harness.
7. **The activity pill has no Hermes animation filter** (`wrapper_unix.py:174-175`), so status will likely flap or stick "active" (INFERRED).
8. **Detach is not exit in Hermes** (primer §6). The wrapper's "agent exited, restart" logic assumes the process exit ends the agent, but on Hermes the gateway session lives on, so restarts accumulate orphaned gateway sessions (INFERRED).

## Clever ideas worth stealing

1. **Bracketed paste plus a separate Enter** (`wrapper_unix.py:64-136`). The docstring records a real failure mode: a long `send-keys -l` split across pty reads, of which Claude submitted only the tail. If herdr-threads' Hermes wake ever moves past a short marker, Herdr should paste with brackets. Hermes handles bracketed paste (primer §9). Our current marker `herdr-threads: attention pending; run herdr-threads inbox` is short and single-line, so it is not affected today (INFERRED).
2. **"No retry after an ambiguous inject failure"** (`wrapper_unix.py:83`). The wake is treated as at-most-once, because a resend could double-submit. This matches our idle-wake stance and is worth stating explicitly in our wake code (INFERRED relevance).
3. **Escalating empty-read texts** (`mcp_bridge.py:666-679`) are a cheap server-side guard against a model polling in a loop. It applies to our `inbox`, if a Hermes model ever loops on it (INFERRED).
4. **Rules on an epoch or refresh cadence** (`wrapper.py:543-559`, re-sent on change or every N triggers, with a sync report back to the server, `:454-471`). It is a bounded way to keep shared standing instructions fresh without per-turn cost. For Hermes the better carrier would be `register_system_prompt_section` (primer §2.4), because it is frozen once per session. This only half-applies to us, given R18 (content-free wakes).
5. **Negative lesson for herdr-threads on Hermes.** agentchattr's failure to reach Hermes through env or config injection after v0.22.1 is the same gateway-env problem the primer flags for our own bridge (`HERDR_PANE_ID` absent or wrong in gateway-run hooks, primer §12.1). The Hermes-native fix that agentchattr also lacks is `ctx.inject_message(session_key=…)` or gateway `prompt.submit` for wake, and `transform_tool_result` or `session.steer` for mid-turn delivery (primer §4, §5.1, §12.2-3). None of these has a busy-interrupt risk or draft risk.

## Verification

Adversarial pass, 2026-10-23, against the agentchattr clone at `d775776a`, the Hermes clone at `f3b5b980`, the primer and the GitHub API.

- I re-checked these and found them correct: HEAD, date, `VERSION`, stars, forks and push date (`gh api`); 0 issues or PRs for `hermes`; the only grep hit is `agents.py:58`; `README.md:5`; the `config.toml` agent list; `_BUILTIN_DEFAULTS` and `_VALID_INJECT_MODES`; the queue format; the `_queue_watcher` line refs, prompt text, ROLE/RULES/identity hint and newline flattening; `chat_read` cursor, `msgs[-limit:]` skip, empty-read texts and breadcrumb; the `_MCP_INSTRUCTIONS` contents and line refs; the auth rejection and `is_agent_family`; the inject-mode code, including `json.loads` failure leading to overwrite; the proxy's `port=0`; the `CLAUDECODE` strip; the gemini trust side effect; the codex-only activity filter; the 3 s restart; the README token figures; Hermes `mcp_servers`/`config.yaml` path; the `${VAR}` resolution text; the CLI options list; `busy_input_mode` default `interrupt`; the paste thresholds; the description scanner being warning-only; and that Hermes MCP code reads only `initialize_result.capabilities`. No `.instructions` use was found on the MCP session, so the instructions claim stays INFERRED.
- **Changed, substantive:** the report said every Hermes integration has to go through the tmux plus MCP path and that there was no authenticated path. That missed the shipped API-agent wrapper (`wrapper_api.py`, `type = "api"`), which can target Hermes' OpenAI-compatible `/v1/chat/completions`. I added a section on it and narrowed limitation 1 to interactive panes.
- **Citations fixed:** `mcp.md:484` (unrelated) removed; prefix cite `mcp.md:1006` changed to `:571-582`; `inject()` four steps are in `_deliver()` `wrapper_unix.py:97-136`; tmux timeout cite `:23` added; env prefix `wrapper_unix.py:213-222`; paste thresholds `config_defaults.py:2506-2508`, `cli_tui_mixin.py:1668-1672`; README schema cost `:274`.
- **Not checked:** the gateway-env claims (taken from primer §1); Hermes tool-result caps; Ink TUI paste semantics; any live run.
