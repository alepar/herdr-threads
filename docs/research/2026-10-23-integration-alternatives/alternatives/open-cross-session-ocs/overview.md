# open-cross-session (ocs): overview

- Upstream: https://github.com/leeguooooo/open-cross-session
- Clone read: `scratchpad/alts/open-cross-session-ocs`, HEAD `1a3cee23a` (= tag `v0.9.4`, 2026-10-22). Not executed.
- Paths below are relative to the clone root. VERIFIED = read in source/docs; INFERRED = my reading, not stated.

## What it is

A single-binary CLI (`ocs`, TypeScript compiled with Bun) that gives Claude Code, Codex (ChatGPT Desktop tasks and terminal TUIs), Pi, Hermes, Grok Build and agy a shared append-only message log plus a per-harness "wake carrier" that pushes the message into the target session. It describes itself as a bridge between the harnesses' own native cross-session features ("ocs's Claude carrier literally rides on the native inbox socket"). VERIFIED: `README.md:13`, `README.md:329-372`, `package.json`.

Its main design choice: no hooks and no MCP. Every carrier is a harness-native *input* API (Claude's UDS inbox, `codex queue`, Codex app-server `turn/steer`, ChatGPT Desktop IPC, Pi `pi.sendMessage`, Hermes WebSocket `prompt.submit`, Grok leader IPC), with cmux keystrokes as the last fallback. The message body is the wake payload. VERIFIED: `README.md:175-205`, `DESIGN.md:21-34`. Grepping `src/` for `UserPromptSubmit|PostToolUse|SessionStart|Stop` hooks finds nothing (VERIFIED, only a comment at `src/claude-inject.ts:264`).

## Architecture

| Aspect | Finding | Evidence |
|---|---|---|
| Process model | No resident daemon for local use. Each `ocs send/dm` is a CLI process that appends to the log and then runs the carriers. Short-lived detached helpers handle asynchronous work: `ocs _claude-wake <job>` (receipt listener, lives up to 5 min + 60 s), `ocs _idle-watch <id>` (2 s polling for `notify-when-idle`), and one Codex backlog waiter per (recipient, channel). | VERIFIED `docs/wake-protocol.md:165-170`, `:250-264`, `:110-113`; `src/idle.ts:29-30` |
| Optional daemons | `ocs lan up` (LAN daemon, TCP 47890); `ocs agy start` (agy headless receiver worker). Both are opt-in. | VERIFIED `README.md:376-405`, `docs/agy.md` |
| Storage | `~/.ocs` (`OCS_HOME`): per-channel JSONL logs, `cursors/<channel>.<consumer>.json`, `idle-subs/`, `codex-wakes/<thread>.<channel>.json`, `wake-jobs/`, `workspace-key` (HMAC secret for stable DM channel names), `agy/jobs.sqlite3`. | VERIFIED `README.md:325-327`, `src/store.ts:961-963`, `docs/wake-protocol.md:110`, `docs/agy.md` |
| Ordering | Per-channel monotonic `seq`, derived from the log tail inside an `O_EXCL` lock file. Stale locks are broken by an atomic rename plus an inode check. There is no separate seq file, so a crash cannot reuse a seq. | VERIFIED `src/store.ts:4-16`, `:238-305`, `:313-349` |
| Sidecar frames | Routing and receipts are separate JSONL lines in the same log (`{"v":1,"type":"receipt","seq",...}`), so old readers skip them. | VERIFIED `docs/wake-protocol.md:304-314`, `src/store.ts:139-205` |
| Transport (local) | Filesystem log + harness-native sockets/CLIs (table below). | VERIFIED |
| Cross-machine | Opt-in LAN daemon: Ed25519 machine identity, pairing by copied text + 6-digit SAS, signed X25519 handshake, AES-256-GCM, trust that expires (8 h default, `--once`, `--forever`). Remote address `<addr>@<peer>`. Each side keeps its own log and seq (multi-party channels are not replicated). Terminal receipts are not sent back across machines. Other networks are reached through a VPN or by running the remote `ocs` over ssh. | VERIFIED `README.md:374-451`, `DESIGN.md:52-60`, `docs/wake-protocol.md:316-320` |

## Identity and session model

- Short ids per host: `claude-<8hex>`, `codex-<8hex>` (the full thread UUID is the exact id; the short form can collide), `pi-<uuid>`, `hermes-<key>`, `grok-<uuid>`, plus `ocs rename <name>`. VERIFIED `README.md:53-77`.
- Self-detection reads environment variables the harness sets for child processes: Claude `CLAUDE_CODE_SESSION_ID` + `CLAUDE_CODE_MESSAGING_SOCKET`, cross-checked against `~/.claude/sessions/<pid>.json` (pid, sessionId and socket must all match, else it walks the parent-process chain); Codex `CODEX_THREAD_ID`; Pi `OCS_PI_SESSION_ID` (exported by the ocs Pi extension); Hermes `HERMES_SESSION_ID`; Grok `GROK_SESSION_ID`. VERIFIED `docs/wake-protocol.md:184-188`, `src/roster.ts:62`, `src/deliver.ts:297-302`, `src/hermes.ts:10-14`, `docs/grok.md`.
- Claude DM channels are named from a salted HMAC of a stable workspace identity (normalized git origin, else the launch dir), so a DM survives restarts and worktrees. If several sessions share a workspace, ocs uses per-session identities instead. VERIFIED `README.md:349-367`, `docs/wake-protocol.md:125-130`.
- Presence comes from the harnesses' own state: Claude `~/.claude/sessions/<pid>.json` (`status` busy/idle, `statusUpdatedAt`); Codex is live if some process holds its rollout file open (`lsof -t <rollout>`), busy if the last lifecycle event in the rollout is `task_started` (with no `task_complete`/`turn_aborted` after it); Hermes `session.active_list`. VERIFIED `docs/wake-protocol.md:160-163`, `src/codex-queue.ts:11-16`, `src/codex-defer.ts:165-195`, `src/hermes.ts:339-373`.

## Injection mechanism (general)

- Payload: a "wake note" carrying the body inline, verbatim up to 4096 bytes. A longer body shows its first 512 bytes plus `… (N bytes total; full text: ocs read <ch>)`. The whole note is capped at 5120 bytes and ends with copy-paste `Reply:` and `Thread:` command lines. A fixed anti-chatter line ("No unsolicited acknowledgements") is included in every note. VERIFIED `docs/wake-protocol.md:28-75`, `src/wake.ts:131`.
- Neutralization: inside the body, `<cross-session-message` is rewritten to `‹…`, and lines that start with protocol markers (`Reply:`, `[ocs wake]`, ...) are prefixed with `> `. VERIFIED `docs/wake-protocol.md:46-49`, `src/wake.ts:85-94`.
- Timing: push only, at send time, through a harness-native input queue. Nothing is delivered at turn start or after a tool call (no hooks). A busy target is handled by the harness's own queue (Claude `priority:"next"`, Pi `followUp`, Hermes `queued:true`, Grok `sendNow:false`), by `turn/steer` into the running Codex turn, or by an ocs-side merged backlog. VERIFIED `src/claude-inject.ts:395-405`, `src/pi-extension.ts:171-179`, `src/hermes.ts:16-18`, `DESIGN.md:30`, `src/deliver.ts:148-187`.
- Dedup / ack: no ack protocol; ocs states that `accepted` "is not a read receipt". After an unknown outcome it never resends ("unknown never replays"), and the sender is told not to resend after exit 2/3. Read position is a per-consumer cursor advanced by `ocs read`; `ocs inbox` lists unread threads. A `--peek` read does not advance the cursor, so a Codex merged wake may remind about it again. VERIFIED `README.md:249-281`, `DESIGN.md:34`, `docs/wake-protocol.md:122`, `src/inbox.ts:1-3`.
- Offline: the message stays in the log; "Persistence has no auto-nudge: nothing watches for sessions coming online". VERIFIED `README.md:349-351`.

## Wake mechanism (general)

The wake and the injection are the same act: submitting into the harness's input queue starts a turn if the session is idle. Idle/busy detection is per harness:

- Claude: ocs does not gate on idle. The native inbox enqueues the message, and Claude Code's `onEnqueue` starts a turn when the session is idle. This applies only when the receiver's `crossSessionInbound` is `accept`; the default `hold` parks the message for manual approval and drops it after 5 minutes. VERIFIED `src/claude-inject.ts:31-40`.
- Codex terminal TUI: if `task_started` is the last event (busy), ocs tries `turn/steer` with `expectedTurnId`. If steer is unavailable, it saves a backlog file and coalesces the unread messages into one wake after the turn ends. When the target is idle, ocs runs `codex queue --thread`. VERIFIED `src/deliver.ts:148-208`, `docs/wake-protocol.md:100-123`.
- cmux fallback: `cmux read-screen --lines 8`. If the screen contains `to interrupt`, the target counts as busy and is skipped; otherwise ocs sends `cmux send` with the note, then `send-key enter`. It does not check for a human draft in the composer: a typed draft would be merged into the submission (INFERRED from the code). VERIFIED `src/roster.ts:248-275`.
- `notify-when-idle`: a one-shot subscription. A detached watcher polls the target's Claude session file every 2 s and injects a `[Cross-session idle notice]` into the subscriber's session. It expires after 6 h. VERIFIED `docs/wake-protocol.md:132-170`.

## Delivery guarantees

| Property | ocs | Evidence |
|---|---|---|
| Durability | The log append is committed before any wake; the output says "stored #ch seq n" first. | VERIFIED `README.md:249`, `DESIGN.md:15` |
| Ordering | Per-channel seq; readers order by seq, never by timestamp. | VERIFIED `src/store.ts:4-6` |
| At-most-once wake | Unknown outcomes are never retried; a fallback runs only when delivery definitely failed. | VERIFIED `DESIGN.md:34`, `src/deliver.ts:169-173` |
| Delivery receipt (Claude) | Uses Claude Code's undocumented `peer_message_status` control frames on a per-wake receipt socket in Claude's socket dir: no receipt within 400 ms means `accepted`; otherwise `held` / `delivered` / `expired` / `refused` / `dropped` / `denied`. One notice reaches the sender if a held message ends up not delivered. There are no receipts on Windows. | VERIFIED `docs/wake-protocol.md:204-302` |
| Read receipt / ACK | None. "Any bookkeeping still counts only the peer's reply." | VERIFIED `docs/wake-protocol.md:236-238` |
| Exit codes | 0 accepted/deferred/parked; 2 stored but wake failed or held; 3 stored, outcome unknown. | VERIFIED `README.md:271-281` |

## Human participation

Humans use `ocs send/read/watch` from a shell with `--as <name>`. Channels are multi-party with `@mentions` and `--reply-to <seq>` (which also wakes that seq's author). There is no human-specific presence or priority model. VERIFIED `README.md:231-247`, `docs/wake-protocol.md:89-91`.

## License and maturity

- MIT; three files are vendored from AgentParty (same author), which is now unmaintained. VERIFIED `LICENSE`, `README.md:493-495`, `DESIGN.md:11`.
- GitHub (gh api, 2026-10-23): 26 stars, 5 forks, 5 open issues, created 2026-09-01, last push 2026-10-22, latest release `v0.9.4` (2026-10-22). Tags v0.9.0 through v0.9.4. Bun test suite, about 58 test files. VERIFIED.
- The project depends on several private or undocumented host protocols (Claude UDS inbox frame and receipts, ChatGPT Desktop IPC, Hermes WebSocket, Codex app-server control socket), each of which it calls "may break with host upgrades". VERIFIED `src/claude-inject.ts:42-43`, `src/codex-ipc.ts:10-11`, `src/hermes.ts:22-23`.

## Harness support

| Harness | Status | Injection carrier | Wake / busy handling | Evidence |
|---|---|---|---|---|
| claude | native | Writes a JSONL `{"type":"user","msgV":1,"msg_id",priority:"next",message}` frame to `/tmp/cc-socks/<pid>.sock` (path from `~/.claude/sessions/<pid>.json`), wrapped in `<cross-session-message from=... from-name=...>`; optional auth line with the peerToken from `~/.claude/sessions/<pid>.<sha256>.key`. | The native queue starts a turn when idle; requires `crossSessionInbound: accept` (`ocs doctor --fix` writes it). Receipts through `peer_message_status`. | VERIFIED `src/claude-inject.ts:11-40,279-292,380-420`, `src/claude-settings.ts:1-13` |
| codex | native | Desktop-hosted tasks: ChatGPT Desktop private IPC `$CODEX_HOME/ipc/ipc.sock`, `thread-follower-start-turn` with a `codex_delegation` provenance envelope (needs a second open task as source). Terminal TUI: `codex queue --thread <id> --message`, or app-server `turn/steer` over `$CODEX_HOME/app-server-control/app-server-control.sock` when the target is busy. cmux keystrokes as the last fallback. | Busy = `task_started` is the last rollout event; steer, else coalesced backlog with one wake after the turn; liveness via `lsof` on the rollout. | VERIFIED `src/deliver.ts:120-252`, `src/codex-ipc.ts:12-17,342-351`, `src/codex-queue.ts:1-20`, `src/codex-steer.ts:1-19`, `src/codex-defer.ts:165-195` |
| hermes | native | Hermes host loopback WebSocket `/api/ws?token=` JSON-RPC `prompt.submit {session_id, text, queued:true}`; rendezvous `host-<role>.json` + token, fingerprint-checked. Shown as a user bubble. | `queued:true`: idle starts a turn ("streaming"), busy queues behind the running turn. | VERIFIED `src/hermes.ts:1-23,203,376` |
| agy | generic | No injection into an existing agy TUI. The opt-in `ocs agy start` worker runs `agy -p <note> --output-format json [--conversation <id>]` headless, with one conversation per ocs channel, a serial SQLite job queue, and the final response sent back to the channel. | Not applicable (headless re-launch; jobs keyed by channel/seq and never re-executed after a crash). | VERIFIED `docs/agy.md`, `src/agy-worker.ts:132-142` |
| goose | none | No mention in the source or docs; only generic `ocs send/read` from a shell. | none | VERIFIED (grep `goose`: 0 hits) |
| pi | native | ocs installs a Pi extension that listens on a token-authenticated UDS under `$OCS_HOME/pi-sessions` and calls `pi.sendMessage({customType:"ocs",content:note,display:true},{deliverAs:"followUp",triggerTurn:true})`. | Idle triggers a turn; busy queues as a follow-up after the current turn. | VERIFIED `src/pi-extension.ts:150-200`, `docs/wake-protocol.md:24-26` |
| opencode | none | No mention; generic CLI only. | none | VERIFIED (grep `opencode`: 0 hits) |

Also supported, outside the requested set: Grok Build (`grok --leader` native IPC, `sendNow:false`) and cmux terminal surfaces. VERIFIED `docs/grok.md`, `DESIGN.md:30-32`.

## Notes for comparison with herdr-threads (INFERRED)

- ocs uses no hooks and does no in-turn delivery of its own. Mid-turn arrival depends entirely on each host's queue semantics (Claude `next`, Codex steer). Herdr-threads delivers in-turn through hooks and the mod channel.
- Codex `turn/steer` with `expectedTurnId` on the app-server control socket is a native path for putting a message into a running Codex turn without keystrokes. It deserves evaluation for herdr-threads' busy-Codex case.
- `codex queue --thread` is a public CLI path that wakes an idle terminal Codex TUI without typing into the pane. ocs's own caveat: it exits 0 even if the target is dead, and the message is presented as a user message (no provenance envelope).
- Claude: ocs rides on the native UDS inbox and `peer_message_status` receipts, which are undocumented. They are an alternative to herdr-threads' mod channel for an idle wake and give an honest held/refused signal.
- Codex busy handling: ocs merges messages that arrive while Codex is busy into a single wake after the turn, instead of the host replaying each message as its own turn.
- The cmux fallback checks only for `to interrupt` on screen and has no composer-draft protection.
- ocs has no priority or threshold model: every `@mention`/DM wakes, unless the sender passes `--no-wake`.
