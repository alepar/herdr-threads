# agent-peers-mcp (mikez93) x Claude Code

- Upstream: https://github.com/mikez93/agent-peers-mcp, read at `2c63379d65ad44845815faa21a0c5c78355002fa`. All paths are relative to the clone root.
- Harness reference: `../../primers/claude.md` (cited below as "primer §N").
- Tags: **VERIFIED** means read in source, docs or Claude Code docs. **INFERRED** means reasoned from those. **unknown** means not found. No code from the clone was run.

## Support status

Claude Code support is **native and first-class**. The whole project grew out of a Claude-only tool (`README.md:21`, a successor of louislva/claude-peers-mcp). VERIFIED. The repo is a GitHub fork of `Co-Messi/agent-peers-mcp` (GitHub API: `fork: true`, parent `Co-Messi/agent-peers-mcp`, 0 stars, last push 2026-10-22). VERIFIED.

There are three Claude delivery surfaces, all in one MCP stdio server, `claude-server.ts`, plus one hook script:

| Surface | Launch mode | Role |
|---|---|---|
| `notifications/claude/channel` push | Terminal `agentpeers` alias with `--dangerously-load-development-channels server:agent-peers` | Live push, polled every 1-3 s |
| `check_messages` MCP tool | Every enabled session | Backfill and authoritative durable read |
| `bin/claude-peer-rewake` `asyncRewake` Stop/StopFailure hook | Sessions **without** the channel (Claude desktop app, incl. remote-SSH) | Idle wake only; bodyless |

There is no `UserPromptSubmit`, `PostToolUse`, `SessionStart` context hook, mod, keystroke typing, native cross-session messaging (`CLAUDE_CODE_MESSAGING_SOCKET`) or plugin monitor (grep of the tree; VERIFIED absent).

## Injection mechanism

### Pipeline: broker → MCP server → durable file → channel

1. **Poll.**
   - A self-scheduling loop (`claude-server.ts:585-594`) runs one in-flight cycle at a time.
   - The delay is 1 s within 120 s of peer activity and 3 s otherwise (`AGENT_PEERS_IDLE_POLL_MS`; `claude-server.ts:53-60`). The comment says a flat 1 s poll cost about a quarter core across ~50 sessions.
   - The broker leases **every** unacked, unleased row for this peer, in `ORDER BY id ASC`, with no LIMIT (`broker.ts:899-910`). The lease is 30 s (`broker.ts:27`). VERIFIED.
2. **Persist before ack** (`claude-server.ts:505-520`). VERIFIED.
   - `inboxStore.queueLeasedMessages([m])` writes `~/.agent-peers-claude/<peer>.json` (0600, atomic rename; `shared/codex-inbox.ts:60-83,154-164`).
   - It also writes a **bodyless** `<peer>.metadata.json` with id, from_*, to_id and sent_at (`shared/codex-inbox.ts:26-40,212-221,275-286`).
   - If the write fails, the message is neither pushed nor acked, so the lease expires and the message is re-offered.
3. **Ack the broker right after persistence**, before the push and whatever the push does: `seen.add(m.id); toAck.push(m.lease_token)` (`claude-server.ts:519-520`, batched `ackMessages` at `:568-574`). VERIFIED. On Claude, the broker ack therefore means "on this machine's disk", not "the model saw it" (`docs/delivery-contract.md:15-18`, and the Claude section at `:31-41`).
4. **Channel push** (`claude-server.ts:527-541`). VERIFIED:

   ```ts
   await mcp.notification({
     method: "notifications/claude/channel",
     params: {
       content: m.text,
       meta: { message_id: String(m.id), from_id: m.from_id, from_name: m.from_name,
               from_peer_type: m.from_peer_type, from_summary: m.from_summary,
               from_cwd: m.from_cwd, sent_at: m.sent_at },
     },
   });
   ```

   - The capability is declared as `experimental: { "claude/channel": {} }` (`claude-server.ts:104-105`).
   - Per primer §4 the model reads `<channel source="agent-peers" message_id="…" from_id="…" from_name="…" from_peer_type="…" from_summary="…" from_cwd="…" sent_at="…">body</channel>`. `source` is deliberately omitted because Claude sets it from the server name (`claude-server.ts:522-526`).
   - After a successful push the message goes into the in-memory ring buffer (`recordDelivered`, `:543`; 50 entries / 15 min, `shared/recent-delivered.ts:30-31`). A push failure is only logged (`:550-552`).

### When it fires

- **Busy session**: whenever a poll finds mail and the session was launched with the dev-channel flag, the push is sent. Per the Claude docs, events "queue into the session"; several that arrive "while Claude is busy ... are delivered together on the next turn" (primer §4, VERIFIED docs). Whether "next turn" means the next model step inside the running turn or the next turn proper is not stated; the project's instructions assume mid-turn visibility (`claude-server.ts:115-119`). INFERRED.
- **Idle**: the project asserts that an idle push "queues invisibly" until the next turn and "may not render visibly even then" (`claude-server.ts:121-129`; `shared/recent-delivered.ts:3-8`; `docs/delivery-contract.md:42-47`; `README.md:420-423`). The primer instead INFERS that channels do start a turn on an idle session (primer §4, open question 3).
  - Nobody has a capture. The project's own test admits the "last mile ... can only be verified manually" (`tests/e2e-live-delivery.test.ts:13-17`). Whether channel push wakes an idle Claude session is **unknown**. The project's whole Claude design assumes it does not.
- **No-channel sessions** (desktop, or a plain `claude` with `AGENT_PEERS_ENABLED=1`): the server still sends the notification, but Claude "drops the events silently" when the server is not loaded as a channel (primer §4). Delivery then rests on `check_messages` alone. INFERRED from code plus primer.

### `check_messages` (backfill, the authoritative read)

- Tool at `claude-server.ts:181-185`, handler at `:259-326`. VERIFIED.
- It returns the union of:
  - the ring buffer minus confirmed ids (`:282`);
  - **all** durable unread entries not blocked in the delivery state machine (`:292-299`).

  The result is sorted by id and rendered by `formatInboxBlock` (`shared/piggyback.ts:35-63`).
- Framing (VERIFIED, `shared/piggyback.ts:40-62`):
  - A fixed header of about 430 bytes: `PEER INBOX — N unread message(s) from your colleagues.\nRead each one. Then: …Do NOT auto-acknowledge.`
  - Then one block per message: `--- message i of N ---`, `from: <name> (<type>, cwd=<cwd>)`, optional `their current work: <summary>`, `sent_at`, `message_id`, `text:` + body, and `reply_action: send_message(to_id="<name>", message="...")`.
  - A trailing `PEER INBOX` line.
- Empty result: a fixed sentence of about 250 bytes, "No peer messages pending (ring buffer covers the last 15 minutes; …)" (`:305-311`).
- **Confirm-on-next-call** (`claude-server.ts:209-224`; `shared/delivery-state.ts:9-34`). VERIFIED.
  - A response is `promote`d only if the request was not aborted (`:320-324`).
  - Its ids are removed from the durable file only when a **later** agent-peers tool call arrives whose arrival generation postdates the promote (an "arrival-causality barrier").
  - An aborted call rolls back and the ids can be re-dealt.
  - This is a real (if indirect) model-receipt signal: "the model issued another call after seeing it".
- Instructions at `claude-server.ts:131-141` tell the model to call `check_messages` "as the FIRST thing you do at the START of every user turn". The tool description repeats this (`:183`). But see Limitations #1: the instructions copy is almost certainly cut off before the model sees it.

### Size bounds

- Broker cap: 500 unacked messages, enforced only for a target whose `last_seen` is stale (an offline durable peer); a live peer is uncapped (`broker.ts:55,838-843,857-862`; conversation mailboxes at `:808-809`). On Claude the broker row is acked right after persistence, so this cap never bounds the local durable file, which has no cap at all (`shared/codex-inbox.ts:154-164`). VERIFIED.
- No per-body length limit or truncation was found in `broker.ts`, `shared/piggyback.ts` or `claude-server.ts`. The only `text.length` is a log preview (`claude-server.ts:549`). VERIFIED by grep, so treat "unbounded" as VERIFIED for these files.
- No page bound on `check_messages`: every unblocked durable entry (unbounded) plus up to 50 ring entries go into a single tool result. Claude's own limits apply:
  - a warning at 10k tokens and a 25k-token cap (`MAX_MCP_OUTPUT_TOKENS`);
  - text results over 50,000 chars are saved to a file (primer §5; `https://code.claude.com/docs/en/mcp`, VERIFIED).

  INFERRED consequence: a large backlog is truncated or spilled by the harness, yet the server `promote`s it as delivered, and the next call confirms and deletes it.
- Channel meta: `from_summary` is peer-authored free text and has no cap in the server. Meta values are strings, so it rides along on every push. INFERRED.

### De-duplication and acknowledgement

- **Channel**: an in-memory `seen` set by broker id. A re-leased row is acked again but not re-pushed (`claude-server.ts:490,499-504`). It is watermark-pruned at max-10,000 (`:557-560`). Scope is in-process only; across restarts delivery is at-least-once (`:16-17`). VERIFIED.
- **Channel vs backfill double show**: a pushed message stays in the durable file, because nothing removes it on push (`:505-552`). The rule "call check_messages every turn" therefore re-presents every channel-delivered body once more in full. The instructions concede this: "check_messages will just re-show the same thing" (`:137-141`). Dedupe is left to the model ("Dedupe is the model's job", `:269-273`). VERIFIED.
- **No semantic ACK**: the protocol forbids "got it" replies (`shared/piggyback.ts:47`; `shared/colleague-prompt.ts:24-25`), and there is no ACK request type. The sender only learns "queued" (`docs/delivery-contract.md:11-13`). VERIFIED.
- **Ordering**: FIFO by broker id. No priorities. VERIFIED.

## Wake mechanism

### Terminal sessions (channel present): no wake by design

`bin/claude-peer-rewake` exits 0 immediately when the Claude command line contains `--channels`/`--dangerously-load-development-channels … server:agent-peers` (`bin/claude-peer-rewake:79-88,136-137`). Nothing else wakes the session. VERIFIED.

Idle terminal sessions are therefore woken only if channel push itself starts a turn, which the project believes it does not (unknown, see above). The README's position is that idle peers "won't magically reply" (`README.md:420-423`).

### Channel-less sessions: `asyncRewake` Stop hook

Installed by hand as a user-level `Stop` and `StopFailure` hook (`README.md:280-284`):

```json
{ "hooks": [ { "type": "command", "command": "/Users/mike/agent-peers-mcp/bin/claude-peer-rewake", "asyncRewake": true, "timeout": 86400 } ] }
```

The mechanism (all VERIFIED from `bin/claude-peer-rewake`):

1. **Arm**: Claude runs the hook in the background after every turn end. It reads and discards stdin (`:130-133`).
   - It deliberately does not use `SessionStart`, because in SDK mode (desktop) an async SessionStart hook "holds the session until it exits" (`:16-18`; `README.md:289`).
   - Consequence: a brand-new session is not wakeable until its first turn ends.
2. **Find its own session** by process tree (`:62-72`):
   - Walk up to 6 ancestors from `getppid()` until a process has exactly one child whose command contains `agent-peers-mcp/claude-server.ts`.
   - Then map that server pid to the broker peer id, with a read-only SQLite query `SELECT id FROM peers WHERE pid = ? AND peer_type = 'claude' ORDER BY last_seen DESC LIMIT 1` (`:91-103`).
3. **Skip** when the session has the channel or runs `-p`/`--print` (`:75-88,136-137`).
4. **Single waiter per session**: a non-blocking `flock` on `~/.agent-peers-claude/rewake/<claude_pid>.lock`. A second Stop hook while a waiter lives exits 0 (`:150-154`). Stale locks for dead pids, and announced files older than 7 days, are deleted (`:139-149`).
5. **Wait**: poll every 3 s (`:42`) for:
   - Claude pid liveness, via `kill(pid, 0)`;
   - peer registration, giving up after 60 s unregistered (`:45,162-164`);
   - the bodyless metadata file's `unread` ids minus an "announced" set persisted per peer, capped at 500 ids (`:106-126,165-168`).
6. **Fire**: on fresh ids it saves them as announced **before** exiting, then writes one stderr line and exits 2 (`:169-177`):

   ```
   Agent Peers: N new peer message(s) from <senders> (ids <ids>). Call the agent-peers check_messages tool to read them, then handle them as a colleague would.
   ```

   Per the Claude docs, an `asyncRewake` exit 2 "wakes Claude immediately even when the session is idle", and stderr becomes a system reminder (primer §1.4, VERIFIED docs). The model then calls `check_messages` for bodies.
7. **Re-arm**: the woken turn ends, `Stop` fires again, and a new waiter starts. Messages that arrive between exit and re-arm are caught by the announced-set diff. VERIFIED logic. Re-arm timing: INFERRED.
8. **Expiry**: it exits 0 after `MAX_SECONDS = 86000` (`:44`), under the 86,400 s hook timeout. An idle session is not wakeable after about 24 h until a human turn re-arms it (`README.md:288`). VERIFIED.

### Idle/busy detection

**None.** VERIFIED that the script has no busy check. The waiter is armed at turn end; if Claude does not cancel a still-running async hook when the next turn starts (not documented; INFERRED), it keeps waiting through that busy turn, the next Stop's waiter exits on the held lock, and a message arriving mid-turn triggers exit 2 mid-turn.

What Claude does with a mid-turn `asyncRewake` exit 2 (fold it in, hold it, or start an extra turn) is **unknown** (primer §10 Q2).

### Draft safety

The hook types nothing, so it cannot overwrite composer text. VERIFIED (no keystroke path). Whether a rewake turn starts while the user has a half-typed draft, and whether the draft survives, is **unknown** (primer §1.4). There is no `$.prompt.read`-style guard, and none is possible from a shell hook. INFERRED.

### Latency

- Channel: poll interval 1-3 s plus broker round trip (`claude-server.ts:53-60`). VERIFIED.
- Rewake:
  - the metadata file is written by the MCP poll (1-3 s), then the waiter polls it (3 s), so 0-6 s before exit 2;
  - then the woken turn spends one more model step on the `check_messages` call before the body is in context.

  INFERRED.

### Failure modes (VERIFIED from code unless marked)

- **Session not woken until its first turn ends**: there is no SessionStart arming. Hook fires only on Stop/StopFailure.
- **Woken once only**: ids are marked announced before exit. If the woken model ignores the reminder or the turn aborts, the messages never trigger another wake. They stay in the durable file for the next manual `check_messages`.
- **About 24 h unwakeable** after the waiter expires (`:44,179`).
- **Path-coupled detection**: the repo must live in a directory named `agent-peers-mcp` (`is_peer_server`, `:58-59`). A different clone dir, or two agent-peers servers under one Claude process, means "no session found", so it exits 0 and silently never wakes (`:68-69`).
- **Ancestor depth of 6**: a wrapper chain deeper than 6 between the hook and Claude defeats detection.
- **Portability**: the script hard-codes `/bin/ps -axo pid=,ppid=,command=` (`:49`). procps-ng accepts that form (VERIFIED by running the system `ps` on Fedora, not the clone's code), and `/bin/ps` exists on merged-/usr distros. The project itself has not tested Linux: `README.md:774` says "Linux should work but untested".
- **Channel detection reads argv only**: a session whose channel is enabled by another route (for example `--channels plugin:…`) is classed as channel-less. The hook then also wakes it, which is harmless duplication. INFERRED.
- **User-level install**: every Claude session on the machine, peer or not, spawns a `ps` scan plus a waiter attempt on every turn end. Most exit fast (`:134-137`). INFERRED cost: small.

## Setup / installation footprint on the harness

All VERIFIED from `README.md:80-110,276-289` and `docs/components/mcp-servers-delivery.md:536-548`.

1. `claude mcp add --scope user --transport stdio agent-peers -- bun "$AGENT_PEERS_DIR/claude-server.ts"`. This writes `~/.claude.json`, so **every** `claude` session spawns the server.
   - Without `AGENT_PEERS_ENABLED=1`, the server connects with zero tools and no broker traffic (`claude-server.ts:363-382`).
   - Desktop sessions (`CLAUDE_CODE_ENTRYPOINT=claude-desktop`) are enabled by default unless `AGENT_PEERS_ENABLED=0` (`:369-377`).
2. A shell alias: `alias agentpeers='AGENT_PEERS_ENABLED=1 claude --dangerously-skip-permissions --dangerously-load-development-channels server:agent-peers'`.
   - It bundles **permission bypass** with the channel flag.
   - It needs the dev-channel confirmation dialog each launch, and claude.ai or Console auth (primer §4).
3. For desktop sessions, a hand-edited `Stop` + `StopFailure` hook in `~/.claude/settings.json`.
4. Side effects on the terminal: the server writes OSC 0/1/2 tab titles (`peer:<name>`) to `/dev/tty` **every second** for the session's life (`shared/tab-title.ts:37-50`; `claude-server.ts:411-412`). It can be disabled with `AGENT_PEERS_DISABLE_TAB_TITLE=1`. INFERRED: inside Herdr this would fight Herdr's pane titles.
5. State: `~/.agent-peers.db`, `~/.agent-peers-secret`, `~/.agent-peers-claude/{<peer>.json,<peer>.metadata.json,rewake/}`. A broker under launchd.

## Token cost

Byte counts come from the source strings. Model-visible size is INFERRED from them.

### Per session

- **MCP server instructions**: about 7,184 chars (5,577 `COLLEAGUE_PROTOCOL` + 1,607 Claude delivery prologue). Claude cuts server instructions at **4,096 chars** (`https://code.claude.com/docs/en/mcp`: "Server instructions, and the description of a tool that loads upfront: 4,096 characters", VERIFIED). The model therefore pays about 4 KB, and the rest is lost (see Limitations #1).
- **Tool schemas**: 5 tools. `check_messages` alone has a 416-char description. Under tool search they are deferred, so only names load (VERIFIED docs, same page).

### Per message

- Channel path: body + about 7 meta attributes (ids, names, cwd, summary, timestamp) ≈ body + 250-400 bytes. Then the **same body again** inside the next `check_messages` block: about 430-byte header + about 200 bytes per message + body. Total ≈ **2 × body + ~0.9 KB** for a single message. INFERRED from `claude-server.ts:527-541` and `shared/piggyback.ts:40-62`.
- Rewake path: about 170-byte reminder + tool call + `[PEER INBOX]` block ≈ body + ~0.8 KB + one extra model step.

### Per quiet turn

If the model follows the rule, each user turn spends one `check_messages` call plus about 250 bytes of "No peer messages pending…" text, plus the tool-call tokens and one extra model step of latency. The rewake hook costs no tokens when quiet. INFERRED.

## Limitations and bugs

1. **The key delivery rule is probably invisible to the model.** INFERRED, but the evidence is strong.
   - The instructions put the 5,577-char colleague protocol first and append the Claude-specific "DELIVERY ON THIS SIDE … RULE: Call `check_messages` as the FIRST thing …" after it. The rule sits at offset 6,570 (`claude-server.ts:108-141`).
   - Claude Code cuts at 4,096 (above). The "EXAMPLE OF GOOD COLLABORATION" section starts at offset 4,052, so the cut falls about 44 chars into it; the rest of the example, the TOOLS list and the whole delivery section (from offset 5,579) are dropped (offsets computed from the source strings). The only surviving copy is the `check_messages` tool description (`:183`), which a deferred tool's model may never read until it searches for it.
   - Unless the user raises `CLAUDE_CODE_MAX_MCP_DESCRIPTION_LENGTH` (v2.1.280+), the turn-start backfill is not reliably instructed. Combined with "no wake for terminal sessions", idle-arrived mail can sit until a human prompts "check messages".
2. **Unproven core assumption.** Their Claude design (ring buffer, the turn-start rule, no wake for channel sessions) rests on "idle channel pushes queue invisibly". This was never captured (`tests/e2e-live-delivery.test.ts:13-17`), and the Claude docs and the primer suggest the opposite (primer §4). If channels do wake an idle session, they are spending an extra show per message for nothing.
3. **Every channel message is shown twice** (channel + backfill), and dedupe is delegated to the model (`claude-server.ts:137-141,269-273`).
4. **Delivered-but-truncated**: there is no page bound on `check_messages`. Harness truncation or file-spill of a large `[PEER INBOX]` still counts as delivered and confirmed (Size bounds above). INFERRED.
5. **Broker ack on persistence, not on read**: the sender-side and broker-side view says acked while the message may never have reached the model. The durable file is the only remaining truth, and it is per-machine and per-peer (`claude-server.ts:515-520`). VERIFIED. An honest at-least-once to disk, not to the model.
6. **Rewake fires regardless of busy state** and only once per id (Wake failure modes above).
7. **No priorities or thresholds**: every new message from any peer wakes a desktop session. The only damping is the per-id announce. VERIFIED (`bin/claude-peer-rewake:168-177`).
8. **Stale docs**:
   - `README.md:420-421` says neither client lets an MCP server trigger a turn, but the README itself ships the rewake hook (`:276-289`).
   - `README.md:426` says Claude's `check_messages` "drains an in-memory ring buffer", but the code unions the durable store (`claude-server.ts:292-299`).

   VERIFIED.
9. **Footprint**: the alias couples the feature to `--dangerously-skip-permissions` and the dev-channel dialog. There is a 1 Hz `/dev/tty` title write. The MCP is spawned globally in every Claude session. VERIFIED.
10. **Untested Claude paths**: no test exercises `claude-peer-rewake` or `claude-server.ts` end to end against a real Claude. Tests cover only broker semantics and the ring buffer (`tests/e2e-live-delivery.test.ts`, `tests/recent-delivered.test.ts`). VERIFIED by grep.

## Clever ideas worth stealing

1. **`asyncRewake` Stop hook as a keystroke-free idle wake.** The idle wake is documented by the Claude docs (VERIFIED docs); neither the project nor we have a capture of it working.
   - It is plain settings-hook machinery: no mod, no TUI typing, no `$.prompt.submit`, and it works where mods are disabled (managed `disableSideloadFlags`, desktop/SDK entrypoints).
   - For herdr-threads' **hooks-only fallback** (today: Herdr native typing with the composer-unsafe problem, primer §9), this could replace typing. The waiter would block on `herdr-threads watch` (bodyless, threshold-filtered) and exit 2 with a content-free marker (R18), and the turn-start delivery would follow.
   - It needs a capture of the mid-turn and draft behaviour first (primer §10 Q2). Our `wake_frontier` would supply the busy gate that their script lacks: the hook could check the daemon's busy view before exiting 2.
2. **Arm on `Stop`, not `SessionStart`**, because an async SessionStart blocks SDK sessions. The re-arm-every-turn pattern plus a per-session `flock` gives exactly one waiter without daemon bookkeeping.
3. **Bodyless metadata file as the waker's only input.** The waker can never leak or ack bodies, and the delivery path stays single. This is the same principle as our content-free wake, done with a file split.
4. **Announced-id set persisted outside the waker** (`rewake/<peer>.announced.json`): a wake fires only for ids not yet announced, so repeated re-arms never re-nudge the same mail. This is comparable to our bounded re-offer ledger.
5. **Confirm-on-next-call with an arrival-causality barrier** (`shared/delivery-state.ts`): it treats "the model issued another call after this response was built" as receipt evidence, and an aborted request rolls the draw back. It is a cheap, harness-neutral receipt signal for any tool-result delivery path. Our `watch ack --via context` path could adopt the same "response promoted only if not aborted" rule.
6. **Write-before-push, ack-after-persist** with fail-closed file checks (0600, single link, owner uid; `shared/codex-inbox.ts:236-251`). A solid local durability pattern.
7. **Adaptive poll interval** (1 s while a conversation is live, 3 s otherwise), motivated by a measured CPU cost across ~50 sessions (`claude-server.ts:49-60`).
8. **Negative lesson**: measure model-visible instruction text against the harness's caps. Their 7 KB instructions, with the critical rule last, are the cautionary example for our R17 rule "put critical details near the start".

## Verification

Adversarial pass, 2026-10-23, against the clone at `2c63379d`, the GitHub API, the primer and `https://code.claude.com/docs/en/mcp`. No clone code was run.

- Re-read every cited range in `claude-server.ts`, `bin/claude-peer-rewake`, `shared/{piggyback,delivery-state,recent-delivered,codex-inbox,tab-title,colleague-prompt,ensure-broker}.ts`, `broker.ts`, `README.md`, `docs/delivery-contract.md`, `docs/components/mcp-servers-delivery.md` and `tests/e2e-live-delivery.test.ts`. Most citations and behaviours hold.
- Recomputed the instruction sizes from the source strings: 5,577 + 1,607 = 7,184 chars; delivery section at 5,579, `RULE:` at 6,570, example at 4,052. Confirmed the 4,096-char cut and `CLAUDE_CODE_MAX_MCP_DESCRIPTION_LENGTH` (v2.1.280+) in the MCP docs, and that tool search loads "only tool names and server instructions" at session start.
- Changed:
  - the broker 500 cap applies only to stale targets (`broker.ts:838-843,857-862`; `:808` is conversation mailboxes), and the Claude local durable file is uncapped;
  - the `check_messages` bound now says the durable part is unbounded;
  - busy-channel timing now quotes the docs ("delivered together on the next turn") and marks mid-turn visibility INFERRED;
  - the waiter surviving a new turn is now marked INFERRED;
  - Linux `ps` portability: procps-ng accepts the form (checked with the local system `ps`);
  - the `check_messages` description is 416 chars, not 418;
  - the cut offset wording, the rewake SQL (now quoted exactly), and the delivery-contract citation for ack-after-persist (`:15-18`);
  - the "asyncRewake VERIFIED working" claim downgraded to documented but not captured;
  - added the fork-of-`Co-Messi/agent-peers-mcp` metadata.
