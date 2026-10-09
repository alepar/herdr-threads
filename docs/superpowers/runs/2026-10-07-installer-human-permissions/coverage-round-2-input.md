## Goals

### ht-uwd (root)
Installer and setup grant owned native Claude/Codex permissions for ordinary herdr-threads reads and communication writes through bare names and validated installed paths, while every action declaring a person or local operator requires an immediate `human` namespace and remains subject to native approval. Preserve canonical daemon authority and exact immutable retry semantics.

# Installer permissions and explicit human commands

Mode B, autonomous. Authoritative scope: [approved brief](approved-brief.md). Branch `installer-permissions`; baseline `c3b8f3f0`. This is a cooperative safety and attribution boundary, not hostile same-user verification.

### ht-uwd.3 — Runtime honest actor routing and human lifecycle
Root accountable commands refuse inferred Human before effects; human communication and check-in retain honest existing provenance and canonical guards.

# Runtime Actor Routing

Epic: `ht-uwd.3`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.
summary: Propagate route through caller/selection/cooperative/operator paths and me init; root accountable actions refuse inferred Human before intent/context retirement

### ht-uwd.5 — Claude narrow permissions and durable legacy ownership migration
Claude grants positive ordinary forms independently of hooks, with exact recoverable migration of owned historical broad rules and no foreign-policy adoption.

# Claude Permission Migration

Epic: `ht-uwd.5`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.
summary: Implement independent Claude positive-family renderer plus exact lifecycle, decouple hook auto-grant/completeness, durable transfer from historical owned broad/

### ht-uwd.5.3 — Crash-safe Claude permission ownership transfer and lifecycle
Interrupted Claude permission transfers retain exact ownership and resume safely; independent lifecycle never restores broad grants or removes foreign policy.

# Claude Permission Migration  Transfer

Epic: `ht-uwd.5.3`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.
summary: Implement independent install/inspect/remove and durable prepared source/destination transfer from exact historical broad/retired entries, never two live owners

### ht-uwd.7 — Independent setup lifecycle and permission consent
Explicit setup/status/unsetup and requested doctor fix manage an independent permission component; hook ownership never authorizes a missing grant.

# Setup Permission Lifecycle

Epic: `ht-uwd.7`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.
summary: Wire permissions beside hooks/skill for explicit setup/status/unsetup, installer-integrations reconciliation and requested doctor fix.

### ht-uwd.8 — Installer validated owned paths and permission gateway
Installer delivers only its validated owned executable spellings to independently consented native permission setup, with truthful update/removal and gateway evidence.

# Installer Permission Gateway

Epic: `ht-uwd.8`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.
summary: Pass canonical installed binary, owned herdr-threads link and ht alias plus pinned routing into component reconciliation.

## Task tree

- ht-uwd · Installer permissions and explicit human namespace (epic) · Observable goal: owned native Claude/Codex ordinary read/write grants with immediate human/operator exclusion, honest actor routing and immutable retry. · deps: none
  - ht-uwd.1 · Namespace grammar and ordinary command catalog · Implement immediate raw argv[1] human route separate from output; reject legacy person/operator aliases/globals with correct replacement. · deps: none
      owns: InvocationActor route and ordinary command catalog.
  - ht-uwd.2 · Immutable original actor classification and retry preflight · Classify validated original SemanticMutation/frozen CallerClaim/IntentScope, including Native lifecycle and contradictions, before completed handoff shortcut ou · deps: ht-uwd.1
      owns: frozen retry origin classifier and preflight.
      consumes: InvocationActor route.
  - ht-uwd.3 · Runtime honest actor routing and human lifecycle (epic) · Propagate route through caller/selection/cooperative/operator paths and me init; root accountable actions refuse inferred Human before intent/context retirement · deps: ht-uwd.1
      owns: runtime actor boundary.
      consumes: InvocationActor route.
    - ht-uwd.3.1 · Guard accountable caller selection before local effects · Own route validation in derive_caller/derive_selection/run_selected/run_cooperative/run_operator and own-inbox handling. · deps: ht-uwd.1
        owns: runtime actor boundary.
        consumes: InvocationActor route.
    - ht-uwd.3.2 · Human check-in and person communication preserve provenance · Wire human me init/check-in and human communication through existing Human claims and operator semantics, preserve agent-to-human A4 guard unless existing opera · deps: ht-uwd.1, ht-uwd.3.1
        owns: human lifecycle/communication route integration.
        consumes: runtime actor boundary.
  - ht-uwd.4 · Seam contract: owned permission component and validated executable inputs · Deliver compilable inert-by-default permission module/API: bounded validated executable spellings and routing, inspect/plan/install/remove backend types, separa · deps: ht-uwd.1
      owns: PermissionInputs, PermissionPlan/Inspection and component manifest schema.
      consumes: ordinary command catalog.
  - ht-uwd.5 · Claude narrow permissions and durable legacy ownership migration (epic) · Implement independent Claude positive-family renderer plus exact lifecycle, decouple hook auto-grant/completeness, durable transfer from historical owned broad/ · deps: ht-uwd.4
      owns: Claude permission backend and historical hook transfer.
      consumes: PermissionInputs, PermissionPlan/Inspection and component manifest schema; ordinary command catalog.
      boundary contract: ht-uwd.4
    - ht-uwd.5.1 · Claude positive ordinary permission renderer · Generate positive ordinary-family and exact supported pinned/output rule forms, never arbitrary middle wildcard covering human. · deps: ht-uwd.4
        owns: Claude rule renderer and representability contract.
        consumes: ordinary command catalog; PermissionInputs.
        boundary contract: ht-uwd.4
    - ht-uwd.5.2 · Decouple hook completeness and writes from new permissions · Remove broad automatic permission grant/requirement from new Claude hooks install/status while preserving historical OwnedPermission fingerprint/pre_existing/su · deps: ht-uwd.4
        owns: independent hook completeness and legacy ownership evidence.
        consumes: separate permission manifest schema.
        boundary contract: ht-uwd.4
    - ht-uwd.5.3 · Crash-safe Claude permission ownership transfer and lifecycle (epic) · Implement independent install/inspect/remove and durable prepared source/destination transfer from exact historical broad/retired entries, never two live owners · deps: ht-uwd.4, ht-uwd.5.1, ht-uwd.5.2
        owns: Claude lifecycle and transfer state machine.
        consumes: Claude rule renderer and representability contract; independent hook completeness and legacy ownership evidence.
        boundary contract: ht-uwd.4
      - ht-uwd.5.3.1 · Durable exact Claude ownership transfer engine · Implement validated prepared record, exact stage derivation and idempotent resume/publication state machine from historical source to independent destination. · deps: ht-uwd.5.1, ht-uwd.5.2
          owns: prepared ownership transfer engine.
          consumes: Claude rule renderer and legacy hook evidence.
      - ht-uwd.5.3.2 · Independent Claude install status remove with shared JSON safety · Wire backend lifecycle to exact transfer engine and current ownership manifest, refreshed settings plans and representability diagnostic; status separates hooks · deps: ht-uwd.4, ht-uwd.5.3.1
          owns: independent Claude lifecycle.
          consumes: prepared ownership transfer engine.
          boundary contract: ht-uwd.4
  - ht-uwd.6 · Codex owned execpolicy backend · Generate exact executable union allow plus matching immediate human prompt, install/status/remove owned rules file under active CODEX_HOME/rules. · deps: ht-uwd.4
      owns: Codex permission backend.
      consumes: PermissionInputs, PermissionPlan/Inspection and component manifest schema.
      boundary contract: ht-uwd.4
  - ht-uwd.7 · Independent setup lifecycle and permission consent (epic) · Wire permissions beside hooks/skill for explicit setup/status/unsetup, installer-integrations reconciliation and requested doctor fix. · deps: ht-uwd.5, ht-uwd.6
      owns: component lifecycle and consent.
      consumes: Claude permission backend; Codex permission backend; permission component API.
    - ht-uwd.7.1 · Explicit permission setup status unsetup and requested doctor fix · Wire backend component into explicit request/response/status/remove, with active isolated roots and independent component result. · deps: ht-uwd.5, ht-uwd.6
        owns: explicit permission lifecycle API.
        consumes: Claude permission backend; Codex permission backend; component API.
    - ht-uwd.7.2 · Installer component reconciliation and explicit permission consent · Add causal isolated missing/declined/noninteractive/hooks-owned consent fixtures at actual reconciliation entry point. · deps: ht-uwd.4, ht-uwd.7.1
        owns: independent permission consent/reconciliation.
        consumes: explicit permission lifecycle API.
        boundary contract: ht-uwd.4
  - ht-uwd.8 · Installer validated owned paths and permission gateway (epic) · Pass canonical installed binary, owned herdr-threads link and ht alias plus pinned routing into component reconciliation. · deps: ht-uwd.7
      owns: installer executable inventory transport.
      consumes: independent component lifecycle and consent.
    - ht-uwd.8.1 · Validate installer executable inventory at Rust setup consumer · Consume parser-reserved raw paths/routing and validate exact owned canonical/link/alias forms; reject foreign or mismatched ownership and ambiguous controls; di · deps: ht-uwd.4, ht-uwd.7
        owns: installer inventory validation/transport consumer.
        consumes: PermissionInputs and permission component lifecycle.
        boundary contract: ht-uwd.4
    - ht-uwd.8.2 · Pass owned installer paths and consent through shell gateway · Exercise actual shell gateway to Rust consumer with isolated ownership/refusal/consent regressions, then implement owned path transport and truthful summaries. · deps: ht-uwd.7, ht-uwd.8.1
        owns: shell inventory gateway and permission consent wording.
        consumes: installer inventory validation/transport consumer.
  - ht-uwd.9 · Human command guidance and immutable continuation rendering · Update owned guidance, generated pending/retry/ready/continuation commands and TRUST-POLICY spelling/native limits. · deps: ht-uwd.2, ht-uwd.3
      owns: lawful namespace suggestions and documentation.
      consumes: InvocationActor route; frozen retry origin classifier; runtime actor boundary.
  - ht-uwd.10 · Configuration smoke: Claude/Codex, bare/ht/absolute/pinned, root/human · Run early isolated parse_argv and Claude/Codex renderer decisions for each bare/ht/canonical/link/alias/pinned, root/human, output/quoted-body/compound configur · deps: ht-uwd.3.1, ht-uwd.5.1, ht-uwd.6
      owns: cross-configuration focused smoke fixtures.
      consumes: Claude rule renderer and representability contract; Codex permission backend; namespace grammar; minimal runtime actor boundary.
  - ht-uwd.11 · Cross-record frozen retry and pre-effect refusal regressions · Add cross-record integration regressions that execute real retry/caller preflight against retained synthetic journal/context records, beyond classifier unit tes · deps: ht-uwd.2, ht-uwd.3.1
      owns: frozen-origin and pre-effect integration regression fixtures.
      consumes: frozen retry origin classifier and preflight; runtime actor boundary.

## Requirements (canonical)
R1 Ordinary root reads and communication writes remain available while person/operator actions require immediate human, independent of output formatting or globals.
R2 Root inferred Human and legacy/operator aliases refuse before identity, intent, accountable effects, completed presentation or cleanup.
R3 Retry classification uses original frozen semantics/claim/scope and retains exact bytes/digest/keys and completed agent historical replay.
R4 Independently owned permission lifecycle uses explicit missing-component consent and never silently grants through hook ownership.
R5 Claude positive ordinary rules migrate exact historical owned broad/retired grants with durable interruption recovery and preserved foreign/preexisting/stronger policy.
R6 Codex exact executable union allow plus immediate human prompt operates under active CODEX_HOME/rules, preserving stronger policy and sandbox/network modes.
R7 Validated bare/canonical/link/alias and pinned forms cover installed paths without foreign PATH adoption or ambiguous native path wildcard widening.
R8 Setup/status/unsetup/installer/requested doctor share permission component semantics and truthful diagnostics.
R9 Human ready/retry/continuation guidance preserves strict wire compatibility and honest A2/A3/A4 provenance, shared owner seams remain coordinated.
R10 Early isolated configuration smoke and focused causal regression tests cover grammar, both backends, quoting/compounds, ownership refusal and immutable replay without native models/real config/shared host/full suite.

## Precomputed graph checks
citation: ht-uwd.11 needs ht-uwd.2 blocker
citation: ht-uwd.8.2 needs ht-uwd.7 blocker
citation: ht-uwd.8.1 needs ht-uwd.4 blocker
citation: ht-uwd.7.2 needs ht-uwd.4 blocker
citation: ht-uwd.7.1 needs ht-uwd.5 blocker
citation: ht-uwd.5.3.2 needs ht-uwd.4 blocker
citation: ht-uwd.5.3.1 needs ht-uwd.5.1 blocker
citation: ht-uwd.5.3 needs ht-uwd.4 blocker
citation: ht-uwd.5.2 needs ht-uwd.4 blocker
citation: ht-uwd.5.1 needs ht-uwd.4 blocker
citation: ht-uwd.3.2 needs ht-uwd.1 blocker
citation: ht-uwd.3.1 needs ht-uwd.1 blocker
citation: ht-uwd.10 needs ht-uwd.3.1 blocker
citation: ht-uwd.9 needs ht-uwd.2 blocker
citation: ht-uwd.8 needs ht-uwd.7 blocker
citation: ht-uwd.7 needs ht-uwd.5 blocker
citation: ht-uwd.6 needs ht-uwd.4 blocker
citation: ht-uwd.5 needs ht-uwd.4 blocker
citation: ht-uwd.4 needs ht-uwd.1 blocker
citation: ht-uwd.3 needs ht-uwd.1 blocker
citation: ht-uwd.2 needs ht-uwd.1 blocker
summary: flag-sweep 0 · unstated 0 · citations 21 (dependent 0, unwired 0, unknown 0)

## Changes since previous round
ht-uwd.11 added for C1 cross-record original-actor integration regressions. ht-uwd.10 narrowed blockers to .3.1/.5.1 minimal runnable runtime and renderer for C2 early configuration exercise. ht-uwd.7.2/.8.2 first sentences explicitly own causal consent and actual shell→Rust ownership/refusal regressions for C3, preserving their existing acceptance.

## Coverage ledger
C1 · r1 · GAP · R10-frozen-regressions · applied — ht-uwd.11 cross-record integration witness.
C2 · r1 · GAP · R10-early-smoke · applied — ht-uwd.10 now waits only minimal runtime and native renderers.
C3 · r1 · GAP · R10-installer-regressions · applied — ht-uwd.8.2 and ht-uwd.7.2 explicitly own gateway/consent regression entry points; no duplicated implementation/unit suite.
