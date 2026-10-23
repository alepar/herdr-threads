# hcom x Hermes Agent: message injection and wake-up

- Upstream: https://github.com/aannoo/hcom (MIT, 597 stars / 94 forks, last push 2026-10-21; GitHub API 2026-10-23). Clone `alts/hcom` at `de33e31a0cdb` (2026-10-21, v0.7.31). It was read only; nothing in it was run.
- Hermes reference: the primer `../../primers/hermes.md`, plus the primer's shallow clone of `https://github.com/NousResearch/hermes-agent` (`alts/hermes-agent`) at `main` `f3b5b980d615` (2026-10-23 21:27 +0200), read only. It is cited as `hermes:<path>:<line>`. Hermes v0.22.1 was published 2026-10-14T23:59Z (`gh release view v0.22.1`).
- Citations without a prefix are `path:line` in the hcom clone. **V** = VERIFIED (read in source or docs), **I** = INFERRED, **unknown** = not found.

## Support status

**Native, first-class, recent.** "Added Hermes Agent, `hcom hermes`: messaging, status, resume, and transcripts" and "Hermes gateway chats as hcom agents" both arrived in hcom 0.7.30 on 2026-10-15 (CHANGELOG.md:59-63) **V**. README lists Hermes as `automatic` delivery (README.md:191) **V**. The integration has four parts:

| Part | Where | |
|---|---|---|
| Python profile plugin (`plugin.yaml` + `__init__.py` + `hcom.json`) that does all delivery | src/hermes_plugin/__init__.py (1532 lines), src/hermes_plugin/plugin.yaml | V |
| Installer, enabler, stale-plugin refresher | src/hooks/hermes.rs | V |
| Argv RPC shared with Pi/OMP: `hcom hermes-start / -status / -read / -beforetool / -stop`, plus `hermes-bridge` for gateway chats | src/hooks/pi_family.rs:1-3, 38-41, 155-216; src/hooks/plugin_rpc.rs:126-216; src/integration_spec.rs:377-384 | V |
| Integration spec: PTY-wrapped (`uses_pty_default: true`), but the idle/ready-prompt/empty-prompt/user-activity gates are off because "the Hermes plugin owns delivery" (`block_on_approval`, `launch_requires_ready` and `launch_ready_on_plugin_bind` stay true) | src/integration_spec.rs:2042-2093; src/pty/screen.rs:785; src/delivery.rs:1669-1704 | V |

There are two participant kinds (src/hermes_plugin/__init__.py:1-21) **V**:
1. **Local agent** (`hcom hermes [...]`). hcom runs `hermes chat ...` inside its PTY proxy with `HCOM_LAUNCHED=1`, `HCOM_PROCESS_ID` and `HCOM_TOOL=hermes`. The plugin's `Host` serves exactly one identity (`Participant`, kind `cli` or `tui`).
2. **Bridge** (a messaging-gateway chat that ran `/hcom start`). The plugin's `GatewayHost` serves N `Bridge` participants and delivers through Hermes' public `ctx.inject_message`.

Tests: 71 unit tests against fakes of Hermes' manager and CLI (tests/hermes_plugin.py). One opt-in real-model lifecycle test covers launch, idle delivery, mid-turn delivery, transcript, kill and resume (tests/real_tool_hermes.rs:82-193) **V**. Neither pins or records a Hermes version **V** (absence).

### Compatibility with the current Hermes (v0.22.1 and later): the local path is probably broken

This is the most important finding for the comparison.

- The local `cli` participant binds only once `manager._cli_ref` is set: `watch_cli` polls `self.cli()` (`getattr(manager, "_cli_ref", None)`) for up to `CLI_READY_TIMEOUT = 120` s and then gives up (src/hermes_plugin/__init__.py:78, 366-367, 455-471) **V**. Idle hand-off for `cli` also needs `cli._pending_input` (:572-578) **V**.
- In Hermes `main` today, nothing assigns `_cli_ref`. Its only assignment is the initializer `self._cli_ref = None  # Set by CLI after plugin discovery` (hermes:hermes_cli/plugins.py:1127; repo-wide grep for `_cli_ref *=`). The classic CLI says explicitly `# No PluginManager._cli_ref: plugins load in the gateway, not here (PLG-7).` (hermes:hermes_cli/cli_tui_mixin.py:1834), and so does the one-shot path (hermes:hermes_cli/cli_single_query.py:483-484). A Hermes test asserts that "No plugin code runs in the classic CLI process on a gateway session" (hermes:tests/hermes_cli/test_gateway_chat_plugin_inject.py:4-5, 26-29) **V**.
- So in the process hcom launched, the plugin is never even loaded (PLG-7). It loads only in the profile's shared gateway **V** (primer section 1). What it does there depends on that gateway's environment:
  - **Gateway started by an operator or service** (no `HCOM_*`): `launched_by_hcom(env)` is false, so the plugin builds a `GatewayHost`, which serves only bridges (src/hermes_plugin/__init__.py:136-141, 1513-1518) **V**. The local agent never binds, and launch readiness (`launch_ready_on_plugin_bind`, a `kind='plugin'` notify endpoint written by `hermes-start`) never arrives (src/delivery.rs:1106-1115; src/hooks/pi_family.rs:155-157) **I**.
  - **Gateway auto-spawned by the first `hcom hermes` client**: Hermes' `spawn_unmanaged_gateway` builds the child env from `served_profile_child_env(base=dict(os.environ), ...)` (which only strips launch-profile residue and credentials for a routed home, hermes:tools/environments/local.py:414-454) and then pops a fixed `DAEMON_STRIPPED_ENV` list and the session-context vars (hermes:hermes_cli/gateway_runtime_start.py:18-32, 57-71). `HCOM_LAUNCHED`, `HCOM_PROCESS_ID` and `HCOM_TOOL` are not on that list **V**. The gateway's plugin then becomes an hcom `Host` that claims *the first launch's* process id (`claim_host`, :127-133) **I**. With `_cli_ref` unset, `on_session_reset` switches it to kind `tui` and binds whatever session resets through it (:900-906). `bind()` re-runs `hermes-start` for every new session id it sees (:406-453), so other panes' sessions in the same gateway can rebind the first agent's identity, and a second `hcom hermes` launch's `HCOM_PROCESS_ID` never reaches any plugin **I**.
- hcom 0.7.30 shipped hours after Hermes v0.22.1. Its plugin still targets the in-process classic CLI, although it already knows the TUI/gateway injectors (`has_tui_message_injector`, `inject_tui_message`, and `has_gateway_message_injector` for bridges, src/hermes_plugin/__init__.py:579-584, 1159-1160) that current Hermes publishes (hermes:hermes_cli/plugins.py:1218-1235; hermes:tui_gateway/plugin_inject.py:24-86) **V**. No live run was possible (Hermes is not installed locally, primer header). **Status: native on Hermes up to the in-process CLI (0.21.x / 0.22.0); local agents INFERRED broken on 0.22.1+; bridges INFERRED working** (every gateway internal the bridge code calls still exists: `_plugin_origin_source`, `_plugin_injection_authorized` in hermes:gateway/run_plugin_injection.py, `_delivery_adapter_for` in hermes:gateway/authz_mixin.py, `reload_gateway_plugins(home, *, profile_home=)` at hermes:gateway/control_socket.py:546) **V** for existence.

Everything below describes the mechanism as hcom designed it. It works as described wherever the plugin runs in the agent's own process.

## Injection mechanism

### Three delivery points, one batch outstanding at a time

| Point | Hermes surface | hcom code | When it fires | What the model sees |
|---|---|---|---|---|
| **Mid-turn** | plugin hook `transform_tool_result` (the first `str` return replaces the tool result) | `Host.transform_tool_result` (src/hermes_plugin/__init__.py:931-936) | after every tool call of the bound main session, **only if `p.pending`** (a notify wake arrived) and the result is a `str` | `f"{result}\n\n{text}"`: the batch appended to the tool output |
| **Turn start** | plugin hook `pre_llm_call` → `{"context": ...}` (Hermes appends it to the current user message) | `Host.pre_llm_call` → `Participant.turn_start` (:915-923, 698-714) | once per turn, before the tool loop, for the main session only (`is_main`) | the bootstrap (first turn of each bound session) and/or a fresh batch, joined by `\n\n` |
| **Idle** | local: `cli._pending_input.put(text)` (classic CLI's between-turns queue) or `manager.inject_tui_message(session_key=..., content=...)` (Ink TUI). Bridges: `ctx.inject_message(text, origin=... or session_key=...)` | `Participant.hand_off` (:568-584); `Bridge.hand_off` (:1119-1131) | from the plugin's own `hcom-deliver` thread when the participant is not busy (see Wake) | the batch as a **user-role turn** |

The mid-turn code path, verbatim (src/hermes_plugin/__init__.py:931-936) **V**:

```python
def transform_tool_result(self, result=None, session_id=None, **_):
    p = self.route(session_id)
    if p is None or not p.pending or not isinstance(result, str):
        return None
    text = p.take_batch("tool_result")
    return f"{result}\n\n{text}" if text else None
```

Hermes calls it from `_apply_transform_tool_result_hook` after `post_tool_call` and before the result enters context, with `next((r for r in hook_results if isinstance(r, str)), result)`: **first string wins**, and every plugin's hook runs (hermes:model_tools.py:831-848) **V**.

Why the hcom plugin bypasses `ctx.inject_message` for the local classic CLI (src/hermes_plugin/__init__.py:14-17) **V**: "`ctx.inject_message` picks the interrupt queue whenever a turn is running, and a turn can start between any busy check and the call." This is confirmed by hermes:hermes_cli/plugins_injection.py:21-22: `queue_ = cli._interrupt_queue if getattr(cli, "_agent_running", False) else cli._pending_input` **V**. Putting the batch directly on `_pending_input` means it "Never interrupts" (:569-571) **V**.

### RPC and payload

- Read: `hcom hermes-read --name N`, which returns `{"messages":[{from,message,intent?,thread?,event_id,timestamp,...}], "has_more": bool}` (src/hooks/plugin_rpc.rs:146-158; src/hooks/common.rs:130-150) **V**. Unread means `event_id > last_event_id` on the instance cursor (overview, src/db/events.rs:103-143) **V**. The plugin rejects a batch whose `event_id`s are not strictly increasing ints (`_valid_batch`, src/hermes_plugin/__init__.py:193-202) **V**.
- Ack: `hcom hermes-read --name N --ack --up-to <last event_id>`, which calls `advance_instance_cursor` and answers `{"acked_to": id}`. The plugin accepts the ack only if `acked_to` echoes the id (src/hooks/plugin_rpc.rs:132-144; src/hermes_plugin/__init__.py:532-536) **V**.
- Every RPC is a fresh `subprocess.run(["hcom", ...], timeout=5.0)` with JSON on stdout. Errors come back as `{"error":...}` on exit 0. An error containing "is not attached to this process" detaches the participant for good (src/hermes_plugin/__init__.py:47, 764-800; src/hooks/plugin_rpc.rs:48-62) **V**. Attachment is checked as `HCOM_PROCESS_ID` → process binding == `--name` (`require_attached`, src/hooks/plugin_rpc.rs:46-62, applied by the dispatcher at src/hooks/pi_family.rs:185) **V**.

### Framing (src/hermes_plugin/__init__.py:144-157) **V**

```text
<hcom>[request:review #812] luna -> nova: please look at PR 4</hcom>
<hcom>[2 new messages] | [new message #813] luna -> nova: a | [thread:x #814] api-kai -> nova: b</hcom>
```

The prefix is `[intent:thread #id]`, `[intent #id]`, `[thread:T #id]` or `[new message #id]`. The plugin formats in Python ("Same shape as the Pi plugin's") and **does not** use the Rust `messages::format_messages_json`, so the per-instance and config `hints` and the one-time tips that the overview flags for Claude and Codex (src/hooks/common.rs:269-285) are **not** appended on Hermes **V** (no `hints` anywhere in the plugin). Bridges with `visibility off` get one extra fixed sentence, `UNSEEN_HINT` (~150 bytes) (:168-169, 1126-1127) **V**.

### Bounds

- At most 50 messages per batch (`limit_delivery_messages`, `MAX_MESSAGES_PER_DELIVERY = 50`, src/hooks/common.rs:261-267; src/shared/constants.rs:19), with `has_more` re-arming delivery **V**. Each message can be up to 1 MiB (`MAX_MESSAGE_SIZE`, src/shared/constants.rs:22) **V**. There is no per-batch byte cap and no truncation in the plugin **V** (absence).
- Hermes' own limits apply after hcom. `pre_llm_call` context above `hooks.output_spill.max_chars` (default 10,000) is spilled to a file with a 500/500 preview (primer 2.2) **V**. For a large tool result carrying a batch, Hermes' handling is **unknown**.

### Dedupe and ack: an admission state machine (the most careful part)

Records move only forward: `handed_off -> offered -> admitted -> acked` (src/hermes_plugin/__init__.py:23-28, 223-239) **V**.

1. **Read without ack.** `begin()` claims the single delivery slot only if `pending`, not `delivering`, and no `out` record exists (:475-484). The plugin therefore never reads a second batch while one is unsettled (:521-522) **V**.
2. **Offered** (hook output: `take_batch` from `pre_llm_call` or `transform_tool_result`) or **handed_off** (idle queue) (:548-566, 586-631) **V**.
3. **Admission evidence**, never the hook's own return value: "Confirm the final context, never the transform's proposed output" (:332-337) **V**.
   - Hand-off → offered: the next `pre_llm_call` user message must *carry* the batch text. `carries()` accepts the exact string or a whole `\n\n`-separated part, because "the TUI and the gateway merge prompts queued behind a busy turn as `earlier + "\n\n" + later`". It also handles multimodal lists (:172-190, 664-671) **V**.
   - Offered → admitted: `confirm()` looks for every message's formatted body in `post_llm_call`'s `conversation_history` user/tool rows (preferring `api_content`, "the exact accepted bytes"), or in Hermes' `state.db`. That is a read-only SQLite query over `messages` joined to `sessions`, using `instr(content|api_content, '#<id>]')` and then a substring check of the body (:299-361, 938-941) **V**.
4. **Ack** only after admission. A failed ack keeps the record, and the 5 s reconcile tick retries it ("Text the model has seen is never withheld because an ack reply went missing", :25-28, 519-546, 673-681) **V**.
5. **Release and redeliver** when Hermes dropped the batch. An unadmitted record is released after `ADMIT_GRACE = 10` s (local CLI or offered), `None` = until session end (TUI hand-off), or `BRIDGE_ADMIT_GRACE = 300` s (bridges). While the classic CLI still holds the text in `_pending_input.queue`, the grace timer restarts (`held`) (:50-52, 62-64, 274-277, 633-662) **V**. The redelivery waits for the next wake or poll, not a tight loop (:659-661) **V**.
6. **Fencing.** A `gen` counter bumps on every bind, session finalize (`on_session_finalize`, not the per-turn `on_session_end`), detach and unload (:279-289, 726-730, 849-856), and any read that returns under an old `gen` is dropped (:23-25, 496-517) **V**. On `/new` the CLI keeps a held batch as the new session's first turn (it is retargeted), but never across an identity change, which "would ack another mailbox" (:437-446, 529-531) **V**.
7. **Bridges after a gateway restart**: `already_run` drops the messages already found in the chat's persisted rows and keeps the original `ack_id` (:1090-1102) **V**.

Delivery is therefore at-least-once into context, with dedupe by content evidence. A duplicate is possible only when the evidence lookup misses text the model did see (I; see Limitations).

## Wake mechanism

**The wake and the body are one act.** The idle batch itself becomes the next user turn. Nothing is typed into the terminal, and the PTY gates are disabled (src/integration_spec.rs:2075-2084; src/pty/screen.rs:785) **V**. The PTY delivery thread runs in "plugin mode" for Hermes and only maintains the title, status and launch outcome (src/delivery.rs:1669-1720) **V**.

### Signal path

1. The plugin binds a TCP listener on `127.0.0.1:0` that accepts and closes connections. Each connection sets `pending` on every participant and fires `self.wake` (src/hermes_plugin/__init__.py:804-839) **V**. The port is registered with `hermes-start --notify-port`, stored as a `kind='plugin'` notify endpoint, and the PTY loop is woken (src/hooks/pi_family.rs:48-67, 151-153) **V**. `hcom send` does not target the recipient: it calls `wake_all`, which connects to every registered wake endpoint system-wide (src/commands/send.rs:481-482; src/notify/wake.rs:49-75) **V**. So every send makes every bound Hermes participant set `pending` and spend one `hermes-read` process on a usually empty read (I, from the code). This port is a doorbell only: it carries no data, so a stranger connecting can only cause an extra read **V**.
2. The `hcom-deliver` thread waits on `self.wake`, with `RECONCILE = 5` s, or a 0.1 s step for `END_RETRY_FOR = 2` s after a turn ends, then runs `tick()` (:53-61, 866-878) **V**.
3. `tick()` retries the ack, releases dropped hand-offs and restores `listening`. While not busy it also forces `pending` every `PENDING_POLL = 60` s (`FALLBACK_PENDING_POLL = 5` s when no notify port exists) and calls `deliver_idle()` (:673-694) **V**.

### Idle versus busy

- `is_busy()` = the plugin's own `busy` flag (set in `turn_start` from `pre_llm_call`, cleared in `turn_end` from `on_session_end`, which fires after every turn) **or** `cli._agent_running` (:369-370, 698-724) **V**.
- `deliver_idle()` checks busy before the read, then **re-checks under the lock right before `hand_off`**, and abandons the record if a turn started in between (:592-611) **V**. The 2 s fast retry covers "The CLI's running flag stays set for a moment after on_session_end" (:58-61) **V**.
- Approval prompts: `on_human_input_request` / `_resolved` write status `blocked:approval` / `active:agent` (:950-962) **V**. A busy agent is never handed a turn, and approval happens mid-turn, so no special gate is needed (I).
- Bridges: busy is only the chat's own turn, because "Hermes queues turns for a busy chat itself" (:995-997, 1051-1052) **V**.

### Draft safety

No keystrokes reach the composer. The classic CLI's `_pending_input` and the TUI's per-session `queued_prompt` are queues separate from the prompt buffer. The TUI injector "only queues" when the session is busy ("a notice must not cancel in-flight work") and drains when idle (hermes:tui_gateway/plugin_inject.py:24-60) **V**. A human's half-typed draft is therefore not clobbered (I: not observed; the queue and buffer are separate objects). What the human *sees* in the classic CLI when a `_pending_input` item starts a turn is unknown. The real-tool test only asserts that the agent replies (tests/real_tool_hermes.rs:130-137) **V**.

### Latency (all I, nothing measured)

Wake path: TCP connect, then thread wake, then one `hcom hermes-read` process start (tens of ms for the Rust binary), then queue put, then Hermes' idle loop picks it up. Expect well under a second when the notify port works. Without the port, up to 5 s. After a rejected hand-off, up to `PENDING_POLL` = 60 s, because the reject path sets `last_poll` and does not re-arm `pending` (src/hermes_plugin/__init__.py:619-626, 688-693) **V** for the code.

### Failure modes

| Failure | Behaviour | |
|---|---|---|
| Hermes ≥ 0.22.1 local session (plugin not in the client; `_cli_ref` never set) | no bind, no delivery, launch readiness never arrives; or misbinding in an inherited-env gateway (see Support status) | I |
| `plugins.isolation: host` | the installer refuses ("hcom needs plugins.isolation: in_process"), and the plugin goes inert with an ERROR log line, because the PID claim and the env scrub would not reach terminal children (src/hooks/hermes.rs:444-478; src/hermes_plugin/__init__.py:1500-1506) | V |
| TUI injector refuses (session `lazy`, `_closing`, `_finalized`, no `history_lock`, or no matching `session_key`) | `inject_rejected` is logged and delivery retries on the next wake or within 60 s (hermes:tui_gateway/plugin_inject.py:33-52; src/hermes_plugin/__init__.py:619-626) | V |
| TUI hand-off passes `session_key=rec.target`, the hook's `session_id` (:583-584); the injector matches `session["session_key"]` | works only if the two are equal for local sessions (unknown; the primer says local keys are `ses_…`) | unknown |
| Single-query (`-q -Q`) runs | skipped (`plugin.single_query_skip`, :463-465). Quiet and one-shot flags are rejected at launch (src/tools/hermes_preprocessing.rs:6-44) | V |
| Notify port fails to bind | 5 s polling while idle. While busy the tick returns early, so **no mid-turn delivery** happens without a notify wake (:683-694, 933) | V (code) / I (effect) |
| Plugin reload or upgrade in a running gateway | `on_unload(host.stop)` bumps `gen`. A new copy adopts the outstanding bridge batches (`adopt`, :1030-1046). A gateway reporting an old `SOURCE_HASH` has its plugin files rewritten and is asked to `reload_gateway_plugins` (src/hooks/hermes.rs:95-140; src/hermes_plugin/__init__.py:1209-1213, 1258-1274) | V |

## Setup and installation footprint

The plugin is per profile and persistent: "Hermes has no per-run plugin path" (src/hooks/hermes.rs:4-8) **V**.

- Files: `<HERMES_HOME>/plugins/hcom/plugin.yaml`, `__init__.py` (embedded via `include_str!`), and `hcom.json`. `hcom.json` records the `hcom` binary path, `HCOM_DIR`/`HCOM_DEV_ROOT` and the `scrub_vars` list. A `.hcom-config-stamp` holds the `config.yaml` size and mtime (src/hooks/hermes.rs:27-32, 53-87, 281-303, 534-565) **V**. An existing foreign plugin named `hcom` is never overwritten (:546-552, 867-890) **V**.
- Hermes config: written only through Hermes' own CLI, pinned with `HERMES_HOME=<home> hermes -p <profile>` and with `HCOM_LAUNCHED` / `HCOM_PROCESS_ID` removed from the child env. The calls are `config get plugins --json` (an isolation check) and `plugins enable hcom` (→ `plugins.enabled`) (src/hooks/hermes.rs:373-404, 444-468) **V**. Installation happens on the first `hcom hermes` for that profile (`ensure_installed`, src/launcher.rs:1897-1899) or on `hcom hooks add hermes` for the ambient profile **V**.
- The injection grant `plugins.entries.hcom.allow_gateway_injection: true` is written **only when the first chat runs `/hcom start`** ("it is the opt-in"). It is recorded in hcom's KV so that uninstall removes it, and a grant the user set is never removed (src/hooks/hermes.rs:480-523) **V**. Local agents do not need it, since they never call `inject_message` **V**.
- Launch argv: `chat` is prepended unless already present, the first prompt goes through `-q`, and resume uses `--resume <sid>` (src/tools/hermes_preprocessing.rs:91-97; src/integration_spec.rs:2085-2101) **V**.
- Env hygiene inside Hermes: the plugin pops other harnesses' detection vars (`CLAUDECODE`, `CODEX_THREAD_ID`, `GEMINI_CLI`, `OPENCODE`, ...) so Hermes' terminal children "are not mistaken for the agent that started Hermes", and it keeps `HCOM_*` and `HERMES_*` (src/hooks/hermes.rs:34-51, 773-798) **V**. `HCOM_PLUGIN_HOST_PID` makes nested Hermes runs and subcommands inert (src/hermes_plugin/__init__.py:79-81, 127-133) **V**.
- Uninstall: `hcom hooks remove hermes` disconnects every bridge, runs `plugins disable`, revokes hcom's grant and deletes the plugin directory in every profile under the root and the registry. A running gateway keeps the loaded copy until it restarts (src/hooks/hermes.rs:634-699) **V**.

## Token cost

| Item | Cost | |
|---|---|---|
| Bootstrap ("hcom CLI primer") | about 700 tokens (README.md:231), injected once per bound session at its first `pre_llm_call` (`bootstrapped_session`, src/hermes_plugin/__init__.py:702-704). It is re-sent after `/new` and after any rebind to a new session id, including the compression continuation rebind (:919-922). It lives in the user message's `api_content`, so it stays in history (primer 2.2) | V / I (size from README) |
| Per message | envelope `<hcom>` + `[intent:thread #id] a -> b: ` + `</hcom>`: about 35-60 bytes plus names (I, from the format). Multi-message batches add `[N new messages] | ` and ` | ` separators. No hints, no tips | V (format) |
| Mid-turn | the same bytes, appended to a tool result. No extra turn | V |
| Idle wake | the batch **is** the user turn, so there is no wake marker, no `inbox` round trip and no extra model call beyond the turn itself | V |
| Quiet turn | **0 model bytes**: `pre_llm_call` returns `None` when it has neither bootstrap nor batch (:714), and `transform_tool_result` returns `None` unless `pending` (:933). Process cost per turn: `hermes-status active` + `listening` RPCs, plus one `hermes-beforetool` RPC per tool call, all asynchronous on the status thread (:381-402, 880-896). While idle: one `hermes-read` every 60 s, plus one per `hcom send` anywhere on the machine (broadcast wake, see Signal path) | V |
| Every message wakes | there is no priority or threshold (overview, "Wake policy"). Each idle delivery costs a full model turn | V |

## Limitations and bugs visible in the source

1. **Local agents on Hermes ≥ 0.22.1** (INFERRED, high confidence): the dependency on `manager._cli_ref` and `cli._pending_input`, private attributes the current Hermes no longer sets in any process, and on the plugin loading in the launched process, which PLG-7 removed. Details in Support status. A secondary risk: an auto-spawned gateway inherits `HCOM_*`, so one launch's identity can be bound to other sessions in the shared gateway.
2. **Private API dependence by design.** Even before 0.22.1, the CLI path read `_cli_ref`, `_pending_input`, `_agent_running` and `_single_query_mode` (src/hermes_plugin/__init__.py:366-370, 455-466, 572-578). `show_notice` reaches into `gateway.run._gateway_runner_ref()` and private runner methods ("Upstream has no plugin send facade yet", :1276-1311) **V**.
3. **No byte bound.** A batch can carry 50 × 1 MiB into a tool result or a user turn. Admission by substring search can then fail if Hermes spills or truncates the text, and the batch is re-released and redelivered every grace period: a redelivery loop of the same large batch (I: Hermes' tool-result size handling not examined).
4. **First-string-wins conflicts.** Every plugin's `transform_tool_result` runs, and the first `str` wins (hermes:model_tools.py:842-845). If another plugin's string wins, hcom has still marked its batch `offered`. Admission then fails and the batch comes back after 10 s, which is correct, but the "mid-turn" delivery silently becomes a later one (I). Hermes bundles a `security-guidance` plugin that registers this hook (hermes:plugins/security-guidance/) **V** for existence.
5. **Rejected hand-off waits up to 60 s** (Failure modes table).
6. **No mid-turn delivery without the notify port**, because the fallback poll is skipped while busy (Failure modes table).
7. **Injected idle text is a real user-role row** in the transcript and in `state.db`. In TUI and gateway sessions it may be merged with a queued human prompt (`carries` exists for this reason) **V**. Nothing distinguishes it from human input except the `<hcom>` tag (I).
8. **`mid-turn` exists only when the agent calls tools.** A long pure-generation turn gets mail only at the next turn (I, inherent to `transform_tool_result`).
9. The state.db evidence query does `instr()` scans over all `messages` rows of the session per message, every 5 s tick while a record is outstanding (:306-328, 673-678) **V**. Cheap for small sessions, linear in transcript size (I).
10. Every RPC is a new `hcom` process with a 5 s timeout. A stalled hcom can therefore delay `pre_llm_call` by up to 5 s per turn (the hook call is synchronous: `turn_start` → `take_batch` → `rpc`) **V** (code) / I (effect).

## Clever ideas worth stealing

1. **Mid-turn delivery through `transform_tool_result`.** It appends pending items to the next tool result of a busy session. herdr-threads delivers only at `pre_llm_call` (primer 11, flag 3). This satisfies our delivery-contract rule 3 (in-turn delivery) for a busy Hermes without any wake. Cheap: gate it on a local "pending" flag so quiet tool calls cost nothing.
2. **Body-as-wake through a host queue, not keystrokes.** The queue is host-arbitrated and cannot clobber a draft. On current Hermes the supported equivalent is `ctx.inject_message(content, session_key=...)` with `allow_gateway_injection: true` (primer 4; hermes:hermes_cli/plugins_injection.py:36-48), the path hcom uses for bridges. Under our contract the wake must stay content-free (R18), so we would inject a short marker and let `pre_llm_call` deliver, or treat the queued turn as the delivery itself. That is worth an explicit decision.
3. **Ack on admission evidence, not on emission.** hcom acks only after the exact formatted body is found in the accepted history (`post_llm_call` history or the `api_content` sidecar in `state.db`), and keeps one batch outstanding until then. herdr-threads' Hermes `lifecycle_ack` acks on hook return. hcom's model gives "seen by the model" semantics and survives a hook output that Hermes dropped (persist-disabled turns, first-string-wins losses, interrupted turns).
4. **`carries()` for merged queued prompts**: an exact whole-part match on `\n\n` boundaries, with unique `#id` markers so a quote does not count.
5. **Generation fencing** (`gen` bumped on bind, end, detach and unload) to drop stale reads after `/new`, resume or plugin reload, plus **adoption of outstanding records across hot reloads**.
6. **Readiness = plugin bind** (a `kind='plugin'` endpoint), not screen scraping. A dead plugin blocks the launch instead of being declared ready falsely (src/delivery.rs:1106-1115).
7. **Stale-plugin detection**: the loaded copy reports `SOURCE_HASH` on every sync, and hcom rewrites the files and asks the gateway to reload through Hermes' own control socket. This applies directly to herdr-threads' owned Hermes plugin after upgrades.
8. **Env hygiene for nested harnesses**: scrub other harnesses' markers from the Hermes process env, and use a PID claim env var so nested Hermes processes stay inert. Both reduce false caller attribution.
9. **A warning for us.** Hermes' auto-spawned gateway inherits the launching client's env except a fixed strip list (hermes:hermes_cli/gateway_runtime_start.py:18-32, 57-71) **V**, and `HERDR_PANE_ID` is not on it **V**. This supports the "worse" case in primer section 12 flag 1: if the first Hermes in a Herdr pane auto-starts the gateway, every Hermes session's plugin hook child in that gateway would inherit **that pane's** `HERDR_PANE_ID` (I: not observed live; an operator-started gateway would instead lack it). That is a misattribution, not a clean refusal, and it should be fixed before anything else on Hermes ≥ 0.22.1.

## Verification

Adversarial pass, 2026-10-23, against the hcom clone at `de33e31a0cdb`, the Hermes clone at `f3b5b980d615` and the GitHub API.

- Checked and held: stars/forks/license/last push (597 / 94 / MIT / 2026-10-21); release dates (hcom v0.7.30 2026-10-15T06:00Z, Hermes v0.22.1 2026-10-14T23:59Z); CHANGELOG and README lines; plugin size (1532 lines), 71 unit tests; every plugin constant and the line ranges of `bind`, `watch_cli`, `hand_off`, `deliver_idle`, `confirm`, `persisted_events`, `tick`, `turn_start`, `transform_tool_result`, `rpc`, the notify server, `setup`, bridge `hand_off` / `already_run`, `reload_plugin`, `show_notice`; the framing format; 50-message / 1 MiB limits; `hermes-read` ack RPC; Hermes `_cli_ref` (only the `None` initializer at plugins.py:1127 assigns it), the PLG-7 comments, `plugins_injection.py` queue choice, `transform_tool_result` first-string-wins (model_tools.py:831-848), the TUI injector, `reload_gateway_plugins`, the bridge's private gateway helpers, and the bundled `security-guidance` plugin's `transform_tool_result` hook.
- Changed: the Hermes clone revision (the cited `2647748a8b05` is not in the available clone; all `hermes:` line numbers match `f3b5b980`); "every PTY gate is off" (approval and launch-readiness gates stay on); `hcom send` wakes every endpoint system-wide via `wake_all`, not only the recipient's ports, with the resulting idle cost added; gateway env inheritance now cites `served_profile_child_env`; `gen` bumps on session finalize, not per-turn session end; TUI injector refusal conditions and line range; real-tool test line range; `require_attached` citation; the `HERDR_PANE_ID` consequence downgraded from VERIFIED to INFERRED (the strip list is verified, the effect is not observed).
- Not re-checked live: the "local path broken on 0.22.1+" conclusion stays INFERRED (no Hermes install).
