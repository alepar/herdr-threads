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
| Operator decision | `ht human seat rebind`, `ht human seat resolve --new-seat`, `ht human seat retire`, `ht human seat rebind --replace` (each with `--operator`) | `operator:local-user:<uid>` |

Seats are never merged. Pane labels, saved addresses, terminal-id hints and Herdr's agent field may *suggest*
candidates in diagnostics; they never move a seat, allocate one, or end a binding.

**C2. Restore holds.** After a new or unknown Herdr incarnation while nonretired seats exist, every saved
seat whose continuity is not structurally proven becomes unresolved, and the daemon takes a coherent baseline
of current targets. Every unowned target in that baseline is **held**: ordinary resolution (`seat resolve`,
`launch`, top-level `SessionStart` enrollment, `ht human me init`) refuses it with inspect / rebind / fresh-seat guidance. Panes authoritatively created
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
- abandon the old seat: `ht human seat retire OLD --operator` (implemented);
- abandon the new role: `ht human seat rebind OLD --pane P --replace NEW --operator` (implemented), which retires NEW
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

**Invocation spelling.** Ordinary root commands act as agents. Person and local-operator
commands require immediate argv[1] `human`: `ht human me init [--operator]`,
`ht human send THREAD --body TEXT`, and `ht human seat rebind ... --operator`.
Put `--state-dir`, `--host-endpoint` and output flags after `human`. `--human` changes
output formatting only; relay, user-intent and delivery flags do not declare a person.
Root accountable actions refuse inferred Human contexts. The namespace records no new
provenance and confers no canonical authority. Subagents read with `--machine` or `--json`
and never mutate, accept or ACK.

Retry classification reads the original validated frozen semantic and scope before
state creation, connection, completed presentation or cleanup. Retained person/operator
intents use `ht human retry REF`. A historical Agent intent remains Agent after a binding
change. Presentation may clone known command argv fields, but never changes the original
claim, operation key, digest, response bytes, cursor or rows, or rewrites peer text.

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
| `cooperative_mod_channel` | mod channel registrations only (process-local) | The pane's top-level Claude session, through the bundled herdr-threads mod's `watch` child, opened a delivery channel for the seat's current binding generation. The daemon decides it against A2 (seat resolved and not held, open `cooperative_top_level` binding with harness `claude`, native session equal to the claim's, no stall cooldown, `mod_delivery` on). It grants nothing but receiving deliveries and making `cooperative_mod_delivery` claims for that binding generation. Never on bindings or receipts. |
| `cooperative_mod_delivery` | receipt action observation; lazy completion claim | The mod reported that a delivery path's predicate held (spec D6): for an ordinary message, that its full body entered the model's context through the mod (an answered tool result carrying the mod's context, or a `$.prompt.submit` that resolved without `drop`; or, after a plugin reload disposed the instance whose submit was still in flight, a main turn whose prompt carries the mod's frame naming the id, or the completion of the turn that was open when the successor loaded (the engine does not report the disposed instance's submit outcome)); for a lazy row, that it was appended to the transcript (`$.session.append` resolved without `deny`). A cooperative delivery claim, not proof that the model read it. The receipt keeps the binding's `cooperative_top_level` provenance separately. A truncated item never carries it. |
| `operator_human` | bindings, receipts | A person declared this pane human with `ht human me init` and acted from it. Best effort: refused where the system sees evidence of an agent (A4). |
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
  `cooperative_top_level` or `managed_launch`, unless the request is `--operator` (implemented). `ht human me init` additionally refuses
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
  provider defers ordinary idle/done wake; soft poke and stash/restore also send no keys.
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
- *Mod channel*: while a mod channel is live for a seat (from registration until its stream ends, through a
  30 s reconnect grace after a drop, and through a 30 s seat-level rebind grace after `Close{binding_changed}`),
  no native wake prompt or poke goes to that seat and the Claude check-in results omit the attention digest
  and ready commands; pending attention stays pending and deadlines and hard-deadline warnings keep running,
  reaching the agent through the mod. A grace that expires, or a `Close` for `retired`, `unresolved`,
  `stalled`, `disabled` or `stopping`, removes the channel and kicks the wake lane. *Stall handover*: a
  channel is stalled when its last mod ACK (or its registration) is more than 10 minutes old and an ordinary,
  non-truncated pending receipt for the seat, published at or before the last pushed attention frame, is
  itself more than 10 minutes old; the daemon closes it (`stalled`) and refuses re-registration of that
  binding generation for 10 minutes, so the native ladder with its composer guards is the only delivery path
  during the cooldown (contract; implemented by ht-j16.2 and ht-j16.4).
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
    identity/status recheck. A missing registered composer provider, nonempty composer or unreadable composer refuses delivery before input is sent;
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
| open a mod delivery channel (`watch`) | the pane's top-level Claude session through its mod, for the current `cooperative_top_level` binding with the matching native session; recorded as `cooperative_mod_channel`; it grants no other row |
| mod delivery ACK (`watch ack`) | the current top-level Claude binding with a live channel (grace included) for its binding generation, or, on resume, for ids delivered under the immediately previous generation of the same native session; the daemon decides each id again before settlement and refuses unknown, unaddressed and truncated ids |
| check in | the pane's top-level agent (hook) or a human via `ht human me init` |
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
Delivery is not a wake. A built-in transition wakes only the affected seat of a still-open
condition (the hard-deadline backstop for the obligation that seat owes); a clear, and every
transition about another seat, waits for the member's next check-in offer. Accepted limit: an idle
member learns of another seat's overdue or cleared obligation only when its pane next checks in. When
more than the bounded window of pending warnings or unoffered notices is visible, narrowing cannot rule
out an older waking notice (such as a service notice), so the wake keeps its unnarrowed answer and the
offer probe stays conservative: it may still wake, never silently drop one.
The same rule decides a live Claude mod channel's attention: the daemon counts only waking warnings in
the channel's attention fingerprint (with the same saturated fallback) and marks each other pending
warning `informational` in the inbox batch, so `watch` raises no `attention` line for it.

**A8. Passive lazy delivery bookkeeping.** Recorded `ordinary`/`lazy` delivery mode is immutable and
independent of sender role, relay and human intent. A lazy audience is frozen by canonical preparation
and publication; recipient identity is immutable, and its pending/displayed progress is monotonic.
Already addressed rows survive leaving, retirement and archival, with no transfer to another seat.
Unpublished rows are invisible and may be discarded in bounded cleanup; published progress is retained.
Lazy delivery creates no receipt, deadline, ACK evidence, attention, wake, poke or automatic adoption.

Only the caller's default text inbox, or the bundled Claude mod as below, may claim completion after complete contiguous body output has
been written and flushed. The completion handler validates the current top-level or human canonical
caller (A2) and exact published addressed message IDs in its deciding transaction; declared subagents
cannot complete delivery. JSON, machine, explicit-seat reads, history, bodies and summaries remain
read-only. Local display journals are cooperative hints, never authority. Completion is idempotent
presentation bookkeeping: it records no ACK actor, receipt provenance, timeline ACK, agreement,
instruction adoption, task completion or proof of model consumption. Partial output and changed
bindings cannot manufacture complete display. Frozen completion retries preserve their original full
caller claim, harness and intent scope through submission and local cleanup. These same-user cooperative
limits are deliberate; no adversarial execution verification is added.

The bundled Claude mod is a second completion source: it completes lazy rows it appended to the transcript
(`watch ack --via append`), decided by the daemon against A2 under the same checks as the mod delivery ACK
(A5) and recorded with the `cooperative_mod_delivery` claim "appended to the transcript". Like inbox
completion it is presentation bookkeeping (no ACK actor, adoption or proof of consumption).

## Accepted limits

Delivery-only cleanup recovery retains a private, bounded versioned terminal record after
intent/progress removal. It preserves exact original immutable intent bytes and the genuine
completed fence result and staged-work report; it records no new provenance. This local
record never authorizes live work or proves canonical completion: every replay must classify
the original actor, compare the canonical namespace and obtain the exact canonical Completed
fence before presentation or cleanup. It permits honest replay after output, unlink or directory
sync failure, at the cost of retained local disk records. It is not live archival protection;
malformed or contradictory records conservatively refuse, and canonical terminal state dominates
stale archival scans. Public new modes require the guarded runtime capability. Delivery
carries the exact frozen Agent original and namespace on restricted phase and read
requests; the daemon compares its independently selected path bytes before preparation,
effects, replay and historical presentation. Read-only preparation checks the canonical
caller, channel and recipient before local publication. Each live effect checks current
recipient eligibility in its deciding transaction. A resolved unbound recipient is allowed;
this neither starts an agent nor accepts an invitation or ACKs a receipt. Completed
presentation checks the exact canonical fence and retained report without live mapping
requirements. Generic runtimes without the required producer/deciding adapter do not
advertise this capability and cannot execute these routes.
Linked bootstrap Invite and Send phases derive their required namespace and exact
payload from canonical child-key registration, full frozen parent and attachment.
Missing selected context refuses even cached history; unregistered legacy phases
retain their existing behavior. Completed linked history skips only live guards.

A retained successful version-1 bootstrap launch report (the only plan version) is
validated against frozen V1 composition: the original generated prompt bytes and the
original Codex/Claude acceptance grammar with empty owned argv, never today's renderer
or registry. Deciding completion, completed preview, terminal decode and presentation
share it. Validation does not repeat the producer's filesystem routing lookup: it
accepts only the verbatim canonical-namespace routing object (valid instance UUID) or
the null-routing form, with identical fallback and suffix; any other routing refuses.
So if a symlink component appears in a not-yet-existing namespace suffix, or the host
endpoint parent changes, between preparation and launch, the launch-time canonical
object differs and that genuine report refuses, without relaunch or data loss.
Fresh Root preparation and launch still apply today's native admission and geometry,
and refuse before effects when today's composer would differ from frozen V1, so a
future composer change stops new Root launches rather than producing reports that
completion cannot validate.

Canonical topology storage has encoded-byte ceilings for retained bootstrap envelopes:
128 KiB for the full frozen identity, creation evidence and attachment; 256 KiB for
an immutable recovery decision; and 2 MiB for the full completed result, including
identity, attachment, launcher report and legacy result. The launch report's inner
JSON retains its independent 1 MiB protocol bound. JSON escaping and repeated nested
fields count toward the storage ceiling; a protocol-shaped envelope need not fit.
Oversized writes must refuse before effects and preserve existing records; bounded
reads of oversized or corrupt data must refuse, never present clipped data or grant
submission permission. Records are never truncated to fit. These limits neither
change attribution nor authorize from a client-local journal or today's occupant.
Public preparation also checks the actual serialized request-bearing local progress
against its existing 128 KiB cap using the observed endpoint witness, before publication,
Begin or reservation. A canonical maximum-size identity need not fit that larger producer
envelope. Future native response metadata is not statically known; an unpersistable created
response conservatively retains possible creation and refuses automatic resubmission.
Transition writers retain the canonical guards and enforce the same limits.

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
- **Native permission rules are cooperative matching limits.** Owned Claude rules allow
  the bare `herdr-threads` / `ht` command text and ask for `human`, `setup`, `unsetup`,
  `doctor fix` and `internal installer-integrations`; owned Codex rules use a literal
  executable-prefix allow with `prompt` prefixes for the same words. The CLI refuses an
  agent that does not write those words first, but quoting or rearranging words to dodge
  a text rule is outside the model. Stronger deny/ask/managed policy remains effective.
  Foreign broad allowances are preserved and can still cover Human commands. Bare names
  cannot attest later PATH, alias or function resolution. Generated matching and native
  checker evidence never prove a live classifier verdict or installed wrapper support.
  Setup changes no general shell, network, sandbox or approval-mode setting.
- **Unattributed public reads retain ordinary daemon spelling.** Public read requests
  carry no invocation actor; a Human subject seat or peer label cannot identify the
  requesting viewer. Known current caller/action producers include Human spelling before
  page fitting. For an explicit Human invocation of a generic public read, the client
  renders a clone of known argv fields and measures its final selected encoding against
  the original byte budget. Namespace overhead that cannot fit reports the exact required
  minimum, including framing and newline, without changing cursor or rows or refitting a
  historical response. Frozen check-in claims determine their own historical ready spelling.
- **Owned configuration locks coordinate participating writers only.** A stable advisory
  lock serializes owned writers, which refresh and revalidate under the guard before each
  replacement. A noncooperating editor changing a file after the final check and before
  rename can escape detection; the guard is not atomic compare-and-swap. Historical
  ownership transfer recovery covers process termination on a functioning filesystem,
  not every power loss or storage failure.
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
- **Mod delivery claims are cooperative.** A same-user process could run `herdr-threads watch ack` and settle
  receipts as `cooperative_mod_delivery`; the daemon checks the canonical binding, its generation and a live
  channel, not the process.
- **Engine-reported idleness is trusted.** The mod takes busy and idle from Claude Code's turn events; the
  post-abort hold bounds the known Esc takeover.
- **A remote mods kill switch returns seats to native wake.** A mod that never loads looks like no channel.
- **A reload during an in-flight submit is settled by inference.** The successor treats the turn open at its load
  as the submitted turn. If that submit produced no turn and the successor sees no turn start for 120 s with an
  empty prompt box, it delivers the ids again; a submitted turn that both started and ended unseen inside the
  reload gap can be delivered twice.
- **An unrecovered handoff is at-least-once.** An item the mod delivered whose ACK did not land before the
  channel dropped stays pending; the mod re-ACKs it after re-registering (same generation, or on resume the
  immediately previous generation of the same native session), and otherwise the agent may see it again
  through `inbox` or the native wake. Never lost. `/branch` keeps the transcript under a new session id, so
  items delivered but not ACKed before a branch may be delivered again.
- **An outer mod could strip context.** A mod wrapping `tool.call` outside herdr-threads' hook could remove
  the attached context after the herdr-threads hook returned; the receipt is already settled.
- **Re-registration resets the stall clock.** Each registration starts a fresh channel whose stall clock
  begins at registration, so a mod whose `watch` keeps reconnecting without ever delivering can hold native
  wake off for as long as it keeps reconnecting. Deadlines and hard-deadline warnings still run; disabling
  the mod (`mod_delivery: off`, or `unsetup`) returns the seat to native wake.
- **A daemon restart can deliver twice.** After a restart the wake lane's first pass can run before the mod
  re-registers, so one item may reach the agent both through native wake and through the mod. At-least-once,
  as above; never lost.
- **Mod frames keep peer text off the header column.** The mod indents every body line by two spaces, as the
  compact `body` read does, and folds thread, sender and id onto the block's header line, so a line starting at
  column 0 (the fixed preamble and each `[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:`
  header) can only come from the mod. Within a header line the thread and sender names are peer-chosen text
  and may contain words that look like markers; the markers the mod writes come after the sender and attribute
  the source without granting permission.

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

### Mod delivery is the receipt (2026-10-09, ht-j16)

The 2026-10 direction forbids blanket read or delivery auto-ACK. In this run the user approved an exception for
the bundled Claude mod: "we already judged delivery is the receipt". Precedent: `cooperative_inbox_display`. The
claim is "the full body entered the model's context through the mod", not proof of reading; the mod never
acks a truncated item; its receipt settles only through the agent's explicit `ack` (the truncation marker names
`body`, which is read-only, then `ack`) or a text `inbox` that displays it in full. Rejected: reusing `cooperative_inbox_display` (a different
action); an explicit agent ACK after mod delivery (keeps the tool-call overhead the goal removes); delivering
only the marker.

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
| Wave 18 `ht human me init` as `operator_human` | env-marker and Herdr-agent refusal | A3, A4 |
| Wave 18 `ht human me init` replaces agent binding | daemon refuses agent-to-human without `--operator` | A4 |
| Wave 30 shared client directories | accepted, documented | A1, Accepted limits |
| `allocator.lock` without `O_NOFOLLOW` | open without following symlinks | Accepted limits |
| Wave 28 second agent via name retry | launch refuses while bound agent is live | A4 |
| W9-2 wake ignores harness | compare bound harness with pane agent | A4 |
| ht-5n6 Codex launched without a prompt never checks in, so no lost-prompt wake | `managed_launch` binding after a correlated launch | A3, A4, A5, Accepted limits |
| W6-R2 "mark it self" wording | "when run in this pane" | A1 |
| Waves 29/19 identifier sizes | accepted; fix the 104-bit comment (it is 112) | Accepted limits |

## Bootstrap creation attempts and administrative recovery (2026-10-08)

A bootstrap freezes the original agent claim, scope, namespace and payload. Its
canonical first attempt exists at Begin. Only the transaction first reserving a
prepared attempt issues submission authorization; status, restart and lost-reply
replay never issue another authorization. A fresh pre-submit decision checks the
original A2 claim, exact attempt and reservation administrative revision. A
transport-proven `NotSubmitted` closes that attempt and allocates a distinct next
attempt. A recorded unknown cannot later be reclassified as transport zero submission.
Unknown creation remains fenced; delayed older creation cannot replace
an inspected noncreation decision or a later attempt.

The public administrative grammar is immediate `human handoff recover REF --attempt N`
with exactly one `--created-pane EXACT_PANE`, `--not-created`, or `--cancel --reason TEXT`.
Pinned routing and output flags follow immediate `human`; a root `--operator` form
cannot substitute for this namespace. Recovery accepts bootstrap references only.
The separate local recovery intent has operator UID scope and retains the original
bootstrap reference, agent identity and exact inspected attempt as immutable data.
Its nested original caller claim is a reference, never the operator decision's caller.
The same effective UID must retry it. Original actor classification runs before
completed presentation or cleanup; root agent retry refuses an operator decision.
Public recovery execution remains inert until the production canonical guards are integrated.

Administrative recovery is recorded separately as `operator:local-user:<uid>`,
using the authenticated local-account peer UID. The decision freezes an explicit
attempt number and the complete assertion payload. Exact replay presents that
historical decision even after later attempts; stale undecided attempts and
contradictory assertions refuse. Each attempt retains at most one creation or
noncreation recovery decision and one subsequent cancellation decision. Cancelling
a confirmed creation retains its earlier decision for historical presentation.
Recovery never rewrites original actor claims,
produces receipts, proves occupancy or reports successful launch. Created-pane
assertions need fresh coherent canonical structural evidence and the ordinary
restore/hold/ownership guards; they do not move or allocate a seat. The retained
`structural_reference` is historical operator-assertion correlation, not fresh
admission or authority. A deciding daemon read has its own unchanged call ID and
must supply full workspace/tab/pane/terminal/process incarnation and genuine
endpoint witness from that same response, published through the canonical lane.
Fresh creation recovery compares the complete witness to the assertion and checks
the admission's lifecycle revision plus exactly one publication for both owned
and unowned targets. Exact saved decisions replay before any fresh host read.
Full witness equality is deliberately conservative: socket metadata changes or
an auxiliary socket for the same server may prevent fresh recovery even when
process identity remains unchanged. Metadata does not prove creation, occupancy
or caller authority, and refusal never repairs or rewrites a frozen witness. The normal
operation lock continuously excludes a known in-flight invocation from fresh
canonical inspection through immutable assertion publication and deciding dispatch.
The frozen recovery request retains a versioned digest of the actual canonical
attempt state, creation, attachment and latest recovery sampled under that lock.
Saved retry never refreshes this inspection. After exact committed historical replay,
the deciding transaction requires the retained binding to match its canonical view;
missing old bindings and changed undecided snapshots conservatively refuse.
New-client fresh recovery and every operator retry require the concrete inspected-
recovery capability before publication or deciding dispatch, even for old unbound
plans. Exact old committed requests keep their absent-field bytes, keys and results.
Older clients may fail closed on new guarded recovery results, including results
nested in bootstrap status; an inspection-aware client is required to present these
new records. Guards are never stripped to fabricate older-client compatibility. No heuristic PID or topology
snapshot proves quiescence, and recovery never kills an unowned process.

Herdr offers correlation rather than creation idempotency. The product therefore
sacrifices automatic progress after an uncertain submission. An operator's
mistaken inspected noncreation or quiescence assertion can permit duplicate
topology. That assertion is retained honestly as an administrative claim, never
as transport proof of zero submission. Created topology is never automatically
closed by product cleanup.

Bootstrap cancellation is an absorbing administrative tombstone, not successful
completion. A nonblank reason of at most 4096 UTF-8 bytes and inspected quiescence
are retained with the original identity, attempt, evidence and exact attachment.
Its deciding transaction must prove that no exact downstream legacy child fence
or live local-journal hint exists. Cancellation retains messages, invitations,
membership, receipts and topology; it releases only the bootstrap's protection.
An already completed bootstrap cannot be cancelled or reactivated. A live legacy
child remains protected indefinitely, including after pane loss or seat
retirement; bootstrap cancellation cannot release that child fence. Broader
legacy-child cancellation is outside this feature.

The additive `RecordBootstrapNotSubmitted` boundary reports only the actual
typed transport's zero-byte branch. It uses its own frozen deterministic attempt
key, distinct from creation evidence; the daemon rechecks the original A2 claim
and canonical namespace before closing that attempt. This is a cooperative
original-caller transport-outcome claim under the frozen `cooperative_top_level`
attribution, not independent daemon attestation of a host write, an operator
inspection or receipt provenance. An incorrect producer claim can permit
duplicate topology. Error classes, response loss and unknown submission never
stand in for this claim. Public dispatch remains inert until the actual typed
transport producer is integrated with all canonical guards.
