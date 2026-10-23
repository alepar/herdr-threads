# CAO (awslabs) x Hermes Agent: message injection and wake

- Alternative: CLI Agent Orchestrator, https://github.com/awslabs/cli-agent-orchestrator, depth-1 clone at `69909e98035905b612fd6b117abff8bf8876220f`. Paths prefixed `cao:` are relative to that clone root.
- Harness: Hermes Agent, https://github.com/NousResearch/hermes-agent, shallow clone at `f3b5b980d6158794beb3d1044fd7b8ca4b8f0b23` (2026-10-23, the source behind `primers/hermes.md`). Paths prefixed `hermes:` are relative to that clone root.
- Read with: `overview.md` (this directory) and `../../primers/hermes.md`.
- Tags: **VERIFIED** = read in source or docs at the cited line. **INFERRED** = reasoned from those, not observed. **unknown** = not found. Nothing was run; no Hermes install was available.

## TL;DR

CAO supports Hermes as a native provider, but the integration uses none of Hermes' own surfaces. It starts `hermes chat` as an interactive TUI in a tmux window or Herdr tab. Injection and wake are a single act: it bracket-pastes the message plus a sender suffix into the composer and presses Enter once, and only when screen regexes (tmux) or Herdr `agent_status` (herdr) say the pane is IDLE or COMPLETED. It uses no Hermes plugin, hook, `inject_message`, gateway RPC or `/queue`. Every message costs a full new user turn. The return path to the supervisor is either the worker calling `send_message` through a `cao-mcp-server` the operator added to the Hermes profile by hand, or (for `handoff`) CAO scraping the worker's last reply from the screen.

Two Hermes-specific weaknesses stand out:
1. Hermes has no swallowed-Enter recovery. CAO lists it in its own `NO_RECOVERY` set because it has no composer reader.
2. INFERRED, high confidence: on Hermes v0.22.1 and later, the documented `CAO_TERMINAL_ID: ${CAO_TERMINAL_ID}` MCP env entry resolves in the shared gateway's environment, not the pane's. Sender identity for `send_message` then breaks or points at the wrong terminal. herdr-threads' Hermes bridge has the same class of problem (primer section 12.1).

---

## 1. Support status

**Native, VERIFIED.**

| Fact | Evidence |
|---|---|
| `ProviderType.HERMES = "hermes"`, registered in the provider manager | `cao:src/cli_agent_orchestrator/models/provider.py:13`, `cao:src/cli_agent_orchestrator/providers/manager.py:16,49,159-168` |
| Provider class `HermesProvider(BaseProvider)`, 342 lines, regex-based TUI parsing | `cao:src/cli_agent_orchestrator/providers/hermes.py:107-342` |
| Launch command `<hermesProfile or "hermes"> chat --yolo --accept-hooks --source cao [--model M]` | `cao:providers/hermes.py:153-192`, `cao:docs/hermes.md:69-81` |
| `hermesProfile` is a Hermes profile **wrapper command** (from `hermes profile alias`), launched directly | `cao:docs/hermes.md:37-60`, `cao:src/cli_agent_orchestrator/models/agent_profile.py:109-112` |
| Backends: tmux (default) and herdr. Hermes is not excluded from herdr; the `_resolve_buffer` docstring (`base.py:657`) names Hermes' empty-buffer ERROR default as the gap the herdr live-buffer fallback closes | `cao:providers/base.py:649-695` |
| Tool restriction: none ("restrict inside the Hermes profile"); enforcement table `"hermes": NONE` | `cao:src/cli_agent_orchestrator/utils/enforcement.py:49`, `cao:docs/hermes.md:172-186` |
| Runtime skill prompt and allowed tools are ignored, each with a logged warning only | `cao:providers/hermes.py:180-190` |
| Agent-plugin MCP servers are not delivered to Hermes (`mcp.provider_unsupported`); plugin skills are | `cao:docs/hermes.md:89-94`, `cao:src/cli_agent_orchestrator/agent_plugins/mcp_mapping.py:127,181` |
| Hermes is "the only in-tree provider that reports `WAITING_USER_ANSWER`" with key-navigated clarify pickers | `cao:docs/hermes.md:145-157`, `cao:src/cli_agent_orchestrator/mcp_server/server.py:142-145,177-220` |

The provider docstring says it was written to be theme-agnostic: "CAO intentionally does not hard-code a concrete Hermes profile name" (`cao:providers/hermes.py:108-114`). The status regexes can be overridden through `CAO_HERMES_*` env vars (`cao:providers/hermes.py:20-49`, `cao:docs/hermes.md:126-143`).

Version targeting: no Hermes version is named anywhere in the provider or docs (VERIFIED by reading both in full). The glyphs it keys on do exist at Hermes `main`. `⏲` is the frozen (idle) prompt timer and `⏱` the live one (`hermes:hermes_cli/cli_status_bar_mixin.py:155-181`). `--accept-hooks`, `--yolo` and `--source` are real `chat` flags (`hermes:hermes_cli/_parser.py:292,307,317-318`). `--source` is a "Session source tag for filtering (default: cli). Use 'tool' for third-party integrations that should not appear in user session lists" (`hermes:hermes_cli/_parser.py:317-318`). CAO uses `cao`, so its sessions do appear in the user's session list (INFERRED).

The CAO agent profile's own body (its system prompt, e.g. "You are a helpful developer agent.", `cao:docs/hermes.md:26-35`) is **never passed to Hermes**. `HermesProvider` does not read `profile.system_prompt`/`profile.prompt`, unlike for example `cao:providers/omp.py:176` (VERIFIED by absence in `hermes.py`). Hermes is also not in `RUNTIME_SKILL_PROMPT_PROVIDERS` (`cao:src/cli_agent_orchestrator/services/terminal_service.py:475-483`). A Hermes worker therefore gets its role only from its Hermes profile and from whatever text arrives in the task paste (INFERRED).

---

## 2. Injection mechanism

### 2.1 Exact mechanism

The message is **typed into the Hermes composer as user input** (keystroke injection). CAO writes no Hermes hook, plugin, MCP resource, `ctx.inject_message`, gateway `prompt.submit`/`session.steer`, `/queue` or `/steer` (VERIFIED: grep over `cao:src/` for Hermes references, listed at `cao:` provider/manager/server/enforcement/mcp_mapping/agent_profile/launch/api; none touches a Hermes API). The model sees the message as an ordinary new user turn.

### 2.2 Code path (send to model)

1. **Sender** (any CAO agent, including a Hermes supervisor with the manual MCP entry) calls MCP `send_message(message, receiver_id=None)` (`cao:src/cli_agent_orchestrator/mcp_server/server.py:926-958`). If `receiver_id` is omitted, the message goes to the recorded caller. An explicit receiver must be the sender's child, its parent, or a discovered sibling (`server.py:943-947`).
2. `_send_message_impl` appends the sender suffix (`cao:src/cli_agent_orchestrator/utils/orchestration.py:2525-2539`) and calls `_send_to_inbox` (`orchestration.py:1391-1453`). That does an HTTP `POST {API_BASE_URL}/terminals/{receiver}/inbox/messages` with `sender_id` and `message` as **query parameters** (`orchestration.py:1424,1440-1445`). The sender identity is `CAO_TERMINAL_ID` from the MCP subprocess's environment. If that variable is unset, the call raises `"CAO_TERMINAL_ID not set - cannot determine sender"` (`orchestration.py:1417-1419`).
3. **API** `create_inbox_message_endpoint` persists a PENDING row, then starts `inbox_service.deliver_pending` on a worker thread. It answers after the paste reaches the pane or after 5 s (`INBOX_IMMEDIATE_DELIVERY_WAIT`), whichever comes first (`cao:src/cli_agent_orchestrator/api/main.py:8373-8447`, `cao:src/cli_agent_orchestrator/constants.py:393`).
4. **`InboxService._deliver_pending_locked`** (`cao:src/cli_agent_orchestrator/services/inbox_service.py:206-379`), under a per-terminal lock:
   - It reads up to `num_messages` PENDING rows (default 1; `0` means drain up to 100, line 213).
   - Gate: status must be IDLE or COMPLETED. The eager mid-turn path needs `accepts_input_while_processing`, which Hermes does **not** override; the base default is `False` (`cao:providers/base.py:376-387`). So for Hermes the gate is strictly IDLE/COMPLETED, whatever `CAO_EAGER_INBOX_DELIVERY` says (`inbox_service.py:216-247`).
   - It marks the rows DELIVERED **before** sending (`inbox_service.py:249-257`).
   - It joins contiguous same-sender rows with `\n` and calls `terminal_service.dispatch_input(..., orchestration_type=SEND_MESSAGE)` (`inbox_service.py:259-295`).
5. **`dispatch_input`** (`cao:services/terminal_service.py:4527-4752`):
   - It refuses if the status is ERROR. For Hermes it also refuses when the status is WAITING_USER_ANSWER, because `blocks_orchestrated_input_while_waiting_user_answer` is True: "Hermes approval and clarify pickers consume pasted text as answers" (`cao:providers/hermes.py:148-151`; guard `terminal_service.py:181-207`).
   - It prepends the `<cao-memory>` block to the terminal's very first message only (`inject_memory_context`, `terminal_service.py:387-465`).
   - It re-checks that the pane has not fallen back to its pre-launch shell (`_refuse_if_provider_gone`, `terminal_service.py:4492-4524`).
   - Then it calls `backend.send_keys(..., enter_count=provider.paste_enter_count, force_bracketed_paste=True, submit_delay=provider.paste_submit_delay)`. For Hermes: `paste_enter_count = 1` ("Hermes submits bracketed paste with a single Enter", `cao:providers/hermes.py:143-146`). `paste_submit_delay` keeps the base default of 0.3 s (`cao:providers/base.py:349-358`).
6. **Backend keystrokes**:
   - **tmux** (`cao:src/cli_agent_orchestrator/clients/tmux.py:1695-1860`):
     1. `send-keys -X cancel` (leaves copy mode).
     2. `load-buffer`, then `paste-buffer`. On tmux < 3.7 the buffer is wrapped in hand-made `ESC[200~ … ESC[201~` and pasted with `-r`. On tmux >= 3.7 the raw bytes are pasted with `-p`, and tmux emits real markers only if the pane enabled DECSET 2004.
     3. Sleep 0.3 s, `-X cancel` again, `send-keys Enter`.
     Shell panes get no wrapping.
   - **herdr** (`cao:src/cli_agent_orchestrator/backends/herdr_backend.py:654-705`): `herdr pane send-text <pane> "\x1b[200~…\x1b[201~"`, a fixed **2.0 s** sleep, then `herdr pane send-keys <pane> Enter`.

### 2.3 Payload format and framing (VERIFIED)

Body as sent, then a blank line, then a suffix (`cao:utils/orchestration.py:125-138,212-214`):

```text
<body>

[Message from terminal 1a2b3c4d. Use send_message MCP tool for any follow-up work.]
```

`assign` uses `[Assigned by terminal X. When done, send results back to terminal X using send_message]` (101 bytes with an 8-hex id). The message suffix is 83 bytes. The suffix is on by default (`CAO_ENABLE_SENDER_ID_INJECTION`, `orchestration.py:128`). It is suppressed only for kiro-cli `@prompt` invocations, never for Hermes (`orchestration.py:148,163-207`). There is no header, no message id, no thread name and no priority marker. A drained batch is several bodies joined by `\n`, each with its own suffix, and nothing separates them (INFERRED from `inbox_service.py:265-266` plus the suffix being applied per send).

How Hermes renders this paste (INFERRED from the primer, section 9):
- prompt_toolkit's bracketed-paste handler collapses any paste of 5 or more lines or 2,000 or more chars into `[Pasted text #N: L lines → <file>]` in the composer. It expands the paste before submission, so the model still gets the full text (`hermes:hermes_cli/config_defaults.py:2506-2508`; primer section 9).
- Because the suffix adds two lines, a body of 3 or more lines always shows as a collapsed placeholder in the pane. That is harmless for the model, but it changes what a screen observer sees.
- On tmux >= 3.7 against a pane without DECSET 2004 the paste is unbracketed. Hermes then falls back to its "more than 1 char per event / 4 or more newlines" heuristic (`_tui_on_text_changed`, primer section 9). Whether multi-line CAO messages then submit per line depends on that heuristic: unknown.

### 2.4 When it fires

Only at a turn boundary: the receiver must already be idle (IDLE/COMPLETED). There is no mid-turn delivery for Hermes. The eager path is off for Hermes, and CAO does not use Hermes' native mid-turn channels (`transform_tool_result`, `/steer`, gateway `session.steer`).

### 2.5 Size bounds

- No body limit on the inbox POST: `message: str` is a plain query parameter with no `max_length` (`cao:api/main.py:8373-8380`). Other endpoints in the same file do use `max_length=262_144` (`api/main.py:1294,1334,1347`), so the omission here stands out.
- The practical bound is the HTTP request-line limit of the ASGI server, because the body travels in the **URL query string**. The value is unknown for the CAO deployment (INFERRED: uvicorn/h11 limits are in the tens of KiB).
- Hermes adds no truncation to typed input. Collapse-to-file is display only (INFERRED, primer section 9).

### 2.6 Dedupe and acknowledgement

- Dedupe is purely sender-side storage state: rows go DELIVERED before the paste (`inbox_service.py:249-257`), plus a per-terminal lock (`inbox_service.py:68-87`). A known duplicate-wakeup race remains across delivery paths, tracked in GH #115 (`cao:docs/inbox-delivery.md:128`).
- There is **no receiver ACK**. `after_dispatch` "is never an acknowledgement that the receiver accepted the message" (`inbox_service.py:180-186`).
- For Hermes there is also no submit confirmation with recovery (section 3.5). DELIVERED therefore means only that the bytes were written into the pane.

---

## 3. Wake mechanism

### 3.1 Exact mechanism

Wake and injection are the same keystroke act: paste plus one Enter into an idle composer submits a turn (`cao:docs/inbox-delivery.md:10-19`). Hermes' native wake primitives go unused (VERIFIED by absence): `ctx.inject_message` (primer section 4), gateway `prompt.submit` (primer section 5.1), and background-process `notify_on_complete` (primer section 7).

### 3.2 Triggers that start a delivery attempt (all reach `deliver_pending`)

| Trigger | Hermes-specific notes | Evidence |
|---|---|---|
| Immediate attempt on POST | Delivers at once if the receiver is already IDLE/COMPLETED | `cao:api/main.py:8396-8433` |
| StatusMonitor events `terminal.*.status` (tmux) | Fires on the IDLE/COMPLETED edge computed by `HermesProvider.get_status` | `cao:inbox_service.py:90-158` |
| Watchdog log poll (tmux), 5 s | Fast precheck uses `get_idle_pattern_for_log()` = `⏲` (env `CAO_HERMES_IDLE_LOG_REGEX`) | `cao:providers/hermes.py:24,267-269`, `cao:docs/inbox-delivery.md:13,77-79`, `cao:constants.py:331` |
| `HerdrInboxService` on `pane.agent_status_changed` → `idle`/`done`, plus a settled-pane sweep on (re)connect | Works only if Herdr recognises the Hermes process as an agent. Otherwise `agent_status` stays `unknown` and no edge events come (INFERRED) | `cao:docs/herdr.md:97-99`, `cao:src/cli_agent_orchestrator/services/herdr_inbox_service.py:919-957` |
| Reconcile sweep, every 30 s, for rows PENDING > 30 s | Provider-agnostic backstop | `cao:docs/inbox-delivery.md:111-124`, `cao:constants.py:345,351` |

### 3.3 Idle/busy detection for Hermes

**tmux backend: screen regexes over the rolling pipe-pane buffer** (`HermesProvider.get_status`, `cao:providers/hermes.py:214-265`). The last 30 lines are taken and the last 8 non-blank lines are tested, in this order:

1. `WAITING_USER_ANSWER` if an approval or clarify picker is in the bottom lines. Patterns cover `y/n`, `[o]nce…[s]ession…[d]eny`, `Choice [o/s/a/D]:`, `Hermes needs your input`, `↑/↓ select … Enter confirm`, including Chinese variants (`hermes.py:30-39,84-104,248-249`).
2. `ERROR` on `Error:|ERROR:|Traceback|hermes … failed:` (`hermes.py:40,251-252`).
3. `PROCESSING` on `msg=interrupt|musing...|Ctrl+C cancel|⏱ \d+s` (`hermes.py:29,254-255`). `⏱` is Hermes' live timer (`hermes:hermes_cli/cli_status_bar_mixin.py:181`).
4. IDLE/COMPLETED if the idle timer `⏲ <dur>` is **unchanged across at least 2 consecutive polls** (`_has_stable_idle_timer`, `hermes.py:271-285`), or if a prompt line matches `^(?!…chrome…).{0,80}(?:❯|✦)\s*$` (`hermes.py:20-23`). It is COMPLETED when both a user line (`^●\s+`) and an assistant header or extractable response exist (`hermes.py:243-260`).
5. Otherwise `PROCESSING` (`hermes.py:262-265`). **Unknown screens default to busy**, which is the safe direction for a keystroke injector.

A quirk worth noticing: `_has_stable_idle_timer` returns True only while `2 <= stable_count <= MAX_STABLE_IDLE_TIMER_POLLS` (default 8, env `CAO_HERMES_MAX_STABLE_IDLE_POLLS`) (`hermes.py:49,285`). After 8 identical readings the timer stops counting as idle evidence, and status then depends on the `❯|✦` prompt regex. On a theme whose prompt does not match, an idle Hermes falls back to PROCESSING once 8 identical polls have passed (INFERRED). The intent is not documented (unknown). It may guard against a frozen timer left in scrollback.

The PROCESSING and approval strings are English UI text from Hermes' locale files: `msg=interrupt` lives in `hermes:locales/en.yaml` (VERIFIED by grep). Under another Hermes locale (Hermes is i18n'd; `t("cli.shared.duration_s")`, `hermes:hermes_cli/cli_status_bar_mixin.py:170-179`), the processing hints and duration strings change (INFERRED). Only the approval patterns carry Chinese alternatives.

**herdr backend: Herdr native `agent_status`** (`cao:backends/herdr_backend.py:866-912`): working → PROCESSING, blocked → WAITING_USER_ANSWER, done → COMPLETED, idle → IDLE (`unknown` → None). `BaseProvider._resolve_native_status` adds a 10 s flush wait before it reports a post-dispatch done/idle as COMPLETED; past 300 s since dispatch the idle path still reports COMPLETED, only with a warning log (`cao:providers/base.py:541-647`). When Herdr reports `unknown`, status falls back to the same Hermes regexes over `herdr` live pane history (`base.py:649-695`). Whether Herdr's agent detection recognises a CAO-launched `hermes chat` is not stated in CAO. herdr-threads' own notes say Herdr does not recognise a bare Hermes on macOS (`integrations/hermes/README.md:44-45` in this repo). On herdr, Hermes wake would then come only from the immediate attempt and the 30 s reconcile sweep, because no edge events arrive (INFERRED).

### 3.4 Draft safety

- **No check for human-typed composer text.** Before the paste CAO only leaves tmux copy mode and refuses pickers or a dead provider (`cao:clients/tmux.py:1767-1780`; `terminal_service.py:4492-4524,4602-4610`).
- `HermesProvider` does not implement `extract_current_composer` (base default `None`, `cao:providers/base.py:340-347`), so CAO cannot even see a draft.
- An operator who is mid-typing in a CAO Hermes pane would have the message pasted into their draft and submitted with it (INFERRED). CAO assumes machine-owned panes.
- Hermes offers a keyboard-only escape hatch CAO does not use: Ctrl+S stashes the draft (primer section 9).
- Picker safety is good: pastes and the bare-Enter recovery are both blocked while WAITING_USER_ANSWER (`terminal_service.py:181-207`, `cao:docs/inbox-delivery.md:35`). Hermes is the provider this matters most for, because its clarify pickers would eat the paste. `--yolo` removes most approval prompts (`hermes.py:167`).

### 3.5 Submit confirmation and recovery

- On tmux, after the paste CAO waits up to 8 s for a status earned after the dispatch boundary (`inbox_service.py:381-411`, `constants.py:378`).
- For Hermes, recovery is **disabled by design**. Hermes is in `TestInboxRecoveryCoverageByProvider.NO_RECOVERY` (`cao:test/services/test_inbox_service.py:1964-1992`), because the bare-Enter and full-re-send recoveries both need a composer reader (`inbox_service.py:422-452`).
- A swallowed Enter is only logged. Hermes has no direct status probe (`supports_direct_status_probe` stays at the base `False`, `cao:providers/base.py:258`) and no composer reader, so the recovery helper sends nothing and logs "composer cannot be read; skipping full re-send" (`cao:services/terminal_service.py:3559-3569`); `_confirm_submitted` then returns at `inbox_service.py:493-497`, before the "may be sitting unsubmitted" error at lines 504-510 is reached. The next delivery then pastes on top of it (`cao:docs/inbox-delivery.md:23`).
- On the herdr backend, confirmation is skipped entirely (`inbox_service.py:301-319`).

### 3.6 Latency

| Case | Latency |
|---|---|
| Already-idle receiver, tmux | Immediate attempt: DB write + status read + 0.3 s submit delay. Sub-second (INFERRED) |
| Already-idle receiver, herdr | Fixed 2.0 s bracketed-paste wait before Enter (`herdr_backend.py:695-699`) |
| Busy receiver | Next IDLE edge. On tmux that edge needs `⏲` stable across 2 polls (status events are output-driven, so roughly one redraw interval of the Hermes status bar, unknown exactly), or the 5 s watchdog |
| Missed edges | Up to ~60 s through the reconcile sweep (30 s interval + 30 s grace) |
| Handoff result | The worker must reach COMPLETED. On herdr that adds the 10 s flush wait (`base.py:600-616`) |

### 3.7 Failure modes (Hermes-relevant)

1. **Status lag → keystrokes into a running turn.** Hermes' default `display.busy_input_mode` is `interrupt` (`hermes:hermes_cli/config_defaults.py:829`). A message pasted while Hermes is actually busy, but CAO still reads IDLE from a stale frame, **interrupts or redirects the turn** instead of queueing (INFERRED, primer section 9 and flag 4). CAO's Claude path relies on Claude buffering input; Hermes has the opposite default.
2. **Swallowed Enter** goes undetected and is never recovered (section 3.5), so the next message piles on top.
3. **Theme or locale drift** breaks the regexes. Overrides are env-only and server-wide (`hermes.py:20-49`).
4. **Herdr does not recognise Hermes** (INFERRED): no edge events, slow sweep-only wake.
5. **Leading `/` in a body**: the classic CLI would dispatch it as a slash command (primer section 4, INFERRED for typed input). CAO does not escape bodies.

---

## 4. Setup and installation footprint on the harness

- **CAO writes nothing into Hermes config.** "CAO does not rewrite Hermes `config.yaml` or inject `mcpServers` automatically" and "does not mutate Hermes profile files or create a temporary overlay config" (`cao:docs/hermes.md:11-13,85-87,184-186`). The stated reason is to keep "Hermes session history attached to the user's chosen profile" (`docs/hermes.md:185-186`).
- **Manual step** for a Hermes agent that needs to *send* (supervisor, or a worker replying through `send_message`): add the following to the Hermes profile's `config.yaml` (`cao:docs/hermes.md:96-124`):

  ```yaml
  mcp_servers:
    cao-mcp-server:
      enabled: true
      command: cao-mcp-server
      env:
        CAO_TERMINAL_ID: ${CAO_TERMINAL_ID}
  ```

  A pure worker that is only ever handed work and whose result is screen-scraped needs no Hermes config at all (INFERRED from `handoff` scraping, `overview.md` "Injection mechanism" item 7).
- **Env** in the pane: `CAO_TERMINAL_ID` (and `CAO_SESSION_NAME`) set by tmux window/pane env or herdr `--env` (`cao:clients/tmux.py:1252,1416,1598`; `cao:backends/herdr_backend.py:10,1094,1120`).
- **Launch flags** carry consent: `--yolo` (no approval prompts) and `--accept-hooks`, which auto-accepts any shell hooks already configured in that profile. CAO adds no hooks itself (VERIFIED). The flag silently approves whatever the user's profile has (INFERRED side effect).
- Optional `hermesProfile` wrapper via `hermes profile alias <name>` (`cao:docs/hermes.md:14-20`).

---

## 5. Token cost

**Per message (receiver):**
- The body.
- The suffix: 83 bytes for messages (about 20 tokens), 101 for assign.
- A `<cao-memory>` block on the terminal's first input only, if memory is enabled (`terminal_service.py:387-465`).
- Most important, **one full Hermes turn per delivery**: a new user message means a full model request over the whole history, plus a reply. Delivery is one message per IDLE cycle by default (`inbox_service.py:206-214`), so N queued messages cost N turns.
- A Hermes supervisor that wants the reply must also make its own `send_message` tool call, which costs another turn on the other side.

**Per quiet turn:**
- **Zero injected bytes.** No hook runs and nothing is added to context when no message is pending (VERIFIED: no Hermes hook or plugin is installed).
- The only standing cost is the `cao-mcp-server` tool schemas on every request when the manual MCP entry is present. `server.py` declares 37 `@mcp.tool` sites (handoff, assign, send_message, memory_*, workflow_*, list_siblings, …; `grep -c "@mcp.tool" cao:src/cli_agent_orchestrator/mcp_server/server.py`). Whether all of them are exposed to a Hermes agent depends on CAO-side gating that was not read: unknown. Hermes has no CAO-native tool restriction (`docs/hermes.md:172-178`).

---

## 6. Limitations and bugs visible in source

1. **Gateway-era identity breakage (INFERRED, high confidence).** Since Hermes v0.22.1 every session's MCP servers are owned by the one gateway process (primer section 1; `hermes:hermes_cli/cli_agent_setup_mixin.py:649,660`: "No MCP discovery here: the gateway session that runs the turn owns its MCP servers"). Hermes renders `${VAR}` in `mcp_servers.*.env` through `agent.secret_scope.get_secret`, which reads the active profile's secret scope or the gateway's own `os.environ`, not the launching pane's environment (`hermes:tools/mcp_tool_config.py:554-581`, `hermes:agent/secret_scope.py:230-260`). An unset whole-value reference is dropped from the child env (`hermes:tools/mcp_tool_config.py:218-225`). Consequences:
   - (a) The gateway does not have `CAO_TERMINAL_ID`: `cao-mcp-server` starts without it, and `send_message` fails with "CAO_TERMINAL_ID not set" (`cao:utils/orchestration.py:1417-1419`).
   - (b) The gateway was first started from a CAO pane and inherited that pane's id: every Hermes session's `cao-mcp-server` claims that one terminal, so replies and `assign` callbacks route to the wrong caller, and the parent/child authorisation in `send_message` evaluates against the wrong node.
   - CAO's doc never mentions the gateway, `terminal.client_env_passthrough` or `gateway.standalone` (VERIFIED by reading `docs/hermes.md` in full). Passthrough would likely not help here: the MCP env path (`hermes:tools/mcp_tool_config.py`) does not read the session `client_env` at all (VERIFIED by grep for `client_env`/`session_client_env` over `tools/mcp_tool*.py`).
   - This is the same failure class herdr-threads faces with `HERDR_PANE_ID` (primer section 12.1).
2. **No swallowed-Enter recovery for Hermes**, by CAO's own enumeration (section 3.5).
3. **Keystroke delivery against `busy_input_mode: interrupt`** (section 3.7.1). CAO does not set `--busy`/`display.busy_input_mode: queue` at launch, though Hermes allows `/busy queue` at runtime (primer section 9).
4. **Profile system prompt dropped** for Hermes, and runtime skill prompt and tool restrictions are ignored with warnings only (section 1). A worker relies entirely on the Hermes profile plus the pasted task.
5. **MCP tool-call timeout mismatch (INFERRED).** `handoff` blocks the supervisor's MCP call for up to `timeout=600` s by default (`cao:mcp_server/server.py:339-344`). Hermes' default MCP tool-call timeout is 300 s (`hermes:tools/mcp_tool_common.py:41`). A Hermes supervisor's default handoff would be cut off at 5 minutes unless the profile entry sets `timeout`. The documented YAML does not (`cao:docs/hermes.md:100-107`).
6. **Unbounded body in a URL query parameter** (section 2.5).
7. **No receiver ACK, no priorities, no threads**: the inbox has six columns (`overview.md`, "Delivery guarantees").
8. **Result scraping** for `handoff` depends on the assistant-header and user-prefix regexes (`hermes.py:41-45,293-334`). Hermes' paste collapse means the echoed user line may be a `[Pasted text #N …]` placeholder rather than the message (INFERRED). That still matches `^●\s+`, so extraction anchors survive, but any check that compares displayed input to the sent text would not.
9. **Stable-timer cap of 8 polls** can flip an idle Hermes back to PROCESSING on non-default themes (section 3.3).
10. `--source cao` keeps CAO sessions in the user's Hermes session lists; Hermes' own guidance for integrations is `--source tool` (`hermes:hermes_cli/_parser.py:317-318`).

---

## 7. Clever ideas worth stealing

1. **Provider-declared "picker eats input" capability** (`blocks_orchestrated_input_while_waiting_user_answer`, `cao:providers/hermes.py:148-151`) is enforced in both the paste path and the bare-Enter path (`terminal_service.py:181-207`). Our Native wake marker for Hermes should equally refuse while Hermes shows an approval or clarify picker (`on_human_input_request` is a clean signal on the hook side, primer section 2.3).
2. **Causal submit confirmation**: a status counts as pickup only when its output-generation stamp is newer than the dispatch boundary (`inbox_service.py:381-411`). That avoids reading the previous turn's COMPLETED as success. It also refuses a full re-send unless the composer is provably empty (`inbox_service.py:431-445`). This applies directly to verifying a Native wake keystroke.
3. **Unknown screen → PROCESSING default** (`hermes.py:265`): when the detector cannot tell, it never injects.
4. **A "provider gone, pane back at shell" guard** before typing (`terminal_service.py:4459-4524`). It compares `pane_current_command` against the pre-launch shell, so a wake marker is never executed as a shell command.
5. **Env-overridable status regexes** (`CAO_HERMES_*`). This is a cheap, version-tolerant escape hatch that fits our "degrade only the broken capability" rule.
6. **The negative example is the most useful lesson.** CAO shows what Hermes integration looks like when it ignores Hermes' native surfaces: regex fragility, no recovery, interrupt risk, a full turn per message. The primer's alternatives are strictly better for us: `ctx.inject_message(session_key=…)` or gateway `prompt.submit` for an idle wake with no keystrokes or draft risk, host-side busy queueing, and `transform_tool_result` or `session.steer` for mid-turn delivery (primer flags 2-3).

---

## 8. Comparison notes for herdr-threads (INFERRED)

| Aspect | CAO x Hermes | herdr-threads x Hermes (primer section 11) |
|---|---|---|
| Injection | The message itself, pasted as a user turn | `pre_llm_call` context (≤ 4 KiB) at turn start; no mid-turn delivery |
| Wake | Same paste + Enter | Herdr Native wake marker (long `MARKER` text) into an empty composer |
| Busy handling | Waits for IDLE (regex/agent_status); no eager path | Wake only when idle; in-turn delivery only at the next `pre_llm_call` |
| Draft safety | None | Native wake requires an empty composer |
| Cost per message | One full turn each | Silent unless priority ≤ T; batched into the next turn's context |
| Harness config | None written; manual MCP entry | Owned plugin + `plugins enable` |
| Gateway (v0.22.1+) identity | `${CAO_TERMINAL_ID}` resolves in the gateway env: likely broken | `HERDR_PANE_ID` read from the gateway env: likely broken |

Both tools still assume a per-pane process environment that Hermes no longer gives hook or MCP children. Keying on `HERMES_SESSION_ID` (injected per terminal command) plus a pane-side witness, as the primer suggests, would fix both.

---

## Verification

Adversarial pass, 2026-10-23, against the same CAO clone (`69909e98`) and Hermes clone (`f3b5b980`). Nothing was executed from either clone.

Checked and confirmed: provider registration and enum; `HermesProvider` launch command, flags, warnings, `paste_enter_count`, `blocks_orchestrated_input_while_waiting_user_answer`; all status regexes, their order in `get_status`, the PROCESSING default and the 2..8 stable-timer window; `enforcement.py`, `mcp_mapping.py`, `answer_user_prompt` Hermes picker path; `docs/hermes.md` quotes and line ranges; `send_message` routing and `_send_to_inbox` (query-parameter POST, `CAO_TERMINAL_ID` error); suffix templates (83 and 101 bytes measured with an 8-hex id from `uuid4().hex[:8]`) and the kiro-only suppression; inbox endpoint (no `max_length`, 5 s wait) versus the `262_144` limits elsewhere; `_deliver_pending_locked` gate, eager path (`accepts_input_while_processing` false for Hermes, `CAO_EAGER_INBOX_DELIVERY` default false), mark-before-send, same-sender join, herdr confirmation skip; `NO_RECOVERY` contains `hermes`; tmux and herdr `send_keys` (cancel, `-r`/`-p` by tmux version, 0.3 s delay, herdr 2.0 s sleep); herdr native status mapping; reconcile constants; settled-pane sweep; `handoff` 600 s default vs Hermes `_DEFAULT_TOOL_TIMEOUT = 300`; Hermes glyphs, `--source` help text, paste-collapse thresholds, `busy_input_mode: interrupt`, `msg=interrupt` in `locales/en.yaml`; `get_secret` and the unset-whole-ref drop in `_build_safe_env`.

Changed:
- Section 1: `base.py:657` reworded; it names Hermes' empty-buffer ERROR default, not a list of target providers.
- Section 3.3: the herdr 300 s path does not "give up" differently; it reports COMPLETED like the 10 s path, only with a warning log.
- Section 3.4: tmux copy-mode cancel citation corrected to `tmux.py:1767-1780`.
- Section 3.5: for Hermes the recovery helper returns early (no probe, no composer reader), so the logged line is the "composer cannot be read" warning, not the "may be sitting unsubmitted" error; citations added.
- Section 6.1: Hermes citation extended to line 660 (the MCP-ownership comment); added the VERIFIED note that the MCP env path does not consult `client_env`.
