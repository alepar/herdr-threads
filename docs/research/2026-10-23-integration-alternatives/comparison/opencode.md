# OpenCode: injection and wake, herdr-threads against twelve alternatives

Scope: OpenCode (https://github.com/anomalyco/opencode, formerly sst/opencode), stable line 1.x (local 1.18.1, latest release 1.19.4) and the unreleased 2.x `dev` line. Inputs:

- ours: `../ours/opencode.md` (herdr-threads at `2de3da1c4`);
- harness surfaces: `../primers/opencode.md` (cited "primer §N");
- alternatives: `../alternatives/<slug>/opencode.md` (cited "`<slug>` report §N").

Tags: **VERIFIED** means read in source or docs, by this pass or by the cited report's adversarial pass. **INFERRED** means deduced, not run. Claims about herdr-threads code were re-read in this pass (section 8). Alternative-side file and line citations are relative to each clone root (scratchpad `alts/<slug>`), with the upstream URL in the cited report.

## 0. Summary

- **We are the only tool in the set with a working core and no OpenCode path at all.** Five alternatives deliver into a live OpenCode session today: hcom, Cotal, agent-talk, agmsg (through the third-party opencode-sentinel plugin) and cross-agent-teams-mcp (xats). Four more get there by typing into the TUI. Herdr's native wake refuses the `opencode` agent kind before any prompt (`src/host/native.rs:1694-1699`, VERIFIED).
- **The winning OpenCode shape is settled by the field: a server-side prompt, not keystrokes.** Every tool that uses `prompt_async` / `session.prompt` (hcom, Cotal, agent-talk, sentinel, xats) is draft-safe by construction. Every tool that types into the TUI (agentchattr, Cyclops, CAO, ocs, the xats tmux fallback) has at least one serious OpenCode-specific defect: Enter on a pending permission dialog grants "Allow once" (agentchattr, Cyclops), a swallowed Enter is unrecoverable (CAO), or a busy TUI is misread as idle (ocs).
- **Nobody honours anything like our delivery contract on OpenCode.** No alternative has priorities or a silent class; every delivery wakes or extends a turn. OpenCode itself offers `noReply: true` (1.x) and `delivery: "queue"` / `resume: false` (2.x), and no alternative uses either for mail (xats knows `noReply` and rejects it on purpose).
- **Two ideas are worth taking almost verbatim:** hcom's ack-on-admission (ack only when the frame is seen in a model request, or on 2.x `session.inbox.delivered` for a pre-chosen message id), and sentinel/hcom's single plugin module that serves 1.x and 2.x by export shape.
- **Our cheapest real fix is not delivery at all:** an OpenCode model in a Herdr pane can today make itself an `operator_human` person seat, because nothing marks it as an agent (ours §7 L2). Every alternative that supports OpenCode identifies its agents by an explicit launch env marker.

## 1. Mechanism table (ours first)

Columns: **Inject** is how message content enters the model context. **Wake** is how an idle session is made to act. **Gate** is the idle/busy or modal check before a wake. **Silent** is whether a non-waking delivery class exists. **Draft** is whether a human's half-typed composer text survives. **ACK** is what counts as delivered.

| Tool | Support | Inject | Wake | Gate | Silent | Draft | ACK | Footprint |
|---|---|---|---|---|---|---|---|---|
| **herdr-threads today** | none (planned, `README.md:101`) | none; the model may run `herdr-threads inbox` by itself, then attribution misfires (ours §7 L2) | none: native wake refused for kind `opencode` (`native.rs:1694-1699`) | n/a | n/a | safe (nothing typed) | n/a | none |
| **herdr-threads planned** (ours §6, INFERRED design) | push channel plugin: `PushChannel` + `cooperative_mod_delivery` + `IdleWake::Channel` | body in turn via `experimental.chat.messages.transform` or `tool.execute.after`; silent items via `noReply` synthetic | content-free `wake up` via plugin `promptAsync` on idle, only when `p <= T` | own idle gate (must, rule 4); draft via optional TUI plugin | yes (`p > T`, rules 1-2) | safe; draft hold needs a TUI plugin | mod-reported predicate (TRUST-POLICY A3 row, needs OpenCode wording) | server plugin file or npm spec; optional `tui.json` plugin; skill |
| hcom | native, released; 1.x and 2.x | whole batch as a user message: 1.x `client.session.promptAsync`, 2.x `ctx.session.prompt({delivery:"steer"})` | same call; push by localhost TCP notify port | none on busy; defers while a permission or question is pending | no | safe (except 2.x first-session typed wake) | **on admission**: text seen in `messages.transform` (1.x) or `session.inbox.delivered` for its own message id (2.x) | per-run `OPENCODE_CONFIG_CONTENT`, content-addressed plugin dir, `hcom pty` wrapper |
| Cotal | native, beta; 1.x and 2.x | batch as user message via HTTP `prompt_async` to a Cotal-owned `serve`; plus prefix into the human's prompt at `chat.message` | same POST | own `busy` flag from events; never prompts a busy session | no (quiet channels are pull-only) | safe text, no draft check | at turn end (`session.idle`), with in-progress renewal; Stop counts as dismissal | managed launch only: `serve` + `attach`, `OPENCODE_CONFIG_CONTENT`, per-launch server password |
| agent-talk | native, opt-in | body as user message via plugin `client.session.promptAsync` | same call | none | no | safe | none (relay acks before the plugin) | hand-copied plugin, skills symlink, env var |
| agmsg (+ opencode-sentinel) | native on paper; inert on a stock install | turn mode: prose rule tells the model to poll; monitor mode: sentinel turns watcher stdout lines into `session.prompt` user messages | sentinel `session.prompt` (1.x), `delivery:"steer"` (2.x); manual `poke` via herdr/tmux | none | no | monitor safe; `poke` unguarded | read state on pipe write (at-most-once across sentinel) | skill, `.opencode/rules/agmsg.md` (never loaded), user-installed sentinel plugin |
| cross-agent-teams-mcp (xats) | native when launched with `--port` | ≤200-char hint as user message via HTTP `prompt_async`; body pulled by MCP `get_inbox` | same POST; tmux paste fallback for plain `opencode` | none | no (`noReply` known and rejected) | HTTP path safe; tmux path not | `get_inbox` read advances cursor; wake `ok` on any 2xx | MCP entry in `opencode.json` (plaintext token), shell launcher |
| agentchattr | not by name; config-only port viable | pointer prompt typed; body pulled by MCP `chat_read` | tmux bracketed paste + Enter | none | no | **clobbered**; Enter approves a pending permission prompt | none (cursor) | `OPENCODE_CONFIG_CONTENT` MCP entry, private tmux session |
| Cyclops | detection manifest only, unverified | one-line doorbell pasted; body via `cyclops inbox claim` in bash | tmux paste + Enter | none (state always `Unknown`) | no | **clobbered**; Enter approves permission, answers questions | claim; otherwise "notified, no verifier" | shared skill seed only |
| CAO (awslabs) | experimental provider | full body pasted as user turn | tmux/herdr paste + fixed delay + Enter; 5 s poller for byte-silent idle | screen regex on footer; skips permission dialog | no | **clobbered** | "pasted" (DELIVERED before send); no recovery for OpenCode | isolated `OPENCODE_CONFIG`/`OPENCODE_CONFIG_DIR` tree |
| open-cross-session (ocs) | none; generic CLI or cmux | CLI pull; or note typed via cmux | cmux send + Enter | screen text `to interrupt` (misses OpenCode's `esc interrupt`) | no | **clobbered** | none | none on OpenCode |
| mcp-agent-mail | MCP config only | model-pulled `fetch_inbox` | none | n/a | n/a | safe | explicit `acknowledge_message` | `opencode.json` MCP entry with plaintext bearer token |
| agent-peers-mcp | none (closed peer-type union) | piggyback block prepended to any of its MCP tool results | none; long-poll cut at 60 s by OpenCode's MCP default timeout | n/a | n/a | safe | confirm on next MCP call (per directory, not per session) | hand-rolled MCP entry |
| ruflo | none | model-pulled MCP `team_inbox`, destructive drain | none | n/a | n/a | safe | none (drain = consume) | none; side-effect skills and `CLAUDE.md` |

Sources per row: ours §1-3, §6; hcom report §2-4; Cotal report §2-4; agent-talk report §2-4; agmsg report (all sections); xats report §2-4; agentchattr report §2-4; Cyclops report (Injection, Wake); CAO report (Injection, Wake); ocs report (Injection, Wake); mcp-agent-mail report; agent-peers report; ruflo report §1-3. All VERIFIED in those reports except where a cell says INFERRED.

## 2. Every difference from our approach

"Our approach" here means the delivery contract and core machinery we run on Claude, Codex, Hermes and agy, plus the OpenCode design in ours §6. Each difference names who does it and the evidence.

- **D1. Body-bearing wake prompt.** hcom, Cotal, agent-talk and sentinel put the whole batch into the prompt that wakes the session (hcom.ts:1029-1037; Cotal src/plugin.ts:997-1038; agent-talk inbox-monitor.ts:73-99; sentinel monitor.tool.ts:38-50). We wake content-free (`CHANNEL_WAKE_UP = "wake up"`, `src/notification/policy.rs:19`; R18) and deliver the body through a separate in-context path.
- **D2. Hint-only wake plus model pull.** xats, agentchattr, Cyclops: the wake says "go read your inbox" and the model fetches the body with an MCP or CLI tool call (xats tools.ts:367-393; agentchattr wrapper.py:526-531; Cyclops DELIVERY.md:129-146). We push the body into context, rule 3.
- **D3. Pull only, no wake.** mcp-agent-mail, ruflo, agent-peers, ocs path A. We wake idle sessions on `p <= T`.
- **D4. Every delivery wakes; no priority, no silent class.** All twelve. We default to p1 and T = 0, so a default send never wakes (rules 1-2; `src/protocol/priority.rs`).
- **D5. Prompting a busy session (mid-turn steering).** hcom (deliberately; CHANGELOG.md:99), agent-talk, sentinel, xats and, by accident, the keystroke tools (agentchattr, Cyclops, ocs): OpenCode persists the message and the running loop picks it up at the next step boundary (primer §3; `runner.ts:115-122` re-checked in section 8). We never nudge a busy session (rule 4) and serve it in-turn by hooks or the channel.
- **D6. Never deliver mid-turn.** Cotal (`if (busy) return`, src/plugin.ts:1209) and CAO (`accepts_input_while_processing` false) hold everything until idle. We deliver in-turn (rule 3).
- **D7. Transport into OpenCode.**
  - in-process plugin `client` (no port): hcom, agent-talk, sentinel; our plan.
  - external HTTP to a launcher-opened port: Cotal (`serve` on loopback with a password), xats (`--port`, no password by default).
  - keystrokes: agentchattr, Cyclops, CAO, ocs, xats fallback, agmsg `poke`, hcom 2.x first-session.
  - Our Herdr native wake would be keystrokes (`agent prompt`), and it refuses OpenCode today.
- **D8. Managed launch versus in-place install.** Cotal, hcom, CAO and agentchattr own the OpenCode launch and inject config per run (`OPENCODE_CONFIG_CONTENT` or `OPENCODE_CONFIG_DIR`); none writes `~/.config/opencode`. Our plan installs a plugin into an OpenCode config dir or `opencode.json` (ours §4), and also works for a user's own `opencode`.
- **D9. Ack semantics.**
  - hcom: on admission, proven by the host (hcom.ts:270-280, 872-878 re-read in section 8).
  - Cotal: at turn end, with JetStream in-progress renewal; a user Stop dismisses (src/plugin.ts:1116-1120, 757-775).
  - agent-peers: confirm on the next MCP call.
  - xats, agentchattr: reading advances a cursor.
  - agent-talk, CAO, ocs: none, or "pasted".
  - Ours: the mod reports a predicate (`cooperative_mod_delivery`, TRUST-POLICY.md row at line 304: an answered tool result carrying context, or a `$.session.append` that resolved without `deny`), plus 60 s re-offers, at most 3 per generation (`src/delivery/ledger.rs:22-24`).
- **D10. Session binding.**
  - hcom: first bind wins, launch env `HCOM_LAUNCHED`, child sessions excluded (hcom.ts:445, 455).
  - agent-talk: latch onto any event's `sessionID` (inbox-monitor.ts:186-199).
  - xats: newest `time_updated` from `GET /session` (register-opencode-self.ts:221-256).
  - Cotal: launcher creates the session; `/new` is adopted under the same identity (src/plugin.ts:1313-1390).
  - sentinel: the session that called its tool.
  - agent-peers: one MCP process per directory, shared by every session.
  - Ours: `CallerEnvShape` plus a session variable our plugin would export through `shell.env`, with the binding rule still open (ours §6).
- **D11. Framing and role.** Every alternative delivers as a non-synthetic user-role message, shown in the TUI as if the human typed it. hcom wraps in `<hcom>…</hcom>` without escaping; Cotal uses an emoji header, a ~205-byte tail instruction and a column-zero rule (framing.ts:64, 96-98); ocs neutralises protocol-looking body lines (src/wake.ts:85-97). Ours: one `[herdr-threads]` header line per item, bodies indented two spaces at every line break class including U+0085, U+2028 and U+2029 (`src/delivery/frame.rs:55-75`, re-read in section 8), and the plan adds `synthetic: true`.
- **D12. Bounds.** Unbounded: agent-talk, Cotal (only `MAX_INBOX = 200`), agmsg, ruflo (500k-char bodies), agent-peers. hcom: 50 messages × 1 MiB. ocs: 4096-byte body, 5120-byte note. xats: 200-char hint. Ours: 8 KiB body with a `body <id>` continuation, 32 items or 64 KiB per page (`src/protocol/watch.rs:63-67`).
- **D13. Static per-request cost.** hcom re-sends a ~3.5 KB bootstrap on every request (hcom.ts:853-863). MCP-based tools carry large instructions or tool catalogues in every request: agent-peers ~8.5 KB, agentchattr ~5.7 KB instructions plus schemas, Cotal ~10 KB of tool descriptions, mcp-agent-mail ~35 KB, xats ~30-40 KB. Ours: the skill's ~350-byte description in OpenCode's listing, with the body loaded on demand (ours §5).
- **D14. Version handling.** Cotal detects the line from `opencode --version` and refuses other majors (opencode-line.ts:4-11). hcom probes `opencode --version` to pick launch shapes, defaulting to 1.x (opencode_preprocessing.rs:180-200), but its plugin module serves both APIs by export. Sentinel picks by shape (`typeof ctx?.session?.prompt === "function"`, sentinel v2.ts:79-95). Ours: no exact-version gate and no version probe as a prerequisite (AGENTS.md "Harness and Herdr versions").
- **D15. Modal (approval) handling.** hcom defers delivery while `permission.asked` or `question.asked` is pending and clears the gate on idle (hcom.ts:695-703, 765-798). CAO skips its `WAITING_USER_ANSWER` state. agentchattr and Cyclops press Enter into the dialog. Ours: the native wake refuses Herdr status `blocked` (`native.rs:1679-1681`, test at `native.rs:5032-5035`); the plan has no modal gate yet.
- **D16. Turn-start delivery written into the human's own text.** Cotal prepends the batch to the human's first text part at `chat.message` (src/plugin.ts:1165-1181). Our Claude path adds hook context next to the prompt and never edits it; the plan would add a separate part.
- **D17. Permission auto-approval.** Cotal injects `permission: "allow"` (src/extension.ts:232); xats' `free-` launcher adds `--auto`; hcom approves exactly one safe `hcom` command per `permission.asked` on 1.x (hcom.ts:705-722). We approve nothing and plan nothing (OpenCode's default agent permission is `"*": "allow"`, agentchattr report §2.1 citing `agent/agent.ts:119-120`).
- **D18. Relaunch fencing.** xats reserves a `runtime_generation` before a relaunch, refuses wakes while it is reserved, commits exact coordinates after an exact-session probe, then sends one recovery prompt (opencode-runtime-recovery.ts:147-336). Our seat continuity is by native session id match (`cooperative_continuity`, TRUST-POLICY A3).
- **D19. Plugin host singleton and reload rebind.** Cotal (`globalThis.__cotalOpencodeHooks`, src/plugin.ts:79-83) and hcom (`claimPluginHost`, `lastBinding` on `globalThis`, hcom.ts:19-25, 243-268) guard against OpenCode instantiating a plugin more than once, and against 2.x evicting an idle directory and reloading its plugins. Our plan does not mention either.

## 3. Pro/con of each difference

Axes: latency, reliability, token cost, draft safety, setup footprint, harness-version fragility, cross-machine, trust. Only axes where the difference matters are listed. "Them" means the alternative's choice; the verdict says which side wins for herdr-threads.

**D1. Body-bearing wake prompt.**
- Latency: them +. One request reaches the body; our content-free wake plus an in-turn delivery path needs the body to land in the same turn, which on OpenCode means `chat.message` or `messages.transform` must run on the wake turn (INFERRED: one request either way if we inject at `chat.message`).
- Token cost: neutral per message. The body is read once either way. But a persisted body-bearing user message is re-sent in history for the rest of the session, and so is ours once delivered.
- Reliability: them -. agent-talk drops silently on a 404 `{error}` (agent-talk report §2), and the body is lost with the failed prompt. A content-free wake that fails loses nothing, because the item stays pending in our daemon.
- Trust: them -. The body arrives with user authority and as a pseudo-human bubble. Ours keeps bodies out of wake prompts (rule (c)).
- Verdict: keep R18. hcom's 2.x variant (type a body-free `<hcom>` wake, then fill it in the host's `prompt` hook so the body persists, hcom.ts:1169-1173) is the same idea as ours, done host-side.

**D2. Hint-only wake plus MCP pull.**
- Latency and tokens: them -. At least two LLM requests per idle wake (hint, then tool call, then result), plus redundant empty pulls on bursts (xats report §5).
- Reliability: them -. agentchattr skips backlog over 20 (mcp_bridge.py:655-663); agent-peers acks beyond OpenCode's 50 KiB truncation unseen (agent-peers report, Size bounds).
- Footprint: them -. An MCP server costs 2-10k tokens of instructions and schemas in every request (D13).
- Verdict: ours wins. Our `inbox` CLI exists as the pull fallback, with no per-request cost.

**D3. Pull only.** Them - on latency (unbounded), reliability (depends on model obedience: "poll after each editing step", mcp-agent-mail app.py:9486) and tokens (empty polls). Ours wins; nothing to take except agentchattr's escalating "do not poll" replies for any pull tool we expose (mcp_bridge.py:666-679).

**D4. Every delivery wakes.**
- Tokens: them -. Every FYI costs a full turn; a chatty peer spends the recipient's quota (agent-talk report §5).
- Latency: them + for urgent items only.
- Verdict: ours wins, and OpenCode makes our contract cheap to express (`noReply`, 2.x `resume:false` / `delivery:"queue"`). Nobody uses them (section 7).

**D5. Prompting a busy session (steer).**
- Latency: them +. The item reaches the model at the next step boundary, and on 2.x `steer` is the documented native mid-turn path (primer §9).
- Tokens: them -. On 1.x a steered prompt is a new persisted user message that the loop must answer, so a p1 FYI extends the current turn (Cotal src/plugin.ts:30-33 calls this COALESCE). Ours delivers in-turn by appending context to a step that was going to happen anyway (`messages.transform`, `tool.execute.after`).
- Reliability: them + on 2.x (stable inbox API, no experimental hook), them neutral on 1.x.
- Harness-version fragility: them +. `prompt_async` is a documented, stable endpoint; our preferred `experimental.chat.messages.transform` is marked experimental, and none of the v1 hook names exist on 2.x (primer §0).
- Rule 4: steering is not a "nudge" in our sense (it does not wake an idle session), but it does extend a turn. Our contract allows in-turn delivery (rule 3), so 2.x `steer` with `resume` set from `p <= T` is contract-compatible (INFERRED).
- Verdict: split. 1.x: prefer `messages.transform` or `tool.execute.after` for p > T in-turn items; use `promptAsync` only on idle. 2.x: map in-turn delivery onto `session.synthetic({delivery:"steer", resume: p <= T})` (primer §9).

**D6. Never deliver mid-turn (Cotal, CAO).** Them - on latency: a DM to a busy agent waits out a whole tool loop, which can be minutes. Them + on simplicity only. Ours wins.

**D7. Transport.**
- In-process plugin client: no port, no credentials, reaches a stock TUI (primer §1; agent-talk docs/opencode-auto-receive.md:88-98 cites opencode#9099 on port discovery). Ours planned; agreed.
- External HTTP (Cotal, xats): needs managed launch, a port and a password the daemon must hold. xats leaves the port unauthenticated by default and pairs it with `--auto`, so any local process can drive an auto-approving agent (xats report §6.7). Cross-machine: the only + it has; it could cross a network, but our peer wire already does that between daemons.
- Keystrokes: worst on draft safety, on modal safety (Enter is "Allow once", agentchattr report §3.3, Cyclops report failure 1) and on version fragility (CAO's footer regexes are calibrated on 1.14.19 and issue #294 is open against 1.17.4).
- Verdict: ours planned (in-process plugin) is right. Do not register OpenCode with `IdleWake::Native` unless section 4 F6 is closed first.

**D8. Managed launch versus in-place install.**
- Footprint: them +. Per-run `OPENCODE_CONFIG_CONTENT` writes nothing into `~/.config/opencode`, and the plugin disappears with the launch (hcom src/hooks/opencode.rs:604-678; agentchattr report §4). That also isolates tests, which AGENTS.md requires.
- Reach: them -. Only sessions the tool launched can join (Cotal src/plugin.ts:200-204). Users who type `opencode` in a Herdr pane get nothing.
- Trust: them -, when secrets ride in the inherited env (agentchattr bearer token in `OPENCODE_CONFIG_CONTENT` reaches every bash child, agentchattr report §4).
- Verdict: offer both. An installed plugin for in-place panes, inert unless it finds a herdr-threads binding (agent-talk's env-gated inert default, inbox-monitor.ts:55-61); and a managed-launch path for `herdr-threads launch` and tests that rides `OPENCODE_CONFIG_CONTENT` with hcom's order-preserving merge.

**D9. Ack semantics.**
- Reliability: hcom +. "Admitted" is proven by the host (the frame text seen in a model request on 1.x, `session.inbox.delivered` on 2.x); a dropped or rejected prompt is redelivered after a 10 s grace, at most 3 times (hcom.ts:13-17, 628-642). Cotal + on crash safety: a turn that fails after display redelivers, a user Stop dismisses.
- Trust: hcom's predicate is stronger evidence than ours would be on OpenCode. Our A3 `cooperative_mod_delivery` predicate is "the API call resolved" on the idle paths. On OpenCode, a 2xx from `prompt_async` proves nothing about the model (it returns 204 and forks, agent-talk report §2; xats reports `ok` on any 2xx while possibly waking the wrong session, xats report §6.9).
- Tokens: Cotal -. At-least-once at turn end redelivers whole batches after any failed turn; it labels them, at least.
- Verdict: adopt hcom's admission check as the OpenCode predicate (section 5, item 3).

**D10. Session binding.** Each alternative fails differently:
- agent-talk latches onto child sessions and even onto deleted sessions (`session.deleted` carries `sessionID`), then drops every later message (agent-talk report §3 failure 1);
- xats' recency pick can select a task-tool child session or another OpenCode process's session, and reports success (xats report §6.1);
- hcom's first bind goes stale after `/new` (hcom report §6.3);
- agent-peers serves every session in a directory from one MCP process (agent-peers report, Limitations 2).
- Cotal's `/new` adoption is the only one that follows the human (src/plugin.ts:1313-1390).
- Verdict: no alternative has a design we should copy whole. Bind by explicit session id (exported via `shell.env`, which nobody uses), exclude sessions with a `parentID`, and follow the session the human selects (`tui.session.select` or a TUI plugin's `api.route.current`), the way Cotal adopts `/new`.

**D11. Framing and role.**
- Trust: ours +. A `synthetic: true` part is hidden from the human's user-text view (primer §3; `index.tsx:376-387` re-checked in section 8) and our header plus column-zero rule already covers every code point Cotal neutralises.
- Tokens: ours + against Cotal's ~245 fixed bytes per batch and agent-peers' 483-byte per-block header.
- What we lack: a redelivery marker (Cotal framing.ts:127, "(redelivered, delivery N …)") and body-line neutralisation of lines that mimic our own header beyond indentation (ocs src/wake.ts:85-97; ours is already safe because a body line can never start at column 0).
- Verdict: ours wins; consider the redelivery marker (section 4 F7).

**D12. Bounds.** Ours wins on every axis. hcom's 50 MiB worst case and agent-talk's single unbounded prompt are context floods waiting to happen.

**D13. Static cost.** Ours wins. hcom's per-request bootstrap is cacheable but is a standing tax; we keep static text in the skill (rule (a)).

**D14. Version handling.**
- Fragility: Cotal -. Refusing an unknown major breaks on the next release line. hcom's probe defaults to 1.x when `--version` fails, which mis-launches 2.x (hcom report §6.10).
- Sentinel's shape detection is versionless and matches our AGENTS.md rule: detect by observed API, degrade only the missing capability.
- Verdict: copy sentinel's shape detection, not the probes.

**D15. Modal handling.**
- Reliability and trust: hcom +. A server-side prompt during a permission dialog is the one hazard left on the plugin path; hcom gates on it and clears the gate on idle.
- Ours: the native wake refuses `blocked`, but whether Herdr reports `blocked` for OpenCode's permission or question dialog is unknown.
- Verdict: the plugin must gate on `permission.asked` / `question.asked`, as hcom does.

**D16. Writing into the human's text (Cotal).** Trust -: the stored history permanently records the human as author of peer text (Cotal docs/connect-opencode.md:167-171). Ours wins; add a separate synthetic part instead.

**D17. Permission auto-approval.** Footprint and trust -: Cotal's `"allow"` plus inherited operator MCP servers gives every spawned agent every tool (Cotal report §6.8). hcom's one-safe-command approval is the defensible version. Verdict: not needed while OpenCode's default is `"*": "allow"`; if a user's config asks for the bash tool, approve only our own read-only commands, hcom-style, and only if a user asks.

**D18. Relaunch fencing (xats).** Reliability +, for launcher-driven relaunches. Our continuity is native-session-id based and does not wake into a recovering process (INFERRED). Low priority for OpenCode; worth reading when we design managed OpenCode launch.

**D19. Plugin singleton and reload rebind.** Reliability +, cheap. OpenCode may call the plugin factory more than once (Cotal src/plugin.ts:79-83), an inherited `OPENCODE_CONFIG_CONTENT` can load a second copy (hcom.ts:19-25), and 2.x reloads plugins after evicting an idle directory without a session event, which caused hcom's "no messages after about an hour idle" bug (hcom CHANGELOG.md:73). Without a guard we would run two `watch` children on one binding (INFERRED). Verdict: adopt.

## 4. Flaws in our approach that the others expose

**F1. No OpenCode path at all (high).**
- Evidence: `builtins()` registers Claude, Codex, Hermes and agy only (`src/harness/registry.rs:1482-1490`); the cooperative wake refuses kind `opencode` (`src/host/native.rs:1694-1699`, test at `native.rs:5043-5044`).
- Exposed by: hcom, Cotal, agent-talk, sentinel and xats all reach a live OpenCode session from a plugin or the server API. agmsg #1663 reports a sentinel-delivered message "answered within 3 seconds" under the herdr terminal driver (agmsg report, Wake mechanism).
- Effect: an OpenCode agent in a Herdr pane misses every item until a human tells it to run `inbox`.

**F2. An OpenCode model can act as a person seat (high, cheap).**
- Evidence: the A4 guard looks for a registered adapter's env marker or a Herdr-reported registered agent kind (`src/cli/mod.rs:355-371`; `src/protocol/authority.rs:127-131`; `src/cli/output.rs:165-177`). Without a marker, `caller::derive` maps `NoHarness` to `CallerOrigin::Human` (`src/cli/caller.rs:69-80`). OpenCode sets `OPENCODE=1`, `AGENT=1`, `OPENCODE_PID` (primer §10), none of which we recognise.
- Exposed by: every alternative that supports OpenCode identifies its agents with an explicit launch marker (`HCOM_LAUNCHED`, `COTAL_NAME`, `OPENCODE_XATS_BASE_URL`, `AGENT_TALK_OPENCODE_SPOOLS`). ocs shows the opposite failure (an OpenCode started inside another harness's shell is attributed to that harness, ocs report Limitations 7), so a marker must be matched to the right harness, not just present.
- Effect: `herdr-threads human me init` succeeds in an OpenCode pane, and its messages are framed `[human]` (ours §7 L2). Within the cooperative model, but it contradicts A4's intent (TRUST-POLICY.md:307).

**F3. Our mod-delivery ACK predicate would be weak on OpenCode (medium).**
- Evidence: the A3 row for `cooperative_mod_delivery` (TRUST-POLICY.md:304) accepts "a `$.session.append` … that resolved without `deny`" on the idle path. The direct OpenCode analogue is a 2xx from `prompt_async` / `prompt({noReply:true})`.
- Exposed by: hcom (hcom.ts:270-280, 872-878) treats a 2xx as hand-off, not delivery, and acks only on admission; agent-talk shows that a 2xx-or-`{error}` client never throws on 404 (SDK v1 `throwOnError` default false, agent-talk report §2) and loses the message; xats shows a 204 into the wrong session reported as `ok`.
- Effect, if ported as-is: an item acked as delivered that never reached the model's context, with no re-offer.

**F4. Session binding for a multi-session TUI is unresolved (medium).**
- Evidence: ours §6 `BindingPolicy` row says "Open design question"; ours §7 L3.
- Exposed by: four different binding failures (D10). Each was found in code or an issue, not hypothetically.
- Effect: without a rule, deliveries reach a child task session, a deleted session or a session the human left.

**F5. Our in-turn path on OpenCode 1.x rides an experimental hook, and nothing on 2.x (medium).**
- Evidence: ours §6 picks `experimental.chat.messages.transform`, with `tool.execute.after` as fallback; on 2.x the v1 hook names occur nowhere (primer §0).
- Exposed by: hcom and sentinel ship one module with a v1 `server` export and a v2 `setup` export (hcom.ts:1217-1221; sentinel index.ts), and both already deliver on 2.x. hcom also confirms the v1 transform works for admission and wake filling in practice (hcom.ts:846-878, with tests in `tests/plugin_delivery.test.ts`).
- Effect: a v1-only plugin breaks on the first 2.x stable release. Not a defect today; a plan gap.

**F6. A native keystroke wake into OpenCode would be unsafe around modals (medium, latent).**
- Evidence: our wake refuses Herdr `blocked`, `working` and `unknown` (`native.rs:1679-1681`); how Herdr classifies OpenCode, including its permission dialog, is unknown (ours §7 L6).
- Exposed by: agentchattr and Cyclops both press Enter into OpenCode's permission prompt, whose default selection is "Allow once" (`permission.tsx:405, 536-538, 607-611` per agentchattr report §3.3 and Cyclops report failure 1); Cyclops' Enter also answers question dialogs with option 1. CAO's byte-silent idle (issue #203) shows Herdr's own status for OpenCode may also be edge-only (INFERRED).
- Effect: only if someone registers OpenCode with `IdleWake::Native` before Herdr's OpenCode classification is measured. The plugin path does not have this hazard.

**F7. Re-offers are not marked as re-offers (low).**
- Evidence: the ledger re-offers an unconfirmed id after 60 s, up to 3 times per generation (`src/delivery/ledger.rs:22-24`); `frame()` has no redelivery marker (`src/delivery/frame.rs:133-163`).
- Exposed by: Cotal's broker-counted `(redelivered, delivery N: an earlier delivery may already have run)` (framing.ts:127), added because at-least-once delivery means the model may already have acted.
- Effect: a model that already acted on an item may act on it again. Applies to every harness, not just OpenCode. The cost would be one marker of about 15 bytes on re-offers only, per rule (f).

**F8. The skill reaches OpenCode by accident (low).**
- Evidence: ours §4 and §7 L4; OpenCode scans `~/.claude/skills` and `~/.agents/skills` (primer §6).
- Exposed by: Cyclops notes OpenCode logs `duplicate skill name` when two seeded copies exist (Cyclops report, Setup); ocs and agmsg ship skills whose identity claims are false on OpenCode.
- Effect: an OpenCode model reads instructions for commands whose attribution then misfires (F2).

**F9. No draft awareness without a second plugin (low, cost).**
- Evidence: the Claude mod holds a wake for up to 120 s while a draft is present (`register.js:16, 536-546`, re-read in section 8). On OpenCode the server plugin cannot see the draft; only a TUI plugin can (`TuiPromptRef`, primer §7).
- Exposed by: no alternative checks the draft on the server-prompt path (hcom, Cotal, agent-talk, xats). Their text survives; a turn still starts under a typing human. Nobody uses a TUI plugin.

Things the others do worse, where ours holds up (no change needed): content-free wakes (R18), priority threshold (rules 1-2), bounded pages and bodies, one-line framing with column-zero indentation, static guidance only in the skill, no version gate.

## 5. Better integration ways, ranked by value/cost

Ranked by value per unit of cost. Each item says what to take and from where.

1. **Minimal OpenCode registration for identity (value high, cost low).** Register host kind `opencode` and env marker `OPENCODE` with `person_guard: true`, before any delivery work (ours §7 L2 fix). Closes F2. Check first that a registered kind with no bound harness still refuses the cooperative wake: it does (`native.rs:1720-1722`, "no bound harness for a cooperative wake"). The marker must not be `AGENT`, which is too generic (ours §6).
2. **In-process server plugin as a push channel (value high, cost medium-high).** Proven transport: the plugin's own `client` (hcom, agent-talk, sentinel). Spawn `herdr-threads watch --harness opencode --session <id>`, which the registry already supports for any `PushChannel` adapter (`src/harness/registry.rs:1434-1443` per ours §2.2). Idle wake: our own busy tracking from `session.status` (Cotal's coalesce guard, src/plugin.ts:1209), then `promptAsync` with a `synthetic: true` `wake up` part, only when `p <= T`. Silent items: `noReply: true` synthetic parts, or in-turn via `messages.transform` / `tool.execute.after`. Closes F1.
3. **Ack on admission (value high, cost low on top of item 2).** Ack an item only when its frame header is seen in a model request (`messages.transform` input, 1.x) or on `session.inbox.delivered` for a message id the plugin chose (2.x); release after a grace and retry a bounded number of times (hcom.ts:270-280, 593-642, 799-808). Add the predicate to TRUST-POLICY A3 in the same commit (AGENTS.md "Trust policy"). Closes F3. This is the OpenCode counterpart of our agy `cooperative_hook_delivery` rule, which already requires finding the frame header in an injected step.
4. **Modal gate in the plugin (value high, cost low).** Defer every prompt while a `permission.asked` or `question.asked` is pending; replay on reply; clear on idle (hcom.ts:695-703, 765-798).
5. **One module, two plugin APIs, chosen by shape (value high for 2.x, cost medium).** Default export `{ server, setup }` (hcom.ts:1217-1221) with sentinel-style shape checks (sentinel v2.ts:79-95). On 2.x map `p <= T` to `session.synthetic({delivery:"steer", resume:true})` and `p > T` to `resume:false` or `delivery:"queue"` (primer §9); nobody does this yet. Closes F5 without a version probe.
6. **Explicit session binding (value medium-high, cost low).** Export our own session variable through `shell.env`, as ours §6 already plans; nobody in the set uses `shell.env`. Exclude sessions with `parentID` (hcom CHANGELOG.md:109 did the same). Follow the human's session switch (Cotal's `/new` adoption) rather than first bind or recency. Closes F4.
7. **Per-run config for managed launch and tests (value medium-high, cost low).** `OPENCODE_CONFIG_CONTENT` with an order-preserving JSONC merge of the caller's value plus our plugin path (hcom src/hooks/opencode.rs:604-678), no secrets in it (agentchattr report §4). Satisfies the "never write real user config" rule for tests and `launch`.
8. **Plugin singleton plus 2.x reload rebind (value medium, cost low).** A `globalThis` host claim and a rebind record keyed by directory (Cotal src/plugin.ts:79-83; hcom.ts:19-25, 243-268).
9. **User Stop is dismissal, not failure (value medium, cost low).** On `MessageAbortedError` or an interrupt command, do not re-drive the batch into a person who just pressed Stop (Cotal src/plugin.ts:757-775, 1428-1460). Our Claude mod has an analogous pause rule (Cotal report §7.5 cross-references it).
10. **TUI plugin for draft-aware wakes and non-model notices (value medium, cost medium).** `TuiPromptRef.current.input` for a draft hold like the mod's 120 s; `api.ui.toast` or `api.attention.notify` for a human-visible notice of `p > T` items with no model tokens (primer §7). Closes F9. Nobody uses TUI plugins.
11. **Redelivery marker (value low-medium, cost low).** A short marker on re-offered items, framed in `frame()` and its JS twin. Closes F7 for every harness.
12. **Compaction note (value low, cost low).** One line through `experimental.session.compacting` saying threads are pending, so state survives compaction (hcom uses the same hook for its identity line, hcom.ts:892-896).

Not recommended (from the evidence): an external HTTP port with managed `serve` + `attach` (Cotal, xats: credentials, managed launch only, unauthenticated by default in xats); keystroke wakes (CAO, Cyclops, agentchattr, ocs: modal approvals, unrecoverable swallowed Enter, footer-regex drift); MCP pull tools (large standing cost, model-obedience delivery); per-request bootstrap text (hcom); version probes (Cotal, hcom).

## 6. Primer surfaces nobody uses yet

Checked against all twelve reports and by grep over the clones under `scratchpad/alts` (section 8). "Nobody" means no alternative uses the surface for message delivery or wake.

| Surface (primer §) | What it offers us | Who comes closest |
|---|---|---|
| `noReply: true` on `prompt` / `prompt_async` (§3) | silent staging for `p > T` (rule 2), durable in history | xats knows it and rejects it (`opencode-server-dispatch.ts:48-65`, spec.md:12) |
| `parts[].synthetic: true` for delivered mail (§3) | hides peer text from the human's user-text view; marks "not typed by the human" | hcom uses it only for its bootstrap part (hcom.ts:853-863) |
| `tool.execute.after` output mutation (§2) | post-tool in-turn delivery, the `PostToolUse` analogue | none (grep: zero hits in any alternative's source) |
| `experimental.chat.messages.transform` for mail bodies (§2) | per-step, ephemeral in-turn delivery with no model call | hcom uses it for bootstrap, wake filling and admission detection, not for delivering mail |
| `shell.env` hook (§2, §10) | export a session id into every agent-run shell for caller attribution | none; sentinel sets its own child's env only (sentinel watcher.ts:24) |
| TUI plugins: `TuiPromptRef`, `api.state.session.status`, `api.route.current`, `api.ui.toast`, `api.attention.notify` (§7) | draft-aware wakes, the shown session, non-model notices | none |
| 2.x `session.synthetic` with `resume: false` / `delivery: "queue"`, inbox `PATCH`/`DELETE` (§9) | the native "wake iff `p <= T`" mapping, and cancellation | hcom and sentinel use `prompt({delivery:"steer"})` only; hcom honours `inbox.cancelled` |
| `chat.message` adding a separate part (§2) | turn-start delivery without editing the human's text | Cotal uses the hook but edits the human's text part |
| `experimental.compaction.autocontinue` (§2) | control over OpenCode's own self-continue turn after compaction | none |
| custom command with `` !`herdr-threads inbox` `` (§6) | a `/inbox` slash command that inlines mail into a user turn, no extra tool call | none (agmsg relies on skill slash commands) |
| `opencode run --attach … -s <id>` (§4) | shell-side inject into a server-backed TUI | none |
| ACP (`opencode acp`) (§4) | herdr-threads as the ACP client | none for OpenCode (agent-peers uses ACP for Droid only) |
| `/tui/control/next` / `response` (§3) | unknown semantics | none |

Surfaces in use that we should avoid: `/tui/append-prompt` + `/tui/submit-prompt` (concatenates onto a draft; agent-talk rejected it in writing, docs/opencode-auto-receive.md:80-86) and `experimental.chat.system.transform` (per-request system edits defeat prompt caching; nobody uses it).

## 7. Open questions carried forward

- How Herdr classifies OpenCode idle, busy and modal states (ours §7 L6). Matters only for a native fallback (F6).
- Whether the TUI renders API-sent `synthetic` parts at all (primer open questions). The scroll filter hides them (`index.tsx:376-387`); the full render path is unknown.
- Whether the 2.x `prompt` hook fires for plugin-initiated prompts (Cotal report §6.4 flags a possible double injection). Relevant to item 5.
- OpenCode's tool-call timeout for a custom long-poll tool: the MCP path defaults to 60 s (agent-peers report, Wake mechanism); a plugin `tool` timeout is unknown.

## 8. Verification

Re-read in this pass, against `/var/home/alepar/AleCode/herdr-threads` at `2de3da1c4`:

- `src/harness/registry.rs:1270-1283` (delivery pairing: `PushChannel` requires `cooperative_mod_delivery`; `HookPull` requires `cooperative_hook_delivery` and `Native`) and `:1482-1490` (four builtins).
- `src/host/native.rs:53` (`WAKE_READY_STATUSES = ["idle", "done"]`), `:1679-1723` (blocked/working/unknown never prompted; unregistered kind refused before the bound-harness check; "no bound harness for a cooperative wake"), `:5020-5044` (test cases `blocked` and `unrecognized` with kind `opencode`).
- `src/cli/caller.rs:69-80`, `src/cli/mod.rs:355-371`, `src/cli/output.rs:165-177`, `src/protocol/authority.rs:127-131` (F2 path).
- `src/delivery/frame.rs:9, 42-75, 133-163` (header, `one_line`, indentation at `\r\n`, `\n`, `\r`, U+000B, U+000C, U+0085, U+2028, U+2029; no redelivery marker).
- `src/protocol/watch.rs:63-67` (8 KiB, 32 items, 64 KiB) and `:168-179` (`ModDeliveryVia` context/submit/append).
- `src/delivery/ledger.rs:22-24` (60 s, 3 per generation).
- `src/notification/policy.rs:14-21` (`MARKER`, `WAKE_UP`, `CHANNEL_WAKE_UP`).
- `integrations/claude/mod/hooks/register.js:12-16, 536-546` (`WAKE_PROMPT`, `DRAFT_WAIT_MS = 120000`, draft hold).
- `TRUST-POLICY.md:300-308` (A3 rows, `cooperative_mod_delivery` wording, `operator_human` best effort).

Spot-checked in the alternative clones (read only, nothing executed):

- hcom `src/opencode_plugin/hcom.ts:268-282` (ack-on-admission comment and `Delivery` record) and `:872-878` (admission by exact text match in `messages.transform`).
- Cotal `extensions/connector-opencode/src/plugin.ts:28-34` (coalesce rationale) and `:1206-1213` (`if (busy) return`).
- agent-talk `extensions/opencode/inbox-monitor.ts:86-99` (body-bearing `promptAsync`, catch re-buffer).
- Grep over every clone under `scratchpad/alts` (excluding `harness-src`) for `shell.env`, `tool.execute.after`, `TuiPlugin`, `api.ui.toast`, `resume: false`, `delivery: "queue"` and `tui/control`: zero hits. `noReply` appears only in xats; `messages.transform` only in hcom; `synthetic` in hcom (bootstrap) and in Cotal's AG-UI mapping code (reading, not sending).

Spot-checked in the OpenCode `v1.18.1` clone (`scratchpad/harness-src/opencode-1.18.1`):

- `packages/opencode/src/session/prompt.ts:1069` (`if (input.noReply === true) return message` before `loop`).
- `packages/opencode/src/effect/runner.ts:115-122` (a `Running` state awaits the existing run, so a busy-time prompt joins it).
- `shell.env` trigger sites at `tool/shell.ts:418`, `session/prompt.ts:555`, `server/.../handlers/pty.ts:71`.
- `packages/tui/src/routes/session/index.tsx:376-387, 629` (synthetic and ignored text parts filtered from user-text views).

Not re-checked here: every other alternative-side citation, which this report takes from the per-alternative reports and their own adversarial passes; GitHub issue states (agmsg #1663/#1462, CAO #203/#294/#562); and OpenCode 2.x (`dev`) behaviour, which nobody has run.
