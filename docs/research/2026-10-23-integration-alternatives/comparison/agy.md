# Comparison: message injection and wake on Antigravity CLI (agy)

- Inputs: `../ours/agy.md`, `../primers/agy.md`, and `../alternatives/<slug>/agy.md` for agent-talk, cotal, agmsg,
  ruflo, mcp-agent-mail, hcom, agentchattr, cross-agent-teams-mcp (xats), agent-peers-mcp, cyclops,
  open-cross-session (ocs) and CAO. Citations into an alternative go through its report (which carries the
  clone path and line); herdr-threads paths are repo-relative at `main` `2de3da1c4`.
- Tags: **VERIFIED** = read in source, docs, a committed capture or a cited report's verified item; **INFERRED** =
  reasoning; **unknown** = nothing read answers it. Claims about our code were re-read for this file (see
  "Verification").
- Caveat that applies to every row: agy is not installed on this host and no tool, ours included, was run live for
  this comparison. Our runtime evidence is the native gate on agy 1.3.2 with a scripted fake model
  (`docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/summary.txt`, 14/14 PASS). The alternatives' evidence is
  1.0.x-1.2.x (agentchattr 1.1.7, agmsg 1.1.27, cyclops 1.1.11-1.1.23, CAO 1.0.10, hcom 1.2.16), or none.

## 1. Mechanism table

| Tool | agy support | Injection carrier (what puts peer text in the model's context) | When | Bounds | Delivery evidence / ACK | Wake | Idle, busy and draft detection | Priority gate |
|---|---|---|---|---|---|---|---|---|
| **herdr-threads** | Native, owned plugin | `PreInvocation` `injectSteps` `systemMessage` (downgrade to `userMessage`); lifecycle check-in at `SessionStart` or first `PreInvocation` | Before every top-level model call: turn start and after each tool result | 32 items / 64 KiB page, 8 KiB body, 4 KiB check-in | Transcript scan for `SYSTEM_SDK` frame header at column 0 → `DeliveryConfirm`; re-offer 60 s × 3, then park for `inbox` | Herdr `agent prompt` types content-free `herdr-threads: wake up` | Herdr `agent_status` idle/done + `AgyComposer` must read `Empty` + focused pane empty for 60 s; any draft, busy footer, permission menu, overlay or live `Agent(` row → refuse | Yes: wake iff `p <= T`; default p1/T=0 never wakes |
| agent-talk | Plugin with skills only | None automatic; model runs `retalk receive` via `run_command` from a skill | When the model decides | None | Relay deletes on read | None | n/a | No |
| cotal | None (generic MCP by hand) | MCP tool result (`cotal_wait` / `cotal_inbox`), plus a "N new messages" line on other cotal tool results | When the model calls a cotal tool | 48,000-char reply, assemble-then-trim | Cleared when the tool result is written to the transport | None; opt-in 50 s long-poll keeps a turn alive | n/a (nothing typed) | Wake-worthy filter for `cotal_wait` only |
| agmsg `turn` | Native, experimental | Project rule file asks the model to run `check-inbox.sh` after each tool call | When the model complies; 60 s cooldown | None | Marked read after stdout write | None | n/a | No |
| agmsg `agy-tui` | Native, experimental | Bracketed paste of a full envelope as a user turn, from a PTY supervisor that owns agy | Only when idle | 20 rows, 64 KiB bodies | Model must print `AGMSG_RECEIVED:<uuid4>`; matched on reconstructed screen; single-writer read guard | The paste is the wake | Own VT model: bottom `>` row above `? for shortcuts … ·`, 0.3 s output quiet, zero-timeout select on stdin and PTY, human-typing pause with 0.6 s hysteresis | No |
| agmsg headless | Native, experimental | `--input-format stream-json` user lines to a separate worker conversation | Every 2 s poll | 20 rows, 64 KiB | stream-json `result` SUCCESS | Worker always wakes; human TUI never | n/a | No |
| ruflo | None (generic MCP by hand) | `team_inbox` MCP tool result (pretty JSON) | When the model calls it | None | Drain = delivery (rename to archive) | None | n/a | Ordering only |
| mcp-agent-mail | None (Gemini CLI installer misfires on `~/.gemini`) | `fetch_inbox` MCP tool result | When the model polls | `limit` ≤ 1000, bodies whole | Separate `acknowledge_message` call | None | n/a | No |
| hcom | Native, released | `PreInvocation` `injectSteps` **`ephemeralMessage`** (bodies), plus a ~1k-token bootstrap ephemeral on every model call | Before every model call | 50 messages, 1 MiB each, no byte cap | Cursor advances on stdout flush; no confirmation possible (ephemeral is not persisted) | hcom-owned PTY: types `<hcom>[intent:thread #id] a -> b</hcom>` metadata preview + Enter | DB status `listening` from `Stop` (`fullyIdle` / `NO_TOOL_CALL`); `? for shortcuts`; `>` row empty; approval scrape (2 of 3 signals); exact-ownership check before Enter | No |
| agentchattr | Native, thin (config + launcher) | `chat_read` MCP tool result; wake prompt text itself | After a wake prompt | 20 messages, no byte cap | Read cursor, no ACK | tmux paste + Enter of `use mcp to read #ch …` | None (blind) | No |
| xats | None (generic MCP + tmux) | `get_inbox` MCP tool result | After a hint | 50-200 rows, bodies unbounded | Cursor advance; sender awaits "read" | tmux bracketed paste of a ≤200-char hint + Enter | 2 s screen-quiet only | No |
| agent-peers-mcp | None (generic MCP by hand) | `[PEER INBOX]` block prepended to any agent-peers tool result | When the model calls one of its tools | None | Confirmed by the *next* tool call | None; 300 s in-turn long-poll | n/a | No |
| cyclops | Native, "weakest member" | Pasted doorbell user turn; body only via `cyclops inbox claim` tool call | As soon as admitted, **including mid-turn** (vendor queues the line) | Summary 200 chars; body unbounded (1 MiB wire) | Screen receipt (staged → left composer + spinner); claim is the only exact proof | tmux paste of doorbell, verify staged bytes, one Enter, never re-pressed | Screen rules on escaped capture (escaped `>` glyph, braille spinner, quota/survey/file-access modals); human draft holds until a pane event | No |
| ocs | Headless delegate only | `agy -p <preamble + note> --conversation <per-channel id>` per message | At send time, one global serial queue | 4096 B body inline (else 512 B preview), 5120 B note | Job id = hash(channel, seq); at-most-once execution and reply | Fresh headless process per job; live TUIs unreachable | n/a (cmux fallback checks only `to interrupt`) | No |
| CAO | Native (tmux only) | Body pasted as a user turn + sender suffix | Only when footer reads idle | None (URL query param) | Marked DELIVERED before paste; no ACK | Same paste: bracketed paste, 1.5 s, one Enter | Footer regexes on a pyte viewport + own turn counter; sticky ready latch; no composer reader | No |

Sources: VERIFIED in each report's "Support status", "Injection mechanism" and "Wake mechanism" sections; our row
VERIFIED in `src/harness/agy/{delivery,composer,decode,plugin,mod}.rs`, `src/host/native.rs:1784-1933`,
`src/notification/dispatch.rs:99-127`.

Headline (INFERRED from the table): only herdr-threads and hcom use agy's in-process hook injection at all, and only
herdr-threads confirms that an injected step actually landed. Everyone else either makes the model pull through a
tool call or types the message (or a pointer to it) as a user turn.

## 2. Differences from our approach

Each difference lists the alternative(s), then pro/con against ours on the axes that matter: latency (L),
reliability (R), token cost (T), draft safety (D), setup footprint (S), harness-version fragility (V),
cross-machine (X), trust (Tr). Axes a difference does not touch are omitted.

### D1. Model-initiated pull through an MCP tool or CLI call, instead of hook push

Who: agent-talk, cotal, agmsg `turn`, ruflo, mcp-agent-mail, agentchattr, xats, agent-peers, cyclops (claim step).

- L: worse. Delivery waits for the model to decide to call a tool; for an idle session it is unbounded. Ours reaches a
  busy session at its next model call (VERIFIED, gate `wake` row c).
- R: worse. Depends on the model obeying a skill, rule or tool description; agmsg's rule path (`.agent/rules/`) may not
  even load on 1.3.x (unknown, agmsg report). Ours does not depend on model compliance to enter context.
- T: worse. At least one extra model round trip per delivery (tool call + result), and the standing MCP schema cost:
  about 8.7k tokens of docstrings for mcp-agent-mail, about 9-10k for xats, thousands for CAO/ruflo (INFERRED estimates
  in those reports; whether agy sends schemas eagerly is unknown). Ours costs 0 bytes on a quiet turn
  (`delivery.rs:1131-1133,1149-1151`).
- D: better or equal. A pull types nothing.
- S: mixed. A hand-written `mcp_config.json` entry is small, but agentchattr and CAO write the user's **global**
  `~/.gemini/config/mcp_config.json`, with live tokens and a corrupt-file reset to `{}` (VERIFIED in both reports). Ours
  writes an owned plugin directory with a digest manifest (`src/harness/agy/plugin.rs:1-7`).
- V: better. MCP tools are a documented agy surface; `systemMessage`'s inner shape is undocumented and we needed the
  downgrade machinery (`delivery.rs:477-569`). This is the one axis where pull wins.
- X: better for HTTP MCP servers (a remote server works the same); ours is local-daemon plus the p2p sync layer
  (not examined here).
- Tr: worse. Peer text arrives as an ordinary tool result (`MODEL/GENERIC`) with no provenance marking; ours lands as a
  `SYSTEM_SDK` step with a fixed frame. Model-supplied identity (`team`/`agent` in ruflo, `connection_id` in cotal)
  lets one session drain another's mail (VERIFIED code, INFERRED risk).

### D2. Content-bearing keystroke delivery (the message, or a pointer to it, typed as a user turn)

Who: agmsg `agy-tui` (full envelope), CAO (full body), agentchattr (instruction prompt), xats (hint), cyclops (doorbell
with claim command), hcom (metadata preview), ocs cmux fallback (5 KB note).

- L: better for idle peers in the best case (1-3 s; agmsg 2 s poll, CAO 1.5 s delay, cyclops ~1 s). Ours wakes only for
  `p <= T`, and a focused pane needs 60 s of observed-empty samples first (`native.rs:71,1744-1782`).
- R: worse. One turn per message in CAO; agentchattr and CAO have no submit recovery on agy; cyclops measured that the
  first paste after agy startup does not stage (1.1.x) and then never re-rings.
- T: worse. Every message costs a full turn and, for pointer forms, a second tool round trip. Bodies typed into the
  composer become permanent user turns. Ours: one content-free wake (22 bytes plus agy's wrapper) and the bodies ride
  the woken turn's first `PreInvocation`.
- D: much worse for the blind ones. agentchattr (no gate), xats (2 s quiet, passes a static draft and a permission
  menu), CAO (sticky ready latch, no composer reader, reads live-subagent screens as idle by its own regexes), ocs
  cmux (`to interrupt` never matches agy's `esc to cancel`). A permission menu is the worst case: Enter can approve a
  command. Ours refuses any non-`Empty` composer (`composer.rs:58-116`).
- V: worse. Each depends on 1.0-1.2 screen strings; cyclops documents a regression per 1.1.x minor. Ours also depends
  on a screen grammar, captured on 1.3.2 (`tests/fixtures/agy/composer/`), but only for the *wake*, not for delivery.
- Tr: worse. Typed text is `USER_EXPLICIT` input, indistinguishable from the human except by content.
- One real pro: the human watching the pane sees who wrote (hcom preview, cyclops header). Our `systemMessage` is
  invisible in the TUI (primer §2.3). See D8.

### D3. `ephemeralMessage` instead of `systemMessage`

Who: hcom.

- T: hcom's quiet-turn cost is far higher (≈1k-token bootstrap re-sent on every model call because ephemeral is not
  persisted). Per delivery, ephemeral costs once per call it is shown, not cumulatively in history.
- R: worse for hcom. The body is visible for one model request only, so a multi-step request loses its text after the
  first tool call (hcom report, limitation 1, INFERRED strong). And it cannot be transcript-confirmed, so hcom acks on
  stdout flush.
- V: better. `ephemeralMessage` was observed working by our own probe (primer §2.3); `systemMessage`'s shape is
  undocumented.
- Verdict: confirms our choice for bodies. Ephemeral fits only idempotent, re-offerable context.

### D4. The integration owns the PTY (or tmux session) instead of going through Herdr

Who: agmsg `agy-tui`, hcom, CAO, cyclops, agentchattr, xats.

- D: better observability. agmsg sees raw keystrokes and pauses delivery on any human input; hcom checks the
  composer equals exactly the text it typed before Enter. We see only Herdr's detection text and `agent_status`.
- S: worse. The user must launch through a wrapper (`agy-tui`, `hcom agy`, tmux session); a plain `agy` gets nothing.
  Ours works on any agy in a Herdr pane after `setup agy`.
- R: mixed. agmsg's supervisor stops on any keystroke during a delivery turn and needs manual `ack`/`replay`.
- Verdict: not for us (Herdr owns the PTY), but two of their checks are portable (ideas B3, B4).

### D5. Hook-maintained busy/idle flag

Who: hcom (`Stop` with `fullyIdle` or `NO_TOOL_CALL(S)` sets `listening`; a delivery ack or `PreToolUse` sets
`active`), cyclops (`PreInvocation` = turn start, `Stop` = turn end).

- R: better as an extra signal: it is native, not scraped, and catches screens the grammar misses. hcom shows the
  failure mode: a `Stop` without `fullyIdle` soft-stops the instance and stalls wakes (hcom report, limitation 9).
- V: better (payload fields are documented).
- Ours parses `fullyIdle` (`src/harness/agy/contract.rs:55`) and uses it for nothing (VERIFIED, grep). See B2.

### D6. Pre-submit verification of the typed text

Who: hcom (exact equality of composer and injected text, re-checked before each Enter), cyclops (exact staged-bytes
proof above the trailer at 0/120/240/480 ms), CAO (dispatch-boundary generation counter).

- R/D: better. They separate "typed" from "submitted" and refuse Enter on a mixed composer.
- Ours: Herdr `agent prompt` types and submits in one call (`native.rs:1896-1919`); we check only *after*, with one
  advisory read 250 ms later, and a held prompt is reported, never re-entered or cleared (`dispatch.rs:99-127`). See F2.

### D7. Model receipt token

Who: agmsg (`AGMSG_RECEIVED:<uuid4>`, construction rule stated, literal never shown).

- R: proves the model attended, not just that the step was injected. Ours proves injection (`SYSTEM_SDK` record,
  `delivery.rs:369-412`).
- T: about 50 output bytes per batch plus a fixed ~230-byte instruction; against token rule (b) as a default.
- Verdict: only plausible for ACK-requested items, where we already require an explicit `herdr-threads ack`.

### D8. Human-visible wake text

Who: hcom (metadata preview, ≤ width−15, sanitized), cyclops (one-line header + summary), xats (hint names the
addressee).

- Pro: the person watching an agy pane sees who wrote and where, which ours hides twice: the wake is content-free (R18)
  and the bodies are an invisible `systemMessage`.
- Con: permanent bytes per wake (40-80), against R18. xats' `→ name@team` addressee check is a correctness aid only
  where identity is pane-inferred, which ours is not.

### D9. Headless delegate instead of a live seat

Who: ocs (`agy -p --conversation <id>` per job), agmsg headless bridge (stream-json worker).

- D: perfect (no TUI). L: process start per job (ocs) or a persistent worker (agmsg). T: ocs re-reads the whole
  per-channel conversation each job, unbounded growth.
- R: ocs is at-most-once with honest "interrupted" reporting; agmsg's measurement shows the headless turn does **not**
  reach the live TUI's in-memory context even on the same conversation id (agmsg report, 1.1.27).
- Verdict: a different product (an "ask an agy" service), not a replacement for a live seat.

### D10. Long-poll tool as the listener

Who: cotal (`cotal_wait`, 50 s), agent-peers (`wait_for_peer_messages`, 300 s).

- L: immediate while held. T: one model call per empty cycle, forever; it keeps the session busy so a human cannot type.
- R: agent-peers' 300 s exceeds agy's 180 s MCP default, which can lose a drawn message if agy sends no cancel
  (agent-peers report, INFERRED).
- Verdict: against the delivery contract's "idle stays idle" goal; only an explicit opt-in mode.

### D11. Setup surface choices

- hcom merges into the **global** `~/.gemini/config/hooks.json` (one shared file where one malformed key drops every
  namespace, primer §2.1) and registers fail-closed `PreToolUse` on every agy session on the machine; it guards each
  command with `command -v hcom || printf <allow JSON>`. cyclops writes `~/.agents/hooks.json`, which may never load
  (its own self-test says workspace only). agmsg writes a project rule file. Ours: an owned plugin directory, no tool
  hooks, so no fail-closed event (VERIFIED, `plugin.rs:58-95`, golden `tests/fixtures/agy/plugin/hooks.json`).
- hcom auto-allows 38 `command(hcom …)` rules including `hcom term` (keystrokes into another agent's PTY without a
  prompt). Ours adds one `command(herdr-threads)` only with consent (`integrations/agy/README.md` "Setup").
- Verdict: ours is the smallest and safest footprint among the native integrations.

### D12. Hook process count per step

- hcom: 3 processes per model call (two `PreInvocation` handlers plus `PostInvocation`) and 2 per tool call, each
  opening SQLite; cyclops: 1-2 per model call and 2 per tool call. Ours: 1 per model call (`PreInvocation`) plus one
  per `Stop` and `SessionStart` (VERIFIED, golden hooks file).

## 3. Flaws in our approach that the others expose

Ordered by severity.

**F1 (medium). A message that arrives after the turn's last model call misses that turn.**
We inject only at `PreInvocation`; `Stop` confirms and prints nothing (`delivery.rs:1292-1335`), and there is no
`PostInvocation` handler (`plugin.rs:68-73`). An item that arrives during the final model call of a turn is not seen
until the next turn. If it is p ≤ T, the session then goes idle and needs a keystroke wake plus a fresh turn; if it is
the default p1 at T = 0, it waits for the next human prompt. agent-talk's Codex `Stop` block-and-continue (cursor
advanced before output so it cannot loop) and agy's own `Stop` `decision:"continue"` show the session could be kept in
the same turn instead (agent-talk report, "Clever ideas" 4; primer §2.3). Nobody implements this on agy either, so it
is a gap, not a lag behind a competitor. VERIFIED (absence in code), impact INFERRED.

**F2 (medium). A wake whose Enter is swallowed leaves our marker in the composer, and nothing removes it.**
`submit_prompt_mode` hands the text to Herdr's `agent prompt`, which types and submits in one call
(`src/host/native.rs:1896-1919`). The only check afterwards is one advisory read; a held marker is reported as
`Unsubmitted`, never re-entered and never cleared (`src/notification/dispatch.rs:99-127`). `AgyComposer` declares
`ctrl+u` but it is unused (`composer.rs:118-127`). Consequences (INFERRED): the composer now reads `Text`, which blocks
every later wake until a human acts, and a human who starts typing extends our marker. Two independent data points say
this is not hypothetical on agy: CAO needed a 1.5 s paste-to-Enter delay because "an Enter sent that soon is consumed
as a literal newline" on Gemini 3.x agy (CAO report, `providers/antigravity_cli.py:261-278`), and cyclops measured that
the first paste after agy startup does not stage (cyclops report, `agy.toml:195-198`). Both are 1.0-1.1 era and use tmux
paste, not Herdr; how Herdr's `agent prompt` types and times Enter is unknown. Our gate's `wake` row passes on 1.3.2
(fake model), so the common path works.

**F3 (medium). The bare-wake fallback instruction is in the skill body, which agy reads only on demand.**
If a wake turn starts and the hook did not inject (daemon down, 1.5 s budget exceeded, transcript missing), the model
sees only `herdr-threads: wake up`. The instruction "if a turn starts with either and no `[herdr-threads] new
messages:` block was injected, run `herdr-threads inbox` once" lives in `integrations/skill/SKILL.md:24`. On agy the
skill is listed by name and path and read via `view_file` (primer §3); the always-present `AGY_COMMAND_GUIDANCE`
(`src/harness/agy/mod.rs:33`) does not carry it. cyclops' doorbell and xats' hint are self-sufficient by contrast
(each names the command to run). VERIFIED text, INFERRED impact.

**F4 (medium). Wake admission rests on Herdr status plus one screen grammar; no native busy signal crosses it.**
hcom and cyclops both derive turn state from `PreInvocation` and `Stop`; we parse `fullyIdle` (`contract.rs:55`) and
use nothing from `Stop` for wake. CAO's regex check on our own 1.3.2 fixtures shows how easily a footer-based reader
takes the live-subagent screen for idle (CAO report, limitation 12); our `Agent(` row check covers that screen
(`composer.rs:76-83`) but a subagent without a visible row would be missed (ours report, "Failure modes"). A hook flag
would also catch "background tasks still running" (`fullyIdle:false`), which no screen shows. INFERRED impact.

**F5 (medium). Validation gaps: agy is outside the required wake e2e, macOS-only, fake model, no Linux.**
AGENTS.md "Pre-release wake e2e" covers Claude and Codex only; the native gate is opt-in and macOS
(`docs/evidence/agy-live-smoke/README.md:14-16`). Delivery-to-wake latency and whether real Gemini acts on an injected
`systemMessage` are unmeasured (ours report, "Latency"). agent-talk runs an auth-free `agy plugin install` smoke on a
Linux CI runner (agent-talk report, `ci.yml:183-220`), which shows a cheap packaging check is possible. VERIFIED.

**F6 (low). Delivery evidence proves injection, not attention.**
A `SYSTEM_SDK` record with our header proves agy put the step in the conversation, not that the model read it.
agmsg's receipt token is the counterexample. For ordinary items this is the right trade (token rules); for
ACK-required items we already need an explicit `ack`. VERIFIED (`delivery.rs:369-412`).

**F7 (low). Unreadable screens are silent to the operator.**
A quota screen, a feedback survey or a new modal reads `Unreadable` → `Unknown` → no wake, ever, until the grammar is
updated (by construction, `composer.rs`). cyclops names quota and survey states and pings an admin after 120 s of hold;
ruflo's ADR-486 gives every adapter capability a `{supported, why}`. We have no per-seat "wake unavailable: <reason>"
surface that I found (not exhaustively searched; INFERRED).

**F8 (low). The human cannot see what peers told the agent.**
`systemMessage` is not shown in the TUI (primer §2.3) and the wake is content-free. hcom adds a metadata preview to the
wake precisely because agy hides hook output (hcom report, `src/delivery.rs:578-584`). A deliberate R18 trade-off, but
nothing in our TUI path compensates (the downgrade to `userMessage` is the only visible mode). VERIFIED fact, impact
INFERRED.

**F9 (low). Focused idle panes wait at least 60 s for a wake.**
`FOCUSED_EMPTY_MILLIS = 60_000` (`native.rs:71`). Every alternative that types wakes in 1-3 s. This is draft safety
(a human looking at an empty composer may be about to type), and it only applies to focused panes; the cost is that a
p0 to a pane the user happens to have focused waits a minute. VERIFIED.

**F10 (low). Shared fragilities, not unique to us.** `plugin.json` carries a `version` key against documented
`additionalProperties:false` (agent-talk's manifest has more extra keys and passes CI); hook env inheritance conflicts
with a 1.1.21 whitelist report (hcom works around it with a payload-schema fallback detector). VERIFIED (primer §2.4,
§3; hcom report idea 6).

Flaws others have that we do **not** share (checked): peer-name and body neutralization (`frame.rs:41-82` strips
C0/C1, DEL, U+2028/2029 and indents bodies, the same class cotal's `attributionSafe` covers); acking on output the
harness discards (we have no tool hooks); subagent/parent conflation (our presence-lock witness, `witness.rs:176-237`,
versus hcom's Path 3 binding); blind paste into a permission menu.

## 4. Better integration ways to consider, ranked by value/cost

| Rank | Idea | From | Value | Cost | Notes |
|---|---|---|---|---|---|
| 1 | Put the bare-wake fallback line into `AGY_COMMAND_GUIDANCE` (or the lifecycle check-in), so it is present without a skill read | F3; cyclops/xats self-sufficient doorbells | Medium: removes a dead-turn failure mode | Trivial: ~90 bytes once per lifecycle context; within rule (a) as per-session static text | State the size per rule (f) |
| 2 | After an `Unsubmitted` verification, if a fresh composer read equals exactly `herdr-threads: wake up` (nothing else), clear it with the declared `ctrl+u`; never touch a composer that differs | F2; hcom exact-ownership, cyclops staged-bytes proof | Medium-high: stops one swallowed Enter from blocking all later wakes | Low-medium: needs a neutral "remove own leftover" trait method; exact-equality guard keeps drafts safe | Also count it in diagnostics |
| 3 | Hook-maintained turn flag (set at `PreInvocation`, cleared at `Stop` with `fullyIdle:true`) used only as an additional refusal (AND with Herdr status and composer), never as admission | F4; hcom, cyclops | Medium: catches background tasks and unmodelled busy screens | Low: state already written per conversation; needs a TTL so a missing `Stop` cannot block forever (hcom's stall) | Neutral trait: adapter-supplied idle evidence |
| 4 | Same-turn delivery at `Stop` via `decision:"continue"` (or `PostInvocation` `injectSteps`) for items that arrived after the last `PreInvocation` and would otherwise need a wake (p ≤ T) | F1; agent-talk's Codex Stop pattern; primer §2.3 | High: converts keystroke wake + new turn into in-turn delivery for a busy session (rules 3-4 compatible) | Medium: probe first (continue cap unknown; `reason` framing and its transcript record type unknown, so confirmation must be re-derived); our `Stop` budget is 1.5 s; never continue on `fullyIdle:false` errors or user cancel | Needs a new neutral end-of-turn trait method |
| 5 | Probe `agentapi send-message <conversation_id>` against a live TUI with agmsg's kiwi/mango memory test (and its effect on a typed draft) | Primer §11.1; agmsg negative result on 1.1.27 | High if positive (keystroke-free wake), likely negative | Low for the probe | Do not build on it before the probe |
| 6 | Probe the statusline/window-title JSON `state` feed as a native idle signal | Primer §9 (no tool uses it) | Medium | Low probe; medium to wire through Herdr | Would cross-check Herdr #2354 |
| 7 | Auth-free Linux `setup agy` / plugin-load smoke in CI | F5; agent-talk CI | Medium: catches plugin-root drift and manifest strictness | Low-medium (agy installer in CI) | Not a version gate; detection only |
| 8 | Add agy to the wake e2e as an opt-in real-model run (latency, systemMessage obedience) | F5 | Medium-high | Medium (macOS, Gemini quota) | Feeds the delivery contract audit |
| 9 | Per-seat wake-capability reason in `doctor` (downgraded to `userMessage`, composer `Unreadable` with the matched screen class, focused-pane hold) | F7; ruflo ADR-486, cyclops named modals | Medium | Low | Diagnostics only, no model-visible bytes |
| 10 | Wake confirmation by the next `PreInvocation` with `invocationNum == 0` for the woken conversation, instead of the 250 ms screen read | CAO dispatch boundary | Low-medium | Low-medium | Turns `NotChecked` into evidence |
| 11 | `ephemeralMessage` for content that needs no confirmation (notices under ht-7jh, check-in digests on re-offer) | hcom (counter-lesson), mcp-agent-mail mapping | Low-medium: no permanent transcript bytes | Low-medium; must never carry bodies or be acked on emit | Keep bodies on `systemMessage` |
| 12 | Opt-in human-visible wake preview (sender + thread id only) | F8; hcom | Low-medium | Low; violates R18 by default, so opt-in with stated size | |
| 13 | Short `[re-offered]` marker on re-offered blocks | cotal delivery-count marker | Low | Low; ~14 bytes per re-offer | Helps a model avoid re-executing a request |
| 14 | Headless agy responder seat bound to a thread | ocs | Speculative | High | Separate product |

Not to adopt (VERIFIED harms in the reports): content-bearing keystroke delivery (D2), writing the global
`mcp_config.json` or `hooks.json`, auto-allowing keystroke-injecting subcommands, `--dangerously-skip-permissions`
(agmsg, CAO), bodies in `ephemeralMessage` (hcom), model-supplied mailbox identity (ruflo, cotal), unbounded tool
results, long-poll listening as a default (cotal, agent-peers).

## 5. Primer surfaces nobody uses yet

From primer §1 and §6-§9, against all 13 tools (VERIFIED absence per each report's grep or code reading):

| Surface | Used by | Status |
|---|---|---|
| `Stop` `decision:"continue"` + `reason` | nobody | hcom answers `{"decision":"allow"}`; ours prints nothing. Idea 4 |
| `PostInvocation` `injectSteps` / `terminationBehavior:force_continue` | nobody injects; hcom and cyclops register it as a no-op or liveness | Idea 4 |
| `PreToolUse` `overwrite` | nobody (hcom/cyclops use `PreToolUse` for status only) | A hack; not recommended |
| `agentapi send-message` / `new-conversation` | nobody | Idea 5; agmsg's same-conversation negative result is the closest evidence |
| Sidecars (`sidecar.json`) | nobody | Unknown whether the CLI runs them |
| Remote Control | nobody (ours refuses `--remote-control` at launch) | No API, Google sign-in |
| `/schedule` / `schedule` tool | nobody | A model-driven poll; against token rules |
| `/goal`, `/btw` | nobody | Mechanism undocumented |
| Python SDK (`pre_turn`, `every`) | nobody | Only for SDK-hosted agents |
| ACP | nobody (not native) | |
| Statusline / window-title JSON `state` | nobody | Idea 6 |
| `ctrl+y` kill-ring restore (draft stash) | nobody | Would enable `PokeCapabilities` beyond `NONE` if it works; untested |
| MCP `notifications/*`, resources, prompts | agent-peers emits `notifications/message`; surfacing on agy unknown | |
| `SessionStart` `injectSteps` | herdr-threads only (hcom fakes it with a `PreInvocation` handler; cyclops omits it) | |
| `transcriptPath` confirmation, `presence/` lock, `subagents/` listing | herdr-threads only (cyclops proposed the transcript match but did not build it) | Independent confirmation that our confirm design is the right one |
| `fullyIdle`, `terminationReason` for state | hcom only; ours parses `fullyIdle` unused | Idea 3 |
| `AGY_CLI_DISABLE_AUTO_UPDATE` | agentchattr launcher | Useful only for test gates; never as a user pin (AGENTS.md versionless rule) |

## 6. Verification

Re-read for this file in herdr-threads at `2de3da1c4`:

- `src/harness/agy/composer.rs:1-132`: rule anchoring, single footer line, `idle_footer`, `Agent(` row check,
  `PokeCapabilities::NONE`, unused `clear_key`/`restore_text`.
- `src/host/native.rs:1784-1933`: `submit_prompt_mode` (witnessed `agent get`, `cooperative_wake_ready`, `Empty`
  composer, focused 60 s window, one `agent prompt` call); `:66,71` constants; `:1508-1545` advisory read.
- `src/notification/dispatch.rs:39,99-127`: `verify_submission` reports `Unsubmitted` and does nothing else.
- `src/harness/agy/delivery.rs:50-66,1295-1352`: tracking limits, probation, abort markers, miss rule.
- `src/harness/agy/decode.rs:10-100,150-188`: step encodings, 4096-byte context cap, lifecycle prefix.
- `src/harness/agy/mod.rs:28-56` (`AGY_COMMAND_GUIDANCE`, budgets) and `plugin.rs:25-55` (skill tail): neither carries
  the bare-wake rule; `integrations/skill/SKILL.md:24` does.
- `src/delivery/frame.rs:1-162`: `one_line` and `indent` neutralization.
- `src/harness/agy/contract.rs:55`: `fullyIdle` parsed; grep finds no other use under `src/harness/agy/`.
- `src/harness/registry.rs:1268-1282`: `HookPull` requires `IdleWake::Native`.
- `docs/evidence/agy-native-gate/2026-10-11-38c5fef6d2e3/summary.txt`: 14/14 PASS.

Not verified here and therefore stated as unknown or INFERRED: how Herdr's `agent prompt` types text and times Enter on
agy; whether ANTIGRAVITY_CONVERSATION_ID leaks into children launched by herdr-threads (ocs's scrubbing idea; our launch
goes through a Herdr pane, and I did not trace its environment); agy `Stop` continue cap and `reason` record type;
every alternative's runtime behaviour on agy 1.3.x.
