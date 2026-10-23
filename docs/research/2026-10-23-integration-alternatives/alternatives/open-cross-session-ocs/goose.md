# open-cross-session (ocs) x Goose (Block)

- Upstream: https://github.com/leeguooooo/open-cross-session, clone `scratchpad/alts/open-cross-session-ocs`,
  HEAD `1a3cee2` (= `v0.9.4`, 2026-10-22). Read only, never executed. Paths are relative to the clone root.
- Goose trees (from the primer `primers/goose.md`): `goose@1.36` (tag v1.36.0, the installed binary) and
  `goose@main` (1.56.0), under `scratchpad/goose-src/{goose-1.36,goose}`.
- VERIFIED = read in source or docs at the cited place. INFERRED = my reading. "unknown" = not determined.

## Support status

**Not supported.** VERIFIED:

- `grep -rni goose` over the whole clone (source, docs, tests, skill) returns 0 hits.
- The DM target resolver knows exactly these namespaces: Grok, `agy`, `surface:N` (cmux), Codex thread
  ids / `codex-<8hex>`, Pi, Hermes, else a Claude session name (`src/roster.ts:668-720`,
  `DmTargetKind = "claude" | "codex-task" | "pi" | "hermes" | "grok" | "agy" | "cmux"`, `src/roster.ts:454`).
- The skill's own description names "Claude Code, Codex, Pi and Hermes" (`src/cli.ts:1494`).
- Self-identity (`resolveSelfName`, `src/roster.ts:100-117`) checks `$OCS_NAME`, then `GROK_SESSION_ID`,
  `OCS_PI_SESSION_ID`, a Claude session (`CLAUDE_CODE_SESSION_ID` plus the messaging socket env, else a walk
  up the process ancestry to a live `~/.claude/sessions/<pid>.json`; `src/wake.ts:243-293`), `CODEX_THREAD_ID`,
  then Hermes. Nothing Goose-specific; Goose's `AGENT_SESSION_ID` is not read anywhere (grep: 0 hits).
- ocs has no terminal-multiplexer carrier other than cmux: no tmux, Herdr, zellij or WezTerm carrier
  (grep over `src/` for `tmux|herdr|zellij|wezterm|send-keys`: only a comment in `src/codex-queue.ts:8` about a
  manual test). Herdr appears only in README as "Remote agent/runtime control remains Herdr's job, not OCS's"
  with `herdr agent prompt` examples (`README.md:443-445`).

So a Goose session can participate in ocs only through two generic paths, analysed below:

1. **Generic CLI** (any harness with a shell tool): the Goose model runs `ocs send/dm/read/inbox --as <name>`.
   Pull only.
2. **Generic cmux surface wake** (`ocs dm surface:N`): if Goose happens to run inside a cmux terminal surface,
   ocs types the wake note plus Enter into it. Not applicable to a Goose in a Herdr pane.

## Injection mechanism

### Path 1: generic CLI (pull through Goose's shell tool)

- Mechanism: the Goose model itself runs `ocs read <ch> --as <name>` or `ocs inbox --as <name>` through its
  developer shell tool. Output enters context as a tool result. VERIFIED for the commands
  (`README.md:300,302`, skill command list `src/cli.ts:1531,1535`); INFERRED for the Goose side (any shell-capable agent can).
- Identity: without a recognised harness env var, `senderName()` resolves `--as` > `$OCS_NAME` > host identity,
  and otherwise fails before storing anything (`src/cli.ts:207-214`; message `failNoSelfName`,
  `src/i18n.ts:561-562`: "cannot infer sender identity; no message was stored or sent ... append --as human").
  A Goose agent must therefore pass `--as <name>` on every call or be launched with `OCS_NAME` in its env
  (INFERRED: Goose's shell tool passes the inherited env through; primer section 11). Nothing binds that name
  to the Goose session: any process can claim it (INFERRED from the resolution order).
- When it fires: only when the model decides to call it. There is no turn-start, post-tool or turn-end
  delivery for Goose: ocs writes no hook entries into any harness config (grep over `src/` for
  `hooks|UserPromptSubmit|PostToolUse`: 0 hits; `src/claude-inject.ts:264` only mentions SessionStart in a
  comment). Its one event-driven integration is the Pi-only extension (`pi.on("session_start", ...)`,
  `src/pi-extension.ts:209-219`), irrelevant to Goose.
- Payload/framing: `ocs read` prints new messages since the consumer cursor and advances it; own messages fold
  to one line (`README.md:302`). The per-consumer cursor (`cursors/<channel>.<consumer>.json`, overview Storage)
  is the only dedupe. `--peek` does not advance it. No ACK: "accepted is not a read receipt"; bookkeeping
  "counts only the peer's reply" (overview, `docs/wake-protocol.md:236-238`).
- Discovery of the skill: `ocs skill install` writes `~/.claude/skills/ocs/SKILL.md`, `~/.codex/skills/...`,
  `~/.grok/skills/...` and a Pi skill (`src/cli.ts:1675-1683,1700-1731`). Goose reads skills from
  `~/.claude/skills` among its global roots, in both trees (VERIFIED: `goose@1.36
  crates/goose/src/skills/mod.rs:290-308`, `goose@main crates/goose/src/skills/mod.rs:362-372`). So a Goose
  user who installed ocs for Claude also gets the `ocs` skill listed in Goose, **by accident**; its text tells
  the agent identity "is auto-detected inside Claude, Codex, and Pi sessions; `--as <name>` overrides"
  (`src/cli.ts:1545`), which is wrong for Goose and leads to a first failed call (INFERRED).

### Path 2: generic cmux surface (push, keystrokes)

- Code path: `ocs dm surface:N "<text>"` -> `resolveDmTarget` returns `kind: "cmux"` (`src/roster.ts:689-691`)
  -> `deliver.ts:524-538` -> `wakeCmuxSurface(ref, wakeNote(...))` (`src/roster.ts:253-275`):

  ```ts
  const screen = spawnSync("cmux", ["read-screen", "--surface", ref, "--lines", "8"], ...);
  if (screen.stdout.includes("to interrupt")) return { ok: false, reason: "busy" };
  const send = spawnSync("cmux", ["send", "--surface", ref, note], ...);
  const enter = spawnSync("cmux", ["send-key", "--surface", ref, "enter"], ...);
  ```
- Payload: the plain wake note, no `<cross-session-message>` wrapper for cmux (code comment: Codex queue / Pi /
  cmux have no wrapper, `src/wake.ts:90`). Shape (`src/wake.ts:117-172`, header `src/i18n.ts:315-325`, policy line
  `src/i18n.ts:512`):

  ```
  [ocs wake] <sender> mentioned you in #<channel> (seq N[, reply to seq M])

  <body, verbatim <= 4096 B, else first 512 B + "… (T bytes total; full text: ocs read <ch> --as <rcv>)">

  Reply when asked, or with a needed decision/result. No unsolicited acknowledgements.
  Reply: ocs send <channel> "<your reply>" --as <receiver> --reply-to N
  Thread: ocs read <channel> --as <receiver>
  ```
  For a cmux receiver the `--as surface-N` form is used because the surface "cannot verify its own identity"
  (`src/deliver.ts:525`, `docs/wake-protocol.md:65-66`).
- Size bound: whole note <= 5120 B (4096 body + 1024 skeleton), UTF-8-safe truncation (`src/wake.ts:40-46,77-84`;
  the 4096 B test is on the neutralised body, `src/wake.ts:146-149`).
- Neutralisation: `<cross-session-message` is defanged and body lines starting with `Reply:`/`Thread:`/
  `[ocs wake]` (and idle/delivery notice headers) are prefixed `> ` (`src/wake.ts:85-98`). VERIFIED.
- Dedupe/ack: none on this path; the message is already in the channel log, the wake is fire-once, and a busy
  surface is simply skipped (`src/i18n.ts:534-535`: "mid-turn; not interrupting. The message is in the
  channel — it will be read on the next turn, or retry later"). Nothing retries later (INFERRED: no watcher for
  cmux, only the Claude `notify-when-idle` watcher exists, overview).
- How it lands in Goose (INFERRED, from the primer section 9): the text becomes the rustyline buffer of the
  `> ` prompt and Enter submits it as a **user message**. The newlines inside the note: `cmux send`'s handling
  of `\n` is unknown. If it sends raw LF, Goose treats Ctrl+J as "insert newline" (`goose@1.36
  crates/goose-cli/src/session/input.rs:85,138-141`) and the note arrives as one multi-line message; if cmux
  maps `\n` to Enter, each line would be submitted as a separate turn. Unknown which.

## Wake mechanism

- Path 1 (CLI): **no wake.** An idle Goose at `> ` never learns of a message; it is read when a human or the
  model next runs `ocs read`/`inbox`. ocs states "Persistence has no auto-nudge" (`README.md:349-351`).
- Path 2 (cmux): the same act as injection (keystrokes + Enter into an idle composer).
  - Idle/busy detection: the substring `to interrupt` anywhere in the last 8 screen lines means busy. This
    string was written for Claude/Codex TUIs, but it **happens to match Goose's spinner**: Goose's thinking
    indicator renders `"(Ctrl+C to interrupt)"` (`goose@1.36 crates/goose-cli/src/session/output.rs:124-139`,
    `goose@main output.rs:170`). VERIFIED both strings; the match is coincidental (INFERRED).
  - It is not a reliable busy signal for Goose: the spinner is stopped (`spinner.stop("")`, `output.rs:143-146`)
    when an agent message is rendered in interactive mode (`goose@1.36 crates/goose-cli/src/session/mod.rs:1211`),
    on an inline notice (`output.rs:255`), before an elicitation form (`mod.rs:1176`) and before a tool
    approval select (`prompt_tool_confirmation`, `mod.rs:1679-1680`); it is re-shown only on a
    `ThinkingMessage` notification (`output.rs:251`) or a new `process_agent_response` call.
    So during streamed assistant text, a running tool, or an approval/elicitation select, the last 8 lines
    usually lack `to interrupt` and ocs classifies a **busy Goose as idle** (INFERRED from the show/hide
    sites). Also, after a turn the old spinner line may still be on screen; whether `stop("")` clears the line
    is cliclack behaviour, unknown.
  - Consequences when misclassified (INFERRED, primer section 9): Goose does not read the tty during a turn, so
    the bytes sit in the tty buffer and become input for the next prompt; in cooked mode the injected Enter
    (CR) may be converted to LF by ICRNL, which Goose binds to "newline", leaving the note as an unsubmitted
    draft rather than a turn (unknown, depends on terminal mode at that moment). During a `GOOSE_MODE=approve`
    tool-approval select or an MCP elicitation form, the typed note plus Enter **answers that prompt**.
  - Draft safety: none. ocs never inspects the composer; a human's half-typed line at `> ` is merged with the
    note and submitted (overview; `src/roster.ts:253-275` reads the screen only for `to interrupt`).
  - Latency: synchronous inside `ocs dm`, three `spawnSync` calls with 5 s timeouts each (`src/roster.ts:256-268`);
    sub-second when cmux responds (INFERRED).
  - Failure modes: cmux absent -> `unavailable`; screen read/send/key failure -> `failed`, exit 2 "stored but
    wake failed" (overview exit codes); busy -> `failed` too (`src/deliver.ts:531-533`). If Goose has exited,
    the note is typed into whatever now runs in the surface, typically the shell (ocs guards against this only
    for auto-fallback Codex surfaces by requiring a live `codex` process, `src/roster.ts:180-224`; an explicit
    `surface:N` target has no process check). VERIFIED for the guard's scope; INFERRED for the shell hazard.
- Herdr panes: none of this applies. A Goose in a Herdr pane is not a cmux surface, and ocs has no Herdr
  carrier. An operator could hand-roll the equivalent with `herdr agent prompt` (README:445 points there), but
  ocs does not.

## Setup/installation footprint on the harness

- Goose config: **nothing.** ocs writes no Goose config, plugin, hook, MCP extension, or `GOOSE_MOIM_*`
  variable (grep: 0 hits).
- Side effects reaching Goose anyway: `ocs skill install` / `ocs doctor --fix` write
  `~/.claude/skills/ocs/SKILL.md` (`src/cli.ts:1678`), which Goose loads as a global skill (see above).
  ocs state lives in `~/.ocs` (`OCS_HOME`).

## Token cost per message and per quiet turn

- Quiet turn: **0 bytes** on both paths. Nothing is injected per turn (no hooks, no MOIM file).
- Static: the accidentally discovered `ocs` skill (Goose lists name + description, about 230 B for the
  description at `src/cli.ts:1494`; the body, about 10-13 KB — `skills/ocs/SKILL.md` is 12,748 B, the embedded
  `SKILL_MD` spans `src/cli.ts:1492-1665` — loads only when the model opens the skill). INFERRED from Goose's
  usual skill listing; the exact listing format was not traced.
- Per message, path 2: header (~60-100 B) + body (<= 4096 B, or 512 B preview) + fixed tail (~250 B:
  reply-policy line, `Reply:` and `Thread:` commands with `--as`). Total <= 5120 B. Plus one full model turn
  started by the Enter, as a user message.
- Per message, path 1: one shell tool call plus `ocs read` output, plus the model's tokens deciding to call it;
  every `--as` mistake costs an extra failed call (INFERRED).

## Limitations and bugs you can see

1. No Goose support of any kind; identity must be supplied by hand (`--as`/`OCS_NAME`) and is unverifiable.
2. No wake for a Goose outside cmux, which includes every Herdr pane.
3. cmux busy check is a substring heuristic that only coincidentally covers Goose and misses the long stretches
   (streaming, tool runs, approvals, elicitation) where Goose's spinner is hidden; misfires can answer an
   approval prompt or leave a stuck draft.
4. No composer/draft check: a human's partial input is concatenated with the wake note and submitted.
5. No retry for a skipped busy surface: the "it will be read on the next turn" message is only true if the model
   happens to run `ocs read` (Goose has no turn hook to surface it).
6. Explicit `surface:N` targets have no live-process check; a closed Goose leaves the note typed into a shell.
7. The skill's identity text is wrong for Goose, and the skill reaches Goose only through Goose's
   `~/.claude/skills` compatibility root.
8. Wake note carries the full body as an unattributed **user** message (no wrapper on cmux), so the model's
   only provenance signal is the `[ocs wake]` header text.

## Clever ideas worth stealing

- **Fixed-size, self-contained wake note with copy-paste reply/read commands** (`src/wake.ts:117-172`): bounded
  (5120 B), UTF-8-safe truncation with an explicit "full text: <command>" continuation, and a degradation
  ladder that drops `ago` then `sender` but never the commands. For a harness like Goose with no hooks
  (primer 3.4), a body-carrying wake is the only one-turn delivery option; herdr-threads' content-free wake
  (R18) would cost Goose an extra `inbox` round trip. Worth weighing per adapter (it conflicts with rule (c) of
  our token rules, so it would need a deliberate exception).
- **Body neutralisation** of protocol lines and wrapper tags (`src/wake.ts:85-98`): cheap, prevents a peer body
  forging `Reply:` lines or a fake header. Applies to our `[herdr-threads]` header line too.
- **Anti-chatter line** "No unsolicited acknowledgements" in every wake (`src/i18n.ts:512`): a one-line nudge
  against ACK ping-pong; we put this in the skill instead, which is cheaper.
- **Liveness guard before typing** for auto-chosen surfaces (`src/roster.ts:180-224`: require a live `codex`
  process in the surface, never trust the title alone). A Goose Native wake in herdr-threads should likewise
  check that the pane's foreground process is `goose` before typing.
- Negative lesson for a herdr-threads Goose adapter: the `(Ctrl+C to interrupt)` hint is present only while
  Goose's spinner runs, so screen-scraping it is not a busy signal. Use hook edges (`UserPromptSubmit` ->
  busy, `Stop` -> idle; primer section 10) and an empty-`> `-row composer reading for draft safety, never
  Ctrl+C to clear (it deletes the draft and arms exit; primer section 9).

## Verification

Adversarial pass, 2026-10-23, against the clone at `1a3cee2` and the Goose trees `goose-1.36` (v1.36.0) and
`goose` (main, 1.56.0).

- Checked and held: 0 `goose` / `AGENT_SESSION_ID` / `moim` hits; no tmux/Herdr/zellij/WezTerm carrier
  (only the `src/codex-queue.ts:8` comment); `DmTargetKind` (`src/roster.ts:454`) and `resolveDmTarget`
  (`src/roster.ts:673-`); `wakeCmuxSurface` code and the `to interrupt` busy test (`src/roster.ts:253-275`);
  the Codex-only live-process guard (`src/roster.ts:181-226`); cmux branch in `src/deliver.ts:524-538`
  (busy reported as `failed`); `senderName` (`src/cli.ts:207-214`); `failNoSelfName`, `wakeReplyPolicy`,
  `dmCmuxBusy`, header (`src/i18n.ts:561-562,512,534-535,315-325`); skill text (`src/cli.ts:1494,1545`),
  `SKILL_MD` span (`src/cli.ts:1492-1665`), `skills/ocs/SKILL.md` = 12,748 B; size constants and
  truncation; README `349-351`, `443-445`; `docs/wake-protocol.md:65-66,236-238`; Goose skill roots
  (both trees), `(Ctrl+C to interrupt)` hint (1.36 `output.rs:124-139`, main `output.rs:170`), Ctrl+J newline
  binding (default `j`, `input.rs:85-91,138-141`), spinner `stop("")` (`output.rs:143-146`).
- Changed: self-identity is not env-only (Claude is also found by process-ancestry walk; `$OCS_NAME` comes
  first), corrected with `src/roster.ts:100-117`, `src/wake.ts:243-293`. "Ships no hooks" narrowed to "writes
  no harness hook config" and the Pi extension's event subscriptions noted. Spinner-hide citations corrected
  (`mod.rs:1211` is the message-render path, the approval select is `mod.rs:1679-1680`; added the re-show site
  `output.rs:251`). Fixed line citations: `README.md:300,302` and `src/cli.ts:1531,1535` (was `301-302`,
  `1540`), `src/cli.ts:1675-1683` (was `1676-1681`), `src/wake.ts:90` for the no-wrapper note (was `82-83`),
  `src/wake.ts:117-172` (was `119-170`), `src/wake.ts:85-98` (was `85-97`), `src/wake.ts:40-46` (was `41-47`).
- Not checked: cmux's `\n` handling and cliclack's `stop("")` line clearing remain unknown; the Goose-side
  terminal-mode consequences remain INFERRED from the primer.
