# cross-agent-teams-mcp (jtianling) x Hermes Agent (Nous Research)

- Upstream: https://github.com/jtianling/cross-agent-teams-mcp, clone at `17df5cf7bd9b66f00460eff05080b0e0495b6ec8`.
  Paths are relative to that clone root unless prefixed `hermes:`.
- `hermes:` paths are relative to a clone of https://github.com/NousResearch/hermes-agent at
  `f3b5b980d6158794beb3d1044fd7b8ca4b8f0b23`. The harness facts come from `../../primers/hermes.md`, and "primer §N" refers to its sections.
- No code was run. Tags: **VERIFIED** means read in source or docs, **INFERRED** means reasoned from them, **unknown** means not found.

## Support status

**Not supported.** Hermes has no adapter, transport, launcher, config snippet or documentation.

- VERIFIED: `grep -rni "hermes\|nous"` over the whole clone matches only the English words "asynchronous"/"synchronous".
- VERIFIED: the closed set of agent types is `'codex' | 'claude-code' | 'opencode' | 'kimi-code' | 'custom'`
  (`src/lib/agent-type.ts:1`). `dispatchPoke` routes anything outside the four named types to
  `dispatchUnknown`, which uses only a bound tmux pane (`src/mcp/transport-dispatch.ts:101-112`, `:297-311`).
- VERIFIED: `mcpsmgr.json` writes config only for `claude-code`, `codex`, `opencode` and `cursor`. There is
  no Hermes entry, and `docs/configs/` holds only `README.md`, `claude-code.md`, `codex-cli.md` and `opencode.md`.
- VERIFIED, `README.md:259`: "Anything that is not Claude Code, Codex, or opencode-via-launcher — cursor, an editor
  extension, your own harness — connects over plain Streamable HTTP and registers as `agent_type="custom"` …
  cross-agent pokes are delivered by injecting text into the agent's tmux pane".

The rest of this report evaluates that generic path, `custom` registration plus tmux paste, against Hermes
as it is today (v0.22.x, where sessions run in the gateway; primer §1).

## Injection mechanism

### What reaches the model

**A wake hint goes into the TUI composer; bodies come only through an MCP tool result.**

1. **Mailbox write.** VERIFIED (overview "Delivery guarantees"): `send_message`, `broadcast` and the other send
   tools persist a row before any wake attempt.
2. **Wake hint.** VERIFIED (`src/mcp/tools.ts:367-393`): the hint is a fixed template capped at 200 chars:
   ```ts
   const render = (who: string, segment: string): string =>
     `新邮件 from ${who}${segment}, 请调 get_inbox 查看`
   ```
   For example: `新邮件 from alice@backend (<36-char uuid>) → bob@backend, 请调 get_inbox 查看` (roughly 90-120 chars).
   The hint is Chinese ("new mail … please call get_inbox to view"). It carries the sender, the recipient label
   and the sender uuid, and no body.
3. **Paste.** For a `custom` row the hint is pasted into the bound tmux pane (Wake mechanism below). Hermes
   receives it as a bracketed paste (`tmux paste-buffer -p`, `src/daemon/tmux-cli.ts:48`). One short line stays
   under Hermes' paste-collapse thresholds of 5 lines or 2,000 chars, so it lands in the composer as literal text
   (primer §9, INFERRED). The `Enter` that follows (`src/daemon/tmux-cli.ts:51-52`) submits it as an ordinary
   **user turn**.
4. **Body pull.** The model must then call the MCP tool `get_inbox`. Hermes exposes it as a tool of the
   `mcp-<server>` toolset (primer §8; `hermes:tools/mcp_tool_registration.py:19-35`). The tool result is the only
   place a body enters context.
   - VERIFIED `src/mcp/get-inbox.ts:30-64`: the default `limit` is 50 and the maximum is 200 rows, in
     `ORDER BY m.event_id ASC`, with a `has_more` flag.
   - VERIFIED: the body schemas are `z.string().min(1)` with no maximum (`src/mcp/tools.ts:2835`, `:2865`, `:2913`), and no
     truncation exists anywhere in the inbox path. **A message has no size bound.** The only cap is Hermes'
     own handling of large tool results, which is not checked here (unknown).
5. **Mail without a wake.** If no wake lands, nothing reaches the model until it calls `get_inbox` on its own
   initiative, or until the user says "check my xats inbox" (`README.md:61`). VERIFIED.

### When it fires

- The **hint** fires only when the pane passes the tmux quiet guard (next section). There is no in-turn or
  mid-turn delivery path for `custom`. xats installs no Hermes hook (`pre_llm_call`, `transform_tool_result`)
  and no plugin. VERIFIED by the absence of any Hermes artifact.
- **Bodies** arrive only inside a `get_inbox` tool result: on the woken turn, on any turn where the model
  chooses to check, or when the user asks.

### Framing

- The hint is the only framing: the template above. VERIFIED.
- The `get_inbox` reply is `{messages, has_more, last_event_id}`; each row carries ids, teams, roles and `from_agent_id`, `from_name`, `from_device`, `subject`, `body`,
  `need_reply` and `sent_at` (`src/mcp/get-inbox.ts:4-25`, `:49-64`). The tool description adds reply-routing guidance
  (`src/mcp/tools.ts:2934`). VERIFIED.
- Nothing marks the content as peer input as opposed to user input. The hint arrives as a user message typed
  into the composer, so the model cannot tell it from the human (INFERRED).

### Dedupe and acknowledgement

- **Cursor-based read.** A default `get_inbox` advances `agents.last_processed_event_id` in the same transaction
  as the read, so a row is returned once. An explicit `since_event_id` is a read-only peek. VERIFIED
  `src/mcp/get-inbox.ts:38-75`, description `src/mcp/tools.ts:2931-2933`.
- **Read receipt.** The sender's `send_message` blocks for up to `await_ack_s` (default 10 s, max 30) and gets
  `ack.status` `"read"` once the cursor passes the message. VERIFIED `src/mcp/tools.ts:166-178`. Hermes' default
  MCP tool timeout is 300 s (`hermes:tools/mcp_tool_common.py:41`), so the blocking send fits (VERIFIED for the
  constant, INFERRED for the fit).
- **No dedupe of wakes.** Each send produces its own hint and poke attempt. A retry tick stops once the cursor passes
  the message (`already_read`) or once the recipient's `last_seen_at` is newer than the send (`recipient_active`)
  (`src/mcp/poke-retry.ts:98-134`; scheduled from `src/mcp/auto-poke-fanout.ts:129-178`). Several unread sends therefore paste several
  hints, each one a separate user turn (INFERRED).

## Wake mechanism

### Exact mechanism (custom → tmux)

`dispatchUnknown` (`src/mcp/transport-dispatch.ts:297-311`) calls `dispatchTmux` (`:118-155`), which calls
`tmuxPokeImpl` (`src/mcp/poke.ts:173-245`). VERIFIED sequence:

1. `verifyPaneHost` (`src/mcp/pane-host-verify.ts:68-93`) checks four things in order:
   - The device must be local.
   - The DB must still say this row owns the pane.
   - If the row has a pid: the pid must be alive and own the pane's tty.
   - If the row has no pid: the pane must exist and no live pid-bearing claimant may own it.
   An unqueryable tmux returns `undecidable`, which becomes `tmux_unavailable`.
2. `runQuietGuard` (`src/mcp/poke-guard.ts:26-31`): capture, sleep `POKE_QUIET_MS` (default 2000), capture
   again. A difference means `guard_failed`. The capture is `tmux capture-pane -p -S -8` (`src/daemon/tmux-cli.ts:21-27`);
   per tmux(1) `-S -8` starts 8 lines into history and the end defaults to the bottom of the visible pane, so it
   compares the **whole visible screen plus 8 history lines**, not only the last 8 lines (VERIFIED, tmux 3.7b man page).
3. Ownership re-check, a fresh capture (same `-S -8` window), then `requireReady`. A readiness check is wired only for Codex
   (the `›` composer prefix). Hermes as `custom` gets **no** readiness check. VERIFIED `src/mcp/poke.ts:196-208` and
   the comment at `:180-182`.
4. `tmux load-buffer -b poke-<hex> -`, ownership re-check, `tmux paste-buffer -b … -t <pane> -p -d`
   (bracketed paste, buffer deleted), wait 400 ms, ownership re-check (a failure here gives `ownership_lost` with the
   text pasted but Enter not sent), then `tmux send-keys -t <pane> Enter`, wait 400 ms, capture the tail.
   VERIFIED `src/mcp/poke.ts:209-233`, `src/daemon/tmux-cli.ts:24-52`.

### Binding a Hermes pane (precondition for any wake)

`register_agent` with `agent_type="custom"` does **not** bind a pane. `inferRuntimeAgentKind` returns `undefined`
for `custom` (`src/mcp/tools.ts:477`), so `autoBindRuntimeIdentity` returns false (`:1027`). Registration then
returns the hint "No usable tmux_pane_id is bound yet … Call `bind_runtime_identity(...)`" (`:1752-1756`). The
test `tests/register-agent-opencode-tmux.test.ts:90-91` states it directly: "auto-bind does not fire for
agent_type='custom'". VERIFIED. (This contradicts `README.md:259`, which says the daemon resolves `pid → tty → pane`
automatically when the agent registers.)

The Hermes model would therefore have to call `bind_runtime_identity` itself
(`src/mcp/tools.ts:3002-3048`), with `agent: "custom"`, a required `process_pattern` (a regex over `ps` command
lines, `src/daemon/runtime-identity.ts:47-52`) and one of:

- **`ui_pid`.** The daemon runs `ps` on that pid, tests the pattern, takes its tty and matches it against
  `tmux list-panes -a` (`src/daemon/runtime-identity.ts:116-177`). Under Hermes ≥ 0.22.1 the model's terminal
  commands are children of the **gateway's** bash backend, not of the pane's client (primer §1, consequence 3).
  `$PPID` therefore names a gateway-side shell, with no tty or the wrong one. The result is `pid_has_no_tty`,
  `agent_process_mismatch` or `tmux_pane_not_found` (INFERRED, high confidence). The model would have to find the
  pane client's pid by other means, for example `ps -t <tty>`, but it does not know its tty either.
- **`ui_tty` plus `tmux_pane_id`.** The model would need `$TMUX_PANE`. Hermes' `client_env` allowlist carries
  only `PATH`, `VIRTUAL_ENV`, `GOPATH`, `GOROOT`, `CARGO_HOME`, `RUSTUP_HOME`, `JAVA_HOME` and the prefixes `CONDA_`, `NODE_`, `NVM_`,
  `PYENV_`, plus the profile's `terminal.client_env_passthrough` (`hermes:agent/session_client_env.py:21-30`).
  `TMUX` and `TMUX_PANE` do not travel by default (VERIFIED for the list; INFERRED for the effect). An operator
  could add `TMUX_PANE` to `client_env_passthrough`, which would make this path work (INFERRED, primer §1).
- **`detect_tmux_pane`** (`src/mcp/tools.ts:1832-1868`) scans every tmux pane machine-wide by process pattern,
  optionally narrowed by `cwd`/`tty`/`title_contains`; `agent:"custom"` requires a `process_pattern`
  (`src/mcp/tools.ts:111-127`). It can find the pane when exactly one Hermes client is running, and it is ambiguous
  with two in the same cwd. The `bind_runtime_identity` description calls it "for debugging only"
  (`src/mcp/tools.ts:3011`). VERIFIED for the tool, INFERRED for the outcome.

The same environment trap breaks `XATS_IDENTITY_KEY`. A launcher export reaches the pane client, but
`printenv` in a Hermes ≥ 0.22.1 terminal tool reads the gateway's environment. The name contains `KEY`, which
Hermes' secret-name policy blocks from ever travelling in `client_env`, even when listed in
`client_env_passthrough` (`hermes:agent/session_client_env.py:48-54`, `hermes:tools/code_execution_env.py:29-35`;
VERIFIED). xats already documents this trap for Codex and server-hosted Kimi (`src/mcp/tools.ts:247-248`, `:260-268`),
but not for Hermes. A Hermes agent following the
generic instructions could read and claim a key exported by whichever shell started the gateway (INFERRED).

### Idle/busy detection

- The only signal is **screen quiescence**: the visible screen (plus 8 history lines) must be byte-identical across 2 s. VERIFIED.
- Hermes-specific consequences (INFERRED, unverified live):
  - While Hermes streams or shows a spinner, the guard fails, which is correct. The wake is retried at 30 s,
    180 s and 600 s (`src/mcp/poke-retry.ts:1-2`, scheduled at `src/mcp/auto-poke-fanout.ts:162-178`).
  - A long silent tool call, or a turn blocked on an approval, clarify or sudo prompt (`on_human_input_request`,
    primer §2.3), can leave the screen static. The guard passes, and the paste plus `Enter` lands mid-turn.
    Hermes' default `display.busy_input_mode: interrupt` makes `Enter` during a running turn **redirect or
    interrupt it** (primer §9). An Enter on an approval prompt may also answer that prompt (unknown which
    option a bare Enter selects).
  - Because the whole screen is compared, anything animating anywhere on Hermes' idle screen (a clock, a token
    counter in the status bar) would make the guard fail forever and turn every wake into three failed retries.
    Whether Hermes' idle screen animates is unknown.

### Draft safety

There is none for `custom`. The guard detects motion, not a non-empty composer. A human's half-typed draft
that is not changing passes the guard: the hint is pasted after the draft and `Enter` submits draft plus hint
as one user message (INFERRED from `src/mcp/poke-guard.ts:26-31` and the absent `requireReady`). Hermes has a
Ctrl+S draft stash (primer §9), and xats does not use it.

### Latency

- Best case: about 2.0 s guard, plus 0.4 s settle, plus 0.4 s, plus the tmux calls. Roughly 3 s to a submitted
  turn, then one model round trip to the `get_inbox` call (VERIFIED constants, INFERRED total).
- With `guard_failed`: retries at +30 s, +180 s and +600 s, then nothing (`src/mcp/poke-retry.ts:1-2`, `:82-87`). VERIFIED. After that the message waits
  for a voluntary `get_inbox`. The sender's unread watchdog fires at 15 min for `need_reply` mail
  (`src/mcp/unread-watchdog.ts:6`).

### Failure modes

- `tmux_unavailable`, `no_transport_available`, `pane_reassigned`, `ownership_lost`, `pane_dead`, `guard_failed`
  (`src/mcp/poke.ts`, `src/mcp/transport-dispatch.ts:297-311`). VERIFIED.
- **No tmux at all.** All tmux calls use the daemon's default tmux server (no `-S`/`-L`, `src/daemon/tmux-cli.ts`,
  `src/daemon/runtime-identity.ts:62-68`). Hermes in a plain Herdr pane, or in tmux on a different socket, gets
  `tmux_unavailable` or `tmux_pane_not_found`, so it has **no wake at all**. The code knows nothing of Herdr,
  zellij or screen (VERIFIED by grep).
- **Shared identity.** Hermes MCP connections are profile-owned and shared. "Within one profile the connection
  is the profile's" (`hermes:tools/mcp_tool_registration.py:517-518`), and "A shared live connection remains owned
  by the profile that opened it" (`:540`). xats binds identity per MCP session (`requireAgent` → `caller()`,
  `src/mcp/tools.ts:697-705`). Two Hermes sessions on one profile would therefore likely share **one** xats seat,
  and the second `register_agent` would re-point that seat. This would cross-deliver mail and wake the wrong pane
  (INFERRED, medium confidence: the connection-sharing rule is verified, but how it applies per session under
  `current_session_tool_scope` was not traced end to end).

## Setup and installation footprint on Hermes

Nothing is automated. VERIFIED: there is no Hermes entry in `mcpsmgr.json` and no Hermes doc. A user would:

1. Run the xats daemon (`npx -y cross-agent-teams-mcp daemon --port 9100`, `mcpsmgr.json` prerequisites).
2. Hand-edit the profile's `~/.hermes/config.yaml` (or `$HERMES_HOME/config.yaml`):
   ```yaml
   mcp_servers:
     cross-agent-teams:
       url: "http://127.0.0.1:9100/mcp"
       # headers: { Authorization: "Bearer <token>" }   # only with --token
   ```
   This shape follows Hermes' documented HTTP MCP entries (`hermes:website/docs/user-guide/features/mcp.md:336-341`,
   field table `:494-500`). It is INFERRED to work and was not tested.
3. Run Hermes inside a tmux pane on the daemon's default tmux socket.
4. Tell the agent "register me to xats as alice", then get it to `bind_runtime_identity` with a `process_pattern`
   such as `hermes_cli|hermes`, and the pane coordinates obtained as described above.

xats installs no plugin, hook, skill or system-prompt section into Hermes, and writes no file under
`~/.hermes`. VERIFIED by absence.

## Token cost

Per message (INFERRED estimates):

- The hint user turn is about 40-70 tokens of mostly CJK text. It costs a **full model turn**, because the woken
  turn's whole job is to call `get_inbox`.
- The `get_inbox` call plus its JSON result costs the body plus about 60-100 tokens of per-row field names and ids.
  A paginated backlog can return up to 200 rows in one result.
- The model usually spends a further turn acting on the result.
- Duplicate wakes (several sends before a read) each add a user turn, because hints are not coalesced.

Per quiet turn:

- Injected text costs nothing, since no hook runs.
- The MCP tool list does cost something on **every request**. xats registers 17 tools in `src/mcp/tools.ts`
  (plus `echo`, `src/mcp/transport.ts:262`; there is no `poke` MCP tool, `tests/poke-tool-registration.test.ts:27-28`),
  with very long descriptions. The description arrays and `*_DESC` constants in `tools.ts`
  total about 20-40 KB of source text (an `awk` byte count over the arrays). The `register_agent` description
  alone is several KB and includes per-harness detection recipes for runtimes Hermes is not.
  - That is roughly 5,000-10,000 tokens of standing tool schema per request, mitigated by prompt caching
    (INFERRED).
  - Hermes can filter a server's tools (`mcp.md` field table). xats does not suggest it.

## Limitations and bugs visible

1. **No Hermes awareness at all.** In particular the `ui_pid=$PPID` and `printenv XATS_IDENTITY_KEY` recipes
   are wrong under Hermes' gateway model, the same trap xats documents for Codex and Kimi (`src/mcp/tools.ts:247-268`). A Hermes agent following the "BRANCH 1" reconnect instruction could claim another pane's identity
   (INFERRED).
2. **README and code disagree** on whether a `custom` registration auto-binds a pane (`README.md:259` against
   `src/mcp/tools.ts:477`, `:1027` and `tests/register-agent-opencode-tmux.test.ts:90-91`). VERIFIED.
3. **Keystroke wake into a harness whose default busy mode is `interrupt`.** A static-screen false "idle"
   interrupts a running turn, or answers an approval prompt (INFERRED).
4. **Draft clobbering**: no composer-empty check for `custom` (INFERRED from source).
5. **Wake is not content-free and not a priority decision**: every send wakes unless `auto_poke:false`. There is
   no threshold (overview). The hint is natural-language Chinese addressed to the model, which puts it in the
   transcript as a user message.
6. **tmux-only, default socket.** Hermes under Herdr's own multiplexer cannot be woken. VERIFIED.
7. **Unbounded bodies** in `get_inbox` (VERIFIED: no maximum on the body schema).
8. **Gateway viewer gating** does not apply here. A keystroke wake requires a client attached to the pane, so it
   works only when a viewer exists (INFERRED). Unlike the native `inject_message` path, it cannot wake a
   detached session.

## Clever ideas worth stealing

- **Read receipt as a blocking send** (`await_ack_s`, at most 30 s, `src/mcp/tools.ts:166-178`). The sender learns
  within seconds whether the recipient's own read happened. The description spells out that `not_yet` is not a
  failure, so senders do not re-send or take over work. VERIFIED.
- **Ownership re-check immediately before each irreversible tmux step**, with the result split into
  `pane_reassigned` (nothing written) and `ownership_lost` (pasted, Enter withheld) (`src/mcp/poke.ts:198-226`).
  This is a good pattern for any keystroke wake into a pane whose owner can change. VERIFIED.
- **The recipient label inside the wake hint** (`→ bob@team`), so a mis-woken pane can tell the hint was not for
  it (`src/mcp/tools.ts:384-388`, described at `:189`). This is a cheap correctness aid for keystroke wakes. It costs bytes, though, and a
  content-free wake (our R18) avoids the problem differently. VERIFIED.
- **Retry ladder stopped by the read cursor** (30/180/600 s, cancelled by `alreadyReadFn`). The ladder cancels
  itself the moment the item is consumed through any path (VERIFIED). For Hermes specifically, the better
  approach is none of xats': `ctx.inject_message(session_key=…)` or gateway `prompt.submit` queues while busy,
  starts a turn while idle, and never touches the composer (primer §4, §5.1).

## Verification

Adversarial pass, 2026-10-23, against the xats clone at `17df5cf7`, the Hermes clone at `f3b5b980` and the primer.
No code was run.

Checked and confirmed: agent-type set and `dispatchPoke`/`dispatchUnknown` routing; `mcpsmgr.json` agents; README
lines 61 and 259; hint template and 200-char cap; `get_inbox` limit, order, cursor transaction; body schemas without
maximum; `await_ack_s` bounds; `verifyPaneHost` order; `tmuxPokeImpl` sequence, 400 ms settles, ownership
re-checks and the Codex-only `requireReady`; custom auto-bind refusal (`tools.ts:477`, `:1027`, test `:90-91`);
`bind_runtime_identity` argument rules; 15-min watchdog; 30/180/600 s retry ladder; default tmux socket; Hermes
`_DEFAULT_TOOL_TIMEOUT = 300`, `client_env` allowlist, MCP connection-sharing docstrings, HTTP `mcp_servers` shape;
primer §1, §2.3, §4, §5.1, §8, §9.

Changed:
- Quiet guard: it compares the whole visible screen plus 8 history lines (`capture-pane -S -8`), not the last 8
  lines; the idle-animation risk is restated accordingly.
- Retry stop conditions: added `recipient_active` (`last_seen_at` newer than the send), cited `src/mcp/poke-retry.ts`.
- Tool count: there is no `poke` MCP tool (a test asserts its absence); the extra tool is `echo`.
- "for debugging only" is in `bind_runtime_identity`'s description, not `detect_tmux_pane`'s.
- `XATS_IDENTITY_KEY` cannot travel to a Hermes session even via `client_env_passthrough` (name contains `KEY`).
- Fixed citations: Codex/Kimi env-trap text (`tools.ts:247-268`, not `:1897`), cursor logic (`get-inbox.ts:38-75`),
  reply guidance (`tools.ts:2934`), hint code (`tools.ts:384-388`), mcp.md field table (`:494-500`);
  `docs/configs/` also has `README.md`; the grep hits include "synchronous".
