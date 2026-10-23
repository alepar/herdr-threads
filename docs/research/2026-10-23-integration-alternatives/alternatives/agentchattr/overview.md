# agentchattr — overview

- Upstream: https://github.com/bcurts/agentchattr (MIT)
- Clone examined: `scratchpad/alts/agentchattr`, `--depth 1`, HEAD `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10, "Ignore Codex idle input particles when detecting activity"), `VERSION` = 0.5.0.
- Paths below are relative to the clone root. VERIFIED = read in source/docs; INFERRED = reasoning, not read.

## What it is

A local, Slack-like chat server (FastAPI web UI on :8300 plus an MCP server on :8200 HTTP / :8201 SSE) where humans and CLI coding agents share channels. When anyone @mentions an agent, a per-agent wrapper types a fixed prompt ("use mcp to read #channel - you're mentioned, take appropriate action and respond") into that agent's terminal; the agent then pulls the conversation through the `chat_read` MCP tool and replies with `chat_send`. VERIFIED: `README.md:5-7`, `README.md:94-106`, `config.toml` `[server]`/`[mcp]`.

Extras on top of the chat: jobs (threads with status), rules (human-approved shared instructions pushed into wake prompts), roles, sessions (templated multi-agent turn-taking), scheduled messages, decision cards, summaries, pins. VERIFIED: `README.md:115-235`.

## Architecture

| Aspect | Finding | Evidence |
|---|---|---|
| Language / size | Python 3.11+, ~9.9k lines of `.py` at top level, vanilla JS UI in `static/` | VERIFIED: `wc -l *.py`; `README.md:3` |
| Process model | One server process (`run.py`: FastAPI app + two FastMCP servers) and one wrapper process per agent instance (`wrapper.py <agent>`). Launch scripts auto-start the server if port 8300 is not listening. | VERIFIED: `run.py`; `macos-linux/start_claude.sh` (`is_server_running`, then `wrapper.py claude`) |
| Agent hosting | Unix: the wrapper creates a dedicated tmux session `agentchattr-<name>` running the CLI and attaches to it; restarts the CLI 3 s after exit unless `--no-restart`. Windows: the CLI runs in the wrapper's console. | VERIFIED: `wrapper_unix.py:186-280`, `wrapper.py:883-894`, `wrapper_windows.py:410` |
| Storage | Append-only JSONL message log `data/agentchattr_log.jsonl`, fsync per message; in-memory list as the working set. Cursors `data/mcp_cursors.json`, roles `data/roles.json`, schedules, jobs, rules, summaries as JSON files. | VERIFIED: `app.py:270-276`, `store.py:52-93`, `run.py:78-81` |
| Server→wrapper transport | Per-agent queue file `data/<name>_queue.jsonl`: server appends a JSON line per trigger; the wrapper polls every 1 s, reads all lines, truncates the file. | VERIFIED: `agents.py:32-54`, `wrapper.py:473-571` |
| Agent→server transport | MCP over streamable HTTP or SSE, bearer token per instance; heartbeats and registration over loopback HTTP. | VERIFIED: `mcp_bridge.py:139-190`, `wrapper.py:388-406`, `app.py:219-227` |
| Daemon | Yes, a long-lived local server; no system service, launched from a terminal. | VERIFIED: `macos-linux/start_claude.sh` |
| Cross-machine | No. Binds 127.0.0.1 by default; non-loopback needs `--allow-network` plus a warning; registration/heartbeat are loopback-only; the queue file is a shared local directory. Export/import zip moves history between machines offline. | VERIFIED: `run.py:126-142`, `app.py:219-227`, `README.md:204-211` |

## Identity and session model

- The wrapper calls `POST /api/register {base}`; the registry assigns a canonical name (`claude`, `claude-2`, ...), a slot, colour, `identity_id` and a random 16-byte token. VERIFIED: `registry.py:19-31`, `registry.py:186`, `wrapper.py:616-625`.
- Identity is bound to the wrapper/tmux session, not to the harness's session id. The token reaches the CLI via its MCP config (bearer header) or a per-instance localhost MCP proxy that stamps `sender` into every `tools/call` (`mcp_proxy.py:39-49`, `mcp_proxy.py:281-310`). The server resolves the bearer token to the instance and overrides any self-declared sender. VERIFIED: `mcp_bridge.py:158-190`.
- After a harness `/resume`, identity recovery is model-driven: the first wake prompt of a multi-instance agent appends a hint to "reclaim your previous identity from your context window", and the model calls `chat_claim(name=...)`; README admits names can land on the wrong slot if launch order changes. VERIFIED: `wrapper.py:422-427`, `wrapper.py:561-563`, `README.md:177-184`, `mcp_bridge.py:97-102`.
- Presence: wrapper heartbeat every 5 s; offline after 10 s; deregistered after 60 s without heartbeat; 409 on heartbeat triggers re-registration. VERIFIED: `README.md:291-294`, `wrapper.py:763-796`, `mcp_bridge.py:45`.

## Injection mechanism (how content reaches the model)

- Two-step: a content-free-ish wake prompt is typed into the TUI, then the model pulls content itself with the `chat_read` MCP tool. Message bodies are never typed into the terminal (except custom session prompts). VERIFIED: `wrapper.py:526-531`, `wrapper.py:567`.
- The wake prompt is not content-free: it carries the channel or job id, and on some triggers the agent's ROLE and the full active RULES text (on first trigger, rules epoch change, or every N triggers; N from server settings, default 10), plus a one-time identity hint. Newlines are flattened to spaces to avoid Claude Code's "[Pasted text]" collapse. VERIFIED: `wrapper.py:533-567`.
- Session-engine triggers carry a full custom prompt (`prompt` field) that replaces the default wake text. VERIFIED: `agents.py:45-47`, `wrapper.py:519-527`, `app.py:877`.
- `chat_read` (`mcp_bridge.py:575-689`): per-agent, per-channel cursor persisted to disk; first read returns the last `limit` (default 20) messages; later reads return messages after the cursor, then `msgs[-limit:]`, then the cursor moves to the last returned id. Output is a JSON array (id, sender, text, type, time, channel, attachments, reply_to); README estimates ~40 tokens of framing per message. No per-message body truncation in `chat_read` (VERIFIED: `_serialize_messages`, `mcp_bridge.py:409-426`).
- Escalating anti-polling text on empty reads ("No new messages. Do not poll — wait for your next prompt." ... "STOP."). VERIFIED: `mcp_bridge.py:666-679`.
- Behavioural contract lives in a long MCP `instructions` string (identity rules, "always respond in chat", token-aware reading, rules, summaries, jobs). VERIFIED: `mcp_bridge.py:54-124`, `mcp_bridge.py:949-955`.
- No harness hooks are used (no UserPromptSubmit/PostToolUse/Stop/SessionStart, no Claude channels, no Codex app-server). VERIFIED: grep for `hook`, `UserPromptSubmit`, `app-server` finds none outside tests/unrelated.
- No mid-turn delivery: a busy agent only sees new messages if it happens to call `chat_read`. INFERRED from the absence of any hook path.

## Wake mechanism

- Unix: `tmux load-buffer` + `tmux paste-buffer -p -d` (bracketed paste if the app enabled it), sleep `max(inject_delay, len*1ms)` (default 0.3 s, configurable per agent, e.g. qwen 1.0), then `tmux send-keys Enter`. Each tmux call has a 5 s timeout; failure is printed and not retried ("a paste may have arrived"). VERIFIED: `wrapper_unix.py:64-136`, `wrapper.py:908`, `config.toml` `[agents.qwen]`.
- Windows: `WriteConsoleInputW` key records into the agent's console, optional `WM_SETFOCUS` "fake focus" Enter backend for Copilot. VERIFIED: `wrapper_windows.py:134-191`, `config.toml` `[agents.copilot] enter_backend`.
- Idle/busy detection is not consulted before injecting. The queue watcher injects whenever the queue file is non-empty; there is no composer-empty check and no busy check. VERIFIED: `wrapper.py:482-567` (no condition other than `has_trigger`). Consequence, INFERRED: a trigger while the agent is mid-turn is typed into the composer and relies on the harness's own queued-input behaviour; a human's half-typed draft in that pane is submitted together with (or concatenated to) the wake text.
- Activity detection exists but only drives the UI status pill: hash of `tmux capture-pane -p` every second, "active" if it changed; Codex prompt-particle animation filtered out. Windows hashes `ReadConsoleOutputW`. VERIFIED: `wrapper_unix.py:139-183`, `wrapper.py:842-879`, `README.md:166-167`.
- Coalescing: all queued lines read in one poll yield one injection (the last line's channel/job/prompt wins). VERIFIED: `wrapper.py:483-531`.
- Loop guard: per-channel hop counter; agent→agent @mentions stop routing after `max_agent_hops` (default 4) until a human message or `/continue`. Agents never route to themselves. VERIFIED: `router.py:52-88`, `app.py:863-873`, `config.toml` `[routing]`.
- Offline target: trigger is still written to the queue file and a system notice is posted; the wrapper clears its queue file on startup, so triggers queued while the wrapper was down are dropped (the messages remain readable via `chat_read`). VERIFIED: `app.py:895-898`, `wrapper.py:719-721`.
- API (non-CLI) agents: `wrapper_api.py` polls the same queue, reads recent messages over `/api/messages`, calls an OpenAI-compatible `/chat/completions`, posts the reply. VERIFIED: `wrapper_api.py:1-18`, `wrapper_api.py:204-330`.

## Delivery guarantees

| Property | Finding |
|---|---|
| Acknowledgement | None at protocol level. The only "seen" signal is the per-agent read cursor and a rules-epoch sync report. Decision cards give human-click answers, not ACKs. VERIFIED: `mcp_bridge.py:566-572`, `wrapper.py:454-471`. |
| Ordering | Total order by monotonically increasing integer id within the single server. VERIFIED: `store.py:17`, `store.py:61-78`. |
| Durability | Messages fsync'd to JSONL per add. Cursors persisted. Wake triggers are not durable across wrapper restarts (queue cleared on start; read-then-truncate in the watcher can drop a line appended between read and truncate — INFERRED race, `wrapper.py:484-486`). |
| Loss on backlog | If more than `limit` (20) messages arrived since the cursor, `chat_read` returns only the newest 20 and advances the cursor past the skipped ones: older unread messages are silently skipped. VERIFIED: `mcp_bridge.py:655-663`. |
| Priorities / deadlines | None; every @mention wakes. VERIFIED: `router.py`, `agents.py`. |
| De-duplication | Cursor-based; targets deduped per message. VERIFIED: `app.py:861`. |

## Human participation

First-class: the human is the primary user of the web UI (channels, @mention toggles, slash commands, jobs/rules panels, voice typing, scheduled messages, notification sounds). Rules activation/editing and job acceptance are human-only. Human messages reset the loop guard. VERIFIED: `README.md:115-266`, `router.py:57-68`. The human can also type directly into the agent's tmux pane ("type `mcp read #general`"). VERIFIED: `README.md:47`.

## Maturity

| Metric | Value (2026-10-23) |
|---|---|
| Stars / forks | 1558 / 239 (GitHub API) |
| Created | 2026-02-26 |
| Last commit / push | 2026-09-10 |
| Latest release | v0.5.0 (2026-07-26); earlier v0.4.1 (2026-05-22), v0.4.0 (2026-04-19) |
| Open issues | 18 |
| Tests | unit tests in `tests/` (injection transport, router, identity, cursors, schedules, ...) |
| License | MIT (`LICENSE`) |

## Harness support

| Harness | Status | How |
|---|---|---|
| claude (Claude Code) | native | Built-in default: per-instance `--mcp-config` file with bearer token, merges project `.mcp.json`; tmux paste + Enter wake; strips `CLAUDECODE` env. VERIFIED: `wrapper.py:131-137`, `wrapper.py:289-298`, `wrapper.py:723`, `macos-linux/start_claude.sh`. |
| codex | native | Built-in default `proxy_flag`: `-c mcp_servers.agentchattr.url="<per-instance proxy>"`; tmux paste wake; Codex-specific activity filter. VERIFIED: `wrapper.py:144-149`, `wrapper_unix.py:139-156`, `macos-linux/start_codex.sh`. |
| hermes | none | No mention in repo. Could be added via `[agents.x] command=...` + an `mcp_inject` mode if Hermes accepts an MCP config that way (unknown). INFERRED. |
| agy (Antigravity) | native | `[agents.antigravity]`: writes/merges `~/.gemini/config/mcp_config.json` with `serverURL` key (a fixed real-user path; "verified against agy 1.1.7"); launcher sets `AGY_CLI_DISABLE_AUTO_UPDATE=1`; tmux wake. VERIFIED: `config.toml` `[agents.antigravity]`, `macos-linux/start_antigravity.sh:2-18`. |
| goose | generic | Not shipped. The tmux wake is CLI-agnostic and config supports `settings_file`/`env`/`flag`/`env_content` MCP injection, so a user-written `[agents.goose]` entry could work if goose's MCP config fits one of those modes (unknown). INFERRED from `wrapper.py:163`, `config.toml` comments. |
| pi | generic (weak) | Not shipped. tmux wake would type the prompt, but the read/reply path requires MCP tools; pi has no built-in MCP to our knowledge, so without an extension it could not call `chat_read`. INFERRED. |
| opencode | generic | Not shipped by name. Kilo CLI is supported via `KILO_CONFIG_CONTENT` env JSON (`mcp.agentchattr.type=remote`); Kilo is an OpenCode derivative and OpenCode accepts the analogous `OPENCODE_CONFIG_CONTENT` (INFERRED, not in repo). VERIFIED for Kilo: `wrapper.py:156-160`, `wrapper.py:300-310`. |

Other shipped harnesses: Gemini CLI, Kimi, Qwen, Kilo, CodeBuddy, GitHub Copilot CLI, MiniMax (API). VERIFIED: `README.md:5`, `config.toml`.

## Notes relevant to herdr-threads

- Same core wake technique as our `Native` policy (type into the composer), but with no idle gate and no draft protection — a weaker contract than our rule 4. VERIFIED absence: `wrapper.py:482-567`.
- Their bracketed-paste-then-Enter detail (`wrapper_unix.py:64-84`) documents a real Claude Code failure (long `send-keys -l` split across pty reads, only the tail submitted). Worth checking against Herdr's typed wake if a wake text ever grows. INFERRED relevance.
- Wake prompt is used as a carrier for rules/role text, contrary to our R18 content-free wake prompts; content itself is pulled via MCP rather than pushed by hooks, so a busy agent gets nothing in-turn.
- The backlog-skip in `chat_read` (`mcp_bridge.py:655-663`) is a correctness flaw we avoid with paged inbox + continuation.
- Writes into real user config for some harnesses (`~/.gemini/config/mcp_config.json`, `~/.gemini/trustedFolders.json`, `~/.codebuddy/.mcp.json`, `~/.copilot/mcp-config.json`). VERIFIED: `config.toml`, `wrapper.py:322-353`.
