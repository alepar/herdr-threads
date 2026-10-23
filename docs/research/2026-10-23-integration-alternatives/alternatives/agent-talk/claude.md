# agent-talk × Claude Code: message injection and wake

- Upstream: https://github.com/xhluca/agent-talk. Paths are relative to the clone root. Shallow clone at HEAD `8a7fc4f1` ("release: 0.2.4", 2026-09-10). No code from the clone was executed. The only thing parsed was the text of the asciinema demo `demos/05-bob.cast`, read with a local `python3 -I` one-liner.
- The transport, [retalk](https://github.com/xhluca/retalk), was not cloned. Claims about it come from agent-talk's skills and docs.
- Harness reference: `../../primers/claude.md`. The local Claude Code was 2.1.309, and the mod types came from the 2.1.308 bundle.
- Tags: **VERIFIED** means read in source, docs, the local tool schema or the demo recording. **INFERRED** means reasoned and not observed. **unknown** means not found.

---

## 1. Support status

**Native.** Claude Code is agent-talk's primary host. VERIFIED: `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, and `skills/init/SKILL.md:52-55` ("These skills are written for Claude Code"). The README's auto-receive table lists Claude Code as "Yes … Built in; choose `auto` delivery in `init`" (`README.md:224`, VERIFIED).

The Claude integration has no hooks, no mod, no MCP server and no channel. It consists of three pieces:

| Piece | What it is | Who starts it | Evidence |
|---|---|---|---|
| Plugin monitor `retalk-inbox` | `monitors/monitors.json` entry, `when: "always"`, running `bin/inbox-monitor.sh` | Claude Code, at session start, in every interactive session with the plugin enabled | `monitors/monitors.json:2-7` |
| Plugin monitor `retalk-requests` | Same, for contact-request (invite registration) events, `bin/requests-monitor.sh` | Claude Code | `monitors/monitors.json:8-13` |
| Model-started `Monitor` tool watch | `tail -n 0 -F <spool> \| grep --line-buffered '"from":'`, plus a ~25 min scheduled-wake backstop | The model, following the `receive`/`init`/`send`/`add` skills whenever `check-mode` is `auto` | `skills/receive/SKILL.md:222-246`, `skills/init/SKILL.md:24-29,501-519`, `skills/send/SKILL.md:111-115`, `skills/add/SKILL.md:39` |
| Pull | `retalk receive --peer … --save`, run by the model | The model or user, on demand | `skills/receive/SKILL.md:99-116` |

A background follower (`bin/follow.sh` → `retalk receive --follow` → `bin/spool-writer.py`) feeds all three. It is harness-neutral and lives outside Claude. The model starts it with a Bash call.

---

## 2. Injection mechanism

### 2.1 Pipeline (VERIFIED unless marked)

```
relay ──poll every N s──▶ retalk receive --follow --quiet   (bin/follow.sh:135-140, N default 60)
        │  NDJSON {"id","from","name","text"[,"group","group_id"|"kind"...]}
        ▼
spool-writer.py: stamp "ts", fan out to every session registered to this user
        │  <user>/sessions/<sid>.ndjson  (+ legacy <user>/inbox.ndjson)   (bin/spool-writer.py:175-211)
        ▼
inbox-monitor.sh: tail -n0 -F both files | awk dedupe on "id"            (bin/inbox-monitor.sh:52-70)
        │  stdout line
        ▼
Claude Code plugin-monitor notification  ──▶ model context
```

Step by step:

1. **Follower.** `follow.sh __run` loops on `retalk receive "${peer_args[@]}" --follow --interval "$interval" --quiet --dir "$UD/identity" … | python3 "$WRITER" --user "$UD"`. It restarts 2 s after an exit (`bin/follow.sh:135-140`). `start` detaches it with `setsid nohup`, so it outlives the Claude session (`bin/follow.sh:94-103`). The interval defaults to 60 s, and the skill suggests `--interval 5` "for a rapid live exchange" (`bin/follow.sh:15-16`, `skills/receive/SKILL.md:211-212`).
2. **Spool writer.** For each stdin line, `stamp()` adds `"ts"` (UTC ISO-8601) and re-serializes with `json.dumps`, so every record stays exactly one line (`bin/spool-writer.py:164-172`). The registry `~/.agent-talk/by-session/*` is re-read at most every 5 s (`:188-190`). The line is appended under `flock` (mode 0600) to `<user>/sessions/<sid>.ndjson` for every sid whose map entry points at this user (`:108-125,142-161,191-194`). It also goes to the legacy `<user>/inbox.ndjson` (`--legacy` on by default, `:195-196,264-266`). Before each append, a spool over 8 MiB is rotated to `.1` (`:69,128-139`).
3. **Monitor command.** `inbox-monitor.sh` takes the session id from `CLAUDE_CODE_SESSION_ID` (an explicit `$1` overrides it). If there is none, it `exec tail -f /dev/null`, so push is off (`bin/inbox-monitor.sh:26-27`). It then polls every 2 s until the map file exists (`:30`), reads the user dir **once** (`:31`), and runs two `tail -n0 -F` processes merged into one awk dedupe. If the pipeline ends, it restarts after 1 s:

   ```bash
   while true; do
     { tail -n0 -F "$SESSION_SPOOL" 2>/dev/null &
       tail -n0 -F "$LEGACY_SPOOL" 2>/dev/null &
       wait; } | dedupe
     sleep 1
   done
   ```
   (`bin/inbox-monitor.sh:51-56`)
4. **Harness.** Claude Code runs the command "as a persistent background process" (https://code.claude.com/docs/en/plugins/manifest-reference.md#monitors) "for the whole session. What it prints reaches Claude as notifications" (https://code.claude.com/docs/en/plugins/components.md#monitors). Both VERIFIED. The entry's `description` is "shown in the task panel and notification summaries" (https://code.claude.com/docs/en/plugins/manifest-reference.md#monitors, VERIFIED).

### 2.2 Exact Claude surface

- **Plugin monitor** (`monitors/monitors.json`). Its `when: "always"` means it "starts at session start and on plugin reload" (manifest reference, VERIFIED). It runs in interactive sessions only. It does not run under `-p`, on Bedrock, Vertex or Foundry, or when telemetry settings disable the Monitor tool (components doc, VERIFIED). Each stdout line becomes a notification. That these arrive as the `task-notification` prompt origin is INFERRED (the types do not name monitors). The mod types describe that origin: `kind: 'task-notification'` is "A background task's notification, dequeued when the session went idle or delivered into a running turn (`turnId` set)" (`claude-code.d.ts` 2.1.308, lines 9287-9291, VERIFIED).
- **Monitor tool watch** (model-started). The current tool schema says: "Each stdout line is an event … notifications arrive in the chat. Events arrive on their own schedule and are not replies from the user". It also says "Stdout lines within 200ms are batched into a single notification" and "Monitors that produce too many events are automatically stopped" (Monitor tool description in this 2.1.309 session, VERIFIED).

### 2.3 Payload format and framing (VERIFIED)

What reaches the model is the **raw spool line** with no header, no label and no instructions:

```json
{"id": "…", "from": "<32-hex fingerprint>", "name": "bob", "text": "Did the relay switch work?", "ts": "2026-…Z"}
```

Variants: group mail adds `group` and `group_id`, and control records carry `kind` (`contact` with a `card`, or `group_leave`) (`skills/receive/SKILL.md:59-131`). The Monitor-tool recipe filters on `'"from":'`, which passes chat and control records alike (`skills/receive/SKILL.md:235`).

Harness framing: the plugin monitor's notification carries its `description`. The Monitor-tool notification carries the watch description. The demo shows it as `● Monitor event: "wake on incoming agent-talk messages from alice"` (`demos/05-bob.cast`, text extracted from the recording, VERIFIED). How the plugin-monitor wrapper looks to the model is unknown: no capture exists.

All semantic framing is done by the **model**, under skill rules. Session rule 1 says every send and receive "renders the exchange as a both-sides markdown transcript (📥 peer / 📤 you, timestamps, quoted bodies…) Real text, never summaries or counts" (`skills/init/SKILL.md:20-23`). The receive skill asks for "~1–3 prior turns" of context pulled from the spool and saved sent copies (`skills/receive/SKILL.md:49-54`). The rule "applies equally to messages a background follower pushes in" (`:37-38`). In the demo, each wake is followed by `Read 1 file, ran 1 shell command` and then the rendered transcript (`demos/05-bob.cast`, VERIFIED). So the model does one extra tool round per message to rebuild context.

### 2.4 When it fires

- **Busy session**: delivered into the running turn (`task-notification` with `turnId`, types, VERIFIED). Whether that happens after the current tool call or at the next model step is unknown.
- **Idle session**: see §3.
- **Latency floor**: relay poll interval (60 s default) + up to 5 s registry staleness for a newly registered session + `tail -F` reaction + ≤ 200 ms batching. A message is therefore **0-60 s late** by design. VERIFIED parameters, INFERRED sum.

### 2.5 Size bounds

There is no bound in agent-talk. The whole body is one JSON line, and the only limit is spool rotation at 8 MiB (`bin/spool-writer.py:69`, VERIFIED by absence elsewhere). Claude's own per-notification cap for monitor lines is unknown. The Monitor-tool WebSocket path ends the watch on frames over 1 MiB (tools reference, VERIFIED), but a cap for command-mode lines is not documented.

### 2.6 De-duplication

- **Plugin monitor**: an in-memory awk `seen[]` keyed on the `"id":"…"` substring, falling back to the whole line (`bin/inbox-monitor.sh:38-49`). Its purpose is to collapse the copy that also lands in the legacy file (`tests/test_monitor.py:50-79`, VERIFIED). The map lives only inside one pipeline iteration: when the `while` loop restarts after a tail exit, the awk process and its map are new (INFERRED from `:51-56`). `tail -n0` keeps old lines from replaying, so this only matters for lines that arrive twice after the restart.
- **Requests monitor**: dedupes on the whole line with `"ts"` removed (`bin/requests-monitor.sh:44-56`).
- **Monitor-tool watch**: no dedupe. It tails only the session spool, so there is no legacy duplicate (VERIFIED `skills/receive/SKILL.md:235`).
- **Across the two push paths: none.** In `auto` mode the skills tell the model to start the Monitor-tool watch "if missing" (`skills/init/SKILL.md:24-26`, `skills/send/SKILL.md:112`), while the plugin monitor runs on its own. Both tail the same session spool, so each message probably reaches the model **twice** (INFERRED; nothing in the repo reconciles the two, and grep for "duplicate|twice" finds only the in-monitor legacy case). The demo shows "1 monitor still running" (`demos/05-bob.cast`), so the plugin monitor may not have been active in that recording (unknown).

### 2.7 Acknowledgement

- **Transport ACK only.** `retalk receive` output is "auto-acked" and the relay deletes the message on delivery (`skills/receive/SKILL.md:103`, `docs/README.md:22`, VERIFIED as a docs claim). On the follower path, the ACK happens when the follower reads the message, before any Claude-side step (INFERRED from `bin/follow.sh:135-140`).
- There is **no model-receipt ACK**, no read cursor for Claude, and no read receipt to the sender (VERIFIED by absence). The receive skill says "The spool is the source of truth; the agent reads it each turn and relays anything new" (`skills/receive/SKILL.md:172-173`). Nothing enforces that, and nothing records what the model has already seen. The skill also says the reliable way "to never miss one" is to read the spool (`:219-220`).
- Recovery is the sealed history (`--save` / `RETALK_SAVE_MESSAGE=1`, replayed by the `history` skill) (`skills/receive/SKILL.md:142-156`, VERIFIED).

---

## 3. Wake mechanism

### 3.1 Two candidate wake paths

**(a) Plugin monitor line → task notification.** The repo contradicts itself on whether this wakes an idle session:

- It does: "a monitor wakes your session the moment a message lands, so replies appear on their own" (`docs/README.md:44`, VERIFIED).
- It does not: "the monitor injects new messages as **background context**, but it can't make the agent speak on its own — they surface on your **next turn**" and "can't prompt the agent unprompted" (`skills/receive/SKILL.md:168-174,218-220`, VERIFIED). The skill front-matter says the same: "(on your next turn)" (`skills/receive/SKILL.md:3`).

Harness side: the types say a `task-notification` is "dequeued when the session went idle" (VERIFIED). Plugin monitors are documented as a Monitor-tool variant ("instead of asking Claude to start them", tools reference), and the Monitor tool says "Claude interjects when an event arrives" (VERIFIED). So on current Claude Code a plugin-monitor line **should** start a turn on an idle session (INFERRED; not captured by us or by the repo). The skill's "next turn" wording looks stale, possibly written when the monitor never received a session id and push was silently off for everyone (`bin/inbox-monitor.sh:19-25`; INFERRED).

**(b) Model-started Monitor tool watch, "Proactive auto-wake via Monitor (recommended)".** The repo's documented "true spontaneous wake":

```
Monitor(
  description: "New agent-talk messages from <peer>",
  persistent: true,
  timeout_ms: 3600000,
  command: "tail -n 0 -F \"<user>/sessions/<session-id>.ndjson\" | grep --line-buffered '\"from\":'"
)
```
(`skills/receive/SKILL.md:230-237`, VERIFIED)

The demo shows it working on an idle session. Bob's pane: `Monitor started · task … · persistent`, then with nobody typing, `● Monitor event: "wake on incoming agent-talk messages from alice"`, then the model reads the spool, renders the transcript and replies with `retalk send`. The README says "on 05-bob the human types nothing at all" (`demos/README.md:11-18`, `demos/05-bob.cast`, VERIFIED from the recording text; recorded 2026-07-14 per the cast header timestamp, on Claude Code v2.1.209, "Sonnet 5 · Claude API").

### 3.2 Harness-version drift breaks the recipe as written (VERIFIED)

- The Monitor tool schema in this 2.1.309 session has **no `persistent` parameter**, and `additionalProperties: false`. A call that passes `persistent: true` should fail input validation (INFERRED from the schema; the model may also drop the field).
- `timeout_ms` has a schema maximum of 3,600,000, but "Deadlines above 1800000ms are capped to 1800000ms" (tool schema; types line ~17339). The docs say "Every watch Claude starts has a deadline: 5 minutes by default, at most 30 minutes … At the deadline the watch ends. Claude gets one notice, so it can start the watch again" (https://code.claude.com/docs/en/tools-reference.md#monitor-tool).
- The July demo (Claude Code v2.1.209 per the cast's banner) printed `Monitor started · … · persistent`, so that build accepted a persistent watch. The 2.1.308 types still carry `persistent?: boolean` ("No timeout — runs until TaskStop or session end") and "0 when persistent" on the Monitor tool **output** (`claude-code.d.ts` lines 21631-21638, VERIFIED), but the model-facing **input** schema no longer offers it. So the model can no longer request a persistent watch; whether the harness still creates persistent ones internally (for example for plugin monitors) is unknown.

Consequence: on current Claude the recommended watch lives **at most 30 minutes**. Re-arming depends on the model handling the expiry notice. The "~25 min scheduled wake-up … as a backstop" (`skills/receive/SKILL.md:245-246`) is model-created (`ScheduleWakeup`/cron), so it too depends on the model remembering. If the model does not re-arm, path (a) is the only push left (INFERRED).

### 3.3 Idle/busy detection

agent-talk does none. It hands every event to the harness, which queues it: idle → a new turn, busy → into the running turn (types, VERIFIED harness behaviour; INFERRED for plugin monitors). There is no gate, priority or threshold. Every message wakes the session (VERIFIED by absence).

### 3.4 Draft safety

Nothing is typed into the composer, so a human's draft text cannot be overwritten (INFERRED from the mechanism). Whether a notification-started turn begins while the human has a half-typed draft, and what happens to that draft, is unknown. agent-talk has no draft hold. herdr-threads' mod holds wakes for 120 s while `$.prompt.read()` shows text (primer §9).

### 3.5 Latency

The wake itself is sub-second once the line lands ("wakes the agent sub-second", `skills/receive/SKILL.md:227-228`, an author claim). The end-to-end delay is dominated by the relay poll interval, 0-60 s by default (§2.4).

### 3.6 Failure modes

| Failure | Effect | Evidence |
|---|---|---|
| Map not written, or written with an empty id (the model pasted `${CLAUDE_SESSION_ID}` into Bash) | The monitor waits forever at `while [ ! -f "$map" ]`, and records go only to the legacy file (still tailed, but only by a monitor that has a map) | `bin/inbox-monitor.sh:30`, `skills/init/SKILL.md:548-554` (VERIFIED) |
| No `CLAUDE_CODE_SESSION_ID` in the monitor env | `tail -f /dev/null`: push silently off | `bin/inbox-monitor.sh:26-27` (VERIFIED). Whether Claude exports it to monitors is a repo claim (`:23-25`), not in the Claude docs (the manifest reference lists monitors as receiving *no* exported path vars) |
| Session re-`init`s as a different user | The monitor read the map once (`:31`), so it keeps tailing the old user's spools | INFERRED from `bin/inbox-monitor.sh:30-34` |
| Lines written before the tail attaches (map just written, 2 s poll, and 5 s registry staleness in the writer), during the 1 s restart gap, or while Claude was closed (resume) | Never pushed, because of `tail -n0`. The relay has already acked, so only reading the spool or history recovers them | INFERRED from `:30,51-56`, `bin/spool-writer.py:188-190` |
| Monitor-tool watch expires (≤ 30 min) and the model does not re-arm | Spontaneous wake stops. Only the plugin monitor remains | §3.2 |
| Chatty peer | "Monitors that produce too many events are automatically stopped" (Monitor tool, VERIFIED). Whether this also applies to plugin monitors is unknown. If it does, push stops silently, and the script's own `while true` cannot restart a process Claude killed | INFERRED |
| `/reload-plugins` | `when: always` starts the monitor on reload. Whether the old process is stopped first, or two run and double-deliver, is unknown. Disabling the plugin mid-session does not stop running monitors ("They stop when the session ends") | manifest reference (VERIFIED for the start), components doc (VERIFIED for disable) |
| `-p` / SDK | No plugin monitor. The Monitor tool is still available, with watches capped at 10 minutes in a `-p` run, so the skill's watch recipe works there only briefly | components doc, tools reference (VERIFIED) |
| Bedrock, Google Cloud Agent Platform (Vertex), Foundry, `DISABLE_TELEMETRY`, `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` | No plugin monitor and no Monitor tool: pull only | components doc, tools reference (VERIFIED) |
| Follower dies | `follow.sh` restarts `retalk` every 2 s. The supervisor survives the session, but nothing restarts it after a reboot (systemd unit optional) | `bin/follow.sh:135-140`, `skills/receive/SKILL.md:254-265` |

---

## 4. Setup and installation footprint on Claude Code

| What | Where | Who writes it | Evidence |
|---|---|---|---|
| Marketplace plus plugin install (`claude plugin marketplace add xhluca/agent-talk`, `… install agent-talk@agent-talk`) | Claude's plugin store and `enabledPlugins` (`~/.claude/…`) | The user, or the model via `init` §1 | `README.md:50-57`, `skills/init/SKILL.md:192-198` |
| Two always-on plugin monitors | In **every** interactive session where the plugin is enabled, including sessions that never use agent-talk: a bash loop plus `sleep 2` poll per monitor until a map appears, then 2-3 `tail` processes and an `awk` | Claude Code | `monitors/monitors.json` (`when: "always"`, not `on-skill-invoke:<skill>`) |
| Optional permission rule `"permissions": {"allow": ["Bash(retalk:*)"]}` | Project `.claude/settings.json` | The model suggests it once | `skills/init/SKILL.md:179-185`, `docs/README.md:18` |
| Recommended **auto permission mode** | Session mode | The user | `README.md:70-72` |
| No hooks, no `settings.json` env, no MCP server, no channel, no mod | — | — | VERIFIED: the plugin has no `hooks/` or `.mcp.json` |
| `~/.agent-talk/by-session/<sid>` (never removed) and `<user>/sessions/<sid>.ndjson`, `.requests.ndjson` (plaintext, 0600) | Outside Claude config | The model via Bash (`init` §4) | `skills/init/SKILL.md:555-561`, `bin/spool-writer.py:235-239` |
| Detached follower and invite watcher (`setsid nohup`) | User process tree, outlives the session | The model via Bash | `bin/follow.sh:94-103` |

Plugin `bin/` is on the Bash tool's PATH (components doc). The skills nevertheless call scripts by `<plugin>/bin/…` with `${CLAUDE_PLUGIN_ROOT}` substituted into skill text (`skills/receive/SKILL.md:190`, VERIFIED).

---

## 5. Token cost

**Per message (INFERRED estimates):**
- The notification: about 100-150 bytes of JSON overhead (`id`, 32-hex `from`, `name`, `ts`, keys) plus the body, plus the harness wrapper and description. The plugin monitor's description is 215 chars (`monitors/monitors.json:5`), and whether the model sees it per event is unknown. If both push paths are live, everything doubles (§2.6).
- One wake turn per message on an idle session. Once the prompt cache has expired (about 5 min, noted by the skill itself at `skills/receive/SKILL.md:250-252`), that turn re-reads the full context.
- Mandatory rendering: the model reads the spool and saved sent copies (one or two tool calls, as in the demo's "Read 1 file, ran 1 shell command"), then emits a markdown transcript of the new message plus 1-3 prior turns from **both** sides. The body is echoed back as output tokens one to four times. In the demo, a reply turn showed `↓ 379 tokens` (`demos/05-bob.cast`).
- No batching beyond Claude's 200 ms window. No priority means every message costs a turn.

**Per quiet turn:** zero injected bytes, since no hook runs on ordinary turns (VERIFIED by absence of hooks). Background costs:
- Monitor-tool expiry notices every ≤ 30 min, plus the ~25 min scheduled backstop: roughly 2-4 idle model turns per hour while "auto" is armed, each probably at uncached prices (INFERRED from §3.2).
- The skills are large (`init` 754 lines, `receive` 272 lines; `wc -l`) but load on demand. In auto mode, `send`, `add` and `init` each tell the model to check or start the follower and Monitor, which adds tool calls after routine sends (`skills/send/SKILL.md:111-115`).

---

## 6. Limitations and bugs visible

1. **The recommended wake recipe is invalid or degraded on current Claude Code** (VERIFIED schema vs `skills/receive/SKILL.md:233-234`). `persistent` no longer exists, and `timeout_ms: 3600000` is capped at 30 min. The 1 h value and the "~25 min backstop" fit a harness that changed under the plugin. This matches the primer's flagged risk.
2. **The repo contradicts itself on idle wake** (VERIFIED): `docs/README.md:44` against `skills/receive/SKILL.md:3,168-174,218-220`. The skill's model-facing text tells the model the plugin monitor cannot wake it, which nudges the model to arm the extra Monitor watch and the scheduled backstop, and so pay for duplicates.
3. **Probable double delivery** from the plugin monitor plus the Monitor-tool watch, with no dedupe between them (INFERRED, §2.6).
4. **Stale init text** (VERIFIED): `skills/init/SKILL.md:65` and `:549-550` say `monitors.json` uses `${CLAUDE_SESSION_ID}`, but `monitors/monitors.json:4` no longer passes it. `docs/README.md:16` still names the map `<CLAUDE_SESSION_ID>`.
5. **Identity is carried by the model.** It must write the session id out literally and remember an absolute `<user>` path on every command (`skills/init/SKILL.md:548-564`, `docs/README.md:16`). A mistake silently disables push or acts as the wrong user (INFERRED). The live-collision guard is "is a follower running for this user" (`skills/init/SKILL.md:528-539`), which says nothing about which session owns it.
6. **No delivery state for Claude.** `tail -n0` start, an in-memory dedupe and no cursor mean that anything landing while the monitor is not attached is never pushed, after the relay has already acked it (§3.6). "The agent reads it each turn" is aspirational (VERIFIED text, INFERRED gap).
7. **Raw peer JSON enters context with no trust framing** from agent-talk. The Claude harness marks monitor events as "not replies from the user" (VERIFIED tool description), which is the safer posture compared with agent-talk's pi and opencode paths (`overview.md`). But agent-talk adds no "peer, not user" marker, and the skills never tell the model to treat bodies as untrusted (grep: none found). The README recommends auto permission mode (`README.md:70-72`), so a malicious or confused peer's text drives a model that runs Bash without prompts (INFERRED risk).
8. **No bounds**: whole bodies, a wake per message, no priority or threshold, no quiet mode (VERIFIED by absence).
9. **Always-on monitors in unrelated sessions**: `when: "always"` spawns two idle loops per Claude session for every enabled install. `on-skill-invoke:init` exists in the manifest schema and would avoid this (manifest reference, VERIFIED option; INFERRED trade-off: it would not start in a resumed session until a skill runs).
10. **Registry never cleaned**: map entries are never removed (`bin/spool-writer.py:235-239`), so spool-writer keeps fanning out to dead sessions indefinitely. The GC (`--gc`, run only from `init` step 4, `skills/init/SKILL.md:560`) sweeps a registered session's spool only once it is untouched for 14 days (`bin/spool-writer.py:232,240`), but every append refreshes the mtime, and the next message recreates a swept spool. So on an active identity, dead sessions' plaintext spools grow until the 8 MiB rotation (VERIFIED code; INFERRED exposure).
11. **The plugin monitor is unavailable under `-p`/SDK** and on non-first-party providers. Under `-p` only a ≤ 10 min model-started watch remains; on non-first-party providers delivery is pull only (VERIFIED docs).

---

## 7. Clever ideas worth stealing

1. **A plugin monitor as a zero-mod push path.** One `monitors/monitors.json` entry gets busy-turn interjection and idle wake from the harness, with no hooks, no mod API, no `CLAUDE_CODE_PLUGIN_DIRS`, and nothing typed into the pane. For herdr-threads this is a candidate fallback where mods are disabled (managed `disableSideloadFlags`, pre-2.1.287) instead of native typing. For example, `herdr-threads watch --lines` under a plugin monitor, as the primer §8 already suggests. It still needs a capture of idle-start behaviour, the wrapper framing, and the noisy-monitor auto-stop rule.
2. **NDJSON, one record per line, as the event unit.** `json.dumps` guarantees one message per notification regardless of newlines in the body (`bin/spool-writer.py:172`). The 200 ms batching then gives free burst coalescing.
3. **Session id from the monitor's environment, not its argv.** `CLAUDE_CODE_SESSION_ID` resolution with a guard against an unexpanded `${…}` literal (`bin/inbox-monitor.sh:26-28`), learned from a silent-off bug. Worth copying as a pattern: treat a literal `${` in an id as absent.
4. **Per-session spool fan-out with a registry**, so several sessions on one identity each get their own copy and read position (`bin/spool-writer.py:107-125,187-194`). herdr-threads already does this in the daemon; the useful part is the simple file contract a stateless monitor can tail.
5. **Harness-marked non-user authority.** Delivering through task notifications ("not replies from the user") rather than as user prompts puts peer text below user authority without custom framing. This matches herdr-threads' rule (e) and costs no per-message boilerplate. The harness enforces it instead of a header.
6. **`on-skill-invoke:<skill>` monitors** (not used by agent-talk, but its always-on cost points to it). Start the watcher only once the integration is actually used in that session.

What not to copy: model-started watches with deadlines (they depend on the model re-arming, and the API has drifted), model-written identity paths, `tail -n0` with no cursor, and mandatory model-rendered transcripts, which multiply token cost per message.

---

## Verification

Checked against the clone at `8a7fc4f` (`monitors/monitors.json`, `bin/inbox-monitor.sh`, `bin/requests-monitor.sh`, `bin/follow.sh`, `bin/spool-writer.py`, `skills/{receive,init,send,add}/SKILL.md`, `docs/README.md`, `README.md`, `demos/README.md`, `demos/05-bob.cast` text, `tests/test_monitor.py`), the live Monitor tool schema in this session, the 2.1.308 `claude-code.d.ts`, and the current docs (`plugins/manifest-reference.md`, `plugins/components.md`, `tools-reference.md#monitor-tool`). Every file:line citation was re-read; all matched or were within one line.

Changed:
- §2.1 step 4: the "persistent background process" quote is from the manifest reference, not the components page; citation split.
- §2.2: mapping of monitor lines to the `task-notification` origin marked INFERRED (the types do not name monitors).
- §3.2: "persistent mode has since been removed" was too strong. The 2.1.308 types still have `persistent` on the Monitor tool output; only the input parameter is gone. Added the demo's Claude Code version (v2.1.209).
- §3.6 and §6.11: wrong that `-p` has no Monitor tool. The tools reference gives `-p` watches a 10 min cap; only the plugin monitor is interactive-only. Split the row; added `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`, and that disabling a plugin mid-session does not stop its monitors.
- §5: monitor description is 215 chars, not ~230.
- §6.10: wrong that fan-out to dead sessions ends at the 14-day GC. Appends refresh mtime and recreate swept spools, and `--gc` runs only from `init`.
- §2.6: dedupe line range widened to `:38-49`.

Not refuted, still INFERRED: idle wake from a plugin-monitor line, double delivery across the two push paths, and the plugin-monitor wrapper framing (no capture exists).
