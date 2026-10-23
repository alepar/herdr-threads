# cross-agent-teams-mcp (jtianling, "xats") x OpenCode

- Upstream: https://github.com/jtianling/cross-agent-teams-mcp. Clone read at
  `17df5cf7bd9b66f00460eff05080b0e0495b6ec8`. Paths without a prefix are relative to that clone root.
- OpenCode source cross-checks use the local v1.18.1 checkout from the primer. Those paths are
  prefixed `[oc 1.18.1]` and are relative to the https://github.com/anomalyco/opencode root.
- Code was read, never run. Every claim is tagged VERIFIED (read in source or docs) or INFERRED.
- Companion docs: `overview.md` (this directory) and `../../primers/opencode.md`.

## 1. Support status

**Native, but only when the user launches OpenCode through the xats launcher.** VERIFIED in:

- `README.md:220-255`;
- `README.agent.md:77-80`, `283-291`;
- `src/mcp/transport-dispatch.ts:237-269`.

There are two OpenCode modes:

| Mode | How it is selected | Wake transport |
|---|---|---|
| **opencode-server** (the native mode) | The user starts `free-xats-opencode` or `xats-opencode`. The launcher runs the TUI with `--port N --hostname 127.0.0.1` and exports `OPENCODE_XATS_BASE_URL`. The model then registers with `agent_type="opencode", base_url=$OPENCODE_XATS_BASE_URL`. | `POST {base_url}/session/{id}/prompt_async` |
| **tmux fallback** | The user starts plain `opencode`. The env var is missing, so the model registers as `agent_type="custom", agent_type_name="opencode"`. A legacy `agent_type="opencode"` row with no `base_url` also lands here. | A tmux bracketed paste plus Enter, with the generic quiet guard |

Sources: `README.md:255`; `src/mcp/transport-dispatch.ts:257-261`; `src/mcp/tools.ts:1891,1894`.

Gating is explicit, by design. The openspec says the env var is "the sole sanctioned signal that the
caller is opencode". An earlier attempt to make OpenCode self-identify was deleted. The reason given:
OpenCode "does not inject `OPENCODE_SESSION_ID` or any other identifying env var into its MCP
subprocess", and its `x-opencode-session` header goes only to LLM providers. VERIFIED:
`openspec/specs/opencode-server-transport/spec.md:113-121`.

Hooks and plugins: none. xats ships no OpenCode plugin (`plugin/*.ts`) and no TUI plugin. It does not
use `chat.message`, `tool.execute.after`, `experimental.chat.messages.transform` or the `event` hook.
The only OpenCode surfaces it uses are:

- the HTTP server API, to wake;
- MCP tools, to pull bodies and act.

VERIFIED by file inventory: `grep -ril opencode` finds only daemon-side TypeScript, docs, openspec,
tests, `mcpsmgr.json` and the repo's own dev `opencode.json` (an MCP `remote` entry, no plugin); there
is no `plugin/` or `.opencode/` directory and no file to drop into `.opencode/plugin/`.

## 2. Injection mechanism

### 2.1 What gets injected, and where

A wake hint becomes a **real user message** in the bound OpenCode session. The body never travels
this way; the model pulls it through the `get_inbox` MCP tool.

The code path, all VERIFIED:

1. `send_message`, `broadcast` or `broadcast_to_role` writes the mailbox row, then calls the auto-poke
   fanout (`src/mcp/auto-poke-fanout.ts:80-127`). A non-tmux delivery skips the `no_pane` and tmux
   checks (`auto-poke-fanout.ts:75-77,96-105`).
2. `createAutoPokeImpl` builds the hint (`src/mcp/tools.ts:405-412`) and calls `poke`.
3. `poke` → `dispatchToTarget` (`src/mcp/poke.ts:340-391`) → `dispatchPoke`
   (`src/mcp/transport-dispatch.ts:101-112`) → `dispatchOpencode` (`transport-dispatch.ts:237-256`).
4. `dispatchOpencodeServerPoke` (`src/mcp/opencode-server-dispatch.ts:34-96`) runs the POST below:

```ts
// src/mcp/opencode-server-dispatch.ts:48-65
const url = `${base_url}/session/${encodeURIComponent(session_id)}/prompt_async`
const body = JSON.stringify({
  parts: [{ type: 'text', text: input.content }],
  noReply: false,
})
response = await fetchImpl(url, { method: 'POST', headers, body, signal })
```

Notes on the payload, all VERIFIED unless tagged:

- **Visibility:** the part has no `synthetic: true`, so the TUI shows it as an ordinary user message
  (`[oc 1.18.1] packages/tui/src/routes/session/index.tsx:1350-1369`: `UserMessage` renders every
  non-synthetic text part). The model receives it in the `user` role, as `prompt_async` creates a user
  message.
- **No per-message `system`, `agent` or `model` override.** It runs with the session's current agent
  and model (INFERRED from OpenCode's prompt defaults).
- **`noReply: false` is deliberate.** The spec calls it REQUIRED, because `noReply: true` would "append
  the message ... WITHOUT triggering the agent loop" (`openspec/specs/opencode-server-transport/spec.md:12`).
  So xats knows the silent-staging mode and chose never to use it.

### 2.2 Payload format and framing

The hint is a fixed Chinese-language template (`src/mcp/tools.ts:367-393`):

```
新邮件 from <sender_name>@<sender_team> (<sender_agent_id>) → <recipient_name>@<recipient_team>, 请调 get_inbox 查看
```

("New mail from ... please call get_inbox to view.")

- It is capped at `HINT_MAX_CHARS = 200`. Over the cap, the hint drops the `→ target` segment first,
  then the sender name, ending with an 8-character id prefix (`tools.ts:384-396`). VERIFIED
- The hint carries no body, subject, priority or `need_reply` marker. VERIFIED by the template.
- It carries no provenance marker that says "not typed by your human". It arrives as a plain user turn.
  The model must infer that it came from the daemon. INFERRED

The manual `poke` tool (`src/mcp/poke.ts:102,271-293`) goes through the same dispatcher. It can carry
up to 8192 bytes of free text, same-team only. VERIFIED

### 2.3 When it fires

- **On every send to that recipient, with no priority or threshold** (unless the sender passes
  `auto_poke:false`; see `overview.md`). VERIFIED: `dispatchOpencode` has no condition apart from the
  generation fence.
- **No busy or idle check before the POST.** VERIFIED: `transport-dispatch.ts:242-256` goes straight
  from the fence check to dispatch. Kimi, by contrast, gets `dispatchKimiServerPokeGated`
  (`transport-dispatch.ts:277`).
- **What OpenCode does with it** (from the primer, `[oc 1.18.1]`):
  - The server always persists the user message.
  - If a run is active, the caller attaches to that run, and the loop picks the new message up at the
    next step boundary (`effect/runner.ts:115-122`, `session/prompt.ts:1092,1111-1116`). The TUI marks
    it ` QUEUED `.
  - So on a busy session the hint lands **mid-turn, after the current LLM step or tool round**, and
    extends the current turn.
  - On an idle session it starts a new turn.

  The OpenCode side is VERIFIED by reading in the primer and has not been run.

### 2.4 Size bounds

| Item | Bound | Tag |
|---|---|---|
| Wake hint | ≤ 200 chars | VERIFIED `tools.ts:367,393-396` |
| Manual poke text | ≤ 8192 bytes UTF-8 | VERIFIED `poke.ts:102,274-277` |
| Message body at send | none: `body: z.string().min(1)` with no max | VERIFIED `tools.ts:2835,2865,2913` |
| `get_inbox` page | default 50 messages, max 200; `has_more` flag | VERIFIED `src/mcp/get-inbox.ts:38,63` |
| `get_inbox` result | one `JSON.stringify` blob of all rows (`toText`, `tools.ts:101-103`) with no per-body truncation | VERIFIED |
| OpenCode side | OpenCode's `Truncate` service presumably spills an oversize MCP result to a file | INFERRED (primer §5) |
| Error detail | body preview capped at 4 KiB (`opencode-server-dispatch.ts:27-32`) | VERIFIED |

### 2.5 Dedupe and acknowledgement

- **No dedupe of wakes.** Each send POSTs its own hint. While a turn is running, several hints become
  several queued user messages. OpenCode folds them into the running loop, because the loop continues
  while the newest user message is newer than the newest assistant message (INFERRED from primer §3).
  After one `get_inbox` drains everything, each later hint still asks the model to call `get_inbox`
  again, and those calls can return empty. INFERRED
- **Reading is the ACK.** A `get_inbox` with no `since_event_id` returns mail past the server-side
  cursor and advances the cursor in the same SQLite transaction. With `limit`, it advances only to the
  last returned row (`src/mcp/get-inbox.ts:41-76`). VERIFIED
- **The sender sees a read receipt.** `send_message` can block up to `await_ack_s` (default 10 s, max
  30 s) for `ack:{status:"read"|"not_yet"}`, and `get_delivery_status` splits `wake_status` from
  `read`. VERIFIED (`overview.md`, `tools.ts:166-178,2950-2960`)
- **`ok` only means OpenCode admitted the message.** The OpenCode wake reports `ok` on any 2xx
  (`opencode-server-dispatch.ts:74-95`). Unlike the Codex transport, nothing confirms that the model
  then called `get_inbox`. VERIFIED by contrast with `codex-appserver-dispatch.ts:117-245`
  (see `overview.md`).

## 3. Wake mechanism

### 3.1 Exact mechanism

The wake and the injection are the same `prompt_async` POST (§2.1). The details below are VERIFIED
unless tagged.

- **Recipient address.** The address is the `DeliverySpec`
  `{kind:'opencode-server', session_id, base_url, runtime_generation?, auth_token_ref?}`, stored on the
  agent row (`src/lib/delivery-spec.ts`, `src/mcp/register-opencode-self.ts:153-163`).
- **Session choice.** When `session_id` is omitted, which is what the tool description tells the model
  to do (`tools.ts:1891`, `1898`), the daemon:
  1. calls `GET {base_url}/global/health`;
  2. lists `GET {base_url}/session`;
  3. picks the entry with the highest `time_updated` (`register-opencode-self.ts:221-256`).
- **Auth.** When `auth_token_ref` names an env var, the daemon reads the password from **its own**
  `process.env` and sends `Authorization: Basic base64(user:password)`. The user is
  `OPENCODE_SERVER_USERNAME` or `opencode` (`src/mcp/opencode-auth.ts:18-31`). The openspec still says
  `Bearer` (`spec.md:13,32`), so the spec is stale and the code matches OpenCode's basic auth.
- **Generation fence.** If the row's `opencode_runtime_generation` is greater than the delivery's
  `runtime_generation`, the dispatch returns `runtime_recovering` without any I/O
  (`transport-dispatch.ts:243-250`).
- **No transport fallback.** When the row has an opencode-server delivery, an HTTP failure returns
  `opencode_connect_failed` or `opencode_inject_failed`, and no tmux paste is tried
  (`transport-dispatch.ts:251-255`; spec "No tmux fallback", `spec.md:54-79`). Codex is different: it
  falls back to tmux (`transport-dispatch.ts:224`).

### 3.2 Recovery protocol (runtime generations)

This protocol is used only when an external launcher drives it. The README names a "launcher" role,
and the CHANGELOG pairs xats with agent-of-empires (`aoe`) for Codex. The flow below is VERIFIED.

1. **Reserve.** Before a relaunch, the launcher calls `reserve-opencode-runtime` (CLI,
   `src/cli.ts:264-276,353-499`) or `POST /api/runtime/opencode/reserve` (`README.md:290-298`). It
   passes the identity key from env `XATS_IDENTITY_KEY` (never in argv) and a new
   `runtime_generation`.
   - The daemon CAS-bumps the fence (`opencode-runtime-recovery.ts:147-227`).
   - From then on, pokes return `runtime_recovering`.
2. **Commit.** After the relaunch, the launcher calls commit with the exact `base_url` and a `ses…`
   `session_id`. The daemon then:
   1. probes `GET /session/{id}` and requires the same id to come back
      (`register-opencode-self.ts:273-336`);
   2. rejects with `runtime_delivery_conflict` if another row holds that pair;
   3. CAS-writes the delivery (`opencode-runtime-recovery.ts:229-336`);
   4. schedules one recovery prompt through the same `prompt_async` path
      (`opencode-runtime-recovery.ts:525-613`).
3. **Recovery prompt.** It is sent once, with a 10 s wall-clock timeout, and cancelled if the row
   changes before the send (`promptStillCurrent`, `:630-656`). Its text, from `:105-118`:

```
[cross-agent-teams OpenCode recovery] Read XATS_IDENTITY_KEY from your own environment. Call the
cross-agent-teams MCP tool reconnect({identity_key: <XATS_IDENTITY_KEY>, agent_type: "opencode",
base_url: "...", session_id: "...", runtime_generation: N}).
```

   The commit reply says `connection_bound:false`. The MCP connection is bound only when the model
   obeys the prompt (`README.md:311-312`).

This path is sound for OpenCode, for two reasons:

- The bash tool inherits the OpenCode process env, so `printenv XATS_IDENTITY_KEY` in OpenCode returns
  the pane's own key. VERIFIED: `[oc 1.18.1] tool/shell.ts:416-425`; primer §10.
- Codex is the contrast. There the readable key belongs to the shared app-server and must not be
  passed (`tools.ts:1896`).

### 3.3 Idle and busy detection

**None.** xats never calls `GET /session/status` and never subscribes to `/event`.
VERIFIED: neither string appears in `src/` (`grep -rn "session/status\|'/event'\|/global/event" src/`
finds no match).

Correctness does not suffer, because OpenCode queues a busy-time prompt and steers it in. The token
policy does suffer: every send to a busy OpenCode extends its current turn with an extra user message.
INFERRED

### 3.4 Draft safety

- **The opencode-server path cannot clobber a human's composer draft.** `prompt_async` writes to the
  session store, not to the TUI composer, which is TUI-local state. VERIFIED structurally (primer §1).
- **A typed draft is safe, but the screen is not.** The injected user bubble appears in the
  transcript while the human is typing, and the agent starts working in the middle of their
  composition. INFERRED
- **The pane might not show the session being woken** (INFERRED; it combines VERIFIED facts):
  1. The bound session is fixed at registration.
  2. If the human then switches the TUI to another session, for example with `/new`, wakes keep
     landing in the old session.
  3. OpenCode runs that session's loop in the same server with no visible pane.
  4. The work happens off-screen and could edit files under the human's feet.
- **The tmux fallback mode** (plain `opencode`) uses the generic guard: an 8-line capture, 2 s of
  quiescence (`src/mcp/poke-guard.ts:3-4,26-31`), a bracketed paste (`tmux paste-buffer -p`,
  `src/daemon/tmux-cli.ts:48`), a 400 ms settle, then Enter (`tmux-cli.ts:52`; `poke.ts:103,173-245`). It has no composer-empty check for
  OpenCode; the `›` readiness allowlist applies to Codex only. So the hazards are (INFERRED):
  - a static human draft gets the hint appended to it and submitted;
  - an injected `/` or `@` opens an autocomplete popup, and Enter then selects the completion
    (primer §7).

### 3.5 Latency

- **Daemon side:** one loopback HTTP POST, which returns 204 at once. There is no quiet-guard delay on
  this path. VERIFIED
- **Idle recipient:** the model needs at least two LLM requests before it acts on the body. INFERRED
  1. The first turn reads the hint and emits a `get_inbox` tool call.
  2. The second step processes the JSON result.
- **Busy recipient:** the hint is seen at the next step boundary, so the wait is the remainder of the
  current LLM call plus its tool round. INFERRED from primer §3.

### 3.6 Failure modes

| Condition | Result | Retried? |
|---|---|---|
| OpenCode not listening (TUI exited, pane closed) | `opencode_connect_failed` | **No, in practice.** `createAutoPokeImpl` maps it to `guard_failed` (`tools.ts:449-451`). The retry ladder (30/180/600 s) runs only when `res.paneId` is set (`auto-poke-fanout.ts:158`), and an opencode-server registration passes no `tmux_pane_id` (`tools.ts:1453-1475`, `register-opencode-self.ts:142-164`). INFERRED that it is null in practice. |
| Unknown session id (HTTP 404) or HTTP 5xx | `opencode_inject_failed` with status and a 4 KiB body preview | As above |
| Recovery in progress | `runtime_recovering` | No. This reason is not in the retry set (`auto-poke-fanout.ts:137,158`). When the recovery prompt arrives later, it asks only for `reconnect`, and `reconnect` returns no unread hint (`grep inbox src/mcp/reconnect.ts` finds nothing). So a dropped wake is not replayed: the mail waits until the model next calls `get_inbox` for its own reasons. INFERRED |
| `auth_token_ref` env var missing in the **daemon's** env | `missing_auth_token`, no I/O | No |
| Wrong session bound (see §6.1) | 204 and reported `ok`, but the wrong session (possibly another OpenCode process's) is woken | Not detected |
| OpenCode waiting on a permission prompt | unknown (no gate in xats; OpenCode behaviour not checked) | — |
| Fresh OpenCode with no session yet | `no_active_session` at register (`register-opencode-self.ts:249-254`) | — |

In every case the mailbox row is already written, so mail is never lost. Only the wake is lost.
VERIFIED (`overview.md`).

## 4. Setup and installation footprint on OpenCode

1. **The MCP server entry.** Its location depends on the install scope:
   - project-level `opencode.json` (the default);
   - global `~/.config/opencode/opencode.json`.

   It is written by the third-party `mcpsmgr` installer or by hand (`README.agent.md:80,468,476-523`;
   `mcpsmgr.json:82-95`). The block VERIFIED at `README.agent.md:500-511`:

   ```json
   { "mcp": { "cross-agent-teams": { "type": "remote", "url": "http://127.0.0.1:9100/mcp",
     "headers": { "Authorization": "Bearer <TOKEN>" }, "enabled": true } } }
   ```

   - The guide warns that a project `opencode.json` then holds the plaintext token
     (`README.agent.md:513-514`).
   - `docs/configs/opencode.md:5-28` shows `"type": "streamable_http"` instead. That does not match the
     OpenCode config union `local | remote` (`[oc 1.18.1] packages/core/src/config/mcp.ts:16,35`), so
     that page is stale. VERIFIED discrepancy.
   - The same page also says there is "no HTTP-based opencode transport" and uses the legacy field
     `client:` (since renamed to `agent_type`) (`docs/configs/opencode.md:32,42`). That contradicts `README.md:220-255`.
2. **Shell launchers** in `~/.zshrc`. They are not shipped as files; the spec calls them
   "copy-pasteable" (`spec.md:91`). VERIFIED `README.agent.md:283-291`:

   ```zsh
   _xats-opencode() { local port; port="$(node -e '...listen(0)...')"
     OPENCODE_XATS_BASE_URL="http://127.0.0.1:${port}" exec opencode --port "${port}" --hostname 127.0.0.1 "$@"; }
   free-xats-opencode() { _xats-opencode --auto "$@"; }   # auto-approve permissions
   xats-opencode()      { _xats-opencode "$@"; }
   ```

   `README.md:228-234` shows a variant without `--auto` under the `free-` name, so the two READMEs
   disagree on what `free-xats-opencode` means.
3. **No OpenCode plugin, skill, slash command, `AGENTS.md` edit or TUI config.** VERIFIED by
   inventory.
4. **Static model guidance** arrives through MCP:
   - the server `instructions` string (mixed Chinese and English, about 4 KB of source text, including
     a per-harness `OPENCODE_XATS_BASE_URL` paragraph; `src/mcp/transport.ts:236-258`), which
     OpenCode places in the system prompt inside `<mcp_instructions>` (primer §5);
   - the tool descriptions.
5. **Registration is a conversational step.** The user tells the agent "Register with xats, name: oc-1,
   team: default", and the model:
   1. runs `printenv OPENCODE_XATS_BASE_URL` through its bash tool;
   2. calls `register_agent` (`README.md:249-253`; detection block `tools.ts:1884-1894`).

## 5. Token cost

### Per message (wake path)

| Item | Cost | Tag |
|---|---|---|
| Hint | about 30-60 tokens as a user message (≤ 200 chars, partly CJK, plus a UUID) | INFERRED |
| Model reaction | one assistant step that emits a `get_inbox` call | INFERRED |
| `get_inbox` result | each message carries about 13 metadata fields as JSON, so roughly 100-150 tokens of envelope per message plus the body | VERIFIED field list `get-inbox.ts:4-19`; token figure INFERRED |
| Request count | at least two full-context LLM requests per idle wake; prompt caching softens this | INFERRED |
| Burst of N sends | can cost N hint messages plus redundant empty `get_inbox` calls | INFERRED |

### Per quiet turn

- **No injected text.** There are no hooks. VERIFIED
- **The fixed cost is large.** OpenCode sends the MCP tool list with every LLM request. INFERRED
  (standard tool-calling; primer §5 covers MCP tool resolution per step).
  - The `register_agent` description alone is about 13.1 KB of source text (`tools.ts:1873-1922`).
  - The shared description constants are about 18.2 KB (`tools.ts:173-345`).
  - Together with the remaining tool descriptions, that is on the order of 30-40 KB, roughly 8-10k
    tokens, in every request. This is INFERRED from source byte counts, and the source includes string
    concatenation syntax, so the real figure is somewhat lower.
  - It is cacheable, but it is paid by every OpenCode agent that has the server configured, registered
    or not.

## 6. Limitations and bugs visible in the source

1. **Recency-based session binding can pick the wrong session** (INFERRED risk; each mechanism
   VERIFIED).
   - The model is told to omit `session_id` (`tools.ts:1891`). The daemon then takes the newest
     `time_updated` from `GET /session` with no filter (`register-opencode-self.ts:221-256`,
     `407`).
   - In OpenCode 1.18.1, `GET /session` lists sessions **by project**. Without `?roots=true`, the list
     includes child sessions (`[oc 1.18.1] packages/opencode/src/session/session.ts:548-555,560`;
     handler `server/routes/instance/httpapi/handlers/session.ts:64-74`).
   - The session database is one per-user file, `Global.Path.data/opencode.db`
     (`[oc 1.18.1] packages/core/src/database/database.ts:43-55`).
   - Three things can therefore win the "newest" slot:
     - another OpenCode instance in the same repository;
     - a task-tool subagent child session;
     - a session the user just switched away from.
   - xats itself names this exact failure for Kimi ("binding the wrong one misdelivers pokes while
     reporting success") and makes `session_id` REQUIRED there (`tools.ts:307-317`, `1890`). It does
     not apply that fix to OpenCode.
2. **No exclusivity check on the plain register path.**
   - The `runtime_delivery_conflict` check exists only on the generation-aware path
     (`register-agent.ts:327-334,514-524`).
   - Plain `register_agent` with `base_url` issues no collision warning. Kimi has one at
     `register-agent.ts:407-415`, and the 0.8.x "delivery coordinate exclusivity" change explicitly
     leaves OpenCode coordinates alone (`CHANGELOG.md:12`).
   - So two rows can wake the same OpenCode session (INFERRED).
3. **The wake binding goes stale when the human changes session.** There is no event subscription.
   After `/new` or a session switch in the TUI, wakes keep going to the old session off-screen (§3.4).
   INFERRED
4. **Every send wakes, with no busy check.** A busy OpenCode gets an extra user turn per send. A
   burst means repeated empty `get_inbox` round trips. VERIFIED absence; cost INFERRED.
5. **Failed wakes are not retried** on the opencode-server path, and wakes dropped during recovery are
   not replayed (§3.6). INFERRED from VERIFIED code paths.
6. **Ambiguous attribution.** The hint is a non-synthetic `user` message with no sender-class marker.
   The model, and the human reading the transcript, see "the user" asking it to call `get_inbox`.
   INFERRED
7. **An unauthenticated local control port, plus `--auto`.**
   - The launcher opens OpenCode's full HTTP API on loopback with no password by default
     (`README.agent.md:283-291`).
   - In the `free-` variant, the session auto-approves permissions (`--auto`; primer §4).
   - Any local process, or a local browser page if OpenCode's CORS allows it (unknown), can therefore
     prompt an agent that runs tools without approval. INFERRED
   - Enabling `OPENCODE_SERVER_PASSWORD` needs the same variable in the **daemon's** environment
     (`opencode-auth.ts:18-31`). In practice that means one shared password, or one ref name per
     instance, configured before the daemon starts. INFERRED
8. **Small races and stale documentation.**
   - The launcher picks a port and then lets OpenCode bind it, a TOCTOU window. INFERRED, minor.
   - The openspec says `Bearer` where the code sends Basic. VERIFIED
   - `docs/configs/opencode.md` is stale on the transport, the field names and the MCP `type`.
     VERIFIED
9. **The wake is not confirmed.** A 204 counts as `ok`, and nothing shows that the model read the
   hint. Codex gets `get_inbox` observation; OpenCode does not, though OpenCode's `/event` stream could
   provide the same. VERIFIED absence.
10. **The 2.x API break is not addressed.** OpenCode's dev line moves to `/api/session/:id/...`
    (primer §9). xats hard-codes the v1 path `/session/{id}/prompt_async`. INFERRED future breakage.
    Whether 2.x keeps v1 routes is unknown.

## 7. Clever ideas worth stealing

1. **Server-side prompt instead of keystrokes.** For OpenCode, an HTTP `prompt_async` to the session
   is strictly better than typing into the composer:
   - it cannot touch a draft;
   - it has no Enter, ESC or autocomplete hazards;
   - it needs no screen heuristics;
   - OpenCode itself queues and steers it in while the session is busy.

   herdr-threads has no OpenCode adapter yet (primer §11). This is the primer's shape 3. A plugin-based
   shape 1 gets the same advantage without a managed port.
2. **An env var set by the launcher as the runtime assertion.** `OPENCODE_XATS_BASE_URL` exists only
   when the user opted in, so the model's `printenv` proves both the harness and its address
   (`spec.md:113-121`). For herdr-threads, a managed OpenCode launch could export the seat or session
   coordinates the same way. An in-process plugin's `shell.env` hook can inject `sessionID` per tool
   call (primer §2), which removes the recency guess entirely.
3. **Generation-fenced relaunch.** The sequence is:
   1. reserve a monotonically increasing `runtime_generation` before relaunch;
   2. refuse wakes (`runtime_recovering`) while it is reserved;
   3. commit exact coordinates with CAS and an exact-session probe;
   4. post one self-recovery prompt, which is cancelled if the row moves.

   It is a clean, checkable protocol for "the process behind this seat changed"
   (`opencode-runtime-recovery.ts:147-336,525-656`). Keeping the identity key out of argv and
   redacting it from CLI output (`cli.ts:313-351`) is good hygiene.
4. **Exact-session probe before trusting coordinates.** `GET /session/{id}` must return the same id,
   with a wall-clock timeout (`register-opencode-self.ts:258-336,368-389`). This is cheap and catches
   stale rows.
5. **A typed error taxonomy per transport.** Examples: `opencode_connect_failed`,
   `opencode_inject_failed` (with status and a bounded body), `missing_auth_token` (before any I/O),
   `runtime_recovering`. Combined with `wake_status` separated from `read`, this gives senders honest
   delivery diagnostics.

Two things not to copy:

- the recency session pick;
- unconditional wakes. Our priority threshold and the `noReply: true` silent staging that OpenCode
  already offers map onto our rules 1-2, and xats does not use either.

## Verification

Adversarial pass, 2026-10-23, against the clone at `17df5cf7` and the OpenCode v1.18.1 source.

Checked and held: the dispatch chain and `prompt_async` body (`opencode-server-dispatch.ts:34-96`); the
generation fence and the absence of a tmux fallback on the opencode-server arm
(`transport-dispatch.ts:237-269`); the openspec quotes (`spec.md:12,13,32,91,115-117`); Basic auth in
code against Bearer in the spec (`opencode-auth.ts:18-31`); the hint template and its 200-char cap; the
`get_inbox` cursor transaction, limits and 14-field row; retry eligibility only with a pane
(`auto-poke-fanout.ts:158`, `poke-retry.ts:1` for 30/180/600 s); session pick by newest
`time_updated` with no filter; the exact-session probe and 10 s timeouts; the recovery prompt text,
the scheduling and `promptStillCurrent`; `runtime_delivery_conflict` only on the generation-aware
path, with the Kimi-only warning (`register-agent.ts:327-334,407-415,514-524`); `CHANGELOG.md:12`; the
launcher snippets and the `--auto` disagreement between the two READMEs; the stale
`docs/configs/opencode.md`; the absence of `session/status` and `/event` in `src/`; the description
byte counts (13,118 and 18,204 bytes); and the OpenCode side (`mcp.ts:16,35`, `session.ts:548-560`,
`database.ts:43-55`, `shell.ts:416-425`, ` QUEUED ` badge).

Changed:
- The server `instructions` string is about 4 KB of mixed Chinese and English at `transport.ts:236-258`.
  The report had said Chinese, about 700 bytes, at `:233-237`.
- The TUI visibility citation now points at `UserMessage` (`index.tsx:1350-1369`). The old range was
  the message-navigation filter.
- Line fixes:
  - hint fallback `tools.ts:384-396`;
  - poke size check `poke.ts:274-277` (`311-314` is `pokeAsDaemon`);
  - custom fallback `tools.ts:1894`;
  - detection block `1884-1894`;
  - `README.md:311-312`;
  - `docs/configs/opencode.md:42`.
- Added the quiet-guard source `poke-guard.ts`.
- The file inventory now notes the repo's own `opencode.json` and `mcpsmgr.json`.
- Dropped the unsupported "pre-0.5" version label.
