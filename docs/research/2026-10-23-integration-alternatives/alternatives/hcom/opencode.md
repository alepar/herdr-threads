# hcom x OpenCode: message injection and wake

- Upstream: https://github.com/aannoo/hcom (MIT; 597 stars / 94 forks; latest release v0.7.31, 2026-10-21; GitHub API figures from `overview.md`, fetched 2026-10-23).
- Clone: `alts/hcom` (scratchpad), `git clone --depth 1`, HEAD `de33e31a0cdb` (2026-10-21). Read only, nothing executed.
- Citations are `path:line` relative to the clone root. **V** = VERIFIED (read in source, tests or CHANGELOG). **I** = INFERRED. OpenCode-side facts come from `../../primers/opencode.md` (cited as "primer §N").
- Kilo Code shares this plugin (`src/integration_spec.rs:1620-1627`; `src/hooks/opencode.rs:421-427`) **V**. Kilo specifics are left out here.

## 1. Support status

**Native, first-class.** `OPENCODE` is a released integration (`released: true`, `src/integration_spec.rs:1529-1538`) **V**. It has its own hook set (`opencode-start|standby|status|read|stop|approve`, `src/integration_spec.rs:360-367`) and a single TypeScript plugin, `src/opencode_plugin/hcom.ts` (1221 lines), which is compiled into the hcom binary (`src/hooks/opencode.rs:409`, `include_str!`) **V**. The plugin targets both plugin APIs from one module:

- **OpenCode 1.x** (the stable line; the local install is 1.18.1) goes through the v1 `server` export, `HcomPlugin` (`hcom.ts:1022-1077`) **V**.
- **OpenCode 2.x** (the dev / pre-release line) goes through the v2 `setup` export, `setupOpenCode2` (`hcom.ts:1123-1213`). The default export carries both (`hcom.ts:1217-1221`) **V**. OpenCode 2 support landed in 0.7.26 (CHANGELOG.md:204, #143) **V**.

Delivery after bootstrap belongs entirely to the plugin. The `hcom pty` wrapper still runs and still owns the terminal, but every PTY gate is off for OpenCode: "Runtime gates off — the TypeScript plugin owns delivery after bootstrap" (`src/integration_spec.rs:1552-1563`) **V**. The PTY delivery thread enters "plugin mode" for OpenCode. It types into the TUI in exactly one case: an OpenCode 2 launch that has no session yet (`src/delivery.rs:1669-1708, 1782-1796, 1421-1460`) **V**.

Hooks are inert outside hcom launches: the plugin requires `HCOM_LAUNCHED=1` (`hcom.ts:455`), and the Rust side gates on `HCOM_PROCESS_ID` (`src/hooks/opencode.rs:348-351`, `common::hook_gate_check`) **V**.

## 2. Injection mechanism

### 2.1 Transport between hcom and the plugin

The plugin keeps no state of its own about mail. It shells out to the hcom binary: `spawn("hcom", args)` with a 15 s SIGKILL timeout. Every RPC exits 0 and returns JSON, `{"error":…}` on failure (`hcom.ts:51-104`; `src/hooks/plugin_rpc.rs:1-5`) **V**.

- `opencode-read --name N` returns `{"messages":[…], "has_more":bool}`: the oldest unread messages, at most `MAX_MESSAGES_PER_DELIVERY` = 50 of them, in event order (`src/hooks/plugin_rpc.rs:119-159`; `src/hooks/common.rs:261-267`; `src/shared/constants.rs:19`) **V**.
- `opencode-read --name N --ack --up-to ID` moves the instance cursor forward only (`plugin_rpc.rs:132-144`, test `ack_never_moves_cursor_backwards` :279-297) **V**.
- Every read, ack, status or stop call must come from the launch that is attached to `--name` (`require_attached`, `plugin_rpc.rs:46-62`; enforced at `src/hooks/opencode.rs:361-367`). A stopped launch therefore cannot read mail for a resumed instance **V**.
- The plugin validates the batch: `event_id` must be an integer and strictly increasing, otherwise the batch is treated as a failed read (`hcom.ts:109-121`) **V**.

### 2.2 Push signal into the plugin

When the plugin binds, it opens a localhost TCP listener (`createServer(...).listen(0,"127.0.0.1")`) and registers its port with `opencode-start --notify-port P`. A connect-and-close on that port triggers `deliverPending("notify")` (`hcom.ts:147-159, 461-468`; `src/hooks/opencode.rs:248-251`, `upsert_notify_endpoint(…,"plugin",port)`) **V**. Senders reach the port through hcom's generic `notify::wake` (overview, "Local transport"). `opencode-status --status listening` also fires a wake on a transition into `listening` (`plugin_rpc.rs:113-115`) **V**. While mail is quiet and the session is idle, the plugin spawns no `hcom` process: the 5 s reconcile timer calls only the in-process `client.session.status()` (v2: nothing, `status()` returns null), and it writes a status only when the status changed (`hcom.ts:356-375, 679-693`) **V**. A working session still spawns one `hcom` process per tool call for status reporting (5).

### 2.3 Body delivery: OpenCode 1 (`client.session.promptAsync`)

```ts
// hcom.ts:1029-1037
const result = await client.session.promptAsync({
  path: { id: sessionID },
  body: { agent, model: {providerID, modelID}, variant, parts: [{ type: "text", text }] },
})
```

- **What:** a real user message (no `synthetic` flag, no `noReply`) carrying the whole formatted batch as one text part. It is persisted in session history and shown in the TUI as a user turn (I, from primer §3: `prompt_async` writes a user message; parts without `synthetic` are not hidden).
- **Agent and model preservation.** The plugin echoes the session's current agent and model, captured from `chat.message` (`hcom.ts:820-834`). The related fix is CHANGELOG.md:357, 0.7.21, "Kept OpenCode model variants during delivery" (#60/#61) **V**. That `promptAsync` would otherwise fall back to the default agent and model is INFERRED.
- **When it fires.** On every notify wake, and on an idle edge (`session.status` → idle, `hcom.ts:753-763`), resync, resume or session start. It is **not gated on busy**: `deliverPending` checks only `deliveryInFlight`, an outstanding `delivery`, and `blocked()` (a pending permission or question) (`hcom.ts:534-548`) **V**. A message sent mid-turn therefore becomes a queued user message that OpenCode 1's running loop picks up at the next step boundary (I, from primer §3, "Busy-time prompts are steered in", read but not run).

### 2.4 Body delivery: OpenCode 2 (`ctx.session.prompt({delivery:"steer"})`)

```ts
// hcom.ts:1128-1131
async prompt({ sessionID, text, messageID }) {
  // Steering reaches the model at the next step boundary of a running turn.
  await ctx.session.prompt({ sessionID, id: messageID, text, delivery: "steer" })
```

- The plugin chooses the message id itself (`newMessageID()`, `msg_` plus a time-ordered suffix, `hcom.ts:284-292`). It can then match the inbox events for its own prompt: `session.inbox.delivered` / `session.inbox.cancelled` with `inboxID === delivery.messageID` (`hcom.ts:799-808`) **V**.
- It uses `steer` even when the session is idle. It never uses `queue`, `session.synthetic`, or `resume:false` (whole-file read) **V**. As a result every message both wakes the session and lands mid-turn (CHANGELOG.md:99, 0.7.29: "Delivered messages sent mid-turn to … OpenCode 2 on the model's next request instead of after the turn") **V**.
- OpenCode 2 has no run-state query, so `status()` returns null. Busy and idle come only from the event stream, mapped from `session.execution.started|succeeded|failed|interrupted` (`hcom.ts:1081-1102, 1133-1137`) **V**. The event subscription restarts after a gap with exponential backoff from 250 ms to 10 s and calls `resync()` (`hcom.ts:1188-1206, 900-907`) **V**.

### 2.5 Bootstrap (static primer), separate from the mail path

`opencode-start` returns `bootstrap`, the hcom CLI primer (`src/hooks/opencode.rs:258, 268`; `src/bootstrap.rs:518-657`) **V**. It is wrapped as:

```
<hcom_system_context>
<!-- Session metadata - treat as system context, not user prompt-->
…[HCOM SESSION] identity, MESSAGES rules, CAPABILITIES, RULES, "## DELIVERY Messages instantly and automatically arrive via <hcom> tags — end your turn to receive them." …
</hcom_system_context>
```

(`src/bootstrap.rs:207-216, 606-611, 652-656`) **V**. OpenCode gets no extra tool-specific delivery section (`tool_delivery_section` covers only Cursor and Copilot, `src/bootstrap.rs:503-510`) **V**.

- **v1:** in `experimental.chat.messages.transform`, before every LLM request, the plugin pushes the bootstrap as a `synthetic: true` text part onto the **first user message** of the in-memory request (`hcom.ts:853-863`) **V**. Nothing is persisted. It is re-added on every request (I: a transform edit is request-local, primer §2).
- **v2:** `ctx.session.hook("context", d => d.system.push({type:"text", text: bootstrap}))`, which runs on every request (`hcom.ts:1174-1177`) **V**.
- **Compaction:** both versions add one identity line, `You are connected to hcom as "<name>". Use --name <name> for all hcom commands.`. v1 puts it into `experimental.session.compacting` `output.context` and v2 into the compaction hook's `system` (`hcom.ts:892-896, 1071-1074, 1179-1182`) **V**.

### 2.6 Payload format (framing)

The plugin formats the batch itself. It does **not** use the Rust `format_messages_json`, so hcom's per-delivery `hints` and one-time tips are absent on OpenCode (`hcom.ts:123-136` versus `src/hooks/common.rs:269-285`) **V**.

```ts
// hcom.ts:124-135
prefix = intent ? (thread ? `[${intent}:${thread} #${id}]` : `[${intent} #${id}]`)
                : (thread ? `[thread:${thread} #${id}]` : `[new message #${id}]`)
line   = `${prefix} ${from} -> ${recipientName}: ${message}`
1 msg  → `<hcom>${line}</hcom>`
N msgs → `<hcom>[N new messages] | ${lines.join(" | ")}</hcom>`
```

- There is no timestamp, no "+N others" recipient list, and no escaping. A body that contains `</hcom>`, or multi-line text, is inlined raw (V, code; any confusion in the model is I).
- The model receives the batch as **user-role text**. It can tell the batch from the human only by the `<hcom>` tag and the bootstrap's routing rule ("hcom message (<hcom> tags …) → run `hcom send` to respond", `src/bootstrap.rs:75-88`) **V**.

### 2.7 Size bounds

At most 50 messages per batch. `has_more` triggers another delivery right after the ack (`hcom.ts:610`) **V**. The per-message cap is 1 MiB (`MAX_MESSAGE_SIZE`, `src/shared/constants.rs:22`). Neither the plugin nor the RPC applies a byte cap or truncation, so one prompt can be up to about 50 MiB (I, from the constants). The only truncation in the plugin applies to tool-activity status arguments, at 2000 characters per string value (`hcom.ts:377-385`) **V**.

### 2.8 Dedupe and acknowledgement (admission state machine)

At most one delivery is outstanding (`delivery`), as a `{text, ackId, hasMore, messageID, sentAt, admitted}` record (`hcom.ts:270-280`) **V**.

1. **Read, then hand off.** `delivery` is set *before* `prompt()` returns, so an admission that arrives early is still matched (`hcom.ts:561-562`) **V**.
2. **Admission.** On v1, admission is when a later `messages.transform` sees a user part whose text equals `sent.text` (`hcom.ts:875-878`). On v2, it is `session.inbox.delivered` for the plugin's own `messageID` (`hcom.ts:799-800`) **V**. "Acked only once it is admitted … so a rejected or dropped prompt stays unread and is redelivered" (`hcom.ts:270-272`) **V**.
3. **Ack.** `flushAck` calls `opencode-read --ack --up-to ackId` and checks that `acked_to` matches. Acks are serialised. A failed ack leaves the delivery outstanding, which blocks new deliveries, and reconcile retries it (`hcom.ts:593-617, 686`) **V**.
4. **Prompt error.** `promptAsync` resolves with `error`. The delivery is cleared and *not* retried at once. The next wake or idle edge retries it, so a persistently failing prompt (missing auth, say) is not resent in a loop (`hcom.ts:576-583`; test `tests/plugin_delivery.test.ts:170`) **V**.
5. **Silent drop.** A delivery that stays unadmitted while the session sits idle past `ADMIT_GRACE_MS` = 10 s is released and redelivered. This happens at most `MAX_UNADMITTED_RETRIES` = 3 times in a row; after that only the next wake retries (`hcom.ts:13-17, 628-642`; test `:182`) **V**.
6. **v2 cancel.** `session.inbox.cancelled` for the plugin's id clears the delivery without redelivering it, so a user who removes a queued hcom prompt keeps it removed (`hcom.ts:802-808`) **V**. INFERRED: the cursor is not advanced, so that mail is offered again on the next wake.
7. **Cursor start.** On a fresh bind the cursor is set to the launch event or the current tail *before* the status flips to listening, so a new agent never receives the global backlog (`src/hooks/opencode.rs:219-221`; `plugin_rpc.rs:64-82`) **V**.
8. **Guarantee:** at-least-once. INFERRED: a plugin or host crash between admission and ack redelivers the same batch to a new process. The model then sees duplicate text, because no message-id dedupe exists on the receiving side.

### 2.9 The "fill a body-free wake" path

A typed or submitted wake whose text is `<hcom>`, `<hcom></hcom>`, or any `<hcom>…</hcom>` without `": "` (`isWakeText`, `hcom.ts:138-145`) is replaced with the real batch:

- **v1:** inside `messages.transform`, on the last user message. The filled text is cached per part id in `filledWakes`, because v1 transform edits are request-local and must be re-applied to every later request (`hcom.ts:325-327, 846-873`; test `:229-241`). Admission happens immediately (`claimWake` → `admit("wake_fill")`, `hcom.ts:644-674`) **V**.
- **v2:** inside `ctx.session.hook("prompt")`, *before* the prompt is stored (`draft.prompt.text = filled`), so the body persists. Admission comes on `inbox.delivered` (`hcom.ts:881-886, 1169-1173`) **V**.

## 3. Wake mechanism

### 3.1 Exact mechanism

The wake and the body are the same act: `promptAsync` (v1) or `session.prompt({delivery:"steer"})` (v2) creates a user message, and OpenCode starts the loop for it (V for the call; I that `prompt` starts execution when idle, from primer §3/§9). No keystrokes, no PTY, no composer.

**Exception: the first session.** OpenCode creates no session until the first prompt (`hcom.ts:909-910`) **V**. hcom handles mail that arrives before then as follows:

- **v1 "standby":** the plugin opens a second TCP port and registers it as the instance's `session_start` endpoint (`opencode-standby`, `src/hooks/opencode.rs:278-304`) **V**. When mail is pending and the PTY screen shows the ready pattern `ctrl+p commands` (`src/integration_spec.rs:1538`), the PTY delivery thread wakes that port (`src/delivery.rs:1427-1435, 1791-1796`) **V**. The plugin then calls `client.session.create`, followed by `client.tui.publish({type:"tui.session.select"})` to show the new session, binds it, and delivers (`hcom.ts:911-937, 1053-1064`) **V**. The wake repeats on each pass until the session binds (`src/delivery.rs:1421-1426`) **V**.
- **v2:** the server plugin "cannot show" a session, so it registers no standby port (`hcom.ts:236-237`; v2 transport has no `startSession`). The PTY thread then **types** a body-free preview, `<hcom>[intent:thread #id] sender -> me</hcom>` (or a bare `<hcom>` when the preview would contain `@` or exceed the input width), sleeps 800 ms and presses Enter (`src/delivery.rs:1436-1460, 584-603`) **V**. The plugin's `prompt` hook fills in the body (2.9).

### 3.2 Idle and busy detection

- **v1:** `session.status` events (`hcom.ts:753-763`), plus a 5 s reconcile through `client.session.status()`, where absence from the map means idle (`hcom.ts:1040-1045, 679-693`) **V**.
- **v2:** execution lifecycle events only (`hcom.ts:1085-1090`) **V**.
- **Approval gate.** `permission.asked` and `question.asked` (when not `blocking:false`) add entries to `pendingPermissions` / `pendingQuestions`. While any entry is outstanding, delivery is deferred (`deliveryPending=true`) and replayed on `*.replied` (`hcom.ts:765-798`; test `:205`) **V**. An idle edge clears both sets so a missed reply cannot gate delivery forever (`hcom.ts:695-703`) **V**.
- **Status reporting.** Status goes back to hcom as `listening`, `active` or `blocked:approval` (`hcom.ts:342-347`). Other agents and the hcom TUI see these states, and `hcom events --idle` uses them (I).
- **Busy is not a delivery gate.** Busy only matters for the grace-period release (`releaseUnadmitted` requires `!sessionBusy`, `hcom.ts:634`) **V**.

### 3.3 Draft safety

- **Plugin path:** safe by construction. The plugin never calls `tui.appendPrompt`, `submitPrompt` or `clearPrompt` (grep over `hcom.ts`: 0 hits) **V**. A server-side prompt does not touch the TUI-local composer (primer §1, VERIFIED there by structure). A human draft survives a delivery. INFERRED: the human sees an hcom user turn appear and the agent start working while they type.
- **v1 standby:** `tui.session.select` switches the displayed session on a fresh launch. Whether a draft typed on the home screen survives the switch is unknown.
- **v2 typed bootstrap wake: not draft-safe.** With every gate off (`require_prompt_empty:false`, `block_on_user_activity:false`, `src/integration_spec.rs:1555-1562`), `start_plugin_session` checks only `screen.ready`. It then injects the text and presses Enter after a fixed 800 ms, with no submit verification (`src/delivery.rs:1436-1450`) **V**. INFERRED: a human who has started typing their first prompt in a fresh OpenCode 2 TUI when mail arrives gets the preview appended to the draft, and the combination is submitted. Because the result no longer starts with `<hcom>`, `isWakeText` rejects it and the body is never filled. The model sees the human's partial text plus a metadata line, and the mail is delivered only on the next wake.

### 3.4 Latency

- Sender `hcom send` → TCP connect to the plugin port → `deliverPending` → one `hcom opencode-read` process spawn → `promptAsync`. INFERRED: sub-second end to end, dominated by the process spawn.
- Mid-turn, the model sees the message at the next step boundary, after the current LLM call and tool round (primer §3/§9; CHANGELOG.md:99).
- Recovery paths are slower: 5 s reconcile, a 10 s admission grace, v2 stream backoff up to 10 s, and the PTY idle wait of 30 s between passes when no notify arrives (`src/delivery.rs:1462`) **V**.

### 3.5 Failure modes visible in code

- **`hcom` missing from the OpenCode process's PATH:** every RPC returns exit 127 and only `hcom.log` records it (`hcom.ts:73-76, 88-94`). There is no user-visible signal from the plugin (V; whether hcom status surfaces it elsewhere is unknown).
- **The plugin did not load** (`--pure`, config error): nothing delivers. The PTY loop treats OpenCode as plugin mode regardless (`src/delivery.rs:1690-1708`) **V**. INFERRED: no PTY fallback, so mail stays unread.
- **OpenCode 2 shared background service:** plugins there "never see hcom's launch env". hcom forces `--standalone`, one private server per launch (`src/tools/opencode_preprocessing.rs:9-11, 31-53`) **V**. hcom also fixed "OpenCode 2 agents not receiving messages after about an hour idle" (CHANGELOG.md:73, 0.7.30, #161). That bug ties in with the location-eviction plugin reload that `lastBinding` on `globalThis` handles (`hcom.ts:243-247`) (I for the link).
- **A second plugin copy in the same process** (inherited `OPENCODE_CONFIG_CONTENT`): `claimPluginHost` makes it inert (`hcom.ts:19-25`). The launcher strips `HCOM_PLUGIN_HOST_PID` for legitimate new hosts (`src/hooks/runtime.rs:26-29`; `src/launcher.rs:509`) **V**.

## 4. Setup and installation footprint

**Per run, through an environment variable. hcom writes no OpenCode config file.**

- `prepare_per_run` publishes the plugin source as `index.ts` in a content-addressed, write-once, owner-only directory under `<HCOM_DIR>/integrations/opencode/<digest>/` (digest: SHA-256 truncated to 16 bytes, `runtime.rs:422-433`). It is a directory because OpenCode 2 rejects a plugin path that is a file (`src/hooks/opencode.rs:622-629`; `src/hooks/runtime.rs:417-419, 441-480`) **V**. Digests unused for `ARTIFACT_MAX_IDLE` (30 days) are swept when a new digest is published (`runtime.rs:451-452, 487, 496`) **V**.
- It sets `OPENCODE_CONFIG_CONTENT` to the caller's own `OPENCODE_CONFIG_CONTENT` (JSONC accepted, key order preserved, because OpenCode resolves permission rules by position) with the `file://` plugin URL appended to `"plugin"` and any stale hcom plugin path removed (`src/hooks/opencode.rs:604-678`) **V**.
- **Auto-approve.**
  - On OpenCode 2 with `auto_approve` on, it appends `permission.bash["<hcom-cmd> <cmd>*"]="allow"` (prefix from `build_hcom_command()`, normally `hcom`) for each `SAFE_HCOM_COMMANDS` entry (`send`, `start`, `list`, `events`, `listen`, …) after the caller's rules (`src/hooks/opencode.rs:636-639, 680-706`; `src/hooks/common.rs:52-64`) **V**.
  - On OpenCode 1 it writes no rules, because a deep-merge would replace a user's `"bash":"allow"`. Instead the plugin answers a `permission.asked` for exactly one safe hcom command with "once" through `opencode-approve` (`hcom.ts:705-722`; `src/hooks/opencode.rs:708-718`). This is POSIX only (CHANGELOG.md:104) **V**. Before 0.7.20 every `hcom` command was approved, `kill` and `reset` included (CHANGELOG.md:368) **V**.
- **Legacy cleanup.** On every launch hcom deletes old hcom-owned `hcom.ts` files from `{plugin,plugins}/` in `~/.config/opencode`, `OPENCODE_CONFIG_DIR`, `~/.opencode` and each `.opencode/` from cwd up to the git root. Ownership is judged by content: an exact source match, the `// hcom-managed-plugin` marker, or `HcomPlugin` plus `HCOM_DIR` (`src/hooks/opencode.rs:720-781, 872-884`) **V**. This is the only write into OpenCode's config tree, and it only deletes. Global installs ended in 0.7.27 (CHANGELOG.md:172) **V**.
- **Launch arguments on OpenCode 2:** `--agent` and `--model` move into `HCOM_OPENCODE_AGENT` / `HCOM_OPENCODE_MODEL`, because the 2.x TUI lacks those flags. The plugin applies them through `ctx.agent.transform` and through `session.switchModel` on each session's first prompt (`src/tools/opencode_preprocessing.rs:14-20`; `hcom.ts:1148-1173`) **V**. `--session X --fork` is forked server-side (`opencode_preprocessing.rs:31-53`) **V**.
- **Version probe.** The 1.x/2.x branch runs `opencode --version` once (cached) and parses the major version, defaulting to 1 on failure (`src/tools/opencode_preprocessing.rs:180-200`) **V**. It selects argument and config shapes at launch. It is a gate on behaviour, not on admission.
- **Wrapper.** OpenCode runs inside `hcom pty` (`uses_pty_default: true`, `src/integration_spec.rs:1571`) **V**. `HERDR_AGENT` is set so Herdr can still classify the pane (overview) **V**.

## 5. Token cost

**Per message:**

- Envelope: `<hcom>` … `</hcom>` plus `[intent:thread #id] from -> to: `, roughly 12-20 tokens. Batches add `[N new messages] | ` and ` | ` separators (I, from the `hcom.ts:124-135` format).
- No per-delivery hints and no tips, which is cheaper than hcom's Claude and Codex path (V, 2.6).
- Each delivery is a **new user turn**, so it costs a full model turn: a reasoning or reply step over the cached context. There is no priority, threshold or silent mode. Every message wakes or steers (V: the only prompt call uses `steer`, no `noReply` / `resume:false`; overview "Wake policy").

**Per quiet turn:**

- **Bootstrap:** about 3.5 KB of static template (sections at `src/bootstrap.rs:68-126, 207-216` measure about 3.5 KB) plus an active-agent snapshot, about 900 tokens (I; the README claims about 700, README.md:231). It is re-sent on **every** LLM request: v1 as a synthetic part on the first user message, v2 through `system.push`. Its text is stable within a session, so provider prompt caching should absorb most of the cost (I). It is still a per-request system-level tax, where a skill would be loaded on demand.
- **No mail-related injection** on a quiet turn (V: transform only fills wakes or adds the bootstrap).
- **Process overhead per tool call:** `tool.execute.before` *awaits* `reportTool`, which spawns `hcom opencode-status --tool …` for every tool call (`hcom.ts:978-981, 380-396`; v1 hook `:1069`, v2 `ctx.tool.hook("execute.before")` `:1178`) **V**. On v1 server hooks are awaited sequentially inside the turn (primer §2). INFERRED: every tool call carries one hcom process start, up to the 15 s timeout if hcom hangs. This costs latency, not tokens.

## 6. Limitations and bugs

1. **Every message wakes.** No priority or threshold and no silent staging. OpenCode 2's native `delivery:"queue"`, `resume:false` and v1 `noReply:true` all exist and go unused (V absence in `hcom.ts`; the APIs are in primer §3/§9). This conflicts directly with our delivery-contract rules 1-2.
2. **Mid-turn steering is unconditional.** A p1-equivalent FYI extends a busy turn, which costs a step and can derail the current task (I).
3. **One OpenCode session per instance; the first bind wins.** `bindIdentity` returns early once `instanceName` is set (`hcom.ts:445`), and every event is filtered by `isBoundSession` (`hcom.ts:750`) **V**. INFERRED: after the human runs `/new` or switches sessions in the TUI, hcom keeps prompting the original session, which is no longer on screen. Mail then wakes a session the human is not watching. Child (task) sessions are correctly excluded (CHANGELOG.md:109) **V**.
4. **v1 filled wakes live only in memory.** `filledWakes` is an in-memory map, cleared on `reset()` (`hcom.ts:327, 438`) **V**. INFERRED: after a plugin or process restart, a historical `<hcom>` wake part reverts to bare `<hcom>` in the model's view of history, so the body that was delivered is lost from context. If that bare wake is the last user message, `claimWake` fills it with the *next* batch instead. This only affects v1 wakes typed by humans or scripts (`hcom term`, `hcom run onidle`), not the `promptAsync` path, which persists the full text.
5. **A bare `<hcom>` reaches the model unexplained** when `claimWake` finds no mail, or a delivery is already outstanding (`hcom.ts:651, 655`) **V**. The OpenCode bootstrap has no "a bare `<hcom>` is a wake trigger" rule (`src/bootstrap.rs:503-510`) **V**. INFERRED: the model may answer an empty wake.
6. **`isWakeText` can swallow human text.** Any human prompt of the form `<hcom>…</hcom>` without `": "` is replaced by queued mail (`hcom.ts:141-145`) **V**. This is an edge case.
7. **Not draft-safe on the v2 first-session wake** (3.3) **V/I**.
8. **No byte cap** on a delivery: 50 × 1 MiB (2.7). Bodies are not escaped (`</hcom>` passes through raw) (2.6) **V**.
9. **Unauthenticated localhost notify ports** (the plugin port and the standby port). A wake only triggers a read, never carries text, so the impact is limited to spurious reads or session creation (V for no auth, `hcom.ts:147-159`; impact I).
10. **Version-keyed launch shape** through `opencode --version` (4). It works, but our own rules forbid a probe as an operational prerequisite. hcom defaults to 1.x on failure, which mis-launches a 2.x install whose `--version` fails (I).
11. **The bootstrap is re-sent on every request** instead of once (5).

## 7. Clever ideas worth stealing

1. **Ack on admission, not on hand-off.** Acknowledge only when the host proves the model saw it: the text appears in a v1 model request, or a v2 `session.inbox.delivered` event arrives for the plugin's own pre-chosen message id. Release a delivery unadmitted after a 10 s grace, with bounded retries (`hcom.ts:270-280, 593-642, 799-808`). This is stronger than "the hook printed it", and it maps directly onto our delivery ledger.
2. **Pre-chosen OpenCode 2 message ids** (`newMessageID`) let the plugin correlate inbox events with its own prompts, separate from the human's. It also lets the plugin respect a human's `inbox.cancelled` (`hcom.ts:284-292, 802-808`).
3. **Gate on approval prompts, not on busy.** Defer delivery while a permission or question is pending, replay on reply, and clear the gates on idle so a missed reply cannot block forever (`hcom.ts:695-703, 765-798`). Our OpenCode adapter needs exactly this, because a server-side prompt during a permission dialog is the one composer-adjacent hazard left.
4. **Per-run injection via `OPENCODE_CONFIG_CONTENT`** with an order-preserving JSONC merge of the caller's value and a content-addressed plugin directory (`src/hooks/opencode.rs:604-678`; `runtime.rs:441-480`). It writes zero files in `~/.config/opencode`, and the plugin disappears with the launch. It fits our "never write real user config" rule for tests and gives users an opt-in launch path.
5. **`--standalone` on OpenCode 2.** The 2.x shared background server does not see per-launch env or plugins. Any env-keyed identity we design for OpenCode 2 must account for this (`opencode_preprocessing.rs:9-11`).
6. **A single-host claim** via an env var holding the PID (`hcom.ts:19-25`), so an inherited config cannot run two delivery engines in one process.
7. **Rebind across plugin reloads**, from a record on `globalThis` keyed by (directory, workspace). OpenCode 2 reloads plugins when it evicts an idle directory and sends no session event (`hcom.ts:243-268`; tests `:304-546`). That is a real 2.x hazard for any long-idle seat (CHANGELOG.md:73).
8. **Fill a content-free wake in the host's own hook.** v2's `prompt` hook rewrites the wake before it is stored, so the body persists and the typed text stays metadata-only (`hcom.ts:1169-1173`). This is the same shape as our R18 content-free wake, but done host-side.
9. **Echo agent and model on plugin-initiated prompts** (`hcom.ts:1025-1037`), so an injected turn does not silently switch the user's model or agent.
10. **For contrast, what not to copy:** the per-request bootstrap (we keep the static text in the skill), unconditional `steer`, and spawning a CLI process on every tool call. We should map `p <= T` onto `steer` plus wake, and `p > T` onto `queue` / `resume:false` (v2) or `noReply:true` (v1). That gives OpenCode the full delivery contract natively, which hcom never attempts.

## Verification

Adversarial pass on 2026-10-23 against clone HEAD `de33e31a0cdb` and the GitHub API.

- Checked: every cited `hcom.ts` range (RPC spawn and timeout, batch validation, framing, `isWakeText`, notify server, `Delivery` record, `newMessageID`, `deliverPending` gates, `flushAck`, `admit`, `releaseUnadmitted`, `claimWake`, reconcile, `setBusy`, approval gating, inbox events, transform and compaction hooks, standby, v1 `HcomPlugin`, v2 `setupOpenCode2`, event backoff, default export); `integration_spec.rs` (OPENCODE spec, gates, hook names, Kilo sharing, `uses_pty_default`); `hooks/opencode.rs` (gate, `require_attached`, start/standby, `include_str!`, per-run config, auto-approve, `handle_approve`, legacy cleanup roots and ownership test); `plugin_rpc.rs`; `common.rs`; `constants.rs`; `delivery.rs` (plugin mode, `start_plugin_session`, 800 ms Enter, preview builder, `IDLE_WAIT`); `bootstrap.rs`; `runtime.rs`; `launcher.rs:509`; `opencode_preprocessing.rs`; CHANGELOG lines 73/99/104/109/172/204/357/368 and their release sections; README:231; test names in `tests/plugin_delivery.test.ts`; stars 597, forks 94, MIT, v0.7.31 on 2026-10-21 (API). All held.
- Changed: 2.2 "spawns no hcom process while mail is quiet" narrowed to an idle session (tool calls spawn one each); 2.3 CHANGELOG:357 is the model-variant fix, the fallback rationale is now INFERRED; 4 digest is a 16-byte-truncated SHA-256 under `<HCOM_DIR>`, sweep is 30 days, auto-approve prefix comes from `build_hcom_command()`; 5 per-tool-call spawn also happens on v2 (`hcom.ts:1178`).
