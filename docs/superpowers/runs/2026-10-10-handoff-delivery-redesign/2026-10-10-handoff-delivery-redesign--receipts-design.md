# Handoff delivery redesign: receipts

## Goal

Every pending ACK-required message has a send-anchored original deadline and is scanned for overdue warning without requiring recipient availability.

Parent: [root design](2026-10-10-handoff-delivery-redesign-design.md); bead ht-0lo.1.

## Decisions and boundaries

The parent spec is the normative behavioral contract for this subtree. It specifies the selected alternatives, compatibility rules and edge cases; this spec scopes ownership and execution, without adding a second interpretation. Every pending ACK-required message has a send-anchored original deadline and is scanned for overdue warning without requiring recipient availability.

## Tasks

### Canonical send-time receipt projection

owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.

### Due discovery and originating-seat warning lifecycle

owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.

## Validation and failure handling

Each task runs its listed configurations before terminal integration with isolated config/daemon/host fixtures and owned test-support children. Preserve immutable keyed historical requests and terminal outcomes; new incompatible fields negotiate before durable effects. All implementation follows AGENTS.md lint/format/test and TRUST-POLICY cooperative canonical authority. No real user config edits, automatic trust or shared server management.

## Promotion refinement

Historical upgrade is a separate leaf ht-0lo.1.3 consuming ht-0lo.1.1. Due discovery ht-0lo.1.2 consumes both the projection and upgrade so its historical discovery scenarios are satisfiable. Handoff ht-0lo.3.2 still depends on ht-0lo.1.1 unchanged: it consumes only new-send semantics, not historical migration.

## Coverage refinements


Originator open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear passive; update TRUST-POLICY A7. No invented seat for service authors. Test notification pending while drafts/working/host outage defer actual native wake.

## Confirmed roast refinements

Healthy-daemon discovery retains the 5,000ms safety tick and 100 actually inspected candidates per successful global due-driver quantum. Every two successful runnable global quanta give each unfinished receipt/invitation class one turn with a 100-candidate allowance; successful `more` continues immediately. All queries enforce finite captured high-water bounds before LIMIT, advance stable cursors, and count inspected rows even if future, terminal, waived or otherwise ineffective. Ordinary receipt scanning lends unused physical/sparse allowance so a non-final snapshot turn can consume its full allowance; native wake admission/completion never gates cursor progress. The 100 limit is a total row-inspection allowance, not 100 repaired rows plus 100 due rows.

During unfinished historical receipt repair, reserve at most ceil(limit/2) candidates for repair and the remaining allowance for ordinary due discovery (at least 50 at limit=100); lend unused repair allowance to due discovery. Advance persistent finite physical/prepared ordinal repair cursors without restarting ordinary due traversal every batch. Inspect each captured repair candidate at most once; published projection alone is repaired, while any physically visited unpublished prepared candidate still counts. Repair completion emits a one-shot `completed` transition and resets the ordinary receipt cursor before due selection in that same committed transaction. Repair progress, that completion/reset decision and due-warning effects commit or roll back together. This fresh post-repair traversal catches repaired deadlines behind a previously advanced due cursor. Empty repair finalization uses zero row allowance and shares the ordinary turn at full capacity. Failed decisions cannot consume the completion transition. Standalone backfill callers retaining a due cursor must honor `completed` too.

Bound definitions: start at a fresh paired discovery cycle with the target obligations already overdue, or restart with reconstructed finite cursors. H is an upper bound on ALL actually inspected candidates in the captured historical physical/prepared repair workload, including future/terminal/waived/unpublished candidates; it is not the number changed. D bounds ALL inspected ordinary receipt candidates in the complete fresh post-repair snapshot, including repaired rows, duplicate physical/sparse projections and finite arrivals captured before that snapshot freezes. I bounds ALL inspected invitation candidates in its initial finite snapshot, including future or subsequently ineffective rows. Q bounds elapsed time of one successful runnable GLOBAL driver quantum, including storage, scheduling and other admitted work. Later insertions outside captured high waters cannot enter or restart those walks, even when their deadlines are earlier. A finite D bound must cover any arrivals before the post-repair snapshot freezes; unbounded arrivals during repair do not justify using only the initial receipt count.

When H=0, empty repair bookkeeping completes within the first ordinary turn without reducing its capacity. With D and I at most N, the original two-class bound remains 5,000ms + (2*ceil(N/100)+2)*Q. For H>0, set R=ceil(H/50)+2: the extra two receipt turns conservatively cover finite-stream/end probes and repair completion. After those R receipt turns, at most ceil(D/100)+1 further receipt turns finish the fresh post-repair snapshot; the +1 covers a full final page's empty completion. Invitations need at most ceil(I/100)+1 invitation turns. Fair alternation therefore gives the conservative GLOBAL bound:

`5,000ms + 2*max(R + ceil(D/100)+1, ceil(I/100)+1)*Q` for H>0.

The bound concerns durable warning detection for still-pending, unextended target obligations, not bounded native key submission. It is conditional on successful bounded storage/driver service, the stated finite workload/snapshot limits and Q; it does not cover outages, repeated errors, unbounded overload, newly expiring rows already passed by a snapshot cursor, or arbitrary arrivals outside the counted snapshots. Such rows belong to a subsequent fresh cycle. Restart resumes or reconstructs finite traversal without losing repair progress; a newly reconstructed traversal must count its inspected workload again. Tests must measure actual inspected work and global quanta, rather than changed rows or receipt-only calls.

Task5 verifies the producer locally: multiple batches, finite captured high waters, late earlier-deadline arrivals excluded from repair, mixed future/terminal/waived/unpublished candidates, all-terminal old backlog, one-shot atomic completion/reset, and rollback/restart. Task7 verifies the global conversion with at least 250 candidates in EACH receipt and invitation class, both H=0 and H>0, blocked native wakes, mixed future/due rows, continuing earlier-deadline arrivals, actual <=100 total inspections per quantum, immediate successful continuation, early ordinary-due progress before long repair exhaustion, final post-repair discovery and restart. Task7 owns normal due-query high-water enforcement/full allowance lending and shared fairness; Task5 does not claim the combined bound from receipt-only tests.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### 2026-10-10 implementation refinements (Task1)

A prerequisite transactional per-instance cutover snapshot distinguishes historical logical running receipts from new sends before advertising send-time semantics. Historical sparse deadlines retain original canonical first availability only when that decision predates the cutover; post-cutover registration cannot restart a dormant obligation. Stored deadlines remain authoritative. Task5 separately owns bounded physical dormant backfill and indexing.

Modern non-lazy client sends conservatively require the capability when canonical recipient information cannot prove every recipient waived; the sender being human says nothing about recipient obligations. The deciding daemon retains human-recipient waiver. This creates a mixed-version preflight refusal limit without creating any new receipt obligation.

The public ordinary OperationStatus route requires verified operation scope that ordinary CLI wire requests lack. Pending or commit-unknown ordinary CLI/service retries therefore conservatively refuse unsupported daemons while retaining exact local journal bytes/state for later recovery. Completed retained compound/local service records remain inspectable without further effects. No fixture-only operation-status success or new inspection protocol is introduced. These accepted limits are recorded in TRUST-POLICY and Task1 report; fresh task review must trace actual public entrypoints.
