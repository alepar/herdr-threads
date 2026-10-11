# Active D interface source review

2026-10-11. **Spec PASS; Quality PASS for the narrow activation source delta and mapped cumulative interface scope.** Runtime activation proof is pending and is explicitly separate from this source verdict. No merge or completed-feature clearance is granted.

Reviewed frozen D **7c3edc2d5d6131258d4f0dfca7e1788ecbe3a4e4**, tree **71d69644a7e1247d5ccb14adbaa40b72bf79dc47**, at `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`. Independently read HEAD/tree, exact diff from reviewed inactive **31e5879781904b9ec20ee5ea3cec7517856260f3**, actual capability route/handler probe and compatibility seams. Only two paths changed: `src/protocol/capabilities.rs` and `tests/integration/mod_delivery.rs`.

## Scope mapping and strengths

All 80 paths in the corrected-D inventory were hashed against the active checkout: **78 retain the exact reviewed inactive blob; two match the inspected delta**. Read-only Git confirms no additional committed changed path. The active capabilities blob is the same as the supplied activation helper `9f2ee80f`.

The cumulative review in `corrected-D-interface-review.md` and its coverage JSON therefore carries forward for unchanged Hook, Watch, AGY, mod, protocol shapes, admission, store/service, codec and completion implementations. This includes the closed I1/I2/I3 findings, exact output/flush/Confirm boundary, bounded capacity repair and source-clearance limits. It does not transform prior inactive or seam execution into active runtime evidence, or replace the independent schema/whole reviews.

The production change is the intended single addition of `DELIVERY_PRESENTATION_V2` to `ADVERTISED` (`src/protocol/capabilities.rs:95`). The new comment accurately describes registered-codec/explicit-confirmation behavior and historical omission fallback. It adds no wire field, migration, authority, larger bound, new wake source or repeated model-visible framing.

## Advertisement, handlers and compatibility

`ControlService` builds actual Capabilities from `ADVERTISED` (`src/daemon/control.rs:315`); its existing provider filters do not suppress this name. The production domain already dispatches PullV2 and explicit Confirm at `src/service/dispatch.rs:1244`. These routes retain their existing canonical admission, fixed-consumer/render validation and exact owner checks. Advertisement activates prepared consumer selection; it does not bypass those checks.

Watch's Accepted response now includes the registered owner's presentation lease through the existing advertised-set gate (`src/daemon/transport/watch_connection.rs:157`). The consumer still needs both the capability and a valid Accepted lease to choose V2 (`src/cli/watch.rs:413`). Omitted capability, omitted lease and malformed lease retain the reviewed historical path. Unconditional inactive omission bytes remain asserted by `tests/daemon/watch_connection.rs:626`, while its actual production transport assertion follows the active set. Historical nested decoder/optional-field coverage is unchanged. This test still uses a fake registration seam; actual same-child elected ownership remains an execution obligation.

AGY's existing capability query now selects its prepared V2 branch; registered selector resolution, exact native encoding, original boot-fenced completion link and write+flush gating are unchanged. Capability omission/refusal still selects V1. The historical native byte controls and installed-bridge omission/refusal test remain in the prepared selection. No additional adapter or neutral-core branch is needed to activate this contract.

The stable capability test already expects the new spelling/list. `every_advertised_capability_has_a_handler` routes this capability to `presentation_capability_routes_use_configured_domain` (`tests/protocol/capabilities.rs:404`). That probe uses actual configured ControlService/DomainService/SqliteStore, canonical check-ins and CreateThread/RequireAck Invite producers; it requires actual advertised support, nonempty canonical output/control stamps, exact Confirm, AlreadyConfirmed and persisted retirement without invitation acceptance (`:1852`). It is a meaningful handler/source probe rather than NoDomain or list membership alone. Actual elected Watch and installed AGY children remain the stronger composition execution checks.

## I4 correction mapped to the active source

The only test change is the previously scoped-reviewed `c4f0eb00` lazy-only oracle correction, transferred and formatted on D. I read the active diff and `ht-finish-watch-lazy-format-proof.json`; the source behavior matches `watch-lazy-oracle-rereview.md`. **I4 is ADDRESSED IN SOURCE at this active checkpoint.** The historical cumulative report correctly retains I4 against immutable31e.

`activation_v2_watch_lazy_only_page_is_passive` now finds the real PullV2 page containing its lazy ID, including a possible already-in-flight initial Pull (`tests/integration/mod_delivery.rs:2561`). It checks the same child's lease, one item, omitted token, empty controls and no canonical outbox. It keeps actual stdout/passivity/pending/no-ACK/no-health assertions, then observes owned-child termination and reconnect grace before inspecting the route for absence of Confirm, ACK, acceptance and lazy completion. It no longer waits for the nonexistent message-only presentation Confirm. No production behavior was changed to satisfy the test.

No Critical, Important or Minor source defect found in this scoped delta. No additional source correction is requested.

## Runtime and readiness boundary

The parent reported first active execution in progress as Cargo session39605, log `/private/tmp/ht-finish-first-active.log`, leak run ID `0efd58be-48be-404c-a215-41ad14b92cc6`. **This source review does not consume or certify a final result from that running selection and makes no GREEN claim.** An output file's existence alone would not prove an authentic complete capture; `/private/tmp/ht-finish-advertised-watch-capture.jsonl` requires successful producer assertions plus source/binary/command identity and digest before downstream replay can be accepted.

Required next evidence remains the actual advertised five elected Watch cases, four registered AGY active cases plus historical negative, capability probes and affected consumer regressions, actual capture provenance and real plugin-engine consumer execution or accurately reported refusal. Newly exposed behavior failures must be classified from their exact diagnostics and returned for source review where needed. Final gates, controlled build/lint-budget checks, exact integration mapping, scoped leak cleanup and other cumulative reviewers remain parent-owned. The prior under-one-minute lint-budget issue is not closed by this source review.

**Ready for the planned active verification: yes, on source grounds. Ready to merge: not established.** The activation delta matches the approved narrow plan, and the cumulative consumer source clearance maps to this exact tree with I4 corrected. Actual runtime evidence must still establish that the newly selected production chains pass.

Only this report was written. No source/Git mutation, Cargo, formatter, lint, native engine, SQL execution, Herdr/tracker write, fixture process or subagent was used. **COMPLETE + QUIESCENT.**
