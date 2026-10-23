# agmsg x Antigravity CLI (agy)

- Upstream: https://github.com/fujibee/agmsg. Clone: shallow, HEAD `6e0315468db3` (2026-10-23, "release: 1.6.0 (#1727)").
  Paths below are relative to the clone root. No code from the clone was executed.
- Harness primer: `../../primers/agy.md`. herdr-threads evidence uses repo-relative paths in this repo.
- Tags: **VERIFIED** means read in source or docs (cited). **INFERRED** means my reading, not confirmed by source or a run.
  **unknown** means nothing I read answers it.

## Support status

agy is supported natively and labelled experimental. VERIFIED.

- Type manifest `scripts/drivers/types/antigravity/type.conf:1-13`:
  - `name=antigravity`, `cli=agy`, `cmd_prefix=$` (the skill is invoked as `$agmsg`);
  - `prompt_arg=--prompt-interactive`, `spawnable=yes`;
  - `hooks_file=.agent/rules/agmsg.md`, `delivery_modes=monitor turn off`.
- `CHANGELOG.md:1145` added gemini and antigravity support (#45). Later releases added the read guard, macOS support,
  Linux auto-resume and the `agy-tui` pass-through (`CHANGELOG.md:209-247`).
- agmsgd, the 1.6.0 daemon, does not cover agy: "delivers to Claude Code and Codex only, so it does not cover Antigravity"
  (`docs/antigravity-monitor-beta.md:22`).
- There are three delivery paths. None of them uses agy's hook system.

| Mode | Mechanism | Reaches the human's TUI conversation? | Wakes idle? |
|---|---|---|---|
| `turn` (default) | A project rule file tells the model to run `check-inbox.sh` "after each tool call" | Yes, when the model complies | No |
| `monitor` via `agy-tui` | A Python PTY supervisor owns agy's terminal and pastes envelopes when the screen matches an idle signature | Yes | Yes |
| `monitor` via headless bridge | A Node bridge drives a separate `agy --input-format stream-json` worker conversation | **No**: a separate conversation | Yes, for the worker |
| `off` | Manual `$agmsg` only | — | No |

**No agy hooks.** agmsg writes no `hooks.json`, no plugin, and no `SessionStart`/`PreInvocation`/`Stop` entries for agy.
VERIFIED by absence: no hook writes under `scripts/drivers/types/antigravity/`, and `install.sh` only installs a skill and a shim
(`install.sh:790-847`).

## Injection mechanism

### A. `turn`: a cooperative rule file

**What gets written.** `delivery.sh set turn antigravity <project>` runs the type plug `agmsg_delivery_apply`, which first stops
any headless bridge (`antigravity-mode.mjs stop`) and then calls `rulefile_apply`
(`scripts/drivers/types/antigravity/_delivery.sh:12-15`). That deletes and rewrites
`<project>/.agent/rules/agmsg.md` (`scripts/lib/delivery-rulefile.sh:11-46`). VERIFIED. The file reads:

```markdown
# agmsg Integration Rule

## PostToolUse
After each tool call, automatically check the agmsg inbox for unread messages.
- Command: '<SKILL_DIR>/scripts/check-inbox.sh' 'antigravity' '<project>'
## Terminal/pane self-awareness ...   (where.sh guidance)
## Teammates: placement, status, and reaching them ...   (team.sh/peek/poke/arrange)
```

**Who runs it.** The model. "PostToolUse" in that file is only a heading. agy's `PostToolUse` hook cannot inject anything
(primer §2.3), and agmsg does not register a hook. The rule is prose the model may or may not follow.
`delivery-rulefile.sh:4-5` describes the mechanism as "tells the agent to poll the agmsg inbox after each tool call". VERIFIED.

**When it fires.**
- Only when the model chooses to call `run_command` with that command.
- Never while the session is idle, and never at turn start unless a tool call happens first.
- No turn boundary forces it. INFERRED from the mechanism.

**What the model sees.** `check-inbox.sh` is shared with the hook harnesses and emits hook-shaped JSON, so the model receives
that JSON as the stdout of a shell tool call. VERIFIED.
- The rule's command passes no event argument, so `EVENT` defaults to `Stop` (`scripts/check-inbox.sh:28`) and the Stop
  output shape below applies.
- `scripts/check-inbox.sh:373-380` builds the text: `"<N> new message(s) in <team>:"` then `"  [<ts>] <from>: <body>"` per message.
- `:462` prints it as `{"decision":"block","reason":"<escaped text>"}`. Newlines are JSON-escaped to `\n`, so the model reads
  one long escaped string inside a JSON wrapper meant for the Claude/Codex Stop hook. INFERRED: an awkward frame for a model
  reading tool output, but it is parseable.
- Quiet turns print nothing for agy. The JSON status is emitted only when the manifest sets `stop_output=json` (`:49-53`),
  and antigravity's manifest does not set it. VERIFIED.

**Cooldown.**
- Each agent has a marker `run/.lastcheck-<agent>` (`:178-198`). A check within `delivery.turn.check_interval`, 60 s by
  default (`:186-192`), exits silently. VERIFIED.
- So a model that obeys "after each tool call" performs a real tool call that usually returns nothing. INFERRED.

**Size bounds.** None in this path. `storage_list_unread` takes an optional `--limit` (`scripts/drivers/storage/sqlite.sh:459-500`),
but `check-inbox.sh` contains no `limit` at all. VERIFIED by grep. No body truncation was found either, so this path is
unbounded, consistent with the overview.

**Dedupe and ack.**
- Read state is consumed only after the `printf` of the payload succeeds (`check-inbox.sh:502-509`). The script states that
  this proves only "the write to this process's stdout returned success", not that the model received it (`:484-490`). VERIFIED.
- No receipt. A second check after the cooldown shows only messages that are still unread.

**Docs drift.** The agy skill overlay describes `turn` as "Check inbox at the end of each assistant turn / Stop hook pulls after
each response" (`scripts/drivers/types/antigravity/template.md:10-11`). That is wrong for agy, which has no Stop hook entry
here and only the rule file. VERIFIED by comparison.

**Unknown: rule path.** Whether agy 1.3.x loads `.agent/rules/` (singular) is unknown.
- The primer's third-party source lists `.agents/rules/*.md` (plural), together with GEMINI.md and AGENTS.md (primer §3).
- agmsg's tests only assert the file's path and content: the turn-mode rule-file tests are written for gemini
  (`tests/test_delivery.bats:1566-1587`), and the antigravity ones check the monitor marker and migration
  (`tests/test_delivery.bats:2780`, `:2794`; `tests/antigravity_bridge.test.mjs:74-86`). I found no live check that agy reads it.
- If agy only reads `.agents/`, turn mode is silently inert. That is a lead to confirm, not a finding.

### B. `monitor` via `agy-tui`: PTY supervisor (the real injection path)

**Process model.**
- `~/.agents/bin/agy-tui` is a shim (`install.sh:790-829`). It execs `antigravity-tui-monitor.sh`, which requires python3 and a
  TTY (`antigravity-tui-monitor.sh:1-20`).
- That script execs `antigravity-tui-supervisor.py`. The supervisor `pty.fork()`s agy in the project dir, relays the user's raw
  keystrokes to it, tees agy's output to the real terminal, and feeds the output into its own VT screen model
  (`antigravity-tui-supervisor.py:579-589`, `:710-738`). VERIFIED.

**Preconditions.** VERIFIED.
- The rule file must carry the marker `<!-- agmsg:antigravity:monitor -->` (`supervisor.py:517-519`). In monitor mode the rule
  file says "Do not call inbox.sh/check-inbox.sh" (`_delivery.sh:69-73`).
- An exclusive `actas` lock and a read reservation are claimed, holding owner, pid, start token and the sha256 of a random
  capability (`supervisor.py:542-547`).

**Polling.** Every `--poll` seconds, 2 by default (`:799`), `maybe_poll` runs (`:631-650`). It skips the poll when the screen
is not idle, when a human is typing, or when a pause latch is set. Otherwise it calls `inbox-transport.sh peek`, which is
`storage_list_unread … --limit 20` (`inbox-transport.sh:71`). VERIFIED.

**Batching and bounds.** VERIFIED.
- Messages are added until their bodies total 64 KiB (`supervisor.py:641-647`).
- A single body over 65,536 bytes calls `fail(...)` (`:644`). That sets `durableAttention` and stops the supervisor (`:483-489`).

**Envelope** (`supervisor.py:590-596`). VERIFIED:

```
[agmsg batch id=<uuid4> count=<n>]
[agmsg message id=<id>]
from: <from>
at: <at>
body:
<body with ESC replaced by the literal text \x1b>
[/agmsg message]
...
[/agmsg batch]
After reading this delivery, first output exactly one line consisting of AGMSG_RECEIVED, an ASCII colon (U+003A),
and the batch id with no spaces, then proceed normally. Do not run tools to inspect the format.
```

**Write.** One `os.write(master, b'\x1b[200~' + data + b'\x1b[201~\r')`: a bracketed paste followed immediately by CR to
submit (`:606-614`).
- Only ESC is neutralized. Other control bytes in a body (CR, ^C) pass through inside the paste. The doc says "Control bytes
  in bodies are escaped" (`docs/antigravity-monitor-beta.md:324`). The code escapes only `\x1b` (`:593`). VERIFIED discrepancy.
  Whether agy treats a CR inside a bracketed paste as literal text is unknown.
- Bracketed paste worked on agy 1.1.27: an isolated live test injected a batch, saw the receipt and acked it
  (`docs/design/antigravity-tui-pty-monitor.md:175`). VERIFIED (their record). On 1.3.x it is unknown; the primer has
  bracketed-paste support as unknown.

**Framing as a user turn.** The envelope arrives as an ordinary user prompt in the live TUI conversation. It is permanent in
the transcript and visible on screen. INFERRED from the mechanism; primer §1 says keystrokes arrive "as a user turn".

**Ack: receipt on the reconstructed screen.**
- After injection, every chunk of agy output is fed to `TerminalScreen`.
- `lines_after(receipt)` looks for `AGMSG_RECEIVED:<batch-id>` as a complete logical line. It joins consecutive wrapped physical
  rows and never joins across a blank row (`:310-326`, `:719-727`).
- The ack is refused (`fail`) when:
  - the receipt appears inside a delivered body (`:598-601`, `:724`);
  - the screen saw an unsupported control sequence, a resize or an alternate-screen toggle during the turn (`:725`, `:170-182`, `:265-273`);
  - an `error:`/`cancel:`/`interrupt:`/`permission denied:` line follows the receipt (`:615-618`, `:726`).
- Otherwise `ack` passes the id list to `inbox-transport.sh ack`, with the capability on fd 3, and marks exactly those ids read
  (`:619-630`, `inbox-transport.sh:72-81`).
- The receipt literal is never in the envelope; only its construction rule is. The batch id is a fresh uuid4, so a body cannot
  pre-forge the receipt (`docs/antigravity-monitor-beta.md:325-326`). VERIFIED.

**Single writer, enforced.**
- While the reservation exists, `storage_mark_read_batch` and `storage_read_cursor_consume` are wrapped by a guard
  (`scripts/lib/bridge-read-guard.sh:82-97`). The guard allows a mark-read only when:
  - the caller's parent pid is the reservation's pid;
  - the capability hash matches;
  - the batch is `completed`;
  - the id set matches exactly (`bridge-read-guard.mjs:76-89`).
- Any other mark-read, including the model running `inbox.sh` or a bare `$agmsg`, appends a `read-denied` row and exits 13
  (`:90-103`). The supervisor sees the violation and stops without acking (`supervisor.py:490-495`). VERIFIED.

**Recovery is manual.** An unresolved batch blocks the next start. The operator runs `agy-tui ack` (the model answered) or
`agy-tui replay` (it did not), with `--batch` and every `--confirm-id` (`supervisor.py:497-516`, `:772-796`). VERIFIED.

### C. `monitor` via the headless bridge

- `antigravity-monitor.sh` execs `antigravity-bridge.mjs` (`antigravity-monitor.sh:19`).
- It spawns `agy --input-format stream-json --output-format stream-json --print-timeout 5m [--conversation <id>]`
  (`antigravity-bridge.mjs:102-108`).
- The first user line is an init prompt: "You are the agmsg headless worker … Do not call inbox.sh … reply with send.sh …"
  (`:128`). The conversation id from `init` is recorded (`:134-135`).
- Every 2 s, `tick` peeks, at most 20 rows and 64 KiB, and writes
  `{"event":"user","message":{"content": JSON.stringify(batch)}}`. The payload is the raw batch JSON, with id, phase and
  messages (`:151-167`, `:95-101`).
- The ack is a `result` with `status:"SUCCESS"`, not a receipt line (`:142-147`).
- `forbiddenTool` scans `step_update` tool events for `inbox.sh`, `check-inbox.sh` or `$agmsg` in the command line, and fails
  the bridge if it finds one (`:14-20`, `:138`). VERIFIED.
- This worker is not the human's conversation. agmsg tested running a headless process against the live TUI's conversation id
  on agy 1.1.27. The headless turn succeeded, but the TUI answered `kiwi` and the external turn's `mango` did not reach the live
  TUI context (my translation of the Japanese record, `docs/design/antigravity-conversation-registration.md:14-17`, `:41-44`;
  English summary in `docs/antigravity-monitor-beta.md:44-54`).
  VERIFIED (their measurement). The bridge is kept for `standalone` operation only.

## Wake mechanism

**`turn`: no wake.** An idle agy session at an empty prompt never notices a message. VERIFIED by mechanism; the README states
the same for turn-mode harnesses generally (`README.md:219`, `:275`).

**`poke` (manual, any mode).** Another agent can run `poke.sh <team> <name>`. Under Herdr this becomes
`herdr agent prompt <pane> <text>` (`scripts/drivers/terminals/herdr/ops.sh:1084-1102`). VERIFIED.
- The draft guard does not cover agy. A type without `input_prompt_marker` is poked unconditionally ("An empty marker means:
  no check, one unconditional terminal_poke call", `scripts/lib/safe-poke.sh:367-372`; `scripts/poke.sh:171-178`), and
  `antigravity/type.conf` sets no marker. VERIFIED.
- Only Herdr's own `agent prompt` behaviour stands between a poke and a human's draft. That behaviour is unknown here.

**`agy-tui`: wake by pasting into the empty composer.** This is the only automatic wake.

*Idle detection* is `input_ready` (`supervisor.py:651-665`). All of these must hold:
- the screen model is certain;
- the UTF-8 decoder is not mid-sequence;
- there has been no agy output for 0.3 s;
- the bottom logical line starts `? for shortcuts` and contains ` ·`;
- after dropping `─━-` rule lines, the last non-empty row above the footer is exactly `>`.

`injection_ready` adds a zero-timeout `select` on stdin and the PTY master, so pending human input or unread child output
defers injection (`:666-672`). The gate is re-evaluated after `peek` and save, immediately before the write (`:649-650`). VERIFIED.

*Version handling.* The docs say the signatures are "pinned to the `agy` version whose screen output was actually measured"
(1.1.27; `docs/antigravity-monitor-beta.md:397-399`). The design requires refusing to start when `agy --version` differs
(`docs/design/antigravity-tui-pty-monitor.md:87`). **The code has no version check.** `grep -i version` over the driver finds
only state `schemaVersion` (`supervisor.py:360`, `:476-481`). VERIFIED. In practice the pin is a fail-closed screen match:
- an unmatched screen simply never reads idle;
- an unknown escape sequence during a receipt turn blocks the ack.

*Does the 1.1.27 signature hold on 1.3.2?* Checked against herdr-threads' genuine 1.3.2 captures. INFERRED: likely yes.
- Idle: the rule line, the `>` row and the rule line above `? for shortcuts … Gemini 3.1 Pro · low` match
  (`tests/fixtures/agy/composer/empty.read-detection.txt`).
- Draft: the `> hello draft` row and the blank-led footer both fail the gate (`draft.read-detection.txt`).
- Busy: the footer is `esc to cancel`, so the gate fails (`busy.read-detection.txt`).
- Live subagent: the footer is `? for shortcuts … · 1 subagent(s)`, but the row above it is `● Agent(...) Working · 1s`, not `>`,
  so the gate fails (`subagent-generating.read-detection.txt`). This is the screen where Herdr reports a false idle, and
  agmsg's bottom-anchored gate rejects it by layout.

*Draft safety.*
- Any keystroke outside a delivery turn sets `humanInputActive` and pauses delivery (`supervisor.py:372-376`, `:737`). Delivery
  resumes on its own only after a non-idle screen, then 0.6 s of stable idle with no pending I/O (`:673-693`, `HUMAN_IDLE_STABLE_SECONDS=0.6` at `:352`).
- A keystroke *during* a delivery turn marks the batch `uncertain` and stops the supervisor (`:731-736`). The one exception is a
  permission or trust modal whose structure matches (`:377-406`).
- The docs admit "The input box is never proven empty" (`docs/antigravity-monitor-beta.md:405-406`).
- A pasted body containing a `>` line followed by `? for shortcuts` latches `manualResumeRequired` after the ack, because it
  could fake the idle signature (`supervisor.py:603-605`, `:627-629`). VERIFIED.

*Busy behaviour.* Nothing is injected mid-turn. A message that arrives while agy works waits for the idle screen, and is then
delivered as a new user turn. INFERRED from the gate; VERIFIED that the gate rejects the `esc to cancel` footer.

*Latency.* At best about the 2 s poll interval plus 0.3 s of output quiet. After human typing, add the non-idle and 0.6 s stable
idle requirement (constants VERIFIED above). The overall figure is INFERRED; there are no measured end-to-end numbers.

*Failure modes.* VERIFIED unless marked.
- Permission dialog during a delivery turn: agy 1.2.5's permission screen did not match, so the keypress stops the supervisor
  with an uncertain batch. This is why the documented launch is `agy-tui … -- --dangerously-skip-permissions`
  (`docs/antigravity-monitor-beta.md:3-16`, `:190`, `:418-425`).
- Any unsupported CSI or ESC sequence during a receipt turn (`supervisor.py:273`, `:294`) means no ack. A real case: `Read` output
  with `CSI ?5W` and `CSI Z` stranded four messages (`docs/antigravity-monitor-beta.md:351-363`).
- A stale receipt left on screen can satisfy `replay`, because echo subtraction is not implemented (`:209-217`, `:383-388`).
- A delivery turn always ends with CR. Injecting into an unrecognized dialog would confirm it (`:393-396`).
- State left over from one run starts the next one paused (`:433-437`).
- Poison message: one body over 64 KiB hits `fail()` before a batch exists, which latches `durableAttention`. `reset-guard`
  clears it (`supervisor.py:573`), but the next poll peeks the same oversized row and fails again. INFERRED from `:641-645`
  together with `:483-489`: a permanent stop until someone removes or reads that message by hand.
- Only TUIs launched through `agy-tui` are covered. "Plain `agy` … is ordinary Antigravity with no agmsg delivery at all"
  (`docs/antigravity-monitor-beta.md:69-72`). `spawn.sh` does not launch through `agy-tui` (no reference in `scripts/spawn.sh`).
  VERIFIED by absence.

**Headless bridge wake.** Each batch starts a turn in the bridge-owned worker, so the worker always wakes. Turns are serialized
(`busy` flag), with a 330 s deadline (`antigravity-bridge.mjs:95-101`, `:151-153`). The restart budget is 3, with 1/5/15 s backoff
(`:121-127`). VERIFIED. The human's TUI is never woken.

## Setup and installation footprint on the harness

VERIFIED unless marked.

- **Skill.** `~/.gemini/config/skills/<cmd>/SKILL.md`, rendered from `scripts/skill-base.md` (31 KB) plus the antigravity
  overlay `template.md` (4.5 KB). It is installed only if `~/.gemini/antigravity-cli` or `~/.gemini/config` exists
  (`install.sh:831-847`, `:1462-1466`).
- **Shim.** `~/.agents/bin/agy-tui`, ownership-tagged and never clobbering a foreign file (`install.sh:790-829`).
- **Project rule file.** `<project>/.agent/rules/agmsg.md`: turn text, or the monitor marker, or absent for `off`.
  - Custom content is refused rather than overwritten when switching to monitor (`_delivery.sh:19-68`).
  - Switching away from monitor refuses while a TUI supervisor is alive (`antigravity-mode.mjs:21`).
- **Runtime state** in the agmsg install's `run/`:
  - `antigravity-tui-pty.<team>__<role>.state.json`;
  - `read-reservation.<…>.json`, its `.violations` file and the `.violations.lock`;
  - the actas lock (`supervisor.py:357-359`, `:542-547`).
- **Not written:** `~/.gemini/config/hooks.json`, plugins, MCP config, and `~/.gemini/antigravity-cli/settings.json`.
  - The docs ask the user to add `permissions.allow` entries for `send.sh`, `identities.sh` and `whoami.sh` by hand, in every
    invocation shape (`docs/antigravity-monitor-beta.md:219-281`).
  - The docs advise against allowing the inbox scripts.
- **Runtime dependencies:** bash and sqlite3 (core); python3 (the TUI supervisor); node plus flock on Linux or python3 on
  macOS for advisory locks (the headless bridge) (`antigravity-monitor.sh:13-18`, `antigravity-tui-monitor.sh:5-6`). The
  supervisor and the read guard also call node (`inbox-transport.sh:75`, `bridge-read-guard.sh:8`).

## Token cost

All INFERRED from the text sizes above; nothing was measured.

**Per session.**
- The rule file, about 1 KB (~250 tokens), if agy loads it.
- The skill's name and path in agy's skill list. The 35 KB skill body is read only when `$agmsg` is invoked (primer §3:
  skills are read on demand with `view_file`).

**`turn`, quiet turn.**
- No injected bytes. But every complying check is a full `run_command` step: model output tokens for the call, an empty
  result, and another model invocation over the whole context.
- Under `request-review` permissions each check is also a permission prompt unless allow-listed. That is the dominant cost and
  the main annoyance.
- With a 60 s cooldown, most checks return nothing.

**`turn`, per message.**
- About 30 bytes of header per team, plus `[ts] from: ` and the body, JSON-escaped and wrapped in
  `{"decision":"block","reason":…}`.
- The result persists as tool output in the conversation.

**`agy-tui`, per batch.**
- About 70 bytes of batch header and trailer, about 60 bytes of tags per message (`[agmsg message id=…]`, `from:`, `at:`,
  `body:`, `[/agmsg message]`), and a fixed instruction of about 230 bytes.
- The model's receipt line, about 52 bytes of output.
- A whole new user turn, which re-reads the full context.
- Bodies are not truncated, up to 64 KiB per batch.

**`agy-tui`, quiet period.** Zero model tokens. The supervisor polls SQLite every 2 s with no model involvement.

**Headless bridge.** Pretty much the raw batch JSON per turn, plus a fixed init prompt of about 330 bytes per worker launch.

## Limitations and bugs I can see

1. **No use of agy hooks.**
   - agy exposes `PreInvocation injectSteps` before every model call, including the calls after tool results (primer §2.2–2.5),
     and herdr-threads uses it.
   - agmsg instead relies on a model-obeyed rule (turn) or a PTY wrapper (monitor), so its in-turn delivery depends on the model
     complying. INFERRED comparison; VERIFIED that agmsg writes no hooks.
2. **Turn mode is cooperative and unverifiable.** There is no enforcement and no receipt, it costs a tool call per check, and
   the rules path (`.agent/` vs `.agents/`) is unconfirmed against current agy.
3. **Skill text misdescribes turn mode** as a Stop hook (`template.md:10-11`).
4. **The version pin is documented but not implemented** (design `:87` vs code). This happens to match herdr-threads' rule of no
   exact-version gates, but the docs overstate the safety.
5. **The only control byte escaped is ESC**, despite the docs (`supervisor.py:593` vs `docs/antigravity-monitor-beta.md:325`).
6. **A poison oversized message** permanently stops the TUI monitor (see "Failure modes").
7. **The recommended mode requires `--dangerously-skip-permissions`.** Messages from peers then drive unconfirmed tool calls
   (`docs/antigravity-monitor-beta.md:10-16`).
8. **A human keystroke during a delivery turn stops the supervisor** and needs manual `ack`/`replay`. The docs say it is "not
   what this driver is for" on shared seats (`:407-417`).
9. **Receipt matching scans the whole screen**, so a stale receipt can produce a false ack on replay (`:209-217`).
10. **The pause hint goes to stderr and is overdrawn by agy's redraws** (`:438-443`).
11. **`poke` has no draft check for agy panes** (no `input_prompt_marker`).
12. **No priority or threshold.** Every unread row wakes a monitored seat. There is no silent delivery tier.
13. **Linux and macOS only.** The live check for release 1.3.1 covered macOS only (`CHANGELOG.md:205-209`).

## Clever ideas worth stealing

1. **Unforgeable model receipt.** The envelope asks the model to print `AGMSG_RECEIVED:<uuid4 batch id>` and states only the
   construction rule, never the literal. A body that contains the receipt blocks the ack. herdr-threads confirms agy delivery by
   finding `SYSTEM_SDK` steps in the transcript, which proves the step was injected, not that the model attended to it. An
   optional receipt token, checked in the transcript rather than on screen, would prove the model read it. It costs about 50
   output bytes, so it fits ACK-requested items only (token-efficiency rule (f)). INFERRED applicability.
2. **Measured negative result.** On agy 1.1.27, a headless or stream-json turn on the TUI's conversation id does not reach the
   live TUI's in-memory context (`antigravity-conversation-registration.md:41-44`).
   - This is strong evidence against any "second process on the same conversation" wake.
   - It is a cautionary data point for the primer's top lead, `agentapi send-message`, which should be tested for exactly this
     before anyone relies on it.
   - The `kiwi`/`mango` memory probe is a cheap, reusable test design for that.
3. **Bottom-anchored idle signature.** The gate requires an empty `>` row, rules allowed, directly above a `? for shortcuts … ·`
   footer, plus 0.3 s of output quiet and a zero-timeout `select` on both input sources just before the write.
   - On our 1.3.2 captures it rejects the live-subagent screen that Herdr reports as idle. INFERRED.
   - Worth cross-checking `AgyComposer` against: "the last row above the footer must be `>`" is a cheap extra discriminator.
4. **Human-typing pause with hysteresis.** Delivery resumes only after a non-idle screen *and then* a stable idle period, so an
   Esc that leaves the old idle screen in place does not resume. Useful if herdr-threads ever adds a supervisor-side wake. Herdr
   owns the PTY today.
5. **Single-writer read guard with a capability on fd 3.** It detects and latches any second consumer that tries to mark read.
   In the same family, the bridge scans the stream-json tool events for the model calling `inbox.sh`. The analogue for us is
   detecting a model that calls `herdr-threads inbox` redundantly after in-turn delivery, as a diagnostic. INFERRED.
6. **Body-imitates-chrome latch.** If a delivered body contains the idle-screen signature, auto-delivery stops until an explicit
   resume. Our screen parser has the same exposure whenever peer text is echoed into the pane above the composer. Worth a
   fixture. INFERRED.
7. **Status that never prints bodies** (`agy-tui status`: ids, from and time only) is "safe to paste into a bug report"
   (`docs/antigravity-monitor-beta.md:106-108`). A good norm for diagnostics.

## Verification

Adversarial pass on 2026-10-23 against the clone at `6e0315468db3` and herdr-threads' `tests/fixtures/agy/composer/`. No clone code was run.

- Checked and confirmed: `type.conf`; the rule-file text and the monitor marker (`delivery-rulefile.sh`, `_delivery.sh`); in
  `check-inbox.sh`, the output format, `stop_output` gating (only codex and copilot set it), the cooldown marker and interval, the
  absence of any limit, and the order of write before mark-read; the `--limit 20` peek; every cited supervisor range (launch,
  envelope, ESC-only escape, paste+CR write, 64 KiB `fail`, receipt matching, failure signature, ack, `input_ready` and
  `injection_ready`, human-input hysteresis at 0.6 s, the permission/trust exception, recovery, the 2 s poll default, and no
  `agy --version` check); the read guard (`bridge-read-guard.sh`/`.mjs`, exit 13); the bridge (args, init prompt,
  `forbiddenTool`, the 330 s deadline, the 3-restart 1/5/15 s backoff, the raw JSON payload); that `poke` has no marker for agy
  and goes through herdr `agent prompt`; the install footprint (skill gate, shim, no hooks.json); the cited beta and design doc
  lines; and the four 1.3.2 composer fixtures against the gate.
- Changed: added the type plug step (`_delivery.sh:12-15`) in front of `rulefile_apply`; noted that `EVENT` defaults to `Stop`;
  replaced the turn-mode test citation, which pointed at gemini tests, with the antigravity tests; corrected the CHANGELOG range
  and one doc line; marked the kiwi/mango wording as a translation rather than a quote; corrected the runtime dependencies
  (flock is Linux-only, and node is used outside the bridge too).
- Not checked: whether agy 1.3.x loads `.agent/rules/`, which remains unknown.
