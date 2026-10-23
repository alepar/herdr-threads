# ruflo x Goose (Block / AAIF): message injection and waking up

Upstream: https://github.com/ruvnet/ruflo, shallow clone at `e00a82f2fa2fa2413dd89592f04aae5d4ca35872`
(2026-10-23). Paths are relative to the clone root. Goose facts come from the primer
(`../../primers/goose.md`, cited as "primer §N"), which read `goose@1.36` (installed) and `goose@main` (1.56.0).
Tags: **VERIFIED** = read in source, docs or GitHub API at the cited place; **INFERRED** = my conclusion;
**unknown** = not determined.

## 1. Support status

**Not supported.** ruflo has no Goose adapter, init target, config writer, hook bundle, plugin or doc.

- `grep -rIil goose` over the whole clone (excluding `.git`) matches only `v3/package-lock.json` and
  `v3/pnpm-lock.yaml`, and every hit in both is the substring of `@opentelemetry/instrumentation-mongoose`
  (`v3/pnpm-lock.yaml:2487,2547,3801,3817`; `v3/package-lock.json:13012,14889,14891`). So the mapper's
  "appears only in a lockfile" is even weaker than stated: **no reference to Goose exists in the repository at
  all**. VERIFIED.
- No Goose config surface is referenced either: no `GOOSE_*` env var, no `.agents/plugins`, no
  `~/.config/goose` (grep). VERIFIED. The only AAIF mention is a generic "Follows AAIF and Open Agent Skills
  specifications" in `v3/implementation/adrs/ADR-027-codex-integration.md:347`. VERIFIED; it is about Codex.
- `init` targets are Claude (default), `--codex`, `--dual`, `--grok` (`v3/@claude-flow/cli/src/commands/init.ts:1878-1902,1948-1953`). VERIFIED.
- GitHub: the only issue mentioning Goose is #310 (open, 2025-07-15, 19 comments), a user's feasibility
  question listing "Goose (Block/Square)" among CLI agents ruflo could target
  (https://github.com/ruvnet/ruflo/issues/310). No release note in the last 30 releases mentions Goose
  (`gh api repos/ruvnet/ruflo/releases?per_page=30`). VERIFIED.

The only path by which ruflo can touch Goose is its **generic "any MCP client" route**: README "Using Codex
or another MCP enabled tool? ... configure command `npx` and arguments `["-y", "ruflo@latest", "mcp", "start"]`
... hook support depends on the client" (`README.md:40-53`). VERIFIED. Everything below evaluates that route
on Goose.

## 2. Injection mechanism

### 2.1 What ruflo would offer on Goose: pull through MCP tools

The relevant tools are the ADR-402 host-agnostic Agent Teams bus, nine MCP tools in
`v3/@claude-flow/cli/src/mcp-tools/team-tools.ts` (registered at `v3/@claude-flow/cli/src/mcp-client.ts:85-86,203-204`).
VERIFIED. Their own descriptions say they are for "hosts without a live message channel"
(`team-tools.ts`, `team_inbox` description, around line 170). VERIFIED.

- **Send**: `team_send {team, to|"*", message, summary?, from?, type?, priority 0-999 (default 2)}`
  -> `sendMessage` in `v3/@claude-flow/cli/templates/grok/scripts/grok-team-bus.mjs:231-273`: under a team lock,
  writes one file per recipient, `.claude-flow/teams/<team>/mailbox/<agent>/<ppp>_<msgid>.json`, flag `wx`,
  mode 0600. VERIFIED.

  ```js
  const messageFileName = (msg) => `${String(msg.priority).padStart(3, '0')}_${msg.id}.json`;
  ```
  (`grok-team-bus.mjs:229`)

- **Receive**: `team_inbox {team, agent, peek?}` -> `readInbox` (`grok-team-bus.mjs:282-306`): lists `*.json`,
  sorts lexically (priority prefix, then `msg_<ms>_<rand>`), and claims each by `rename` into `archive/`, so
  concurrent drains are disjoint. The tool result is the full JSON
  `{success, action:"inbox", team, agent, messages:[{id, teamId, from, to, summary, content, type, priority, timestamp}], peek}`.
  VERIFIED.
- **Project root** (where the mailbox lives): `CLAUDE_FLOW_CWD` if set and not `/` or home, else the MCP server's
  `process.cwd()` (`v3/@claude-flow/cli-core/src/mcp-tools/types.ts:38-59`, re-exported by `v3/@claude-flow/cli/src/mcp-tools/types.ts:8`; called from `team-tools.ts:57`). VERIFIED. Goose's stdio extension
  working directory is not covered by the primer; INFERRED it is Goose's launch directory, so a Goose session and
  a Claude session started in the same project dir share one mailbox. Unknown for sure.

### 2.2 How that lands in Goose's context

- **Exact mechanism**: a model-initiated MCP `tools/call` of `team_inbox`; the JSON result is the tool result in
  Goose's conversation. Nothing reaches the Goose model unless the Goose model itself decides to call the tool.
  INFERRED from 2.1 plus primer §6 (Goose is an ordinary MCP tool client).
- **When it fires**: only when the model calls it. The ADR-402 design relies on the spawn prompt telling the
  agent to "Read your inbox first: team_inbox (team=..., agent=...)" (`grok-team-bus.mjs:83-103`, `buildSpawnPlan`).
  That prompt is built for a Grok/Claude *subagent the lead spawned*; there is no ruflo step that would put it
  into a free-standing Goose session. On Goose the person would have to tell the model its team and agent name,
  or bake it into a `.goosehints`/recipe/`--system` text by hand. INFERRED.
- **No push**. ruflo's stdio MCP server handles the client's `notifications/initialized`
  (`v3/@claude-flow/cli/src/mcp-server.ts:1010`) and sends exactly one server-initiated frame: a non-standard
  `server.initialized` at startup advertising `tools.listChanged: true` (`mcp-server.ts:822-836`). No
  message-bearing notification exists (the only `method:` frame in the file is that one). VERIFIED. Even if it
  did, Goose shows MCP notifications only to the UI and only during that server's tool call (primer §6). VERIFIED
  (primer).
- **Hooks**: ruflo's injection hooks (UserPromptSubmit `hook-handler.cjs route` stdout, `swarm-hooks.sh` PreToolUse
  `additionalContext`) are Claude-format files in `.claude/settings.json` (overview §4). Goose reads hooks only
  from `.agents/plugins/<name>/hooks/hooks.json`, and has **no context-injection field on any hook event**
  (primer §3.1, §3.4). So even a hand-copied ruflo hook would inject nothing into Goose: Goose discards
  UserPromptSubmit output. VERIFIED (both sides); consequence INFERRED.
- **Not usable on Goose either**: `team_on_stop` is wired from a `SubagentStop` hook
  (`v3/@claude-flow/cli/templates/grok/hooks/subagent-stop-team.json:3-8`, command uses `$CLAUDE_PROJECT_DIR`);
  Goose 1.36 parses `SubagentStop` but never emits it and main removed it (primer §3.2). VERIFIED.
- **Size bounds**: input-side only. `team_send` rejects `message` > 500,000 chars, `summary` > 1,000, `type` > 64;
  `team_spawn` prompt > 100,000 (`team-tools.ts` handlers, `validateText(..., 500_000)` etc.). VERIFIED. There is
  **no output bound**: one `team_inbox` drain returns every pending message with full bodies in one tool result.
  VERIFIED (no limit in `readInbox`). Goose's own handling of an oversized tool result is outside this report
  (unknown).
- **Framing**: raw pretty-printed JSON envelope wrapped in `{success:true, ...}` (`team-tools.ts:57`;
  `JSON.stringify(result, null, 2)` as one text content block, `mcp-server.ts:985`); no header line, no trust
  marker. VERIFIED.
- **Dedupe / ACK**: drain-by-rename is the only "ack": once returned, a message is archived and never offered
  again, whether or not the model read or acted on it. No receipts, no re-offer, no sender-visible
  acknowledgement. `peek:true` returns without archiving (so repeated peeks repeat content). VERIFIED.

## 3. Wake mechanism

**None.** ruflo cannot make an idle Goose session act.

- No keystroke injection into agent panes as a delivery path (`tmux send-keys` appears only in the console's
  own `ruflo-sb-*` shell sandboxes, `v3/docs/adr/ADR-442-console-sandbox-page-and-plugins-in-tools.md:15`,
  `plugins/ruflo-console/hooks/sandbox.ts:58`, and in the console's e2e test driver dismissing Claude prompts,
  `plugins/ruflo-console/scripts/e2e-lib.sh:101-112`), no long-poll/blocking MCP
  tool in the team set, no Stop-hook block-and-continue, no ACP client. VERIFIED (overview §5, grep; the nine
  `team_*` handlers in `team-tools.ts` all return immediately).
- The headless-relaunch "wake" ruflo uses elsewhere is hard-coded to two CLIs: `executeHeadless` builds
  `claude -p <prompt> --output-format text` when `platform === 'claude'` and otherwise
  `codex exec --sandbox ... <prompt>` (`v3/@claude-flow/codex/src/dual-mode/orchestrator.ts:205-241`). VERIFIED.
  Pointing `codexCommand` at `goose` would pass `exec --sandbox`, which `goose` does not accept (primer §2 flag
  list). INFERRED.
- The ADR-402 "wake" is lead-side: `team_on_stop` returns a hint `Spawn or resume agent "<x>" for the next plan
  step` (`grok-team-bus.mjs:354-385`) for the lead model to act on. Goose has no spawn adapter in the spawn plan
  (`host` keys are grok/claude, `ADR-402...md:52-89`). VERIFIED.
- **Idle/busy detection, draft safety, latency**: not applicable; nothing is delivered to an idle session. Latency
  is "until the Goose model next chooses to call `team_inbox`", which for an idle session is never. INFERRED.
- **Failure modes**: a message to an idle Goose agent sits in the mailbox indefinitely; the sender gets
  `{success:true, action:"send", message, recipients}` (`grok-team-bus.mjs:268`) and no signal that nobody will read it. VERIFIED (send result shape) / INFERRED
  (consequence).

## 4. Setup / installation footprint on Goose

ruflo writes **nothing** for Goose. The person must add the extension by hand, e.g. `goose configure` ->
add stdio extension `npx -y ruflo@latest mcp start`, or per session
`goose session --with-extension 'npx -y ruflo@latest mcp start'` (primer §2 flags). INFERRED (no ruflo doc for
Goose). Running `npx ruflo init` in the project would still write the Claude-side files (`.claude/settings.json`,
`.claude/helpers/*`, `CLAUDE.md`) that Goose ignores; mailbox state appears under `.claude-flow/teams/` on first
`team_create`. VERIFIED (init targets, team bus paths); "Goose ignores them" INFERRED from primer §3.1.

Recommended narrowing: `mcp start --tools team` advertises only the team category
(`v3/@claude-flow/cli/src/commands/mcp.ts:93-96`; selector semantics in `mcp-server.ts:320-345`). VERIFIED.

## 5. Token cost

- **Fixed per request (tool schema)**: unfiltered, ruflo advertises its whole catalogue (README claims 314 tools).
  ruflo itself ships `assessMcpSchemaOverhead` (bytes/4 estimate, `mcp-server.ts:347-370`) because this is large;
  the actual figure for the full list was not measured (code not executed). With `--tools team` the nine schemas
  come from a 10,995-byte source file (`wc -c team-tools.ts`), so INFERRED roughly 1.5-2.5k tokens per provider
  call. This is paid on every Goose request, idle or not, unless Goose's tool router trims it (unknown).
- **Per quiet turn**: zero injection (nothing is pushed) plus the schema above; if the person instructs the model
  to poll `team_inbox` every turn, about 100-200 tokens per empty call+result. INFERRED.
- **Per message**: the full JSON envelope (~10 fields, pretty-printed) plus body, roughly 80-120 tokens of framing
  per message, plus the model's tool-call tokens. INFERRED from the envelope in `grok-team-bus.mjs:251-261`.

## 6. Limitations and bugs visible

1. No Goose support of any kind; the "lockfile mention" is `mongoose`. VERIFIED.
2. Pull-only, model-initiated delivery: a Goose agent never learns a message exists unless prompted to look.
   INFERRED.
3. No wake path; idle Goose sessions are unreachable. VERIFIED (absence).
4. Drain = consumed: a drain whose result the model ignores, truncates or loses to compaction loses the message
   permanently; no re-offer. VERIFIED (mechanism) / INFERRED (risk).
5. Unbounded drain output: 100 queued 500k-char messages are returned in one tool result. VERIFIED (no cap).
6. Identity is a free-text agent name the model passes as an argument; nothing binds it to Goose's
   `session_id`/`AGENT_SESSION_ID` (primer §11). Two Goose sessions using the same name steal each other's mail.
   VERIFIED (`safeName` only) / INFERRED (collision).
7. Mailbox location depends on the MCP server's cwd or `CLAUDE_FLOW_CWD`; a Goose launched in a subdirectory
   would use a different mailbox from its peers. INFERRED.
8. The comms-protocol prompt hard-codes Grok wording ("Grok Build 1.0.41 runs you as a general-purpose subagent";
   `grok-team-bus.mjs:92`), so even reused manually it misdescribes a Goose agent. VERIFIED.

## 7. Clever ideas worth stealing

Nothing Goose-specific (there is nothing). Generic, applicable to a herdr-threads Goose adapter:

- **`--tools <category>` schema trimming with a built-in overhead estimator** (`mcp-server.ts:320-370`): if we
  ever expose an MCP surface to Goose, advertise only the handful of tools needed and measure the fixed schema
  cost per request. VERIFIED feature.
- **Drain-by-rename for disjoint concurrent readers** (`grok-team-bus.mjs:272-306`): cheap and correct for a
  file-based fallback; our SQLite receipts are stronger, but the trick fits a MOIM-file staging step (write the
  pending page to a temp file, `rename` it into the `GOOSE_MOIM_MESSAGE_FILE` path atomically so Goose never
  reads a torn file; primer §5). INFERRED adaptation.
- **Priority in the filename** (`ppp_` prefix) so plain `ls | sort` is the delivery order. INFERRED as minor.

What ruflo's absence confirms for us (INFERRED): on Goose the real options are the ones the primer lists, none of
which ruflo uses: the `tom` MOIM file for in-turn delivery after every tool batch without a tool call (primer §5),
the main-only Stop block for turn-end delivery (primer §3.4), Native keystroke wake into the `> ` prompt with a
Goose composer reading (primer §9), and ACP `session/prompt` / `steer` for ACP-hosted Goose (primer §7). An
MCP pull tool in ruflo's style is the weakest of these: no wake, no in-turn push, and per-request schema cost.

## Verification

Checked against the clone at `e00a82f2`, the GitHub API and the Goose primer:

- Confirmed: no `goose` reference outside the two lockfiles' `instrumentation-mongoose` lines (cited lines
  exact); no `GOOSE_*` / `.agents/plugins` / `~/.config/goose`; init targets (`init.ts:1878-1902,1948-1954`);
  README MCP route (`README.md:40-53`); nine `team_*` tools, `category: 'team'`, "hosts without a live message
  channel" (`team-tools.ts:170`), size limits (`team-tools.ts:121,152-154`), priority 0-999 default 2
  (`team-tools.ts:146`, `parsePriority` `grok-team-bus.mjs:211`); `sendMessage`/filename/`wx`/0600
  (`grok-team-bus.mjs:229-270`); drain-by-rename with no output cap; spawn protocol text incl. Grok 1.0.41 line
  (`:83-103`, `:92`); `onStop` hint (`:354-385`); `SubagentStop` template; `executeHeadless` claude/codex only
  (`orchestrator.ts:205-241`); `--tools` selector and `assessMcpSchemaOverhead` (`mcp-server.ts:320-370`,
  `mcp.ts:93-96`); ADR-402 host keys grok/claude; issue #310 (open, 2025-07-15, 19 comments) is the only
  Goose hit in issue search; 0 of the last 30 release notes mention Goose; primer claims on hooks location,
  no injection field, SubagentStop, notifications UI-only, no `goose exec`.
- Fixed: the server does send one server-initiated frame (`server.initialized`, non-standard) at startup, so
  "sends no server-initiated notifications" was wrong; AAIF citation was `:347,410` (only `:347` mentions
  AAIF); `getProjectCwd` range and its re-export; `readInbox` line ranges; `tmux send-keys` also appears in
  the console sandbox hook and e2e test driver (still not a delivery path); send result shape.
