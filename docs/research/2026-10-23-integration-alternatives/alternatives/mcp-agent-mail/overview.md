# mcp_agent_mail: overview

- Upstream: https://github.com/Dicklesworthstone/mcp_agent_mail (Python, cloned at `83ed8d7c97af67f1e6b4700bb8ace08739171ed3`, commit date 2026-10-23)
- Successor: https://github.com/Dicklesworthstone/mcp_agent_mail_rust (Rust). Only its README was read (raw `main`, 2026-10-23); it was not cloned.
- Paths below are relative to the Python clone root unless they are prefixed `rust:README.md`.

## What it is

It is a mail-like coordination layer for coding agents ("gmail for your coding agents"). It runs as an MCP server and gives agents memorable names (adjective+noun, e.g. `GreenCastle`), inboxes and outboxes, threads, search and summaries. It also provides advisory file-reservation leases with an optional git pre-commit guard, and a web UI where a human can read mail and post as an overseer. VERIFIED `README.md:1-31`, `README.md:311-319`.

The Python README now says: "Superseded by mcp_agent_mail_rust. This Python version is no longer maintained; use the Rust version for new installs." VERIFIED `README.md:3`. Even so, the Python repo still receives commits: it was pushed 2026-10-23 per the GitHub API, and the PostToolUse hook rework is recent (GH #273, #274 are cited in `scripts/hooks/check_inbox.sh:92-94`).

It belongs to a larger stack: Beads (`br`), a task tracker, and `bv`, a task viewer. The README also advertises a commercial "Companion" iOS app and host automation with "Message Stacks" that "fan those instructions out to every enrolled agent - without touching tmux". VERIFIED `README.md:47-52`, `README.md:329-333`. The Companion is closed source, so how it injects messages is unknown.

## Architecture

| Aspect | Finding | Status |
|---|---|---|
| Process model | One long-lived server process. `serve-http` is the default: FastMCP over Streamable HTTP, default `127.0.0.1:8765`, no SSE, WebSockets disabled. `serve-stdio` is also available for direct CLI integration. | VERIFIED `src/mcp_agent_mail/cli.py:1202-1246`, `README.md:313` |
| Storage | Dual persistence. Every canonical message plus per-recipient inbox/outbox copies are Markdown files in a per-project Git repo (`messages/YYYY/MM/<id>.md`, `agents/mailboxes/...`, `file_reservations/<sha1>.json`, attachments as webp). SQLite + FTS5 handles indexing, search, directory and leases. Non-SQLite `DATABASE_URL` is rejected. | VERIFIED `README.md:314-317`, `README.md:335-361`, `CHANGELOG.md` v0.3.2 |
| Transport to agents | MCP tool calls (`fetch_inbox`, `send_message`, `acknowledge_message`, macros) and MCP resources (`resource://inbox/{agent}`). Hooks reach the same JSON-RPC endpoint directly with `curl` (`tools/call fetch_inbox`). | VERIFIED `src/mcp_agent_mail/app.py:9446`, `scripts/hooks/check_inbox.sh:103-110` |
| Daemon | Yes: the HTTP server, with background workers such as the ACK-TTL scanner. | VERIFIED `src/mcp_agent_mail/http.py:1195-1251` |
| Cross-machine | The server is reachable over HTTP with bearer/JWT auth, and Docker/compose deploy samples exist. Projects are namespaced by `project_key` (an absolute path). "Products" link projects for cross-project inbox and search. Static export bundles can be signed with Ed25519 and encrypted with age for sharing. Nothing federates between servers. | VERIFIED `README.md:669-700`, `src/mcp_agent_mail/app.py:12395-12773`; "no federation" is INFERRED (no peer protocol found) |

## Identity and session model

- **Registration:** an agent calls `register_agent` / `create_agent_identity` / `macro_start_session` with `project_key`, `program` and `model`. The server assigns or accepts a name and issues a per-agent `registration_token`. That token is required for `fetch_inbox` when no authenticated MCP session exists, which is always the case for hook curls. VERIFIED `src/mcp_agent_mail/app.py:6116-6255`, `9865-9900`, `scripts/hooks/check_inbox.sh:23-28`.
- **Hooks carry a fixed identity:** each hook bakes in `AGENT_MAIL_PROJECT`, `AGENT_MAIL_AGENT` and the tokens at install time (written into `.claude/settings.local.json` or a gitignored wrapper). So one installed project is in effect one agent identity per hook config. A second agent in the same project and harness gets someone else's reminders unless it is reconfigured. VERIFIED `scripts/integrate_claude_code.sh:180-212`, `scripts/integrate_codex_cli.sh:200-228`. The "wrong reminders" consequence is INFERRED.
- **Pane identity:** `WindowIdentity` ties a `window_uuid` (a tmux or terminal window) to a persistent display name per project, with expiry. The canonical file is `~/.local/state/agent-mail/identity/<sha256(project)[:12]>/<$TMUX_PANE>`, containing the name and an epoch, written atomically at mode 600 and treated as stale after 24 h. VERIFIED `src/mcp_agent_mail/models.py:161-182`, `docs/IDENTITY_CONTRACT.md`, `scripts/identity-resolve.sh`. The key is tmux-specific (`$TMUX_PANE`).
- **Contact policy per agent:** `open`, `auto` (the default: allowed when there is shared thread/reservation/recent-contact context, otherwise a contact request is required), `contacts_only` or `block_all`. Overseer messages bypass it. VERIFIED `README.md:1725-1740`, `README.md:441-452`.
- **Rust successor:** optional `MESSAGING_SESSION_IDENTITY` binds identities to the MCP session (the stdio process or the `Mcp-Session-Id` header) and refuses "borrowed names" (`SESSION_IDENTITY_MISMATCH`). The README says it explicitly "does not authenticate the model, provider or OS process". VERIFIED `rust:README.md:615-656`.

## Injection mechanism (general)

Injection is pull-based. The model must call `fetch_inbox`, or read `resource://inbox/...`, to see message content. The only push into context is a count-only reminder emitted by hooks:

1. **PostToolUse(Bash) reminder** (`scripts/hooks/check_inbox.sh`, VERIFIED):
   - Rate limit: at most once per `AGENT_MAIL_INTERVAL` (default 120 s), tracked in `/tmp/mcp-mail-check-<agent>` (lines 66-80).
   - The hook curls `fetch_inbox` with `unread_only=true, include_bodies=false, limit=10` (lines 95-110). It counts messages and counts those with importance high/urgent (lines 128-158).
   - It emits `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"You have N unread message(s)... Use fetch_inbox to read them; reading or acknowledging a message stops this reminder."}}` (lines 160-172).
   - It is silent when there is no mail. Every failure is silently `exit 0`, including no config, placeholder values, curl failure and `isError`.
   - Bodies are never injected, so the model then spends a tool round trip on `fetch_inbox`.
   - The count is capped at 10 by `limit`, so with more than 10 messages the reminder still says "10". INFERRED from `limit:10`.
2. **SessionStart (Claude, Gemini):** runs `mcp_agent_mail.cli file_reservations active` and `acks pending <agent> --limit 20` (plain stdout). VERIFIED `scripts/integrate_claude_code.sh:197-205`, `scripts/integrate_gemini_cli.sh:284-287`. Claude Code adds SessionStart stdout to context, so pending ACKs surface at session start. That last point is INFERRED from Claude Code hook semantics, not from this repo.
3. **PreToolUse(Edit):** `file_reservations soon` warns about leases about to expire.
4. **PostToolUse on `send_message`:** runs `list-acks`.
5. **PostToolUse on `file_reservation_paths`:** runs `file_reservations list`.
   - Items 3-5 emit plain stdout. The repo's own comment says plain PostToolUse stdout "is shown to the human in the terminal but does NOT reach the agent" (`check_inbox.sh:30-36`). These hooks are therefore likely invisible to the model. VERIFIED that they are plain stdout (`scripts/integrate_claude_code.sh:207-221`). The visibility consequence is INFERRED.
6. **Instructions blurb:** a ready-made AGENTS.md/CLAUDE.md block tells agents to register, check the inbox and ACK (`README.md:153-188`). There is also a `SKILL.md`. VERIFIED.
7. **Overseer framing:** a fixed preamble with emoji and a "pause current work, prioritize this, resume afterward" instruction. It is prepended into the message body itself, so it is paid per message and is model-visible prose. VERIFIED `README.md:457-479`.

**Codex history:** the old `notify` program route (`scripts/hooks/codex_notify.sh`) was abandoned because "Codex CLI spawns the notify program with stdin, stdout and stderr redirected to /dev/null and never shows or feeds its output to the model" (GH #273). Codex now uses the same PostToolUse `additionalContext` envelope via project-local `.codex/hooks.json`, which needs Codex trust of the project `.codex/` layer and per-hook hash review. VERIFIED `scripts/hooks/codex_notify.sh:1-10`, `scripts/integrate_codex_cli.sh:187-248`.

## Wake mechanism (general)

The OSS server has no wake for an idle agent. VERIFIED by absence: none of `tmux`, `send-keys`, `wake`, MCP `notifications/` or resource subscriptions occur in `src/` apart from the `WindowIdentity` docstring and internal asyncio wakeups.

- **Signal files:** when `NOTIFICATIONS_ENABLED=true` (default false), `send_message` writes `{signals_dir}/projects/<slug>/agents/<agent>.signal` with id, from, subject and importance metadata. It debounces at 100 ms, skips bcc recipients, and clears the file on `fetch_inbox`. The docstring says agents "can watch these files using inotify/FSEvents/kqueue". No consumer ships in the repo: no watcher, no keystroke injector, no hook reads it. VERIFIED `src/mcp_agent_mail/config.py:160-180`, `config.py:515-518`, `src/mcp_agent_mail/storage.py:3616-3720`, `src/mcp_agent_mail/app.py:5826-5850`, `app.py:9548-9550`. "No consumer" was VERIFIED by grep (only `config.py`, `share.py`, `storage.py` and `cli.py` mention `.signal`).
- **Hooks fire only inside a turn** (PostToolUse after Bash), so an agent idle at the prompt gets nothing until a human or something else starts a turn. INFERRED from the hook events chosen.
- **Wake is outsourced to humans and to proprietary tooling:**
  - The README refers to a "historical zsh loop from the tweet thread" that broadcasts prompts into every Codex pane via tmux. It is not in the repo, and the README calls it "still handy if you are running the OSS stack by itself".
  - The commercial Companion "Message Stacks" is the supported automated cadence.
  - VERIFIED `README.md:47-52`, `README.md:329-333`. How either works is unknown: not public.
- **No idle/busy detection and no draft protection exist in the OSS code**, because nothing types into panes. VERIFIED by absence.

## Delivery guarantees

| Property | Finding | Status |
|---|---|---|
| Durability | Committed to SQLite and a Git-backed Markdown archive. On failure, `send_message` rolls back orphan rows. | VERIFIED `app.py:5800-5826` |
| ACK | Per-message `ack_required` flag. `acknowledge_message` sets `read_ts` and `ack_ts` and is idempotent. `mark_message_read` is separate. `fetch_inbox` does NOT mark mail read. | VERIFIED `app.py:9467-9480`, `app.py:9776-9800` |
| Overdue ACK | Optional `ACK_TTL_ENABLED` (default false, 1800 s) background scan that only logs `ack-warning`. Optional escalation mode `log` or `file_reservation`. The sender gets no notification. | VERIFIED `config.py:207-215`, `config.py:550-552`, `http.py:1195-1251` |
| Ordering | Inbox is `ORDER BY created_ts DESC` (newest first) with a limit (default 20, capped at 1000). Importance (`normal`/`high`/`urgent`) is a field and an `urgent_only` filter; it does not affect sort order. | VERIFIED `app.py:4628-4629`, `app.py:9456-9516` |
| De-dup | Reminders stop once messages are read or ACKed (`unread_only=true`, GH #274). There is also a 120 s rate limit. Nothing de-duplicates re-reads of the same body. | VERIFIED `check_inbox.sh:90-100` |
| Bounding | `limit` on `fetch_inbox`, and bodies are opt-in (`include_bodies=false` default). No body truncation was found. | VERIFIED for limit/bodies; truncation is unknown |
| Priority-based wake | None. Importance only changes the reminder wording ("N urgent/high priority"). | VERIFIED `check_inbox.sh:161-165` |

## Human participation

- Web UI at `/mail` covers the unified inbox, threads, search, reservations, an archive time-travel view and a network graph. VERIFIED `README.md:363-432`, `src/mcp_agent_mail/templates/`.
- Overseer composer at `/mail/{project}/overseer/compose`:
  - Sends as a per-project `HumanOverseer` agent (program `WebUI`, model `Human`).
  - Messages are always `high` importance, bypass contact policy, and get the "pause and prioritize" preamble.
  - It does not wake the recipient: agents see the message only "when agents check their inbox". VERIFIED `README.md:435-552`.
- The Rust successor adds an interactive TUI (`am`) with toasts. VERIFIED `rust:README.md:1204-1213`.

## License

"MIT License (with OpenAI/Anthropic Rider)". The rider names OpenAI, Anthropic, their affiliates and anyone acting on their behalf as "Restricted Parties" and restricts their use. It controls over the MIT text, propagates to derivative works, and terminates on breach. The license is therefore not OSI-open, and GitHub reports it as `NOASSERTION`. The Rust successor uses the same MIT+Rider. VERIFIED `LICENSE:1-46`, `rust:README.md:9`. Consequence for us: do not copy code. Studying ideas is fine; borrowing implementation is not. INFERRED, not legal advice.

## Maturity

| | Python | Rust |
|---|---|---|
| Stars | 2222 (232 forks, 4 open issues) | 210 |
| Created | 2025-10-23 | 2026-02-05 |
| Last push | 2026-10-23 | 2026-10-23 |
| Releases | v0.3.2 (2026-04-16) latest GitHub release; v0.3.0, v0.2.1 (Jan 2026); `pyproject` says 0.3.4 | unknown |
| Status | "no longer maintained" per README, yet still committed to | active, recommended |

Source: GitHub REST API, fetched 2026-10-23. VERIFIED.

The Python server is a very large single module (`app.py` is 14,670 lines, about 35.7k lines in total under `src/`). VERIFIED with `wc`.

## Harness support

"Hook" here means that `scripts/integrate_*.sh` installs harness hooks. Every harness that speaks MCP can use the tools themselves.

| Harness | Status | How | Evidence |
|---|---|---|---|
| claude | native | MCP HTTP config plus `.claude/settings.json` hooks. Hooks installed: SessionStart (active reservations, pending ACKs); PreToolUse(Edit) (leases expiring soon); PostToolUse(Bash) (rate-limited unread-count `additionalContext`); PostToolUse on `send_message` and `file_reservation_paths` (plain stdout). No wake. | VERIFIED `scripts/integrate_claude_code.sh:90-221` |
| codex | native | `codex.mcp.json` / config.toml MCP. Project-local `.codex/hooks.json` PostToolUse(Bash) runs the same `check_inbox.sh` in JSON mode through a gitignored token wrapper. The `notify` route was deprecated: its output never reaches the model. No wake. | VERIFIED `scripts/integrate_codex_cli.sh:187-248`, `scripts/hooks/codex_notify.sh:1-10` |
| hermes | none | No integration script and no mention found. It is usable only if Hermes speaks MCP over HTTP or stdio, which was not checked here. | VERIFIED absence (grep) |
| agy | none | No mention of agy or Antigravity. A Gemini CLI integration exists (`~/.gemini/settings.json` SessionStart, BeforeTool and AfterTool hooks, inbox check after `shell\|run_command`). Whether that applies to agy is unknown. | VERIFIED `scripts/integrate_gemini_cli.sh:230-311`; agy applicability is unknown |
| goose | none | No integration and no mention. Goose is an MCP client, so it could use the tools as a generic client without reminders or wake. | VERIFIED absence; generic use is INFERRED |
| pi | generic | Python: nothing. The Rust successor supports "Oh My Pi (OMP)" (a pi-derived agent) by writing `.omp/mcp.json` MCP config only. No hooks are described. Whether OMP integration covers upstream pi is unknown. | VERIFIED `rust:README.md:17`, `rust:README.md:488-530` |
| opencode | generic | `scripts/integrate_opencode.sh` writes `opencode.json` MCP config (or uses `opencode mcp add`) and bootstraps registration. No hooks or plugin, so no reminders and no wake. | VERIFIED `scripts/integrate_opencode.sh:1-40` (no `hook` occurrences) |

Other harnesses with integration scripts: Cline, Cursor, Windsurf, Factory Droid and GitHub Copilot. VERIFIED `scripts/integrate_*.sh`.

## Takeaways for herdr-threads

These are INFERRED analysis, not facts about upstream.

- **Same in-turn channel as ours.** For Claude and Codex they also use the PostToolUse `additionalContext` envelope, and they learned the same lesson: Codex `notify` output never reaches the model.
- **They inject a count, not content,** which costs an extra `fetch_inbox` round trip. Our in-turn body delivery avoids that.
- **Their reminder has a 120 s floor.** Mail sent shortly after a check is invisible to an active agent for up to two minutes.
- **No idle wake in OSS.** The `.signal` file is a hook point that nothing consumes. A file-watch plus keystroke daemon is left to users or to the paid Companion. Our Herdr-native wake with an idle check and draft protection is a real differentiator.
- **Newest-first inbox; importance is not a sort or wake key.** Our (priority, deadline, age) order and threshold-gated wake are stricter.
- **Overseer messages are not token-efficient.** The emoji preamble is repeated in every message body, which is the opposite of our R17 rule.
- **Identity is baked into per-project hook config,** keyed on `$TMUX_PANE` for pane identity. That is weaker than seat binding by Herdr pane, and multi-agent-per-project setups need manual configuration.
- **Ideas worth considering:**
  - The Git-backed human-auditable archive.
  - Advisory file leases with a pre-commit guard.
  - Contact policies (`auto` / `contacts_only`).
  - The Rust successor's session-bound identity that refuses borrowed names. It is cooperative, not authentication, which is consistent with our TRUST-POLICY stance.
