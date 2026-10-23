# Harness primer: OpenCode (`opencode`)

Upstream: https://github.com/anomalyco/opencode (formerly sst/opencode). Docs: https://opencode.ai/docs/

Purpose: the shared reference for later alternative x harness reports. It lists every OpenCode surface that can put text into the model's context or wake an idle session, with evidence. Each claim is marked **VERIFIED** (read in source, docs or local CLI help) or **INFERRED** (deduced, not observed running).

## 0. Versions and evidence base

| Item | Value |
| --- | --- |
| Local install | `/var/home/alepar/.opencode/bin/opencode`, `opencode --version` = `1.18.1` (VERIFIED, observed 2026-10-23) |
| Latest GitHub release | `v1.19.4`, 2026-10-22 (VERIFIED, `gh release list -R anomalyco/opencode`) |
| `dev` branch | `packages/*/package.json` version `2.1.4`, commit `f9f463be7831`, 2026-10-23. Tags `v2.0.0` (2026-09-11) to `v2.1.4` (2026-10-23) exist, but none has a GitHub Release (VERIFIED). INFERRED: 2.x ships on a pre-release or beta channel (there is an `OPENCODE_CHANNEL` flag) and is not the stable line yet. |
| Source read | the `v1.18.1` tag (commit `99f638d8293f`), which matches the local binary, and `dev` HEAD for 2.x. Paths below are relative to the clone root. A path marked `[1.18.1]` comes from the v1.18.1 tag, one marked `[dev]` from dev HEAD. |
| Plugin API stability | the v1 `Hooks` interface (`packages/plugin/src/index.ts`) is byte-identical in v1.18.1 and v1.19.4 (VERIFIED, diff over `gh api .../contents/packages/plugin/src/index.ts?ref=v1.19.4`). On `dev` (2.x) the v1 hook names (`chat.message`, `tool.execute.after`, `experimental.chat.messages.transform`) occur nowhere in `packages/` (VERIFIED by grep), so 2.x replaces the plugin API (section 9). |

Doc pages used: https://opencode.ai/docs/plugins/, https://opencode.ai/docs/server/, https://opencode.ai/docs/sdk/. Not fetched, cited by URL only: https://opencode.ai/docs/mcp-servers/, https://opencode.ai/docs/acp/, https://opencode.ai/docs/commands/, https://opencode.ai/docs/config/, https://opencode.ai/docs/cli/.

## 1. Architecture: what it means for injection

- OpenCode is a client/server system. A session loop runs in a server (an in-process worker for the TUI, or `opencode serve`), and the TUI, web UI, `opencode run`, ACP and plugins are clients of its HTTP API. (VERIFIED: `[1.18.1] packages/opencode/src/cli/cmd/tui.ts:210-256`, which spawns a Worker and talks RPC to it.)
- **Plain `opencode` (TUI) exposes no network port by default.** The TUI talks to its worker through an in-process fetch at `http://opencode.internal`. It opens a real HTTP listener only when `--port`, `--hostname` or `--mdns` is passed. (VERIFIED: `[1.18.1] packages/opencode/src/cli/cmd/tui.ts:233-246`: `external = hasArg("--port") || hasArg("--hostname") || network.mdns === true`.) The server docs page says "Running `opencode` starts both a TUI and a server … picks its port at random", which the 1.18.1 source contradicts (docs stale or describing an older generation; VERIFIED discrepancy).
- Consequence: **an outside process (the herdr-threads daemon) cannot reach a stock TUI session over HTTP.** Its options are:
  - (a) an in-process server plugin, which gets an SDK `client` bound to the worker's in-process fetch (VERIFIED: `[1.18.1] packages/opencode/src/plugin/index.ts:141-146`);
  - (b) launching the TUI with `--port`, or running `opencode serve` and attaching with `opencode attach <url>`;
  - (c) keystroke injection into the pane.
- A session's prompt is independent of the TUI composer. Text sent through `POST /session/:id/message` or `prompt_async` never touches the composer draft (VERIFIED by structure: the composer is TUI-local state in `[1.18.1] packages/tui/src/component/prompt/index.tsx`, and the server endpoints write messages straight to the session store). **This is the main structural advantage over keystroke wakes: a server-side prompt cannot clobber a human's draft.**

## 2. Server plugin hooks (v1 API, stable line 1.x)

A plugin is a JS/TS module exporting `server: Plugin`, where `Plugin = (input: PluginInput, options?) => Promise<Hooks>` (VERIFIED: `[1.18.1] packages/plugin/src/index.ts:56-80`).

`PluginInput` carries `client` (SDK v1 client), `project`, `directory`, `worktree`, `serverUrl`, `$` (Bun shell) and `experimental_workspace`.

### Load locations

These are VERIFIED, except the npm cache path, which comes from the docs.

- **Files:** `{plugin,plugins}/*.{ts,js}` in every config dir (`[1.18.1] packages/opencode/src/config/plugin.ts:21`). The config dirs are:
  - the global config dir (`~/.config/opencode`);
  - every `.opencode/` dir between cwd and home;
  - `OPENCODE_CONFIG_DIR` (`config/config.ts:425-462`, `config/tui.ts:171-210`).
- **npm specs:** listed in the config `"plugin"` array, as a string or as `[spec, options]` (`index.ts:70-72`). Docs: they are cached in `~/.cache/opencode/node_modules/`.
- **Env overrides:** `OPENCODE_CONFIG` (a file), `OPENCODE_CONFIG_DIR`, `OPENCODE_CONFIG_CONTENT` (inline JSON), `OPENCODE_DISABLE_PROJECT_CONFIG` (`[1.18.1] packages/core/src/flag/flag.ts:21-64`, `packages/opencode/src/config/config.ts:250,401`). These allow an isolated test install without touching `~/.config/opencode` (INFERRED from the semantics; not exercised).
- **`--pure` / `OPENCODE_PURE`** runs without external plugins (VERIFIED: CLI help). A user can therefore silently disable an integration.

### Dispatch semantics

- **Server hooks run sequentially, and each is awaited inside the turn.** `Plugin.trigger` loops over hooks with `yield* Effect.promise(async () => fn(input, output))` and returns the mutated `output` (VERIFIED: `[1.18.1] packages/opencode/src/plugin/index.ts:280-293`). A slow hook stalls the turn. A rejected promise becomes an Effect defect (INFERRED: this probably fails the operation; the docs use `throw` in `tool.execute.before` to block a tool).
- **`event` is fire-and-forget**, and only receives events whose `location.directory` equals the plugin's directory (VERIFIED: `plugin/index.ts:251-256`, `void hook["event"]?.(...)`).

### Hook table (VERIFIED signatures, `[1.18.1] packages/plugin/src/index.ts:222-335`; trigger sites as cited)

| Hook | When | What reaches the model | Notes for injection |
| --- | --- | --- | --- |
| `event({event})` | every bus event (session.status, session.idle, message.*, permission.*, tui.*, …) | nothing directly | The idle detector for a plugin wake. Event list: https://opencode.ai/docs/plugins/ (session.idle, session.status, session.compacted, message.updated, permission.asked, tui.prompt.append, …). |
| `chat.message(input, {message, parts})` | once per user prompt, before it is saved (`session/prompt.ts:999-1009`) | yes: mutated `parts` are persisted (`prompt.ts:1046-1047`) and sent | **Turn-start injection**, analogous to Claude `UserPromptSubmit`. Fires only when a user message is created, including via the API. Added parts are durable in history. |
| `experimental.chat.messages.transform({}, {messages})` | **before every LLM step** of the loop (`session/prompt.ts:1255`), also before the compaction request (`session/compaction.ts:350`) | yes: the in-memory `msgs` array is converted to model messages right after (`prompt.ts:1257-1262`) | **Per-step, mid-turn, ephemeral injection.** It runs after each tool round. Mutations are not persisted unless the plugin also writes parts. This is the closest analogue of a post-tool delivery that needs no model call. Marked experimental. |
| `experimental.chat.system.transform({sessionID, model}, {system})` | each LLM request (`session/llm/request.ts:68-72`), also agent generation (`agent/agent.ts:381`) | yes, as system prompt text | Per-request system injection. Mutating the system prompt every turn defeats prompt caching (INFERRED). |
| `tool.execute.before({tool, sessionID, callID}, {args})` | before every tool call (`session/tools.ts:106-110` and the MCP resource tools) | args only | Can rewrite args or throw to block (docs example). |
| `tool.execute.after({tool, sessionID, callID, args}, {title, output, metadata})` | after every tool call, before the result returns to the model (`session/tools.ts:121-129`) | **yes: the mutated `output.output` is the tool result the model sees** | Post-tool injection, analogous to Claude `PostToolUse` `additionalContext`. It appends inside a tool result, so the model cannot tell it from tool output unless it is framed. Applies to built-in, custom and MCP tools; `code-mode.ts:142,181` triggers it too. |
| `command.execute.before({command, sessionID, arguments}, {parts})` | slash command expansion (`prompt.ts:1461`) | yes (`parts`) | Injection on command use only. |
| `shell.env({cwd, sessionID?, callID?}, {env})` | bash tool spawn (`tool/shell.ts:416-425`) and PTY creation (`plugin/pty-environment.ts:18`, `server/.../handlers/pty.ts:71`) | no | **Identity channel:** a plugin can export e.g. `OPENCODE_SESSION_ID` into every agent-run shell command. `sessionID` is absent for user PTYs. |
| `experimental.session.compacting({sessionID}, {context, prompt?})` | before the compaction summary request (`compaction.ts:343-348`) | yes, into the summary prompt | Keeps "you have pending threads" state across compaction. |
| `experimental.compaction.autocontinue(..., {enabled})` | after compaction, before the synthetic "continue" user turn (`compaction.ts:454-456`) | controls whether an auto-continue turn happens | OpenCode itself appends a synthetic user turn to keep going after compaction, so it has a native self-continue mechanism. |
| `chat.params`, `chat.headers` | each LLM request (`session/llm/request.ts:115`) | sampling params and headers only | Not useful for content. |
| `permission.ask(input, {status})` | permission prompts | no | Could auto-allow the `herdr-threads` CLI. Docs list events `permission.asked` and `permission.replied`. |
| `tool` (map of `ToolDefinition`) | registers custom tools | tool results | A `herdr_threads_*` tool is possible, as is a blocking long-poll tool. Tool timeouts: unknown. |
| `tool.definition({toolID}, {description, parameters})` | tool schema build | tool descriptions | Static guidance only. |
| `experimental.text.complete` | after assistant text completes (`session/processor.ts:517`) | rewrites the stored assistant text | Not an injection surface. |
| `config`, `auth`, `provider`, `dispose` | lifecycle and config | no | `config` can add MCP servers and commands programmatically (INFERRED from the `Config` type). |

**No Stop-hook equivalent.** v1 has no hook that runs when the loop is about to exit and can return "block and continue". The loop exits when the last assistant message finished without tool calls and is newer than the last user message (VERIFIED: `[1.18.1] packages/opencode/src/session/prompt.ts:1111-1130`). The equivalent effect comes from `event` on `session.idle` (or `session.status` → idle) plus `client.session.promptAsync`, which starts a new turn (INFERRED composition of VERIFIED parts; not run).

## 3. Server HTTP API and SDK (inject and wake)

Docs: https://opencode.ai/docs/server/ and https://opencode.ai/docs/sdk/. Routes VERIFIED in `[1.18.1] packages/opencode/src/server/routes/instance/httpapi/groups/session.ts:80-104` and `groups/tui.ts:36-50`.

| Endpoint (SDK method) | Semantics | Use |
| --- | --- | --- |
| `POST /session/:id/message` (`client.session.prompt`) | creates a user message and runs the loop, then waits for the reply. Body: `parts[]` (`text`/`file`/`agent`/`subtask`), `agent`, `model`, `system`, `tools`, `noReply` (VERIFIED: `packages/sdk/js/src/gen/types.gen.ts:2590-2601`). | **Wake** an idle session with content. |
| `POST /session/:id/prompt_async` (`client.session.promptAsync`) | same body; returns 204 without waiting (VERIFIED in docs and `groups/session.ts:96,329-341`) | **Fire-and-forget wake**, the natural call from a plugin `event` handler. |
| `noReply: true` | saves the user message and returns without starting the loop (VERIFIED: `session/prompt.ts:1069`). Docs: "adds context without an AI response … useful for plugins". | **Silent delivery**: the item is in history at the next turn with no wake. It maps onto herdr-threads rule 2 (p > T: no wake). |
| `parts[].synthetic: true` / `parts[].ignored: true` | `synthetic` parts reach the model but the TUI hides them from user-text views (VERIFIED: TUI filters at `[1.18.1] packages/tui/src/routes/session/index.tsx:383-387,629`). `ignored` parts are dropped from model input (VERIFIED: `packages/opencode/src/session/message-v2.ts:206`). | Framing hook for "not typed by the human". |
| `system` (per message) | appended to the system prompt for that request (VERIFIED: `session/llm/request.ts:58-61`, `input.user.system`) | Per-message system note. |
| `POST /session/:id/command`, `/shell` | run a slash command or a shell command in the session | `/shell` errors with `BusyError` if busy (`session/run-state.ts:96-107`). |
| `POST /session/:id/abort` | cancels the run | |
| `GET /session/status` (`client.session.status`) | map of non-idle sessions: `busy`, or `retry` with attempt and next (VERIFIED: `session/status.ts:30-48`, `packages/schema/src/session-status-event.ts:9-32`) | **Idle/busy probe.** Idle sessions are absent from the map (they are deleted on idle). |
| `GET /event`, `GET /global/event` (SSE; `client.event.subscribe`) | bus events. The first event is `server.connected` (docs). | **Push idle detection**: `session.status` `{sessionID, status}` and the deprecated `session.idle` (VERIFIED: `session-status-event.ts:35-49`). |
| `POST /tui/append-prompt` | inserts text into the composer at the cursor and moves to the buffer end. It **does not clear an existing draft**; it concatenates (VERIFIED: `packages/tui/src/component/prompt/index.tsx:237-248`, `input.insertText(...)`). | Keystroke-free typing; still mixes with a human draft. |
| `POST /tui/submit-prompt`, `/tui/clear-prompt`, `/tui/execute-command`, `/tui/show-toast`, `/tui/select-session`, `/tui/publish` | drive the TUI composer and UI | `clear-prompt` + `append-prompt` + `submit-prompt` is a composer-level wake. It would destroy a draft. |
| `GET /tui/control/next`, `POST /tui/control/response` | a request/response queue for TUI control | Semantics not investigated: unknown. |

**Busy-time prompts are steered in, not rejected.** `prompt()` always persists the user message and then calls `loop()` → `state.ensureRunning(...)`. If a run is already `Running`, the caller simply awaits the existing run (VERIFIED: `[1.18.1] packages/opencode/src/effect/runner.ts:115-122`, `session/prompt.ts:1052-1071,1343-1347`). The running loop re-reads all messages at the top of every step (`prompt.ts:1092`) and continues while the newest user message is newer than the newest assistant message (`prompt.ts:1111-1116`). So a message sent mid-turn is seen at the next step boundary, after the current LLM call and tool round (VERIFIED by reading; not run). The TUI labels such messages ` QUEUED ` (`packages/tui/src/routes/session/index.tsx:1373-1436`) and has a "Manage queued prompts" binding (`<leader>q`, `packages/tui/src/config/keybind.ts:102`). Delivery needs no idle/busy branching for correctness. It needs it only for token policy: a p1 item sent with `prompt_async` to a busy session extends the current turn, which an idle-only wake rule would not do.

**Authentication:** `OPENCODE_SERVER_PASSWORD` / `OPENCODE_SERVER_USERNAME` enable basic auth (VERIFIED: CLI help for `run`/`attach`). An external listener is unauthenticated unless that is set (INFERRED).

## 4. CLI, headless modes, resume (VERIFIED from `opencode --help` and `<cmd> --help` on 1.18.1)

- `opencode [project]`: the TUI. Options: `--port`, `--hostname` (default `127.0.0.1`), `--mdns`, `--cors`, `-m/--model`, `-c/--continue`, `-s/--session <id>`, `--fork`, `--prompt <text>` (initial prompt), `--agent`, `--auto` (auto-approve permissions that are not denied), `--mini` (minimal UI; rejects network flags, `tui.ts:153-158`), `--pure`.
- `opencode run [message..]`: one-shot. Options: `--attach <url>` (run against a running server), `-s`, `-c`, `--fork`, `--format json` (raw JSON events), `-f` (files), `--command`, `-i/--interactive` (split-footer mode), `--dir`, `-p/-u` (basic auth).
  - **`opencode run --attach http://127.0.0.1:PORT -s <sessionID> "text"` injects into, and wakes, a live session of a server-backed TUI from the shell** (INFERRED from help plus the routing; not run).
- `opencode serve`: headless server; docs give default port 4096 (help shows `--port` default `0`). `opencode web`: server plus web UI. `opencode attach <url>`: TUI client for an existing server (`-s`, `-c`, `--fork`, `--dir`).
- `opencode acp`: an ACP (Agent Client Protocol) stdio agent. Advertises `loadSession`, MCP over http/sse, prompt `image` and `embeddedContext`, and session `close`/`fork`/`list`/`resume` (VERIFIED: `[1.18.1] packages/opencode/src/acp/service.ts:110-129`; sets `OPENCODE_CLIENT=acp`, `cli/cmd/acp.ts:23`). ACP is editor-driven: the client sends `session/prompt`. It is a wake channel only if herdr-threads becomes the ACP client, which would replace the TUI in the pane. Docs: https://opencode.ai/docs/acp/
- `opencode session list|delete`, `opencode export [sessionID]` (`--sanitize`), `opencode import <file>`, `opencode db` (database tools). Sessions live in SQLite (`OPENCODE_DB` flag; VERIFIED the name only).
- `opencode mcp add|list|auth|logout|debug`, `opencode plugin <module>` (installs a plugin and **updates config**: never run against the real config), `opencode agent create|list`, `opencode debug config|paths|skill|agent|…`.

## 5. MCP client

VERIFIED: `[1.18.1] packages/opencode/src/mcp/index.ts`.

- **Client capabilities:** only `roots` is advertised. `sampling`, `elicitation` and `tasks` are commented out with issue links #11948, #23066 and #28567 (`mcp/index.ts:39-49`). An MCP server therefore **cannot use sampling or elicitation to start model work.**
- **Notifications handled:**
  - `notifications/message` (logging) goes **to OpenCode's log only** (`mcp/index.ts:457-460,474-492`) and never reaches the model or the UI.
  - `notifications/tools/list_changed` refreshes the tool defs and publishes `ToolsChanged` (`mcp/index.ts:462-471`). The new tool set applies at the next step's `SessionTools.resolve` (INFERRED from `prompt.ts:1226`).
  - No handler is registered for `resources/updated`, `resources/list_changed` or `prompts/list_changed` (VERIFIED by grep). **Server-pushed notifications never reach the model.**
- **Resources:** exposed to the model through built-in tools (`list_mcp_resources`, templates, read), which exist only when some server advertises resources (`session/tools.ts:136-221`). These are model-pulled, with no push or subscription.
- **Server `instructions`:** included in the system prompt inside `<mcp_instructions>` (VERIFIED: `session/system.ts:112-125`, `mcp/index.ts:399,615-622`). This is a static per-session explanation slot (like a skill).
- **MCP prompts** become slash commands (`source: "mcp"`, VERIFIED: `command/index.ts:105-112`).
- **Tool output** is truncated through the `Truncate` service, which spills to a file with `outputPath` metadata (VERIFIED for the resource tools, `session/tools.ts:196-205`). INFERRED: the same applies to normal tools.

Conclusion: in OpenCode, MCP is a pull-only surface (tools and resources the model chooses to call) plus static instructions. It cannot wake a session.

## 6. Other context surfaces

- **Skills:** `SKILL.md` files from `{skill,skills}/**/SKILL.md` in OpenCode config dirs, and also from `.claude/skills/**/SKILL.md` and `.agents/skills/**/SKILL.md` (VERIFIED: `[1.18.1] packages/opencode/src/skill/index.ts:21-25`). This can be disabled with `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS` or `OPENCODE_DISABLE_EXTERNAL_SKILLS` (flag names VERIFIED). Skills are listed in the system prompt and become slash commands (`command/index.ts:134-147`). The external dirs are resolved under the home directory: `path.join(global.home, ".claude" | ".agents")` (VERIFIED: `skill/index.ts:187-191`). INFERRED: an installed `~/.claude/skills/herdr-threads/SKILL.md` would be picked up by OpenCode as-is.
- **Instructions:** `AGENTS.md`, plus `CLAUDE.md` and `~/.claude/CLAUDE.md` unless the Claude Code prompt is disabled (`OPENCODE_DISABLE_CLAUDE_CODE`; VERIFIED: `session/instruction.ts:62-66,122`). These are static.
- **Custom slash commands:** Markdown in `{command,commands}/**/*.md` (VERIFIED: `config/command.ts:15`). Templates support `$ARGUMENTS`, `$1`… and **`` !`cmd` `` shell expansion** (VERIFIED: `session/prompt.ts:1390-1391,1592`). A `/inbox` command whose template is ``!`herdr-threads inbox` `` would inline pending mail into a user turn. Docs: https://opencode.ai/docs/commands/
- **Read-tool reminders:** the built-in `read` tool appends loaded instruction files inside `<system-reminder>` (VERIFIED: `tool/read.ts:356`). This is an internal precedent for in-band framing.
- **Plan/build reminders:** OpenCode itself injects synthetic text parts into the last user message per step (VERIFIED: `session/reminders.ts:15-90`). This is the internal pattern the `messages.transform` hook generalises.

## 7. TUI behaviour relevant to keystroke injection

VERIFIED: `[1.18.1] packages/tui/src/component/prompt/index.tsx` and `config/keybind.ts`.

- **Keys:**
  - Enter = `input_submit` (`return`). Newline = `shift+return`, `ctrl+return`, `alt+return` or `ctrl+j` (`keybind.ts:163-164`).
  - Escape = `session_interrupt`, enabled only while not idle (`keybind.ts:97`, `index.tsx:391-397`). **An injected ESC while the session is busy aborts the turn.**
  - Leader = `ctrl+x` (`keybind.ts:41`).
  - `ctrl+c` / `ctrl+d` exit the app (`keybind.ts:48`). Never inject them.
- **Bracketed paste** is handled by OpenTUI's `onPaste` (`index.tsx:1395-1418`), including CR-only newline normalisation for Windows. A paste of 3 or more lines, or more than 150 characters, collapses into a `[Pasted ~N lines]` extmark that expands on submit, unless `experimental.disable_paste_summary` is set (`index.tsx:1205-1210`). An empty bracketed paste triggers clipboard paste (`index.tsx:1409-1411`).
- **Submit guard:** `submit()` refuses an empty input, refuses while an autocomplete popup is visible (`auto()?.visible`), and serialises overlapping submits (`index.tsx:930-966`). **An injected `/` or `@` opens autocomplete, and Enter then selects the completion instead of submitting** (INFERRED from the `auto()?.visible` guard).
- **Busy submit:** submitting while busy is not blocked in the composer path (no status check in `submitInner`, `index.tsx:946-1079`). The server queues and steers the message (section 3).
- **Draft:** the draft is TUI-local state. `/tui/append-prompt` concatenates onto it. Persisted prompt history and stash exist (INFERRED: unknown detail). **Herdr's composer read is the only external way to see the draft**, unless a TUI plugin is installed (below).

### TUI plugins

A separate plugin kind, exporting `tui: TuiPlugin` (VERIFIED: `[1.18.1] packages/plugin/src/tui.ts:581-634`). It receives `api`:

- `api.client`: the SDK, with `session.promptAsync` and the rest;
- `api.event.on(type, …)`: the bus;
- `api.state.session.status(sessionID)`: idle/busy/retry;
- `api.route.current`: which session is shown;
- `api.ui.toast`, `api.attention.notify` (desktop notification and sound);
- `api.slots.register`: UI slots, including `session_prompt` with a `ref` giving `TuiPromptRef {focused, current: {input, parts, mode}, set, reset, submit}` (`tui.ts:183-209,455-486`);
- `api.keymap`, `api.command` (legacy) for commands and slash entries.

**A TUI plugin can therefore read the exact composer draft and the focus state, and can wake only when the draft is empty and the session is idle. It can also show a non-model toast or notification for p>T items.** Config comes from `tui.json`/`tui.jsonc` in the global config dir, `.opencode/` dirs, `OPENCODE_CONFIG_DIR` and `OPENCODE_TUI_CONFIG` (VERIFIED: `packages/opencode/src/config/tui.ts:171-210`).

## 8. Idle/busy observability summary

| Signal | Source | Push/pull | Draft-aware |
| --- | --- | --- | --- |
| `session.status` / `session.idle` events | server bus → plugin `event` hook, SSE `/event` | push | no |
| `GET /session/status` | server | pull | no |
| `api.state.session.status()` + `TuiPromptRef.current.input` | TUI plugin | pull and reactive | **yes** |
| Herdr `agent.get` status for agent kind `opencode` | Herdr screen detection | pull | composer read via Herdr |

The Herdr row is INFERRED: the herdr-threads tests use `"opencode"` as an agent kind Herdr reports (`src/host/native.rs:5043-5044`), and the local `herdr` binary contains `opencode` strings.

## 9. OpenCode 2.x (dev, unreleased on the stable channel): a native inbox

**VERIFIED** on `[dev]`; behaviour not run. 2.x rewrites the plugin API (`@opencode/plugin`, Promise and Effect variants, `packages/plugin/src/README.md`). It adds a **durable per-session inbox with explicit delivery modes**, which is the closest native match to herdr-threads' delivery contract seen in any harness.

- **Inbox items:** `SessionInbox.Item` = `user | synthetic | compaction | move`, each with `delivery: "steer" | "queue"` (`packages/schema/src/session-inbox.ts:11-50`).
  - `steer` is promoted at the next step boundary mid-work.
  - `queue` waits for the idle boundary, when the session picks up fresh work (`packages/core/src/session/inbox.ts:44-49`).
- **`session.synthetic({sessionID, text, description?, metadata?, delivery?="steer", resume?})`:** "Durably admit synthetic session input and schedule execution unless resume is false". It is exposed as `POST /api/session/:sessionID/synthetic` (`packages/protocol/src/groups/session.ts:396-416`; implementation `packages/core/src/session/session.ts:283-322`).
  - `resume !== false` calls `execution.wake(sessionID)`.
  - `resume: false` admits silently, which maps onto rule 2.
  - A synthetic item reaches the model as a plain `role: "user"` message containing `text` (`packages/core/src/session/runner/to-llm-message.ts:274-275`). There is no automatic framing, so the sender must frame it.
- **Inbox management:** `GET /api/session/:id/inbox`, `DELETE /api/session/:id/inbox/:inboxID` (cancel), `PATCH /api/session/:id/inbox/:inboxID` (change delivery). `session.interrupt` takes `{resume}`; on resume only steer items restart work, while queued prompts stay parked (`core/src/session/execution.ts:149-167`). Also `POST /api/experimental/session/:id/wait`, `GET /api/session/active`, and `POST /api/session/:id/prompt` with `delivery`. Prompt and command default to `steer` (`core/src/session/prompt.ts:50`, `command.ts:32`).
- **Plugin hooks:**
  - `ctx.session.hook("prompt" | "context" | "compaction" | "generate" | "title" | "model.request" | "http.request" | "http.response" | "retry" | "experimental.ws.*")`. `context` exposes a mutable `{system, messages, tools, options}` right before provider dispatch (`packages/plugin/src/promise/session.ts:14-36,139-152`).
  - `ctx.tool.hook("execute.before" | "execute.after")`. `execute.after` exposes a mutable `result` or `error` (`promise/tool.ts:38-64`).
  - `ctx.event.subscribe()` and `ctx.session.{prompt,synthetic,interrupt,wait,…}` (`promise/session.ts:154-176`).
  - Built-in precedent: the plan plugin splices a reminder into `messages` in the `context` hook and persists it with `ctx.session.synthetic({..., resume: false})` (`packages/core/src/plugin/plan.ts:57-73`).
- **Crash recovery:** executions are claimed durably and resumed after a restart, with a bounded number of resume attempts (`core/src/session/execution/restart.ts:20-43,193-225`).
- **Compatibility:** no v1 hook names exist on dev (section 0). A v1 plugin written for 1.x will need a port for 2.x (INFERRED: whether 2.x ships a v1 shim is unknown; none was found).

## 10. Identity and session tracking

- The OpenCode main process sets `AGENT=1`, `OPENCODE=1` and `OPENCODE_PID=<pid>` in its environment (VERIFIED: `[1.18.1] packages/opencode/src/index.ts:75-77`). The bash tool spawns with `{...process.env, ...shell.env-hook output}` (VERIFIED: `tool/shell.ts:416-425`), so every agent-run `herdr-threads` call inherits those variables.
- **By default no session ID reaches the child.** A plugin must add one through `shell.env`, where `sessionID` and `callID` are provided for agent tool calls. User PTYs get `OPENCODE_TERMINAL=1` (`packages/core/src/pty.ts:174`) and no `sessionID`.
- Session IDs are stable across `-c`/`-s` resume. `--fork` creates a new ID (VERIFIED from help).
- One TUI process can host many sessions (session list, child sessions from the task tool, `<leader>` quick slots). The session shown in the pane is `api.route.current` (TUI plugin) or the `tui.session.select` event. **A pane does not map one-to-one to a session** (INFERRED; it matters for seat binding).

## 11. What herdr-threads does for OpenCode today

VERIFIED from this repo at `2de3da1c4`.

- **No adapter.** `builtins()` registers only Claude, Codex, Hermes and agy (`src/harness/registry.rs:1482-1490`). There is no `src/harness/opencode*` and no `integrations/opencode/`. The README lists "Pi, OpenCode | Planned" (`README.md:101`).
- **Herdr native wake refuses it.** An idle pane whose Herdr agent kind is `opencode` is "unrecognized": the wake is refused before any prompt, and no composer read happens (`src/host/native.rs:5018-5044`, `5983-5992`). OpenCode agents can still use the CLI manually, with no hooks, in-turn delivery or wakes (INFERRED).
- **Prior research:** `docs/research/harness-adapters-2026-10-03/other-harnesses-and-prior-art.md:7,11` noted only that OpenCode plugins are JS/TS hook objects and that `shell.env` carries optional `sessionID` and `callID`.

## 12. Candidate integration shapes (for the comparison reports)

These shapes are INFERRED design options built from the VERIFIED surfaces above. None has been run.

1. **Server plugin "channel"** (the analogue of the Claude mod):
   - Subscribe to the herdr-threads daemon from the plugin.
   - Deliver in turn through `experimental.chat.messages.transform` (per step, ephemeral, no model call) or `tool.execute.after` (appended to tool output).
   - Deliver at turn start through `chat.message`.
   - Wake on `p <= T` when `session.status` is idle via `client.session.promptAsync({parts:[{type:"text", text:"wake up", synthetic:true}]})`.
   - Stage silent items with `noReply: true`.
   - Benefits: no composer contact, so no draft clobbering, and no Herdr keystrokes.
   - Risk: an experimental hook name, plus a v2 API break.
2. **Server plugin plus a TUI plugin**, for draft-aware wakes (`TuiPromptRef.current.input === ""`) and non-model toasts for silent items.
3. **External HTTP** (launch the TUI with `--port`, or `serve` + `attach`): the daemon calls `prompt_async`/`noReply` and watches `/event` directly. This needs managed launch and auth (`OPENCODE_SERVER_PASSWORD`).
4. **Native keystroke wake via Herdr**, as for Codex/agy. It works through the composer and must avoid ESC (interrupt) and `/`/`@` autocomplete. It is the weakest fit, given options 1-3.
5. **On 2.x**: `POST /api/session/:id/synthetic` with `delivery: "steer" | "queue"` and `resume` maps almost directly onto herdr-threads priorities, i.e. resume iff `p <= T`.

Open questions (unknown):

- the tool-call timeout for a long-poll custom tool;
- whether `experimental.chat.messages.transform` survives into 1.19.x behaviour unchanged (the signature is identical);
- the semantics of `/tui/control/*`;
- whether the TUI shows `synthetic` user parts sent via the API at all;
- how Herdr detects OpenCode idle/busy.
