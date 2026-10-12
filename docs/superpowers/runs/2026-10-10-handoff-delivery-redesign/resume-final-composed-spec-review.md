# Final composed specification review

2026-10-11. **Scoped source PASS; final runtime/readiness remains pending.** No new confirmed specification defect was found in the cumulative seams inspected. This is an independent specification review, not a code-roast round, exhaustive runtime clearance, or authorization to merge/release.

## Identity and method

- Candidate: `ROOT/.worktrees/upstream-composition-v090`, HEAD `9a573c3f2a5420ff0c3fee07a3e836d189e88a83`, tree `5158b885c8988b1c3ebf5b96047751a6e4949212`.
- Actual local Main comparison: `23801c98ece1a8c437420d6cd823c5780993f386`, already included by candidate merge `7f1879f9`. Origin b6 and installed0.8.1 are parent-provided coordination facts; no release/install inspection was performed.
- ROOT: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign`. WS: `ROOT/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish`. RUN: `ROOT/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign`.
- Read the root design, Task14 combined acceptance brief and supersessions, Task15 final author requirements, composition plan, source author handoffs, current TRUST policy, prior whole/composition reviews, notice-Clear review, and final Q1 Spec/Quality reviews. Reviewed source and cumulative diff against actual Main, and specifically the production delta since the prior e85 composition review. Historical reports guide coverage; their runtime approvals do not transfer automatically.
- Read-only Git/filesystem work only. No Cargo, product/SQL runtime, Herdr, subagent, source/index/Git mutation or process-owning test action. This report is the only write. Candidate HEAD/tree and clean status were checked before writing. Appendix maps every changed source/test/tool/schema path; mapping is review coverage, not a claim of fresh line-by-line inspection or execution of every inherited file.

## Requirements and conclusions

| Requirement | Source assessment and evidence |
| --- | --- |
| A: deadlines, invitation semantics, canonical origins | Send-time effective/physical/sparse/backfill paths and confirming-only invitation due/wake semantics remain composed with Main binding attribution. Modern explicit Lazy and historical omitted timed policy remain distinct; duplicate invitations/direct pairs retain original policy/origin. Canonical origin wake survives informational offer and partial settlement; Clear stays passive. Current A7 removes the previously reported contradictory affected-seat-only paragraph. Source coverage carries from whole-epic review and v090 review, checked against current policy/wake seams. |
| B/C14: frozen bodies, original/retry, launch and compatibility | Body-bearing plans/automatic identities retain bounded exact serialization, summary versus optional artifact and no-relaunch/unknown outcome boundaries. Some-only AutomaticBootstrap replies select the 2MiB ceiling before client allocation; unrelated/error/historical None retain the ordinary ceiling. Client checks family, serialized original and normal correlation. New optional policy fields use absent serialization for legacy bytes. Native setup is advisory; chosen native inputs and actually used hook fingerprints retain rechecks. No claim that supported aggregate capacity or original failed public paths have newly passed. |
| D15 and Main: exact authority and durable ownership | `binding_scope` resolves canonical claim then verifies connected/open top-level role, mapping/target/execution and recovery hold (`invitation_content.rs:264`). Store independently compares internal binding hints. Consumer/proof PKs are `(binding_id,consumer)`; item PK adds invitation. Hook reconnect uses Main's admitted canonical path with bounded view; Watch requires existing exact Live owner. Per-seat permit serializes work but never selects/aliases a sibling. Canonical invitations and receipt obligations remain seat-owned. |
| D15: physical presentation is not settlement | `confirm` validates entire emitted subset against original descriptor slots before effects (`invitation_content.rs:823`), keeps current/last token masks, and returns only currently applicable acknowledged controls. It mutates private presentation, never receipt ACK, invitation acceptance, ordinary read cursor or Lazy completion. Watch/Hook emitters confirm captured stamps only after successful output/flush. The mod's separately authorized full-message completion remains distinct; invitations are non-ackable and passive Lazy invitation never enters idle-submit work. |
| Main sibling semantics and Q1 body retirement | Normal and exact-key ordinary paths use the actual reader binding; pending receipt OR own unread publication retains eligibility. A sibling ACK cannot reclaim this reader's carry/offers. Q1 classifier checks positive captured manifest/recipient/member/receipt authority plus exact own cursor; Pending remains eligible even after own display advancement. Terminal+own displayed retires only the one exact private continued body. Public Inbox cursor strictness remains. The accumulator validates exact identity/byte offset or skip relation before retirement. No public cursor/ACK/offer stamp is synthesized. |
| Cancellation/rollback | `store/mod.rs:1302` wraps all tentative Pull writes and final work/output checks in `execute_budgeted_decision`. The existing wrapper checks cancellation after Decision clock sampling and before acceptance; refusal rolls back. Service ledger/replay installation happens only after successful return. The final Q1 phase-controlled test, not merely its ordinal-clock predecessor, establishes the post-tentative-save boundary. Accepted commit is not retrospectively undone. |
| Notice Clear and current logical masks | The carried-notice/active-episode continuation remains at `invitation_content.rs:2661`, is saved in outbox_more and replayed. It grants another bounded canonical relevance advance rather than manufacturing absence. Current positive witness survives unrelated fence changes; negative absence also matches the binding notice frontier. Controls retain original slots, exact epoch/version applicability and one current/one last token; stale/foreign/subset retries cannot acknowledge a newer episode. |
| Owner replay and logical history | Registry captures once per successful Live lease, including empty, fenced by exact binding/generation/native/execution/lease/incarnation. Full nonzero u8 Offer counts remain valid, including count4. Physical replay does not call logical record for its emitted keys, mutate normal cursor/carry/partial, advance public display/receipt, or overwrite current post-store native outlook. Replacement/incarnation/full-stamp pruning remains; no same-lease refill after loss. Registry and ledger locks are not nested across store work. |
| Retention and source work | At most two consumer rows per used binding, invitation/binding bookkeeping plus cleanup debt; no false global durable cap. Registry64 owners×256 stamps; ledger4096 bindings×256 offers with fixed consumer caches. Ordinary modes remain mutually exclusive. Reconciliation256 visits/2560 probes; due/replay100 attempts/1803 probes/32 hydration; carry32/579; private continuation15 probes plus one charged ordinary candidate. Fresh discovery433, Refresh160, relevance13 sources/26 seeks/100 visits and bounded keyed/witness work; publication100 physical rows before recipient filtering/200 probes/3 seeks. Output32/65536 bytes, sizing352, carry155731, partial49415 remain. These are separate source/probe envelopes, not an all-SQL/VM or wall-clock guarantee. |
| Source SQL and indexes | Pending invitation initial/thread indexes and binding-leading revisit/cleanup indexes match their bounded ORDER/LIMIT traversals. Relevance declares exact indexed source slots, bounded negative fallback/high waters and local positive authority; warning close-sweep lookup limits before after-position interpretation. Exact ordinary key candidates are charged before filtering. Current Main session-key index is retained for human eligibility. Runtime plans/VM bounds still require refreshed matrix evidence; static index presence is not that proof. |
| Physical/schema preservation | Independently compared all35 published migration Git blobs between actual Main and candidate: byte-exact. Candidate adds only36–45; schema latest45 retains Main32–35 behavior/audits.37 explicitly preserves both physical and sparse `ack_binding_id` columns and copy lists.44/45 first-create binding-specific private state; no published-history reinterpretation. Actual35 test now inserts historical attributed messages originally, avoiding immutable-message UPDATE, and retains populated snapshots, forced37 rollback at36, retry and physical reopen. Runtime still pending at report cutoff. STRICT44 invitation quick_check and up-to4C45 validation are acknowledged startup costs, not constant-read claims. |
| New Main238 and automatic optimization | Main seat-level wake selectors are retained in both candidate/current_authority, alongside candidate open-origin and confirming-only invitation behavior; exact delivery never substitutes this selector for BindingId. The automatic optimization shares only an already-parent-guarded child reader; `validate_phase` still runs the requested replay guard and request validation, preserving stronger fresh/live rejection and historical replay. New expired-permit/atomic-refusal controls are retained. Their source presence and an optimization mutant PASS do not establish the original expiry cause. |

## Prior findings and execution evidence

**Q1 resolved within its scoped correction.** Independently read `/private/tmp/ht-C-v090-q1-eight-corrected/{execution.json,run.log,leaks.log}`. Exact6cf/tree18ea ran8/8 PASS, nextest1.730s, wrapper2.125s, UUID633b9ed6-ae76-4fdb-8254-8611e6afd0a2, leaks0. Read both fresh final Q1 reviews and inspected the retained production changes. Source identity is still historical6cf; this is not relabeled as a9a execution. The sibling oracle requires B's exact8192 bytes/truncation while A advances, and Decision cancellation checks full private-row rollback/retry. Their causal review distinguishes malformed fixture failure from actual cancellation RED.

**Q2 resolved for its corrected ten-control scope on this exact candidate.** During this review parent completed `/private/tmp/ht-C-v090-q2-composed-corrected/{execution.json,run.log,leaks.log}`; read all three. Exact9a/tree5158,10/10 PASS, nextest1.603s, wrapper1.942s, UUIDef65dc54-f3dc-4a56-9cd0-a7d266d1d222, leaks0. Tests now separate manual seat ACK from full-body binding display, use actual AdvanceCursors, preserve pending/truncated/full stamps and paired unread sibling controls, and assert2048/2304/2560 where the canonical branch warrants them. They do not merely adjust counts. Listing/build took141.089s after the multi-file import; this is not a passing controlled incremental-build speed measurement.

**Remaining parent gates:** genuine Main31/35 upgrade/reopen/rollback and new Main wake execution were running at cutoff; expanded657 matrix and final gates remain pending. Q1 lint69.133s exceeded the60s budget per parent; retain it as a real observation, with final controlled measurement pending. No current final lint/default-feature/performance or657 PASS is inferred.

Historical D314 on old Main and f330391 execution are source-specific evidence only, not runtime clearance of the9a composition. Preserve automatic-expiry cause **UNKNOWN**, despite duplicate-parent-guard optimization mutant PASS. Preserve original hook **MissingCheckedIn cause UNKNOWN**. Narrow passes, fixed fixtures or current source review cannot waive those failures; the complete original strict scenarios and causal distinctions remain required. Modern max-escaped-body capacity, startup/body separation, operator recovery/response loss, H/D/I bounded driver proofs, Hook/Watch activation and health/frontier independence remain subject to their actual final selections. A pending runtime gate is not itself promoted to an invented source defect.

## Evidence key for path map

- **A:** root/receipts/invitations design; RUN task1/2/7/10/12 reports; WS `resume-whole-epic-spec-review.md`; current send-time/origin/invitation policy and v090 store/schema author reports. Requirements: deadlines, Lazy/required acceptance, origins, bounded discovery.
- **B:** root/handoff design, task13 report and Task14 brief; whole-epic Spec review; automatic capacity/expiry review history. Requirements: exact body/policy/original, aggregate/wire bounds, summary/recovery and launch safeguards. Current optimization source inspected; original expiry remains unknown.
- **C:** hooks design and Task14 brief; whole-epic Spec review; v090 frontend/CLI author reports. Requirements: stable event commands/ownership, advisory readiness, unchanged fallback and registered codec/emit paths. Original hook cause remains unknown.
- **D:** Task15 report/refinements; WS active-D-whole, notice-clear, prior v090 Spec/Quality, delivery/presentation/store-binding/neutral/frontend author reports; current inspected producer/store/consumer chain. Requirements: passive content, physical confirmation, exact binding, controls, owner replay, retention and bounded work.
- **Q:** final Q1 Spec/Quality plus current continuation source and actual6cf8; v090 ledger/service author Q2 follow-up plus actual9a10. Requirements: private body progress, canonical display-based reclaim, cancellation/rollback and sibling independence.
- **S:** v090 schema author, active schema review, current migration blob comparison/copy/index inspection. Requirements: immutable1–35, additive36–45, attribution/history/physical upgrade and indexed bounded sources; runtime migration proof pending.
- **V:** Task14 validation contract, .config grouping/test registration and relevant source-author inventory. Test/support/tool paths are mapped to the behavior they exercise; a mapping does not claim execution. Leak tooling has no product authority. All unlisted prose-only run artifacts are evidence documentation, outside the source map.

## Complete changed source/test/tool path map

The mechanical inventory below is the exact actualMain238→candidate9a delta restricted to `src/`, `migrations/`, `tests/`, `scripts/`, executable JS/TS assets and `.config/nextest.toml`. Every included path is listed exactly once. Current source inspection is concentrated on the composed seams above; evidence codes explicitly carry prior static coverage for unchanged implementation rather than silently transferring prior runtime approvals.

| Path | Requirements / review evidence |
| --- | --- |
| `.config/nextest.toml` | V; bounded/isolated validation support |
| `integrations/claude/mod/hooks/register.js` | D; C; passive invitation, control subsets and mod full-display separation |
| `integrations/claude/mod/tests/delivery.test.ts` | D; C; passive invitation, control subsets and mod full-display separation |
| `integrations/claude/mod/tests/fixtures/watch-activation.ts` | D; C; passive invitation, control subsets and mod full-display separation |
| `migrations/0036_invitation_confirmation.sql` | S; A/B/D |
| `migrations/0037_receipt_send_time.sql` | S; A/B/D |
| `migrations/0038_handoff_body_bounds.sql` | S; A/B/D |
| `migrations/0039_receipt_deadline_backfill.sql` | S; A/B/D |
| `migrations/0040_invitation_lifecycle.sql` | S; A/B/D |
| `migrations/0041_invitation_warning_origin.sql` | S; A/B/D |
| `migrations/0042_receipt_warning_origins.sql` | S; A/B/D |
| `migrations/0043_automatic_handoff_body_bounds.sql` | S; A/B/D |
| `migrations/0044_invitation_presentation.sql` | S; A/B/D |
| `migrations/0045_presentation_relevance.sql` | S; A/B/D |
| `scripts/canary/test_seam_integration.py` | C; V; stable ownership and Main demo evidence |
| `scripts/canary/test_tier0_checks.py` | C; V; stable ownership and Main demo evidence |
| `scripts/check-no-leaked-processes` | V; bounded/isolated validation support |
| `scripts/harness-canary.sh` | C; V; stable ownership and Main demo evidence |
| `scripts/tests/test_check_no_leaked_processes.py` | V; bounded/isolated validation support |
| `scripts/validate-native-demo.py` | C; V; stable ownership and Main demo evidence |
| `src/archival_legacy.rs` | B; A; C14 compatibility |
| `src/cli/commands.rs` | B; A; C14 compatibility |
| `src/cli/direct_send.rs` | B; A; C14 compatibility |
| `src/cli/doctor.rs` | C; B launch safety |
| `src/cli/handoff.rs` | B; A; C14 compatibility |
| `src/cli/handoff_delivery.rs` | B; A; C14 compatibility |
| `src/cli/help.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/cli/hook.rs` | C; D; registered codec and output completion |
| `src/cli/journal.rs` | B; A; C14 compatibility |
| `src/cli/launch.rs` | C; B launch safety |
| `src/cli/mod.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/cli/setup.rs` | C; B launch safety |
| `src/cli/topology_handoff.rs` | B; A; C14 compatibility |
| `src/cli/topology_handoff/automatic.rs` | B; A; C14 compatibility |
| `src/cli/topology_runtime.rs` | B; A; C14 compatibility |
| `src/cli/watch.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/client/local.rs` | B; A; C14 compatibility |
| `src/client/service.rs` | B; A; C14 compatibility |
| `src/daemon/control.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/daemon/transport.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/daemon/transport/watch_connection.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/delivery/assemble.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/delivery/invitation.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/delivery/ledger.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/delivery/mod.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/delivery/page.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/delivery/presentation.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/harness/adapter.rs` | C; D; registered codec and output completion |
| `src/harness/agy/decode.rs` | C; D; registered codec and output completion |
| `src/harness/agy/delivery.rs` | C; D; registered codec and output completion |
| `src/harness/agy/launch.rs` | C; B launch safety |
| `src/harness/agy/mod.rs` | C; D; registered codec and output completion |
| `src/harness/agy/paths.rs` | C; B launch safety |
| `src/harness/bridge.rs` | C; D; registered codec and output completion |
| `src/harness/claude.rs` | C; D; registered codec and output completion |
| `src/harness/claude/setup.rs` | C; B launch safety |
| `src/harness/claude_mod.rs` | C; D; registered codec and output completion |
| `src/harness/codex.rs` | C; D; registered codec and output completion |
| `src/harness/codex/setup.rs` | C; B launch safety |
| `src/harness/hermes/launch.rs` | C; B launch safety |
| `src/harness/hermes/runtime.rs` | C; B launch safety |
| `src/harness/launch.rs` | C; B launch safety |
| `src/harness/registry.rs` | C; D; registered codec and output completion |
| `src/harness/setup.rs` | C; B launch safety |
| `src/host/native.rs` | C; B launch safety |
| `src/ports.rs` | D; Q; A/B where shared |
| `src/protocol/capabilities.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/protocol/commands.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/protocol/delivery.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/protocol/handoff.rs` | B; A; C14 compatibility |
| `src/protocol/handoff/automatic.rs` | B; A; C14 compatibility |
| `src/protocol/handoff/automatic_bootstrap.rs` | B; A; C14 compatibility |
| `src/protocol/output_compact.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/protocol/results.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/protocol/watch.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/protocol/wire.rs` | B; A; C14 compatibility |
| `src/scheduler/deadlines.rs` | A; D; Main binding/frontier preservation |
| `src/scheduler/mod.rs` | A; D; Main binding/frontier preservation |
| `src/service/delivery_presentation.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/service/delivery_pull.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/service/dispatch.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/service/kicks.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/service/mod.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/service/mod_channels.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/store/attention.rs` | A; D; Main binding/frontier preservation |
| `src/store/control.rs` | A; D; Main binding/frontier preservation |
| `src/store/effective.rs` | A; D; Main binding/frontier preservation |
| `src/store/handoff/automatic.rs` | B; A; C14 compatibility |
| `src/store/invitation_content.rs` | D; Q; A/B where shared |
| `src/store/invitation_due.rs` | A; D; Main binding/frontier preservation |
| `src/store/materialization.rs` | A; D; Main binding/frontier preservation |
| `src/store/messages.rs` | A; D; Main binding/frontier preservation |
| `src/store/mod.rs` | D; Q; A/B where shared |
| `src/store/presentation_relevance.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/store/queries.rs` | D; Q; A/B where shared |
| `src/store/receipts.rs` | A; D; Main binding/frontier preservation |
| `src/store/schema.rs` | S; A/B/D |
| `src/store/seats.rs` | A/B/D cross-feature protocol, admission, presentation; v090 whole review |
| `src/store/service_controls.rs` | A; D; Main binding/frontier preservation |
| `src/store/topology_handoff.rs` | B; A; C14 compatibility |
| `src/store/topology_handoff/attachment.rs` | B; A; C14 compatibility |
| `src/store/topology_handoff/automatic.rs` | B; A; C14 compatibility |
| `src/store/topology_handoff/persistence.rs` | B; A; C14 compatibility |
| `src/store/wake.rs` | A; D; Main binding/frontier preservation |
| `src/test_support/history.rs` | V; bounded/isolated validation support |
| `src/test_support/mod.rs` | V; bounded/isolated validation support |
| `tests/cli/commands.rs` | D; Q/Main compatibility where relevant; V |
| `tests/cli/cooperative.rs` | D; Q/Main compatibility where relevant; V |
| `tests/cli/direct_send.rs` | B; A; V |
| `tests/cli/follow_retry.rs` | B; A; V |
| `tests/cli/handoff.rs` | B; A; V |
| `tests/cli/handoff_delivery.rs` | B; A; V |
| `tests/cli/hook.rs` | C; D; V |
| `tests/cli/hook_neutral.rs` | C; D; V |
| `tests/cli/launch.rs` | C; D; V |
| `tests/cli/lazy_display.rs` | D; Q/Main compatibility where relevant; V |
| `tests/cli/lazy_send.rs` | D; Q/Main compatibility where relevant; V |
| `tests/cli/output.rs` | D; Q/Main compatibility where relevant; V |
| `tests/cli/topology_handoff.rs` | B; A; V |
| `tests/cli/watch.rs` | D; Q/Main compatibility where relevant; V |
| `tests/combined.rs` | V; bounded/isolated validation support |
| `tests/contracts.rs` | C; D; V |
| `tests/daemon/transport.rs` | D; Q/Main compatibility where relevant; V |
| `tests/daemon/watch_connection.rs` | D; Q/Main compatibility where relevant; V |
| `tests/delivery/extraction.rs` | D; Q/Main compatibility where relevant; V |
| `tests/delivery/ledger.rs` | D; Q/Main compatibility where relevant; V |
| `tests/fixtures/agy/plugin/hooks.json` | C; D; V |
| `tests/handoff_topology_authority.rs` | B; A; V |
| `tests/handoff_topology_cli.rs` | B; A; V |
| `tests/harness/adapter_smoke.rs` | C; D; V |
| `tests/harness/agy_decode.rs` | C; D; V |
| `tests/harness/agy_delivery.rs` | C; D; V |
| `tests/harness/agy_launch.rs` | C; D; V |
| `tests/harness/agy_setup.rs` | C; D; V |
| `tests/harness/canary_payloads.rs` | C; D; V |
| `tests/harness/claude_mod.rs` | C; D; V |
| `tests/harness/launch.rs` | C; D; V |
| `tests/harness/setup.rs` | C; D; V |
| `tests/hook_entrypoint.rs` | C; D; V |
| `tests/hook_entrypoint/agy.rs` | C; D; V |
| `tests/integration/agy_sweep.rs` | D; Q/Main compatibility where relevant; V |
| `tests/integration/channel_activation.rs` | D; Q/Main compatibility where relevant; V |
| `tests/integration/lazy_attention.rs` | A; D; V |
| `tests/integration/mod_delivery.rs` | D; Q/Main compatibility where relevant; V |
| `tests/integration/notice_wake.rs` | A; D; V |
| `tests/integration/topology_activation.rs` | B; A; V |
| `tests/lifecycle_ux.rs` | D; Q/Main compatibility where relevant; V |
| `tests/native/demo/test_demo_verify.py` | C; V; stable ownership and Main demo evidence |
| `tests/protocol/capabilities.rs` | D; Q/Main compatibility where relevant; V |
| `tests/protocol/delivery.rs` | D; Q/Main compatibility where relevant; V |
| `tests/protocol/lazy_inbox_v2.rs` | D; Q/Main compatibility where relevant; V |
| `tests/protocol/watch.rs` | D; Q/Main compatibility where relevant; V |
| `tests/scheduler/deadlines.rs` | A; D; V |
| `tests/scheduler/dispatch.rs` | D; Q/Main compatibility where relevant; V |
| `tests/scheduler/poke_flow.rs` | A; D; V |
| `tests/service/cooperative.rs` | D; Q/Main compatibility where relevant; V |
| `tests/service/crash_matrix.rs` | D; Q/Main compatibility where relevant; V |
| `tests/service/delivery_pull.rs` | D; Q; A/B where shared |
| `tests/service/kicks_contract.rs` | D; Q/Main compatibility where relevant; V |
| `tests/service/lanes.rs` | D; Q/Main compatibility where relevant; V |
| `tests/service/mod_channels.rs` | D; Q/Main compatibility where relevant; V |
| `tests/service/worker_health.rs` | D; Q/Main compatibility where relevant; V |
| `tests/setup_cli.rs` | C; D; V |
| `tests/store/archival_legacy.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/attention.rs` | A; D; V |
| `tests/store/catch_up.rs` | A; D; V |
| `tests/store/commit_kicks.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/control.rs` | A; D; V |
| `tests/store/cooperative_checkin.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/direct_threads.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/effective.rs` | A; D; V |
| `tests/store/facade.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/handoff_fences.rs` | B; A; V |
| `tests/store/invitation_content.rs` | D; Q; A/B where shared |
| `tests/store/invitation_due.rs` | A; D; V |
| `tests/store/lazy26_prerequisite.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/lazy_inbox.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/materialization.rs` | A; D; V |
| `tests/store/queries.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/receipts.rs` | A; D; V |
| `tests/store/retention.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/schema.rs` | S; A/B/D |
| `tests/store/service_send.rs` | D; Q/Main compatibility where relevant; V |
| `tests/store/topology_attachment_guard.rs` | B; A; V |
| `tests/store/topology_attempts.rs` | B; A; V |
| `tests/store/topology_handoff.rs` | B; A; V |
| `tests/store/topology_handoff_schema.rs` | S; A/B/D |
| `tests/store/wake_discovery_cost.rs` | A; D; V |

Inventory total: **188 paths**, including **86 src paths** and **10 additive migration paths**. All35 published migration blobs matched actual Main. The source map is exhaustive for the stated filter; broader documentation is covered through the named normative/evidence inputs.

Review lane QUIESCENT. No owned processes were started.
