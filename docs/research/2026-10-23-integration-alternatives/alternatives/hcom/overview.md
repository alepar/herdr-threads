# hcom: overview

- Upstream: https://github.com/aannoo/hcom (PyPI `hcom`, Homebrew `aannoo/hcom/hcom`)
- Clone used: `alts/hcom` (scratchpad), `git clone --depth 1`, HEAD `de33e31a0cdb` (2026-10-21 04:37 +0100, release v0.7.31). The clone was only read; nothing in it was executed.
- Citations are `path:line` relative to the clone root. **V** = VERIFIED (read in source or docs), **I** = INFERRED.

## What it is

hcom ("hook comms") connects independent coding agents running in separate terminals, and optionally on separate machines. Agents can message each other, watch each other's activity, subscribe to events, and spawn, fork, resume or kill other agents (README.md:5-7, 90-98) **V**. You launch an agent by putting `hcom` in front of the tool's command (`hcom claude`, `hcom codex`, ...) and then prompt it normally (README.md:5, 42-65) **V**. The README lists 14 tools: claude, codex, opencode, grok, hermes, pi, omp, agy, copilot, qoder, cursor, kimi, kilo and gemini. Any other tool can join manually with `hcom start` / `hcom listen` (README.md:9, 174-192) **V**. Agents learn the CLI from a primer of about 700 tokens injected at launch (README.md:231) **V**.

Closest relative: of all the alternatives, this is the one nearest to herdr-threads in scope and technique. It uses hooks for in-turn delivery, a Claude Code **mod** that submits a content-free `<hcom>` wake (the same mechanism family as our Claude mod), a per-tool integration registry, and it even ships a Herdr terminal preset. The main architectural difference: hcom runs **every interactive agent inside its own PTY proxy** (`hcom pty`). Gates and wakes are driven by screen scraping of that PTY, not by Herdr's pane state.

## Architecture

| Aspect | Finding | |
|---|---|---|
| Process model | One `hcom` PTY-proxy process per interactive agent. It wraps the tool in its own PTY, scrapes the screen (VT100 emulation, `src/pty/screen.rs`), and runs a delivery thread (`src/delivery.rs:1580` `run_delivery_loop`). Interactive `hcom claude` (no `-p`) is routed to `claude-pty` (src/launcher.rs:1762-1766). Every other tool is PTY-wrapped by default (src/integration_spec.rs `uses_pty_default: true`; src/launcher.rs:97-100) | V |
| Hook processes | Hooks are short-lived `hcom <hook-name>` invocations: JSON on stdin for Claude, Codex and agy, argv RPC for the OpenCode, Pi and Hermes plugins (src/integration_spec.rs:44-55) | V |
| Daemon | No central message daemon. "Hooks record activity to a local SQLite database and deliver messages from it" (README.md:100-106). The only long-lived background process is the optional MQTT `relay-worker` (src/relay/worker.rs:1-7; src/commands/daemon.rs:1-4) | V |
| Storage | SQLite `~/.hcom/hcom.db` in WAL mode, busy_timeout 5000 (src/db/mod.rs:163, 219-224). Messages are rows in a shared `events` table (`type='message'`) (src/db/events.rs:125-129). `HCOM_DIR` relocates all state (README.md:313-318) | V |
| Local transport / wake signalling | Localhost TCP. Each poll loop (PTY delivery thread, Stop-hook poll, mod poll, `hcom listen`, `events --wait`, plugins) binds `127.0.0.1:0` and records its port in `notify_endpoints`. Senders "connect-and-close" to wake it (src/notify/mod.rs:1-45; src/notify/wake.rs:1-70; src/notify/server.rs:1-60). `hcom send` broadcasts the wake to every registered port (`wake_all`, src/notify/wake.rs:54-73). Text reaches the PTY through a separate localhost TCP **inject** server (src/pty/inject.rs:1-70) | V |
| Auth on local ports | None found. The inject and notify listeners accept any local connection, and the inject server writes the text it receives into the PTY (src/pty/inject.rs:47-66; src/delivery.rs:1299-1324). SECURITY.md:39 states that hcom "does not defend against another user on the same account". A TCP port on 127.0.0.1 is also reachable by *other* OS accounts on the host, which is wider than same-account (I) | V/I |
| Cross-machine | Yes. An MQTT relay over public brokers (broker.emqx.io, ...) or a private one, with payloads end-to-end encrypted by a PSK carried in a join token (XChaCha20-Poly1305) and a replay guard (README.md:152-163; src/relay/mod.rs:1-30; SECURITY.md:11-32). Membership is all-or-nothing: a peer can launch, kill and drive agents remotely (SECURITY.md:20, 34-38) | V |
| Terminals | Spawns agents into kitty, wezterm, tmux, zellij, waveterm, cmux, **herdr**, iTerm and others (README.md:142-150). The herdr preset uses `herdr tab create` plus `herdr pane run`, and sets the `HERDR_AGENT=<tool>` hint on `hcom pty` so Herdr can still classify the wrapped agent (src/shared/terminal_presets.rs:560-573; src/launcher.rs:690-700). It records `HERDR_SOCKET_PATH` so it can close the pane later (src/pidtrack.rs:33, 517, 680-687) | V |
| Hook installation | Per run wherever the tool allows it. Claude gets `--settings <published settings.json>` and `--plugin-dir <mod>` (src/hooks/claude.rs:2693-2766, 2872-2941). Codex gets `-c hooks.*` session-flag overrides, plus a hook-trust preflight cached per Codex version and declaration digest (src/hooks/codex.rs:240-300). agy writes a `hcom-lifecycle` group into the global `~/.gemini/config/hooks.json`, guarded by `command -v hcom` (src/hooks/antigravity.rs:68-118). Hermes installs a profile plugin (src/hooks/hermes.rs:1-9). OpenCode and Pi load managed TS plugins | V |

## Identity and session model

- **Names.** Instances get short random names (`luna`, `nova`), optionally prefixed by a tag (`api-luna`), and are addressed as `@name`, `@tag-` or `@name:DEVICE` (src/commands/send.rs:20-24). Names are never reused (CHANGELOG.md, 0.7.31 "Added") **V**.
- **Binding.** "3-tier binding (process → session → ad-hoc)" (src/identity.rs:1) **V**. The launcher sets `HCOM_PROCESS_ID`, and hooks run only when it is present (`hook_gate_check`, src/hooks/common.rs:123-127). So hooks are inert in non-hcom launches **V**. Each hook binds the tool's session id to the process binding (src/instance_binding.rs:1-40). The Claude mod's poll is honoured only if the process binding, the session and the validated Claude session owner all agree (`validated_owner`, src/hooks/claude_mod.rs:420-433). "Only root hooks establish ownership; mod arguments cannot bind a session" (:331) **V**.
- **Rename and resume.** The PTY delivery loop re-reads its process binding every cycle and migrates notify endpoints when the instance name changes (src/delivery.rs:60-102) **V**. A stop and resume replaces the instance row. `created_at` fences any delivery read from the old row (src/hooks/common.rs:467-480, 544-554; ack fence at :315-334) **V**.
- **Subagents.** Claude Task subagents get their own identity through `hcom start --name <agent-id>`. They poll from `SubagentStop`, with a `subagent_timeout` keep-alive (README.md:208-210, 302; src/hooks/claude.rs:2715) **V**.
- **Senders.** `hcom send` from a plain shell is an "external" sender (`-b`, `--from`). Subagents are refused `--from`/`-b` as "external sender spoofing" (src/commands/send.rs:144, 778-791). Routing namespaces are `ext_`/`sys_`/instance (:450-455) **V**. This is cooperative identity with no cryptographic sender binding locally (I, consistent with SECURITY.md:37-39).

## Injection mechanisms (general)

All message bodies are framed by `messages::format_messages_json`. The whole batch is wrapped in a single `<hcom>…</hcom>` tag. Each message is `[intent:thread #id] sender → recipient (+N others): text`, a multi-message batch is prefixed `[N new messages] | … | …`, and the per-instance or config `hints` string is appended to **every** delivery, plus one-time "tips" on first receipt of a thread or intent (src/messages.rs:548-652, 655-668) **V**. Bounds: at most 50 messages per delivery (`MAX_MESSAGES_PER_DELIVERY`, src/shared/constants.rs:19) and a per-message cap of 1 MiB (`MAX_MESSAGE_SIZE`, :22; enforced at src/messages.rs:143). No per-delivery byte cap was found. One delivery can therefore carry up to about 50 MiB of text into context (I, from the two constants).

| Point | Mechanism | |
|---|---|---|
| Mid-turn (Claude) | `PostToolUse` → `hookSpecificOutput.additionalContext` (model) plus `systemMessage` (human display) (src/hooks/claude.rs:1685-1743) | V |
| Turn start (Claude, PTY) | `UserPromptSubmit` delivers pending mail as `additionalContext` on **any** prompt, the human's or the `<hcom>` wake (src/hooks/claude.rs:1872-1905) | V |
| Idle, non-PTY (Claude `-p`, subagents) | The `Stop` / `SubagentStop` hook *blocks*. It long-polls the DB with a TCP-wake select (hook timeout 86400 s) and returns `{"decision":"block","reason":<batch>}` with exit 0 (src/hooks/common.rs:426-632; src/hooks/claude.rs:1788-1840, 2710) | V |
| Codex | `PostToolUse` (mid-turn) and `UserPromptSubmit` **only when the prompt is exactly `<hcom>`**. Both put the batch in `systemMessage` (TUI) and `additionalContext` (model) (src/hooks/codex.rs:750-762, 858-881, 918-938). Bootstrap goes through `developer_instructions` at launch (:854) | V |
| agy | `PreInvocation`/`PostInvocation` (mapped from gemini-sessionstart/beforeagent/afteragent) return `{"injectSteps":[{"ephemeralMessage": ctx}]}`. agy's `PostToolUse` cannot inject and returns `{}` (src/hooks/gemini.rs:590-640, 460-470) | V |
| OpenCode | TS plugin. OpenCode 1: `client.session.promptAsync` with the whole batch as a user text part. OpenCode 2: `ctx.session.prompt({…, delivery:"steer"})`, which "reaches the model at the next step boundary of a running turn". Bootstrap goes through `session.hook("context")` → `draft.system.push` (src/opencode_plugin/hcom.ts:534-585, 1022-1040, 1124-1178) | V |
| Pi | Extension. `pi.sendUserMessage(text, idle ? undefined : {deliverAs:"steer"})` on `tool_result`, `turn_end` and `agent_settled`. A bare `<hcom>` typed into the input is replaced with the batch through the `input` hook's `{action:"transform"}`. Bootstrap is a hidden custom message at `before_agent_start` (src/pi_plugin/hcom.ts:283-300, 489-577) | V |
| Hermes | Python plugin. Mid-turn: appended to the tool result (`transform_tool_result`). Turn start: `pre_llm_call` context. Idle: queued as the next user turn on the classic CLI's private `_pending_input` queue, or through the TUI's `inject_tui_message` (src/hermes_plugin/__init__.py:1-30, 568-584, 915-931) | V |
| Manual (any tool) | `hcom listen` blocking poll, or the agent running `hcom` commands it learned from the primer (README.md:110-121; src/notify/mod.rs:33-36) | V |

## Wake mechanisms (general)

1. **PTY keystroke injection (default for Codex and agy; opt-in `claude_delivery inject` for Claude).** The delivery thread waits on its notify port (250 ms retry fallback, src/delivery.rs:1327) and opens a "gate" before typing (src/delivery.rs:656-688, 1008-1089). The checks run in order: DB status must be `listening` (set by the Stop hook), no approval prompt (hooks, or Codex OSC9 / title scrape, latched for 400 ms against redraw flicker), no keystroke within the user-activity cooldown (0.5 s, 3 s for Claude), a 1.5 s settle after an observed prompt submit, ready pattern visible, and composer empty. The empty-composer check uses VT100 *dim* attribute detection to tell placeholder text from a human draft. It also refuses to type while a Claude subagent navigator or session switcher overlay is focused (src/delivery.rs:854-930, 944-975, 1035-1040) **V**. It then types `<hcom>` (Claude and Codex, whose hooks display the body) or a short metadata-only preview `<hcom>[intent:thread #id] a -> b</hcom>` (agy, Gemini, OpenCode bootstrap). It never types the body, strips `@` and C0/ESC characters, and falls back to the bare trigger when the preview would wrap (src/delivery.rs:560-654, 1299-1324) **V**. Submission is verified in phases: the text must render as the *exclusive* composer content (a draft containing it counts as "mixed" and is never submitted), Enter is pressed (at most 3 attempts), and the cursor must advance within 10 s (src/delivery.rs:1327-1419, 1513) **V**. A Claude box that holds only a leftover `<hcom>` is re-submitted with Enter rather than typed over (src/delivery.rs:687-688, 773-782) **V**.
2. **Claude mod (default since 0.7.31, needs Claude Code ≥ 2.1.287).** `register.js` starts on `session.start` only for hcom-launched interactive PTY sessions with native delivery. Every 250 ms it spawns `hcom claude-mod-poll --session-id <id>`. That process long-polls its own notify port for up to 10 s and returns `{wake:true}` only when the row is idle, has pending mail, and this cursor has not already been attempted (`wake_due`). The mod then calls `$.prompt.submit({text:'<hcom>', asUser:true})`, and the `UserPromptSubmit` hook delivers the body. A wake that resolves into a different session is dropped (src/claude_mod/hooks/register.js:35-138; src/hooks/claude_mod.rs:230-278, 413-418; CHANGELOG.md 0.7.31 "Changed") **V**. Health states `tui:mod-failed|unavailable|unacknowledged|waiting` are surfaced in status. There is "no automatic PTY fallback"; the user has to switch to inject manually (src/hooks/claude_mod.rs:27-33, 102-195) **V**.
3. **Stop-hook block-and-continue** for headless `-p` Claude and subagents (see the injection table) **V**.
4. **Harness-native prompt APIs** in the plugin tools: OpenCode `promptAsync` / `prompt(steer)`, Pi `sendUserMessage`, Hermes `_pending_input` / `inject_tui_message`. Here the wake and the body are the same act. The PTY gates are switched off for OpenCode, Pi and Hermes because "the plugin owns delivery after bootstrap" (src/integration_spec.rs:1554-1563, 1914-1922, 2075-2083) **V**.
5. **Scripting.** `hcom events --idle X --wait N && hcom send …`, and `hcom run onidle` types text into another agent once one goes idle (README.md:136-140, 349) **V**.

Wake policy: every pending message wakes an idle recipient. `hcom send` has no delivery-priority, threshold or "silent" option (src/commands/send.rs:55-100 flag set; no `priority` in send.rs) **V**/**I**. The only distinction is `intent` (`request|inform|ack`), which affects framing and request-watch subscriptions, not whether a message wakes the recipient (src/commands/send.rs:83-85, 420-478) **V**.

## Delivery guarantees

- **Ack = cursor advance.** Each instance has one forward-only `last_event_id` cursor over the global `events` table. Unread messages are `id > cursor AND type='message'`, filtered in Rust by scope or mention (`should_deliver_to`) (src/db/events.rs:103-143) **V**. Hook paths prepare the batch *without* committing and commit the ack only after stdout is flushed (`prepare_pending_messages` → `commit_delivery_ack`), forward-only and fenced by `created_at` (src/hooks/common.rs:304-346, 411-424) **V**. Plugins use an outstanding-record state machine, handed_off → offered → admitted → acked, and redeliver an unadmitted batch after a 10 s grace ("text the model has seen is never withheld because an ack reply went missing", src/hermes_plugin/__init__.py:23-29; src/opencode_plugin/hcom.ts:12-17; src/pi_plugin/hcom.ts:520-530) **V**.
- **No per-message read receipt** for the sender beyond the cursor and status `deliver:<sender>`. Semantic acks are messages with `--intent ack --reply-to`, with ack-on-ack and ack-of-inform loops refused (src/commands/send.rs:425-440) **V**. A request to specific mentions auto-creates "request-watch" subscriptions (:470-478) **V**.
- **Ordering.** By global event id (`ORDER BY id`, src/db/events.rs:128) **V**. No priority ordering.
- **Durability.** SQLite WAL on the local disk. Messages persist across agent restarts, and resume (`hcom r`) restores subscriptions (src/delivery/antigravity.rs:45-48) **V**. The relay syncs events across devices via retained MQTT state plus backfill (src/relay/mod.rs:1-8; src/relay/backfill.rs) **V**.
- **At-least-once vs at-most-once.** The hook path is at-least-once up to the stdout flush. A hook that prints and then crashes before the commit redelivers (I, from the commit order). The PTY path re-types up to 3 times when the cursor does not advance, and Claude fast-fails (src/delivery.rs:1382-1398) **V**.

## Human participation

- Humans type into the agent's real terminal. hcom treats any keystroke as user activity and defers its own injection (src/delivery.rs:977-988, 1029-1034) **V**.
- `hcom` with no arguments opens a TUI dashboard (agents, messages, status, launch, kill) (README.md:84-88; src/tui/) **V**.
- `hcom send -b @luna -- …` from any shell or script, as an external sender (README.md:123-134) **V**. `hcom term [name]` views or injects into an agent's PTY screen (README.md:263) **V**.
- Hermes gateway chats (Telegram, Discord) can join with `/hcom start` (README.md:212-222) **V**.
- Human-visible echo: the `systemMessage` mirrors each Claude and Codex delivery in the TUI (src/hooks/claude.rs:1729-1740; src/hooks/codex.rs:754-762) **V**.

## License and maturity

| | |
|---|---|
| License | MIT (LICENSE; README.md:390-392) **V** |
| Language | Rust binary plus TS plugins (OpenCode, Pi, OMP, Copilot) and a Python plugin (Hermes). About 199k lines in total including the lockfiles (wc over the clone) **V** |
| Stars / forks | 597 / 94 (GitHub API, 2026-10-23) **V** |
| Created | 2025-07-21 (GitHub API) **V** |
| Last commit / release | 2026-10-21, v0.7.31 (latest). v0.7.30 on 2026-10-15, v0.7.29 on 2026-10-12 (GitHub releases) **V** |
| Cadence | Releases every few days, with a 1117-line CHANGELOG. Python was replaced by Rust in an earlier 0.7.x release (CHANGELOG.md:590) **V** |
| Tests | 14 files under `tests/`, including PTY-delivery, relay-roundtrip and real-tool Codex suites (tests/test_pty_delivery.rs, tests/test_relay_roundtrip.rs, tests/real_tool_codex.rs) **V** |

## Harness support

| Harness | Status | Injection | Wake | |
|---|---|---|---|---|
| claude | native | `PostToolUse` / `UserPromptSubmit` `additionalContext` + `systemMessage`; `Stop` block-loop for `-p` and subagents | Claude mod `$.prompt.submit('<hcom>')` (default) or PTY typing `<hcom>` (inject mode) | V |
| codex | native | `PostToolUse`, and `UserPromptSubmit` on an exact `<hcom>` prompt (`additionalContext`); per-run `-c hooks.*` | PTY typing `<hcom>`, gated on Stop-set `listening`, OSC9/title approval scrape and dim-text draft detection | V |
| hermes | native | Plugin: `transform_tool_result` (mid-turn), `pre_llm_call` (turn start) | Plugin queues the next user turn (`_pending_input` / `inject_tui_message`); PTY gates off | V |
| agy | native | `PreInvocation`/`PostInvocation` `injectSteps[].ephemeralMessage`; `PostToolUse` cannot inject | PTY typing a metadata-only `<hcom>…</hcom>` preview line; hooks in global `~/.gemini/config/hooks.json` | V |
| goose | none | No mention anywhere in the clone (grep `goose`, case-insensitive) | Could join only generically through `hcom start` / `hcom listen` (README.md:110-121, 192) | V (absence) / I (generic path) |
| pi | native | Extension `sendUserMessage` (`deliverAs:"steer"` mid-turn); `input` transform of a bare `<hcom>` | Same API when idle; readiness from the plugin bind | V |
| opencode | native | Plugin `session.promptAsync` (v1) / `session.prompt({delivery:"steer"})` (v2); `system.push` bootstrap | Same prompt API; a body-free typed wake only to start a missing OpenCode 2 session | V |

## Points worth carrying into the comparison

1. **Same Claude technique as ours, with different plumbing.** Both projects use a mod that submits a content-free wake under plugin attribution. hcom's mod *spawns a process every 250 ms*, and that process long-polls a TCP port for up to 10 s (I: roughly one process start per 10 s while idle, more while busy). It never streams bodies. Bodies always come through `UserPromptSubmit`/`PostToolUse`. herdr-threads' mod channel puts items into context directly.
2. **Its own PTY proxy instead of the multiplexer.** Owning the PTY gives hcom keystroke timing (user-activity cooldown), VT attribute data (dim placeholder vs a real draft) and exclusive-ownership checks before Enter. These are richer draft-protection signals than a pane-content check. The cost: one more layer between the terminal and the agent, and Herdr only sees `hcom pty` unless `HERDR_AGENT` is set.
3. **Codex delivers at `UserPromptSubmit` only for the wake prompt.** A human prompt does not pull mail in, so mail stays queued until a tool call or the next wake (I: this avoids hijacking a human turn but delays delivery).
4. **agy `injectSteps[].ephemeralMessage`** at Pre/PostInvocation, where herdr-threads uses `systemMessage`. Worth checking which one agy keeps in context and which it shows.
5. **Plugin "steer" APIs** in OpenCode 2 and Pi deliver mid-turn at the next step boundary with no typing at all. This is a candidate for our goose, pi and opencode adapters.
6. **Weaknesses relative to our contract:** no priority or threshold (every message wakes, which our delivery rule 1 forbids by default), static `hints` appended to every delivery (against our token rule a), no per-delivery byte cap (50 × 1 MiB), unauthenticated localhost TCP inject and wake ports, a dependence on Hermes' private `_pending_input`, an O(all messages since cursor) scan filtered in Rust, and a 24 h blocking Stop hook for headless Claude.
