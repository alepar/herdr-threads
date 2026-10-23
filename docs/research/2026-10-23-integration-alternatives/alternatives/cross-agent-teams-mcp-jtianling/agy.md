# cross-agent-teams-mcp (jtianling, "xats") x Antigravity CLI (agy)

- Upstream: https://github.com/jtianling/cross-agent-teams-mcp
- Clone read: `17df5cf7bd9b66f00460eff05080b0e0495b6ec8`. All paths are relative to the clone root.
  The code was read and never run. agy is not installed on this host (see `primers/agy.md` §0), so
  nothing below was observed live.
- Tags: **VERIFIED** means read in xats source or docs, or in the agy primer's verified items.
  **INFERRED** means my reasoning. **unknown** means no source answers it.

## 1. Support status

**Not supported. agy is never mentioned.** VERIFIED:

- `grep -rniE "antigravity|\bagy\b|gemini"` over the clone finds two hits, and neither is
  integration code:
  - `.gitignore:110` (`.gemini/`);
  - one line in an archived openspec note saying "Claude / GPT / Gemini" models follow soft tool
    guidance (`openspec/changes/archive/2026-04-18-add-poke-mcp-tool/design.md:101`).
- The runtime enum has no agy value:
  `export type AgentType = 'codex' | 'claude-code' | 'opencode' | 'kimi-code' | 'custom'`
  (`src/lib/agent-type.ts:1`, mirrored at `src/mcp/tools.ts:109`).
- The installer manifest has entries only for `claude-code`, `codex`, `opencode` and `cursor`
  (`mcpsmgr.json`, `agents` object). There is no agy entry, so `npx mcpsmgr add ...` cannot write an
  agy config from this manifest. Whether `mcpsmgr` itself knows agy's config paths is unknown; it is
  a separate project.
- `grep -rn "herdr|zellij"` over `src/` finds nothing. tmux is the only terminal multiplexer it knows.

**What would apply to agy is the generic path.** README calls it "Other coding agents (cursor, ...)"
(`README.md:257-259`):

- agy connects over plain Streamable HTTP MCP and registers as `agent_type="custom"`.
- Wakes are a tmux paste into the agent's pane, and only when agy runs inside tmux.

The rest of this report describes that path as it would behave on agy, using `primers/agy.md`.

## 2. Injection mechanism

### 2.1 Exact mechanism: pull only, through an MCP tool

xats never pushes a message body into an agy model's context. The only injection is a short wake
hint (§3), typed into the TUI as a user turn. Bodies arrive only as the **result of the model's own
`get_inbox` MCP tool call**. VERIFIED: `src/mcp/tools.ts:196` (the send_message contract):

> Auto-poke injects only a SHORT wake-up hint (新邮件 from <sender> → <recipient_name>@<recipient_team>,
> 请调 get_inbox 查看), NOT the body — read bodies via get_inbox.

The 新邮件 hint means roughly "new mail from <sender>, please call get_inbox to view it".

On agy this works because agy supports MCP tools over Streamable HTTP. The config takes the
`serverUrl` key, in `~/.gemini/config/mcp_config.json`, `.agents/mcp_config.json` or a plugin's
`mcp_config.json` (primer §4, VERIFIED from docs/mcp). INFERRED: a hand-written entry
`{"mcpServers":{"cross-agent-teams":{"serverUrl":"http://127.0.0.1:9100/mcp"}}}` would expose the
xats tools to agy. Not tested.

### 2.2 Code path

1. **Sender.** The model calls the `send_message` / `broadcast` MCP tool. The mailbox row is written
   first (`src/mcp/tools.ts:178`: "The mailbox row is written BEFORE the wait starts").
2. **Hint.** Auto-poke builds the hint in `buildAutoPokeHint` (`src/mcp/tools.ts:367-397`):
   ```ts
   const render = (who: string, segment: string): string =>
     `新邮件 from ${who}${segment}, 请调 get_inbox 查看`
   ```
   Its length is capped at `HINT_MAX_CHARS = 200` (`:367`). It sheds the target segment first, then
   the sender's name, and falls back to an 8-character agent id prefix (`:393-396`).
3. **Dispatch.** `dispatchPoke` → `dispatchUnknown` for `custom` (`src/mcp/transport-dispatch.ts:101-112,297-311`).
   It uses the row's `tmux_pane_id` or returns `no_transport_available`.
4. **Paste.** `dispatchTmux` → `tmuxPokeImpl` (`transport-dispatch.ts:118-155`, `src/mcp/poke.ts:173-245`); see §3.
5. **Read.** The woken agy model calls `get_inbox`. `src/mcp/get-inbox.ts:30-76` reads past the
   server-side cursor in one SQLite transaction and advances it:
   ```sql
   WHERE m.to_team = ? AND m.event_id > ?
     AND ( m.to_agent_id = ? OR (m.to_role IS NOT NULL AND m.to_role = ?) )
   ORDER BY m.event_id ASC LIMIT ?
   ```
   (`get-inbox.ts:55-59`). The cursor update is `UPDATE agents SET last_processed_event_id = ? ... WHERE ... < ?`
   (`:69-75`).

### 2.3 When it fires

- The **hint** is a new user turn, typed only when the guard says the pane is quiet (§3).
- The **bodies** arrive only when the model chooses to call `get_inbox`: after a hint, after a user
  request ("check my xats inbox", `README.md:61`), or on the model's own initiative.
- **There is no turn-start, mid-turn or turn-end injection.** xats installs no hooks for any harness
  (overview, INFERRED from the file inventory).
- agy's real in-turn surfaces go unused: `PreInvocation` `injectSteps` before every model call,
  `PostInvocation`, and `Stop` `continue` (primer §2.3, §2.5).
- So a message that arrives while agy is mid-turn is invisible to that turn. It waits for the retry
  wake (§3) or for the model to call `get_inbox`. INFERRED.

### 2.4 Size bounds

- Hint: at most 200 characters (`tools.ts:367`).
- `get_inbox` page: default 50 rows, at most 200 (`get-inbox.ts:38`: `Math.min(args.limit ?? 50, 200)`),
  with `has_more` for pagination (`:63`).
- **Bodies are not truncated.** The send schemas are `body: z.string().min(1)`, with no maximum
  (`tools.ts:2835,2865,2913`). The only 4 KiB `MAX_BODY_PREVIEW_BYTES` bounds are in the opencode and
  Kimi HTTP dispatchers (`src/mcp/opencode-server-dispatch.ts:27`, `src/mcp/kimi-server-dispatch.ts:47`),
  which agy never uses.
- A 50-message page of unbounded bodies can therefore come back as one tool result. agy's per-call
  MCP limits are a timeout (180 s default), not a size cap (primer §4). How agy handles a huge tool
  result is unknown.
- Manual `poke` text is capped at 8192 bytes (`poke.ts:102,274-277`).

### 2.5 Framing

- The hint is plain text in Chinese plus ASCII: `新邮件 from alice@team (<uuid>) → bob@team, 请调 get_inbox 查看`.
- The `→ name@team` segment exists so a pane that receives a misdirected hint can tell it was not the
  addressee (`tools.ts:196`). That is the right instinct for pane-inferred identity.
- `get_inbox` returns JSON rows: `id, event_id, from_team, to_team, from_agent_id, to_agent_id, to_role,
  subject, body, need_reply, sent_at, from_role, from_name, from_device` (`get-inbox.ts:49-52`),
  wrapped by `toText` as an MCP text result.
- On agy, the hint lands as an ordinary user turn: `source:"USER_EXPLICIT"`, typed through the
  composer (primer §7, INFERRED). The model cannot tell it from the human typing, apart from its
  content.
- The first character is `新`, not `?` or `/`, so the hint avoids agy's `?` help overlay and slash
  commands (primer §8). INFERRED.

### 2.6 Dedupe and ack

- **Dedupe of bodies** comes from the server-side cursor: a default `get_inbox` never returns a row
  twice (`get-inbox.ts:44-46`, "two concurrent default calls cannot both see the same unread tail").
- **There is no dedupe of hints.** Each send attempts its own wake. Several sends to an idle agy give
  the following sequence (INFERRED from `auto-poke-fanout.ts` and `poke-retry.ts`):
  1. The first hint submits a turn.
  2. While that turn runs, the guard fails for the next ones, which are retried at 30/180/600 s
     (`src/mcp/poke-retry.ts:1-2`).
  3. A retry stops if the cursor has passed the message (`alreadyReadFn`, `poke-retry.ts:30-36`) or if
     the recipient's `last_seen_at` is newer than `sent_at` (`:126-135`).
  4. Otherwise a later hint can wake agy into an empty `get_inbox`.
- **Ack.**
  - The sender blocks for up to `await_ack_s` (default 10, max 30) and gets `ack:{status:"read"|"not_yet"}`.
    "read" means the recipient's cursor passed the row (`tools.ts:166-178`).
  - There is no harness-level confirmation that the hint reached agy's model. The tmux path returns
    `ok` after `capture_after` (`poke.ts:236-237`) and does not inspect the capture.
  - The sender learns the wake worked only when agy calls `get_inbox`.

## 3. Wake mechanism

### 3.1 Exact mechanism: tmux bracketed paste, then Enter

`tmuxPokeImpl` (`src/mcp/poke.ts:173-245`), with tmux calls from `src/daemon/tmux-cli.ts`:

1. **Quiet guard** (`src/mcp/poke-guard.ts:26-31`): `capture-pane -p -S -8` (`tmux-cli.ts:21-28`), sleep
   `POKE_QUIET_MS` (default 2000, `poke-guard.ts:3`), capture again, then `before === after ? 'pass' : 'fail'`.
   The code names this a "tail" (`GUARD_TAIL_LINES = 8`), but with no `-E`, tmux's `-S -8` starts
   8 lines into the scrollback and ends at the bottom of the visible pane. The capture is therefore
   **the whole visible screen plus 8 history lines**, not the last 8 rows (VERIFIED from the tmux
   manual's `capture-pane` `-S`/`-E` semantics, https://man7.org/linux/man-pages/man1/tmux.1.html).
2. **Ownership check** (`confirmOwnership`), then a fresh capture of the same shape. `requireReady` is checked
   too, but only Codex callers pass one. `dispatchTmux` passes none (`transport-dispatch.ts:134-141`),
   so agy gets **no composer allowlist**.
3. `tmux load-buffer -b poke-<hex> -` from stdin (`tmux-cli.ts:30-45`).
4. Another ownership check, then `tmux paste-buffer -b <buf> -t <pane> -p -d` (`tmux-cli.ts:47-49`).
   `-p` sends **bracketed paste** sequences if the application asked for them.
5. Wait 400 ms (`PASTE_SETTLE_MS`). Abort with `ownership_lost` if ownership moved (`poke.ts:225-233`).
6. `tmux send-keys -t <pane> Enter` (`tmux-cli.ts:51-53`), wait 400 ms, then capture the tail again.

### 3.2 Pane binding on agy: a manual second step

This is the main practical barrier.

- `register_agent` with `agent_type="custom"` **does not bind a pane**:
  `inferRuntimeAgentKind` returns `undefined` for custom (`tools.ts:477`), and
  `autoBindRuntimeIdentity` returns `false` straight away (`tools.ts:1026-1027`). README:259 says the
  daemon "will resolve pid → tty → pane automatically when you register". For `custom`, the code
  contradicts that (VERIFIED).
- The register description does tell the model to fall back to `bind_runtime_identity(...)` when
  binding does not converge (`tools.ts:1918`).
- For custom, `bind_runtime_identity` requires `process_pattern` (`tools.ts:129-157,3002-3047`).
  `src/daemon/runtime-identity.ts:47-52,116-150` verifies it this way (the pattern is compiled as a
  case-insensitive regex, `:51`):
  1. `ps -p <ui_pid> -o tty=,command=` (`src/daemon/pid-tty.ts:20-24`) must match the pattern.
  2. The pid's tty must equal exactly one `tmux list-panes -a -F '#{pane_id}\t#{pane_tty}'` row
     (`runtime-identity.ts:66`, `:148-150`).
  3. The weaker alternative is `ui_tty` plus `tmux_pane_id`.
- For agy, the model would have to:
  - pick a pattern such as `agy|antigravity`, which are the process names Herdr detects (primer §9);
  - supply `ui_pid=$PPID` from a `run_command` call.
- Whether `$PPID` in agy's `run_command` shell is the agy TUI process is **unknown**. If agy spawns
  commands through a helper or without a controlling tty, the result is `agent_process_mismatch` or
  `pid_has_no_tty`. The `ui_tty`+`$TMUX_PANE` fallback also needs a tty in the command shell, which is
  unknown for agy. INFERRED.
- Detection order: agy matches none of the probes (KIMI/OPENCODE/CODEX_THREAD_ID/CLAUDECODE,
  `tools.ts:1890-1894`), so the model lands on `custom` with an `agent_type_name` it has to invent,
  such as `antigravity`. Rule 5 says "prefer `agent_type_name="unknown"` over a wrong guess"
  (`tools.ts:1894`). agy sets `ANTIGRAVITY_AGENT=1` in its command shell (primer §2.4), but xats does
  not probe for it.

Without a bound pane, every send to agy reports `no_pane` (`auto-poke-fanout.ts:100-101`). That is
terminal and never retried (`docs/configs/README.md:62`). The mail just sits in the mailbox.

### 3.3 Idle and busy detection on agy

The only detection is "did the captured screen (visible pane plus 8 scrollback lines, §3.1) change
in 2 s". On agy screens (primer §8; the mapping is INFERRED):

| agy screen | Guard result | Consequence |
|---|---|---|
| Idle, empty composer (`>` plus `? for shortcuts`) | pass, if nothing animates | Correct wake |
| Busy turn (`esc to cancel`, ticking timer or streaming) | fail, if anything on the visible screen changes in 2 s | Retry later; correct |
| Subagent running (`● Agent(x) Working · 1s`) | fail (timer ticks) | Correct. Herdr itself misreports this screen as idle (Herdr #2354) |
| **Static typed draft** in the composer | **pass** | The hint is appended to the human's draft and **Enter submits both** |
| **Permission prompt** (a menu, `>` on the selected option) | **pass** while it waits | Pasted text goes into a menu; **Enter confirms the highlighted option, which can approve a command** |
| `?` help overlay or another static overlay | pass | Pasted keys go to the overlay. Behaviour unknown |
| Busy turn waiting on a long silent tool (nothing on screen changes for 2 s; whether agy always shows a ticking timer while busy is unknown) | pass | The hint is typed mid-turn. What agy does with input typed during a turn (queue or ignore) is unknown |

The Codex path got a composer allowlist (`codex-prompt-readiness.ts`, the `›` prefix) because of the
same class of problem. That allowlist is not applied to custom targets (VERIFIED,
`transport-dispatch.ts:134-141`).

### 3.4 Draft safety

None beyond quiescence. There is no empty-composer check, no stash, and no `ctrl+u`. Bracketed paste
(`-p`) only keeps a multi-line paste from submitting early; it does not protect a draft. If agy does
not enable bracketed paste mode, tmux sends the bytes plain. Whether agy supports bracketed paste is
unknown (primer §8). The hint is a single line, so either way the text lands in the composer and the
explicit `Enter` submits it. INFERRED.

### 3.5 Latency

- Best case: the 2 s quiet window, plus about 0.8 s of settle time, plus tmux calls. About 3 s from
  send to submitted turn. VERIFIED constants, INFERRED total.
- Then one model turn to decide to call `get_inbox`, then a second model call with the result. A
  first-time MCP tool call may also hit agy's permission flow: `mcp(server/tool)` rules, and in
  `request-review` mode a prompt is likely. That prompt waits for a human. INFERRED.
- Busy recipient: 30 s, 180 s, 600 s, then give up (`retry_exhausted`). Retries live in daemon memory
  only and are lost on restart (`docs/configs/README.md:61`).

### 3.6 Failure modes

- **Not in tmux** (for example agy in a Herdr pane, which is how herdr-threads runs agents): no wake
  at all, mailbox only. VERIFIED that tmux is the only path (`transport-dispatch.ts:297-311`; no
  herdr/zellij code).
- **Pane never bound** (§3.2): `no_pane`, silent, mailbox only.
- **Retry cancelled while mail is unread.** Any xats tool call by the recipient refreshes
  `last_seen_at`, because `run()` → `touchIfRegistered()` (`tools.ts:648-660`). The retry tick then
  stops with `recipient_active` (`poke-retry.ts:126-135`) even if `get_inbox` was never called. Two
  examples: a busy agy that sends its own message, or one that calls `list_agents`. INFERRED
  consequence, VERIFIED code.
- **Unconfirmed wake.** `ok` is reported after Enter whatever the screen shows afterwards
  (`poke.ts:234-237`). If agy dropped the input (an overlay, or a modal state), the sender sees
  `poked:true` and `ack:not_yet`.
- **The unread watchdog** (15 min, `need_reply` only) alerts the *sender*. On agy the sender is woken
  through the same guard-dependent paste (overview).
- **Cross-machine:** a pane on another device is never injected (`pane-host-verify.ts:70`,
  `row.device !== localDevice → REASSIGNED`).

## 4. Setup and installation footprint on agy

There is no agy installer. A user would do this by hand (INFERRED from `README.md:49-61,257-259` and primer §4):

- Start the daemon: `npx -y cross-agent-teams-mcp@latest daemon --port 9100 &` (`README.md:53`). It
  stores data in `~/.cross-agent-teams-mcp/data.db` unless `CROSS_AGENT_TEAMS_MCP_HOME` or `--db`
  overrides it (`src/cli.ts:25-27,55`).
- Add a Streamable HTTP MCP entry to one of:
  - `~/.gemini/config/mcp_config.json`, through `agy mcp add` or by hand;
  - `.agents/mcp_config.json`, behind folder trust;
  - a plugin's `mcp_config.json`.
  Add an `Authorization: Bearer` header in `headers` if the daemon was started with `--token`.
- Run agy inside tmux.
- Optionally add permission allow rules for `mcp(cross-agent-teams/get_inbox)` and friends in
  `~/.gemini/antigravity-cli/settings.json`. That is the only permission file agy honours (primer §7).

xats writes **no hooks, plugin, skill or rules** for agy, and touches nothing under `~/.gemini`
itself. VERIFIED: there is no such code. Registration is model-driven, per session, on the user's
request ("Register me to xats as alice", `README.md:327-337`). Plan on a second `bind_runtime_identity`
call for wakes (§3.2).

## 5. Token cost

**Per message (idle agy that wakes).** INFERRED estimates:

| Item | Size |
|---|---|
| Hint user turn | ≤ 200 characters, about 40-70 tokens; CJK characters tokenize densely |
| agy's own wrapping of a user turn (`<USER_REQUEST>` plus `<ADDITIONAL_METADATA>` local time). The primer (§2.3) verifies this wrapping only for a hook-injected `userMessage`; that a typed turn gets the same wrapping is INFERRED | Some tens of tokens; exact size unknown |
| Model turn emitting the `get_inbox` call | Small |
| Tool result | The JSON row: about 14 metadata fields, two UUIDs and ISO timestamps, roughly 80-120 tokens of overhead per message plus the body |
| Second model call, to act on the result | One extra call |

In total, at least two model calls and about 150-250 tokens of overhead per woken message before the
body. A spurious later hint costs a full extra turn with an empty `get_inbox`.

**Per quiet turn: nothing is injected.** The standing cost is the tool schema:

- 17 `registerTool` calls in `src/mcp/tools.ts`; some are conditional, such as `subscribe_channel_wake`.
- The literal description text measures about 19.8 k characters in tool `description:[...]` arrays,
  16.5 k in shared `*_DESC` constants, and 2.5 k in parameter `.describe()` strings. That comes to
  **about 39 k characters, around 9-10 k tokens**.
- That text rides in agy's tool list on **every model request**, quiet turns and post-tool calls
  included, unless the user limits it with `enabledTools`.
- The 39 k figure is VERIFIED by a character count over the source literals (my script, not theirs).
  The token figure is INFERRED, and the exact share sent depends on which tools register.
- The `register_agent` description alone is several kilobytes of detection rules for other harnesses
  (`tools.ts:1873-1920`). Every byte of it is irrelevant to agy.
- Whether agy caches tool schemas across requests is unknown.

## 6. Limitations and bugs

1. **No agy support at all.** There is no agent type, installer entry or probe. VERIFIED (§1).
2. **The README claim does not hold for custom agents.** README says custom agents auto-bind on
   register; the code returns early for `custom` (`tools.ts:477,1026-1027`). VERIFIED.
3. **Draft clobber.** A static human draft passes the quiet guard and gets submitted with the hint.
   The same INFERRED hazard is in the overview.
4. **Permission-menu hazard on agy.** A pending agy permission prompt is static, so it passes the
   guard. `Enter` then confirms the selected option (primer §8). INFERRED, and serious: a wake could
   approve a command. herdr-threads' `AgyComposer` reads this screen as `Unreadable` for exactly this
   reason (primer §10).
5. **No in-turn delivery.** A busy agy never sees new mail until a retry lands after the turn. agy's
   `PreInvocation` `injectSteps` path goes unused, though it exists and is verified (primer §2.5).
6. **Retry cancellation on any activity** (§3.6) can strand unread mail with no further wake.
7. **Wakes are tmux-only.** There is nothing for Herdr, zellij or plain terminals. On our deployment
   (Herdr panes) agy would get no wakes.
8. **Every send wakes.** There is no priority or threshold (overview). That conflicts with
   delivery-contract rules 1-2.
9. **Unbounded bodies** in `get_inbox` results (§2.4).
10. **Heavy standing schema cost** of about 9-10 k tokens per request (§5).
11. **Hint language.** The hint is Chinese. An agy model will most likely handle it, but it is an odd
    choice for a general tool (INFERRED). The wording is hard-coded.

## 7. Clever ideas worth stealing

- **Ownership rechecks right before each irreversible step**, with distinct errors:
  `pane_reassigned` means nothing was written, `ownership_lost` means the text was pasted and Enter
  was never sent (`poke.ts:197-233`). Separating "typed" from "submitted" is a useful outcome type for
  our wake path too.
- **`pokeWroteContent`** (`poke.ts:262-269`) decides, in the module that owns the stage order,
  whether text was left in the pane. That avoids callers re-deriving it from error names.
- **Named tmux buffer with `paste-buffer -d`** and best-effort cleanup on abort, so a message never
  lingers in a shared paste buffer (`poke.ts:146-169`). It applies to any multiplexer paste path.
- **Recipient-addressed hint** (`→ name@team`), so a wrongly targeted pane can tell the wake was not
  for it (`tools.ts:196`). That is cheap and worth having wherever identity is pane-inferred.
- **Retry cancelled by read receipt** (`alreadyReadFn`, `poke-retry.ts:30-36`). They key it on the
  cursor; their extra `last_seen_at` short-circuit is the part not to copy (§6.6).
- **`pid → tty → tmux pane` verification against a process-name pattern** (`runtime-identity.ts:116-150`).
  It works as a harness-neutral fallback for binding an unknown harness to a pane, though herdr-threads
  already gets this from Herdr's pane identity.
- **A loopback REST "lifeboat"** for when the MCP transport dies (`README.md:267-325`). A hook-based
  integration could also call it, INFERRED: an agy `PreInvocation` hook could `GET /api/inbox` and
  return the rows as `injectSteps`. That would turn xats into in-turn delivery on agy with no
  keystrokes. xats does not do this; it is the obvious missing piece.

## Verification

Adversarial pass, 2026-10-23, against the clone at `17df5cf` and `primers/agy.md`.

Checked and held: the grep hits (`.gitignore:110`, the openspec note), `AgentType` (`agent-type.ts:1`,
`tools.ts:109`), the `mcpsmgr.json` agents (`claude-code, codex, opencode, cursor`), no herdr/zellij
code in `src/`. Also held: the hint text and the 200-character cap with its shedding order
(`tools.ts:196,367-397`), `dispatchPoke`/`dispatchTmux`/`dispatchUnknown` and the missing
`requireReady` on the custom path, and the tmux stage order with its ownership rechecks
(`poke.ts:173-245`). The remaining held items: the `get_inbox` SQL, limits and atomic cursor
advance, the unbounded `body` schemas, the 4 KiB previews only in the opencode/kimi dispatchers, the
retry delays and the `alreadyReadFn`/`last_seen_at` cut-offs, `touchIfRegistered` in `run()`, the
early return for `custom` in auto-bind, `bind_runtime_identity`'s `process_pattern` requirement, the
`no_pane` terminal rule, the device check in `pane-host-verify.ts:70`, the 17 `registerTool` calls
(two conditional), and the `.describe()` total of about 2.5 k characters. Primer citations (§2.3,
§2.4, §2.5, §4, §7, §8, §9, §10) say what the report says.

Changed:
- §3.1/§3.3: the quiet guard's `capture-pane -S -8` covers the whole visible pane plus 8 scrollback
  lines, not "the last 8 rows". The table rows that assumed a bottom-8-row window are corrected.
- §3.2: the `ps -p` call is in `src/daemon/pid-tty.ts:20-24`, not `runtime-identity.ts`; added the
  case-insensitive regex note and the `list-panes` line.
- §3.5: the retry-memory citation is `docs/configs/README.md:61`, not `:60`.
- §4: the daemon command is `cross-agent-teams-mcp@latest ... &` (`README.md:53`); the data path is
  sourced to `src/cli.ts`.
- §5: the `<USER_REQUEST>` wrapping is verified in the primer only for hook-injected `userMessage`,
  so applying it to a typed turn is marked INFERRED.
- §7: the lifeboat citation is `README.md:267-325`.

Not re-derived: the 39 k-character description total. A cruder count of all single-quoted
literals in `tools.ts` gives 49 k characters, which is consistent with it.
