# hcom x Goose (Block / AAIF)

- Upstream: https://github.com/aannoo/hcom, clone `alts/hcom` (scratchpad), HEAD `de33e31a0cdbba4c5863f7cc050d8b79aa50bcdf` (2026-10-21, v0.7.31). MIT. 597 stars / 94 forks (GitHub API 2026-10-23, from [overview.md](overview.md)).
- Goose facts come from the [Goose primer](../../primers/goose.md) and its two source trees (`goose@1.36` = v1.36.0, `goose@main` = 1.56.0), under `scratchpad/goose-src/`.
- Citations are `path:line`, relative to the hcom clone root unless marked `goose@…`. **V** = VERIFIED (read in source or docs). **I** = INFERRED. Nothing in either clone was executed.

## 1. Support status

**Not supported.** **V**

- A case-insensitive `grep -rni goose` over the whole hcom clone finds no match. That covers source, docs, CHANGELOG, tests and scripts. **V**
- The README tool table lists 14 tools. Goose is not one of them. Its last row is `Anything else | manual via hcom listen | hcom start (run inside tool)` (README.md:174-192) **V**. The README restates this generic path: "Any other AI tool without hooks can join by running `hcom start` … prompt: run: `hcom start --help` follow what it says and stay connected" (README.md:110-121) **V**.
- `hcom goose` is not a launcher. A first token is a launch tool only if it parses as a released `Tool` (`is_launch_tool`, src/router.rs:48-50). Otherwise the router answers `Unknown command 'goose'` with a typo suggestion or the main help (src/router.rs:62-92) **V**.
- hcom cannot recognise a Goose environment. `detect_tool` walks each integration's env markers, then `HCOM_TOOL`, and falls back to `Tool::Adhoc` (src/shared/tool_detection.rs:79-98). No integration declares a Goose marker, so inside Goose every hcom command sees the tool `adhoc` **V** (marker list read; the absence follows from the grep).
- hcom ships no MCP server (no MCP-server code in `src/`; the "mcp" hits are pass-through launch flags of other tools in src/integration_spec.rs:659-1216, plus comments, screen-scrape test fixtures (src/pty/screen.rs:1238) and Grok ACP method names (src/delivery/grok.rs:627, 791)) **V**. So the MCP-extension route that Goose offers is not available either.

What remains for Goose is hcom's generic **adhoc** mode, plus an undocumented PTY wrapper for arbitrary commands. Both are analysed below.

## 2. Injection mechanism (the adhoc path as it would run under Goose)

There is no Goose hook, plugin, MCP or system-prompt integration. Messages reach the Goose model only as **stdout of `hcom` commands that the model itself runs through Goose's shell tool** (the developer extension `shell`). There are two variants.

### 2.1 Joining: `hcom start` (bootstrap through tool output)

- The model runs `hcom start` because the user told it to (README.md:117-119). `start_bare` sees no `HCOM_PROCESS_ID` and a non-launched context, so it sets `tool = "adhoc"`. It generates a fresh name, creates the instance row, and prints `[hcom:<name>]`, the bootstrap, and `[hcom:<name>]` again. The repeat is deliberate, so that `| tail` still shows the name (src/commands/start.rs:687-708, 782-845) **V**.
- The adhoc bootstrap includes `DELIVERY_ADHOC`, which starts "Messages do NOT arrive automatically" and sets out a `CONNECTED MODE` rule: "Run exactly one foreground blocking command: `hcom listen --name {instance_name} --timeout [large_num]` … When it returns … you MUST manually run `hcom listen` again … Do not wrap `hcom listen` in `while`, `watch`, … background jobs" (src/bootstrap.rs:218-242, selected at :607-617) **V**. It also adds `INLINE_SEND_NOTICE` = "Read hcom command output fully; it can consume incoming mail." (src/bootstrap.rs:244, 615-617) **V**.
- Identity persists only through the model. Goose's shell tool spawns a fresh process for each call (`run_command`, `goose@1.36 crates/goose/src/agents/platform_extensions/developer/shell.rs:506-518`, spawns a new child with piped stdout/stderr and `stdin(Stdio::null())`) **V**. For a Goose shell, hcom identity is resolved from `--name`, or from a process binding that only hcom launches create (resolution order system sender > internal session id > `--name` > `HCOM_PROCESS_ID` binding, src/identity.rs:462-466, 535-560). `send` and `listen` are identity-gated and error with "Use --name <yourname> or run inside an hcom-launched session" (src/identity.rs:25, 679-700) **V**. So every `hcom send` and `hcom listen` the Goose model issues must carry `--name <name>`, which it has to remember from the bootstrap. Nothing in Goose's env carries it **I**.

### 2.2 Delivery path A: blocking `hcom listen`

- Code path: `cmd_listen` resolves the identity, marks an adhoc row `listening` (`set_adhoc_listening`, src/commands/listen.rs:109-121, 272), binds a TCP notify server on 127.0.0.1 and registers it as kind `listen` (:275-282), then enters `listen_loop` (:324-425) **V**.
- Loop: it checks for an external stop, then `InlineBatch::take` (unread prefix). If the inbox is empty, it waits on the notify socket in slices of at most 30 s until the timeout. The default timeout is 86400 s (:194-206, 404-421) **V**. `hcom send` fans a connect-and-close wake out to every registered instance-loop port (`wake_all`, src/notify/wake.rs:53-74; `WakeKind::Listen`, src/notify/mod.rs:28-46) **V**.
- Payload (text mode): `\n<messages>\n<remaining note>`. One message renders as `[intent:thread #id] sender -> recipient: text`. Several render on **one line** as `[N new messages] | m1 | m2 …` (`format_messages_text`, src/commands/listen.rs:60-89; prefix rules `format_envelope_prefix`, src/cli_context.rs:382-401) **V**. If more mail arrived after the batch was taken, a trailing `[hcom] new message(s) arrived — run: hcom listen --name <name>` line follows (`arrived_since_note`, src/cli_context.rs:288-295; src/commands/listen.rs:375-381) **V**. `--json` instead prints one JSON object per message with `from, text, event_id, reply_id, intent, thread` (src/commands/listen.rs:92-103) **V**. Status lines such as `[Listening for messages to luna. Timeout: 86400s]` and `[Timeout: no messages after …]` go to stderr (:303, 395) **V**. Goose's shell tool returns stdout *and* stderr to the model (`ShellOutput { stdout, stderr, … }`, goose@1.36 shell.rs:177-190) **V**, so these lines are model-visible as well **I**.
- The listen output carries no per-instance `hints` and no one-time tips. Those belong to the hook formatter `messages::format_messages_json` (overview, "Injection mechanisms") **V** (the listen formatter was read).
- When it fires: only while the model is inside a `hcom listen` tool call. It returns on the **first** non-empty batch, so one message wakes it (:354-385) **V**.

### 2.3 Delivery path B: piggyback on any hcom command

- For adhoc instances only, the router appends unread mail after the output of **every** hcom command the model runs (`maybe_deliver_pending_messages`, src/cli_context.rs:349-382; receiver limited to `tool == "adhoc"` by `inline_receiver`, :236-247) **V**. Framing:
  ```
  ────────────────────────────────────────
  [hcom]
  ────────────────────────────────────────
  [intent #id] sender → me (+N others): text      (or "[N new messages] | … | …")
  [+K more unread — run: hcom listen --name me]
  ```
  (src/cli_context.rs:370-381, 405-466) **V**. `hcom send` claims this delivery itself, after persisting the outgoing message (src/commands/send.rs:1160-1180) **V**.
- Skipped for `--json` output (src/cli_context.rs:362-364) **V**.
- So mail reaches a busy Goose agent mid-turn, but only when it happens to run some `hcom …` command (`send`, `list`, `events`, …). That is the "tool output" analogue of a PostToolUse hook, restricted to hcom's own tool calls **I**.

### 2.4 Bounds, de-dup, ack (both paths)

- Batch: a chronological prefix of at most 50 messages (`MAX_MESSAGES_PER_DELIVERY`, src/shared/constants.rs:19). The remainder is announced as `[+K more unread — run: hcom listen --name …]` (src/cli_context.rs:253-288) **V**. Per message: 1 MiB (`MAX_MESSAGE_SIZE`, src/shared/constants.rs:22) **V**. There is no per-batch byte cap **V** (none found).
- Goose truncates the shell output at 2000 lines or 50,000 bytes. It saves the full text to a file and returns only the **last 50 lines** plus a notice (goose@1.36 shell.rs:150-152, 734-775) **V**. On 1.36 the preview has no byte cap. Because a multi-message listen batch is a single line, a large batch comes through the "last 50 lines" preview essentially whole, so Goose 1.36 does not bound it; one wake can push up to about 50 MiB (50 × 1 MiB) into context **I** (from the constants and the preview logic). Goose main adds `OUTPUT_PREVIEW_BYTES = 10_000` and trims the preview's tail to it (goose@main shell.rs:161-164, 789-797) **V**, so on main the model sees at most the last ~10 KB of an oversized batch and the rest only in the saved file **I**.
- Ack = cursor advance, made **after** `stdout.write_all + flush` succeeds. If the write fails, nothing is acked. Any retry may duplicate a message but never skips one. The cursor only moves forward (`InlineBatch::emit`, src/cli_context.rs:296-344) **V**. Under Goose, "flushed into the pipe of Goose's shell tool" counts as delivered, whether or not Goose later truncates, discards on cancel, or compacts it out **I**.
- There is no de-dup beyond the cursor, and no read receipt to the sender beyond status `deliver:<sender>` (src/cli_context.rs:335-345) **V**.

## 3. Wake mechanism

### 3.1 What "wake" means here

hcom cannot wake an idle Goose session that sits at the `> ` prompt. The adhoc gates say so: "Adhoc delivery is manual via `hcom listen` / hookless CLI unread, so these conservative gates are documented but mostly unused" (src/integration_spec.rs:2338-2347) **V**. The only "wake" is that a **blocked `hcom listen` tool call returns**. Goose then makes the next provider call within the *same* turn **I** (Goose's loop is provider call → tool calls → provider call; primer §1). So Goose is never idle in CONNECTED MODE. It sits in one endless turn, parked inside a tool call.

- Latency: a sender's TCP connect wakes `server.wait` at once, then one SQLite read **V** (src/commands/listen.rs:410-421). Sub-second **I**. Without a notify server it falls back to a 100 ms poll (:412-416) **V**.
- Idle/busy detection: none is needed, because the agent is "busy by construction". The adhoc row shows `listening` while blocked and `inactive` after the call returns (src/commands/listen.rs:109-133) **V**.
- Draft safety: nothing is typed, so the composer is never at risk. But while `hcom listen` blocks, Goose CLI does **not read the terminal**. Only a Ctrl+C task is armed (primer §9, `goose@1.36 crates/goose-cli/src/session/mod.rs:1086-1092`) **V**. A human cannot talk to the agent without Ctrl+C, which cancels the whole run. Anything typed meanwhile sits in the tty buffer and becomes the next prompt (primer §9) **I**.

### 3.2 Failure modes (Goose specific)

1. **The loop depends on obedience.** If the model forgets to re-run `hcom listen`, ends its turn, or the user interrupts, the agent drops to `> `. After that, no hcom mechanism reaches it. Mail waits until a human prompts it **I** (from §3.1 and the absence of any typed wake for adhoc).
2. **Shell tool timeouts.** Goose's shell tool kills a command only if the model passes `timeout_secs` (goose@1.36 shell.rs:170-175, 536-540) **V**. The bootstrap asks for `--timeout [large_num]`, but the model picks both numbers, and a model-chosen `timeout_secs` below hcom's timeout turns every quiet period into a "Command timed out" error result **I**.
3. **Turn budget and context growth.** CONNECTED MODE is one Goose `reply` that never ends, and each delivery is one more provider round trip in that turn. Goose caps this at `max_turns`, default 1000 (goose@1.36 crates/goose/src/agents/agent.rs:67, 1641) **V**. The whole connected session also accumulates in a single turn's history **I**. How compaction behaves there is unknown.
4. **Identity reaping.** `hcom list` and `hcom listen` run `cleanup_stale_instances(db, 3600, 3600)` (src/commands/list.rs:213; src/commands/listen.rs:817 test). The test shows that an adhoc row left `inactive` for more than 3600 s is deleted, while a quiet `listen` timeout is not treated as an exit (src/commands/listen.rs:820-860) **V**. A Goose agent that left CONNECTED MODE for an hour loses its name. Its next `--name` call fails, and re-running `hcom start` mints a new name, because names are never reused (overview, "Identity") **I**.
5. **Wrong-tool detection.** If Goose was started from a shell that inherited another tool's markers (for example from inside an hcom-launched Claude pane), `detect_tool` would classify the Goose shell commands as that tool (src/shared/tool_detection.rs:79-98) **I**.
6. **Ack before the model has seen the mail.** See §2.4: Ctrl+C after the flush, or Goose truncation or compaction, loses acknowledged mail **I**.

### 3.3 The unofficial PTY route (`hcom pty goose`)

`hcom pty <cmd>` accepts arbitrary commands, kept "explicitly ad-hoc": "Arbitrary commands are supported for diagnostics and tests" (src/pty/mod.rs:74-105; src/main.rs:162-175, where `goose` → `PtyTarget::AdhocCommand`, with no ready patterns) **V**. Even with an instance name, its automatic delivery loop would never type into Goose:

- The gates come from `ToolConfig::for_tool(Tool::Adhoc)`, with `require_idle: true` and `require_prompt_empty: true` (src/pty/shared.rs:558; src/integration_spec.rs:2338-2347) **V**.
- `prompt_empty` is computed by `get_input_box_text(target.name())`, which returns `None` for any name that does not parse as a tool (src/pty/screen.rs:765-795). `None` makes `prompt_empty = false` (src/pty/shared.rs:210-222) **V**. `get_input_box_text` also returns `None` for `Tool::Adhoc` itself, and an `AdhocCommand` target's delivery tool is `Tool::Adhoc` (src/pty/mod.rs:101-106). So whenever `evaluate_gate` gets past the idle check it stops at `prompt_has_text` (src/delivery.rs:1066-1080) **V**, and adhoc rows reach `listening` only from inside `hcom listen` anyway (src/commands/listen.rs:109-121) **V**.
- Manual typing would work. `hcom term inject <name> "<text>" --enter` writes straight to the PTY's inject port with no gate (src/commands/term.rs:86-118) **V**. The bundled `hcom run onidle` only waits for a *watched* agent's `listening` status and then calls that same `term inject` (src/scripts/bundled/onidle.sh:1-20 usage) **V**; it checks no composer either. That is an operator tool. It is not wired into delivery, and it has no Goose composer, approval or elicitation awareness **I**. Under Goose it would answer a tool-approval select or wipe nothing, depending on the screen at the time (primer §9 hazards) **I**.

## 4. Setup and installation footprint on Goose

- hcom writes **no Goose configuration**: no `~/.agents/plugins`, no `~/.config/goose/*`, no extension entry, no `GOOSE_MOIM_MESSAGE_FILE` **V** (no Goose code exists).
- State lives only in `~/.hcom/hcom.db` (or `HCOM_DIR`) (overview, "Architecture") **V**. The user's step is a prompt: "run: `hcom start --help` follow what it says and stay connected" (README.md:117-119) **V**.
- Side note: the hcom repo root has a `gemini-extension.json` (name, version and a description string; no hooks or MCP entries) and a `skills/hcom-agent-messaging/SKILL.md` (3858 B, setup and troubleshooting prose) (gemini-extension.json:1-5) **V**. Goose's plugin installer accepts Gemini-format extensions that contain `skills/*/SKILL.md` (goose@1.36 crates/goose/src/plugins/formats/gemini.rs:11, 24-60, 102-121; tried after Open Plugins, plugins/mod.rs:266-278) **V**. The hcom root has none of the Open Plugins manifests (`.goose-plugin/plugin.json`, `.plugin/plugin.json`, `plugin.json`) or component markers (`hooks/hooks.json`, `commands`, `agents`) (goose@1.36 plugins/formats/open_plugins.rs:14-21, 40-55) **V**, so the Gemini path would be the one taken **I**. So `goose plugin install https://github.com/aannoo/hcom` would probably install that one skill, with no hooks **I** (not tried, and not documented by hcom). It would teach Goose about hcom, but it adds no delivery or wake.

## 5. Token cost

All estimates are **I**, from string sizes in the source.

- **Join:** the adhoc bootstrap is printed into context once. The constants `IDENTITY_HEAD`, `MESSAGES`, `CAPABILITIES`, `RULES` and `IDENTITY_TAIL` total about 3.0 KB of template before placeholder expansion (src/bootstrap.rs:68-125; all included for adhoc, :565-603), plus `DELIVERY_ADHOC` (about 1.0 KB, :218-242), `INLINE_SEND_NOTICE` (:244) and an active-instances snapshot of up to 8 entries (:288). That makes roughly 1.0k-1.4k tokens, against the README's figure of about 700 tokens for the hooked primer (README.md:231).
- **Per message (listen path):** the model's tool call (`hcom listen --name luna --timeout 86400`, about 25-40 tokens with Goose's tool-call JSON), the stderr banner (about 15 tokens), the framing `[intent:thread #id] a -> b: ` (about 10-15 tokens) and the body. Then comes the real cost: **one full-context provider call for every returned batch**, inside an ever-growing turn. The 50-message batching amortises this only when messages arrive in bursts.
- **Per message (piggyback path):** a box of 3 lines × 40 `─` characters (about 30-40 tokens) plus the framing and body. Its tool call was already being made.
- **Quiet turn:** zero for a Goose session that is not listening, since nothing is injected. In CONNECTED MODE, every listen timeout costs one tool result (`[Timeout: no messages after Ns]`) plus a provider call to re-issue listen. With a large timeout that is negligible.
- **No priority filtering.** Every message ends a listen and costs an inference, with no threshold or silent sends (overview, "Wake policy") **V**.

## 6. Limitations and bugs

1. Goose is not supported: no launcher, no detection, no hooks, no MCP server (§1) **V**.
2. There is no idle wake. Delivery requires the agent to burn a turn inside a blocking tool call, and the human is locked out of the pane while it does (§3.1) **V/I**.
3. The PTY path for unknown tools is dead code for delivery, because the `prompt_empty` gate can never be satisfied. Only ungated manual `term inject` remains (§3.3) **V**.
4. Delivery is acknowledged on a pipe flush, not on model admission. Goose truncation, cancel or compaction can lose acknowledged mail (§2.4) **I**.
5. There is no per-batch byte cap. On Goose 1.36 a one-line `|`-joined batch defeats the 50-line tail preview; on main the 10 KB preview cap instead hides all but the batch's tail from the model (§2.4) **I**.
6. Identity is carried as `--name` in the model's memory. An hour inactive gets it reaped (§3.2.4) **V/I**.
7. Stderr status banners are model-visible under Goose, because its shell tool returns both streams (§2.2) **I**.
8. The bootstrap rules are imperative prose ("you MUST … repeat") and cost tokens on every join. Compliance is not enforced **V/I**.

## 7. Comparison with what Goose actually offers (from the primer)

How well would hcom's generic mechanism work there, and what would a real Goose integration use instead?

| Goose surface (primer §) | hcom uses it? | Fit for hcom-style delivery | |
|---|---|---|---|
| Shell tool runs `hcom listen` / `hcom send` (§9, §11) | yes (adhoc) | Works; costs a turn and locks the human out; obedience-based | V/I |
| Hooks `UserPromptSubmit`/`PostToolUse` (§3.2) | no | Cannot add context in either version, so hcom's Claude/Codex hook delivery would not port | V (primer) |
| `Stop` block-and-continue (§3.4, main only) | no | The analogue of hcom's headless Claude `Stop` long-poll (overview, injection table). Fixed "policy hook denial" framing, capped at 8 consecutive blocks, fires only at turn end | V (primer) / I |
| `tom` MOIM file `GOOSE_MOIM_MESSAGE_FILE` (§5) | no | In-turn after every tool batch, no tool call. Needs the env var at launch (1.36) and the consumer must clear the file. The closest thing to PostToolUse context | V (primer) / I |
| MCP long-poll tool (§6) | no (no MCP server) | Same semantics as `hcom listen`, with the session id from `AGENT_SESSION_ID` instead of `--name` | I |
| ACP `session/prompt` + `steer` + active-run updates (§7) | no | Clean idle wake and mid-turn delivery with pickup ids, but only for ACP-hosted Goose, not a terminal pane | V (primer) |
| Keystroke typing into `> ` (§9) | not for Goose (`get_input_box_text` has no Goose layout) | Possible with a dim-hint empty-line detector. Must never send Ctrl+C; must avoid approval and elicitation selects | V/I |

Verdict: the generic path works in the narrow sense that a cooperative Goose model can exchange messages. It is strictly worse than what hcom does for Claude, Codex or Pi: no wake from idle, no in-turn delivery except during hcom commands, and an occupied, uninterruptible session **I**.

## 8. Clever ideas worth stealing

1. **Piggyback delivery on every CLI call** for hookless harnesses (src/cli_context.rs:349-382). Any `herdr-threads` command that a Goose agent (or any hookless agent) runs could append its pending rows and advance the cursor after a successful write. Mail then arrives in-turn without hooks, at no extra model call. This generalises our `accept` backlog print (AGENTS.md "Token-efficient communication") **I**.
2. **Ack strictly after `write_all + flush`, forward-only cursor**, with "retry may duplicate, never skips" (src/cli_context.rs:305-331). It is a simple rule that is easy to reason about. We should add admission evidence (for example the next PostToolUse/Stop hook) where Goose truncation can hide output **I**.
3. **`--json` disables piggybacking** (src/cli_context.rs:362-364), so machine output stays pure. Notes go to stderr in JSON mode (src/commands/listen.rs:378-379) **V**.
4. **A quiet poll is not an exit.** Listen timeouts set a non-`exit:` status context so the 60 s exit-reaper does not delete a waiting identity (src/commands/listen.rs:123-133, 820-860) **V**. This is relevant if we ever let a hookless seat wait by long-poll.
5. **The name is printed twice around the bootstrap** so `| tail` cannot hide it (src/commands/start.rs:836-845). It is a small fix for a real failure mode, where duplicate identities appear when a model cannot see its own name **V**.
6. **Arbitrary PTY targets never inherit a known tool's delivery behaviour** (src/main.rs:162-175, src/pty/mod.rs:74-78). This fail-closed default matches our "degrade only the broken capability" rule. Our Native wake for an unknown composer layout should likewise refuse rather than guess **V/I**.

What not to copy: imperative "CONNECTED MODE" prose that parks a model in an endless tool call. For Goose, our path is the `tom` MOIM file for in-turn delivery, plus a Native typed content-free wake with a Goose composer reader for idle panes (primer §12 mapping notes). The Stop block on main is a secondary turn-end path **I**.

## Verification

Adversarial pass, 2026-10-23, against the hcom clone at `de33e31a0cdb`, the Goose trees `goose-1.36` and `goose` (main) under `scratchpad/goose-src/`, the Goose primer, and the GitHub API (597 stars, 94 forks, MIT: confirmed). Nothing was executed.

Confirmed as written: no `goose` match anywhere in hcom; README table (14 tools plus "Anything else", README.md:174-192), `hcom start` prompt (README.md:111-118), ~700-token primer (README.md:231); `is_launch_tool` and unknown-command path; `detect_tool` falls back to `Adhoc`; `start_bare` adhoc selection and the doubled `[hcom:<name>]`; `DELIVERY_ADHOC` and `INLINE_SEND_NOTICE` text and selection; identity gate error text; `listen` loop (86400 s default, 30 s waits, 100 ms fallback, returns on first batch, stderr banners, adhoc-only status); text and JSON formatters; router piggyback (`inline_receiver` adhoc-only, `--json` skip, 40-char rules, called from src/router.rs:1083, 1121); 50-message prefix, 1 MiB body cap, ack after `write_all + flush`, forward-only cursor; `wake_all` and `WakeKind::Listen`; reaper test (`cleanup_stale_instances(db, 3600, 3600)`, quiet listen not an exit); adhoc gates; `PtyTarget::AdhocCommand` with no ready patterns; `term inject` ungated; Goose `ShellOutput` with stdout and stderr, 2000-line/50,000-byte limits, 50-line preview, `timeout_secs` only when passed, `DEFAULT_MAX_TURNS = 1000`, Ctrl+C-only task while busy, Gemini-format plugin install after Open Plugins; primer facts used in §7 (no hook context field, Stop block cap 8 on main, `tom` MOIM file, `AGENT_SESSION_ID`, ACP steer).

Changed:
- §2.1: the Goose spawn citation pointed at `resolve_login_shell_path` (shell.rs:222); corrected to `run_command`, shell.rs:506-518. Added hcom's identity resolution order.
- §1: the "only MCP hits" claim was too narrow; extended the range to integration_spec.rs:659-1216 and listed the other non-server hits. The "no MCP server" conclusion stands.
- §2.4 and §6.5: Goose main caps the truncation preview at 10,000 bytes (`OUTPUT_PREVIEW_BYTES`); the "~50 MiB per wake" claim now applies to 1.36 only.
- §2.2: added the `arrived_since_note` trailer line.
- §3.3: `onidle` waits on a watched agent's `listening` status before injecting, so it is not literally ungated; `term inject` is. Tightened the gate wording (the idle check comes first; `Tool::Adhoc` also yields `None`).
- §4: `gemini-extension.json` also has a description; added the Open Plugins manifest check that makes the Gemini path the one taken.
- §5: the identity template is about 3.0 KB at bootstrap.rs:68-125, not 5 KB at :20-125; token estimate lowered to 1.0k-1.4k.
