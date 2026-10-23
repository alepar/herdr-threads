# agent-peers-mcp (mikez93) x Antigravity CLI (agy)

- Upstream: https://github.com/mikez93/agent-peers-mcp, read at `2c63379d65ad44845815faa21a0c5c78355002fa`. Paths are relative to the clone root.
- Harness facts come from `../../primers/agy.md` (cited as "primer §N").
- Tags: VERIFIED = read in source, docs or the primer's sources. INFERRED = my reasoning. unknown = nothing answers it.

## Support status

**Not supported.** VERIFIED. The evidence:

- A case-insensitive grep for `agy|antigravity|gemini` over the whole clone (excluding `.git`, `node_modules`) matches one line only: `"gemini"` in a word list of persona/name tokens (`shared/peer-identity.ts:15`). No server, launcher, hook, doc or test mentions agy.
- The broker schema allows only four peer types: `peer_type TEXT NOT NULL CHECK(peer_type IN ('claude', 'codex', 'hermes', 'droid'))` (`broker.ts:145`, repeated in the migrations at `broker.ts:307,454`). A first-class `agy` peer would need a schema migration. The tool schemas list the same enum (`codex-server.ts:235`, `claude-server.ts:153`).
- The per-session MCP servers are `claude-server.ts`, `codex-server.ts`, `hermes-server.ts` and `droid-server.ts`. `droid-server.ts` is a thin wrapper that sets `AGENT_PEERS_RUNTIME` and imports `codex-server.ts` (`droid-server.ts:6-9`). `hermes-server.ts` does the same by default (`hermes-server.ts:22,44`), but with `AGENT_PEERS_HERMES_V2=1` it starts a separate conversation runtime instead (`hermes-server.ts:40-42`).

**What would happen if a user wired it in anyway.** INFERRED from the code. agy speaks stdio MCP (primer §4), so the generic path is to register `codex-server.ts` as an agy MCP server:

```json
// ~/.gemini/config/mcp_config.json (or a plugin's mcp_config.json)
{"mcpServers":{"agent-peers":{"command":"bun","args":["<repo>/codex-server.ts"],
  "env":{"AGENT_PEERS_ENABLED":"1","AGENT_PEERS_WAKE_ENABLED":"0"}}}}
```

It would then run like this:

- **Peer type.** With no `AGENT_PEERS_RUNTIME`, the server registers as `peer_type: "codex"` (`codex-server.ts:103-107,984-985`). Peers would see agy as a "Codex" peer, and its instructions would say "The current Codex CLI does NOT surface…" (`codex-server.ts:177`). The identity is wrong but harmless.
- **Activation gate.** Without `AGENT_PEERS_ENABLED=1` the server lists zero tools and does nothing (`codex-server.ts:863-868`).
- **Desktop-wake probe.** `desktopWakeEligible` is true for the codex runtime unless `AGENT_PEERS_WAKE_ENABLED` is `0` or `1` (`shared/codex-desktop-wake.ts:13-16`). The server would then construct a `CodexDesktopWakeRegistration` (`codex-server.ts:937-942`) and call `observe(req.params._meta, …)` on every tool call (`codex-server.ts:785-786`). `observe` reads `_meta.threadId` (`codex-desktop-wake.ts:18-22`) and returns early when there is none (`codex-desktop-wake.ts:144`); only with a thread id does it attempt discovery once, and the `/usr/sbin/lsof` socket scan runs only on macOS (`codex-desktop-wake.ts:35-37`). Whether agy sends a `_meta.threadId` is unknown; if it does not, the probe is a no-op (INFERRED). Setting `AGENT_PEERS_WAKE_ENABLED=0` skips it entirely.

The rest of this report treats that configuration as the "generic mechanism on agy".

## Injection mechanism

### What the generic path would do

**Transport.** A tool-result piggyback: the `[PEER INBOX]` block is prepended to the response of **any** agent-peers MCP tool. VERIFIED in code. On agy, MCP tools are a supported, documented surface (primer §4), so this would work as far as the code is concerned. INFERRED; not tested on agy.

**Code path.** VERIFIED:
1. A background poll every 1 s while peer activity is recent and 3 s otherwise (`codex-server.ts:88-101`) leases messages from the broker. The query is `SELECT … WHERE to_id = ? AND acked = 0 AND (lease expired or none) ORDER BY id ASC`, with no `LIMIT` (`broker.ts:905-910`). Each lease lasts 30 s (`broker.ts:27`).
2. `applyLeasedToQueue` persists the messages to the durable inbox `~/.agent-peers-codex/<peer-id>.json` before anything else (`codex-server.ts:386-398`).
3. It then sends a signal-only `notifications/message` "preview" for each new message (`codex-server.ts:410-437`, text at `shared/piggyback.ts:27-33`).
4. On every `CallTool`, `withPiggyback` (`codex-server.ts:479-547`) runs these steps under a lock:
   - confirm the previous call's presented ids, prune them from disk and enqueue their acks;
   - flush the acks;
   - run an inline poll;
   - read the durable queue and drop ids that are blocked (drawn or presented);
   - `draw` the rest (`acquireInboxBatch`, `:551-681`).

   The handler then runs, and the response text is `formatInboxBlock(fresh) + toolText` (`:533-534`).

**When it fires on agy.** Only when the agy model calls one of the six agent-peers tools: `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages` or `rename_peer` (`codex-server.ts:227-299`). VERIFIED. Nothing is injected at turn start or between ordinary tool calls unless the model chooses to call `check_messages`. Two prompts ask it to:

- The tool description: "Call this at the START of every user turn" (`codex-server.ts:262-265`). Tool descriptions reach the agy model.
- The MCP server `instructions` field: "RULE: Call `check_messages` as the FIRST thing you do every time the user sends you a message" (`codex-server.ts:184-189`). Whether agy surfaces MCP server `instructions` to the model at all is **unknown**: primer §4 lists tools as the only documented MCP feature.

So on agy, delivery latency is "until the model next decides to call an agent-peers tool". INFERRED.

**The preview notification on agy.** Whether agy surfaces `notifications/message` to the model or the TUI is **unknown**, and nothing suggests it does (primer §4, §11.6). The preview carries no body, so it cannot cause a double read even if it is surfaced (`shared/piggyback.ts:19-26`). VERIFIED design intent.

**Payload format.** VERIFIED, `shared/piggyback.ts:35-62`:

```
PEER INBOX — N unread message(s) from your colleagues.
Read each one. Then:
  - If it's a question you can answer now: reply briefly via send_message(to_id="<from_name>", ...).
  - ...(4 more instruction lines)...
Do NOT auto-acknowledge. "Got it" / "on it" is noise.

--- message 1 of N ---
from: <name> (<peer_type>, cwd=<cwd>)
their current work: <summary>        (optional)
sent_at: <iso>
message_id: <id>
text:
<body>
reply_action: send_message(to_id="<name>", message="...")

PEER INBOX
```

On agy the block would arrive as an ordinary MCP tool result (`MODEL/GENERIC` in the transcript; primer §7). It would not be a `SYSTEM_SDK` step, so nothing on the agy side marks it as peer input. INFERRED.

**Size bounds.** VERIFIED:
- No per-body limit: grep finds no `text.length` check or truncation in `broker.ts`, `shared/*.ts` or `codex-server.ts`.
- No per-page limit: the poll has no LIMIT (`broker.ts:905-910`), and every unblocked queued message is drawn into one response (`codex-server.ts:659-672`).
- The only cap is 500 undelivered messages (`broker.ts:55`), enforced at send time. It applies unconditionally to conversation mailboxes (`broker.ts:802-809`), but for an ordinary peer only while that peer is stale: a peer whose `last_seen` is within the stale cutoff is accepted without a count check (`broker.ts:838-843`). A wired-in agy would be an ordinary, live, polling peer, so its sends are effectively uncapped.

So one tool result has no fixed upper bound on message count or bytes (INFERRED from the above). agy documents no MCP tool-result size limit, so its behaviour on a very large result is unknown.

**Framing.** VERIFIED: a 7-line behavioural header repeated on every delivery, plus 6-8 lines of fields per message (above). Every value is plain text with no escaping. A body that contains `--- message 2 of 3 ---` or `PEER INBOX` could spoof the structure (INFERRED).

**De-duplication and acknowledgement.** VERIFIED, `shared/delivery-state.ts`; `codex-server.ts:1-55,551-581`.
- **Lifecycle.** Each message moves through drawn → presented → confirmed, and draws are made under a lock so that parallel tool calls never deal the same id twice (`codex-server.ts:472-477`).
- **Ack timing.** The broker ack is sent only when the *next* tool call arrives, which the code treats as evidence that the previous response reached the model ("arrival-causality barrier", `:490-496`).
- **Cancellation.** A cancelled request rolls its draws back (`:535-541`).
- **No semantic ACK.** There are no ACK requests that a model must answer, and the colleague protocol discourages replies like "got it" (`shared/piggyback.ts:47`).

**On agy, the confirm rule is weaker than it looks.** INFERRED. "The next agent-peers tool call" is the *only* confirmation evidence. If the agy model reads a `[PEER INBOX]` and never calls an agent-peers tool again in that session, the message stays on disk unconfirmed. The broker then re-offers it every 30 s lease cycle, and the triage refreshes the token (`codex-server.ts:363-382`). The same id is not re-dealt while it is "presented", so the model does not see a duplicate. But the message is never acked.

## Wake mechanism

**None for agy.** VERIFIED. The project has exactly four wakers, and each depends on a harness-specific API:

| Waker | Harness API it needs | Present on agy? |
|---|---|---|
| Codex wake daemon: `turn/start` / `turn/steer` over a managed `codex app-server` (`shared/wake-daemon.ts`, `shared/app-server-client.ts`) | Codex app-server JSON-RPC | No: agy has no `app-server`, `serve` or `rpc` subcommand (primer §5) |
| Codex Desktop exact-thread registration (`shared/codex-desktop-wake.ts`) | Codex `_meta.threadId` plus a parent unix socket found with `lsof` | No |
| Droid ACP or native relay (`shared/droid-native-relay.ts`, `wakeable-droid.ts`) | ACP / Factory daemon IPC | No: agy has no native ACP (primer §5) |
| Claude `bin/claude-peer-rewake` (`asyncRewake` Stop hook, exits 2 to wake) | Claude Code `asyncRewake` hook semantics (`bin/claude-peer-rewake:10-15`) | No: per primer §2.3, stdout decides and exit codes 1/2 behave like 0 (third-party, symposium.dev; agenticcontrolplane disagrees), and the primer documents no background or rewake hook mode (absence INFERRED) |
| Hermes T2 host RPCs (`shared/hermes-conversation-wake.ts`) | Pinned Hermes RPCs | No |

**The only "wake" available would be the in-turn long-poll.** `wait_for_peer_messages` keeps the *current* turn alive. It cannot wake an idle agy. VERIFIED: its own description says "it is not a fully idle wake mechanism" (`codex-server.ts:268-271`). The loop polls every 500 ms until a fresh unread message appears or the deadline passes (`shared/wait-for-peer-messages.ts:58-80`). Its default and maximum is 300 s for the codex runtime (`codex-server.ts:122-123`).

**Idle/busy detection, draft safety and latency on agy.** These do not apply. With no waker, an idle agy sitting at its composer stays idle until a human types. Mail then reaches it only if that turn calls an agent-peers tool. Draft safety holds trivially, because nothing is ever typed. INFERRED.

**Failure modes specific to agy.** INFERRED unless noted.

1. **Long-poll timeout mismatch can lose a message.**
   - The server's wait defaults to 300 s. agy's default MCP per-tool-call timeout is 180 s (primer §4).
   - agy would give up on the call first. What happens next depends on whether agy then sends MCP `notifications/cancelled`, which is **unknown**.
   - If it does, the SDK aborts `extra.signal`. The wait loop ignores the signal (`wait-for-peer-messages.ts:58-80` never checks it), but `withPiggyback` sees `signal.aborted` and rolls the draw back (`codex-server.ts:535-541`). Nothing is lost.
   - If agy does **not** send a cancellation, a message that arrives at, say, t = 200 s is drawn into a response that agy has already discarded. That response is promoted (`:544`), and the next agent-peers call confirms it, prunes it from disk and acks it (`:569-578`). The message is then lost silently, although the project's contract is at-least-once.
   - Workaround: have the model pass `timeout_ms` < 180000, or set agy `timeoutSeconds` above 300 for this server (primer §4).
2. **Delivery depends on the model choosing to call `check_messages`.** If agy does not surface the MCP `instructions`, the only prompt left is the tool description.
3. **Possible MCP approval prompts.** MCP tool calls are subject to agy permission rules (`mcp(server/tool)`, primer §4). In `request-review` mode an approval prompt might appear for each call (unknown default). Every inbox check could then need a human click.

## Setup / installation footprint on the harness

The upstream project writes nothing for agy. VERIFIED: there is no agy installer.

A manual wiring would need:
- one `mcpServers.agent-peers` entry in `~/.gemini/config/mcp_config.json`, in `.agents/mcp_config.json`, or in a plugin's `mcp_config.json` (primer §4);
- `AGENT_PEERS_ENABLED=1` in that entry's `env` (`codex-server.ts:859-868`).

Shared state outside the agy config (VERIFIED, overview "Architecture"):
- the broker on `127.0.0.1:7900`;
- `~/.agent-peers.db`, `~/.agent-peers-secret`;
- `~/.agent-peers-codex/<peer>.json` plus `<peer>.metadata.json`.

Broker ownership assumes launchd on macOS. Clients run `launchctl kickstart` and spawn the broker themselves only with `AGENT_PEERS_SPAWN_BROKER=1` (overview, `docs/ARCHITECTURE.md:42-72`). The server calls `ensureBroker` at startup (`codex-server.ts:922-923`).

There is also a side effect on the agy pane. VERIFIED in code; the impact on agy is INFERRED:
- Every second, the MCP server writes OSC 0/1/2 title sequences to `/dev/tty` (`shared/tab-title.ts:37-76`; started at `codex-server.ts:919-920`).
- Under agy that tty is the agy pane's tty, and the agy statusline and window title feature (primer §9) could fight over the title. Whether Herdr's agy detection reads the pane title is unknown.
- `AGENT_PEERS_DISABLE_TAB_TITLE=1` turns it off (`tab-title.ts:53`).

## Token cost per message and per quiet turn

All figures are estimates (INFERRED, about 4 bytes per token).

- **Static, per session:**
  - Six tool schemas with descriptions, roughly 2 KB, about 500 tokens (`codex-server.ts:227-299`).
  - If agy surfaces MCP `instructions` (unknown), add the delivery prologue (about 2.5 KB, `:173-223`) and `COLLEAGUE_PROTOCOL`, which is most of the 6.4 KB `shared/colleague-prompt.ts`. That is about 2,000-2,500 tokens more.
- **Per delivery:**
  - The header is about 440 bytes (about 110 tokens), sent once per block.
  - Each message adds about 200 bytes of fields (from/cwd, sent_at, message_id, reply_action), about 50 tokens, plus the body.
  - Then the tool-call round trip itself: the model's `check_messages` call plus the "Checked inbox." result, about 30-60 tokens.
  - For comparison, herdr-threads frames each item with a one-line header and states its rules once, in the skill (AGENTS.md R17).
- **Per quiet turn:**
  - If the model obeys "call check_messages every turn", each quiet turn costs one tool call and the `Checked inbox.` text, about 30-60 tokens plus a model round trip.
  - If it does not, the cost is zero and the mail is not delivered.
  - herdr-threads' agy `PreInvocation` path emits nothing on quiet turns (primer §10).

## Limitations and bugs you can see

1. **No agy support at all.** The `peer_type` CHECK constraint blocks a first-class agy peer (`broker.ts:145`), so a wired-in agy would register as `codex` (VERIFIED).
2. **Delivery is pull-only and model-discretionary on agy.** agy's own push surfaces go unused: `PreInvocation` `injectSteps`, which fires before every model call, including mid-turn after tool results (primer §2.2, §2.5), and `Stop` `continue`. INFERRED.
3. **No idle wake on agy.** Every waker is harness-specific (table above). VERIFIED.
4. **Possible silent loss when the long-poll outlives agy's tool timeout** (300 s vs 180 s), if agy does not send `notifications/cancelled`. INFERRED. A related point: the wait loop never checks the abort signal (`shared/wait-for-peer-messages.ts:58-80`, VERIFIED), so a cancelled wait still holds its MCP request slot until the deadline.
5. **No bounding.** There is no body truncation and no page cap, and the 500-message send cap does not apply to a live ordinary peer, so one tool result can carry an unbounded number of full bodies (`broker.ts:838-843,905-910`; `codex-server.ts:659-672`). VERIFIED mechanism.
6. **Verbose framing repeated on every delivery**, and the plain-text structure can be spoofed by message bodies (`shared/piggyback.ts:40-62`). VERIFIED framing; spoofing is INFERRED.
7. **Confirmation needs a second agent-peers call.** A model that reads one inbox and never calls again leaves the message unacked and re-leased every 30 s for the life of the session. It is not re-shown, but it is never closed (`codex-server.ts:363-382,569-578`). VERIFIED mechanism; impact INFERRED.
8. **Identity follows the MCP process, not the agy conversation.** One `codex-server.ts` process per agy MCP spawn means the same peer persists across `/clear` (a new `conversationId`, primer §2.2). Delivery state that was "presented" to the old conversation is confirmed by the new one. INFERRED. Whether agy subagents get their own MCP processes, and so their own peers, is unknown.
9. **Mismatched instructions**: "This is a hard constraint of the Codex runtime" would be shown to agy (`codex-server.ts:177-182`). VERIFIED text.
10. **macOS-centric.** launchd broker ownership, `/usr/sbin/lsof`, and `ps -axo` in the Claude rewake hook (`bin/claude-peer-rewake:49`). VERIFIED.

## Clever ideas worth stealing

1. **Signal-only preview versus authoritative body.** The push notification carries sender and id but never the body, so a client that surfaces both cannot double-reply (`shared/piggyback.ts:16-33`). VERIFIED. This matches our content-free wake (R18). For agy it suggests that an `ephemeralMessage` "mail waiting" hint (primer §11.3) must stay bodyless if it ever coexists with the `systemMessage` delivery.
2. **Arrival-causality barrier for confirm-on-next-call.** Confirm eligibility is snapshotted at request *entry*, so a call parked in a long wait cannot confirm a response built after it was issued (`codex-server.ts:490-496`, `shared/delivery-state.ts`). VERIFIED. herdr-threads confirms agy offers from the transcript instead (primer §10), which is stronger evidence. The barrier is still relevant if we ever add a blocking MCP tool on agy.
3. **Draw rollback on cancellation** (`codex-server.ts:535-541`). VERIFIED. It is cheap insurance for any pull path that can be aborted.
4. **The sender filter on the long-poll never consumes non-matching mail.** It only governs what ends the wait (`codex-server.ts:332-340`). VERIFIED. This is worth copying if we offer an agy-side blocking wait, which would be bounded below agy's 180 s MCP timeout. The mismatch above shows why that bound matters.
5. **A bodyless metadata file as the waker's only input** (`~/.agent-peers-codex/<peer>.metadata.json`, `shared/codex-inbox.ts:87-115,214-215,275`). VERIFIED. It separates "should we wake" from "what to deliver" at the file-permission level. Our wake path already reads only attention state; no change is needed, but this is a clean pattern.
6. **Persist the tty at registration** (`getTty`, `shared/peer-context.ts:17-31`; the `tty` column in `peers`). VERIFIED. A cheap hint for mapping a process-bound identity to a terminal. We already bind by `HERDR_PANE_ID`, which is better. INFERRED.

## Verification

Adversarial pass against the clone at `2c63379d` (GitHub API: `mikez93/agent-peers-mcp`, default branch `main`) and `../../primers/agy.md`.

Checked and held: the agy/gemini grep result; the `peer_type` CHECK at `broker.ts:145,307,454` and tool enums; runtime selection and the `codex` default (`codex-server.ts:103-107,984-985`); the activation gate (`:863-868`); the instruction and tool-description quotes (`:177-189,262-271`); the poll intervals, unbounded lease query and 30 s lease; the `applyLeasedToQueue` persist-then-preview order; the `withPiggyback`/`acquireInboxBatch` sequence, arrival barrier, rollback and promote lines; the `[PEER INBOX]` format (`shared/piggyback.ts:35-62`); the wait loop's 500 ms poll, 300 s default/max and absence of any abort-signal check; the tab-title keepalive and opt-out; `launchctl kickstart` / `AGENT_PEERS_SPAWN_BROKER`; the `asyncRewake` hook and `ps -axo`; the bodyless metadata file; primer claims for MCP config, 180 s tool timeout, no app-server/ACP, `PreInvocation` mid-turn injection.

Changed:
- 500-message cap: it applies to ordinary peers only while they are stale (`broker.ts:838-843`), so a live agy peer is uncapped; "up to 500 bodies per result" was wrong (Size bounds, limitation 5).
- Desktop-wake probe: it returns early without `_meta.threadId` and runs `lsof` only on macOS; the old text implied `lsof` ran on each call. Whether agy sends `threadId` is now marked unknown.
- `hermes-server.ts` is not always a thin wrapper (`AGENT_PEERS_HERMES_V2` branch).
- Wake table, Claude row: the exit-code claim is attributed to its third-party sources, which disagree, and "no rewake mode" is marked INFERRED, because the primer does not state it.
