# agmsg x OpenAI Codex CLI: message injection and wake

- Upstream: https://github.com/fujibee/agmsg. The clone read is shallow, HEAD `6e031546` (2026-10-23, release 1.6.0). Paths are relative to the clone root. No code from it was executed.
- GitHub (gh api, 2026-10-23): 1,564 stars, last push 2026-10-23T05:39Z. Latest release v1.6.0.
- Harness facts (hook semantics, queue engine, app-server) come from `../../primers/codex.md`, cited as "primer §N".
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means my reasoning, not confirmed by source or by a run. **unknown** means not found.

---

## 1. Support status

**Native, with three delivery paths.** All three ship in 1.6.0. VERIFIED: `scripts/drivers/types/codex/type.conf:150` (`delivery_modes=monitor turn off`), `README.md:269-290`, `docs/codex-monitor-beta.md:1-12`.

| Path | Status | What carries the message | What wakes an idle session |
|---|---|---|---|
| **`turn`** | default for Codex without monitor or agmsgd (`README.md:275`) | Stop hook `decision:"block"` with the bodies, plus a PostToolUse `additionalContext` copy | nothing. "A Codex session that is not on monitor (legacy) or agmsgd delivery only looks at its inbox on its next turn, so it never notices a message you `send` after it goes idle." (`README.md:219`) |
| **`monitor (legacy)`** | beta, still the fallback mode | a turn the agmsg bridge starts with app-server `turn/start`, with the bodies inline | the same `turn/start`, started from a per-seat app-server that the TUI joins through `--remote` |
| **agmsgd** | beta in 1.6.0, the recommended choice at the prompt (`README.md:273`) | content-free notice; the model then runs `inbox.sh` itself | `codex queue --thread <uuid> --message <nudge>` (Codex's durable queue) |

Identity: one seat is `(team, agent)` plus a recorded Codex thread UUID and the effective `CODEX_HOME`. The model writes this record during `actas` by running `codex-record-session.sh <team> <name>`, which prefers `CODEX_THREAD_ID` from the tool shell and otherwise falls back to a rollout whose `session_meta` cwd matches the project, only when that match is unique (`codex-record-session.sh:24-33`). VERIFIED: `scripts/drivers/types/codex/template.md:29-32`, `scripts/drivers/types/codex/codex-record-session.sh:1-37`, `scripts/lib/role-session.sh:140,163,196`.

---

## 2. Injection mechanism

### 2.1 `turn` mode: Stop hook `decision:block` (the verified path)

**Hook entry.** `delivery.sh set turn codex <project>` writes the entry into the **project's** `.codex/hooks.json`, not into `$CODEX_HOME`. VERIFIED: `type.conf:122` (`hooks_file=.codex/hooks.json`), `scripts/delivery.sh:135-150,270-279`. The entry has the shape below (`scripts/lib/hooks-json.sh:180-188`, VERIFIED). There is no `timeout`, so Codex's 600 s default applies (primer §1.1). `commandWindows` is added because `hook_windows_wrap=yes` (`type.conf:149`).

```json
{"matcher":"","hooks":[{"type":"command","command":"'<skill>/scripts/check-inbox.sh' 'codex' '<project>'"}]}
```

**When it fires.** It fires at the end of every assistant turn, behind two guards:

- `stop_hook_active: true` in stdin makes the hook exit silently (`scripts/check-inbox.sh:76-79`). Mail cannot chain more than one continuation.
- A cooldown: if `run/.lastcheck-<AGENT>` was touched less than `delivery.turn.check_interval` ago (60 s by default), the hook prints the status JSON and exits (`check-inbox.sh:177-198`). VERIFIED.

**Code path.** All of it is in `check-inbox.sh`, VERIFIED:

1. Read stdin with a 2 s `timeout cat` (`:67-74`), and parse `session_id` by sed (`:86-95`).
2. Resolve the project and its identities. Only the **first** registered agent for `(project, type)` is used (`:129-163`).
3. Per team, skip the team when an actas lock is held by another session or its state is unknown (`:312,326`).
4. Read the unread rows through `storage_list_unread` and format them in an in-memory SQLite query (`:331-362`).
5. Build the text (`:372-380`). Newlines and tabs inside a body are escaped as the literal two-character sequences `\n` and `\t`, so each message stays on one line (`:355`).

```
<N> new message(s) in <team>:
  [<ts>] <from>: <body>
  ...
```

6. JSON-escape the text with sed/awk (`:440`) and emit:

```sh
printf '{\n  "decision": "block",\n  "reason": "%s"\n}\n' "$ESCAPED" || EMIT_RC=$?   # :462
```

7. Mark the displayed ids read, but only if the `printf` succeeded (`:502-511`, the #677 ordering). The comment states the limit plainly: a successful write "does NOT prove the hook runtime parsed the payload, or that the model received it" (`:490-496`).

**What the model sees.** In Codex, the `reason` of a Stop `decision:"block"` "becomes a new continuation prompt, treated as a new user prompt" (primer §1.5). The peer bodies therefore enter the context **as a user-role prompt**, and Codex runs one more sampling round. INFERRED from primer §1.5 together with `:462`.

**Quiet turns.** Codex has `stop_output=json` (`type.conf:124`). Every quiet or cooled-down Stop therefore prints `{"continue":true,"systemMessage":"agmsg: no new messages"}` or `"agmsg: check skipped (cooldown)"` (`check-inbox.sh:49-53,192,521`). In Codex, `systemMessage` is a UI warning and not model context (primer §1.4). INFERRED: this costs zero model tokens, but it shows a UI line at the end of every turn.

**Bounds.** None in agmsg. There is no per-message, per-team or per-payload byte cap anywhere in `check-inbox.sh`. The #777 comment covers only the argv limit on the SQL statement, not the output size (`:339-350`). VERIFIED by reading the whole file. What Codex does with a large Stop `reason` is **unknown**. The primer documents spill-to-disk above about 2,500 tokens for hook output generally (primer §1.4), but whether that applies to the Stop `reason` is not stated. INFERRED risk: a large backlog could reach the model only as a head-and-tail preview while every row is marked read.

**Dedupe and ack.** Read state is the only acknowledgement. Rows are marked through `storage_mark_read_batch`, which is recipient-scoped and idempotent and touches only the ids that were displayed (`:392-410`). A watcher racing this hook can produce a rare duplicate, and the code accepts that knowingly (`:97-123`). There is no ACK request, priority or deadline (overview).

### 2.2 `turn` mode: PostToolUse `additionalContext` (mid-turn, additive)

- **Installed only when** the type opts in (`posttooluse_output=hookSpecificOutput`, `type.conf:132`) **and** `codex --version` is at least `posttooluse_min_cli=0.149.1`. The check is fail-closed, so an unreadable version means the entry is not installed (`type.conf:133-148`, `delivery.sh:212-223,276-278`). VERIFIED. The matcher is `""`, so the hook runs after every tool (`hooks-json.sh:188`).
- **Payload**: `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"<same text>"}}` (`check-inbox.sh:448-451`). Quiet calls emit **no bytes** (`:50`).
- **Cooldown**: this hook has its own marker, `.lastcheck-<AGENT>.posttooluse`, so it cannot suppress Stop (`:170-181`). That bounds it to one store read per minute.
- **Never marks read** (`:398-410`). As a result, a message that arrives mid-turn is shown at the next tool boundary, shown again at the next PostToolUse a minute later if the turn is still running, and shown a third time in the Stop `reason`, which also costs a continuation round. The design accepts this: "A duplicate display is harmless; an invisible loss is not." VERIFIED intent. INFERRED cost: two or more copies of every body per long turn.
- **Not observed end-to-end.** The source says "Reaching the model with it is not yet observed end-to-end" (`type.conf:130-131`; `check-inbox.sh:171-172` calls it "the UNVERIFIED path"). The primer confirms that Codex treats PostToolUse `additionalContext` as model context (primer §1.4).

### 2.3 Monitor (legacy): app-server bridge `turn/start`

**Topology.** VERIFIED in `scripts/drivers/types/codex/codex-monitor.sh`:

- One app-server per **seat**, never shared: `codex app-server -c shell_environment_policy.set.AGMSG_CODEX_SEAT_KEY="…" --listen ws://127.0.0.1:0`. The script reads the port from the "listening on:" banner after stripping ANSI codes (`:160-257`).
- The TUI is then exec'd as `codex --remote ws://127.0.0.1:<port>` (`:366-373`).
- The doc still describes an app-server "socket" (`docs/codex-monitor-beta.md:124-126`), but the code says "codex 0.141+ accepts only ws:// (not unix://) for the TUI's --remote" (`codex-monitor.sh:183-185`). This is doc drift.
- Because of `--remote`, "an agent's shell_command runs inside the app-server process rather than the TUI client" (`_app-server.sh:19-20`, stated for codex 0.146). That app-server was started from the pane's own shell, so tool children inherit the pane's environment (INFERRED from `codex-monitor.sh:201-220`). This sidesteps the problem of the shared Codex daemon leaking another pane's environment (primer §2.1, §9).

**Bridge start.** Codex fires SessionStart on the session's **first turn** (`docs/codex-monitor-beta.md:128-130`). The hook does not open the socket. It writes a request file under `run/`. An out-of-sandbox `codex-bridge-launcher.sh` then starts `codex-bridge.js`, which finds the live thread through `thread/loaded/list` ("On codex 0.141+ the SessionStart hook cannot resolve the thread id", `codex-bridge-launcher.sh:15-22`). For `codex resume <id>`, the monitor script arms the request before Codex starts (#1401, `codex-monitor.sh:304-356`). VERIFIED.

**Detection.** The bridge spawns `watch-once.sh` *inside the app-server* through `process/spawn`, with `outputBytesCap: 8192` (`codex-bridge.js:1384-1406`). `watch-once.sh` polls the store every 2 s for up to 300 s and exits 0 (`status=pending max_id=…`), 2 (timeout, re-arm) or 1 (error). It never marks anything read (`watch-once.sh:1-18`; defaults at `codex-bridge.js:234-235`). VERIFIED.

**Injection.** On exit 0 the bridge calls `tryStartTurn` (`codex-bridge.js:1563-1614`). The launcher always passes `--inline-inbox` (`codex-bridge-launcher.sh:924`), so the bridge:

1. Re-resolves the eligible pairs (`eligible-pairs.sh`), then runs `inbox.sh <team> <name> --quiet` for each one. `inbox.sh` prints and **marks read** (`:1707-1732`; `scripts/inbox.sh:86-121`).
2. Sends:

```js
await this.client.request("turn/start", {
  threadId: this.threadId,
  input: [{ type: "text", text: prompt, text_elements: [] }],
  cwd: this.opts.project, runtimeWorkspaceRoots: this.opts.workspaceRoots });   // :1588-1593
```

The prompt is built at `:1686-1697`:

```
agmsg delivered the following unread messages for <team>/<name>:

<N> new message(s):

  [<ts>] <from>: <body>

Continue the conversation in this Codex thread. If a reply to an agmsg sender is needed, send it with:
<abs path>/send.sh <team> <name> <to> <message>
```

- **Bounds**: none on the prompt. The 8 KiB cap applies only to watch-once's stdout. VERIFIED.
- **Ack**: the rows are already read when `turn/start` goes out. If the request fails, only the wake is retried, not the payload (`:1577-1583`, VERIFIED). That leaves an at-most-once window for the bodies (INFERRED; the same conclusion as the overview).
- **Loop guard**: a repeated unchanged `max_id` makes the bridge exit (`staleWakeLimit` defaults to 1, `:1804-1822`).

### 2.4 agmsgd: content-free notice through `codex queue`

- **Daemon loop.** A single Node ≥ 22.13 service per install (`scripts/lib/daemon-state.sh:151`) polls every 5 s (`scripts/daemon/main.mjs:24,214-223`). The `channels/*.mjs` files cited below live under `scripts/daemon/channels/`. Seats are processed one after another, which "also serializes codex queue writes that share a CODEX_HOME, avoiding Codex's measured concurrent-write loss" (`channels/codex-queue.mjs:542-551`). VERIFIED.
- **Per-seat decision** (`codex-queue.mjs:291-515`): skip the seat if a bridge for it is running, if its role-session record (thread + `codex_home`) is missing, if the project opted out, if storage is not SQLite, if a notice is already pending for it, or if an earlier queue write for the same `CODEX_HOME` still has an unresolved child process (`shared_codex_home_queue_pending`, `codex-queue.mjs:249-256,314,452-454`). Otherwise `readUnreadSnapshot` checks whether any unread row lies **past the last confirmed `up_to` cursor** (`codex-queue-store.mjs:186-217`). If one does, the daemon creates a `pending` row in `run/install.db` (`scripts/daemon/db.mjs:1`) and runs:

```js
const args = ["queue", "--thread", thread, "--message", message];     // codex-queue-io.mjs:68
// env: { ...env, CODEX_HOME: codexHome }, no shell, 10 s timeout (codex-queue.mjs:33), process-group kill
```

- **Message**: `[agmsg:<uuidv4>] New messages are waiting. Check your agmsg inbox.` (`codex-queue-io.mjs:15-24`), 94 bytes. It carries no bodies and no count.
- **Success parse**: the daemon matches `^Queued message <uuid> for thread <uuid>`. Exit errors `-32600` and `-32603` map to `archived` and `no_rollout` (`codex-queue-io.mjs:138-149`).
- **Confirmation** (`codex-queue-io.mjs:152-338`). The notice counts as confirmed when either:
  - the item id is still present in `$CODEX_HOME/queue_1.sqlite` `queued_items` (read-only open), or
  - the nonce appears in a `user_message` or user `response_item` row of the one `sessions/**/rollout-*-<thread>.jsonl`.

  If both are readable and absent for 60 s, the notice expires (`QUEUE_CONFIRMATION_TTL_MS`, `codex-queue-io.mjs:14,283-305`) and the next poll may queue a new one. Any unreadable observation keeps it **pending**, which is fail-closed: no re-send.
- **"Noticed" is not "read".** Confirmation advances the per-seat `up_to` cursor. Messages at or below it never trigger another notice, whether or not the model ever ran `inbox` (`codex-queue.mjs:89-95,437`). VERIFIED.
- **Bodies**: they arrive only when the model, prompted by the notice and the skill, runs `inbox.sh`, which prints `N new message(s):` and one line per message and then marks read (`scripts/inbox.sh:86-121`).

---

## 3. Wake mechanism

### 3.1 `turn` mode

There is no wake (`README.md:219,275`, VERIFIED). The cooldown makes this worse (INFERRED from `check-inbox.sh:177-198`). Suppose a message arrives and the agent ends a turn less than 60 s after its last Stop check. That Stop is skipped, the session goes idle, and the message waits for the next human prompt. The marker is keyed by agent name only (`run/.lastcheck-<AGENT>`, `:180`), so sessions that share an agent name across teams or projects in one install share a cooldown.

### 3.2 Monitor (bridge)

- **Mechanism**: an app-server `turn/start` on the seat's thread. It types no keystrokes (VERIFIED, `codex-bridge.js:1588`).
- **Idle/busy**: the bridge tracks the thread's state from `thread/status/changed` (`active` or `idle`), `turn/started`, `turn/completed` and `turn/failed`. A wake that arrives while a turn is active is held in `pendingWake` and delivered when that turn ends (`:1031-1054,1473-1561`). The app-server "does not reliably send turn/completed" (#41), so a 60 s silence watchdog, re-armed by **any** thread-scoped notification, closes a turn that never reports completion (`:1616-1639`, `--turn-timeout` default 60, `:104-109,244`). VERIFIED.
- **Draft safety**: no composer read happens or is needed. `turn/start` runs server-side, beside whatever is typed in the TUI composer (INFERRED, consistent with primer §3). Whether the `--remote` TUI keeps a half-typed draft when a server-started turn begins is **unknown**.
- **Latency**: up to the 2 s watch-once poll, plus the time `turn/start` takes, plus the end of any active turn (INFERRED from the defaults).
- **Failure modes** (VERIFIED unless marked):
  - The bridge does not exist until the first human turn after a restart, because SessionStart fires on the first turn (`docs/codex-monitor-beta.md:20-26,128-130`).
  - Bridges orphan when the TUI closes (#149).
  - The bridge stops after 3 consecutive watch-once failures (`:1446-1455`) or one stale wake (`:1804-1822`). The launcher restarts it.
  - If the app-server fails to start, an interactive launch fails open to plain Codex with a stderr banner. A spawned seat fails closed (`codex-monitor.sh:133-158`).
  - **The bridge auto-declines every approval, permission and elicitation request it receives**, which keeps the headless bridge from wedging the thread (#299, `codex-bridge.js:1055-1067,1655-1674`). INFERRED risk: the bridge and the human's TUI are two clients on one app-server. If the server routes a server→client approval request to the bridge connection, a prompt the human should answer is silently declined. Which connection receives it is **unknown** (Codex routing not traced).
  - INFERRED: the per-seat app-server listens on an unauthenticated loopback WebSocket (no `--ws-auth`, `codex-monitor.sh:218-220`). Any local process can drive that seat's thread. The primer calls WebSocket "experimental and unsupported" (§2.1).

### 3.3 agmsgd (`codex queue`)

- **Mechanism**: Codex's durable queue. `thread/queue/add` persists the item and calls `wake_if_loaded`. The dispatcher starts a turn only through `start_turn_if_idle` with `turn_trigger:"queue"` (primer §3). agmsg itself does no idle detection. VERIFIED: agmsg leaves this to Codex (`codex-queue-io.mjs:68`). The arbitration details are VERIFIED in the primer.
- **Busy**: the queued item waits until the running turn ends. Nothing reaches the model mid-turn, and agmsgd installs no hooks for in-turn delivery ("mode: agmsgd … no hooks file needed", `_delivery.sh:209-243`).
- **Draft safety**: inherent. Nothing is typed into the composer (primer §3).
- **Latency**: the daemon poll (≤ 5 s) plus the `codex queue` spawn. Dispatch is immediate when the thread is loaded on the same server, and takes up to about 20 s across processes (primer §3, INFERRED there).
- **Failure modes**:
  - **Interrupted thread** (INFERRED from primer §3 together with `codex-queue-io.mjs:283-289`). Codex does not dispatch after an *interrupted* turn. The item stays in `queue_1.sqlite`, agmsgd reads "present" and marks the notice **confirmed**, and the `up_to` cursor advances. A human Ctrl-C therefore suppresses the wake until the next completed turn, and agmsgd never re-notifies for those messages.
  - **Confirmation means "accepted by Codex's queue", not "seen by the model"**: queue-row presence alone confirms (`:284-286`). The rollout-nonce check is stronger evidence, but it only matters once Codex has removed the queue row.
  - **Shared daemon environment.** Under agmsgd the monitor script execs plain Codex with no `--remote` and no `--no-daemon` (`codex-monitor.sh:112-120`); that this attaches to Codex's shared daemon is INFERRED from primer §2.1, and agmsg's own comment says the SessionStart hook "runs under the profile's shared app-server daemon" (`_session-start.sh:113-116`). agmsg handles this by refusing to pick a seat when there are several registrations (`_session-start.sh:110-117`, citing #1112 and #1468). VERIFIED.
  - **`--no-daemon`.** agmsg adds `--no-daemon` only on elevated Windows shells, in the shim's plain launch and the monitor's fail-open fallback (`codex-shim.sh:165-182`, `codex-monitor.sh:146-153`). Whether `codex queue` reaches a `--no-daemon` TUI is contested (primer §3). agmsg does not address that case (INFERRED).
  - **Seat binding.** `codex queue` needs the exact thread UUID and `CODEX_HOME`. Both come from `codex-record-session.sh`, run by the model at `actas`, or from a SessionStart refresh when the project has exactly one registration (`_session-start.sh:110-138`). An unseated session gets the model-visible line `AGMSG delivery: this session is not connected to a seat yet. Run $agmsg actas <name> in Codex to receive messages.` (`:140-143`).
  - Ephemeral threads such as `/side` cannot be queue targets (primer §3). agmsg does not mention them (unknown whether it handles them).
  - Windows needs Codex ≥ 0.157 (`README.md:157`, `windows-codex-queue.mjs`). This is a version gate.

### 3.4 Manual `poke` (any mode)

`poke.sh` → `safe-poke` takes two input-box snapshots about 1 s apart. For Codex it requires the `›` marker line, a blank line, and a footer containing `·`, and it refuses with code 14 on a real draft or 15 when the box cannot be located (`type.conf:151-160`, `scripts/lib/safe-poke.sh:33-36,74-83`). Under Herdr only (#1384), a stationary draft in an **unfocused** pane is not refused: agmsg saves it to a file under `run/`, clears the box, pokes, and retypes the draft only after the box reads back empty, verifying the restore byte-for-byte (`safe-poke.sh:90-120,160-175`, VERIFIED). A focused pane still gets 14. Under Herdr it then calls `herdr agent prompt <pane> <text>`, which is "ASSERTED, NOT yet measured" (`scripts/drivers/terminals/herdr/ops.sh:30,1084-1102`). VERIFIED. This is an agent-invoked fallback, not part of the automatic paths.

---

## 4. Setup and installation footprint on the harness

| What | Where | When | Evidence |
|---|---|---|---|
| Skill (`SKILL.md` 48,607 B rendered, Codex-typed by default; `agents/openai.yaml`) | `~/.agents/skills/<cmd>/` | `install.sh` | `install.sh:5,1311-1317`, `README.md:601-603` |
| `[sandbox_workspace_write] writable_roots += <skill>/{db,teams,run,ext-tools}` (backs up to `config.toml.bak`) | `~/.codex/config.toml`, and also `$CODEX_HOME/config.toml` when it differs; **only if the file already exists** | `install.sh`, `--update` | `install.sh:628-740`, `scripts/lib/codex-config.sh:22-52` |
| Hook entries in `turn`: Stop (+ PostToolUse when CLI ≥ 0.149.1). In `monitor`/`both`: SessionStart + SessionEnd (+ Stop and the gated PostToolUse in `both`) | **project** `.codex/hooks.json` | `delivery.sh set <mode>`, and `codex-monitor.sh` on every launch unless the file matches a stored marker | `delivery.sh:244-316`, `codex-monitor.sh:273-297` |
| `codex()` shell function (printed for the user to paste) or the optional PATH shim | user's shell profile / `~/.agents/bin/codex` | `delivery.sh set monitor` | `_delivery.sh:12-36`, `docs/codex-monitor-beta.md:53-96` |
| Login service (launchd / `systemd --user` / Task Scheduler) | OS | `agmsg daemon enable` | `CHANGELOG.md:10` |
| Hook trust | not handled | n/a | INFERRED from absence: no `trust`/`/hooks` handling in the Codex driver or `delivery.sh` (grep). Codex runs non-managed hooks only after the user trusts them by hash, and project hooks only when the project `.codex/` layer is trusted (primer §1.1). `delivery.sh` avoids needless rewrites, which would change the hash and the file mode (#1429, `delivery.sh:318-339`) |

---

## 5. Token cost

The numbers are byte counts read from the code. Token counts are INFERRED at roughly 4 bytes per token.

| Path | Per message delivered | Per quiet turn |
|---|---|---|
| `turn` Stop | One header line per team (`N new message(s) in <team>:`, about 25 B) + `  [<ts>] <from>: <body>` per message (about 30 B + body). Plus **one extra sampling round**, because the reason is a continuation prompt. | 0 model bytes. A UI `systemMessage` line on every turn end (`check-inbox.sh:521`). |
| `turn` PostToolUse | The same text again at each tool boundary, at most once a minute, until Stop marks it read. Typically one or two duplicates per body in a long turn. | 0 bytes (`:50`) |
| Monitor bridge | About 150 B of framing + one absolute `send.sh` path (about 60 B) + the inbox text (`codex-bridge.js:1686-1697`; the two-path variant is the non-inline prompt, which the launcher does not use). It costs one full turn. | 0. No turn is started when the inbox is empty (`codex-bridge.js:1565-1572`). |
| agmsgd | About 94 B notice as a user turn → the model's tool call to `inbox.sh` → the inbox output. That is **two sampling rounds** per notice: one to read the notice, one after the tool result. | 0 |
| Static | The skill body loads when invoked. Its size is large: `scripts/skill-base.md` 30,951 B + `template.md` 7,200 B render into a 48,607 B `SKILL.md`. SessionStart plain-stdout lines (the writable-roots notice, the unseated notice) are model context in Codex (primer §1.4). | n/a |

---

## 6. Limitations and bugs visible in the code

1. **No idle wake in `turn` mode**, and the 60 s cooldown can strand mail that arrives just before a turn ends (§3.1). The cooldown key is per agent name, not per session (`check-inbox.sh:178-180`).
2. **No size bound** on the Stop `reason`, the PostToolUse context or the bridge prompt. Codex's handling of large hook text could truncate a payload whose rows are already marked read (§2.1, INFERRED).
3. **Peer bodies enter as user-role prompts** (Stop `reason` and bridge `turn/start` both). The framing is minimal (`[ts] from: body`) and sets no trust boundary for content from another agent (INFERRED from primer §1.5).
4. **PostToolUse delivery is admittedly unverified** end-to-end and duplicates bodies by design (`type.conf:130-131`, `check-inbox.sh:398-410`).
5. **Version-floor gates**: PostToolUse is installed only at CLI ≥ 0.149.1 (fail-closed when the version is unreadable). agmsgd needs Codex ≥ 0.157 on Windows. Herdr-threads' own policy forbids gating a feature on a version (AGENTS.md "Harness and Herdr versions"); agmsg's are minimum floors, not exact pins, and `type.conf:139-147` records that an older CLI silently ignores the entry anyway.
6. **Bridge marks read before `turn/start` succeeds**, which leaves an at-most-once window for the bodies (`codex-bridge.js:1577-1583`).
7. **Bridge auto-declines approvals** on a shared app-server (INFERRED risk, §3.2). The **unauthenticated loopback WebSocket** app-server is a second INFERRED risk.
8. **Monitor needs one human turn** after each restart (SessionStart fires on the first turn). Bridges orphan (#149). The doc says "socket" while the code uses a loopback ws:// port (doc drift).
9. **agmsgd "noticed" cursor**: once a notice is confirmed, those messages never trigger another notice, even if the model ignored the notice or the turn was interrupted. Queue-row presence counts as confirmation, though it is not delivery (§3.3).
10. **Only the first registered agent** per `(project, type)` is served by the Stop hook (`check-inbox.sh:131-163`; the "Codex caveat", `docs/actas.md:64-68`, adds that `actas` does not narrow which pair turn mode polls).
11. **Hook trust is not handled.** A fresh `.codex/hooks.json` in an untrusted project silently does nothing until the user trusts it in `/hooks` (INFERRED, §4).
12. **Sandbox writable roots** are added only if `config.toml` already exists at install time. If Codex is installed later, the user gets a notice instead (`codex-config.sh:54-103`).

---

## 7. Ideas worth stealing (for herdr-threads)

Each idea below cites the agmsg evidence and the corresponding herdr-threads behaviour in primer §10.

1. **`codex queue` for content-free wakes** (agmsgd). It types nothing, reads no composer, cannot clobber a draft, and Codex arbitrates idle vs busy on the server side. Our current wake is Herdr `agent.prompt` gated on `agent_status` plus a composer read that must be `Empty` (primer §10). A queued `herdr-threads: wake up` would keep rule (c) of our token-efficiency rules (content-free wake prompts). Two caveats before adopting it: the behaviour after an interrupted turn, and reachability under our `--no-daemon` advice (primer §3, §11.1).
2. **Delivery confirmation by nonce**: put a short random nonce in the wake text, then confirm it in `queue_1.sqlite` or in the rollout's user rows (`codex-queue-io.mjs:152-277`). We could use the same check to tell "wake typed" from "wake reached the model", and expire and retry after a TTL. The tri-state handling (present / absent / unreadable, with unreadable never treated as absent) is worth copying as well.
3. **Serialize queue writes per `CODEX_HOME`**: agmsg reports a measured concurrent-write loss (`codex-queue.mjs:542-551`). Any adoption of `codex queue` on our side should do the same.
4. **Per-seat app-server whose tool children inherit the pane's environment**: `codex app-server … -c shell_environment_policy.set.<VAR>=…` with `--remote` from the pane (`codex-monitor.sh:201-220`). It solves the shared-daemon identity leak that makes us advise `--no-daemon`, and it gives a client `thread/status/changed`, `turn/steer` and `thread/inject_items` (primer §11.2-3). The cost is a shim around `codex` and a resident bridge. agmsg itself now calls this path "legacy".
5. **Watchdog re-armed by any thread-scoped notification** for app-server clients, because `turn/completed` is unreliable (#41, `codex-bridge.js:1616-1639`). This is useful if we ever track busy/idle through the app-server.
6. **Emit, then mark**: consume read state only after the stdout write succeeds, and state in the code exactly what that write proves (`check-inbox.sh:475-511`). Our hook path ACKs on `inbox` display instead, but the same ordering applies to any hook that carries bodies.
7. **Stop `decision:block` as end-of-turn delivery** (primer §11.5). agmsg proves it works on Codex. Its weaknesses (unbounded size, user-role framing, a wasted continuation on low-priority mail) suggest using it only for pending p0 items, with our 4,608-byte cap.
8. **PostToolUse with matcher `""`** covers every tool. Our `PreToolUse ^Bash$` misses turns that only use `apply_patch` or MCP tools (primer §11.4). agmsg's per-minute cooldown and "additive, never consumes" stance are a reasonable model if we add a second tool-boundary hook.
9. **SessionStart auto-rebind for a sole registration**: after a restart, record the new thread plus `CODEX_HOME` only when exactly one seat could match, and otherwise say "unseated" (`_session-start.sh:110-138`). This fits our "never bind on heuristic evidence" rule because it refuses whenever the match is ambiguous.

---

## Verification

Checked against the clone at `6e03154` and `gh api repos/fujibee/agmsg` (1,564 stars, pushed 2026-10-23T05:39Z, latest release v1.6.0 published 2026-10-22). Re-read every cited range in `type.conf`, `README.md`, `docs/codex-monitor-beta.md`, `check-inbox.sh` (whole file), `delivery.sh`, `hooks-json.sh`, `codex-monitor.sh`, `codex-bridge.js`, `codex-bridge-launcher.sh`, `watch-once.sh`, `inbox.sh`, `_session-start.sh`, `_app-server.sh`, `_delivery.sh`, `codex-record-session.sh`, the daemon `main.mjs` and `channels/codex-queue{,-io,-store}.mjs`, `install.sh`, `codex-config.sh`, `safe-poke.sh` and `herdr/ops.sh`, and primer §1.1, §1.4, §1.5, §2.1, §3. The Stop/PostToolUse code path, cooldowns, emit-then-mark ordering, bridge `turn/start` flow and mark-before-start window, watchdog, auto-decline, queue nonce and tri-state confirmation, and skill sizes all held.

Changed:
- Identity: added the unique-cwd rollout fallback in `codex-record-session.sh`.
- Shared-daemon claim under agmsgd: the cited lines show a plain exec only; the daemon attachment is now marked INFERRED, with agmsg's own comment cited.
- agmsgd skip rule: "another pending item shares `CODEX_HOME`" narrowed to "an earlier write has an unresolved child process".
- Bridge token cost: inline prompts carry one script path, not two.
- Poke: added the Herdr-only save/clear/poke/restore path for unfocused panes, which the report had missed; it is directly relevant to our composer-`Empty` rule.
- "Exact-version gates" corrected to minimum-version floors.
- Citation fixes: daemon channel paths, nonce lines, TTL lines, the `turn/failed` handler, `--no-daemon` sites, the "Codex caveat" source (`docs/actas.md`, not the README), and the doc-drift wording (the doc says "socket", not "unix socket").
