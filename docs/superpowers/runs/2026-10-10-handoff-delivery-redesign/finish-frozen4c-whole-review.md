# Cumulative D audit at frozen 4c4988d9

Spec ISSUES

Quality ISSUES

This is a fresh cumulative audit of base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218` through frozen HEAD `4c4988d9ee937c333d602bac25dca25fc2c07d37`, tree `1589136ce7568bcd72b0050fa8d947e6f49503ab`. All 74 changed paths are accounted for below and in `coverage.json`. This is not a PR roast or a clean verdict inherited from preparation. Production source was not modified, Cargo was not run, and no git, tracker or Herdr state was mutated by this auditor.

## Supported findings

### D-WHOLE-1 — Important: V2 ordinary offers exhaust their ledger permanently after canonical settlement

At `src/service/delivery_presentation.rs:163–185`, V2 snapshots every remembered offer and computes new-message capacity as 256 minus `entry.offers.len()`. At lines 218–222 it records every offered/truncated ordinary message, but never reconciles this set against canonical settlement. `src/store/invitation_content.rs:1989–1990` then refuses a new message when that capacity reaches zero. `src/delivery/ledger.rs:14` establishes the 256-entry bound.

The real Watch settlement path, `src/service/dispatch.rs:817–833`, performs `AckModDelivered` and updates channel acknowledgement time, without reclaiming the settled offer. The existing removals are in the legacy whole-walk path (`src/service/delivery_pull.rs:672`, `retain_pending`) and Hook `DeliveryConfirm` (`src/service/delivery_pull.rs:817`, `remove`). Watch V2 clients use the former public ACK command, not the Hook-only confirmation endpoint. Ordinary explicit/manual settlement also lacks V2 reconciliation.

Thus one unchanged binding can deliver and canonically settle 256 ordinary messages, yet message 257 remains withheld indefinitely while the carry retains continuation work. Repeated Watch continuation can keep requesting pages without releasing a slot. Restart, generation rotation, or a legacy full walk may mask the defect; these are not normal V2 progress. Existing saturation tests exercise full pending ledgers rather than cumulative successfully settled offers.

Fix shape: reclaim canonical settled offers through bounded reconciliation or equivalent settlement bookkeeping, respecting the declared physical-work budget and generation ownership. Do not evict still-pending offers or give invitation presentation confirmation receipt semantics. Add a public-path regression using actual V2 Watch Pull and `AckModDelivered` for more than 256 cumulative offers, plus a relevant ordinary explicit-ACK case. Confirm message 257 emits, settled IDs no longer consume capacity, and pending IDs retain their retry limits. The finding is source-supported; no runtime reproduction was executed by this auditor.

### D-WHOLE-2 — Important: three private revisit invitation IDs allocate before validation

In `src/store/invitation_content.rs`, `revisit_candidates` reads private `invitation_id` columns into owned `String` values at lines **1148, 1181 and 1225**. These are the cleanup, render-key dirty sizing, and size-ordered revisit readers. They bypass the bounded borrowed-value reader used elsewhere. The dirty path then calls infallible `InvitationId::new` at line 1190; the size-ordered path does the same after reading its row.

A repaired/FK-disabled private row can contain an overlong or malformed ID. The driver allocates that unbounded text before enforcing the 128-byte ID contract; the typed constructor can panic instead of returning `StoreCorrupt`. Cleanup may treat malformed private data as stale and mutate it rather than refuse the operation. SQL row LIMITs bound row count, not text bytes.

Fix shape: use the shared borrowed SQLite value/type/UTF-8/byte guard before ownership, then fallible `InvitationId::parse`, consistently for all three readers. Return the existing typed corruption error before effects; retain the current row/seek budgets. Add malformed, oversized and wrong-storage-class cases that actually reach each of the three production reads, with transactional no-effect assertions. Existing private-state/item/refresh/context/proof guard tests do not cover these reads. This is a previously known gap independently verified in the frozen source.

### D-WHOLE-3 — Important: the registered canonical replay test fails on a stale V2 request and stale response assertions

`tests/service/delivery_pull.rs:2425` starts `presentation_v2_replays_one_canonical_invitation_until_emit_confirmation`. Its raw request at lines 2436–2438 omits the now-required `render` field (`src/protocol/delivery.rs:160–163`, no default). The assertion at line 2439 consequently fails before exercising replay. Later assertions around lines 2489, 2500 and 2511 still compare confirmation results to obsolete scalar strings rather than the structured `status` and `acknowledged_controls` object. This test is registered via `src/service/delivery_pull.rs:847`; it is not dead code.

Fix shape: supply the current Hook render object, including both selectors, and assert the structured confirmation result while preserving token reuse across restart, exact stale/current-token handling and no invitation acceptance. Run this exact registered test after repair.

This is also an **observed runtime failure** in the parent's existing `/private/tmp/ht-finish-frozen-sweep.log`: the 88-selected frozen-source nextest run reports 87 passed / 1 failed in 2.841s, with `missing field render` at the cited line. The auditor read that log but did not run the command. The preparation final-24 filter omitted this regression, so its green result never established the whole changed service-test surface.

## Contract and ownership conclusions

The audit used the current accepted source and constraints: fresh Pull physical source cap **433**, passive Refresh **160**, relevance **13 sources / 26 shared seeks** plus declared keyed/witness work, publication **100 visits / 200 keyed probes / 3 seeks**, sizing **352 candidate passes** plus final RPC encoding, and **32 blocks / 65,536 actual encoded bytes**. Private carry **155731** and partial **49415** caps remain. Historical 217/333 limits and a constant-time migration interpretation are superseded. Confirm performs no discovery. U+10FFFF is allowed only as the exhausted `after_thread` sentinel.

Canonical daemon state remains the authority; invitation presentation confirmation is distinct from receipt ACK, acceptance, launch, registration and completion. The reviewed producer/consumer paths keep ordinary lazy work passive and require real native stdout or Watch emission plus flush before confirming controls. Native codec identity is frozen for the current invocation; confirmed hints use only the daemon-acknowledged subset. Watch ownership and cutoff use exact binding/session/lease scope and serialize replacement. These positive source conclusions do not override the findings above.

The former unconditional Watch lease finding is **resolved in this frozen source**: `src/daemon/transport/watch_connection.rs:157` gates the lease on the actually advertised presentation capability. The source intentionally still leaves that capability unadvertised. The current transport test checks actual unadvertised omission and a helper advertised branch; this is not evidence of a complete real advertised production flow. Activation remains an explicit later gate.

Migrations 1..39 have no cumulative diff. The new STRICT presentation schema and additive relevance migration retain their accepted startup `quick_check` cost, which scales with retained rows/indexes. Migration tests use real populated released39/40 databases and transaction rollback/reopen assertions; this does not imply an outer atomic 39-through-41 transaction or constant startup work. All 87 repeated source-fence trigger bodies were mechanically compared across 29 canonical tables. The schema peer owns additional database-depth review.

## Restored attention coverage and source provenance

All **14 restored attention/relevance test bodies** in `tests/store/attention.rs` were inspected. They cover canonical required-Lazy overlay changes; quiet proof behavior; retained membership and exact witness revalidation; repaired and unchanged captured-high membership; direct legacy and direct association behavior during ordinary Lazy churn; canonical receipt removal; fallback progress under source load; all 13 stale sources; exact covering SQL plans and bounded successor VM work; sweep mutations/overflow rollback; exact unavailable episodes and late canonical publication; and oversized/null/malformed/invalid-UTF-8 private proof refusal before effects.

The restoration evidence records two necessary fixture adaptations: direct association initialization now clears `unavailability_open`, and the all-sources test recognizes the canonical source-7 projection instead of inserting a duplicate. It also includes harmless `if let` cleanup and the added proof-corruption regression. Original pre-cleanup bytes were not retained. The claim is restored behavioral assertions and canonical seams, **not byte-identical historical recovery**.

Frozen HEAD/tree and clean worktree were checked directly. Every path hash in the final entrypoint map matched the frozen files. The final-affected and clippy logs' SHA-256 values matched the recorded sourcepoint. The original final-affected log shows 24/24 selected tests passed (2.340s test run; 59.70s build), including all 14 restored attention tests and ten selected seams. The clippy log ends successfully at 46.96s. The parent's broader frozen-source log establishes 87/88 selected tests passing and the one replay failure above; its scoped leak log says `no leaked test processes`. Exact log hashes are recorded in `coverage.json`. These are existing observed logs, not new audit executions or source-inferred PASS claims.

## Explicit coverage limits and follow-up gates

- Every changed path is included, but large added service, agy, Watch and JavaScript test files received function/assertion inventory plus targeted semantic body review; this is not a claim that every fixture line was independently audited. Repeated SQL trigger bodies were structurally compared instead of all being reread.
- Current/final accepted refinement reviews and relevant disposition/sourcepoint artifacts informed the audit. Superseded historical review pages were not exhaustively reread. The final source and current constraints control the verdict.
- No full suite, live advertised V2 activation sweep, default-feature check, final successor/integration readiness gate, or controlled single-source incremental timing measurement was performed by this auditor. The selected runtime logs cannot establish those results.
- Schema and interface peers provide independent depth. Their conclusions are not substituted for this report's stated coverage or used to manufacture a clean whole-task result.
- The ledger reclamation scenario and all three private revisit corruptions need actual red/green regressions on a mapped successor source. Helper branches mentioned by the parent have not been transferred into frozen D or reviewed here.
- The documented Main026 same-ID control composition changes are outside this frozen audit. A frozen-D test still expects the third same-ID attachment to fail; that expectation must be adjusted only with the explicitly mapped integration behavior.
- Any successor change requires a focused recheck of affected callers, bounded-work accounting and relevant tests before this cumulative verdict can be upgraded. Final runtime gates must name the actual successor/integration SHA.

## Full changed-path coverage

The machine-readable artifact records SHA-256, review level, notes, finding links and added-function inventory for every path. No changed path is omitted.

| Changed path | Audit coverage |
| --- | --- |
| `TRUST-POLICY.md` | Accepted bounded presentation contract, receipt versus display semantics, migration quick_check cost, final work budgets and private-state caps. |
| `integrations/claude/mod/hooks/register.js` | Watch protocol admission, ordered rendering, queued/inflight epoch cutoffs, exact-decimal comparisons, actual emit completion and retry semantics. |
| `integrations/claude/mod/tests/delivery.test.ts` | Changed test/function and assertion inventory; focused passive invitation, truncation, epoch, malformed event and retry checks; not every fixture body audited line by line. |
| `migrations/0040_invitation_presentation.sql` | Complete new presentation table/index schema and STRICT constraints; retained-row startup quick_check is explicitly accepted. |
| `migrations/0041_presentation_relevance.sql` | Additive scalars, relevance overlays/proofs/indexes, bootstrap and fence layout; mechanically verified all 87 fence triggers over 29 tables share one body. Repeated bodies not individually reread. |
| `src/cli/hook.rs` | Rendered invocation ownership, actual stdout write+flush completion, confirmed-subset hints and frozen completion endpoint/boot. |
| `src/cli/launch.rs` | Advisory invitation selection and native readiness result instead of accidental automatic control effects. |
| `src/cli/watch.rs` | Capability/session-scoped V2 fetch, owner lease, ordered event emission, flush-before-confirm and continuation handling. |
| `src/daemon/control.rs` | Watch admission obtains correct presentation lease; declaration and invocation dependencies followed. |
| `src/daemon/transport/watch_connection.rs` | OwnerAccepted presentation lease is capability-gated in frozen source. Earlier unconditional-lease issue is resolved; actual advertised runtime still staged. |
| `src/delivery/assemble.rs` | Exact-body acquisition and partial/whole delivery construction changes; invitation metadata separated from ordinary bodies. |
| `src/delivery/invitation.rs` | Complete bounded canonical invitation formatting, advisory context and truncation/whole geometry. |
| `src/delivery/ledger.rs` | Offer capacity256, reoffer accounting, warning eligibility, retain_pending/remove paths and separate invitation cache. |
| `src/delivery/mod.rs` | New presentation/invitation module wiring. |
| `src/delivery/page.rs` | Legacy invitation display geometry and page budgeting changes checked against new bounded presentation path. |
| `src/delivery/presentation.rs` | Complete actual native/Watch sizing, ordered projection, static render identity and candidate/final encode accounting. |
| `src/harness/adapter.rs` | Neutral default invocation/completion and codec policies; no adapter-specific branch introduced into core. |
| `src/harness/agy/decode.rs` | Native System/User response codec framing, actual serialized sizes and completion payload decoding. |
| `src/harness/agy/delivery.rs` | Frozen render admission, invocation/emit split, physical completion, retry/rebind and confirmed hint subset. |
| `src/harness/agy/launch.rs` | Actual native readiness and advisory launch behavior. |
| `src/harness/agy/mod.rs` | Adapter method wiring for native presentation and completion. |
| `src/harness/agy/paths.rs` | Stable supported hook event mapping and canonical output paths. |
| `src/harness/registry.rs` | Forwarding new neutral adapter capabilities/defaults. |
| `src/ports.rs` | Presentation store port, input/output/work shapes and bounded operation seams. |
| `src/protocol/capabilities.rs` | New named capability remains intentionally unadvertised; production activation is deferred, not itself an audit defect. |
| `src/protocol/commands.rs` | V2 command variants and argument wiring. |
| `src/protocol/delivery.rs` | Full V2 types, required render profile, bounded selector/control validation, structured confirmations, token/epoch validation. |
| `src/protocol/output_compact.rs` | Compact rendering of new structured outcomes; no invitation acceptance implication. |
| `src/protocol/results.rs` | V2 result variants, typed refusal/confirmation outcomes. |
| `src/protocol/watch.rs` | Optional capability-gated lease/content fields, omission behavior and ordered V2 item shape. |
| `src/scheduler/mod.rs` | Passive presentation work does not create wakes; canonical warning eligibility bypass remains actionable-only. |
| `src/service/delivery_presentation.rs` | Complete V2 scope/codec admission, per-seat serialization, ledger snapshot/record, canonical pull and confirm routes. |
| `src/service/delivery_pull.rs` | Cumulative change reviewed, legacy whole-walk reconciliation and Hook confirmation removals followed; registered stale V2 regression identified. |
| `src/service/dispatch.rs` | Public command routing, AckModDelivered canonical mutation/registry ack and absence of V2 ledger reclamation. |
| `src/service/kicks.rs` | Presentation bookkeeping passive; canonical context producers request bounded passive refresh. |
| `src/service/mod.rs` | New presentation service wiring and registry state. |
| `src/service/mod_channels.rs` | Exact live owner/lease admission, per-seat permit, passive refresh, cutoff ordering and passive no-stall behavior. |
| `src/store/attention.rs` | Bounded relevance query entrypoints and canonical fingerprint integration; restored source-specific tests evaluated separately. |
| `src/store/control.rs` | Canonical timed invitation/control context changes, legacy/modern producer paths and transaction epochs. |
| `src/store/invitation_content.rs` | Complete private state/carry/outbox reader, bounded invitation walks/revisits, hydration, publication, ordinary carry, pull/refresh/confirm behavior. |
| `src/store/mod.rs` | Transaction ownership and presentation port wrappers, canonical decision placement. |
| `src/store/presentation_relevance.rs` | Complete bounded local relevance proof, 13-source shared sweep, direct overlays, witness revalidation and private proof guards. |
| `src/store/queries.rs` | Exact wanted-ID hydration, digest-free view, fixed publication range and generated marker inverses. |
| `src/store/schema.rs` | Migration40/41 dispatch, catalog/quick_check validation and released migration compatibility. |
| `src/store/seats.rs` | Digest-free current binding/seat view supporting bounded canonical presentation admission. |
| `src/store/service_controls.rs` | Control attachment/detachment context epochs and timed confirmation overlay behavior. |
| `src/test_support/mod.rs` | Test-support exposure/wiring for owned test capabilities. |
| `tests/cli/hook.rs` | Updated invocation fixture construction and actual completion behavior. |
| `tests/cli/hook_neutral.rs` | Default inert completion and successful write+flush gating assertions. |
| `tests/cli/launch.rs` | Four advisory display/native readiness regressions and negative guards. |
| `tests/cli/lazy_display.rs` | Updated typed output field expectations and existing lazy display behavior. |
| `tests/cli/output.rs` | Compact structured protocol output expectations. |
| `tests/cli/watch.rs` | Added function/assertion inventory plus selected real owner/session/write/flush/close/foreign-ACK/replaced-owner cases; large fixture not exhaustively line-read. |
| `tests/combined.rs` | Explicit suite registration consistent with autotests=false; target feature wiring checked in unchanged Cargo manifest. |
| `tests/daemon/watch_connection.rs` | Actual currently-unadvertised OwnerAccepted lease omission, helper advertised-positive branch and lease registration checks; no real advertised activation claim. |
| `tests/delivery/extraction.rs` | Exact goal/body omission and timed invitation reactivation extraction tests. |
| `tests/fixtures/agy/plugin/hooks.json` | Expected stable hook event and CLI route fixture. |
| `tests/harness/agy_decode.rs` | Actual escaped System/User response geometry and serialization assertions. |
| `tests/harness/agy_delivery.rs` | Added function/assertion inventory, physical-emission helpers and selected codec/loss/refusal/expiry/rebind/confirmed-subset cases; not every large fixture body line-read. |
| `tests/harness/agy_launch.rs` | Readiness fixture migration to actual native outcome. |
| `tests/harness/agy_setup.rs` | Stable event mapping and exact generated command route checks. |
| `tests/hook_entrypoint/agy.rs` | Real CLI lazy turn visibility, no acceptance and native wake behavior assertions. |
| `tests/integration/agy_sweep.rs` | Explicit require-ack on nudge-class sweep message aligns delivery contract. |
| `tests/protocol/delivery.rs` | Full new wire roundtrip/validation and structured control confirmation tests. |
| `tests/protocol/lazy_inbox_v2.rs` | Current presentation capability staging expectation. |
| `tests/protocol/watch.rs` | Optional lease serialization omission and wire roundtrip checks. |
| `tests/service/delivery_pull.rs` | All added function names inventoried; targeted bodies for canonical replay, partial/carry, exact budgets, ordinary slot saturation, publication/rebind and corruption guards. Not every 4679-line addition individually line-read; stale replay found. |
| `tests/service/kicks_contract.rs` | Passive presentation bookkeeping versus canonical producer refresh assertions. |
| `tests/service/mod_channels.rs` | Complete added owner/admission/grace/passive no-stall/permit/cutoff regression bodies. |
| `tests/store/attention.rs` | All 14 restored test bodies reviewed for canonical producers, 13-source sweeps, exact plans/VM bounds, rollover/rollback and corruption. Restoration is adapted behavior, not a byte-identical historical recovery. |
| `tests/store/control.rs` | Complete added control attachment/required-Lazy overlay tests. Existing same-ID third-attach failure is correct for frozen D but needs documented Main026 composition adjustment. |
| `tests/store/invitation_content.rs` | Changed tests and targeted full bodies for carry/partial caps, private guards, callback budgets and real STRICT populated migration work. No constant-cost migration claim. |
| `tests/store/queries.rs` | Complete exact-ID hydration, marker inverse, bounded publication SQL/VM and staging-to-late-manifest tests. |
| `tests/store/schema.rs` | Complete populated released39/40 upgrade/reopen, rootpage/catalog preservation, transaction rollback and pending-index work tests. |
