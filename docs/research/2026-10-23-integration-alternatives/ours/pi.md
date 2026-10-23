# herdr-threads x Pi coding agent (pi-mono)

- Subject: herdr-threads at `main` `2de3da1c4` (2026-10-23). Paths without a prefix are relative to the
  herdr-threads repo root.
- Harness: Pi, <https://github.com/badlogic/pi-mono> (now <https://github.com/earendil-works/pi>), clone
  `a7b94a03` (coding-agent 1.4.0). Paths prefixed `pi:` are relative to the pi-mono clone root. Harness
  facts not re-read here come from `../primers/pi.md` (cited as "primer §N").
- Nothing was executed. VERIFIED = read in source or docs at the cited line. INFERRED = my reading of the
  code, not tested. "Unknown" = not established.

## Support status

**Not supported (planned).** No adapter, no integration, no tests that exercise a Pi session.

- `builtins()` registers exactly Claude, Codex, Hermes and agy (plus the test-only `synthetic_fourth` under
  `test-support`). VERIFIED `src/harness/registry.rs:1482-1497`.
- No `src/harness/pi*`, no `integrations/pi/` (directory listing: `integrations/` holds `agy claude codex
  hermes skill`). VERIFIED.
- No Pi env var is known to the code: `AI_AGENT`, `PI_CODING_AGENT`, `PI_SESSION_ID` occur nowhere in `src/`,
  `integrations/` or `tests/`; the only hit is the wake-e2e Herdr wrapper stripping `AI_AGENT` from its own
  caller environment. VERIFIED (grep), `scripts/wake-e2e-herdr:39`.
- The string `pi` appears only as a deliberately *unrecognized* Herdr agent kind in tests:
  `src/host/native.rs:5984-5995` (`blocked_and_unrecognized_panes_make_no_composer_read`) and
  `tests/harness/launch.rs:1567-1580` (a bound pane whose kind is `pi` is "not a live bound agent"). VERIFIED.
- Beads: `bd search pi` finds no Pi item (two unrelated hits). VERIFIED. Prior research only listed Pi's
  `session_start` reasons and its repo rename: `docs/research/harness-adapters-2026-10-03/other-harnesses-and-prior-art.md:9,25,34`.
  VERIFIED.

So the rest of this report describes (a) what actually happens today when a Pi agent in a Herdr pane uses
herdr-threads through its generic surface, which is worse than "no support" in one respect (see
"Misattribution"), and (b) what a Pi adapter would need.

### What happens today when a Pi agent runs `herdr-threads`

Traced through the code; the end-to-end outcome is INFERRED (not run), each step VERIFIED at its line.

1. Pi's LLM-called `bash` tool spawns the shell with piped stdio (`pi:packages/coding-agent/src/core/tools/bash.ts:112-116`)
   and an env carrying `AI_AGENT=pi`, `PI_CODING_AGENT=true` (set process-wide at startup,
   `pi:packages/coding-agent/src/cli/setup.ts:5-7`) and `PI_SESSION_ID` (exposed by default,
   `bash.ts:199-201,250`). `HERDR_PANE_ID` is inherited from the pane (INFERRED, ordinary env inheritance).
2. Caller derivation walks the ancestry for a registered `CallerEnvShape`; none matches a Pi process, so it
   returns `EnvRefusal::NoHarness` (`src/harness/caller_env.rs:46-47,341-350`).
3. `NoHarness` with no registered harness marker set is classified as a **person**: `CallerOrigin::Human`
   (`src/cli/caller.rs:23-31,69-81`). The marker check consults only registered adapters' markers
   (`src/cli/output.rs:165-177`; registry `env_markers`, `src/harness/registry.rs:1391-1395`), and none is
   `AI_AGENT`/`PI_CODING_AGENT`. (Exception: a Pi started from a shell that still carries another
   harness's marker, e.g. `CLAUDECODE`, is marked and gets `Refused(NoHarness)` instead; INFERRED.)
4. A non-agent origin locates the seat through the pane's saved context. With no seat mapped to the pane the
   command fails with guidance that ends "(a person in their own pane: `herdr-threads human me init`)"
   (`src/cli/mod.rs:3342-3350`).
5. If the model (or the user) follows that guidance, the A4 person-pane guard does **not** stop it:
   `agent_evidence` looks for a registered person-guard env marker, then for a Herdr pane kind that a
   registered adapter claims (`src/cli/mod.rs:335-372`, `src/protocol/authority.rs:127-131`). Pi's markers
   are unregistered and `pi` is no adapter's `host_kinds`, so there is no evidence and `me init` succeeds
   (INFERRED from those two checks; `selection_from_context` applies the same rule per command,
   `src/cli/mod.rs:3632-3659`).
6. The Pi session is then a person seat: binding provenance `operator_human` (TRUST-POLICY.md:307, "Best
   effort: refused where the system sees evidence of an agent (A4)"), its messages carry the `[human]`
   author marker to recipients (`integrations/claude/mod/hooks/register.js:35-43` `markers()`; same rule in
   `src/delivery/frame.rs`), and person-authored messages count as priority in summaries (AGENTS.md
   "Pre-release wake e2e", `is_priority`). INFERRED consequence.
7. Output form: stdout is a pipe, so `Presentation::Auto` picks the machine/compact form
   (`src/cli/output.rs:211-214`). VERIFIED code; INFERRED for Pi.

Net: a Pi agent can read and send mail manually, but only by impersonating a person seat. That is the main
flaw this report surfaces for Pi.

## Injection mechanism

**None.** herdr-threads puts nothing into a Pi model's context on its own.

- No in-turn delivery: delivery-contract rule 3 (hooks/channels deliver every pending item) is implemented
  only by the Claude mod channel (`integrations/claude/mod/hooks/register.js`, `tool.call` at :1092 and the
  idle append at :560-575), the hook-pull runner in `src/cli/hook.rs` for Codex/Hermes, and agy's
  `PreInvocation` step (`src/harness/agy/delivery.rs`). All are keyed on a registered adapter. VERIFIED.
- The push channel (`herdr-threads watch`) refuses any harness that does not declare a push delivery policy:
  `--harness` is a `PossibleValuesParser` over `push_channel_ids()` (`src/cli/commands.rs:1208-1222`),
  which today is `["claude"]` (asserted in `src/harness/registry.rs:2651`). VERIFIED. So even a hand-written
  Pi extension cannot open a channel under its own id.
- The only path is pull: the model runs `herdr-threads inbox` in `bash` (as a person seat, see above).
  - Payload: compact rows, one per item, `next:` continuation lines (`src/protocol/output_compact.rs`).
    VERIFIED (AGENTS.md "Token-efficient communication").
  - Timing: whenever the model decides to call it. Nothing tells it to; the skill
    (`integrations/skill/SKILL.md`) is installed only into registered harnesses' config by `setup`, and no
    code writes it to a Pi-discovered location such as `~/.agents/skills/` (grep for `.agents` in `src`,
    `integrations` finds nothing). VERIFIED.
  - Bounds: default inbox page bounds (32 items / 64 KiB, `src/service/delivery_pull.rs:37,40`), long bodies
    truncated at 8 KiB with a `body <id>` continuation (`src/protocol/watch.rs:63`, AGENTS.md). VERIFIED.
  - Framing: none beyond inbox rows; Pi wraps the tool result in its own bash-tool output (truncation per
    Pi's tool rules, primer §2.3). INFERRED.
  - ACK: inbox ACKs on display (`integrations/skill/SKILL.md` "What an ACK means"), recorded as the person
    seat's ACK. Dedup: the read cursor. VERIFIED (doc), INFERRED for Pi.

## Wake mechanism

**None. An idle Pi pane is never woken, and never typed into.**

- Herdr's pane agent kind selects the composer parser through `by_host_kind`; for `pi` there is none, so the
  observation is `HostUiState::Unknown` with no composer read (`src/host/native.rs:2032-2047`, test
  :5984-5995). VERIFIED.
- Every wake path requires `Idle`: the hint gate `may_hint` (`src/notification/policy.rs:112-133`, the `ui ==
  Idle` term at :122) and the native-authority wake (`src/notification/dispatch.rs:238-240`). An `Unknown` pane
  fails both. VERIFIED. The cooperative path additionally needs a binding whose harness has a registered
  adapter and recognized execution (`dispatch.rs:228-236`, `policy.rs` `execution_recognized`). INFERRED that
  a person-seat binding in a Pi pane can never reach a composer write.
- Draft safety: trivially safe (nothing is typed). Latency: unbounded (next manual `inbox`).
- Failure mode worth noting: because no wake exists, a p0 item (ACK request, invitation, `--nudge`) to a
  seat in a Pi pane silently waits; delivery-contract rule 5 ("always wake") does not hold for Pi. The sender
  gets an overdue warning only if the item has a deadline (INFERRED from the generic deadline path; not
  traced for this case).

For reference, what our supported harnesses do (VERIFIED): Claude's mod appends the framed batch via
`$.session.append`, then submits the content-free `wake up` (`register.js:12,560-629`), with a 120 s draft hold
(`register.js:16,542-544`); Codex/agy get Herdr-typed `herdr-threads: wake up` into an empty composer
(`src/notification/policy.rs:14-21`, `src/harness/agy/delivery.rs:68-82`), with `MARKER` (names `inbox`) as
the default for any adapter without a turn-start injector (`src/harness/adapter.rs:466-471`).

## Setup footprint

Today: none. `herdr-threads setup` has no Pi target (setup dispatches per registered adapter; `SetupFailure`
`UnsupportedHarness` for unknown ids, `src/harness/setup.rs:3597`). Nothing is written to `~/.pi`.
VERIFIED.

## Token cost per message and per quiet turn

- Quiet turn: **0 bytes** (no injection exists). VERIFIED.
- Per message: only what the model pays to run `herdr-threads inbox` itself: one tool call (the command
  text plus Pi's bash tool framing) plus one compact row per item and its indented body. INFERRED; no Pi-specific
  measurement. A missed message costs nothing until the model looks, which is also the problem.
- For comparison, a channel delivery frame is `[herdr-threads] new messages:` (29 bytes) plus one header line
  per item (`[herdr-threads] message <id> in <thread> from <sender>[ markers]:`) and the body indented two
  spaces (`register.js:58-67`, `src/delivery/frame.rs:9`). VERIFIED. A Pi extension reusing it would cost the
  same, with the content arriving as a Pi `custom` message that reaches the model as a `user` role message
  (primer §2.3, `pi:packages/coding-agent/src/core/messages.ts:162-169`).

## Known limitations

1. **Misattribution as a person** (flaw, INFERRED end to end, each step VERIFIED above): Pi's own agent markers
   (`AI_AGENT=pi`, `PI_CODING_AGENT=true`) are not registered, so `CallerOrigin::Human` and A4 both miss it, and
   the no-seat error text actively suggests `human me init`. TRUST-POLICY A4 is documented as best effort, so
   this is within the stated limit, but cheap to close: an adapter that registers `AI_AGENT` (or
   `PI_CODING_AGENT`) as a `person_guard` `AgentEnvMarker` and `pi` as a `host_kind` would make Pi panes
   refuse `me init` and turn a Pi caller without an adapter shape into `Refused(NoHarness)` instead of `Human`
   (`src/cli/caller.rs:76-79`). Caveat: `AI_AGENT` is a generic name; registering it under the Pi adapter
   would also flag any other harness that sets it (the wake-e2e wrapper already treats it as a caller marker,
   `scripts/wake-e2e-herdr:39`), which is the desired A4 direction but makes the marker not Pi-specific. A
   registry rule forbids duplicate markers (`src/harness/registry.rs:1340-1347`), so only one adapter can own it.
2. No in-turn delivery, no wake, no ACK-by-delivery (rule 3 and rule 5 unmet for Pi). VERIFIED by absence.
3. Herdr pane detection for Pi is assumed only from our own fixtures (`"pi"` kind); whether Herdr really
   reports `pi` is unknown (primer §7).
4. `push_channel_ids()` and the TRUST-POLICY wording of `cooperative_mod_channel`/`cooperative_mod_delivery`
   ("today Claude", `$.session.append` predicates, TRUST-POLICY.md:303-304) are written for the Claude mod; a
   Pi channel needs its own predicate text in the same commit (AGENTS.md "Trust policy": a new way to
   attribute needs a defined provenance). VERIFIED text.
5. No beads track Pi. VERIFIED.

## For a harness we do not support yet: what our adapter traits would need, and which primer surfaces fit best

### Recommended shape: push channel, harness self-wake (Claude-mod-like)

Pi's in-process extension API is a closer fit for our `PushChannel` + `IdleWake::Channel` pairing than Claude
Code's mod engine is, because one call both delivers and wakes. The registry already allows
`(PushChannel, cooperative_mod_delivery, Channel|Native)` and `(HookPull, cooperative_hook_delivery, Native)`
only (`src/harness/registry.rs:1270-1282`). VERIFIED.

| Contract need | Pi surface (primer) | herdr-threads trait / piece |
|---|---|---|
| Identity of a CLI caller | `PI_SESSION_ID` in LLM `bash` env (§5; `pi:.../tools/bash.ts:199-201`); process is Node with `process.title = "pi"` (`pi:.../cli/setup.ts:5`) | `caller_env()` -> `CallerEnvShape { process_names: ["pi"], session_var: "PI_SESSION_ID", process_match: argv contains `@earendil-works/pi-coding-agent` (INFERRED, like Claude's `@anthropic-ai/claude-code` match, `src/harness/claude.rs:942-955`), pid_var: none (Pi exports no pid var, unknown) }` |
| Agent markers, A4 | `AI_AGENT=pi`, `PI_CODING_AGENT=true` (§5) | `AdapterMetadata.agent_env_markers` with `person_guard: true`; `host_kinds: ["pi"]` (`src/harness/adapter.rs:474-486,352-358`) |
| Channel transport | extension `session_start` spawns a child, `session_shutdown` stops it (§2.1 rule) | `watch --harness pi --session $id` once `delivery_policy()` returns `PushChannel` (`commands.rs:1208-1222` then admits `pi`) |
| Idle delivery + wake | `pi.sendMessage({customType:"herdr-threads", content: frame, display:true}, {triggerTurn:true})` when `ctx.isIdle()` (§2.3, `pi:.../agent-session.ts:2357-2393`) | `DeliveryPolicy { PushChannel, COOPERATIVE_MOD_DELIVERY_PROVENANCE, IdleWake::Channel }`; no wake prompt at all, which satisfies R18 more strictly than Claude's append + `wake up` |
| Silent (p > T) items | `sendMessage` without `triggerTurn` (idle: appended to history; busy: deferred to end of turn, `agent-session.ts:2384-2391`) | the mod's `passive()` split (`register.js:630-632`) |
| In-turn delivery (rule 3) | busy `sendMessage` default = steer, polled after each tool batch (§2.3); or `tool_result` content append | the mod's `tool.call` path; prefer steer, one framed batch per call because the default `one-at-a-time` queue spreads N messages over N turns (§2.3) |
| Never nudge busy (rule 4) | `ctx.isIdle()`, `agent_start`/`agent_settled`, `ui_prompt_start/end` (§2.4) | mod `busy()` / `idleOpen()` gates (`register.js:559`) |
| Draft safety | `ctx.ui.getEditorText()` (TUI only; `""` in RPC) (§2.4) | the mod's `DRAFT_WAIT_MS` hold (`register.js:16,542-544`) |
| Delivery ACK | `sendMessage` resolves `Promise<void>` with no deny/drop result (`agent-session.ts:2357-2360`) | a new predicate in TRUST-POLICY for `cooperative_mod_delivery` ("the extension's `sendMessage` resolved without throwing"); weaker than Claude's `deny` signal, so state it honestly |
| Managed launch | `pi -e <ext>` loads an extension for one process without config writes (§2.1); `--session-id` pins the session id up front (§4) | `launch_policy()`; `--session-id` would let launch pre-bind the seat deterministically |
| Setup | `~/.pi/agent/extensions/` or a Pi package; `PI_CODING_AGENT_DIR` for isolated tests (§2.1) | `setup`/`status`/`unsetup`, `default_config_root` honouring `PI_CODING_AGENT_DIR` |
| Skill | Pi discovers `~/.agents/skills/` and `.pi` skills (agmsg report, `pi:.../docs/skills.md:61`) | install `integrations/skill/SKILL.md` there, or ship it inside the Pi package |

Required trait surface (VERIFIED as the trait's required methods, `src/harness/adapter.rs:15-278`): `metadata`,
`contracts`, `observe_install`, `admit`, `version_ladder`, `classify`, `decode`, `encode`,
`attribute_runtime`, `setup`, `status`, `unsetup`; plus the optional `caller_env`, `delivery_policy`,
`launch_policy`, `text`, `default_config_root`. `composer_policy` (`adapter.rs:1352-1359`) would be needed only
for a Herdr-typed fallback (below). Pi has no hook subprocess, so `required_hook_events` stays empty and the
hook-shaped `classify/decode/encode` would carry lifecycle events the extension reports through the CLI
(INFERRED; how much of the hook contract machinery a channel-only adapter must implement is unknown without
trying, the Claude adapter implements both).

Neutral-core changes (INFERRED): none needed for the trait shape, the registry is open (`synthetic_fourth`
proves a fifth registration, `registry.rs:1491-1497`). Needed outside the adapter: TRUST-POLICY text for the
Pi delivery predicate; `push_channel_ids` picks Pi up automatically.

### Fallback shape: Herdr-typed native wake

`(HookPull, cooperative_hook_delivery, Native)` with a `ComposerPolicy` reading Pi's editor from the pane.
Worse fit: Pi has no out-of-process hook model, so "hook pull" would still be an extension spawning the CLI,
and typed text into a Pi that turned busy between the idle read and the keystrokes becomes a **steer** of the
running task (`pi:.../interactive-mode.ts:3333-3338`, primer §6), a softer failure than a lost wake but a rule-4
violation in spirit. Fullscreen alt-screen rendering and the `[paste #N]` collapse (primer §6) also make
composer scraping fragile. Use only if the extension route is blocked.

### Surfaces that do not fit

- MCP: pull-only, no sampling, no resource-update handling; cannot push or wake (primer §3).
- RPC/SDK: the process owner's channel; cannot attach to an interactive Pi in a pane (primer §4).
- Headless re-launch (`pi -p --session`): no session lock, unsafe while the TUI owns the session (primer §4).

### Open risks for an adapter

- Custom messages reach the model as `user` role (primer §2.3): our frame's trust stance (peer text never
  overrides the user) relies on the skill and the `[herdr-threads]` header only. INFERRED.
- `sendMessage(triggerTurn)` during compaction: `prompt()` throws then (primer §2.3); whether `_startRun` from a
  custom message also does is unknown. The extension must retry on `session_compact` end.
- `/new`, `/resume`, `/fork`, reload replace the runtime (primer §2.1): the extension must close and reopen its
  `watch` child per session id, the analogue of the mod's session-generation handling (`register.js` `S.gen`).
