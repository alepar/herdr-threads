# opencode extension surface: research for a herdr-threads harness adapter

Date: 2026-11-14. Research only; no production code.

## Method and evidence

- **Observed version:** `opencode --version` = `1.18.1` (local binary `~/.opencode/bin/opencode`, embedded Bun 1.3.14). Facts are not pinned to it; it is only the version observed.
- **Source read:** `anomalyco/opencode` tag `v1.18.1`, commit `99f638d8293f6985726ba509da602296c4963497` (`github.com/sst/opencode` 301-redirects to `anomalyco/opencode`). Paths below are relative to that tree.
- **Docs read:** the docs shipped in the same tree, `packages/web/src/content/docs/*.mdx` (published at `https://opencode.ai/docs/<name>`): `plugins`, `config`, `cli`, `permissions`, `keybinds`, `skills`, `server`.
- **Live observation:** the binary run with `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `TMPDIR` under a scratch dir, `OPENCODE_DISABLE_AUTOUPDATE=1`, no credentials. A model was simulated by a local OpenAI-compatible mock server (`captures/mock-openai-server.py.txt`) configured as a custom provider in a project `opencode.json`; it logs every request (so what reaches the model is observed exactly) and emits scripted `bash` / `task` / `question` tool calls. A probe plugin (`captures/probe-plugin.js.txt`) logged every hook call and event, and drove SDK calls from inside the plugin. The TUI ran in a private tmux server at 120x40.
- **Confidence tags:** `CONFIRMED-observed` (seen in a live run), `CONFIRMED-source` (read in the tagged source), `CONFIRMED-docs`, `UNCONFIRMED`.
- **Captures:** `captures/NN-*.txt` (`capture-pane -p`), `.ansi` (`capture-pane -e -p`), `.meta` (tmux `alternate_on`, cursor, mouse flags); `captures/cli-help.txt`.
- Not covered: a real model (model-dependent behaviour is out of scope here), macOS, Windows, the desktop/web clients, ACP mode.

---

## 1. Process identity

| Fact | Evidence | Tag |
|---|---|---|
| The binary is one self-contained ELF executable (178 MB, Bun-compiled); no node wrapper. | `file -L ~/.opencode/bin/opencode`: `ELF 64-bit LSB executable, x86-64, dynamically linked`. Plugin saw `Bun.version = 1.3.14`, `process.execPath = ~/.opencode/bin/opencode`. | CONFIRMED-observed |
| The TUI is **one OS process**: `comm=opencode`, `/proc/<pid>/exe` = the binary, `cmdline` = the binary plus user args (`/var/home/.../opencode` with no args for a plain start). No child processes at rest. | `pstree -ap` showed `sh → opencode` only; `cat /proc/P/comm` = `opencode`; 36 threads (72 with `--port`). | CONFIRMED-observed |
| Server/backend runs as a **Bun Worker thread inside the same process**, not a separate process. The TUI thread talks to it over an in-process RPC "fetch"; plugins run in the worker. | `packages/opencode/src/cli/cmd/tui.ts` (`new Worker(file, {env})`, `createWorkerFetch`), `src/cli/tui/worker.ts`. Plugin init saw `Bun.isMainThread = false`, `process.argv = ['bun', '/$bunfs/root/src/cli/tui/worker.js']`, same pid as the TUI. | CONFIRMED-source + observed |
| A shell command run by the agent's `bash` tool is a **direct child of the `opencode` process**, in its own session and process group (`setsid`), with no controlling tty. A compound command runs under `/usr/bin/bash -c <cmd>` (the user's `$SHELL`); a single simple command is exec'd by that bash, so the command itself is the direct child. | Observed chain (TUI): `show.sh(sid=pid, tty=?) → opencode(pid 411873, tty=pts/24) → sh → tmux: server`. With `cmd; sleep 4`: `show.sh → bash -c ... (sid=pid) → opencode`. Source: `src/tool/shell.ts` `cmd()` uses `detached: process.platform !== "win32"`, `stdin: "ignore"`. | CONFIRMED-observed + source |
| Subagent (`task` tool) shell commands have the same parent: the one `opencode` process. | Observed: child-session bash chain ends at the same pid; `OPENCODE_PID` identical. | CONFIRMED-observed |
| In **client/server mode** (`opencode serve` + `opencode attach <url>` or `run --attach`), the shell tool's parent is the **server** process (`opencode serve ...`, no tty), not the TUI client in the pane. Plugins also run in the server process (there on the main thread). | Observed with `serve --port` + `run --attach`: chain `show.sh → opencode serve --port 18767 (tty=?)`. | CONFIRMED-observed |
| `opencode run` (non-interactive) is likewise one process; same env and chain shape. | Observed. | CONFIRMED-observed |

Implication: walking the parent chain from a shell-tool command reaches the `opencode` TUI process (and from it the pane) in the default single-process mode; it does not in attach mode.

## 2. Session id and environment

| Fact | Evidence | Tag |
|---|---|---|
| opencode sets exactly three variables on its own process, inherited by every shell-tool command: `AGENT=1`, `OPENCODE=1`, `OPENCODE_PID=<pid of the opencode process>`. | `src/index.ts` yargs middleware (lines ~75-77). Observed in tool output: `AGENT=1`, `OPENCODE=1`, `OPENCODE_PID=411873`. | CONFIRMED-source + observed |
| **No session-id variable is set natively.** The shell env is `{...process.env, ...<plugin shell.env output>}`. | `src/tool/shell.ts` `shellEnv`. Observed env had no `OPENCODE_SESSION*`. | CONFIRMED-source + observed |
| A plugin **can add env to every shell tool call** through the `shell.env` hook, whose input carries `{cwd, sessionID, callID}`. | Probe set `PROBE_SESSION_ID`; the command saw `PROBE_SESSION_ID=ses_e26b86463ffe00SJEoEd8WYiQk`, `PROBE_CALL_ID=call_7`. In a subagent it saw the **child** session id. | CONFIRMED-observed |
| `shell.env` is also triggered for the user `!shell` composer mode (`session/prompt.ts:554`, with `sessionID`) and for PTY creation (`server/.../handlers/pty.ts`, **without** `sessionID`). | Source only. | CONFIRMED-source |
| `OPENCODE_CALLER` is only *read* (IDE detection, `src/ide/index.ts`), never set for children. `OPENCODE_TERMINAL=1` is set only for PTY terminals (`core/src/pty.ts`). `OPENCODE_CLIENT` defaults to `cli` and is `acp` under `opencode acp`. | Source. | CONFIRMED-source |
| Session ids look like `ses_` + 26 chars, e.g. `ses_e26b86463ffe00SJEoEd8WYiQk` (prefix `ses`, hex time part + random base62). Messages `msg_...`, parts `prt_...`, permissions `per_...`, questions `que_...`. | `core/src/id/id.ts`; observed. | CONFIRMED-source + observed |
| The provider request carries the session id in headers `x-session-id` and `x-session-affinity` (openai-compatible provider). | Mock server log. | CONFIRMED-observed |
| **A session is created lazily, at the first prompt**, not at TUI start: a fresh TUI (home screen) has no session; `session.created` fires when the first message is submitted. | Observed: no session event at startup; `session.created` then `chat.message` on first Enter. | CONFIRMED-observed |
| **`/new`**: same process, returns to the home screen, emits **no server event**; the next prompt creates a new session id. | Observed: probe log empty after `/new`; `session.created` with a new id on the next prompt. | CONFIRMED-observed |
| **Session switch (`/sessions`, `ctrl+x l`)**: same process; **no event reaches a server plugin**; the next prompt's hooks carry the switched-to session id. | Observed. | CONFIRMED-observed |
| **`--continue` / `--session <id>`**: new process, **same session id**; no session event at startup (plugin `init` + `config` only); the id first appears in the next `chat.message`. | Observed. | CONFIRMED-observed |
| **`--fork`** (with `--continue`/`--session`): a **new session id** is created at startup (`session.created`, title suffixed `(fork #1)`), no `parentID`. | Observed. | CONFIRMED-observed |
| **Compaction (`/compact`, auto)**: **same session id**; `experimental.session.compacting` hook, then `session.compacted {sessionID}`. | Observed. | CONFIRMED-observed |
| One process holds many sessions at once (the displayed one, others in the list, child sessions); any of them can run a turn. | Observed (prompting a non-displayed session over HTTP ran it). | CONFIRMED-observed |
| On exit the TUI prints `Continue  opencode -s <ses_id>`. | Observed. | CONFIRMED-observed |

Implication: there is no ready-made "session id claim" in the shell env; the adapter's plugin must add one via `shell.env` (which is per tool call and correct for child sessions). The current session is unknowable to a server plugin until a hook fires for it.

## 3. Plugin API

### Locations and loading

| Fact | Evidence | Tag |
|---|---|---|
| Local plugins: `{plugin,plugins}/*.{ts,js}` in every **config directory**. Config directories are: the global config dir (`$XDG_CONFIG_HOME/opencode`), every `.opencode/` from the cwd up to the worktree root, `$HOME/.opencode/`, and `$OPENCODE_CONFIG_DIR` if set. | `src/config/plugin.ts` (`Glob.scan("{plugin,plugins}/*.{ts,js}")`), `src/config/paths.ts` (`directories`). Observed: plugins loaded from `$XDG_CONFIG_HOME/opencode/plugins/`, `<proj>/.opencode/plugins/` and `$OPENCODE_CONFIG_DIR/plugins/` in one run (three `init` calls). | CONFIRMED-source + observed |
| npm plugins: `"plugin": ["name", ["name", {options}]]` in `opencode.json`; path-like specs resolve relative to the declaring config file; installed with Bun at startup into the cache dir. `opencode plugin <module> [-g]` installs and edits config. | `docs/plugins.mdx`, `src/config/plugin.ts`, `opencode plugin --help`. | CONFIRMED-docs + source |
| A plugin module exports one or more async functions (legacy: every export must be a plugin function) or a default `{ id?, server }` object (v1 module). Signature: `(input, options?) => Promise<Hooks>`. | `packages/plugin/src/index.ts` (`Plugin`, `PluginModule`), `src/plugin/index.ts` (`getLegacyPlugins`, `applyPlugin`). | CONFIRMED-source |
| Plugin input keys: `client` (SDK v1 client), `project`, `directory`, `worktree`, `experimental_workspace`, `serverUrl`, `$` (Bun shell). | Observed `Object.keys(input)`; `PluginInput` type. | CONFIRMED-observed + source |
| The plugin runs under the embedded Bun runtime (no external Bun/Node needed for a dependency-free plugin); `node:fs` etc. work. | Observed. | CONFIRMED-observed |
| Plugin `init` runs **at TUI start** (instance bootstrap), before any session exists. It can run more than once per process (observed twice per plugin file with `--port`, and after a config reload via `SIGUSR2`); a `dispose` hook exists. | Observed; `src/cli/tui/worker.ts` `reload`. | CONFIRMED-observed |
| `--pure` (or `OPENCODE_PURE=1`) disables all external plugins. | Help text; observed (probe log empty under `run --pure`). | CONFIRMED-observed |
| A separate **TUI plugin** kind exists (module exporting `{ tui }`, declared in `tui.json` `plugin`, `OPENCODE_TUI_CONFIG`), with `api.route.current` (`{name:"home"}` or `{name:"session", params:{sessionID}}`), `api.state.session.status/permission/question`, an event bus, dialogs and toasts. Not exercised. | `packages/plugin/src/tui.ts`, `src/config/tui.ts`. | CONFIRMED-source (behaviour UNCONFIRMED) |

### Hook list (exact names, `packages/plugin/src/index.ts` `Hooks`)

All `(input, output) => Promise<void>` hooks mutate `output` in place; hooks from all plugins run sequentially in load order (`Plugin.trigger`).

| Hook | Input | Output (mutable) | Sees session id | Can inject model-visible text | Tag |
|---|---|---|---|---|---|
| `event` | `{ event: {id, type, properties} }` | none | per event (`properties.sessionID`) | no (read-only; fire-and-forget, not awaited) | observed |
| `config` | merged config | config | no | no | observed |
| `tool` | map of custom tool definitions | n/a | via tool ctx | yes, as a tool result | source |
| `chat.message` | `{sessionID, agent?, model?, messageID?, variant?}` | `{message: UserMessage, parts: Part[]}` | yes | **yes**: pushing a `{type:"text", text, synthetic:true, id, sessionID, messageID}` part adds text the model sees in that user message; it is persisted and hidden in the TUI bubble | observed |
| `chat.params` | `{sessionID, agent, model, provider, message}` | `{temperature, topP, topK, maxOutputTokens, options}` | yes (+`agent`) | no | observed |
| `chat.headers` | same as `chat.params` | `{headers}` | yes (+`agent`) | no | observed |
| `permission.ask` | `Permission` | `{status: "ask"\|"deny"\|"allow"}` | - | no. **Declared but never triggered** in this version (no `plugin.trigger("permission.ask")` call site; not seen when a permission was asked) | source + observed |
| `command.execute.before` | `{command, sessionID, arguments}` | `{parts}` | yes | yes (slash-command parts) | source |
| `tool.execute.before` | `{tool, sessionID, callID}` | `{args}` | yes | no (can rewrite args) | observed |
| `shell.env` | `{cwd, sessionID?, callID?}` | `{env}` | yes (for tool calls) | no | observed |
| `tool.execute.after` | `{tool, sessionID, callID, args}` | `{title, output, metadata}` | yes | **yes**: appending to `output.output` reaches the model as part of the tool result (and shows in the TUI tool block) | observed |
| `experimental.chat.messages.transform` | `{}` | `{messages: {info, parts}[]}` | only via `messages[].info.sessionID` | yes (rewrites the message list sent to the model; not persisted) | observed (fires), injection by source |
| `experimental.chat.system.transform` | `{sessionID?, model}` | `{system: string[]}` | yes (optional) | **yes**: a pushed string became a second `system` message on every model call | observed |
| `experimental.session.compacting` | `{sessionID}` | `{context: string[], prompt?}` | yes | yes (compaction prompt only) | observed (fires) |
| `experimental.compaction.autocontinue` | `{sessionID, agent, model, provider, message, overflow}` | `{enabled}` | yes | no | source |
| `experimental.text.complete` | `{sessionID, messageID, partID}` | `{text}` | yes | rewrites assistant text | observed (fires) |
| `experimental.provider.small_model` | `{provider}` | `{model?}` | no | no | source |
| `tool.definition` | `{toolID}` | `{description, parameters}` | no | yes (tool descriptions, every step) | observed (fires) |
| `auth`, `provider`, `dispose` | provider auth / model list / teardown | - | - | - | source |

Notes on injection points:

- **Turn start:** `chat.message` fires once per user message (TUI submit, `--prompt`, `promptAsync`, HTTP, subagent task prompt, queued messages), **when the message is stored**, which for a queued message is while the previous step is still running. `experimental.chat.system.transform` fires on **every** model call, including the title generator (`agent: "title"`, small model), compaction, and each tool-loop step; its input has no `agent`, so it cannot tell a title call from a main call (the `chat.params`/`chat.headers` hook fired for the same call does carry `agent`).
- **After a tool call:** `tool.execute.after` output mutation, confirmed for `bash`, `task` and `question`. Call sites: `src/session/tools.ts` (built-in, MCP, MCP-resource tools), `src/session/prompt.ts` (`task` via subtask), `src/tool/code-mode.ts`.
- **Mid-turn without a tool:** a `noReply` user message (section 4) is picked up by the next step of the running turn.
- **Child-session visibility:** hook inputs carry only `sessionID`, never `parentID`. Whether a session is a child is known from the `session.created` event (`properties.info.parentID`) or `client.session.get({path:{id}})` (returns `parentID` for a child; observed), or `client.session.children`.

### Events delivered to the `event` hook

Delivered for the plugin's own directory only (`event.location.directory === ctx.directory`), as `{id, type, properties}` (`src/plugin/index.ts`). Observed in live runs: `session.created`, `session.updated`, `session.status`, `session.idle`, `session.compacted`, `session.diff`, `session.error` (source), `message.updated`, `message.part.updated`, `message.part.delta`, `permission.asked`, `permission.replied`, `question.asked`, `question.replied`, `tui.prompt.append`, `tui.command.execute`, `tui.toast.show`, `plugin.added`, `catalog.updated`, `reference.updated`, `integration.updated`. The v2 SDK type union (`packages/sdk/js/src/v2/gen/types.gen.ts`) additionally lists `session.deleted`, `message.removed`, `message.part.removed`, `todo.updated`, `command.executed`, `file.edited`, `file.watcher.updated`, `tui.session.select`, `question.rejected`, `permission.v2.*`, `question.v2.*`, `session.next.*`, `pty.*`, `lsp.updated`, `vcs.branch.updated`, `mcp.tools.changed`, `server.connected`, `global.disposed`, `installation.*`, `workspace.*`, `worktree.*` (CONFIRMED-source, not all observed). The published docs list (`plugins.mdx`) is older (`permission.asked`, `session.idle`, `session.status`, `tui.*`, ...).

Observed payload shapes:

```
session.created   {sessionID, info:{id, slug, version, projectID, directory, path, title, parentID?, agent?, permission?, cost, tokens, time}}
session.status    {sessionID, status:{type:"busy"}} | {sessionID, status:{type:"idle"}}
session.idle      {sessionID}
session.compacted {sessionID}
permission.asked  {id:"per_...", sessionID, permission:"bash", patterns:[...], metadata:{command}, always:["touch *"], tool:{messageID, callID}}
question.asked    {id:"que_...", sessionID, questions:[{question, header, options:[{label, description}]}], tool:{messageID, callID}}
question.replied  {sessionID, requestID, answers:[["Red"]]}
tui.prompt.append {text}        tui.command.execute {command:"prompt.submit"}
```

No event is emitted for TUI navigation (`/new`, session switch, dialogs opening) to a server plugin (CONFIRMED-observed).

## 4. Submitting a turn from outside or from a plugin

| Fact | Evidence | Tag |
|---|---|---|
| **A plugin can wake an idle session itself.** `client.session.promptAsync({path:{id}, body:{parts:[{type:"text", text}]}})` returned HTTP 204 and started a turn on an idle session; the message appears in the TUI as a user message and the reply renders normally. This works **in-process, with no HTTP port open** (the plugin's client is wired to `Server.Default().app.fetch` when the server has no URL). | `src/plugin/index.ts` (client construction); observed `ctl.wake → status 204 → session.status busy → ... → idle`. | CONFIRMED-observed + source |
| `promptAsync` does **not touch the composer**: a typed draft (`kept draft`) stayed intact while the plugin-initiated turn ran. | `captures/14-subagent-running-with-draft.txt`. | CONFIRMED-observed |
| **Busy session:** a prompt (typed + Enter, or `promptAsync`) is **stored immediately as a user message, shown with a `QUEUED` badge, and consumed by the next model step of the running loop**; nothing is rejected or lost. Two messages queued during one step were both delivered in the same following request, as consecutive `user` messages, with no wrapper text. `chat.message` fires at queue time. | `captures/11-busy-with-queued-messages.txt`; mock request log; `src/session/prompt.ts` (`prompt` always `createUserMessage`, then `loop` → `SessionRunState.ensureRunning` joins the existing runner; `runLoop` re-reads messages each step and exits only when the last assistant message is newer than the last user message). | CONFIRMED-observed + source |
| **`noReply: true`** (`client.session.prompt({body:{noReply:true, parts}})`): stores a user message and returns it **without starting a turn**. Idle: no `session.status` change, the message shows in the transcript and is in context for the next turn. Mid-turn (during a tool call): shown `QUEUED` and included in the **next step of the same turn**, right after the tool result. | Observed both; `src/session/prompt.ts:1069`. | CONFIRMED-observed |
| `client.tui.appendPrompt({body:{text}})` + `client.tui.submitPrompt()` work from a plugin (events `tui.prompt.append`, `tui.command.execute {command:"prompt.submit"}`), but they **append to and submit the user's draft**: `my draft` + `TUI-APPENDED` was submitted as one message. | Observed. | CONFIRMED-observed |
| `client.tui.showToast({body:{message, variant}})` shows a toast box top-right. | `captures/13-toast.txt`. | CONFIRMED-observed |
| Other TUI controls in the SDK: `tui.clearPrompt`, `tui.executeCommand`, `tui.openHelp/openSessions/openThemes/openModels`, `tui.publish`, `tui.control.next/response`. | `packages/sdk/js/src/gen/sdk.gen.ts`. | CONFIRMED-source (untested) |
| **The TUI exposes no HTTP port by default.** A listener exists only when `--port`, `--hostname` or `--mdns` is passed on the command line (config `server.port` alone is not consulted by the TUI's "external" check). | `src/cli/cmd/tui.ts` (`external = hasArg("--port") || hasArg("--hostname") || network.mdns === true`); observed: no listening socket by default, `127.0.0.1:18766` with `--port 18766`. Plugin `serverUrl` is the placeholder `http://localhost:4096/` without a port, the real URL with one. | CONFIRMED-source + observed |
| With `--port`, the HTTP API works from outside: `POST /session/{id}/prompt_async` → 204 and the turn runs in the TUI; `GET /session`, `GET /session/status`. `OPENCODE_SERVER_PASSWORD` enables HTTP Basic auth (user `opencode` or `OPENCODE_SERVER_USERNAME`): 401 without, 200 with. Without a password `serve` warns the server is unsecured. | Observed with curl. | CONFIRMED-observed |
| `opencode serve` (headless), `opencode attach <url>`, `opencode run --attach <url>` exist. | Help; observed `serve` + `run --attach`. | CONFIRMED-observed |

Implication: opencode can be a `Channel`-style harness (the plugin submits the wake turn itself, composer untouched, busy case queued in-turn) instead of a `Native` composer-typing wake.

## 5. Idle/busy detection

| Fact | Evidence | Tag |
|---|---|---|
| `session.status` event: `{sessionID, status}` with `status` one of `{type:"idle"}`, `{type:"busy"}`, `{type:"retry", attempt, message, action?, next}`. | `packages/schema/src/session-status-event.ts`. | CONFIRMED-source (`retry` not observed) |
| `session.idle` event: `{sessionID}`; published together with every `status: idle`; marked `// deprecated` in source. | Same file; `src/session/status.ts` `set`. Observed after every turn. | CONFIRMED-source + observed |
| `busy` is published at the start of the run and again at each loop step (several `busy` events per turn); `idle` once when the runner finishes (also after an interrupted/rejected turn and after `/compact`). | Observed sequences; `src/session/run-state.ts` (`onBusy`/`onIdle`), `prompt.ts` `runLoop`. | CONFIRMED-observed + source |
| A session waiting on a permission prompt or a question stays `busy` (no status change); the wait is signalled by `permission.asked` / `question.asked` and ended by `*.replied`. | Observed. | CONFIRMED-observed |
| `client.session.status()` returns a map of **non-idle** sessions only; an idle session is absent (`{}` when all idle). State is in memory, per process. | Observed `{}`; `status.ts` deletes on idle. | CONFIRMED-observed + source |
| Child sessions emit their own `session.status`/`session.idle` with the child id; the parent stays `busy` while the `task` tool runs. | Observed. | CONFIRMED-observed |
| A noReply message causes no status event. | Observed. | CONFIRMED-observed |

## 6. Subagents

| Fact | Evidence | Tag |
|---|---|---|
| The `task` tool (`{description, prompt, subagent_type, task_id?, command?, background?}`) creates a **child session** via `sessions.create({parentID: ctx.sessionID, ...})`: new `ses_` id, `info.parentID` = parent id, title `"<description> (@<agent> subagent)"`, `agent`, `permission: [{permission:"task", pattern:"*", action:"deny"}]`. | `src/tool/task.ts:144`; observed `session.created`. | CONFIRMED-source + observed |
| **All plugin hooks fire for the child session with the child's `sessionID`**: `chat.message` (with `agent`, `model`, `messageID`), `experimental.chat.system.transform`, `tool.execute.before/after`, `shell.env`, plus `session.status`/`session.idle` events. | Observed sequence. | CONFIRMED-observed |
| Shell env in a child differs only by what the plugin's `shell.env` adds (it receives the child id); `AGENT`, `OPENCODE`, `OPENCODE_PID` are identical; same parent process. | Observed. | CONFIRMED-observed |
| The parent sees the child result as the `task` tool output `<task id="ses_..." state="completed"><task_result>...`, and `tool.execute.after` fires in the parent for `task`. | Observed. | CONFIRMED-observed |
| `task_id` resumes an earlier child session; `background: true` runs it asynchronously (gated by `OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS`). | `src/tool/task.ts`, `cli.mdx`. | CONFIRMED-source (untested) |
| Child sessions are hidden from `opencode session list` and the TUI session picker, but returned by `GET /session` (with `parentID`) and `client.session.children`. TUI shows `ctrl+x down view subagents`. | Observed. | CONFIRMED-observed |
| A `promptAsync` into a child session id would be accepted like any session. | Follows from source; not tested. | UNCONFIRMED |

## 7. Config locations and overrides

| Fact | Evidence | Tag |
|---|---|---|
| Global paths (Linux): config `$XDG_CONFIG_HOME/opencode`, data `$XDG_DATA_HOME/opencode` (`opencode.db` SQLite, `log/`, `snapshot/`, `repos/`), state `$XDG_STATE_HOME/opencode` (`model.json`, `prompt-history.jsonl`, `locks/`), cache `$XDG_CACHE_HOME/opencode` (`models.json`, `bin/`), tmp `os.tmpdir()/opencode`. | `opencode debug paths`; `core/src/global.ts`; `ls`. | CONFIRMED-observed + source |
| **Credentials:** `$XDG_DATA_HOME/opencode/auth.json` (also `mcp-auth.json`); `OPENCODE_AUTH_CONTENT` (JSON) overrides the file content. Provider keys are also picked up from the environment (`ANTHROPIC_API_KEY` listed under "Environment" by `opencode providers list`). | `src/auth/index.ts`; observed `providers list`. | CONFIRMED-source + observed |
| Config merge order (later wins): remote `.well-known/opencode` → global (`config.json`, `opencode.json`, `opencode.jsonc` in the config dir) → `OPENCODE_CONFIG` file → project `opencode.json[c]` (from cwd up to the worktree) → `opencode.json[c]` inside each `.opencode/` dir and `OPENCODE_CONFIG_DIR` → `OPENCODE_CONFIG_CONTENT` (inline JSON) → account/org config → managed dir (`/etc/opencode` on Linux) → macOS MDM. `OPENCODE_PERMISSION` (JSON) is deep-merged into `permission` after all of it. | `src/config/config.ts`; `docs/config.mdx`; observed `OPENCODE_CONFIG_CONTENT` and `OPENCODE_PERMISSION` in `opencode debug config`. | CONFIRMED-source + observed |
| `OPENCODE_CONFIG_DIR` adds a config directory (its `opencode.json`, `plugins/`, `skills/`, `commands/`, agents load) **in addition to** the XDG one, and replaces the XDG dir as the location of the global `AGENTS.md`. | Observed: plugins from both dirs loaded; global `AGENTS.md` in the XDG dir was not loaded when `OPENCODE_CONFIG_DIR` was set; `core/src/global.ts` `make()`. | CONFIRMED-observed + source |
| `OPENCODE_DISABLE_PROJECT_CONFIG` skips project `opencode.json` and project `.opencode/` dirs. Other switches: `OPENCODE_DISABLE_AUTOUPDATE`, `OPENCODE_DISABLE_DEFAULT_PLUGINS`, `OPENCODE_DISABLE_CLAUDE_CODE[_PROMPT|_SKILLS]`, `OPENCODE_DISABLE_EXTERNAL_SKILLS`, `OPENCODE_DISABLE_MODELS_FETCH`, `OPENCODE_DISABLE_AUTOCOMPACT`, `OPENCODE_DISABLE_MOUSE`, `OPENCODE_DISABLE_TERMINAL_TITLE`, `OPENCODE_TUI_CONFIG`, `OPENCODE_ENABLE_QUESTION_TOOL`, `OPENCODE_EXPERIMENTAL*`. | `cli.mdx` env table; `src/config/paths.ts`; grep of `OPENCODE_*` in source. | CONFIRMED-docs + source |
| **opencode writes into every config directory** at startup: `.gitignore`, `package.json` (dependency `@opencode-ai/plugin`), `package-lock.json`/`bun.lock`, `node_modules/` (network install, background). The global config dir also gets a seeded `opencode.jsonc` (`{"$schema": ...}`) unless `OPENCODE_CONFIG`, `OPENCODE_CONFIG_DIR` or `OPENCODE_CONFIG_CONTENT` is set. | Observed in the XDG config dir, `<proj>/.opencode/` and `$OPENCODE_CONFIG_DIR`; `src/config/config.ts` (`ensureGitignore`, `npmSvc.install`, `loadGlobal`). | CONFIRMED-observed + source |
| **Fully isolated run:** set `HOME`, the four `XDG_*_HOME`, `TMPDIR` (else `/tmp/opencode` is created), `OPENCODE_DISABLE_AUTOUPDATE=1`; nothing under the real home was touched. Credentials for such an instance: provider env keys, or a copy of `auth.json` in `$XDG_DATA_HOME/opencode/`, or `OPENCODE_AUTH_CONTENT`. A custom provider can be declared in `opencode.json` (`provider.<id>.npm = "@ai-sdk/openai-compatible"`, `options.baseURL`, `models`). | Observed (whole study ran this way; `auth.json` copy and `OPENCODE_AUTH_CONTENT` by source only). | CONFIRMED-observed (auth copy: CONFIRMED-source) |
| **Instructions:** project `AGENTS.md` (first match walking up; `CLAUDE.md` fallback unless disabled; `CONTEXT.md` deprecated), global `<config>/AGENTS.md` and `~/.claude/CLAUDE.md`, plus config `"instructions": [paths/globs/URLs]`. | `src/session/instruction.ts`; observed project `AGENTS.md` and an `instructions` file in the system prompt. | CONFIRMED-source + observed |
| **Skills:** `SKILL.md` files are discovered from `{skill,skills}/**/SKILL.md` in every config dir (XDG config, `.opencode/`, `~/.opencode/`, `OPENCODE_CONFIG_DIR`), from `skills/**/SKILL.md` under `~/.claude`, `~/.agents` and project `.claude` / `.agents`, and from config `skills.paths` / `skills.urls`. They are listed in the system prompt (`<available_skills>`) and loaded on demand by the `skill` tool. | `src/skill/index.ts`; observed all nine test skills in `opencode debug skill` and in the system prompt. | CONFIRMED-source + observed |
| An existing `~/.agents/skills/<name>/SKILL.md` or `~/.claude/skills/...` is therefore visible to opencode without any install step. | Same. | CONFIRMED-observed |
| **Custom commands:** `{command,commands}/**/*.md` in config dirs (frontmatter + template, `$ARGUMENTS`) or config `command`. | `src/config/command.ts`; observed `htcmd` in `debug config`. | CONFIRMED-source + observed |
| **Bash permission:** `"permission": {"bash": {"*": "ask", "herdr-threads *": "allow"}}`; simple wildcards, **last matching rule wins**. A compound command is parsed (tree-sitter) and each sub-command is checked; "Allow always" offers `<prefix> *`. Paths outside the project also need `external_directory`. `opencode run` auto-rejects an `ask` (no UI); `--auto` approves everything not denied. | `docs/permissions.mdx`; observed `OPENCODE_PERMISSION='{"bash":{"herdr-threads *":"allow"}}'` merged; observed prompt for `touch *`; `run` printed `permission requested ... auto-rejecting`. | CONFIRMED-docs + observed |

## 8. CLI launch

From `opencode --help` (saved in `captures/cli-help.txt`) and `src/cli/cmd/tui.ts`:

- `opencode [project]`: start the TUI; positional `project` is the directory to start in (resolved against `$PWD`, then `chdir`). CONFIRMED-observed (help) + source.
- `--prompt <text>`: initial prompt, **submitted automatically** at startup (observed: session created and the turn ran with no keypress). If stdin is not a TTY its content is read to EOF and prepended to the prompt. CONFIRMED-observed + source.
- `-m, --model provider/model`; `--agent <name>` (observed: `--agent plan` made the first message `agent: "plan"`); `-c, --continue` (last session); `-s, --session <id>` (validated at startup, error exit on an unknown id); `--fork` (needs `-c` or `-s`). CONFIRMED-observed.
- `--auto` (hidden aliases `--yolo`, `--dangerously-skip-permissions`): auto-approve permissions not explicitly denied. `--pure`: no external plugins. `--port`, `--hostname`, `--mdns`, `--cors`: expose the server. `--mini`: a different, minimal interface (section 9). `--print-logs`, `--log-level`. CONFIRMED-observed (help) / source.
- `opencode run [message..]`: non-interactive; `--format json`, `-c`, `-s`, `--fork`, `-m`, `--agent`, `-f file`, `--title`, `--attach <url>`, `--dir`, `--command`, `--auto`. **It reads stdin to EOF when stdin is not a TTY** and hangs on an open pipe; run it with `</dev/null` (observed two hangs, fixed by redirecting stdin). CONFIRMED-observed + source (`src/cli/cmd/run.ts:416`).
- Diagnostics: `opencode debug paths|config|skill|info`, `opencode session list`, `opencode providers list`. CONFIRMED-observed.

## 9. TUI composer

Raw startup sequence (pipe-pane): `ESC[?1049h` (alternate screen), `ESC[>4;1m` (modifyOtherKeys), `ESC[?u` (kitty keyboard query), `ESC[?2004h` (bracketed paste), mouse `?1000h ?1002h ?1003h ?1006h`, focus `?1004h`, sync output `?2026h`.

| Fact | Evidence | Tag |
|---|---|---|
| Full-screen TUI on the **alternate screen**, mouse reporting on (tmux `alternate_on=1`, `mouse_any_flag=1`). | `.meta` files. | CONFIRMED-observed |
| Kitty keyboard protocol: queried at startup and requested through opentui (`useKittyKeyboard: {}` in `packages/tui/src/app.tsx`); plain tmux did not answer, and legacy key input worked. Behaviour on a kitty-capable terminal not observed. | Raw log; source. | CONFIRMED-source (enabled path UNCONFIRMED) |
| **Home screen (no session):** logo, then a composer box centred horizontally and vertically (cols 24-98, rows 19-23 at 120x40): left border `┃` (blue, color 75), dim placeholder `Ask anything... "<example>"` (color 8; the example rotates: "Fix broken tests", "What is the tech stack of this project?", "Fix a TODO in the codebase"), a status row `Build · <Model> <Provider>`, bottom edge `╹▀▀▀...`, then `tab agents  ctrl+p commands`; a tip line; footer with cwd and version. No prompt glyph. Cursor at the start of the text row. | `captures/01-*`, `18-*`. | CONFIRMED-observed |
| **Session screen:** composer docked at the bottom, full width: rows `┃` (blank) / `┃  <text>` / `┃` (blank) / `┃  Build · Model Provider` / `╹▀▀▀`, then a 1-2 row footer (cwd, token count, `ctrl+p commands`). **The empty composer here shows no placeholder**, just a blank text row. | `captures/09-*`. | CONFIRMED-observed |
| Typed draft renders as plain text (color 255) in the text row; multi-line drafts grow the box upward. | `captures/02-*`, `19-*`. | CONFIRMED-observed |
| **Enter submits.** Newline: `shift+return`, `ctrl+return`, `alt+return`, `ctrl+j` (`ctrl+j` observed). `ctrl+c` clears a non-empty composer and **exits the app when the composer is empty**; `ctrl+d` exits; `escape` interrupts a running turn; leader is `ctrl+x`. | `docs/keybinds.mdx`; observed `ctrl+j`, `ctrl+c` clear, `ctrl+c` exit. | CONFIRMED-docs + observed |
| **Bracketed paste** inserts text without submitting, newlines preserved; a longer paste (6 lines) collapses to a `[Pasted ~6 lines]` token. Text typed by `tmux send-keys -l` followed by `Enter` submits. | `captures/19-*`; observed. | CONFIRMED-observed |
| **Typing + Enter while busy queues** the message (`QUEUED` badge), see section 4. While busy the footer shows a progress bar and `esc interrupt`; the composer stays editable. | `captures/10-*`, `11-*`, `12-*`. | CONFIRMED-observed |
| **Modal dialogs are overlays above the composer**, with a title row ending in `esc`: command palette (`ctrl+p`, "Commands"), model picker ("Select model"), provider connect ("Connect a provider"), session list ("Sessions"), help ("Help", `esc/enter`). The composer stays partly or **fully visible underneath**; with Help open the composer row looks exactly like an empty composer. Keys go to the dialog. Signal: cursor moves into the dialog's search field, or is hidden (`cursor_flag=0`) for Help. | `captures/03-*`, `05-*` to `08-*`. | CONFIRMED-observed |
| `/` in the composer opens an autocomplete list **above** the composer (`/agents`, `/connect`, `/debug`, `/diff`, `/editor`, `/exit`, `/help`, `/init`, `/mcps`, `/models`, ...); the composer keeps focus. | `captures/04-*`. | CONFIRMED-observed |
| **Permission prompt replaces the composer**: `△ Permission required`, the command, and `Allow once   Allow always   Reject ... ⇆ select  enter confirm`; cursor hidden. | `captures/15-*`. | CONFIRMED-observed |
| **Question tool replaces the composer**: question text, numbered options, `Type your own answer`, `↑↓ select  enter submit  esc dismiss`; cursor hidden. The tool was offered to the model in the TUI but not under `opencode run`. | `captures/17-*`; mock tool lists. | CONFIRMED-observed |
| **No blocking first-run or provider-connect screen**: with no credentials the TUI opens straight to the composer with a free default model (`Big Pickle`, provider OpenCode) and a tip `Run /connect to add an AI provider`. No folder-trust prompt was seen. | `captures/18-*`. | CONFIRMED-observed |
| `--mini` is a different UI: inline (no alternate screen, no mouse), placeholder line without the box. | `captures/20-*`. | CONFIRMED-observed |
| Toasts render as a bordered box at the top-right of the transcript. | `captures/13-*`. | CONFIRMED-observed |

## 10. Risks and unavailable capabilities

1. **Experimental hook names.** `experimental.chat.system.transform`, `experimental.chat.messages.transform`, `experimental.session.compacting`, `experimental.compaction.autocontinue`, `experimental.text.complete` carry the prefix and may be renamed. The stable-named injection paths are `chat.message` (turn start) and `tool.execute.after` (post-tool); `session.prompt` with `noReply` is plain SDK. (CONFIRMED-source for names.)
2. **`permission.ask` hook is dead** in the observed version (declared, never triggered). (CONFIRMED-source + observed.)
3. **`session.idle` is deprecated** in source in favour of `session.status`. A `session.next.*` event family and "v2" plugin API (`packages/plugin/src/v2`, `OPENCODE_EXPERIMENTAL_EVENT_SYSTEM`) exist alongside the v1 surface, so the v1 surface may be superseded. (CONFIRMED-source; timeline UNCONFIRMED.)
4. **No session at start and no navigation events.** A server plugin cannot learn which session the TUI displays (home, `/new`, switch, `--continue`) until a hook fires for it; a wake target has to be the last session that had a turn in this process, or come from a TUI plugin (`api.route.current`, untested). (CONFIRMED-observed.)
5. **Plugin load failures are quiet.** A plugin whose init throws is logged only in `$XDG_DATA_HOME/opencode/log/opencode.log` (`failed to load plugin`), nothing in the TUI; install/entry failures publish a `session.error` event (visibility in the TUI UNCONFIRMED). A hook that throws inside `Plugin.trigger` is not caught there (`Effect.promise`), so it can fail the turn (CONFIRMED-source, effect not observed).
6. **Plugins are off** under `--pure` / `OPENCODE_PURE`, and in attach mode they run in the server process, where the pane's process is only a client. (CONFIRMED-observed.)
7. **Startup dependency install.** Each config dir gets `@opencode-ai/plugin` installed over the network; plugin loading waits for it when plugins are configured (`config.waitForDependencies()`). Offline behaviour measured later: [native-gate.md](native-gate.md), "Startup dependency install". A dependency-free plugin needs nothing from it. (CONFIRMED-source + observed writes.)
8. **`tui.appendPrompt` + `submitPrompt` clobber a user draft**; `promptAsync` does not. (CONFIRMED-observed.)
9. **`experimental.chat.system.transform` also fires for title and compaction calls** and cannot tell them apart; text injected there is sent on every model call of the turn. (CONFIRMED-observed.)
10. **Composer-typing wake hazards** (if a Native wake were used): overlays that leave an empty-looking composer visible (Help), `ctrl+c` exiting on an empty composer, the home-screen composer with no session, placeholder text that rotates, and the session-screen empty composer having no placeholder at all. (CONFIRMED-observed.)
11. **Queued prompt timing is model-loop dependent:** a prompt sent to a busy session is delivered at the next step, which may be the same turn or right after its final text; there is no "reject if busy" option on `prompt`/`promptAsync` (only `session.shell` returns `BusyError`). (CONFIRMED-source.)
12. **Auto-update:** `OPENCODE_DISABLE_AUTOUPDATE` / `"autoupdate": false`; the TUI otherwise checks for upgrades one second after start. (CONFIRMED-source.)
13. **Unauthenticated instances are usable** (free default model), so "no credentials" does not stop an isolated test instance from making real model calls. (CONFIRMED-observed that the model is selected; no call was made.)

## Open / unconfirmed points

- Behaviour with a real model (does it act on injected text, does it call `inbox` unprompted): not tested.
- `session.status` `retry` shape in practice; `session.error` shape for plugin failures.
- TUI plugin API (`api.route.current`, state) in practice; whether one module can export both `server` and `tui` (types say no: `tui?: never`).
- Kitty keyboard mode on a terminal that supports it (what Enter is sent as).
- `promptAsync` into a child session; background subagents; `task_id` resume.
- Offline startup with plugins configured; visibility of plugin install errors in the TUI.
- macOS/Windows differences (managed config, `detached` on Windows).
- Whether Herdr panes answer the kitty/capability queries opencode sends at startup.
