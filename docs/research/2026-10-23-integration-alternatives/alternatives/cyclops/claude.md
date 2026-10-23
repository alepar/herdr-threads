# Cyclops x Claude Code: message injection and wake

- Upstream: https://github.com/cyclops-team/cyclops. The clone was read as untrusted data and not executed: `scratchpad/alts/cyclops`, commit `a84d44b8` (2026-09-06), version 1.1.0.
- Paths are relative to the clone root. Every claim is tagged **VERIFIED** (read in source or docs), **INFERRED** (reasoned from those), or **unknown**.
- Companion files: `overview.md` (architecture, identity, receipt states) and `../../primers/claude.md` (Claude Code surfaces). This file covers only the Claude path, in more depth.

---

## 1. Support status

**Native and measured.** VERIFIED from `resources/manifests/claude.toml`:

- `version_tested = "2.1.221"` (`:33`).
- The header records re-measurements on 2.1.222 (ghost-text probe), 2.1.239 (composer extraction and clear key) and 2.1.243 (idle trailer) (`:1-7`).
- Individual rules cite later live captures on 2.1.232, 2.1.236, 2.1.240, 2.1.246, 2.1.251 and 2.1.261 (`:121`, `:135`, `:167-179`, `:202`, `:278-286`, `:391-397`).
- The newest Claude evidence in the repo is 2.1.261 (2026-09-05). This workstation runs 2.1.309. Whether the screen rules still match 2.1.309 is **unknown**. Section 6 covers why that matters.

The pane is bound by process name or argv basename `claude`. It reads the argv of the foreground process group, so a `claude` started from a shell binds even when `pane_current_command` reports a version string (`claude.toml:34-43`, VERIFIED).

---

## 2. Injection mechanism

### 2.1 The channel: one typed user turn

Cyclops has a single way to get a message into Claude's context: it **types into the tmux pane**. Hooks report lifecycle and never inject content. There is no MCP server, no channel, no mod, no `additionalContext` and no `asyncRewake`. VERIFIED:

- `src/cyclops/src/hook.rs:1-6`: "fast, silent, exit 0 no matter what".
- `src/cyclops/tests/terminal_silence.rs:7-9`: "`cyclops hook` must write nothing at all, on stdout or stderr".
- `hook.rs:37-43`: `run` always returns 0.

What reaches the model is a **doorbell line** submitted as an ordinary user prompt. The model then has to run a Bash tool call to fetch the body.

### 2.2 Code path (VERIFIED)

1. **Accept.** `msg.send` fsyncs the message and one mailbox entry per recipient before it answers (`docs/development/DELIVERY.md:38-44`). One FIFO worker runs per recipient (`DELIVERY.md:45-49`; `src/cyclopsd/src/delivery/worker.rs`).
2. **Gate.** `admit` (`src/cyclopsd/src/delivery/gate.rs:1203-1323`) runs these checks in order:
   - pane present, alive, not in copy-mode (`:1209-1222`);
   - a manifest binds the pane (`:1224`);
   - one fresh capture is fused into a state (`:1230-1241`);
   - named blocks: dead, quota, modal, permission (`:1244-1288`). A modal with `auto_dismiss` and `decline_keys` gets its keys, at most `MAX_DECLINES = 3` (`delivery/mod.rs:82`);
   - the composer hold (`:1294`, `fusion::composer_is_held`);
   - the binding: the foreground process is the agent, or the screen still reads as the agent (`:1306-1322`).
3. **Spool.** The payload goes through `load-buffer` into a per-attempt tmux buffer, from a 0600 file under the 0700 cyclops home (`src/cyclopsd/src/delivery/inject.rs:78-105`).
4. **Commit.** `on_write()` records `writing` and claims the composer barrier *before* the await. Then `paste_buffer(&self.buffer, pane_id, true, true)` runs, which is `paste-buffer -p -d` (bracketed paste, then delete the buffer) (`inject.rs:111-145`).
5. **Verify.** Escaped `capture-pane -J` reads are taken at `VERIFY_DELAYS_MS = [0, 120, 240, 480]` (`delivery/mod.rs:72`; `inject.rs:202-226`). The proof is an exact composer row followed only by the vendor's declared trailer chrome (`inject.rs:336-404` `trailer_follows`, `:410` `exact_row_proof`), so a line a human typed below the paste cannot ride along. If the row is not proven, the attempt still proceeds and is recorded as unverified (`inject.rs:227-237`).
6. **Submit.** `send-keys Enter`, exactly once (`inject.rs:147-152`; `claude.toml:372`; `DELIVERY.md:67-68`).
7. **Receipt.** See §2.6.

### 2.3 Payload format (VERIFIED)

Doorbell, format 4 (`src/cyclops-proto/src/notification.rs:985-1002`):

```rust
format!("[cyclops from {sender_label} to {recipients}] {summary} | {}", render_doorbell_v3(attempt_id))
// -> [cyclops from implementer to reviewer] The rate limiter is ready for review. | cyclops inbox claim m-att_--AAAAAAQACAAAAAAAAAAQ
```

- Recipients: up to three labels, then `<a>, <b>, +N`, or `all` (`notification.rs:1007-1019`).
- The token is the 128-bit attempt id in 22 characters (`DELIVERY.md:136-142`).

The body is retrieved by `cyclops inbox claim m-att_...`, which returns an envelope (`skills/cyclops/SKILL.md:270-276`; renderer `print_claim_payload`, `src/cyclops/src/main.rs:2992-3018`, which also prints a `Summary:` line when the message has one and omits `TO:` for older daemons):

```
[cyclops cyc-d7e4ba90-1f3a6c2e] TO: reviewer  FROM: admin  SUBJECT: Review the rate limiter
Please look at retry.rs before the next run.
Reply: cyclops reply cyc-d7e4ba90-1f3a6c2e --summary "..." --body "..."
[cyclops:end cyc-d7e4ba90-1f3a6c2e]
```

`--raw` pastes the whole rendered message into the composer and presses Enter with no composer check (gate returns `Admission::Raw` before the copy-mode and composer checks, `gate.rs:1217-1219`). Its bytes come from a different renderer, `render_payload` (`src/cyclopsd/src/delivery/terminal.rs:14-28`): header `[cyclops <id>] FROM: <from>  SUBJECT: <subject>` with no `TO:`, the body, a `Reply: cyclops send <from> ...` hint (omitted for FYI and admin senders), and the sentinel. It is used only on explicit request (`DELIVERY.md:157-168`).

### 2.4 When it fires

- **Idle pane:** at once, once the gate admits (event-driven via tmux control mode; overview).
- **Busy pane (the agent is working on its own or a human's turn):** also at once. "A working agent gets its doorbell during the turn; the vendor queues the line" (`README.md:206`, VERIFIED). `composer_is_held` holds only on a seen human draft or on a hold that a delivery owns (`src/cyclopsd/src/fusion.rs:2386-2398`). A working agent is not a hold. `DELIVERY.md:100-103` lists "a working agent" explicitly among the cases that proceed. On Claude, a line typed while tools run is passed to the model "as soon as those tool calls finish, within the same turn"; leftovers go out at turn end (primer §7). So busy delivery is **mid-turn at the next tool boundary, or at turn end if no tool call follows** (INFERRED from the primer plus Cyclops' documented behaviour).
- **Busy because of an earlier doorbell:** held. `ComposerHold::TurnStarted` with an owner keeps the next doorbell for that recipient waiting until the screen shows the doorbell's turn ended (`fusion.rs:2391-2393`). This serializes doorbells per recipient, one per turn.
- **Contradiction, VERIFIED:** the Claude manifest says the opposite. "Claude 2.1.243 can queue input during ordinary generation but can interrupt active tool flow. Working is therefore not a supported write-safe state", with `safe_states = ["idle"]` (`claude.toml:379-382`). `safe_states` and `unsafe_states` are parsed (`src/cyclops-manifest/src/lib.rs:936-938`), but grep for `.safe_states`/`.unsafe_states` finds only a test assertion (`src/cyclops-manifest/tests/shipped_rules.rs:142`) and fixture strings (`lib.rs:1652`, `src/cyclopsd/tests/common/mod.rs:206`); no non-test code reads them. INFERRED: the 1.1.0 gate ignores the manifest's own warning. Cyclops writes into a working Claude that its manifest says can have its tool flow interrupted.

### 2.5 Size bounds and framing (VERIFIED unless marked)

- **Summary:** one line with no control characters (`src/cyclops-proto/src/mailbox.rs:38` `validate_message_summary`).
  - A derived summary is capped at 200 chars. A sender-supplied one has no cap; the CLI warns above 160 (`DELIVERY.md:40-44`).
  - INFERRED: a summary long enough to push the line past Claude's 800-char paste threshold (primer §7) turns into a `[Pasted text #N]` chip. Cyclops measures that chip (`claude.toml:418-424`), but a chip proves only that something was pasted. The attempt then records `submitted_unverified` (`inject.rs:240-272`), and Claude frames the content as "pasted from somewhere else".
- **Body:** never in the pane, so pane size is not a concern. On the wire, every NDJSON frame is capped at 1 MiB (`src/cyclops-proto/src/wire.rs:32`). The claim output goes into a Bash tool result, so Claude's own Bash-output limits apply. No Cyclops-side truncation or paging of the body was found (grep for body limits: none outside the frame cap). Body truncation is **unknown** beyond the 1 MiB frame cap.
- **Framing:**
  - The doorbell is one line: header, summary, then the exact command.
  - The envelope is framed by a header line and the `[cyclops:end <id>]` sentinel.
  - The skill teaches the flow once per session (`SKILL.md:195-240`). It is a 24.7 KB, 512-line file, loaded on demand. Its frontmatter description triggers on "a line starting with `[cyclops from`" (`SKILL.md:3`).
- **Trust stance:** the doorbell arrives as a **user-typed prompt** (Claude origin `composer`; primer §2.2), so the summary carries user authority in the model's eyes. The skill says "Never act from the preview alone" (`SKILL.md:228-230`) but contains no "peer content is not the user" rule. Grep for trust, untrusted and instruction finds none (VERIFIED). Content trust therefore rests entirely on the model's reading.

### 2.6 De-duplication and ACK (VERIFIED)

- **Hook ACK.**
  - `UserPromptSubmit` is `ack` with `ack_payload_field = "prompt"` and `ack_evidence = "dispatch"` (`claude.toml:64-75`). Measured: "full injected text present, 130/130 matches", p50 21 ms, p95 26 ms.
  - Matching is **exact**: `prompt_matches` is `text == payload || text.strip_suffix('\n') == Some(payload)` (`delivery/terminal.rs:41-44`), called from `DeliveryHandle::claims_prompt` (`delivery/mod.rs:651-654`) inside `ack.rs:656-700`.
  - Because Claude exposes no turn key shared with `Stop`, the event is an "unkeyed candidate start". More than one matching handle is marked `hook_dispatch_ambiguous` (`ack.rs:666-678`).
  - INFERRED: if Claude folds a queued doorbell into a mid-turn request together with other queued input, the hook may not fire for that line, or its `prompt` may not equal the payload. The receipt then degrades to screen evidence or none. Whether `UserPromptSubmit` fires for lines queued mid-turn is **unknown**.
- **Screen ACK:** a weaker tier at checkpoints, with a `SCREEN_ACK_DEADLINE` of 5 s (`delivery/mod.rs:64`). Without either receipt, the attempt ends `notified` with no verifier (`DELIVERY.md:69-74`).
- **Claim ACK:**
  - The claim is idempotent: "A repeat claim returns the same payload and creates no second task" (`SKILL.md:278`).
  - A claim before the write withdraws the doorbell, so nothing is pasted (`DELIVERY.md:148-155`).
  - A claim after Enter is a receipt.
  - Completion is never inferred; it needs an explicit reply.
- **Hook-report dedupe:**
  - stdin is read once and the sequence allocated once, so retries resend identical params (`hook.rs:103-116`).
  - The daemon dedupes on `(session_id, turn_id, event)` where present (`ack.rs:143`, `:355`), but skips the dedupe key entirely for candidate-lifecycle events (`ack.rs:350-351`). INFERRED: Claude's `UserPromptSubmit` is declared a candidate (`claude.toml:65`), so its reports are not deduped this way.
  - Reports are accepted only from a process inside the reported pane, checked by kernel peer credentials (`docs/reference/hooks.md:170-184`).
- **No re-paste:** the doorbell is never re-pasted. Post-write failures are never retried automatically (`DELIVERY.md:76-92`; `STATUS.md:109-112`).

---

## 3. Wake mechanism

### 3.1 Mechanism (VERIFIED)

The doorbell *is* the wake: paste, then Enter, submits a user turn. "This is the wake-up signal, not the complete message" (`SKILL.md:209-211`). Cyclops uses no harness-native wake for Claude: no `asyncRewake`, mod `$.prompt.submit`, `SendMessage` socket, channel, or `claude agents`. There is no priority or threshold, so **every message wakes an idle Claude** (INFERRED from the absence of any priority field in `DELIVERY.md` and the doorbell format).

### 3.2 Idle/busy detection (VERIFIED from `claude.toml:12-28` and its rules)

The manifest fuses three sensors:

| Sensor | What it proves on Claude | Evidence |
|---|---|---|
| Hooks (`--settings` file) | `UserPromptSubmit` is a *candidate* turn start; `SessionStart` or any authenticated edge gives process-bound liveness that admits a fresh clean pane as idle. **No end hook is declared**, so a turn never ends from a hook | `claude.toml:13-18`, `:64-68`; `resources/hooks/claude/settings.json.tmpl:12-21` |
| Title (`#{pane_title}` via control mode) | Braille spinner = working (`:82-89`). The `✳` sparkle is *not* trusted as idle: "1203 title-idle flaps in six hours on one pane, 25 doorbell writes admitted behind them" | `claude.toml:24-28`, `:91-98` |
| Screen (escaped capture) | The only sensor that confirms idle. `composer_completed_terminal_suffix_2_1_246` matches the `<glyph> <Verb> for 5m 11s · done 3:04 PM` row in colour 246, then the box rule, an empty `❯`, the box rule and the status trailer, with no active 215-coloured row between them | `claude.toml:143-183` |

- `Stop` and `StopFailure` are "liveness telemetry only". The reason given: Claude's Stop can continue the turn through `additionalContext`, so a Stop is a candidate end that triggers a screen re-check (`ack.rs:636-641`; template `:16-18`).
- The active-step rule is style-bound (glyph in `38;5;215` followed by a timer) at priority 1150, above the title spinner (1100), the completed-turn rules (1147-1148) and every composer rule, but below the modal and permission rules (1200-1300) (`claude.toml:121-141`). The reason: "while a tool task runs, Claude keeps the idle sparkle in the title AND keeps the composer prompt on screen".
- The "esc to interrupt" text rule is kept, but it was absent for a whole 17 s streaming turn in an unfocused pane (`claude.toml:305-315`).
- Interactive permission prompts are screen-only. "the Notification and PermissionRequest hooks did NOT fire in the measured run" (`claude.toml:353-367`).

### 3.3 Draft safety (VERIFIED)

The guard holds only on **positive** evidence of human input. "Ambiguous or absent composer evidence does not hold a doorbell" (`DELIVERY.md:10-16`, `:96-103`). For Claude the evidence is measured byte-level SGR conventions (`claude.toml:214-303`):

- **Real typed input:** `ESC[39m❯<NBSP>` followed by an *unstyled* byte means a draft (`composer_has_staged_input`, `:214-242`). This also covers the collapsed `[Pasted text #1 +80 lines]` chip.
- **Ghost or suggestion text:** `❯<NBSP>ESC[2m` (DIM) is read as idle (`composer_ghost_suggestion`, `:244-264`). It is deliberately narrow. Their note: "A ghost we fail to recognize costs a delivery that waits; a draft we fail to recognize costs the human's work". The ghost styling itself is "not yet captured live on Claude" (`:264`).
- **Styled input that is not DIM**, such as a typed `/model` in `38;5;153`, holds (`composer_styled_input`, `:266-286`). It is the "fail-safe floor".
- **NO_COLOR mode:** a structural rule anchors the prompt and the box rule at column zero (`:288-303`).
- **Completed turn with a draft:** `composer_completed_terminal_suffix_with_input` ends the turn but holds for the draft (`:186-202`). This fixed a bug where "one stray keystroke left the pane reading 'working' ... and held every delivery behind a draft that would never clear".
- **Release:** a held attempt is released by a pane event (draft erased, turn ended), never a timer. It pings the admin once after `gate_hold_notify_ms` = 120 s (`README.md:209-212`; `DELIVERY.md:271`).
- **Modals:**
  - The Chrome-extension dialog ("Enter to confirm" / "Esc to keep") is auto-dismissed with Escape. It once "swallowed a 4 KB paste entirely" (`claude.toml:335-351`).
  - The folder-trust dialog, where Escape **exits** Claude, outranks that rule at priority 1300 and is never auto-dismissed (`:317-333`).
- **Clear key:** `clear_keys = ["C-c"]`. One `C-c` clears a multiline composer and a 92-line chip; `C-e C-u` cleared only the last line (`claude.toml:373-376`). The send path does not use it to wipe a draft: a draft holds the attempt. Where `clear_keys` is used is **unknown** from this read.
- **Known gap:** "A person can type between the final capture and the tmux write; there is no input lease across that gap" (`STATUS.md:106-108`). Spooling before the final capture narrows the window (`inject.rs:15-25`).

### 3.4 Latency (VERIFIED / INFERRED)

- **Gate:** event-driven, one capture (VERIFIED).
- **Paste verify:** at most 480 ms of re-reads (`delivery/mod.rs:72`, VERIFIED).
- **Hook ACK:** p50 21 ms after submit (VERIFIED).
- **End to end:** an idle wake lands well under a second after acceptance (INFERRED; no end-to-end benchmark read).
- **The extra hop:** the model must emit a Bash call to `cyclops inbox claim`. That is one model step plus process spawn before the body is in context (INFERRED).
- **Holds:** a held doorbell waits for the next pane event, unbounded, with one ping at 120 s (VERIFIED).

### 3.5 Failure modes (VERIFIED unless marked)

- **Screen rules drift with Claude versions.** The idle-confirming rule is a long escaped regex over the status rows. The manifest documents one past break: a two-unit duration and a two-row trailer meant "no Claude turn ever ended on visual evidence and the pane stayed working until the next prompt" (`claude.toml:168-175`). INFERRED consequence: the doorbell-owned `TurnStarted` hold (`fusion.rs:2393`) never releases, so the next doorbell to that Claude waits indefinitely, with only the 120 s admin ping.
- **12-hour clock assumption.** The completed-row regex requires `done \d{1,2}:\d{2}\s+(?:AM|PM)` (`claude.toml:160`, `:166`). INFERRED: a Claude build or locale that prints a 24-hour time never confirms idle, giving the same stall.
- **Pasting into a working Claude** despite the manifest's warning that it can interrupt tool flow (§2.4).
- **Doorbell swallowed by an unknown modal.** If the screen matches no modal rule, the paste proceeds and verification fails, so the attempt is `submitted_unverified`. Enter may then confirm the modal's default choice (INFERRED; the Chrome case is exactly this, now covered by a rule).
- **`attention_required`** (paste or submit failed, pane rebound after paste, daemon restart mid-attempt) needs a human `cyclops requeue` (`DELIVERY.md:76-80`, `:201-210`).
- **Hooks not wired** (Claude launched without `--settings` and without the opt-in `~/.claude/settings.json` merge): no hook ACK and no SessionStart liveness. Delivery still happens on screen evidence (`hooks.md:201-207`).

---

## 4. Setup and installation footprint on Claude Code (VERIFIED)

- **Default, non-invasive.** `cyclops hooks install claude --agent <label>` renders `$CYCLOPS_HOME/hooks/claude/<label>/settings.json` (`hooks.md:13-40`). Panes started by `cyclops start --agents claude` launch as `claude --settings '<file>'` (`claude.toml:44-58`; `src/cyclops/src/workspace.rs:612-622`, asserted by a test at `:2231`). The tool refuses to write into `~/.claude` through `--dest` (`hooks.md:36-38`).
- **Opt-in global wiring.** `scripts/install.sh` ends with `cyclops start --setup-only --wire-hooks`, which does two things (`hooks.md:113-145`; `src/cyclops/src/hookset.rs:180-189`):
  - it **merges the hook entries into `~/.claude/settings.json`**, backing the file up as `<path>.before-cyclops` before the first edit (`src/cyclops/src/wiring.rs:984-1001`);
  - it seeds `~/.claude/skills/cyclops/SKILL.md`.
  
  Consent is recorded at `~/.cyclops/vendor-wiring-consented`, and `CYCLOPS_NO_VENDOR_HOOKS=1` declines the step.
  - Doc drift: the template says "cyclops never edits ~/.claude for you" (`resources/hooks/claude/settings.json.tmpl:7-10`). That holds only without the installer's opt-in.
  - `CLAUDE_CONFIG_DIR` is never consulted. Grep of the whole clone finds no hit, and the Claude root is `user_home.join(".claude")` (`src/cyclops/src/consumer.rs:181`), while the same match honours `CODEX_HOME`, `KIMI_CODE_HOME` and `QWEN_HOME` (`consumer.rs:170-198`). INFERRED: a user running Claude with a relocated config dir (as this workstation does with `~/.aisw` profiles) gets hooks wired into a file that Claude never reads.
- **Hooks written:** eight synchronous `command` hooks: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `StopFailure`, `Notification`, `PermissionRequest` (`settings.json.tmpl:22-49`).
  - Each runs `cyclops hook <Event>`. The event name is on the command line because "the receiver must not trust payloads to self-identify" (`:18-21`); the origin is agy, whose payloads carry no event-name field (`hook.rs:8-10`).
  - None are `async`, and none use matchers.
- **Per-hook budget:** 3 s total (stdin 1 s, connect 1.5 s, hello 0.5 s), typically single-digit ms (`hook.rs:30-35`). Failures are logged to `$CYCLOPS_HOME/hook-errors.log`, never surfaced.

---

## 5. Token cost

| Event | Model-visible bytes | Notes |
|---|---|---|
| Quiet turn | **0** | Hooks print nothing (`terminal_silence.rs:7-9`), VERIFIED |
| Session start | Skill description only (about 100 tokens in the skill listing, `SKILL.md:3`). The full skill (24.7 KB, roughly 6k tokens, INFERRED from size) loads when triggered, typically by the first doorbell | VERIFIED size; load trigger INFERRED |
| One message to an idle Claude | A **new user turn**: doorbell line (about 30-60 tokens plus summary), then a model Bash call (`cyclops inbox claim m-att_...`, about 30 output tokens), then the envelope as a tool result (header, body, reply hint, sentinel). A full turn re-reads the cached context | INFERRED from the format |
| One message to a busy Claude | The same doorbell line queued into the running turn, plus the claim round trip | INFERRED |
| N messages | N doorbells, serialized one per turn by the owned `TurnStarted` hold (`fusion.rs:2393`). No batching | INFERRED from the hold logic |

Per message, the cost is one mandatory extra model step (the claim) and, when idle, a whole turn. There is no batching and no priority filter.

---

## 6. Limitations and bugs visible from the source

1. **Manifest says "working is not write-safe", the gate writes anyway** (`claude.toml:379-382` vs `gate.rs:1289-1296`, `fusion.rs:2386-2398`; `safe_states` unused). VERIFIED inconsistency; the effect is INFERRED.
2. **Idle confirmation depends on one screen regex**, with no hook-derived turn end. A Claude UI change, 24-hour time, or a changed status trailer stalls the per-recipient queue behind the owned hold (§3.5). The evidence stops at 2.1.261, and Claude ships builds very frequently.
3. **Every message wakes.** No priority, no threshold, no lazy delivery (INFERRED, absence).
4. **Body by pull only.** The model must run a tool to see the content, and `SKILL.md:228-230` must persuade it not to act on the summary.
5. **Peer text arrives with user authority** (a composer-origin prompt). There is no framing that separates peer content from the user's own instructions (§2.5).
6. **`CLAUDE_CONFIG_DIR` ignored** for global wiring (§4).
7. **PreToolUse and PostToolUse are wired synchronously on every tool call, unmatched**, each a process spawn plus a socket round trip of up to 3 s. The whole stdin payload is forwarded, `tool_input`/`tool_response` included (`hook.rs:106-123`). A large tool response over 1 MiB exceeds the frame cap and fails silently into the log (INFERRED from `wire.rs:32`). Whether the daemon persists those payloads was not read: **unknown**.
8. **ACK requires byte-exact prompt equality** (`terminal.rs:41-44`), which is fragile to any Claude prompt normalization or queued-line merging (INFERRED).
9. **Type-over race** between the final capture and the paste remains (`STATUS.md:106-108`).
10. **Ghost-text rule unmeasured on Claude** (`claude.toml:264`). If Claude renders suggestions without DIM, they fall to `composer_styled_input` and hold. This fails safe: a delay, not lost input.
11. **No coverage of compaction or resume:** the manifest has no rule for `/compact` or resume states (INFERRED from the rule list).

---

## 7. Clever ideas worth stealing

1. **Measured, data-driven composer discriminators at the SGR byte level.** `❯<NBSP>` plus unstyled text means a real draft, `ESC[2m` means a ghost, any other styling holds (`claude.toml:214-286`). This directly answers our open problem: herdr-threads treats any text in a Claude composer as `Unsafe`, because it may be a prompt suggestion (primer §9; `src/harness/claude/composer.rs`). On our fallback (non-mod) path, an escaped capture could tell suggestion from draft. Caveat: Herdr would need to expose escaped pane text.
2. **Spool, then final capture, then commit with the barrier armed before the await** (`inject.rs:15-45`, `:111-131`). This shrinks the type-over window and makes the write boundary a durable fact. It applies to our native typing wake.
3. **Terminal-proof verification.** The pasted row must be followed only by the vendor's trailer chrome (`inject.rs:324-404`), so a human's extra line can never be submitted along with ours.
4. **Modal rules with auto-dismiss only where Escape is proven safe**, and a higher-priority rule for the dialog where Escape exits (`claude.toml:317-351`). Our native wake could adopt the "Chrome extension detected" dismissal.
5. **Receipt honesty vocabulary:** `submitted` / `submitted_unverified` / `notified(verified_by hook|screen|none)` / `attention_required`, with no timed retries. Also a **claim before write withdraws the doorbell** (`DELIVERY.md:148-155`): if the model already read the item in-turn, no wake is typed. Our analogue is to cancel a pending nudge once an in-turn delivery is ACKed. Whether we already do this should be checked.
6. **Stop treated as a candidate end only**, because Claude's Stop can continue through `additionalContext` (`ack.rs:636-641`). This matters for any design of ours that infers idle from `Stop`.
7. **Silent, budgeted hook client** (3 s hard budget, never fails the vendor, errors to a log file), with the **event name in argv**, not trusted from the payload (`hook.rs:1-43`).
8. **Title is distrusted as an idle signal**, with measured evidence (1203 flaps in 6 h) (`claude.toml:24-28`). This is useful if we ever consider Herdr's title-based detection for Claude.

### Contrast with herdr-threads (INFERRED, brief)

- **Our mod path:**
  - in-turn delivery of the body through `tool.call` `context`;
  - `$.session.append` plus `$.prompt.submit('wake up')` for idle wakes;
  - `$.prompt.read` for the draft;
  - turn events instead of screen scraping.
  
  This avoids most of Cyclops' fragility (the screen regexes, the claim round trip, user-authority framing, mid-tool-flow typing). Cyclops' techniques apply mainly to our **no-mod fallback** (native Herdr typing), where Cyclops is considerably more careful than a composer-text check.

---

## Verification

Adversarial pass against clone `a84d44b` (2026-09-06) and the Claude primer.

- **Checked and held:** manifest header, `version_tested`, binding, hook ACK fields and latencies, title/screen/composer/modal/permission rules and their line ranges, `clear_keys`, `safe_states` text (`claude.toml`); `hook.rs` silence, budget, single stdin read; `terminal_silence.rs`; hook template (eight synchronous unmatched hooks, comment text); `gate.rs` `admit` order; `fusion.rs:2386-2398` `composer_is_held`; `inject.rs` spool, commit barrier, verify loop, unverified fallthrough, `trailer_follows`/`exact_row_proof`; `paste-buffer -p -d` (`src/cyclops-tmux/src/control.rs:943-964`); `delivery/mod.rs` constants; `prompt_matches`/`claims_prompt`; `ack.rs` ambiguity and candidate-end handling; doorbell v4 format and recipient list; summary validation; 1 MiB frame cap; DELIVERY, README, STATUS, hooks.md citations; `.before-cyclops` backup; SKILL.md size (24,772 bytes, 512 lines), description trigger, and absence of any trust rule.
- **Corrected:** the claim envelope renderer is `main.rs:2992-3018`, not `terminal.rs:14-28`; `--raw` pastes the different `render_payload` format, not the claim envelope. Active-step rule does not rank above modal/permission rules. `safe_states` is read by a test assertion, not only the parser fixture. `CLAUDE_CONFIG_DIR` citation moved from a test (`consumer.rs:304`) to the real resolver (`consumer.rs:181`). `workspace.rs:2231` is a test. `DELIVERY.md:270` -> `:271`. Version-evidence line list completed.
- **Added:** hook-report dedupe is skipped for candidate events (`ack.rs:350-351`); busy delivery can also land at turn end; `DELIVERY.md:100-103` explicitly lets a working agent proceed.
- **Not checked:** repo stats (the report states none); body truncation remains unknown.
