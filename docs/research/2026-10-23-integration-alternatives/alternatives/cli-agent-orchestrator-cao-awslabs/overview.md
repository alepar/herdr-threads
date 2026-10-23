# CLI Agent Orchestrator (CAO, awslabs): overview

- Upstream: https://github.com/awslabs/cli-agent-orchestrator
- Clone read: depth-1 clone at commit `69909e98035905b612fd6b117abff8bf8876220f` (2026-10-23 18:49 +1100). All paths below are relative to that clone root.
- Docs site: https://awslabs.github.io/cli-agent-orchestrator/ (not fetched; repo `docs/` read instead).
- Evidence tags: VERIFIED = read in source/docs at the cited line; INFERRED = my reading, not stated.

## What it is

A supervisor/worker orchestrator for interactive coding CLIs. A local `cao-server` (FastAPI) launches provider CLIs (Kiro default, Claude Code, Codex, Antigravity, Hermes, Kimi, MiniMax, Copilot, OpenCode, Oh My Pi, Cursor, Grok) in tmux (default) or herdr panes, and gives agents an MCP server (`cao-mcp-server`) with `handoff` (blocking), `assign` (async) and `send_message` (inbox) tools. VERIFIED: `README.md:29-57`, `CODEBASE.md:9-19`, `src/cli_agent_orchestrator/mcp_server/server.py:333,643,926`.

It is much larger than a messaging layer: workflows, flows/scheduling, memory, self-learning, skills, Web UI, MCP Apps, OTel, Kubernetes deployment (VERIFIED: `README.md:125-172`). Messaging is a means to the supervisor/worker pattern, not the product.

Notably, CAO already supports **Herdr as a terminal backend** (experimental), consuming Herdr's `pane.agent_status_changed` events. VERIFIED: `docs/herdr.md:1-17,97-109`.

## Architecture

| Aspect | Finding | Evidence |
|---|---|---|
| Process model | One long-running `cao-server` (FastAPI, port 9889 default) per host; one `cao-mcp-server` stdio subprocess per agent CLI; agent CLIs run as full interactive TUIs in tmux windows or herdr tabs | VERIFIED `README.md:97-112`, `CODEBASE.md:9-19,62-76` |
| Storage | SQLite via SQLAlchemy under `~/.aws/cli-agent-orchestrator` (override `CAO_HOME_DIR`, dir mode 0700). `inbox` table: id, sender_id, receiver_id, message, status, created_at | VERIFIED `src/cli_agent_orchestrator/constants.py:135-157`, `src/cli_agent_orchestrator/clients/database.py:276-286` |
| Transport (agent to server) | MCP tool in the agent's process does an HTTP `POST {API_BASE_URL}/terminals/{receiver}/inbox/messages?sender_id=..&message=..` (bearer when auth enabled) | VERIFIED `src/cli_agent_orchestrator/utils/orchestration.py:1391-1453` |
| Transport (server to agent) | Keystrokes into the pane: tmux `load-buffer` + `paste-buffer` (bracketed), delay, `send-keys Enter`; herdr `pane send-text` (optionally wrapped in `ESC[200~..ESC[201~`) + `pane send-keys Enter` | VERIFIED `src/cli_agent_orchestrator/clients/tmux.py:1695-1860`, `src/cli_agent_orchestrator/backends/herdr_backend.py:654-720`, `docs/inbox-delivery.md:5-8` |
| Status detection | tmux: `pipe-pane` log/FIFO + per-provider regexes over terminal output (spinners, prompt glyphs, footers). herdr: `herdr pane get` `agent_status` (working/blocked/done/idle/unknown) mapped to PROCESSING/WAITING_USER_ANSWER/COMPLETED/IDLE, plus socket events | VERIFIED `src/cli_agent_orchestrator/providers/claude_code.py:57-198`, `src/cli_agent_orchestrator/backends/herdr_backend.py:866-912`, `src/cli_agent_orchestrator/providers/base.py:541-647` |
| Daemon | Yes, `cao-server`, must be running (user starts it in a terminal) | VERIFIED `README.md:97-101` |
| Cross-machine | Yes, optional: `target_host` places a worker on a remote CAO node; results route back via `CAO_CALLBACK_URL` / `CAO_CALLBACK_TERMINAL_ID` (requires `CAO_ADVERTISED_URL`); elastic workers via a broker gateway; EKS example | VERIFIED `src/cli_agent_orchestrator/utils/orchestration.py:1391-1402,1440-1452`, `server.py:557-640` (assign description), `constants.py:502`, `README.md:146-147` |

## Identity and session model

- Identity is the **terminal id**: 8 lowercase hex chars, injected into every agent process as env `CAO_TERMINAL_ID`; the MCP subprocess inherits it and uses it as `sender_id`. Malformed values are a hard error. VERIFIED `src/cli_agent_orchestrator/utils/orchestration.py:217-235`.
- Sessions map to tmux sessions or herdr workspaces; terminals to windows/tabs. VERIFIED `docs/herdr.md:88-95`.
- Topology is a tree: `handoff`/`assign` record `caller_id` on the worker row; `send_message` without `receiver_id` routes to the recorded caller. An explicit receiver must be the caller's child, parent, or (with the `discovery` grant) a listed sibling; self-send refused. VERIFIED `src/cli_agent_orchestrator/mcp_server/server.py:926-958`, `src/cli_agent_orchestrator/utils/orchestration.py:2442-2520`.
- No persistent identity across terminal lifetimes beyond the DB row; workers are typically torn down after a handoff (`/exit`). Claude Code can be launched with `--resume <session id>`. VERIFIED `server.py:383-391` (handoff docstring, step 6), `src/cli_agent_orchestrator/providers/claude_code.py:461`. INFERRED: no notion of a durable "seat" that survives a CLI restart the way herdr-threads seats do.

## Injection mechanism (general)

1. Sender agent calls MCP `send_message` (or `assign`); the server persists a PENDING inbox row. VERIFIED `orchestration.py:1391-1453`.
2. Delivery types the message **as user input** into the receiver's TUI composer (bracketed paste then Enter). The model sees it as a new user turn; there is no hook, no MCP resource/notification, no system-prompt edit for inbox delivery. VERIFIED `docs/inbox-delivery.md:5-8,23`; `src/cli_agent_orchestrator/services/inbox_service.py:283-295` (dispatch_input).
3. Framing: a plain-text suffix appended to the body, not a header: `[Message from terminal {id}. Use send_message MCP tool for any follow-up work.]` or for assign `[Assigned by terminal {id}. When done, send results back to terminal {id} using send_message]`. Suppressed for kiro-cli `@prompt` invocations. Enabled by default (`CAO_ENABLE_SENDER_ID_INJECTION`). Codex handoffs additionally get a `[CAO Handoff] Supervisor terminal ID: ...` prose banner telling it not to call send_message. VERIFIED `orchestration.py:125-138,183-207,1183-1212`.
4. When: by default only when the receiver is IDLE or COMPLETED. With `CAO_EAGER_INBOX_DELIVERY=true` and a provider flag `accepts_input_while_processing` (only Claude Code today), also mid-turn while PROCESSING, relying on the TUI to buffer the paste as the next queued prompt. Never into an open dialog for providers whose picker would eat the paste (`blocks_orchestrated_input_while_waiting_user_answer`). VERIFIED `docs/inbox-delivery.md:17-19,41-75`, `inbox_service.py:218-248`, `claude_code.py:1470-1474`.
5. Batching: default one message per IDLE cycle (`num_messages=1`); a drain mode joins up to 100 pending messages with `\n`, grouped by contiguous sender. Ordering oldest first. VERIFIED `inbox_service.py:160-167,213-214,259-266`, `server.py:933-936` ("Messages are delivered in order (oldest first)").
6. Bounding/truncation of message bodies: unknown (no limit found on the inbox POST in the lines read).
7. Handoff result return is not a message: the server waits for COMPLETED and **scrapes the worker's last response from the terminal buffer** (`extract_last_message_from_script`), then tears the terminal down. VERIFIED `server.py:383-391`, `src/cli_agent_orchestrator/providers/base.py:448-458`.

## Wake mechanism (general)

Wake and injection are the same act: pasting the message plus Enter into an idle composer submits a new turn. Idle detection drives when it happens. VERIFIED `docs/inbox-delivery.md:10-15`.

Delivery triggers (all converge on `check_and_send_pending_messages()` / `deliver_pending`):

| Trigger | Detail | Evidence |
|---|---|---|
| Immediate | API attempts delivery right after persisting; POST returns once paste reached the pane (bounded 5 s) | VERIFIED `docs/inbox-delivery.md:12,37` |
| Status events | `InboxService.run` subscribes to `terminal.*.status`; IDLE/COMPLETED spawns a per-terminal delivery thread; bursts coalesced | VERIFIED `inbox_service.py:90-158` |
| Watchdog (tmux) | `PollingObserver` every 5 s on terminal log files; fast idle-regex precheck | VERIFIED `docs/inbox-delivery.md:13,77-79` |
| herdr events | `HerdrInboxService` subscribes to `pane.agent_status_changed`; delivers on `idle`/`done` | VERIFIED `docs/herdr.md:97-99` |
| OpenCode poller | provider-specific 5 s poll because OpenCode logs stop changing once settled | VERIFIED `docs/inbox-delivery.md:126-128`, `docs/opencode-cli.md:3` |
| Reconcile sweep | every 30 s, re-attempts rows PENDING > 30 s | VERIFIED `docs/inbox-delivery.md:111-124` |

Busy/idle: per-provider screen regexes on tmux; herdr native `agent_status` on herdr, with a 10 s "flush wait" before treating done/idle-after-dispatch as COMPLETED and a 300 s give-up. VERIFIED `base.py:541-647`.

Draft clobbering: no check for a human's typed-but-unsent composer text was found (grep for "draft" in `src/` finds nothing relevant). Before pasting, tmux copy-mode is cancelled (`send-keys -X cancel`) and the guard refuses if the pane's foreground process is back to the pre-launch shell or the status is ERROR. VERIFIED `clients/tmux.py:1769-1777`, `src/cli_agent_orchestrator/services/terminal_service.py:4489-4524`. INFERRED: an operator attached to a worker pane mid-typing would get the inbox paste merged into their draft and submitted; CAO assumes panes are machine-owned.

Submit confirmation (recent, #78): after paste, waits up to 8 s for a status newer than the dispatch boundary; if absent, sends a bare Enter when the provider can read its composer and the text is still there (codex, claude_code, kiro_cli), re-sends in full only when the composer is readable and empty, else logs the drop. Up to 2 recoveries. Skipped on herdr backend. VERIFIED `docs/inbox-delivery.md:21-39`, `inbox_service.py:300-337`, `CHANGELOG.md:10-19`.

## Delivery guarantees

- States: PENDING, DELIVERED, FAILED only. VERIFIED `src/cli_agent_orchestrator/models/inbox.py:17-22`.
- DELIVERED means "typed into the pane", set **before** the paste to prevent double delivery via re-entrant status events; transient errors (pane unresolvable, dialog open) revert to PENDING, others go to FAILED. VERIFIED `inbox_service.py:250-257,338-379`.
- No receiver acknowledgement: `after_dispatch` "is never an acknowledgement that the receiver accepted the message". Eager mid-turn deliveries that are lost if the agent errors are accepted as DELIVERED ("acceptable for v1"). VERIFIED `inbox_service.py:180-186`, `docs/inbox-delivery.md:105-107`.
- Ordering: oldest first per receiver; per-terminal lock serializes read-mark-send because the DB has no atomic claim. Known duplicate-wakeup race across paths, tracked in GH #115. VERIFIED `inbox_service.py:68-80`, `docs/inbox-delivery.md:128`.
- Durability: SQLite rows survive server restart; on restart, ghost terminal records are cleaned (herdr). VERIFIED `docs/herdr.md:101-109`. INFERRED: messages to a terminal whose CLI died are stranded/FAILED; no re-binding to a new process.
- No priorities, deadlines, threads or ACK requests in the inbox model. VERIFIED by the 6-column schema, `clients/database.py:276-286`.

## Human participation

- Operator watches via `tmux attach`, `herdr attach`, Web UI at `http://localhost:9889` (PTY WebSocket), or ops MCP server; can type directly into panes. VERIFIED `README.md:111-113,127-131`, `CODEBASE.md:14-18`.
- `answer_user_prompt` tool answers a provider's open dialog on a human's behalf through a separate guarded path. VERIFIED `server.py:1051-1052`, `docs/inbox-delivery.md:35`.
- Humans are not addressable inbox participants (sender defaults to "operator"/"supervisor" labels; no human inbox). VERIFIED `server.py:151,239`, `src/cli_agent_orchestrator/mcp_server/app_tools.py:446-449`. INFERRED from the schema.

## License and maturity

- License: Apache-2.0. VERIFIED `LICENSE:1-3`, GitHub API.
- Stars 1465, forks 300, created 2025-07-29, last push 2026-10-23 (GitHub API, fetched 2026-10-23). Latest release v2.5.3 (2026-10-08); `pyproject.toml:3` already says 2.5.4. Very active, large test suite, heavy issue-driven hardening (#78, #896, #1018 etc. in comments). VERIFIED.
- herdr backend labeled "Experimental". VERIFIED `docs/herdr.md:15`.

## Harness support

| Harness | Status | Notes |
|---|---|---|
| claude | native | `providers/claude_code.py`; launch `claude --dangerously-skip-permissions` or `--permission-mode`, `--append-system-prompt-file`, `--mcp-config ... --strict-mcp-config`, optional `--resume`. Screen-regex status; composer readable; only provider with eager mid-turn delivery. No Claude hooks used for delivery. VERIFIED `claude_code.py:450-562,1470-1474` |
| codex | native | `providers/codex.py` (~3k lines of TUI parsing); `codex --yolo --no-alt-screen --disable shell_snapshot` or `--profile`; regex status; composer readable; `[CAO Handoff]` banner. No app-server or hooks. VERIFIED `codex.py:35-260,1599-1640`, `orchestration.py:1183-1212` |
| hermes | native | `providers/hermes.py`; `<profile> chat --yolo --accept-hooks --source cao`; regexes overridable by env; MCP server must be added to the Hermes profile manually; blocks input on pickers. VERIFIED `hermes.py:19-48,143-191`, `docs/hermes.md:72,97` |
| agy | native (tmux only) | `providers/antigravity_cli.py`; `agy --dangerously-skip-permissions`, `-i` first-message system prompt; registers MCP servers per launch. herdr backend explicitly not supported. VERIFIED `antigravity_cli.py:328-373`, `docs/antigravity-cli.md:24,43` |
| goose | none | Not a provider; Goose appears only as an MCP Apps host for CAO's fleet UI. VERIFIED `README.zh-CN.md:233`, provider list `src/cli_agent_orchestrator/providers/` |
| pi | generic | No `pi` provider. Oh My Pi (`omp`, https://github.com/can1357/oh-my-pi) is supported with `--append-system-prompt` and `--extension` (MCP via extension dir). VERIFIED `providers/omp.py:172-205`, `docs/omp-cli.md:1-3`. INFERRED: OMP is a pi-derived fork; plain pi itself is unknown/unsupported |
| opencode | native (experimental) | `providers/opencode_cli.py`; footer regexes; dedicated 5 s inbox poller workaround (#203). VERIFIED `opencode_cli.py:40-64`, `docs/opencode-cli.md:3` |

## Takeaways for herdr-threads (INFERRED)

- CAO is the clearest example of "keystroke injection as both inject and wake": every delivery is a new user turn, which is the opposite of our delivery contract (rules 2-4: wake only by priority threshold, in-turn delivery via hooks/channels). It also spends a full turn per message.
- Their submit-confirmation (#78) and dialog guard (#896) are hard-won lessons on paste+Enter fragility that apply to our Native wake marker path: confirm a post-dispatch status transition, read the composer before re-sending, never press Enter into a picker.
- Their herdr backend consumes Herdr's `agent_status` directly; the idle-vs-done ambiguity they patch with `_task_dispatched` and a 10 s flush wait is worth comparing with how we read Herdr state.
- Result return by scraping the TUI's last message is fragile (per-provider extraction regexes); our explicit message model avoids that.
