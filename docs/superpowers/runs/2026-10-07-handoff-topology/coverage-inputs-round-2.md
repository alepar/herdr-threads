## Goals

### ht-qhz (root)
An agent can hand off work to a newly created Herdr tab through one durable command, or deliver work to an existing peer without restarting it. Exact routing, canonical seat authority, explicit channel choice and conservative replay prevent duplicate topology, invitations, messages and native launches across failures.

Status: user-approved scope; autonomous Mode B refinement and design roast enabled. Source prerequisite 8106f5cade8ac9d4c2dd8e9b3281e8df05abd8ab remains frozen. This run's branch is super-auto/handoff-topology; integration/release follows the external main window, not an implementation bead.

### ht-qhz.2 — Canonical bootstrap attempt fences and atomic attachment
Canonical exact identity/attempt transitions/atomic attachment remain durable and absorbing across concurrent replay and response loss.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.2. Autonomous Mode B.
summary: Implement canonical exact namespace identity, one-use Reserve authorization, durable creation evidence/attempt decisions and absorbing completion with indexed a

### ht-qhz.4 — New-tab durable CLI bootstrap coordinator
A new-tab command freezes exact intent and performs at-most-one native creation per canonical attempt, then existing guarded launch and terminal replay.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.4. Autonomous Mode B.
summary: Own new-tab durable BootstrapPlan publication, live submission and terminal replay.

### ht-qhz.5 — Existing-peer delivery without native launch
An existing canonical peer receives exact staged work and replayable completion without native launch or identity replacement.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.5. Autonomous Mode B.
summary: Implement HandoffDelivery immutable journal and exact staged create/invite/send composition without launch preflight, native start or registration.

## Task tree

- ht-qhz · Native topology handoff and existing-peer delivery (epic) · Deliver approved native new-tab handoff, no-relaunch peer delivery and exact durable topology recovery; closable at reviewed merge-ready source. · deps: none
  - ht-qhz.1 · Seam contract: topology bootstrap and delivery identities · Land only compilable additive BootstrapPlan/DeliveryPlan, state/result types and inert protocol/CLI/journal dispatch wiring without changing old Handoff seriali · deps: none
      owns: versioned topology and delivery boundary contract;
      owns: guard inputs/results wired end-to-end inert-by-default.
      consumes: existing CallerClaim, IntentScope, HandoffIdentity and peer actor-route/classify_original_actor seam.
  - ht-qhz.2 · Canonical bootstrap attempt fences and atomic attachment (epic) · Implement canonical exact namespace identity, one-use Reserve authorization, durable creation evidence/attempt decisions and absorbing completion with indexed a · deps: none
      owns: canonical bootstrap state transitions and submission permission;
      consumes: topology boundary contract.
    - ht-qhz.2.1 · Schema and exact bootstrap identity · Land minimal additive0028 schema, unique immutable namespace/keys and bootstrap current-state lookup with inert command wiring. · deps: ht-qhz.1
        owns: canonical bootstrap persistence and exact identity lookup;
        consumes: topology boundary contract.
    - ht-qhz.2.2 · One-use creation attempts and operator recovery decisions · Implement canonical reserve/evidence/recovery state transitions in deciding transactions. · deps: ht-qhz.1, ht-qhz.2.1
        owns: canonical bootstrap state transitions, one-use submission authorization and administrative recovery decisions;
        owns: final pre-submit canonical guards; concurrent/lost-reserve/recovery tests.
        consumes: canonical bootstrap persistence and exact identity lookup.
        boundary contract: ht-qhz.1
    - ht-qhz.2.3 · Atomic recipient/thread attachment and absorbing completion · Implement cross-fence exact resolution/recipient/downstream attachment and bootstrap terminal state. · deps: ht-qhz.1, ht-qhz.2.1
        owns: canonical bootstrap attachment, archival protection and absorbing completion;
        owns: canonical attachment/completion guards; atomic/race/terminal tests.
        consumes: canonical bootstrap persistence and exact identity lookup.
        boundary contract: ht-qhz.1
  - ht-qhz.3 · Witnessed bounded native tab creation · Implement the exact audited tab.create socket request/result mapping under the existing host witness and budget machinery. · deps: ht-qhz.1
      owns: typed CreatedTab and submission outcome production boundary;
      owns: final native witness recheck; correlated response-loss tests.
      consumes: topology boundary contract and adapter-owner launch/registry seam.
  - ht-qhz.4 · New-tab durable CLI bootstrap coordinator (epic) · Own new-tab durable BootstrapPlan publication, live submission and terminal replay. · deps: none
      owns: BootstrapPlan publication/resume and new-tab dispatch;
      consumes: canonical bootstrap state transitions and typed CreatedTab/submission outcomes.
    - ht-qhz.4.1 · New-tab grammar and immutable publication · Implement strict target/channel grammar and additive BootstrapPlan publication. · deps: ht-qhz.1
        owns: frozen bootstrap publication and normalized new-tab grammar;
        consumes: topology boundary contract.
    - ht-qhz.4.2 · At-most-one native creation and exact seat attachment · Implement operation-lock/reserve/local-progress/native-call/evidence/guarded-resolution path. · deps: ht-qhz.1, ht-qhz.2.2, ht-qhz.2.3, ht-qhz.3, ht-qhz.4.1
        owns: BootstrapPlan live resume and native submission progress;
        owns: final canonical guard enforcement; crash/concurrency/loss/seat-guard tests.
        consumes: frozen bootstrap publication, canonical bootstrap state transitions, canonical bootstrap attachment and typed CreatedTab/submission outcomes.
        boundary contract: ht-qhz.1
    - ht-qhz.4.3 · Downstream launch and terminal bootstrap replay · Compose exact existing Handoff downstream with persistent possible-start fence and bootstrap completion/report cleanup. · deps: ht-qhz.1, ht-qhz.2.3, ht-qhz.4.2
        owns: bootstrap downstream completion and absorbing terminal replay;
        owns: downstream canonical guard enforcement; report/flush/removal replay tests.
        consumes: BootstrapPlan live resume and canonical bootstrap attachment.
        boundary contract: ht-qhz.1
  - ht-qhz.5 · Existing-peer delivery without native launch (epic) · Implement HandoffDelivery immutable journal and exact staged create/invite/send composition without launch preflight, native start or registration. · deps: none
      owns: DeliveryPlan execution and --existing grammar;
      consumes: topology/delivery boundary contract and existing canonical Handoff fences.
    - ht-qhz.5.1 · Exact existing recipient and durable staged-work delivery · Implement --existing selector grammar, canonical seat lookup and keyed shared staging with immutable DeliveryPlan. · deps: ht-qhz.1
        owns: DeliveryPlan staged-work execution and existingpeer selector resolution;
        owns: delivery canonical guards; no-launch/invitation-episode tests.
        consumes: topology/delivery boundary contract and existing canonical Handoff fences.
        boundary contract: ht-qhz.1
    - ht-qhz.5.2 · Delivery retry and completed presentation cleanup · Integrate journal retry and terminal reporting for existing-peer durable delivery. · deps: ht-qhz.5.1
        owns: DeliveryPlan retry and absorbing terminal presentation;
        consumes: DeliveryPlan staged-work execution.
  - ht-qhz.6 · Explicit human uncertain-topology recovery · Integrate human handoff recover through the approved peer InvocationActor/classify_original_actor boundary with separate operator scope and exact original boots · deps: ht-qhz.1, ht-qhz.2.2
      owns: administrative recovery dispatch and operator audit provenance;
      owns: recovery canonical guards; contradiction/quiescence-claim tests.
      consumes: canonical bootstrap state transitions and peer human namespace actor classifier.
      boundary contract: ht-qhz.1
  - ht-qhz.7 · Bootstrap and delivery legacy archival compatibility · Extend bounded read-only importer for new identities/progress and exact namespace protection without granting topology submission or caller authority. · deps: ht-qhz.2.3
      owns: bootstrap/delivery legacy veto and terminal precedence integration;
      owns: exact namespace/delayed-scan/restart terminal-veto tests.
      consumes: canonical bootstrap state transitions and new journal boundary contract.
  - ht-qhz.8 · Handoff-first operation guidance and CLI help · Replace manual new-tab bootstrap recipes with practical handoff-first routing and preserve invitation relevance/ACK semantics. · deps: ht-qhz.1
      owns: integrations/skill/SKILL.md operation priorities, handoff help and pressure scenario inputs;
      consumes: approved topology/delivery/recovery grammar boundary contract.
  - ht-qhz.9 · Configuration smoke: pane-launch, new-tab and existing-peer · Actually exercise all three command modes with explicit pinned routing and both Claude/Codex option payloads using owned model-free helpers. · deps: ht-qhz.4.3, ht-qhz.5.2
      owns: mode matrix actual CLI fixtures and source-bound RED/GREEN smoke evidence;
      owns: cross-component failure tests; owned topology/child teardown; UUID-scoped leak evidence.
      consumes: BootstrapPlan resume and DeliveryPlan execution.
  - ht-qhz.19 · Seam integration: live-effect authority guards · Verify shared live-effect authority guard wiring across canonical reservation, native submission, attachment, downstream delivery and human recovery. · deps: ht-qhz.2.2, ht-qhz.2.3, ht-qhz.4.2, ht-qhz.4.3, ht-qhz.5.1, ht-qhz.6
      owns: cross-effect authority-guard integration tests;
      consumes: canonical bootstrap transitions/attachment, native submission progress, downstream replay, delivery execution and administrative recovery dispatch.
