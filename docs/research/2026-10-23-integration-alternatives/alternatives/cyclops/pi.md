# Cyclops x Pi coding agent

- Alternative: Cyclops, <https://github.com/cyclops-team/cyclops>. Clone `scratchpad/alts/cyclops`, commit
  `a84d44b8f5476c10ecf849d46370d2cb0238dab9` (2026-09-06, a single squashed commit: `git log` shows 1 commit).
  Paths without a prefix are relative to that clone root.
- Harness: Pi (<https://github.com/badlogic/pi-mono>, now <https://github.com/earendil-works/pi>). Clone
  `scratchpad/alts/pi-mono`, commit `a7b94a03`; paths prefixed `pi:` are relative to that clone. Harness facts
  not re-derived here come from `../../primers/pi.md` (cited as "primer §N").
- Repo stats (from the overview's `gh api` call, 2026-10-23): v1.1.0, 14 stars, 0 forks, last push 2026-09-08.
- Nothing was executed from either clone. Tags: VERIFIED = read in source or docs at the cited line;
  INFERRED = my reading or a combination of verified facts; "unknown" = not determinable from what I read.

## Summary

Cyclops "supports" Pi only through its universal tmux path. It ships a docs-only manifest with no screen rules
and no hooks, and it seeds the skill. A message reaches Pi as one doorbell line that is pasted into Pi's editor,
followed by Enter. Pi's model then runs `cyclops inbox claim m-att_...` through its `bash` tool to fetch the body.
Nothing in-process is used. Pi's extension API, which can inject messages and start turns without keystrokes, is
not used at all. In this configuration Cyclops cannot see a human draft, an idle or busy state, a modal, or a
turn end. Every doorbell is pasted and submitted blind, and recorded as `submitted_unverified`, then `notified`
with no verifier. Pi happens to tolerate this better than most harnesses, because Enter while busy becomes a
steer rather than an interrupt. A human's draft is not protected.

## Support status

| Item | Finding | Evidence |
|---|---|---|
| Tier | "Unverified terminal CLI with a manifest": written from vendor docs, `version_tested = "unverified"` | VERIFIED `resources/manifests/pi.toml:2-4,36`; `docs/guides/install.md:486-496,538` |
| Measurement | "Nothing was measured: no pi binary was run on the evidence host and no turn, title, or screen was captured" | VERIFIED `resources/manifests/pi.toml:3-4` |
| Screen rules | Only one rule, `title_useless` (state `unknown`, region `pane_title`). There is no idle, working, human-input, modal or quota rule, and no composer prompt or trailer layout | VERIFIED `resources/manifests/pi.toml:41-47`; the whole file is 55 lines |
| Hooks | None. "Pi's events ... are handled by in-process TypeScript extensions ... so no [hooks] or [hooks.wiring] table is declared. An extension that shells out to `cyclops` would be the vendor-side bridge; none ships here." | VERIFIED `resources/manifests/pi.toml:17-23` |
| Manifest loading | Compiled into the binary and written to `~/.cyclops/manifests` on setup | VERIFIED `src/cyclops/src/manifests.rs:201-202`; `docs/guides/install.md:157-163` |
| Skill | Seeded to `~/.pi/agent/skills/cyclops/SKILL.md` | VERIFIED `src/cyclops/src/skillseed.rs:317-321`; `docs/guides/install.md:538` |
| Pi reads that skill dir | Yes: user `skills: join(globalBaseDir, "skills")`; it also reads `~/.agents/skills/` | VERIFIED `pi:packages/coding-agent/src/core/package-manager.ts:2474,2484`; `pi:packages/coding-agent/docs/skills.md:61` |
| Host | tmux only. Herdr is not a target | VERIFIED overview (`README.md:9-15`) |

Net: Pi gets **generic** support. The generic mechanism does work on Pi (see "Wake" below), but with every guard
that Cyclops advertises for measured vendors switched off.

## Injection mechanism

### Channel

The pane is the only channel. There is no MCP server, no Pi extension, and no hook. VERIFIED for Pi by
`resources/manifests/pi.toml:17-23`; for Cyclops overall see the overview ("Injection mechanism", items 4-5).

### Code path (per delivery)

1. **Accept.** `msg.send` fsyncs the message plus one mailbox entry per recipient before it answers. VERIFIED
   `docs/development/DELIVERY.md:38-44`.
2. **Queue.** There is one FIFO worker per recipient. VERIFIED `DELIVERY.md:45-49`.
3. **Gate** (`gate.rs` `admit`, `src/cyclopsd/src/delivery/gate.rs:1203-1323`). For a Pi pane:
   - present, alive, not in copy-mode (1209-1222);
   - the manifest binds (1223-1226). See "Binding" below;
   - one fresh capture and fused detection (1230-1241). With no Pi rules, `det.state` is `Unknown` (INFERRED from
     the manifest content);
   - named blocks (1244-1288): none can match, because Pi has no blocked rules;
   - composer hold (1294): `fusion::composer_is_held` holds only on a positive `human_input` match or a hold the
     delivery owns. Pi has no `human_input` rule, so a human draft never holds. VERIFIED `DELIVERY.md:96-109`;
     `STATUS.md:103-105`;
   - binding (1306-1322): admitted when `binding.leader == binding.agent || det.state != AgentState::Unknown`.
     For Pi the state is always `Unknown`, so the write is admitted **only while the Pi process itself is the
     tty's foreground process-group leader**. When Pi has handed the terminal to a child (for example an
     interactive program its `bash` tool started in the foreground), the attempt holds with
     `foreground_not_agent`. VERIFIED code; the consequence for Pi is INFERRED.

   ```rust
   Some(binding)
       if binding.manifest == manifest_id
           && (binding.leader == binding.agent || det.state != AgentState::Unknown) =>
   ```
   (`gate.rs:1307-1309`)
4. **Spool and paste.** `load-buffer` goes into a per-attempt buffer named `cyc-<pid>-<seq>`, from a 0600 spool.
   The occupant is re-checked, then `paste-buffer -p -d`, bracketed when the app opted in. VERIFIED
   `src/cyclopsd/src/delivery/inject.rs:78-145`; `gate.rs:586-615`. Pi enables bracketed paste (primer §6;
   `pi:packages/tui/src/terminal.ts:211-212`).
5. **Verify.** There are capture retries at 0/120/240/480 ms (`src/cyclopsd/src/delivery/mod.rs:72`;
   `inject.rs:202-226`). The exact-row proof requires the manifest's `composer_trailers` layout, and
   `trailer_follows` returns `false` when the layout is empty:

   ```rust
   // Unmeasured layout cannot answer the question.
   if layout.is_empty() || layout_esc.len() != layout.len() {
       return false;
   }
   ```
   (`inject.rs:340-343`). For Pi, staging therefore **never** verifies, and every Pi doorbell takes the
   "Staging was unobservable. The doorbell proceeds to submit once" branch (`inject.rs:227-237`). VERIFIED code;
   that it holds for every Pi doorbell is INFERRED from `pi.toml` having no trailers.
6. **Enter.** One more occupant check, then `send-keys Enter` exactly once (`gate.rs:676-713`). It is recorded as
   `record_submitted_unverified()` because staging did not verify (`gate.rs:714-718`).
7. **Receipt.** There is no hook tier for Pi. The screen tier needs either `working_seen` or `changed && id_staged`
   (`gate.rs:2758-2771`); Pi has no working rule and `id_staged` is false. After `SCREEN_ACK_DEADLINE` = 5 s
   (`delivery/mod.rs:64`), the attempt settles as `notified`, no verifier, and the composer barrier is released
   (`gate.rs:810,815-822`). A later `cyclops inbox claim` by the recipient is the only positive receipt
   (`DELIVERY.md:73-74,148-155`). VERIFIED code; INFERRED for Pi.

### Payload format

One line, format 4:

```text
[cyclops from <sender> to <recipients>] <summary> | cyclops inbox claim m-att_<22 chars>
```

VERIFIED `src/cyclops-proto/src/notification.rs:992-1002`; `DELIVERY.md:127-146`. The body never reaches the pane
on this path. A derived summary is at most 200 characters on one line. A sender-supplied summary is one line with
no cap; the CLI only warns above 160 characters (`DELIVERY.md:40-44`). The sample doorbell is 127 bytes.

The model then runs the claim in its `bash` tool and gets the envelope:

```text
[cyclops cyc-...] TO: reviewer  FROM: admin  SUBJECT: ...
<body>
Reply: cyclops reply cyc-... --summary "..." --body "..."
[cyclops:end cyc-...]
```

VERIFIED `skills/cyclops/SKILL.md:270-276`.

How Pi receives the line (VERIFIED in Pi; the combination with Cyclops is INFERRED):
- Idle: the bracketed paste goes into Pi's editor and Enter submits it as an ordinary user prompt, which starts a run.
- Busy (streaming): Pi's interactive mode turns Enter into `session.prompt(text, { streamingBehavior: "steer" })`
  (`pi:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3333-3338`). The steer is delivered after
  the current tool batch, before the next model request (primer §2.3). This is effectively mid-turn injection.
- Compacting: it is queued as a steer (`interactive-mode.ts:3321-3329`).
- Long sender-supplied summaries: Pi collapses pastes longer than 1000 characters or 10 lines into a `[paste #N]`
  marker and expands it on submit (primer §6). The doorbell is one line, so only the character limit can trigger
  this. The submitted text is still complete. Verification would fail regardless.

### When it fires

As soon as the per-recipient FIFO reaches the entry and the gate admits. For Pi, admission depends only on the
pane being alive, out of copy-mode and having Pi as the foreground leader. It does not depend on Pi's idle or
busy state (INFERRED from `gate.rs:1203-1323` with an empty rule set).

### Size bounds

- Pane bytes: one line. It is about 60 bytes plus labels plus the summary (at most 200 if derived, unbounded if
  supplied). VERIFIED.
- Body: Cyclops applies no truncation that I found on the claim path. It goes through Pi's `bash` tool output,
  so Pi's own tool-output truncation applies: the last 2000 lines or 50 KB, whichever is hit first, with the
  full output saved to a temp file (`packages/coding-agent/src/core/tools/truncate.ts:11-12`,
  `packages/coding-agent/src/core/tools/bash.ts:255,352`, https://github.com/badlogic/pi-mono at `a7b94a0`).
  VERIFIED. A body over that limit loses its head (tail truncation).

### Framing

- Per message: the `[cyclops from A to B]` header, the summary, and the claim command. In the envelope: a
  `TO/FROM/SUBJECT` header, a reply hint, and a `[cyclops:end <id>]` sentinel. VERIFIED.
- Static explanation lives once in the skill. Pi puts the skill's name, description and path into the system
  prompt, and the model reads `SKILL.md` on demand (`pi:packages/coding-agent/docs/skills.md:43-45`). The
  description explicitly routes on "when a line starting with `[cyclops from` appears in your composer"
  (`skills/cyclops/SKILL.md:3`). VERIFIED.
- Role in Pi's context: a plain `user` message, typed as if by the human. There is no custom-message type and no
  distinction from the human's own prompts (INFERRED; see primer §2.3 on how a custom message would differ).

### De-duplication and acknowledgement

- A repeat claim returns the same payload and creates no second task (`SKILL.md:278`).
- A claim before the write boundary withdraws the doorbell, and nothing is pasted (`DELIVERY.md:148-155`).
- The doorbell is never re-pasted, and Enter is never pressed twice (`DELIVERY.md:63-68`).
- ACK tiers for Pi: no `hook`. `screen` is unreachable (see step 7). What remains is "notified, no verifier"
  plus the claim as receipt. Completion is only an explicit reply.
- Claim identity: "a claim by the exact recipient" (`DELIVERY.md:74`). The daemon resolves the caller from the
  process tree, binding to the nearest agent process above the peer (`DELIVERY.md:185-187`, which describes
  headless registration; `SKILL.md:336-339`; overview "Headless identity"). For Pi, the
  `bash`-tool child's ancestry reaches the Pi process. That process is recognised as Pi when its argv reads `pi`,
  either through `process.title` (below) or as the script token after `node`. INFERRED.

### Binding: is the "pin the pane manually" note still true?

The manifest says Pi shows as `node`, so the pane must be pinned with `cyclops name %N <label> --manifest pi`
(`pi.toml:11-15`). I believe this is **stale on Linux** and probably unnecessary everywhere:

- The daemon now reads the script behind an interpreter. `argv_programs` returns the first non-flag token after
  `node|nodejs|bun|deno|python*` (`src/cyclopsd/src/fusion.rs:2887-2927`), and `bind_manifest_for` falls back to
  argv (`fusion.rs:1668-1677`). VERIFIED. `docs/guides/install.md:575-579` agrees. `docs/reference/MANIFESTS.md:141-148`
  ("the daemon takes only the first argv token") and the `pi.toml` comment predate this. That is doc drift (VERIFIED).
- Pi sets `process.title = APP_NAME` at startup, and `APP_NAME` defaults to `"pi"`
  (`pi:packages/coding-agent/src/cli/setup.ts:4-5`; `pi:packages/coding-agent/src/config.ts:579`). VERIFIED.
  On Linux, Node's process-title setter rewrites the argv area. So `ps -o args=` and tmux's
  `pane_current_command` (read from `/proc/<pid>/cmdline` on Linux) should read `pi`, which matches
  `process_names = ["pi"]` directly. INFERRED; not measured.
- Even without the title, an npm install runs `node /…/bin/pi`, and the script token's basename is `pi`. INFERRED.
- The pin (`cyclops name … --manifest <id>`, `src/cyclops/src/copy.rs:188`) chooses only which rules read the
  pane. It cannot establish who may speak for it. That proof always comes from live argv (`fusion.rs:1680-1690`).
  If argv were unreadable, a pin would not rescue the binding (INFERRED from `admitted_binding_observation`,
  `fusion.rs:1907-1941`).

## Wake mechanism

- **Mechanism.** The doorbell is the wake: the bracketed paste plus one `Enter` submits a user turn
  (`SKILL.md:209-211`). For Pi, no harness API is involved. VERIFIED.
- **Idle/busy detection.** None for Pi: there is no working or idle rule, no hooks, and the title is useless
  (`pi.toml:41-47`). Even for measured vendors a working agent does not hold (`DELIVERY.md:100-102`). Pi's busy
  behaviour (Enter means steer) means a doorbell to a busy Pi becomes a steer message that lands after the
  current tool batch. It neither interrupts a running tool nor is lost. VERIFIED in each codebase; the combined
  behaviour is INFERRED. Compared with our delivery contract, this violates rule 4 (no nudge while busy). On Pi
  in particular the cost is redirection of the running task, not breakage.
- **Draft safety: none.** No `human_input` rule exists, so "a doorbell to one of those panes never detects a
  draft and is effectively a raw write with a receipt" (`DELIVERY.md:105-109`; `STATUS.md:103-105`). Pi's editor
  inserts the paste at the cursor of whatever draft exists, and the following Enter submits draft plus doorbell
  as one prompt (INFERRED from primer §6). This would also happen while a human is mid-sentence.
- **The `pi.toml` "fail closed" claim is wrong for 1.1.0.** The manifest says "no idle or working rule ships and
  deliveries fail closed (not write-ready) until someone measures the composer" (`pi.toml:28-30`). The gate
  never consults `write_ready`, `safe_states` or `unsafe_states`. `admit` proceeds on `Unknown` when
  leader == agent (`gate.rs:1306-1315`). `safe_states`/`unsafe_states` are parsed
  (`src/cyclops-manifest/src/lib.rs:936-938`) but read nowhere else outside tests. `write_ready` gates only the
  reopening of an attempt that is already `blocked_pre_write` (`src/cyclopsd/src/messaging_runtime.rs:140-171,373-396`;
  `src/cyclopsd/src/mailbox/service.rs:509-516`). VERIFIED by grep and reading. This resolves the inconsistency
  the overview left as unknown: deliveries to Pi **proceed**, as `DELIVERY.md:105-109` and `install.md:489-490`
  say.
- **Modals.** Pi has no blocked rules, so a paste into Pi while one of its own dialogs is open (an extension
  `ui.select` or `confirm`, the model picker, a project-trust prompt) is typed into that dialog. Enter may then
  confirm it. What actually happens is unknown; nothing in Cyclops prevents it (INFERRED).
- **Latency.** Paste immediately on admission, verify retries up to 480 ms, then Enter. The total is under about
  1 s from gate admission (VERIFIED constants, `delivery/mod.rs:72`). Pi's start from a submitted prompt is
  immediate when idle. When busy, it waits for the end of the current tool batch (primer §2.3).
- **Holds while a doorbell is outstanding.** The owned hold ("a doorbell staged and not yet consumed, or the turn
  it started that has not ended") is released when the attempt settles without a receipt after 5 s
  (`gate.rs:815-822`). For Pi, turn end is never observed, so the next FIFO doorbell goes out about 5 s after
  the previous one, whatever Pi is doing. A burst of N messages therefore becomes N pasted prompts, each
  submitted as either a new turn or a steer. Pi's default `steeringMode = one-at-a-time` delivers queued steers
  one per turn (primer §2.3). INFERRED.
- **Failure modes.**
  - `foreground_not_agent` hold while a Pi tool owns the tty foreground (`gate.rs:1316-1317`). It is released on
    the next pane event, with one admin ping after `gate_hold_notify_ms`, default 120 s (`DELIVERY.md:24,54-56`;
    `gate.rs:1175-1193`; `src/cyclopsd/src/config.rs:60`).
  - `binding_unprovable` if `ps` cannot read argv: held once, then a durable `blocked_pre_write`
    (`gate.rs:1139-1147`).
  - Physical paste or submit failure: `attention_required`, and a human must `cyclops requeue`. There is no
    automatic retry (`DELIVERY.md:76-80`).
  - A doorbell that Pi did not actually submit (dialog open, or Enter swallowed) sits in the editor. The next
    doorbell is appended to it and both are submitted together (INFERRED from the composer-unreadable rule,
    `gate.rs:818-821`).
  - Pi in RPC, print or JSON mode has no TUI composer, so the pane path does not apply. Such a Pi would be a
    "headless" recipient using `cyclops inbox next --wait` (`SKILL.md:327-359`). That would require the model to
    call a blocking tool, which is not a wake (INFERRED).
  - Headless Pi (`--mode rpc` under a script) could still register `--self` and long-poll, but nothing in
    Cyclops drives Pi's RPC `prompt` command. Unknown whether anyone has tried.

## Setup and installation footprint on the harness

- `cyclops start --setup-only --wire-hooks` writes Cyclops's own config and manifests under `~/.cyclops`. For
  Pi, **no Pi configuration is written**: no settings, no extension, no `mcp.json`. VERIFIED `install.md:129-133,538`;
  `pi.toml:17-23`.
- Skill: it creates `~/.pi/agent/skills/cyclops/SKILL.md` only when the parent `~/.pi/agent/skills` already
  exists and is private. It never creates the directory and never overwrites an edited copy
  (`install.md:201-206`). Consent is persisted at `~/.cyclops/vendor-wiring-consented`, so later starts finish
  wiring for newly installed CLIs (`install.md:209-215`). VERIFIED. Pi also reads `~/.agents/skills/`
  (`pi:…/docs/skills.md:61`). On a machine where Codex, Cursor, goose and others got the shared copy, Pi may
  see the skill twice, and Pi warns on name collisions (`pi:…/docs/skills.md:85`). INFERRED.
- Per pane: possibly `cyclops name %N <label> --manifest pi` (operator action; see Binding).
- Launch: `launch = "pi"` (`pi.toml:39`). There is no `-e` extension and no `--session-id`.

## Token cost

- **Per message** (INFERRED from formats):
  - Doorbell as a user prompt: about 30-80 tokens (127 bytes in the sample, plus the summary).
  - The model's claim tool call: about 20-40 output tokens.
  - Envelope: about 40-60 tokens of framing plus the body.
  - One extra model request: the claim result comes back in a new request that carries the whole context. It is
    mostly cached, but it is a full extra round trip.
  - First time per session: the model probably reads the 24.8 KB `SKILL.md` (about 6k tokens). The description
    tells it to on seeing `[cyclops from` (`SKILL.md:3`; size from `wc -c`).
  - When the doorbell arrives as a steer mid-task, the model also spends a turn context-switching (unbounded,
    task dependent).
- **Per quiet turn:** zero. No hooks or extension run, and nothing is injected. VERIFIED by absence.
- **Per session:** the skill's name, description (about 445 bytes, about 110 tokens; `SKILL.md:3`) and path go into the system
  prompt (`pi:…/docs/skills.md:43`).
- **Wake cost:** every message wakes. There is no priority or threshold (overview "Wake mechanism"), so an idle
  Pi pays a full turn for every message, including FYIs.

## Limitations and bugs visible

1. **No draft guard on Pi.** A human typing in Pi's editor gets the doorbell spliced into their text and
   submitted. Cyclops documents this as a known limit (`STATUS.md:103-105`), but its own skill also tells
   agents to prefer the doorbell over `--raw`, and the two behave the same on Pi.
2. **The manifest's "fail closed" comment contradicts the gate** (see Wake). An operator who reads `pi.toml`
   will expect no deliveries to Pi. They actually happen, unguarded.
3. **Stale pin advice and doc drift** on interpreter binding: `pi.toml:11-15` and `MANIFESTS.md:141-148`
   versus `fusion.rs:2887-2927` and `install.md:575-579`.
4. **Busy-pane doorbells become Pi steers.** This is mid-task redirection that our rule 4 forbids. Each doorbell
   lands as a `user` prompt that the model may treat as a new instruction from the human, because it carries no
   type distinction.
5. **Unverifiable receipts.** Every Pi attempt is `submitted_unverified` and then `notified` with no verifier.
   The rich receipt taxonomy degenerates to "Enter was pressed" plus an optional claim.
6. **No burst coalescing.** About 5 s per doorbell, each a separate prompt, with Pi turn-end invisible.
7. **Pi dialogs are unprotected.** Text plus Enter can land in an extension dialog or picker.
8. **No use of Pi's in-process API.** Pi offers `pi.sendMessage(..., {triggerTurn})`, `ctx.isIdle()`,
   `ctx.ui.getEditorText()`, `agent_settled` and `before_agent_start` (primer §2.3-2.4). These would give an
   exact idle check, a draft check, keystroke-free wake, in-turn delivery and real ACKs. Cyclops even names the
   missing bridge (`pi.toml:21-23`) but did not build it.
9. **tmux only.** None of this applies under Herdr.

## Clever ideas worth stealing

- **Script-behind-interpreter binding** (`argv_programs`, `fusion.rs:2887-2927`). Bind `node …/pi` as `pi`, and
  never list `node` itself. Herdr has an agent kind `pi` (our test fixtures use it, primer §7), but the same idea
  is useful wherever we read process trees ourselves.
- **Separating "which rules read this pane" from "who may speak for it"** (`fusion.rs:1680-1690`). A manual pin
  chooses the parser only; identity always comes from live argv.
- **Claim before write withdraws the doorbell** (`DELIVERY.md:148-155`). If the recipient already pulled the
  item through the CLI, no wake bytes are written. Our equivalent is the attention-version check. It is worth
  confirming that our wake path re-checks pending state immediately before typing.
- **Honest receipt vocabulary for unmeasured vendors.** `submitted_unverified` and `notified` with no verifier,
  instead of claiming delivery. If we ever add a pane-typing fallback for Pi, it should record the same
  uncertainty.
- **One-line doorbell with an exact claim token.** It keeps pane bytes small and gives an exact-attempt
  receipt. For Pi specifically, though, an in-process extension makes the claim round trip unnecessary.
  `pi.sendMessage` can carry the framed body directly as a custom message. That is the better design, and it is
  the one our Pi adapter should use (primer §9).
- **Anti-idea to avoid:** pasting blind into a TUI that has no measured composer. For Pi there is no need: the
  extension API gives a draft check (`getEditorText`) and a wake without keystrokes. Typing into Pi should be a
  last-resort fallback at most, gated on Herdr's composer read.

## Verification

Adversarial pass, 2026-10-23, against the cyclops clone at `a84d44b8` (1 commit) and the pi-mono clone at
`a7b94a0`. Nothing executed from either clone.

- Checked and confirmed: `pi.toml` (all cited lines, 55 lines, one `title_useless` rule, the stale "pin" and
  "fail closed" comments); `gate.rs` `admit` (1203-1323, copy-mode, manifest, capture, blocks, composer hold at
  1294, binding predicate 1307-1309, `ForegroundNotAgent`), spool/paste (586-615), Enter and
  `record_submitted_unverified` (676-718), `settle_without_receipt` (810-822), `binding_unprovable`
  (1139-1147), hold ping (1175-1193), `tier2_evidence` (2758-2771); `inject.rs` (78-145, 202-237, 340-343);
  `delivery/mod.rs:64,72`; `fusion.rs` (`composer_is_held` 2386, `bind_manifest_for` 1668-1677, pin-vs-identity
  comment 1680-1690, `admitted_binding_observation` 1907-1941, `argv_programs` 2887-2927); `gate.rs` never reads
  `write_ready`/`safe_states` (grep), which are read only in `messaging_runtime.rs`/`mailbox/service.rs` and
  `cyclops-manifest/src/lib.rs:936-938`; `notification.rs:992-1002`; `DELIVERY.md` 38-80, 96-109, 127-155;
  `STATUS.md:103-105`; `SKILL.md` 3, 209-211, 270-278, 327-359 (24,772 bytes); `install.md` 129-133, 157-163,
  201-216, 486-496, 538, 575-579; `MANIFESTS.md:141-148`; `manifests.rs:201-202`; `skillseed.rs:317-321`;
  `copy.rs:188`; sample doorbell 127 bytes; `gh api`: 14 stars, 0 forks, pushed 2026-09-08, latest release v1.1.0.
  Pi side: `interactive-mode.ts:3321-3338`, `setup.ts:4-5`, `config.ts:579`, `terminal.ts:211-212`,
  `docs/skills.md:43-45,61,85`.
- Changed: the Pi skill-dir citation (`resource-loader.ts:1026` classifies a resource's source, it does not load
  skills) now points at `package-manager.ts:2474,2484`; the claim-identity citation (`DELIVERY.md:185-199` is
  headless registration) now cites `DELIVERY.md:74` for the receipt rule and labels the ancestry text; skill
  description size corrected from 459 to about 445 bytes (459 was the whole frontmatter line); the 120 s hold
  ping now cites the `gate_hold_notify_ms` default (`config.rs:60`); "Herdr reports agent kind `pi`" narrowed to
  what primer §7 supports (our fixtures use that kind).
