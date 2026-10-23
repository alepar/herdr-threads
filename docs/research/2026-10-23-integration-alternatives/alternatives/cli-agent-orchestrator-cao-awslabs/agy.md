# CAO (awslabs) x Antigravity CLI (agy): message injection and wake

- Upstream: https://github.com/awslabs/cli-agent-orchestrator, depth-1 clone at
  `69909e98035905b612fd6b117abff8bf8876220f` (2026-10-23). Paths are relative to the clone root.
- Read alongside: `overview.md` (this directory) and `../../primers/agy.md`.
- Tags: **VERIFIED** = read in CAO source/docs (or in a herdr-threads agy capture, named as such);
  **INFERRED** = my reasoning, not observed; **unknown** = no source answers it.
- Nothing was executed; agy is not installed on this host (primer §0), so no live behaviour was observed.

## TL;DR

CAO drives agy purely as a screen: it launches `agy` in a tmux window, gives it CAO's MCP server by
editing the user's **global** `~/.gemini/config/mcp_config.json`, and delivers every inbox message by
**tmux bracketed paste + 1.5 s + one Enter** into the composer when a footer regex says the TUI is
idle. Injection and wake are one act: each message is a new user turn. No agy hook
(`SessionStart`/`PreInvocation`/`Stop`), plugin, `injectSteps`, `agentapi`, headless or SDK surface
is used. Mid-turn delivery is off for agy, the submit-confirmation recovery cannot act for agy (no
composer reader), the herdr backend is documented as unsupported, and the sticky "ready" latch means a
human draft does not block a paste.

## Support status

| Item | Finding | Evidence |
|---|---|---|
| Provider | Native: `AntigravityCliProvider`, provider type `antigravity_cli` | VERIFIED `src/cli_agent_orchestrator/models/provider.py:15`, `providers/manager.py:9,51,179-180` |
| Version observed by CAO | "observed on `agy` 1.0.10, full-screen TUI" (herdr-threads validates 1.3.2/1.3.3) | VERIFIED `providers/antigravity_cli.py:9`; primer §0 |
| Backend | tmux only. herdr "not yet supported: herdr ... skips the FIFO pipeline for providers it has no native status integration for, so agy's state is never observed and terminals time out" | VERIFIED `docs/antigravity-cli.md:20-24` |
| Code gate for herdr | None found: the provider still calls `_resolve_native_status()` and `_resolve_buffer()` at the top of `get_status`, so it is not refused, only documented as broken | VERIFIED `providers/antigravity_cli.py:730-738`; grep for a herdr/agy refusal found nothing. INFERRED: since Herdr does detect agy (`agent_status`, primer §9), the doc may be stale; untested |
| Launch | `agy --dangerously-skip-permissions [--model "<m>"] [-i "<guarded system prompt>"]` | VERIFIED `providers/antigravity_cli.py:328-371`, `docs/antigravity-cli.md:35-43` |
| Tool restriction | Soft (prompt text only), `PROVIDER_ENFORCEMENT["antigravity_cli"] = PROMPT` | VERIFIED `utils/enforcement.py:46`, `providers/antigravity_cli.py:350-356`, `docs/antigravity-cli.md:83-85` |
| Mid-turn (eager) delivery | Not enabled: agy does not override `accepts_input_while_processing` (base default `False`; only Claude Code opts in) | VERIFIED `providers/base.py:377-387`, `docs/inbox-delivery.md:85-90`; no override in `antigravity_cli.py` |

## Injection mechanism

### Path, end to end

1. **Sender** (any CAO agent) calls the MCP tool `send_message(message, receiver_id?)` in its
   `cao-mcp-server` subprocess. VERIFIED `mcp_server/server.py:926-958`.
2. `_send_message_impl` resolves the receiver (default: the recorded caller), refuses self-send and
   receivers outside the assign/handoff tree, then appends the sender suffix. VERIFIED
   `utils/orchestration.py:2442-2537`.
3. `_send_to_inbox` does `POST {API_BASE_URL}/terminals/{receiver}/inbox/messages` with
   `sender_id` and `message` as **query parameters**. VERIFIED `utils/orchestration.py:1391-1453`
   (`params = {"sender_id": sender_id, "message": message}`, `requests.post(..., params=params)`).
4. The endpoint inserts a PENDING row and starts `inbox_service.deliver_pending` on a worker thread,
   answering once the paste reached the pane or after 5 s. VERIFIED `api/main.py:8373-8447`,
   `constants.py:393`.
5. `_deliver_pending_locked` reads **one** pending message (default `num_messages=1`), checks status is
   IDLE/COMPLETED, marks it DELIVERED, then calls `terminal_service.dispatch_input`. VERIFIED
   `services/inbox_service.py:210-296`.
6. `dispatch_input` refuses on ERROR, on an open dialog for providers that block orchestrated input
   (agy does), and if the pane's foreground process is the pre-launch shell; prepends a one-time
   `<cao-memory>` block to the first message; arms the status monitor; samples the dispatch boundary;
   calls `provider.mark_input_received()` (agy increments `_turns`); then `get_backend().send_keys(...,
   enter_count=1, force_bracketed_paste=True, submit_delay=1.5)`. VERIFIED
   `services/terminal_service.py:4580-4715`, `services/terminal_service.py:387-412`,
   `providers/antigravity_cli.py:248-278,980-983`.
7. The tmux backend: `send-keys -X cancel` (leave copy mode), `load-buffer` + `paste-buffer -p`
   (tmux >= 3.7) or hand-wrapped `ESC[200~…ESC[201~` with `paste-buffer -r` (tmux < 3.7), sleep
   `submit_delay`, `send-keys -X cancel` again, `send-keys Enter`. VERIFIED
   `clients/tmux.py:1686-1866`, `docs/inbox-delivery.md:5-8`.

The agy-specific tuning is two properties:

```python
# providers/antigravity_cli.py:261-278
def paste_submit_delay(self) -> float:
    """Gemini 3.x ``agy`` needs longer than the 0.3s base default to settle
    the bracketed-paste end marker. An Enter sent that soon is consumed as a
    literal newline inside the input box, so the pasted task is left
    UNSUBMITTED ..."""
    return 1.5
def paste_enter_count(self) -> int:
    return 1
```

VERIFIED. INFERRED from that comment: agy does enable bracketed paste (DECSET 2004), which the primer
§8 lists as unknown. On tmux >= 3.7 this matters: if agy did not enable 2004, `paste-buffer -p` would
send raw text and each line of a multi-line message would submit separately (`docs/inbox-delivery.md:8`).

### What reaches the model

- A plain **user turn** typed into the composer. agy's transcript would log it as
  `USER_EXPLICIT/USER_INPUT` (primer §7), not a `SYSTEM_SDK` hook step. INFERRED from the primer; CAO
  never reads the transcript.
- Payload = `[<cao-memory>…</cao-memory>\n\n]` (first message after init only, when memory exists) +
  body + `"\n\n"` + suffix:

```python
# utils/orchestration.py:137-139
MESSAGE_SUFFIX_TEMPLATE = (
    "[Message from terminal {terminal_id}. Use send_message MCP tool for any follow-up work.]"
)
```

  `assign` uses `[Assigned by terminal {id}. When done, send results back to terminal {id} using
  send_message]` (`:133-136`). Suffix on by default (`CAO_ENABLE_SENDER_ID_INJECTION`, `:128`);
  suppressed only for kiro-cli `@prompt` invocations, never for agy (`:150`, `:210-212`). VERIFIED.
- No message id, no thread, no priority, no timestamp, no ACK request in the framing. VERIFIED (the
  suffix is the only frame; the inbox schema has six columns, overview "Delivery guarantees").

### When it fires

- Only when the receiver's status is IDLE or COMPLETED. agy is never eager, so a message that arrives
  mid-turn waits until the footer returns to `? for shortcuts`. VERIFIED
  `services/inbox_service.py:217-248`.
- Never into WAITING_USER_ANSWER: agy sets `blocks_orchestrated_input_while_waiting_user_answer =
  True` because "approval dialogs / pickers consume pasted text as the answer". VERIFIED
  `providers/antigravity_cli.py:248-258`, `services/terminal_service.py:181-200,4603-4609`. (For a
  non-eager provider like agy the inbox status gate already skips WAITING_USER_ANSWER; the flag adds
  the `dispatch_input` refusal and the PENDING revert, `services/inbox_service.py:349-371`.)
- No injection at turn start, after a tool call, or before a model call: CAO uses no agy hook at all.
  VERIFIED (no hooks.json or plugin written; grep for `hooks.json`/`injectSteps` in the agy provider
  finds nothing).

### Size bounds

- No length check on the inbox POST: `message: str` is an unconstrained query parameter. VERIFIED
  `api/main.py:8373-8380`. (Other endpoints carry `max_length=262_144`, `api/main.py:1294,1334,1347`;
  this one does not.)
- INFERRED: the practical bound is the HTTP request-line limit of the server stack (message sent in
  the URL query string), which is unknown here; a large body would fail at the HTTP layer rather than be
  truncated. No truncation or continuation exists. The full body is pasted into agy's composer
  verbatim.

### Batching, ordering, dedupe, ack

- One message per IDLE/COMPLETED cycle in practice: every caller uses the default `num_messages=1`
  (`services/inbox_service.py:133,531`, `api/main.py:1670,8417`); the drain-to-100 mode joined by `\n`
  exists but has no caller. VERIFIED by grep. So N queued messages cost N agy turns.
- Oldest first. VERIFIED `clients/database.py:3041-3043` (`created_at` ascending), also the tool
  docstring `mcp_server/server.py:939-940`.
- Dedupe: rows are marked DELIVERED **before** the paste, so re-entrant status events cannot paste
  twice; a per-terminal lock serializes read-mark-send. VERIFIED `services/inbox_service.py:250-257`,
  `docs/inbox-delivery.md:128` (residual duplicate-wakeup race, GH #115; that doc names a
  `check_and_send_pending_messages()` that no longer exists in `src/`, so it is partly stale).
- No acknowledgement: DELIVERED means "typed into the pane"; `after_dispatch` "is never an
  acknowledgement that the receiver accepted the message". VERIFIED `services/inbox_service.py:180-186`.

### Role prompt and handoff return (adjacent injection)

- Launch-time context: the profile system prompt, skill catalog and (if restricted) security prompt go
  in via `-i` as the **first user message**, with a guard: "Acknowledge your role in one sentence, then
  wait for tasks. Do not take any action or use any tools until you receive a specific task." VERIFIED
  `providers/antigravity_cli.py:346-365`. agy has no system-prompt flag in CAO's usage; this costs one
  model turn at launch.
- A handoff's result is not a message: the server waits for COMPLETED and scrapes the last response
  from the screen via `extract_last_message_from_script` (text between the last echoed `> <query>` and
  the next `─` rule, minus `▸` thoughts, `●` tool lines, tips, spinner, survey). VERIFIED
  `providers/antigravity_cli.py:862-945`.

## Wake mechanism

### Mechanism

Wake = injection: the paste + Enter into an idle composer submits a new turn. There is no separate
content-free wake marker and no agy-native wake (`agentapi send-message`, `/schedule`, Remote
Control, stream-json, SDK are all unused). VERIFIED (`docs/inbox-delivery.md:10-19`; grep of the
provider).

Triggers that call `deliver_pending` for agy (VERIFIED in source):

1. Immediate attempt inside the POST (`api/main.py:8396-8433`).
2. `InboxService.run` on `terminal.*.status` IDLE/COMPLETED events (one task per terminal, coalesced;
   `services/inbox_service.py:90-158`).
3. Reconcile sweep every 30 s for rows PENDING > 30 s (`constants.py:345,351`, `api/main.py:248-261`,
   `services/inbox_service.py:553-588`).

`docs/inbox-delivery.md:10-15` also documents a 5 s watchdog `PollingObserver` on the log with an
idle-pattern precheck. No such observer exists in `src/` at this commit: grep finds no `watchdog`
import, no `PollingObserver`, and no caller of `get_idle_pattern_for_log` (agy defines it,
`providers/antigravity_cli.py:854-856`, but nothing calls it). The only 5 s poller is
OpenCode-specific (`api/main.py:237-244`). VERIFIED by grep; the doc is stale.

### Idle/busy detection for agy

Footer-anchored screen classification over a **pyte-composited viewport** (on by default,
`CAO_PYTE_STATUS=true`, `constants.py:285`; agy opts in with `supports_screen_detection = True`,
`providers/antigravity_cli.py:770-778`). Order (`get_status_from_screen`, `:804-852`), over the last
12 non-blank rows:

| Status | Rule |
|---|---|
| WAITING_USER_ANSWER | `↑/↓ navigate`, `[y/n]`, `Allow once/always`, `Do you want to …`, `enter Toggle/Confirm` (`:152-157`) |
| PROCESSING | footer `esc to cancel` or a braille spinner line ending in `...`/`…` (`:110,119`) |
| IDLE / COMPLETED | footer `? for shortcuts`; IDLE if `_turns == 0`, else COMPLETED (`:113,846-847`) |
| ERROR | `Error:`, `panic:`, `agy: …error/failed`, `Traceback` anywhere (`:160-162`) |
| UNKNOWN | anything else |

Why pyte: "the append-only pipe-pane log keeps BOTH strings ... the stale `esc to cancel` pins the
terminal to PROCESSING forever" (`:771-777`). A mid-burst probe catches the rising edge so a running
turn is not read as IDLE ("the delivery check re-sent a task that the worker was already running",
`:780-802`). VERIFIED.

IDLE vs COMPLETED cannot be told from the screen, so it is a CAO-side turn counter bumped on every
`dispatch_input` (`:238-246,980-983`). INFERRED: a turn the human types by hand does not bump it.

Cross-check against herdr-threads' genuine agy 1.3.2 captures (VERIFIED,
`tests/fixtures/agy/composer/`): empty composer shows `? for shortcuts` (`empty.read-detection.txt`),
busy shows `⣷  Generating...` and `esc to cancel` (`busy.read-detection.txt`), so CAO's two main rules
hold on 1.3.2. Applying CAO's `get_status_from_screen` precedence and regexes (copied as strings, not
CAO code) to the bottom 12 non-blank rows of each fixture gives: `empty` IDLE/COMPLETED, `busy`
PROCESSING, `draft` UNKNOWN, `permission-prompt` WAITING_USER_ANSWER (`↑/↓ Navigate`, line 25),
`subagent-generating` and `subagent-tool-running` **IDLE/COMPLETED**. VERIFIED by that regex check.
The subagent screens draw `? for shortcuts` with no `esc to cancel` and no spinner row
(`subagent-generating.read-detection.txt:18-20`), so CAO, like Herdr's `agent_status` (primer §9),
reads an agy whose subagent is still working as ready, and pastes the next inbox message into it.

### Draft safety

None for agy, and the latch makes it worse than the screen suggests:

- agy provides no `extract_current_composer`, so CAO cannot see composer text. VERIFIED (base default
  `None`, `providers/base.py:340-347`; no override in `antigravity_cli.py`).
- With a typed draft, agy 1.3.2 replaces the `? for shortcuts` footer with a blank-led one (VERIFIED
  capture `tests/fixtures/agy/composer/draft.read-detection.txt:9-11`), so a fresh classification would
  read UNKNOWN.
- But the status monitor latches ready states and **refuses ready → PROCESSING/UNKNOWN downgrades**
  unless an input dispatch armed a revert. VERIFIED `services/status_monitor.py:33-60,359-364`.
- INFERRED consequence: once agy is latched COMPLETED, an operator who attaches and starts typing still
  reads COMPLETED, and the next inbox message is pasted into the draft and submitted with it. CAO
  assumes panes are machine-owned (overview "Wake mechanism").

### Latency

- Idle receiver: paste + 1.5 s `paste_submit_delay` + Enter, plus status-read time. VERIFIED delay;
  INFERRED total about 2 s.
- Busy receiver: waits for the end of the current turn, then the next IDLE/COMPLETED edge (pyte
  quiescence 0.2 s, `constants.py:306`), else the 30 s sweep (effective 30-60 s); there is no 5 s
  watchdog for agy (above). VERIFIED constants; INFERRED end-to-end.
- Queues drain at one message per agy turn.

### Failure modes

- **Swallowed Enter (#78) is unrecoverable for agy.** Submit confirmation waits 8 s for a post-dispatch
  status; recovery is bare Enter only for providers that read their composer (codex, claude_code,
  kiro_cli) and full re-send only with composer evidence; "neither read its composer nor probe (others):
  nothing; the drop is logged". agy is in "others". VERIFIED `docs/inbox-delivery.md:21-33`,
  `services/inbox_service.py:413-510`. The row stays DELIVERED, the text sits in agy's composer, and the
  1.5 s delay exists precisely because this happened on agy (`providers/antigravity_cli.py:261-270`).
- **Pile-up.** A second delivery after a swallowed Enter pastes on top of the first unsubmitted text
  (`docs/inbox-delivery.md:23`). INFERRED for agy, as above.
- **Startup dialogs.** Workspace trust and the feedback survey block IDLE; CAO answers them itself
  (Enter accepts "Yes, I trust this folder"; `0`+Enter skips the survey). VERIFIED
  `providers/antigravity_cli.py:132-146,543-648`. A new agy dialog CAO has no regex for would hang
  init until the 180 s floor (`:85-86,700-708`).
- **Stale footer on raw path.** With `CAO_PYTE_STATUS=false` the documented stale-`esc to cancel` bug
  returns (`:771-777`). VERIFIED.
- **Process gone.** Pasting into a bare shell is refused by the foreground-process check, residual
  window of one tmux round trip acknowledged in code. VERIFIED `services/terminal_service.py:4492-4526,4622-4644`.
- **Message lost on error.** A message whose turn errors is still DELIVERED; nothing re-offers it.
  VERIFIED (no ACK model).

## Setup / installation footprint on the harness

| Write | Detail | Evidence |
|---|---|---|
| `~/.gemini/config/mcp_config.json` (global file, primer §4; whether the IDE/App also read it: unknown) | Read-modify-write merge; adds one entry per MCP server per terminal keyed `{server_name}-{terminal_id}` with `env.CAO_TERMINAL_ID` and optional `cwd`; atomic temp+rename, mode preserved; in-process lock only | VERIFIED `providers/antigravity_cli.py:302-304,373-470` |
| Same file, cleanup | Removes only entries whose `env.CAO_TERMINAL_ID` matches; on every register, prunes entries whose terminal id has no DB row | VERIFIED `:472-541,955-977` |
| Same file, corruption handling | Unreadable JSON, a non-object root or a non-object `mcpServers` is replaced with `{}` and **written back**, discarding the user's content | VERIFIED `:404-427` |
| Workspace trust | Accepted automatically for the session cwd (persisted by agy, location unknown) | VERIFIED `:615-625`; persistence INFERRED |
| hooks.json, plugins, skills, settings.json permissions | None written | VERIFIED (grep) |

Prerequisite: the user installs agy and completes sign-in and onboarding once by hand
(`docs/antigravity-cli.md:9-10`).

## Token cost

- **Per message:** one full agy turn per message (one message per cycle). Model-visible overhead is the
  suffix, 83 bytes including the 8-hex id (`[Message from terminal xxxxxxxx. Use send_message MCP tool for any
  follow-up work.]`), plus the `"\n\n"` separator (about 20 tokens). The real cost is the turn: the model re-reads its context and
  usually replies, and a reply is another `send_message` that costs the peer a turn. INFERRED.
- **First message only:** the `<cao-memory>` block, size unbounded here (unknown).
- **Per launch:** the `-i` system prompt + skill catalog + guard, spent as a full user turn plus the
  model's one-sentence acknowledgement. VERIFIED mechanism; size depends on profile.
- **Per model call (always):** the `cao-mcp-server` tool schemas. `server.py` registers about 35 tools
  (handoff, assign, send_message, memory_*, workflow_*, …; 37 `@mcp.tool` decorators, some
  conditional), with access gated mostly at call time (`mcp_server/server.py:1280,1657`). INFERRED: a
  standing schema cost of thousands of tokens per request, larger than any per-message framing.
- **Quiet turn:** zero injection (no hooks). VERIFIED.

## Limitations and bugs I can see

1. **Cross-terminal MCP identity leak (INFERRED, high).** Entries are added at launch and removed only
   at cleanup, so while terminal A lives, `mcp_config.json` still holds `cao-mcp-server-<A>` with
   `CAO_TERMINAL_ID=A` when terminal B launches. agy loads every server in the global file, so B's agy
   also spawns A's server and can call `send_message`/`handoff` **as A**. The doc's serialization claim
   ("each agy process reads the config ... before the next terminal writes", `docs/antigravity-cli.md:75`)
   only protects B's own entry, not against seeing A's. The same applies to the user's personal agy
   CLI sessions started while CAO terminals live (IDE/App only if they read this file, unknown): they
   gain CAO tools bound to some worker's identity. Evidence for the mechanism:
   `providers/antigravity_cli.py:373-470` (add), `:955-977` (removal in `cleanup`; stale-owner prune
   only on a later register, `:522-541`), primer §4 (global file). Not observed live.
2. **Writes the user's global agy config** and can overwrite it with `{}` on a parse error
   (`:404-427`); a second cao-server process or agy itself can lose an update (acknowledged, `:391-393`).
3. **Draft clobbering** via the sticky ready latch (above).
4. **No recovery for swallowed Enter on agy** (above); the fix is a fixed 1.5 s sleep.
5. **No mid-turn delivery**: agy's `PreInvocation` `injectSteps` (primer §2.3) would allow it, but CAO
   uses keystrokes only, so a busy agy worker sees nothing until its turn ends.
6. **N turns for N messages**; no batching in practice.
7. **Auto-accepting workspace trust** for any cwd CAO launches in, and `--dangerously-skip-permissions`
   always (`:328`). Soft-only tool restriction.
8. **Screen-scraped handoff results** depend on agy's chrome glyphs (`▸`, `●`, `─`, `? for shortcuts`);
   a renderer change breaks extraction or status, and CAO's patterns were observed on 1.0.10.
9. **herdr backend unusable for agy by its own docs**, so CAO-on-Herdr cannot host agy today.
10. **Unbounded message size** sent as a URL query parameter.
11. **IDLE vs COMPLETED by CAO's own counter**: a human-typed turn or an agy restart in the pane is
    invisible to it (INFERRED). Worse, a turn the human submits is not armed by `notify_input_sent`,
    so the latch should keep reading COMPLETED through it and an inbox paste can land mid-turn
    (INFERRED from `services/status_monitor.py:359-364`).
12. **False ready during subagent work**: the live-subagent screen matches the idle footer (above), so
    CAO pastes into an agy that is still working. VERIFIED by applying CAO's regexes to the capture.

## Clever ideas worth stealing

1. **Dispatch boundary as causal evidence.** Sample an output-generation counter after arming and
   before the first key, and accept only a status earned at a later generation as proof the paste was
   submitted (`services/terminal_service.py:4675-4690`, `services/inbox_service.py:396-412`). Our Native
   wake into agy could confirm "the marker was submitted" the same way (screen generation from Herdr, or
   the `PreInvocation` with `invocationNum` 0 that a submitted wake triggers) instead of trusting the
   keystroke.
2. **Per-harness paste-to-Enter delay and Enter count** as adapter properties (`:261-278`). If our wake
   marker is ever swallowed on agy under load, this is the knob; a recovery should then read the
   composer before pressing Enter again, as CAO does for its composer-readable providers.
3. **Pyte-composited viewport for footer detection** instead of the raw stream, plus a positive-evidence
   mid-burst probe (`:770-802`). Relevant if we ever read agy screens ourselves rather than through
   Herdr's `agent_status`, which the primer notes reports false idle during subagents.
4. **Dialog guard as a capability flag** (`blocks_orchestrated_input_while_waiting_user_answer`), applied
   on every machine-originated path including recovery Enters (`docs/inbox-delivery.md:35`). Our
   `AgyComposer` already reads a permission screen as `Unreadable`; the lesson is to apply the same check
   to any retry Enter.
5. **Ownership-tagged, liveness-pruned config entries** (`env.CAO_TERMINAL_ID` as owner tag, prune rows
   whose owner is gone, `:472-541`). A sound pattern for any shared-file registration, though our owned
   plugin directory avoids shared-file edits in the first place.
6. **Mark DELIVERED before typing, revert to PENDING on transient refusals** (`services/inbox_service.py:250-257,338-371`).
   This closes re-entrant double delivery without an atomic claim. We confirm from the transcript instead,
   which is stronger, but the transient/terminal split on refusal reasons is a useful model.

Not worth stealing for herdr-threads: content-bearing keystroke delivery (it breaks delivery-contract
rules 2-4 and spends a turn per message), writing the global `mcp_config.json`, auto-accepting trust,
and scraping replies from the screen.

## Compared with herdr-threads on agy (brief)

| Concern | CAO | herdr-threads (primer §10) |
|---|---|---|
| Injection | Paste body as a user turn | `PreInvocation` `injectSteps` `systemMessage` (downgrade to `userMessage`), every top-level model call |
| Mid-turn | Never for agy | Yes, before each post-tool model call |
| Wake | The paste itself, every message | Content-free `herdr-threads: wake up`, only for priority <= T |
| Idle detection | CAO footer regex on a pyte screen + own turn counter | Herdr `agent_status` + `AgyComposer` screen read |
| Draft safety | None (sticky latch) | Any non-empty composer reads `Unreadable`: no wake |
| Delivery confirmation | Status change after dispatch; no recovery for agy | Transcript scan for `SYSTEM_SDK` steps |
| Harness config | Global `~/.gemini/config/mcp_config.json` per-terminal entries | Owned plugin dir `~/.gemini/config/plugins/herdr-threads/` |
| Batching | 1 message per turn | Pages of up to 32 items / 64 KiB, bodies at 8 KiB |

## Verification

Adversarial pass against the clone at `69909e98` and herdr-threads' agy 1.3.2 fixtures; nothing from
the clone was executed.

- Checked and held: provider registration, 1.0.10 note, tmux-only/herdr doc, launch flags and `-i`
  guard, soft enforcement, `accepts_input_while_processing` default, send path (`send_message`,
  query-param POST, 5 s immediate wait, DELIVERED-before-paste, `num_messages=1` callers), suffix text
  and 83-byte length, `<cao-memory>` prepend, `paste_submit_delay` 1.5 / Enter count 1 (base 0.3 / 2),
  tmux paste path, pyte opt-in and status precedence, `_turns` counter, sticky latch, submit-confirmation
  table and agy's lack of `extract_current_composer`, startup dialogs and 180 s floor, `mcp_config.json`
  merge/cleanup/prune/corruption reset, 37 `@mcp.tool` decorators, no `max_length` on the inbox POST.
- Changed: removed the 5 s watchdog trigger and its latency step (documented, absent from `src/`);
  fixed dialog-guard and process-gone citations (`terminal_service.py:595-608`/`615-645` pointed at
  unrelated rollback code); oldest-first now cites the DB query; latch blocks ready → PROCESSING too;
  replaced the "unknown" subagent/permission paragraph with a regex check of the fixtures (subagent
  screens read ready: new limitation 12); dropped the unsupported "shared by IDE and App" attribution
  to primer §4; added the human-turn latch inference to limitation 11.
