# agentchattr x Claude Code

- Upstream: https://github.com/bcurts/agentchattr (MIT). Clone examined at HEAD `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10), `VERSION` 0.5.0. GitHub metadata (1558 stars, v0.5.0 released 2026-07-26) is taken from `overview.md`.
- Paths are relative to the clone root. **VERIFIED** means read in source or docs. **INFERRED** means reasoned from source plus the harness primer (`../../primers/claude.md`, cited as "primer §N"). **unknown** means not determined.
- No code from the clone was executed. The only things run were `python3 -I` AST reads of `mcp_bridge.py` to measure string lengths.

## Support status

**Native, and the default, first-listed harness.** VERIFIED:

- `config.toml:10-14` declares `[agents.claude] command = "claude"`. The launchers are `macos-linux/start_claude.sh`, which ends in `.venv/bin/python wrapper.py claude`, and `start_claude_skip-permissions.sh:64`, which adds `--dangerously-skip-permissions`. Windows has `.bat` equivalents.
- `wrapper.py:131-137` holds the built-in provider default:
  ```python
  "claude": {"mcp_inject": "flag", "mcp_flag": "--mcp-config",
             "mcp_transport": "http", "mcp_merge_project": True},
  ```
- For Claude, the integration is exactly two things: an MCP server reached over streamable HTTP, and keystroke injection through tmux. There are no Claude hooks, no plugin or mod, no channels, no cross-session messaging and no SDK use. A grep for `hook`, `UserPromptSubmit`, `asyncRewake`, `claude/channel`, `SendMessage` and `CLAUDE_CODE_MESSAGING` found nothing relevant (VERIFIED, see `overview.md`). The one Claude-specific accommodation in the injection path is flattening newlines to dodge the paste collapse (`wrapper.py:564-567`). The one Claude-motivated accommodation in the launch path is removing `CLAUDECODE` from the environment (`wrapper.py:723`; it is stripped for every agent, not only Claude).

## Injection mechanism

### Shape: wake text is pushed, content is pulled

1. A message posted in the web UI or through `chat_send` is routed by `@mention` (`router.py:52-81`, `app.py:852-898`). For each target, the server appends one JSON line to `data/<name>_queue.jsonl` (`agents.py:32-54`):
   ```python
   entry = {"sender": ..., "text": message, "time": ..., "channel": channel}
   # + "prompt" (session engine) / "job_id"
   ```
   The `text` field carries the message, but the Claude wrapper never forwards it. Only `channel`, `job_id` and `prompt` are used (`wrapper.py:489-531`). VERIFIED.
2. The wrapper's `_queue_watcher` thread polls the file every 1 s, reads every line and truncates the file (`wrapper.py:480-486`, `571`). It then builds the wake text (`wrapper.py:526-531`):
   ```python
   prompt = f"use mcp to read #{channel} - you're mentioned, take appropriate action and respond"
   # job:     "use mcp to read job_id={job_id} - you're mentioned in a job thread, ..."
   # session: the custom prompt verbatim
   ```
   This text is pasted into Claude's TUI (see the Wake section). Claude receives it as an ordinary user prompt. VERIFIED.
3. The model is expected to call the MCP tool `chat_read` (12 tools are registered, `mcp_bridge.py:943-959`). That call is the only route by which message bodies reach Claude's context: they arrive as a tool result. The model replies with `chat_send`. VERIFIED.

So injection happens **only when the model chooses to call a tool**. The wake prompt asks it to make that call. The MCP `instructions` string asks it to "respond in chat" (`mcp_bridge.py:76-80`) and to read "one read per relevant channel per turn" (`mcp_bridge.py:81-85`).

### Extra content carried by the wake prompt (VERIFIED, `wrapper.py:535-567`)

- `ROLE: <role>` is appended on every trigger when a role is set. The wrapper fetches it fresh from `GET /api/roles` with a 3 s timeout (`wrapper.py:430-439`).
- `RULES: r1; r2; ...` is appended:
  - on the first trigger,
  - when the rules epoch changes, or
  - every `refresh_interval` triggers (server setting `rules_refresh_interval`, default 10, `app.py:2125-2130`).

  Each rule is capped at 160 chars (`rules.py:9`, `:120`). There is no cap on how many rules are active (VERIFIED: none found in `rules.py`). After injecting, the wrapper reports `POST /api/rules/agent_sync/<name> {epoch}` (`wrapper.py:454-471`, `559`). This is a rules "seen" receipt, but it records that the text was typed, not that the model read it.
- An identity hint (`wrapper.py:422-427`) is added once, on the first trigger of a multi-instance (slot > 1) wrapper.
- Session-engine prompts (`session_engine.py:309-338`) are multi-paragraph: SESSION, GOAL, PHASE, ROLE and INSTRUCTION, plus a "You MUST respond using chat_send" footer, joined with `\n\n`.

All of this is flattened with `prompt.replace("\n", " ")` before pasting. The comment at `wrapper.py:564-566` explains why: "multi-line text triggers paste detection in CLIs (Claude Code shows "[Pasted text +N]")". VERIFIED.

### `chat_read` payload (VERIFIED, `mcp_bridge.py:575-689`, `409-426`)

- **Output:** a JSON array of `{id, sender, text, type, time, channel[, attachments{name,url,file_path}][, reply_to]}`.
- **Identity breadcrumb:** in multi-instance setups, `[identity: <name> | label: <label>]` is prepended.
- **Job reads:** they return a synthetic header entry with `id = -1` and the title, body, status and assignee, followed by every message in the job thread, with no limit (`mcp_bridge.py:598-637`).
- **Empty reads:** they return escalating text: "No new messages. Do not poll — wait for your next prompt." then a second, firmer message, then "... STOP. Repeated empty reads waste tokens." (`mcp_bridge.py:666-679`).

### Cursor semantics, dedupe and ack

- Each agent has a cursor per channel key (the channel name, or `__all__` when no channel is given), persisted to `data/mcp_cursors.json` (`mcp_bridge.py:566-572`, `442-454`).
- A read with a cursor returns `get_since(cursor)[-limit:]` with `limit` defaulting to 20, then moves the cursor to the last id returned (`mcp_bridge.py:648-663`). The first read returns the last 20 messages.
- Dedupe comes only from the cursor. There is no message-level ACK, read receipt or delivery ledger. VERIFIED.

### When injection fires

| Claude state | What happens | Evidence |
|---|---|---|
| Idle at an empty prompt | The wake prompt is pasted and submitted, a new turn starts, and the model (ideally) calls `chat_read` | VERIFIED code; turn start is standard TUI behaviour |
| Busy (generating or running tools) | The paste plus Enter is typed into the composer anyway. Claude queues typed input and "passes it to Claude as soon as those tool calls finish, within the same turn" (primer §7). In practice that gives mid-turn delivery of the *wake text*, and the model then has to call `chat_read` within the turn | INFERRED from primer §7 plus the lack of any busy gate (`wrapper.py:502-567`). This corrects `overview.md`'s "no mid-turn delivery": the wake text arrives mid-turn, while the bodies still need a tool call |
| Model calls `chat_read` on its own | Bodies arrive in the tool result, at any time | VERIFIED |

### Size bounds

- **No per-message body truncation.** Neither `chat_read` nor `store.add` caps message text (VERIFIED: no cap in `mcp_bridge.py:345` or `store.py`). Summaries are capped at 1,000 chars (`mcp_bridge.py:924`). Job proposals are capped at 80/1,000 chars (`mcp_bridge.py:375-376`).
- **Page bound** is `limit` messages (default 20, chosen by the caller). Job reads are unbounded.
- **Claude-side limits:** a large `chat_read` result hits Claude's MCP output warning at 10k tokens and is cut at the 25k default cap (`MAX_MCP_OUTPUT_TOKENS`). Text over 50k chars goes to a file (primer §5). Twenty long human messages can reach this. INFERRED.
- **Wake-text length:** this is unbounded once RULES are included. Above about 800 chars, Claude collapses the paste to `[Pasted text #N]` even on a single line (primer §7: "more than 800 chars **or** more than 3 lines"). Flattening newlines removes only the second trigger. INFERRED.

## Wake mechanism

### Exact mechanism (VERIFIED, `wrapper_unix.py:64-136`)

- The wrapper owns a dedicated tmux session named `agentchattr-<name>`. It creates the session with `tmux new-session -d -s ... <env -u CLAUDECODE ...> claude --mcp-config <file>` and then attaches the user's terminal to it (`wrapper_unix.py:186-256`).
- Delivery runs these steps, each tmux call with a 5 s timeout:
  ```python
  pane = tmux display-message -p -t <session> "#{pane_id}"   # resolve once
  tmux load-buffer -b agentchattr-inject-<pid>-<rand> -       # text on stdin
  tmux paste-buffer -p -d -b <buf> -t <pane>                   # -p: ESC[200~..ESC[201~ if the app enabled DECSET 2004
  sleep(max(inject_delay, len(text) * 0.001))                  # default 0.3 s (wrapper.py:908)
  tmux send-keys -t <pane> Enter
  ```
- The docstring records a real Claude Code failure that this design fixes (`wrapper_unix.py:67-75`; also `tests/test_inject_transport.py:1-21`):
  - with a plain `send-keys -l`, the pty's input queue splits a long prompt across reads;
  - Claude treats a large read as a paste, and "when typed text follows a paste before Enter, submits only the typed text";
  - on macOS, a 1204-byte prompt arrived as its last 182 chars.

  Bracketed paste lets Claude reassemble the split prompt. The test suite drives a real tmux with a raw-mode reader. The docstring says Claude's own handling was "verified separately against Claude Code by transcript".
- **Retries:** none. On failure the wrapper prints `INJECT FAILED`, drops the buffer and returns False. "Failure does not imply that retrying is safe: a paste may have arrived" (`wrapper_unix.py:77-83`). The watcher ignores the return value (`wrapper.py:567`), so the trigger is lost.

### Idle and busy detection

- **No gate before the wake.** The watcher injects whenever the queue file holds at least one parseable line (`wrapper.py:483-502`). The server's routing does not check activity either (`app.py:886-898`): the only gates are the loop guard, pending-instance and session-turn checks, and `agents.is_available`, which means "registered" (`agents.py:15-16`). `is_online` only adds an "appears offline — message queued" notice. VERIFIED.
- **Activity detection exists, but only feeds the UI's "busy" pill:**
  - it hashes `tmux capture-pane -p` once a second and counts any change as active (`wrapper_unix.py:159-183`);
  - it posts the result through heartbeats (`wrapper.py:842-879`);
  - the server expires it after 8 s (`mcp_bridge.py:32`, `769-779`).

  The watcher sets `trigger_flag` before each injection, so the pill shows busy through the model's thinking phase (`wrapper.py:503-506`). Codex gets an animation filter (`wrapper_unix.py:139-156`); Claude gets no filter. VERIFIED.
- For Claude, a screen hash cannot tell a running spinner from a static idle prompt that has a draft in it. It is a liveness hint, not an idle oracle. INFERRED.

### Draft safety

There is none. VERIFIED: no composer read, no `C-u`, no stash and no check. Consequences, INFERRED from primer §7:

- **Human draft present:** the wake text is pasted at the cursor inside the draft and Enter submits both. The human's half-typed text gets sent with "use mcp to read #general ...".
- **Dialog open** (a permission prompt, `/model`, or similar): the paste and the Enter go to the dialog instead. The Enter may confirm the highlighted option. The exact effect depends on the dialog and is unknown. This matters because `start_claude.sh` launches without `--dangerously-skip-permissions`, so permission prompts on `mcp__agentchattr__*` or other tools can appear (VERIFIED: the launcher passes no permission flag; it is unknown whether Claude prompts for MCP tools under the user's own settings).
- **Prompt suggestion showing:** the suggestion is dim text in an empty box, and the paste replaces it. This case is harmless.
- **Busy:** the wake text joins Claude's typed-input queue. That is safe, but it costs extra input tokens (see the token section).

### Coalescing and loss

- **One injection per poll.** All lines read in one poll produce a single wake. Each of `channel`, `job_id` and `prompt` is taken from the last line that carries it, and a custom `prompt` beats `job_id`, which beats `channel` (`wrapper.py:489-529`). So a `job_id` from an earlier line can outlive a later plain channel mention. If mentions in #a and #b land in the same 1 s window, the wake names only #b. The model reads #a only if it decides to do so on its own. VERIFIED code; consequence INFERRED.
- **Read-then-truncate race.** `readlines()` followed by `write_text("")` (`wrapper.py:484-486`) drops a line the server appends between those two calls. INFERRED.
- **Wrapper restart.** The wrapper clears its queue file on start (`wrapper.py:719-721`), so wakes queued while it was down are dropped. The messages stay readable through the cursor. VERIFIED.

### Latency (INFERRED from code)

- **Wrapper side**, from the queue append to Enter:
  - 0-1 s of poll wait (`wrapper.py:571`);
  - 0.5 s of fixed sleep (`wrapper.py:506`);
  - up to two loopback HTTP fetches (role and rules, 3 s timeout each);
  - the three tmux calls;
  - 0.3 s, or 1 ms per char if longer.

  That is typically about 1-2 s.
- **Claude side:** about one model turn, then a `chat_read` round trip. Under tool search, the first use in a session probably also needs a ToolSearch call to load the deferred `mcp__agentchattr__chat_read` schema (primer §5: MCP tools are "deferred under tool search"). INFERRED.

### Failure modes (summary)

- **tmux:** not installed, which exits the wrapper (`wrapper_unix.py:35-44`); a dead pane; or a timeout. Each failure loses the trigger with no retry.
- **Claude not ready:** a wake typed during Claude's startup, onboarding or trust dialog lands in that dialog. INFERRED.
- **Loop guard:** after `max_agent_hops` (default 4) agent-to-agent hops, a channel stops routing until a human posts or someone runs `/continue` (`router.py:69-81`, `app.py:863-873`). This is intended behaviour, not a fault, but it means an agent-sent @mention can silently produce no wake.

## Setup and installation footprint on Claude

VERIFIED unless marked:

- **Per-instance MCP config file.** The wrapper writes `<data_dir>/provider-config/<instance>-mcp.json`, with `data_dir` defaulting to `./data` inside the agentchattr checkout, and passes it as `--mcp-config <file>` (`wrapper.py:289-298`, `100-124`):
  ```json
  {"mcpServers": {"<project .mcp.json servers...>": {...},
                  "agentchattr": {"type": "http", "url": "http://127.0.0.1:8200/mcp",
                                  "headers": {"Authorization": "Bearer <16-byte hex token>"}}}}
  ```
  - It merges the project's `.mcp.json` servers into this file and drops any `agentchattr` entry from them (`wrapper.py:85-97`).
  - The docstring says this allows `--strict-mcp-config`, but the flag is never passed (VERIFIED: `launch_args = [flag, str(settings_path)]` only, `wrapper.py:298`). Claude therefore loads project servers twice, once natively and once through the file. How it resolves the same-name duplicates is unknown.
- **No writes to `~/.claude`, `~/.claude.json`, settings, hooks or plugins for Claude.** By contrast, Gemini, CodeBuddy, Copilot and agy get real-user-config writes. This makes the Claude footprint the cleanest of the shipped harnesses.
- **Environment.** `CLAUDECODE` is stripped, through `env -u CLAUDECODE` inside the tmux command (`wrapper.py:723`, `wrapper_unix.py:209-222`), so the wrapper can be launched from inside a Claude session. Extra CLI args pass through (`wrapper.py:605`, `382`).
- **Process model.** Claude always runs inside agentchattr's own tmux session. It is auto-restarted 3 s after it exits, but only while the user's terminal is still attached and `--no-restart` is not set; after a detach the wrapper just waits for the session to end and then stops (`wrapper_unix.py:236-273`). This is a hosting requirement: Claude cannot join from an existing terminal pane and keep its wake.
- **Manual alternative.** `claude mcp add agentchattr --transport http http://127.0.0.1:8200/mcp` (`README.md:309-312`) writes to Claude's own config and gets no token. That makes the session unauthenticated, and an unauthenticated `sender="claude"` is rejected with "authenticated agent session required" (`mcp_bridge.py:185-186`). If a user did both, a second unauthenticated `agentchattr` server could shadow the authenticated one. Which one wins is unknown.

## Token cost

Estimates below assume about 4 chars per token (INFERRED). Character counts were measured.

| Item | Cost | Evidence |
|---|---|---|
| MCP `instructions` | 5,736 chars, about 1,400 tokens, once per session. Claude cuts server instructions at 4,096 chars (primer §4), so about 1,640 chars are never seen (see Limitations) | measured by AST; primer §4 |
| 12 tool schemas | 3,928 chars of docstrings (measured by AST), plus parameter schemas, about 1,000-1,300 tokens. Under tool search they are deferred, so the cost is paid on first load rather than up front. README claims "~850" | measured; `README.md:274` |
| Wake prompt (user turn) | about 85 chars, about 20 tokens, plain. Every 10th trigger and every rules change adds the full RULES text; ROLE is added every time | `wrapper.py:531-560` |
| Model turn to call `chat_read` | one assistant tool call plus the result. README says "30 + 40 per message" of framing | `README.md:275` |
| Reply through `chat_send` | about 45 tokens of overhead | `README.md:276` |
| Quiet turn (no mail) | **zero injection.** Nothing runs on turns without a mention: no hooks, no polling inside the turn. Any `chat_read` the model makes anyway costs about 30 tokens plus the anti-poll text | VERIFIED by absence |
| Busy-turn wake | The wake prompt enters the running turn as queued user input. The model usually interrupts its plan to `chat_read`. Each mention then costs at least one extra tool round trip inside a turn that was already working | INFERRED, primer §7 |

Compared with herdr-threads, per-message framing is heavier: a JSON object with 6 or more keys per message, against our one header line. Each delivery also costs a full model round trip, because the model has to call `chat_read` (rule 3 in our contract avoids that round trip).

## Limitations and bugs

1. **`chat_send` moves the sender's read cursor past unread messages.** VERIFIED code: `mcp_bridge.py:348` `_update_cursor(sender, [msg], channel)` sets the channel cursor to the id of the agent's own new message. The failure sequence (INFERRED) is:
   - Claude reads #general and its cursor lands on 10;
   - while it works, others post 11 and 12;
   - it replies (id 13) and its cursor jumps to 13;
   - its next `chat_read(channel="general")` returns nothing after 13.

   Messages 11 and 12 are never delivered by cursor, and the wake for them (if they @mentioned Claude) produces "No new messages. Do not poll". The same applies to `chat_propose_job` (`:384`).
2. **Backlog skip.** `msgs[-limit:]` followed by moving the cursor to the last id silently skips older unread messages once more than 20 have piled up (`mcp_bridge.py:655-663`). VERIFIED.
3. **Server instructions are truncated for Claude.** At 5,736 chars against a 4,096 cap, the cut falls inside the Summaries paragraph. The Summaries tail, all of the Jobs guidance and the "Proposing Jobs" five-check rule are lost. So is the `[suggestion]` prefix convention, which appears nowhere else (the `chat_send` docstring does not mention it, `mcp_bridge.py:203-218`). VERIFIED for the length; the cap is from primer §4 (stated in the MCP doc). That Claude loses exactly this tail is INFERRED.
4. **A stale bearer token after re-registration breaks Claude until restart.**
   - On a 409 heartbeat, the wrapper re-registers, gets a new token and rewrites the `--mcp-config` file (`wrapper.py:780-787`, `680-690`).
   - Claude read that file once, at launch, and its own comment admits as much: "Most CLIs won't re-read mid-session" (`wrapper.py:679`).
   - Claude's MCP client keeps sending the old header, so every tool call returns "Error: stale or unknown authenticated agent session. Re-register and retry." (`mcp_bridge.py:173-174`). The model has no means of re-registering.
   - Proxy-mode harnesses such as Codex avoid this because the proxy's token is updated in place (`wrapper.py:707-709`). Claude is on the non-proxy path (`wrapper.py:639`).

   VERIFIED code; the user-visible effect is INFERRED.
5. **No idle gate and no draft guard** (see the Wake section). This is weaker than our contract's rule 4. It can submit a human's draft, or answer a dialog.
6. **Wakes collapse to the last channel within a poll window, and are lost on races, restarts and tmux failures** (see Coalescing and loss).
7. **The wake prompt is not content-free.** RULES and ROLE text are typed as user input. Past about 800 chars, Claude collapses the paste into `[Pasted text]` and treats it as pasted content: Claude is "told ... to follow instructions inside it only where the message you typed asks it to" (primer §7). Here *nothing* was typed except the paste, so the trust Claude places in long rule sets or session prompts is uncertain. INFERRED.
8. **Wake-driven compliance.** Injection depends on the model obeying "use mcp to read". Nothing checks that it did. There is no ACK, and the rules-sync receipt is sent on typing, not on reading (`wrapper.py:559`).
9. **Hosting lock-in.** Claude must run in agentchattr's tmux session and cannot be wired to an existing pane. Detaching keeps it running, but the wrapper's restart loop owns the process.
10. **Unauthenticated loopback trigger surface.** The queue file sits in a local directory and the wrapper trusts every line in it. Anyone who can write `data/` can make Claude run a custom `prompt`, typed as the user. This is consistent with a same-user local trust model. VERIFIED: `wrapper.py:519-527`.

## Clever ideas worth stealing

1. **Bracketed paste for typed wakes** (`wrapper_unix.py:64-136`): `load-buffer`, then `paste-buffer -p -d`, then a delay that grows with length, then Enter. A failure is treated as "do not retry, it may have arrived". Our Native wake text is short (`herdr-threads: wake up`), so the split-read bug does not bite today. If a fallback ever types longer text into Claude, this is the fix, and their real-tmux raw-reader test (`tests/test_inject_transport.py`) is a good model for a regression test. INFERRED relevance.
2. **Escalating empty-read replies** that tell the model to stop polling (`mcp_bridge.py:666-679`). This is a cheap guard against a model that loops on `inbox` after "nothing new". Our quiet-path wording could adopt the "do not poll, wait for the next prompt" idea in one short line.
3. **Reply-target defaulting:** `chat_send` without a channel goes to the channel or job the sender last read (`mcp_bridge.py:223-236`). It removes a common class of model mistakes. Note that bug 1 above lives right next to it, so copy the defaulting, not the cursor side effect.
4. **Periodic re-injection of shared rules** on an epoch change, or every N wakes (`wrapper.py:542-560`). This counters rules decaying out of attention after compaction. If we wanted something similar, it belongs in the SessionStart `compact` check-in, not in wake text (R18). Their per-agent "rules seen at epoch" freshness report (`app.py` `/api/rules/freshness`) is a nice operator view.
5. **Loop guard by per-channel hop count**, reset by any human message (`router.py:57-81`). This is a simple brake on agent-to-agent ping-pong that wakes repeatedly.
6. **Activity via a screen-hash diff with harness-specific noise filters** (`wrapper_unix.py:139-183`), plus marking the agent busy *before* injecting (`trigger_flag`, `wrapper.py:503-506`). This is crude, but it shows that a "busy since wake" latch avoids an idle misread while the model thinks before its first output. Our mod's `ASSUMED_BUSY_IDLE_MS` serves the same purpose.
7. **Stripping `CLAUDECODE`** so a Claude can be launched from inside a Claude session (`wrapper.py:723`). This is relevant to our `launch` paths.

## Bottom line for herdr-threads

For Claude, agentchattr is the "no harness integration" baseline: MCP tools for content, with tmux paste plus Enter as the only wake. It shows what we gain from the mod path:

- in-turn push without a tool round trip (`tool.call` context);
- an idle-only `$.prompt.submit` wake with a draft hold;
- bounded, paged, ACKed delivery.

It also confirms that our hooks-only fallback (Herdr typing) is in the same family as agentchattr's wake but stricter, because it has a composer check and `clear_key`. None of the stronger Claude primitives in the primer (`asyncRewake`, native cross-session messaging, channels, plugin monitors) are used by agentchattr. It offers no alternative technique for Claude that we lack. Its value to us is in the transport details (bracketed paste) and the model-guidance details (anti-poll text, reply defaulting), plus three bugs to avoid: cursor-on-send, backlog skip and oversized instructions.

## Verification

Adversarial pass on 2026-10-23 against the clone at `d775776a` (read only, nothing executed except `python3 -I` AST reads) and the GitHub API.

- Checked and confirmed: HEAD date, `VERSION` 0.5.0, 1558 stars, MIT, v0.5.0 published 2026-07-26 (API); `config.toml:10-14`, launchers (incl. `.bat` twins) and the skip-permissions flag; `_BUILTIN_DEFAULTS` for claude; queue entry shape (`agents.py:32-54`); the `_queue_watcher` loop, wake strings, ROLE/RULES/identity-hint logic, newline flattening and the 0.5 s / 1 s sleeps (`wrapper.py:473-571`); rules 160-char cap with no count cap (`rules.py:9`, `:100-107`, `:120`); `rules_refresh_interval` default 10 (`app.py:2125-2130`); the full tmux paste sequence, 5 s timeouts, no-retry docstring and activity hasher (`wrapper_unix.py:23`, `64-183`); `tests/test_inject_transport.py:1-21`; cursor, backlog-skip and cursor-on-send code (`mcp_bridge.py:345-348`, `384`, `566-572`, `648-663`); empty-read escalation; job reads; `ACTIVITY_TIMEOUT = 8`; summary cap; stale-token error and the unauthenticated-family rejection (`mcp_bridge.py:173-186`); `--strict-mcp-config` never passed (`wrapper.py:298`); token is `secrets.token_hex(16)` (`registry.py:28`); queue cleared on start (`wrapper.py:719-721`); README token table and manual `claude mcp add`; loop guard (`router.py:52-81`, `app.py:863-873`); `_MCP_INSTRUCTIONS` is 5,736 chars, the 4,096 cut lands in the Summaries paragraph (starts at 3,883), Jobs starts at 4,569 and `[suggestion]` at 5,577.
- Changed: coalescing rule (fields are taken per field from the last line that carries them, with prompt > job_id > channel precedence, not all from the last line); restart loop (only while attached and without `--no-restart`); tool-docstring size now the measured 3,928 chars; added the server's actual trigger gates (`agents.is_available` = registered); `CLAUDECODE` is stripped for every agent, not only Claude.
- Not re-verified here (taken from the primer): Claude's 4,096-char instruction cap, 800-char paste collapse, busy-input queueing and MCP output caps.
