# Cotal: overview

- Upstream: https://github.com/Cotal-AI/Cotal (site https://cotal.ai/, docs https://docs.cotal.ai)
- Clone used: `alts/cotal` (scratchpad), `git clone --depth 1`, HEAD `6426fb538e0b` (2026-10-23 22:04 +0200). The clone was only read; nothing in it was executed.
- Citations are `path:line` relative to the clone root. **V** = VERIFIED (read in source or docs), **I** = INFERRED.

## What it is

Cotal describes itself as "the open pub/sub standard for AI agents" (README.md:13). It gives agents from different vendors a shared space with presence, channels, direct messages and role-addressed anycast, built on NATS + JetStream (README.md:33-53, 115-150) **V**. The wire spec is `SPEC.md` (662 KB) and covers subjects, envelopes, presence, channels, the JetStream binding, auth, an "endpoint control surface" and a workflow language called "Cotal Lang" (SPEC.md section list; README.md:152-178) **V**. The reference implementation is a TypeScript pnpm monorepo, npm `cotal-ai` / `@cotal-ai/*` **V**. Cotal Cloud (a hosted mesh) is in closed alpha behind a waitlist (https://cotal.ai/) **V**.

Scope note: this is a much larger system than herdr-threads. It covers a multi-agent orchestration platform (a manager that spawns agents, workflow runs, a web dashboard, cloud sandboxes), and messaging is one part of it.

## Architecture

| Aspect | Finding | |
|---|---|---|
| Transport | NATS core subjects for live traffic, JetStream streams for durable traffic. Subject families: `chat.>` (multicast), `inst.>` (unicast), `svc.>` (anycast) (README.md:282-283; docs/presence-and-delivery.md:158-200) | V |
| Broker / daemon | `cotal up` starts a local NATS broker, a "delivery daemon" (Plane-3: a fan-out writer, a trusted reader and a membership registry) and a detached "manager" that spawns and supervises agents (README.md:79-90; docs/delivery-daemon.md:1-62) | V |
| Process model per harness | One connector per agent session. Claude: an MCP server plus hook processes in each session (docs/connect-claude.md:130-149). Codex: a host process that drives `codex app-server` (docs/connect-codex.md:68-75). OpenCode: an in-process plugin inside `opencode serve` (docs/connect-opencode.md:70-73). Hermes: a launcher that owns the mesh endpoint and runs `hermes gateway run`, with a Python plugin bridged over a Unix socket (docs/connect-hermes.md:96-110). pi: an in-process extension (docs/connect-pi.md:15-27) | V |
| Storage | JetStream streams and KV (presence buckets, per-member durable stores, the membership registry). Each reader keeps its own bookmark (README.md:271-276; docs/presence-and-delivery.md:202-210) | V |
| Spaces | Several meshes can run side by side, each with its own broker (`cotal meshes`, `cotal use`, `--space`) (README.md:44-50) | V |
| Cross-machine | Yes. The broker can run on a server, and NATS clustering is supported. Laptops, workstations and cloud containers can join one space (README.md:50-53, FAQ "does it run distributed") | V |
| Terminal runtimes | The manager's agent runtimes are a PTY, plus tmux, cmux, Orca and **Herdr** (extensions/herdr/README.md). The Herdr runtime runs agents in a dedicated named Herdr session `cotal-<space>`, never the user's default session (extensions/herdr/README.md:19-27), and requires herdr >= 0.8.0 (:11-15) | V |

## Identity and session model

- **Identity is carried in the NATS subject.** `cotal up` uses JWT auth by default. The broker binds the sender in each subject to the agent's credential, and a receiver drops any payload that claims a different sender (README.md:88-90, 265-268; FAQ "Can an agent impersonate another?") **V**. This is adversarial-grade identity with default-deny per-agent ACLs. herdr-threads uses a cooperative same-user model instead.
- Each agent's identity card is an A2A `AgentCard`. Its `role` is the anycast address (README.md:143-147) **V**.
- **Lifecycles.** A DM durable belongs to the recipient's lifecycle. A message sent to an agent reaches the same session when it reconnects, but a new agent spawned under the same name does not receive it (docs/presence-and-delivery.md:216-220) **V**.
- **Claude session binding.** A managed session joins under the identity given by `COTAL_NAME`, `COTAL_LINK` or `COTAL_AGENT_FILE`. A plain `claude` session stays inert until the conversation calls `cotal_connect` (docs/connect-claude.md:199-203). In a personal native session, command hooks supply the `session_id` and `transcript_path` binding, and `cotal_connect` requests an identity for that conversation, which a person must approve on a web page (docs/connect-claude.md:243-268, 299-303) **V**.
- **One consumer per inbox.** A crash-safe same-host claim allows only one consuming connector per (broker, space, principal, lifecycle) (docs/connect-claude.md:233-241) **V**.
- **Presence states** are `idle`, `waiting`, `working` and `offline`, with heartbeats, `activeAt`, an optional `condition` (rate_limit, approval, ...) and an attention mode (docs/presence-and-delivery.md:9-40) **V**. Each harness's hooks or events map onto these states (Claude hooks: docs/connect-claude.md:542-568).

## Injection mechanisms (general)

Every connector exposes the same `cotal_*` tool surface. Pull reads go through `cotal_inbox`, and remote or hosted clients use the long-poll `cotal_wait` (docs/connect-claude.md:880-888; docs/embedding.md:450-481) **V**. Push delivery depends on the harness:

| Harness | Push injection path | |
|---|---|---|
| Claude (managed seat) | `SessionStart` and `UserPromptSubmit` command hooks drain the inbox and inject it as `additionalContext`. The docs call this "the single authoritative path" (docs/connect-claude.md:404-406) | V |
| Claude (personal native session) | A native hooks module (`hooks/hooks.json` `"modules": ["./native-wake.js"]`). When idle it calls `$.prompt.submit({text})`. When busy, at a main-agent `tool.call` boundary, it calls `$.session.append({message:{type:'user',...}})` (extensions/connector-claude-code/hooks/native-wake.js:73-100, 309-315). It deliberately does not append in `turn.step`, because that request already has a fixed message snapshot (:316-321) | V |
| Codex | `turn/start` on a `codex app-server` thread. A directed message that arrives mid-turn goes in through `turn/steer` with `expectedTurnId` (docs/connect-codex.md:76-79; extensions/connector-codex/src/host.ts:7-8) | V |
| OpenCode | Each batch is injected as a turn through the authenticated OpenCode server HTTP API. Pending items are also folded into the next native prompt, and a running turn is never interrupted (extensions/connector-opencode/src/plugin.ts:11-16) | V |
| Hermes | A gateway platform adapter builds a `MessageEvent`, and the gateway starts a fresh agent for each inbound message (extensions/connector-hermes/plugin/cotal/adapter.py:3, 61, 236; docs/connect-hermes.md:96-100) | V |
| pi | A custom message with `triggerTurn: true, deliverAs: "steer"` (extensions/pi/src/driver.ts:34, 350; docs/connect-pi.md:29-33) | V |

**Bounds.** Claude native inline delivery carries at most 8 whole messages per batch. Each rendered message must be at most 1,024 code points and each socket frame at most 4,096 characters. Larger messages fall back to a fixed one-line pointer, `Cotal: a new message for connection_id <id>. Read it with cotal_inbox.`, plus a preview (docs/connect-claude.md:338-343) **V**. A `cotal_inbox` pull is bounded: directed mail comes first, then channel traffic, then replay. An oversized message is delivered in parts across calls (docs/connect-claude.md:474-479) **V**.

**Framing.** On Claude, a managed channel message arrives as `<channel source="cotal" from="bob" kind="dm" ...>` (docs/connect-claude.md:229-231). Lines that start at column zero belong to the connector, and peer-controlled text is neutralized so no line break or bracket survives (:534-540). A delivered block has a header, the items and a tail. The tail tells the model to do the work, verify it, then reply. They added it after a weak model confirmed work it had not done (docs/presence-and-delivery.md:186-200). A redelivered copy is marked as such in the injected frame, using the broker's delivery count (:236-240) **V**. Contrast: herdr-threads keeps static text out of each delivery (R17), while Cotal adds an instruction tail to every block.

## Wake mechanisms (general)

- **Claude, managed seat.** An MCP channel notification (`notifications/claude/channel`) wakes an idle session, and the hook drain then delivers the content. The nudge never acks, and a rejected nudge is retried with bounded backoff (docs/connect-claude.md:427-439). The launch uses `--dangerously-load-development-channels server:cotal`, and the runtime answers the confirm dialog by watching the screen and pressing Enter (:142-149, 204-216). On Team and Enterprise accounts, channels must be enabled by an organization admin. Without them, a session gets no idle wake and only hook delivery (:79-103) **V**.
- **Claude, personal native session.** The native module's `$.prompt.submit`, under plugin-sender attribution (docs/connect-claude.md:327-336) **V**. After a user Stop (`turn.complete` with `isAborted`), both wake paths **pause** until the next person-origin prompt (`origin.kind` in composer, bridge or sdk) (native-wake.js:283-292, 322-333; docs/connect-claude.md:357-362) **V**. herdr-threads may want the same rule.
- **Codex.** A real `turn/start` from the app-server host. The Codex TUI attaches to the same thread with `codex resume --remote` (docs/connect-codex.md:76, 113-119) **V**.
- **OpenCode, Hermes, pi.** Each wakes through its native turn API (see the table above) **V**.
- **No keystroke injection for messages.** tmux, cmux and Herdr keystrokes (`send-keys`, `herdr pane run`, `sendKeys`) are used only for spawn, startup dialogs, graceful `/exit` stop, ctrl+c interrupt and the operator command `cotal input` (extensions/herdr/src/runtime.ts:182-183, 220-245; extensions/tmux/src/driver.ts:401-407; docs/cli.md:3141-3153) **V**. Message wake never types into a composer, so a human's draft is never at risk from it **I** (no message-wake keystroke path was found).
- **Priority, the "two tiers".** Directed traffic (DMs, anycast, `@mentions`) always nudges. Ambient channel chatter is held mid-turn and gets one batch nudge on the `Stop` to idle transition (docs/connect-claude.md:441-444). Attention modes `open`, `dnd` and `focus` plus per-channel `quiet` and `muted` refine this (:455-532; README.md:278-279). Attention is advisory, and any peer can wake a `dnd` agent by naming it (:527-531) **V**. This plays the role of our priority threshold T, but it is keyed on addressing mode rather than a numeric priority.
- **Busy vs idle detection** comes from harness hooks or events: Claude `UserPromptSubmit`, `Stop` and `Notification`; Codex app-server events; OpenCode bus events; Hermes `pre_llm_call` and `post_llm_call` (docs/connect-claude.md:551-559; docs/connect-hermes.md:213-220) **V**.

## Delivery guarantees

- **Durability.** DMs and anycast are always at-least-once through JetStream durables. Live channel traffic is at-most-once unless the channel is marked `durable`, in which case the delivery daemon fans a copy out to each member (docs/presence-and-delivery.md:202-210; docs/delivery-daemon.md:1-16) **V**.
- **Ack-on-surface.** Claude hook drain acks only after the hook relay confirms its stdout write succeeded (a receipt is sent back over the control socket). It does not ack when the reply is merely formatted. This guards against a lost reply, accepting duplicates instead (docs/connect-claude.md:407-426) **V**. Codex and OpenCode ack when the turn completes and renew the ack wait with JetStream in-progress acks while a long turn runs (docs/connect-codex.md:89-98; docs/connect-opencode.md:107-123) **V**. pi acks only IDs the provider confirmed, at a terminal boundary (docs/connect-pi.md:29-48) **V**. Claude native inline delivery commits after the host confirms admission: a stored prompt row, or an appended row ID (docs/connect-claude.md:349-355) **V**.
- **Ordering.** There is no global order. Per-consumer delivery follows the backing stream except where redelivery or backfill interleaves older messages (SPEC.md:261-264) **V**. Directed mail is drained before channel traffic (docs/connect-claude.md:474-476) **V**.
- **Overflow.** The local inbox is bounded. When it overflows, pull-only items are evicted first. An evicted DM is left unacked, so the broker redelivers it later (docs/connect-claude.md:483-490) **V**.
- **Send result.** A send reports `stored seq N` and the recipient's roster status at send time. A send never claims `delivered` (docs/presence-and-delivery.md:212-221) **V**.
- **No ACK-request primitive** in the herdr-threads sense, where the recipient explicitly acknowledges and the sender gets a deadline warning. Acks are transport-level. Workflow `checkpoint` and `ask` effects cover human approvals (README.md:154-178) **I** (no per-message ACK-request found in the docs read).

## Human participation

- `cotal chat` opens a live session with its own identity, inbox and presence (docs/cli.md:3943-3952) **V**. `cotal send`, `cotal console` (TUI), `cotal watch` and a read-only web dashboard are also available (README.md:81-84, 180-212) **V**.
- `cotal attach` and `cotal input` stream and drive a managed agent's PTY (docs/cli.md:238-239, 3141-3153) **V**.
- Workflow checkpoints are answered from the CLI or by an agent through `cotal_run` (README.md:171-178) **V**.
- On user-auth meshes, people sign in, and a person approves each Claude conversation identity on a web page (docs/connect-claude.md:255-268) **V**.

## License and maturity

- License: Apache-2.0 for everything in the repo. A future hosted server may be offered under AGPL plus a commercial license (LICENSING.md) **V**.
- GitHub (gh api, 2026-10-23): 338 stars, 43 forks. Repo created 2026-06-02, last pushed 2026-10-23. Latest releases are v0.100.0, v0.100.1 and v0.100.2, all on 2026-10-23 **V**. That is three releases in one day, PR numbers above 4,500 and branch names like `factory/lane-...`, which point to a heavily automated agent-driven development pipeline **I**.
- Connector maturity per the project itself: Claude stable, OpenCode beta, Codex beta, Jcode beta, Hermes alpha, pi alpha (docs/connectors.md:23) **V**.

## Harness support

| Harness | Status | How (evidence) |
|---|---|---|
| claude | native | Installed plugin plus MCP server plus command hooks. Managed seats get idle wake from a development channel. Personal sessions get idle and tool-boundary delivery from a native hooks module (docs/connect-claude.md; extensions/connector-claude-code/hooks/). Stable. **V** |
| codex | native | A host process drives `codex app-server`: `turn/start` to wake, `turn/steer` mid-turn, `cotal_*` served over a loopback MCP endpoint, the TUI attached with `codex resume --remote`. Managed spawns only, POSIX only, no resume. Beta (docs/connect-codex.md:68-117, 276-303). **V** |
| hermes | native | A launcher supervises `hermes gateway run`, and an in-gateway Python platform-adapter plugin injects a `MessageEvent` per message (a fresh agent each time, no mid-turn steer). Headless, no TUI, Unix only, `hermes-agent >=0.18,<0.22`. Alpha (docs/connect-hermes.md; docs/connectors.md:25-27). **V** |
| agy | none | No connector, and no mention of Antigravity or agy anywhere in the repo docs (grep over all `*.md`). **V** (absence) |
| goose | none | No connector, and no mention in the repo. **V** (absence) |
| pi | native | An in-process pi extension: custom messages with `triggerTurn` and `deliverAs: "steer"`, acks gated on provider confirmation, pins pi 0.79.10. Alpha (docs/connect-pi.md; extensions/pi/src/driver.ts:34, 350). **V** |
| opencode | native | An in-process plugin inside `opencode serve`, config injected through `OPENCODE_CONFIG_CONTENT`. It injects turns over the server HTTP API, with the TUI attached to that session. 1.x and 2.x. Beta (docs/connect-opencode.md:70-149; extensions/connector-opencode/src/plugin.ts:1-37). **V** |

Generic: Gemini CLI, Cursor, Windsurf and Codex receive only Agent Skills (`~/.agents/skills/`) from `cotal setup`, with no push delivery (docs/connect-claude.md:55-73) **V**. There is also a sixth connector, Jcode, which uses the Harness API over a Unix socket (docs/connect-jcode.md) **V**.

## Points worth carrying into the comparison

1. **Native mod parity.** Cotal's Claude native module uses the same plugin-mod surface as our `register.js`: `prompt.submit`, `session.append` at `tool.call`, and `turn.complete`. Two rules stand out: append only at `tool.call`, never at `turn.step` (snapshot-fixed), and pause wakes after a user abort until the next person-origin prompt. Compare both with our mod **V**.
2. **Codex via app-server** gives real wake and mid-turn `turn/steer` with no composer typing, but only for agents Cotal launches. It does not attach to a user's existing Codex TUI session the way our Herdr-native wake does **V**/**I**.
3. **Ack semantics.** Ack after the stdout-write receipt (Claude hooks), at turn completion with in-progress renewal (Codex, OpenCode), or after provider confirmation (pi). This is a finer "delivered" definition than display-ACK **V**.
4. **Per-block instruction tail** (do, verify, reply). It trades tokens for task follow-through, the opposite of our R17 framing minimalism **V**.
5. **Heavier security model.** JWT, subject-bound sender identity, default-deny ACLs and person-approved conversation identities. Our trust policy rules out adversarial verification by design **V**.
