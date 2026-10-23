# agent-peers-mcp (mikez93) x Hermes Agent: message injection and wake

- Upstream: https://github.com/mikez93/agent-peers-mcp, read at `2c63379d65ad44845815faa21a0c5c78355002fa` (2026-10-21). It is a fork of `Co-Messi/agent-peers-mcp`; GitHub API on 2026-10-23 reports 0 stars, 0 forks, last push 2026-10-22. Paths below without a prefix are relative to that clone.
- Hermes reference: the primer `../../primers/hermes.md` and a shallow clone of https://github.com/NousResearch/hermes-agent at `f3b5b980d6158794beb3d1044fd7b8ca4b8f0b23` (2026-10-23). Paths in it are written `hermes-agent:<path>`.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from them. **unknown** means not found. Nothing was run.

---

## 1. Support status

There are two Hermes modes, and they are very different.

| Mode | Selected by | Injection | Wake | Runs on upstream Hermes? |
|---|---|---|---|---|
| **Legacy** (default) | `hermes-server.ts` without `AGENT_PEERS_HERMES_V2=1` | MCP tool results (`[PEER INBOX]` piggyback), same transport as Codex | **none**. The only option is a bounded 60 s `wait_for_peer_messages` inside a turn | yes. It is a plain stdio MCP server |
| **v2 / T1-T2** (opt-in) | `AGENT_PEERS_HERMES_V2=1` | the same piggyback, one identity per Hermes conversation, taken from host-supplied `params._meta` | T2 only: host RPC `session.wake_admit` driven by `session.lifecycle_snapshot` | **no**. It needs a patched Hermes host. See below |

Evidence:
- `hermes-server.ts:40-45` (VERIFIED) is the whole entry point. It sets `AGENT_PEERS_RUNTIME=hermes` (line 22), applies kill switches (lines 24-38), and then either imports the v2 runtime or imports `./codex-server.ts`. So legacy Hermes is the Codex server with a few `RUNTIME_PEER_TYPE === "hermes"` branches.
- `docs/delivery-contract.md:73-89` (VERIFIED) describes legacy Hermes: "**Bounded polling, never push.** Waits cap at **60s** … `check_messages` is canonical."
- `docs/hermes-conversation-t1-activation.md:19-20` (VERIFIED) says of v2: "**No v2 autonomous wake.** T1 live acceptance is identity/status and private read/ack only. T2 adds actual lifecycle and exact queued wake."
- `docs/hermes-conversation-t1-activation.md:24-31` (VERIFIED) names the host side. v2 needs a "Paired host source supplied by Kepler9672: `7ec7864550549968fb226d6dba568d367caf55b4` … package `~/Hermes/maintainers/kepler/patches/hermes-conversation-meta-20260909-home`". That package forces `AGENT_PEERS_HERMES_BACKEND_ID` and `AGENT_PEERS_HERMES_HOME` into stdio children. `shared/hermes-gui-lifecycle-bridge.ts:1` says "Lifecycle/wake translator pinned to Hermes 65d7a5b06a (Kepler9908)".
- **Upstream Hermes does not have the v2 host surfaces.** VERIFIED as far as these checks go:
  - `gh api repos/NousResearch/hermes-agent/commits/65d7a5b06a` returns "No commit found for SHA".
  - GitHub code search on the default branch finds no `lifecycle_snapshot` and no `hermes_attachment`. `wake_admit` matches only a test name, `test_native_completion_wake_admits_…`.
  - A grep of the upstream clone for `wake_admit|lifecycle_snapshot|wake_reconcile|hermes_attachment|AGENT_PEERS|hermes/conversation_id|hermes/backend_id` finds nothing relevant.
  - Upstream's MCP `call_tool` passes only `arguments` and an optional progress callback, with no `_meta` identity fields (`hermes-agent:tools/mcp_tool_handlers.py:446-448`).
  - So v2's strict parser would throw `session_context_required` on every tool call (`shared/hermes-conversation-context.ts:34-39`; `shared/hermes-conversation-adapter.ts:82`). It is INFERRED that v2 would therefore be unusable on stock Hermes.
- Maturity: v2 is "Opt-in source, not deployed by this change" (`docs/hermes-conversation-bindings.md:8`). The live acceptance it lacks is listed: "This does **not** prove 50 real Hermes chats, an actual host lifecycle feed, backend SIGKILL/restart, live idle wake" (`docs/hermes-conversation-bindings.md:174-175`). VERIFIED.

**Net:** on stock Hermes this tool supports **MCP pull injection only, with no idle wake**. Everything wake-related for Hermes lives on a private patched host and is unproven live.

---

## 2. Injection mechanism

### 2.1 Legacy mode (what stock Hermes gets)

**Surface:** MCP **tool results**. The server registers six tools: `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages`, `rename_peer` (`codex-server.ts:229-291`, VERIFIED). Every call goes through `withPiggyback` (`codex-server.ts:479-548`, VERIFIED). It prepends a `[PEER INBOX]` block of unread mail to the tool's own result text:

```ts
// codex-server.ts:533-534
const inbox = formatInboxBlock(fresh);
const finalText = inbox + toolText;
```

The `check_messages` handler itself does nothing (`codex-server.ts:729-732`: "Piggyback already polled + injected; nothing extra to do.").

**When it fires:**
- Only when the Hermes model calls one of these tools. That can happen mid-turn, since the result arrives at that tool boundary, but never on its own initiative.
- The tool description asks for a call "at the START of every user turn" (`codex-server.ts:262-265`, VERIFIED).
- The longer "RULE: Call `check_messages` as the FIRST thing you do every time the user sends you a message" lives in the MCP server `instructions` (`codex-server.ts:173-219`). I found no code in upstream Hermes that puts an MCP server's `InitializeResult.instructions` into model context. `initialize_result` is used only for capabilities (`hermes-agent:tools/mcp_tool_registration.py:187-204`), and a grep for `instructions` in the MCP tooling finds only the MCP Events feature. INFERRED, medium confidence: **on Hermes the model never sees the colleague protocol or the "call check_messages every turn" rule.** It sees only the six tool descriptions and the header repeated inside each `[PEER INBOX]` block.

**Background poll versus presentation:**
- The MCP process polls the broker on a timer: 1 s for 120 s after peer activity, else 3 s (`codex-server.ts:94-101`, `1036-1044`). Each poll leases rows for 30 s, persists them to the on-disk inbox `~/.agent-peers-hermes/<peer-uuid>.json` plus a bodyless `<peer>.metadata.json` (`shared/codex-inbox.ts:85-115`), and sends a body-free MCP `notifications/message` preview (`codex-server.ts:398-433`).
- Upstream Hermes writes MCP logging notifications to `agent.log` only, never to the model (primer section 8, `hermes-agent:tools/mcp_tool_health.py:73-97`, VERIFIED there). **So on Hermes the preview push is dead weight.**
- At tool-call time `acquireInboxBatch` runs an inline poll as well (`codex-server.ts:623-640`), so the inbox is current at the instant of the call.

**Payload format:** `shared/piggyback.ts:35-63` (VERIFIED).

```
PEER INBOX — 2 unread message(s) from your colleagues.
Read each one. Then:
  - If it's a question you can answer now: reply briefly via send_message(to_id="<from_name>", ...).
  - If it needs investigation: go investigate. ...
  - If it's FYI: note it internally, don't auto-reply. Silence is fine.
  - If you disagree: push back with a specific reason.
Do NOT auto-acknowledge. "Got it" / "on it" is noise.

--- message 1 of 2 ---
from: kepler-repo-claude (claude, cwd=/Users/…/repo)
their current work: <summary, optional>
sent_at: 2026-…
message_id: 1234
text:
<full body>
reply_action: send_message(to_id="kepler-repo-claude", message="...")

…

PEER INBOX
```

**Size bounds:**
- The broker poll returns **every** unleased unacked row with no `LIMIT` (`broker.ts:901-910`, VERIFIED).
- Unread mail is capped at 500 per durable peer (`broker.ts:55`).
- I found no per-message or per-block byte limit in `shared/piggyback.ts` or `broker.ts`. INFERRED absent.
- The only effective bound is Hermes' own MCP result cap: 2,000,000 chars, head/tail truncated (`hermes-agent:tools/mcp_tool_content.py:25-34`, VERIFIED). At 500 unread messages that cap is enormous. INFERRED.

**Framing:**
- A 7-line behavioural header is repeated on **every** non-empty block (about 483 bytes of fixed text, measured from `piggyback.ts:40-47`).
- Each message adds 6-7 lines of metadata, including the sender's absolute `cwd` and a `reply_action` hint.
- The order is FIFO by broker id. There are no priorities (`broker.ts:909`).

**De-duplication and ack (confirm-on-next-call):** VERIFIED `codex-server.ts:551-680` and `shared/delivery-state.ts`; `docs/delivery-contract.md:60-71`.
1. **Draw.** Fresh unread ids are marked drawn into *this* call's response (`delivery.draw`, line 672). Drawn ids are blocked from re-dealing.
2. **Promote.** Once the response is fully built and the request was not aborted, the draw becomes confirm-eligible (line 544). An aborted request rolls back (line 539).
3. **Confirm.** A **later** tool call that arrived after that promote proves the model got the earlier response (an "arrival-causality barrier", `delivery.newArrival()` at line 496). The call then prunes the ids from the on-disk inbox and only *then* enqueues broker acks (lines 569-583).
4. A **flush** sends acks with typed outcomes `acked | expired | unknown | wrong_session` (lines 585-614).
5. Parallel tool calls are serialised by a per-process mutex (`withPiggybackLock`, line 477), so two concurrent calls cannot both present the same message.

There is no model-facing ACK. The header tells the model *not* to acknowledge.

**Sender-filtered wait:** `wait_for_peer_messages(from=…)` ends only on mail from that sender. Other mail still surfaces in the block and is not swallowed (`codex-server.ts:325-341`; `docs/known-issues-2026-08-08.md:7-13` shows the bug that motivated it). VERIFIED.

### 2.2 v2 mode (patched host only)

- **Same surface and the same `formatInboxBlock`** (`shared/hermes-conversation-adapter.ts:8,208`, VERIFIED).
- **Identity is per conversation.** The key is `[hermes/home, hermes/conversation_id]` from request `_meta` (`docs/hermes-conversation-bindings.md:53-79`; `shared/hermes-conversation-context.ts:34-58`). There is one `DeliveryState`, lock and inbox slot per conversation key (`shared/hermes-conversation-adapter.ts:88-96`).
- **No server `instructions` at all.** v2 has only one-line tool descriptions, the tool name with underscores turned into spaces plus "for this conversation's private peer identity." (e.g. "check messages for this conversation's private peer identity.", `shared/hermes-conversation-mcp.ts:8-22`). INFERRED: this is a deliberate token cut, or the authors noticed that Hermes ignores instructions.
- A sender-filtered wait leaves non-matching mail **undrawn**, so it does not appear in the wait's response (`shared/hermes-conversation-adapter.ts:195-197`). This differs from legacy, where it is shown. VERIFIED.

---

## 3. Wake mechanism

### 3.1 Legacy: none

- The server says so itself: "Do not assume this Hermes session will surface MCP messages while the agent is idle" (`codex-server.ts:177`). The contract says "Bounded polling, never push" (`docs/delivery-contract.md:75`). VERIFIED.
- No wake registry entry is made for Hermes. `registerWakeableSessionIfEnabled` runs only `if (RUNTIME_IS_CODEX)` (`codex-server.ts:1001-1015`), so `isWakeableSession` stays false. VERIFIED.
- **In-turn stand-by:** `wait_for_peer_messages` blocks the current turn. It polls every 500 ms for up to 60 s, which is both the default and the maximum for Hermes: "Hermes surfaces are per-turn processes … the Codex-grade 5-minute wait is always wrong there … 60s is the ceiling." (`codex-server.ts:118-124`). Out-of-range timeouts are clamped (`shared/wait-for-peer-messages.ts:43-45`). The wait only extends a turn that is already running; an idle Hermes stays idle. VERIFIED.
- Idle/busy detection, draft safety, latency: not applicable. Delivery latency is "until the model next calls an agent-peers tool", which for an idle Hermes is **until a human types something** (and then only if the model obeys the tool description). INFERRED.

### 3.2 v2 / T2: host-RPC admission (patched host only)

Code: `shared/hermes-conversation-wake.ts`, `shared/hermes-gui-lifecycle-bridge.ts`, `shared/hermes-runtime-attachment.ts`, `shared/hermes-conversation-composition.ts`, `shared/hermes-conversation-runtime.ts`. All VERIFIED as source. None of it is proven live (section 1).

**Transport:**
- The host gives the MCP child a private "attachment" descriptor file, named by `AGENT_PEERS_HERMES_ATTACHMENT` (`shared/hermes-conversation-runtime.ts:104,172-175`).
- The file is strictly validated: mode 0600, nlink 1, same uid, parent directory 0700, no symlinks, at most 16 KiB, and a loopback-only `ws://127.x|[::1]:port/api/ws?hermes_attachment=<uuid>` endpoint whose literal spelling is checked before URL normalisation (`shared/hermes-runtime-attachment.ts:12-48`).
- The child opens a WebSocket JSON-RPC connection, then checks that `gateway.ping` returns the same `home` and `backend_id` (lines 57-89).
- The child refuses any method other than `session.lifecycle_snapshot`, `session.wake_admit` and `session.wake_reconcile`, and refuses a caller-supplied `profile` (lines 137-153).

**Idle/busy detection:**
- Wake eligibility requires a host snapshot that meets all of these (`shared/hermes-conversation-wake.ts:239-256`):
  - at most 10 s old and not in the future;
  - `busy === false`, `queued === false`, `compacting === false`;
  - `state` either `live` with no end reason, or `terminal` because of `automatic_reap` (`idle_timeout | lru_evict | ws_orphan_reap`, bridge lines 21 and 108) and not a cron session.
- Then `current()` re-checks binding generation, lease, session segment and that the exact unread-id set is unchanged (`shared/hermes-conversation-composition.ts:132-141`).
- The host decides idleness. There is no screen scraping. VERIFIED.

**Admission request:** `shared/hermes-conversation-wake.ts:212-219`.

```ts
attempt_id: randomUUID(), attempt_sequence, peer_id, context, binding_generation,
expected_lifecycle_generation: lifecycle.lifecycle_generation,
resume: lifecycle.state === "terminal",
queued: true, hidden: true,
notice: "You have pending Agent Peers mail for this conversation. Call check_messages once. "
  + "Only that tool response is authoritative; do not infer message content from this notice.",
```

- The wake is **bodyless**. It asks the host for a **hidden, queued** turn, and it can **resume** a session the host reaped. The text is about 170 bytes.
- `expected_lifecycle_generation` fences the request against a lifecycle change between snapshot and admit.

**Exactly-once-ish bookkeeping:**
- The coordinator writes an `uncertain` attempt row to SQLite **before** the RPC ("Persist uncertainty BEFORE the RPC, including before a crash or lost reply", line 234).
- Receipts follow `absent | unknown | deferred | rejected | accepted | started | completed | cancelled` and never regress (lines 161-176).
- After a restart a pending attempt is reconciled with `session.wake_reconcile` before any new attempt. Only the *same* attempt may be replayed (lines 182-197).
- Re-wakes for an unchanged unread set back off 60 s, then 5 min, then 30 min, and then stop (`wake_exhausted`, lines 75 and 199-206). New mail resets the backoff.

**Draft safety:** the request carries `queued: true, hidden: true` and never types keystrokes. Whether the host leaves a composer draft alone is the host's business and unknown. INFERRED safe by construction.

**Latency:** each pass has a 5 s deadline (lines 109-110). The composition `tick()` is driven by the runtime's shared scheduler (`shared/hermes-conversation-runtime.ts:160-171`). T1 docs call it a "shared one-second scheduler" (`docs/hermes-conversation-t1-activation.md:16-18`). INFERRED: about 1-6 s from arrival to admission.

**Failure modes (VERIFIED from code unless marked):**
- An unavailable snapshot or RPC error results in `deferred` or `wake_unavailable`. It is never treated as absence (bridge lines 158-161).
- `inventory_complete` must be true. Paginated snapshots of 128 ids must agree on the live signature or the pass abstains (bridge lines 141-157).
- One adapter process per host backend UUID. Duplicates serve zero tools (`shared/hermes-conversation-runtime.ts:111-136`).
- If the binding owner changes during admission, the RPC is refused (`composition.ts:66-70`).
- If a fresh unread id arrives while the wake is `completed` but the model never called `check_messages`, the coordinator re-wakes with backoff. After three retries the conversation stays silent until new mail arrives. INFERRED from lines 199-206.

---

## 4. Setup and installation footprint on Hermes

**Legacy** (`README.md:165-177`, `docs/delivery-contract.md:86-89`, VERIFIED):
```bash
hermes mcp add agent-peers --command bun --env AGENT_PEERS_ENABLED=1 \
  --args "$HOME/agent-peers-mcp/hermes-server.ts"
hermes mcp test agent-peers
```
- This writes one `mcp_servers.agent-peers` entry into the profile's Hermes `config.yaml` (INFERRED from `hermes mcp add`). Per profile, the contract adds `PEER_NAME`, `AGENT_PEERS_CWD` and `AGENT_PEERS_ENABLED: '1'` to the server's `env`. A live session joins with `/reload-mcp`.
- `AGENT_PEERS_CWD` exists because "without it a Hermes surface inherits whatever cwd its host process happened to have (observed: `/`, the profile dir, and a workdir — three 'identities' for one agent)" (`codex-server.ts:929-933`).
- No hooks, plugins or system-prompt edits are made on the Hermes side.
- Outside Hermes:
  - the broker daemon (launchd `com.mike.agent-peers-broker`, macOS-centric);
  - `~/.agent-peers.db` and `~/.agent-peers-secret`;
  - `~/.agent-peers-hermes/` holding the inboxes, `name-claims/` locks, and the optional `disabled` flag file.
- Kill switches (`hermes-server.ts:5-16,24-38`):
  - the flag file `~/.agent-peers-hermes/disabled` makes every surface inert;
  - `AGENT_PEERS_HERMES_ROLE=passive` makes one surface serve zero tools.

**v2:**
- Everything above.
- The env interpolation `AGENT_PEERS_HERMES_V2: "${AGENT_PEERS_HERMES_V2}"` in the profile's MCP `env`, set to `1` only by the parent `serve` process (`docs/hermes-conversation-t1-activation.md:39-52`).
- A **patched Hermes** that injects `AGENT_PEERS_HERMES_BACKEND_ID`, `AGENT_PEERS_HERMES_HOME` and (T2) `AGENT_PEERS_HERMES_ATTACHMENT` into the stdio child, sends `hermes/*` `_meta` on each `tools/call`, and serves the three lifecycle/wake RPCs.
- New SQLite tables in the broker DB, among them `hermes_conversations` and `hermes_conversation_segments` (`shared/hermes-conversation-bindings.ts:49,70`), `hermes_conversation_tokens` and `hermes_conversation_inboxes` (`shared/hermes-conversation-broker.ts:23,28`), `hermes_adapter_processes` (`shared/hermes-conversation-runtime.ts:111`), `hermes_lifecycle_sequences` (`shared/hermes-conversation-lifecycle-port.ts:12`), `hermes_wake_attempts` (`shared/hermes-conversation-wake.ts:79`), and orphan-disposal tables (`shared/hermes-conversation-orphans.ts:18,24`).

---

## 5. Token cost

These are estimates at about 4 chars per token. INFERRED unless marked.

| Item | Legacy | v2 |
|---|---|---|
| Static, every request (tool schemas in the toolset) | 6 tools. `list_peers` carries a long selection reminder (`shared/peer-list.ts:6-8`). The tool literal source is about 2.65 KB (`codex-server.ts:227-299`) plus about 0.9 KB of `list_peers` description, so roughly **400-700 tokens** per LLM call, whether or not mail exists | 6 one-line descriptions, roughly **150-250 tokens** |
| MCP server `instructions` (about 8 KB: 5.7 KB `COLLEAGUE_PROTOCOL` from `shared/colleague-prompt.ts:13` plus about 2.6 KB delivery rules) | probably **0**: not surfaced by Hermes (section 2.1, INFERRED) | none |
| Quiet turn | If the model obeys the "every turn" description, one `check_messages` call per user turn. That is a tool-call round trip (call about 20 tokens, plus result `Checked inbox.` about 5 tokens) and **an extra model step** (one more inference over the whole context). If the model ignores it, 0 | the same |
| Per non-empty delivery | about **120 tokens** of fixed header (480 B, measured) per block | the same |
| Per message | about **50-80 tokens** of framing (from, cwd, summary, sent_at, id, reply_action) **plus the full body, untruncated** | the same |
| Idle wake | n/a | bodyless notice of about 170 B (about 40 tokens) as a hidden queued turn, **plus** a mandatory `check_messages` round trip to fetch the content, so every wake costs at least 2 model steps |

The 60 s wait can also be costly: a model told to "stand by" may loop `wait_for_peer_messages`. Each empty return is a full model step for about 15 tokens of output ("No peer messages arrived within 60000ms.", `codex-server.ts:755`). INFERRED.

---

## 6. Limitations and bugs

1. **No idle wake on stock Hermes.** A message to an idle Hermes waits for a human prompt *and* for the model to choose to call an agent-peers tool (VERIFIED: section 3.1). This is the main gap.
2. **The "call check_messages every turn" contract probably never reaches the model.** It lives in MCP `instructions`, and I found no upstream Hermes code that surfaces them (INFERRED, section 2.1). Only the tool description carries the habit.
3. **The preview `notifications/message` is useless on Hermes.** Upstream logs it to `agent.log` (primer section 8). It still fires on every poll that finds new mail (`codex-server.ts:398-433`).
4. **One identity per profile in legacy mode on Hermes 0.22.1 and later** (INFERRED, medium confidence):
   - Since 0.22.1 every CLI, TUI and Desktop session of a profile runs in one gateway (primer section 1). Config-defined MCP connections are keyed `(owner_scope, name)` (`hermes-agent:tools/mcp_tool_scope.py:1-31`); session-owned MCP exists only for ACP editor servers (`hermes-agent:gateway/session_local_mcp.py:1-12`).
   - So all panes running the same profile probably share **one** `agent-peers` stdio child, which means one peer UUID and one inbox.
   - Whichever session's model calls an agent-peers tool first gets every message, including mail meant for a different pane's work.
   - The name-claim election (`shared/hermes-claims.ts`) does not help: it arbitrates gateway against `serve` *processes*, not conversations.
   - v2's `_meta` keying fixes exactly this, but only on the patched host.
5. **Duplicate presentation across MCP restarts.**
   - `DeliveryState` (drawn/promoted/confirmed) is in memory only (`shared/delivery-state.ts:36`). The on-disk inbox removes ids only at confirm.
   - When the MCP child is recycled or respawned after the last delivering call, the next process re-presents mail the model already saw. Hermes can do this through `idle_timeout_seconds`, `/reload-mcp`, gateway restart, or per-turn surfaces as the authors observed (`broker.ts:34-46`).
   - This is at-least-once by design, but on Hermes "last agent-peers call of a turn" is common, so duplicates are expected. INFERRED.
6. **Unbounded batch size.** No `LIMIT` on the poll and no body truncation. A 500-message backlog is one giant tool result (`broker.ts:901-910`; section 2.1).
7. **The verbose header repeats on every delivery,** about 120 tokens (`shared/piggyback.ts:40-47`). Static guidance is re-sent per message, which is the opposite of our rule (a).
8. **Mislabelled text in Hermes instructions.** Only the first line is conditional on runtime (`codex-server.ts:177`). The rest still says "This is a hard constraint of the Codex runtime" (line 181) and "keeps this same Codex turn alive" (line 201). It is cosmetic, and probably moot given item 2.
9. **cwd and identity depend on static config.** `AGENT_PEERS_CWD` and `PEER_NAME` are per profile, not per session or pane (`docs/delivery-contract.md:86-87`). Under the shared gateway, `process.cwd()` is the gateway's own directory (primer section 1). Peer discovery by cwd/repo is therefore wrong for any Hermes session not in the configured directory. INFERRED.
10. **v2 depends on one private host fork.** It is pinned to commit `65d7a5b06a`, which does not exist upstream (section 1), and it carries maintainer codenames and local patch paths. Its strict `_meta` parser fails closed on stock Hermes, as it should, but this means the "Hermes wake" in the overview is not portable today.
11. **Latent edge.** A non-numeric `timeout_ms` throws inside `beforeReadQueue`. The throw is caught and logged, and the tool then returns "wait_for_peer_messages did not run." with `isError` (`codex-server.ts:500-504,749-750`). This is harmless but opaque to the model.

---

## 7. Clever ideas worth stealing

These are INFERRED value judgements, with pointers to the relevant herdr-threads gaps from primer section 12.

1. **Take identity from host-supplied request metadata, not process env** (v2, `docs/hermes-conversation-bindings.md:53-79`). This is the same problem we face on Hermes 0.22.1 and later: our hook child inherits the *gateway's* env, so `HERDR_PANE_ID` is missing or wrong (primer section 12.1).
   - Their answer: never trust env or tool arguments for identity; read a host-stamped per-call context; refuse when it is missing.
   - For us, a per-call session id in plugin hooks could play that role (INFERRED; the primer offers `HERMES_SESSION_ID`, section 12.1, but this report did not verify the hook kwarg). We could key Hermes bindings on `(HERMES_HOME, session_id)` and attach the pane as a separate, verified witness.
2. **Admit the wake through the host, never by keystrokes, and gate on a fresh lifecycle snapshot** (`busy/queued/compacting === false`, at most 10 s old, `expected_lifecycle_generation` fence; `shared/hermes-conversation-wake.ts:239-256`).
   - Upstream Hermes has the public primitives for this: `ctx.inject_message(..., session_key=…)` from our plugin, or gateway `prompt.submit` (primer sections 4 and 5.1).
   - Their bookkeeping is a good template for our wake ledger: persist `uncertain` before the call, reconcile by `attempt_id` after a crash, keep receipts monotonic, back off per unread-set signature.
   - It would also avoid our default `busy_input_mode: interrupt` hazard (primer section 12.4).
3. **Ask for a hidden, queued, bodyless wake turn** (`queued: true, hidden: true`, plus a notice that says "do not infer content from this notice"). This matches our R18. The `hidden` flag is worth looking for in Hermes' `inject_message`/`prompt.submit` so wake rows do not clutter the human's transcript.
4. **Confirm-on-next-call ack with an arrival-causality barrier** (`codex-server.ts:479-548,551-583`). This gives a precise "the model really received it" signal for pull-only transports, without a model-visible ACK. It applies directly if we ever deliver through tool results (primer section 12.3, `transform_tool_result`): treat the next hook or tool call after the transformed result as proof of delivery.
5. **Sender-filtered waits that never consume unrelated mail** (`codex-server.ts:325-341`). A good rule for any `herdr-threads wait`-style long poll.
6. **Cheap operator kill switches that need no env access** (the flag file `~/.agent-peers-hermes/disabled`, and `ROLE=passive` serving zero tools; `hermes-server.ts:5-38`). They are useful exactly because a Hermes-managed child's env is hard to control.
7. **Do not copy:** the per-delivery behavioural header, the unbounded batch, the dead `notifications/message` push, or reliance on MCP `instructions` for a load-bearing habit on a harness that may not surface them.

---

## Verification

Adversarial pass on 2026-10-23 against the clone at `2c63379d65ad44845815faa21a0c5c78355002fa`, the `hermes-agent` clone at `f3b5b980d6158794beb3d1044fd7b8ca4b8f0b23`, the primer, and the GitHub API.

Checked and confirmed: repo metadata (fork of `Co-Messi/agent-peers-mcp`, 0 stars, 0 forks, pushed 2026-10-22); `hermes-server.ts` entry/kill switches; legacy contract and v2 T1/T2 doc quotes; the `65d7a5b06a` pin and the 422 "No commit found" from upstream; no `wake_admit`/`lifecycle_snapshot`/`hermes_attachment`/`hermes/conversation_id` in the upstream clone (only the `test_native_completion_wake_admits_…` test name); upstream `call_tool` passes no `_meta` identity; `initialize_result` used for capabilities only, and the other `instructions` hits in upstream (`tools/connectors/*`, `agent/coding_context.py`) are catalog or operator text, not MCP server instructions; logging notifications go to `agent.log`; 2,000,000-char result cap; the six tools, `withPiggyback` steps, confirm/flush/draw line numbers, poll cadence, 30 s lease (`broker.ts:27`), uncapped poll query, 500-message cap; the wake registry is Codex-only, and `desktopWakeEligible` (`shared/codex-desktop-wake.ts:13-15`) also excludes Hermes; 60 s wait ceiling and clamp; non-finite `timeout_ms` throw path; v2 strict `_meta` parser, per-conversation slots, undrawn non-matching wait mail; attachment validation, method allowlist, `gateway.ping` scope check; wake eligibility, admission request (notice is 171 bytes), receipts, backoff `[60 s, 5 min, 30 min]`, 5 s deadline; bridge reap set and 128-id pagination; composition owner checks.

Changed: lock, arrival-barrier and rollback line numbers (477, 496, 539); header is 7 lines / about 483 bytes, not 8 lines; tool-schema size and range (about 2.65 KB at 227-299 plus about 0.9 KB of `list_peers` description); MCP `instructions` size about 8 KB, not 6 KB; the v2 tool description quote; the SQLite table list and its citations (the tables are spread over several modules, not runtime lines 110-114); the plugin-hook `session_id` kwarg in section 7 is now marked INFERRED.
