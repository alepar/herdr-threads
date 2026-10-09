# Hermes global enrollment through its hooks (ht-4ne)

Root spec of the super-auto run `2026-10-09-hermes-global-enrollment` (bead ht-4ne).
Evidence: [bare-hermes-detection-evidence.md](bare-hermes-detection-evidence.md).
Prior design: [Hermes native adapter](../2026-10-03-harness-adapters/2026-10-03-harness-adapters--hermes-design.md).

## Goal

A bare Hermes CLI session (`hermes`, no `herdr-threads launch --kind hermes`) started in a
Herdr pane whose selected Hermes profile has the herdr-threads plugin installed and enabled
gets that pane's seat and its first qualified top-level turn receives the herdr-threads
context, exactly as a launched one does; delegated Hermes children still never enroll or
check in, and every TRUST-POLICY guard (restore holds, collisions, A2) still applies.

## Problem description

Claude and Codex enroll "globally": any top-level `SessionStart` in a Herdr pane runs the
journaled ordinary seat resolver (`launch::resolve_seat`) and then a lifecycle check-in
(TRUST-POLICY C2 "Startup enrollment"). Hermes enrolls only through `launch`
(TRUST-POLICY.md:490-494). The run's investigation found that the launch dependency is
narrower than the policy text suggests: no launch token or record reaches callback
admission. The gate is two code facts. (a) `src/cli/hook.rs:2909-2914` sets `enroll` only for
`EventIntent::Lifecycle(Startup|Clear|Resume)`, and Hermes callbacks never decode to
`Lifecycle` (`pre_llm_call` is `QualifiedTurn`, `on_session_start` is an observer, `hermes.rs:720-776`).
(b) Without enrollment, check-in needs a seat already resolved for the pane, and for a Hermes
pane only `launch` creates one. The test `startup_enrollment_hermes_qualified_unowned_never_resolves`
(`tests/cli/hook.rs:6965`) locks this in. The guarded Herdr process hint
(`hermes/launch.rs:185`) matters only to native launch (Herdr's readiness poll); callback
admission never consults it. Herdr 0.9.1 does not process-detect a bare macOS Hermes (argv0
`python3`, no `setproctitle`), and Herdr's own Hermes plugin only sets `pane.agent_session`,
which herdr-threads does not read for authority (evidence §A, §B).

## Main challenges

Three. First, **top-level evidence at a lifecycle point**: `on_session_start` has no
`parent_session_id`, which is why the 2026-10-03 design made it observer-only. Second,
**latency**: the bridge gives `pre_llm_call` a 1.2 s local cap and lifecycle callbacks 4.5 s
(`integrations/hermes/__init__.py:486-491`); seat resolution needs a fresh current-target read
and may need to start the daemon, which fits the lifecycle budget but not reliably the turn
budget. Third, **no context sink at session start**: Hermes ignores `on_session_start`'s
return value, so a check-in made there would commit a check-in offer (which settles notices,
skill guide "Warnings") whose context the model never sees.

## Key decisions made

Enrollment and check-in are split across two callbacks. A top-level Hermes CLI
`on_session_start` **enrolls only** (journaled `ResolveSeat` through the same ordinary guarded
resolver Claude/Codex use; no check-in, no offer). The existing qualified `pre_llm_call` then
checks in against the now-resolved seat and delivers context, unchanged. As a fallback, a
qualified top-level CLI `pre_llm_call` that finds the pane `Unowned` may enroll too, within its
own budget, without starting the daemon. The top-level witness is in-band and Hermes-specific: the native `platform` tag.
Hermes 0.21.5 starts delegated children with `platform="subagent"`
(`hermes-agent/tools/delegate_tool.py:239`) and fires `on_session_start` from
`agent/conversation_loop.py:833-835` with `platform=agent.platform`; the CLI's top-level agent
runs with `platform="cli"`. Enrollment therefore requires `platform == "cli"` on both paths, which
is narrower than today's check-in filter (`None`, `""` or `cli`). No new A3 provenance:
enrollment records none (as for Claude/Codex) and the check-in stays `cooperative_top_level`.
Whether a harness may enroll from a given decoded event becomes an adapter-declared policy
instead of an intent hard-coded in `hook.rs`. Hermes does not adopt the agy run's generic
harness-local top-level witness: its role evidence is in-band, so nothing is duplicated; the
Hermes requirements for that witness are recorded below so the agy design can take them in.

## Decision points, by section

### D1. Which callback enrolls

**Recommended: a top-level CLI `on_session_start` enrolls, and a qualified top-level CLI
`pre_llm_call` enrolls as a fallback when the pane is `Unowned`.** `on_session_start` fires once,
at the first turn of a new session, before the first model call, inside the 4.5 s lifecycle cap,
so it can start the daemon and do a fresh target read. The fallback covers a timed-out or skipped
start (stale timeout observation, daemon start slower than the cap) without a session-long gap. The
fallback never calls `ensure_running`: with the daemon down it stays quiet for that turn (today's
behavior) and the next turn tries again; the start callback is the only enrollment path that may start
the daemon.
*Considered:* `pre_llm_call` only, discarded because 1.2 s cannot reliably cover
`ensure_running` plus `ResolveSeat` plus check-in, so the first turn of a cold machine would
silently stay unseated; `on_session_start` only, discarded because one missed callback would
leave the whole session without a seat until `/new`; reading Herdr's `pane.agent_session` as an
enrollment trigger, discarded because it is a Herdr-plugin claim herdr-threads does not own, is
absent when the user did not install Herdr's integration, and Herdr's agent fields may only
suggest (C1).

### D2. Top-level witness

**Recommended: the native `platform` tag, `platform == "cli"`, on `on_session_start`; for the
`pre_llm_call` fallback, the existing empty `parent_session_id` plus `platform == "cli"`.** The
bridge computes it and sends it as the envelope's `role_association` (`role: "top"`, a new
adapter-local provenance value `cli_session_start` for the start callback; the existing
`explicit_parent` for the turn, with the platform carried so Rust can require `cli` for
enrollment). Rust trusts it the same cooperative way it trusts `explicit_parent` today (TRUST-POLICY
A1: a disobedient child is indistinguishable; no adversarial verification). A start callback whose
platform is not exactly `cli` is still forwarded as today's observer (role unknown), never as an
enrollment. *Considered:* treating every `on_session_start` that passes the bridge's current filter
(`None`, `""`, `cli`) as top-level, discarded because programmatic agents created without a
platform (tool- or plugin-spawned `AIAgent`s) report `""` and could replace the parent's binding
through an agent-to-agent lifecycle check-in (A4); waiting for the agy generic witness, discarded
because Hermes's evidence is already in-band and the agy design is unsettled (thread `teAfuOE4V`,
no reply at spec time); Herdr's `agent_session.session_start_source`, discarded per D1.

### D2a. Envelope and policy contract (pinned)

The bridge envelope's `role_association` object gains one field, `enrollment_witness`, whose
value is the string `"cli_platform"` or JSON `null`; it is always present (strict parse,
byte-identical decode, `hermes.rs` `parse`/`decode`) when the bridge sends it. For version skew, an
envelope with the field **absent** is accepted and decoded exactly as `null` (an older installed
bridge keeps today's launch-only behavior instead of being refused; re-running `setup hermes` installs
the new bridge, and the user docs say so). The bridge sets it to `"cli_platform"`
exactly when the native callback's `platform` is the string `"cli"`, and to `null` otherwise.
Per callback:

| Callback | `role` | `provenance` | `enrollment_witness` |
|---|---|---|---|
| `on_session_start`, platform `"cli"` | `"top"` | `"cli_session_start"` (new) | `"cli_platform"` |
| `on_session_start`, platform `None` or `""` | `"unknown"` | as today | `null` |
| `pre_llm_call`, no parent | `"top"` | `"explicit_parent"` | `"cli_platform"` iff platform `"cli"`, else `null` |
| `pre_llm_call`, parented (child) | answered locally by the bridge, never forwarded (as today) | | |
| `post_tool_call`, `on_session_reset` | as today | as today | `null` |

Rust side: `enum EnrollmentPolicy { Lifecycle, WitnessedStartAndUnownedTurn }` returned by
`HarnessAdapter::enrollment_policy()` (default `Lifecycle`; Hermes
`WitnessedStartAndUnownedTurn`), and `enum EnrollmentKind { Lifecycle, SessionStart, UnownedTurn }`
returned by `DecodedEvent::enrollment(policy)`. `Lifecycle` is today's rule (top-level
`Lifecycle(Startup|Clear|Resume)` with context delivery, can check in). `SessionStart` is
`EventIntent::Enroll` with role top-level and witness `cli_platform`. `UnownedTurn` is a
top-level `QualifiedTurn` with witness `cli_platform`, honored only after `find_seat` returns
`Unowned`. A witness on any other callback, or `cli_session_start` with a `null` witness, is a
strict-parse refusal, and so is `role: "child"` with a non-null witness.

### D3. Enrollment without check-in at session start

**Recommended: a new decoded intent, `EventIntent::Enroll`, that runs `resolve_seat` (with
`ensure_running`, top-level lifecycle budget) and returns no context and no check-in.** The
next qualified `pre_llm_call` checks in as today's `StartupAttach` `Startup` (or `Clear` after a
declared reset) and returns the bootstrap context. Nothing is offered that cannot be shown.
*Considered:* a full lifecycle check-in at `on_session_start`, discarded because its offer would
settle notices the model never sees (Hermes drops the return value); synthesizing the context
into the next turn from the start-time check-in, discarded as a new delivery mechanism with its
own receipt semantics for no gain.

### D4. Adapter policy seam

**Recommended: replace the hard-coded `enroll` expression at `hook.rs:2909-2914` with
`DecodedEvent::enrollment(&self) -> Option<EnrollmentKind>` driven by a new adapter method,
`HarnessAdapter::enrollment_policy()` (default `EnrollmentPolicy::Lifecycle`, which reproduces today's rule
exactly for Claude and Codex), with Hermes returning `WitnessedStartAndUnownedTurn` (names pinned in D2a), which admits `Enroll` (D3)
and, when the pane is `Unowned`, a top-level `QualifiedTurn` whose envelope platform is `cli`.**
Enrollment still needs `role == TopLevel` and `can_check_in()` or the `Enroll` intent, and still
never runs for subagent, observer, `Current`/compact or `DeclaredReset` events. The fallback
path checks `find_seat` first and calls the resolver only on `Unowned`, so a steady-state turn
adds no resolver call. (`find_seat`'s hard-coded `harness: Harness::Claude` selection at
`hook.rs:1346` is left alone: `selected_generation`, `src/cli/mod.rs:183-212`, never reads the
harness, so it has no effect.) *Considered:* an `if harness == hermes` branch in `hook.rs`, discarded because the
registry exists so the hook path stays harness-neutral (the agy run is testing exactly that);
mapping Hermes `on_session_start` to `Lifecycle(Startup)`, discarded because the lifecycle path
also checks in (D3).

### D5. Seat continuity, reset and resume

**Recommended: reuse what exists and claim nothing new.** `/new` fires `on_session_reset`
(`reason: new_session`, recorded as today's declared-reset hint) and then the new session's
`on_session_start`; enrollment reuses the pane's resolved seat (resolver no-op), and the next
turn consumes the hint as `Clear` on the same seat (A4 agent-to-agent). A resumed Hermes session
(`--resume`, `/resume`) emits no `Resume` lifecycle, so it gets no C1 cooperative continuity: in
a pane with its seat still resolved it checks in as a new `Startup`; in a restored pane after a
Herdr restart it meets the restore hold and the existing inspect/rebind guidance. Recorded as an
accepted limit. *Considered:* deriving `Resume` from Herdr's `agent_session` source or from the
Hermes session database, discarded (heuristic evidence must never move a seat, AGENTS.md).

### D6. Wake and recognition

**Recommended: no change; document the limit.** Wake and pokes go only to a pane whose Herdr agent
kind matches the bound harness (A4 *Wake*). A globally enrolled bare Hermes on macOS is not
process-detected, so its seat receives messages through the turn-time bridge context only and is
not woken while idle. On Linux, or with Hermes's optional `setproctitle` installed, Herdr likely
recognizes it (unverified). `launch --kind hermes` keeps the guarded process hint and stays the way
to get a wakeable Hermes. *Considered:* treating `pane.agent_session.agent == "hermes"` as
recognition for the wake recheck, deferred to a follow-up issue (it needs a TRUST-POLICY decision
about a Herdr-plugin-reported field and evidence about its lifetime after the process exits; the
probe saw later reports not replace the first).

### D7. Scope of "global"

**Recommended: every Hermes CLI session in a Herdr pane whose selected profile has herdr-threads
installed (`setup hermes --profile NAME`) and enabled (manual `hermes plugins enable
herdr-threads`).** Setup and enablement are unchanged; bare multi-harness `setup` keeps skipping
Hermes. Outside Herdr the bridge stays silent (`foreign_session`, `hook.rs:3029-3050`). Other
Hermes platforms (`tui`, `desktop`, `acp`, gateway) stay unsupported, as today. *Considered:*
auto-enabling the plugin in setup, discarded (setup never edits Hermes YAML; native permissions
stay manual).

### D8. Policy and documentation

**Recommended: the code change that lets Hermes enroll updates TRUST-POLICY.md in the same commit
(AGENTS.md):** C2 "Startup enrollment" names the Hermes CLI start callback and the turn fallback
as enrollment events through the same resolver; the accepted limit "Hermes enrolls only through
`launch`" is replaced by "Hermes enrolls through its CLI hooks" stating the platform witness, the
no-check-in start, the absent C1 resume continuity (D5) and the wake limit (D6); a decision record
entry (2026-10-09, ht-4ne) records the choice and the rejected alternatives. User-facing text that
says "launch only" is updated separately: `integrations/skill/SKILL.md:43`, `src/cli/launch.rs:119-129`,
`README.md` (the Hermes paragraph), and setup/status guidance where it mentions launch.

### D9. Verification

**Recommended:** unit and hook tests with synthetic envelopes (allocation on a CLI start, reuse on
`/new`, fallback enrollment on an `Unowned` turn, no enrollment for `subagent`/`""`/`None`
platforms, parented turns, resets, observers, restore holds and non-Herdr runs; Claude/Codex
behavior unchanged; the old `..._never_resolves` test inverted to its new meaning), Python bridge
tests for the envelope it sends per platform, and one **configuration smoke** that drives the real
bridge module with synthetic Hermes callbacks against an isolated named Herdr session and an
isolated daemon (never the shared server, never `~/.hermes`). A live run of the real Hermes binary
is out of scope: a fresh `HERMES_HOME` triggers a source-update dependency sync keyed on the real
install root (evidence, "Not verified").

## Requirements for the agy generic top-level witness

Recorded for the agy run (thread `teAfuOE4V`). Hermes needs nothing from it now. If a generic
witness lands, it should: accept adapter-supplied in-band evidence (Hermes: native `platform` tag
on start, `parent_session_id` on turns) as a witness source; not require role evidence on
session-start callbacks for harnesses that have it later in the turn; keep enrollment-without-check-in
(D3) expressible; never turn a Herdr-reported field into authority.

## Follow-on (outside this tree)

- Wake recognition for bare Hermes via Herdr's `agent_session` (D6): needs a policy decision and a
  lifetime probe with a live Hermes.
- Hermes resume continuity (D5): needs a native resume signal.
- Live bare-Hermes end-to-end run in an isolated `HERMES_HOME` that does not touch the real install.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
