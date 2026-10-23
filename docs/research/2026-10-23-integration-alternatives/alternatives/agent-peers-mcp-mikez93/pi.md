# agent-peers-mcp (mikez93) x Pi coding agent

- Alternative: https://github.com/mikez93/agent-peers-mcp, read at `2c63379d65ad44845815faa21a0c5c78355002fa`. Paths without a prefix are relative to that clone.
- Harness: Pi (https://github.com/badlogic/pi-mono, read at `a7b94a03f09f20f4937fa1a76339014c60d261ad`, coding-agent 1.4.0). Paths prefixed `pi:` are relative to the pi-mono clone.
- Harness facts not cited here come from `../../primers/pi.md`.
- Tags: VERIFIED = read in source or docs. INFERRED = my reasoning, not stated anywhere.

## Support status

**Not supported.** VERIFIED:

- The peer-type union is closed: `export type PeerType = "claude" | "codex" | "hermes" | "droid";` (`shared/types.ts:5`).
- The broker schema enforces the same union: `CHECK(peer_type IN ('claude', 'codex', 'hermes', 'droid'))` (`broker.ts:145,307,454`).
- `codex-server.ts` chooses its runtime from `AGENT_PEERS_RUNTIME`. Only `hermes` and `droid` are recognised, and anything else falls back to `codex` (`codex-server.ts:103-107`).
- A grep of the whole clone for `pi-mono`, `badlogic`, `earendil`, `pi-coding`, `PI_CODING`, `AI_AGENT`, `.pi/` and the whole word `pi` (with `api` excluded) finds nothing.
- No Pi extension, launcher or install prompt exists. The Install section covers Claude (`~/.claude.json` plus the `agentpeers` alias) and Codex (`~/.codex/config.toml`) (`README.md:61-160`); Hermes (`README.md:165-180`) and Factory Droid (`README.md:356`) have their own sections. None mentions Pi.

The rest of this report covers what happens if you take the project's generic part, the stdio MCP server, and point Pi's built-in MCP client at it. It also covers why none of its wake paths carry over.

### Which server a Pi user would use (INFERRED)

`claude-server.ts` delivers mainly through the experimental `claude/channel` notification. Pi's MCP client advertises only `roots` and has no handler for that notification (primer section 3; `pi:packages/mcp/src/client.ts:176-180,525-526`). So on Pi the Claude server is reduced to its `check_messages` fallback.

`codex-server.ts` is built for exactly this kind of host: a client whose only MCP input surface is tool results. A Pi user would therefore run `codex-server.ts` with `AGENT_PEERS_ENABLED=1` and register as peer type `codex` under a name ending in `-codex` (`codex-server.ts:949`, `shared/peer-identity.ts:89-93`). Other peers would see the Pi agent as a Codex.

## Injection mechanism

### Exact mechanism: the "piggyback" block on MCP tool results (VERIFIED)

1. **Background poll.** The poll loop (`codex-server.ts:1033-1042`) leases every unacked row from the broker. The broker query has no LIMIT and orders by id (`broker.ts:901-910`). Rows go to the durable on-disk inbox before anything else happens (`codex-server.ts:389-398`).
2. **Preview push.** The server then sends a body-free `notifications/message` "preview" for each new id (`codex-server.ts:410-436`, text built in `shared/piggyback.ts:27-33`).
   - **On Pi this goes nowhere.** Pi's MCP runtime writes `notifications/message` to `~/.pi/agent/mcp.log` and never shows it to the model (`pi:packages/coding-agent/src/extensions/mcp/runtime.ts:367`, `pi:packages/coding-agent/docs/mcp.md:98`).
3. **Delivery on the next tool call.** Each call to any agent-peers tool goes through `withPiggyback` (`codex-server.ts:479-546`). It confirms the previous response's draws (`:569-583`), flushes acks (`:597-620`), polls inline (`:631-635`), draws every unblocked queued message (`:661-677`), and then prepends the block:

   ```ts
   const inbox = formatInboxBlock(fresh);
   const finalText = inbox + toolText;          // codex-server.ts:533-534
   ```

On Pi, that text comes back as the content of an `mcp__agent-peers__<tool>` tool result.

### When it fires on Pi (VERIFIED mechanics, INFERRED consequence)

Delivery happens only when the model itself calls an agent-peers tool: `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages` or `rename_peer`. Nothing arrives at turn start or after an unrelated tool.

On Codex the server's `instructions` carry the load-bearing rule "Call `check_messages` as the FIRST thing you do every time the user sends you a message" (`codex-server.ts:173-223`, the rule at `:184-189`). **Pi does not show server instructions to the model.**

- The `mcp_servers` system-prompt section contains only the *first line* of the instructions, and only for servers with `codemode` or `deferred` tools (`pi:packages/coding-agent/src/extensions/mcp/index.ts:184-200`; `pi:docs/mcp.md:71,202`). Here that line is `YOU ARE A COLLEAGUE, NOT A TOOL.` (`shared/colleague-prompt.ts:13`).
- The full instructions reach only `tool_search` ranking and codemode's `describeNamespace()` (`pi:docs/mcp.md:234`: "Server instructions are not part of any tool description"; `pi:src/extensions/tool-search/tool.ts:114`).

What remains is the one-sentence tool description of `check_messages`, "Call this at the START of every user turn" (`codex-server.ts:262-266`). That sentence is visible only if the tool is declared to the model.

### Exposure trap (VERIFIED Pi behaviour, INFERRED effect)

Pi's default MCP exposure is `codemode`: tools are "neither declared to the model nor listed", and the model reaches them only by writing codemode scripts (`pi:docs/mcp.md:193-197`). With the default exposure:

- the model has no description telling it to poll, so delivery depends entirely on the model deciding to look;
- "Codemode scripts receive the complete result and can reduce it before returning output to the model" (`pi:docs/mcp.md:232`). A script that returns only part of the result (for example a parsed peer list) **drops the `PEER INBOX` block**. The server cannot tell the difference: the response was fully built, so `delivery.promote(callId)` runs (`codex-server.ts:544`), and the next call confirms, prunes and acks the messages (`:569-583`). The result is silent loss under an at-least-once contract.

A working Pi setup therefore needs `"exposure": "direct"` in `mcp.json`. Nothing in the project tells a user this.

### Payload format and framing (VERIFIED, `shared/piggyback.ts:35-62`)

The header repeats the reply instructions on every delivery:

```
PEER INBOX — N unread message(s) from your colleagues.
Read each one. Then:
  - If it's a question you can answer now: reply briefly via send_message(to_id="<from_name>", ...).
  ... (3 more instruction lines) ...
Do NOT auto-acknowledge. "Got it" / "on it" is noise.
```

Each message then gets these lines: `--- message i of N ---`, `from: <name> (<peer_type>, cwd=<cwd>)`, optional `their current work:`, `sent_at:`, `message_id:`, `text:`, the body, and `reply_action: send_message(to_id="<name>", message="...")`. The block closes with `PEER INBOX`.

Two Pi-specific points:

- The `reply_action` names the tool `send_message`, but on Pi the callable name is `mcp__agent-peers__send_message` (`pi:docs/mcp.md:191`). INFERRED: models usually map the short name, but the hint is technically wrong on Pi.
- The block arrives in the **tool result**, so there is no `user`-role custom message. Its trust framing is that of a tool output.

### Size bounds

- Server side: no body or page limit. The broker refuses new sends once 500 unacked messages are queued for a durable peer that is stale (offline); a live peer has no cap (`broker.ts:55,838-843,857-862`; the same limit for conversation sends at `:808-809`). The lease returns all of them (`broker.ts:901-910`). A grep for size constants in `broker.ts`, `codex-server.ts` and `shared/broker-client.ts` finds none. VERIFIED by absence.
- Pi side: MCP text results over 20 KB "reach the model with their middle removed around a `…N chars truncated…` marker". The full text goes to a temp file named in the result (`pi:docs/mcp.md:232`). VERIFIED.
- Combined effect (INFERRED): a backlog larger than 20 KB is middle-truncated by Pi, yet every drawn message is promoted and later confirmed and acked. Messages in the middle are counted as delivered when the model saw only a pointer to a temp file. Their content survives in that file, but the server cannot know the model never read it.

### De-duplication and ack (VERIFIED)

- The ack is tied to the model's next call, not to reading the response ("confirm-on-next-call"). Messages drawn into response N are acked to the broker at the start of response N+1 (`codex-server.ts:551-583`). The ack carries no meaning for the agent's work, and the colleague prompt discourages replies that only acknowledge (`shared/piggyback.ts:47`).
- An arrival-generation barrier stops a call that was parked in a long wait from confirming a response built after it started (`codex-server.ts:491-497`).
- A lock serialises the draw, so two tool calls issued in parallel in one Pi turn cannot both draw the same id (`codex-server.ts:469-477`).
- **Cancellation is handled correctly on Pi.**
  - Pi's client sends `notifications/cancelled` on timeout or abort (`pi:packages/mcp/src/client.ts:553-575`).
  - The SDK aborts `extra.signal` when it receives it.
  - The server then rolls back the draws instead of promoting them (`codex-server.ts:535-541`).
- If the agent-peers server restarts, its in-memory dedupe state is lost. The project's contract still admits duplicate presentation (lease expiry, crash between persist and ack); the on-disk store dedupes by message id (`docs/delivery-contract.md:20-23`).

## Wake mechanism

**There is none for Pi.** VERIFIED. Each of the project's wake paths depends on a specific harness:

| Wake path | Why it does not reach Pi |
|---|---|
| Codex `wake-daemon.ts` → `turn/start` / `turn/steer` over `codex app-server` | Needs a Codex app-server thread bound through `codex-peer` launch claims (`shared/wake-daemon.ts:288-298`, `shared/wake-launch-claims.ts`). Pi has no app-server or socket, and RPC mode replaces the TUI and cannot be attached to a running Pi (primer section 4). |
| Claude `bin/claude-peer-rewake` (`asyncRewake` Stop hook) | Pi has no subprocess-hook system; its extension events run in-process (primer section 1). |
| Droid ACP or native relay | Droid-specific. Pi has no ACP (primer section 4). |
| Hermes T2 host RPC | Hermes-specific. |
| `notifications/message` preview | Logged to `mcp.log` and never shown to the model (above). |

### Long-poll fallback: `wait_for_peer_messages` on Pi

This tool keeps the *current* turn alive. It does not wake an idle session (`codex-server.ts:268-290`, `shared/wait-for-peer-messages.ts:58-80`).

- **Timeout mismatch.** On a `codex` runtime the tool's default and maximum wait are 300 s (`codex-server.ts:122-123`). Pi's per-request MCP timeout defaults to 60 s and is reset only by progress notifications (`pi:docs/mcp.md:68`; `pi:src/extensions/mcp/runtime.ts:210-211`). The server never sends progress during the wait (grep: no `progress` in `codex-server.ts`). VERIFIED.
- **Consequence (INFERRED from the verified pieces).**
  - A default call fails in Pi after 60 s with "Request timed out", and Pi sends `notifications/cancelled`.
  - The server keeps waiting anyway: the wait loop takes no abort signal (`shared/wait-for-peer-messages.ts:3-12,58-80`; the call site passes only timeout and sender, `codex-server.ts:805-808`). When that late call finishes it rolls back, so nothing is lost. The cost is a parked call and a failed tool call in the transcript.
  - The fixes are `"timeout": 300` in `mcp.json`, or a model that passes `timeout_ms <= 55000`.
- **Busy, idle and draft handling.** These questions do not arise: the model is mid-turn by definition, and nothing is typed into the TUI.
- **Latency.** 500 ms poll inside the wait (`codex-server.ts:124`).

### Idle/busy detection and draft safety

Not applicable: nothing is ever injected into an idle Pi. Pi gives an extension exactly what such a feature would need, namely `ctx.isIdle()`, `agent_settled` and `ctx.ui.getEditorText()` (primer section 2.4). The project does not use any of it.

### Failure modes (INFERRED)

- An idle Pi agent receives nothing until a human types and the model happens to call an agent-peers tool. The model has no instruction to do so, because the instructions are dropped (above).
- Senders get no signal that the recipient is not wakeable. `send_message` success means "queued" (`docs/delivery-contract.md:6-27`), and the Pi peer is labelled `codex`, so other agents may assume Codex-grade wake behaviour.

## Setup and installation footprint on Pi

There is no supported path. A hand-rolled setup would need the following (INFERRED, assembled from verified pieces):

- Clone the repo and run `bun install`. On macOS, also install the launchd broker (`com.mike.agent-peers-broker`). On Linux there is no launchd, so set `AGENT_PEERS_SPAWN_BROKER=1` so that the client spawns the broker (`shared/ensure-broker.ts:14,38-57`; `README.md:190,774`).
- Shared state the project writes: `~/.agent-peers.db`, `~/.agent-peers-secret`, and `~/.agent-peers-codex/<peer>.json` plus `.metadata.json` (overview).
- Add an entry to `~/.pi/agent/mcp.json`, or use `pi mcp add`, or `.pi/mcp.json` after project trust (`pi:docs/mcp.md:32`):

  ```json
  { "mcpServers": { "agent-peers": {
      "command": "bun", "args": ["<dir>/codex-server.ts"],
      "env": { "AGENT_PEERS_ENABLED": "1", "PEER_NAME": "..." },
      "exposure": "direct", "timeout": 300 } } }
  ```

  The project's own convention (`README.md:416`) is a global registration plus an env gate, so that the MCP is a no-op unless activated (`codex-server.ts:863`). On Pi, putting `AGENT_PEERS_ENABLED` in the `mcp.json` `env` enables it for **every** Pi session. Gating per session would need `env: {"AGENT_PEERS_ENABLED": "${AGENT_PEERS_ENABLED}"}` and an alias. VERIFIED: Pi expands `${VAR}` in `env` (`pi:packages/coding-agent/docs/mcp.md:81`). Unknown: whether an unset variable becomes an empty string is unknown.
- **Terminal side effect.** The server rewrites the terminal title through `/dev/tty` every second (`shared/tab-title.ts:37,52-63,91-93`). It does this from a child process that shares Pi's controlling terminal. Pi also sets its own title (`ctx.ui.setTitle`, primer section 7). INFERRED: the two fight over the title, and out-of-band writes into a full-screen TUI's tty are risky. `AGENT_PEERS_DISABLE_TAB_TITLE=1` turns this off (`shared/tab-title.ts:53`).
- **Session switches.** Pi closes every MCP connection on `session_shutdown` and reconnects on `session_start`, and both fire on `/new`, `/resume`, `/fork` and reload (`pi:src/extensions/mcp/index.ts:1114-1124,1252-1258`). So the agent-peers process restarts and re-registers.
  - Without `PEER_NAME`, the agent gets a new ephemeral UUID, and mail to the old one is orphaned once the row ages out (overview, identity section).
  - With `PEER_NAME`, the agent reclaims the old UUID within 60-90 s.
  - INFERRED: one peer identity therefore follows the Pi *process and name*, not the Pi session id.

## Token cost

These are estimates at about 4 characters per token. INFERRED, from `shared/piggyback.ts:35-62`.

- **Per message.** About 480 bytes of fixed header per *block* (about 120 tokens), plus about 200 bytes of per-message scaffolding (about 50 tokens: from, cwd, sent_at, id, reply_action), plus the body. There is no truncation below Pi's 20 KB cap.
- **Per quiet turn.**
  - If the model never calls the tools (likely on Pi, see above): 0 tokens, and 0 delivery.
  - If it does follow the "call `check_messages` every turn" description: one tool call plus a `Checked inbox.` result (`codex-server.ts:729-731`), about 30-60 tokens including call overhead, every turn.
- **Static.**
  - With `direct` exposure: six tool schemas declared every request. The `list_peers` description and `wait_for_peer_messages` alone are several hundred bytes.
  - With `codemode` exposure: one line in `mcp_servers`.
  - The 6.4 KB colleague protocol (`shared/colleague-prompt.ts`) mostly never reaches a Pi model, which saves tokens and loses behaviour at the same time.

## Limitations and bugs visible for Pi

1. Delivery depends on the server's `instructions`, which Pi does not show to the model. The only remaining poll cue is one tool-description sentence, and only under `direct` exposure. VERIFIED (Pi side), INFERRED (effect).
2. Under the default `codemode` exposure, a script that trims a tool result loses the inbox while it is still confirmed and acked. VERIFIED mechanics, INFERRED loss.
3. A backlog over 20 KB is middle-truncated by Pi but treated as fully delivered. VERIFIED mechanics, INFERRED loss of visibility.
4. The 300 s default wait exceeds Pi's 60 s MCP timeout. There are no progress heartbeats, and the server's wait loop ignores cancellation. VERIFIED.
5. There is no wake and no mid-turn delivery except on the model's own agent-peers calls. Pi's `sendMessage({triggerTurn})` and `tool_result` hooks go unused. VERIFIED absent.
6. The peer type is hard-coded to the 4-value union, so a Pi agent masquerades as `codex`. VERIFIED (`shared/types.ts:5`, `broker.ts:145`).
7. The `reply_action` hint uses an unprefixed tool name that is not callable verbatim in Pi. VERIFIED (`pi:docs/mcp.md:191`).
8. The once-per-second `/dev/tty` title writes run concurrently with Pi's TUI. VERIFIED behaviour, INFERRED risk.
9. Linux broker ownership is untested upstream (`README.md:774`).

## Clever ideas worth stealing

- **Confirm-on-next-call with rollback on cancel** (`codex-server.ts:535-583`). For a pull-only host, the next request is the only evidence that the previous response arrived, and an aborted request rolls its draws back. It works unchanged against Pi's client, which does send `notifications/cancelled`. This is a sound fallback ack rule for any MCP-tool delivery path we might add, provided the host cannot edit results before the model sees them (it fails under codemode or truncation, above).
- **Arrival-generation barrier** (`codex-server.ts:491-497`). The confirm eligibility is snapshotted at request entry, so a call parked in a long poll cannot confirm a sibling's later response. Worth copying if we ever ship a blocking "wait for mail" tool.
- **Draw under a lock, with handlers outside it** (`codex-server.ts:469-477,498-517`). Parallel tool calls in one turn cannot double-present an item, and slow handlers do not serialise.
- **A body-free preview, separate from the authoritative body**, so that a host which *does* render previews does not see the same body twice (`shared/piggyback.ts:16-33`). This matches our R18. On Pi the preview's natural home would be an extension's `sendMessage` without a trigger, not MCP logging.
- **The lesson for a herdr-threads Pi adapter** (INFERRED). This project's experience confirms the primer's conclusion: on Pi, MCP alone gives delivery only when the model chooses to pull, and its server instructions are dropped. A real integration has to be an extension loaded with `pi -e`:
  - `before_agent_start` for turn-start delivery;
  - `tool_result` append or streaming `sendMessage` steer for in-turn delivery;
  - `sendMessage({triggerTurn:true})` gated on `ctx.isIdle()` and an empty `getEditorText()` for wake;
  - bounded framing applied by us before Pi's 20 KB tool-result cut.

## Verification

Checked against the agent-peers clone at `2c63379d`, the pi-mono clone at `a7b94a03` (coding-agent 1.4.0) and `../../primers/pi.md`. Confirmed: peer-type union and broker CHECK, runtime selection, `-codex` naming, absence of any Pi reference, poll/lease/persist/preview path, `withPiggyback` lock, arrival barrier, confirm/rollback/promote lines, instruction text and `check_messages` description, block format, 300 s wait and 500 ms poll, no progress or abort in the wait, tab-title keepalive, broker spawn gate, and on the Pi side: roots-only capabilities, `mcp.log` routing, `notifications/cancelled` on timeout, 60 s default timeout, codemode default exposure, `mcp_servers` first-line summary, 20 KB middle truncation, `mcp__<server>__<tool>` naming, session_start/shutdown reconnect.

Changed:
- README support claim: Hermes and Droid have their own sections; the install range was off.
- The inbox banner is `PEER INBOX`, not `[PEER INBOX]` (the bracketed form appears only in the preview and instruction text); the header has 3, not 4, elided instruction lines; header size ~480 bytes, not ~350.
- The 500-message cap applies only to a stale durable target; citation corrected.
- `shared/wait-for-peer-messages.ts` is 80 lines; citations moved to `:58-80`.
- Restart dedupe overstated as "prevents double delivery"; now matches the contract doc.
- `${VAR}` expansion is documented (mcp.md:81, not :79); marked VERIFIED.

