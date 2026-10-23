# agent-peers-mcp (mikez93): overview

- Upstream: https://github.com/mikez93/agent-peers-mcp (a GitHub fork of https://github.com/Co-Messi/agent-peers-mcp, which in turn calls itself a "hardened successor" of https://github.com/louislva/claude-peers-mcp; `README.md:21`).
- Read at commit `2c63379d65ad44845815faa21a0c5c78355002fa` (2026-10-21), shallow clone. All paths below are relative to the clone root.
- Evidence tags: VERIFIED = read in source or docs at that commit; INFERRED = my reading, not stated outright.

## What it is

It is a local, single-machine peer-discovery and messaging layer for coding agents. It runs as a loopback HTTP + SQLite broker daemon, and each agent session loads an MCP server that registers the session as a peer. It supports Claude Code, Codex CLI, Hermes Agent and Factory Droid (`package.json` description; `README.md:3-5`). VERIFIED.

The fork has its own design. It is not a light patch: idle-wake subsystems for Codex (app-server), Droid (ACP or the native daemon relay), Claude (an `asyncRewake` Stop hook) and Hermes (host RPC, opt-in) all live only in the fork's docs and code. The parent Co-Messi repo was last pushed on 2026-07-13 (GitHub API). VERIFIED that these subsystems are present in the fork. INFERRED that they are fork-only, because the parent was not cloned.

It ships a "colleague protocol" prompt shared by every server (`shared/colleague-prompt.ts`, `README.md:52`). The prompt tells agents not to auto-ack ("got it" is noise), to investigate before replying, and to ping proactively. VERIFIED.

## Architecture

| Aspect | Finding | Evidence |
|---|---|---|
| Language/runtime | Bun + TypeScript, `bun:sqlite`, `@modelcontextprotocol/sdk`, no build step | `AGENTS.md:7`, `package.json`. VERIFIED |
| Broker | One daemon per machine, `broker.ts`, `127.0.0.1:7900`, SQLite at `~/.agent-peers.db` (0600). Owned by launchd (`com.mike.agent-peers-broker`). Clients run `launchctl kickstart` and never spawn it unless `AGENT_PEERS_SPAWN_BROKER=1` | `docs/ARCHITECTURE.md:42-72,153-174`; `AGENTS.md:28-29`. VERIFIED. macOS-centric: README says "Linux should work but untested" (`README.md:774`) |
| Auth | Per-user shared secret `~/.agent-peers-secret` (0600) sent as the `X-Agent-Peers-Secret` header, plus a rotating per-peer session token. Readiness check is fail-closed (`/ready` with a matching `db_id`) | `broker.ts:1389`; `docs/ARCHITECTURE.md:161-171`; `README.md:545`. VERIFIED |
| Per-session process | One MCP server per agent session: `claude-server.ts`, `codex-server.ts`, `hermes-server.ts`, `droid-server.ts`. Each polls the broker (Claude: 1 s active / 3 s idle) | `claude-server.ts:53-59`; `README.md:259-266`. VERIFIED |
| Client-side durable inbox | `~/.agent-peers-{claude,codex,hermes,droid}/<peer-uuid>.json` (0600), plus a **bodyless** `<peer>.metadata.json` that wakers read | `docs/ARCHITECTURE.md:144-149`; `docs/delivery-contract.md:96-98`. VERIFIED |
| Wake helpers | Codex: per-session `codex app-server` plus one wake daemon per machine (`wake-daemon.ts`, `bin/codex-peer`). Droid: `droidpeer` long-lived ACP host or native relay. Claude desktop: `bin/claude-peer-rewake` hook. Hermes: T2 host-RPC coordinator (opt-in) | see "Wake" below. VERIFIED |
| Cross-machine | **No.** Designed in `docs/plans/cross-machine-federation-tailscale.md` but deliberately not built. Only a `host` column and `message_uid` exist as groundwork | `docs/ARCHITECTURE.md:180-184`; `AGENTS.md:75-77`. VERIFIED |
| Opt-in gate | The MCP stays a no-op unless `AGENT_PEERS_ENABLED=1`. Sessions with `PAPERCLIP_AGENT_ID` / `PAPERCLIP_RUN_ID` are refused | `README.md:415-418`; `AGENTS.md:37-42`. VERIFIED |

## Identity and session model

- One session is one peer with one UUID. Names are an exclusive layer on top: a name that is already taken gets a suffix (`vector-2`) rather than a rejection. The default name is `<persona from AGENTS.md>-<repo>-<harness>`, overridable with `PEER_NAME` (`docs/ARCHITECTURE.md:76-90`; `README.md:194-203`). VERIFIED.
- A row is durable only with `PEER_NAME` set and `AGENT_PEERS_EPHEMERAL != 1`. Ephemeral rows age out about 60 s after heartbeats stop. A restart that keeps the same `PEER_NAME` within 60-90 s reclaims the UUID and clears stale leases. A `prev_id` re-points unacked mail to the new incarnation, but only when the old row is gone (`docs/delivery-contract.md:48-50,121-122`; `README.md:57`). VERIFIED.
- The binding is to a process, not a pane:
  - The Claude rewake hook finds its peer by walking the process tree: hook ancestor = the Claude process, whose `claude-server.ts` child pid = `peers.pid` (`bin/claude-peer-rewake:17-20`). VERIFIED.
  - Codex wakeable launches bind `peer_id -> {app_server_url, thread_id, cwd, rollout_path}` through a launcher-to-MCP-child claim handshake (`shared/wake-launch-claims.ts`, `shared/wake-registry.ts`). VERIFIED.
  - The Codex app-server spawns one MCP per thread (forks, subagents, reviews). An atomic exclusive-create "root election" therefore gives one identity per launch, and the losers serve an empty MCP (`README.md:398-413`). VERIFIED.
- Hermes:
  - Legacy mode runs one durable peer per profile. A file-lock election decides between the gateway and serve surfaces (`shared/hermes-claims.ts`; `docs/delivery-contract.md:79-85`). VERIFIED.
  - Opt-in v2 (`AGENT_PEERS_HERMES_V2=1`) gives one peer per Hermes conversation. The identity key is `[home, conversation_id]`, taken from host-supplied MCP `params._meta` (`hermes/conversation_id`, `hermes/backend_id`, …), never from tool arguments (`docs/hermes-conversation-bindings.md:7-12,55-80`). VERIFIED.
- Codex Desktop / SSH app-server threads discover the parent app-server's unix socket via `lsof` (macOS only) and register as wakeable by exact `threadId` (`shared/codex-desktop-wake.ts:1-45`). VERIFIED.

## Injection (how a message reaches the model)

| Harness | Path | When | Evidence |
|---|---|---|---|
| Claude Code (terminal `agentpeers`) | MCP experimental capability `claude/channel`. The server emits `notifications/claude/channel` with `content` = full body and `meta` = {message_id, from_id, from_name, from_peer_type, from_summary, from_cwd, sent_at}. The model sees `<channel source="agent-peers" …>` | Mid-turn whenever the poll finds mail. The server instructions say that when Claude is idle the push "queues invisibly" until the next turn | `claude-server.ts:100-140,520-545`; `docs/delivery-contract.md:44-47`. VERIFIED |
| Claude Code (fallback / desktop app) | `check_messages` tool returns a `[PEER INBOX]` block that unions the ring buffer (last 15 min, max 50) with all unread durable mail. Instructions tell the model to call it first on every user turn | Turn start (prompt-instructed), or on demand | `claude-server.ts:108-140`; `shared/recent-delivered.ts:30-31`. VERIFIED |
| Codex CLI | Piggyback: the `[PEER INBOX]` block is prepended to the **next agent-peers tool response** (any of `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages`, `rename_peer`). Also `notifications/message` previews, which the README says current Codex drops ("dormant") | Only when the model calls one of its tools. The prompt asks for `check_messages` at every turn start | `codex-server.ts:229-291`; `shared/piggyback.ts:20-80`; `README.md:262-266,428-429`. VERIFIED |
| Hermes | Same durable piggyback transport as Codex. Bounded `wait_for_peer_messages`, capped at 60 s | On tool call | `docs/delivery-contract.md:73-78`. VERIFIED |
| Factory Droid | Same shared durable piggyback transport | On tool call (after a bodyless wake) | `docs/delivery-contract.md:91-109`. VERIFIED |

Framing (VERIFIED, `shared/piggyback.ts:35-60`):
- A verbose header of about 8 lines of behavioural instructions on every block ("Read each one. Then: … Do NOT auto-acknowledge").
- Per message: `from` (with cwd), optional `their current work`, `sent_at`, `message_id`, `text`, and a `reply_action: send_message(...)` hint.

Static instructions are repeated per delivery. This is the opposite of our R17 rule. INFERRED.

Bounding: the broker caps unread mail at 500 per durable peer (`broker.ts:55,808`). The poll returns **all** unleased unacked rows with no LIMIT (`broker.ts:901-910`). No per-body size limit or truncation was found (grep of `broker.ts`, `shared/piggyback.ts`). The absence of a cap is INFERRED and should be treated as unknown.

Ordering: by broker message id ascending (`broker.ts:909`). There are no priorities. VERIFIED.

De-duplication: an in-memory seen-set by `message_id` with watermark pruning (`claude-server.ts:546-552`), plus the on-disk store keyed by id (`docs/delivery-contract.md:20-23`). VERIFIED.

## Waking an idle agent

All wakes are **bodyless**. The wake prompt only tells the model to call `check_messages`. Wakers read only the bodyless metadata file and never ack (`docs/wakeable-codex.md:52-66`; `shared/wake-daemon.ts:91-92`; `shared/droid-launcher.ts:69`). VERIFIED.

- **Codex (wakeable mode, `codex-peer`):**
  - The launcher starts `codex app-server --listen ws://127.0.0.1:<port>` and a visible `codex resume --remote <url> <thread>` TUI on the same thread (`README.md:322-327`). VERIFIED.
  - The wake daemon polls every 5 s by default (`CODEX_PEER_DAEMON_INTERVAL`). On each pass it re-reads the thread and validates thread_id, cwd and rollout path, then:
    - sends `turn/start` with the wake prompt only when the thread status is `idle`;
    - skips `waitingOnApproval`, `waitingOnUserInput`, `systemError` and `notLoaded` (`shared/wake-daemon.ts:288-298`).
  - **Contradiction with the README.** The README says an active thread is "never nudged" (`README.md:338`). The current code instead **steers** an active turn with `turn/steer` (`expectedTurnId`-fenced, `buildSteerPrompt`) whenever the client supports it (`shared/wake-daemon.ts:180-200,263-285`; `shared/app-server-client.ts:247-263`). VERIFIED in code. The README is stale.
  - Backoff: a re-wake of the same unread-set signature (sha256 of peer id + sorted ids) backs off 5 m → 30 m → 2 h and is then abandoned. A new message always wakes at once (`docs/wakeable-codex.md:114-122`; `shared/wake-daemon.ts:85-88,370-400`). VERIFIED.
  - Idle vs busy comes from the app-server thread status, not a pane heuristic. The code admits a race window between the idle check and `turn/start` (`shared/wake-daemon.ts:170-173`). VERIFIED.
  - Draft safety: the wake goes through app-server JSON-RPC, not keystrokes, so a composer draft is not typed over. INFERRED; not tested in the docs.
  - It requires `historyMode: "legacy"` on `thread/start` because codex-cli 0.151.0 made paginated history the default (`docs/wakeable-codex.md:94-100`). VERIFIED. This is a version coupling.
- **Codex Desktop / SSH-hosted threads:** exact-thread registration via a discovered parent app-server socket (macOS `lsof`). "App-owned stdio hosts simply stay non-wakeable" (`shared/codex-desktop-wake.ts:1-2`). VERIFIED.
- **Claude Code:**
  - Terminal sessions have the channel, but an idle-queued push does not start a turn (`docs/delivery-contract.md:44-47`). VERIFIED per their docs. So channel-equipped sessions are **not** woken by this project.
  - Sessions without the channel (desktop app) use `bin/claude-peer-rewake`, a user-level `Stop`/`StopFailure` command hook with `asyncRewake: true` and a timeout of 86400. It waits in the background, polling the metadata file every 3 s. On a new unannounced id it exits 2 with sender names and ids, which wakes Claude even when idle, and Claude then calls `check_messages`. It exits 0 at once if the session already has the channel or runs with `-p` (`README.md:276-289`; `bin/claude-peer-rewake:1-45`). VERIFIED.
- **Factory Droid:**
  - `droidpeer` runs either a long-lived ACP client over `droid exec --output-format acp`, or a native-TUI relay over Factory's internal daemon IPC protocol ("not ACP", "Verified against Droid 0.213.0") (`shared/droid-native-relay.ts:1-3`). VERIFIED.
  - When idle it sends `DROID_WAKE_PROMPT`. Mail that arrives mid-turn waits for the turn to end, and native queued human input also defers the wake (`docs/delivery-contract.md:99-104`). VERIFIED.
  - A pre-existing unmanaged Droid TUI is not wakeable (`README.md:367`). VERIFIED.
- **Hermes:**
  - Legacy mode has no wake ("Bounded polling, never push"). The model must poll (`docs/delivery-contract.md:75`). VERIFIED.
  - The opt-in v2/T2 coordinator calls host RPCs `session.lifecycle_snapshot`, `session.wake_admit` and `session.wake_reconcile`, with busy/queued/compacting lifecycle flags. It is "pinned to Hermes 65d7a5b06a" (`shared/hermes-gui-lifecycle-bridge.ts:1-15`; `shared/hermes-conversation-wake.ts:1-30`). VERIFIED. Whether upstream Hermes exposes those RPCs is unknown. The commit pin and agent-codename citations suggest a private or patched Hermes build (INFERRED).
- **Long-poll fallback:** `wait_for_peer_messages` blocks up to 300 s on Codex and 60 s on Hermes, with an optional `from` filter that never consumes non-matching mail (`docs/delivery-contract.md:57-59,75-76`). VERIFIED.

## Delivery guarantees

- At-least-once, stated honestly. Steps (`docs/delivery-contract.md:6-27`; `docs/ARCHITECTURE.md:99-140`). VERIFIED:
  1. The broker queues the message (`send_message` success means queued, not seen).
  2. The broker leases it to the MCP for 30 s (`broker.ts:27`).
  3. The MCP persists it to the on-disk inbox before presenting it.
  4. The broker ack happens only on confirm.
- Confirm-on-next-call is a `DeliveryState` machine with four transitions: `draw`, `promote`, `rollback`, `markConfirmed`. Its "arrival-causality barrier" snapshots eligibility at request entry, so a call parked in a long wait cannot confirm a later response (`shared/delivery-state.ts`; `AGENTS.md:48-56`). VERIFIED.
- Claude's channel path is the exception: it acks the broker as soon as the message is durably persisted, before or regardless of the push (`claude-server.ts:505-519`). The durable entry is then removed only by `check_messages` confirm-on-next-call (`docs/delivery-contract.md:33-41`). VERIFIED.
- Acks are typed: `acked | expired | unknown | wrong_session`. VERIFIED.
- Ordering is FIFO by id per recipient. There are no threads, groups or priorities; it is 1:1 peer messaging only. INFERRED from the schema and API: the tools are `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages` and `rename_peer` (`codex-server.ts:229-291`).
- Messages to dead peers become "orphans", visible through `cli.ts orphaned-messages`. Unread durable mail has no TTL (`AGENTS.md:72-74`). VERIFIED.
- There is no semantic ACK request or receipt that the model must answer. The colleague protocol actively discourages acknowledgements. VERIFIED (`shared/piggyback.ts:41-47`).

## Human participation

- `bun cli.ts send <name> "<text>"` sends from a shell. There are also status, peers, rename, orphan and inbox-inspection commands (`README.md:240-251`). VERIFIED.
- Humans do not get an inbox or seat of their own; operators read state through the CLI. INFERRED.
- There is no thread model in which a human is a member. INFERRED.

## License and maturity

- License: MIT (`LICENSE`; copyright line "Co-Messi"). VERIFIED.
- GitHub API on 2026-10-23: 0 stars, 0 forks, created 2026-06-19, pushed 2026-10-22, 0 open issues, no releases, about 128 commits. It is a fork of Co-Messi/agent-peers-mcp, which has 8 stars and was last pushed 2026-07-13. VERIFIED.
- `package.json` version 0.1.0, `private: true`. VERIFIED.
- There is a large test suite (around 70 `tests/*.test.ts` files) and CI (`.github/workflows/ci.yml`). VERIFIED that the files exist; the tests were not run.
- It looks like a personal tool: launchd label `com.mike…`, `/Users/mike/...` paths in the README, and beads issue tracking (INFERRED). It is heavily engineered for its size.

## Harness support

| Harness | Status | Notes |
|---|---|---|
| claude | native | MCP server with `claude/channel` push mid-turn (terminal launch with `--dangerously-load-development-channels`), plus `check_messages`. Idle wake only through the `asyncRewake` Stop-hook script for channel-less (desktop) sessions. VERIFIED |
| codex | native | Piggyback `[PEER INBOX]` on tool results. Idle wake and mid-turn steer through a managed `codex app-server` (`turn/start` / `turn/steer`), only for sessions started via `codex-peer` or Desktop app-server threads. A plain `codex` TUI is not wakeable. VERIFIED |
| hermes | native | `hermes-server.ts`, polling only (60 s wait cap). The opt-in v2 per-conversation identity and host-RPC wake are pinned to a specific Hermes commit. Upstream availability of the RPCs is unknown |
| agy | none | No mention in code or docs (grep). VERIFIED absent |
| goose | none | Only appears as a word in the random-name list (`shared/names.ts:20`). VERIFIED absent |
| pi | none | No mention. VERIFIED absent |
| opencode | none | No mention. VERIFIED absent |

The fork also supports Factory Droid (native: ACP or native-daemon relay wake), which is not in our list.

## Notable points for herdr-threads (INFERRED)

- **Codex `turn/steer` through app-server.** It injects into a running turn with `expectedTurnId` fencing. This is a native mid-turn delivery channel for Codex that we do not use, but it only works when Codex is hosted on an app-server the integration owns.
- **Claude `asyncRewake` Stop hook.** It is a documented, harness-native idle wake that types no keystrokes. It may be an alternative to the mod-channel wake for sessions without our mod.
- **Bodyless metadata file as the waker's only input.** The waker never sees bodies and never acks. This matches our R18 idea, implemented as a file-based split.
- **Weak points:**
  - A verbose per-delivery instruction header.
  - No priorities or thresholds: every new message wakes Codex and Droid, with only backoff.
  - Wake is limited to processes launched through its own wrappers.
  - Its own docs (README vs code) disagree on whether busy Codex turns get nudged.
  - Coupled to the Codex history mode (`legacy`) and to a pinned Droid version and Hermes commit.
  - macOS/launchd-only broker ownership.
