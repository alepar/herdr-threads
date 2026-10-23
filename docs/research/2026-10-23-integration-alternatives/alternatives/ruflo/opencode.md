# ruflo x OpenCode: message injection and waking

Upstream: https://github.com/ruvnet/ruflo. Clone read: shallow, HEAD `e00a82f2fa2fa2413dd89592f04aae5d4ca35872`. Paths are relative to the clone root unless they carry a URL.
Harness reference: `../../primers/opencode.md` (OpenCode 1.18.1 stable, 2.x dev). Ruflo context: `overview.md` in this directory.
Tags: **VERIFIED** = read in source, docs or GitHub; **INFERRED** = my conclusion; **unknown** = not found.

## 1. Support status

**Not supported.** Ruflo has no OpenCode adapter, no OpenCode plugin, no `init --opencode`, and no OpenCode branch in any comms path.

Evidence. A case-insensitive grep for `opencode`, `sst/` and `anomalyco` over the whole clone (lockfiles excluded) finds the five kinds of hit below plus `anomalyco` false positives (`AnomalyContext`, `AnomalyDetectionService` in `v3/plugins/financial-risk/` and `v3/@claude-flow/plugin-iot-cognitum/`); none is integration code:

| Hit | What it is | Tag |
|---|---|---|
| `v3/docs/adr/ADR-451-mod-capability-roadmap.md:217-218` | Prior-art bullet: "In-process plugins that register tools: opencode. TS/JS plugins with `tool.execute.before/after` and a `tool()` helper ... (summary-grade). The mod API has `$.tool.register`; only ruflo-console uses it." This is a lesson for ruflo's *Claude* mod API, not an OpenCode integration. | VERIFIED |
| `plugins/ruflo-metaharness/skills/harness-mint/SKILL.md:46-48`, `plugins/ruflo-metaharness/commands/ruflo-metaharness.md:50` | `opencode` in the host list of `npx metaharness new --host <id>`, which scaffolds a new harness project from a template into a temp dir. It is a separate npm project (`metaharness`), shelled out to, and has nothing to do with ruflo's messaging. | VERIFIED |
| `plugins/ruflo-console/hooks/data/skills.ts:105-112` | `{ id: 'opencode', label: 'OpenCode' }` in `AGENT_TARGETS`, the fixed list passed as `--agent` to the third-party `npx -y skills@1.7.2` CLI (vercel-labs/skills) so the person can install a skill into OpenCode's skill folder from the ruflo console. A skill-install convenience, not delivery. | VERIFIED |
| `data/npm-downloads.latest.json:963,1044` | Download counts for `@metaharness/host-opencode` (9,319) and `@metaharness/opencode` (171). Statistics only. | VERIFIED |
| `ruflo/src/ruvocal/.../parseIncompleteMarkdown.ts` | `getOpenCodeFenceIndex` = "open code fence" in a Markdown parser. False positive. | VERIFIED |

GitHub. Issue #2421 "Opencode" (open since 2026-06-19) asks whether OpenCode support is planned; the maintainer replied only "Try my npx metaharness or the metaharness capabilities in ruflo" (https://github.com/ruvnet/ruflo/issues/2421). Issue #310 "Feasibility study of making claude-flow a universal orchestration layer" (open since 2025-07-15) has repeated OpenCode requests; a contributor notes `.mcp.json`, `settings.json` and `CLAUDE.md` are "tightly coupled to claude", and the 2026-08-17 nightly triage comment records "needs human input" with no closed PRs (https://github.com/ruvnet/ruflo/issues/310). VERIFIED.

Community: `rvcgeeks/opencode-ruflo-workspace` (0 stars, created and last pushed 2026-07-25) pairs the OpenCode TUI with ruflo's MCP server plus a build-time "ruflo-plugin-translator" that "writes .opencode/" (commands and agents). Per its README, the integration is static translation plus MCP; it describes no injection or wake mechanism. VERIFIED (README via `gh api repos/rvcgeeks/opencode-ruflo-workspace/readme`; code not read).

Mapper's note confirmed: none; ADR-451 surveys OpenCode only as prior art. VERIFIED.

The rest of this report covers what ruflo's **generic** mechanisms would do in OpenCode, based on the primer.

## 2. Injection mechanism

### 2.1 What ruflo has that is harness-agnostic

Of the eight injection paths in `overview.md` section 4, only the MCP pull path (item 2) is host-neutral and implemented; the CLI below is a second front end to the same store. Item 6 (ADR-385 coordinator inbox) is a proposal of unknown status, and item 8 (tmux) targets shell sandboxes only.

1. **MCP pull: `team_send` / `team_inbox` (ADR-402 bus).** The MCP server is a plain stdio server (`npx -y ruflo@latest mcp start`); OpenCode is an MCP client. VERIFIED (both sides).
2. **The CLI copy of the same store**: `node scripts/grok-team-bus.mjs inbox|send ...`, which `init --grok` copies into a project. An OpenCode agent could call it through its bash tool. VERIFIED that the CLI exists (`v3/@claude-flow/cli/templates/grok/scripts/grok-team-bus.mjs:471-488`); OpenCode use INFERRED.

Everything else is Claude-only or Grok-only. Claude Agent Teams `SendMessage`, the `.claude/settings.json` command hooks (`hook-handler.cjs route` on UserPromptSubmit), Claude mods (`$.prompt.submit`) and Grok's SubagentStop hook have no OpenCode wiring in ruflo. VERIFIED by the absence above. OpenCode has no Claude-style command hooks; its hook surface is JS/TS plugins (primer section 2), so `.claude/settings.json` hooks would not run there (INFERRED from the primer's surface list).

### 2.2 The MCP path in detail (what would happen in OpenCode)

**Send.** `team_send` validates the body (up to 500,000 characters, summary up to 1,000; `v3/@claude-flow/cli/src/mcp-tools/team-tools.ts:150-154`). The MCP tool loads the template `grok-team-bus.mjs` as its store (`team-tools.ts:8-11,36-44`). It then takes the team lock and writes one file per recipient (VERIFIED, `grok-team-bus.mjs:231-268`):

```js
const messageFileName = (msg) => `${String(msg.priority).padStart(3, '0')}_${msg.id}.json`;
...
fs.writeFileSync(path.join(dir, messageFileName(msg)), JSON.stringify(msg, null, 2) + '\n',
  { encoding: 'utf8', flag: 'wx', mode: 0o600 });
```

- Store: `<project>/.claude-flow/teams/<team>/mailbox/<agent>/`. The project root is the MCP server's cwd (`getProjectCwd()`, `team-tools.ts:57`; layout in `templates/grok/scripts/grok-team-store.mjs:5-9,44-57`). VERIFIED.
- Message: `{id: "msg_<ms>_<rand8>", teamId, from, to, summary, content, type, priority, timestamp}`. Priority 0-999, default 2 (`grok-team-bus.mjs:38-39`); `to="*"` fans out to the current members. VERIFIED.

**Receive.** `team_inbox` (drain by default, `peek` optional). The model in OpenCode has to choose to call it (VERIFIED: `team-tools.ts:167-183`; pull-only because OpenCode's MCP client handles no resource or prompt notifications, primer section 5).

```js
for (const f of files) {           // sorted lexically: priority prefix, then id
  ...
  fs.renameSync(file, archived);   // claim by rename into archive/
  ...
  if (err.code === 'ENOENT') continue; // another reader claimed it
```

(VERIFIED, `grok-team-bus.mjs:284-306`.)

| Property | Behaviour | Tag |
|---|---|---|
| Hook / API | MCP `tools/call` to `team_inbox`; OpenCode exposes it as an MCP tool (named per OpenCode's MCP naming; exact prefix unknown). | VERIFIED (ruflo); naming unknown |
| When it fires | Only when the model calls the tool. Nothing triggers it at turn start or after a tool round. | VERIFIED (no other trigger exists); OpenCode side VERIFIED in primer section 5 |
| Payload format | The MCP reply is `{content:[{type:'text', text: JSON.stringify(result, null, 2)}]}` (`v3/@claude-flow/cli/src/mcp-server.ts:985`), where `result = {success:true, action:'inbox', team, agent, messages:[...full message objects...], peek}` (`success` added by `run()`, `team-tools.ts:57`; the rest from `grok-team-bus.mjs:305`). The model reads pretty-printed JSON. | VERIFIED |
| Framing | None beyond the JSON keys. No "from another agent" header, no trust note. | VERIFIED |
| Size bounds | None in ruflo: one drain returns **every** pending message in full, each up to 500k chars. OpenCode's own tool-output truncation (the `Truncate` service, spilling to a file) would be the only bound. | VERIFIED (ruflo); OpenCode truncation for MCP tools INFERRED in primer section 5 |
| Ordering | Lexical filename sort: zero-padded priority, then `msg_<ms>_...`, which is roughly creation order within a priority (ms timestamps sort lexically only while they have the same digit count, true until 2286). | VERIFIED code; ordering consequence INFERRED |
| Dedupe | Drain-by-rename: a message is handed to at most one drain; concurrent drains get disjoint sets. | VERIFIED |
| ACK | None. A drain *is* consumption. If the drained reply is truncated, lost to compaction, or ignored, the message is gone from the inbox (it stays in `archive/` on disk, but no tool re-offers it). | VERIFIED (no ack path); consequence INFERRED |
| Identity | The caller names itself: `agent` is a free parameter. Nothing binds an OpenCode session to a mailbox name. | VERIFIED (`team-tools.ts:173-182`) |

**Prompting.** The only text that tells an agent to read its inbox is the spawn-plan protocol from `team_spawn`. It is written for Grok: "Grok Build 1.0.41 runs you as a general-purpose subagent ... There is no Claude SendMessage tool. Read your inbox first: team_inbox (team=..., agent=...)" (VERIFIED, `grok-team-bus.mjs:90-105`). The plan's `host` object has only `grok` and `claude` keys (`grok-team-bus.mjs:113-127`). An OpenCode lead could pass this prompt to its own `task` subagent tool, but the Grok-specific lines would be wrong for it. INFERRED.

### 2.3 Static context ruflo would put into OpenCode as a side effect

These are not message injection, but they show up in OpenCode's context if ruflo was initialised for Claude in the same project:

- `ruflo init` generates `CLAUDE.md` with an "Agent Comms (SendMessage-First Coordination)" section: "Named agents coordinate via `SendMessage`, not polling or shared state" (VERIFIED, `v3/@claude-flow/cli/src/init/claudemd-generator.ts:72-89`). OpenCode loads `CLAUDE.md` unless `OPENCODE_DISABLE_CLAUDE_CODE` is set (primer section 6). OpenCode has no `SendMessage` tool, so this guidance would mislead an OpenCode model. INFERRED.
- `init` copies skills into `.claude/skills` (`v3/@claude-flow/cli/src/init/executor.ts:157,213,845`); the repo itself ships 38 under `.claude/skills` and 135 under `.agents/skills` (VERIFIED, `find ... -name SKILL.md`). OpenCode scans both trees and lists every skill in the system prompt (primer section 6). INFERRED: dozens of skill descriptions on every request.
- MCP registration: `init` writes `.mcp.json` with `mcpServers['claude-flow']` (VERIFIED, `v3/@claude-flow/cli/src/init/mcp-generator.ts:49-97`). Whether OpenCode reads `.mcp.json` is unknown (the primer does not cover it); its own MCP config is the `mcp` key of `opencode.json` (https://opencode.ai/docs/mcp-servers/, not fetched).

## 3. Wake mechanism

**None for OpenCode, and none for any idle interactive peer.** VERIFIED by absence (`overview.md` section 5; the grep in section 1).

What the generic mechanisms could do on OpenCode:

| Ruflo mechanism | Works on OpenCode? | Why | Tag |
|---|---|---|---|
| MCP server push | No | Ruflo's server advertises `tools: {listChanged: true}` in its `initialize` reply (`mcp-server.ts:926-938`) and repeats it in a non-standard `server.initialized` frame (`mcp-server.ts:822-836`), but never sends any notification (the only `notifications/*` case is the inbound `notifications/initialized`, `mcp-server.ts:1010`). OpenCode handles `tools/list_changed` by refreshing tool defs, logs `notifications/message`, and advertises no sampling or elicitation, so no MCP traffic can start a turn (primer section 5). | VERIFIED both sides |
| Lead re-dispatch (`team_on_stop` hint "Spawn or resume agent X") | Only with a lead driving it | The hint comes back in a tool result to whichever model calls `team_on_stop` (`grok-team-bus.mjs:354-385`). Ruflo wires the call to Grok's SubagentStop hook only. On OpenCode, a plugin `event` handler on `session.idle` could call it, but none exists. | VERIFIED (ruflo); OpenCode wiring INFERRED |
| Headless relaunch per task (`claude -p` / `codex exec` in `v3/@claude-flow/codex/src/dual-mode/orchestrator.ts:203-241`, `v3/@claude-flow/codex/src/loop/index.ts:219-241`) | Not for OpenCode | The binary paths are configurable (`claudeCommand`/`codexCommand`, defaults `claude`/`codex`, `orchestrator.ts:65-66,115-116`), but `platform` is typed `'claude' \| 'codex'` (`orchestrator.ts:33,48`) and each branch builds that CLI's argv. An `opencode run "<prompt>"` worker is technically possible (primer section 4) but is not implemented. It would be a fresh process, not a wake of the pane's session. | VERIFIED (platform set and argv); OpenCode feasibility INFERRED |
| Console `$.prompt.submit` / `/loop` | No | Claude Code mod APIs. | VERIFIED |
| tmux `send-keys` | No | Used only for console-owned shell sandboxes `ruflo-sb-*` (ADR-442:15), never for agent panes. | VERIFIED |

- **Idle/busy detection:** none. Ruflo never observes OpenCode's `session.status`. VERIFIED by absence.
- **Draft safety:** not applicable, because nothing touches the composer. VERIFIED by absence.
- **Latency:** unbounded. A message sits in the mailbox until a model happens to call `team_inbox`. INFERRED.
- **Failure modes:** a silent miss if no prompt tells the OpenCode agent to poll. If it polls in a loop, every poll is a billed model step. INFERRED.

## 4. Setup and installation footprint on OpenCode

- **Ruflo writes no OpenCode config.** No `opencode.json`/`opencode.jsonc`, no `.opencode/plugin*`, no `tui.json`. VERIFIED by absence.
- **Manual setup a user would need (INFERRED):** add an `mcp` entry for `npx -y ruflo@latest mcp start` to `opencode.json` by hand. Optionally set `CLAUDE_FLOW_MCP_TOOLS=team` in that entry's environment, so that only the `team_*` schemas are advertised. The selector matches a category or name prefix (VERIFIED, `mcp-server.ts:320-345,414-421`).
- **Indirect writes:**
  - The ruflo console's skills page can run `npx -y skills@1.7.2 add ... --agent opencode`, which writes into OpenCode's skill directory, global or project scope (VERIFIED, `plugins/ruflo-console/hooks/data/skills.ts:12-20,105-115,162-164`; the target paths are the skills CLI's and were not read).
  - A Claude-oriented `ruflo init` leaves `CLAUDE.md`, `.claude/skills/` and `.mcp.json` in the project, which OpenCode partly picks up (section 2.3).

## 5. Token cost

- **Per message (INFERRED from VERIFIED formats):**
  - Each message costs its body plus about 250-350 bytes of pretty-printed JSON keys and envelope (`id`, `teamId`, `from`, `to`, `summary`, `type`, `priority`, `timestamp`), plus the `{success, action, team, agent, messages, peek}` wrapper once per call.
  - The model also spends a tool call (one LLM step) to fetch it, plus the step that reads the result.
  - A sender pays for a `team_send` call as well.
- **Per quiet turn:**
  - Delivery itself costs zero, since nothing is injected.
  - The standing cost is the MCP tool catalogue in every request: by default all ruflo tools. The README claims 314; I did not measure their schema size. INFERRED: tens of KB per request unless `CLAUDE_FLOW_MCP_TOOLS` narrows it.
  - Any ruflo skills OpenCode discovers add their listing to every system prompt (section 2.3).
  - A polling agent pays one empty `team_inbox` round trip per poll; an empty drain returns about 100 bytes of JSON.

## 6. Limitations and bugs visible

1. **Pull-only, no wake.** On OpenCode this degrades to "the model may remember to check". VERIFIED (mechanism); INFERRED (behaviour).
2. **Destructive read with no ACK.** A drain removes messages before the model has used them. A truncated or compacted tool result loses them silently. Unlike herdr-threads' display-ACK with receipts, nothing re-offers them. VERIFIED (code); consequence INFERRED.
3. **Unbounded drain.** There is no page size or byte cap, and a single message can be 500k chars, so one `team_inbox` call can flood the context or be truncated by OpenCode. VERIFIED (ruflo side).
4. **Self-asserted identity.** Any OpenCode session can drain any agent's mailbox by passing its name. VERIFIED (`agent` is a free parameter). In ruflo's cooperative model this is a misdelivery risk, not a security one. INFERRED.
5. **Project-cwd scoping.** The store lives under the MCP server's cwd. Two OpenCode sessions in different directories, or one with `--dir`, would not share a team. INFERRED from `getProjectCwd()`.
6. **Host coupling.** The spawn protocol text and the plan `host` keys are Grok and Claude only, and ruflo's generated `CLAUDE.md` mandates `SendMessage`, which OpenCode lacks. VERIFIED (text); misleading effect on OpenCode INFERRED.
7. **No session binding.** Ruflo never learns OpenCode's `sessionID`, and has no `shell.env` export like the one the primer suggests (section 10). VERIFIED by absence.

## 7. Clever ideas worth stealing

Ruflo's OpenCode story is empty, so there are only a few ideas to take, none OpenCode-specific:

- **Advertised-schema filter by category or prefix** (`CLAUDE_FLOW_MCP_TOOLS=team`, `mcp-server.ts:326-345`): execution stays registered and only the per-request catalogue shrinks. If herdr-threads ever ships an MCP surface, this is the cheap way to keep its standing per-request cost small. VERIFIED (mechanism).
- **Priority encoded in a zero-padded filename prefix plus claim-by-rename:** a lock-free, multi-reader-safe drain for a file transport, with lexical order equal to delivery order. Our SQLite store is stronger, but this is a reasonable shape for any client-local spool (for example, a plugin-side buffer while the daemon is unreachable). VERIFIED (mechanism); applicability INFERRED.
- **The prior-art bullet ruflo itself drew (ADR-451:217-218):** OpenCode's in-process plugins with `tool.execute.before/after` and custom tool registration. The primer confirms this is the right surface for an OpenCode adapter: `experimental.chat.messages.transform` or `tool.execute.after` for in-turn delivery, and `session.idle` plus `client.session.promptAsync` for a draft-safe wake. Ruflo noted the surface and never built on it. VERIFIED (ADR text); design INFERRED.

Net for herdr-threads (INFERRED): ruflo offers no OpenCode technique to copy and no evidence about OpenCode behaviour. Its generic MCP pull path would run on OpenCode, but only as a weaker version of our CLI `inbox`, with destructive reads, no ACK, no bounds and no wake. Primer section 12, shape 1 (a server plugin channel) remains the path to build.

## Verification

Checked against the clone at `e00a82f2`, `gh api` (issues #2421, #310 and comments; `rvcgeeks/opencode-ruflo-workspace` metadata and README) and the OpenCode primer.

- Confirmed as written: the ADR-451, metaharness, `skills.ts`, npm-downloads and Markdown-parser hits; issue #2421 reply, #310 "tightly coupled" comment and 2026-08-17 triage ("needs human input", "Zero PRs have closed"); community repo (0 stars, created/pushed 2026-07-25, translator writes `.opencode/`); `messageFileName`/`wx`/`0o600` write; drain-by-rename and ENOENT skip; priority 0-999 default 2; spawn protocol text and `host` keys; `team_on_stop` hint; MCP reply envelope (`mcp-server.ts:985`); `CLAUDE_FLOW_MCP_TOOLS` filter; `CLAUDE.md` SendMessage section; `.mcp.json` `claude-flow` key; 38 / 135 SKILL.md counts; ADR-442:15 tmux scope; README "314 MCP tools" (`README.md:113`); primer sections 2, 4, 5, 6, 10, 12 say what is attributed to them.
- Changed: grep result now names the `anomalyco` false positives; section 2.1 count of host-neutral paths restated against `overview.md` section 4; `team-tools.ts` line numbers corrected (validation 150-154, `getProjectCwd` 57, `team_inbox` 167-183, `agent` 173-182) and the store/template relation cited; MCP-push row corrected (capability is advertised in `initialize`, no notification is ever sent); headless row corrected (commands configurable, platform set fixed) and its paths given in full.
- Not checked: OpenCode `.mcp.json` support and MCP tool naming (left unknown), the skills CLI target paths, OpenCode truncation for MCP tools (left INFERRED).
