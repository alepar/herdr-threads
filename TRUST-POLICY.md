# herdr-threads trust policy

Status: adopted 2026-10-01; B5 guards implemented (epic `ht-rzi`). Normative for seat continuity, caller
attribution, receipt provenance and operator repair. Where an older design document requires adversarial
proof of who is calling, this policy supersedes it. C5's guard is owned by B4 (`ht-p03.2`) and is the one
guard still marked **required**.

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
| Cooperative continuity | A resumed top-level session (SessionStart source `resume`) whose harness session id uniquely matches an unresolved seat's last binding | `cooperative_continuity` |
| Operator decision | `seat rebind`, `seat resolve --new-seat`, `seat retire`, `seat rebind --replace` | `operator:local-user:<uid>` |

Seats are never merged. Pane labels, saved addresses, terminal-id hints and Herdr's agent field may *suggest*
candidates in diagnostics; they never move a seat, allocate one, or end a binding.

**C2. Restore holds.** After a new or unknown Herdr incarnation while nonretired seats exist, every saved
seat whose continuity is not structurally proven becomes unresolved, and the daemon takes a coherent baseline
of current targets. Every unowned target in that baseline is **held**: ordinary resolution (`seat resolve`,
`launch`, `me init`) refuses it with inspect / rebind / fresh-seat guidance. Panes authoritatively created
after the baseline are not held. Holds are durable and survive daemon restart. A hold is released by:
operator rebind or fresh seat on that target; cooperative continuity on that target (C1); or, instance-wide,
when no unresolved nonretired seats remain (implemented, nothing left to protect).

**C3. Collisions resolve by abandonment, never by merge.** When a rebind finds its target owned by another
live seat, the refusal offers exactly two resolutions, each as ready argv:
- abandon the old seat: `seat retire OLD --operator` (implemented);
- abandon the new role: `seat rebind OLD --pane P --replace NEW --operator` (implemented), which retires NEW
  and rebinds OLD in one decision so the target cannot be claimed in between. NEW's pending obligations settle
  as recipient-retired; nothing moves from NEW to OLD.

**C4. Availability ends only on evidence.** A joined seat stays available across a daemon restart when its
mapping is structurally reconfirmed; the open binding carries forward to the new host epoch (implemented). Availability ends when the mapping becomes
unresolved, the seat retires, or a check-in replaces the binding.

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
  dispatch (implemented);
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
  `cooperative_top_level`, unless the request is `--operator` (implemented). `me init` additionally refuses
  when agent evidence is present: one of the three allowlisted environment markers (`CLAUDECODE`,
  `CODEX_SANDBOX`, `CODEX_SANDBOX_NETWORK_DISABLED`), or Herdr reporting a Claude or Codex agent in the
  pane. Other agent kinds are not evidence, and a failed Herdr read counts as no evidence. `--operator`
  overrides the refusal through a local per-execution mark set only after the daemon accepts the request
  (implemented, best effort, client-side).
- *Second agent*: `launch` refuses to start an agent for a seat whose bound agent Herdr reports live in
  another pane (implemented).
- *Wake*: a wake prompt goes only to an agent of the bound harness (implemented).

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
  state. Sandbox-writable files are opened without following symlinks (implemented for `allocator.lock`).
- **Unseen exits.** An agent that exits to its shell stays bound until the next check-in or retirement (C5).
- **Launch is not check-in.** A successful `launch` means Herdr started the agent, not that it registered.
  Launch forms without captured hook evidence are refused (implemented for `codex resume`).
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
