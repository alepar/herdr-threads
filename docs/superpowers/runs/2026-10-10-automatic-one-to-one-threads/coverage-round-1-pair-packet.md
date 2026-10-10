Fresh independent coverage reviewer. You did not author this tree. CALL NO TOOLS. Entire review window consists only of the complete input packet below. Check defensible omission/seam/edge/acceptance/configuration issues. Return required structured findings and root requirement mapping.

## Scope
pair; local scope nodes: ht-e99.1.2, ht-e99.1.1, ht-e99.1; external nodes supplied only to resolve consumed producer artifacts.

## Spec (complete)
# Automatic one-to-one threads: pair-boundary

Parent: [root spec](2026-10-10-automatic-one-to-one-threads-design.md) · bead ht-e99.1

## Goal

Provide the canonical pair identity, atomic resolver and immutable enrollment/audience invariant consumed by direct send and handoff.

## Problem description

This subtree provides one precisely owned boundary of the root automatic-channel behavior. The root spec is normative for syntax, authority, invitation consent, lifecycle and compatibility.

## Main challenges

Canonical pair identity must be unique and indexed, while ordinary entry points must never enroll a third seat or send outside the pair. Response loss must replay the original channel without fresh effects.

## Key decisions made

Split the compilable schema/protocol contract from its canonical transaction and membership enforcement. The first leaf supplies an inert capability boundary; the second activates only after all guards exist. Neither client consumes a partially implemented resolver.

## Decision points

### Contract ownership

The contract leaf owns the next additive migration with pair uniqueness and FKs, strict EnsureDirectThread command/result/permit/hash types, capability token, dispatch adapter signature and compatibility fixtures. Its default runtime returns unsupported until the guarded implementation is installed; it must not advertise support early. Existing serialized commands/results remain unchanged. Use the ordinary ThreadCreated/ID result if its semantics fit; no new result fields are needed just for presentation.

### Transaction and enrollment

The mechanism leaf owns the actual accountable transaction using the contract, create/invite event/projection wiring, reversed concurrency convergence, archived/left/rejected guidance and historical replay. It also owns every pair-only check in ordinary/service/operator enrollment and send preparation/publication, plus TRUST-POLICY A5. Resolve all membership through canonical effective/voluntary semantics, not stale raw state. All checks and activation land together so no advertised resolver can create a porous pair. Existing ordinary APIs stay unchanged for unmarked threads.

### Acceptance and retention

Tests use real store transactions and barriers for reverse concurrent ensure, saved-operation replay and third-participant attempts. Pair retention preserves history; cleanup cannot delete the unique pair identity and permit a new duplicate conversation while its seats/history remain valid. Migration applies only newly allocated schema with no historical mutation.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

Normative root spec:
# Automatic one-to-one threads

## Goal

An agent sends work using only a target seat ID, and a handoff without an explicit channel automatically uses the same reusable one-to-one channel. Each automatic channel reserves exactly the sender and recipient seats; messages, invitations, history, summaries, receipts and wake otherwise follow ordinary thread behavior.

## Problem description

Today send requires a thread, and handoff requires explicit existing/new channel intent. Agents repeatedly create and invite the same peer. Automatic routing should remove that setup while preserving exact identity, cooperative attribution, invitation acceptance and retry safety.

## Main challenges

Selection must converge for opposite-direction concurrent calls, remain durable across response loss, and never broadcast private work to a third participant. The current send path includes all joined members; a client-only two-member check is insufficient. Handoff has immutable V1 records, canonical fences, native possible-start recovery and new-tab bootstrap attachment; automatic routing must extend these without rewriting historical semantics.

## Key decisions made

Use an additive canonical pair registry and an accountable EnsureDirectThread operation. Direct send has its own frozen compound CLI intent with independent stable ensure/send keys; it then invokes ordinary send, rather than inventing a second delivery engine. Handoff gains an explicit automatic channel representation/version used only for new automatic plans. Existing explicit forms and V1 journals keep their bytes, hashes and behavior. Creation joins the caller and ordinarily invites the peer; reservation is not invitation acceptance. The two seats are immutable and third-seat enrollment is rejected.

## Decision points

### Interface and recipient intent

Add `ht send --seat SEAT --body TEXT`, with existing --file/--stdin body sources. The positional THREAD becomes optional and mutually exclusive with --seat; neither is invalid. Keep `send THREAD` behavior unchanged. Do not overload the positional token by guessing whether it names a seat or thread, and do not add pane-target send in this change. --seat selects the recipient, never the caller. Reject self-target, missing, retired, foreign-instance, unresolved or held recipient before pair creation. A resolved unbound seat is valid, as in handoff delivery. Existing sender A2 guards still decide every fresh effect.

Preserve default lazy send. --nudge enables ordinary delivery; --require-ack remains explicit and may name only the target seat in direct mode (duplicates deduplicate). Reject sender/third-seat ACK requests and --require-ack-pane in direct mode, rather than silently broadening or retargeting. Existing deadline, relay and user-intent validation remains. Direct lazy send must explicitly include the invited target in its frozen audience; lazy delivery does not imply joining, attention or receipt. Direct --nudge without explicit ACK follows ordinary thread-send recipient/obligation rules, including the invited target as an explicit addressed recipient. Preserve ordinary human send through immediate `human send --seat`; handoff retains its existing Agent-only restrictions.

Omitting --thread and --new-thread in handoff selects automatic routing for all supported destinations: --existing --seat, --existing with pane resolution, launch into an existing pane, and --new-tab after its guarded recipient seat exists. Explicit --thread and --new-thread retain ordinary behavior. --thread-name/--topic/--goal continue to require --new-thread. No changes to target qualification, kind/native argument grammar, body source or recovery namespace.

### Canonical identity and storage

Allocate the next unused additive schema migration at implementation time (never reuse a historical number). Store one row per canonical `(instance_id, min(seat_a,seat_b), max(seat_a,seat_b))`, with a unique unordered pair and unique thread reference, foreign keys, and distinct-seat checks. Names/topics are presentation, never identity. Automatic thread name may be absent; topic and goal default deterministically to `1:1 SEAT_A SEAT_B` using canonical sorted IDs. Rename/topic changes do not affect reuse.

Never adopt an arbitrary existing two-member thread: a manually scoped conversation may be unrelated. Only the registry identifies an automatic pair. No backfill of ordinary channels, no seat merge, no transfer when a seat retires or rebinds, and no pair generation rollover. Pair rows remain with their thread history and participate in normal retention/FK policy; cleanup must not leave a live mapping to a missing thread or silently permit a duplicate logical channel. The registry lookup is indexed and constant-size, not a directory scan.

EnsureDirectThread is an additive capability-gated command with caller claim, operation key and exact recipient. It executes in the ordinary accountable BEGIN IMMEDIATE transaction: replay exact saved result first through established historical rules; for a new effect decide A2, recipient eligibility, registry lookup and creation/reuse atomically. Creation inserts the thread, caller joined membership/interval, recipient ordinary invitation episode and pair row together, including ordinary revision bumps, timeline and attention projections. Reversed simultaneous A→B/B→A converge to one thread. Both joined and invited pair members qualify as the same pair; a reverse sender whose invitation is pending is told to accept rather than silently enrolled. A saved Ensure result never authorizes a new send; the normal send rechecks current state.

Use existing cooperative_top_level/operator_human actor provenance, record initiator and target honestly in the create/invite events, and never claim that the recipient joined or accepted. No new receipt provenance. Update TRUST-POLICY.md A5 with automatic pair reservation and the two-seat enrollment restriction in the same implementation commit.

### Membership and lifecycle

An automatic channel's participant universe is the immutable pair, with invited seats counted as reserved participants. It may have fewer than two joined members until acceptance or after leaving; the term 1:1 never claims both joined. Enforce this in every canonical enrollment path, including ordinary invite, self-join, service ensure/invite/ownership conversion and any operator invitation recovery. Either pair seat may use ordinary accept/join/leave and required-invitation rules; no operation may add a third seat, and services cannot adopt/own automatic channels. Non-pair membership attempts fail before effects. Ordinary unmarked threads retain existing enrollment.

No automatic resurrection after explicit lifecycle decisions. An archived pair returns the exact existing channel and actionable reopen guidance, without creating another pair or reopening it. A caller who left must explicitly join before direct send; an invited caller explicitly accepts. A target who left or rejected must be explicitly reinvited through the ordinary pair channel before further direct send. Ensure does not reinvite, settle old episodes or synthesize acceptance. Retired or unresolved target refusal never remaps to another seat. Existing addressed messages/obligations survive all these transitions normally. Reopening/joining/reinviting then reuses the same pair. Pair metadata stays immutable even after leaving/archival. This retains normal lifecycle semantics rather than surprising users by undoing leave/reject/archive.

Pair enforcement is permanent, eliminating the ensure→send third-member race. Direct-message preparation and publication also validate registry pair identity and reject inconsistent legacy/corrupt membership rather than broadcast. Group recipients supplied through raw protocol cannot bypass the restriction. Membership revisions and canonical audience validation retain their existing race checks. No privacy claim against same-user database edits is made.

### Durable direct send

Introduce an additive DirectSend semantic containing original frozen caller, namespace (instance plus canonical state directory/host endpoint), target seat, body/options, ensure key and send key. Persist the whole immutable intent before Ensure. A private bounded progress record stores the selected thread and exact ordinary send request after Ensure; protect execution with the journal's existing per-operation lock and atomic/synced publication patterns. Retry obtains the exact ensure result by its original key, verifies saved thread equality and resumes the same ordinary send key. It never resolves a new recipient, changes binding/actor claims or replaces a selected thread. Expose pending-ops and one `retry REF` recovery command with normal intent-kind/status presentation. Original actor classification happens before connection, presentation or cleanup.

Validate syntax, body, direct-recipient flags, caller/target and feature capability before publishing or creating a pair wherever the existing read-only preflight supports it. Authoritative validation remains transactional. The ensure and send are deliberately separate durable effects: a later send refusal/crash can leave an empty pair/invitation; report that staged thread and retain the compound intent. Never roll back committed invitations or messages. Retries do not duplicate creation/invitation/send, including lost responses and restart. An exact completed retry presents historical output and cleans up its own recovery files, without creating/reopening/joining/sending again; retain existing completed-result/binding rules. The final output names the ordinary thread and message IDs and preserves current presentation budgets.

This is preferred over a single new direct-delivery engine because ordinary send already implements bounded preparation, lazy audience freezing, receipt deadlines and publication. A client directory scan or sequential unjournaled create/invite/send was rejected for races and lost-body recovery.

### Handoff integration and compatibility

Add automatic channel intent as an explicit new serialized variant, with new plan versions only where current strict validators require them. Preserve exact V1 absent-field bytes, digests, accepted launch-report grammar, child keys and historical presentation. Do not reinterpret HandoffChannel::New or an old missing thread field as automatic. Negotiate the new capability before journal publication and first effects; an old daemon refuses safely without a client-only fallback. Capability constants, descriptor documentation and strict wire fixtures are owned with the resolver protocol; raise the wire version only if the actual compatibility rules require it, not speculatively.

Automatic existing-peer plans freeze the recipient and Ensure key before effects. The canonical handoff fence registers/validates this ensure child as the channel-selection phase and attaches its exact result inside the resolver transaction, before invite/send. Every restricted child effect/replay must match original caller, payload, namespace and canonical fence.thread; never accept an arbitrary child thread merely because the plan initially had none. The ensure's ordinary invitation may make a subsequent ordinary invite an idempotent no-op, but the frozen steps remain exact.

For --new-tab, topology creation/unknown/recovery stays unchanged. Once guarded attachment yields the recipient seat, the downstream automatic plan resolves the pair and registers the selected thread with the existing bootstrap/child fence. Preserve downstream-complete/outer-incomplete recovery and at-most-one possible native submission; automatic thread selection grants no topology creation/start permission. Startup remains managed_launch rather than check-in, invitation acceptance, ACK or task completion. Existing archival fences protect staged work until canonical completion; completed history never revives protection. No host mutation is needed for pair resolution alone.

### Testing, documentation and finish

Focused store tests exercise migration from the current schema, exact pair identity and indexed selection, reversed concurrency, response-loss replay, binding/namespace mismatch, corrupt membership, third-seat enrollment via all entry points, lifecycle refusal/reuse, human/unbound recipients, ordinary thread isolation and normal history. CLI tests exercise both send forms/body sources, lazy/nudge/ACK/deadline/relay/intent matrix and pending retry at every stage. Handoff tests exercise each of the four destination configurations early, old V1 golden journals/reports, restricted child equality, lost ensure/invite/send replies, restart and topology downstream completion recovery. Use existing fake host seams; any process test uses isolated HOME/config and spawn ownership helpers.

Update README, embedded agent skill/manual/help with target-seat send, threadless handoff, lazy default, invitations, pair membership/lifecycle and exact recovery examples. Do not edit real user configuration. No routine full-suite run while ht-zo4 remains open. Per-change cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings, relevant target-selected focused tests; before merge also nice scripts/check-default-features. Stop owned helpers and run scoped leak checks after any full/integration sweep. Keep five-minute suite/one-minute incremental lint budgets visible; do not raise them.

The caller owns the requested `join dev-herdr-threads` and integration-ready --nudge announcement when the reviewed integration branch is ready. Never push or stash; no shared Herdr server or unowned workspace/pane changes.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

## Parent goal chain
An agent sends work using only a target seat ID, and a handoff without an explicit channel automatically uses the same reusable one-to-one channel. Each automatic channel reserves exactly the sender and recipient seats; messages, invitations, history, summaries, receipts and wake otherwise follow ordinary thread behavior.

## Task tree (complete bulk dump fields)
ht-e99 (epic) Automatic one-to-one threads
parent: none
blocking deps: (none)
description:
Implement automatic target-seat-only send and threadless handoff. Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md

ht-e99.1 (epic) Canonical pair resolver and immutable membership boundary
parent: ht-e99
blocking deps: (none)
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver capability-gated EnsureDirectThread, next additive pair migration, canonical sorted-seat indexed identity, atomic create/invite/reuse, A2/history replay and permanent third-seat/service ownership guards. owns: EnsureDirectThread protocol and canonical pair resolver/membership invariant; consumes: existing accountable transaction and ordinary invitation semantics. Files: src/protocol/{commands,results,authority,capabilities}.rs, src/store/{mod,control,messages,service_substrate,schema}.rs, migrations, TRUST-POLICY.md, focused store/protocol tests. Acceptance: reversed concurrency converges, replay does not recreate, invalid inputs have no effects, pair membership guard covers ordinary/service/operator entry points, lifecycle refusal guidance correct, raw sends cannot address third seats, ordinary threads unchanged, migration tests and focused checks pass. Keep only boundary/mechanism and its own acceptance tests; CLI and handoff adoption belong to siblings.

ht-e99.1.1 (task) Pair schema and compilable resolver protocol contract
parent: ht-e99.1
blocking deps: (none)
description:
Own the next additive pair migration, sorted unique pair/index/FKs, EnsureDirectThread command/result/permit/hash/capability, inert dispatch adapter and old-wire compatibility fixtures. owns: pair schema and EnsureDirectThread compilable interface; consumes: ordinary protocol/accountability conventions. Files: migrations, src/protocol/{commands,results,authority,capabilities}.rs, src/store/schema.rs and inert src/store/mod.rs adapter, focused schema/protocol tests. Acceptance: schema upgrades/current data intact, strict roundtrip and old fixtures pass, unsupported default does not advertise resolver. No usable pair creation until guarded mechanism lands. Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.

ht-e99.1.2 (task) Atomic pair resolver and permanent audience guards
parent: ht-e99.1
blocking deps: ht-e99.1.1
description:
Implement accountable EnsureDirectThread atomic creation/reuse/replay and create/invite projections, canonical recipient checks, pair-only ordinary/service/operator enrollment and raw-send preparation/publication checks; activate capability only with full mechanism. owns: canonical pair resolver implementation and immutable pair guard; consumes: pair schema and EnsureDirectThread interface. Files: src/store/{control,mod,messages,service_substrate}.rs and related service entry points, TRUST-POLICY.md, focused store tests. Acceptance: reverse concurrency one pair, invalid caller/target no effects, invited reverse caller guidance, archived/left/rejected explicit recovery, binding and namespace revalidation, no third participant/send audience, existing normal channels unchanged, operation replay historical, indexed bounded lookup. Schema contract is prerequisite (needs: ht-e99.1.1). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.1.1: consumes pair schema and EnsureDirectThread interface

ht-e99.2 (task) Durable target-seat-only send CLI
parent: ht-e99
blocking deps: ht-e99.1.2
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver send --seat SEAT, preserved positional ordinary send, lazy/nudge/ACK/body matrix and frozen DirectSend compound journal with ensure/send keys, original caller/namespace, progress equality, pending-ops and exact retry/completed presentation. owns: DirectSend semantic/CLI coordinator and normal-send adapter; consumes: EnsureDirectThread protocol and canonical pair invariant. Files: src/cli/{commands,mod,journal,retry}.rs, src/cli/direct_send.rs if cohesive, tests/cli/{commands,journal,direct_send}.rs, tests/combined.rs. Acceptance: all body sources/modes, invited lazy target frozen audience, nudge target addressed, extra recipients/self invalid before effects, human send retained, lost ensure/send replies and restart do not duplicate, cleanup/historical replay preserve original actor, old journals unchanged; focused checks pass.
Promotion override: sp:demoted-by-session — parser/journal/coordinator/recovery are inseparable parts of one reviewable compound CLI behavior, not five independent merge-worthy deliverables; root spec fixes each decision, so further split would create ceremony below the cohesion floor.
blocked-by ht-e99.1.2: consumes EnsureDirectThread protocol and canonical pair invariant

ht-e99.3 (epic) Automatic channel handoff composition
parent: ht-e99
blocking deps: (none)
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver explicit automatic channel variant/version for newly prepared handoffs, canonical Ensure child registration/attachment, exact namespace/fence thread matching, all destination paths including --new-tab downstream. owns: automatic handoff plan/fence composition and recovery; consumes: EnsureDirectThread protocol and canonical pair invariant. Files: src/cli/{handoff,handoff_delivery,topology_handoff,journal}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and runtime modules, tests/cli/{handoff_delivery,topology_handoff}.rs, tests/handoff_topology_authority.rs. Acceptance: execute one fake-host request each for existing-seat, existing-pane, launched-pane and new-tab omission early; explicit --thread/--new-thread unchanged; V1 golden byte/hash/report compatibility; lost ensure/invite/send/start and child-complete/outer-incomplete recovery; no autoaccept/ACK/relaunch; canonical fences protected, focused checks pass.

ht-e99.3.1 (task) Automatic handoff plan and canonical attachment contract
parent: ht-e99.3
blocking deps: ht-e99.1.2
description:
Implement new-only automatic variant/version and strict compatibility validation, shared client helper signatures/stubs, registered Ensure child keys, exact canonical namespace/thread attachment and restricted phase validation. owns: automatic handoff serialized/shared adapter boundary and canonical Ensure attachment; consumes: EnsureDirectThread resolver protocol and canonical pair guard. Files: src/protocol/handoff.rs, shared src/cli/{handoff,journal}.rs types, src/store/{handoff,topology_handoff/runtime}.rs, focused golden/authority tests. Acceptance: V1 exact byte/hash/report fixtures unchanged; automatic Ensure cannot attach arbitrary thread, replay/live original caller/namespace correct; producer paths inert until implemented. Consumed resolver available (needs: ht-e99.1.2). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.1.2: consumes EnsureDirectThread protocol and canonical pair invariant

ht-e99.3.2 (task) Existing-seat and pane automatic handoff producer
parent: ht-e99.3
blocking deps: ht-e99.3.1
description:
Wire omitted channel in parser and existing-recipient/pane/native launch plans through shared automatic helper; freeze original target/caller/namespace/ensure keys and preserve override metadata rules. owns: parser omission and existing-recipient automatic producer; consumes: automatic handoff shared plan/attachment contract. Files: src/cli/commands.rs, src/cli/{handoff,handoff_delivery}.rs, tests/cli/{commands,handoff_delivery}.rs. Acceptance: one fake-host full request each existing-seat, existing-pane and launched-pane; explicit --thread/--new-thread unchanged; lost ensure/invite/send/start replies, retry/completed cleanup and recipient lifecycle preserve no duplicate effects/autoaccept/ACK. Shared contract available (needs: ht-e99.3.1). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.3.1: consumes automatic handoff shared plan and canonical attachment contract

ht-e99.3.3 (task) New-tab automatic handoff producer
parent: ht-e99.3
blocking deps: ht-e99.3.1
description:
Wire threadless new-tab into shared automatic downstream plan after guarded recipient attachment, with unchanged topology/possible-start recovery and absorbing completion. owns: automatic topology downstream producer and reconciliation; consumes: automatic handoff shared plan/attachment contract. Files: src/cli/topology_handoff.rs, topology-specific store/runtime adapters if necessary, tests/cli/topology_handoff.rs, tests/handoff_topology_authority.rs. Acceptance: one fake-host full new-tab invocation; lost attachment/ensure/send/start response, child-complete/outer-incomplete restart recovery, exact selected pair and no repeated topology/start; explicit old new-thread modes remain. Shared contract available (needs: ht-e99.3.1). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.3.1: consumes automatic handoff shared plan and canonical attachment contract

ht-e99.4 (task) Public guidance for automatic direct channels
parent: ht-e99
blocking deps: (none)
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver README/manual/embedded agent skill/help examples covering send --seat, all threadless handoff targets, explicit override forms, lazy default, invitation acceptance, two-seat reservation and archive/leave/reject recovery. owns: user-facing direct-channel documentation; consumes: syntax and semantics fixed in root spec (no code artifact dependency). Files: README.md, skills/herdr-threads/SKILL.md or actual embedded source, docs/manual.md or current manuals. Acceptance: repo search reconciles live send/handoff required-thread statements and examples; no claim launch accepts/ACKs or lazy wakes, no broad permission/config changes. Verify current guidance sources before editing; keep unrelated historical specs intact.
## Flagged tasks
ht-e99.2 sp:demoted-by-session (root scope only; nested scopes do not sweep unrelated task).
## Changes since previous round
(empty: round1)
## Rejected-findings ledger
(empty: round1; recorded promotion override in task description must be acknowledged in flag sweep)
## Requirements (canonical)

## Review checks and output contract
# Coverage Reviewer Subagent Prompt Template

Use this template for a `super-design` coverage reviewer. **Model: opus.** The same
template serves both per-subepic passes and the root pass — the caller fills in
the inputs below; nothing in the template branches on which pass it is. The
reviewer works in isolated context and must never have authored the tree.

```
Task tool (general-purpose), model: opus:
  description: "super-design coverage review: [SCOPE_NAME]"
  prompt: |
    You are a fresh-context coverage reviewer for a task tree. You did not author this
    tree. Your job is error-of-omission detection: a missed gap is worse than a false
    positive — the caller unions your findings with two other reviewers and verifies each
    one's evidence against these same inputs before applying it, so anything unsupported
    is filtered there. Surface anything plausible, and make the evidence checkable: a
    finding whose cited ids or quoted text don't hold up is dropped without a fix.
    A rubber-stamp is a failure.

    **Your entire review window is this prompt. Call no tools: do not read files, do not
    explore the repository, do not query the tracker** — the caller assembled everything
    this pass may consider into the sections below, and the bound is the point: coverage
    is judged goal-vs-tree, not tree-vs-codebase, and a reviewer that roams re-reviews
    scope some other pass owns. If the input below looks insufficient to judge the goal —
    a section empty that shouldn't be, a task tree that references specs you weren't
    given — report that as a finding (**type: INSUFFICIENT-INPUT**, naming what's
    missing) instead of going to look for it.

    ## Goal
    [GOAL — the applicable spec's `## Goal` section, verbatim]

    ## Parent goal chain
    [PARENT_GOAL_CHAIN — parent goals down to this node, outermost first; empty for the
    root pass]

    ## Spec
    [SPEC_PROSE — the relevant spec text for this scope. Above ~15 specs in the tree,
    this may be summarized to subepic goal/summary sections instead of full spec text.]

    ## Task tree
    [TASK_TREE — every task's id, title, description, and the ids it depends on (its
    blocking deps), for the scope under review — INCLUDING epic-typed nodes and their own
    blocking deps, marked `(epic)`: an epic→epic edge gates every leaf under the dependent
    epic and is invisible to a leaves-only dump, which blinds the edge audit to the
    single most expensive edge class. Leaf tasks and their dependency lists are
    dumped from the bd DB via the BULK dump (`bd list --json`), not read from spec files
    and not assembled from per-bead `bd show --json` — `show`'s dependencies field
    underreports blocking edges (verified bd 1.0.5: `show` returned no usable edges where
    the bulk dump did), so a `(none)` from `show` is not evidence of absence.]

    ## Flagged tasks
    [FLAGGED_TASKS — task ids carrying flag `sp:frozen-promotion` or
    `sp:demoted-by-session`]

    ## Changes since the previous round
    [CHANGED_TASKS — task ids created or amended by the previous round's applied fixes,
    each with the finding it answers. EMPTY on round 1.]

    ## Rejected-findings ledger
    [REJECTED_LEDGER — contents of
    docs/superpowers/specs/<root-slug>-coverage-ledger.md, if any. Findings matching an
    entry here were already disposed of — do not resurface them without new evidence.]

    ## Requirements (canonical)
    [REQUIREMENTS_CANONICAL — the orchestrator's R1…Rn list (root pass only; EMPTY for a
    per-subepic pass — skip check 0 entirely when this is empty), derived once per round
    from the root `## Goal` and root spec, one observable outcome per id. Stable across
    rounds and resumes: never renumber an existing id.]

    ## Checks

    0. **Requirement mapping (root pass only).** For each canonical requirement, list the
       task ids that deliver it. You may propose a new requirement the canonical list
       missed as `R-new: <text>` — the orchestrator appends it to next round's list; never
       renumber or reuse an existing id yourself. Every canonical requirement with no
       mapped task is **also** reported as a `GAP` in the findings list below (check 1
       still applies as before; this makes the omission explicit rather than folding it
       silently into the forward trace). Emit this mapping as the `requirements` output
       block, **before** the findings list.

    1. **Forward trace → GAP.** Decompose the goal into its necessary elements. Every
       element must map to at least one task or spec section. An unmapped element is a
       `GAP`.
    2. **Backward trace → ORPHAN.** Every task must serve some goal element. A task that
       serves none is an `ORPHAN`. Tasks titled `Seam contract:`, `Seam integration:`, or
       `Integration sweep:` are seam machinery created by coverage disposition, not decomposition — they
       serve the boundary or the tree they name and are never `ORPHAN`s.
    3. **Walking skeleton.** Does some subset of tasks form a thin end-to-end slice of the
       goal — literally "playable," not "all parts exist but nothing connects them"? Use
       a premortem framing to test it: the tree was fully executed and the goal still
       wasn't met — why?
    4. **Seam trace → UNOWNED-SEAM.** For every pair of tasks that exchange a named piece
       of data or an interface — one defines a config value the other reads, one exposes a
       function the other calls, one writes a format the other parses — check that exactly
       one task's description **owns** the boundary (its "owns:" line names it; the
       counterpart "consumes:" it). If no task owns the wiring between them, that is an
       `UNOWNED-SEAM`: name both task ids and the exchanged thing concretely, and quote
       the description text that implies the exchange. The canonical miss this check
       exists for: a config parameter implemented in a schema task and honored in a
       consumer task, with the pass-through wiring owned by neither — both tasks satisfy
       their local descriptions and the parameter ships inert. Disjoint files are not
       evidence of independence; the exchange is dataflow, not file overlap. Do not
       report a seam whose boundary a task plainly owns, and do not invent exchanges the
       descriptions don't imply.
    5. **Edge audit → NARRATIVE-EDGE.** For every blocking dep in the task tree, first read the
       dependent's description for its `blocked-by <blocker-id>: consumes <artifact>` line
       matching this edge's blocker id. No such line — or a line whose `<artifact>` cannot be
       resolved to something the blocker actually delivers (its own description, acceptance, or
       fixed token: `all leaves (integration sweep)` for a sweep edge, `boundary contract` for a
       seam-contract edge) — is a finding on its own: report it with `unstated` in the evidence,
       naming the edge (dependent ← blocker) and the missing or unresolvable line. When the line
       is present and resolves, use it (plus the two descriptions) to name the specific artifact
       the dependent consumes and the blocker produces, and check for three further failures, one
       kind: (a) UNNAMEABLE — no artifact connects them; the edge encodes narration or "that area
       first". (b) MISDIRECTED — the artifact is real but a DIFFERENT task (often earlier in
       the same sub-epic) produces it; the edge gates the dependent behind work it never
       consumes; propose repointing at the actual producer. (c) DUPLICATED — a sibling
       already owns the integration with that sub-epic and carries the same edge; propose
       consuming the owning sibling's artifact instead. If the edge's only justification is
       an UNANSWERED design question (neither description decides who owns the exchanged
       data), report it as `UNOWNED-SEAM` instead — the boundary needs a contract, not an
       ordering. Do not flag edges onto `Seam contract:` beads (that is the seam mechanism
       working), and do not flag an edge whose artifact you can name and locate at its
       producer — a real edge is not a finding.
       Audit edges on `(epic)` nodes with EXTRA scrutiny: an epic→epic edge claims every
       leaf under the dependent epic needs all of the blocking epic finished. That claim is
       rarely true and its cost is the whole subtree's idle time — if the dependents' real
       needs are specific leaves of the blocking epic, the finding is MISDIRECTED with the
       proposed fix "narrow to leaf-level edges: <dependent leaf> → <specific producer
       leaf>, and drop the epic-level edge". An epic edge you can fully justify (every leaf
       genuinely consumes the whole epic's output) is not a finding, but say so explicitly
       in one line rather than skipping it.
    6. **Acceptance satisfiability → UNSATISFIABLE-ACCEPTANCE.** Two passes, the
       mechanical one FIRST. (a) MECHANICAL — for every `(needs: <id>)` citation in a
       task's acceptance text, look the cited id up in the task tree and walk the blocking
       edges: the cited bead must be a transitive BLOCKER of the task (it completes first).
       If it is a transitive DEPENDENT of the task, that is a finding, whatever the prose
       says — the criterion cannot be met at completion time. If no edge path connects the
       two in either direction, that is a finding of the `unwired` variant — propose the
       edge. This pass is a graph walk, not a judgment: do it for every citation, and report
       each result. (b) PROSE — where an acceptance criterion names another deliverable
       WITHOUT a citation ("the X validator accepts...", "passes the Y suite", "conforms to
       §N's contract"), resolve that deliverable to the task that produces it yourself and
       apply the same direction test; a producing task that is a transitive dependent is
       unsatisfiable — the cycle lives across the acceptance-text/dependency-graph boundary,
       which no single-artifact check spans. Propose either restating the criterion in
       terms of an artifact that exists when the task completes (adding the `(needs: <id>)`
       citation so the next round checks it mechanically), or re-pointing the edge so the
       validator genuinely precedes it. A criterion referencing a blocker is correct and is
       not a finding. (Measured: six prose-only review passes missed one instance of this
       class; the citation convention exists so that pass (a) cannot.)
    7. **Configuration coverage → UNEXERCISED-CONFIGURATION.** Where the spec (root or
       subepic) ENUMERATES runtime configurations — modes, player counts, platforms,
       feature-flag combinations, a test matrix — check that some task in the tree
       *exercises* each enumerated configuration (runs it end to end: one decision, one
       request, one invocation — not merely reads or unit-tests the code behind it), and
       that this task is not solely the terminal readiness/integration gate. An enumerated
       configuration that nothing runs before the terminal gate is a finding: evidence is
       the quoted enumeration and the configurations with no exercising task; the proposed
       fix is a `Configuration smoke:` leaf (name the entry point or command per
       configuration if the spec gives one) depending on the beads that make those
       configurations runnable, or naming the one existing bead that should absorb the
       exercise. A spec that enumerates nothing yields no finding of this kind — do not
       invent a configuration space.
    8. **Flag sweep.** Every id in "Flagged tasks" is an automatic finding — known-
       underdesigned work must not sail through silently — *unless* it already has an
       entry in the rejected-findings ledger, which takes precedence over the sweep.
    9. **Previous round's fixes.** Every task named in "Changes since the previous round"
       was created or amended by this run's own coverage loop and has been read by no one.
       Run checks 1-7 over them as over any other task, and answer the two questions only
       these raise: does the fix actually close the finding it claims to (if not, re-report
       the original finding, with the shortfall as evidence), and did the fix introduce a
       seam, edge, or acceptance criterion of its own that checks 4-6 would flag? An empty
       section means this is round 1 — skip this check; its emptiness is NOT an
       INSUFFICIENT-INPUT.

    ## Required structured output (do NOT write a prose essay)

    **Root pass only:** first, a `requirements` block mapping every canonical id to the
    task ids that deliver it (`R-new: <text>` entries appended for any you propose) —
    this comes **before** the findings list, one line per requirement:

    ```
    requirements:
    R1 → bd-101, bd-104
    R2 → (unmapped)
    R-new: <text> → bd-107
    ```

    Then, one entry per finding:
    - **type:** `GAP` | `ORPHAN` | `UNOWNED-SEAM` | `NARRATIVE-EDGE` | `UNSATISFIABLE-ACCEPTANCE` (with `dependent` or `unwired` in the description) | `UNEXERCISED-CONFIGURATION` | flag-sweep
    - **description:** the problem, one sentence
    - **evidence:** the unmapped goal element, the orphaned task id, the seam's two
      participant task ids + the exchanged data/interface + the quoted description text
      implying the exchange, the edge (dependent ← blocker) + which of unstated/unnameable/
      misdirected/duplicated it is + the quoted description text (or the absence of a
      `blocked-by` line, for `unstated`), the quoted acceptance
      criterion + the producing (or cited) task id + the dependency path proving it is a
      transitive dependent (or the absence of any path, for `unwired`), the quoted
      configuration enumeration + the configurations with no exercising task, or the flag
      and its task id
    - **proposed fix:** a new leaf task under a named epic, or a new subepic needing
      design; for `UNOWNED-SEAM`, name the boundary to be contracted (the caller turns a
      verified seam into a contract bead + integration bead — see SKILL.md §Coverage); for
      `NARRATIVE-EDGE`, drop the edge, or name the task it should repoint to; for
      `UNSATISFIABLE-ACCEPTANCE`, the restated criterion (with its `(needs: <id>)`
      citation), the edge to re-point, or the edge to add; for `UNEXERCISED-CONFIGURATION`,
      the `Configuration smoke:` leaf (per-configuration entry point, and the beads it
      depends on) or the existing bead to extend

    Report only defensible findings. Quality over quantity — but do not soften.
```
