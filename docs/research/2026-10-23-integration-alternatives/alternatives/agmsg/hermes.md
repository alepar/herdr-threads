# agmsg x Hermes Agent: message injection and wake

- Upstream: https://github.com/fujibee/agmsg (MIT, 1,564 stars, latest release v1.6.0 on 2026-10-22; `gh api repos/fujibee/agmsg`). Clone read: shallow, HEAD `6e031546` (2026-10-23). Paths are relative to the clone root. Nothing from the clone was executed.
- Harness reference: `../../primers/hermes.md` (Hermes v0.22.5 source, cited below as "primer §N").
- Tags: **VERIFIED** = read in agmsg source, docs or issues (cited). **INFERRED** = my reasoning, not confirmed by source or a run. **unknown** = not found.

## TL;DR

agmsg supports Hermes only as a "beta" type with **no automatic injection and no wake of its own**. It installs a `SKILL.md` that Hermes exposes as the `/agmsg` slash command. The model or the human runs `/agmsg`, and the skill tells the model to run `inbox.sh`, whose stdout (the whole unread backlog, mark-read on display) is the injection. Hermes is not spawnable (#279, open). Two generic, type-blind paths can still reach a Hermes pane: `poke.sh` through the Herdr driver (`herdr agent prompt`) and the agmsg desktop app's PTY kickoff. For Hermes both skip the draft check, because the manifest declares no `input_prompt_marker`. agmsg uses none of Hermes' native surfaces (`pre_llm_call`, `transform_tool_result`, `ctx.inject_message`, gateway `prompt.submit`/`session.steer`).

Correction to the mapper's note: the Hermes command is **`/agmsg`**, not `$agmsg`. Hermes' manifest sets no `cmd_prefix`, so the renderer defaults to `/` (`scripts/lib/skill-render.sh:40-41`). Only antigravity, codex, gemini and opencode set `cmd_prefix=$`. VERIFIED.

## 1. Support status

| Fact | Evidence |
|---|---|
| Hermes is listed as a supported agent ("beta" since 1.1.0) | VERIFIED `README.md:26` (supported-agents logo alt text; the README itself carries no "beta" label for Hermes), `CHANGELOG.md:1043` under `## [1.1.0]` ("Add Hermes Agent as a beta agent type"). PR #118 ("feat: add Hermes Agent support") was closed **unmerged**; the support landed by another commit (not traced) |
| Manifest: `cli=hermes`, `detect=explicit`, `readiness_sentinel=no`, `delivery_modes=off`. It has no `spawnable`, `session_env`, `detect_proc`, `input_prompt_marker`, `rename_cmd`, `resume_arg` or `hooks_file` | VERIFIED `scripts/drivers/types/hermes/type.conf:1-14` |
| Delivery plug is a no-op: "Hermes has no agmsg automatic delivery hook: there is no SessionStart / Stop equivalent to write, so the only valid mode is `off`" | VERIFIED `scripts/drivers/types/hermes/_delivery.sh:1-21` |
| `delivery.sh set monitor/turn/both … hermes` is rejected by the central gate | VERIFIED `scripts/delivery.sh:878`; tests `tests/test_delivery.bats:3595-3640` |
| Not spawnable: Hermes' only prompt-taking modes (`-z`, `chat -q`) are one-shot. A two-step `-q` then `--continue` boot is "an unverified idea" | VERIFIED `type.conf:5-11`, `template.md:7`, `README.md:237`, issue #279 (open, filed 2026-07-01: the old `spawnable=yes` failed with an argparse error). PR #119 (Hermes profiles in spawn) was closed unmerged. `spawnable=yes` was later dropped under #279 (`tests/test_spawn.bats:431-432`) |
| Identity: `detect=explicit`, so a Hermes session is never auto-detected from its environment. The skill passes the type literally (`whoami.sh "$(pwd)" hermes`) | VERIFIED `docs/agent-types.md:23`, `tests/test_install.bats:1384` |
| Process-ancestry detection has no manifest `detect_proc` for Hermes. It falls back to hard-coded case arms (bug #626 was that cursor/grok/hermes used to match `claude codex gemini`) | VERIFIED `scripts/lib/resolve-project.sh:318-321`, issue #626 (closed) |
| No Hermes-version handling of any kind, and nothing about the v0.22.1 gateway shift | VERIFIED by absence (`grep -ri hermes` over the clone, ~35 files; none mentions a Hermes version, the gateway or `HERMES_HOME`) |

Verdict: **supported as a manual-only messaging skill.** It is "unsupported" for every automatic injection or wake mode.

## 2. Injection mechanism

### 2.1 The one native path: skill, then a terminal tool call to `inbox.sh` (pull, model-initiated)

1. **Install.** `install.sh` renders `scripts/skill-base.md` plus the Hermes overlay to `~/.hermes/skills/<cmd>/SKILL.md`. It does so only if `~/.hermes` already exists. VERIFIED `install.sh:1435-1445` (fresh install) and `install.sh:1097-1101` (`--update`).
   ```sh
   HERMES_SKILL_DIR="$HOME/.hermes/skills/$CMD_NAME"
   if [ -d "$HOME/.hermes" ]; then
     mkdir -p "$HERMES_SKILL_DIR" || exit 1
     agmsg_render_skill hermes "$CMD_NAME" "$HERMES_SKILL_DIR/SKILL.md" || exit 1
   ```
   Hermes "exposes installed skills as dynamic slash commands", so this file makes `/agmsg` exist (VERIFIED as a quote from the PR #118 body, a contributor's claim; the Hermes side is not checked here).
2. **Trigger.** Someone types `/agmsg`, or the model decides to invoke the skill. The skill's default action is "**IMMEDIATELY** run inbox check for each TEAM: `~/.agents/skills/<cmd>/scripts/inbox.sh $TEAM $AGENT`. Do NOT ask the user what to do" (VERIFIED `scripts/skill-base.md:90-94`). The Hermes overlay adds "Hermes has no automatic delivery mode. Keep delivery `off` and check `/agmsg` manually" (VERIFIED `scripts/drivers/types/hermes/template.md:4`).
3. **Delivery.** The model runs `inbox.sh` through Hermes' `terminal` tool. Its stdout becomes a **tool result**. That tool result is the whole injection. (INFERRED: this is the generic Hermes mechanism. Skills are text instructions and the scripts are run by the model's terminal tool.)

**Payload format** (VERIFIED `scripts/inbox.sh:86-95`):
```
N new message(s):

  [<created_at>] <from>: <body with \n and \t escaped to literal "\n"/"\t">
```
or `No new messages.` (`inbox.sh:37`, `:49`). Every message is one display line. There is no thread, no id shown to the model, no priority and no marker.

**When it fires:** only when the skill is invoked, which means on a human or model decision. There is no turn-start, no mid-turn and no turn-end path. VERIFIED by `delivery_modes=off` and the no-op `agmsg_delivery_apply` (`_delivery.sh:11`).

**Size bounds: none.** `inbox.sh` prints the **entire** unread backlog with unbounded bodies. Its only size engineering avoids `ARG_MAX` by writing the SQL to a temp file (VERIFIED `inbox.sh:61-84`, #777/#1045). Hermes' own terminal tool truncation would be the only bound (unknown limit; not in the primer).

**Dedupe / ack:** read-on-display. Only the ids of the rows actually printed are marked read, so a message arriving between SELECT and mark is never marked unseen. That is a good race fix (VERIFIED `inbox.sh:109-124`). A mark failure is reported on stderr: "some or all may be shown again (#1011)". No explicit ACK concept; no per-session cursor (per (team, agent) recipient).

**Side effects on each inbox read:** `agmsg_self_name_on_action` and `agmsg_self_rename_on_action` run on each read (VERIFIED `inbox.sh:19-27`, `send.sh:77-83`). Self-name reads the pane from the environment (`HERDR_ENV=1`, `HERDR_PANE_ID`, `scripts/drivers/terminals/herdr/ops.sh:275-285`). It then names the Herdr pane and **writes the placement record** that `poke.sh` later needs (`scripts/lib/self-name.sh:103-235`, `record` flag at `:231`). Self-rename is a no-op for Hermes, which has no `rename_cmd` (`scripts/lib/self-rename.sh:137-139`). Measured cost is 0.2-0.3 s per inbox (`self-name.sh` header). **This matters: the only way a Hermes seat becomes pokeable is by having run `/agmsg` (or send/history) once from a terminal command whose environment carries the pane id.**

### 2.2 Generic, type-blind paths that also land in a Hermes pane

These are not Hermes integrations. They are agmsg's universal fallbacks, and they apply to Hermes because nothing excludes it.

**(a) `poke.sh` through the Herdr terminal driver.** Another member's model runs `/agmsg poke <hermes-seat> <text>`. The skill says to use poke "to reach a member whose watcher is not delivering (that is what it is for)" (VERIFIED `scripts/skill-base.md:180-196`). The code path:
- `poke.sh:125-127`: the placement record must exist ("a hand-joined member gets one when a terminal-aware session names its pane"). For Hermes that requires §2.1's self-name.
- `poke.sh:177-179`: `INPUT_MARKER="$(agmsg_type_get "$TYPE" input_prompt_marker)"`. For Hermes this is **empty**.
- `scripts/lib/safe-poke.sh:425`: `if [ -n "$marker" ]; then …draft checks…` is skipped entirely for Hermes. The code goes straight to `safe-poke.sh:494` `terminal_poke "$id" "$text"`.
- `scripts/drivers/terminals/herdr/ops.sh:1087-1112`: `herdr agent prompt <pane> "<text>"`, which "submits on its own". The driver header marks the argv as "ASSERTED, NOT yet measured against a live call" (`ops.sh:30-34`).

The payload is the **full free-form text** the poking model chose, typed and submitted as a Hermes user turn. It has no framing, no bound and no dedupe. The submitted text becomes a real user-role message (INFERRED from primer §9).

**(b) agmsg desktop app (Tauri) PTY kickoff.** The app spawns only `spawnable=yes` types as agents. Any other member gets a plain `bash` pane (VERIFIED `app/src/App.tsx:1210-1255`, `app/src-tauri/src/agmsg.rs:604-617`). For every non-native pane (Hermes is listed by name as one, `App.tsx:78-81`), each new team message addressed to that pane's label is injected (VERIFIED `App.tsx:1089-1117`):
```ts
const flat = e.payload.body.replace(/\s+/g, " ").trim();
const preview = flat.length > 80 ? `${flat.slice(0, 80)}…` : flat;
const kickoff = `[agmsg] ${e.payload.from}: "${preview}" — run /${cmdName} to check it.`;
void invoke("pty_inject", { id: pane.id, text: kickoff });
```
`pty_inject` writes the text, sleeps 300 ms, then writes `\r` (VERIFIED `app/src-tauri/src/pty.rs:310-327`). If the human started `hermes` in that bash pane, the kickoff lands in Hermes' composer (INFERRED). It is bounded to 80 body chars plus a pointer, and there is no idle check.

## 3. Wake mechanism

| Path | Mechanism | Idle/busy detection | Draft safety | Status for Hermes |
|---|---|---|---|---|
| Native type support | none | n/a | n/a | VERIFIED absent (`_delivery.sh`, `type.conf:14`) |
| `poke.sh` via Herdr | `herdr agent prompt` (types and submits) | **none**. Any non-zero exit of `herdr agent prompt` is mapped to agmsg exit 12 ("may be gone, or have no live agent", `ops.sh:1089-1112`); that Herdr requires a *recognized* agent is INFERRED | **none for Hermes**: the two-snapshot check (#1321/#1322) and the #1384 save/clear/poke/restore recovery run only when `input_prompt_marker` is set (`safe-poke.sh:425`, `:476-490`) | Works in principle; model-initiated, not automatic |
| Desktop app | PTY write + `\r` after 300 ms | none | none | Only for a Hermes the human launched inside the app's bash pane |
| Watcher / Monitor / Stop hook / agmsgd | n/a | n/a | n/a | Not applicable: `delivery_modes=off`. `watch.sh:1084` says "the per-turn hook is absent on hermes" |

**Latency.** Native: unbounded (until a human or the model types `/agmsg`). Poke: one Herdr RPC, sub-second once someone decides to poke (INFERRED). Desktop app: the event-listener latency plus 300 ms.

**Failure modes (INFERRED unless cited):**
1. **Busy clobber.** A poke or kickoff that lands while Hermes is generating hits `display.busy_input_mode`, which is **`interrupt`** by default. The keystroke wake then **redirects or interrupts the running turn** (primer §9). agmsg does not look at busy state for Hermes at all.
2. **Draft clobber.** With no `input_prompt_marker`, a poke submits into a composer that may hold a human's half-typed text, which gets merged into the submission. agmsg's own docs admit the residual race even for types with the check (`poke.sh:34-42`). For Hermes, the window is the whole draft.
3. **Herdr does not recognize Hermes.** If `herdr agent prompt` refuses a pane with no recognized agent, agmsg reports exit 12. herdr-threads' notes say a bare Hermes on macOS is not recognized by Herdr (primer §11). Whether Herdr's `agent prompt` works for an unrecognized process is unknown.
4. **Hermes ≥ v0.22.1 gateway (most important).** Terminal commands run as children of the shared gateway daemon. Their environment is not the pane's (primer §1). Two consequences:
   - `HERDR_PANE_ID` is absent. Then self-name exits (`self-name.sh:128-132`, `unknown:*`/empty return 0; `ops.sh:275-286` yields nothing without `HERDR_ENV=1` or a pane id), no placement record is written, and **poke fails with "no placement record"** (`poke.sh:127`).
   - Or the pane id is the one the gateway inherited from whichever pane started it. Then the Hermes seat's record points at the wrong pane, and a poke types into someone else's session.

   agmsg already hit the second shape with Codex's shared app-server. Self-name's corroboration read, which asks the pane for its agmsg label, was added to catch it (#1130, `self-name.sh` header "the environment is not this seat's at all"). Whether `agmsg_terminal_name_self_safe` then refuses or overwrites the other seat's pane name is not traced (unknown). `whoami.sh "$(pwd)" hermes` also depends on the terminal tool's cwd being the session cwd and not the gateway's. The primer shows per-session env injection per terminal command and a `ctx.session_cwd()` plugin API (primer §1); that the terminal tool's cwd is the session cwd is INFERRED.
5. **Big pastes.** A long poke body (5 or more lines, or 2,000 or more chars) is collapsed by Hermes into `[Pasted text #N: …→ <file>]` (primer §9). Whether `herdr agent prompt` sends it as a bracketed paste is unknown.

## 4. Setup / installation footprint on the harness

- Writes: `~/.hermes/skills/<cmd>/SKILL.md`, and only if `~/.hermes` exists (VERIFIED `install.sh:1440-1445`). It does not honour `$HERMES_HOME` or Hermes profiles: the path is hard-coded under `$HOME` (VERIFIED). A profile with its own `HERMES_HOME` would not see the skill (INFERRED).
- Shared runtime and store: `~/.agents/skills/<cmd>/scripts/…` plus the per-team SQLite file. The skill hard-codes `~/.agents/skills/<cmd>/scripts` (VERIFIED `tests/test_install.bats:1386`, `skill-base.md:39`).
- No `config.yaml` hooks, no plugin, no MCP server, no project files. `delivery.sh set off hermes` writes nothing: `[ ! -e "$TEST_PROJECT/.hermes/agmsg.json" ]` (VERIFIED `tests/test_delivery.bats:3620-3626`).
- Uninstall removes `~/.hermes/skills/<cmd>` (VERIFIED `uninstall.sh:560-572`, fixed in #1469/#1471).
- Consent prompts in Hermes: none needed (no hooks). The model's `terminal` calls go through Hermes' normal tool approval (INFERRED).

## 5. Token cost

- **Per quiet turn: 0.** Nothing is injected, because nothing runs automatically. A skill index entry (name plus the one-line description, `skill-base.md:1-4`, about 230 bytes) probably sits in Hermes' skill listing for every session (INFERRED; how Hermes lists skills is not covered by the primer).
- **Per inbox check:** the rendered SKILL.md body is **~31 KB** (`scripts/skill-base.md` is 30,951 bytes; the Hermes overlay adds ~0.6 KB). If Hermes loads the full skill body on each `/agmsg` invocation, that is roughly 7-8k tokens per check (INFERRED; whether Hermes re-loads it or caches it per session is unknown). Then there is one terminal tool call (`whoami.sh` on first use, then `inbox.sh`).
- **Per message:** ~25-30 bytes of framing (`  [<ISO ts>] <from>: `) plus the full body, unbounded. A poke costs the poked body as a user turn plus a whole model turn. A desktop-app kickoff is ≤ ~120 bytes, but it forces the follow-up `/agmsg`, so you pay the skill cost again.
- Compared with herdr-threads on Hermes: we inject a ≤ 4 KiB check-in at every `pre_llm_call` (primer §11). That costs on non-quiet turns only, but it is automatic. agmsg costs zero until someone asks, and then a lot.

## 6. Limitations and bugs visible in source

1. No automatic delivery or wake on Hermes, although Hermes offers `pre_llm_call` shell hooks (`{"context": …}` JSON), `transform_tool_result`, and `ctx.inject_message` (primer §0, §3, §4). The `_delivery.sh:4` rationale, "there is no SessionStart / Stop equivalent to write", is outdated against Hermes' hook set: `on_session_start` and `pre_llm_call` exist, and `pre_llm_call` is documented as the `UserPromptSubmit` equivalent (primer §3). INFERRED: agmsg's `check-inbox.sh` (already a Stop/PostToolUse hook for other types, `scripts/check-inbox.sh:8-39`) could be wired as a Hermes `pre_llm_call` shell hook if its output were wrapped as `{"context": …}`.
2. Poke into Hermes has **no draft or busy guard**, because the manifest has no `input_prompt_marker`. Combined with Hermes' default `busy_input_mode: interrupt`, a peer's poke can abort a running Hermes turn (INFERRED).
3. The `~/.hermes` path is hard-coded. It ignores `HERMES_HOME`/profiles and is skipped silently when `~/.hermes` does not exist yet, so installing Hermes after agmsg means rerunning `install.sh --update` (VERIFIED condition `install.sh:1441`).
4. The inbox is unbounded, has no thread or priority, and escapes newlines in bodies. That makes it compact for one-liners and unreadable for structured bodies.
5. The placement record depends on the seat having acted once. A fresh Hermes seat that has never run `/agmsg` cannot be poked (VERIFIED `poke.sh:127` message).
6. The Herdr `agent prompt` argv is unverified upstream ("ASSERTED", `ops.sh:30-34`).
7. Spawn has stayed broken or refused since #279 (open since 2026-07-01). Nobody has tested the `-q` then `--continue` idea, and Hermes' gateway-era `hermes --resume ID` / `-c` (primer §6) would make a two-step boot plausible.
8. Nothing accounts for Hermes ≥ v0.22.1's gateway environment (§3 failure mode 4).

## 7. Clever ideas worth stealing

1. **Mark read only the ids actually displayed** (`inbox.sh:109-124`). This closes the display/mark race without a transaction around the read. Our cursor advance should keep the same property. We already ACK on display, so this is a check to make, not a change.
2. **Self-naming on every action, with label corroboration** (`self-name.sh` header, #1130). A seat re-asserts its pane binding whenever it acts. It does not trust an environment that "agrees on somebody else's pane": it asks the terminal what label the pane carries. This is directly relevant to our probable Hermes gateway breakage (primer §12 item 1), where an inherited `HERDR_PANE_ID` would bind every Hermes session to the gateway's first pane. A cheap witness check ("does the pane we think we are in actually host this session?") would catch that.
3. **Manifest-driven capability gating** (`delivery_modes=off` checked centrally, `doctor.sh:759-775` not warning for no-delivery types). A type that cannot be woken says so as data, and every consumer degrades the same way. This matches our "degrade only the broken capability" rule.
4. **Poke exit taxonomy** (10 unreachable, 12 no live agent, 13 unsupported, 14 draft in progress, 15 box not locatable) plus `--retries` with backoff, used only for 14/15 (taxonomy `ops.sh:1090-1099`, `poke.sh:184-195`, `:214`; retry options `poke.sh:69-101`). That is a clear contract for a model-invoked wake.
5. **Desktop-app kickoff: flatten the preview and cap it at 80 chars, then point at the pull command** (`App.tsx:1098-1115`). Its stated rationale is that typing raw bodies "feeds arbitrary user text straight into a TUI's input box" and breaks the keystroke replay. This is a middle ground between our content-free `wake up` (R18) and full-body typing. We should not adopt it, because our R18 rule is stricter and content arrives via `pre_llm_call`. Still, it confirms that a short single-line wake text is the safe keystroke shape.
6. Negative lesson: **don't ship keystroke poke to a harness without a draft and busy guard.** For Hermes the better path is native: `ctx.inject_message(session_key=…)` or gateway `prompt.submit`, which queue on busy and never touch the composer (primer §4, §5.1). Neither agmsg nor we use it yet.

## 8. Would agmsg's generic mechanism work well on Hermes? (assessment, INFERRED from the primer)

- **The CLI-plus-skill pull works on any Hermes version**, because it needs only the model's terminal tool. Identity resolves by `(pwd, type)`, not by process tree, so it survives the gateway shift as long as the terminal tool's cwd is the session cwd. That is its strength: no version dependence, and nothing to break.
- **Wake via Herdr poke** works on pre-0.22.1 Hermes when Herdr recognizes the pane, at the cost of interrupt-on-busy and draft risk. On ≥ 0.22.1 it probably fails, or misroutes, for lack of a correct `HERDR_PANE_ID` in the gateway-run terminal commands, unless the operator lists `HERDR_PANE_ID` in `terminal.client_env_passthrough` (primer §1).
- **No MCP path is used.** On Hermes an MCP server would not have helped wake anyway: Hermes ignores `resources/list_changed` and logs `notifications/message` without showing it to the model (primer §8).
- Overall: on Hermes, agmsg is a manual mailbox. It cannot honour anything like our delivery contract's rules 2-4: wake by priority, serve in-turn, never nudge busy.

## Verification

Adversarial pass on 2026-10-23 against the clone at HEAD `6e031546`, `gh api repos/fujibee/agmsg` (1,564 stars, MIT, latest release v1.6.0 published 2026-10-22), the issues and PRs cited, and the Hermes primer.

Confirmed as cited: the Hermes manifest, delivery plug and overlay; the `cmd_prefix` default `/` and the four `$` types; the central mode gate (`delivery.sh:875-880`) and its tests; the install and `--update` blocks; the skill default action and poke guidance (`skill-base.md:90-94`, `:195`); `inbox.sh` format, ARG_MAX temp file, and mark-only-displayed ids; self-name, self-rename and the `record` flag; poke's placement-record check, the empty-marker skip (`safe-poke.sh:425`, `:476-490`, `:494`) and the ASSERTED herdr argv; the desktop app kickoff, bash fallback and `pty_inject` 300 ms `\r`; `watch.sh:1084`; uninstall (#1469/#1471); doctor's no-delivery handling; `check-inbox.sh` hook roles; the skill-base size (30,951 B) and overlay (662 B); and the primer facts on gateway env, `busy_input_mode: interrupt`, paste collapse, MCP notifications, the `pre_llm_call` 4,096-byte check-in and the macOS recognition note.

Changed: PR #118 and #119 were both closed unmerged (the report implied #118 shipped the support); "beta" comes from the CHANGELOG, not the README; #279 opened 2026-07-01, not 2026-06; exit 12 is agmsg's mapping of any `herdr agent prompt` failure, so "needs a Herdr-recognized agent" is now INFERRED; the poke exit taxonomy citation moved from the retry-option parsing to `ops.sh:1090-1099` and `poke.sh:184-195`, `:214`; the self-name early-exit range is `:128-132`; the session-cwd claim is now attributed correctly and marked INFERRED; the "hits listed in this report" phrase was replaced with what the grep found.
