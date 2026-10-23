# agmsg × Claude Code: message injection and wake

- Upstream: https://github.com/fujibee/agmsg. Paths are relative to the clone root. The clone is shallow, at HEAD `6e031546` (2026-10-23, "release: 1.6.0 (#1727)"). No code from it was executed.
- Context: [overview.md](overview.md) covers agmsg in general and [../../primers/claude.md](../../primers/claude.md) covers the Claude Code surfaces. Section references such as "§3" point into the primer.
- Tags: **VERIFIED** means read in source or docs (cited). **INFERRED** means my reasoning, not confirmed by source or a run. **unknown** means not found.

## 1. Support status

Claude Code is agmsg's first-class harness. The `claude-code` type manifest declares `cli=claude`, `spawnable=yes`, `delivery_modes=monitor turn both off`, `hooks_file=.claude/settings.local.json`, session detection by `CLAUDE_CODE_SESSION_ID`, and draft-check data (`input_prompt_marker=❯`, `input_prompt_boxed=yes`). VERIFIED `scripts/drivers/types/claude-code/type.conf:2-54`.

There are three delivery paths for Claude, plus two manual ones:

| Path | Injection | Idle wake | Status | Evidence |
|---|---|---|---|---|
| `monitor` (shown as "legacy" since 1.6.0, still the default when agmsgd is not chosen) | A `SessionStart` directive makes the model start the **Monitor tool** on `watch.sh`. Each stdout line becomes a Monitor event. | yes, through the Monitor event (with caveats, §3.1) | shipped | VERIFIED `README.md:274`, `scripts/session-start.sh:449-573` |
| `turn` | A **Stop hook** prints `{"decision":"block","reason":<messages>}` | **no** | shipped | VERIFIED `scripts/delivery.sh:270-279`, `scripts/check-inbox.sh:462` |
| `both` | Monitor plus the Stop hook as a safety net | through the Monitor only | shipped | VERIFIED `scripts/delivery.sh:280-290` |
| **agmsgd** (beta, recommended in 1.6.0) | A content-free notice is written into **Claude Code's own cross-session messaging socket** (`CLAUDE_CODE_MESSAGING_SOCKET`). The model then runs `inbox.sh` itself. | yes, through Claude Code's native "idle → new turn" behaviour | beta | VERIFIED `scripts/daemon/channels/claude-code-queue*.mjs`, `README.md:157,273` |
| `off` / manual | The model or user runs `/agmsg` → `inbox.sh` | no | shipped | VERIFIED `README.md:277` |
| `poke` (fallback, agent-invoked) | Types into another seat's pane (`herdr agent prompt` under Herdr, tmux `send-keys`), behind a draft guard | yes (keystrokes) | shipped | VERIFIED `scripts/poke.sh`, `scripts/lib/safe-poke.sh`, `scripts/drivers/terminals/herdr/ops.sh:1084-1102` |

Claude gets **no PostToolUse hook**. Only a type whose manifest carries `posttooluse_output` gets one (today: codex), and `claude-code`'s manifest has no such key. So in `turn` mode a Claude seat sees mail only at turn end. VERIFIED `scripts/delivery.sh:185-223`, `type.conf` (key absent).

Distribution: npm `npx agmsg`, `setup.sh`, or the Claude Code plugin marketplace (`.claude-plugin/marketplace.json`). The plugin ships no `hooks/` directory, so hooks always come from `agmsg delivery …` writing the project's settings file (VERIFIED: `.claude-plugin/plugin.json` and the repo root have no hooks dir). Stars and releases: see overview.md (1,564 stars, `v1.6.0` released 2026-10-22).

---

## 2. Injection mechanisms

### 2.1 Monitor stream (`monitor` / `both`)

**Arming: a model-obeyed directive, not a harness API.** `SessionStart` runs `session-start.sh <type> <project>` (`scripts/delivery.sh:264-268`). After the identity, worktree and duplicate-watcher checks, it prints plain stdout (which becomes SessionStart context, primer §1.2), telling the model to call Monitor itself. VERIFIED `scripts/session-start.sh:533-572`:

```
AGMSG monitor mode: invoke the Monitor tool now with the following parameters,
before any other action in this session.

  command: <watch.sh> <instance-id> <project> claude-code --max-seconds=1750
  description: agmsg inbox stream
  persistent: true
  timeout_ms: 1800000
...
Use Claude Code's generic Monitor tool only. First confirm ToolSearch
select:Monitor finds that generic tool; ...
```

- A resumed seat whose role is known (a role-session record, or an actas lock owned by this sid) gets a variant with a 4th argument `<role>`. The 4th argument narrows the subscription and re-claims the actas lock. VERIFIED `:368-421`, `:467-518`.
- A project with several seats and no record for this session gets a "standing down" text instead. No watcher is started, because an unfiltered watcher would consume other seats' mail. VERIFIED `:574-601`.
- Before the directive, a duplicate watcher left by `/clear` is killed. This is tracked through `run/cc-instance.<claude_pid>`. VERIFIED `:161-287`.
- Worktree sub-sessions (`.claude/worktrees/*`) are skipped (#367), because a persistent Monitor would keep the sub-session alive past its task. VERIFIED `:141-151`.

**Streaming.** `watch.sh` polls SQLite every `delivery.monitor.poll_interval` seconds (default 5). VERIFIED `scripts/watch.sh:29-31,493,1815`. Each unread message is one stdout line, and the Monitor tool turns each line into a notification. VERIFIED `scripts/watch.sh:1681`:

```bash
if ! printf '%s | %s | %s → %s | %s\n' "$ts" "$team" "$from" "$to" "$body"; then
  cleanup; exit 0
fi
```

- Newlines and tabs in the body are escaped to `\n`/`\t`, and CR is dropped (`watch.sh:1623`), so one message is one line.
- There is **no size bound** on a line: no truncation and no paging in `watch.sh`, and its storage query `storage_watch_after` has no `LIMIT` (`scripts/drivers/storage/sqlite.sh:529-554`). VERIFIED (absence). How Claude Code's Monitor handles a very long line is unknown.
- `ctrl:despawn` bodies are consumed and acted on (seat teardown), never shown. VERIFIED `watch.sh:1664-1679,1743-1767`.

**Dedupe and read-marking.** The watcher advances the shared per-(team, agent) read cursor and inserts `message_read` events. It does this only for ids whose `printf` succeeded, and only after re-checking pair ownership, both right before delivering and right before consuming. VERIFIED `watch.sh:1642-1647,1704-1726`. `inbox.sh` and the Stop hook read the same state, so `both` mode does not double-deliver, apart from a one-poll interleave race the authors explicitly accept. VERIFIED `scripts/check-inbox.sh:97-123`. "Read" means "written to the Monitor's stdout", not "the model saw it".

**Renewal (the 30-minute Monitor cap).** The watch is launched with `--max-seconds=1750`. At the top of a cycle past that point, it prints one line and exits. VERIFIED `watch.sh:1287-1306`:

- if it delivered anything this run, or `AGMSG_CC_MONITOR_KEEP_ALIVE` is set: `agmsg watch: re-arm - … invoke the Monitor tool again with command: … persistent: true timeout_ms: 1800000`. The model is told to re-invoke silently, "no message to the user, no re-armed … since announcing it every 30 minutes wastes tokens" (`session-start.sh:459-460`);
- otherwise: `agmsg watch: stopping - no messages were delivered in the last 1750s, so this watch is not re-armed`;
- if agmsgd has taken over this seat: `stopping - this seat's delivery has moved to agmsgd's native channel` (`watch.sh:1247-1293`).

The "stop when quiet" default is deliberate: re-arm became opt-in in 1.3.x (#1287, #1315). VERIFIED `CHANGELOG.md:222`. See §6 for the consequence.

### 2.2 Stop hook (`turn` / `both`)

**Hook.** `settings.local.json` gets a `Stop` entry `check-inbox.sh 'claude-code' '<project>'`. VERIFIED `scripts/delivery.sh:270-272`.

**Code path** (`scripts/check-inbox.sh`):

1. It reads hook stdin with a 2 s `timeout cat` (`:67-74`). If `"stop_hook_active": true`, it exits silently (`:77-79`). This is the loop guard: a block is never chained.
2. It resolves the project (`:129`) and the identity, choosing **the first registered agent for (project, type)** rather than the session's in-memory `actas` role (`:134-163`, comment `:309-311`). It then checks the actas lock state and skips any team whose role is held by another live session or is unknown (`:312,326`).
3. Cooldown: if the marker `run/.lastcheck-<AGENT>` is younger than `delivery.turn.check_interval` (default 60 s), it exits silently. Otherwise it **touches the marker before querying** (`:177-198`).
4. Per team it lists unread rows through the storage facade and formats them (`:331-379`). `storage_list_unread` accepts an optional `--limit` (`sqlite.sh:456-500`), but this call passes none:

```
<N> new message(s) in <team>:
  [<ts>] <from>: <body with \n escaped>
```

5. It emits Claude's block shape with the shell builtin `printf`, so a closed stdout counts as a failed delivery. VERIFIED `:462`:

```bash
printf '{\n  "decision": "block",\n  "reason": "%s"\n}\n' "$ESCAPED" || EMIT_RC=$?
```

6. It marks the displayed ids read **only after the write returned success** (#677), accepting duplicates over loss: "a duplicate is visible and a silence is not". VERIFIED `:475-511`. The code says plainly that this proves the write succeeded, not that the model saw the text (`:490-496`).
7. On a quiet turn Claude gets **zero bytes**, because `stop_output` is unset for `claude-code`. Codex and Copilot get a JSON status (`:30-53`). VERIFIED.

**When it fires.** Only at the end of an assistant turn. `decision:block` makes Claude continue with `reason` fed back (primer §1.3). The result is one extra model call per delivered batch, inside the turn that was ending. Mid-turn delivery: none.

**Bounds.** None in agmsg. Claude Code caps hook output fields at 10,000 chars, with a spill file and a 2,000-char preview (primer §1.2). Whether the Stop `reason` field gets that cap is unknown. INFERRED: a large backlog is either cut to a preview Claude is not told to read, while agmsg has already marked every id read, or passed through whole.

A documented-contract aside the authors measured: the stdout control JSON was processed on exit 0, 1, 2 and 3 alike (Claude Code 2.1.226, `-p`). That is why the hook always exits 0 when it delivers. VERIFIED `check-inbox.sh:413-433`.

### 2.3 agmsgd notice over the native cross-session socket (beta)

**Destination publication** (SessionStart, `scripts/drivers/types/claude-code/_session-start.sh`):

- It reads `CLAUDE_CODE_MESSAGING_SOCKET` (strips `uds:`; also accepts a Windows named pipe) and `CLAUDE_CONFIG_DIR`, falling back to `~/.claude` only when unset. Control characters and relative paths are refused. VERIFIED `scripts/lib/role-session.sh:574-596`.
- It decides the seat: a role-session record or an owned actas lock, else the only registered pair. It refuses on ambiguity, and refuses when the pid cannot be resolved. VERIFIED `_session-start.sh:76-130`.
- It claims the actas lock with an exact-token match, and reclaims only from a **dead** owner (`:132-148`). A missing record is created only after the claim (`:150-160`).
- It publishes `messaging_socket`, `claude_config_dir`, `session` and `messaging_token` into the role-session record, written mode 600 (`role-session.sh:256-282`), and then **reads it back** (`role-session.sh:601-607`).
- On success it prints one line, `AGMSG delivery: agmsgd is running and will deliver to this session directly (native channel). No Monitor needed.`, and `exit 0`, skipping the Monitor directive. Most failure paths print one line ending in a "Next: … arm the Monitor below" hint (one adds "then run agmsg doctor"); a malformed destination, a daemon that is not ready, or an empty session id returns silently. Every failure falls through to §2.1. VERIFIED `_session-start.sh:29,52,79,161-171`.
- The authors measured that the socket path **changes on every `--resume`** (new process) but not on `/clear` or `/compact`, so the record is rewritten at every SessionStart. VERIFIED (comment) `_session-start.sh:10-15`.

**Daemon loop.** `agmsgd` is Node ≥ 22.13, one per install, a login service (systemd user unit `~/.config/systemd/user/agmsgd-<id>.service` or a launchd plist). It polls every 5 s. VERIFIED `scripts/daemon/main.mjs:24,223`, `scripts/daemon.sh:259,306`. Per registered Claude seat (`claude-code-queue.mjs:164-350`):

- It skips seats whose project opted out of agmsgd, has a mismatched record, a missing socket, a non-sqlite store, and so on, and records a reason for `agmsg daemon status`.
- It asks "is there any unread message above the tip recorded at the last **confirmed** notice?" (`readUnreadSnapshot`, `codex-queue-store.mjs:186-216`). This is a boolean plus a new tip; no bodies are read.
- It allows **at most one outstanding notice per seat**, never re-sent on a timer ("Claude Code holds a message received mid-work until the work pauses, so a long wait is normal, not a loss", `:181-185`; `createPending` throws `seat_already_pending`, `:115-116`).
- It re-sends only when the socket is gone or the record moved (`/compact`, restart, `--resume`), or when the notice was **held** and the other permission-mode claim has not been tried (`:203-234`).

**Wire format.** This answers the primer's open question §10.1, as measured by agmsg on 2026-10-05 (VERIFIED as implemented, `scripts/daemon/channels/claude-code-queue-io.mjs:127-182`). One connection, an optional auth line, then one JSON line, and the socket is closed with `end()`:

```js
const payload = {
  msgV: 1,
  msg_id: randomUUID(),
  type: "user",
  message: {
    role: "user",
    content: `<cross-session-message from="agmsgd"${fromMode === "bypass" ? ' from-name="agmsgd" from-mode="bypass"' : ""}>\n${body}\n</cross-session-message>`,
  },
  priority: "next",
  from: "agmsgd",
};
const auth = token ? `${JSON.stringify({ type: "auth", token })}\n` : "";
```

- Body: `[agmsg:<uuid>] New messages are waiting. Check your agmsg inbox.` VERIFIED `codex-queue-io.mjs:20-25`. About 75 bytes of body; the whole `content` with its wrapper is about 135 bytes.
- The **sender** writes the `<cross-session-message …>` wrapper, including a self-declared `from-mode="bypass"`. The mode is chosen by reading the receiver's transcript tail for `"permissionMode":"bypassPermissions"` (`claude-code-queue-io.mjs:188-216`, `claude-code-queue.mjs:322`). INFERRED: Claude Code's hold policy ("a bypass receiver holds unless the sender also bypasses", primer §3) is judged from this sender-supplied attribute. That is why agmsg retries once with the other claim after a hold (`claude-code-queue.mjs:214-226`).
- Before writing, it refuses anything that is not a socket owned by the same uid, and refuses symlinks (`claude-code-queue-io.mjs:102-121`).
- `priority:"next"`: exact semantics unknown. INFERRED: "deliver at the next break, not as an interrupt".

**Delivery confirmation by transcript nonce.** Before sending, agmsg records the transcript's byte length. Afterwards it scans only what was appended past that point (256 KiB chunks), and a line counts only if it reads back byte-identical (rewrite safety). VERIFIED `claude-code-queue-io.mjs:221-362`. Verdicts (`judgeLine`, `:242-268`):

- `type:"user"` row whose `origin.body` contains `[agmsg:<nonce>]` → **delivered** (idle path: a new turn started with it);
- `type:"attachment"`, `attachment.type:"queued_command"`, prompt contains the nonce → **delivered** (absorbed into a running turn; measured 2026-10-15);
- `type:"system"`, `subtype:"informational"` with the unbracketed nonce in a preview → **held** (permission dialog / `crossSessionInbound`);
- `queue-operation enqueue` is deliberately **not** evidence, since it fires on receipt even when the message is held.

The transcript path is `<CLAUDE_CONFIG_DIR>/projects/<recorded project path with / → ->/<sid>.jsonl` (other characters on win32, `:35-38`). If that file is missing, a unique match under `projects/*/` is used instead (`:53-82`).

**After the notice.** agmsgd never sends bodies and never marks anything read. The model has to act on the notice by running `inbox.sh` through Bash, which prints `N new message(s):` plus `  [ts] from: body` lines and marks the displayed ids read. That output is also unbounded (`storage_list_unread` is called without `--limit`, `inbox.sh:45`). VERIFIED `scripts/inbox.sh:45,84-124`. INFERRED: a Bash tool result is subject to Claude Code's own output truncation, so a huge backlog can be marked read while only partly shown.

### 2.4 Framing summary

| Path | What the model reads | Static explanation |
|---|---|---|
| Monitor | `<ts> \| <team> \| <from> → <to> \| <body>` inside Claude's Monitor-event framing | about 2.7 KB of directive at SessionStart (directive 1.7 KB + re-arm note 0.65 KB + team line 0.28 KB + terminal line), plus the re-arm line every ~29 min |
| Stop | `N new message(s) in <team>:` + `  [ts] from: body` as the block `reason` | none per turn; the `/agmsg` command doc when loaded |
| agmsgd | `<cross-session-message from="agmsgd">[agmsg:<uuid>] New messages are waiting. Check your agmsg inbox.</cross-session-message>`, then the `inbox.sh` output | one line at SessionStart |

There are no ids in the framing (the message id is not shown), no priorities and no deadlines. VERIFIED (formats above, and overview "Ordering").

---

## 3. Wake mechanisms

### 3.1 Monitor event (`monitor` / `both`)

- **Mechanism.** A Monitor event on an idle session makes Claude "interject" (primer §8). agmsg calls this "real-time push", ~5 s latency. VERIFIED `README.md:274`. Wake-on-idle relies on Claude Code's Monitor semantics (INFERRED in the primer; agmsg's README treats it as working).
- **Idle/busy detection.** None in agmsg. Claude Code decides when an event is consumed. INFERRED: busy sessions receive Monitor events between tool calls or at turn end, as the host decides.
- **Draft safety.** None in agmsg. A Monitor event does not touch the composer. INFERRED: if Claude starts a turn while the user has a draft, the draft stays, because Monitor is not keystroke injection.
- **Latency.** Poll interval 5 s plus host scheduling.
- **Failure modes** (VERIFIED unless marked):
  - **Priming.** "the receiving agent doesn't react to its first inbound message until it has taken at least one turn this session", so you have to say "hi" first (`README.md:294`). INFERRED cause: the Monitor is started by the model obeying the SessionStart directive, which only happens once the model takes its first turn. Until then nothing is watching.
  - **Self-expiry when quiet.** A watch that delivered nothing for ~29 min prints `stopping` and is not re-armed (`watch.sh:1302-1303`). After that the seat has **no idle wake at all** (monitor mode installs no Stop hook, `delivery.sh:264-268`) until a human prompts it, a `poke` lands, or the next SessionStart. A long-idle seat that gets its first message after 30 minutes is deaf.
  - **Re-arm depends on the model.** Re-arming is a model action triggered by a printed line. A model that ignores it, or is busy and summarises it away, ends delivery. The authors keep a natural-language "re-arm" request in the command doc, and a team-wide re-arm through `poke`, to recover from this (`template.md` `execute-extra` slot).
  - **Wrong tool.** The directive spends text on "First confirm ToolSearch select:Monitor finds that generic tool; Azure Monitor, MCP monitor … are not valid … Do not start watch.sh with Bash/background/nohup". INFERRED: models were observed using the wrong tool.
  - Monitor is unavailable in `-p`/SDK, on Bedrock/Vertex/Foundry, or with telemetry disabled (primer §8). agmsg's fallback advice there is "switch to turn delivery or manual inbox checks" (`session-start.sh:562-564`).

### 3.2 Stop hook (`turn`)

There is **no idle wake.** The README states it directly: delivery waits "until your next interaction" (`README.md:275`). Latency is unbounded for an idle seat, and for a busy one it is the turn end, gated further by the 60 s cooldown (§6).

### 3.3 agmsgd native socket

- **Mechanism.** Claude Code's own cross-session inbox: "When the receiving session is idle, Claude Code starts a new turn with the message", and mid-turn it is read "between tool calls" (primer §3). agmsg leans on this fully: it neither types keystrokes nor tracks idle/busy.
- **Idle/busy detection.** It is delegated to Claude Code. The transcript tells agmsg afterwards which case happened: a `user` row (idle → new turn) or a `queued_command` attachment (busy → absorbed). VERIFIED `claude-code-queue-io.mjs:230-268`.
- **Draft safety.** None needed by agmsg. INFERRED: a socket delivery does not go through the composer, so a typed draft survives. Whether Claude Code starts the turn while the composer holds a draft is unknown.
- **Latency.** The daemon polls every 5 s, plus Claude Code's own scheduling. "a few seconds" (`README.md:273`).
- **Failure modes:**
  - **Hold.** Under `crossSessionInbound` or a permission-mode mismatch, the notice is held behind an approval dialog. agmsg retries once with the other mode claim, then marks the seat `blocked: peer_held` (`claude-code-queue.mjs:45-51,214-230`). If this is discovered at SessionStart, the seat falls back to Monitor (`_session-start.sh:162-165`). INFERRED: a hold that begins **mid-session**, after SessionStart chose native and skipped the Monitor, leaves the seat with no automatic delivery until the next SessionStart.
  - **Daemon stops mid-session.** The fallback to the watcher applies only "from their next session start" (`docs/design/agmsgd-architecture.md:116`). A running seat that skipped the Monitor has no path in the meantime. INFERRED from the same `exit 0` at `_session-start.sh:169`.
  - **Notice ignored.** A notice counts as confirmed once it is in the transcript, whether or not the model ran `inbox.sh`. The next notice is sent only for messages **newer than the confirmed tip** (`latestConfirmedCursor`, `claude-code-queue.mjs:83-89,295`). A model that reads "check your inbox" and does nothing is never reminded about those messages. VERIFIED (logic). Effect INFERRED.
  - **Skill dependency.** The notice does not say how to check the inbox. INFERRED: the model has to know the `/agmsg` command (a ~50 KB rendered doc; see §5) or remember `inbox.sh` from earlier context.
  - **Platform.** No Windows before Claude Code 2.1.302 (`README.md:157`). The sqlite store is required (`_session-start.sh:53-56`).

### 3.4 `poke` (agent-initiated, Herdr-relevant)

This is a manual fallback for a seat whose watcher is not delivering. Under Herdr, `terminal_poke` calls `herdr agent prompt <pane> <text>`, which submits by itself (`scripts/drivers/terminals/herdr/ops.sh:1084-1102`). Before typing, `safe-poke` runs a three-generation draft guard (VERIFIED `scripts/lib/input-box.sh:1-47`, `scripts/lib/safe-poke.sh:1-130`):

1. It locates the composer from manifest data (`❯` between the last two `─` rules for Claude).
2. It reads the pane **styled** (`herdr pane read --format ansi`) and treats any visible text **outside an SGR-dim span** as a real draft, then refuses with code 14. Claude's prompt suggestion and Codex's placeholder are drawn dim, while typed text is not (measured live 2026-09-21/22).
3. If nothing non-dim is visible, it compares two snapshots ~1 s apart to catch someone starting to type.
4. Herdr only (#1384): if a real draft is present but the pane is **not focused**, the draft counts as abandoned. agmsg saves it to a file, clears the box, pokes, retypes the draft, and verifies the retyped text byte-for-byte. If the pane gains focus mid-way, it aborts with 14.

Code 15 means "box not located". The code states that the guard narrows the clobber window but does not close it (`type.conf:46-48`).

---

## 4. Setup and installation footprint on Claude Code

| Written | When | Content | Evidence |
|---|---|---|---|
| `~/.agents/skills/<cmd>/` | install | scripts, `db/messages.db`, `teams/`, `run/` (pidfiles, markers, role-session records with the messaging token, mode 600) | VERIFIED overview, `role-session.sh:282` |
| `~/.claude/commands/<cmd>.md` | install (when `~/.claude` exists) | rendered `/agmsg` command: shared `scripts/skill-base.md` (31 KB) + `claude-code/template.md` overlay (21 KB) | VERIFIED `install.sh:60,1079,1408`; sizes by `wc -c` |
| `<project>/.claude/settings.local.json` | `agmsg delivery <mode>` (first join asks) | **per project**: `SessionStart`+`SessionEnd` (monitor), `Stop` (turn), all three (both), none (off). Prior agmsg entries are stripped first. Atomic temp-copy write, a content-equal check by JSON tree (no rewrite when unchanged), refuses a read-only file, preserves the file mode | VERIFIED `scripts/delivery.sh:155-316,359-416` |
| Permissions | not written; the command doc tells the user to add `Bash(~/.agents/skills/<cmd>/scripts/*)` to `~/.claude/settings.json` or the project's `settings.local.json` | otherwise "the user is asked to confirm essentially every call" | VERIFIED `template.md` `execute-extra` slot |
| agmsgd | `agmsg daemon enable` | user systemd unit `~/.config/systemd/user/agmsgd-<id>.service` or a LaunchAgent plist; `run/install.db` | VERIFIED `scripts/daemon.sh:259,306` |
| Claude config dir | **read only**: agmsgd reads `<CLAUDE_CONFIG_DIR>/projects/*/<sid>.jsonl` (tail for `permissionMode`, appended region for the nonce) | | VERIFIED `claude-code-queue-io.mjs:53-82,188-216,315-362` |

Hooks are per project and per mode, so every project needs `agmsg delivery …`. INFERRED: a session started in an unconfigured project gets nothing. Spawned seats are started as `claude -n <team>-<agent> [--resume <uuid>]` with `CLAUDE_CODE_SESSION_ID` unset (`type.conf:11,33,42`).

---

## 5. Token cost

All costs below are model-visible bytes. Static numbers are VERIFIED with `wc -c` on the source text; runtime totals are INFERRED.

| Event | monitor | turn | agmsgd |
|---|---|---|---|
| Session start | ~2.8 KB directive (≈700 tokens), **plus the model's own ToolSearch + Monitor tool calls**, plus a one-time agmsgd intro line when agmsgd is off (`lib/agmsgd-intro.sh:36`) | 0 | ~110 B line |
| Quiet turn | 0 | **0 bytes** (claude-code has no `stop_output`; cooldown and quiet exits are silent) | 0 |
| Per message, idle seat | one Monitor event per message (line ≈ 40 B + body + host framing), each possibly a model turn | not delivered until the next human turn | **two model calls per batch**: the notice turn (~135 B) → a Bash `inbox.sh` call → the result (`N new message(s):` + ≈20 B per message + body) |
| Per message, busy seat | event folded in by the host | one extra model call at Stop (`decision:block`) carrying the batch | notice absorbed as `queued_command`, then the same `inbox.sh` round trip |
| Periodic | re-arm line + Monitor re-invoke every ~29 min while traffic flows | none | none |
| On demand | `/agmsg` command doc ≈ 50 KB source (≈12k tokens; the rendered size after slot substitution is unknown) | same | same, and likely needed to know how to check the inbox |

Under Herdr, each non-cooldown Stop also renames the pane through the terminal driver ("one terminal resolution plus the driver's rename calls per turn, per team", `check-inbox.sh:227-261`). That costs latency only, no tokens.

---

## 6. Limitations and bugs

1. **Monitor goes deaf after ~29 quiet minutes.** By default it is not re-armed (`watch.sh:1294-1303`, `CHANGELOG.md:222`), and monitor mode has no Stop hook. A seat waiting for a slow peer misses the reply until a human or a poke wakes it. VERIFIED (code); effect INFERRED.
2. **Priming.** No delivery until the model's first turn (`README.md:294`). VERIFIED symptom.
3. **The cooldown marker is touched before the query, at every Stop** (`check-inbox.sh:183-198`). Mail that arrives within 60 s of any Stop, delivering or not, waits for the next Stop that is ≥ 60 s later. In a burst of short turns that is fine. A turn that ends 30 s after the previous one skips mail sent in between, and the seat may then go idle with that mail undelivered. VERIFIED (code); effect INFERRED.
4. **The cooldown marker is keyed only on the agent name** (`run/.lastcheck-$AGENT`, `:178-180`), shared across teams, projects and sessions in one install. Two seats named `alice` in different projects suppress each other's checks. VERIFIED (code); effect INFERRED.
5. **The Stop hook delivers as the first registered agent for (project, type), not the session's `actas` role** (`:134-163`, `:309-311`). This is mitigated by skipping teams whose lock another live session holds (`:312-326`). VERIFIED.
6. **No bounds** in Monitor lines, the Stop `reason` or `inbox.sh` output. Combined with mark-on-write, an oversized batch can be consumed while Claude shows a 2,000-char preview or truncates the Bash result. VERIFIED absence; effect INFERRED.
7. **agmsgd notice without content.** It costs an extra tool round trip per batch, depends on the model obeying "Check your agmsg inbox", and is never repeated for the same messages once confirmed (§3.3). VERIFIED logic.
8. **Native-path gaps mid-session.** A daemon stop or a new hold after SessionStart leaves the seat without the Monitor fallback until the next SessionStart (`agmsgd-architecture.md:116`; `_session-start.sh:169`). VERIFIED doc; effect INFERRED.
9. **Self-declared `from-mode="bypass"`.** agmsg claims the receiver's permission mode in the wrapper so that a bypass seat accepts it (`claude-code-queue-io.mjs:146,151-156`). This works with Claude Code's trust model rather than inside it. A future Claude Code that verifies the claim, or changes the undocumented wire shape (`msgV:1`), breaks agmsgd silently. agmsg's mitigation is that a hold is detected and reported. INFERRED.
10. **No ACK, priority or deadline.** Read means "written to stdout or the socket". VERIFIED overview.
11. **Per-project hook install** in `settings.local.json`. Not global, so a new project is silent until configured. VERIFIED.

---

## 7. Ideas worth stealing

1. **Native cross-session socket as the idle wake.** The wire shape is captured: `{"type":"auth","token":…}\n` then `{"msgV":1,"msg_id":…,"type":"user","message":{"role":"user","content":"<cross-session-message from=…>…</cross-session-message>"},"priority":"next","from":…}\n` (`claude-code-queue-io.mjs:140-158`). It types no keystrokes and needs no mod, and Claude Code itself decides idle versus mid-turn. For herdr-threads it is a candidate fallback wake when the mod is unavailable (managed `disableSideloadFlags`, `--hooks-only`, Claude < 2.1.287): today that case falls back to Herdr typing with an always-`Unsafe` composer. It fits rule (c), content-free wake prompts: send `wake up` and let the hook or channel deliver. Caveats: the shape is undocumented, version-tolerant only by observation, and subject to the hold and bypass-claim issue in §6.9. We would have to capture it ourselves first.
2. **Transcript-nonce confirmation** of a wake: `user` row means a new turn, `queued_command` attachment means absorbed mid-turn, `system/informational` means held. It reads only the region past a recorded offset, with a byte-stable re-read (`claude-code-queue-io.mjs:221-362`). This is real evidence that the model saw the wake, and it could back our wake receipts or `wake=` diagnostics. A herdr-threads version would have to treat the transcript as a hint, never as authority (TRUST-POLICY A2).
3. **Telling a prompt suggestion from a real draft by SGR dim** in a styled Herdr pane read (`input-box.sh:26-38`). We currently mark any composer text `Unsafe` because it might be a suggestion (`src/harness/claude/composer.rs`), and ask users to turn suggestions off. A dim-span test in the Claude adapter's composer classifier could make the no-mod fallback usable with suggestions on.
4. **Abandoned-draft recovery keyed on pane focus** (#1384): an unfocused pane with a stationary draft has its draft saved to a file, cleared, poked and retyped, with a byte-for-byte check (`safe-poke.sh:90-130`). This is a more aggressive policy than our 120 s hold; worth weighing against our draft-safety guarantees rather than adopting outright.
5. **Mark only after the write succeeded, and say so honestly**: the comment block at `check-inbox.sh:475-501` states exactly what a successful `printf` proves. We already ACK on display; the explicit "duplicate beats silence" framing is a good standard for our own hook paths.
6. **One outstanding wake per seat, never re-sent on a timer**, re-sent only when the destination changed (socket gone, record moved after `--resume`/`/compact`). This matches our "one wake in flight" gate, and it adds the destination-changed trigger. Our mod re-offers on a timer (60 s spacing, 3 per generation); agmsg's view is that a long hold is normal for Claude.
7. **The socket changes on `--resume` and survives `/clear` and `/compact`**, and the destination is republished at every SessionStart with a read-back check (`_session-start.sh:10-15,161-171`). This is useful if we ever address the socket.

Things *not* to copy:

- the model-obeyed Monitor directive, which causes priming, re-arm, wrong-tool prose and self-expiry;
- the shared, before-query cooldown marker;
- unbounded payloads;
- version-gating hook installs (their Codex PostToolUse gate), which our "never gate on an exact version" rule forbids.

---

## Verification

Adversarial pass on 2026-10-23 against the clone at `6e03154`, `gh api repos/fujibee/agmsg` (1,564 stars; `v1.6.0` published 2026-10-22T21:13Z) and the Claude primer.

Checked and confirmed: `type.conf` keys and the absent `posttooluse_output` (only codex has it); hook wiring per mode (`delivery.sh:185-290`); the Monitor directive text, role and stand-down variants, worktree skip and cc-instance dedupe (`session-start.sh`); watch.sh poll interval, line format, escaping, despawn handling, mark-after-printf, the pair re-checks and the renew/stop logic (`:1247-1306`); check-inbox.sh stdin timeout, `stop_hook_active` guard, first-agent identity, cooldown marker path and touch-before-query, block JSON at `:462`, mark-after-write and the exit-code comment; the agmsgd wire payload, auth line, socket ownership checks, permission-mode tail read, nonce judging and 256 KiB scan, one-pending-per-seat, confirmed-tip cursor, retry-other-mode; `inboxNudge` text; poll interval 5 s; unit/plist paths; architecture doc `:116`; CHANGELOG `:222`; Herdr `terminal_poke`; the dim-span draft guard, codes 14/15 and the #1384 focus recovery; install paths and the 31 KB + 21 KB command-doc sources; the template's permission and re-arm text; primer §1.2, §3, §8, §9, §10.1 and `src/harness/claude/composer.rs`.

Changed:
- watch.sh's query is `storage_watch_after` (`sqlite.sh:529-554`), not `storage_list_unread`; `storage_list_unread` does take an optional `--limit`, which `inbox.sh` and `check-inbox.sh` do not pass.
- `_session-start.sh` citations were off by about ten lines (the file has 172); corrected all of them. "Every failure path prints … Next: arm the Monitor" was too strong: three early exits are silent.
- README line numbers for the agmsgd, monitor and off rows corrected (273, 274, 277).
- Transcript path is keyed on the recorded project path, encoded differently on win32.

Not independently checked: the ~2.7 KB directive and ~110 B/~135 B size figures (plausible from the text, not re-measured), and whether Claude Code applies its 10,000-char cap to a Stop `reason` (still unknown).
