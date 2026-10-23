# Cotal x Claude Code: message injection and wake

- Upstream: https://github.com/Cotal-AI/Cotal (site https://cotal.ai/). Clone `alts/cotal`, HEAD `6426fb538e0b` (2026-10-23). The clone was only read; nothing in it was executed.
- Citations are `path:line` relative to the clone root. **V** = VERIFIED (read in source or docs), **I** = INFERRED, "unknown" = not found.
- Harness reference: `../../primers/claude.md` (cited as "primer §N").
- Main sources read in full: `extensions/connector-claude-code/hooks/native-wake.js`, `hooks/hooks.json`, `src/{hooks,hook,mcp,monitor,wake-feed,native-deliveries,native-binding}.ts`, `extensions/connector-core/src/{control,relay,inline-delivery,framing}.ts`, `gateway.ts:185-620`, `docs/connect-claude.md`. `claude-plugin/cotal/hooks/` is byte-identical to `extensions/connector-claude-code/hooks/` (checked with `diff -r`) **V**.

## 1. Support status

Claude Code is Cotal's most mature connector. `docs/connectors.md:23` rates it "stable" **V**. Cotal has **two separate delivery architectures** for Claude, and they share almost nothing at the injection layer:

| | Managed seat | Personal native session |
|---|---|---|
| What it is | A `claude` launched by Cotal's manager or by `cotal spawn`, with `COTAL_NAME`/`COTAL_LINK`/`COTAL_AGENT_FILE` in its environment (`docs/connect-claude.md:142-149, 199-203`) **V** | A `claude` the user started, with the plugin installed at user scope. It joins only when the model calls `cotal_connect` (`docs/connect-claude.md:243-254`) **V** |
| Injection | Command hooks: `SessionStart` and `UserPromptSubmit` drain the inbox into `additionalContext` (`src/hooks.ts:319-368`) **V** | Mod (`native-wake.js`): `$.prompt.submit` when idle, `$.session.append` after a main-agent `tool.call` when busy (`hooks/native-wake.js:94-96`) **V** |
| Wake | MCP channel push `notifications/claude/channel`, which needs `--dangerously-load-development-channels server:cotal` (`src/mcp.ts:305, 338-342`; `docs/connect-claude.md:145-146`) **V** | `$.prompt.submit({text})` with the whole message batch as the prompt (`native-wake.js:96`) **V** |
| Ack point | The hook relay's stdout write succeeded, confirmed back over the control socket (`connector-core/src/relay.ts:85-102, 178-190`) **V** | The host returned the stored row's uuid, and for appends the stored text equals the offered text (`native-wake.js:102-114`) **V** |
| Precondition | Channels allowed. Team/Enterprise orgs need `channelsEnabled` (`docs/connect-claude.md:79-103`) **V** | A signed-in mesh whose authority offers a connection-approval page. "A static-auth or open mesh has no page where a person approves a connection, so the plugin opens no identity there" (`docs/connect-claude.md:250-252, 255-267`) **V** |

The managed path is the older, generic one (hooks plus a channel). The personal path is the newer mod-based one, and it is the closest analogue to herdr-threads' `register.js`. An earlier personal-session design used a Claude **plugin monitor** (`extensions/connector-claude-code/CHANGELOG.md:193`: "the plugin declares a monitor, each local server writes the connection id of each wake to a private socket of its own, and the monitor turns each valid id into a fixed event line"). The shipped `monitors/monitors.json` is now `[]`, and the monitor binary is spawned by the mod instead (`native-wake.js:180`). `monitor.ts:62-64` still accepts the classic `CLAUDE_PID` monitor environment **V**.

## 2. Injection mechanism

### 2.1 Managed seat: hook drain (the "single authoritative path")

**Registration.** `hooks/hooks.json:4-95` registers nine events: `SessionStart`, `UserPromptSubmit`, `PreToolUse` (all tools), `PostToolUse` (matcher `^mcp__.+__cotal_(connect|disconnect)$`), `PostToolBatch`, `Notification` (matcher `permission_prompt|agent_needs_input|elicitation_dialog`), `Stop`, `StopFailure` and `SessionEnd`. Every one runs `node "${CLAUDE_PLUGIN_ROOT}/dist/hook.cjs"` **V**.

**Code path.**
1. `src/hook.ts:19` calls `runHookRelay`. The relay reads the event JSON from stdin, dials the session's control socket and sends `{token, event, handoff: true}` (`connector-core/src/relay.ts:207-211`) **V**.
2. The control server (inside the MCP server process) checks the token in constant time (`control.ts:248`), then calls the handler it was started with (`control.ts:273`), which for a managed Claude seat is `createClaudeHandle().handle` (`src/mcp.ts:285-290`) **V**.
3. Only two events inject anything:
   - `SessionStart`: `[channelBriefing, surfaceAutomatic, surfaceTurns]` joined into `additionalContext` (`src/hooks.ts:354-355`) **V**.
   - `UserPromptSubmit`: `[surfaceAutomatic, surfaceTurns]` (`src/hooks.ts:366-367`) **V**.
   - `PreToolUse`, `PostToolBatch`, `Notification`, `Stop` and `SessionEnd` return `{}` for delivery purposes (`src/hooks.ts:369-446`) **V**.
   - **There is no mid-turn hook delivery on a managed seat.** No `PostToolUse` drain exists, and the `PostToolUse` matcher covers only Cotal's own connect and disconnect tools **V**.
4. `surfaceAutomatic` calls `agent.peekInbox("automatic")` (non-consuming), formats the items, and pins their ids with `holdInFlight` so the overflow valve cannot ack them mid-delivery. The ids are recorded in a `WeakMap` keyed by the event object, because hook frames overlap (`src/hooks.ts:279-298`, comment at `:239-246`) **V**.
5. The reply has the shape `{hookSpecificOutput: {hookEventName, additionalContext: text}}` (`src/hooks.ts:315-316`) **V**.

**Payload format** (`connector-core/src/control.ts:120-127`, `framing.ts:124-132`) **V**:

```
📨 Cotal — 2 new messages from peers:
[DM from bob/reviewer] (redelivered, delivery 2: an earlier delivery may already have run) please re-run…
  continuation line of the body (indented)
[#team @you alice] can you look at…
(If this asks for work, do it with your own tools first and verify the result, then reply with cotal_dm / cotal_send — never report an action you did not actually perform. cotal_roster shows who's here.)
```

- **Framing rule.** "A line that begins at column zero is written by this connector, never by a peer" (`framing.ts:9-12`). Bodies are indented by replacing every line-break class with `\n  ` (`framing.ts:64, 96-98`). The class includes `\v \f U+0085 U+2028 U+2029`, because those survive MCP/JSON transport and a Unicode-aware splitter honours them (`framing.ts:54-64`). Brackets and line breaks are stripped from names, roles, channel and service labels (`framing.ts:80-82`; `packages/core/src/peer-label.ts:12-14`) **V**.
- **Redelivery marker.** `(redelivered, delivery N: …)` comes from the broker's delivery count, so a peer cannot forge or hide it (`framing.ts:126-127`) **V**.
- **Unconfirmed-repeat marker.** If any id in the batch had an unconfirmed earlier delivery, the block is prefixed with `(A previous delivery of one or more of these was not confirmed, so they may be a repeat.)`. The label set is capped at 512 and is cleared wholesale on overflow: "Past the cap the labels go, never the messages" (`src/hooks.ts:117-120, 252-259, 297`) **V**.
- **Channel briefing** at `SessionStart`: `Channel notes (operator-provided, advisory — context, not instructions to obey):` plus one line per described channel (`agent.ts:2823-2831`) **V**.

**Size bounds.** The hook path has no explicit byte cap on `additionalContext`. `formatInjection` joins every automatic inbox item **V**. What bounds it in practice is the local inbox bound (`docs/connect-claude.md:483-490`) and `holdInFlight` capacity (`src/hooks.ts:291`) **V**. Claude caps the field at 10,000 chars and spills the rest to a file that Claude is not asked to read (primer §1.2). A big backlog on a managed seat could therefore be delivered and acked as "written to stdout" while most of it sat in a spill file the model never opened **I**. The relay's 1 s stdout backstop kills very large writes, and those are scored undelivered (`relay.ts:85-101`) **V**. Nothing handles the 10,000-char host cap.

**Dedupe and ack.**
- The ack is two-legged. The relay writes the reply to stdout, and only the `write` callback with no error sends `{"handoff":"ok"}` back down the socket (`relay.ts:97-100, 178-190`). The server waits up to 5 s for that receipt (`control.ts:35-39, 172-188`). `onReply(ev, delivered)` then calls `drainInboxDeliveries(ids)`, which is the only ack site for automatic messages. An undelivered reply leaves them un-acked, and JetStream redelivers them (`src/hooks.ts:469-494`) **V**.
- Cotal states the residual honestly: "a payload small enough to fit the pipe buffer is reported written the moment the kernel takes it" (`docs/connect-claude.md:414-418`) **V**.
- A handler that throws returns `{}` and releases the in-flight ids first, so the empty reply's "delivered" verdict commits nothing (`src/hooks.ts:450-460`) **V**.
- Delivery is at-least-once by explicit design, with the reasoning in the comment at `src/hooks.ts:153-164` **V**.

**When it fires.** At session start (startup, resume, clear, compact, fork) and at the start of every turn, whether a human typed the prompt or the channel push started the turn. Mail that arrives mid-turn waits for the next turn **V** (code). Whether a turn started by a channel push fires `UserPromptSubmit` is the premise of the whole design: "so the drain runs *now* instead of at the next prompt" (`docs/connect-claude.md:427-430`). Cotal's docs assert it, and we have not verified it against Claude's docs (primer §4 says idle wake by channel is itself only INFERRED; primer §10 open question 3) **I**.

### 2.2 Managed seat: the channel push content

The channel notification is **only a nudge, not the body**. `createWakePolicy` emits one of these (`src/hooks.ts:588-593`) **V**:

- `📨 New dm from bob/reviewer — delivering your Cotal inbox now.` with meta `{kind, from, from_id, role?, channel?, to_role?, mentions?, mentioned?}` (`connector-core/src/tool-specs.ts:652-661`);
- `📨 N Cotal messages waiting — delivering your inbox now.` with meta `{kind:"batch"}`;
- `📨 You were mentioned by X on #ch — pull it with cotal_inbox.` (focus mode, `src/hooks.ts:627-631`).

Claude renders these as `<channel source="cotal" kind="dm" from="bob" …>📨 …</channel>` (primer §4). The server instructions describe peer messages as arriving in `<channel …>…</channel>` tags (`src/mcp.ts:315-317`), and `docs/connect-claude.md:229-231` says "Inbound mesh messages arrive in context as `<channel …>…</channel>`". The code puts only the nudge sentence in the channel tag and sends the body through the hook. The docs and instructions therefore overstate what the tag carries **V** (code vs docs). A managed seat thus gets **two model-visible artifacts per wake**: the nudge in a channel tag, and the hook's `additionalContext` block, assuming the channel-started turn fires `UserPromptSubmit` (§2.1) **I**.

### 2.3 Personal native session: the mod path

**Process topology** **V**:
- The plugin's stdio MCP server (`dist/mcp.cjs`, `src/mcp.ts:74-145`) opens a "wake feed". This is a Unix socket at `$TMPDIR/cotal-claude-wake-<uid>/<claudePid>-<serverPid>.sock`, mode-0700 directory, plus a sibling `.native` socket for lifecycle and receipts (`src/wake-feed.ts:25-37, 63-66, 85-149`).
- The mod's `session.start` hook spawns `node dist/monitor.cjs --native --session <id>` via `$.process.spawn` (`native-wake.js:180, 278-282`).
- The monitor finds every live server socket for its host pid, a sibling under the same Claude parent (`process.ppid`), by scanning the directory every 1 s (`monitor.ts:14-15, 62-64, 91-137`). It sends `{"op":"watch",session,inline:1}` and re-emits validated frames as fixed stdout lines (`monitor.ts:97-118`).
- The monitor exits when `kill(claudePid, 0)` fails (`monitor.ts:128-133`).

**Two kinds of offer reach the mod** (stdout lines parsed at `native-wake.js:182-212`) **V**:
1. **Inline batch** (`Cotal inline delivery: {json}`): whole messages, validated by `inlineFrame` (`native-wake.js:31-38`):
   - at most 8 items;
   - frame at most 4,096 chars;
   - text must start with `Cotal: inline messages for connection_id <id>.\n`.
2. **Pointer plus preview**:
   - `Cotal preview for connection_id <id>: {"unread":N,"rows":[…≤8 rows ≤240 cp]}`, then
   - `Cotal: a new message for connection_id <id>. Read it with cotal_inbox.`, a fixed regex-checked line (`native-wake.js:1-2`; `monitor.ts:18-20, 115`).
   - The preview is used **only for the UI disclosure**. The model gets the fixed pointer line alone (`native-wake.js:188-196, 252`).

**Inline payload** (`connector-core/src/inline-delivery.ts:37-48`; `framing.ts:115-122`) **V**:

```
Cotal: inline messages for connection_id 3f2c….
📨 Cotal — 1 new message from peers:
[DM from bob] [sender_id="…" message_id="…" context_id="…"] body…
(If this asks for work, do it … cotal_roster shows who's here.)
These complete messages were delivered inline; do not fetch them again with cotal_inbox. Native delivery can repeat after an unconfirmed handoff; use message_id to avoid repeating work. 2 other messages remain pending; use cotal_inbox to read those.
```

**Selection and bounds** (`inline-delivery.ts:24-57`) **V**:
- Messages are taken from `peekInbox("automatic")` in inbox order (directed first, `docs/connect-claude.md:474-476`).
- An item whose rendered line exceeds **1,024 code points** is skipped, not truncated ("without consuming a preview or truncating a body", `:22`).
- At most **8 items** per batch.
- The adapter's `fits()` check measures the final JSON frame against **4,096 chars** (`native-deliveries.ts:6, 51-53`).
- A message that never fits goes out through the pointer path and costs the model a `cotal_inbox` tool call **V**.

**When it fires** (`native-wake.js:120-165, 293-336`) **V**:
- `active` is set on main-agent `turn.start` and `turn.step` and cleared on `turn.complete`. Subagents are filtered by `e.agentId === undefined`.
- **Idle** (`!active`): `$.prompt.submit({ text })` is called with the full batch text, which starts a new turn.
- **Busy**: inside the `tool.call` hook, *after* `await next(input)` returns, and only for the main agent and when `!next.signal.aborted` (`:313-314`), the mod calls `$.session.append({message:{type:'user',content:[{type:'text',text}]}})`. This adds a hidden user row that "a running turn's requests" read "from the loop's next top" (primer §2.2). It does **not** use the tool result's `context` field, which herdr-threads uses; no rationale for that choice is stated in the module or docs **I**.
- In `turn.step` the mod deliberately does nothing: "This request already owns its message snapshot. Appending here stores a row without putting it in that request, and a final response would leave no next request to read it" (`:317-323`).
- After `turn.complete`, anything still pending goes out through `prompt.submit` (`:334`).
- An offer whose mode no longer matches (it was prepared idle and the turn is now busy, or the reverse) is `defer`red back to the server and kept locally (`:87-91`).

**Dedupe, ack and receipts (claim/commit protocol)** (`native-wake.js:40-118`; `native-deliveries.ts:55-117`) **V**:
1. `claim`: the server checks the batch is still current (no explicit `cotal_inbox` read superseded it) and the reader is alive. It returns the frame and starts a 30 s admission deadline (`native-deliveries.ts:41, 82-90`).
2. The mod checks the claimed frame byte-for-byte against the offer (`native-wake.js:79-80`).
3. The mod submits or appends.
   - **Idle**: confirmation comes from the mod's own `session.append` hook (`door: 'prompt'`). That hook fires when the engine stores a prompt row whose origin is this plugin and whose text contains the offered bytes on whole lines, and it records the row's uuid. The mod waits for it up to 2 s (`native-wake.js:102-104, 222-240`). The comment notes Claude may wrap plugin prompts in its own attribution text, so the match is "complete offered bytes on whole lines and our plugin origin, never a marker alone" (`:227-229`).
   - **Busy**: the returned `uuid` is used, and the stored text must equal the offered text (`:105-106`).
4. `commit` with the uuid, which makes the server run `batch.settle(true)` → `drainInboxDeliveries` (ack) (`native-deliveries.ts:98-100`; `inline-delivery.ts:66-70`).
   - A missing uuid, `drop` or `deny`, or a thrown error **pauses** delivery (`native-wake.js:97-109`).
   - The frame is `reject`ed, so the server releases it un-acked. It is kept in `retries` for a later `retry` (`:53-56`).
- Message ids in the text let the model itself dedupe repeats ("use message_id to avoid repeating work") **V**.
- Only one inline offer is in flight per session. A second short arrival while one is open does not produce a competing pointer notification (`src/mcp.ts:98-101`; `native-deliveries.ts:56`) **V**.

**Transcript UI.** A `ui.render` hook on `UserMessage` collapses Cotal's own prompt rows into a one-line button such as `Cotal · 2 messages · 1 pending ›`, which expands on press (`native-wake.js:241-277`). It matches only rows whose request id it recorded or whose origin is this plugin, never by text alone (`:244-249`) **V**.

### 2.4 Pull and long-poll fallbacks (both modes)

- `cotal_inbox` is a bounded pull: directed mail first, an oversized message split into parts across calls, and items cleared only after their last part goes out (`docs/connect-claude.md:474-479`) **V**.
- `cotal_wait` (personal and hosted) blocks until `inboxWake() > 0`, for at most `GATEWAY_WAIT_SECONDS = 50`. The bound is chosen because "Claude Code gives a remote server 60 seconds to that byte" (`gateway.ts:189-193, 321-344, 522-573`). It ends only on wake-worthy items: directed messages, @mentions, and ambient chatter in `open` mode (`gateway.ts:304-309`) **V**.
- Every other gateway tool result appends `N new messages waiting. Read them with cotal_inbox.` unless the native reader owns full-content delivery (`gateway.ts:355-361, 607`; `src/mcp.ts:117-120`) **V**.

## 3. Wake mechanism

### 3.1 Managed seat: dev channel

- **Mechanism.** The MCP server declares `capabilities.experimental["claude/channel"]` (`src/mcp.ts:305`) and pushes `notifications/claude/channel` (`:340`). The wake policy turns on only after the MCP handshake shows the client speaks `claude/channel`, or when `COTAL_CHANNEL` forces it (`:353-361`) **V**.
- **Who is nudged** (`src/hooks.ts:618-632`) **V**:
  - directed messages (DM, anycast, @mention): always nudged;
  - ambient channel traffic: nudged only in `open` attention while `status !== "working"`;
  - at `Stop`, if `pendingWake() > 0`, one batch nudge goes out through `requestWake()` (`src/hooks.ts:439`; `agent.ts:1709-1734`).
  - Open-ended run turns count as directed (`agent.ts:1710-1712`).
- **Idle/busy detection.** Presence comes from hooks: `UserPromptSubmit` → working, `Stop`/`StopFailure` → idle (`src/hooks.ts:357-362, 409-430`) **V**. The wake itself does not gate on busy for directed mail. It relies on Claude queueing a channel event that arrives mid-turn until "the next turn" (primer §4) **I**.
- **Retry and dedupe.**
  - A rejected push is retried with exponential backoff, 1 s up to 30 s (`src/hooks.ts:512-513, 559-580`) **V**.
  - An `announced` WeakMap stops JetStream redeliveries during a long tool call from re-printing a nudge for an item an earlier push already covered. A redelivery nudges again once a hook frame has carried the item or the push failed (`src/hooks.ts:542-551, 603-606, 619, 644-646`) **V**.
  - A focus-mode @mention is acked and dropped at ingest. It is held as `pendingMentionWake` until *its own* push succeeds, because nothing else can replay it (`src/hooks.ts:537-541, 571-576, 595-601`) **V**.
- **Draft safety.** A managed seat is assumed to have no human at the terminal (`docs/connect-claude.md:151-152`) **V**. The channel push does not type into the composer, and its interaction with a typed draft is unknown (primer §4).
- **Startup.** Launching requires `--dangerously-load-development-channels server:cotal`, which raises a confirm dialog. The runtime screen-scrapes for the dialog title and presses Enter once, with a 15 s bound (`docs/connect-claude.md:204-216`) **V**. Startup also answers Claude's workspace-trust dialog with keystrokes (Enter, or a move to Yes then Enter) and refuses to guess on an unknown layout (`docs/connect-claude.md:217-227`) **V**. These startup keystrokes are the only typed input in this path; no message is ever typed.
- **Failure modes** **V** (docs):
  - Channels blocked by an org (Team/Enterprise without `channelsEnabled`) leaves **no idle wake**. The seat silently degrades to hook-only delivery, and "A successful spawn or a connected MCP server does not prove idle wake is available" (`docs/connect-claude.md:95-98, 207-211`).
  - Channels are a research preview with Anthropic auth only (`:446-449`).
  - "Live-only traffic has no durable retry" (`:439`).
  - Primer §4 adds that the v2 MCP runtime does not register channel servers on protocol revision 2026-07-28, a future-breakage risk **I**.

### 3.2 Personal native session: `$.prompt.submit`

- **Mechanism.** `$.prompt.submit({ text })` with the full batch, or the fixed pointer line (`native-wake.js:96, 144`) **V**. This is a host-native queued prompt: it "starts a turn of its own once the session is idle (never folded into a running turn)" (primer §2.3). Claude attributes it to the plugin ("The cotal plugin sent a message: …", primer §2.3), and Cotal never sets `asUser` **V** (`asUser` is absent from the call). Cotal's docs promise "plugin-sender attribution, never the person's own identity" (`docs/connect-claude.md:332-333`) **V**.
- **Trigger.** The server's gateway `signal()` fires once per connection when wake-worthy work arrives and no `cotal_wait` holds the connection. It is latched until a read, or until an inline settlement rearms it (`gateway.ts:398-419`; `src/mcp.ts:95-116`) **V**. It does not compete with an active `cotal_wait` (`docs/connect-claude.md:357`) **V**.
- **Idle/busy detection.** The in-process `active` flag from `turn.start`/`turn.step`/`turn.complete`, main agent only (`native-wake.js:293-296, 317-336`) **V**. No `$.prompt.read()` and no `prompt.edit` observation appear anywhere in the module (grep over `src/`, `hooks/` and `docs/connect-claude.md`: no `prompt.read` or `prompt.edit`; `composer` occurs only as a `prompt.submit` origin kind at `native-wake.js:287`) **V** (absence).
- **Draft safety: none of its own.** The mod relies entirely on `prompt.submit`'s own queueing semantics. A person half-way through typing at an idle prompt can have a Cotal turn start underneath them **I**. It is unknown whether Claude keeps the draft intact in that case. herdr-threads holds a wake for up to 120 s while the composer holds text (`DRAFT_WAIT_MS`, primer §9). Cotal has no equivalent.
- **Stop respect.** After `turn.complete` with `isAborted` (the user pressed Esc or Stop), `paused = true` and all offers are rejected back un-acked (`native-wake.js:328-333`). Paused also comes from a host `drop`/`deny`, or from an unconfirmed admission (`:97-109, 145-158`). Only a `prompt.submit` whose `origin.kind` is `composer`, `bridge` or `sdk` resumes, with fresh offers (`:283-292`). Delivery stays pending "without a retry loop" (`docs/connect-claude.md:360-361`) **V**. A peer message, a timer or another plugin's prompt can never restart work after Stop **V**.
- **Latency** **I**:
  - Idle: about one socket hop, then a short-lived `monitor.cjs --native-delivery` process per receipt step, then `prompt.submit`. Each `claim`/`commit` is a `$.process.run` of node with a 3 s timeout (`native-wake.js:45-48`), so an idle wake pays at least two node process starts before the turn is committed.
  - A new server socket is discovered within 1 s (`monitor.ts:15`).
  - Busy: the next main-agent tool boundary. A long-running tool defers delivery indefinitely; the offer stays held by the live reader (`native-deliveries.ts:59-60`).
- **Failure modes** **V**:
  - Without the native module (Claude's own gates: older than 2.1.287, `--bare`, `--safe-mode`, `disableAllHooks`, managed `allowManagedModsOnly`; primer §2.1, not stated by Cotal), no feed reader attaches, and the `cotal_connect` listening text tells the model "Automatic incoming messages are unavailable in this session ... call cotal_wait to listen" (`src/mcp.ts:121-124`) **V**. If the module loads but its reader fails, `listen` logs "Cotal incoming auto-wake is unavailable. Run cotal setup, then reopen Claude." to the transcript UI only (`native-wake.js:215-218`) **V**.
  - A personal session on a static or open mesh gets no identity at all (§1).
  - "attachment alone does not prove native admission" (`docs/connect-claude.md:362`).
- **Desktop Chat and hosted connector:** no wake at all. "A chat with no turn running cannot be woken by any server" (`docs/connect-claude.md:392-394, 884-888`) **V**.

## 4. Setup and installation footprint on the harness

- **`cotal setup`** **V**:
  - materializes a local marketplace at `~/.cotal/claude-plugin/` (or `$COTAL_HOME`, or `%LOCALAPPDATA%\Cotal`), built atomically by staging and rename (`src/setup.ts:162-228`);
  - writes `.claude-plugin/marketplace.json` there (`:283-288`);
  - then shells out to Claude's own CLI: `claude plugin marketplace add <dir>`, `claude plugin marketplace update cotal-mesh`, `claude plugin install cotal@cotal-mesh --scope user` and `cotal-skills@cotal-mesh --scope user` (`:290-299, 354-363`).
  - Cotal therefore never edits `~/.claude/settings.json` itself. Claude records the user-scope plugin enablement and the marketplace **I** (Claude's own behaviour for `plugin install --scope user`).
  - It reads `~/.claude/plugins/installed_plugins.json` (or `$CLAUDE_CODE_PLUGIN_CACHE_DIR`) to decide whether a refresh is owed: "never rewrite them ourselves" (`:301-324`).
  - Older per-project local-scope installs are uninstalled (`docs/connect-claude.md:184-188`).
- **Plugin contents** **V**:
  - `.claude-plugin/plugin.json`;
  - `.mcp.json`, which runs `node --optimize-for-size --max-semi-space-size=1 ${CLAUDE_PLUGIN_ROOT}/dist/mcp.cjs`;
  - `hooks/hooks.json` with the nine command hooks plus `"modules": ["./native-wake.js"]`;
  - an empty `monitors/monitors.json`;
  - `dist/{mcp,hook,monitor}.cjs`;
  - `dist/cli.json` (the recorded `cotal` CLI argv), written by setup and not part of the committed plugin tree (`src/setup.ts:76`; `src/cli-record.ts:5`; read at `native-wake.js:23`).
- **Per-session runtime state** **V**:
  - `$TMPDIR/cotal-claude-wake-<uid>/` (feed sockets);
  - `$TMPDIR/cotal-claude-native-<uid>/<session>.json[.host]`, rewritten on every main-agent `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolBatch`, `Notification`, `Stop`, `StopFailure` and `SessionEnd` hook in a personal session (subagent events and `PostToolUse` are skipped), and deleted at `SessionEnd` (`src/native-binding.ts:10-18, 73-96`);
  - `CLAUDE_PLUGIN_DATA` records of hosted `connection_id`s (`docs/connect-claude.md:874-877`).
- **Managed launches** add flags and environment rather than config (`docs/connect-claude.md:142-149, 164-170, 188-189`) **V**:
  - `--mcp-config '{…}'`;
  - `--dangerously-load-development-channels server:cotal`;
  - `--permission-mode bypassPermissions` by default;
  - `--append-system-prompt-file <0600 persona file>`;
  - `--plugin-dir <package copy>`;
  - environment `COTAL_*`.
- **Claude Desktop** (macOS and Windows): a `cotal` entry in `claude_desktop_config.json` (`docs/connect-claude.md:365-379`) **V**.
- **Skills:** a second plugin, `cotal-skills`, at user scope, plus `~/.agents/skills/` for other harnesses (`docs/connect-claude.md:55-73`) **V**.

## 5. Token cost

Byte counts were measured with Python on the literal strings, with a 36-char uuid for ids **V** (measurement).

| Item | Bytes (approx.) |
|---|---|
| `formatInjection` header `📨 Cotal — 1 new message from peers:` | 40 |
| Per-block instruction tail ("If this asks for work, do it … cotal_roster shows who's here.") | **205** |
| Inline prefix `Cotal: inline messages for connection_id <uuid>.` | 79 |
| Inline trailer ("These complete messages were delivered inline; … use message_id …") | **186**, plus about 70 when some remain pending |
| Per-item inline metadata `[sender_id="…" message_id="…"] ` (more with `context_id`/`reply_to`) | about 101 to 200 |
| Pointer line `Cotal: a new message for connection_id <uuid>. Read it with cotal_inbox.` | 102, plus a `cotal_inbox` round trip |
| Managed channel nudge `📨 New dm from bob — delivering your Cotal inbox now.` | 57, plus `<channel …>` tag attributes (about 60 to 100) |
| Claude's own plugin-attribution wrapper on `prompt.submit` | unknown; Cotal notes it exists (`native-wake.js:227`) |

- **Per message, personal inline idle wake:** about 79 + 40 + 205 + 186 = **about 510 bytes of fixed framing per batch**, plus about 100 to 200 bytes of metadata per item, plus the sender attribution line and body. The batch also arrives as a user turn of its own, which costs a model call.
- **Per message, managed seat:** about 40 + 205 = **about 245 bytes of framing** in the hook block, plus the channel nudge (about 120 to 160 bytes with tag), plus a turn. Every hook-drained block repeats the 205-byte tail. herdr-threads' framing is a one-line header per item with no static tail (R17), so Cotal spends about 0.4 to 0.7 KB more per delivered batch **I**.
- **Per session, managed:** the MCP server `instructions` (about 1.5 KB of literal text at `src/mcp.ts:307-329`, plus four imported constants whose length was not measured), the full `cotal_*` tool catalog (deferred under tool search, primer §5), and a channel briefing at `SessionStart` when channels have descriptions **V**/**I**. Personal sessions get `LOCAL + GATEWAY_CONNECTING + CODE_LISTENING`, about 1.3 KB (`src/mcp.ts:48-67`; `gateway.ts:616-627`) **I** (estimated from source).
- **Per quiet turn:** zero model tokens. With an empty inbox, every hook returns `{}` or empty stdout (`src/hooks.ts:281-282, 315-316`; `relay.ts:88`), and the mod injects nothing **V**. The cost is CPU and latency instead. **Every** `PreToolUse`, `PostToolBatch`, `Stop` and similar event spawns a `node dist/hook.cjs` process, and in a personal session each main-agent one also rewrites a 0600 state file, and once the conversation has connected (a `.host` record exists) forwards it over a socket (`hooks/hooks.json:24-33`; `src/native-binding.ts:73-92`) **V**. Cotal works to keep that process light (`docs/connect-claude.md:180-183`, `pnpm smoke:claude-hook-weight`) **V**.

## 6. Limitations and bugs visible from source

1. **No mid-turn delivery on managed seats.** Hooks drain only at `SessionStart` and `UserPromptSubmit` (§2.1). A DM that arrives during a 20-minute turn reaches the model only at the next `UserPromptSubmit`, which in practice means after `Stop` and a fresh channel-started turn **V** (code) / **I** (that a channel-started turn fires `UserPromptSubmit`). The personal mod path has mid-turn append; the managed path does not.
2. **The hook block ignores Claude's 10,000-char cap.** There is no size bound on `formatInjection` output in the hook path, and an ack on stdout write means a spilled overflow counts as delivered **I** (cap from primer §1.2; no bound found in `src/hooks.ts`/`control.ts`). The native inline path is tightly bounded (4,096-char frame); the hook path is not.
3. **No draft guard on personal idle wake.** There is no `$.prompt.read` check (§3.2) **V** (absence). `prompt.submit` can start a turn while the person is typing **I**.
4. **Docs and instructions overstate the channel payload.** They say messages arrive "as `<channel …>…</channel>`" (`docs/connect-claude.md:229-231`; `src/mcp.ts:315-317`), but the channel carries only a nudge sentence and the body comes through the hook (§2.2) **V**. A model told to "read" channel tags may reply to the nudge instead of the hook block **I**.
5. **Double framing per managed wake.** A nudge sentence in the channel tag *and* the hook block, plus the 205-byte tail each time **V**.
6. **A big personal message costs a round trip.** Bodies over 1,024 code points, or batches that do not fit 4,096 chars of JSON, fall back to a pointer that needs a `cotal_inbox` tool call. The model gets no preview (the preview is UI-only) **V**.
7. **Admission confirmation depends on text matching** of Claude's stored prompt row (`native-wake.js:226-230`), with a 2 s window. If a future Claude rewrites plugin prompt text more heavily, every idle delivery would "fail to confirm", and the mod would **pause until a human types**. That is a silent stall, though it fails safe for message loss **I**.
8. **The repeat-label cap clears every label** on overflow, so after a burst of unconfirmed deliveries genuine repeats arrive unlabelled. Cotal documents this as accepted (`src/hooks.ts:256-259`) **V**.
9. **Personal native wake requires a signed-in mesh with a person-approval web page.** A local static or open mesh, the common single-machine case, gives a personal session no identity, so it has no wake at all (`docs/connect-claude.md:250-252`) **V**.
10. **Managed startup needs a screen-scraped keystroke** to accept the dev-channels dialog, and if neither the dialog nor the input screen is recognised within 15 s the seat ends with a bounded error (`docs/connect-claude.md:204-216`) **V**, so a change in Claude's dialog wording would break managed startup **I**. The runtime refuses to guess on an unknown dialog (`docs/connect-claude.md:222-223`) **V**.
11. **Each idle inline wake runs several short node processes**: a `claim` run, a `commit` run, and possibly `retry`, each with a 3 s timeout (`native-wake.js:45-48`). Under load those timeouts push delivery into the pause state (`:97, 107`) **I**.
12. **The per-block instruction tail is static text repeated in every delivery.** By herdr-threads' rules (a), (b) and (f) it belongs once per session. Cotal added it deliberately after a weak model "confirmed" work it had not done (`control.ts:112-119`) **V**.

## 7. Clever ideas worth stealing

1. **Host-confirmed admission as the ack** (claim → admit → commit with the stored row uuid and an exact text match; `native-wake.js:73-118`; `native-deliveries.ts`). It is stronger than "the hook returned" and stronger than display-ACK. It is a cheap upgrade for our `watch ack --via append|submit`: record the `session.append` result uuid, and for `prompt.submit` observe our own row in a `session.append` hook before acking **V**/**I**.
2. **Pause on user abort, resume only on person-origin input** (`origin.kind ∈ {composer, bridge, sdk}`) (`native-wake.js:283-292, 328-333`). herdr-threads has a post-abort hold. Cotal's rule is stricter and event-driven (no timer), so it is worth comparing directly **V**.
3. **Never append in `turn.step`.** The request snapshot is fixed, and a final response leaves no next request to read the row (`native-wake.js:317-323`). Check our mod for the same trap **V**.
4. **Append after `await next(e)` in `tool.call`, not as result `context`.** It lands as a real (hidden) user row at the loop's next top, so it is visible in `session.messages()` and has a uuid to ack against **V**. The trade-off: a tool result's `context` is "one reminder, as a PostToolUse hook's is" (primer §2.2), while an appended user row may carry more weight with the model **I**.
5. **A `ui.render` collapse of the plugin's own wake rows** into `Cotal · N messages · M pending ›`, keyed by recorded request id, never by text (`native-wake.js:241-277`). This is a direct UX improvement for our `wake up` rows **V**.
6. **Two-legged hook ack through a stdout-write receipt** (`relay.ts:85-102, 178-190`; `control.ts:142-201`). It is relevant if we ever deliver content through `SessionStart` or `UserPromptSubmit` `additionalContext` **V**.
7. **Line-break neutralization covering U+2028, U+2029, U+0085, `\v` and `\f`**, plus bracket stripping in sender labels (`framing.ts:54-82`). Our framing rule (header line plus indented body) has the same "column zero is ours" property. We should verify that our indenter catches these code points too **V**/**I**.
8. **Broker-sourced redelivery marker** `(redelivered, delivery N: an earlier delivery may already have run)` and a `message_id` the model can dedupe on (`framing.ts:126-127`; `inline-delivery.ts:43`). It is a compact, honest at-least-once signal **V**.
9. **Per-message nudge dedupe across redeliveries** (`announced` WeakMap) with bounded backoff retry of a rejected push (`src/hooks.ts:512-611`). The pattern applies to any wake path where a nudge can fail silently **V**.
10. **Inline only whole messages, never truncated**, falling back to a pointer, and telling the model the `remaining` count (`inline-delivery.ts:22-57`). Our 8 KiB truncate-with-continuation is different. Cotal never shows a partial body as if it were complete **V**.
11. **Sibling discovery by shared parent pid.** The MCP server and the mod-spawned monitor are both children of the Claude host, so `process.ppid` names the session's sockets without any env plumbing (`wake-feed.ts:1-9`; `monitor.ts:62-64`). It is a neat trick for matching a mod helper to an MCP server, though it depends on Claude spawning both directly **V**/**I**.
12. **Self-exit when the host pid dies** (`kill(pid, 0)` every second; `monitor.ts:128-133`). It is cheap leak prevention for helper processes **V**.
13. **Guard against duplicate integration paths.** A `PreToolUse` hook refuses `cotal_*` tools from any other MCP server in a managed session, decided in the hook process itself so it holds when the control socket is down (`docs/connect-claude.md:864-872`) **V**.

## 8. Quick contrast with herdr-threads on Claude (for the comparison step)

| Aspect | Cotal (personal mod) | Cotal (managed) | herdr-threads (mod) |
|---|---|---|---|
| Idle wake | `prompt.submit(full batch)` | dev channel push | `session.append` rows + `prompt.submit('wake up')` (content-free, R18) |
| Busy delivery | `session.append` after main `tool.call` | none until next turn | `tool.call` result `context` |
| Draft guard | none | n/a (no human assumed) | 120 s `$.prompt.read` hold |
| Abort handling | pause until person-origin prompt | n/a | post-abort hold |
| Ack | host row uuid + exact stored text | hook stdout-write receipt | `watch ack --via context/append/submit` |
| Framing overhead per batch | about 510 B fixed + about 100 to 200 B per item | about 245 B + nudge | header line per item, no tail |
| Works without channels or approval page | needs a signed-in mesh with approval | needs channels allowed for idle wake | yes |

Sources for the herdr-threads column: primer §9 **V**.

## Verification

Adversarial pass against the clone at `6426fb538e0b` and the primer. Checked every `path:line` citation in §1-§7 against source (`native-wake.js` in full; `hooks.json`; `src/{hook,hooks,mcp,monitor,wake-feed,native-deliveries,native-binding,setup,cli-record}.ts`; `connector-core/src/{control,relay,inline-delivery,framing,gateway,agent,tool-specs}.ts`; `packages/core/src/peer-label.ts`; `docs/connect-claude.md`; `docs/connectors.md`; the CHANGELOG; both `monitors.json`; `.mcp.json`), re-ran `diff -r` on the two `hooks/` trees (identical), and re-measured the §5 byte counts (they match; the inline prefix is 78 bytes, 79 with its newline).

Changed:
- `docs/connectors.md:22` → `:23`; `CHANGELOG.md:193` → `extensions/connector-claude-code/CHANGELOG.md:193`; `dist/cli.json` citation `setup.ts:31` (asset list, which omits it) → `setup.ts:76`.
- `control.ts:273` calls the injected handler, not `createClaudeHandle()` directly; added the `mcp.ts` wiring.
- "Primer §4 open question 3" → primer §10 open question 3, and marked the channel-turn → `UserPromptSubmit` premise, the double-artifact claim and §6.1's timing as INFERRED where they depend on it.
- §3.1: startup keystrokes also answer the workspace-trust dialog (`docs/connect-claude.md:217-227`); the "only keystroke" claim was wrong.
- §3.2: the no-module conditions (2.1.287, `--bare`, safe mode) come from the primer, not Cotal; the `native-wake.js:215-218` log covers a failed reader, not a missing module. The absence grep was overstated: `composer` does occur, as an origin kind at `native-wake.js:287`.
- "Deliberately not `context`" was unsupported; now INFERRED with no stated rationale.
- Personal state-file writes are not on every hook (`PostToolUse` and subagent events are skipped) and forwarding needs a prior binding.
- §6.10: the dialog-wording breakage is an inference from the documented 15 s bound, now marked INFERRED.

No other claim was refuted.

