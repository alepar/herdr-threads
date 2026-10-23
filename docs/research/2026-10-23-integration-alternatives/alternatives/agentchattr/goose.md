# agentchattr x Goose (Block / AAIF)

- Alternative: agentchattr, <https://github.com/bcurts/agentchattr> (MIT), clone `scratchpad/alts/agentchattr`,
  HEAD `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10), `VERSION` 0.5.0. 1558 stars / 239 forks, latest
  release v0.5.0 (2026-07-26) (GitHub API, recorded in `overview.md`).
- Harness: Goose; source trees `goose@1.36` (tag v1.36.0, matches the installed binary) and `goose@main`
  (c3bfc5f, 1.56.0), as described in `primers/goose.md`. Paths are relative to each clone root.
- Tags: VERIFIED = read in source or docs at the cited place; INFERRED = my reasoning; "unknown" = not determined.
  No agentchattr or goose code was executed.

## 1. Support status

**Not supported. There is no Goose integration of any kind.** VERIFIED:

- `grep -rni goose` over the whole agentchattr clone returns nothing (no config entry, launcher, test, README line).
- The shipped agent table in `config.toml:10-94` lists claude, codex, gemini, antigravity, kimi, qwen, kilo,
  codebuddy, copilot and minimax (API). Launchers exist only for those plus a generic `start_api_agent`
  (`macos-linux/start_*.sh`, `windows/start_*.bat`).
- Built-in MCP-injection defaults exist only for `claude`, `gemini`, `codex`, `kimi`, `kilo`
  (`wrapper.py:131-161`); `_provider_from_command` resolves only those exact executable names (`wrapper.py:166-180`).

What a user can still do is declare a generic agent in `config.toml` or `config.local.toml` (`config_loader.py:104-125`
merges `[agents]` from the local file). The README claims "Any MCP-compatible agent can participate — no special
integration needed" (`README.md:297`). The rest of this report works out what that generic path gives Goose, using
the primer. Summary: **it can work, through one specific `mcp_inject` mode (`proxy_flag`) and goose's
`--with-streamable-http-extension` flag. The wake works for an idle `goose session`. For a busy one it probably
fails silently, because of how goose binds Ctrl+J.**

### 1.1 Which `mcp_inject` mode fits Goose

agentchattr's wrapper has five ways to get its MCP server into the CLI (`wrapper.py:163`, `_apply_mcp_inject`
`wrapper.py:206-319`). Each one checked against Goose:

| Mode | What it writes | Fits Goose? |
|---|---|---|
| `settings_file` | merges `{"mcpServers": {"agentchattr": {"type":"http", <http_key>: url, "trust": true, headers}}}` JSON into a path, plus `security.folderTrust.enabled=true` (`wrapper.py:38-82`, `234-252`) | **No.** Goose's user config is YAML (`~/.config/goose/config.yaml`, `extensions:` map). Goose does not read a `.mcp.json`-style `mcpServers` file at a project or user path: a grep of `goose@1.36/crates` for `.mcp.json` finds nothing. VERIFIED. On main, `.mcp.json` is read only inside an Open Plugins plugin directory, and its schema needs `command` (stdio only, `goose@main crates/goose/src/plugins/mcp_servers.rs:12-30`), so an HTTP URL cannot be expressed there. VERIFIED. |
| `env` | the same JSON in `data/provider-config/<name>-settings.json`, path exported in `mcp_env_var` (`wrapper.py:254-287`) | **No.** Goose has no environment variable that points at an MCP settings JSON (none appears in the primer's survey). INFERRED/unknown. Side effect: `inject_cfg["mcp_inject"] == "env"` also triggers `_ensure_gemini_folder_trusted`, which writes `~/.gemini/trustedFolders.json` (`wrapper.py:737-738`, `322-353`). VERIFIED. |
| `flag` | a Claude-shaped `{"mcpServers": ...}` file passed as `<mcp_flag> <path>` (`wrapper.py:289-298`) | **No.** Goose has no flag that takes an MCP config file (observed `goose session --help` in the primer: only `--with-extension`, `--with-streamable-http-extension`, `--with-builtin`). VERIFIED via primer section 2. |
| `env_content` | the env var holds `{"mcp": {"agentchattr": {"type":"remote", "url":..., "headers":...}}}` (Kilo/OpenCode shape, `wrapper.py:300-310`) | **No.** Goose reads no such variable. INFERRED/unknown. |
| `proxy_flag` | starts a per-instance localhost identity proxy, then `template.format(server=, url=<proxy>)` split on whitespace becomes argv (`wrapper.py:312-317`, `641-661`) | **Yes.** `goose session --with-streamable-http-extension URL` exists in both trees (`goose@1.36 crates/goose-cli/src/cli.rs:169-178`, `goose@main cli.rs:196`). VERIFIED. |

A working generic entry would therefore be (INFERRED, not shipped, not run):

```toml
[agents.goose]
command = "goose"
cwd = ".."
label = "Goose"
mcp_inject = "proxy_flag"
mcp_proxy_flag_template = "session --with-streamable-http-extension {url}"
```

This works because the wrapper builds argv as `[*mcp_args, *extra_args]` (`wrapper.py:382`), so `session` lands
first. A bare `goose` runs `handle_default_session()` with `streamable_http_extensions: Vec::new()`
(`goose@1.36 crates/goose-cli/src/cli.rs:2189`, `2000-2025`), and the extension flags belong to the `session` and
`run` subcommands only. VERIFIED. The template split is a plain `str.split()`, so the URL must not contain spaces,
and it does not (`http://127.0.0.1:<ephemeral>/mcp`).

The flag cannot carry headers: `parse_streamable_http_extension` builds `ExtensionConfig::StreamableHttp` with
`headers: HashMap::new()` (`goose@1.36 crates/goose-cli/src/session/mod.rs:320-350`). VERIFIED. That rules out the
direct bearer-token modes anyway. The proxy is the right fit: it forwards the instance token as `Authorization:
Bearer` and stamps `sender`/`name` into `tools/call` arguments (`mcp_proxy.py:1-24`, `281-310`). VERIFIED.

If no `mcp_inject` is configured, `needs_proxy` is true (`wrapper.py:639`), so the proxy starts. But
`_apply_mcp_inject` returns no args for an empty mode (`wrapper.py:221-223`), so goose would launch with no
agentchattr extension at all, and the model could not call `chat_read` / `chat_send`. VERIFIED code path;
INFERRED outcome. The `README.md:313-323` manual route (`.mcp.json` in the project root) does not work for goose,
because goose does not read that file. VERIFIED absence, see above.

## 2. Injection mechanism

Under the generic entry, injection is identical to every other CLI agent. It happens in two steps, and Goose adds
nothing of its own.

### 2.1 Step 1: a wake prompt typed into the TUI (carries metadata, not message bodies)

- Server side: an @mention calls `AgentTrigger.trigger`, which appends one JSON line `{sender, text, time, channel,
  [prompt], [job_id]}` to `data/<name>_queue.jsonl` (`agents.py:32-54`). VERIFIED.
- Wrapper side: `_queue_watcher` polls once per second. When the file is non-empty it reads every line, then
  truncates the file (`wrapper.py:480-486`). It then builds one prompt for the whole batch (`wrapper.py:526-531`):

  ```python
  prompt = f"use mcp to read #{channel} - you're mentioned, take appropriate action and respond"
  ```

  or the `job_id=` variant, or a session-engine `prompt` that replaces it entirely. VERIFIED.
- Text the wrapper appends (`wrapper.py:533-563`), VERIFIED:
  - `ROLE: <role>` when a role is set. The role is fetched over HTTP on every trigger.
  - `RULES: <rule1>; <rule2>...` on the first trigger, when the rules epoch changes, or every `refresh_interval`
    triggers (default 10, configurable on the server). The wrapper then reports the epoch it synced to.
  - A one-time `_IDENTITY_HINT` (239 chars, `wrapper.py:422-427`) for instances with `slot > 1`
    (`wrapper.py:800`, `561-563`).
- LF is replaced by a space before typing (`prompt.replace("\n", " ")`, `wrapper.py:564-567`); CR is not
  touched. VERIFIED. For Goose the LF flattening matters (INFERRED, good): an embedded LF would be read as Ctrl+J,
  which goose binds to "insert newline". A CR inside a role or rule would still submit early (INFERRED; the
  wrapper's own text contains none).
- No bound. The wake prompt grows with the active rules text, and the role and rules are not truncated.
  VERIFIED (no length check in `wrapper.py:533-567`).

### 2.2 Step 2: the model pulls content over MCP

- In Goose the agentchattr server appears as a streamable-HTTP extension named from the URL:
  `name_to_key("127.0.0.1_<port>_mcp")` (`goose@1.36 crates/goose-cli/src/session/mod.rs:321-340`). VERIFIED.
  Tools are prefixed `<ext>__<tool>`, and goose strips the prefix before dispatch (`goose@1.36
  crates/goose/src/agents/extension_manager.rs:1604`). VERIFIED. So the server and the proxy's
  `_SENDER_PARAMS` lookup (`mcp_proxy.py:39-49`) see plain `chat_read`. INFERRED from those two reads. The model
  sees something like `127_0_0_1_54321_mcp__chat_read`. The wake text "use mcp to read #general" still points it
  there clearly enough (INFERRED). Because the proxy port is ephemeral (`port: int = 0`, `mcp_proxy.py:84`), the
  extension name changes on every wrapper restart, which is cosmetic for a fresh session. Its effect on a
  `goose session -r` resume that recorded the old extension is unknown.
- `chat_read(sender, since_id, limit=20, channel, job_id)` (`mcp_bridge.py:575-689`), VERIFIED:
  - Keeps a per-sender, per-channel cursor. The first read returns the last `limit` messages; later reads return
    `get_since(cursor)` and then `msgs[-limit:]`, and the cursor advances to the last returned id
    (`mcp_bridge.py:648-663`). **If more than 20 messages arrived, the older ones are skipped silently** (cursor
    moves past them).
  - Returns a JSON array (id, sender, text, type, time, channel, attachments, reply_to). The body of each message
    is not truncated (`_serialize_messages`, `mcp_bridge.py:409-426`).
  - On an empty read it returns escalating anti-poll text ("Do not poll", then "STOP. Repeated empty reads waste
    tokens") (`mcp_bridge.py:666-679`).
  - For multi-instance agents it prepends `[identity: name | label: ...]` (`mcp_bridge.py:682-689`).
- The behavioural contract is the FastMCP server `instructions` string (`mcp_bridge.py:54-124`, 5,736 chars as
  evaluated; the source lines are 6.3 KB; passed at `mcp_bridge.py:950-955`).
  **Goose puts every extension's MCP instructions into its system prompt**: `get_extensions_info` maps each
  extension's `get_instructions()` into `ExtensionInfo` for the prompt manager (`goose@1.36
  crates/goose/src/agents/extension_manager.rs:1069-1082`, `prompt_manager.rs:37,52,62-67`). VERIFIED. The
  "always respond in chat, not in the terminal" rule therefore reaches a goose model the same way it reaches
  Claude.
- When it fires: only when the model decides to call `chat_read`, which in practice means right after a wake.
  No goose surface pushes content (VERIFIED in the primer: hooks have no context field, section 3.4; MCP
  notifications reach only the UI, section 6). agentchattr uses neither MOIM nor Stop-block either: grep for
  `hook`/`MOIM` finds nothing (VERIFIED). **A busy goose gets no mid-turn delivery** unless the model happens to
  call `chat_read` on its own. INFERRED.
- Dedupe/ack: dedupe comes only from the server-side cursor. There is no acknowledgement. Coalescing happens at
  the wrapper: N triggers read in one poll produce one wake, and the last line's channel/job/prompt wins
  (`wrapper.py:488-531`), so a mention in a second channel within the same second loses its channel hint.
  VERIFIED code; INFERRED consequence.

## 3. Wake mechanism

### 3.1 Exact mechanism

The wrapper owns a dedicated tmux session `agentchattr-<name>` running `env -u CLAUDECODE [VAR=val] goose
session --with-streamable-http-extension <proxy>`. It runs `kill-session` first, then `new-session -d`, then
attaches (`wrapper_unix.py:204-255`, `wrapper.py:890`). VERIFIED. Injection (`wrapper_unix.py:97-136`), VERIFIED:

```python
tmux load-buffer -b <buf> -          # prompt bytes on stdin
tmux paste-buffer -p -d -b <buf> -t <pane>   # bracketed only if the app enabled DECSET 2004
time.sleep(max(delay, len(text) * 0.001))     # inject_delay default 0.3 s
tmux send-keys -t <pane> Enter
```

Each call has a 5 s timeout. A failure is printed and not retried (`wrapper_unix.py:64-94`).

### 3.2 Idle goose (empty `> ` prompt): works

VERIFIED in rustyline 18.0.0 source (crate tarball from crates.io, read only): goose builds its editor config
without touching bracketed paste (`goose@1.36 crates/goose-cli/src/session/mod.rs:546`), and rustyline's default is
`enable_bracketed_paste: true` (`rustyline src/config.rs:266`), written as `ESC[?2004h` on entering raw mode
(`src/tty/unix.rs:1448-1462`). Raw mode clears `ICRNL` (`src/tty/unix.rs:1616-1619`), so the CR from
`send-keys Enter` arrives as `\r` and maps to `Enter` (`src/keys.rs:56`), which submits. The pasted text lands
in the buffer as one block and goose submits it as a user message. The model then calls the extension's
`chat_read`. With the default `GOOSE_MODE=auto` (`goose@1.36 crates/goose/src/config/goose_mode.rs:24-34`,
VERIFIED via primer) there is no approval prompt for that call.

### 3.3 Busy goose (mid-turn): probably a stuck, unsubmitted draft

Mostly VERIFIED in code; the end-to-end outcome still needs a pty experiment. This is the most important
Goose-specific finding.

1. While a turn runs, the goose CLI does not read the terminal; only a Ctrl+C signal task is active (primer section
   9, `goose@1.36 crates/goose-cli/src/session/mod.rs:1086-1092`, VERIFIED). On leaving readline, rustyline
   restores the original termios and writes `ESC[?2004l` (`rustyline src/tty/unix.rs:114-123`, VERIFIED), so the
   tty is in cooked mode with bracketed paste off. The paste therefore arrives as plain bytes, is echoed into
   goose's output, and is queued by the line discipline.
2. `send-keys Enter` sends CR. With `ICRNL` set in the pane's original termios (the usual default; INFERRED for
   a tmux pane), the line discipline stores it as **LF**.
3. When the turn ends, rustyline re-enters raw mode with `TCSADRAIN`, which keeps queued input
   (`rustyline src/tty/unix.rs:1646`, VERIFIED), and reads `wake text + LF`. Rustyline maps `0x0a` to Ctrl+J
   (`rustyline src/keys.rs:52`, VERIFIED), and goose binds **Ctrl+J to `Cmd::Newline`**, not to accept
   (`goose@1.36 crates/goose-cli/src/session/input.rs:85-92,138-145`; `goose@main input.rs:175`; the key is
   `GOOSE_CLI_NEWLINE_KEY`, default `j`). VERIFIED binding.
4. Result: the wake text sits in the composer followed by an inserted newline, **not submitted**. The agent stays
   idle with an apparent draft, and every later @mention during that state also lands in the buffer (no busy
   check, see 3.4) and is not submitted either. A human must press Enter.

If instead the pane's tty is configured without `ICRNL`, the bytes stay CR and the wake submits after the turn
as the next user message. That path is merely late, not lost. Only the `ICRNL` state of the pane is INFERRED;
the rest of the chain is VERIFIED in source. Exception: while a cliclack prompt is up mid-turn (approval,
elicitation) the terminal is read, see 3.4.

### 3.4 Idle/busy detection and draft safety: none

VERIFIED: the watcher injects whenever `has_trigger` is true (`wrapper.py:502-567`). There is no composer reading,
no busy check and no draft check. The only "activity" logic hashes `tmux capture-pane -p` every second for the web
UI pill (`wrapper_unix.py:159-183`). It has a Codex-specific particle filter and nothing for goose
(`provider == "codex"`, `wrapper_unix.py:174-175`). Goose's spinner would make it read "active" while busy and
"idle" when static, which is adequate for the pill only. INFERRED.

Goose-specific hazards (INFERRED, from the primer section 9):

- **Human draft:** the paste is appended to whatever the human has typed, and Enter submits the concatenation as
  one message. Nothing is lost, but the draft is sent early and mixed with the wake text.
- **Tool approval prompt** (`GOOSE_MODE=approve|smart_approve`): the CLI shows a `cliclack::select` whose first
  item is "Allow" (`goose@1.36 crates/goose-cli/src/session/mod.rs:1689-1712`, VERIFIED). An injected Enter would
  confirm the highlighted choice, so **a chat @mention could approve a pending tool call**. The pasted letters
  are interpreted by cliclack first (behaviour unknown).
- **Elicitation forms** (`goose@1.36 crates/goose-cli/src/session/elicitation.rs`) and `cliclack::confirm`
  prompts (the `/compact` confirm, the plan "act on it?" confirm, the top-up URL confirm; `mod.rs:943,995,1658`,
  VERIFIED that the prompts exist) can be answered the same way. The first two follow a human's slash command, so
  they matter only when a human has one open.
- **`GOOSE_PROMPT_EDITOR`:** the composer is `$EDITOR`, and the paste goes into the editor instead (primer
  section 9).
- **Ctrl+C is never sent** by agentchattr (VERIFIED: `inject` sends only paste and Enter), so goose's
  "Ctrl+C clears the buffer / twice exits" is not triggered. This is good.

### 3.5 Latency

From mention to typed prompt it takes up to 1 s of poll, plus a fixed 0.5 s sleep (`wrapper.py:503-506`), plus 2
to 4 local HTTP calls (role once or twice, active rules, and a rule-sync POST when rules are injected; 3 s timeouts
each, `wrapper.py:430-469`, `533-558`), plus at least 0.3 s of
inject delay. That is about 1-2 s nominal (INFERRED). Then the model needs one extra round trip for `chat_read`
before it can act.

### 3.6 Failure modes

All VERIFIED in code unless marked:

- Triggers queued while the wrapper is down are discarded: the queue is truncated at startup (`wrapper.py:719-721`).
- Read-then-truncate race: a line appended between `readlines()` and `write_text("")` is lost
  (`wrapper.py:484-486`; race INFERRED).
- No retry after a failed tmux call, and the wake is lost (`wrapper_unix.py:77-94`; the watcher ignores the
  return value, `wrapper.py:567`).
- The backlog skip in `chat_read` beyond 20 messages.
- On a busy goose, the wake is probably stuck as an unsubmitted draft (3.3, INFERRED).
- If goose exits, the wrapper restarts it after 3 s (`wrapper_unix.py:267-273`). The restart is a fresh `goose
  session` (new goose session id), so the conversation context is gone. The chat cursor survives on the server,
  so the new session reads only new messages, not the history. INFERRED from `wrapper.py`/`mcp_bridge.py`.
  Nothing passes `-r`. If the human detached from tmux (Ctrl+B D) earlier, a later goose exit ends the wrapper
  with no restart (`wrapper_unix.py:258-265`, VERIFIED).
- A first trigger while goose is still connecting extensions or showing the first-run `configure` flow
  (`handle_default_session` runs `handle_configure` when no config exists, `cli.rs:2001-2003`; `goose session`
  behaviour similar, unknown) types into whatever screen is up.

## 4. Setup and installation footprint on the harness

Under the generic entry, INFERRED from the code paths above:

- Goose config files: **none written.** `proxy_flag` adds only argv. No `~/.config/goose/config.yaml` edit, no
  plugin, no hook.
- Process wrapping: goose runs inside a tmux session that agentchattr owns. In a Herdr pane that means tmux nested
  in the pane, and Herdr sees the `tmux attach` client, not goose. INFERRED.
- Environment: `CLAUDECODE` is unset, plus any `strip_env` (`wrapper.py:723-724`, `wrapper_unix.py:213-222`).
  Nothing goose-specific.
- agentchattr state: `data/<name>_queue.jsonl`, the registry entry, a per-instance proxy on an ephemeral
  127.0.0.1 port (`mcp_proxy.py:84,101-103,319`; the wrapper passes no port, `wrapper.py:652-657`).
- Mis-configuration risk: choosing `mcp_inject = "env"` would write `~/.gemini/trustedFolders.json` even for
  goose (`wrapper.py:737-738`). VERIFIED code path.

## 5. Token cost

Per message delivered (INFERRED estimates; byte counts VERIFIED):

- Wake prompt typed as a user message: about 85-95 chars, roughly 20-25 tokens. Add the role, and on every 10th
  trigger, epoch change or first trigger, the full rules text (unbounded).
- One model tool call `chat_read`, then the result: JSON with about 40 tokens of framing per message (README
  estimate, `overview.md`) plus the bodies, up to 20 messages, untruncated. On multi-instance, plus the identity
  breadcrumb.
- The reply goes out via `chat_send` (another tool call). Every delivery costs at least two extra model round
  trips: wake turn to `chat_read`, then `chat_read` to `chat_send`.

Standing cost per request (system prompt and tools), paid on every provider call including quiet turns:

- MCP `instructions` of 5,736 chars (`mcp_bridge.py:54-124`, measured on the evaluated string), roughly
  1.3-1.5k tokens,
  injected into goose's system prompt (VERIFIED placement, `extension_manager.rs:1069-1082`).
- 12 `chat_*` tool schemas: about 3.9 KB of docstrings in total, measured by AST over `mcp_bridge.py`, plus
  parameter schemas, roughly 1-1.3k tokens. All 12 are registered (`_ALL_TOOLS`, `mcp_bridge.py:943-958`,
  VERIFIED). The README says 11 tools (`README.md:297`); it omits `chat_decision`.
- Per quiet turn beyond that: **zero**. Nothing is injected without a wake. Prompt caching softens the standing
  cost if the provider supports it (unknown per provider).

## 6. Limitations and bugs visible for Goose

1. Not shipped. No `[agents.goose]`, no launcher, no built-in `_BUILTIN_DEFAULTS["goose"]`. The obvious generic
   attempt (`mcp_inject` unset, or `settings_file`/`.mcp.json` per the README) silently gives goose no chat tools
   (section 1.1). VERIFIED code paths.
2. A wake into a busy goose probably ends up as an unsubmitted draft because LF is bound to newline (3.3).
   High impact; code chain VERIFIED except the pane's `ICRNL` state; not tested end to end.
3. No idle/busy or draft gate. A wake can approve a pending tool call (3.4). INFERRED from VERIFIED UI code.
4. No in-turn delivery even though Goose has a usable surface: the default-enabled `tom` extension re-reads
   `GOOSE_MOIM_MESSAGE_FILE` before each provider call (primer section 5). agentchattr does not use it. VERIFIED
   absence.
5. Restarted sessions lose context. The wrapper never resumes (`-r`) the previous goose session. INFERRED.
6. The tool names carry an ugly prefix that varies with the port (`127_0_0_1_<port>_mcp__chat_read`), which may
   reduce how reliably the model matches the "use mcp to read" instruction. INFERRED.
7. Inherited generic flaws: backlog skip above 20 messages, lossy queue file (startup truncate and the read/truncate
   race), last-line-wins coalescing, wake text not bounded (rules), no ACK, no priority, so every @mention wakes.
   VERIFIED (`overview.md` and the code cited above).

## 7. Clever ideas worth stealing

- **Paste-buffer plus bracketed paste, then a separate Enter** (`wrapper_unix.py:64-136`). For goose this matters
  doubly: one bracketed block keeps any LF from being read as Ctrl+J, and the separate CR is the only key that
  submits. If herdr-threads ever types into a goose pane, the wake marker must be followed by CR in raw mode and
  must never contain an LF. Our content-free `herdr-threads: wake up` already satisfies the second condition.
  INFERRED applicability.
- **Proxy-stamped identity** (`mcp_proxy.py:281-310`): the model never sees or supplies its own identity. For
  goose, where the 1.36 shell tool does not expose `AGENT_SESSION_ID` (primer section 11), binding identity at
  the transport is attractive. The herdr-threads equivalent would be an MCP extension launched per pane with
  `HERDR_PANE_ID` in its env. Goose passes `AGENT_SESSION_ID` to stdio extensions (primer section 6). INFERRED.
- **CLI flag injection with zero config writes** (`--with-streamable-http-extension` via `proxy_flag`). A managed
  goose launch could add herdr-threads the same way (or `--with-extension 'HERDR_PANE_ID=... herdr-threads mcp'`)
  and set `GOOSE_MOIM_MESSAGE_FILE` in the same argv/env. That gives an in-turn delivery path without touching
  `~/.config/goose`. INFERRED design idea, not something agentchattr does.
- **Escalating "do not poll" text on empty reads** (`mcp_bridge.py:666-679`): a cheap guard against a model that
  loops on an inbox tool. INFERRED value for our `inbox` output.
- **Counter-examples to avoid**, not to steal: rules carried in the wake prompt (contradicts our R18
  content-free wakes), and ungated typing (contradicts delivery-contract rule 4). Goose's approval prompt makes the
  second one actively dangerous.

## Verification

Adversarial pass, 2026-10-23, against the agentchattr clone at `d775776`, `goose-src/goose-1.36` and
`goose-src/goose`, the rustyline 18.0.0 crate tarball (crates.io, read only, not built) and the GitHub API.

- Confirmed as written: stars/forks/release (1558/239, v0.5.0 2026-07-26); no `goose` anywhere in agentchattr;
  `_BUILTIN_DEFAULTS`, `_provider_from_command`, the five inject modes and their payloads; `needs_proxy` with an
  empty mode; argv order; queue write/read/truncate and startup truncate; wake prompt strings; paste-buffer /
  Enter sequence and 5 s tmux timeout; activity checker's Codex-only filter; proxy sender stamping and
  ephemeral port; `chat_read` cursor, `[-limit:]` skip, anti-poll text and identity breadcrumb; goose
  `--with-streamable-http-extension` on `session`/`run` only, headerless `parse_streamable_http_extension`,
  `name_to_key` naming, `__` prefix split, extension instructions in the system prompt (`system.md:23-24`),
  default `GOOSE_MODE=auto`, approval `select` with "Allow" first, Ctrl+J newline binding, no `.mcp.json` in 1.36
  and stdio-only plugin `.mcp.json` on main.
- Changed: identity hint size (239 chars, not about 280) and its `slot > 1` condition; newline flattening
  replaces LF only, not CR; instructions size (5,736 chars evaluated, 6.3 KB was the source span) and token
  estimate; all 12 tools are registered (was INFERRED); HTTP calls per wake (2-4, not 2-3); bad citations for
  the proxy port (`270-290` was the SSE rewrite) and the no-retry line; the `mod.rs:943,995,1658` prompts are
  compact/plan/top-up confirms, elicitation cited separately; idle and busy goose analysis (3.2, 3.3) upgraded
  from INFERRED to VERIFIED in rustyline source (bracketed paste default on, off outside readline, `ICRNL`
  cleared only in raw mode, `TCSADRAIN`, `0x0a` = Ctrl+J), leaving only the pane's `ICRNL` state INFERRED;
  added the no-restart-after-detach case and the `start_api_agent` launcher.
