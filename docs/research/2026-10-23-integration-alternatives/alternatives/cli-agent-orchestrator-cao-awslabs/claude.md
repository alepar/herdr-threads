# CAO (awslabs) x Claude Code: message injection and wake

- Upstream: https://github.com/awslabs/cli-agent-orchestrator. Depth-1 clone at `69909e98035905b612fd6b117abff8bf8876220f`. All paths below are relative to the clone root.
- Read with: `overview.md` (this directory) and `primers/claude.md`.
- Tags: **VERIFIED** means read in source or docs at the cited line. **INFERRED** means my reasoning, not stated anywhere. **unknown** means not found.
- Nothing from the clone was executed.

## 1. Support status

**Native, first-class.** VERIFIED:
- `src/cli_agent_orchestrator/providers/claude_code.py` (1,645 lines) is a full `BaseProvider`.
- `docs/claude-code.md` documents it.
- The e2e suites cover handoff, assign and send_message for `claude_code` (`docs/claude-code.md:154-172`).

How Claude Code is integrated, in one line: CAO launches the interactive TUI in a tmux window (or herdr tab) and treats it as a black box. It gives the model an MCP server for sending messages. It delivers incoming messages by typing them into the composer. It reads busy/idle state by regex-parsing the screen, or from herdr's `agent_status`. **No Claude Code hooks, mods, channels, native cross-session messaging, `claude agents --json` or OSC 7501 are used.** VERIFIED: a grep of `src/` for `UserPromptSubmit|PostToolUse|asyncRewake|SessionStart|"--settings"|notifications/claude/channel|CLAUDE_CODE_MESSAGING|claude agents|7501` finds only a comment about the spinner text "(running UserPromptSubmit hook)" (`claude_code.py:284`).

Claude Code is one of two providers with `accepts_input_while_processing = True` (the other is MiniMax Code, `providers/minimax_code.py:156-158`), which enables eager mid-turn delivery. The docs list only Claude Code. VERIFIED: `claude_code.py:1469-1476`, `docs/inbox-delivery.md:85-90`.

## 2. Injection mechanism

### 2.1 Path, end to end (VERIFIED)

1. **The sender model calls the MCP tool** `send_message(message, receiver_id=None)` on `cao-mcp-server`, a stdio subprocess of the sender's CLI (`src/cli_agent_orchestrator/mcp_server/server.py:926-958`).
   - Omitting `receiver_id` routes to the recorded caller (docstring `server.py:941-943`; the `caller_id` lookup in `_send_message_impl`, `utils/orchestration.py:2442`, `:2470-2495`).
   - The topology check allows only a child, the parent, or a discovered sibling (`server.py:945-948`; `utils/orchestration.py:2518-2523`).
2. **The suffix is appended** by `append_sender_suffix` (called at `utils/orchestration.py:2532-2538`, template at `:137-139`, join at `:212`):
   ```python
   return message + "\n\n" + template.format(terminal_id=terminal_id)
   # MESSAGE_SUFFIX_TEMPLATE =
   #   "[Message from terminal {terminal_id}. Use send_message MCP tool for any follow-up work.]"
   ```
   It is on by default (`CAO_ENABLE_SENDER_ID_INJECTION`, `:128`). `assign` uses `ASSIGN_SUFFIX_TEMPLATE` (`:133-136`).
3. **The HTTP POST** `{API_BASE_URL}/terminals/{receiver}/inbox/messages` carries **`sender_id` and `message` as query parameters** (`utils/orchestration.py:1424,1440-1445`).
4. **The server persists the row and delivers at once.** `create_inbox_message_endpoint` (`src/cli_agent_orchestrator/api/main.py:8373-8447`):
   - persists a PENDING row;
   - spawns `inbox_service.deliver_pending` on a worker thread;
   - answers once the paste has reached the pane, or after `INBOX_IMMEDIATE_DELIVERY_WAIT = 5.0` s (`constants.py:393`), whichever comes first.
5. **The status gate** in `_deliver_pending_locked` (`src/cli_agent_orchestrator/services/inbox_service.py:206-248`):
   - delivers if the status is IDLE or COMPLETED;
   - if it is PROCESSING, delivers only when `CAO_EAGER_INBOX_DELIVERY=true` (default `false`, `constants.py:322`) and the provider says `accepts_input_while_processing` (Claude: true once `_initialized`);
   - **never** delivers into WAITING_USER_ANSWER for Claude, because `blocks_orchestrated_input_while_waiting_user_answer` is True (`claude_code.py:1478-1499`; `inbox_service.py:242-245`).
6. **Mark, then send.** Rows are marked **DELIVERED before typing** (`inbox_service.py:250-257`). Contiguous same-sender runs are joined with `"\n"` (`:264-266`).
7. **`terminal_service.dispatch_input`** (`src/cli_agent_orchestrator/services/terminal_service.py:4527-4747`):
   - refuses ERROR state (`:4589-4594`) and an open dialog (`:4603-4609`);
   - on the first message after init only, prepends a `<cao-memory>` block when the memory backend returns one (`inject_memory_context`, `:387-456`, called at `:4620`);
   - re-checks that the pane's foreground process is not the bare shell (`_refuse_if_provider_gone`, `:4644`);
   - arms the status monitor and samples the "dispatch boundary" generation (`:4654-4690`);
   - snapshots the screen tail hash for staleness (`claude_code.py:1501-1514`);
   - finally calls `backend.send_keys(..., enter_count=2, force_bracketed_paste=True, submit_delay=2.0)` (`:4708-4715`). Claude does not override `paste_enter_count`, so it gets the base default of **2** (`providers/base.py:170-181`). `paste_submit_delay` is 2.0 s (`claude_code.py:1460-1467`).
8. **Typing into the pane.**
   - **tmux** (`src/cli_agent_orchestrator/clients/tmux.py:1754-1857`):
     - `send-keys -X cancel` leaves copy mode (`:1776-1780`);
     - `load-buffer`, then `paste-buffer`. On tmux < 3.7 the body is wrapped by hand in `ESC[200~ … ESC[201~` and pasted with `-r` (`:1800-1811`). On tmux ≥ 3.7 the raw bytes are pasted with `-p`, so tmux adds real markers only if the pane enabled DECSET 2004 (`:1812-1821`);
     - `sleep(2.0)`, then for each of the 2 Enters: copy-mode cancel and `send-keys Enter`, with 0.5 s between Enters (`:1836-1857`).
   - **herdr** (`src/cli_agent_orchestrator/backends/herdr_backend.py:645-705`):
     - `herdr pane send-text <pane> "\x1b[200~"+keys+"\x1b[201~"`;
     - a hard-coded `sleep(2.0)` (the provider's `submit_delay` is accepted but ignored, `:661-663`, `:698-699`); the wrap is skipped when the pane's foreground process is a bare shell (`:685-690`);
     - `herdr pane send-keys <pane> Enter`, twice with no gap (`:704-705`).

### 2.2 What the model sees

- **VERIFIED:** an ordinary **user turn** in the composer, whose text is `<body>\n\n[Message from terminal ab12cd34. Use send_message MCP tool for any follow-up work.]`. The first message after launch also has `<cao-memory>…</cao-memory>` prepended. Nothing reaches the model as a system reminder, hook context, tool result or channel tag.
- **INFERRED, from primer §7:** Claude Code collapses pastes over 800 chars or over 3 lines to `[Pasted text #N +k lines]`. It marks pasted content as "text you pasted from somewhere else" and, where that feature flag is on, tells the model to follow instructions inside it only where the typed message asks.
  - CAO's turn is 100% paste with no typed part. The `\n\n` suffix makes it at least 3 lines, so any multi-line body (or one over 800 chars) crosses the collapse threshold; a one-line short body gives exactly 3 lines and does not.
  - So under that flag, a CAO instruction may be read as quoted, untrusted material rather than a request. Not captured. CAO's docs and code don't mention it.

### 2.3 When it fires

- **Default:** only when the screen-parsed status is IDLE or COMPLETED, so it fires at turn boundaries (VERIFIED, `docs/inbox-delivery.md:17-19`).
- **Eager** (`CAO_EAGER_INBOX_DELIVERY=true`): also while PROCESSING.
  - Claude Code queues typed input during a turn and "passes it to Claude as soon as those tool calls finish, within the same turn" (primer §7, VERIFIED from Claude docs).
  - So eager mode is in effect **mid-turn injection after the current tool batch**, entering as a queued user message (INFERRED from the two facts).
  - Submit confirmation is skipped for eager deliveries (`inbox_service.py:300,328-330`).
- Triggers that reach this gate (VERIFIED): the immediate POST; status events `terminal.*.status` = IDLE/COMPLETED from the pipe-pane StatusMonitor (`inbox_service.py:109-151`); herdr `pane.agent_status_changed` idle/done (`services/herdr_inbox_service.py:1039`, routed to `deliver_pending`, `:931`); and a 30 s reconcile sweep for rows PENDING longer than 30 s (`inbox_service.py:553-588`; `constants.py:345,351`; loop `api/main.py:261`). The 5 s poll (`api/main.py:241-243`) covers OpenCode terminals only. `docs/inbox-delivery.md:11-12` still describes a 5 s `PollingObserver` log watchdog, but no `PollingObserver` exists in `src/`; the doc is stale.

### 2.4 Size bounds

- **No explicit limit on the body.** VERIFIED for the endpoint signature: a bare `message: str` query parameter, `main.py:8373-8379`. Other endpoints in the same file do use `max_length` (`:1260,1294`).
- **INFERRED:** because the body travels in the **URL query string**, the practical ceiling is the ASGI server's request-line limit (`uvicorn[standard]`, `pyproject.toml:46`), roughly tens of KB, beyond which the sender gets an HTTP 4xx. Exact value unknown.
- No truncation or continuation exists on the receiving side; the whole body is pasted.
- Drain mode joins up to 100 rows (`inbox_service.py:213`), but the status-event consumer always calls with `num_messages=1` (the default, `:160-167`, `:133`). So normally one message per wake.

### 2.5 Framing (VERIFIED)

- A trailing suffix, not a header. It carries no message id, no thread and no priority.
- The sender is identified only by the 8-hex terminal id.
- The kiro `@prompt` exception (`orchestration.py:148,208-211`) never applies to Claude.

### 2.6 Dedupe and acknowledgement (VERIFIED)

- **Dedupe** is structural:
  - a per-terminal lock serializes the read→mark→send sequence (`inbox_service.py:68-88`);
  - mark-before-send stops re-entrant status events from re-delivering (`:250-255`);
  - nothing is resent after a successful paste ("reverting it to PENDING would let the reconcile sweep paste it a second time", `:449-452`).
- GH #115 tracks a known residual duplicate-wakeup race (`docs/inbox-delivery.md:128`).
- **No receiver ACK.**
  - DELIVERED means "typed into the pane" (`inbox_service.py:180-186`).
  - A **submit confirmation** (non-eager tmux only) waits up to 8 s for a PROCESSING/COMPLETED/WAITING status whose output generation exceeds the dispatch boundary (`inbox_service.py:381-411`; `constants.py:378`).
  - If it sees none, it recovers up to 2 times (`constants.py:383`): a **bare Enter** if Claude's composer still holds the text, or a full re-send if the composer is readable and empty (`inbox_service.py:413-510`; `terminal_service.py:3344-3360,3416-3557`).
  - Claude's `extract_current_composer` reads the text between the two `─{8,}` rails of the boxed `❯` prompt and ignores the `Try "…"` placeholder (`claude_code.py:1525-1570`).
  - Confirmation is **skipped on herdr** (`inbox_service.py:301-317`).

## 3. Wake mechanism

### 3.1 Mechanism (VERIFIED)

Injection and wake are the same act. Pasting into an idle composer and pressing Enter submits a new turn.

### 3.2 Idle/busy detection for Claude

**tmux backend.** Regexes run over the raw `pipe-pane` stream after `strip_terminal_escapes` (`claude_code.py:1006-1294`). The layers, in order:
- a tail-hash staleness guard that returns PROCESSING while the screen hasn't changed since the dispatch (`:1049-1057`);
- a spinner-before-last-separator walk (`:1059-1100`);
- the new-TUI boxed `❯` detection (`NEW_TUI_BOX_PATTERN`, `:266-269`) plus a spinner directly above the box (`:1200-1228`);
- a 15 s "turn end grace" for the newest TUI, which draws no spinner while text streams (`TURN_END_GRACE_S`, `:241`; `_turn_still_in_flight`, `:1311`);
- background-task wait lines (`:221-224,1262-1271`);
- dialog footers for WAITING_USER_ANSWER, matched in the bottom 6 lines (`:1142-1186`);
- COMPLETED when the screen shows a completion summary or `⏺`/`●` plus a `❯` prompt (`:1273-1288`).

The comments document about 15 GitHub issues fixed against Claude TUI redesigns (for example GH #392, #407, #459, #865 and #908).

**herdr backend.** `herdr pane get` → `agent_status` (`herdr_backend.py:866-914`), mapped as working→PROCESSING, blocked→WAITING_USER_ANSWER, done→COMPLETED, idle→IDLE. After a dispatch, idle or done must persist for 10 s ("flush wait") before it counts as COMPLETED, and the idle path gives up after 300 s (`providers/base.py:541-647`).

### 3.3 Draft safety

**None for a human's draft.** No composer read happens before the paste: `dispatch_input` goes straight from the guards to `send_keys` (`terminal_service.py:4567-4715`). The composer reader is used only for post-hoc recovery.

The status logic doesn't treat a draft as busy either. `IDLE_PROMPT_PATTERN = r"[>❯][\s\xa0]"` (`claude_code.py:132`) matches `❯ my draft`, so a finished turn with a typed draft still reads COMPLETED. **INFERRED:** an operator typing into a Claude worker pane gets the inbox paste appended to the draft, and both are submitted together by the Enter.

What CAO does guard against (VERIFIED):
- tmux copy mode, cancelled before the paste and before each Enter (`tmux.py:1767-1780,1843-1853`);
- open dialogs (`dispatch_input` `:4603-4609`, issue #896);
- a pane whose foreground process has gone back to the shell (`:4622-4644`, issue #1018);
- non-TUI shells, which get no bracket markers (`tmux.py:1781-1799`).

### 3.4 Latency (VERIFIED components; totals INFERRED)

- **tmux, idle receiver:** POST, then status read, paste, 2.0 s, Enter, 0.5 s, Enter. That is about 2.5 s plus tmux round trips. If the immediate attempt sees a stale status and the pane produces no output, the 30 s sweep plus 30 s grace sets the worst case at about 60 s (issue #131, `docs/inbox-delivery.md:111-124`).
- **herdr:** delivery is event-driven on `idle`/`done`, then a 2.0 s sleep. Post-dispatch COMPLETED adds a 10 s flush wait, which affects handoff completion, not wake.
- **Busy receiver, default mode:** delivered after the turn ends and the regexes confirm it. The newest TUI can add up to the 15 s `TURN_END_GRACE_S`.

### 3.5 Failure modes

| Failure | Mechanism | Status |
|---|---|---|
| Enter swallowed (paste sits unsubmitted, status still IDLE) | 8 s confirmation, then bare Enter (composer readable) | VERIFIED, tmux only; herdr skips it (`inbox_service.py:301-317`) |
| Paste into an AskUserQuestion/plan dialog answers it | blocked by `blocks_orchestrated_input_while_waiting_user_answer` | VERIFIED (#896) |
| Regex drift after a Claude TUI change gives false IDLE/COMPLETED: premature paste, premature `/exit` on handoff | patched per issue | VERIFIED history (#459 comment, `claude_code.py:90-98`) |
| Prompt suggestion (dim text) in an empty box | not handled; `extract_current_composer` filters only the `Try "…"` placeholder | INFERRED risk to bare-Enter-vs-resend recovery |
| Second Enter (`enter_count=2`) | CAO's stated rationale: after a bracketed paste the first Enter adds a newline and the second submits (`providers/base.py:172-180`). If the first Enter already submits, the second lands in an empty composer; during eager mid-turn queueing its effect is unknown | rationale VERIFIED; effect INFERRED / unknown |
| Bracket-end injection | on herdr and tmux < 3.7 the body is wrapped by hand with no sanitisation, so a body containing `ESC[201~` ends the paste early and following newlines submit as separate prompts. CAO explicitly refuses `-S` on tmux ≥ 3.7 for this reason (`tmux.py:1741-1744`) but not on the other two paths | INFERRED from `herdr_backend.py:688`, `tmux.py:1810` |
| Eager message lost if the turn errors | accepted ("acceptable for v1") | VERIFIED `docs/inbox-delivery.md:105-107` |
| Receiver CLI died | ERROR status gives FAILED; an orphaned receiver row is failed by the sweep | VERIFIED `inbox_service.py:369-375,567-580` |

## 4. Setup and installation footprint on Claude Code (VERIFIED)

- **It writes the real user config:** every init reads `Path.home()/.claude/settings.json` (hard-coded; `CLAUDE_CONFIG_DIR` is ignored) and, if `skipDangerousModePermissionPrompt` is not already `true`, writes it (`claude_code.py:597-651`, early return at `:626-627`, called at `:897`). It is atomic via `os.replace` and keeps the file mode, but is last-writer-wins against Claude itself (`:607-612`). A settings file that fails to parse as JSON is replaced by `{"skipDangerousModePermissionPrompt": true}`, losing its contents (`:620-629`).
- **Per-terminal temp files** under `CAO_HOME_DIR/tmp/` (mode 0600, removed in `cleanup`, `:1635-1645`):
  - `<id>.prompt`, passed with `--append-system-prompt-file` (`:512-526`);
  - `<id>.mcp.json`, passed with `--mcp-config … --strict-mcp-config` (`:528-564`). `--strict-mcp-config` drops the user's own MCP servers. `CAO_TERMINAL_ID` is put into each server's `env` because "Claude Code does not automatically forward parent shell env vars to MCP subprocesses" (`:529-548`).
- **Launch flags:**
  - `--dangerously-skip-permissions`, or `--permission-mode <m>` from the profile, omitted under root (`:449-455`);
  - optional `--resume <sid>`, `--model`, `--effort`, `--fallback-model` (`:460-510`);
  - `--disallowedTools` for tool restrictions (`:571-576`);
  - `--agent <name>` for native-agent profiles (`:465-483`).
- **Environment:** every `CLAUDE*` variable is unset in the pane except the Bedrock/Vertex/Foundry and effort ones, to avoid "nested session" refusal (`:582-595`).
- **Project file:** the built-in memory plugin writes a `<!-- cao-memory -->` block into `<cwd>/.claude/CLAUDE.md` (`src/cli_agent_orchestrator/plugins/builtin/claude_code_memory.py:1-5,32-35`).
- **Startup dialogs:** CAO answers them by keystroke: bypass (Down + Enter), workspace trust (moves to "Yes" if needed), and the Bedrock model-upgrade nudge (types "2") (`claude_code.py:653-874`).
- **Not used:** hooks, plugins, mods and channel flags. Nothing is installed into Claude beyond the above.

## 5. Token cost

- **Per message (INFERRED from VERIFIED framing):**
  - the body, plus the suffix (about 90 chars, about 25 tokens), plus the `<cao-memory>` block on the first message only;
  - **plus a full model turn**: every delivery is a user prompt, so the receiver re-reads its context (mostly cached) and generates a reply.
  - In default mode each message to an idle agent costs one turn of its own, because one row is delivered per IDLE cycle (`num_messages=1`). N queued messages cost N turns.
- **Per quiet turn:** zero. No hooks run and nothing is injected (VERIFIED: no hooks).
- **Fixed per session (VERIFIED):**
  - MCP `instructions` of about 300 chars (`server.py:75-88`);
  - schemas for up to 37 `@mcp.tool` functions (`grep -c`: 37), narrowed by `--disallowedTools`.
  - Under Claude's tool search the MCP schemas are deferred (primer §5), so the upfront cost depends on that setting. The exact token count is unknown.

## 6. Limitations and bugs visible

1. **No draft guard** for humans sharing a Claude pane (§3.3).
2. **Every message is a turn.** There is no non-waking or low-priority delivery and no notion of "deliver on next natural turn". The only way to avoid a wake is to send while the receiver is busy and keep eager mode off.
3. **Paste semantics change model trust** (§2.2, INFERRED). The message arrives as a collapsed paste with no typed text, which current Claude Code may tell the model to treat as non-instructions.
4. **A screen-regex arms race.** The Claude status parser carries a long tail of per-redesign patches (spinner glyph sets, the `● high · /effort` footer, reduced motion, boxed prompt, background waits). A single Claude release can break wake or handoff. Our harness-versionless rule would treat this as runtime-detected breakage, but CAO has no degrade path: a wrong status means a wrong action.
5. **On herdr** the bracket wrap is unsanitised, submit confirmation is off, and 2 Enters go out with no gap (`herdr_backend.py:688-705`).
6. **Query-string transport** for message bodies: an implicit size cap, and bodies end up in access logs and URLs (INFERRED).
7. **Writes the global `~/.claude/settings.json`** whenever the key is absent (checked on every launch, §4), which affects the user's own non-CAO Claude sessions: it silences the bypass-permissions warning globally, and it clobbers a malformed settings file.
8. **The handoff result is screen-scraped** from the last `⏺`/`●` block (`claude_code.py:1572-1629`), not sent as a message, so it is lossy with tables, wrapped lines and long output.
9. **No ACK of model receipt.** DELIVERED means typed.

## 7. Clever ideas worth stealing

1. **Causal submit confirmation.** Sample an output-generation counter *before* typing (`terminal_service.py:4676-4690`) and accept only a busy status earned *after* that boundary (`inbox_service.py:386-408`). This is the right fix for "Enter swallowed but the screen still reads idle". It applies directly to our Native wake-marker path (Herdr typing `wake up`): confirm that `agent_status` went `working` after our send, not merely that it isn't idle.
2. **Recovery picked by reading the composer.** Bare Enter if the text is still in the composer, full re-send only if the composer is readable *and* empty, nothing if unknown (`terminal_service.py:3344-3360`). We already read Claude's composer from Herdr detection text in the hooks-only fallback. The same three-way answer would make a retry of our wake marker safe.
3. **A dialog guard as a provider capability** (`blocks_orchestrated_input_while_waiting_user_answer`). Machine input never goes into a picker, including the bare-Enter recovery path. Herdr's `blocked` state maps naturally onto this. Worth checking that our Native wake refuses on `blocked`.
4. **Re-checking the foreground process right before typing** (#1018). This refuses to type into a pane that fell back to a shell, where a pasted message would run as commands.
5. **Cancelling copy mode before the paste and before each Enter** (#654). The same class of hazard may exist for Herdr's own scroll or selection modes (unknown).
6. **The reconcile sweep with a grace window** (30 s / 30 s). It is a cheap safety net that adopts only messages the fast paths have demonstrably missed, so it never races them.
7. **Eager mid-turn delivery by typing while busy.** Claude queues typed input and delivers it after the current tool batch. This gives in-turn delivery without hooks. For us it is only a fallback idea for a hooks-only Claude without the mod, and draft and paste-trust concerns apply. Our mod's `tool.call` `context` is strictly better: invisible, bounded, ACKed and draft-safe.

## 8. Contrast with herdr-threads for Claude (INFERRED, brief)

| | CAO | herdr-threads (primer §9) |
|---|---|---|
| Inject (busy) | typed paste, queued by the TUI (eager mode only) | mod `tool.call` `context` beside the next tool result |
| Inject (idle) | typed paste + Enter = user turn | `$.session.append` hidden rows, then `$.prompt.submit('wake up')` |
| Wake policy | every message wakes | wake only if priority ≤ T; default send never wakes |
| Draft safety | none | 120 s draft hold via `$.prompt.read` |
| Idle oracle | screen regex or herdr `agent_status` | mod `turn.start`/`turn.complete` (fallback: Herdr state) |
| ACK | none (DELIVERED = typed) | `watch ack --via context/append/submit`, explicit ACK requests |
| Claude config written | `~/.claude/settings.json` key, `<cwd>/.claude/CLAUDE.md` block | user settings `env.CLAUDE_CODE_PLUGIN_DIRS` + hooks |

CAO's Claude path shows what we would be running if we had no mod or hooks. Its value to us lies in the hardening lessons on the keystroke fallback (ideas 1-5), not in its delivery model.

## Verification

Adversarial pass against the clone at `69909e98` (HEAD confirmed). Nothing executed.

Checked and held: line count and hook/channel grep (§1); `send_message` signature and topology check; suffix templates, `CAO_ENABLE_SENDER_ID_INJECTION` default and join; query-string POST (`orchestration.py:1424,1440-1445`); endpoint signature, 5 s wait and `max_length` elsewhere (`main.py`); status gate, eager/dialog logic, mark-before-send, same-sender join, lock, 8 s / 2-retry submit confirmation and the herdr skip (`inbox_service.py`); `dispatch_input` guards, boundary and `send_keys` arguments (`terminal_service.py`); base `paste_enter_count` = 2 and Claude's 2.0 s delay; tmux copy-mode cancels, paste-flag branches, 0.5 s Enter gap; herdr wrap and double Enter; herdr status mapping, 10 s flush and 300 s give-up (`base.py:541-647`); `IDLE_PROMPT_PATTERN`, staleness guard, background-wait pattern, composer reader; settings write, temp files, launch flags, env unset, memory plugin (`pyproject.toml:135` entry point); 37 `@mcp.tool`; doc citations for #131, #115, "acceptable for v1".

Changed:
- §1: Claude Code is not the only provider with `accepts_input_while_processing`; MiniMax Code also sets it.
- §2.1: tightened the `receiver_id` and topology citations (the cited `server.py` lines are docstring; added the implementing function); `<cao-memory>` is prepended only when the backend returns a block; herdr ignores `submit_delay`, skips the wrap for bare shells, and the line citations were added.
- §2.2: a one-line body plus the suffix is exactly 3 lines, which does not exceed the "over 3 lines" collapse threshold. The old "already makes it at least 3 lines" implied every delivery collapses.
- §2.3: removed the tmux `PollingObserver` 5 s watchdog as a trigger. It appears only in `docs/inbox-delivery.md`; the source's 5 s poll is OpenCode-only (`api/main.py:241-243`). Added the herdr event citations.
- §3.2: `_turn_still_in_flight` is at `:1311`, not inside `:225-250`.
- §3.5: the second-Enter row now gives CAO's documented rationale (first Enter adds a newline, second submits).
- §4 / §6.7: the settings write happens only when the key is absent, ignores `CLAUDE_CONFIG_DIR`, and replaces an unparsable settings file (newly found, VERIFIED `claude_code.py:614-629`).
