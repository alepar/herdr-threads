# Handoff delivery redesign: invitations

## Goal

Ordinary invitations are lazy unless confirmation was explicitly requested; timed invitations settle at acceptance and notify originator on expiry independently of registration.

Parent: [root design](2026-10-10-handoff-delivery-redesign-design.md); bead ht-0lo.2.

## Decisions and boundaries

The parent spec is the normative behavioral contract for this subtree. It specifies the selected alternatives, compatibility rules and edge cases; this spec scopes ownership and execution, without adding a second interpretation. Ordinary invitations are lazy unless confirmation was explicitly requested; timed invitations settle at acceptance and notify originator on expiry independently of registration.

## Tasks

### Invitation confirmation protocol and schema contract

owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.

### Invitation acceptance, due and delivery lifecycle

owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.

### Invitation CLI and presentation contract

owns: ordinary invite --lazy default, --require-ack/--deadline implication and conflicts, compact/JSON/inbox/help/embedded skill presentation; consumes: invitation confirmation protocol/store contract. Files: src/cli parser invitation branches/mod/output and help/skill resources, relevant CLI tests. Invoke actual CLI in isolated daemon/config for bare lazy, require-ack default duration, explicit deadline, conflicting flags, duplicate and historical keyed retry. Required/operator commands remain unchanged.

## Validation and failure handling

Each task runs its listed configurations before terminal integration with isolated config/daemon/host fixtures and owned test-support children. Preserve immutable keyed historical requests and terminal outcomes; new incompatible fields negotiate before durable effects. All implementation follows AGENTS.md lint/format/test and TRUST-POLICY cooperative canonical authority. No real user config edits, automatic trust or shared server management.

## Coverage refinements


Original inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.

## Confirmed roast refinements

Healthy-daemon discovery retains the 5,000ms safety tick, a TOTAL cap of 100 actually inspected candidates per global due-driver quantum, immediate successful continuation and fair receipt/invitation turns independent of native wake. Queries enforce finite stable high-water snapshots before LIMIT; earlier-deadline arrivals outside those high waters cannot restart or starve the captured walk. With no historical repair work (H=0), full-capacity ordinary scanning and at most N inspected candidates per class retain the original bound: `5,000ms + (2*ceil(N/100)+2)*Q`.

Historical repair requires the upgrade-aware contract in [receipts design](2026-10-10-handoff-delivery-redesign--receipts-design.md), also explained in [upgrade-discovery-refinement.md](upgrade-discovery-refinement.md). For H>0 the bound is `5,000ms + 2*max(ceil(H/50)+2 + ceil(D/100)+1, ceil(I/100)+1)*Q`. H counts ALL inspected captured deadline-repair AND physical/sparse projection-bootstrap candidates, counting duplicate visits again, including staged/terminal/future/waived rows; D bounds the complete fresh post-repair ordinary due snapshot including repaired rows and finite captured arrivals; I bounds the retained invitation snapshot. Repair retains at least 50 ordinary-due candidate slots per receipt turn, then resets the ordinary cursor exactly once at atomic repair completion, before same-call due selection. This is an upgrade-completion reset, never an arrival-driven or every-batch reset. Task7 must enforce normal physical/sparse high-water predicates and fill/lend unused allowance. The receipts design's definitions, accounting, proof conditions and Task5/Task7 review scenarios are normative for both formulas.

The bound starts with already-overdue target obligations at a fresh discovery cycle (or restart with reconstructed finite cursors); Q includes elapsed storage/scheduling of successful runnable GLOBAL quanta. It promises durable warning detection, not bounded native key submission; outages, repeated errors, unbounded overload, uncounted arrivals and rows expiring after their cursor passed are outside the stated snapshot guarantee. Validate at least 250 candidates in EACH invitation and receipt class, H=0/H>0, blocked wakes, earlier-deadline continuing arrivals, actual inspected counts, all-terminal historical repair, one-shot atomic completion/reset and restart. Existing Task5 and Task7 ownership covers this refinement; no new implementation leaf is introduced.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


### Bounded projection bootstrap refinement (Task7)

Migration35 captures only indexed MAX physical receipt ordinal and raw sparse receipt_state rowid plus durable progress/completion; it performs no historical INSERT SELECT scan. Immediate canonical association/eligibility triggers cover new work. Bounded receipt-turn repair bootstraps historical open-origin associations and monotonic sparse due projections from genuine old rows. H includes ALL visits from Task5 physical/prepared deadline repair AND migration35 physical/sparse bootstrap; duplicate physical visits count again, as do terminal/future/waived/unpublished or otherwise ineffective rows actually visited. D and I include every actually traversed snapshot candidate, including future/lazy/non-due candidates whenever visited; neither means only warnings emitted or currently due rows.

All repair producers share the same at-most50 repair slots within the100 receipt-turn allowance. Unused quota and zero-row end probes lend within that same quantum; there is no hidden phase turn or second safety tick. The existing R=ceil(H/50)+2 conservative envelope is retained only with that measured full-allowance/end-probe proof. One combined global completion/reset occurs atomically only after ALL repair/bootstrap producers finish, before same-transaction ordinary due selection. Existing Task5 producer-local completion semantics remain intact; a local completion cannot prematurely consume the combined global reset. Failed decisions roll back progress, completion/reset and warning effects together. H=0 empty bookkeeping leaves the full100 ordinary slots in its first turn.

Fresh Task7 and integration review must prove genuine34→35 populated rollback/reopen, every preexisting eligible sparse obligation and associated open origin reconstructed without startup-wide scan, all visited work/lending/end probes, combined reset exactly once, all-terminal old backlog, continuing earlier-deadline arrivals, and deletion/reinsertion/rowid reuse during bootstrap. Triggers give newly eligible sparse candidates fresh monotonic ordinals; historical raw rowid reuse cannot lose obligations or rewrite canonical receipt/sender identity. Native wake admission remains independent. The conditional H/D/I bound and original H=0 formula remain unchanged, with expanded H honestly measured.
