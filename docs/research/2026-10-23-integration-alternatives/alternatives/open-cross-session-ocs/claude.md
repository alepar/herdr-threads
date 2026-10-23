# open-cross-session (ocs) x Claude Code

- Upstream: https://github.com/leeguooooo/open-cross-session
- Clone read: `scratchpad/alts/open-cross-session-ocs`, HEAD `1a3cee23a` (= `v0.9.4`). Read only, nothing executed. Paths below are relative to the clone root.
- Tags: **VERIFIED** = read in source, docs, or a read-only local check; **INFERRED** = my reasoning; **unknown** = not found.
- Local read-only checks were made on this machine with Claude Code 2.1.309 (session registry written by 2.1.308). No config was written.
- Harness reference: `../../primers/claude.md` (cited below as "primer §N").

## 1. Support status

**Native.** ocs has no hooks, no MCP server and no mod. It uses Claude Code's built-in cross-session messaging inbox (primer §3) directly. It writes a JSONL `user` frame into the target session's per-session Unix socket, and Claude Code itself does the rest: it queues the frame, starts a turn if the session is idle, and otherwise delivers it between tool calls. VERIFIED: `src/claude-inject.ts:11-14`, `:380-419`, `:511-578`; `docs/wake-protocol.md:19-26`. Grepping `src/` for `UserPromptSubmit|PostToolUse|SessionStart|Stop` finds only a comment (VERIFIED, overview).

The only other things ocs puts on the Claude side are a skill file and one settings key (§4).

The Claude module is vendored from AgentParty, the same author's project, which is now unmaintained (`src/claude-inject.ts:3-9`, `docs/wake-protocol.md:3`). The README is frank about it: "for a quick claude↔claude direct message, native is smoother — ocs's Claude carrier literally rides on the native inbox socket" (`README.md:369-370`). VERIFIED.

## 2. Injection mechanism

### 2.1 Addressing: how ocs finds the target socket (VERIFIED)

1. **Roster.** ocs lists `~/.claude/sessions/*.json` (hard-coded to `homedir()/.claude/sessions`, overridable only by the test variable `OCS_CLAUDE_NATIVE_SESSIONS_DIR`; `src/claude-inject.ts:55-98`). Each `<pid>.json` must be a regular file, not a symlink, at most 64 KiB and owned by the current uid. The file-name pid must equal the pid inside the file, and the pid must be alive (`:115-194`). The fields it reads are `pid`, `sessionId`, `name`, `cwd`, `status`, `statusUpdatedAt`, `kind`, `messagingSocketPath` and `procStart`.
2. **Target selection.** A mention matches by exact Claude session `name` first, then by a unique workspace alias (`src/wake.ts:302-340`). Several live sessions with the same name are all woken, on the stated assumption that a duplicate name means the same person running several sessions. Self-exclusion works by both the sender's pid and the sender's name (`:326-333`).
3. **Pinned resolution.** `resolveSessionSocketByPid(pid, expectSessionId)` re-reads `<pid>.json` and refuses if `sessionId` differs. This guards against pid reuse and against `/clear` swapping the session (`src/claude-inject.ts:241-260`).
4. **Socket checks.** The target socket must pass `lstat` checks: it is a socket, not a symlink, and owned by the uid (`:444-463`). ocs then probes it with a zero-byte `connect` and immediately destroys the connection (`:326-348`).
5. **Auth.** ocs reads the optional peer token from `~/.claude/sessions/<pid>.<sha256(socketPath)>.key` (`{"peerToken":"<32hex>"}`). The token is required on Windows and optional on macOS and Linux (`:278-319`, `:549-553`).

### 2.2 Self-identity (VERIFIED)

`selfPidFromEnv` takes the pid from the `CLAUDE_CODE_MESSAGING_SOCKET` file name and accepts it only if `sessions/<pid>.json` matches both `CLAUDE_CODE_SESSION_ID` and the socket path. Otherwise ocs walks up the parent-process chain, up to 10 hops, through `/proc/<pid>/stat` or `ps` (`src/wake.ts:234-293`, `docs/wake-protocol.md:184-188`). With this identity, the woken model can run `ocs send/dm` without `--as`.

### 2.3 Wire payload (VERIFIED, `src/claude-inject.ts:361-419`)

The payload is one or two JSONL lines per connection:

```json
{"type":"auth","token":"<32hex>"}                       // only if a key file was readable
{"type":"user","msgV":1,"msg_id":"<uuid>","from":"uds:<receipt sock>","priority":"next",
 "message":{"role":"user","content":"<cross-session-message ...>\n<note>\n</cross-session-message>"}}
```

- The wrapper is `<cross-session-message from="uds:<sock>" from-name="<sender>" from-mode="prompting">`. Its attribute order is fixed (`from`, `from-session`, `hop-chain`, `from-name`, `from-mode`) because "the receiver re-serializes and requires byte equality" (`:361-378`, the comment at `:364`). ocs rejects a `from-name` that contains `"`, `<`, `>`, CR or LF (`:516-518`).
- `priority` defaults to `"next"`. The type also allows `"now"` and `"later"`, but no caller passes anything other than the default (grep of `priority` in `src/`, VERIFIED). What `now` and `later` do in Claude Code is unknown.
- Each line may be at most 1 MiB (`CLAUDE_INBOX_MAX_LINE_BYTES`, `:57-58`, `:561-565`). The note itself is capped far below that (§2.4).
- The write is one-shot: connect, then `end(payload)`, then `destroy()`, with a 2 s timeout (`:589-612`). The destroy-on-success exists because a receiver that never closes its end used to keep `ocs send` from exiting (`:580-588`).
- The schema is taken from Claude Code internals (minified names such as `pSd`, `NMr`, `USd` are cited in the comments) and was "measured" on 2.1.236 and 2.1.285. Claude's public docs do not document it (primer §3: "The message line's JSON schema is **not documented**"). VERIFIED: `src/claude-inject.ts:42-43`, `src/claude-receipt.ts:3`.

### 2.4 The note inside the wrapper (VERIFIED, `src/wake.ts:39-172`, `docs/wake-protocol.md:28-75`)

```
[ocs wake] <sender> mentioned you in #<channel> (seq <N>[, reply to seq <M>])

<body>

Reply when asked, or with a needed decision/result. No unsolicited acknowledgements.
Reply: ocs dm <sender> "<your reply>"          (Claude DM; else: ocs send <ch> "<your reply>" --reply-to <N>)
Thread: ocs read <channel>
```

- **Bounds.** A body of up to 4096 bytes is inlined verbatim. A longer body shows its first 512 bytes, cut on a UTF-8 character boundary (`truncateUtf8`, `:77-83`), followed by `… (<total> bytes total; full text: ocs read <ch>)`. The skeleton budget is 1024 bytes, so a whole note is at most 5120 bytes. If the skeleton runs over budget, ocs drops the `ago` field, then the sender; the `Reply:` and `Thread:` lines are never dropped (`:131-172`).
- **Neutralization.** Inside the body, `<cross-session-message` and `</cross-session-message` (any case) become `‹...`. Lines starting with `Reply:`, `Thread:`, `[ocs wake]`, `[Cross-session idle notice]` or `[ocs delivery notice]` (or their Chinese forms) get a `> ` prefix (`:85-98`). The size thresholds are applied to the neutralized text.
- **Anti-chatter.** The fixed line "Reply when asked ... No unsolicited acknowledgements." goes into every note (`src/i18n.ts:512`). It was added after agents' mutual acknowledgements produced a self-sustaining loop (issue #23 title: "agent 互相确认会触发 durable execution 自激循环并持续刷屏", gh api, VERIFIED).

### 2.5 When it fires (VERIFIED)

ocs only pushes, and only at send time. The sender runs `ocs send`/`ocs dm`, the CLI appends to the log, then it writes the frame (`DESIGN.md:15`, `README.md:249`). Nothing fires at turn start or after a tool call, and ocs does not deliver anything in-turn on its own. Mid-turn arrival is Claude Code's native behaviour: "The receiving Claude reads the message between tool calls during an active turn ... When the receiving session is idle, Claude Code starts a new turn" (primer §3).

ocs does no busy-gating or coalescing for Claude. Every message is its own frame. This contrasts with ocs's Codex path, which steers into the running turn or merges a backlog into one wake (`src/deliver.ts:148-187`, compared with the Claude branch at `:450-479`).

### 2.6 Dedupe, ack and receipts (VERIFIED)

- **No resend.** ocs never replays a send. `unknown` outcomes are not retried, and exit codes 2 and 3 tell the sender "stored, do not resend" (`README.md:249`, `:271-281`; `DESIGN.md:34`).
- **No dedupe of its own.** Claude Code itself "drops identical repeats arriving within a short window, and queues at most 50 accepted messages" (primer §3). ocs notes differ by seq, so the native dedupe would only catch a literal duplicate (INFERRED).
- **No read ACK.** The wake does not advance the receiver's read cursor; only `ocs read` does (`src/inbox.ts`, overview). "Any bookkeeping still counts only the peer's reply" (`docs/wake-protocol.md:236-238`).
- **Delivery receipts** use the undocumented `peer_message_status` protocol (`src/claude-receipt.ts`, `src/wake-helper.ts`, `src/wake-receipt.ts`, `docs/wake-protocol.md:204-302`):
  - The sender binds a receipt socket `<dir of target socket>/<16hex>.sock` with mode 0600, using umask `0o177` during bind (`src/claude-receipt.ts:110-135`, `:213-241`). The receiver accepts a reply path only if it matches `/^\/\S*\.sock$/` and sits in the receiver's own socket directory. It also sends receipts only to the process that wrote the frame, checked by peer credentials (`docs/wake-protocol.md:242-248`).
  - Because of that process model, the frame is written by a detached helper, `ocs _claude-wake <job>`. The helper listens, writes the frame with `from` and `msg_id`, and then:
    - if no receipt arrives within **400 ms**, it records `accepted` (with `accept` policy the receiver sends nothing);
    - on `held`, it waits up to 5 min + 60 s for a terminal receipt;
    - it maps `delivered`, `expired`, `expired`+`status_detail:"refused"` → `refused`, `dropped` and `denied`;
    - while waiting on `held` it re-checks the target every 2 s and gives up as `unknown` after 3 consecutive misses (`src/wake-helper.ts:162-188`);
    - if the receipt listener cannot be opened, it falls back to a frame without `from` and reports `plain` (`:115-123`).

    VERIFIED: `src/wake-helper.ts:63-160`, `src/claude-receipt.ts:67-91`.
  - The CLI waits at most 1.5 s for the helper's first line. If none arrives, it reports `unknown` and exits 3, and never writes the frame itself. Only when the helper never started (no helper command, receipts off, spawn failure) does the CLI write a receipt-less frame in-process (`src/wake-receipt.ts:25-30`, `:257-335`).
  - Receipts are stored as sidecar lines `{"v":1,"type":"receipt","seq",...}` in the channel log (`docs/wake-protocol.md:304-314`).
  - A held message that is not delivered produces one `[ocs delivery notice]` into the **sender's** session. That notice is injected without `from`, so it generates no receipt and cannot loop (`src/wake-helper.ts:190-215`, `src/deliver.ts:328-356`).
  - Receipts are switched off by `OCS_NO_RECEIPTS=1`, and are not available on Windows (`src/wake-receipt.ts:23-24`, `:199-206`).

## 3. Wake mechanism

- **Mechanism.** The wake and the injection are the same frame. With `crossSessionInbound: "accept"`, Claude Code enqueues the frame and "idle 会话被 onEnqueue 唤起开新 turn", meaning `onEnqueue` starts a new turn on an idle session (`src/claude-inject.ts:31-40`). There is no keystroke, no `$.prompt.submit` and no hook. VERIFIED (ocs claim, consistent with primer §3).
- **Idle/busy detection.** None for the wake itself. ocs leaves it to Claude's queue. ocs does read `status` (`busy`/`idle`) and `statusUpdatedAt` from `<pid>.json` for `notify-when-idle`: a detached `ocs _idle-watch` polls every 2 s, needs 3 misses before it decides the target is gone, and expires after 6 h (`src/idle.ts:1-60`, `docs/wake-protocol.md:132-170`). A read-only check of this session's registry file shows these fields exist on 2.1.308, along with `peerProtocol: 1`, `peerFeatures: ["notify_idle", ...]`, `kind`, `name` and `version` (VERIFIED, local).
- **Draft safety.** Nothing is typed into the composer, so a human's draft cannot be clobbered by keystrokes. Whether Claude Code starts a native-inbox turn while the composer holds an unsent draft, and what happens to that draft, is unknown: neither the docs nor ocs say. The cmux carrier is not a fallback for this native path. It is a separate target kind (`surface:<n>`) for Claude or Codex TUIs running in cmux, and a fallback only for Codex Desktop (`README.md:241`, `:244`; `src/deliver.ts:524-535`). It checks for `to interrupt` on the last 8 screen lines and then types the note + Enter, with no draft check (`src/roster.ts:248-276`, VERIFIED; that a typed note would merge with a draft is INFERRED).
- **Latency.**
  - From the sender's side: helper spawn, connect and write, then a 400 ms receipt window; the CLI returns within 1.5 s. `held` was measured at 30-50 ms after the write (`src/claude-receipt.ts:8`, `:32-33`).
  - Time from frame to turn start: unknown, and not measured by ocs.
  - A session busy in a long tool call reads the frame at the next tool boundary (native, primer §3).
- **Failure modes:**
  1. **Inbound gate.** With `hold`, the message waits for a human to approve it in a dialog and is dropped after 5 minutes. With `refuse` it is discarded. ocs surfaces both through receipts and exit code 2 (VERIFIED, `README.md:240`, `:251-281`).
  2. **The default may be stated wrongly.** ocs repeatedly states that `hold` is the default (`src/claude-inject.ts:35`, `README.md:240`). Claude's docs say that without a setting a prompting receiver *accepts* unless the sender is in bypass mode (primer §3). ocs may describe an older default, or a case where its sender counts as unverified. unknown.
  3. **No live socket.** The cases are a dead pid, a `sessionId` mismatch, a failed probe, a socket not owned by the user (`InjectFailureReason`, `src/claude-inject.ts:421-428`), and `--bare` sessions, which do not bind the inbox (primer §3). In each case the message stays in the log and nothing wakes the session later: "Persistence has no auto-nudge" (`README.md:349-351`).
  4. **Schema drift.** The frame and receipt formats are private. ocs says "可能随版本收紧", i.e. they may tighten in later versions (`src/claude-inject.ts:42-43`). A changed schema would most likely fail silently as `accepted`, since a missing receipt reads as success (INFERRED).
  5. **Helper killed.** A helper killed mid-hold leaves a socket in Claude's directory. The next wake removes it through `sweepWakeJobs`, which deletes only paths recorded in a job file (`src/wake-receipt.ts:150-186`).

## 4. Setup and installation footprint on the harness (VERIFIED)

| Write | When | Content |
|---|---|---|
| `~/.claude/settings.json` key `crossSessionInbound: "accept"` | only `ocs doctor --fix` | Read, change one key, write the whole file. If the file exists, a backup `settings.json.ocs-backup-<ts>` is taken first; if it does not, ocs creates it with just this key. ocs refuses to touch a file it cannot parse (`src/claude-settings.ts:15-19`, `:36-77`, `src/cli.ts:1320-1343`). |
| `~/.claude/skills/ocs/SKILL.md` (12,748 bytes) | the installer, `ocs skill install`, `doctor --fix`, `ocs upgrade` | Atomic write through temp file + rename; also written to `~/.codex/skills/...` and the Grok and Pi equivalents (`src/cli.ts:1675-1731`, `:1345-1360`, `:1310`). `install.sh` first tries the third-party `npx skills@1.5.23 add ... --agent claude-code --global`, then runs `ocs skill install` anyway (`install.sh:69-86`); where that CLI writes is unknown. |
| A temporary `<16hex>.sock` in Claude's socket dir | per wake, during receipts | Mode 0600, removed on exit or by the sweeper (`src/claude-receipt.ts:22-25`). |

ocs writes no hooks, no MCP server registration, no plugin and no `CLAUDE_CODE_PLUGIN_DIRS` entry. Its own state lives in `~/.ocs`.

**Every path is hard-coded to `~/.claude`.** `CLAUDE_CONFIG_DIR` appears nowhere in `src/`, `docs/`, `README.md` or `skills/` (grep, VERIFIED). A read-only check on this machine shows what that means:

- This session runs with `CLAUDE_CONFIG_DIR=~/.aisw/profiles/claude/claude-2`. Its registry file is `$CLAUDE_CONFIG_DIR/sessions/2425194.json`, and `~/.claude/sessions` does not list it (VERIFIED, local).
- Its socket is `/run/user/1001/cc-socks/2425194.sock`, not `/tmp/cc-socks/` (VERIFIED, local). This part is harmless, because ocs takes the socket path from the registry file.
- So ocs cannot see, address or identify any session that runs under a non-default config dir.
- `doctor --fix` would edit a `settings.json` that such a session never reads (INFERRED), and the skill would land in the wrong skills directory (INFERRED).

## 5. Token cost

- **Per message:** the body plus the note skeleton plus the wrapper, plus whatever framing Claude Code adds natively (unknown).
  - A typical DM skeleton comes to about 250-350 bytes: header about 60, anti-ack line 84, `Reply:` about 40-80, `Thread:` about 40-70.
  - The wrapper with a receipt socket adds about 110 bytes, of which `from="uds:/run/user/1001/cc-socks/<16hex>.sock"` alone is about 50.
  - That is roughly 90-120 tokens of overhead per message (INFERRED from the strings in `src/i18n.ts:315-327`, `:512` and `src/claude-inject.ts:366-378`). The skeleton is capped at 1024 bytes and the whole note at 5120.
  - Each message that arrives while the target is busy costs that again, because nothing is coalesced.
- **Hidden second read:** the wake does not advance the read cursor, and the note's `Thread:` line invites `ocs read <channel>`. A diligent model is likely to re-read a body it has already seen in order to mark it read (INFERRED).
- **Per quiet turn:** zero, because no hooks run.
- **Per session:** only the skill's frontmatter description (about 200 chars, `skills/ocs/SKILL.md:1-4`) is listed. The 12.7 KB body loads when the skill is invoked (Claude skill loading behaviour, INFERRED from the primer and harness conventions).

## 6. Limitations and bugs visible in the source

1. **`CLAUDE_CONFIG_DIR` is ignored** (§4). Every session in a non-default config dir is invisible to ocs, so self-identity fails and the session cannot be woken. VERIFIED by grep and by the local registry location.
2. **`from-mode="prompting"` is always sent.** `fromMode` defaults to `"prompting"` and no caller overrides it (`src/claude-inject.ts:522`; callers at `src/wake.ts:368-375`, `src/wake-helper.ts:93-102`). It is sent even when the sending Claude session runs in bypass mode. If Claude's inbound policy ("a prompting receiver accepts unless the sender is bypass") trusts this attribute, ocs misstates the sender's mode and sidesteps that hold. Whether Claude derives the mode from evidence or trusts the attribute is unknown. VERIFIED code; impact INFERRED.
3. **The `from` reply address is ephemeral.** The wrapper's `from` is the helper's temporary receipt socket, not the sender's session socket. Claude presents `from` as the reply address (primer §3: "plain text with the sender's name and a reply address"). A model that answers with native `SendMessage` to that address would reach a socket that has already been deleted. ocs relies on the `Reply:` command line instead. VERIFIED code (`src/wake-helper.ts:127`); impact INFERRED.
4. **No busy coalescing for Claude.** N messages to a busy Claude produce N frames and N native deliveries, and the native queue holds at most 50 (primer §3). ocs built coalescing for Codex (issue #41) but not for Claude. VERIFIED.
5. **`accepted` is not proof of delivery.** It means only that no receipt arrived within 400 ms. A receiver that no longer sends receipts, or a slow event loop, also reads as `accepted`. ocs documents this honestly (`docs/wake-protocol.md:236-238`). VERIFIED.
6. **Self-notices may be held, and nobody learns of it.** The `[ocs delivery notice]` into the sender's own session carries no `from`, so it gets no receipt and is subject to the sender's own inbound gate. The helper is `setsid`-detached, and once the CLI exits it is no longer a descendant of the Claude process. Claude's "own-child" exemption (primer §3) therefore probably does not apply (INFERRED).
7. **Offline messages wait for the user.** A message to an offline session sits in the log until the user or the model runs `ocs inbox`. Nothing notices a session coming back (`README.md:349-351`). VERIFIED.
8. **No priority model.** Every mention or DM wakes the target unless `--no-wake` is passed. Every message enters the context at `priority:"next"`. VERIFIED.
9. **Private protocol, no capability gate.** ocs never reads the registry's `peerProtocol`/`peerFeatures` before writing a frame (grep shows no use; VERIFIED). It probes only that the socket connects.
10. **cmux carrier.** It has no draft guard, and `to interrupt` as the busy signal is a fragile screen-scrape (`src/roster.ts:263`). VERIFIED.

## 7. Clever ideas worth stealing

1. **Claude's own registry as an idle oracle.**
   - `<config>/sessions/<pid>.json` carries `status` (busy/idle), `statusUpdatedAt`, `sessionId`, `messagingSocketPath`, `name` and `peerFeatures`.
   - Claude Code itself writes it, so reading it costs one file read with no spawn. That is cheaper than `claude agents --json` (primer §6, Q5) and more reliable than screen-scraping.
   - herdr-threads' no-mod fallback, the Herdr native typing wake, could gate on it instead of on detection text. It needs `CLAUDE_CONFIG_DIR`-aware lookup, which ocs lacks, and pid + `sessionId` pinning, which ocs does well.
2. **The native inbox as a delivery carrier.**
   - A frame with `priority:"next"` gives an idle-to-new-turn wake and a busy-to-next-tool-boundary delivery from one write, and the model sees a native `cross-session-message` provenance wrapper.
   - For herdr-threads, the mod's spawned `watch` is a child of the Claude process. It could post to `CLAUDE_CODE_MESSAGING_SOCKET` and plausibly qualify for the own-child exemption (primer §3), which sidesteps `crossSessionInbound`.
   - Cost: the frame schema is private, and native framing replaces our R17 header. This needs a capture first (primer Q1); ocs's `buildInjectFrames` and `wrapCrossSessionMessage` are a ready reference for that capture.
3. **`peer_message_status` receipts.** `msg_id` / `orig_msg_id` correlation plus a same-directory reply socket gives an honest "held / refused / dropped / expired" signal from Claude itself. herdr-threads' mod path has no equivalent "the harness refused this" signal for a peer frame, because it does not use one.
4. **Stored and woken are reported separately.** The output says "stored #ch seq n" first, then a wake outcome of accepted, failed or unknown, with distinct exit codes (0/2/3) and the rule that `unknown` is never replayed. This is a clean contract for an agent-facing CLI.
5. **Body neutralization.** Rewriting wrapper-tag openers to `‹` and prefixing protocol-looking lines with `> ` stops a body from forging a header or a reply command. herdr-threads' header-line framing (R17) would benefit from the same rule, so that a body line cannot pass for a `[herdr-threads] ...` header.
6. **An anti-acknowledgement line**, born of a real ack-storm incident (issue #23). herdr-threads keeps this in the skill rather than per message, per rule (a), and should keep it that way. The incident still confirms the need.
7. **A hard skeleton budget with a degradation ladder.** The 1024-byte skeleton drops optional fields first and never drops the actionable lines. It is a reasonable pattern for any bounded wake prompt.

## Verification

Adversarial pass against the clone at `1a3cee23a` (tag `v0.9.4` confirmed), `gh api` for issues #23 and #41, the primer §3, and read-only local checks (`claude --version` 2.1.309; `$CLAUDE_CONFIG_DIR/sessions/2425194.json` keys and `peerFeatures`; `~/.claude/sessions` lists only another session).

Checked and held: every cited line range in `src/claude-inject.ts`, `src/wake.ts`, `src/wake-helper.ts`, `src/claude-receipt.ts`, `src/deliver.ts`, `src/roster.ts`, `src/i18n.ts`, `src/idle.ts` constants, `src/claude-settings.ts`, `src/cli.ts` doctor path, `docs/wake-protocol.md` (§1, §2, §4, §6), `README.md:240-281`, `:349-351`, `:369-370`, `DESIGN.md:15`, `:34`; the greps (no hook events, no `CLAUDE_CONFIG_DIR`, no `peerProtocol`/`peerFeatures`, no non-default `priority`); SKILL.md size 12,748 bytes; issue #23 title.

Changed:
- Wrong citations fixed: receipts-off/Windows gate is `src/wake-receipt.ts:23-24`, `:199-206` (was `:60-67`); CLI wait logic is `:257-335` (was `:118-197`); line limit `:57-58`; skill install `src/cli.ts:1675-1731`.
- cmux corrected: it is a separate `surface:<n>` target and a Codex Desktop fallback, not a fallback for the Claude native path.
- `doctor --fix`: backup only when `settings.json` exists, otherwise it creates the file.
- Added: helper's `plain` fallback and target-gone exit during `held`; the CLI's in-process receipt-less write when the helper never starts; the installer's third-party `npx skills` step.
