## Task 11: Integration sweep: handoff delivery redesign

filesTouched: ["tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/handoff_topology_cli.rs", "tests/integration/notice_wake.rs", "tests/integration/setup_canary_seam.rs", "tests/scheduler/deadlines.rs", "tests/cli/skill.rs", "integrations/skill/SKILL.md", "README.md", "TRUST-POLICY.md", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs"]

bead: ht-0lo.5

Dependencies: ht-0lo.2.3, ht-0lo.2.2, ht-0lo.6, ht-0lo.1.3, ht-0lo.1.1, ht-0lo.7, ht-0lo.3.2, ht-0lo.2.1, ht-0lo.4.1, ht-0lo.4.2, ht-0lo.3.1, ht-0lo.1.2

### Full bead text (canonical snapshot)

```json
{
  "id": "ht-0lo.5",
  "title": "Integration sweep: handoff delivery redesign",
  "description": "owns: remaining cross-feature integration wiring, documentation consistency and end-to-end tests no owning leaf covers; consumes: all implementation leaves. Verify long durable inbox handoff, all native delivery paths, send-time unavailable-recipient expiry, lazy/timed invitation acceptance and independent sender warnings, retained keyed retry and stable per-event owned hooks. Fix small integration gaps inline; file blockers for larger discoveries. Run focused isolated tests, cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and nice scripts/check-default-features on the final candidate. Do not run the routine full suite while ht-zo4 restriction remains. Tests own/tag/stop all children/private sessions and never alter shared server or real user configs. Files: integration tests, embedded skills/help/docs, TRUST-POLICY.md and minimal seam wiring.\nblocked-by ht-0lo.1.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.1.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.1.3: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.2.3: consumes all leaves (integration sweep)\nblocked-by ht-0lo.3.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.3.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.4.1: consumes all leaves (integration sweep)\nblocked-by ht-0lo.4.2: consumes all leaves (integration sweep)\nblocked-by ht-0lo.6: consumes all leaves (integration sweep)\nblocked-by ht-0lo.7: consumes all leaves (integration sweep)",
  "acceptance_criteria": "Main handoff flows complete end to end with exact frozen bodies/routing and independent receipt/invitation deadlines. Overdue originators receive durable guarded waking attention; late confirmation clears passively. All owned native configurations and legacy retries remain safe. Final format, focused tests, all-feature clippy and default-feature gates pass; owned processes stopped. (needs: ht-0lo.1.1) (needs: ht-0lo.1.2) (needs: ht-0lo.1.3) (needs: ht-0lo.2.1) (needs: ht-0lo.2.2) (needs: ht-0lo.2.3) (needs: ht-0lo.3.1) (needs: ht-0lo.3.2) (needs: ht-0lo.4.1) (needs: ht-0lo.4.2)",
  "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
  "status": "open",
  "priority": 2,
  "issue_type": "task",
  "owner": "235553+alepar@users.noreply.github.com",
  "created_at": "2026-10-10T23:09:38Z",
  "created_by": "Alexey Parfenov",
  "updated_at": "2026-10-10T23:18:42Z",
  "metadata": {
    "depth": 1,
    "order": 5
  },
  "labels": [
    "sp:ht-0lo"
  ],
  "dependencies": [
    {
      "id": "ht-0lo.2.3",
      "title": "Invitation CLI and presentation contract",
      "description": "owns: ordinary invite --lazy default, --require-ack/--deadline implication and conflicts, compact/JSON/inbox/help/embedded skill presentation; consumes: invitation confirmation protocol/store contract. Files: src/cli parser invitation branches/mod/output and help/skill resources, relevant CLI tests. Invoke actual CLI in isolated daemon/config for bare lazy, require-ack default duration, explicit deadline, conflicting flags, duplicate and historical keyed retry. Required/operator commands remain unchanged.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.2: consumes invitation confirmation store behavior",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:15Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:33Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2.2",
      "title": "Invitation acceptance, due and delivery lifecycle",
      "description": "owns: canonical lazy/timed invitation create/duplicate/accept/reject/cancel/retire behavior, due scanner and inviter-addressed warning routing plus read/wake/mod projections; consumes: invitation mode schema/wire contract. Files: src/store/{control,invitation_due,wake,queries,service_controls,seats}.rs as needed, daemon projections, TRUST-POLICY invitation clauses and focused tests. Run lazy invite no native wake/deadline, timed never-register and register-without-accept expiry, acceptance clears, duplicate never mutates policy, inviter leaving warning, required membership and explicit rejection notice cases.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md\nblocked-by ht-0lo.2.1: consumes invitation mode schema/wire contract\nOriginal inviter attribution frozen at invitation creation; overdue inviter open warning waking under existing native/mod guards even after leaving, clear passive; update TRUST-POLICY A7. Duplicate caller never retargets originator warning.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:14Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:51Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.6",
      "title": "Define fair due-scan progress and conditional detection bound",
      "description": "Design-only fix for confirmed roast R1-1. owns: normative finite-backlog scanner progress/latency contract and early multi-batch validation; consumes: existing5s safety tick,100candidate batch and frozen cursor semantics. Update root/receipt/invitation specs and task acceptances; implementation remains owned by ht-0lo.1.2,ht-0lo.1.3 and ht-0lo.2.2.",
      "acceptance_criteria": "Normative specs and implementation owners explicitly resolve the confirmed design finding; independent next roast verifies the changed contract.",
      "status": "closed",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T23:18:25Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:55Z",
      "closed_at": "2026-10-10T23:18:55Z",
      "close_reason": "Design contract and implementation task acceptance updated; independent round2 will verify.",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.3",
      "title": "Upgrade dormant historical receipt obligations",
      "description": "owns: transactional upgrade/backfill of formerly dormant historical ACK obligations, sparse/physical due projection consistency and idempotent schema marker; consumes: canonical send-time receipt projection. Preserve already running deadlines, terminal states/provenance and immutable request/result digests. Files: src/store/{schema,effective,materialization}.rs and focused migration/store tests. Exercise legacy unavailable receipts upgrade/restart/replay and overdue indexed discovery, bounded warning burst.\nblocked-by ht-0lo.1.1: consumes canonical send-time receipt projection\nExercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
      "acceptance_criteria": "Legacy dormant physical/manifest ACK obligations receive send-time deadlines while existing started/terminal/waived obligations remain unchanged. Idempotent restart and focused migration tests pass. Exercise dormant historical upgrade over multiple100candidate batches and verify no overdue initial row is lost or starved by newly arriving work; discovery contract is owned by ht-0lo.1.2.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:49:48Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:48Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.1",
      "title": "Canonical send-time receipt projection",
      "description": "owns: receipt original deadline semantics, preserving running deadlines and terminal provenance; consumes: existing immutable send timestamps/durations. Files: src/store/{effective,materialization,schema,messages}.rs and preparation modules, TRUST-POLICY deadline clauses, focused store tests. Cover sparse and physical receipts, restart, lazy/human waiver, registration not resetting and explicit A6 extension distinction.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nReceipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:10Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:44Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.7",
      "title": "Define ordinary-send receipt semantic compatibility boundary",
      "description": "Design-only fix for confirmed roast R1-4. owns: named send-time receipt capability and pre-effect modern client refusal boundary; consumes: existing capability discovery mechanism. Update root/receipt/handoff specs and ht-0lo.1.1 acceptance. No historical request mutation or new adversarial verification.",
      "acceptance_criteria": "Normative specs and implementation owners explicitly resolve the confirmed design finding; independent next roast verifies the changed contract.",
      "status": "closed",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T23:18:35Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:59Z",
      "closed_at": "2026-10-10T23:18:59Z",
      "close_reason": "Design contract and implementation task acceptance updated; independent round2 will verify.",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.3.2",
      "title": "Handoff confirmation policy wiring and recovery documentation",
      "description": "owns: handoff --deadline/--require-invite-ack/--invite-deadline parsing/freeze/pass-through, modern lazy invitation plus addressed ACK-required body staging across new-tab/empty-pane/existing paths, retained child digest compatibility and source/help/skill docs; consumes: frozen handoff payload, invitation confirmation wire contract and send-time receipt semantics. Files: src/cli/{handoff,handoff_delivery,topology_handoff,mod}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and related tests/help/skill. Run isolated actual handoff per all three paths/body sources; crash/retry after stage and possible-start/unknown-create never duplicates/restages/relaunches. Older plans absent new fields replay original keyed requests, modern deadlines freeze before staging; routing exact3field fallback and no auto trust/accept/ACK.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nblocked-by ht-0lo.3.1: consumes frozen handoff payload\nblocked-by ht-0lo.2.1: consumes invitation confirmation wire contract\nblocked-by ht-0lo.1.1: consumes send-time receipt semantics\nAlso owns: separating hook/setup/trust/profile readiness diagnostics from launch admission; src/cli/launch.rs prepare_launch and src/harness/launch.rs configuration checks. Missing/untrusted/wrong-profile hooks do not block staged work or otherwise structurally safe launch. Preserve empty-shell/seat/caller/native argv and owned launch-configuration fingerprint safety, and truthful native trust refusal; never auto-approve. Test missing/stale profile hooks through guarded handoff and observe durable ACK deadline warning without registration.\nblocked-by ht-0lo.2.2: consumes canonical invitation lifecycle\nblocked-by ht-0lo.1.2: consumes overdue receipt warning lifecycle\nHook readiness is advisory: keep owned launch configuration fingerprint recheck only when that configuration is actually used; absence cannot veto and no fingerprint is fabricated. Unsupported environment-overrides/native grammar/host-kind safety refusals remain.\nRequire messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Observe durable overdue warning for an unregistered recipient (needs: ht-0lo.1.2). Require messages.send_time_receipts_v1 before new handoff effects; consume ht-0lo.1.1 capability contract and verify unsupported-daemon refusal before staging/launch.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:20Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:53Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.2.1",
      "title": "Invitation confirmation protocol and schema contract",
      "description": "owns: additive invitation delivery/confirmation wire fields, capability refusal and schema fields/indexes/legacy defaults; consumes: existing operation digests and membership episodes. Files: src/protocol commands/results/capabilities or existing capability module, src/store/schema.rs and schema tests. Absent historical fields mean legacy timed behavior; explicit modern Lazy requests retain timing/inviter origin without altering old bytes. Freeze duration at creation, preserve required and human invitation semantics.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--invitations-design.md",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:13Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:13Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.4.1",
      "title": "Stable native event dispatch and command generation",
      "description": "owns: hook-event HARNESS EVENT parser/registered-codec dispatch and stable per-event shell command grammar; consumes: existing harness event registry and canonical routing. Files: src/cli/{hook,mod,setup}.rs, src/harness/{setup,claude,codex}.rs, registry declarations/tests. Keep legacy hook syntax; validate invalid pairs before effects; command stability includes executable/route/harness/event only. Exercise declared Claude/Codex events with fixtures through dedicated entrypoints; retain Hermes callback and bundled mod route compatibility.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:17Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:17Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.4.2",
      "title": "Owned stable registration migration and tooling",
      "description": "owns: manifest-recorded exact ownership/install identity, repeat setup in place, recoverable UUID/shared-command migration, safe unsetup and status/trust/canary/launcher command compatibility; consumes: stable native per-event command grammar. Files: src/harness/setup.rs and claude/codex setup modules, src/cli setup/launch consumers, canary/tooling/docs/tests. Invoke isolated Claude/Codex setup-repeat/migration/interruption/unsetup with foreign identical/drifted groups and ensure no ownership theft, no trust writes, no group shifting on equivalent reinstall. Hermes/Claude mod regressions stay passing.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--hooks-design.md\nblocked-by ht-0lo.4.1: consumes stable native per-event command grammar",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:18Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:48:31Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.3.1",
      "title": "Handoff instruction source acquisition and frozen payload",
      "description": "owns: positional/file/stdin source exclusivity, bounded nonempty UTF-8 65,536-byte acquisition and exact bytes frozen in all handoff intent forms; consumes: existing compound intent/body schema. Files: src/cli/handoff.rs parser and validation, src/cli/{handoff_delivery,topology_handoff}.rs saved plans and associated parser sources/tests. Preserve topic/goal limits separately; invalid sources fail before mutations; mutate/delete source after initial staging and prove retry keeps original content. Native argv contains fixed routing/inbox only.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--handoff-design.md\nAlso owns: body-bearing envelope capacity calculation and limits: bootstrap identity/progress512KiB, recovery1MiB; preserve metadata-only creation/attachment128KiB, completed2MiB/globalwire1MiB unless actual serializer worst-case fixtures prove a narrowly scoped bound adjustment necessary. Files additionally src/store/topology_handoff.rs and persistence.rs, src/cli/journal.rs, TRUST-POLICY encoded-limit clauses. Reuse src/cli/input.rs; protocol body validator must follow normal nonempty UTF-8 message bytes (NUL/whitespace are not identifier validation). Inventory all repeated bodies/nested originals; actual serializer64KiB six-byte escape public wire/canonical/local retained/retry fixtures prove support under ordinary bounded metadata. Exact serialized preflight before effects; retain historical bytes/digests and no clipping.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:19Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:59:16Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo.1.2",
      "title": "Due discovery and originating-seat warning lifecycle",
      "description": "owns: bounded indexed due discovery plus sender-addressed open/clear warning routing and guarded poke/wake calculations; consumes: send-anchored canonical receipt projection. Files: src/store/{receipts,poke,wake,queries,schema}.rs, scheduler tests. Exercise unbound/unregistered/unavailable/pending-invite recipient sends through expiry and late ACK; originator leaving does not lose notification.\nSpec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign--receipts-design.md\nblocked-by ht-0lo.1.1: consumes send-anchored canonical receipt projection\nblocked-by ht-0lo.1.3: consumes historical dormant receipt upgrade\nOriginator open warning is waking attention under existing safe composer/host/mod guards even after leaving; clear passive; update TRUST-POLICY A7. No invented seat for service authors. Test notification pending while drafts/working/host outage defer actual native wake.\nHealthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "acceptance_criteria": "Implement and run every scenario named in this description; preserve the parent spec contract. Focused tests, format and per-change clippy pass. Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.",
      "status": "open",
      "priority": 2,
      "issue_type": "task",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:48:11Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T23:18:46Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "blocks"
    },
    {
      "id": "ht-0lo",
      "title": "Handoff delivery redesign",
      "description": "Deliver thread-carried complete handoff instructions with send-time receipt deadlines, lazy optional-confirmation invitations and stable native hook entrypoints. Spec: docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md. User authorized autonomous implementation; integration merge remains coordinator decision.",
      "spec_id": "docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md",
      "status": "open",
      "priority": 2,
      "issue_type": "epic",
      "owner": "235553+alepar@users.noreply.github.com",
      "created_at": "2026-10-10T22:45:27Z",
      "created_by": "Alexey Parfenov",
      "updated_at": "2026-10-10T22:57:33Z",
      "labels": [
        "sp:ht-0lo"
      ],
      "dependency_type": "parent-child"
    }
  ],
  "parent": "ht-0lo",
  "dependent_count": 0,
  "dependency_count": 13,
  "comment_count": 0,
  "revision": "-6971199537433044500"
}
```

### Acceptance criteria (verbatim)

Main handoff flows complete end to end with exact frozen bodies/routing and independent receipt/invitation deadlines. Overdue originators receive durable guarded waking attention; late confirmation clears passively. All owned native configurations and legacy retries remain safe. Final format, focused tests, all-feature clippy and default-feature gates pass; owned processes stopped. (needs: ht-0lo.1.1) (needs: ht-0lo.1.2) (needs: ht-0lo.1.3) (needs: ht-0lo.2.1) (needs: ht-0lo.2.2) (needs: ht-0lo.2.3) (needs: ht-0lo.3.1) (needs: ht-0lo.3.2) (needs: ht-0lo.4.1) (needs: ht-0lo.4.2)

### Global constraints (verbatim project instructions)

##### Agent guidelines for herdr-threads

###### Development rules

- Per-change check: `nice cargo clippy --locked --all-targets --all-features -- -D warnings` (clippy already type-checks every target; a separate `cargo check` is redundant). Before a merge, also `nice scripts/check-default-features` (a default-feature `cargo check --all-targets` that fails on any warning; CI runs the full default-feature clippy job as well); format: `cargo fmt`.
- Tests: run the tests relevant to your change (`nice cargo test --locked --all-features <filter>`); the full suite is `nice cargo nextest run --locked --all-targets --all-features` (cargo-nextest: one process per test, every binary in one pool; test groups in `.config/nextest.toml` keep the budget- and latency-sensitive chains one at a time; `nice scripts/full-suite-gate N` runs it N times in a row with leak checks, and `nice scripts/flake-hunt` hunts flaky tests). A test that must not overlap others of its kind gets a nextest test group, not only an in-binary mutex (a mutex does not span processes).
- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- After a full-suite run (and after each run of the 10-run gate or the integration sweep), run `scripts/check-no-leaked-processes`. Export `HT_LEAK_RUN_ID=<uuid>` before the suite and pass `--run-id "$HT_LEAK_RUN_ID"` to it, so another worktree's live run is never flagged. A non-zero exit lists each leaked pid; stop them (or rerun with `--kill`) and fix the test that leaked them.
- Test targets are listed explicitly (`autotests = false`). A new suite that keeps no process-wide state goes into `tests/combined.rs` as a `#[path]` module (one binary, one link); a suite that redirects stdio, installs a panic hook, runs an in-process daemon, arms failpoints or changes the umask gets its own `[[test]]` entry. A target that uses `herdr_threads::test_support` (including `test_support::spawn`) needs `required-features = ["test-support"]`; otherwise `scripts/check-default-features` and the default-feature clippy job fail.
- The daemon's owner watch exists only in `test-support` builds: always test with `--all-features`.

###### Trust policy

[TRUST-POLICY.md](TRUST-POLICY.md) is normative for seat continuity, caller attribution, receipt provenance
and operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
`src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
`src/cli/hook.rs`, launch or wake.

- Do not add adversarial caller verification; the model is cooperative and same-user.
- Record claims honestly: a new way to attribute an action needs a provenance value defined in the policy.
- Never merge seats, never move a seat or end a binding on heuristic evidence.
- Decide in the daemon against the canonical view (A2); client-local files are hints, never authority.
- A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit.

###### Speed budgets

- The full test suite must stay under **5 minutes** wall-clock (`cargo nextest run --locked --all-targets --all-features`). Treat a regression past it as a bug: fix slow or serialized tests rather than raising the budget.
- An incremental build must stay under **1 minute** (edit one source file, then `cargo build` / `cargo test --no-run`), and so must the per-change lint (`cargo clippy --all-targets --all-features -- -D warnings`).
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.


### Normative parent contracts (verbatim)

#### Acceptance and validation

- Long instructions survive restart/retry unchanged; native argv excludes the body. Invalid, oversized, conflicting or unreadable sources fail before durable writes.
- New and previously dormant ACK obligations expire without recipient activity; already running deadlines do not move. Late registration/acceptance never restarts the clock. Due scans are bounded/indexed, sender receives warnings, late receipt clears normally.
- Lazy ordinary invitation produces no overdue/native wake; explicitly timed invitation warns the originator and acceptance settles automatically. No acceptance/ACK is synthesized by launch or registration.
- Equivalent reinstall emits identical per-event commands while foreign and drifted configuration survives; explicit legacy migration is recoverable and honest about native trust needing review.
- Existing bootstrap unknown-create/possible-start guarantees, human waivers, receipt provenance, required memberships, routing and hook trust boundaries continue passing their focused regression tests.
- Run cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings and focused tests per change; before merge readiness also nice scripts/check-default-features. Use test-support-owned children/private Herdr sessions, stop all owned processes, and perform scoped leak checks after any full suite.

#### Non-goals and follow-on

No auto-trust, recipient auto-accept, task-adoption ACK, delivery exactly-once claim, automatic 1:1 thread selection, server restart, seat repair heuristics, real user config writes, deployment or merge into main. The parent run notifies its established main-tab discussion when the integration branch is ready for review/merge.

#### Confirmed roast refinements

Healthy-daemon discovery contract: retain the5,000ms safety tick and100candidate per-drive cap; continue immediately while successful due progress reports more. Each class advances a stable finite high-water cursor fairly; newly arriving obligations do not restart/starve the current snapshot. Native wake admission/completion never gates cursor progress. For a snapshot with at most N candidates per class and successful runnable driver quanta taking at most Q ms including storage/scheduling, durable pending-warning detection finishes within5,000ms +(2*ceil(N/100)+2)*Q; this is conditional on the daemon running and successful bounded storage, not a guarantee across outages, repeated errors, unbounded overload or native key submission. Restart resumes/reconstructs fair discovery and scans already overdue obligations. Early fake-clock tests use at least250candidates in EACH invitation and receipt class, blocked native wakes, mixed future/due rows, continuing arrivals and restart; verify every initial overdue row is warned within the formula and arrivals cannot starve either class. Historical upgrade also uses multiple bounded batches without losing overdue indexed discovery.

Receipt semantic capability: advertise messages.send_time_receipts_v1 only when canonical send-time projection and its ordinary send handlers are active. Modern clients require this capability before every new ACK-required ordinary send or handoff durable/native effect; absent/refused discovery produces explicit unsupported-daemon error, with no silent legacy fallback. All CLI entry paths and service-capable client sends that promise this contract are covered. Old clients talking to an upgraded daemon receive canonical new-send semantics without modifying their old wire requests. Lazy/human-waived sends remain exempt; completed retained keyed historical operations remain inspectable/replayable with original bytes/digests/outcomes and are never rewritten. A pending historical retry on an unsupported daemon refuses before additional effects. Test newCLI/olddaemon refusal, oldCLI/newdaemon canonical send, current/current unbound sends and lazy/human exemptions using capability fixtures.

### Coordinator execution contract

Coordinator ruling: ht-0lo.1.2 owns shared due driver/fair scheduling and combined fairness tests; ht-0lo.2.2 owns invitation-class cursor/store API/isolated tests. Preserve full requirements plus explicit cross-task seam and final sweep coverage. Final default-feature check is parent-owned after PR roast per explicit sequencing.

Read TRUST-POLICY.md before any protected source changes. Only implement declared ownership; mechanical compile-preserving adaptations are permitted. Report any additional write path before expanding filesTouched. Do not write root run-state/report/ledger from task worktrees. Test additions reuse existing module targets unless process-wide state requires a dedicated test target.

### TDD implementation and independently testable deliverable

1. Review final combined implementation against root and four subtree specs; identify uncovered interface cases and add focused isolated end-to-end tests for long durable inbox body, all delivery paths, receipt and invitation independent deadlines, originating-seat wake after leave, exact keyed retry and stable owned event configuration.
2. Run relevant focused tests on integration tip, cargo fmt and all-feature clippy, fix small remaining seam/docs inconsistencies inline; file larger blocker discoveries rather than concealing them. Preserve explicit process ownership and stop all private sessions/helpers. Export a fresh HT_LEAK_RUN_ID and run scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID" after the integration sweep.
3. Parent coordinator owns readiness after PR roast: default-feature gate and any final expensive branch checks run once against the roast-cleared exact tip, with SHA evidence. This leaf supplies focused sweep evidence and does not falsely report the deferred gate as passed. No routine full suite under ht-zo4 restriction.
4. Return final implementation-seam test evidence and remaining blockers to parent. Parent owns report, roasts, final readiness, notify-main milestone and merge decision.

Terminal sweep also verifies expanded H and combined bootstrap completion from the Task7 bounded projection bootstrap refinement; no due-only or changed-row count may establish the global bound.

### Settled integration coverage and actual fixture preflight

Preparation is not execution or dispatch authorization. Before dispatch, consume the actual merged Task7 and Task10 branch/report/review, the settled Task1/4/5/6/8/9 reports, Task10 brief, fixture-preflight.md, execution-seams.md, multi-originator-warning-refinement.md and upgrade-discovery-refinement.md including its Bounded projection bootstrap refinement. Do not invent not-yet-landed Task7/10 APIs, migration ordinals or GREEN claims. Historical full bead snapshots above retain their original text; this explicit coordinator sequencing governs: Task11 owns the focused integration sweep and its own fresh review; parent owns final PR roast and expensive/default-feature exact-tip readiness AFTER roast.

Select actual cross-feature fixtures on the merged integration tip, using existing registered targets and declared filesTouched. Add missing integration coverage only where the combined contract is not established by the actual producer tests; do not rerun every leaf suite or create blanket source/test scope. Any actual extra source path or new suite requires scoped coordinator admission first. Repository autotests=false: a new stateless suite belongs in tests/combined.rs via path module; process-wide stdio/panic-hook/in-process-daemon/failpoint/umask state requires its own explicit [[test]] target. A target using test_support requires required-features=["test-support"]. Do not add a target speculatively; verify actual registration and nonzero selected test count before claiming evidence.

Focused integration cases must connect real public/canonical paths:

- Message and invitation independence: ordinary invite defaults explicit Lazy and remains fully displayable/discoverable without timed/native attention; RequireAck/explicit deadline and historical None retain timed semantics. Handoff body ALWAYS ACK-required, separately frozen message duration versus optional invitation duration, staged while invitation pending. Observe canonical commit-time message deadline and invitation-creation deadline, never registration-based starts. Test unavailable/unbound/managed-unregistered and register-without-accept recipients. Registration/launch settles neither invitation acceptance nor message receipt; explicit accept and accountable ACK have their separate effects. Required/operator/human waiver behavior remains intact. Lazy explicit rejection remains truthful.
- Durable handoff: actual new-tab, explicit empty-pane launch and existing-recipient delivery preserve positional/file/stdin exact content/newlines, including maximum ordinary and six-byte-escaped65,536-byte bodies. Preserve Task4 measured envelope/archival limits and exact pre-effect serializer refusals. Mutate/delete source and alter launch environment after stage; real retry cutpoints preserve body/policy/bytes/digests/child keys. Completed children are not restaged; unknown-create/possible-start cannot automatically recreate/relaunch. Assert fixed startup inbox instruction and exact three-field routing match; null/different/ambiguous instance or canonical state/endpoint cannot override pinned fallback. Native argv contains no task body/body path/arbitrary task command.
- Setup/readiness: equivalent stable-event reinstall preserves exact owned commands/order; unmanaged identical/foreign/drifted groups survive, including canonical legacy foreign registrations; explicit recoverable migration preserves ownership/adoption and honest native trust status. HookInspection remains separate from diagnostic registration_conflict, with existing fullconfig fingerprint semantics. Missing/untrusted/stale/wrong-profile readiness is advisory to otherwise structurally safe handoff launch; actually used owned configuration still receives its exact fingerprint recheck. Preserve fresh empty-shell/occupant/caller/seat/continuity, host-kind/native grammar/environment-override and trust-refusal controls. No automatic trust, acceptance, ACK or adoption claim.
- Mixed versions/retries: require messages.send_time_receipts_v1 and invitation.confirmation_v1 before NEW corresponding durable/native effects. Unsupported discovery produces truthful refusal before journaling/allocation/staging/launch. Historical absent policy retains original exact requests/digests; completed retained outcomes remain readable. Preserve Task1 accepted conservative unsupported-daemon pending/commit-unknown retry refusal and locally unknown human-audience limit; do not invent public ordinary OperationStatus verified-scope success. Compare frozen linked policy exactly, not permissive Some acceptance.
- Canonical warning lifecycle: exercise actual discover_wake_candidates→offer/load/reserve/native plus real mod/watch paths for EVERY original sender/inviter, including a departed origin and settled informational notice frontier. For shared receipt condition use at least two canonical authors and three exact physical/published-manifest associations; one coalesced open event, no first-source/current-membership shortcut or synthetic origin event. Leave no unrelated pending row masking upstream exclusions. Enumerate actual indexed WAKE_PROBES/seat_has_pending_rows, instance/live/nonhuman ordinal-highwater walk, bounded source enumeration, canonical association/author eligibility, notice/legacy offer frontier, current binding/witness/reservation and native/mod admission gates. Partial settlement preserves the same shared open condition while any required pending unwaived association remains; no per-author quieting mandate. Final settlement emits one passive clear with unchanged receipt provenance. Draft/composer/working/host outage defers keys while durable warning persists; recovery resumes correctly without bypassing guards.

Global discovery integration must measure actual inspected work and successful GLOBAL driver quanta with at least250 candidates in EACH class, H=0 and H>0, blocked wakes, mixed future/due/ineffective rows, continuous earlier-deadline arrivals, restart, early ordinary-due progress and terminal-only old backlog. TOTAL100 candidates/quantum includes repair and due, fair unfinished class service every two successful quanta, immediate continuation without a second5s tick, SQL highwater before LIMIT and full allowance lending. Expanded H counts ALL Task5 physical/prepared deadline-repair AND physical/sparse projection-bootstrap visits, duplicates again, including future/terminal/waived/unpublished rows. D/I count every actually visited snapshot candidate, not changed/warned rows. All repair producers share at-most50 repair slots; unused quota/zero-row probes lend immediately. Combined global completion resets ordinary traversal exactly once only after ALL producers finish, atomically with progress/warnings; rollback cannot lose it or let local completion prematurely reset.

Verify conditional bounds: H=0 and max(D,I)<=N gives5000ms+(2ceil(N/100)+2)Q; H>0 gives5000ms+2max(ceil(H/50)+2+ceil(D/100)+1,ceil(I/100)+1)Q. Q covers successful runnable global storage/scheduling, snapshots start with already-overdue obligations, D covers finite arrivals before its postrepair snapshot freezes; this is durable detection, not bounded native keys or outages/errors/unbounded overload. Inspect actual merged bootstrap implementation against the published genuine34→35 contract: indexed MAX capture without historical startup INSERT SELECT, bounded reconstruction of every eligible old sparse obligation/open origin, removed tails, rollback/reopen, rowid deletion/reinsertion/reuse and fresh monotonic sparse ordinals without canonical identity rewrite. Build historical fixtures by executing immutable real migrations through the claimed old version BEFORE seeding; assert version/catalog then physically reopen/upgrade. Never initialize latest and relabel user_version or delete current indexes to masquerade as historical data.

### Sweep evidence, cleanup and parent-owned final gates

Run only relevant focused integration selections with --locked --all-features; record exact integration SHA, command/filter, selected/passed/failed/ignored counts and real failure cutpoints. Setup/compiler errors and zero-selection runs are not behavioral evidence. Keep source frozen during builds; do not weaken assertions/timeouts/budgets or repeat already-passed checks for presentation. Task11 runs cargo fmt and nice cargo clippy --locked --all-targets --all-features -- -D warnings and receives its own fresh review. No routine full suite under ht-zo4.

Export a fresh unique HT_LEAK_RUN_ID BEFORE the focused sweep and inherit it in every owned test child/helper/private Herdr session. Use herdr_threads::test_support::{spawn_owned,command,tag} (never bare Command::spawn in tests), isolated HOME/CLAUDE_CONFIG_DIR/CODEX_HOME and named private Herdr sessions. Stop all owned children/daemons/helpers after the focused sweep, then run exactly scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID" and record exit/result. A nonzero result is unfinished work: stop only identified owned processes, fix ownership leakage and recheck scoped cleanup. Never signal foreign processes, restart shared Herdr or alter real user config. Any later full-suite run also requires its own scoped cleanup check.

Parent alone owns adversarial final PR roast, final nice scripts/check-default-features and required expensive branch checks against the roast-cleared exact SHA, readiness report, main-tab notification and merge decision. Report those deferred checks as deferred, never passed on behalf of the parent. Return focused integration evidence, fresh-review result and explicit blockers; this brief does not authorize implementation/execution, expanded paths, final readiness or external notification.

Canonical producer preflight refinement after Task10 RED: real native send publication intentionally stores author_kind NULL (migration0002 documents deriving native from immutable actor seat); Task7 explicit-native-only projection missed those senders. Consume Task10 final merged correction and actual public ordinary-send/handoff coverage, not synthetic explicit-native rows alone. A canonical sparse send requires actual bounded materialization::advance_work before due scans in store-only tests; drive real pending jobs, never manually inject receipt_state merely to make detection succeed. Require same-instance seat, exact published recipient association and service/built-in/unknown negatives. Retained completed handoff outcomes must remain readable without modern capabilities while pending/new operations refuse before effects, with immutable report corruption validation preserved.
