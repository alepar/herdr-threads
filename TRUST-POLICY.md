# herdr-threads trust policy

Status: adopted 2026-10-01; B5 guards implemented (epic `ht-rzi`). Normative for seat continuity, caller
attribution, receipt provenance and operator repair. Where an older design document requires adversarial
proof of who is calling, this policy supersedes it. C5's guard is owned by B4 (`ht-p03.2`) and is the one
guard still marked **required**. Amended 2026-10-02 for thread summaries and deadline extension (epic ht-1ip).

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
mapping is structurally reconfirmed; the open cooperative or operator binding carries forward to the new host epoch (implemented; a native binding re-registers instead, and a send before the first reconciliation pass assumes the carry rather than warning). Availability ends when the mapping becomes
unresolved, the seat retires, or a check-in replaces the binding.
Herdr not answering (a timed-out or refused connection, a read that finishes past its budget) and a
daemon-side failure to stage a capture are missing evidence, not evidence of change: they write no host
invalidation, so seats, bindings and the published view stay **frozen** until Herdr answers (implemented,
ht-yms). Meanwhile anything that needs a live Herdr read (current-target resolution, continuity, wake
prompts) is refused as transient; store decisions that need no host read (sends, ACKs) proceed. Only
evidence invalidates: an incomplete enumeration, an unknown or new incarnation (C2), an incoherent capture.

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
| `derived_summary` | summary blocks only | The block was written by an agent acting for the seat (the top-level agent or a child summary worker, which the CLI cannot tell apart) under the seat's claim, with the model it declared. Never on receipts, never delivery, and never authority for any state change other than storing that block. Submission validation bounds what a block can claim. |

**Recorded message claims.** `messages.author_role` (`human`, `agent`, `service`) and `messages.relays_user`
are claims recorded at send time in the deciding transaction: `author_role` is `service` for a programmatic
sender, otherwise it comes from the sender's open binding (a `human` harness binding, i.e.
`operator_human`, gives `human`; any other binding gives `agent`); a sender with no open binding and seat-less
system events record none. Messages written before the
summary migration carry a **backfilled** role (`author_role_backfilled = 1`) derived from the sender's binding
covering the message's decision time, and none (read as `agent`) where no binding covers it. `relays_user` is
set by `send --relays-user`; like everything an agent sends it is a cooperative claim (A1); service sends
record 0 and pre-migration rows are 0. Neither field authorizes anything; together they only rank a message as
*priority* for summaries and for the catch-up hold bypass.

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
- *Poke*: a soft-deadline poke is the fixed reminder
  `herdr-threads: receipt due in <N>s on <thread-ids>; run herdr-threads inbox` (thread ids only),
  submitted through the wake dispatcher and its limits to the seat's bound native agent of the bound
  harness. It is decided from a fresh observation immediately before the prompt, only when the pane is not
  focused and the agent is idle, or in an active turn or with typed input only where the harness recipe
  declares `poke_during_turn` or `composer_stash` from captured evidence
  (`docs/evidence/poke-spike/findings.md`; the recipe that lists the installed version the daemon's
  admission observer last observed, re-observed when the binary changes, so an unobserved version or one
  the admission ladder admits only optimistically, or one it refuses, declares neither; harness version
  evidence (verified by use) never adds a declaration). It is never sent in an approval or
  question state or an unknown state. A poke carries no authority and changes no receipt state other than
  `soft_poked_at`, which is set only when the host accepts the prompt; the hard-deadline warning stays the
  backstop. When an ordinary wake is due for the same seat, one prompt goes out with the poke text, and only
  when the poke itself is eligible and needs no stash.
  - *Idle is read, not assumed.* Herdr's `agent_status` does not show typed input, so the adapter also reads
    the composer (`agent read --source detection`). The state is idle only when `agent_status` is `idle` or
    `done` and the composer is empty; composer text is typed input (human input); a `working` status with an
    empty composer is an active turn, and with composer text it is unknown (a poke would merge into the
    draft); `blocked` is an approval or question. Claude's placeholder counts as empty only when it is exactly
    a captured placeholder (`docs/evidence/poke-spike/findings.md`); any other placeholder-shaped row is
    unknown and skipped. Claude Code draws a prompt suggestion in the composer after a turn, and the
    detection text cannot tell it from typed input; until its styling is captured, any Claude composer text
    other than a captured empty marker is not known empty and is never stashed. An unreadable composer, a failed read or any other status is
    unknown and skipped. The same reader classifies the pane and drives the stash, so they cannot disagree.
    Ordinary wakes are unchanged: composer content never refuses an ordinary wake or advances its retry
    step. An ordinary wake goes out over typed input or a prompt suggestion as it did before the composer
    reader existed (it merges with a draft; see Accepted limits); an active turn or an approval or question
    refuses it, as Herdr's idle/done recheck always did. Only a poke is skipped when the composer cannot be
    classified or stashed with confidence, and the skip is for that poke only.
  - *Composer stash.* Where a recipe declares `composer_stash`, a poke into a pane with typed, unsent input
    first reads and clears that input, then submits the poke, then retypes the saved text without submitting
    it. A failure before the poke aborts it with nothing submitted; a failed retype is recorded as a
    diagnostic and the saved text is kept in the daemon log for the operator, never discarded. The stash is
    refused (the poke skipped) when the composer holds an image or pasted-text placeholder, which does not
    survive a retype, when a composer row's display width (Unicode width, East Asian ambiguous characters counted
    wide) lies within 12 columns of the pane width (a soft wrap cannot be told from a newline), when a row
    holds a character whose rendered width cannot be determined (a control character, an emoji variation
    selector or joiner), when a composer row ends in whitespace, when the pane width is unknown and the
    harness shows no rule from which to infer it, or when the harness is Claude and the composer holds any
    text (not known empty, above).
    The stash clears with a bounded `ctrl+u` loop and proceeds only when a second read shows the composer
    empty; if that never happens the poke is aborted and the typed text, which may already be partly cleared,
    is kept in the daemon log.
  - *During a turn.* Where a recipe declares `poke_during_turn`, the poke is submitted with `agent prompt`,
    which queues it into the running turn at the next tool boundary (steering, not a separate user turn). The
    adapter's recheck allows a working agent for that call only.
  - *Post-send verification.* The wake path's one-shot post-send check (read the composer, press the
    submit key once if the prompt is still held) is skipped for a poke queued into a running turn and for a
    poke whose stashed draft was retyped: there the composer legitimately holds text, and a submit key
    would send the person's draft.
  - *Declarations.* Claude 2.1.287 declares both (spike evidence). No Codex recipe declares either: the
    spike tested Codex 0.160.0 (mock provider), which no recipe covers, so Codex seats are poked only when
    idle with an empty composer. Claude's `composer_stash` declaration currently never stashes, because no
    Claude composer text is known to be a typed draft.
  - *Retry spacing.* A seat whose poke was skipped or failed is re-evaluated no sooner than the wake retry
    spacing afterwards, so an ineligible seat costs one observation per spacing rather than one per tick.
    The spacing is in memory; a restart re-evaluates once.

**A5. Who may do what.**

| Action | Who |
|---|---|
| ACK, accept, send, leave, archive, reopen | the seat's current binding (top-level agent or declared human) |
| check in | the pane's top-level agent (hook) or a human via `me init` |
| rebind, fresh seat, retire, replace, orphan-thread invite | operator |
| summary, summary job, summary submit | the seat's binding or its children (summary workers), all under the seat's claim: read-mostly; submit only stores a validated block for a live lease issued to the seat |
| anything else on behalf of another seat | nobody by design; possible by spoofing (Accepted limits) |

The operator never ACKs, accepts, sends or advances a checkpoint.

**A6. Effective receipt deadlines.** A receipt's effective deadline is the later of its frozen deadline and
the `extension_until` of the latest catch-up row for the receipt's (seat, thread). The daemon decides it (A2)
from catch-up state the seat itself entered through its accountable claim (a `summary` call that returned
work). It is extended only by stored summary progress (entry, each newly stored block, and the exit grace),
never on heuristic evidence, and frozen deadlines are never rewritten: they stay stored and displayed beside
the effective one. Overdue classification, warnings, pending receipts and the soft-deadline poke use the
effective deadline.

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
- **Any invocation of a seat can enter catch-up.** CLI calls cannot tell a seat's top-level agent from its
  child workers, so any `summary` call by the seat may open its catch-up row. Harmless: workers exist only
  after the parent's own `summary` entered the row, and an active row keeps its frontier, so repeated or
  worker calls never move it.
- **Summary blocks name the seat, not the invocation.** A block records its author seat and declared model;
  it does not record whether a child wrote it, because the CLI cannot know.
- **No poke into typed input or a running turn where no recipe declares it.** A harness version whose
  recipe declares neither `composer_stash` nor `poke_during_turn` (every Codex version, and every Claude
  version except 2.1.287; see docs/evidence/poke-spike/findings.md): a seat whose agent is working or whose
  composer holds typed input is skipped, and only the hard-deadline warning reaches it. Where `composer_stash`
  is declared, a draft with an image or pasted-text placeholder, a row whose display width is near the pane width or cannot be determined, or an unknown
  pane width (Codex shows no rule to infer it from) is skipped the same way.
- **Pokes skip Claude panes that show composer text.** A Claude pane whose composer shows a prompt
  suggestion (or any typed text) is not poked: the suggestion cannot be told from a draft without captured
  styling. The skip is for that poke only and is retried after the wake retry spacing; only the
  hard-deadline warning reaches a pane that keeps showing a suggestion (ht-1ip.46).
- **Ordinary wakes merge with a draft.** An ordinary wake into an idle agent pane that holds a typed, unsent
  draft submits the wake text merged with the draft (`agent prompt` merges, poke spike Q5), as before the
  composer reader existed; only soft-deadline pokes stash and restore a draft.
- **Composer reads see the screen, not the buffer.** Herdr's detection read shows composer rows without
  trailing whitespace, so whitespace typed at the end of a draft row is invisible: a stashed draft is retyped
  without it, and a draft of only spaces reads as empty and is poked over. A draft typed to match a captured
  Claude placeholder exactly reads as empty the same way. A Claude placeholder that was never captured reads
  as unknown, so that pane is not poked (only the hard-deadline warning reaches it) until the placeholder is
  captured and listed.
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

### Thread summaries and deadline extension (2026-10-02)

Summaries are agent-produced, daemon-validated data under `derived_summary`; priority comes from recorded
claims (`author_role`, `relays_user`), not from the text. Catch-up extends effective deadlines only on stored
progress (A6). The A4 poke rule (soft-deadline pokes) is defined with its behaviour above.

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
| ht-yms Herdr timeout ended every binding | unavailability freezes state, writes no invalidation | C4 |
| Wave 18 `me init` as `operator_human` | env-marker and Herdr-agent refusal | A3, A4 |
| Wave 18 `me init` replaces agent binding | daemon refuses agent-to-human without `--operator` | A4 |
| Wave 30 shared client directories | accepted, documented | A1, Accepted limits |
| `allocator.lock` without `O_NOFOLLOW` | open without following symlinks | Accepted limits |
| Wave 28 second agent via name retry | launch refuses while bound agent is live | A4 |
| W9-2 wake ignores harness | compare bound harness with pane agent | A4 |
| W6-R2 "mark it self" wording | "when run in this pane" | A1 |
| Waves 29/19 identifier sizes | accepted; fix the 104-bit comment (it is 112) | Accepted limits |
