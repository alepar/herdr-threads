# CAO (awslabs) x OpenAI Codex CLI: message injection and wake

- Upstream: https://github.com/awslabs/cli-agent-orchestrator, depth-1 clone at `69909e98035905b612fd6b117abff8bf8876220f` (2026-10-23). Paths are relative to the clone root. Repo metadata (Apache-2.0, about 1.5k stars, very active, v2.5.3 latest release) is in `overview.md` (GitHub API, fetched 2026-10-23).
- Harness reference: `../../primers/codex.md` (Codex CLI 0.163.0). Codex source paths `codex-rs/...` refer to the primer's read-only clone of `openai/codex` at `rust-v0.163.0`.
- Tags: **VERIFIED** means read in source or docs at the cited line. **INFERRED** means my reasoning, not observed. **unknown** means not found.

## Support status

**Native, tmux-first, with an experimental herdr backend.** VERIFIED.

- Provider: `src/cli_agent_orchestrator/providers/codex.py` (2,985 lines, almost all TUI screen parsing), user doc `docs/codex-cli.md`.
- Launch line, built by `_build_codex_command` (`codex.py:1570-1845`):
  - `codex --yolo` by default (`--dangerously-bypass-approvals-and-sandbox`). With a profile `codexProfile` it is `codex --profile <name>` instead (`codex.py:1597-1632`).
  - Always `--no-alt-screen --disable shell_snapshot` (`codex.py:1633`). Inline mode keeps output in scrollback for `capture-pane`. The `shell_snapshot` flag avoids SIGTTIN under tmux (`docs/codex-cli.md:150-155`).
  - Optional `--model` (`codex.py:1638-1640`).
  - The system prompt goes in as `-c "developer_instructions=$(cat <file>)"` (`codex.py:1727-1738`).
  - Per-profile MCP servers go in as `-c mcp_servers.<name>.*` (`codex.py:1742-1825`), followed by `codexConfig` overrides and `-c check_for_update_on_startup=false` (`codex.py:1834-1840`).
- Integration surfaces CAO does **not** use with Codex: hooks, `notify`, the app-server, `codex queue`/`thread/queue/*`, `turn/steer`, `thread/inject_items`, `codex exec`/`resume`. The terms `app-server`, `codex exec`, `codex queue`, `hooks.json` and `turn/steer` do not occur in `src/` (VERIFIED, grep). Everything goes through keystrokes into the TUI and regex parsing of the screen.
- Per-provider capability flags as they apply to Codex:
  - `accepts_input_while_processing` keeps the base default `False` (`providers/base.py:377`; only `claude_code.py:1470` and `minimax_code.py:157` override it). Codex therefore never gets eager mid-turn delivery. `docs/inbox-delivery.md:88-89` lists Codex as "may buffer input, contributions welcome".
  - `blocks_orchestrated_input_while_waiting_user_answer = True` (`codex.py:1400-1412`).
  - `extract_current_composer` is implemented, so the composer is readable (`codex.py:2795-2830`).
  - `supports_direct_status_probe` keeps the base `False` (`base.py:258`; Codex does not override it).
  - `paste_enter_count` keeps the base 2 (`base.py:171-181`) and `paste_submit_delay` the base 0.3 s (`base.py:350-360`). Codex overrides neither (grep of `providers/`).

## Injection mechanism

### Exact path

The message goes in as **a user turn typed into the Codex composer**: a bracketed paste followed by Enter. There is no hook, no MCP resource or notification, and no developer-context injection. VERIFIED.

1. **The sender's model calls the MCP tool `send_message(message, receiver_id?)`** (`src/cli_agent_orchestrator/mcp_server/server.py:926-958`). The tool's model-visible description says: "The message will be delivered when the destination terminal is IDLE. Messages are delivered in order (oldest first)."
2. **Routing and framing happen in the sender's MCP subprocess** (`utils/orchestration.py:2442-2540`).
   - A missing receiver resolves to the recorded caller. An explicit one must be a parent, a child or a discovered sibling. Self-sends are refused (`orchestration.py:2500-2522`).
   - The sender suffix is then appended (`orchestration.py:2532-2538`, template at `:137-139`, composer at `:183-207`):
     ```python
     MESSAGE_SUFFIX_TEMPLATE = (
         "[Message from terminal {terminal_id}. Use send_message MCP tool for any follow-up work.]"
     )
     ...
     return message + "\n\n" + template.format(terminal_id=terminal_id)
     ```
     The suffix is on by default (`ENABLE_SENDER_ID_INJECTION`, env `CAO_ENABLE_SENDER_ID_INJECTION`, default `true`, `:128`; also skipped when the sender has no `CAO_TERMINAL_ID`, `:2532`). Otherwise it is suppressed only for kiro-cli `@prompt` invocations (`STRUCTURED_PROMPT_PROVIDERS`, `:148`), so a Codex receiver gets it unless a deployment turns injection off.
3. **HTTP to the server.** `_send_to_inbox` POSTs `{API_BASE_URL}/terminals/{receiver}/inbox/messages?sender_id=..&message=..`. The body travels as a **query parameter** (`orchestration.py:1391-1453`).
4. **The server persists and tries immediately.** `api/main.py:8373-8447` creates a PENDING row, then runs `inbox_service.deliver_pending` on a worker thread. It answers the POST once the paste has reached the pane, or after `INBOX_IMMEDIATE_DELIVERY_WAIT` = 5 s (`constants.py:393`), whichever comes first.
5. **The status gate** in `_deliver_pending_locked` (`services/inbox_service.py:206-248`): delivery proceeds only when status is IDLE or COMPLETED. For Codex the eager branch is never eligible, because `accepts_input_while_processing` is False, so any other status returns with the row still PENDING.
6. **Mark, then send.** The row is marked DELIVERED **before** typing (`inbox_service.py:250-257`). `terminal_service.dispatch_input` then (`services/terminal_service.py:4527-4760`):
   - refuses on ERROR or an open dialog (`:4589-4615`);
   - prepends `<cao-memory>` to the first message only (`:4624`, `inject_memory_context` at `:387`);
   - re-checks that the pane's foreground process is not the pre-launch shell (`_refuse_if_provider_gone`, `:4644`);
   - arms the status monitor, clears the rolling buffer and samples the dispatch boundary (`:4655-4696`);
   - calls the Codex `record_dispatched_message` (`:4706`), which captures a pre-dispatch screen baseline (`codex.py:1449-1475`);
   - finally calls `get_backend().send_keys(..., enter_count=2, force_bracketed_paste=True, submit_delay=0.3)` (`:4708-4715`).
7. **The keystrokes on tmux** (`clients/tmux.py:1686-1866`):
   - `send-keys -X cancel` first, to leave copy mode (`:1776-1780`).
   - `load-buffer`, then `paste-buffer`. On tmux < 3.7 the buffer is wrapped in hand-made `ESC[200~ … ESC[201~` markers and pasted with `-r` (`:1810-1811`). On tmux ≥ 3.7 it is pasted raw with `-p`, and tmux emits the markers because the pane has DECSET 2004 enabled (`:1813-1826`).
   - `sleep(0.3)`, then `Enter`, `sleep(0.5)`, `Enter`, re-cancelling copy mode before each Enter (`:1836-1858`).
   - On herdr the equivalent is `herdr pane send-text <pane> ESC[200~…ESC[201~`, a fixed `sleep(2.0)`, then two `pane send-keys Enter` with no gap between them (`backends/herdr_backend.py:645-703`).

**What the Codex model sees** (INFERRED from the above and primer §7): a normal user message whose text is `<body>\n\n[Message from terminal abcd1234. Use send_message MCP tool for any follow-up work.]`. The first message after launch also carries a leading `<cao-memory>` block when memory is enabled. The first Enter submits the turn, since Codex submits on Enter after a bracketed paste. The second Enter lands on an empty composer while the turn is starting. INFERRED: in Codex that is a no-op. If the first Enter was swallowed it becomes the submit, which is presumably why the base default of 2 is harmless here.

### Other Codex-specific framing

- **Handoff banner.** A blocking `handoff` to a Codex worker gets a 333-byte prose prefix (`orchestration.py:1183-1212`):
  ```python
  f"[CAO Handoff] Supervisor terminal ID: {supervisor_id}. "
  "This is a blocking handoff — the orchestrator will automatically "
  "capture your response when you finish. ... Do NOT use send_message ..."
  ```
  VERIFIED. It exists because Codex otherwise calls `send_message` back instead of answering inline (docstring, `:1184-1189`).
- **Assign suffix.** `[Assigned by terminal X. When done, send results back to terminal X using send_message]`, 103 bytes with its leading blank line (`orchestration.py:133-136,2182-2188`). VERIFIED.
- **Handoff result return is not a message.** The server waits for COMPLETED, scrapes the reply off the screen with `extract_last_message_from_script` (`codex.py:2832-2954`; the last `›` user echo up to the next empty `›` prompt or footer, with the completion footer stripped), then sends `/exit` (`codex.py:2967-2969`; `server.py:383-391`). VERIFIED.

### When it fires

- Only at an **idle boundary**, as a new turn. There is never mid-turn or tool-boundary delivery for Codex (VERIFIED, `inbox_service.py:218-248`). A message for a Codex agent that is working, for example a supervisor in a long turn, waits until the whole turn ends.
- **One message per idle cycle.** `deliver_pending` defaults to `num_messages=1` (`inbox_service.py:160-166,213`). The status-event consumer calls it with the default (`:132-134`), and no production caller passes `num_messages=0` (grep). N pending messages therefore cost N sequential Codex turns. VERIFIED.

### Size bounds

- **CAO:** no limit was found. The POST endpoint declares `message: str` with no `max_length` (`api/main.py:8373-8380`), and the inbox table stores it as is (`clients/database.py:276-286`, per overview). INFERRED: the practical ceiling is the HTTP request line, because the body is a URL query parameter. Uvicorn/h11's default incomplete-event limit is commonly 16 KiB (not verified here), so a large message may be rejected with an HTTP error rather than truncated. Unknown in practice.
- **Codex:** pastes over 1,000 chars appear in the composer as a `[Pasted Content N chars]` placeholder and are expanded on submit (`codex-rs/tui/src/bottom_pane/chat_composer.rs:450,1896,6245-6294`, VERIFIED). CAO has no handling for that placeholder: "Pasted Content" does not occur in `src/` (grep). See the limitations section.
- **No truncation or continuation scheme.** The whole body becomes one user turn.

### De-duplication and acknowledgement

- **Dedupe.** It comes from a per-terminal lock around read, mark and send (`inbox_service.py:68-88`) plus marking DELIVERED before typing, so a re-entrant status event cannot paste twice (`:250-257`). There is no message-id dedupe on the model side. VERIFIED.
- **Ack.** There is none from the model. DELIVERED means "typed into the pane". `after_dispatch` "is never an acknowledgement that the receiver accepted the message" (`inbox_service.py:180-186`). The states are only PENDING, DELIVERED and FAILED (overview). VERIFIED.
- **Submit confirmation (tmux only).** After the paste, `_confirm_submitted` waits up to 8 s (`INBOX_SUBMIT_CONFIRM_TIMEOUT`, `constants.py:378`) for a PROCESSING, COMPLETED or WAITING status whose output generation is newer than the dispatch boundary (`inbox_service.py:381-411`). Recovery then has three branches (`terminal_service.py:3416-3600`), with up to 2 recoveries (`constants.py:383`):
  - **Bare Enter**, when the last 64 normalized chars of the message (`_CURRENT_COMPOSER_PROBE_MAX_CHARS`, `terminal_service.py:279`) are found in the composer text that the Codex `extract_current_composer` returns (`terminal_service.py:3344-3375`).
  - **A full re-send** would need `supports_direct_status_probe`. Codex lacks it, so this branch is **never taken for Codex** (`terminal_service.py:3578-3589`).
  - **Otherwise nothing is sent**; the drop is only logged (`inbox_service.py:504-510`).

  On herdr the confirmation is skipped entirely (`inbox_service.py:300-317`). VERIFIED.

## Wake mechanism

### Exact mechanism

Wake and injection are the same act. The paste plus Enter into an idle composer starts a Codex turn whose user message *is* the delivery. VERIFIED (`docs/inbox-delivery.md:5-19`).

Delivery triggers for a Codex terminal:

| Trigger | Detail | Evidence |
|---|---|---|
| Immediate attempt | On POST, if the cached status is already IDLE/COMPLETED | `api/main.py:8396-8438` |
| Status event | `terminal.*.status` IDLE/COMPLETED spawns a per-terminal delivery thread; bursts are coalesced into one re-check | `inbox_service.py:90-158` |
| Watchdog | `PollingObserver` every 5 s on the tmux pipe-pane log, with an idle-regex precheck | `docs/inbox-delivery.md:13,77-79` |
| Reconcile sweep | Every 30 s, for rows PENDING > 30 s | `constants.py:345`, `inbox_service.py:553-588` |
| herdr backend | `HerdrInboxService` on `pane.agent_status_changed` → `idle`/`done`; also a sweep of panes already settled at subscribe time | `services/herdr_inbox_service.py:137-141,920-950,1039` |

### Idle and busy detection for Codex

- **tmux: screen regexes over a pyte-composited viewport.** `supports_screen_detection = True` (`codex.py:1335-1340`). `status_recheck_uses_screen` and `supports_midburst_processing_probe` are True (`:1342-1353`). The classifier is `_status_of_frame` (`codex.py:2527-2746`). Key signals:
  - Idle composer glyph `IDLE_PROMPT_PATTERN = r"(?:❯|›|»|codex>)"` in the last 5 rows (`:35,40,2621-2624`).
  - Spinner `TUI_PROGRESS_PATTERN` ("• Working (12s • esc to interrupt)", `:114`).
  - Status bar `TUI_FOOTER_PATTERN` (`:94`).
  - On Codex ≥ 0.155, the completion footer "Worked for 32s • 4:19 PM" (`:127-143`).
  - Approval menus and modals (`:303-384`, `:2595-2616`).
  - Trust, update and login startup dialogs (`:203-247`, `:2575-2593`).
  - The error cell `■` (`:150`).
- **Version-keyed rules.** `initialize()` reads the `OpenAI Codex (vX.Y.Z)` banner and enables the "footer ends a turn" rules only when the release is ≥ 0.155 **and** a known status bar is visible (`codex.py:2302-2322`). A provider rebuilt after a cao-server restart keeps the older rules (`docs/codex-cli.md:77`). VERIFIED.
- **Dispatch attribution.** Before each paste CAO stores the exact dispatched message and a pre-dispatch screen baseline (`codex.py:1449-1475`). It then classifies a turn as COMPLETED-empty only when a fresh, complete, soft-wrap-aware echo of *that* message is followed by a footer with nothing in between (`_dispatched_echoes` `:659-726`, `_empty_current_dispatch` `:1557-1568`). This keeps a stale previous-turn footer from reading as "done". VERIFIED.
- **Exit veto.** A ready verdict is turned into ERROR when the pane's foreground process is back to the pre-launch shell (`codex.py:2395-2445`; `terminal_service.py:4491-4524`). This exists because a crashed Codex leaves `› Error: …` on screen, the `›` reads as an idle composer, and a task was once typed into bash (#1018). VERIFIED.
- **herdr: Herdr's native `agent_status`.** The mapping is working → PROCESSING, blocked → WAITING_USER_ANSWER, done → COMPLETED, idle → IDLE, and unknown → fall back to screen parsing (`backends/herdr_backend.py:866-912`; `base.py:541-647`). Done, and idle after a dispatch, wait 10 s before COMPLETED, with a 300 s give-up. VERIFIED.

### Draft safety

**None before the paste.** VERIFIED:
- `extract_current_composer` is called only from the recovery path (`terminal_service.py:3319-3375`), never before a first dispatch (grep: there are no other call sites).
- The classifier treats the composer row, whatever text it holds, as footer chrome. `_compute_tui_footer_cutoff` folds the `›` line above the status bar into the footer (`codex.py:427-463`). A human draft therefore does not change IDLE/COMPLETED.

INFERRED consequence: when an operator is typing in an attached Codex pane, the inbox paste is appended to their draft and the Enter submits both as one turn. CAO assumes its panes are machine-owned.

### Busy-turn handling

INFERRED from Codex primer §7: Codex treats Enter during a running turn as a *steer*, and Tab as a local queue. CAO avoids that only by never pasting while PROCESSING. A misclassified frame (UNKNOWN keeps the last status; COMPLETED latched) could therefore steer an active turn. The sticky-latch and boundary machinery (`terminal_service.py:4648-4696`) exists to reduce exactly that.

### Latency

- If the receiver is idle when the message arrives: about 0.3 s paste-to-Enter on tmux, or 2 s on herdr, plus the HTTP and status hop. VERIFIED (constants above).
- If the receiver is busy: from the end of its turn to the next IDLE/COMPLETED event. That is near-immediate on the status event, ≤ 5 s through the watchdog and ≤ 60 s through the reconcile safety net (30 s grace + 30 s interval). INFERRED from the cited intervals.
- On Codex ≥ 0.155 a turn ends only once the completion footer is drawn, so detection is footer-bound. INFERRED.

### Failure modes (VERIFIED unless noted)

- **Swallowed Enter.** It is recovered by a bare Enter only when the composer shows the message tail. Otherwise it is logged and the message stays in the composer as DELIVERED (`inbox_service.py:454-510`). On herdr it goes undetected (`:300-317`).
- **Approval prompt open.** Under a `codexProfile` with a prompting approval policy the message stays PENDING, and the doc warns that sessions "will time out waiting for input that nothing can deliver" (`docs/codex-cli.md:191-197`).
- **Codex exited to the shell.** Delivery is refused (ERROR). Rows go FAILED, or PENDING when the pane is unresolvable (`inbox_service.py:338-379`).
- **Interrupted turn.** INFERRED: an error cell `■` "ends a turn" (`codex.py:150`, `docs/codex-cli.md:71`), so CAO reads COMPLETED and delivers the next message right after a human Ctrl-C. It does not pause the way `codex queue` does (primer §3).
- **Duplicate-wakeup race** across the immediate, watchdog, sweep and OpenCode paths is acknowledged and tracked in GH #115 (`docs/inbox-delivery.md:128`).

## Setup and installation footprint on the harness

CAO writes **no persistent Codex config**. Everything is per-launch `-c` overrides. VERIFIED (`docs/codex-cli.md:106-108,221-229`; `codex.py:1740-1840`).

| Item | Where | Evidence |
|---|---|---|
| `developer_instructions` | `-c` reading `$(cat ~/.aws/cli-agent-orchestrator/tmp/<tid>.codex_developer_instructions)`, a mode-0600 file removed on cleanup | `codex.py:1414-1422,1727-1738,2971-2984` |
| CAO MCP server | `-c mcp_servers.cao-mcp-server.{command,args,cwd,env,env_vars=["CAO_TERMINAL_ID",…],tool_timeout_sec=600.0,omit_tools_from=["deferred"]}` | `codex.py:1742-1825` |
| Update dialog | `-c check_for_update_on_startup=false` | `codex.py:1838-1840` |
| Trust, update and login dialogs | trust and update are auto-dismissed by keystrokes; login is left for a human | `codex.py:1847-2064`, `docs/codex-cli.md:25-30` |
| Memory | a delimited `<!-- cao-memory:begin -->…end -->` block written into the **user's `<cwd>/AGENTS.md`** on terminal creation | `plugins/builtin/codex_memory.py:1-41,105-190`, `docs/codex-cli.md:125-148` |
| Sandbox | `--yolo` by default, which makes `~/.codex` sandbox settings inert | `docs/codex-cli.md:157-175` |
| Hooks, `notify`, `hooks.json` trust | none | grep |

INFERRED: the AGENTS.md write runs on `post_create_terminal`. Whether Codex has already read AGENTS.md by then is a race. Unknown.

## Token cost

- **Per delivered message:** a full Codex user turn. The visible framing is the body plus 85 bytes (`\n\n[Message from terminal xxxxxxxx. Use send_message MCP tool for any follow-up work.]`, about 20 tokens). A Codex handoff adds a 333-byte banner (about 75 tokens) and an assign adds 103 bytes. Measured with Python on the templates. INFERRED: the dominant cost is not the framing but the turn itself. Each message triggers a sampling round over the whole context (prompt-cache dependent) plus the model's reply. With one message per idle cycle, N messages cost N turns.
- **Per quiet turn:** **0 bytes injected by CAO.** It has no hooks, so nothing is added on turns without mail (VERIFIED by the absence of hooks).
- **Standing per-request cost:**
  - `developer_instructions`: the security preamble (`constants.py:913-918`, about 250 bytes), the profile prompt and the skill-list prompt. The source notes that this "commonly produces several KB … observed live at 8+KB" (`codex.py:1673-1677`).
  - Every CAO MCP tool schema, forced into the initial tool list by `omit_tools_from=["deferred"]`, bypassing Codex's `tool_search` deferral (`codex.py:1815-1825`). `server.py` has 37 `@mcp.tool` sites, some conditional. Exact bytes unknown.

## Limitations and bugs I can see

1. **Draft clobbering** (VERIFIED, no pre-paste composer check; INFERRED effect). A human draft in an attached pane is merged with the inbox message and submitted.
2. **Every message is a wake and a full turn.** There are no priorities or thresholds and no silent "deliver on next turn" path. Codex cannot receive mid-turn delivery at all, so a busy Codex supervisor sees worker results only after its turn ends (VERIFIED, `accepts_input_while_processing` default).
3. **Large-paste placeholder.** INFERRED, from VERIFIED Codex behaviour plus VERIFIED CAO probe logic. A message over 1,000 chars sits in the composer as `[Pasted Content N chars]`, so if its Enter is swallowed `_message_in_composer` cannot find the 64-char tail. The bare-Enter recovery is then skipped (no full re-send either, since there is no probe), and the message stays unsubmitted while marked DELIVERED. The echo-attribution code (`_dispatched_echoes`) assumes the literal text is rendered, which holds for the transcript echo after submit (INFERRED) but not for the composer.
4. **Shared app-server daemon.** INFERRED risk, unverified. CAO launches Codex without `--no-daemon` and relies on `env_vars=["CAO_TERMINAL_ID"]` to pass identity to the MCP subprocess (`codex.py:1797-1805`). Since about 0.157 the TUI attaches to a machine-wide daemon by default (primer §2.1). If MCP servers are spawned in the daemon's process environment, as reported for hooks (primer §2.1, §9), the `CAO_TERMINAL_ID` the MCP server sees could be missing or another pane's, which would misattribute `send_message`. No mention of the daemon, `--no-daemon` or `CODEX_EXEC_SERVER_URL` appears in `src/`, `docs/` or `CHANGELOG.md` (grep). Worth an experiment before trusting either approach.
5. **Regex fragility with version-specific branches.** The rules cover footer formats per release (0.155, 0.156-0.158, 0.159+; `codex.py:127-143`), status-bar layouts per release (`:84-94`) and approval UI variants (`:303-384`). A provider rebuilt after a server restart silently falls back to the older rules (`docs/codex-cli.md:77`). VERIFIED.
6. **No confirmation on herdr**, so swallowed Enters go undetected there (VERIFIED, `inbox_service.py:300-317`).
7. **Enter after an interrupt.** INFERRED. A human Ctrl-C produces an error cell, which reads as turn end, so the next queued message is typed immediately and undoes the interrupt's intent.
8. **Result scraping for handoffs.** The reply is extracted off the screen and can be cut by scrollback or wrapping. It raises "No Codex response found" when no marker is visible (`codex.py:2936-2952`). VERIFIED.
9. **Soft-only tool restrictions.** Codex has no native allowlist in CAO's model, so restrictions are a prompt preamble only (`codex.py:1651-1661`; `skills/cao-provider/references/lessons-learnt.md:144`). VERIFIED.

## Clever ideas worth stealing

1. **Generation-stamped submit confirmation** (`inbox_service.py:381-411`; `terminal_service.py:4655-4696`). After typing, require a busy or done status *earned after* a sampled output-generation boundary, so a latched status from the previous turn never counts. This applies directly to our Native wake marker: confirm the wake took instead of assuming it did.
2. **Composer-evidence-gated recovery** (`terminal_service.py:3344-3375,3505-3589`). Re-press Enter only when the composer visibly still holds our text, and never re-paste without proof the composer is empty and the agent is idle. Our content-free `wake up` makes this cheap: a short fixed probe string.
3. **Foreground-process veto right before typing** (`terminal_service.py:4491-4524`, `codex.py:2395-2445`). Compare the pane's current command with the pre-launch shell baseline, so wake text is never typed into a shell when Codex has died. It is complementary to Herdr's `agent_status`.
4. **Cancel tmux copy mode before every Enter** (`clients/tmux.py:1776-1780,1849-1852`). Relevant if we ever drive tmux panes outside Herdr.
5. **Per-dispatch echo attribution** (`codex.py:1449-1568`). Tie "turn done" to the echo of the exact text we sent, which disambiguates a stale footer. Useful if we ever need turn-end evidence beyond `agent_status`.
6. **Codex launch-config lessons** (VERIFIED in source comments):
   - `tool_timeout_sec` must be a TOML float (`600.0`); an integer silently falls back to 60 s (`codex.py:1806-1814`).
   - `omit_tools_from=["deferred"]` keeps a server's tools out of `tool_search` deferral (`:1815-1825`).
   - A `$(cat file)` substitution avoids the 4,096-byte `MAX_CANON` tty limit on typed launch lines (`:1665-1708`).
   - `check_for_update_on_startup=false` suppresses the update dialog (`:1838-1840`).

   The first two apply to any herdr-threads MCP or `-c` integration with Codex.
7. **Keying parser rules on observed screen features** (banner version *and* visible status bar, `codex.py:2302-2322`). This is close to our own "degrade on what was observed" rule. Note, though, that CAO also uses version thresholds as gates, which our policy forbids.

## Relevance to herdr-threads (INFERRED, for the comparison phase)

- CAO's Codex path is the opposite of our delivery contract. In CAO every message is a user turn and a wake, delivered only between turns. In herdr-threads a default p1 never wakes anyone (rules 1-2), bodies arrive in-turn through hooks (rule 3), and the wake prompt carries no content (R18).
- CAO has no draft guard. We refuse a wake when the composer is not `Empty`, which is strictly safer.
- CAO has no wake confirmation on herdr; on tmux, ideas 1-3 above are its edge over us. Neither CAO nor we use Codex's `codex queue`/`thread/queue/add` (server-side idle dispatch, no typing, primer §3). That remains the strongest native alternative to keystroke wakes for both projects.

## Verification

Adversarial pass, 2026-10-23, against the clone at `69909e98` and the primer's `openai/codex` clone.

- Checked and holding: launch line (`--yolo`/`--profile`, `--no-alt-screen --disable shell_snapshot`, `developer_instructions=$(cat …)`, `mcp_servers.*` with `env_vars` + `CAO_TERMINAL_ID`, `tool_timeout_sec=600.0`, `omit_tools_from=["deferred"]`, `check_for_update_on_startup=false`); capability flags (`accepts_input_while_processing` overridden only in `claude_code.py`/`minimax_code.py`; Codex overrides no `paste_*` or probe flag); `send_message` description; suffix/assign/handoff templates and their byte counts (85/103/333, re-measured); query-parameter POST; 5 s immediate wait, 8 s confirm, 2 resubmits, 30 s reconcile; status gate, mark-before-send, per-terminal lock, `num_messages=1` with no production caller passing 0; recovery branches in `redeliver_dropped_message_with_boundary` (bare Enter on composer evidence, full re-send gated on probe); herdr confirmation skip; tmux copy-mode cancel, `-r`/`-p` split at 3.7, 0.3 s/0.5 s timing; herdr 2 s sleep and back-to-back Enters; herdr `agent_status` mapping and 10 s / 300 s waits; version gating in `initialize()`; footer cutoff folding the `›` line; `codex_memory` writing `<cwd>/AGENTS.md` on `post_create_terminal`; absence of hooks/app-server/queue/steer/`--no-daemon`/"Pasted Content" in `src/` (grep); Codex `LARGE_PASTE_CHAR_THRESHOLD = 1000` and placeholder (`chat_composer.rs:450,1896`); doc citations in `docs/inbox-delivery.md` and `docs/codex-cli.md`; 37 `@mcp.tool` sites; GitHub API (1465 stars, Apache-2.0, latest release v2.5.3).
- Changed: the sender suffix is default-on but switchable (`CAO_ENABLE_SENDER_ID_INJECTION`), not unconditional; corrected the `STRUCTURED_PROMPT_PROVIDERS` line (`:148`).
- Not re-verified: the uvicorn/h11 request-size limit (already marked unverified) and the shared-daemon MCP environment risk (already INFERRED).
