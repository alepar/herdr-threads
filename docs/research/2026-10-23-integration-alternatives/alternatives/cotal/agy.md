# Cotal x Antigravity CLI (agy)

- Upstream: https://github.com/Cotal-AI/Cotal (site https://cotal.ai/)
- Clone: `alts/cotal` (scratchpad), `git clone --depth 1`, HEAD `6426fb538e0b` (2026-10-23 22:04 +0200). The clone was read only. Nothing in it was executed or installed.
- Citations: `path:line` relative to the clone root. **V** = VERIFIED (read in source or docs). **I** = INFERRED. Harness facts come from `primers/agy.md` (cited as "primer §N").

## Support status

**Not supported.** Cotal has no agy connector and does not mention Antigravity anywhere in its code or docs.

| Evidence | |
|---|---|
| The connector matrix lists only Claude Code, OpenCode, Codex, Hermes, Jcode and pi (docs/connectors.md:21-35; `extensions/` holds `connector-{claude-code,codex,hermes,jcode,opencode}` and `pi`) | V |
| `grep -riI 'antigravity\|\bagy\b'` over the clone, excluding `.git` and `node_modules`, finds exactly one hit: the string `"antigravity_oauth.json"` in a list of credential files that the **Jcode** connector keeps out of a private `JCODE_HOME` (extensions/connector-jcode/src/private-state.ts:9-16). That hit is Jcode's own Antigravity OAuth file. It is not an agy integration. | V |
| "Gemini" appears only as **Gemini CLI**, one of the harnesses that receives Agent Skills through `~/.agents/skills/` (implementations/cli/src/lib/agent-skills.ts:15-17; README.md:75; docs/connect-claude.md:67-73). Gemini CLI is a different product from Antigravity CLI. | V |
| cotal.ai lists "Claude Code, Codex, Cursor, Gemini CLI, OpenCode, Hermes, pi and your own models" and says connectors ship for Claude Code, OpenCode and Hermes. It does not mention Antigravity, agy or Google (fetched 2026-10-23). | V |
| The Herdr runtime README names `gemini` only as one of the agent kinds Herdr itself recognizes (extensions/herdr/README.md:63-69). It is not a Cotal connector. | V |

The rest of this report asks which of Cotal's **generic** mechanisms could reach an agy session today without any agy-specific code, and how well each would work.

## Injection mechanism

### A. What Cotal has that is not harness-bound

**A1. The stdio "gateway" MCP server (`cotal_connect` / `cotal_wait`).** This is the only Cotal surface that a non-Claude MCP client could use unchanged.

- Entry point. The Claude plugin's MCP binary is `claude-plugin/cotal/dist/mcp.cjs`, launched as `node … mcp.cjs` (claude-plugin/cotal/.mcp.json). Without `COTAL_NAME`/identity env it does not join the mesh. It serves the gateway instead (extensions/connector-claude-code/src/mcp.ts:151-154) **V**:
  ```ts
  if (!hasIdentity()) {
    ...
    await serveLocal(process.argv.includes("--desktop"));
  ```
- `--desktop` mode is the harness-agnostic variant. It opens no native wake feed (`const feed = desktop ? undefined : await openWakeFeed(process.ppid)`, mcp.ts:84). It passes no `tools` override, so the default `GATEWAY_TOOLS` are offered (mcp.ts:91; gateway.ts:89, 383), and sends `DESKTOP` + `GATEWAY_INSTRUCTIONS` (mcp.ts:131). `cotal setup` writes exactly this entry into Claude Desktop's config, using absolute paths to `node` and to `~/.cotal/claude-plugin` (docs/connect-claude.md:365-369) **V**. Nothing stops an operator from adding the same command to agy's `~/.gemini/config/mcp_config.json`, because agy speaks stdio MCP (primer §4). **I**: untested, and Cotal neither documents nor writes it.
- Tools. The conversation first calls `cotal_connect` with a name. That call provisions an identity through the CLI plumbing `cotal app-identity provision --require-approval --name …` (extensions/connector-claude-code/src/app-host.ts:114). It can return "approval pending" with a link that a person must approve (app-host.ts:117; gateway.ts:475-477). Every later routed tool takes the returned `connection_id` as a required argument (gateway.ts:267, 364-367) **V**. Whether a static local mesh, as opposed to a user-auth mesh, skips that approval is **unknown**: the plumbing lives in the CLI and was not traced.
- **How a message reaches the model.** Only as an **MCP tool result**, when the model calls a tool:
  - `cotal_wait` blocks until a wake-worthy message is buffered (DMs, role requests and @mentions always; other channel traffic only in `open` attention, gateway.ts:528-531; docs/embedding.md:450-453), then returns the same reply `cotal_inbox` would give (gateway.ts:522-569) **V**.
  - Every other routed tool except `cotal_inbox` appends one line when messages are waiting, `"${n} new message(s) waiting. Read it/them with cotal_inbox."` (gateway.ts:355-361, 605-607) **V**. This is a piggyback notice on unrelated tool results, but it fires only on Cotal's own tools.
  - The gateway's own description says it plainly: "A remote MCP client gives the server no way to start a turn, so a conversation hears a message only when it calls a tool" (gateway.ts:370-372; docs/embedding.md:450) **V**.
- Payload format. This is the inbox reply text. A head line `"N message(s):"` (or `(peek: nothing cleared)`) is followed by one line per item (extensions/connector-core/src/tool-specs.ts:936-941) **V**. Per-item framing (`formatItem`, extensions/connector-core/src/framing.ts:124-133) **V**:
  - `[DM from <label>] <body>`
  - `[@<service> from <label>] <body>`
  - `[#<channel>[ @you] <label>] <body>`
  - Optional `(history) ` and `(redelivered, delivery N: an earlier delivery may already have run) ` prefixes.
  - The body is indented so no line of it reaches column 0 (`fmtBody`, framing.ts:96-98).
  - Brackets and line breaks are stripped from peer-named fields (`attributionSafe`, framing.ts:80-82).
- When it fires. Only inside a running turn, at the moment the model chooses to call `cotal_wait`, `cotal_inbox` or another `cotal_*` tool. agy's `PreInvocation`/`SessionStart` hooks are **not** used. Cotal therefore gets none of agy's real turn-start or post-tool-call injection (primer §2.5) **V** (no agy hook config exists in the repo).
- Size bounds. One `cotal_inbox`/`cotal_wait` reply is at most `INBOX_WINDOW_CHARS = 48_000` (tool-specs.ts:225). The reply is assembled first and then trimmed until it fits. Directed mail ranks before channel traffic, which ranks before replay (tool-specs.ts:240-280; the rank is at :279). A message larger than the window goes out in parts, one per call, and is cleared with its last part (docs/mcp-tools.md:93, 99) **V**. The local buffer is `MAX_INBOX = 200`. On overflow, pull-only items are evicted first, then other channel traffic, and directed mail only when the whole buffer is directed; an evicted DM is left unacked for broker redelivery (extensions/connector-core/src/agent.ts:227, 1143-1150; docs/connect-claude.md:483-490) **V**. The agy side: one 48 KB tool result is well under anything agy is known to cap, though agy's own tool-result limit is **unknown**.
- Dedupe and ack. A read clears (acks) only the items its reply actually carried ("SELECT, RENDER, THEN CLEAR EXACTLY WHAT WENT OUT", tool-specs.ts:924-931). `peek` clears nothing. A `cotal_wait` whose client cancels the call stops listening, and a read whose client is gone clears nothing (docs/embedding.md:457-460) **V**. Redelivery: an evicted or unacked directed item is redelivered by the broker after its ack wait (docs/connect-claude.md:486-488), and a redelivered copy is marked with the broker's delivery count (framing.ts:126-127) **V**. That this is JetStream, at-least-once for DMs and anycast, is from the Cotal overview, not re-checked here **I**. The ack point is "the MCP server wrote the tool result". Whether agy actually put that result into the model's context is not confirmed. The Claude hook path goes further and waits for a write receipt (overview.md "Delivery guarantees"), and herdr-threads goes further still with its transcript confirmation (primer §10) **I**.

**A2. Agent Skills (`~/.agents/skills/`).** `cotal setup` reconciles Cotal's `SKILL.md` files (today `team-topology`) into `~/.agents/skills/<name>/SKILL.md` for "Codex, Cursor, OpenCode, Gemini CLI, Windsurf" (agent-skills.ts:7-28; setup.ts:1081-1094) **V**. agy loads skills from plugins, `~/.gemini/config/skills/` and **workspace** `.agents/skills/` (primer §3). A global `~/.agents/skills/` is not in agy's documented list, so agy probably does not see these skills (**I**). Either way they are static instructions, not message delivery.

**A3. CLI.** `cotal send -t|-c|-r` posts from a shell (docs/cli.md:3865-3880) **V**. agy's model could run it through `run_command`, after a `command(cotal)` permission allow (primer §7), but only for sending. `cotal chat --stdio` is an NDJSON participant under its own identity (docs/cli.md:3977-3981), meant for programs, not a model. `cotal history` reads channel history (docs/cli.md:4189). No CLI command reads a connection's buffered DMs for an unmanaged agent the way `herdr-threads inbox` does **I** (none found in the docs/cli.md section headings; individual flags not traced).

### B. What Cotal has that is Claude-bound and would not carry over

- The command hooks forward the hook's stdin JSON to a control socket and print the reply (extensions/connector-core/src/relay.ts:1-8). All dispatch keys on Claude's `hook_event_name` (relay.ts:30-33; extensions/connector-claude-code/src/hooks.ts:313-316). The reply is Claude's `{hookSpecificOutput:{hookEventName, additionalContext}}` (hooks.ts:315-316) **V**. agy payloads carry **no event-name field** and are camelCase (`conversationId`). agy output must be `{"injectSteps":[…]}` (primer §2.2-2.3). So the relay would treat every agy event as unknown and inject nothing **I**: the dispatch is a `switch (event)` on that field, and an unmatched event yields no context.
- The personal-session binding requires Claude's `session_id`, `transcript_path` and `hook_event_name` (extensions/connector-claude-code/src/native-binding.ts:73-77) **V**.
- The native wake module (`hooks.json` `"modules": ["./native-wake.js"]`) uses the Claude Code plugin-mod runtime (`$.prompt.submit`, `$.session.append`, claude-plugin/cotal/hooks/native-wake.js:95-96, 143-144). agy has no equivalent **V** (claude-plugin/cotal/hooks/hooks.json:2; primer §1 has no such surface).
- The block framing `📨 Cotal — N new message(s) from peers:` plus the "do it, verify, then reply" tail (extensions/connector-core/src/control.ts:120-126) appears only on hook injection. A gateway `cotal_wait` reply does not carry it **V** (`cotal_wait` returns `runFor(inbox, …)`, gateway.ts:566).

## Wake mechanism

**There is none for agy.**

- Gateway mode. With no `wake` option (the `--desktop` path passes none, mcp.ts:131), `watch` returns before installing any signal (`if (!wake) return;`, gateway.ts:398-399). The only "wake" is a `cotal_wait` call that the model is already blocked in. Cotal's own docs: "A chat with no turn running cannot be woken by any server" (docs/connect-claude.md:391-394) and "A conversation with no turn running is not woken" (:884-888) **V**.
- The long-poll as a substitute. `cotal_wait` holds for at most `GATEWAY_WAIT_SECONDS = 50` (gateway.ts:188-193) and then returns `"No new messages in 50 seconds. Call cotal_wait again to keep listening."` (gateway.ts:563-564). The server instructions tell the model to keep calling it "when the person asks you to stay on Cotal or keep listening" (gateway.ts:616-619) **V**. Under agy, 50 s sits inside the default 180 s MCP tool timeout (primer §4), so the call would not time out **I**.
  - Idle/busy detection: none. The agent is by construction "busy" (inside a tool call) the whole time it listens. The agy footer would show `esc to cancel` (primer §8) **I**.
  - Draft safety: trivially safe, since nothing is typed. A human cannot type a new prompt while the turn runs, and pressing `esc` cancels the listening turn **I**.
  - Latency: near-immediate while a wait is held. With no wait held, the latency is "until the model next calls a cotal tool", which can mean never.
  - Failure modes **I**:
    1. Each empty 50 s cycle costs one model call that re-issues `cotal_wait`, so a listening agy session spins forever. This runs against the delivery contract's goal that done, idle sessions stay idle (AGENTS.md "Delivery contract", preamble); none of the six numbered rules addresses a model-driven poll directly.
    2. A model that stops re-calling goes deaf until a human types something.
    3. Cotal does not use agy's `Stop` `continue` or `/goal` to automate the loop (primer §6 lists both as block-and-continue primitives).
    4. Context grows by one tool call plus one "no messages" result per 50 s.
- Managed-agent keystroke paths. `cotal input` and the tmux/Herdr runtimes type into the PTY (docs/cli.md:3141-3160; extensions/herdr/src/runtime.ts:182-183, 220-245). They work only for agents the Cotal manager spawned, and the manager spawns only through a registered connector (docs/connectors.md:68-74; docs/authoring-a-connector.md:13-34) **V**. With no agy connector, Cotal cannot spawn agy, so `cotal input` cannot reach an agy pane. Cotal also never uses keystrokes for message wake on any harness (overview.md "No keystroke injection for messages") **V**.

## Setup/installation footprint on the harness

- **What Cotal writes for agy today: nothing.** `cotal setup` writes the Claude plugin (marketplace install), the Claude Desktop `claude_desktop_config.json` entry on macOS and Windows, the `cotal-skills` plugin, `~/.agents/skills/<name>/SKILL.md` with a digest manifest under `~/.cotal`, and a seeded persona (docs/connect-claude.md:37-73, 365-379; agent-skills.ts:7-28) **V**. It does not touch `~/.gemini/**` **V** (no path or reference in the repo).
- **What a hand-rolled agy hookup would need** **I**:
  - One `mcpServers.cotal = {command: "<abs node>", args: ["<~/.cotal/claude-plugin>/dist/mcp.cjs", "--desktop"]}` in `~/.gemini/config/mcp_config.json` or a plugin `mcp_config.json` (primer §4).
  - Probably an `mcp(cotal/*)` permission allow so `cotal_wait` does not prompt each time (primer §4, §7).
  - `cwd` matters. In `--desktop` mode the server resolves the mesh from the filesystem root, so it picks the `cotal use` mesh, else the only one running (mcp.ts:76-79; docs/connect-claude.md:391-392) **V**.
  - Without `--desktop` it would call `openWakeFeed(process.ppid)`, which opens a wake socket keyed by the parent pid (wake-feed.ts:85-94) and serves the Code instructions with `NATIVE_GATEWAY_TOOLS` (mcp.ts:91, 94). No Claude native module would attach, so `listening` would report "Automatic incoming messages are unavailable … call cotal_wait to listen" (mcp.ts:121-124) **V** for the text. The runtime result under agy is unknown.

## Token cost per message and per quiet turn

All INFERRED from the code paths above; nothing was measured.

- **Per message.** One `cotal_wait` or `cotal_inbox` tool call, model-emitted with a `connection_id` UUID argument, plus a tool result of a head line and one framed line per item. That is the same body cost as herdr-threads, plus a model round trip for the call. If the model does not listen, the message waits until some cotal tool returns the one-line "N new messages waiting" notice, which costs about 15 tokens and then another `cotal_inbox` call.
- **Per quiet turn (not listening).** Zero. No hook runs and nothing is injected.
- **Per quiet interval (listening).** One model invocation per 50 s, re-reading the whole conversation context, to emit a `cotal_wait` call and receive about 15 tokens of "No new messages". At agy's context sizes this is the dominant cost, and it is unbounded over time.
- **Per session static.** The MCP server instructions (`DESKTOP` + `GATEWAY_CONNECTING` + `GATEWAY_LISTENING`, mcp.ts:55-59; gateway.ts:616-630) run to about 1,000 characters. The tool schemas for all `GATEWAY_TOOLS` are each carried with a `connection_id` parameter, and every tool description is several sentences (docs/mcp-tools.md), so the schema cost is several thousand tokens per session **I**. Whether agy loads MCP tool schemas eagerly or lazily is unknown (primer §4 does not cover it).

## Limitations and bugs you can see

1. **No push and no wake on agy.** Delivery is pull-only, through a model-initiated tool call. An idle agy session is unreachable. **V** (gateway.ts:370-372; docs/connect-claude.md:391-394), applied to agy **I**.
2. **Listening is a spin loop** that also holds the session "busy" so a human cannot type. That is the opposite of the "done and idle stays idle" goal **I**.
3. **Ack before admission.** The gateway clears a message once its tool result is returned to the MCP transport, with no confirmation that the agy model saw it. A tool result that agy truncates or drops is lost after ack **I**. agy's handling of a 48 KB result is unknown.
4. **Identity binding is per conversation, by model honesty.** In gateway mode the model is told "Never reuse another conversation's connection_id" (gateway.ts:267, 627). The only enforcement is in the Claude plugin, and it covers **hosted**-connector connections: a `PostToolUse` hook records the hosted `cotal_connect` id under `CLAUDE_PLUGIN_DATA` and refuses it from a `--fork-session` fork, while a resumed session and its subagents keep the session id and pass (docs/connect-claude.md:868, 874-882) **V**. So even on Claude a subagent may share its parent's connection. Under agy nothing at all would stop a subagent from reusing its parent's `connection_id`, or a `/clear`ed conversation (new conversation id, primer §2.2) from keeping a stale one **I**. If agy runs one stdio MCP server per agy process, its connections outlive `/clear` **I**.
5. **Mesh discovery in `--desktop` mode ignores the project.** It uses the filesystem root as cwd (mcp.ts:76-79) **V**, so a workspace-scoped mesh would be missed **I**.
6. **Signed-in or cloud spaces are refused** in the local server without a native transcript binding ("refuses before identity issuance", docs/connect-claude.md:381-383) **V**. An agy session could therefore only join a local mesh, or the hosted HTTPS connector, which also cannot start a turn (:388-389) **V**.
7. **`~/.agents/skills` probably invisible to agy.** It is not in agy's documented skill roots **I** (primer §3).

## Clever ideas worth stealing

1. **A long-poll tool as an explicit opt-in listener** (`cotal_wait`, gateway.ts:522-569). It returns at once if something is already buffered, takes a bounded hold below every client timeout (the 50 s rationale is in gateway.ts:189-192), and its "returns the same reply as inbox" design means one framing serves both. For herdr-threads on agy this could be a fallback when keystroke wake is unavailable (Herdr down, or a non-Herdr terminal), or a "stay on the thread" mode a human asks for. It must stay opt-in, given the spin cost. **V** (mechanism) / **I** (applicability)
2. **Waiting-notice piggyback** on every non-inbox tool result (gateway.ts:355-361, 607-608). It is a cheap, bounded hint (about 15 tokens) riding a result the model already reads. herdr-threads' agy `PreInvocation` systemMessage already covers mid-turn delivery better, but the pattern fits harnesses whose only in-turn surface is a tool result. **V**
3. **Assemble-then-trim response budgeting** (tool-specs.ts:240-280). Build the whole reply, measure the real string, and drop trailing items until it fits. Clear only `shown`, and never clear a reply that could exceed the host's cap. This removes the "budgeted items but not the header or footer" class of overflow bug. Worth checking against our page assembly (`src/delivery/page.rs`, 32 items / 64 KiB). **V**
4. **Broker delivery-count marker** `(redelivered, delivery N: an earlier delivery may already have run)` (framing.ts:126-127). It tells the model that a redelivered work order may already have been executed. herdr-threads re-offers (60 s, max 3) and could carry a similarly short marker on re-offers that the transcript scan did not confirm. **V**
5. **Neutralize every peer-controlled field in a frame**, both brackets and every Unicode line separator (`attributionSafe`, framing.ts:64-82; `LINE_BREAK` covers `\r\n?` and `[\n\v\f\u0085\u2028\u2029]`). Check our header and body indenting against the same class. agy's `systemMessage` text is model-visible and column-0 sensitive: our transcript confirmation requires a header at column 0 (primer §10). **V**
6. **The server tells the model whether it will be woken** (`wake.listening`, mcp.ts:121-128; Code path only, the `--desktop` path has no `wake`). The connect reply states "automatic delivery connected / notification-only / unavailable, call cotal_wait". An honest one-line capability statement at bind time beats the model guessing. **V**

## Verification

Adversarial pass on 2026-10-23 against clone HEAD `6426fb538e0b`, https://cotal.ai/ (re-fetched) and `primers/agy.md`.

- **Checked and held.** No agy connector, with the single `antigravity_oauth.json` hit in the Jcode credential list (grep re-run). The connectors matrix. Gemini CLI only through `~/.agents/skills`. The cotal.ai harness and connector wording. The gateway's pull-only delivery and the "no way to start a turn" text. `cotal_wait`: the 50 s bound and the empty-wait reply. The waiting notice. The inbox head line, the 48,000-char window, assemble-then-trim and its rank order. `formatItem` framing, `fmtBody` and `attributionSafe`. `MAX_INBOX = 200` and eviction. Select-render-clear acking. Relay and hook dispatch on `hook_event_name`, and the `hookSpecificOutput` reply. The `formatInjection` header and tail. The native-wake mod calls. `cotal input`, plus Herdr keystrokes used only for stop and interrupt. The `~/.agents/skills` reconcile. The Desktop config entry. Signed-in spaces refused. The primer facts on agy hook payloads, `injectSteps`, skill roots, MCP config and the 180 s timeout, the footer, and permissions.
- **Fixed.** Line citations in mcp.ts, gateway.ts, tool-specs.ts, framing.ts, native-binding.ts and connect-claude.md. `--desktop` uses the default tools and `DESKTOP` instructions (mcp.ts:131), not `GATEWAY_INSTRUCTIONS` alone. `if (!wake) return` is in `watch`, not `fire`. The eviction order now has its middle tier. Added the wake-worthy definition for `cotal_wait`. The fork guard covers hosted-connector ids only, and Claude subagents pass it too, so subagent reuse is not agy-specific. The "AGENTS.md rules 2 and 4" attribution is corrected to the contract's preamble goal. Replaced the garbled `LINE_BREAK` character list with escapes.
- **Downgraded to INFERRED or unknown.** JetStream at-least-once (taken from the overview, not re-traced). agy MCP schema laziness (the primer does not cover it). One MCP server per agy process surviving `/clear`. The absence of an inbox-read CLI command (checked against section headings only).
