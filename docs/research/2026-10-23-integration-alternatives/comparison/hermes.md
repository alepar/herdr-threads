# Hermes Agent: herdr-threads compared with 12 alternatives

Scope: how each tool puts a peer message into a Hermes Agent model's context and how it wakes an idle Hermes session, compared with herdr-threads at `main` `2de3da1c4`.

Inputs:
- `../ours/hermes.md`, our own implementation;
- `../primers/hermes.md`, the Hermes surfaces, from a read-only clone of https://github.com/NousResearch/hermes-agent at `f3b5b980` (latest release v0.22.5);
- `../alternatives/<slug>/hermes.md` for agent-talk, cotal, agmsg, ruflo, mcp-agent-mail, hcom, agentchattr, cross-agent-teams-mcp-jtianling (xats), agent-peers-mcp-mikez93, cyclops, open-cross-session-ocs and cli-agent-orchestrator-cao-awslabs (CAO).

Tags: **V** = VERIFIED (read in code or docs; for this report I re-read the load-bearing ones, listed under "Verification"), **I** = INFERRED, **unknown** = not found. A `hermes:` path is relative to the Hermes clone root; an unprefixed path is in this repository. Claims about an alternative cite that alternative's report, which cites the clone line.

## TL;DR

1. **Of the 13 tools, three wake a stock, current Hermes through the host's own queue, with no keystrokes.**
   - hcom: `ctx.inject_message` for messaging-gateway chats.
   - Cotal: a gateway platform adapter, but only in its own headless gateway, and only on Hermes 0.18 to 0.21.
   - ocs: gateway JSON-RPC `prompt.submit {queued: true}`, but only for Desktop and `hermes serve` sessions.

   None of the three reaches a classic-CLI Hermes in a terminal pane on v0.22.1 or later. That is the case herdr-threads targets, so on current Hermes nobody has a working wake for it. Our own wake is dead code: Hermes has no `ComposerPolicy`, so `submit_prompt_mode` always refuses (**V**, `src/host/native.rs:1876-1878`, `:2111-2113`).
2. **Only hcom delivers mid-turn**, through `transform_tool_result` (**V**, hcom report, `src/hermes_plugin/__init__.py:931-936`). We deliver only at `pre_llm_call`.
3. **Only hcom acknowledges on evidence of admission.** It acks only after it finds the exact framed text in the accepted history or in `state.db`. We complete lazy rows and advance the attention mark *before* the Python bridge decides whether to return the text (**V**, `src/cli/hook.rs:4475-4502`, `integrations/hermes/__init__.py:567-596`). That leaves a loss window that grows under the 0.22.1+ shared gateway.
4. **Every tool that takes identity from the process env or the process tree is probably broken on Hermes 0.22.1 and later.** That list is herdr-threads, hcom (local agents), Cyclops, CAO, agmsg, xats and agent-peers legacy (**I**, high confidence). The tools that survive carry identity explicitly:
   - per-command paths or arguments (agent-talk, ruflo);
   - host-stamped per-call metadata (agent-peers v2, patched host only);
   - the gateway session context (Cotal);
   - two session env signals (ocs).
5. **Several existing surfaces are unused by everyone.** No tool uses `terminal.client_env_passthrough` plus `bound_client_env()`. No tool uses `session.steer`, `display_kind: "hidden"`, `register_system_prompt_section`, `on_session_end` as an idle edge, or the `notify_on_complete` self-wake. Section 6 lists them.

---

## 1. Mechanism table (ours first)

"0.22.1+" means the gateway era, in which every CLI, TUI and Desktop session runs in one gateway process (primer §1).

| Tool | Hermes support | Injection surface | When it injects | Wake | Busy handling | Draft and modal safety | Identity key | Works on 0.22.1+? |
|---|---|---|---|---|---|---|---|---|
| **herdr-threads** | Experimental owned Python plugin; live-measured only on 0.21.5 | `pre_llm_call` → `{"context"}` ≤ 4,096 B, appended to the user message (**V**) | Turn start only; `post_tool_call` is an observer (**V**) | Declared `IdleWake::Native` keystroke (`MARKER`), **always refused**: no composer policy (**V**) | n/a (no wake ever fires) | Moot today. If a wake were typed, nothing guards approval pickers (**I**) | `HERDR_PANE_ID` from the hook child's inherited env, plus process ancestry (**V**) | **Probably broken** (**I**): env missing, or inherited from the pane that started the gateway |
| hcom | Native, first-class (0.7.30) | `pre_llm_call` context, **`transform_tool_result` mid-turn**, idle hand-off as a user turn (**V**) | Turn start, after every tool call while pending, and idle | Body-as-turn through `cli._pending_input` / `inject_tui_message` (local), `ctx.inject_message` (bridges) (**V**) | Host queue; re-checks busy under a lock before hand-off (**V**) | No keystrokes (**V**) | `HCOM_PROCESS_ID` env plus plugin PID claim (**V**) | Local agents **probably broken**: `_cli_ref` is never set and PLG-7 moved plugins out of the client (**V**, `hermes:hermes_cli/plugins.py:1127`, `cli_tui_mixin.py:1834`). Bridges likely work (**I**) |
| Cotal | Alpha; version gate `>=0.18,<0.22` | Gateway platform `MessageEvent` → `handle_message`; hooks inject nothing (**V**) | On arrival, idle or busy | The message *is* a turn in its own headless gateway session per peer (**V**) | Hermes default `interrupt`: a follow-up aborts the running turn (**V** on main) | No TUI, no composer (**V**) | Launch material plus gateway session context (**V**) | **Refused at launch** on 0.22.x (**V**) |
| ocs | Native for Desktop and `hermes serve` only | Gateway JSON-RPC `prompt.submit {session_id, text, queued:true}` (**V**) | At send time | The same call; an idle session starts a turn (**V**) | `queued:true` forces queue mode; Hermes merges text-only queued prompts (**V**) | No composer contact (**V**) | `HERMES_SESSION_ID` **and** `HERMES_SESSION_SOURCE`, Hermes ranked last (**V**) | Terminal sessions **unsupported**: the gateway role publishes no token (**V**) |
| agent-peers (legacy) | stdio MCP server | `[PEER INBOX]` block prepended to its own MCP tool results (**V**) | Only when the model calls one of its tools | None; a bounded 60 s `wait_for_peer_messages` inside a turn (**V**) | n/a | n/a | One MCP child per profile (**I**) | Pull works; identity is probably one per profile (**I**) |
| agent-peers (v2) | Patched private host only | Same as legacy | Same as legacy | Host RPC `session.wake_admit {queued, hidden}`, bodyless notice, gated on a fresh lifecycle snapshot (**V**) | Host-decided (**V**) | No keystrokes (**V**) | Host-stamped `_meta` per call (**V**) | **No**: upstream Hermes lacks the RPCs (**V**) |
| Cyclops | Unverified manifest, written before the gateway shift | Doorbell line pasted into the composer; the body is pulled by `inbox claim` (**V**) | As soon as the gate admits | tmux bracketed paste + Enter (**V**) | None: interrupts under the default mode (**I**) | **None**: the Enter can approve a dangerous-command overlay (**I**, strong) | Kernel process ancestry (**V**) | Identity **probably broken** (**I**) |
| CAO | Native provider (tmux or Herdr) | The whole message pasted as a user turn plus an 83 B suffix (**V**) | Only when the pane reads IDLE/COMPLETED | Paste + one Enter (**V**) | Screen-regex or Herdr-status gate; an unknown screen counts as busy (**V**) | Refuses while a Hermes picker is up; no draft check (**V**) | `CAO_TERMINAL_ID` through the MCP env (**V**) | MCP identity **probably broken** (**I**) |
| agmsg | Beta; delivery `off` | Skill → `inbox.sh` terminal tool result (**V**) | Only when `/agmsg` runs | Generic `poke.sh` through `herdr agent prompt`; desktop-app PTY kickoff (**V**) | None (**V**) | **None** for Hermes: no `input_prompt_marker` (**V**) | `(pwd, type)` and a placement record taken from `HERDR_PANE_ID` (**V**) | Poke **probably broken** or misrouted (**I**) |
| xats | `custom` type only | Chinese wake hint pasted; body through MCP `get_inbox` (**V**) | After a 2 s quiet-screen guard | tmux paste + Enter, default socket only (**V**) | Screen-quiet guard only (**V**) | None (**I**) | Per MCP session; the binding recipes are env-based (**V**) | Binding **probably broken**; no wake under Herdr (**V**: default tmux socket) |
| agentchattr | None shipped | MCP `chat_read` pulled after a tmux wake prompt (**V**) | After the wake | tmux paste + Enter with no gate (**V**) | None: interrupts (**I**) | None (**I**) | Per-run bearer token that no Hermes config path can carry (**I**) | Authenticated pane path **does not work**; headless API path only (**I**) |
| mcp-agent-mail | None | MCP `fetch_inbox` pull (**V**) | Model-initiated | None (**V**) | n/a | n/a | Model passes `agent_name` and a token in every call (**V**) | Pull works (**I**) |
| agent-talk | None | Skill → `retalk receive` terminal tool result (**V**) | Model-initiated | None (**V**) | n/a | n/a | Explicit `--dir` on every command (**V**) | Pull works (**I**) |
| ruflo | None | MCP `team_inbox` drain (**V**) | Model-initiated | None (**V**) | n/a | n/a | Tool arguments; the mailbox is rooted at the MCP cwd (**V**) | Pull works; the mailbox root may be wrong (**I**) |

---

## 2. Differences from our approach

Each row is one way an alternative differs from herdr-threads. Pros and cons are scored on latency, reliability, token cost, draft safety, setup footprint, harness-version fragility, cross-machine reach and trust. "Theirs" means the alternative's approach and "ours" means today's code.

### D1. Wake through the host queue instead of keystrokes (hcom, Cotal, ocs, agent-peers v2)

- **Theirs.**
  - `ctx.inject_message(session_key=…)`: the plugin already runs in the gateway, and the call needs `plugins.entries.<id>.allow_gateway_injection: true` (**V**, `hermes:hermes_cli/plugins_injection.py:36-40`).
  - Gateway `prompt.submit {queued: true}` (**V**, `hermes:tui_gateway/contracts/prompt_voice.py:35`; `session_auto_continue.py:406` `mode = "queue" if queued else _load_busy_input_mode()`).
  - A platform `MessageEvent` (Cotal).
- **Ours.** A Herdr keystroke into an empty composer, which today never fires.
- **Pros of theirs:**
  - *Draft safety:* complete. The text goes to a queue, never the prompt buffer.
  - *Busy safety:* the host queues while busy. `queued:true` also avoids Hermes' default `busy_input_mode: interrupt`, which a mistimed keystroke would trigger.
  - *Reliability:* no screen scraping, no composer provider, and no Herdr agent recognition, which a bare Hermes on macOS lacks (`ht-77b`).
  - *Latency:* sub-second on loopback (**I**).
  - Durable on accept in queue mode (**V**, ocs report, `session_auto_continue.py:436-442`).
  - Pickers are safe: injected text "cannot … approve tools, or resolve pending confirmation and clarification prompts" (**V**, `hermes:website/docs/user-guide/features/plugins.md`, gateway-mode bullet list).
- **Cons of theirs:**
  - *Visibility.* "When that is a classic CLI session, the turn and its reply are in the session's transcript, but the attached classic CLI does not display them" (**V**, plugins.md, keyless-injection paragraph). The doc says this about keyless injection; for keyed injection into a classic-CLI session it is unknown. A woken agent that acts where the human cannot see it in their pane breaks the Herdr model of a human-supervised pane.
  - *Key mapping.* `session_key` "is the stable routing key, not the CLI session ID" (**V**, plugins.md), and how it maps to the hook's `session_id` is unknown.
  - *Version fragility.* `prompt.submit` over the control-socket ticket is a Python-internal protocol. The ticket path speaks the canonical contract (`started | queued | unknown | terminal`), not the legacy `streaming | queued` reply that ocs maps (**V**, ocs report §6.2).
  - *Trust.* The injected row is user-role (D3).
  - *Setup.* `allow_gateway_injection` is one more config grant.
  - *Cross-machine:* none. Every path is same-host, which matches ours.

### D2. Mid-turn delivery through `transform_tool_result` (hcom)

- **Theirs.** After every tool call of the bound session, and only when a doorbell has set `pending`, the plugin returns `f"{result}\n\n{batch}"` (**V**). Hermes keeps the first `str` result among all plugins' returns (**V**, `hermes:model_tools.py:842-845`).
- **Ours.** None: Hermes items wait for the next `pre_llm_call`. Delivery-contract rule 3 is met only at turn start.
- **Pros:**
  - *Latency:* a busy agent gets mail at its next tool boundary.
  - It serves rule 4 (never nudge a busy session) for free.
  - *Token cost:* zero on quiet tool calls when gated on a local pending flag.
- **Cons:**
  - *Reliability.* First-string-wins: Hermes bundles a `security-guidance` plugin that also registers this hook (**V** for existence, hcom report). If that plugin's string wins, the batch is silently dropped. That is survivable only with evidence-based acks (D4).
  - *Trust.* Peer text appended to a tool result looks like tool output. Our `INBOX_HEADER` framing would have to travel with it.
  - *Cost.* For us, each tool call is a process spawn unless a long-lived listener sets the flag.
  - A long turn of pure generation still gets nothing.

### D3. The body is the wake (hcom idle hand-off, Cotal, ocs, CAO, agmsg poke) versus a content-free wake (agent-peers v2, Cyclops, our R18 design)

- **Theirs.** The message text itself becomes a user-role turn.
- **Ours, as designed.** Wake text plus hook delivery. Today the declared text is `MARKER` rather than `WAKE_UP` (**V**, `src/notification/policy.rs:14,17`; no Hermes `DeliveryPolicy`).
- **Pros of body-as-wake:**
  - one model step instead of two;
  - no dependence on the woken turn's `pre_llm_call` succeeding within 1.2 s.
- **Cons of body-as-wake:**
  - *Trust:* peer text sits in a user-role row with user authority. ocs flags this itself, and Hermes renders it as a user bubble.
  - *Token cost:* no threshold, so every message is a turn.
  - It conflicts with AGENTS.md (c): wake prompts carry no content.
- **Verdict.** Keep the content-free wake. agent-peers v2's notice ("do not infer message content from this notice") and Cyclops's doorbell both confirm the shape. The ideal is a content-free `inject_message`/`prompt.submit` followed by our `pre_llm_call` injection, and `display_kind: "hidden"` could keep the wake row out of the human's view (section 6).

### D4. Acknowledge on evidence of admission, not on emission (hcom, agent-peers)

- **Theirs.**
  - hcom moves forward through `handed_off → offered → admitted → acked`. It acks only after it finds the exact framed body in `post_llm_call`'s `conversation_history` (`api_content`) or in `state.db`, and releases and redelivers when Hermes dropped the batch (**V**, hcom report).
  - agent-peers treats the next tool call after a promoted response as proof (an arrival-causality barrier).
- **Ours.** The Rust child emits, then commits the lazy rows, then commits the attention mark, then exits (**V**, `src/cli/hook.rs:4475-4502`). The Python bridge can still discard the output afterwards, in these cases (**V**, `integrations/hermes/__init__.py:570-596`):
  - a sequence change;
  - the deadline passes;
  - the reader snapshot goes stale;
  - strict re-validation fails;
  - the final `entry_lock` re-acquire fails.
- **Pros of theirs:**
  - *Reliability:* "seen by the model" semantics;
  - it survives persist-disabled turns, interrupted turns and first-string-wins losses.
- **Cons of theirs:**
  - complexity: hcom's plugin is 1,532 lines;
  - `state.db` scans that are linear in transcript size;
  - a dependency on Hermes' private `state.db` schema, which adds version fragility.
- **A cheaper middle ground for us.** Have the bridge acknowledge back. Rust would complete the lazy rows only when the bridge reports that it returned the context to Hermes. One option is a second short `hook hermes --commit <event_id>` call, or the next callback's envelope could carry a "returned" bit for the previous event_id.

### D5. Where identity comes from

- **Theirs**, roughly from best to worst:
  - host-stamped per-call `_meta` (agent-peers v2);
  - the gateway session context (`gateway.session_context.get_session_env`, Cotal);
  - two env signals per command, `HERMES_SESSION_ID` plus `HERMES_SESSION_SOURCE` (ocs);
  - an explicit identity path or argument on every call (agent-talk, ruflo);
  - the process env or the kernel process tree (hcom local, Cyclops, CAO, agmsg, xats, ours).
- **Ours.** `HERDR_PANE_ID` from the hook child's inherited `os.environ` (**V**, `src/cli/hook.rs:255`; `Popen` is called with no `env=`, `integrations/hermes/__init__.py:627-628`).
- **What the code says about the failure mode:**
  - An auto-spawned gateway strips only a fixed list (`DAEMON_STRIPPED_ENV`), and `HERDR_PANE_ID` is not on it (**V**, `hermes:hermes_cli/gateway_runtime_start.py:18-32`). The pane that first starts the gateway therefore probably owns every later Hermes session (**I**).
  - `client_env` would let `HERDR_PANE_ID` travel if the operator listed it. `_travels` blocks only secret-looking names and the `HERMES_` prefix (**V**, `hermes:agent/session_client_env.py:48-54`; `tools/code_execution_env.py:29-35`). The value is bound to a `ContextVar` "for this session's turns only" (**V**, `hermes:gateway/session_policy.py:152-173`), so it never reaches `os.environ`.
  - Whether a plugin hook callback runs inside that scope, so that `bound_client_env()` returns the pane's value, is **unknown**. It is likely, because the hooks run within the turn.
- **Pros of explicit or host-stamped identity:**
  - correct under the gateway;
  - no dependence on how the gateway was started.
- **Cons:**
  - Explicit model-passed identity (agent-talk, ruflo, mcp-agent-mail) is honour-based and gets lost on compression.
  - Under TRUST-POLICY A3 a session-keyed seat needs its own provenance value.

### D6. Static guidance once per session versus on every presenting turn (Cotal, hcom)

- **Theirs.**
  - Cotal puts it in `SOUL.md` and a 195 B `platform_hint`, both system-prompt bytes.
  - hcom injects a bootstrap of about 700 tokens once per bound session.
  - agent-peers is the counter-example: a 483 B header on every delivery.
- **Ours.** About 2.4 KB of instructions, the subagent paragraph, routing JSON and ready commands, re-sent on every turn that presents anything (**V**, run-3 `08` vs `13`).
- **Pros of theirs:**
  - *Token cost:* roughly 600 tokens saved per presenting turn;
  - it complies with AGENTS.md rule (a).
- **Cons:**
  - A static section frozen at session start cannot carry per-seat routing that changes. Keep the seat id and ready commands per turn, but move the explanations out.

### D7. Compact per-message framing (Cotal, hcom, ocs) versus a JSON check-in envelope

- **Theirs.**
  - Cotal: `[dm from bob / reviewer] body`, which neutralizes line breaks and brackets.
  - hcom: `<hcom>[intent #id] a -> b: …</hcom>`.
  - ocs: a header line, a bounded body, `Reply:` and `Thread:` lines, and forged-command neutralization.
- **Ours.** `INBOX_HEADER` plus JSON-escaped rows inside a check-in envelope, not the R17 frame that the Claude mod and agy use (**I**, ours §2.4).
- **Pros:**
  - fewer bytes;
  - easier for the model to parse.
- **Cons:**
  - None of the three gives an ACK id in the frame, except hcom's `#id`.
  - Our JSON escaping is the stronger injection boundary, so a move to R17 should keep the body-indent neutralization of `src/delivery/frame.rs`.

### D8. Body bounds

- **Theirs.** ocs inlines up to 4,096 B, else a 512 B head plus `… (N bytes total; full text: ocs read)`, with the note capped at 5,120 B (**V**). Most others are unbounded: hcom allows 50 × 1 MiB, agent-peers 500 rows, CAO puts the body in a URL query string, xats has no schema maximum.
- **Ours.** 4,096 B of context, whole items only, and a `…` continuation line (**V**, `src/cli/hook.rs:74`, `hook_inbox.rs:26-33`).
- **Assessment.** Ours is as good as the best. ocs's head plus pointer is the same idea as our 8 KiB truncation with a `body <id>` continuation elsewhere. No change needed.

### D9. Busy-path semantics

- **Theirs.**
  - ocs sets `queued:true` and says "mandatory" (**V**).
  - Cotal leaves `busy_input_mode` at `interrupt`, so a peer's follow-up aborts the running turn and a busy-ack leaks to the mesh (**V** on main).
  - The keystroke tools (Cyclops, agentchattr, agmsg, xats) interrupt the turn when they misjudge idle (**I**).
- **Ours.** Not applicable today. Any future RPC or inject path must pin queue semantics, or a nudge becomes an interrupt and violates rule 4.

### D10. Picker and modal guard

- **Theirs.** CAO declares `blocks_orchestrated_input_while_waiting_user_answer` and refuses pastes and bare Enters while an approval or clarify picker is up (**V**). Cyclops has no such rule, and its Enter selects the preselected "once" choice on Hermes' dangerous-command overlay (**I**, strong; Cyclops report limitation 1).
- **Ours.** Not applicable while no `ComposerPolicy` exists. If we ever add one, it must hold on `on_human_input_request` until `_resolved` (hook signals, primer §2.3), not only on a composer read.

### D11. Version handling and stale-plugin repair

- **Theirs.**
  - Cotal hard-gates `<0.22` and refuses today's Hermes. That is the anti-pattern AGENTS.md forbids.
  - hcom has no version gate. Its loaded plugin reports `SOURCE_HASH`; when the hash is stale, hcom rewrites the files and asks the gateway to `reload_gateway_plugins` over Hermes' control socket (**V**).
  - hcom refuses `plugins.isolation: host` explicitly, with an ERROR log (**V**).
- **Ours.**
  - No version gate, which is good.
  - `setup-status` reports a stale bridge, but the user must re-run setup.
  - No check for `plugins.isolation: host`. Our code never mentions it (**V**, grep), and the effect on our bridge in host isolation is unknown.
- **Pros of hcom's approach:**
  - *Reliability after upgrades:* the gateway is long-lived and keeps the old code loaded until it restarts (**V**, hcom report, uninstall note).
- **Cons:**
  - It depends on a private control-socket verb.

### D12. Wake bookkeeping

- **Theirs.**
  - agent-peers v2 persists an `uncertain` attempt before the RPC, reconciles by `attempt_id` after a crash, keeps receipts monotonic, and backs off 60 s, 5 min and 30 min, then stops until new mail arrives (**V**).
  - ocs sets `written` before `send`, so failures split into `unavailable`, `failed` and `unknown-outcome`, and it never resends (**V**).
  - Cyclops uses no timer, only events (**V**).
- **Ours.** A 30/60/120/300 s ladder that a Hermes target refuses forever (**I**, ours §3.2). It costs Herdr reads, not tokens.
- **Pros of theirs:**
  - honest outcomes;
  - bounded retries.
- **Con for us.** Ours wastes nothing model-side, but it hides a permanently dead capability. AGENTS.md says to degrade the broken capability; it does not say to retry forever.

### D13. Setup footprint

| Tool | What it writes into Hermes |
|---|---|
| ocs | nothing |
| agent-talk, mcp-agent-mail, ruflo, xats, CAO | a manual skill or MCP entry, or nothing |
| agmsg | `~/.hermes/skills/<cmd>/SKILL.md`; ignores `HERMES_HOME` |
| Cyclops | a fenced text merge into `~/.hermes/config.yaml`; default profile only; a YAML indent risk |
| hcom | a per-profile plugin, auto-enabled through `hermes plugins enable`; the injection grant written only on opt-in |
| **ours** | a per-profile owned plugin (three files, mode 0600); the user runs `plugins enable` by hand |

- Ours is in line with hcom, which is the other tool that actually integrates, but adds one manual step.
- An injection-based wake adds an `allow_gateway_injection` grant. hcom's pattern for it: write it on opt-in, record that we wrote it, and never remove a grant the user set.

### D14. Model-initiated pull (agent-talk, agmsg, mcp-agent-mail, ruflo, agentchattr, xats, agent-peers legacy)

- **Pros:**
  - It works on any Hermes version.
  - It needs no plugin.
  - Peer text stays in tool results, which is the safer channel.
- **Cons:**
  - It has no wake.
  - It depends on the model remembering to check. MCP server `instructions` are probably never surfaced by Hermes (**I**, agentchattr and agent-peers reports), so "check every turn" rules are lost.
  - It carries a standing tool-schema tax on every request: about 850 tokens for agentchattr, about 9-11k for mcp-agent-mail, about 5-10k for xats, and tens of thousands for ruflo (all **I**, from the reports).
- **Assessment.** Our hook push is strictly better while it works. The pull path matters only as a fallback when the bridge is inert. That fallback already exists through the model's `herdr-threads inbox`, which the gateway env does not break as long as the CLI identifies the seat (D5).

### D15. Per-session versus per-plugin-instance callback serialization

- **Theirs.** hcom routes every callback by `session_id` to a participant (`self.route(session_id)`) and fences each participant with its own `gen` counter. Cotal keys its sessions per chat.
- **Ours.**
  - One `Bridge` per plugin instance.
  - A non-blocking `entry_lock`/`child_lock` drops concurrent callbacks (**V**, `__init__.py:458-459`).
  - One global `self.sequence` invalidates any in-flight callback when any other callback enters (**V**, `:461`, `:570`).
  - In the 0.22.1+ gateway one plugin instance serves every session of the profile (primer §1). Two panes' turns therefore starve each other, and each lost callback can also lose delivered rows (D4). (**I**)
- **Fix.** Key the sequence, the locks and the child slot per `session_id`.

---

## 3. Flaws in our approach that the others expose

| # | Flaw | Evidence | Exposed by | Severity |
|---|---|---|---|---|
| F1 | **The idle wake for Hermes never fires, and the docs say it does.** Hermes declares no `ComposerPolicy`; `read_composer_text` returns `Unreadable` and `submit_prompt_mode` refuses anything but `Empty`. The README and `launch_notes` still call a launched Hermes "wakeable". | `src/host/native.rs:1876-1878`, `:2111-2113`; no `composer_policy` in `src/harness/hermes*` (grep); `integrations/hermes/README.md:44-46`; `src/harness/hermes.rs:36-39` (all **V**) | hcom, ocs and Cotal show that a host-queue wake exists and is draft-safe | high |
| F2 | **Probable misattribution or refusal on Hermes ≥ 0.22.1.** The hook child inherits the gateway's env. An auto-spawned gateway keeps the first pane's `HERDR_PANE_ID`, which is not in `DAEMON_STRIPPED_ENV`, and binds every session to that pane's seat. An operator-started gateway lacks the variable, so every hook refuses quietly. Model-issued CLI calls are gateway children, so `caller_env` ancestry finds no pane. | `integrations/hermes/__init__.py:627-628` (**V**); `src/cli/hook.rs:255` (**V**); `hermes:hermes_cli/gateway_runtime_start.py:18-32` (**V**); the effect is **I**, not observed live | hcom and Cyclops hit the same class of bug; ocs and agent-peers v2 avoid it with session-keyed identity | high |
| F3 | **Rows are completed before the bridge returns them.** Five bridge-side discard paths run after `lazy.commit()` and `attention.commit()`. | `src/cli/hook.rs:4475-4502`; `integrations/hermes/__init__.py:570-596` (**V**); how often it happens is **I** | hcom's admission-evidence ack; agent-peers' confirm-on-next-call | high on the gateway (F2 and D15 make concurrency normal), medium on classic |
| F4 | **One global sequence and non-blocking locks across all sessions** in a gateway-hosted plugin instance. | `__init__.py:458-461`, `:570` (**V**); the cross-session effect is **I** | hcom's per-session routing and `gen` fencing | medium-high |
| F5 | **No mid-turn delivery** although Hermes offers `transform_tool_result`; delivery-contract rule 3 is met only at turn start. | `src/harness/hermes.rs:968, 1017-1021` (observer decode, per ours §2.2) **V**; `hermes:model_tools.py:831-848` **V** | hcom | medium |
| F6 | **The static instruction block repeats on every presenting turn**, about 2.4 KB, including the subagent paragraph a top-level agent does not need. This breaks token rule (a). | ours §5, run-3 `08` vs `13` (**V**) | Cotal (system prompt), hcom (bootstrap once) | medium |
| F7 | **Wake text is `MARKER`, not the content-free `WAKE_UP`**, although `pre_llm_call` injects the content (R18). | `src/notification/policy.rs:14,17`; no Hermes `DeliveryPolicy` (**V**) | agent-peers v2 bodyless notice, Cyclops doorbell | low (moot until F1 is fixed) |
| F8 | **The bridge fails inert silently** when Hermes module internals move (`NativeProvider` raises). | ours §3.4, `__init__.py:84-102, 770-775` (**V**) | Cotal reports a dead reader as a retryable fatal; hcom's `SOURCE_HASH` sync | medium |
| F9 | **The platform filter drops anything but `cli` / empty.** Ink TUI sessions report `platform="tui"` in at least some gateway paths, so `hermes --tui` panes probably get no context. | `__init__.py:493-494` (**V**); `hermes:tui_gateway/session_lifecycle.py:809` uses `platform="tui"` (**V**); what `pre_llm_call` receives for a TUI turn is **unknown** (`agent.platform`, `hermes:agent/turn_context.py:819`) | hcom handles `cli` and `tui` participant kinds | medium (needs a check) |
| F10 | **No check for `plugins.isolation: host`.** | grep finds nothing in `integrations/hermes/` or `src/harness/hermes*` (**V**) | hcom refuses with an ERROR log | low-medium |
| F11 | **Refused wakes retry forever** with no surfaced "capability absent" state. | `src/notification/policy.rs:49`, `dispatch.rs:129-142` (per ours §3.2) | agent-peers v2 (exhausting backoff); ocs (honest outcome taxonomy) | low |
| F12 | **No live evidence after the 0.21.5 runs, and Hermes is absent from the release wake e2e.** | ours §1; AGENTS.md "Pre-release wake e2e" (**V**) | hcom ships an opt-in real-model lifecycle test (`tests/real_tool_hermes.rs`); nobody has 0.22.x evidence | medium |
| F13 | **`plugin.yaml` `provides_hooks` lists 5 hooks; the bridge registers 8.** | `integrations/hermes/plugin.yaml:6-11` (**V**); whether Hermes enforces the list is **unknown** | none directly | low |

---

## 4. Better integration ways, ranked by value/cost

The value/cost score is the judgement of this report (**I**); every mechanism it relies on is cited.

| Rank | Change | Value | Cost | Notes and dependencies |
|---|---|---|---|---|
| 1 | **Fix pane identity under the gateway (F2).** In the bridge, read `HERDR_PANE_ID` and `HERDR_ENV` from `agent.session_client_env.bound_client_env()` when it is bound, and pass them explicitly to `Popen(env=…)`. `setup hermes` documents or adds `terminal.client_env_passthrough: [HERDR_PANE_ID, HERDR_ENV]` for the profile. Refuse (never fall back to `os.environ`) when the gateway's own `os.environ` carries a `HERDR_PANE_ID` but the session binds none, because that is exactly the misattribution case. | Very high: without it nothing works on current Hermes | Small in the bridge; one config key in setup | Verify live that hook callbacks run inside `client_env_scope` (unknown). Needs a TRUST-POLICY note: the pane claim now arrives through Hermes' client-env relay, which is still client-supplied, so it is the same cooperative claim with a new carrier. Add ocs's two-signal rule (`HERMES_SESSION_ID` plus `HERMES_SESSION_SOURCE`) to `caller_env` for model-issued CLI calls |
| 2 | **Correct the docs now (F1).** Stop calling a launched Hermes "wakeable" in the README, `launch_notes` and TRUST-POLICY, and file a bead. | High (honesty; users rely on it) | Trivial | Independent of everything else |
| 3 | **Bridge-confirmed completion (F3).** Rust defers `lazy.commit()` and `attention.commit()` until the bridge confirms it returned the context to Hermes, using a second short call or a "returned" bit carried in the next envelope. | High: closes the loss window | Medium (protocol plus durable journal) | A cheaper version of hcom's admission evidence. Optionally strengthen it later: at the next `pre_llm_call`, check that `conversation_history` carries the previous turn's injected marker |
| 4 | **Per-session callback state in the bridge (F4).** Key `sequence`, the locks and the child slot by `session_id`. | High on the gateway | Small to medium | Prerequisite for 3 being meaningful under concurrency |
| 5 | **Idle wake through `ctx.inject_message(content=WAKE_UP, session_key=…)` from a long-lived bridge thread** that holds a daemon watch, as a new `IdleWake` variant (Channel-like, harness self-wake). The content-free text then hands delivery to `pre_llm_call` (R18). Busy handling stays with Hermes, which queues ("Active sessions use the existing busy-session queue", plugins.md). | High: restores rule 2 and 6 for Hermes | Medium-high: needs a daemon watch in Python, an `allow_gateway_injection` grant, a `DeliveryPolicy`/`Registry::new` pairing, and a provenance value | Two open risks. (a) The classic CLI may not display the injected turn, so a woken agent acts where the human cannot see it in the pane. Check live; if true, accept it only for the Ink TUI and Desktop, or pair it with `ctx.post_message`. (b) Mapping the `session_key` to the hook's `session_id` is unknown. On refusal, degrade to "not wakeable" with an honest state (F11) |
| 6 | **Alternative to 5: the daemon calls gateway `prompt.submit {queued:true}`** over the profile's control-socket ticket (`hermes:hermes_cli/gateway_client.py:225-258`). | Same as 5, and needs no Python listener | High: an internal Python protocol, ticket minting from Rust, and the canonical `started | queued | unknown | terminal` outcome mapping | Better fit for our daemon-driven wake ledger. Worse version fragility. Take ocs's rules: `queued:true` is mandatory, resolve the runtime id fresh on every send, set `written` before send, and treat `4090 SESSION_OWNER_UNCONFIRMED` as `unknown`, not as failed |
| 7 | **Move the static guidance into `ctx.register_system_prompt_section`** (≤ 4,000 chars, once per new session); per-turn context keeps only the seat line, ready commands, digest and rows (F6). | Medium: about 600 tokens per presenting turn | Small | Frozen across compression and resume (primer §2.4). Drop the subagent paragraph from top-level context |
| 8 | **Mid-turn delivery through `transform_tool_result` (F5).** Append the pending block only when a local pending flag is set; the flag comes from the long-lived thread of item 5, or from a cheap attention-version check. | Medium | Medium; each call costs a spawn without the listener | Must tolerate first-string-wins (needs item 3). Commit the lazy rows as `in_turn` so rule 4 suppression applies (today it applies only to `Tool` events, ours §2.2) |
| 9 | **Fail visibly and self-heal the plugin (F8, F10).** Log an ERROR and surface it in `doctor` or `setup-status` when `NativeProvider` fails or `isolation: host` is set. Report the bridge hash on every callback; on a mismatch, ask the operator to restart the gateway, or call the gateway's plugin reload (hcom's approach). | Medium | Small (the reload path costs more) | hcom's reload uses a private control-socket verb; keep that optional |
| 10 | **Add a Hermes row to the wake e2e or a separate opt-in live test** covering 0.22.x classic CLI and TUI (F12). | Medium | Medium; real-model quota | Without it, items 1, 5 and 8 stay inferred |
| 11 | **A bounded wake ledger for Hermes**: agent-peers-style monotonic receipts, a backoff that exhausts, and an `uncertain` attempt persisted before the call (F11). | Low-medium | Small (our ledger already exists) | Only once item 5 or 6 lands |

Not recommended:
- **A `ComposerPolicy` for Hermes' prompt_toolkit composer**, which would revive the keystroke wake. Every keystroke alternative for Hermes shows the hazards:
  - interrupt-by-default busy mode;
  - approval overlays that Enter answers ("once" is preselected);
  - consent and sudo prompts that eat input;
  - vim key mode;
  - Herdr not recognizing a bare Hermes.
- **Body-as-wake** (D3).
- **MCP-based delivery.** Hermes ignores resource `list_changed` and logs `notifications/message` only (primer §8).

---

## 5. What the alternatives do worse than us (keep)

- **Priority and threshold wake (rules 1 and 2).** No alternative has a priority or threshold model on Hermes, so each message either wakes or does nothing.
- **Bounded, whole-item, JSON-escaped injection** with a fixed give-way order (`fit_context`) and an explicit peer-data notice. Most alternatives are unbounded (D8).
- **No keystrokes ever reach a Hermes composer today.** That makes us safer than Cyclops, CAO, agentchattr, agmsg and xats, though only because our wake is dead (F1).
- **An `Unchanged` attention-version check**, so a quiet turn injects 0 bytes (**V**, `tests/integration/hermes_enrollment_smoke.rs:877-887`).
- **No version gate**, unlike Cotal.

---

## 6. Primer surfaces nobody uses yet

| Surface (primer §) | Use for us | Who uses it |
|---|---|---|
| `terminal.client_env_passthrough` plus `agent.session_client_env.bound_client_env()` / `tool_env()` (§1) | Pane identity for hook children and model CLI calls (rank 1) | nobody |
| Gateway `session.steer` ("inject text into the next tool result without interrupting", §5.1) | Daemon-driven mid-turn delivery with no per-tool spawn | nobody (hcom uses the plugin `transform_tool_result` instead) |
| `prompt.submit` / `inject_message` with `display_kind: "hidden"` (`hermes:tui_gateway/contracts/prompt_voice.py:33`, "only \"hidden\" is honoured") | A content-free wake row hidden from the human transcript | nobody (agent-peers v2 asks a patched host for `hidden: true`) |
| `ctx.register_system_prompt_section` (§2.4) | Static guidance once per session (rank 7) | nobody (Cotal uses `SOUL.md` and `platform_hint`, its own managed profile) |
| `on_session_end` as the per-turn idle edge, and `on_human_input_request` / `_resolved` (§2.3, §10) | An idle and blocked signal from the plugin, instead of Herdr screen status | hcom uses `on_session_end` and the human-input hooks for its own status, not as a wake gate for others |
| `pre_verify` `continue` (§2.2) | Not suitable: it fires only on code-editing turns and is capped at 3 | nobody (correctly) |
| `terminal(background=true, notify_on_complete=true)` around a blocking `herdr-threads wait` (§7) | A model-armed self-wake with no injection grant; viewer-gated | nobody ships it (agent-talk's and mcp-agent-mail's reports infer it) |
| OSC 9 / OSC 777 turn-end and permission bells (`display.bell_on_complete`, §10) | A Herdr-side idle and permission edge without screen regexes | nobody |
| `ctx.post_message` (§4) | A human-visible notice next to a classic-CLI injected turn that the CLI does not display | nobody |
| `hermes --source tool` (`hermes:hermes_cli/_parser.py:317-318`) | Keep our launched sessions out of user session lists, if that is wanted | nobody (CAO uses `--source cao`) |

---

## 7. Open questions that block the top recommendations

1. Does a plugin hook callback run inside `client_env_scope`, so that `bound_client_env()` returns the pane's passthrough values? (unknown; it decides rank 1)
2. What `platform` does `pre_llm_call` receive for a gateway-hosted classic CLI turn and for an Ink TUI turn? (unknown; F9)
3. Does a keyed `inject_message`, or a `prompt.submit` from a second client, display in an attached classic CLI? (unknown; the keyless doc says no)
4. How does the hook's `session_id` map to the `session_key` that `inject_message` expects? (unknown)
5. Are `inject_message` and `prompt.submit` viewer-gated, as internal completions are? (unknown; primer §6)
6. Does an auto-spawned gateway actually keep the first pane's `HERDR_PANE_ID` in a live run? (**I** from the strip list; not observed)

---

## Verification

Re-checked for this report on 2026-10-23:
- **Our code at `2de3da1c4`:**
  - the composer guard and `Unreadable` short-circuit (`src/host/native.rs:1876-1878`, `:2111-2113`);
  - no `composer_policy` in any Hermes file (grep);
  - `composer_policy`/`delivery_policy` defaults of `None` (`src/harness/adapter.rs:216-231`);
  - `IdleWake { Native, Channel }` (`adapter.rs:455-458`);
  - `MARKER`/`WAKE_UP` (`src/notification/policy.rs:14,17`);
  - `HERDR_PANE_ID` from the process env (`src/cli/hook.rs:255`);
  - `MAX_CONTEXT = 4096` (`hook.rs:74`);
  - emit, then `lazy.commit()`, then `attention.commit()` (`hook.rs:4475-4502`);
  - Hermes encode caps (`src/harness/hermes.rs:1046-1069`);
  - `hook_inbox.rs` page constants and `INBOX_HEADER`;
  - the bridge's non-blocking `entry_lock`, global `sequence`, platform filter, post-child discard checks and `Popen` without `env=` (`integrations/hermes/__init__.py:455-461`, `488-494`, `565-596`, `602-628`);
  - the README "wakeable" text and `launch_notes`;
  - `plugin.yaml` `provides_hooks`;
  - no `isolation` handling (grep).
- **The Hermes clone at `f3b5b980`:**
  - `plugins_injection.py:12-48` (the classic CLI interrupt queue when `_agent_running`, the `allow_gateway_injection` check, the TUI injector);
  - `session_auto_continue.py:406` (`queued` forces queue mode);
  - `prompt_voice.py:33,35` (`display_kind` hidden, `queued`);
  - `model_tools.py:838-848` (`transform_tool_result` first-string-wins);
  - `gateway_runtime_start.py:18-32` (`DAEMON_STRIPPED_ENV` has no `HERDR_*`);
  - `agent/session_client_env.py` (`_travels`, `bound_client_env`, `tool_env`, `ContextVar` binding) and `tools/code_execution_env.py:29-35` (secret substrings);
  - `gateway/session_policy.py:152-173` (client env "bound for this session's turns only");
  - `agent/turn_context.py:812-820` (`pre_llm_call` passes `session_id` and `platform`);
  - `gateway_client.py:15, 225-232` (ticket);
  - `plugins.md` gateway-injection text (keyless classic-CLI display caveat, `session_key` "not the CLI session ID", busy-session queue, no slash commands or approvals).
- **Taken from the alternative reports without re-reading their clones:** all per-tool line citations in sections 1 and 2. Each report states its own adversarial verification pass.
