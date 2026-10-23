# ruflo x Pi coding agent (pi-mono, badlogic) — injection and wake

Upstream: https://github.com/ruvnet/ruflo, shallow clone at `e00a82f2fa2fa2413dd89592f04aae5d4ca35872`
(2026-10-23). Paths without a prefix are relative to the ruflo clone root. Pi paths are relative to the
pi-mono clone (`a7b94a03f`, see `../../primers/pi.md`) and prefixed `pi:`. One external package by the
same author was read as data (not executed): `@metaharness/host-pi-dev@0.1.3` from the npm registry
(repository `https://github.com/ruvnet/agent-harness-generator`, `packages/host-pi-dev`), paths prefixed
`hpd:`.

Tags: **VERIFIED** = read in source or docs at the cited location; **INFERRED** = my conclusion; unknown
= not found.

## Support status

**Not supported. ruflo has no Pi adapter, no Pi delivery path and no Pi wake path.** VERIFIED.

Evidence:

- `ruflo init` host flags are `codex`, `grok` and `dual`; there is no `pi` flag.
  `v3/@claude-flow/cli/src/commands/init.ts:1878,1884,1902`. VERIFIED.
- A grep of the whole clone for `pi-mono`, `badlogic`, `earendil`, `pi-coding-agent`, `pi.dev`, `.pi/`,
  `PI_CODING_AGENT`, `PI_SESSION_ID`, `"pi"` and Pi's `sendMessage(..., triggerTurn)` returns only:
  - `plugins/ruflo-metaharness/skills/harness-mint/SKILL.md:47` and
    `plugins/ruflo-metaharness/commands/ruflo-metaharness.md:50`: `pi-dev` in the list of `--host`
    values for the `harness mint` scaffolder;
  - `v3/@claude-flow/cli/src/mcp-tools/metaharness-tools.ts:569`: `pi-dev` as an example `host` string
    in the `metaharness_learn` tool schema;
  - `v3/docs/adr/ADR-451-mod-capability-roadmap.md:246`: "Cursor, Cline, Roo, Pi (pi-mono) and
    LangGraph middleware were not researched";
  - `data/npm-downloads.latest.json:972,1053`: download counts for `@metaharness/host-pi-dev` and
    `@metaharness/pi-dev`.
  VERIFIED.
- The `host` value is passed through untouched to the external `metaharness` CLI:
  `plugins/ruflo-metaharness/scripts/mint.mjs:35,40,119` (`['new', cliName, '--template', ARGS.template,
  '--host', ARGS.host]`). ruflo itself contains no Pi code. VERIFIED.

### The one Pi artefact in the ruflo ecosystem: `@metaharness/host-pi-dev`

Out of tree, but it is what `ruflo harness mint --host pi-dev` would end up calling (INFERRED: mint
shells `npx metaharness new ... --host pi-dev`; I did not read the `metaharness` CLI's host resolution).
It is a **static scaffolder**, not a runtime integration:

- Identifies its target correctly: "NOT Inflection's Pi.ai ... This is the badlogic/pi-mono Pi coding
  agent" (`hpd:README.md`; `hpd:dist/index.js:3-9`). VERIFIED.
- `generateConfig(spec)` returns four file contents: `pi-extension/src/index.ts`, `AGENTS.md`,
  `SYSTEM.md`, `trust.json` (`hpd:dist/index.js:100-109`). VERIFIED.
- The generated extension only registers tools that forward to the metaharness wasm kernel:

  ```js
  pi.registerTool({name, description, inputSchema}, async (args) => {
    const { loadKernel } = await import('@metaharness/kernel');
    const kernel = await loadKernel();
    return kernel.invokeTool(name, args);
  });
  ```
  wrapped in `import type { PiContext } from 'pi-agent-sdk'; export async function register(pi) {...}`
  (`hpd:dist/index.js:35-56`). VERIFIED. It subscribes to no Pi event and never calls `sendMessage`,
  `sendUserMessage`, `before_agent_start`, `tool_result` or `agent_settled`. VERIFIED (whole file read,
  110 lines).
- For an `autonomous` spec block it writes into AGENTS.md that Pi "has no native goal/heartbeat/
  gateCommand/maxTurns autonomous-loop surface" and the block "is not projected on this host
  (documented no-op)" (`hpd:dist/index.js:62-68`). VERIFIED. So the ecosystem's own adapter explicitly
  declines to drive Pi turns.

Conclusion: neither ruflo nor its Pi scaffolder injects peer messages into a Pi session or wakes one.

## Injection mechanism

**None Pi-specific.** VERIFIED (above). What would happen if a person wired ruflo's generic surfaces
into Pi by hand:

### A. ruflo MCP server through Pi's built-in MCP client (works, pull-only, hidden by default)

- Setup a person could do: `pi mcp add ruflo -- npx -y ruflo@latest mcp start` (`pi:packages/coding-agent/docs/mcp.md:9-12`
  shows the `pi mcp add` form; ruflo's start command is from its README per `overview.md` section 2).
  INFERRED that this connects; not run.
- The peer-message tools are the ADR-402 bus: `team_send` / `team_inbox` / `team_broadcast`
  (`v3/@claude-flow/cli/src/mcp-tools/team-tools.ts:133,168-184,185`). `team_inbox` describes itself as
  "the Grok/Codex-side counterpart to team_send for hosts without a live message channel"
  (`team-tools.ts:170`). VERIFIED.
- Store and payload: one file per message per recipient,
  `.claude-flow/teams/<team>/mailbox/<agent>/<NNN>_<msgid>.json`, `NNN` = zero-padded priority 0-999
  (default 2), body JSON `{id, teamId, from, to, summary, content, type, priority, timestamp}`, written
  with `flag: 'wx', mode: 0o600` under a team lock
  (`v3/@claude-flow/cli/templates/grok/scripts/grok-team-bus.mjs:38-39,229,231-268`). VERIFIED.
- Read path: `readInbox` lists `*.json`, sorts lexically (so priority then id), and for a drain renames
  each into `archive/` before reading; ENOENT on rename means another reader won it
  (`grok-team-bus.mjs:270-306`). The model gets `{success, action, team, agent, messages, peek}`
  pretty-printed as one MCP text block (`team-tools.ts:47-61`, `v3/@claude-flow/cli/src/mcp-server.ts:985`).
  VERIFIED.
- **When it fires on Pi**: only when the model calls the tool. Pi's MCP client advertises only `roots`,
  handles no resource-update notifications and appends `notifications/message` to a log, never to the
  model (`../../primers/pi.md` section 3, citing `pi:packages/mcp/src/client.ts:176-180`,
  `pi:packages/coding-agent/src/extensions/mcp/runtime.ts:367,380-382`). So an MCP server cannot push into
  Pi at all. VERIFIED (primer).
- **Exposure trap**: Pi's default MCP exposure is `codemode`: tools are "neither declared to the model
  nor listed in the codemode description" (`pi:packages/coding-agent/docs/mcp.md:195`). With ruflo's
  ~314 tools (README claim, `overview.md` section 1) the model would see only a one-line `mcp_servers`
  system-prompt entry and would have to discover `team_inbox` via `searchTools()` inside a codemode
  script. To make it callable directly, the person would need `"toolExposure": {"team_*": "direct"}`
  (`mcp.md:206-221`). VERIFIED (Pi docs); effect on ruflo INFERRED.
- Bounding: at send, `team_send` rejects a `message` over 500,000 and a `summary` over 1,000 via
  `validateText` (`team-tools.ts:150-154`; unit not checked, the validator lives in
  `@claude-flow/cli-core`); the bus `sendMessage` itself has no limit (`grok-team-bus.mjs:231-268`), so the
  CLI path is unbounded; `team_inbox` returns every pending message with no page or per-read cap
  (`grok-team-bus.mjs:283-306`). Pi middle-truncates MCP text results over 20 KB and saves the full text
  to a temp file (`mcp.md:232`). VERIFIED.
- Framing: none; the model reads pretty-printed JSON (mailbox files are also stored as
  `JSON.stringify(msg, null, 2)`, `grok-team-bus.mjs:262`). VERIFIED.
- Dedupe/ACK: drain-by-rename is the only consumption record; no ACK, no redelivery if the model loses
  the result (compaction, crash after rename). VERIFIED for the code; consequence INFERRED.
- Identity: the caller passes `agent` and `team` as free strings; nothing binds them to a Pi session
  (`team-tools.ts:175-178`; Pi's `PI_SESSION_ID` is never read). VERIFIED.

### B. The bus CLI through Pi's `bash` tool (works, pull-only)

`grok-team-bus.mjs` has a `runCli` with `send`, `inbox`, `status`, `plan`, `on-stop`, ...
(`grok-team-bus.mjs:450-492`). It is copied only by `init --grok` (template path); a Pi model could call
`node <path>/grok-team-bus.mjs inbox --team T --agent A` through `bash`. Same semantics as A, minus the
MCP exposure issue. VERIFIED (CLI exists); usage on Pi INFERRED.

### C. Claude-only paths do not transfer

Agent Teams `SendMessage`, the teammate-plugin (`~/.claude/teams/...`), Claude mods
(`$.prompt.submit`) and the Claude hooks in `.claude/settings.json` have no Pi counterpart in ruflo
(`overview.md` sections 4 and 9). Pi has no hook-subprocess model: its equivalents (`before_agent_start`,
`tool_result`, `agent_before_settle`) exist only for in-process extensions (primer section 2.2). ruflo
ships no Pi extension. VERIFIED.

## Wake mechanism

**None.** VERIFIED by absence (no Pi code; the scaffolder registers tools only, `hpd:dist/index.js:35-56`).

Generic ruflo wake surfaces mapped onto Pi:

| ruflo surface | Pi applicability | Tag |
|---|---|---|
| Grok SubagentStop hook → `on-stop` plan-advance hint for the lead (`v3/@claude-flow/cli/templates/grok/hooks/subagent-stop-team.json`; `grok-team-bus.mjs:354-385`) | Pi has no SubagentStop event or native subagent tool in the primer's event table; nothing fires it. | INFERRED from primer section 2.2 |
| Headless relaunch per task (`claude -p` / `codex exec`, `v3/@claude-flow/codex/src/dual-mode/orchestrator.ts:203-241`) | No Pi command line in ruflo. A Pi equivalent (`pi -p --session-id <id> "..."`) would be a fresh process, and Pi has no session-file lock, so it is unsafe while an interactive Pi owns the session (primer section 4). | VERIFIED (absence in ruflo); Pi behaviour unknown |
| Console `/loop` scheduling (ADR-414) | Claude Code feature; no Pi equivalent wired. | VERIFIED (Claude-only) |
| tmux `send-keys` (ADR-442) | Only into ruflo's own shell sandboxes, never agent panes. | VERIFIED (`overview.md` section 4.8) |

- Idle/busy detection: none for Pi. Draft safety: not applicable (nothing types into Pi).
- Latency: an idle Pi only "receives" a ruflo message when a person prompts it and the model then
  happens to call `team_inbox`. Unbounded. INFERRED.
- Failure mode: silent. A message sits in `mailbox/<agent>/` until some turn pulls it. INFERRED.

What a working Pi wake would need (not in ruflo, for comparison): an in-process extension that watches
the store from `session_start` and calls `pi.sendMessage({...}, {triggerTurn: true})` when
`ctx.isIdle()` and `ctx.ui.getEditorText()` is empty, as the upstream `file-trigger.ts` example does
without the guards (primer sections 2.3-2.5, `pi:packages/coding-agent/examples/extensions/file-trigger.ts:14-41`).
INFERRED.

## Setup/installation footprint on the harness

- ruflo itself writes **nothing** into Pi config: no `~/.pi/agent/*`, no `.pi/`, no `mcp.json`.
  VERIFIED (grep for `.pi/` in the clone: no hits).
- `harness mint --host pi-dev` (via `metaharness new`) writes a **new directory** outside the calling
  repo, default `/tmp/ruflo-mint-<ts>-<name>/`, dry-run unless `--confirm`
  (`plugins/ruflo-metaharness/skills/harness-mint/SKILL.md:14-25`). VERIFIED. Its contents are the four
  files from `hpd:dist/index.js:100-109`, meant to be published as an npm package and installed with
  `pi install npm:<harness>` (`hpd:dist/index.js:30-34`). VERIFIED.
- A manual MCP hookup (section A) would add one entry to `~/.pi/agent/mcp.json` or project `.pi/mcp.json`
  (`pi:packages/coding-agent/docs/mcp.md:32` per primer). INFERRED (person's action, not ruflo's).

## Token cost per message and per quiet turn

- Per quiet turn on Pi: **zero** from ruflo; nothing is injected. With a manual MCP hookup at default
  `codemode` exposure: one `mcp_servers` system-prompt line for the server (`mcp.md:202`), cached.
  With `direct` exposure of all ~314 tools: their full schemas on every request, which would be very
  large (INFERRED; I did not measure ruflo's schema size).
- Per message: one tool call plus the pretty-printed JSON of every drained message (about 200 bytes of
  envelope plus body, INFERRED from the field list at `grok-team-bus.mjs:250-260`), plus the model's
  turn spent deciding to poll. No per-message framing to strip.

## Limitations and bugs you can see

1. **Scaffolder targets a Pi API that does not exist at the read commit.** VERIFIED against pi-mono
   `a7b94a03f`:
   - Pi loads an extension's **default export** as a factory and rejects anything else with
     "Extension does not export a valid factory function" (`pi:packages/coding-agent/src/core/extensions/loader.ts:584-592,647-650`).
     The generated file has only a named `export async function register(pi)` (`hpd:dist/index.js:53`).
   - Pi's `registerTool(tool: ToolDefinition)` takes **one** object with `name`, `label`,
     `description`, TypeBox `parameters` and `execute(...)`
     (`pi:packages/coding-agent/src/core/extensions/types.ts:577-595,650,1675-1677`). The generated code
     passes `{name, description, inputSchema}` plus a second handler argument.
   - `import type { PiContext } from 'pi-agent-sdk'`: no package or type by that name exists in pi-mono
     (grep: no hits). Type-only, so erased at runtime, but a sign the API was not checked.
   - `trust.json` with `{schema:1, trusted_extensions:[{name, source, allow, deny}]}`
     (`hpd:dist/index.js:85-98`) does not match Pi's trust store, which holds per-directory project-trust
     decisions `{path, decision}` (`pi:packages/coding-agent/src/core/trust-manager.ts:11-14,214`;
     `pi:packages/coding-agent/docs/security.md:55-70`). Pi has no per-extension trust or allow/deny
     list in this file. If someone copied the generated file to `~/.pi/agent/trust.json` it would at
     best be ignored and at worst replace the person's saved trust decisions (INFERRED; I did not read
     the trust store's parse-error handling).
   - "Pi explicitly ships with NO MCP" (`hpd:dist/index.js:20-27`, README) is stale: Pi now has a
     built-in MCP client (`pi:packages/coding-agent/docs/mcp.md:1-3`).
   INFERRED overall: a mint for `pi-dev` produces an extension Pi would refuse to load.
2. No push, no wake, no idle detection, no session identity for Pi (sections above). VERIFIED.
3. The ADR-402 bus has no ACK, no read-side bound (only a 500,000 send cap on the MCP path), and a
   drain is final once renamed (see section A).
   VERIFIED code; risk INFERRED.
4. Default `codemode` exposure in Pi hides ruflo's comms tools from the model unless configured
   (section A). VERIFIED (Pi docs).
5. ruflo's own research explicitly skipped Pi (`ADR-451-mod-capability-roadmap.md:246`). VERIFIED.

## Clever ideas worth stealing

Little that is Pi-specific. Generic points that would carry over to a herdr-threads Pi adapter
(all INFERRED):

- **Priority-in-filename + drain-by-rename** gives lock-free disjoint drains between two readers of one
  mailbox (`grok-team-bus.mjs:229,280-303`). Not better than our SQLite receipts, but a reminder that
  a Pi extension reading a local spool could claim items by rename if it ever needs a file fallback.
- **Declared no-op for unsupported capabilities**: the scaffolder writes "not projected on this host
  (documented no-op)" instead of silently dropping the `autonomous` block (`hpd:dist/index.js:62-68`).
  Matches our "degrade only the broken capability, say so" rule.
- **Ship the integration as a `pi install npm:...` package with a default-deny tool posture**
  (`hpd:dist/index.js:30-34,85-98`): the packaging idea is sound even though the trust file is wrong.
  For managed launch, `pi -e <ext>` attaches an extension without any config write (primer 2.1),
  which is the better option for us.
- Pi-side, the right design is the one ruflo did not build: an in-process extension using
  `before_agent_start` (turn-start delivery), `tool_result` or streaming `sendMessage` (in-turn), and
  `sendMessage(..., {triggerTurn:true})` gated on `ctx.isIdle()` plus an empty
  `ctx.ui.getEditorText()` (idle wake with draft safety, no keystrokes). See primer section 9.
- And, by counter-example: verify a harness's extension API against its source before shipping a
  generator for it. The scaffolder's four API mismatches are the cost of not doing so.

## Verification

Adversarial pass, 2026-10-23, against the ruflo clone at `e00a82f2f`, pi-mono at `a7b94a0`, and the
`@metaharness/host-pi-dev@0.1.3` tarball (read only).

Checked and holding: no `pi` init flag (`init.ts:1878,1884,1902`); the complete grep hit list (re-run:
only the five listed locations); mint passes `--host` through (`mint.mjs:35,40,119`); mint target
defaults and dry-run (`SKILL.md:14-25`); every `hpd:dist/index.js` range, the named-only `register`
export, `pi-agent-sdk` absent from pi-mono; Pi's default-export factory requirement (`loader.ts:584-592,647-650`),
single-argument `registerTool(ToolDefinition)` (`types.ts:577-595,1675-1677`), trust store
`{path, decision}` (`trust-manager.ts:11-14,214`), built-in MCP (`mcp.md:3`), codemode default exposure,
20 KB truncation (`mcp.md:232`), MCP client registering only `roots`/`ping` and routing
`notifications/message` to a log (`client.ts:172-182`, `runtime.ts:367`; it also handles
`tools/list_changed`, `runtime.ts:380`, which is not a model push); `file-trigger.ts:14-41`;
`isIdle`, `getEditorText`, `before_agent_start`, `tool_result`, `agent_before_settle`, `agent_settled`,
`sendMessage(..., {triggerTurn})` all exist in `types.ts`; mailbox naming, `wx`/`0o600`, drain-by-rename.

Changed: `team-tools.ts:75` (that is `team_create`) replaced by 185 (`team_broadcast`); "raw JSON array"
corrected to the `{success, ..., messages}` object wrapped at `mcp-server.ts:985`; "no body limit"
corrected (500,000 message / 1,000 summary cap on `team_send`, none on the CLI path or on reads);
framing citation 263 -> 262 and re-scoped to the on-disk format; `mcp.md` lines 193-196 -> 195,
205 -> 206, 200 -> 202; hpd file length 111 -> 110; SubagentStop row attributed to ruflo's Grok hook
template. Not checked: GitHub releases (no claim depends on them).
