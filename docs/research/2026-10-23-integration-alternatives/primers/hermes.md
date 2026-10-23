# Harness primer: Hermes Agent (`hermes`, Nous Research)

This primer lists every Hermes Agent surface that can put text into the model's context or wake an idle session. The later alternative x harness reports use it as their shared reference. The primer is about the harness itself. Section 11 says briefly what herdr-threads does today.

- **Not installed locally.** `which hermes` finds nothing on `PATH`. `~/.hermes/` exists (dated 2026-06-14), but it holds only empty data directories and no install. No CLI help was observed. No Hermes config was read or written.
- Source: a shallow clone of `https://github.com/NousResearch/hermes-agent` at `main` `f3b5b980d6158794beb3d1044fd7b8ca4b8f0b23` (committed 2026-10-23). The clone is treated as untrusted data: it was read and nothing in it was run. Paths below are relative to the clone root.
- Docs: the same clone's `website/docs/**` (Docusaurus, `url: https://hermes-agent.nousresearch.com`, `baseUrl: /docs/`, `website/docusaurus.config.*` line 11-12). A file `website/docs/X.md` is served at `https://hermes-agent.nousresearch.com/docs/X` (the slug is INFERRED from the Docusaurus convention; the file contents are VERIFIED).
- Releases (VERIFIED, `gh release list -R NousResearch/hermes-agent`): latest is **v0.22.5** (2026-10-23). v0.22.0 came out 2026-10-14 and v0.21.7 on 2026-10-12. The committed package version is always `0.0.0`. Release jobs derive the real version, and `0.21.5` was the last legacy CalVer release (`website/docs/developer-guide/stable-releases.md` lines 8 and 16).
- herdr-threads' native evidence is for **Hermes 0.21.5** (`integrations/hermes/README.md`). Since **v0.22.1**, Hermes has made a large architectural change. Section 1 describes it; it is the most important fact in this primer.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from those and not observed. **unknown** means not found.

---

## 0. Summary: which surfaces inject and which wake

| Surface | Injects into context | Wakes an idle session | Mid-turn delivery | Notes |
|---|---|---|---|---|
| Plugin hook `pre_llm_call` (Python `ctx.register_hook`) | yes, appended to the **current user message** | no | no (once per turn, before the tool loop) | herdr-threads' current context path. Spills to a file above 10,000 chars |
| Shell hook `pre_llm_call` (`config.yaml` `hooks:`) | yes, `{"context": ...}` on stdout | no | no | Claude-Code-compatible JSON. First-use consent per `(event, command)` |
| Plugin hook `transform_tool_result` | yes: it **replaces or extends any tool result** before the model sees it | no | **yes**, at every tool result | herdr-threads does not use it |
| Plugin hook `pre_verify` | yes, as a synthetic user turn (`continue`) | no (keeps an ending turn going) | at turn end, only when code was edited | Capped by `agent.max_verify_nudges` (default 3) |
| `ctx.register_system_prompt_section` | yes, frozen into the system prompt **once per new session** | no | no | Max 4,000 chars per section and 8,000 in total |
| **`ctx.inject_message(content, session_key=...)`** | yes, as queued user input | **yes**: an idle session starts a turn | busy: queued for the next turn (gateway and TUI); in-process classic CLI: **interrupts** the turn | Needs `plugins.entries.<id>.allow_gateway_injection: true` in gateway mode |
| `ctx.post_message(text)` | no (a notice for the human only) | no | n/a | In the classic CLI it is printed at the end of the next turn |
| Gateway JSON-RPC `prompt.submit` / `session.steer` / `session.redirect` | yes | **yes** (`prompt.submit` on an idle session) | `session.steer`: "Inject text into the next tool result without interrupting the turn" | Same-user private control socket mints a ticket. WebSocket `/api/ws`. A native wake API with no keystrokes |
| OpenAI-compatible API server `/v1/runs`, `/v1/runs/{id}/steer` | yes | starts runs (its own sessions) | `steer` at the next tool boundary | HTTP and SSE. Not attached to a pane's session |
| ACP (`hermes acp`) | `session/prompt` | host-driven | `cancel` only | stdio JSON-RPC for IDEs |
| Background-process notifications (`terminal(background=true, notify_on_complete=true)` or `watch_patterns`) | yes, `[IMPORTANT: ...]` text with up to 2,000 chars of output tail | **yes**, when the session is idle **and a viewer is attached** | queued | The model must start the process itself. Watch patterns are rate-limited |
| `/loop`, `/goal`, cron | stored prompt | yes (self-scheduled) | no | Every fire costs a full turn |
| Keystroke injection into the classic CLI (prompt_toolkit) or the Ink TUI | yes, as a user prompt | yes when idle | busy: `display.busy_input_mode` decides. The default is **`interrupt`** | Pastes of 5 or more lines or 2,000 or more chars collapse to a file reference |
| MCP tools/resources | only when the model calls a tool (`read_resource` and the other utilities are generated tools) | no | tool result only | `tools/list_changed` refreshes. Resource and prompt `list_changed` are **ignored**. Logging goes to `agent.log` only. Sampling and elicitation are supported |
| Gateway event hooks (`~/.hermes/hooks/*/HOOK.yaml`) | no | no | no | Observers in the messaging gateway only |
| Outbound webhooks (`hooks.outbound`) | no | no (they can wake *another* Hermes through its inbound webhook) | no | Notify-only |

---

## 1. The architectural shift: every session runs in one gateway process (v0.22.1 and later)

This is the most important fact for herdr-threads. Everything else in this primer should be read with it in mind.

- **VERIFIED** (v0.22.1 release notes, `gh release view v0.22.1`, line 104): "Route normal classic and one-shot launch through the gateway". The same release adds a "transport-only classic gateway client" and "gateway-owned local sessions through authenticated controls". The v0.22.0 notes add "one gateway per machine is the default" and say that one gateway serves every profile.
- **VERIFIED** `hermes_cli/gateway_agent.py` lines 1-6: "``run_conversation`` submits ``prompt.submit`` and replays the gateway's event stream ... The CLI process never builds an agent, never runs a tool and never writes ``state.db``: the turn runs in the gateway."
- **VERIFIED** `hermes_cli/cli_agent_setup_mixin.py` lines 649-655: `from hermes_cli.gateway_agent import GatewayAgent as AIAgent  # the gateway runs the turn` is unconditional. "No deferred plugin discovery either: the gateway that runs the turn loads the plugins (PLG-7)." "No MCP discovery here: the gateway session that runs the turn owns its MCP servers."
- **VERIFIED** `website/docs/user-guide/features/plugins.md` line 95: "Every Hermes session (`hermes`, `hermes --tui`, the Desktop app, messaging and cron) runs in the one gateway, which discovers plugins once for the whole process". Lines 1105-1109: "Hooks, tools and injected turns of every chat and every local session (CLI, TUI, Desktop) run in the one gateway process, not in the client you launched. ... `os.getcwd()` is the gateway's own directory ... Use `ctx.session_cwd()`".
- **VERIFIED** `agent/session_client_env.py` lines 1-11 and 21-30: a shell client sends only an **allowlisted** `client_env` with `session.create`. "the one gateway now runs every session, so they are frozen with the session's policy and bound for its turns only, never written to the gateway's ``os.environ``". The allowlist holds `HERMES_EPHEMERAL_SYSTEM_PROMPT`, `HERMES_PREFILL_MESSAGES_FILE`, `HERMES_INFERENCE_PROVIDER`, `HERMES_ENABLE_PROJECT_PLUGINS`, toolchain names (`PATH`, `VIRTUAL_ENV`, `GOPATH`, `GOROOT`, `CARGO_HOME`, `RUSTUP_HOME`, `JAVA_HOME`), the prefixes `CONDA_`, `NODE_`, `NVM_`, `PYENV_`, and the profile's `terminal.client_env_passthrough` list. Names starting with `HERMES_` and secret-looking names never travel (`_travels`, lines 48-54).
- **VERIFIED** `tools/environments/base_session_env.py` lines 12-31: the gateway injects per-session variables (`HERMES_SESSION_*`, `HERMES_UI_SESSION_ID`, ...) fresh into every terminal command's process env. One long-lived bash backend serves many sessions.

Consequences for any pane-bound integration (all INFERRED, none observed live):
1. A plugin hook callback, and any subprocess it starts with `subprocess.Popen` and the default env, runs with the **gateway daemon's** environment. It does not run with the pane's. `HERDR_PANE_ID` from the pane is absent unless the operator lists it in `terminal.client_env_passthrough`. Even then it is reachable only through `agent.session_client_env.tool_env()` / `toolchain_environ()` inside the session's context, not through `os.environ`.
2. If the gateway daemon was first started from inside one Herdr pane, it may have **inherited that pane's `HERDR_PANE_ID`**. Every later Hermes session in other panes would then attribute to the wrong pane. Whether the bootstrap scrubs the environment is unknown.
3. The model's terminal tool calls (`herdr-threads inbox`, `ack`, ...) are children of the gateway, not of the pane's process tree. Process-ancestry recognition (Herdr's agent detection, or herdr-threads' `caller_env` process walk) cannot find the pane from them. They do get `HERMES_SESSION_ID` injected per command.
4. The process in the pane is now a thin client (still `python` with `hermes_cli`). Screen-based idle/busy detection of the TUI is unaffected (INFERRED).

`gateway.standalone: true` is named in the v0.22.0 notes as a temporary opt-out from the *multiplexed messaging gateway*. Nothing found shows that it restores in-process classic CLI turns (unknown).

---

## 2. Plugin hooks (Python plugins, `ctx.register_hook`)

Docs: `https://hermes-agent.nousresearch.com/docs/user-guide/features/hooks` (`website/docs/user-guide/features/hooks.md`). Registry: `hermes_cli/plugins.py` lines 108-203 (`VALID_HOOKS`).

### 2.1 Packaging and consent (VERIFIED)
- A directory plugin is `plugin.yaml` plus `__init__.py` exposing `register(ctx)` (`hermes_cli/plugins.py` lines 1-8). The sources are bundled, user (`~/.hermes/plugins/<name>/`, or `$HERMES_HOME/plugins` per profile), project (`./.hermes/plugins/`, **not loaded by the gateway**, plugins.md line 95), and pip entry points (`hermes_agent.plugins`).
- General plugins are disabled by default. `hermes plugins enable <name>` adds the name to `plugins.enabled` (plugins.md lines 169-179).
- `plugins.isolation: host` (the default is `in_process`) runs plugins in a separate plugin-host process (plugins.md lines 853-877). `ctx.post_message` is unavailable there (line 1013).
- Callbacks receive kwargs plus `telemetry_schema_version="hermes.observer.v1"`. Timeout-bounded hooks (`pre_llm_call`, `post_tool_call`, `pre_tool_call`) use `plugins.hook_callback_timeout`: default 30 s, `0` disables it, max 600 (`hermes_cli/plugins_dispatch.py` lines 164-165; hooks.md line 396). On timeout `pre_tool_call` fails **closed** and the others fail open.

### 2.2 Hooks that put text into the model's context (VERIFIED, hooks.md table lines 449-496 and sections)

| Hook | Timing | Return that reaches the model |
|---|---|---|
| `pre_llm_call` | Once per turn, before the tool loop, after compression (`agent/turn_context.py` `_collect_pre_llm_call_context`, lines 799-853) | `{"context": str}` or a plain str. All returns are joined with `\n\n` in plugin order (alphabetical by directory) and appended to the **current user message**, never to the system prompt (hooks.md lines 700-719). Each piece is spilled to disk above `hooks.output_spill.max_chars` (default 10,000; `tools/hook_output_spill.py` line 24, with a 500/500 head/tail preview). It is skipped entirely when `agent._persist_disabled` is set (turn_context.py lines 806-807). The string user message keeps clean `content`, and the injected bytes go to an `api_content` sidecar that replay reuses. A multimodal turn gets an extra durable text part. |
| `transform_tool_result` | After `post_tool_call`, before the result is appended to the conversation | The first `str` replaces the tool result (hooks.md lines 1671-1700). It applies to every tool. **This is a mid-turn injection channel.** |
| `transform_terminal_output` | Inside `terminal`, and for background-process notifications | The first str replaces the output (lines 1704-1745) |
| `pre_verify` | At the edited-code verify gate, when a turn would finish | `{"action":"continue","message"}` or the Claude Stop shape `{"decision":"block","reason"}`. The message becomes a synthetic user turn and the loop runs again. Capped by `agent.max_verify_nudges` (default 3) (lines 825-886). It fires only when the turn edited code. |
| `pre_tool_call` | Before each tool | `block` (the message becomes the tool error the model sees), `approve` (escalates to the human), or `modify` (merges args) (lines 548-592) |
| `transform_llm_output` | Before the final reply is delivered and persisted | Replaces the assistant's text (the model does not see it until the next turn's history) |

### 2.3 Observer hooks relevant to session tracking (VERIFIED, hooks.md table)
- `on_session_start`: first turn of a new session. Payload `session_id`, `model`, `platform`. Fires only `if not conversation_history`.
- `on_session_end`: fires **after every turn** despite its name (`completed`, `interrupted`, `turn_exit_reason`, ...). It is the only per-turn "turn finished" observer, so it can serve as an idle-edge signal (INFERRED).
- `on_session_finalize`: teardown (`/new`, quit, gateway shutdown). Its return can show a notice to the human; it never becomes a model turn.
- `on_session_reset`: `/new` / `/reset`. The gateway form carries `old_session_id` and `new_session_id`.
- `post_compaction`, `agent_loop_stopped` (interrupt), `subagent_start` / `subagent_stop` (`parent_session_id`, `child_session_id`, ...), `on_human_input_request` / `_resolved` (the agent is blocked on a person: approval, clarify, sudo), `pre_command` (a slash command, observer only), and the streaming observers `on_stream_start/delta/end` and `on_interim_message`.
- `pre_llm_call` kwargs: `session_id`, `task_id`, `turn_id`, `user_message`, `conversation_history`, `is_first_turn`, `model`, `platform`, `parent_session_id`, `sender_id` (turn_context.py lines 809-822). Delegated children run with `platform="subagent"` (per herdr-threads' bridge; the upstream doc names `parent_session_id`).

### 2.4 Other plugin context APIs (VERIFIED, `hermes_cli/plugins.py`)
- `register_system_prompt_section(id, content|callable, position="after_memory", max_chars<=4000)` (lines 967-992). The callable runs **once per new session**, and its bytes are frozen across compression and resume (hooks.md lines 401-443). It suits static guidance such as a skill pointer. It is useless for per-message delivery.
- `register_tool`, `register_command` (in-session `/name`), `register_cli_command`, `register_skill`, `dispatch_tool`, `call_mcp` (allowlisted per server), `llm` (host-owned completions), `subagent_lifecycle` (includes `steer(handle, text)` for subagents, `agent/subagent_lifecycle.py` line 378), and `emit` / `subscribe` (a namespaced in-process event bus).
- `register_middleware` (request rewrite and execution wrapping; `hermes_cli/middleware.py`, `website/docs/developer-guide/middleware.md`). Not examined in depth (unknown whether a request-kind middleware can append messages mid-loop).

---

## 3. Shell hooks (`config.yaml` `hooks:`)

VERIFIED, hooks.md lines 1847-2122.
- They cover the same `VALID_HOOKS` set and run as subprocesses with JSON on stdin (`hook_event_name`, `tool_name`, `tool_input`, `session_id`, `cwd`, `profile`, `extra{...}`). The subprocess runs with that profile's `HERMES_HOME`.
- stdout JSON: `{"context": ...}` for `pre_llm_call`; block, modify and approve for `pre_tool_call` (exit 2 also blocks); continue for `pre_verify`. Timeout defaults to 60 s, capped at 300 s. They fail open unless `fail_closed: true` (only `pre_tool_call`).
- Consent: a first-use prompt per `(event, command)`, persisted to `shell-hooks-allowlist.json`. Only a foreground `hermes chat` asks. The gateway, ACP and cron never prompt and need `--accept-hooks`, `HERMES_ACCEPT_HOOKS=1`, `hooks_auto_accept: true` or a manual allowlist entry (lines 2071-2100). Since sessions now run in the gateway, the CLI "asks on its terminal before it opens its gateway session" (line 2073).
- Docs state that `UserPromptSubmit` is deliberately not a separate event and that `pre_llm_call` is the equivalent (line 2053). There is no Stop-hook "block and continue" equivalent for ordinary turns. `pre_verify` covers it only for code-editing turns (INFERRED from the `pre_verify` contract).

---

## 4. Plugin-initiated turns: `ctx.inject_message` (a native wake primitive)

VERIFIED: `hermes_cli/plugins.py` lines 596-603, `hermes_cli/plugins_injection.py` lines 12-70, plugins.md "Injecting Messages" lines 1016-1103.

- Signature: `ctx.inject_message(content, role="user", *, session_key=None, origin=None) -> bool`. A non-`user` role is prefixed `[role] `.
- **In-process classic CLI** (`manager._cli_ref` set, pre-v0.22.1 shape): idle puts the text on `cli._pending_input`, which becomes the next turn. If `_agent_running`, the text goes to `cli._interrupt_queue`, which **interrupts** the running turn "the same as a user typing a new message and pressing Enter" (plugins_injection.py lines 20-23; plugins.md lines 1047-1052; the interrupt loop is `hermes_cli/cli_chat_turn_mixin.py` lines 404-425). The text is processed by `_tui_process_one_input`. In that path a leading `/` would be dispatched as a slash command (INFERRED from `hermes_cli/cli_tui_runtime_mixin.py` lines 88-120).
- **Gateway mode** (the current default for CLI, TUI and Desktop, section 1): it needs `plugins.entries.<plugin>.allow_gateway_injection: true` (plugins_injection.py lines 36-40). With `session_key`, the text enters the session's normal path. "Active sessions use the existing busy-session queue rather than starting a competing turn". Injected text "cannot invoke slash commands, approve tools, or resolve pending confirmation and clarification prompts". `True` only means the request was accepted for asynchronous dispatch (plugins.md lines 1062-1073). The session must belong to the plugin's own profile.
- **Keyless** gateway injection goes to the calling session when one is bound. Otherwise it goes to "the one local session of the plugin's profile that was started by the classic CLI (`hermes chat`) and has a viewer attached". If ambiguous it is refused. Caveat: "When that is a classic CLI session, the turn and its reply are in the session's transcript, but the attached classic CLI does not display them." Keyless injection "may be retired later" (plugins.md lines 1054-1060).
- **Ink TUI / Desktop**: pass the durable `session_key` (`ses_…`). "a busy session keeps the message for the next turn, an idle session starts one" (plugins.md line 1084).
- No public HTTP endpoint or CLI exposes this. The plugin must know the session key itself (line 1102). The session id reaches plugins as the `session_id` hook kwarg and reaches terminal commands as `HERMES_SESSION_ID` (INFERRED that the two match the `session_key` the injector expects for local sessions; the docs say local CLI/TUI keys are the `ses_…` stored id).
- Idle vs busy: the host decides, so the plugin needs no idle detection. The human's composer draft is not touched: the text goes to a queue, not the prompt_toolkit buffer (INFERRED from the code paths above).

`ctx.post_message(text)` (plugins.md lines 963-1014) is a notice for the human. It is not model context. In the classic CLI it is "queued as a notice line, printed at the end of the next turn".

---

## 5. Headless, server and RPC modes

### 5.1 Gateway JSON-RPC (the protocol the CLI, TUI and Desktop use)
VERIFIED `website/docs/developer-guide/programmatic-integration.md` lines 37-123, `website/docs/developer-guide/gateway-local-sessions.md`, `website/docs/developer-guide/gateway-classic-cli.md`.
- Methods include `prompt.submit` ("Send a user turn to a live session; busy sessions queue / steer / redirect instead of refusing", `tui_gateway/contracts/prompt_voice.py` lines 82-83), `session.steer` ("Inject text into the next tool result without interrupting the turn"), `session.redirect` (`tui_gateway/contracts/sessions.py` lines 639-642), `session.interrupt`, `session.create`, `session.resume`, `session.list`, `session.history`, `session.status` (rendered `/status` text), `session.info` (includes "lazy-agent status"), `command.dispatch`, `approval.respond`, and `subagent.steer`. Events streamed back: `message.delta`, `message.complete`, `tool.start`, `tool.complete`, ... Several viewers can attach to one session (programmatic-integration.md line 60).
- Auth: a same-user private control socket under the profile home mints a single-use ticket (`verb: "session-ticket"`). The client then dials `ws(s)://<origin>/api/ws` with subprotocol `hermes-gateway-ticket.<ticket>` (`hermes_cli/gateway_client.py` lines 15, 37-43, 225-258). An external same-user process could in principle use `prompt.submit` / `session.steer` against a pane's session id: an idle wake with **no keystrokes and no composer risk**, plus mid-turn steering (INFERRED; the protocol is Python-internal and not documented as a stable third-party API, though programmatic-integration.md invites "any external host").
- Busy handling: an ordinary `prompt.submit` during a turn is steered, redirected or queued per `display.busy_input_mode` (programmatic-integration.md line 80).
- Turns started by the gateway itself (background completion, async delegation) carry `display_kind` / `display_metadata` (line 105).

### 5.2 OpenAI-compatible API server
VERIFIED programmatic-integration.md lines 127-217: `POST /v1/runs`, `GET /v1/runs/{id}/events` (SSE), `POST /v1/runs/{id}/steer` ("appended to the live run and becomes visible to the agent after the next tool boundary"; an undelivered steer comes back as `pending_steer`), and `/stop`. Headers `X-Hermes-Session-Id` and `X-Hermes-Session-Key`. It is meant for frontends, not for a pane's existing TUI session (INFERRED).

### 5.3 ACP
VERIFIED `website/docs/developer-guide/acp-internals.md` lines 128-165: `hermes acp` is a stdio JSON-RPC server (new_session, prompt, cancel, fork). Sessions persist to `state.db`. It is host-driven: a wake means the host sends `session/prompt`.

### 5.4 One-shot and serve
`hermes chat -q '…' [-Q]`, `hermes -z '…'` (both through the gateway), `hermes serve` (headless backend), `hermes dashboard`, and `hermes gateway` (`website/docs/reference/cli-commands.md` lines 49-108). `--format stream-json` gives JSONL (gateway-classic-cli.md line 67). Exit codes: 0 completed, 1 failed, 2 unsupported option, 3 pending control, 130 detach (lines 39-44).

---

## 6. Session identity and resume (VERIFIED, gateway-classic-cli.md)

- The client prints the persisted `Session:` id to stderr. `hermes --resume ID`, `hermes -c [title]` and `--resume latest` resume. `/quit`, `/exit`, `/detach`, EOF and **closing the terminal only detach**: the gateway-owned session and any running execution stay alive (gateway-classic-cli.md lines 15-19; gateway-local-sessions.md lines 29-33).
- Logical session ids are stable across compression and reset. The physical transcript target rotates (gateway-local-sessions.md lines 18-27). `on_session_reset` reports `old_session_id` / `new_session_id` on the gateway path.
- `HERMES_SESSION_ID` is injected into every terminal command (base_session_env.py lines 12-31). Plugins get `session_id` in hook kwargs.
- A completion (background process or async delegation) that arrives for a CLI or TUI session with **no viewer attached does not start a turn**: it is written as a note for the next turn (`website/docs/user-guide/features/delegation.md` line 633). Wakes are therefore viewer-gated for these internal events (VERIFIED). Whether `inject_message` / `prompt.submit` are viewer-gated is unknown.

---

## 7. Background processes, watch patterns, loops (model-driven self-wake)

VERIFIED `tools/process_registry.py` lines 58-90, `tools/process_registry_notifications.py` lines 1-60, `tools/terminal_tool_background.py` lines 15-22 and 106-140, `hermes_cli/cli_tui_runtime_mixin.py` lines 72-81.
- `terminal(background=true, notify_on_complete=true)`: when the process exits, an `[IMPORTANT: ...]` notification with up to `COMPLETION_OUTPUT_CHARS = 2000` chars of output tail is injected. An idle session drains notifications in its idle tick (`_drain_process_notifications("cli-idle")`) and starts a turn.
- `watch_patterns`: a match mid-run fires a notification. Limits: one per 15 s per session, 3 strike windows disable it, and the lifetime cap is 8 hits, after which it falls back to `notify_on_complete`. The design intent is "ONLY for rare one-shot mid-process signals".
- `heartbeat`: periodic, at least 60 s apart.
- Pattern for a messaging tool (INFERRED): the model starts a blocking `<tool> wait` in the background with `notify_on_complete=true`. When a message arrives the process exits and Hermes wakes the idle session with the output. The cost is that the model must re-arm the wait after every wake. Children's processes are killed when a subagent finishes (delegation.md line 256).
- `/loop <interval> <prompt>` re-runs a prompt on an interval in the session. `/goal` is a standing goal with a judge, and it can park on a background process (`website/docs/user-guide/features/goals.md` lines 160-164). There are also cron jobs. Each fire is a full model turn.

---

## 8. MCP client

VERIFIED `tools/mcp_tool.py`, `tools/mcp_tool_health.py`, `tools/mcp_tool_transport.py`, `tools/mcp_tool_sampling.py`, `tools/mcp_tool_events.py`, `tools/mcp_tool_registration.py`. Docs `https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp`.
- MCP servers are owned by the gateway session that runs the turn (section 1). Supported transports: stdio, HTTP (OAuth and mTLS).
- Tools are registered as `mcp-<server>` toolsets. Resources and prompts are exposed through generated **tools** (`list_resources`, `read_resource`, `list_prompts`, `get_prompt`; `tools/mcp_tool_registration.py` lines 19-35). The model sees resource content only by calling them.
- `message_handler`: **only `tools/list_changed` triggers a refresh**. `prompts/list_changed` and `resources/list_changed` are logged and ignored (`tools/mcp_tool_health.py` lines 99-123). No `resources/updated` subscription handling was found (unknown, INFERRED absent).
- `notifications/message` (logging) goes to `agent.log`, capped at 2,000 chars, and **never to the model** (`mcp_tool_health.py` lines 73-97). The v0.22.3 notes mention "MCP progress notifications streamed as live tool progress" (display only, INFERRED).
- Sampling (`sampling/createMessage`, optional tool use) and elicitation (form mode goes through Hermes' approval UI; URL mode is declined) are supported (`tools/mcp_tool_sampling.py` lines 1-2, 88-181, 254-257).
- **MCP Events (experimental)**: `mcp_servers.<name>.events: true` adds `list_event_types` / `subscribe_event` / `unsubscribe_event` tools. Deliveries arrive as HTTPS **webhooks** to the gateway's webhook platform, and each event runs a prompt (`tools/mcp_tool_events.py` lines 1-11 and 86-89). Modes `poll` and `push` are not implemented. It needs a public `https://` callback (`_callback_base`). It targets gateway webhook sessions, not a pane's local session (INFERRED).
- So, no: Hermes does not surface server-pushed MCP notifications into the model's context. There is no channel-style push (VERIFIED for list_changed and logging; INFERRED for the rest).
- Reverse direction: the official **mcp-serve** plugin makes Hermes an MCP server that can "list and read your messaging conversations, wait for new messages and send messages" (`website/docs/user-guide/features/mcp.md` line 1085). Its long-poll tool details live in `https://github.com/NousResearch/hermes-official-plugins/tree/main/mcp-serve` (not read; unknown).

---

## 9. TUI input behaviour relevant to keystroke injection

Classic CLI (`hermes`, `hermes chat`, `--cli`) is prompt_toolkit (`HermesCLI`, `cli.py`, `hermes_cli/cli_tui_mixin.py`). The Ink TUI is `hermes --tui` (`ui-tui/`, `tui_gateway/`).
- **Busy input** (VERIFIED `hermes_cli/cli_init_mixin.py` lines 147-152, `hermes_cli/config_defaults.py` line 829, `hermes_cli/cli_tui_mixin.py` lines 1478-1535): `display.busy_input_mode` is `interrupt` by default, or `queue` / `steer`. With the default, Enter while the agent runs uses `agent.redirect()` when supported (it cancels the current model request and appends a correction) or the interrupt queue. **A keystroke wake that lands while the agent is busy redirects or interrupts the turn.** Tab still queues in the modern layout (`hermes_cli/cli_modern_layout.py` line 37). `/busy queue|steer|interrupt` switches modes at runtime (`website/docs/reference/slash-commands.md` line 96).
- `/queue <prompt>` queues for the next turn, with list, edit, rm, move and clear. `/steer <prompt>` appends to the last tool result after the next tool call, "no interrupt, no new user turn" (slash-commands.md lines 52-53; `agent/interrupt_control.py` lines 310-321).
- **Bracketed paste** (VERIFIED `hermes_cli/cli_tui_mixin.py` lines 1668-1708, 1939): it is handled. A paste of 5 or more lines (`paste_collapse_threshold`) or 2,000 or more chars (`paste_collapse_char_threshold`) collapses to `[Pasted text #N: L lines → <file>]`. It is expanded before submission (`hermes_cli/cli_stream_mixin.py` lines 212-216). Leaked paste wrappers and mouse reports are stripped. There is a fallback heuristic for terminals without bracketed paste (`_tui_on_text_changed`, more than 1 char per event or 4 or more newlines). A short single-line wake marker is unaffected (INFERRED).
- **Composer drafts**: Ctrl+S pushes the current draft onto a **stash stack** and clears the composer. On an empty composer, Ctrl+S pops it back, or opens a browse panel when several are stashed (`hermes_cli/cli_tui_mixin.py` lines 1760-1775, `hermes_cli/prompt_stash.py`). The v0.22.0 notes add "Ctrl+C that clears your draft instead of killing the turn". A keystroke driver could stash and restore a human's draft with Ctrl+S (INFERRED; no API, keyboard only). `/undo` puts the removed message back in the composer (gateway-classic-cli.md line 27).
- Opt-in vim keys (v0.22.0) change key semantics in the prompt (INFERRED risk for injected keystrokes).

---

## 10. Idle/busy observability

- Hooks: `pre_llm_call` (turn start) and `on_session_end` (turn end, every turn) bracket a turn. `on_human_input_request` / `_resolved` mark "blocked on a person". `agent_loop_stopped` marks interrupts. `on_stream_*` marks streaming (VERIFIED, section 2.3). All of these run in the gateway process.
- Terminal signals: with `display.bell_on_complete` / `bell_on_prompt` (both default false), the CLI emits BEL plus **OSC 9** (`ESC ] 9 ; Hermes: <context> BEL`) and, on Warp, **OSC 777** `warp://cli-agent` with JSON `{"event":"stop"|"permission_request","session_id",...}` (`hermes_cli/terminal_notify.py` lines 1-12 and 44-77; `hermes_cli/cli_init_mixin.py` lines 139-140; `hermes_cli/config_defaults.py` lines 850-851). A terminal multiplexer could read these as turn-end and needs-input edges (INFERRED; Herdr support unknown).
- Gateway RPC: `session.info` (lazy-agent status), `session.status` (rendered text), and event subscription (`message.complete`, ...). With `session.resume`, `inflight` describes the running turn (programmatic-integration.md lines 103-105).
- The placeholder text changes while busy (`cli.modern.placeholder_busy_<mode>`, `hermes_cli/cli_tui_mixin.py` line 794). The status bar shows the busy indicator (`/indicator`).

---

## 11. What herdr-threads does today for Hermes (brief)

VERIFIED from this repo unless marked.
- An owned Python plugin `integrations/hermes/__init__.py` plus `plugin.yaml` (`provides_hooks: pre_tool_call, pre_llm_call, post_tool_call, on_session_start, on_session_reset`; the code also registers `on_session_finalize`, `subagent_start` and `subagent_stop`, `HOOKS` line 32). Setup installs it under `<HERMES_HOME>/plugins/herdr-threads`. The user enables it with `hermes --profile NAME plugins enable herdr-threads`.
- Per callback it spawns `herdr-threads --state-dir … --host-endpoint … hook hermes` with `subprocess.Popen(..., close_fds=True)` and the **default (inherited) environment** (`__init__.py` lines 602-628). The Rust hook takes the pane from `HERDR_PANE_ID` (`src/cli/hook.rs` lines 5, 255, 1780).
- Context: only a top-level `pre_llm_call` is context-bearing. It returns `{"context": <composed check-in text>, "lifecycle_ack": …}`, with the context capped at 4,096 bytes and the output at 8,192 (`src/harness/hermes.rs` lines 1026-1069). `post_tool_call` is observer-only (allowlisted IDs, no context), so there is **no mid-turn delivery** today. A parented `pre_llm_call` (platform `subagent`) gets a fixed read-only restriction text locally.
- `pre_tool_call` is a person gate: it returns `approve` for `herdr-threads human|setup|unsetup|doctor fix|…` (`__init__.py` lines 708-754).
- Wake: no `DeliveryPolicy` override, so the native Herdr wake applies (an `IdleWake::Native` keystroke into an empty composer) with the default `MARKER` text `herdr-threads: attention pending; run herdr-threads inbox` (`src/harness/adapter.rs` lines 229-238 and 469-471; `src/notification/policy.rs` line 14). Hermes declares no composer provider and no in-turn poke (`src/harness/hermes.rs` lines 47-48). A seat is wakeable only through `herdr-threads launch --kind hermes`, which supplies a guarded process hint. A bare Hermes on macOS is not recognized by Herdr (`integrations/hermes/README.md`).
- Identity: `caller_env` uses process names `hermes`, `session_var: "HERMES_SESSION_ID"`, and `is_hermes_launcher` (python argv containing `hermes_cli` or `/.hermes/`) (`src/harness/hermes.rs` lines 805-829). Resume is not supported (a resumed session starts fresh on the pane's seat).
- Native evidence: three live runs on **Hermes 0.21.5**, before the v0.22.1 gateway shift (`integrations/hermes/README.md`).

---

## 12. Flags and open questions for the comparison reports

1. **Probable breakage on Hermes 0.22.1 and later** (INFERRED, high confidence from source): hooks and the model's terminal tools run in the shared gateway daemon (section 1). The bridge's hook child inherits the gateway's env, and `src/cli/hook.rs` reads `HERDR_PANE_ID` from its own env. Expected symptoms: `HERDR_PANE_ID missing` (quiet refusal, no context, no enrollment) or, worse, a pane id inherited from whichever pane first started the gateway, so all Hermes sessions bind to that pane's seat. The same applies to model-issued `herdr-threads` CLI calls (caller attribution by process tree and env). This needs a live check on v0.22.5 before anything else. Candidate fixes: have the operator add `HERDR_PANE_ID` to `terminal.client_env_passthrough` and have the bridge read it with `agent.session_client_env.tool_env()` / pass `toolchain_environ()` to `Popen` (INFERRED to work for terminal tools; for hook callbacks it depends on the client-env binding being active in the hook's context, which is unknown). Alternatively, key seats by `HERMES_SESSION_ID` plus a pane-side witness from the client process.
2. **Better wake path than keystrokes**: `ctx.inject_message(..., session_key=<ses_id>)` with `allow_gateway_injection: true` (from the plugin, which already runs in the gateway) or gateway RPC `prompt.submit` (from the daemon) wakes an idle session. The host queues it when busy, it needs no idle detection, and it cannot clobber a draft. Caveats: the classic CLI may not *display* a gateway-injected turn (plugins.md line 1060), and the injected text is a real user-role row in the transcript.
3. **Mid-turn delivery exists natively and is unused**: `transform_tool_result` (append pending items to a tool result) or gateway `session.steer` (the "next tool result" without interrupt) would serve a busy session in-turn (delivery contract rule 3/4). Today herdr-threads delivers only at `pre_llm_call`.
4. **Keystroke wake risk**: the default `busy_input_mode: interrupt` means a mistimed keystroke wake redirects or interrupts a running turn instead of queueing, unlike Claude's queue-on-busy.
5. **Token efficiency**: the wake text is the long `MARKER` ("…run herdr-threads inbox") rather than content-free `WAKE_UP`, although `pre_llm_call` already injects the check-in on the woken turn (AGENTS.md R18). Whether the composed Hermes context carries bodies or only a digest needs checking (INFERRED: a digest; `compose_context` in `src/cli/hook.rs` lines 602-633).
6. `register_system_prompt_section` could carry the static skill and trust explanation once per session (rule (a)) instead of per-turn text.
7. Unknown: whether `inject_message` or `prompt.submit` start a turn when no viewer is attached (internal completions do not, delegation.md line 633); how `session_key` maps to the CLI's printed `Session:` id; whether the gateway bootstrap scrubs inherited env; whether `gateway.standalone` or any flag restores in-process CLI turns.
