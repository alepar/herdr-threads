# open-cross-session (ocs) x OpenCode

- Upstream: https://github.com/leeguooooo/open-cross-session. The clone read is `scratchpad/alts/open-cross-session-ocs` at HEAD `1a3cee23a` (tag `v0.9.4`). It was not executed.
- OpenCode source: the primer's `v1.18.1` clone (`scratchpad/harness-src/opencode-1.18.1`, https://github.com/anomalyco/opencode). The harness facts come from `primers/opencode.md`.
- Paths are relative to each clone root. VERIFIED means read in source or docs. INFERRED means my reading of the code, not observed running.

## Support status

**Not supported.** ocs has no OpenCode carrier, no discovery and no identity detection for it.

- `grep -rniE 'opencode|open-code|sst/|anomalyco'` over the whole clone (source, docs, skills, tests, installer) returns 0 hits. VERIFIED.
- The GitHub issue search `repo:leeguooooo/open-cross-session opencode` returns 0 results (gh api, 2026-10-23). VERIFIED.
- Self-identity resolution checks, in order: `$OCS_NAME`, Grok, Pi, Claude, Codex, Hermes (`src/roster.ts:91-120`). It has no OpenCode branch, and it does not read `OPENCODE`/`OPENCODE_PID`. VERIFIED.
- The roster (`ocs who`) lists Claude, Codex, Pi, Hermes, Grok, agy and cmux surfaces only (`src/roster.ts:436-451`, `src/cli.ts:995-999`). VERIFIED. An OpenCode session never shows up as an agent.
- The skill describes itself as for "Claude Code, Codex, Pi and Hermes sessions" (`skills/ocs/SKILL.md:3`). The installer targets Claude Code, Codex and Pi (`install.sh:66-87`). VERIFIED.

An OpenCode agent therefore has only two generic paths:

1. **The CLI**, run from OpenCode's bash tool: `ocs send/read/inbox/watch --as <name>`.
2. **The cmux keystroke carrier.** It applies only if OpenCode runs inside a cmux terminal surface and the sender addresses it as `ocs dm surface:<n>`.

The rest of this report analyses those two paths against the OpenCode primer.

## Injection mechanism

### Path A: CLI pull (the agent polls)

- **Mechanism.** The OpenCode model runs `ocs read <channel> --as <me>` or `ocs inbox --as <me>` through its bash tool. Output enters context as an ordinary tool result. VERIFIED that these commands exist (`README.md:295-310`). INFERRED that this is how an OpenCode agent would use them; nothing in ocs documents OpenCode.
- **When it fires.** Only when the model decides to call the tool. There is no hook, so nothing reaches the context at turn start or after an unrelated tool call. VERIFIED that ocs installs no hooks (overview; `grep` for hook names in `src/` finds only a comment).
- **Identity.** It has to be explicit. `senderName()` falls through to `fail(M.failNoSelfName)` when neither `--as`, `$OCS_NAME` nor a known harness env var is present (`src/cli.ts:207-214`). The send is then refused with "cannot infer sender identity; no message was stored or sent" (`src/i18n.ts:561-562` en, `:882` zh). VERIFIED.
  - Hazard (INFERRED): OpenCode launched from inside a Claude Code, Codex or Hermes shell inherits `CODEX_THREAD_ID` / `HERMES_SESSION_ID`, or has a Claude pid in its ancestor chain. ocs would then silently attribute the OpenCode agent's sends and reads to the parent harness's identity. `src/roster.ts:91-92` acknowledges the same issue for Hermes ("a Claude/Codex opened inside Hermes also inherits it") and orders Hermes last for that reason. Nothing guards against OpenCode doing the same.
- **Read position and dedupe.** A per-consumer cursor file (`~/.ocs/cursors/<channel>.<consumer>.json`) advances on `ocs read`. `--peek` leaves it in place. VERIFIED (overview, `README.md:305`).
- **Ack.** There is none. A `wake: accepted` result is "**Not proof it was read.**" (`README.md:273`). VERIFIED.
- **Bounds.** No per-read page bound was found for `ocs read`. Unknown. Any large output is subject to OpenCode's own tool-output truncation, which spills to a file (primer section 5, INFERRED for bash).

### Path B: cmux keystrokes (push into the TUI composer)

The code path, VERIFIED:

1. `resolveDmTarget("surface:N")` returns `{kind:"cmux", name:"surface-N", identity:"cmux:surface:N", cmuxRef}` (`src/roster.ts:689-690`).
2. `deliver.ts:524-537` builds the wake note and calls `wakeCmuxSurface(ref, note)`.
3. `wakeCmuxSurface` runs these steps (`src/roster.ts:253-275`):

   ```ts
   const screen = spawnSync("cmux", ["read-screen", "--surface", ref, "--lines", "8"], ...);
   if (screen.stdout.includes("to interrupt")) return { ok: false, reason: "busy" };
   const send = spawnSync("cmux", ["send", "--surface", ref, note], ...);
   const enter = spawnSync("cmux", ["send-key", "--surface", ref, "enter"], ...);
   ```

4. The text lands in OpenCode's composer and is submitted with Enter. OpenCode turns it into a normal user message, which goes through `chat.message` and is persisted (primer section 2).

Payload and framing, VERIFIED (`src/wake.ts:131-167`, `docs/wake-protocol.md:28-75`):

```
[ocs wake] <sender> mentioned you in #<channel> (seq <N>[, reply to seq <M>][, <ago>])

<body, verbatim up to 4096 B, else first 512 B + "… (<total> bytes total; full text: ocs read <ch> --as surface-N)">

Reply when asked, or with a needed decision/result. No unsolicited acknowledgements.
Reply: ocs send <channel> "<your reply>" --as surface-N --reply-to <N>
Thread: ocs read <channel> --as surface-N
```

- cmux targets cannot self-identify, so the Reply and Thread lines carry `--as surface-N` (`src/deliver.ts:525`, `docs/wake-protocol.md:65-66`). VERIFIED. This happens to be what makes the OpenCode agent's reply work at all.
- The note has no `<cross-session-message>` wrapper; that wrapper is Claude-only. The body is still neutralised: `<cross-session-message` becomes `‹…`, and lines starting with `Reply:`/`Thread:`/`[ocs wake]` get a `> ` prefix (`src/wake.ts:85-97`). VERIFIED. In OpenCode, the model sees the note as text the human typed, with no provenance. INFERRED from the primer: it is not a `synthetic` part, so the TUI shows it as the user's own bubble.
- Size cap: the note is at most 5120 B (4096 body + 1024 skeleton, `src/wake.ts:40-46`). VERIFIED.
- Dedupe: at-most-once. A failed or busy outcome is not retried, and the message stays in the log (`src/deliver.ts:530-536`, `DESIGN.md:34`). VERIFIED.

How the note behaves in OpenCode's composer is INFERRED and unknown in parts:

- If `cmux send` pastes with bracketed paste, which is unknown (cmux is third-party and was not read), OpenCode collapses any paste of 3+ lines or more than 150 characters into a `[Pasted ~N lines]` extmark that expands on submit (`packages/tui/src/component/prompt/index.tsx:1205-1210` in OpenCode). The wake note is always multi-line, so the human sees a placeholder rather than the text. Submission still works.
- If `cmux send` instead types newlines as Return, each line of the note would submit as a separate user message, because Enter is `input_submit` (primer section 7). That would be a disaster. ocs's own Codex test only asserts the argv (`test/codex.test.ts:532-535`), so this is not covered for any harness.

## Wake mechanism

- **Mechanism.** Injection and wake are the same act (path B). Submitting the composer starts a turn if OpenCode is idle. VERIFIED for ocs (`src/roster.ts:249-252`), INFERRED for OpenCode.
- **Path A has no wake at all.** An idle OpenCode agent at an empty prompt never learns about a message. "Persistence has no auto-nudge" (`README.md:349-351`). VERIFIED.
- **Busy detection is broken for OpenCode.** ocs treats a surface as busy only if its last 8 screen lines contain the literal `to interrupt` (`src/roster.ts:263`). That is the Claude/Codex footer wording. A busy OpenCode TUI renders `esc interrupt`, and it shows `esc again to interrupt` only after one ESC has been pressed (OpenCode `packages/tui/src/component/prompt/index.tsx:1584-1588`). VERIFIED for both sides. The consequence follows (INFERRED from these two facts):
  - ocs considers a busy OpenCode idle and submits anyway.
  - The outcome is benign by luck. OpenCode does not reject a busy submit. The server persists the message, the TUI labels it `QUEUED`, and the running loop picks it up at the next step boundary (primer section 3, `effect/runner.ts:115-122`, `prompt.ts:1111-1116`). So the message is steered into the running turn rather than lost.
  - This breaks ocs's own stated promise that "cmux never types into a busy TUI" (`README.md:47`).
- **Draft safety: none.** ocs never inspects the composer. A human's half-typed draft gets the note appended at the cursor, and Enter submits both as one message. VERIFIED that the code has no draft check (`src/roster.ts:253-275`). This matches the overview's finding for Codex. OpenCode gives an external process no way to see the draft except a screen read (primer section 7). ocs's 8-line screen read could in principle detect a non-empty composer, but it does not look.
- **Autocomplete hazard (INFERRED).** The note starts with `[ocs wake]`, so a leading `/` or `@` cannot open OpenCode's autocomplete popup. But if `cmux send` types character by character and the body contains `@word`, the popup could still open. The final Enter would then select a completion instead of submitting, because `submit()` refuses while `auto()?.visible` (primer section 7). Unknown, since it depends on the cmux send mode.
- **Target validation.** For an explicit `surface:N`, ocs does not check that any agent process is running in the surface. The process filter exists only in the automatic Codex fallback (`src/roster.ts:182-213`, which requires `process.name === "codex"`). VERIFIED. The consequence (INFERRED): if OpenCode has exited and the pane is back at a shell, the multi-line note is typed into the shell and Enter executes it, lines such as `Reply: ocs send ...` included. The code comment at `src/roster.ts:182-184` names exactly this risk ("otherwise after Codex exits the wake body gets typed into the shell"), but guards only the Codex fallback.
- **Latency.** The wake is synchronous in the sender's CLI process: `cmux ping` (2 s timeout), then `read-screen`, `send` and `send-key` (5 s timeout each) (`src/roster.ts:154-158`, `253-270`). VERIFIED. That is effectively immediate when it works.
- **Failure reporting.** A busy surface or a cmux error logs `dmCmuxBusy` / `dmCmuxFailed` and makes the send exit 2 (`wake: failed`). The message stays stored (`src/deliver.ts:530-536`, `README.md:278`). VERIFIED. For OpenCode the busy branch never triggers (see above), so the realistic failures are "cmux not installed", "pane not in cmux" and "wrong surface".
- **No idle subscription.** `notify-when-idle` polls Claude session files only (`docs/wake-protocol.md:132-170`). VERIFIED. An OpenCode peer cannot be waited on, and it cannot subscribe either: it has no Claude session for the notice to land in.

## Setup and installation footprint on the harness

- **On OpenCode: nothing.** No plugin, no `opencode.json` edit, no MCP server, no TUI plugin. VERIFIED (no OpenCode paths in the installer or in `src/cli.ts:1678-1680`).
- **Incidental skill exposure (INFERRED).** The embedded installer writes `~/.claude/skills/ocs/SKILL.md` (`src/cli.ts:1678`). `install.sh:75-77` also runs `npx skills add ... --global --agent claude-code --agent codex --agent pi`, and `ocs upgrade` treats `~/.agents/skills/ocs` as one place a skill install may live (`src/upgrade.ts:151`, detection only, not a write); which directories the third-party `skills` CLI actually writes is unknown. OpenCode loads both `~/.claude/skills/**/SKILL.md` and `~/.agents/skills/**/SKILL.md` unless the external-skill flags disable them (primer section 6, `skill/index.ts:21-25,187-191`). So an OpenCode user on a machine with ocs installed probably gets the `ocs` skill listed, with text claiming that identity is auto-detected (`skills/ocs/SKILL.md:54`). For OpenCode that claim is false, so the agent must learn `--as` from the failure message.
- **Machine-wide footprint.** The `ocs` binary, `~/.ocs` (logs, cursors), and optionally cmux. VERIFIED (overview).

## Token cost

- **Per message (path B).** One user turn containing the note. The skeleton is header plus policy line plus Reply/Thread lines, around 250-400 B for typical channel and name lengths (INFERRED from `src/wake.ts:151` and the i18n strings). The body is up to 4096 B inline, and the whole note is at most 5120 B. On top of that comes the model's full turn. The "No unsolicited acknowledgements" line exists to suppress reply chatter (`src/i18n.ts:512`). VERIFIED.
- **Per message (path A).** One bash tool call plus its output, and the model must decide to poll. Polling while nothing is waiting costs a tool round each time (INFERRED).
- **Per quiet turn: zero.** No hooks, no system-prompt edits, no MCP instructions. VERIFIED. The only static cost is the skill's listing line (`skills/ocs/SKILL.md:3`, about 230 B) if OpenCode picks it up; the 12.7 KB body loads on demand (INFERRED, OpenCode lists skills in the system prompt).

## Limitations and bugs visible for OpenCode

1. **No native carrier, discovery or identity.** OpenCode is a CLI-only participant unless it runs under cmux. VERIFIED.
2. **The cmux busy check looks for Claude/Codex wording and misses OpenCode's `esc interrupt`.** A busy OpenCode is treated as idle. VERIFIED on both sides; the effect is INFERRED. It happens to be harmless because OpenCode queues busy submits, but it contradicts `README.md:47`.
3. **No draft protection.** A human's draft is merged into the wake submission. VERIFIED.
4. **No liveness check on explicit `surface:N`.** After OpenCode exits, the note could be typed into a shell and executed (INFERRED; the risk is named in the code at `src/roster.ts:182-184`).
5. **Multi-line note over keystrokes.** Whether newlines survive as text or act as Enter depends on cmux send semantics, which are unknown and untested (`test/codex.test.ts:532-535` asserts only the argv).
6. **No provenance.** In OpenCode the message looks human-typed: it is not a `synthetic` part and has no wrapper. VERIFIED that there is no wrapper for cmux; the rest is INFERRED.
7. **Inherited-env identity theft.** An OpenCode started inside another harness's shell is attributed to that harness (INFERRED, analogous to `src/roster.ts:91-92`).
8. **Skill text misstates OpenCode identity.** It says auto-detected (INFERRED applicability, VERIFIED text at `skills/ocs/SKILL.md:54`).

### How well the generic mechanism fits OpenCode (using the primer)

- **The CLI works**, but it is pull-only and gives no wake. That is the floor.
- **cmux keystrokes work** but use OpenCode's weakest surface (primer section 12, option 4). ocs chose not to use OpenCode's best surfaces, none of which needs keystrokes:
  - a server plugin calling `client.session.promptAsync`;
  - a plugin that, on `session.status` = idle, calls `prompt_async` with `synthetic:true` parts;
  - `noReply:true` for silent staging;
  - a TUI plugin reading `TuiPromptRef.current.input` for draft-aware wakes.
- **ocs's own architecture already points to the right OpenCode shape.** ocs's Pi carrier is an in-harness extension that listens on a token-authenticated, mode-0600 UDS and calls the harness's native input API with "queue if busy, trigger a turn if idle" semantics (`src/pi-extension.ts:140-200`):

  ```ts
  pi.sendMessage({ customType: "ocs", content: frame.note, display: true, ... },
                 { deliverAs: "followUp", triggerTurn: true });
  ```

  An OpenCode server plugin would be a direct port. The UDS listener would call `client.session.promptAsync({ path: {id}, body: { parts: [{ type: "text", text: note }] } })`, which OpenCode queues when busy and runs when idle (primer section 3). On 2.x it would call `POST /api/session/:id/synthetic` with `delivery:"queue"` instead (primer section 9). The plugin would also export `OCS_*_SESSION_ID` through `shell.env` (primer section 10), as the Pi extension does with `process.env.OCS_PI_SESSION_ID` (`src/pi-extension.ts:200`). All of this is INFERRED; ocs has not done it.

## Clever ideas worth stealing

- **Native input queue instead of keystrokes.** ocs uses each harness's own "submit, queue if busy" API: Pi `followUp`/`triggerTurn`, Claude `priority:"next"`, Hermes `queued:true`. For OpenCode, `promptAsync` from an in-process plugin is the same pattern. herdr-threads currently has no OpenCode adapter, and Herdr's native wake refuses the `opencode` agent kind (primer section 11). That makes an OpenCode server plugin modelled on ocs's Pi extension the natural first adapter: draft-safe by construction, with no composer contact.
- **In-harness UDS listener with a per-session token**, registered under a state dir (`src/pi-extension.ts:140-200`, a 16 KiB frame cap at `:22`, `chmod 0600`). A cheap, local and authenticated push channel from the daemon into the harness process. It is comparable to herdr-threads' Claude mod channel and could be reused for an OpenCode plugin.
- **Body neutralisation** (`src/wake.ts:85-97`). Rewrite wrapper-tag openers and prefix lines that mimic protocol markers, so a peer body cannot forge framing or a reply command. herdr-threads' header-line framing could apply the same rule to body lines that start with `[herdr-threads]`.
- **Copy-paste Reply and Thread lines carrying the exact `--as` identity** when the receiver cannot self-identify (`docs/wake-protocol.md:65-66`). For an OpenCode seat before an adapter exists, this is the minimum that makes replies attributable.
- **A process-verified terminal target** before typing (`src/roster.ts:182-213`): only type into a surface whose foreground process is the expected agent. ocs applies this only to Codex. The lesson for herdr-threads' native wakes is to pair the screen status with a process check.
- **Exit-code contract**: `stored:true` is never safe to resend, and `unknown` is never replayed (`README.md:265-281`). It is a clear at-most-once wake contract for a CLI sender.

**Counter-lesson.** Screen-text busy detection keyed on one harness's footer wording (`"to interrupt"`) silently misclassifies other TUIs. Any keystroke-wake path herdr-threads adds for OpenCode should use OpenCode's own status (the `session.status` event or `GET /session/status`) rather than screen text.

## Verification

Adversarial pass, 2026-10-23, against the ocs clone at `1a3cee23a` (v0.9.4), the OpenCode 1.18.1 clone and `primers/opencode.md`.

- Re-checked: the zero-hit OpenCode grep and the GitHub issue search (`total_count` 0); `resolveSelfName` order (`src/roster.ts:91-119`); roster kinds (`src/roster.ts:436-451`, `src/cli.ts:995-999`); `senderName` (`src/cli.ts:207-214`); the cmux target and wake code (`src/roster.ts:154-158,182-213,253-275`, `src/deliver.ts:524-537`); note building and neutralisation (`src/wake.ts:85-97,131-167`, `docs/wake-protocol.md:28-75`); the Pi extension (`src/pi-extension.ts:22,131-200`); the skill text and installer (`skills/ocs/SKILL.md:3,54`, `install.sh:66-87`); README lines 47, 302, 349-351; `DESIGN.md:34`; `test/codex.test.ts:532-535`; that `src/` installs no hooks. On the OpenCode side: the paste summary (`prompt/index.tsx:1204-1211`), the `esc interrupt` footer (`:1584-1588`), the external skill dirs (`skill/index.ts:21-25,187-191`), and primer sections 3, 7, 11 and 12. All held.
- Corrected: the read-receipt quote, which was not verbatim (now `README.md:273`); the no-identity error, which now quotes the English string at `src/i18n.ts:561-562`; the size-cap citation (`src/wake.ts:40-46`, not 46-52); the failed-exit citation (`README.md:278`); the skeleton header, which now includes the optional `<ago>`; and the `~/.agents/skills` claim. `src/upgrade.ts:151` only detects that directory and does not write it. The `skills` CLI invocation is at `install.sh:75-77`.
