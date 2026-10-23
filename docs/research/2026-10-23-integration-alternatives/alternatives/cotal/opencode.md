# Cotal x OpenCode: message injection and wake

- Alternative: Cotal, https://github.com/Cotal-AI/Cotal (site https://cotal.ai/). Clone `alts/cotal` (scratchpad), HEAD `6426fb538e0b` (2026-10-23). Read only; nothing executed.
- Harness: OpenCode (anomalyco/opencode, formerly sst/opencode). Primer: `../../primers/opencode.md`. Where OpenCode's own behaviour is cited, the source is the v1.18.1 tag clone in the scratchpad (`harness-src/opencode-1.18.1`), paths relative to its root.
- Citation form: `src/...` is relative to `extensions/connector-opencode/`; bare `agent.ts`, `control.ts`, `framing.ts`, `orientation.ts`, `tool-specs.ts` are in `extensions/connector-core/src/`; `docs/...`, `packages/...` and `extensions/...` are relative to the Cotal clone root; `[oc 1.18.1]` paths are relative to the OpenCode clone. **V** = VERIFIED (read in source or docs), **I** = INFERRED.

## 1. Support status

| Item | Finding | |
|---|---|---|
| Status | Native connector, **beta** (docs/connectors.md:23; docs/connect-opencode.md:1-12) | V |
| Lines | OpenCode 1.x (`opencode-ai` >= 1.16) and 2.x (`@opencode/cli` >= 2.0). The line is detected from `opencode --version` at spawn; any other major is refused (docs/connect-opencode.md:20-22; extensions/connector-opencode/src/opencode-line.ts:4-11; src/launch.ts:186-187) | V |
| Code | 1.x: `src/plugin.ts` (1,542 lines). 2.x: `src/plugin2.ts` (454 lines). One bundle exports both: `export default { id: "cotal", server: cotal, setup: setupCotal }` (src/plugin.entry.ts:10). Launcher shim: `src/serve.ts`, `src/launch.ts`. Spawn spec: `src/extension.ts` | V |
| Who can join | **Only sessions Cotal launches.** The plugin is inert without `COTAL_NAME` etc.: `if (!hasIdentity()) { log("no COTAL_NAME — not a managed session; staying off the mesh"); return {}; }` (src/plugin.ts:200-204; plugin2.ts:92). Nothing is installed in the user's OpenCode config (section 4), so a user's own `opencode` TUI cannot join | V |
| Missing | No session resume (`--resume` throws, issue #154), no tool-sharing (`connectors.opencode.mcpServers` throws), no mid-turn steering, 2.x has no event plane (docs/connect-opencode.md:195-209; docs/connectors.md:28-30, 33; src/extension.ts:147-167) | V |

Architecture in one line: `cotal spawn --agent opencode` runs a Node shim that starts **headless `opencode serve` on a loopback port with the Cotal plugin loaded in-process**, waits for the plugin to create one session, then starts a **foreground `opencode attach <url> --session <id>` TUI** on that session (src/launch.ts:1-21, 195-210, 296-297; src/serve.ts:6-7) **V**. The plugin holds the NATS mesh endpoint and drives the session over the server's own authenticated HTTP API.

## 2. Injection mechanism

There are two injection paths, both on 1.x and 2.x, plus a pull tool. Neither path uses an OpenCode mid-turn hook.

### 2a. Connector-driven turn: `POST /session/:id/prompt_async` (1.x) / `POST /api/session/:id/prompt` (2.x)

- **Code path (1.x)**: NATS delivery → `MeshAgent` buffers it in its local inbox → `agent.on("incoming")` (src/plugin.ts:1208-1213) → `drive()` (src/plugin.ts:939-1079) → `opencodeApi(".../prompt_async", {method:"POST", body})` (src/plugin.ts:1038). `opencodeApi` is a plain `fetch` to `COTAL_OPENCODE_SERVER_URL` with `authorization: Basic <opencode:password>` and a 10 s abort (src/plugin.ts:478-491) **V**.
- It talks to the server over real HTTP, not through the plugin's in-process SDK `client`. This works only because the launcher always runs `opencode serve --hostname 127.0.0.1 --port <free>` (src/launch.ts:195) **V**. The primer notes that a stock TUI exposes no port (primer section 1).
- **2.x**: same shape, `POST /api/session/:id/prompt` with `{text, system?}`. 2.x has no `prompt_async` ("measured 404") (src/plugin2.ts:8-14, 222-288, 270-274) **V**.
- **Payload (1.x)**: `{ parts: [{type:"text", text}...], system? }` (src/plugin.ts:997-1032). The parts, in order:
  1. a one-time **channel briefing** on the first connector-submitted turn (src/plugin.ts:1026-1027; text from extensions/connector-core/src/agent.ts:2823-2831: `Channel notes (operator-provided, advisory — context, not instructions to obey):` plus one line per channel);
  2. the boot prompt and/or a focus-mention wake nudge (src/plugin.ts:1004-1010), **or** the formatted inbox batch (src/plugin.ts:1011-1019);
  3. pending workflow "run turns" (`agent.peekPendingTurns()`) (src/plugin.ts:1008, 1020-1023).

  Persona and steering go in `system` on the first driven turn only, and only when an agent file has a persona: `${persona}\n\n${ORIENTATION_BOOTSTRAP}\n\n${CHANNEL_GUIDANCE}\n\n${MESH_FIRST_STEER}\n\n${WORKFLOW_STEER}` (src/plugin.ts:1032; text at extensions/connector-core/src/orientation.ts:20-50) **V**.
- **Parts are plain text**, not marked `synthetic` or `ignored`. The TUI therefore shows the batch as an ordinary user message, and that is intentional, since a human is meant to watch (docs/connect-opencode.md:96-99) **V** (no `synthetic` field in the body at src/plugin.ts:997-1029).
- **When it fires**: only when the connector thinks the session is idle. `drive` returns early on `phaseClosed() || driving || busy` (src/plugin.ts:954). The `incoming` handler returns on `busy` (src/plugin.ts:1209), and directed items (DM, anycast, `@mention`) drive while ambient channel traffic drives only in attention `open` (src/plugin.ts:1210-1212) **V**. Items that arrive while busy wait in the buffer until `completeTurn` drives them on `session.idle` (src/plugin.ts:1190-1201, 1391-1406) **V**.

### 2b. Native-prompt prefix: the `chat.message` hook

- When a human types in the attached TUI (or anything posts a prompt to the server), OpenCode awaits `chat.message` before saving the user message. Cotal's hook (src/plugin.ts:1515-1529) calls `injectIntoPrompt(output)` (src/plugin.ts:1165-1181), which **prepends the pending automatic batch to the human's first text part**:

  ```ts
  if (driving || awaitingTurnEnd) return; // drive() already injected, or one surfaced batch is open
  const computed = computeInjection(agent);
  ...
  textPart.text = `${computed.prefix}\n\n${textPart.text}`;
  ```

  `computeInjection` is shared by 1.x and 2.x (src/plugin.ts:185-197). The 2.x equivalent is `ctx.session.hook("prompt", e => { e.prompt.text = prefix + "\n\n" + e.prompt.text })` (src/plugin2.ts:343-349) **V**.
- The result is durable. The prefix becomes part of the persisted user message, so peer text and human text share one record with no boundary. Cotal notes this as the reason its event plane never publishes user text (docs/connect-opencode.md:167-171) **V**.

### 2c. Pull: `cotal_inbox`

- On OpenCode, `cotal_inbox` pulls and clears **only quiet-channel ambient** (plus a read-only focus recall). Automatic traffic is owned by the driver (src/tools.ts:7-8, 23-36; docs/connect-opencode.md:120-122) **V**. There are 24 `cotal_*` tool specs (20-24 advertised, depending on capabilities), registered as native plugin tools from shared specs (src/tools.ts:20-60; tool-specs.ts:683-705, 1879-1881) **V**.

### Not used

None of the following appear in plugin.ts or plugin2.ts (grep) **V**:

- `experimental.chat.messages.transform` (per-step, mid-turn);
- `tool.execute.after` (post-tool output append);
- `noReply` (silent staging);
- `experimental.chat.system.transform`;
- `/tui/append-prompt`;
- keystrokes.

The other 1.x hook keys are `tool` (the `cotal_*` tools), `tool.execute.before` (for presence, src/plugin.ts:1499-1502), `event` and `dispose` (src/plugin.ts:1303, 1505-1536); 2.x adds a `model.request` hook for model reporting (src/plugin2.ts:354). The connector matrix says "Mid-turn steering: ✗" for OpenCode (docs/connectors.md:28) **V**.

### Framing

`formatInjection` (extensions/connector-core/src/control.ts:120-127) **V**:

```
📨 Cotal — 2 new messages from peers:
[DM from ada (reviewer)] body line 1
  body line 2
[#build @you bob] ...
(If this asks for work, do it with your own tools first and verify the result, then reply with cotal_dm / cotal_send — never report an action you did not actually perform. cotal_roster shows who's here.)
```

- Item lines come from `formatItem` (extensions/connector-core/src/framing.ts:124-132): `[DM from X]`, `[@service from X]`, `[#chan @you X]`.
- **Column-zero rule**: every line break class (including U+2028, U+2029 and U+0085) in a body is re-indented, so only the connector writes at column 0 (framing.ts:64, 96-98).
- Names and labels have brackets and newlines stripped (framing.ts:80-82).
- A broker-counted marker is added on redelivery: `(redelivered, delivery N: an earlier delivery may already have run)` (framing.ts:127).
- A backfill marker: `(history)` (framing.ts:125).
- The tail sentence was added because a weak model reported work it had not done (control.ts:112-119 comment) **V**.

### Size bounds

- **No per-batch count or byte cap and no truncation on the OpenCode path.** `drive` injects `agent.peekInbox("automatic")` whole (src/plugin.ts:1012-1017; agent.ts:1374-1376) **V**. The only bounds are these:
  - the local inbox buffer, `MAX_INBOX = 200`, with overflow eviction (agent.ts:227, 1143-1164);
  - the in-flight hold ceiling, `MAX_IN_FLIGHT = 400`; past it, `surface()` refuses and the batch waits (agent.ts:232, 1392-1405; src/plugin.ts:1033);
  - the broker's NATS `max_payload` per published message (stock NATS default 1 MiB; deployment-configurable) **I**. No chat-path cap was found in connector-core.
- Contrast: Cotal's Claude native path caps at 8 messages, 1,024 code points each (overview, "Bounds"). The OpenCode path has no equivalent. A burst of 200 long DMs becomes a single user turn **I**.

### Dedupe and ack

- **Two-phase surface then ack.** `surface(ids)` calls `agent.holdInFlight(ids)` (src/plugin.ts:1133-1138). The held deliveries get a JetStream in-progress ack (`redelivery.working()`) every `ackWaitMs/2` (agent.ts:1439-1472). The ack wait is 60 s (docs/connect-opencode.md:107-112) **V**.
- **The ack happens only at turn end.** `session.idle` → `completeTurn()` → `ackSurfaced()` → `agent.drainInboxDeliveries(surfaced)`, which the code calls "the sole ack site" (src/plugin.ts:1116-1120, 1190-1196, 1405). For a native-prompt prefix the ack is armed by setting `awaitingTurnEnd = busy = true` (src/plugin.ts:1177-1180) **V**.
- **A failed turn** (`session.error` other than `MessageAbortedError`) leaves the batch unacked, abandons the hold and retries with exponential backoff from 1 s to 30 s (src/plugin.ts:84-85, 802-811, 1456-1476). **A user Stop** (`MessageAbortedError`, or a `tui.command.execute` `session.interrupt` within 30 s) acks the batch as dismissed (src/plugin.ts:757-775, 1441, 1458-1460, 1486-1489) **V**.
- **2.x** acks on `session.execution.succeeded` and on `.interrupted`, and abandons on `.failed`, with replay protection keyed on the event's durable log position (src/plugin2.ts:290-323, 391-412) **V**.
- **Dedupe**: there is a commit-aware cross-path dedup set (`markHandled`, agent.ts:1547-1560), and a redelivered copy is labelled for the model rather than suppressed (framing.ts:127). Delivery is explicitly at-least-once (docs/connect-opencode.md:117-119) **V**.

## 3. Wake mechanism

- **Mechanism**: the same `prompt_async` call as in 2a. It creates a user message and starts the loop on an idle session. There are no keystrokes, no composer writes and no TUI routes. The TUI is "a pure viewer" that renders the session's events (src/launch.ts:12-14; src/plugin.ts:932-938) **V**.
- **Idle/busy detection**: the connector keeps its own `busy` flag, fed by the plugin `event` hook:
  - `session.status` `busy` sets `busy = true` and presence `working`; `idle`/`retry` statuses update presence only and do not clear `busy` (src/plugin.ts:1408-1420);
  - `session.idle` is the only turn-end and ack site (src/plugin.ts:1391-1406);
  - `session.error` closes a failed turn (src/plugin.ts:1422-1477);
  - `drive` itself sets `busy` before the POST (src/plugin.ts:1034-1037).

  The busy guard exists because "opencode would COALESCE onto it (no reject)": a prompt sent to a running session joins the run in flight (src/plugin.ts:30-33, 501-502). This matches the primer's section 3 **V**. On 2.x the state is `execution: "submitted" | "running"`, driven by `session.execution.*` events (src/plugin2.ts:151-161, 391-412) **V**.
- **Draft safety**: structurally safe. The server-side prompt never touches the TUI composer (primer section 1) **V**. The plugin does not check whether a human has a half-typed draft, though. A wake starts a turn while the human types; the draft survives, and if they submit mid-turn the prompt coalesces into the running turn (src/plugin.ts:30-33) **I**. A pane-level "human is typing, hold the wake" rule does not exist here **V** (no TUI plugin, no composer read).
- **Which items wake** (the attention model): directed always wakes; ambient wakes only in `open`; quiet ambient never wakes and stays pull-only. A focus-mode `@mention` is acked and dropped at ingest, and a one-slot nudge `📨 You were mentioned by X on #chan — read it with cotal_inbox.` is driven as a turn instead. On 1.x it is held until a submission carrying it lands, and re-armed if that turn fails (src/plugin.ts:525-562, 1214-1228, 1463-1471; agent.ts:1709-1726; docs/connect-opencode.md:123-127) **V**. On 2.x there is no slot: `mention-wake` calls `drive("📨 You were mentioned — read it with cotal_inbox.")` once, and `drive` returns without retry if a turn is running (src/plugin2.ts:228, 334-336), so a focus mention arriving mid-turn gets no wake **I** (from reading). There is no numeric priority or threshold.
- **Boot floor**: an operator `--prompt` (env `COTAL_OPENCODE_PROMPT`) is the first connector-submitted turn. Peer traffic waits behind it (src/plugin.ts:514-524, 1081-1111) **V**.
- **Latency**: NATS push → `incoming` event → one loopback HTTP POST. It is event-driven with no polling, so expect milliseconds plus model time-to-first-token **I** (not measured). After a busy turn, the next batch goes out on `session.idle` with no delay (src/plugin.ts:1200) **V**.
- **Failure modes**:
  - The POST fails (server busy or down, 10 s timeout): the batch is released and retried with backoff (src/plugin.ts:1067-1075) **V**.
  - `session.idle` never arrives (the 1.x event is deprecated per the primer section 3): `busy` sticks, and every later delivery buffers forever. No watchdog was found, only the session-less `session.error` handling (src/plugin.ts:1423-1427) **I**.
  - The plugin handshake must appear on stderr within 60 s (`[cotal-session] <id>`), or the launch aborts (src/launch.ts:224-274; src/plugin.ts:881-897) **V**.
  - The launcher has to "poke" the server repeatedly (`GET /session` or `/api/config` every 200 ms, up to 300 times), because OpenCode loads plugins lazily on the first instance request (src/launch.ts:243-260) **V**.
  - An instance dispose (a global config change, `POST /instance/dispose`) ends the seat (docs/connect-opencode.md:134-138; src/plugin.ts:1531-1536) **V**.
  - `/new` in the TUI is adopted as a context reset under the same identity, with a serialized session swap (src/plugin.ts:1313-1390) **V**.

## 4. Setup and installation footprint on the harness

- **Nothing is written to the user's OpenCode config.** Config rides in `OPENCODE_CONFIG_CONTENT` (inline JSON, the highest merge layer) (docs/connect-opencode.md:75-79; src/extension.ts:119-122, 283) **V**. The content is:

  ```json
  { "$schema": "https://opencode.ai/config.json", "permission": "allow",
    "plugin": ["<cotal>/dist/plugin"],
    "command": { "reconnect": { "description": "...", "template": "Reconnecting to the Cotal mesh… Call the cotal_reconnect tool now ..." } },
    "agent": { "cotal": { "mode": "primary", "model": "...", "variant": "..." } }, "default_agent": "cotal" }
  ```

  The source is src/extension.ts:230-281. `agent`/`default_agent` are added only when a model, variant or launch options are given **V**.
- `permission: "allow"`: every spawned agent runs with all tools auto-approved (src/extension.ts:121, 232) **V**.
- **Inheritance**: because this is a merge layer, the spawned agent inherits the operator's MCP servers (docs/connect-opencode.md:76-79; src/extension.ts:157-160) **V**; that global plugins and providers load too follows from the same merge **I**. Auth/config stay on the operator's HOME/XDG roots (src/launch.ts:140-141) **V**.
- **Per-agent DB**: `OPENCODE_DB=<workspace>/.cotal/opencode/<name>/opencode.db`, plus `serve.<token>.pid` records (src/launch.ts:140-149, 194-210) **V**.
- **Server auth**: a per-launch random `OPENCODE_SERVER_PASSWORD` (24 bytes hex), passed only through the environment and never in argv. The launcher comment cites CVE-2026-22812 for the unauthenticated default (src/launch.ts:16-21, 52-55, 199-207) **V**.
- **The TUI env is stripped** of `OPENCODE_CONFIG_CONTENT` and every `COTAL_*`, so the viewer never loads a second plugin instance (src/launch.ts:287-294) **V**.
- Identity and credentials reach the plugin through env and a 0600 launch-material file, which the plugin scrubs after reading so agent-run shells never inherit them (src/plugin.ts:206-212; src/extension.ts:293-304) **V**.
- A local control socket (`COTAL_CONTROL_SOCKET`) carries cooperative shutdown (src/plugin.ts:613-621; src/extension.ts:285-291) **V**.

## 5. Token cost

**Per delivered batch (a model turn each time):**

- Fixed framing: the head `📨 Cotal — N new messages from peers:` (~40 bytes) plus the tail (~205 bytes), so ~245 bytes of fixed text per batch, before any item **V** (counted from control.ts:122-125).
- Per item: `[DM from <label>] ` (~20-40 bytes) plus the body, plus ~70 bytes when redelivered **V**.
- Each batch is a **full user turn**. The model re-reads the whole conversation (cached prefix) and must produce a response. Even an FYI channel message in `open` mode costs a turn **I**.
- First driven turn only: the channel briefing (one line per described channel) and, with a persona, a system block of persona plus about 1.2 KB of steering text (orientation.ts:20-50) **V**.

**Per quiet turn:**

- Zero injected bytes. `computeInjection` returns `undefined` when nothing is pending, so `chat.message` leaves the prompt unchanged (src/plugin.ts:190-193, 1167-1168) **V**.
- The standing cost is the tool surface: 24 `cotal_*` specs are declared, of which 20-24 are advertised (`cotal_spawn`/`cotal_persona`/`cotal_personas` need `spawn`, `cotal_run` needs `run` on an authed mesh; tool-specs.ts:700-703, 1879-1880) **V**. Their descriptions total roughly 10 KB before parameter schemas, in every LLM request **I** (approximate count, not re-measured). That is roughly 2.5k+ tokens per request, mostly prompt-cached **I**.

## 6. Limitations and bugs seen

1. **The persona and mesh steering are silently lost after the first turn (1.x). V mechanism, I impact.**
   - Cotal sends them once, as the per-message `system` field, then sets `primed = true` (src/plugin.ts:504, 1032, 1066).
   - In OpenCode, the per-message `system` applies only to requests whose **latest user message** carries it. The loop builds each request with `user: lastUser` ([oc 1.18.1] packages/opencode/src/session/prompt.ts:1096, 1272-1273), and `prepare` appends `input.user.system` ([oc 1.18.1] packages/opencode/src/session/llm/request.ts:56-62).
   - So the second batch, or the first human prompt, runs with no persona, no `ORIENTATION_BOOTSTRAP` and no `MESH_FIRST_STEER`.
   - The same pattern exists on 2.x (src/plugin2.ts:261, 276); 2.x semantics are unknown.
   - Fix: send it on every connector turn, or use `experimental.chat.system.transform`. Setting `agent.cotal.prompt` in the injected config would also persist it, but an agent prompt *replaces* OpenCode's provider system prompt ([oc 1.18.1] packages/opencode/src/session/llm/request.ts:60), which Cotal deliberately avoids (src/plugin.ts:1030-1031) **V**.
2. **There is no mid-turn delivery, even though OpenCode offers it.**
   - A DM to a busy agent waits for the whole turn to end, which can be minutes on a long tool loop (src/plugin.ts:1209).
   - `experimental.chat.messages.transform` (per step) or `tool.execute.after` would deliver at tool boundaries with no extra model call (primer section 2).
   - Cotal chose "never interrupt a running turn" to avoid coalescing (src/plugin.ts:16, 1203). That conflates "don't start a second prompt" with "don't add context". OpenCode 1.x already reads a mid-turn prompt at the next step boundary (primer section 3), and 2.x has a native `session.synthetic` with `delivery: "steer"` (primer section 9), neither of which Cotal uses.
3. **Unbounded batches.** There is no count, byte or per-body cap on the injected turn (section 2). A large peer message goes into context whole.
4. **Possible double injection on 2.x. I.**
   - The 2.x `prompt` hook has no `driving`/`awaitingTurnEnd` guard (src/plugin2.ts:343-349). Compare 1.x at src/plugin.ts:1166.
   - `peekInbox` returns held, unacked items too (agent.ts:1374-1376, 1543-1545).
   - If 2.x fires the `prompt` hook for the connector's own `POST /api/session/:id/prompt`, the batch that `drive` already wrote into `text` gets prefixed a second time.
   - Separately, a human prompt during a driven turn re-injects the same held batch, and `surface()` swaps the hold to the new id set (src/plugin2.ts:194-199).
   - Whether the 2.x hook fires for API prompts is unknown.
5. **Wake ignores the human.** No composer or focus check is made, so a peer batch can start a turn while the operator is mid-draft in the attached TUI. The text is safe, but the turn competes with what the operator is about to ask **I**.
6. **Peer text merged into the human's message** (2b). The model cannot tell which part the human typed except by the `📨 Cotal` header. The stored history permanently records the human as the author of peer content (docs/connect-opencode.md:167-171) **V**.
7. **Managed launch only.** There is no way to attach to a user's existing OpenCode session; it needs a Cotal-owned `serve` plus `attach`. There is no resume (#154), so a crashed seat's conversation is not continued **V**.
8. **`permission: "allow"` plus inherited operator MCP servers** gives every spawned agent auto-approved access to all of the operator's tools (src/extension.ts:232; docs/connect-opencode.md:76-79) **V**.
9. **Reliance on the deprecated `session.idle`** as the only 1.x turn-end and ack site (src/plugin.ts:1391-1406). If upstream stops emitting it, the seat wedges busy and nothing is acked, so everything redelivers **I**.
10. **The comment density is a maintenance signal.** About half of plugin.ts's 1,542 lines are comments (766 by a `//`/`*` line count), largely race narratives (#596, #599, #600, #715, #719) around one boolean state machine (`busy`, `driving`, `awaitingTurnEnd`, `swapping`, `stopping`, `bootPrompt`, `pendingWake`). The many reproduced races suggest the approach is fragile **I**.

## 7. Clever ideas worth stealing

1. **Server-side prompt as the wake.** Driving `prompt_async` against a `serve` instance the connector owns, with the TUI attached via `opencode attach --session <id>`, gives a wake with no keystrokes and no draft risk. For herdr-threads, an OpenCode adapter could do the same from an in-process plugin with the SDK `client` (no port needed, primer shape 1), or by managed launch (`serve` + `attach` in the Herdr pane).
2. **Coalesce guard.** Never `prompt` a busy session, because OpenCode merges it into the running turn. Track busy from `session.status` and close turns on the idle event. That is a must-have for any OpenCode adapter that honours our rule 4 (no nudge to a busy session).
3. **Prefix at `chat.message`.** This is the turn-start delivery analogue of our `UserPromptSubmit` path. Prefer adding a separate `synthetic: true` part over mutating the human's text part, to keep authorship clean (primer section 3).
4. **Ack at turn end with in-progress renewal.** A batch is "delivered" only when the turn that carried it finishes. A failed turn redelivers; a user Stop counts as dismissal. Compare our display-ACK: Cotal's definition survives a turn that crashes after display.
5. **User Stop ≠ failure.** `MessageAbortedError`, or a `session.interrupt` TUI command within 30 s, is treated as dismissal, so the batch is not re-driven into a person who just pressed Stop (src/plugin.ts:1428-1460). This matches the Claude-native "pause after abort" rule in the overview.
6. **Broker-counted redelivery marker in the frame** (`(redelivered, delivery N: ...)`). It is cheap, unforgeable and tells the model a previous copy may already have run. Our `[pN]`-style markers could carry the same.
7. **Column-zero framing rule** with full Unicode line-break neutralization (U+2028, U+2029, U+0085) and bracket stripping in names (framing.ts:54-98). It is worth checking our `frame()` and `src/delivery/frame.rs` against those three code points.
8. **Inline config via `OPENCODE_CONFIG_CONTENT`** with the plugin path and `permission`, the TUI env stripped of it, and a per-launch server password passed only in env. It is a zero-footprint, isolated test install for an OpenCode adapter. It fits our "never write the real user config" rule.
9. **Process-global hook guard** (`globalThis.__cotalOpencodeHooks`). OpenCode may call the plugin factory once per scope, so the first call wires the endpoint and later calls return the same hooks (src/plugin.ts:79-83, 205) **V**. Any herdr-threads OpenCode plugin needs this.
10. **Refusing prompts during teardown** by throwing from `chat.message`, the only cancel lever 1.x offers (src/plugin.ts:1509-1516).

## Verification

Checked against the Cotal clone at `6426fb538e0b`, `docs/connectors.md`, `docs/connect-opencode.md`, the OpenCode 1.18.1 clone and the primer. Re-read every cited line range in `src/plugin.ts`, `src/plugin2.ts`, `src/launch.ts`, `src/serve.ts`, `src/extension.ts`, `src/tools.ts`, `src/plugin.entry.ts`, `src/opencode-line.ts`, plus `agent.ts`, `control.ts`, `framing.ts`, `orientation.ts` and `tool-specs.ts`. Grepped both plugins again for the "Not used" hooks. The mechanism claims hold: `prompt_async` / `/api/session/:id/prompt` drive, the `chat.message` / `prompt` prefix, ack at turn end with renewal, Stop counted as dismissal, the busy guard, the framing rules, the launch and auth shape, and the per-message `system` loss in [oc 1.18.1] prompt.ts:1096, 1272-1273 and request.ts:56-62.

Changed:
- The citation header was wrong: `src/` paths are relative to `extensions/connector-opencode/`, not the clone root. Bare core file names are now mapped.
- The connectors.md line numbers were off by one. Mid-turn steering is on :28, and the "Missing" row cites :28-30, 33.
- The `max_payload` bound cited the session-rail transport (endpoint-session-rail.ts:161), which does not carry chat messages. It is now a stock NATS default, marked I.
- `session.status` does more than presence: `busy` gates `drive`, and `idle` does not clear it.
- The hook list now includes `tool`, `dispose` and 2.x `model.request`.
- MCP-server inheritance is documented. Plugin and provider inheritance is now marked I.
- The focus-mention hold is 1.x only. Added how 2.x behaves (no slot, one-shot nudge).
- The tool count was inconsistent (22 vs 24). It is now 24 declared and 20-24 advertised, and the description-size figure is marked I.
- Fix #1: the `agent.cotal.prompt` suggestion replaces OpenCode's default system prompt (request.ts:60). The caveat is added.
- Limitation 2 now notes that OpenCode's own step-boundary pickup and 2.x `session.synthetic` steering are both unused.
- The comment-density claim is quantified (766 of 1,542 lines).

