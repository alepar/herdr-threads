# Seat recovery after a Herdr restart, daemon restart or upgrade (ht-dtx0)

Status: design, 2026-10-27. Folds in ht-e0t7 (doctor text). ht-mp72 (frozen host lane) is assessed in D4.

## Problem

A Herdr restart is a new incarnation. C2 unresolves every saved seat that has no structural proof
(`MarkUnresolved`, reason `other`) and ends its bindings with `seat_unresolved`. The only way back without an
operator is C1 cooperative continuity: the session's own top-level lifecycle or turn event, carrying its native
session id. On the laptop (2026-10-26, v2.3.1) Herdr's restore relaunched every pane with `claude --resume <sid>`
at 21:13:08Z. The old daemon was frozen (`capture frozen`, host endpoint witness unavailable) until a
replacement started at 21:25:53Z. Its first publication unresolved 23 seats. The idle resumed sessions made
no C1 claim that survived, so 22 seats stayed unresolved and the p0 mail waited for days.

There are two independent gaps:

- **G1. A claim made during the lag is lost.** The daemon answers a resume (or same-session) `ContinuityCheckIn`
  that arrives while the host is frozen, or before the epoch's reconciliation pass, with a retryable error:
  `ServiceBusy`, `HostUnavailable` or `StaleHostObservation`. The hook retries only until its own deadline (about
  5 s) and keeps a client-local intent, which only that session's next hook replays. An idle session has no next
  hook.
- **G2. An idle session with no hook call has no trigger.** The round-2 evidence shows that `wZ:p1` made no
  hook call that reached a daemon. The Claude mod runs `watch --harness claude --session <sid>` inside the same
  Claude process and retries it every 30 s indefinitely. It exits 2 on `no_binding`, `unresolved` and `held`, so
  a `pgrep` snapshot rarely catches it. The watch never submits continuity, though: it holds the session's own
  claim and throws it away.

Unresolving the seat is correct (C2) and stays. What changes is how the seat comes back.

## Decisions

### D1. The push-channel watch makes the session's own C1 claim (closes G2 for the Claude mod)

When `watch` cannot register, it makes one C1 attempt for its own session before exiting 2. That covers two
cases: caller derivation finds no seat for the pane (`no_binding`), and the daemon refuses with `unresolved`,
`held` or `no_binding`. The claim is the existing `ContinuityCheckIn`, with:

- target: the caller pane;
- harness: the watch's env claim (a push-channel harness only);
- native session: the `--session` value;
- liveness: the harness process fingerprint the watch already computes;
- source: `same_session`.

It is sent only when the daemon advertises `seat.continuity_same_session_v1`. No new wire value or capability is
introduced.

- **One journal for both paths.** The watch uses the same pane continuity intent journal and scope as the hook,
  through the same code (the hook's continuity core, extracted from `reattach_by_continuity`). A pending intent
  for the same harness and session is reused under its operation key. In particular, a resume intent that the
  session's SessionStart left pending replays the daemon's committed decision (see D2). Otherwise a
  `same_session` intent is recorded. A reused intent's replay is installed only after the pane-seat listing
  confirms it (the existing `replay_is_current` rule).
- **No retry loop in the watch.** One submission per watch run. The mod's existing backoff (1, 2, 5, 10, 30 s) is
  the retry cadence, so an unenrolled pane costs one extra daemon call every 30 s, on a path that already
  retries.
- **No presentation check-in.** A reattached watch does not present attention through a `Current` check-in, as
  the hook does. It re-locates its claim, registers the channel in the same run, and the channel delivers. A
  fast-forward notice from the reattachment is dropped; the channel's first delivery covers the pending items.
  This is an accepted gap: a cursor fast-forward past more than one page is rare and is visible in `inbox`.
- **The mod re-reads its session id before every watch restart** (`syncSession` already runs when `restartAt` is
  due). The watch claims the id the engine reports at spawn, which keeps the stale-sid window to one spawn.

Trust: this is a C1 trigger, so TRUST-POLICY C1 is amended. Its strength equals a `UserPromptSubmit` claim: the
same process, the same native session id from the harness engine, and the harness process fingerprint. Every C1
guard is unchanged:

- a unique top-level match, preferring a binding ended `seat_unresolved`;
- no resolved owner on the target;
- managed_launch, plugin_context, subagent and human bindings are excluded;
- a new native session never takes an old seat.

### D2. The daemon keeps a claim that arrived during the lag and decides it after reconciliation (closes G1, every harness)

When `Identity::continuity` refuses a claim only because the evidence is not ready, the daemon stores the
claim before answering. "Not ready" is decided by where the refusal happens, not by its error code, because one
code covers both kinds of case. The rule is internal to the daemon; nothing about it is on the wire:

- **Kept** (transient): `decide_continuity` refuses with `ServiceBusy` because reconciliation lags, or the
  fresh host read fails (Herdr frozen, unreachable, or answering for a newer boot or epoch than the daemon has
  published).
- **Not kept** (definitive): every other refusal, including a `ContinuityTargetGuard` mismatch, a structural-proof
  mismatch, `NotFound`, `Conflict` and `TargetAlreadyOwned`.

The stored claim is the exact command, including operation id, execution and liveness, in a new node table
`kept_continuity_claims`:

- keyed by (host, target, harness, native session), so claims from two sessions in one pane, such as a `/clear`
  successor, never overwrite each other; a repeated claim from the same session replaces its own row;
- at most 256 rows in all; beyond that, the claim is answered but not kept.

The reply is unchanged (the same retryable error), so the wire does not change.

The observation worker re-submits every kept claim of its host through `Identity::continuity`, under the stored
operation key, at most 16 per pass, oldest first. It does so after each completed reconciliation pass that
recorded the marker for the current recovery boot and epoch, so `reconciliation_lags` is false. Each attempt is
the ordinary C1 transaction against the canonical view (A2). Because the operation key is the same, the session's
own later resubmission (hook or watch) replays the same result.

A kept row is removed:

- on reattachment;
- on a refusal from the deciding transaction other than lag (`NotFound`, `Conflict`, `TargetAlreadyOwned`,
  `InvalidRequest`, a stale structural proof);
- when its liveness fingerprint's process is no longer alive (`LivenessFingerprint::is_alive`), checked before
  each attempt;
- 1 h after it was received, whatever happened in between.

A transient failure during re-submission, such as a host read failure or a guard race between the read and the
transaction, keeps the row for the next pass. So a claim that keeps failing costs at most one host read per pass
for one hour. A process that is gone cannot be the session that made the claim; that is the guard against deciding
a claim for a session that has left the pane.

The claim and its evidence still come from the session itself. The daemon only defers the decision, and decides
it against current evidence. The client-local intent journal stays a hint that the daemon never reads (option (c)
in the brief is rejected). TRUST-POLICY C1 is amended: "a claim refused only for lag is kept and decided after
the epoch's reconciliation".

Daemon frozen, then replaced: the frozen daemon stores the claim in the shared store, and the replacement
decides it on its first unlagged publication. Daemon absent: a lifecycle hook's bounded `ensure_running` starts
one, and the claim reaches it during its lag.

### D3. Doctor text by reason (ht-e0t7)

`unresolved_seats_line` stops promising structural reconfirmation for `other` seats. New text, at most 256
bytes (about 240 for a 2-digit count):

`unresolved seats: N; lifecycle check-ins are refused until resolved; host_invalidation seats reconfirm when a
coherent snapshot shows their pane; other seats reattach when their session makes a C1 claim; else `seat rebind
SEAT --pane PANE --operator``

### D1b. The watch ensures a daemon

`watch` today only reads the published endpoint (`cli::connect`). With no daemon it exits `daemon_unavailable`,
and the mod retries without ever starting one. The watch now runs the same bounded `ensure_running` (2 s) that
lifecycle hooks use when the endpoint is unavailable or stale. The owner lock already makes concurrent ensures
safe: a live owner is never displaced. With this, an idle mod session brings up a daemon within one backoff step
after the old one died, and D1 then claims the seat.

### D4. ht-mp72: the old daemon died with Herdr's panes (fixed here)

The investigation (code-verified) finds:

- The observation lane retries indefinitely, with 30 s capped backoff.
- The error log is rate-limited but still writes a summary every 30-60 s.
- Nothing is pinned to the first Herdr server: the witness is per call, and background captures pass
  `expected_boot: None`.
- `ensure_running` never starts a second daemon while a live owner holds the lock.

So the 12-minute silence followed by a client-ensured replacement means the old daemon had exited. The cause is
on the restart path. `spawn_detached` (`src/daemon/lifecycle.rs`) calls only `process_group(0)`, and the daemon
installs no SIGHUP handler. A daemon that a hook ensured from inside a Herdr pane therefore stays in that pane's
terminal session, with the pane's pty as its controlling tty. Herdr's shutdown hangs up or kills that session,
and the daemon dies silently. This happens on every Herdr restart: the plugin's daemon is nearly always
ensured from a pane.

Fix: `spawn_detached` calls `setsid()` in `pre_exec`, giving the daemon its own session, its own process group and
no controlling tty, so pane teardown cannot reach it. The daemon then outlives the Herdr restart. Its lane
retries, publishes the new incarnation, and reconciles within one backoff step (at most 30 s). No other change.
`setsid` cannot fail in a freshly forked child that is not a group leader; on failure, spawning fails as it does
today.

Not done (follow-up): logging an external SIGTERM, and treating a `Superseded` capture loop as a failure. Neither
is on this incident's path.

## /clear and the stale sid

`/clear` starts a new native session S' in the same Claude process.

- D1: the mod adopts S' on `session.end(clear)` and re-reads the id before each restart, so a claim for the old S
  can only come from a watch spawned before the change. Such a claim names S, and only a seat with a binding for S
  matches. It reattaches the role to the pane the process occupies, which is the pane's role before the restart.
  S' then checks in on that resolved pane through the ordinary path, exactly as `/clear` in a resolved pane does
  today. No seat is created or merged, and S' never takes a seat through C1, because C1 matches S only.
- D2: rows are kept per session, so the claims of S and S' both stay. S's row reattaches the seat; the S' row is
  refused (`NotFound`, because S' has no binding) and removed. The order does not matter: the S' refusal changes
  nothing.

Residual: after a reattachment for S that comes after `/clear`, the open binding names S rather than S'. That lasts
until S' makes its next check-in on the now-resolved pane: up to one spawn interval for D1, and until the next
hook for D2. Wakes typed into the pane reach S', which is the pane's live session. Accepted limit, stated in
TRUST-POLICY.

## Per-harness coverage

| Harness / stack | Resume hook reached a daemon (frozen, lagging, freshly ensured) | No hook reached a daemon, session idle |
|---|---|---|
| Claude + mod | D2 (kept), and D1 replays the same intent | D1: the watch claims `same_session` within about 30 s of the daemon deciding |
| Claude hooks-only | D2 | next turn (existing same_session C1); accepted limit |
| Codex (hooks) | D2 | next turn; accepted limit |
| Hermes CLI | D2 (`on_session_start`/`pre_llm_call` C1 claims) | next turn; accepted limit |
| Daemon restart, same Herdr incarnation | structural reconfirm / carry (C4), unchanged | unchanged |

Why the hooks-only stacks have no idle-time signal:

- **Claude hooks-only:** Claude Code runs hooks only on lifecycle, prompt and tool events. With no mod, no
  in-process code runs while the session is idle.
- **Codex:** hooks run on SessionStart, UserPromptSubmit, tool events and Stop. There is no timer, and no plugin
  process outlives a hook.
- **Hermes:** plugin callbacks (`on_session_start`, `pre_llm_call`) run only on session and model events.

All three get D2 for their resume hook. With no hook at all, they recover on their next turn or by operator
repair. This accepted limit goes into TRUST-POLICY with the C1 amendment.

Rejected: a wake or a rebind keyed on Herdr's `agent_session` field. The brief rules this out, and it would move
or wake on heuristic evidence. A doctor suggestion is allowed, but it is not added here.

## Acceptance criteria

1. **D1, idle session, no hook.** Integration test with the test-support daemon and a fake Herdr: a seat bound to
   S on P, then a new-incarnation coherent snapshot where P has a new terminal and S is idle. A
   `watch --session S` run reattaches the seat on P with a `cooperative_continuity` decision. A p0 send then
   wakes it (channel registered, attention delivered).
2. **D2, frozen or lagging.** A resume claim submitted while the host is frozen (and, separately, while
   reconciliation lags) gets the retryable refusal. Once the host answers and reconciliation completes, the seat
   is resolved with no further client call. A later resubmission of the same operation replays the
   reattachment. Run once for Claude and once for Codex.
3. **D2, daemon replaced.** A kept claim survives a daemon restart and is decided by the replacement.
4. **Negative cases:**
   - S2, a different session, never takes the seat, through D1 or through D2;
   - two unresolved candidates are refused, and the kept row is removed;
   - an agent-field match with no claim triggers nothing;
   - a kept claim whose process is dead is removed without a decision;
   - a guard mismatch at intake is not kept.
5. **D2 rules** (store-level tests):
   - per-session keying: claims from S and S' on one target both stay, then S reattaches and S' is removed;
   - the 256-row cap: the claim is answered but not kept;
   - expiry after 1 h.
6. **D1, older daemon.** Against a daemon that does not advertise `seat.continuity_same_session_v1`, the watch
   sends no claim.
7. **D3.** Doctor text as specified, bounded at 256 bytes.
8. **D4 and D1b.** A daemon ensured by a child that is a session leader with a pty survives a hangup and kill of
   that session. With no daemon running, `watch` ensures one and then registers or claims.
9. **Checks:** clippy -D warnings, fmt, harness-neutral, wire-compat, default features, the focused tests, and
   one full suite with the leak check.
