# mcp_agent_mail x OpenCode: message injection and wake

- Upstream: https://github.com/Dicklesworthstone/mcp_agent_mail (Python), clone at `83ed8d7c97af67f1e6b4700bb8ace08739171ed3`. Paths are relative to that clone root unless they are prefixed `rust:README.md`. That prefix means the Rust successor's README (https://raw.githubusercontent.com/Dicklesworthstone/mcp_agent_mail_rust/main/README.md, fetched 2026-10-23). The Rust code was not read.
- Harness: OpenCode (https://github.com/anomalyco/opencode, formerly sst/opencode). Harness facts come from `../../primers/opencode.md` (cited as "primer §N"), which read OpenCode `v1.18.1` and `dev`.
- License: MIT plus an OpenAI/Anthropic rider (see `overview.md`). Study the ideas, but do not copy any code.
- Labels: VERIFIED means read in source or docs. INFERRED means deduced. Unknown means not established.

## Support status

**Generic (MCP config only).** There are no hooks, no plugin, no reminders and no wake.

- **The only OpenCode integration is `scripts/integrate_opencode.sh`** (194 lines). VERIFIED.
  - It writes an `mcp` entry into `opencode.json` and prints a hint about `opencode mcp add` (it does not run it).
  - It writes a run helper and makes two best-effort bootstrap curls (`ensure_project`, `register_agent`).
  - The words `hook`, `plugin` and `.opencode/` do not occur in it. VERIFIED by reading the whole file.
- **The auto-installer calls it** when `opencode` is on PATH or `~/.opencode` exists (`scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh:126,157-162`). `scripts/install.sh:802` runs that auto-installer. VERIFIED.
- **Other references are incidental.** VERIFIED:
  - `CHANGELOG.md:374` mentions the setup script.
  - `.gitignore:297-298` ignores `/opencode.json` in the agent-mail repo itself, because it "contains bearer tokens".
  - `src/mcp_agent_mail/app.py:2612-2616` lists `"opencode"` among known program names; such a name is never honoured as an explicit agent name (`app.py:3363-3364`): it is replaced by a generated name in the default `coerce` mode, rejected in `strict` mode.
  - `README.md:2532` names OpenCode among tools the auto-installer wires up.
- **The Rust successor does not list OpenCode as a supported agent.** Its list is Claude Code, Codex CLI, Gemini CLI, OMP, Copilot CLI, "and any MCP-compatible client" (`rust:README.md:17`). OpenCode files appear only as config sources that OMP reads, and those are checked for drift (`rust:README.md:521-534`). VERIFIED.
  - INFERRED: the Rust `am setup run` does not configure OpenCode.
- **Compared with Claude and Codex:** the integration scripts for those harnesses install the PostToolUse(Bash) `check_inbox.sh` reminder (`overview.md`, "Injection mechanism"). OpenCode does not get even that count-only reminder.

## Injection mechanism

### What exists: model-pulled MCP tools and resources

- **Transport.** OpenCode connects as a remote MCP client to `http://<HTTP_HOST>:<HTTP_PORT><HTTP_PATH>`.
  - The default path is `/api/` (`src/mcp_agent_mail/config.py:396`). The script builds the URL from `get_settings()` at `scripts/integrate_opencode.sh:38-54`. VERIFIED.
  - The entry written is:
    ```json
    "mcp": { "mcp-agent-mail": { "type": "remote", "url": "<url>",
             "headers": { "Authorization": "Bearer <token>" }, "enabled": true } }
    ```
    (`scripts/integrate_opencode.sh:82-91`, `111-123`). VERIFIED.
- **Surface the model sees.** VERIFIED for the counts and the size; INFERRED that FastMCP uses docstrings as descriptions, which is FastMCP's default.
  - The server defines 49 `@mcp.tool` registrations (51 textual occurrences; two are in comments, `app.py:307,14636`). Their docstrings and `description=` strings total about 35-36.5 KB depending on whether both are counted (measured with read-only AST scripts over `app.py`).
  - It also defines 29 `@mcp.resource` templates, including `resource://inbox/{agent}{?project,since_ts,urgent_only,include_bodies,limit,agent_token,format}` (`app.py:13980-13990`).
  - Through OpenCode, resources are reachable only through its built-in `list_mcp_resources` and read tools (primer §5).
- **How a message gets into context.** The model has to decide to call `fetch_inbox` (`app.py:9446-9554`), `macro_start_session` (`app.py:9876`, which returns an inbox snapshot of `inbox_limit=10` without bodies), or read the inbox resource. The JSON result is the tool output. No other path exists for OpenCode. VERIFIED.
- **What prompts the model to call it.** Static text only. VERIFIED, except where noted.
  - The FastMCP server `instructions` string is generic and says nothing about polling (`app.py:5221-5228`). OpenCode puts it into `<mcp_instructions>` in the system prompt (primer §5).
  - The `fetch_inbox` docstring says "Poll after each editing step in an agent loop to pick up coordination messages" (`app.py:9486`).
  - The optional AGENTS.md blurb (`README.md:153-188`) says "check inbox with `fetch_inbox`". `scripts/install.sh:971-984` offers to run `docs insert-blurbs`. OpenCode reads `AGENTS.md` (primer §6). `integrate_opencode.sh` itself does not insert the blurb.
  - `SKILL.md` (13.6 KB) is not installed by any integrate script; `grep` of `scripts/` finds no copy. INFERRED from primer §6: if a user put it in `~/.claude/skills/` for Claude, OpenCode would pick it up as well.

### Payload format, bounds, order

All VERIFIED.

- **Per-message dict:** `{id, project_id, sender_id, thread_id, reply_to, topic, subject, importance, ack_required, created_ts, attachments, [body_md]}`, plus `from`, `kind` and `read_at` (`app.py:1843-1862`, `4650-4665`).
  - This is JSON by default. `format='toon'` gives a compact encoding (`app.py:5223-5226`).
  - No framing or "this came from another agent" marker exists beyond the fields themselves.
- **Bounds:** `limit` defaults to 20 and is clamped to 1000 (`app.py:1549-1556`). Bodies are opt-in (`include_bodies=False`).
  - No per-body truncation exists on the server side. INFERRED from primer §5: OpenCode's `Truncate` service may spill very large tool output to a file.
- **Order:** newest first (`ORDER BY created_ts DESC`, `app.py:4628`). Importance only filters (`urgent_only`); it does not sort.

### When it fires

Only when the model calls the tool. VERIFIED by the absence of any OpenCode hook or plugin.

- No turn-start injection: OpenCode's `chat.message` hook is not used.
- No mid-turn injection: `tool.execute.after` and `experimental.chat.messages.transform` are not used.
- No session-start digest. On Claude, SessionStart hooks print pending ACKs and reservations; OpenCode has nothing similar.
- A message sent while the agent is busy is invisible until the model next polls.
  - If the model does not follow the "poll after each editing step" guidance, the message waits until a human asks, possibly for the rest of the session. INFERRED.

### De-duplication and acknowledgement

- **`fetch_inbox` does not mutate read state** (`app.py:9466-9480`). Read state changes only through `mark_message_read` (`app.py:9706`) or `acknowledge_message` (`app.py:9784`), which are explicit tool calls. VERIFIED.
- **No de-duplication without discipline.**
  - A model that polls without `unread_only=true` gets the same 20 newest messages every time.
  - A model that polls with `unread_only=true` but never marks mail read sees the same mail every time.
  - VERIFIED for the semantics. The token cost of the re-reads is INFERRED.
- **ACK is per message**, through `ack_required` and an idempotent `acknowledge_message`. An overdue ACK only produces a server log line (`overview.md`, "Delivery guarantees"). Nothing reaches the OpenCode user or the sender. VERIFIED.

### Authentication: the hidden per-call cost on OpenCode

- **The configured path is stateless.** The script points OpenCode at the configured base (`/api/`). The server mounts the base and `/api` as stateless apps and only `/mcp` as stateful (`src/mcp_agent_mail/http.py:1130-1150`, `1775-1787`).
  - The source comment says of stateless mode: it "never issues an `Mcp-Session-Id` header, so session-bound agent authentication (#148) could never persist across HTTP tool calls" (`http.py:1137-1141`). VERIFIED.
- **Consequence:** every read of the inbox from OpenCode must carry `registration_token`. Session bindings are keyed by `ctx.session_id` (`app.py:5263-5283`). With no persisted session, `_authenticate_agent` falls through to the token check and raises `AUTHENTICATION_REQUIRED` without one (`app.py:5357-5430`).
  - The exception is a matching `MCP_AGENT_MAIL_WINDOW_ID`, which the script does not set.
  - VERIFIED for the code path. That every call needs the token is INFERRED, because OpenCode's remote-MCP session-id handling is unknown and there is no session id to keep anyway.
  - The token is `secrets.token_urlsafe(32)`, 43 characters (`app.py:3800`).
  - The model must keep it in context and repeat it on every `fetch_inbox`, `acknowledge_message` and similar call.
  - INFERRED: compaction can lose it. OpenCode's `experimental.session.compacting` hook (primer §2) could preserve it, but nothing uses that hook.

## Wake mechanism

**None.** VERIFIED by absence.

- `integrate_opencode.sh` installs nothing that runs while OpenCode is idle.
- The server pushes nothing that OpenCode would act on:
  - HTTP responses use `json_response=True` (`http.py:1134,1149`).
  - OpenCode's MCP client routes `notifications/message` to its log only and handles neither `resources/updated` nor sampling (primer §5).
- **Idle/busy detection and draft safety:** not applicable, because nothing types into the pane or calls OpenCode's server API.
- **Latency:** unbounded. An idle OpenCode session acts only when a human submits a prompt. VERIFIED, given that nothing wakes it.
- **The one hook point is unconsumed.** With `NOTIFICATIONS_ENABLED=true` (default false), `send_message` writes `{signals_dir}/projects/<slug>/agents/<agent>.signal` as JSON: `{timestamp, project, agent, message:{id, from, subject, importance}}` (`src/mcp_agent_mail/storage.py:3616-3686`). `fetch_inbox` clears it (`app.py:9548-9550`, `storage.py:3689-3716`). VERIFIED.
  - Nothing in the repo watches these files for any harness, OpenCode included. VERIFIED by grep (`overview.md`, "Wake mechanism").
- **Signal-file quirks.** VERIFIED in code; the consequences are INFERRED.
  - The 100 ms debounce drops a second signal entirely rather than coalescing it on the trailing edge (`storage.py:3647-3655`). The file keeps the first message's metadata, and a modify-event watcher gets no event for the second message.
  - The docstring path `{signals_dir}/{project_slug}/{agent_name}.signal` (`config.py:168`) does not match the actual `projects/<slug>/agents/` layout (`storage.py:3659`).
- **The README's tmux "zsh loop" and the commercial Companion "Message Stacks"** are the advertised wake routes (`README.md:47-52`, `329-333`). Neither is in the repo or specific to OpenCode.
  - A naive tmux keystroke loop into an OpenCode pane has specific hazards (primer §7):
    - ESC aborts a busy turn.
    - `/` or `@` opens autocomplete, which swallows the Enter.
    - Text appends onto a human draft.
    - A paste over 150 characters collapses into `[Pasted ~N lines]`.

### How well the generic mechanism could be made to wake OpenCode

This is INFERRED design analysis based on primer §§2-3 and §7. None of it exists upstream.

- **Best fit: an OpenCode server plugin.**
  - It would watch the `.signal` file, or poll `fetch_inbox` itself with a stored token.
  - On a `session.status` idle event it would call `client.session.promptAsync({parts:[{type:"text", text:"<short wake>", synthetic:true}]})`.
  - It would deliver bodies mid-turn through `tool.execute.after` or `experimental.chat.messages.transform`.
  - It never touches the composer, so it is draft-safe by construction (primer §1).
  - The `importance` field could gate the wake (urgent/high wakes; normal stages silently with `noReply:true`).
  - Two caveats: agent-mail identity is per project, not per OpenCode session, and one TUI can host many sessions (primer §10).
- **External HTTP:** needs OpenCode launched with `--port`, or `serve` plus `attach` (primer §3). agent-mail has no client for it.
- **tmux keystrokes:** the weakest option (see the hazards above). Using the TUI's `/tui/append-prompt` endpoint instead is still not draft-safe, because it concatenates onto the draft (primer §3).

## Setup and installation footprint on the harness

All VERIFIED against `scripts/integrate_opencode.sh`, except where noted.

| Item | Where | Notes |
|---|---|---|
| MCP server entry `mcp-agent-mail` | `${PROJECT_DIR:-<agent-mail repo root>}/opencode.json` (lines 31-32, 66) | Merged with `jq` into an existing file (76-107), or created with `$schema` (109-124). A `.bak` is made first (69-71). The file is made mode 600 (127). |
| Bearer token, in plaintext | inside that `opencode.json` (`headers.Authorization`) | The token is resolved from env, the systemd credential, `.env`, the user config or the legacy run helper (`scripts/lib.sh:114-163`). If none is found, a fresh one is generated (`integrate_opencode.sh:58-62`). |
| Run helper | `scripts/run_server_with_token.sh` in the agent-mail repo (lines 141-145, `lib.sh:207-243`) | Starts `serve-http`. |
| Server-side registration | `ensure_project` plus `register_agent(name=$USER, program="opencode", model="default")` over raw JSON-RPC curl (lines 155-181) | The response, including `registration_token` and the actual (usually generated) agent name, is discarded (`>/dev/null`). |
| Hooks, plugins, `.opencode/`, AGENTS.md, skills, `~/.config/opencode` | none | No global config is touched. `opencode mcp add` is only suggested (130-139). |

## Token cost

These are estimates (INFERRED), derived from VERIFIED sizes.

- **Per quiet turn: about 0 injected bytes.** Nothing runs on quiet turns, so the hook-path cost is zero.
- **Static cost per request.** About 35-36.5 KB of tool descriptions plus JSON schemas for 49 tools, on the order of 9-10k tokens, are present in every LLM request. Prompt caching amortises them, but they still occupy the context.
  - The server can shrink this with `TOOLS_FILTER_ENABLED`/`TOOLS_FILTER_PROFILE=core|minimal` ("up to ~70%", `config.py:126-150`). The OpenCode script does not set it.
- **Per message:**
  - one or more model-initiated tool calls, each repeating `project_key`, `agent_name` and the 43-character token;
  - a result of about 250-400 bytes of JSON metadata per message, plus `body_md` when requested;
  - often a second call to `acknowledge_message` or `mark_message_read`.
  - Each call is a full model step, which costs output tokens and latency on top of the payload.
- **Per poll with nothing new:** a whole tool round trip returning `[]`. With "poll after each editing step" guidance, these empty polls are the dominant recurring cost. A reminder-free harness like OpenCode depends on that guidance more than Claude or Codex do.

## Limitations and bugs visible

1. **Wrong default target directory.** Without `--project-dir`/`PROJECT_DIR`, `opencode.json` is written into the agent-mail repo itself (lines 31-32, 66), and the project registered is that repo's path (163-167). OpenCode launched in the user's project will not see the server. VERIFIED for the code; the effect is INFERRED.
2. **Bootstrap passes `name=$USER` to `register_agent` and throws away the result** (lines 164, 175-176).
   - A typical username (no `-`, `_` or `.`) fails `validate_explicit_agent_id`, which requires a separator (`utils.py:216-236`), and is not an adjective+noun name, so the default `coerce` mode silently generates a random adjective+noun name instead (`app.py:3361-3399`). Each bootstrap run therefore creates a new randomly named agent whose name and token are both discarded: an orphan identity. VERIFIED code path; the orphan effect is INFERRED.
   - Only a username containing a separator (e.g. `john.doe`) is honoured as-is (`app.py:3363-3368`). Then the token is lost, and a later `register_agent(name=$USER)` fails with `AUTHENTICATION_REQUIRED` because the existing identity needs its token (`app.py:6218-6227`). VERIFIED code path; the consequence is INFERRED.
   - The server's own mistake detector exists precisely because "hooks or scripts pass $USER instead of the actual agent name" (`app.py:2660-2668`, `2724-2731`); in `register_agent` it is consulted only in `strict` mode (`app.py:3380-3381`).
   - `curl -f` does not detect JSON-RPC tool errors, which come back as HTTP 200, so "Registered agent on server" can print on failure. INFERRED.
3. **Stateless endpoint plus no stored token.** The OpenCode model must carry its registration token by hand on every call (see Authentication above). The stateful `/mcp` mount exists, but the script does not use it.
4. **The generated token is not persisted for the server.**
   - Unlike `integrate_codex_cli.sh:62-70`, which saves to `.env`, the OpenCode script never writes a generated token anywhere except `opencode.json`. VERIFIED.
   - The run helper then generates yet another random token if none resolves (`lib.sh:227-238`), so the bearer tokens can diverge.
   - This is masked locally because `HTTP_ALLOW_LOCALHOST_UNAUTHENTICATED` defaults to true (`config.py:430`, `http.py:724-760`). It would bite with a remote server or with the bypass disabled. INFERRED.
5. **Token-bearing config in the project root.** `opencode.json` holds the bearer token, but only the agent-mail repo's own `.gitignore` ignores it. The script adds no ignore entry to the target project. VERIFIED.
   - INFERRED: a token that would otherwise be committed with the project's `opencode.json`. This matters only if the user commits that file.
6. **Limited config-format and dry-run handling.** VERIFIED for the code; the effects are INFERRED.
   - `--dry-run` is ignored on the merge path: `jq` plus `mv` write directly (lines 76-107), while only `write_atomic` honours `DRY_RUN` (`lib.sh:256-260`).
   - `opencode.jsonc` and `.opencode/opencode.json` are not handled, and `jq` fails on a commented `opencode.json`.
7. **No idle wake, no in-turn push, no turn-start digest, no ACK escalation to the user.** An overseer message marked "pause current work" (`README.md:457-479`) still waits for the model to poll. VERIFIED.
8. **Newest-first, unread-agnostic default.** A default `fetch_inbox` (limit 20, not `unread_only`) can hide older unread mail behind 20 newer read ones. VERIFIED from semantics.
9. **One identity per project, not per OpenCode session.** Parallel OpenCode sessions in one project share whatever name the model registered, or register separate random names, with no binding to the session or pane. INFERRED from the absence of any session plumbing (no `shell.env` or session-id use).

## Clever ideas worth stealing

These are INFERRED value judgements about VERIFIED features. Implement them independently; do not copy code.

- **Tool-surface profiles** (`TOOLS_FILTER_PROFILE=core|minimal|messaging|custom`, `config.py:126-150`). If we ever ship an MCP surface for OpenCode, a minimal profile keeps the static per-request cost small.
- **Explicit `unread_only` plus separate read and ACK verbs.** `fetch_inbox` is side-effect-free, so a passive peek can never consume mail. Contrast our ACK-on-display; the split is useful for preview surfaces, such as an OpenCode TUI-plugin toast that must not ACK.
- **Signal file as a harness-neutral doorbell:** a tiny JSON per (project, agent) carrying only id, from, subject and importance, and cleared on read. For OpenCode, a server plugin watching a per-seat doorbell and calling `promptAsync` on idle would be a dependency-free wake path. Do it with a trailing-edge debounce, unlike theirs.
- **Server-side name-mistake detector** (`app.py:2686-2731`): it gives actionable errors when a model passes a program, model or `$USER` value as its identity. That is cheap guidance for less capable OpenCode models.
- **Rust `am inbox-events` durable cursors** (`rust:README.md:742-761`): body-free, append-only delivery events with `next_cursor`, and `cursor_expired`/`cursor_ahead` recovery. It is a clean contract for an external OpenCode plugin that must resume after a restart without re-delivering.

## Verification

Adversarial pass on 2026-10-23 against clone `83ed8d7c`, the Rust README (fetched the same day) and `../../primers/opencode.md`.

- Checked: all of `scripts/integrate_opencode.sh` (194 lines; the JSON entry, `opencode mcp add` only suggested, bootstrap curls, no hook/plugin/`.opencode/`), the auto-installer detection and call, `install.sh:802` and the blurb offer, `.gitignore`, `CHANGELOG.md:374`, `config.py` defaults (`/api/`, notifications off, 100 ms debounce, localhost bypass on, tool-filter profiles), `fetch_inbox` (side-effect-free, limit clamp, newest-first, signal clear), `macro_start_session`, the inbox resource, `_authenticate_agent`, the session-binding comment, `http.py` stateless/stateful mounts, the signal-file code, `lib.sh` token resolution and run helper, `README.md` blurb/tmux FAQ/overseer preamble, Rust README lines 17, 521-534 and 742-761, and the primer sections cited.
- Changed:
  - Bootstrap `$USER` claim was wrong for typical usernames: without a separator the name is coerced to a random generated name (orphan identity per run), not honoured; the honoured-then-locked-out path applies only to separator-bearing usernames.
  - Tool count 51 -> 49 (two textual hits are comments); description size given as a 35-36.5 KB range; resource count "about 25" -> 29.
  - `"opencode"` program-name entry restated precisely (coerced or rejected, not a hard reservation); added the `README.md:2532` mention.
  - Payload fields gained `kind`.
  - Citation fixes: `macro_start_session` 9873 -> 9876, `config.py:166` -> 168, `integrate_codex_cli.sh:63-68` -> 62-70, `lib.sh:219-229` -> 227-238.
