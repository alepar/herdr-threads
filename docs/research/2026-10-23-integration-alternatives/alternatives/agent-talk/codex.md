# agent-talk x OpenAI Codex CLI: injection and wake

- Upstream: https://github.com/xhluca/agent-talk, shallow clone at `8a7fc4f1` (v0.2.4). Paths are relative to the clone root. No code from the clone was executed.
- Harness reference: `../../primers/codex.md` (Codex 0.163.0, source tag `rust-v0.163.0`). Codex source paths below (`codex-rs/...`) are from that primer's read-only clone.
- agent-talk tested its Codex integration against **Codex 0.147** (`docs/codex-auto-receive.md:5,89-102`). Where 0.163 behaviour differs from what agent-talk assumes, this report says so.
- Tags: **VERIFIED** = read in source or docs (cited). **INFERRED** = my reasoning, not observed. **unknown** = not determinable from what was read.

## 1. Support status

**Native, two tiers.** VERIFIED.

| Tier | What | Default | Evidence |
|---|---|---|---|
| Hooks | `SessionStart` and `UserPromptSubmit` return `additionalContext` with the message bodies; `Stop` returns `decision:"block"` with the bodies in `reason` | Installed by `install-hooks.py`, inert until `AGENT_TALK_CODEX_SPOOLS` is set in the Codex process environment | `extensions/codex/inbox-hook.py:13-24,175-176`, `extensions/codex/install-hooks.py:31-55` |
| Idle wake | The spool writer with `--wake-codex` calls the app-server daemon over its control socket: `thread/loaded/list`, `thread/read`, then `turn/start` with a fixed nudge that carries no body | Off. The user has to start Codex with `codex-with-daemon` and add `--wake-codex` to the follower | `bin/codex_wake.py:300-334`, `bin/spool-writer.py:206-209`, `extensions/codex/codex-with-daemon:24-39` |
| Pull | `receive` skill, `retalk receive --peer … --save`, which the model runs | Always available | `skills/receive/SKILL.md` (overview) |

Distribution: a Codex plugin (`.codex-plugin/plugin.json`, skills only, with no `hooks` entry) installed with `codex plugin marketplace add xhluca/agent-talk` and `codex plugin add agent-talk@agent-talk` (`README.md:80-82`). The hooks are **not** shipped as plugin hooks. The `init` skill tells the model to run the installer script, which edits `config.toml` (`skills/init/SKILL.md:621-631`). VERIFIED.

## 2. Injection mechanism

### 2.1 Pipeline up to the spool (shared with the other harnesses)

`retalk receive --follow --interval 60 --quiet` → `spool-writer.py` → `<user>/sessions/<sid>.ndjson` for every sid in `~/.agent-talk/by-session/` that maps to this user (`bin/spool-writer.py:183-196`; the registry is re-read at most every 5 s, `:189-190`). On Codex the `sid` is the **`$CODEX_THREAD_ID` of the session that ran `init`**. The model reads it with `echo` and writes it out literally (`skills/init/SKILL.md:632-647`). The user then **relaunches** Codex with `AGENT_TALK_CODEX_SPOOLS=<user>/sessions/<that-id>.ndjson` (`:649-654`). The spool keeps the old name: "Any unique id works, because the spool is just a file" (`:645-647`). VERIFIED. That the relaunched session gets a new thread id is INFERRED (a fresh `codex` start, not a resume).

So the session's identity for delivery is **an environment variable naming a file**. The hook ignores Codex's hook payload, `session_id` included: `sys.stdin.read()  # Codex sends the event payload; we do not need it` (`inbox-hook.py:177-180`). VERIFIED.

### 2.2 The hook script

`extensions/codex/inbox-hook.py` takes one argv selecting the event (`:39-43`).

- **Read** (`drain`, `:91-152`): for each spool in `AGENT_TALK_CODEX_SPOOLS` (colon-separated, `:49-51`), it seeks to the persisted byte offset, reads to EOF, and keeps any partial trailing line for the next run (`:112-120`). It parses NDJSON and skips `kind == "contact"` records and records without `text` (`:138-142`). It dedupes by `id` against the persisted id list (`:143-147`).
- **Cursor** (`<sessions>/.codex-hook-state.json`, keyed by absolute spool path; `:54-88`): `{offset, ids[-500:], sha, sha_len}`. `sha` is the sha256 of the file's first `min(offset+consumed, 256)` bytes. On the next run, a shrunken file resets the offset to 0 (`:97-98`), and a prefix whose hash no longer matches also resets it (truncate-and-refill, `:99-106`). In both cases the id list keeps replays out. VERIFIED.
- **Advance before output**: `write_state(...)` runs inside `drain` (`:150-151`), before `main` prints anything. The docstring states the reason: "a hook that reported the same message twice would block every turn forever" (`:29-31`). VERIFIED. The write is atomic (tmp + `os.replace`), and an `OSError` is swallowed (`:82-88`). INFERRED consequence: when the state write fails, the message is still printed, the cursor stays put, and the next Stop re-blocks with the same message. The loop guard rests entirely on that write succeeding, because `stop_hook_active` is never read (see 2.4).

### 2.3 Payloads (VERIFIED `inbox-hook.py:162-213`)

Body rendering, joined with blank lines:

```python
lines.append(f"From {msg['name']}:\n{msg['text']}")
```

`name` falls back from `name` to `from` to `"peer"` (`:148`). There is no message id, timestamp, thread or priority in the model-visible text. The spool writer stamps `ts`, but the hook drops it.

`SessionStart` and `UserPromptSubmit` print:

```json
{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":
 "N new agent-talk message(s) waiting:\n\n<bodies>\n\nSurface these to the user verbatim as a chat transcript before continuing with their request."}}
```

`Stop` prints:

```json
{"decision":"block","reason":"N new agent-talk message(s) arrived while you were working:\n\n<bodies>\n\nShow each message to the user verbatim as a chat transcript (quote the text, name the sender), then reply over agent-talk if it asks something, using the send skill. If no reply is needed, say so briefly and stop."}
```

When there is nothing new, the hook prints nothing and exits 0 (`:183-184`). When `AGENT_TALK_CODEX_SPOOLS` is unset, it returns before reading stdin (`:175-176`).

### 2.4 When each path fires, and how Codex treats it

| Event | When it fires | How Codex treats the output (primer) | Authority |
|---|---|---|---|
| `SessionStart` (no matcher, so `startup`, `resume`, `clear`, `compact` and `fork` all match) | Session lifecycle | developer context (`primers/codex.md` §1.4) | developer |
| `UserPromptSubmit` | Every human prompt, and (INFERRED from agent-talk's 0.147 test, see 3.1) the app-server `turn/start` used by the wake | context added before the prompt | developer |
| `Stop` | Each end of a sampling loop with no follow-up | `reason` becomes "a new continuation prompt, treated as a new user prompt"; `codex-rs/core/src/session/turn.rs:737-754` records the hook prompt message and `continue`s **the same turn** with `stop_hook_active = true` | **user** |

There is no `PreToolUse` or `PostToolUse` hook, so mail that lands mid-turn waits until the turn's end (Stop). It is never added at a tool boundary. VERIFIED by absence in `install-hooks.py:31-55`.

The `Stop` hook ignores `stop_hook_active`. A peer that keeps sending while each continuation runs therefore keeps the turn alive indefinitely: every Stop finds new bytes and blocks again. Codex's own loop has no cap here (`turn.rs:737-754`, the loop continues on every block that carries a prompt). VERIFIED code, INFERRED consequence.

The overview already flags the authority split. The wake path refuses to push peer text as a user turn (`bin/codex_wake.py:6-11`), but the Stop path pushes the full peer bodies as exactly such a turn. VERIFIED both. INFERRED that this is inconsistent.

### 2.5 Size bounds

- agent-talk: none. Every pending message is joined into one string (`inbox-hook.py:162-166,186`). The only bound is the 8 MiB spool rotation (`bin/spool-writer.py:69`). VERIFIED.
- Codex: `SessionStart` and `UserPromptSubmit` additional contexts go through `maybe_spill_additional_contexts` (`codex-rs/hooks/src/events/user_prompt_submit.rs:126-127`, `session_start.rs:206-207`). Above about 2,500 tokens, the model gets a head-and-tail preview and the rest is saved under `hook_outputs` (`primers/codex.md` §1.4). VERIFIED. INFERRED: a large backlog therefore arrives **partially hidden**, and agent-talk's instruction to "Surface these … verbatim" cannot be fully followed.
- Codex `Stop` `reason`: the spiller does not appear anywhere in `codex-rs/hooks/src/events/stop.rs` (grep for `spill` finds nothing; `reason` goes straight to `continuation_prompt`, `:313-319`). VERIFIED by absence. INFERRED: a big backlog delivered at Stop is injected in full as a user message.

### 2.6 Dedupe and acknowledgement

- Dedupe: byte cursor plus the last 500 ids, persisted per spool (2.2). VERIFIED. The cursor is shared by every Codex process that has the same spool in its environment, so it acts as a "first reader wins" lock, not a per-session one (INFERRED; see 4 and 6).
- ACK: at the transport level only. The follower's `retalk receive` acknowledges at the relay before the record reaches the spool (overview, "Delivery guarantees"). The hook has no read receipt, and the sender gets no signal that the model saw the message. VERIFIED by absence in `inbox-hook.py`.

## 3. Wake mechanism

### 3.1 Exact mechanism (VERIFIED `bin/codex_wake.py`, `bin/spool-writer.py:206-209`)

1. The user launches `codex-with-daemon` (`extensions/codex/codex-with-daemon:24-39`). It runs `codex app-server daemon start` and checks for `"status":"started"` or `"alreadyRunning"`, then `exec codex --remote unix:// "$@"`. If the daemon cannot start, it runs plain `codex` and prints a two-line warning.
2. The follower runs `spool-writer.py --wake-codex` (passed through `bin/follow.sh:57`; persisted in `follow.opts` so restarts keep it, `:117-125`).
3. After each record has been appended to at least one registered spool, the writer calls `codex_wake.maybe_wake(user_dir, spools)` **synchronously** in its read loop (`spool-writer.py:206-209`).
4. `maybe_wake` (`codex_wake.py:300-334`):
   - It returns if the socket `$CODEX_HOME/app-server-control/app-server-control.sock` is missing (`:30,47-49,306-307`). The production path function (`app_server_control_socket_path`, re-exported from the `codex_app_server_transport` crate) is not in the sparse clone, so the real 0.163 default is unknown from source. Codex test fixtures disagree: `codex-rs/app-server-daemon/src/lib.rs:1408` (inside `mod tests`, `:1163`) and `codex-rs/tui/src/status/tests.rs:1727` use `app-server-control.sock`; `codex-rs/app-server-daemon/src/update_loop_tests.rs:445` uses `server.sock` (the primer's §2.1 guess). INFERRED: `app-server-control.sock` is likely right, since agent-talk's idle-wake test passed on 0.147 (`docs/codex-auto-receive.md:96-99`) and the status fixture renders it as a user-visible address.
   - It returns unless a nudge is due (3.3).
   - It opens a hand-rolled WebSocket over the Unix socket, offering no extensions, because the daemon "rejects handshakes offering permessage-deflate" (`:18-20,63-88`). Masked text frames (`:91-107`). Replies are matched by JSON-RPC id, and interleaved notifications are skipped (`:167-176`).
   - `initialize` with `clientInfo {name:"agent-talk"}`, then the `initialized` notification (`:313-315`).
   - `thread/loaded/list {}` (`:316`). The 0.163 schema returns `{data: [threadId], nextCursor}` (`codexprimer/schema/v2/ThreadLoadedListResponse.json`, VERIFIED). `nextCursor` is ignored, which is harmless for the single-thread case.
   - **It stands down unless exactly one thread is loaded** (`:317-322`).
   - `thread/read {threadId}` gives `thread.status.type`, and anything other than `"idle"` counts as busy and is skipped (`:206-220`). `ThreadStatus` is `notLoaded | idle | systemError | active{activeFlags}` (schema, VERIFIED), so `systemError`, `notLoaded` and `active` with `waitingOnApproval` or `waitingOnUserInput` are all skipped.
   - `turn/start {threadId, input:[{type:"text", text:NUDGE}]}` (`:325-327`).
   - It records the nudge mark only after `turn/start` succeeds (`:330-332`).
5. The `turn/start` turn fires `UserPromptSubmit`, whose hook (running in the daemon) attaches the bodies as `additionalContext`. The nudge tells the model to show them, and covers the case where nothing is attached: "If nothing is attached, the inbox was already drained; say so briefly and stop" (`:38-44`). VERIFIED text. That `UserPromptSubmit` fires for an app-server `turn/start` is INFERRED: agent-talk's 0.147 test reports the woken pane showing the nudge turn and then the agent quoting the message verbatim (`docs/codex-auto-receive.md:96-99`), which only the hook could have attached, but the doc does not name the event. Unchecked for 0.163.

The NUDGE is 301 bytes (about 65 tokens). It is a body-free, but not content-free, user prompt: it names the product and instructs the model. VERIFIED (measured).

### 3.2 Idle and busy detection

The server reports idle and busy on its side (`thread/read`). Nothing reads the screen. An unreadable status counts as **not busy**, so the wake fails open: "nudging a busy thread is only noise while not nudging an idle one loses the wake" (`:207-219`). VERIFIED.

There is a race between `thread/read` and `turn/start` (no compare-and-swap, `:323-327`). What 0.163 `turn/start` does on a thread that has just become busy is unknown from what I read: it might error, queue, or start a parallel turn. INFERRED risk: small.

A busy thread is never retried on a timer. The design relies on the Stop hook (`:323-324`; test `tests/test_codex_wake.py:276` "busy_thread_skipped_then_retried" retries on the *next record*). Gap (INFERRED): a record that lands after the turn's last Stop hook ran but before the status flips to `idle` is skipped as busy, and no Stop follows. It then waits for the next human prompt or the next record.

### 3.3 Nudge suppression (VERIFIED `codex_wake.py:246-297`)

`<user>/sessions/.codex-wake-state.json` maps each spool to its size at the last nudge. A new nudge is due only when, for some spool, there is no mark, the spool shrank, or the **hook's cursor offset** (read from `.codex-hook-state.json`) has reached the mark. The wake therefore never stacks while announced mail is unread, and it re-arms once the hook drains.

INFERRED failure mode: if the nudged turn runs without the hook draining (hooks untrusted under `/hooks`, `AGENT_TALK_CODEX_SPOOLS` missing from the **daemon's** environment, or the hook path broken), the cursor never passes the mark. The writer then never nudges again until the spool rotates or is swept, or some later working hook run drains past the mark. In this state every later message gets **no wake at all**, and the model said "inbox was already drained" once.

### 3.4 Draft safety

`turn/start` goes through the app-server, so no keystrokes are typed and the composer buffer is not touched. VERIFIED that nothing types (`codex_wake.py` has no pty or tmux path). INFERRED: a human mid-draft in the TUI then watches an injected turn start beside the draft. The draft survives but the human's turn is pre-empted. agent-talk does no draft check, and the daemon status `idle` does not cover a half-typed composer.

### 3.5 Latency

INFERRED from the cited constants: relay poll of up to 60 s by default (`bin/follow.sh:15-16`; `--interval 5` for "rapid live exchange", `skills/receive/SKILL.md:211-212`), plus up to 5 s before a newly registered session is seen (`spool-writer.py:189`), plus a wake attempt of at most 3 s (`codex_wake.py:33`), plus model start. The relay poll dominates. A Stop delivery happens at the next end of a sampling loop with no follow-up.

Because the wake is synchronous inside the writer's stdin loop, a hanging daemon delays fan-out of the *next* record by up to 3 s (INFERRED; tests `:289,299` check that a hanging or refusing server does not break delivery).

### 3.6 Failure modes

| Failure | Behaviour | Evidence |
|---|---|---|
| No daemon or no socket | Silent no-op. Delivery falls back to the hooks at the next prompt or the next Stop | VERIFIED `codex_wake.py:13-16,306-307`; doc test `docs/codex-auto-receive.md:100-102` |
| More than one loaded thread | Stand down | VERIFIED `:317-322` |
| Daemon restart | Kills every attached session; the launcher avoids `daemon bootstrap` because its auto-updater restarts the daemon | VERIFIED `docs/codex-auto-receive.md:156-159` |
| Stale daemon environment | Hooks run inside the daemon with the daemon's start-time `AGENT_TALK_CODEX_SPOOLS`. The launcher only warns | VERIFIED `codex-with-daemon:11-16,28-31`, `docs/codex-auto-receive.md:180-185` |
| `--remote` session cwd | Takes the daemon's cwd on 0.147 | VERIFIED doc `:186-189` |
| Hooks untrusted | One nudge, then permanent silence (3.3) | INFERRED |
| Socket access | "Any local process that can open the daemon's socket can drive your session" | VERIFIED doc `:170-171` |

## 4. Setup and installation footprint on the harness

VERIFIED `extensions/codex/install-hooks.py`:

- **Appends** a marked block (`# >>> agent-talk inbox hooks >>>` … `<<<`) to `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`), with three inline `[[hooks.SessionStart]]`, `[[hooks.UserPromptSubmit]]` and `[[hooks.Stop]]` tables. Each has `type="command"`, `command='python3 "<abs path to inbox-hook.py>" <event>'`, `async=false` and `statusMessage="Checking agent-talk inbox"` (`:27-55,119-128`). Idempotence is "marker present", so the block is never rewritten.
- **Copies** `codex-with-daemon` to `~/.local/bin` (mode 0755) and warns when that directory is not on PATH (`:130-144`).
- Does **not** write hook trust. The user approves the three entries under `/hooks`, or passes `--dangerously-bypass-hook-trust` (`:17-20`; doc `:80-85`).
- The session launch is changed by hand: the env var, optionally the launcher, and (doc) the standalone Codex install, because "the daemon will not run from the npm package alone" (`docs/codex-auto-receive.md:147-149`).
- Outside Codex: `~/.agent-talk/by-session/<thread-id>`, `<user>/sessions/*.ndjson`, `.codex-hook-state.json` and `.codex-wake-state.json`.

INFERRED issues with the footprint:
- The hook command embeds the **absolute path of the plugin copy that ran the installer**. A plugin upgrade that moves the cache directory leaves the old path in `config.toml`, and the marker check means re-running the installer does not fix it. The result is hooks that fail to start.
- No uninstall path. `--check` only reports.
- Inline `[hooks]` in `config.toml` coexist with `hooks.json` and plugin hooks ("Hooks from all sources run", primer §1.1), so the block does not conflict with herdr-threads' `hooks.json` groups. Both run on every event.

## 5. Token cost

INFERRED estimates from the measured strings (bytes / 4.5 ≈ tokens).

| Case | Model-visible cost | Other cost |
|---|---|---|
| Quiet turn (nothing new) | **0 tokens.** The hook prints nothing (`inbox-hook.py:183-184`) | Two `python3` process starts per turn (`UserPromptSubmit` + `Stop`), plus one at session start. Each reads and hashes 256 bytes and rewrites nothing when the size is unchanged (`:107-108` returns before `write_state`). A "Checking agent-talk inbox" status line flashes each time |
| Message at prompt or session start | header about 13 tokens + `From <name>:\n` + full body + 93-byte trailer (about 20 tokens) | Spilled to a preview above about 2,500 tokens |
| Message at Stop | header about 13 + body + 213-byte trailer (about 47 tokens) | **One more sampling round** on the full context, plus the model's transcript echo ("verbatim") and the model's reply. The echo roughly doubles the body's cost in output tokens |
| Idle wake | NUDGE about 65 tokens + the `UserPromptSubmit` payload above | A whole new turn: one full-context sampling request at minimum. When the hook already drained, the turn is spent for a "say so briefly" |

The per-message overhead of about 35 to 60 tokens is small. The real cost is that **every** delivery is told to re-print the message verbatim, and that every Stop delivery and every wake buys a full sampling round. Nothing is batched by priority, and nothing stays silent. VERIFIED instruction text; INFERRED cost.

## 6. Limitations and bugs I can see

1. **Codex 0.157+ default daemon attach undermines the "exactly one thread" rule and "plain codex is unchanged"** (INFERRED, high confidence). agent-talk's assumptions come from 0.147: "a plain `codex` keeps its app-server in-process" (`codex-with-daemon:6-8`, `docs/codex-auto-receive.md:150-152,199-202`). On 0.163 the TUI tries the local daemon first and falls back to embedded only when none is running (`primers/codex.md` §2.1, `codex-rs/tui/src/lib.rs:560-600`). Once `codex-with-daemon` has started a daemon, every later plain `codex` (without `--no-daemon`) attaches to it too, with three consequences: (a) `thread/loaded/list` returns several threads and the wake stands down; (b) those sessions run their hooks with the **daemon's** environment, so they share one `AGENT_TALK_CODEX_SPOOLS`; (c) through the shared cursor, whichever session's hook fires first consumes the mail, possibly a session that never opted in.
2. **Ghost-thread wake** (INFERRED). A thread stays loaded for a 30-minute grace period after its last client leaves (`primers/codex.md` §2.2-2.3). Suppose the user closes their only attached session, and within 30 minutes no other session attaches, or the user runs a `--no-daemon` one. The orphan is then "the one loaded thread" and `idle`, so `turn/start` runs a turn nobody sees. Its `UserPromptSubmit` hook advances the shared cursor, and the mail is marked delivered into a session no human or live TUI is watching.
3. **Subagents count as loaded threads** (INFERRED, unverified). If a multi-agent subagent thread shows up in `thread/loaded/list`, spawning one disables the wake.
4. **Stop-loop guard has a single point of failure** (2.2, 2.4): `stop_hook_active` is ignored, and a failed state write repeats the block.
5. **Unbounded backlog** (2.5): the `Stop` reason is unspilled, the `additionalContext` is spilled to a head-and-tail preview, and in both cases the instruction to echo verbatim multiplies the cost.
6. **Peer text as a user prompt at Stop**, against the project's own rule in `codex_wake.py:6-11`.
7. **No tool-boundary delivery**: a long agentic turn hears nothing until it ends. `PostToolUse` or async hooks exist (`primers/codex.md` §1.4-1.6). Note that agent-talk's doc says `async` is "parsed but not implemented" (`docs/codex-auto-receive.md:205-209`). That was true of their 0.147 reading, but 0.163 implements async command hooks (`primers/codex.md` §1.6, `codex-rs/hooks/src/engine/dispatcher.rs:134-141`).
8. **Identity is a model-transcribed file name plus a process environment**: the thread id from `echo`, copied literally, then a relaunch. Session identity and hook payload are never cross-checked. A wrong spool path is silent.
9. **Busy-to-idle edge gap** (3.2) and **permanent wake silence after a non-draining nudge** (3.3).
10. **No read ACK and no priority**: every message is treated the same, pushed whole and echoed, and the sender learns nothing.
11. The hook drops the `ts` and `id` that the writer has, so the model cannot cite or dedupe a message by id when it replies.

## 7. Clever ideas worth stealing

1. **Native app-server idle wake** (`turn/start` over `app-server-control.sock`) gated by server-side `thread/read` status, with no typing, no composer read and no clobbered draft. herdr-threads could use it with a **content-free** text (our rule (c)), and could get better targeting than agent-talk's "exactly one thread" by mapping our binding to the Codex `thread_id` from the hook payload (`session_id`/`thread id`, primer §9). The durable `thread/queue/add` (primer §3) is the even safer sibling, since it is idle-gated on the server, though it is not dispatched after an interrupt. Conflict to resolve: we advise `--no-daemon`, and this path needs the daemon.
2. **Hand-rolled minimal WebSocket client with no extensions** (`codex_wake.py:63-157`): about 100 lines of standard library, plus the concrete fact that the daemon refuses permessage-deflate. That is useful if we ever talk to the socket from a hook-weight process.
3. **Stop `decision:"block"` for mail that arrived mid-turn**, so the session never goes idle with pending p0 mail and a later nudge becomes unnecessary. For us it should be gated on priority `<= T` and on `stop_hook_active`, and should carry a digest or pointer rather than bodies.
4. **Nudge suppression keyed on the reader's cursor** (`_nudge_due`): no second wake until the consumer has moved past the mark of the last wake. That matches our "re-offer bounded" idea, and it is cheap state.
5. **Truncate-and-refill detection** (offset plus sha256 of the first 256 bytes) for any file cursor.
6. **Cursor advanced before output**, which makes a repeated Stop idempotent by construction. We should keep this rule in mind if we add a Stop hook.
7. **Wake mark recorded only after a successful `turn/start`**, so a failed attempt re-arms on the next record.
8. A cautionary point rather than an idea: a body-free wake prompt can still cost a full turn when the inbox is already empty. agent-talk accepts this with "say so briefly and stop". Our content-free `wake up` has the same exposure, and gating the wake on a fresh pending check just before `turn/start` would avoid it.

## Verification

Adversarial pass, 2026-10-23, against the clone at `8a7fc4f1`, the Codex `rust-v0.163.0` sparse clone and schema dump the primer used, and `primers/codex.md`.

- Checked and confirmed: every `extensions/codex/inbox-hook.py`, `install-hooks.py`, `codex-with-daemon`, `bin/codex_wake.py` and `bin/spool-writer.py` line citation (hook events, state file layout, sha prefix, advance-before-output, payload strings, marker idempotence, launcher copy, nudge gating, one-thread rule, busy check, mark-after-success); `.codex-plugin/plugin.json` (skills only, no `hooks`); `README.md:80-82`; `skills/init/SKILL.md:621-654`; `bin/follow.sh:15-16,57,117-125`; `skills/receive/SKILL.md:211-212`; `tests/test_codex_wake.py:276,289,299`; every `docs/codex-auto-receive.md` line cited; Codex `turn.rs:737-754` (block records the hook prompt, sets `stop_hook_active`, `continue`s, no cap), `stop.rs:313-319` (no spiller), `user_prompt_submit.rs:126-127` and `session_start.rs:206-207` (spiller), `dispatcher.rs:134-141` (async scheduled), `ThreadLoadedListResponse` and `ThreadStatus` schemas. Measured: NUDGE 301 bytes; trailers 93 and 213 bytes.
- Changed: the control-socket claim (3.1). `lib.rs:1408` is a test fixture, the production path function is outside the sparse clone, and another fixture uses `server.sock`, so "matches Codex source" and "the primer is wrong" were overstated; now INFERRED. "`UserPromptSubmit` fires for `turn/start`, VERIFIED by agent-talk on 0.147" downgraded to INFERRED (the doc reports the observed result, not the event). The relaunched session's new thread id marked INFERRED. The permanent-wake-silence failure mode now notes that a later successful hook drain also re-arms it.
- Not checkable here: the Codex `build_hook_prompt_message` item role (crate not in the sparse clone; the "user" authority rests on the hooks docs quote in the primer); retalk itself (not cloned).
