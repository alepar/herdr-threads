# agmsg x Goose (Block / AAIF): message injection and wake

- Alternative: agmsg, <https://github.com/fujibee/agmsg>, shallow clone HEAD `6e0315468db3` (2026-10-23, release 1.6.0). Paths marked `agmsg:` are relative to that clone root. No agmsg code was executed.
- Harness: Goose. Paths marked `goose@1.36:` or `goose@main:` are relative to the Goose clones described in `../../primers/goose.md` (v1.36.0 `f13f369a`, matching the installed binary, and main `c3bfc5f7`, 1.56.0).
- Repo stats (from `overview.md`, gh api 2026-10-23): 1,564 stars, latest release v1.6.0 (2026-10-22).
- Tags: **VERIFIED** means read in source or docs at the cited place. **INFERRED** means my reading. "unknown" means not determined.

## 1. Support status

**agmsg does not support Goose.** VERIFIED:

- No type driver exists. `scripts/drivers/types/` contains `agmsg-app antigravity claude-code codex copilot cursor devin ext-tool gemini grok-build hermes opencode` and nothing else (directory listing, agmsg).
- The word does not appear anywhere. `git grep -niIw -e goose -e goosed` over every text file in the clone (svg/png excluded, because their base64 produces false matches) returns nothing. `git grep` for `GOOSE_`, `MOIM`, `.agents/plugins` and `aaif` also returns nothing. agmsg uses none of Goose's injection surfaces (hooks, Top-Of-Mind file, ACP).
- Unregistered types are refused when a seat joins. A Goose session cannot join as `goose`. VERIFIED `agmsg: scripts/join.sh:16-23`:

  ```bash
  if ! agmsg_is_known_type "$AGENT_TYPE"; then
    echo "Unknown agent type: '$AGENT_TYPE' (supported: ...)" >&2
    exit 1
  fi
  ```

  `spawn.sh:133` and `doctor.sh:471` apply the same gate; `whoami.sh:43` applies it only when a type argument is passed. The only extension point is an external type driver under `<root>/plugins` or `$AGMSG_PLUGIN_DIRS`, which loads only after `agmsg plugin trust types/<name>` (VERIFIED `agmsg: scripts/lib/type-registry.sh:14-17,45`). None exists for Goose. `plugins/` holds only a README.
- agmsg cannot be installed as a Goose plugin. The repo ships `.claude-plugin/plugin.json` (`agmsg: .claude-plugin/plugin.json:1-21`). Goose's Open Plugins loader looks for `.goose-plugin/plugin.json`, `.plugin/plugin.json` or `plugin.json`, plus `hooks/hooks.json` / `commands` / `agents` components (VERIFIED `goose@1.36: crates/goose/src/plugins/formats/open_plugins.rs:14-21`). agmsg has none of these. INFERRED: `goose plugin install https://github.com/fujibee/agmsg` would not recognise it.

Everything below therefore describes what happens when a user points agmsg's generic pieces (bash scripts, shared skill, `poke`, desktop app) at a Goose session, plus what a Goose type driver could do. It is not a description of shipped Goose support.

### 1.1 Accidental exposure: Goose reads agmsg's shared skill

This matters in practice and was not in the mapper's note.

- agmsg installs its runtime and a **shared** `SKILL.md` at `~/.agents/skills/<cmd>/` (VERIFIED `agmsg: install.sh:5,22,1301`). Types without their own skill file read that shared file. It is **Codex-typed**: "The shared ~/.agents/skills/<name>/SKILL.md is Codex-typed (whoami ... codex) and would mis-identify a Copilot session" (VERIFIED `agmsg: install.sh:1413-1415`, also `:67-74`, `:1425-1427`, `:1448-1450`).
- Goose reads `~/.agents/skills` as a global skill root. VERIFIED `goose@1.36: crates/goose/src/skills/mod.rs:291-307`:

  ```rust
  if let Some(h) = home.as_ref() {
      dirs.push((h.join(".agents").join("skills"), true));
  }
  ```

  The Skills platform extension is `default_enabled: true` (VERIFIED `goose@1.36: crates/goose/src/agents/platform_extensions/mod.rs:195-198`).
- Result (INFERRED): on any machine with agmsg installed, every Goose session lists the `<cmd>` skill (default name `agmsg`, `agmsg: install.sh:1294`; frontmatter `name: __SKILL_NAME__`, `scripts/skill-base.md:2`). If the model loads it (`load_skill`, `goose@1.36: crates/goose/src/skills/client.rs:60-70`), it receives instructions written for Codex: `whoami.sh "$(pwd)" codex`, join as `codex`, delivery choices `agmsgd` / `monitor (legacy)` / `turn` / `off` (VERIFIED `agmsg: scripts/drivers/types/codex/template.md:6,11-26`). A Goose session following them would **register as a Codex seat**. `agmsg delivery turn --type codex` would then write `.codex/hooks.json`, which Goose never reads. `monitor` / `agmsgd` would target a Codex thread that does not exist. The seat would appear in the roster as `codex`, and peers would `poke` it using Codex's input-box marker. This misidentification hazard is the same one that #1449 and the Copilot/OpenCode/Grok comments describe for other harnesses. Goose is not on the list of types that receive their own skill file (`AGMSG_TYPES_WITH_OWN_SKILL_FILE`, `agmsg: install.sh:81`).

## 2. Injection mechanism

agmsg has no Goose injection path (VERIFIED, section 1). The generic paths available to a Goose session are listed below.

### 2.1 Manual pull through the shell tool (works today)

- Mechanism: the model runs `~/.agents/skills/<cmd>/scripts/inbox.sh <team> <agent>` with Goose's developer `shell` tool. The output is a normal tool result. VERIFIED: `inbox.sh` "Shows unread messages and marks them as read" (`agmsg: scripts/inbox.sh:3-6`). It marks only the displayed ids after printing them (`:109-121`).
- When it fires: only when the model decides to call it, either because the user asks or because a skill or rule tells it to. Nothing in Goose triggers it automatically.
- Framing: plain rows from the storage driver. Bounds: none found. The overview states there is no per-message or per-batch byte bound in the agmsg read paths (`overview.md`, injection table). INFERRED: Goose's own shell-output handling is the only limit. Its size is unknown, not traced.
- Dedupe/ack: read-on-display through `message_read` events and a per-(team, agent) cursor. There is no peer ACK request (VERIFIED `overview.md` "Delivery guarantees").
- Identity caveat: `inbox.sh` takes `<team> <agent>` explicitly, so it works with any type string. `check-inbox.sh` and `whoami.sh` resolve identity from `(project, type)` (VERIFIED `agmsg: scripts/check-inbox.sh:129-163`). A Goose session therefore has to use a registered type name, which in practice means a borrowed one, and that brings the section 1.1 hazards.

### 2.2 `check-inbox.sh` as a Goose Stop hook (not shipped; would half-work)

agmsg's Stop-hook payload happens to be **wire-compatible with the Stop hook on Goose main** (INFERRED from the two sources, not run):

- agmsg emits `{"decision":"block","reason":"<N new message(s) in team:\n  [ts] from: body ...>"}` on exit 0. VERIFIED `agmsg: scripts/check-inbox.sh:372-380,440-462`. It reads `session_id` from stdin JSON (`:86-95`), which the Goose hook payload carries (primer section 3.3).
- Goose main's `classify_output` treats `decision == "block"` as `PolicyDeny{reason}` (VERIFIED `goose@main: crates/goose/src/hooks/mod.rs:897-950`). The Stop operation then injects it as an agent-only user message and continues the turn. VERIFIED `goose@main: crates/goose/src/agents/state_machine/ops_stop_hook.rs:19-27,95-117`:

  ```rust
  "Stop hook `{plugin}` blocked ending this turn:\n\n{reason}\n\n\
   Address this policy hook denial before trying to stop again."
  ```

  Blocks are capped at 8 per turn (`DEFAULT_STOP_HOOK_BLOCK_CAP`, `goose@main: crates/goose/src/agents/agent.rs:56`, override `GOOSE_STOP_HOOK_BLOCK_CAP`; the count is of denied messages since the turn's kickoff, `ops_stop_hook.rs:102-107`, warning text `:37-45`). Goose sends no `stop_hook_active`, so agmsg's loop guard (`check-inbox.sh:76-79`) never triggers. INFERRED: no loop occurs anyway, because rows are marked read after the emit and the 60 s cooldown marker (`:165-198`) returns "skipped" on the immediate re-Stop. For a borrowed type without `stop_output=json` (e.g. `claude-code`), quiet and cooldown turns print nothing (`:30-53`), which Goose main reads as Allow (`hooks/mod.rs:939-942`). The type the shared skill steers toward, `codex`, sets `stop_output=json` (VERIFIED `agmsg: scripts/drivers/types/codex/type.conf:124`), so quiet and cooldown turns print `{"continue":true,"systemMessage":...}` (`check-inbox.sh:49-53`). Goose main has no `decision` in that object and classifies it as `HookFailure` (VERIFIED `hooks/mod.rs:943-950`). INFERRED: Stop is fail-open (primer section 3.4), so the turn still ends, but every quiet turn records a hook failure.
- What the model sees: agmsg's batch, framed as a "policy hook denial" that it must "address". That framing is wrong for peer mail (INFERRED: it invites the model to treat a message as a compliance demand).
- **Goose 1.36 (installed) loses mail on this path.** Stop in 1.36 is a fire-and-forget `emit` whose output is discarded (VERIFIED primer section 3.2, `goose@1.36: crates/goose/src/agents/agent.rs:2286`). agmsg marks rows read as soon as the `printf` to stdout succeeds (VERIFIED `agmsg: scripts/check-inbox.sh:501-511`). The code itself admits that this proves only that the write succeeded, "not that the model received it" (`:489-495`). Wiring this hook on 1.36 would mark every delivered message read with zero model visibility: silent loss. agmsg has no transcript or receipt check for this path, so it would not notice.
- Footprint, if someone wired it by hand: `<project>/.agents/plugins/agmsg/{plugin.json,hooks/hooks.json}` with a `Stop` entry `check-inbox.sh <borrowed-type> <project>`. agmsg ships no such file.

### 2.3 PostToolUse / `additionalContext` (would not work)

agmsg's mid-turn path emits `hookSpecificOutput.additionalContext` only for types whose manifest sets `posttooluse_output=hookSpecificOutput` (VERIFIED `agmsg: scripts/check-inbox.sh:34-37,441-456`). Goose has no context-injection field on any hook in either version (VERIFIED primer section 3.4), so the output would be ignored. agmsg never marks read on this path (`check-inbox.sh:398-407`), so nothing would be lost, and nothing would be delivered either.

### 2.4 Cooperative rule file (the pattern agmsg uses for agy, OpenCode, Cursor, Gemini, Grok)

- For hookless harnesses agmsg writes an always-applied rule: "After each tool call, automatically check the agmsg inbox ... Command: check-inbox.sh '<type>' '<project>'" (VERIFIED `agmsg: scripts/drivers/types/cursor/_delivery.sh:25-49`; same shape for opencode/antigravity per `overview.md`).
- INFERRED Goose equivalent: `.goosehints` / `AGENTS.md`, or the Top-Of-Mind text. A Goose driver could reuse the pattern. It is cooperative, not enforced: every compliant check costs one shell tool round trip, and the 60 s cooldown makes most of them return "skipped" (or, with `stop_output=json`, a status line).

### 2.5 Goose surfaces agmsg does not use

These come from the primer and are not agmsg mechanisms. They are listed only so the comparison is complete:

- **Top-Of-Mind file** (`GOOSE_MOIM_MESSAGE_FILE`): injected before every provider call, capped at 64 KiB, with no tool call needed. It is the best fit for in-turn delivery on Goose (primer section 5). agmsg does not use it. INFERRED: agmsg's "mark read only after a verified display" rule could not be met here, because the file is re-sent on every call with no receipt. agmsg's PostToolUse "additive, never consume" stance (`check-inbox.sh:398-407`) would be the matching policy.
- **ACP `steer`** (main): mid-turn injection with a pickup id (primer section 7). agmsg does not use it.

## 3. Wake mechanism

agmsg has no Goose wake. VERIFIED by absence (section 1). The generic options:

### 3.1 `poke` through the herdr terminal driver

- Path: `poke.sh <team> <name> --body-file F` → placement record → `terminal_poke` → `herdr agent prompt <pane> <text>`. VERIFIED `agmsg: scripts/poke.sh:124-187`, `agmsg: scripts/drivers/terminals/herdr/ops.sh:1086-1111`.
- Prerequisites a Goose seat lacks:
  - A placement record (`run/...` written by spawn or by the self-naming hook). Without one, poke dies with "no placement record" (`poke.sh:125-126`). A hand-joined Goose seat with no hooks never gets one (INFERRED from `poke.sh:126`'s message).
  - A live Herdr agent in the pane. `herdr agent prompt` "needs a running agent". When there is none, the driver returns 12 (`ops.sh:1088-1097,1102-1110`). Herdr has no Goose detection (primer section 10, INFERRED from a `strings` check). INFERRED: `agent prompt` would fail for a Goose pane, so herdr-driver pokes to Goose would fail with exit 12.
- Draft safety: `agmsg_safe_poke` runs only when the type's manifest sets `input_prompt_marker`. With no marker, the check is skipped and the poke is one unconditional `terminal_poke` call (VERIFIED `agmsg: scripts/poke.sh:171-180`, `agmsg: scripts/lib/safe-poke.sh:366-372`). A Goose seat registered under a borrowed type would be checked against that type's marker: `❯` boxed for Claude, `›` plus a footer for Codex. Goose's prompt is `"> "` with a dim hint (primer section 9), so INFERRED: the box would not be found and the poke would be refused with **exit 15** ("input box could not be located"). That is fail-closed, so it is safe, but it means no wake.
- Busy detection: none in poke itself (VERIFIED, `terminal_poke` at `ops.sh:1086-1111` checks nothing before `agent prompt`). The herdr readiness helper `terminal_team_input_ready` (`ops.sh:1058-1081`) serves `team --fix`, not poke. It also requires a Herdr agent of the expected kind (`agent_kind_mismatch`, `:1076-1077`) and accepts `idle|done|working`, so it would refuse a Goose pane, and it would count a mid-turn agent as ready.

### 3.2 `poke` through the tmux driver (if the Goose pane is in tmux)

- Mechanism: `send-keys -l -- "$text"`, `sleep 0.3`, then `send-keys Right Enter` as a separate burst. VERIFIED `agmsg: scripts/drivers/terminals/tmux/ops.sh:556-564`. INFERRED for Goose's rustyline prompt: the text lands in the buffer, `Right` at end-of-line is a no-op, and `Enter` submits. That would wake an idle `goose session`.
- Hazards on Goose (INFERRED from primer section 9):
  - When Goose is busy, the bytes sit in the tty and become the *next* prompt after the turn, not a mid-turn steer.
  - If Goose is at a tool-approval or elicitation select, the keys answer that prompt.
  - With no input marker, there is no draft check, and text is appended to a human's half-typed line.

### 3.3 Desktop app PTY injection

- The app writes `[agmsg] <from>: "<preview>" — run /<cmd> to check it.`, waits 300 ms, then writes `\r`, immediately and with no idle check. VERIFIED `agmsg: app/README.md:34-44`, `agmsg: app/src-tauri/src/pty.rs:10-19,309-327`. This is agent-agnostic at the PTY layer, and Goose's rustyline would submit it (INFERRED).
- It is not reachable for Goose today: the app spawns only manifest types (`AgentType` read from `type.conf`, `agmsg: app/src-tauri/src/agmsg.rs:506-531`), and its state classifier knows only `claude-code|claude|codex|gemini|grok|grok-build` (`agmsg: app/src-tauri/src/agent_state.rs:168`). It is also irrelevant to Herdr panes, because the app must own the PTY.
- If it were reachable: the immediate write lands in Goose's tty while busy and becomes the next prompt (INFERRED, primer section 9). It also appends to any draft.

### 3.4 agmsgd, Monitor, Codex bridge, agy supervisor

None of these applies to Goose. agmsgd has only the Claude cross-session socket and `codex queue` channels (VERIFIED `overview.md`). Goose has no Monitor-like tool that streams events into context (primer section 6: MCP notifications go to the UI only). The Codex bridge and the agy PTY supervisor are specific to those harnesses.

### 3.5 Latency and failure modes (summary)

| Path | Idle wake? | Latency | Main failure |
|---|---|---|---|
| Manual `inbox.sh` | no | when the model next chooses to check | messages wait until the human asks |
| Stop hook on Goose main (hand-wired) | no, turn end only | 0 to 60 s cooldown after a turn ends | "policy denial" framing; block cap 8 |
| Stop hook on Goose 1.36 (hand-wired) | no | n/a | **silent loss**: marked read, never shown |
| herdr `poke` | intended | immediate | exit 12 (no Herdr agent) or 15 (no input marker match) (INFERRED) |
| tmux `poke` | yes (INFERRED) | ~0.3 s | clobbers drafts, answers approval prompts, queues behind a busy turn |
| desktop app inject | n/a for Goose | immediate | type not spawnable |

## 4. Setup / installation footprint on Goose

- agmsg writes nothing Goose-specific. VERIFIED: there is no Goose branch in `install.sh` (grep), and no `.agents/plugins`, `.goosehints` or Goose config writes.
- Indirect footprint: `~/.agents/skills/<cmd>/` (scripts, SQLite DB, `run/`, the Codex-typed `SKILL.md`). Goose auto-discovers that skill (section 1.1). Uninstalling agmsg removes it from Goose too (INFERRED).

## 5. Token cost

- Per quiet turn: **zero** on Goose today, because nothing is injected. INFERRED: the only standing cost is the `agmsg` skill entry (name plus a one-sentence description, `agmsg: scripts/skill-base.md:1-4`) in Goose's skill list in the system prompt (`goose@1.36: crates/goose/src/skills/client.rs:224-240`).
- On `load_skill`: about 38 KB of instructions (`skill-base.md` 30,951 B + `codex/template.md` 7,200 B before slot rendering, `wc -c`; INFERRED ~9-10k tokens), for the wrong harness.
- Per message (manual pull): one shell tool call plus a result of one row per message, unbounded (section 2.1). A hand-wired Goose main Stop hook adds about 40 bytes of Goose wrapper plus agmsg's header and rows, with one extra model turn per block.

## 6. Limitations and bugs visible

1. **No Goose type**, and `join.sh` rejects unknown types. A Goose user has to masquerade as another type. VERIFIED section 1.
2. **The shared Codex-typed skill leaks into Goose** through `~/.agents/skills`, steering a Goose session to register as `codex`. INFERRED consequence; both halves VERIFIED (section 1.1). This is a latent agmsg bug of the #1449 kind.
3. **Read-on-stdout-write** (`check-inbox.sh:489-511`) has no receipt check. On any runtime that discards hook stdout (Goose 1.36 Stop), it turns into silent loss. agmsg documents the gap but keeps the design.
4. **Unknown-type pokes skip draft protection** (empty marker means an unconditional poke). On tmux that means typing into a human's draft. VERIFIED `poke.sh:171-180`.
5. **No bounds** on batch size in hook or inbox output (`overview.md`).

## 7. Clever ideas worth stealing

- **Fail-closed draft guard, split into two codes** (`safe-poke.sh:378-386`): 14 means "someone is typing" (two snapshots ~1 s apart differ), and 15 means "cannot find the input box" (refuse rather than guess). For a Goose composer reader we would add, an unknown layout should likewise refuse, never fall through to typing.
- **herdr draft save/clear/poke/restore** (#1384, `safe-poke.sh:270-352`): for an *unfocused, abandoned* draft, save it to a mode-0600 file, clear the box, deliver, retype the draft and verify the read-back. The file is removed only when the retyped draft reads back byte-identical. On a mismatch, or when the box does not read back empty, the file is kept and its path is reported (`safe-poke.sh:349-352`). On Goose this would need care: clearing must not use Ctrl+C, which deletes the buffer and on an empty line arms exit (primer section 9). Ctrl+U / Ctrl+A Ctrl+K are the rustyline-safe clears (INFERRED).
- **Additive mid-turn and consuming turn-end** (`check-inbox.sh:398-407`): a path whose model receipt is not observed (for Goose: the Top-Of-Mind file) may show an item early but never consumes it. Only a verified path consumes. This maps directly onto a Goose design where the MOIM file previews pending items and a later verified display (inbox or Stop) acknowledges them.
- **Per-type wire shape as manifest data** (`stop_output`, `posttooluse_output`), not code branches (`check-inbox.sh:30-37`). This matches our adapter-trait rule, and Goose main's Stop decision JSON would be another value of that data.
- **Counter-lesson**: agmsg's PTY inject dropped idle detection because Claude's spinner never went quiet (`pty.rs:10-19`). On Goose the busy tty queues keystrokes into the next prompt (primer section 9), so an idle-agnostic inject is *less* harmful there than on Claude. It still clobbers drafts and approval prompts, so it does not justify skipping the composer check.

## Verification

Adversarial pass on 2026-10-23 against the agmsg clone (HEAD `6e0315468db3`), both Goose trees and `gh api repos/fujibee/agmsg`. I checked about 45 concrete claims.

Confirmed: the stats (1,564 stars, v1.6.0 published 2026-10-22); the type list; the absence of `goose`, `GOOSE_`, `MOIM` and `.agents/plugins` anywhere in agmsg; the `join.sh` gate; the trust rule in `type-registry.sh`; Goose's manifest and marker lists in `open_plugins.rs`; the skill roots and `default_enabled` flags; the Codex-typed shared skill (`install.sh` lines and `AGMSG_TYPES_WITH_OWN_SKILL_FILE`); the byte sizes 30,951 and 7,200. I also confirmed the `check-inbox.sh` emit, mark-after-write and PostToolUse no-consume lines, Goose main's `classify_output` and Stop denial text, the 1.36 fire-and-forget Stop at `agent.rs:2286`, poke's placement record and marker skip, the herdr and tmux `terminal_poke`, the desktop app's PTY inject and its type gate, and the cursor rule text.

Changed:
- Corrected the gate lines: `spawn.sh:133`, `doctor.sh:471`, `whoami.sh:43`. The `whoami.sh` gate applies only when a type is passed.
- The skill name is `<cmd>` (default `agmsg`), not always `agmsg`.
- Block cap: 8 per turn, counted since the turn's kickoff, with a cited default constant. Neither "8 consecutive" nor the warning-text line range was the right evidence.
- Quiet-turn behaviour: the borrowed `codex` type has `stop_output=json`, so Goose main classifies its status output as `HookFailure` (fail-open) rather than Allow. The original said quiet turns print nothing.
- The herdr readiness helper (`terminal_team_input_ready`) serves `team --fix`, not poke, and requires a matching Herdr agent kind. The original implied that poke used it.
- Draft recovery: the saved file is removed when the read-back matches and kept otherwise. The original said it was always kept.

Not re-verified, taken from `overview.md` or the primer: agmsgd channel list, absence of bounds, Herdr having no Goose detection, and Goose 1.36 hook loading paths.
