# Cotal x Hermes Agent (Nous Research): injection and wake

- Upstream: https://github.com/Cotal-AI/Cotal (site https://cotal.ai/, docs https://docs.cotal.ai). Clone `alts/cotal`, HEAD `6426fb538e0b` (2026-10-23). The clone was only read; nothing in it was executed.
- Hermes reference: the primer `../../primers/hermes.md` and the Hermes clone `alts/hermes-agent` at `f3b5b980d615` (main, 2026-10-23, the v0.22.x line). That clone is **newer than the range Cotal supports (0.18 to 0.21)**. Where this report cites Hermes code, the code is from main. Whether 0.18 to 0.21 behaves the same is marked INFERRED.
- Citations are `path:line` relative to the clone root. Connector code lives under `extensions/connector-hermes/` (shortened below to `ch/`), and the shared core under `extensions/connector-core/src/` (shortened to `core/`). **V** = VERIFIED (read in source or docs), **I** = INFERRED.

## Bottom line

Cotal does not attach to a person's Hermes TUI session. It runs its own **headless `hermes gateway run`**, with a Python **gateway platform plugin** named `cotal`. Each mesh message becomes a native `MessageEvent` passed to `BasePlatformAdapter.handle_message`, the same path a Telegram or Slack message takes. Each sending peer, and each channel, gets its own gateway session: `dm:<peerId>` or `channel:<name>`. The turn's final reply is routed back to the mesh automatically. Waking is free: an inbound platform message *is* a turn. Busy handling is left to Hermes. On the Hermes code we read, a message that arrives during a running turn in the same session **interrupts that turn by default**. Nothing is typed into a terminal, so there is no draft to clobber. There is also no human composer to protect.

---

## 1. Support status

| Item | Finding | |
|---|---|---|
| Maturity | Alpha. "it runs today ... but with real constraints" (docs/connect-hermes.md:9-12; docs/connectors.md:23) | V |
| How it binds | "native Python plugin, socket-bridged" (docs/connectors.md:24). A TS launcher/sidecar owns the mesh endpoint and supervises `uv run --project <connector> hermes gateway run` (ch/src/binary.ts:11-19; ch/src/launch.ts:1-16) | V |
| Version gate | `hermes-agent >=0.18,<0.22`, asserted at launch, refused outside the range (ch/src/launch.ts:29-50, 244-272; ch/pyproject.toml:26-28; docs/connect-hermes.md:20-26). Hermes' own installer gives 0.22.x since 2026-10-14, "which is outside the range and refused" (docs/connect-hermes.md:23-25). The latest Hermes is v0.22.5 (primer, header). **So a default Hermes install today is refused.** | V |
| Platform | Unix only, because of the AF_UNIX bridge (docs/connect-hermes.md:16-17) | V |
| UI | Headless: "Watch the real TUI: ✗ (headless gateway)" (docs/connectors.md:26). "Mid-turn steering: none" (docs/connectors.md:28) | V |
| Initial prompt | `cotal spawn --prompt` throws: "the gateway has no first-turn carrier wired" (docs/connect-hermes.md:259-260) | V |
| Event plane | Hermes publishes no AG-UI event plane, so a launch needs `--no-events` (docs/connect-hermes.md:35-42) | V |
| Run modes | **Managed**: the launcher builds a disposable profile. **Adopted** (`COTAL_HERMES_ADOPT_HOME`): the gateway runs on the operator's profile (docs/connect-hermes.md:135-185). **Standalone**: the user's own gateway with the plugin installed, where the plugin spawns the bundled Node sidecar itself (ch/plugin/cotal/__init__.py:1-15, 93-136; ch/src/standalone.ts:1-10; ch/bin/install.mjs:1-20) | V |
| Contradiction in their docs | The connectors page says "fresh agent per message ... no live turn to steer" (docs/connectors.md:56-57). The adapter's own docstring says `handle_message` "wakes an idle session or **queues + interrupts a running one** ... so a peer can DRIVE a live turn" (ch/plugin/cotal/adapter.py:3-6). Hermes main does interrupt by default (section 3). "Fresh agent" refers to the `AIAgent` object; the conversation persists per chat ("The gateway keeps one session per mesh chat", docs/connect-hermes.md:196-197). | V (both texts) / I (which one holds on 0.21) |

---

## 2. Injection mechanism

### 2.1 Path, end to end

1. **Mesh to sidecar.** The TS `MeshAgent` receives over NATS/JetStream and classifies each item into the `automatic` lane or the `pull-only` lane (core/agent.ts:1063-1131). On Hermes the sidecar sets `config.channelRepliesPullOnly = true` (ch/src/sidecar.ts:46-48), so a channel message that *replies* to another one goes pull-only unless it @mentions the seat. Without this, two Hermes seats answer each other forever (core/agent.ts:1119-1123; ch/CHANGELOG.md:215) **V**. Historical backfill is pull-only as well (core/agent.ts:1114-1118) **V**.
2. **Sidecar to plugin (bridge).** Newline-delimited JSON over an authenticated Unix socket. The first frame must be `subscribe` carrying the launch's control token (ch/src/bridge.ts:291-305). The socket path is derived from that token (ch/src/launch.ts:75-97) **V**. `pump()` sends the **oldest automatic item, one at a time**:
   ```ts
   const pump = (): void => {
     if (!adapter || awaitingId) return;
     const next = agent.peekInbox("automatic").find((i) => !deferred.has(i.recvKey));
     if (!next) return;
     awaitingId = next.recvKey;
     sendFrame(adapter, { t: "incoming", msg: wireItem(agent, next) });
   };
   agent.on("incoming", () => pump());
   agent.on("wake", () => pump());
   ```
   (ch/src/bridge.ts:143-154) **V**. The wire item is flattened in `wireItem` (ch/src/bridge.ts:76-103).
3. **Plugin reader thread to gateway loop.** `BridgeClient._dispatch` calls `_on_incoming`, which runs `asyncio.run_coroutine_threadsafe(self._inject(msg), loop)` and attaches `_maybe_ack` as the done callback (ch/plugin/cotal/bridge_client.py:162-176; ch/plugin/cotal/adapter.py:159-165) **V**.
4. **`_inject`.** It mints the chat: `channel:<ch>` for a channel message, `dm:<fromId>` for a DM or anycast (ch/plugin/cotal/adapter.py:189-198). It builds a `SessionSource` with `build_source(chat_id, chat_name, chat_type, user_id, user_name)` (:229-235). It wraps the framed text in `MessageEvent(text=format_injection(msg), message_type=TEXT, source=..., message_id=msg["id"])` (:236-249) and finishes with `await self.handle_message(event)` (:256) **V**.
5. **Answer routing.** When a DM answers a question that one of this gateway's sessions asked (`answersQuestion` plus a `contextId` this plugin minted), it runs in the asking session (ch/plugin/cotal/adapter.py:200-235; ch/plugin/cotal/replies.py:1-16). If the asker is a session on **another Hermes platform** (Telegram, ...), the answer goes in through **`ctx.inject_message(content, session_key=...)`** (ch/plugin/cotal/replies.py:44-46; ch/plugin/cotal/__init__.py:158-160). This needs `plugins.entries.cotal.allow_gateway_injection: true` (docs/connect-hermes.md:237-251) **V**. This is the only use of `inject_message`. The primary path is the platform adapter.

### 2.2 Payload and framing

```python
def format_injection(msg: dict) -> str:
    kind = attribution_safe(msg.get("kind"))
    sender = attribution_safe(msg.get("fromName") or "peer")
    role = msg.get("fromRole")
    who = f"{sender} / {attribution_safe(role)}" if role else sender
    history = "(history) " if msg.get("historical") is True else ""
    return f"[{kind} from {who}] {history}{body_safe(msg.get('text'))}"
```
(ch/plugin/cotal/framing.py:43-60) **V**.

- The result is one header line, `[dm from bob / reviewer] body`, with continuation lines indented two spaces. Every Unicode line break (`\r\n`, `\n\v\f`, U+0085, U+2028, U+2029) is neutralized in the body, and those breaks plus both brackets are neutralized in the attribution. The stated rule: "A LINE THAT BEGINS AT COLUMN ZERO IS WRITTEN BY THE CONNECTOR, NEVER BY A PEER" (ch/plugin/cotal/framing.py:1-30, 33-40) **V**. herdr-threads' `src/delivery/frame.rs:55-70` indents bodies at the same line-break set. Its header `one_line` (frame.rs:42-52) replaces every C0/C1 control plus U+2028/U+2029, which is broader, but it does not neutralize brackets (**V**, our repo).
- **No message id and no thread id in the text.** The id travels only as `MessageEvent.message_id` (ch/plugin/cotal/adapter.py:248), so the model cannot cite or ACK it **V**. That is acceptable here, because the reply is automatic (2.4).
- **No per-block instruction tail on Hermes.** The overview's "do, verify, reply" tail applies to the Claude hook drain. The Hermes path renders only `format_injection`, so it differs from Cotal's Claude framing (ch/plugin/cotal/adapter.py:245) **V**.
- **One message per turn, never batched.** One `MessageEvent` carries one mesh item (ch/src/bridge.ts:28-31) **V**. Hermes may merge or debounce queued text while busy (main: `merge_pending_message_event`, `_queue_text_debounce`, gateway/platforms/base.py:4146-4157 in the Hermes clone) **V** (main) / **I** (0.21).
- Static framing is placed once per session, never per message: the persona plus four guidance paragraphs go into `SOUL.md` (ch/src/launch.ts:149-152; core/orientation.ts:20-49), and a `platform_hint` goes into the system prompt (ch/plugin/cotal/__init__.py:169-173). Hermes treats `platform_hint` as cached system-prompt bytes (Hermes `plugins/platforms/*/adapter.py` comments, e.g. `plugins/platforms/whatsapp/adapter.py:1235`) **V**.

### 2.3 When it fires

- **Immediately on arrival, idle or busy.** `pump` runs on every `incoming` event. It checks no presence state, no attention mode and no turn boundary (ch/src/bridge.ts:143-154) **V**. Unlike Cotal's Claude connector, the Hermes bridge does not hold ambient channel chatter until Stop-to-idle. `post_llm_call` calls `requestWake()` when `pendingWake() > 0` (ch/src/hermes-hooks.ts:54-55), and the bridge's `wake` listener only calls `pump()` again (ch/src/bridge.ts:154). The pump itself never holds an item back, so the "two tiers" rule (docs/connect-claude.md:441-444) does not apply to Hermes. Normal (non-quiet) ambient in `dnd` is pushed as eagerly as a DM, although the attention table says dnd ambient "never wakes" (docs/connect-claude.md:460-464) **I** (from code: only `focus` ack-drops ambient at ingest, core/agent.ts:1105-1112; otherwise ingest buffers it as automatic, core/agent.ts:1123-1127, and `peekInbox("automatic")` returns it, core/agent.ts:1374-1376). Quiet-channel ambient is pull-only and skipped (docs/connect-hermes.md:122-124) **V**.
- Hermes **hooks inject nothing**. `pre_llm_call` and the rest return `None` and only relay presence (ch/plugin/cotal/hooks.py:108-126; ch/src/hermes-hooks.ts:1-11 "hooks never inject or ack") **V**. There is no `pre_llm_call` context, no `transform_tool_result` and no mid-tool-loop delivery.

### 2.4 Replies, tools and pull

- **The reply is free.** The gateway hands the turn's final text to `CotalAdapter.send()`, which publishes it back to the origin (channel broadcast, or a DM to the sender) as a reply to the triggering message (ch/plugin/cotal/adapter.py:138-150; ch/src/bridge.ts:156-160) **V**. A plain answer needs no tool call.
- Proactive actions use the `cotal_*` tools. The descriptors are generated by the sidecar from the shared specs, written to `COTAL_TOOLS_FILE`, and registered as Hermes plugin tools whose handlers forward over the bridge with a 30 s timeout (ch/plugin/cotal/tools.py:96-104, 39-56; ch/plugin/cotal/bridge_client.py:216-247; ch/src/tool-schema.ts:33-40) **V**.
- `cotal_inbox` on Hermes is published with **no parameters** and is forced to `scope: "pull-only"`. It surfaces only quiet ambient and channel replies, never the automatic lane, "so it cannot race the connector's automatic per-turn delivery ack" (ch/src/bridge.ts:162-190; ch/src/tool-schema.ts:28-37) **V**.

### 2.5 Size bounds

- **Inbound: no truncation on the Hermes path.** `format_injection` emits the whole body (ch/plugin/cotal/framing.py:60). The 1,024-code-point and 8-message inline caps live in the shared `core/inline-delivery.ts:31-32`, which the Hermes connector never calls **V** (absence in `ch/`). The effective ceiling is the NATS `max_payload` (packages/core/src/transfer.ts:175-180) **I**. `max_message_length=8000` in `register_platform` is Hermes' **outbound** chunking cap for replies (ch/plugin/cotal/__init__.py:175; Hermes `gateway/platform_registry.py:84` "smart-chunking cap") **V**.
- Local inbox: `MAX_INBOX = 200`. Overflow evicts pull-only items first, then channel items, and never acks an evicted directed message, so JetStream redelivers it (core/agent.ts:227, 1143-1191) **V**.

### 2.6 Dedupe and acknowledgement

- **Serial, with one item in flight.** `awaitingId` holds the receive key until the plugin answers `delivered` (ack, then `drainInboxDeliveries([key])`, then the next pump) or `deferred` (left unacked, re-offered after `DEFER_MS = 30_000`, and the items behind it go first) (ch/src/bridge.ts:66, 200-226) **V**.
- **What counts as "delivered":** `_maybe_ack` sends `delivered` when the `_inject` coroutine completes without an exception (ch/plugin/cotal/adapter.py:167-187). The authors flag that this is unproven: "`handle_message` returning means the event was *queued*, not necessarily *consumed into a turn* ... Until proven, this acks on the inject coroutine completing without error" (:170-175) **V**. On Hermes main, `handle_message` "returns quickly by spawning a background task" (Hermes `gateway/platforms/base.py:3989-3991`), and it can return **without accepting** the event: `_drop_unresolved` (:4009), a routing mismatch "Dropping internally routed event" (:4019), or the internal-event slot check (:4138). Cotal never reads the host's `event._gateway_accepted` flag. **So in practice this is ack on host return, and some of those paths drop the event silently** (V on main / I on 0.21).
- **Dedupe**: done in `MeshAgent` by id. A pending duplicate keeps one entry, and handled ids are remembered in two rotating windows (core/agent.ts:1030-1049, 1544-1550) **V**. There is no ACK-request primitive and no reader-visible receipt. The model has no way to acknowledge a message other than replying.

---

## 3. Wake mechanism

- **Mechanism: a native platform message.** On an idle session, `handle_message` starts a background turn (Hermes `gateway/platforms/base.py:4027-4031`: `_start_session_processing`) **V** (main). No keystrokes, no tmux, no polling and no long-poll tool are involved. Cotal's own Herdr, tmux and PTY runtimes type only for spawn, dialogs and stop (overview, "No keystroke injection for messages") **V**.
- **Busy detection belongs to the host.** The session is busy if `session_key in self._active_sessions` (Hermes `base.py:4027`). The event then goes to `_handle_message_while_active` and on to the runner's busy handler, which applies `busy_input_mode` (Hermes `gateway/run_busy.py:887-955`). On main the default is `display.busy_input_mode: "interrupt"` (`hermes_cli/config_defaults.py:829`; `gateway/run_config_loaders.py:256`; `base.py:1940` is only the adapter's pre-sync fallback), which calls `_interrupt_running_agent_for_busy_event` (`run_busy.py:972`), queues the message as the next turn, and sends a busy-ack reply debounced to one per 30 s (`run_busy.py:960-996`). Cotal's docs quote it as "Interrupting current task" (docs/connect-hermes.md:126); on main the text comes from `_compose_busy_ack_message`, and the exact wording was not found. The ack can be switched off with `HERMES_GATEWAY_BUSY_ACK_ENABLED` (`run_busy.py:976`), which Cotal does not set **V** (grep). **Cotal's managed `config.yaml` does not set `busy_input_mode`** (ch/src/launch.ts:135-147), so the default applies **V** (main) / **I** (0.21; the adapter docstring, ch/plugin/cotal/adapter.py:3-6, says "queues + interrupts", which agrees).
- **Busy is scoped per session, not per seat.** A DM from peer B while peer A's turn runs goes to a different session (`dm:B`) and **starts a concurrent turn** on the same seat **I** (each chat_id maps to its own session key; adapter.py:193-198). A second DM from A interrupts A's own turn.
- **Busy-ack noise:** the busy notice is an ordinary `send()` to the mesh, so peers receive "Interrupting current task"-style replies. Cotal's docs acknowledge that this happens (docs/connect-hermes.md:125-127) **V**.
- **Presence**, used by peers and run relays but not by delivery: `pre_llm_call` and `pre_tool_call` set `working`, `post_llm_call` and `on_session_end` set `idle`, and `gateway_startup` and `on_session_start` reset to idle without counting as a turn end (ch/src/hermes-hooks.ts:28-62; docs/connect-hermes.md:213-220). Each relay is a one-shot Unix-socket write with a 2 s timeout that fails silently (ch/plugin/cotal/hooks.py:28, 62-90) **V**.
- **Draft safety: not applicable.** There is no TUI and no human composer. An operator watches the launcher's PTY output (gateway logs) through `cotal attach` (ch/src/launch.ts:15) **V**/**I**.
- **Latency:** push, from NATS to the sidecar event to the socket frame to `run_coroutine_threadsafe`, so sub-second plus the host's own startup per turn **I** (not measured). A deferred answer waits 30 s (ch/src/bridge.ts:66) **V**. Hermes' optional `human_delay` and busy text debounce are not configured by Cotal **I**.
- **Failure modes:**
  1. **Bridge wedge on an unexpected exception.** If `_inject` raises anything other than `InjectionRefused` (for example in `build_source`, `seed_chat` or `handle_message`), or the future is cancelled, `_maybe_ack` sends **neither** `delivered` nor `deferred` (ch/plugin/cotal/adapter.py:176-187). The sidecar keeps `awaitingId` set, and `pump()` returns early from then on (ch/src/bridge.ts:144), so **all inbound delivery stops** until the adapter resubscribes. Its JetStream redelivery re-emits `incoming` but hits the same guard **I** (from code). The same class of bug shipped once before: "the bridge no longer stops after the first automatic item of a gateway boot" (ch/CHANGELOG.md:227) **V**.
  2. **Silent drop by the host after ack** (2.6) **I**.
  3. **A dead reader thread** is reported as a *retryable fatal*, so Hermes' reconnect watcher repairs it. Reconnect reopens the singleton client with a generation counter and `shutdown(SHUT_RDWR)` to unpark a blocked `recv` (ch/plugin/cotal/adapter.py:93-131; ch/plugin/cotal/bridge_client.py:249-341) **V**.
  4. **A sidecar restart** self-heals: the client redials with 2 s backoff and resubscribes (ch/plugin/cotal/bridge_client.py:22, 76-92) **V**.
  5. **A refused `inject_message`** (no `allow_gateway_injection`): the answer stays unacked and is re-offered every 30 s indefinitely, with a log line each time (docs/connect-hermes.md:247-251) **V**.
  6. **Version drift:** every Hermes from 0.22 on is refused at launch (section 1) **V**.

---

## 4. Setup and installation footprint

**Managed (default)**, all inside `$TMPDIR/cotal-hermes-<id>` (ch/src/seat-home.ts:30-34):
- `profiles/cotal-<id>/` as `HERMES_HOME`. Using a named profile gives the seat its own systemd unit, `hermes-gateway-cotal-<id>`, instead of clobbering the operator's `hermes-gateway.service` (ch/src/seat-home.ts:1-29; ch/CHANGELOG.md:218) **V**.
- `config.yaml` is regenerated every launch (ch/src/launch.ts:135-147) **V**:
  ```yaml
  plugins:
    enabled: [cotal]
  gateway:
    platforms:
      cotal:
        enabled: true
  approvals:
    mode: off
  model: "<model>"
  ```
- A copy of `plugins/cotal/`, `SOUL.md` (persona plus orientation, channel, mesh-first and workflow guidance; only when the agent file has a persona), `cotal-tools.json`, and a `tmp/` used as the gateway's TMPDIR (ch/src/launch.ts:129-152, 349, 390-403) **V**.
- Environment for the gateway: `HERMES_HOME`, `TMPDIR`, `COTAL_CONTROL_SOCKET`, `COTAL_BRIDGE_SOCKET`, `COTAL_TOOLS_FILE`, a launch-material file path carrying the control token (not the token itself in env), and optionally `COTAL_HERMES_FORK_SESSION` (ch/src/launch.ts:350-403). The plugin itself sets `COTAL_ALLOW_ALL_USERS=true` and `COTAL_HOME_CHANNEL=mesh`, so Hermes' pairing and home-channel prompts do not intercept peers (ch/plugin/cotal/__init__.py:148-154) **V**.
- On stop: SIGTERM, a 1.5 s drain, then a tree kill and removal of the root. A `--resume` fork's root is kept (ch/src/launch.ts:56-59, 409-453) **V**.

**Adopted** (`COTAL_HERMES_ADOPT_HOME`): writes only `plugins/cotal/` and `cotal-tools.json`. It never writes `config.yaml` or `SOUL.md`, and it refuses unless the operator has enabled the plugin and the platform. Hermes itself may rewrite `config.yaml` for `onboarding.seen` (ch/src/launch.ts:186-239; docs/connect-hermes.md:153-185) **V**.

**Standalone installer** (`npx @cotal-ai/connector-hermes install`): copies the plugin into `<HERMES_HOME>/plugins/cotal`, runs `hermes plugins enable cotal`, and writes `COTAL_LINK` or `COTAL_SPACE`/`COTAL_NAME`/`COTAL_SERVERS` into `<HERMES_HOME>/.env`. Uninstall strips only those keys (ch/bin/install.mjs:1-20, 45, 415-458, 497-512) **V**. This path does not run the version check, which lives only in `launch.ts` **V** (absence: `assertHermesVersion` appears nowhere else in `ch/`).

---

## 5. Token cost

- **Per message: one full Hermes turn.** That is the system prompt (Hermes base + `SOUL.md` guidance of about 1.2 KB from core/orientation.ts:20-50, four strings totalling 1,200 characters + a 195-byte `platform_hint`), the tool schemas, that chat's history, and the framed message. The framing overhead is about `[dm from <name> / <role>] `, roughly 20-40 bytes per message **I** (computed from the format string). The `cotal_*` surface has 24 tool specs with long multi-sentence descriptions (`grep -c 'name: "cotal_'` gives 24 in core/tool-specs.ts, a 124 KB file), and they are sent on every LLM call of every turn. That is likely several thousand tokens a call, though the exact figure is unknown (not rendered). Only `cotal_spawn`, `cotal_persona(s)` and `cotal_run` are capability-gated (core/tool-specs.ts:689-703) **V**/**I**.
- **The reply costs no tool call** (2.4). Compared with "read inbox, then call send", this saves one model round trip per answer **I**.
- **Per quiet turn: zero.** No hook returns context and nothing polls. A seat runs turns only when a message arrives **V** (hooks.py returns `None`).
- **Hidden costs:** every peer gets its own session, and with it its own copy of system prompt and history (**I**). Each interrupt wastes the partial turn (**I**). Channel ambient in `open`/`dnd` is pumped one message per turn (2.3), so a busy channel costs N turns, mitigated only by Hermes' busy-queue merging **I**.

---

## 6. Limitations and bugs visible in source

1. **Default Hermes is refused today.** The ceiling is exclusive of 0.22, and Hermes ships 0.22.5 (section 1). This is the opposite of herdr-threads' "never gate on an exact version" rule (AGENTS.md, "Harness and Herdr versions") **V**.
2. **Bridge wedge** on a non-`InjectionRefused` exception or a cancellation (section 3, failure 1) **I** (code path read; not reproduced).
3. **Ack-on-queue presented as ack-on-surface.** The authors leave an open "VALIDATION GATE" (adapter.py:170-175). Host drop paths after `handle_message` returns lose the message silently (2.6) **I**.
4. **The interrupt default** turns a peer's follow-up into an abort of the running turn, and the busy notices leak to the mesh as replies (section 3) **V** (main) / **I** (0.21).
5. **No two-tier attention on Hermes.** Ambient is pumped as eagerly as directed mail, whatever the presence state (2.3) **I**.
6. **Presence is per seat while turns are per session.** With concurrent sessions (peer A and peer B), session A's `post_llm_call` writes `idle` while B is still `working` (ch/src/hermes-hooks.ts:50-53) **I**.
7. **`waiting` is unreachable.** The TS handle maps `approval_wait` to `waiting` (ch/src/hermes-hooks.ts:47-49), but the Python plugin never registers or relays an `approval_wait` event. It registers only five hooks (ch/plugin/cotal/__init__.py:179-183; hooks.py:108-126). Approvals are `off` in managed mode anyway, but an adopted profile keeps the operator's approval settings (docs/connect-hermes.md:167-169). Cotal's own docs say `approval_wait` writes `waiting` (docs/connect-hermes.md:215-216), so the docs and the code disagree **V** (absence).
8. **No integration with a human's interactive Hermes.** Mesh traffic lands in separate gateway sessions keyed by peer, never in the conversation a person is having in `hermes` or `hermes --tui`. The only bridge into another session is the answer-to-a-question `inject_message` path (2.1 step 5) **V**/**I**.
9. **No inbound size bound** on the model-visible text (2.5) **I**.
10. **The model cannot reference message ids**, because none appear in the frame (2.2) **V**.

---

## 7. Clever ideas worth stealing

1. **A Hermes gateway platform adapter as the wake primitive.** `ctx.register_platform` plus `handle_message(MessageEvent)` gives a native wake with no keystrokes, and Hermes' own idle/busy queue handles the rest. In Hermes 0.22.1 and later every local session (CLI, TUI, Desktop) runs in the one gateway (primer §1), so a plugin there could reach a pane's session. For a session that already exists, the closer fit is `ctx.inject_message(..., session_key=<ses_…>)`, which Cotal already uses for cross-platform answers. The primer (§4, §12 item 2) proposes the same for herdr-threads. Cotal's code is a working reference for the `inspect.signature(...).parameters` probe (`"session_key" in ...`, ch/plugin/cotal/__init__.py:158-160) and for the refusal handling (keep the message unacked and re-offer it every 30 s). **Caveat for us:** set or recommend `busy_input_mode: queue` (or deliver mid-turn via `transform_tool_result`), or a nudge lands as an interrupt. That would violate delivery-contract rule 4.
2. **A deferred lane that never blocks the head.** A delivery the host refuses is held for a fixed delay while later items keep flowing (ch/src/bridge.ts:214-226). This is a clean answer to head-of-line blocking for a seat whose wake path is temporarily refused.
3. **Making the host's reconnect machinery the health check.** A dead reader is reported as a retryable fatal, so Hermes' watcher retries it instead of the platform sitting "connected and deaf" (ch/plugin/cotal/adapter.py:107-127). Our Hermes plugin's hook bridge could use the same idea: treat a broken daemon link as a host-visible failure, not a quiet no-op.
4. **Auto-reply suppression on reply chains** (`channelRepliesPullOnly`) and **pull-only historical backfill**, which prevents turn storms. They measured "119 injected digests / 0 assistant turns" before the fix (core/agent.ts:1114-1123). This matters to us for any future auto-wake on group threads.
5. **Overflow that never acks directed mail.** Pull-only items go first, channel items second, and directed messages are left for broker redelivery (core/agent.ts:1143-1191).
6. **Lifecycle idle is not a turn end** (`resetStatus` vs `setStatus`, ch/src/hermes-hooks.ts:32-39). A reconnect or session start landing mid-turn must not complete pending work. This is relevant to our check-in and ACK accounting on `on_session_start`.
7. **Secrets in a 0600 launch-material file, not in env.** Descendants of the gateway, including the model's terminal tools, do not inherit the bearer by accident (ch/src/launch.ts:350-380; ch/plugin/cotal/hooks.py:32-51). Our Hermes bridge passes `--state-dir`/`--host-endpoint` on argv instead; no secret is involved, so this note is for completeness.
8. **A named Hermes profile for an isolated seat**, which avoids clobbering the operator's systemd unit (ch/src/seat-home.ts:1-29). It is useful for our wake-e2e or isolated Hermes tests.
9. **The model's final reply routed automatically to the asker.** It saves a tool call per answer. It does not fit herdr-threads directly: our agents live in human-facing panes, and their replies are not addressed to peers. A "reply goes to the thread that woke you" option for headless worker seats would be the analogue **I**.

### What it means for herdr-threads' Hermes integration (short)

- The approach does not transfer as-is. Cotal owns a headless gateway and its sessions. herdr-threads serves a person's Hermes in a Herdr pane, and that session lives in the shared gateway from 0.22.1 on (primer §1).
- The transferable part is the **plugin-side native wake**. Cotal's code shows that a plugin running in the gateway can push turns into sessions without keystrokes. It also shows the failure modes to design for: ack-on-queue, the interrupt default, refusal deferral, and a deaf platform.
- Cotal's choice to reach Hermes only through a plugin inside the gateway also sidesteps the `HERDR_PANE_ID`-in-gateway-env breakage the primer flags (§12 item 1). Its plugin never needs the pane's environment, because identity comes from the launch material and the gateway session context (`gateway.session_context.get_session_env`, ch/plugin/cotal/tools.py:59-82) **V**. For us, that points to keying Hermes seats on `HERMES_SESSION_KEY`/`session_id` from the session context rather than on process env **I**.

---

## Verification

An adversarial pass on 2026-10-23 re-read every cited range in the Cotal clone (`6426fb538e0b`), the Hermes clone (`f3b5b980`), the Hermes primer and our `src/delivery/frame.rs`. Checked: all `ch/src/*.ts` and `ch/plugin/cotal/*.py` citations (bridge pump, defer and ack handling, `_inject`/`_maybe_ack`, framing, hooks, replies, tools, launch, seat-home, installer, pyproject), the core/agent.ts classification, overflow and dedupe ranges, the docs/connect-hermes.md and docs/connectors.md quotes, and the Hermes `handle_message`, busy-handling, `inject_message`, `platform_registry` and hook-firing paths. Most claims held.

Changed:
- `CHANGELOG.md:215` corrected to `ch/CHANGELOG.md:215`. The repo root has no CHANGELOG.
- `tool-schema.ts` citations corrected. The file has 41 lines, so `:47-49` and `:52-59` could not exist; they are now `:28-37` and `:33-40`.
- The `wake` listener is cited at bridge.ts:154. The post-turn `requestWake` is at hermes-hooks.ts:54-55. Added that `focus` still ack-drops ambient at ingest, so the "eager ambient" finding applies to `open` and `dnd` only, against the documented dnd "never wakes" rule.
- The 1,024/8 inline caps live in the shared `connector-core/src/inline-delivery.ts`, not in Claude-specific code.
- `platform_hint` is 195 bytes, not 230. The SOUL guidance was measured at 1,200 characters.
- The Hermes busy default is now cited at `config_defaults.py:829` and `run_config_loaders.py:256`. `base.py:1940` is only a pre-sync fallback. The "Interrupting current task" wording is attributed to Cotal's docs, because it was not found in Hermes main. Added the 30 s ack debounce and the `HERMES_GATEWAY_BUSY_ACK_ENABLED` switch, which Cotal leaves unset.
- The standalone installer's missing version check was upgraded from INFERRED to VERIFIED (absence).
- Added that Cotal's docs claim `approval_wait` maps to `waiting`, which the plugin never registers.
- Our `frame.rs` comparison was narrowed: the body line-break set is the same, the header control set is broader, and we do not neutralize brackets.
