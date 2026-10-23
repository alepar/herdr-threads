# agent-talk x Pi coding agent (pi-mono)

- Alternative: agent-talk, <https://github.com/xhluca/agent-talk>, shallow clone at `8a7fc4f1` (2026-09-10, "release: 0.2.4"). Paths written as `agent-talk:<path>:<line>` are relative to that clone root.
- Harness: Pi, <https://github.com/badlogic/pi-mono> (now `earendil-works/pi`), clone at `a7b94a03`, coding-agent `1.4.0`. Paths written as `pi:<path>:<line>` are relative to the pi-mono clone root. The harness facts come from `docs/research/2026-10-23-integration-alternatives/primers/pi.md` (in herdr-threads) and were re-checked in source where this report depends on them.
- retalk (the transport CLI) was not cloned. Claims about it come from agent-talk's own docs.
- No code from either clone was executed. Tags: **VERIFIED** means read in source or docs at the cited line. **INFERRED** means my reading of that code, not observed in a run.

## 1. Support status

**Native.** Pi is the harness where agent-talk's push path is simplest and has the fewest moving parts. VERIFIED:

- `agent-talk:package.json:11-27` makes the repo a Pi package. It has the `pi-package` keyword and the manifest `"pi": {"skills": ["./skills"], "extensions": ["./extensions"]}`.
- `agent-talk:extensions/inbox-monitor.ts:1-172` is the Pi extension, 172 lines. Its only runtime import is `node:fs`; it also has a type-only import of `ExtensionAPI` from `@earendil-works/pi-coding-agent` (`:24-25`).
- `agent-talk:docs/pi-auto-receive.md` is the design note. `agent-talk:README.md:146-164,225` covers the quickstart and lists auto-receive as "Yes". `agent-talk:skills/init/SKILL.md:578-594` is the init step "4b. Enable auto-receive on pi".
- CI (`agent-talk:.github/workflows/ci.yml:102-136`) checks two things. `pi install "$PWD"` must find every skill, counted through `pi --mode rpc` `get_commands`. The extension must attach, which the job proves by grepping for its `session_start` notify with `AGENT_TALK_PI_SPOOLS` set. CI never checks that a message is actually injected. The doc claims a manual live test between two Pi sessions, with the injected custom message arriving "about two to three seconds after the send" (`agent-talk:docs/pi-auto-receive.md:136-150`). That is the author's claim, and I did not reproduce it.

Only one extension file is loaded, and the stray files next to it are ignored. VERIFIED: `"./extensions"` is a directory entry. Pi expands it through `collectFilesFromPaths` and then `collectAutoExtensionEntries` (`pi:packages/coding-agent/src/core/package-manager.ts:2605-2616`, `:608-660`). That function takes top-level `*.ts`/`*.js` files, plus subdirectories only if they have `index.ts`/`index.js` or a `package.json` pi manifest. So `extensions/inbox-monitor.ts` loads, and `extensions/codex/` (Python) and `extensions/opencode/inbox-monitor.ts` (no index) do not.

## 2. Injection mechanism

### 2.1 Pipeline

```
relay --(retalk receive --follow, poll every N s)--> follower (bin/follow.sh, setsid nohup)
      --stdout NDJSON--> bin/spool-writer.py --> <user>/sessions/<sid>.ndjson  (+ legacy <user>/inbox.ndjson)
      --fs.watch + 1 s setInterval--> extensions/inbox-monitor.ts (in the Pi process)
      --pi.sendMessage({customType:"agent-talk-inbox",...}, {triggerTurn:true})--> AgentSession
```

The first two stages are shared with every harness (see `overview.md`). Only the last hop is Pi-specific. VERIFIED: `agent-talk:bin/follow.sh:136`, `agent-talk:bin/spool-writer.py:175-211`, `agent-talk:extensions/inbox-monitor.ts:126-171`.

### 2.2 Exact API and code path

1. `session_start` reads `AGENT_TALK_PI_SPOOLS`, a list of absolute paths separated by colons, and calls `startWatch` on each path. When the list is non-empty and the session has a UI, it shows a notify. VERIFIED `inbox-monitor.ts:45-51,158-167`. The variable is read from `process.env`, so it must be set before Pi starts. VERIFIED `skills/init/SKILL.md:581-589`.
2. `startWatch` **seeks to EOF** (`w.offset = fs.statSync(path).size`), installs `fs.watch(path, () => drain(w))` (wrapped in try, because the file may not exist yet) and a `setInterval(drain, 1000)` backstop. VERIFIED `inbox-monitor.ts:126-144`.
3. `drain` stats the file. If `size < offset` it treats the file as truncated or rotated, resets to 0 and clears the carry. It reads `[offset, size)` synchronously, advances `offset` by the bytes read, keeps an unterminated tail in `carry`, and parses each complete line as JSON. Lines that do not parse are skipped. VERIFIED `inbox-monitor.ts:82-124`.
4. `deliver(rec)` skips `kind === "contact"` records and records with an empty `text`, skips ids already in `seenIds`, and then:

```ts
// agent-talk:extensions/inbox-monitor.ts:68-79
const who = rec.name || rec.from || "a peer";
pi.sendMessage(
  { customType: "agent-talk-inbox",
    content: `New agent-talk message from ${who}:\n\n${text}`,
    display: true,
    details: { from: rec.from, name: rec.name, id: rec.id } },
  // Idle: trigger a turn now. Streaming: queued and delivered after the
  // current assistant turn finishes its tool calls (steer default).
  { triggerTurn: true },
);
```

5. On the Pi side, `pi.sendMessage` (`pi:packages/coding-agent/src/core/extensions/loader.ts:384-387`) calls the bound action at `pi:packages/coding-agent/src/core/agent-session.ts:3448-3455`. That action is fire-and-forget: `sendCustomMessage(...).catch(err => runner.emitError(...))`. `sendCustomMessage` (`agent-session.ts:2357-2393`) then takes one of two branches:
   - when streaming and `triggerTurn !== false`, it calls `this.agent.steer(appMessage)` (no `deliverAs`, so steer);
   - when idle with `triggerTurn` set, it calls `this._startRun({messageType:"custom", message})`, which wakes the session (section 3).

   VERIFIED.
6. On the wire to the model, a `custom` message becomes a plain **`user` role** message whose text is `content` (`pi:packages/coding-agent/src/core/messages.ts:162-169`). `details` (from, name, id) is never sent to the model (primer 2.3, `pi:packages/coding-agent/docs/message-types.md:235`). The message is persisted as a `CustomMessageEntry` in the session JSONL. `display:true` renders it in the TUI. VERIFIED.

### 2.3 When it fires

- **Idle.** It fires immediately and starts a run (section 3).
- **Busy (streaming).** The message is queued as a steer. Pi polls steering at loop start and after each complete tool batch, before the next model request (`pi:packages/agent/src/agent-loop.ts:184-185,210-215,304`, primer 2.3). So the message lands mid-run after the current tool batch. It does not interrupt a running tool. VERIFIED (Pi). The extension comment says "after the current assistant turn finishes its tool calls", which matches.
- **Bursts.** With the default `steeringMode: "one-at-a-time"` (`pi:packages/coding-agent/src/core/settings-manager.ts:723-724`, `pi:packages/coding-agent/src/core/settings-defaults.ts:7`, `pi:packages/coding-agent/docs/settings.md:31`), N queued messages are injected one per model request. INFERRED from the Pi code: a burst of N records drained while idle becomes one `_startRun` plus N-1 steers. The reason is that `_startRun` sets `_isAgentRunActive = true` synchronously at entry (`agent-session.ts:1873-1874`), and `isStreaming` returns that flag (`:1479-1481`). Each later steer needs its own model request.
- **Pi not running, or started after the message arrived.** Never. The cursor starts at EOF, so the backlog is left to the `receive`/`history` skills (`inbox-monitor.ts:129-130`, `docs/pi-auto-receive.md:99-101`). VERIFIED.
- No turn-start (`before_agent_start`), post-tool (`tool_result`) or settle (`agent_before_settle`) hook is used. Grep of the extension finds `pi.on` only for `session_start` and `session_shutdown`. VERIFIED.

### 2.4 Payload, size bounds, framing

- Record shape: `{"id","from","name","text"}`, plus `ts` stamped by the spool writer (`inbox-monitor.ts:27-35`, `spool-writer.py:164-172`). VERIFIED.
- Framing: one prose header, `New agent-talk message from <who>:`, a blank line, then the raw body. The model-visible text carries no message id, fingerprint, timestamp, thread or group, untrusted-input marker or reply instruction. `<who>` is `rec.name`, falling back to the fingerprint (`from`). VERIFIED. Whether `name` is the receiver's own contact label or a name the sender chose is unknown, because that is decided in retalk.
- Bounds: **none**. The body is pushed whole with no truncation, paging or per-message cap. A single `drain` reads the whole unread tail into one `Buffer.alloc(len)` (`inbox-monitor.ts:99-101`). The only ceiling is the 8 MiB spool rotation (`spool-writer.py:69,128-139`). VERIFIED. Pi itself adds no bound (primer 2.3).

### 2.5 Dedupe and acknowledgement

- Dedupe uses an **in-memory** byte offset per path plus an **in-memory** `Set` of ids (`inbox-monitor.ts:56-57,64-67`). Neither survives a Pi restart, `/reload`, `/new`, `/resume` or `/fork`. Each of those replaces the extension runtime (primer 2.1), and `session_shutdown` calls `stopAll()` (`inbox-monitor.ts:146-156,169-171`). That causes no duplicates, because the new watcher seeks to EOF again, but every message that arrives in the gap is lost (2.6). `seenIds` grows without bound for the life of the process. A record with no `id` bypasses the id check and relies on the offset alone (`inbox-monitor.ts:64`). VERIFIED code, INFERRED consequence.
- Acknowledgement: there is no reader ACK. The relay ACK happened earlier, when the follower's `retalk receive` drained the message, and the relay deletes on delivery (`overview.md`, "Delivery guarantees"). Nothing tells the sender that the model saw the message. `sendMessage` returns `void` and errors only reach `emitError`. By the time Pi could fail, the id is already in `seenIds` and the offset has already moved. VERIFIED code, INFERRED consequence.

### 2.6 Does the spool ever get written for a Pi session? (likely broken in the documented flow)

The documented setup appears to leave the extension watching a file nobody writes to. INFERRED, high confidence, because every link in the chain was read:

1. The spool writer appends only to `<user>/sessions/<sid>.ndjson` for each `sid` registered in `~/.agent-talk/by-session/<sid>` that points at this user. It re-reads the registry every 5 s (`spool-writer.py:108-125,188-194`). VERIFIED.
2. Registration is init step 4, which is explicitly "Claude Code only — skip this step on ... pi" (`skills/init/SKILL.md:541-547`). Step 4b for Pi only tells the user to relaunch with `AGENT_TALK_PI_SPOOLS=".../<user>/sessions/<session-id>.ndjson" pi` (`:578-589`). It never writes the registry entry and never says where `<session-id>` comes from. The Codex step 4d, by contrast, says registration is needed "just as much" and does it (`:632-644`). VERIFIED.
3. With no registration, the writer reaches only the legacy `<user>/inbox.ndjson`, and only while `--legacy` is on (default on "for one release", `spool-writer.py:28-30,195-196,264-267`). The writer's own docstring concedes this: "Only Claude Code writes the session map, so on every other host that is the normal case" (`spool-writer.py:48-52`, again at `:78-82`). The Pi docs no longer point at that file, although the extension's stale header comment still does (`inbox-monitor.ts:7,14`). VERIFIED.
4. `spool-writer.py --gc` deletes any spool whose sid is not registered ("an unregistered session is gone for certain", `spool-writer.py:235-245`). Claude's init step 4 runs `--gc` (`skills/init/SKILL.md:560`). So a hand-created Pi spool is swept the next time the same user is set up on Claude. VERIFIED code, INFERRED interaction.

So push on Pi works only if the model improvises the registration, or if the user points `AGENT_TALK_PI_SPOOLS` at the legacy `inbox.ndjson`. The author's live test (`docs/pi-auto-receive.md:136-150`) probably predates the switch to per-session spools, or was set up by hand. That is INFERRED and not checkable from a shallow clone. There is also a chicken-and-egg problem: the variable must name the spool *before* Pi starts, but a fresh `pi` creates a new session id, and the Pi path never uses `PI_SESSION_ID` (grep: no hits in the clone). As the Codex step admits, "any unique id works" (`:646-647`), so the id is effectively a free-form label.

## 3. Wake mechanism

- **Mechanism.** It uses Pi's harness-native in-process API: `pi.sendMessage(..., {triggerTurn:true})` while idle runs `AgentSession._startRun({messageType:"custom"})` (`agent-session.ts:2378-2383,1873-1909`). No keystrokes, no TUI composer, no pty and no external process are involved. A deferral covers messages sent while `agent_settled` handlers are running (`agent-session.ts:2379-2381`). VERIFIED. The wake and the content are the same object: the woken run's first message is the peer's text.
- **Idle and busy detection.** The extension does none. It calls the same API in both states and relies on `sendCustomMessage` branching on `this.isStreaming` (that is, `_isAgentRunActive`). VERIFIED. It never calls `ctx.isIdle()`, never looks at `agent_settled` or `ui_prompt_*`, and never checks compaction. Behaviour during an idle manual compaction is unknown. `isStreaming` does not cover compaction (`agent-session.ts:1479-1486`), so the code takes the `_startRun` path, and whether that conflicts with an in-flight compaction is something I did not trace.
- **Busy policy.** A busy session gets the message as a steer after the current tool batch, so no second turn is started. This matches herdr-threads delivery rule 4 (never nudge a busy session; serve it in-turn). VERIFIED.
- **Wake policy.** Every message wakes, with no priority, threshold or quiet mode. Compare herdr-threads rules 1-2, where a default send never wakes. INFERRED from the absence of any such logic.
- **Draft safety.** There is none, but nothing gets clobbered either. `sendMessage` does not touch the editor, so a half-typed human draft stays in the composer while the agent starts a run under it. When the human presses Enter, the draft becomes a steer into that run (`pi:packages/coding-agent/docs/usage.md:35,40`). Pi exposes `ctx.ui.getEditorText()` and `onTerminalInput` (primer 2.4), and the extension uses neither. INFERRED from the code paths. Draft-clobbering is not possible, but an unwanted interjection while the human is typing is.
- **Latency.** The follower's relay poll is `--interval 60` by default, or about 5 s on request (`bin/follow.sh:15-16`). The spool-writer adds nothing, `fs.watch` fires at once, and the 1 s poll is the backstop. So the worst case is roughly the relay interval plus 1 s, and the median is about half the interval. VERIFIED parameters, INFERRED arithmetic. The 2-3 s the doc measured (`docs/pi-auto-receive.md:141-145`) implies a short interval or a faster retalk follow mode. Unknown which.
- **Failure modes.**
  - Pi down, or between `/new`/`/reload`/`/resume`/`/fork` and the next `session_start`: messages are lost to push (EOF seek). The relay has already acked them, so they are recoverable only through `history` (sealed copy) and only if the model or user thinks to look. VERIFIED code, INFERRED consequence.
  - `sendCustomMessage` rejects (for example `_prepareRun` throws in `_checkCompaction` or in a `before_agent_start` handler, `agent-session.ts:1877-1885,1926-1935`): the error goes to `emitError`, and the message is already marked seen, so it is lost. INFERRED.
  - Rotation race: the writer rotates *before* appending once the file is ≥ 8 MiB (`spool-writer.py:142-145`). Records that reached the old file within the last ≤1 s before rotation are never read, because the new file's `size < offset` resets the reader to 0 on the new inode. INFERRED, an edge case.
  - File deleted and recreated (gc, rotation): the `fs.watch` handle stays on the old inode, and only the 1 s poll keeps working. That is functional but slower. INFERRED.
  - A malformed line, or a record with empty `text`, is skipped silently. VERIFIED `inbox-monitor.ts:62-63,117-121`.
  - Several Pi sessions on one identity: each needs its own spool, which needs its own registration (2.6). If two sessions watch the same file, both get pushed and both reply. INFERRED.
  - Headless modes: in `--mode rpc`/`json`/`print` the extension still loads and still triggers turns (`ctx.hasUI` gates only the notify, `inbox-monitor.ts:161`). In `-p` print mode, a message arriving mid-run is steered into a one-shot run. INFERRED.

## 4. Setup and installation footprint on the harness

VERIFIED unless marked:

- `pi install git:github.com/xhluca/agent-talk` adds a package declaration to `~/.pi/agent/settings.json` (or `.pi/settings.json` with `-l`) (`pi:packages/coding-agent/docs/packages.md:21`). It clones into `<agentDir>/git/...`, which is `~/.pi/agent/git` (`pi:packages/coding-agent/src/core/package-manager.ts:2198-2200`). Every skill and `extensions/inbox-monitor.ts` load in every Pi session from then on.
- Pi itself gets no hooks file, no MCP config and no settings beyond the package entry. The extension stays inert unless `AGENT_TALK_PI_SPOOLS` is set (`inbox-monitor.ts:12-17`).
- Per launch, the user must start Pi with `AGENT_TALK_PI_SPOOLS=...` in the environment (init step 4b). The skill can only *tell* the user to relaunch (`skills/init/SKILL.md:583-585,593-594`).
- Outside Pi: `~/.agent-talk/users/<name>/` (identity, contacts, passphrase path, `check-mode`, `receive-from`, follower pid/opts) and a detached follower process per user (`bin/follow.sh`). The `~/.agent-talk/by-session/` registry should also be here but is not written for Pi (2.6).
- The `pi -e <path>` per-process loading (primer 2.1) is not used, so there is no zero-config-write option.

## 5. Token cost

- **Per session (static).** Pi adds each skill's name, description and path to the system prompt (`pi:packages/coding-agent/docs/skills.md:43`). The agent-talk `SKILL.md` front-matter totals about 5.3 KB, roughly 1.3k tokens, measured with `awk` over `skills/*/SKILL.md`, plus paths. That is paid in every Pi session where the package is installed, whether or not agent-talk is in use. Full skill bodies load only when the model opens them, but they are large: `init` is 47 KB, `receive` 15 KB, `send` 6.6 KB (`wc -c`). Replying to one pushed message probably means opening `send` (about 1.6k tokens) unless it is already in context. INFERRED.
- **Per message.** Header overhead is about 35 bytes plus the sender name, roughly 10 tokens, plus the body, uncapped. Every message also costs **at least one extra model request**: one `_startRun` when idle, or one steer request per message when busy under `one-at-a-time`. Each of those re-sends the context, which prompt caching may soften. The prose carries no ids, so a reply has to come from the skills (an extra file read, and `retalk` CLI calls whose output adds tokens too). INFERRED.
- **Per quiet turn.** Zero. No per-turn hook runs and nothing is injected when there is no mail. The watcher is a `stat` per second inside the Pi process and costs no tokens. VERIFIED by absence.

## 6. Limitations and bugs visible

1. **The documented Pi setup probably never receives push** (2.6): there is no `by-session` registration, and the docs point at a per-session spool that nothing writes. VERIFIED chain, INFERRED outcome.
2. **Message loss across restarts and session switches**: in-memory cursor plus EOF seek, after the relay has already acked (2.5, 3). VERIFIED code.
3. **Peer text arrives with user authority**: `custom` becomes a `user` message with no "peer, not user" framing (`messages.ts:162-169`). That contradicts agent-talk's own reasoning for the Codex wake, which deliberately pushes no body (`agent-talk:bin/codex_wake.py:6-10`). VERIFIED code, INFERRED risk.
4. **Every message wakes and gets its own turn**: there is no priority or threshold, no batching, and no rate limit. A chatty peer burns quota, and bursts are serialized one per request (2.3). VERIFIED absence, INFERRED cost.
5. **No size bound** on bodies or on a drain read (2.4). VERIFIED.
6. **No ids in model-visible text**, so the model cannot cite or dedupe by id, and replying depends on skills plus a `name`-to-contact mapping. VERIFIED `inbox-monitor.ts:72`.
7. **No draft or human-presence check**, although Pi offers `getEditorText()`/`onTerminalInput` (3). VERIFIED absence.
8. **The env var is fixed at launch** and the skill cannot change it, so turning push on means relaunching. VERIFIED.
9. **Stale header comment** pointing at `<user>/inbox.ndjson` (`inbox-monitor.ts:7,14`) against the docs' per-session spool. VERIFIED.
10. **Errors are swallowed**: the `sendMessage` failure path and parse failures are silent (3). VERIFIED.
11. **Never uses `PI_SESSION_ID`** (exported to the model's bash tool, primer 5) or `ctx.sessionManager.getSessionId()` to name or register the spool, which would remove the guessing. VERIFIED absence.

## 7. Clever ideas worth stealing (for a herdr-threads Pi adapter)

herdr-threads has no Pi adapter today: `builtins()` has claude, codex, hermes and agy, and a `pi` pane is `HostUiState::Unknown` (primer 8). agent-talk shows the minimal version that works and, by its gaps, what to add.

1. **An in-process extension is the right integration shape for Pi** (VERIFIED as viable). It needs no keystrokes, no Herdr composer typing and no screen-scraping. It is a `Channel`-class wake ("the harness wakes itself"), like the Claude mod. A herdr-threads `integrations/pi/` extension would hold the daemon connection (or watch-stream) and call `pi.sendMessage`.
2. **Use the busy branch as rule 3/4 for free.** `sendMessage` while streaming queues a steer that lands after the current tool batch. That gives in-turn delivery to a busy session without a nudge, which is exactly delivery-contract rules 3-4. VERIFIED Pi behaviour.
3. **Map priority onto Pi's delivery options, which agent-talk does not do.** For `p <= T`, use `triggerTurn:true` (wake). For `p > T` while idle, use `deliverAs:"nextTurn"` or `triggerTurn:false` (silent, arrives with the next run). While busy, always steer. Pi has all four branches (`agent-session.ts:2370-2391`). That keeps rules 1-2 intact where agent-talk wakes on everything. INFERRED design.
4. **A one-second stat-poll backstop beside `fs.watch`** is cheap insurance against missed watch events and file recreation. VERIFIED pattern (`inbox-monitor.ts:136-142`). We would watch a daemon stream rather than a file, but the "event plus slow poll" pairing carries over.

These are improvements over agent-talk that its gaps point to, all INFERRED:

- **Persist the cursor in the daemon, not in memory.** Our receipt and offer ledger already does this.
- **Bind seat to session through `ctx.sessionManager.getSessionId()`, and handle `session_start.reason`/`previousSessionFile` for `/new`/`/resume`/`/fork` continuity.** This avoids agent-talk's model-written ids.
- **Draft guard through `ctx.ui.getEditorText()` before a `triggerTurn` wake.** If the composer is non-empty, defer the wake (or fall back to `nextTurn`). This closes the human-interjection gap that agent-talk leaves open and that Herdr-typed wakes cannot close.
- **Load at managed launch with `pi -e <ext>`** rather than `pi install`, so there are no writes to `~/.pi`, and use `PI_CODING_AGENT_DIR` for isolated tests.
- **Keep our compact framing (`[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:`) and the 8 KiB truncation with `body <id>` continuation.** agent-talk's version shows the cost of skipping both: no ids, and unbounded bodies landing as user turns.
- **Consider `steeringMode`.** Under the default `one-at-a-time`, deliver one framed *page* per `sendMessage` (our page assembler) rather than one message per call, so a burst costs one request, not N.

## Verification

Adversarial pass, 2026-10-23, against the agent-talk clone at `8a7fc4f1` (commit date 2026-09-10 confirmed) and the pi-mono clone at `a7b94a03` (coding-agent `1.4.0` confirmed). No code was executed.

- Checked every `agent-talk:` citation: `extensions/inbox-monitor.ts` (all line ranges), `package.json:11-27`, `.github/workflows/ci.yml:102-136`, `docs/pi-auto-receive.md`, `README.md:146-164,225`, `skills/init/SKILL.md` (541-547, 560, 578-594, 632-647), `bin/spool-writer.py`, `bin/follow.sh:15-16,136`, `bin/codex_wake.py:6-10`, and the `PI_SESSION_ID` grep (no hits). All hold, apart from the fixes below.
- Checked every `pi:` citation: `extensions/loader.ts:384-387`, `agent-session.ts` (1479-1486, 1873-1909, 1926-1935, 2357-2393, 3448-3455), `messages.ts:162-169`, `agent/src/agent-loop.ts:184-185,210-215,304`, `settings-manager.ts:723-724`, `package-manager.ts` (578-660, 666-672, 2198-2200, 2605-2616), and the `docs/` files packages/skills/usage/settings/message-types. All hold. The extension-discovery claim (only `inbox-monitor.ts` loads, and `extensions/codex/` and `extensions/opencode/` are skipped) is confirmed by `resolveExtensionEntries` at `package-manager.ts:578-606`.
- Re-measured the skill sizes: front-matter about 5.2 KB, `init` 47 KB, `receive` 14.6 KB, `send` 6.6 KB. They match.
- Fixed:
  - `DEFAULT_MAX_BYTES` is at `spool-writer.py:69`, not `:68`.
  - Pi doc paths now carry their `packages/coding-agent/` prefix (`usage.md`, `message-types.md`, `settings.md`).
  - Added `settings-defaults.ts:7` as the source for the `one-at-a-time` default.
  - Corrected "no dependencies beyond `node:fs`" to note the type-only Pi import.
- Added:
  - In 2.6, the spool writer's own admission that only Claude Code writes the session map (`spool-writer.py:48-52,78-82`). This strengthens the "broken in the documented flow" inference.
  - In 2.5, a note that records with no id are deduped by offset only.
- Not checkable here: retalk's behaviour (relay ack and delete on delivery, and where `name` comes from), which rests on agent-talk docs through `overview.md`, and the author's live 2-3 s test.
