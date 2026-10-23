# mcp_agent_mail x Claude Code

- Upstream: https://github.com/Dicklesworthstone/mcp_agent_mail (Python), clone at `83ed8d7c97af67f1e6b4700bb8ace08739171ed3`. Paths are relative to the clone root.
- Successor: https://github.com/Dicklesworthstone/mcp_agent_mail_rust. Only its README was read (raw `main`, 2026-10-23); paths prefixed `rust:README.md`.
- Harness reference: `../../primers/claude.md` (cited as "primer §N").
- Tags: **VERIFIED** = read in source or docs; **INFERRED** = reasoned, not observed; **unknown** = not found. No code from the clone was executed.

## Support status

**Native, injection only; no wake.** VERIFIED.

- A dedicated installer, `scripts/integrate_claude_code.sh`, registers the HTTP MCP server and installs five hook entries carrying six commands (`scripts/integrate_claude_code.sh:20-26`, `:197-230`).
- The model reaches mail only through MCP tools (`fetch_inbox`, `acknowledge_message`, `mark_message_read`, ...) or `resource://inbox/{agent}` (`src/mcp_agent_mail/app.py:9446-9558`, `:9698`, `:9776`, `:13981`).
- The only push into Claude's context is a **count-only** `PostToolUse(Bash)` reminder (`scripts/hooks/check_inbox.sh:160-172`) plus a plain-stdout `SessionStart` dump of leases and pending ACKs (`scripts/integrate_claude_code.sh:197-205`).
- Nothing in the OSS code wakes an idle Claude session (VERIFIED by absence, see "Wake mechanism").
- Rust successor: it lists Claude Code as a supported agent and documents MCP config only (`rust:README.md:17`, `:427-445`). The README describes no Claude hooks and no wake for it. It adds a durable cursor feed (`am inbox-events`, MCP `fetch_inbox_events`) "for monitors" (`rust:README.md:742-761`, `:782-784`), but it ships no Claude consumer for that feed. VERIFIED (README only); whether the Rust installer writes hooks is unknown.

## Injection mechanism

### 1. Pull: MCP tools and resources (the only path that carries bodies)

The transport is FastMCP over Streamable HTTP. It is registered as `{"type":"http","url":..., "headers":{"Authorization":"Bearer <token>"}}` (`scripts/integrate_claude_code.sh:259-271`). The server's MCP `instructions` are a single 3-sentence string (about 280 chars) with no mail guidance; the only usage hint is the optional `format='toon'` output (`src/mcp_agent_mail/app.py:5221-5228`). Claude delivers server instructions at connect (primer §4, 4,096-char cap), so this costs about 70 tokens per session. VERIFIED (string); token figure INFERRED.

`fetch_inbox` (`app.py:9454-9557`):

- It "Retrieve[s] recent messages for an agent without mutating read/ack state" (`app.py:9468`). The default is `limit=20`, clamped to 1000 by `_validate_limit` (`app.py:1549-1561`), and `include_bodies=False` (`app.py:9454-9466`).
- `_list_inbox` orders the results `ORDER BY created_ts DESC`, newest first (`app.py:4628`). `unread_only` filters `MessageRecipient.read_ts IS NULL` (`app.py:4639-4646`).
- Per-row payload: `id, project_id, sender_id, thread_id, reply_to, topic, subject, importance, ack_required, created_ts, attachments` (`app.py:1843-1861`) plus `from`, `from_project*`, `kind`, `read_at` (`app.py:1894-1905`, `:4650-4665`). `body_md` is included only if requested. An optional `format` argument (`app.py:9465`) can return TOON instead of JSON.
- Side effect: when notifications are enabled, it clears the agent's `.signal` file, **whether or not bodies were returned** (`app.py:9548-9550`).

Read and ACK are separate calls. `mark_message_read` sets `read_ts` (`app.py:9762-9764`). `acknowledge_message` sets both `read_ts` and `ack_ts` and is idempotent (`app.py:9784-9850`). VERIFIED.

Framing: raw JSON tool results. The only model-visible prose framing is the human-overseer preamble, which the web overseer composer prepends to the stored body of overseer messages only (`src/mcp_agent_mail/http.py:3802-3819`): a roughly 14-line block with emoji, "Please prioritize the instructions below over your current tasks ... The human's guidance supersedes all other priorities." (`README.md:457-479`). VERIFIED.

Size bounds:
- The web overseer composer caps bodies at 50,000 characters, less the preamble (`src/mcp_agent_mail/http.py:3796-3827`). VERIFIED.
- No cap was found on MCP `send_message` bodies (unknown).
- On the Claude side, MCP results warn at 10k tokens and cap at 25k by default (primer §5). A `limit=20, include_bodies=true` fetch can therefore be truncated by the harness (INFERRED).

The model is told to poll by an optional CLAUDE.md/AGENTS.md blurb: "check inbox with `fetch_inbox` and acknowledge with `acknowledge_message`" (`README.md:154-188`, line 171). The `fetch_inbox` docstring advises: "Poll after each editing step in an agent loop" (`app.py:9486`). VERIFIED.

### 2. Push: `PostToolUse(Bash)` unread-count reminder

Installed entry (`scripts/integrate_claude_code.sh:180`, `:211-213`):

```
{ "matcher": "Bash", "hooks": [ { "type": "command", "command":
  "AGENT_MAIL_PROJECT='<abs>' AGENT_MAIL_AGENT='<Name>' AGENT_MAIL_URL='<url>' AGENT_MAIL_TOKEN='<bearer>'
   AGENT_MAIL_REGISTRATION_TOKEN='<reg>' AGENT_MAIL_HOOK_FORMAT='json' AGENT_MAIL_INTERVAL='120'
   '<project>/.claude/hooks/check_inbox.sh'" } ] }
```

Code path in `scripts/hooks/check_inbox.sh` (VERIFIED):

1. Configuration (`:42-64`): if project or agent is unset, or a placeholder (`YOUR_`, `PLACEHOLDER`, `<...>`), the hook does `exit 0` silently.
2. Rate limit (`:66-80`): `RATE_FILE="/tmp/mcp-mail-check-${AGENT//[^a-zA-Z0-9]/_}"`. If fewer than `INTERVAL` (120) seconds have passed, it exits. Otherwise it writes `NOW` to the file **before** the fetch.
3. Probe (`:95-110`): a JSON-RPC `tools/call fetch_inbox` with `{"limit":10,"include_bodies":false,"unread_only":true, registration_token?}`, sent by `curl --max-time 3`. Each curl is a fresh, unauthenticated MCP request, so the per-agent `registration_token` is required (`:23-28`).
4. Errors (`:113-120`): an empty response, or a grep match on `"isError":true`, ends in `exit 0`.
5. Counting (`:128-158`): an inline `python3` parses `structuredContent.result` or the legacy `content[0].text`, and counts the total and the number with `importance in ("urgent","high")`.
6. Output (`:160-172`), only when the count is greater than 0:

   ```
   {"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":
    "You have N unread message(s) in your inbox (M urgent/high priority). Use fetch_inbox to read them; reading or acknowledging a message stops this reminder."}}
   ```

   The fallback `text` format (`:173-183`) is an emoji banner on plain stdout. The script's own comment says this "does NOT reach the agent" (`:30-36`).

When it fires: only after a `Bash` tool call. Not after Read, Edit, Write, Grep or MCP calls, and never at turn start or turn end. At most one probe per 120 s per agent name. Per primer §1.2, `additionalContext` is wrapped as a system reminder beside the tool result. The delivery is therefore mid-turn, but only to an agent that is actively running shell commands. VERIFIED (matcher and script); timing semantics from the primer.

Bounds: the text is a fixed sentence of at most about 156 characters, far below Claude's 10,000-char cap. The count saturates at 10 because the probe sends `limit:10` and counts `len(msgs)` (`:97`, `:145`). VERIFIED.

De-dup / ACK:
- The reminder repeats every 120 s, on the next Bash call, until each message has `read_ts` set (`unread_only`, GH #274, `:90-94`).
- `fetch_inbox` does **not** set `read_ts` (`app.py:4640-4645`). A model that obeys "Use fetch_inbox to read them" and then stops is reminded again two minutes later. Among MCP tools, only `mark_message_read` or `acknowledge_message` silences the reminder (the web UI's `mark-read` / `mark-all-read` endpoints also set `read_ts`, `http.py:3094`, `:3180`), and the reminder text does not name `mark_message_read`. VERIFIED (code). That this nag occurs in practice is INFERRED.
- No hook output is acknowledged. The server does not know whether the reminder reached the model.

### 3. `SessionStart`: lease and ACK tables (plain stdout)

`scripts/integrate_claude_code.sh:197-205` installs two command hooks with `matcher: ""`:

```
cd '<mcp_dir>' && uv run python -m mcp_agent_mail.cli file_reservations active '<project>'
cd '<mcp_dir>' && uv run python -m mcp_agent_mail.cli acks pending '<project>' '<Agent>' --limit 20
```

- Both commands print Rich `Table`s through a default `Console()` (`src/mcp_agent_mail/cli.py:108`, `:4755-4772`, `:4876-4907`). `acks pending` lists `Msg ID, Thread, Subject, Kind, Created, Read, Ack Age`, newest first. VERIFIED.
- They query the SQLite DB directly from the install directory. They do not go through the HTTP server (`cli.py:4723-4737`, `:4848-4867`). VERIFIED.
- `matcher: ""` fires on every source (startup, resume, clear, compact, fork; primer §1.2).
- SessionStart plain stdout enters context (primer §1.2). So every session start, resume, clear and compact injects **two box-drawn tables, even when they are empty**: no `if rows` guard exists before `console.print(table)` (`cli.py:4772`, `:4907`). VERIFIED (code); the context effect follows from the primer. It also costs two `uv run` Python cold starts.
- This is the only place where subjects of pending ACK-required mail are pushed. Bodies are never pushed.

### 4. Other hooks (most likely invisible to the model)

All three print plain-stdout Rich tables:

- `PreToolUse` `matcher: "Edit"` runs `file_reservations soon --minutes 10` (`:207-209`).
- `PostToolUse` on `mcp__mcp-agent-mail__send_message` runs `list-acks` (`:215-217`).
- `PostToolUse` on `mcp__mcp-agent-mail__file_reservation_paths` runs `file_reservations list` (`:219-221`).

Per primer §1.2, plain exit-0 stdout on Pre/PostToolUse does not reach the model, and upstream says so itself (`check_inbox.sh:30-36`). These hooks therefore cost latency, roughly a Python start each per Edit, and deliver nothing to Claude. VERIFIED (plain stdout); "invisible" is INFERRED from the primer and upstream's own comment.

## Wake mechanism

**None for Claude Code.** VERIFIED by absence:

- No `Stop`, `UserPromptSubmit`, `Notification`, `asyncRewake`, channel, monitor, cross-session socket or `tmux send-keys` path appears in `scripts/integrate_claude_code.sh` or `scripts/hooks/`.
- `src/` has no MCP `notifications/` or resource subscriptions (grep: `subscribe|notifications/` returns no hits in `src/mcp_agent_mail/*.py`).

What exists instead:

- **Signal files, unconsumed.** With `NOTIFICATIONS_ENABLED=true` (default false), `send_message` writes `{signals_dir}/projects/<slug>/agents/<Agent>.signal`: JSON with timestamp, project and agent, plus id/from/subject/importance if metadata is enabled (default on). Writes are debounced at 100 ms per (project, agent) (`src/mcp_agent_mail/storage.py:3623-3686`).
  - The docstring says agents "can watch" it. No watcher ships for Claude.
  - The Claude hook's own count probe calls `fetch_inbox`, which deletes the signal (`app.py:9547-9549`). Any future watcher would therefore lose the signal as soon as a Bash call happens, even though the model never read the mail. VERIFIED (code); the interaction is INFERRED.
- **Humans and paid tooling.** The README points to a "historical zsh loop" that feeds panes via tmux (not in the repo) and to the commercial Companion "Message Stacks" ("without touching tmux") (`README.md:46-52`, `:331-333`). How those work: unknown.

Consequences for Claude (INFERRED):
- **Idle/busy detection:** none. Nothing needs it, because nothing types.
- **Draft safety:** trivially safe, since nothing writes to the composer.
- **Latency:**
  - An idle session never sees mail until a human prompts it.
  - A busy session sees a count on its first Bash call at least 120 s after the previous probe. Mail that arrives just after a probe waits up to 120 s plus the time to the next Bash call.
  - A turn with no Bash calls (pure Read/Edit, or MCP-only) gets no reminder at all.
- **Failure modes:**
  - Every hook failure is a silent `exit 0`, including expired tokens, a server that is down, or a renamed agent. The agent simply goes deaf, with no diagnostic in context or terminal.

## Setup / installation footprint

`scripts/integrate_claude_code.sh` writes (VERIFIED):

| Path | Content | Lines |
|---|---|---|
| `<project>/.claude/hooks/check_inbox.sh` | copy of the hook script, `chmod +x` | `:90-100` |
| `<project>/.claude/settings.json` | five hook entries, six commands (one SessionStart entry with two commands, PreToolUse Edit, PostToolUse x3), merged with jq, mode 644 | `:197-237` |
| `<project>/.claude/settings.local.json` | `mcpServers["mcp-agent-mail"]` with bearer header (plus optional `morph-mcp`), mode 600 | `:243-290` |
| `<project>/.gitignore` | `.claude/settings.local.json` | `:293`, `scripts/lib.sh:446-470` |
| `~/.claude/settings.json` (**user config**) | `mcpServers["mcp-agent-mail"]` with bearer, mode 600 | `:295-383` |
| `claude mcp add --scope user` and `--scope project` | user `~/.claude.json` and project `.mcp.json` entries with `-H "Authorization: Bearer <token>"` | `:393-399` |
| `<mcp_dir>/scripts/run_server_with_token.sh` | server launcher | `:386-390` |
| `<project>/.agent-mail/identity` | registered agent name | `scripts/lib.sh:963-968` |
| `/tmp/mcp-mail-check-<Agent>` | rate-limit timestamp (at runtime) | `check_inbox.sh:67-80` |

- Registration happens once, at install, with hard-coded `program="claude-code" model="claude-sonnet"` (`:135`). The resulting name and `registration_token` are baked into the hook command (`:180`).
- Where `claude mcp add` writes is per Claude Code docs, not this repo: user scope goes to `~/.claude.json`, project scope to `.mcp.json` (INFERRED).
- `mcpServers` inside `settings.json` / `settings.local.json` may be ignored by Claude Code, since MCP config lives in `.mcp.json` and `~/.claude.json`. Unknown; not verified against the docs here.

## Token cost

Estimates are INFERRED at about 4 chars per token.

- **Per quiet turn:** 0. The PostToolUse hook prints nothing when there is no unread mail or when it is rate-limited. Latency is not free, though: every Bash call forks bash, and every 120 s it also runs curl and three to five `python3` starts: two or three `json_escape` calls, the counter, and one more for the JSON envelope when mail is pending (`check_inbox.sh:83-96`, `:128`, `:171-172`). Every Edit pays a `uv run python` start for an invisible table.
- **Per session start / resume / clear / compact:** two Rich tables, empty or not. Roughly 150-300 tokens when empty, plus about 25-60 tokens per lease or ACK row. Add about 70 tokens of MCP instructions at connect.
- **Per message (steady state):**
  1. reminder: about 45 tokens, repeated every 120 s of Bash activity until marked read;
  2. a `fetch_inbox` call (`include_bodies=false`): about 40 tokens of call and about 120-180 tokens of JSON per row (15+ fields);
  3. a second fetch with bodies, or `resource://`, for the body;
  4. a `mark_message_read` or `acknowledge_message` call, about 60 tokens each, per message.

  That is at least 3 model round trips and roughly 400+ tokens of overhead before the body itself. A model that skips step 4 pays the reminder again every 2 min.
- **Tool surface:** 49 `@mcp.tool` and 29 `@mcp.resource` registrations (grep count in `app.py`). Claude defers MCP tools under tool search (primer §5), so the per-turn schema cost is bounded by deferral. Optional `TOOLS_FILTER_ENABLED=true` with `TOOLS_FILTER_PROFILE=core|minimal|messaging|custom` trims the surface (`src/mcp_agent_mail/config.py:127-157`). VERIFIED (counts and config).

## Limitations and bugs

1. **Secrets in the git-tracked settings file.** VERIFIED.
   - The section header says `settings.json: HOOKS ONLY (no secrets, git-tracked)` (`integrate_claude_code.sh:182-184`), and the file is made world-readable (`chmod 644`, `:237`).
   - But the PostToolUse hook command written into it embeds `AGENT_MAIL_TOKEN='<bearer>'` and `AGENT_MAIL_REGISTRATION_TOKEN='<reg>'` (`:180`, `:212`, `:228`).
   - The Codex integrator avoids exactly this with a mode-0700, gitignored wrapper (`scripts/integrate_codex_cli.sh:211-228`). Claude's integrator does not.
   - `claude mcp add --scope project` also puts the bearer into the project `.mcp.json` (`:398`), a file meant to be committed (INFERRED from Claude docs).
2. **Reinstall cannot repair a stale hook.** VERIFIED.
   - `json_append_hook` returns the config unchanged if any existing command contains the identifier `check_inbox.sh` (`scripts/lib.sh:422-441`).
   - So a hook first written with the `YOUR_AGENT_NAME` placeholder (server down at install, `:163-169`), or with an old token, stays that way. This holds even though the installer tells the user to "reconfigure with: ./scripts/integrate_claude_code.sh" (`:168`).
   - With a placeholder, the hook silently no-ops forever (`check_inbox.sh:62-64`).
3. **One baked identity per project config.** VERIFIED (config).
   - Every Claude session in the project, and every subagent, since the hook ignores stdin and has no `session_id`/`agent_id` check, probes the same install-time agent name.
   - The blurb tells the model to `register_agent` itself (`README.md:169`), which auto-generates a fresh name when `name` is omitted (`app.py:6139`). The model can then end up reading and ACKing a different inbox from the one its hook watches. INFERRED.
   - The per-pane identity contract (`docs/IDENTITY_CONTRACT.md:8-30`, keyed on `$TMUX_PANE`) is not used by the Claude hook at all. VERIFIED (`check_inbox.sh` reads env only).
4. **Rate-limit slot keyed only by agent name, written before the fetch.** VERIFIED (`check_inbox.sh:67-80`).
   - Two projects with the same agent name share the 120 s window.
   - A failed curl still consumes the window.
   - A subagent's Bash call can consume the window, so its reminder lands in the subagent's context while the main loop waits another 120 s. INFERRED.
5. **The reminder tells the model to use the one call that does not silence it** (`fetch_inbox` does not mark read; `app.py:4640-4645` versus `check_inbox.sh:162-164`). VERIFIED.
6. **The count caps at 10, and the inbox is newest first.** Importance changes only wording, never ordering or delivery (`check_inbox.sh:97`, `:161-165`; `app.py:4628`). VERIFIED.
7. **SessionStart injects empty tables on every start, resume, clear and compact.** The PreToolUse and other PostToolUse hooks produce output the model cannot see. VERIFIED (code); visibility per primer.
8. **`PreToolUse` matcher is `Edit` only.** It misses `Write`/`MultiEdit`/`NotebookEdit` and Bash-driven edits (`:208`). VERIFIED (matcher); a moot point while its output is invisible.
9. **Silent failure everywhere.** Nothing tells the user or the model that mail delivery is broken (`check_inbox.sh:51-64`, `:113-120`, `:151-153`). VERIFIED.
10. **The signal-file wake hook point is defeated by the reminder probe** (see "Wake mechanism"). VERIFIED (code paths); the effect is INFERRED.
11. **Writes the real user config** (`~/.claude/settings.json`, `claude mcp add --scope user`) during a per-project integration (`:295-399`). VERIFIED.

## How the generic mechanism fares on Claude, and what a better fit would be (INFERRED, from the primer)

- **MCP pull:** it works, but only when the model chooses to call it. Claude has no MCP push except channels (primer §4, §5).
- A **long-poll `fetch_inbox`** would hold a turn open and cannot wake an idle session (primer §5).
- The repo's own building blocks would map cleanly onto Claude-native wake surfaces:
  - A `Stop` hook (primer §1.3) running the same count probe could block a turn that is ending when there is unread urgent mail. It needs no rate limit, because it fires once per turn.
  - An `asyncRewake` hook that waits on the `.signal` file and exits 2 (primer §1.4) would give a hook-only idle wake.
  - A plugin monitor tailing `am inbox-events` (Rust) or the signal directory would give a push notification line (primer §8).
  - None of these is implemented upstream.

## Clever ideas worth stealing

INFERRED value judgements; the mechanisms themselves are VERIFIED.

- **Silent-when-empty, unread-gated reminder.** The design rule "never re-remind a message the agent already read" (GH #274, `check_inbox.sh:90-94`) matches our quiet-turn rule. Their bug, recommending a non-marking read, is the cautionary tale: the instruction text and the ACK semantics must agree. Our inbox ACK-on-display avoids this.
- **Explicit statement that plain PostToolUse stdout never reaches the model** (`check_inbox.sh:30-36`, `:168-170`). This is independent confirmation of our envelope choice; worth a regression test on our side if one does not already exist.
- **Placeholder detection in the hook** (`check_inbox.sh:56-64`): a cheap guard against half-configured installs. Ours should fail loud rather than silent, though.
- **Secrets out of hook command lines through a gitignored 0700 wrapper** (Codex path, `integrate_codex_cli.sh:211-228`). It is a good pattern; their own Claude installer just doesn't follow it.
- **Rust durable delivery cursor** (`rust:README.md:742-761`):
  - body-free, oldest-first events;
  - `next_cursor` is persisted only after processing;
  - explicit `cursor_expired` / `cursor_ahead` errors carrying `oldest_available_cursor` / `tail_cursor`.

  It is a clean contract for an external monitor or waker, comparable to our watch ACK ledger, and its error vocabulary for cursor recovery is worth comparing against ours.
- **Pending-ACK subjects at SessionStart.** Surfacing outstanding obligations at session start is useful. Doing it as a compact, non-empty-only list would be the token-efficient version, and our check-in already does this.
- **Tool-surface profiles** (`TOOLS_FILTER_PROFILE=minimal`, `config.py:127-157`): if we ever expose MCP tools, offer a minimal profile from day one.
- Licence note: MIT with an OpenAI/Anthropic rider (see `overview.md`). Ideas only, no code.

## Verification

Checked against the clone at `83ed8d7c`, the Rust README (raw `main`, identical to the scratch copy) and the primer. Mechanisms, hook names, matchers, the reminder text, rate limit, ACK semantics, signal files, the installer footprint and the secret-in-`settings.json` finding all hold. Changes:

- `@mcp.tool` count 49 and `@mcp.resource` 29 confirmed (decorator lines; a plain substring grep gives 51 because of two docstring mentions).
- Fixed citations: tool lines (`mark_message_read` `:9698`, `acknowledge_message` `:9776`), `fetch_inbox` docstring `:9468`, signal clear `:9548-9550`, `config.py:127-157`.
- Corrected: MCP instructions are 3 sentences (about 70 tokens, not 60); reminder about 156 chars (not 190); probe python starts are 3-5 (not 2); SessionStart sources include `fork`; the installer writes five entries with six commands; tool filtering needs `TOOLS_FILTER_ENABLED=true`.
- Narrowed: the overseer preamble applies only to web-composer overseer messages; the web UI's mark-read endpoints also silence the reminder.
- Upgraded the count cap at 10 to VERIFIED (`limit:10` plus `len(msgs)`).
- Added: optional TOON output format; signal JSON also carries project and agent.
- Left as unknown: whether Claude Code reads `mcpServers` from `settings.json`, and whether the Rust installer writes hooks (its README says only that it writes agent configs, `rust:README.md:425`).
