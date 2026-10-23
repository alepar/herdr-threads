# agent-talk x OpenCode: message injection and wake

- Alternative: agent-talk, https://github.com/xhluca/agent-talk. The clone is shallow at `8a7fc4f1` ("release: 0.2.4"). Paths are relative to the clone root. No code from it was executed.
- Harness: OpenCode, https://github.com/anomalyco/opencode. The source read is the `v1.18.1` tag (`99f638d`, which matches the local binary) and `dev` HEAD `f9f463b`. OpenCode paths are marked `[oc 1.18.1]` or `[oc dev]`. Background is in `../../primers/opencode.md`.
- Tags: **VERIFIED** means read in source or docs (cited). **INFERRED** means my own reasoning, not run.

## 1. Support status

**Native, in-process push. Opt-in, and fragile in practice.** VERIFIED: `README.md:226`, which lists "opencode | Yes" for auto-receive.

agent-talk supports OpenCode with two pieces:

1. **Skills.** The repo's `skills/` directory is symlinked into OpenCode's skill dir: `ln -sfn "$PWD/agent-talk/skills" ~/.config/opencode/skills` (VERIFIED `README.md:166-188`). OpenCode then lists the 15 prose skills (`init`, `send`, `receive`, ...). The model runs `retalk` itself through its bash tool.
2. **A v1 server plugin**, `extensions/opencode/inbox-monitor.ts` (203 lines). It is copied by hand into `~/.config/opencode/plugins/` or `<project>/.opencode/plugins/`. It tails NDJSON spool files and pushes each new record into the session with `client.session.promptAsync` (VERIFIED `extensions/opencode/inbox-monitor.ts:63-201`, `docs/opencode-auto-receive.md:133-155`).

The plugin does nothing unless `AGENT_TALK_OPENCODE_SPOOLS` is set before OpenCode launches (VERIFIED `inbox-monitor.ts:55-61,184`).

No OpenCode-specific tests exist. `tests/` mentions OpenCode only in `validate_plugin.py:226-229`, which checks that the doc file exists (VERIFIED by grep). The author's verification was a headless `prompt_async` call against OpenCode v1.17.20 "without a provider login". The model actually replying was not exercised (VERIFIED `docs/opencode-auto-receive.md:157-175`).

The end-to-end pipeline:

```
relay --(HTTP poll, default 60 s)--> retalk receive --follow  (bin/follow.sh)
      --> spool-writer.py  (stamps ts, fans out to registered session spools, plus legacy inbox.ndjson)
      --> <spool>.ndjson  (mode 0600, flock append, 8 MiB rotation)
      --> OpenCode plugin: fs.watch + 1 s poll, byte offset, id Set
      --> client.session.promptAsync({path:{id: activeSession}, body:{parts:[{type:"text", text}]}})
      --> OpenCode persists a real user message and starts or steers the agent loop
```

## 2. Injection mechanism

### Exact API

The plugin calls `client.session.promptAsync`, which is `POST /session/:id/prompt_async`. The `client` is the SDK v1 client that OpenCode hands to every server plugin. In a stock TUI that client is bound to the worker's in-process fetch, so no network port is needed. VERIFIED: `[oc 1.18.1] packages/opencode/src/plugin/index.ts:141-146` builds the client with `fetch: Server.Default().app.fetch` when there is no server URL.

On the server side, the handler runs `requireSession`, forks `promptSvc.prompt(...)`, and returns 204 at once (VERIFIED `[oc 1.18.1] packages/opencode/src/server/routes/instance/httpapi/handlers/session.ts:311-328`). A failure inside the forked prompt is only logged and published as `session.error`.

### Code path

1. **Load** (`inbox-monitor.ts:63,184,203`). The module exports the same function as a named export and as the default export. OpenCode 1.18.1 has no `server` key here, so it takes the legacy path and iterates `Object.values(mod)`. A `seen` Set dedupes the two exports, so the plugin is instantiated once (VERIFIED `[oc 1.18.1] plugin/index.ts:95-121`).
   - The plugin's state lives in `InstanceState`, which is one per project directory, not one per session (VERIFIED `[oc 1.18.1] plugin/index.ts:128-130`).
   - The source comment "the plugin function runs once per session" (`inbox-monitor.ts:182-183`) is therefore wrong. INFERRED: one plugin instance serves every session in that directory.
2. **Watch** (`inbox-monitor.ts:162-180`). Each spool path gets three things:
   - a start offset of `stat.size` (EOF), so the existing backlog is never pushed;
   - `fs.watch(path, () => drain(w))`;
   - a 1 s `setInterval(drain)` backstop, which also covers the case where the file does not exist yet.
3. **Drain** (`inbox-monitor.ts:118-160`). It reads `[offset, size)` and keeps a partial last line in `carry`. It splits on `\n`, runs `JSON.parse` per line, and silently skips lines that do not parse. A spool that shrinks (`size < offset`) resets the offset to 0, which handles rotation and truncation.
4. **Filter** (`inbox-monitor.ts:101-110`):
   - drops `kind === "contact"`;
   - drops records with empty `text`, which includes `group_leave` control records;
   - drops ids already in `seenIds`.
   The id is added to the Set **before** injection is attempted.
5. **Inject** (`inbox-monitor.ts:73-99`):

   ```ts
   await client.session.promptAsync({
     path: { id: activeSession },
     body: { parts: [{ type: "text",
       text: `New agent-talk message from ${who}:\n\n${text}` }] },
   });
   ```

   `who = rec.name || rec.from || "a peer"`. If no session is known yet, the record is pushed onto an in-memory `pending` array.
6. **Session latch** (`inbox-monitor.ts:186-199`). The `event` hook takes `props.sessionID ?? props.info.id` from **any** bus event. When that id differs from `activeSession`, it switches to it and flushes `pending`.
   - In 1.18.1, almost every session-scoped event carries a top-level `sessionID`. That includes `message.updated`, `message.part.updated`, `session.status`, `session.created`, `session.updated` and **`session.deleted`** (VERIFIED `[oc 1.18.1] packages/schema/src/v1/session.ts:571-622`).
   - OpenCode delivers only events whose `location.directory` matches the plugin's directory (VERIFIED `[oc 1.18.1] plugin/index.ts:251-258`).

### Payload format and framing

- The payload is one plain `text` part: `New agent-talk message from <name>:\n\n<body>` (VERIFIED `inbox-monitor.ts:88`; the body is `trim()`med first, `:78`). The fixed framing is 31 bytes plus the name.
- The payload has no message id, no timestamp, no fingerprint (when a name is present) and no group or room name.
  - `group` and `group_id` are dropped. grep finds no `group` in any extension (VERIFIED).
  - So a group-room message and a direct message look identical to the model, even though the receive skill requires group mail to be threaded by `group_id` (VERIFIED `skills/receive/SKILL.md:58-91`). INFERRED: the model may reply 1:1 to a room message.
- The part is not marked `synthetic`. It becomes an ordinary user message, shown in the TUI as if the human had typed it (VERIFIED: primer section 3; the user-message renderer at `[oc 1.18.1] packages/tui/src/routes/session/index.tsx:1359-1368` drops only `synthetic` text parts; `:383-387` is the same filter for scroll navigation).
- The text gives no instruction to the model, such as "render it" or "reply". What the agent does next depends on the `receive` skill being in context, or on the model's own judgment (INFERRED).
- Trust: the body of a remote, possibly cross-organization peer arrives with **user authority**. The framing only says who sent it. The Codex wake path refuses to push bodies for exactly this reason (VERIFIED `bin/codex_wake.py:6-10`). The OpenCode path ignores that reasoning (INFERRED inconsistency, as in the overview).

### When it fires

- **Idle session:** at once. The user message starts the loop (VERIFIED primer section 3; `[oc 1.18.1] session/prompt.ts:1052-1071`).
- **Busy session:** the message is persisted at once. The running loop picks it up at the next step boundary, after the current LLM call or tool round, so it is steered into the current turn rather than rejected (VERIFIED by reading, primer section 3: `effect/runner.ts:115-122`, `prompt.ts:1092,1111-1116`). The TUI tags it ` QUEUED `.
- **No session yet:** OpenCode is open on its home screen, or no session-scoped event has fired. The record is buffered and flushed into **whichever session first emits an event** (INFERRED from `inbox-monitor.ts:68-71,195-198`).
- **Process start:** never for backlog, because the watcher starts at EOF (VERIFIED `inbox-monitor.ts:165-171`).

### Size bounds

There are none. A body of any size goes in one prompt, and a drain reads the whole appended range into one `Buffer` (VERIFIED `inbox-monitor.ts:135-138`). The only limit is the spool writer's 8 MiB rotation (VERIFIED `bin/spool-writer.py:69,128-140`). There is no truncation, no paging and no batching. Each record is its own `promptAsync` call and its own user message.

### Dedupe and acknowledgement

- **Dedupe:** a per-spool byte offset plus a `Set` of message ids. Both are **in memory only** (VERIFIED `inbox-monitor.ts:49,65`). A restart forgets both, and the EOF start avoids re-delivery at the cost of never delivering the gap.
- **Ack:**
  - The relay-level ack is done by the follower (`retalk receive` is "auto-acked"; VERIFIED `skills/receive/SKILL.md:103`) before the record reaches the spool.
  - The plugin has no ack of its own, no read receipt and no delivery confirmation.
  - A failed `promptAsync` usually loses the message. The SDK v1 client does not throw on non-2xx responses unless `throwOnError` is set; it returns `{error}` (VERIFIED `[oc 1.18.1] packages/sdk/js/src/gen/client/client.gen.ts:165-179`, `sdk/js/src/error-interceptor.ts:1-20`).
  - So the plugin's `catch` re-buffer (`inbox-monitor.ts:93-98`) runs only on a thrown fetch error, not on a 404 for a gone session. On a 404 the record has already entered `seenIds` and is dropped silently. INFERRED from VERIFIED parts; not run.

## 3. Wake mechanism

- **Mechanism:** the same call. In OpenCode v1, `promptAsync` both injects and wakes. Injection and wake are one act: every message that passes the filter starts or extends a turn. There is no silent mode. OpenCode offers `noReply: true` for that (primer section 3), and agent-talk does not use it (VERIFIED by absence).
- **Idle/busy detection:** none in agent-talk. It relies on OpenCode's server queueing and steering, which handles busy sessions correctly for delivery: nothing is rejected and there is no double run (INFERRED from VERIFIED OpenCode internals). The plugin never reads `session.status` and never calls `GET /session/status` (VERIFIED by absence). The effect is that a message arriving mid-turn lengthens the current turn.
- **Draft safety: structurally safe.** `prompt_async` writes to the session store and never touches the TUI composer (VERIFIED primer section 1). The author explicitly rejected `/tui/append-prompt` plus `/tui/submit-prompt` because "it can collide with text the user is composing" (VERIFIED `docs/opencode-auto-receive.md:80-86`).
  - INFERRED residual effect: the human's draft survives, but a turn starts under them. If they then press Enter, their prompt is queued behind the agent's reaction to the peer.
- **Latency:**
  - The relay poll interval dominates. The default is 60 s, and the skill suggests `--interval 5` for live exchanges (VERIFIED overview; `bin/follow.sh:15-16`).
  - Spool to plugin takes 1 s or less (an `fs.watch` event, or the poll).
  - The plugin call returns at once. The LLM turn starts after that.
- **Failure modes:**
  1. **Wrong-session injection (latch drift).** The latch follows any event's `sessionID` (`inbox-monitor.ts:190-198`), so the target is "the last session that emitted anything". INFERRED cases:
     - A subagent spawned by the `task` tool runs in a child session whose events carry its own `sessionID`. A peer message landing while a subagent runs, or before the parent emits its next event, is injected into the **child** session. That child is invisible in the main view and terminates with the subagent's result. The message is effectively lost to the main agent and the human.
     - Two sessions in one TUI, or a background session still streaming, make the target nondeterministic.
     - A `session.deleted` event carries `sessionID` (VERIFIED schema), so deleting a session in the session list **latches onto the deleted session**. Every later `promptAsync` returns 404 as `{error}`, which is not thrown, so every message is dropped until another session emits an event.
  2. **The documented spool is never written** (INFERRED, high confidence; this sharpens the overview's flaw).
     - The docs and init step 4c tell the user to set `AGENT_TALK_OPENCODE_SPOOLS="<user>/sessions/<session-id>.ndjson"` (VERIFIED `skills/init/SKILL.md:609-612`, `docs/opencode-auto-receive.md:145-147`).
     - The spool writer copies records only to sessions listed in `~/.agent-talk/by-session/` (VERIFIED `bin/spool-writer.py:108-125,189-194`). Its own docstring says "Only Claude Code writes the session map, so on every other host that is the normal case" (VERIFIED `bin/spool-writer.py:48-52`).
     - Step 4c registers nothing, unlike Codex step 4d (VERIFIED `skills/init/SKILL.md:596-619` vs `skills/init/SKILL.md:632-647`), and the Claude Code registration step 4 is explicitly replaced by 4c on OpenCode (VERIFIED `skills/init/SKILL.md:547`).
     - OpenCode exposes no session id to the shell by default (VERIFIED primer section 10), and the path has to be chosen before OpenCode starts anyway. So `<session-id>` has no meaningful value here.
     - What does work today: the plugin's own header says to watch `<user>/inbox.ndjson` (VERIFIED `inbox-monitor.ts:8,24`). The writer still appends there because `--legacy` defaults on (VERIFIED `bin/spool-writer.py:28-30,195-196,264-266`).
     - That fallback is marked "on by default for one release". The doc, the skill and the code disagree, and the one setup that works is slated for removal.
     - Also, `--gc` deletes spools whose sid is unregistered (VERIFIED `bin/spool-writer.py:235-246`). A hand-created, unregistered session spool would be swept.
  3. **Restart gap.** Messages that arrive while OpenCode is down, or before the plugin loads, are acked at the relay and never pushed. Recovery needs the model to run `history` or `receive` (VERIFIED EOF start; INFERRED consequence).
  4. **Unordered sends.** `deliver` calls `void inject(rec)` without awaiting (`inbox-monitor.ts:109`). A burst of records produces concurrent `promptAsync` calls. Order in the session depends on fork scheduling (INFERRED; likely in order with in-process fetch, not guaranteed).
  5. **Rotation edge.** At 8 MiB the writer renames the file to `.1` and starts a new one (VERIFIED `spool-writer.py:137`). Bytes appended to the old inode after the last drain and before the rename are never read. `fs.watch` normally fires first, so the window is about 1 s or less (INFERRED).
  6. **`--pure` / `OPENCODE_PURE`** disables external plugins (VERIFIED `[oc 1.18.1] plugin/index.ts:177`), which silently turns off auto-receive.
  7. **OpenCode 2.x.** The plugin targets the v1 hook object (`@opencode-ai/plugin` `Plugin`). On `dev` (2.x) the plugin API is rewritten, and no v1 loader (`getLegacyPlugins`) or `promptAsync`/`prompt_async` reference was found in code under `packages/` (VERIFIED by grep at `f9f463b`; the only hits are the web docs, e.g. `packages/web/src/content/docs/server.mdx:176`, which still document `prompt_async`). INFERRED: the plugin breaks on 2.x unless a v1 shim appears. Whether one will is unknown.

## 4. Setup and installation footprint

| What | Where | Written by | Evidence |
| --- | --- | --- | --- |
| Skills symlink | `~/.config/opencode/skills` -> checkout `skills/`, or a project's `.opencode/skills`. `ln -sfn` **replaces** any existing `skills` entry. INFERRED: a real `skills/` directory there would make `ln` place the link inside it instead. | User, by hand | VERIFIED `README.md:176` |
| Plugin file | `~/.config/opencode/plugins/inbox-monitor.ts`, or `<project>/.opencode/plugins/`, or an npm spec in `opencode.json` `"plugin"`. The npm package is not published as far as the repo shows: `package.json` describes the pi package. | User, by hand (a copy, so it does not update with `git pull`) | VERIFIED `docs/opencode-auto-receive.md:135-139`, `skills/init/SKILL.md:602-603` |
| Env var | `AGENT_TALK_OPENCODE_SPOOLS=<colon list>`, which must be set at launch and requires a relaunch to change | User shell | VERIFIED `skills/init/SKILL.md:606-612` |
| Follower | `setsid nohup` loop `retalk receive --follow \| spool-writer.py`, pid files under `<user>/` | Model, through skills | VERIFIED `bin/follow.sh:72-115` |
| State | `~/.agent-talk/users/<name>/` (identity, contacts, `check-mode`, `receive-from`, spools) | Model, through skills | VERIFIED overview |

Nothing writes `opencode.json`, `tui.json`, MCP config or permissions. No `shell.env` hook exports identity, so the model has to carry `--dir <user>/identity` paths in its own memory (VERIFIED by absence in `inbox-monitor.ts`).

## 5. Token cost

- **Per message:**
  - Framing is 31 bytes plus the name plus the full body, as one persisted user message, so it stays in history for the rest of the session and is re-sent on every later request.
  - When the session is idle, each message costs **at least one full agent turn**: system prompt, history and tool loop. The `receive` skill also tells the model to render a transcript with "1-3 turns of context", which may add `retalk history` tool calls (INFERRED from `skills/receive/SKILL.md:33-57`).
  - A burst of N messages yields N user messages. They coalesce into one run only if they land while it is running (INFERRED).
- **Per quiet turn:** the plugin adds **0 bytes**, since it registers no `chat.message`, `messages.transform` or `system.transform` hook (VERIFIED: only `event` is returned, `inbox-monitor.ts:186-200`). The skills add a standing system-prompt cost: 15 skills whose front matter totals about 5.3 KB, roughly 1.3k tokens if all of it is listed (VERIFIED byte count over `skills/*/SKILL.md`; INFERRED that OpenCode lists name plus description per skill). Full skill bodies (about 117 KB in total) load only on use.
- **No priority or threshold.** Every message wakes, so a chatty peer spends the recipient's quota at will (VERIFIED by absence).

## 6. Limitations and bugs

1. Documented spool path versus registry. The documented configuration delivers nothing; only the legacy `inbox.ndjson` path works (section 3, failure 2). This is a bug.
2. The session latch is too greedy: child sessions, deleted sessions and background sessions can all capture delivery (section 3, failure 1). The fix is in reach: track `api.route.current` from a TUI plugin, or latch only on `session.status` or user `message.updated` events of root sessions (no `parentID`). This is a bug.
3. Silent loss when a session is gone. `promptAsync` returns `{error}` and does not throw, so the re-buffer never runs, and the id is already marked seen (section 2). This is a bug.
4. The comment "runs once per session" is wrong; the plugin runs once per directory instance. This is a documentation defect that hides failure 1.
5. Room context is dropped: group and direct mail are framed identically (section 2).
6. Contact requests are not consumed. There is no consumer for `<sid>.requests.ndjson` or `requests.ndjson` on OpenCode (VERIFIED by absence), so invite registrations surface only through the `id` skill's status block.
7. Peer bodies arrive with user authority, unbounded, as visible pseudo-human messages.
8. Every message triggers a turn: there is no silent staging (`noReply`) and no batching.
9. Delivery state lives only in memory, and the watcher starts at EOF.
10. The plugin is a copied file, so updates need a manual re-copy. The skills, by contrast, are a symlink.

## 7. Clever ideas worth stealing

1. **An in-process server plugin as the wake channel.** It uses the plugin's pre-bound `client` instead of an external HTTP attach. This sidesteps the fact that a stock TUI opens no port (primer section 1) and the port-discovery problem the author cites, [opencode#9099](https://github.com/anomalyco/opencode/issues/9099) (VERIFIED `docs/opencode-auto-receive.md:88-98`). For herdr-threads this is the natural "Channel" policy for OpenCode: the harness wakes itself and never touches the composer. It maps onto delivery-contract rule 6 the way the Claude mod does (INFERRED).
2. **`prompt_async` instead of TUI keystrokes or `/tui/*`.** The author's written reasoning for rejecting `append-prompt` and `submit-prompt` (draft collision) matches ours. OpenCode's server-side busy steering means a wake needs no idle check for correctness, only for token policy (VERIFIED primer section 3).
3. **An env-gated, inert-by-default plugin.** Installing it changes nothing until a session opts in (`inbox-monitor.ts:25-27`). A useful pattern for a global plugin dir shared by many sessions; we would gate on our own seat or binding instead of a user env var.
4. **`fs.watch` plus a 1 s poll backstop, with a shrink-resets-offset rule.** Cheap and robust on filesystems where `fs.watch` misses events. Codex's sha256 head fingerprint (overview) is the stronger variant.

What herdr-threads should do better than this, given the OpenCode surfaces in the primer (INFERRED design notes):

- Send the wake content-free, rule (c), with `synthetic: true`, and only when `p <= T` and `session.status` is idle.
- Stage `p > T` items with `noReply: true`, or better, deliver them in turn through `experimental.chat.messages.transform` or `tool.execute.after`.
- Bind to a seat by explicit session id, exported through `shell.env`, rather than by the latest event.
- Treat `{error}` returns as failures.

## Verification

Adversarial pass, 2026-10-23, against the agent-talk clone at `8a7fc4f` and the OpenCode `v1.18.1` (`99f638d`) and `dev` (`f9f463b`) clones.

- Checked and confirmed: every `inbox-monitor.ts` line citation (load, watch, drain, filter, inject, latch, catch re-buffer, unawaited `void inject`); `README.md:166-188,176,226`; `docs/opencode-auto-receive.md` ranges; `skills/init/SKILL.md:596-619,606-612,632-647`; `skills/receive/SKILL.md:33-57,58-91,103` (`group_leave` has no `text`); `bin/spool-writer.py` (8 MiB rotation, registry-only fan-out, `--legacy` default on, `--gc` sweep, docstring at 48-52); `bin/follow.sh:15-16,72-115`; `bin/codex_wake.py:6-10`; `tests/validate_plugin.py:226-229` as the only OpenCode test mention; 15 skills, front matter 5,388 bytes, bodies 116,746 bytes; no `group` in `extensions/`. OpenCode: `plugin/index.ts:95-121` (legacy path, `seen` dedupe), `:128-130` (`InstanceState`, keyed by directory in `effect/instance-state.ts`), `:141-146` (in-process fetch client), `:177` (`flags.pure`), `:251-258` (directory filter; `event` hooks are fired with `void`, not awaited); `handlers/session.ts:311-328`; `schema/src/v1/session.ts:571-622` (`session.deleted` carries `sessionID`); `client.gen.ts:165-179` and `error-interceptor.ts` (no throw without `throwOnError`, default `false` on `promptAsync`); `prompt.ts:1052-1071` (`noReply` returns before `loop`), `:1092`, `:1111-1116`; `runner.ts:115-122`; ` QUEUED ` at `tui/.../session/index.tsx:1436`.
- Changed: framing size 30 -> 31 bytes (`"New agent-talk message from "` is 28 bytes plus `":\n\n"`), and noted the body trim; TUI synthetic-hiding citation moved from the scroll-navigation filter (`index.tsx:383-387`) to the user-message renderer (`:1359-1368`); the 2.x grep claim narrowed, since `prompt_async` still appears in `dev` web docs (no code hits); added the `init:547` evidence that OpenCode skips session registration.
- Not checked: GitHub API metadata (not needed for any claim here); the 404 status for a deleted session (`requireSession` -> `mapStorageNotFound`, left INFERRED); subagent-session and ordering failure modes (INFERRED, not run).
