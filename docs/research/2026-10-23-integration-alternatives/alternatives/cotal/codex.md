# Cotal x OpenAI Codex CLI: message injection and wake

- Upstream: https://github.com/Cotal-AI/Cotal (site https://cotal.ai/). Clone `alts/cotal`, `git clone --depth 1`, HEAD `6426fb538e0b` (2026-10-23 22:04 +0200). The clone was only read; nothing in it was executed or installed.
- Citations are `path:line` relative to the clone root unless prefixed `codex:`, which means the Codex source clone used by the primer (`rust-v0.163.0`, `codex-rs/...`). **V** = VERIFIED (read in source or docs), **I** = INFERRED (reasoned, not observed running).
- Read with: `../../primers/codex.md` (harness primer) and `overview.md` (Cotal overview).
- Main files: `extensions/connector-codex/src/{host.ts,app-server.ts,mcp.ts,tui.ts,extension.ts}`, the shared core in `extensions/connector-core/src/{agent.ts,control.ts,framing.ts,orientation.ts,tool-specs.ts}`, and `docs/connect-codex.md`.

## 1. Support status

| Item | Finding | |
|---|---|---|
| Supported? | Yes, as a dedicated **native connector**, `@cotal-ai/connector-codex` v0.100.2 (`extensions/connector-codex/package.json:4`). It is seeded with the CLI and needs no Codex-side plugin (`docs/connect-codex.md:16-25`). | V |
| Maturity | **Beta** (`docs/connectors.md:23`; `docs/connect-codex.md:1,10-12`). | V |
| Binding model | A **host process** (Node) embeds the mesh endpoint and drives a `codex app-server` child it spawns itself, over WebSocket JSON-RPC (app-server v2, `experimentalApi: true`) (`app-server.ts:1-35`, `:426-430`; `docs/connect-codex.md:68-75`). | V |
| Scope | **Managed spawns only**: `cotal spawn --agent codex` or a manifest's `agent: codex` (`docs/connect-codex.md:35-46`). A missing `COTAL_*` identity makes the host throw: "this binary is never run standalone by an operator" (`host.ts:32-33`, `:342`). There is no path that attaches to a Codex session the user started themselves. | V |
| Unsupported | `--resume` throws, because a thread brought up by `thread/resume` lacked the `cotal_*` MCP tools on 0.145.0 (`extension.ts:147-156`; `docs/connect-codex.md:297-298`). Exact-session continuation throws (`extension.ts:146`). Tool-sharing (`connectors.codex.mcpServers`) throws (`extension.ts:160-161`). Containers are not supported (`docs/connectors.md:34`). | V |
| Platform | POSIX only, because `auth.json` is symlinked into the isolated home (`docs/connect-codex.md:105-108`; `host.ts:192-209`). | V |
| Codex versions | Minimum 0.145.0, tested on 0.145.0 and 0.146.0 (`docs/connect-codex.md:27-33`; `app-server.ts:95`). The primer's installed 0.163.0 was not tested by Cotal. The flags used (`app-server --listen ws://… --ws-auth capability-token --ws-token-file`, `resume --remote … --remote-auth-token-env`) were not checked against 0.163.0 help here. | V / unknown |
| Non-managed Codex | `cotal setup` only drops Agent Skills into `~/.agents/skills/` for Codex and other harnesses. That gives no push delivery and no wake (`docs/connect-claude.md:55-73`; `implementations/cli/src/commands/setup.ts:602`). | V |

## 2. Injection mechanism

### 2.1 The three channels into the model's context

| # | Channel | Codex API | Role in history | When | Code |
|---|---|---|---|---|---|
| 1 | Static briefing | `thread/start.developerInstructions` | developer | once per thread | `app-server.ts:434-437`; text built in `host.ts:146-161` | V |
| 2 | Idle delivery (wake) | `turn/start {threadId, input:[{type:"text", text}]}` | **user** turn | thread idle and a wake-worthy item arrives | `app-server.ts:497-503`; caller `host.ts:607-660` | V |
| 3 | Mid-turn delivery | `turn/steer {threadId, input:[…], expectedTurnId}` | user input appended to the **active** turn | a *directed* item arrives while the host's own turn runs | `app-server.ts:509-523`; caller `host.ts:666-693` | V |
| (4) | Primer | `thread/inject_items` with one user `message` item `[cotal] This session is a Cotal mesh peer.` | user history item, no model call | once at thread start, only to make a rollout exist so the TUI can `resume` | `app-server.ts:104`, `:442-454` | V |
| (5) | Pull tool | MCP `cotal_inbox` | tool result | only when the model calls it, and on Codex it returns **only pull-only (quiet) ambient** | `mcp.ts:99-118` | V |

The connector uses **no Codex hooks** at all: no `hooks.json`, no `UserPromptSubmit`/`PreToolUse`/`Stop`. The child runs on an isolated `CODEX_HOME`, so the operator's hooks never load (`host.ts:27-30`; `app-server.ts:158`, `:323`) **V**. Every push goes through app-server turn APIs.

### 2.2 Code path for one inbound message

1. The NATS/JetStream delivery reaches `MeshAgent.ingest` (`extensions/connector-core/src/agent.ts:1008`). It is deduplicated by message id against handled ids and against items still pending. A durable duplicate replaces the ack handle and re-emits `incoming` (`agent.ts:1018-1048`). The item is buffered in a 200-entry local inbox (`agent.ts:227`, `:1133-1180`) **V**.
2. The host's `incoming` handler (`host.ts:1074-1083`) runs:
   ```ts
   const directed = item.kind !== "channel" || item.mentionsMe;
   if (driver.busy || awaitingTurnEnd) { if (directed) void steerPending(); return; }
   if (directed || agent.attention === "open") void drive();
   ```
   **V**
3. `drive()` (`host.ts:607-660`) runs when the host is ready, not shutting down, not already driving, `!driver.busy` and `!awaitingTurnEnd`. It peeks (does **not** drain) every `automatic` item (`agent.peekInbox("automatic")`, `agent.ts:1374-1376`) and formats them with `formatInjection`. It prepends the channel briefing once per thread and appends any pending workflow "run turn" text (`host.ts:632-641`). It records the exact receive keys in `surfaced`, arms `awaitingTurnEnd` **before** the await, then calls `driver.startTurn(parts.join("\n\n"))` **V**.
4. `steerPending()` (`host.ts:666-693`) runs only while the host's **own** turn is open (`driver.busy && awaitingTurnEnd`). It selects automatic items that are not already surfaced and are directed (`i.kind !== "channel" || i.mentionsMe`), then calls `driver.steer(inj)`. The steer targets `currentTurnId`, which prefers an owned turn (`app-server.ts:282-286`), and carries `expectedTurnId`, so a turn that just ended rejects the steer instead of binding it to a newer turn (`app-server.ts:505-523`). Ids are added to `surfaced` only if the steer was accepted **and** the turn is still open, otherwise they stay buffered for the next boundary (`host.ts:684-688`) **V**.
5. The model replies with the MCP tools `cotal_dm`, `cotal_send` and the rest. They are served by the host on a loopback streamable-HTTP MCP endpoint, which the app-server reaches as an MCP client (`mcp.ts:1-28`, `:82-241`) **V**.

### 2.3 Payload format and framing

`formatInjection` (`extensions/connector-core/src/control.ts:120-127`):

```ts
const head = `📨 Cotal — ${items.length} new message${items.length === 1 ? "" : "s"} from peers:`;
const tail = `(If this asks for work, do it with your own tools first and verify the result, then reply with ` +
  `cotal_dm / cotal_send — never report an action you did not actually perform. cotal_roster shows who's here.)`;
return `${head}\n${items.map(options.metadata ? fmtDeliveryItem : fmtItem).join("\n")}\n${tail}`;
```

The Codex host calls it without `metadata`, so it uses `fmtItem` (`host.ts:628`, `:677`) **V**. Per item (`framing.ts:124-132`):

- `[DM from <label>] <body>`, `[@<service> from <label>] <body>`, `[#<channel>[ @you] <label>] <body>`.
- `(history) ` prefix for backfill. `(redelivered, delivery N: an earlier delivery may already have run) ` when the broker's delivery count is above 1 (`framing.ts:125-127`).
- Neutralization rule: "A LINE THAT BEGINS AT COLUMN ZERO IS WRITTEN BY THE CONNECTOR, NEVER BY A PEER" (`framing.ts:9-12`). Body line breaks, including U+2028, U+2029, U+0085, VT and FF, become `\n  ` (`framing.ts:64`, `:96-98`). Brackets and line breaks are stripped from names, channel and service labels (`framing.ts:80-82`).

The focus-mode mention hint is a separate, content-free frame: `📨 You were mentioned by <label> on #<channel> — read it with cotal_inbox.` (`host.ts:1089`) **V**.

A sample wake turn for one DM (I, assembled from the code above):

```
📨 Cotal — 1 new message from peers:
[DM from bob (reviewer)] please re-run the tests on main
(If this asks for work, do it with your own tools first and verify the result, then reply with cotal_dm / cotal_send — never report an action you did not actually perform. cotal_roster shows who's here.)
```

The text goes into the user role, through `turn/start` and `turn/steer` `input:[{type:"text"}]` (`app-server.ts:499-502`, `:513-517`) **V**. Codex has no separate "peer" role, so a peer message is a user message, and the trust stance lives in `developerInstructions` and the tail.

### 2.4 Size bounds

- **No connector-side bound on the batch.** `drive()` takes *all* automatic items (`host.ts:624-629`), and `steerPending` takes all directed ones (`host.ts:673-677`). The only cap is the 200-item local inbox (`agent.ts:227`) **V**. No per-message truncation was found on the Codex path. The Claude native path has 8 items, 1,024 code points and a pointer fallback (overview); none of that is used here **V** (absence in `host.ts`).
- **Codex's own bound.** app-server rejects `turn/start` (`turn_start_inner`) and `turn/steer` (`turn_steer_inner`, via `validate_v2_input_limit`) input whose text exceeds `MAX_USER_INPUT_TEXT_CHARS` with "Input exceeds the maximum length of … characters." (`codex:codex-rs/app-server/src/request_processors/turn_processor.rs:505-523,553-575,1044-1068`) **V**. The constant's value is in `codex-protocol`, which is not in the sparse clone: unknown. How Cotal handles that rejection is covered in §6.
- Per-message size is otherwise bounded only by the NATS broker's `max_payload`, which Cotal leaves at the server default unless configured (`packages/core/src/provision.ts:3165-3171`). The stock NATS default is 1 MB **I** (not read in Cotal source).
- Hook spill limits (2,500 tokens, primer §1.4) do not apply, because no hooks are used **V**.

### 2.5 De-duplication and acknowledgement

- **Dedup at ingest** by message id: a handled id is acked and dropped, and a pending duplicate keeps one entry with the freshest durable ack handle (`agent.ts:1018-1048`). Id-less messages are never coalesced (`agent.ts:1009-1017`) **V**.
- **No double steering within a turn**: `steerPending` filters out `surfaced` receive keys (`host.ts:671-675`) **V**.
- **Ack at the turn boundary, by exact id** (`host.ts:711-756`): when an *owned* turn ends, `drainInboxDeliveries(ids)` runs if the status is `completed`, or `interrupted` and the host is not shutting down. That is the operator's Escape, which **dismisses** the batch (`host.ts:741-742`; `docs/connect-codex.md:89-91`). `failed` leaves the batch un-acked and retries with backoff from 1 s to 30 s (`host.ts:90-91`, `:590-603`, `:743`). `unknown` leaves it un-acked (`host.ts:695-707`). A turn the TUI user started (`owned=false`) **never** acks (`host.ts:712-730`) **V**.
- **Turn ownership** is claimed from the `turn/start` *response* while it is decoded, in wire order, not in the awaited continuation. A terminal that arrives while a `turn/start` is outstanding is buffered until ownership settles (`app-server.ts:190-216`, `:704-727`, `:767-789`, `:861-864`). Late `turn/started` events after their terminal are tombstoned (`app-server.ts:796-805`) **V**. This is careful race handling.
- **Ack-wait renewal: not applied on Codex.** The core has `holdInFlight` and `keepOwned`, which renew a held delivery's JetStream ack wait at half the wait (`agent.ts:1392-1406`, `:1435-1449`). OpenCode and the Claude inline path call it (`extensions/connector-opencode/src/plugin.ts:1134`; `extensions/connector-core/src/inline-delivery.ts:57`), but **the Codex host never calls `holdInFlight`** (grep over `extensions/connector-codex/`) **V**. The overview says Codex "renew[s] the ack wait with JetStream in-progress acks while a long turn runs". For Codex that claim is not supported by the code, and `docs/connect-codex.md:89-98` does not say it either **V**. For what this means, see §6.
- **App-server crash**: `surfaced` is cleared without an ack, and the batch re-drives into the replacement thread (`host.ts:1010-1016`, `:1063-1066`) **V**.

## 3. Wake mechanism

### 3.1 Mechanism

A real **`turn/start`** on the thread the host owns (`app-server.ts:491-503`). No keystrokes are typed, nothing goes into the composer, and no hooks run. This is the primer's "App-server `turn/start`" row, used by the client that holds the thread loaded (primer §0, §2.2) **V**. Mid-turn, directed items use **`turn/steer`** instead, which never wakes and fails when no turn is active (primer §2.2) **V**.

Wake triggers in the host:

| Trigger | Action | Code |
|---|---|---|
| `incoming`, idle, directed item, or ambient under `open` attention | `drive()` | `host.ts:1074-1083` | V |
| `mention-wake` (focus mode; the body was ack-dropped at ingest) | latch the hint; steer it if busy, else `drive(hint)`; the latch is consumed only on accept | `host.ts:1084-1103`, `:647-649` | V |
| `wake` (core `requestWake`, Stop→idle batch flush) | `drive()` if not busy | `host.ts:1104-1106`; `agent.ts:1728-1734` | V |
| Own turn completes | ack, then pump `drive()` if `pendingWake() > 0` (not after `failed`, which waits for backoff) | `host.ts:745-754` | V |
| TUI user's turn completes | pump `drive()` for what buffered meanwhile | `host.ts:716-729` | V |
| App-server restarted | unconditional `drive()` | `host.ts:1063-1066` | V |
| Launch | `COTAL_CODEX_PROMPT` first turn, else drive buffered | `host.ts:1317-1320` | V |

Wake policy, the "two tiers": directed traffic (DM, anycast, @mention) always wakes. Ambient channel traffic wakes only under attention `open`; `dnd` and `focus` hold it, and quiet channels are pull-only (`agent.ts:1701-1726`; `host.ts:22-24`) **V**. There is no numeric priority.

### 3.2 Idle/busy detection

- `driver.busy` is `liveTurns.size > 0`. `liveTurns` is maintained from the app-server's broadcast `turn/started` and `turn/completed` notifications. These cover the host's own turns **and** turns started by the attached TUI (`app-server.ts:190-195`, `:275-277`, `:791-876`) **V**. It is the primer's "precise native signal", read by the client that owns the server (primer §2.3) **V**.
- `awaitingTurnEnd` additionally marks that the host has a batch open (`host.ts:574`, `:645`) **V**.
- The host does **not** use `thread/status/changed`. Its notification switch ignores everything but turn, item and MCP-startup events (`app-server.ts:877-878`) **V**. Approval or `waitingOnUserInput` states therefore do not feed `busy`. Approvals are auto-declined anyway (§3.4) **V**.

### 3.3 Draft safety

- The wake never touches the composer. `turn/start` is server-side, and the attached TUI is just another app-server client (`tui.ts:1-11`) **V**. No `draft` or `composer` handling exists anywhere in the connector (grep) **V**, and none is needed for the wake path. **I**: a half-typed draft in the attached TUI should survive a mesh-driven turn starting beside it, because the composer is client-local state. That was not observed.
- While a **human-started** turn is running, `drive()` is blocked by `driver.busy` and `steerPending` is blocked by `!awaitingTurnEnd` (`host.ts:612`, `:667`). Peer DMs therefore wait for the human's turn to finish and are delivered by the foreign-turn pump (`host.ts:716-729`) **V**. One exception: the focus `mention-wake` hint *is* steered into whatever turn is live, the human's included (`host.ts:1091-1095` → `app-server.ts:282-286`) **V**.

### 3.4 Unattended-safety measures that make the wake work

- `approval_policy="never"` is enforced, and an interactive policy is refused at launch (`host.ts:233-243`). Approval server requests that arrive anyway are **declined**, or answered with `-32601`, so a turn never hangs (`app-server.ts:734-753`) **V**.
- The host's own MCP server gets `default_tools_approval_mode="approve"`, so no per-call elicitation can hang a turn nobody watches (`host.ts:288-292`) **V**.
- `check_for_update_on_startup=false` on the TUI, so a detached agent never sits on a key-press gate (`tui.ts:42-45`) **V**.
- MCP readiness gate: the host waits for `mcpServer/startupStatus/updated` = `ready` for `cotal` (30 s) before announcing presence. Otherwise it is fatal, "refusing to join the mesh without the cotal_* tools" (`app-server.ts:84-86`, `:525-579`, `:816-824`; `host.ts:1299-1302`) **V**.
- Auth probe via `account/read` before going online (`host.ts:1283-1298`) **V**.

### 3.5 Latency

- Idle wake: push-driven end to end. A NATS delivery fires `incoming`, which calls `drive()`, which calls `turn/start` over loopback. There are no polling loops on this path **V** (code path). **I**: wake latency is milliseconds plus model time to first token.
- Busy, directed: one `turn/steer` RPC. Codex consumes steered input at its next sampling step (primer §0). If the model is inside the built-in `sleep` tool, that tool returns early on new input (primer §5) **I**.
- Busy, ambient: waits for the turn boundary. Busy behind a human-typed turn: waits for that turn to end (§3.3) **V**.

### 3.6 Failure modes and recovery

| Failure | Handling | Code |
|---|---|---|
| `turn/start` rejected or errored | `surfaced` cleared, `awaitingTurnEnd` reset, retry with exponential backoff 1 s → 30 s | `host.ts:652-656`, `:590-603` | V |
| Turn `failed` | un-acked; presence condition mapped from `codexErrorInfo`; backoff retry | `host.ts:743`, `:898`; `docs/connect-codex.md:137-149` | V |
| Steer races turn end | `expectedTurnId` rejects the steer → ids stay buffered → boundary drive; terminal handler awaits `steerSettled` first | `host.ts:679-688`, `:731-732` | V |
| Any RPC hangs | 60 s per-request timeout; 3 consecutive timeouts SIGKILL the child | `app-server.ts:75-81`, `:653-673` | V |
| WebSocket drops while child alive | SIGKILL the child → one recovery path | `app-server.ts:472-480` | V |
| App-server crash | restart in place (new listener, new token, **new thread**), re-drive the un-acked batch; more than 3 crashes in 120 s is fatal | `host.ts:987-1068`, `:92-95` | V |
| TUI exits | exit 0 means a clean retirement of the agent; non-zero means `shutdown(1)` | `host.ts:1159-1176` | V |
| Host retirement (SIGTERM, manager shutdown) | interrupts the live turn, which stays un-acked; redelivery to a later same-name spawn "is not promised" | `host.ts:1179-1243`; `docs/connect-codex.md:95-98` | V |

## 4. Setup and installation footprint on the harness

- **Nothing is written to the operator's Codex config.** No `~/.codex/config.toml`, `hooks.json`, MCP entries or trust records (`docs/connect-codex.md:99-104`; `host.ts:27-30`) **V**. Codex's own plugin list may show a `cotal` plugin from the `cotal-mesh` marketplace. That is the Claude adapter, and the docs tell users not to install it (`docs/connect-codex.md:21-25`) **V**.
- **Per agent**, it creates `<workspaceRoot>/.cotal/codex/<slug>-<sha256(space\0name)[:12]>/` as a private `CODEX_HOME` (`host.ts:170-211`; `extension.ts:201-204`) **V**. It contains:
  - `auth.json`, a **symlink** to the operator's `$CODEX_HOME/auth.json` (or `~/.codex/auth.json`), re-linked on every launch so token refreshes do not fork. Symlinked managed path components are refused (`host.ts:179-209`) **V**.
  - `remote-token`, the app-server capability token, created 0600 with `O_EXCL|O_NOFOLLOW` and removed on stop (`app-server.ts:116-144`, `:303-307`, `:645-646`) **V**.
  - `host.log`, once the TUI owns the terminal (`host.ts:1147-1153`) **V**.
  - Codex's own `sessions/…/rollout-*.jsonl` and state, which Codex writes there (`docs/connect-codex.md:178-181`) **V**.
- **All config goes on the command line** of the child as `-c` overrides (`app-server.ts:310-319`; `host.ts:216-294`) **V**:
  - `approval_policy="never"`, `sandbox_mode="workspace-write"`, `sandbox_workspace_write={network_access=true}`, optional `model` and `model_reasoning_effort`, and operator `--opt k=v`. The `mcp_servers` namespace is reserved and refused.
  - `mcp_servers.cotal.url="http://127.0.0.1:<port>/mcp"`, `mcp_servers.cotal.bearer_token_env_var="COTAL_MCP_TOKEN"` (passed by name, so it never appears on argv) and `mcp_servers.cotal.default_tools_approval_mode="approve"`.
- **Processes per agent**: the host (node), `codex app-server --listen ws://127.0.0.1:0 --ws-auth capability-token --ws-token-file …`, and optionally `codex resume --remote <ws-url> --remote-auth-token-env COTAL_CODEX_REMOTE_TOKEN -c check_for_update_on_startup=false <threadId>` (`app-server.ts:310-318`; `tui.ts:35-54`) **V**. The children get the host's environment with all `COTAL_*` variables scrubbed (`app-server.ts:320-326`; `tui.ts:48-52`) **V**.
- **Side effect of isolation** (I): the managed Codex does **not** see the user's MCP servers, user hooks, user `config.toml`, profiles or keyring credentials (keyring is documented as unsupported, `docs/connect-codex.md:104-105`). Project `AGENTS.md` should still load, because cwd is the agent's workspace. Project `.codex/` hooks probably do not run, because the trust records live in the user's real home.

## 5. Token cost

### Per session (static, cached after the first request)

- `developerInstructions` concatenates `ORIENTATION_BOOTSTRAP`, `feedbackLine`, `MESH_FIRST_STEER`, `WORKFLOW_STEER`, `CHANNEL_GUIDANCE` and a Codex-specific paragraph, plus any persona (`host.ts:146-161`; `extensions/connector-core/src/orientation.ts:20-49`; `extensions/connector-core/src/config.ts:386-398`) **V**. Roughly 2,000 characters, about 450-550 tokens, without a persona **I** (estimated from the literals).
- MCP tool catalog: up to 24 `cotal_*` tools, some gated by capability (`extensions/connector-core/src/tool-specs.ts:683-705`). Their top-level descriptions total about 13,900 characters of literal text (measured over the 24 `description:` literals in `tool-specs.ts`, interpolations excluded; on Codex `cotal_inbox` gets a shorter override, `mcp.ts:108-110`), about 3,400 tokens before parameter schemas **I**. herdr-threads puts no tool catalog into Codex.
- Primer item: about 10 tokens (`app-server.ts:104`) **V**.
- One-time channel briefing on the first driven turn, if channels have descriptions (`agent.ts:2823-2831`; `host.ts:637-641`) **V**.

### Per message

- Framing overhead per batch: the header (about 10 tokens) plus the instruction tail (about 205 characters, about 45 tokens) **I**. Per item: the bracket attribution (about 5-10 tokens) plus the body, which is **not truncated** **V**.
- Every idle delivery is a **full model turn** (`turn/start`), even for a message that needs no reply. The developer instructions even tell the model that "silent acknowledgement is correct" (`host.ts:157-159`) **V**. **I**: the real cost per idle wake is one complete sampling round over the whole cached history plus the response, far more than the injected bytes.
- A steer adds the same framing into the running turn. No extra turn is needed, but the model typically spends an extra sampling step on it **I**.

### Per quiet turn

- **Zero.** There are no hooks and no per-turn digest, and nothing is injected when the inbox is empty. `drive()` returns early when there are no items (`host.ts:625`) **V**. That matches herdr-threads' "empty hook output on quiet turns" goal, and beats it, since no hook process runs at all **I**.

## 6. Limitations and bugs

1. **Managed-only, and Cotal owns the whole session.** Cotal cannot deliver to a Codex TUI the user launched. The managed agent runs on an isolated home without the user's MCP servers, hooks or config, with forced `approval_policy=never` and a network-on sandbox. The docs name that last combination as an exfiltration risk (`docs/connect-codex.md:236-286`) **V**.
2. **Context loss on app-server crash.** A restart calls `thread/start` again. It never calls `thread/resume`, because that is "not supported" for lack of MCP tools on resume. The agent therefore continues on a **new, empty thread** and loses its conversation (`app-server.ts:434`; `host.ts:1030-1058`; `docs/connect-codex.md:183-191`) **V**. Only the un-acked batch is carried over.
3. **No resume at all** (`extension.ts:147-156`). Every spawn is a fresh thread, so long-lived agent memory depends on the mesh, not on Codex history **V**.
4. **Unbounded batch against Codex's input limit (probable wedge).** `drive()` joins every automatic item into one `turn/start` text (`host.ts:624-646`). If it exceeds `MAX_USER_INPUT_TEXT_CHARS`, app-server rejects it (`codex:…/turn_processor.rs:553-575`). The catch clears `surfaced` and calls `scheduleErrorRetry`, which re-drives the *same* oversized set (nothing drains automatic items without a turn, so it only grows until the 200-item overflow valve evicts) with backoff capped at 30 s, apparently forever (`host.ts:652-656`, `:590-603`) **I**. A single oversized DM (the broker allows about 1 MiB, I) would likewise block every later directed message, because each drive includes all pending items.
5. **`unknown` terminal status re-drives immediately.** The docs say an unknown terminal outcome is left "un-acked with no retry of its own" (`docs/connect-codex.md:91-92`). The code sees the un-acked ids still pending after an `unknown` and calls `drive()` at once without backoff: `status !== "failed" && agent.pendingWake() > 0` (`host.ts:743-753`). A server that repeatedly returns an unrecognized status would hot-loop full model turns on the same batch **I**. The docs and the code disagree.
6. **No ack-wait renewal during long turns.** The Codex host never calls `holdInFlight`, so surfaced items keep the broker's ack timer. The endpoint default is 60 s (`agent.ts:571`; `packages/core/src/endpoint-binding.ts:708`). In a turn longer than the ack wait, JetStream redelivers. For a DM durable, the duplicate goes back to the same reader and is coalesced, and its next frame shows `(redelivered, delivery 2…)` even though nothing went wrong (`agent.ts:1039-1047`; `framing.ts:127`). For **anycast** (a role queue), the redelivery can go to a *different* role holder while this agent is still working, which means duplicate work **I** (`agent.ts:1160-1163` and `:1435-1438` document exactly this hazard for un-held items). The overview's claim that Codex renews in-progress acks is not borne out by the Codex connector code.
7. **Operator interrupt dismisses peer mail.** Escape in the TUI acks the whole batch, including directed items that were steered in and not acted on (`host.ts:741-742`). It is a deliberate choice that matches OpenCode, but a human interrupting for an unrelated reason silently loses the peer's request **V**/**I**.
8. **Human turns block DMs, with one inconsistency.** A peer DM that arrives during a human-typed turn is neither steered nor started until that turn ends (`host.ts:667`, `:1078-1081`). The focus mention hint *is* steered into the human's turn (`host.ts:1091-1095`). The driver comment says steering into a TUI-owned turn "is deliberate" (`app-server.ts:279-281`), but `steerPending` never does it **V**.
9. **Peer text is user-role input.** `turn/start` and `turn/steer` take `input:[{type:"text"}]`, so the model sees peer messages as the user speaking. The Codex `additionalContext` field on `turn/start` and `turn/steer` (primer §2.2) is not used **V**. Framing neutralization and the developer instructions are the only separation **V**.
10. **Same-uid isolation is explicitly not a boundary.** Tokens live in child environments and agent homes, so a sibling agent can drive another agent's Codex or impersonate it on the mesh (`docs/connect-codex.md:287-292`; `mcp.ts:24-27`; `app-server.ts:22-27`) **V**.
11. **Experimental upstream surface.** app-server v2 with `experimentalApi`. Every protocol shape is pinned in one file, and a gated live smoke re-proves them (`app-server.ts:29-34`; `docs/connect-codex.md:300-302`) **V**. Cotal tested only 0.145 and 0.146 **V**.
12. **Quitting the TUI retires the agent.** The pty process is the agent's lifecycle (`host.ts:1162-1172`) **V**.
13. **The rollout path must be discovered** for the event plane. `thread/start` writes nothing, so the primer inject exists partly to materialize the file, polled at 250 ms up to 40 times (`host.ts:104-107`, `:797-867`) **V**. This does not affect delivery.

## 7. Clever ideas worth stealing

1. **Own the app-server, keep the real TUI** (`codex app-server --listen ws://127.0.0.1:0 --ws-auth capability-token` + `codex resume --remote <url> --remote-auth-token-env VAR <thread>`). A herdr-threads *managed launch* of Codex could do the same. The host owns wake (`turn/start`) and steer (`turn/steer`), the human still gets the genuine Codex UI in the Herdr pane, and there is no composer read, no typing, no paste-burst hazard and no draft clobbering (primer §7). That removes the `composer_stash` limitation (`src/harness/recipe.rs:28-33`) and the "non-empty draft refuses the wake" rule (AGENTS.md, `side-draft` case) for launched sessions **I**. It also sidesteps the shared-daemon environment leak, because the host's private listener is the only server (primer §2.1) **I**.
2. **Ownership-aware turn accounting.** The app-server broadcasts every client's turns. Cotal claims ownership from the `turn/start` response in wire order, buffers ambiguous terminals while a start is outstanding, tombstones late `turn/started` events, and acks only on owned turns. It pumps delivery after a *foreign* (human) turn ends (`app-server.ts:190-216`, `:704-727`, `:791-876`; `host.ts:711-756`). This is the correct model for any herdr-threads client that shares a thread with a human **V**.
3. **`turn/steer` with `expectedTurnId`** as race-safe mid-turn delivery, with a settlement barrier (`steerSettled`) so the ack decision waits for the steer verdict (`host.ts:679-688`, `:731-732`). It maps directly onto herdr-threads rule 3 (in-turn delivery for busy sessions) without relying on `PreToolUse ^Bash$` coverage (primer §11 item 4) **V**/**I**.
4. **`thread/inject_items` as a zero-cost write** (`app-server.ts:442-454`). Cotal uses it only to materialize the rollout. The same call is the primer's "silent delivery" (§2.2): a p1 item could be appended to history with no turn, so it is seen on the next turn, exactly herdr-threads' "a silent item arrives on its next turn" semantics, with no hook and no `inbox` round trip **I**.
5. **Fail fast on a mute peer.** Wait for `mcpServer/startupStatus/updated == ready` before announcing presence (`app-server.ts:525-579`). Pre-approve one's own MCP tools with `default_tools_approval_mode="approve"` so an unattended turn never blocks on an elicitation (`host.ts:288-292`). Auto-decline approval requests rather than leave them unanswered (`app-server.ts:734-753`) **V**.
6. **MCP over loopback streamable HTTP instead of app-server `dynamicTools`.** Dynamic tools route to the client that owns the turn, and the TUI refuses them ("Dynamic tool calls are not available in TUI yet"), so they would disappear on human-typed turns (`mcp.ts:4-16`; `app-server.ts:12-15`) **V**. This matters if herdr-threads ever exposes tools in a shared-thread setup.
7. **Broker-sourced redelivery marker in the frame**: `(redelivered, delivery N: an earlier delivery may already have run)` (`framing.ts:125-127`). It is cheap (about 15 tokens, only on a redelivered item) and tells the model to check before redoing side effects. herdr-threads re-offers (`REOFFER_AFTER_MS`) carry no such marker **I**.
8. **Positional anti-forgery framing.** Only connector lines start at column zero. Every Unicode line break in a peer body is indented, and brackets are stripped from attribution fields (`framing.ts:9-12`, `:64-98`). herdr-threads' indented body (R17) is the same idea; the Unicode line-break class (U+2028, U+2029, U+0085, VT, FF) is worth checking against `src/delivery/frame.rs` **I**.
9. **Bounded crash recovery with generation fencing.** Each app-server incarnation has a generation number, stale async tails stand down (`superseded(gen)`), and more than 3 restarts in 120 s is fatal (`host.ts:914-932`, `:987-1068`; `app-server.ts:225-255`) **V**.

Not worth copying: the per-block instruction tail, which adds about 45 tokens to every delivery against herdr-threads' token-efficiency rule (a)/(b). A full user turn for every non-directed ambient item under `open` attention is not worth copying either, because it violates herdr-threads delivery invariant 1 (a default send never wakes anyone). Cotal has no equivalent of the p1 "silent" tier: every directed item wakes.

## Verification

Adversarial pass on 2026-10-23 against the clone at `6426fb538e0b`, the Codex `rust-v0.163.0` sparse clone and the primer. Re-read every cited range in `host.ts`, `app-server.ts`, `mcp.ts`, `tui.ts`, `extension.ts`, `connector-core/src/{agent.ts,control.ts,framing.ts,orientation.ts,config.ts,tool-specs.ts,inline-delivery.ts}`, `docs/connect-codex.md`, `docs/connectors.md`, `docs/connect-claude.md` and `setup.ts`. The mechanism claims held: no hooks, `turn/start` wake, `turn/steer` with `expectedTurnId`, owned-turn acking, the Escape dismissal, `unknown` re-driving at once, and the Codex host never calling `holdInFlight`. The last was confirmed by grep: only OpenCode (`plugin.ts`, `plugin2.ts`), Claude (`hooks.ts`) and `inline-delivery.ts` call it.

Changed:
- `connectors.md` line numbers were off by one (maturity is on 23, containers on 34).
- The Codex input-limit citation `:1609` pointed at the detached-review prompt. The steer check is `turn_steer_inner` (`:1044-1068`).
- The 1 MiB "session rail" citation is the §13.6 console session protocol, not chat delivery. It is replaced by the broker `max_payload` (INFERRED default).
- Run-turn text is appended to the drive, not prepended.
- The tool-description size was re-measured (about 13,900 characters).
- The `composer_stash` reference pointed at primer §10, which does not mention it. It now cites `src/harness/recipe.rs`.
- Added the 60 s ack-wait source and the second agent.ts hazard comment.

