# cross-agent-teams-mcp (jtianling) x Goose (Block)

- Alternative: https://github.com/jtianling/cross-agent-teams-mcp, clone at
  `17df5cf7bd9b66f00460eff05080b0e0495b6ec8`. Paths written as `xats:<path>:<line>` are relative to that
  clone's root. The code was read and never run.
- Harness: Goose. Paths written as `goose@1.36:<path>:<line>` are in the v1.36.0 tree described in
  `../../primers/goose.md` (the primer). That tree matches the installed binary.
- Tags: VERIFIED means I read it in source or docs. INFERRED means it is my reading and the source does not
  state it. "unknown" means I could not determine it.

## Support status

**Not supported. Goose appears nowhere in the repository.** VERIFIED:

- `grep -rni goose` over the whole clone (excluding `.git` and `node_modules`) finds no match.
- The agent-type enum has no Goose member:
  `export type AgentType = 'codex' | 'claude-code' | 'opencode' | 'kimi-code' | 'custom'`
  (`xats:src/lib/agent-type.ts:1`).
- The README names native wakes for Claude Code, Codex and opencode (Kimi has a native HTTP transport in code,
  `dispatchKimi`, `xats:src/mcp/transport-dispatch.ts:110`, but the README never mentions it). Everything else ("cursor, an
  editor extension, your own harness") is filed as `custom`: it "connects over plain Streamable HTTP and
  registers as `agent_type="custom"` ... There is no dedicated wake-up transport for these; cross-agent pokes
  are delivered by injecting text into the agent's tmux pane" (`xats:README.md:259`, also `README.md:61`).
- `docs/configs/` holds only `claude-code.md`, `codex-cli.md`, `opencode.md` and `README.md`. There is no
  Goose config snippet.

The rest of this report asks whether the generic `custom` path, meaning an MCP Streamable HTTP extension
plus a tmux paste wake, would work under Goose, and how well. It uses the primer for Goose's behaviour.

**Verdict, INFERRED:** it mostly works for a `goose session` running in a real tmux pane.

- Tools work, and the idle wake usually works.
- No body is ever injected. Every message costs a model `get_inbox` call.
- The wake can corrupt a static human draft, or answer a Goose approval prompt.
- Inside a Herdr pane with no tmux around it, wake does not work at all: there is no tmux pane to bind, so
  you get mailbox-only delivery.

## Injection mechanism

### What gets into Goose's context, and how

There are only two paths, and neither uses a hook or any Goose-specific surface.

1. **A wake hint typed into the composer (tmux paste).** This is the only text that reaches the model
   without the model asking for it. The text is fixed (`xats:src/mcp/tools.ts:369-397`, VERIFIED):

   ```ts
   const render = (who: string, segment: string): string =>
     `新邮件 from ${who}${segment}, 请调 get_inbox 查看`
   ```

   With names filled in it reads
   `新邮件 from alice@team (<uuid>) → bob@team, 请调 get_inbox 查看` ("new mail from ..., please call
   get_inbox"). It is capped at `HINT_MAX_CHARS = 200` by dropping the target segment, then the sender's
   display name (`tools.ts:367`, `tools.ts:388-393`). The body is never in it: "Auto-poke injects only a SHORT
   wake-up hint ..., NOT the body" (`tools.ts:196`). In Goose the hint arrives as an ordinary user message,
   typed at the `> ` rustyline prompt and submitted with Enter (INFERRED from primer section 9).

2. **The model pulls the body through an MCP tool.** The model calls `get_inbox`. Goose exposes it as
   `<extension-name>__get_inbox`, because Goose prefixes extension tool names
   (`goose@1.36:crates/goose/src/agents/extension_manager.rs:1285`, `format!("{}__{}", name, tool.name)`,
   VERIFIED). The result is one MCP text block holding `JSON.stringify` of
   `{messages:[{id,event_id,from_team,to_team,from_agent_id,from_name,from_device,from_role,to_agent_id,to_role,subject,body,need_reply,sent_at}],has_more,last_event_id}`
   (`xats:src/mcp/tools.ts:101-103`, `xats:src/mcp/get-inbox.ts:8-25,49-75`, VERIFIED).

Paths that are not used, and why:

| Path | Used? | Why not | Tag |
|---|---|---|---|
| Hooks (UserPromptSubmit, Stop, ...) | No | xats ships no hooks for any harness | VERIFIED: file inventory, overview |
| MCP server notifications | No | xats only pushes over SSE to its Claude channel proxy (`subscribe_channel_wake` is reserved for `role=__channel_proxy__`, `tools.ts:3051-3054`). Goose would not surface such pushes to the model anyway: notifications reach only the UI, and only during a tool call (primer section 6) | VERIFIED (xats); VERIFIED (Goose, primer) |
| MCP sampling or elicitation | No | Not used by xats | INFERRED from the tool list (`tools.ts` `registerTool` names) |
| Goose MOIM / `tom` file | No | xats has no Goose code | VERIFIED (no Goose mention) |

### When it fires

- **Idle Goose:** the hint is submitted as a new user turn. The model reads it and (normally) calls
  `get_inbox` in that turn. INFERRED.
- **Busy Goose:** the guard usually stops the paste while the thinking spinner animates (see Wake). If the
  paste does land mid-turn, the bytes sit in the tty and become the next prompt after the turn ends. They
  are never a mid-turn injection: Goose's CLI does not read the terminal while busy (primer section 9,
  `goose@1.36:crates/goose-cli/src/session/mod.rs:1086-1092`). INFERRED.
- **Mid-turn body delivery:** none. The model sees bodies only when it chooses to call `get_inbox`. VERIFIED
  (no other path exists).

### Size bounds

| Item | Bound | Tag |
|---|---|---|
| Wake hint | 200 characters (`tools.ts:367`) | VERIFIED |
| Manual `poke` text | 8192 bytes (`poke.ts:102,274-275`) | VERIFIED |
| `get_inbox` page | `limit` default 50, maximum 200 messages (`get-inbox.ts:38`) | VERIFIED |
| Message body | No length cap: the schema is `body: z.string().min(1)` (`tools.ts:2835,2865,2913`) | VERIFIED |

Because bodies have no cap, a single `get_inbox` result can be 200 bodies of arbitrary size in one JSON
string. Goose has no truncation of its own for MCP tool results that I found (unknown). That is a real
context-blowup risk.

### Framing

- The hint is a bare Chinese/English sentence with no marker prefix. In a Goose transcript it is
  indistinguishable from the human typing it. INFERRED.
- Bodies arrive as raw JSON, with all metadata fields repeated per message. The JSON carries no
  trust-stance framing, and no per-delivery instructions beyond the tool descriptions.

### De-duplication and acknowledgement

- **Cursor.** A default `get_inbox` (no `since_event_id`) returns rows past
  `agents.last_processed_event_id`. It advances the cursor in the same SQLite transaction, so two concurrent
  calls cannot both see the same tail (`get-inbox.ts:44-75`, VERIFIED). Reading is the ACK.
- **Read receipt for the sender.** `send_message` blocks up to `await_ack_s` (default 10 s, maximum 30 s)
  and reports `ack.status = "read"` only if the recipient's own `get_inbox` returned the message
  (`tools.ts:166-178`, VERIFIED). This fits Goose's MCP tool timeout, which falls back to 300 s when the
  extension sets none (`goose@1.36:crates/goose/src/config/extensions.rs:11`,
  `goose@1.36:crates/goose/src/agents/extension_manager.rs:67`, VERIFIED).
- **No de-dup of hints.** Each send pastes its own hint. Five sends to an idle Goose produce five pasted
  user turns, unless the guard happens to fail on the later ones because the first wake set the spinner
  moving. INFERRED.
- The inbox is scoped to the caller's team: `WHERE m.to_team = ?` (`get-inbox.ts:49-58`, VERIFIED).

## Wake mechanism

### Exact mechanism

The call chain for a `custom` agent, VERIFIED:

- `dispatchPoke` → `resolveAgentType` → `dispatchUnknown` (`xats:src/mcp/transport-dispatch.ts:101-111`).
- `dispatchUnknown` uses the stored `tmux_pane_id`, or else returns `no_transport_available`
  (`transport-dispatch.ts:297-311`).
- That leads to `dispatchTmux` (`transport-dispatch.ts:118-155`) → `tmuxPokeImpl` (`xats:src/mcp/poke.ts:173-245`).

`dispatchTmux` runs step 1 itself (`transport-dispatch.ts:125-133`); `tmuxPokeImpl` first checks that tmux is on
PATH (`poke.ts:183-185`) and runs the rest:

1. `verifyPaneHost` (`xats:src/mcp/pane-host-verify.ts:68-111`). It returns REASSIGNED in any of these cases:
   - the row is on another device;
   - the DB no longer gives this agent the pane;
   - the bound pid is dead;
   - the pid is neither the pane's `pane_pid` nor on the pane's tty.

   If tmux cannot be queried, the result is "undecidable" and maps to `tmux_unavailable`. It never injects
   on unknown.
2. The quiet guard: `capture-pane -p -S -8`, sleep `POKE_QUIET_MS` (default 2000 ms), capture again, and
   require the two captures to be identical (`xats:src/mcp/poke-guard.ts:3-31`, `poke.ts:192-196`).
3. A synchronous ownership re-check (`poke.ts:201-203`).
4. `tmux load-buffer -b poke-<hex> -`, with the hint on stdin (`xats:src/daemon/tmux-cli.ts:30-45`).
5. A second ownership re-check. On failure the buffer is deleted (`poke.ts:217-222`).
6. `tmux paste-buffer -b <buf> -t <pane> -p -d` (`tmux-cli.ts:47-49`). The `-p` flag asks for bracketed paste
   if the application enabled it.
7. Sleep 400 ms (`PASTE_SETTLE_MS`), then a third ownership re-check (`ownership_lost`: text pasted, Enter not
   sent) (`poke.ts:224-233`).
8. `tmux send-keys -t <pane> Enter` (`tmux-cli.ts:51-53`), sleep 400 ms, then capture the tail for the
   caller's diagnostics (`poke.ts:234-237`).

The Codex-only `requireReady` composer allowlist (`›` prefix) is not applied to custom targets:
`dispatchTmux` never passes `requireReady` (`transport-dispatch.ts:134-141`, VERIFIED). So Goose gets only
the screen-quiescence guard, with no "is this the composer" check.

### Binding a Goose pane (a prerequisite for any wake)

- `register_agent` with `agent_type:"custom"` does **not** auto-bind a pane, even when `ui_pid` is given.
  `inferRuntimeAgentKind` returns `undefined` for `custom`, so `autoBindRuntimeIdentity` returns false
  (`tools.ts:473-477`, `tools.ts:1026-1027`, VERIFIED). The register reply then carries the hint
  "No usable tmux_pane_id is bound yet ... Call `bind_runtime_identity(...)`" (`tools.ts:1752-1756`).
- This contradicts the README's claim that for custom agents "the daemon will resolve `pid → tty → pane`
  automatically when you register" (`README.md:259`). The code needs a second, model-driven call. VERIFIED
  that both texts exist; the contradiction is INFERRED from reading both.
- The model must therefore call `bind_runtime_identity({agent:"custom", process_pattern:"goose", ui_pid:<pid>})`.
  `process_pattern` is mandatory for custom (`tools.ts:129-155`, VERIFIED). The daemon then:
  - runs `ps -p <pid> -o tty=,command=` (`xats:src/daemon/pid-tty.ts:14-24`);
  - tests the command against the case-insensitive regex;
  - finds the single tmux pane whose tty equals the pid's tty (`xats:src/daemon/runtime-identity.ts:47-52,116-177`).
- What pid would Goose's model pass? From Goose's shell tool, `$PPID` is the `goose` process itself. Goose
  spawns `$SHELL -c <cmd>` directly (`goose@1.36:crates/goose/src/agents/platform_extensions/developer/shell.rs:644-653`)
  except under Flatpak, where `flatpak-spawn --host` sits in between (`shell.rs:631-643`). `ps` on that pid
  shows `goose session ...` with the pane's tty, so the pid → tty → pane binding should succeed. INFERRED,
  because nothing here was run.
- The alternative `ui_tty` plus `tmux_pane_id` pair (`runtime-identity.ts:179-200`) also works, because the
  shell tool inherits `$TMUX_PANE` (primer section 11: the env is not cleared). INFERRED.

### Idle/busy detection

Busy detection is screen quiescence only, over the last 8 lines across 2 s. VERIFIED.

How that maps onto Goose's screens (INFERRED from the primer and Goose source):

- **Idle `> ` prompt with the dim hint `Enter to send · Ctrl+J newline`** (primer section 9): static, so the
  guard passes. That is correct.
- **Model streaming or the thinking spinner:** the spinner is a `cliclack::spinner()` with a
  random thinking message (or `Thinking...` when `RANDOM_THINKING_MESSAGES` is false) followed by a dim
  `(Ctrl+C to interrupt)` (`goose@1.36:crates/goose-cli/src/session/output.rs:118-149`).
  It animates, so the guard fails, which is also correct.
- **A long silent tool call:** for example a shell command that prints nothing for more than 2 s, if Goose
  shows no spinner during tool execution (unknown; spinner show/hide sites are
  `output.rs:251-255,333-338`, `mod.rs:700-704`). The screen is static, so the guard passes while Goose is
  busy. The hint plus Enter then sits in the tty buffer and becomes the next user message after the turn.
  The busy agent loses nothing, because the shell tool's stdin is `Stdio::null()`
  (`goose@1.36:.../developer/shell.rs:516`, VERIFIED), so a running command cannot swallow the bytes. But
  cooked-mode echo prints the hint into the middle of the output, and the "wake" is really a deferred
  prompt. INFERRED.
- **Approval prompt** (`GOOSE_MODE=approve|smart_approve`) **or an MCP elicitation form** (primer section 9):
  these are static, so the guard passes. The pasted text and Enter are delivered to a `cliclack` select or
  form. Enter most likely accepts the highlighted option, which could approve a tool call the human had not
  decided on. INFERRED hazard; the exact select behaviour is unknown.

### Draft safety

There is none for Goose.

- A human's half-typed line at `> ` that has not changed for 2 s passes the guard.
- The hint is then appended to the draft at the cursor, and Enter submits draft plus hint as one message.
  INFERRED from `poke-guard.ts` (it compares screens and does not inspect the composer) and rustyline
  semantics.
- xats never sends Ctrl+C, which is good: in Goose, Ctrl+C clears the buffer, and a second Ctrl+C exits
  (primer section 9). VERIFIED that the code sends only paste and `Enter` (`tmux-cli.ts:47-53`).
- `GOOSE_PROMPT_EDITOR` mode (an `$EDITOR` per prompt) would send the paste into vim or nano instead.
  INFERRED.

### Latency

- Best case is about 2.8 s from send to Enter: the 2 s guard, plus the 400 ms paste settle, plus tmux exec
  overhead (`poke-guard.ts:3`, `poke.ts:103`). Then add one model turn before `get_inbox` runs. VERIFIED for
  the constants; the total is INFERRED.
- If the guard fails, retries come at 30 s, 180 s and 600 s, after which the wake is abandoned. The delays are
  `RETRY_DELAYS_MS` (`xats:src/mcp/poke-retry.ts:1`), each scheduled after the previous tick (`poke-retry.ts:79-88`).
  A retry stops early once the recipient's cursor passes the message, or once the recipient's `last_seen_at`
  (refreshed by any xats tool call) is newer than the send (`recipient_active`), or on a terminal reason
  (`pane_reassigned`, `no_pane`, `tmux_unavailable`) (`poke-retry.ts:61-65,105-135,165-175`,
  `xats:src/mcp/auto-poke-fanout.ts:158-176`). Retries
  skip the guard inside the poke (`skipGuard: true`) because the scheduler ran `paneGuardFn` first
  (`auto-poke-fanout.ts:168-172`, `poke-retry.ts:136`). VERIFIED.
- So a Goose that is busy for more than about 13.5 minutes gets no wake for that message. INFERRED.

### Failure modes

| Failure | Outcome | Tag |
|---|---|---|
| No tmux, for example a Goose pane in Herdr without tmux | `bind_runtime_identity` → `tmux_unavailable` or `tmux_pane_not_found`; every send reports `no_pane` / `no_transport_available`; mailbox only | VERIFIED (`runtime-identity.ts:124-132,149`, `transport-dispatch.ts:303-310`); the Herdr case is INFERRED |
| Model skips `bind_runtime_identity` | Same as the row above (the register reply only *hints* at the call) | VERIFIED |
| Goose restarted in the same pane | The bound `runtime_ui_pid` is dead → `pane_reassigned`, which is never retried; the model must re-register and re-bind | VERIFIED (`pane-host-verify.ts:101-102`, `tools.ts:227`, `poke-retry.ts:61-65`) |
| Several tmux panes share a tty | `ambiguous_tty_match` | VERIFIED (`runtime-identity.ts:150-158`) |
| Hint lands while a Goose approval or elicitation form is open | It answers the prompt | INFERRED |
| Remote device | Never injected (`row.device !== localDevice` → REASSIGNED) | VERIFIED (`pane-host-verify.ts:70`) |
| MCP session replaced | `register_agent` from a second MCP session force-closes the earlier one. If Goose reconnects the Streamable HTTP session (for example after a daemon restart), the model must call `reconnect` | VERIFIED (`README.md:267`); Goose's reconnect behaviour is unknown |

## Setup and installation footprint on Goose

xats writes nothing Goose-specific. VERIFIED: there is no Goose entry in `mcpsmgr.json` or `docs/configs/`.
The user must:

1. Start the daemon: `npx cross-agent-teams-mcp daemon --port 9100`. It keeps its data in
   `~/.cross-agent-teams-mcp/data.db` (`README.md:76-93`, VERIFIED).
2. Add a Streamable HTTP extension to Goose by hand, either per session with
   `goose session --with-streamable-http-extension http://127.0.0.1:9100/mcp` (primer section 2), or
   persistently in Goose's `config.yaml` as a `type: streamable_http` extension with `uri`, optional `headers`
   for the bearer token and `timeout` (schema `goose@1.36:crates/goose/src/agents/extension.rs:220-237`,
   VERIFIED; the exact YAML layout is INFERRED). A `--token` daemon needs
   `headers: {Authorization: "Bearer <t>"}`, or `?token=` in the URI (`README.md:269-271`).
3. Run Goose inside a tmux pane.
4. Tell the model "Register me to xats as bob". It calls `register_agent{agent_type:"custom",
   agent_type_name:"goose", ...}` and then `bind_runtime_identity{agent:"custom", process_pattern:"goose",
   ui_pid:$PPID}` (`tools.ts:1164,1223-1228`, VERIFIED for the fields).

There are no Goose hooks, plugins, `.agents/plugins` entries or MOIM settings. That is low footprint and
fully reversible, but it is manual and model-dependent.

## Token cost

- **Per message, recipient side, INFERRED estimate:**
  - The hint is about 60-120 characters as a user turn.
  - Then one assistant tool call plus a JSON result. The result is about 300 bytes of metadata per message
    plus the body.
  - That is at least one extra model round trip per wake, more than herdr-threads' in-turn delivery would
    need.
- **Per message, sender side:** `send_message` returns poke diagnostics and an ACK object. It also holds the
  sender's turn for up to 10 s by default. VERIFIED for the wait (`tools.ts:166`).
- **Per quiet turn:** nothing is injected (VERIFIED: no hooks). But every provider call carries xats' tool
  schemas. The tool descriptions are unusually long: the shared description constants alone
  (`tools.ts:173-360`) are about 18.5 KB of source, `register_agent`'s description (`tools.ts:1873-1930`) is
  about 12.8 KB, and `tools.ts` registers 17 tools (`registerTool` calls). My estimate is roughly 30-50 KB,
  about 8-12 k tokens, of tool schema in every Goose request for as long as the extension is enabled.
  INFERRED estimate from source sizes; the daemon was not run to measure `tools/list`. Prompt caching
  softens this on providers that support it.

## Limitations and bugs visible for Goose

1. **No Goose support at all.** Goose falls into `custom`, so it never gets the native transports.
2. **README versus code on custom binding.** The README promises an automatic pid → tty → pane bind at
   registration. The code skips the auto-bind for `custom` (`tools.ts:477`). Waking a Goose therefore
   depends on the model making a second correct call with a regex it has to invent.
3. **tmux-only wake.** A Goose in a Herdr pane, which is our deployment, would get no wakes, because the
   daemon looks only at tmux panes (`runtime-identity.ts:124-132`). INFERRED for Herdr.
4. **Draft clobbering.** The quiet guard tests "unchanged", not "empty". For Goose there is no composer
   reader (the Codex `›` allowlist is Codex-only), so a static draft gets the hint appended and then
   submitted.
5. **It can answer interactive prompts.** Goose's approval selects and elicitation forms are static screens
   that pass the guard.
6. **No priority or threshold.** Every default send wakes the recipient unless the sender passes
   `auto_poke:false`. One wake per send, with no coalescing.
7. **Unbounded bodies** in a `get_inbox` page (up to 200 messages, with no per-body cap).
8. **Fixed hint language.** The hint is Chinese with an English tool name. It is not configurable, and it
   is model-visible in every wake.
9. **No in-turn delivery.** Goose's MOIM file (primer section 5) and the Stop-hook block on main (primer
   section 3.4) both go unused. A busy Goose sees a message only on its next `get_inbox`, or after its turn
   ends and a retry fires.
10. **A busy-but-static screen** (a long silent tool call) lets the guard pass, which leaves a deferred
    prompt plus echo garbage in the output. INFERRED.

## Clever ideas worth stealing

1. **The ownership re-check is synchronous with the write, at every step** (before load, before paste,
   before Enter), with distinct outcomes:
   - `pane_reassigned`: nothing was written;
   - `ownership_lost`: text was pasted but Enter was never sent.

   `pokeWroteContent()` centralises "did this leave text in the pane" (`poke.ts:197-245,247-269`). For a
   Goose Native wake in herdr-threads, the same three checkpoints map cleanly onto Herdr's pane identity.
2. **Named, self-deleting paste buffers** (`paste-buffer -d`, plus best-effort deletion on abort,
   `poke.ts:146-169`), so a failed wake never leaves text parked in a buffer.
3. **Never inject when ownership is undecidable.** An unqueryable multiplexer maps to `tmux_unavailable`
   (`pane-host-verify.ts:37-40`, `transport-dispatch.ts:128-131`). That matches our "client hints are never
   authority" stance.
4. **A real read receipt in the send reply.** `send_message` holds up to 10 s and reports whether the
   recipient actually read the message (`ack.status`). Its tool description also tells the sender in plain
   words not to act on `not_yet` (`tools.ts:173-178`). The guidance wording about not taking over the
   recipient's work is a useful pattern for our skill text.
5. **Retry wakes stop on read.** Retries check the cursor (`alreadyReadFn`) before every attempt
   (`auto-poke-fanout.ts:141,167`). That is cheap, and it prevents a stale wake into a session that already
   caught up.
6. **The hint names its addressee** (`→ name@team`, `tools.ts:196`), so a wrong pane that receives a wake
   can tell it was not the target. For us this matters only if a wake ever carries content. Our wake marker
   is content-free by design (R18), so the idea transfers only as a diagnostic field in the daemon's own
   wake log.

What herdr-threads should do differently for Goose (INFERRED, for the comparison phase):

- Read Goose's composer: empty means the `> ` row with the dim `Enter to send` hint. Refuse wakes when the
  composer is non-empty, or when a cliclack select or form is on screen.
- Use the `tom` MOIM file for in-turn delivery, so a busy Goose gets bodies after each tool batch with no
  tool call.

## Verification

Checked against the xats clone at `17df5cf7` and the `goose-1.36` tree: the Goose grep, agent-type enum,
`docs/configs/` listing, README 61/259/267/269 and 76-93, hint builder and cap, `toText`, `get-inbox.ts`
(fields, limit, transaction, team scope), body schemas, `AWAIT_ACK_*`, `bindRuntimeIdentity` schema and
`process_pattern` rule, `inferRuntimeAgentKind`/auto-bind, the register hint, `dispatchPoke`/`dispatchUnknown`/
`dispatchTmux`, `tmuxPokeImpl` steps and ownership checks, `poke-guard.ts`, `tmux-cli.ts`, `pane-host-verify.ts`,
`runtime-identity.ts`, `auto-poke-fanout.ts`, `poke-retry.ts`, `requireReady` (only Codex callers pass it);
Goose tool-name prefix, extension timeout, `StreamableHttp` schema, shell spawn and `Stdio::null()`, spinner,
and primer sections 3.4, 5, 6, 9 and 11.

Changed:
- Kimi is not in the README; it is native only in code.
- `verifyPaneHost` runs in `dispatchTmux`, not `tmuxPokeImpl`.
- The spinner text is a random thinking message by default, not always `Thinking...`.
- Retries also stop on `recipient_active` (any xats call after the send) and on terminal reasons; cited `poke-retry.ts`.
- Tool count is 17, not 20+.
- Fixed citations: hint builder range, `poke` 8192-byte check, `ps` call (`pid-tty.ts`), "never retried" (`tools.ts:227`),
  Goose 300 s fallback (`extension_manager.rs:67`).
