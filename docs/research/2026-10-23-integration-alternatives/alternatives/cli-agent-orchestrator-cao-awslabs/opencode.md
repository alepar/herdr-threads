# CAO (awslabs) x OpenCode: message injection and wake

- Upstream: https://github.com/awslabs/cli-agent-orchestrator. Clone read: depth-1 at `69909e98035905b612fd6b117abff8bf8876220f`. All paths are relative to that clone root.
- Harness: OpenCode (https://github.com/anomalyco/opencode, formerly sst/opencode). Surfaces are from `../../primers/opencode.md`.
- Tags: **VERIFIED** means I read it in source, docs or the GitHub API at the cited place. **INFERRED** means it is my reading and was not run. Nothing here was executed.

## TL;DR

CAO drives a stock interactive OpenCode TUI inside a tmux window (or a herdr tab). It touches none of OpenCode's integration surfaces: no plugin, no server API, no `prompt_async`, no `noReply`, no TUI plugin. Every inbox message is pasted into the composer as a bracketed paste. CAO then waits a fixed delay (1.0 s on tmux, 2.0 s on herdr) and presses Enter. So the paste is both the injection and the wake, and every message costs a full new user turn.

Idle and busy are read with screen regexes on OpenCode's footer:

- `ctrl+p commands` means idle.
- `esc interrupt` means working.
- `▣ agent · model · Ns` marks a completed turn.
- `△ Permission required` means a permission dialog is open.

OpenCode's alt-screen TUI stops emitting bytes once it settles. So the tmux-side wake trigger (in #203's time a log-mtime watchdog; today the FIFO/StatusMonitor status-event pipeline) never fires for an already-idle OpenCode, and messages stayed PENDING forever (#203). The fix is a dedicated 5 s poller scoped to OpenCode terminals.

OpenCode is in CAO's "no recovery" class:

- CAO cannot read OpenCode's composer.
- A swallowed Enter therefore leaves the message unsubmitted in the composer while the row already says DELIVERED.
- CAO logs this and does nothing else.

There is no draft protection, no priority and no receiver ACK.

## Support status

| Item | Finding | Evidence |
|---|---|---|
| Provider | `opencode_cli`, native class `OpenCodeCliProvider` | VERIFIED `src/cli_agent_orchestrator/providers/opencode_cli.py:67` |
| Maturity | "Experimental". Multi-agent `assign`/`send_message` back to a supervisor depends on "a temporary OpenCode-specific inbox polling fallback for #203" until #115 replaces the delivery paths with one coordinator | VERIFIED `docs/opencode-cli.md:3` |
| #203 | "post-settle inbox messages stuck in PENDING state": supervisor flows "silently deadlock". Opened 2026-04-24, closed 2026-05-01 | VERIFIED GitHub API `issues/203` |
| #115 | the "Event driven architecture" PR that the docs still point to as the eventual fix. Closed **unmerged** 2026-07-01 | VERIFIED GitHub API `pulls/115` (`merged: false`) |
| Consequence of #115 | the "temporary" poller is the permanent mechanism today; the docs are stale on this point. An event bus did land separately (`InboxService.run` subscribes to `terminal.*.status`, `inbox_service.py:90-158`; `docs/event-driven-architecture.md`), but not the single coordinator | INFERRED (bus: VERIFIED) |
| Version calibration | the regex fixtures were captured from `opencode 1.14.19` on 2026-04-20 | VERIFIED `test/providers/fixtures/OPENCODE_FIXTURES.md:5-6` |
| Open breakage | #294: "opencode_cli initialization times out with OpenCode 1.17.4: TUI starts but terminal status remains unknown". Still open | VERIFIED GitHub API `issues/294` |
| Gap to our primer | the primer is based on OpenCode 1.18.1 locally and 1.19.4 as the latest release; CAO's support on those versions is unknown | INFERRED |
| Backends | tmux (default) and herdr. No OpenCode-specific herdr code exists. On herdr, status comes from Herdr's `agent_status` via `_resolve_native_status`, which falls back to a live pane read when Herdr reports unknown. Herdr-side wake is `HerdrInboxService`: one broadcast `pane.updated` subscription, delivering via `inbox_service.deliver_pending` when a pane turns `idle`/`done` | VERIFIED `providers/opencode_cli.py:223-231`, `providers/base.py:541-647,649-680`, `services/herdr_inbox_service.py:1-15,1024-1039,1344`, `docs/herdr.md:97-99`; no `opencode` hits in `src/cli_agent_orchestrator/backends/` or `services/herdr_inbox_service.py` (grep) |
| Herdr recognising OpenCode | whether Herdr reports an agent kind for an OpenCode pane is unknown here; the primer infers that it does | primer section 8 |

## Injection mechanism

### Path, end to end

1. **The sender calls the MCP tool `send_message(message, receiver_id?)`.** This is the `cao-mcp-server` stdio subprocess. Its docstring says "delivered when the destination terminal is IDLE … in order (oldest first)". VERIFIED `src/cli_agent_orchestrator/mcp_server/server.py:926-958`.
2. **A plain-text suffix is appended as its own paragraph.** VERIFIED `src/cli_agent_orchestrator/utils/orchestration.py:2532-2538`, template at `:137-139`, `append_sender_suffix` at `:187-212`:
   ```python
   MESSAGE_SUFFIX_TEMPLATE = (
       "[Message from terminal {terminal_id}. Use send_message MCP tool for any follow-up work.]"
   )
   ...
   return message + "\n\n" + template.format(terminal_id=terminal_id)
   ```
   - The suffix is on by default (`CAO_ENABLE_SENDER_ID_INJECTION`, `:128`).
   - Only kiro-cli gets it suppressed for a leading `@`. OpenCode is named explicitly as reading a leading `@path` as a prose file mention, so it always gets the suffix. VERIFIED `orchestration.py:141-148`.
3. **The body is sent as a URL query parameter.** `requests.post(f"{base_url}/terminals/{receiver_id}/inbox/messages", params={"sender_id":..., "message":...})`. VERIFIED `orchestration.py:1424,1440-1445`.
4. **The server persists a PENDING row and attempts delivery at once on a worker thread.** The POST answers once the paste reached the pane, waiting at most `INBOX_IMMEDIATE_DELIVERY_WAIT` = 5 s. VERIFIED `src/cli_agent_orchestrator/api/main.py:8373-8447`, `constants.py:393`.
5. **`InboxService._deliver_pending_locked` runs under a per-terminal lock.** VERIFIED `src/cli_agent_orchestrator/services/inbox_service.py:206-299`. It:
   - reads up to `num_messages` (default 1) PENDING rows, oldest first;
   - gates on cached status IDLE or COMPLETED;
   - marks the rows DELIVERED **before** sending;
   - joins same-sender runs with `\n`;
   - calls `terminal_service.dispatch_input(..., orchestration_type=SEND_MESSAGE)`.
6. **`dispatch_input` prepares and sends the paste.** VERIFIED `src/cli_agent_orchestrator/services/terminal_service.py:4527-4715`. In order:
   - refuses if status is ERROR or the pane is back at its pre-launch shell (`:4589-4594`, `:4492-4524`, `:4644`);
   - on the first input after init only, prepends a `<cao-memory>` block (`:4620`, `inject_memory_context` at `:387-411`);
   - arms the status monitor, clears the rolling buffer and samples the dispatch boundary (`:4654-4690`);
   - calls `backend.send_keys(..., enter_count=1, force_bracketed_paste=True, submit_delay=1.0)`.
7. **tmux `send_keys` performs the keystrokes.** VERIFIED `src/cli_agent_orchestrator/clients/tmux.py:1686-1866`:
   - `send-keys -X cancel` leaves copy mode. This is a tmux copy-mode command, not an ESC keystroke to the app.
   - `load-buffer` loads the text. On tmux < 3.7 it is wrapped in hand-made `ESC[200~…ESC[201~` and pasted with `paste-buffer -r`. On tmux >= 3.7 it is pasted raw with `paste-buffer -p`, which brackets it only if the pane enabled DECSET 2004.
   - `sleep(submit_delay)`, then another copy-mode cancel, then `send-keys Enter`.
   - **On herdr** the backend differs: `herdr pane send-text` with a hand-made `ESC[200~…ESC[201~` wrap (skipped if the foreground is a shell), a fixed `sleep(2.0)`, then `pane send-keys Enter`. `submit_delay` is accepted but ignored. VERIFIED `backends/herdr_backend.py:645-705`.
8. **OpenCode's own delay and Enter count.** VERIFIED `providers/opencode_cli.py:114-124`:
   - `paste_enter_count = 1`: "OpenCode TUI submits on a single Enter after bracketed paste".
   - `paste_submit_delay = 1.0`: "OpenCode's TUI can swallow an Enter sent too soon after the bracketed-paste end marker … (see #479)". Honoured by the tmux backend only (step 7).

### What the model sees

The text arrives as an ordinary **user message** typed into the composer: the body, a blank line, then the bracketed suffix line. There is no header, sender kind, thread, message id or priority marker.

On OpenCode it becomes a normal user turn in session history. INFERRED from primer section 7: OpenCode's paste handler collapses a paste of 3 or more lines, or more than 150 characters, into a `[Pasted ~N lines]` extmark that expands on submit. So the composer shows a placeholder, while the model still gets the full text.

### When it fires

- **Only at an idle boundary.** It needs IDLE or COMPLETED. VERIFIED `inbox_service.py:218-248`.
- **No mid-turn delivery for OpenCode.** Eager mid-turn delivery requires `accepts_input_while_processing`. OpenCode keeps the base default `False` (VERIFIED `providers/base.py:376-387`; not overridden anywhere in `opencode_cli.py`, read in full). CAO's docs list OpenCode only as a candidate ("may buffer input"). VERIFIED `docs/inbox-delivery.md:85-90`.
- **Untapped mid-turn path.** OpenCode itself accepts a busy-time submit and steers it in at the next step boundary (primer section 3). CAO does not use that.

### Size bounds and truncation

- **No body bound** on the inbox route. `message: str` is a plain query parameter. VERIFIED `api/main.py:8374-8380`. The only `max_length` limits in `main.py` belong to other models (`:1260,1294,1334,1347`).
- **Real limit, INFERRED:** the HTTP request-line size of the uvicorn/h11 stack, because the body travels in the URL. Its exact value was not checked, so: unknown.
- **No truncation or continuation.** In drain mode the cap is 100 rows (`inbox_service.py:213`), and the inbox drain uses the default of 1.

### De-duplication

- **Inside CAO:**
  - The per-terminal lock serialises read, mark and send (`inbox_service.py:68-88`).
  - Rows are marked DELIVERED before the paste so a re-entrant status event cannot re-send them (`:250-257`).
  - A full re-send after a drop is gated on composer evidence, so it is never attempted for OpenCode (below).
- **Known race:** docs still cite the multi-path duplicate-wakeup race tracked as #115. VERIFIED `docs/inbox-delivery.md:128`. The function the docs name, `check_and_send_pending_messages`, no longer exists in `src/` (grep); the per-terminal lock in `InboxService.__init__` is the code's answer to that race (`inbox_service.py:68-80`). VERIFIED.
- **Inside the model's context:** none. Each delivery is a separate user turn.

### Acknowledgement

- **None from the receiver.** DELIVERED means "typed into the pane". `after_dispatch` "is never an acknowledgement that the receiver accepted the message". VERIFIED `inbox_service.py:180-186`, `models/inbox.py` states PENDING, DELIVERED and FAILED (overview).
- **Submit confirmation is a weaker form.** CAO waits up to 8 s for PROCESSING, COMPLETED or WAITING_USER_ANSWER with an output generation newer than the boundary. It proves only that the TUI started a turn. VERIFIED `inbox_service.py:381-411`, `constants.py:373-383`.

## Wake mechanism

**Mechanism.** The same paste plus Enter. On an idle composer, Enter submits a new user turn, and that is the wake. CAO does not use OpenCode's `session.promptAsync`, `/tui/submit-prompt` or a plugin `session.idle` handler (no CAO code references them; grep for `opencode` across `src/` shows only the provider, config, permissions and install code). VERIFIED.

### Triggers for an OpenCode receiver

All of them converge on `deliver_pending`:

| Trigger | Cadence | Works for OpenCode? | Evidence |
|---|---|---|---|
| Immediate, on POST | once | yes, if the cached status is already IDLE or COMPLETED | VERIFIED `api/main.py:8396-8432` |
| Status-event consumer (`terminal.*.status` = IDLE/COMPLETED) | on transition | yes, at the end of a turn: the turn-completion frame produces bytes | VERIFIED `inbox_service.py:90-158`; INFERRED that OpenCode emits the transition |
| Log-mtime watchdog (`PollingObserver`, 5 s) | historical | **gone from current code** (no `PollingObserver` in `src/`, grep); `docs/inbox-delivery.md:13` still describes it. At #203 time it did not fire once settled: the alt-screen TUI "emits no pty bytes once the turn is idle", so the log mtime froze and `ctrl+p commands` never reappeared in the incremental byte stream. The same silence starves today's FIFO/StatusMonitor events (`constants.py:324-331`) | VERIFIED GitHub `issues/203` RCA, grep |
| Herdr `pane.updated` (`HerdrInboxService`), herdr backend only | on Herdr agent-status transition to `idle`/`done` | depends on Herdr classifying the OpenCode pane: unknown | VERIFIED `services/herdr_inbox_service.py:1024-1039,1344`; OpenCode support INFERRED |
| **OpenCode poller** | every `INBOX_POLLING_INTERVAL` = 5 s | yes; it was added for this case | VERIFIED `api/main.py:237-245,1653-1655`, `inbox_service.py:540-551`, `constants.py:324-331` |
| Reconcile sweep | every 30 s, rows PENDING > 30 s | yes, as a backstop | VERIFIED `constants.py:345-351`, `inbox_service.py:553-588` |

The poller is a plain loop. `list_pending_receiver_ids_by_provider("opencode_cli")` is a DB join that returns receivers with PENDING rows (VERIFIED `clients/database.py:2872-2885`). Each is fanned out to `deliver_pending` over up to 8 threads (`constants.py:395-399`, `inbox_service.py:512-538`):

```python
async def opencode_inbox_delivery_daemon(registry):
    while True:
        await asyncio.sleep(INBOX_POLLING_INTERVAL)
        await asyncio.to_thread(inbox_service.poll_opencode_pending_messages, registry)
```

### Idle and busy detection

`get_status` is ordered (VERIFIED `providers/opencode_cli.py:205-288`, patterns at `:40-64`):

1. **Native status first.** On herdr, Herdr's agent state is used; on tmux this is always None.
2. **WAITING_USER_ANSWER** when `△ (Permission required|Always allow)` appears with no idle footer after it.
3. **PROCESSING** when a line holds `\besc interrupt\b`, unless a later line holds the idle footer or a completion marker. The later-line case is a stale alt-screen remnant guard.
4. **COMPLETED** when the last `▣ … · … · (Nm )Ns` marker is followed by `ctrl+p commands` and no later `▣`.
5. **IDLE** when `ctrl+p commands` is present and there is no live `esc interrupt`.
6. Otherwise **UNKNOWN**.

A pyte screen variant mirrors this order (`:290-352`). `supports_direct_status_probe = True` allows a live capture-pane probe (`:126-131`).

**Weakness:** the idle footer is identical on the splash screen and a conversation-ready frame. "Readiness cannot prove the TUI will accept input — an OpenCode splash frame carries the same idle footer". This caused #562, where a handoff worker "remains on the OpenCode splash screen" and never gets its prompt. VERIFIED `services/agent_step.py:66-70,193-201`, GitHub `issues/562`.

**Draft safety: none.**

- There is no composer read: `extract_current_composer` is not overridden, so it returns `None` (base `providers/base.py:340-347`).
- The idle gate looks only at footer text, so a human's typed draft does not change the status.
- The paste lands at the cursor and the trailing Enter submits draft plus message as one prompt. INFERRED from the primer: OpenCode's composer is TUI-local and its paste inserts at the cursor.

CAO treats panes as machine-owned. CAO launches OpenCode with `OPENCODE_DISABLE_MOUSE=1`, so mouse scrolling enters tmux copy mode instead of scrolling the TUI, which keeps the completion marker in the captured frame (`docs/opencode-cli.md:268-276`).

**Dialog safety: partial.**

- The status gate skips WAITING_USER_ANSWER, so nothing is pasted while the permission dialog shows.
- OpenCode does not set `blocks_orchestrated_input_while_waiting_user_answer` (base default `False`, `providers/base.py:389-405`). So `dispatch_input`'s last-moment dialog guard (`terminal_service.py:4603-4609`) does not protect OpenCode in the window between the gate and the paste. INFERRED.
- What OpenCode's permission widget does with a paste plus Enter is unknown.
- CAO writes only `allow`/`deny` permissions so the dialog should not appear (`docs/opencode-cli.md:98,311-315`).

**Latency** (INFERRED from the constants):

- **Already idle with a fresh cached status:** about 1 s on tmux, 2 s on herdr (the submit delay), plus the time OpenCode takes to start.
- **Missed by the immediate path:** at most 5 s more (the poller).
- **Missed by the poller as well:** up to 30 + 30 s (the sweep).
- **Several messages for one terminal:** one per idle cycle (`num_messages=1`), so each later message waits for the next turn to end.

### Failure modes

1. **Swallowed Enter, with no recovery for OpenCode.**
   - After 8 s without pickup evidence, `_confirm_submitted` calls `redeliver_dropped_message_with_boundary`. VERIFIED `inbox_service.py:413-510`, `terminal_service.py:3416-3556`.
   - For OpenCode the direct probe may still find the turn started. Otherwise `_message_in_composer` is `None` (no reader), so nothing is sent and the drop is only logged ("the message may be sitting unsubmitted in the agent's prompt").
   - CAO's own test lists `opencode_cli` in `NO_RECOVERY` ("Probe but no composer reader"). VERIFIED `test/services/test_inbox_service.py:1978,1987-1990`; docs table `docs/inbox-delivery.md:27-31`.
   - The row stays DELIVERED, so the next delivery is pasted on top of the stuck one. This is the `{MSG1}{MSG2}` pile-up from #78 (`constants.py:353-361`). INFERRED that it applies to OpenCode, given NO_RECOVERY.
2. **Post-settle silence.** This was #203, now handled by the poller.
3. **Version drift breaks status.**
   - #294 is open: OpenCode 1.17.4 initialisation stays UNKNOWN. While a terminal reads UNKNOWN, delivery never fires because the gate needs IDLE or COMPLETED. VERIFIED `inbox_service.py:220`.
   - Footer strings are calibrated on 1.14.19.
4. **Completion detection needs the marker and the footer in one frame.** This is why mouse scrolling is disabled. VERIFIED `docs/opencode-cli.md:268-276`.
5. **Herdr backend skips submit confirmation entirely.** Drops there go undetected for every provider. VERIFIED `inbox_service.py:300-317`.
6. **A CLI that exits between the check and the paste.** A one-round-trip window remains in which the pasted bytes could reach a shell. CAO calls this unclosable for a typing design. VERIFIED `terminal_service.py:4633-4639`.

## Setup and installation footprint on the harness

**Config isolation.** CAO runs OpenCode with `OPENCODE_CONFIG` and `OPENCODE_CONFIG_DIR` pointing at `~/.aws/opencode/` and does not touch `~/.config/opencode/`. VERIFIED `docs/opencode-cli.md:72-92`, `constants.py:464-466`.

**Files written:**

- `~/.aws/opencode/opencode.json`, from `cao install`:
  - an `mcp` entry per profile server, e.g. `cao-mcp-server` as `{"type":"local","command":[...],"enabled":true}` (translation at `utils/opencode_config.py:151-200`);
  - a global deny `"<server>*": false` under `tools`;
  - a per-agent re-enable under `agent.<id>.tools`.
- `cao-grants.json`: a sidecar recording the grants CAO wrote.
- `agents/<id>.md`: an OpenCode agent file with frontmatter `description`, `mode` (default `all`) and a full `permission:` allow/deny map, with the profile prompt as body (`models/opencode_agent.py:8-23`, `utils/opencode_permissions.py`).
- A symlink `skills -> ~/.aws/cli-agent-orchestrator/skills/`, so CAO skills load through OpenCode's native `skill` tool.
- On first launch, OpenCode itself installs `@opencode-ai/plugin` (about 57 MB of `node_modules`). This is OpenCode's own plugin bootstrap for the config dir; **CAO installs no OpenCode plugin.** VERIFIED `docs/opencode-cli.md:21-25,80-92,127-141,155-213`.

**Launch command.** An inline-env command typed into the tmux shell. VERIFIED `providers/opencode_cli.py:186-203`:

```text
OPENCODE_CONFIG=… OPENCODE_CONFIG_DIR=… OPENCODE_DISABLE_AUTOUPDATE=1 OPENCODE_DISABLE_MOUSE=1
OPENCODE_DISABLE_TERMINAL_TITLE=1 OPENCODE_CLIENT=cao TERM=xterm-256color opencode --agent <id> [--model <m>]
```

- There is no `--port`, so the TUI exposes no HTTP API (primer section 1). CAO could not call `prompt_async` even if it wanted to.
- What `OPENCODE_CLIENT=cao` changes inside OpenCode is unknown.

**Identity.** `CAO_TERMINAL_ID` is not written to config. It reaches the MCP subprocess through the inherited tmux window environment. VERIFIED `docs/opencode-cli.md:213`. CAO does not track the OpenCode session id; the pane is the identity (INFERRED: no session-id handling in `opencode_cli.py`).

**Known footprint caveats.** VERIFIED `docs/opencode-cli.md:241-284`:

- A project-local `opencode.json` overrides CAO's MCP wiring.
- Parallel `cao install` runs race on the shared `opencode.json`.
- `--yolo` cannot widen permissions at runtime.

## Token cost

**Per message.**

- **A full new user turn.** The body plus the suffix (about 90 bytes, roughly 20-25 tokens) becomes a user message. Each message triggers its own model request, re-sending the whole conversation (prompt-cache dependent).
- **Batching:** none by default, one message per idle cycle. N queued messages cost N turns.
- **Memory block:** the first input after init can carry a `<cao-memory>` block (`terminal_service.py:387-411,4620`).
- **Replies:** the receiver usually answers with another `send_message` call: tool schema plus the call plus the tool result. INFERRED.

**Per quiet turn: zero.** There are no hooks, no per-turn injection and no polling inside the model. CAO's 5 s poller and the screen parsing cost only server CPU and tmux round trips.

**Static per session:**

- the `cao-mcp-server` tool schemas (`handoff`, `assign`, `send_message` and others), plus any MCP `instructions` OpenCode puts in `<mcp_instructions>` (primer section 5);
- skill metadata, with bodies loaded on demand;
- the agent prompt.

## Limitations and bugs visible

1. **Every delivery is a wake.** There is no silent or staged delivery, even though OpenCode offers `noReply: true` and a 2.x `synthetic`/`resume:false` (primer sections 3 and 9).
2. **No draft protection, and none is possible with this design,** because OpenCode's composer cannot be read from the screen in CAO's code.
3. **No recovery from a swallowed Enter** (NO_RECOVERY). The pile-up of later messages into one composer is possible.
4. **The splash screen and a ready frame look the same** (#562). The fixed 1.0 s submit delay is a guess.
5. **Footer-string coupling.** It is calibrated on 1.14.19; #294 is open against 1.17.4.
6. **The "temporary" 5 s poller is the real mechanism.** Its replacement (#115) was closed unmerged, and the docs still describe it as pending.
7. **No priority, threading, deadline or ACK in the model.** The sender gets "pasted" as success.
8. **The message body travels in a URL query string,** with no explicit size bound.
9. **Dialog guard gap.** OpenCode does not declare `blocks_orchestrated_input_while_waiting_user_answer`. INFERRED risk if a permission prompt opens in the race window.
10. **Result return for `handoff` scrapes the TUI** (`extract_last_message_from_script`, `opencode_cli.py:354-458`). It depends on `┃` user bars, a 5-space agent indent and the `▣` marker, within a roughly 41-line alt-screen viewport (`:133-151`). It is fragile by construction.
11. **No mid-turn delivery,** although OpenCode natively queues and steers busy-time prompts (`QUEUED` label, primer section 3).

## Clever ideas worth stealing

1. **A provider-scoped slow poll for a harness whose idle state is silent.** When the idle signal is edge-triggered on output and the TUI goes byte-silent at idle, add a level-triggered sweep. Scope it to that provider, and keep a slower global sweep with a grace window so it never races the fast paths (`docs/inbox-delivery.md:111-128`). For us: if Herdr's OpenCode status is edge-only, our wake frontier needs a level re-check. INFERRED.
2. **A causal pickup check.** Count a status as evidence only if its output generation is newer than a boundary sampled after the buffer was cleared and before the first key (`terminal_service.py:4676-4690`, `inbox_service.py:381-411`). This beats "status changed" checks that a stale COMPLETED can satisfy. It applies directly to our Native wake marker confirmation.
3. **A recovery decision table keyed on composer readability.** Visible text means a bare Enter. A readable empty composer means a full re-send. An unreadable one means do nothing and log. Never re-paste blind (`docs/inbox-delivery.md:27-35`). For OpenCode, where the composer cannot be read through Herdr, this argues for using the server API, not keystrokes. A server-side prompt never touches the composer, so the whole table becomes moot.
4. **A last-moment "pane back at shell" check** before typing: compare the foreground command with the recorded pre-launch shell (`terminal_service.py:4459-4524`). This is cheap insurance for any keystroke wake.
5. **Skipping bracketed-paste escapes when the pane's foreground is a shell,** and respecting tmux >= 3.7 paste sanitisation instead of bypassing it with `-S` (`clients/tmux.py:1710-1746`).
6. **Config isolation through `OPENCODE_CONFIG`/`OPENCODE_CONFIG_DIR` on a CAO-owned tree,** and recording its own grants in a sidecar so uninstall removes only what CAO provably wrote (`docs/opencode-cli.md:72-92,186-211`). This is a good pattern for a future herdr-threads OpenCode adapter's managed launch and tests. It fits the AGENTS.md rule against writing real user config.
7. **What not to copy.** CAO's OpenCode path is a worked example of why a keystroke wake is the weakest fit for OpenCode (primer section 12, shape 4):
   - four tracked items (issues #203, #294, #562 and PR #479) are about readiness, idle detection or dropped submits;
   - none of them could occur with a server plugin using `event` on `session.status` idle plus `client.session.promptAsync({parts:[{type:"text", text:"wake up", synthetic:true}]})` and `noReply` staging. INFERRED.

## Verification

Adversarial pass against the clone at `69909e98` and the GitHub API (issues #203, #294, #562, PRs #115, #479). Nothing executed.

Checked and held: provider class and regexes (`opencode_cli.py:40-64,67,114-131,186-288,290-352,354-458`), `get_status` order, `extract_current_composer` default `None`, `accepts_input_while_processing` and `blocks_orchestrated_input_while_waiting_user_answer` defaults (`base.py:340-405`) and the `_dialog_blocks_orchestrated_input` predicate (`terminal_service.py:181-206`), sender suffix and kiro-only `@` exemption (`orchestration.py:125-212,2525-2540`), query-param POST (`:1424-1445`), the inbox endpoint and 5 s wait (`api/main.py:8373-8447`), lock, mark-before-send, same-sender join, herdr skip of confirmation, pickup check, recovery gates and the fan-out sweeps (`inbox_service.py:64-588`), constants (`constants.py:324-399,464-466`), the OpenCode poller (`api/main.py:237-245,1653-1655`), the DB join (`database.py:2870-2883`), `NO_RECOVERY` (`test_inbox_service.py`), #562 wording (`agent_step.py:66-70,193-201`), fixture version 1.14.19, config isolation, skills symlink, plugin bootstrap, `CAO_TERMINAL_ID` inheritance and known limitations in `docs/opencode-cli.md`, issue/PR states and dates, absence of `promptAsync`/`prompt_async`/`noReply`/`session.idle` in `src/`.

Changed:
- The log-mtime `PollingObserver` watchdog no longer exists in `src/`; marked it historical and named today's FIFO/StatusMonitor trigger (TL;DR, trigger table).
- Added the herdr-backend wake path (`HerdrInboxService`, `pane.updated` idle/done) and the herdr `send_keys` variant (own ESC wrap, fixed 2.0 s, `submit_delay` ignored), and the 1.0 s vs 2.0 s split in TL;DR and latency.
- Fixed citations: `tmux.py` `send_keys` is `:1686-1866`; docs recovery table is `inbox-delivery.md:27-31`.
- #479 is a PR, not an issue; reworded "more than 3 issues".
- Mouse-disable is CAO's launch env, not OpenCode's TUI behaviour.
- Noted the event bus that landed outside #115, and that `check_and_send_pending_messages` (named in the docs) is gone from code.
