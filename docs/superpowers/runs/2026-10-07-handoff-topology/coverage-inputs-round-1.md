## Goals

### ht-qhz (root)
An agent can hand off work to a newly created Herdr tab through one durable command, or deliver work to an existing peer without restarting it. Exact routing, canonical seat authority, explicit channel choice and conservative replay prevent duplicate topology, invitations, messages and native launches across failures.

Status: user-approved scope; autonomous Mode B refinement and design roast enabled. Source prerequisite 8106f5cade8ac9d4c2dd8e9b3281e8df05abd8ab remains frozen. This run's branch is super-auto/handoff-topology; integration/release follows the external main window, not an implementation bead.

## Task tree

- ht-qhz · Native topology handoff and existing-peer delivery (epic) · Deliver approved native new-tab handoff, no-relaunch peer delivery and exact durable topology recovery; closable at reviewed merge-ready source. · deps: none
  - ht-qhz.1 · Seam contract: topology bootstrap and delivery identities · Land only compilable additive BootstrapPlan/DeliveryPlan, state/result types and inert protocol/CLI/journal dispatch wiring without changing old Handoff seriali · deps: none
      owns: versioned topology and delivery boundary contract;
      consumes: existing CallerClaim, IntentScope, HandoffIdentity and peer actor-route/classify_original_actor seam.
  - ht-qhz.2 · Canonical bootstrap attempt fences and atomic attachment · Implement canonical exact namespace identity, one-use Reserve authorization, durable creation evidence/attempt decisions and absorbing completion with indexed a · deps: ht-qhz.1
      owns: canonical bootstrap state transitions and submission permission;
      consumes: topology boundary contract.
  - ht-qhz.3 · Witnessed bounded native tab creation · Implement the exact audited tab.create socket request/result mapping under the existing host witness and budget machinery. · deps: ht-qhz.1
      owns: typed CreatedTab and submission outcome production boundary;
      consumes: topology boundary contract and adapter-owner launch/registry seam.
  - ht-qhz.4 · New-tab durable CLI bootstrap coordinator · Implement new-tab grammar, frozen normalization, operation lock, canonical reserve before local possible-creation/native submission, exact CreatedTab recording  · deps: ht-qhz.2, ht-qhz.3
      owns: BootstrapPlan publication/resume and new-tab dispatch;
      consumes: canonical bootstrap state transitions and typed CreatedTab/submission outcomes.
  - ht-qhz.5 · Existing-peer delivery without native launch · Implement HandoffDelivery immutable journal and exact staged create/invite/send composition without launch preflight, native start or registration. · deps: ht-qhz.1
      owns: DeliveryPlan execution and --existing grammar;
      consumes: topology/delivery boundary contract and existing canonical Handoff fences.
  - ht-qhz.6 · Explicit human uncertain-topology recovery · Integrate human handoff recover through the approved peer InvocationActor/classify_original_actor boundary with separate operator scope and exact original boots · deps: ht-qhz.2
      owns: administrative recovery dispatch and operator audit provenance;
      consumes: canonical bootstrap state transitions and peer human namespace actor classifier.
  - ht-qhz.7 · Bootstrap and delivery legacy archival compatibility · Extend bounded read-only importer for new identities/progress and exact namespace protection without granting topology submission or caller authority. · deps: ht-qhz.2
      owns: bootstrap/delivery legacy veto and terminal precedence integration;
      consumes: canonical bootstrap state transitions and new journal boundary contract.
  - ht-qhz.8 · Handoff-first operation guidance and CLI help · Replace manual new-tab bootstrap recipes with practical handoff-first routing and preserve invitation relevance/ACK semantics. · deps: ht-qhz.1
      owns: integrations/skill/SKILL.md operation priorities, handoff help and pressure scenario inputs;
      consumes: approved topology/delivery/recovery grammar boundary contract.
  - ht-qhz.9 · Configuration smoke: pane-launch, new-tab and existing-peer · Actually exercise all three command modes with explicit pinned routing and both Claude/Codex option payloads using owned model-free helpers. · deps: ht-qhz.4, ht-qhz.5
      owns: mode matrix actual CLI fixtures and source-bound RED/GREEN smoke evidence;
      consumes: BootstrapPlan resume and DeliveryPlan execution.
