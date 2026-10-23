# Harness primer: Antigravity CLI (`agy`, Google)

Harness id `agy`. This primer is the shared basis for the alternative x harness reports. It covers
every surface of agy that can put text into the model's context or wake an idle session, and ends
with what herdr-threads does today.

Evidence tags: **VERIFIED** means read in official docs, in a captured agy artifact committed to this
repo, or in herdr-threads source. **INFERRED** means reasoned from those sources and not observed.
**unknown** means no source answers it. A third-party source is named as one, and where it
conflicts with another source the conflict is stated.

## 0. Provenance and local status

- **agy is not installed on this host.** VERIFIED (2026-10-23): `which agy` finds nothing on PATH.
  `~/.gemini/antigravity-cli/` holds only an empty `sandbox/` directory. No CLI help was observed
  live in this session. Nothing under `~/.gemini` was written.
- **Captured help.** The repo holds the `agy --help` output of 1.3.2:
  `tests/fixtures/agy/help/agy-1.3.2.txt:1-39`. That file and the other `tests/fixtures/agy/**`
  captures come from genuine agy 1.3.2 (official darwin-arm build), run model-free against a scripted
  fake Gemini server (`tests/fixtures/agy/composer/README.md:7-11`).
- **Versions covered.** The repo's probes and gates cover agy 1.3.2/1.3.3. The root design lists the
  surface "verified by probe" at
  `docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter-design.md:45-82`.
  The original research report it cites (`~/Documents/Agy_Integration_Research_20261008/`) is not
  on this host. A third-party issue reports 1.3.4 shipping by mid-October 2026
  ([session-peer #307](https://github.com/abruption/session-peer/issues/307)).
- **Latest gate run.** The native gate passed all 14 flows on genuine agy at herdr-threads `38c5fef6d2e3`:
  boot, setup, permission, allow-rewrite, enroll, subagent, clear, resume, delivery, downgrade,
  wake, launch, handoff and unsetup. VERIFIED: `docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/summary.txt`.
- **Coverage gaps.**
  - The required pre-release wake e2e covers only Claude and Codex (AGENTS.md "Pre-release wake e2e").
  - agy coverage is the opt-in native gate and a one-off live smoke (`docs/evidence/agy-live-smoke/`).
  - Both are macOS-only. The live smoke refuses non-darwin hosts (`docs/evidence/agy-live-smoke/README.md:14-16`).
  - Linux agy is unvalidated.

## 1. Surface summary

| Surface | Can inject into the model's context? | Can wake an idle session? | Status |
|---|---|---|---|
| `SessionStart` hook `injectSteps` | Yes, at the first model call of a new conversation and after `/clear` | No: it fires only when a prompt is submitted | VERIFIED |
| `PreInvocation` hook `injectSteps` | Yes, before every model call, including the calls that follow a tool result | No | VERIFIED |
| `PostInvocation` hook `injectSteps` + `terminationBehavior` | Yes, after a model call; `force_continue` keeps the loop going | Only extends a running turn | VERIFIED (docs); never exercised by herdr-threads |
| `Stop` hook `decision:"continue"` + `reason` | `reason` is injected as a system message, and the loop re-enters | Only stops the session going idle (block-and-continue); capped | VERIFIED (docs); cap value unknown |
| `PreToolUse` `overwrite` | Only a notice naming the changed argument keys, prepended to the tool result | No | VERIFIED (docs) |
| `PostToolUse` | No: its output is `{}` | No | VERIFIED (docs) |
| Plugin skills, rules, agents, GEMINI.md/AGENTS.md | Static context: skill names and paths at session start, rules loaded | No | VERIFIED |
| MCP tools | Only as tool results when the model calls a tool | No; server-pushed notifications unknown | VERIFIED / unknown |
| Keystrokes into the TUI (Herdr / tmux) | Yes, as a user turn | Yes | VERIFIED (herdr-threads uses it) |
| Remote Control (`--remote-control`, `/remote-control`) | Remote prompts "stream live into your local terminal TUI" | Probably (INFERRED); idle versus busy behaviour undocumented | VERIFIED (docs), partial |
| `agentapi send-message <conversation_id> <prompt>` | Sends a message to an existing conversation | Possibly; delivery to an idle TUI is unestablished | VERIFIED as documented for sidecars; CLI TUI behaviour unknown |
| `/schedule` / built-in `schedule` tool | The prompt fires as a "high-priority notification" | Docs: wakes "the existing agent with its full conversation history" | VERIFIED (docs); only the model can schedule it |
| Sidecars (`sidecar.json`, `builtin: schedule`) | Through `agentapi` | Each run starts a fresh conversation | VERIFIED (docs) |
| Headless `-p` with `--input-format stream-json` | Every stdin line is a user turn in one process | Yes, but only for a process we own (no TUI) | VERIFIED (docs) |
| Python SDK (`google.antigravity`) | `agent.chat()`; `pre_turn` hooks; `every(n, fn)` triggers | Yes, but only for agents the SDK hosts | VERIFIED (docs), partial |
| ACP (Agent Client Protocol) | None natively; community PTY/SDK wrappers only | — | VERIFIED: feature request open |

## 2. Hooks

### 2.1 Where hooks are configured

**Locations.** VERIFIED: [docs/hooks](https://antigravity.google/docs/hooks/).
- Workspace: `.agents/hooks.json` (behind folder trust, per the third-party
  [agenticcontrolplane](https://agenticcontrolplane.com/controls/antigravity) page).
- Global: `~/.gemini/config/hooks.json`, shared by the CLI, IDE and App.
- Plugin: a `hooks.json` inside the plugin directory.
- The docs list `~/.gemini/antigravity-cli/settings.json` as "not loaded" for hooks.
- `/hooks` inspects the loaded hooks in the CLI.

**Schema.** The top level maps a hook *name* (a namespace) to its config.
- Each named hook may set `enabled` (default `true`) and one or more event keys.
- `PreToolUse` and `PostToolUse` take `[{matcher, hooks:[handler…]}]`. The matcher is a regex over
  agy's own tool names (`run_command`, `view_file`, `browser_.*`); `""` or `"*"` matches all.
- `SessionStart`, `PreInvocation`, `PostInvocation` and `Stop` take a flat handler list.
- A handler is `{type:"command", command, timeout}`, with the timeout in seconds (default 30).
  VERIFIED: docs/hooks.

**Pitfalls.**
- Every top-level key is a namespace, and **one malformed key silently drops the whole file**.
  VERIFIED by the repo probe (root design `:47-49`).
- Unknown event keys are accepted and never fire. Third-party:
  [symposium.dev](https://symposium.dev/design/agent-details/antigravity-cli.html).
- A hook's cwd is the directory that contains its `hooks.json`. VERIFIED by probe (root design `:49`);
  symposium.dev agrees.
- Handlers run through `sh -c`. Third-party: symposium.dev.

### 2.2 Events and stdin payloads

The payload carries **no event-name field**: one command must be registered per event, with the
event in argv. VERIFIED: root design `:54`, `tests/fixtures/agy/payloads/README.md:3`, and
agenticcontrolplane. Field names are camelCase JSON on stdin.

**Common fields:** `conversationId`, `workspacePaths`, `transcriptPath`, `artifactDirectoryPath`,
`modelName`. VERIFIED: docs/hooks and the fixtures (`tests/fixtures/agy/payloads/session-start-top.json:1-9`).

| Event | Extra stdin fields | When it fires |
|---|---|---|
| `SessionStart` | none | At the **first prompt** of a new conversation, not at process launch. Also after `/clear` (alias `/new`), which makes a new id. Not on `-c` or `--conversation` resume. VERIFIED: root design `:64-67`; symposium.dev agrees. It was undocumented on 1.2.x/1.3.2 and appears in the current official docs. |
| `PreInvocation` | `invocationNum` (0-indexed, resets each user turn), `initialNumSteps` | Before **every** model call, including the calls after each tool result. VERIFIED: docs; `tests/fixtures/agy/payloads/pre-invocation-inv1.json:4-5`; genuine post-tool-call injection at `tests/fixtures/agy/delivery/README.md:11-14`. A resumed conversation's first call shows `initialNumSteps` > 1 (fixture `pre-invocation-inv0-resume.json:4-5`: 12). A third-party gist claims `invocationNum` 1 is the first call; that conflicts with the docs and the fixtures. |
| `PostInvocation` | same as PreInvocation | After a model call. VERIFIED: docs. |
| `PreToolUse` | `toolCall{name,args}`, `stepIdx` | Before a tool runs. VERIFIED: docs. |
| `PostToolUse` | `toolCall`, `stepIdx`, `error` | After a tool runs. It carries **no tool output**, and fires on non-tool steps with `toolCall: null` (third-party: agenticcontrolplane). |
| `Stop` | `executionNum`, `terminationReason`, `error`, `fullyIdle` | At the end of the turn. The TUI reports `terminationReason: "NO_TOOL_CALL"` (fixture `stop-no-tool-call.json:8`); the docs list `model_stop`, `max_steps_exceeded` and `error`. An injection abort shows `terminationReason:"ERROR"` and `error:"pre-invocation hook: failed to inject steps … unknown injected step type: <nil>"` (fixture `stop-inject-error.json:4-8`). `fullyIdle` is true only when "the agent is finished and all background commands and async tasks have completed" (docs). |

There is **no exit or session-end hook**. VERIFIED: root design `:67`.
There is **no compaction hook**; a PreCompact/PostCompact feature request is open:
[antigravity-cli #1317](https://github.com/google-antigravity/antigravity-cli/issues/1317).

### 2.3 Hook output: what reaches the model

**`injectSteps`** is accepted by `SessionStart`, `PreInvocation` and `PostInvocation`.
VERIFIED: docs and the repo probe. Each step holds exactly one of these:

| Step | Semantics | Evidence |
|---|---|---|
| `ephemeralMessage` (string) | Visible to the model for one request only, never persisted | VERIFIED: probe, root design `:58` |
| `userMessage` (string) | A permanent user turn. The transcript logs `source:"SYSTEM_SDK", type:"USER_INPUT"` with content wrapped as `<USER_REQUEST>…</USER_REQUEST>` plus an `<ADDITIONAL_METADATA>` local-time block | VERIFIED: genuine capture `tests/fixtures/agy/delivery/transcript-genuine-user-message.jsonl:1` |
| `systemMessage` (object `{"systemMessage":"…"}`) | Permanent. agy wraps it as "The following is a `<SYSTEM_MESSAGE>` not actually sent by the user … The following system message was injected by a PreInvocation hook:"; transcript `source:"SYSTEM_SDK", type:"SYSTEM_MESSAGE"`. **Not shown in the TUI.** It was undocumented on 1.3.2; the docs now list the key but not its inner shape | VERIFIED: `transcript-genuine-system-message.jsonl:1`, root design `:60` |
| `toolCall` (object) | **Broken on 1.3.2/1.3.3**: internal-only and deprecated; aborts the turn with `unknown injected step type: <nil>` | VERIFIED: probe, root design `:61`, `design-decisions.md:13` |

Failure behaviour of injected steps. VERIFIED: probe, root design `:62-63`.
- A well-formed but unsupported step **aborts the turn**.
- Malformed output is ignored silently, **together with any valid sibling steps**.
- `PreInvocation` fails open.

**Other outputs:**
- **`PreToolUse`**:
  - Its `decision` is required: `allow`, `auto_approve`, `deny`, `ask`, `force_ask` or `deny_unless_prior_grant`.
  - It may also return `reason`, `overwrite` (shallow-merged into the tool args; "a notice naming the
    changed keys is prepended to the tool result") and `permissionOverrides`. VERIFIED: docs.
  - It **fails closed**: `{}` or an empty object denies, and a crash, timeout or non-zero exit blocks the call.
    VERIFIED: the repo probe (root design `:63`: "`{}` denies, empty output allows") and agenticcontrolplane.
    The two disagree on whether *empty stdout* allows.
  - `overwrite` is a narrow mid-turn text path: it changes tool *arguments*, and the model sees only a key-name
    notice. A hook could rewrite `run_command` args to `echo` a message, but that is a hack (INFERRED).
- **`PostToolUse`**: `{}` only. No injection path. VERIFIED: docs.
- **`PostInvocation`**: `injectSteps` plus `terminationBehavior` (`force_continue`, `terminate` or
  default). VERIFIED: docs. herdr-threads has never exercised it, so its runtime behaviour is unverified.
- **`Stop`**: `decision:"continue"` re-enters the loop, and `reason` is "injected as a system message"
  only when continuing. Any other value lets the session stop. VERIFIED: docs. Consecutive continues
  are capped at a built-in limit of unknown value (agenticcontrolplane).
- **`PreInvocation` cannot block a model call**: a `deny` is ignored on 1.2.14
  ([antigravity-cli #1183](https://github.com/google-antigravity/antigravity-cli/issues/1183)).
- **Exit codes**: exit 1 and exit 2 behave like exit 0, and only stdout decides (symposium.dev).
  agenticcontrolplane says a non-zero exit is a failure, and PreToolUse fails closed on it. Both
  are third-party; the docs say nothing about exit codes.

### 2.4 Hook environment

- VERIFIED by the repo probe on 1.3.2 (root design `:78-80`): hooks inherit agy's environment
  (`HERDR_PANE_ID` included) plus `ANTIGRAVITY_CONVERSATION_ID`. The model's `run_command` shell
  gets `ANTIGRAVITY_AGENT=1`.
- **Conflict:** agenticcontrolplane (verified on 1.1.21) says "Only whitelisted variables reach the
  subshell a hook runs in." Either this changed between 1.1.21 and 1.3.2, or the whitelist covers what
  we use. herdr-threads relies on the inheritance (INFERRED from `docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter--agy-adapter-design.md:467`: "agy hooks inherit the pane environment").
  This is a fragility to watch.

### 2.5 Timing summary for injection

- Turn start: `SessionStart`, when the conversation is new, then `PreInvocation` with `invocationNum` 0.
- Mid-turn: `PreInvocation` before each model call that follows a tool result. This is a real
  mid-turn path. VERIFIED: genuine capture after `run_command`, `tests/fixtures/agy/delivery/README.md:11-14`.
  - A third-party issue claims "no mid-turn injection path"
    ([agents-can-communicate #171](https://github.com/automatis-tools/agents-can-communicate/issues/171)).
  - Our capture refutes that for between-tool-call model requests.
  - What really does not exist is injection *during* a running tool. VERIFIED: README `integrations/agy/README.md:12-13`.
- Turn end: `PostInvocation` and `Stop` can extend the turn and inject (`reason`, `injectSteps`).
- Idle: no hook fires.

## 3. Plugins, skills, rules, agents, sidecars

**Plugin layout.** VERIFIED: [docs/plugins](https://antigravity.google/docs/plugins/).
- A directory with a required `plugin.json`, plus optional `mcp_config.json`, `hooks.json`, `skills/`,
  `agents/` (custom subagents and personas) and `rules/` (markdown constraints).
- `plugin.json` allows `name` (CLI-required, `^[a-zA-Z0-9-_]+$`), `description` and `$schema`.
- **`additionalProperties: false`**: docs/plugins. Yet herdr-threads writes a `version` key
  (`tests/fixtures/agy/plugin/plugin.json:1`), and the gate passes on 1.3.2. A future strict
  validation could reject our manifest. INFERRED risk.

**Locations (conflicting).**
- docs/plugins: global `~/.gemini/config/plugins/` and workspace `.agents/plugins/` for App and IDE. The
  CLI "stages" plugins in `~/.gemini/antigravity-cli/plugins/<name>/`
  ([docs/cli/features](https://antigravity.google/docs/cli/features/)).
- herdr-threads writes `~/.gemini/config/plugins/herdr-threads/`, and the gate passes there. VERIFIED: gate summary.
- Enable state lives in `~/.gemini/config/config.json` `plugins[<name>].enabled`; absent means enabled.
  VERIFIED: probe, `integrations/agy/README.md:60-63`. The docs name no file.
- CLI management: `agy plugin install|uninstall|list|enable|disable` (help `:36-37`) and `/plugin`.

**Skills.**
- `skills/<name>/SKILL.md` with `name`/`description` frontmatter, from plugins, `~/.gemini/config/skills/`
  and `.agents/skills/`. Each is listed by name and path in the system prompt, and the model reads the body
  with `view_file`. VERIFIED: root design `:81`.
- `~/.gemini/config/skills.json` registers external paths. Third-party: symposium.dev.

**Custom instructions.** `GEMINI.md`, `AGENTS.md` and `.agents/rules/*.md` are loaded walking up to the
repository root. Third-party: symposium.dev. These are static, session-level text only.

**Sidecars.** VERIFIED: [docs/sidecars](https://antigravity.google/docs/sidecars).
- Background processes that Antigravity launches and restarts: `~/.gemini/config/sidecars/<id>/sidecar.json`
  or `~/.gemini/config/plugins/<plugin>/sidecars/<id>/`.
- Disabled until `"enabled": true` in `~/.gemini/config/config.json`.
- Fields: `command|builtin`, `args`, `restart_policy`, `env`.
- Sidecars talk to the agent only through **`agentapi`**:
  - `agentapi new-conversation <prompt>` (needs a `projectId`);
  - `agentapi send-message <conversation_id> <prompt>`.
- Output goes to `~/.gemini/antigravity/sidecar_data/<id>/`.
- The docs do not say whether sidecars run with the CLI or only with the App/IDE: unknown.
- The `agentapi` binary sits at `~/.gemini/antigravity-cli/bin/agentapi`, "a thin wrapper to
  `agy agentapi`". Third-party:
  [Hermes skill doc](https://hermes-agent.nousresearch.com/docs/user-guide/skills/optional/autonomous-ai-agents/autonomous-ai-agents-antigravity-cli).
  `agentapi` is not in the 1.3.2 `--help` subcommand list (help `:27-39`), so it is hidden or newer.

## 4. MCP

VERIFIED: [docs/mcp](https://antigravity.google/docs/mcp/).

**Configuration.**
- Files: global `~/.gemini/config/mcp_config.json`, workspace `.agents/mcp_config.json`, plugin `mcp_config.json`.
- `agy mcp add` writes the global file with no scope flag. Third-party: symposium.dev.
- Schema: `mcpServers{<id>:{command|serverUrl(url), args, env, cwd, headers, oauth, disabled,
  enabledTools, disabledTools, timeoutSeconds}}`. Defaults are 300 s to connect and 180 s per tool
  call; a negative value disables both.
- Permission rules use `mcp(server/tool)`.

**Transports.** stdio and Streamable HTTP. The CLI docs say legacy HTTP+SSE is "not supported"; the
SDK section of the same page lists SSE. The page contradicts itself.

**Features.**
- Tools: yes.
- Resources: mentioned only for the IDE.
- Prompts, `notifications/*`, `list_changed`, sampling, elicitation: **not documented**, so unknown.
  Whether the agy client surfaces a server-pushed notification to the model or the TUI is unknown.
  Nothing suggests that it does.

**Long-poll tools.** A blocking MCP tool would hold for up to the 180 s default (configurable,
negative disables). That is a long-poll option. INFERRED; untested on agy.

**Oddity.** OAuth tokens are stored in `~/.gemini/antigravity/mcp_oauth_tokens.json` (note: `antigravity`, not `antigravity-cli`).

## 5. Headless, SDK, server and remote modes

**Print mode.** `-p`/`--print`/`--prompt`. VERIFIED: help `:18-22` and [docs/cli/headless](https://antigravity.google/docs/cli/headless/).
- `--output-format text|json|stream-json`.
- stream-json emits `init`, `step_update` (`step_type` user_input/agent_response/tool/checkpoint,
  `tool_info`, `subagent_info`) and `result` events.
- **`--input-format stream-json`** reads one `{"event":"user","message":{"content":"…"}}` per stdin
  line and runs a turn for each, in one process. It exits when stdin closes.
  - `control_request`/`control_response` lines and slash commands are errors (exit 2).
  - This is a driveable long-lived headless agent, but not the user's TUI.
- `--print-timeout`: the docs give a default of 5m, and help `:20` says 0s. The two disagree.
- Headless loads no workspace `.agents/` config unless given an absolute `--add-dir` (third-party:
  symposium.dev), so plugins and hooks in the workspace may not apply. Global plugins presumably
  do (INFERRED).

**Resume.**
- `-c`/`--continue` resumes the most recent conversation. It is *not* an alias of `--conversation <id>`.
- `--conversation <id>` resumes a specific one.
- In the TUI: `/resume` (`/switch`, `/conversation`), `/fork` (`/branch`), `/rewind` (`/undo`).
- VERIFIED: help `:4-6`, [docs/cli/reference](https://antigravity.google/docs/cli/reference/).
- Whether in-TUI `/resume` or `/fork` fire `SessionStart` is **unknown**. CLI resume fires none (§2.2).

**Remote Control.** VERIFIED: [docs/remote-control](https://antigravity.google/docs/remote-control/).
- Started with `--remote-control`, `/remote-control [on|off]`, or the daemon `agy remote-control start|status|stop`.
- The CLI "establishes a reverse tunnel for the current session". "Prompts submitted from your
  remote device stream live into your local terminal TUI as they happen."
- The session is tied to the agy process. The dashboard is antigravity.google.com, and auth uses Google sign-in.
- There is no documented API, and idle-versus-busy handling is undocumented.
- Terminal sessions do not appear under the daemon instance
  ([antigravity-cli #1157](https://github.com/google-antigravity/antigravity-cli/issues/1157)).
- herdr-threads refuses `--remote-control` at launch (`integrations/agy/README.md:101-103`).

**Python SDK** (`google.antigravity`). VERIFIED, partial: [docs/sdk/overview](https://antigravity.google/docs/sdk/overview/),
[docs/sdk/lifecycle](https://antigravity.google/docs/sdk/lifecycle/).
- `Agent`/`LocalAgentConfig` with `agent.chat()`.
- Hooks `@hooks.pre_turn` ("intercept or modify messages") and `@hooks.on_tool_error`.
- Triggers `every(60, fn)`.
- `conversation_id` resume (at least 32 characters, alphanumeric and hyphens).
- A third-party guide claims a trigger can `ctx.send("…")` to inject a user-like message
  ([dev.to guide](https://dev.to/googleai/google-antigravity-sdk-the-developer-guide-4o8m)); not on the official page.
- Whether the SDK can attach to a running CLI conversation is unknown. Conversation ids from the CLI
  are UUIDs (36 characters), so length is not a barrier (INFERRED).

**ACP.** No native Agent Client Protocol mode.
- Feature request: [antigravity-cli #31](https://github.com/google-antigravity/antigravity-cli/issues/31).
- Community bridges wrap agy via PTY or the SDK, for example
  [shindgew/agy-acp](https://github.com/shindgew/agy-acp) and [jiridanek/agy-acp](https://github.com/jiridanek/agy-acp).

**Other subcommands** (help `:27-39`): `agent(s)`, `changelog`, `install`, `mcp`, `mic-serve`, `models`,
`plugin(s)`, `remote-control`, `update`. There is no `serve`, `app-server` or `rpc`.

## 6. Native scheduling and self-continuation (wake-adjacent)

**`/schedule <when> <task>`** and the model-callable **`schedule` tool**. VERIFIED: [docs/commands/schedule](https://antigravity.google/docs/commands/schedule).
- Two forms:
  - one-shot timers (`DurationSeconds`, `Prompt`, `TimerCondition`);
  - cron (`CronExpression`, `Prompt`, `MaxIterations`, `IsDaemon`).
- When it fires, "the agent receives your instruction as a high-priority notification", and schedules
  "Wake the existing agent with its full conversation history".
- Schedules are conversation-scoped and managed in `/tasks`.
- **This is a native idle wake that agy performs on itself.**
  - It is triggered only by the model's own tool call or the user's slash command; there is no external API.
  - Busy-time behaviour is undocumented.
  - A recurring "check herdr-threads" cron would be a poll loop, which the token-efficiency rules discourage (INFERRED).
- A third-party guide claims a 15-minute cap on one-shot timers (unverified).

**`/goal <objective>`** works continuously until the objective is met, with "Stop interception".
VERIFIED: [docs/slash-commands](https://antigravity.google/docs/slash-commands/). The mechanism is
undocumented beyond the guide link.

**`/btw <query>`** runs an out-of-band query in a background thread "without interrupting" the main
agent and without adding to its history. VERIFIED: docs/slash-commands.

**Stop `continue`** and **PostInvocation `force_continue`** (§2.3) are the hook-level
block-and-continue primitives. A Stop hook could long-poll for messages, bounded by its `timeout`, and
answer `continue` with the message as `reason`. INFERRED. The bound is the timeout (default 30 s, the
maximum is unknown) and the continue cap. That keeps a session spinning, not idle.

## 7. Session identity and lifecycle (where it affects delivery)

**Conversation id.** It is `conversationId` in every payload, and `ANTIGRAVITY_CONVERSATION_ID` in the hook env. VERIFIED.

**On-disk stores** under `~/.gemini/antigravity-cli/`. VERIFIED: root design `:68-72`.
- `brain/<id>/`, with the transcript at `brain/<id>/.system_generated/logs/transcript_full.jsonl`.
- `conversations/<id>.db`.
- `presence/<id>.lock`: created for top-level conversations (`/clear` included, about 2.4 s before its
  `SessionStart`), never for a child.
- `cache/last_conversations.json`.

**Subagents.**
- Hooks fire in children under their own `conversationId` with **no parent or role field**.
- The parent's `brain/<parent>/.system_generated/subagents/<child>.json` is written about 120 ms before
  the child's `SessionStart`. VERIFIED: root design `:69-72`.

**Transcript record schema.** `step_index`, `source`, `type`, `status`, `created_at`, `content`.
- Sources and types: `MODEL/PLANNER_RESPONSE`, `MODEL/GENERIC` (tool output), `USER_EXPLICIT/USER_INPUT`,
  and `SYSTEM_SDK` for hook-injected steps.
- VERIFIED: `tests/fixtures/agy/delivery/README.md:15-22`. This is how herdr-threads confirms delivery.

**Permissions.** VERIFIED: root design `:73-77`, [docs/cli/features](https://antigravity.google/docs/cli/features/).
- Only `~/.gemini/antigravity-cli/settings.json` `permissions.allow|ask|deny` is honoured.
- `command(herdr-threads)` matches whole leading words and refuses chains.
- agy rewrites that file itself and can drop unknown keys.
- A hook allow does not approve a command.
- Modes: `request-review`, `always-proceed`, `strict`; `--mode accept-edits|plan` (the docs also list `auto`).

## 8. TUI input behaviour (keystroke injection)

From genuine 1.3.2 captures under Herdr 0.9.1: VERIFIED, `tests/fixtures/agy/composer/README.md:42-61` and the
`*.read-detection.txt` files.

**Composer layout.**
- Two pane-width `─` rules enclose the composer. The first row starts `>`, and `>` alone means empty.
  Continuation rows are indented two spaces.
- The footer under the bottom rule shows the state:
  - idle and empty: `? for shortcuts …` (`empty.read-detection.txt:8-11`);
  - draft present: blank-led footer (`draft.read-detection.txt:9-11`);
  - turn running: `esc to cancel` (`busy.read-detection.txt:10-14`).
- **A draft is visible on screen**, so clobber avoidance is screen-parse based.

**Keys** (`help-overlay.read-detection.txt:10-35`).
- `enter` sends.
- Newline: `shift+enter`, `alt+enter`, `ctrl+j`, or `\`+`enter`.
- `ctrl+g` opens the prompt in `$EDITOR`.
- `ctrl+u` deletes back to the line start and, at a line start, joins the row to the one above, so
  repeated presses clear a multi-line draft.
- `ctrl+c` does **not** clear; it arms exit.
- `esc` clears an empty prompt or halts streams ([docs/cli/reference](https://antigravity.google/docs/cli/reference/)).
- `ctrl+y` yanks from the kill ring, so a draft killed with `ctrl+u` might be restorable. INFERRED; untested.
- `?` in an empty composer opens a help overlay, so a wake text starting `?` would misfire (INFERRED).
- `/` starts slash commands, and `Tab` confirms autofill.

**Bracketed paste.** Support is **unknown**. `ctrl+v` pastes "media or clipboard blocks". Herdr's
`pane send-text` typed `hello draft` correctly (composer README `:30`).

**Screens that are not the composer.**
- A permission prompt (`permission-prompt.read-detection.txt:14-26`) is a menu: `>` marks the
  selected option and `enter` confirms it. Typing into it could **approve a command**, so it must read
  as not-idle.
- Subagent status region: `● Agent(<name>)  Working · 1s`, with the footer gaining ` · N subagent(s)`
  (`subagent-generating.read-detection.txt:15-20`). **Herdr reports `idle` for this screen** (Herdr issue #2354).

**Seeded setting.** `altScreenMode: never` was seeded for the captures (composer README `:20-22`).
How the alternate-screen mode affects detection is unknown.

## 9. Idle and busy observability

- **Herdr.** It detects agy by process name (`agy`, `antigravity`, `antigravity-cli`) and parses its screen states.
  - Agent kind is `agy`, with `agent_status` in `herdr agent list` (`composer/agent-list.txt:1`).
  - Herdr also installs its own `herdr` block in `~/.gemini/config/hooks.json` that reports `conversationId`.
  - VERIFIED: root design `:82`, `integrations/agy/README.md:124-128`.
  - Known false idle: subagent work (above).
- **Hooks.** `Stop` with `fullyIdle:true` marks the true end of a turn, including background tasks. VERIFIED: docs.
  - Nothing marks "user started typing" or "turn started", other than `PreInvocation` `invocationNum` 0.
  - A hook-maintained busy/idle flag (set at PreInvocation, cleared at Stop when `fullyIdle`) is
    possible. INFERRED; not done today.
- **Statusline and window title.** A custom statusline or window title "can receive agent metadata as JSON,
  including working directory, model, token usage, and state". VERIFIED: [docs/cli/features](https://antigravity.google/docs/cli/features/).
  - This is a native, push-style state feed that is not screen-scraped.
  - Its schema and `state` values are unknown.
  - This is a candidate idle signal (INFERRED).
- **Headless.** The stream-json `step_update.state` and `result.status` (`RUNNING`, `WAITING`, …) apply to `-p` only.

## 10. What herdr-threads does today (brief)

All the code is in `src/harness/agy/`; the operator guide is `integrations/agy/README.md`.

**Install.**
- `setup agy` writes an owned plugin `~/.gemini/config/plugins/herdr-threads/`
  (`plugin.json`, `hooks.json`, `skills/herdr-threads/SKILL.md`).
- It registers `SessionStart` (10 s), `PreInvocation` (10 s) and `Stop` (5 s). No tool hooks, and no `PostInvocation`.
  (`src/harness/agy/plugin.rs:60-72`; golden `tests/fixtures/agy/plugin/hooks.json:1`.)
- It optionally adds one `command(herdr-threads)` allow member, with consent (`integrations/agy/README.md:25-46`).

**Role and lifecycle** (`src/harness/agy/witness.rs`).
- Top-level means a presence lock exists *and* no subagent listing names the conversation. Anything
  else, including any doubt, is treated as a child.
- Resume is detected at the first `PreInvocation` (`invocationNum==0`, `initialNumSteps>1`, conversation
  not known); a missed `/clear` is detected the same way.
- (agy-adapter design D5, `…--agy-adapter-design.md:248-302`.)

**Delivery** (`HookPull`, `src/harness/agy/delivery.rs`).
- At every top-level `PreInvocation`:
  1. It confirms earlier offers by scanning the transcript from the recorded offset.
     - Only `SYSTEM_SDK` steps count, with the header at column 0 (`delivery.rs:369-372`).
     - The scan is bounded to 8 MiB (`:54`).
  2. It sends `DeliveryPull` with the cached attention version.
  3. It emits one `{"injectSteps":[{"systemMessage":{"systemMessage":…}}]}` step, or nothing.
- At `Stop` it confirms the same way.
- **Downgrade.** If the step aborts the turn (`failed to inject steps` / `unknown injected step type`,
  `delivery.rs:65`) or misses on two turns, it switches to `userMessage` for that executable identity.
  It retries `systemMessage` 24 h later on probation (`:59`).
- Bounds: pages of at most 32 items or 64 KiB; bodies truncated at 8 KiB (AGENTS.md "Token-efficient communication").
- Byte-locked output: `tests/fixtures/agy/delivery/output-system-message.json:1`.

**Wake** (`IdleWake::Native`, `delivery.rs:69-81`).
- Herdr types the content-free `herdr-threads: wake up` (`src/notification/policy.rs:17`) into the
  empty composer. The woken turn's `PreInvocation` injects the bodies.
- `AgyComposer` (`src/harness/agy/composer.rs`):
  - it reads `Unreadable` (so no wake) for any non-empty, busy, permission, overlay or live-subagent screen;
  - `PokeCapabilities::NONE` (`:51-53`): no draft stash and no poke during a turn;
  - clear key `ctrl+u` (`:121-123`).

**Not used.**
- `PostInvocation`, `Stop` `continue`, `ephemeralMessage`, MCP, `agentapi`, Remote Control,
  `/schedule`, stream-json, the SDK and statusline metadata.
- The design explicitly parked `agentapi send-message` as a wake channel (root design `:357`).

## 11. Open questions and leads for the comparison reports (INFERRED unless noted)

1. **`agentapi send-message`** might deliver into the *existing* conversation, and wake it, without
   keystrokes or composer parsing.
   - Unknown: whether it reaches a TUI-owned conversation, what it does to a typed draft, and whether
     it needs a registered or ready session (session-peer #307 found an idle fresh TUI unregistered
     before its first input, and one send's outcome unknown).
   - This is the strongest lead against keystroke injection.
2. **Statusline JSON `state`** might be a native idle/busy signal that avoids Herdr's false idle during subagents.
3. **`ephemeralMessage`** could carry re-offers or the check-in digest without leaving permanent transcript text.
   - Our confirmation, however, depends on a persisted `SYSTEM_SDK` record, so ephemeral steps cannot be transcript-confirmed.
4. **`PostInvocation` `injectSteps`** might deliver items that arrive during a turn's last model call before `Stop`, without the next turn.
   - **`Stop` `continue`** could turn "arrived while busy" into same-turn handling instead of an idle wake.
   - This would be at odds with delivery-contract rule 4 only if misused for idle sessions.
5. **Fragilities.**
   - `systemMessage`'s inner shape is undocumented, mitigated by the downgrade.
   - `plugin.json` `additionalProperties:false` versus our `version` key.
   - Conflicting plugin root (`config/plugins` vs `antigravity-cli/plugins`).
   - Hook env sanitization reported on 1.1.21.
   - One malformed key drops a whole `hooks.json`.
   - Unknown `/resume` and `/fork` hook behaviour inside a running TUI.
   - Linux unvalidated.
6. **MCP push.** Notification surfacing is undocumented. A blocking MCP tool (180 s default) is the only MCP-side long-poll.

## Sources

**Official docs:**
- [Hooks](https://antigravity.google/docs/hooks/)
- [Plugins](https://antigravity.google/docs/plugins/)
- [CLI features](https://antigravity.google/docs/cli/features/)
- [CLI reference](https://antigravity.google/docs/cli/reference/)
- [Headless](https://antigravity.google/docs/cli/headless/)
- [MCP](https://antigravity.google/docs/mcp/)
- [Remote Control](https://antigravity.google/docs/remote-control/)
- [Slash commands](https://antigravity.google/docs/slash-commands/)
- [Schedule](https://antigravity.google/docs/commands/schedule)
- [Sidecars](https://antigravity.google/docs/sidecars)
- [SDK overview](https://antigravity.google/docs/sdk/overview/)
- [SDK lifecycle](https://antigravity.google/docs/sdk/lifecycle/)

**Issues:**
- [antigravity-cli #31](https://github.com/google-antigravity/antigravity-cli/issues/31) (ACP)
- [antigravity-cli #1157](https://github.com/google-antigravity/antigravity-cli/issues/1157) (Remote Control daemon)
- [antigravity-cli #1183](https://github.com/google-antigravity/antigravity-cli/issues/1183) (PreInvocation deny)
- [antigravity-cli #1317](https://github.com/google-antigravity/antigravity-cli/issues/1317) (compaction hooks)

**Third-party:**
- [symposium.dev agy notes](https://symposium.dev/design/agent-details/antigravity-cli.html)
- [agenticcontrolplane controls](https://agenticcontrolplane.com/controls/antigravity) and
  [hooks reference](https://agenticcontrolplane.com/blog/antigravity-hooks-reference)
- [session-peer #307](https://github.com/abruption/session-peer/issues/307)
- [agents-can-communicate #171](https://github.com/automatis-tools/agents-can-communicate/issues/171)
- [Hermes agy skill doc](https://hermes-agent.nousresearch.com/docs/user-guide/skills/optional/autonomous-ai-agents/autonomous-ai-agents-antigravity-cli)

**Repo evidence:**
- `tests/fixtures/agy/**`
- `docs/superpowers/runs/2026-10-09-agy-harness-adapter/`
- `docs/evidence/agy-native-gate/`, `docs/evidence/agy-live-smoke/`
- `src/harness/agy/`
- `integrations/agy/README.md`
