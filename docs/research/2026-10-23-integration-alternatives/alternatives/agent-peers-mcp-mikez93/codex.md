# agent-peers-mcp (mikez93) x OpenAI Codex CLI

- Upstream: https://github.com/mikez93/agent-peers-mcp, read at commit `2c63379d65ad44845815faa21a0c5c78355002fa`. Paths are relative to the clone root. Nothing in the clone was executed.
- Harness reference: `../../primers/codex.md` (Codex CLI 0.163.0). "Primer §N" points there.
- Tags: **VERIFIED** = read in source or docs at that commit. **INFERRED** = my reasoning from those, not observed running. **unknown** = not found.

## 1. Support status

**Native, in two tiers.** `codex-server.ts` is a first-class Codex MCP server. Its header says it is "shared by Codex CLI, Hermes Agent, and Factory Droid" (`codex-server.ts:1-4`), and its peer type defaults to `codex` (`codex-server.ts:103-107`). VERIFIED.

| Tier | How the session is started | Injection | Idle wake | Mid-turn push |
|---|---|---|---|---|
| **Plain** | `codex`, with one `[mcp_servers.agent-peers]` block in `~/.codex/config.toml` (`README.md:140-145`) | `[PEER INBOX]` piggyback on the next agent-peers tool result | **none** (`README.md:177,421`) | none. The model must call an agent-peers tool. |
| **Wakeable** | `codex-peer`, which runs a private `codex app-server --listen ws://127.0.0.1:<port>` plus a visible `codex resume --remote <url> <thread>` TUI (`README.md:322-327`; `shared/wakeable-launcher.ts:352-465`) | same piggyback | app-server `turn/start` with a bodyless wake prompt, sent by a wake daemon that polls every 5 s (`shared/wake-daemon.ts:186-213`) | app-server `turn/steer` with a bodyless steer prompt (`shared/wake-daemon.ts:190-203,270-285`) |
| **Desktop / SSH app-server thread** | a Codex app-server whose MCP child can find the parent's unix socket. The discovery uses `lsof` and is macOS-only. | same piggyback | same daemon, registered by exact `threadId` after the first agent-peers tool call (`shared/codex-desktop-wake.ts:13-99,131-156`) | same steer |

VERIFIED: all three rows. The README says no MCP reload can retrofit wakeability onto an existing stock thread; the thread has to be relaunched with `codex-peer resume <session-id>` (`README.md:177`).

## 2. Injection mechanism

### 2.1 Channel: a piggyback on MCP tool results (VERIFIED)

Codex drops MCP server notifications. They are only logged (primer §4, `logging_client_handler.rs:74-92`). The project's own comments reach the same conclusion: "the authoritative delivery channel is the [PEER INBOX] block in tool responses (Codex's only MCP input surface)" (`codex-server.ts:162-172`). So a message's content reaches the model only inside the text result of one of the server's six tools: `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages`, `rename_peer` (`codex-server.ts:227-299`).

Every `CallTool` goes through `withPiggyback` (defined at `codex-server.ts:479-547`, called from the `CallTool` handler at `:693-812`). It prepends the inbox block to that tool's own output:

```ts
// codex-server.ts:533-534
const inbox = formatInboxBlock(fresh);
const finalText = inbox + toolText;
```

`check_messages` itself does nothing and returns the text `Checked inbox.` (`codex-server.ts:729-732`). It exists only to carry the piggyback.

There is no hook, no `additionalContext`, no `thread/inject_items` and no keystroke path for content. VERIFIED by reading `codex-server.ts` in full and grepping for `hooks` and `notify`, which found nothing relevant.

### 2.2 When it fires

- **Only when the model calls an agent-peers tool.** VERIFIED (`codex-server.ts:178-182`: "you only see them when YOU call an agent-peers tool").
- **The turn-start cadence depends on the model obeying an instruction.** The MCP `instructions` say: "RULE: Call `check_messages` as the FIRST thing you do every time the user sends you a message" (`codex-server.ts:184-189`). The `check_messages` tool description repeats it (`codex-server.ts:262-265`). VERIFIED. Nothing in the harness enforces it (INFERRED).
- **Mid-turn delivery happens only if the model calls another agent-peers tool**, or if the wake daemon steers the turn (§3.3). VERIFIED.
- **Inline poll at call time.** Under the lock, each call first confirms the previous delivery, flushes acks, then polls the broker inline so that mail that landed in the last second is included (`codex-server.ts:551-681`, steps 1a, 1b, 2 and 3). VERIFIED.
- **Background poll.** It runs every 1 s for 120 s after peer activity, and every 3 s otherwise (`codex-server.ts:94-101`). It writes leased mail to the durable inbox file and to the bodyless metadata file. It does not present anything to the model (`codex-server.ts:446-470`; `shared/codex-inbox.ts:154-164,212-221`). VERIFIED.

### 2.3 Payload and framing (VERIFIED, `shared/piggyback.ts:35-63`)

```
PEER INBOX — N unread message(s) from your colleagues.
Read each one. Then:
  - If it's a question you can answer now: reply briefly via send_message(to_id="<from_name>", ...).
  - If it needs investigation: go investigate. ...
  - If it's FYI: note it internally, don't auto-reply. Silence is fine.
  - If you disagree: push back with a specific reason.
Do NOT auto-acknowledge. "Got it" / "on it" is noise.

--- message 1 of N ---
from: <name> (<peer_type>, cwd=<cwd>)
their current work: <summary>        (optional)
sent_at: <iso>
message_id: <int>
text:
<body verbatim>
reply_action: send_message(to_id="<name>", message="...")

PEER INBOX
```

- The behavioural header is 481 characters (483 UTF-8 bytes, measured from `shared/piggyback.ts:40-47` with N as one digit). It is repeated on **every** non-empty delivery. VERIFIED.
- Each message adds about 6 framing lines: absolute cwd, ISO timestamp, the summary, and a reply hint that repeats the sender name. VERIFIED.
- Peer text and summaries are interpolated verbatim. The project's own security review files this as S3, "Peer-supplied text and summaries are interpolated verbatim into an instruction block" (`docs/issues/SECURITY-CONCERNS.md`, S3 heading). VERIFIED.

### 2.4 Bounds

- **Count**: unbounded per delivery. Every unconfirmed durable row is presented. The broker poll has no `LIMIT` (`broker.ts:901-910`). The only cap is 500 unacked rows per recipient at send time (`MAX_QUEUED_PER_DURABLE_PEER`, `broker.ts:55`; enforced for peers at `:842,857` and for Hermes conversation mailboxes at `:808-809`). VERIFIED.
- **Body size**: no cap. The project's own security review says so: "No size limits on message bodies, summaries, or request bodies" (`docs/issues/SECURITY-CONCERNS.md` S4). A grep of `broker.ts`, `shared/piggyback.ts` and `codex-server.ts` found no truncation either. VERIFIED.
- **What Codex does with an oversized tool result** depends on Codex's own MCP output handling (`tools.<t>.output_token_limit`, primer §4). That default was not established in the primer, so it is unknown.

### 2.5 De-duplication and acknowledgement (VERIFIED)

The state machine lives in `shared/delivery-state.ts` and is used in `codex-server.ts:303-315,479-681`:

1. **Lease.** The broker leases each row for 30 s (`broker.ts:27`).
2. **Persist.** The MCP upserts the row into `~/.agent-peers-codex/<peer>.json` (mode 0600) before anything else (`codex-server.ts:386-398`).
3. **Draw.** The row is drawn into this call's response.
4. **Promote.** Once the response is fully built, the draw is promoted. A cancelled request (`extra.signal.aborted`) rolls back instead, so the row can be dealt again (`codex-server.ts:535-545`).
5. **Confirm, prune, ack.** These happen only when the **next** agent-peers call arrives (`codex-server.ts:569-582`).

An "arrival-causality barrier" snapshots confirm eligibility when the request enters, before any long wait. A parked `wait_for_peer_messages` call therefore cannot confirm a sibling call's later response (`codex-server.ts:490-496`).

Parallel tool calls are serialized by a lock, so the same message cannot be dealt into two responses (`codex-server.ts:472-477`).

There is no semantic ACK. The model is told not to acknowledge (`shared/piggyback.ts:47`). The "ack" is transport-level only and means "the next call proves the previous result arrived".

### 2.6 Dormant secondary channel (VERIFIED)

Each new message also triggers an MCP `notifications/message` (`codex-server.ts:410-437`). Its text is signal-only: "new message from X … will appear as [PEER INBOX] in your next agent-peers tool response" (`shared/piggyback.ts:27-33`). The README calls it "dormant" on current Codex (`README.md:266,428-429`). This matches primer §4, where notifications are only logged.

### 2.7 Long-poll tool `wait_for_peer_messages` (VERIFIED)

- **Behaviour.** It blocks for up to 300 s on Codex (`codex-server.ts:122-124`), polling every 500 ms. An optional `from` filter only decides what *ends* the wait; mail from other senders still surfaces (`codex-server.ts:326-341`).
- **Wakeable sessions.** There it returns at once, because the daemon will wake the session instead (`codex-server.ts:734-748,799-803`; `shared/wait-for-peer-messages.ts:33-49`).
- **Timeout conflict (INFERRED).** Codex's MCP `tool_timeout_sec` defaults to 60 s (primer §4). The wire-only config block does not raise it (`README.md:141-145`). A 300 s wait on a plain Codex session should therefore be cut off by Codex at 60 s.
- **Consequences of that cut-off (INFERRED).** The request may be aborted, which rolls the draws back. Whether Codex sends MCP `notifications/cancelled` on a timeout is unknown. The model then sees a tool timeout error instead of the "No peer messages arrived" text.

## 3. Wake mechanism (wakeable tier only)

### 3.1 Moving parts

**Launcher `codex-peer`.** Source: `bin/codex-peer` → `shared/wakeable-launcher.ts:352-465`. VERIFIED. It launches in two phases.

Phase 1 runs on a throwaway app-server whose agent-peers MCP is disabled (`AGENT_PEERS_ENABLED=0`):
- It calls `thread/start` with `historyMode: "legacy"`. That parameter requires the `experimentalApi` capability (`shared/wakeable-launcher.ts:490-507`; `shared/app-server-client.ts:119-126`).
- It then calls `thread/name/set`, which forces the rollout JSONL onto disk without running a model turn (`shared/app-server-client.ts:194-212`).

Phase 2 starts the real app-server on `ws://127.0.0.1:<ephemeral>`:
- The app-server gets `-c mcp_servers.agent-peers.required=true` and `AGENT_PEERS_WAKE_LAUNCH=1` (`shared/wakeable-launcher.ts:290-296`).
- A visible `codex resume --remote <url> <thread>` TUI is attached to the same thread (`:90-112,448-456`).
- When the TUI exits, the launcher kills the app-server (`:457-459`).

**Identity binding.** VERIFIED.
- The app-server, not the TUI, spawns one MCP child per Codex session/thread. Those children race an exclusive-create root claim, and the loser serves zero tools (`codex-server.ts:870-907`; `docs/wakeable-codex.md:138-195`).
- The winner reads the launch claim, matched by cwd and tty with a 30 s wait (`codex-server.ts:883-894`).
- It then writes `peer_id -> {app_server_url, thread_id, rollout_path, pids, session-token hash}` into `~/.agent-peers-codex/wake-registry.json` (`codex-server.ts:1180-1233`).

**Bodyless metadata.** Every inbox change also rewrites `~/.agent-peers-codex/<peer>.metadata.json`. It holds the id, sender, cwd, summary and `sent_at`, and no body and no lease token (`shared/codex-inbox.ts:26-40,212-221,275-286`). VERIFIED.

**Daemon.** VERIFIED.
- `bin/codex-peer` runs `bun wake-daemon.ts --quiet-noop` in a shell loop every `CODEX_PEER_DAEMON_INTERVAL` seconds, 5 by default, as a fresh process per pass (`bin/codex-peer:14,305-326`).
- Every launch auto-starts it with `nohup`. A pidfile and lock directory keep it to a single instance (`bin/codex-peer:228-262`).

### 3.2 One wake pass (VERIFIED, `shared/wake-daemon.ts:102-261`)

For each registry entry:
1. If the peer's metadata has no unread messages, skip it, with no network call (`:130-134`).
2. If the peer is known to be wedged (`systemError`) and still inside its 5 m → 30 m → 2 h re-check backoff, skip it (`:141-145`).
3. Compute a signature: sha256 of `peer_id` plus the sorted unread ids (`:84-88,147`).
4. Connect over WS with a 5 s timeout per connect and per request (`shared/app-server-client.ts:65-69`). Check `thread/loaded/list` (`:150-154`).
5. Call `thread/read`. `validateThread` compares thread id, cwd and rollout path with the registry, then maps the status (`:287-300`):
   - `idle` → wake
   - `active` with no flags → steer, if the client supports it
   - `active{waitingOnApproval|waitingOnUserInput}` → skip
   - `notLoaded` / `systemError` / anything else → skip
6. Claim the signature in the wake ledger (`:163-167`, `WakeLedger.claim` `:367-401`):
   - The first sight of a signature fires at once.
   - Re-firing the *same* signature waits 5 m, then 30 m, then 2 h, then gives up ("abandoned").
   - A new message changes the signature and fires at once.
7. **Re-read the thread.** If its state changed, release the claim (`:169-184`).
8. If the thread is active, steer it (`:190-203`). Otherwise send `turn/start` (`:204-213`). Either way, the ledger records `nudged`.

The wake turn's input is a single text item:

```ts
// shared/wake-daemon.ts:90-93 (one line in source; reflowed here)
`[agent-peers wake]
You have pending agent-peers messages in your local MCP inbox. Immediately call the agent-peers
check_messages tool once. Treat that tool response as the only authoritative message content. Do
not infer message content from this wake signal. After handling the inbox normally, return to
waiting. Wake id: ${wakeId}`
```

`turn/start` is sent with `clientUserMessageId: agent-peers-wake-<uuid>` (`shared/app-server-client.ts:220-235`).

### 3.3 Busy turns: `turn/steer` (VERIFIED)

```ts
// shared/app-server-client.ts:237-265
thread/turns/list {threadId, limit:1, sortDirection:"desc", itemsView:"notLoaded"}  -> in-progress turn id
turn/steer {threadId, expectedTurnId, clientUserMessageId, input:[{type:"text", text}]}
```

- **Steer prompt.** It is 426 characters plus a 36-character wake UUID (462 total), and bodyless: "[agent-peers message while you work] … Call the agent-peers check_messages tool once now … Then continue the task you were working on" (`shared/wake-daemon.ts:97-100`).
- **Fencing.** An `expected active turn id` or `no active turn` error returns null (`:282`). The daemon then re-reads: an idle thread gets a normal wake turn, and anything else releases the claim and retries on the next pass (`:192-202`).
- **Doc contradiction.** The README still says a deeply-thinking active thread "is never nudged" (`README.md:338`), and `docs/wakeable-codex.md:120-122` says an active thread "is skipped before the ledger is touched". Both contradict this code. `docs/wake-daemon.md:24` matches the code.

### 3.4 Idle and busy detection

- **Source.** Detection comes only from app-server thread status, read with a `thread/read` poll every 5 s. There is no subscription to `thread/status/changed`, no screen scraping and no `notify` hook. VERIFIED (`shared/wake-daemon.ts:156-161`; `shared/app-server-client.ts:184-192`).
- **Precision.** The signal is precise (primer §2.3).
- **Acknowledged race.** "This narrows, but cannot atomically eliminate, the gap between observing idle and turn/start (which can append to a busy turn)" (`shared/wake-daemon.ts:169-172`). VERIFIED as their statement.
- **Interrupted turns.** After a human Ctrl-C the thread reads `idle`, so the daemon wakes it. That differs from Codex's own `thread/queue`, which refuses to auto-dispatch after an interrupt (primer §3). INFERRED from the status mapping, which has no interrupted case.

### 3.5 Draft safety

- **No typing.** No keystrokes are sent; `turn/start` and `turn/steer` are app-server RPCs. The composer draft belongs to the `--remote` TUI process and should survive. INFERRED; the docs do not test it.
- **Steer still competes with the human's turn (INFERRED).** A steer lands inside a turn the human started and tells the model to handle the peer mail and then continue. Per primer §7, an Enter that the human types during that wake turn becomes a steer of the wake turn, not a new turn.
- **Spinner bug.** The project documents a UI defect: the `--remote` TUI shows "working" forever after a turn started by another client, so an idle peer *looks* hung. Users are told to trust `codex-peer live` (`docs/wakeable-codex.md:207-220`). VERIFIED as documented.

### 3.6 Latency (INFERRED from the constants)

| Step | Delay |
|---|---|
| Sender → recipient MCP background poll | ≤ 1 s active, ≤ 3 s idle (`codex-server.ts:94-101`), which writes the metadata |
| Metadata → daemon pass | ≤ 5 s (`bin/codex-peer:14`), plus a `bun` cold start per pass |
| Wake turn | one model sampling round to emit the `check_messages` call, then a second round with the inbox |

Total: about 1–8 s until the wake turn starts, plus two sampling rounds before the model can act on the content.

### 3.7 Failure modes

| Mode | Behaviour | Evidence |
|---|---|---|
| App-server hung or dead | `app_server_unreachable`; skipped, the pass continues with the next peer | `shared/wake-daemon.ts:234-237` VERIFIED |
| `turn/start` throws | ledger `failed`. The attempt still counts, so the next try waits out the 5 m backoff | `:230-232` with `claim` `:380-397` VERIFIED code; the 5 m consequence is INFERRED |
| Thread in `systemError` | logged with a "bounce it" hint, rechecked on a 5 m → 30 m → 2 h backoff, never woken | `:69-78,505-510` VERIFIED |
| Waiting on approval or user input | skipped every pass; once the turn resumes it is `active`, so it gets steered | `:294-295`; `docs/wake-daemon.md:25` VERIFIED |
| Same unread set after 4 wakes | abandoned; it surfaces on the next tool call or user turn | `:381-391` VERIFIED |
| Launch claim not matched within 30 s (cwd/tty) | the MCP registers as a peer but is not wakeable. `repair-wake` matches by cwd/tty and refuses when ambiguous | `codex-server.ts:883-894`; `docs/wakeable-codex.md:130-134` VERIFIED |
| Codex history-mode change | 0.151.0 made `paginated` the default, which silently broke every launch until `historyMode:"legacy"` was pinned. That relies on an experimental API parameter | `docs/wakeable-codex.md:94-100` VERIFIED |
| TUI closed | the launcher kills the app-server, and the registry row is pruned after a 30 min dead grace | `shared/wakeable-launcher.ts:457-459`; `shared/wake-daemon.ts:67,251-254` VERIFIED |
| Non-macOS Desktop / SSH thread | `lsof` discovery returns `[]` off darwin, so the thread is not wakeable | `shared/codex-desktop-wake.ts:35` VERIFIED |

### 3.8 Redundant wake after an FYI message (INFERRED, not documented)

The metadata file mirrors the durable store. Rows that were *presented* but not yet confirmed stay in that store until the **next** agent-peers call (§2.5). Take this sequence:

1. A wake turn calls `check_messages`.
2. The message is FYI, so the model correctly does not reply.
3. The turn ends with no further agent-peers call.

The metadata still lists the same ids, so the signature is unchanged. After the 5 m backoff the daemon fires a second full-context wake. That turn's `check_messages` only confirms the old rows and returns `Checked inbox.`. The colleague protocol discourages replies ("Silence is fine"), so this pattern should be common.

The docs claim the backoff "removes the routine 'one redundant wake' on the happy path" (`docs/wakeable-codex.md:119`). The backoff only postpones that wake by 5 minutes. A pending-presented flag in the metadata, or a confirm at turn end, would remove it.

## 4. Setup and installation footprint on Codex

| Item | Written by | Evidence |
|---|---|---|
| `~/.codex/config.toml`: `[mcp_servers.agent-peers] command="bun" args=["…/codex-server.ts"] env={AGENT_PEERS_ENABLED="1"}`. No `required` and no `tool_timeout_sec` | **a Codex agent following a paste-in prompt** ("wire-only install"); there is no installer script | `README.md:115-160` VERIFIED |
| No hooks, no `hooks.json`, nothing to trust in `/hooks`, no `notify`, no skills or plugins | n/a | grep VERIFIED |
| `~/.local/bin/codex-peer` symlink | `codex-peer install` | `bin/codex-peer:7,329-335` VERIFIED |
| Per launch, only as `-c` overrides, never in `config.toml`. Phase-2 app-server: `PEER_NAME`, `AGENT_PEERS_WAKE_LAUNCH=1`, `required=true`, plus fresh-thread model and permission overrides (`:225-242,290-296`). Visible TUI: `AGENT_PEERS_WAKE_*` `-c` env overrides and process env (`:88-112,244-264,333-350`), which the code says are ignored in `--remote` mode, so the launch claim is authoritative (`:409-413`). Phase-1 app-server: `AGENT_PEERS_ENABLED=0` (`:307-331`) | launcher | `shared/wakeable-launcher.ts` VERIFIED |
| Hard-coded fresh-thread defaults: `model="gpt-5.6-sol"`, `model_reasoning_effort="high"`, `approval_policy="never"`, `sandbox_mode="danger-full-access"` ("Mike's full-access contract"). Native flags can narrow them | launcher | `shared/wakeable-launcher.ts:141-148,298-305,494-500` VERIFIED |
| `~/.agent-peers-codex/`:<br>`<peer>.json` (bodies)<br>`<peer>.metadata.json`<br>`wake-registry.json`<br>`wake-launch-claims/`<br>`wake-ledger/`<br>`wake-observe/`<br>`logs/<peer>-app-server.log`<br>`wake-daemon.{pid,log,lock}`<br>All files 0600 in a 0700 directory, fail-closed on read | MCP, launcher, daemon | `shared/codex-inbox.ts:51-58,223-257`; `shared/wake-registry.ts:169`; `shared/wake-launch-claims.ts:92`; `shared/bounded-log.ts:56-60`; `bin/codex-peer:11-13` VERIFIED |
| Shared broker state `~/.agent-peers.db` and `~/.agent-peers-secret`, plus the launchd broker. The Claude-side install owns these | Claude install | overview; `README.md:63` VERIFIED |
| A background daemon process that outlives the terminal | `ensure_daemon` (`nohup`) | `bin/codex-peer:228-262` VERIFIED |

## 5. Token cost

Sizes are source character counts, measured on the clone. Tokens assume about 4 characters per token (INFERRED).

| Item | Size | When |
|---|---|---|
| MCP `instructions` (delivery prologue about 2.8 KB plus `COLLEAGUE_PROTOCOL`, a source file of about 6.4 KB) | ≈ 8–9 KB ≈ 2k tokens | once per session; static, possibly cached |
| 6 tool schemas | small (a few hundred tokens, INFERRED) | per session |
| **Quiet user turn** (no mail) | one obligatory `check_messages` call: the model's call plus the `Checked inbox.` result, ≈ 20–40 tokens, and **one extra sampling round-trip** per user turn | every user turn, if the model obeys |
| Per delivery | 481-character header (≈ 120 tokens) + ≈ 6 framing lines per message (≈ 40–60 tokens plus cwd/summary) + body | each non-empty piggyback |
| Wake | 324-character prompt plus a 36-character wake UUID, 360 total (≈ 90 tokens) **plus a full re-bill of the thread context**, which the project acknowledges (`docs/wakeable-codex.md:105-108`), and 2 sampling rounds | each idle wake |
| Steer | 462-character prompt including the UUID (≈ 115 tokens) plus one extra tool round inside the running turn | each busy delivery |
| Idle with no mail | 0 model tokens; the daemon reads local files and makes a WS call only when metadata shows unread mail | `docs/wakeable-codex.md:101-104` VERIFIED |

## 6. Limitations and bugs

1. **The plain `codex` TUI cannot be woken.** Turn-start delivery depends on the model following an instruction. Mid-turn delivery happens only if the model happens to call an agent-peers tool. VERIFIED.
2. **Wakeability is fixed at launch.** It needs the project's own launcher, which replaces the user's normal `codex` invocation with an app-server plus a `--remote` TUI. VERIFIED. In Herdr terms this changes how the pane's process is started (INFERRED).
3. **The wakeable app-server is unauthenticated on TCP.**
   - The app-server listens on `ws://127.0.0.1:<port>` with no `--ws-auth`, and that port is published in `wake-registry.json`. The project's own security doc rates this HIGH: any local process can `turn/start` into a session running `approval_policy=never` + `danger-full-access` (`docs/issues/SECURITY-CONCERNS.md` S2; `shared/wakeable-launcher.ts:388-390`). VERIFIED.
   - Primer §2.1 lists `--ws-auth capability-token|signed-bearer-token` and unix sockets as alternatives.
4. **Version coupling to history mode.** The launcher depends on `historyMode:"legacy"`, an experimental-API parameter, and on rollout JSONL files existing. VERIFIED. Codex has already broken this once (0.151.0).
5. **Hard-coded personal defaults** for model, reasoning effort and full-access sandbox. VERIFIED.
6. **No priorities or thresholds.**
   - Every new message wakes an idle session, or steers a busy one. VERIFIED.
   - A "p1-style" silent delivery is not possible except by leaving the session non-wakeable. INFERRED.
7. **Steering a busy turn is unconditional.** It ignores whether the human started that turn. VERIFIED code. The README and `docs/wakeable-codex.md` contradict it (`README.md:338`; `docs/wakeable-codex.md:120-122`).
8. **A redundant wake follows every unreplied delivery**, 5 minutes later. INFERRED; see §3.8.
9. **Interrupted turns are woken.** A Ctrl-C'd turn is treated as idle, so the daemon wakes it. INFERRED.
10. **Polling, not events.** A fresh `bun` process starts every 5 s, and it re-reads thread status instead of subscribing to `thread/status/changed` or `turn/completed`. VERIFIED. Cheap when quiet, but adds up to 5 s of latency (INFERRED).
11. **Unbounded inbox block** in both message count and body size; there is no `body <id>` continuation. VERIFIED (code, and `SECURITY-CONCERNS.md` S4).
12. **Static instructions are repeated in every block.** The 481-character header goes into every delivery, and per-message lines carry absolute cwd and reply hints. VERIFIED.
13. **`wait_for_peer_messages` is longer than Codex's tool timeout.** The 300 s default exceeds Codex's 60 s `tool_timeout_sec` on plain sessions. INFERRED.
14. **Discovery and liveness lean on macOS.** The Desktop-thread path is macOS-only (`lsof`), and the broker is owned by launchd. VERIFIED.
15. **Phantom "working" spinner** after externally started turns. VERIFIED as documented.

## 7. Clever ideas worth stealing

1. **`turn/steer` with `expectedTurnId` for busy sessions** (`shared/app-server-client.ts:247-265`).
   - It is a native, typing-free mid-turn delivery. The fence is the turn id from `thread/turns/list(limit:1, desc)`. On a turn-id mismatch the code falls back to `turn/start` only after re-reading idle.
   - Herdr-threads today serves busy Codex sessions only at `PreToolUse ^Bash$` (primer §10).
   - **Caveat for us:** it needs a connection to the app-server that holds the thread, which conflicts with our `--no-daemon` advice (primer §11.2).
2. **The app-server status gate before every wake**, including identity re-validation (thread id, cwd, rollout path) and a second `thread/read` just before sending (`shared/wake-daemon.ts:156-184,287-300`). It is more precise than screen-scraped `agent_status` plus a composer read, and it cannot type over a draft.
3. **Bodyless wake and steer prompts that carry a wake id.** This is the same as our R18. The *steer* variant tells the model to resume its task afterwards, which is a useful wording pattern.
4. **The signature ledger.** sha256 of the recipient plus the sorted unread ids, with escalating re-wake backoff and a hard cap. A new message always wakes at once. It is a simple, crash-safe file ledger that bounds the cost of an ignored inbox. Our re-offer bound (60 s, 3 per generation) is similar in intent.
5. **Confirm-on-next-call with an arrival-causality barrier and abort rollback** (`codex-server.ts:479-547`). Delivery to a model is not marked done until a later call proves the previous tool result was received. It handles cancelled tool calls (`extra.signal`). Our `inbox` "ACK on display" is stronger semantically, but the abort-rollback idea applies to any of our in-turn hook deliveries.
6. **The bodyless metadata file as the waker's only input** (`shared/codex-inbox.ts:212-221`). It separates "is there mail" from "what is the mail" at the file-permission level.
7. **Root election for per-thread MCP children** (`codex-server.ts:870-907`). The Codex app-server spawns one MCP per thread (forks, subagents, reviews). An exclusive-create claim plus `AGENT_PEERS_WAKE_LAUNCH=1` keeps it to one identity per launch, and the losers serve an empty MCP so `required=true` still lets the thread start. This is relevant if we ever ship an MCP for Codex.
8. **`_meta.threadId` on MCP `tools/call`** as an authoritative thread identity. The project notes it is absent on `tools/list` (`docs/wakeable-codex.md:160-166`; `shared/codex-desktop-wake.ts:18-22`). This is a cheap identity source for an MCP-based integration.
9. **Materializing without a model turn** through `thread/name/set` (`shared/app-server-client.ts:194-212`). It is a zero-token way to persist a thread before attaching a TUI, if we ever manage app-server-hosted Codex.
10. **Wake-aware long-poll suppression.** `wait_for_peer_messages` becomes a no-op once the session is known to be wakeable, so a model cannot pin its own turn "working" waiting for a reply (`codex-server.ts:734-748`).

## Verification

Adversarial pass against the clone at `2c63379`, the primer, and measured string lengths.

- Checked and confirmed: the tool list, `withPiggyback` and its abort rollback, the confirm-on-next-call steps and arrival barrier, the lock, the `Checked inbox.` no-op, the poll cadences, the `notifications/message` preview, the `wait_for_peer_messages` limits and wakeable skip, the root election and wake registry, the bodyless metadata, the daemon loop and `ensure_daemon`, every step of the wake pass (status mapping, ledger schedule, 4-attempt cap, re-read, steer fencing and fallback), the 5 s RPC timeout, `historyMode:"legacy"` with `experimentalApi`, the `thread/name/set` materialization, the hard-coded model and permission defaults, the macOS-only `lsof` discovery, the README install block, security items S2 and S3, and the primer sections cited.
- Corrected: the inbox header is 481 characters, not 558. The wake prompt is 324 characters plus the UUID, not 344. The steer prompt is 426 plus the UUID, not 446. The `withPiggyback` citation pointed at its call site. The 500-row cap citation pointed only at the conversation-mailbox branch. The per-launch `-c` row put the TUI's `AGENT_PEERS_WAKE_*` overrides on the app-server.
- Upgraded: "no body-size cap" moves from INFERRED to VERIFIED, because the project's own S4 states it.
- Added: `docs/wakeable-codex.md:120-122` contradicts the steer code, as `README.md:338` does.
- Left INFERRED: the 60 s `tool_timeout_sec` cut-off, the interrupted-turn wake, draft safety, latency, token estimates, and the redundant FYI wake (§3.8). Their reasoning follows from the code that was read.
