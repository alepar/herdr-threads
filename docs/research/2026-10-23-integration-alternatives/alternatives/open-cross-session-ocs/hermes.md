# open-cross-session (ocs) x Hermes Agent: message injection and wake

- Alternative: https://github.com/leeguooooo/open-cross-session. Clone `scratchpad/alts/open-cross-session-ocs`, HEAD `1a3cee23a` (= `v0.9.4`). It was read and not executed. Paths marked `ocs:` are relative to that clone.
- Harness: https://github.com/NousResearch/hermes-agent. Clone `scratchpad/alts/hermes-agent`, `main` `f3b5b980` (2026-10-23, newer than release v0.22.5). It was read and not executed. Paths marked `hermes:` are relative to that clone. The shared reference is the harness primer `../../primers/hermes.md`.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from what was read and not observed live. **unknown** means not found.

## TL;DR

- ocs sends a message into a Hermes session with one JSON-RPC call to the Hermes **host's loopback WebSocket**: `prompt.submit {session_id, text, queued: true}`. It installs no plugin, adds no hook and changes no Hermes config. The message body itself starts the turn: an idle session starts a turn, and a busy session queues the message behind the running turn. Nothing is interrupted, and there are no keystrokes. VERIFIED.
- Coverage is narrow. ocs only finds hosts that publish a token, which are the `desktop-serve` and `serve` roles (Hermes Desktop and `hermes serve`). Its own changelog says that a terminal `hermes` / `hermes --tui` "is not in the host, not supported yet". This was live-verified only on **Hermes 0.21.5 + Desktop**. VERIFIED.
- Since v0.22.1, terminal Hermes sessions run in the `gateway` host role. That role's record publishes no port and no token. So the ocs carrier almost certainly cannot reach the terminal panes that herdr-threads cares about (INFERRED, high confidence). The same *protocol* is reachable through the gateway's private control-socket ticket (`hermes_cli/gateway_client.py`). That makes it the best wake path available to herdr-threads for Hermes: content-carrying or content-free, queue-on-busy, and it cannot damage a draft.
- The most valuable fact to take from ocs: **`queued: true` is mandatory**. Without it, Hermes' default `busy_input_mode: interrupt` *redirects or interrupts* the running turn. VERIFIED on both sides.

---

## 1. Support status

**Native, for Hermes Desktop and `hermes serve` sessions only.** VERIFIED:

- `ocs:src/hermes.ts:1-24` (the module header comment) describes discovery and delivery. `ocs:src/hermes.ts:35` reads:
  ```ts
  export const HERMES_HOST_ROLES = ["desktop-serve", "serve"] as const;
  ```
- `ocs:CHANGELOG.md:96-105` (0.7.3, issue #40) covers the feature. Line 104 records the live verification: "macOS, Hermes Agent 0.21.5 + Hermes Desktop". The flow it describes: a Claude Code on another Mac sends a LAN DM, the Hermes conversation starts a turn, Hermes replies through `ocs`, and a third message reports `queued` while the session is busy. Line 105 states the limitation: "只覆盖 Hermes Desktop / `hermes serve` 托管的会话；终端里单独跑的 `hermes` / `--tui` 不在宿主里，暂不支持". In English: "only covers sessions hosted by Hermes Desktop / `hermes serve`; a standalone `hermes` / `--tui` in a terminal is not in the host, not supported yet".
- `ocs:README.md:243`: "Hermes Desktop (or `hermes serve`) running as the same user, with the session open."
- `ocs:src/capabilities.ts:10` declares `busyDelivery: "queued"` for `hermes` and `deliveryReceipt: "none"` (line 12).
- Tests: `ocs:test/hermes.test.ts` runs against a fake host (`fakeHermes`, lines 48-105). There is no live Hermes in CI (INFERRED from the fake).

**Status for the terminal Hermes that herdr-threads targets: not supported.** The current Hermes source supports this:

- Hermes has three host roles: `ROLE_GATEWAY = "gateway"`, `ROLE_SERVE = "serve"` and `ROLE_DESKTOP_SERVE = "desktop-serve"` (`hermes:gateway/host_rendezvous.py:65-73`). VERIFIED.
- The gateway role publishes `hr.publish_record(hr.ROLE_GATEWAY, profiles=(), home=...)` (`hermes:gateway/run.py:4887`, again at `:5039`) and later `publish_record(ROLE_GATEWAY, profiles=..., home=...)` (`hermes:gateway/run_adapters.py:1062-1065`). Neither call passes `host`, `port` or `token` (the signature is at `hermes:gateway/host_rendezvous.py:597-627`). Only serve and desktop-serve publish host, port and token (`_publish_host_rendezvous`, `hermes:hermes_cli/web_server.py:1078-1128`). VERIFIED.
- Classic `hermes chat` / `--cli` "use the gateway authority" through "a single-use private control ticket" (`hermes:website/docs/developer-guide/gateway-classic-cli.md:8-13`). The client mints the ticket over the profile's private control socket and dials `/api/ws` with subprotocol `hermes-gateway-ticket.<ticket>` (`hermes:hermes_cli/gateway_client.py:37-43, 225-258`). VERIFIED.
- Consequence (INFERRED): from v0.22.1, a terminal Hermes session is owned by the `gateway` process. ocs never looks at that role, and the role has no `?token=` credential to offer in any case. Two things are **unknown**: whether a running `hermes serve` lists or forwards gateway-owned terminal sessions in its own `session.active_list` (there is some owner-forwarding machinery, `hermes:tui_gateway/methods_prompt.py:705-760`, see section 6), and whether ocs's role list will grow.

---

## 2. Injection mechanism

### 2.1 Exact API

Hermes' gateway JSON-RPC 2.0 runs over the loopback WebSocket that the Desktop window itself uses. VERIFIED, `ocs:src/hermes.ts:200-297`:

```ts
const url = `ws://${hostPart}:${host.port}/api/ws?token=${encodeURIComponent(host.token)}`;
...
ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
```

Every delivery opens a fresh connection and makes two calls (`ocs:src/hermes.ts:356-398`):

```ts
const live = parseActiveList(await connection.call("session.active_list", {}))
  .find((session) => session.key === sessionKey);
if (live === undefined) return { ok: false, reason: "not-open", ... };
const reply = await connection.call(
  "prompt.submit",
  { session_id: live.runtimeId, text: note, queued: true },
  { onWritten: () => { submitted = true; } },
);
```

- `session.active_list` maps the durable `session_key` (the ocs address) to the 8-hex **runtime id** that the RPCs take. The runtime id is "resolved fresh before each send ... never stored" (`ocs:src/hermes.ts:13-14`, `:306-325`). VERIFIED.
- On the Hermes side, `PromptSubmitParams` has `text`, `queued: bool | None` ("client queue drain — the busy path must hold it, never redirect/steer"), `display_kind`, `surface` and others (`hermes:tui_gateway/contracts/prompt_voice.py:26-50`). The result `status` is one of `streaming | queued | steered | redirected` (`:56-61`). VERIFIED.

### 2.2 Credential discovery and hardening (VERIFIED, `ocs:src/hermes.ts:79-162`)

- Lock dir: `$HERMES_GATEWAY_LOCK_DIR`, else `$XDG_STATE_HOME/hermes/gateway-locks`, else `~/.local/state/hermes/gateway-locks` (`:79-85`). This matches Hermes' `host_state_dir()` (`hermes:gateway/host_rendezvous.py:12-13, 163-167`).
- For each role, ocs reads `host-<role>.json` and `host-<role>.token`. Each file must be a regular file and not a symlink, must be at most 16 KiB (record) or 4 KiB (token), must have mode `& 0o077 == 0`, and must be owned by our uid (`:109-116`). The record's role must match, its pid must be alive (`kill(pid,0)`; `EPERM` counts as alive), and its host must be loopback ("The token goes in the URL: never hand it to anything but loopback", `:132-133`). The sha256 of the token, first 16 hex characters, must equal the record's `tokenFingerprint` (`:142-145`). This is the same derivation as `hermes:gateway/host_rendezvous.py:229-231`.
- Both files are re-read on every connect, because "the token changes with each backend restart" (`:6-7`).

### 2.3 When it fires

Only at send time. `ocs dm hermes-<id>` (`ocs:src/deliver.ts:505-512`) or an `@hermes-<id>` mention in `ocs send` (`ocs:src/cli.ts:606-624`, via `splitWakeMentions`, `ocs:src/wake.ts:186-202`) runs it synchronously in the sender's CLI process. There is no hook, no turn-start pull, no post-tool delivery and no daemon. VERIFIED. If the target is offline, the message stays in the log, and "Persistence has no auto-nudge" (`ocs:README.md:349-351`, per the overview). Nothing re-delivers it when the session reopens. VERIFIED.

Delivery receipts go back to the sender through the same carrier (`deliverNotice`, `ocs:src/deliver.ts:330-367`). ocs strips its own session env vars first (`SELF_ENV_KEYS`, `:295-305`), so the self-wake guard cannot swallow the notice. VERIFIED.

### 2.4 Payload and framing (VERIFIED, `ocs:src/wake.ts:117-171`, `ocs:docs/wake-protocol.md:28-75`)

The `text` is a plain "wake note". Hermes gets **no** `<cross-session-message>` XML wrapper: that wrapper is added only on the Claude UDS path. `deliver.ts:507-509` and `cli.ts:613` pass the output of `wakeNote(...)` straight through:

```
[ocs wake] <sender> mentioned you in #<channel> (seq <N>[, reply to seq <M>][, <ago>])

<body>

Reply when asked, or with a needed decision/result. No unsolicited acknowledgements.
Reply: ocs send <channel> "<your reply>" --reply-to <N>      (or: ocs dm <sender> "<your reply>")
Thread: ocs read <channel>
```

- For Hermes, `implicitReceiver: true` drops `--as`, because the receiver's identity is inferred from `HERMES_SESSION_ID` (`ocs:src/deliver.ts:509`, `ocs:src/cli.ts:613`).
- Hermes renders the note as an ordinary **user bubble**. "a JSON client cannot set another author", so the header line alone marks the text as cross-session data (`ocs:src/hermes.ts:18-20`, `ocs:CHANGELOG.md:100`). VERIFIED. Hermes also offers `display_kind` (only `"hidden"` is honoured, `hermes:tui_gateway/contracts/prompt_voice.py:33`), and ocs does not use it. VERIFIED. On the legacy contract a client-supplied `_turn_author` is refused with 4124 (`prompt_voice.py:48-51`). The canonical contract accepts `turn_author {id, name, is_bot, origin}`, but for "memory attribution only" (`hermes:tui_gateway/contracts/canonical_sessions.py:190-195, 222-225`); whether it changes the rendered author is unknown.
- Neutralization: in the body, `<cross-session-message` becomes `‹…`. Lines that start with `Reply:`, `Thread:`, `[ocs wake]` or the notice headers get a `> ` prefix, so a sender cannot forge a reply command (`ocs:src/wake.ts:85-98`). VERIFIED.

### 2.5 Size bounds (VERIFIED)

- Body: inlined verbatim up to 4096 bytes, measured after neutralization. Above that, ocs inlines the first 512 bytes, cut on a character boundary, plus `… (<total> bytes total; full text: ocs read <ch>)`. The skeleton may use at most 1024 bytes and shrinks by dropping `ago`, then the sender. The `Reply:` and `Thread:` lines are never dropped. The note never exceeds 5120 bytes (`ocs:src/wake.ts:40-46, 131-171`).
- The Hermes carrier also refuses anything empty or over `HERMES_WAKE_MAX_BYTES = 16 KiB` (`ocs:src/hermes.ts:38, 361-364`). The 5120 cap binds first.
- Hermes-side collapse: the 5-line / 2000-char paste-collapse thresholds in the primer belong to the prompt_toolkit composer. They do not apply to RPC text (INFERRED; RPC text bypasses the composer).

### 2.6 Dedupe and acknowledgement (VERIFIED)

- At-most-once. `entry.written = true` is set *before* `ws.send` (`ocs:src/hermes.ts:245-249`). Any transport failure after that point is `unknown-outcome` (exit 3, "frame was written — do NOT resend", `ocs:src/i18n.ts:377-378`). A JSON-RPC error reply counts as a definite refusal: `failed`, "nothing was enqueued" (`ocs:src/hermes.ts:385-388`). A reply status other than `streaming` or `queued` is also `unknown-outcome` (`:380-383`).
- There is no ACK and no read receipt. `accepted` is "not a read receipt". The reader's position is a per-consumer cursor that `ocs read` advances (overview, `ocs:README.md:249-281`). ocs deduplicates nothing for the model. Hermes does some of it: a text-only submit identical to the in-flight prompt is dropped, and text-only submits that arrive while busy are merged into one queued slot (`hermes:tui_gateway/session_auto_continue.py:147-176`, section 3.2). VERIFIED.

---

## 3. Wake mechanism

### 3.1 Exact mechanism

Injection and wake are one act. `prompt.submit` on an idle session starts a turn and returns `streaming`. ocs reports `wake(hermes): started a turn` (`ocs:src/hermes.ts:381`, `ocs:src/i18n.ts:375`). VERIFIED.

### 3.2 Idle and busy detection

ocs does no detection of its own. It reads the `status` field of `session.active_list` (`idle | working | starting | waiting`) for `ocs who` (`ocs:src/hermes.ts:170-171, 321`, `ocs:src/roster.ts:322-330`), but it never gates on that field. Hermes decides. VERIFIED:

- `_handle_busy_submit` (`hermes:tui_gateway/session_auto_continue.py:400-452`):
  ```python
  mode = "queue" if queued else _load_busy_input_mode()
  ```
  "``queued=True`` (client queue drain) forces queue mode: a 'run after' message must NEVER become a live correction." With `queued` absent, the default mode `interrupt` redirects the running turn, "falling back to hard interrupt + queue". ocs notes the same thing: "`queued` is mandatory — without it Hermes' default busy mode interrupts the running turn" (`ocs:src/hermes.ts:16-17`).
- In queue mode the prompt is enqueued, and it is "Durable AT ACCEPT ... a cold resume sees the queued message and a backend restart cannot lose it" (`session_auto_continue.py:436-442`). The reply is `{"status": "queued"}`. Hermes coalesces on its side. `_enqueue_prompt` says "Text-only arrivals share a slot and merge losslessly": a plain-string submit without `turn_author` is appended to the existing `queued_prompt` text with a blank line between (`hermes:tui_gateway/session_auto_continue.py:147-176`). ocs sends exactly that shape. So N ocs notes that arrive during one busy turn become **one** follow-up turn whose user text is the N notes joined, and `_drain_queued_prompt` fires it when the turn ends (`:457-472`). VERIFIED in code, not observed live. This is the Codex pathology ocs fixed client-side in 0.7.4 (#41, `ocs:CHANGELOG.md:84-92`); for Hermes the host already prevents it, so ocs needs no fix there.
- Race: if the turn ends between the busy check and the enqueue, `_handle_busy_submit` returns `None`, and the caller retries on the idle session (`methods_prompt.py:888-907`). This race is handled server-side. VERIFIED.
- Side effect: a queued submit sets `session["_turn_user_input"] = True`, meaning "the running turn no longer counts as unattended" (`session_auto_continue.py:415-416`). The effect on unattended or auto-continue policy is unknown.

### 3.3 Draft safety

Safe by construction. The text never touches a composer buffer. It goes to the session's admission path, and the Desktop or TUI composer keeps whatever the human is typing (INFERRED from the code path; the primer section 4 makes the same inference). ocs does no screen reading for Hermes, unlike its cmux fallback (`ocs:src/roster.ts:248-275`). VERIFIED.

### 3.4 Latency

Per send: two file reads, one WebSocket handshake (`HERMES_CONNECT_TIMEOUT_MS = 3000`), one `session.active_list`, and one `prompt.submit` (`HERMES_RPC_TIMEOUT_MS = 10000`) (`ocs:src/hermes.ts:36-37`). On a healthy loopback the wake should land in well under a second, with the turn starting immediately (INFERRED; no timing evidence was published). In the worst case a send blocks the sender's CLI for 13 s before it reports `unknown-outcome`.

### 3.5 Failure modes (VERIFIED unless marked)

| Condition | ocs result | Exit |
|---|---|---|
| No record or token, wrong mode or owner, non-loopback host, dead pid, fingerprint mismatch | `unavailable` (stored only) | 2 |
| Session not in `session.active_list` | `not-open` (stored only, no later re-delivery) | 2 |
| JSON-RPC error reply | `failed`, "nothing was enqueued" | 2 |
| Socket dies or times out after the frame left | `unknown-outcome`, never resent | 3 |
| Reply status not `streaming` or `queued` | `unknown-outcome` | 3 |

- Host restart race: the token rotates. The fingerprint check turns a half-written pair into `unavailable` ("host restarting?") rather than an auth failure (`ocs:src/hermes.ts:143-144`).
- **Misclassified refusal (INFERRED bug on current Hermes).** Hermes can answer `prompt.submit` with JSON-RPC error `4090`, reason `SESSION_OWNER_UNCONFIRMED`: "did not confirm your message in time. It may already be queued there: check the chat before sending it again" (`hermes:tui_gateway/methods_prompt.py:712-750`). ocs maps every RPC error to `failed` / "nothing was enqueued" (`ocs:src/hermes.ts:385-388`) and tells the user "message is already stored; do not resend" (`ocs:src/i18n.ts:379-380`). So the at-most-once wake survives. But the outcome is reported as a definite non-delivery when it may in fact have been delivered. A sender that re-wakes manually could produce a duplicate turn.
- The ocs connection rebinds as a session viewer and then closes immediately. On submit, Hermes calls `_rebind_live_transport(sid, session, t)` for the current transport (`hermes:tui_gateway/methods_prompt.py:876-882`). That function *adds* a viewer without displacing the existing ones (`hermes:tui_gateway/session_lifecycle.py:914-932`). The Desktop window keeps its stream. Whether a turn started on a session whose only viewer was ocs (a headless `hermes serve` with no window attached) survives the ocs socket closing depends on the ws-orphan reaper, which interrupts only detached turns without fresh activity (`session_lifecycle.py:935-944`). Probably fine, but unknown.
- The submit also resets per-session client state. `session["client_surface"] = params.get("surface") if ... else ""` and `voice_turn = params.get("voice_turn") is True` (`hermes:tui_gateway/methods_prompt.py:860-866`). An ocs message clears a `hud` or `voice-live` surface flag for that turn (VERIFIED code; impact INFERRED minor).

---

## 4. Setup and installation footprint on the harness

**None.** VERIFIED: "不装插件、不改 Hermes 配置", meaning no plugin and no Hermes config change (`ocs:CHANGELOG.md:98`). ocs only reads Hermes' rendezvous files. It writes no Hermes plugin, no `config.yaml` hooks and no `plugins.entries`. It does not install its skill into Hermes either: `ocs skill install` writes `~/.claude/skills/ocs`, `~/.codex/skills/ocs`, `$GROK_HOME/skills/ocs` and Pi's skill dir (`integrationPaths`, `ocs:src/cli.ts:1675-1683`). `~/.agents/skills/ocs` is only scanned for updates of a copy installed by other means (`ocs:src/upgrade.ts:150-151`). Whether Hermes loads `~/.agents/skills` is unknown. The only `.agents/skills` references found in Hermes are in the skills-hub importer (`hermes:tools/skills_hub_skillssh.py:56`).

Requirements on the user's side: Hermes Desktop or `hermes serve` running as the same OS user, with the session open, and the `ocs` binary on the Hermes gateway's terminal `PATH` so the model can run the `Reply:` command. On v0.22+ that `PATH` is the gateway's env plus the client-env allowlist, per primer section 1 (INFERRED).

Identity inside Hermes (VERIFIED, `ocs:src/hermes.ts:68-77`):

```ts
// HERMES_SESSION_SOURCE marks a per-command session binding; a stray HERMES_SESSION_ID alone
// (exported by hand, leaked into a login shell) is not proof we are inside that session.
if (typeof source !== "string" || source === "") return null;
```

Hermes still bridges both variables per command (`HERMES_SESSION_SOURCE` and `HERMES_SESSION_ID` are in `_SESSION_VARS`, `hermes:gateway/session_context.py:34-48`). VERIFIED. In ocs's self-detection order, Hermes comes **last**, because "it injects HERMES_SESSION_ID into every terminal command; Claude/Codex opened inside Hermes inherit it too" (`ocs:src/roster.ts:91-92`). VERIFIED.

---

## 5. Token cost

- **Per message:** one full user turn per message. The note costs about 250-350 bytes of skeleton (roughly 70-100 tokens: header, the anti-chatter line, `Reply:`, `Thread:`) plus the body (up to 4096 bytes, about 1k tokens). The skeleton bytes are an estimate from the template (INFERRED). Each wake also pays for a full model turn over the cached history. Messages that arrive while the session is busy share one follow-up turn, because Hermes merges text-only queued prompts (VERIFIED in code, section 3.2); each still carries its full skeleton. A reply costs the model one `ocs send/dm` tool call. Reading longer bodies costs an extra `ocs read`.
- **Per quiet turn:** zero. No hook runs and nothing is injected (VERIFIED, no hooks). There is also no static skill text in the Hermes session, because ocs installs none there. The model learns the protocol only from the `Reply:` / `Thread:` lines in each note, which is why those lines are never dropped.

---

## 6. Limitations and bugs

1. **Terminal Hermes is unsupported** (VERIFIED for ocs; section 1). On v0.22.1+ every terminal session is gateway-owned, and that role exposes no token-auth WebSocket. This is the case herdr-threads cares about.
2. **Verified only on Hermes 0.21.5** (`ocs:CHANGELOG.md:104`). Hermes has since moved Desktop sessions onto an "authority-backed WebSocket", and legacy session methods now go through a restricted fallback. `session.active_list` is one of the allowed ones (`hermes:tui_gateway/ws_legacy_fallback.py:1-36`). Whether `?token=` on a serve backend still reaches `prompt.submit` for authority-owned sessions is unknown. The fallback also keeps every `prompt.*` method on the authority (`_SESSION_NAMESPACES`, `hermes:tui_gateway/ws_legacy_fallback.py:32-36`), and the authority's `prompt.submit` answers with an admission receipt whose `status` is `queued | started | unknown | terminal` (`hermes:tui_gateway/contracts/canonical.py:29-38`, `canonical_sessions.py:205-243`), never `streaming`. If an ocs connection is answered by the authority, an idle start comes back `started`, which ocs classifies as `unknown-outcome`, exit 3 (`ocs:src/hermes.ts:380-383`). INFERRED; which contract a `?token=` connection gets is unknown. Hermes' auth contract test still lists `/api/ws` as accepting "a token-bearing loopback client" (`hermes:tests/e2e/core/dashboard/test_dashboard_auth.py:50`), so the token path itself still exists (VERIFIED as a test spec).
3. **A busy burst becomes one merged turn, not N.** Hermes merges text-only queued prompts (`session_auto_continue.py:147-176`). The merged user text repeats each note's skeleton (VERIFIED in code).
4. **`4090 SESSION_OWNER_UNCONFIRMED` is reported as "not delivered"** when it may have been queued (section 3.5; INFERRED bug).
5. **No self-skip on the DM path.** `ocs dm` checks for self only on Grok (`ocs:src/deliver.ts:514-517`) and checks Hermes only for channel mentions (`ocs:src/cli.ts:608`). A Hermes session that runs `ocs dm hermes-<self>` would queue a message to itself (INFERRED; low impact).
6. **No re-delivery when a session reopens.** A message sent to a closed session is `not-open`, stored only, and never pushed later (VERIFIED).
7. **Provenance is honour-based.** The note is a user-role row that is indistinguishable from the human's typing in the transcript. Only the `[ocs wake]` header marks it (VERIFIED, `ocs:src/hermes.ts:18-20`). `display_kind: "hidden"` exists but would hide the message from the human as well (VERIFIED param; trade-off INFERRED).
8. **Private protocol.** ocs itself calls it "Hermes' private protocol" (`ocs:src/hermes.ts:23`). Hermes' rendezvous record has `protocolVersion` (`host_rendezvous.py:53-55`). ocs ignores that field, so a version bump would not be detected (VERIFIED: `lookupRole` reads only `pid`, `host`, `port`, `tokenFingerprint` and `role`).
9. **No priority or threshold model.** Every DM or mention wakes unless the sender passes `--no-wake`. This conflicts with herdr-threads' delivery contract rules 1-2 (VERIFIED in the overview).

---

## 7. Ideas worth taking (for herdr-threads)

1. **Wake Hermes through the gateway RPC instead of keystrokes, with `queued: true`.** herdr-threads currently types `MARKER` into the pane (primer section 11). On Hermes' default `busy_input_mode: interrupt`, a mistimed keystroke wake redirects the turn. A `prompt.submit {session_id, text, queued:true}` has none of these problems: the host decides between idle and busy, the turn is never interrupted, a draft is never touched, the message is durable on accept, and `streaming` versus `queued` comes back as an honest answer. For terminal sessions on v0.22+, herdr-threads would mint the ticket the way `hermes_cli/gateway_client.py:_session_ticket` does (control socket `verb: "session-ticket"`, `:225-258`). It cannot use ocs's `?token=` path. Doing so takes a Python-internal protocol dependency, so it should degrade to the keystroke wake on any refusal (consistent with AGENTS.md "degrade only the broken capability"). The ticket path speaks the canonical contract (`queued`, an admission id and `status` `started | queued | unknown | terminal`, `hermes:tui_gateway/contracts/canonical.py:29-38`; the classic CLI's `/queue` uses it, `hermes:hermes_cli/gateway_queue.py:51`), not ocs's legacy `streaming | queued` reply, so the outcome mapping must be written against that. The text can be the content-free `herdr-threads: wake up` (R18). The turn's `pre_llm_call` hook then injects the check-in (a digest or bodies, still to be checked, primer section 12.5). That keeps rule (c) and avoids an extra user row per message.
2. **`queued` turns every busy-path wake into run-after.** For herdr-threads this is also the safe default if it ever sends content through RPC. The alternative `session.steer` gives mid-turn delivery (primer section 5.1).
3. **Map durable key to runtime id fresh on every send, never cache it** (`ocs:src/hermes.ts:13-14`). This is robust across host restarts and session reattach.
4. **Rendezvous hardening:** owner-only mode, uid check, no symlinks, size caps, a loopback-only host before a credential goes into a URL, pid liveness, and a fingerprint check against the published record (`ocs:src/hermes.ts:109-147`). This is a cheap checklist for herdr-threads' own discovery of any host endpoint.
5. **Never claim more than the frame proves.** The `written` flag is set before `send` and splits `unavailable`, `failed` and `unknown`. It maps cleanly onto herdr-threads' wake receipts. One correction is needed: an RPC error with an "it may already be queued" reason must stay `unknown`, which ocs gets wrong.
6. **Require two signals for self-identity:** `HERMES_SESSION_ID` together with `HERMES_SESSION_SOURCE`, and rank Hermes last, because a Claude or Codex started from a Hermes terminal inherits those variables (`ocs:src/hermes.ts:68-77`, `ocs:src/roster.ts:91-92`). This is directly relevant to herdr-threads' `caller_env` for Hermes (`session_var: "HERMES_SESSION_ID"`, primer section 11). It may also help fix the gateway-env attribution problem described in primer section 12.1.
7. **Coalescing is the host's job on Hermes.** Text-only queued submits merge into one turn (section 3.2), so an RPC wake does not multiply turns while busy. herdr-threads should still send at most one wake per idle edge; a content-free wake plus hook delivery keeps each merged wake to one short line.

---

## Verification

Adversarial pass on 2026-10-23 against ocs `1a3cee23a` (`gh api` confirms tag `v0.9.4` at that SHA) and the Hermes clone at `f3b5b980`.

- Checked and held: every `ocs:src/hermes.ts` citation (roles, timeouts, 16 KiB cap, file hardening, fingerprint, `written`-before-`send`, RPC-error/`unknown` mapping), `capabilities.ts:10-12`, CHANGELOG 0.7.3 lines 96-105, README 243/255/349, `deliver.ts` (SELF_ENV_KEYS, `deliverNotice`, DM path, Grok-only self-skip), `cli.ts:606-624`, `wake.ts` caps and template, `i18n.ts:375-381`, `roster.ts:91-92, 248-275, 322-330`; Hermes `host_rendezvous.py` roles/version/fingerprint/`publish_record`, `gateway_client.py` ticket path, classic-CLI doc, `prompt_voice.py`, `_handle_busy_submit`, the 4090 `SESSION_OWNER_UNCONFIRMED` path, submit surface reset and rebind, `_rebind_live_transport`, `session_context.py` vars.
- Wrong, fixed: "a busy burst becomes N turns" (sections 2.6, 3.2, 5, 6.3, 7.7). Hermes' `_enqueue_prompt` merges text-only queued prompts into one slot, so it becomes one merged turn.
- Wrong, fixed: the skill paths. `~/.agents/skills` is scanned for updates, not installed to; the install list is `cli.ts:1675-1683`.
- Citations fixed: `web_server.py` 1080-1092 -> 1078-1128; `run.py` 4880-4889 -> 4887 (+5039); `wake.ts` 190 -> 186 and 85-96 -> 85-98; CHANGELOG 85-93 -> 84-92; `fakeHermes` 47 -> 48. The e2e test at `test_dashboard_auth.py:50` is an auth-contract spec, not a dial; reworded.
- Added: the authority's canonical `prompt.submit` replies `started` rather than `streaming`, and the legacy fallback keeps `prompt.*` on the authority (section 6.2, 7.1); canonical `turn_author` (section 2.4). Section 7.1 now says `pre_llm_call` injects the check-in (digest or bodies, unverified), not "the bodies".

