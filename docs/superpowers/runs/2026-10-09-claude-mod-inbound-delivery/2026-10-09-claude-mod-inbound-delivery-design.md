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
- The daemon setting `mod_delivery` is `on` (the default).
- The seat is resolved and not held.
- An open binding with provenance `cooperative_top_level` exists, with harness `claude`.
- Its native session equals the claim's.
- The binding generation is not in a stall cooldown (D7).

On success it records a process-local registration `(seat, binding_generation) → connection`. It then answers `WatchAccepted{attention_version}` and from then on pushes `WatchFrame` values:
- `Attention{version}` whenever the seat's attention changes. The notify call sites are:
  - an ordinary send to the seat;
  - lazy row publication;
  - invitation and required invitation;
  - warning open or clear;
  - notice publication;
  - catch-up release;
  - settlement by another path (hook or inbox ACK).
- `Close{reason}`, with reason `replaced`, `binding_changed`, `retired`, `unresolved`, `stalled`, `disabled` or `stopping`, after which the daemon closes the stream.

**Generation.** The registration is keyed by the binding generation. A lifecycle check-in (`/clear`, resume, `/branch`) rotates the generation and closes the channel with `binding_changed`; a plugin reload does not rotate it. A `binding_changed` close starts a seat-level *rebind grace* (D7).

**Lifetime.**
- The registration lives in memory only. A daemon restart drops every registration, and the clients reconnect.
- At most one registration per seat. A second watch for the same seat and generation replaces the first, and the old stream gets `Close{replaced}`.
- Watch connections have their own admission budget: a separate semaphore of 64, outside the ordinary `MAX_CONNECTIONS`, so one-shot requests are never starved. Over the cap, registration is refused with `busy` (exit 2).
- A capability string, `MOD_WATCH`, gates the feature, and `PROTOCOL_VERSION` stays at 6.
- Turning `mod_delivery` off closes every live channel with `Close{disabled}`.

**Status read.** A read-only `ModChannelStatus` (live channel count, seats, harness, connected-since, the `mod_delivery` setting) is exposed on an existing status result for `setup-status`.

**Considered:**
- Polling with `HistoryRange::After` (the `follow` precedent): no liveness, and it adds latency.
- Reusing the service connection: wrong identity class. Service authors cannot be receipt recipients (A5).
- Pushing full bodies in frames: it duplicates the read paths and their bounds, and makes a slow mod back-pressure the daemon.

### D3. Seat identity, check-in and the `watch` process contract

`watch` neither enrolls, allocates nor rebinds. The settings `SessionStart` hook still does lifecycle check-in (C1/C2, "Startup enrollment").

`watch --harness claude --session <id>` claims the existing binding: pane from `HERDR_PANE_ID`, native session id from `--session`. The mod passes the id from `$.session.id()`, read after any `session.end`, never inside it.

The registration is recorded as a channel claim with a new provenance, `cooperative_mod_channel`. It means the pane's top-level session, through its mod, opened a delivery channel. It grants nothing except receiving deliveries and making `cooperative_mod_delivery` claims for that binding generation.

Exit codes, each preceded by one JSON status line on a non-zero exit:

| Exit | Meaning | Mod reaction |
|---|---|---|
| 0 | stdout closed or stream ended (including daemon restart) | restart with backoff |
| 1 | other error | restart with backoff |
| 2 | refused, retryable (`no_binding`, `session_mismatch`, `held`, `unresolved`, `cooldown`, `busy`) | retry with backoff 1, 2, 5, 10, then every 30 s |
| 3 | permanent (`disabled` by setting or `HERDR_THREADS_MOD_DELIVERY=off`, no `HERDR_PANE_ID`, `not_claude`, unsupported daemon) | stop until reload |

Exit 2 covers the race where the mod starts before the SessionStart hook's check-in commits.

`watch` also exits when its parent dies: it polls `getppid()` each second, because `$.process.spawn` closes stdin.

**Considered:**
- Letting the watch connection itself perform a Lifecycle check-in: that duplicates enrollment and moves continuity decisions into a second path.
- Trusting `HERDR_PANE_ID` alone without matching the session: it would let a stale mod from a replaced session hold the channel.

### D4. What is delivered

`watch` streams one JSON object per line on stdout (`schema: 1`), each carrying a stable `id` and a `kind`:

| kind | Content | Source |
|---|---|---|
| `message` | thread id and name, sender, `author_role`, `relays_user`, `user_intent`, the full body or a truncated marker, whether an ACK is required | pending receipts, via `InboxBatch`-style bounded pages |
| `lazy` | the same fields, for lazy rows | `lazy_delivery::pending_page` |
| `attention` | the existing fixed marker (invitations, warnings, deadlines, notices, anything without a body), carrying the attention version | the attention digest |
| `status` | connected, refused or closing, with a reason | watch lifecycle |

**Paging.** Bodies over a per-message limit (8 KiB) are streamed as their first 8 KiB with `truncated: true` and `…truncated; run herdr-threads body <id>, then herdr-threads ack <id>`. Each page is bounded (at most 32 items or 64 KiB), with paging until drained after every `Attention` frame and after connecting.

**Framing.** The mod frames all peer text as untrusted data, matching the hook's `untrusted_peer_data` convention:
- a fixed instruction header naming herdr-threads that treats every body as untrusted data and explains the markers (they attribute the source and grant no permission) and says that header lines come only from herdr-threads and that every body line is indented;
- one block per item: `[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:` followed by the body, every body line indented by two spaces (as the compact `body` read does), so only the mod's header lines start at column 0; thread, sender and id are folded onto the header line, where `<thread>`/`<sender>` are names when supplied, else ids, and the markers are `[human]`, `[relays user]`, then `[query]`, `[request]` or `[rule]` — the same fixed text every other read path shows.

Text never starts with `/`, which `$.prompt.submit` refuses.

A truncated item is never acked by the mod, and `body` is read-only. The marker therefore names both steps: `body` to read the rest, then `ack` to settle the receipt; a text `inbox` that displays the item in full also settles it. A truncated lazy row has no receipt and stays pending until a text `inbox` shows it.

**Considered:** delivering only the marker, as the native wake does. That keeps every tool call and loses the main benefit.

### D5. Mod delivery state machine

**Startup.** At load (`session.start`, including after a reload) the mod:
- checks for the APIs it needs (`$.prompt.submit`, `$.session.append`, `$.process.spawn`, `$.store`) and stays inert without them;
- then spawns `watch`.

**Busy state.** Busy and idle come from engine events, keyed by turn id: the set of open main-conversation turn ids. Events carrying `agentId` (subagents) are ignored, and their tool calls never carry context. The open set is kept in `$.state`, which survives a module reload; a module variable does not.

After a load with no recorded state, the mod starts **assumed busy**. It leaves that state at the first `turn.complete`, or after 5 s with no `turn.start` and `$.prompt.read()` succeeding. A stale open id, from a lost `turn.complete`, is cleared when a later turn of the main conversation starts or completes.

| Item | While busy (main turn open) | While idle |
|---|---|---|
| `message`, `attention` | Attach to the next main-conversation `tool.call` result as `context`. If the turn ends first, deliver after it per the idle rule. | `$.prompt.submit` of one batched, framed text. Never awaited inside a hook. Never called while any main turn is open. |
| `lazy` | `$.session.append` (user-role hidden row) at once | `$.session.append` at once; it starts no turn |

**Idle-submit gates**, in order:
1. **Post-abort hold.** After a `turn.complete` with `isAborted`, idle submits are held until either:
   - a later non-aborted main turn completes; or
   - 120 s pass with no open turn and an empty prompt box (`$.prompt.read()`).

   While this hold is active, the draft rule below does not apply: a non-empty box keeps the hold.
2. **Draft.** Outside a post-abort hold, an idle submit waits up to 120 s while the prompt box holds text, then submits. The spike showed the draft stays in the box.
3. **One submit in flight.** Items arriving while a submit is pending wait for the next opportunity.

**Other rules:**
- **Attention items** are delivered once per attention version and once after each `watch` (re)start. They are never acked.
- **De-duplication and the delivered-but-unacked set.** These live in `$.store`, keyed by session id.
  - On `session.end` with reason `clear`, the set is discarded and the cleared session gets everything re-streamed.
  - On `resume`, the set is kept. The restored transcript already holds those deliveries, and the mod re-acks them after re-registering (D6).
  - `/branch` reports `resume` but has a new session id. Its new key starts empty, so items delivered but not acked before a branch may be delivered again: an accepted limit.
- **Visibility.** Each context or append delivery writes one dim `$.ui.log` line (`herdr-threads: delivered N message(s)`), because those paths are invisible in the TUI.
- **Restart.** On every `watch` exit, and on `session.end` (`clear` or `resume`), the mod:
  - stops the child;
  - waits for the handler to return;
  - re-reads `$.session.id()`;
  - restarts `watch` per the D3 exit table.

**Considered:**
- Submitting whenever a message arrives and letting the engine queue it: this produced the Esc takeover in the spike.
- A single busy flag: broken by the event ordering the spike observed.
- Delivering ordinary messages through append: no wake for an idle agent.

### D6. Receipts: `watch ack` and `cooperative_mod_delivery`

**Delivered predicates.** The mod acks an item only when that path's predicate holds on the resolved result:

| Path | Delivered when |
|---|---|
| `context` | The final `tool.call` result returned by the mod's hook after `next(e)` is the answered-result variant (`result` present, `deny` absent, `isError` not true) and carries the mod's context entry. A deny, error or interrupted call keeps the items pending, and they are re-attached or submitted later. |
| `submit` | `$.prompt.submit` resolved without `drop`. A `drop` keeps the items pending, logs the reason, and backs off 30 s before the next submit. |
| `append` | `$.session.append` resolved without `deny`. For lazy rows the claim is "appended to the transcript". |

Then the mod runs `herdr-threads watch ack --session <id> --via context|submit|append <ids...>` through `$.process.run`.

The daemon decides `AckModDelivered` per id against A2:
- the current binding and generation, with provenance `cooperative_top_level`;
- a live watch registration for that generation, a registration in reconnect grace included.

**Resume re-ack.** An ack whose session id equals the current binding's native session is also accepted for ids delivered under the immediately previous generation of that same native session. This is the resume case: the session id is unchanged and the transcript holds the delivery.

It then:
- **Ordinary pending receipts:** settles them exactly as `AckDisplayed` does, with `ack_observation.action_provenance = "cooperative_mod_delivery"`. The binding's `cooperative_top_level` provenance is kept separately.
- **Lazy ids:** completes them as displayed (A8), with the same claim.

**Per-id results**, printed by `watch ack` as one JSON object per id:

| Result | Meaning | Mod reaction |
|---|---|---|
| `settled` | settled now | drop from the set |
| `already_settled` | idempotent success | drop from the set |
| `refused_terminal` | unknown, not addressed to the seat, truncated (decided from the stored body length, never a client hint) | drop from the set |
| `stale_generation` | the binding generation changed, and the id was not delivered under the immediately previous generation of the same native session | drop from the set; the item re-streams to the new session |
| `retryable` | no live channel yet, daemon busy or unreachable | keep; retry on the next `Attention` frame, after the next registration, and every 30 s while registered |

The process exit code is 0 when every id has a result. Otherwise it is non-zero, and every id is treated as retryable. Only `settled` and `already_settled` count as a mod ack for the stall predicate.

**User direction.** This is a user-approved change to the 2026-10 "no read/delivery auto-ACK" direction. In this session the user stated delivery is the receipt for now ("we already judged delivery is the receipt"). The precedent is `cooperative_inbox_display`. For ordinary messages the claim is "the full body entered the model's context through the mod"; it is not proof the model read it.

**Considered:**
- Reusing `cooperative_inbox_display`: it means "the text inbox command flushed a page", a different action.
- Letting the agent ACK explicitly after a mod delivery: it keeps the tool-call overhead the goal removes.

### D7. Routing and fallback

**The registry.** A process-local `ModChannels` service holds the registry. Three consumers check it:
- the wake dispatcher, before reserving a native prompt or poke (`NativeWakeDispatcher::attempt` and `can_reserve_poke`);
- the Claude hook check-in result for `SessionStart` and `PreToolUse(Bash)`, the only installed Claude hooks, which omit their attention digest and ready commands when `mod_channel_live` is set;
- the scheduler's attention kick, which also notifies channels.

**Live.** A channel is live from registration until its stream ends, and through the reconnect grace that follows (below). While it is live:
- No native prompt or poke is sent.
- Pending attention stays pending.
- Deadlines and hard-deadline warnings keep running. Warnings reach the agent as `attention` items through the mod.

**Reconnect grace.** When a watch connection drops (exit, crash, reload), the registry keeps the entry in grace for 30 s, and it still counts as live for all three consumers.
- If the same binding generation re-registers within the grace, the grace ends and the mod re-acks its delivered-but-unacked set.
- If not, the entry is removed and the wake lane is kicked for that seat, so the existing ladder resumes from current pending state.
- **Rebind grace.** A `Close{binding_changed}` (`/clear`, resume, `/branch`) keeps a seat-level entry for 30 s that counts as live for all three consumers, including the `SessionStart` check-in result that rotated the generation, so its digest is omitted. A registration for the new generation ends the rebind grace. If none arrives in time, the entry is removed and the wake lane is kicked.
- A `Close` for `retired`, `unresolved`, `stalled`, `disabled` or `stopping` removes the entry at once, with no grace, and kicks.
- Nothing is marked delivered by a disconnect. An item streamed but never acked stays pending and is re-streamed on reconnect, or reaches the agent through the native wake. Across an unrecovered handoff that is at-least-once, never lost: an accepted limit.

**Stall handover.** A channel counts as stalled when all of these hold:
- it is live;
- its last mod ack (or its registration, if it has none) is more than 10 minutes old;
- some ordinary pending, non-truncated receipt for the seat was published at or before the last pushed `Attention` frame, and is itself more than 10 minutes old.

A long post-abort hold with the user away is the typical cause. On stall the daemon:
- sends `Close{stalled}`;
- removes the entry without grace and kicks the wake lane;
- refuses re-registration for that binding generation for 10 minutes (`cooldown`, exit 2).

The native ladder, with its existing composer guards, is the only delivery path during the cooldown.

**Considered:**
- Hard suppression with no stall bound: one stuck mod would silence a seat forever.
- Keeping a stalled channel open alongside the native ladder: two paths could deliver the same item.
- A per-message hand-off timer: needless complexity.

### D8. Install, upgrade, uninstall

The mod's files (`.claude-plugin/plugin.json` with a `types` field, `hooks/hooks.json`, `hooks/register.js`, and `types/index.d.ts` declaring the `$.state` values the mod uses) are embedded with `include_str!` and written by `setup claude` to `<state>/claude-mod/herdr-threads/`, versioned with the binary.

Rewriting them on upgrade triggers a module reload in running sessions. That is safe because of the D5 assumed-busy startup and `$.state` turn tracking, and the D7 reconnect grace.

**Before writing anything, `setup claude` checks two things:**
- **Managed policy.** If managed settings set `disableSideloadFlags`, it does not write `CLAUDE_CODE_PLUGIN_DIRS`: Claude Code would refuse to start. The install stays hooks-only and says so. The check reads the managed settings files Claude Code documents for the platform. Server-delivered managed settings are cached under the config dir and read when present.
- **Process environment.** If `CLAUDE_CODE_PLUGIN_DIRS` is set in its own environment, it warns that a settings `env` value replaces the shell value. The warning includes the user's directories in the written value, so they are not silently lost.

**The settings change.** The user's Claude `settings.json` gains `env.CLAUDE_CODE_PLUGIN_DIRS`:
- the existing settings value, if any, is preserved, and the mod directory is appended with `:`;
- the change is recorded in the owned setup manifest, the same fingerprinted ownership as the hook groups, including any directories setup copied in from the shell environment;
- `unsetup` removes the appended mod path, the copied-in directories and the files. It deletes the key when nothing the user wrote remains.

**`setup-status` reports:**
- whether the mod is installed;
- the managed-policy and shell-environment conditions above;
- whether `claude --version` is at least 2.1.287 (older versions never load the mod, which is the fallback);
- the daemon's `mod_delivery` setting and live channels, from `ModChannelStatus`.

It also states that an install does not prove any session loaded the mod.

`setup-status` and every later `setup claude` re-run the managed-policy check. When the policy appears after install, they report that Claude Code will refuse to start and offer to remove the written path (`setup claude --hooks-only`). Accepted limit: the daemon cannot see a session that never started.

Tests use an isolated `HOME` and `CLAUDE_CONFIG_DIR`.

**Considered:**
- `claude plugin install` from a local marketplace (the documented route): it writes Claude's plugin state outside our manifest, requires the `claude` CLI at setup time, and is equally subject to managed policy.
- Asking users to pass `--plugin-dir`: not automatic.

### D9. Trust policy

Amend TRUST-POLICY.md in the bead that introduces the constants:
- **A3:** add `cooperative_mod_channel` (channel registrations only) and `cooperative_mod_delivery` (receipt action observation; lazy completion as "appended to the transcript").
- **A4:** a live channel (grace included) replaces native wake and poke for its seat, and the stall handover applies.
- **A5:** rows for opening a channel and for mod delivery ACK.
- **A8:** the mod as a second lazy completion source.
- **Accepted limits:**
  - the mod claim is cooperative: a same-user process could run `watch ack`;
  - engine-reported idleness is trusted;
  - a remote mods kill switch returns seats to native wake;
  - an unrecovered handoff is at-least-once;
  - an outer mod could strip context after the herdr-threads hook returned.
- **Decision record:** the user direction above.

### D10. Testing and race stress

- **Mod unit tests.** `claude plugin test` runs `.test.ts` files with stubbed events and no session or sign-in. They cover:
  - every D5 and D6 rule;
  - a seeded randomized stress (at least 500 schedules per run) that interleaves:
    - `turn.start`/`turn.complete`, including aborted turns and out-of-order completion;
    - `tool.call` results: answered, denied, error and subagent;
    - submit results with and without `drop`;
    - prompt-box text, stream items, child exits;
    - reloads mid-turn;
    - `/clear`, resume and `/branch` with delivered-but-unacked items;
    - transient `retryable` ack results.

  Invariants checked:
  - no submit while a main turn is open;
  - no submit within the post-abort hold;
  - every streamed id delivered at most once and acked at most once;
  - nothing acked whose delivered predicate failed, or that was truncated.

  If the `claude` binary is unavailable, the test harness skips the suite with a recorded reason.
- **Rust tests:** watch registration and refusals, push frames and the notify call sites, close reasons, grace and stall cooldown, the per-id `AckModDelivered` results, routing suppression, setup and unsetup manifests (including the managed-policy and shell-env cases).
- **Mod ledger.** When `HERDR_THREADS_MOD_LEDGER` names a file, the mod appends one JSON line per decision (received, delivered, acked, held, submit, refused, restart). The stress tests assert against it.
- **Live stress.** `tests/native/claude_mod/stress.py` drives real TUI sessions in a private tmux server with an isolated, signed-in profile, over N iterations of the spike's boundary scenarios:
  - queued user prompt;
  - Stop-hook continuation;
  - Esc;
  - permission dialog;
  - `/clear` and `/resume`;
  - reload mid-turn;
  - a UserPromptSubmit hook that blocks (drop).

  It is gated (it needs a signed-in isolated profile). Its read-only fallback, when no profile is available, runs only the unit-level stress and records the gap.

### D11. Scope and non-goals

**Out of scope:**
- Codex: no clean native door, per the research.
- Desktop, VS Code and `claude -p` delivery. The mod's hooks run there, but `watch` exits 3 without `HERDR_PANE_ID`.
- Deleting the hooks or the native wake path.
- Changing deadlines.
- Per-message delivery policy (ht-tqx).

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

- **2026-10-09 — divergences found while implementing (super-auto run, fix loop rounds 1–2):**
  - *Notify call sites.* D2's per-call-site `ModChannels::notify` became a table-commit observer: the registry worker compares each live seat's attention fingerprint after commits and pushes `attention` itself. `ModChannels::notify` and `record_attention_push` remain on the trait for tests only.
  - *`mod_delivery` is boot-only.* The setting is read when the daemon starts; there is no runtime kill switch (`set_mod_delivery` has no production caller). Changing it takes a daemon restart, which ends every live channel anyway.
  - *`replaced` exits 3.* A Close with reason `replaced` (a newer channel for the seat took over) stops the old `watch` with exit 3, which the D3 table did not list; the replaced mod instance must not reconnect and fight the new one.
  - *Liveness is per seat.* Wake candidates, pokes and the hook digest all ask one per-seat question (any registry entry whose grace has not expired, of any generation). The first cut asked per generation on the wake path, which reopened the /clear turn-boundary race D7 closes (ht-ows); fixed by ht-j16.17.
  - *Mod launch argv.* The mod originally ran bare `herdr-threads` from PATH. `setup claude` now writes the hooks' exact invocation (absolute executable, `--state-dir`, `--host-endpoint`) into the installed mod (ht-j16.20); the truncation marker renders the same selectors (ht-j16.24).
  - *Truncated bodies.* `body` is read-only, so the marker names `body` then `ack` for ordinary items and a body-only form for lazy rows (ht-j16.23, ht-j16.24).
  - *Notices while live.* Notices are not attention items; while a channel is live the tool-boundary check-in still offers pending notices (without ready commands) (ht-j16.21).
  - *Managed policy.* Verified against Claude Code 2.1.295 that server-managed settings are cached at `<config dir>/remote-settings.json`; setup also reads `managed-settings.d/*.json` and treats an unreadable source as unsafe (ht-j16.22).
  - *Accepted windows.* After a daemon restart the wake lane's first pass can run before the mod re-registers, so one item may be delivered both natively and by the mod (at-least-once, as D5 allows). A mod whose `watch` keeps reconnecting resets the stall clock on each registration, so a mod that reconnects but never delivers can hold native wake off until it stops reconnecting.
