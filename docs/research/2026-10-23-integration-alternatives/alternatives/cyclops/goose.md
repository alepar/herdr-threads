# Cyclops x Goose (Block): message injection and wake

- Alternative: Cyclops, <https://github.com/cyclops-team/cyclops>, clone at commit `a84d44b8f5476c10ecf849d46370d2cb0238dab9` (2026-09-06), version 1.1.0. Read only; nothing was built or run.
- Harness: Goose, primer `../../primers/goose.md`. Goose sources: `goose@1.36` (tag v1.36.0, matches the local binary) and `goose@main` (1.56.0). Both are shallow clones under `scratchpad/goose-src/`.
- Paths without a prefix are relative to the Cyclops clone root. `goose@...` paths are relative to the Goose clones.
- Tags: VERIFIED means read in source or docs at the cited place. INFERRED means my reading, not stated anywhere. "unknown" means not determined.

## Support status

**Generic and unverified, with hook wiring. Nothing has been measured against a real goose.** VERIFIED:

- The manifest header reads: "UNVERIFIED best effort from primary documentation. No goose binary was available on the evidence host, so this file binds the pane and declares the documented hook contract and nothing more." (`resources/manifests/goose.toml:1-8`). It also sets `version_tested = "unverified"` (`goose.toml:31`).
- The hook template says the same: "No edge from a goose turn has been observed by Cyclops yet." (`resources/hooks/goose/hooks.json.tmpl:6-8`). So do the install instructions: "This vendor is unverified: no goose edge has been measured yet." (`src/cyclops/src/hookset.rs:668-684`).
- Goose is one of seven "reviewed" unverified manifests. `src/cyclops-manifest/tests/shipped_rules.rs:106-143` pins what such a manifest may contain:
  - no screen rule except `unknown` or `blocked_modal`;
  - no `composer_semantic` on any rule;
  - no `auto_dismiss`;
  - empty `clear_keys`;
  - `verify_before_submit = true`.
- The manifest ships exactly one screen rule, `title_useless` with state `unknown` (`goose.toml:50-56`). It has no composer prompt or continuation patterns, no idle, working or modal rules, and no turn-end declaration.
- What ships for goose (`CHANGELOG.md:69-72`, `docs/guides/install.md:485-502`):
  - the manifest, bound by process name `goose` and launched as `goose session` (`goose.toml:32-35`);
  - a hook-only Open Plugin template;
  - automatic hook wiring;
  - the shared skill at `~/.agents/skills/cyclops/SKILL.md`.
- Cyclops's own backlog item 1 is to "Measure a composer rule for each unverified manifest (... goose ...) so a doorbell to those panes can detect a human draft. Until then a doorbell there is effectively raw." (`docs/development/NEXT.md:9-11`). VERIFIED.

### The docs conflict, resolved from source

The manifest comment says "No idle or working rule ships, so deliveries fail closed (not write-ready) until someone measures the composer" (`goose.toml:25-26`).

The code does not do that. The delivery gate `admit` (`src/cyclopsd/src/delivery/gate.rs:1203-1323`) never reads the `write_ready` verdict. VERIFIED. Its steps:

1. Pane present, alive, not in copy-mode (`gate.rs:1209-1222`).
2. A manifest binds the pane (`gate.rs:1224-1226`). Goose binds by process name.
3. One fresh capture (`gate.rs:1230-1241`).
4. Named blocks: dead, quota, modal, permission (`gate.rs:1244-1288`). Goose has no rules that could produce any of these.
5. `composer_is_held` (`gate.rs:1294`). It returns true only for a delivery-owned hold, a `HumanInput` composer semantic, or `IdleWithInput` (`src/cyclopsd/src/fusion.rs:2386-2398`). Goose has no rule that yields either, so outside a delivery-owned hold this is always false.
6. Binding: admit when `binding.leader == binding.agent || det.state != AgentState::Unknown` (`gate.rs:1306-1315`).

`write_ready` is computed (`src/cyclops-proto/src/state.rs:761-820`) and is false for goose: there is no clean-composer screen evidence. VERIFIED. The consumers I found (`grep -rn write_ready src/cyclopsd/src`) are:

- reopening an attempt that is already parked as `WriteReadinessChanged` (`src/cyclopsd/src/messaging_runtime.rs:140-171,373`, `src/cyclopsd/src/mailbox/service.rs:509-517`);
- a "receipt-latency hint, never write authority" (`messaging_runtime.rs:173-189`; `fusion.rs:546-556`);
- the readiness-change broadcast (`fusion.rs:2400-2412`, `readiness_key`) and status display.

None of them is on the `admit` path.

INFERRED from that code: a fresh doorbell to a goose pane whose foreground process is goose is admitted and written, whether goose is idle or busy. Everything except the manifest comment agrees:

- "Ambiguous or absent composer evidence does not hold a doorbell" (`docs/development/DELIVERY.md:10-16`).
- "the seven unverified ones do not [carry a human-input rule], so a doorbell to one of those panes never detects a draft and is effectively a raw write with a receipt" (`DELIVERY.md:105-109`).
- `STATUS.md:104-106`, `docs/guides/install.md:485-489` and `skills/cyclops/SKILL.md:236-238,247-249` say the same.

The goose manifest comment is stale. VERIFIED that the two disagree; INFERRED which one is current, by reading the code.

## Injection mechanism

### Exact mechanism

A tmux paste of one "doorbell" line into goose's `rustyline` prompt, then one Enter. Goose reads it as a user prompt, and the model has to run `cyclops inbox claim m-att_<token>` through its shell tool to get the body. VERIFIED for Cyclops in general (`DELIVERY.md:127-146`; `skills/cyclops/SKILL.md:195-234`). INFERRED for goose, because it was never exercised.

Hooks never carry content. For goose nothing else could: Goose hooks have no context-injection output (primer section 3.4). Cyclops's `cyclops hook` is "fast, silent, exit 0 no matter what" (`src/cyclops/src/hook.rs:1-6,37-43`). It prints nothing, so goose reads every event as allow, including the blocking `PreToolUse` and the main-only blocking `Stop` (`hooks.json.tmpl:20-23`). VERIFIED. Cyclops uses neither of goose's model-visible surfaces: the `tom`/MOIM file and the main-only Stop-block reason. Nothing in the clone mentions MOIM, `GOOSE_MOIM_MESSAGE_FILE` or ACP. VERIFIED by `grep -ri goose` over the clone; every hit is listed in the support section's sources.

### Code path

1. **Accept.** `msg.send` fsyncs the message plus one mailbox entry per recipient before it answers (`DELIVERY.md:38-44`). VERIFIED.
2. **Queue.** One FIFO worker per recipient (`DELIVERY.md:45-49`; `src/cyclopsd/src/delivery/worker.rs`). VERIFIED.
3. **Gate.** `admit`, as described above. For goose the only real checks are: pane alive, not in copy-mode, process named `goose`, foreground leader is goose, and no delivery-owned hold. VERIFIED code; INFERRED goose outcome.
4. **Spool.** `load-buffer` into a per-attempt tmux buffer from a 0600 spool file (`src/cyclopsd/src/delivery/inject.rs:78-105`). VERIFIED.
5. **Write boundary.** `on_write` claims the composer barrier and records `writing` with the binding (pane root, leader, agent, manifest), then `paste-buffer -p -d` runs (`gate.rs:616-657`; `inject.rs:111-130`). `-p` is bracketed only "when the app opted in" (`inject.rs:80-81`). VERIFIED.
   - Goose's `rustyline` likely enables bracketed paste while it reads a line (primer section 9, INFERRED there). While goose is busy, `rustyline` is not reading, so the paste probably arrives unbracketed. INFERRED.
6. **Readback.** Captures at 0, 120, 240 and 480 ms (`VERIFY_DELAYS_MS`, `src/cyclopsd/src/delivery/mod.rs:72`; loop in `inject.rs:188-238`).
   - Each re-read calls `exact_staging_proof` (`inject.rs:512-533`), which needs two things: `staged_representation` = `VisibleTarget`, and `exact_composer_content_from_joined_capture` = `Visible(content)` matching the payload. VERIFIED.
   - The second check always fails on goose. It goes through `exact_composer_content_for_state`, which returns `Unsupported` when the manifest lacks `composer_prompt` or `composer_continuation` (`inject.rs:641-646,676-681`). Goose's manifest has neither. VERIFIED code.
   - The first check would fail as well. With no `IdleWithInput` rule, the exact-row proof needs a screen row that equals the payload byte for byte (`inject.rs:410-440`: `if idle_rules.is_empty() { return p == want || r == want; }`). Goose draws its prompt as `"> "` + buffer (primer section 9), so the row never equals the payload. INFERRED.
   - Result: readback always fails on goose, whatever goose's prompt looks like. Every doorbell is recorded `submitted_unverified` after about 480 ms of re-reads. INFERRED from code. The code comment covers this case: "Staging was unobservable. The doorbell proceeds to submit once and finishes as delivered_unverified." (`inject.rs:228-229`).
7. **Enter.** One occupant re-check, then `send-keys Enter`, pressed exactly once (`gate.rs:676-718`; `goose.toml:60` `submit = "Enter"`). VERIFIED.
8. **Receipt wait.** Up to `SCREEN_ACK_DEADLINE` = 5 s (`mod.rs:64`; `gate.rs:2398-2440`). The ACK section below covers the goose outcome.

### Payload format (what goose's model sees)

```text
[cyclops from implementer to reviewer] The rate limiter is ready for review. | cyclops inbox claim m-att_--AAAAAAQACAAAAAAAAAAQ
```

VERIFIED, `DELIVERY.md:132-146`:

- The header carries up to three recipient labels, then `+N`, or `to all`.
- The token is the 128-bit attempt id in 22 characters.
- The body never reaches the pane.

The claim returns an envelope. VERIFIED: `SKILL.md:270-276`; the CLI renders it in `print_claim_payload` (`src/cyclops/src/main.rs:2992-3018`), which also prints a `Summary:` line when there is one. `src/cyclopsd/src/delivery/terminal.rs:14-28` is a different renderer, the direct-payload format (no `TO:`, reply hint `cyclops send`):

```text
[cyclops cyc-...] TO: reviewer FROM: admin SUBJECT: ...
<body>
Reply: cyclops reply cyc-... --summary "..." --body "..."
[cyclops:end cyc-...]
```

### When it fires

The doorbell is written as soon as the recipient's FIFO head clears the gate. Goose's idle or busy state does not change that, because the gate cannot see it. INFERRED from the gate code above.

- **Idle goose.** The line lands in the `> ` prompt and Enter submits it. This is a normal turn start, and the paste is the wake.
- **Busy goose.** The goose CLI does not read the terminal during a turn (primer section 9: only a Ctrl+C task listens; "Message queuing is not available in the goose CLI"). The pasted bytes and the CR sit in the tty input queue. The cooked-mode tty probably echoes them into goose's streaming output. INFERRED.
  - Whether `rustyline` then consumes the queued line as the next prompt or discards it is **unknown**. It depends on whether `rustyline`'s switch to raw mode flushes pending input; I did not trace that.
  - If it is consumed, the doorbell becomes the first prompt after the turn ends. That would be a delayed wake, not a mid-turn injection. INFERRED.
- **Busy goose at a tool-approval or elicitation select** (`GOOSE_MODE=approve|smart_approve`, MCP elicitation; primer section 9). The pasted text and Enter go to the interactive select. Cyclops has no goose modal rule (`goose.toml` has none, and `shipped_rules.rs:126-139` forbids an unmeasured state), so it does not hold. INFERRED hazard: Enter may confirm whatever option is highlighted.

### Size bounds

- Summary: a derived summary is one line of at most 200 characters. A sender-supplied summary has no cap; the CLI warns above 160 (`DELIVERY.md:40-44`). VERIFIED.
- Pane line: header + summary + about 50 bytes of claim command. About 300 bytes for a derived summary. INFERRED arithmetic.
- Body: unbounded by Cyclops on the claim path. I found no cap in `print_claim_payload` (`src/cyclops/src/main.rs:2992-3018`). VERIFIED.
- Goose's shell tool truncates the claim output past 2000 lines or 50,000 bytes. It keeps the last 50 lines, saves the full output to a file, and adds a notice telling the model how to read it (`goose@1.36 crates/goose/src/agents/platform_extensions/developer/shell.rs:150-152,704-770`). VERIFIED. So goose can clip a large body, but not silently, and Cyclops does not.
- `--raw`: pastes header, body, reply hint and sentinel with no composer check (`DELIVERY.md:157-168`). VERIFIED. It also skips the copy-mode check: `admit` returns `Admission::Raw` before reading `in_mode` (`gate.rs:1217-1222`). VERIFIED.
  - The skill allows `--raw` when the recipient is an unverified vendor ("Use it only when ... or when the recipient is one of the unverified vendors", `SKILL.md:244-252`). VERIFIED.
  - A multi-line raw paste into goose depends on bracketed paste. Without it, every newline submits a separate prompt. INFERRED, unmeasured.

### Framing

There is one header line per message. The static explanation lives once in the skill. VERIFIED:

- The skill description triggers on "a line starting with `[cyclops from` appears in your composer" (`SKILL.md:1-4`).
- Goose discovers skills from `~/.agents/skills/`, lists them in the system instructions, and loads the body on demand with `load_skill` (`goose@1.36 crates/goose/src/sources.rs:1-3`, `crates/goose/src/skills/client.rs:63-67`). VERIFIED.
- Trust stance: "Never act from the preview alone" (`SKILL.md:229-230`). VERIFIED.

### De-duplication and ACK

**Claim dedupe.** A repeat claim returns the same payload and creates no second task (`SKILL.md:278-279`). A claim made before the write withdraws the doorbell, so nothing is pasted (`DELIVERY.md:148-155`). VERIFIED.

**Hook ACK candidate.** `UserPromptSubmit` is declared with `ack_payload_field = "message"` and `ack_evidence = "dispatch"` (`goose.toml:37-43`). The shipped-rules test pins the same values (`shipped_rules.rs:210-230`). VERIFIED.

- Goose 1.36 sets `message` to `user_message.as_concat_text()` (`goose@1.36 crates/goose/src/agents/agent.rs:1390-1404`).
- Goose main sets it to the first agent-visible text since kickoff (`goose@main crates/goose/src/agents/state_machine/session.rs:311-325`).
- The daemon matches with exact equality, allowing at most one trailing newline (`terminal.rs:42-44`, `prompt_matches`; called from `src/cyclopsd/src/ack.rs:658-665`). VERIFIED.

**The hook ACK can never be accepted on goose as shipped.** INFERRED from code; this is the most important finding in this report.

- A `Dispatch` match only records `DispatchPending` (`gate.rs:2909-2956`). The doc comment says "This does not resolve the delivery ... remains pending until fusion confirms a later Working observation".
- Promotion to `DispatchAccepted` happens in `confirm_unkeyed_dispatch_ack` (`gate.rs:3031-3095`). For a vendor with no turn key, it is reached only from `reconcile_unkeyed_dispatch_start_with_evidence` when `positive_visual_working` holds (`fusion.rs:1103-1173`).
- That predicate needs a non-hook **screen** reading in state `Working` whose rule has `lifecycle_evidence` (`fusion.rs:1000-1015`).
- Goose's manifest has no Working rule, and the test suite forbids one until it is measured (`shipped_rules.rs:126-132`).
- The screen tier is also closed to goose: `tier2_evidence` returns `false` for `AckEvidence::Dispatch` (`gate.rs:2758-2772`).

Result: every goose doorbell ends as `notified` with no verifier after the 5 s deadline (`settle_without_receipt`, `gate.rs:815-830`). The exception is when the model claims the message within that window: "A claim by the exact recipient after Enter is also a receipt" (`DELIVERY.md:73-74`).

**`cyclops hooks selftest` on goose depends on timing.** VERIFIED that selftest "Exit 0 when the ack hook fired, 1 otherwise" (`docs/reference/hooks.md:196-199`). The daemon decides `hook_ack` in one of two ways (`src/cyclopsd/src/selftest.rs:617-650`). VERIFIED code:

- While the worker still holds the attempt, `hook_ack` needs `verified_by == Hook`, which never happens on goose (above).
- Once the worker has retired the attempt, `hook_ack` is true when any `UserPromptSubmit` edge from the current occupant arrived after the test started and the state is resolved.

INFERRED: a correctly wired goose usually exits 1 on the first path, but can exit 0 on the second. Selftest is not a reliable verdict on goose in either direction.

**Draft concatenation also breaks the match.** If a human draft `fix the` is in the composer, the paste appends to it. Goose then submits `fix the[cyclops from ...]`, and `message` no longer equals the payload, so it does not match. INFERRED from `prompt_matches`. Cyclops records this honestly as unverified, but the human's draft has been submitted mixed with the doorbell.

## Wake mechanism

### Exact mechanism

The doorbell paste plus Enter is the wake: "This is the wake-up signal, not the complete message" (`SKILL.md:209-211`). VERIFIED. Cyclops uses no goose-native API, even though goose has three:

- ACP `session/prompt` and `_goose/unstable/session/steer` (primer section 7);
- headless `goose run -r --session-id` (primer section 8);
- the Stop-hook block (main only; primer section 3.4).

All three are absent from the clone. VERIFIED by grep.

### Idle/busy detection

None that can gate a write. VERIFIED:

- **Screen.** Unmeasured, with only `title_useless` (`goose.toml:50-56`). No screen rule can move `det.state` off `Unknown`. VERIFIED. I did not trace whether the candidate hook lane (`fusion.rs:3965-3968`) can change the fused state on its own. If it can, step 6 of the gate would also admit goose while a child holds the foreground. Unknown.
- **Hooks.** `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse` and `Stop` are wired (`hooks.json.tmpl:27-45`).
  - `UserPromptSubmit` is a `candidate` turn start (`goose.toml:39-40`).
  - No turn end is declared, on purpose. Goose `Stop` "may be blocked by a hook and re-fired and shares no turn field with UserPromptSubmit (payloads carry only session_id) ... the loader refuses a candidate end without turn_key_fields" (`goose.toml:18-23`).
  - `Stop` is wired for liveness only.
- The `liveness_admits_idle` path needs a clean idle composer screen rule (`fusion.rs:2414-2437`), so goose never gets it. VERIFIED code; INFERRED goose effect.

So "a working agent ... proceed[s]" applies in full to goose (`DELIVERY.md:96-103`).

**Foreground binding is the only busy-adjacent check.** The gate holds `ForegroundNotAgent` when the foreground leader is not goose and the screen is `Unknown` (`gate.rs:1306-1318`). Goose's shell tool spawns children with stdin null and piped stdout/stderr (`goose@1.36 .../developer/shell.rs:513-516`), so a child never takes the tty foreground. INFERRED: goose stays the foreground leader through a whole turn, and a busy goose is admitted.

### Draft safety

There is none. Cyclops says so plainly. VERIFIED:

- "Unverified manifests never detect a human draft; deliveries to those panes are effectively raw, with a receipt." (`STATUS.md:104-106`).
- "A person can type between the final capture and the tmux write; there is no input lease across that gap." (`STATUS.md:107-109`).

Cyclops does not destroy a draft: `clear_keys` is empty, and the test enforces that (`shipped_rules.rs:140`). That matters because Ctrl+C in goose kills the whole buffer, and a second one exits (primer section 9). The cost instead is concatenation: the draft plus the doorbell are submitted together as one prompt. INFERRED.

The composer barrier stops a second doorbell from stacking on an unconsumed first one, but only until the 5 s receipt deadline releases it (`gate.rs:815-821`). The doc comment says: "A doorbell left in the composer reads as human input and holds; an unreadable composer does not." Goose's composer is unreadable. INFERRED: a doorbell still sitting in goose's tty buffer (busy goose) does not stop the next doorbell after 5 s, so several can queue in the tty.

### Latency

These figures are from code constants. None were measured on goose.

- Idle goose:
  - gate and capture: one tmux capture;
  - paste;
  - 480 ms of readback attempts that always fail (INFERRED above);
  - Enter: goose starts the turn.
  - Wake latency is therefore about 0.5 s plus tmux round trips. INFERRED.
- FIFO throughput per goose recipient: each attempt then waits out the 5 s receipt deadline unless the model claims sooner. So consecutive messages to one goose pane are spaced by about 5.5 s or by the claim latency, whichever is shorter. INFERRED from `mod.rs:64,72` and `gate.rs:771-812`.
- Held attempts (modal and similar) wait for pane events, never timers, and ping the admin once after 120 s (`DELIVERY.md:24,54-56,270-271`). This is VERIFIED but unreachable on goose, because no goose rule produces a hold.

### Failure modes

| Failure | Mechanism | Tag |
|---|---|---|
| Human draft concatenated and submitted | No composer rule, so no `HumanInput` hold | VERIFIED doc + code; INFERRED goose effect |
| Doorbell answers a tool-approval or elicitation select | No modal rule; Enter goes to the cliclack select | INFERRED |
| Busy goose: line echoed into output, maybe lost or maybe deferred to the next prompt | CLI does not read the terminal while busy | INFERRED; deferral is unknown |
| Every receipt is `notified` with no verifier | Dispatch ACK needs a visual Working frame; the screen tier is disabled for Dispatch | INFERRED from code |
| Selftest verdict depends on timing (usually 1) | Same; a fallback path in `selftest.rs:624-649` counts a raw hook edge | INFERRED from code |
| Wrong pane bound | Process-name binding only (`goose.toml:32-33`); `pane_current_command` = `goose` is "inferred, not measured" (`goose.toml:11-12`) | VERIFIED as an unmeasured assumption |
| `GOOSE_PROMPT_EDITOR` set | The composer is `$EDITOR`, so the paste lands in an editor buffer | INFERRED from primer section 9 |
| Daemon restart mid-attempt | `attention_required`, operator `requeue` | VERIFIED `DELIVERY.md:201-210` |
| Context limit < 32k | Irrelevant: Cyclops does not use MOIM | n/a |

## Setup and installation footprint on the harness

- **Prepare** (`cyclops hooks install goose --agent <label>`): renders `$CYCLOPS_HOME/hooks/goose/<label>/hooks.json` only. It "refuses to write into ... any `.agents` ... directory" (`docs/reference/hooks.md:13-40`). VERIFIED.
- **Automatic wiring** (`cyclops start --setup-only --wire-hooks`, run at the end of `scripts/install.sh`; consent marker `~/.cyclops/vendor-wiring-consented`; opt out with `CYCLOPS_NO_VENDOR_HOOKS=1`). Source: `hooks.md:113-145`; `src/cyclops/src/hookset.rs:432-500`; `src/cyclops/src/consumer.rs:200-201,228-247`. VERIFIED.
  - The gate is that `~/.config/goose` exists (`consumer.rs:200-201`, `hookset.rs:442-445`).
  - It creates `~/.agents/plugins/cyclops/hooks/` if missing (`hookset.rs:446-449`).
  - It merges its entries into `~/.agents/plugins/cyclops/hooks/hooks.json`, backing up any existing file once as `*.before-cyclops`.
  - It seeds `~/.agents/skills/cyclops/SKILL.md`, a single copy shared with Codex, Cursor, Gemini, OpenCode, Amp and Crush. It creates only a missing final file below an existing parent.
- **The plugin file** has five events, no matcher, `timeout: 5`, and the command `{cyclops_bin} hook <Event>` (`hooks.json.tmpl:27-45`). VERIFIED.
  - Goose discovers every child directory of `~/.agents/plugins` as a plugin, enabled by default unless listed in `disabledPlugins` (`goose@1.36 crates/goose/src/plugins/discovery.rs:38-90`).
  - `hooks/hooks.json` alone is an Open Plugins component marker (`goose@1.36 crates/goose/src/plugins/formats/open_plugins.rs:14-21`).
  - So the hook-only plugin loads. VERIFIED.
  - Goose must be restarted to pick it up (`hookset.rs:682`; primer section 3.1).
- **No other goose config is touched.** That covers `config.yaml`, `settings.json`, extensions, `GOOSE_MOIM_MESSAGE_FILE` and MCP. VERIFIED by grep.
- **Hook authentication.** No pane id is needed in the payload: `agent.state.report` is accepted only from a process inside the pane, checked by kernel peer credentials and an ancestry walk (`hooks.md:170-184`). Goose runs hooks as `sh -c` children, so the ancestry reaches goose (primer section 3.1). VERIFIED mechanism; INFERRED for goose.

## Token cost

| Item | Cost | Tag |
|---|---|---|
| Per message, the pane line as a user prompt | About 300 bytes, roughly 70-90 tokens, with a derived summary; unbounded with a sender summary | INFERRED from `DELIVERY.md:40-44,132-146` |
| Per message, the claim round trip | One extra model step: a tool call (`cyclops inbox claim m-att_...`, about 30 output tokens), then a tool result carrying the body plus about 150-250 bytes of envelope (header, reply hint, sentinel), then another provider call that re-sends the conversation | INFERRED |
| Per message, the wake turn itself | A full goose turn. Goose 1.36 also adds its MOIM `<info-msg>` to every provider call (primer section 5); that is goose's own cost, not Cyclops's | INFERRED |
| Per message, on a busy goose | An echoed doorbell may land in the next prompt after the turn, which costs a turn the agent may not need | INFERRED |
| Per quiet turn | **Zero model-visible bytes.** Hooks print nothing (`hook.rs:1-6`). Latency only: five hook processes per turn, plus `PreToolUse`/`PostToolUse` per tool call, each `sh -c cyclops hook` with a 3 s budget and "single-digit milliseconds" typical (`hook.rs:29-35`) | VERIFIED |
| Per session | The skill name and description in goose's skill list, about 600 bytes (`SKILL.md:1-4`), plus the 24.8 KB body (about 6k tokens) if the model calls `load_skill` | VERIFIED size (`wc -c` = 24772); INFERRED tokens |
| Priority/threshold savings | None. Every message produces a doorbell and a turn | INFERRED; no priority concept (overview) |

## Limitations and bugs I can see

1. **The manifest contradicts the code.** `goose.toml:25-26` says "fail closed (not write-ready)". The gate does not consult `write_ready` (`gate.rs:1203-1323`), and five other documents say "effectively raw". Anyone reading the manifest would believe goose is protected. VERIFIED that they conflict.
2. **The goose hook ACK is dead code as shipped.** `ack_evidence = "dispatch"` with no turn key needs a visual Working frame (`fusion.rs:1000-1015,1103-1173`). Goose's manifest is forbidden one until measurement, and the screen tier is disabled for Dispatch (`gate.rs:2770`). The declared ACK can never upgrade a receipt. INFERRED from code.
3. **Readback can never succeed.** `exact_staging_proof` needs composer-content extraction, which returns `Unsupported` without `composer_prompt` (`inject.rs:512-533,676-681`). Every goose doorbell is `submitted_unverified` and spends 480 ms re-reading first. INFERRED from code.
4. **Mid-turn writes into a CLI that cannot accept input.** Cyclops's "the vendor queues the line" assumption (overview) is false for the goose CLI ("Message queuing is not available in the goose CLI", primer section 9). INFERRED consequence: tty echo garbles output, and the line is either lost or deferred.
5. **Interactive prompts are not modelled.** Approval and elicitation selects are not modals in the manifest, so a doorbell can answer them. INFERRED.
6. **Binding is unmeasured.** `pane_current_command` = `goose` is assumed (`goose.toml:11-12`). Goose installed as a wrapper script, or run under `uvx`/`npx`-like launchers, would show another name, and the pane would read `no_manifest`. INFERRED.
7. **No idle detection means no wake-only-when-idle policy.** There is no busy-avoidance either, and no deferral to turn end.
8. **The `Stop` hook is wired as blocking on goose main.** `cyclops hook` returns in at most 3 s against a 5 s timeout, so it adds up to 3 s at every turn end in the worst case. It is fail-open (primer section 3.4). VERIFIED budget; INFERRED impact.
9. **The skill allows `--raw` for unverified vendors** (`SKILL.md:244-252`). On goose, a multi-line raw paste is safe only if bracketed paste is active, which is unmeasured, and the body then lands in the pane as a prompt. INFERRED.

## Clever ideas worth stealing

- **Exact hook-payload ACK keyed on the injected bytes.** Matching `UserPromptSubmit.message` to the attempt's exact payload (`terminal.rs:42-44`, `ack.rs:658-665`) proves this specific line became this turn's prompt. On goose, `message` is populated in both 1.36 and main. For a future herdr-threads goose adapter, our content-free wake marker plus a goose `UserPromptSubmit` hook gives a cheap "wake landed" receipt. We would not need Cyclops's dispatch-acceptance machinery, because our wake text is not a payload that sibling hooks could reject. INFERRED.
- **An honest receipt vocabulary for unverifiable harnesses.** `submitted_unverified` / `notified(verified_by: none)` (`DELIVERY.md:233-251`) records "we typed it, we cannot prove it landed" without retrying on a clock. A goose Native wake from us would need the same honesty until a composer reader exists.
- **Dispatch vs receipt as distinct ACK strengths** (`src/cyclopsd/src/delivery/mod.rs:612-618`). A prompt hook firing does not prove the turn ran, because a sibling hook can reject it. This matters for goose `PreToolUse`/`Stop` interplay and for our own hook-based delivery accounting.
- **Hook-only Open Plugin discovery.** Dropping `~/.agents/plugins/<name>/hooks/hooks.json` (no `plugin.json`) is enough for goose to load hooks (VERIFIED in goose source above). It is the lowest-footprint way for herdr-threads to get goose turn edges (`UserPromptSubmit` = busy, `Stop` = idle) for a hook-maintained busy flag, which is what Native wake needs (primer sections 10 and 12).
- **"No matcher" as a documented hazard.** Goose silently skips a rule whose matcher is the invalid regex `*` (`hooks.json.tmpl:16-18`, test `hookset.rs:1744-1750`). Worth copying as a test if we ever ship goose hooks.
- **Hook authentication by peer credentials and ancestry** instead of payload fields (`hooks.md:170-184`). Goose payloads carry only `session_id` (primer section 3.3), so pane attribution must come from the process tree or the inherited `HERDR_PANE_ID`.

What Cyclops misses on goose, and herdr-threads should not (INFERRED):

- The `tom`/MOIM file gives in-turn delivery after every tool batch with no tool call (primer section 5).
- The goose-main Stop-block gives turn-end delivery.
- ACP `steer` and `session/prompt` give mid-turn injection and wake for ACP-hosted goose (primer section 7).

Cyclops's tmux-only, one-line-plus-claim design leaves all of these unused. On goose it pays a mandatory claim round trip per message, and every write is unguarded.

## Verification

An adversarial pass re-read the cited sources at Cyclops `a84d44b` and the goose clones (`goose-1.36` = v1.36.0 `f13f369`, `goose` = 1.56.0 `c3bfc5f`).

Confirmed:

- Manifest: header, `version_tested`, bindings, hooks table, the single rule, `submit`.
- The hook template and the hookset install text.
- `shipped_rules.rs`: the reviewed list, the forbidden fields, and the goose `message`/dispatch pins.
- The `admit` steps and their line ranges, and `composer_is_held`.
- The dispatch path: `record_dispatch_candidate` -> `confirm_unkeyed_dispatch_ack`, whose only caller is `fusion.rs:1162`. Also `positive_visual_working` and `tier2_evidence` `Dispatch => false`.
- `settle_without_receipt`, `SCREEN_ACK_DEADLINE` = 5 s and `VERIFY_DELAYS_MS`, and `prompt_matches`.
- The `hook.rs` contract and budget.
- `consumer.rs` and `hookset.rs` wiring and its gate.
- Doc quotes in `DELIVERY.md`, `STATUS.md`, `NEXT.md`, `SKILL.md` and `hooks.md`.
- Goose source: the `UserPromptSubmit` `message` field in 1.36 and main, plugin discovery defaulting to enabled, the `hooks/hooks.json` component marker, `stdin(Stdio::null())` in the shell tool, and `load_skill`.
- No MOIM, steer, `session/prompt` or `goose run` reference anywhere in the Cyclops clone.

Changed:

- The `write_ready` consumers list was not exhaustive; it now adds `can_decide_notification_now` and the readiness broadcast.
- Readback reasoning: `exact_staging_proof` also requires composer-content extraction, which is `Unsupported` without `composer_prompt`. Readback fails regardless of goose's `"> "` prefix.
- The envelope render citation was wrong. `terminal.rs:14-28` renders the direct-payload format; the claim envelope is `src/cyclops/src/main.rs:2992-3018`.
- Goose shell truncation limits are known (2000 lines / 50,000 bytes, with a notice and the full output saved), so "limit unknown, silently clipped" was wrong.
- Selftest: "would exit 1" was too strong. A retired-worker fallback counts a raw `UserPromptSubmit` edge.
- The skill "allows" `--raw` for unverified vendors rather than "recommends" it. Added that `--raw` also skips the copy-mode check.
- Added an "unknown" on whether candidate hook edges can lift the fused state off `Unknown`.
