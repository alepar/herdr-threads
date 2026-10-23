# agmsg x OpenCode: message injection and wake-up

- Upstream: https://github.com/fujibee/agmsg. The clone is shallow at HEAD `6e031546` (2026-10-23, release 1.6.0). Paths without a prefix are relative to the agmsg clone root. No code from it was executed.
- Third-party dependency: https://github.com/tsukimiya/opencode-sentinel. I read it at `main` `793d7939` (committed 2026-10-15; package v0.2.0, CHANGELOG dated 2026-10-16; 0 stars) through `gh api .../contents`. Paths prefixed `sentinel:` are relative to that repo's root.
- OpenCode source comes from the primer's clones: `[1.18.1]` is tag v1.18.1 and `[dev]` is dev HEAD `f9f463be`. Harness facts not re-derived here come from `primers/opencode.md`.
- Tags: VERIFIED means read in source, docs or an upstream issue (cited). INFERRED means my own reading, not run.

## Support status

agmsg lists OpenCode as a **native** type with three delivery modes. In practice, **neither automatic mode reaches the model on a stock install**, and agmsg's own issue tracker says so.

| Item | Value | Evidence |
|---|---|---|
| Type manifest | `name=opencode`, `cli=opencode`, `spawnable=yes`, `cmd_prefix=$`, `prompt_arg=--prompt`, `model_arg=--model`, `detect_proc=opencode opencode-*`, `hooks_file=.opencode/rules/agmsg.md`, `readiness_sentinel=yes`, `delivery_modes=monitor turn off` | VERIFIED `scripts/drivers/types/opencode/type.conf:1-13` |
| Modes | `turn`: a markdown rule tells the model to run `check-inbox.sh` "after each tool call". `monitor`: the rule tells the model to start `watch.sh` under opencode-sentinel's `sentinel_monitor` tool, with turn as a written fallback. `off`: manual `$agmsg` only. `both` is rejected. | VERIFIED `scripts/drivers/types/opencode/_delivery.sh:41-108`, `docs/opencode.md:87-102` |
| Not covered | agmsgd (the 1.6.0 daemon) has channels only for Claude Code and Codex. There is no OpenCode plugin of agmsg's own: "No native OpenCode plugin integration". | VERIFIED `docs/opencode.md:162-165`; overview, agmsgd row |
| Known broken (V1) | Issue **#1663** (open, 2026-10-20, OpenCode 1.19.0 + sentinel 0.1.5, **under the herdr terminal driver**) says: "`.opencode/rules/agmsg.md` is not loaded by OpenCode". In addition, the sentinel-launched watcher's owner token never matches the `actas-claim.sh` lock, so the watcher refuses the pair ("not serving these pairs"). | VERIFIED `gh api repos/fujibee/agmsg/issues/1663` |
| Known broken (V2) | Issue **#1462** (open, opened by the sentinel author) says that on OpenCode 2.0, TUI `--model` is rejected, TUI `--prompt` only pre-fills the composer, `$agmsg` is plain text, and `.opencode/rules/` is not loaded. | VERIFIED `gh api repos/fujibee/agmsg/issues/1462` |
| Origin | The monitor mode came from the sentinel author's fork. It landed upstream as #570 (2026-07-31), a maintainer re-land of the author's PRs #546 and #547. | VERIFIED `sentinel:docs/adr/0001-defer-agmsg-upstream-contribution.md:7-11` |

### The rule file is dead weight (VERIFIED independently of #1663)

OpenCode 1.18.1 builds its instruction set from three sources (`[1.18.1] packages/opencode/src/session/instruction.ts:60-68,110-150`):

- the global `~/.config/opencode/AGENTS.md` (or `~/.claude/CLAUDE.md`);
- the first project `AGENTS.md`, `CLAUDE.md` or `CONTEXT.md` found walking up;
- the paths and globs listed in config `instructions`.

A grep for `rules` across `[1.18.1] packages/opencode/src` finds no loader for `.opencode/rules/`. On `[dev]`, `packages/core/src` has none either. The OpenCode docs (`[dev] packages/web/src/content/docs/rules.mdx:109-116`) say rule directories must be listed under `instructions`, e.g. `".cursor/rules/*.md"`.

agmsg never writes `instructions`: a grep for `opencode.json` or `"instructions"` in agmsg finds only unrelated jev docs. Its bats tests only assert that the file exists (`tests/test_delivery.bats:2582-2695`), never that OpenCode loads it.

**So on a stock install, `turn` and `monitor` both behave exactly like `off`.** The difference shows only when the user hand-adds `.opencode/rules/agmsg.md` to `instructions`, or when the model happens to read the file.

The rest of this report describes the mechanism as designed, which is what you get once the rule is loaded, for example via the #1663 workaround.

## Injection mechanism

### Turn mode: cooperative self-poll by the model

**Hook or API.** There is none. The "hook" is prose in the rule file:

```
## PostToolUse
After each tool call, automatically check the agmsg inbox for unread messages.
- Command: '$SKILL_DIR/scripts/check-inbox.sh' '$type' $project_q
```

(VERIFIED `scripts/drivers/types/opencode/_delivery.sh:44-50`.) The project path is `printf %q`-escaped, because the rule is command text the model pastes into its shell (`_delivery.sh:26-37`). The model runs it through OpenCode's built-in bash tool, so OpenCode itself does nothing agmsg-specific (INFERRED).

**Code path** (`scripts/check-inbox.sh`):

1. The event defaults to `Stop` (`:28`). OpenCode's manifest has no `stop_output` or `posttooluse_output`, so the quiet path prints nothing (`:33-37`, `:49-53`). VERIFIED.
2. stdin is read only if it is not a TTY, with a 2 s timeout (`:67-74`). Under the bash tool there is no hook JSON, so no `session_id` is found, and `GROK_SESSION_ID` is unset, so `SESSION_ID` stays empty (`:86-95`). VERIFIED code; the bash tool's stdin shape is INFERRED.
3. The identity comes from `identities.sh <project> opencode`. It takes **the first registered agent** for (project, type), not the session's in-memory `actas` role (`:134-163`, comment `:309-311`). VERIFIED.
4. Cooldown: `run/.lastcheck-<agent>` mtime against `delivery.turn.check_interval` (default 60 s). Inside the window the script exits silently (`:177-198`). VERIFIED.
5. The actas lock: an `other:*` or `unknown:*` state skips the team (`:312-326`). The OpenCode skill's `actas` steps take no lock: the base slot runs `identities.sh`, then `join.sh`, then sets FROM in memory, and the opencode `template.md` has no `actas` slot (`scripts/skill-base.md:138-145`). The state is therefore normally `free`, and delivery proceeds. VERIFIED code; INFERRED outcome.
6. Rows come from `storage_list_unread`. Each team adds a header line plus one line per message: `N new message(s) in <team>:` and then `  [<ts>] <from>: <body>`. Newlines and tabs in a body are escaped to `\n`/`\t` (`:331-380`). VERIFIED.
7. Output: `{"decision":"block","reason":"<escaped text>"}` (`:462`). This is the Claude/Codex Stop-hook shape, printed verbatim into a bash tool result, so the model reads a JSON blob whose `"decision":"block"` means nothing to OpenCode. VERIFIED emit; INFERRED effect.
8. Rows are marked read only after the `printf` succeeds (`:502-511`). VERIFIED.

**When it fires.** Only when the model chooses to run the command. "After each tool call" is self-referential: the check is itself a tool call. Only the 60 s cooldown keeps a literal-minded model from looping, and even then each loop costs one LLM step (INFERRED). Nothing fires on an idle session.

**Bounds.** The text has no per-message or per-batch byte limit (overview, VERIFIED by absence). OpenCode's own tool-output `Truncate` spill-to-file (primer §5) is the only effective bound (INFERRED). The #777 fix moves the SQL to stdin to avoid `MAX_ARG_STRLEN`. It does not limit what is shown (`:339-362`). VERIFIED.

**Framing.** There is no sender-trust framing beyond `[ts] from: body`. The payload sits inside a bash tool result, so the model sees it as command output.

**Dedupe and ACK.** Read state is the only signal: an event-log `message_read` row plus a per-(team, agent) cursor shared with `watch.sh`. There is no peer ACK. A duplicate is possible if a mark fails after a successful write. The authors deliberately prefer that to loss (`:498-501`). VERIFIED.

### Monitor mode: a stdout stream turned into user messages

**Hook or API.** The rule tells the model:

```
- command: '$SKILL_DIR/scripts/watch.sh' "\${SENTINEL_SESSION_ID:--}" $project_q $type
- description: agmsg inbox stream
Do not launch it more than once per session
```

(VERIFIED `_delivery.sh:72-82`.) There is no `active_name` 4th argument, so this is a *broad* watcher over every pair for (project, opencode).

**Code path, agmsg side** (`scripts/watch.sh`):

1. `-` maps to an empty session id (`:59-60`). An empty id becomes `agmsg-<uuid>` (`:111-122`). It is then normalized to a composite `<sid>.<opencode pid>` by walking up the ppid chain for `detect_proc` (`:140`; `scripts/lib/instance-id.sh:369-378`; `scripts/lib/resolve-project.sh:487-527`). VERIFIED.
2. The watcher polls `storage_watch_after` every 5 s. That function is one snapshot transaction that returns **all** unread rows after the cursor, with no `LIMIT` (`scripts/drivers/storage/sqlite.sh:529-553`). VERIFIED.
3. One stdout line per message: `<ts> | <team> | <from> → <to> | <body>`, with newlines escaped (`watch.sh:1681`, header `:32-35`). Ids are consumed (cursor plus `message_read`) only for lines whose `printf` succeeded, and only after the pair ownership is re-checked (`:1704-1725`). VERIFIED.
4. Liveness: the watcher exits once the composite agent pid is positively dead (`:1340-1365`). VERIFIED.

**Code path, sentinel side** (VERIFIED `sentinel:src/lib/watcher.ts`, `sentinel:src/monitor.tool.ts`, `sentinel:src/v1.ts`):

1. `spawn(command, {shell:true, detached:true, env:{...process.env, SENTINEL_SESSION_ID: sessionID}})` (`watcher.ts:20-25`).
2. stdout is split into lines. Lines are batched over **200 ms**, and each is prefixed `[monitor:<mon_id>] ` (`watcher.ts:47-56,113-117`).
3. Flood guard: more than 100 lines in 1 s kills the process group and sends a "FLOOD DETECTED" notice (`watcher.ts:68-89`).
4. stderr goes to `/tmp/sentinel-<id>-stderr.log` (`watcher.ts:121-124`). A non-zero exit sends "process exited unexpectedly" (`:126-141`).
5. **Delivery on V1:** `v2Client.session.prompt({sessionID, parts:[{type:"text", text}]})`. That is `POST /session/:id/message`, a real `role:user` message with no `synthetic` flag, no `noReply` and no `system` field (`monitor.tool.ts:38-50`). The client reuses the host's in-process fetch, because the stock TUI has no TCP listener (`v1.ts:21-31`, matching primer §1). The code comment explains the choice: "on V1 the steer delivery is admitted but never promoted on an idle session, so only this route reaches the session in every state" (`monitor.tool.ts:35-37`).
6. **Delivery on V2:** `ctx.session.prompt({sessionID, text, delivery:"steer"})` (`sentinel:src/v2.ts:104-108`), the 2.x native inbox (primer §9).
7. The send is fire-and-forget. A failure is only `console.error`'d (`watcher.ts:38-44,54`).

**What the model sees** (INFERRED by composing the two formats), as one user turn per 200 ms batch:

```
[monitor:mon_lx3k9a_ab12] 2026-10-23T10:00:01Z | myteam | alice → oc-worker | please review PR 12
```

**When it fires.** About every 5 s, whether the session is idle or busy. On a busy V1 session the user message is persisted at once, and the running loop picks it up at the next step boundary, i.e. it is steered in mid-turn (primer §3, VERIFIED by reading `runner.ts:115-122`, `prompt.ts:1092,1111-1116`). The sentinel README says something weaker: "steer notifications are not processed until the current turn completes" (`sentinel:README.md:130`). That line describes steer delivery, not the V1 parts route sentinel actually uses, so the source reading stands; the busy-session timing is not observed (INFERRED).

**Bounds and framing.** agmsg sets no size bound. The sentinel's only bound is the 100 lines/s flood kill. Each message is framed by the monitor id plus agmsg's pipe-separated line. It arrives as **user-authored text**, indistinguishable in role from the human (INFERRED from the missing `synthetic`).

**Dedupe and ACK.** The watcher consumes on pipe write, not on OpenCode receipt, so sentinel is outside agmsg's at-least-once boundary (INFERRED from `watch.sh:1681-1725` plus `watcher.ts:38-44`). There is no ACK.

### Manual and skill path (`off`, and the fallback for everything)

`$agmsg` makes the model run `inbox.sh $TEAM $AGENT`, which displays the inbox and marks it read (`scripts/skill-base.md:90-94`). VERIFIED.

On V1, skills are exposed as `/agmsg` slash commands. The skill template is `SKILL.md` plus a base-dir note (`[1.18.1] packages/opencode/src/command/index.ts:134-147`). `$agmsg` is therefore plain prose that the model must map to the skill. That works on 1.x by convention (INFERRED). On 2.x it is observed to fail without natural-language help (#1462).

## Wake mechanism

| Mode | Wakes an idle session? | How | Evidence |
|---|---|---|---|
| `off`, `turn` | **No.** The inbox is checked only when the model is already working and chooses to check. | none | VERIFIED rule text; `README.md:219,275` (the same statement for turn mode in general) |
| `monitor` with sentinel on V1 | **Yes.** `POST /session/:id/message` creates a user message and runs the loop (`state.ensureRunning`). An idle session starts a turn. | harness-native server API, in-process | VERIFIED `sentinel:src/monitor.tool.ts:38-50`; primer §3. sentinel README: "Idle sessions respond within 500ms" (`sentinel:README.md:130`) |
| `monitor` with sentinel on V2 | Yes, through `delivery:"steer"`, verified by the sentinel author on 2.0.29 | 2.x inbox | VERIFIED `sentinel:src/v2.ts:104-108`, `sentinel:CHANGELOG.md:20-23` |
| `poke` (manual, agent-invoked) | Yes, by typing into the pane | herdr: `herdr agent prompt <pane> <text>`; tmux: send-keys | VERIFIED `scripts/poke.sh:177-182`, `scripts/drivers/terminals/herdr/ops.sh:1084-1110` |

- **Idle/busy detection.** There is none, by design. Sentinel never checks `session.status`. OpenCode's prompt path is correct in both states: an idle session starts a turn, and a busy one steers at the next step. **Every** agmsg message therefore triggers or extends a model turn. Priority and silent delivery do not exist (INFERRED from code; agmsg has no priorities, see overview).
- **Draft safety.**
  - Monitor mode never touches the TUI composer. It is a server-side message, so a human's half-typed draft survives (primer §1, VERIFIED by structure). The session does start a turn under the human's cursor, and the human's later Enter queues behind it (INFERRED).
  - `poke` for OpenCode has **no draft guard**. `input_prompt_marker` is unset in the opencode manifest, and an empty marker means "no check, one unconditional terminal_poke call" (VERIFIED `scripts/lib/safe-poke.sh:366-372`, `scripts/drivers/types/opencode/type.conf`).
  - Whether `herdr agent prompt` itself refuses or handles an OpenCode pane is unknown. The agmsg herdr driver marks that argv as "ASSERTED, NOT yet measured" (`ops.sh:30-31`).
- **Latency.** Up to the 5 s poll, plus 200 ms batching, plus model time. #1663 reports a test message "answered within 3 seconds" with the workaround in place (VERIFIED issue text).
- **Spawn readiness.** `spawn.sh opencode` launches `opencode --model <m> --prompt "$agmsg actas <name>"`. V1's `--prompt` auto-submits; V2's only pre-fills (#1462). For a monitor-mode project, `readiness_sentinel=yes` makes spawn wait for a ready file (`scripts/spawn.sh:974-1090`). Only an exclusive watcher (with `active_name`) writes that file (`watch.sh:1053-1060`), and the rule starts a broad one. A monitor-mode OpenCode spawn should therefore end in `status=timeout` after 90 s (exit 3) unless `--no-wait` is given (INFERRED; the bats test uses `--no-wait`, `tests/test_spawn.bats:493-500`).

### Failure modes

1. **The rule is never loaded**, so nothing auto-delivers, silently (above; #1663 §1). The docs admit the fallback is honour-system: "An agent that ignores it delivers nothing in `monitor` mode, and nothing reports that" (VERIFIED `docs/opencode.md:127-131`).
2. **The owner-token mismatch.** An `actas-claim.sh` run through the bash tool yields the owner `-.<pid>`. The sentinel-spawned watcher derives its owner from `ses_…`. The watcher then refuses the pair (VERIFIED #1663 §2 with an observed log line).
3. **Loss across the sentinel boundary.** The watcher marks a message read once its line enters the pipe. If `session.prompt` then fails, the error goes only to `console.error` in the OpenCode process and the message is gone (INFERRED from `watch.sh:1681-1725` plus `watcher.ts:38-44`).
4. **Flood kill on backlog.** A fresh watcher dumps every unread row in one unbounded poll (`sqlite.sh:539-551`, no `LIMIT`; `watch.sh:26-28`: "a fresh watcher delivers existing unread messages"). More than 100 rows within 1 s trip sentinel's flood guard. That kills the watcher after the watcher has already consumed rows that sentinel dropped (INFERRED composition).
5. **Watcher notices become prompts.** Status lines that `watch.sh` prints to stdout ("no available identities … nothing to do", "ERROR: cannot open message DB", "delivery … is STUCK …", `watch.sh:1021-1023,1049,1533,1570`) each become a user message that wakes the model (INFERRED).
6. **Sentinel on old V1 hosts.** sentinel 0.2.0 needs V1 ≥ 1.18.29 (`sentinel:README.md:20-25`). The local 1.18.1 install (primer §0) would need sentinel 0.1.x.
7. **Orphans.** If OpenCode is SIGKILLed, sentinel's dispose does not run (`sentinel:README.md:129`). agmsg's composite-pid liveness guard then still stops `watch.sh` within one poll (`watch.sh:1340-1365`). VERIFIED both; the combination is INFERRED.
8. **Multiple sessions per process.** Sentinel monitors are keyed by the session that called the tool (`sentinel:src/lib/manager.ts:76-84`). If the human switches the TUI to another session, deliveries keep going to the original one (INFERRED; primer §10 notes that a pane is not one session).

## Setup and installation footprint on the harness

| Write | When | Evidence |
|---|---|---|
| `~/.config/opencode/skills/<cmd>/SKILL.md`, an OpenCode-typed render of `skill-base.md` (about 31 KB of source before slot rendering) | `install.sh`, only if `~/.config/opencode/` already exists | VERIFIED `install.sh:1424-1433`, `install.sh:1092-1096`; `wc -c scripts/skill-base.md` = 30,951 |
| `~/.agents/skills/<cmd>/SKILL.md` (Codex-typed; overwritten with the OpenCode render by `--agent-type opencode`, which breaks Codex identity) | install | VERIFIED `docs/opencode.md:15-36` |
| `<project>/.opencode/rules/agmsg.md` (about 1.1 KB turn / 1.7 KB monitor after rendering), replaced on each mode change and removed for `off` | `delivery.sh set <mode> opencode <project>` | VERIFIED `_delivery.sh:39-107`; sizes from rendering the heredocs with sample paths |
| `opencode.json` `"plugin": ["opencode-sentinel"]` | **the user**, via `opencode plugin opencode-sentinel` (agmsg does not do it) | VERIFIED `docs/opencode.md:111-113`, `sentinel:README.md:29-49` |
| `~/.agents/skills/<cmd>/run/.lastcheck-<agent>`, pid, ready and log files | runtime | VERIFIED `check-inbox.sh:177-198`, `watch.sh:142` (`RUN_DIR="$SKILL_DIR/run"`; the header comment at `:36-37` still says `~/.agents/agmsg/run`, which is stale) |
| Uninstall removes `~/.config/opencode/skills/<cmd>` | `uninstall.sh` | VERIFIED `uninstall.sh:562,571` |

No OpenCode config, plugin, hook or `instructions` entry is written by agmsg. That is the root cause of failure 1.

## Token cost

Numbers are byte estimates from the source, not measurements. All INFERRED unless noted.

- **Static per session.** Only the skill's name and description are in the system prompt, at about 250 bytes (the description line `skill-base.md:3` is 246 bytes). The full skill (tens of KB) loads only on `$agmsg`. The rule file costs 0 bytes on a stock install because it is not loaded. If wired into `instructions`, it is about 1.1-1.7 KB in every request's system prompt, and three quarters of that is terminal-awareness prose unrelated to delivery (`_delivery.sh:52-63,91-102`).
- **Turn mode, per quiet tool call** (if the model complies): one extra bash tool call, about 100-150 output tokens for the long absolute command, plus an empty result. **That is one extra LLM step per real tool call, which roughly doubles the step count of a tool-heavy turn, with full context re-read each step.** This is the dominant cost, and it buys a check that the 60 s cooldown makes a no-op most of the time.
- **Turn mode, per message:** a JSON wrapper of about 45 bytes, a team header of about 30 bytes, and `[ts] from: body` per message (about 30 bytes plus the body). It arrives inside the extra tool call above, so no further step is needed.
- **Monitor mode, per quiet turn:** 0 bytes and 0 calls. The watcher prints nothing.
- **Monitor mode, per message:** a `[monitor:mon_…] ` prefix of about 25 bytes plus about 50 bytes of `ts | team | from → to |` plus the body. Each batch is a **new user turn**, so every message costs a full model request (the whole context) even when it is low priority, plus whatever the model then does (reading, replying through `send.sh`). Startup costs one tool call (`sentinel_monitor`) and its about 70-byte result (`sentinel:src/lib/core.ts:58`).

## Limitations and bugs

1. The rule-file location is not an OpenCode instruction source in V1 or V2, so both automatic modes are inert by default (VERIFIED; #1663, #1462).
2. Turn mode is purely cooperative. The prose "after each tool call" is self-referential and prices a whole LLM step per check. A Stop-hook payload (`decision:block`) is reused as model-visible text in a context where it has no meaning (VERIFIED emit; INFERRED effect).
3. Identity in turn mode is "first registered agent for (project, opencode)". Two OpenCode seats in one project read one inbox. `actas` changes FROM only, not the receive side (VERIFIED `check-inbox.sh:134-163,309-311`).
4. The owner-token mismatch between `actas-claim.sh` and the sentinel watcher (VERIFIED #1663).
5. At-most-once across the sentinel pipe, and flood-kill loss on large backlogs (INFERRED, see failure modes).
6. There is no priority or silent class. Every monitor delivery wakes or extends a turn. `noReply:true` staging and `synthetic` framing, both available in OpenCode (primer §3), go unused.
7. User-role injection: messages arrive as plain `role:user` text, the same role as the human's own input. The model cannot tell a peer from its operator by role (INFERRED).
8. A hard dependency on a 0-star third-party plugin that the user installs. No capability detection exists: "agmsg writes the rule; it does not detect whether the tool exists" (VERIFIED `docs/opencode.md:127-131`).
9. The spawn readiness wait cannot succeed in monitor mode (broad watcher, no ready file), so expect a 90 s timeout without `--no-wait` (INFERRED).
10. V2 breaks spawn (`--model` rejected, `--prompt` pre-fill only), skill invocation and the rule file. The direction is still open (VERIFIED #1462).

## Clever ideas worth stealing

1. **Wake through the session API, not the composer (sentinel).** An in-process server plugin calls `client.session.prompt` with the host's in-process fetch (`sentinel:src/v1.ts:21-31`). This reaches a stock TUI that has no TCP port, starts a turn when the session is idle, and steers in when it is busy. It never touches the human's draft. This is our primer's candidate shape 1, proven to work in the field (#1663: an answer within 3 s). For herdr-threads it would replace keystroke wakes for OpenCode entirely. We would do it with a content-free `wake up` (R18) and the body via `messages.transform` or `tool.execute.after`, not by pushing bodies as user turns.
2. **The V1/V2 dual entrypoint in one package** (`sentinel:src/index.ts`, `docs/adr/0002`). `setup(ctx)` serves 2.x and a lazily imported `server(input)` serves 1.x. On 1.18.x, `setup` sees a partial context and no-ops (`sentinel:src/v2.ts:79-95`). This is a clean answer to "OpenCode 2.x replaces the plugin API" (primer §9) that needs no version probe, which fits our versionless rule: it detects the capability by shape (`typeof ctx?.session?.prompt === "function"`).
3. **The measured V1 caveat:** "steer delivery is admitted but never promoted on an idle session" on V1, so they use the message route (`sentinel:src/monitor.tool.ts:35-37`). This is a useful field fact for our design: on 1.x, idle wake must go through `prompt`/`prompt_async`, not an inbox-style steer.
4. **`SENTINEL_SESSION_ID` exported into the spawned child's environment** (`watcher.ts:24`). This is the same idea as the primer's `shell.env` identity channel. OpenCode gives agent-run shells no session id by default, so a plugin must inject one. #1663 shows the trap: inject it into **every** agent shell (`shell.env`), not just the plugin's own children, or the claim and the watcher disagree.
5. **The flood guard plus `until` one-shot waits** (`watcher.ts:68-89,93-111`). A generic "wake me when this line appears" tool is a cheap, bounded long-poll substitute, and the flood cap protects context. We should cap per-wake payloads ourselves rather than rely on a kill.
6. **`printf %q` for paths in model-executed command text** (`_delivery.sh:26-37`). This is a small robustness point for any skill or rule text in which we print commands for the model to run.
7. **Negative lesson.** Never ship delivery that depends on the model honouring prose ("after each tool call…"). agmsg's own docs and issue tracker document the silent failure. Our rule 3 (deliver through hooks and channels, not model calls) holds up here.

## Verification

Adversarial pass on 2026-10-23 against the agmsg clone at `6e031546`, the opencode-sentinel source at `793d7939`, the OpenCode `v1.18.1` and `dev` (`f9f463be`) clones, `gh api` (agmsg issues #1462 and #1663, latest release `v1.6.0`, sentinel stars 0) and `primers/opencode.md`.

- Re-read and confirmed: the type manifest; both rule heredocs and the `printf %q` quoting in `_delivery.sh`; the `check-inbox.sh` steps (event default, stdin guard, first-agent identity, cooldown, actas skip, row format, `decision:block` emit, mark after write); the `watch.sh` session-id handling, liveness guard, ready-file gating and status lines; `storage_watch_after` with no `LIMIT`; the sentinel spawn env, 200 ms batching, 100 lines/s flood kill, stderr log, V1 `session.prompt` parts route, V2 `delivery:"steer"` and the V2 shape check; OpenCode 1.18.1 instruction sources (`instruction.ts:60-68,110-150`) and the absence of a `.opencode/rules` loader in 1.18.1 and dev core; the skill slash-command template; the spawn readiness wait (monitor or both only; turn and off skip it) and the 90 s, exit 3 timeout; the install and uninstall paths; the herdr `agent prompt` poke and the empty-marker poke path; the contents of #1462 and #1663.
- Fixed: the printf line in `watch.sh` is `:1681`, not `:1680`, and the consume block starts at `:1704`. Sentinel `793d7939` was committed 2026-10-15; 2026-10-16 is the CHANGELOG date for 0.2.0. #570 re-landed #546 and #547, not #547 alone. The runtime-file citation now points at `watch.sh:142` (`$SKILL_DIR/run`), because the `:36-37` header comment names a stale path. A failed sentinel delivery goes to `console.error` in the OpenCode process, not to the watcher's stderr log. The skill description is about 250 bytes, not 230.
- Added: the sentinel README describes busy-session steer timing more weakly than the source reading; the mid-turn claim is now marked as not observed.
