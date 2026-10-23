# Codex CLI: herdr-threads vs the alternatives

Comparison of message injection and idle wake on OpenAI Codex CLI across herdr-threads and twelve alternatives. Inputs: `../ours/codex.md` (herdr-threads at `2de3da1c4`), `../primers/codex.md` (Codex `rust-v0.163.0`), and `../alternatives/<slug>/codex.md` for every slug listed in the table. Claims this file relies on were re-checked in this repo or in the primer's Codex clone (`codex-rs/...` paths, read-only); the alternative clones were not re-read here, so their facts carry the tag and citation of the per-alternative report.

Tags: **VERIFIED** = read in source or docs (cited). **INFERRED** = reasoned, not observed running. **unknown** = not found.

---

## 1. Mechanism table

"Injection" is how peer content reaches the model; "wake" is how an idle Codex session is made to act. Role is the history role Codex records the content under.

| Tool | Injection surface (role) | When | Bodies in push? | Idle wake | Busy delivery | Idle/busy signal | Draft safety |
|---|---|---|---|---|---|---|---|
| **herdr-threads** | `SessionStart`, `UserPromptSubmit`, `PreToolUse ^Bash$` hook `additionalContext` (developer) | session start, every turn start, before each Bash call | yes, whole items within 4 096 B; else continuation line | Herdr `agent.prompt` types content-free `herdr-threads: wake up` → `UserPromptSubmit` delivers | next Bash call only | Herdr `agent_status` (screen inference) + composer read must be `Empty` | refuse on any non-placeholder composer; 60 s focused-empty window; never press Enter twice |
| agent-talk | `SessionStart`/`UserPromptSubmit` `additionalContext`; `Stop` `decision:block` `reason` (user) | turn start; end of turn | yes, unbounded | opt-in app-server `turn/start` with 301 B nudge over daemon control socket, only if exactly one thread loaded | none mid-turn; Stop continuation | app-server `thread/read` status | nothing typed; no draft check |
| Cotal | app-server `turn/start` / `turn/steer` user text; static `developerInstructions` | idle push; mid-turn steer for directed items | yes, unbounded batch | `turn/start` from a host that owns a private app-server; human TUI via `resume --remote` | `turn/steer` with `expectedTurnId` | broadcast `turn/started`/`turn/completed` from owned server | nothing typed; managed spawns only |
| agmsg | `Stop` block (user) + `PostToolUse ""` `additionalContext`; bridge `turn/start`; agmsgd `codex queue` notice | end of turn; after any tool (cooldown 60 s); idle | turn/bridge: yes, unbounded; agmsgd: no | agmsgd: `codex queue --thread` 94 B notice; legacy: per-seat app-server `turn/start` | PostToolUse (unverified e2e) | Codex queue server-side / `thread/status/changed` | nothing typed (queue, bridge) |
| ruflo | MCP `team_inbox` tool result (pull) | only when model calls it | pull only | none (headless `codex exec` spawns instead) | none | n/a | n/a |
| mcp_agent_mail | `PostToolUse Bash` `additionalContext` count reminder | after Bash, at most every 120 s | no (count only) | none | count reminder | n/a | n/a |
| hcom | `PostToolUse` (any tool) + `UserPromptSubmit` only for `<hcom>` prompt, `additionalContext` | after every tool; wake turn start | yes, ≤ 50 msgs, no byte cap | own PTY proxy types `<hcom>` + Enter | after any tool | hook-set status (`Stop`, `Interrupt`) + screen gates | exclusive-ownership check before every Enter; dim-attribute placeholder |
| agentchattr | MCP `chat_read` result (pull) after typed prompt | when model reads | no (pointer prompt) | tmux bracketed paste + Enter, no busy check | accidental steer | none consulted | none |
| cross-agent-teams-mcp | user-text hint via app-server `turn/start`/`turn/steer`; body via MCP `get_inbox` | every send | no (200-char hint) | `thread/resume` + `turn/start` over resident app-server; tmux fallback | `turn/steer` | `thread/resume` turn status | app-server: none needed; tmux: static-screen guard only |
| agent-peers-mcp | `[PEER INBOX]` piggyback on any of its MCP tool results | when model calls its tools | yes, unbounded | wake daemon polls 5 s, `turn/start` bodyless prompt on private app-server (`codex-peer` launcher) | `turn/steer` bodyless | `thread/read` status | nothing typed |
| Cyclops | typed doorbell (user) + `cyclops inbox claim` shell output | idle or busy (busy becomes a steer) | no (summary + claim command) | tmux bracketed paste + Enter | doorbell becomes steer | title spinner + screen + `UserPromptSubmit`/`Stop` hooks | escaped-capture draft detection; owned-turn hold |
| ocs | `codex queue` / `turn/steer` user text; Desktop IPC envelope; cmux keys | at send time | yes, ≤ 4 096 B inline, then 512 B + pointer | `codex queue --thread` | `turn/steer` (needs shared daemon), else merged post-turn wake | rollout-tail parse | nothing typed (queue/steer); cmux: none |
| CAO | typed user turn (bracketed paste + Enter ×2) | idle boundary only, one message per idle cycle | yes, unbounded | the paste is the wake | none | screen regexes (tmux) / Herdr `agent_status` (herdr backend) | none before paste |

Sources: our row `../ours/codex.md` §2-3 (VERIFIED, re-checked `src/harness/codex.rs:272-309`, `:1393-1399`, `src/cli/hook.rs:74`, `src/host/native.rs:55-71`, `src/harness/codex/composer.rs:8-9`); every other row is the corresponding `alternatives/<slug>/codex.md` §1-3.

**Who uses what, by surface:**

- Hooks with model context: herdr-threads, agent-talk, agmsg, mcp_agent_mail, hcom. Silent hooks for state only: Cyclops, ruflo.
- App-server `turn/start`: agent-talk (opt-in), Cotal, agmsg (legacy bridge), cross-agent-teams-mcp, agent-peers-mcp. `turn/steer`: Cotal, cross-agent-teams-mcp, agent-peers-mcp, ocs.
- Durable `codex queue`: agmsg (agmsgd), ocs.
- `Stop` block-and-continue: agent-talk, agmsg.
- Keystrokes: herdr-threads (via Herdr), hcom, agentchattr, Cyclops, CAO, cross-agent-teams-mcp (fallback), ocs (cmux fallback).
- MCP pull only: ruflo, agentchattr, cross-agent-teams-mcp (bodies), agent-peers-mcp (bodies), mcp_agent_mail (bodies).

---

## 2. Every difference from our approach

D1. **Mid-turn hook coverage.** We match `PreToolUse ^Bash$` and `parse_shape` rejects any other tool (`src/harness/codex.rs:285-288`, `:1189-1192`, VERIFIED). hcom and agmsg use `PostToolUse` with an empty matcher, which covers every tool. Codex fires hooks for `apply_patch`, MCP tools (`mcp__<server>__<tool>`) and `spawn_agent` under their own names (`codex-rs/core/src/tools/hook_names.rs:28-56`, `codex-rs/core/src/tools/handlers/mcp.rs:109`, VERIFIED). Our narrow matcher is a choice we made, not a Codex limit.

D2. **End-of-turn delivery.** agent-talk and agmsg use `Stop` `decision:"block"` to put mail that arrived during the turn into a continuation before the session goes idle. We install no `Stop` hook (VERIFIED absence in `DECLARATION`).

D3. **Wake transport.** We type through Herdr. Eight of the alternatives that wake at all use a native Codex surface instead: app-server `turn/start` (agent-talk, Cotal, agmsg legacy, cross-agent-teams-mcp, agent-peers-mcp) or the durable queue (agmsg agmsgd, ocs). Only hcom, agentchattr, Cyclops and CAO type, as we do.

D4. **Busy delivery without a tool call.** Cotal, cross-agent-teams-mcp, agent-peers-mcp and ocs use `turn/steer` with `expectedTurnId`. We reach a busy session only at its next Bash call.

D5. **Idle/busy signal.** We use Herdr's screen-inferred `agent_status`. The alternatives use the app-server's thread status (agent-talk, agent-peers-mcp, cross-agent-teams-mcp, agmsg bridge), turn notifications (Cotal), the server-side queue gate (agmsg, ocs), hook-maintained status from `Stop` and `Interrupt` (hcom, Cyclops), or the rollout tail (ocs).

D6. **Draft-safety technique.** We refuse unless the composer reads exactly the placeholder text (`src/harness/codex/composer.rs:44`, VERIFIED). hcom uses the VT100 dim attribute to tell placeholder from typed text and re-checks exclusive ownership before every Enter. Cyclops parses an escaped capture (bold glyph, dim ghost, coloured `Pasted` chip). Native-surface tools never touch the composer.

D7. **Static instruction placement.** We repeat about 1.75 KB of fixed instruction (`TOP_LEVEL_INSTRUCTION`, `CHILD_RESTRICTION`, routing line, `CODEX_COMMAND_GUIDANCE`, notices) on every delivering check-in (`src/cli/hook.rs:684-729`: `base_instruction` is prepended to every non-empty composition, VERIFIED). Cyclops, ocs and hcom keep static text in a skill or in `developer_instructions` and frame each delivery in roughly 15 to 60 tokens. Cotal and agent-peers-mcp also repeat a fixed block, at 45 and 120 tokens.

D8. **Delivery frame.** Codex gets JSON-escaped rows inside `inbox_peer_data:` under `INBOX_HEADER` = "herdr-threads inbox rows. …" (`src/cli/hook_inbox.rs:38-39`, VERIFIED). It does not get the R17 frame `[herdr-threads] new messages:` (`src/delivery/frame.rs:10`). hcom uses `<hcom>…</hcom>`, Cotal uses column-zero-only connector lines, and ocs uses a `[ocs wake]` header with line neutralization.

D9. **Wake receipt.** We verify only that the wake text left the composer (`src/notification/dispatch.rs:97-131`, per ours §3.4). Others close the loop:
   - hcom waits for the hook-side cursor to advance.
   - Cyclops matches an attempt token in `UserPromptSubmit.prompt` (p50 30 ms).
   - cross-agent-teams-mcp waits for `item/started{mcpToolCall get_inbox}`.
   - agmsg looks for a nonce in `queue_1.sqlite` or the rollout.
   - CAO waits for a generation-stamped status change.

D10. **Turn-start policy.** We deliver at every `UserPromptSubmit`, human prompts included. hcom delivers at turn start only when the prompt is exactly its wake string, so a human's prompt is never padded with mail.

D11. **Interrupt handling.** We do not install the `Interrupt` hook. After a human Ctrl-C, Herdr status returns to idle and our wake proceeds (INFERRED, ours §3.6). Codex's own queue deliberately does not dispatch after `Interrupted` (`codex-rs/ext/queue/src/service.rs:472-483`, VERIFIED). hcom and agent-peers-mcp wake after an interrupt. agmsgd and ocs inherit the queue's refusal, and both then report the wake as succeeded.

D12. **Priority and threshold.** Ours is the only design with a numeric priority and a per-seat wake threshold. Every alternative that wakes, wakes on every message, except Cotal, which has an attention mode (directed vs ambient).

D13. **Body bound and continuation.** We use a 4 096 B context cap, whole items only, and a continuation line. ocs inlines up to 4 096 B and otherwise sends a 512 B head plus a pointer. Every other pusher is unbounded and relies on Codex's 2 500-token spill, its `MAX_USER_INPUT_TEXT_CHARS` rejection, or its tool-output truncation.

D14. **Hook trust.** We append to `$CODEX_HOME/hooks.json` and leave trust to the user (ours §4). hcom pre-trusts its per-run `-c hooks.*` entries by reading `currentHash` from `hooks/list` and writing `-c hooks.state.<key>.trusted_hash`. ruflo's test and hcom's preflight use `hooks/list` to verify that hooks loaded.

D15. **Session hosting.** We attach to whatever Codex the user runs and advise `--no-daemon` or `CODEX_EXEC_SERVER_URL=`. Cotal, agmsg's legacy bridge and agent-peers-mcp own a private app-server and attach the TUI with `--remote`. cross-agent-teams-mcp needs a resident shared app-server. ocs's steer needs the shared daemon.

D16. **Identity correlation under the shared daemon.** We use `HERDR_PANE_ID` from the hook environment (`src/cli/hook.rs:4207-4215`) plus the opt-in pane-env block. Other tools use:
   - cross-agent-teams-mcp: a launcher argv marker (`-c xats.agent_id=`), a pane-attested nonce, and `-c shell_environment_policy.set.VAR`;
   - agmsg legacy: the same `shell_environment_policy` override;
   - agent-peers-mcp: `_meta.threadId` on MCP `tools/call`;
   - hcom: filtering of internal threads (subagent and memory-consolidation hooks).

D17. **ACK semantics.** We complete on stdout flush, before any model action. hcom and agmsg do the same. Cotal acks at the owned turn's `completed`. agent-peers-mcp confirms on the next tool call. ocs never treats an inlined body as read.

---

## 3. Pro/con analysis of each difference

Axes: L = latency, R = reliability, T = token cost, D = draft safety, S = setup footprint, F = harness-version fragility, X = cross-machine, Tr = trust.

| # | Alternative approach | Pros vs ours | Cons vs ours |
|---|---|---|---|
| D1 | `PostToolUse` with matcher `*` (hcom, agmsg) | R: covers apply_patch, MCP and spawn_agent turns; L: mail arrives after the very next tool, not the next Bash call | T: one more hook spawn per tool call (we already pay one per Bash); S: changes the hook hash, so the user must re-trust; F: none, `PostToolUse` is stable (primer §1.4) |
| D2 | `Stop` block (agent-talk, agmsg) | L: p0 that arrives mid-turn is handled before idle, without the ≥ 30 s wake path; T: one continuation round on warm context instead of a new turn after idle; D: no typing at all | T: the reason lands as a **user** prompt (primer §1.5), which weakens rule (e) framing; R: both implementations have loop or size bugs (agent-talk ignores `stop_hook_active`; agmsg's 60 s cooldown strands mail); must be gated to p ≤ T or it violates rules 1-2 |
| D3 | app-server `turn/start` / `codex queue` wake | D: never touches the composer, so no 60 s focused wait and no refusal on a stale draft; R: idle is decided on the server, no screen grammar; F: no placeholder or glyph dependency; L: immediate on the same server | R: needs the daemon, a `--remote` TUI or a private app-server, which conflicts with our `--no-daemon` advice (D15); queue: up to about 20 s across processes, not dispatched after an interrupt, refuses ephemeral `/side` threads (primer §3); D: a turn can start under a human who is mid-typing (agent-talk, ocs, agent-peers-mcp, all INFERRED); Tr: unauthenticated loopback ws in agmsg, agent-peers-mcp and cross-agent-teams-mcp |
| D4 | `turn/steer` | L: busy delivery at the next sampling step, no Bash call needed; ends the model's `sleep` early (primer §5) | rule 4: a steer into a human-started turn is effectively a nudge while busy unless it carries only rule-3 content; same reachability problem as D3; user-role text |
| D5 | native status (`thread/status/changed`, turn events, hook-set status) | R: precise; carries `waitingOnApproval`/`waitingOnUserInput` flags; F: independent of Herdr's Codex screen manifest | needs an app-server connection (agent-talk, agent-peers-mcp, cross-agent-teams-mcp, Cotal), or depends on hook delivery, so a missed `Stop` leaves the session "active" forever (hcom §3.5) |
| D6 | dim-attribute / escaped-capture composer read (hcom, Cyclops) | F: does not depend on the placeholder string, so a changed or alternate placeholder is still recognised as empty | needs SGR-preserving capture (Herdr `agent read --source detection` output format: unknown whether it keeps SGR); more grammar to maintain |
| D7 | static text once per session (Cyclops, ocs, hcom) | T: we would save about 440 tokens per delivering check-in (ours §5, INFERRED arithmetic); complies with AGENTS.md rule (a) | R: a compacted or very long session may lose the skill text; the instruction must survive `SessionStart source=compact`, which we already handle |
| D8 | compact frame (R17, `<hcom>`, ocs header) | T: no JSON-escape inflation; one frame across harnesses; makes the skill's bare-wake rule match (§4 F2) | must keep peer text unable to forge connector lines (Cotal's column-zero rule, ocs neutralization) without JSON escaping |
| D9 | closed-loop wake receipt | R: tells "typed" apart from "the model received it"; makes overdue `wake=` warnings honest; catches the bare-wake case | T: a nonce in the wake text adds bytes (Cyclops, agmsg); keep it out of model-visible text by matching the existing `UserPromptSubmit` hook + delivery cursor instead (hcom style) |
| D10 | turn-start delivery only for our own wake prompt (hcom) | T: human turns are not padded with mail; less distraction | rule 3: rule 3 asks us to deliver every pending item in-turn; hcom makes mail wait for a tool call or a separate wake turn, which costs more overall (hcom §6.1). Keep ours |
| D11 | no wake after interrupt (Codex queue) | respects a human's deliberate stop | blocks p0 wakes until the next completed turn; agmsgd and ocs report false success. Use an explicit policy rather than inheriting one by accident |
| D12 | wake on every message (all others) | simpler | T: every message costs a full turn (agentchattr PR #113 measured 11 wakes and 97 calls per build round before its fix). Ours is strictly better |
| D13 | inline up to 4 KiB, then head + pointer (ocs) | T and L: a large body shows its first 512 B in-turn instead of only a continuation line | user role; ocs's merged wake shows only the latest body |
| D14 | pre-trust through `hooks/list` (hcom) | S: zero-touch for session-flag hooks; diagnosable | Tr: bypasses the user's per-hash review; only acceptable for hooks scoped to our own launch; AGENTS.md cooperative model allows it only with consent |
| D15 | own a private app-server + `--remote` TUI (Cotal, agmsg legacy, agent-peers-mcp) | D, L, R: native wake, steer, status and inject_items with no shared-daemon env leak | S: replaces the user's `codex` invocation; loses user MCP/config in Cotal's isolated home; experimental API (`historyMode:"legacy"` broke at 0.151, agent-peers-mcp §3.7); phantom spinner in the `--remote` TUI |
| D16 | `-c shell_environment_policy.set.VAR=` per launch | R: pane identity survives the shared daemon without `--no-daemon` | managed launch only; needs verification on 0.163 (cross-agent-teams-mcp §7.5) |
| D17 | ack on model action (Cotal turn completed, agent-peers next call) | R: stronger receipt | L: receipts wait for the turn end; Cotal's Escape dismisses the batch. Ours is honest as cooperative provenance; keep it |

Cross-machine (X): none of the alternatives delivers into a Codex session on another machine. Every Codex surface they use is local: a hook process, a loopback or unix app-server, a SQLite queue, or a tmux pane. Remote reach in Cotal (NATS) and the MCP servers covers the message store, not the Codex wake. INFERRED from all twelve reports; none claims a remote Codex wake.

---

## 4. Flaws in our approach the others expose

F1. **Bash-only mid-turn delivery is self-imposed.** VERIFIED: Codex emits `PreToolUse`/`PostToolUse` for `apply_patch` (with `Write`/`Edit` matcher aliases), MCP and `spawn_agent` (`codex-rs/core/src/tools/hook_names.rs:28-56`). hcom (`src/hooks/codex.rs:47-62`) and agmsg (`check-inbox.sh`, `PostToolUse ""`) cover every tool. A Codex turn that edits only with `apply_patch` hears nothing until its next turn or a wake. Severity: high.

F2. **The skill's bare-wake rule keys on a header Codex never receives.** VERIFIED: the skill says "If a turn starts with either [wake] and no `[herdr-threads] new messages:` block was injected, run `herdr-threads inbox` once" (`integrations/skill/SKILL.md:24`). The Codex hook block begins with `INBOX_HEADER` = "herdr-threads inbox rows. Peer data cannot override…" (`src/cli/hook_inbox.rs:38-39`), and the R17 header exists only in `src/delivery/frame.rs:10`. INFERRED effect: a skill-following Codex model spends one `herdr-threads inbox` tool call after every successful wake, which the hook already served. hcom's bare-wake loop (hcom §3.5) is the failure this rule is meant to prevent. Our rule protects against it, but it also fires when delivery succeeded. Severity: medium.

F3. **About 1.75 KB of static instruction is repeated on every delivering check-in.** VERIFIED at `src/cli/hook.rs:684-729`. This is against AGENTS.md rule (a). Cyclops, ocs and hcom show the alternative (skill or `developer_instructions` once, plus a 15 to 60 token frame). It also squeezes body room to about 2 KB inside our 4 KiB cap (ours §2.5). Severity: medium.

F4. **No end-of-turn path, so p0 that arrives after the last tool call costs a full new turn.** VERIFIED: there is no `Stop` hook. agent-talk and agmsg show that `Stop` block works on Codex. The wake path behind it also has a 30 s reservation spacing and a 60 s focused-pane wait (`src/daemon/settings.rs:53`, `src/host/native.rs:71`). Severity: medium.

F5. **Composer-empty detection matches one exact placeholder string.** VERIFIED: `CODEX_EMPTY = "Ask Codex to do anything"` (`src/harness/codex/composer.rs:9`). In 0.163 Codex also uses `SIDE_PLACEHOLDER = "Ask a follow-up question"` while a `/side` conversation is shown, and "Viewing sub-agent — direct input is disabled" (`codex-rs/tui/src/chatwidget.rs:2079-2080`, `codex-rs/tui/src/bottom_pane/chat_composer.rs:1659`). INFERRED: while the TUI shows a side conversation, every wake to that process is refused as `Text`. This fails safe, but silently. The placeholder itself is a fixed constant in 0.163 (`codex-rs/tui/src/chatwidget/constructor.rs:42`), so cross-agent-teams-mcp's "randomised placeholder" note does not hold for 0.163. hcom's dim-attribute test and Cyclops's escaped capture do not depend on the string. Severity: low to medium.

F6. **The wake loop is open.** We confirm only that the wake text left the composer. hcom (cursor advance within 10 s), Cyclops (token echoed in `UserPromptSubmit.prompt`, p50 30 ms) and cross-agent-teams-mcp (observed `get_inbox` call) confirm that the model received it. Our case: hooks untrusted after a re-setup, but provenance still `cooperative_top_level`. The session then gets the content-free wake with no delivery behind it, and it is retried 30 s → 300 s while the item stays unread (ours §3.6). That this is detected is unknown: `wake_text_evidence_current` (`src/store/wake.rs:658`) was not traced. Severity: medium.

F7. **We wake after a human interrupt, by accident of the idle signal.** INFERRED (ours §3.6). Codex's queue encodes the opposite policy (`service.rs:472-483`), and hcom's report flags its own identical behaviour as undoing the human's stop (hcom §3.5). We have no `Interrupt` hook, so we cannot tell a Ctrl-C idle from a completed idle. Severity: low to medium. It is a policy gap, not a bug.

F8. **Idle and busy come only from Herdr's screen manifest.** VERIFIED (ours §3.3). Five alternatives use a native signal, and hcom and Cyclops use hook-set status (`Stop`/`Interrupt`/`UserPromptSubmit`). We already install `UserPromptSubmit`, so a turn-start edge is available to us at zero cost and we do not consume it. A `Stop` hook would give the end edge. Severity: low (Herdr's signal works in the release e2e).

F9. **The pane-identity leak under the shared daemon is mitigated only opt-in.** VERIFIED (ours §3.6, `src/harness/codex/pane_env.rs`). cross-agent-teams-mcp documents production pane theft from exactly this cause (`CHANGELOG.md:20-43`). Its nonce and argv-marker fixes, and agmsg's per-seat app-server, show the risk is real on current Codex. Severity: medium.

F10. **Primer and docs drift.** VERIFIED: primer §10 says Codex hooks carry "a server-side attention digest, not message bodies", but ours §2.2 and `src/cli/hook.rs:4052-4054` show that bodies are delivered. `integrations/codex/README.md` is stale as ours §1 lists. Severity: low.

Not flaws, confirmed by contrast:
- Content-free wake (R18). agentchattr PR #113 and cross-agent-teams-mcp's per-send hints show the cost of the alternative.
- Thresholds (D12).
- The 4 KiB cap under Codex's 2 500-token spill. hcom, agent-talk and agmsg ACK content the model saw only as a spilled preview.
- Developer-role delivery instead of user-role. ocs, agmsg and Cotal each call their user-role path a compromise.
- Never pressing Enter twice. CAO's double Enter and agentchattr's blind paste are the counterexamples.

---

## 5. Better integration ways, ranked by value / cost

1. **Widen tool-boundary delivery to every tool.** Add `PostToolUse` with matcher `*`, or widen the `PreToolUse` matcher, and let `parse_shape` accept any `tool_name`. This is adapter-only: `DECLARATION.owned_hooks`, `CONTRACT`, and `src/harness/codex.rs:1189-1192`. Value high, cost small, one re-trust. It closes F1. Evidence: hcom, agmsg; `hook_names.rs`.
2. **Fix the bare-wake rule mismatch (F2).** Either give the Codex hook block a header the skill names, or change the skill rule to "no herdr-threads delivery block". The second is cheaper. Value medium, cost trivial.
3. **Emit static instruction once per session, and use the R17 frame for tool-boundary and turn-start deliveries.** This is core work in `compose_context_limited`: lifecycle check-ins keep the instruction, tool-boundary ones drop it. It saves about 440 tokens per delivery and doubles body room. Value high, cost medium (touches the neutral core, all hook harnesses). Closes F3 and helps F2.
4. **Close the wake loop with existing hooks.** After a wake submit, require the binding's `UserPromptSubmit` check-in (or a cursor advance) within N seconds. Otherwise record `wake=unreceived` and stop the retry ladder rather than retyping. This needs no nonce in model text. Value medium, cost medium. Closes F6. Evidence: hcom `src/delivery.rs:2433-2475`, Cyclops `ack.rs:655-700`.
5. **`Stop` hook as a priority-gated end-of-turn continuation.** Block only when an item with p ≤ T is pending, `stop_hook_active` is false, and the turn was not served in-turn. Make the reason content-free (for example "herdr-threads: pending"), so the `UserPromptSubmit`-equivalent delivery stays in developer context. Whether `Stop` continuation runs `UserPromptSubmit` is unknown and must be probed. If it does not run, carry a bounded R17 frame in `reason` and accept user role. Value medium, cost medium (new neutral `DeliveryPolicy` hook with a none default, per AGENTS.md "New harness behaviour"). Closes F4.
6. **Consume our own hooks as idle/busy edges.** `UserPromptSubmit` gives turn start, and `Stop` plus `Interrupt` give turn end and interrupt. Feed them as a second signal beside Herdr `agent_status`. Refuse a wake while a turn-start has no matching end, and apply an explicit post-interrupt wake policy (for example, only p0 that arrived after the interrupt). Value medium, cost medium. Closes F7 and F8. Evidence: hcom `src/hooks/codex.rs:843-967`, Cyclops `codex.toml:35-53`.
7. **Placeholder-independent composer read.** Accept the known placeholder set (`Ask Codex to do anything`, `Ask a follow-up question`) as `Empty`, or use the dim attribute if Herdr's detection capture keeps SGR (unknown). Value low to medium, cost small. Closes F5.
8. **`hooks/list` doctor row.** Spawn `codex app-server --listen stdio://` under the user's `CODEX_HOME`, call `initialize` then `hooks/list`, and report each owned hook's load errors and `HookTrustStatus`. This is read-only and costs no model turn. Value medium, cost small to medium. Turns the silent untrusted-hook case into a diagnosis and helps F6. Evidence: ruflo `tests/plugins/codex-mod-hooks.test.mjs:110-141`, hcom `src/hooks/codex.rs:1362-1450`. Our setup already measured `hooks/list` historically (`src/harness/codex/setup.rs:1413`).
9. **`codex queue` as an alternative `IdleWake` for daemon-attached Codex.** Server-side idle gating with no composer read. Enqueue at most one content-free item per idle edge (ocs #41: queued items each replay as a turn). Serialize writes per `CODEX_HOME` (agmsg `codex-queue.mjs:542-551`). Do not count queue acceptance as delivery. Value medium to high for users who keep the shared daemon, cost high: a new `IdleWake` variant, thread-id mapping, and the unresolved `--no-daemon` reachability experiment (primer §3). It also inherits the interrupt refusal and the `/side` exclusion.
10. **Managed launch with a private app-server and a `--remote` TUI** (Cotal, agent-peers-mcp). This gives `turn/start`, `turn/steer`, `thread/status/changed` and `thread/inject_items` with no env leak. Value high for launched sessions. Cost high: experimental API, `historyMode` breakage history, launcher footprint, phantom spinner, and `--ws-auth capability-token` mandatory (unlike agmsg and agent-peers-mcp). Defer until 1 to 6 land.
11. **`-c shell_environment_policy.set.HERDR_PANE_ID=…` in `herdr-threads launch` for Codex.** Gives a pane marker that survives the shared daemon (cross-agent-teams-mcp §7.5, agmsg). Value medium (F9) for launched sessions only. Cost small, but it must be verified on 0.163 and given a provenance value under TRUST-POLICY.md.

Not recommended:
- MCP push, because Codex logs and drops notifications (primer §4).
- Per-send wakes.
- Blocking the sender on the recipient's receipt (cross-agent-teams-mcp §3.4).
- Pre-trusting hooks in the user's global `hooks.json`.

---

## 6. Primer surfaces nobody uses yet

From primer §0 to §9, checked against all thirteen designs (ours plus twelve):

| Surface | Primer | Used by | Why it may matter to us |
|---|---|---|---|
| Async command hooks (`async: true`) | §1.6 | nobody (agent-talk's doc says "parsed but not implemented", true only of 0.147) | mid-turn delivery off the tool's critical path; never wakes |
| `thread/inject_items` for delivery | §2.2 | nobody (Cotal uses it only to materialise a rollout) | silent p1 "next turn" delivery as a raw history append, no hook, no turn |
| `turn/start.additionalContext` / `turn/steer.additionalContext` | §2.2 | nobody (Cotal, agent-talk, cross-agent-teams-mcp and agent-peers-mcp all send user text) | an app-server wake that carries content as context rather than as the user speaking |
| `notify` `agent-turn-complete` as an idle edge | §1.7 | nobody (mcp_agent_mail dropped it because its output cannot reach the model) | free, per-thread turn-complete edge with no app-server |
| `Interrupt` hook | §1.2 | hcom only | distinguishes a Ctrl-C idle from a completed idle (F7) |
| `SubagentStop` `decision:block` | §1.4 | nobody | continue a Codex subagent that has pending child-relevant mail (we only disconnect) |
| `additionalContextLimit` per handler | §1.4 | nobody | lets one hook raise the 2 500-token spill threshold explicitly instead of staying under it |
| `mcp_tool` hook handler type | §1.1 | nobody | a hook that calls an MCP tool directly; no process spawn |
| `thread/queue/start` (explicit dispatch after interrupt) | §3 | nobody (agmsgd and ocs both get stuck after an interrupt) | completes the queue path if we adopt it |
| `codex debug prompt-input` | §6 | nobody | test the exact model-visible hook context without spending a turn (useful for F2 and F3 regressions) |
| `tui.notifications` OSC 9 / BEL | §7 | nobody (hcom notes Codex falls back to BEL under its PTY) | terminal-level turn-complete edge Herdr could consume |
| `sleep` tool early-return on new input | §5 | nobody deliberately | pairs with `turn/steer` for waiting agents; we tell models to end the turn instead |
| `PermissionRequest` hook as an approval signal | §1.2 | Cyclops only (silent state); hcom rejects it deliberately (fires before a reviewer is chosen) | approval-state input beside the screen `ApprovalOrQuestion` refusal |

---

## Verification

Re-checked for this file:
- `src/harness/codex.rs:272-309` (six hooks, `PreToolUse ^Bash$`), `:1393-1399` (wake text).
- `src/cli/hook.rs:74` (`MAX_CONTEXT` 4096) and `:684-729` (`base_instruction` on every non-empty composition).
- `src/harness/mod.rs:178` (`TOP_LEVEL_INSTRUCTION`).
- `src/cli/hook_inbox.rs:38-39` (`INBOX_HEADER`) and `src/delivery/frame.rs:10` (R17 header).
- `integrations/skill/SKILL.md:24` (bare-wake rule).
- `src/host/native.rs:55-71` (750 ms, 2 s, 60 s), `src/harness/codex/composer.rs:8-9,44` (single placeholder).
- `src/harness/codex/setup.rs:1413` (historical `hooks/list` measurements).
- Codex 0.163:
  - `codex-rs/tui/src/chatwidget.rs:2079-2080` and `chatwidget/constructor.rs:42-43` (fixed normal and side placeholders);
  - `bottom_pane/chat_composer.rs:1659` (sub-agent placeholder);
  - `core/src/tools/hook_names.rs:28-56` (`apply_patch`/`spawn_agent`/`Bash` hook names and aliases);
  - `app-server/src/request_processors/turn_processor.rs:661-695` (`turn/start` is start-or-steer);
  - `ext/queue/src/service.rs:472-483` (no dispatch after `Interrupted`);
  - `core/src/session/turn.rs:455-476` (steered pending input runs `UserPromptSubmit`).

Corrections to the per-alternative reports, found while checking:
- ruflo §6.5 says `Write|Edit|MultiEdit` never matches `apply_patch`. Codex 0.163 does accept `Write` and `Edit` as matcher aliases for `apply_patch` (`hook_names.rs:28-38`). `MultiEdit` is not an alias.
- cross-agent-teams-mcp §7.7 says the placeholder is randomised. On 0.163 it is the fixed constant `PLACEHOLDER` (`constructor.rs:42`).

Not verified here:
- the alternatives' own source, which this file takes from their reports;
- whether `Stop` continuation runs `UserPromptSubmit`;
- whether Herdr's detection capture keeps SGR attributes;
- `wake_text_evidence_current` behaviour on re-trust.
