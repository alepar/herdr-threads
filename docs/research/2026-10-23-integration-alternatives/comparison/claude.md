# Comparison: message injection and wake on Claude Code

- Inputs: [../ours/claude.md](../ours/claude.md) (herdr-threads at `2de3da1c4`), [../primers/claude.md](../primers/claude.md), and the twelve `../alternatives/<slug>/claude.md` reports. "ours §N", "primer §N" and "<slug> §N" point into those files.
- Tags: **VERIFIED** means read in our code, in a cited clone source, in docs, or in a read-only local check made for this report. **INFERRED** means reasoning. **unknown** means not found.
- Every claim about herdr-threads that this report relies on was re-read in the tree at `2de3da1c4` (list at the end). Claims about alternatives rest on the per-tool reports, which cite their clone at a pinned commit. Where this report re-checked a claim locally, it says so.

---

## 1. Mechanism table

"Busy" means a turn is running. "Idle" means the session waits at an empty prompt. "In-band" means the message body reaches the model without the model calling a tool.

| Tool / mode | Busy injection | Idle injection | Wake primitive | Who wakes | Draft guard | Body in-band, bounds | Receipt ("ACK") point |
|---|---|---|---|---|---|---|---|
| **herdr-threads, mod** (primary) | Waking items: `tool.call` result `context` on the next answered main tool call. Passive items: `$.session.append` hidden row, at once | `$.session.append` rows, then a wake | `$.prompt.submit({text:'wake up'})`, plugin-attributed, content-free | p ≤ T (default T=0, p1: silent) | `$.prompt.read()`, hold up to 120 s, then wake anyway | Yes; 32 items / 64 KiB per batch, 8 KiB per body + continuation | `watch ack --via context\|append\|submit` when text is attached or appended |
| **herdr-threads, hooks only** (fallback) | `SessionStart` and **Bash-only** `PreToolUse` `additionalContext`, ≤ 4096 B, ≤ 8 rows | Next `SessionStart`/Bash boundary | Herdr `agent prompt` types `herdr-threads: attention pending; run herdr-threads inbox` | p ≤ T | Composer must read `Empty`; Claude text is always `Unsafe`; 60 s empty window if focused | Hook page yes; after a wake, a Bash `inbox` round trip | ACK on display (hook output flushed) |
| agent-talk | Plugin-monitor line or model-armed Monitor-tool watch, raw NDJSON | Same, as a task notification (idle turn INFERRED) | Harness monitor notification | Every message | None (nothing typed) | Yes, unbounded | None; relay ACKs when the follower reads |
| Cotal, personal (mod) | `$.session.append` after `await next()` in main `tool.call` | `$.prompt.submit` with the **whole batch** as prompt text | `$.prompt.submit`, plugin-attributed | Directed / @mention | None | Inline ≤ 8 items, ≤ 1,024 cp each, ≤ 4,096-char frame; else a pointer + `cotal_inbox` | Claim → host row uuid + exact stored-text match → commit |
| Cotal, managed seat | None mid-turn | `SessionStart` / `UserPromptSubmit` `additionalContext` drain | Dev channel `notifications/claude/channel` nudge | Directed; ambient in `open` mode | n/a (no human assumed) | Hook drain, no byte cap | Hook stdout write receipt over control socket |
| agmsg, `monitor` | Monitor-tool event per line | Same | Monitor event; watch armed by the model from a SessionStart directive, self-expires after ~29 quiet min | Every message | None | Yes, unbounded | Marked read when printed |
| agmsg, `turn` | Stop hook `decision:block` + `reason` | None | None | — | — | Yes, unbounded | Marked after the write succeeds |
| agmsg, agmsgd (beta) | Native cross-session socket notice, absorbed between tool calls | Notice starts a turn; model runs `inbox.sh` | Claude native `CLAUDE_CODE_MESSAGING_SOCKET` | One outstanding notice per seat | None needed (INFERRED) | No (Bash pull, unbounded) | Transcript nonce confirms the wake; marked on `inbox.sh` display |
| ruflo | Native in-team `SendMessage`; MCP `team_inbox` pull | Native `SendMessage` (in-team only) | Claude's own; none across sessions | n/a | `prompt.fill('replace')` can clobber | Pull drains everything | None |
| mcp_agent_mail | Count-only Bash `PostToolUse` reminder, ≥ 120 s apart | None | None | — | — | No (MCP pull) | Explicit `acknowledge_message` / `mark_message_read` |
| hcom, native (default) | `PostToolUse` (all tools) + `UserPromptSubmit` `additionalContext` | `UserPromptSubmit` fired by the wake prompt | Mod `$.prompt.submit({text:'<hcom>', asUser:true})` from a 10 s long-poll process | Every message | None in native mode | Yes, ≤ 50 msgs, no byte cap | Cursor commit after stdout flush |
| hcom, `-p` / subagents | Stop / SubagentStop block-loop long-poll | — | Stop hook blocks until mail | Every message | — | Yes | Same |
| agentchattr | Typed wake text queued by the TUI; body via MCP `chat_read` | tmux paste + Enter | tmux `paste-buffer -p` + Enter | Every @mention | None | No (MCP pull, 20 rows) | Cursor only |
| cross-agent-teams (xats) | Channel hint queued to next turn; body via `get_inbox` | Channel hint | Dev channel, tmux fallback | Every send | tmux quiet-screen guard only | No | `get_inbox` cursor = read |
| agent-peers | Channel push of the body (dev flag) | Channel (project believes it does not wake) or `asyncRewake` Stop hook | `asyncRewake` exit 2 with a bodyless reminder | Every new id, once | None | Channel yes; `check_messages` unbounded | Broker ACK on local persist; confirm-on-next-call |
| Cyclops | Doorbell typed into a working pane, queued by the TUI | Bracketed paste + Enter | tmux paste | Every message | SGR-byte composer discriminator; release on pane event | No (Bash `claim`) | Exact-match `UserPromptSubmit` hook ACK, then claim |
| open-cross-session (ocs) | Native socket frame read between tool calls | Native socket frame starts a turn | Claude native cross-session socket | Every mention / DM | None needed (INFERRED) | Yes, ≤ 4,096 B inline, ≤ 5,120 B note | `peer_message_status` receipts (held/refused/dropped) |
| CAO (awslabs) | Typed paste in eager mode (off by default) | Paste + 2 Enters | tmux / Herdr typing | Every message | None | Yes, whole body pasted | None (DELIVERED = typed); causal submit confirmation |

Sources: ours §1-§3; agent-talk §1-§3; cotal §1-§3; agmsg §1-§3; ruflo §2-§3; mcp-agent-mail "Injection"/"Wake"; hcom §1-§3; agentchattr "Injection"/"Wake"; cross-agent-teams §2-§3; agent-peers "Injection"/"Wake"; cyclops §2-§3; open-cross-session §2-§3; CAO §2-§3. Our rows re-verified in code (§8).

### 1.1 Where we sit

- Only three tools carry the body in-band on **both** busy and idle paths with a bounded page: herdr-threads (mod), Cotal personal, and ocs. hcom does it too but without a byte bound. Everyone else either pulls through a tool call (agentchattr, xats, Cyclops, agmsgd, mcp_agent_mail) or delivers unbounded raw lines (agent-talk, agmsg monitor). VERIFIED from the reports cited above.
- Only herdr-threads has a **non-waking delivery class**. Every other tool wakes on every message (agentchattr, xats, hcom, Cyclops, CAO, ocs, agent-talk, agent-peers) or on every directed message (Cotal). VERIFIED by each report's "every message wakes" finding.
- Only herdr-threads and Cyclops hold an automatic wake for a human draft on Claude. Cyclops does it from screen bytes; we do it from `$.prompt.read()`. hcom does it only in its opt-in typing mode, and agmsg only in its manual, agent-invoked `poke` (styled two-snapshot guard, agmsg §poke). VERIFIED (cyclops §3.3; hcom §3.2-§3.3; cotal §3.2; ours §3.1).
- Two tools use Claude's **native cross-session socket** (agmsgd, ocs), three use **channels** (Cotal managed, xats, agent-peers), one uses **`asyncRewake`** (agent-peers), one uses **plugin monitors** (agent-talk), two use the **Monitor tool** (agent-talk, agmsg). We use none of these. VERIFIED (primer §9 "Not used today"; re-grepped, §8).

---

## 2. Every difference from our approach

Each difference is named D-n and analysed in §3.

| # | Difference | Who does it differently |
|---|---|---|
| D1 | Busy delivery for waking items uses `session.append` after the tool call instead of `tool.call` result `context` | Cotal personal |
| D2 | Idle wake carries the **body in the submitted prompt** instead of appended rows plus a content-free `wake up` | Cotal personal (full batch), ocs (note in the socket frame), CAO (pasted body) |
| D3 | Idle wake through the **native cross-session socket** instead of `$.prompt.submit` | agmsgd, ocs |
| D4 | Idle wake through an **MCP channel** | Cotal managed, xats, agent-peers |
| D5 | Idle wake through an **`asyncRewake` Stop hook** | agent-peers |
| D6 | Push through a **plugin monitor / Monitor tool** | agent-talk, agmsg monitor |
| D7 | Turn-end delivery through a **Stop `decision:block`** | agmsg turn, hcom headless |
| D8 | Body delivered at **`UserPromptSubmit`**, with the wake only submitting a marker | hcom, Cotal managed |
| D9 | Mid-turn hook delivery on **every tool** (`PostToolUse` unmatched) instead of Bash only | hcom (we are Bash-only in the hooks-only stack) |
| D10 | **Body pulled by a tool call** after a wake hint | agentchattr, xats, Cyclops, agmsgd, mcp_agent_mail, agent-peers rewake |
| D11 | **Every message wakes**; no threshold or passive class | all twelve |
| D12 | **No draft guard**, or a guard that never expires | none: hcom native, Cotal, agentchattr, CAO, xats; never expires: Cyclops |
| D13 | Draft detection by **SGR dim attribute** on a styled pane read instead of plain detection text | agmsg `safe-poke`, hcom inject, Cyclops |
| D14 | After a user abort, **pause until a person-origin prompt** instead of a timed hold | Cotal personal; hcom (submission_failed until the cursor moves) |
| D15 | **Host-confirmed receipt** (stored row uuid, transcript nonce, native receipts, exact-match `UserPromptSubmit`) instead of ACK on attach/append | Cotal, agmsgd, ocs, Cyclops |
| D16 | Wake prompt sent **`asUser: true`** (no plugin frame) | hcom |
| D17 | Empty wakes **refused at `UserPromptSubmit`** | hcom |
| D18 | Wake claim **keyed on the delivery cursor** in durable storage | hcom |
| D19 | **Idle oracle** from Claude's own session registry `status` or screen regexes, instead of mod turn events / Herdr `agent_status` | ocs (registry, for idle-watch), CAO and Cyclops (screen) |
| D20 | **Per-run** `--settings` / `--plugin-dir` (content-addressed) instead of persistent `settings.json` edits | hcom, Cyclops default |
| D21 | Install through **Claude's marketplace** instead of `CLAUDE_CODE_PLUGIN_DIRS` | Cotal, agent-talk, ruflo |
| D22 | **Owns the PTY / tmux session** instead of riding an existing Herdr pane | hcom, agentchattr, Cyclops, CAO |
| D23 | **Cross-machine** transport (mesh, relay, HTTP broker) | Cotal, agent-talk, mcp_agent_mail, xats |
| D24 | **Per-delivery static instructions** in the frame (tail lines, reply hints, anti-ack line) | Cotal (205 B tail), ocs (anti-ack + `Reply:`/`Thread:`), CAO suffix, agent-peers header |
| D25 | **Identity carried by the model** or by pid walks instead of daemon-decided seat binding | agent-talk, xats, agent-peers rewake, ocs |
| D26 | **No automatic fallback** when the primary path breaks | hcom (manual opt into inject mode), agmsgd (only at next SessionStart) |
| D27 | Health surfaced as **status with the remedy** | hcom (`tui:mod-unavailable` etc.) |
| D28 | **Whole messages only, never truncated**, a pointer for the rest | Cotal inline |
| D29 | Body **neutralisation** of protocol-looking lines (prefix `> `, tag rewrite) | ocs, ruflo (`│ `) |

---

## 3. Pro/con of each difference

Dimensions: latency (Lat), reliability (Rel), token cost (Tok), draft safety (Draft), setup footprint (Setup), harness-version fragility (Ver), cross-machine (XM), trust. "+" favours the alternative, "−" favours ours.

**D1. `session.append` instead of `tool.call` `context` for busy waking items (Cotal).**
- \+ Lat/Rel: an appended row enters "a running turn's requests from the loop's next top" (primer §2.2), so it does not need an *answered* tool result. Our waking items wait for an answered main `tool.call` (`register.js:829-831`) and so stall through a long `Agent` call or a tool-free generation (ours §6.3). We already append passive items at any time (`register.js:635-669`), so the mechanism is proven in our own mod.
- \+ Rel: the append returns a stored-row uuid that can back a receipt (Cotal `native-wake.js:105-106`).
- − Trust/Tok: `context` is "one reminder, as a PostToolUse hook's is" (primer §2.2); an appended hidden user row may weigh more with the model (INFERRED, cotal §7.4). Cotal adds the rule "never append in `turn.step`" because a final step leaves no next request (cotal §2.3); our `pumpLazy` appends passive rows during any busy moment, so a passive row appended during the final step is read only on the next turn (INFERRED from `register.js:635-669`; harmless for non-waking rows, but not for waking ones).
- Verdict: worth adopting as a *second* busy path for waking items when no tool result has arrived within a bounded time, not as a replacement.

**D2. Body in the submitted prompt (Cotal, ocs, CAO).**
- \+ Tok: one artifact per wake instead of rows + `wake up`.
- − Trust: a plugin prompt is read as "The <name> plugin sent a message: …" (primer §2.3); a CAO paste is a user turn and may be framed "pasted from somewhere else" (CAO §2.2). Our rows keep the peer body in hidden user rows framed by our header, and the wake is content-free (R18, AGENTS.md (c)).
- − Rel: Cotal must then match the stored row's text to confirm admission, which is fragile to any engine rewrap (cotal §6.7).
- Verdict: keep ours.

**D3. Native cross-session socket (agmsgd, ocs).**
- \+ Draft: nothing touches the composer; Claude decides idle vs busy itself.
- \+ Lat: busy delivery "between tool calls" regardless of whether a tool result is "answered" (primer §3).
- \+ Rel/Setup: works without a mod. It is the strongest candidate for our hooks-only fallback, where today we type into an always-`Unsafe` composer (ours §6.7).
- \+ Receipts: ocs gets `held / refused / dropped / expired` from Claude (`peer_message_status`, ocs §2.6); agmsg confirms the wake by transcript nonce (agmsg §2.3).
- − Ver: the wire schema is undocumented (primer §3); ocs and agmsg reverse-engineered it (`msgV:1`, `priority:"next"`, attribute order that must be byte-identical). A tightening breaks silently (ocs §3 failure 4, agmsg §6.9).
- − Trust: agmsg self-declares `from-mode="bypass"` to dodge holds (agmsg §6.9); ocs always sends `from-mode="prompting"` (ocs §6.2). Both work around Claude's inbound policy rather than inside it. A child-of-session post (our `watch` is spawned by the mod) should qualify for the documented own-child exemption (primer §3), which neither alternative uses: agmsgd is a login service, and ocs's helper is `setsid`-detached (ocs §6.6). INFERRED.
- − Tok: native framing replaces our one-line header and adds a wrapper; the 50-message queue and short-window dedupe are Claude's.
- Verdict: high-value fallback candidate; needs our own capture first.

**D4. MCP channel (Cotal managed, xats, agent-peers).**
- \+ Draft: not typed.
- − Setup/Ver: research preview, `--dangerously-load-development-channels` (a confirm dialog per launch), allowlist, Anthropic auth only, `channelsEnabled` on Team/Enterprise (primer §4). xats shows the failure: without the flag events are dropped silently while the sender sees `delivered` (cross-agent-teams §3.5.1).
- − Rel: no ACK (primer §4). Whether a channel event even starts an idle turn is disputed: agent-peers believes it does not, the primer infers it does (agent-peers "When it fires"; primer §10 Q3).
- Verdict: not for an unattended plugin. Agrees with primer §4.

**D5. `asyncRewake` Stop hook (agent-peers).**
- \+ Draft: types nothing. + Setup: plain settings hook; works where mods are refused (managed `disableSideloadFlags`, `--hooks-only`, older Claude).
- \+ Ver: documented behaviour ("wakes Claude immediately even when the session is idle", primer §1.4), unlike D3.
- − Rel: one wake per hook run; must re-arm at each `Stop`; a new session is not wakeable before its first turn ends (agent-peers "Failure modes"); the timeout bounds each wait.
- − Draft/Lat unknown: mid-turn exit 2 and a typed draft are uncaptured (primer §10 Q2). agent-peers has no busy gate; our daemon could supply one (the waiter asks `wake_frontier` before exiting 2).
- Verdict: strong fallback wake; cheaper than D3 to trust because documented.

**D6. Plugin monitor / Monitor tool (agent-talk, agmsg monitor).**
- \+ Setup: one `monitors.json` entry, no mod API.
- − Rel: Monitor-tool watches are capped at 30 min and re-armed by the model (agent-talk §3.2, agmsg §3.1: "deaf after ~29 quiet minutes"). A plugin monitor runs for the session but is interactive-only, not on Bedrock/Vertex/Foundry, and noisy monitors are auto-stopped (agent-talk §3.6).
- − Setup for us: a plugin monitor ships inside a plugin; ours is loaded through `CLAUDE_CODE_PLUGIN_DIRS`, which managed `disableSideloadFlags` refuses (primer §2.1), so it would not help in that fallback case unless installed through a marketplace (INFERRED).
- − Tok/Trust: no ACK of model receipt, no draft hold, one notification per line.
- Verdict: low value for us.

**D7. Stop `decision:block` (agmsg turn, hcom headless).**
- \+ Lat for a busy session that makes no tool call: mail can be appended to a turn that is ending.
- − It cannot wake an idle session (primer §1.3); loop cap of 8 blocks; one extra model call per batch. agmsg's cooldown marker touched before the query loses mail for 60 s windows (agmsg §6.3).
- Verdict: the mod already delivers at idle through `turn.complete → pump`; only relevant to the hooks-only stack, where D8/D9 are cheaper.

**D8. Body at `UserPromptSubmit` (hcom, Cotal managed).**
- \+ Tok: a typed or submitted marker turns into a full delivery with no `inbox` round trip. Our hooks-only wake types `run herdr-threads inbox` because no turn-start hook injects (ours §3.1 table, `policy.rs:14`), costing at least two model requests per wake (ours §5).
- \+ It also covers a human's own next prompt.
- − One more hook process per prompt (cheap). Concurrent hooks can double-deliver before commit (hcom §2.8).
- Verdict: adopt for the hooks-only stack; it lets `wake_text` become `WAKE_UP` (`policy.rs:17`) under the existing adapter contract.

**D9. `PostToolUse` on every tool (hcom).**
- \+ Lat: hooks-only mid-turn delivery after Read/Edit/MCP calls, not only Bash. Ours is Bash-only (`src/harness/claude.rs:801-802`), the same blind spot mcp_agent_mail has (mcp-agent-mail "Push").
- − One hook process per tool call (our Bash path already pays one; the live-channel early exit keeps it to zero bytes, ours §2.3).
- Verdict: adopt (matcher change), with the same live-channel early exit.

**D10. Body pulled by a tool call after a hint.**
- \+ Tok on huge bodies: the model reads only what it asks for.
- − Tok/Lat: at least one extra model step per wake; deferred MCP tools may also need ToolSearch (agentchattr "Latency"). Compliance depends on the model (agmsgd notice "never repeated once confirmed", agmsg §3.3; mcp_agent_mail's reminder names the one call that does not silence it, §Limitations 5).
- Verdict: ours (in-band, bounded, continuation for the rest) is better. This confirms AGENTS.md rule 3.

**D11. Every message wakes.**
- \+ Lat for the recipient of every message.
- − Tok: every message costs a turn; broadcast to N idle sessions starts N turns (hcom §6.6); thundering herd (hcom `wake_all`, §6.7). ocs had to add an anti-ack line after an ack storm (ocs §2.4, issue #23).
- Verdict: our threshold model is the main advantage we have over every alternative. Keep.

**D12. No draft guard (hcom native, Cotal, agentchattr, CAO, xats) vs Cyclops' guard that never expires.**
- \+ (no guard) Lat: no 120 s delay. `$.prompt.submit` queues a separate turn and does not write the composer (primer §2.3), so the draft text itself is INFERRED to survive.
- − (no guard) Draft: a turn starts while the person is mid-sentence (cotal §6.3, hcom §6.5); typing tools (agentchattr, CAO) submit the draft with the wake (agentchattr "Draft safety", CAO §3.3).
- Cyclops: holds on positive evidence only and releases on a pane event, never a timer, with a 120 s admin ping (cyclops §3.3). Ours expires after 120 s and wakes anyway (`register.js:542-546`).
- Neither our expiry behaviour nor "plugin turn under a draft" has been captured live: the mod evidence lists "the draft hold" under **Not exercised** (`docs/evidence/claude-mod-delivery/README.md:202-208`, VERIFIED). Nobody else has captured it either.
- Verdict: our design is the best of the set, but its key property is untested.

**D13. SGR-dim draft detection (agmsg, hcom, Cyclops).**
- \+ Draft/Rel: tells a dim prompt suggestion from a typed draft, so a fallback typing wake can fire with suggestions on. Ours reads Herdr detection text, which cannot, so Claude composer text is always `Unsafe` (`src/harness/claude/composer.rs:83-91`; native read is `herdr agent read <pane> --source detection` with no format flag, `src/host/native.rs:2117`).
- \+ Setup: removes the need to ask users to turn `promptSuggestionEnabled` off (ours §4).
- Herdr already offers it: `herdr agent read --format ansi` and `herdr pane read --format ansi` exist in the installed Herdr 0.9.5 (VERIFIED, `--help`), and agmsg uses `herdr pane read --format ansi` (agmsg §3.4).
- − Ver: SGR conventions are a screen contract; Cyclops' rules stop at 2.1.261 and note the ghost styling is "not yet captured live" (cyclops §3.3). Fails safe if a ghost is mis-read as a draft (a held wake, not lost input).
- Verdict: high value, low cost. Top recommendation for the fallback.

**D14. Pause until a person-origin prompt after abort (Cotal).**
- \+ Trust: after Esc, no peer, timer or plugin can restart work; resumes only on `origin.kind ∈ {composer, bridge, sdk}` (cotal §3.2).
- − Lat: a seat whose user walked away after Esc never wakes again. Ours releases after 120 s with no open turn and an empty box (`register.js:891-899`), which fits rule 2 (an idle session wakes on a p ≤ T item).
- Our trigger is broader: any last main tool result with `isError === true` counts as a reject (`register.js:121`), which may treat an ordinary failing tool as an abort (ours §6.1). Cotal keys on `isAborted` only (cotal §3.2).
- Verdict: keep the timed release; narrow the trigger (see §4 F6).

**D15. Host-confirmed receipts.**
- \+ Rel: Cotal records the stored row uuid and an exact text match (cotal §2.3); agmsgd scans the transcript for its nonce and tells "new turn" from "absorbed mid-turn" from "held" (agmsg §2.3); ocs gets native receipts (ocs §2.6); Cyclops matches the submitted prompt exactly in `UserPromptSubmit` (cyclops §2.6).
- Ours settles on the engine's answer to `append` without reading its uuid: the result is checked only for `missing`/`deny` (`register.js:570-592`, `:655-666`), and `context` is ACKed once the hook returns (`register.js:440`).
- − Ver: text matching and transcript scanning are fragile (Cotal pauses delivery if the match fails, cotal §6.7). A uuid check is cheap and not text-dependent.
- Verdict: low-cost hardening; adopt the uuid half, not the text match.

**D16. `asUser: true` (hcom).** + Tok: drops the plugin frame (size unknown). − Trust: the model reads the wake as the user's own words. Our plugin attribution is the right call under rule (e). Keep ours.

**D17. Refuse empty wakes (hcom).** Our wake is submitted only after rows were appended, and a main `turn.start` clears a pending wake (`register.js:593`, `:792-793`), so an empty `wake up` should not occur on the mod path (INFERRED from code). Relevant only if the hooks-only wake gains a `UserPromptSubmit` delivery (D8): then a marker that finds nothing should be blocked with `hookSpecificOutput.suppressOriginalPrompt` (hcom §2.3, noting hcom puts the field in the wrong place).

**D18. Durable wake claim keyed on the cursor (hcom).** Our pending wake already lives in `$.state` across reloads (`persistTurns`, `register.js:593-594`; `turns.wake`/`turns.submitting` protocol, ours §3.3). No gap.

**D19. Idle oracle from Claude's session registry (ocs).**
- \+ Rel: Claude writes `<CLAUDE_CONFIG_DIR>/sessions/<pid>.json` with `status`, `statusUpdatedAt`, `sessionId`, `messagingSocketPath`, `peerFeatures` (VERIFIED by a read-only local read of this session's file: keys include `status` = `busy` while this report ran, and `peerFeatures` = `notify_idle, reply_across_default_dirs, artifact_yield`). One file read, no spawn, no screen scraping.
- − Ver: undocumented file; must be a hint, never authority (TRUST-POLICY A2). ocs ignores `CLAUDE_CONFIG_DIR` and so misses such sessions (ocs §6.1); we already resolve `CLAUDE_CONFIG_DIR` (`src/harness/claude.rs:1041`).
- Screen regexes (CAO, Cyclops) are the opposite end: per-release patch trails (CAO §6.4; Cyclops' 12-hour-clock regex, cyclops §3.5).
- Verdict: a cheap second opinion for the native fallback's idle gate.

**D20. Per-run `--settings` / `--plugin-dir` (hcom, Cyclops).** + Setup: no persistent user-config edits; concurrent launches. − Only works for sessions the tool launches; our Herdr model has users start `claude` themselves, so persistent install is required. Keep ours; our manifests and backups already bound the footprint (ours §4).

**D21. Marketplace install (Cotal, agent-talk, ruflo).** + Setup: Claude owns enablement; survives `disableSideloadFlags` (INFERRED, the flag targets `CLAUDE_CODE_PLUGIN_DIRS`). − Plugin updates go through Claude's cache; our hot-reload-by-rewrite (ours §4) depends on a plugin dir we own. Worth a note as the escape hatch for managed environments that refuse sideloading.

**D22. Owning the PTY (hcom, agentchattr, Cyclops, CAO).** + Lat/Rel: full screen state, idle-edge forwarding, Esc recovery (hcom §3.1). − Setup: users must launch through the tool. Herdr owns the pane for us. Not applicable.

**D23. Cross-machine.** Cotal (mesh), agent-talk (relay), mcp_agent_mail and xats (HTTP) cross machines; we are local, and Claude's native socket is same-machine (primer §3). Out of scope for injection; noted for completeness.

**D24. Per-delivery static text.** − Tok: Cotal's 205 B tail per block and 186 B inline trailer (cotal §5), ocs's anti-ack and reply lines (ocs §5), agent-peers' 430 B header (agent-peers "Framing"). Ours is one header line per item (R17). Keep ours. Cotal's tail exists because a weak model "confirmed" work it had not done (cotal §6.12); that rule belongs once in our skill.

**D25. Model-carried identity.** − Rel: agent-talk's model must copy its session id (agent-talk §6.5); xats binds by a `ps` walk plus model-supplied `$PPID` (cross-agent-teams §3.5.4). Our daemon decides bindings from hook and claim evidence (TRUST-POLICY). Keep ours.

**D26. No automatic fallback.** − Rel for hcom (mail waits until a human types, hcom §6.11) and agmsgd (no Monitor until the next SessionStart, agmsg §6.8). Ours falls back to native wake when no channel is live or after the stall handover (ours §3.3). Keep ours; shorten the handover (F9).

**D27. Health as status with the remedy (hcom).** + Rel/Setup: `tui:mod-unavailable|failed|unacknowledged|waiting`, each with the fix (hcom §3.2). We report mod presence in `setup-status` (`MOD_MIN_VERSION`, ours §1); a per-seat runtime state (inert, stalled, held on draft/abort) visible to the user is not verified here (unknown).

**D28. Whole messages only (Cotal).** + Trust: never shows a partial body as complete. Ours truncates at 8 KiB with an explicit `…truncated` marker and a continuation (`register.js:93-99`), which is honest and saves the round trip for the first 8 KiB. Keep ours.

**D29. Body neutralisation (ocs, ruflo).** Ours indents every non-empty body line by two spaces, splitting on `\r\n`, `\n`, `\r`, `\v`, `\f`, U+0085, U+2028, U+2029 (`register.js:30-32`; Rust port `src/delivery/frame.rs:56-70`), and flattens controls in ids and names (`register.js:29`, `frame.rs:46`). A body line can therefore never start at column 0, which is stronger than ocs's prefix list. No gap (VERIFIED).

---

## 4. Flaws in our approach that the others expose

Severity is for agents on Claude in Herdr panes.

| # | Flaw | Evidence (ours) | Exposed by | Severity |
|---|---|---|---|---|
| F1 | The no-mod fallback is close to useless on default Claude: any composer text, including the default-on prompt suggestion, reads `Unsafe`, so most idle panes refuse the wake. Herdr can already return styled text, and we do not ask for it | `src/harness/claude/composer.rs:83-91`; `src/host/native.rs:2117` (`--source detection`, no `--format`); `herdr agent read --help` lists `--format text\|ansi` (Herdr 0.9.5) | agmsg `safe-poke` dim-span test via `herdr pane read --format ansi` (agmsg §3.4); hcom `screen.rs:688-730`; Cyclops `claude.toml:214-303` | high |
| F2 | Hooks-only idle wake costs an extra `inbox` tool round trip, because Claude has no turn-start hook from us, so the typed marker must name `inbox` | `src/harness/claude.rs:793-812` (4 hook groups, no `UserPromptSubmit`); `src/notification/policy.rs:14`; ours §5 | hcom and Cotal managed deliver at `UserPromptSubmit` (hcom §2.3; cotal §2.1) | medium |
| F3 | Hooks-only mid-turn delivery reaches only Bash calls | `src/harness/claude.rs:801-802` (`"matcher":"Bash"`) | hcom `PostToolUse` unmatched (hcom §2.2) | medium |
| F4 | Busy **waking** items wait for an answered main tool result; a long `Agent` call or tool-free generation delays ACK requests and p0 items until turn end, although `session.append` would reach the next model request | `register.js:825-837` (only `tool.call` attaches waking items); passive items use append at any time `:635-669` | Cotal busy path is append after `tool.call` (cotal §2.3); native socket delivers between tool calls (primer §3) | medium |
| F5 | The draft-hold behaviour, including the "wake anyway after 120 s" branch and what a plugin turn does to a typed draft, has never been exercised live | `register.js:536-546`; `docs/evidence/claude-mod-delivery/README.md:202-208` ("Not exercised … the draft hold") | Cyclops' event-released hold (cyclops §3.3) shows the alternative policy; nobody has a capture, which makes ours the claim to prove | medium |
| F6 | Post-abort hold triggers on any `isError === true` last main tool result, so an ordinary failing tool may hold wakes up to 120 s | `register.js:116-121`, `:818-820` | Cotal keys on `isAborted` and person-origin resumption (cotal §3.2) | low-medium |
| F7 | Receipts settle on the engine's answer without checking the stored row; `context` is ACKed when the hook returns | `register.js:440`, `:570-592`, `:655-666` | Cotal uuid + text check (cotal §2.3); agmsgd nonce (agmsg §2.3) | low |
| F8 | The native fallback's idle gate depends on Herdr `agent_status` and screen text, while Claude publishes its own `busy/idle` in the session registry | ours §3.1 table; local registry read (§3 D19) | ocs (ocs §3, §7.1) | low |
| F9 | Stall handover takes 10 + 10 minutes before native wake takes over, and an attention-only seat never trips it | ours §6.9 (`src/protocol/watch.rs:75-77`, design D7) | hcom shows `mod-unacknowledged` after 10 s (hcom §3.4) — it does not fall back, but it detects much sooner | low |
| F10 | Stale docs: `integrations/claude/README.md` still says unlisted versions are refused | ours §1, §6.11 | — (our own) | low |

What the others do **not** expose: framing injection (D29 covered), unbounded payloads (we are bounded where every other in-band tool except Cotal inline and ocs is not), every-message wakes (D11), model-carried identity (D25), persistent wake claim (D18).

---

## 5. Better integration ways, ranked by value / cost

Value is for Claude agents in Herdr panes. Cost counts code, captures and version risk. All items keep the harness-neutral rule: Claude-specific code stays under `src/harness/claude*` and the mod.

| Rank | Change | Value | Cost | Basis |
|---|---|---|---|---|
| 1 | **Styled composer read for Claude.** Read the pane with `--format ansi`; classify dim-only text after `❯` as a suggestion (`Empty`), unstyled text as `Text`/draft, anything else `Unsafe`. Capture the 2.1.30x SGR bytes first | high: makes the fallback wake work with suggestions on; drops the setup question | low: one adapter `ComposerPolicy` path plus a capture; fails safe | F1; agmsg, hcom, Cyclops; Herdr `--format ansi` VERIFIED |
| 2 | **Hooks-only: add `UserPromptSubmit` delivery and switch the fallback marker to `WAKE_UP`.** The hook injects the page at turn start; the typed wake becomes content-free (R18); an empty marker is blocked with `hookSpecificOutput.suppressOriginalPrompt` | medium-high: one model request per wake instead of two; also serves the human's own prompts | low: one hook group, existing check-in code, `wake_text` override | F2; hcom, Cotal managed |
| 3 | **Hooks-only: widen the tool matcher** (`PostToolUse` or `PreToolUse` on all tools), keeping the live-channel zero-byte early exit | medium | low | F3; hcom |
| 4 | **Mod: append waking items when no answered tool result arrives within a bound** (for example while an `Agent` call runs), never in the final step | medium | low-medium: a timer path plus a test; ACK `via append` already exists | F4; Cotal |
| 5 | **Capture the draft and abort behaviour live** (plugin `prompt.submit` while the box holds text; the 120 s expiry; a failing Bash result's `isError`) and narrow `userRejected` to what the capture shows | medium: turns two INFERRED properties into evidence | low-medium: a spike like `submit-race-spike.md` | F5, F6 |
| 6 | **`asyncRewake` waiter as the hooks-only idle wake.** A `Stop` hook with `asyncRewake: true` blocks on the daemon (threshold-filtered, busy-gated by `wake_frontier`), exits 2 with a content-free marker; delivery follows through rank 2 | medium-high: wakes without typing, where mods are refused | medium: re-arm logic, timeout, capture of mid-turn and draft behaviour (primer §10 Q2) | agent-peers; primer §1.4 |
| 7 | **Record the `session.append` uuid** in the ACK path (and treat a missing uuid as a refusal to settle) | low-medium | low | F7; Cotal |
| 8 | **Session registry as a busy/idle hint** for the native fallback (`$CLAUDE_CONFIG_DIR/sessions/<pid>.json` `status`, pinned by `sessionId`) | low-medium | low; hint only (A2) | F8; ocs |
| 9 | **Native cross-session socket from the mod-spawned `watch` child** (own-child exemption) as a delivery path when the mod's APIs are missing | potentially high, overlaps ranks 1-6 | high: undocumented schema, own capture, framing loss, silent breakage | D3; agmsgd, ocs |
| 10 | **Faster stall detection** (seconds, like hcom's 10 s unacknowledged state) with a visible status, before the 10-minute handover | low-medium | medium (touches design D7) | F9; hcom |
| 11 | **`ui.render` collapse of our `wake up` rows** into one line keyed by recorded request id | low (UX) | low | Cotal §7.5 |
| 12 | Marketplace install as an escape hatch for managed `disableSideloadFlags` | low (rare environment) | medium (update path) | D21 |

Not recommended: channels (D4), Monitor-tool watches (D6), Stop block loops for interactive sessions (D7), every-message wakes (D11), `asUser: true` (D16), bodies in the wake prompt (D2), tool-call pulls after hints (D10), model-carried identity (D25).

---

## 6. Primer surfaces nobody uses

Checked against all thirteen reports (twelve alternatives plus ours). "Used" means used for agent-to-agent injection or wake.

| Surface (primer §) | Used by | Note |
|---|---|---|
| `claude agents --json` `status` (§6) | nobody | Supported external busy/idle probe; cost per call unknown (primer §10 Q5). The registry file (D19) is a cheaper unofficial cousin |
| OSC 7501 program status (§7) | nobody | Not reported inside tmux; Herdr panes are not tmux, so it may be observable by Herdr (unknown) |
| `$.session.send` from a mod (§2.3) | nobody | "The same delivery the SendMessage tool makes", to a session id. A mod could deliver to a **peer** session natively; ruflo calls the `SendMessage` tool instead and leaves cross-session unverified (ruflo §2.1) |
| Own-child exemption on the messaging socket (§3) | nobody | agmsgd and ocs post from non-child processes and fight the inbound policy instead (D3) |
| `notify_when_idle` (§3) | nobody natively | ocs polls the registry itself (ocs §3) |
| `session.receive` to annotate native deliveries (§2.2) | ruflo, only to drop | Could add our `[pN]` or thread markers to native peer messages |
| `prompt.context` first-message blocks (§2.2) | nobody | A place for the check-in on the mod path instead of `SessionStart` (INFERRED) |
| `prompt.compose` / `prompt.section` (§2.2) | ruflo (mission text, not mail) | A cache-stable standing digest is possible; per-message use would break R17 |
| `prompt.suggest` control (§2.4) | nobody | The mod could suppress the dim suggestion while mail is pending, which would also clean the fallback's screen read (INFERRED) |
| `Notification idle_prompt` (§1.5) | hcom only (status) | 60 s late; not a wake |
| `TeammateIdle` exit 2 (§1.5) | nobody (ruflo believes it invalid, ruflo §3.1) | Agent-teams only |
| `FileChanged` + `watchPaths` (§1.5) | nobody | Injection ability unknown |
| MCP tasks extension (§5) | nobody | "When a task ends the result goes to Claude": a possible MCP-side wake, uncaptured |
| `SessionStart` `initialUserMessage` (§1.2) | nobody | `-p` only |
| `PostToolBatch` (§1.1) | Cotal registers it, injects nothing | A once-per-batch alternative to per-tool hooks for rank 3 |
| Cron / `ScheduleWakeup` (§8) | agent-talk (model-created backstop) | Model-created; unfit for a plugin |

---

## 7. Bottom line

- The mod path is the strongest Claude integration in the set. No alternative combines in-band bounded delivery, a non-waking class, a draft hold and an automatic fallback. Cotal personal is the closest; it lacks the threshold and the draft hold and repeats ~0.5 KB of static text per batch (cotal §5).
- Our weak side is the **fallback** (no mod): an always-`Unsafe` composer (F1), an `inbox` round trip per wake (F2) and Bash-only mid-turn delivery (F3). Ranks 1-3 fix all three with small, Claude-adapter-local changes, and rank 6 adds a typing-free wake on documented machinery.
- Two of our mod properties are still unproven claims rather than evidence: what a plugin turn does to a human's draft, and whether ordinary tool errors trip the post-abort hold (F5, F6). Rank 5 settles both.

---

## 8. Verification

Re-read in herdr-threads at `2de3da1c4` for this report (VERIFIED):

- `integrations/claude/mod/hooks/register.js`: `WAKE_PROMPT = 'wake up'` (`:12`); `DRAFT_WAIT_MS` (`:16`); batch caps (`:23-24`); `oneLine`, `LINE_BREAK`, `indent` (`:29-32`); `frame` (`:58-65`); truncation continuation (`:93-99`); `userRejected` (`:116-121`); `complete` and the `context` ACK (`:403-440`); draft hold and expiry (`:536-546`); `deliverIdle` append → wake, result checked only for `missing`/`deny` (`:561-628`); `passive` (`:631-632`); `pumpLazy` (`:635-669`); `turn.start` clears the wake (`:792-793`); abort hold (`:818-820`, `:891-899`); `onToolCall` "answered" rule (`:825-837`); io wiring with no `asUser` and no uuid read (`:986-989`); no `turn.step`, `ui.render` or `prompt.submit` hook registered (`:1054-1100`).
- `src/harness/claude.rs:793-812` (four hook groups, Bash matcher); `src/harness/claude/composer.rs:83-91` (`Text` → `Unsafe`); `src/host/native.rs:71` (`FOCUSED_EMPTY_MILLIS`), `:2117` (`agent read … --source detection`); `src/notification/dispatch.rs:260-265` (HumanInput/ActiveTurn refused; Blocked refused per test at `native.rs:5789-5792`); `src/notification/policy.rs:14-19` (`MARKER`, `WAKE_UP`, `CHANNEL_WAKE_UP`); `src/delivery/frame.rs:46`, `:56-70` (line-break set); `src/delivery/ledger.rs:22-24`; `CLAUDE_CONFIG_DIR` handling in `src/harness/claude.rs:1041`.
- Grep of `src/` and `integrations/` for `asyncRewake`, `CLAUDE_CODE_MESSAGING_SOCKET`, `UserPromptSubmit`, `PostToolUse`, `claude agents`, `monitors.json`, `ui.render`, `session.receive`, `suppressOriginalPrompt`: hits only in Codex/agy code and a generic event classifier (`src/daemon/harness_evidence.rs:193`), none in the Claude path.
- `docs/evidence/claude-mod-delivery/README.md:202-208` lists the draft hold as not exercised; `submit-race-spike.md` shows a queued plugin prompt survives Esc on 2.1.295.
- Local, read-only: `herdr --version` 0.9.5; `herdr agent read --help` and `herdr pane read --help` list `--format text|ansi`; this session's `$CLAUDE_CONFIG_DIR/sessions/<pid>.json` has `status`, `statusUpdatedAt`, `sessionId`, `messagingSocketPath`, `peerProtocol`, `peerFeatures`. Nothing was written.

Not verified here: any alternative claim beyond its own report's citations; live behaviour of `asyncRewake`, channels, plugin monitors, the native socket, or styled composer bytes on 2.1.309.
