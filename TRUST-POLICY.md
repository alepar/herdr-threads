# herdr-threads trust policy

Status: adopted 2026-10-01; B5 guards implemented (epic `ht-rzi`). Normative for seat continuity, caller
attribution, receipt provenance and operator repair. Where an older design document requires adversarial
proof of who is calling, this policy supersedes it. C5's guard is owned by B4 (`ht-p03.2`) and is the one
guard still marked **required**. Amended 2026-10-02 for thread summaries and deadline extension (epic ht-1ip), and 2026-10-03 for the
`managed_launch` binding (ht-5n6), and 2026-10-03 for human receipt waivers.

## Abstract

herdr-threads gives coding agents in Herdr panes a durable mailbox: threads, invitations, messages that an
agent recipient may be required to explicitly ACK, deadlines, and a safe wake when something is pending. Every one of those
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
`launch`, top-level `SessionStart` enrollment, `me init`) refuses it with inspect / rebind / fresh-seat guidance. Panes authoritatively created
after the baseline are not held. Holds are durable and survive daemon restart. A hold is released by:
operator rebind or fresh seat on that target; cooperative continuity on that target (C1); or, instance-wide,
when no unresolved nonretired seats remain (implemented, nothing left to protect).

**Startup enrollment.** Installed Claude/Codex top-level `SessionStart` lifecycle hooks enroll
`HERDR_PANE_ID` through the same journaled ordinary canonical guarded seat resolver as `launch`.
It freshly reads the actual target and decides against A2: reuse its resolved seat, or allocate only
when ordinary resolution permits a genuinely new pane. An unresolved mapping, restore hold or collision
is never bypassed by creating, moving, rebinding or retiring a seat. A resumed session attempts C1
cooperative continuity first, even when the client's seat listing appears resolved; pending recovery,
ambiguous matches and unusable recovery evidence stop enrollment. A confirmed no-match may proceed
only through ordinary guards; a resume without a usable native session cannot allocate a seat.
Resolution and check-in retain their separate durable operation journals: concurrent/replayed startup
and response loss cannot split a pane's role; an uncertain resolution remains explicitly retryable.
A historical resolution result is revalidated against the current pane mapping before check-in.
Enrollment itself grants no receipt authority; check-in retains `cooperative_top_level` provenance.
Subagent callbacks, `PreToolUse` and Current/compact checks never allocate; the exact installed event
registration must match the native payload before service or journal access.

**C3. Collisions resolve by abandonment, never by merge.** When a rebind finds its target owned by another
live seat, the refusal offers exactly two resolutions, each as ready argv:
- abandon the old seat: `seat retire OLD --operator` (implemented);
- abandon the new role: `seat rebind OLD --pane P --replace NEW --operator` (implemented), which retires NEW
  and rebinds OLD in one decision so the target cannot be claimed in between. NEW's pending obligations settle
  as recipient-retired; nothing moves from NEW to OLD.

**C4. Availability ends only on evidence.** A joined seat stays available across a daemon restart when its
mapping is structurally reconfirmed; the open cooperative or operator binding carries forward to the new host epoch (implemented; a native binding re-registers instead, and a send before the first reconciliation pass assumes the carry rather than warning). A `managed_launch` binding is never availability, so it is not carried; it stays open and unregistered until a check-in replaces it. Availability ends when the mapping becomes
unresolved, the seat retires, or a check-in replaces the binding.
Herdr not answering (a timed-out or refused connection, a read that finishes past its budget) and a
daemon-side failure to stage a capture are missing evidence, not evidence of change: they write no host
invalidation, so seats, bindings and the published view stay **frozen** until Herdr answers (implemented,
ht-yms). Meanwhile anything that needs a live Herdr read (current-target resolution, continuity) is
refused as transient, and wake prompts and pokes are not attempted: the wake lane is frozen, with no reservation,
refusal record or Herdr call, until the observation lane's first answered capture kicks it (implemented,
ht-72q); store decisions that need no host read (sends, ACKs) proceed. Only
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
| `cooperative_inbox_display` | receipt action observation | The selected top-level agent's text `inbox` command fully wrote and flushed a bounded page before claiming that its listed complete messages were displayed. It is a cooperative output claim, not proof that the model consumed the text. The receipt retains the binding's `cooperative_top_level` provenance separately. JSON, machine, foreign-seat and other read commands do not make this claim. |
| `operator_human` | bindings, receipts | A person declared this pane human with `me init` and acted from it. Best effort: refused where the system sees evidence of an agent (A4). |
| `cooperative_continuity` | seat rebinds only | The seat was reattached because a resumed harness session id matched (C1). Never on receipts. |
| `operator:local-user:<uid>` | audit of administrative decisions | The local account made a repair or recovery decision. Never on receipts. |
| `managed_launch` | bindings only | Herdr's guarded `agent.start` in this pane was observed starting the harness; the agent has not checked in. Never on receipts; authorizes nothing but a wake prompt (an ordinary wake or a soft-deadline poke) to the bound harness. The daemon records it only after the launcher's host-correlated `ObservedStartup` (never on an unconfirmed start), only on a seat with no open binding, decided against the effective observation (A2). Its session and execution are `launch:` placeholders no caller claim can match, it has no `registered_at`, and it starts no availability or receipt timer. |
| `canonical_binding_composer` | archival sample association only | The daemon associated a fresh registered-parser composer read with an already registered cooperative top-level binding after exact canonical binding, structural, activity and timing validation. The harness/session/execution in the association come from that binding; the host does not attest occupancy or current execution. This value authorizes no seat, binding, membership, receipt or invitation transition. |
| `daemon_lifecycle` | automatic archive events only | The daemon applied the configured quiet-channel lifecycle policy after complete canonical validation. No actor seat, binding or receipt claim; no membership or continuity change. |
| `legacy_local_journal_hint` | handoff archival vetoes only | A bounded read-only scan found a pending compound in this instance's intents directory. It can only prevent automatic archival; it authorizes no action and claims no native execution. |
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

**Recorded human intent.** Optional `messages.user_intent` (`query`, `request`, `rule`) is a
cooperative send-time claim under the existing caller provenance, independently of author role and
`relays_user`. The daemon decides eligibility against the canonical send-time binding: ordinary Human
may classify with or without relay, ordinary Agent needs relay; services and system events cannot
classify. Missing intent remains unclassified, including every historical message; there is no
historical inference, body parsing or adversarial verification. Intent changes neither attention
priority nor receipt attribution, and grants no permission or authority.

Under `derived_summary`, classified query/request sources are deterministic open work and rules are
active constraints within their thread. Workers compare supplied live sources (including earlier
unstored chunks) with current chunk messages. Query/request resolution cites a later answer/completion
or explicit human/relayed cancellation/withdrawal/replacement; an agent can report an answer or
completion, but cannot unilaterally withdraw a human question. A replacement question has its own
source. Partial answers, promises, silence and ACKs are insufficient completion evidence. Rules can
only be superseded by later ordinary human/relayed input with `rule_change: withdrawn|replaced` and an
exact nonempty quote; compliant behavior does not end a rule. The worker judges semantic closure and
the daemon validates structure, status and citation evidence, under the existing `derived_summary`
provenance. Unclassified sources retain legacy Done/Superseded behavior without being labeled rules.
No send or inbox operation automatically resolves this work, no classified source is duplicated as
model open work, and summary closure never changes receipt state or installs global instructions.

**A4. Binding-kind transitions.**
- *Human to agent*: a hooked agent's lifecycle check-in replaces a human binding. Allowed.
- *Agent to agent, same seat*: a lifecycle check-in (startup, `/clear`, resume) replaces the binding. Allowed.
  A `managed_launch` binding is replaced this way, and only this way: a Current (tool-boundary) check-in never
  adopts or registers it, and C1 continuity never matches it. A lifecycle check-in prepared one seat
  generation before the launch was recorded (the launched agent checking in while `launch` reports it) is
  accepted against that binding (implemented, ht-5n6).
- *Agent to human*: the daemon refuses a human lifecycle check-in while the open binding is
  `cooperative_top_level` or `managed_launch`, unless the request is `--operator` (implemented). `me init` additionally refuses
  when agent evidence is present: one of the three allowlisted environment markers (`CLAUDECODE`,
  `CODEX_SANDBOX`, `CODEX_SANDBOX_NETWORK_DISABLED`), or Herdr reporting a registered adapter's recognized native host kind in the
  pane. Registry metadata alone never establishes native recognition, continuity or caller authority.
  Unregistered agent kinds are not evidence, and a failed Herdr read counts as no evidence. `--operator`
  overrides the refusal through a local per-execution mark set only after the daemon accepts the request
  (implemented, best effort, client-side).
- *Second agent*: `launch` refuses to start an agent for a seat whose bound agent (`cooperative_top_level`
  or `managed_launch`) Herdr reports live in another pane (implemented).
- *Wake*: a wake prompt goes only to a native host kind declared by the bound registered adapter
  (implemented). Recognized aliases do not change the binding or merge seats. A missing composer
  provider permits ordinary idle/done wake, but soft poke and stash/restore send no keys.
- *Poke*: a soft-deadline poke is the fixed reminder
  `herdr-threads: receipt due in <N>s on <thread-ids>; run herdr-threads inbox` (thread ids only),
  submitted through the wake dispatcher and its limits to the seat's bound native agent of the bound
  harness. It is decided from a fresh observation immediately before the prompt, only when the pane is not
  focused, the composer is empty, and the agent is idle/done. Working agents are deferred even
  where a historical recipe declares `poke_during_turn` from captured evidence
  (`docs/evidence/poke-spike/findings.md`). Existing exact-version recipe declarations remain
  historical captured qualification, not operational admission. Ordinary daemon observation resolves
  executables without probing runtime metadata; absent current-runtime qualification declares neither
  capability. Valid contract input, historical version evidence and manifest/release ladders cannot
  qualify the current runtime or grant compact recovery, composer stash, turn-time poke or native
  receipts. Until a separately safe current-runtime qualifier exists, richer behavior remains unavailable.
  It is never sent in an approval or
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
    unknown and skipped. The same reader classifies observations and final native delivery admission.
    Ordinary native wakes also require an empty composer, checked after their final
    identity/status recheck. A nonempty or unreadable composer refuses delivery before input is sent;
    pending attention and receipt obligations remain intact and the prior retry ladder is restored.
    Unfocused empty panes may wake immediately. Focused panes require a minute of observed empty
    composer samples, measured with the daemon's monotonic clock. Qualification is process-local,
    bound to seat/target/terminal/harness/server identity, bounded to 1024 windows, and restarts after
    a failed final admission, a successful send, clock reversal, or a sample gap over one minute.
    Intervening composer reads that fail or observe nonempty/unreadable input reset qualification.
    Soft-deadline-only pokes retain their stricter unconditional focused-pane skip; the one-minute
    focused rule applies to ordinary wakes. Status/focus checks and screen reads are an approximation,
    not keystroke telemetry.
  - *Drafts are deferred.* Attention never stashes, clears, retypes or submits a known draft, including
    soft-deadline pokes even if an old recipe declares composer stash. Existing adapter stash helpers
    are retained but the notification dispatcher never invokes them. A parked draft or unreadable
    screen can defer attention indefinitely. Claude suggestions may also defer delivery unless
    disabled through the existing user-approved setup option; this change never edits configuration.
  - *Verification is read-only.* A held marker yields an unsubmitted diagnostic/uncertain delivery;
    verification never sends an additional Enter, which could submit newly typed user text.
  - *During a turn.* Unsolicited attention never queues into a reported active turn. The dispatcher
    refuses ActiveTurn and both native prompt entry points require idle/done, regardless of an old
    turn-time capability declaration. Pending obligations wait for a later eligible observation.
    Composer emptiness never establishes idle status. These host status observations are sampled
    screen/lifecycle inference, not independent proof of physical turn completion.
  - *Post-send verification.* The read-only check never changes the composer or sends keys.
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
| ACK, accept, reject an ordinary invitation, send, leave, archive, reopen | the seat's current binding (top-level agent or declared human) |
| display ACK after text inbox output | the current top-level agent binding, for exact canonical pending agent receipts fully displayed on the page; the daemon decides eligibility again before settlement |
| check in | the pane's top-level agent (hook) or a human via `me init` |
| record a launch binding (`managed_launch`) | `launch`, after a host-correlated startup, on a seat with no open binding; it grants no row above |
| rebind, fresh seat, retire, replace, orphan-thread invite | operator |
| service-authored send, notify, and managed-thread controls (ensure, invite, topic, release, archive, reopen) | the registered service connection |
| summary, summary job, summary submit | the seat's binding or its children (summary workers), all under the seat's claim: read-mostly; submit only stores a validated block for a live lease issued to the seat |
| automatic quiet-channel archive | daemon lifecycle policy, after fresh complete eligibility and protected-work validation; never service-owned channels |
| anything else on behalf of another seat | nobody by design; possible by spoofing (Accepted limits) |

Automatic archival changes only the channel archive flag and appends a seatless lifecycle event. It
never retires or moves a seat, leaves a membership, settles a receipt or invitation, or releases a
requirement. Explicit reopen starts fresh grace. Human, unregistered, unresolved, held and uncertain
joined occupants block automatic cleanup. Pending compound handoffs remain protected indefinitely;
completed compound identities are absorbing tombstones. Exact historical completed retry may present
and clean up its retained local intent, even after archival or binding change, but cannot resume any
create/invite/send/launch side effect or revive protection. A legacy hint is reconciled only by exact
immutable compound identity; absence or removal of a local file never completes a canonical fence.

The operator never ACKs, accepts, rejects invitations, sends or advances a checkpoint.
The service never ACKs, accepts or rejects invitations and is never a receipt recipient.

An ordinary invitation rejection is the addressed seat's explicit, accountable decision about one
invitation episode, with a nonblank reason of at most 4096 UTF-8 bytes. It retains the actor seat, binding
generation, timestamp and existing `cooperative_top_level` or `operator_human` observation in an immutable
ledger, and publishes an attributed thread info event. It settles only that invitation's attention and
warning condition: never a message ACK or waiver, an acceptance, or a fabricated join/leave interval.
Declared subagents cannot reject. Required invitations remain service-owner controlled; a recipient must
ask that owner to release the requirement and reread its current state before attempting rejection.
Reinvitation creates a fresh episode; replay of an old rejection cannot reject that new invitation.

**Voluntary self-join.** An explicit `join THREAD` enrolls the caller's seat in an active
thread of its instance under the ordinary A2 deciding transaction and its existing
`cooperative_top_level` or `operator_human` claim. The immutable native-authored join
info event retains actor seat, binding generation, observation provenance and decision
time; the membership interval starts at that event's canonical decision sequence.
It creates no invitation, invitation acceptance, receipt ACK or binding. A pending
ordinary invitation requires explicit acceptance or rejection first; a pending required
invitation requires exact-revision `accept-required`. Join cannot accept, release or alter
a service requirement. An already joined seat is a settled no-op. Leaving and joining
again creates a fresh interval without changing historical recipient snapshots or
obligations. Replay returns its original result even after leaving, archival or a binding
change; it never recreates membership. Archived threads require a member or service
owner to reopen them before a new join. Discovery and names confer no additional authority.

**A5a. Human seats do not owe ACKs.** The daemon classifies a recipient from its canonical open
binding when staging a send. A human binding gets the message through thread membership but no
receipt expectation, even when the sender requested an ACK; an unbound or agent bound recipient
keeps the existing expectation. A human lifecycle check-in waives every pending obligation already
owed by that seat, including obligations inherited from an earlier agent binding. It records a
durable decision cutoff and a bounded reconciliation job in the same decision. The job records
each receipt table's high water independently and sets the separate `ack_required=0` obligation
marker on retained pending rows; later agent mail stays outside those bounds. The receipt stays
physically pending as historical evidence, but its effective state is `not_required`; it leaves
pending, wake and overdue scans. No ACK, ACK actor or ACK provenance is fabricated. A later agent
binding cannot revive the waived obligation; messages addressed to that agent afterward can require
ACKs. Already acknowledged receipts retain their exact historical status and provenance. A person
may still explicitly ACK an older addressed receipt; that action is attributed `operator_human`,
but the system never demands it. Invitation acceptance, reads and message history are separate
from receipt expectations.

For an existing store, migration derives the last human check-in cutoff from recorded human
binding generations and seat availability decision sequences. An open human binding can waive
through the current decision sequence; a closed one stops at its own check-in. A pending send
published within a recorded human binding's time interval is also waived, including a send made
while that binding was temporarily unregistered and its availability provenance is absent. The
end of a closed binding is exclusive: mail published after it ends, including mail for a newly
managed launched agent before that agent's first availability decision, remains owed. Legacy
manifest rows with recorded `operator_human` recipient availability are waived as well. The
one time reconciliation marks waived physical and sparse rows and removes pending projections;
old warning events, receipt timestamps, and ACK provenance remain historical evidence. A send
at the exact recorded end time is assigned to the successor for this migration, because the
millisecond clock cannot order simultaneous decisions within that boundary.

Recovery and human waiver may close many warning conditions. Their canonical decision records
one durable close sweep with a frozen condition boundary, recipient decision cutoff, and membership
high water; budgeted workers publish one clear transition per condition. A covered condition is
logically closed for active-warning queries before its clear event is published. The captured
snapshot, rather than worker time, controls clear recipients. A new open in the same thread waits
for its prior marked clear to publish, so a blocked send can return `StoreBusy` and require a fresh
operation after bounded preparation cleanup. A sweep cannot clear conditions opened after its
boundary or claim that an unprocessed clear event was delivered.

**A6. Effective receipt deadlines.** A receipt's effective deadline is the later of its frozen deadline and
the `extension_until` of the latest catch-up row for the receipt's (seat, thread). The daemon decides it (A2)
from catch-up state the seat itself entered through its accountable claim (a `summary` call that returned
work). The first entry, and any entry after a ready or superseded row, grants one p99 job duration;
re-entry after a stall grants nothing until a block is stored (the most recent row ended stalled and no
block for the thread was stored since), and a repeated call on an active row grants nothing. Otherwise it
is extended only by stored summary progress (each newly stored block, and the exit grace after a Ready),
never on heuristic evidence, so a stall and re-entry cycle cannot postpone a warning. Frozen deadlines are
never rewritten: they stay stored and displayed beside the effective one. Overdue classification, warnings, pending receipts and the soft-deadline poke use the
effective deadline.

**A7. Informational warning delivery.** Service notices and canonical built-in warning open/clear
events have separate delivery rows for their frozen recipients. A committed check-in settles only
the bounded prefix of attributed events it carries to the exact current binding generation and
execution. A global decision watermark cannot settle uncarried events, including late attribution.
History and immutable operation results remain readable. A successor occupant gets its own offers;
response loss can lose an informational offer after commitment, and an explicit operation retry
can present the retained result again. Delivery is neither invitation acceptance nor receipt ACK,
agreement, adoption or task completion. Logical publication tokens stay unchanged by projection;
an advertised read-only delivery hint lets hooks drain newly attributed and remaining notice pages.

**A8. Passive lazy delivery bookkeeping.** Recorded `ordinary`/`lazy` delivery mode is immutable and
independent of sender role, relay and human intent. A lazy audience is frozen by canonical preparation
and publication; recipient identity is immutable, and its pending/displayed progress is monotonic.
Already addressed rows survive leaving, retirement and archival, with no transfer to another seat.
Unpublished rows are invisible and may be discarded in bounded cleanup; published progress is retained.
Lazy delivery creates no receipt, deadline, ACK evidence, attention, wake, poke or automatic adoption.

Only the caller's default text inbox may claim completion after complete contiguous body output has
been written and flushed. The completion handler validates the current top-level or human canonical
caller (A2) and exact published addressed message IDs in its deciding transaction; declared subagents
cannot complete delivery. JSON, machine, explicit-seat reads, history, bodies and summaries remain
read-only. Local display journals are cooperative hints, never authority. Completion is idempotent
presentation bookkeeping: it records no ACK actor, receipt provenance, timeline ACK, agreement,
instruction adoption, task completion or proof of model consumption. Partial output and changed
bindings cannot manufacture complete display. Frozen completion retries preserve their original full
caller claim, harness and intent scope through submission and local cleanup. These same-user cooperative
limits are deliberate; no adversarial execution verification is added.

## Accepted limits

These are decisions, not bugs. Each is safe to rely on only as stated.

- **Bounded advisory evidence suppression.** Negotiated v2 recording remembers resumed creating-CLI
  lifecycle suppression with a fixed 8-KiB monotonic hash filter, scoped by harness, session, domain,
  origin and contract. Hold expiry/eviction cannot forget a recorded suppression during that recorder's
  lifetime. Hash collisions or saturation may withhold lifecycle milestones from unrelated new sessions;
  they never grant evidence verification, seat continuity or receipt authority. The filter is in memory
  and starts empty on daemon restart; suppression does not claim durable session history.

- **Global hooks enroll supported top-level Herdr sessions.** A user-level Claude/Codex hook
  enrolls every supported top-level lifecycle session in its Herdr instance, including sessions
  started outside `launch`, subject to the ordinary canonical guards above. Outside Herdr it
  remains silent. No wrapper or global configuration change is made by enrollment. The actual
  pane environment must come from the user's foreground harness configuration; enrollment
  cannot infer a different attaching TUI pane or turn a shared-server environment into proof.
- **Hermes enrolls only through `launch`.** Startup enrollment stays Claude/Codex only. A Hermes
  session started outside `launch --kind hermes` gets no seat and no context: the bridge's hooks
  find no resolved seat for the pane, and Herdr does not recognize the bare process as an agent
  without the guarded launch process hint. Delegated Hermes children are answered locally with the
  subagent restriction and never reach the daemon.
- **Foreground harness execution is user managed.** Hooks and tools must inherit the TUI
  pane's `HERDR_PANE_ID`. A shared harness server started in another pane can instead supply
  its own pane environment; herdr-threads does not recover the attaching TUI pane from
  process names, cached sessions or focus. Setup and setup-status inspect user settings and
  explain the required foreground configuration without changing it. A configured setting
  is advisory, not execution proof: wrappers and higher-precedence settings remain the
  user's responsibility. Claude's `disableAgentView: true` disables its background agent
  supervisor. For captured Codex 0.160.1, `daemon_auto_start = false` still permits attachment
  to an existing server; native `--no-daemon` is the supported opt-out. Optional
  `HERDR_THREADS_CODEX_OPTS` and `HERDR_THREADS_CLAUDE_OPTS` add user-selected launch
  arguments, with no automatic daemon argument by default. No argument or settings check
  grants receipt authority, moves a seat or relaxes the canonical binding checks in A2.
- **Child agents can ACK.** Subagents are instructed not to; the plugin cannot tell them from their parent.
- **Same-user spoofing.** Any same-user process can select another seat, set `HERDR_PANE_ID`, or write another
  seat's files under the instance `contexts/` and `intents/` directories (writable from the Codex sandbox by
  design). Forged client files cannot satisfy A2's binding match, but they can disrupt that seat's local
  state. Sandbox-writable files are opened without following symlinks (implemented for `allocator.lock`).
- **Unseen exits.** An agent that exits to its shell stays bound until the next check-in or retirement (C5).
- **Launch is not check-in.** A successful `launch` means Herdr started the agent, not that it registered.
  Its only record is a wake-eligible occupant claim (`managed_launch`, A3) on a seat with no open binding: not
  a registration, availability, receipt or authority for any accountable action, and replaced by the agent's
  first lifecycle check-in. Launch forms without captured hook evidence are refused (implemented for
  `codex resume`).
- **Process hints are cooperative recognition input.** A registered launch policy may require Herdr's
  narrow `process_hint` mode, advertised by each compatible start negotiation and fenced to the actual
  local server peer before submission. The request and startup correlation record the requested mode,
  not an effective child environment or proof of native execution. Same-user processes can inherit or
  imitate recognition hints; platform support, environment readability and descendant coverage can
  prevent recognition. Hints add no registration, receipt, continuity or caller authority and no new
  provenance (A1–A3, C1); arbitrary environment overrides remain refused. A possibly submitted start
  with an uncertain response remains unknown, without a safe fallback or inferred binding.
- **A launch binding can outlive its agent.** Like any binding (Unseen exits), a `managed_launch` binding
  whose agent exits before checking in stays until a check-in replaces it or the seat is unresolved or
  retired; a later `launch` into the seat then records nothing (the seat already has an open binding) and a
  wake for it is refused when Herdr's agent kind differs from the recorded harness.
- **Restore costs operator time.** A genuinely new role in a restored pane waits for an explicit choice when
  cooperative continuity does not apply.
- **Any invocation of a seat can enter catch-up.** CLI calls cannot tell a seat's top-level agent from its
  child workers, so any `summary` call by the seat may open its catch-up row. Harmless: workers exist only
  after the parent's own `summary` entered the row, and an active row keeps its frontier, so repeated or
  worker calls never move it.
- **Summary blocks name the seat, not the invocation.** A block records its author seat and declared model;
  it does not record whether a child wrote it, because the CLI cannot know.
- **Optional runtime qualification may be absent.** Core contracts can operate cooperatively without
  runtime metadata. Unknown metadata inherits no optional compact, composer, turn-time poke or native
  receipt capability. Historical captured declarations (Claude 2.1.287's richer behavior; every Codex
  version declares neither rich poke capability) remain evidence for their exact captures, never a grant
  to an unidentified current runtime. Working agents and typed drafts always defer native attention,
  including with historical turn-time qualification. The hard-deadline warning remains
  durable pending attention, subject to the same native composer admission when waking.
- **Unavailable-runtime diagnostics are bounded advisory history.** Failures use only the existing
  harness/session/contract producer scope, never an invented version/build key or account authority.
  The first event/field stays sticky across later successful inputs. Storage keeps at most 256 rows per
  harness with deterministic eviction, a 30-day retention projection and at most 20 rows per read;
  Health shows a failure only in its 24-hour window. Retention expiry or bounded eviction can retire a
  failure; success cannot clear it. Empty or missing session identity records a bounded parse diagnostic
  without inventing a session. Codex rollout creator metadata never identifies the current runtime,
  including startup and known resume. Exact attributed historical evidence remains unchanged.
- **Pokes skip Claude panes that show composer text.** A Claude pane whose composer shows a prompt
  suggestion (or any typed text) is not poked: the suggestion cannot be told from a draft without captured
  styling. The skip is for that poke only and is retried after the wake retry spacing; only the
  hard-deadline warning reaches a pane that keeps showing a suggestion (ht-1ip.46). Claude shows one after
  every turn by default, so this skips most idle Claude panes unless suggestions are off: with
  `promptSuggestionEnabled: false` an idle Claude composer reads empty (a bare prompt or a captured
  placeholder) and the poke is sent. `setup claude` explains this and offers to turn suggestions off; it
  writes the setting only on an interactive yes or `--disable-prompt-suggestions`, never silently, and
  `unsetup` reverts what it set (ht-6jt). A project or managed setting, or the session's
  `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION`, can turn them back on; the poke then skips as above.
- **Native attention uses sampled composer/focus guards.** Check-to-send focus or input races are
  accepted: a user may type or change focus after the final read. Empty samples do not establish an
  absence of keystrokes, cursor moves, or type/delete activity between samples. Focus is Herdr's pane
  selection, not every attached client's physical attention. Restart/outage gaps can delay focused
  wakes; nonempty/unreadable composers can defer indefinitely. No atomic host/client buffer claim is made.
- **Composer reads see the screen, not the buffer.** Herdr's detection read shows composer rows without
  trailing whitespace, so whitespace typed at the end of a draft row is invisible, and a draft of only
  spaces can read as empty and be nudged over. A draft typed to match a captured
  Claude placeholder exactly reads as empty the same way. A Claude placeholder that was never captured reads
  as unknown, so native attention remains deferred until the placeholder is captured and listed or
  the composer becomes recognizably empty; hard-deadline warnings remain pending.
- **Conservative archival liveness.** Joined-agent archival qualifies against the exact existing cooperative binding and fresh structure plus empty composer evidence from its registered parser. The reported host kind selects that parser and must agree with the bound harness; it is not occupancy or current-execution attestation. Unseen exits and same-user imitated recognition remain the cooperative model's accepted limits. A shell/unrecognized kind, absent provider, unreadable composer or changed canonical binding cannot qualify. Archival does not establish liveness, availability, execution or receipt authority. Repeated composer-aware idle observations use a 60-second cadence
  with a maximum 120-second gap and fresh evidence window. Boot changes, outages, clock discontinuities
  and missed observations restart qualification. Partial/malformed/inaccessible legacy journal coverage
  vetoes cleanup. Every published noncompound intent, even a valid Send or ACK, also vetoes
  coverage until recovered or absent in a fresh scan. This creates no canonical hint and the importer
  never deletes intents. Fully verified compounds alone may create veto hints, with completed canonical
  identity taking precedence transactionally. Published records follow the journal's immutable/rename
  contract; source generation tracks directory publication/removal/replacement. Bounded validation uses a global canonical mutation fence, so continuous unrelated
  activity or a channel too large to refresh within that window can keep it open indefinitely. Structural
  snapshots alone never prove idle; the existing composer visibility limits apply to archival too.
- **Probabilistic identifiers.** Pagination cursor binding tags are 48 bits (about 2^-48 acceptance per forged
  or stale cursor; scope, direction and order are still compared exactly). Send-preparation ids are drawn from
  62^8 ≈ 2^47.6; reuse of a retired id has probability about (retired ids) × 2^-47.6 and is harmless once
  nothing references it.

## Decision record

### Wrapper compatibility and foreground execution (2026-10-06)

A user's `codex` alias resolves to a wrapper that rejects native `--no-daemon`. Removing
automatic injection preserves that wrapper's argument contract. The isolated native Codex
0.160.1 probe shows that model-selected Bash execution and lifecycle hooks both inherit the
app-server environment; overriding tool environment alone does not fix hooks. User decision:
rely on user-managed foreground settings, inspect and explain them during installation, and
offer optional per-harness launch arguments through environment variables. No additional
wrapper or heuristic TUI-to-seat association is introduced. Setup never writes these
foreground settings. The retained measurements and source limits are in
`docs/compatibility/wrapper-seat-detection.md`.

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
claims (`author_role`, `relays_user`), not from the text. Catch-up extends effective deadlines once on the
first entry or an entry after a ready or superseded row, and otherwise only on stored progress (A6; re-entry
after a stall grants nothing until a block is stored, ht-hqg). The A4 poke rule (soft-deadline pokes) is defined with its behaviour above.

### Managed launch binding (2026-10-03, ht-5n6)

Codex 0.159.3's TUI runs SessionStart hooks only at the first turn, so an agent launched without an initial
prompt never checked in, its seat had no open binding, and the lost-prompt idle-recovery wake (which requires
one, A4 *Wake*) never fired. Decision (user, 2026-10-03): `launch` reports its host-correlated startup and the
daemon opens an unregistered `managed_launch` binding (A3) on a seat with no open binding; it carries the
harness the wake recheck compares and nothing a caller can claim. Rejected: waking a resolved but unbound
seat whose pane Herdr reports as an idle agent (Herdr's agent field may only suggest, C1/C5); documenting
lost-prompt recovery as unsupported on Codex 0.159.3 and later.

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
| ht-5n6 Codex launched without a prompt never checks in, so no lost-prompt wake | `managed_launch` binding after a correlated launch | A3, A4, A5, Accepted limits |
| W6-R2 "mark it self" wording | "when run in this pane" | A1 |
| Waves 29/19 identifier sizes | accepted; fix the 104-bit comment (it is 112) | Accepted limits |
