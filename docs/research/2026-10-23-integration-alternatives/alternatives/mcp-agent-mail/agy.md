# mcp_agent_mail x Antigravity CLI (agy)

- Upstream: https://github.com/Dicklesworthstone/mcp_agent_mail. The Python clone is at `83ed8d7c97af67f1e6b4700bb8ace08739171ed3`. Paths are relative to the clone root.
- Rust successor: https://github.com/Dicklesworthstone/mcp_agent_mail_rust. Only its README (raw `main`, fetched 2026-10-23) was read. Those citations are written `rust:README.md:N`.
- The agy facts come from the primer `../../primers/agy.md`, cited as `primer §N`. Read it alongside this report.
- Tags:
  - VERIFIED: read in source or docs.
  - INFERRED: reasoned from those sources.
  - unknown: no source answers it.

## Support status

**Not supported.** Neither the Python repo nor the Rust README mentions agy or Antigravity.

- VERIFIED:
  - A case-insensitive grep of the whole clone for `antigravity` returns 0 hits, and the word `agy` returns none.
  - The Rust README's supported-agent list is "Claude Code, Codex CLI, Gemini CLI, Oh My Pi (OMP), GitHub Copilot CLI, and any MCP-compatible client" (`rust:README.md:17`).
  - The Rust README's Gemini section configures only `~/.gemini/settings.json` (`rust:README.md:466-485`).

**The nearest integration is Gemini CLI, and it does not reach agy.** `scripts/integrate_gemini_cli.sh` writes Gemini CLI files, and agy does not load any of them as config. Of the files it writes, the hook script itself is the only one an agy hook could call (see "Setup/installation footprint" below). Each item below lists what the script writes, then what agy reads.

1. **MCP config.**
   - Script: `~/.gemini/mcp.json` (`:107-136`) and `~/.gemini/settings.json` `.mcpServers` (`:259-279`). Both use the Gemini key `httpUrl` (comment `:85`, `:237-238`).
   - agy: reads `~/.gemini/config/mcp_config.json` or `.agents/mcp_config.json`, and calls the URL key `serverUrl(url)` (primer §4).
   - VERIFIED (script). INFERRED that agy ignores both files: neither path is in agy's documented list.
2. **Hook location.**
   - Script: `~/.gemini/settings.json` `.hooks` (`:283-295`).
   - agy: reads hooks from `~/.gemini/config/hooks.json`, `.agents/hooks.json` or a plugin `hooks.json` (primer §2.1). The docs even list `~/.gemini/antigravity-cli/settings.json` as "not loaded" for hooks.
   - VERIFIED (script). INFERRED for agy.
3. **Hook schema.**
   - Script: `{"hooks":{"SessionStart":[{"matcher":"","hooks":[…]}], "BeforeTool":…, "AfterTool":…}}` (`:283-295`).
   - agy: the top level maps a hook *name* (a namespace) to its config, and the event keys are `PreToolUse`/`PostToolUse`, not `BeforeTool`/`AfterTool` (primer §2.1).
   - A third-party source says agy accepts unknown event keys but never fires them (primer §2.1).
   - So even if someone copied this block into `~/.gemini/config/hooks.json`, nothing would fire. One malformed key can also silently drop the whole file, which would take any other hooks in it, ours included, down with it (primer §2.1). INFERRED.
4. **Tool matcher.**
   - Script: `"shell|run_command"` (`:292`). `run_command` happens to be agy's shell tool name (primer §2.1).
   - agy: `PostToolUse` output is `{}` only, so it has no injection path (primer §2.3).
   - The inbox reminder was designed for an after-tool event. agy's after-tool event cannot carry it whatever the matcher. VERIFIED (primer §2.3, docs).
5. **Output format.**
   - Script: `INBOX_CHECK_CMD` does not set `AGENT_MAIL_HOOK_FORMAT` (`:228`). So `check_inbox.sh` prints its plain-text banner (`scripts/hooks/check_inbox.sh:173-182`).
   - The only alternative is the Claude envelope `{"hookSpecificOutput":{…"additionalContext":…}}` (`:171-172`).
   - agy: needs `{"injectSteps":[…]}` from `SessionStart`/`PreInvocation`/`PostInvocation` (primer §2.3), and it silently ignores malformed output (primer §2.3).
   - Neither format is something agy parses. VERIFIED (script). INFERRED (agy).

**An accidental partial install is likely on an agy-only host.**

- The auto-installer treats any `~/.gemini` directory as "Gemini installed": `HAS_GEMINI=0; [[ -d "${HOME}/.gemini" ]] && HAS_GEMINI=1`. It then runs `integrate_gemini_cli.sh --yes`. VERIFIED: `scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh:124`, `:145-147`.
- agy creates `~/.gemini/antigravity-cli/` (primer §0, §7).
- So on an agy host the installer will:
  - write inert Gemini config;
  - register a server identity with `program:"gemini-cli", model:"gemini"` (`integrate_gemini_cli.sh:173-175`);
  - put plaintext bearer and registration tokens into `~/.gemini/settings.json` (`:228`, `:251-295`).
- All of that is useless to agy. INFERRED.

**What remains on agy is generic MCP-client use.** It works only if the user hand-writes an agy MCP entry. See the generic assessment in each section below.

## Injection mechanism

### What upstream has (for the harnesses it supports)

Injection is pull-only. Message content enters a model's context only as the **result of a tool call the model chooses to make**:

- `fetch_inbox` (`src/mcp_agent_mail/app.py:9446-9558`), and
- `resources/read resource://inbox/{agent}…` (`app.py:13981`), and
- `macro_start_session`, which returns an inbox snapshot (`app.py:9873-9890`, `inbox_limit=10` at `:9884`, fetched via `_list_inbox` at `:9930-9933`).

The only push is a count-only reminder from a hook (`scripts/hooks/check_inbox.sh`):

```sh
ARGS_JSON="{…\"limit\":10,\"include_bodies\":false,\"unread_only\":true}"      # :97-99
RESPONSE=$(curl -s --max-time 3 -X POST "${URL}" … \
  -d "{\"jsonrpc\":\"2.0\",…\"method\":\"tools/call\",\"params\":{\"name\":\"fetch_inbox\",…}}") # :103-110
MSG_TEXT="You have ${MSG_COUNT} unread message(s) in your inbox. Use fetch_inbox to read them; …" # :164
```

VERIFIED. The details:

| Property | Upstream behaviour | Evidence |
|---|---|---|
| When it fires | After a shell tool call (`Bash` on Claude, `shell\|run_command` on Gemini). Rate-limited by `/tmp/mcp-mail-check-<agent>` to once per `AGENT_MAIL_INTERVAL` (120 s). | VERIFIED `check_inbox.sh:66-80`, `integrate_gemini_cli.sh:228`, `:292` |
| Payload | A count plus an urgent/high count. No ids, senders or bodies. | VERIFIED `check_inbox.sh:128-165` |
| Bounding | `limit:10`, so the count saturates at 10 (INFERRED). `fetch_inbox` caps `limit` at 1000 (`app.py:9510-9512`, `_validate_limit` `app.py:1549-1556`) and never truncates bodies: `_message_to_dict` copies `body_md` whole (`app.py:1843-1861`). | VERIFIED |
| Framing | `fetch_inbox` returns a JSON list. Each row is `_message_to_dict`'s `{id, project_id, sender_id, thread_id, reply_to, topic, subject, importance, ack_required, created_ts, attachments, [body_md]}` (`app.py:1844-1860`), to which `_list_inbox` adds `from` (plus `from_project*`/`from_address` for cross-project senders, `app.py:1894-1905`), `kind` and `read_at` (`app.py:4650-4664`; docstring `app.py:9496-9498`). TOON output is optional (`format`). | VERIFIED |
| Ordering | `.order_by(desc(Message.created_ts))`, newest first. | VERIFIED `app.py:4628-4629` |
| De-dup | The reminder uses `unread_only=true`, so read or ACKed mail stops re-triggering it (GH #274, `check_inbox.sh:90-100`). `fetch_inbox` itself never marks mail read ("without mutating read/ack state", `app.py:9468`). Repeated `fetch_inbox` calls re-return the same bodies. | VERIFIED |
| ACK | `acknowledge_message` sets read+ack. `mark_message_read` is separate. Each is its own tool call by the model. | VERIFIED (overview, `app.py:9776-9800`) |
| Auth on the model's path | `_authenticate_agent` accepts the caller if the MCP session is already bound to the agent. Otherwise it requires `registration_token` (or a `MCP_AGENT_MAIL_WINDOW_ID` window-identity match). The binding key is `ctx.session_id`, or a random `orphan:` key when there is none. | VERIFIED `app.py:5357-5428`, binding key `app.py:5263-5287` |

### What applies on agy today

**Nothing is injected automatically.**

- No upstream hook targets agy. VERIFIED, see "Support status".
- So the only injection path on agy is the generic one: the user adds the server to `~/.gemini/config/mcp_config.json`. The model then calls `fetch_inbox` because a rule, skill or AGENTS.md blurb told it to (`README.md:153-188`). INFERRED.
- When it fires: only when the model decides to poll.
- Mid-turn: there is no external trigger, and agy delivers MCP results only as tool results (primer §1, §4).

**Session identity on agy.** Whether agy's Streamable HTTP client keeps and re-sends `Mcp-Session-Id` across tool calls is unknown (primer §4 documents transports, not session handling). Over stdio the issue does not arise: the Python server also has a `serve-stdio` command (`src/mcp_agent_mail/cli.py:1245-1255`, VERIFIED), and `ctx.session_id` is then a stable per-process uuid (comment `app.py:5265-5268`, VERIFIED). The rest of this paragraph applies to HTTP only.

- If it does not, every `fetch_inbox` resolves to a fresh `orphan:` key (`app.py:5263-5287`). The model must then pass `registration_token` in the tool arguments on every call.
- That puts the secret into the agy transcript `brain/<id>/.system_generated/logs/transcript_full.jsonl` (primer §7). INFERRED.
- The Rust successor's `MESSAGING_SESSION_IDENTITY` exists to keep the token out of the transcript, but it depends on the same session-id echo (`rust:README.md:612-656`).

**A port of their reminder to agy.** INFERRED; upstream has not done it.

- Event: `check_inbox.sh` would have to move to `PreInvocation`, the only injection event that fires before every model call, including those after a tool result (primer §2.2, §2.5). `PostInvocation` also accepts `injectSteps` but fires after the call and is unexercised (primer §2.3).
- Output: it would have to emit `{"injectSteps":[{"ephemeralMessage":"You have N unread…"}]}`. This needs a third `AGENT_MAIL_HOOK_FORMAT` branch. Neither existing branch (`check_inbox.sh:167-183`) produces it.
- With `ephemeralMessage`, the nag would not persist in the transcript (primer §2.3). That fits a count-only reminder well.
- Cost: `PreInvocation` fires before every model call, but the 120 s rate file keeps most of them to a `stat` and an early exit (`:70-77`).
- Identity: it would still be one baked identity per hook config (`AGENT_MAIL_AGENT` in the command string, `integrate_gemini_cli.sh:228`). agy subagents fire hooks too, under their own `conversationId` with no role field (primer §7), so a child would also get the parent's reminder.

## Wake mechanism

**None, on any harness.** The OSS code never types into a terminal and never calls a harness API.

- VERIFIED by absence (overview "Wake mechanism (general)"): no `tmux`, `send-keys` or MCP `notifications/` in `src/`.
- The only wake-adjacent artefact is the **signal file**:
  - Off by default: `NOTIFICATIONS_ENABLED` defaults to false (`src/mcp_agent_mail/config.py:515`).
  - Written to `{signals_dir}/projects/<slug>/agents/<agent>.signal` with `{timestamp, project, agent, message:{id, from, subject, importance}}` (`src/mcp_agent_mail/storage.py:3616-3680`).
  - Debounced in memory, default 100 ms (`storage.py:3646-3655`, `config.py:518`).
  - Deleted when `fetch_inbox` runs (`app.py:9548-9550`, `storage.py:3689-3715`).
- No consumer ships. The docstring just says agents "can watch these files using inotify/FSEvents/kqueue" (`config.py:160-176`). VERIFIED.
- The README hands wake off to two places, neither public (VERIFIED):
  - "the historical zsh loop from the tweet thread", which is not in the repo (`README.md:333`);
  - the commercial Companion stack's "Message Stacks" ("no manual tmux choreography required", `README.md:50`, `:333`).

On agy specifically:

| Question | Answer | Status |
|---|---|---|
| Exact mechanism | None. An agy session idle at `>` stays idle until a human or another tool types into it. | VERIFIED (absence) |
| Idle/busy detection | None. | VERIFIED (absence) |
| Draft safety | Not applicable: nothing types. | VERIFIED (absence) |
| Latency | Unbounded for an idle agy session. In a busy session the agent learns of mail only when it next chooses to poll. Even a hypothetical PreInvocation port would add up to 120 s from the rate limit. | INFERRED |
| Failure modes | Silent: a sender has no way to know the recipient never woke. The ACK-TTL scanner (default off, 1800 s) only logs `ack-warning` server-side (overview "Delivery guarantees"). | VERIFIED (scanner); INFERRED (consequence) |

What a user could bolt on for agy with upstream primitives (INFERRED; none of it exists):

1. **A signal-file watcher that types a wake prompt into the agy pane.** It would need everything herdr-threads' `AgyComposer` already does:
   - parse the composer for an empty `>`, the footer `? for shortcuts` versus `esc to cancel`, and the permission-prompt menu, where typing could approve a command (primer §8);
   - avoid the false idle Herdr reports during subagent work (primer §8, Herdr #2354);
   - avoid wake text that starts with `?`.
   - The signal file also carries no priority threshold, so every message would wake.
   - It is deleted only by `fetch_inbox`, which does not mark mail read. A woken agent that reads but never ACKs leaves the DB "unread" while the signal is gone. The two notions of "pending" diverge.
2. **The Rust `am inbox-events` / `fetch_inbox_events`.**
   - A body-free, oldest-first durable cursor feed, described as "restart-safe inbox monitors" (`rust:README.md:742-762`, `:782-784`). VERIFIED (README).
   - This is a better watcher input than the signal file: it is durable, ordered and per-recipient, and has no 100 ms debounce loss.
   - It still has no idle detection and no agy wake channel.
3. **agy-native self-wake.**
   - The model could `/schedule` a cron "check agent mail" (primer §6). That is a poll loop with a per-firing model cost.
   - A `Stop` hook could long-poll `fetch_inbox` and answer `decision:"continue"` with the message as `reason` (primer §6). That keeps the session spinning, bounded by the hook timeout (default 30 s) and an unknown continue cap. It is not an idle wake.
4. **A blocking MCP tool** (agy's default tool timeout is 180 s, primer §4). Upstream has no long-poll tool: `fetch_inbox` returns at once.

## Setup/installation footprint on the harness

**Upstream writes nothing that agy reads.** For reference, here is what `integrate_gemini_cli.sh` writes, all VERIFIED. That is what an agy user gets if the auto-installer misfires on their `~/.gemini`.

| Path | Content | Lines |
|---|---|---|
| `<project>/gemini.mcp.json` | `{"mcpServers":{"mcp-agent-mail":{"httpUrl":…,"headers":{"Authorization":"Bearer …"}}}}`, `chmod` via `set_secure_file` | `:63-97` |
| `~/.gemini/mcp.json` | The same server entry without headers, plus an optional `morph-mcp` | `:107-140` |
| `~/.gemini/settings.json` | A jq merge: `.mcpServers["mcp-agent-mail"]`, plus `.hooks.SessionStart` (two `uv run python -m mcp_agent_mail.cli …` commands), `.hooks.BeforeTool` (`write_file\|replace\|edit_file`) and `.hooks.AfterTool` (three entries, the inbox check among them). It **replaces** these hook arrays outright (`=`), so the user's own SessionStart/BeforeTool/AfterTool hooks are lost; a backup is made first. | `:230-349` |
| `<project>/.gemini/hooks/check_inbox.sh` | A copy of the hook script | `:209-219` |
| `<project>/scripts/run_server_with_token.sh` | The server launcher | `:99-102` |
| Server DB / Git archive | A new agent identity (`program gemini-cli`) | `:162-198` |

Tokens are embedded in plaintext in the hook command string (`AGENT_MAIL_TOKEN='…' AGENT_MAIL_REGISTRATION_TOKEN='…'`, `:228`).

**A minimal hand-made agy setup** would be one entry in `~/.gemini/config/mcp_config.json`:

```json
{"mcpServers":{"mcp-agent-mail":{"serverUrl":"http://127.0.0.1:8765/api/","headers":{"Authorization":"Bearer <token>"}}}}
```

The default path is `/api/` (`config.py:396`), and the key `serverUrl` comes from primer §4. Add the AGENTS.md blurb on top. INFERRED; untested. Permission rules for the tools would go in `~/.gemini/antigravity-cli/settings.json` as `mcp(mcp-agent-mail/fetch_inbox)` (primer §4, §7).

## Token cost per message and per quiet turn

All numbers are byte counts from the clone. Token figures assume about 4 characters per token (INFERRED).

**Per message (generic MCP use on agy):**

1. A `fetch_inbox` call. The arguments are about 150-250 bytes: an absolute `project_key`, the agent name, maybe a `registration_token`.
2. Its result. Each message is a JSON object of about 14 keys (more for cross-project senders). That is roughly 300-400 bytes without a body, so about 75-100 tokens per message. INFERRED from `app.py:1844-1860`, `:4650-4664`.
3. With `include_bodies=true`, the full `body_md` comes back untruncated. Without it, the model needs a second call for bodies.
4. An `acknowledge_message` call per ACK-required message.
5. A sender's "overseer" messages also carry a fixed preamble with emoji in the body (overview).

So a typical message costs **2-3 model tool round trips**, each of which re-sends the conversation to the model. There is no in-turn body injection. VERIFIED (mechanism). INFERRED (sizes).

A hypothetical agy PreInvocation reminder would add a 129-byte sentence (measured with `N=3`, `check_inbox.sh:164`), about 30 tokens, at most once per 120 s.

**Per quiet turn:**

- **Hooks:** zero. Nothing is installed on agy, and the reminder is silent when there is no unread mail (`check_inbox.sh:160`).
- **MCP tool schemas.**
  - The server exposes 49 `@mcp.tool` functions. Their docstrings total **34,628 characters**, about 8.7k tokens (measured statically with Python `ast` over `app.py`, without importing it). `send_message` alone is 5,993 characters, `fetch_inbox` 1,759.
  - FastMCP uses the docstring as the tool description when none is passed. `fetch_inbox` passes none (`@mcp.tool(name="fetch_inbox")`, `app.py:9446`); 10 lifecycle/admin tools (e.g. `health_check`, `retire_agent`, `purge_old_messages`) do pass `description=`, so for them the docstring count overstates the description. INFERRED from FastMCP convention; the `description=` count is VERIFIED by the same `ast` scan. The parameter schemas come on top.
  - Whether agy sends every MCP tool definition with every model request (eagerly) or loads them lazily is unknown (primer §4 is silent). If eagerly, this is the dominant cost, about 9k+ tokens per model call, cache-dependent.
  - `TOOLS_FILTER_ENABLED=true` with `TOOLS_FILTER_PROFILE=minimal|core` can cut it ("up to ~70%"). It defaults off (`config.py:128-157`, `:507`).
- **Static instructions:**
  - the AGENTS.md blurb, 2,794 bytes (`README.md:155-187`);
  - optionally `SKILL.md`, 13,599 bytes;
  - plus the server `instructions` string, about 300 bytes (`app.py:5221-5226`).

## Limitations and bugs you can see

1. **No agy support, and the Gemini path is wrong for agy on every axis:**
   - file (`~/.gemini/settings.json` versus `~/.gemini/config/hooks.json`);
   - URL key (`httpUrl` versus `serverUrl`);
   - event names (`AfterTool` versus `PostToolUse`);
   - output format (plain text or Claude envelope versus `injectSteps`);
   - the event's capability (agy's after-tool event cannot inject).
   - VERIFIED for the script side. INFERRED for the agy side, from primer §2 and §4.
2. **The auto-installer's `~/.gemini` probe misfires on agy-only hosts.** It mints a bogus `gemini-cli` identity and writes plaintext secrets into an inert file (`automatically_detect…sh:124`, `:145-147`). INFERRED consequence.
3. **The Gemini integration sends the reminder in text mode.** It never sets `AGENT_MAIL_HOOK_FORMAT=json`, yet the script's own header says plain after-tool stdout "does NOT reach the agent" on Claude (`check_inbox.sh:30-36`, `integrate_gemini_cli.sh:228`). Whether Gemini CLI surfaces plain stdout from AfterTool is unknown. On agy it certainly would not (primer §2.3).
4. **The installer clobbers user hooks.** It assigns `.hooks.SessionStart`, `.hooks.BeforeTool` and `.hooks.AfterTool` with `=`, not append (`integrate_gemini_cli.sh:284-295`). The same pattern in an agy `hooks.json` would delete other plugins' hooks. INFERRED.
5. **One identity per hook config**, baked in at install time (`:228`). agy subagents fire hooks under their own conversation ids with no role field (primer §7), so a port would remind children as the parent. INFERRED.
6. **The signal file has an off-spec path and a lossy lifecycle:**
   - The docstring says `{signals_dir}/{project_slug}/{agent_name}.signal` (`config.py:168`). The code writes `projects/<slug>/agents/<agent>.signal` (`storage.py:3659`). VERIFIED mismatch.
   - It holds only the most recent message.
   - The debounce is in-process memory (`storage.py:3620`), so it resets on server restart; a second message inside the 100 ms window writes no new signal (`storage.py:3651-3653`).
   - It is cleared by `fetch_inbox` rather than by read or ACK state (`app.py:9548-9550`).
7. **Newest-first inbox with no body truncation.** A long backlog arrives in reverse order, and a single huge body is returned whole (`app.py:4628`, `:1859-1860`). VERIFIED.
8. **The registration token can leak into the model's transcript** if the server is reached over HTTP and the agy MCP client does not keep the session id (see Injection). INFERRED. Whether it does is unknown; `serve-stdio` avoids it.
9. **No sender feedback when a recipient never sees mail**, because no wake exists and the ACK-TTL only logs. VERIFIED (overview).

## Clever ideas worth stealing

These are all INFERRED analysis. Do not copy code: the licence carries an OpenAI/Anthropic rider (overview "License").

1. **`unread_only` as the reminder's de-dup key.** The nag stops because the agent read the mail, not because of a separate offer ledger. Our ledger is stricter, with bounded re-offers and transcript confirmation. Their failure mode (GH #274, a standing nag every interval) is a good regression case to keep in mind for our re-offer limits.
2. **A durable, body-free inbox event cursor for monitors** (Rust `am inbox-events`: `next_cursor`, `oldest_available_cursor`, `cursor_expired`/`cursor_ahead`, `rust:README.md:742-762`). The contract is clean: persist the cursor only after processing, and recover explicitly from history-retention gaps. It is worth comparing with how our wake/notification paths resume after a daemon restart.
3. **Tool-surface profiles** (`TOOLS_FILTER_PROFILE=minimal|core`, `config.py:128-157`). An explicit knob for the per-request schema cost of an MCP surface. It is relevant if herdr-threads ever exposes MCP tools to agy, where tool definitions may ride every model call.
4. **`ephemeralMessage` is the right agy slot for a count-only nudge.** Upstream does not use it, but their count-only design maps naturally onto agy's one-request, non-persisted step (primer §2.3). For us, it suggests putting re-offers or check-in digests into `ephemeralMessage` while keeping bodies in the transcript-confirmable `systemMessage` (primer §11, item 3).
5. **The signal-file seam.** The server emits one debounced per-recipient file and leaves the wake policy to an external watcher. This decouples the server from harness specifics. For agy, the watcher would still need our composer parsing. Their omission of it shows that the parsing is the hard, valuable part.

## Verification

Adversarial pass on 2026-10-23 against the clone at `83ed8d7c`, the Rust README (raw `main`, refetched) and the agy primer. No clone code was executed; the `ast` docstring scan parsed `app.py` as text.

- Checked and held: zero `antigravity`/`agy` hits in the clone and the Rust README; Rust supported-agent list (`:17`) and Gemini section (`:466-485`); every `integrate_gemini_cli.sh` citation (paths, `httpUrl`, `program:"gemini-cli"`, `.hooks.*` assigned with `=`, `shell|run_command` matcher, plaintext tokens in `INBOX_CHECK_CMD`); the auto-installer's `~/.gemini` probe (`:124`, `:145-147`); `check_inbox.sh` rate limit, `unread_only`, count-only text and both output branches; the 129-byte reminder (re-measured); `fetch_inbox` "without mutating", signal clear, binding key and `_authenticate_agent`; 49 tools, 34,628 docstring characters, `send_message` 5,993, `fetch_inbox` 1,759 (re-measured); AGENTS.md blurb 2,794 bytes and `SKILL.md` 13,599 bytes; `NOTIFICATIONS_ENABLED`, `TOOLS_FILTER_ENABLED`, ACK-TTL (off, 1800 s, `config.py:550-551`) defaults; `/api/` default path; signal-file docstring/code path mismatch; Rust session identity (`:612-656`) and inbox-event cursors (`:742-762`, `:782-784`); primer facts for hook locations, schema, `PostToolUse` `{}`, `injectSteps` step types, Stop/`timeout`, 180 s MCP tool timeout, subagent hooks, Herdr #2354; `AgyComposer` exists (`src/harness/agy/composer.rs`).
- Corrected: `from`, `kind`, `read_at` are added by `_list_inbox`, not only listed in the docstring (per-message size estimate raised accordingly); `fetch_inbox` limit-cap, docstring, signal debounce/clear and `ToolFilterSettings` line ranges; ordering is a SQLAlchemy `order_by(desc(...))`, not literal SQL; the README's wake hand-off cites the "historical zsh loop from the tweet thread" at `README.md:333` (not a "tmux zsh loop" at `:47-52`); "only per-model-call injection event" narrowed to "only event before every model call" (`PostInvocation` also injects); "none passes a description" corrected (10 tools pass `description=`); "four written files" corrected.
- Added: the Python server's `serve-stdio` (`cli.py:1245`) sidesteps the HTTP session-id/token-in-transcript concern; the 100 ms debounce default (`config.py:518`).
