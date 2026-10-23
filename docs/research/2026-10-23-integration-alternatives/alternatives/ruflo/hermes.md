# ruflo x Hermes Agent (Nous Research)

- Alternative: ruflo (formerly Claude Flow), https://github.com/ruvnet/ruflo. I read a shallow clone at `e00a82f2fa2fa2413dd89592f04aae5d4ca35872` (2026-10-23). Paths are relative to the clone root. Nothing in it was executed.
- Harness reference: the primer `../../primers/hermes.md` (Hermes Agent `main` `f3b5b980`, latest release v0.22.5). I did not reread the Hermes clone for this report. Hermes facts below cite the primer section they come from.
- Companion: `overview.md` in this directory (ruflo's architecture, comms paths and delivery guarantees).
- Tags: **VERIFIED** means read in source, docs or the GitHub API. **INFERRED** means reasoned from those and not observed. **unknown** means not found.

## Support status

**ruflo does not support Hermes Agent as a host.** It has no Hermes adapter, init flag, plugin, hook bundle, MCP registration recipe or worker launcher. In ruflo, Hermes is a competitor to learn from, not a harness to integrate with.

Evidence:
- `grep -rniI hermes` over the clone (excluding lockfiles) gives these hits, none of them an integration. VERIFIED:
  - Borrowed design patterns: `v3/@claude-flow/providers/src/anthropic-provider.ts:330` ("Prompt caching (hermes-agent pattern)"), `v3/@claude-flow/cli/src/mcp-tools/agent-execute-core.ts:276` (same), `v3/@claude-flow/cli/src/mcp-tools/tool-loop-guardrail.ts:2` ("hermes-agent tool_guardrails pattern"), `v3/@claude-flow/cli/src/mcp-tools/hooks-tools.ts:42` ("think_scrubber pattern"), and the tests for these in `v3/@claude-flow/cli/__tests__/hermes-tier1.test.ts:1-8`. These change ruflo's own LLM calls and its learning trajectory. They do not touch Hermes.
  - The host string `hermes` as an example: `v3/@claude-flow/cli/src/mcp-tools/metaharness-tools.ts:569` (`'Target host harness (e.g. claude-code, codex, pi-dev, hermes)'`, the `metaharness_learn` tool) and `plugins/ruflo-metaharness/skills/harness-mint/SKILL.md:47` (the host list for `metaharness new`). Both pass `--host` through to the external, optional `metaharness` npm package (`v3/@claude-flow/cli/package.json:149,158-160`, an optional peer dependency; `plugins/ruflo-metaharness/scripts/mint.mjs:119`). That package scaffolds a *new* agent harness for a host. It is not a messaging integration. Its `@metaharness/hermes` and `@metaharness/host-hermes` packages exist on npm (download counts in `data/npm-downloads.latest.json:891` and `:945`), but it is not in this repo, and I did not read it (unknown contents).
  - Hermes model ids in the HuggingChat UI config (`ruflo/src/ruvocal/chart/env/prod.yaml:187-249`, `routes.chat.json:173-190`). These are models, not the harness.
  - Strategy documents: `docs/IMPROVEMENT-ROADMAP.md:64` ("Hermes Agent ... builds shareable skills").
- Init flags: the only alternative hosts are `--codex` and `--grok` (`v3/@claude-flow/cli/src/commands/init.ts:1878-1888`). VERIFIED.
- The team bus labels its hosts "grok, claude, codex" (`v3/@claude-flow/cli/src/mcp-tools/team-tools.ts:86`). The spawn plan emits host blocks only for `grok` and `claude` (`v3/@claude-flow/cli/templates/grok/scripts/grok-team-bus.mjs:111-127`). VERIFIED.
- The headless worker launcher knows only `claude` and `codex` (`v3/@claude-flow/codex/src/dual-mode/orchestrator.ts:207`, `config.platform === 'claude' ? claudeCommand : codexCommand`). VERIFIED.
- GitHub (`gh api search/issues`, 2026-10-23): 12 issues mention Hermes. The relevant ones are #1907, "ADR-113: strategic capability gaps vs. Nous Research Hermes Agent" (open, 2026-05-11), which lists R-1...R-11 (skill synthesis, an agentskills.io interop, a messaging-gateway plugin for Telegram, Discord and Slack, and others), and #2237, "Hermes-Agent Tier-1 adoptions — prompt caching, reasoning scrub, tool-loop breaker" (closed). None of them proposes running ruflo inside Hermes. The ADR-113 file sits on a branch and is not in the clone at HEAD (`v3/docs/adr/` has no ADR-113). VERIFIED.
- Releases: no release body in the last 100 releases matches `hermes` (`gh api repos/ruvnet/ruflo/releases?per_page=100`, filtered with jq). VERIFIED.

What a Hermes user could still do: ruflo's README offers a generic path for "your client" (`README.md:50-53`: `npx -y ruflo@latest mcp start`; "hook support depends on the client"). Hermes is an MCP client (primer section 8). The rest of this report describes that generic path on Hermes and maps ruflo's mechanisms onto Hermes surfaces. Unless a statement is tagged otherwise, it is INFERRED from the two sources and was not observed.

## Injection mechanism

### The generic path: the ruflo MCP server's `team_*` tools, pulled by the model

- **Exact mechanism.** The operator would register the ruflo stdio MCP server in Hermes' `config.yaml` `mcp_servers:` (the key is VERIFIED in primer section 8; the exact entry ruflo would need is not documented anywhere in ruflo). Hermes would then expose ruflo's tools as an `mcp-<server>` toolset (primer section 8, VERIFIED). The messaging pair is `team_send` and `team_inbox` (`v3/@claude-flow/cli/src/mcp-tools/team-tools.ts:133,168`, VERIFIED). `team_inbox`'s own description names the role: "the Grok/Codex-side counterpart to team_send for hosts without a live message channel" (`team-tools.ts:170`, VERIFIED). Hermes would be one of those hosts.
- **Code path.** `team_inbox` → `run('readInbox', ...)` (`team-tools.ts:182-183`) → `bus.readInbox(getProjectCwd(), opts)` (`team-tools.ts:57`) → `readInbox` in `templates/grok/scripts/grok-team-bus.mjs:282-306`. VERIFIED. The drain (condensed from `:290-304`):

  ```js
  for (const f of files) {
    let file = path.join(dir, f);
    if (!peek) {
      const archived = path.join(dir, 'archive', f);
      try { fs.renameSync(file, archived); }
      catch (err) { if (err.code === 'ENOENT') continue; throw err; } // another reader claimed it
      file = archived;
    }
    const msg = readJson(file);
    if (msg) messages.push(msg);
  }
  ```
- **When it fires.** Only when the model calls `team_inbox`. Nothing calls it at turn start or after a tool call. The trigger is the spawn-plan "comms protocol" prompt: ``Read your inbox first: team_inbox (team=${t.name}, agent=${agent}).`` (`grok-team-bus.mjs:95`, VERIFIED). That prompt reaches only an agent that ruflo spawned through a spawn plan. A long-lived Hermes session in a pane gets it only if a person pastes it in or adds it to a skill or the system prompt. Hermes has no MCP push into context: `resources/list_changed` and `prompts/list_changed` are ignored, and `notifications/message` goes to `agent.log` (primer section 8, VERIFIED). So the pull model is the only option on this path, and ruflo does not claim otherwise.
- **Payload format.** The tool result is `{"success":true,"action":"inbox","team","agent","messages":[...],"peek"}`. Each message is the stored JSON `{id: "msg_<ms>_<rand8>", teamId, from, to, summary, content, type, priority, timestamp}` (`grok-team-bus.mjs:247-257,305`; wrapper `team-tools.ts:57`). VERIFIED. There is no header line or rendering: the model reads raw JSON.
- **Ordering.** The filename is `<priority padded to 3 digits>_<id>.json` (`grok-team-bus.mjs:229`), and `pendingFiles` sorts it lexically (`:274`). The order is therefore priority 0-999 ascending, then send-millisecond ascending. VERIFIED. Two messages in the same millisecond order by their random suffix (INFERRED).
- **Size bounds.** On send, `message` may be up to 500,000 chars and `summary` up to 1,000 (`team-tools.ts:152-153`, VERIFIED). On read there is no cap: one `team_inbox` call drains *every* pending file into one tool result (`grok-team-bus.mjs:288-304`, VERIFIED). Hermes' own handling of very large MCP tool results is unknown here (the primer documents spill for `pre_llm_call` output and the terminal tool, not for MCP results).
- **Framing.** None beyond JSON keys. The sender identity is the self-declared `from` string (default `lead`, `:234`). VERIFIED.
- **Dedupe and ack.** Drain-by-rename gives at-most-once delivery across concurrent readers: each file goes to exactly one drainer (`:278-281` comment and `:294-298`, VERIFIED). There is no ACK and no receipt. A drained message is consumed whether or not the model actually read it (INFERRED). `peek: true` reads without draining (`team-tools.ts:177`).

### Where the mailbox lives on Hermes (a likely breakage)

- `getProjectCwd()` returns `CLAUDE_FLOW_CWD` if set and not a "non-project dir", else `process.cwd()` (`v3/@claude-flow/cli-core/src/mcp-tools/types.ts:53-59`, re-exported by `v3/@claude-flow/cli/src/mcp-tools/types.ts`, which `team-tools.ts:16` imports; VERIFIED). The store is `.claude-flow/teams/<team>/...` under that root (overview section 2, VERIFIED).
- On Hermes 0.22.1 and later, MCP servers are owned by the gateway session that runs the turn, and "`os.getcwd()` is the gateway's own directory" for plugins (primer section 1, VERIFIED for plugins). Whether Hermes starts a stdio MCP child in the session's working directory is unknown. If it does not, every Hermes session in every pane shares one mailbox tree in the gateway's directory, and messages for project A and project B mix (INFERRED). `CLAUDE_FLOW_CWD` in the server's `env:` would pin one root per server entry, not per session (INFERRED).
- On the other hand, identity is fully explicit. The agent name and team are tool arguments the model supplies. Nothing depends on the pane's environment, so the `HERDR_PANE_ID`-inheritance problem in primer section 12.1 does not apply (INFERRED). The price is that identity is whatever the model remembers or claims: `from` is unchecked.

### ruflo's other injection paths, mapped onto Hermes

| ruflo path (overview section 4) | On Hermes | Tag |
|---|---|---|
| Claude Code Agent Teams `SendMessage` (ruflo's primary path on Claude) | Not present. Hermes has no SendMessage. Its nearest equivalents are `ctx.inject_message` (plugin) and gateway `prompt.submit` / `session.steer` (primer sections 4 and 5.1). ruflo ships neither. | VERIFIED absence in ruflo; mapping INFERRED |
| Spawn-plan prompt (handoff carried in the child's first prompt) | Would work only for a child Hermes started by a lead. No spawn-plan host block exists for Hermes (`grok-team-bus.mjs:111-127`), and the delegation tool `delegate_task` takes its own goal and context (primer section 2.3), not ruflo's plan. | VERIFIED (ruflo); INFERRED (fit) |
| `hook-handler.cjs route` on UserPromptSubmit (router choice and memory recall printed to stdout, `.claude/helpers/hook-handler.cjs:791-831`) | Hermes' equivalent is a `pre_llm_call` shell hook that returns `{"context": ...}` on stdout (primer section 3). ruflo's route prints plain text, not that JSON. Whether Hermes accepts non-JSON stdout as context is unknown. Either way, it carries routing and memory, not peer messages. | VERIFIED (ruflo output shape); fit unknown |
| `swarm-hooks.sh pre-task` (counts only, as PreToolUse `additionalContext`) | Hermes `pre_tool_call` returns only block, approve or modify. Only a block's message reaches the model (as the tool error); an allowed call has no context channel like `additionalContext` (primer sections 2.2 and 3). The path is not even wired in ruflo (overview section 4.4). | VERIFIED |
| SubagentStop → `team_on_stop` hint ("Spawn or resume agent X", `grok-team-bus.mjs:354-385`) | Hermes `subagent_stop` is an observer hook. Its return does not reach the parent model (primer section 2.3, which lists it among the observers). The hint would be lost. | INFERRED |
| Mod `$.prompt.submit` / `$.prompt.fill` | Claude Code mods only. There is no Hermes analog in ruflo. | VERIFIED |

## Wake mechanism

- **Exact mechanism on Hermes: none.** ruflo has no keystroke injection into agent panes, no long-poll or blocking-wait tool, no app-server or gateway RPC client, and no Stop-hook loop (overview section 5, VERIFIED by grep). The `team_*` tool set has no wait operation (`team-tools.ts:73-274`: create, spawn, send, inbox, broadcast, plan, status, on_stop and shutdown; VERIFIED). An idle Hermes session waiting at an empty prompt stays idle until a person types. A message sits in `mailbox/<agent>/` until the model next decides to call `team_inbox`.
- **ruflo's general approach** is to avoid waking altogether. A lead session spawns children (native subagents or headless `claude -p` / `codex exec`), and each "wake" is a fresh process (`orchestrator.ts:203-241`, VERIFIED). The Hermes analogs would be `hermes -z '…'` or `hermes chat -q '…'` through the gateway (primer section 5.4). ruflo does not launch Hermes (`orchestrator.ts:207`, VERIFIED). Even if it did, that would start a new or one-shot session, not wake the pane's live one (INFERRED).
- **Idle/busy detection.** Not applicable: nothing pushes into a session.
- **Draft safety.** Trivially safe, because nothing types into the composer. That safety is a side effect of having no wake, not a design (INFERRED).
- **Latency.** Unbounded. It equals the time until the model next calls `team_inbox` (INFERRED).
- **Failure modes (INFERRED).** (1) An idle recipient never learns about mail. (2) A recipient that never ran ruflo's protocol prompt never polls. (3) With a shared gateway cwd, polls may read the wrong project's mailbox, or a team created in one project is "not found" in another (`loadTeam` throws "not found — run create first", called at `grok-team-bus.mjs:284`, defined in `templates/grok/scripts/grok-team-store.mjs:266-270`).
- **What a ruflo-style user would bolt on (not in ruflo).** Hermes offers native wake primitives that ruflo could use and does not: `terminal(background=true, notify_on_complete=true)` around a blocking wait (primer section 7; ruflo has no blocking CLI, though `node scripts/grok-team-bus.mjs inbox` exists as a non-blocking CLI copy, `grok-team-bus.mjs:96-97`), `ctx.inject_message(session_key=…)` from a Hermes plugin, and gateway `prompt.submit` (primer sections 4 and 5.1). All INFERRED as possibilities.

## Setup/installation footprint on the harness

- **Shipped by ruflo for Hermes: nothing.** No writes to `~/.hermes`, no `plugin.yaml`, no `config.yaml` edit (VERIFIED: no init flag or generator targets Hermes, `init.ts:1878-1888`, and `v3/@claude-flow/cli/src/init/` has generators only for Claude settings, CLAUDE.md, MCP, helpers, statusline and Grok).
- **Manual generic setup (INFERRED).** One `mcp_servers:` entry in the Hermes profile's `config.yaml` that runs `npx -y ruflo@latest mcp start`, optionally with `CLAUDE_FLOW_CWD` in its `env`. The cost is an `npx @latest` fetch at server start and the full ruflo MCP tool surface (the README claims 314 tools, overview section 1). Whether Hermes can filter which tools of a server are registered is not covered in the primer (unknown).
- **Project-side writes by ruflo itself (VERIFIED).** `.claude-flow/teams/<team>/team.json` with a lock file, and `mailbox/<agent>/NNN_<id>.json` files created with `flag: 'wx'` and mode 0600 (`grok-team-bus.mjs:258-266`), under the resolved project root.

## Token cost per message and per quiet turn

- **Per quiet turn (INFERRED).** No injected bytes, because nothing is pushed. The standing cost is the MCP tool schemas in every request: ruflo registers its whole surface, a few hundred tools by its own count. That is likely tens of thousands of tokens per request before prompt caching, which is far more than any per-turn check-in. A quiet *poll* turn costs one tool call plus about 70 bytes of result plus the team and agent names (`{"success":true,"action":"inbox","team":…,"agent":…,"messages":[],"peek":false}`) and the model's reasoning around it.
- **Per message (INFERRED from the payload shape).** About 200 to 250 bytes of JSON overhead per message (id, teamId, from, to, summary, type, priority, timestamp, and the wrapper), plus the body and summary. Bodies arrive in full (up to 500,000 chars accepted on send) with no truncation or continuation, so one large handoff can flood the context.
- **Protocol prompt.** About 600 bytes per spawned agent, once, in the spawn prompt (`grok-team-bus.mjs:90-104`). It is Grok-specific text ("Grok Build 1.0.41 runs you as a general-purpose subagent", `:92`) that would mislead a Hermes model if reused verbatim. VERIFIED (text); size INFERRED.

## Limitations and bugs you can see

1. **No Hermes support, and no wake on any host for a live peer.** Covered above. VERIFIED.
2. **Drain-on-read without ACK.** The rename happens before the model sees the result. If the MCP call's result is truncated, dropped by a Hermes MCP timeout or reconnect, or lost to compaction, the message is already in `archive/` and is never redelivered (`grok-team-bus.mjs:294-303`). Recovery is a manual read of `archive/`. INFERRED.
3. **Unbounded drain.** One call returns every pending message (`:288-304`). There is no page size and no continuation. INFERRED risk on large backlogs.
4. **Corrupt files are silently skipped after being archived.** `readJson` returns null on any read or parse error (`templates/grok/scripts/grok-team-store.mjs:102-108`), so `msg` is falsy and the file stays in `archive/` (`grok-team-bus.mjs:302-303`). VERIFIED that nothing reports it.
5. **cwd-rooted store vs Hermes' gateway process model.** See "Where the mailbox lives". INFERRED, needs a live check.
6. **Self-declared sender.** `from` defaults to `lead`, and any caller can pass any name (`:234`). Fine under a cooperative model, but there is no attribution evidence at all. VERIFIED.
7. **The protocol prompt is hard-wired to Grok** (`:92-94`). Reusing it on another host is wrong text. VERIFIED.
8. **Subagent-stop hint loop does not port.** On Hermes the `subagent_stop` return is not model-visible, so the lead never sees "Spawn or resume agent X" (primer section 2.3). INFERRED.

## Clever ideas worth stealing

- **Atomic claim by rename** (`grok-team-bus.mjs:278-298`). Concurrent drainers get disjoint sets, and `ENOENT` means "someone else got it". It is a cheap, lock-free at-most-once claim on a plain filesystem. We already have SQLite with receipts, which is stronger, but the pattern fits a fallback file spool, such as a hookless client dropping items for the daemon.
- **Priority in the filename** (`:229`, `:274`). A plain lexical `readdir` sort gives priority-then-age order with no index. The same idea as our `DeliveryOrderKey`, at zero cost.
- **Defensive file creation**: `flag: 'wx'` (never write through a pre-existing file or symlink), mode 0600, and `ensureRealDir` checks (`:258-266`). Worth mirroring in any file our Hermes bridge or setup writes.
- **Identity as an explicit tool argument** rather than ambient env. On Hermes 0.22.1 and later, ambient pane env is exactly what breaks (primer section 12.1). ruflo's pattern of carrying identity in the call ("who am I" passed by the caller) is a fallback shape for us: the bridge could pass the session's `HERMES_SESSION_ID` plus a pane witness explicitly, instead of relying on an inherited `HERDR_PANE_ID`. This is INFERRED applicability. We must still record that claim honestly under TRUST-POLICY.md, not trust it as authority.
- **`peek` vs drain** as separate operations (`team-tools.ts:177`). It gives a read-only look for subagents. We already make subagent inbox reads read-only; this confirms the shape.
- Not worth stealing: ruflo takes nothing from Hermes' comms surfaces. Its Hermes "adoptions" are prompt caching, a reasoning scrub and a tool-loop breaker (#2237). For our Hermes integration, the useful native primitives (`inject_message`, gateway `prompt.submit` / `session.steer`, `transform_tool_result`) come from the primer, not from ruflo.

## Verification

Adversarial pass, 2026-10-23, against the clone at `e00a82f2` (HEAD confirmed), the GitHub API and the Hermes primer.

- Checked and upheld: every `hermes` grep hit (a fresh `grep -rliI` finds no integration; extra hits are `dev.yaml` model ids and substring false positives); init flags `--codex` / `--grok` only (`init.ts:1878-1888`); team-bus host label and spawn-plan host blocks (`grok` and `claude` only); `orchestrator.ts:207` platform switch; `team_send` / `team_inbox` definitions, descriptions and the 500,000 / 1,000 caps; the `readInbox` rename drain, filename priority sort, message shape, `from` default `lead`, `wx` + 0600 writes; the Grok-specific protocol prompt (`:91-98`); `team_on_stop` hint text; `hook-handler.cjs route` plain-text stdout on UserPromptSubmit (`.claude/settings.json:69-74`); `swarm-hooks.sh` PreToolUse `additionalContext` (`:627`); README generic MCP path (`README.md:50-53`); issues #1907 (open, 2026-05-11, R-2 gateway and R-4 agentskills.io present) and #2237 (closed); 12 issues match `hermes`; 0 of the last 100 release bodies match; no ADR-113 file under `v3/docs/adr/`. Primer facts cited (sections 1, 2.2, 2.3, 3, 4, 5.1, 5.4, 7, 8, 12 item 1) match the primer text.
- Changed: npm citation split into `@metaharness/hermes` (`:891`) and `@metaharness/host-hermes` (`:945`); `getProjectCwd` path noted as re-exported through `cli/src/mcp-tools/types.ts`; `readJson`-returns-null upgraded from INFERRED to VERIFIED (`grok-team-store.mjs:102-108`) and `loadTeam`'s definition cited; `team-tools.ts` range corrected to `:73-274`; `pre_tool_call` row corrected (a block message does reach the model); `src/init/` path made full and CLAUDE.md generator added; quiet-poll result size corrected from about 60 to about 70 bytes plus names; the drain snippet marked as condensed.
