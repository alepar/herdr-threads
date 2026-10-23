# mcp_agent_mail x Goose (Block)

- Alternative: https://github.com/Dicklesworthstone/mcp_agent_mail (Python), clone at
  `83ed8d7c97af67f1e6b4700bb8ace08739171ed3`. Paths written as `am:<path>:<line>` are relative to that clone's
  root. The code was read and never run. Short forms: `am:app.py`, `am:config.py`, `am:storage.py`, `am:cli.py`
  are under `src/mcp_agent_mail/`; `am:check_inbox.sh` is `scripts/hooks/check_inbox.sh`.
- Successor: https://github.com/Dicklesworthstone/mcp_agent_mail_rust. Only its README was read (raw `main`,
  2026-10-23); paths written as `rust:README.md:<line>`.
- Harness: Goose. Paths written as `goose@1.36:<path>:<line>` are in the v1.36.0 tree described in
  `../../primers/goose.md` (the primer), which matches the installed binary. Primer sections are cited as
  "primer §N".
- Tags: VERIFIED means read in source or docs. INFERRED means my reading, not stated by the source. "unknown"
  means not determined.

## Support status

**Not supported. Goose appears nowhere in either project.** VERIFIED:

- `grep -rniI goose` over the whole Python clone finds no match.
- The integration scripts are `integrate_{claude_code,cline,codex_cli,cursor,factory_droid,gemini_cli,github_copilot,opencode,windsurf}.sh`;
  there is no Goose script (`ls am:scripts/`).
- The auto-detect installer names only `~/.claude, ~/.codex, ~/.cursor, ~/.factory, ~/.gemini` plus a
  best-effort OpenCode probe (`am:scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh:21,126`).
- The Rust README lists "Claude Code, Codex CLI, Gemini CLI, Oh My Pi (OMP), GitHub Copilot CLI, and any
  MCP-compatible client" (`rust:README.md:17`). `grep -ni goose` over the Rust README finds no match.

So Goose falls under "any MCP-compatible client". The rest of this report asks how well that generic path
works under Goose, using the primer for Goose's behaviour.

**Verdict, INFERRED:** the mailbox works, delivery is pull-only, and there is no wake.

- Goose can mount the server as a Streamable HTTP (or stdio) extension and call every tool.
- No message content and no reminder ever reaches the model unless the model itself calls `fetch_inbox`
  (or reads a `resource://` view). Whether it does depends entirely on the AGENTS.md blurb and the model's
  diligence.
- The upstream reminder hook (`check_inbox.sh`) cannot be reused as-is: Goose hooks have no
  `additionalContext` field, so its output never reaches the model (primer §3.4).
- An idle `goose session` stays idle forever. Nothing upstream types into a pane, and the optional `.signal`
  files have no consumer.

## Injection mechanism

### What can get into Goose's context, and how

| Path | Reaches the model? | When | Evidence |
|---|---|---|---|
| Model calls `fetch_inbox` (MCP tool) | yes, as a tool result | only when the model decides to call it | `am:src/mcp_agent_mail/app.py:9446-9554` VERIFIED |
| Model calls `read_resource` on `resource://inbox/...`, `resource://views/urgent-unread/...` etc. | yes, as a tool result of Goose's platform `read_resource` tool | only when the model calls it | `am:app.py:13980,14099,14170` VERIFIED; Goose resource access is model-pull only (primer §6) VERIFIED |
| MCP server `instructions` | yes, in the system prompt, once per session | session start | server side `am:app.py:5221-5228` VERIFIED; Goose puts extension instructions into the system prompt, `goose@1.36:crates/goose/src/agents/extension_manager.rs:1077-1079`, `prompt_manager.rs:132` VERIFIED |
| AGENTS.md blurb (`am:README.md:153-188`) | yes, if the user pastes it into AGENTS.md | session start | Goose loads `AGENTS.md` by default as a hints file, `goose@1.36:crates/goose/src/hints/load_hints.rs:11,17-21` VERIFIED |
| `ctx.info("Fetched N messages ...")` log notification from `fetch_inbox` | **no**, user display only | during the tool call | emit `am:app.py:9551-9554` VERIFIED; Goose forwards `notifications/message` to the UI only (primer §6) VERIFIED |
| `check_inbox.sh` reminder as a Goose `PostToolUse` hook | **no** | after a tool call | Goose hook output has no context field (primer §3.4) VERIFIED; consequence INFERRED |
| `.signal` file | **no** (no consumer) | on send | `am:src/mcp_agent_mail/storage.py:3616-3700` VERIFIED |

The only content path is therefore the model's own tool call. There is nothing in-turn or push-like.

### The tool call itself

`fetch_inbox(project_key, agent_name, limit=20, urgent_only=False, include_bodies=False, since_ts=None,
topic=None, unread_only=False, registration_token=None, format=None)` (`am:app.py:9453-9466`, VERIFIED).

- **Payload:** a JSON list, each item `{ id, subject, from, created_ts, importance, ack_required, kind,
  read_at, [body_md] }` (`am:app.py:9492-9498`, VERIFIED). `format='toon'` returns `{format:'toon',
  data:'<TOON>'}` instead (`am:app.py:5221-5226`, VERIFIED).
- **Bounds:** `limit` defaults to 20 and is capped at 1000 (`am:app.py:9510-9512`, VERIFIED). Bodies are
  opt-in. No body truncation was found (unknown, as in the overview). On top of that, Goose's
  `large_response_handler` post-processes every tool result
  (`goose@1.36:crates/goose/src/agents/agent.rs:427-428`, VERIFIED); its thresholds were not read (unknown).
- **Order:** newest first, `.order_by(desc(Message.created_ts))` in `_list_inbox` (`am:app.py:4628`, VERIFIED). Importance is a filter, not
  a sort key.
- **Framing:** none beyond the JSON fields. Overseer messages carry the emoji preamble inside `body_md`
  (`am:README.md:457-479`, VERIFIED).
- **De-dup:** `fetch_inbox` "does NOT mark messages read" (`am:app.py:9476-9482`, VERIFIED). A model that
  polls without `unread_only=true` re-reads the same mail every time. De-dup is the model's job: it must call
  `mark_message_read` or `acknowledge_message`.
- **ACK:** `acknowledge_message` is a separate tool call per message (overview, VERIFIED there).

### Identity inside Goose

- Authentication of `fetch_inbox` goes through `_authenticate_agent`. If the MCP session is already bound
  to the agent (by an earlier `register_agent` / `create_agent_identity` in the same session) no token is
  needed; otherwise a `registration_token` is required (`am:app.py:5357-5430`, VERIFIED).
- The session key is `ctx.session_id`, "derived from the `mcp-session-id` header for HTTP transport, or a
  generated UUID stored on the in-memory session for stdio" (`am:app.py:5230-5240`, `5263-5287`, VERIFIED).
  INFERRED: a Goose Streamable HTTP extension keeps one MCP session per Goose agent process, so a single
  `macro_start_session` at session start binds the identity for the rest of that process. A Goose restart
  or `goose session -r` gets a new MCP session, and the model must re-register or present the token, which
  then sits in the transcript. Bindings are in server memory and also expire after
  `MCP_AGENT_MAIL_SESSION_BINDING_TTL_SECONDS` (default 86400, minimum 60) without use
  (`am:config.py:599`, `am:app.py:5244-5246`, VERIFIED).
- The fallback rebind is `MCP_AGENT_MAIL_WINDOW_ID`, a **server** setting (`am:src/mcp_agent_mail/config.py:597`,
  `am:app.py:5403-5413`, VERIFIED). INFERRED: it is usable per agent only if each Goose session spawns its own
  stdio server (`serve-stdio`, `am:src/mcp_agent_mail/cli.py:1245-1256`) with that env in the extension's
  `envs`. With a shared HTTP server it is one value for everyone.
- The pane identity helpers take a pane id as an optional argument, default to `$TMUX_PANE`, and refuse to
  run when neither is given (`am:scripts/identity-resolve.sh:130-132`, `identity-write.sh:56-59`, VERIFIED).
  In a Herdr pane with no tmux, nothing upstream supplies the id, so the path is dead unless a user passes one
  by hand. INFERRED.
- Goose passes `AGENT_SESSION_ID` to stdio extensions (primer §6, §11). mcp_agent_mail does not read it
  (grep finds no `AGENT_SESSION_ID` in `am:src/`, VERIFIED). So Goose's own session id plays no part.

### Could the upstream reminder hook be ported to Goose?

The Claude/Codex reminder is a `PostToolUse(Bash)` hook that curls `fetch_inbox` with
`unread_only=true, include_bodies=false, limit=10`, rate-limited to once per 120 s, and prints a
`hookSpecificOutput.additionalContext` envelope when `AGENT_MAIL_HOOK_FORMAT=json` (the script defaults to a
plain-text banner; both integration scripts set `json`: `am:scripts/integrate_claude_code.sh:180`,
`am:scripts/integrate_codex_cli.sh:222`) (`am:scripts/hooks/check_inbox.sh:47-48,66-80,95-110,160-172`,
VERIFIED):

```sh
printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":%s}}\n' ...
```

Under Goose, INFERRED from the primer:

- Goose would run it: hooks are `sh -c` commands in a plugin's `hooks/hooks.json`, payload on stdin, env
  inherited (primer §3.1, §3.3). The matcher would have to be retargeted: in 1.36 the developer platform
  extension is registered with `unprefixed_tools: true`, so its shell tool is exposed as plain `shell`
  (`goose@1.36:crates/goose/src/agents/platform_extensions/mod.rs:154-158`,
  `extension_manager.rs:1277-1285`, `platform_extensions/developer/mod.rs:124`, VERIFIED), and a `Bash` regex
  does not match it. INFERRED that the hook's `tool_name` is that public name.
- The output would be discarded. Goose's hook decision parser reads only `decision`/`reason` (and `banner` on
  main); there is no `additionalContext` (primer §3.4, VERIFIED there). Plain stdout is not shown to the model
  either.
- **A working port would need a different sink:** write the reminder text into the file named by
  `GOOSE_MOIM_MESSAGE_FILE`, which Goose's default-on `tom` platform extension re-reads before every provider
  call (primer §5, VERIFIED there). `PostToolUse` is awaited inside the tool-result future before the result
  is returned (`goose@1.36:agent.rs:436-440`, VERIFIED), so a hook that rewrites the MOIM file lands in the
  very next provider request. INFERRED composition; nothing upstream does this. The env var must be in Goose's
  environment at launch on 1.36 (primer §5).
- Side effect, VERIFIED by composition: when `NOTIFICATIONS_ENABLED=true`, every `fetch_inbox` clears the
  recipient's `.signal` file (`am:app.py:9548-9550`). The reminder hook's own count-only curl is a
  `fetch_inbox`, so a reminder poll silently consumes the wake signal even though the model has read nothing.

## Wake mechanism

**None.** VERIFIED by absence, as in the overview: no `tmux`, `send-keys`, MCP `notifications/` push or
resource subscription occurs in `am:src/` apart from docstrings.

What exists, and what it would mean for Goose:

- **Signal files** (opt-in, `NOTIFICATIONS_ENABLED`, default false; `am:config.py:515-518`, VERIFIED).
  `send_message` writes `{signals_dir}/projects/<slug>/agents/<agent>.signal` with
  `{timestamp, project, agent, message:{id, from, subject, importance}}` (`am:storage.py:3654-3676`,
  VERIFIED). The docstring says agents "can watch these files using inotify/FSEvents/kqueue"
  (`am:config.py:162-166`, VERIFIED). Nothing ships that watches them. A Goose wake built on them would have to
  be a user-written watcher that types into the pane.
- **Upstream's suggested wake** is a human, a "historical zsh loop" that broadcasts prompts into tmux panes
  (not in the repo), or the commercial Companion app (`am:README.md:47-52,329-333`, VERIFIED). How either works is
  unknown.
- **Goose-native options upstream does not use** (primer, INFERRED applicability):
  - MCP server notifications cannot wake Goose: notifications are subscribed only during that server's tool
    call and go to the UI (primer §6). Even if mcp_agent_mail pushed, Goose would not act.
  - ACP `session/prompt` and main-only `steer` would be clean wake and mid-turn paths, but only for a Goose
    hosted by `goose serve`/`goose acp`, not a `goose session` in a terminal pane (primer §7).
  - `goose run -r --session-id ID -t ...` re-launch creates a second, unsynchronised agent on a conversation
    that a pane may still hold (primer §8).
  - Keystroke injection into the rustyline `> ` prompt is the only way to wake a `goose session` pane
    (primer §9).

**Idle/busy detection:** none upstream (nothing types). Goose's CLI exposes no machine-readable state; a
DIY watcher would need screen reading or a `UserPromptSubmit`/`Stop` hook flag (primer §10). INFERRED.

**Draft safety:** none upstream. For a DIY keystroke wake in Goose (primer §9, VERIFIED there):
Ctrl+C with text in the buffer deletes the human's draft and a second Ctrl+C on an empty line exits the
session; keys typed while busy sit in the tty and become the next prompt after the turn; injected keys
during a tool-approval or elicitation select would answer it. The signal file gives no hint about any of
this.

**Latency:** unbounded for an idle Goose (no wake). For a busy Goose, as soon as the model chooses to call
`fetch_inbox`, which upstream never forces. INFERRED.

**Failure modes, INFERRED:**

- A message to an idle Goose waits until a human types something and the model then happens to poll.
- A model that forgets the blurb never sees mail at all; there is no reminder fallback on Goose.
- A restarted Goose session loses its MCP session binding and gets `AUTHENTICATION_REQUIRED` on
  `fetch_inbox` until it re-registers (the hook path would fail silently, `am:check_inbox.sh:113-120`).

## Setup and installation footprint on Goose

Upstream writes nothing for Goose. VERIFIED (no script). A manual setup would be, INFERRED:

- Server: `uv run python -m mcp_agent_mail.cli serve-http` on `127.0.0.1:8765` with a bearer token
  (overview, VERIFIED there), or `serve-stdio` per Goose session.
- Goose extension in `~/.config/goose/config.yaml` (or `--with-streamable-http-extension URL` per run):
  a `streamable_http` extension with `uri`, `headers: {Authorization: "Bearer ..."}`, `timeout`
  (`DEFAULT_EXTENSION_TIMEOUT = 300` s; the field itself is optional) and optionally `available_tools` to trim the tool list
  (`goose@1.36:crates/goose/src/agents/extension.rs:221-248,405-415`,
  `goose@1.36:crates/goose/src/config/extensions.rs:11`, VERIFIED field names).
- The AGENTS.md blurb pasted by hand (`am:README.md:153-188`).
- Optional server-side tool trimming: `TOOLS_FILTER_ENABLED=true`, `TOOLS_FILTER_PROFILE=core|minimal|messaging|custom`,
  advertised as "reducing context overhead by up to ~70%" (`am:config.py:130-154`, VERIFIED).
- Optional (my port, not upstream): a Goose plugin under `.agents/plugins/<name>/hooks/hooks.json` that runs a
  reminder script and writes `GOOSE_MOIM_MESSAGE_FILE` (primer §3.1, §5).

## Token cost

Estimates are INFERRED from byte counts at about 4 bytes per token.

- **Per session (fixed):**
  - The AGENTS.md blurb is 2,925 bytes (`sed -n 153,188p README.md | wc -c`), about 730 tokens, paid in every
    Goose session in that repo.
  - Server instructions are about 300 bytes (`am:app.py:5221-5226`), about 75 tokens, and they say nothing
    about checking the inbox.
  - Tool schemas: the Python server registers 51 `@mcp.tool` handlers (`grep -c` on `am:app.py`), about 40
    unique names visible at module level. Their schema size was not measured (unknown); Goose resends the
    tool list with every provider request, so this is the dominant fixed cost unless `TOOLS_FILTER` or
    Goose's `available_tools` trims it. INFERRED.
- **Per message:** one model tool call to `fetch_inbox` (a few dozen tokens of arguments), a result of about
  150-250 bytes of metadata per item without bodies, then usually a second call with `include_bodies=true`
  or `resource://message/...` for the body, plus a separate `acknowledge_message` call when `ack_required`.
  Overseer messages add a preamble of about 400 bytes per message (`am:README.md:461-476`). INFERRED.
- **Per quiet turn:** zero injected bytes, because nothing injects. The real cost is a model that polls
  `fetch_inbox` "after each editing step", as the docstring recommends (`am:app.py:9484-9490`, VERIFIED): each
  empty poll is a tool round trip with a `[]` result. INFERRED.

## Limitations and bugs visible from here

- **No Goose integration at all**, despite Goose being a mainstream MCP client. VERIFIED.
- **The reminder design assumes a hook context field** (`additionalContext`) that Goose does not have. Any
  Goose port must find another sink. VERIFIED on the Goose side (primer §3.4).
- **Reminder polls consume the wake signal.** Every `fetch_inbox`, including the hook's count-only curl or a
  `topic`-filtered / `limit=1` call, deletes the `.signal` file unconditionally (`am:app.py:9548-9550`,
  VERIFIED). A watcher would miss mail that was counted but not read. INFERRED consequence.
- **Signal debounce drops metadata.** Signals within 100 ms of the previous one are skipped, so the file keeps
  the first message's id/subject/importance (`am:storage.py:3646-3654`, VERIFIED). An urgent message that
  follows a normal one inside the window is not reflected. INFERRED consequence.
- **Signal docstring and code disagree on the path:** `{signals_dir}/{project_slug}/{agent_name}.signal`
  (`am:config.py:168`) vs `projects/<slug>/agents/<agent>.signal` (`am:storage.py:3659,3710`). VERIFIED.
- **Identity is per MCP session or token**, and the pane helpers need `$TMUX_PANE`. A Goose in a Herdr pane
  without tmux has no pane identity, and restarts need the token in the transcript. VERIFIED / INFERRED as
  marked above.
- **Rate-file collisions:** `check_inbox.sh` keys its rate limit on `/tmp/mcp-mail-check-<agent>`
  (`am:check_inbox.sh:67`, VERIFIED), so two harnesses using the same agent name throttle each other.
  INFERRED.
- **Newest-first with a limit** means a flood can hide older unread mail beyond `limit` (overview). INFERRED.

## Clever ideas worth stealing

- **Count-only reminder with a "this stops when you read it" clause**
  (`am:check_inbox.sh:162-164`, VERIFIED). For Goose, where we cannot cheaply put bodies in context on every
  harness version, a one-line pending-count marker in the MOIM file is a cheap fallback that tells the model
  why it should run `herdr-threads inbox`. Our R17/R18 rules would want an id-based marker rather than prose.
  INFERRED.
- **Server-side tool profiles** (`TOOLS_FILTER_PROFILE`, `am:config.py:130-154`) to keep schemas small for a
  harness that resends the tool list each request. If we ever ship an MCP surface for Goose, a minimal
  profile matters. INFERRED.
- **MCP session binding instead of a token in every call** (`am:app.py:5325-5336`), and the Rust successor's
  refusal of "borrowed names" (`rust:README.md:612-656`). For Goose, binding by the Goose session id (exported to
  stdio extensions as `AGENT_SESSION_ID` and passed to hooks as `session_id` in the payload, primer §3.3, §6) plus `HERDR_PANE_ID` is the analogue. INFERRED.
- **Durable body-free delivery cursor for monitors** (Rust `fetch_inbox_events` / `am inbox-events`, oldest
  first, `next_cursor` persisted only after processing, `rust:README.md:742-762`, VERIFIED). That is the right
  shape for a watcher that decides when to wake, and it avoids the signal-file deletion race above.
  INFERRED.
- **What not to copy:** the signal file as the wake contract (no consumer, lossy debounce, cleared by any
  read), and the per-message overseer preamble. The license rider also rules out copying code
  (overview, License).

## Net for herdr-threads

INFERRED. mcp_agent_mail offers Goose nothing beyond a generic MCP mailbox. For our own Goose adapter the
useful lessons are negative: a pure pull model on Goose means no wake and model-dependent delivery. The
primer's candidates (MOIM file for in-turn delivery, a Native keystroke wake with a goose composer reader that
never sends Ctrl+C) remain the only routes for a `goose session` in a Herdr pane, and this alternative
explores neither.

## Verification

Adversarial pass on 2026-10-23 against the clone at `83ed8d7`, the Rust README (raw `main`), the primer and the
`goose-1.36` tree (`f13f369`) the primer cites.

Checked and confirmed: no `goose` string in the Python clone or the Rust README; the integration-script list and
auto-detect targets; `fetch_inbox` signature, payload fields, limit cap, "does NOT mark read", poll docstring,
signal clearing and `ctx.info` (`am:app.py:9446-9554`); server instructions and session-key comment
(`am:app.py:5221-5240`); `_authenticate_agent` and the window-id fallback (`am:app.py:5357-5430`); resource URIs
(`am:app.py:13980,14099,14170`); 51 `@mcp.tool` sites, 40 unique names; signal settings, debounce, path mismatch
(`am:config.py:162-168,515-518`, `am:storage.py:3612-3715`); `serve-stdio`; no `AGENT_SESSION_ID`, `tmux`
send-keys or `notifications/` push in `src/`; `check_inbox.sh` args, rate file, interval 120 s, envelope and
error exit; blurb 2,925 bytes; tool-filter profiles; Rust README lines 17, 612-656, 742-762; Goose
extension instructions into the system prompt (`extension_manager.rs:1077-1079`, `prompts/system.md:23-24`),
AGENTS.md default hints, `large_response_handler`, awaited `PostToolUse`, `streamable_http` fields,
`DEFAULT_EXTENSION_TIMEOUT = 300`.

Changed: the Goose 1.36 shell tool is `shell`, not `developer__shell` (developer extension is unprefixed);
`check_inbox.sh` emits the JSON envelope only with `AGENT_MAIL_HOOK_FORMAT=json`; identity helpers accept an
explicit pane id; the order claim now cites `am:app.py:4628` directly; "closed" Companion app is "commercial";
overseer preamble is about 400 bytes, not 500; hooks receive the session id in the payload, not as
`AGENT_SESSION_ID`; added the session-binding TTL; added a key for the short `am:` paths.
