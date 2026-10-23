# agentchattr x Pi coding agent (pi-mono)

- Alternative: agentchattr, <https://github.com/bcurts/agentchattr>, clone `scratchpad/alts/agentchattr` at
  HEAD `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10, VERSION 0.5.0). GitHub API 2026-10-23:
  1558 stars, 239 forks, last push 2026-09-10, `open_issues_count` 18 (issues plus PRs).
- Harness: Pi, <https://github.com/badlogic/pi-mono>, clone `scratchpad/alts/pi-mono` at
  `a7b94a03f09f20f4937fa1a76339014c60d261ad` (`packages/coding-agent` 1.4.0). Pi paths below are relative to
  `packages/coding-agent/` unless they start with `packages/`.
- Shared background: `overview.md` (agentchattr) and `../../primers/pi.md`. Not repeated here except where Pi
  changes the outcome.
- Tags: VERIFIED = read in source or docs at the cited line; INFERRED = reasoning, not read or run. Nothing was
  executed: neither agentchattr nor Pi is installed, so every "it would work" below is INFERRED.

## Support status

**Not supported.** VERIFIED:

- No Pi entry in `config.toml` (agents: claude, codex, gemini, antigravity, kimi, qwen, kilo, codebuddy,
  copilot, minimax; `config.toml:10-94`), no `macos-linux/start_pi.sh` or `windows/start_pi.bat`, no `pi` in
  `_BUILTIN_DEFAULTS` (`wrapper.py:131-161`). `grep -rniE '\bpi\b|pi-coding|pi\.dev|badlogic|earendil'` over
  the clone finds nothing.
- The MCP behavioural contract names base identities for Anthropic, OpenAI, Google, Qwen and Kilo products
  only (`mcp_bridge.py:60-68`); Pi is absent.
- GitHub issue/PR search on `repo:bcurts/agentchattr` for `pi agent`, `pi-mono`, `pi.dev`, `badlogic`,
  `mcp pi`: 0 hits each (a control search for `codex` returns 37, so search works).

**Correction to the mapper's note.** The note says Pi "lacks" MCP without an extension. That is out of date for
Pi 1.4.0: Pi ships a built-in MCP client (`builtin:mcp`) for stdio and streamable HTTP, configured in
`~/.pi/agent/mcp.json` or trusted-project `.pi/mcp.json`. VERIFIED: `docs/mcp.md:3,30-32,64`,
`src/extensions/mcp/config.ts:146-157`. So agentchattr's generic path is not blocked on MCP. It is blocked on
three Pi-specific details, analysed below: (1) there is no per-process MCP-config flag or env var that any
agentchattr `mcp_inject` mode can target, (2) Pi's default tool exposure hides MCP tools from the model, and
(3) Pi never shows MCP server `instructions` to the model, which is where agentchattr keeps its whole
behavioural contract.

Verdict (INFERRED): a user-written `[agents.pi]` entry can be made to work, at "generic, degraded" quality:
the tmux wake works as well as on Claude, read/reply works only after a manual exposure override, and the
agent runs without agentchattr's instructions.

## Generic path, step by step

### What a user would configure

The only `mcp_inject` mode that can produce a Pi-readable config is `settings_file` (INFERRED from the mode
list `wrapper.py:163` and the Pi CLI, which has no MCP-config flag: `docs/cli.md:16,195-199,318-333` list only
`pi mcp add|remove|list|login|logout`, `-e`, `-ne`, `--no-mcp`):

```toml
[agents.pi]
command = "pi"
cwd = ".."
mcp_inject = "settings_file"
mcp_settings_path = "~/.pi/agent/mcp.json"   # or ".pi/mcp.json" (project, needs Pi project trust)
mcp_transport = "http"
mcp_http_key = "url"                         # required: the default key "httpUrl" leaves no `url`
```

`mcp_http_key = "url"` is not optional: agentchattr's default key is `httpUrl` (`wrapper.py:232`
`http_key = inject_cfg.get("mcp_http_key", "httpUrl")`), and Pi rejects an entry with neither `url` nor
`command` ("needs either \"command\" (stdio) or \"url\"", `src/core/mcp-servers.ts:304`). VERIFIED.

Why the other modes fail for Pi (INFERRED from the code paths, VERIFIED code):

| Mode | What it does | Why not Pi |
|---|---|---|
| none (no `mcp_inject`, command not a builtin) | `_resolve_mcp_inject` returns `{}` (`wrapper.py:183-194`); `needs_proxy` is then true (`wrapper.py:639`) and an identity proxy starts on an OS-assigned port (`mcp_proxy.py:84-89`, `wrapper.py:652-661`), but nothing passes its URL to the CLI (`_apply_mcp_inject` returns `[], {}, None` for an empty mode, `wrapper.py:221-223`) | Pi never learns the proxy URL; the user cannot pre-configure a random port. |
| `flag` | writes `{"mcpServers":...}` and passes `[mcp_flag, path]` (`wrapper.py:289-298`) | Pi has no flag that takes an MCP JSON file; `-e` expects an extension module. |
| `proxy_flag` | expands a CLI-flag template with the proxy URL (`wrapper.py:312-317`) | No Pi flag sets an MCP server URL. |
| `env` | writes a file and exports its *file* path in `mcp_env_var` (`wrapper.py:254-287`) | Pi's only config-location variable is `PI_CODING_AGENT_DIR`, a *directory* that also holds auth and settings (`docs/environment-variables.md:81`). |
| `env_content` | exports JSON in an env var, Kilo/OpenCode shape `{"mcp":{...,"type":"remote"}}` (`wrapper.py:300-310`) | Pi reads no config-content variable. |

### What `settings_file` writes into Pi's config

`_write_json_mcp_settings` (`wrapper.py:38-82`) merges into the target file:

```python
entry = {"type": "http", http_key: url, "trust": True}          # wrapper.py:66
entry["headers"] = {"Authorization": f"Bearer {token}"}          # wrapper.py:70
servers[SERVER_NAME] = entry                                     # wrapper.py:71, replaces the whole entry
existing["security"]["folderTrust"]["enabled"] = True            # wrapper.py:75-79 (Gemini setting)
```

Pi accepts it (VERIFIED): `readConfigFile` reads only `mcpServers` and `autoEnableCodemode`
(`src/extensions/mcp/config.ts:102-108`), so the top-level `security` key is ignored; `validateMcpServerConfig`
reads named fields only, accepts `type: "http"` with an http `url` and string `headers`, and does not reject
the extra `trust` key (`src/core/mcp-servers.ts:248-291`). The URL is `http://127.0.0.1:8200/mcp`
(`wrapper.py:197-203`), FastMCP streamable HTTP (`mcp_bridge.py:949-971`); Pi rejects only legacy SSE
(`src/core/mcp-servers.ts:270`), so `mcp_transport = "sse"` must not be used.

### The exposure trap

Pi's default exposure for an MCP server is `codemode`: tools are "neither declared to the model nor listed in
the codemode description" (`docs/mcp.md:191-195`; `exposureOf` defaults to `"codemode"`,
`src/extensions/mcp/index.ts:135-137`). agentchattr's writer cannot set `exposure`, and because it replaces the
whole `agentchattr` entry on every launch and on every token change (`wrapper.py:71`, `wrapper.py:680-690`), a
`"exposure": "direct"` that the user (or Pi's `/mcp` UI, which saves to the defining file,
`docs/mcp.md:90`) adds to `~/.pi/agent/mcp.json` is wiped on the next wrapper start. VERIFIED code; the wipe is
INFERRED from the overwrite.

Two ways out (INFERRED):

1. A project override that survives: Pi lets a trusted project's `.pi/mcp.json` carry an entry with no
   `command`/`url`/`type` that overrides only `enabled`, `exposure`, `toolExposure` of the user-level server
   (`src/extensions/mcp/config.ts:75-80,109-121`; `docs/mcp.md:34`):
   `{"mcpServers": {"agentchattr": {"exposure": "direct"}}}`. With `direct`, all 12 agentchattr tools
   (`mcp_bridge.py:943-946`) are declared as `mcp__agentchattr__chat_read` etc. (`docs/mcp.md:77,191`), and the
   first prompt waits up to 10 s for the server (`docs/mcp.md:100`).
2. Leave it at `codemode`: the model sees only a system-prompt line in the `mcp_servers` section,
   `- mcp__agentchattr (codemode): <first line of the server instructions, cut to 250 chars>`
   (`src/extensions/mcp/index.ts:156-163,185-221`; namespace `mcp__<server>` from `src/core/mcp-servers.ts:155-157`), and must write a codemode script that calls
   `mcp__agentchattr__chat_read` to act on the wake prompt "use mcp to read #general". Plausible for a strong
   model, but every wake then costs a script-writing step, and it is untested (INFERRED).

### The lost contract

agentchattr's behaviour rules (always reply with `chat_send`, do not poll, identity recovery, rules, jobs,
summaries) live only in the MCP `instructions` string, 5736 chars (`mcp_bridge.py:54-124`, passed at
`mcp_bridge.py:955`; size measured with `ast`). Pi uses server instructions only for the one-line summary above
(only for `codemode`/`deferred` servers, `src/extensions/mcp/index.ts:195-198`) and for `describeNamespace()` in
codemode scripts (`src/extensions/mcp/index.ts:396`; `docs/mcp.md:234`: "Server instructions are not part of
any tool description"). VERIFIED. So with `direct` exposure the Pi model sees **none** of the contract, and with
`codemode` it sees the first 250 chars of the first line. What remains is the wake text ("take appropriate
action and respond", `wrapper.py:531`) and the tool docstrings (3928 chars over 12 tools, measured). INFERRED
consequence: a Pi agent is more likely than Claude/Codex to answer in its terminal instead of `chat_send`, to
re-poll after "No new messages", and to skip `chat_rules(list)` at session start.

## Injection mechanism

Same two-step design as on every harness (`overview.md`, "Injection mechanism"); what is specific to Pi:

- **Carrier**: the wake prompt is typed into Pi's editor and becomes an ordinary `user` message
  (INFERRED: it goes through the interactive submit path, `src/modes/interactive/interactive-mode.ts:3321-3338`).
  Content is then pulled by the model through `mcp__agentchattr__chat_read` (a tool call; Pi's tool pipeline,
  `docs/mcp.md:252`). No Pi extension event (`before_agent_start`, `tool_result`, `sendMessage`) is used:
  agentchattr has no hook code for any harness (VERIFIED, `overview.md` line 41) and nothing Pi-specific.
- **Code path** (VERIFIED): server `AgentTrigger.trigger` appends one JSON line to `data/<name>_queue.jsonl`
  (`agents.py:32-54`) → wrapper `_queue_watcher` polls each 1 s, reads all lines, truncates the file
  (`wrapper.py:480-486,571`) → builds the prompt (`wrapper.py:526-531`), appends ROLE, RULES (first trigger,
  rules epoch change or every N-th trigger) and a one-time identity hint (`wrapper.py:533-563`), flattens
  newlines (`wrapper.py:564-567`) → `wrapper_unix.inject` (`wrapper_unix.py:64-136`).
- **Payload**: `use mcp to read #<channel> - you're mentioned, take appropriate action and respond`, or the job
  variant, or a session-engine custom prompt (`wrapper.py:526-531`). `chat_read` returns a JSON array
  `{id,sender,text,type,time,channel[,attachments,reply_to]}` (`mcp_bridge.py:409-426`); a job read returns a
  header entry plus the **whole** job thread, with no limit or cursor (`mcp_bridge.py:598-637`).
- **When it fires**: only when the model calls `chat_read`, i.e. after a wake prompt or on its own initiative.
  No turn-start or post-tool push. On Pi, however, a wake typed while Pi is busy becomes a **steer** that Pi
  inserts after the current tool batch (see Wake), so the *pointer* to new mail arrives mid-run; the content
  still needs a tool call. INFERRED from `interactive-mode.ts:3333-3338` and `packages/agent/src/agent-loop.ts`
  steering polls (primer section 2.3).
- **Size bounds**: agentchattr bounds a channel read to `limit` (default 20) messages and no per-body limit
  (`mcp_bridge.py:575-578,662`). Pi adds its own bound: direct-call MCP text results over 20 KB "reach the model
  with their middle removed" and the full text goes to a temp file (`docs/mcp.md:232`). VERIFIED both. INFERRED:
  a long job thread (unbounded) or 20 long messages lose their middle on Pi, which silently drops whole messages
  from the JSON array, a failure mode Claude/Codex do not have in the same place. Paste side: a wake prompt over
  1000 chars or 10 lines is stored as a paste and shown as `[paste #N 1234 chars]`, expanded on submit
  (`packages/tui/src/components/editor.ts:1089-1096,1298-1316`); flattened wake text is one line, so only very
  long RULES or session prompts hit this, and it is harmless (VERIFIED expansion; INFERRED harmless).
- **Framing**: none beyond the JSON fields; on multi-instance an `[identity: <name> | label: <label>]`
  breadcrumb line is prepended (`mcp_bridge.py:681-688`).
- **Dedupe/ack**: per-agent per-channel cursor moved to the last returned id (`mcp_bridge.py:566-572,662-663`);
  if more than `limit` arrived, older unread ones are skipped (`overview.md` "Loss on backlog"). No ACK. Coalescing:
  all queue lines in one poll produce one wake, last line's channel wins (`wrapper.py:488-531`).
- **Identity on Pi**: the bearer token in the `headers` of the written entry makes the server ignore the
  model-supplied `sender` (`mcp_bridge.py:139-172`). VERIFIED. Two caveats, INFERRED:
  - `mcp_settings_path` is one shared file for all Pi instances (no per-instance path for `settings_file`,
    `wrapper.py:234-248`). A second `pi` wrapper overwrites the first one's token. The first Pi process keeps
    its already-connected config until `/reload` or a dropped-connection reconnect re-reads it (unknown whether a
    reconnect re-reads the file), after which it would post **as the second instance**.
  - On a 409 re-registration the wrapper rewrites the file (`wrapper.py:680-690,780-787`) but Pi does not re-read
    `mcp.json` mid-session (`docs/mcp.md:28`: "Run `/reload` after ... changing a server outside the session"),
    so every call returns "stale or unknown authenticated agent session" (`mcp_bridge.py:173-174`) until the
    human reloads.
  - Pi's own session id (`PI_SESSION_ID`, primer section 5) is never used; identity is the tmux session/wrapper.

## Wake mechanism

- **Mechanism** (VERIFIED, harness-agnostic): `tmux load-buffer` + `tmux paste-buffer -p -d -t <pane>`, sleep
  `max(inject_delay, len*1 ms)` (default 0.3 s, `wrapper.py:908`), `tmux send-keys Enter`
  (`wrapper_unix.py:97-136`). Pi enables bracketed paste (`packages/tui/src/terminal.ts:211-212`), so the paste
  arrives as one `ESC[200~...ESC[201~` block. Pi in tmux: plain `Enter` submits even without extended-keys
  config; tmux merely cannot distinguish Shift+Enter (`docs/tmux.md:3`). INFERRED: the Enter works.
- **Idle/busy detection**: none before injecting (VERIFIED `wrapper.py:482-567`). Pi's own semantics then decide
  (VERIFIED `interactive-mode.ts:3321-3345`):
  - idle: the text is submitted as a new prompt, which starts a run;
  - streaming: `prompt(text, {streamingBehavior: "steer"})`, delivered after the current tool batch, before the
    next model request (primer 2.3). Unlike Claude Code's queued input, this steers the running task;
  - compacting: queued as a steer.
  So on Pi a busy agent is "nudged" mid-run every time, which is the opposite of herdr-threads' rule 4, though it
  is also an accidental in-turn pointer delivery (INFERRED).
- **Activity indicator**: hash of `tmux capture-pane -p` each second (`wrapper_unix.py:159-183`); the Codex
  particle filter is applied only for `provider == "codex"` (`wrapper_unix.py:174-175`), and
  `_provider_from_command("pi")` returns `""` (`wrapper.py:166-180`), so no Pi filter. Unknown whether Pi's
  idle TUI animates; if it does, the UI pill shows "active" forever. The pill does not gate wakes either way.
  Pi's OSC 7501 program status (`working/idle/...`, primer section 7) is not forwarded by tmux and not read.
- **Draft safety**: none (VERIFIED absence). INFERRED Pi-specific hazards:
  - a human draft gets the wake text inserted at the cursor and submitted together;
  - a submitted line beginning with `!` runs as a user bash command (`!!` excluded from context), not a prompt
    (VERIFIED `src/modes/interactive/interactive-mode.ts:3303-3315`), so a human draft starting with `!` plus
    the pasted wake text would run the wake text as part of a shell command;
  - if an extension dialog, a selector (`/resume`, `/model`, `/mcp`) or a `/` command menu is open, the paste
    lands there and Enter selects or confirms whatever is highlighted;
  - pressing Escape (abort) after a steer was queued returns queued messages to the editor (`docs/usage.md:38,40`:
    "Aborting returns queued messages to the editor"; also `Alt+Up`, line 37), so a wake can reappear as a stale
    draft.
- **Latency** (INFERRED from code): ≤1 s queue poll + 0.5 s pre-inject sleep (`wrapper.py:506`) + role and rules
  HTTP fetches (3 s timeouts each, `wrapper.py:430-451`) + paste + 0.3 s + Enter: about 2 s nominal; then on a
  cold Pi the first prompt may wait up to 10 s for `direct` MCP servers (`docs/mcp.md:100`), then one model
  request for `chat_read` and one for `chat_send`.
- **Failure modes**: tmux errors are printed and not retried (`wrapper_unix.py:84,100-135`); the read-then-truncate
  race can drop a trigger line (`wrapper.py:484-486`, INFERRED); triggers while the wrapper is down are dropped at
  start (`wrapper.py:719-721`); the wrapper restarts `pi` 3 s after it exits (`wrapper_unix.py:267-273`), starting
  a **new** Pi session (no `--continue`/`--session-id` passed, `wrapper_unix.py:205-207`), so the agent loses its
  context and must recover identity from scratch (INFERRED).

## Setup / installation footprint on the harness

- Writes `~/.pi/agent/mcp.json` (real user config) if the user follows the obvious recipe: one `agentchattr`
  entry with a live bearer token in plain text, `"trust": true`, plus a stray top-level `security.folderTrust`
  object; other servers in the file are preserved (`wrapper.py:55-81`). Alternative: project `.pi/mcp.json`
  (relative paths resolve against the agent `cwd`, `wrapper.py:242-245`), which Pi reads only for trusted
  projects (`docs/mcp.md:32`; `docs/security.md:27-40`) and which then sits in the repo working tree with a token
  in it. VERIFIED code; recipe INFERRED.
- Plus, for usable tools, a project `.pi/mcp.json` exposure override (above).
- Creates a tmux session `agentchattr-<name>` and kills a stale one of that name on each (re)start
  (`wrapper.py:890`, `wrapper_unix.py:238-247`); strips `CLAUDECODE` from the env (`wrapper.py:723`).
- No Pi extension, skill, `settings.json`, keybinding or `PI_CODING_AGENT_DIR` change. Token file is never
  removed on exit (VERIFIED: no cleanup in `wrapper.py:916-936`).
- Pi would allow a footprint-free attach (`pi -e <ext>` per process, `pi.registerMcpServer` per session without
  writing config: `docs/cli.md:195`, `docs/mcp.md:258-262`), but agentchattr has no way to use it.

## Token cost

INFERRED estimates (chars/4), not measured on a live Pi:

- **Per message (wake path)**: wake prompt about 20 tokens (more on ROLE/RULES triggers, every 10th by default,
  `wrapper.py:546-559`); a `chat_read` call plus about 40 tokens of JSON per message plus the body
  (`README.md` estimate cited in `overview.md`); a `chat_send` call and its short result. Two model requests
  minimum. With `codemode` exposure, add a script-writing step per read and per send.
- **Per quiet turn**: zero from agentchattr's side (nothing injected). But with `direct` exposure, Pi declares
  all 12 tool schemas in **every** request, about 3.9k chars of docstrings plus parameter schemas, on the order
  of 1.5-2k tokens per request (cacheable), whether or not the turn has anything to do with chat. With
  `codemode`, the standing cost is the `mcp_servers` section line, about 70 tokens.
- **Anti-poll text** on empty reads: 12-20 tokens each, escalating (`mcp_bridge.py:666-679`).

## Limitations and bugs visible for Pi

1. No shipped support; the generic `settings_file` mode is the only working way in, and it targets real user
   config with a plaintext token (above).
2. Default Pi exposure (`codemode`) leaves the tools undeclared (reachable only from codemode scripts); the writer
   resets any user change to the entry (`wrapper.py:71`).
3. The MCP `instructions` contract never reaches a Pi model (Pi design, `docs/mcp.md:234`).
4. Shared settings file means multi-instance token cross-over and stale tokens after 409 (above).
5. Typed wake into a busy Pi steers the running task; no draft, dialog or bash-mode guard.
6. Pi's 20 KB middle-truncation can silently cut messages out of a `chat_read` JSON array; job reads are
   unbounded (`mcp_bridge.py:598-637`).
7. Restart starts a fresh Pi session (no resume flag).
8. Activity pill has no Pi-specific filter.

## Clever ideas worth stealing (and how Pi changes them)

- **Bracketed paste then Enter** (`wrapper_unix.py:64-84`): one atomic paste survives split pty reads. Relevant
  to any typed wake on Pi too; Pi stores pastes over 1000 chars as an expandable marker, so the wake must stay
  short anyway (herdr-threads' content-free wake already does).
- **One wake per poll for many triggers** (`wrapper.py:488-531`): cheap coalescing.
- **Escalating "No new messages, stop polling" tool text** (`mcp_bridge.py:666-679`): a model-facing
  backpressure signal in the tool result itself.
- **Bearer token per instance, server overrides self-declared sender** (`mcp_bridge.py:139-172`): attribution
  does not depend on the model naming itself.
- **Counter-lesson for a herdr-threads Pi adapter** (INFERRED): everything agentchattr has to fake from outside
  (idle check, draft check, wake, in-turn delivery, per-process config) Pi offers in-process: an extension
  loaded with `pi -e` can `registerMcpServer` with `exposure: "direct"` and no config write, wake with
  `pi.sendMessage(..., {triggerTurn: true})` only when `ctx.isIdle()` and `ctx.ui.getEditorText()` is empty,
  deliver in-turn via `tool_result` or a non-triggering `sendMessage`, and put the static contract in a skill
  rather than in MCP `instructions` that Pi drops (primer sections 2.3, 2.4, 9). agentchattr's design shows what
  goes wrong without those: steering a busy run, clobbering drafts, and a contract that silently never arrives.

## Verification

Adversarial pass, 2026-10-23, against the agentchattr clone at `d775776a` and the pi-mono clone at `a7b94a03`
(`packages/coding-agent` 1.4.0), plus the GitHub API. Nothing executed.

Checked and held: no Pi support (grep over the clone, `config.toml` agent list, start scripts, `_BUILTIN_DEFAULTS`,
issue search 0 hits for `pi-mono`/`badlogic`, control `codex` 37); every `mcp_inject` branch and its line range;
`_write_json_mcp_settings` lines 66/70/71/75-79; Pi `readConfigFile`, project-override rules, `validateMcpServerConfig`
(SSE rejection, string headers); `codemode` default exposure, 250-char summary, 20 KB middle truncation, 10 s
direct-server wait, `/reload` requirement, instructions not in tool descriptions (only other use: `tool_search`
ranking, `src/extensions/tool-search/tool.ts:114`); instruction size 5736 chars and docstrings 3928 chars over
12 tools (re-measured with `ast`); queue watcher, coalescing, ROLE/RULES/identity hint, flattening; tmux
paste/Enter sequence, delay, restart loop with no resume flag; `chat_read` cursor, `limit` 20, unbounded job read,
escalating empty-read text, breadcrumb; bearer-token identity and the stale-token error; Pi bracketed paste,
1000-char/10-line paste marker, `!` bash path, streaming steer and compaction queue; stars/forks/push date.

Changed: open-issue count labelled as issues plus PRs (`open_issues_count`); added that `mcp_http_key = "url"` is
required (default `httpUrl` is rejected by Pi); the `mcp_servers` line names the namespace `mcp__agentchattr`,
not `agentchattr`; idle-submit citation widened to `interactive-mode.ts:3321-3345`; Escape/abort citation narrowed
to `docs/usage.md:37-40`; limitation 2 says "undeclared" instead of "invisible", since codemode scripts reach the tools.
