# mcp_agent_mail x Pi coding agent (pi-mono)

- Alternative: https://github.com/Dicklesworthstone/mcp_agent_mail (Python), cloned at `83ed8d7c97af67f1e6b4700bb8ace08739171ed3`. Paths without a prefix are relative to this clone.
- Successor: https://github.com/Dicklesworthstone/mcp_agent_mail_rust. Only `README.md` on `main` was read (fetched 2026-10-23), cited as `rust:README.md:N`. Latest GitHub release is `v0.3.39` (2026-10-13), from the GitHub REST API.
- Harness: Pi, https://github.com/badlogic/pi-mono, at `a7b94a03f09f20f4937fa1a76339014c60d261ad`. Paths prefixed `pi:` are relative to that clone. The primer is `../../primers/pi.md`.
- Tags: VERIFIED means read in source or docs at the cited line. INFERRED means my reading. Unknown means not determinable from what was read.

## Support status

**Not supported for upstream Pi, in either implementation. Pi can use it only as a generic MCP client, with no reminder hook and no wake.**

| Version | What exists for Pi | Status |
|---|---|---|
| Python (this clone) | Nothing. A case-insensitive grep over the whole tree for `pi-mono`, `badlogic`, `oh my pi`, `omp`, `.pi/`, `pi-coding` and a bare `pi` word finds no hit. There is no `scripts/integrate_pi*.sh`. The auto-detector probes only `~/.claude`, `~/.codex`, `~/.cursor`, `~/.factory`, `~/.gemini` and `opencode`. | VERIFIED, grep; `scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh:21,126` |
| Rust successor | Supports "Oh My Pi (OMP)" (https://omp.sh). `am setup run --agent omp` writes `.omp/mcp.json` plus the active OMP profile's user config (`~/.omp/agent/mcp.json` or `~/.omp/profiles/<name>/agent/mcp.json`). Only MCP config is described: no hooks, no extension, no wake. | VERIFIED `rust:README.md:17`, `rust:README.md:488-560` |

**OMP is not upstream Pi.**

- OMP lives at https://github.com/can1357/oh-my-pi, homepage `omp.sh`, about 36k stars. The GitHub API reports `fork: false` with no parent. VERIFIED, REST API 2026-10-23.
- Its lineage from Pi is INFERRED, from the name and the `PI_*` environment variables the Rust README says OMP honours (`rust:README.md:491-497`).
- The two have diverged on paths and config:

| Aspect | OMP, per the Rust README | Upstream Pi |
|---|---|---|
| Project MCP config | `.omp/mcp.json` | `.pi/mcp.json`, read only after project trust. VERIFIED `pi:packages/coding-agent/docs/mcp.md:32` |
| User MCP config | `~/.omp/agent/mcp.json` | `~/.pi/agent/mcp.json`. VERIFIED same line |
| Config dir override | `PI_CONFIG_DIR` | `PI_CODING_AGENT_DIR` only. VERIFIED `pi:packages/coding-agent/docs/environment-variables.md:81` |
| Profiles | `OMP_PROFILE` / `PI_PROFILE` | none found. Grep for `PI_CONFIG_DIR\|PI_PROFILE` in `pi:packages/coding-agent/src` and docs finds nothing |
| Settings format and sources | YAML `config.yml`; also merges `.claude/settings.json`, `.codex/config.toml`, `opencode.json` and others (`rust:README.md:515-525`) | none of this found in Pi docs |

- Upstream Pi's repository never mentions OMP. VERIFIED: grep for `oh-my-pi|omp.sh|oh my pi` in the pi-mono clone finds nothing.
- **Consequence:** `am setup run --agent omp` writes files that upstream Pi never reads. On upstream Pi you add the server by hand, with `pi mcp add` or by editing `~/.pi/agent/mcp.json`. INFERRED from the path mismatch.
- Whether OMP honours `PI_CODING_AGENT_DIR` with upstream semantics is unknown. The README asserts it but was not checked against OMP source.

The rest of this report covers the only mechanism that applies to upstream Pi: the generic MCP server, used through Pi's built-in MCP client.

## Injection mechanism

### What mcp_agent_mail provides

Injection is **pull only**. Message content reaches the model only when the model itself calls an MCP tool, or a resource-read tool, and gets the content back as a tool result.

- **Tool `fetch_inbox`**, at `src/mcp_agent_mail/app.py:9446-9470`. VERIFIED.
  - Arguments: `project_key`, `agent_name`, `limit=20`, `urgent_only`, `include_bodies=False`, `since_ts`, `topic`, `unread_only=False`, `registration_token`, `format` (`json` or `toon`).
  - Docstring: "Retrieve recent messages for an agent without mutating read/ack state." It suggests "Poll after each editing step in an agent loop" (`app.py:9471-9495`).
  - The docstring lists each item as `{ id, subject, from, created_ts, importance, ack_required, kind, read_at, [body_md] }` (`app.py:9497-9501`). The actual row is wider: `_message_to_dict` also emits `project_id`, `sender_id`, `thread_id`, `reply_to`, `topic` and `attachments` (`app.py:1843-1861`), and `_list_inbox` adds sender identity, `kind` and `read_at` (`app.py:4650-4665`). VERIFIED.
  - `limit` is capped at 1000 (`app.py:9511-9514`). Ordering is newest first (overview, `app.py:4628-4629`).
- **Resources** such as `resource://inbox/{agent}`: 29 `@mcp.resource(` registrations. VERIFIED by count in `app.py`. In Pi they are reachable only through Pi's generic `list_mcp_resources` and `read_mcp_resource` tools, again pulled by the model (`pi:packages/coding-agent/docs/mcp.md:236-248`). Those resource tools take "the widest exposure among those servers" (`mcp.md:244`), so with only this server at the default `codemode` they too are codemode-only, not declared to the model. VERIFIED.
- **ACK** is separate:
  - `acknowledge_message` sets read and ACK timestamps; `mark_message_read` marks read only. `fetch_inbox` marks nothing (overview, `app.py:9467-9480`, `9776-9800`). VERIFIED.
  - Every displayed message costs a second round trip to clear it. Otherwise the `unread_only` view keeps returning it.
- **Framing** comes only from the server:
  - The server sends a one-line MCP `instructions` string: "You are the MCP Agent Mail coordination server. ..." (`app.py:5221-5228`). VERIFIED.
  - Tool results are raw JSON, or TOON when `format='toon'`.
  - Overseer messages carry an emoji "pause and prioritize" preamble inside the body (overview, `README.md:457-479`).
- **Dedupe:** none at the server for repeated reads of the same body. `unread_only=true` is the only filter. VERIFIED (overview).
- **Size bounds:**
  - Bodies are opt-in (`include_bodies=False`). `body_md` is returned untruncated (`app.py:1860` assigns `message.body_md` directly).
  - The only truncation helpers in `app.py` apply to TOON stderr and to summary excerpts (`app.py:947`, `app.py:5016` `[:800]`). VERIFIED.

**Not available on Pi:** the count-only reminder hook. `scripts/hooks/check_inbox.sh` is wired only as a Claude, Codex, Gemini or Factory shell hook.

- Its JSON mode emits a Claude-specific `hookSpecificOutput.additionalContext` envelope (`scripts/hooks/check_inbox.sh:12-16,167-172`). Its text mode prints a banner for the human (`:173-184`). VERIFIED.
- Pi has no shell-hook subprocess model at all. Its hooks are in-process TypeScript extension event handlers (primer §1; `pi:packages/coding-agent/docs/extensions.md:3-5`).
- So nothing mcp_agent_mail ships ever pushes into a Pi session. VERIFIED by absence.

### How it actually lands in Pi's context

Every step below is Pi behaviour (VERIFIED in Pi docs) applied to this server (INFERRED):

1. **Connection.**
   - Pi's built-in MCP client speaks stdio and streamable HTTP and rejects legacy SSE (`pi:.../docs/mcp.md:3,62-64,78-79`).
   - The Python server's default `serve-http` is streamable HTTP (JSON responses) at `http://127.0.0.1:8765/api/` (`src/mcp_agent_mail/config.py:396`, `cli.py:1202-1246`), with no SSE, so Pi can connect. `serve-stdio` also exists (`cli.py:1245-1246`).
   - **The default `/api/` endpoint is stateless.** It is built with `stateless_http=True` and "never issues an `Mcp-Session-Id` header"; only the `/mcp` alias is stateful (`src/mcp_agent_mail/http.py:1130-1150,1776-1786`). VERIFIED.
   - Pi's HTTP transport keeps and resends `Mcp-Session-Id` (`pi:packages/mcp/src/transports/streamable-http.ts:328,335-337`), but that only helps against `/mcp/`.
2. **Exposure: the decisive Pi-specific trap.**
   - Pi's default server exposure is `codemode`: tools are "neither declared to the model nor listed in the codemode description" and are callable only from codemode scripts (`pi:.../docs/mcp.md:191-198`).
   - Out of the box, a Pi model never sees a `fetch_inbox` tool declaration. It sees one line in the system prompt's `mcp_servers` section, taken from the configured `description` or else from the first line of the server instructions (`mcp.md:71,202`; `pi:packages/coding-agent/src/extensions/mcp/index.ts:184-188` splits on `\n`). The Python instructions string has no newline, so the "first line" is the whole ~270-character string, TOON hint included. VERIFIED.
   - To call `fetch_inbox`, the model must write a codemode script that calls `mcp__<server>__fetch_inbox`, found with `searchTools()`, `describeTool()` or `ALL_TOOLS` (`mcp.md:195`).
   - The AGENTS.md blurb mcp_agent_mail recommends (`README.md:153-188`) names bare tools (`fetch_inbox`, `acknowledge_message`). Under codemode those names do not exist as model tools. INFERRED: expect extra model steps or confusion.
   - Workaround: `toolExposure` set to `direct` for a handful of tools (`mcp.md:206-221`).
   - Declaring all 49 Python tools (`@mcp.tool(` count in `app.py`) direct would put every schema into every request. The server can trim its own surface with `TOOLS_FILTER_ENABLED` and `TOOLS_FILTER_PROFILE=minimal|core|messaging|custom`, which claims "up to ~70%" context reduction (`config.py:125-158`). VERIFIED.
3. **When it fires.** Only when the model decides to call the tool:
   - inside a turn, as a normal tool call;
   - never at turn start;
   - never after an unrelated tool call;
   - never while the agent is idle.

   INFERRED from the absence of any push path. Pi's client handles only `notifications/message`, which goes to `~/.pi/agent/mcp.log` and never to the model, and `tools/list_changed`. It sends only the `roots` capability: no sampling, no elicitation, no resource subscriptions (primer §3). So even a server that pushed could not reach a Pi model.
4. **Payload and bounds on the Pi side.**
   - The result arrives as an MCP tool result: JSON text in a `tool` role message.
   - Pi middle-truncates text results over 20 KB around a `…N chars truncated…` marker and saves the full text to a temp file (`mcp.md:232`).
   - An `include_bodies=true, limit=20` fetch of long messages can therefore lose its middle messages. The model can recover them from the temp file at extra cost. INFERRED.
5. **Dedupe and ACK on Pi.** Whatever the model does. With `unread_only=true` and a diligent `acknowledge_message` after each read, each message is shown once. Without them, every poll re-shows the newest 20. INFERRED.

### Identity on Pi

- Each call names `agent_name` and `project_key` explicitly.
- `_authenticate_agent` accepts a call when the MCP session is already bound to that agent, for example after this session's `register_agent`. Otherwise it requires `registration_token`, or a `MCP_AGENT_MAIL_WINDOW_ID` window-identity match (`app.py:5332-5336,5357-5427`). VERIFIED.
- Bindings are keyed by the MCP session id and held in in-memory dicts with a 24 h TTL (`app.py:5230-5249`). VERIFIED.
- On the default `/api/` URL there is no session id (stateless, see Connection), so no binding survives between calls: every protected call, `fetch_inbox` included, needs `registration_token` or the window-identity fallback. INFERRED from VERIFIED code. Pointing Pi at `/mcp/` instead lets Pi's resent `Mcp-Session-Id` keep the binding. INFERRED.
- Pi reconnects "on the next call" after a drop (`mcp.md:100`), presumably with a fresh `initialize` and session id; a server restart also clears the in-memory bindings. Either way the model must replay its `registration_token`, which then sits in the model-visible transcript. INFERRED.
- The Rust successor's `MESSAGING_SESSION_IDENTITY` exists to keep the token out of the transcript (`rust:README.md:614-656`).
- Pi's per-command `PI_SESSION_ID` env (primer §5) is not used. Nothing in mcp_agent_mail ties an agent name to a Pi session id or a Herdr pane. The only pane key is `$TMUX_PANE` (overview).

## Wake mechanism

**None for Pi, and none in mcp_agent_mail generally.** VERIFIED by absence (overview, "Wake mechanism"): no `tmux`, `send-keys` or MCP push in `src/`.

| Candidate | Status for Pi |
|---|---|
| MCP push | Impossible. The Pi client ignores everything except `notifications/message` (to a log file) and `tools/list_changed` (primer §3). VERIFIED in Pi; the consequence is INFERRED. |
| Signal files | Wired, but nothing consumes them. See the detail below. |
| tmux broadcast loop, or Companion "Message Stacks" | Not in the repo. The Companion is a commercial product (`README.md:47-52,329-333`); closed source is INFERRED. Applied to Pi, see the detail below. |
| Stop-hook continue loop | None. Pi's equivalent would be an extension's `agent_before_settle` handler (primer §2.2); mcp_agent_mail ships none. |
| Long-poll tool | None. `fetch_inbox` returns immediately. Pi would allow a blocking tool for up to the 60 s request timeout, extended by progress notifications (`mcp.md:68`), but that only helps an agent already in a turn. |

**Signal files in detail.**

- With `NOTIFICATIONS_ENABLED=true` (default false), `send_message` calls `emit_notification_signal` (`app.py:5843`).
- That writes `{signals_dir}/projects/<slug>/agents/<agent>.signal` (`storage.py:3656-3679`, default dir `~/.mcp_agent_mail/signals`, `config.py:516`). VERIFIED. The file holds:

  ```python
  signal_data = {"timestamp": ..., "project": project_slug, "agent": agent_name}
  if settings.notifications.include_metadata and message_metadata:
      signal_data["message"] = {"id": ..., "from": ..., "subject": ..., "importance": ...}
  signal_path.write_text(json.dumps(signal_data, indent=2), ...)
  ```

- Nothing consumes it for any harness. VERIFIED by grep (overview).
- It is the only hook point a Pi wake could build on. INFERRED; see "Generic mechanism" below.

**A keystroke broadcast loop applied to Pi.** In Pi, typing plus Enter while busy becomes a **steer** into the current run, not a rejected input (primer §6; `pi:.../docs/usage.md:35,40`). INFERRED consequences:

- A blind broadcast would redirect busy Pi agents mid-task.
- It would splice into any half-typed human draft.
- Both would violate our delivery-contract rule 4.

**Idle/busy detection, draft safety, latency:** none of them exist, because there is no wake. Latency to first sight of a message is unbounded for an idle Pi agent. For a busy one, it is "whenever the model next chooses to poll". INFERRED.

## Setup and installation footprint on Pi

**Python.** Nothing is written for Pi. The user adds the server by hand. INFERRED from `mcp.md:5-20,30-60`. Two ways:

- `pi mcp add agent-mail --url http://127.0.0.1:8765/mcp/` (the stateful endpoint; `/api/` works but cannot keep a session binding), plus `--bearer-token-env-var` when bearer auth is on. This writes `~/.pi/agent/mcp.json`, or `.pi/mcp.json` with `-l`.
- Edit `mcpServers` by hand. Pi accepts the Claude-style shape directly (`mcp.md:44-60,108-110`).

Then add the AGENTS.md blurb (`README.md:153-188`), and set `"exposure"`/`"toolExposure"` so the model can see the tools.

**Rust, OMP only:**

- Writes `.omp/mcp.json` and the active OMP profile's `mcp.json` with `{"mcpServers":{"mcp-agent-mail":{"type":"http","url":"http://127.0.0.1:8765/mcp/"}}}`.
- When bearer auth is on, it adds an `Authorization` header and appends `.omp/mcp.json` to `.gitignore` (`rust:README.md:546-560`).
- `--no-user-config` skips the user file (`rust:README.md:506`).
- `am setup status` is a read-only inventory of OMP's settings authorities (`rust:README.md:506-543,699-712`). VERIFIED in the README.
- On upstream Pi none of these files are read: `.pi/` is not `.omp/`. INFERRED.
- The JSON shape itself is valid for Pi: `type: "http"` is accepted (`mcp.md:78`).

**Footprint summary:**

- One MCP server entry. No extension, no hook, no settings change.
- A long-lived server process (Python `serve-http`, or Rust `am`) on 127.0.0.1:8765.
- A per-project Git archive and SQLite store (overview).

## Token cost

These are estimates (INFERRED); no measurement was taken.

| Event | Cost |
|---|---|
| Per quiet turn, no mail | Zero tokens from mcp_agent_mail beyond the static per-request cost: nothing is injected. |
| Static, `codemode` (Pi default) | One `mcp_servers` system-prompt entry carrying the whole one-line instructions string (~270 chars), about 60-80 tokens, unless a short `description` is configured. |
| Static, `direct` | Each declared tool's schema in every request. 49 tools, several with long docstrings like `fetch_inbox`'s 40-line one, likely cost thousands of tokens per request. A `minimal`/`messaging` tool filter or a short `toolExposure` allowlist is needed. |
| Per poll, empty | One tool call plus a `[]` result: model output tokens for the call and a few input tokens. Under codemode, add the script text and possibly a `searchTools()` round trip. |
| Per message | One `fetch_inbox` round trip; with `include_bodies=true`, the full JSON envelope per message (the docstring fields plus `project_id`, `sender_id`, `thread_id`, `reply_to`, `topic`, `attachments` and `body_md`). Then usually one `acknowledge_message` or `mark_message_read` round trip. Without bodies, a further fetch is needed to read content. So two to three tool round trips per message, each re-reading the full context on the provider side. |
| Polling discipline | If the model follows the README advice to "poll after each editing step", every editing step pays one extra round trip even when the inbox is empty. |

## Limitations and bugs visible for Pi

1. **Wrong config target for upstream Pi.** The Rust installer's "Pi-family" support writes `.omp/` paths only. Nothing configures `.pi/mcp.json` or `~/.pi/agent/mcp.json`. INFERRED.
2. **Default codemode exposure hides the tools.** The model is told, through the AGENTS blurb, to call `fetch_inbox`, but under Pi's default it has no such tool, only a codemode namespace. Neither version documents Pi exposure settings. VERIFIED that neither the Python repo nor the Rust README mentions `exposure`, `codemode` or `toolExposure`; the confusion itself is INFERRED.
3. **No push, no wake.** An idle Pi agent never learns of mail. A busy one learns only by polling voluntarily. No reminder, not even the count-only one, because `check_inbox.sh` targets shell-hook harnesses. VERIFIED absence.
4. **The signal file is lossy, and racy against the reminder hook.** VERIFIED in code: overwrite at `storage.py:3679`, clear at `app.py:9548-9550`. Consequences INFERRED:
   - It is overwritten per send, so only the latest message's metadata survives.
   - It is debounced at 100 ms per agent.
   - Any `fetch_inbox` call clears it, whatever the filters (`app.py:9548-9550`), including a `limit=1` or `urgent_only` fetch, or the hook's own `curl` fetch on another harness.
   - A watcher can therefore miss a pending signal that a filtered read cleared while unread mail remains.
5. **Middle truncation at 20 KB.** Pi truncates large `fetch_inbox` results with bodies in the middle. The server has no per-body cap and no page bound in bytes. INFERRED interaction of VERIFIED facts.
6. **Newest-first ordering with a `limit`.** A burst larger than `limit` hides the oldest unread messages until the newer ones are marked read. Importance does not sort. VERIFIED (overview); the consequence is INFERRED.
7. **Identity is session or token based, not Pi-session based.** On the default stateless `/api/` URL no binding persists at all. After a reconnect the model must re-present its `registration_token`, which puts the credential in the transcript (Python). Two Pi panes in one project share nothing that keeps them apart other than the agent name the model chooses. INFERRED.
8. **A broadcast-keystroke wake (the README's historical zsh loop) would steer busy Pi runs** and splice into human drafts, because of Pi's typed-while-busy semantics (primer §6). INFERRED.

## Generic mechanism: would it work on Pi, and how well?

| mcp_agent_mail surface | Works on upstream Pi? | Quality |
|---|---|---|
| MCP tools over streamable HTTP or stdio | Yes. VERIFIED: transport compatibility (`mcp.md:3,64,78`). | Usable only after setting `direct` exposure for a minimal tool set. Pull only. |
| MCP resources | Yes, through `list_mcp_resources`/`read_mcp_resource` (`mcp.md:236-248`). | Pull only. No subscription. |
| `check_inbox.sh` reminder | No. There is no Pi shell-hook event to attach it to. | An extension could shell out to it from a `tool_result` handler (INFERRED); none exists. |
| Signal file | Not consumed. | It would fit Pi well: see below. |
| tmux keystroke wake | Possible but harmful (busy steer, draft clobber). | Poor. |
| Python `mcp_agent_mail.cli` (`acks pending`, `file_reservations ...`) via Pi's `bash` tool | Yes, as CLI calls (INFERRED; the commands are VERIFIED in `scripts/integrate_claude_code.sh:197-205`). | Pull only. |

**What a proper Pi integration would look like** (INFERRED; mcp_agent_mail does not do this). It would be a roughly 50-line Pi extension, loaded with `pi -e` or installed as a package:

1. **Watch the signal file.** In `session_start`, run `fs.watch` on the directory `~/.mcp_agent_mail/signals/projects/<slug>/agents/`, filtered to `<name>.signal`. This is the pattern of upstream's `file-trigger.ts` example (`pi:packages/coding-agent/examples/extensions/file-trigger.ts:14-41`), except that example watches the file itself; here the server unlinks the file on every `fetch_inbox` (`storage.py:3712-3716`), which would end a watch on the file path. INFERRED.
2. **Act on a change:**
   - If `ctx.isIdle()` and `ctx.ui.getEditorText() === ""`, call `pi.sendMessage({customType:"agent-mail", content:"<count/subject>", display:true}, {triggerTurn:true})`. That is a keystroke-free, draft-safe wake.
   - If busy, call `sendMessage` without a trigger, or append to `tool_result`, for in-turn delivery.
   - Primer §2.3-2.4, §9.
3. **Inject bodies directly.** A `before_agent_start` handler could inject unread bodies, so the model skips the `fetch_inbox` round trip.

Because of limitation 4, a robust version would re-check with `fetch_inbox unread_only=true` instead of trusting the signal file's contents. This is the same shape as our own planned Pi path (primer §9 "Channel-like"). mcp_agent_mail provides the event source for it but not the consumer.

## Clever ideas worth stealing

These are INFERRED judgments. License note from the overview: the MIT+Rider bars copying code, so take ideas only.

- **Server-side tool filtering profiles** (`TOOLS_FILTER_PROFILE=minimal|core|messaging|custom`, `config.py:125-158`). They cut a large surface to the few tools a polling agent needs. For any of our MCP surface, and especially for Pi's `direct` exposure where every schema is paid per request, ship a minimal profile by default.
- **A per-agent signal file as a harness-neutral wake event** (`storage.py:3616-3720`). A file per recipient that any harness-side watcher can consume (Pi extension, inotify daemon) decouples "something arrived" from "how this harness wakes". Their version is lossy. A correct version would be a monotonically increasing counter or cursor rather than overwrite-then-delete-on-any-read.
- **Durable body-free event cursors for monitors** (Rust `fetch_inbox_events` / `am inbox-events`):
  - Oldest first, `next_cursor` persisted only after processing.
  - `cursor_expired` and `cursor_ahead` errors with explicit restart points (`rust:README.md:745-786`).
  - A clean contract for an out-of-process watcher such as a Pi extension. It fixes exactly the lossiness of the signal file.
- **`am setup status`: a read-only drift inventory** of every settings authority that decides whether the MCP entry actually loads. It reports drift instead of a false green when an authority disables project MCP (`rust:README.md:506-543,699-712`). For Pi this would mean checking project trust, `exposure`, `-builtin:mcp` and `pi-mcp-adapter` replacement (`mcp.md:264-268`). All of these can silently make an installed integration invisible.
- **Session-bound identity that keeps tokens out of the transcript** (Rust `MESSAGING_SESSION_IDENTITY`, `rust:README.md:614-656`). It is explicitly cooperative, not authentication, which is consistent with our TRUST-POLICY.
- **Counter-example, not to steal:** pull-only delivery into Pi. Pi's extension API (`sendMessage` with `triggerTurn`, `before_agent_start`, `tool_result`, `isIdle`, `getEditorText`) makes push delivery and draft-safe wake easy in-process. A messaging system that stops at MCP leaves all of that unused.

## Verification

Adversarial check on 2026-10-23 against the clone at `83ed8d7`, pi-mono at `a7b94a0`, the Rust README re-fetched from `main` (2177 lines), the GitHub REST API and the Pi primer.

Checked and confirmed:

- `fetch_inbox` signature, docstring and limit cap, newest-first ordering, signal clear on any fetch.
- `acknowledge_message` semantics, the instructions string, 49 `@mcp.tool(` and 29 `@mcp.resource(` registrations.
- `_truncate_text` and `[:800]` scope, untruncated `body_md`.
- Signal-file write, debounce and default dir, `emit_notification_signal` call site, no consumer, no `tmux`/`send-keys` in `src/`.
- `_authenticate_agent` paths, tool-filter profiles, `/api/` default path.
- `check_inbox.sh` envelope, the auto-detector's agent list, the absence of any Pi/OMP mention.
- README blurb, Companion and overseer lines.
- Rust OMP section, `MESSAGING_SESSION_IDENTITY`, `inbox-events` cursors, `v0.3.39` of 2026-10-13, and oh-my-pi `fork: false` with about 36k stars.
- Pi docs: `mcp.md` exposure, truncation, timeout, reconnect and resource lines, `environment-variables.md:81`, `usage.md` steer semantics, `file-trigger.ts`, transport session-id handling, and the primer sections cited.

Changed:

- Added that the default `/api/` endpoint is stateless and issues no `Mcp-Session-Id`, so session binding cannot persist there. Changed the setup URL to `/mcp/`, rewrote the identity bullets, and added the point to limitation 7. Python bindings being in memory is now VERIFIED, not INFERRED.
- The `mcp_servers` line is the whole ~270-character instructions string, not its first sentence (`index.ts:184-188`), so the codemode static cost estimate went from 15-30 to 60-80 tokens.
- The actual `fetch_inbox` row is wider than its docstring, which changed the payload and token-cost rows.
- Under default exposure, Pi's resource tools are codemode-only as well.
- Softened codemode discovery: `searchTools()`, `describeTool()` or `ALL_TOOLS`.
- The Companion is now described as commercial, with closed source marked INFERRED.
- The extension sketch now watches the directory, because the server unlinks the signal file.
- Fixed citations: `check_inbox.sh` lines, `mcp.md:202`, `streamable-http.ts:335-337`.
