# agentchattr x OpenCode

- Alternative: agentchattr, https://github.com/bcurts/agentchattr (MIT), clone HEAD `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10), `VERSION` 0.5.0. Maturity (stars, releases) is in `overview.md`.
- Harness: OpenCode, https://github.com/anomalyco/opencode. Source read at tag `v1.18.1` (matches the local binary), as in `primers/opencode.md`.
- Citations: `agentchattr:<path>:<line>` is relative to the agentchattr clone root. `opencode:<path>:<line>` is relative to the OpenCode v1.18.1 clone root.
- Every claim is tagged **VERIFIED** (read in source or docs) or **INFERRED** (reasoned, not observed running). Nothing was executed.

## 1. Support status

**Not supported by name.**

- **VERIFIED:** a case-insensitive grep for `opencode` / `open code` across the whole agentchattr clone finds nothing. The README's harness list is Claude Code, Codex, Gemini CLI, Copilot CLI, Kimi, Qwen, Kilo CLI, CodeBuddy and MiniMax, plus "any MCP-compatible agent can join" (`agentchattr:README.md:5`). There is no `start_opencode.sh` or `.bat`, and there is no `[agents.opencode]` in `config.toml`.
- **VERIFIED:** with no configuration, `opencode` gets no MCP wiring at all. `_BUILTIN_DEFAULTS` has entries only for `claude`, `gemini`, `codex`, `kimi` and `kilo` (`agentchattr:wrapper.py:131-161`). `_provider_from_command` maps only exact executable names that appear in that table (`wrapper.py:166-180`).
  - For a user-added `[agents.opencode] command = "opencode"` with no `mcp_*` keys, `_resolve_mcp_inject` returns `{}` (`wrapper.py:183-194`).
  - `needs_proxy` then becomes true and a local identity proxy is started (`wrapper.py:639-661`). But `_apply_mcp_inject` returns no launch args and no env for an empty mode (`wrapper.py:221-223`), so OpenCode is never told the proxy URL.
  - Result (**INFERRED**): the wrapper types wake prompts into a session that has no `agentchattr_*` tools. The model sees "use mcp to read #general ..." and cannot act on it.
- **Closest shipped analogue: Kilo CLI.** **INFERRED:** Kilo is an OpenCode derivative; this is not stated in the repo.
  - **VERIFIED:** Kilo uses `mcp_inject = "env_content"` with `mcp_env_var = "KILO_CONFIG_CONTENT"` (`wrapper.py:156-160`).
  - **VERIFIED:** the writer builds `{"mcp": {"agentchattr": {"type": "remote", "url": ..., "enabled": true, "headers": {"Authorization": "Bearer <token>"}}}}` and puts it in the env var. No file is written (`wrapper.py:300-310`).
- **A config-only port to OpenCode is viable. VERIFIED on the OpenCode side:**
  - OpenCode reads `OPENCODE_CONFIG_CONTENT` (`opencode:packages/core/src/flag/flag.ts:22`) and merges it as a "local" source after global, `OPENCODE_CONFIG`, project files and `.opencode/` dirs (`opencode:packages/opencode/src/config/config.ts:468-476`). An existing user `mcp` map is therefore extended, not replaced.
  - OpenCode's remote MCP schema is exactly `{type:"remote", url, enabled?, headers?, oauth?, timeout?}` (`opencode:packages/core/src/v1/config/mcp.ts:44-59`), so the Kilo payload is valid OpenCode config byte for byte.
  - The user-written entry would be:

    ```toml
    [agents.opencode]
    command = "opencode"
    cwd = ".."
    label = "OpenCode"
    mcp_inject = "env_content"
    mcp_env_var = "OPENCODE_CONFIG_CONTENT"
    mcp_transport = "http"
    ```

    **INFERRED:** this would work. It is not shipped, not documented and not tested by agentchattr.
  - The base name must be a configured agent: registration rejects unknown bases (`agentchattr:registry.py:193-194`, `app.py:2177-2179`), and the registry is seeded from the merged `[agents]` table (`app.py:306-307`). The entry can live in `config.local.toml`, which adds agents not already defined in `config.toml` (`agentchattr:config_loader.py:120-133`).

All sections below assume this user-added entry.

## 2. Injection mechanism

agentchattr never pushes message bodies into OpenCode's context. It types a short pointer prompt (section 3), and the model pulls the content with an MCP tool call.

### 2.1 Path of a message

1. **Router decides the targets.** A human or agent posts in the web UI or via `chat_send`. The router picks @mentioned agents; a human message with no mention goes to `routing.default` (`"none"` unless configured), and an agent message is routed only on an explicit mention (`agentchattr:router.py:52-81`, `app.py:315`); targets are resolved to instances and deduped (`app.py:852-861`). The loop guard and session turn guard apply (`app.py:863-893`).
2. **Server writes a trigger line.** `agents.trigger` appends one JSON line `{"sender","text","time","channel"[, "prompt"][, "job_id"]}` to `data/<name>_queue.jsonl` (`agentchattr:agents.py:32-54`). `text` is the triggering message, but the default wake prompt never uses it (VERIFIED: `wrapper.py:526-531`).
3. **Wrapper builds the wake prompt.** The wrapper polls the file every 1 s, reads all lines and truncates the file (`wrapper.py:480-486`, `:571`). It builds one prompt:
   - `use mcp to read #<channel> - you're mentioned, take appropriate action and respond`, or the `job_id=` variant, or a session-engine custom prompt (`wrapper.py:526-531`);
   - plus `ROLE: ...` if a role is set (`:535-540`);
   - plus `RULES: r1; r2; ...` on the first trigger, on a rules-epoch change, or every N triggers (`:542-559`);
   - plus a one-time identity hint for multi-instance agents (`:561-563`, `:422-427`);
   - with newlines flattened to spaces (`:564-567`).
4. **The OpenCode model calls `agentchattr_chat_read`.** OpenCode names MCP tools `<client>_<tool>` (`opencode:packages/opencode/src/mcp/catalog.ts:119`).
5. **Permission.** The default agent permission is `"*": "allow"` (`opencode:packages/opencode/src/agent/agent.ts:119-120`), so no prompt appears, unlike Gemini, which needs folder trust (VERIFIED). A user config with stricter permissions would prompt (INFERRED).
6. **`chat_read` returns the messages** (`agentchattr:mcp_bridge.py:575-689`):
   - The server keeps a cursor per `(sender, channel-key)`. The first read returns the last `limit` (default 20) messages. Later reads return messages after the cursor, then `msgs[-limit:]`, and the cursor moves to the last returned id (`:648-663`, `_update_cursor` `:566-572`).
   - The payload is a compact JSON array: `id, sender, text, type, time, channel[, attachments][, reply_to]` (`:409-426`). Multi-instance agents get an `[identity: name | label: x]` breadcrumb line prepended (`:681-688`).
7. **OpenCode post-processing.** The MCP result text is joined and passed through `Truncate.output` (`opencode:packages/opencode/src/session/tools.ts:425-470`). The limits are 2000 lines / 50 KiB, configurable via `tool_output` (`opencode:packages/opencode/src/tool/truncate.ts:15-16,77-81`). Overflow spills to a file and `outputPath` goes into the metadata.
8. **The model replies** with `agentchattr_chat_send` (`mcp_bridge.py:193`).

### 2.2 Properties

| Property | Finding |
|---|---|
| Exact surface | MCP tool result (model-pulled). No OpenCode plugin, no `chat.message` / `tool.execute.after` / `messages.transform` hook, no HTTP API, no `noReply`. VERIFIED: agentchattr contains no OpenCode-specific code. |
| When it fires | Only when the model calls `chat_read`, which in practice is right after a typed wake. No mid-turn delivery: a busy OpenCode agent sees new chat only if it happens to call `chat_read` again, or when the wake typed during its turn is steered in (section 3.3). INFERRED from the absence of any hook path. |
| Size bounds | agentchattr caps the count (20) but not the bytes. There is no per-body truncation in `_serialize_messages` (VERIFIED). OpenCode's own 50 KiB / 2000-line truncation is the only byte cap (VERIFIED, OpenCode side). |
| Framing | JSON objects with a `sender` field; no "this is peer input" framing per message. Static framing lives in the MCP `instructions` string (5,736 chars, measured from `agentchattr:mcp_bridge.py:54`). OpenCode puts it in the system prompt inside `<mcp_instructions><server name="agentchattr">` (`opencode:packages/opencode/src/session/system.ts:112-129`). VERIFIED. |
| Dedupe | Cursor-based per channel key. The cursor key is `channel or "__all__"` (`mcp_bridge.py:648-652`, `:566-572`), so reading `#general` and then reading all channels re-returns the same messages. VERIFIED code; the duplicate effect is INFERRED. |
| Ack | None. Advancing the cursor on read is the only "seen" state (VERIFIED). |
| Backlog loss | More than 20 new messages: only the newest 20 are returned and the cursor jumps past the rest, so older unread messages are silently skipped (`mcp_bridge.py:655-663`: `get_since` then `msgs[-limit:]` then `_update_cursor`, VERIFIED). |
| Truncation and cursor interaction | When the 20 messages exceed OpenCode's 50 KiB cap, the model sees a truncated head plus a spill-file pointer, but agentchattr's cursor has already advanced past all 20. Recovery depends on the model reading the spill file. INFERRED, from VERIFIED parts on both sides. |
| Anti-polling | An empty read returns escalating text: "Do not poll — wait for your next prompt." … "STOP. Repeated empty reads waste tokens." (`mcp_bridge.py:666-679`). VERIFIED. |
| Server push | None. agentchattr sends no MCP notifications, and OpenCode would drop them anyway: only `notifications/message` (logged) and `tools/list_changed` are handled (primer section 5; `opencode:packages/opencode/src/mcp/index.ts:457-492`). VERIFIED. |

## 3. Wake mechanism

### 3.1 Mechanism (Unix)

- **tmux session.** The wrapper runs `opencode` inside a private tmux session `agentchattr-<name>`:
  - It kills any stale session of that name, creates a new one with an `env -u CLAUDECODE OPENCODE_CONFIG_CONTENT='<json>' opencode ...` command line, and attaches. It restarts the CLI 3 s after exit unless `--no-restart` is given (`agentchattr:wrapper_unix.py:186-280`).
  - The env prefix exists because `subprocess.run(env=)` only affects the tmux client (`:209-222`).
- **Paste then Enter.** Injection is `tmux load-buffer` → `tmux paste-buffer -p -d -t <pane>`, then a sleep of `max(inject_delay, len*1ms)` (default `0.3` s, `wrapper.py:908`), then `tmux send-keys Enter` (`wrapper_unix.py:64-136`).
  - `-p` brackets the paste only if the app enabled bracketed paste. OpenCode does: OpenTUI `onPaste` (primer section 7; `opencode:packages/tui/src/component/prompt/index.tsx:1395-1418`).
  - Each tmux call has a 5 s timeout. A failure is printed and not retried.
- **Windows.** `WriteConsoleInputW` key records (`wrapper_windows.py:134-191`). Whether OpenTUI on Windows reads console input records correctly is unknown.
- **Pause before injecting.** The trigger flag is set and the wrapper sleeps 0.5 s first (`wrapper.py:503-506`). Worst-case latency is therefore about 1 s poll + 0.5 s + 0.3 s ≈ 1.8 s to submit, plus one model turn and the `chat_read` round trip (INFERRED arithmetic from VERIFIED constants).

### 3.2 How OpenCode's composer handles the typed prompt

- **Default prompt.** It is 73 characters plus the channel name (80 for `#general`) and stays under OpenCode's paste-summary threshold (3 or more lines, or more than 150 chars; `opencode:packages/tui/src/component/prompt/index.tsx:1204-1210`), so it is inserted as plain text (VERIFIED thresholds).
- **With ROLE / RULES.** The prompt usually exceeds 150 chars and collapses into a `[Pasted ~1 lines]` extmark. The primer reads the extmark as expanding on submit, so the model still gets the full text (INFERRED, not run). agentchattr's flattening of newlines avoids the line-count trigger but not the length trigger.
- **Enter submits the composer.** `input_submit` (primer section 7). The prompt starts with `use mcp`, not `/` or `@`, so typing it opens no autocomplete. The RULES or ROLE text may contain `@name`. Whether a pasted (as opposed to typed) `@` opens the autocomplete popup and makes Enter select a completion instead of submitting is unknown.

### 3.3 Idle/busy detection and draft safety

- **No gate. VERIFIED.** The queue watcher injects whenever the file has a line. There is no idle check, no busy check and no composer read (`wrapper.py:482-567`).
- **Activity detection drives the UI pill only.** It is the hash of `tmux capture-pane -p`, compared each second (`wrapper_unix.py:159-183`). The Codex-specific filter is not applied, because `_provider_from_command("opencode")` returns `""` (`wrapper.py:891-894`). OpenCode's busy spinner makes "active" correct while it works; whether the idle TUI has any animation that would read as permanently active is unknown.
- **Busy session.** OpenCode does not block submit while busy, and the server persists the message, labels it `QUEUED`, and steers it in at the next step boundary (primer section 3; `opencode:packages/opencode/src/effect/runner.ts:115-122`, `session/prompt.ts:1092,1111-1116`). So a wake typed mid-turn is not lost. It extends the running turn with "use mcp to read #channel", which gives agentchattr de facto mid-turn delivery on OpenCode. This is a happy accident of OpenCode's semantics, not a design. INFERRED composition.
- **Human draft.** The paste is inserted at the cursor of whatever the human has typed, and Enter submits the concatenation. The draft is consumed, and the model gets a garbled mixed prompt. INFERRED from VERIFIED parts: no draft check (`wrapper.py`); the composer inserts at the cursor (`input.insertText`, `index.tsx:1214`).
- **Permission dialog: a real hazard.** While a permission request is pending, the session composer is disabled (`opencode:packages/tui/src/routes/session/index.tsx:236`) and the full-screen "Permission required" prompt owns the keys.
  - Its options are `{once: "Allow once", always, reject}`, with `selected` initialised to the first key, `once` (`opencode:packages/tui/src/routes/session/permission.tsx:405,536-538`).
  - Enter (`return`) runs `onSelect(store.selected)` (`permission.tsx:607-611`), which replies `"once"` (`:426-431`).
  - Consequence (**INFERRED**, not run): an @mention that arrives while OpenCode is waiting on a permission prompt (a bash `ask` rule, `external_directory`, a `.env` read, `doom_loop`) **auto-approves that tool call once**. The paste goes nowhere useful and the Enter approves. The same applies to any other modal that treats Enter as "select default", such as the question dialog (unknown detail).
- **Session routing.** The wake lands in whichever session the TUI is currently showing. One OpenCode TUI can host many sessions and child task sessions (primer section 10). If the human navigated elsewhere, the wake goes to the wrong conversation. Identity is per wrapper and process, not per OpenCode session. INFERRED.
- **Escape is never sent** (only paste and Enter), so the "ESC interrupts a busy turn" hazard does not arise (VERIFIED: `wrapper_unix.py:104-131`).

### 3.4 Failure modes

| Case | Behaviour | Evidence |
|---|---|---|
| tmux error or timeout | Printed, returns False, no retry; the trigger is consumed (file already truncated) | VERIFIED `wrapper_unix.py:84-136`, `wrapper.py:484-486` |
| Trigger appended between read and truncate | Lost (race) | INFERRED `wrapper.py:484-486` |
| Wrapper restart | Queue cleared on start, so pending triggers are dropped; messages stay readable via `chat_read` | VERIFIED `wrapper.py:719-721` |
| Server re-registers the instance (409 on heartbeat) | New token. `_rewrite_mcp_config` re-runs `_apply_mcp_inject`, but in `env_content` mode that only builds a dict; the running OpenCode keeps the old bearer header, fixed at connect in `requestInit` (`opencode:packages/opencode/src/mcp/index.ts:269-283`). The code comments say most CLIs won't re-read mid-session (`wrapper.py:678-679`). Tool calls likely fail auth until OpenCode restarts. | VERIFIED parts; outcome INFERRED |
| Burst of triggers | Coalesced into one wake (last channel wins); other channels' mentions are not named in the prompt | VERIFIED `wrapper.py:488-531` |
| Agent-to-agent ping-pong | Stopped after `max_agent_hops` (default 4) per channel until a human speaks or `/continue` is typed | VERIFIED `router.py:57-88`, `app.py:303,863-873` |
| MCP server down at OpenCode start | OpenCode tries StreamableHTTP, then SSE (`mcp/index.ts:269-283`). On failure the tools are missing for the session (INFERRED); wakes still type prompts the model cannot satisfy. | partly VERIFIED |
| `opencode --pure` | Disables external plugins only; MCP from config content is unaffected (INFERRED) | primer section 2 |

## 4. Setup and installation footprint

- **Harness config files written: none.** `env_content` writes no file (`wrapper.py:300-310`). OpenCode skips seeding `~/.config/opencode/opencode.json` when `OPENCODE_CONFIG_CONTENT` is set (`opencode:packages/opencode/src/config/config.ts:248-256`). This is the cleanest footprint agentchattr has for any harness. Contrast `~/.gemini/...` and `~/.codebuddy/.mcp.json` (`overview.md`). VERIFIED.
- **agentchattr's own files:** queue file `data/<name>_queue.jsonl`, cursors, registry state in its `data/` dir (`overview.md`).
- **Process footprint:** a tmux session `agentchattr-<name>`, which the wrapper creates and kills by that name (`wrapper_unix.py:238-247`). Plus a wrapper process with heartbeat, watcher and activity threads. No proxy is started in `env_content` mode (`wrapper.py:639`). VERIFIED.
- **Secret exposure:**
  - The bearer token sits in the `OPENCODE_CONFIG_CONTENT` value on the tmux command line, so it is visible to same-user tools that list tmux start commands (INFERRED).
  - OpenCode's bash tool spawns children with `process.env` (primer section 10), so every agent-run shell command inherits `OPENCODE_CONFIG_CONTENT`, token included. A nested `opencode` started by the agent would also attach to agentchattr under the same identity (INFERRED).
- **User-visible change:** the agent runs in a tmux session the wrapper attaches to, not in the user's own terminal or pane. With Herdr this means tmux inside a Herdr pane, and Herdr's own agent detection would see a tmux client, not OpenCode (INFERRED).

## 5. Token cost

These are estimates at about 4 chars per token. All are INFERRED from VERIFIED sizes.

- **Static, per request (cached):**
  - MCP `instructions` of 5,736 chars ≈ 1.4k tokens, in the system prompt on every LLM request (`system.ts:112-129`).
  - Twelve tool schemas (`mcp_bridge.py:943-946`), with docstrings totalling 3,928 chars plus parameter schemas ≈ 1.2-1.5k tokens.
  - That is roughly 2.6-3k tokens of fixed overhead in every request of every turn, related to chat or not. It is cacheable, but it is still context.
- **Per wake:**
  - Wake prompt ≈ 20-25 tokens; with RULES on the first trigger and every 10th trigger, plus ROLE, it can be hundreds.
  - One full model turn: the model reads the prompt, emits a `chat_read` call (~30 output tokens), then reads the result at about 40 tokens of JSON framing per message plus bodies (README estimate cited in `overview.md`).
  - Usually a second tool call for `chat_send`.
  - Minimum: two LLM steps per mention, even for a message that needs no reply.
- **Per quiet turn:** zero injected bytes; only the static overhead above. Polling is discouraged by text, not prevented: an empty `chat_read` costs one tool round trip plus about 15-25 tokens of "STOP" text.
- **Busy-time wake on OpenCode:** the steered "use mcp to read" adds an extra step to the running turn even for chatter the agent did not need (section 3.3).

## 6. Limitations and bugs

1. **Unsupported out of the box.** An `opencode` entry without `mcp_*` keys starts a useless proxy, never points OpenCode at it, and wakes a model that has no tools for the job (section 1). VERIFIED code; the outcome is INFERRED.
2. **Enter on a pending permission prompt approves it once.** This is the most serious OpenCode-specific flaw (section 3.3). INFERRED from VERIFIED TUI code.
3. **Draft clobbering:** no composer or idle check (VERIFIED).
4. **Wrong-session wakes** in a multi-session TUI (INFERRED).
5. **Backlog skip** above 20 messages, and cursor-versus-OpenCode-truncation drift (section 2.2).
6. **No ack, no priorities:** every @mention wakes, and a busy agent's turn is extended by OpenCode steering (VERIFIED absence; INFERRED effect).
7. **Stale bearer token after a re-register:** the `env_content` mode cannot refresh a running process (section 3.4).
8. **Trigger loss** on the read/truncate race and on wrapper restart (section 3.4).
9. **Ignores better surfaces.** OpenCode offers a server-side prompt API that cannot touch the draft (`prompt_async`, `noReply`), idle events, and in-turn hooks (primer sections 2-3), and agentchattr uses none of them. The keystroke path is the weakest of the primer's five integration shapes (primer section 12, shape 4).

## 7. Clever ideas worth stealing

1. **Inline config via `OPENCODE_CONFIG_CONTENT` for managed launches.** The value is merged as a local source after the global, `OPENCODE_CONFIG`, project and `.opencode/` sources (`config.ts:468-476`); only the account config, the managed config dir, macOS managed preferences and `OPENCODE_PERMISSION` merge after it (`config.ts:504,516-550`). It also suppresses the global-config seed write (`config.ts:248-256`). It can carry `mcp` entries and also a `plugin` array (`opencode:packages/core/src/v1/config/config.ts:56`).
   - A herdr-threads managed OpenCode launch could load our server plugin per pane with zero writes to `~/.config/opencode`. That satisfies our "never write real user config" rule and keeps tests isolated.
   - Caveat: put no secrets in it, because shell children inherit it.
   - INFERRED applicability; the mechanism is VERIFIED.
2. **Paste as one bracketed paste, then Enter after a length-scaled delay** (`wrapper_unix.py:64-136`). If Herdr's native wake ever types anything longer than a short marker into OpenCode, use bracketed paste, not per-key typing. Our content-free `wake up` stays below OpenCode's 150-char paste-summary threshold.
3. **Escalating empty-read replies** that tell the model to stop polling (`mcp_bridge.py:666-679`). This is a cheap behavioural brake for any pull tool we expose.
4. **Rules-epoch sync report** (`wrapper.py:454-471`, `:542-559`): the wrapper reports which rules version the agent has been shown. This is a lightweight "seen policy version N" signal. Our check-in or skill version could be tracked the same way.
5. **What not to copy:**
   - an ungated Enter keystroke (it auto-approves OpenCode permission prompts);
   - content-bearing wake prompts (RULES/ROLE, contrary to our R18);
   - a lossy skip-to-newest cursor.

   Before herdr-threads ships any keystroke wake for OpenCode, it must check the Herdr composer read *and* the absence of a modal dialog. A plugin `promptAsync` wake avoids the issue entirely.

## Verification

Adversarial pass, 2026-10-23, against the agentchattr clone at `d775776a` and the OpenCode `v1.18.1` clone (`99f638d8293f`). Nothing was executed except read-only measurement scripts over the sources.

- **Checked and held:** no `opencode` string in the agentchattr clone, no `start_opencode.*` in `macos-linux/` or `windows/`, no `[agents.opencode]`; `_BUILTIN_DEFAULTS` (5 entries), `_provider_from_command`, `_resolve_mcp_inject` returning `{}`, `needs_proxy` and the empty-mode early return; the `env_content` payload; queue watcher (1 s poll, read-then-truncate, 0.5 s pause, prompt variants, ROLE/RULES/identity hint, newline flattening, last-channel-wins); queue cleared on start; `inject` paste/Enter sequence and delay; activity hash and Codex-only filter; tmux session lifecycle and env(1) prefix; `chat_read` cursor, `msgs[-limit:]`, escalating empty reads, breadcrumb; `_serialize_messages` has no byte cap; `_MCP_INSTRUCTIONS` is 5,736 chars; 12 tools with 3,928 chars of docstrings (measured); rules refresh default 10 (`app.py:2129`). OpenCode side: `OPENCODE_CONFIG_CONTENT` flag and merge point, seed-write skip, remote MCP schema, `toolName`, default `"*": "allow"`, `Truncate` limits and `tool_output`, `<mcp_instructions>` system block, StreamableHTTP-then-SSE transports with headers fixed in `requestInit`, notification handlers, paste-summary threshold, composer disabled during permissions/questions, permission dialog default `once` and Enter handler.
- **Changed:** section 7 said the inline config is "merged last"; account config, managed config and `OPENCODE_PERMISSION` merge after it (`config.ts:504,516-550`). Routing step now notes `routing.default` for unmentioned human messages. Default prompt length corrected from "about 85" to 73 + channel name (80 for `#general`). Citations tightened: cursor logic `mcp_bridge.py:648-663`, composer insert `index.tsx:1214` (was 1212), loop guard adds `/continue` and `app.py:303,863-873`, base-name rule now cites the registry seed (`app.py:306-307`) and says the entry may live in `config.local.toml`.
- **Not re-checked:** primer-sourced claims (steering/`QUEUED` semantics, bash child env, `--pure`, multi-session routing) and `overview.md` figures were taken as cited.
