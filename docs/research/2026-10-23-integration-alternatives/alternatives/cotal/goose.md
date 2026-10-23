# Cotal x Goose (Block): message injection and wake

- Alternative: Cotal, <https://github.com/Cotal-AI/Cotal>, site <https://cotal.ai/>. Clone `alts/cotal`
  (scratchpad), `git clone --depth 1`, HEAD `6426fb538e0b33597d751324bfa674b513d48c49` (2026-10-23 22:04 +0200).
  The clone was only read; nothing in it was executed.
- Harness: Goose, see `../../primers/goose.md` (goose 1.36.0 installed, main = 1.56.0). Goose facts below are
  taken from the primer and cite it as "primer §N".
- Citations: `path:line` relative to the Cotal clone root. **V** = VERIFIED (read in source or docs),
  **I** = INFERRED.

## Support status

**Not supported.** Cotal has no Goose connector and does not mention Goose anywhere.

- Case-insensitive `grep -rIil goose` over the whole clone (code, docs, SPEC, plugin assets) finds nothing **V**.
  `grep` for ACP / "Agent Client Protocol" (Goose's server protocol) also finds nothing **V**.
- The shipped connectors are `extensions/connector-{claude-code,codex,opencode,hermes,jcode}` plus
  `extensions/pi`, and the runtimes are `extensions/{tmux,cmux,orca,herdr}` (directory listing) **V**. The
  connector table lists the same six harnesses (docs/connectors.md:21-35) **V**.
- A spawn is resolved through the connector registry, and "A pin naming an unregistered connector fails the
  spawn loudly rather than silently falling back" (docs/connectors.md:74-79; packages/core/src/agent-file.ts:45)
  **V**. So `cotal spawn --agent goose` cannot work.
- cotal.ai names Claude Code, Codex, Cursor, Gemini CLI, OpenCode, Hermes and pi, and does not mention Goose or
  Block (fetched 2026-10-23) **V**.
- `gh search issues/prs --repo Cotal-AI/Cotal goose` returns one hit, issue #828, a Jcode launch-error bug
  unrelated to Goose (its body matched the word) **V**. No Goose issue, PR or roadmap entry was found **V**.
- The "generic" tier that `cotal setup` covers is Agent Skills only. It reconciles `~/.agents/skills/` for "Every
  other harness (Codex, Cursor, OpenCode, Gemini CLI, Windsurf/Devin)", with no push delivery
  (docs/connect-claude.md:55-73) **V**. Goose is not in that list. Whether Goose reads `~/.agents/skills/` is
  outside this report; Goose plugins do ship `skills/*/SKILL.md` (primer §4) **I**.

The rest of this report covers what a Goose user could do with Cotal's generic pieces, and how well each would
work given the primer.

## Injection mechanism

There is no Goose injection path. These are the Cotal pieces that could be pointed at Goose.

### A. The stdio MCP server, hand-wired as a Goose extension

Cotal's Claude plugin ships one MCP entry point, `dist/mcp.cjs` (source
`extensions/connector-claude-code/src/mcp.ts`). It has two modes, chosen by whether a launch identity is in the
environment (`mcp.ts:147-154`) **V**:

```ts
if (!hasIdentity()) {
  process.stderr.write("[cotal-connector] no COTAL_NAME — not a managed session; serving cotal_connect for this computer's meshes\n");
  await serveLocal(process.argv.includes("--desktop"));
  return;
}
const session = await serveClaudeSession({ env: process.env, input: process.stdin, output: process.stdout, ... });
```

**A1. Unmanaged / `--desktop` gateway mode.** `serveLocal(true)` serves `serveGatewayStdio` with no native feed:
`cotal_connect`, `cotal_wait`, `cotal_disconnect` and the routed `cotal_*` tools, each of which needs a
`connection_id` (`mcp.ts:74-145`; `extensions/connector-core/src/gateway.ts:712-724`) **V**. The `--desktop`
branch skips `openWakeFeed` and the native tool set (`mcp.ts:84,91`) and serves the Claude-Desktop instructions
(`DESKTOP`, `mcp.ts:55-59,131`) **V**. Goose would load it as an ordinary stdio extension (primer §6) **I**.

The blocker is identity. `cotal_connect` shells out to `cotal app-identity provision --require-approval`
(`connector-claude-code/src/app-host.ts:114`), and provisioning refuses on every mesh except a signed-in
(user-auth) one **V**:

```ts
if (t.mode === "user") { ... provisionUserApp(...) ... }
// A static or open mesh has no authority where a person approves a connection, so it opens none.
return { ok: false, error: `the mesh "${t.space}" has no human approval for app conversations, so no identity was opened; ...` };
```

(`implementations/cli/src/commands/app-identity.ts:94-101`; docs/connect-claude.md:249-252) **V**. A default local
`cotal up` mesh is JWT-authed with no approval authority (docs/getting-started.md:118,132), so a Goose
conversation in gateway mode gets an error from `cotal_connect` and never joins **I** (from the code path; not
run).

A signed-in mesh does not rescue the local server either. Signed-in provisioning requires a native session id
and throws "a signed-in conversation needs its native session identity before connecting, and this app supplies
none" without one (`app-identity.ts:146-161`) **V**. That session binding comes from Claude Code's own
lifecycle/transcript binding (`app-host.ts:102-103`; docs/connect-claude.md:254-256), which Goose cannot supply,
and the `--desktop` server refuses signed-in spaces outright (docs/connect-claude.md:381-382) **V**. So the
local stdio server joins no mesh for a Goose session, static or signed-in **I** (from the code path; not run).

The only remaining gateway route is the **hosted** connector a signed-in space advertises
(`cloudCapabilities.hostedConnector`, an HTTPS MCP URL; docs/connect-claude.md:381-389) **V**, which Goose could
add as a streamable-HTTP extension (primer §6) **I**. It exists only on Cotal Cloud, which cotal.ai describes as
"in closed alpha behind a waitlist" (fetched 2026-10-23) **V**; its auth flow for a non-Claude client is unknown.
The gateway tool behaviour below (A1) applies to that hosted path **I**.

**A2. Managed mode, wired by hand.** With `COTAL_NAME` and credentials set, `startSession` requires a
control socket and token for the lifecycle hooks and refuses without them: "no control endpoint for the
lifecycle hooks: set COTAL_CONTROL_SOCKET ... and COTAL_CONTROL_TOKEN" (`mcp.ts:238-243`; comment at
`mcp.ts:231-237` allows `COTAL_CONTROL_TOKEN` "in a session driven by hand") **V**. It then registers the plain
`cotal_*` tools (`mcp.ts:334`; `connector-core/src/tools.ts:23-35`) and advertises the
`experimental["claude/channel"]` capability (`mcp.ts:305`) **V**. A hand-built env with credentials from the
manager could plausibly get a Goose session onto a local mesh this way **I**. There is no documented recipe and
no `--agent goose`, so the manager does not issue such credentials for an unknown harness **I**.

### What reaches the model in either mode

- **Only tool results.** Delivery happens only when the model calls `cotal_inbox` (or `cotal_wait`, A1 only).
  - Goose hooks have no `additionalContext` field; only a PreToolUse deny reason and (main only) a Stop-block
    reason or SessionStart banner reach the model, all with fixed framing (primer §3.4). Cotal's Claude hook relay (`SessionStart` and
    `UserPromptSubmit` drain into `additionalContext`, docs/connect-claude.md:404-406) has no Goose equivalent,
    and Cotal ships no Goose hook anyway **V**.
  - Goose's MOIM/`tom` file (primer §5) is the main Goose surface that could carry pushed content into a
    terminal session (ACP steer, primer §7, needs an ACP-hosted session). Cotal uses neither **V** (absence).
- **Payload.** `cotal_inbox` returns `"<N> messages:"` followed by one line per item:
  - `[DM from <peer>] <body>`
  - `[@<role> from <peer>] <body>`
  - `[#<channel>[ @you] <peer>] <body>`

  Bodies are indented so no peer line can reach column zero. A `(history) ` marker or a
  `(redelivered, delivery N: an earlier delivery may already have run) ` marker is prepended when it applies
  (`connector-core/src/framing.ts:96-149`; `tool-specs.ts:259-330`) **V**. Peer-controlled attribution fields are
  stripped of line breaks and brackets (`framing.ts:80-82`) **V**. The hook-injection tail ("do it with your own
  tools first and verify the result ...", `connector-core/src/control.ts:120-127`) is used only by the hook
  path, so a Goose tool pull would not get it **I**.
- **When.** Only when the model calls the tool. In A1, every other routed tool appends
  `N new messages waiting. Read them with cotal_inbox.` when wake-worthy mail is buffered
  (`gateway.ts:355-361,607-608`) **V**. In A2 this notice is not added (`tools.ts:28-34`) **V**.
- **Size bounds.** One `cotal_inbox` reply is at most `INBOX_WINDOW_CHARS = 48_000` characters
  (`tool-specs.ts:225`) **V**. The reply is assembled, then measured, and trailing items are dropped until it
  fits. Items are ordered DMs and anycast first, then channel traffic, then replayed history. Items that are held
  back are named in a note. A message larger than one window is sent in parts across calls
  (`tool-specs.ts:236-258`; docs/connect-claude.md:474-479) **V**. Goose adds no truncation of its own on tool
  results beyond its normal context handling **I**.
- **Dedupe / ack.** A read clears, and acks to JetStream, only the items it actually returned. `peek` clears
  nothing. Nothing is cleared while the response is still hypothetical (`tool-specs.ts:916-958`) **V**.
  Redeliveries carry the broker's delivery count in the frame (`framing.ts:126-127`) **V**. In gateway mode a read
  whose request signal was aborted clears nothing (`gateway.ts:345-353`; `tool-specs.ts:923`) **V**; A2's plain
  registration passes no signal (`tools.ts:28-32`) **V**. Either way this is ack-on-return inside the server
  process, not ack-on-model-read: if Goose drops the result after the server returned it (crash, context loss),
  the message is already acked **I**.

## Wake mechanism

**No wake for an idle Goose session.**

- **Channel nudge: dead.** In A2 the wake policy pushes `notifications/claude/channel` (`mcp.ts:338-341`), and
  only when the client's `initialize` advertises `experimental["claude/channel"]` or `COTAL_CHANNEL` forces it
  (`mcp.ts:350-358`) **V**. Goose advertises no such capability, so the channel stays off **I**. Even if it were
  forced on, Goose routes server notifications only to the UI, and only while a tool of that server is running
  (primer §6) **V**. A forced notification would therefore reach no subscriber and never reach the model **I**.
- **`cotal_wait` long-poll (A1 only).** It blocks up to `GATEWAY_WAIT_SECONDS = 50` (max, per call), returns at
  once if wake-worthy mail is already buffered, and otherwise returns as soon as one arrives, with the
  `cotal_inbox` reply. Without mail it returns "No new messages in N seconds. Call cotal_wait again to keep
  listening." (`gateway.ts:189-193,522-566`) **V**. "Wake-worthy" follows attention: DMs, role requests and
  @mentions always; other channel traffic only in `open` (docs/embedding.md:450-453) **V**. Per the previous
  section, the only gateway Goose could reach is the hosted Cloud connector **I**.
  - This works only inside a running Goose turn, as a model-driven listen loop that the server instructions ask
    for: "When the person asks you to stay on Cotal or keep listening, call cotal_wait again each time it
    returns" (`gateway.ts:616-619`) **V**. Goose renders MCP server instructions; how it surfaces them to the
    model is unknown.
  - Costs on Goose:
    - Each empty return is a full provider round trip **I**.
    - The loop counts against `max_turns` (primer §1), so it is not "idle" in any useful sense **I**.
    - Goose's per-extension tool timeout default is unknown (primer §6). It must exceed 50 s, or `seconds` must
      be lowered **I**.
- **Busy/idle detection.** None for Goose. Cotal's presence comes from harness hooks or events (Claude
  `UserPromptSubmit`/`Stop`, docs/connect-claude.md:551-559; Codex app-server and the OpenCode in-process plugin,
  docs/connectors.md:23-25) **V**. A2's
  presence would be driven by the missing hook relay, so the Goose session's roster status would not track its
  turns **I**.
- **Keystroke wake.** `cotal input --name <n> --text <t>` types one line into a managed agent's terminal
  (docs/cli.md:3142-3156) **V**; the manager writes it to the seat's PTY as a bracketed paste in 2048-char slices,
  then `\r` (implementations/manager/src/manager.ts:275-287,11034-11042) **V**.
  This is an operator command, not a message-wake path, and it needs a manager-spawned seat, which in turn needs
  a connector **V**. If someone did type into a `goose session` prompt, primer §9 applies:
  - text typed while busy becomes the next prompt after the turn;
  - Ctrl+C would wipe a draft;
  - approval and elicitation prompts would swallow keys.

  Cotal's `input` op has no draft check (none in manager.ts:11020-11050) **V** for that op; other paths unchecked.
- **Draft safety / latency / failure modes:** not applicable, since there is no wake.
  - With `cotal_wait`, latency is the time from arrival to tool return, sub-second **I**, plus one model call.
  - Failure modes: the model stops looping, the tool timeout cuts the wait, or the turn ends. After the turn
    ends, mail waits for the next human prompt plus a model-initiated `cotal_inbox` **I**.

## Setup / installation footprint on Goose

Cotal writes nothing for Goose **V** (absence of any Goose path in `cotal setup`, docs/connect-claude.md:55-73;
docs/setup-internals.md). A user would hand-add a stdio extension **I**, either:

- `goose session --with-extension 'node ~/.cotal/claude-plugin/dist/mcp.cjs --desktop'` (flag form, primer §2); or
- the equivalent extension entry in Goose's own config. The exact config file and keys were not verified here.

Either local form loads but joins no mesh (see A1); the hosted Cloud connector URL as an HTTP extension is the
only gateway form that could connect **I**.

The plugin copy path is from docs/connect-claude.md:367-369 **V**. In A2 the user would also have to supply
`COTAL_NAME`, mesh credentials, `COTAL_CONTROL_SOCKET` and `COTAL_CONTROL_TOKEN` in the extension env **V**
(mcp.ts:238-243). No hooks, plugin or MOIM file would be installed **I**.

## Token cost

- **Per quiet turn:** the tool schemas of every `cotal_*` tool, on every provider request, because Goose sends
  extension tools with each call **I**. The schemas are verbose. The `cotal_inbox` description alone is about
  800 characters (`tool-specs.ts:915`), and `cotal_wait` plus the `connection_id` argument on every routed
  tool add more **V** (text) / **I** (the total was not measured). The gateway server instructions are about 770
  characters (`gateway.ts:616-630`) **V**; whether Goose puts them into the model context is unknown. There is no
  per-turn injection when quiet, because nothing is injected **I**.
- **Per message:** the model spends one tool call (a `cotal_inbox` call, or a `cotal_wait` return) plus the
  framed lines: about 20-40 bytes of header per item plus the body, and a one-line head **V** (format) / **I**
  (sizes).
- **Listening:** each empty `cotal_wait` return costs a whole model turn of context re-send. That is the
  dominant cost and is unbounded while the model keeps listening **I**. herdr-threads' rule that idle sessions
  stay idle (AGENTS.md "Delivery contract") is exactly what this loop violates.

## Limitations and bugs you can see

1. **No Goose support at all.** No connector, no docs, no hooks, no ACP client **V**.
2. **The local gateway server joins no mesh for Goose.** It refuses on static or open meshes by design
   (`app-identity.ts:99-101`), and on signed-in meshes without a native Claude session binding
   (`app-identity.ts:146-161`) **V**. The only no-code Goose path is Cotal Cloud's hosted connector (closed
   alpha) with per-conversation human approval **V**/**I**.
3. **Managed mode is Claude-shaped.**
   - It insists on a control socket for hooks Goose does not run (`mcp.ts:238-243`) **V**.
   - It advertises a Claude-only experimental capability (`mcp.ts:305`) **V**.
   - Its presence would not track Goose turns **I**.
4. **No push without a tool call.** Mail reaches a Goose model only on a model-initiated call. Goose's
   notification routing (primer §6) closes the channel route, and Cotal does not use MOIM or ACP **V**/**I**.
5. **Ack on return, not on consumption.** A cancelled Goose turn after a successful `cotal_inbox` loses those
   items with no redelivery **I**. Contrast Cotal's own Claude hook path, which acks only after a
   stdout-write receipt (docs/connect-claude.md:407-426) **V**.
6. **Long-poll vs Goose's tool timeout.** The timeout is unknown. If it is under 50 s, every `cotal_wait`
   errors **I**.

## Clever ideas worth stealing (for a herdr-threads Goose adapter)

1. **The "N new messages waiting" rider on every other tool result** (`gateway.ts:355-361,607-608`) **V**. It is a
   cheap, one-line, in-band hint that costs nothing on quiet calls. For Goose, where hooks cannot inject, a
   `herdr-threads` MCP extension could append the same line to its own tool results. Better still, the daemon
   could write the pending items into the `tom` MOIM file (primer §5), which Cotal never thought to use.
2. **Ack exactly what the assembled reply carried, measured on the finished string**, with directed mail first,
   history last, and oversized items in parts (`tool-specs.ts:236-258`) **V**. Our 8 KiB truncation plus `body
   <id>` covers the same ground. Their "build, measure, then drop the tail" rule is a sound guard against
   framing writers that were not budgeted.
3. **The redelivery marker taken from the broker's count** (`framing.ts:126-127`) **V**. For a MOIM-file design,
   where Goose re-sends the same bytes on every call (1.36), an explicit marker on items that were shown before
   would help the model ignore repeats.
4. **A bounded long-poll as an opt-in "stay on the mesh" mode** (`gateway.ts:522-566`) **V**. It is the only wake
   Cotal could offer Goose. We should not adopt it as default (it breaks the idle-stays-idle invariant), but it
   is a reasonable explicit `herdr-threads wait` for a user who asks a Goose agent to listen.
5. **Capability-gated wake** (`mcp.ts:350-358`): push only when the client's `initialize` advertises support,
   else stay silent **V**. This matches our "degrade only the broken capability" rule. A Goose adapter could key
   on the ACP `session/prompt` / `_goose/unstable/session/steer` surface (primer §7) when Goose is ACP-hosted,
   and fall back to Native typing into a `goose session` pane otherwise.

## Verification

Adversarial pass, 2026-10-23, against the Cotal clone at `6426fb538e0b`, cotal.ai, `gh search` and the Goose primer.

- Confirmed: no `goose`/ACP hits in the clone; connector list and table (docs/connectors.md:21-35); unregistered
  connector fails loud; cotal.ai harness list (no Goose/Block); `gh search` (issue #828 only, no PRs); `mcp.ts`
  mode split, control-socket refusal, `claude/channel` capability and gating; gateway `cotal_wait` (50 s, text),
  waiting-notice rider and its absence in A2; 48 000-char inbox window and assemble-then-drop rule; framing and
  attribution stripping; hook-path `formatInjection` tail; hook ack receipts; primer §3.4/§5/§6/§9 facts.
- Changed:
  - A1: the local stdio server also fails on signed-in meshes (needs a native Claude session binding,
    `app-identity.ts:146-161`; Desktop refuses signed-in spaces). Added the hosted Cloud connector as the only
    gateway route; "closed alpha" now cited to cotal.ai. Updated limitation 2 and the setup section to match.
  - Hook claim narrowed: Goose has no `additionalContext`, but deny/Stop-block/banner text can reach the model.
  - Ack: gateway reads with an aborted signal clear nothing; loss window is after return.
  - Keystroke wake citation was wrong (herdr runtime lines are confirm keys and `/exit`); now cites the manager's
    bracketed-paste `input` op.
  - Citation fixes: `mcp.ts:138` -> `55-59,131`; `app-identity.ts:89-102` -> `94-101`; presence sources split
    across two docs; `cotal_inbox` description ~800 chars (was ~900); instructions ~770 chars at `616-630`.
