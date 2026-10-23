# ruflo x OpenAI Codex CLI: message injection and waking up

Upstream: https://github.com/ruvnet/ruflo. Clone read: shallow, HEAD `e00a82f2fa2fa2413dd89592f04aae5d4ca35872`. Paths are relative to the clone root. Harness reference: `../../primers/codex.md` (Codex 0.163.0). Context: `overview.md` in this directory.
Tags: **VERIFIED** means read in source or docs at the cited location. **INFERRED** means my conclusion from what was read. **unknown** means not found. Nothing from the clone was executed.

## TL;DR

- Ruflo supports Codex as a **tool and config host**: an MCP server, skills, an `AGENTS.md`, a `.codex/config.toml`, and Codex plugins with policy and telemetry hooks. It does **not** support Codex as a **messaging peer**. No Ruflo code path puts a message from another agent into a live Codex session's context. Codex reads its mail only when the model itself calls the `team_inbox` MCP tool, or runs the bus CLI or `ruflo memory search` from a shell. VERIFIED (sections 2 and 3).
- **Waking: none.** Ruflo never wakes an idle interactive Codex TUI. Every Codex "turn" Ruflo starts is a new headless process: `codex exec` (dual-mode workers, `ruflo-codex loop`, console `@codex`) or `codex exec resume <thread>` (the console terminal). It uses no app-server, no `codex queue`, no `turn/steer`, no keystrokes, and no Stop-hook continuation. VERIFIED by reading the code and by grep (section 3).
- Ruflo's Codex hooks never inject model context. `ruflo-core` hooks print nothing on Codex. `ruflo-adr`, `ruflo-ddd` and `ruflo-sparc` only emit `permissionDecision: "deny"`. `ruflo-swarm` is an explicit no-op. The only proven context injection on Codex (`UserPromptSubmit` `additionalContext`, 8 of 9 live runs) belongs to an adapter that is **Proposed, not built** (ADR-491). VERIFIED.

---

## 1. Support status

| Surface | State | Evidence |
|---|---|---|
| `ruflo init --codex` / `--dual` | Built. The CLI loads the optional `@claude-flow/codex` package, or runs `npx @claude-flow/codex@latest`. | VERIFIED `v3/@claude-flow/cli/src/commands/init.ts:61-125` |
| MCP server registered with Codex | Built: `codex mcp add ruflo -- npx -y ruflo@latest mcp start` | VERIFIED `v3/@claude-flow/codex/src/mcp-config.ts:7-9,51-74,106-109`; `v3/@claude-flow/codex/src/initializer.ts:411-482` |
| Codex plugin `ruflo-core@ruflo` (lifecycle hooks) | Built, but the hooks are telemetry only, with empty stdout to Codex. They stay inactive until a person trusts them in `/hooks`. | VERIFIED `initializer.ts:484-565`; `plugins/ruflo-core/hooks/hooks.json`; `plugins/ruflo-core/scripts/ruflo-hook.cjs:501-522,630-638` |
| Codex plugins `ruflo-adr`, `ruflo-ddd`, `ruflo-sparc` | Built: `SessionStart` (status file only) and `PreToolUse` (a secret/scope guard that can deny). Not installed by `init`, which adds only `ruflo-core@ruflo`; a person adds these by hand | VERIFIED `plugins/ruflo-adr/hooks/codex-hooks.json`; `plugins/ruflo-adr/.codex-plugin/plugin.json`; `scripts/lib/codex-mod-runtime.mjs:90-107`; `initializer.ts:493-561` |
| Codex plugin `ruflo-swarm` | Built as a deliberate no-op (`"hooks": {}`) | VERIFIED `plugins/ruflo-swarm/hooks/codex-hooks.json`; `tests/plugins/codex-swarm-hook-manifest.test.mjs:7-14` |
| `ruflo-mods` Codex adapter (routing context through `UserPromptSubmit`, tool checks) | **Proposed, not built** | VERIFIED `v3/docs/adr/ADR-491-ruflo-mods-host-integration-claude-and-codex.md:3,52-71` |
| ADR-402 team bus (`team_send` / `team_inbox`) | Usable from Codex as MCP tools (pull). The spawn plan has no Codex host adapter, only `grok` and `claude`. | VERIFIED `v3/@claude-flow/cli/src/mcp-tools/team-tools.ts:133-183`; `v3/@claude-flow/cli/templates/grok/scripts/grok-team-bus.mjs:83-130` |
| Headless Codex workers | Built: dual-mode orchestrator, `ruflo-codex loop`, console `@codex`, console terminal | VERIFIED (section 3) |
| Waking an idle interactive Codex peer | **Not supported** | VERIFIED by absence (section 3.1) |

Overall: **partial support.** Codex works as a worker and a tool host. Codex does not work as a long-lived peer that receives pushed messages. INFERRED from the table.

---

## 2. Injection mechanism

Four mechanisms can carry another agent's text into a Codex model. Only the two pull paths (2.1 and 2.2) reach a live interactive session, and both work only when the model asks for its mail.

### 2.1 Pull through an MCP tool: `team_inbox` (the main path into a live Codex TUI)

- **What it is.** The Ruflo MCP server (`[mcp_servers.ruflo]`, stdio, `npx -y ruflo@latest mcp start`) exposes `team_create`, `team_spawn`, `team_send`, `team_inbox`, `team_broadcast`, `team_plan`, `team_status`, `team_on_stop` and `team_shutdown` (`team-tools.ts:73-261`). The description of `team_inbox` frames it as "the Grok/Codex-side counterpart to team_send for hosts without a live message channel" (`team-tools.ts:168-183`). VERIFIED.
- **Code path.** The MCP handler calls `run('readInbox', …)` (`team-tools.ts:181-182`), which dynamically imports `templates/grok/scripts/grok-team-bus.mjs` (`team-tools.ts:36-45`). `readInbox` (`grok-team-bus.mjs:282-306`) lists `mailbox/<agent>/*.json`, sorted lexically (`:272-275`). Unless `peek` is set, it claims each file by `renameSync` into `archive/` and returns every message. VERIFIED. Excerpt (condensed from `:290-299`):

  ```js
  for (const f of files) {
    let file = path.join(dir, f);
    if (!peek) {
      const archived = path.join(dir, 'archive', f);
      try { fs.renameSync(file, archived); }
      catch (err) { if (err.code === 'ENOENT') continue; throw err; } // another reader claimed it
  ```
- **Store.** `.claude-flow/teams/<team>/mailbox/<agent>/NNN_<msgid>.json`. NNN is the priority, zero-padded (0-999, default 2, lower is read first): `grok-team-bus.mjs:38-39,229`. Each file is written with `flag: 'wx'`, mode 0600, under a team lock (`:240-268`). VERIFIED.
- **Payload format.** The message object is `{id: "msg_<ms>_<rand8>", teamId, from, to, summary, content, type, priority, timestamp}` (`grok-team-bus.mjs:247-257`). The MCP server returns the whole result as `JSON.stringify(result, null, 2)` in one text content block (`v3/@claude-flow/cli/src/mcp-server.ts:985`). The model therefore sees pretty-printed JSON, roughly `{success, action:"inbox", team, agent, messages:[…], peek}`. VERIFIED. No per-message header line and no trust framing: unknown beyond the tool description. INFERRED: none exists.
- **When it fires.** Only when the Codex model chooses to call the tool. Something has to tell it to: the spawn-plan "comms protocol" (`Read your inbox first: team_inbox (team=…, agent=…)`, `grok-team-bus.mjs:90-105`) or the lead's own prompt. That protocol text is hard-coded for Grok ("Grok Build 1.0.41 runs you as a general-purpose subagent", `:92`), even when the team's `host` is `codex`. VERIFIED. No hook, timer or notification makes it fire. A Codex session that is generating learns of new mail only if it polls. VERIFIED by absence: no Codex hook in the clone calls the bus.
- **Size bounds.** `team_send` accepts `message` up to 500,000 characters and `summary` up to 1,000 (`team-tools.ts:150-154`). `readInbox` has **no page bound**: it drains and returns every pending message in one result. VERIFIED. On the Codex side, the MCP tool output is subject to Codex's per-tool `output_token_limit` and generic tool-output truncation (primer §4). INFERRED: a large backlog is truncated by Codex **after** Ruflo has already archived it.
- **Dedupe and ack.** Claiming by rename makes concurrent drains disjoint (VERIFIED, `:277-281`). The drain itself is the acknowledgement. Once the file moves to `archive/`, the message is consumed whether or not the model's context ever received it: for example, the MCP call times out (`tool_timeout_sec`), Codex truncates the output, or the turn is interrupted. No re-offer and no receipt. INFERRED from the code. `peek: true` reads without consuming (`:286`).

### 2.2 Shell pull: the bus CLI and `ruflo memory`

- The spawn protocol names a fallback "Without the Ruflo MCP server": `node scripts/grok-team-bus.mjs inbox --team <t> --agent <a>` and `… send …` (`grok-team-bus.mjs:96-98`). Same store, same semantics. VERIFIED.
- Dual-mode workers coordinate through shared memory, not mail: `npx ruflo@latest memory search --query … --namespace <ns>` before work and `memory store --key <id>-result` after (`v3/@claude-flow/codex/src/dual-mode/orchestrator.ts:291-308`). VERIFIED. This is a blackboard, not addressed messaging. It has no ordering, no ack, and no notion of "new since last read" (INFERRED).

### 2.3 Prompt at spawn (headless workers)

The main way text "from another agent" reaches Codex in Ruflo is the **initial prompt of a fresh `codex exec` process**: VERIFIED.

- Dual-mode: `codex exec --sandbox <read-only|workspace-write> --skip-git-repo-check [-m M] <enhancedPrompt>`, with the prompt as the last positional argument and stdin closed (`orchestrator.ts:205-252`). Fix #2947: an open stdin made `codex exec` block in `resolve_root_prompt` (`:246-252`). Output is capped at `maxOutputBytes` (1 MiB default, `:99-118,256-262`). The run times out at 5 min by default (`:114,264-267`).
- Loop: `codex exec --sandbox workspace-write --skip-git-repo-check <buildCodexLoopPrompt(state)>` once per iteration (`v3/@claude-flow/codex/src/loop/index.ts:218-224`). The prompt carries the loop name, the iteration number, the task, and an instruction to create a marker file when done (`:99-114`). Stored output is truncated to 8,000 characters (`:172-176,304-306`).
- Console `@codex` (person to Codex): `codex exec -s read-only --skip-git-repo-check --ephemeral -C <cwd> -`, with the prompt on stdin (`v3/docs/adr/ADR-465-interactive-control-plane-and-multi-model-bridge.md:62`). VERIFIED (ADR text).
- Agent docs for an interactive Codex coordinator tell it to fan out `codex exec … "<prompt>" &` from its own shell and then `wait` (`.claude/agents/dual-mode/codex-coordinator.md`, lines 8-12 and "Step 2"). VERIFIED.

Framing is plain prose (`You are a CODER agent in a collaborative dual-mode swarm…`, `orchestrator.ts:294-307`). Dedupe and ack do not apply: one process per message.

### 2.4 Resume of a stored thread (person-driven)

The console's terminal view keeps one Codex thread per project. Its first turn is `codex exec --json --sandbox read-only --skip-git-repo-check -`. Every later turn is `codex exec resume --json --skip-git-repo-check -c sandbox_mode="read-only" <thread_id> -`, with the text on stdin (`plugins/ruflo-console/hooks/harness.ts:48-56`). The `thread_id` comes from the `thread.started` JSONL event (`plugins/ruflo-console/hooks/stream.ts:49-58`, `harness.ts:217-223`). VERIFIED. A person triggers each turn. Only starting or resuming a session goes through a confirm card ("Enter shows the command, Enter again runs it"); after that "the session is live for this Claude Code session and Enter sends" (`harness.ts:7-9,130-137`). This is the closest Ruflo comes to "deliver to an existing Codex conversation", and it creates a second process on the rollout, never one that reaches a live TUI (primer §6).

### 2.5 Hooks: what Ruflo's Codex hooks actually output

| Plugin | Events (Codex) | stdout to Codex | Model context? | Evidence |
|---|---|---|---|---|
| ruflo-core | PreToolUse `Bash`, `Write\|Edit\|MultiEdit`; PostToolUse same; `PostToolUseFailure`; PreCompact `manual`/`auto`; Stop | PreToolUse: **empty** on Codex (detected by `PLUGIN_ROOT`/`PLUGIN_DATA` env or a `turn_id` in the payload). Post/Stop: CLI output swallowed. PreCompact: plain-text "guidance" lines | No (PreCompact plain text: unknown, see §6) | VERIFIED `plugins/ruflo-core/hooks/hooks.json:4-93`; `ruflo-hook.cjs:31-33,519-522,528-555,630-644` |
| ruflo-adr / ddd / sparc | SessionStart, PreToolUse (no matcher) | SessionStart: nothing ("No extra instructions or model turn"). PreToolUse: `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":…}}` only on a deny | Only the deny reason | VERIFIED `scripts/lib/codex-mod-runtime.mjs:90-107` |
| ruflo-swarm | none | n/a | No | VERIFIED `plugins/ruflo-swarm/hooks/codex-hooks.json` |
| ruflo-mods (proposed) | UserPromptSubmit `additionalContext`, PreToolUse deny | route plus memory | Would be yes; **not built** | VERIFIED `ADR-491…md:32-35,63-67` |

No Ruflo Codex hook reads the team bus, the swarm mailbox or federation inbound. VERIFIED by grep: `team_inbox` and `readInbox` appear only in `team-tools.ts`, the bus, its test, ADR-402 and the Grok templates' docs, skill and rules.

---

## 3. Wake mechanism

### 3.1 Idle interactive Codex session: none

- No keystroke injection into an agent pane. `send-keys` appears only for console-owned shell sandboxes (`v3/docs/adr/ADR-442-console-sandbox-page-and-plugins-in-tools.md:15`; overview §4.8). VERIFIED.
- No Codex-native wake. A grep for `app-server`, `codex queue`, `thread/queue`, `turn/steer` and `exec resume` over the clone (excluding `node_modules`) found:
  - `app-server` only in two tests and one analysis ADR. `tests/plugins/codex-mod-hooks.test.mjs:116-141` spawns `codex app-server --listen stdio://` only to call `hooks/list` and check that six hooks load.
  - `exec resume` only in the console (§2.4) and its ADR (`v3/docs/adr/ADR-407-ruflo-console-cockpit.md:81`).
  - Nothing for `queue` or `steer`.

  VERIFIED.
- No Stop-hook `decision:"block"` continuation. The ruflo-core Stop hook runs `session-end` telemetry and prints nothing (`hooks.json:84-93`, `ruflo-hook.cjs:640-644`). VERIFIED.
- No long-poll tool. `team_inbox` returns at once, and nothing blocks waiting for mail. VERIFIED (`grok-team-bus.mjs:282-306`).
- The session browser (ADR-486) lists Codex rollouts read-only. Every adapter declares `messaging: no, resume: no`, with the rule "browsing must never start or wake one" (`v3/docs/adr/ADR-486-session-workspace.md:32-50`, Codex row `:39`). VERIFIED.

### 3.2 What Ruflo does instead: replace "wake" with "spawn a fresh process"

| Path | Trigger | Idle/busy detection | Draft safety | Latency | Failure modes |
|---|---|---|---|---|---|
| Dual-mode worker (`orchestrator.ts:157-286`) | Orchestrator, after `dependsOn` completes. It polls its own in-memory worker map every 500 ms (`:313-330`) | Not needed: a new process | n/a (no TUI) | Codex cold start plus one full turn. The ordering wait has up to 500 ms of granularity (VERIFIED) | Timeout SIGTERM at 5 min with no partial result kept as success (`:264-267`). A non-zero exit rejects with stderr (`:273-277`). In `workspace-write`, the sandbox blocks network by default, so the worker's `npx ruflo@latest memory …` protocol needs a cached package (INFERRED from Codex's sandbox defaults, not covered by the primer; untested). The worker env drops every variable ending in `KEY`, `API_KEY`, `TOKEN`, `SECRET`, `PASSWORD` or `CREDENTIAL(S)` (`orchestrator.ts:603-613`), so a Codex worker that authenticates through `OPENAI_API_KEY` in the environment loses it (VERIFIED filter; impact INFERRED) |
| `ruflo-codex loop` (`loop/index.ts:116-216`) | Timer: sleeps `intervalSeconds` (default 270 s) between iterations, up to `maxIterations` (default 10) | Sequential and self-owned | n/a | ≥ 270 s per iteration by default | No memory across iterations except files, because each iteration is a fresh `codex exec` without `resume` (`:218-224`). A stop or completion is noticed only between iterations (`:286-291`). Each iteration is capped at 30 min by default (`:122`) and reported as exit code 124 on timeout (`:273-275`). Unless the completion marker exists, any non-zero exit, a timeout included, ends the loop as `failed` (`:190-196`). The sandbox defaults to `workspace-write` but is an option (`:219`) |
| Console `@codex` / terminal | A person presses Enter on a confirm card | n/a | n/a | One headless turn | 10-min run cap (`harness.ts:20-21`). Read-only sandbox |
| Team plan advance (`team_on_stop`) | Grok's `SubagentStop` hook. No Codex wiring exists | n/a | n/a | n/a | Returns only a *hint* ("Spawn or resume agent X") that a lead model must act on (overview §5) |

INFERRED design property: Ruflo avoids every hard part of our problem (idle detection, busy versus idle, the human's draft, the Enter paste-burst window) because it never addresses a live Codex composer.

---

## 4. Setup and installation footprint on Codex

What `ruflo init --codex` (or `--dual`) writes or runs, in order (`v3/@claude-flow/codex/src/initializer.ts`):

| Target | Content | Scope | Evidence |
|---|---|---|---|
| `AGENTS.md` (project root) | Generated instructions. Skipped if the file exists | project | VERIFIED `:112-121`; generator `src/generators/agents-md.ts` (758 lines) |
| `.agents/config.toml` | A large Ruflo config with `[mcp_servers.*]`, `[[skills.config]]`, `[swarm]`, `[hooks] enabled/pre_task/post_task/train_on_edit`, `[workers.*]`. Codex does not read `.agents/config.toml` (INFERRED), so the `[hooks]` keys are Ruflo-internal. In a file Codex does load, they would collide with Codex's own inline `[hooks]` table (INFERRED, primer §1.1) | project | VERIFIED `:123-131`; `src/generators/config-toml.ts:411-428` |
| `.agents/skills/<skill>/SKILL.md` (+ scripts, references) | Bundled skills | project | VERIFIED `:133-143,352,652-702` |
| `.codex/AGENTS.override.md` | A local override template | project, gitignored | VERIFIED `:165-168` |
| `.codex/config.toml` | Written only if absent, and never rewritten, even with `--force`. Standard is `approval_policy="on-request"`, `sandbox_mode="workspace-write"`, `web_search="cached"`. `--codex-trust full` writes `never`, `danger-full-access` and `live` | project, gitignored | VERIFIED `:171-195,592-647`; history in `v3/docs/adr/ADR-490-three-host-e2e-integration-loop.md:127` (before #4070 the default was full access) |
| `codex mcp add ruflo -- npx -y ruflo@latest mcp start` | Registers the MCP server. Idempotent through `codex mcp list --json`, with a text fallback | user `$CODEX_HOME/config.toml` (INFERRED: that is where `codex mcp add` writes) | VERIFIED `:411-482`; `mcp-config.ts:106-109` |
| `codex plugin marketplace add ruvnet/ruflo --ref main` + `codex plugin add ruflo-core@ruflo` | A git clone of the whole repo (17.5 s measured; 180 s budget). Codex reads the Claude-format `.claude-plugin/marketplace.json` | user `$CODEX_HOME` | VERIFIED `initializer.ts:538-561`; `ADR-490…md:59,126` |
| Hook trust | **Not written.** Init prints "ACTION REQUIRED … open /hooks … trust them. Hooks are installed but remain INACTIVE until you complete this review." | n/a | VERIFIED `initializer.ts:484-500` |
| Runtime files | `~/.cache/ruflo-codex/<plugin>/sessions/<sha256(cwd\0session_id)>.json` plus a courtesy status in the project (adr/ddd/sparc). Temp slot and stdin files under `$TMPDIR/ruflo-hook-*` (core) | user cache, tmp | VERIFIED `codex-mod-runtime.mjs:37-65`; `ruflo-hook.cjs:99-166` |
| Team bus | `.claude-flow/teams/<team>/…` when `team_*` is used | project | VERIFIED `grok-team-bus.mjs` |

Note: `ruflo-core` also ships `.mcp.json`, which declares the same `ruflo` server through `${CLAUDE_PLUGIN_ROOT}/scripts/mcp-launch.cjs` (`plugins/ruflo-core/.mcp.json`). Codex does load an installed plugin's MCP declarations, with `.mcp.json` as the default file name (`codex-rs/ext/mcp/src/plugin/provider.rs:15,52-60`, contributed per thread in `codex-rs/ext/mcp/src/plugin.rs:122-160`; https://github.com/openai/codex at `8308618`, VERIFIED). So a Codex install that has both the plugin and the `codex mcp add` entry declares `ruflo` twice; how Codex resolves the same server name from two contributors was not traced (unknown).

---

## 5. Token cost

**Per message (pull through `team_inbox`).**
- One model tool call, plus the MCP result. VERIFIED shape: pretty-printed JSON (`mcp-server.ts:985`) with a wrapper of about 90 bytes (`success`, `action`, `team`, `agent`, `peek`) and about 250 bytes of fields per message (`id`, `teamId`, `from`, `to`, `summary`, `type`, `priority`, `timestamp`, 2-space indent and newlines), plus the body. INFERRED estimate: about 300-350 bytes of overhead per message, which is about 6-8x the 4-byte `[pN]` marker plus the one-line header of herdr-threads.
- The body is not truncated by Ruflo (up to 500,000 characters). It is bounded only by Codex's tool-output limits (INFERRED).
- An empty poll still costs a full tool round trip (call plus `{"success":true,…,"messages":[]}` result) every time the model checks. The model must check, because nothing pushes.

**Per quiet turn.**
- Hooks: ruflo-core and adr/ddd/sparc inject **0 bytes** of model context on an allowed tool call or a session start (VERIFIED, §2.5). The cost is wall-clock instead: one Node spawn per matching tool call (23-43 ms for the classic handler measured on PreToolUse(Bash), `ADR-491…md:49-50`). ruflo-core also starts a detached `ruflo` CLI per event, capped at 4 concurrent, with extra events dropped except `session-end` (Stop), which is never dropped (`ruflo-hook.cjs:83-97,108-140,407-409`).
- Static context: the Ruflo MCP server advertises a very large catalogue ("300+ tools", `plugins/ruflo-core/.claude-plugin/plugin.json` description; README claims 314). If Codex sends every MCP tool schema on every request, this is the dominant per-turn cost, and it is independent of messaging. Whether Codex 0.163 defers or searches MCP tools instead of inlining them: unknown, not checked here. Ruflo writes no `enabled_tools` or `disabled_tools` (VERIFIED by grep). The generated `AGENTS.md` (from a 758-line generator) is also read at session start (INFERRED: several KB).
- Headless paths re-pay the full system prompt, `AGENTS.md`, tools and skills per spawned process. The loop re-pays them every 270 s, and dual-mode once per worker (INFERRED).

---

## 6. Limitations and bugs visible in the source

1. **No push into a live Codex session.** Messaging to an interactive Codex depends on the model remembering to call `team_inbox`. A Codex that is idle at the prompt never sees mail until a human types. VERIFIED (§2-3).
2. **Drain counts as ack, so messages can be lost.** The rename into `archive/` happens before the MCP result reaches Codex. Any later loss is silent: a tool timeout, output truncation, an interrupted turn, or a model that ignores a long result (`grok-team-bus.mjs:290-304`). INFERRED.
3. **Unbounded drain.** There is no page size, so one call can return hundreds of KB. Codex then truncates it, which is #2 again. VERIFIED (no bound) plus INFERRED (truncation).
4. **The spawn plan is Grok-only.** For a team with `host: "codex"`, `team_spawn` still emits "Grok Build 1.0.41 runs you as a general-purpose subagent… do not call spawn_subagent", plus only `grok` and `claude` host blocks (`grok-team-bus.mjs:90-130`). A Codex lead or child gets wrong instructions. VERIFIED.
5. **The ruflo-core hook matchers do not fit Codex tools.** `Write|Edit|MultiEdit` never matches Codex's `apply_patch` (INFERRED from primer §11.4 and ADR-491 noting `apply_patch` as the edit tool). `PostToolUseFailure` is not a Codex event, since it is absent from the primer's schema enum (VERIFIED in the primer §1.2) and from Ruflo's own list of event names read from the 0.162.0 binary (`ADR-491…md:21-22`), so that group is dead on Codex. Whether Codex warns about the unknown event: unknown.
6. **PreCompact prints plain text to Codex.** `precompactManual` / `precompactAuto` write emoji guidance lines to stdout (`ruflo-hook.cjs:528-555`), with no Codex branch. Codex's PreCompact output is a JSON contract (`continue:false`, primer §1.4). Whether non-JSON stdout is ignored, logged as invalid, or reaches anything: unknown. This is the same class as #2816/#2856, which hit PreToolUse (`ruflo-hook.cjs:508-517`).
7. **Codex detection is a heuristic.** `isCodexPluginHost` checks `PLUGIN_ROOT`/`PLUGIN_DATA` in the environment, or a `turn_id` field in the payload (`ruflo-hook.cjs:519-522`). It is fine for its purpose, but it keys host behaviour on payload shape. VERIFIED.
8. **Version pinning and a crash note.** ADR-490/491 pin Codex 0.162.0 because 0.162.1 crashes, and propose a doctor warning for one exact version (`ADR-491…md:79,103`; `ADR-490…md:55`). This is the opposite of our "never gate on an exact version" rule. It is diagnostic, not a gate, as written. VERIFIED.
9. **The headless loop has no conversation continuity.** Each iteration is a fresh `codex exec` and never `exec resume` (`loop/index.ts:218-224`), so state carries over only through files and the marker. VERIFIED.
10. **Network inside dual-mode workers.** Workers run `--sandbox workspace-write`, and their protocol is `npx ruflo@latest memory …`. Codex's workspace-write blocks network by default, so this depends on an npx cache hit (INFERRED; untested).
11. **The MCP `resources` capability was advertised but not served.** Codex offered `list_mcp_resources`, the model called it first, got `-32601`, and gave up before reaching the tools. Ruflo fixed this by answering with empty lists (`ADR-490…md:125`; `mcp-server.ts:999-1007`). VERIFIED. This is a real Codex trap for any MCP integration.
12. **Double MCP registration** through `codex mcp add` plus the plugin's `.mcp.json`, which Codex does load (§4, VERIFIED). The name-clash outcome is unknown.

---

## 7. Clever ideas worth stealing (for herdr-threads)

1. **Verify hook loading through the app-server without a model turn.** `tests/plugins/codex-mod-hooks.test.mjs:110-141` does the following:
   - starts `codex app-server --listen stdio://` under a private `CODEX_HOME`;
   - sends `initialize` (with `experimentalApi`), then `hooks/list {cwds:[root]}`;
   - asserts `warnings == []`, `errors == []`, the hook count, `handlerType`, `async:false`, `enabled:true` and `sourcePath`.

   This is a cheap, keyless doctor or test for our `setup codex`. It would catch manifest-schema rejections (Ruflo hit `unknown field 'modules'`, #3688) and could report trust status (`HooksListEntry` carries `HookTrustStatus`, primer §1.1). VERIFIED in Ruflo. Adoption INFERRED.
2. **Report "installed but untrusted" honestly.** Init prints an explicit ACTION REQUIRED line, and the proposed doctor shows untrusted hooks as `!`, never `✓` (`initializer.ts:484-500`; `ADR-491…md:80-81`). Our setup also never writes trust, so a `doctor` row backed by `hooks/list` trust status would close the loop. INFERRED.
3. **Do not advertise MCP capabilities you do not serve** (finding #11). This only matters if herdr-threads ever ships an MCP surface for Codex, but the failure mode costs a whole session. VERIFIED in Ruflo.
4. **Empty stdout is the safe default for Codex hooks.** Codex rejects unknown top-level keys ("hook returned invalid pre-tool-use JSON output", `ruflo-hook.cjs:508-517`), so the "allow" verdict is exit 0 with no output. We already emit only `hookSpecificOutput.{hookEventName, additionalContext}`, which agrees. VERIFIED.
5. **Close stdin for `codex exec`.** An open stdin pipe hangs `codex exec` in `resolve_root_prompt` (`orchestrator.ts:246-252`, #2947). Relevant to any headless check or relaunch we do. VERIFIED in Ruflo.
6. **Claim-by-rename for concurrent readers** gives disjoint drains with no lock (`grok-team-bus.mjs:277-304`). Our SQLite store already gives stronger guarantees, so this is evidence only, not something to adopt. Take the negative lesson too: never treat "handed to the tool" as "seen by the model". Our ACK-on-display through `inbox` has the same shape, but the display happens inside our own bounded output, so truncation is ours to control. INFERRED.
7. **A reference model for inbox semantics** (`v3/@claude-flow/codex/src/harness/in-memory-inbox-reference.ts:41-174`). Its properties:
   - identity is `sha256(len:issuer|len:messageId)`;
   - an exact retry returns `duplicate:true`;
   - reusing an id with different content goes to **quarantine**;
   - expiry and a monotonic cursor;
   - acknowledgement is separate from receive.

   The idea of quarantining a reused id with a different digest is a cheap integrity check we could apply to client-supplied idempotency keys on `send`. INFERRED.
8. **Resume by captured thread id.** The console reads `thread.started.thread_id` from `codex exec --json` and drives later turns with `codex exec resume <id> -`, with the prompt on stdin "so it cannot be read as a flag" (`harness.ts:1-10,48-56`; `stream.ts:53-58`). This is useful if we ever offer a headless "answer for an offline Codex seat" mode. It does not apply to a live pane (primer §6). VERIFIED in Ruflo.

---

## 8. Does Ruflo's generic mechanism work well on Codex? (assessment using the primer)

- **MCP pull (`team_inbox`)** works on Codex, because Codex supports stdio MCP tools (primer §4). It is the weakest delivery model available:
  - no push, since Codex logs and drops MCP server notifications (primer §4, `logging_client_handler.rs:74-92`);
  - no wake;
  - mail arrives only when the model polls;
  - a blocking long-poll variant would be capped by `tool_timeout_sec` (60 s by default) and would keep the session "busy" (primer §4).

  Compared with herdr-threads (a hook digest at `UserPromptSubmit` and `PreToolUse ^Bash$`, plus a Herdr content-free wake into an empty composer), it is strictly worse for an interactive peer. INFERRED.
- **Headless `codex exec`** fits a lead/worker pipeline, which is Ruflo's model. It cannot address a live pane and re-pays the full context per message. INFERRED.
- **What Ruflo leaves unused**, all of it relevant to herdr-threads (primer §11):
  - `codex queue` / `thread/queue/add`: a server-side idle-gated wake with no draft risk;
  - `turn/steer`: mid-turn delivery;
  - `thread/inject_items`: silent delivery;
  - `Stop` `decision:"block"`: end-of-turn delivery;
  - async hooks;
  - `thread/status/changed` for idle and busy;
  - `notify` `agent-turn-complete`.

  Ruflo offers no evidence for or against these; none of them is exercised in the clone. VERIFIED by absence.
- **Ruflo's one live Codex measurement that matters to us**: `UserPromptSubmit` `hookSpecificOutput.additionalContext` reached the model in 8 of 9 runs on Codex 0.162.0 through OpenRouter (`gpt-5.1-codex-mini`). The one miss was the model ignoring context that had been delivered (`ADR-491…md:34`). This independently corroborates the injection channel herdr-threads relies on for Codex. VERIFIED (ADR text), not reproduced here.

---

## Verification

Adversarial pass against the clone at `e00a82f2` and the Codex primer. Nothing was executed.

Checked and holding: `init.ts` optional-package load; `mcp-config.ts` registration command; `initializer.ts` (AGENTS.md, `.agents/config.toml` and its `[hooks]` keys, override, `.codex/config.toml` never rewritten and its standard/full values, `codex mcp list --json` fallback, marketplace add with the 180 s budget, the ACTION REQUIRED text); `ruflo-core/hooks/hooks.json` events and matchers; `ruflo-hook.cjs` (empty PreToolUse stdout on Codex, `isCodexPluginHost`, PreCompact plain-text output, swallowed CLI output, slot cap); `codex-mod-runtime.mjs` (SessionStart emits nothing, PreToolUse emits only a deny); the `ruflo-swarm` no-op manifest; `team-tools.ts` (bounds 500,000/1,000, drain by default, bus loader); `grok-team-bus.mjs` (priority file names, `wx`/0600 under the team lock, rename claim, no page bound, Grok-only spawn protocol, team `host` defaulting to `grok`); `mcp-server.ts:985` pretty-printed JSON and the empty `resources/*` answers; the orchestrator's argv, `stdin.end()` (#2947), 1 MiB cap, 5 min timeout, 500 ms dependency poll and memory protocol; `loop/index.ts` (prompt, 270 s/10 defaults, 8,000-char truncation, no `resume`); console `harness.ts`/`stream.ts` argv, `thread.started` capture and the 10 min cap; ADR-465:62, ADR-442:15, ADR-486:39-42, ADR-490:55,59,125-127, ADR-491 rows; `codex-mod-hooks.test.mjs` app-server `hooks/list` test (opt-in through `CODEX_NATIVE_LOADER`); the inbox reference model; the "300+ tools" and README "314" claims; no `enabled_tools`/`disabled_tools`; greps for `app-server`, `exec resume`, `queue`, `steer`, `inject_items`.

Changed:
- §2 intro and §2.1 heading: the shell pull (2.2) also reaches a live TUI, so `team_inbox` is the main path, not the only one.
- §2.1: the MCP team tool list was incomplete. Added `team_create`, `team_spawn` and `team_shutdown`, with the line range.
- §2.1: marked the code excerpt as condensed. It is not verbatim.
- §2.4: the console confirm card gates only a session start or resume. Later turns go out on Enter. The claim that each turn is confirmed was wrong.
- §1: `ruflo-adr`/`ddd`/`sparc` are not installed by `init`. Only `ruflo-core@ruflo` is.
- §2.5 grep claim: `team_inbox` also appears in ADR-402 and the Grok template docs. None of those is a Codex hook, so the conclusion stands.
- §3.1: `exec resume` also appears in ADR-407:81.
- §3.2 dual-mode: the network inference was cited to primer §6, which does not cover sandbox network. It is now unsourced INFERRED. Added the worker-env filter that strips `*KEY`/`*TOKEN` variables (`orchestrator.ts:603-613`).
- §3.2 loop: added the 30 min per-iteration timeout, that a non-zero exit ends the loop, and that the sandbox is an option.
- §4 skills citation: added `copyBundledSkills` (`:352`). `generateSkill` starts at `:652`.
- §5: the telemetry cap never drops `session-end`.
- §6 #5: added ADR-491's binary-derived event list as corroboration.

Not checked: GitHub releases and API metadata. They were not needed for any claim in this file.
