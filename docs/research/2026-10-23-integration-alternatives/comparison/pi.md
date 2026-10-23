# Comparison: message injection and wake on Pi (pi-mono)

- Subject: herdr-threads `main` `2de3da1c4`. Pi clone `a7b94a03` (coding-agent 1.4.0), cited `pi:<path>:<line>`
  relative to the pi-mono clone root (<https://github.com/badlogic/pi-mono>).
- Inputs: `../ours/pi.md`, `../primers/pi.md`, and `../alternatives/<slug>/pi.md` for the 12 alternatives. Each
  alternative claim below is taken from its report, which cites the clone line; the report is named in brackets,
  for example [hcom]. Where this comparison depends on a claim, I re-read it in herdr-threads or pi-mono
  and say so.
- Tags: **V** = VERIFIED (read in source or docs at the cited line, here or in the cited report's verification
  pass), **I** = INFERRED, "unknown" = not established. Nothing was executed.

## 0. New facts established in this pass

These were read for this comparison and change conclusions in the per-tool reports:

1. **Pi's extension `sendMessage` is void and fire-and-forget.** The bound action calls
   `this.sendCustomMessage(...).catch(err => runner.emitError(...))` and returns nothing
   (`pi:packages/coding-agent/src/core/agent-session.ts:3446-3456`) **V**. So an extension can never observe
   "sendMessage resolved without throwing". The predicate `../ours/pi.md` proposes for a Pi
   `cooperative_mod_delivery` ("the extension's `sendMessage` resolved without throwing") **cannot be
   observed**. An honest Pi delivery predicate has to come from a later event that carries our id (see 4.3).
2. **A steered *custom* message is dropped on abort and is not restored to the editor.** `sendCustomMessage`
   with `isStreaming` puts the message on `this.agent.steer(appMessage)`, the agent's steering queue
   (`agent-session.ts:2370-2377`; `pi:packages/agent/src/agent.ts:308-310`) **V**. On Escape, interactive mode
   calls `restoreQueuedMessagesToEditor`, then `clearAllQueues`, then `session.clearQueue()`, which copies only
   the *text* lists `_steeringMessages`/`_followUpMessages` back to the editor and calls
   `agent.clearAllQueues()` (`pi:packages/coding-agent/src/modes/interactive/interactive-mode.ts:4700-4752`;
   `agent-session.ts:2466-2475`; `agent.ts:318-331`) **V**. `_steeringMessages` is filled only by
   `_queueSteer(text)` for typed or `sendUserMessage` input (`agent-session.ts:2302-2303`) **V**. Consequence
   (**I** from those paths): a `sendMessage` steer (Cotal, agent-talk, and our planned design) disappears
   silently when the human aborts. A `sendUserMessage` steer (hcom) is pasted back into the editor and joined
   with the draft. This settles the "unknown" in [cotal] and confirms the duplicate in [hcom].
3. **`ctx.isIdle()` already excludes compaction** (`!this._isAgentRunActive && !this.isCompacting`,
   `agent-session.ts:1484-1486`) **V**. `sendCustomMessage` branches on `isStreaming` only
   (`agent-session.ts:1479-1481,2372`) **V**, so a `triggerTurn` send during an idle compaction takes the
   `_startRun` path, and what happens then is unknown. Gating the wake on `ctx.isIdle()` avoids that case.
   None of the four tools that wake through `sendMessage` gates on `isIdle()` before sending (hcom uses it
   only to choose between steer and run, [hcom]).
4. **Our default text inbox fits under Pi's bash-tool truncation.** Pi's bash tool keeps the last 2000 lines or
   50 KB (`pi:packages/coding-agent/src/core/tools/truncate.ts:11-12`, `truncateTail` in
   `output-accumulator.ts:92`) **V**. Our generic page default is `DEFAULT_PAGE_BYTES = 16_384`
   (`src/protocol/pagination.rs:6`, used by `page()` in `src/cli/commands.rs:2233-2237`) **V**, and bodies are
   cut at 8 KiB. So agmsg's "mark read, then Pi drops the head" bug [agmsg] does not hit our default. It
   could hit us with an explicit `--max-bytes` above about 50 000, up to the 65 536 cap
   (`src/protocol/pagination.rs:7`) (**I**).

## 1. Mechanism table

"Native" means the tool ships Pi-specific code. Rule numbers are the AGENTS.md delivery contract.

| Tool | Pi support | Injection (what puts text in context, when) | Wake (idle Pi) | Busy handling | Draft safety | Receipt / ACK | Bounds |
|---|---|---|---|---|---|---|---|
| **herdr-threads (ours)** | None. `builtins()` = claude, codex, hermes, agy (`src/harness/registry.rs:1482-1490`) **V** | Pull only: the model runs `herdr-threads inbox` through Pi's `bash` tool, and only as a misattributed person seat (`src/cli/caller.rs:76-79` returns `Human` on `NoHarness` without a registered marker) **V** | None. A `pi` pane has no parser, so `HostUiState::Unknown` (`src/host/native.rs:2039-2047`) and `may_hint` needs `Idle` (`src/notification/policy.rs:122`) **V** | n/a | Trivially safe | Display ACK on a text inbox (`cooperative_inbox_display`, TRUST-POLICY.md:302) **V** | 16 KiB page, 8 KiB body **V** |
| hcom [hcom] | Native, first-class (`hcom pi`) | In-process extension. `sendUserMessage(batch)` as a real user message: a steer from `tool_result`/`turn_end` while busy, a new run on `agent_settled`/notify-port while idle. Bootstrap through `before_agent_start` | Same call: wake = delivery, no marker | Steer after the tool batch | None. Editor never read; a wake starts a run under a typing human | Admission: `message_end` user text `includes(sent)`, then a cursor ack; 10 s grace release; at-least-once | 50 msgs, 1 MiB each, no batch byte cap |
| Cotal [cotal] | Native, alpha (pins Pi 0.79.10 in docs) | In-process. `sendMessage({customType, content, details:{batchId, ids}}, {triggerTurn:true, deliverAs:"steer"})`, one batch in flight; forced pump at every `agent_start` | Same call | Steer | None. Editor never read | Three proofs: `message_start` (dequeued), `context` (in a request), `after_provider_response` 2xx; commit at a terminal `agent_end`; ids ride in `details` (not sent to the model) | 32 items, no byte cap; 203-char instruction tail on every batch |
| ocs [open-cross-session-ocs] | Native, TUI only | In-process. Per-session 0600 UDS plus a token; `sendMessage(note, {deliverAs:"followUp", triggerTurn:true})` | Same call | followUp: lands only at the run's natural stop | None | `queued` reply sent right after the void call; the sender never retries an unknown outcome | 4 KiB inline body, 16 KiB frame |
| agent-talk [agent-talk] | Native (Pi package) | In-process. `fs.watch` on a spool plus a 1 s poll; `sendMessage(..., {triggerTurn:true})` | Same call | Steer | None | None. In-memory offset and id set; the relay already acked | None. The documented spool is likely never written for Pi |
| agmsg [agmsg] | None. Pi picks up the shared skill from `~/.agents/skills` and registers as `codex` | Pull via `bash` (`inbox.sh`) | Only a manual `poke` (Herdr `agent prompt`); fails closed on Codex's `›` marker | Typed text becomes a steer | Fail-closed marker check, or none for markerless types | Mark read on display | Unbounded, so it loses messages under Pi's 50 KB tail cut |
| CAO [cli-agent-orchestrator-cao-awslabs] | None (only Oh My Pi) | tmux paste plus Enter as a user turn, between turns only | Same keystrokes | Not delivered (waits for IDLE); a stale verdict becomes a steer | None | DELIVERED before the paste | None |
| Cyclops [cyclops] | Generic tmux manifest, no rules | One-line doorbell paste plus Enter; the model then `claim`s the body via `bash` | Same keystrokes, blind | Doorbell becomes a steer | None (no `human_input` rule) | `submitted_unverified`, then `notified`; the claim is the receipt | 1 line plus the claim payload |
| agentchattr [agentchattr] | None (hand config) | tmux paste of a pointer, then MCP `chat_read` pull | Same keystrokes, blind | Steer | None | Cursor advance on read | 20 msgs; Pi's 20 KB MCP middle cut |
| xats [cross-agent-teams-mcp-jtianling] | None (`custom`) | tmux paste of a 200-char hint, then MCP `get_inbox` | Keystrokes after a 2 s screen-quiet guard; tmux only | Guard usually fails; otherwise a steer | None (motion guard only) | Cursor advance on read | 50-200 msgs; Pi's 20 KB MCP cut |
| agent-peers [agent-peers-mcp-mikez93] | None (as `codex`) | Piggyback block on any agent-peers MCP tool result | None | n/a | n/a | Confirm on the next call, roll back on cancel | None; Pi's 20 KB cut |
| mcp_agent_mail [mcp-agent-mail] | None (Rust targets OMP's `.omp/` only) | MCP pull (`fetch_inbox`), hidden by Pi's default `codemode` exposure | None. The signal file is never consumed | n/a | n/a | Separate `acknowledge_message` | `limit`, no byte cap |
| ruflo [ruflo] | None. The `pi-dev` scaffolder emits an extension Pi would refuse to load | MCP/CLI pull (`team_inbox`) | None | n/a | n/a | Drain by rename | None on read |

Summary: four tools (hcom, Cotal, ocs, agent-talk) converge on one shape: an in-process extension calls
`pi.sendMessage`/`sendUserMessage` with a turn trigger, and the wake *is* the delivery. Every keystroke and MCP
design is strictly worse on Pi. Every tool ships the trigger without the guards Pi offers: no `isIdle()` gate
before the send, no `getEditorText()`, no `ui_prompt_*`, and no silent `triggerTurn:false`/`nextTurn` branch.

## 2. Differences from our approach, with pro/con

Dimensions: latency (L), reliability (R), token cost (T), draft safety (D), setup footprint (S), harness-version
fragility (F), cross-machine (X), trust (Tr).

### 2.1 They ship a Pi adapter at all; we ship none

hcom, Cotal, ocs and agent-talk are native. We have no in-turn delivery, no wake, and no rule-5 guarantee for a
Pi seat (`../ours/pi.md` "Injection", "Wake") **V**.
- Pro for them: L sub-second (hcom, ocs: a local socket and an in-process call, **I**). R: Pi agents
  participate at all.
- Con for them: F. Each one tracks Pi's extension API. Cotal *documents* an exact pin, 0.79.10, but only
  feature-probes at runtime [cotal]. ruflo's scaffolder targets an API that does not exist [ruflo]. That is the
  failure mode of an unverified API.
- For us: the con stays small if we follow our versionless rule: feature-probe `pi.sendMessage`/`pi.on` and
  degrade only the missing capability (AGENTS.md "Harness and Herdr versions").

### 2.2 Wake = delivery (one call) vs our content-free wake prompt

On Claude we `$.session.append` the frame, then submit `wake up` (`integrations/claude/mod/hooks/register.js:12`)
**V**. On Codex and agy, Herdr types `herdr-threads: wake up` (`src/notification/policy.rs:14-21`) **V**. On Pi
every native tool starts the run with the content itself, so there is no marker.
- Pro: T. No marker user message (our Claude path adds `wake up`, about 2 tokens, plus a user turn record). R:
  no window between "content appended" and "turn started" (Cotal's stated reason, [cotal]). It satisfies R18
  more strictly than Claude does today.
- Con: none for Pi. It is not portable to harnesses without an in-process API.
- Adopt for Pi: yes.

### 2.3 Custom message (`sendMessage`) vs user message (`sendUserMessage`)

hcom uses `sendUserMessage` [hcom]. Cotal, ocs and agent-talk use `sendMessage` with a `customType`.
- Both reach the model as `user` role (`pi:packages/coding-agent/src/core/messages.ts:162-169`) **V**, so Tr is
  equal at the model.
- Custom messages: `details` carries ids at zero token cost (V, primer §2.3); they render as a typed block in the
  TUI; and they bypass Pi's `input` chain [cotal]. On abort they are **silently dropped** (fact 0.2).
- User messages: they show as a user turn; on abort they are pasted into the editor, joined with the draft, and
  can come back as a duplicate [hcom].
- For us: `sendMessage`, with the ACK withheld until a delivery proof (2.5), so a dropped steer is re-offered
  by our existing ledger (`REOFFER_AFTER_MS`, `MAX_OFFERS_PER_GENERATION`, AGENTS.md) **I**.

### 2.4 Busy handling: steer vs follow-up vs nothing

Cotal, hcom and agent-talk steer: the message lands after the current tool batch
(`pi:packages/agent/src/agent-loop.ts:304`, primer §2.3) **V**. ocs uses followUp: it lands only at the run's
natural stop [open-cross-session-ocs]. Keystroke tools steer by accident (typed Enter while busy becomes a
steer, `interactive-mode.ts:3333-3338`, primer §6) **V**.
- Steer. Pro: rule 3 (in-turn) with the right latency. Con: it can redirect a long task. With the default
  `steeringMode: "one-at-a-time"` (`pi:packages/coding-agent/src/core/settings-defaults.ts:7`) **V**, N separate
  sends cost N requests.
- FollowUp. Pro: it never redirects. Con: L is unbounded on long runs, and rule 3 is not met mid-turn.
- Ours (planned): steer one framed page per call, which is our page assembler. Rule 4 holds, because a steer is
  in-turn delivery, not a nudge.

### 2.5 Delivery proof

Ours on Claude: `$.session.append` resolved without `deny` (TRUST-POLICY.md `cooperative_mod_delivery`) **V**.
Ours on Codex: the frame header found in the transcript (`cooperative_hook_delivery`) **V**. Theirs on Pi:
- Cotal: the `message_start` + `context` + 2xx chain, matched on `details.batchId`.
- hcom: `message_end` substring.
- ocs, agent-talk: nothing, or "the call returned".

| | R | Tr (honest claim) | Cost |
|---|---|---|---|
| Cotal chain | best: proves the batch entered a provider request | strongest | three handlers, in-process |
| hcom substring | good, false-positive on quoted text | heuristic | one handler |
| "sendMessage returned" | lies on async failure | dishonest (fact 0.1) | none |

Adopt Cotal's shape, keyed on `details` ids, with `context` (or `message_start`) as our predicate. This is the
Pi analogue of our Codex transcript check. The Pi events exist (`pi:packages/coding-agent/src/core/extensions/types.ts:886,918,1076`) **V**.

### 2.6 Priority / threshold vs "every message wakes"

Every native tool wakes on every message. hcom never uses `triggerTurn:false`/`nextTurn` [hcom]. Cotal wakes on
all channel chatter at the default `open` attention [cotal].
- Ours: rules 1-2 (p1 default, T = 0). Pi has all four branches we need (`agent-session.ts:2370-2391`) **V**.
- Pro (ours): T and model attention, because idle sessions stay idle. Con: none, given Pi's API.

### 2.7 Idle/busy detection

Ours: Herdr's pane status plus a per-harness composer parser (`src/host/native.rs:2032-2047`) **V**. Theirs:
in-process `isStreaming` (implicitly), `agent_settled` (hcom), and a private state machine (Cotal).
- In-process. Pro: R exact, with no screen scrape and no `Unknown`. Con: it exists only inside the extension;
  the daemon sees it only through the channel.
- Ours for Pi: none, so no wake. The in-process signal is the fix; a Herdr parser for Pi is not.

### 2.8 Draft safety

None of the 12 reads `ctx.ui.getEditorText()` (all reports) **V**. In-process tools never clobber, because
`sendMessage` does not touch the editor. They still start a run under a typing human, whose Enter then becomes a
steer (primer §6) **I**. Keystroke tools splice the wake into the draft (Cyclops, CAO, agentchattr, xats) **I**.
Ours on Claude: a 120 s draft hold read from `io.promptRead()` (`register.js:16,538-545`) **V**.
- For Pi, ours is better if ported. `getEditorText()` (`types.ts:237`) and `onTerminalInput` (`types.ts:163`)
  **V** give an exact read with no scrape.

### 2.9 Abort handling

Cotal holds after an abort until the human's next turn [cotal]. hcom redelivers and restarts the aborted run
[hcom]. Our Claude mod already holds: `abortHoldSince` on `isAborted`/`aborted`/user reject (`register.js:818-820`),
and the idle path returns `held('post_abort')` (`register.js:541`) **V**. So the [cotal] suggestion "probably for
the Claude mod as well" is already done there.
- For Pi: key the hold on `agent_settled.aborted` (`types.ts:1034-1036`) **V**, and combine it with fact 0.2,
  which means an un-proven steer must be re-offered, not counted.

### 2.10 Attach and setup footprint

- hcom: `pi -e <content-addressed path>` per launch, no `~/.pi` writes, removes only its own legacy copies.
- Cotal: `pi --extension` for managed seats; a manual `cp` for interactive Pi.
- ocs: a global `~/.pi/agent/extensions/ocs.ts`, so every Pi opens a socket.
- agent-talk: `pi install`, which writes settings.json.

S: `-e` is zero-footprint and fits our managed launch. An unmanaged Pi needs a global extension (ocs) or a
package (agent-talk). Make the global one inert without an env or binding, as hcom and Cotal do.

### 2.11 Transport from the daemon into the extension

- hcom: a TCP notify port, then a `hcom pi-read` spawn per event, including per tool call and per turn
  (5 s timeouts on the critical path) [hcom].
- ocs: a per-session UDS with a 256-bit token [open-cross-session-ocs].
- Cotal: an in-process NATS client.
- agent-talk: a file spool.

Our Claude mod runs a long-lived `herdr-threads watch` child (`src/cli/commands.rs:1208-1222`) **V**.
- R, L: a persistent stream (ours, Cotal) beats spawn-per-event (hcom). hcom also wakes every plugin on every
  send (`wake_all`) [hcom].
- X: Cotal (NATS) and agent-talk (relay) cross machines, and ocs has a LAN daemon. Ours is local by design
  (our p2p work is separate). This is not a Pi-specific gap.

### 2.12 Identity and session continuity

- Cotal launches with `--session-id <uuid>`, checks `PI_SESSION_ID` against `COTAL_PI_EXPECTED_SESSION`, and
  keeps a process-global `Symbol.for` runtime map across `/new`/`/resume`/`/fork`/reload [cotal].
- hcom fences stale processes with `HCOM_PROCESS_ID` and a generation counter [hcom].
- ocs is keyed on the Pi session uuid, so `/new` breaks reachability [open-cross-session-ocs].
- Ours: a Pi caller has no adapter shape, so it is classified `Human` (`src/cli/caller.rs:76-79`) **V**.
- Tr: Cotal's exact pre-binding matches TRUST-POLICY's "never on heuristic evidence". Ours today is worse than
  none: a misattributed person seat (`../ours/pi.md` "Misattribution").

### 2.13 Framing and token cost

| Tool | Per-delivery framing | Static per request |
|---|---|---|
| Ours (Claude frame) | `[herdr-threads] new messages:` (29 B) + one header line per item (`register.js:58-67`) **V** | skill description |
| hcom | `<hcom>[intent #id] a -> b: ...</hcom>`, unescaped | bootstrap of about 700 tokens per session, repeated after `/reload` |
| Cotal | header + 203-char instruction tail on every batch | about 24 tools, ~3-5k tokens |
| ocs | about 250-300 B skeleton with reply commands | 12.7 KB skill |
| agent-peers | about 480 B header per block | n/a |

- Ours is the leanest per delivery (R17). Nobody else bounds bodies at 8 KiB with a continuation except ocs
  (4 KiB inline + `ocs read`).
- Cotal and ocs neutralize peer text against line forgery: indented bodies, `> ` prefixes for marker lines.
  Ours indents bodies two spaces (`register.js:58-67`) **V**. Whether we neutralize a body line that starts with
  `[herdr-threads]` after the indent is unknown here; the indent alone already keeps it off column zero (**I**).

## 3. Flaws in our approach that the others expose

1. **No Pi path at all, while four competitors have one** (high). Evidence: 1-table rows; `src/harness/registry.rs:1482-1490` **V**.
2. **Pi agents are silently classified as persons** (high). `CallerOrigin::Human` on `NoHarness` without a
   registered marker (`src/cli/caller.rs:76-79`) **V**; the no-seat error suggests `human me init`
   (`src/cli/mod.rs:3342-3350`) **V**. agmsg's Codex misregistration on Pi [agmsg] is the same failure class.
   Pi sets `AI_AGENT=pi` and `PI_CODING_AGENT=true` (`pi:packages/coding-agent/src/cli/setup.ts:6-7`) **V**, so
   a fix costs almost nothing.
3. **The push channel is hard-gated to registered push adapters** (medium). `watch --harness` accepts only
   `push_channel_ids()` (`src/cli/commands.rs:1216`), which today is `["claude"]` (`src/harness/registry.rs:2651`)
   **V**. A community Pi extension cannot reuse our channel the way third parties reuse agent-talk's or ocs's
   transport. This is deliberate (TRUST-POLICY provenance), but it means Pi waits for us.
4. **Our planned Pi delivery predicate was unobservable** (medium, design). `../ours/pi.md` proposed "the
   extension's `sendMessage` resolved without throwing". It cannot be observed (fact 0.1) **V**. Cotal's proof
   chain is the honest replacement.
5. **A steer-based Pi design loses items on abort unless the ACK waits** (medium, design). Fact 0.2 **V**/**I**.
   hcom's and Cotal's bugs show both directions: a duplicate when the message is restored, a stuck hold when
   it is dropped.
6. **Rule 5 ("p0 always wakes") fails silently for unrecognized panes** (medium). A Pi pane yields
   `HostUiState::Unknown`, so `may_hint` is false (`src/notification/policy.rs:112-133`) **V**. ocs tells the
   sender at send time (`unavailable`/`failed`/`unknown-outcome`, exit 2/3) [open-cross-session-ocs], and Cotal
   publishes a presence reason. Our sender learns only through a deadline's overdue warning (**I**, not traced
   for a no-deadline item).
7. **The idle wake depends on a per-kind Herdr composer parser** (low for Pi, general). Every non-Claude idle
   wake goes through `by_host_kind` parsers (`src/host/native.rs:2039-2047`) **V**. In-process harnesses (Pi,
   and OpenCode by analogy) make that parser unnecessary. Our neutral core has no notion of "the harness reports
   its own idle", except through the Claude mod channel.
8. **The skill does not reach Pi** (low). `setup` installs it only for registered harnesses (`../ours/pi.md`)
   **V**. agmsg and Cyclops reach Pi for free through `~/.agents/skills` / `~/.pi/agent/skills`
   (`pi:packages/coding-agent/docs/skills.md:61`) **V**.

Not flaws (checked): body and page bounds survive Pi's 50 KB bash truncation by default (fact 0.4), and our
per-delivery framing is the smallest of the field (2.13).

## 4. Better integration ways, ranked by value/cost

| # | Change | Value | Cost | Evidence |
|---|---|---|---|---|
| 1 | Register Pi agent markers now, before any adapter: `PI_CODING_AGENT` (Pi-specific; `AI_AGENT` is generic and only one adapter may own a marker, `src/harness/registry.rs:1340-1347`) as a `person_guard` `AgentEnvMarker`, plus `host_kinds: ["pi"]`. A Pi caller then becomes `Refused(NoHarness)` and A4 refuses `me init` | high (closes misattribution) | very low | `src/cli/caller.rs:76-79`, `src/cli/mod.rs:335-372`, `src/harness/adapter.rs:474-486` **V** |
| 2 | Pi adapter `(PushChannel, cooperative_mod_delivery, Channel)`: an in-process extension that holds one `herdr-threads watch --harness pi --session $PI_SESSION_ID` child per session (`session_start` / `session_shutdown`). Per item: if `p <= T`, `ctx.isIdle()`, empty `getEditorText()`, no `ui_prompt` open and no post-abort hold, then `sendMessage({customType:"herdr-threads", content: page, details:{ids}}, {triggerTurn:true})`. If idle and `p > T`: `{triggerTurn:false}` (silent append). If busy: a default steer, one framed page per call | very high (rules 1-6 for Pi; L sub-second; 0 marker tokens) | medium (adapter + about 300 lines of TS + TRUST-POLICY text) | `agent-session.ts:2357-2393` **V**; convergent design in [hcom], [cotal], [open-cross-session-ocs] |
| 3 | Pi delivery predicate = our id in `details` observed in a `context` event (the batch is in a provider request), or at least `message_start`. Until then, no ACK; on `agent_settled{aborted}` or a missing proof, re-offer through the existing ledger | high (honest TRUST-POLICY claim; survives fact 0.2) | low inside #2 | `types.ts:886,1076,1034-1036` **V**; [cotal] |
| 4 | Managed launch: `pi -e <content-addressed ext>` + `--session-id <uuid>` chosen by `launch`, with the extension checking `PI_SESSION_ID` against the expected id (exact pre-binding, no heuristics) | high (Tr, S = 0 config writes) | low | primer §2.1, §4; [hcom] `-e`; [cotal] expected-session check |
| 5 | Session continuity: handle `session_start.reason` + `previousSessionFile` for `/new`/`/resume`/`/fork`/reload as a binding transition, never a seat merge; keep the watch child in a process-global map (Cotal's `Symbol.for`) so reload does not reconnect | medium | low-medium | primer §2.1, §5; [cotal] |
| 6 | Sender-side "recipient not wakeable" outcome for a p0 item whose target pane is `Unknown`, a one-line warning in `send` output | medium (rule-5 honesty, every harness) | low-medium (core change, rule-(f) size statement) | flaw 6; [open-cross-session-ocs] outcome classes |
| 7 | Typed-wake fallback: if Herdr ever types `herdr-threads: wake up` into a Pi with the extension loaded, an `input` handler transforms it into the pending page, or `handled` when empty (hcom). Only as a fallback | low-medium | low | `types.ts:1167-1187` (primer) ; [hcom] |
| 8 | Ship the skill to Pi through `~/.agents/skills/` or the extension package; render it harness-neutral (agmsg's lesson) | low-medium | very low | `pi:.../docs/skills.md:61` **V**; [agmsg] |
| 9 | Test `tool_result` content append as the in-turn path instead of steer: it keeps peer text attached to a tool result rather than a `user` turn, which is better for trust framing, and it is not cleared on abort | medium (Tr), unknown R | low to try | `types.ts:1268` **V**; suggested in [hcom] §7.4, nobody does it |
| 10 | Generalize "harness reports its own idle" so an in-process channel adapter needs no Herdr composer parser (the neutral core already allows `IdleWake::Channel`; it may need no change, unknown until tried) | medium (OpenCode too) | unknown | `src/harness/registry.rs:1270-1282` **V** |

Rejected: tmux/Herdr keystroke wake into Pi as the primary path. On Pi a typed Enter during a stale-idle window
becomes a steer, a paste splices into drafts, and `[paste #N]` collapse and fullscreen rendering make scraping
fragile (primer §6). Also rejected: MCP as the Pi channel. Pi's client is roots-only, ignores
resource/notification push, and hides tools under `codemode` by default (primer §3). Four reports hit exactly
this ([mcp-agent-mail], [agentchattr], [agent-peers-mcp-mikez93], [cross-agent-teams-mcp-jtianling]).

## 5. Primer surfaces nobody uses yet

Across the 12 reports (each states the absence after a grep of the tool's Pi code; **V** per report):

| Surface | Primer | Possible use for us |
|---|---|---|
| `ctx.ui.getEditorText()` / `onTerminalInput` | §2.4 | exact draft hold, and "human is typing" from keystrokes |
| `ui_prompt_start` / `ui_prompt_end` | §2.2 | hold wakes while an extension dialog waits on the human (the analogue of CAO's `Allow tool:` gate) |
| `sendMessage(..., {triggerTurn:false})` and `deliverAs:"nextTurn"` | §2.3 | silent arrival for `p > T` (rules 1-2) |
| `before_agent_start` returning `message` for mail | §2.2 | turn-start delivery of the pending page, bound to a human-started turn (hcom uses it only for its bootstrap) |
| `tool_result` content replacement | §2.2 | in-turn delivery attached to a tool result (#9) |
| `turn_end` / `agent_before_settle` `entries` + `continue` | §2.2 | "deliver before stopping" without a new run; loops unless guarded |
| `context` as an *injection* (request-local, not persisted) | §2.2 | ephemeral status, e.g. "N pending p>T items" with zero transcript growth; Cotal uses `context` only to observe |
| `agent_settled.aborted` | §2.2 | post-abort hold (hcom listens to `agent_settled` but ignores the flag) |
| `pi.appendEntry` (persisted, not sent) | §2.3 | a local delivery record in the session JSONL for diagnostics and receipt evidence |
| `pi.registerMcpServer` (with `exposure: "direct"`) | §3 | per-session tool surface with no `mcp.json` write, if we ever want tools |
| `registerFlag` / `getFlag` | §2.1 | `pi -e ht.js --ht-seat <id>` instead of env plumbing |
| OSC 7501 program status | §7 | host-side idle/busy for Herdr, if Herdr consumes it (unknown) |
| `steeringMode`/`followUpMode` = `all` | §2.3 | not ours to set (user config); batching one page per call achieves the same |
| `cache_warming_decision` | §2.2 | not relevant to delivery |
| RPC / SDK | §4 | not attachable to an interactive Pi; only for a headless managed seat, which nobody builds |

## 6. Bottom line

The field has converged: on Pi, the integration is an in-process extension that calls `pi.sendMessage` with a
trigger, and the wake is the delivery. hcom and Cotal are the references, Cotal for delivery proof and abort
handling, hcom for zero-footprint `-e` attach and launch readiness. What none of them does, and our delivery
contract requires, is decide *whether* to trigger:
- priority against the seat's threshold;
- `isIdle()`;
- an empty editor;
- no open dialog;
- no post-abort hold.

Our design is better in principle and absent in practice. The cheapest win (#1) closes an actual
misattribution today. The main build (#2-#4) should use a `details`-id proof instead of the unobservable
`sendMessage` result that `../ours/pi.md` proposed.
