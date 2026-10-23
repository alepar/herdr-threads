# CAO (awslabs) x Pi coding agent (pi-mono, badlogic)

- Alternative: https://github.com/awslabs/cli-agent-orchestrator, depth-1 clone at `69909e98035905b612fd6b117abff8bf8876220f`. Paths without a prefix are relative to that clone root.
- Harness: https://github.com/badlogic/pi-mono (now presented as earendil-works/pi), clone at `a7b94a03f09f20f4937fa1a76339014c60d261ad`, `packages/coding-agent` 1.4.0. Paths prefixed `pi:` are relative to that clone root.
- Basis: `overview.md` (this directory) and `../../primers/pi.md`.
- Tags: VERIFIED = read in source or docs at the cited line. INFERRED = my reading, not stated.

## Support status

**Plain Pi is not supported.** CAO supports Oh My Pi (`omp`, https://github.com/can1357/oh-my-pi), which is a different program.

| Fact | Evidence |
|---|---|
| The provider enum has no `pi` entry and no generic or custom-command provider. The values are `kiro_cli, claude_code, codex, kimi_cli, copilot_cli, opencode_cli, hermes, cursor_cli, antigravity_cli, omp, grok_cli, mcode, mock_cli`. | VERIFIED `src/cli_agent_orchestrator/models/provider.py:7-20` |
| A grep of the whole tree for `pi-mono`, `badlogic`, `earendil`, `pi-coding` and a bare `pi` word finds only OMP references and an unrelated `plan_identifier as pi` import alias in a test. | VERIFIED, grep; `test/services/test_plan_identifier.py:16` |
| The OMP provider hard-codes the `omp` executable. It fails when `shutil.which("omp")` finds nothing, and the command is literally `["omp", ...]`. | VERIFIED `src/cli_agent_orchestrator/providers/omp.py:163-169` |
| The OMP provider is pinned to OMP 17.2.10 TUI fixtures. | VERIFIED `providers/omp.py:34`, `docs/omp-cli.md:9,141-143` |
| OMP is not a GitHub fork of pi-mono: the API reports `fork: false`. Upstream Pi never mentions OMP, and the two have diverged on config paths (`.omp/` and `~/.omp/agent/` versus `.pi/` and `~/.pi/agent/`). | VERIFIED in the sibling report `../mcp-agent-mail/pi.md:19-31` (GitHub API, greps); `docs/omp-cli.md:42-43`; `pi:packages/coding-agent/docs/mcp.md:32` |

The rest of this report does two things. It describes what CAO does for OMP, its nearest provider, because that is the only Pi-family code path. It then works out what would happen to plain Pi under each part of that mechanism, using the primer.

## Injection mechanism

### What CAO does (OMP path, shared inbox code)

1. **Sender side.** The sending agent calls the MCP tool `send_message(message, receiver_id?)` on `cao-mcp-server` (VERIFIED `src/cli_agent_orchestrator/mcp_server/server.py:926-960`). The tool sends an HTTP `POST {base}/terminals/{receiver}/inbox/messages`, with **the body as a query-string parameter** (`params = {"sender_id": ..., "message": ...}`; VERIFIED `src/cli_agent_orchestrator/utils/orchestration.py:1424,1440-1443`). The server persists a PENDING row (VERIFIED `src/cli_agent_orchestrator/api/main.py:8373`, overview "Injection mechanism").
2. **Receiver side.** No hook, extension event, MCP notification or system-prompt edit is involved. The message is **typed into the receiver's TUI composer as user input**:
   - `InboxService._deliver_pending_locked` checks status. It proceeds only when the receiver is IDLE or COMPLETED (eager mid-turn delivery needs both the global `CAO_EAGER_INBOX_DELIVERY` flag, default false, and `accepts_input_while_processing`, which is False by default and not overridden by OMP). It marks the rows DELIVERED *before* sending, joins same-sender runs with `\n`, and calls `terminal_service.dispatch_input(...)` (VERIFIED `src/cli_agent_orchestrator/services/inbox_service.py:206-296`; flags `providers/base.py:377-388`, `constants.py:322`).
   - On tmux, the client runs `send-keys -X cancel` to leave copy mode, then `load-buffer` and `paste-buffer -p` (or a manual `ESC[200~..ESC[201~` wrap on tmux < 3.7). It sleeps `paste_submit_delay` (0.3 s default) and sends `Enter` `paste_enter_count` times, which is 1 for OMP (VERIFIED `src/cli_agent_orchestrator/clients/tmux.py:1769-1856`, `providers/base.py:171-181,348-357`, `providers/omp.py:109-111`).
   - The core of the send is short:
     ```python
     subprocess.run(["tmux", "paste-buffer", *paste_args, "-b", buf_name, "-t", target], check=True)
     time.sleep(submit_delay)
     for i in range(enter_count):
         ...
         subprocess.run(["tmux", "send-keys", "-t", target, "Enter"], check=True)
     ```
     (`clients/tmux.py:1827-1856`)
3. **Payload and framing.** The body is plain text plus a trailing paragraph, `[Message from terminal {id}. Use send_message MCP tool for any follow-up work.]`. For `assign` the trailer is `[Assigned by terminal {id}. When done, send results back to terminal {id} using send_message]`. It is on by default through `CAO_ENABLE_SENDER_ID_INJECTION` (VERIFIED `utils/orchestration.py:125-138`). There is no header, id, priority or thread marker.
4. **When it fires.** Only between turns, as a new user turn. It is never mid-turn for OMP, because eager delivery is not eligible (above).
5. **Size bounds.** No limit was found on the inbox POST (`api/main.py:8373-8450` takes `message: str`, and there is no max constant in `constants.py`). VERIFIED by absence. Because the body travels in the URL, the practical ceiling is the HTTP server's request-line limit (INFERRED; the exact value is unknown).
6. **Batching.** By default one message per idle cycle (`num_messages=1`). A drain mode takes up to 100 (VERIFIED `inbox_service.py:213`).
7. **Dedupe and ack.** Rows are marked DELIVERED before the paste, so re-entrant status events do not paste twice (VERIFIED `inbox_service.py:250-257`). DELIVERED means "typed into the pane", not "read by the model". There is no receiver ACK (VERIFIED, overview "Delivery guarantees"). A known cross-path duplicate-wakeup race is tracked in GH #115 (VERIFIED `docs/inbox-delivery.md:128`).
8. **OMP-specific context.** The role/profile context is a system-prompt append: CAO writes `$CAO_HOME_DIR/tmp/omp/<tid>/context.md` (mode 0600) and passes `--append-system-prompt <file>`. It never uses `--system-prompt` (VERIFIED `providers/omp.py:174-191`, `docs/omp-cli.md:43-46`). This is static launch context, not message delivery.

### What it would mean for plain Pi (INFERRED unless cited)

- **Keystroke injection works on Pi's TUI.** Pi enables bracketed paste (VERIFIED `pi:packages/tui/src/terminal.ts:211-212`). Enter submits (VERIFIED `pi:packages/coding-agent/docs/usage.md:11-40`). Pastes over 10 lines or 1000 chars collapse to a `[paste #N ...]` marker and are expanded on submit (VERIFIED `pi:packages/tui/src/components/editor.ts:1298-1316,1092-1093`). So `paste_enter_count = 1`, as for OMP, should submit the whole message as one user turn. This was not tested.
- **The model sees a plain `user` message.** That is the same role a Pi extension's `sendMessage` would produce on the wire (`pi:packages/coding-agent/src/core/messages.ts:162-169`), so the keystroke path loses nothing in role terms compared to the native API. It does lose `display`/`customType` rendering and leaves the human's composer exposed.
- **Mis-timed pastes are not lost.** If a paste lands while Pi is actually busy (a stale IDLE verdict), Pi turns typed text plus Enter into a **steer** rather than rejecting it (VERIFIED `pi:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3333-3338`, `docs/usage.md:35,40`). The message reaches the model after the current tool batch. That is more forgiving than TUIs that drop or merge input. With the default `one-at-a-time` steering mode, several such steers would spread over turns (VERIFIED `pi:packages/coding-agent/docs/settings.md:31-32`).

## Wake mechanism

### What CAO does (OMP path)

Wake and injection are the same act: the paste plus Enter into an idle composer starts a turn. VERIFIED, overview "Wake mechanism".

| Aspect | OMP behaviour | Evidence |
|---|---|---|
| Triggers | Immediate attempt after POST; status-event bus subscription `terminal.*.status` on IDLE/COMPLETED, fed by the pipe-pane StatusMonitor; 30 s reconcile sweep for rows PENDING > 30 s; herdr `pane.agent_status_changed` events on the herdr backend. The docs also describe a 5 s `PollingObserver` log watchdog, but no `PollingObserver` exists in `src/` at this commit (the only "watchdog" code is the unrelated pipe-pane liveness watchdog), so that doc section looks stale | VERIFIED `inbox_service.py:90-160` (subscribe at :109), `constants.py:345,351`, `api/main.py:261`, `docs/herdr.md:99`; docs claim `docs/inbox-delivery.md:12-13`; absence by grep of `src/` |
| Busy/idle detection | Regexes over the raw pipe-pane stream or a pyte screen. PROCESSING = `Working… ⟨esc⟩` or `Running … ⟨esc⟩`; WAITING_USER_ANSWER = `Allow tool: <name>`; ERROR = `Error: No model selected.`; ready = `╰─…─╯` frame or `in: N out: N … tok/s:` footer. A later ready frame makes older markers stale. IDLE versus COMPLETED is decided by a dispatch counter `_turns` | VERIFIED `providers/omp.py:36-55,224-299,352-354` |
| Idle-log pattern | `get_idle_pattern_for_log()` returns only the status-line footer pattern, which the code itself says "OMP's rendered terminal can omit". It has no caller in `src/` (only tests), so it does not affect delivery | VERIFIED `providers/omp.py:38-44,301-302`; grep of `src/` |
| Dialog safety | `blocks_orchestrated_input_while_waiting_user_answer = True`: no paste while an `Allow tool:` dialog is up; refused deliveries go back to PENDING | VERIFIED `providers/omp.py:113-115`, `providers/base.py:390-405`, `inbox_service.py:349-371` |
| Dead CLI | If the pane's foreground command is back to the pre-launch shell, status is ERROR and the paste is refused | VERIFIED `providers/omp.py:123-125,257-262` |
| Draft safety | None. No composer read: OMP does not override `extract_current_composer` (the base returns `None`), so a human's half-typed text gets the paste inserted into it and submitted | VERIFIED `providers/base.py:340-347`; absence in `providers/omp.py` |
| Submit confirmation | Waits up to 8 s for a status earned after the dispatch boundary. Recovery (bare Enter or full re-send) needs a composer reader or a direct probe, and OMP has neither (`supports_direct_status_probe = False`), so a swallowed Enter is only logged. The check is skipped entirely on the herdr backend | VERIFIED `providers/omp.py:89-90`, `inbox_service.py:300-347,413-511`, `constants.py:373-383` |
| Latency | tmux: immediate when the receiver is already idle; otherwise the next IDLE/COMPLETED status event or the 30 s sweep (up to about 60 s for a message the fast paths missed: 30 s grace plus 30 s interval, INFERRED). Each paste adds a fixed 0.3 s `submit_delay` | VERIFIED `docs/inbox-delivery.md:111-124`, `constants.py:345,351`, `providers/base.py:349-358` |

Failure modes seen in the code:
- If the regexes do not match a newer OMP release, the status is UNKNOWN. That is neither IDLE nor COMPLETED, so delivery stalls (INFERRED from `omp.py:264-286` and `inbox_service.py:219-245`). The docs' answer is to "recapture terminal fixtures" for each release (VERIFIED `docs/omp-cli.md:141-143`).
- A delivery marked DELIVERED but never submitted is never retried (VERIFIED `inbox_service.py:450-452` docstring: "left DELIVERED either way").

### What it would mean for plain Pi

- **Screen detection would not work on plain Pi.** The OMP chrome strings (`OMP session`, `⟨esc⟩`, `Allow tool:`, `tok/s`) do not occur in pi-mono's TypeScript sources (VERIFIED, grep over `pi:packages/**/*.ts`). On tmux, plain Pi would always read UNKNOWN and never get a delivery (INFERRED).
- **The herdr backend could sidestep this.** `_resolve_native_status` reads Herdr's `agent_status`, and Herdr appears to classify Pi panes: herdr-threads fixtures use agent kind `"pi"` (primer §7, INFERRED there). Under the herdr backend, CAO would then get working/idle/done/blocked for Pi without any regex, with a 10 s flush wait after `done` or post-dispatch `idle` before COMPLETED is reported; past 300 s since dispatch it only logs a "giving up" warning, the result is the same (VERIFIED `src/cli_agent_orchestrator/backends/herdr_backend.py:866-912`, `providers/base.py:541-647`). It still needs a provider class to launch Pi. Whether Herdr's `blocked` covers Pi's extension dialogs (`ui_prompt_start`) is unknown.
- **Draft safety is no worse than OMP, but it is a missed opportunity.** Pi exposes `ctx.ui.getEditorText()` to an in-process extension (VERIFIED `pi:packages/coding-agent/src/core/extensions/types.ts:230-237`). CAO's external keystroke path cannot use it.
- **Pi's native wake is not used.** An extension calling `pi.sendMessage(..., {triggerTurn: true})` while `ctx.isIdle()` wakes Pi with no keystrokes (VERIFIED `pi:packages/coding-agent/src/core/agent-session.ts:2378-2383`; upstream example `pi:packages/coding-agent/examples/extensions/file-trigger.ts:14-41`). CAO's OMP extension root is a no-op `export default function () {}` (VERIFIED `providers/omp.py:202`), so it subscribes to nothing.

## Setup and installation footprint

OMP (VERIFIED `providers/omp.py:149-222,339-350`, `docs/omp-cli.md:36-65`):
- Nothing is written to `~/.omp`, `.omp/` or the working tree. Everything goes in `$CAO_HOME_DIR/tmp/omp/<terminal-id>/` (dir 0700, files 0600):
  - `context.md`, passed as `--append-system-prompt`;
  - `index.js` (`export default function () {}`) and `.mcp.json` (`{"mcpServers": {...}}`, with `CAO_TERMINAL_ID` added to stdio servers' env), passed as `--extension <dir>`.
- The directory is removed in `cleanup()` when the terminal is torn down.
- CAO's own state lives in `~/.aws/cli-agent-orchestrator` (overview, Architecture).
- The command is `omp [--model M] [--append-system-prompt F] [--extension D]`. Approval, tool and profile flags are deliberately omitted (VERIFIED `docs/omp-cli.md:38-40`).

Plain Pi, if the same launcher were pointed at a `pi` binary (INFERRED):
- `--model`, `--append-system-prompt <text|path>` and `--extension <dir>` all exist with compatible meaning (VERIFIED `pi:packages/coding-agent/docs/cli.md:65,194-197,229-231`). Pi loads a directory with an `index.js` entry (VERIFIED `pi:packages/coding-agent/docs/extensions.md:46`).
- **The `.mcp.json` sibling trick is OMP-only.** Pi's MCP config is read from `~/.pi/agent/mcp.json`, from a trusted `<project>/.pi/mcp.json`, and from servers an extension registers with `pi.registerMcpServer` (VERIFIED `pi:packages/coding-agent/src/extensions/mcp/config.ts:1-30`, `docs/mcp.md:32,258-262`). A grep for `.mcp.json` in Pi's sources finds no extension-sibling discovery. Pi would load the empty extension and **never start `cao-mcp-server`**, so the agent would have no `send_message`, `handoff` or `assign`.
- Even if registered, Pi's default MCP exposure is `codemode`: the tools are neither declared to the model nor listed in the codemode description; they are reachable only from `codemode` scripts that discover them with `searchTools()`/`describeTool()`/`ALL_TOOLS` (VERIFIED `pi:packages/coding-agent/docs/mcp.md:191-197`). The model would have to know to look for them. A working Pi port would need the extension to call `pi.registerMcpServer("cao", {..., exposure: "direct"})`, or a `toolExposure` override. Either is a one-line extension, and neither needs a config write.

## Token cost

- **Per message:** the body, plus a trailer of about 85 bytes (`[Message from terminal xxxxxxxx. Use send_message MCP tool for any follow-up work.]`; VERIFIED template `utils/orchestration.py:136-138`), **plus a whole new model turn**. That turn re-reads the conversation (prompt-cache dependent) and the model's reply is new output. For a supervisor gathering N worker results, that is N turns at the default `num_messages=1` (INFERRED).
- **Per quiet turn: zero.** Nothing is injected when there is no mail. Delivery is out-of-band through keystrokes (VERIFIED: no hook or extension handler exists).
- **Static, per session:** the appended context file (profile prompt, skill prompt, optional security prompt; `omp.py:174-191`), plus CAO MCP tool schemas wherever the client declares them. `server.py` defines 37 `@mcp.tool` entries, several of them variant definitions (VERIFIED, grep count). How many a given profile exposes is unknown. Under Pi's default `codemode` exposure the schemas would cost almost nothing, but the model would reach the tools only through codemode discovery (above).

## Limitations and bugs

1. **No plain-Pi support.** The OMP provider is bound to the `omp` binary, OMP-only `.mcp.json` discovery, and OMP 17.2.10 screen chrome (VERIFIED, above). Running plain Pi would fail at three points: the binary check, MCP wiring and status detection.
2. **Every message is a user turn.** There is no in-turn delivery for OMP (eager delivery is off by default and unavailable to OMP), no priority, and no "silent until next turn" mode. It contradicts herdr-threads' delivery contract rules 1-3 (INFERRED).
3. **Human drafts get clobbered.** There is no composer read for OMP. A paste lands inside whatever the human typed, and Enter submits both (INFERRED from the absence of a reader; same conclusion as the overview).
4. **Silent drop on OMP.** With no composer reader and no direct probe, a swallowed Enter leaves the row DELIVERED with only a log line (VERIFIED `inbox_service.py:413-511`).
5. **Stale docs on wake triggers.** `docs/inbox-delivery.md` still describes a 5 s `PollingObserver` watchdog that the source no longer contains, so the doc overstates wake coverage (VERIFIED, grep of `src/`).
6. **Version coupling.** The regexes are pinned to captured fixtures of one OMP release (VERIFIED `omp.py:34`, `docs/omp-cli.md:141-143`). That is the opposite of herdr-threads' versionless stance.
7. **Message body in the URL query string.** No size bound, and bodies can show up in access logs (INFERRED; `orchestration.py:1424-1443`).
8. **Handoff results are scraped from the screen.** OMP's `extract_last_message_from_script` relies on two exact ANSI background escapes (`\x1b[49m`, `\x1b[48;2;5;5;5m`) to split user from assistant blocks (VERIFIED `omp.py:57-62,304-334`). A theme change or a Pi port would break it.
9. **Tool restrictions are advisory only.** On OMP they are prompt text only (VERIFIED `docs/omp-cli.md:80-84`, `SECURITY.md:551`).

## Clever ideas worth stealing

- **A per-process, config-free attach directory.** One private dir per terminal (0700/0600), passed by flags, removed on teardown, and never touching the harness's own config (`omp.py:149-222,339-350`). For a herdr-threads Pi adapter, the equivalent is `pi -e <ht-extension>` at managed launch, with `PI_CODING_AGENT_DIR` for tests. Pi supports both (primer §2.1). Our extension would carry the real logic instead of a no-op entry.
- **Additive, never replacing.** Use `--append-system-prompt` and never `--system-prompt`, and leave native approval, tool and profile flags alone (`docs/omp-cli.md:38-46`). That is a good rule for any Pi launch we manage.
- **Native config wins name conflicts.** CAO's MCP entries sit at a lower priority so user definitions win (`docs/omp-cli.md:57-60`). Pi has the same precedence built in: file-configured servers override `registerMcpServer` ones (`pi:packages/coding-agent/docs/mcp.md:260`).
- **Dialog gating with a revert to PENDING.** Never paste into an approval dialog, and treat the refusal as transient rather than failed (`inbox_service.py:349-371`). Our Native wake path should hold the same line. In Pi the in-process signal is `ui_prompt_start`/`ui_prompt_end` (primer §2.2), better than screen matching.
- **Mark before dispatch.** Marking DELIVERED before typing stops re-entrant wake events from double-sending (`inbox_service.py:250-257`). It is cheap and applies to any keystroke wake.
- **Inject the sender id structurally.** The reply path does not depend on the model remembering who wrote to it (`utils/orchestration.py:125-128`, issue #284). We already carry `from <sender>` in the header line; the lesson is to keep reply routing in data, not prose.

**Net for herdr-threads (INFERRED):** CAO's mechanism would be the weakest of the available options on Pi. Pi's extension API gives in-process idle detection (`isIdle`, `agent_settled`), draft protection (`getEditorText`), keystroke-free wake (`sendMessage({triggerTurn: true})`), turn-start delivery (`before_agent_start`) and in-turn delivery (steer queue or `tool_result` append). None of these need screen scraping. A Pi adapter should be a `Channel`-class policy built on those, attached with `-e`, and should not copy CAO's paste-and-Enter design.

## Verification

Adversarial pass against the CAO clone at `69909e98` and the pi-mono clone at `a7b94a03` (both HEADs confirmed).

Checked and held: provider enum; `omp` binary check and command; OMP 17.2.10 pinning; `send_message` tool and query-string body; sender-suffix templates; mark-DELIVERED-before-send; same-sender `\n` join; `num_messages`/100 drain; tmux cancel, load/paste-buffer, 0.3 s delay, `paste_enter_count` (base 2, OMP 1); dialog gating; dead-CLI ERROR; no composer reader; 8 s confirmation and its skip on herdr; herdr `agent_status` mapping; artifact dir, `index.js`, `.mcp.json`, cleanup; omp-cli doc lines; SECURITY.md:551; 37 `@mcp.tool` entries; every cited Pi line (bracketed paste, paste markers, steer on Enter while streaming, `steeringMode` default, `getEditorText`, `triggerTurn`, MCP config paths, exposure table, registration precedence); absence of OMP chrome strings and `.mcp.json` in Pi sources.

Changed:
- Removed the 5 s `PollingObserver` watchdog and its "fast idle-regex precheck" as live triggers: neither exists in `src/` (no `PollingObserver`, `get_idle_pattern_for_log()` has no non-test caller). Updated the Triggers, idle-pattern, Latency, failure-mode and limitation 5 entries.
- Eager delivery also needs the global `CAO_EAGER_INBOX_DELIVERY` flag (default false).
- Herdr-backend 300 s: it is a warning log only, not a different outcome.
- Pi `codemode` MCP tools are reachable from codemode scripts, not unreachable.
- Fixed line citations in `inbox_service.py` (206-296, 349-371, 413-511, 450-452), `constants.py` and `base.py`.
