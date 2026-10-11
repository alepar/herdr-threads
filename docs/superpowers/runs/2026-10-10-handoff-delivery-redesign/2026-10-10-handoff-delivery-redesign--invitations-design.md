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


## Authorized upstream composition refinement — supersedes automatic invitation-flag refusal

Parent fact-driven ruling: Ensure actually creates the initial invitation. Automatic handoff supports --require-invite-ack and --invite-deadline for INITIAL pair creation, independently from the always-addressed ACK-required body and frozen message deadline. Add optional frozen Ensure creation policy Lazy/RequireAck plus duration; ABSENT means historical timed behavior and preserves old bytes/digests/keys. Modern direct send explicitly requests Lazy for a NEW pair invitation. Modern automatic handoff defaults that initial invitation to Lazy and can request timed acceptance. Registration/launch is never acceptance or ACK.

Preserve canonical pair lifecycle exactly: existing joined/pending episodes are reused without policy/deadline/origin changes; rejected/left pairs refuse without reinvite. A later caller requesting different creation policy cannot override/reset an existing episode. Present the ACTUAL retained policy/outcome honestly; AlreadyJoined creates no new invitation/obligation. Do not manufacture a separate Invite child: automatic remains Ensure→Send with exact canonical attachment/publication/completion proof. Existing explicit paths retain their Invite children. Test initial Lazy/RequireAck/default-duration/explicit-duration, historical absent exact bytes, duplicate policy mismatch retaining original, AlreadyJoined, rejected/left refusal and independent message/invitation deadlines (7/11 seconds).

All automatic and explicit handoff routes acquire exactly one nonempty valid UTF-8 source <=65536 bytes once, freeze exact body/policy/durations before effects, preserve NUL/whitespace/newlines and retry without rereading. Require send-time/invitation/automatic/direct capabilities as applicable BEFORE journal/allocation/Ensure/topology/native effects. Hook/setup/trust/profile readiness is advisory; structural/caller/seat/native grammar/actually-used owned fullconfig fingerprint checks and truthful native trust refusal remain mandatory. No auto trust/accept/ACK/adoption.

Released main478ec4a9 migrations0029_direct_threads and0030_automatic_handoff remain exact. Our unlanded29..35 shift31..37; add migration38 for automatic body-bearing original/recovery capacities proven with actual serializers, never rewrite released30 SQL. Preserve historical retained rows/requests/digests, genuine main30→combined upgrade/rollback/reopen and all dependent FK relationships. Count nested body copies across child/attachment/recovery/terminal/archival and enforce exact limits before effects; no clipping/unconditional globalwire increase.

Owned composition epic ht-0lo.8 has serial leaves .8.1/.8.2/.8.3, plan ordinals12/13/14. Work remains in separate isolated candidate workspace, distinct task branches/provenance reusing one owned warm validation slot serially. No provisional or unresolved composition merges into root integration until the whole reviewed A/B/C contract passes. Root closure waits. Parent owns final PR roast and subsequent expensive/defaultfeature exact-tip readiness. Prior11 ordinals and three edge-audit budget remain unchanged.


### Ensure creation-policy capability refinement

New optional Ensure creation-policy fields require their OWN honest additive capability, provisionally direct_threads.ensure_creation_policy_v1; invitation.confirmation_v1 and old automatic/direct capability do not establish support in released v0.7.0 Ensure handlers. Task13 owns constant, canonical activation/advertisement and every modern caller preflight. Advertise only when canonical Ensure policy creation is actually active. Modern Lazy directsend remains exempt from messages.send_time_receipts_v1 but MUST require this new Ensure-policy capability BEFORE journal/pair/allocation/native effects; modern automatic policy likewise requires it before any topology/durable/native effects, in addition to applicable automatic/invitation/receipt capabilities. Explicit historical absent fields preserve historical behavior/bytes and completed inspection; unsupported modern semantics never silently fall back. Test real old-v0.7 capability peers for automatic and direct routes, INCLUDING Lazy directsend, and assert no new journal/pair/topology/native effects. Task12 supplies the actual schema/canonical activation-ready cutpoint if needed with additive38; do not invent extra DDL merely to gate a capability. This is parent-authorized autonomous bounded composition within ht-0lo.8; no new confirmation request is required.


## Parent-authorized AGY composition refinement — latest provisional migration sequence

Mainab2291c5 v0.8.0 supersedes the earlier upstream478ec4a9 composition target. Released migrations0029_direct_threads.sql,0030_automatic_handoff.sql and0031_seat_wake_on.sql remain byte-for-byte exact. Our unlanded redesign29..35 shift to32..38, and automatic capacity migration becomes0039_automatic_handoff_body_bounds.sql. Prior31..37/capacity38 proposals are planning history only. Execute genuine populated released main31→combined latest upgrades/rollback/reopen; never relabel initialized latest or rewrite historicalSQL. A imports actualab2291c5 only after its frozen build drains; an owned-branch478 mechanical checkpoint is permitted ancestry, never ungated root promotion.

Additional bounded Task15 ht-0lo.8.4 owns AGY neutral hook-event/setup/launch and delivery-ledger/pull semantic composition after Task12. Task13 remains dependent only on Task12. Task14 consumes BOTH Task13 and Task15 actual reviewed artifacts; no invented B↔D dependency. Work stays in isolated .8 candidate branches/provenance; one owned warm validation slot has one writer/Cargo command at a time. Whole A/B/C/D contract must clear before root integration/closure. Original11 ordinals and prior edge-audit budget unchanged.

Preserve parent-authorized optional frozen Ensure creationpolicy Lazy/RequireAck+duration absenthistoricaltimed, modern directsendLazy initialinvite and automatichandoff independentACKbody/invite deadlines; no superseded automaticflag refusal. Task13 also owns the approved structural response-only transport seam: request-known AutomaticBootstrap plus matching result family can use a measured narrow response allowance; default request/service/watch/unrelatedreply limits remain1MiB. Numeric ceiling requires actual serializer evidence, no clipping/dedup/history rewrite.

AGY hooks/setup/launch compose via neutral registry/adapter traits, not harness-name branches in neutral core. Stable per-event owned hook command bridges remain exact-route, equivalent reinstall unchanged, foreign/drifted groups survive and explicit migration recoverable. AGY rejects rewritten/ownednativeargv: preserve structural/native grammar/host and usertrust refusal, while hook/setup/plugin/profile readiness is diagnostic-only for an otherwise safe launch. Recheck actually-used owned fullconfig fingerprint only when used; absence never fabricates one. No auto trust/accept/ACK/adoption or version admission gates.

New in-turn delivery must expose independent still-open warnings for EACH canonical original receipt sender/inviter, including departed origins, after informational frontier settlement and offered-ledger/Unchanged/whole-read suppression. Preserve one coalesced condition/event, exact associations, partial/final lifetime, passive clear, canonical nullable attribution and service-author handling; never synthesize sender from NULL/currentmembership. Lazy invitation is discoverable but not timed/native attention, RequireAck/historicalNone remaintimed. Preserve finite due-driver highwaters/budget/fairness/expandedH combinedreset; delivery visibility or native completion cannot gate discovery. Message ACK remains receipt only.

Read ACTUAL upstream AGENTS.md. Every NEW commit includes Outside-adapter: <none | comma-separated repo-relative paths/globs> accounting for all changed paths outside src/harness/agy/; diff remains authoritative. Run scripts/check-harness-neutral in addition to fmt/clippy, no new permissive allowlist entries without actual frozen-wire/registration rationale. No real ~/.claude/~/.codex/~/.aisw/~/.gemini writes. Use isolatedHOME/config, taggedchildren/private sessions and scoped leak cleanup. Lazy messages never idlewake unless wake_on:all opted-in; ACKmessages nudgeclass, generating/tool-running servedinturn notnudged; wakeprompt contentfree. Never gate on exact harnessversion, degrade only observedbroken capability. Parent owns final roast then exact-tip expensive/defaultfeature readiness.


### Lazy invitation positive delivery acceptance clarification

Lazy invitation confirmation/nativewake suppression MUST NOT suppress in-turn pages, turn-hook/channel context or bookkeeping/discoverability (R9/R17). Preserve full display/completeness: whole(item) is not actionability. Give Lazy invitation proper informational classification while carrying it through actual delivery::Assembler and public turn-hook/service DeliveryPull output. Add positive actual Lazy invitation page/context/bookkeeping assertions AND nativewake/confirmationtimer absence under ordinary wake_on policy, with explicit wake_on policy behavior retained. RequireAck and historical None retain their timed attention. Preserve independent departed canonical sender/inviter warning visibility through ledgerUnchanged/whole_read/noticefrontier and final passiveclear; never remove rows just to suppress attention.
