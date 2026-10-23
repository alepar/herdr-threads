# agmsg x Pi coding agent (pi-mono)

- Alternative: agmsg, <https://github.com/fujibee/agmsg>, shallow clone HEAD `6e031546` (2026-10-23, release 1.6.0).
  Paths without a prefix are relative to the agmsg clone root.
- Harness: Pi, <https://github.com/badlogic/pi-mono> clone `a7b94a03` (coding-agent 1.4.0). Paths prefixed
  `pi:` are relative to the pi-mono clone root. Harness facts not re-verified here come from
  `../../primers/pi.md` (cited as "primer §N").
- No code from either clone was executed. VERIFIED = read in source or docs at the cited line. INFERRED =
  my reading. "Unknown" = not established.

## Support status

**Not supported.** agmsg has no Pi type driver, no Pi-specific code and no mention of Pi.

- The type registry is the directory list `scripts/drivers/types/`: `agmsg-app antigravity claude-code
  codex copilot cursor devin ext-tool gemini grok-build hermes opencode`. No `pi`. VERIFIED (directory
  listing). `docs/agent-types.md:3-4` lists the same set.
- A case-insensitive search for `pi-mono`, `pi-coding`, `badlogic`, `earendil`, `PI_CODING_AGENT`,
  `AI_AGENT`, `.pi/` and the bare word `pi` returns nothing relevant. VERIFIED (grep over the clone).
- `join.sh` refuses any type that is not registered:
  ```bash
  if ! agmsg_is_known_type "$AGENT_TYPE"; then
    echo "Unknown agent type: '$AGENT_TYPE' (supported: ...)" >&2
    exit 1
  fi
  ```
  VERIFIED `scripts/join.sh:16-23`. A Pi seat therefore cannot join as `pi`. It can only join under some
  other type's name.
- The desktop app's agent-agnostic PTY injection only covers types it can spawn, and it spawns only
  manifests with `spawnable=yes` and a `cli=`. VERIFIED `app/src-tauri/src/agmsg.rs:604-620`. Pi has no
  manifest, so the app cannot launch it.
- No external Pi plugin is referenced. A third-party type could be dropped in under `plugins/types/<name>/`
  after `agmsg plugin trust`. VERIFIED `plugins/README.md:1-29`, `docs/agent-types.md:75-91`. None exists
  for Pi as far as this repo shows (unknown outside it).

The rest of this report covers what actually happens when a Pi agent uses agmsg through its generic
surface, and how well that works.

### The accidental path: Pi auto-loads agmsg's shared skill, typed as Codex

This is the main Pi-specific finding. A standard agmsg install puts the shared skill at
`~/.agents/skills/<cmd>/SKILL.md` (default `<cmd>` = `agmsg`). VERIFIED `install.sh:57-77`. Pi
auto-discovers exactly that location:

- "Pi also supports the Agent Skills locations `~/.agents/skills/` and `.agents/skills/`." VERIFIED
  `pi:packages/coding-agent/docs/skills.md:61`. The code is
  `userAgentsSkillsDir = join(getHomeDir(), ".agents", "skills")`, which is always scanned. VERIFIED
  `pi:packages/coding-agent/src/core/package-manager.ts:2484,2575-2587`.
- The discovery walk stops descending at the first directory that holds a `SKILL.md`. VERIFIED
  `pi:.../package-manager.ts:402-421`, where `return entries` follows the first `SKILL.md` hit. Pi
  therefore registers the single `agmsg` skill and does not pick up the many `*.md` files deeper in the
  install tree. Because the walk returns early, the agents-mode rule that turns nested `.md` files into
  skills (`:443-450`) never reaches them. VERIFIED by reading the code. INFERRED that no stray skills
  appear in practice.
- Pi puts only the skill's name, description and path into the system prompt, and the model reads the full
  `SKILL.md` when the task matches or when the user types `/skill:agmsg`. VERIFIED
  `pi:.../docs/skills.md:43-47`. The agmsg frontmatter is valid (`name: __SKILL_NAME__`, with a
  description). VERIFIED `scripts/skill-base.md:1-4`.

The shared file is rendered for one type only. Unless the user re-renders it with `--agent-type`, that
type is **Codex**: "it is what any type NOT in this list reads as its ONLY instructions (today: codex,
gemini, cursor, devin ...)", and the detection fallback is `_agmsg_detected_type="codex"`. VERIFIED
`install.sh:67-77,1011-1019`. Every command in the rendered file carries the type: the base text uses
an `__AGENT_TYPE__` placeholder that rendering fills with `codex` (`scripts/skill-base.md:33,58,79`),
and the Codex overlay also spells it out literally in its PowerShell variant
(`whoami.sh "$(pwd)" codex`, `scripts/drivers/types/codex/template.md:6`). VERIFIED.

Consequence (INFERRED from the above): a Pi model that follows the skill registers itself as **type
`codex`**. It then gets Codex's delivery menu (agmsgd / monitor / turn / off,
`scripts/drivers/types/codex/template.md:13-25`), Codex's actas step (`codex-record-session.sh`,
`:31`) and Codex's input-box marker for pokes. None of these fit Pi. The sections below follow that path.

## Injection mechanism

agmsg has no injection into Pi. The only route by which a message body reaches a Pi model is **the model
running an agmsg script through Pi's `bash` tool**. The script's stdout becomes the tool result.

| Path | Exact surface | Fires when | Verdict on Pi |
|---|---|---|---|
| Manual inbox | `~/.agents/skills/agmsg/scripts/inbox.sh <team> <agent>` via Pi's `bash` tool | Only when the model or user decides to check (`/skill:agmsg` → the skill's "check inbox" default) | Works. VERIFIED script, INFERRED fit |
| Cooperative self-poll | `check-inbox.sh <type> <project>` after each tool call, as in the opencode/antigravity rule files | Only if the model obeys a rule text | Not installed for Pi: rule files go to `.opencode/rules/`, `.agent/rules/` and similar, which Pi does not read. Pi reads `AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md` (VERIFIED `pi:.../src/core/resource-loader.ts:212`). A user could paste the rule into `AGENTS.md` by hand (INFERRED) |
| Stop hook `decision:block` / PostToolUse `additionalContext` | `.codex/hooks.json`, written by `delivery turn` for the Codex type | Never on Pi | Pi has no subprocess hooks and ignores `.codex/hooks.json` (primer §1, §2.2). INFERRED: `turn` silently does nothing |
| Monitor stream | `watch.sh` under Claude's Monitor tool or opencode-sentinel | Never on Pi | Pi has no resident-stream tool. A `watch.sh` run in `bash` would block that tool call (see Wake) |

### Payload format (manual inbox)

```bash
echo "$COUNT new message(s):"
echo ""
...
  echo "  [$ts] $from: $body"
```

VERIFIED `scripts/inbox.sh:86-95`. One header line, then one line per message. Newlines and tabs inside the
body are escaped to `\n` / `\t` (`:75`). "No new messages." is printed when there is nothing (`:37,49`).
There is no team or thread name and no message id in the display.

### Bounds

- agmsg: none. The full unread backlog is printed. The temp-file construction exists precisely because
  backlogs exceed 128 KiB of argv (`scripts/inbox.sh:61-82`). VERIFIED.
- Pi: the `bash` tool result is **tail-truncated** to the last 2000 lines or 50 KB, and the full output is
  saved to a temp file. VERIFIED `pi:.../src/core/tools/bash.ts:255`,
  `pi:.../src/core/tools/truncate.ts:11-12`, `pi:.../src/core/tools/output-accumulator.ts:92`
  (`truncateTail`).
- **Interaction bug (INFERRED from the two VERIFIED facts):** `inbox.sh` marks *every displayed id* read
  (`:120-124`). When the backlog exceeds 50 KB, Pi keeps only the tail, so the **oldest** messages are
  marked read while the model sees only a "truncated" notice and a temp-file path. Recovery depends on the
  model choosing to `read` the temp file. agmsg's at-least-once rule ("a duplicate is visible and a silence
  is not", `scripts/check-inbox.sh:500-507`) does not hold here, because a successful stdout write is not
  model receipt. agmsg's own comments concede that point (`:478-493`).

### Framing, dedupe, ack

- Framing: the plain lines above. There is no marker telling the model it is peer input rather than tool
  output. The skill text (loaded once) carries the explanation. VERIFIED `scripts/skill-base.md`
  (whole file).
- Dedupe: read state is a per-(team, agent) projection over an append-only event log, and display marks
  read by explicit id, so a message arriving between SELECT and mark stays unread. VERIFIED
  `scripts/inbox.sh:109-124`, overview "Storage".
- Ack: no peer ACK. Read-on-display is the only signal. VERIFIED (overview "Delivery guarantees").

### Side effects of each `inbox.sh` / `send.sh` on Pi

Each action runs `agmsg_self_name_on_action`, which names the seat's own terminal pane, and
`agmsg_self_rename_on_action`. VERIFIED `scripts/inbox.sh:19-27`, `scripts/send.sh:71-77`,
`scripts/lib/self-name.sh:1-20`. Under Herdr the pane comes from the inherited `HERDR_PANE_ID`. VERIFIED
`scripts/drivers/terminals/herdr/ops.sh:253-281`. INFERRED: Pi's `bash` tool inherits Pi's environment,
so this works from Pi and creates the placement record that `poke` needs (below). The self-rename step
types the type's `rename_cmd` plus the seat name into the seat's own pane through `agmsg_safe_poke`,
using the type's input marker. For Codex that is `/rename <name>` with the `›` marker. VERIFIED
`scripts/lib/self-rename.sh:138-139,300-304`, `scripts/drivers/types/codex/type.conf:74`,
`scripts/send.sh:78`. INFERRED: on Pi the marker check fails to locate the box (as for `poke`, below),
so the self-poke is refused and nothing is typed. If it were typed, Pi would receive a literal
`/rename ...`. Pi's equivalent is `/name` (VERIFIED `pi:.../src/core/slash-commands.ts:30`), so
`/rename` would most likely reach the model as text (INFERRED).

## Wake mechanism

**There is no wake for Pi in agmsg.** An idle Pi waiting at its prompt never learns of a new message from
agmsg on its own. Per mechanism:

| agmsg wake path | Why it does not reach Pi | Evidence |
|---|---|---|
| Claude Monitor event | Pi has no Monitor tool | primer §2 (no such tool) |
| agmsgd, Claude channel | Needs `CLAUDE_CODE_MESSAGING_SOCKET`/`_TOKEN` published at Claude's SessionStart | VERIFIED `scripts/drivers/types/claude-code/_session-start.sh:10-31` |
| agmsgd, Codex channel (`codex queue --thread <id>`) | Needs a recorded Codex thread. In Pi, `CODEX_THREAD_ID` is unset, so `codex-record-session.sh` falls back to inference (app-server probe, then the cwd-unique rollout scan) | VERIFIED `scripts/drivers/types/codex/codex-record-session.sh:24-38,81-83` |
| Codex app-server bridge (`turn/start`) | Requires the TUI to be `codex --remote` against agmsg's app-server | VERIFIED `docs/codex-monitor-beta.md:14-26` |
| Antigravity PTY supervisor | agy-only, with screen signatures measured on agy | VERIFIED `docs/antigravity-monitor-beta.md:1-55` |
| Desktop app PTY inject | Only for app-spawned types with a manifest | VERIFIED `app/src-tauri/src/agmsg.rs:604-620` |
| **`poke`** (manual, peer-invoked) | **Partially works**, see below | VERIFIED `scripts/poke.sh` |

### `poke` into a Pi pane (the only wake that can reach Pi)

A teammate (or a human) runs `poke.sh <team> <pi-seat> --body-file f`. Steps:

1. Placement record lookup. It fails with "no placement record" unless the seat was spawned or has
   self-named its pane. VERIFIED `scripts/poke.sh:125-126`. On Pi the record exists after the Pi seat's
   first `inbox.sh` or `send.sh` (self-name, above). INFERRED.
2. The type comes from the roster, not the record (`:131-142`), so a misregistered Pi seat is treated as
   `codex`.
3. Draft guard: `INPUT_MARKER="$(agmsg_type_get "$TYPE" input_prompt_marker)"` (`:177`), passed to
   `agmsg_safe_poke` (`:182`). For codex the marker is `›`, and a codex-specific footer witness is
   required: "the marker line, a blank line, then a status footer line containing `·`". VERIFIED
   `scripts/drivers/types/codex/type.conf:151-160`. Pi's editor is not Codex's widget. INFERRED: the
   locator fails, so the poke is **refused with exit 15** ("input box could not be located", `:190-193`).
   It fails closed. That is safe for drafts, but the wake never happens. If the seat were instead
   registered as a type with no `input_prompt_marker` (e.g. `devin`, `hermes`, `gemini`; only claude-code
   and codex set one, VERIFIED grep of `type.conf`), the check would be **skipped entirely** and the text
   typed unconditionally (`:171-176` comment). That is no draft safety at all.
4. Submission under Herdr: `herdr agent prompt <pane> <text>`, which "submits on its own". VERIFIED
   `scripts/drivers/terminals/herdr/ops.sh:1084-1102`. Whether Herdr's `agent prompt` accepts a pane whose
   agent kind is `pi` is unknown. The primer infers that Herdr detects Pi panes (primer §7). Under tmux it
   is `send-keys -l`, then arrow + Enter in a separate burst (`scripts/poke.sh:26-31`).

What the keystrokes do inside Pi (primer §6, VERIFIED there):

- Idle: the typed text is submitted as a user prompt, so it is a real wake. The poke body is whatever the
  sender wrote. The skill teaches content-bearing pokes (`scripts/skill-base.md:180-196`, per overview),
  so this is not content-free.
- Busy: Enter while streaming becomes a **steer** into the current run
  (`pi:.../src/modes/interactive/interactive-mode.ts:3333-3341`). The poke is not lost. It lands mid-task
  after the current tool batch. agmsg does no busy check beyond the input-box snapshot (INFERRED from
  `safe-poke.sh`; it compares box contents, not agent state).
- A non-empty draft: the text is inserted at the cursor and submitted together with the draft (primer
  §6, INFERRED there).
- Text starting with `/agmsg` (the Claude Code invocation form): Pi tries extension commands, then
  `/skill:` expansion, then prompt templates (VERIFIED `pi:.../src/core/agent-session.ts:2101-2133`).
  `/agmsg` matches none of these, so it reaches the model as literal text. INFERRED. The Pi form is
  `/skill:agmsg`. The desktop app's kickoff notice (`[agmsg] <from>: "<preview>" — run /<cmd> to check
  it.`, `app/README.md:37-42`) never reaches Pi, because the app only spawns manifest types.

Latency: manual, so human or peer-model latency. agmsg has no timer. Failure modes: exit 15 under the
Codex misregistration; no wake at all for any seat nobody pokes; a typed wake into a busy Pi steers the
current task.

### A blocking long-poll from inside a turn (generic, not taught for Pi)

`scripts/drivers/types/codex/watch-once.sh` is a one-shot oracle. It waits until unread mail exists, exits
0 (mail), 2 (timeout, default 300 s) or 1, and never marks read. VERIFIED `watch-once.sh:6-18,46`. Pi's
`bash` tool has **no default timeout** (`bash.ts:42`) and accepts up to about 24.8 days
(`bash.ts:22-25`). INFERRED: a Pi model told to "run watch-once.sh, then inbox.sh, loop" would get
in-session waiting while the turn is open, at the price of a permanently busy session. Escape aborts it. It
is not a wake from idle, and agmsg does not offer it for Pi. It lives in the Codex type directory as
the app-server bridge's oracle (optional `--owner`, `AGMSG_CODEX_OWNER_ID`, `:9,45`), so it is not a
supported general interface.

## Setup / installation footprint on the harness

What an agmsg install writes that affects Pi (VERIFIED paths, INFERRED Pi effect):

- `~/.agents/skills/agmsg/` (skill dir, scripts, `db/messages.db`, `teams/`, `run/`). Pi auto-loads
  `SKILL.md` from it (above). This is the only Pi-visible artifact, and Pi never writes it itself.
- Nothing under `~/.pi/agent/` or `.pi/`. agmsg installs no Pi extension, MCP config, settings change or
  AGENTS.md edit. VERIFIED by absence (grep).
- Choosing `turn` for the Codex-typed Pi seat writes `.codex/hooks.json` in the project
  (`scripts/drivers/types/codex/type.conf:122`), which Pi ignores. Choosing `agmsgd` registers a login
  service (`scripts/drivers/types/codex/template.md:15`) that will never reach Pi. Both leave dead
  configuration behind. INFERRED.
- Risk (INFERRED from `codex-record-session.sh:24-38,108-130`): when the Pi seat runs the Codex actas step
  in a project where a real Codex session's rollout is the unique cwd match, the inference could seat
  *that Codex thread* under the Pi seat's name. agmsgd would then `codex queue` the Pi seat's notices into
  someone else's Codex conversation. The script's own quality guard ("biased toward recording NOTHING when
  unsure") makes this unlikely but not impossible. Unknown in practice.

## Token cost

- Per session, quiet: the Pi system prompt carries the skill's name, description (about 230 bytes,
  `scripts/skill-base.md:3`) and path. VERIFIED mechanism `pi:.../docs/skills.md:43`.
- On first use: the model reads the rendered shared `SKILL.md`. Estimated from `scripts/skill-base.md`
  (30,951 bytes) plus the Codex overlay slots (`template.md`, 7,200 bytes, partly replacing empty slots).
  INFERRED: the rendered file is about 30-38 KB, roughly 8-10k tokens. Pi's `read` tool head-truncates at
  2000 lines / 50 KB, so it fits.
- Per message: the inbox header (about 20 bytes) plus `  [<ts>] <from>: <body>` per message, plus the
  `bash` tool call and result framing. No per-message boilerplate. VERIFIED format.
- Per quiet turn: **zero**, because nothing runs. If a user hand-installs the cooperative
  "check-inbox after each tool call" rule into `AGENTS.md`, every tool call costs one extra model-issued
  `bash` call. The 60 s cooldown (`scripts/check-inbox.sh:177-198`) limits it to an empty result, but each
  call is still a full tool round trip with its tokens and latency. INFERRED.

## Limitations and bugs visible

1. No Pi support. The type name `pi` is rejected (`join.sh:20-23`).
2. Silent misidentification: Pi reads the Codex-typed shared skill and registers as `codex`, with no
   warning. agmsg's detection list falls back to `claude-code` when `whoami.sh` gets no type
   (`docs/agent-types.md:57`), and the Codex skill always passes `codex`. No manifest detects Pi's own
   markers `AI_AGENT=pi` / `PI_CODING_AGENT=true` (primer §5; grep of `detect=` lines finds none).
3. Delivery choices offered to the misregistered seat (`agmsgd`, `monitor`, `turn`) all silently fail on
   Pi. Only `off` (manual) is honest.
4. Body loss under Pi's tail truncation (above): agmsg's mark-on-write meets a harness that keeps only the
   tail.
5. No idle wake. `poke` fails closed (exit 15) on Pi under the Codex marker, or types blindly under a
   markerless type.
6. Session identity: agmsg reads `AGMSG_SESSION_ID` / `CLAUDE_CODE_SESSION_ID` / `CODEX_THREAD_ID`
   (`scripts/poke.sh:242`) and walks the process tree for the type's binary
   (`scripts/lib/resolve-project.sh:437-452,487-534`). Pi's `PI_SESSION_ID` (exported to its `bash` tool,
   primer §5) is not used, and a `node`/`pi` process never matches `codex*`. INFERRED: the actas lock
   falls back to the bare-session-id path, so two Pi panes may not be told apart.

## Clever ideas worth stealing (for a herdr-threads Pi adapter)

- **Fail-closed draft guard with a distinct code.** The poke separates "box located and someone is
  typing" (14) from "cannot locate the box" (15), retries only those two (never a driver failure), and
  refuses rather than guessing. VERIFIED `scripts/poke.sh:69-79,185-193`,
  `scripts/lib/safe-poke.sh:33-43,449,502`. For Pi we can do better
  in-process with `ctx.ui.getEditorText()` and `onTerminalInput` (primer §2.4). Keep the 14/15 distinction
  in our diagnostics.
- **Self-naming on action.** Every send or read repairs the seat's pane mapping from its own environment,
  with a cheap mark check. VERIFIED `scripts/lib/self-name.sh:1-54`. This fits a Pi CLI path where the
  `bash` tool carries `PI_SESSION_ID` plus `HERDR_PANE_ID`.
- **Mark read only after the write succeeds, and say so.** The comments state precisely what a successful
  write does and does not prove (`scripts/check-inbox.sh:474-507`). The Pi truncation case shows the
  limit: a Pi extension delivering through `pi.sendMessage` and confirming through the persisted
  `CustomMessageEntry` in the session JSONL (primer §2.3) would give real receipt evidence, the Pi
  analogue of agmsgd's transcript-nonce check.
- **Shared `~/.agents/skills/` as a zero-config skill channel.** Pi, Codex and others read it. It is a
  cheap way to ship our skill to Pi. The lesson from agmsg is to render it **harness-neutral**: no
  hard-coded type in commands, and detection from the harness's own markers (`AI_AGENT=pi`). Otherwise
  every non-Codex reader inherits Codex identity.
- **Type driver as data plus an optional delivery plug** (`docs/agent-types.md:8-38,64-73`). A Pi plug
  could write a `.pi/extensions/` file, or launch with `pi -e`, which needs no config write (primer §2.1).
  agmsg never did this, but its model would allow it. This matches our "new harness = adapter, neutral
  default" rule.

## Bottom line for Pi

agmsg reaches Pi only by accident. Pi picks up the shared Codex-typed skill from `~/.agents/skills/`, and
the model then pulls its inbox by hand through `bash`. There is no in-turn delivery, no idle wake, a
misregistered identity, dead delivery options, and a body-loss edge from Pi's 50 KB tail truncation.
agmsg's generic mechanisms are pull-by-shell and type-into-the-pane, and both are weak fits for Pi. Pi's
strength is in-process: `sendMessage` with `triggerTurn`, the `isIdle()` and editor-text checks, and
`before_agent_start` / `tool_result` injection (primer §2.3-2.4, §9). agmsg has no component that uses
any of it.

## Verification

Adversarial pass, 2026-10-23, against agmsg `6e03154` (latest release `v1.6.0` per `gh api`) and pi-mono
`a7b94a0` (coding-agent `package.json` version 1.4.0). No code was executed.

Re-read and held: the type list and the `join.sh` rejection; the app's `spawnable=yes` + `cli=` filter;
the plugin trust flow; the install comment about the shared skill and the `codex` fallback; the skill
frontmatter (description is about 230 bytes); the `inbox.sh` format, escaping, argv/temp-file rationale and
mark-by-displayed-id; the `check-inbox.sh` cooldown and write-then-mark comments; Codex `type.conf`
(`hooks_file`, marker `›` with footer witness); `input_prompt_marker` set only for claude-code and codex;
`poke.sh` record lookup, roster-type override and exit 14/15; the Herdr `HERDR_PANE_ID` self-id and
`agent prompt` poke; the `codex-record-session.sh` resolution order; `CALLER_SID` env list; the
claude-code detection fallback and absence of any Pi `detect=`; Pi's `~/.agents/skills` scan and
early return at the first `SKILL.md`; the context-file list; the bash tool's 2000-line/50 KB tail
truncation, no default timeout and ~24.8-day cap; streaming Enter as steer; command expansion order;
`AI_AGENT=pi` / `PI_CODING_AGENT` / `PI_SESSION_ID`.

Changed:
- "hard-codes the type in every command" was overstated: commands use the `__AGENT_TYPE__` placeholder
  filled at render; only the PowerShell variant spells out `codex`. Dropped the irrelevant `install.sh:81`.
- Self-rename: replaced "not traced further" with the traced mechanism (types `/rename <name>` through
  `agmsg_safe_poke`) and Pi's `/name` equivalent.
- `poke` retries cover both 14 and 15, not only 14. Fixed, with citations.
- The desktop app's kickoff text begins with `[agmsg]`, not `/agmsg`. Corrected the bullet.
- Citation fixes: `interactive-mode.ts:3333-3341`, `watch-once.sh:6-18,46`, `app/README.md:37-42`.
  The `watch-once.sh` `--owner` flag is optional, now described as such.
