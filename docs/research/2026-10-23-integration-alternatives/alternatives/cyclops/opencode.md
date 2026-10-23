# Cyclops x OpenCode: message injection and wake

- Upstream: https://github.com/cyclops-team/cyclops. Clone read (untrusted, never executed): `scratchpad/alts/cyclops` at commit `a84d44b8f5476c10ecf849d46370d2cb0238dab9` (2026-09-06). Cyclops 1.1.0, MIT. 14 stars, 0 forks, last push 2026-09-08 (from `gh api`, recorded in `overview.md`).
- OpenCode source used to check how the TUI reacts to what Cyclops sends: the `v1.18.1` tag at `scratchpad/harness-src/opencode-1.18.1` (https://github.com/anomalyco/opencode). The harness background is in `../../primers/opencode.md`.
- Paths are relative to each clone root. Each claim is tagged **VERIFIED** (read in source or docs) or **INFERRED**. Nothing was run.

## Support status

**Generic and unverified.** OpenCode has a detection manifest and nothing else: no hook, no plugin, no MCP server and no composer rule.

- The manifest's own header says so. VERIFIED, `resources/manifests/opencode.toml:1-26`:
  - `version_tested = "unverified"`.
  - "No OpenCode binary was available on the evidence host."
  - "OpenCode has no shell-command hook … its lifecycle surface is a JavaScript plugin (`chat.message`, `session.idle`, `tool.execute.before`, ...) … Cyclops ships no plugin. This manifest binds the pane so the agent's own `cyclops send` resolves its identity, and the skill is seeded; hooks are a follow-up that needs a live OpenCode."
- The manifest declares:
  - `[hooks] config_mechanism = "none: …"` (`:36-37`);
  - one rule, `title_useless`, with `state = "unknown"` (`:39-45`);
  - an `[injection]` block: `load-buffer + paste-buffer -p`, `submit = "Enter"`, `verify_before_submit = true`, `safe_states = ["idle"]`, `unsafe_states = ["blocked_modal","blocked_permission","idle_with_input"]` (`:47-53`).
  - It has no idle, working, `idle_with_input`, modal, permission or quota rule, and no `composer_prompt` or `composer_continuation`. VERIFIED: that is the whole file.
- The project documents the same status. VERIFIED:
  - `STATUS.md:44-49` lists OpenCode among the seven unverified manifests.
  - `docs/reference/hooks.md:99-105`: "Those manifests bind the pane and seed the skill, nothing more."
  - `docs/development/NEXT.md:9-11`: "Measure a composer rule for each unverified manifest (… OpenCode …) … Until then a doorbell there is effectively raw."
  - `CHANGELOG.md:69-72`: "they bind the pane and recognize startup dialogs and nothing more". For OpenCode even that is not true, because the manifest has no dialog rule.
  - A test pins the absence of hooks: `src/cyclops-manifest/tests/shipped_rules.rs:233-239` asserts that `ack`, `turn_start` and `turn_end` are all `None` for OpenCode.
- OpenCode derivatives get the same treatment. Kilo has "the same surface as opencode.toml", with no wiring (`resources/manifests/kilo.toml:13-17`, VERIFIED).

So the only OpenCode-specific data are a process name and the generic paste path. The rest of this report traces what that generic path does when it meets the OpenCode 1.18.1 TUI.

## Injection mechanism

### Channel

The pane is the only channel. The daemon pastes one "doorbell" line into the OpenCode composer through tmux and presses Enter, so the line becomes a user turn. The model then runs a CLI command to fetch the body. No OpenCode API, plugin hook or MCP path is used.

### Code path (VERIFIED by reading)

1. **Accept.** `msg.send` fsyncs the message and one mailbox entry per recipient (`docs/development/DELIVERY.md:38-44`). The work then goes to one FIFO worker per recipient (`:45-49`).
2. **Gate.** `gate()` loops on `admit()` (`src/cyclopsd/src/delivery/gate.rs:1086-1196`). For an OpenCode pane, `admit()` (`gate.rs:1203-1323`) does the following:
   - Pane present, not dead, not in copy-mode (`:1209-1222`).
   - Manifest bind by `current_command`, argv or vendor ancestry. `opencode` is in both `process_names` and `argv_basenames` (`gate.rs:1224`; `fusion.rs:1639-1678, 2036-2056`).
   - One fresh capture. The state comes out `Unknown`, because the only rule is `title_useless`.
   - Named blocks: only `Dead`, `BlockedQuota`, `BlockedModal` and `BlockedPermission` hold (`gate.rs:1244-1288`). The OpenCode manifest has no rule that can produce any of them.
   - Composer: `composer_is_held` (`fusion.rs:2386-2398`) is true only for a delivery-owned hold (`Staged`, or an owned `TurnStarted`), a `HumanInput` semantic, or `IdleWithInput`. The OpenCode manifest cannot produce the last two.
   - Binding: admit when `binding.leader == binding.agent || det.state != Unknown` (`gate.rs:1306-1315`). With OpenCode's state always `Unknown`, the doorbell is admitted exactly when the foreground process is the `opencode` process the manifest matched. Otherwise it holds with `ForegroundNotAgent`.
3. **Paste.** `attempt_delivery` (`gate.rs:574-813`):
   - It spools the bytes into a per-attempt buffer `cyc-<pid>-<seq>` (`:586-608`).
   - It checks that the occupant has not changed (`:611-615`).
   - It calls `inject()`, which runs `paste_buffer(buffer, pane, true, true)`: bracketed paste, with the buffer deleted after use (`inject.rs:111-145`).
4. **Readback.** Up to 4 captures at 0, 120, 240 and 480 ms (`delivery/mod.rs:72`; `inject.rs:203-226`). For OpenCode the proof can never succeed:
   - `exact_staging_proof` needs `exact_composer_content_from_joined_capture` to return `Visible`.
   - That function returns `Unsupported` when the manifest has no `composer_prompt` or `composer_continuation` (`inject.rs:676-681`).
   - So `exact_staging_proof` is `None` (`inject.rs:528-531`), and `inject()` falls through to "Staging was unobservable. The doorbell proceeds to submit once" (`inject.rs:227-237`).
5. **Enter.** It checks the occupant again, then sends `send-keys Enter` once (`gate.rs:680-713`). Enter is pressed whether or not staging was verified. The fact recorded is `record_submitted_unverified()` (`gate.rs:714-718`).
6. **Receipt.** There is no hook ACK, because there are no hooks. The screen tier `tier2_evidence` needs `working_seen || (changed && id_staged)` (`gate.rs:2758-2772`). OpenCode has no working rule, and `id_staged` is always false. So after `SCREEN_ACK_DEADLINE` = 5 s (`delivery/mod.rs:64`) the attempt ends in `settle_without_receipt`: `notified` with no verifier, cause `no_receipt`, and the composer barrier released (`gate.rs:810, 822-849`). A later `cyclops inbox claim` by the recipient is the only positive receipt (`DELIVERY.md:73-74`).

**Net for OpenCode:** every doorbell is admitted and pasted, Enter is pressed, and the attempt is recorded as `submitted_unverified` and then `notified` with no verifier. The project's docs say this outright (VERIFIED):

- `STATUS.md:103-105`: "Unverified manifests never detect a human draft; deliveries to those panes are effectively raw, with a receipt."
- `DELIVERY.md:105-109` says the same.

The manifest comment "deliveries fail closed (not write-ready) until someone measures the composer" (`opencode.toml:25-26`) is **stale relative to the gate**:

- `write_ready` is a published status flag and a reopen hint (`messaging_runtime.rs:138-171`; `mailbox/service.rs:509-516`).
- `admit()` never reads it.
- `safe_states` and `unsafe_states` are parsed but unused by the gate. The overview found them only in the manifest struct and parser tests (`overview.md:65`).

This settles the open "unknown" in `overview.md:66`: 1.1.0 proceeds, it does not fail closed. VERIFIED by reading `admit()` end to end; not run.

### Payload format

One line (VERIFIED, `DELIVERY.md:129-146`; `skills/cyclops/SKILL.md:196-203`):

```text
[cyclops from implementer to reviewer] The rate limiter is ready for review. | cyclops inbox claim m-att_--AAAAAAQACAAAAAAAAAAQ
```

The model runs the claim through OpenCode's bash tool and gets the framed envelope (VERIFIED, `SKILL.md:270-283`):

```text
[cyclops cyc-…] TO: reviewer  FROM: admin  SUBJECT: …
<body>
Reply: cyclops reply cyc-… --summary "…" --body "…"
[cyclops:end cyc-…]
```

### When it fires

Whenever the per-recipient worker reaches the message and `admit()` passes. For OpenCode that means **any time the foreground process is `opencode`**: idle, generating, running a tool, or waiting on a permission or question dialog. It never waits for a turn boundary (VERIFIED, from the gate logic above).

What OpenCode then does with the line depends on its state (VERIFIED in OpenCode source; the end-to-end behaviour is INFERRED):

| OpenCode state | What happens |
|---|---|
| **Idle, empty composer** | The paste lands and Enter submits a new user turn. |
| **Busy (generating or running a tool)** | The TUI has no busy check on submit (`packages/tui/src/component/prompt/index.tsx:946-1079`). The server persists the message, the TUI labels it ` QUEUED `, and the running loop picks it up at the next step boundary (primer §3). No ESC is sent, so the turn is not interrupted. This is a "steer" delivery. |
| **Human draft present** | `onPaste` inserts at the cursor (`index.tsx:1214`), and Enter submits the human's draft and the doorbell together. |
| **Permission or question dialog open** | The doorbell is lost and the Enter keypress acts on the dialog (failure modes 1-2). |

### Size bounds

- **Summary.** A derived summary is cut to one line of at most 200 characters. A sender-supplied summary has no cap; the CLI warns above 160 characters (VERIFIED, `DELIVERY.md:40-44`).
- **Body.** Unbounded in the pane path, because the body never enters the pane. When claimed, it arrives as bash tool output. OpenCode's tool-output truncation applies there (primer §5, INFERRED for bash).
- **OpenCode paste chip.** A paste of 3 or more lines, or more than 150 characters, is replaced in the composer by a `[Pasted ~N lines]` chip (`index.tsx:1205-1212`, `pasteText` at `:1148-1180`), and the text is expanded on submit (`index.tsx:35, 435`). VERIFIED.
  - The chip is on by default and can be turned off (`paste_summary_enabled` / `experimental.disable_paste_summary`, `index.tsx:1206-1208`). VERIFIED.
  - Fixed overhead: the documented example's header `[cyclops from implementer to reviewer] ` is 39 characters and ` | cyclops inbox claim m-att_<22>` is 51, so that 127-character example stays under 150. With labels that long, any summary over ~60 characters produces a chip. How often real doorbells cross the line is unknown.
  - The model still receives the full text. Cyclops' readback could not see it anyway: no OpenCode `composer_chips` are declared.

### Framing

- One header `[cyclops from X to Y]`, then the summary, then the claim command. The skill explains the flow once (`SKILL.md:195-240`).
- The skill's description tells the model to use it "when a line starting with `[cyclops from` appears in your composer" (`SKILL.md:3`, VERIFIED).
- OpenCode lists skills in the system prompt and loads the body on demand (primer §6). So the doorbell is what prompts the skill load.
- The line arrives as an ordinary user message. OpenCode cannot mark it `synthetic` or distinguish it from human typing (INFERRED; the paste path has no metadata).

### De-duplication and ack

- **Claim.** A repeat claim returns the same payload and "creates no second task" (`SKILL.md:278-279`).
- **Withdrawal.** A claim made before the write boundary withdraws the doorbell, and no pane bytes are written (`DELIVERY.md:148-155`).
- **Receipt.** OpenCode has no hook ACK, so the receipt comes from the claim, or is "notified, no verifier".
- **Completion** is a separate fact, set only by an explicit reply (`README.md` "These are different facts", per `overview.md:79`).
- **No timer retries.** Pre-write failures re-enter the gate once (`delivery_retry_max` = 1). Post-write outcomes are never retried (`DELIVERY.md:76-92`). A doorbell OpenCode swallowed is therefore never re-sent (see Limitations).

## Wake mechanism

**The doorbell is the wake.** Paste plus Enter submits a user turn: "This is the wake-up signal, not the complete message" (`SKILL.md:209-211`, VERIFIED). No OpenCode-native wake is used:

- not `prompt_async`;
- not a plugin `session.idle` handler;
- not `opencode run --attach`;
- not the 2.x `synthetic`/`resume` API.

### Idle and busy detection

None for OpenCode.

- The state is always `Unknown`: no screen rules, and no hooks to report lifecycle (`opencode.toml`; `shipped_rules.rs:233-239`).
- Busy panes are not distinguished from idle ones, and get the doorbell mid-turn. This is by design for all vendors: "A working agent gets its doorbell during the turn; the vendor queues the line" (README "The composer rule", per `overview.md:64`).
- On OpenCode, queuing a line mid-turn is real native behaviour (primer §3: `runner.ts:115-122`, `prompt.ts:1092,1111-1116`). So a busy OpenCode agent gets the doorbell at its next step boundary. INFERRED end to end.

### Draft safety

**None on OpenCode.** VERIFIED:

- `STATUS.md:103-105`; `NEXT.md:9-11`.
- `composer_is_held` can only be true for OpenCode through a delivery-owned hold (`fusion.rs:2391-2397`). That hold serialises Cyclops' own doorbells for the ~0.5 s readback plus the 5 s receipt window, and nothing more.
- A doorbell left unsent in the composer (Enter swallowed) does not hold the next one either. "A doorbell left in the composer reads as human input and holds; an unreadable composer does not" (`gate.rs:817-821`), and OpenCode's composer is unreadable to Cyclops.
- A second doorbell is therefore appended to the first. INFERRED.
- The general race also applies: "A person can type between the final capture and the tmux write; there is no input lease" (`STATUS.md:106-108`).

### Latency

- The gate is event-driven: tmux control-mode pane events, no polling (`STATUS.md:11-13`; `gate.rs:1161-1194`). An OpenCode pane never holds, so the gate passes on the first evaluation.
- Paste to Enter is bounded by the readback ladder, about 480 ms plus capture round trips, because OpenCode verification always runs to the last delay (`delivery/mod.rs:72`; `inject.rs:203-226`). INFERRED arithmetic.
- The worker then waits up to 5 s for a receipt that never comes (`delivery/mod.rs:64`). Per-recipient FIFO throughput is therefore about one doorbell per ~5.5 s unless the recipient claims sooner. INFERRED from the worker shape.
- The wake itself is immediate on Enter, when the doorbell lands in an idle composer.

### Failure modes

All INFERRED from VERIFIED code on both sides; none was observed running.

1. **Permission dialog: the doorbell approves a pending tool call.**
   - While a permission request is pending, the session view hides and disables the composer: `visible = … && permissions().length === 0 …`, `disabled = permissions().length > 0 || …` (`packages/tui/src/routes/session/index.tsx:235-236`), and renders `PermissionPrompt` instead (`:1283-1288`).
   - The permission prompt's options are `{ once: "Allow once", always: "Allow always", reject: "Reject" }` (`permission.tsx:405`). The default selection is `keys[0]`, which is `once` (`permission.tsx:536-538`). `return` runs `props.onSelect(store.selected)` (`permission.tsx:607-611`).
   - The OpenCode manifest has no `blocked_permission` rule, so the gate admits. The bracketed paste has no composer to land in, and Cyclops' Enter **grants "Allow once"** to whatever tool call the agent was waiting on.
   - The doorbell text is lost. The attempt still settles as `notified` with no verifier after 5 s, and is never retried. The message sits unclaimed in the mailbox until someone runs `cyclops inbox list`.
   - This is the most serious defect in the pairing: an unrequested approval, and a silently stranded message.
2. **Question dialog.** `QuestionPrompt` replaces the composer the same way (`index.tsx:1289-1294`). VERIFIED in `packages/tui/src/routes/session/question.tsx`: the selection starts at option 0 (`:25-30`), `return` runs `selectOption()` (`:280`), and for a single single-select question `pick()` replies at once with that first option (`:64-79, 105-125`). For a multi-question request it records the first answer and moves to the next tab without submitting (`:81-82`). Only when the question has no listed options (so index 0 is the custom-text entry, `:38`) does Enter open editing instead (`:106-117`). So the doorbell's Enter answers the agent's question with option 1 in the common case. INFERRED end to end.
3. **Subagent view.** When the shown session has a `parentID`, the composer is not rendered (`index.tsx:235, 1295-1298`). The paste goes nowhere and the doorbell is lost (same stranding as 1).
4. **Human draft.** The draft is submitted together with the doorbell (see the table above).
5. **Autocomplete open** (e.g. a human mid-`@file`). `submit()` refuses while `auto()?.visible` (`index.tsx:958`). The doorbell stays in the composer, and the next doorbell is appended to it.
6. **npm install.** The launcher may leave `node` in the foreground (`opencode.toml:17-23`).
   - Binding then depends on `argv_bound_manifest`/`manifest_for_argv` matching the script basename `opencode` ("the script behind an interpreter first", `fusion.rs:2047-2056`).
   - If the matched process is not the foreground leader, `admit()` holds with `ForegroundNotAgent` forever, because the state is never non-`Unknown` (`gate.rs:1306-1317`). After `gate_hold_notify_ms` (120 s) it pings the admin once.
   - Pinning the pane with `cyclops name %N <label> --manifest opencode` (`fusion.rs:1644-1667`) fixes the manifest choice, but not the leader/agent comparison. Unknown which case a real npm install hits.
7. **`--mini` / `opencode run -i` UIs.** Different composers; not analysed (unknown).

## Setup and installation footprint on OpenCode

- **Writes no OpenCode config:**
  - nothing to `~/.config/opencode/opencode.json`, `tui.json`, `plugins/` or `.opencode/`;
  - no MCP entry;
  - no plugin.
  
  VERIFIED: `opencode.toml:36-37`; `hooks.md:99-105`; no OpenCode entry among the wired files in `hooks.md:115-121`.
- **The one write is the skill**, `~/.agents/skills/cyclops/SKILL.md`, shared with Codex, Cursor, Gemini, goose, Amp and Crush. VERIFIED: `docs/guides/install.md:184-185, 535`; `src/cyclops/src/skillseed.rs:233-239`; path from `src/cyclops/src/consumer.rs:132-137`.
  - **Consent:** only under `--wire-hooks` (which `scripts/install.sh` passes), honouring `CYCLOPS_NO_VENDOR_HOOKS` (`skillseed.rs:13-22`; `hooks.md:115-130`).
  - **Precondition:** only when `~/.config/opencode` exists (`install.md:190-192`).
  - **Ownership checks:** only into an owner-controlled parent (`skillseed.rs:535-560`).
  - **Never overwrites:** existing files and operator edits are left untouched, with FNV-1a hashes of every released body distinguishing old seeds from edits (`skillseed.rs:34-90`).
- **Possible duplicate skill.** OpenCode also reads `~/.claude/skills`, where Cyclops seeds a second copy for Claude Code (primer §6; `hooks.md:122`). On a machine with both, OpenCode sees two `cyclops` skills, logs `duplicate skill name`, and the later one wins (`packages/opencode/src/skill/index.ts:125-134`). Harmless but noisy. INFERRED.
- **Identity.** Cyclops reads no OpenCode session id and ships no `shell.env` plugin (OpenCode has that plugin hook; by default no session id reaches the bash tool's environment, primer §10). The sender is derived from kernel peer credentials plus an ancestry walk. A `cyclops` command run by OpenCode's bash tool is a descendant of the `opencode` process: the TUI hosts the server in a Worker of the same process (primer §1), so it sends as the pane (`DELIVERY.md:257-266`). INFERRED for OpenCode specifically.
  - One TUI process can host many OpenCode sessions (primer §10). Cyclops binds the pane, not the session, so the doorbell goes to whichever session is shown. INFERRED.

## Token cost

**Per message.** INFERRED estimates:

| Item | Cost |
|---|---|
| Doorbell user turn | ~40-110 tokens: 60-300 characters |
| Claim tool call | ~30 tokens |
| Envelope | body plus ~60 tokens of header, reply hint and end sentinel |
| Turn overhead | a full model turn (reasoning, then the claim) before any useful work, plus the reply |

- Mid-turn on a busy agent, the doorbell becomes an extra queued user message, which extends the turn.
- There is no priority or threshold, so **every message wakes**, including FYI traffic (`overview.md:68`).
- The first doorbell of a session also triggers loading the skill body: 24,772 bytes, about 6k tokens (`wc -c skills/cyclops/SKILL.md`, VERIFIED size). The skill is loaded on demand through OpenCode's skill tool (primer §6).

**Per quiet turn.** Zero bytes injected: no hook, no plugin, no MCP. The standing cost is the skill's name and description in OpenCode's skill listing, about 120 tokens per request (`SKILL.md:3`; INFERRED from OpenCode listing skills in the system prompt).

## Limitations and bugs

1. **Enter approves OpenCode permission prompts** (failure mode 1). The root cause is the gate's "proceed unless a named block is positively seen" policy combined with a manifest that names no blocks.
2. **Doorbells silently stranded.** Any state where the paste has no composer (permission, question, subagent view) still records `notified`, and nothing re-sends. Recovery is manual (`cyclops messages`, `inbox list`).
3. **No draft guard.** Documented (`STATUS.md:103-105`), but on OpenCode it means submitting a human's half-typed prompt.
4. **Stale and contradictory docs.** VERIFIED:
   - `opencode.toml:25-26` says "fail closed". The code and `DELIVERY.md:105-109` proceed.
   - `safe_states = ["idle"]` is declared but not enforced.
   - `CHANGELOG.md:71-72` says the unverified manifests "recognize startup dialogs", but the OpenCode manifest has no dialog rule.
5. **The pane is the only channel, while OpenCode has a good programmatic one.**
   - Cyclops ignores everything OpenCode offers for in-process delivery: plugin `chat.message`, `tool.execute.after`, `experimental.chat.messages.transform`, `session.idle` + `client.session.promptAsync`, `noReply`, and on 2.x `synthetic` with `delivery: steer|queue`.
   - All of these avoid the composer entirely (primer §2-3, §9).
   - The manifest says why: a plugin is code in the OpenCode process and nothing was run live (`opencode.toml:9-15`).
6. **Unmeasured detection.** The paste chip, the composer border and the title are all unmeasured, and the npm `node` foreground is unmeasured. Readback verification is structurally impossible until `composer_prompt` and `composer_continuation` are measured.
7. **Throughput.** ~5.5 s per doorbell per recipient, because every attempt waits out the 5 s receipt window. INFERRED.
8. **Project activity.** No commits since 2026-09-06 (`overview.md:94`). OpenCode has since moved from 1.18 to 1.19.4 and dev 2.x (primer §0), so the unverified manifest is drifting further from reality.

## Clever ideas worth stealing

- **Honest receipt vocabulary for an unverifiable harness.** `submitted_unverified` and `notified` with no verifier are journalled distinctly from a verified receipt (`gate.rs:714-718, 822-849`). When we add OpenCode, a keystroke fallback should record the same honesty, instead of a single "delivered" bit.
- **Positive-evidence-only holds, but pair them with positive blocks.** Cyclops' rule ("an unreadable composer proceeds") is right for liveness and wrong without block rules. The OpenCode lesson for us: a Herdr native wake into OpenCode must positively recognise the permission and question prompts and refuse them, because Enter there is an approval.
  - The safer design is to avoid the composer entirely: plugin `promptAsync` while idle, steered or queued messages while busy (primer §12 shapes 1-2).
- **Withdraw on early claim.** If the recipient reads the mail before the wake is written, no bytes are written (`DELIVERY.md:148-155`). This maps onto our wake path: re-check the frontier under the same lock immediately before typing.
- **One shared skill seed with operator-edit detection.** Seeding `~/.agents/skills/` once for every harness that reads it, with hashes of released bodies to tell old seeds from operator edits (`skillseed.rs:34-90`), is a tidy install pattern. OpenCode reads `~/.agents/skills` and `~/.claude/skills`, so our existing skill may already reach it (primer §6).
- **Manifest as data, with negative tests.** `shipped_rules.rs:233-237` asserts what an unverified vendor must *not* claim (no ACK, no turn hooks). That is a cheap guard against a docs-only adapter overclaiming capabilities.

## Verification

Adversarial pass, 2026-10-23, reading the Cyclops clone at `a84d44b8` and OpenCode `v1.18.1`; nothing executed.

- Checked and confirmed: the whole `opencode.toml` (1-53) and `kilo.toml:13-17`; `STATUS.md:44-48, 103-108`; `hooks.md:99-105, 115-130`; `NEXT.md:9-11`; `CHANGELOG.md:69-72`; `shipped_rules.rs`; `gate()`/`admit()` (`gate.rs:1086-1196, 1203-1323`, including the `leader == agent || state != Unknown` binding test and `ForegroundNotAgent`); `attempt_delivery` (`:574-718`), `settle_without_receipt` (`:815-849`), `tier2_evidence` (`:2758-2772`), `latch_hold`; `inject.rs` paste, readback ladder and `Unsupported` fallthrough; `delivery/mod.rs` constants (5 s, `[0,120,240,480]`); `config.rs` defaults (`delivery_retry_max = 1`, `gate_hold_notify_ms = 120000`); `fusion.rs:2386-2398, 1639-1678, 2036-2056`; `write_ready` absent from `delivery/gate.rs` and used for reopen in `messaging_runtime.rs:138-171`, `mailbox/service.rs:509-516`; `safe_states` read only by the manifest struct and tests; `DELIVERY.md` sections cited; `SKILL.md` lines cited and 24,772-byte size; `install.md:184-192, 535`, `skillseed.rs`, `consumer.rs:132-137`; repo stats (`gh api`: 14 stars, 0 forks, pushed 2026-09-08, MIT, head commit 2026-09-06). OpenCode: session `index.tsx:235-236, 1283-1298`; `permission.tsx:405, 408-425, 536-538, 607-611`; prompt `index.tsx:35, 430-437, 946-965, 1148, 1205-1214`; `skill/index.ts:125-134`.
- Changed: `shipped_rules.rs` range corrected to 233-239. The paste-chip estimate rewritten: the documented 127-character example does not chip, so "most doorbells chip" was unsupported; the chip's config switch added. Failure mode 2 (question dialog) upgraded from unknown to read-in-source: Enter picks option 1. The identity bullet corrected: OpenCode does have a `shell.env` plugin hook; Cyclops just does not use it.
- Not checked: the primer's own OpenCode citations (`runner.ts`, `prompt.ts`, queued-prompt rendering), which this report takes from the primer; whether a bracketed paste reaches any handler while the permission or question prompt is shown (the "doorbell text is lost" claim stays INFERRED).
