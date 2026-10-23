# agent-peers-mcp (mikez93) x OpenCode

- Upstream: https://github.com/mikez93/agent-peers-mcp, read at `2c63379d65ad44845815faa21a0c5c78355002fa`. Paths without a prefix are relative to that clone's root.
- OpenCode paths are marked `[oc 1.18.1]` and are relative to a clone of https://github.com/anomalyco/opencode at tag `v1.18.1`, the version installed locally (see `primers/opencode.md`, section 0).
- VERIFIED means read in source or docs. INFERRED means my deduction; nothing here was run.

## Support status

**OpenCode is not supported.** VERIFIED:

- `grep -rniI -e opencode -e open-code -e sst/ -e anomalyco` over the clone, with `.git` excluded, finds nothing.
- The peer type is a closed union: `export type PeerType = "claude" | "codex" | "hermes" | "droid";` (`shared/types.ts:5`).
- The broker enforces the same set in SQL: `peer_type TEXT NOT NULL CHECK(peer_type IN ('claude', 'codex', 'hermes', 'droid'))` (`broker.ts:145`, `broker.ts:307`, `broker.ts:454`). Even a correctly labelled OpenCode peer needs a schema migration. The broker has done this twice before, for hermes and droid (`broker.ts:423-478`).
- The runtime selector inside the shared polling server only knows `hermes`, `droid` or else `codex` (`codex-server.ts:103-107`).
- Every wake subsystem is tied to one harness: the Codex app-server (`shared/wake-daemon.ts`, `shared/app-server-client.ts`), the Droid ACP/native relay (`shared/droid-*.ts`), the Claude `asyncRewake` hook (`bin/claude-peer-rewake`) and the Hermes host RPCs (`shared/hermes-*.ts`). None targets OpenCode.

The rest of this report asks what would happen if a user registered the generic MCP server (`codex-server.ts`, the "durable polling transport" the Hermes and Droid wrappers reuse) as a local MCP server in OpenCode. It is checked against the OpenCode primer and the OpenCode 1.18.1 source.

## Injection mechanism

### What the generic server does

These points are VERIFIED unless marked otherwise.

- **Transport.** It is a stdio MCP server with `capabilities: { logging: {}, tools: {} }` and a long `instructions` string (`codex-server.ts:158-226`).
- **Background poll.** The server polls the broker every 1 s while active and every 3 s after 120 s without activity (`codex-server.ts:94-101`). Leased messages are written to a durable JSON inbox before anything else happens (`codex-server.ts:383-397`). The inbox lives at `~/.agent-peers-<runtime>/<peer-uuid>.json`, which under the default runtime is `~/.agent-peers-codex/` (`shared/codex-inbox.ts:57,103-106`).
- **Authoritative delivery is piggyback.** `withPiggyback` wraps every tool call (`codex-server.ts:479-547` and its locked section `acquireInboxBatch` at `codex-server.ts:551-690`; dispatch at `codex-server.ts:693-696`):
  1. It confirms whatever the previous call presented.
  2. It flushes acks.
  3. It runs an inline poll.
  4. It draws all unconfirmed queued messages.
  5. It prepends them to the tool's own text:

     ```ts
     const inbox = formatInboxBlock(fresh);
     const finalText = inbox + toolText;
     ```

     (`codex-server.ts:533-534`)
- **The six tools** are `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages` and `rename_peer` (`codex-server.ts:228-291`). All of them piggyback. `check_messages` does nothing else: `return { text: "Checked inbox." }` (`codex-server.ts:729-732`).
- **Preview push.** One `notifications/message` per new message carries a body-free preview: sender, peer type, and a pointer telling the model to call `check_messages` (`codex-server.ts:410-435`, `shared/piggyback.ts:27-33`).

### How OpenCode receives each piece

- **Tool results reach the model. VERIFIED.**
  - OpenCode wraps each MCP tool as a session tool (`[oc 1.18.1] packages/opencode/src/session/tools.ts:390-470`).
  - It joins the text content parts and passes them through `truncate.output(...)`, so the `[PEER INBOX]` block becomes part of the tool result the model sees (`session/tools.ts:426-464`).
  - Tool names are prefixed with the configured server key, as `sanitize(clientName) + "_" + sanitize(name)` (`[oc 1.18.1] packages/opencode/src/mcp/catalog.ts:117-119`). A server registered as `agent-peers` therefore exposes `agent-peers_check_messages`. The server's instructions still say "call `check_messages`". INFERRED: models usually resolve this, but it is a small mismatch.
- **Preview notifications are dropped.** OpenCode registers a handler for `LoggingMessageNotificationSchema` that only writes to OpenCode's own log through `Effect.logInfo("MCP server log", …)` (`[oc 1.18.1] packages/opencode/src/mcp/index.ts:457-459,474-490`). The preview never reaches the model or the UI. VERIFIED. This is the same "dormant" state the README describes for Codex (`README.md:262-266`).
- **The Claude channel is irrelevant.** `notifications/claude/channel` comes from `claude-server.ts` only. OpenCode advertises no such capability and has no handler for it (primer section 5). VERIFIED by absence.
- **Server instructions reach the system prompt.** OpenCode puts them in the system prompt inside `<mcp_instructions>` (primer section 5, `[oc 1.18.1] packages/opencode/src/session/system.ts:112-127`). VERIFIED. The text is Codex-specific, though: the line "The current Codex CLI does NOT surface mid-task MCP push notifications" is chosen by `RUNTIME_IS_CODEX` (`codex-server.ts:177`). Under OpenCode the model reads a description of a different harness. VERIFIED that the text is selected that way. INFERRED that it confuses the model only mildly.

### Timing

- Mail reaches the model **only when the model calls one of the server's tools**.
- The instructions ask for `check_messages` "as the FIRST thing you do every time the user sends you a message" (`codex-server.ts:184-189`). The tool description repeats "Call this at the START of every user turn" (`codex-server.ts:264`).
- There is no harness-side trigger. The server uses none of OpenCode's hooks:
  - `chat.message`;
  - `experimental.chat.messages.transform`;
  - `tool.execute.after`.

  So mail that lands mid-turn waits until the model happens to call an agent-peers tool. VERIFIED by structure. The latency bound of "one user turn" holds only if the model obeys the instruction. INFERRED.

### Size bounds

- **Broker side:** sends are refused once a recipient holds 500 unacked messages, but only when that recipient is a durable peer that has gone stale (`p.last_seen >= ? OR (SELECT COUNT(*) ...) < 500`, `broker.ts:55`, `broker.ts:838-843`, error at `broker.ts:857-861`; the conversation path checks it at `broker.ts:805-809`). A live peer's backlog is uncapped. The lease query returns every eligible row `ORDER BY id ASC` with no LIMIT (`broker.ts:901-910`). VERIFIED.
- **No per-body truncation and no paging** in the server or in `formatInboxBlock`: `shared/piggyback.ts:35-63` embeds `m.text` as-is. VERIFIED by reading.
- **OpenCode's own truncation is the only bound.** Tool output is cut to `MAX_LINES = 2000` / `MAX_BYTES = 50 * 1024`, both configurable through `tool_output` (`[oc 1.18.1] packages/opencode/src/tool/truncate.ts:15-16,77-81`). The overflow spills to a file, and `outputPath` is recorded in the metadata (`session/tools.ts:464-469`). VERIFIED.
- **Consequence (INFERRED):**
  - A large backlog is still drawn and marked as presented in full.
  - The model sees only the first 50 KiB, plus a pointer to the spill file.
  - On the next call it is confirmed and acked as if it had all been read.
  - Messages past the cut are therefore acked unseen, unless the model reads the spill file. Their only remaining trace is the spill file.

### Framing

Each block starts with a fixed 7-line behavioural header of 483 bytes for one message (measured from the template): "Read each one. Then: … Do NOT auto-acknowledge" (`shared/piggyback.ts:38-47`). Each message then gets:

```
--- message i of n ---
from: <name> (<peer_type>, cwd=<cwd>)
their current work: <summary>      (optional)
sent_at: …
message_id: …
text:
<body>
reply_action: send_message(to_id="<name>", message="...")
```

The banner is the bare words `PEER INBOX` (no brackets, despite the server's prose calling it `[PEER INBOX]`); header, message blocks and a closing `PEER INBOX` are joined with blank lines (`shared/piggyback.ts:38,49-62`). VERIFIED.

There is no marker that separates the block from the tool's real output, apart from the banner words. In OpenCode it sits inside an ordinary tool result. INFERRED.

### Dedupe and acknowledgement

- **State machine.** `shared/delivery-state.ts` moves messages through draw, promote, rollback and markConfirmed. The arrival-causality barrier is `delivery.newArrival()`, taken at request entry (`codex-server.ts:494-496`).
- **Confirm-on-next-call.** A presented id is confirmed, pruned from disk and acked to the broker only when the *next* tool call arrives (`codex-server.ts:551-586`).
- **Cancellation rolls back.** A cancelled request rolls its draws back (`codex-server.ts:535-546`). OpenCode passes its tool `abortSignal` into `client.callTool(..., { signal: options.abortSignal })` (`[oc 1.18.1] packages/opencode/src/mcp/catalog.ts:54-66`). INFERRED: a user ESC-interrupt (primer section 7) therefore cancels the MCP request, and the server rolls back correctly.
- **Model-facing ack.** There is none, and the colleague protocol discourages replies of the "got it" kind (`shared/colleague-prompt.ts:22-25`).
- **The confirm signal is weak in OpenCode** (INFERRED). The signal is "the same MCP process got another call". In OpenCode, MCP clients are per-Instance, that is per project directory (`InstanceState.make`, `[oc 1.18.1] mcp/index.ts:492`; cwd from `InstanceState.directory`, `mcp/index.ts:345-346`), and they are shared across every session in that instance. So:
  - A call from session B, or from a `task` subagent's child session, confirms a block that was shown to session A.
  - A block can also be drawn by whichever session calls first. A subagent may consume peer mail meant for the main conversation.

## Wake mechanism

**None for OpenCode.** VERIFIED:

- `wait_for_peer_messages`:
  - It is a long-poll that keeps the *current* turn alive. The tool description says it "is not a fully idle wake mechanism" (`codex-server.ts:270`).
  - Default and max is 300 s for non-Hermes runtimes (`codex-server.ts:122-124`).
  - It polls every 500 ms (`codex-server.ts:124`), with an optional `from` filter (`codex-server.ts:276-283`, `804-808`).
  - It sends no MCP progress notifications (grep for `progress` in `codex-server.ts` and `shared/wait-for-peer-messages.ts` finds none).
- **OpenCode cuts the long-poll at 60 s.**
  - OpenCode calls MCP tools with `timeout` from per-server `timeout` or `experimental.mcp_timeout`, else `undefined` (`[oc 1.18.1] mcp/index.ts:660-684`).
  - With `undefined`, MCP SDK 1.29.0 (OpenCode's pinned dependency, `packages/opencode/package.json:83`) applies `DEFAULT_REQUEST_TIMEOUT_MSEC = 60000`. VERIFIED in a local copy of the SDK's `dist/esm/shared/protocol.js:8`; INFERRED to be the same version.
  - `resetTimeoutOnProgress: true` and an `onprogress` hook are set (`[oc 1.18.1] packages/opencode/src/mcp/catalog.ts:60-65`), but they never help because the server never sends progress.
  - So under stock OpenCode config the 300 s wait fails at 60 s with a timeout error. The request is cancelled, and the server rolls back any draw. INFERRED.
  - A user can raise `timeout` in the MCP config entry.
- **The Codex wake daemon does not apply.** It talks Codex app-server JSON-RPC (`turn/start`, `turn/steer`) to a thread registered by `codex-peer` (`shared/app-server-client.ts:228,257`; `shared/wake-daemon.ts:180-200,264-285`). OpenCode has no such server. The MCP child would not even try: it registers for wake only when `isWakeLaunchEnv` holds, which needs launcher-set `AGENT_PEERS_WAKE_*` variables (`codex-server.ts:882`, `1236-1245`). One Codex-only path does arm under OpenCode: `desktopWakeEligible` is true for runtime `codex` with no `AGENT_PEERS_WAKE_*` set (`shared/codex-desktop-wake.ts:13-16`, `codex-server.ts:937-942`), but it registers only after seeing a Codex `threadId` in the request `_meta` (`shared/codex-desktop-wake.ts:131-155`), and OpenCode's `callTool` sends only `name` and `arguments` (`[oc 1.18.1] packages/opencode/src/mcp/catalog.ts:54-58`). VERIFIED; it stays dormant.
- **The Droid ACP host does not apply.**
  - It is the architecturally closest piece. `droidpeer` owns a long-lived ACP stdio agent and sends `session/prompt` with the content-free `DROID_WAKE_PROMPT` when idle (`shared/droid-launcher.ts:69,284`; `shared/droid-acp-client.ts:146-152`).
  - OpenCode does ship `opencode acp` (primer section 4).
  - But the spawn is hardcoded: `spawn(opts.droidCommand ?? "droid", ["exec", "--output-format", "acp"], …)` (`shared/droid-acp-client.ts:320`). A `--droid opencode` override would run `opencode exec --output-format acp`, which is not an OpenCode command. INFERRED from the primer's CLI list.
  - Even if ported, the ACP host replaces the TUI in the pane with a headless agent (primer section 4).
- **The Claude `asyncRewake` hook does not apply.** It is a Claude Code `Stop`-hook contract (`README.md:270-289`). OpenCode v1 has no Stop-hook equivalent (primer section 2).
- **Idle/busy detection, draft safety, latency:** not applicable, because nothing wakes an idle OpenCode session.
  - An idle OpenCode TUI sees peer mail only when the human next types something and the model obeys the "call `check_messages` first" rule.
  - Draft safety is trivially kept: nothing is typed.
  - Failure mode: silent indefinite delay. Mail sits in the durable inbox and the broker with no TTL (`AGENTS.md:72-74`). There is no sender-side signal that the recipient is idle and unwakeable. INFERRED.
- **Herdr would not fill the gap either.** herdr-threads' native wake refuses Herdr agent kind `opencode` (primer section 11). This is irrelevant to agent-peers, but it means no existing tool in this study wakes OpenCode.

## Setup/installation footprint on the harness

No OpenCode recipe exists. VERIFIED by grep. A hand-rolled install would need the following (INFERRED, modelled on the Codex wire-only install at `README.md:75,140-147`):

- An `mcp` entry in `opencode.json` / `~/.config/opencode/opencode.json`:
  ```json
  "agent-peers": { "type": "local",
                   "command": ["bun", "<clone>/codex-server.ts"],
                   "environment": { "AGENT_PEERS_ENABLED": "1" } }
  ```
  - OpenCode merges `mcp.environment` over `process.env` for the child (`[oc 1.18.1] packages/opencode/src/mcp/index.ts:347-357`). VERIFIED.
  - Without `AGENT_PEERS_ENABLED=1` the server exposes zero tools (`codex-server.ts:859-868`). VERIFIED.
  - Optionally add `PEER_NAME` for a durable identity and `AGENT_PEERS_DISABLE_TAB_TITLE=1` (see the tab-title bug below).
  - An optional `"timeout"` raises the 60 s tool-call ceiling.
- The shared install: the repo clone, `bun install`, the broker under launchd `com.mike.agent-peers-broker`, `~/.agent-peers.db`, and `~/.agent-peers-secret`. The label is at `shared/ensure-broker.ts:20`, and the client runs `launchctl kickstart` (`shared/ensure-broker.ts:36-43`). Self-spawn is off unless `AGENT_PEERS_SPAWN_BROKER=1` (`shared/ensure-broker.ts:14,57`), so on Linux you need that flag or your own service unit. VERIFIED.
- Runtime files: `~/.agent-peers-codex/<uuid>.json` plus metadata. These sit next to real Codex peers' files, distinguished only by peer UUID.
- **Permission prompts.** Each MCP call goes through `ctx.ask({ permission: key, … })` with the prefixed tool name as key (`[oc 1.18.1] packages/opencode/src/session/tools.ts:408`). The agent defaults start from `"*": "allow"` (`[oc 1.18.1] packages/opencode/src/agent/agent.ts:119-120`), so MCP tools run without a prompt unless the user's `permission` config says otherwise. VERIFIED.
- Nothing is written into OpenCode's plugin, command or skill directories. The project never uses the OpenCode plugin API.

## Token cost per message and per quiet turn

Byte counts were measured in the clone; tokens assume about 4 bytes each.

**Static, every LLM request:**

| Item | Size |
|---|---|
| `COLLEAGUE_PROTOCOL` | ≈5.7 KB (`shared/colleague-prompt.ts`) |
| Delivery prologue | ≈2.8 KB (`codex-server.ts:173-226`) |
| Total in `<mcp_instructions>` | ≈8.5 KB, ≈2.1k tokens |
| Six tool schemas and descriptions | ≈2.6 KB (`codex-server.ts:228-300`) plus the peer-list description, about 0.7k tokens |

INFERRED: most of this is prompt-cached, but it is paid on every request and on every compaction.

**Per user turn, even with no mail:**

- If the model obeys, it makes one `check_messages` tool call. That is one extra LLM step: tool-call tokens plus the result `Checked inbox.`
- The full context is re-sent for that step, mostly from cache.
- INFERRED: this is roughly 30-60 output and input tokens plus one round trip of latency per turn. It is the dominant quiet-turn cost, and it is model-driven, so it cannot be skipped cheaply. Compare herdr-threads' empty hook output on quiet turns.

**Per message:**

- 483 B header per block (one message).
- About 150-250 B of per-message fields: `from`, cwd, optional summary, `sent_at`, id, and `reply_action`.
- Plus the full body, untruncated up to OpenCode's 50 KiB cut.
- A single short message costs about 650-750 B, roughly 170 tokens, of framing. herdr-threads uses one header line.

## Limitations and bugs you can see

1. **Wrong identity label.** The peer registers as `peer_type: "codex"`, because `AGENT_PEERS_RUNTIME` is unset or not recognised (`codex-server.ts:103-107,985`). Other peers see "codex", and the default name gets a `-codex` suffix (`codex-server.ts:949`, `shared/peer-identity.ts:89-96`). VERIFIED code path; INFERRED effect.
2. **Shared-identity problem in OpenCode.** OpenCode has one MCP process per Instance (directory), serving all sessions, child `task` sessions and session switches in that TUI (`[oc 1.18.1] packages/opencode/src/mcp/index.ts:345-346,492`; primer section 10). The result is one peer for many conversations. Mail is delivered to whichever session calls first, and confirmation can come from a different session. agent-peers solved the matching problem for the Codex app-server with root election (`README.md:398-413`), but nothing equivalent runs for OpenCode. INFERRED.
3. **Long-poll is cut at 60 s by the default OpenCode MCP timeout**, so the advertised 300 s wait errors out (see Wake mechanism). INFERRED, from VERIFIED parts.
4. **Unbounded draw combined with harness truncation** means messages past 50 KiB are acked unseen (see Size bounds). INFERRED.
5. **The tab-title keepalive writes to the OpenCode TUI's terminal.**
   - `writeTitle` opens `/dev/tty` and writes OSC 0, 1 and 2 every `KEEPALIVE_INTERVAL_MS = 1_000` (`shared/tab-title.ts:37,43-61,88-93`), unless `AGENT_PEERS_DISABLE_TAB_TITLE=1`.
   - The OpenCode stdio child is spawned with piped stdio but is not detached (`[oc 1.18.1] packages/opencode/src/mcp/index.ts:347-357`, no `detached` option passed). INFERRED: it keeps the controlling terminal, so these writes reach the pane once a second, in between OpenTUI's frame writes.
   - INFERRED: OSC sequences draw nothing, but they could split a frame write, and they overwrite whatever title OpenCode or Herdr sets.
6. **Codex-specific instructions** are shown to an OpenCode model, and the tool name in them is unprefixed while OpenCode exposes `agent-peers_check_messages` (see Injection mechanism).
7. **Delivery depends on model compliance** with a per-turn "call check_messages first" rule. No OpenCode hook enforces it, so a long autonomous turn never sees mail unless the model touches agent-peers.
8. **No priorities or thresholds,** and FIFO by id (`broker.ts:909`). Even if a wake were added, every message would be equally urgent.
9. **macOS-centric broker ownership** (launchd). On Linux, where this repo runs OpenCode, the broker needs `AGENT_PEERS_SPAWN_BROKER=1` or a hand-written unit (`README.md:774`: "Linux should work but untested").

## Clever ideas worth stealing

INFERRED, judged against OpenCode's surfaces.

- **Confirm-on-next-call, with an arrival-causality barrier and abort rollback** (`codex-server.ts:478-560`, `shared/delivery-state.ts`). This is a clean at-least-once rule for any pull channel where the harness gives no delivery receipt.
  - In OpenCode, a herdr-threads server plugin could use the stronger signal it has: `tool.execute.after` / `experimental.chat.messages.transform` run in-process, per session (`sessionID` in the hook input).
  - The idea carries over (ack only after evidence the step completed), but the evidence should be tied to the session.
- **Body-free preview kept separate from authoritative delivery,** so a client that surfaces both cannot trigger a double reply (`shared/piggyback.ts:15-33`). This maps onto herdr-threads R18, content-free wakes. For OpenCode, the preview's natural home would be a TUI-plugin `api.ui.toast` for p > T items (primer section 7). That is a non-model signal, which agent-peers cannot reach through MCP.
- **Short-circuiting the long-poll when the session is wakeable** (`codex-server.ts:788-803`, reply text at `734-749`). Do not let the model pin a turn waiting when an external waker exists. This is relevant if herdr-threads ever offers a blocking `wait` tool alongside an OpenCode plugin wake.
- **Persist before present, and never ack on draw.** The durable inbox is written before any notification (`codex-server.ts:386-397`). herdr-threads already keeps the canonical state in the daemon; the useful discipline is that the client never acks a message at draw time.
- **Counter-example.** The 483 B per-block behavioural header and the 8.5 KB instructions are what *not* to do under herdr-threads' token rules (a) and (b). In OpenCode, static guidance belongs once in a skill (`SKILL.md`, which OpenCode also loads from `.claude/skills`, primer section 6) or in MCP `instructions`, never per delivery.

## Verification

Checked against the agent-peers clone at `2c63379d`, the OpenCode `v1.18.1` source and the local MCP SDK `protocol.js`. Confirmed: no OpenCode references; the closed `PeerType` and SQL CHECK; runtime selection; the poll cadence; persist-before-push; the piggyback order (confirm, ack flush, inline poll, draw, prepend); `check_messages` being a no-op; the body-free preview; the wait defaults; no progress notifications; OpenCode's tool wrapping, name prefixing, logging-only notification handler, `<mcp_instructions>` placement, truncation limits, env merge, per-Instance clients, timeout resolution and SDK 1.29.0 pin; the hardcoded `droid exec` spawn; the `launchctl` broker; the tab-title writes.

Changed:
- Header: 7 lines and 483 B, not 8 lines and 584 B. Per-message framing cost cut to about 170 tokens.
- Broker cap: the 500 limit applies only to stale durable recipients; a live peer is uncapped. Citations fixed.
- Permission: the "unknown" default is now VERIFIED as allow (`agent.ts:119-120`).
- Added the Codex desktop-wake path. It arms under OpenCode but stays dormant, because there is no `_meta.threadId`.
- The `-codex` name suffix is now cited to code, not `ARCHITECTURE.md` (which does not state it).
- Corrected line ranges: `withPiggyback`, the `confirm` and `rollback` blocks, the `session/tools.ts` lines, `mcp/index.ts`, `system.ts`, `ensure-broker.ts`, `droid-acp-client.ts`, the wait short-circuit, the wake-daemon RPCs, and the tool descriptions.
- Noted that the banner text is `PEER INBOX`, without brackets.

Not re-checked: the primer-sourced claims (sections 2, 4, 6, 7, 10, 11) and the byte counts of the static instructions. Those match the clone at about 5.7 KB, 2.8 KB and 2.6 KB.
