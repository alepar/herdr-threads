# mcp_agent_mail x Hermes Agent (Nous Research)

- Alternative: https://github.com/Dicklesworthstone/mcp_agent_mail (Python), clone at `83ed8d7c97af67f1e6b4700bb8ace08739171ed3`. Paths without a prefix are relative to that clone root.
- Rust successor: https://github.com/Dicklesworthstone/mcp_agent_mail_rust, README only (raw `main`, fetched 2026-10-23). These paths are prefixed `rust:README.md`.
- Harness: https://github.com/NousResearch/hermes-agent, clone at `f3b5b980d6158794beb3d1044fd7b8ca4b8f0b23`. These paths are prefixed `hermes:`. Background is in `../../primers/hermes.md`.
- Nothing from either clone was executed. I got the tool-description sizes by parsing `app.py` with Python's `ast` module, without importing or running it.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from what was read. **unknown** means not found.

## Support status

**Not supported.** Neither repo has a Hermes integration, an install script or any mention of Hermes.

- `grep -rniI -e hermes -e nous` over the whole Python clone matches only English words such as "asynchronous" and "synchronous". There is no hit for Hermes or Nous Research. VERIFIED.
- The `scripts/` directory has integration scripts only for Claude Code, Cline, Codex CLI, Cursor, Factory Droid, Gemini CLI, GitHub Copilot, OpenCode and Windsurf. VERIFIED (`ls scripts/`).
- The auto-detect installer probes `~/.claude`, `~/.codex`, `~/.cursor`, `~/.factory`, `~/.gemini` and `opencode`. It never probes `~/.hermes` or a `hermes` binary. VERIFIED: `scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh:21`, `:126-173`.
- `scripts/hooks/check_inbox.sh:2` names its targets: "Claude Code / Codex CLI / Gemini CLI / Factory Droid". VERIFIED.
- The Rust README lists "Claude Code, Codex CLI, Gemini CLI, Oh My Pi (OMP), GitHub Copilot CLI, and any MCP-compatible client" (`rust:README.md:17`). Grepping that README for `hermes|nous` finds nothing. VERIFIED.

What remains is **generic MCP-client use**. Hermes is an MCP client: it supports stdio and HTTP servers configured under `mcp_servers:` in `config.yaml` (`hermes:website/docs/user-guide/features/mcp.md:32-40`, `:336-340`). VERIFIED. The rest of this report covers that generic path: what it delivers on Hermes, what it cannot do, and what an integration would have to build.

## Injection mechanism

### What exists generically: pull through MCP tool calls

- **Channel.** The model sees a message only when it calls an MCP tool itself. The tools are `fetch_inbox`, `fetch_topic`, `macro_start_session` (which returns an inbox snapshot), `search_messages` or `summarize_thread`, or Hermes' generated `read_resource` tool on `resource://inbox/{agent}...`. Hermes exposes MCP resources only through generated tools (`hermes:tools/mcp_tool_registration.py:19-35`, primer section 8). VERIFIED.
- **Server code path.** `fetch_inbox` is at `src/mcp_agent_mail/app.py:9446-9556`. It authenticates (`_authenticate_agent`, `app.py:5357`), lists rows through `_list_inbox` (`app.py:4592`), and orders them `ORDER BY created_ts DESC` with `limit` (`app.py:4628-4629`). If notifications are enabled, it also clears the agent's signal file (`app.py:9548-9550`). It does **not** mark messages read: "Retrieve recent messages for an agent without mutating read/ack state" (`app.py:9467`). VERIFIED.
- **Payload.** A JSON list of dicts: `{id, project_id, sender_id, thread_id, reply_to, topic, subject, importance, ack_required, created_ts, attachments, [body_md]}` (`_message_to_dict`, `app.py:1843-1861`). The docstring says `from` and `read_at` are also present (`app.py:9493-9496`). `format='toon'` switches to the TOON encoding, applied in the tool wrapper (`_apply_tool_output_format`, `app.py:497-504`; advertised in the server instructions, `app.py:5221-5226`). VERIFIED.
- **Size bounds.**
  - `limit` defaults to 20 and is capped at 1000 (`app.py:9509-9512`). VERIFIED.
  - Bodies are opt-in (`include_bodies=False`). When they are included, `body_md` is returned whole. `_message_to_dict` does no truncation (`app.py:1858-1859`). VERIFIED.
  - The only bound on the Hermes side is a 2,000,000-char hard cap on an MCP text result (`hermes:tools/mcp_tool_content.py:18-34`). Above about 50K a separate budget/spillover layer takes over (comment at `:18-19`; that layer was not read). VERIFIED for the cap; the spillover behaviour is unknown.
- **Framing.** Raw JSON in a tool result. Nothing marks messages as peer input or says "do not obey".
  - Hermes scans MCP tool *descriptions* for injection phrases (`hermes:tools/mcp_tool_schema.py:20-32`). VERIFIED.
  - It does not pattern-scan tool *results*: the description scan is called only at registration (`hermes:tools/mcp_tool_registration.py:297`), and result rendering only strips Unicode tag characters and applies the size cap (`hermes:tools/mcp_tool_content.py:10,32-34,314`). VERIFIED by absence in those files at `f3b5b980`.
  - Overseer messages carry a fixed preamble inside the body: "🚨 MESSAGE FROM HUMAN OVERSEER 🚨 ... Temporarily pause your current work" (`README.md:457-479`). VERIFIED.
- **Dedupe and ACK.**
  - `unread_only=True` skips rows marked read through `mark_message_read` or `acknowledge_message` (`app.py:9475-9480`). VERIFIED.
  - `acknowledge_message` sets `read_ts` and `ack_ts` and is idempotent (`app.py:9784-9800`). VERIFIED.
  - Nothing tracks what has been offered to the model's context. Two `fetch_inbox` calls return the same rows again unless the model marks them read or passes `since_ts`. The model has to do all of this bookkeeping itself. INFERRED from the read-only contract.
- **Server instructions.** FastMCP is built with a one-sentence `instructions` string (`app.py:5221-5228`). VERIFIED.
  - I found no code in Hermes' MCP client that reads `InitializeResult.instructions` into the prompt. A grep of `hermes:tools/` for `instructions` hits only the catalog/connector paths and event-prompt code (`hermes:tools/connectors/mcp.py:531`, `hermes:tools/mcp_tool_events.py:44-89`). INFERRED: the instructions are unused for ordinary configured servers.
  - The "register, check inbox, ACK" protocol therefore reaches a Hermes model only if the operator pastes the README blurb (`README.md:153-188`) into AGENTS.md, a skill or a system-prompt section.
- **Server-side MCP log notifications.** The server emits `ctx.info(...)` on 57 call sites, for example "Fetched N messages" (`app.py:9551-9554`) and "Message N created" (`app.py:5851`). Hermes routes `notifications/message` to `agent.log` only, capped at 2,000 chars, and never to the model (`hermes:tools/mcp_tool_health.py:73-97`). VERIFIED for both sides. These logs are no help for injection.

### What does not carry over: the hook reminder

For Claude and Codex, mcp_agent_mail's only push into context is `scripts/hooks/check_inbox.sh` running as a PostToolUse(Bash) hook. On a Hermes shell hook it would deliver **nothing**, for three separate reasons:

1. **Wrong event semantics.** Hermes `post_tool_call` is an observer: "After blocked, error, or successful result; return ignored" (`hermes:website/docs/user-guide/features/hooks.md:452`). VERIFIED. The only shell-hook event that carries context is `pre_llm_call` (`hooks.md:1872`). VERIFIED.
2. **Wrong output shape.**
   - In JSON mode, `check_inbox.sh` prints `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":...}}` (`check_inbox.sh:167-172`).
   - Hermes' parser accepts only a top-level `"context"` string for non-tool events: `context = data.get("context")` (`hermes:agent/shell_hooks.py:664-666`, dispatched at `:672-682`). The envelope parses as a dict, has no `context` key, and is dropped.
   - In text mode, the emoji banner (`check_inbox.sh:174-182`) is not JSON and is dropped with a warning (`shell_hooks.py:678-680`).
   - VERIFIED both shapes.
3. **Environment.**
   - The script reads its identity from `AGENT_MAIL_PROJECT`, `AGENT_MAIL_AGENT`, `AGENT_MAIL_TOKEN` and `AGENT_MAIL_REGISTRATION_TOKEN` (`check_inbox.sh:42-48`), and exits silently if they are missing (`:51-54`). VERIFIED.
   - Hermes shell hooks run in the gateway with `build_subprocess_env(scrub_secrets=is_multiplex_active())` (`hermes:agent/shell_hooks.py:497-507`). VERIFIED.
   - Variables set in the pane's shell therefore do not reach the hook (primer section 1). Under multiplexing, secret-looking names such as `*_TOKEN` are likely scrubbed. INFERRED; I did not read the scrub list.
   - Values inlined in the `command:` string (`env AGENT_MAIL_AGENT=... check_inbox.sh`; it runs via `shlex.split`, `shell=False`, `hooks.md:1882`) would work. They would also put the tokens in `config.yaml`. INFERRED.

A small adapter would make the reminder work. It would run the same curl as a **`pre_llm_call`** shell hook and print `{"context": "You have N unread message(s)..."}`. That injects once per turn, appended to the user message (primer section 2.2). INFERRED: nothing like it exists upstream. It would still be count-only and turn-start-only. Hermes' mid-turn channel, `transform_tool_result`, exists only for Python plugins; the shell-hook table lists context injection only for `pre_llm_call` (`hooks.md:1872`). VERIFIED.

## Wake mechanism

**None, on any harness, in the OSS code.** Hermes is no exception.

- The server never types into panes, never calls a harness API, and sends no MCP push that a client would act on. VERIFIED by absence (overview "Wake mechanism"; grep of `src/` for `tmux`, `send-keys`, `wake`, `notifications/`).
- Wake is left to the "historical zsh loop from the tweet thread" tmux broadcast and to the closed-source commercial Companion's "Message Stacks" (`README.md:50`, `:331-333`). How those work is unknown.

**The one hook point: signal files.**

- With `NOTIFICATIONS_ENABLED=true` (default false, `config.py:515-518`), `send_message` writes `{signals_dir}/projects/<slug>/agents/<agent>.signal` for to and cc recipients (not bcc) (`app.py:5835-5849`). The file holds `{timestamp, project, agent, message:{id, from, subject, importance}}`. A per-(project, agent) debounce of 100 ms applies (`storage.py:3623-3686`). `fetch_inbox` deletes the file (`storage.py:3689-3720`, `app.py:9548-9550`). VERIFIED.
- Nothing in the repo consumes it. VERIFIED (overview).

**Hermes' native wake surfaces versus mcp_agent_mail:**

| Hermes surface | Can mcp_agent_mail drive it today? | Notes |
|---|---|---|
| MCP server push into context | No | Hermes acts only on `tools/list_changed`. Resource `list_changed` is ignored, and no `resources/updated` handling was found (`hermes:tools/mcp_tool_health.py:99-123`). The Python server sends no such push anyway. VERIFIED / INFERRED |
| MCP Events (experimental, `mcp_servers.<n>.events: true`) | No | Hermes needs the server to implement the experimental `events/*` extension; deliveries arrive as webhooks to the gateway at an `https://` `public_url` (`hermes:tools/mcp_tool_events.py:1-11`, `:67-73`, `:345-346`). Each event starts a separate unattended agent run, not a turn in the existing session (`:420-431`). VERIFIED. mcp_agent_mail implements no such extension: INFERRED from absence |
| `ctx.inject_message(..., session_key)` (plugin) | No plugin exists | A Hermes plugin that watches the `.signal` file (inotify) and calls `inject_message(session_key=ses_…)` with `allow_gateway_injection: true` would be a real wake. The host queues it when busy, and no draft is at risk (primer section 4). INFERRED design; unbuilt |
| Gateway RPC `prompt.submit` | No | Same idea from an external daemon (primer section 5.1). Unbuilt |
| `terminal(background=true, notify_on_complete=true)` | Partly, with the model's cooperation | The model could start `inotifywait -e create,modify <signal path>` (or a curl poll loop) as a background process. When a message arrives the process exits, and Hermes wakes the idle session with an `[IMPORTANT: ...]` note and up to 2,000 chars of output tail (primer section 7). The note is viewer-gated: no viewer attached means no turn (primer section 6). The model must re-arm the watcher after every wake. Nothing in mcp_agent_mail suggests this. INFERRED |
| `/loop <interval> <prompt>` or cron | Yes, crudely | "check fetch_inbox" every N minutes. Every fire is a full model turn even when the inbox is empty. INFERRED |
| Keystrokes (tmux) | Only through the external zsh loop or the Companion | With Hermes' default `display.busy_input_mode: interrupt`, a blind broadcast into a busy pane **interrupts or redirects the running turn** (primer section 9). A blind broadcast also lands on top of any composer draft. The tweet-thread loop has no idle check that I could see, but it is not in the repo, so that is unknown |

- **Idle/busy detection:** none. VERIFIED by absence.
- **Draft safety:** none, because nothing types. With the external tmux loop the risk depends on that script, which is unknown.
- **Latency:** unbounded for an idle Hermes agent: it sees mail only when something else starts a turn and the model chooses to check. During an active turn, latency depends on when the model polls. There is no hook-driven 120 s reminder on Hermes as there is on Claude and Codex. INFERRED.
- **Failure modes:**
  - The agent stays idle forever with unread urgent mail.
  - Importance (`urgent`) has no wake effect anywhere (overview "Delivery guarantees").
  - The ACK-TTL scanner only logs `ack-warning` server-side (`config.py:207-215`, `http.py:1195-1251`). Neither the sender nor the recipient is told.

## Setup and installation footprint on the harness

There is nothing upstream for Hermes. A manual generic setup would be:

- **Run the server** (`serve-http`, `127.0.0.1:8765`, path `/api/`, `config.py:396`), or use `serve-stdio`. VERIFIED.
- **One `mcp_servers:` entry in `~/.hermes/config.yaml`** (or the profile's `$HERMES_HOME/config.yaml`). For example, `url: "http://127.0.0.1:8765/api/"` plus `headers: {Authorization: "Bearer <token>"}`, following the documented `url` and `headers` shape (`hermes:website/docs/user-guide/features/mcp.md:336-340`). Then `/reload-mcp` or restart the gateway. VERIFIED that the shape exists. Whether a plain `http://` loopback URL is accepted without TLS was not checked (unknown).
  - Because the gateway owns MCP servers for every session (primer section 1), this one entry serves **every** Hermes session of the profile. That is good for reach. Identity, though, is purely whatever `agent_name` and `registration_token` the model passes in each call. INFERRED.
- **Optional tool trimming.** On the server side, `TOOLS_FILTER_ENABLED=true` with `TOOLS_FILTER_PROFILE=minimal|core|messaging|custom` (`config.py:132-154`, `:494-511`). On the Hermes side, `mcp_servers.<name>.tools.include` (`mcp.md:180-183`). VERIFIED.
- **Instructions.** The README blurb pasted into the project AGENTS.md, a Hermes skill, or a `register_system_prompt_section` plugin. INFERRED; upstream ships only the blurb and `SKILL.md`.
- **No hooks, no plugin, no `~/.hermes/plugins/` content.** Upstream writes nothing into Hermes config at all. VERIFIED by absence.

## Token cost

- **Tool schemas, paid on every model request** (all INFERRED, standard MCP-in-tools-array behaviour):
  - The Python server registers **49 tools** and 29 resources (`grep -c '@mcp.tool('` / `'@mcp.resource('` in `app.py`).
  - Their docstrings, which FastMCP uses as descriptions, total **34,628 chars**. The largest are `send_message` (5,993), `create_agent_identity` (2,723), `ensure_project` (2,677), `register_agent` (2,674) and `reply_message` (2,270); `fetch_inbox` is 1,759. VERIFIED by AST parse (cleaned docstrings; raw docstrings total 41,337).
  - With parameter schemas, the unfiltered tool surface is roughly 9-11k tokens per request, mitigated only by provider prompt caching.
  - `TOOLS_FILTER_PROFILE=minimal` claims up to ~70% less (`config.py:132`). VERIFIED claim, not measured.
  - Hermes does not shorten MCP descriptions: `_convert_mcp_schema` passes the description through `strip_unicode_tags` only (`hermes:tools/mcp_tool_schema.py:186-193`). VERIFIED. Whether a Hermes tool-search/deferral mode keeps them out of some requests was not checked (`toolsets.py`, `model_tools.py` mention it).
- **Per quiet turn:** zero injected bytes, because no hook exists. The schema overhead above is still paid. With the hypothetical `pre_llm_call` curl adapter, a quiet turn is also zero bytes (silent when there is no mail, `check_inbox.sh:160`). INFERRED.
- **Per message, at minimum:**
  - One model tool call to `fetch_inbox`. That is about 250-350 bytes of JSON metadata per row without bodies, or the full `body_md` with them. INFERRED from `_message_to_dict`.
  - Usually a second round trip with `include_bodies=true` or `summarize_thread`.
  - One `acknowledge_message` or `mark_message_read` call per message. There is no batch ACK among the Python MCP tools; the Python server has only an HTTP route `POST /mail/{project}/inbox/{agent}/mark-all-read` (`src/mcp_agent_mail/http.py:3180-3181`), which the model does not see as a tool. VERIFIED. The Rust README lists a `mark_all_read` tool (`rust:README.md:774`).
  - Each call also pays argument tokens for `project_key`, `agent_name` and `registration_token`. INFERRED.
- **Overseer messages** add the ~400-byte preamble per message (`README.md:462-477`). VERIFIED text, size INFERRED.

## Limitations and bugs visible for Hermes

1. **No idle wake at all.** Hermes' best-in-class native wake APIs (`inject_message`, `prompt.submit`) go unused because no plugin exists. INFERRED.
2. **No in-turn push.**
   - The Claude/Codex reminder script cannot be reused on Hermes as-is: wrong event, wrong JSON shape, env not inherited (section "What does not carry over"). VERIFIED reasons.
   - Even ported to `pre_llm_call`, it would be count-only, turn-start-only, and rate-limited by a `/tmp/mcp-mail-check-<agent>` file keyed on the agent name (`check_inbox.sh:66-80`).
   - In the one-gateway model, that file lives in the gateway's `/tmp`. INFERRED to be harmless but shared by every session of that agent name.
3. **Identity drift.**
   - On Hermes, identity is whatever the model remembers to pass in each call. After compression, `/new` or resume, the model may lose its `agent_name` or `registration_token`. They live only in the transcript. INFERRED.
   - The Python server's pane identity (`$TMUX_PANE`-keyed files, overview "Identity") cannot help either. Hermes tool and hook subprocesses run in the gateway, not the pane (primer section 1), so `$TMUX_PANE` would be the gateway's, or absent. INFERRED.
   - The Rust server's `MESSAGING_SESSION_IDENTITY` keys on the MCP session (`rust:README.md:612-656`). Since the gateway owns the MCP connection per session, a gateway-shared connection could merge identities across Hermes sessions. Hermes keeps one MCP connection per server per process, keyed by the bare server name (or `(profile scope, name)` under a profile multiplexer), not per Hermes session (`hermes:tools/mcp_tool.py:453-460`, `hermes:tools/mcp_tool_scope.py:1-14`, at `f3b5b980`; VERIFIED). So every Hermes session in one gateway process shares one MCP session, and the Rust session identity would merge them (INFERRED).
4. **The registration token is model-visible** in every call's arguments and in the transcript. VERIFIED that the argument exists (`app.py:9464`). The Rust session binding exists to fix exactly this (`rust:README.md:614-617`).
5. **Newest-first ordering with a limit.** A backlog of more than 20 messages hides the oldest unless the model pages. Importance is not a sort key. VERIFIED (`app.py:4628-4629`).
6. **Busy-pane keystroke hazard** if users fall back to the README's tmux broadcast loop. Hermes' default `busy_input_mode: interrupt` turns a mistimed broadcast into an interrupt or redirect of the running turn (primer section 9). INFERRED.
7. **License.** The MIT+Rider restricts OpenAI, Anthropic and parties acting for them (`LICENSE:1`, rider at `:12-62`). Study ideas only; copy no code (overview).

## Clever ideas worth stealing

1. **A body-free signal file as a wake hook point** (`storage.py:3616-3686`). This is the right primitive for Hermes' background-process wake: a tiny file the server touches and the reader clears, with a 100 ms debounce. For herdr-threads on Hermes, an equivalent "attention changed" file per seat could drive either a plugin `inject_message` watcher or a model-armed `notify_on_complete` watcher. Neither needs keystrokes. INFERRED.
2. **The read path never mutates state; read and ACK are separate verbs** (`app.py:9467`, `9475-9480`). This is the opposite of our "inbox ACKs on display". It shows the cost: the model does the bookkeeping and pays a round trip per ACK. That supports our choice. INFERRED.
3. **`unread_only` as the reminder's dedupe key** (GH #274, `check_inbox.sh:90-99`). It stops the reminder from nagging about mail already handled. Our `Unchanged` attention-version check is the stronger version of the same lesson. INFERRED.
4. **Server-side and client-side tool-surface trimming** (`TOOLS_FILTER_PROFILE`, Hermes `tools.include`). For Hermes, a minimal messaging profile keeps the schema tax per request small. That is relevant if we ever expose herdr-threads over MCP. INFERRED.
5. **Rust durable delivery cursors** (`am inbox-events`, `fetch_inbox_events`: oldest-first, body-free, cursor advanced only after processing, `CURSOR_EXPIRED` and `CURSOR_AHEAD` recovery; `rust:README.md:742-761`, `:1560`). This is a clean contract for an external monitor, such as a Hermes plugin that wakes on new events. The README does not say whether the CLI blocks or long-polls (unknown).

### Direct implications for herdr-threads on Hermes (INFERRED)

- mcp_agent_mail proves the gap rather than filling it. On Hermes, a pure MCP integration gets neither push nor wake.
- The gateway-era primitives this alternative ignores are where we should go: `ctx.inject_message(session_key=…)` for idle wake, and `transform_tool_result` for mid-turn delivery (primer section 12, items 2-3).
- The trap that breaks their shell hook also threatens our bridge: the gateway env, not the pane env (primer section 12, item 1).

## Verification

Adversarial pass on 2026-10-23 against mcp_agent_mail `83ed8d7c`, hermes-agent `f3b5b980`, the Rust README (raw `main`, re-fetched) and `primers/hermes.md`. Nothing from either clone was executed; docstring sizes were recomputed with an AST parse from a script outside the clone.

- Checked and confirmed: support status (no Hermes/Nous hit, `scripts/` list, installer probes `:21`, `:126-173`, `check_inbox.sh:2`, `rust:README.md:17`); `fetch_inbox` signature, read-only docstring, limit default 20 and 1000 cap, `_list_inbox` ordering, signal clear; `_message_to_dict`; `acknowledge_message` idempotency; signal file emit/clear, to/cc only, 100 ms debounce, `NOTIFICATIONS_ENABLED` default false; `check_inbox.sh` env, rate file, `unread_only`, JSON and text output lines; config citations (`HTTP_PATH`, tool filter, ack TTL) and `http.py` ack-warning log; 49 tools / 29 resources; overseer preamble; Hermes `mcp.md` config shapes, `hooks.md:452`, `:1872`, `:1882`, `shell_hooks.py` env and `context` parser, MCP 2M-char cap, description scanner, log-notification routing and `list_changed` handling, generated resource tools; Rust README lines 17, 612-656, 742-761, 774, 1560; primer section references.
- Changed: `ctx.info` call-site count 64 -> 57; TOON citation now points at the wrapper that applies it; the fifth-largest docstring is `reply_message`, not `fetch_inbox`; README wake citation `:45-52` -> `:50`, `:331-333`; LICENSE rider range; added the Python HTTP-only mark-all-read route; MCP Events row now states the HTTPS `public_url` requirement and that each event starts a separate unattended run (VERIFIED).
- Not checked (left as stated): whether Hermes scans MCP tool results, the secret-scrub list, plain-`http://` acceptance for `url:` servers, per-session vs per-gateway MCP connections.
