# hcom x Claude Code: message injection and wake-up

- Upstream: https://github.com/aannoo/hcom. Clone `alts/hcom` (scratchpad), HEAD `de33e31a0cdb` (2026-10-21, v0.7.31), MIT, 597 stars / 94 forks on 2026-10-23 (see `overview.md`). The clone was only read. Nothing in it was built or run.
- Citations are `path:line` relative to the clone root unless prefixed. **V** = VERIFIED (read in source or docs), **I** = INFERRED, **unknown** = not found.
- Claude harness facts come from `../../primers/claude.md` (abbreviated *primer §N*). Mod type facts come from the engine's bundled types for 2.1.308, `/tmp/claude-1001/bundled-skills/2.1.308/3f619893785228e70faaa08b2d139759/plugin-authoring/types/claude-code.d.ts` (abbreviated *d.ts*).

## 1. Support status

**Native. Claude is hcom's most complete integration.** **V**

hcom runs Claude in three modes. Each one injects and wakes differently.

| Mode | How it starts | Injection | Idle wake |
|---|---|---|---|
| Interactive, `claude_delivery native` (default since 0.7.31) | `hcom claude` → `claude-pty` PTY proxy + `--settings` hooks + `--plugin-dir <mod>` | `PostToolUse` / `UserPromptSubmit` `additionalContext` (+ `systemMessage` echo) | Mod's `$.prompt.submit({text:'<hcom>', asUser:true})`, driven by a `hcom claude-mod-poll` long-poll |
| Interactive, `claude_delivery inject` (opt-in) | Same PTY proxy, no mod | Same hooks | PTY proxy types `<hcom>` + Enter, after a 6-step screen-scrape gate |
| Headless `-p`, and Task subagents | `--settings` hooks only (no mod for `-p`) | `Stop` / `SubagentStop` returns `decision:"block"` with the batch as `reason`. Subagents also get `PostToolUse` | The Stop hook itself long-polls (up to 86,400 s for `-p`, 30 s default for subagents) |

Evidence:
- The mode is frozen per launch. Print sessions skip the mod (`src/hooks/claude.rs:2848-2870`).
- The default is `DEFAULT_DELIVERY = "native"` (`src/config.rs:278`), and CHANGELOG.md:20 records the change to native in 0.7.31.
- Hooks are inert outside hcom launches: they return immediately without `HCOM_PROCESS_ID` (`src/hooks/claude.rs:135-140`).

## 2. Injection mechanism

### 2.1 Hook set (per-run `--settings`)

`CLAUDE_HOOK_CONFIGS` (`src/hooks/claude.rs:2699-2718`) **V** registers 13 events. Each one is an exec-form `command` hook, `{"type":"command","command":<hcom>,"args":[..., "<suffix>"]}` (`:2982-2987`):

| Event | Suffix | Timeout | Delivery role |
|---|---|---|---|
| SessionStart | sessionstart | default | Bootstrap (~700 tokens, README.md:231) as `additionalContext`; binds session → process (`:1152-1227`) |
| UserPromptSubmit | userpromptsubmit | default | **Turn-start delivery** in PTY mode, on any prompt (`:1843-1919`) |
| PreToolUse (Bash\|PowerShell\|Agent\|Task\|Write\|Edit) | pre | default | Status/actor tracking; no mail |
| PostToolUse (all tools) | post | **86400** | **Mid-turn delivery** (`:1648-1701`) |
| Stop | poll | **86400** | PTY: set `listening` and notify. Headless: block-loop delivery (`:1789-1840`) |
| SubagentStop | subagent-stop | **86400** | Subagent block-loop delivery (`:2504-2662`) |
| Notification | notify | default | `idle_prompt` → `listening` (`:1965-1970`) |
| PermissionRequest | permission-request | default | `blocked` status (`:1922-1946`) |
| PostToolUseFailure, PermissionDenied, StopFailure, SubagentStart, SessionEnd | — | default | Lifecycle only |

It also sets `env.CLAUDE_CODE_STOP_HOOK_BLOCK_CAP = "0"` (`:2813-2818`) **V**. The comment says Claude's 8-in-a-row block cap would otherwise "drop the message hcom has already handed over". Two things are unknown: whether `0` means "unlimited" or "zero blocks allowed" in Claude, and whether the primer's statement that the cap is "raised" with this variable covers the value 0 (primer §1.3).

### 2.2 Mid-turn: `PostToolUse`

`handle_posttooluse` → `get_posttooluse_messages` (`src/hooks/claude.rs:1685-1689, 1724-1743`) **V**:

```rust
let prepared = common::prepare_pending_messages(db, instance_name)?;
let model_context = common::format_messages_json_for_instance(db, &prepared.messages, instance_name);
let user_display  = common::format_hook_messages_for_instance(db, &prepared.messages, instance_name);
json!({ "systemMessage": user_display,
        "hookSpecificOutput": { "hookEventName": "PostToolUse", "additionalContext": model_context } })
```

- It fires after **every** tool call of the root session. The matcher is empty, so every tool qualifies (`:2708`) **V**. Claude places the result beside the tool result as a system reminder (primer §1.2) **V**.
- On a shell tool, a one-time fallback bootstrap can be merged in front of the mail. Contexts are joined with `\n\n---\n\n` and `systemMessage`s with ` + ` (`:1676-1683, 1746-1786`) **V**.
- A turn with no tool calls gets no mid-turn delivery. Its mail waits for Stop → idle wake (I, from the hook set).

### 2.3 Turn start: `UserPromptSubmit`

`handle_userpromptsubmit` (`src/hooks/claude.rs:1872-1884`) **V**: when `ctx.is_pty_mode`, any prompt drains the queue into `additionalContext` plus `systemMessage`. The trigger can be the human's own prompt or the `<hcom>` wake. hcom's Codex integration differs: there, only an exact `<hcom>` prompt pulls mail (overview.md, "Codex delivers at `UserPromptSubmit` only for the wake prompt").

A bare `<hcom>` wake that arrives with nothing to deliver is refused in two places:
- in the handler, when another hook already delivered the batch (`:1909-1916`);
- in the router, when the session cannot be attributed: no binding, a historical generation, or a subagent view (`:356-368, 437-444, 477-481`).

The refusal returns `{"decision":"block","reason":…,"suppressOriginalPrompt":true}` (`:198-205`) **V**, so the model never sees an empty wake. A Claude `UserPromptSubmit` block shows `reason` to the user and keeps it out of context (primer §1.2) **V**.

Caveat: the engine types list `suppressOriginalPrompt` as **`hookSpecificOutput.suppressOriginalPrompt`** (d.ts:1336-1339, 1397). hcom emits it at the top level, so Claude probably ignores it (I). This fits hcom's own screen code, which handles "Claude puts a prompt its UserPromptSubmit hook blocked back into the box" (`src/pty/screen.rs:912-920`) by pressing Enter on the leftover `<hcom>` (inject mode, `src/delivery.rs:1066-1077`) **V**. It is unknown whether a mod-submitted `<hcom>` that gets blocked lands in the human's composer in native mode.

### 2.4 Headless and subagents: Stop/SubagentStop block-loop

`handle_poll` (`src/hooks/claude.rs:1789-1840`) **V**:
- In PTY mode it sets `listening`, pings notify ports and returns empty (`:1804-1809`).
- In `-p` mode it runs `common::poll_messages(db, name, timeout, …)` (`:1820`). The timeout is config `timeout`, default 86,400 s (`src/config.rs:238`).
- The loop `src/hooks/common.rs:524-632` works like this:
  1. read unread messages;
  2. fence on the row's `created_at` (so a stop+resume is detected);
  3. if there is mail, return `{"decision":"block","reason": <formatted batch>}`;
  4. otherwise `wait_readable` on a private `127.0.0.1:0` TCP listener (30 s slices) and drain the wake connections.
- Before returning mail it checks for orphaning: if stdin reports POLLERR/POLLNVAL after at least one wait, and the instance is not a background one, it returns nothing (`:556-562, 634-642`) **V**.
- It always exits 0: "Claude ignores stdout JSON on exit 2 for Stop" (`:1836-1838`; `src/hooks/common.rs:434-436`) **V**.

`subagent_stop` uses the same poll with `subagent_timeout`, default 30 s (`src/config.rs:219`; `src/hooks/claude.rs:2602-2606`). On the first delivery it prepends the subagent bootstrap to `reason` (`:2609-2632`) **V**. `subagent_posttooluse` delivers mid-turn into subagents (`:2667-2690`) **V**. Subagent hooks are routed by `agent_id`, and only when the shared `session_id` has an hcom root binding (`:325-410`) **V**.

### 2.5 Foreground Task "freeze" summary

During a foreground Task, the root's hooks do not fire for the subagent's tool calls. When the Task's `PostToolUse` arrives, `end_task` → `deliver_freeze_messages` delivers everything since the root cursor. That covers subagent-to-subagent chatter and root-scoped mail. It goes out as one plain-text `additionalContext` block, `[Task tool completed - Message history during Task tool] … [End of message history. …]` (`src/hooks/claude.rs:1230-1427`) **V**. Background Tasks (`status:"async_launched"`, since CC 2.1.198) are skipped (`:1240-1247`) **V**. A root `PostToolUse` for a Task/Agent tool returns only `end_task`'s output and never reaches the normal §2.2 delivery (`:490-495`) **V**.

### 2.6 Payload format and framing

`format_messages_json` wraps the whole batch in one tag (`src/messages.rs:656-671`) **V**:

```
<hcom>[request:review #812] luna → nova: please check the diff</hcom>
<hcom>[3 new messages] | [inform #813] luna → nova (+2): … | [thread:x #814] … | … | [<hints>]</hcom>
```

- The prefix is `[intent:thread #id]`, `[intent #id]`, `[thread:name #id]` or `[new message #id]`. A relayed message uses `#id:DEVICE` (`:492-536`) **V**.
- Batch items are joined with ` | ` on one line, and bodies are not escaped (`:603-624`) **V**. A body containing ` | ` or `</hcom>` therefore makes the batch ambiguous to parse. Only the PTY preview strips tags (`src/delivery.rs:630-640`) **V**/I. This is the same cooperative-trust stance as ours, but less parseable than one header line per item.
- Per-instance or config `hints` are appended to **every** delivery as ` | [hints]` (`src/messages.rs:626-640`) **V**. The default `hints` is empty (`src/config.rs:340`), so this costs tokens only when someone configures it **V**.
- One-time tips (`recv:thread:*`, `recv:intent:*`) are appended on first receipt (`src/messages.rs:642-651`) **V**.
- `systemMessage` is the same text without the `<hcom>` wrapper and without tips. It is shown to the human only (primer §1.2) **V**.

### 2.7 Size bounds

- At most 50 messages per delivery (`MAX_MESSAGES_PER_DELIVERY`, `src/shared/constants.rs:19`; `src/hooks/common.rs:261-267`) **V**.
- A per-message cap of 1 MiB at send time (`src/shared/constants.rs:22`; `src/messages.rs:143-148`) **V**.
- **No per-delivery byte cap** **V** (absence in `prepare_raw_messages`, `src/hooks/common.rs:351-395`).
- Claude caps `additionalContext` at **10,000 chars**. Above that it saves the text to a file and shows a 2,000-char preview, "Claude Code doesn't ask Claude to read the file" (primer §1.2) **V**. hcom commits the cursor after the stdout flush whatever Claude does with the text, so a batch over 10 k chars is acked although the model saw only a 2 k preview (I). See §7.
- The freeze summary has no 50-message limit either (`src/hooks/claude.rs:1272-1413`) **V**.

### 2.8 Dedupe and acknowledgement

- **Cursor = ack.** One forward-only `last_event_id` per instance. Unread means `id > cursor AND type='message'`, filtered by scope and mention in Rust (`src/db/events.rs:118-143`) **V**.
- **Prepare, then commit after flush.** `prepare_pending_messages` does not move the cursor (`src/hooks/common.rs:304-311`). The dispatcher writes stdout, flushes it, and only then calls `commit_delivery_ack` (`src/hooks/claude.rs:172-184`) **V**.
- `ack_hook_delivery` is forward-only and fenced by `created_at`, so a delayed ack never rewinds a newer delivery and an ack from a replaced row is void (`src/hooks/common.rs:313-346`) **V**.
- Consequence: delivery is at-least-once up to the flush. Concurrent hooks (PostToolUse racing UserPromptSubmit) can both read the same batch before either commits. Nothing locks the rows between prepare and commit, so duplicate delivery is possible in that window (I).
- Exception: `end_task` writes `last_event_id` **before** its stdout is written (`src/hooks/claude.rs:1254-1261`), and does it with a plain position update instead of the fenced ack **V**. The freeze summary is therefore at-most-once (I).
- There is no model-receipt ACK. "Ack" means "the hook printed it" **V**. Semantic acks are messages sent with `--intent ack` (overview.md) **V**.

## 3. Wake mechanism

### 3.1 Idle/busy source of truth

`db.is_idle(name)` is `status == "listening"` (`src/db/instances.rs:1176-1182`) **V**. The status is written by hooks:
- `listening` from **Stop** (PTY branch, `src/hooks/claude.rs:1805-1808`), from **Notification `idle_prompt`** (`:1966-1970`), and from SessionStart (`:1196-1202`);
- `active` from the delivery ack and from UserPromptSubmit (`:1900, 1917`; `src/hooks/common.rs:336-345`);
- `blocked` from PermissionRequest and elicitation (`:1922-1946, 1975-1983`).

**Esc gap:** an interrupt fires no Stop hook, so status stays `active`. The PTY loop repairs this when mail is pending and the screen shows the ready pattern with no approval and 10 s of quiet output: it flips the status to `listening` with `pty:recovered` (`src/delivery.rs:1800-1822`) **V**.

### 3.2 Native (default): the mod

`src/claude_mod/hooks/register.js` (136 lines) plus `src/hooks/claude_mod.rs` **V**:

1. **Arming.** `session.start` arms the poll only when all of the following hold: the session is interactive, `HCOM_PROCESS_ID` is set, `HCOM_PTY_MODE=1`, `HCOM_CLAUDE_DELIVERY=native`, and `HCOM_MOD_COMMAND` holds a valid argv array. It then starts `$.clock.every(250, poll)` (`register.js:104-120`).
2. **One probe at a time.** `poll()` sets a `probing` guard and runs `hcom claude-mod-poll --session-id <id>` via `$.process.run` with a 30 s timeout and `cwd:'/'`. The `cwd` dodges ENOENT after the session's directory is deleted (`register.js:14-17, 32-48`).
3. **Poll process.** `poll()` in Rust (`claude_mod.rs:230-278`):
   - validate ownership: the process binding, the session and the validated Claude session owner must all agree (`:420-433`), and only root hooks can set ownership (`:331`);
   - heartbeat;
   - bind a fresh `NotifyServer` on `127.0.0.1:0`, register it as `kind='plugin'`, and **wait up to 10 s** (`WAIT`, `:14`) unless a wake is already due;
   - deregister the port;
   - `finish()` atomically marks the mod healthy and **claims** the wake by storing `attempted_cursor = cursor` (`:373-411`).
   - Claiming requires `wake_due` (`:413-418`):

   ```rust
   !current.submission_failed && db.is_idle(name) && db.has_pending(name)
       && current.attempted_cursor != Some(db.get_cursor(name))
   ```
4. **Submit.** On `{wake:true}` the mod calls `void $.prompt.submit({ text: '<hcom>', asUser: true })` without awaiting it, and switches later polls to `--observe` until the promise settles (`register.js:51-77`). The engine queues the prompt and runs it once the session is idle, never folded into a running turn (primer §2.3) **V**. `asUser:true` means the model reads the bare `<hcom>` without the "The hcom plugin sent a message" frame, while hooks still see origin `plugin` (d.ts:9410-9424) **V**.
5. **Body.** The queued `<hcom>` prompt fires `UserPromptSubmit`, and the classic hook delivers the batch (§2.3). The mod never carries a body: "The binary owns identity, the queue, and acknowledgement. This module only wakes Claude." (`register.js:1`) **V**.
6. **Acknowledgement of the wake.** The next `load()` sees the cursor moved, logs `claude.mod_acknowledged`, and clears `attempted_cursor` (`claude_mod.rs:346-362`). A wake for an unchanged cursor is never claimed twice, and the claim survives module reloads because it is kept in the DB `kv`, not in `$.state` (`:61-62`) **V**.
7. **Stale-session guard.** A `prompt.submit` hook drops an hcom `<hcom>` whose submitting session differs from the current `$.session.id()`, for example after `/clear` or `/resume` (`register.js:122-129`) **V**. `session.end` bumps a generation counter and cancels the timer unless the reason is clear/resume (`:131-135`) **V**.
8. **Failure reporting.** `--event submitted|submission-failed|poll-failed` reports go back through the same binary (`register.js:19-30, 60-89`; `claude_mod.rs:282-324`). Every failed poll backs off 20 ticks, about 5 s. From the third consecutive failure on, each one is reported as `poll-failed`, and the UI log fires once, at the third (`register.js:8-13, 78-89`) **V**. `report_health` writes the status context `tui:mod-failed|mod-unavailable|mod-unacknowledged|mod-waiting`. Every message ends with the manual opt-out: "messages stay queued until a prompt is typed in Claude; to use inject delivery: hcom config claude_delivery inject, then hcom kill/r" (`claude_mod.rs:28-34, 103-195`). There is **no automatic fallback to PTY typing** **V**.
9. **Idle-edge forwarding.** The PTY proxy's loop checks every 1 s. On a busy→idle edge with mail pending, it pings the mod's `plugin` port so the 10 s wait ends at once (`src/delivery.rs:1759-1765, 1823-1830`) **V**.

**Senders** call `notify::wake_all` after every `hcom send` (`src/commands/send.rs:482`). That is a connect-and-close to **every** registered instance-loop port on the machine (`pty`, `hook`, `listen`, `events-wait`, `plugin`, ...; never `inject`), with a 50 ms connect timeout and at most 32 workers (`src/notify/wake.rs:18-28, 49-73`; `src/notify/mod.rs:63-69`) **V**.

**Draft safety (native).** The mod reads neither `$.prompt.read()` nor `prompt.edit` **V** (absence in `register.js`). The manifest claims it wakes "without changing the input draft" (`src/claude_mod/.claude-plugin/plugin.json`) **V**. This is plausible, because `$.prompt.submit` queues a separate prompt rather than writing the composer (I, from primer §2.3). The cost is that **a wake can start a turn while the human is mid-sentence in the box**. hcom does not wait for the human, unlike our 120 s `DRAFT_WAIT_MS` hold (primer §9) (I).

**Interrupt.** After Esc, `$.prompt.submit` resolves `{drop: 'the session was stopped…'}` (primer §2.3). The mod reports `submission-failed`, which sets `submission_failed = true` with the error text. `wake_due` stays false until a hook advances the cursor, typically when the human's next prompt runs UserPromptSubmit (`claude_mod.rs:297-313, 346-362, 413-418`) **V**/I. This honours the interrupt. The status meanwhile shows `tui:mod-failed`.

**Latency.**
- Sender to wake: one TCP connect (≤50 ms), poll-process exit and JSON parse, `$.prompt.submit`, the engine starting the turn, then one more `hcom userpromptsubmit` process. That totals well under a second when idle (I; no measurement in the repo found).
- After Stop: ≤1 s (the PTY loop period) (I, from `src/delivery.rs:1764`).
- After Esc: about 10 s of output quiet before recovery, plus the drop behaviour above (I).

### 3.3 Inject mode (opt-in): PTY typing

The PTY delivery thread waits on its notify port, then evaluates `evaluate_gate` (`src/delivery.rs:1008-1089`) **V**. For Claude the gate config is `require_idle`, `require_prompt_empty`, `block_on_user_activity` and `block_on_approval`, with `require_ready_prompt=false` because "status bar hides in accept-edits mode" (`src/integration_spec.rs:1356-1366`) **V**. The checks run in this order:

1. DB status is `listening`;
2. no approval (`blocked`) is pending;
3. no keystroke within **500 ms**: `USER_ACTIVITY_COOLDOWN_MS` (`src/pty/shared.rs:35, 554`). The "3 s for Claude" in the doc comment at `src/delivery.rs:679` is stale. 3000 appears only in tests **V**;
4. no Claude navigation overlay is focused: the subagent navigator, or the session switcher detected by its placeholder `describe a task for a new session` (`src/delivery.rs:1035-1046`; `src/pty/screen.rs:626, 903-906`);
5. at least 1.5 s since an observed submit (`SUBMIT_SETTLE_COOLDOWN_MS`, `src/delivery.rs:930, 1052-1061`);
6. the input box is empty. `get_claude_input_box` finds the bottom-most `❯`/`>` row between `─` rules and classifies the text after the prompt. Mostly-**dim** cells count as placeholder, which means empty. Normal-intensity text counts as a draft. A multi-row draft is detected from rows between the prompt and a bottom border as wide as the top one (`src/pty/screen.rs:826-898, 688-730`) **V**. Dim prompt suggestions (primer §7) therefore read as empty, so hcom can type over a suggestion where Herdr's plain-text view cannot tell the two apart (I).

It then types the constant `<hcom>` (`WAKE_TRIGGER`, `src/delivery.rs:561, 2067-2072`). The body is never typed **V**. The tag-stripped, `@`-free preview text (`:585-614`) is used only for tools outside the Claude/Codex/Cursor/Kimi/Copilot/Qoder/Pi/OMP list **V**. Before pressing Enter it requires **exclusive** ownership of the box: the box text must equal the injected text exactly, and a box that merely contains it counts as `Mixed`, which is never submitted (`:1333-1364`) **V**. A Claude box holding only a leftover `<hcom>` is submitted with Enter alone (`:1066-1077`). If the cursor has not advanced after verification, Claude **fast-fails** instead of retyping (`:1381-1398`) **V**.

### 3.4 Failure modes (summary)

| Failure | Behaviour | |
|---|---|---|
| CC < 2.1.287, `allowManagedModsOnly`, mods off | `tui:mod-unavailable` ("never connected"). Mail waits for a human prompt or a tool call | V `claude_mod.rs:139-146` |
| Managed `disableSideloadFlags` | Claude exits at start. hcom matches the message and suggests inject mode | V `claude_mod.rs:36-52` |
| Esc interrupt | No Stop, so a 10 s quiet-screen recovery to `listening`, then the wake drops and stays parked until hook progress | V/I §3.2 |
| A menu is open when the wake is queued | `tui:mod-unacknowledged` after 10 s | V `claude_mod.rs:147-157` |
| Host sleep | One poll failure is tolerated, because the timeout counts during suspend | V `register.js:8-10` |
| `/clear`, `/resume` | `session.start` does not re-fire. `poll()` re-reads `$.session.id()` every cycle. A stale-session wake is dropped | V `register.js:42-43, 122-129` |
| Folder not trusted | `--settings` hooks do not run until the user accepts | V `claude.rs:2836-2839` |

## 4. Setup and installation footprint on Claude

- **Nothing persistent in Claude's config.** Everything is passed per run **V**:
  - `--settings <HCOM_DIR>/integrations/claude/<digest>/settings.json`. The file is the caller's last `--settings` merged with hcom's hooks, `env` (`HCOM_CLAUDE_DELIVERY`, `HCOM_MOD_COMMAND`, `CLAUDE_CODE_STOP_HOOK_BLOCK_CAP`, and the pinned `HCOM_PROCESS_ID`/`HCOM_DIR`/… so background-daemon handoff keeps identity) and optional `permissions.allow` for hcom commands (`src/hooks/claude.rs:2796-2946`);
  - `--plugin-dir <HCOM_DIR>/integrations/claude-mod/<digest>/` (`claude_mod.rs:197-215`).
  - The files are content-addressed and write-once, kept in mode-0700 directories with 0600 files, because merged caller settings may hold secrets (`src/hooks/runtime.rs:436-470, 555-563`) **V**.
- **Exception: legacy cleanup.** Every launch calls `cleanup_legacy_per_run`. It rewrites `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR/settings.json`, plus a legacy `<HCOM_DIR parent>/.claude/settings.json`) to strip hooks that older hcom versions wrote, when any are found. It is atomic and guarded by `hook_cleanup_allowed` (`src/hooks/claude.rs:2950-2961, 3243-3265`) **V**. So hcom does write the real user config, but only to remove its own old entries.
- Claude's environment gets `HCOM_*` variables. The Bash tool's `CLAUDE_CODE_MESSAGING_SOCKET`/`TOKEN` and `CLAUDE_CODE_CHILD_SESSION` are listed as instance-state env that hcom clears for child launches (`src/integration_spec.rs:1338-1344`) **V**. hcom does not use the messaging socket for delivery (I, from grep: it appears only there).
- Wrapping: interactive Claude runs under `hcom claude-pty`, a PTY proxy with VT100 emulation, even in native mode. The proxy supplies the idle-edge forwarding, Esc recovery and health reporting **V** (overview.md).

## 5. Token cost

**Per message (I, from the formats above):**
- frame overhead: about 12-20 tokens (`<hcom>[intent:thread #id] sender → recipient: ` plus `</hcom>`), plus ` | ` per extra item;
- the body;
- any configured `hints` on every delivery, and a one-time tip;
- if the wake started an idle turn: the `<hcom>` user prompt (about 3 tokens), the hook-name system-reminder wrapper, and the turn itself;
- the `systemMessage` copy is display-only and costs no model tokens (primer §1.2).

**Per quiet turn: zero model tokens.** PostToolUse and UserPromptSubmit print nothing when nothing is pending (`src/hooks/claude.rs:1700, 1917-1918`) **V**.

Process cost is not zero (I, from the code):
- one `hcom post` process (SQLite open plus a scan of unread events since the cursor) after **every** tool call;
- one `hcom claude-mod-poll` process roughly every 10 s per session, idle or busy, each doing several `with_write_scope` DB writes;
- one extra poll cycle in **every** Claude session on the machine for each `hcom send`, because `wake_all` pings every port.

**Per session:** the bootstrap, about 700 tokens (README.md:231), plus the `DELIVERY_AUTO` rules for hcom-launched sessions (`src/bootstrap.rs:207-216, 608-611`) **V**.

## 6. Limitations and bugs visible in source

1. **A batch over 10 k chars is acked but not read.** There is no per-delivery byte cap (§2.7), while Claude moves `additionalContext` over 10,000 chars to a file the model is not told to read. 50 messages, or one large body, cross that easily (I).
2. **Tips are lost on the PostToolUse and UserPromptSubmit paths.**
   - `prepare_raw_messages` formats the batch once with the tip checker, and `mark()` writes the tip as seen at once (`src/hooks/common.rs:167-180, 362`; `src/messages.rs:623-648`).
   - Both hook paths then format **again** for `additionalContext` (`src/hooks/claude.rs:1726-1727, 1878-1879`), and by then the tip is already marked.
   - Only the Stop path uses `prepared.formatted` (`src/hooks/common.rs:569-572`).
   - The tip is also marked before the ack commits.
   - Net effect: interactive Claude probably never sees thread or intent tips (I; not executed).
3. **`suppressOriginalPrompt` is in the wrong place.** It is emitted at the top level, while the types expect it in `hookSpecificOutput` (§2.3). Blocked bare wakes are probably put back into the composer. That would explain the "leftover wake" workaround (I).
4. **Freeze summary acks before display and is unbounded.** `end_task` moves the cursor before stdout and without the `created_at` fence. The summary has no 50-message cap and no `<hcom>` wrapper (§2.5) **V**/I.
5. **No draft hold in native mode.** An idle wake starts a turn while the human is typing (§3.2) (I).
6. **Every message wakes.** There is no priority or threshold, and `send` has no silent option (overview.md) **V**. A broadcast to N idle Claudes starts N turns.
7. **Thundering herd.** `wake_all` on every send restarts every mod poll and Stop poll on the host (§3.2) **V**/I.
8. **Duplicate window.** Concurrent hooks can deliver the same batch twice before either commits (§2.8) (I).
9. **Unauthenticated localhost TCP.** Any local process can connect to a notify port and force a poll cycle (harmless: it only causes a check). In inject mode it can also reach the inject port, which writes text into the PTY (overview.md; `src/pty/inject.rs`) **V**/I.
10. **24 h Stop hook in `-p`.** A headless run's Stop hook holds the process for up to 86,400 s waiting for mail, by design (`src/config.rs:238`) **V**. Whether `STOP_HOOK_BLOCK_CAP=0` really lifts the cap is unknown (§2.1).
11. **No automatic fallback.** A broken mod leaves mail parked until a human prompt. The fix is manual (`hcom config claude_delivery inject` + restart) **V**.
12. **Scan cost.** `get_unread_messages` reads every `type='message'` event after the cursor and filters in Rust (`src/db/events.rs:125-143`). The cursor moves only on a delivery to this instance, so traffic addressed to others is rescanned on every tool call (I).

## 7. Clever ideas worth stealing

1. **Wake claim keyed on the cursor** (`attempted_cursor`, `claude_mod.rs:61-62, 395-399, 413-418`). The mod never re-submits a wake until a hook proves progress by moving the cursor, and the claim lives in durable storage, so it survives mod hot-reload. This makes "one wake in flight" hold across reloads. Our equivalent is in-memory mod state (I).
2. **The `<hcom>` prompt is refused when it has nothing to deliver.** `UserPromptSubmit` blocks a bare wake that found nothing pending or could not be attributed, and shows the human why (`src/hooks/claude.rs:186-215, 1909-1916`). The model never spends a turn on an empty wake. We could apply the same idea to `wake up`, if our mod channel ever submits one without rows (I). Use the documented `hookSpecificOutput.suppressOriginalPrompt`.
3. **Mod `prompt.submit` guard against stale sessions** (`register.js:122-129`). Cheap protection against `/clear` and `/resume` races.
4. **Health surfaced as status, with the remedy in it** (`tui:mod-unavailable|failed|unacknowledged|waiting`, `claude_mod.rs:103-195`). It distinguishes "never connected", "stopped polling", "wake queued but not accepted after 10 s" and "submission failed".
5. **Match managed-policy refusals on Claude's own startup text** (`disableSideloadFlags`, `claude_mod.rs:36-52`) instead of reading managed settings files.
6. **Dim-attribute draft detection** (`src/pty/screen.rs:688-730, 826-898`). It tells a prompt suggestion or placeholder from a real draft, and it adds the exclusive-ownership check before Enter. This would solve our "Claude composer text is always Unsafe" fallback (primer §9), but only if Herdr can expose cell attributes (unknown).
7. **Esc recovery.** Esc fires no Stop, so hcom treats "ready prompt + 10 s of no output + mail pending" as idle (`src/delivery.rs:1800-1822`).
8. **Per-run `--settings` + `--plugin-dir`, content-addressed, 0600.** This leaves no persistent config edits and supports concurrent launches (`src/hooks/runtime.rs:436-470`). Caveat: Claude keeps only the last `--settings`, so hcom has to merge the caller's (`src/hooks/claude.rs:2836-2847`).
9. **Pin identity env in settings `env`** for sessions moved to Claude's background daemon, whose process env belongs to another launch (`src/hooks/claude.rs:2897-2946`).
10. **Stop-hook orphan check.** It refuses to hand mail to a dead parent, detected by POLLERR/POLLNVAL on stdin (`src/hooks/common.rs:556-562`).

## 8. Contrast with herdr-threads (brief)

Our side is taken from primer §9; there is a separate report for it.

| | hcom | herdr-threads |
|---|---|---|
| Body path | Classic hooks only (`PostToolUse`/`UserPromptSubmit` `additionalContext`) | Mod: `tool.call` `context` (busy) and `$.session.append` rows (idle) |
| Wake | Mod `$.prompt.submit('<hcom>', asUser)` from a 10 s long-poll process respawned in a loop | Mod `$.prompt.submit('wake up')` (plugin attribution) from one long-lived `watch` stream |
| Wake policy | Every message | Priority ≤ T (default T = 0, so a default send is silent) |
| Draft guard (mod) | None | 120 s `$.prompt.read` hold |
| Without a mod | Mail waits, or the user opts into inject typing | Herdr native typing wake |
| Size bound | 50 msgs, no byte cap, so Claude's 10 k cap applies silently | 4096-byte hook cap; 8 KiB body truncation with a `body <id>` continuation; 32-item / 64 KiB pages |
| Owns the PTY | Yes (`hcom claude-pty`) | No (Herdr pane) |

## Verification

Adversarial pass on 2026-10-23 against clone HEAD `de33e31a0cdb` (read only), the GitHub API, the primer and the 2.1.308 d.ts.

- Checked and confirmed: stars, forks, license and the v0.7.31 release date (API, 2026-10-23); the hook table and timeouts (`claude.rs:2699-2717`), `CLAUDE_CODE_STOP_HOOK_BLOCK_CAP="0"`, print sessions skipping the mod, exec-form hook entries, the inert-without-`HCOM_PROCESS_ID` exit, prepare-then-commit-after-flush, the bare-wake block and its top-level `suppressOriginalPrompt` (d.ts:1336-1339, 1397 place it in `hookSpecificOutput`), the PostToolUse/UserPromptSubmit/Stop handlers, the poll loop, freeze summary ordering, subagent Stop and PostToolUse, the full `register.js` and the `claude_mod.rs` state machine (`wake_due`, `finish`, `load`, health contexts, opt-out text, sideload hint), the PTY gate order and constants (500 ms, 1.5 s, exclusive ownership, Claude fast-fail), dim-cell draft detection, Esc recovery and idle-edge forwarding, `wake_all`, per-run publish (content-addressed, 0600), legacy cleanup of `~/.claude/settings.json`, the 50-message and 1 MiB limits, the message prefix and batch format, the tip double-format path, and the `asUser` semantics (d.ts:9412-9424).
- Fixed: the overview cross-reference wording (§2.3, "Claude's handling of Codex" read as Claude's behaviour); the orphan check also requires a non-background instance (§2.4); the mod backs off after every failed poll, not only after three (§3.2 item 8); the cited `src/delivery.rs:580-654` range does not strip C0/ESC, because for Claude the constant `<hcom>` is typed and the preview path applies only to other tools (§3.3); the `DELIVERY_AUTO` citation pointed at `COPILOT_DELIVERY` (§5, now `bootstrap.rs:207-216`); the `wake_all` target list (§3.2); the tip citations (§6.2).
- Added: Task/Agent `PostToolUse` bypasses the normal delivery path (§2.5).
- Left as INFERRED or unknown, unchanged: whether `STOP_HOOK_BLOCK_CAP=0` means unlimited, whether a top-level `suppressOriginalPrompt` is ignored, whether a mod wake starts a turn over a human draft, the duplicate-delivery window, and latency figures. None were executed.
