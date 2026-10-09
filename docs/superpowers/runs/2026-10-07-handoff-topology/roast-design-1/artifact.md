# Settled design review artifact

Source revision: 18a4c0c1. The following root spec, nested specs and full settled task tree jointly form the named artifact. Review their combined contract and ownership.


# Source: 2026-10-07-handoff-topology-design.md

## Goal

An agent can hand off work to a newly created Herdr tab through one durable command, or deliver work to an existing peer without restarting it. Exact routing, canonical seat authority, explicit channel choice and conservative replay prevent duplicate topology, invitations, messages and native launches across failures.

Status: user-approved scope; autonomous Mode B refinement and design roast enabled. Source prerequisite 8106f5cade8ac9d4c2dd8e9b3281e8df05abd8ab remains frozen. This run's branch is super-auto/handoff-topology; integration/release follows the external main window, not an implementation bead.

## Problem description

The existing handoff command requires a pane that already exists, so agents receiving a new-tab request manually bootstrap host topology and seats outside the compound. Existing working peers also need durable delivery without launching another agent.

## Main challenges

Herdr creation is correlated but not idempotent. Failures can lose a successful creation response, local progress is not canonical authority, and topology must compose with strict seat restoration, frozen actor attribution, archival protection and independent invitation/receipt semantics.

## Key decisions made

Use a separate immutable bootstrap and canonical one-use creation reservation, exact correlated topology recording, ordinary seat guards and explicit operator recovery of unknown creation. Preserve old handoff serialization and add a separate delivery mode; keep topic/goal/participant selection explicit.

## Context and decisions

The 2026-10-03 CLI handoff and 2026-10-04 archival seam already provide immutable HandoffPlan journals, exact keyed create/invite/send, absorbing canonical Begin/Complete fences and a possible-start launch boundary. They remain historical contracts. The pending handoff draft and five BEFORE samples show manual tab/seat creation because handoff requires an existing pane. The current host transport is bounded, correlated and witnessed; tab.create is not currently admitted. Installed Herdr protocol22/schema1 exposes tab.create with workspace_id, cwd, label, env and focus, returning tab/root_pane; a request correlation ID is not an idempotency key. Parent rechecked installed schema with `/Users/alepar/.local/bin/herdr api schema --output`: protocol22/schema1 SHA256226d4ecbd128d2e6bc84e4c8ddcec21ba9c7e51a0aafffcf087111ead3f1fa9a, exact projection retained in native-schema-audit.json. Implementation rechecks it before expanding the exact allowlist.

Recommended approach: a separate immutable topology bootstrap plus canonical durable submission fence, followed by the existing compound handoff. An uncertain native creation requires an explicit local operator decision. This preserves at-most-one submission per creation attempt while permitting inspected recovery. Rejected alternatives: manual native bootstrap in the skill (does not meet goal); automatic adoption by labels or before/after snapshots (no canonical authority and unrelated topology may race); automatic resubmission after a lost response (duplicates tabs); permanent refusal after uncertain creation (safe but unnecessarily strands work).

Topic/goal matching remains an agent's practical cooperative assessment. The product requires explicit thread selection and exact intended seat selection; it does not invent a semantic matching engine or treat labels as membership evidence. Receipt ACK, invitation acceptance, registration and task adoption are separate.

## CLI and compatibility

```
herdr-threads handoff --new-tab parser-fix --space w4 --cwd /absolute/worktree \
  --new-thread --thread-name parser-fix --topic 'Parser fix' \
  --goal 'Fix and verify parser defect' --kind codex -- 'Task'
herdr-threads handoff --pane w4:pX --thread THREAD --kind codex -- 'Task'
herdr-threads handoff --existing --pane w4:pX --thread THREAD -- 'Task'
herdr-threads handoff --existing --seat SEAT --new-thread \
  --topic 'Parser review' --goal 'Review parser change' -- 'Task'
herdr-threads retry REF
```

Exactly one target mode: new tab, existing explicit pane launch, or existing peer delivery. --new-tab LABEL conflicts with --existing/--tab/--pane/--seat; --cwd applies only to new-tab, must normalize to an absolute existing directory before publication, and defaults to invocation cwd. --space for creation is a uniquely resolved live workspace ID; omission uses the caller's live workspace, never focus. --new-tab is intentional creation even when an identical label exists. It does not focus the new tab. --existing requires exactly one --pane or --seat; parents may qualify pane selection but conflict with --seat. Explicit launch retains scoped --space/--tab/--pane behavior. Missing existing selectors fail rather than creating topology.

--kind is required for creation/launch. --kind, --harness-binary, --name and --agent-arg are forbidden in delivery. The one quoted argument after -- is the durable body; native options remain repeatable --agent-arg=OPTION. Preserve optional user-managed HERDR_THREADS_CODEX_OPTS/CLAUDE_OPTS, shell-like parsing without expansion and exact frozen argv. Delivery ignores launch option environment and never performs native preflight/start. Existing handoff defaults for new-thread topic/goal remain backward compatible; new-tab recipes explicitly supply both. Exactly one --thread ID_OR_NAME or --new-thread; thread names resolve once, ambiguity refuses, resulting ID is frozen. New thread fields conflict with --thread.

Delivery resolves an existing canonical seat without allocating, moving, rebinding or registering one. Unresolved, held, retired and foreign-instance seats refuse. An unbound resolved seat may receive staged durable work but the report must not claim a working/available session. The sender must be joined to an existing channel. Invite only when the intended recipient lacks compatible current participation; use existing canonical invitation replay semantics, do not create a second episode on retry. Address the durable body to that exact recipient even while acceptance is pending. New thread creation joins the sender. Delivery output is an honest delivery/staged-work report; never fabricate a LaunchResult or started/available session. Neither mode accepts nor ACKs for the recipient. Launch mode retains the bound-agent guard and never silently relaunches an existing working session.

The installer-permissions lane owns the immediate `human` namespace and invocation actor gate. Planned recovery surface is `herdr-threads human handoff recover REF --created-pane EXACT_PANE` or `--not-created`, with pinned routing/state/socket/format flags following immediate argv1 `human`. Exact recovery option and dispatch signatures must use that lane's approved contract before the recovery leaf is edited; no independent root --operator shortcut. Recovery is an administrative operator action, not person-attributed receipt communication. Human namespace invocation and read-only --human output formatting are distinct.

## Immutable identities and owned interfaces

Do not add fields to or reinterpret old HandoffPlan, HandoffRequest, HandoffIdentity, semantic digests or retained reports. Add separate journal variants `HandoffBootstrap` and `HandoffDelivery` with their own validation/digest identities. The seam-contract leaf owns versioned BootstrapPlan, DeliveryPlan, topology state/result/error types and inert dispatch/signatures; every wire entry is declared there once. Existing handoff internals may extract a staged-work helper but old serialization stays unchanged and golden historical journals replay identically.

Bootstrap immutable identity freezes exact canonical instance/state-dir/host-endpoint namespace, original CallerClaim/IntentScope, compound key and digest, workspace ID, absolute cwd, label, focus=false, env payload (empty string-map in this grammar, no environment-copy feature; never copy caller HERDR_PANE_ID, identity or routing into the new tab), normalized channel choice/body, harness/options, topology operation keys and deterministic seat-resolution/downstream keys. Canonical identity is (instance, original actor scope, compound key, immutable digest and full frozen payload comparison), never global compound key alone. The daemon compares submitted normalized namespace with its own selected InstancePaths, not only the same caller-supplied path from an earlier command or a copied instance UUID. Same ID/different payload, copied UUID in a foreign state root or endpoint, actor mismatch, altered argv and routing mismatches refuse before side effects. Path normalization uses the existing archival namespace contract; original frozen bytes are never rewritten.

Native interface: a typed `create_tab` request produces either correlated `CreatedTab {tab, root_pane, host_incarnation, terminal witness}` or a bounded submission outcome. Herdr response provides terminal_id but no host incarnation field; the latter comes from the existing transport peer/socket/process witness, never an invented response field. Use existing socket identity/kernel peer/process witness and transport budget/cancellation; validate exact result type, workspace containment, pane/tab coherence, host incarnation and correlation. Missing, empty or uncertain required terminal witness after submitted creation is outcome_unknown, never permission to recreate. No uncorrelated executable stdout or label discovery counts as creation evidence. A transport request ID correlates only. Adapter owner w4:pCC owns launch/registry interfaces; topology code consumes the agreed typed host boundary and does not introduce harness-provider selection APIs.

Canonical interface: BeginBootstrap, ReserveBootstrapAttempt, RecordBootstrapCreated, AttachBootstrapHandoff, CompleteBootstrap, CheckBootstrapSubmission, BootstrapStatus and operator recovery are additive commands/results, wired under current expected-instance/boot validation and A2 deciding transactions. All mutation keys are deterministic children of the frozen compound and phase/attempt. One additive migration creates bootstrap fences, per-attempt submission states, recovery decisions and exact downstream attachments with unique constraints and live-thread archival indexes; main approved exact ONE number0028 via mp6wXQY6o after warning0025/lazy0026/adapters0027; the canonical store leaf must absorb those actual reviewed predecessors before editing/applying0028, without placeholder migrations. Historical migrations remain byte-for-byte untouched. Wire/protocol admission changes are coordinated with peer owner; capability/schema mismatch refuses before tab.create or semantic mutation, never silently degrades into manual creation.

## Topology state machine and response loss

Bootstrap starts `prepared`, no native effect. Begin installs a canonical exact fence and protects an existing selected thread in the same transaction after membership/archive checks. It records a frozen create-thread child key for future atomic attachment, so a requested new thread attaches to both bootstrap and downstream fence in its deciding create transaction. Live fence never expires or completes because a local file disappeared, a seat retired, invitations settled or a receipt was ACKed.

Reserve changes one attempt to `possible_creation` in the daemon transaction BEFORE returning its one-use submission authorization. Only the transaction that first transitions that attempt may issue authorization; a replay/status can report possible creation but must never return permission to submit again. Local progress durably records possible creation before native submission too. A crash or response loss anywhere after reservation conservatively leaves possible creation, including when no bytes reached Herdr. Concurrent retry processes are serialized by the operation lock AND canonical attempt uniqueness; a copied journal or a daemon restart cannot issue a second permission. A local journal is a conservative fence, never authority.

Immediately before native submission, CheckBootstrapSubmission validates the original current caller A2 claim, exact live attempt and unchanged administrative decision in a fresh canonical view without issuing another submission permission. The native transport freshly rechecks the same host incarnation/socket/process witness at the final write boundary; earlier reservation does not indefinitely authorize later effects. This is a bounded cooperative pre-submit check, not a distributed transaction across daemon and Herdr. Native tab.create is invoked at most once per reserved attempt. A correlated successful result is persisted locally and recorded canonically by exact attempt before seat resolution. Once canonical creation evidence exists, loss of the local save/reply can recover through BootstrapStatus; never create a second tab. If no durable confirmed result exists, malformed result, disconnect, cancellation, crash, timeout or failed result persistence gives `outcome_unknown`, ready inspection/recovery guidance and no new creation. Only proven NotSubmitted evidence may mark a no-effect attempt retryable automatically; if the transport cannot prove it, unknown is mandatory. No guessed error class resets the boundary.

The result records the witnessed exact tab/pane/terminal/incarnation. Retry never resolves its label again. Current host reads and canonical ordinary guarded seat resolution must still confirm that exact topology, same incarnation and mapping. Contradiction, missing pane, incarnation change, restore hold or unresolved mapping refuse without relocating or closing anything. Creation witness is not a bypass for restore holds; only ordinary post-baseline guards determine eligibility. Actual canonical recipient and downstream immutable handoff identity attach once transactionally; response-loss replay returns that same attachment. Attachment includes the deterministic resolve operation key/result, not a new ad hoc resolution.

After attachment, launch mode runs existing Begin/Complete, exact create/invite/send children and persisted possible-start gate. The downstream start gate is never reset by topology recovery. The bootstrap is completed only after canonical downstream completion and retained terminal report validation. Existing-thread protection exists before topology effects. For new-thread mode attachment/create transactions own cross-fence links. Completed bootstrap is absorbing: exact retry presents/flushed retained historical result and removes only its own local journal/progress; zero tab creation, resolution, invitation/message, registration or launch, even after archival, membership departure, binding change, daemon restart or delayed legacy scan. Corrupt/missing retained presentation reports refuse rather than invent completion output.

## Human recovery, authority and cleanup

The separate recovery action records an operator:local-user:<uid> decision in its own scope. It references the bootstrap's immutable original agent identity and exact attempt but never mutates its frozen CallerClaim, IntentScope or digest. Original SemanticMutation+CallerClaim harness+IntentScope are classified BEFORE every retry path, including completed presentation/cleanup; recovery argv and current binding cannot reclassify the original operation. The installer lane supplies `cli::actor_route::InvocationActor { Agent, Human }`, `split_actor_argv(&[String]) -> Result<(InvocationActor, Vec<String>), ApiError>` (preserve argv0, strip immediate argv1 human only), nonserialized `ParsedCli.actor`, and `journal::classify_original_actor(&IntentScope, &SemanticMutation) -> io::Result<OriginalActor> { Agent, HumanOrOperator }`. New bootstrap/delivery variants integrate into that exact original actor classifier; no fake wire field or new frozen claim discriminator. Root accountable routes refuse human/operator origin; person routes reject inappropriate agent operation replay according to that shared contract.

--created-pane records the operator's explicit same-user assertion that this exact result belongs to the uncertain attempt. It must freshly obtain coherent current structural evidence and ordinary seat guards; never adopt by label, transfer a seat, release a restore hold or fabricate launch/occupancy evidence. Recovery can record topology evidence but does NOT perform downstream actions; the original agent retry still needs current A2 authority for live work. --not-created explicitly asserts both inspected noncreation and that the original attempt is quiescent/cannot still create; a known in-flight invocation holding the normal operation lock refuses recovery. Neither snapshots, status, labels nor heuristic PID discovery proves quiescence, and recovery never kills unowned processes. It records the operator's inspection assertion and closes that uncertain attempt as no-effect, enabling a distinct next attempt. Replaying either recovery key is presentation only; concurrent contradictory recovery refuses. A delayed success from an older attempt after a no-created decision is a conflict that cannot overwrite a later attempt or attach/launch either topology automatically. Exact decisions are retained and auditable. Completed bootstrap recovery cannot reactivate it.

Accepted limit: Herdr lacks tab creation idempotency. Across response loss the product sacrifices automatic progress; a mistaken noncreation or quiescence assertion can permit duplicate topology, so evidence/claim provenance remains explicit. Refusing recovery forever is not chosen. This new limit and operator action semantics update TRUST-POLICY.md in the same implementation change.

Product cleanup never closes a created tab automatically. It may now contain a working session, human activity or committed work. Confirmed exact IDs are owned-evidence inventory; uncertain creation records only possible ownership and never authorizes closing a lookalike. Explicit reports preserve tab/pane, journal/compound/attempt, downstream refs, status and ready guarded recovery/inspection argv. Private fixtures alone tear down their owned topology and processes. Worker evidence/worktree/tab remain until independently verified DONE+MERGED.

## Archival and compatibility

Extend bounded read-only legacy validation for the new bootstrap/delivery shapes, exact original namespace and allowed progress layouts. Unknown/malformed/inaccessible/oversized/symlink/partial coverage continues vetoing archival. Valid bootstrap with selected existing thread is a veto; new-thread bootstrap attaches canonical created thread by exact frozen child key. Import creates only legacy_local_journal_hint protection, never submission permission or authority. Bootstrap absorbing completion dominates stale scan samples in the deciding import transaction, including after restart; delivery completion similarly uses existing canonical Handoff identity. No importer deletes or rewrites journals or historical digests. Operation statuses and migration replay stay bounded; reject version/capability mismatch before side effects.

## Skill guidance and validation

Priority FIRST handoff for work in another/new tab, including --existing peer delivery; SECOND send only after reading channel topic+goal/remit/history and verifying the intended canonical recipient participates; THIRD join/invite when membership itself is needed. Practical relevant ordinary invitation acceptance without another human confirmation is preserved from frozen8106; required/subagent/trust/receipt rules remain unchanged. Ambiguous topic/goal or membership is surfaced with discovery options, never guessed. No manual tab/seat bootstrap recipe remains for ordinary new-tab handoff. Pending/ref/retry guidance preserves receipts vs adoption/completion.

CLI TDD records meaningful observed failing assertions before behavior edits and corresponding GREEN: grammar conflicts/missingselectors; exact route/options freezing; new-tab success/replay; pre-submit proven no-effect; reservation response loss; host response loss; crash/save failure; concurrent duplicate invocation; existing-thread fence before native effects; host incarnation/restore guard; operator assertion replay/contradiction; existing-peer no-launch with joined/invited/unbound states; bootstrap attachment/complete/archival/delayed scan; historical journal golden compatibility. Where possible use deterministic model-free fake socket peers for loss/replay and one isolated owned native Herdr session for actual typed topology boundary. No live Claude/Codex model or real config is needed.

A Configuration smoke leaf actually invokes all three modes (legacy pane launch, new-tab launch, existing-peer delivery) against owned test fixtures, including pinned state/socket routing and two harness option payloads via owned helper executables. Terminal Integration sweep owns remaining cross-seam end-to-end cases and focused checks, not a full suite. Parent orchestrator owns five independent AFTER agent pressure evaluations paired against retained BEFORE new-tab/existingpeer/ambiguity/invitation samples and honest scores. The retained BEFORE explicitly assumed handoff lacks tab creation, so comparison to the new product is a coupled capability+guide evaluation. A guide-only causal claim needs at least five new no-guidance-control and five candidate samples with identical truthful current CLI help/capability in both arms; never retain the false old limitation in AFTER or invent reproduced baseline failure. The skill author leaf supplies candidate guidance and scenario inputs but spawns no agents. Final verification records exact sourceSHA/base/tree, nice locked clippy alltargets/allfeatures -Dwarnings, nice scripts/check-default-features, fmt/check, gitdiffcheck, relevant CLI/replay/topology/authority tests, rawlogs and UUID-scoped owned process/topology inventory. Children use test_support::spawn owned helpers, are stopped/reaped and scoped-leak checked. No shared-server restart, real configs or per-tab full suite.

## Follow-on and implementation ownership

Implementation tree ends at reviewed merge-ready deliverables; main landing permission/window, integrated full suite/CI, release and local installation are external coordinator follow-on. The user already authorized reviewed worker main-only landing/push after concrete coordinated window; this is not general push authority. No deployment bead may prevent legitimately closing this run's root epic. Preserve frozen prerequisite8106 and evidence; main advances must be absorbed before final review without editing old migrations. Adapter/human namespace peer interfaces are coordinated directly; only actual blocking shared boundaries gate their integration leaf, no global HOLD.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


# Source: 2026-10-07-handoff-topology--canonical-bootstrap-design.md

## Goal

Canonical exact identity/attempt transitions/atomic attachment remain durable and absorbing across concurrent replay and response loss.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.2. Autonomous Mode B.

## Decisions

Inherit the root spec exact grammar, immutable identities, authority and failure contracts verbatim; no independent namespace or retry policy. Split only independently reviewable artifacts with actual consumed boundaries.

### Schema and exact bootstrap identity

Land minimal additive0028 schema, unique immutable namespace/keys and bootstrap current-state lookup with inert command wiring.
owns: canonical bootstrap persistence and exact identity lookup;
consumes: topology boundary contract.
Files: migrations/0028_handoff_topology.sql, src/store/topology_handoff.rs, src/store/schema.rs, src/store/mod.rs.
Acceptance: actual reviewed0026/0027 predecessors absorbed before migration edit; no placeholders; migration/upgrade/tamper and exact payload/namespace mismatch RED/GREEN; historical bytes and old Handoff shapes unchanged; bounded indexed lookups.

### One-use creation attempts and operator recovery decisions

Implement canonical reserve/evidence/recovery state transitions in deciding transactions.
owns: canonical bootstrap state transitions, one-use submission authorization and administrative recovery decisions;
consumes: canonical bootstrap persistence and exact identity lookup.
Files: src/store/topology_handoff.rs (attempt transition module if split), tests/store/topology_attempts.rs.
Acceptance: concurrent/lost-reserve reply yields at most one authorization; exact same-attempt evidence replay; proven NotSubmitted vs unknown; operator created-pane/not-created replay/contradiction and late older creation conflict; live A2 checks and completed recovery cannot reactivate; meaningful RED/GREEN.

### Atomic recipient/thread attachment and absorbing completion

Implement cross-fence exact resolution/recipient/downstream attachment and bootstrap terminal state.
owns: canonical bootstrap attachment, archival protection and absorbing completion;
consumes: canonical bootstrap persistence and exact identity lookup.
Files: src/store/topology_handoff.rs (attachment module if split), src/store/control.rs, src/store/archival.rs.
Acceptance: existing-thread protected before topology; create-thread attaches in deciding create transaction to both fences; deterministic resolve key/result; contradictory attach refuses; completed historical presentation bypasses only live guards, no effects or revived protection; delayed importer terminal precedence; RED/GREEN.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


# Source: 2026-10-07-handoff-topology--bootstrap-coordinator-design.md

## Goal

A new-tab command freezes exact intent and performs at-most-one native creation per canonical attempt, then existing guarded launch and terminal replay.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.4. Autonomous Mode B.

## Decisions

Inherit the root spec exact grammar, immutable identities, authority and failure contracts verbatim; no independent namespace or retry policy. Split only independently reviewable artifacts with actual consumed boundaries.

### New-tab grammar and immutable publication

Implement strict target/channel grammar and additive BootstrapPlan publication.
owns: frozen bootstrap publication and normalized new-tab grammar;
consumes: topology boundary contract.
Files: src/cli/commands.rs (new-tab arguments), src/cli/topology_handoff.rs (publication), src/cli/mod.rs.
Acceptance: actual CLI parse-conflict RED/GREEN; absolute existing cwd, livecaller workspace vs focus, exact state/socket/options/child keys; no manual nativecreate or effects during parse; historical pane grammar unchanged.

### At-most-one native creation and exact seat attachment

Implement operation-lock/reserve/local-progress/native-call/evidence/guarded-resolution path.
owns: BootstrapPlan live resume and native submission progress;
consumes: frozen bootstrap publication, canonical bootstrap state transitions, canonical bootstrap attachment and typed CreatedTab/submission outcomes.
Files: src/cli/topology_handoff.rs (live coordinator), src/cli/retry.rs.
Acceptance: RED/GREEN successful create, lost reserve/host reply/save, crash/concurrentcopiedjournal, local/canonical recovery, incarnation/restorehold exactrecipient guards and deterministic resolution; never close user tab or auto adopt label; reports retain exact IDs/attempts.

### Downstream launch and terminal bootstrap replay

Compose exact existing Handoff downstream with persistent possible-start fence and bootstrap completion/report cleanup.
owns: bootstrap downstream completion and absorbing terminal replay;
consumes: BootstrapPlan live resume and canonical bootstrap attachment.
Files: src/cli/topology_handoff.rs (downstream/terminal), src/cli/retry.rs, src/cli/handoff.rs (existing compound helper only).
Acceptance: RED/GREEN no duplicate create/invite/message/launch on failed flush/removal/restart/archive/binding change; frozen terminal report required; completed replay only presents/removes owned intent, wrongnamespace refuses; launch possible-start preserved.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


# Source: 2026-10-07-handoff-topology--existing-delivery-design.md

## Goal

An existing canonical peer receives exact staged work and replayable completion without native launch or identity replacement.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.5. Autonomous Mode B.

## Decisions

Inherit the root spec exact grammar, immutable identities, authority and failure contracts verbatim; no independent namespace or retry policy. Split only independently reviewable artifacts with actual consumed boundaries.

### Exact existing recipient and durable staged-work delivery

Implement --existing selector grammar, canonical seat lookup and keyed shared staging with immutable DeliveryPlan.
owns: DeliveryPlan staged-work execution and existingpeer selector resolution;
consumes: topology/delivery boundary contract and existing canonical Handoff fences.
Files: src/cli/handoff_delivery.rs, src/cli/commands.rs (existing mode), src/cli/handoff.rs (shared staging extraction).
Acceptance: actualCLI RED/GREEN no allocation/rebind/start/preflight/registration, held/unresolved/retired/foreign refusal, canonical joined/pendinginvited/unboundreporting, exact invitation episode reuse and send children; original Handoff serialization/golden unchanged.

### Delivery retry and completed presentation cleanup

Integrate journal retry and terminal reporting for existing-peer durable delivery.
owns: DeliveryPlan retry and absorbing terminal presentation;
consumes: DeliveryPlan staged-work execution.
Files: src/cli/handoff_delivery.rs (retry/report), src/cli/retry.rs.
Acceptance: RED/GREEN changedcurrentbinding/archive/restart completion cleanuponly, failedflush/removal retry, exactorigin classifier before all paths, zero duplicate invitation/message or nativeeffects; retained report corruption refuses.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*


# Source: tree-settled.json

[
  {
    "id": "ht-qhz.20",
    "title": "Integration sweep: durable topology handoff",
    "description": "Verify integrated new-tab, legacy pane-launch and existing-peer flows plus unknown topology/recovery/archival boundaries through focused end-to-end tests.\nowns: terminal focused integration gaps and exact merge-ready source/check/process/topology evidence;\nconsumes: every implementation and seam-integration deliverable.\nFiles: tests/handoff_topology_integration.rs or combined module; run source-bound verification evidence.\nAcceptance: add meaningful cross-seam RED/GREEN tests not already owned; small gaps fixed inline, large blockers filed; exact sourceSHA/base/tree, raw focused CLI/replay/authority/topology logs, nice locked clippy alltargets/allfeatures -Dwarnings, nice scripts/check-default-features, fmt/check and gitdiffcheck, UUID-owned process/topology inventory and cleanup. No full suite or main mutation/release/window task; main owns integrated exact combined suite/CI and release/install. Parent independently dispatches pressure evaluations and final source review.\nblocked-by ht-qhz.19: consumes all leaves (integration sweep)\nblocked-by ht-qhz.5.2: consumes all leaves (integration sweep)\nblocked-by ht-qhz.5.1: consumes all leaves (integration sweep)\nblocked-by ht-qhz.4.3: consumes all leaves (integration sweep)\nblocked-by ht-qhz.4.2: consumes all leaves (integration sweep)\nblocked-by ht-qhz.4.1: consumes all leaves (integration sweep)\nblocked-by ht-qhz.2.3: consumes all leaves (integration sweep)\nblocked-by ht-qhz.2.2: consumes all leaves (integration sweep)\nblocked-by ht-qhz.2.1: consumes all leaves (integration sweep)\nblocked-by ht-qhz.9: consumes all leaves (integration sweep)\nblocked-by ht-qhz.8: consumes all leaves (integration sweep)\nblocked-by ht-qhz.7: consumes all leaves (integration sweep)\nblocked-by ht-qhz.6: consumes all leaves (integration sweep)\nblocked-by ht-qhz.3: consumes all leaves (integration sweep)\nblocked-by ht-qhz.1: consumes all leaves (integration sweep)",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:12:58Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:12:58Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.4.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:03Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:09Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.9",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:06Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.19",
        "type": "blocks",
        "created_at": "2026-10-08T06:12:59Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T06:12:58Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.5.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:00Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.2.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:04Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.4.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:02Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.8",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:06Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.5.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:12:59Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.7",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:07Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.2.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:05Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.6",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:08Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.4.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:01Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.2.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:03Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.20",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:13:10Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 15,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.19",
    "title": "Seam integration: live-effect authority guards",
    "description": "Verify shared live-effect authority guard wiring across canonical reservation, native submission, attachment, downstream delivery and human recovery.\nowns: cross-effect authority-guard integration tests;\nconsumes: canonical bootstrap transitions/attachment, native submission progress, downstream replay, delivery execution and administrative recovery dispatch.\nFiles: tests/handoff_topology_authority.rs or combined module; no new guard policy.\nAcceptance: deterministic RED/GREEN invalidate caller/attempt/host incarnation between earlier preparation and final pre-submit boundary, all effects refuse under current canonical guards; no static client claim becomes authority; completed cleanup bypasses only currentlive checks after original actor classification; operator incorrectquiescence accepted limit honestly asserted, no distributed atomicity claim.\nblocked-by ht-qhz.2.2: consumes canonical bootstrap state transitions\nblocked-by ht-qhz.2.3: consumes canonical bootstrap attachment\nblocked-by ht-qhz.4.2: consumes BootstrapPlan live resume\nblocked-by ht-qhz.4.3: consumes bootstrap downstream completion\nblocked-by ht-qhz.5.1: consumes DeliveryPlan staged-work execution\nblocked-by ht-qhz.6: consumes administrative recovery dispatch",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:09:16Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:09:16Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.19",
        "depends_on_id": "ht-qhz.2.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:18Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.19",
        "depends_on_id": "ht-qhz.2.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:17Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.19",
        "depends_on_id": "ht-qhz.4.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:19Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.19",
        "depends_on_id": "ht-qhz.4.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:18Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.19",
        "depends_on_id": "ht-qhz.5.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:20Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.19",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T06:09:16Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.19",
        "depends_on_id": "ht-qhz.6",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:21Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 6,
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.5.2",
    "title": "Delivery retry and completed presentation cleanup",
    "description": "Integrate journal retry and terminal reporting for existing-peer durable delivery.\nowns: DeliveryPlan retry and absorbing terminal presentation;\nconsumes: DeliveryPlan staged-work execution.\nFiles: src/cli/handoff_delivery.rs (retry/report), src/cli/retry.rs.\nAcceptance: RED/GREEN changedcurrentbinding/archive/restart completion cleanuponly, failedflush/removal retry, exactorigin classifier before all paths, zero duplicate invitation/message or nativeeffects; retained report corruption refuses.\n\nblocked-by ht-qhz.5.1: consumes DeliveryPlan staged-work execution",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:01:23Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:01:39Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.5.2",
        "depends_on_id": "ht-qhz.5",
        "type": "parent-child",
        "created_at": "2026-10-08T06:01:23Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.5.2",
        "depends_on_id": "ht-qhz.5.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:40Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-qhz.5"
  },
  {
    "id": "ht-qhz.5.1",
    "title": "Exact existing recipient and durable staged-work delivery",
    "description": "Implement --existing selector grammar, canonical seat lookup and keyed shared staging with immutable DeliveryPlan.\nowns: DeliveryPlan staged-work execution and existingpeer selector resolution;\nowns: delivery canonical guards; no-launch/invitation-episode tests.\nconsumes: topology/delivery boundary contract and existing canonical Handoff fences.\nFiles: src/cli/handoff_delivery.rs, src/cli/commands.rs (existing mode), src/cli/handoff.rs (shared staging extraction).\nAcceptance: actualCLI RED/GREEN no allocation/rebind/start/preflight/registration, held/unresolved/retired/foreign refusal, canonical joined/pendinginvited/unboundreporting, exact invitation episode reuse and send children; original Handoff serialization/golden unchanged.\nblocked-by ht-qhz.1: consumes boundary contract\nboundary contract: ht-qhz.1",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:01:21Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:33Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.5.1",
        "depends_on_id": "ht-qhz.5",
        "type": "parent-child",
        "created_at": "2026-10-08T06:01:21Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.5.1",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:22Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 3,
    "comment_count": 0,
    "parent": "ht-qhz.5"
  },
  {
    "id": "ht-qhz.4.3",
    "title": "Downstream launch and terminal bootstrap replay",
    "description": "Compose exact existing Handoff downstream with persistent possible-start fence and bootstrap completion/report cleanup.\nowns: bootstrap downstream completion and absorbing terminal replay;\nowns: downstream canonical guard enforcement; report/flush/removal replay tests.\nconsumes: BootstrapPlan live resume and canonical bootstrap attachment.\nFiles: src/cli/topology_handoff.rs (downstream/terminal), src/cli/retry.rs, src/cli/handoff.rs (existing compound helper only).\nAcceptance: RED/GREEN no duplicate create/invite/message/launch on failed flush/removal/restart/archive/binding change; frozen terminal report required; completed replay only presents/removes owned intent, wrongnamespace refuses; launch possible-start preserved.\n\nblocked-by ht-qhz.4.2: consumes BootstrapPlan live resume\nblocked-by ht-qhz.2.3: consumes canonical bootstrap attachment\nboundary contract: ht-qhz.1\nblocked-by ht-qhz.1: consumes boundary contract",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:01:20Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:33Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.4.3",
        "depends_on_id": "ht-qhz.2.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:38Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.3",
        "depends_on_id": "ht-qhz.4.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:36Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.3",
        "depends_on_id": "ht-qhz.4",
        "type": "parent-child",
        "created_at": "2026-10-08T06:01:20Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.3",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:10Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 3,
    "dependent_count": 3,
    "comment_count": 0,
    "parent": "ht-qhz.4"
  },
  {
    "id": "ht-qhz.4.2",
    "title": "At-most-one native creation and exact seat attachment",
    "description": "Implement operation-lock/reserve/local-progress/native-call/evidence/guarded-resolution path.\nowns: BootstrapPlan live resume and native submission progress;\nowns: final canonical guard enforcement; crash/concurrency/loss/seat-guard tests.\nconsumes: frozen bootstrap publication, canonical bootstrap state transitions, canonical bootstrap attachment and typed CreatedTab/submission outcomes.\nFiles: src/cli/topology_handoff.rs (live coordinator), src/cli/retry.rs.\nAcceptance: RED/GREEN successful create, lost reserve/host reply/save, crash/concurrentcopiedjournal, local/canonical recovery, incarnation/restorehold exactrecipient guards and deterministic resolution; never close user tab or auto adopt label; reports retain exact IDs/attempts.\n\nblocked-by ht-qhz.4.1: consumes frozen bootstrap publication\nblocked-by ht-qhz.2.2: consumes canonical bootstrap state transitions\nblocked-by ht-qhz.2.3: consumes canonical bootstrap attachment\nblocked-by ht-qhz.3: consumes typed CreatedTab/submission outcomes\nSession promotion override: fresh reviewer counted six sequencing steps, but canonical transitions and typed native transport are separately owned predecessors. This leaf is one atomic live coordinator deliverable consuming those APIs, with no unresolved design decision or independently reviewable split beyond already-separated publication and terminal handling. Kept LEAF to satisfy cohesion/floor; not size denial.\nboundary contract: ht-qhz.1\nblocked-by ht-qhz.1: consumes boundary contract",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:01:08Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:34Z",
    "labels": [
      "sp:demoted-by-session",
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.4.2",
        "depends_on_id": "ht-qhz.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:34Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.2",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:08Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.2",
        "depends_on_id": "ht-qhz.4.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:29Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.2",
        "depends_on_id": "ht-qhz.2.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:33Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.2",
        "depends_on_id": "ht-qhz.4",
        "type": "parent-child",
        "created_at": "2026-10-08T06:01:08Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.2",
        "depends_on_id": "ht-qhz.2.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:31Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 5,
    "dependent_count": 3,
    "comment_count": 0,
    "parent": "ht-qhz.4"
  },
  {
    "id": "ht-qhz.4.1",
    "title": "New-tab grammar and immutable publication",
    "description": "Implement strict target/channel grammar and additive BootstrapPlan publication.\nowns: frozen bootstrap publication and normalized new-tab grammar;\nconsumes: topology boundary contract.\nFiles: src/cli/commands.rs (new-tab arguments), src/cli/topology_handoff.rs (publication), src/cli/mod.rs.\nAcceptance: actual CLI parse-conflict RED/GREEN; absolute existing cwd, livecaller workspace vs focus, exact state/socket/options/child keys; no manual nativecreate or effects during parse; historical pane grammar unchanged.\nblocked-by ht-qhz.1: consumes boundary contract",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:01:04Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:01:04Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.4.1",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:05Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.4.1",
        "depends_on_id": "ht-qhz.4",
        "type": "parent-child",
        "created_at": "2026-10-08T06:01:04Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-qhz.4"
  },
  {
    "id": "ht-qhz.2.3",
    "title": "Atomic recipient/thread attachment and absorbing completion",
    "description": "Implement cross-fence exact resolution/recipient/downstream attachment and bootstrap terminal state.\nowns: canonical bootstrap attachment, archival protection and absorbing completion;\nowns: canonical attachment/completion guards; atomic/race/terminal tests.\nconsumes: canonical bootstrap persistence and exact identity lookup.\nFiles: src/store/topology_handoff.rs (attachment module if split), src/store/control.rs, src/store/archival.rs.\nAcceptance: existing-thread protected before topology; create-thread attaches in deciding create transaction to both fences; deterministic resolve key/result; contradictory attach refuses; completed historical presentation bypasses only live guards, no effects or revived protection; delayed importer terminal precedence; RED/GREEN.\n\nblocked-by ht-qhz.2.1: consumes canonical bootstrap persistence and exact identity lookup\nboundary contract: ht-qhz.1\nblocked-by ht-qhz.1: consumes boundary contract",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:01:03Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:34Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.2.3",
        "depends_on_id": "ht-qhz.2",
        "type": "parent-child",
        "created_at": "2026-10-08T06:01:03Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.2.3",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:06Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.2.3",
        "depends_on_id": "ht-qhz.2.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:27Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 2,
    "dependent_count": 5,
    "comment_count": 0,
    "parent": "ht-qhz.2"
  },
  {
    "id": "ht-qhz.2.2",
    "title": "One-use creation attempts and operator recovery decisions",
    "description": "Implement canonical reserve/evidence/recovery state transitions in deciding transactions.\nowns: canonical bootstrap state transitions, one-use submission authorization and administrative recovery decisions;\nowns: final pre-submit canonical guards; concurrent/lost-reserve/recovery tests.\nconsumes: canonical bootstrap persistence and exact identity lookup.\nFiles: src/store/topology_handoff.rs (attempt transition module if split), tests/store/topology_attempts.rs.\nAcceptance: concurrent/lost-reserve reply yields at most one authorization; exact same-attempt evidence replay; proven NotSubmitted vs unknown; operator created-pane/not-created replay/contradiction and late older creation conflict; live A2 checks and completed recovery cannot reactivate; meaningful RED/GREEN.\n\nblocked-by ht-qhz.2.1: consumes canonical bootstrap persistence and exact identity lookup\nboundary contract: ht-qhz.1\nblocked-by ht-qhz.1: consumes boundary contract",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:00:19Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:35Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.2.2",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:04Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.2.2",
        "depends_on_id": "ht-qhz.2",
        "type": "parent-child",
        "created_at": "2026-10-08T06:00:19Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.2.2",
        "depends_on_id": "ht-qhz.2.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:25Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 2,
    "dependent_count": 4,
    "comment_count": 0,
    "parent": "ht-qhz.2"
  },
  {
    "id": "ht-qhz.2.1",
    "title": "Schema and exact bootstrap identity",
    "description": "Land minimal additive0028 schema, unique immutable namespace/keys and bootstrap current-state lookup with inert command wiring.\nowns: canonical bootstrap persistence and exact identity lookup;\nconsumes: topology boundary contract.\nFiles: migrations/0028_handoff_topology.sql, src/store/topology_handoff.rs, src/store/schema.rs, src/store/mod.rs.\nAcceptance: actual reviewed0026/0027 predecessors absorbed before migration edit; no placeholders; migration/upgrade/tamper and exact payload/namespace mismatch RED/GREEN; historical bytes and old Handoff shapes unchanged; bounded indexed lookups.\nblocked-by ht-qhz.1: consumes boundary contract\nExternal source gate: main approved ONE0028 via mp6wXQY6o; absorb actual reviewed lazy0026/adapters0027 before edits, no placeholders. Only this leaf is parked while unavailable; independent ready work proceeds.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T06:00:13Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:01:48Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.2.1",
        "depends_on_id": "ht-qhz.2",
        "type": "parent-child",
        "created_at": "2026-10-08T06:00:13Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.2.1",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:00:15Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 3,
    "comment_count": 0,
    "parent": "ht-qhz.2"
  },
  {
    "id": "ht-qhz.9",
    "title": "Configuration smoke: pane-launch, new-tab and existing-peer",
    "description": "Actually exercise all three command modes with explicit pinned routing and both Claude/Codex option payloads using owned model-free helpers.\nowns: mode matrix actual CLI fixtures and source-bound RED/GREEN smoke evidence;\nowns: cross-component failure tests; owned topology/child teardown; UUID-scoped leak evidence.\nconsumes: BootstrapPlan resume and DeliveryPlan execution.\nFiles: tests/handoff_topology_cli.rs or combined module, tests/combined.rs only if needed, run smoke evidence.\nAcceptance: new-tab creates exactly one tab, legacy launch uses exact pane, existing-peer invokes zero start; retries do not duplicate topology/invites/messages/launch; isolated HOME/config dirs, test_support owned children stopped/reaped and UUID-scoped cleanup; no per-tab full suite.\nblocked-by ht-qhz.4.3: consumes bootstrap downstream completion and terminal replay\nblocked-by ht-qhz.5.2: consumes DeliveryPlan retry and terminal presentation",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:37Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:35Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.9",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:37Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.9",
        "depends_on_id": "ht-qhz.4.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:45Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.9",
        "depends_on_id": "ht-qhz.5.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:47Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 2,
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.8",
    "title": "Handoff-first operation guidance and CLI help",
    "description": "Replace manual new-tab bootstrap recipes with practical handoff-first routing and preserve invitation relevance/ACK semantics.\nowns: integrations/skill/SKILL.md operation priorities, handoff help and pressure scenario inputs;\nconsumes: approved topology/delivery/recovery grammar boundary contract.\nFiles: integrations/skill/SKILL.md, src/cli/handoff.rs (help only), docs/cli.md or existing handoff doc after source audit, run pressure-scenarios.md.\nAcceptance: FIRSThandoff SECONDverifiedtopic-goal+participant send THIRDjoin/invite, ambiguous membership surfaces, existing peer no relaunch, human recovery argv honest, no installed-permission claim before peer approval; meaningful BEFORE/AFTER pressure evaluation is parent orchestrator-owned rather than implementer spawning agents.\nblocked-by ht-qhz.1: consumes boundary contract\nPressure ownership contract: retained BEFORE says missing tab capability and is coupled product+guide baseline; parent new control/candidate arms receive identical truthful current help,5+replicates when isolating wording effect; never invent RED behavior absent from baseline. Implementer supplies scenarios/candidateguide only.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:35Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:13:11Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.8",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:35Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.8",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T05:55:36Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.7",
    "title": "Bootstrap and delivery legacy archival compatibility",
    "description": "Extend bounded read-only importer for new identities/progress and exact namespace protection without granting topology submission or caller authority.\nowns: bootstrap/delivery legacy veto and terminal precedence integration;\nowns: exact namespace/delayed-scan/restart terminal-veto tests.\nconsumes: canonical bootstrap state transitions and new journal boundary contract.\nFiles: src/archival_legacy.rs, src/store/archival.rs, tests/archival*.rs.\nAcceptance: RED/GREEN custom namespace/copiedUUID, malformed/inaccessible/oversized/symlink/partial scans, delayed scan after completed bootstrap, restart retained journal, atomic created-thread protection, delivery completed replay; zero old journal rewriting or revived fence.\nblocked-by ht-qhz.2.3: consumes canonical bootstrap attachment and absorbing completion",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:34Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:36Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.7",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:34Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.7",
        "depends_on_id": "ht-qhz.2.3",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:44Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.6",
    "title": "Explicit human uncertain-topology recovery",
    "description": "Integrate human handoff recover through the approved peer InvocationActor/classify_original_actor boundary with separate operator scope and exact original bootstrap/attempt references.\nowns: administrative recovery dispatch and operator audit provenance;\nowns: recovery canonical guards; contradiction/quiescence-claim tests.\nconsumes: canonical bootstrap state transitions and peer human namespace actor classifier.\nFiles: src/cli/topology_recover.rs, src/cli/commands.rs (peer-approved human handoff recovery integration), src/cli/retry.rs, TRUST-POLICY.md.\nAcceptance: RED/GREEN root refusal before ANY completed cleanup, created-pane ordinary structural/seat guards, not-created next-attempt authorization, contradictory/concurrent/repeated assertions, delayed prior creation conflict, completed absorbing refusal; never reclassifies frozen origin or performs downstream actions; document mistaken not-created accepted limit. Exact human integration is gated only on named peer source availability; no root --operator fallback.\nblocked-by ht-qhz.2.2: consumes canonical bootstrap state transitions\nboundary contract: ht-qhz.1\nblocked-by ht-qhz.1: consumes boundary contract\n--not-created is the explicit cooperative assertion both inspected noncreation and original attempt quiescence/cannot still create. Refuse known in-flight normaloperationlock; never infer quiescence from PID/labels/current snapshot or kill unowned process. Incorrect assertion can duplicate; record accepted limit.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:32Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:36Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.6",
        "depends_on_id": "ht-qhz.2.2",
        "type": "blocks",
        "created_at": "2026-10-08T06:01:42Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.6",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T06:09:13Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.6",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:32Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 2,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.5",
    "title": "Existing-peer delivery without native launch",
    "description": "Implement HandoffDelivery immutable journal and exact staged create/invite/send composition without launch preflight, native start or registration.\nowns: DeliveryPlan execution and --existing grammar;\nconsumes: topology/delivery boundary contract and existing canonical Handoff fences.\nFiles: src/cli/handoff_delivery.rs, src/cli/handoff.rs (extract staged-work helper preserving serialization), src/cli/commands.rs (existing selector parser), src/cli/retry.rs.\nAcceptance: actual CLI RED/GREEN pane/seat modes, joined/invited/unbound recipient reporting, zero native launches, no seat allocation/rebind, exact keyed invite/send replay including reinvitation episode safety, canonical held/unresolved/retired refusal, historical Handoff golden unchanged.",
    "design": "docs/superpowers/runs/2026-10-07-handoff-topology/2026-10-07-handoff-topology--existing-delivery-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:30Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:01:24Z",
    "metadata": {
      "sp_depth": 1,
      "sp_order": 3
    },
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.5",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:30Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.4",
    "title": "New-tab durable CLI bootstrap coordinator",
    "description": "Own new-tab durable BootstrapPlan publication, live submission and terminal replay.\nowns: BootstrapPlan publication/resume and new-tab dispatch;\nconsumes: canonical bootstrap state transitions and typed CreatedTab/submission outcomes.\nFiles: src/cli/topology_handoff.rs, src/cli/commands.rs (new-tab parser only), src/cli/mod.rs, src/cli/retry.rs.\nAcceptance: meaningful actual CLI RED/GREEN new-tab success/retry, concurrent invocation, reserve/host reply loss, save/output/removal failure; zero duplicated creates; exact state/socket/argv/options, restore holds/incarnation refusal; fence existing thread before create; never closes user topology; old pane handoff unchanged.",
    "design": "docs/superpowers/runs/2026-10-07-handoff-topology/2026-10-07-handoff-topology--bootstrap-coordinator-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:28Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:01Z",
    "metadata": {
      "sp_depth": 1,
      "sp_order": 2
    },
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.4",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:28Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.3",
    "title": "Witnessed bounded native tab creation",
    "description": "Implement the exact audited tab.create socket request/result mapping under the existing host witness and budget machinery.\nowns: typed CreatedTab and submission outcome production boundary;\nowns: final native witness recheck; correlated response-loss tests.\nconsumes: topology boundary contract and adapter-owner launch/registry seam.\nFiles: src/host/transport.rs, src/host/native.rs, src/host/mod.rs.\nAcceptance: recheck installed protocol/schema model-free; strict correlation, coherent tab/root_pane/workspace/incarnation; refusal before submission, cancellation/lost/malformed reply outcome_unknown; deterministic fake socket RED/GREEN; one owned isolated native fixture if available with read-only schema fallback documented, no real agent start/config write.\nblocked-by ht-qhz.1: consumes boundary contract",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:26Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:36Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.3",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:26Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-qhz.3",
        "depends_on_id": "ht-qhz.1",
        "type": "blocks",
        "created_at": "2026-10-08T05:55:27Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 1,
    "dependent_count": 2,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.2",
    "title": "Canonical bootstrap attempt fences and atomic attachment",
    "description": "Implement canonical exact namespace identity, one-use Reserve authorization, durable creation evidence/attempt decisions and absorbing completion with indexed archival protection.\nowns: canonical bootstrap state transitions and submission permission;\nconsumes: topology boundary contract.\nFiles: src/store/topology_handoff.rs, src/store/mod.rs, src/store/control.rs, src/store/archival.rs, src/store/schema.rs, migrations/0028_handoff_topology.sql (number only after main allocation).\nAcceptance: meaningful RED/GREEN exact payload mismatch, concurrent reserve one permission, lost reserve replay no permission, creation attachment in deciding transaction, current A2 guards, completed current-state presentation and stale recovery conflict tests; one additive migration only; old migration bytes unchanged. Bound unique rows/indexes and replay statements.\nExternal source gate: main approved ONE additive topology migration0028 via mp6wXQY6o. This leaf must absorb actual reviewed migrations0026lazy and0027adapters before editing/applying0028; no fabricated placeholder predecessors or historical-byte changes. Parent scheduler parks only this leaf while actual predecessors are unavailable.",
    "design": "docs/superpowers/runs/2026-10-07-handoff-topology/2026-10-07-handoff-topology--canonical-bootstrap-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:25Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:01:03Z",
    "metadata": {
      "sp_depth": 1,
      "sp_order": 1
    },
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.2",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:25Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz.1",
    "title": "Seam contract: topology bootstrap and delivery identities",
    "description": "Land only compilable additive BootstrapPlan/DeliveryPlan, state/result types and inert protocol/CLI/journal dispatch wiring without changing old Handoff serialization.\nowns: versioned topology and delivery boundary contract;\nowns: guard inputs/results wired end-to-end inert-by-default.\nconsumes: existing CallerClaim, IntentScope, HandoffIdentity and peer actor-route/classify_original_actor seam.\nFiles: src/protocol/handoff.rs, src/protocol/commands.rs, src/protocol/results.rs, src/cli/journal.rs, src/cli/commands.rs, src/ports.rs, src/cli/mod.rs.\nAcceptance: inert-by-default compiles, original actor classification runs before new journal retry, exact historical journal/digest golden unchanged; no topology effect or speculated human shortcut.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:55:24Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T06:10:37Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependencies": [
      {
        "issue_id": "ht-qhz.1",
        "depends_on_id": "ht-qhz",
        "type": "parent-child",
        "created_at": "2026-10-08T05:55:24Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 11,
    "comment_count": 0,
    "parent": "ht-qhz"
  },
  {
    "id": "ht-qhz",
    "title": "Native topology handoff and existing-peer delivery",
    "description": "Deliver approved native new-tab handoff, no-relaunch peer delivery and exact durable topology recovery; closable at reviewed merge-ready source. Spec docs/superpowers/runs/2026-10-07-handoff-topology/2026-10-07-handoff-topology-design.md. Release and main window are external follow-on.",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-08T05:54:22Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-08T05:54:22Z",
    "labels": [
      "sp:ht-qhz"
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0
  }
]
