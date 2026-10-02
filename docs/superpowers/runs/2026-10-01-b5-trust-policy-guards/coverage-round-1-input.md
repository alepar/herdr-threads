## Goal
Every guard TRUST-POLICY.md marks **required** is implemented and tested in herdr-threads, so that the
running daemon and CLI enforce the continuity invariants C2–C4 and the attribution invariants A2 and A4
(plus the accepted-limits hardening), and the user-facing docs describe the shipped behaviour.

## Parent goal chain
(root pass — empty)

## Spec
### Implementation design (root spec)
# B5 trust policy guards — implementation design

Epic: `ht-rzi`. Normative policy: [TRUST-POLICY.md](../../../../TRUST-POLICY.md) (adopted 2026-10-01). This file
adds only the implementation-level decisions the policy leaves open; where they differ, the policy wins.

## Goal

Every guard TRUST-POLICY.md marks **required** is implemented and tested in herdr-threads, so that the
running daemon and CLI enforce the continuity invariants C2–C4 and the attribution invariants A2 and A4
(plus the accepted-limits hardening), and the user-facing docs describe the shipped behaviour.

## Scope

In scope: the required guards of C2, C3, C1 (cooperative continuity), C4, A2 (expected daemon boot), A4
(agent-to-human, second agent, wake harness), the accepted-limits hardening (`allocator.lock` without
symlink following, refuse `codex resume` launch form), the W5-2 test, the W6-R2 wording, the identifier
comments, and the operator-facing docs for new commands (`docs/operations.md`, `docs/agent-usage.md`,
README where it names behaviour).

Non-goals:
- C5 / P10 and W5-1: deletions owned by the B4 run (bead `ht-p03.2`, other branch). Not touched here.
- Adversarial verification of any kind (policy principle 1).
- New harness support beyond Claude and Codex.
- Native (real Claude/Codex) demonstration runs; deterministic tests with stand-in agents are the bar for
  this epic, except the evidence capture `ht-rzi.2` needs (see below).

## Implementation decisions

1. **Hold lift (C2).** The instance-wide `baseline_hold_unclaimed` flag is cleared in the same transaction
   that leaves zero unresolved nonretired seats (rebind, fresh seat, retire, replace, cooperative
   continuity, reconciliation retirement). Per-target `recovery_baseline_releases` stay as they are.
2. **Collisions (C3).** `seat retire SEAT --operator` reuses the bounded retirement cutover
   (`begin_retirement`); it is an operator action audited `operator:local-user:<uid>`. `seat rebind OLD
   --pane P --replace NEW --operator` performs NEW's retirement cutover and OLD's rebind in one deciding
   transaction. Both join the typed administrative actions; neither ACKs, accepts or sends.
3. **Cooperative continuity (C1, `ht-rzi.2`).** The harness session id from the hook payload is persisted
   on `occupant_bindings`. A top-level *lifecycle* check-in in a pane whose target is held or unowned, and
   whose session id equals the last binding session of exactly one unresolved nonretired seat, rebinds that
   seat to the target in the deciding transaction (releasing the hold, recording provenance
   `cooperative_continuity` in seat history). Herdr's own `agent_session` report (`pane.report_agent_session`
   from Herdr's integration hooks, visible via `agent get`/`agent list`) is a **hint**: when present it must
   equal the claimed session id or the reattachment is refused to the operator path; when absent the claim
   alone suffices (Herdr may lack the integration, e.g. Codex panes observed without it). A Herdr
   `session_start_source` of `startup`/`new`/`clear` (a fresh session) also refuses reattachment. Never
   proof, never on receipts. Schema change via the next migration number; coordinate numbering with B4.
4. **Agent-to-human refusal (A4).** Enforced in the daemon's lifecycle check-in decision; the CLI
   environment-marker check in `me init` is advisory hardening on top.
5. **Expected boot (A2).** Optional envelope field; absent means today's behaviour (programmatic clients
   keep working). The daemon refuses a mismatch before dispatch with a definite error.
6. **Binding carry-forward (C4).** Applied only by structural reconfirm (same terminal, Herdr boot and
   incarnation); an incarnation change still unresolves.

## Coordination

The B4 run (`.worktrees/remaining-herdr-threads-findings`) edits `src/ports.rs`, `src/store/seats.rs`,
`src/identity/reconcile.rs` and adds a migration. Whichever branch lands on `main` second resolves the
conflicts; this run does not wait on it.


### Normative policy (TRUST-POLICY.md, referenced by the root spec)
# herdr-threads trust policy

Status: adopted 2026-10-01. Normative for seat continuity, caller attribution, receipt provenance and
operator repair. Where an older design document requires adversarial proof of who is calling, this policy
supersedes it. Guards marked **required** below are decided but not yet implemented; they are tracked under
the beads epic `ht-rzi`.

## Abstract

herdr-threads gives coding agents in Herdr panes a durable mailbox: threads, invitations, messages that a
named recipient must explicitly ACK, deadlines, and a safe wake when something is pending. Every one of those
features rests on two questions the system must answer about each request: *which seat is this*, and *who
acted*. A seat is a long-lived role ("the reviewer in the right-hand pane") that has to outlive everything
that happens to the processes behind it: daemon restarts, Herdr restarts, pane moves, `/clear`, session
resume, and an operator's repairs. An ACK is only worth recording if it says something true about who gave it.

The first design answered both questions adversarially: the daemon would prove, service-side, that a caller
was the top-level agent of its pane and not a subagent, a script or another agent impersonating it. That proof
needed evidence the host does not provide (current-execution attestation, synchronous transcript roots) and it
defended against an opponent who does not exist in the deployment: every agent, script and daemon involved
runs as the same local user, on the same machine, on behalf of the same person. Code running as that user can
already read the database, edit the files and type into the panes. A boundary against it is theatre.

The vision is therefore **cooperative honesty with conservative continuity**:

1. **Cooperate, don't police.** Agents are told what they may do (the top-level agent accepts and ACKs;
   subagents read and summarize) and are trusted to do it. The system does not try to stop a same-user
   program from lying. It refuses to fabricate evidence it does not have.
2. **Record claims as claims.** Every receipt says *how* it was attributed: a prompted top-level claim, a
   person who declared their pane human, a local-account operator decision. Provenance is the product; a
   reader of the record must be able to tell an agent's ACK from a person's.
3. **Never lose a role silently, never invent one.** When continuity is uncertain the system holds and asks;
   it does not guess, merge, or let a newcomer take a seat's place. Losing a seat strands its threads and
   obligations; misattributing one corrupts the record. Both are worse than a short wait for the operator.
4. **Decide against one view.** Each decision is made by the daemon, inside one transaction, against one
   canonical picture of the world: the daemon boot the caller addressed, the seat's current binding, and the
   effective host observation. Clients and local files are hints.
5. **Use only evidence that exists.** A transition may depend only on evidence the production host adapter
   actually produces. Heuristics may suggest; they never move a seat or end a binding.

What follows turns that vision into invariants, names who may do what, and states the accepted limits
explicitly, so that a weakness is a documented decision rather than a surprise.

## Vocabulary

- **Seat**: a durable plugin identity for one role. Threads, invitations and receipts belong to seats.
- **Target**: a Herdr pane address the seat is currently mapped to. A locator, not an identity.
- **Binding**: the seat's current occupant record: harness (`claude`, `codex`, `human`), harness session,
  execution, generation, the daemon boot and host epoch it was recorded under, and its provenance.
- **Herdr boot / incarnation**: one Herdr server process. A Herdr restart is a new incarnation; terminal ids
  and pane addresses from the old one prove nothing about the new one.
- **Daemon boot**: one herdr-threads daemon process. A daemon restart does not change the Herdr incarnation.
- **Operator**: a request made with `--operator`, accepted when the kernel peer UID matches the daemon owner.
  This is the local account, not proof of a person.

## Continuity invariants

**C1. How a seat's mapping may change.** Exactly three ways:

| Path | Trigger | Recorded as |
|---|---|---|
| Structural reconfirm | Same terminal, same Herdr boot and incarnation | automatic, no new provenance |
| Cooperative continuity | A top-level check-in whose harness session id uniquely matches an unresolved seat's last binding | `cooperative_continuity` |
| Operator decision | `seat rebind`, `seat resolve --new-seat`, `seat retire`, `seat rebind --replace` | `operator:local-user:<uid>` |

Seats are never merged. Pane labels, saved addresses, terminal-id hints and Herdr's agent field may *suggest*
candidates in diagnostics; they never move a seat, allocate one, or end a binding.

**C2. Restore holds.** After a new or unknown Herdr incarnation while nonretired seats exist, every saved
seat whose continuity is not structurally proven becomes unresolved, and the daemon takes a coherent baseline
of current targets. Every unowned target in that baseline is **held**: ordinary resolution (`seat resolve`,
`launch`, `me init`) refuses it with inspect / rebind / fresh-seat guidance. Panes authoritatively created
after the baseline are not held. Holds are durable and survive daemon restart. A hold is released by:
operator rebind or fresh seat on that target; cooperative continuity on that target (C1); or, instance-wide,
when no unresolved nonretired seats remain (**required**, nothing left to protect).

**C3. Collisions resolve by abandonment, never by merge.** When a rebind finds its target owned by another
live seat, the refusal offers exactly two resolutions, each as ready argv:
- abandon the old seat: `seat retire OLD --operator` (**required**: new command);
- abandon the new role: `seat rebind OLD --pane P --replace NEW --operator` (**required**), which retires NEW
  and rebinds OLD in one decision so the target cannot be claimed in between. NEW's pending obligations settle
  as recipient-retired; nothing moves from NEW to OLD.

**C4. Availability ends only on evidence.** A joined seat stays available across a daemon restart when its
mapping is structurally reconfirmed; the open binding carries forward to the new host epoch (**required**,
today it lapses until the agent's next check-in). Availability ends when the mapping becomes unresolved, the
seat retires, or a check-in replaces the binding.

**C5. Only producible evidence drives transitions.** The production Herdr adapter reports structure
(terminal, incarnation, generation) but not occupancy or current execution. Code paths that need an observed
empty shell or a verified execution do not exist in production and are removed, not kept as dead branches
(**required**, folded into the removal of the pre-cooperative verification layer). Consequence: an agent that
exits back to its shell stays bound until a check-in replaces it or its pane disappears.

## Attribution invariants

**A1. The caller is the pane's seat, cooperatively.** A command acts as the seat mapped to `HERDR_PANE_ID`
(never the focused pane), or as the seat named by an explicit selector. Any process running as the user can
act as any seat: a child agent, a script, another seat's agent writing the shared client directories. This is
in-contract (see Accepted limits).

**A2. One canonical decision view.** Every accountable mutation is decided by the daemon, in its deciding
transaction, against:
- the daemon boot the client addressed: a request carrying a different expected boot is refused before
  dispatch (**required**, today only the response is checked);
- the seat being resolved, nonretired and not on a held target;
- the exact binding generation, harness, harness session and execution of the open binding;
- the effective host observation (published snapshot or newer current-target read) at the current host boot
  and epoch, never the raw per-target rows.

Client-local state (`contexts/`, `intents/`) only selects what to ask; it never authorizes.

**A3. Provenance values.** Exactly these, each with one meaning:

| Value | Where | Meaning |
|---|---|---|
| `cooperative_top_level` | bindings, receipts | The pane's top-level agent claimed the action through its hook-registered check-in. Prompted, not proven: a disobedient child is indistinguishable. |
| `operator_human` | bindings, receipts | A person declared this pane human with `me init` and acted from it. Best effort: refused where the system sees evidence of an agent (A4). |
| `cooperative_continuity` | seat rebinds only | The seat was reattached because a resumed harness session id matched (C1). Never on receipts. |
| `operator:local-user:<uid>` | audit of administrative decisions | The local account made a repair or recovery decision. Never on receipts. |

**A4. Binding-kind transitions.**
- *Human to agent*: a hooked agent's lifecycle check-in replaces a human binding. Allowed.
- *Agent to agent, same seat*: a lifecycle check-in (startup, `/clear`, resume) replaces the binding. Allowed.
- *Agent to human*: the daemon refuses a human lifecycle check-in while the open binding is
  `cooperative_top_level`, unless the request is `--operator` (**required**). `me init` additionally refuses
  when agent environment markers are present (`CLAUDECODE`, `CODEX_*`) or Herdr reports an agent in the pane
  (**required**, best effort, client-side).
- *Second agent*: `launch` refuses to start an agent for a seat whose bound agent Herdr reports live in
  another pane (**required**).
- *Wake*: a wake prompt goes only to an agent of the bound harness (**required**).

**A5. Who may do what.**

| Action | Who |
|---|---|
| ACK, accept, send, leave, archive, reopen | the seat's current binding (top-level agent or declared human) |
| check in | the pane's top-level agent (hook) or a human via `me init` |
| rebind, fresh seat, retire, replace, orphan-thread invite | operator |
| anything else on behalf of another seat | nobody by design; possible by spoofing (Accepted limits) |

The operator never ACKs, accepts, sends or advances a checkpoint.

## Accepted limits

These are decisions, not bugs. Each is safe to rely on only as stated.

- **Child agents can ACK.** Subagents are instructed not to; the plugin cannot tell them from their parent.
- **Same-user spoofing.** Any same-user process can select another seat, set `HERDR_PANE_ID`, or write another
  seat's files under the instance `contexts/` and `intents/` directories (writable from the Codex sandbox by
  design). Forged client files cannot satisfy A2's binding match, but they can disrupt that seat's local
  state. Sandbox-writable files are opened without following symlinks (**required** for `allocator.lock`).
- **Unseen exits.** An agent that exits to its shell stays bound until the next check-in or retirement (C5).
- **Launch is not check-in.** A successful `launch` means Herdr started the agent, not that it registered.
  Launch forms without captured hook evidence are refused (**required** for `codex resume`).
- **Restore costs operator time.** A genuinely new role in a restored pane waits for an explicit choice when
  cooperative continuity does not apply.
- **Probabilistic identifiers.** Pagination cursor binding tags are 48 bits (about 2^-48 acceptance per forged
  or stale cursor; scope, direction and order are still compared exactly). Send-preparation ids are drawn from
  62^8 ≈ 2^47.6; reuse of a retired id has probability about (retired ids) × 2^-47.6 and is harmless once
  nothing references it.

## Decision record

### F6: restored-pane creation versus repair reservation (resolved 2026-10-01)

Roast 1 left open whether a restored pane may get a fresh seat before the operator repairs the old one. The
dissent mattered most under the adversarial model; under this policy the risk is a stranded or misattributed
seat, never a compromise. Decision: keep restore holds (C2, already implemented), make them lift when nothing
is left to repair, add cooperative continuity so a resumed session reattaches itself (C1), and resolve
collisions only by explicit abandonment (C3). Rejected: optimistic allocation with merge (merges are the
misattribution this policy exists to prevent); narrowing holds by address or terminal hints (unreliable across
an incarnation, so either F6 again or the same holds); no reservation (loses obligations on every Herdr
restart).

### Residual trust-edge findings (bucket B5)

| Finding | Disposition | Invariant |
|---|---|---|
| F6 restore holds | guards + docs | C1, C2, C3 |
| P10 occupancy never EmptyShell | remove dead paths with the verification-layer removal | C5 |
| W5-1 decision fence on raw rows | removed with native permits | A2 |
| W5-2 `stage_recipient` unpinned | add the missing generation-change test | C4 |
| W6-C3 `codex resume` launch uncaptured | refuse until captured | Accepted limits |
| P35 request crosses daemon boot | expected boot in the request envelope | A2 |
| O1 unavailable after daemon restart | carry binding forward on structural reconfirm | C4 |
| Wave 18 `me init` as `operator_human` | env-marker and Herdr-agent refusal | A3, A4 |
| Wave 18 `me init` replaces agent binding | daemon refuses agent-to-human without `--operator` | A4 |
| Wave 30 shared client directories | accepted, documented | A1, Accepted limits |
| `allocator.lock` without `O_NOFOLLOW` | open without following symlinks | Accepted limits |
| Wave 28 second agent via name retry | launch refuses while bound agent is live | A4 |
| W9-2 wake ignores harness | compare bound harness with pane agent | A4 |
| W6-R2 "mark it self" wording | "when run in this pane" | A1 |
| Waves 29/19 identifier sizes | accepted; fix the 104-bit comment (it is 112) | Accepted limits |


## Task tree
### ht-rzi (epic) — B5: Cooperative trust policy guards (TRUST-POLICY.md)
blocking deps: (none)

Implement the required guards decided in TRUST-POLICY.md (adopted 2026-10-01, branch trust-model-invariants). Resolves F6 and the B5 bucket of docs/history/remaining-findings-2026-10-01.md. P10 and W5-1 are not here: they are deletions folded into B4 (ht-p03.2). Each child names the policy invariant it implements; a change that weakens an invariant must update TRUST-POLICY.md in the same commit.

acceptance: 

### ht-rzi.1 — F6 repair path: hold lift, seat retire, rebind --replace
blocking deps: (none)

TRUST-POLICY C2/C3. (1) Clear host_instances.baseline_hold_unclaimed once no unresolved nonretired seat remains (today sticky until next recovery epoch: src/store/seats.rs:684 and the set at :1589); evaluate in the transactions that resolve/retire the last unresolved seat. (2) New operator command 'seat retire SEAT --operator' using the existing bounded retirement cutover (begin_retirement path in src/store/seats.rs; today retirement only comes from reconciliation of an absent target). (3) 'seat rebind OLD --pane P --replace NEW --operator': retire NEW and rebind OLD onto P in one deciding transaction; nothing moves NEW->OLD; NEW's pending obligations settle as recipient-retired. (4) The rebind refusal for an owned target carries both resolutions as argv. Files: src/store/seats.rs, src/store/effective.rs, src/cli/commands.rs, src/protocol/commands.rs + results.rs, docs/operations.md, docs/agent-usage.md.

owns: hold-lift predicate (clear host_instances.baseline_hold_unclaimed when no unresolved nonretired seat remains), seat retire --operator, seat rebind --replace --operator, collision refusal argv. consumes: existing begin_retirement cutover. files: src/store/seats.rs, src/store/effective.rs, src/store/schema.rs, src/protocol/commands.rs, src/protocol/results.rs, src/cli/commands.rs, docs/operations.md, docs/agent-usage.md.

acceptance: Tests: hold lifts after last unresolved seat is rebound or retired (and not before); retire --operator retires and settles obligations; rebind --replace is atomic (no window where P is unowned, concurrent resolve loses); refusal error lists both argv; operator audit label recorded; non-operator calls get operator_required.

### ht-rzi.2 — Cooperative continuity: session-id reattachment of unresolved seats
blocking deps: (none)

TRUST-POLICY C1. Persist the harness session id (Claude session_id, Codex session id) on occupant_bindings at check-in (hooks already parse it: src/harness/claude.rs:130, src/harness/codex.rs:979; it is not stored today). When a top-level lifecycle check-in arrives in a held or unowned target and its session id uniquely matches the last binding of exactly one unresolved nonretired seat, rebind that seat to the target, release the hold, record provenance cooperative_continuity (new constant in src/protocol/authority.rs, never on receipts). No match / multiple matches / fresh session -> unchanged operator path. The hook today never allocates (src/cli/hook.rs find_seat); this is a new daemon-side decision, not client-side. Herdr's agent list exposes agent_session for claude panes (source herdr:claude): consider cross-checking it as a consistency hint, never as proof. First step: capture native evidence that Claude --resume/--continue and Codex resume keep the session id across a Herdr restart (docs/compatibility).

owns: occupant_bindings harness session column + migration, cooperative_continuity provenance constant, session-id reattachment decision in the lifecycle check-in path, Herdr agent_session hint comparison, docs/compatibility evidence note. consumes: hold-lift predicate (owned by ht-rzi.1) — call it after reattachment if it exists, else release the per-target hold only. files: src/store/seats.rs, src/store/schema.rs, migrations/, src/protocol/authority.rs, src/protocol/commands.rs, src/cli/hook.rs, src/harness/claude.rs, src/harness/codex.rs, src/host/native.rs, docs/compatibility/. Spec: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-design.md decision 3.

acceptance: Captured evidence for both harnesses' resume session-id stability (or the harness excluded). Tests: unique match reattaches and releases the hold; zero and multiple matches leave the seat unresolved and the pane held; retired seats never match; provenance cooperative_continuity appears in seat history and never on receipts; daemon restart mid-way is idempotent.

### ht-rzi.3 — Agent-to-human binding guards for me init (Wave 18)
blocking deps: (none)

TRUST-POLICY A3/A4. (1) Daemon-side: refuse a lifecycle check-in with harness=human while the seat's open binding has provenance cooperative_top_level, unless the request is --operator (store lifecycle path src/store/seats.rs ~3671-3695 ends any open binding today). Daemon-side because a forged contexts/ file can bypass the CLI (Wave 30). (2) Client-side best effort in src/cli/me.rs (and derive_selection in src/cli/mod.rs ~1064 when the local context is Human): refuse when CLAUDECODE or CODEX_* env markers are present, or Herdr reports an agent in the pane (one pane read). Guidance: --new-seat --operator.

owns: daemon refusal of agent-to-human lifecycle check-in without --operator; me init / derive_selection agent-marker refusal. files: src/store/seats.rs (lifecycle check-in), src/cli/me.rs, src/cli/mod.rs, docs/operations.md.

acceptance: Tests: me init over a live cooperative_top_level binding is refused with operator guidance; --operator path allowed and audited; human->agent check-in still replaces the human binding; me init with CLAUDECODE set refuses; flagless command from a Human context with agent markers refuses.

### ht-rzi.4 — Launch and wake harness guards (Wave 28, W9-2, W6-C3)
blocking deps: (none)

TRUST-POLICY A4 / Accepted limits. (1) Wave 28: launch_managed (src/harness/launch.rs ~440-560) refuses when the seat's open binding is agent provenance and Herdr reports a claude/codex agent in that binding's pane (seat inspect + agent get); the name suffix retry (src/host/native.rs:210-216) then cannot start a second agent for the seat. (2) W9-2: carry the binding harness into SafeWakeTarget (src/ports.rs ~1533) and compare with agent.agent in cooperative_wake_ready (src/host/native.rs ~964-994). (3) W6-C3: refuse the top-level 'codex resume' launch form (src/harness/launch.rs:283) like fork/review until a live capture exists. Coordinate with B6 ht-p03.28 (launch correctness) to avoid overlapping edits.

owns: launch live-bound-agent refusal, SafeWakeTarget harness field + cooperative_wake_ready comparison, codex resume launch-form refusal. files: src/harness/launch.rs, src/cli/launch.rs, src/ports.rs (SafeWakeTarget), src/host/native.rs, src/store/wake.rs.

acceptance: Tests: launch onto a seat whose bound agent is live elsewhere is refused with a clear message; wake to a pane whose agent kind differs from the bound harness is refused (no prompt sent); 'codex resume' launch refused with evidence message.

### ht-rzi.5 — Daemon-restart continuity: expected boot in requests, binding carry-forward (P35, O1, W5-2)
blocking deps: (none)

TRUST-POLICY A2/C4. (1) P35: add optional expected_boot to the WireRequest envelope (src/client/local.rs ~155, src/protocol/wire.rs); LocalSocketClient already holds descriptor.boot_id; the daemon refuses a mismatch before dispatch with a definitive DaemonBootChanged error (client maps it to a retryable definite rejection, not UnknownOutcome). (2) O1: when a structural reconfirm (same terminal, Herdr boot and incarnation) is applied after a daemon restart, carry the open cooperative binding forward to the new host epoch (Reconfirm/ReconfirmStructure apply in src/store/seats.rs ~1811-1840, ~2087-2096; effective_registered_availability src/store/schema.rs:1356), so the first send no longer warns recipient_unavailable. (3) W5-2: store test for current binding + bumped snapshot generation -> available_at NULL and exactly one prepared_unavailable_warnings row (src/store/messages.rs ~421).

owns: WireRequest expected_boot field and pre-dispatch refusal error code, binding carry-forward on structural reconfirm, W5-2 store test. files: src/protocol/wire.rs, src/protocol/results.rs (error code), src/client/local.rs, src/daemon/ (dispatch), src/store/seats.rs (reconfirm apply), src/store/schema.rs, tests/store/receipts.rs or tests/service/cooperative.rs.

acceptance: Tests: request with stale expected_boot refused pre-dispatch, nothing applied; daemon stop/ensure then send to a joined, structurally reconfirmed seat starts its timer with no warning; a Herdr-incarnation change still makes it unresolved; W5-2 test present.

### ht-rzi.6 — Trust-edge hygiene: O_NOFOLLOW allocator.lock, wording, id comments
blocking deps: (none)

TRUST-POLICY Accepted limits. (1) src/cli/journal.rs:467 private_open (:1066-1072) uses secure_options (O_NOFOLLOW) like the context journal (src/harness/context.rs ~893). (2) W6-R2: src/harness/mod.rs:243 hint -> '...mark it as self when run in this pane'. (3) src/protocol/ids.rs:133 comment says 104 random bits; it is 112. (4) Comment the send-preparation id reuse bound (62^8 ~ 2^47.6; src/store/public_ids.rs fresh()).

files: src/cli/journal.rs, src/harness/mod.rs, src/protocol/ids.rs, src/store/public_ids.rs.

acceptance: allocator.lock leaf symlink is refused (test); hint text updated with its test; comments corrected.


## Precomputed graph checks
summary: flag-sweep 0 · unstated 0 · citations 0 (dependent 0, unwired 0, unknown 0)


## Changes since the previous round
(empty — round 1)

## Coverage ledger
(empty)

## Requirements (canonical)
R1 The instance-wide restore hold lifts once no unresolved nonretired seat remains (C2)
R2 `seat retire SEAT --operator` exists and retires a seat through the bounded retirement cutover (C3)
R3 `seat rebind OLD --pane P --replace NEW --operator` retires NEW and rebinds OLD in one decision (C3)
R4 A rebind refused because the target is owned carries both resolutions as argv (C3)
R5 A top-level lifecycle check-in whose harness session id uniquely matches an unresolved seat's last binding reattaches that seat with `cooperative_continuity` provenance; ambiguity, a fresh session, or a contradicting Herdr agent_session hint falls back to the operator path (C1)
R6 A joined seat stays available across a daemon restart when its mapping is structurally reconfirmed (C4)
R7 A request addressed to a different daemon boot is refused before dispatch (A2)
R8 The daemon refuses an agent-to-human lifecycle check-in over an open cooperative_top_level binding unless --operator (A4)
R9 `me init` refuses when agent environment markers are present or Herdr reports an agent in the pane (A4)
R10 `launch` refuses to start an agent for a seat whose bound agent is live in another pane (A4)
R11 A wake prompt goes only to an agent of the seat's bound harness (A4)
R12 `allocator.lock` is opened without following symlinks (Accepted limits)
R13 The `codex resume` launch form is refused until captured (Accepted limits)
R14 A test pins stage_recipient staging unavailable when the effective observation generation differs (W5-2)
R15 The self-marker hint says "when run in this pane"; ids.rs comment says 112 bits; prep-id reuse bound is commented
R16 Operator and agent docs (docs/operations.md, docs/agent-usage.md, README) describe the new commands and behaviours

