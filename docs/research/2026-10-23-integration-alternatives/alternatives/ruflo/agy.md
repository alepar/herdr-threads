# ruflo x Antigravity CLI (agy): message injection and wake

Upstream: https://github.com/ruvnet/ruflo. Clone read: shallow, HEAD `e00a82f2fa2fa2413dd89592f04aae5d4ca35872`
(latest releases v3.60.1 and v3.60.0 on 2026-10-23, v3.59.17 on 2026-10-22, from `gh api repos/ruvnet/ruflo/releases`).
Paths are relative to the clone root. The agy facts come from the harness primer `../../primers/agy.md`
(§ numbers below). The ruflo architecture summary is in `overview.md` next to this file.

Tags: **VERIFIED** means read in source, docs or the GitHub API. **INFERRED** means my conclusion from what I read.
**unknown** means I found no source that answers it.

## 1. Support status

**ruflo does not support agy.** VERIFIED.

- A case-insensitive grep of the clone for `antigravity` and for `agy` as a whole word finds nothing outside `node_modules`.
  VERIFIED.
- The clone has no agy hook surface. A grep for `injectSteps`, `PreInvocation`, `PostInvocation`, `ANTIGRAVITY`,
  `.agents/hooks` and `mcp_config.json` finds one hit, in `docs/USERGUIDE.md:1447`. That line is
  Windsurf's `~/.codeium/windsurf/mcp_config.json`, not agy's. VERIFIED.
- The `init` flags cover Claude (the default), `--codex`, `--dual` and `--grok`. There is no agy or Gemini flag
  (`v3/@claude-flow/cli/src/commands/init.ts:1878-1907`). The init generators are
  `claudemd`, `grok`, `mcp`, `settings`, `statusline` and `helpers`, under `v3/@claude-flow/cli/src/init/`. VERIFIED.
- The headless worker orchestrator types the platform as `'claude' | 'codex'`
  (`v3/@claude-flow/codex/src/dual-mode/orchestrator.ts:33,48,207`). VERIFIED.
- The console session browser has adapters only for `claude`, `codex` and `ruflo`
  (`v3/docs/adr/ADR-486-session-workspace.md:36-40`). VERIFIED.
- The adjacent mentions do not amount to agy support:
  - `Gemini CLI` appears as a skills-install target id, `{ id: 'gemini-cli', label: 'Gemini CLI' }`
    (`plugins/ruflo-console/hooks/data/skills.ts:107`). It is an argument passed to the third-party `skills` CLI, not an adapter.
  - Gemini CLI's BeforeModel and AfterModel hooks are surveyed as prior art
    (`v3/docs/adr/ADR-451-mod-capability-roadmap.md:213-214`).
  - "Gemini" also appears as an LLM provider (`v3/@claude-flow/providers/src/google-provider.ts`). That is
    model routing, not a harness.
  - The GitHub issue search for "antigravity" returns #1567 and #954. #1567 mentions it only because the reporter ran
    "Claude Code extension: ... 2.1.77 (Antigravity)", which is Claude Code inside the Antigravity IDE.
    #954 (closed, 2026-01) reports MCP protocol robustness bugs and cites a third-party
    `antigravity-claude-flow-mcp-router` (github.com/JLMA-Pro-Trading) that fronts the claude-flow MCP server for
    Antigravity; the maintainers praised it but shipped no agy integration. The search for "agy" returns nothing.
    VERIFIED (`gh search issues`, `gh api repos/ruvnet/ruflo/issues/1567`, `.../issues/954` and its comments).

The README offers a generic path for any MCP client: "`npx -y ruflo@latest mcp start`. MCP exposes Ruflo tools to
your client. The console and mods above are Claude Code integrations; hook support depends on the client."
(`README.md:50-53`). VERIFIED. The rest of this report covers what that generic path, and ruflo's other host-agnostic
pieces, would do on agy.

### Pieces that would land on an agy host anyway

- **A default `ruflo init` writes `.agents/skills/ruflo/SKILL.md`** into the project.
  - The write is in `maybeInstallSkillsSh`, `v3/@claude-flow/cli/src/commands/init.ts:338-390`, with
    `const skillDir = path.join(ctx.cwd, '.agents', 'skills', 'ruflo')` at `:345`. It is called at `:865` unless
    `--skip-claude` is passed. VERIFIED.
  - The file is about 2 KB (comment at `:205-211`). Its description names Gemini and says nothing about agy (`:218`). VERIFIED.
  - agy discovers workspace skills under `.agents/skills/` and lists each by name and path in the system prompt
    (primer §3). So agy would list this skill, by accident rather than by design. INFERRED.
- **A default `ruflo init` on a machine with `codex` on PATH also writes Codex files agy reads.**
  `maybeAutoDetectCodex` (`init.ts:145-196`, called at `:864`) runs the Codex initializer unless `--no-codex-detect`,
  `--skip-claude` or `--format json`, producing `AGENTS.md`, `.agents/config.toml` and `.agents/skills/*`
  (`init.ts:186-189`; `v3/@claude-flow/codex/src/initializer.ts:112-118`). It needs `@claude-flow/codex` resolvable,
  else it only prints a hint (`:164-170`). VERIFIED. agy loads `AGENTS.md` and `.agents/skills/` (primer §3), so
  those Codex instructions and skills would also reach an agy session in that project. INFERRED.
- **Nothing is written specifically for agy.** ruflo writes no `.agents/mcp_config.json`, no
  `~/.gemini/config/mcp_config.json`, no `hooks.json` and no agy plugin. VERIFIED by the grep above. A user who wants
  MCP on agy adds it by hand.

## 2. Injection mechanism

### 2.1 What ruflo provides that could reach agy

ruflo's only host-agnostic messaging is the ADR-402 "Agent Teams" bus. It is exposed two ways:

- as the MCP tools `team_send`, `team_inbox`, `team_broadcast`, `team_status`, `team_on_stop` and others
  (`v3/@claude-flow/cli/src/mcp-tools/team-tools.ts:71-274`);
- as a zero-dependency CLI over the same store, `node scripts/grok-team-bus.mjs <create|spawn|send|inbox|...>`
  (`v3/@claude-flow/cli/templates/grok/scripts/grok-team-bus.mjs:10,450-497`).

VERIFIED. The ADR's status is "Proposed", but its implementation phases 1-4 are marked done
(`v3/docs/adr/ADR-402-host-agnostic-agent-teams.md:3,116-122`). VERIFIED.

The other mechanisms are Claude-only and would do nothing on agy:

- Claude Code's native `SendMessage` and Agent Teams (`CLAUDE.md:570-626`).
- The teammate-plugin's `~/.claude/teams` mailbox.
- The `UserPromptSubmit` stdout routing context (`.claude/helpers/hook-handler.cjs:791-831`).
- The mods' `$.prompt.submit`.

VERIFIED that they are Claude-bound (overview §4). That they are inert on agy is INFERRED: agy has no `UserPromptSubmit`
event, and it ignores hook output that is not JSON (primer §2.2-2.3).

### 2.2 The exact path on agy: pull through an MCP tool call

agy's client calls the stdio MCP server's `tools/call`. The server runs the tool and wraps the result:

```ts
// v3/@claude-flow/cli/src/mcp-server.ts:978-985
const result = await callMCPTool(toolName, toolParams, { sessionId, transport: 'stdio' });
...
result: { content: [{ type: 'text', text: JSON.stringify(result, null, 2) }], isError },
```

`team_inbox` loads the bus and calls `readInbox`:

```js
// grok-team-bus.mjs:282-305 (abridged)
const files = pendingFiles(dir);                 // readdir, *.json, lexical sort (:272-275)
for (const f of files) {
  if (!peek) { fs.renameSync(file, archived); }  // claim by rename; ENOENT => another reader took it
  const msg = readJson(file); if (msg) messages.push(msg);
}
return { action: 'inbox', team, agent, messages, peek };
```

VERIFIED.

**When it fires.** Only when the agy model decides to call `team_inbox`. VERIFIED (it is a tool). Nothing
triggers that call:

- no agy hook nudges it;
- the server sends no MCP notification;
- the only prompt that tells the model to call it is the spawn plan's "comms protocol",
  `Read your inbox first: team_inbox (team=..., agent=...)` (`grok-team-bus.mjs:90-105`).

That protocol text reaches a model only if a lead passes it as the child's prompt. VERIFIED. agy's `start_subagent`
or other subagent tool would have to carry it, and `team_spawn` emits only `host.grok` and `host.claude` blocks
(`:107-129`). There is no agy block. VERIFIED.

**Server push.** The server answers `notifications/initialized` and otherwise only serves requests.
It advertises the `resources` capability but serves an empty list (`mcp-server.ts:998-1015`). It emits no
`notifications/*` to the client. VERIFIED. On agy, server-pushed notifications are undocumented anyway (primer §4).

**Payload format.** One MCP text content block. It holds pretty-printed JSON:
`{"success":true,"action":"inbox","team":…,"agent":…,"messages":[…],"peek":false}`.

Each message is the object built at `grok-team-bus.mjs:247-257`:

```js
{ id: `msg_${Date.now()}_${rand8}`, teamId, from, to, summary, content, type, priority, timestamp }
```

VERIFIED. The bus result is spread into `{success: true, ...}` (`team-tools.ts:57`, an object, not a string), and
`mcp-server.ts:984` stringifies that once with 2-space indent. There is no header line and no human-oriented framing.
VERIFIED from the code.

**Size bounds.**

| Bound | Value | Evidence |
|---|---|---|
| Send, MCP path | `message` ≤ 500,000 characters; `summary` ≤ 1,000; `type` ≤ 64; `team_spawn` `prompt` ≤ 100,000 | `team-tools.ts:121,152-154`, via `validateText`, which checks length and strips NULs only (`v3/@claude-flow/cli-core/src/mcp-tools/validate-input.ts:120-131`). VERIFIED |
| Send, CLI path | No length limit: `sendMessage` checks only that the content is non-empty | `grok-team-bus.mjs:231-237`. VERIFIED |
| Read | **No paging and no truncation.** One drain returns every pending file in full | `:282-305`. VERIFIED |
| agy side | Whether agy truncates large MCP results: unknown | primer §4 is silent |

A backlog of large handoffs therefore becomes one unbounded tool result. INFERRED.

**Ordering.** Files are named `NNN_<msgid>.json`, with the priority zero-padded to 3 digits (`:229`), and read in
lexical order (`:274`). So the order is priority ascending, then the `msg_<ms>_` timestamp. VERIFIED.

There is one quirk. Millisecond timestamps compare correctly as strings only while they have the same number of
digits, which holds until the year 2286. Two messages in the same millisecond sort by random suffix. INFERRED.

**De-duplication and ACK.**

- A drain is the consumption. The file moves to `mailbox/<agent>/archive/`, and concurrent drains are disjoint
  (`:277-281,292-298`). VERIFIED.
- There is no ACK and no receipt. VERIFIED (overview §6).
- `peek:true` returns everything pending on every call, and it has no cursor (`:286-301`). VERIFIED.
- Consequence for agy (INFERRED): a drained message counts as delivered even if the model never reads it. Examples:
  - the turn is aborted after the tool result (an agy injected-step error aborts the turn, primer §2.3);
  - the user presses `esc`;
  - the result is too big for the model to use.

  Recovery is manual, from `archive/`.

**Identity.** The model must supply `team` and `agent` on every call. These names come from prompt text, not from
agy's `conversationId` (`team-tools.ts:173-178`). VERIFIED. Two consequences (INFERRED):

- An agy session that loses its spawn prompt has no way to rediscover its inbox name. That happens after `/clear`,
  where agy issues a new id and `SessionStart` fires again (primer §2.2), or after a compaction that drops the prompt.
- A model that is confused about its name can drain another member's mail.

**Project root.** The store lives under `getProjectCwd()`. That is `CLAUDE_FLOW_CWD`, unless it names a root or
home directory, else `process.cwd()` (`v3/@claude-flow/cli-core/src/mcp-tools/types.ts:47-58`). VERIFIED.

On agy, the MCP server's cwd depends on the `cwd` field in `mcp_config.json` (primer §4). With a global
`~/.gemini/config/mcp_config.json` entry and no `cwd`, the default working directory is unknown. If it is not the
project directory, the teams land in the wrong `.claude-flow/` unless `CLAUDE_FLOW_CWD` is set in `env`. INFERRED risk.

### 2.3 Paths ruflo has that agy could, but would not, use

- **Hook context.** ruflo's only hook that injects message state is `swarm-hooks.sh pre-task`. It is not wired by
  any shipped hooks file (overview §4.4). It prints Claude's
  `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","additionalContext":"**Swarm Activity**: ... New messages: N ..."}}`
  (`.claude/helpers/swarm-hooks.sh:620-628`). VERIFIED.

  If a user registered it as an agy `PreToolUse` hook, the object would have no `decision` field. agy's `PreToolUse`
  fails closed, and `{}` denies (primer §2.3). So the hook would **deny every tool call whenever mail is pending**.
  INFERRED from the two VERIFIED facts. Registered as `PreInvocation`, it would be ignored, because it carries no
  `injectSteps` (primer §2.3).

- **The CLI fallback.** The spawn protocol already tells a host without MCP to run
  `node scripts/grok-team-bus.mjs inbox --team T --agent A` (`grok-team-bus.mjs:96-98`). VERIFIED.

  On agy that is a `run_command`. Under `request-review` mode it needs a permission grant, because only
  `~/.gemini/antigravity-cli/settings.json` `permissions.allow` approves commands and a hook allow does not
  (primer §7). INFERRED. The output is pretty-printed JSON of the same bus result, with `ok: true` in place of the
  MCP path's `success: true` (`:437,513`). VERIFIED. The CLI resolves its root from `--root`, `CLAUDE_PROJECT_DIR`,
  `GROK_WORKSPACE_ROOT` or `process.cwd()` (`:454-456`), not `getProjectCwd()`. VERIFIED.

## 3. Wake mechanism

**ruflo has no wake mechanism for agy, or for any idle interactive peer.** VERIFIED (overview §5). The grep finds
no keystroke injection into agent panes for delivery, no `agentapi`, no Remote Control and no `/schedule` use.
`tmux send-keys` appears only for console sandboxes (`plugins/ruflo-console/tests/sandbox.spec.ts:86`) and in the
console's Claude e2e driver for dismissing startup dialogs (`plugins/ruflo-console/scripts/e2e-lib.sh:101-110`).
VERIFIED. The clone has no agy references at all.

The wake patterns ruflo does have, and how each fares on agy:

| ruflo pattern | Evidence | On agy |
|---|---|---|
| Lead-driven re-dispatch: SubagentStop → `team_on_stop` → a hint "Spawn or resume agent X" | `grok-team-bus.mjs:354-385`; `templates/grok/hooks/subagent-stop-team.json` | agy has no SubagentStop event. Hooks fire in children under their own `conversationId`, with no parent, role or description field (primer §2.2, §7). The `<role>:<agent>@<team>` spawn-description trick (`grok-team-bus.mjs:57-71`) cannot be parsed from an agy payload. A child-side `Stop` hook could call `on-stop`, but it would not know its own agent name unless one was baked into an env var or file. **Not portable as is.** INFERRED |
| Headless relaunch per task: `claude -p` / `codex exec` | `orchestrator.ts:205-241` | An `agy -p ... --output-format text` branch would be easy to add. It does not exist (the platform enum is claude/codex). Each "wake" would be a fresh conversation unless it passes `--conversation <id>`. That resumes headless, outside the user's TUI (primer §5). INFERRED |
| Harness-native background notification (Claude): teammates spawned with `run_in_background: true` | `CLAUDE.md:292-308` (overview §5) | agy's nearest native equivalent is a `/schedule` or `schedule`-tool timer, which "wakes the existing agent" (primer §6). ruflo never uses it. VERIFIED absence |
| Scheduled turns: the console launches Claude's `/loop` | `v3/docs/adr/ADR-414-console-loop-manager.md:10-30` | The agy analogue would be a cron `/schedule` that tells the agent to call `team_inbox`. That is a billed poll per tick, and only the user or the model can create it (primer §6). INFERRED |

**Idle/busy detection and draft safety.** Not applicable: ruflo never types into an agy composer, so it cannot
clobber a draft. VERIFIED by absence. The flip side is that a message to an idle agy session sits on disk until a
person or a schedule prompts that session. INFERRED.

**Latency.** With pull only, latency is the time until the model next decides to call `team_inbox`. For an idle
session that is unbounded. INFERRED.

**Failure modes** (all INFERRED):

- A silent backlog when the recipient is idle.
- Silent loss-of-attention when a drain is not followed by the model acting on it.
- Mail sent to the wrong team root when the MCP server's cwd is not the project.
- A model that never calls the tool because nothing in its standing context mentions the inbox.

  The platform skill at `.agents/skills/ruflo/SKILL.md` describes swarms and tool counts, not "check your team
  inbox" (`init.ts:216-240`). VERIFIED that the content says nothing about team inbox polling, in the portion read.

## 4. Setup and installation footprint on agy

| Item | Written by ruflo? | Evidence |
|---|---|---|
| `.agents/skills/ruflo/SKILL.md` (project) | Yes, on a default `init`; opt out with `--no-skills-sh` or `RUFLO_NO_SKILLS_SH=1` | `init.ts:338-390,865`. VERIFIED |
| `.claude-flow/teams/<team>/team.json`, `team.lock`, `mailbox/<agent>/NNN_<id>.json` (mode 0600), `archive/` | Yes, at runtime, by the bus | `grok-team-bus.mjs:136-168,259-266`; ADR-402 `:149`. VERIFIED |
| MCP registration in `~/.gemini/config/mcp_config.json` or `.agents/mcp_config.json` | No. The user must add `{"mcpServers":{"ruflo":{"command":"npx","args":["-y","ruflo@latest","mcp","start"]}}}` by hand, ideally with `--tools team` and a `cwd` or `CLAUDE_FLOW_CWD` | VERIFIED absence; the `--tools` flag is at `v3/@claude-flow/cli/src/commands/mcp.ts:92-96`; the config shape is from primer §4 |
| agy hooks, plugin, permissions, rules | None | VERIFIED absence |
| Claude artifacts from the same `init` (`.claude/settings.json`, helpers, `CLAUDE.md`) | Yes, but agy ignores them | overview §9. agy reads `AGENTS.md` and `GEMINI.md`, not `CLAUDE.md` (primer §3). INFERRED |
| Codex artifacts (`AGENTS.md`, `.agents/config.toml`, `.agents/skills/*`) | Yes, when `codex` is on PATH and `@claude-flow/codex` resolves; agy reads `AGENTS.md` and `.agents/skills/` | `init.ts:145-196,864`. VERIFIED; agy pickup INFERRED |

## 5. Token cost

**Fixed cost per request (the MCP tool catalogue).**

- An unfiltered `mcp start` advertises every registered tool. The README claims 314.
- ruflo itself flags this. `doctor` estimates the catalogue as JSON bytes ÷ 4 and warns at ≥ 8,000 tokens or
  ≥ 20% of a configured window (`v3/@claude-flow/cli/src/mcp-server.ts:347-380`;
  `v3/@claude-flow/cli/src/commands/doctor.ts:1599-1621`). VERIFIED.
- The actual unfiltered figure is unknown, because I did not run it.
- Filtering to the team tools (`--tools team`, or `CLAUDE_FLOW_MCP_TOOLS=team`) leaves 9 tools. From the source text at
  `team-tools.ts:71-274` I estimate about 5 KB of names, descriptions and schemas, so roughly 1.2-1.5k tokens on every
  model call. INFERRED estimate.
- Whether agy loads every MCP tool schema eagerly or defers it is unknown.

**Per message delivered.**

- The body, plus about 200-250 bytes of JSON keys, ids, timestamps and 2-space indent per message (`id`, `teamId`,
  `from`, `to`, `summary`, `content`, `type`, `priority`, `timestamp`), plus the result envelope. That is about
  60-80 tokens of framing per message, against our one header line. INFERRED estimate from `grok-team-bus.mjs:247-257`
  and `mcp-server.ts:984`.
- The `summary` field duplicates content the model already gets in `content`. INFERRED.
- Add the model's own tool-call tokens (about 30-50) and, if a permission prompt fires, a human round trip. INFERRED.

**Per quiet turn.**

- If the model never polls, the cost is zero beyond the fixed catalogue.
- If it polls once a turn, as the spawn protocol suggests ("Read your inbox first"), each empty poll costs a tool call
  plus a result of about 100 bytes, `{"success": true, "action": "inbox", ..., "messages": [], "peek": false}`.
  That is about 60-90 tokens and one extra model round trip per poll. INFERRED.
- A `/schedule` cron poll on top costs a full billed turn per tick. INFERRED.

## 6. Limitations and bugs visible (as they bear on agy)

1. **No agy support at all.** Every agy delivery would be model-initiated pull, with no turn-start or mid-turn
   injection, although agy offers both (`SessionStart` and `PreInvocation` `injectSteps`, primer §2.5). VERIFIED absence.
2. **Drain-is-delivery.** There is no ACK, receipt or redelivery, so an aborted turn after a drain loses the message
   from the pending set (`grok-team-bus.mjs:290-301`). VERIFIED code; the consequence is INFERRED.
3. **Unbounded read.** There is no page limit and no body truncation on `team_inbox`, and the CLI `send` has no size cap
   (`:231-237,282-305`). VERIFIED.
4. **Identity is prompt text.** `team` and `agent` are caller-supplied on every call, with no binding to
   `conversationId` (`team-tools.ts:173-182`). Any agy session can drain any mailbox. VERIFIED code; the risk is INFERRED.
5. **`team_spawn` emits only `host.grok` and `host.claude`** (`grok-team-bus.mjs:113-128`). Its protocol text hard-codes
   "Grok Build 1.0.41 runs you as a general-purpose subagent" and "There is no Claude SendMessage tool" (`:92-94`).
   An agy child would get Grok-specific instructions. VERIFIED.
6. **Hook output shapes are Claude-only.** Porting `swarm-hooks.sh` to agy `PreToolUse` would deny tools, as described
   in §2.3. INFERRED.
7. **Wrong-root risk.** The store root follows the MCP server's cwd (`types.ts:53-58`). VERIFIED code; the agy-side
   default cwd is unknown.
8. **Accidental skill exposure.** The `.agents/skills/ruflo` skill is visible to agy in that project even though ruflo
   supports nothing there. INFERRED.

## 7. Clever ideas worth stealing

1. **Measure and gate the fixed per-request context cost.** `doctor`'s "MCP Schema Overhead" check computes the bytes of
   the advertised catalogue, converts them to tokens and warns above a threshold or window ratio, with a concrete fix
   (`mcp-server.ts:347-380`, `doctor.ts:1599-1621`). VERIFIED.

   We have no equivalent measurement of our own standing per-session cost on agy: the skill listing, the plugin
   description, a quiet-turn `PreInvocation` output (empty today) and the systemMessage wrapper overhead. A
   `doctor`-style line reporting those bytes per harness would keep the "Token-efficient communication" rules
   measurable rather than reviewed by eye. INFERRED, worth a small follow-up.

2. **Advertise-time tool filtering** (`--tools <category|prefix|name>`, `mcp-server.ts:320-345`). VERIFIED. If we ever
   ship an MCP surface for agy (for example a long-poll tool, primer §11.6), expose only the few tools we need and keep
   execution of the rest internal.

3. **Disjoint claims by rename.** A drain claims each file by `rename` into `archive/` and skips on `ENOENT`
   (`grok-team-bus.mjs:290-299`). Lock-free concurrency for multiple readers. VERIFIED. Our SQLite store already gives
   stronger guarantees, so this is only useful as a pattern for client-side hint files, which must stay non-authoritative
   (TRUST-POLICY A2). INFERRED.

4. **A capability table with a reason per harness adapter** (ADR-486 `HarnessAdapter.capabilities`, each
   `{supported, why}`, `ADR-486...md:30-40`). VERIFIED. Surfacing "wake: unavailable (reason)" per seat in our
   diagnostics would make agy's degraded states visible: the systemMessage → userMessage downgrade, an
   `Unreadable` composer. INFERRED.

5. **A CLI fallback named in the protocol text**: "Without the Ruflo MCP server, use the CLI: ..."
   (`grok-team-bus.mjs:96-98`). VERIFIED. We are CLI-first already, so this confirms our choice rather than adding to it.

What ruflo does **not** teach for agy: how to wake an idle TUI session, how to inject mid-turn, or how to confirm
delivery. On all three, herdr-threads' agy adapter is ahead:

- `PreInvocation` `systemMessage` injection with transcript confirmation and the `userMessage` downgrade;
- Herdr native wake into an empty composer, read by `AgyComposer` (primer §10).

The leads worth pursuing remain the primer's own: `agentapi send-message`, the statusline `state` signal, and
`PostInvocation` and `Stop` `continue` for items that arrive while busy (primer §11). ruflo offers no evidence on any
of them. INFERRED.

## Verification

Adversarial pass against the clone at `e00a82f2`, `gh api repos/ruvnet/ruflo/releases`, `gh search issues` and the agy primer.

Checked and confirmed: the absence greps (`antigravity`, `agy`, agy hook names; the single `docs/USERGUIDE.md:1447` Windsurf hit);
init flags (`init.ts:1878-1907`) and generator list; orchestrator platform type (`orchestrator.ts:33,48`) and headless args
(`:205-241`); ADR-486 capability table; `skills.ts:107`; ADR-451 `:213-214`; ADR-402 status, phases and lock/mailbox layout;
`README.md:50-53`; `maybeInstallSkillsSh` (`init.ts:338-390`, `:345`, `:865`, skill text `:216-240`); the 9 team tools and
the 500,000 / 1,000 / 64 / 100,000 limits; `validateText`; `getProjectCwd`; `mcp-server.ts` tool call wrapping, empty
resources and no server notifications; `readInbox`, `sendMessage`, file naming, rename claims and 0600 mode; spawn plan
protocol text and `host.grok` / `host.claude` only; `onStop` hint; `swarm-hooks.sh:620-628` output shape; ADR-414;
`doctor` schema-overhead check and `--tools` filter; release tags and dates; primer §2-§7 facts as cited.

Changed:
- Issue search for "antigravity" returns #954 as well as #1567; #954 cites a third-party Antigravity MCP router. Added.
- "Nothing else is written for agy" was incomplete: a default `init` with `codex` on PATH also writes `AGENTS.md` and
  `.agents/skills/*`, which agy reads. Added to §1 and the §4 table.
- "JSON is stringified twice" was wrong: the bus result is spread into an object and stringified once. Corrected.
- The CLI fallback output uses `ok: true`, not `success: true`, and resolves its root differently from the MCP path. Corrected.
- `send-keys` also appears in the console's Claude e2e driver, not only sandboxes; added citations.
- `CLAUDE.md:14` did not support "harness-native background notification"; replaced with `CLAUDE.md:292-308`.
