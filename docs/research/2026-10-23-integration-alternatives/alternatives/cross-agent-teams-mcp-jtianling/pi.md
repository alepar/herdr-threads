# cross-agent-teams-mcp (jtianling, "xats") x Pi coding agent

- Alternative: https://github.com/jtianling/cross-agent-teams-mcp, clone at
  `17df5cf7bd9b66f00460eff05080b0e0495b6ec8`. Paths without a prefix are relative to that clone.
- Harness: Pi (https://github.com/badlogic/pi-mono, now presented as earendil-works/pi). Paths prefixed
  `pi:` are relative to the pi-mono clone at `a7b94a03f09f20f4937fa1a76339014c60d261ad`, as read for
  `../../primers/pi.md`.
- Code was read, never run. VERIFIED = read in source or docs at the cited line. INFERRED = my reading.

## Support status

**Not supported. Pi is never named.** Pi can still use xats as a generic `custom` MCP client. In that
mode it wakes only when Pi runs inside tmux, and only after the model makes an extra tool call.

- The repo never mentions Pi. A case-insensitive search for `pi-mono`, `badlogic`, `pi-coding`,
  `earendil`, `.pi/` or the whole word `pi` in `*.md`/`*.ts` (outside `node_modules`) finds nothing. The
  only match for the broader pattern is a hash inside `pnpm-lock.yaml`. VERIFIED.
- The closed agent-type enum has no Pi member:
  `export type AgentType = 'codex' | 'claude-code' | 'opencode' | 'kimi-code' | 'custom'`
  (`src/lib/agent-type.ts:1`, and the zod copy at `src/mcp/tools.ts:109`). VERIFIED.
- `mcpsmgr.json:36-110` has installer entries only for `claude-code`, `codex`, `opencode` and `cursor`.
  There is no Pi entry, so `npx mcpsmgr add ...` cannot configure Pi. VERIFIED.
- Under the README, every other harness counts as "Other coding agents (cursor, ...)". It connects over
  Streamable HTTP, registers as `agent_type="custom"` and is woken by text injected into its tmux pane
  (`README.md:257-259`). VERIFIED. This generic path is the whole of the Pi story below.

Can Pi be an MCP client of xats at all? Yes. Pi ships a built-in MCP client that speaks streamable HTTP
(pi: `packages/coding-agent/docs/mcp.md:64,78`). It reads `~/.pi/agent/mcp.json` or `.pi/mcp.json`
(mcp.md:32) and has a `pi mcp add <name> --url <url>` command (pi: `docs/cli.md:325`). xats serves
Streamable HTTP at `http://127.0.0.1:9100/mcp`. Connecting therefore works; VERIFIED for both halves,
the combination INFERRED. The mapper's "unknown" can be resolved to **generic, degraded**.

## Injection mechanism

### What reaches Pi's context

xats never injects a message body into any harness. It only types a wake hint. The model has to fetch
bodies itself with the `get_inbox` MCP tool. VERIFIED: `src/mcp/tools.ts:196` ("Auto-poke injects only
a SHORT wake-up hint ... NOT the body — read bodies via get_inbox").

For a `custom` (Pi) row, the hint takes this path:

1. `send_message` writes the mailbox row, then auto-pokes.
   `dispatchPoke` routes on the effective agent type, and `custom` falls through to
   `dispatchUnknown` (`src/mcp/transport-dispatch.ts:101-112`). VERIFIED.
2. `dispatchUnknown` uses `target.tmux_pane_id` if one is bound. Otherwise it returns
   `no_transport_available` with `{channel_subscribed:false, tmux_pane_set:false}`
   (`transport-dispatch.ts:297-311`). VERIFIED.
3. `dispatchTmux` checks pane ownership (`verifyPaneHost`, `src/mcp/pane-host-verify.ts:68-111`).
   It then calls `tmuxPokeImpl` with no `requireReady` predicate (`transport-dispatch.ts:118-155`).
   The `›` composer allowlist is wired only for Codex. VERIFIED.
4. `tmuxPokeImpl` (`src/mcp/poke.ts:173-245`) runs these steps:
   - the quiet guard;
   - an ownership re-check;
   - `capture-pane`;
   - `tmux load-buffer -b poke-<hex> -`;
   - another ownership re-check;
   - `tmux paste-buffer -b <buf> -t <pane> -p -d`, a bracketed paste
     (`src/daemon/tmux-cli.ts:47-49`);
   - a 400 ms settle, then another ownership re-check;
   - `tmux send-keys -t <pane> Enter` (`tmux-cli.ts:51-53`);
   - a second 400 ms settle and a closing `capture-pane`.

   VERIFIED.

On the Pi side, bracketed paste is enabled (pi: `packages/tui/src/terminal.ts:211-212`). A one-line hint
of 200 characters or fewer stays under the 10-line / 1000-character threshold, so it is not collapsed
to a `[paste #N]` marker (pi: `packages/tui/src/components/editor.ts:1298-1316`). Enter submits the
hint as a normal **user message** (pi: `docs/usage.md:11-40`). INFERRED from the Pi sources, not
observed.

### Payload and framing

```ts
// src/mcp/tools.ts:388-396
const render = (who: string, segment: string): string =>
  `新邮件 from ${who}${segment}, 请调 get_inbox 查看`
for (const candidate of [render(sender, to), render(sender, ''), render(fromAgentId.slice(0, 8), '')]) {
  if (candidate.length <= HINT_MAX_CHARS) return candidate
}
```

- Shape: `新邮件 from alice@backend (<uuid>) → bob@backend, 请调 get_inbox 查看`. That is a Chinese
  "new mail ... please call get_inbox", hard-capped at `HINT_MAX_CHARS = 200` characters by dropping
  segments (`tools.ts:367-397`). VERIFIED.
- No marker distinguishes the hint from human input. In Pi it is indistinguishable from a typed prompt
  and is persisted in the session JSONL as a user turn. INFERRED.
- The manual `poke` tool can type arbitrary text of up to 8192 bytes into a same-team peer's pane
  (`src/mcp/poke.ts:102,271-293`, per the overview). VERIFIED. Into Pi, a multi-line poke over 10 lines
  or 1000 characters becomes a collapsed paste marker. It is expanded on submit, so the model still
  receives the full text. INFERRED from pi editor.ts:1092-1093,1298-1316.

### Body pull: `get_inbox`

- The output is JSON-stringified rows (`toText`, `tools.ts:101-103`). Each row has these columns: `id`,
  `event_id`, `from_team`, `to_team`, `from_agent_id`, `to_agent_id`, `to_role`, `subject`, `body`,
  `need_reply`, `sent_at`, `from_role`, `from_name`, `from_device`. Each page also carries `has_more`
  and `last_event_id` (`src/mcp/get-inbox.ts:48-76`). VERIFIED.
- Bounds:
  - The page holds 50 messages by default and at most 200 (`get-inbox.ts:38`). VERIFIED.
  - The schema caps neither message count per sender nor body size. `body: z.string().min(1)` is the
    only constraint (`tools.ts:2835,2865,2913`). VERIFIED.
  - The only byte bound is on the Pi side: MCP text results over 20 KB have their middle removed, and
    the full text goes to a temp file (pi: `docs/mcp.md:232`). VERIFIED.
- When it fires: only when the model calls the tool. Nothing arrives at turn start or after a tool
  batch. VERIFIED: the repo has no hooks and no Pi extension (overview).
- Dedupe and ack:
  - A default `get_inbox` advances the server-side cursor `agents.last_processed_event_id` in the same
    SQLite transaction as the read (`get-inbox.ts:44-76`). An explicit `since_event_id` is a
    non-advancing peek. VERIFIED.
  - The cursor advance doubles as the read receipt: `send_message` blocks up to `await_ack_s` (default
    10, max 30) and reports `ack.status:"read"` once the recipient's `get_inbox` returned the message
    (`tools.ts:166-178`). VERIFIED.
  - Consequence for Pi (INFERRED): Pi's 20 KB middle-cut means a large page is acked as read even though
    the model saw only its head and tail. The rest sits in a temp file that the model may never open.
- Ordering: `ORDER BY m.event_id ASC` (`get-inbox.ts:58`). VERIFIED. There is no priority field.

### How Pi exposes the tools (Pi-specific)

- Pi's default MCP exposure is `codemode`, under which tools are "neither declared to the model nor
  listed in the codemode description" (pi: `docs/mcp.md:195`). VERIFIED.
- Under `codemode`, the model learns of the server only from a `mcp_servers` system-prompt line built
  from the first line of the server `instructions` (pi: `src/extensions/mcp/index.ts:184-188`), truncated
  to at most `MAX_SERVER_DESCRIPTION_CHARS = 250` characters (index.ts:158, 217). VERIFIED.
- xats's `instructions` are one roughly 3,000-4,000-byte line with no newline, mostly Chinese, about
  naming and registration (`src/mcp/transport.ts:236-258`). VERIFIED. Because of the 250-character cut,
  codemode shows only its opening (the Chinese "xats is the abbreviation of cross-agent-teams..."
  sentence); the registration rules never reach the system prompt. A codemode script can still read the
  full text with `describeNamespace("mcp__<server>")` (pi: `docs/mcp.md:234`). INFERRED from the code.
- In codemode the model reaches `get_inbox` only by writing a codemode script that calls
  `mcp__cross_agent_teams__get_inbox` (Pi replaces `-` with `_` in tool names, pi: `docs/mcp.md:77`)
  after `searchTools()`/`describeTool()`. INFERRED.
- The per-tool rules (DETECTION order, REPLY RULE, ack semantics) live only in tool descriptions, which
  codemode hides. The model would miss them unless it calls `describeTool()`. INFERRED.
- Practical setup therefore needs `"exposure": "direct"` (pi mcp.md:197, `pi mcp add --exposure`,
  cli.md:331). That declares all 17 xats tools on every request. VERIFIED that there are 17
  `registerTool` calls in `tools.ts`; the exposure effect is INFERRED.

## Wake mechanism

### Exact mechanism

The mechanism is a tmux paste of the hint followed by Enter (as above). No Pi-native wake path exists.

- xats ships no Pi extension, so it cannot use `pi.sendMessage(..., {triggerTurn:true})`.
- It has no RPC client either.
- Pi's MCP client handles only `notifications/message` (logged to a file) and `tools/list_changed`
  (pi: `src/extensions/mcp/runtime.ts:367,380-382`; primer §3). VERIFIED.

So even the daemon's SSE `notifications/channel_wake` stream, which the Claude proxy consumes, could not
wake Pi. INFERRED.

### Binding the pane (the precondition, and the weakest link)

- The README says the daemon resolves `pid → tty → pane` automatically at registration
  (`README.md:259`). **For `custom` that is not what the code does.**
  - `inferRuntimeAgentKind` returns `undefined` for `agent_type === 'custom'` (`tools.ts:477`).
  - `autoBindRuntimeIdentity` therefore returns `false` before it looks at `ui_pid`
    (`tools.ts:1026-1027`).
  - The register response instead carries a `hint`: "No usable tmux_pane_id is bound yet ... Call
    `bind_runtime_identity(...)`" (`tools.ts:1755`).

  VERIFIED.
- The Pi model must then make a second call:
  `bind_runtime_identity({agent:"custom", process_pattern:"<regex>", ui_pid:<pid>})`.
  `process_pattern` is mandatory for `custom` (`tools.ts:129-157`, `src/daemon/runtime-identity.ts:47-52`).
  The daemon then checks three things:
  - `ps -p <pid> -o tty=,command=` (`src/daemon/pid-tty.ts:22`, called from
    `runtime-identity.ts:138`) matches the regex (`runtime-identity.ts:138-146`);
  - the pid has a tty;
  - exactly one `tmux list-panes -a` row has that tty (`runtime-identity.ts:147-176`).

  VERIFIED.
- Getting `ui_pid` from Pi:
  - Pi's `bash` tool spawns the shell directly from the Pi node process (`detached: true`;
    pi: `src/core/tools/bash.ts:112-117`), so `$PPID` inside the tool is the Pi process. INFERRED.
  - Pi sets `process.title = "pi"` (pi: `src/cli/setup.ts:5`, `src/config.ts:577`), so
    `process_pattern: "^pi$"` would match on Linux. INFERRED; macOS `ps` command output may differ.
  - The detached shell has no controlling tty, but the check reads the tty of `$PPID`, not of the shell.
    INFERRED.
- Nothing tells a Pi model to do any of this. The DETECTION list in `register_agent` falls through to
  `custom` with `agent_type_name` (`tools.ts:1894`). The `bind_runtime_identity` description says
  "use `custom` together with `process_pattern`" (`tools.ts:3008`). VERIFIED. Whether a given model
  completes the chain is unknown.

### Idle/busy detection

- Busy detection is screen quiescence only. `runQuietGuard` captures the pane, sleeps
  `POKE_QUIET_MS` (default 2000 ms), captures again, and fails on any difference
  (`src/mcp/poke-guard.ts:3-31`). VERIFIED.
- The capture is `tmux capture-pane -t <pane> -p -S -8` with no `-E` (`tmux-cli.ts:21-27`). VERIFIED.
  Per tmux(1) semantics this is the **whole visible screen plus up to 8 history lines**, not "the last
  8 lines" as the names `GUARD_TAIL_LINES`/`TAIL_LINES` suggest. INFERRED.
  Pi's default fullscreen TUI runs on the alternate screen (pi: `docs/cli.md:233-234`), so in practice
  the guard compares the full visible frame. INFERRED.
- Effect on a busy Pi:
  - While Pi streams or shows its working indicator, the frame changes, so the guard fails with
    `guard_failed`. The daemon retries at 30 s, 180 s and 600 s (`src/mcp/poke-retry.ts:1-2`) and stops
    once the cursor passes the message (`alreadyReadFn`, `src/mcp/auto-poke-fanout.ts:129-178`). VERIFIED.
  - If a busy Pi happens to be visually static for 2 s (for example, a long silent tool with no
    animated indicator; whether Pi always animates is unknown), the paste lands and Enter is pressed.
  - Pi turns Enter-while-busy into a **steer**, delivered after the current tool batch
    (pi: `src/modes/interactive/interactive-mode.ts:3333-3338`; primer §6). The hint then reaches the
    model mid-run instead of being lost. INFERRED.
- Herdr-threads' rule 4 ("never nudge a busy session") is therefore not guaranteed, but the failure
  mode is mild with Pi. INFERRED.
- xats does not use Pi's real idle signals: `agent_settled`, `ctx.isIdle()`, OSC 7501
  `working`/`idle`. VERIFIED by absence.

### Draft safety

None for Pi. The guard detects motion, not a non-empty composer.

- A human who has typed `refactor the` and paused for 2 s has a static screen. The hint is
  bracket-pasted at the cursor and Enter submits `refactor the新邮件 from ...` as one prompt. INFERRED
  from poke.ts:191-234 and Pi editor insertion.
- The Codex-only `›` readiness allowlist (`src/mcp/codex-prompt-readiness.ts:21-29`) is not applied to
  `custom`. VERIFIED.
- Pi offers exactly the primitive needed (`ctx.ui.getEditorText()`, pi: `src/core/extensions/types.ts:230-237`),
  but only from inside an extension.

### Latency

- Best case: about 2.8 s of daemon-side work (2 s guard, plus 400 ms, plus 400 ms) before Enter. Then
  one model turn whose first action must be a `get_inbox` call, so the body arrives one model
  round-trip after the wake. INFERRED from the constants in poke-guard.ts:3 and poke.ts.
- Busy case: 30 s, 180 s or 600 s. After the 600 s retry nothing else happens. The message waits for
  whenever the model next calls `get_inbox` on its own, which Pi never does unprompted. INFERRED from
  the retry schedule.
- The unread watchdog pokes the *sender* after 15 minutes for `need_reply` mail
  (`src/mcp/unread-watchdog.ts:6,31`). VERIFIED.

### Failure modes (Pi-specific)

| Situation | Result | Tag |
|---|---|---|
| Pi runs in a Herdr pane, not tmux | `bind_runtime_identity` fails (`tmux_unavailable` / `tmux_pane_not_found`, runtime-identity.ts:124-149). Every send then reports `no_transport_available`. Mail is stored and never woken. | VERIFIED code path; Herdr not being tmux is INFERRED |
| Model registers but never calls `bind_runtime_identity` | Same: no pane, no wake | VERIFIED (tools.ts:1755) |
| Pi's MCP session expires or the daemon restarts | Pi silently retries on a **new** MCP session (pi: `src/extensions/mcp/runtime.ts:288-294`). xats bindings are per-connection, so every call returns `unknown_agent` (tools.ts:662-670, 697-705). | VERIFIED both halves |
| Recovery after that | The hint lists `reconnect` keys for kimi, opencode, codex and claude only. `reconnect({ui_pid})` is documented as a Claude lookup (`src/mcp/reconnect.ts:35-60`). It queries `runtime_ui_pid` without a type filter, so it might work for a Pi row whose pid was stored; unknown. Otherwise the model re-registers with the same `(team,name)`, which reuses the `agent_id` and repeats the bind dance. | VERIFIED / unknown |
| `/new`, `/resume`, `/fork`, `/reload` in Pi | The built-in MCP extension closes its connections on `session_shutdown` (pi: `src/extensions/mcp/index.ts:1252-1258`), so the binding is lost as above. The tmux pane binding survives in the DB, because the Pi pid is unchanged. | VERIFIED / INFERRED |
| Pi restarted in the same pane | New pid, so `verifyPaneHost` sees a dead `runtime_ui_pid` and returns `pane_reassigned` (pane-host-verify.ts:101-102). No wake until rebind. | VERIFIED |
| Human draft present and static | Hint is merged into the draft and submitted | INFERRED |
| Pi in compaction | Pi queues Enter-input during compaction as a steer (pi: interactive-mode.ts:3321-3328). Benign. | INFERRED |

## Setup and installation footprint on Pi

- The user runs the daemon: `npx -y cross-agent-teams-mcp daemon --port 9100` (`mcpsmgr.json:26-34`).
  It writes `~/.cross-agent-teams-mcp/data.db` (overview). VERIFIED.
- Pi configuration is manual, with no installer support:
  `pi mcp add cross-agent-teams --url http://127.0.0.1:9100/mcp --exposure direct`. This writes
  `~/.pi/agent/mcp.json`, or `.pi/mcp.json` with `-l` (pi: `docs/cli.md:325,331`). VERIFIED Pi
  behaviour; the exact entry is my recommendation. If the daemon runs with `--token`, add
  `--bearer-token-env-var CROSS_AGENT_TEAMS_MCP_TOKEN` (`mcpsmgr.json:17-24`).
- Nothing else is written into Pi: no extension, no settings, no hooks.
- Run Pi inside tmux, which is mandatory for any wake.
- Per session, the human says "register me to xats as bob". The model then makes three calls:
  1. `register_agent({agent_type:"custom", agent_type_name:"pi", name, project_dir, ui_pid:$PPID})`
     (it first has to run the `printenv` probes the description demands);
  2. `bind_runtime_identity(...)`;
  3. `get_inbox` on each wake.

  VERIFIED tool contracts.

## Token cost

- **Per quiet turn:**
  - The schema cost is the 17 xats tool declarations on every request with `direct` exposure. The
    description strings in `tools.ts` total about 36 KB (crude regex sum of description arrays) plus
    about 2 KB of `.describe()` text, roughly 9-10 k tokens. They are prompt-cached after the first
    request. INFERRED estimate from measuring the source.
  - With `codemode`, the cost is instead one `mcp_servers` line of at most ~250 characters of the
    `instructions` in the system prompt (pi index.ts:158, 217), plus codemode overhead on use. VERIFIED
    cap; cost INFERRED.
  - No per-turn injection happens.
- **Per message (idle recipient):**
  - The hint, at most 200 characters (~30-60 tokens).
  - One wake turn, whose first assistant output must be a `get_inbox` tool call.
  - The `get_inbox` JSON: per message, about 14 fields including two or three UUIDs and team names,
    roughly 350-500 bytes of envelope plus the body.
  - The model's reply, which is usually a `send_message` call. That call blocks up to 10 s for an ack
    and returns `poked`/`poke_skip_reasons`/`ack` JSON.

  So there is at least one extra model round-trip per wake compared with in-turn body delivery.
  INFERRED from the schemas.
- **Busy recipient:** zero injection while the guard fails. If a retry lands mid-run it costs the same
  as above, but inside the current run as a steer. INFERRED.

## Limitations and bugs visible from the source

1. **No Pi wake outside tmux.** Herdr users get mailbox-only behaviour. VERIFIED (tmux-only path).
2. **The README overstates auto-binding for `custom`.** README.md:259 promises automatic
   `pid → tty → pane` at register. The code skips custom (tools.ts:477, 1026-1027), so a second,
   model-driven `bind_runtime_identity` with a hand-written regex is required. VERIFIED.
3. **The guard window is mislabelled.** `capture-pane -S -8` without `-E` captures the full screen plus
   history (tmux-cli.ts:24), so any change anywhere on screen (a clock, a spinner in another region)
   fails the guard. That is safe but makes it fire `guard_failed` more often. INFERRED from tmux(1).
4. **No draft protection** for `custom` panes (no readiness predicate, transport-dispatch.ts:134-141).
   INFERRED consequence.
5. **The ack is "displayed", not "seen".** Combined with Pi's 20 KB middle-cut of MCP results, a large
   `get_inbox` page is acked although its middle never reached the model. INFERRED.
6. **Bindings are per MCP connection.** Pi's transparent session-expiry retry and its MCP reconnect on
   `/new`/`/reload` both detach the seat silently, and the recovery hint offers no Pi key. VERIFIED
   (tools.ts:662-670; pi runtime.ts:288-294, index.ts:1252-1258).
7. **Big model-visible schema.** About 38 KB of tool prose, partly Chinese, is attached to every
   request. It also carries harness-specific rules (kimi, codex, claude) that are irrelevant to Pi.
   VERIFIED size estimate (crude).
8. **No priority.** Every send wakes the recipient unless the sender sets `auto_poke:false`. VERIFIED
   (overview).
9. **Unbounded body size per message.** No schema cap was found. VERIFIED by reading the schemas.
10. **`process_pattern` is compiled from model input** with `new RegExp(raw, 'i')`
    (runtime-identity.ts:51). In a cooperative model that is fine, but an unanchored `pi` matches
    `pip`, `api` and so on. With `ui_pid` only the pid's own command is tested, which limits the damage.
    VERIFIED code; INFERRED impact.

## Clever ideas worth stealing (and what the Pi primer says we should do instead)

- **A delivery receipt as a side effect of the read.** The transactional cursor advance on read plus a
  sender-side `await_ack_s` short block (tools.ts:166-178) gives the sender a "read" signal within
  seconds without a separate ACK call. herdr-threads already ACKs on display. The *short blocking wait
  on send* (default 10 s, max 30 s so it stays inside the harness's tool timeout) is the piece worth
  considering. VERIFIED.
- **The target is named in the wake hint.** `→ bob@team` lets a wrongly-woken pane see that the hint
  was not meant for it (tools.ts:196). It is a cheap misdelivery diagnostic. Our content-free
  `herdr-threads: wake up` deliberately omits it, and the trade-off is worth noting. VERIFIED.
- **Ownership re-checks between every await and the final keystroke,** with a distinct
  `ownership_lost` error for "pasted but not executed" (poke.ts:197-233). This pattern applies to any
  keystroke wake herdr-threads does through Herdr. VERIFIED.
- **A bounded retry ladder that stops on read** (30/180/600 s, auto-poke-fanout.ts:129-178) instead of
  unbounded re-nudging. VERIFIED.
- **A loopback REST "lifeboat"** for an agent whose MCP transport died (README.md:267-319). VERIFIED.

What xats does *not* do, and what Pi makes easy (INFERRED, per primer §2.3-2.5 and §9):

- A small Pi extension loaded per process (`pi -e`, with no config write) could replace every Pi
  weakness above:
  - subscribe to the daemon from `session_start`;
  - on mail, if `ctx.isIdle()` and `ctx.ui.getEditorText()===""`, call
    `pi.sendMessage({customType:"xats", content}, {triggerTurn:true})`. That is a draft-safe,
    tmux-free, keystroke-free wake carrying the body, so no `get_inbox` round-trip is needed;
  - while streaming, call `sendMessage` with the default steer, which is in-turn delivery after the
    tool batch;
  - identify the seat with `ctx.sessionManager.getSessionId()` instead of pid, tty and regex evidence.

  This is also the shape herdr-threads should use for a Pi adapter. It is a `Channel`-class policy in
  our delivery contract.

## Verification

Adversarial pass against the xats clone at `17df5cf` and the pi-mono clone at `a7b94a0`. Checked: the
agent-type enum and zod copies, the `mcpsmgr.json` agent entries, the README "Other coding agents" text,
`dispatchPoke`/`dispatchUnknown`/`dispatchTmux`, the `tmuxPokeImpl` step order and ownership re-checks,
the tmux CLI arguments, the quiet guard, the hint builder and its 200-character cap, `get_inbox` columns,
limit, ordering and cursor transaction, the `await_ack_s` schema, the 17 `registerTool` calls, custom
auto-bind skipping, the `bind_runtime_identity` schema and verification chain, the recovery hints,
`resolveReconnect`/`findByRuntimeUiPid`, `verifyPaneHost`, the retry and watchdog constants, and on the
Pi side the MCP docs (config, exposure, 20 KB cut), notification handlers, session-expiry retry,
`session_shutdown` close, bracketed paste, the paste-collapse threshold, steer on Enter while streaming
or compacting, `bash` spawn, `process.title`, and `getEditorText`. The tool-description size (~38 KB
including the shared `*_DESC` constants) was re-measured and holds.

Changed:
- Codemode: the `mcp_servers` line is truncated to 250 characters (pi index.ts:158, 217). The old claim
  that the whole ~3 KB `instructions` string lands in Pi's system prompt was wrong; fixed in the
  exposure section and the token-cost section.
- The codemode tool name is `mcp__cross_agent_teams__get_inbox`, because Pi maps `-` to `_`.
- Citations corrected: `ps -p` is in `src/daemon/pid-tty.ts:22`; pi `cli.md:323,332` became `325,331`;
  pi `config.ts:579` became `577`; the README lifeboat range became `267-319`; the transport.ts range became
  `236-258`; retry constants are now cited at `src/mcp/poke-retry.ts:1-2`.
