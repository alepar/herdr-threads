# mcp_agent_mail x OpenAI Codex CLI

- Upstream: https://github.com/Dicklesworthstone/mcp_agent_mail (Python), clone HEAD `83ed8d7c97af67f1e6b4700bb8ace08739171ed3` (2026-10-23). Paths are relative to that clone root.
- Rust successor: https://github.com/Dicklesworthstone/mcp_agent_mail_rust. Only its README was read (raw `main`, 2026-10-23). Those references are prefixed `rust:README.md`.
- Harness reference: `../../primers/codex.md` (Codex CLI 0.163.0). Primer sections are cited as "primer §N".
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from those, not observed running. **unknown** means not found. Nothing from the clone was executed.

## 1. Support status

**Native, but injection only, and count-only. No wake.** VERIFIED.

- `scripts/integrate_codex_cli.sh` is the one-stop installer. Its banner (`:19-26`) lists endpoint detection, a bearer token, `codex.mcp.json`, a server launcher and the `PostToolUse` hook. The script body also does what the banner omits:
  1. Mints or reuses a bearer token (`:55-70`) and writes `codex.mcp.json` (`:80-115`).
  2. Registers an agent (`:144-171`).
  3. Installs a project-local Codex `PostToolUse` hook (`.codex/hooks.json`) that runs `check_inbox.sh` in JSON mode (`:186-267`).
  4. Upserts the MCP server into `~/.codex/config.toml` (`:277-476`) and overwrites `<project>/.codex/config.toml` (`:478-510`).
- The auto-detect installer runs it whenever `~/.codex` exists (`scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh:122,135-137`). VERIFIED.
- The older route was the Codex `notify` program (`scripts/hooks/codex_notify.sh`). It is deprecated and no longer installed. The reason given: "current Codex CLI spawns the `notify` program with stdin, stdout and stderr redirected to /dev/null and never shows or feeds its output to the model" (GH #273). VERIFIED `scripts/hooks/codex_notify.sh:1-10`, `scripts/integrate_codex_cli.sh:188-194,269-275`. This matches the primer: legacy `notify` stdio is null (primer §1.7).
- The repo has no tests for the Codex hook path. A grep of `tests/` for `integrate_codex_cli`, `inbox_wrapper` or `check_inbox` finds nothing. VERIFIED.
- Rust successor: the README documents Codex only as MCP config (`[mcp_servers.mcp_agent_mail] url = "http://127.0.0.1:8765/mcp/"` plus optional `http_headers`). Its curl installer "auto-configures detected Codex CLI configs for HTTP MCP URL mode". VERIFIED `rust:README.md:302,455-464`. Whether the Rust installer also writes a Codex hook is unknown: the README does not mention one.

## 2. Injection mechanism

### 2.1 What gets into context

The only push into the Codex model's context is one **unread-count reminder**, delivered through `PostToolUse` `hookSpecificOutput.additionalContext`. Message bodies never come through the hook. The model must call the `fetch_inbox` MCP tool (or read `resource://inbox/...`) to see content. VERIFIED.

### 2.2 Code path

1. **Hook registration** (`scripts/integrate_codex_cli.sh:230-264`). The installer merges this entry into `<project>/.codex/hooks.json` under `hooks.PostToolUse`:
   ```json
   { "matcher": "Bash", "hooks": [ { "type": "command", "command": "'<abs>/.codex/hooks/inbox_wrapper.sh'", "timeout": 10 } ] }
   ```
   The merge goes through `json_append_hook` (`scripts/lib.sh:422-442`). It needs jq and de-duplicates on the substring `inbox_wrapper.sh` in an existing `command`. Without jq, and with an existing file, it only prints the entry for a manual edit. VERIFIED.
2. **Wrapper** (`integrate_codex_cli.sh:213-228`). This is a mode-0700 script that exports the identity and secrets, then `exec`s the copied hook script:
   ```bash
   export AGENT_MAIL_PROJECT='${TARGET_DIR}'
   export AGENT_MAIL_AGENT='${_AGENT}'
   export AGENT_MAIL_URL='${_URL}'
   export AGENT_MAIL_TOKEN='${_TOKEN}'
   export AGENT_MAIL_REGISTRATION_TOKEN='${_REG_TOKEN:-}'
   export AGENT_MAIL_HOOK_FORMAT='json'
   export AGENT_MAIL_INTERVAL='120'
   exec '${INBOX_HOOK}' "\$@"
   ```
   The wrapper path is added to `.gitignore` (`:228`). The wrapper exists so that secrets never land in `hooks.json` (`:211-212`). VERIFIED.
3. **Hook body** (`.codex/hooks/check_inbox.sh`, a copy of `scripts/hooks/check_inbox.sh`):
   - It ignores the Codex hook stdin payload entirely. It never reads `session_id`, `tool_name` or `turn_id`. VERIFIED by absence of any stdin read in `scripts/hooks/check_inbox.sh:1-186`.
   - It exits 0 silently when the project or agent is unset or looks like a placeholder (`:50-64`).
   - **Rate gate** (`:66-80`): the state file is `/tmp/mcp-mail-check-<agent>`. If fewer than `INTERVAL` seconds (120) have passed, it exits. Otherwise it writes `NOW` to the file *before* fetching.
   - **Fetch** (`:95-110`): it sends a one-shot JSON-RPC `tools/call fetch_inbox` with `{project_key, agent_name, registration_token, limit:10, include_bodies:false, unread_only:true}` via `curl -s --max-time 3`.
     - This works without an MCP `initialize` handshake because the default path `/api/` is mounted as a **stateless** FastMCP app with `json_response=True`. A header-fixup wrapper adds the `Accept: application/json, text/event-stream` header (`src/mcp_agent_mail/http.py:1130-1150,1758,1773-1786`).
     - Loopback requests get the bearer header synthesized when `HTTP_ALLOW_LOCALHOST_UNAUTHENTICATED` is set, which is the default (`src/mcp_agent_mail/config.py:430`, `http.py:1851-1858`). VERIFIED.
   - **Auth**: a stateless request has no bound session, so `_authenticate_agent` requires the `registration_token` and compares it with `hmac.compare_digest` (`src/mcp_agent_mail/app.py:5357-5427`). VERIFIED.
   - **Count** (`check_inbox.sh:128-158`): it parses `structuredContent.result` or the legacy `content[0].text`. It computes `total` and `urgent`, where urgent means importance `high` or `urgent`.
   - **Emit** (`:160-172`):
     ```bash
     printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":%s}}\n' ...
     ```
     The text is one of two strings:
     - `You have N unread message(s) in your inbox. Use fetch_inbox to read them; reading or acknowledging a message stops this reminder.`
     - The same with ` (U urgent/high priority)` inserted after "inbox".

     VERIFIED.
4. **Codex side**: `PostToolUse` `additionalContext` is added to model context after the tool result; plain stdout from PostToolUse is ignored. VERIFIED (primer §1.4). That it reaches the next sampling request within the same turn is INFERRED (the primer's table says only "context"). The repo's comment makes the same point, written for Claude (`check_inbox.sh:30-36,167-170`).

### 2.3 When it fires

- It fires only after a tool whose name matches the (unanchored) regex `Bash`. Codex reports its shell tool to hooks as `tool_name: "Bash"`: VERIFIED in herdr-threads' live 0.158.0 PreToolUse captures (`tests/fixtures/codex-0.158.0-live/run1.pre-tool-use.root.json` and siblings) and relied on by our `^Bash$` matcher (primer §10). That this exact PostToolUse hook fires is INFERRED, since upstream has no test.
- It never fires at turn start: there is no `UserPromptSubmit` or `SessionStart` hook for Codex, unlike the Claude installer, which adds SessionStart (`scripts/integrate_claude_code.sh:226`; overview §Injection mechanism). VERIFIED by `integrate_codex_cli.sh` writing only `PostToolUse`. It does not fire after `apply_patch`, MCP tool calls (including `fetch_inbox` itself), web search, or a pure-reasoning turn: INFERRED, on the assumption that Codex reports those tools under names not containing `Bash` (their hook tool names were not checked).
- At most once per 120 s per agent name, machine-wide, because the rate file lives in `/tmp`. VERIFIED.
- Net effect: an agent that is coding mostly with `apply_patch`, or that is between turns, sees nothing. INFERRED.

### 2.4 Size bounds

- The reminder is fixed at about 129-156 bytes. Measured: the shortest form (N=1) is 129 bytes and the longest (N=10, U=10) is 156 bytes, before JSON escaping. That is far below Codex's roughly 2,500-token spill threshold (primer §1.4). VERIFIED.
- The count is capped at 10 by `limit:10`. With 37 unread messages the model is told "10". INFERRED from `check_inbox.sh:97`.
- The follow-up `fetch_inbox` is bounded by `limit` (default 20, clamped to 1000) and `include_bodies` (default false). No per-body truncation was found (`app.py:9454-9516`). VERIFIED for limit and bodies; truncation is unknown, as in the overview.

### 2.5 Framing

There is no message framing in the hook: it is a count, an urgency count, and one instruction sentence. The words "reading or acknowledging a message stops this reminder" are repeated in every reminder. That sentence is static explanation paid on every delivery, which is what our rule (a) forbids. VERIFIED text; the judgement is INFERRED.

### 2.6 De-duplication and acknowledgement

- **Re-announce suppression** comes from `unread_only=true` (GH #274, `check_inbox.sh:90-94`). A message stops counting only once it has `read_ts`, set by `mark_message_read` or `acknowledge_message`.
- **Bug: the reminder text is misleading.** `fetch_inbox` explicitly does **not** mark mail read: "A bare `fetch_inbox` call does NOT mark messages read" (`app.py:9466-9481`). The reminder says "Use fetch_inbox to read them; reading ... stops this reminder". A model that does what it is told, and calls `fetch_inbox` without a follow-up `mark_message_read` or `acknowledge_message`, is reminded again every 120 s for the same mail. VERIFIED text and semantics; the nag-loop consequence is INFERRED.
- Between rate windows there is no dedupe: every 120 s with unread mail produces another reminder. Nothing tracks "already offered in this turn or generation". VERIFIED.
- ACK is a separate, explicit MCP tool call (`acknowledge_message` sets `read_ts` and `ack_ts`, `app.py:9776-9850`). The hook never ACKs and never reports delivery. VERIFIED.

### 2.7 How the model's own MCP calls authenticate (affects delivery)

- The installer points Codex at `http://<host>:<port>/api/` (`integrate_codex_cli.sh:52`, default `HTTP_PATH="/api/"` at `config.py:396`), which is the **stateless** mount. The comment at `http.py:1137-1146` says stateless mode "never issues an `Mcp-Session-Id` header, so session-bound agent authentication ... could never persist across HTTP tool calls". The stateful mount is `/mcp`.
- INFERRED consequence: every protected call the Codex model makes (`fetch_inbox`, `acknowledge_message`, `send_message`) must carry its `registration_token` in the arguments (`app.py:5401-5421` raises AUTHENTICATION_REQUIRED otherwise). The token therefore sits in the model's context for the whole session. The only token-free fallback is a server-wide `MCP_AGENT_MAIL_WINDOW_ID` window identity (`app.py:5402-5412`, `config.py:597`), which is a server setting, not per session. The Python installer moved Codex *from* `/mcp/` *to* `/api/` on purpose ("older installs wrote /mcp/ but the server defaults to /api/", `integrate_codex_cli.sh:286-287`); the Rust README configures Codex at `/mcp/` (`rust:README.md:461`). Whether the Rust `/mcp/` is stateful like the Python `/mcp` alias is unknown.
- Neither Codex config gets an auth header. `~/.codex/config.toml` gets only `url` (`integrate_codex_cli.sh:345-346,449-454`). The project `.codex/config.toml` gets `url` with the comment "localhost allowed without Authorization" (`:494-497`). Both rely on the loopback bypass (`config.py:430`). The bearer token is embedded only in `codex.mcp.json` (`:80-115`). INFERRED: Codex does not read `codex.mcp.json`, because its MCP config lives in `config.toml` `[mcp_servers]` (primer §4). The file is dead weight for Codex, but it holds a live secret, set to mode 0600 (`set_secure_file`).

### 2.8 Identity binding (affects delivery)

- The hook's identity is the agent the **installer** registered: `register_agent` with no name, `program:"codex-cli"`, `model:"gpt-5-codex"`, `task_description:"setup"` (`integrate_codex_cli.sh:144-158`). The server auto-generates the name. The installer prints "Your agent name is: X" to the human (`:513-515`). VERIFIED.
- Nothing tells the Codex *session* that name or token. Codex gets no SessionStart hook, and the AGENTS.md blurb says "call `ensure_project`, then `register_agent`" (`README.md:169`). `register_agent` without `name` creates a **new** random identity unless the server has `MCP_AGENT_MAIL_WINDOW_ID` set (`app.py:6139`, `3353-3360`). To reuse the installer's name, the caller must present that agent's token (`app.py:6217-6227`), which exists only in the 0700 wrapper. VERIFIED.
- INFERRED consequence: a Codex session that follows the blurb registers as a different agent from the one the hook watches. Peers who mail the session's name get no reminder, and the hook reminds about the installer identity's mail, which the session cannot read without that token. This works only if the human copies the printed name, and somehow the token, into the session, or if the model reads `.codex/hooks/inbox_wrapper.sh` with Bash.
- One identity per project checkout. Two Codex panes in one project share the hook, the identity and the `/tmp` rate file, so whichever pane runs Bash first after the window opens consumes the reminder. Subagent sessions are unknown: the hook does not look at `session_id`. INFERRED.

## 3. Wake mechanism

**None.** VERIFIED by absence. Nothing in the Python source or scripts types into a pane, calls `codex queue`, uses the app-server, or consumes the signal files (overview §Wake).

| Aspect | Finding |
|---|---|
| Mechanism | None in OSS. The README points to a "historical zsh loop from the tweet thread" that broadcasts prompts into Codex panes via tmux. It is not in the repo. The commercial Companion's "Message Stacks" is a closed alternative (`README.md:47-52,329-333`). VERIFIED that both are referenced; how they work is unknown. |
| Signal file | When `NOTIFICATIONS_ENABLED=true` (default false), `send_message` (through `_deliver_message`, `app.py:5837-5848`) writes `{signals_dir}/projects/<slug>/agents/<agent>.signal` (`storage.py:3616,3659`), and `fetch_inbox` clears it (`app.py:9548-9550`). Default off: `config.py:515`. There is no consumer for Codex or any other harness. VERIFIED (overview). |
| Idle/busy detection | None. |
| Draft safety | None needed, since nothing types. INFERRED for the tmux loop: a blind broadcast into a Codex TUI with a live draft appends to it. During a running turn, Enter submits a **steer** of the active turn instead of starting a new one (primer §7). A fast typed burst followed by Enter within 120 ms becomes a newline, not a submit (primer §7, paste-burst). |
| Latency | An idle Codex session never learns of mail until a human starts a turn **and** that turn runs Bash **and** the 120 s window is open. A busy session waits up to 120 s plus the time to its next Bash call. INFERRED. |
| Failure modes | Server down, curl timeout (3 s), placeholder config, a missing registration token (`integrate_codex_cli.sh:165-168` warns "will silently no-op") and `isError` all exit 0 silently. The rate file is written **before** the fetch (`check_inbox.sh:79-80`), so a failed fetch still uses up the 120 s window. Untrusted or changed hooks are skipped by Codex until a human reviews them in `/hooks` (primer §1.1). VERIFIED script behaviour; Codex trust behaviour per primer. |

**A missed opportunity.** The deprecated `notify` route was triggered by `agent-turn-complete` (`codex_notify.sh:12-19`), which is exactly a native idle edge per thread. Primer §1.7 notes `notify` "gives a turn-complete (idle) edge per thread". Upstream dropped it because its *output* cannot reach the model. They did not repurpose it as an idle signal that could drive a wake, for example `notify` → mail pending → `codex queue --thread <thread-id> --message ...`. INFERRED.

## 4. Setup and installation footprint on the harness

Everything below is written by `scripts/integrate_codex_cli.sh` after an interactive `confirm` (skipped with `--yes`). VERIFIED by line.

| Path | Content | Lines |
|---|---|---|
| `~/.codex/config.toml` (**user-level, real config**) | Upserts `[mcp_servers.mcp_agent_mail] url = ...`. Optionally adds `[mcp_servers.morph-mcp]` with `MORPH_API_KEY` in plain text. Backed up first. Written atomically by a regex-based TOML line editor. | `:277-476` |
| `<project>/.codex/config.toml` | **Overwritten** (with a backup), not merged: `[mcp_servers.mcp_agent_mail] url`, plus Morph if a key exists. Any other project-local Codex settings are lost, except in the backup. | `:478-510` |
| `<project>/.codex/hooks.json` | PostToolUse `Bash` entry merged in (with jq), mode 644. The command is an **absolute path**. | `:230-265` |
| `<project>/.codex/hooks/check_inbox.sh` | Copy of the hook script, +x. | `:200-209` |
| `<project>/.codex/hooks/inbox_wrapper.sh` | Mode 0700. Holds the project, agent, URL, bearer token and registration token. Gitignored. | `:211-228` |
| `<project>/.gitignore` | Appends `.codex/hooks/inbox_wrapper.sh` under a comment that says "# Claude Code local settings (contains secrets)" (copy-paste from the Claude helper). | `:228`, `scripts/lib.sh:446-462` |
| `<project>/codex.mcp.json` | Mode 0600, with the bearer token embedded. Not read by Codex (INFERRED, §2.7). | `:80-115` |
| `<repo>/.env` | `HTTP_BEARER_TOKEN` if one was generated. | `:62-70` |
| `<repo>/scripts/run_server_with_token.sh` | Server launcher. | `:117-120` |
| Server DB | A registered agent per install run (`task_description:"setup"`). | `:144-148` |

Harness-side trust:
- Codex loads project-local hooks and config only once the project's `.codex/` layer is trusted. The new hook then needs review and trust by hash in `/hooks` (`:266-267`; primer §1.1).
- INFERRED: the trusted hash covers the hook *definition*, which is a fixed wrapper path, not the wrapper's contents. Re-running the installer, which rotates the agent name and registration token and re-copies `check_inbox.sh`, therefore does not trigger re-review. That is convenient for upgrades, but it lets a trusted hook change behaviour without review.
- Running the installer again registers yet another agent (new name and registration token; the bearer token is reused if one exists, `:56`) and rewrites the wrapper, so the hook silently moves to a new identity. INFERRED from `:144-148` (no `name` passed) and `:213-226`.

## 5. Token cost

| Situation | Cost | Status |
|---|---|---|
| Session start | The full MCP tool catalogue: 51 `@mcp.tool` definitions in `app.py` with long docstrings (the `fetch_inbox` docstring alone is about 40 lines, `app.py:9466-9505`), plus a 4-sentence server `instructions` string (`app.py:5221-5226`). Tool filtering (off by default, `TOOLS_FILTER_ENABLED`, `config.py:507`; profiles `full`, `core`, `minimal`, `messaging`, `custom`) can cut this by "up to ~70%" (`config.py:127-157`). The AGENTS.md blurb (about 2.9 KB, `README.md:153-188`) is paid in every session that includes it. Exact token count unknown. | VERIFIED counts; size INFERRED |
| Quiet turn (no mail) | **0 model tokens.** The hook prints nothing. Machine cost: one bash, python3 and curl per Bash call, at most once per 120 s. | VERIFIED `check_inbox.sh:160` |
| Turn with unread mail | About 35-40 tokens of reminder (129-156 bytes), at most every 120 s. | VERIFIED bytes; tokens INFERRED |
| Per message read | At least one extra model tool round trip. The `fetch_inbox` call is about 100 bytes of arguments including the registration token. The result is JSON metadata per message (`id, subject, from, created_ts, importance, ack_required, kind, read_at`), plus `body_md` only if `include_bodies=true`, so reading bodies needs a second call or a refetch. Then **another** call (`mark_message_read` or `acknowledge_message`) per message to stop the reminder. So about 2-3 model tool calls per message, versus 0 in-turn calls for herdr-threads bodies on Claude, and 1 (`inbox`, which displays and ACKs) on Codex. | VERIFIED tool shapes; call count INFERRED |
| Nag cost | While mail stays unread, about 40 tokens every 120 s of Bash activity, indefinitely. | INFERRED |

## 6. Limitations and bugs

1. **No wake.** An idle Codex session never sees mail. VERIFIED.
2. **Count only.** Content always costs an extra round trip, and the reminder caps N at 10. VERIFIED / INFERRED.
3. **Narrow trigger.** Bash-only `PostToolUse`. Turns that use only `apply_patch` or MCP tools, and turn starts, get nothing. Codex supports `UserPromptSubmit` and `SessionStart` context (primer §1.4), which upstream does not use for Codex. VERIFIED.
4. **The 120 s floor is machine-wide per agent name, and the window is consumed before the fetch.** A message sent one second after a check waits up to two minutes. A failed fetch also eats the window. VERIFIED.
5. **The reminder says `fetch_inbox` stops it, and it does not.** Only `mark_message_read` or `acknowledge_message` set `read_ts`. This can produce a standing nag. VERIFIED / INFERRED.
6. **Identity split between hook and session.** The installer registers one name, and the blurb tells the session to register another (§2.8). Mail for the session's name never triggers the reminder. INFERRED, high confidence.
7. **One identity per project checkout.** Multiple Codex panes in one repo collide on the identity and the rate file. VERIFIED design; consequence INFERRED.
8. **Stateless `/api/` endpoint for the model.** The registration token has to live in model context, and session binding never persists (§2.7). INFERRED from `http.py:1137-1146`.
9. **Project `.codex/config.toml` is overwritten, not merged** (`integrate_codex_cli.sh:490`). VERIFIED.
10. **Writes the user's real `~/.codex/config.toml`** with a hand-rolled regex TOML editor, which can include a third-party API key in plain text. VERIFIED. The edit is restricted to the agent-mail and Morph sections; robustness against unusual TOML is unknown.
11. **`hooks.json` holds an absolute path** to a gitignored wrapper. If the file is committed, other checkouts get a hook that points at a missing file. Codex then shows it as untrusted, or as failing silently. INFERRED.
12. **No `session_id`, `turn_id` or `tool_name` use.** No per-session dedupe and no subagent filtering. VERIFIED by absence.
13. **Silent failure everywhere** (`exit 0`). Nothing tells the operator or the model that delivery is broken. The installer warns once at install time about a missing token (`:165-168`). VERIFIED.
14. **No tests for the Codex path.** VERIFIED (grep).
15. **Shared Codex app-server daemon** (primer §2.1, §9): if Codex runs hooks in the daemon's process tree, the wrapper's baked-in env is unaffected, since identity is hard-coded and not pane-derived. Paradoxically, that makes this hook immune to the env-leak problem that herdr-threads works around with `--no-daemon`. INFERRED.

## 7. Clever ideas worth stealing

Upstream code is under MIT plus a rider that restricts OpenAI, Anthropic and their agents (overview §License). Ideas only; no code.

1. **Stateless one-shot JSON-RPC endpoint for hooks** (`http.py:1137-1150`). Hooks can call the server with a single POST and no handshake, while real clients get the stateful `/mcp`. Our hooks already talk to our daemon directly, so this mostly confirms our design. The split mount, which keeps old one-shot clients working while adding sessions, is a neat compatibility trick.
2. **Secrets in a gitignored wrapper, a stable command string in the trusted hook.** The trusted hash stays stable across token rotation (§4). If herdr-threads ever needs per-install parameters in a Codex hook, a stable command plus a changing side file avoids forcing a `/hooks` re-trust on every setup. The trade-off is that a reviewed hook can change behaviour unreviewed, which needs to be acknowledged.
3. **`unread_only` as the reminder dedupe key.** Re-announcing stops once state changes server-side, not on a client-side cursor. We already decide in the daemon (our digest token and the `Unchanged` path). This confirms that keying on canonical read state is the right shape.
4. **Rust `inbox-events` / `fetch_inbox_events`: a durable, body-free, oldest-first delivery cursor for monitors.** It advances only after processing and has explicit `cursor_expired` / `cursor_ahead` errors (`rust:README.md:744-761,782-784,1560`). This is a clean contract for an external watcher, such as a Codex `notify` or app-server bridge, that converts events into `codex queue` wakes. It is worth comparing with our watch protocol's resume semantics.
5. **Lesson confirmed independently.** Codex `notify` output never reaches the model (GH #273). The converse, which upstream missed, is worth our attention: `notify` `agent-turn-complete` (or app-server `thread/status/changed` → `idle`) is a free, precise **idle edge** per thread. A herdr-threads Codex adapter could use it to schedule a pending p0 wake right after the turn ends, through `codex queue` (server-side idle check, no draft clobbering; primer §3), instead of polling Herdr `agent_status` plus a composer read. INFERRED; it needs the queue reachability experiment that primer §3 lists first.
6. **What not to copy:**
   - Count-only reminders that force tool round trips.
   - A Bash-only trigger.
   - Rate limiting that consumes the window before the fetch.
   - Identity baked in at install time and disconnected from the live session.
   - Static instruction prose repeated in every reminder.

## Verification

Checked against clone HEAD `83ed8d7c`, the Rust README (raw `main`, 2026-10-23), the Codex primer and herdr-threads' Codex fixtures. About 60 claims checked; none of the core findings was refuted (count-only `PostToolUse` reminder, no wake, stateless `/api/`, `fetch_inbox` not marking read, installer identity split, overwritten project config).

Changed:
- §1: the installer's printed five-step list (`:19-26`) did not match the list given here; I rewrote it from the script body, with line ranges.
- §1, §2.7: Rust README line ranges (`455-464`, `/mcp/` at `:461`).
- §2.2 step 4: "developer context" and "next sampling request within the turn" are not in primer §1.4 for PostToolUse. The first is reworded; the second is now INFERRED.
- §2.3: the `Bash` tool name is now backed by herdr-threads' live fixtures rather than the primer. The apply_patch/MCP exclusion is now INFERRED, and the Claude SessionStart citation is fixed.
- §2.4, §5: reminder size corrected to 129-156 bytes (the 155 bytes was for U=3, not the longest form).
- §2.7: the claim that upstream "noticed" was refuted. The Python installer deliberately moved Codex from `/mcp/` to `/api/` (`:286-287`). I also added the server-wide window-identity fallback.
- §2.8: blurb line `README.md:169` (not 167); the name-less `register_agent` caveat for `MCP_AGENT_MAIL_WINDOW_ID`; and corrected `app.py` ranges.
- §3: signal-file emission cited to `_deliver_message` and `storage.py`.
- §4: a re-run reuses an existing bearer token; only the agent name and registration token change.
- §5: tool filtering also needs `TOOLS_FILTER_ENABLED` (default false), and the blurb is about 2.9 KB, not 2.4 KB.

