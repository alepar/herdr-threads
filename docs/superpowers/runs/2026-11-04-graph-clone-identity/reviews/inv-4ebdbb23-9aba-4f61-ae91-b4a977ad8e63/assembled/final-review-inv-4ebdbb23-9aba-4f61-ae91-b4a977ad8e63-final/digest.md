## diffstat

 TRUST-POLICY.md                                    |  169 ++-
 docs/programmatic-service-client.md                |   46 +
 .../2026-11-04-graph-clone-identity-design.md      |  283 ++++
 ...lone-identity-roast-design-1-step-back-keys.txt |    5 +
 ...raph-clone-identity-roast-design-1-step-back.md |    7 +
 ...26-11-05-graph-clone-identity-roast-design-1.md |  144 ++
 ...26-11-05-graph-clone-identity-roast-design-2.md |   65 +
 .../coverage-ledger.md                             |   40 +
 .../coverage-round-1-requirements.md               |   14 +
 .../findings-ledger-design.md                      |    9 +
 .../findings-overrides-design.md                   |    0
 .../2026-11-04-graph-clone-identity/friction.md    |    2 +
 .../runs/2026-11-04-graph-clone-identity/run.md    |   36 +
 .../2026-11-04-graph-clone-identity/task-tree.json |  541 +++++++
 docs/superpowers/specs/INDEX.md                    |    1 +
 integrations/claude/mod/hooks/register.js          |    5 +-
 .../claude/mod/tests/named_watch_env.test.ts       |   15 +
 integrations/hermes/README.md                      |    3 +
 integrations/hermes/__init__.py                    |    2 +-
 integrations/hermes/test_bridge.py                 |   23 +-
 migrations/node/0016_graph_clone_identity.sql      |  115 ++
 .../node/0016_graph_clone_identity_after.sql       |    7 +
 scripts/wire-compat-baseline.txt                   |    6 +
 src/cli/caller.rs                                  |   10 +
 src/cli/caller_location.rs                         |   49 +-
 src/cli/commands.rs                                |    2 +
 src/cli/hook.rs                                    |   71 +-
 src/cli/journal.rs                                 |   43 +-
 src/cli/launch.rs                                  |   13 +-
 src/cli/me.rs                                      |   19 +-
 src/cli/mod.rs                                     |   26 +-
 src/cli/retry.rs                                   |    1 +
 src/cli/site_capability.rs                         |  118 +-
 src/daemon/transport/service_connection.rs         |   37 +-
 src/harness/bridge.rs                              |    6 +-
 src/harness/context.rs                             |   14 +
 src/harness/hermes.rs                              |    3 +-
 src/host/mod.rs                                    |    5 +
 src/identity/reconcile.rs                          |   10 +-
 src/identity/repair.rs                             |   20 +-
 src/ports.rs                                       |   42 +-
 src/protocol/authority.rs                          |    3 +
 src/protocol/capabilities.rs                       |    4 +
 src/protocol/commands.rs                           |   61 +
 src/protocol/ids.rs                                |   30 +
 src/protocol/output.rs                             |   21 +-
 src/protocol/output_compact.rs                     |   18 +-
 src/protocol/pagination.rs                         |    4 +-
 src/protocol/results.rs                            |   60 +
 src/protocol/service.rs                            |  299 +++-
 src/service/dispatch.rs                            |    7 +
 src/service/kicks.rs                               |    8 +
 src/service/launch_gate.rs                         |    1 +
 src/store/archival.rs                              |   18 +-
 src/store/continuity_keys.rs                       |  654 +++++++++
 src/store/control.rs                               |    5 +-
 src/store/control/invitation.rs                    |   11 +-
 src/store/control/retirement.rs                    |  221 ++-
 src/store/invitation_due.rs                        |    7 +-
 src/store/kept_continuity.rs                       |    9 +-
 src/store/materialization.rs                       |    7 +-
 src/store/materialize.rs                           |    2 +
 src/store/mod.rs                                   |    7 +-
 src/store/poke.rs                                  |    8 +-
 src/store/queries.rs                               |   16 +-
 src/store/receipts.rs                              |    7 +-
 src/store/schema.rs                                |   29 +-
 src/store/seats.rs                                 |    2 +
 src/store/seats/continuity.rs                      |  555 ++++++--
 src/store/seats/cooperative.rs                     |   43 +-
 src/store/seats/key_verdict.rs                     |  457 ++++++
 src/store/seats/observation.rs                     |   70 +-
 src/store/seats/operator.rs                        |   63 +-
 src/store/seats/publication.rs                     |  191 ++-
 src/store/seats/resolution.rs                      |   61 +-
 src/store/seats/reversible.rs                      |  602 ++++++++
 src/store/service_controls.rs                      |   44 +-
 src/store/topology_handoff/attempts.rs             |    1 +
 src/sync/facts/mod.rs                              |    2 +
 src/sync/facts/seat.rs                             |  101 +-
 src/test_support/mod.rs                            |    6 +
 tests/cli/caller_location.rs                       |    7 +-
 tests/cli/cooperative.rs                           |   10 +
 tests/cli/handoff.rs                               |   22 +
 tests/cli/hook.rs                                  |  380 +++++
 tests/cli/hook_local_evidence.rs                   |    2 +
 tests/cli/hook_neutral.rs                          |    2 +
 tests/cli/hook_pull_lazy.rs                        |    2 +
 tests/cli/human.rs                                 |  117 ++
 tests/cli/journal.rs                               |   84 +-
 tests/cli/launch.rs                                |  128 ++
 tests/cli/nudge_threshold.rs                       |    2 +
 tests/cli/read_cost_names.rs                       |    2 +
 tests/cli/summary.rs                               |    2 +
 tests/cli/topology_handoff.rs                      |    8 +
 tests/cli/topology_recover.rs                      |    2 +
 tests/cli/watch.rs                                 |    2 +
 tests/combined.rs                                  |    2 +
 tests/contracts.rs                                 |    2 +
 tests/daemon/control.rs                            |    2 +
 tests/daemon/harness_evidence.rs                   |    2 +-
 tests/daemon/inherited_name.rs                     |    2 +
 tests/daemon/lifecycle.rs                          |    1 +
 tests/daemon/multi_host.rs                         |    5 +
 tests/daemon/node_layout.rs                        |    2 +
 tests/daemon/self_retire.rs                        |    3 +
 tests/daemon/service_session_capability.rs         |   80 +-
 tests/fixtures/golden/help/setup.txt               |    3 +-
 tests/handoff_topology_authority.rs                |    1 +
 tests/harness/agy_delivery.rs                      |    3 +
 tests/harness/agy_wake.rs                          |    1 +
 tests/hc_messaging.rs                              |    1 +
 tests/hook_entrypoint.rs                           |    4 +
 tests/host/bootstrap_recovery.rs                   |    3 +-
 tests/identity/kept_continuity_rules.rs            |   28 +
 tests/identity/reconcile.rs                        |    2 +
 tests/integration/lazy_attention.rs                |    3 +
 tests/integration/notice_wake.rs                   |    1 +
 tests/integration/trust_policy.rs                  |   71 +
 tests/lifecycle_ux.rs                              |    1 +
 tests/named_seats.rs                               |    1 +
 tests/protocol/capabilities.rs                     |   23 +
 tests/protocol/error_class.rs                      |    1 +
 tests/protocol/output.rs                           |   44 +
 tests/service.rs                                   |    4 +
 tests/service/composition.rs                       |    4 +
 tests/service/continuity_key_session.rs            |  580 ++++++++
 tests/service/cooperative.rs                       |   12 +-
 tests/service/crash_matrix.rs                      |    4 +
 tests/service/delivery_pull.rs                     |    5 +
 tests/service/graph_d2.rs                          |    6 +-
 tests/service/graph_send.rs                        |    3 +-
 tests/service/lazy_hook_pull.rs                    |    1 +
 tests/service/mod_ack_dispatch.rs                  |    2 +
 tests/service/no_wake_reports.rs                   |    1 +
 tests/service/observation_lane.rs                  |    3 +
 .../service/presentation_owner_budget_controls.rs  |    2 +
 tests/service/resolution.rs                        |   11 +
 tests/service/reversible_session.rs                |  269 ++++
 tests/store/catch_up.rs                            |    1 +
 tests/store/continuity_key_decisions.rs            | 1505 ++++++++++++++++++++
 tests/store/continuity_keys.rs                     | 1147 +++++++++++++++
 tests/store/control.rs                             |  195 ++-
 tests/store/cooperative_checkin.rs                 |   15 +-
 tests/store/facade.rs                              |    6 +
 tests/store/fact_convergence.rs                    |    1 +
 tests/store/fact_convergence_seat.rs               |   79 +-
 tests/store/first_contact.rs                       |    3 +
 tests/store/kept_continuity.rs                     |    1 +
 tests/store/locator_hint.rs                        |    1 +
 tests/store/named_allocation.rs                    |    1 +
 tests/store/named_continuity.rs                    |   25 +-
 tests/store/named_presence.rs                      |    1 +
 tests/store/named_restore.rs                       |    1 +
 tests/store/notice_replay.rs                       |    1 +
 tests/store/pane_form_refusal.rs                   |    1 +
 tests/store/queries.rs                             |    2 +
 tests/store/restored_pane.rs                       |  316 +++-
 tests/store/retention.rs                           |    1 +
 tests/store/reversible_facts.rs                    |  409 ++++++
 tests/store/reversible_reads.rs                    |  952 +++++++++++++
 tests/store/reversible_retirement.rs               | 1254 ++++++++++++++++
 tests/store/schema.rs                              |  259 ++++
 tests/store/service_reads.rs                       |   37 +
 tests/store/service_send.rs                        |    3 +
 tests/store/topology_attachment_guard.rs           |    6 +
 tests/support/peer_wire_freeze.rs                  |   14 +
 tests/sync/facts_seat.rs                           |   57 +-
 tests/sync/facts_seat_reversible.rs                |  978 +++++++++++++
 tests/sync/peer_wire_fixtures.rs                   |    9 +-
 170 files changed, 14615 insertions(+), 379 deletions(-)

## generated-input provenance

## Generated-input provenance
Captured range: cf13ed76c1c2871b321ad290d03229cb785c400e..0755e609de0328e6d73584dd65eb2854e1fd7bf7
Selection: complete; selected product/input paths: 170; omitted historical payloads: 0.
diffstat.txt describes selected product/input paths. Reports, findings and collection metadata remain ordinary review inputs.
- Outside captured range: 34 informational entries (neither selected nor waived); the first 20 are listed; the full list is review-inputs.json outOfRange.
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:78: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:106: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:109: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:111: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:112: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:116: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:117: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 0755e609de0328e6d73584dd65eb2854e1fd7bf7:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:78: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 0755e609de0328e6d73584dd65eb2854e1fd7bf7:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:106: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 0755e609de0328e6d73584dd65eb2854e1fd7bf7:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:109: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 0755e609de0328e6d73584dd65eb2854e1fd7bf7:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:111: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 0755e609de0328e6d73584dd65eb2854e1fd7bf7:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:112: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 0755e609de0328e6d73584dd65eb2854e1fd7bf7:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:116: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): 0755e609de0328e6d73584dd65eb2854e1fd7bf7:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:117: malformed collection record: record outside approvals or invalid prefix
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-264d-4506-beaa-307c9600bb8d: invalid failed assembly archive: failed bounded archive contains successful assembly or split claims
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58 inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f: invalid failed assembly archive: failed bounded archive contains successful assembly or split claims
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: incomplete claimed collection
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: incomplete claimed final report
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: invalid failed assembly archive: missing bounded evidence: final-review-inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb.md
- Outside captured range (informational; neither selected nor waived): cf13ed76c1c2871b321ad290d03229cb785c400e:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:62 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: incomplete assembled bundle: digest.md
Open each report/archive at its recorded revision. Omission supplies no finding disposition or verdict.

## report summaries

### task 1
Status: none
Assumptions:
- No bead amendments or comments exist (`bd comments` is empty); the bead's and the brief's acceptance agree.
- Temp sweep script placed under `/tmp/sp-44bdca52/ht-ma6i.5` rather than the session scratchpad the brief names, because the dispatch's temp rule takes precedence. Removed afterwards.
- `ContinuityKey`'s comment in `ids.rs` is `//` rather than the brief's `///`: rustc's `unused_doc_comments` refuses a doc comment on a macro invocation under `-D warnings`.
- `service_connection.rs` no longer needs `ServiceOperation` or `SERVICE_SESSION_CAPABILITY_V2`; its test module got them through `super::*`, so `tests/daemon/service_session_capability.rs` imports both directly.
Concerns:
- `ReserveContinuityKey.seat`, `ContinuityKeyReservation.seat` and `ContinuityAlert.seat` are plain `Option` without `skip_serializing_if`, as the brief's fixtures require (`"seat": null` round-trips).
- `service_controls` reaches the clone identity refusal for the four mutations only after the replay lookup and `ensure_reserved_author`. A pre-existing `operations` row under the same key would answer its stored result or `OperationPayloadMismatch` rather than `Unsupported`. That is the brief's pla
### task 2
Status: none
Assumptions:
- Brief vs bead on `record_conflict`. The brief's "same key and kind: Unchanged, nothing written" would leave a stale `owner_seat_id` when the same refusal is re-judged against a new owner, and the bead (and spec 4.2) says a re-judge "updates key, kind, seat and owner", with an alert only on a key o
- Bindable seat: the host kind is an allow-list (`h.kind='herdr'`), so any future kind is refused too; spec 4.2 says "on a Herdr host". A seat with no host row (remote) reads `InvalidRequest`, through a LEFT JOIN, not `NotFound`.
- A re-reserve naming a retired seat already bound to the key is refused (`InvalidRequest`), under "a retired seat is refused". A replay of the original operation key still returns the journaled result.
- `another_author` fixtures insert a second `service_authors` row with foreign keys off: the table holds one author per node, so no second real author can exist on one daemon. That is the only way to exercise the refusal; with one author per node, this refusal looks unreachable on a live daemon toda
Concerns:
- Test placement: `rebind_clears_the_targets_conflict_row` lives in `tests/store/control.rs` as `continuity_keys_rebind_clears_the_targets_conflict_row`, not in `tests/store/continuity_keys.rs`, because it needs that file's private published-baseline fixtures. The `continuity_keys` filter still sele
- Gap for a follow-up: spec 4.2 also clears a hold-bearing row when the operator "creates a fresh seat on the target". The brief's chokepoint covers rebinds only, and that fresh-seat branch of `mutate_operator` does not call `rebind_unresolved_seat`, and no later task in the plan names it. Not added
- The `TableFull` daemon log the spec asks for belongs to the caller of `record_conflict`, a later task; this module only reports it.
### task 3
Status: none
Assumptions:
none
Concerns:
- `continuity_keys::release_for_final` has no production caller yet: a finally retired seat keeps its key row until ht-ma6i.3 lands, so until then such a key reads `key_retired_reversibly`. The ledger task left this to the finalization path.
- The hook does not send the key yet (client leaf). Until it does, no production caller reaches a keyed path, so a standing conflict row cannot arise either. `api_failure` will surface the conflict text for `TargetUnresolved`; spec says a conflict is quiet in the hook, which the client leaf must dec
- The acceptance item "a later pane close is reversible" is asserted as `key_of_seat` returning the key, per the brief's scope note.
- `kept_keyed_claim_is_decided_after_lag_with_no_further_event` writes the key into the kept row with SQL, because no client sends it yet. The client leaf should replace that with a real keyed hook.
- `src/store/seats/continuity.rs` grew by about 300 lines of key helpers. They could move beside the judge, but the brief named this file and `key_verdict.rs` must stay free of database access.
### task 4
Status: none
Assumptions:
- No bead amendments or comments existed; the bead's acceptance and the brief agree.
- Health: the brief names `src/daemon/health.rs`, but its unresolved listing comes from `queries::unresolved_seat_summary`, so the exclusion is in `src/store/queries.rs` and health.rs is unchanged. `tests/store/service_reads.rs` is the narrowed test.
- Hold arming (`observation.rs` near 699 and 2894) also composes the waiting fragment, beyond the brief's list: an unretired seat protects no pane, so it should not arm a hold through the "unresolved" branch.
- Files beyond the brief: `src/store/queries.rs`, `src/store/control.rs`, `src/store/seats/publication.rs`, `src/test_support/mod.rs` (`operator_actor`, so a suite outside the crate can finalize as the operator), `tests/store/fact_convergence_seat.rs`, `tests/store/restored_pane.rs`, `tests/store/co
- `unretire` takes no `context`, since nothing it does needs one, unlike the brief's signature. `finalize` returns `()` and `operate` wraps the result.
- Unretired finalization copies the row's closure target and boot but not its evidence: that is an ordinary final retirement decided now, and the evidence described the earlier close. Reversible finalization copies both evidence columns, as acceptance requires.
- Operator finalization records `operator_retire` with the audit label, not `service_finalize`, because the actor is the operator.
- The deadline shift does not move `available_at`, as the spec names deadlines only. Consequence: after a long freeze a soft poke comes earlier than the same fraction of the remaining time would.
- A logical receipt with a stored row whose deadline is NULL also gets the projection plus the shift, because the effective read would otherwise project an unshifted deadline.
Concerns:
- Two writers of `retirements` remain, `insert_retirement_job` and `insert_replicated_retirement_job` (the `seat.retired` applier, whose row has no closure inputs and origin `replicated`). They sit side by side in `control/retirement.rs` and both release the key, so this is one choke point in spirit
- Route coverage: `seat retire --self` has no case in `every_retirement_route_releases_the_key`, since it is for named seats, which can never hold a key. It goes through `retire_seat` and so `insert_retirement_job`. The acceptance list's "a relaunch with the same key" passes the key in the command, 
- The session suite makes a seat reversible by writing the fence's columns over SQL, because the fixture host is unavailable; the real fence is covered by the store suites.
- `has_nonretired_seat` still counts a waiting seat, so a new incarnation on a host holding only waiting seats arms a hold and lifts it at the end of the pass, rather than taking the atomic empty-pass path. `an_unretired_seat_is_outside_the_reconciliation_plan` confirms the lift.
- `src/store/seats/reversible.rs` holds the local half only; `.7` adds the facts, `project_receipt` and the invitee apply. Until then a peer cannot produce a window fact, so `finalization_advances_retired_seq` simulates one with a `prepared_recipients` row.
### task 6
Status: none
Assumptions:
- The bead has no comments or amendments, and it agrees with the brief.
- `alerts_page` takes `instance` as well as the brief's `(conn, page)`, because every cursor binds its instance.
- `src/cli/human.rs` is unchanged, despite the bead's file list. It has no `seat inspect` form: `render` returns None and the caller emits the text form, and the brief's line 254 is `seat list`. The shared line therefore covers "compact and human", and `seat_inspect_human_output_prints_the_key_line_
- `src/protocol/output.rs` and `src/protocol/pagination.rs` were outside the brief's list and were needed: the generic `seat inspect` text is rendered in output.rs, and the feed needs its own cursor scope so another read's cursor is refused.
- The key line carries no conflict or bound_by, per the brief's format; `--json` has them. Spec 4.7 says "4 bytes plus the key and owner"; with the separator and newline it is 6, as the commit states.
- A conflict or alert whose host endpoint is not a canonical locator is `StoreCorrupt`. Recording a conflict needs a located host, so this should not occur; if it did, `seat inspect` of that seat would fail.
Concerns:
- `PersistentServiceClient` still registers v2 and its `result_matches` knows no v3 result, so it cannot call any of the five operations. The plan left `src/client/service.rs` alone ("no v3 caller yet"), and the doc says so plainly. If the graph service uses this client rather than its own framing, 
- The feed has no resume cursor: the last page has none, so a poller rereads up to 256 rows from the start and skips ordinals it has seen, as the doc states. A resume-after-ordinal parameter would be a protocol change.
- The doc states UnretireSeat and FinalizeRetirement results and refusals from the spec and plan Task 4 (`Unauthorized`, `NotFound`). They answer `Unsupported` on this branch until ht-ma6i.3 lands, so those rows are unverified here. Plan Task 4 also says to narrow the same Unsupported test "to Conti
- `retirement_state` is documented here and filled by ht-ma6i.10; it is `None` on this branch.
### task 7
Status: none
Assumptions:
- Requirements: bead checklist plus the 10:21 comment (no amendment to acceptance). The comment predicted REUSE or a `stack:` base; the script actually printed REBUILT with no stack commit because ht-ma6i.2 landed between its reads. `SEAT_CONTINUITY_KEY_V1` confirmed at src/protocol/capabilities.rs:
- "No new model-visible text" overrides the brief's reading that line 388 already keeps a conflict silent: `api_failure` would have put the detail into lifecycle context, so a key conflict is now Quiet, classified by the prefix `KEY_CONFLICT_CAUSE` shared with the store's formatter. This also silenc
- `CooperativeCheckIn` gains the field too, though the brief names two mutations: the bead requires the key on the check-in, and a lifecycle check-in is durable.
- `commands.rs:811,952` stay `None`. `into_command` has no client for the capability read. `seat resolve` is served at mod.rs:1306 and a plain check-in by the bridge, and the spec feeds the plain CLI only on its seat-resolving path. So a plain `check-in` sends no key.
- Launch and handoff scrub: Herdr `agent.start` carries argv only and threads composes no environment there, so the scrub is the shared list (daemon, shim) plus `peer_launcher::child_env`'s allowlist. The two named tests assert those lists rather than a started agent's environment.
- Plain CLI top level: `caller::active_is_top_level`; a refused origin counts as not top-level.
Concerns:
- **Integration moved past base and this commit conflicts on merge, in TRUST-POLICY.md only.** Tip 9fc71db09 (ht-ma6i.3 landed); `git merge-tree` reports one content conflict there. I did not rebase, as the template forbids it for a branch reported stacked, although this one carries no parent commit
- `key_bound_elsewhere_mints_no_seat_on_plain_cli` uses a scripted daemon reply; the real refusal is covered at store level by ht-ma6i.2. No end-to-end plain CLI run against a real daemon, and the Hermes and mod paths ran only in their own test harnesses, never a live harness.
- The mod assumes `process.spawn`'s `env` overlays the inherited environment, as the named and terminal forwards imply: a keyed pane seat now passes `env` where it passed none. I found no doc for this. If `env` replaced the environment, a keyed pane seat's watch child would lose `HERDR_PANE_ID`. Wor
- Conflict classification is by string prefix on `ApiError.detail`; a typed code would be sturdier but would change ht-ma6i.2's wire answer.
- `tests/cli_env_read_scan.rs` NAMES does not list the new variable, so the "one helper" is not policed there. Left as the brief said (add only if it fails); one line if wanted.
- `subagent_sends_nothing` (registered fixture) can pass with no keyed command sent at all; `subagent_event_presents_no_key` is the test with teeth.
- `AGENTS.md` "What we do today" not updated (not asked).
- Hermes asset tests are thread-flaky under plain `cargo test`; worth a follow-up bead.
### task 8
Status: none
Assumptions:
- Amendment used: the bead comment of 2026-11-05 10:27 (re-dispatch, setup path, plan-only files in scope, do not run remove-task-worktree on exit 4).
- "Multinode through config_mixed_versions' shape" is read as the shape of the check (a node parks unknown kinds, later kinds apply), run on two-registry stores with `fact_exchange`, not as a new daemon cluster test.
- Home-node window and resume tests author `seat.unretired` directly because the integration `Node` clock stands still, so a real un-retire would shift by zero. The real un-retire's shift is covered by `deadlines_keep_remaining_time...` from ht-ma6i.3, its authoring by published_seat_authors_both_fa
- A local row with closure inputs that receives `seat.retired` keeps them in its job; one with none gets the replicated job, per "from the row".
Concerns:
- Changes to existing tests the brief did not name: `held_kinds_list_matches_the_spec` (two new names), `seat_clone_identity_payloads_round_trip` (the "not registered yet" assertion goes), `generator_kind_table_covers_builtins` (counts `UNCHAINED_KINDS`), two `unretire` calls in reversible_retiremen
- The peer-wire freeze chain cannot take a new kind: `current_encoders_reproduce_2_1_1_fixtures` pins it to the 2.1.1 golden, so the two kinds are in a new `UNCHAINED_KINDS` and have no frozen envelope. A later freeze will not cover them until that test is reworked. Follow-up.
- Removed `clone_identity_not_served` from src/store/service_controls.rs. It is ht-ma6i.3's leftover, dead on `task-ht-ma6i.3` and failing clippy -D warnings there too. If .3's fix pass removes it as well the merge is clean; if .3 keeps it the merge may need a trivial resolution.
- Accepted in the policy text: in rev/unret/final, a node that parked the facts never shifted the deadline of an obligation the final fact then settled, so `deadline_at` on that terminal row differs from a node that knew the kinds by the window. The snapshot compares deadlines only while the seat ca
- A fact applied during the window has its deadline shifted by the whole window on un-retire, including the part before it arrived, so it gets slightly more time. Same shape as spec 4.3's uniform shift; left as is.
### task 10
Status: none
Assumptions:
- No bead amendments or comments exist (`bd comments`: none). The bead and the brief agree except as listed here.
- The brief names `output_compact.rs:919` (`seat_row`) and `human.rs:254` (`seats`) as the print sites. Those render the `seats` list from `SeatSummary`, which has no `retirement_state`. Spec 4.5 and 4.7 and the bead's "owns" line put the field and its text on `seat inspect`, so the line is there, i
- The brief says to mount the new file with `#[path]` in `src/store/seats/reversible.rs`. It is mounted from `tests/store/reversible_retirement.rs` instead, as a child module, so it can use that file's private fixtures unchanged.
- The brief's filesTouched does not list `src/protocol/output.rs`; it is the only place the `seat inspect` text is built.
- "Spec 4.7's 9 bytes more than `retired`" does not match the string the spec itself gives: ` (reversible)` is 13 bytes. The line as shipped is 28 bytes, stated in the commit message per AGENTS.md (f).
- "Unread on reattach" follows spec 4.3: "send preparation ... leaves the seat out of a window message's audience, which is the agreed 'no obligations from the window' and which .10 pins as unread history".
Concerns:
none

## interfaces and seams

Exported signatures:
src/cli/caller.rs: pub(super) fn active_is_top_level() -> bool {
src/cli/caller_location.rs: pub(crate) fn caller_continuity_key(
src/cli/caller_location.rs: pub(crate) fn may_present_key(&self, top_level: bool) -> bool {
src/cli/caller_location.rs: pub(crate) fn is_own_pane(&self, target: &HostTargetId) -> bool {
src/cli/site_capability.rs: pub(crate) fn presented_continuity_key<C: LocalClient + ?Sized>(
src/cli/site_capability.rs: pub(crate) fn own_pane_continuity_key<C: LocalClient + ?Sized>(
src/harness/context.rs: pub(crate) fn with_continuity_key(
src/harness/context.rs: pub(crate) fn continuity_key(&self) -> Option<&crate::protocol::ids::ContinuityKey> {
src/host/mod.rs: pub const CONTINUITY_KEY_ENV: &str = "HERDR_THREADS_CONTINUITY_KEY";
src/ports.rs: pub struct RetirementInputs {
src/ports.rs: pub enum RetirementStart {
src/ports.rs: pub fn into_job(self) -> Option<RetirementJob> {
src/protocol/authority.rs: pub const COOPERATIVE_CONTINUITY_KEY_PROVENANCE: &str = "cooperative_continuity_key";
src/protocol/capabilities.rs: pub const SEAT_CONTINUITY_KEY_V1: &str = "seat.continuity_key_v1";
src/protocol/output_compact.rs: pub(super) fn inspect_key_line(key: &SeatContinuityKey) -> String {
src/protocol/output_compact.rs: pub(super) fn inspect_retirement_line(state: SeatRetirementState) -> Option<&'static str> {
src/protocol/results.rs: pub const KEY_CONFLICT_CAUSE: &str = "continuity key conflict ";
src/protocol/results.rs: pub enum ContinuityConflictKind {
src/protocol/results.rs: pub enum ContinuityKeyBinding {
src/protocol/results.rs: pub struct ContinuityKeyConflict {
src/protocol/results.rs: pub struct SeatContinuityKey {
src/protocol/results.rs: pub enum SeatRetirementState {
src/protocol/service.rs: pub const SERVICE_SESSION_CAPABILITY_V3: &str = "service_session_v3";
src/protocol/service.rs: pub enum ServiceSessionLevel {
src/protocol/service.rs: pub fn from_capability(name: &str) -> Option<Self> {
src/protocol/service.rs: pub fn capability(self) -> &'static str {
src/protocol/service.rs: pub fn required_by(operation: &ServiceOperation) -> Self {
src/protocol/service.rs: pub struct ReserveContinuityKey {
src/protocol/service.rs: pub struct ReleaseContinuityKey {
src/protocol/service.rs: pub struct UnretireSeat {
src/protocol/service.rs: pub struct FinalizeRetirement {
src/protocol/service.rs: pub struct ServiceContinuityAlertsQuery {
src/protocol/service.rs: pub struct ContinuityKeyReservation {
src/protocol/service.rs: pub struct SeatUnretired {
src/protocol/service.rs: pub struct ContinuityAlert {
src/store/archival.rs: pub(crate) fn has_live_remote_member(tx: &Connection, thread: &str) -> Result<bool, ApiError> {
src/store/continuity_keys.rs: pub const MAX_CONFLICT_ROWS: i64 = 256;
src/store/continuity_keys.rs: pub const MAX_ALERT_ROWS: i64 = 256;
src/store/continuity_keys.rs: pub struct KeyRow {
src/store/continuity_keys.rs: pub struct ConflictFacts {
src/store/continuity_keys.rs: pub enum ConflictRecord {
src/store/continuity_keys.rs: pub struct ConflictRow {
src/store/continuity_keys.rs: pub fn lookup_key(conn: &Connection, key: &ContinuityKey) -> Result<Option<KeyRow>, ApiError> {
src/store/continuity_keys.rs: pub fn key_of_seat(conn: &Connection, seat: &SeatId) -> Result<Option<KeyRow>, ApiError> {
src/store/continuity_keys.rs: pub fn reserve(
src/store/continuity_keys.rs: pub fn release(
src/store/continuity_keys.rs: pub fn bind(
src/store/continuity_keys.rs: pub fn release_for_final(conn: &Connection, seat: &SeatId) -> Result<(), ApiError> {
src/store/continuity_keys.rs: pub fn record_conflict(
src/store/continuity_keys.rs: pub fn clear_conflicts_of_key(conn: &Connection, key: &ContinuityKey) -> Result<(), ApiError> {
src/store/continuity_keys.rs: pub fn clear_conflict_on_target(
src/store/continuity_keys.rs: pub fn clear_alert_only_if_owner_changed(
src/store/continuity_keys.rs: pub fn hold_kind(
src/store/continuity_keys.rs: pub fn clear_conflicts_absent_from(
src/store/continuity_keys.rs: pub fn conflict_of_key(
src/store/continuity_keys.rs: pub fn seat_view(conn: &Connection, seat: &SeatId) -> Result<Option<SeatContinuityKey>, ApiError> {
src/store/continuity_keys.rs: pub fn alerts_page(
src/store/continuity_keys.rs: pub(crate) fn kind_name<T: Serialize>(value: &T) -> Result<String, ApiError> {
src/store/control/retirement.rs: pub(crate) fn emit_seat_retired(
src/store/control/retirement.rs: pub(crate) fn emit_if_published<K: FactKind>(
src/store/control/retirement.rs: pub(crate) fn retirement_transition(
src/store/control/retirement.rs: pub(crate) fn insert_retirement_job(
src/store/control/retirement.rs: pub(crate) fn insert_replicated_retirement_job(
src/store/invitation_due.rs: pub(crate) const INVITATION_DUE_SELECTION: &str = concat!(
src/store/seats/continuity.rs: pub(super) type KeyConflictStash = std::cell::RefCell<Option<continuity_keys::ConflictFacts>>;
src/store/seats/continuity.rs: pub(super) struct C1Session<'a> {
src/store/seats/continuity.rs: pub(super) enum KeyCaller<'a> {
src/store/seats/continuity.rs: pub(super) struct KeyDecision {
src/store/seats/continuity.rs: pub(super) fn refuse(
src/store/seats/continuity.rs: pub(super) fn key_conflict_cause(
src/store/seats/continuity.rs: pub(super) fn key_decision(
src/store/seats/continuity.rs: pub(super) fn record_stashed_conflict(
src/store/seats/continuity.rs: pub(super) fn settle_key(
src/store/seats/continuity.rs: pub(super) fn guard_continuity_pin(
src/store/seats/cooperative.rs: pub fn check_in_digest(command: &CheckIn) -> Result<[u8; 32], ApiError> {
src/store/seats/key_verdict.rs: pub struct KeyFacts<'a> {
src/store/seats/key_verdict.rs: pub enum KeyState<'a> {
src/store/seats/key_verdict.rs: pub enum BoundSeatState {
src/store/seats/key_verdict.rs: pub enum KeyVerdict {
src/store/seats/key_verdict.rs: pub fn judge(facts: &KeyFacts<'_>) -> KeyVerdict {
src/store/seats/key_verdict.rs: pub fn only_alerts(kind: ContinuityConflictKind) -> bool {
src/store/seats/publication.rs: pub fn materialize_seat_retired_reversible(
src/store/seats/publication.rs: pub fn materialize_seat_unretired(
src/store/seats/resolution.rs: pub(super) fn resolution_recovery_refusal(
src/store/seats/reversible.rs: pub(crate) fn is_frozen(conn: &Connection, seat: &SeatId) -> Result<bool, ApiError> {
src/store/seats/reversible.rs: pub(crate) fn retirement_state(
src/store/seats/reversible.rs: pub(crate) enum RowState {
src/store/seats/reversible.rs: pub(crate) enum FinalizeBy<'a> {
src/store/seats/reversible.rs: pub(crate) fn row_state(conn: &Connection, seat: &SeatId) -> Result<Option<RowState>, ApiError> {
src/store/seats/reversible.rs: pub(crate) fn insert_fact_row(
src/store/seats/reversible.rs: pub(crate) fn insert_row(
src/store/seats/reversible.rs: pub(crate) fn delete_row(tx: &Transaction<'_>, seat: &SeatId) -> Result<(), ApiError> {
src/store/seats/reversible.rs: pub(crate) fn delete_unretired_row(tx: &Transaction<'_>, seat: &SeatId) -> Result<(), ApiError> {
src/store/seats/reversible.rs: pub(crate) fn shift_pending_deadlines(
src/store/seats/reversible.rs: pub(crate) fn unretire(
src/store/seats/reversible.rs: pub(crate) fn unretire_by_fact(
src/store/seats/reversible.rs: pub(crate) fn finalize_by_fact(
src/store/seats/reversible.rs: pub(crate) fn finalize(
src/sync/facts/seat.rs: pub struct SeatRetiredReversiblePayload {
src/sync/facts/seat.rs: pub struct SeatUnretiredPayload {
src/sync/facts/seat.rs: pub struct SeatRetiredReversible;
src/sync/facts/seat.rs: pub struct SeatUnretired;
src/test_support/mod.rs: pub fn operator_actor() -> crate::protocol::authority::OperatorActor {

New files:
docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-04-graph-clone-identity-design.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-05-graph-clone-identity-roast-design-1-step-back-keys.txt
docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-05-graph-clone-identity-roast-design-1-step-back.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-05-graph-clone-identity-roast-design-1.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-05-graph-clone-identity-roast-design-2.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/coverage-ledger.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/coverage-round-1-requirements.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/findings-ledger-design.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/findings-overrides-design.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/friction.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/run.md
docs/superpowers/runs/2026-11-04-graph-clone-identity/task-tree.json
migrations/node/0016_graph_clone_identity.sql
migrations/node/0016_graph_clone_identity_after.sql
src/store/continuity_keys.rs
src/store/seats/key_verdict.rs
src/store/seats/reversible.rs
tests/service/continuity_key_session.rs
tests/service/reversible_session.rs
tests/store/continuity_key_decisions.rs
tests/store/continuity_keys.rs
tests/store/reversible_facts.rs
tests/store/reversible_reads.rs
tests/store/reversible_retirement.rs
tests/sync/facts_seat_reversible.rs

Seam contract beads:
ht-ma6i.5 Seam contract: clone identity schema delta 16 and protocol shapes

## deferred minors

Task 2 (ht-ma6i.8): minor (deferred): [missing-clear-path] spec 4.2 fresh-seat branch of mutate_operator (src/store/seats/operator.rs) does not call clear_conflict_on_target; implementer-reported gap, no later bead names it
Task 2 (ht-ma6i.8): minor (deferred): [unwired-helper] release_for_final, hold_kind, clear_alert_only_if_owner_changed and record_conflict have no production caller yet (src/store/continuity_keys.rs); left to later leaf tasks, TableFull daemon log belongs to the caller
Task 6 (ht-ma6i.9): minor (deferred): [doc-drift] docs/programmatic-service-client.md ContinuityAlerts row lists InvalidBudget/InvalidCursor but alerts_page returns InvalidRequest for an invalid page request
Task 6 (ht-ma6i.9): minor (deferred): [missing-v3-client] PersistentServiceClient has no v3 surface (documented in the doc; no v3 caller in this epic's scope)
Task 3 (ht-ma6i.2): minor (deferred): [unreachable-in-production] continuity_keys::release_for_final has no production caller until ht-ma6i.3, so a finally retired seat's key reads key_retired_reversibly meanwhile (src/store/continuity_keys.rs)
Task 3 (ht-ma6i.2): minor (deferred): [client-leaf-gap] hook does not send the key yet, so keyed paths are reachable only from tests; the kept-claim lag test writes the key by SQL (tests/integration/trust_policy.rs); the client leaf must replace it and decide whether a conflict is quiet in the hook
Task 3 (ht-ma6i.2): minor (deferred): [file-size-growth] src/store/seats/continuity.rs grew by about 300 lines of key helpers that could live beside the judge
Task 3 (ht-ma6i.2): minor (deferred): [tdd-order] tests were written alongside the code rather than red-first; two tests (claimless_resolve_records_a_hold_bearing_conflict, a_claim_with_a_free_key_decides_as_a_keyless_claim) were never run red
Task 4 (ht-ma6i.3): minor (deferred): [tdd-gap] report admits production code was written before tests; Kills: comments on new tests were not all observed failing
Task 4 (ht-ma6i.3): minor (deferred): [two-insert-sites] src/store/control/retirement.rs:283,306 retirements has two writers (insert_retirement_job, insert_replicated_retirement_job); both release the key, but the choke point is two functions
Task 4 (ht-ma6i.3): minor (deferred): [missing-route-test] seat retire --self has no case in every_retirement_route_releases_the_key (named seats cannot hold keys)
Task 4 (ht-ma6i.3): minor (deferred): [fixture-substitution] session suite makes a seat reversible via raw SQL; the real fence is covered only by store suites
Task 4 (ht-ma6i.3): minor (deferred): [stale-hold-count] has_nonretired_seat still counts a waiting (unretired) seat, so a new incarnation on a host holding only waiting seats arms then lifts a hold instead of the atomic empty-pass path
Task 4 (ht-ma6i.3): minor (deferred): [deferred-evidence-check] relaunch with the same key keeps the seat (B3) is untested for key judgment until ht-ma6i.2 lands; key is ignored on that path today
Task 4 (ht-ma6i.3): minor (deferred): [semantic-note] deadline shift does not move available_at, so soft pokes come earlier after a long freeze
Task 8 (ht-ma6i.7): minor (deferred): [freeze-chain-gap] tests/support/peer_wire_freeze.rs: the two new kinds sit in UNCHAINED_KINDS and have no frozen wire envelope; a later freeze must rework current_encoders_reproduce_2_1_1_fixtures (the report records this as a follow-up)
Task 8 (ht-ma6i.7): minor (deferred): [tdd-order-departure] the report says code was written before tests, with RED captured afterwards by disabling the implementation; the mixed-version absolute-state assertion was never run red
Task 8 (ht-ma6i.7): minor (deferred): [merge-conflict-risk] src/store/service_controls.rs: removal of clone_identity_not_served overlaps ht-ma6i.3 and may need a trivial resolution if .3 keeps it
Task 8 (ht-ma6i.7): minor (deferred): [accepted-limit-doc] in rev/unret/final, a node that parked the facts has a different terminal deadline_at than a node that knew them, and a window fact's deadline shifts by the whole window; both are recorded in the report and the policy text
Task 7 (ht-ma6i.6): minor (deferred): [fragile-string-classification] src/cli/hook.rs api_failure: KEY_CONFLICT_CAUSE prefix match on ApiError.detail also silences the hold-row cause for a keyless hook, which the model saw before
Task 7 (ht-ma6i.6): minor (deferred): [unverified-assumption] integrations/claude/mod/hooks/register.js:1146 process.spawn env overlay (not replace) semantics unverified; a keyed pane seat now passes env
Task 7 (ht-ma6i.6): minor (deferred): [missing-scan-entry] tests/cli_env_read_scan.rs NAMES omits HERDR_THREADS_CONTINUITY_KEY so the one-helper rule is not policed
Task 7 (ht-ma6i.6): minor (deferred): [merge-conflict] integration tip 9fc71db09 conflicts with this commit in TRUST-POLICY.md; resolve and rerun tests against ht-ma6i.3
Task 7 (ht-ma6i.6): minor (deferred): [flaky-test] harness::hermes::assets::tests Conflict failures under parallel cargo test, not run at base; repeated runs shown only
Task 7 (ht-ma6i.6): minor (deferred): [indirect-test] launch/handoff scrub tests assert scrub lists, not a started child's environment
Task 7 (ht-ma6i.6): minor (deferred): [missing-evidence] test logs removed; report prose is the only evidence location
Task 10 (ht-ma6i.10): minor (deferred): [print-site-deviation] retirement line is printed in src/protocol/output.rs seat inspect, not at the brief's output_compact.rs:919 / human.rs:254 (those render SeatSummary, which has no retirement_state); justified, no seat-list display of reversible state
Task 10 (ht-ma6i.10): minor (deferred): [unread-history-only] window_messages_are_unread_on_reattach pins window mail as history with no delivery row, so a reattached agent is not told it exists (spec 4.3 agreed); bead wording 'unread on reattach' holds only in that sense; graph side should confirm
Task 10 (ht-ma6i.10): minor (deferred): [wall-clock-assertion] shift_of_1000_pending_rows_finishes_inside_the_call_budget asserts a real 3s wall-clock budget in tests/store/reversible_reads.rs; may need a nextest test group if it flakes under starvation
Task 10 (ht-ma6i.10): minor (deferred): [duplicate-test-fixture] reversible_reads.rs copies a ~40-line node fixture from reversible_facts.rs
Task 10 (ht-ma6i.10): minor (deferred): [reattach-seam-untested] window pin reattaches via rebind_unresolved_seat plus seeded cursors, not a keyed check-in
Task 10 (ht-ma6i.10): minor (deferred): [missing-evidence] report's full logs are not retained (excerpts only); recorded run failure 'mod test: golden cases cover every DP2 case' has no baseline in this task's evidence (task touches no mod code), attribution gap and owner left unchanged

## amendments

ht-ma6i.7: re-dispatch now; stack conflict in ht-8qzo no longer exists, no human reconcile needed
ht-ma6i.6: re-dispatch now; stack conflict in ht-ocoy no longer exists, no human reconcile needed
ht-ma6i.1: hold until review bead ht-ma6i.16 closes, then re-dispatch
ht-ma6i.4: hold until review bead ht-ma6i.16 closes, then re-dispatch
