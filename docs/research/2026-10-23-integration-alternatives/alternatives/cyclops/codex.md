# Cyclops x OpenAI Codex CLI: injection and wake

- Upstream: https://github.com/cyclops-team/cyclops. The clone was read only and never executed: `scratchpad/alts/cyclops`, commit `a84d44b8f5476c10ecf849d46370d2cb0238dab9` (2026-09-06). Paths are relative to the clone root.
- Codex source cross-checks use the primer's read-only clone of `openai/codex` at tag `rust-v0.163.0` (`scratchpad/codex-src/codex`, paths `codex-rs/...`). Cyclops measured Codex 0.149.1 and saw 0.150.1. The newer source is used only to predict behaviour on current Codex. Every such prediction is marked INFERRED.
- Repo stats (`gh api repos/cyclops-team/cyclops`, 2026-10-23): MIT, 14 stars, 0 forks, created 2026-08-01, last push 2026-09-08.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from what was read. **unknown** means not found.
- Read first: [overview.md](overview.md) (architecture, identity, receipts) and [../../primers/codex.md](../../primers/codex.md).

---

## 1. Support status

**Native. The manifest is measured. Codex is one of the five measured vendors.** VERIFIED:

- `resources/manifests/codex.toml:22-30` has `version_tested = "0.149.1"`, `process_names = ["codex"]` and `launch = "codex"`. The header (`:1-20`) records measurements on 0.145.0, 0.147.0, 0.149.0 and 0.149.1, ending with "fresh, resumed, and daemon-restart delivery matrix was completed on 0.149.1 (2026-08-26)". "Daemon" here means the Cyclops daemon, not Codex's app-server.
- `STATUS.md:137` says "Codex CLI | 0.149.1 | 0.150.1 fresh-pane discovery in the final two-agent exercise; historical fresh, resumed, restart, draft, modal, raw, collapsed, claim, and reply evidence | Full current-version live matrix beyond the exercised cells".
- Sensor tiers (`codex.toml:13-20`):
  - The pane **title** gives a working signal only: a Braille spinner prefix from 0.149.0 on.
  - **Hooks** are authoritative for turn edges and ACK.
  - The **screen** is required for idle, human input and modal states.

What "native" means here: Cyclops reads Codex through tmux screen captures and a few silent lifecycle hooks, and writes to it **only through the terminal**. It does not use any Codex programmatic surface: app-server, `codex queue`, `turn/steer`, `thread/inject_items`, MCP or `exec resume`. A grep of `--include=*.rs/*.md/*.toml` for `app-server|no-daemon|CODEX_EXEC_SERVER` finds only design notes plus Cyclops's own unrelated `cyclops start --no-daemon` flag (`docs/guides/install.md:414`, `src/cyclops/tests/workspace_cli.rs:349-358`); nothing refers to Codex's app-server or `--no-daemon`. VERIFIED. The design notes list `codex queue --thread <id> --message <text>`, app-server connections and `exec resume` as "real control paths" that still "need a contract probe" (`docs/development/archive/MESSAGING_ARCHITECTURE_REVIEW.md:1356-1362`; `ADDENDUM_REVIEW.md:364`). Headless vendor protocols are the designated fallback "if TUI injection ever becomes untenable" (`docs/development/HANDOFF.md:384-396`). Not built. VERIFIED.

---

## 2. Injection mechanism

### 2.1 What reaches the model

There are two steps, and the body never touches the pane on the default path.

1. **Doorbell, one line typed into the Codex composer.** It becomes a user prompt. The format is `render_doorbell_v4` ("notification format 4") in `src/cyclops-proto/src/notification.rs:992-1001` (VERIFIED):

   ```rust
   format!("[cyclops from {sender_label} to {recipients}] {summary} | {}",
           render_doorbell_v3(attempt_id))   // "cyclops inbox claim m-att_<22 chars>"
   ```

   Example (`docs/development/DELIVERY.md:132-134`):
   `[cyclops from implementer to reviewer] The rate limiter is ready for review. | cyclops inbox claim m-att_--AAAAAAQACAAAAAAAAAAQ`

2. **Body, pulled by a tool call.** The skill tells the model to run the claim command exactly once (`skills/cyclops/SKILL.md:200-232,260-279`). Codex runs `cyclops inbox claim m-att_...` through its shell tool. The tool output is the framed envelope (`SKILL.md:270-276`):

   ```
   [cyclops cyc-d7e4ba90-1f3a6c2e] TO: reviewer  FROM: admin  SUBJECT: Review the rate limiter
   Please look at retry.rs before the next run.
   Reply: cyclops reply cyc-d7e4ba90-1f3a6c2e --summary "..." --body "..."
   [cyclops:end cyc-d7e4ba90-1f3a6c2e]
   ```

   The claim envelope is printed client-side by `print_claim_payload` (`src/cyclops/src/main.rs:2992-3019`): the `TO/FROM/SUBJECT` header (older daemons without a recipient label omit `TO:`), an optional `Summary:` line, the body, a `Reply: cyclops reply <id> ...` hint omitted only for `fyi`, and the end sentinel. VERIFIED. (`render_payload` in `src/cyclopsd/src/delivery/terminal.rs:14-28` is the daemon's direct/raw payload: no `TO:`, a `Reply: cyclops send <from> ...` hint omitted for `fyi` and for admin senders.)

**Hooks inject nothing.** `src/cyclops/src/hook.rs:1-6` says: "fast, silent, exit 0 no matter what". The file has no `println!`/`print!`/stdout write (grep, VERIFIED). This matters on Codex, because **plain stdout from a `SessionStart` or `UserPromptSubmit` hook is added to model context** (primer §1.4). Cyclops's silence means it adds zero context bytes per turn. VERIFIED (code). The Codex effect is INFERRED from the primer.

### 2.2 Code path (VERIFIED unless marked)

| Step | Where | What |
|---|---|---|
| Accept | `DELIVERY.md:38-44` | The message plus one mailbox entry per recipient, fsynced before replying |
| FIFO worker | `DELIVERY.md:45-49`; `src/cyclopsd/src/delivery/worker.rs` | One worker per recipient, strictly ordered |
| Gate | `src/cyclopsd/src/delivery/gate.rs:1203-1325` (`admit`) | Pane alive and not in copy-mode; manifest bound; fresh fused capture; named blocks; `fusion::composer_is_held`; foreground binding |
| Spool | `src/cyclopsd/src/delivery/inject.rs:90-105` | `load-buffer` into a per-attempt buffer through a 0600 spool file. Retryable, touches no pane |
| Write boundary | `inject.rs:111-145` | `on_write()` records `writing` and claims the composer barrier **before** `paste_buffer(&self.buffer, pane_id, true, true)` (bracketed `-p`, delete `-d`) |
| Readback | `inject.rs:188-200+`; `codex.toml:261-270` | One escaped `capture-pane -e`, wraps joined. Verify the pattern `["<message_id>", "Pasted"]` |
| Submit | `inject.rs:147-152`; `codex.toml:256` | `send-keys Enter`, exactly once (`DELIVERY.md:67-68`) |
| Receipt | `src/cyclopsd/src/ack.rs:655-700` | A `UserPromptSubmit` hook report whose `prompt` field (`codex.toml:59` `ack_payload_field = "prompt"`) the delivery handle `claims_prompt(text)` gives `verified_by: hook`. Otherwise screen evidence; otherwise none after 5 s |

### 2.3 When it fires

- **Idle Codex:** the paste and Enter start a new turn.
- **Working Codex:** the gate does **not** hold on "working". `composer_is_held` (`src/cyclopsd/src/fusion.rs:2386-2398`) holds only on a human draft (`HumanInput` / `IdleWithInput`), on a doorbell staged but not consumed, or on a turn that **a Cyclops doorbell started** and that has not ended (`TurnStarted` with `hold_owner`). A turn a human started proceeds. `README.md` "The composer rule" also says "A working agent gets its doorbell during the turn; the vendor queues the line". VERIFIED.
  - On Codex, Enter in a busy composer **submits a steer of the active turn**. It does not join the TUI's local Tab-queue (primer §7, `chat_composer.rs`). INFERRED: a mid-turn Cyclops doorbell therefore becomes steered user input. The model sees it at the next sampling boundary of the human's turn.
  - Codex 0.163 runs `UserPromptSubmit` on steered pending input with the **active turn's** `turn_id` (`codex-rs/core/src/session/turn.rs:455-476` calls `run_hooks_and_record_inputs`, which calls `inspect_pending_input`, which builds `UserPromptSubmitRequest { turn_id: turn_context.sub_id, .. }`; `codex-rs/core/src/hook_runtime.rs:677-702`). VERIFIED in Codex source. See §6 for why this matters to Cyclops's dedupe.
  - The manifest disagrees with this: `codex.toml:271` declares `safe_states = ["idle"]`, but `safe_states` is only parsed (`src/cyclops-manifest/src/lib.rs:936`) and never consulted by `gate.rs` (grep, VERIFIED). The manifest's own note at `codex.toml:176-182` describes the old bug, "the gate would paste into a live generation", as something the working rule fixes. Under 1.1.0 semantics it now pastes mid-turn on purpose. INFERRED: the comment is stale.
- **Back-to-back messages to one Codex:** the owned-turn hold serializes them. Doorbell N+1 waits until Codex's `Stop` for the turn doorbell N started arrives with the same `(session_id, turn_id)` (`codex.toml:40-53` `turn_key_fields`). So Cyclops never steers into a turn it started itself. One message means one turn. VERIFIED (`fusion.rs:2382-2393`; `DELIVERY.md:96-100`).

### 2.4 Size bounds

- **Doorbell:**
  - A derived summary is at most 200 characters, one line.
  - A sender-supplied summary has **no cap**. The CLI warns above 160 characters (`DELIVERY.md:40-44`). VERIFIED.
  - Codex collapses a paste over 1,000 characters into a `[Pasted Content N chars]` chip (`codex-rs/tui/src/bottom_pane/chat_composer.rs:450` `LARGE_PASTE_CHAR_THRESHOLD = 1000`, VERIFIED in Codex source). Cyclops measured the chip (`codex.toml:100-124,301-305`) and verifies a chip by the literal `Pasted` (`codex.toml:262-270`), because the chip hides the attempt id.
  - INFERRED: a doorbell with a summary over about 950 characters reaches Codex as a chip. The model still receives the full pasted text when it is submitted (the chip is display-only).
- **Body:** no per-message body cap was found. The only bound is the 1 MiB NDJSON frame (`src/cyclops-proto/src/wire.rs:31` `MAX_JSON_BYTES = 1_048_576`). VERIFIED. Truncation is left to Codex's own shell-tool output limits: unknown/not traced here.
- **Raw mode** (`send --raw`) pastes the header, the whole body, the reply hint and the `[cyclops:end <id>]` sentinel, with no composer check (`DELIVERY.md:157-168`). A Codex body over 1,000 characters becomes a chip. Its readback is verified by the trailer layout regexes (`codex.toml:274-313`). VERIFIED.

### 2.5 Framing

One header line per doorbell: `[cyclops from A to B]`, then the summary, then the claim command. The envelope is TO/FROM/SUBJECT with an end sentinel. The skill (24,772 bytes, `skills/cyclops/SKILL.md`) explains the flow once. It is seeded at `~/.agents/skills/cyclops/SKILL.md` for Codex (`docs/reference/hooks.md:124-126`). Codex loads skills on demand. INFERRED: the skill body is not in every turn. VERIFIED (paths). The skill says "Never act from the preview alone" (`SKILL.md:224-225`).

### 2.6 Dedupe and ACK

- **Claim idempotence.** A repeat claim returns the same payload and "creates no second task" (`SKILL.md:278-279`). A socket claim before the write withdraws the doorbell, so no pane bytes are written (`DELIVERY.md:148-155`). VERIFIED.
- **Hook dedupe.** User-level and project-level Codex hooks both fire when the directory is trusted (`codex.toml:74-76`). The daemon dedupes on the reporter sequence number, then on `session|turn|event` (`ack.rs:333-362`, `dedupe_ids` at `ack.rs:143-155`). VERIFIED.
- **Receipt tiers.**
  - Hook ACK: `UserPromptSubmit.prompt` equals the pasted payload, matched by `claims_prompt` (`ack.rs:658-668`; `claims_prompt` at `src/cyclopsd/src/delivery/mod.rs:651-654` calls the exact-match helper `prompt_matches`, `terminal.rs:42-44`, which allows only one trailing newline). The ACK must arrive within `ack_timeout_ms` = 1500 (`DELIVERY.md:270`). Measured ACK latency is p50 30.4 ms and p95 34.8 ms (`codex.toml:60-61`).
  - Otherwise screen evidence. Otherwise `notified` with no verifier after 5 s. VERIFIED.
- **Completion** is a separate fact: an explicit reply. It is never inferred from idle (overview.md). The model never "ACKs" in our sense. A claim is retrieval, not completion (`SKILL.md:279-280`).

---

## 3. Wake mechanism

### 3.1 Mechanism

**The doorbell is the wake.** Cyclops uses the same bracketed paste and Enter on Codex as everywhere else. No Codex API is involved (`SKILL.md:209-211`: "This is the wake-up signal, not the complete message"). VERIFIED. Every message wakes the recipient. There is no priority or threshold (overview.md, INFERRED from the absence of any such field).

### 3.2 Idle and busy detection on Codex (VERIFIED, `codex.toml`)

| Signal | Rule | Notes |
|---|---|---|
| Working (title) | `title_working_spinner`, priority 1110, `^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] \S.*$` on the pane title (`:155-165`) | Measured on 0.149.0. Older versions keep a static title |
| Working (screen) | `screen_working`, priority 1100: `contains "esc to interrupt"` or `^\s*•\s+(?:Working\|Thinking)\b` (`:167-194`) | It must outrank the composer rules, because Codex keeps the ghost composer on screen for the whole turn (a measured past bug) |
| Human draft | `composer_typed_input`, priority 1050, `human_input`. An escaped regex: the bold-only `ESC[1m›`, then a non-SGR byte or a non-dim SGR (a colored `Pasted` chip) (`:93-124`) | Escaped capture only. It separates the dim ghost text from bare typed text. A transcript `›` is bold+dim, so it is excluded |
| Ghost suggestion | `composer_ghost_suggestion`, priority 1040: `ESC[2m` directly after the glyph (`:126-136`) | Idle and safe |
| Fallback | `composer_empty_or_ghost`, priority 1000, plain `^\s*›`, `ambiguous` (`:138-153`) | On a plain capture this also matches typed text. The esc rules outrank it |
| Turn edges | Hooks `UserPromptSubmit` (start) and `Stop` (end), keyed on `["session_id","turn_id"]` (`:35-53`) | VERIFIED against Codex `rust-v0.149.0` source, per the comment |
| Cancelled approval | `approval_cancelled_terminal`: "Conversation interrupted - tell the model what to do differently" plus the dim ghost composer, `active_start_terminal = true` (`:196-212`) | "Codex 0.149.1 does not emit Stop when the human cancels a command approval". This screen rule ends the stranded start latch |
| Modals | `trust_dialog` (hold, never auto-dismiss) (`:214-223`); `update_dialog` (auto-dismissed with `["3","Enter"]`, "Skip until next version") (`:225-239`); `approval_prompt` (hold; INFERRED rule, not reproduced) (`:241-252`) | The update dialog is "HIGHEST-SEVERITY INJECTION HAZARD": a blind Enter runs `brew upgrade --cask codex` |

The composer semantic is computed independently of the runtime winner. A human draft typed while Codex shows "Working" is therefore still detected and holds the doorbell (`fusion.rs:3160-3178`, `:4232-4240` `screen_composer_semantic_esc`). VERIFIED.

### 3.3 Draft safety

- The doorbell holds on a positively seen draft (escaped capture), a staged or unconsumed doorbell, an owned turn, a modal, a permission prompt or a quota screen (`DELIVERY.md:94-125`).
- An unreadable composer **proceeds**, and the attempt is journaled as `submitted_unverified` if the readback fails (`DELIVERY.md:96-103`). VERIFIED.
- `clear_keys = ["C-c"]`: one C-c clears the whole multiline composer and a 92-line chip "without submitting or exiting" (`codex.toml:257-260`, measured 0.149.0). In this commit `clear_keys` is only parsed and validated (`src/cyclops-manifest/src/lib.rs:928,1307-1322`); a grep of `src/` finds no daemon code that sends it, so delivery never clears a composer. VERIFIED (absence).
- Known race: "A person can type between the final capture and the tmux write; there is no input lease across that gap" (`STATUS.md:106-108`). The barrier (`on_write` before `paste-buffer`) only stops two **Cyclops** attempts from racing (`inject.rs:116-131`). VERIFIED.
- Codex specifics: the paste is bracketed (`-p`), so Codex's 120 ms paste-burst Enter suppression (primer §7) should not apply. The readback capture also sits between the paste and Enter. INFERRED.

### 3.4 Latency

- Hook ACK: p50 30 ms (measured).
- Gate-to-Enter: one capture, spool, paste and one escaped readback with "bounded re-reads for a slow repaint" (`DELIVERY.md:63-66`). Exact latency is unknown; INFERRED sub-second.
- A held attempt resumes on the next pane event, with no polling (tmux control-mode notifications). It pings the admin once after `gate_hold_notify_ms` = 120 s (`DELIVERY.md:24,54-56,270`). VERIFIED.
- Title and screen changes arrive at about 1 s tmux granularity (`docs/guides/wait.md` "Limits"). VERIFIED.

### 3.5 Failure modes (VERIFIED unless marked)

- **Hooks silently absent.** A project-local `.codex/hooks.json` does not load in an untrusted directory, and `--dangerously-bypass-hook-trust` does not help (`codex.toml:64-72`, "4 consecutive exec runs fired zero hooks"). Delivery still works, but receipts drop to the screen tier. The owned-turn hold then needs screen lifecycle evidence to release.
- **Hook-definition trust.** New or changed commands are skipped until they are trusted in `/hooks` (`docs/reference/hooks.md:48-59`). Moving the binary changes the command string. Cyclops greps the wired files for the old binary path so it can name the stale copy (`src/cyclops/src/hookset.rs:1214-1245`).
- **Post-write failures** (`paste_failed`, `submit_failed`, `pane_rebound_after_paste`) become `attention_required`, which needs a human `cyclops requeue`. There is no automatic retry (`DELIVERY.md:76-80,228-231`).
- **The trust dialog on first launch** blocks delivery until a human answers it. It is never auto-answered (`codex.toml:214-223`).
- **Codex shared app-server daemon (≥ about 0.157). INFERRED, high risk, untested by Cyclops.**
  - Cyclops authenticates the sender, the claimer and hook reports by walking kernel peer-credential ancestry into a watched pane. A "vendor process outside every watched pane ... is denied" (`DELIVERY.md:257-266`), and `agent.state.report` is accepted "only from a process inside the very pane it reports for" (`docs/reference/hooks.md:170-184`).
  - The primer (§2.1, §9) reports that Codex TUIs now attach to a shared machine-wide app-server daemon by default, so hooks and tools may run in the daemon's process tree.
  - If so, `cyclops hook ...` and `cyclops inbox claim ...` launched by Codex would descend from the Codex daemon, not from the pane. The hook ACKs would be denied, and the claim itself could be refused. That would break both receipt and body retrieval.
  - Cyclops was measured on 0.149.1 and 0.150.1, before the daemon default, and gives no Codex `--no-daemon` advice (grep; its only `--no-daemon` is its own `cyclops start` flag). Whether a daemon spawned from a pane stays inside that pane's ancestry is unknown.
- **Interrupt.** Codex 0.163 has an `Interrupt` hook event (primer §1.2). Cyclops does not wire it (`resources/hooks/codex/hooks.json.tmpl:24-36` registers only `UserPromptSubmit`, `Stop` and `PermissionRequest`), and the manifest's `available` list predates it (`codex.toml:54-58`). An Esc-interrupted owned turn is released only by the screen rule. That rule's text, "Conversation interrupted", is the same string Codex prints on Esc, so it probably covers interrupts too. INFERRED.

---

## 4. Setup and installation footprint on Codex

VERIFIED (`docs/reference/hooks.md`, `src/cyclops/src/consumer.rs:168-210`, `hookset.rs:163-178`, `workspace.rs:2238-2275`):

- **Prepare (default):**
  - `cyclops hooks install codex --agent <label>` writes only `$CYCLOPS_HOME/hooks/codex/<label>/hooks.json` and prints merge instructions.
  - It refuses to write into `~/.codex` (`hooks.md:11-40`).
  - The template registers three silent command hooks (`resources/hooks/codex/hooks.json.tmpl:24-36`):

    ```json
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "{cyclops_bin} hook UserPromptSubmit --agent {label}" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "{cyclops_bin} hook Stop --agent {label}" }] }],
    "PermissionRequest":[{ "hooks": [{ "type": "command", "command": "{cyclops_bin} hook PermissionRequest --agent {label}" }] }]
    ```
- **Opt-in auto-wiring.** `scripts/install.sh` ends with `cyclops start --setup-only --wire-hooks`, which merges entries into `$CODEX_HOME/hooks.json`. The default is `~/.codex/hooks.json`. Codex hooks stay at user level on purpose, because of the directory-trust trap (`consumer.rs:175-183`).
  - It backs up the original before the first edit and never creates a missing vendor directory.
  - Consent is recorded in `~/.cyclops/vendor-wiring-consented`. `CYCLOPS_NO_VENDOR_HOOKS=1` opts out (`hooks.md:113-145`).
- **Shared file, no label.** Every Codex pane reads one `$CODEX_HOME/hooks.json`, so the shared render strips `--agent` (`hookset.rs:163-178` `render_shared`). The daemon derives the reporter from the socket peer. `cyclops start` passes `CYCLOPS_AGENT=<label>` in the pane env, only to namespace the hook sequence counter (`workspace.rs:2257-2275`; `hook.rs:27-28,96-101`).
- **Skill.** `~/.agents/skills/cyclops/SKILL.md` (shared with Cursor, Gemini, goose, OpenCode, Amp and Crush). It is created only if the parent directory already exists (`hooks.md:122-130`).
- **Manual steps the user must do.** Trust the hook definitions in `/hooks`. Optionally pre-seed `[projects."<dir>"] trust_level = "trusted"` in `config.toml`; Cyclops prints this and never writes it (`hookset.rs:580-592`). Prove the wiring with `cyclops hooks selftest <label>`, which costs one trivial turn (`hooks.md:186-199`).
- Launch is the bare `codex` command, with no flags (`codex.toml:27-30`).

---

## 5. Token cost

INFERRED from the formats above. The token counts are rough, at about 4 characters per token.

| Event | Model-visible bytes | Notes |
|---|---|---|
| Quiet turn (no mail) | **0** | The hooks are silent (`hook.rs`), and no per-turn digest is added |
| Per message, idle recipient | doorbell about 110 to 250 chars (30 to 70 tokens), as a user prompt | It **starts a full new turn**, and the claim command is part of it |
| Per message, claim | the model writes one shell call (about 15 to 25 tokens), then the envelope: the body plus about 150 to 250 chars of framing, including the `Reply:` hint (40 to 60 tokens) | The extra sampling round trip is mandatory. The body is never in the doorbell |
| Per message, busy recipient | the doorbell as a steer, then the same claim tool call inside the human's turn | It lands mid-task with no priority filter |
| N queued messages | N separate turns (owned-turn hold, FIFO) | No batching |
| Session-static | the skill index entry; the 24.8 KB body loads only if invoked | `skills/cyclops/SKILL.md` |
| Self-test | one trivial turn | `hooks.md:192-197` |

Compared with herdr-threads on Codex (primer §10), where a quiet turn also costs 0 bytes:

- **Cyclops pays:** one turn and one tool round trip **per message**, and a wake for **every** message.
- **herdr-threads pays:** an attention digest of up to 4,608 bytes in-turn, plus an `inbox` call, and wakes only when p ≤ T.

---

## 6. Limitations and bugs visible from source

1. **Mid-turn doorbells probably lose their hook ACK on Codex.** INFERRED, with high confidence, from two VERIFIED facts:
   - Codex runs `UserPromptSubmit` for steered input with the active turn's `turn_id` (`codex-rs/core/src/hook_runtime.rs:684-694`; `session/turn.rs:458-476`).
   - Cyclops dedupes confirmed lifecycle events on `turn.dedupe_key(event)`, a length-prefixed concatenation of the turn-key fields (`session_id`, `turn_id`) and the event name (`src/cyclopsd/src/turnkey.rs:33-42`; used at `ack.rs:350-361`). Codex's `UserPromptSubmit` is `turn_start_evidence = "confirmed"`, so it is not exempt as a "Candidate" (`codex.toml:35-36`).

   So the steer's `UserPromptSubmit`, which carries the doorbell, has the same key as the human's turn-start report. It returns `duplicate: true` before ACK matching runs (`ack.rs:358-361` precedes `:655`), and the receipt falls to the screen tier or to none. Cyclops's tests only show one `UserPromptSubmit` per `(s,t)` (`ack.rs:896-908`).
2. **Busy-recipient injection.** The gate ignores the manifest's `safe_states = ["idle"]` (`codex.toml:271`, unused by `gate.rs`). On Codex, a doorbell typed into a busy composer becomes a `turn/steer`-like redirect of the human's or the agent's current task. This breaks rule 4 of our delivery contract.
3. **The shared app-server daemon** may break kernel-ancestry identity for hooks and claims on Codex ≥ about 0.157 (§3.5). The design has no answer to it.
4. **Version-fragile scraping.** Measured bugs on record in the manifest:
   - The working rule was shadowed by the composer rules, so the gate "would paste into a live generation" (`codex.toml:176-183`).
   - A case-sensitive `Esc to interrupt` clause "could never fire" (`:185-187`).
   - Chip blindness: "the gate would paste a second message on top of the first" (`:100-106`).
   - A truecolor glyph change on 0.149.1 (`:107-108`).
   - The status-row anchor depends on the model family name `gpt-` (`codex.toml:289,293,308-313`).

   Each of these is a future regression class.
5. **No priority or threshold.** Every message wakes Codex and costs a full turn.
6. **No use of Codex's native queue.** `codex queue`/`thread/queue/add` gives an idle-only, server-side, draft-safe wake with no screen reading (primer §3). It is listed in the design review but not implemented. VERIFIED (absence).
7. **Uncapped sender summary.** It can push the doorbell into a chip, and the readback then proves only "something was pasted" (`codex.toml:262-270`).
8. **Stale docs.** The manifest comment says the gate refuses `working`, which contradicts the 1.1.0 gate. `STATUS.md:44-49` says "twelve" manifests while 52 ship (overview.md).
9. **Inactivity.** There have been no pushes since 2026-09-08. Codex has shipped 0.150 to 0.163 since, unmeasured, including the daemon default.

---

## 7. Clever ideas worth stealing

1. **Token-in-prompt hook ACK, keyed on `(session_id, turn_id)`.** The doorbell carries an attempt token. Codex's `UserPromptSubmit.prompt` echoes it within about 30 ms, which proves that this exact typed line became model input. Cheap, and stronger than a screen read. We type `herdr-threads: wake up`. A short nonce in the wake marker plus a `UserPromptSubmit` match would give us a verified wake receipt per attempt. Watch the steer dedupe trap in §6.1.
2. **Owned-turn hold.** Cyclops holds the next wake until the `Stop` of the turn the previous wake started, matched by `turn_key_fields`, so it never stacks wakes. Codex is the one vendor where the start and end hooks share a turn key. This applies directly to our Codex wake path.
3. **Escaped-capture composer discrimination.** Bold `›` with dim ghost text versus bare typed text versus a colored `Pasted` chip, and transcript `›` (bold+dim) excluded. This is more precise than plain-text placeholder matching. Our `src/harness/codex/composer.rs` could check it as a second sensor.
4. **The composer semantic is evaluated apart from the runtime winner** (`fusion.rs:4232-4240`). A draft typed during "Working" is still seen.
5. **Measured modal table with decline keys.** The Codex update dialog's preselected "Update now (runs brew upgrade)" makes a blind Enter dangerous. Cyclops declines with `3`, then `Enter`, and re-reads the screen before the confirming key (`codex.toml:225-239`; `gate.rs` `send_decline_keys`). Worth checking against our Native wake: we type into an "empty composer", but a modal is not a composer.
6. **Screen-ended latch for hookless turn ends.** Codex 0.149.1 emits no `Stop` on a cancelled approval. Cyclops ends the latch on the exact interruption text plus a ghost composer (`codex.toml:196-212`).
7. **Write-boundary accounting.** `spool` (retryable) is separate from `commit` (post-write), and `PasteCommandUnwritten` is the only failure that counts as "nothing written" (`inject.rs:57-76,111-145`). This gives an honest `submitted_unverified` / `attention_required` vocabulary and no blind retries.
8. **User-level Codex hooks, never project-local.** The measured finding is that untrusted directories silently load zero project hooks (`codex.toml:64-72`). We already install into `$CODEX_HOME/hooks.json`. The observation is still worth recording.
9. **The doorbell withdraws itself if the body was pulled first** (`DELIVERY.md:148-155`). This is like our in-turn delivery making a later wake unnecessary. Cyclops makes it an explicit, durable withdrawn state under the same store lock.

---

## Verification

Adversarial pass, 2026-10-23, against the clone at `a84d44b` and the Codex `rust-v0.163.0` clone.

- Checked and held: repo stats (`gh api`: MIT, 14 stars, 0 forks, created 2026-08-01, pushed 2026-09-08); every `codex.toml` line range cited (header, `[hooks]`, all rules, `[injection]`, trailer regexes); `STATUS.md:44-49,106-108,137`; `DELIVERY.md` ranges (accept, gate, paste, capture, Enter, receipt, hold list, doorbell, withdraw, raw, identity, config); `fusion.rs:2380-2398,3157-3178,4233-4240`; `inject.rs:57-152,188-205`; `gate.rs` `admit` (1203, ends about 1323) and `send_decline_keys` (1592); `ack.rs:143-155,332-362,654-702,896-908`; `hook.rs` silence (no stdout writes) and `CYCLOPS_AGENT`; the Codex hooks template; `hooks.md` ranges; `consumer.rs:168-183`; `hookset.rs:164-178,575-595,1211-1248`; `safe_states` unused by the gate; 52 manifests, 5 measured; design-note quotes; Codex `turn.rs:455-476`, `turn.rs:933-953`, `hook_runtime.rs:677-702`, `LARGE_PASTE_CHAR_THRESHOLD = 1000`; README "The composer rule"; `wait.md` "Limits".
- Changed: the claim-envelope renderer is `print_claim_payload` (`src/cyclops/src/main.rs:2992-3019`), not `terminal.rs` `render_payload`, which is the raw payload with a different header and reply hint; the doorbell function is `render_doorbell_v4`; the `claims_prompt` location; the dedupe key form (length-prefixed, `turnkey.rs:33-42`), with the §6.1 inference unchanged; the grep note now names Cyclops's own `--no-daemon` hits; `clear_keys` is parsed but never sent (it was "not traced").
