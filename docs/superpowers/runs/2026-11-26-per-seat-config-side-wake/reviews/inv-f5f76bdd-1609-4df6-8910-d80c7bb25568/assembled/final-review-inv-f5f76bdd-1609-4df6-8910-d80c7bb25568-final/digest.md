## diffstat
 AGENTS.md                                          |   12 +-
 CHANGELOG.md                                       |    6 +
 README.md                                          |    1 +
 RELEASE.md                                         |   19 +-
 TRUST-POLICY.md                                    |  126 +-
 docs/agent-usage.md                                |    4 +
 docs/evidence/ansi-composer/README.md              |    2 +-
 docs/install.md                                    |   15 +-
 .../2026-11-26-per-seat-config-side-wake-design.md |  382 +++
 ...fig-side-wake-roast-design-1-step-back-keys.txt |    3 +
 ...at-config-side-wake-roast-design-1-step-back.md |    6 +
 ...-26-per-seat-config-side-wake-roast-design-1.md |  139 +
 ...-26-per-seat-config-side-wake-roast-design-2.md |   51 +
 ...-27-per-seat-config-side-wake-roast-design-3.md |   76 +
 ...e-wake-roast-pr-1-final-review-keys.refmap.json |   85 +
 ...nfig-side-wake-roast-pr-1-final-review-keys.txt |   11 +
 ...-config-side-wake-roast-pr-1-step-back-keys.txt |   20 +
 ...r-seat-config-side-wake-roast-pr-1-step-back.md |   14 +
 ...6-11-27-per-seat-config-side-wake-roast-pr-1.md |  156 +
 .../amend-code-1.md                                |   48 +
 .../coverage-findings-round-1.md                   |   17 +
 .../coverage-findings-round-2.md                   |   22 +
 .../coverage-ledger.md                             |   61 +
 .../coverage-round-1-requirements.md               |   16 +
 .../coverage-round-2-requirements.md               |   16 +
 .../evidence/leak-check.txt                        |    3 +
 .../evidence/run-2/driver.log                      |    5 +
 .../evidence/run-2/side-results.json               |  138 +
 .../evidence/run-3/driver.log                      |    1 +
 .../evidence/run-3/side-results.json               |  645 ++++
 .../evidence/side_spike.py                         |  190 ++
 .../findings-ledger-design.md                      |   16 +
 .../findings-ledger.md                             |   23 +
 .../findings-overrides-design.md                   |    0
 .../findings-overrides.md                          |    3 +
 .../friction.md                                    |    9 +
 .../profile-inv1.json                              | 3144 ++++++++++++++++++
 .../profile-inv1.md                                |  324 ++
 .../bounded-coverage.json                          |  329 ++
 .../bounded-draft.md                               |   72 +
 .../diffstat.txt                                   |  109 +
 .../generated-input-provenance.md                  |   27 +
 .../review-inputs.json                             | 3421 ++++++++++++++++++++
 ...iew-inv-78cf59fa-89c0-4cd2-9bc6-b210465af1d2.md |   75 +
 .../roast-design-1-tree.md                         |  205 ++
 .../roast-design-2-tree.md                         |  206 ++
 .../roast-design-3-tree.md                         |  290 ++
 .../2026-11-26-per-seat-config-side-wake/run.md    |   61 +
 .../spike-side-identity.md                         |   90 +
 docs/superpowers/specs/INDEX.md                    |    1 +
 docs/validation/herdr-release-matrix.md            |    2 +-
 evidence/ht-z41e.11/checks.log                     |   16 +
 evidence/ht-z41e.11/seat_config_sweep.log          |   13 +
 evidence/ht-z41e.3/checks.log                      |   11 +
 evidence/ht-z41e.3/clippy.log                      |    4 +
 evidence/ht-z41e.3/golden.log                      |   34 +
 evidence/ht-z41e.3/lib-tests.log                   |  285 ++
 evidence/ht-z41e.8/selftest.log                    |  439 +++
 integrations/codex/README.md                       |    6 +
 integrations/skill/SKILL.md                        |    4 +-
 migrations/node/0020_seat_config.sql               |   13 +
 migrations/node/0021_side_queue_routing.sql        |    5 +
 migrations/node/0022_binding_auxiliary_session.sql |    7 +
 scripts/release-qualify                            |  103 +-
 scripts/tests/test_release_qualify.py              |    3 -
 scripts/wake-e2e.py                                |  299 +-
 scripts/wake_e2e_queue.py                          |   95 +-
 scripts/wake_e2e_queue_selftest.py                 |   78 +-
 scripts/wake_e2e_selftest.py                       |  181 +-
 src/app.rs                                         |    3 +-
 src/cli/commands.rs                                |  163 +-
 src/cli/config_seat.rs                             |  229 ++
 src/cli/doctor.rs                                  |   43 +
 src/cli/hook.rs                                    |    2 +
 src/cli/mod.rs                                     |   77 +
 src/cli/nudge_threshold.rs                         |    2 +-
 src/cli/peer_launch.rs                             |    1 +
 src/cli/site_capability.rs                         |   51 +-
 src/daemon/settings.rs                             |   32 +-
 src/harness/adapter.rs                             |   26 +
 src/harness/agy/decode.rs                          |    1 +
 src/harness/bridge.rs                              |   14 +-
 src/harness/claude.rs                              |    1 +
 src/harness/claude/permission_rules.rs             |    8 +
 src/harness/codex.rs                               |   37 +
 src/harness/codex/composer.rs                      |   23 +-
 src/harness/codex/permission_rules.rs              |   52 +
 src/harness/composer.rs                            |    5 +-
 src/harness/hermes.rs                              |    1 +
 src/harness/mod.rs                                 |    4 +
 src/harness/opencode/decode.rs                     |    1 +
 src/harness/pi/decode.rs                           |    1 +
 src/harness/registry.rs                            |   91 +
 src/host/native.rs                                 |   56 +
 src/notification/dispatch.rs                       |    7 +
 src/notification/queue.rs                          |  129 +-
 src/ports.rs                                       |   42 +
 src/ports/wake_intents.rs                          |    9 +
 src/protocol/authority.rs                          |    9 +
 src/protocol/capabilities.rs                       |    9 +
 src/protocol/commands.rs                           |   59 +
 src/protocol/handoff.rs                            |    1 +
 src/protocol/mod.rs                                |    1 +
 src/protocol/results.rs                            |   36 +
 src/protocol/seat_config.rs                        |  200 ++
 src/protocol/service.rs                            |    1 +
 src/scheduler/mod.rs                               |   30 +-
 src/service/config.rs                              |   17 +-
 src/service/delivery_pull.rs                       |   43 +-
 src/service/dispatch.rs                            |   59 +-
 src/service/kicks.rs                               |    3 +
 src/store/archival.rs                              |    1 +
 src/store/bindings.rs                              |   26 +
 src/store/control/retirement.rs                    |    2 +
 src/store/invitation_content.rs                    |    1 +
 src/store/mod.rs                                   |   54 +
 src/store/queries.rs                               |   15 +
 src/store/schema.rs                                |   15 +
 src/store/seat_config.rs                           |  523 +++
 src/store/seats/continuity.rs                      |    1 +
 src/store/seats/cooperative.rs                     |   12 +-
 src/store/seats/threshold.rs                       |   75 +-
 src/store/wake.rs                                  |    3 +-
 src/store/wake_intents.rs                          |   19 +-
 src/store/wake_intents/selection.rs                |   88 +-
 src/test_support/history.rs                        |    1 +
 src/test_support/mod.rs                            |    1 +
 tests/cli/accept_delivery.rs                       |    1 +
 tests/cli/commands.rs                              |  109 +
 tests/cli/config_seat.rs                           |  854 +++++
 tests/cli/cooperative.rs                           |    2 +
 tests/cli/direct_send.rs                           |    1 +
 tests/cli/doctor_json.rs                           |   43 +
 tests/cli/follow_retry.rs                          |    1 +
 tests/cli/golden_contract.rs                       |    1 +
 tests/cli/handoff.rs                               |    1 +
 tests/cli/hook.rs                                  |    5 +
 tests/cli/hook_inbox.rs                            |    1 +
 tests/cli/journal.rs                               |    1 +
 tests/cli/lazy_display.rs                          |    2 +
 tests/cli/lazy_markers.rs                          |    1 +
 tests/cli/lazy_recovery_context.rs                 |    1 +
 tests/cli/output.rs                                |    1 +
 tests/cli/wake_on.rs                               |    1 +
 tests/cli/watch.rs                                 |    1 +
 tests/combined.rs                                  |    2 +
 tests/contracts.rs                                 |    2 +
 tests/daemon/launch_gate_guards.rs                 |    1 +
 tests/daemon/multi_host.rs                         |    1 +
 tests/daemon/self_retire.rs                        |    1 +
 tests/daemon/watch_connection.rs                   |    1 +
 .../codex-0.163.0-side/pre-tool-use-root-null.json |   12 +
 .../codex-0.163.0-side/pre-tool-use-root.json      |   12 +
 .../pre-tool-use-side-nonnull.json                 |   12 +
 .../codex-0.163.0-side/pre-tool-use-side.json      |   12 +
 .../session-start-fork-nonnull.json                |    9 +
 .../codex-0.163.0-side/session-start-root.json     |    9 +
 tests/fixtures/codex-0.163.0-side/stop-side.json   |   11 +
 .../user-prompt-submit-root.json                   |   10 +
 .../user-prompt-submit-side.json                   |   10 +
 tests/fixtures/herdr-matrix/codex.js               |   11 +-
 tests/fixtures/wake_intent.rs                      |  144 +
 tests/harness/agy_delivery.rs                      |    1 +
 tests/harness/agy_wake.rs                          |    1 +
 tests/harness/bridge.rs                            |    2 +
 tests/harness/codex.rs                             |   73 +
 tests/harness/role_witness.rs                      |    1 +
 tests/hook_entrypoint/agy.rs                       |    2 +-
 tests/integration/herdr_release_smoke.rs           |   28 +-
 tests/integration/lazy_attention.rs                |    3 +
 tests/integration/notice_wake.rs                   |    1 +
 tests/integration/seat_config_sweep.rs             |  242 ++
 tests/integration/summary_flow.rs                  |    1 +
 tests/integration/sweep_remaining.rs               |    1 +
 tests/integration/topology_activation.rs           |    2 +
 tests/lifecycle_ux.rs                              |    1 +
 tests/notification/queue_runtime.rs                |  110 +-
 tests/protocol/capabilities.rs                     |   88 +-
 tests/protocol/delivery.rs                         |    1 +
 tests/protocol/priority.rs                         |    1 +
 tests/protocol/watch.rs                            |    1 +
 tests/release/release_qualify_test.py              |  148 +-
 tests/scheduler/dispatch.rs                        |   61 +
 tests/scheduler/poke_flow.rs                       |    1 +
 tests/scheduler/wake_receipt.rs                    |  259 +-
 tests/service/composition.rs                       |    2 +
 tests/service/cooperative.rs                       |   25 +
 tests/service/crash_matrix.rs                      |    3 +
 tests/service/delivery_pull.rs                     |   41 +
 tests/service/graph_d2.rs                          |    2 +
 tests/service/graph_send.rs                        |    1 +
 tests/service/host_routing.rs                      |    1 +
 tests/service/lazy_hook_pull.rs                    |    1 +
 tests/service/mod_ack_dispatch.rs                  |    1 +
 tests/service/mod_channels.rs                      |    1 +
 tests/service/mod_digest_flag.rs                   |    2 +
 tests/service/named_startup.rs                     |    1 +
 tests/service/peer_handoff.rs                      |    1 +
 .../service/presentation_owner_budget_controls.rs  |    1 +
 tests/service/resolution.rs                        |    3 +
 tests/service/seat_config.rs                       |  629 ++++
 tests/service/turn_state.rs                        |    1 +
 tests/store/attention.rs                           |    1 +
 tests/store/binding_inbox.rs                       |    1 +
 tests/store/bindings.rs                            |    5 +
 tests/store/catch_up.rs                            |    1 +
 tests/store/control.rs                             |    4 +
 tests/store/cooperative_checkin.rs                 |  229 ++
 tests/store/deadline_window.rs                     |    1 +
 tests/store/facade.rs                              |    2 +
 tests/store/fact_convergence.rs                    |    1 +
 tests/store/fact_convergence_invitation.rs         |    1 +
 tests/store/fact_convergence_membership.rs         |    1 +
 tests/store/handoff_fences.rs                      |    1 +
 tests/store/invitation_content.rs                  |    1 +
 tests/store/lazy_inbox.rs                          |    2 +
 tests/store/lazy_publication.rs                    |    1 +
 tests/store/messages.rs                            |    1 +
 tests/store/mod_ack.rs                             |    1 +
 tests/store/multi_binding_readers.rs               |    1 +
 tests/store/multi_host.rs                          |    1 +
 tests/store/named_presence.rs                      |    2 +
 tests/store/notice_replay.rs                       |    1 +
 tests/store/nudge_threshold.rs                     |    1 +
 tests/store/peer_handoff.rs                        |    2 +
 tests/store/priority_send.rs                       |    1 +
 tests/store/query_work_q4.rs                       |    1 +
 tests/store/receipts.rs                            |    2 +
 tests/store/retention.rs                           |    1 +
 tests/store/reversible_reads.rs                    |    2 +
 tests/store/reversible_retirement.rs               |   54 +
 tests/store/schema.rs                              |   82 +
 tests/store/seat_config.rs                         |  795 +++++
 tests/store/send_fence.rs                          |    1 +
 tests/store/service_send.rs                        |    1 +
 tests/store/snapshot_eligibility.rs                |    1 +
 tests/store/summary.rs                             |    1 +
 tests/store/wake.rs                                |   44 +
 tests/store/wake_activation_recovery.rs            |   73 +
 tests/store/wake_intents.rs                        |   32 +-
 tests/store/wake_on.rs                             |    1 +
 tests/store/wake_queue_cost.rs                     |  360 ++
 tests/sync/facts_message.rs                        |    1 +
 tests/sync/facts_receipt.rs                        |    1 +
 244 files changed, 19015 insertions(+), 442 deletions(-)

## generated-input provenance
## Generated-input provenance
Captured range: 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3..c3e9ab0becd137adb68f23a1c9e1a87669d03168
Selection: complete; selected product/input paths: 244; omitted historical payloads: 0.
diffstat.txt describes selected product/input paths. Reports, findings and collection metadata remain ordinary review inputs.
- Outside captured range: 48 informational entries (neither selected nor waived); the first 20 are listed; the full list is review-inputs.json outOfRange.
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:78: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:106: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:109: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:111: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:112: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:116: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:117: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): c3e9ab0becd137adb68f23a1c9e1a87669d03168:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:78: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): c3e9ab0becd137adb68f23a1c9e1a87669d03168:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:106: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): c3e9ab0becd137adb68f23a1c9e1a87669d03168:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:109: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): c3e9ab0becd137adb68f23a1c9e1a87669d03168:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:111: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): c3e9ab0becd137adb68f23a1c9e1a87669d03168:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:112: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): c3e9ab0becd137adb68f23a1c9e1a87669d03168:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:116: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): c3e9ab0becd137adb68f23a1c9e1a87669d03168:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:117: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-264d-4506-beaa-307c9600bb8d: invalid failed assembly archive: failed bounded archive contains successful assembly or split claims
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58 inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f: invalid failed assembly archive: failed bounded archive contains successful assembly or split claims
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: incomplete claimed collection
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: incomplete claimed final report
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: invalid failed assembly archive: missing bounded evidence: final-review-inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb.md
- Outside captured range (informational; neither selected nor waived): 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: incomplete assembled bundle: digest.md
Open each report/archive at its recorded revision. Omission supplies no finding disposition or verdict.

## report summaries
task 1:
  A: - Ctrl-C delivered to the stand-in via `pane send-text "\x03"` (no new helper); release-smoke not executed (needs private Herdr via scripts/herdr-release-smoke).
  A: - Lines above carry no source-bound evidence file kept in repo; evidence below refers to scratch logs.
task 2:
  A: - writer_binding stores the claim's binding_id (BindingId) for agent writes.
  A: - "pending p3 becomes owed" asserted as wake_work attention_version bump via note_new_attention (as tests/store/wake_on.rs does), not a full pending-row fixture.
  A: - Service tests per binding class: pane Claude, pane Codex, named seat, terminal (modeled as a host row of kind hc). Plain-CLI vs hook-registered vs managed-launch are the same claim to the daemon, so they are not separate fixtures.
  A: - Human tier with a subagent claim attached is refused early (accountable_claim); human tier normally carries no claim.
  A: - programmatic_wake description stays generic (neutral core); Codex example is in TRUST-POLICY.
  A: - is_default for a table key means "no row" (an explicit off is a set, not default).
  C: - TRUST-POLICY ask-rule table: agy/opencode/Hermes/Pi cells are read from setup code only (ht-z41e.3 verifies).
  C: - Terminal class is modeled by host-kind relabel in the fixture, not a real hc session.
  C: - tests/store/reversible_retirement.rs got one added test (needs its Fx fixture); brief named tests/store/seat_config.rs.
  C: - Build target dir of this worktree left in place (seeded clone) for later tasks.
task 3:
  Status: IMPLEMENTED (evidence-only spike, no Rust). base a3a6e3619d3aaf92f15e133208da201950f060e2, head ecf09b4d254f59c279c350be729b975fc19ed00e, tree 9fab85a7e866b76eab1b5e5c67c922f73a723016.
  A: - Owned PTY instead of an isolated Herdr workspace and no herdr-threads daemon: S1-S4 concern Codex behaviour and the raw hook payload is the adapter's input. The "owned Herdr workspace closed" acceptance line is therefore not literally exercised.
  A: - Binding rows (`seat inspect`) were not captured; "marker" is a hook-payload field, persistence by the adapter is for ht-z41e.7.
  C: - run-2: a side turn ("SIDE_READY") produced no Stop hook within 90 s once; run-3 and the earlier fixture work normally. Unexplained, possibly model latency; one trial per S2/S4 case, no soak.
  C: - Screen evidence is raw stripped terminal text (no emulator); the side-session submit of the preserved draft is the strong proof.
  C: - Real-account Codex ran (quota spent); credential identifiers were scrubbed from captures.
task 4:
  Status IMPLEMENTED. base 0ad0a8428286875d7b00dcbe5ec21914250ad36f (stack: ht-z41e.2), head 5b26452d0bd4a1d8605051950b540f2f5597984f, stacked true.
  A: - Agent-path refusals are at parse time (no round trip), which satisfies "before any round trip".
  A: - `--seat` is a shared clap flag refused for agents in `parse_argv`, not by clap itself.
  A: - A bare `ht human config seat set K V` from a pane defaults to the pane's located seat (not in the brief; consistent with the brief's "or the caller's seat"). The default fill is unit-tested through a `#[cfg(test)]` seam; no end-to-end pane test.
  A: - Subagent write refusal happens in `run` after caller derivation, as with nudge-threshold.
  A: - Evidence logs are committed under evidence/ht-z41e.3; tested source tree equals HEAD^{tree} minus those files.
task 5:
  A: - Wake fixture (tests/fixtures/wake_intent.rs) now seeds the seat with programmatic_wake on; queue_runtime sets it explicitly per case.
  A: - "fake-peer two seats" test uses a second seat row without binding (selection for b has no binding either way); flag isolation is asserted via programmatic_enabled.
  A: - Scheduler-level "queue attempt follows" is shown as a new reservation; QueuePort path itself covered by selection tests.
  A: - stop-run-processes --check errored ("git worktree inventory failed") for the tmp root argument; manual ps found no leftover processes.
  A: - install.md mentions `unset`; the CLI is Task 3's.
task 6:
task 7:
  A: - Arrival (R5): already satisfied structurally. Queue wakes never await generic contact (existing test queue_transport_never_awaits_generic_contact_or_marks_unreceived) and are fulfilled only by exact target_json scope; added a test that a side turn does not fulfill and a root turn does. No schedule
  A: - Side-composer native fallback crossing: covered by Task 1's existing test `cooperative_wake_defers_on_a_side_composer_and_prompts_once_it_reads_empty` (src/host/native.rs: TargetUnsafe before still_owed, nothing typed, prompted once main placeholder returns); I added no duplicate at the queue-runt
  A: - A lone concurrent (fork) binding is now never queue-targeted (was targeted before).
  A: - No new model-visible text.
task 8:
  A: - Spike S2 is "drop" per TRUST-POLICY/AGENTS from task 7 (busy side does not refuse root), so under --queue-proof side-nudge/side-draft with sides expect Accepted on the root (bead text mentions "keep"; implemented code is authoritative).
  A: - side-draft under queue: passes on intact draft + delivered, no refusal warning required (queue transport never types).
  A: - side-nudge uses the root-baseline proof but not the side_sessions requirement (sides may have ended after Ctrl+C); only side-queue demands non-empty side_sessions.
  A: - Deferred minors (AGENTS.md/release-qualify wording, TmuxRecipient, inspect.getsource test) not touched: none of those lines rewritten.
  C: - No live run (Task 10 runs it). The side pane comparison (prompt rows, SIDE-*-DONE answers, wake marker count) and the side-native-defer refusal timing are untested against a real Codex.
  C: - side-queue relies on the root turn running while the side chat is displayed; if Codex redraws the side view for it, side_view_changed would fail and need loosening.
task 9:
task 11:
  A: - PostToolUse fixture from the brief skipped: Codex's registered hooks and `parse_shape` have no PostToolUse event (it is rejected as Invalid), so there is no payload to classify; Stop is covered instead.
  A: - Evidence rides on `CallerClaim` (as the brief says), set only by the bridge on the check-in claim; a journaled check-in retried through `herdr-threads retry` (src/cli/retry.rs) against a daemon that lacks the capability is not stripped and would be refused as an unknown field. Only matters for a d
  A: - A side whose first daemon contact is something other than a hook check-in (no such path found in the Codex hook flow) would carry no evidence.
  A: - Test fixture `fixture()` is now unused by seat_config_sweep; kept with `#[allow(dead_code)]` like its siblings.
task 12:
task 13:
task 14:
task 15:
task 16:
  Status: IMPLEMENTED. base 5920db007 (stack: ht-z41e.26), head f077f5814, tree f4d85071cac3435f29fcae6ac33512f16380552c.

## interfaces and seams
src/cli/commands.rs: pub enum ConfigSeatAction {
src/cli/commands.rs: pub fn seat(&self) -> Option<&SeatId> {
src/cli/commands.rs: pub fn seat_mut(&mut self) -> Option<&mut Option<SeatId>> {
src/cli/commands.rs: pub const CONFIG_SEAT_FLAG_REFUSAL: &str =
src/cli/config_seat.rs: pub const SEAT_CONFIG_REFUSAL: &str = "herdr-threads: daemon predates seat config; restart it";
src/cli/config_seat.rs: pub const GET_NEEDS_SEAT: &str = "herdr-threads: config seat get needs --seat S (no caller seat)";
src/cli/config_seat.rs: pub const HUMAN_NEEDS_SEAT: &str =
src/cli/config_seat.rs: pub const SUBAGENT_REFUSAL: &str =
src/cli/config_seat.rs: pub const LIST_TRAILER: &str = "set: ht config seat set KEY VALUE (agent tier); ht human config seat set [--seat S] KEY VALUE (any key)";
src/cli/config_seat.rs: pub fn render_list() -> String {
src/cli/config_seat.rs: pub fn render_list_json() -> Result<String, serde_json::Error> {
src/cli/config_seat.rs: pub(crate) fn run_list<W: Write>(parsed: &ParsedCli, writer: &mut W) -> Result<(), RunError> {
src/cli/config_seat.rs: pub(crate) fn run<C: LocalClient + ?Sized, W: Write>(
src/cli/mod.rs: pub(crate) fn default_seat_for_test(action: &mut CliAction, seat: crate::protocol::ids::SeatId) {
src/cli/nudge_threshold.rs: pub(crate) fn has_no_idle_wake<C: LocalClient + ?Sized>(
src/cli/site_capability.rs: pub fn drop_unadvertised_auxiliary<C: LocalClient + ?Sized>(
src/cli/site_capability.rs: pub fn require_site_capability<C: LocalClient + ?Sized>(
src/harness/adapter.rs: pub struct RootCandidate<'a> {
src/harness/registry.rs: pub fn wake_process_root(
src/ports.rs: pub(crate) fn with_programmatic_wake(mut self, enabled: bool) -> Self {
src/ports.rs: pub fn programmatic_wake(&self) -> bool {
src/protocol/capabilities.rs: pub const SEAT_CONFIG_V1: &str = "seat.config_v1";
src/protocol/capabilities.rs: pub const SEAT_AUXILIARY_SESSION_V1: &str = "seat.auxiliary_session_v1";
src/protocol/commands.rs: pub struct GetSeatConfig {
src/protocol/commands.rs: pub struct SetSeatConfig {
src/protocol/commands.rs: pub struct UnsetSeatConfig {
src/protocol/results.rs: pub struct SeatConfigView {
src/protocol/results.rs: pub struct SeatConfigEntry {
src/protocol/results.rs: pub struct SeatConfigWrite {
src/protocol/seat_config.rs: pub enum ConfigTier {
src/protocol/seat_config.rs: pub fn as_str(self) -> &'static str {
src/protocol/seat_config.rs: pub enum ConfigValues {
src/protocol/seat_config.rs: pub enum ConfigBacking {
src/protocol/seat_config.rs: pub struct ConfigKey {
src/protocol/seat_config.rs: pub const REGISTRY: &[ConfigKey] = &[
src/protocol/seat_config.rs: pub fn key(name: &str) -> Result<&'static ConfigKey, String> {
src/protocol/seat_config.rs: pub fn human_tier_refusal(key: &str, args: &str) -> String {
src/protocol/seat_config.rs: pub fn values_label(&self) -> String {
src/protocol/seat_config.rs: pub fn validate(&self, value: &str) -> Result<String, String> {
src/store/seat_config.rs: pub(crate) fn record_audit(
src/store/seat_config.rs: pub(crate) fn delete_for_final_retirement(tx: &Connection, seat: &SeatId) -> Result<(), ApiError> {
src/store/seat_config.rs: pub fn effective_value(
src/store/seat_config.rs: pub const PROGRAMMATIC_WAKE: &str = "programmatic_wake";
src/store/seat_config.rs: pub(crate) fn programmatic_wake_on(db: &Connection, seat: &SeatId) -> bool {
src/store/seat_config.rs: pub fn seats_with_value(db: &Connection, key: &str, value: &str) -> Result<Vec<String>, ApiError> {
src/store/seat_config.rs: pub fn get(db: &Connection, seat: &SeatId, key: Option<&str>) -> Result<SeatConfigView, ApiError> {
src/store/seat_config.rs: pub fn set_agent(
src/store/seat_config.rs: pub fn unset_agent(
src/store/seat_config.rs: pub fn set_operator(
src/store/seat_config.rs: pub fn unset_operator(
src/store/seats/cooperative.rs: pub(crate) fn require_local_seat_setting(
src/store/seats/threshold.rs: pub(crate) fn write_threshold(
src/store/seats/threshold.rs: pub(crate) fn write_column(
tests/fixtures/wake_intent.rs: pub(crate) fn fixture_with(
tests/fixtures/wake_intent.rs: pub(crate) fn set_programmatic_wake(store: &SqliteStore, seat: &str, on: bool) {
tests/fixtures/wake_intent.rs: pub(crate) fn enroll_payload(
tests/store/cooperative_checkin.rs: pub(crate) fn check_in_payload(
new public files: see diffstat; seam beads: none (no bead in the ht-z41e tree has a title starting `Seam contract:`)

## deferred minors
Task 1 (ht-z41e.1): minor (deferred): [doc-drift] docs/evidence/ansi-composer/README.md:22 still says the /side placeholder is added to the known placeholders, contradicting the edit on line 21
Task 1 (ht-z41e.1): minor (deferred): [unexecuted-test] tests/integration/herdr_release_smoke.rs herdr_release_wake side-close path and inverted /side expectation (plus codex.js Ctrl-C change) compiled by clippy but not run (ignored, needs private Herdr)
Task 1 (ht-z41e.1): minor (deferred): [missing-evidence] test log is only in scratch /tmp/sp-6cd888ee/ht-z41e.1/test.log, not retained with the repo
Task 3 (ht-z41e.6): minor (deferred): [setup-deviation] spike-side-identity.md Method: owned PTY, no Herdr session/workspace and no daemon (bead asked for both); 'workspace closed' acceptance is vacuous; disclosed and does not affect S1-S4 conclusions
Task 3 (ht-z41e.6): minor (deferred): [unverified-persistence] spike-side-identity.md S1: binding-row (seat inspect) persistence of transcript_path not captured; deferred to ht-z41e.7 via the doc's follow-ups
Task 3 (ht-z41e.6): minor (deferred): [single-trial-evidence] S2/S4 observed in one trial each; run-2 had an unexplained side Stop timeout (kept as failing evidence)
Task 3 (ht-z41e.6): minor (deferred): [weak-screen-evidence] S4 screen captures are raw stripped text with no emulator; side-session draft submit is the stronger proof
Task 2 (ht-z41e.2): minor (deferred): [identity-mismatch] dispatched HEAD 970fff16 is the tree hash of commit 8af93f415; reviewed that commit, whose tree matches
Task 2 (ht-z41e.2): minor (deferred): [service-test-fixture-modeling] terminal seat class is modeled by relabeling the host kind to hc, and plain-CLI/hook-registered/managed-launch are not separate fixtures (the daemon sees the same claim); report-documented
Task 2 (ht-z41e.2): minor (deferred): [missing-evidence] the report's test/clippy/check-script outputs were not independently rerun; only final.log was read (646 passed)
Task 4 (ht-z41e.3): minor (deferred): [seam-only-test] human config seat set defaulting to the located pane seat is unit-tested through a cfg(test) seam, with no end-to-end pane test (tests/cli/config_seat.rs:709)
Task 4 (ht-z41e.3): minor (deferred): [process-inspection-refused] stop-run-processes --check failed in the implementer's environment (git worktree inventory failed, exit 4); leak check was a manual ps only
Task 5 (ht-z41e.4): minor (deferred): [weak-isolation-test] tests/store/wake_queue_cost.rs per_seat_flag test: second seat 'b' has no binding, so isolation is asserted only through programmatic_enabled, not a real two-seat queue selection
Task 5 (ht-z41e.4): minor (deferred): [test-level-gap] stale-authority fallback is tested at store arm level only, not through the runtime (report states fixture cannot interleave)
Task 5 (ht-z41e.4): minor (deferred): [deferred-doc-update] AGENTS.md pre-release wake e2e paragraph and scripts/release-qualify, wake-e2e.py still name the node programmatic_wake setting; deferred to ht-z41e.5
Task 5 (ht-z41e.4): minor (deferred): [missing-evidence] report records the test command and counts but no retained log or evidence array
Task 6 (ht-z41e.5): minor (deferred): [missing-edge-case-coverage] scripts/wake-e2e.py TmuxRecipient has its own start() and never calls enable_queue_wake, so tmux Codex seats stay native under --queue-proof (previously node-wide); likely intended, undocumented
Task 6 (ht-z41e.5): minor (deferred): [source-text-test] scripts/wake_e2e_selftest.py ordering test for Recipient.start uses inspect.getsource index comparison, fragile to refactors
Task 7 (ht-z41e.7): minor (deferred): [marker-deviation] Codex root marker is the persisted concurrent_session (SessionStart fork), not the spike's transcript_path (not stored on the binding row); stated in report, TRUST-POLICY and commit; fails closed to native when two roots read.
Task 7 (ht-z41e.7): minor (deferred): [mis-enrolled-side] a lone side binding first seen via a later event (not enrolled as fork) with the root binding gone reads as the root and is queue-targeted; narrow edge, no marker to detect it.
Task 7 (ht-z41e.7): minor (deferred): [missing-red-run] report states no RED run was recorded; tests written with the implementation.
Task 7 (ht-z41e.7): minor (deferred): [missing-evidence] report carries no structured evidence array or sourceTree; commands and pass counts are given only in prose.
Task 7 (ht-z41e.7): minor (deferred): [queue-level-side-composer-test] the native-fallback-on-side-composer acceptance is covered only by Task 1's existing native.rs test, not a queue-runtime-level test.
Task 8 (ht-z41e.8): minor (deferred): [no-live-run] side-queue and side-native-defer are untested against a real Codex; side_view_changed may need loosening if the root turn redraws the side view (Task 10 live run)
Task 8 (ht-z41e.8): minor (deferred): [spec-wording-drift] bead text mentions the S2 busy-sibling keep outcome, but the implementation follows ht-z41e.7's S2 drop result as recorded in TRUST-POLICY/AGENTS
Task 9 (ht-z41e.11): minor (deferred): [weak-integration-coverage] tests/integration/seat_config_sweep.rs flow 1 asserts the store value (effective_value), not a seat-differentiated queue selection; covered per seam by Task 5 test
Task 9 (ht-z41e.11): minor (deferred): [weak-integration-coverage] tests/integration/seat_config_sweep.rs flow 3 native half only checks read_composer on a side placeholder, not an actual wake deferral; per-seam test lives in host::native (Task 1)
Task 9 (ht-z41e.11): minor (deferred): [redundant-assertion] tests/integration/seat_config_sweep.rs wakes_on helper calls wake() twice with a near-tautological second assertion
Metrics: completions — review clean 9 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 6 · cancelled 0
Task 13 (ht-z41e.24): minor (deferred): [inline-sql-duplication] src/store/seat_config.rs:491 final-retirement check is inline SQL rather than a shared helper from src/store/control/retirement.rs; acceptable but a later refactor could share it
Task 14 (ht-z41e.25): minor (deferred): [doc-drift] scripts/wake-e2e.py bindings_idle docstring says settled() stands in for bindings without a turn row, but the code calls only wait_idle()
Task 14 (ht-z41e.25): minor (deferred): [unverified-assumption] scripts/wake-e2e.py REFUSED_OUTCOMES counts timed_out as a pre-send refusal (report states it as an assumption)
Task 11 (ht-z41e.21): minor (deferred): [tdd-no-red-run] report states implementation preceded tests and no RED run was recorded
Task 11 (ht-z41e.21): minor (deferred): [retry-journal-downgrade] src/cli/retry.rs journaled check-in carrying auxiliary_session is not stripped against a daemon lacking seat.auxiliary_session_v1 (deny_unknown_fields refusal); only on daemon downgrade between record and retry
Task 11 (ht-z41e.21): minor (deferred): [cleanup-inspection] stop-run-processes --check could not run; leak check done by hand with ps

## amendments
ht-z41e.8:     race, not a task problem. No requirement changed and no amendment or triage
ht-z41e.8:     as written, including the amended per-case queue mode (side-queue needs --
ht-z41e.8:     3. Scope is unchanged. Read the queue-intent root and side session fields
