# agentchattr x OpenAI Codex CLI

- Upstream: https://github.com/bcurts/agentchattr (MIT). Clone examined: `scratchpad/alts/agentchattr`, HEAD `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10, "Ignore Codex idle input particles when detecting activity"), `VERSION` 0.5.0. GitHub API on 2026-10-23: 1558 stars, 239 forks, 18 open issues and PRs (GitHub's combined `open_issues_count`), last push 2026-09-10, latest release `v0.5.0`.
- Paths are relative to the clone root. **VERIFIED** means read in source, docs or issue text. **INFERRED** means reasoned from those. **unknown** means not found.
- Harness reference: `../../primers/codex.md` (Codex CLI 0.163.0). Read `overview.md` in this directory first. This file covers only what is specific to Codex, or what plays out differently on Codex.

## 1. Support status

**Native, built in.** VERIFIED:

- `config.toml:16-20` ships `[agents.codex] command = "codex"`.
- `wrapper.py:144-149` gives Codex a built-in MCP injection default:
  ```python
  "codex": {
      "mcp_inject": "proxy_flag",
      "mcp_proxy_flag_template": '-c mcp_servers.{server}.url="{url}"',
      # mcp_merge_project disabled — Codex reads .mcp.json natively, ...
  },
  ```
- Launchers: `macos-linux/start_codex.sh` (ends `.venv/bin/python wrapper.py codex`), `macos-linux/start_codex_bypass.sh:64` (`wrapper.py codex -- --dangerously-bypass-approvals-and-sandbox`), and Windows `windows/start_codex.bat` and `start_codex_bypass.bat`.
- Codex is the only provider with its own activity filter (`wrapper_unix.py:139-156`, `174-175`, tests in `tests/test_wrapper_activity.py`).
- The built-in default is chosen by the exact executable name, so a renamed identity such as `[agents.codex2] command = "C:\Tools\codex.exe"` still gets it (`wrapper.py:166-194`; test `tests/test_identity_contract.py:200-214`; PR #82).
- An open PR, #112, was live-tested against "Codex CLI 0.163.0" on native Windows. That is the same Codex version as our primer (VERIFIED, PR body).

No hooks, app-server, `codex queue`, `turn/steer`, `notify` or `codex exec` are used. A grep for `hook`, `app-server`, `queue --thread`, `exec` and `notify` in the Python sources finds nothing related to Codex (VERIFIED).

## 2. Injection mechanism

agentchattr never pushes message bodies into Codex. Content reaches the model in two steps: a typed wake prompt, then the model's own MCP tool call.

### 2.1 MCP wiring: a per-instance identity proxy and a `-c` override

1. `wrapper.py:616-625`: the wrapper calls `POST /api/register {base:"codex"}` and gets a name (`codex`, `codex-2`, ...) and a random bearer token (VERIFIED).
2. `wrapper.py:633-661`: `proxy_flag` mode makes `needs_proxy` true, so the wrapper starts an `McpIdentityProxy` on `127.0.0.1:<OS-assigned port>` in front of the real streamable-HTTP MCP server on `:8200`. Then `proxy_url = http://127.0.0.1:<port>/mcp` (VERIFIED).
3. `wrapper.py:312-317` expands the template and splits it on whitespace:
   ```python
   expanded = template.format(server=SERVER_NAME, url=proxy_url or "")
   launch_args = expanded.split()
   ```
   The result is `["-c", 'mcp_servers.agentchattr.url="http://127.0.0.1:NNNN/mcp"']`. `wrapper_unix.py:205-207` shell-quotes every argument into the tmux command, so Codex receives the TOML string literal with its quotes intact (VERIFIED; test `tests/test_identity_contract.py:338-352`).
4. The proxy (`mcp_proxy.py:149-186`) forwards every POST, GET and DELETE. It adds `Authorization: Bearer <token>` and `X-Agent-Token`, and in `_maybe_inject_sender` (`mcp_proxy.py:281-316`) it rewrites `params.arguments.sender` (or `name`) on each `tools/call` to the registered name. The server then resolves the bearer token to the instance and ignores any sender the model declared (`mcp_bridge.py:158-190`) (VERIFIED).

The result is that Codex's own config never contains the token. Identity survives renames: `set_runtime_identity` updates `proxy.agent_name` and `proxy.token` in place (`wrapper.py:707-709`) with no Codex restart (VERIFIED). No file is written for Codex (`inject_env = {}` and `settings_path = None`, test `:213-214`).

### 2.2 The wake prompt that carries the pointer

The server appends a trigger line to `data/<name>_queue.jsonl` for every @mention target that is registered, not `pending`, and allowed by the session turn guard, unless the loop guard has paused the channel (`app.py:852-898`, `agents.py:15-16,32-54`). The wrapper's watcher (`wrapper.py:473-571`) polls that file every 1 s, reads every line, truncates the file, and builds one prompt:

```python
prompt = f"use mcp to read #{channel} - you're mentioned, take appropriate action and respond"
# or, for a job: f"use mcp to read job_id={job_id} - you're mentioned in a job thread, ..."
# or a session-engine custom prompt (full text)
```
(`wrapper.py:526-531`, VERIFIED.) It then appends:
- `ROLE: <role>` if the instance has a role (`:536-540`).
- `RULES:\n<rule1>; <rule2>...` on the first trigger, when the rules epoch changes, or on every `refresh_interval`-th trigger (default 10, set by the server) (`:543-559`).
- A one-time `_IDENTITY_HINT` of 239 characters for multi-instance slots (`:422-427`, `:561-563`).

Newlines are flattened to spaces (`:564-567`). The prompt reaches Codex as a **user message**, through a tmux paste (§3).

### 2.3 The pull: `chat_read`

The model calls `chat_read(channel=...)` through Codex's MCP client. The handler is `mcp_bridge.py:575-689`:

- The cursor is per sender and per channel key (`__all__` for an unfiltered read), persisted to `data/mcp_cursors.json`. The first read returns the last `limit` (20) messages. Later reads return `get_since(cursor)`, then `msgs[-limit:]`, and move the cursor to the last returned id (`:648-663`).
- Payload: a JSON array of `{id, sender, text, type, time, channel[, attachments][, reply_to]}` (`_serialize_messages`, `:409-426`). Multi-instance agents get a prefix `[identity: codex-2 | label: ...]` (`:681-688`).
- Empty reads return escalating text: "No new messages. Do not poll — wait for your next prompt." up to "STOP." (`:666-679`).
- Job reads return a header entry with the job title and body, then the whole thread with no limit (`:597-637`).

Answers go out through `chat_send` (`mcp_bridge.py:193-`).

### 2.4 Properties

| Property | Finding |
|---|---|
| When it fires | Only when the model calls `chat_read`. That happens after a typed wake prompt, or on the model's own initiative. There is no turn-start or tool-boundary injection, because no Codex hooks are installed (VERIFIED absence). |
| Mid-turn | **Codex-specific accident.** Paste plus Enter while a Codex turn is running is a **steer** into the active turn, not a queued new turn (primer §7: Enter steers, Tab queues). So a wake that lands mid-turn reaches the model at its next sampling step as extra user input. INFERRED by combining `wrapper.py:482-567` (it injects with no busy check) with primer §7. Not observed. |
| Size bounds | None per message at HEAD. `chat_read` returns at most 20 messages but does not truncate bodies (VERIFIED, `:409-426`). Open PR #113 adds a 2,000-character text cap (VERIFIED, PR body). The Codex side caps MCP tool output through `tools.<t>.output_token_limit` (primer §4). agentchattr does not set it (VERIFIED absence of any `output_token_limit` or `tool_timeout` in the repo). |
| Framing | A JSON array with about 40 tokens of framing per message, the README's estimate (overview). No header line, priority or ACK marker. |
| Dedupe | Cursor-based. Wake lines are coalesced: every line read in one poll gives one injection, and the **last** line's channel, job and prompt win (`wrapper.py:488-531`). A mention in `#a` followed by one in `#b` within the same second produces only "read #b". INFERRED from that code; `#a` is still readable if the model reads all channels. |
| Acknowledgement | None. The only "seen" signal is the cursor, plus a rules-epoch sync report (`wrapper.py:454-471`). |
| Loss | (a) A backlog beyond 20 messages is skipped: the cursor jumps past messages it never returned (`:655-663`). (b) **`chat_send` moves the sender's cursor for that channel key to its own new message** (`mcp_bridge.py:348`, `_update_cursor(sender, [msg], channel)`; `:566-573`). Anything others posted between Codex's last read and its send is never returned. Open PR #112 calls this F1 ("mentions sent while an agent is busy are silently skipped", reproduced 3/3) (VERIFIED in source and PR). (c) The queue file is truncated at wrapper start (`wrapper.py:719-721`), and read-then-truncate (`:484-486`) can lose an append that races it (PR #112 F2, VERIFIED). |

## 3. Wake mechanism

### 3.1 Mechanism (VERIFIED, `wrapper_unix.py:64-136`)

Codex runs inside a dedicated tmux session, `agentchattr-<name>` (`wrapper.py:890,914`, `wrapper_unix.py:204,244-248`). The wrapper attaches the user's terminal to that session. Delivery is:

```python
tmux display-message -p -t <session> "#{pane_id}"          # resolve the pane once per delivery
tmux load-buffer -b agentchattr-inject-<pid>-<rand> -       # prompt on stdin
tmux paste-buffer -p -d -b <buf> -t <pane>                  # bracketed paste if the app enabled it
sleep(max(inject_delay, len(text) * 0.001))                 # default 0.3 s
tmux send-keys -t <pane> Enter
```

Each tmux call has a 5 s timeout. A failure is printed and **not retried**, because "a paste may have arrived" (`:76-83`).

On Codex this defeats the TUI's paste-burst heuristic in two ways (primer §7: an Enter within 120 ms of a fast burst becomes a newline). The text arrives as one bracketed paste, which Codex supports, and Enter follows at least 300 ms later. Either one alone would be enough (INFERRED). PR #3 (closed, not merged) proposed 150 ms delays between keystrokes for "text typed but Enter not registered" on Codex and Gemini (VERIFIED, PR body). Whether the current 0.3 s default descends from it is unknown.

On Windows, `WriteConsoleInputW` key records go into Codex's console. `wrapper_windows.py:343-350` notes that newer Codex builds call `SetConsoleMode` around frame draws and can strip `ENABLE_VIRTUAL_TERMINAL_PROCESSING`, so the wrapper re-asserts it every 10 ms (VERIFIED comments; not examined further).

### 3.2 Idle and busy detection

**Not consulted before a wake.** `_queue_watcher` injects whenever the queue file is non-empty (`wrapper.py:482-567`, no other condition, VERIFIED). Busy state drives only the UI pill:

- Every second, `tmux capture-pane -p` output is hashed. "Active" means the hash changed (`wrapper_unix.py:159-183`).
- **Codex filter** (`wrapper_unix.py:139-156`): Codex animates single-dot braille "particles" (`⠁⠂⠄⠈⠐⠠⡀⢀`) in and around its idle composer. Without the filter, an idle Codex would look busy all the time. The filter finds the last line whose text, after stripping particles, starts with the composer glyph `›`, blanks particles on that line, and clears the lines directly above and below if they hold only particles. Output above the composer, real braille, spinners such as `⠋ Working`, and typed text still count as activity (tests `tests/test_wrapper_activity.py:25-52`).
- The status is posted to `/api/heartbeat` and expires after `ACTIVITY_TIMEOUT = 8` s (`mcp_bridge.py:32`, `:762-779`). `agents.py:24` exposes `busy` only to the UI (`static/chat.js:1862`) (VERIFIED). `router.py` and `app.py` never read it.

Open PR #112 commit `1c6d8b6` (F7, "wake-up lost mid-turn") would make the watcher hold a wake while the agent is active or waiting on approval, with a 180 s starvation bound (VERIFIED, PR body; not merged at HEAD).

### 3.3 Draft safety

**None.** A wake pastes into whatever is in Codex's composer and presses Enter (INFERRED from `wrapper_unix.py:97-136`; the code never reads or clears the composer). What happens then:
- Idle composer with a human draft: the wake text is pasted at the cursor and the combined text is submitted as one user turn. The human's half-typed text is sent (INFERRED).
- Busy Codex: Enter turns the combined text into a steer of the running turn (primer §7), or it is lost if the composer is in a modal state, such as an approval prompt or a `/` popup (INFERRED).
- History: PR #1 proposed sending `Escape` before each injection to clear stacked input, and PR #3 proposed `C-u` line clearing. Both were closed without merging, and neither is in `wrapper_unix.py` at HEAD (VERIFIED: PR state and bodies via the GitHub API). Whether any draft wipe ever shipped is unknown (the clone is shallow).

### 3.4 Latency (INFERRED from the code)

The server appends to the queue file at once. The watcher polls every 1 s, then sleeps 0.5 s "to signal activity" (`wrapper.py:503-506`), makes two HTTP calls of up to 3 s each (`_fetch_role`, `_fetch_active_rules`), pastes, waits 0.3 s, and presses Enter. That is about 1 to 2 s to keystroke, then Codex's turn: one sampling round to decide to call `chat_read`, the MCP round trip through the proxy, then reasoning and `chat_send`.

### 3.5 Failure modes

- **Approval prompts.** The default launcher passes no approval flags. Whether Codex asks before each `chat_read` or `chat_send` MCP call depends on the user's Codex approval and MCP tool approval settings (`default_tools_approval_mode`, primer §4). The agentchattr default is unknown. While Codex waits on such a prompt, a pasted wake plus Enter could answer or disturb it (INFERRED). A `_bypass` launcher exists for this reason (VERIFIED). PR #112 F5 reports approval waits as invisible (VERIFIED, PR body).
- **Shared app-server daemon.** Since about 0.157, the Codex TUI attaches to a machine-wide daemon by default (primer §2.1). agentchattr does not pass `--no-daemon`. It is unknown whether a per-invocation `-c mcp_servers.agentchattr.url=...` override is applied per thread in the daemon or can collide when two Codex instances (`codex`, `codex-2`) share one daemon with different proxy URLs under the same server name. Not verifiable from this repo. It is the first thing to test.
- **Restarts lose the conversation.** When Codex exits, the wrapper relaunches a **fresh** `codex` 3 s later with the same flags (`wrapper_unix.py:236-273`). There is no resume. PR #3 (closed, not merged) describes an earlier Codex `resume_flag = "exec resume"` and proposed `resume --last` (VERIFIED, PR body); no resume flag exists at HEAD (VERIFIED: no `resume` in `wrapper.py` or `config.toml`). Identity on a manual `/resume` is model-driven through `chat_claim` (overview).
- **The agent shows as available before Codex is ready.** The instance is registered before the CLI starts, so a mention during Codex's startup, login or trust screen is pasted into that screen. This is open issue #90. Codex's own startup-draft buffer (primer §7) may save the text in some of those states (INFERRED).
- **Header defect (fixed).** Issue #80: the proxy once sent `Mcp-Session-Id` twice, which broke some clients during the handshake. Codex with the default config was named as a proxy-routed client (VERIFIED, issue). At HEAD, `_send_response_headers` lists the header once (`mcp_proxy.py:137-147`).
- **Proxy timeouts.** POST requests through the proxy time out after 30 s and GET streams after 300 s (`mcp_proxy.py:170`, `:200`). The proxy buffers the whole upstream POST response before replying. That works for agentchattr's short tools but would break streamed responses (INFERRED).
- **Server restart.** The upstream MCP session dies, and the proxy at HEAD does not re-handshake (PR #112 F4, observed with Claude: "MCP tools gone for the session"). Whether Codex's MCP client reconnects on its own is unknown.

## 4. Setup and installation footprint on Codex

VERIFIED:
- **No Codex config file is written.** Not `~/.codex/config.toml`, not `hooks.json`, no trust entries. Everything goes on argv: `codex -c mcp_servers.agentchattr.url="http://127.0.0.1:<port>/mcp" [user extra args]`.
- The wrapper strips `CLAUDECODE` (and any per-agent `strip_env`) from the environment through `env -u` inside tmux (`wrapper.py:723-724`, `wrapper_unix.py:209-222`).
- Runtime artefacts live only under agentchattr's `data/`: queue file, cursors, registry. A tmux session named `agentchattr-<name>`.
- The README's manual-registration section tells "Codex / other agents" to add the server to the project's `.mcp.json` (`README.md:314-325`). The code comment says the same ("Codex reads .mcp.json natively", `wrapper.py:147`). Our primer lists Codex's MCP config only as `[mcp_servers.<name>]` in `config.toml` (primer §4). In the sparse 0.163.0 source clone, `.mcp.json` appears only as a plugin's default MCP file (`codex-rs/ext/mcp/src/plugin/provider.rs:15`, `DEFAULT_MCP_CONFIG_FILE`); no project-root reader was found (VERIFIED for the checked-out paths). INFERRED: Codex does not read a project-root `.mcp.json`, so the README's manual path and the code comment are likely wrong for Codex.
- A user-level `[mcp_servers.agentchattr]` already defined with `command=` (stdio) would be merged with the `-c ...url=` override, giving an entry with both `command` and `url`. How Codex handles that is unknown (INFERRED edge case).

## 5. Token cost

| Item | Cost | Evidence |
|---|---|---|
| Static, per session | MCP server `instructions`: 5,736 characters (about 1,400 to 1,500 tokens), plus schemas for 12 tools (`chat_send`, `chat_read`, `chat_resync`, `chat_join`, `chat_who`, `chat_rules`, `chat_decision`, `chat_channels`, `chat_set_hat`, `chat_claim`, `chat_summary`, `chat_propose_job`) | VERIFIED length (`mcp_bridge.py:54-124`, measured); tool list `:943-946`. Token count INFERRED at about 4 characters per token. |
| Wake prompt | 80 characters (about 20 tokens) by default. It grows by the role text, by the full active rules on the first trigger, on an epoch change and on every 10th trigger, and by 239 characters once for the identity hint | VERIFIED `wrapper.py:531-563` |
| Per message delivered | At least one sampling round to call `chat_read`, the tool result (about 40 tokens of JSON framing per message plus the body, up to 20 messages, bodies not capped), then usually one more round for `chat_send`. The MCP instructions also invite `chat_join`, `chat_rules(list)` and `chat_summary(read)` "at session start" | VERIFIED `mcp_bridge.py:57,87,104`. PR #113 measured on Codex: **11 wakes and 97 calls** for one build round before its prompt rewrite, versus 4 wakes and 46 calls after, with input tokens down about 65% (VERIFIED, PR body; their measurement, on different tasks, "indicative rather than exact"). |
| Quiet turn | **Zero.** No hooks, so a Codex turn with no mention adds nothing to context | VERIFIED absence |
| Ack-only traffic | Every @mention wakes at full cost. There are no priorities or thresholds, and agent-to-agent pings are bounded only by the loop guard (`max_agent_hops` = 4) | VERIFIED `router.py:52-81`, `config.toml:99` |

## 6. Limitations and bugs visible for Codex

1. **No busy gate, no draft protection.** On Codex a mid-turn wake becomes a steer, and a human's draft gets submitted (§3.2, §3.3). This is the opposite of our delivery rule 4.
2. **The wake is user-role text that carries policy.** Rules and role are re-sent inside the user prompt. Codex treats that as a user instruction, which has more authority than hook-added context (primer §1.4 lists `SessionStart` `additionalContext` as developer context) (INFERRED).
3. **Cursor loss:** the backlog over 20 is skipped, and `chat_send` advances the read cursor past unread messages (`mcp_bridge.py:348`, `:655-663`).
4. **Wake loss:** coalescing keeps only the last channel; the queue is truncated at startup; read-then-truncate races; there is no retry on a tmux failure.
5. **No delivery to a busy Codex unless the steer accident happens.** Codex hooks (`UserPromptSubmit`, `PostToolUse`, `Stop` block) are unused, so a long turn that never calls `chat_read` sees nothing until the next wake.
6. **Screen-scrape activity is fragile against Codex TUI changes.** The filter depends on the `›` glyph and on the particle set. A new Codex animation would make an idle Codex look permanently busy again. That was the bug fixed at HEAD (INFERRED from the commit subject and tests).
7. **Fresh relaunch on exit** throws away the Codex conversation. Identity recovery after resume is left to the model.
8. **Behaviour against the shared daemon is untested** with several instances (§3.5).

## 7. Clever ideas worth stealing

1. **Token-free harness config through a per-instance identity proxy** (`mcp_proxy.py:281-316`, `wrapper.py:144-149`). The harness sees only `http://127.0.0.1:<port>/mcp`, and the proxy stamps identity and bearer on every call, with no config file and no secret in `~/.codex`. For herdr-threads, if we ever expose an MCP surface to Codex, a `-c mcp_servers.<x>.url=` (or `command=`) launch override in managed launch avoids writing `$CODEX_HOME/config.toml` at all. Caveat: the shared-daemon question in §3.5.
2. **Bracketed paste, then a delayed Enter** (`wrapper_unix.py:64-136`). The docstring documents a measured Claude Code truncation (a 1204-byte prompt arrived as its last 182 characters). For Codex it sidesteps the 120 ms paste-burst Enter suppression. Our `herdr-threads: wake up` is short, but this is the right pattern if a wake ever grows. It is also worth checking how Herdr's `agent.prompt` sends text.
3. **The Codex idle-animation filter** (`wrapper_unix.py:139-156`). If our Codex composer classifier (`src/harness/codex/composer.rs`) or Herdr's `agent_status` ever diff screens, Codex's braille particles around `›` must be masked. The test fixtures in `tests/test_wrapper_activity.py` are a compact capture of that grammar (`› Ask Codex to do anything` plus particles).
4. **Escalating empty-read replies** (`mcp_bridge.py:666-679`): a cheap, model-facing way to stop polling loops in a pull design.
5. **Not retrying after a partial keystroke failure** ("a paste may have arrived", `wrapper_unix.py:83`). It is honest about whether keystrokes can be retried. That matches our bias toward never double-typing.
6. **What not to copy, and what it confirms:** the measured token result in PR #113 (wakes cost far more than chat history) supports our choices of content-free wakes (R18), thresholds that let p1 mail stay silent, and in-turn hook delivery. agentchattr's own fix direction converges on fewer wakes and one read per wake.

## Verification

Adversarial pass on 2026-10-23 against the clone at `d775776`, the GitHub API and the Codex primer (plus the sparse Codex 0.163.0 source clone for `.mcp.json`).

Checked and held: HEAD, VERSION, stars/forks/push date; `config.toml` Codex entry and `max_agent_hops = 4`; `_BUILTIN_DEFAULTS["codex"]`, exact-executable provider resolution and its tests; registration, proxy start and `proxy_flag` expansion; the proxy's header forwarding, sender rewrite, 30 s / 300 s timeouts and single `Mcp-Session-Id`; token-based identity resolution; `chat_read` cursor, limit, escalating empty-read text, job reads and breadcrumb; `chat_send` cursor move; the queue watcher (read-then-truncate, coalescing, 0.5 s sleep, prompt text of 80 characters, role/rules/identity-hint suffixes, newline flattening); startup truncation; tmux paste-then-Enter, 5 s timeouts, no retry; the Codex particle filter and its tests; `busy` reaching only the UI; fresh relaunch after 3 s; MCP instructions length (5,736 characters) and the 12-tool list; launchers; PR #112 (F1, F2, F4, F5, F7) and PR #113 figures; issues #80 and #90; primer §1.4, §2.1, §4, §7.

Changed: identity hint is 239 characters, not about 270 (two places); PRs #1 and #3 were closed without merging, so the "removed draft wipe", "0.3 s default is the result" and "`resume_flag` fixed in #3" claims were reworded to what the PR bodies say; `open_issues_count` includes PRs; trigger conditions in `app.py` narrowed (registered, not pending, turn guard, loop guard); the `chat_send` cursor move is per channel key; the Windows `SetConsoleMode` note now cites `:343-350` and says what the comment says; the server-restart line no longer asserts Codex cannot reconnect; `.mcp.json` now carries Codex source evidence; session-name and PR #113 caveat citations tightened.
