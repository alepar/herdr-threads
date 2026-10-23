# ruflo (formerly Claude Flow) — overview

Upstream: https://github.com/ruvnet/ruflo (README badges still point at `ruvnet/claude-flow`; the npm package is `ruflo` / `claude-flow`).
Clone read: shallow, HEAD `e00a82f2fa2fa2413dd89592f04aae5d4ca35872` (2026-10-23 15:04 -0400). All paths below are relative to the clone root.
Evidence tags: **VERIFIED** = read in source or docs at the cited location; **INFERRED** = my conclusion from what was read; **unknown** = not found.

## 1. What it is

A very large "agent meta-harness" around Claude Code and Codex: an npm CLI plus an MCP server (README claims 314 MCP tools, 26 CLI commands), a Claude Code plugin marketplace (`plugins/ruflo-*`, about 60 plugins), Claude Code "mods" (function hooks, `$.prompt.submit`, panes), a TUI console, vector memory (AgentDB/RuVector), swarm "coordinators", and a cross-machine "federation" plugin. README: "An agent meta-harness for Claude Code and Codex" (`README.md:5`), "Ruflo is the harness — the execution layer around Claude Code and Codex" (`README.md`, "Agent = Model + Harness" paragraph). VERIFIED.

It is **not primarily a peer-messaging system between long-lived interactive sessions**. Its multi-agent story is: one lead interactive session spawns child agents (Claude Code `Task`/Agent Teams `SendMessage`, Grok `spawn_subagent`, or headless `claude -p` / `codex exec` workers) and coordinates them. INFERRED from the sections below.

## 2. Architecture

| Aspect | Finding | Tag / evidence |
|---|---|---|
| Process model | No resident message daemon for agent comms. MCP server started per client (`npx -y ruflo@latest mcp start`, `README.md` "Get started"). Hook commands spawn `node .claude/helpers/hook-handler.cjs <sub>` per event (`.claude/settings.json:69-153`). A `daemon-manager.sh` exists for background workers, not for message delivery. | VERIFIED (settings, README); "no comms daemon" INFERRED |
| Storage, Agent Teams bus (ADR-402) | Project-local files: `.claude-flow/teams/{teamId}/team.json` + `mailbox/{agent}/NNN_<msgid>.json` (zero-padded priority prefix), drained by rename into `mailbox/{agent}/archive/`. `team.json` writes take an O_EXCL lock file. | VERIFIED `v3/docs/adr/ADR-402-host-agnostic-agent-teams.md:78-82,146-152`; `v3/@claude-flow/cli/templates/grok/scripts/grok-team-bus.mjs:229-306` |
| Storage, teammate-plugin (Claude-bound) | One JSON array per teammate at `~/.claude/teams/<team>/mailbox/<teammateId>.json`, read-modify-write, mode 0600, oldest 10% dropped at `maxMessages`; read clears the file. | VERIFIED `v3/plugins/teammate-plugin/src/teammate-bridge.ts:286,2143-2180,1173-1199` |
| Storage, legacy swarm shell helpers | `.claude-flow/swarm/{queue,mailbox,messages}/` JSON files; `swarm-comms.sh` priority queue 0-3, `swarm-hooks.sh` message read with `.read=true` marking. | VERIFIED `.claude/helpers/swarm-comms.sh:9-75`, `.claude/helpers/swarm-hooks.sh:19-160` |
| In-process bus | `v3/@claude-flow/swarm/src/message-bus.ts`: EventEmitter + priority deques, `ackTimeoutMs` 5000, bounded retries with linear backoff. In-memory only; serves the swarm coordinator's internal agent records, not harness sessions. | VERIFIED `message-bus.ts:145-257`; scope INFERRED |
| Transport (local) | MCP tool calls (`team_send`, `team_inbox`, ...) and a CLI copy of the same store (`grok-team-bus.mjs` `runCli`). | VERIFIED `v3/@claude-flow/cli/src/mcp-tools/team-tools.ts:75,133,170` |
| Cross-machine | `@claude-flow/plugin-agent-federation`: Ed25519-signed envelopes over WSS (default port pattern `ws://host:9100`), five-level trust ladder, per-peer budget/circuit breaker, PII scrubbing, optional WireGuard mesh. Inbound envelopes are verified then emitted as `federation:inbound:<type>` events on the plugin event bus; no app-level ACK by design. | VERIFIED `docs/federation/README.md:1-80`; `v3/docs/adr/ADR-109-receive-side-dispatch.md:25-44,132-136`; `v3/@claude-flow/plugin-agent-federation/src/application/inbound-dispatcher.ts:29,469` |
| Cross-machine "talk to a peer agent" | Console target `@peer-<host>` = federation helper `dispatch`, i.e. `claude -p` over ssh on the peer; reply is the peer's stdout. BBS rooms (`federation_bbs_publish` / `_watch`) and an X relay channel are polled. | VERIFIED `v3/docs/adr/ADR-465-interactive-control-plane-and-multi-model-bridge.md:54-62` |

No subscriber in non-test source turns a `federation:inbound:*` event into model context (grep over `*.ts` found only the dispatcher and its tests). VERIFIED (absence within the grep); consequence "federation does not deliver into a live session" INFERRED.

## 3. Identity and session model

- ADR-402 bus: an agent is a **name unique within a team** (`<role>:<agent>@<team>` spawn description, parsed by the SubagentStop hook). Identity is a naming convention passed through the spawn prompt, not a binding to a harness session. VERIFIED `ADR-402...md:81,151`; `templates/grok/scripts/grok-subagent-stop-hook.mjs:8-14,52-79`.
- teammate-plugin: ids generated per spawn, team name from `CLAUDE_CODE_TEAM_NAME`. VERIFIED `teammate-bridge.ts:1053-1060`.
- Legacy `swarm-hooks.sh`: `AGENT_ID="${AGENTIC_FLOW_AGENT_ID:-agent_$(date +%s)_<random>}"` — a fresh random id per hook invocation unless the env var is set, so directed messages to an id cannot be matched by a later invocation. VERIFIED `.claude/helpers/swarm-hooks.sh:27-29`; consequence INFERRED.
- Session browser (console, ADR-486): identity = `harness + canonical home + native id` from transcript filenames (`<config>/projects/<folder>/<id>.jsonl`, Codex `rollout-*-<id>.jsonl`); ambiguous rows are "unassigned"; all adapters declare `messaging: no`, `resume: no` ("browsing must never start or wake one"). VERIFIED `v3/docs/adr/ADR-486-session-workspace.md:32-50`.
- Federation: node identity is an Ed25519 keypair in `.claude-flow/federation/keys-<nodeId>.json`. VERIFIED `docs/federation/README.md` Quick start step 2.

## 4. Injection mechanisms (how a message gets into a model's context)

1. **Claude Code native Agent Teams `SendMessage`** (primary on Claude). Ruflo's CLAUDE.md doctrine: "Communication happens via `SendMessage`, not polling or shared memory"; named background agents (`run_in_background: true`) wait for each other's SendMessage. Delivery is Claude Code's own (ADR-465 measured "Message queued for delivery ... at its next tool round"). Ruflo adds nothing to the delivery path. VERIFIED `CLAUDE.md:570-626`; `ADR-465...md:20`.
2. **Pull via MCP tool** `team_inbox` (drain or peek) on hosts without a live channel (Grok, Codex): the model must call it. Prompt from `team_spawn` embeds the "comms protocol". VERIFIED `team-tools.ts:170`; `ADR-402...md:38-59`.
3. **Spawn prompt** carries the handoff (spawn plan `prompt`, optional Grok `resume_from` for large payloads). VERIFIED `ADR-402...md:52-89`.
4. **Hook `additionalContext`, counts only**: `swarm-hooks.sh pre-task` emits a PreToolUse `additionalContext` with "Pending handoffs: N / New messages: N" (no bodies). Not referenced by any shipped hooks JSON in the clone. VERIFIED `.claude/helpers/swarm-hooks.sh:608-631`; "not wired" VERIFIED by grep (no `swarm-hooks` reference in any `.json/.ts/.cjs/.mjs`).
5. **UserPromptSubmit routing context**: `hook-handler.cjs route` prints router choice and memory recall to stdout on every prompt (Claude adds stdout as context). This is task routing and memory, not peer messages. On Codex the same pattern via `hookSpecificOutput.additionalContext` was proven live in 8 of 9 runs (ADR-490 matrix) but the Codex adapter is "Proposed / not built". VERIFIED `.claude/helpers/hook-handler.cjs:791-831`; `v3/docs/adr/ADR-491-ruflo-mods-host-integration-claude-and-codex.md:3,32-34`.
6. **Coordinator-polled guidance inbox** (proposed): `swarm guide <id>` appends to `.swarm/sessions/<id>/inbox.jsonl`; the swarm coordinator polls between turns and injects into the next worker prompt. VERIFIED as design `v3/docs/adr/ADR-385-streaming-interactive-long-horizon-swarm-execution.md:34,53`; implementation status unknown.
7. **Mod `$.prompt.submit` / `$.prompt.fill`** (ruflo-console): submits or fills the *own* session's prompt, person-triggered (`@claude` target, `/loop` launcher), from a clock tick. VERIFIED `plugins/ruflo-console/hooks/register.ts:137-141`; `ADR-465...md:56`; `v3/docs/adr/ADR-414-console-loop-manager.md:22`.
8. **tmux send-keys**, only into console-owned throwaway shell sandboxes `ruflo-sb-*` (shell commands, not agents): `send-keys -t <t> -l <text> ; send-keys -t <t> Enter`. VERIFIED `v3/docs/adr/ADR-442-console-sandbox-page-and-plugins-in-tools.md:15`.

Bounding/framing/dedup: ADR-402 bus has priority 0-999 in the filename (lower read first, lexical sort), drain-by-rename gives disjoint concurrent drains, no body size limit found (unknown). teammate-plugin drops oldest messages silently at `maxMessages`. `in-memory-inbox-reference.ts` in `@claude-flow/codex` models unacked retention, monotonic cursor resume, exact-retry dedup and issuer-id quarantine, but is explicitly "non-durable ... never an enforce-mode transport", a conformance reference only. VERIFIED `grok-team-bus.mjs:211-229,278-306`; `teammate-bridge.ts:2167-2171`; `v3/@claude-flow/codex/src/harness/in-memory-inbox-reference.ts:42-50`.

## 5. Wake mechanisms (idle agent acting on a new message)

- **None for a peer interactive session.** No keystroke injection into agent panes, no app-server, no long-poll tool, no Stop-hook block-and-continue loop for messages were found. VERIFIED by absence in grep (`send-keys` only in console sandbox; `app-server` only in two Codex test files and one analysis ADR); INFERRED as a design property.
- **Lead-side re-dispatch**: SubagentStop hook → `team_on_stop` marks the member idle, advances the plan, and returns a *hint* "Spawn or resume agent X for the next plan step". The lead model must act on it. VERIFIED `grok-team-bus.mjs:354-385`; `templates/grok/hooks/subagent-stop-team.json:3-8`.
- **Headless relaunch per task**: dual-mode orchestrator and `ruflo-codex loop` spawn `claude -p <prompt> --output-format text` or `codex exec --sandbox ... <prompt>` per worker/iteration; federation `@peer-<host>` runs `claude -p` over ssh. Every "wake" is a fresh process. VERIFIED `v3/@claude-flow/codex/src/dual-mode/orchestrator.ts:203-241`; `v3/@claude-flow/codex/src/loop/index.ts:219-241`; `ADR-465...md:61-62`.
- **Harness-native background notification**: Claude's own background-task completion and SendMessage delivery ("Never continuously check status after spawning a swarm — wait for results"). VERIFIED `CLAUDE.md:14`; mechanism is Claude Code's, not ruflo's.
- **Scheduled turns**: the console launches Claude Code's `/loop` (cron/self-paced) after a confirm card, warning that every tick is a billed turn. VERIFIED `ADR-414...md:10-30`.
- Cross-session SendMessage from a mod to a peer session: "`ListAgents` from a mod works, lists peer sessions; never sent to a peer: cross-session delivery is unverified". VERIFIED `ADR-465...md:23`.
- Idle/busy detection and draft protection: not applicable (nothing types into a live agent composer). The `$.prompt.fill` path uses `mode: 'replace'` on the person's own composer. VERIFIED `register.ts:141`.

## 6. Delivery guarantees

| Path | ACK | Ordering | Durability | Evidence |
|---|---|---|---|---|
| ADR-402 team bus | none (drain = consumed) | priority prefix then id (`msg_<ms>_<rand>`), lexical | files on disk; archive kept | VERIFIED `grok-team-bus.mjs:229-306` |
| teammate-plugin | `acknowledged:false` field set on send; no ack path found | append order | file on disk; read truncates file (non-atomic read+clear and unlocked read-modify-write: lost messages possible under concurrency, INFERRED); overflow drops oldest | VERIFIED `teammate-bridge.ts:1101-1110,1189-1194,2150-2180` |
| swarm-comms.sh | none | priority 0-3 dirs | files; message JSON built by unescaped heredoc interpolation (quotes in content break JSON, INFERRED) | VERIFIED `.claude/helpers/swarm-comms.sh:31-45` |
| swarm-hooks.sh | `.read=true` flag, but reads do not filter on it (re-reads possible, INFERRED) | `ls -t` newest first, `head -n limit` | files | VERIFIED `.claude/helpers/swarm-hooks.sh:122-160` |
| swarm MessageBus | ack timeout 5 s + retries | priority deques | in-memory | VERIFIED `message-bus.ts:145-257` |
| Federation | none at app layer ("WebSocket already provides delivery confirmation") | unknown | audit log; inbound in-memory queue + events | VERIFIED `ADR-109...md:132-136` |
| Claude SendMessage | Claude Code's own | Claude Code's own | Claude Code's own | n/a |

## 7. Human participation

The person drives the lead Claude Code session and the ruflo console/TUI (`npx ruflo tui`, `/ruflo`). Console "Conversation" targets let the person message `@claude` (visible turn), `@<agent>` (SendMessage, "queued only"), hive/task stores, BBS rooms, peers, Codex or OpenRouter, each with a confirm card showing transport, what leaves the machine and how a reply arrives. Federation explicitly is "not human chat". VERIFIED `ADR-465...md:28-70`; `docs/federation/README.md` "When to use" table.

## 8. License and maturity

- License: MIT, "Copyright (c) 2024-2026 ruvnet". VERIFIED `LICENSE:1-3`; GitHub API `license.spdx_id = MIT`.
- GitHub (API, 2026-10-23): 78,732 stars, 9,247 forks, 969 open issues, created 2025-06-02, last push 2026-10-23T19:42Z. VERIFIED (`gh api repos/ruvnet/ruflo`).
- Releases: v3.60.1 (2026-10-23), v3.60.0 (2026-10-23), v3.59.17 (2026-10-22); root `package.json` version 3.60.1. Very high release cadence. VERIFIED.
- Maturity caveats: much of the multi-host comms is ADR-stage ("Proposed": ADR-402, ADR-491; ADR-385 design), and the repo itself flags legacy unaudited hook shims (`plugin/hooks/hooks.json` `_legacy_unaudited_shim`). VERIFIED. Breadth far exceeds verified depth on the comms path. INFERRED.

## 9. Harness support

| Harness | Status | Notes |
|---|---|---|
| claude | native | Plugin marketplace, hooks (UserPromptSubmit route, SessionStart restore, PreToolUse/PostToolUse, Stop, SubagentStop: `.claude/settings.json:69-153`), mods via function hooks (Claude Code >= 2.1.287, `README.md`), MCP. Peer comms = Claude's native Agent Teams `SendMessage` + teammate-plugin. VERIFIED |
| codex | native (partial) | `@claude-flow/codex` package, `init --codex/--dual`, `[mcp_servers.claude-flow]` in config.toml, skills; ruflo-core Codex hooks are telemetry with empty stdout; ruflo-swarm Codex hooks are a deliberate no-op; mods Codex adapter proposed only. Comms via `team_inbox` pull or headless `codex exec`. VERIFIED `ADR-491...md:16-19,52-71`; `plugins/ruflo-swarm/hooks/codex-hooks.json`; `v3/@claude-flow/codex/README.md:733` |
| hermes | none | Only cited as a design pattern source ("hermes-agent pattern") and as an example `host` string in metaharness tool schema. VERIFIED `v3/@claude-flow/cli/src/mcp-tools/metaharness-tools.ts:569`; `v3/@claude-flow/providers/src/anthropic-provider.ts:330` |
| agy | none | No mention of agy/Antigravity in the clone (grep). VERIFIED absence |
| goose | none | Only in a lockfile. VERIFIED absence in source |
| pi | none | `pi-dev` appears only as an example host name in metaharness docs/schema; no adapter. VERIFIED `plugins/ruflo-metaharness/skills/harness-mint/SKILL.md:47-48` |
| opencode | none | Surveyed in ADR-451 as prior art (plugins with `tool.execute.before/after`); example host name in harness-mint. No adapter. VERIFIED `v3/docs/adr/ADR-451-mod-capability-roadmap.md:217-218` |
| (extra) grok | native | `init --grok`, `.grok/config.toml` MCP, SubagentStop → `team_on_stop`; the reason the ADR-402 bus exists. VERIFIED `ADR-402...md:116-152` |
| (any MCP client) | generic | `npx -y ruflo@latest mcp start`; "hook support depends on the client". VERIFIED `README.md` |

## 10. Takeaways for herdr-threads (INFERRED)

- Ruflo sidesteps waking entirely: work is driven by a lead session spawning children (native subagents or headless `-p`/`exec`), so there is no idle-peer problem to solve. It offers no answer to our core problem (wake an idle, long-lived interactive peer without clobbering a draft).
- Its ADR-402 drain-by-rename mailbox with priority-prefixed filenames is a simple, lock-light local store; our SQLite store with receipts is strictly stronger on ACK and ordering.
- Useful evidence it gathered: Codex `UserPromptSubmit` `additionalContext` injection works (8/9 runs, Codex 0.162.0), Codex command hooks require per-hash human trust in `/hooks`, and command-hook spawn costs 23-43 ms per call (`ADR-491...md:32-50`).
- ADR-465 records a measured Claude mod finding relevant to our Claude mod: `$.tool.call({tool:'SendMessage'})` returns "queued for delivery ... at its next tool round", and cross-session delivery from a mod is unverified (`ADR-465...md:20-23`).
- Anti-patterns to avoid seen in its legacy helpers: per-invocation random agent ids, read-and-truncate mailboxes, unescaped JSON in shell, silent overflow drops, `read` flags that are never filtered on.
