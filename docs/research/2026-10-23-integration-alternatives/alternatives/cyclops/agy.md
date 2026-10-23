# Cyclops x Antigravity CLI (agy)

- Upstream: https://github.com/cyclops-team/cyclops. The clone was read but not executed: `scratchpad/alts/cyclops`, commit `a84d44b8f5476c10ecf849d46370d2cb0238dab9` (2026-09-06).
- Paths are relative to the clone root unless prefixed `herdr-threads:`.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from them. **unknown** means no source answers it.
- Background: [overview.md](overview.md) and the harness primer [../../primers/agy.md](../../primers/agy.md).

## Support status

**Native and measured, but Cyclops itself calls it the weakest member of the fleet.** VERIFIED: `resources/manifests/agy.toml:6-7` ("THIS IS THE WEAK MEMBER OF THE FLEET").

| Fact | Value | Evidence |
|---|---|---|
| Manifest | `resources/manifests/agy.toml`, id `agy`, `process_names = ["agy"]`, `launch = "agy"` | VERIFIED `agy.toml:9-17` |
| Full-ruleset version claim | `version_tested = "1.1.11"` | VERIFIED `agy.toml:12` |
| Later narrow live evidence | Several later versions were measured cell by cell: 1.1.13 (no ghost text), 1.1.18 (clear-key probe), 1.1.21 (escaped prompt identity, file-access modal), 1.1.22 (claim, reply, human-draft hold, truecolor trailer) and 1.1.23 (wrapped composer) | VERIFIED `findings.md` F58 (1.1.13), F65 (1.1.18), F70/F71 (1.1.21), F77 (1.1.22), F83 (1.1.23); `STATUS.md:138` lists only 1.1.11 and the 1.1.22 cells |
| Receipt tier | `ReceiptRequirement::Screen`. Claude, Codex, Cursor and Kimi get `ExactHook` | VERIFIED `src/cyclops/src/consumer.rs:84-90` |
| Hook ACK | None. The self-test "always exits 1" on agy | VERIFIED `docs/reference/hooks.md:196-199` |
| Measurement host | macOS with tmux 3.6a. The pane title reads `mac` | VERIFIED `findings.md` F70/F71 headers; `agy.toml:62-66` |
| Linux agy | Not measured | INFERRED: no Linux capture is cited for agy |
| Version gap vs. us | Every Cyclops measurement is 1.1.x. Our captures and gates are 1.3.2/1.3.3. Nothing in Cyclops was measured on 1.3.x | VERIFIED (primer §0) |
| Project activity | 14 stars, 0 forks, MIT. Created 2026-08-01, last push 2026-09-08 | VERIFIED `gh api repos/cyclops-team/cyclops` (queried 2026-10-23) |

## Injection mechanism

**Summary: the pane is the only path into agy's context.** Cyclops pastes a one-line "doorbell" into agy's composer through tmux and presses Enter. The model reads that line as a user prompt and runs `cyclops inbox claim m-att_…` as an agy `run_command` tool call. The claim's tool result carries the body.

No agy hook output is used. `injectSteps`, `systemMessage`, `userMessage` and `ephemeralMessage` are never emitted. VERIFIED: `cyclops hook` "never prints … exit 0 no matter what" (`src/cyclops/src/hook.rs:3-6,37-43`; `src/cyclops/src/main.rs:1023-1027`). A grep of `src/` for `injectSteps` finds no hit.

### Code path

1. **Accept.** `msg.send` fsyncs the message and one mailbox entry per recipient before answering. VERIFIED `docs/development/DELIVERY.md:38-44`.
2. **Gate.** `admit` (`src/cyclopsd/src/delivery/gate.rs:1203-1325`) runs on one fresh escaped capture. See "Wake mechanism".
3. **Spool.** `TmuxInjector::spool` runs `load-buffer` into a per-attempt buffer named `cyc-<pid>-<seq>`, through a private 0600 spool file. VERIFIED `gate.rs:584-610`; `src/cyclopsd/src/delivery/inject.rs:78-106`.
4. **Occupant re-check.** The occupant is checked again just before the paste (`gate.rs:611-616`).
5. **Commit (the write boundary).** The `on_write` closure records `writing` with the exact binding (pane root, leader, agent generation, manifest) and claims the composer barrier. Then it runs `paste-buffer -p -d`, a bracketed paste that deletes the buffer. VERIFIED `inject.rs:111-145`; `gate.rs:622-660`.
6. **Verify.** The pane is re-captured at 0, 120, 240 and 480 ms (`VERIFY_DELAYS_MS`, `src/cyclopsd/src/delivery/mod.rs:72`). Each capture is tested with `exact_staging_proof`:
   - the escaped prompt `ESC[94m>ESC[39m` followed by the exact doorbell bytes;
   - continuation rows with agy's two-cell gutter stripped (`composer_continuation_regex = '^  (?P<content>.*)$'`);
   - the styled trailer immediately below: a box rule, then a `Gemini … · Ctx:` status row.

   VERIFIED `inject.rs:188-239`; `agy.toml:205-225`.
7. **Enter.** One more occupant check, then `send-keys Enter` (`[injection].submit`). It is pressed once and never re-pressed. VERIFIED `gate.rs:680-718`; `agy.toml:188-190`; `DELIVERY.md:67-68`.
8. **Receipt.** agy has no hook ACK, so the receipt is a screen check at 250, 750, 1500, 3000 and 5000 ms (`mod.rs:63-67`):
   - Confirmed if the doorbell left the composer *and* (the working spinner was seen *or* the composer window changed while the id had staged). VERIFIED `gate.rs:2700-2770`, rule at `:2767`.
   - Otherwise, after 5 s, the attempt is `notified` with no verifier. VERIFIED `DELIVERY.md:69-74`.
9. **Body.** The model runs `cyclops inbox claim m-att_<22 chars>`. The output is an envelope: `[cyclops cyc-…] TO: … FROM: … SUBJECT: …`, the body, a `Reply: cyclops reply …` hint and `[cyclops:end cyc-…]`. VERIFIED `skills/cyclops/SKILL.md:255-270`.

### Payload format

```text
[cyclops from implementer to reviewer] The rate limiter is ready for review. | cyclops inbox claim m-att_--AAAAAAQACAAAAAAAAAAQ
```

VERIFIED `DELIVERY.md:127-146`.

- Header: `[cyclops from <sender> to <recipients>]`. Up to three recipient labels are listed; beyond that it is `<first>, <second>, +N`, or `to all` for a broadcast.
- Summary: one line.
- Claim command: the 22-character token encodes the 128-bit attempt id.
- The body is never in the pane.

### When it fires

As soon as the per-recipient FIFO worker reaches the message and the gate admits it. That includes **mid-turn**: a working agent is admitted (see below). There is no deferral to agy's next model call, because no hook is used for content.

### Size bounds

- **Summary.** A derived summary is at most 200 characters on one line. A sender-supplied summary has no cap; the CLI warns above 160. VERIFIED `DELIVERY.md:40-44`.
- **Wrapping.** On agy 1.1.23 a 70-column pane wraps the doorbell across the prompt row and three continuation rows. The manifest therefore widens the composer window to 8 rows. VERIFIED `agy.toml:93-96`; F83.
- **Body.** There is no body-specific cap in the delivery docs. The only bound found is the daemon wire frame, 1 MiB of JSON (`src/cyclops-proto/src/wire.rs:31-32`). The claim output is not truncated or paged. INFERRED: no truncation code was found.

### Framing

- One header line per doorbell. The full skill (`skills/cyclops/SKILL.md`, 24,772 bytes) explains the protocol once.
- The installer seeds the skill at `~/.gemini/antigravity-cli/skills/cyclops/SKILL.md`. VERIFIED `docs/guides/install.md:181`; `src/cyclops/src/consumer.rs:323-330`.
- agy lists skills by name and path in the system prompt, and the body is read on demand (primer §3).

### Dedupe and ACK

- **Repeat claims.** A repeat claim returns the same payload "and creates no second task". VERIFIED `SKILL.md:267-268`.
- **Claim before the paste.** A claim made over the socket before the write boundary withdraws the doorbell, so nothing is pasted. VERIFIED `DELIVERY.md:148-155`.
- **Doorbell ACK on agy: screen-only.** VERIFIED `agy.toml:34-49`:
  - agy has no `UserPromptSubmit`.
  - `PreInvocation` fires 48 ms after Enter, but its payload carries no prompt text, so "the hook ACK degrades from 'this exact message arrived' to 'a turn started'".
  - The manifest names the fix, reading `transcriptPath` and matching the marker there, but does not implement it. INFERRED: no transcript reader was found in `src/`.
- **Claim as receipt.** A claim after Enter is also a receipt (`DELIVERY.md:74`). On agy this is the only exact proof that the model got the message.

## Wake mechanism

**Summary: the doorbell is the wake.** Pasting the line and pressing Enter submits a user turn. Cyclops uses none of agy's own mechanisms: `agentapi`, `/schedule`, Remote Control, stream-json, `Stop` `continue` or `PostInvocation` `force_continue`. VERIFIED: there is no code for any of these (`grep -rn agentapi src` is empty). The skill calls the doorbell "the wake-up signal" (`SKILL.md:209-211`).

### Idle and busy detection (screen rules, `agy.toml`)

| Rule | State | Match | Priority |
|---|---|---|---|
| `composer_empty` | idle / clean | `^\s*>\s*$` in the bottom 4 rows | 1000 |
| `composer_has_input` | `idle_with_input` / `human_input` | Escaped `^\x1b\[94m>\x1b\[39m\s+\S`, so a transcript echo (`ESC[1mESC[34m>`) is not taken for the composer | 1050 |
| `screen_working` | working | A braille spinner `[\x{2800}-\x{28FF}]+` plus a word | 1100 |
| `quota_exhausted` | blocked_quota | "Individual quota reached" | 1400 |
| `feedback_survey` | blocked_modal | Auto-declined with key `0` | 1300 |
| `file_access_permission` | blocked_permission | Three clauses; never dismissed | 1500 |

VERIFIED `agy.toml:68-186`.

- **Spinner priority.** `screen_working` must outrank `composer_empty`, because agy clears the composer at turn start and shows a bare `>` for the whole turn. The first manifest got this wrong and read every turn as idle. VERIFIED `agy.toml:111-119`.
- **Hooks as liveness.** Hooks feed turn-state fusion: `PreInvocation` is treated as `turn_start` and `Stop` as `turn_end` (`agy.toml:24-27`). A `PreInvocation` creates an unkeyed turn start that only an unkeyed end can settle (`src/cyclopsd/src/fusion.rs:9175-9191`, test).
- **Payload fields mostly ignored.** `initialNumSteps`, `fullyIdle` and `terminationReason` are never read (VERIFIED: a grep of `src/` finds no hit). `conversationId` and `invocationNum` are read only as the fallback report-dedupe key `(session, turn)` (VERIFIED `src/cyclopsd/src/ack.rs:143-155`, agy test at `:890-891`). So `Stop` with `fullyIdle:false` (background tasks still running) counts as a turn end. INFERRED.

### What the gate admits (`gate.rs:1203-1325`)

- **Holds:**
  - a dead pane, a missing pane, or copy-mode;
  - no manifest bound;
  - `blocked_quota`, `blocked_permission`, or a `blocked_modal` without `auto_dismiss` (the feedback survey has `auto_dismiss = true`, so the gate sends its decline key `0`, at most `MAX_DECLINES` times, and retries; `gate.rs:1245-1286`);
  - `composer_is_held`: a seen human draft, or a doorbell this recipient has staged and not yet consumed, or the turn that doorbell started (`fusion.rs:2386-2398`).
- **Proceeds:** everything else, including `working` and `unknown`. In step 6 an `Unknown` state is admitted whenever the foreground leader is the agent: `binding.leader == binding.agent || det.state != AgentState::Unknown` (`gate.rs:1309`).
- **`safe_states` is never read.** The manifest's `safe_states = ["idle"]` (`agy.toml:193`) is parsed but used nowhere outside the manifest struct and its tests. VERIFIED `src/cyclops-manifest/src/lib.rs:936,1652`. The README states the real policy: "A working agent gets its doorbell during the turn; the vendor queues the line" (`README.md:206`).
  - Whether agy *does* queue a line typed and submitted mid-turn is unknown. No Cyclops finding measures it for agy, and our primer does not establish it.

### Draft safety

- **Detecting a draft.** A positively seen human draft (the escaped `composer_has_input` rule) holds the attempt. The hold is released by a pane event (the draft erased, the turn ended), never by a timer. After `gate_hold_notify_ms` (120 s) the admin gets one ping. VERIFIED `DELIVERY.md:50-57,96-103,270-271`; live on 1.1.22: "human-draft hold, partial deletion refusal, and final-backspace release" (`STATUS.md:138`).
- **No clearing.** On agy 1.1.18 the staged payload survived `C-c`, `Escape`, `C-a C-k` and `Home C-k`, so no clear sequence ships, "automated clear keys remain unavailable". VERIFIED `findings.md` F65; `STATUS.md:138`. Our primer found that `ctrl+u` clears on 1.3.2, and repeated presses clear a multi-line draft (primer §8). Cyclops never tested `ctrl+u`.
- **Race.** "A person can type between the final capture and the tmux write; there is no input lease across that gap." VERIFIED `STATUS.md:106-108`.

### Latency

- `PreInvocation` fires 48 ms after Enter (`agy.toml:37-38`).
- The pipeline adds one gate capture, a spool round trip, up to 480 ms of verify re-reads (the loop sleeps `delay - last_delay`, so the offsets are cumulative; `inject.rs:200-205`), then Enter. INFERRED: about 1 s from admission to the turn starting.
- A held attempt waits on events with no upper bound.

### Failure modes

1. **The first paste after startup is dropped.** Measured: "the first paste after TUI startup did not stage" (`agy.toml:195-198`). The manifest says verify-before-submit "catches this", but the 1.1.0 code does not recover:
   - If staging is never observed, `inject` falls through: "The doorbell proceeds to submit once and finishes as delivered_unverified" (`inject.rs:227-238`).
   - Enter is pressed on an empty composer (`gate.rs:712-718` records `submitted_unverified`).
   - No spinner and no change follow, so the receipt times out to `notified` with no verifier (`gate.rs:810-813`).
   - Nothing re-rings it. There are no clock retries (`DELIVERY.md:16`), and the 1.0 `unclaimed_reminder_ms` key is ignored (`DELIVERY.md:272-274`).
   - The message stays claimable in the mailbox, but the agent is never woken for it.

   INFERRED from the code path. The manifest comment predates the 1.1.0 "never re-paste" rule. Earlier code withheld Enter on a failed proof instead: F77 (agy 1.1.22, 2026-08-28) records `attention_required` / `verify_failed` with no Enter. The commit read here submits once regardless (`gate.rs:705-718`; `DELIVERY.md:63-66`).
2. **Unmodelled permission menus.** The manifest models only the "File access" modal (F71). agy 1.3.2's run-command prompt ("Run this command? > 1. Yes, run command …", herdr-threads `tests/fixtures/agy/composer/permission-prompt.read-detection.txt:19-23`) matches no Cyclops rule unless its `>` happens to paint exactly `ESC[94m>ESC[39m`. Its styling is unknown, and Cyclops has no fixture for this screen (`src/cyclops-manifest/tests/fixtures/agy_*` lists only file-access, working and wrapped-composer captures).
   - An unmatched screen fuses to `unknown`, and the gate admits `unknown` when agy is the foreground leader (`gate.rs:1309`).
   - INFERRED hazard: the paste plus Enter could land on the menu, and Enter would confirm the selected default ("Yes, run command").
   - The F71 note says fusion "correctly failed closed to `unknown`". For writes, `unknown` is not closed.
3. **False screen receipts on busy panes.** Tier-2 evidence accepts `working_seen` alone (`gate.rs:2767`). A doorbell pasted while agy is already spinning can be "confirmed" by the turn that was already running. INFERRED.
4. **A quota screen blocks for days.** The quota screen "Resets in 135h…" holds the gate. It is a terminal state that needs an operator (`agy.toml:155-171`).
5. **Restart.** In-flight attempts become `attention_required` and need a manual `cyclops requeue` (`DELIVERY.md:203-207,228-231`).
6. **Version drift.** Every 1.1.x release changed something the screen rules depend on:
   - the composer glyph styling (1.1.21, F70);
   - the trailer colour and an empty `Ctx` (1.1.22, F77);
   - the wrap gutter (1.1.23, F83). The fix needed a manifest-digest-keyed legacy shim, `LEGACY_AGY_PRE_GUTTER_MANIFEST_SHA256` (`inject.rs:775-797`).

   1.3.x is unmeasured. Our 1.3.2 captures show a different footer vocabulary (`? for shortcuts`, `esc to cancel`; primer §8). INFERRED: the 1.1.x trailer regex (`Gemini … · Ctx:`) may not match 1.3.x. If it does not, every exact proof fails, every doorbell becomes `submitted_unverified`, and screen receipts drop to `working_seen` only.

## Setup and installation footprint on agy

| What | Where | Evidence |
|---|---|---|
| Prepared hook artifact (never touches vendor config) | `$CYCLOPS_HOME/hooks/agy/<label>/hooks.json`; the operator copies or merges it into `<workspace>/.agents/hooks.json` | VERIFIED `docs/reference/hooks.md:17-40,60-64`; `src/cyclops/src/hookset.rs:597-605` |
| Opt-in installer wiring (`cyclops start --setup-only --wire-hooks`, passed by `scripts/install.sh`) | Merges into **`~/.agents/hooks.json`** under the shared name `cyclops` with no `--agent`. The identity comes from socket peer credentials, plus `CYCLOPS_AGENT` set per pane through tmux `-e`. The original file is copied to `hooks.json.before-cyclops`. The consent marker is `~/.cyclops/vendor-wiring-consented`; `CYCLOPS_NO_VENDOR_HOOKS=1` declines | VERIFIED `hooks.md:115-145`; `hookset.rs:164-180`; `consumer.rs:323-330`; `CHANGELOG.md:185-206` |
| Events registered | `PreInvocation`, `PostInvocation`, `Stop`, and `PreToolUse`/`PostToolUse` with matcher `"*"`. Each is a distinct self-tagging command `cyclops hook <Event> [--agent <label>]`. No `SessionStart` | VERIFIED `resources/hooks/agy/hooks.json.tmpl:17-35` |
| Skill | `~/.gemini/antigravity-cli/skills/cyclops/SKILL.md`. Created only if the parent directory already exists and is private | VERIFIED `install.md:181,193-205`; `hooks.md:122-131` |
| Permissions | None written. No `command(cyclops)` allow rule in `~/.gemini/antigravity-cli/settings.json` | VERIFIED: a grep for `antigravity-cli/settings` and `command(cyclops` finds nothing |
| Plugin, MCP, sidecar | None | VERIFIED (grep) |

Footprint concerns:

- **Path disagreement.** Cyclops's own sources disagree about where agy reads hooks:
  - The self-test diagnosis says "agy only reads .agents/hooks.json in the workspace" (`src/cyclopsd/src/selftest.rs:6-7,409-412`).
  - The installer writes `~/.agents/hooks.json`.
  - The agy docs name the global file as `~/.gemini/config/hooks.json` (primer §2.1).

  INFERRED: `~/.agents/hooks.json` loads only when agy's workspace or customization root is `$HOME`, so installer-wired hooks may never fire for agents run in a project directory. Whether agy walks up to `~/.agents/` is unknown. The manifest's "customization root" wording (`agy.toml:21`) and agy's walk-up for context files (`agy.toml:227-232`) leave it open. The symptom would be `hooks unverified`, which only degrades liveness. Delivery still works.
- **Fail-closed tool hooks.** `PreToolUse` with an empty stdout is relied on to allow. The repo probe says empty allows and `{}` denies; one third-party source disagrees (primer §2.3). If a future agy treats empty as deny, every agy tool call would be blocked. INFERRED risk.
- **Hook overhead.** Each tool call spawns two Cyclops processes (pre and post), and each model call spawns one or two (`PreInvocation`/`PostInvocation`). The hook budget is 3 s (`hook.rs:32`). The typical cost is "single-digit milliseconds" (`hook.rs:29-30`), but a wedged daemon adds up to 3 s to every agy tool call.
- **Merge risk.** "One malformed key silently drops the whole file" (primer §2.1). Merging into a shared `hooks.json` carries that risk; the backup copy mitigates it.

## Token cost

**Per message (INFERRED from the formats; nothing measured):**

1. **Doorbell user turn.** About 100-250 characters: header, summary (up to 200 or more) and claim command. agy wraps typed input in `<USER_REQUEST>` plus an `<ADDITIONAL_METADATA>` local-time block. That wrapping was observed for hook `userMessage` steps; INFERRED for typed input (primer §2.3). Roughly 60-120 tokens.
2. **A mandatory extra tool round trip.** The model emits a `run_command` call for `cyclops inbox claim …`, then reads the envelope. The envelope costs header (~30 tokens) + body (unbounded) + reply hint + end sentinel (~40 tokens).
3. **Permission prompt.** If agy runs in `request-review` mode, the claim command needs approval each time. No allow rule is seeded, so either a human clicks or the agent stalls. INFERRED.
4. **Every doorbell starts a new turn**, priority aside. Cyclops has no priority or threshold.

**Per quiet turn:**

- Zero model-visible bytes: hooks print nothing (`hook.rs:3-6`).
- The cost is process spawns only: 2 per tool call and 1-2 per model call.
- One-time cost: the skill name and path in the system prompt. Reading the 24.8 KB skill body, if the model opens it, costs about 6k tokens (INFERRED).

**Compared with herdr-threads on agy:** a default p1 send costs nothing until the next turn's `PreInvocation` `systemMessage` carries the body. There is no claim call and no extra tool round trip (primer §10).

## Limitations and bugs

1. **No in-context delivery at all.** The working mid-turn path, agy's `PreInvocation` `injectSteps` before each model call, is unused (primer §2.5). Every message costs a turn plus a tool call. *VERIFIED, by absence.*
2. **The first doorbell after agy starts can be lost silently** and is never re-rung (failure mode 1). *INFERRED from the code path.*
3. **ACK on agy is "a turn started"**, never message-specific. The `transcriptPath` marker match the manifest proposes is not built. *VERIFIED.*
4. **The manifest contradicts the code on busy panes.** `safe_states = ["idle"]` and the `unsafe_states` list are dead configuration; the gate pastes into working panes. *VERIFIED.*
5. **`unknown` admits writes.** Combined with an unmodelled run-command permission menu, this is a possible command-approval hazard (failure mode 2). *INFERRED.*
6. **Hooks wired for a non-loading path.** The installer path `~/.agents/hooks.json` contradicts the project's own "workspace only" diagnosis (setup section). *INFERRED risk.*
7. **No `SessionStart` registered.** It exists on current agy (primer §2.2) but not on 1.1.x. Fusion's liveness-based "unknown → idle" admission requires an authenticated `SessionStart` or `UserPromptSubmit` edge (`fusion.rs:2414-2427`; the edge check is `admitting_edge_qualifies`, `src/cyclopsd/src/ack.rs:177-198`, and agy's manifest lists neither event, `agy.toml:23`), so it can never qualify for agy. *INFERRED.* This does not block writes, but it keeps agy panes in `unknown` more often, which item 5 then admits.
8. **No clear key**, so a staged doorbell left in the composer cannot be removed automatically. It reads as `human_input` and blocks later doorbells until a human clears it (`gate.rs:816-821` comment: "A doorbell left in the composer reads as human input and holds"). *VERIFIED.*
9. **Unaddressed agy states:**
   - Subagents (`● Agent(<name>) Working`) are not modelled.
   - `fullyIdle` is ignored.
   - `/clear` and resume have no handling.
   - The conversation id is used only as a report-dedupe key (`ack.rs:152`), not to track session identity or delivery. *VERIFIED.*
10. **Never measured on 1.3.x, or on Linux.** *VERIFIED.*

## Clever ideas worth stealing

1. **Escaped-glyph discrimination of the active composer.** The active prompt is `ESC[94m>ESC[39m`; a transcript echo is `ESC[1mESC[34m>` (F70, `agy.toml:85-91`). Both read `> text` once escapes are stripped. If our `AgyComposer` parses plain text, an echoed `> …` row above the composer could be mistaken for a draft (safe direction) or confuse the bottom-rule anchoring. Worth checking against our 1.3.2 fixtures. INFERRED.
2. **Spinner-first busy detection.** Key "working" on the braille spinner glyph range, not on an open status-word set (`agy.toml:120-130`). It complements our footer `esc to cancel` signal, and might catch screens where the footer is absent.
3. **Measured named modals with explicit decline keys.**
   - The quota screen as a terminal, operator-alert state, distinct from "busy". We should check what our composer does on agy's "Individual quota reached".
   - The feedback survey, declined with `0`. Our composer presumably reads it `Unreadable`, so no wake, but nothing tells the operator. INFERRED.
4. **Verify the staged bytes before Enter, with a terminal trailer anchor.** Staging is proven only when the exact bytes sit immediately above the measured trailer rows (`inject.rs:336-405`). Our wake text is content-free and short, so the benefit is smaller, but a readback after typing `herdr-threads: wake up` would catch agy's measured first-input-after-startup drop (`agy.toml:195-198`). Does Herdr's `send-text` hit the same drop on a freshly started agy? Unknown; worth a probe in our native gate.
5. **Honest receipt vocabulary.** `submitted` / `submitted_unverified` / `notified(verified_by hook|screen|none)` / `attention_required`, plus "never retried on a clock". It is a cleaner way to report our idle wakes than a single bit.
6. **Event-driven hold release.** A held wake retries on the next pane event (the draft erased, the turn ended), with one admin ping after 120 s, instead of polling.
7. **The `transcriptPath` idea**, which Cyclops proposed and we already ship: we confirm deliveries by scanning `SYSTEM_SDK` transcript steps (primer §10). Cyclops's own analysis independently reached the same conclusion, that payload-matchable ACK on agy needs the transcript.

## Verification

Adversarial pass on 2026-10-23 against clone `a84d44b` and `gh api repos/cyclops-team/cyclops`.

- **Re-read and confirmed:**
  - `agy.toml` rule ids, states, priorities, regions and every cited line range;
  - `consumer.rs:84-90,323-330`; `hook.rs:3-6,29-43`; `main.rs:1023-1027`;
  - `gate.rs` `admit` (1203-1325, `:1309`), the paste/Enter path (584-718), `settle_without_receipt` (810-821) and `tier2_evidence` (`:2767`);
  - `inject.rs:78-145,188-239,336-405,775-797`; `mod.rs:63-72`; `fusion.rs:2386-2398,9175-9191`;
  - `DELIVERY.md` sections cited; `SKILL.md` (24,772 bytes, `:209-211,256-278`); `hooks.md`; `install.md:181`; `hookset.rs:164-180,597-605`; `selftest.rs:6-7,409-412`;
  - `CHANGELOG.md:185-206`; the agy hooks template; `safe_states` usage (struct and tests only); `wire.rs` 1 MiB cap; the four agy fixtures;
  - no `injectSteps` or `agentapi` in `src/`;
  - repo stats (14 stars, 0 forks, MIT, created 2026-08-01, pushed 2026-09-08).
- **Corrected:**
  - `invocationNum`/`conversationId` *are* read, as the fallback dedupe key (`ack.rs:143-155`). They had been listed as never read or untracked.
  - Verify re-reads total 480 ms, not 840 ms.
  - The `+N` header format: with more than three recipients it is `<first>, <second>, +N`.
  - An auto-dismiss modal (the feedback survey) gets decline keys, not a plain hold.
  - The version-evidence citation: `STATUS.md:138` names only 1.1.22.
  - The herdr-threads permission-prompt fixture lines are `:19-23`.
  - Added the `ack.rs` location of the liveness-edge check.
  - Noted that F77 documents the older withhold-Enter behaviour.
