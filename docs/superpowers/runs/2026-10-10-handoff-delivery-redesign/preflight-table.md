# Plan preflight table

Initial dispatch clearance: Tasks 1 (ht-0lo.1.1), 2 (ht-0lo.2.1), and 3 (ht-0lo.4.1) are internally consistent and independent logical producers. Their common schema/capability/CLI files merge serially; no common API requires a pre-dispatch decision. All three must preserve unrelated changes when rebasing.

## Initial task internal consistency

| Task | Ownership and checks | Verdict |
| --- | --- | --- |
| 1 / .1.1 | Send-time projection + honest availability separation; advertise messages.send_time_receipts_v1 only with active handlers; require on modern new ACK sends; absent legacy fields/keys retained; lazy/human exemptions and A6 preserved. | PASS |
| 2 / .2.1 | Additive optional confirmation representation; absent means legacy timed; schema stores original inviter/timing; capability refusal before effects; mechanical Invite literal compile fixes retain historical absence. Modern CLI default remains .2.3. | PASS |
| 3 / .4.1 | New stable hook-event grammar maps registry codecs, rejects invalid pairs before effects; legacy dispatch retained, random shell marker removed from new command only; manifest identity retained; native ownership migration remains .4.2. | PASS |

## Initial producer/consumer and shared-file pairs

| Producer / consumer | Shared files or interface | Required compatibility | Verdict |
| --- | --- | --- | --- |
| .1.1 / .2.1 | src/store/schema.rs; src/protocol/capabilities.rs; src/cli/mod.rs | Independent additive schema/capability edits; no version/index/name collision; preserve each other on serial merge. | PASS: serialize merge |
| .1.1 / .4.1 | src/cli/mod.rs | Independent send capability guard and hook dispatch branches; preserve both. | PASS: serialize merge |
| .2.1 / .4.1 | src/cli/commands.rs; src/cli/mod.rs | Additive invite representation construction and hook-event parser are independent. No required mutation ordering. | PASS: serialize merge |

## Shared driver ruling

Coordinator adopted: .1.2 owns shared due driver/fair scheduling and combined fairness tests; .2.2 owns invitation-class cursor/store API/isolated tests. .1.2 must depend on .2.2, and .2.2 must not depend on future shared-driver completion. The full fairness acceptance remains on both beads but is satisfied by producer-class tests followed by consumer combined tests and final sweep.

## Final readiness ruling

The sweep leaf runs focused integration checks, stops owned processes and performs scoped leak check. Parent runs default-feature and any final expensive once-per-branch checks after PR roast against its cleared exact tip, then reports readiness and notifies main. This changes sequencing, never removes acceptance.

## Remaining task internal consistency

| Task | Ownership and checks | Verdict |
| --- | --- | --- |
| 4 / .3.1 | Exact source bytes acquired once, normal nonempty body validation including NUL/whitespace, all three paths and actual serialized retained-envelope maxima; metadata limits separate; historical bytes/digests retained. If real duplicated escaping requires a narrower extra bound adjustment, justify with serializer fixture and update TRUST-POLICY, not unconditional frame widening. | PASS: conditional encoded-bound evidence required |
| 5 / .1.3 | Projection precedes transactional semantics marker/backfill, running/terminal states unchanged; physical and manifest indexes searchable; multiple bounded batches/restart/arrivals tested. Shared scheduler does not belong here. | PASS |
| 6 / .2.2 | Additive mode schema precedes canonical lifecycle; original inviter persists independent of membership; duplicate no policy/originator change; explicit accept only settles; lazy rejection still truthful. Class cursor and isolated tests owned here; shared fairness owned Task7. | PASS under coordinator ruling |
| 7 / .1.2 | Projection and migration landed, invitation class API from Task6 required; combined scheduler preserves 5s/100 bounds, finite high-water fairness and immediate successful continuation. Warn original sender even after leave under current wake guards; clear passive; no service-seat invention. | PASS once added .2.2 edge lands |
| 8 / .2.3 | Lifecycle precedes modern explicit lazy default and capability negotiation; required/operator unaffected, historical saved omission unchanged. Presentation describes lazy/timed accurately. | PASS |
| 9 / .4.2 | Stable grammar precedes exact-owned-group/status/migration updates; installation identity is independent of command string; explicit migration only; repeat in-place; no trust writes; Hermes transport/mod route preserved. | PASS |
| 10 / .3.2 | Frozen payload, receipt capability and warning lifecycle, invitation wire and lifecycle landed. Every modern effect capability-gated; historical child bytes unchanged. Hook readiness advisory; actual used owned config fingerprint still checked. Can consume old or new hook setup representation by preserving route/fingerprint API. | PASS: review concurrent .4.2 launch seam |
| 11 / .5 | Depends all implementation leaves and closed design refinements; owns uncovered integration scenarios and small seam/docs fixes. Parent owns post-PR-roast exact-tip default-feature/final checks and notification. Scoped leak check still runs after focused sweep. | PASS under parent sequencing ruling |

## Interface producer / consumer matrix

| Producer | Consumer | Contract consumed | Preflight disposition |
| --- | --- | --- | --- |
| .1.1 | .1.3 | Canonical send-time projection, existing running deadlines and marker-compatible manifest/physical representation | Preserve old bytes/provenance; migration does not substitute availability. |
| .1.1 + .1.3 | .1.2 | Indexed discoverable pending deadlines, no unavailable-recipient deferral | Sparse/physical lookup and cursor queries must agree; multi-batch upgrade evidence. |
| .2.1 | .2.2 | Optional confirmation mode, original inviter/timing fields, legacy defaults and due indexes | Canonical lifecycle selects absence=legacy timed, explicit Lazy untimed. |
| .2.2 | .1.2 | InvitationDueCursor/InvitationDueBatch bounded selection/apply and has_more/high-water progression | Added dependency per coordinator. Producer owns class API/tests; consumer shared fairness. No reverse dependency. |
| .2.2 | .2.3 | Lazy/timed lifecycle and accurate inbox/query projection | Modern CLI explicitly emits Lazy; no separate registration confirmation. |
| .3.1 | .3.2 | Complete acquired body and exact frozen retained request envelope | New policy fields additive; historical missing fields replay original digest, no source reread. |
| .1.1 | .3.2 | messages.send_time_receipts_v1 and committed send-time deadlines | Refuse unsupported before any new staging/native effects. |
| .1.2 | .3.2 | Durable sender overdue notification independent of registration | Missing profile/hook launch scenario proves pending warning despite no check-in. |
| .2.1 + .2.2 | .3.2 | Frozen invitation confirmation representation and canonical lifecycle | Lazy invitation by default, separate explicit invitation deadline; do not accidentally reuse message deadline. |
| .4.1 | .4.2 | Stable per-event command grammar and canonical-route extraction | Ownership entries/status compare actual event commands, not assumed common command. |
| .4.2 (shared seam, no logical dependency) | .3.2 | Owned launch configuration fingerprint and setup diagnostics | Fingerprint actual used config; missing config advisory. Rebase/seam review must preserve conservative ownership and non-vetoing hook readiness. |
| .1.2 + .2.2 | .5 | Originator warning open wakes/clear passive, both fair due classes | Final combined tests prove no sender membership dependency and no scan dependence on native wake. |
| All 10 implementation leaves | .5 | Complete independently tested delivery/confirmation/hooks behavior | Integration fixes only small wiring gaps; large scope becomes blocker. |

## Exhaustive declared shared-file task pairs

Every pair below shares at least one declared write file. Reads are excluded. For pairs with a dependency, the producer lands before consumer dispatch. Independent pairs use isolated worktrees, hot-file cap and serial merge; rebase requires seam review of both changes. API-specific rulings above override mechanical compatibility. File overlap alone is never a logical dependency.

| Task pair | Shared write files | Disposition |
| --- | --- | --- |
| 1 (ht-0lo.1.1) / 2 (ht-0lo.2.1) | `src/cli/mod.rs`, `src/cli/retry.rs`, `src/protocol/capabilities.rs`, `src/store/schema.rs`, `tests/protocol/capabilities.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 1 (ht-0lo.1.1) / 3 (ht-0lo.4.1) | `src/cli/mod.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 1 (ht-0lo.1.1) / 4 (ht-0lo.3.1) | `TRUST-POLICY.md` | Independent or transitively ordered; serial merge + overlap seam review. |
| 1 (ht-0lo.1.1) / 5 (ht-0lo.1.3) | `src/store/effective.rs`, `src/store/materialization.rs`, `src/store/schema.rs`, `tests/store/effective.rs`, `tests/store/materialization.rs`, `tests/store/receipts.rs` | Producer before consumer; preserve prior API and tests. |
| 1 (ht-0lo.1.1) / 6 (ht-0lo.2.2) | `TRUST-POLICY.md`, `src/store/schema.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 1 (ht-0lo.1.1) / 7 (ht-0lo.1.2) | `TRUST-POLICY.md`, `src/store/schema.rs`, `tests/store/receipts.rs` | Producer before consumer; preserve prior API and tests. |
| 1 (ht-0lo.1.1) / 8 (ht-0lo.2.3) | `src/cli/mod.rs`, `src/cli/retry.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 1 (ht-0lo.1.1) / 10 (ht-0lo.3.2) | `src/cli/mod.rs` | Producer before consumer; preserve prior API and tests. |
| 1 (ht-0lo.1.1) / 11 (ht-0lo.5) | `TRUST-POLICY.md` | Producer before consumer; preserve prior API and tests. |
| 2 (ht-0lo.2.1) / 3 (ht-0lo.4.1) | `src/cli/commands.rs`, `src/cli/mod.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 2 (ht-0lo.2.1) / 4 (ht-0lo.3.1) | `src/cli/commands.rs`, `src/cli/handoff.rs`, `src/cli/journal.rs`, `tests/cli/handoff_delivery.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 2 (ht-0lo.2.1) / 5 (ht-0lo.1.3) | `src/store/schema.rs`, `tests/store/schema.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 2 (ht-0lo.2.1) / 6 (ht-0lo.2.2) | `src/store/schema.rs`, `tests/store/control.rs` | Producer before consumer; preserve prior API and tests. |
| 2 (ht-0lo.2.1) / 7 (ht-0lo.1.2) | `src/store/schema.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 2 (ht-0lo.2.1) / 8 (ht-0lo.2.3) | `src/cli/commands.rs`, `src/cli/mod.rs`, `src/cli/retry.rs`, `src/cli/threads.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 2 (ht-0lo.2.1) / 10 (ht-0lo.3.2) | `src/cli/commands.rs`, `src/cli/handoff.rs`, `src/cli/mod.rs`, `tests/cli/handoff_delivery.rs`, `tests/handoff_topology_authority.rs` | Producer before consumer; preserve prior API and tests. |
| 2 (ht-0lo.2.1) / 11 (ht-0lo.5) | `src/cli/handoff.rs`, `tests/cli/handoff_delivery.rs` | Producer before consumer; preserve prior API and tests. |
| 3 (ht-0lo.4.1) / 4 (ht-0lo.3.1) | `src/cli/commands.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 3 (ht-0lo.4.1) / 8 (ht-0lo.2.3) | `src/cli/commands.rs`, `src/cli/mod.rs`, `tests/cli/commands.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 3 (ht-0lo.4.1) / 9 (ht-0lo.4.2) | `src/cli/setup.rs`, `src/harness/setup.rs`, `tests/harness/setup.rs` | Producer before consumer; preserve prior API and tests. |
| 3 (ht-0lo.4.1) / 10 (ht-0lo.3.2) | `src/cli/commands.rs`, `src/cli/mod.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 4 (ht-0lo.3.1) / 6 (ht-0lo.2.2) | `TRUST-POLICY.md` | Independent or transitively ordered; serial merge + overlap seam review. |
| 4 (ht-0lo.3.1) / 7 (ht-0lo.1.2) | `TRUST-POLICY.md` | Independent or transitively ordered; serial merge + overlap seam review. |
| 4 (ht-0lo.3.1) / 8 (ht-0lo.2.3) | `src/cli/commands.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 4 (ht-0lo.3.1) / 10 (ht-0lo.3.2) | `src/cli/commands.rs`, `src/cli/handoff.rs`, `src/cli/handoff_delivery.rs`, `src/cli/topology_handoff.rs`, `src/protocol/handoff.rs`, `src/store/topology_handoff.rs`, `tests/cli/handoff.rs`, `tests/cli/handoff_delivery.rs`, `tests/cli/topology_handoff.rs`, `tests/handoff_topology_cli.rs` | Producer before consumer; preserve prior API and tests. |
| 4 (ht-0lo.3.1) / 11 (ht-0lo.5) | `TRUST-POLICY.md`, `src/cli/handoff.rs`, `src/cli/handoff_delivery.rs`, `src/cli/topology_handoff.rs`, `tests/cli/handoff.rs`, `tests/cli/handoff_delivery.rs`, `tests/cli/topology_handoff.rs`, `tests/handoff_topology_cli.rs` | Producer before consumer; preserve prior API and tests. |
| 5 (ht-0lo.1.3) / 6 (ht-0lo.2.2) | `src/store/schema.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 5 (ht-0lo.1.3) / 7 (ht-0lo.1.2) | `src/store/schema.rs`, `tests/store/receipts.rs` | Producer before consumer; preserve prior API and tests. |
| 6 (ht-0lo.2.2) / 7 (ht-0lo.1.2) | `TRUST-POLICY.md`, `src/store/mod.rs`, `src/store/queries.rs`, `src/store/schema.rs`, `src/store/wake.rs`, `tests/store/wake.rs` | Producer before consumer; preserve prior API and tests. |
| 6 (ht-0lo.2.2) / 11 (ht-0lo.5) | `TRUST-POLICY.md` | Producer before consumer; preserve prior API and tests. |
| 7 (ht-0lo.1.2) / 11 (ht-0lo.5) | `TRUST-POLICY.md`, `tests/scheduler/deadlines.rs` | Producer before consumer; preserve prior API and tests. |
| 8 (ht-0lo.2.3) / 9 (ht-0lo.4.2) | `integrations/skill/SKILL.md` | Independent or transitively ordered; serial merge + overlap seam review. |
| 8 (ht-0lo.2.3) / 10 (ht-0lo.3.2) | `README.md`, `integrations/skill/SKILL.md`, `src/cli/commands.rs`, `src/cli/mod.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 8 (ht-0lo.2.3) / 11 (ht-0lo.5) | `README.md`, `integrations/skill/SKILL.md`, `tests/cli/skill.rs` | Producer before consumer; preserve prior API and tests. |
| 9 (ht-0lo.4.2) / 10 (ht-0lo.3.2) | `integrations/skill/SKILL.md`, `src/cli/launch.rs`, `src/harness/launch.rs`, `tests/harness/launch.rs` | Independent or transitively ordered; serial merge + overlap seam review. |
| 9 (ht-0lo.4.2) / 11 (ht-0lo.5) | `integrations/skill/SKILL.md`, `tests/integration/setup_canary_seam.rs` | Producer before consumer; preserve prior API and tests. |
| 10 (ht-0lo.3.2) / 11 (ht-0lo.5) | `README.md`, `integrations/skill/SKILL.md`, `src/cli/handoff.rs`, `src/cli/handoff_delivery.rs`, `src/cli/topology_handoff.rs`, `tests/cli/handoff.rs`, `tests/cli/handoff_delivery.rs`, `tests/cli/topology_handoff.rs`, `tests/handoff_topology_cli.rs` | Producer before consumer; preserve prior API and tests. |

## Hot files and exact write-boundary notes

`src/cli/mod.rs`, `src/cli/commands.rs`, `src/store/schema.rs`, `TRUST-POLICY.md` and embedded skill docs are shared but distinct ownership regions are required. Each leaf updates only its own clauses/branches; no whole-file stylistic cleanup. Apply the configured hotFileCap, not artificial reads-as-writes serialization.

Task2 has a deliberately larger write list because adding a public Invite struct field requires compile-preserving edits to every existing struct literal. These are mechanical legacy `None` adaptations, not ownership of later modern CLI/lifecycle behavior. Task implementers report additional concrete write paths before expanding declared boundaries; do not automatically include every read dependency.

No source change or live configuration mutation occurred during planning. SDD scripts were invoked through bash because the installed script is not executable. Durable progress ledger remains run-directory/progress.md; scratch is only briefs, review packages, temporary reports.

## Decisions and unresolved conflicts

Resolved by coordinator: one shared due-driver owner (.1.2), invitation class owner (.2.2), and .1.2 dependency on .2.2. Resolved by parent: default-feature and final expensive branch verification after PR roast, with exact-tip SHA. No missing human design decision prevents dispatch.

Implementation evidence still required (not a design blocker): actual worst-case serializer capacity in Task4; ownership/launch-fingerprint cross-seam in Tasks9/10; protocol literal compile updates in Task2; combined fair driver bound with both classes in Task7. These cannot be asserted from planning alone.
