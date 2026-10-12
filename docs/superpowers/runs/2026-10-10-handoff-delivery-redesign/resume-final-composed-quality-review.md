# Final composed candidate: scoped static quality review

Verdict: **scoped static QUALITY PASS; no new actionable blocking source defect found.** This is the requested fresh precompletion review, not a code-roast round, runtime certification, exhaustive line-by-line audit, or merge-readiness approval. Pending final gates remain pending.

## Immutable scope

- Candidate: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090`.
- HEAD `9a573c3f2a5420ff0c3fee07a3e836d189e88a83`; tree `5158b885c8988b1c3ebf5b96047751a6e4949212`.
- Actual local Main `23801c98ece1a8c437420d6cd823c5780993f386`; origin/main `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`. Independently reobserved these Git identities and clean candidate status at the end of inspection. Installed 0.8.1 and release status are parent-provided facts, not runtime observations by this reviewer.
- Reviewed cumulative Main238→candidate changed-path inventory, current critical functions and callers, selected cumulative diffs, design and TRUST policy, Main-v090 author reports, prior whole/composition reviews, fresh final Q1 spec/quality reports, Main238 composition-impact report, and the automatic-validation independent review. The old f330/b046/D314 reviews are historical context, never clearance for newly composed Main code.
- No Cargo, build, test, plugin runtime, Herdr command, source/index/Git mutation, or subagent was used. This report is the sole write. Read-only source/Git/evidence inspection started no owned daemon or background process.

## Critical composition assessment

| Area | Current-source assessment |
| --- | --- |
| Canonical authority and exact binding | Service Hook/Watch admission uses the exact claim and resolves a canonical binding. `invitation_content::binding_scope` independently checks the resolved binding's role, connected/open state, seat/instance/execution/generation/target/current target generation and recovery holds. Pull compares its internal binding hint with that result. Durable consumer/items/private proofs and ledger/replay scopes are binding-owned; the per-seat mutex only serializes decisions. No seat-latest selector is substituted for caller authority. |
| Reconnect and siblings | Hook V1/V2 share the same plain-reconnect admission; V2 selects the bounded presentation view. Watch requires an already-connected exact binding and actual Live lease. Same-binding resume preserves the binding's ledger; sibling bindings retain distinct outboxes, partial/carry/read state and lease snapshots. Canonical receipt settlement remains shared, but ordinary/Lazy revalidation uses the admitted reader before reclamation. |
| Q1 private continuation and transaction | Current `queries.rs::continued_ordinary` positively checks captured recipient, message/manifest, membership, captured ordinal/decision bounds, exact binding cursor and matching effective receipt identity. Only terminal authority plus this reader's covering display cursor retires the body; Pending stays eligible and missing authority refuses. Private accumulator/skip identity and byte offsets are validated before retirement. Public inbox validation stays strict. `store/mod.rs::presentation_pull` uses `execute_budgeted_decision`; tentative save and final envelope checks occur before decision acceptance, with cancellation rollback and callback removal before rollback. Service ledger/replay updates follow successful store return. |
| Q1 accounting | Retirement consumes one of the existing 100 ordinary candidate visits. The separately observable `ordinary_continuation_key_probes` ceiling is 15, with no due/replay/carry combination and no second ordinary source walk. Existing body hydration/setup remain separately accounted. The 6cf final spec/quality reports' eight controls distinguish real own-read retirement, Pending, sibling unread, malformed/missing authority, SQL abort and two cancellation boundaries. The fixed-call clock test alone is not proof of the same late boundary after adding callbacks; the Decision-sample control supplies that boundary witness. |
| Q2 reclamation correction | Current fixtures no longer equate manual seat ACK with own display. The manual-ACK window test requires cursor/stamps unchanged, then performs actual display advancement before reclaiming. Logical precedence/waiver controls keep both bindings unread first, advance one, and require the sibling still retained. Physical legacy/missing-key controls remain. Correct exact counts are 2048 Lazy, 2304 published ordinary, and 2560 worst-case physical terminal for 256 keys. Parent's exact-candidate ten-case recorded run passes (details below). |
| Physical confirmation and controls | Confirm resolves the exact binding, validates current/last token, complete incoming stamp subset and original descriptor slots/masks before effects. It acknowledges only applicable captured controls. Confirm performs no relevance walk, ordinary ACK, public cursor advance or invitation acceptance. Current/last replay cannot acquire a sibling or later token's controls. Health recording is restricted to a fresh applicable Attention and matching currently Live binding/generation/session/lease. The separate retained legacy observer path remains; this review does not claim that all health timestamps originate exclusively in V2 physical Confirm. |
| Notice, Clear and relevance | Binding notice frontier and canonical execution are retained. Relevance has a fixed 13-source model, 100 combined source/fallback row allowance, shared 26 seek budget and explicit point/witness accounting. Positive witness applicability is canonical; the global fence invalidates negative proofs. Unknown may delay Clear under continuing unrelated mutations; it does not authorize new output. Current/last token slot stability and epoch-scoped frontend Clear survive composition. |
| Watch replay, locking and current outlook | Registry stores at most 64 entries with at most 256 full captured offer stamps per owner. Capture is installed only after successful store return; even empty successful capture is finite for the lease. Scope includes BindingId, generation, execution, native session, lease and ledger incarnation. Full stamp comparison prunes progress, sibling snapshots refuse, incarnation loss abandons without same-lease recapture. Register/replacement/disconnect/close use the same owner permit around exact entries. Store work does not hold registry/ledger mutexes; replay application does not nest those locks. Physical owner replay preserves logical counts/timestamps, parked ordinary state and the current post-store native outlook. |
| Fixed runtime bounds | Fresh physical discovery remains 433; publication 100 manifest visits/200 keyed probes/3 seeks; refresh 160. Ordinary modes remain exclusive: due/replay 100 attempts, 32 hydrations, 1803 probes/100 availability seeks; carry 32 bodies, 579 probes/32 seeks; reconciliation 256 keys/2560 probes. Negative caches are two per binding, at most 256 full-stamp keys each, within the 4096-binding ledger lifetime. Output is 32 items/65536 actual encoded bytes, ordinary body 8192 raw bytes, carry 155731, partial 49415, 352 sizing passes plus one final RPC encode. These are relation/allocation envelopes, not measured SQLite VM instructions or wall-clock guarantees. |
| Lazy and model-facing delivery | Modern ordinary invitations and Ensure creation policy default Lazy; legacy omitted policy retains historical interpretation. Timed invitation/required membership requests remain separate from message receipt. Mod passivity includes Notice, unpromoted Lazy, nonwaking invitations and already-settled ordinary mail. Promoted Lazy is assembled as Message with wakes=true, preserving wake_on:all. Message/Lazy complete output remains eligible for the existing completion path even with ACK=false, so own cursor advancement survives sibling receipt settlement. Invitation/Notice/Attention never enter those message ACK batches. |
| Frontend output and codecs | Watch keeps independent presentation/binding-cursor negotiation, the exact registering claim, actual lease, bounded structured/encoded-size validation, flush-before-Confirm, explicit stamp subset handling, Close-before-continuation and success-only truncation hints. Hook completion is tied to successful write/flush, pinned original route/boot, bounded payload and registered codec selection. agy preserves a frozen supported selector and verifies actual encoded bytes; local hints/probation cannot supply canonical receipt or token authority. |
| Main238 wake repair | Both Main canonical seat notice binding selectors survive in wake candidate/current-authority reads. These remain per-seat wake coordination, distinct from exact caller binding admission. Candidate `open_origin` override still keeps an open original-sender warning actionable after informational delivery; confirming-invitation-only age/batching survives. Presentation offer history cannot erase canonical open-origin waking attention. |
| Send-time receipt and invitation work | Effective logical/physical receipt rules preserve running/terminal history and separate availability from canonical send-time deadline. Backfill and due discovery share finite allowances and captured high waters before candidate filtering; ordinal continuation handles future/cancelled candidates without unbounded skipping. Sparse due projection and completion reset remain transactional. Creation-only Ensure policy does not rewrite an existing pair's episode; required attach excludes historically linked requirements and creates a distinct timed episode where a voluntary Lazy invite exists. Command/context epoch updates remain transactional and checked. |
| Handoff, automatic guard and wire limits | Body source/policy is frozen before durable/native effects, capability checks precede new semantics, and exact child requests retain policy/body/keys. The private `current_after_parent_guard` reader has two guarded callers; phase validation retains its original stricter parent guard plus request validation. The removed work is redundant parent hydration within the same canonical transaction, not TTL relaxation or cached authorization. Wire expansion is restricted to modern Some-policy automatic response family, with matching returned identity; generic and historical None paths remain 1 MiB. Bounded summary/local diagnostics remain separate from canonical launch authority and possible-start recovery. |
| Stable setup and launch | Stable native event commands remove random command comments while exact manifest/group/fingerprint ownership remains. Canonical route/event parsing and foreign/drift conflict refusal remain; legacy marker recognition is diagnostic rather than ownership. Setup readiness stays advisory to otherwise guarded launch, with optional owned configuration rechecked only when actually used. Hermes callback/runtime and native host grammar stay adapter-owned. |

## Schema and upgrade assessment

The Main238→candidate migration diff contains only added versions 36–45; published migrations 1–35 are byte-identical. `schema.rs` includes all versions in their actual order and latest schema is 45. Migration37 explicitly copies physical `receipts.ack_binding_id` and sparse `receipt_state.ack_binding_id`, preserving ordinals/rowids and sequence high water. It does not infer an attribution from current bindings. Presentation44/45 use binding-owned keys and foreign keys.

Read `published_v35_upgrade_preserves_exact_ack_bindings_and_delivery_history`: it constructs actual V35, records distinct physical/sparse ACK bindings and independent binding delivery/frontier history, induces V37 post-copy FK failure, requires rollback to V36 with exact old catalog/data and original foreign_keys setting, removes the fault, then upgrades/reopens. That is meaningful source coverage; its current runtime outcome remains parent-owned. No reviewer-run migration, exhaustive migration45 trigger proof or startup-cost measurement is claimed. Accepted one-time quick_check/index work remains distinct from healthy runtime bounds.

## Evidence and unresolved gates

Independently read `/private/tmp/ht-C-v090-q2-composed-corrected/execution.json`, `run.log`, and `leaks.log`. They identify this exact 9a HEAD/tree, ten exact selected library cases, run exit 0, **10 passed / 4832 skipped, 1.603 s nextest**, 1.942 s wrapper, and zero scoped leaks for `ef65dc54-f3dc-4a56-9cd0-a7d266d1d222`. Enumeration/build took 141.089 s after a multi-file import; it is not a controlled one-source incremental measurement. The metadata records binary SHA256 `d85357d8739028349d1eb7c05b08018d1d7ff02ac980d18c627243315a58110a`; the reviewer did not rebuild or establish independent source-to-binary provenance.

Q1's actual eight-case 6cf PASS, causal cancellation RED controls, and sibling-control correction were assessed through the fresh independent final spec/quality reports and current code; those reports are evidence references, not fresh 9a runtime execution by this reviewer. The 71bb/342 automatic negative/optimization evidence remains narrowly scoped: the restored duplicate-guard mutant also passed. Redundant hydration is real, but **historical permit-expiry cause/fix is unproven**. The original Hook startup failure's cause remains **unknown**. Neither historical issue is waived by this static PASS.

At report time parent-owned actual Main35/schema/new-Main wake execution and the refreshed 657-case matrix were pending, as were final exact-tip gates and controlled speed measurements. The parent's prior strict lint gate passed functionally but took 69.133 s; it does not satisfy the <60 s lint budget. Final incremental build and lint measurements must receive explicit disposition. No old f330/D314 pass or prepared selection count substitutes for these current gates.

The historical unused `FrozenInvitation`/`block_text`/`join_blocks` maintenance observation still applies in `src/delivery/invitation.rs`: it is an apparent second representation beside the actual `FrozenPresentation`. No new runtime defect follows from it; cleanup is optional, not a prerequisite imposed by this review.

## Coverage interpretation

The appendix maps every one of the 86 changed `src/` paths against actual Main238 to its reviewed functional seam. `same f330 blob` is only an exact historical source correspondence; `changed since f330` explicitly requires the current composition assessment above. Neither label transfers a historical verdict. Inspection concentrated on critical current functions/callers, cumulative interface diffs, and selected regression bodies. Peripheral path mapping and author/history corroboration are not a claim that every line or every dependency received equal fresh depth.

Outside `src/`, reviewed the actual Claude mod production diff, migration37 and presentation/schema ownership seams, and the current migration inventory. The other added migration bodies and peripheral scripts/config/tests were covered by correspondence, author/prior-review context and targeted source tests, not a new exhaustive runtime audit. Broader native/plugin behavior, all migration45 trigger cases, adversarial corruption permutations, saturated non-ASCII continuation/query-plan experiments, fsync latency and live-model uptake remain outside this static review. No change weakens the cooperative same-user trust model or calls for adversarial verification.

## Source-path appendix

| Changed path vs Main238 | Functional coverage | Source correspondence to historical f330 | Candidate blob |
| --- | --- | --- | --- |
| `src/archival_legacy.rs` | Bounded retained handoff origin/progress readers | same f330 blob | `010f059c3d56` |
| `src/cli/commands.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | changed since f330 | `2e4f98b0b059` |
| `src/cli/direct_send.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | same f330 blob | `54c953cc0c63` |
| `src/cli/doctor.rs` | Stable hook ownership and advisory launch | same f330 blob | `787ffe51a48d` |
| `src/cli/handoff.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | same f330 blob | `354228b1a14f` |
| `src/cli/handoff_delivery.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | same f330 blob | `f5bded1f9b4b` |
| `src/cli/help.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | changed since f330 | `df439e2bc440` |
| `src/cli/hook.rs` | Exact claim, codec/output completion, negotiation, replay | changed since f330 | `b7997fe59cb9` |
| `src/cli/journal.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | changed since f330 | `a2e355f4299c` |
| `src/cli/launch.rs` | Stable hook ownership and advisory launch | changed since f330 | `d0b056aa03dc` |
| `src/cli/mod.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | changed since f330 | `7169a88074fa` |
| `src/cli/setup.rs` | Stable hook ownership and advisory launch | same f330 blob | `e93b6a44609c` |
| `src/cli/topology_handoff.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | same f330 blob | `372c1cd8ca67` |
| `src/cli/topology_handoff/automatic.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | same f330 blob | `d248bae9e125` |
| `src/cli/topology_runtime.rs` | Handoff/body/policy propagation, durable retry, preflight and CLI presentation | same f330 blob | `58b32a6fdd9c` |
| `src/cli/watch.rs` | Exact claim, codec/output completion, negotiation, replay | changed since f330 | `755c2b5bd263` |
| `src/client/local.rs` | Request-scoped capabilities, frame/lease plumbing and wake guard | same f330 blob | `43cf2e586a34` |
| `src/client/service.rs` | Request-scoped capabilities, frame/lease plumbing and wake guard | same f330 blob | `f6340eb65bb9` |
| `src/daemon/control.rs` | Request-scoped capabilities, frame/lease plumbing and wake guard | same f330 blob | `a2b1d23110b7` |
| `src/daemon/transport.rs` | Request-scoped capabilities, frame/lease plumbing and wake guard | same f330 blob | `e88bd46d04b4` |
| `src/daemon/transport/watch_connection.rs` | Request-scoped capabilities, frame/lease plumbing and wake guard | same f330 blob | `ae1316bc17a4` |
| `src/delivery/assemble.rs` | Bounded assembly/rendering, binding ledger, full stamps and output budgets | changed since f330 | `989d2391f31e` |
| `src/delivery/invitation.rs` | Bounded assembly/rendering, binding ledger, full stamps and output budgets | same f330 blob | `377e24fe3e39` |
| `src/delivery/ledger.rs` | Bounded assembly/rendering, binding ledger, full stamps and output budgets | changed since f330 | `a632dd090d2a` |
| `src/delivery/mod.rs` | Bounded assembly/rendering, binding ledger, full stamps and output budgets | same f330 blob | `285bfbcb8f0b` |
| `src/delivery/page.rs` | Bounded assembly/rendering, binding ledger, full stamps and output budgets | same f330 blob | `df1e880d4a18` |
| `src/delivery/presentation.rs` | Bounded assembly/rendering, binding ledger, full stamps and output budgets | same f330 blob | `b254b485e76f` |
| `src/harness/adapter.rs` | Registered codec/completion seam and hook/event admission | changed since f330 | `0b70d6ab1e74` |
| `src/harness/agy/decode.rs` | Registered codec/completion seam and hook/event admission | changed since f330 | `29bcd4d6f487` |
| `src/harness/agy/delivery.rs` | Registered codec/completion seam and hook/event admission | same f330 blob | `9c93e97c42b8` |
| `src/harness/agy/launch.rs` | Stable setup ownership, route generation, optional owned launch configuration | same f330 blob | `887af29504e6` |
| `src/harness/agy/mod.rs` | Registered codec/completion seam and hook/event admission | changed since f330 | `13642bd933b5` |
| `src/harness/agy/paths.rs` | Stable setup ownership, route generation, optional owned launch configuration | same f330 blob | `2eff8776f885` |
| `src/harness/bridge.rs` | Registered codec/completion seam and hook/event admission | changed since f330 | `d61d0f59d15c` |
| `src/harness/claude.rs` | Stable setup ownership, route generation, optional owned launch configuration | changed since f330 | `a3f4b5ed5bbd` |
| `src/harness/claude/setup.rs` | Stable setup ownership, route generation, optional owned launch configuration | same f330 blob | `6da3b4a1cbc3` |
| `src/harness/claude_mod.rs` | Registered codec/completion seam and hook/event admission | same f330 blob | `cb6a786863fe` |
| `src/harness/codex.rs` | Stable setup ownership, route generation, optional owned launch configuration | changed since f330 | `f1ffa4ce94f4` |
| `src/harness/codex/setup.rs` | Stable setup ownership, route generation, optional owned launch configuration | changed since f330 | `dfb7e82ec9b4` |
| `src/harness/hermes/launch.rs` | Stable setup ownership, route generation, optional owned launch configuration | same f330 blob | `3fcc060b5d60` |
| `src/harness/hermes/runtime.rs` | Stable setup ownership, route generation, optional owned launch configuration | same f330 blob | `f9e1a902ce96` |
| `src/harness/launch.rs` | Stable setup ownership, route generation, optional owned launch configuration | changed since f330 | `116adeccbad7` |
| `src/harness/registry.rs` | Stable setup ownership, route generation, optional owned launch configuration | changed since f330 | `c284eba2fca7` |
| `src/harness/setup.rs` | Stable setup ownership, route generation, optional owned launch configuration | changed since f330 | `a4d246a4cef3` |
| `src/host/native.rs` | Optional owned configuration test initializers; production native guards inherited | changed since f330 | `1e5bd1a176c6` |
| `src/ports.rs` | Binding-exact APIs, ownership snapshots and observable work counters | changed since f330 | `0d10b20cad80` |
| `src/protocol/capabilities.rs` | Additive wire types, capability semantics and actual result/frame bounds | changed since f330 | `2bd1cf53f39f` |
| `src/protocol/commands.rs` | Additive wire types, capability semantics and actual result/frame bounds | changed since f330 | `83a6086b2049` |
| `src/protocol/delivery.rs` | Additive wire types, capability semantics and actual result/frame bounds | same f330 blob | `ae0cb5471c92` |
| `src/protocol/handoff.rs` | Frozen body/policy/original, actual encoded admission and retry representation | changed since f330 | `689d654cfcb7` |
| `src/protocol/handoff/automatic.rs` | Frozen body/policy/original, actual encoded admission and retry representation | same f330 blob | `cb669a0b7f96` |
| `src/protocol/handoff/automatic_bootstrap.rs` | Frozen body/policy/original, actual encoded admission and retry representation | same f330 blob | `cdf315b27a91` |
| `src/protocol/output_compact.rs` | Additive wire types, capability semantics and actual result/frame bounds | changed since f330 | `84531f5bd146` |
| `src/protocol/results.rs` | Additive wire types, capability semantics and actual result/frame bounds | changed since f330 | `a7e1e0e99cdf` |
| `src/protocol/watch.rs` | Additive wire types, capability semantics and actual result/frame bounds | changed since f330 | `2f46e1df757b` |
| `src/protocol/wire.rs` | Additive wire types, capability semantics and actual result/frame bounds | changed since f330 | `ff442206352b` |
| `src/scheduler/deadlines.rs` | Finite due continuation and independent open-origin wake condition | same f330 blob | `ddf3d6babb38` |
| `src/scheduler/mod.rs` | Finite due continuation and independent open-origin wake condition | changed since f330 | `6e8f85319581` |
| `src/service/delivery_presentation.rs` | Canonical admission, post-store installation, registry ownership and dispatch | changed since f330 | `1309f3252031` |
| `src/service/delivery_pull.rs` | Canonical admission, post-store installation, registry ownership and dispatch | changed since f330 | `479fd28d26bd` |
| `src/service/dispatch.rs` | Canonical admission, post-store installation, registry ownership and dispatch | changed since f330 | `3de8c8f82c2f` |
| `src/service/kicks.rs` | Canonical admission, post-store installation, registry ownership and dispatch | changed since f330 | `3fcc6311b914` |
| `src/service/mod.rs` | Canonical admission, post-store installation, registry ownership and dispatch | changed since f330 | `cdc9f6e704a2` |
| `src/service/mod_channels.rs` | Canonical admission, post-store installation, registry ownership and dispatch | changed since f330 | `b300f64cd7a1` |
| `src/store/attention.rs` | Current binding presentation/continuation/notice/relevance and deciding transaction | changed since f330 | `c9a91d9ca4eb` |
| `src/store/control.rs` | Creation-only Ensure, fresh required episodes, checked command/context epochs | changed since f330 | `32068c972f7f` |
| `src/store/effective.rs` | Send-time receipt/availability, bounded due discovery and canonical wake | changed since f330 | `2d9aa40c7543` |
| `src/store/handoff/automatic.rs` | Exact child policy/body/guard and canonical retained bounds | changed since f330 | `d9944b990f93` |
| `src/store/invitation_content.rs` | Current binding presentation/continuation/notice/relevance and deciding transaction | changed since f330 | `99aaab9b59df` |
| `src/store/invitation_due.rs` | Send-time receipt/availability, bounded due discovery and canonical wake | same f330 blob | `08c0ee73c4bf` |
| `src/store/materialization.rs` | Send-time receipt/availability, bounded due discovery and canonical wake | changed since f330 | `407c477829c0` |
| `src/store/messages.rs` | Send-time receipt/availability, bounded due discovery and canonical wake | changed since f330 | `87bed65d569e` |
| `src/store/mod.rs` | Current binding presentation/continuation/notice/relevance and deciding transaction | changed since f330 | `a60b8358a609` |
| `src/store/presentation_relevance.rs` | Current binding presentation/continuation/notice/relevance and deciding transaction | changed since f330 | `bf0045701a17` |
| `src/store/queries.rs` | Current binding presentation/continuation/notice/relevance and deciding transaction | changed since f330 | `b31d44506496` |
| `src/store/receipts.rs` | Send-time receipt/availability, bounded due discovery and canonical wake | changed since f330 | `ffb9a5967b38` |
| `src/store/schema.rs` | Published-prefix preservation, actual35→45 upgrade and rollback source | changed since f330 | `593455d2b7db` |
| `src/store/seats.rs` | Current binding presentation/continuation/notice/relevance and deciding transaction | changed since f330 | `0f2229282aea` |
| `src/store/service_controls.rs` | Creation-only Ensure, fresh required episodes, checked command/context epochs | same f330 blob | `4b8add495ebf` |
| `src/store/topology_handoff.rs` | Exact child policy/body/guard and canonical retained bounds | same f330 blob | `3fbb9e8b5fda` |
| `src/store/topology_handoff/attachment.rs` | Exact child policy/body/guard and canonical retained bounds | same f330 blob | `d79b5f2cfc88` |
| `src/store/topology_handoff/automatic.rs` | Exact child policy/body/guard and canonical retained bounds | same f330 blob | `547944293334` |
| `src/store/topology_handoff/persistence.rs` | Exact child policy/body/guard and canonical retained bounds | changed since f330 | `212318ef4d06` |
| `src/store/wake.rs` | Send-time receipt/availability, bounded due discovery and canonical wake | changed since f330 | `f7136335c156` |
| `src/test_support/history.rs` | Owned test fixtures/process support; peripheral correspondence | changed since f330 | `1ff400cbb984` |
| `src/test_support/mod.rs` | Owned test fixtures/process support; peripheral correspondence | changed since f330 | `069c66cebd6a` |

Inventory check: 86 changed src paths; each appears exactly once above.

Additional changed implementation/validation paths:

| Path | Coverage |
| --- | --- |
| `integrations/claude/mod/hooks/register.js` | Fresh production diff inspection: passive classification, exact epochs/context dedup, complete message completion |
| `migrations/0036_invitation_confirmation.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0037_receipt_send_time.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0038_handoff_body_bounds.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0039_receipt_deadline_backfill.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0040_invitation_lifecycle.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0041_invitation_warning_origin.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0042_receipt_warning_origins.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0043_automatic_handoff_body_bounds.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0044_invitation_presentation.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `migrations/0045_presentation_relevance.sql` | Added unpublished redesign migration; actual Main1–35 unchanged; schema/source and historical correspondence (37 and 44/45 composition targeted above) |
| `scripts/canary/test_seam_integration.py` | Peripheral validation/process-support correspondence; no runtime executed; inherited Main package/demo changes are not release evidence |
| `scripts/canary/test_tier0_checks.py` | Peripheral validation/process-support correspondence; no runtime executed; inherited Main package/demo changes are not release evidence |
| `scripts/check-no-leaked-processes` | Peripheral validation/process-support correspondence; no runtime executed; inherited Main package/demo changes are not release evidence |
| `scripts/harness-canary.sh` | Peripheral validation/process-support correspondence; no runtime executed; inherited Main package/demo changes are not release evidence |
| `scripts/tests/test_check_no_leaked_processes.py` | Peripheral validation/process-support correspondence; no runtime executed; inherited Main package/demo changes are not release evidence |
| `scripts/validate-native-demo.py` | Peripheral validation/process-support correspondence; no runtime executed; inherited Main package/demo changes are not release evidence |
