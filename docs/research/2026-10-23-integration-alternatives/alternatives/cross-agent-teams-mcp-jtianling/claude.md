# cross-agent-teams-mcp (jtianling) × Claude Code

- Upstream: https://github.com/jtianling/cross-agent-teams-mcp, clone at `17df5cf7bd9b66f00460eff05080b0e0495b6ec8`.
- Paths are relative to the clone root. The code was read and never run. No Claude session was driven.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means my reasoning, not stated or observed. **unknown** means not found.
- Harness facts come from `../../primers/claude.md` (the primer), cited as "primer §N".

## 1. Support status

**Native support, two MCP servers per Claude session** (VERIFIED, `README.md:111-145`, `mcpsmgr.json` `agents.claude-code`):

1. `cross-agent-teams`: an MCP `http` server pointing at the daemon (`http://127.0.0.1:9100/mcp`). It carries every agent-facing tool (`register_agent`, `send_message`, `get_inbox`, `reconnect`, `bind_channel`, ...).
2. `cross-agent-teams-channel`: a stdio MCP server, the channel proxy (`plugins/cross-agent-teams-channel/`), launched with `npx -y -p cross-agent-teams-mcp cross-agent-teams-channel --daemon-url ...`. It declares the `claude/channel` capability and relays daemon wakes as `notifications/claude/channel`.

The channel only works when Claude is launched with `claude --dangerously-load-development-channels server:cross-agent-teams-channel` (VERIFIED, `README.md:139-145`; `README.agent.md:294-295` defines the aliases `xats-claude` and `free-xats-claude`). Without the flag, Claude has tools but no push wake. In that case the daemon may fall back to a tmux paste. It does so only when a tmux pane is bound and either no channel subscriber is attached or the sink threw synchronously (see §2.2 step 5 and §3.3).

Claude Code is the project's best-supported harness: "Claude Code gets push wake out of the box" (VERIFIED, `README.md:60`). The only Claude-specific code paths are `dispatchClaude` (`src/mcp/transport-dispatch.ts:157-203`), the channel auto-bind (`src/mcp/auto-bind-channel.ts`) and the proxy package.

## 2. Injection mechanism

### 2.1 What reaches the model

The model never receives message bodies by push. Only a **wake hint** is pushed. Bodies enter context solely as the **result of the `get_inbox` MCP tool**, which the model must choose to call (VERIFIED, `src/mcp/tools.ts:196`: "Auto-poke injects only a SHORT wake-up hint ... NOT the body — read bodies via get_inbox").

So there are two injection surfaces:

| Surface | Claude API used | Content | Tag |
|---|---|---|---|
| Wake hint | MCP channel notification `notifications/claude/channel` → Claude renders `<channel source="cross-agent-teams-channel">…</channel>` (primer §4) | `新邮件 from <sender>@<team> (<agent_id>) → <name>@<team>, 请调 get_inbox 查看` ("new mail from …, please call get_inbox to view") | VERIFIED source; the rendering is from the primer |
| Body | MCP tool result of `get_inbox` (JSON text) | `{messages:[{id,event_id,from_team,to_team,from_agent_id,from_name,from_device,from_role,to_agent_id,to_role,subject,body,need_reply,sent_at}], has_more, last_event_id}` | VERIFIED `src/mcp/get-inbox.ts:4-25,47-77` |
| Startup hint | the same channel notification, `meta:{source:'cross_agent_teams_mcp', kind:'startup_bind_hint'}` | about 3 KB of registration instructions (see §2.4) | VERIFIED `plugins/cross-agent-teams-channel/src/cli.ts:72-126` |

The design uses no hooks: no `SessionStart`, `UserPromptSubmit`, `PostToolUse` or `Stop`, and no mod (INFERRED from the file inventory: the repo ships no `hooks.json`, no settings writer and no plugin manifest beyond `plugins/cross-agent-teams-channel/plugin.json`, which only names `dist/cli.js`).

### 2.2 Code path, sender to Claude context

1. A sender calls `send_message`. The mailbox row is written first. `fanoutAutoPoke` then runs for each recipient (VERIFIED `src/mcp/auto-poke-fanout.ts:80-127`; mailbox-first is stated in `src/mcp/tools.ts:178`).
2. `createAutoPokeImpl` builds the hint with `buildAutoPokeHint` and calls `poke(...)` with `allowCrossTeam: true` (VERIFIED `src/mcp/tools.ts:399-424`, hint at `:367-393`).
3. `poke` → `dispatchToTarget` → `dispatchPoke` with `meta: {}` (VERIFIED `src/mcp/poke.ts:271-293,341-389`; note `meta: {}` at `:388`).
4. `dispatchPoke` routes `agent_type === 'claude-code'` to `dispatchClaude` (VERIFIED `src/mcp/transport-dispatch.ts:106-107`).
5. `dispatchClaude` tries the channel first, when `delivery.kind === 'claude-channel'` and the csid has a subscriber; otherwise it falls back to tmux (VERIFIED `transport-dispatch.ts:162-184`):

   ```ts
   if (target.delivery.kind === 'claude-channel' && channelSubscribed && deps.channelWakeFanout) {
     const result = sendChannelWake(deps.channelWakeFanout, target.delivery.channel_session_id, input)
     if (result.ok) return { ok: true, transport_used: 'claude-channel', ... }
     sinkFailed = result.reason === 'sink_failed'
   }
   if (paneId) return dispatchTmux(deps, target, paneId, input.content, input.skipGuard)
   ```

6. `sendChannelWake` drops meta keys that do not match `^[A-Za-z0-9_]+$`, matching Claude's identifier rule for meta keys (primer §4). It then pushes the JSON-RPC notification `notifications/channel_wake` into the proxy's MCP session (VERIFIED `src/daemon/channel-wake-send.ts:3-37`).
7. The sink registered by `subscribe_channel_wake` calls `transport.send(payload)` on the proxy's Streamable HTTP server transport, so the payload travels over the proxy's SSE GET stream (VERIFIED `src/mcp/tools.ts:3050-3071`).
8. The proxy receives it in `client.fallbackNotificationHandler`, filtered to `method === 'notifications/channel_wake'` (VERIFIED `plugins/cross-agent-teams-channel/src/daemon-client.ts:76-82`).
9. `relayChannelWake` re-emits it to Claude over stdio and swallows every error (VERIFIED `plugins/cross-agent-teams-channel/src/proxy.ts:15-27`):

   ```ts
   const notif = { method: 'notifications/claude/channel', params }
   const p = server.server.notification(notif)
   if (p && typeof p.catch === 'function') p.catch(() => { /* host closed — drop silently */ })
   ```

10. Claude puts `<channel source="cross-agent-teams-channel">新邮件 …</channel>` into context (primer §4). The model is expected to call `get_inbox`, which returns bodies and advances the server-side cursor in the same transaction (VERIFIED `src/mcp/get-inbox.ts:39-77`).

### 2.3 When it fires

- It fires **on every send, for every recipient, unconditionally**. There is no priority, threshold or idle filter: "Delivery is NOT filtered by online/idle" (VERIFIED `src/mcp/tools.ts:197`). `auto_poke:false` is the only opt-out, and the sender chooses it (VERIFIED `tools.ts:182`).
- On the Claude side, timing is the channel's: when Claude is busy, "events queue into the session", and when several arrive "they're delivered together on the next turn" (primer §4). That a channel event starts a turn on an idle session is INFERRED in the primer and is the premise of this project's "push wake" claim (`README.md:60`).
- Bodies enter context only when the model calls `get_inbox`, which is model-paced. No turn-start or post-tool delivery exists. A busy Claude that never calls `get_inbox` sees its queued hint at the next turn and has to spend a tool call to read.

### 2.4 Payload format and size bounds

- Hint: Chinese-language template, at most 200 characters (`HINT_MAX_CHARS`). Segments are shed in order (the target segment first, then the sender display name, falling back to an 8-char agent_id prefix) (VERIFIED `src/mcp/tools.ts:367-393`). A typical hint is about 100 to 130 characters (INFERRED: names plus a 36-char UUID).
- No agent-facing `poke` tool exists at this revision: the 18 registered tools (`register_agent`, `send_message`, `send_message_by_id`, `broadcast`, `broadcast_to_role`, `get_inbox`, `get_delivery_status`, `reconnect`, `bind_channel`, `subscribe_channel_wake`, `echo`, ...) include none named `poke` (VERIFIED by grepping `registerTool(` over `src/`). The internal `poke()` caps its prompt at `PROMPT_MAX_BYTES = 8192` bytes, rejects self-targets, and is same-team unless `allowCrossTeam` (VERIFIED `src/mcp/poke.ts:102,271-293`). Its only callers are auto-poke (`src/mcp/tools.ts:413`, `allowCrossTeam: true`) and the unread watchdog (`src/mcp/unread-watchdog.ts:264`), so in practice the channel only carries the ≤200-char hint, the watchdog alert and the startup hint.
- Meta: auto-pokes send `meta: {}` (VERIFIED `poke.ts:370,381,388`), so the `<channel>` tag carries only Claude's own `source` attribute. The startup hint sends `meta.source` and `meta.kind`. Whether a `source` meta key collides with Claude's own `source` attribute is unknown.
- `get_inbox`: default `limit` 50, cap 200 rows (VERIFIED `get-inbox.ts:38`). There is **no byte bound** on `body` (schemas are `z.string().min(1)` with no `max`, VERIFIED `src/mcp/tools.ts:2835,2865,2913`). A large page therefore hits Claude's MCP output limits: a warning at 10k tokens, a 25k default cap, and spill-to-file above 50k chars (primer §5). The response is JSON with every metadata field repeated per message.
- Startup hint: about 3.1 KB without an identity key and about 3.7 KB with one (INFERRED: measured over the source lines `cli.ts:90,101,109,111-112,119-120` with `wc -c`). It is emitted on **every successful proxy (re)registration**, not once per session, because `onSequenceComplete` runs inside the reconnect loop (VERIFIED `cli.ts:227-233`, `daemon-client.ts:192-205`). A daemon restart or SSE drop therefore re-injects about 3 KB into each Claude session's context (INFERRED).

### 2.5 Framing

- Framing comes from Claude's channel wrapper (`<channel source=…>`) plus the free-text Chinese hint. The hint carries sender, sender agent_id, recipient and an instruction. It carries no message id, no subject and no priority (VERIFIED `tools.ts:367-393`).
- The `→ recipient@team` segment exists so that a pane receiving a misrouted hint can tell it was not addressed to it (VERIFIED `tools.ts:196`). This is a defensive answer to the identity-binding bugs below.
- Bodies arrive as raw JSON in a tool result, with no trust framing. The only trust statement is in the server `instructions` (about 3,070 chars / 3.7 KB UTF-8, mixed Chinese and English with roughly 330 CJK characters, under Claude's 4,096-char instructions cut, primer §4) and in tool descriptions (VERIFIED `src/mcp/transport.ts:236-259`; size measured over the string literals).

### 2.6 De-duplication and acknowledgement

- **De-dup of hints:** none. N sends produce N channel events. Claude batches events that queue while busy (primer §4), but each one is still a separate `<channel>` block (INFERRED).
- **De-dup of bodies:** via the cursor. A default `get_inbox` returns only rows past `last_processed_event_id` and advances it atomically, so two concurrent calls cannot both see the same tail (VERIFIED `get-inbox.ts:39-46,69-75`).
- **ACK = display:** "read" means the recipient's `get_inbox` returned the row. The sender can block up to `await_ack_s` (default 10 s, max 30 s) inside `send_message` for this receipt (VERIFIED `tools.ts:166-178`). `get_delivery_status` separates `wake_status` (the transport accepted) from `read` (the cursor passed) (VERIFIED `tools.ts:2950-2960`).
- **Unread watchdog:** a `need_reply` message still unread after 15 minutes triggers a daemon poke **to the sender** (VERIFIED `src/mcp/unread-watchdog.ts:6`, `tools.ts:177`). For a Claude sender, this alert also travels as a channel event.
- Claude itself never acknowledges channel notifications (primer §4). The `wake_status: delivered` the daemon records for a Claude channel wake therefore only means "written to the proxy's SSE stream" (VERIFIED `transport-dispatch.ts:174-179`).

## 3. Wake mechanism

### 3.1 Exact mechanism

The Claude wake is the channel event itself. The daemon pushes `notifications/channel_wake` to the proxy over the proxy's MCP SSE stream. The proxy re-emits it as `notifications/claude/channel` on stdio, and Claude, launched with the dev-channel flag, starts a turn when idle (the idle wake is INFERRED, primer §4). No keystrokes, no `asyncRewake`, no mod and no native `SendMessage` socket are involved (VERIFIED by the code path in §2.2; no other Claude path exists in `src/`).

### 3.2 Channel subscription and seat binding

1. Proxy start: it mints a fresh `csid = randomUUID()` per process, with no persistence (VERIFIED `cli.ts:212-213`).
2. Proxy registration: `register_agent` as `agent_type:'custom'`, `role:'__channel_proxy__'`, `name: channel-proxy-<pid>`, `team:'default'`, `delivery:{kind:'claude-channel', channel_session_id: csid}`, and `claude_ui_pid: findClaudeUiPid()`. It then calls `subscribe_channel_wake({channel_session_id})`, which only `__channel_proxy__` rows may call (VERIFIED `daemon-client.ts:87-126`, `src/mcp/subscribe-channel-wake.ts:20-30`).
3. `findClaudeUiPid` walks up to 8 parents with `ps -o ppid=,args=` until `basename(argv[0]) === 'claude'`. On a miss it returns the proxy's own parent pid (VERIFIED `plugins/cross-agent-teams-channel/src/find-claude-pid.ts:27-46`).
4. The model registers itself on its own HTTP MCP session: `register_agent({agent_type:"claude-code", name, team, ui_pid: $PPID, project_dir})` (VERIFIED `cli.ts:112`, `docs/configs/claude-code.md:48-62`).
5. Auto-bind: the daemon finds the most recent live (`last_seen_at` within 5 min) `__channel_proxy__` row with `claude_ui_pid = ui_pid` on the same device. It writes that proxy's csid into the agent's `delivery`, but only if the csid currently has a subscriber (VERIFIED `src/mcp/auto-bind-channel.ts:7,72-120`; call site `src/mcp/tools.ts:1626-1641`).
6. The same `ui_pid` also drives a pid → tty → tmux pane runtime bind, which provides the tmux fallback target (VERIFIED `tools.ts:1029-1036`, `1729-1734`).
7. Recovery after a context clear or restart: `reconnect({ui_pid:$PPID})`, or `reconnect({identity_key})` when a launcher exported `XATS_IDENTITY_KEY`. `bind_channel({channel_session_id})` re-points delivery to a new csid (VERIFIED `cli.ts:100-119`, `src/mcp/bind-channel.ts:33-56`).

### 3.3 Idle/busy detection and draft safety

- **Channel path: no busy check at all.** `dispatchClaude` never inspects Claude's state (VERIFIED `transport-dispatch.ts:157-183`). Busy handling is delegated to Claude's channel queue, which holds events and delivers them together on the next turn (primer §4). This is safe for drafts: a channel event does not touch the composer (INFERRED from primer §4; channel content is not typed).
- **Busy Claude:** the hint lands at the next turn boundary. It is not mid-turn between tool calls, because channels queue to the next turn (primer §4). No body reaches the model until it calls `get_inbox`.
- **tmux fallback path:** the quiet guard captures 8 tail lines, waits 2 s, captures again and fails on any difference. Then come `load-buffer`, `paste-buffer -d`, a 400 ms settle, and `Enter`, with ownership re-checked before load, before paste and before Enter (VERIFIED `src/mcp/poke-guard.ts:26-31`, `src/mcp/poke.ts:173-245`). There is no composer-content check for Claude. The Codex-only `›` readiness allowlist (`requireReady`) is not passed for Claude (VERIFIED: `dispatchTmux` calls `tmuxPoke` without it, `transport-dispatch.ts:134-141`; only `codex-recovery-poke.ts:636` and `codex-seeding-poke.ts:416` supply it). A static, untouched human draft therefore passes the guard and gets the hint pasted onto its end and submitted with Enter (INFERRED). The foreground-carrier proof is Codex-only; the source flags "the same latent hazard exists for claude/other TUI targets" as an untaken follow-up (VERIFIED `poke.ts:405-418`).
- **Retries:** only a tmux `guard_failed` with a pane, and Kimi busy, are retried, at 30 s, 180 s and 600 s, stopping once the cursor passes the message (VERIFIED `auto-poke-fanout.ts:129-178`). A channel wake is never retried, because it always "succeeds" (§3.5).

### 3.4 Latency

- Channel: the daemon reaches the proxy over SSE and then stdio, sub-second (INFERRED). An idle Claude then starts a turn and must call `get_inbox`, which costs one model round trip plus one tool call before the body is read.
- tmux fallback: at least 2 s (quiet guard) plus 0.4 s settle plus 0.4 s post-Enter capture (VERIFIED constants `poke-guard.ts:3`, `poke.ts:103`). A failed guard waits for retries at 30 s, 180 s and 600 s.
- `send_message` blocks the sender for up to 10 s by default, waiting for the read receipt (VERIFIED `tools.ts:166`). On every send to a Claude that is busy or slow to read, this is 10 s of sender wall-clock (INFERRED).

### 3.5 Failure modes

1. **Missing dev-channel flag means a silent black hole.** If Claude was not launched with `--dangerously-load-development-channels server:cross-agent-teams-channel`, the proxy still connects and subscribes, but Claude "drops the events silently" (primer §4). Because the csid is subscribed, `dispatchClaude` returns `ok:true, transport_used:'claude-channel'` and **never falls back to the bound tmux pane** (VERIFIED `transport-dispatch.ts:168-179`). The sender sees `poked:true` and `wake_status: delivered`, and nothing wakes. The only signal is the 15-minute unread watchdog (INFERRED from the composition of the two VERIFIED behaviours).
2. **`channel_sink_failed` is effectively unreachable.** The production sink never throws: it returns silently when there is no transport, and it catches the async rejection of `t.send` (VERIFIED `src/mcp/tools.ts:3065-3071`). Only the test fakes throw (`tests/channel-sink-failure-honest.test.ts:46-50`). A dead SSE GET stream (the proxy has not opened one yet, or it dropped) therefore also reports success (INFERRED). The comment in `channel-wake-fanout.ts:25-33` claims this path was made honest, but that honesty holds only for synchronous throws.
3. **Proxy errors are swallowed** at the Claude edge (`proxy.ts:22-27`). The daemon cannot learn that Claude refused the notification, because the MCP channel has no ACK (VERIFIED code; primer §4).
4. **Pid correlation is fragile** (INFERRED):
   - `findClaudeUiPid` matches only when the first token of an ancestor's `ps args` has basename `claude`. The 8-hop walk tolerates the `npx -y -p …` hop when Claude itself runs as `claude`. A Claude started as `node …/cli.js` or through a renamed wrapper never matches, and the fallback then returns the proxy's immediate parent, which with the documented `npx` launch is npx or a shell, not Claude. `claude_ui_pid` then ≠ the model's `$PPID` and auto-bind misses (`no_proxy_row`). (`docs/configs/claude-code.md:50` still describes the older rule, proxy `process.ppid` == `ui_pid`.)
   - The model's `$PPID` (from its Bash tool) has to equal the Claude pid. That holds only if Claude spawns the shell directly. Bash sandbox wrappers or shell snapshots could break it (unknown).
   - The model also has to run Bash to learn `$PPID`, which costs an extra tool call at registration (INFERRED; no tool hands it over).
5. **A context clear or restart needs model cooperation.** The startup hint asks the model to ask the user before registering (VERIFIED `cli.ts:111`). An unregistered Claude gets no wakes at all.
6. **The 5-minute liveness window on proxy rows.** Auto-bind requires the proxy's `last_seen_at` within 5 min. The proxy's heartbeat is an `echo` tool call every 30 s (VERIFIED `auto-bind-channel.ts:7,111`, `daemon-client.ts:157-178`, `echo` registered at `src/mcp/transport.ts:262`). Tool text says `last_seen_at` is "refreshed by any xats tool call" (VERIFIED `tools.ts:215`); that this covers `echo` is INFERRED (the touch path itself was not read).
7. **Takeover on duplicate registration.** A second MCP session that registers the same `(device, team, name)` force-closes the first (VERIFIED `README.md:267`, `CHANGELOG.md:64`). Two Claude panes that pick the same name silently steal each other's seat (INFERRED).
8. **Protocol gating** (unknown): the primer notes that the v2 MCP runtime does not register channel servers on protocol revision 2026-07-28 (primer §4). The proxy pins `@modelcontextprotocol/sdk ^1.0.0` (`plugins/cross-agent-teams-channel/package.json:15`), and which revision it negotiates is unknown. Channels are also unavailable on Bedrock, Vertex and Foundry auth and need `channelsEnabled` on Team/Enterprise plans (primer §4). None of this is checked or reported by xats (INFERRED: no such check found).

## 4. Setup and installation footprint on Claude

- **MCP config:** two entries in the project `.mcp.json` (or `~/.claude.json`), written by `npx mcpsmgr add jtianling/cross-agent-teams-mcp -a claude-code` or by hand (VERIFIED `README.md:96-137`, `mcpsmgr.json` `agents.claude-code.servers`). The channel entry is documented as **project-level only**. A global install (`claude mcp add --scope user`) gets tools but no push wake (VERIFIED `README.agent.md:51-53,566-572`).
- **Launch flag:** each Claude must be started with the dev-channel flag, usually through shell aliases `xats-claude` and `free-xats-claude` (the latter adds `--dangerously-skip-permissions`) written to `~/.xats.sh` (sourced from `~/.zshrc`) after user consent (VERIFIED `README.agent.md:42-44,92-114,294-295`). The flag raises an interactive confirmation dialog in Claude (primer §4).
- **Runtime:** `npx -y` fetches `cross-agent-teams-mcp` on every proxy spawn, `@latest` in the README variant (VERIFIED `README.md:127-134`). Every Claude session spawns a Node proxy child plus a Streamable HTTP session to the daemon. A resident daemon must already be running (VERIFIED `mcpsmgr.json` prerequisites).
- **No hooks, settings, mods or Claude config edits** beyond MCP server entries (INFERRED from the inventory).
- **Doc bug:** `docs/configs/claude-code.md:9-19` says to put `mcpServers` in `~/.claude/settings.json`. Claude reads MCP servers from `.mcp.json` or `~/.claude.json`, not from `settings.json` (INFERRED from Claude's MCP docs as summarised in the primer §5, which name no settings.json route; the README's own `.mcp.json`/`~/.claude.json` is correct).

## 5. Token cost

- **Per message, recipient side:**
  - one `<channel>` block of about 100 to 200 chars of hint, roughly 50 to 80 tokens with CJK (INFERRED);
  - plus a model turn deciding to call `get_inbox`, the tool-call tokens, and the JSON result (about 300 bytes of per-message metadata plus the body, INFERRED from the field list);
  - plus, on an idle wake, a whole extra turn whose system prompt and context are re-read (cache-dependent, INFERRED).
- **Per message, sender side:** the `send_message` result, carrying `poked`, `poke_skip_reasons`, `retry_*` and `ack:{status, waited_ms}` (VERIFIED field names `tools.ts:178-195`).
- **Per quiet turn:** zero injected bytes. There are no hooks, so a turn with no mail adds nothing (VERIFIED by the absence of hooks).
- **Fixed per-session cost:**
  - server `instructions`, about 3,070 chars / 3.7 KB, mixed Chinese and English (VERIFIED `transport.ts:236-259`);
  - 18 registered tool schemas (including internal `echo` and `subscribe_channel_wake`; some are registered conditionally) whose descriptions are long; `send_message` alone is a few KB including `AWAIT_ACK_DESC` (VERIFIED `tools.ts:166-200`). Claude's tool search may defer the schemas (primer §5), and the startup hint explicitly anticipates deferred tools (VERIFIED `cli.ts:109`);
  - the about 3 KB startup hint, repeated on every proxy reconnect (§2.4).

## 6. Limitations and bugs visible

1. Silent no-wake when the dev-channel flag is missing, and no tmux fallback because the subscription "succeeds" (§3.5.1).
2. Channel success is a write into a stream, not delivery; `channel_sink_failed` is dead code in production (§3.5.2).
3. The body always costs a separate model-chosen tool call. Nothing is delivered in-turn, so a busy Claude in a long tool loop does not see mail until the next turn boundary, and even then only a hint (§2.3).
4. Every send wakes. There is no priority or threshold, and hints are not coalesced, so chatty peers drive many idle turns (INFERRED from VERIFIED §2.3).
5. The tmux fallback can paste into a static human draft and press Enter. There is no Claude composer check, and the carrier proof is Codex-only, by the source's own admission (§3.3).
6. Seat binding hangs on a `ps`-walk heuristic plus a model-supplied `$PPID` (§3.5.4). Registration is model-mediated and gated on asking the user.
7. The startup hint is re-injected on every proxy reconnect (§2.4).
8. `get_inbox` has no byte bound, so large bodies run into Claude's MCP output caps (§2.4).
9. Channels are a research preview with auth, allowlist and protocol-revision gates. The dev flag means a confirmation dialog at each launch and cannot be used under `-p`/SDK (primer §4). Unattended or scripted Claude sessions get no push wake.
10. The wake hint is in Chinese regardless of locale, and the startup hint tells the model to address the user in English (VERIFIED `tools.ts:388`, `cli.ts:111`). This is a minor mismatch but costs tokens on non-CJK models.

## 7. Clever ideas worth stealing

1. **A channel proxy as a thin stdio relay in front of a shared HTTP daemon.** The proxy holds no state. It subscribes with a fresh csid per process and re-emits pushes (`proxy.ts`, `daemon-client.ts`). If herdr-threads ever offers a channel path (for operators who accept the flag), the same split applies: a tiny `herdr-threads channel` stdio server fed by the existing watch stream. The proxy also tells the model its csid, so the model can bind or rebind.
2. **The csid is bound to the seat by daemon-side pid correlation, not by the model copying an id.** The proxy reports `claude_ui_pid`, the model reports `ui_pid`, and the daemon joins them (`auto-bind-channel.ts:102-120`). It is weak as implemented, but the shape (each side reports its own runtime evidence, the daemon joins) matches our A2 "decide in the daemon" stance.
3. **Separate `wake_status` from `read`** in delivery status, and say in the tool description that one is not the other (`tools.ts:2950-2960`). Our sender-side diagnostics could expose the same distinction.
4. **The recipient-named hint (`→ name@team`)** lets a pane that receives a misrouted wake detect it cheaply (`tools.ts:196`). Our content-free wake prompt cannot do this. That is a deliberate trade-off (R18), but worth noting as a seat-confusion diagnostic.
5. **Bounded `await_ack_s` on send**, together with tool text that tells the sender what *not* to conclude from `not_yet` (`tools.ts:166-178`). It is a cheap way to stop agents from re-sending or taking over work after a short silence.
6. **Mailbox-first, then wake, with retries that stop once the cursor passes the message** (`auto-poke-fanout.ts:141,167`). This mirrors our contract and is a good invariant to keep explicit.
7. **A deferred-tool-aware startup hint** ("the tools named below may not be directly callable yet … Load one first", `cli.ts:109`). This acknowledges Claude's tool search, and it is relevant if our check-in text ever names MCP tools.

What not to copy: trusting channel "accepted" as delivery, unbounded hint-per-send wakes, the tmux quiet guard as draft protection, and `ps`-name matching for seat binding.

## Verification

Adversarial pass against the clone at `17df5cf7`, read only and never run, and against primer §4 and §5.

- **Checked and confirmed:**
  - the README/mcpsmgr two-server setup, the dev-channel flag and the aliases;
  - `dispatchPoke` → `dispatchClaude` routing and its channel-then-tmux order;
  - `sendChannelWake` meta sanitising, the subscribe sink (`if (!t) return` plus `.catch`) and the proxy's `fallbackNotificationHandler`;
  - `relayChannelWake` swallowing errors;
  - the hint template, `HINT_MAX_CHARS` and the shedding order;
  - the `get_inbox` fields, the limit of 50/200, the atomic cursor and no body max in the zod schemas;
  - the startup hint lines and their size (3,106 bytes over the cited lines) and its re-emission in the reconnect loop;
  - `findClaudeUiPid`, the auto-bind 5-minute window and the subscriber check, `subscribe_channel_wake` role gating and `bind_channel`;
  - the quiet guard (8 lines, 2 s), the 400 ms settle and the ownership rechecks, and the Codex-only carrier-proof comment;
  - retries limited to `guard_failed`/kimi busy;
  - `await_ack_s` (10 s default, 30 s max) and the 15-minute watchdog;
  - the `wake_status` vs `read` description;
  - takeover (`README.md:267`, `CHANGELOG.md:64`) and the `settings.json` doc bug (`docs/configs/claude-code.md:9-19`).
- **Changed:**
  - removed the non-existent manual `poke` tool (no such tool is registered) and restated the `poke()` bound and its callers;
  - corrected the server `instructions` size and language ("3.4 KB, mostly Chinese, close to the 4,096 cut" became "about 3,070 chars, mixed, under the cut") and widened the range to `:236-259`;
  - corrected the `~/.zshrc` alias target to `~/.xats.sh`;
  - clarified that the pid walk tolerates the npx hop and fails only when no ancestor is named `claude`;
  - made the `last_seen_at` refresh partly VERIFIED (`tools.ts:215`);
  - fixed the citations `tools.ts:179` → `:182` and `poke.ts:178-181` → `transport-dispatch.ts:134-141`, and fixed the §1 cross-reference.
