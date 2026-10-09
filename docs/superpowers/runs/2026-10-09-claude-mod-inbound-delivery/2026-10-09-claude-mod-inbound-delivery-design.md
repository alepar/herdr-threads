# Claude Code mod inbound delivery

## Goal

A Claude Code session in a Herdr pane receives its herdr-threads messages from a bundled Claude Code mod, without a terminal send-keys nudge:
- between tool calls while a turn runs;
- as a new turn when the session is idle;
- passively for lazy messages.

Each fully delivered message settles its receipt. Whenever the mod is not connected, delivery falls back to today's hooks plus send-keys wake, losing nothing.

Mode B (one-shot), autonomous super-auto run. Inputs:
- spike: [`docs/research/claude-mod-delivery-spike/README.md`](../../../research/claude-mod-delivery-spike/README.md)
- research: `~/Documents/Claude_Code_Mods_Delivery_Research_20261008/report.md`
- normative: [TRUST-POLICY.md](../../../../TRUST-POLICY.md)

## Problem description

Today's inbound path relies on observation from outside the session. The daemon decides from sampled Herdr status and composer reads whether a Claude pane is idle. It then types a fixed marker into the pane (`herdr-threads: attention pending; run herdr-threads inbox`). The agent must run `inbox`, read and ACK.

This is chatty and racy:
- **Misjudged idleness.** Between turns the pane can look idle while Claude Code is still in its turn.
- **Composer guards defer delivery indefinitely.** Prompt suggestions and drafts keep the composer from reading empty.
- **Every message costs extra tool calls.** Each one needs a wake, an inbox call and an ACK.

Claude Code 2.1.287 and later loads *mods*: in-process JavaScript plugins with a typed API. The spike (2.1.294) showed a mod can deliver text in three ways:
- **Mid-turn:** attached after a tool result, as PostToolUse-style context.
- **On idle:** as a new turn (`$.prompt.submit`), which the engine queues until it is idle.
- **Passively:** as a hidden user row (`$.session.append`) that starts no turn.

The spike also showed that a mod can:
- read the pane environment;
- run a child process for the session's life;
- survive `/clear` and `/resume`.

## Main challenges

1. **A push channel.** The daemon has no streaming request today: every request is one-shot. The only persistent connection is the service connection, which is request/response. A connection lost to a crash, the remote kill switch or a reload must stop mod routing at once.
2. **Attribution.** "Delivered into the model's context" has to be recorded honestly as a new cooperative claim. TRUST-POLICY lists provenances exhaustively, and the 2026-10 user direction forbids blanket read or delivery auto-ACK.
3. **Turn-boundary races found by the spike:**
   - A submit queued during a busy turn takes over right after the user presses Esc.
   - The next turn's `turn.start` can arrive before the aborted turn's `turn.complete`.
   - A crash of the shared hooks worker unloads the mod, and messages in flight are lost.
4. **Fallback without loss or duplication.** It must work across reconnects, `/clear`, `/resume`, reloads and Claude versions without mods.
5. **Installing a plugin** into the user's Claude configuration through the existing owned-settings machinery, reversibly.

## Key decisions made

- **Mod and stream.** A bundled mod runs a session-long `herdr-threads watch` child. `watch` holds a new persistent *watch connection* to the daemon. The connection is a registered delivery channel for the seat's current binding and doubles as the liveness signal.
- **Notify, then fetch.**
  - The daemon pushes only small "attention changed" frames.
  - `watch` fetches bounded pages of message bodies through existing read paths, then writes them to stdout as JSON lines.
  - The mod delivers them and reports each delivery through `herdr-threads watch ack`. The daemon then settles ordinary receipts under a new action provenance, `cooperative_mod_delivery`, and lazy rows as display-complete.
- **Routing.** While a live watch registration exists for the seat's current binding generation, the daemon suppresses native wake prompts, pokes and tool-boundary hook digests for that seat. Disconnecting kicks the normal wake lane.
- **Cursor.** The unacked cursor is the existing pending receipt and lazy state. A reconnect re-streams everything still pending, and the mod de-duplicates by message id.
- **Hooks stay.** The settings hooks remain for enrollment and lifecycle check-in.
- **Install.** The mod's files are embedded in the binary, written into the state directory by `setup claude`, and loaded through `env.CLAUDE_CODE_PLUGIN_DIRS` in the user's Claude settings, owned by the existing setup manifest.

## Decision points, by section

### D1. Delivery channel: mod plus `watch` child

**Chosen:** a Claude Code mod (`integrations/claude/mod/`, plugin name `herdr-threads`) that, at `session.start`, spawns `herdr-threads watch --harness claude --session <id>` with `$.process.spawn`, for the session's life.

`$.process.spawn` is the documented session-long child pattern. The child inherits the pane environment (`HERDR_PANE_ID`, state dir), so it resolves the seat exactly as hooks do. The hooks module itself has no file or socket access, so the child carries the daemon's Unix socket.

**Considered:**
- The cross-session inbox socket (`CLAUDE_CODE_MESSAGING_SOCKET`): its frame format is undocumented, it holds messages for bypass-mode seats, and it gives no receipt point.
- MCP channels: preview, allowlisted, and broken on the v2 MCP runtime.
- Polling `herdr-threads inbox` from a mod timer, as fleet does: no push, 1–3 s latency, a process per poll.

### D2. Daemon push: a watch connection kind

**Chosen:** `serve_connection` recognises a new first frame, `WatchRequest`, just as it already recognises the service frame. It carries:
- the ordinary envelope fields (version, expected instance and boot);
- the caller claim the CLI builds for this pane (`CallerClaim`, harness `claude`, native session id).

The daemon decides registration in one transaction against A2:
- The seat is resolved and not held.
- An open binding with provenance `cooperative_top_level` exists.
- Its harness is `claude`.
- Its native session equals the claim's.

On success it records a process-local registration `(seat, binding_generation) → connection`. It then answers `WatchAccepted{attention_version}` and from then on pushes `WatchFrame` values:
- `Attention{version}` when the seat's `attention_version` advances or lazy rows are published for it;
- `Close{reason}` when the binding is replaced or ended, the seat is retired or unresolved, or the daemon is stopping, after which the daemon closes the stream.

The registration is held in memory only. Dropping the connection removes it, and a daemon restart drops all of them; the clients reconnect. At most one registration per seat: a second watch for the same seat and generation replaces the first (the old stream gets `Close{replaced}`). A capability string, `MOD_WATCH`, gates the feature, and `PROTOCOL_VERSION` stays at 6.

**Considered:**
- Polling with `HistoryRange::After` (the `follow` precedent): no liveness, and it adds latency.
- Reusing the service connection: wrong identity class. Service authors cannot be receipt recipients (A5).
- Pushing full bodies in frames: it duplicates the read paths and their bounds, and makes a slow mod back-pressure the daemon.

### D3. Seat identity and check-in

`watch` neither enrolls, allocates nor rebinds. The settings `SessionStart` hook still does lifecycle check-in (C1/C2, "Startup enrollment").

`watch` claims the existing binding: pane from `HERDR_PANE_ID`, native session id from `--session`, which the mod passes from `$.session.id()`. A mismatch is refused with a typed reason (`no_binding`, `session_mismatch`, `not_claude`, `held`, `unresolved`):
- `watch` exits non-zero with one JSON status line.
- The mod retries with backoff: 1, 2, 5, 10, then every 30 s.

That covers the race where the mod starts before the SessionStart hook's check-in commits.

The registration is recorded as a channel claim with a new provenance, `cooperative_mod_channel`. It means the pane's top-level session, through its mod, opened a delivery channel. It grants nothing except receiving deliveries and making `cooperative_mod_delivery` claims for that binding generation.

**Considered:**
- Letting the watch connection itself perform a Lifecycle check-in: that duplicates enrollment and moves continuity decisions into a second path.
- Trusting `HERDR_PANE_ID` alone without matching the session: it would let a stale mod from a replaced session hold the channel.

### D4. What is delivered

`watch` streams one JSON object per line on stdout (`schema: 1`), each carrying a stable `id` and a `kind`:

| kind | Content | Source |
|---|---|---|
| `message` | thread id and name, sender, `author_role`, `relays_user`, `user_intent`, the full body or a truncated marker, whether an ACK is required | pending receipts, via `InboxBatch`-style bounded pages |
| `lazy` | the same fields, for lazy rows | `lazy_delivery::pending_page` |
| `attention` | the existing fixed marker (invitations, warnings, deadlines, notices, anything without a body) | the attention digest |
| `status` | connected, refused or closing, with a reason | watch lifecycle |

Bodies over a per-message limit (8 KiB) are delivered as their first 8 KiB plus `…truncated; run herdr-threads body <id>`, and are never ACKed by the mod. Each page is bounded (at most 32 items or 64 KiB), with paging until drained after every `Attention` frame.

The mod frames all peer text as untrusted data, matching the hook's `untrusted_peer_data` convention:
- a fixed instruction header naming herdr-threads;
- one block per item: `[herdr-threads] <kind> <id> in <thread> from <sender>:` followed by the body.

Text never starts with `/`, which `$.prompt.submit` refuses.

**Considered:** delivering only the marker, as the native wake does. That keeps every tool call and loses the main benefit.

### D5. Mod delivery state machine

Busy and idle come from engine events, keyed by turn id: a set of open main-conversation turn ids. Events carrying `agentId` are ignored.

| Item | While busy (main turn open) | While idle |
|---|---|---|
| `message`, `attention` | Attach to the next main-conversation `tool.call` result as `context`. If the turn ends first, deliver after it per the idle rule. | `$.prompt.submit` of one batched, framed text. Never awaited inside a hook. Never called while any main turn is open. |
| `lazy` | `$.session.append` (user-role hidden row) at once | `$.session.append` at once; it starts no turn |

Rules:
- **Hold after interrupt.** After a `turn.complete` with `isAborted`, idle submits are held until a later non-aborted main turn completes, or 120 s pass with no open turn and an empty prompt box (`$.prompt.read()`). Context and append are unaffected.
- **One submit in flight.** Items arriving while a submit is pending wait for the next delivery opportunity.
- **De-duplication.** A per-session set of delivered and acked ids, kept in `$.store` keyed by session id, so a module reload doesn't re-deliver.
- **Visibility.** Each context or append delivery writes one dim `$.ui.log` line (`herdr-threads: delivered N message(s)`), because those paths are invisible in the TUI.
- **Restart.** When the `watch` child exits, or on `session.end` (clear, resume), the mod stops the child, re-reads `$.session.id()` and restarts `watch` with backoff. `session.start` runs again after a reload and starts it fresh.

**Considered:**
- Submitting whenever a message arrives and letting the engine queue it: this produced the Esc takeover in the spike.
- A single busy flag: broken by the event ordering the spike observed.
- Delivering ordinary messages through append: no wake for an idle agent.

### D6. Receipts: `watch ack` and `cooperative_mod_delivery`

When a delivery resolves (context returned from the hook chain, submit resolved, append resolved), the mod runs `herdr-threads watch ack --session <id> --via context|submit|append <ids...>` through `$.process.run`.

The daemon decides `AckModDelivered` against A2:
- the current binding and generation, with provenance `cooperative_top_level`;
- a live watch registration for that generation.

It then:
- **Ordinary pending receipts:** settles them exactly as `AckDisplayed` does, with `ack_observation.action_provenance = "cooperative_mod_delivery"`. The binding's `cooperative_top_level` provenance is kept separately.
- **Lazy ids:** completes them as displayed (A8), with the same claim.
- Refuses anything else: unknown ids; truncated ids (decided from the stored body length against the 8 KiB limit, never from a client hint); ids not addressed to the seat; a stale generation. The refused items stay pending.

Only items the mod delivered in full may be acked; `watch ack` refuses ids the stream marked truncated.

This is a user-approved change to the 2026-10 "no read/delivery auto-ACK" direction. In this session the user stated delivery is the receipt for now ("we already judged delivery is the receipt"). The precedent is `cooperative_inbox_display`. The claim is "the full body entered the model's context through the mod", which is not a proof that the model read it.

**Considered:**
- Reusing `cooperative_inbox_display`: it means "the text inbox command flushed a page", a different action.
- Letting the agent ACK explicitly after a mod delivery: it keeps the tool-call overhead the goal removes.

### D7. Routing and fallback

**The registry.** A process-local `ModChannels` service holds the registry. Three consumers check it:
- the wake dispatcher, before reserving a native prompt or poke (`NativeWakeDispatcher::attempt` and `can_reserve_poke`);
- the tool-boundary hook result, which omits its attention digest when a channel is live;
- the scheduler's attention kick, which also notifies channels.

**While a channel is live:**
- No native prompt or poke is sent.
- Pending attention stays pending.
- Deadlines and hard-deadline warnings keep running. Warnings reach the agent as `attention` items through the mod.

**On disconnect, or `Close` for any reason:**
- The registry entry is removed.
- The wake lane is kicked for that seat, so the existing ladder resumes at once from current pending state.
- Nothing is marked delivered by the disconnect.

A message streamed but never acked stays pending and is re-streamed on reconnect, or reaches the agent through the native wake.

A channel counts as stalled when it is live, its last mod ack (or its registration, if it has none) is more than 10 minutes old, and some ordinary pending receipt for the seat was published at or before the last pushed `Attention` frame and is itself more than 10 minutes old. The registry (D2) owns this state. For example, the mod is holding after an interrupt and the user has walked away. The daemon then lets the native wake ladder run for that seat, with the existing composer guards, while keeping the channel. Any later mod ack clears the stall.

**Handoff.** An item the mod delivered but whose ack did not land before the channel dropped stays pending. The mod keeps delivered-but-unacked ids in `$.store` and re-acks them after re-registering, instead of delivering them again. If it never re-registers, the agent may see the item again through `inbox` or the native wake. That is at-least-once across a handoff, and never lost (an accepted limit).

**Operator switch.** `HERDR_THREADS_MOD_DELIVERY=off` makes `watch` exit 3 without registering, so delivery stays on hooks plus wake. `setup-status` reports it.

**Considered:**
- Hard suppression with no stall bound: one stuck mod would silence a seat forever.
- A per-message hand-off timer: needless complexity.

### D8. Install, upgrade, uninstall

The mod's files (`.claude-plugin/plugin.json`, `hooks/hooks.json`, `hooks/register.js`) are embedded with `include_str!` and written by `setup claude` to `<state>/claude-mod/herdr-threads/`, versioned with the binary. Rewriting them on upgrade is safe, because Claude Code reloads plugin directories.

The user's Claude `settings.json` gains `env.CLAUDE_CODE_PLUGIN_DIRS`:
- the existing value, if any, is preserved, and the mod directory is appended with `:`;
- the change is recorded in the owned setup manifest, the same fingerprinted ownership as the hook groups;
- `unsetup` removes only the appended path and the files.

`setup-status` reports:
- whether the mod is installed;
- whether `claude --version` is at least 2.1.287 (older versions simply never load the mod, which is the fallback);
- whether a channel is currently live for any Claude seat.

Tests use an isolated `HOME` and `CLAUDE_CONFIG_DIR`.

**Considered:**
- `claude plugin install` from a local marketplace (the documented route): it writes Claude's plugin state outside our manifest, and requires the `claude` CLI at setup time.
- Asking users to pass `--plugin-dir`: not automatic.

### D9. Trust policy

Amend TRUST-POLICY.md in the bead that introduces the constants:
- **A3:** add `cooperative_mod_channel` (channel registrations only) and `cooperative_mod_delivery` (receipt action observation; lazy completion).
- **A4:** a live channel replaces native wake and poke for its seat, and the stall bound applies.
- **A5:** rows for opening a channel and for mod delivery ACK.
- **A8:** the mod as a second lazy completion source.
- **Accepted limits:**
  - the mod claim is cooperative: a same-user process could run `watch ack`;
  - engine-reported idleness is trusted;
  - a remote mods kill switch silently returns seats to native wake.
- **Decision record:** the user direction above.

### D10. Testing and race stress

- **Mod unit tests.** `claude plugin test` runs `.test.ts` files with stubbed events and no session or sign-in. They cover:
  - every D5 rule;
  - a seeded randomized stress (at least 500 schedules per run) that interleaves `turn.start`/`turn.complete` (including aborted turns and out-of-order completion), `tool.call`, prompt-box text, stream items, child exits and reloads.

  Invariants checked:
  - no submit while a main turn is open;
  - no submit within the hold after an aborted turn;
  - every streamed id delivered at most once and acked at most once;
  - nothing acked that was not delivered in full.

  If the `claude` binary is unavailable, the test harness skips the suite with a recorded reason.
- **Rust tests:** watch registration and refusals, push frames, close on binding change, `AckModDelivered` deciding rules, routing suppression and stall, disconnect kick, setup and unsetup manifests.
- **Mod ledger.** When `HERDR_THREADS_MOD_LEDGER` names a file, the mod appends one JSON line per decision (received, delivered, acked, held, submit, refused, restart). The stress tests assert against it.
- **Live stress.** `tests/native/claude_mod/stress.py` drives real TUI sessions in a private tmux server with an isolated, signed-in profile, over N iterations of the spike's boundary scenarios: queued user prompt, Stop-hook continuation, Esc, permission dialog, `/clear`, reload. It is gated (it needs a signed-in isolated profile). Its read-only fallback, when no profile is available, runs only the unit-level stress and records the gap.

### D11. Scope and non-goals

**Out of scope:**
- Codex: no clean native door, per the research.
- Desktop, VS Code and `claude -p` delivery. The mod's hooks run there, but `watch` refuses without `HERDR_PANE_ID`.
- Deleting the hooks or the native wake path.
- Changing deadlines.
- Per-message delivery policy (ht-tqx).

### D12. Coverage amendments (2026-10-09)

These sharpen D2–D8 and govern where they differ.

- **Generation.** "Generation" is the binding generation. A lifecycle check-in (`/clear`, resume) rotates it; a plugin reload does not.
  - Acks for items delivered under an older generation are refused as stale. Those items re-stream to the new session, whose context no longer holds them.
  - A reload re-registers under the same generation and re-acks.
- **Reconnect grace.** When a watch connection drops, the daemon waits 30 s before the native kick. A re-registration of the same generation within that window cancels the kick, and the mod re-acks. Otherwise the kick runs: at-least-once across an unrecovered handoff, never lost (an accepted limit).
- **Stall handover.** When a channel stalls, the daemon sends `Close{stalled}` and refuses re-registration for that generation for 10 minutes. The native ladder is then the only delivery path.
  - Items the stream marked truncated never count toward a stall.
  - A truncated item is shown with `run herdr-threads body <id>`. It settles through the agent's own inbox or ACK.
- **Operator switch.** The daemon setting `mod_delivery=on|off` is read at registration. Turning it off closes live channels with `Close{disabled}`. `HERDR_THREADS_MOD_DELIVERY=off` remains a per-session override. `setup-status` reports the daemon setting.
- **`watch` exit codes:**

  | Exit | Meaning | Mod reaction |
  |---|---|---|
  | 0, 1 | stopped or other error | restart with backoff |
  | 2 | refused, retryable | retry with backoff |
  | 3 | permanent: disabled, no `HERDR_PANE_ID`, unsupported | stop until reload |

  `watch` also exits when its parent dies: it polls `getppid()`, because `$.process.spawn` closes stdin.
- **Mod startup check.** The mod checks for the APIs it needs before spawning `watch`, and stays inert without them.
- **Delivered-but-unacked ids.** They persist in `$.store` keyed by session id. Failed acks are retried. A generation change discards the set.
- **Subagent tool calls never carry context.**
- **Drafts.** An idle submit waits up to 120 s while the prompt box holds text, then submits. The spike showed a draft is preserved.
- **Attention items** route like messages and are never acked. They are re-sent once per attention version and after each `watch` (re)start. When no channel is live, the native ladder is their fallback.
- **Hook digests.** Claude installs only the `SessionStart` and `PreToolUse(Bash)` hooks. Both omit their attention digest while a channel is live.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
