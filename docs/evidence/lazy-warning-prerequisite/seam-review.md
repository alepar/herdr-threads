# Warning prerequisite integration seam review

Task ht-big.9; isolated branch task-ht-big.9. Starting integration base a22d32739a0b472a92fe4a31b82a294e22940702. Common source base c3b8f3f0d1c67781d39c0ff433c6f144647fe6d7. No shared main mutation.

## Exact source and independent pre-consumption review

- Object/parent(s): 97fb9cf20ca4b5e344602fe386d39f59954f4a6e 06b7ac5e100f3c4b7f3c6762ae7e8dae87d7d135
- Object/parent(s): 8106f5cade8ac9d4c2dd8e9b3281e8df05abd8ab 36d86ad9d69472d5c29ed4d827e858f67ea35975
- Object/parent(s): 8265da8a008cac2c401ca5d05059ed699651afa0 93cc227fac8c0d73e578e2d87260e48a523075a6
- Object/parent(s): 737df497d04bb3cf6368d8167b359af680f7dcc4 08d4e59955c1e847d8894575087b249c008b0311

Fresh owner reviewer /root/warning_ready_combined_review APPROVE at frozen source737df497d04bb3cf6368d8167b359af680f7dcc4, treeab9153239f88636e239d7f1192dbd6992d221539, actual-main base97fb9cf20ca4b5e344602fe386d39f59954f4a6e, tree62d85c1c496622c26fc4eb6c481c41bc30186e47. Conditional mandatory source gates were completed before consumption (89 focused tests, 412 library tests, clippy/default/fmt/diff/leak0). Published review and bead comments supplied by coordinator were read before import.

- Review artifact: /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-warning-ready/target/handoff-warning-ready/independent-review.md; SHA256 631a3bc14c2d302ba4ab8ec297e98f163e6fe88e1cfcb49a57dbc032f8f171b3
- Freeze manifest: /private/tmp/handoff-warning-ready-freeze.json; SHA256 3480163fe24dea201843bf9be5dc01ae97e2abef9daa16c3182089963bef6491
- Frozen full binary diff against actual main SHA256: 06cbffcfb597a59505d8295ccc990dae833e0a58c671d03bb8f139cdf226db6f

## Consumed revision chain and composition

309288e9e560c5e826afdac8dbfc56b84cdaf979 test: advance legacy handoff upgrade expectation for warning migration
897dfcc6468f3c4b3eca71ba46af71b9a784d530 fix: assess invitation goals and settle warning notifications once
f1d6e081076d6ae5e6cae144713fde2087f6314f Merge commit '97fb9cf20ca4b5e344602fe386d39f59954f4a6e' into task-ht-big.9

Merged exact actual main first (seven-file archival/test delta), then cherry-picked exact warning8106 once and exact successor8265 once. All merges/cherry-picks were conflict-free. No broader topology history consumed. Original objects remain untouched. Imported docs/evidence and guide/policy retained in full.

No constructor amendments were needed: both cooperative SendMessage constructors already carried DeliveryMode::Ordinary from the protocol seam. Warning import changes no SendMessage mode serialization/digest, and lazy send stays inert Unsupported. InboxBatchV2, CompleteInboxDelivery and MessageDeliveryModes remain unadvertised/inert. The source warning capability alone is newly advertised, backed by its handler. Warning exact-occupant ordinal prefix settlement, late recipient/manifest projection, open/clear identity and historical evidence remain source-exact. Actual-main archival initialization guard retained. Migration0025 and schema LATEST_VERSION=25 are byte-identical to reviewed frozen source; no migration26/27 created.

- Composed binary diff against actual main (before this evidence-only commit) SHA256: e8212dbf372531dcc4a5a11c2d56d706dd307a836a652518c23ec5bb005580ae

### Entire warning8106 inventory (79 files)

- TRUST-POLICY.md
- docs/evidence/invitation-relevance/after-1.md
- docs/evidence/invitation-relevance/after-2.md
- docs/evidence/invitation-relevance/after-3.md
- docs/evidence/invitation-relevance/after-4.md
- docs/evidence/invitation-relevance/after-5.md
- docs/evidence/invitation-relevance/assessment.md
- docs/evidence/invitation-relevance/before-1.md
- docs/evidence/invitation-relevance/before-2.md
- docs/evidence/invitation-relevance/before-3.md
- docs/evidence/invitation-relevance/before-4.md
- docs/evidence/invitation-relevance/before-5.md
- docs/evidence/invitation-relevance/clippy.log
- docs/evidence/invitation-relevance/default-features.log
- docs/evidence/invitation-relevance/focused-final.log
- docs/evidence/invitation-relevance/focused-mailbox.log
- docs/evidence/invitation-relevance/green.log
- docs/evidence/invitation-relevance/hook-entrypoint-final.log
- docs/evidence/invitation-relevance/hook-entrypoint-recheck.log
- docs/evidence/invitation-relevance/hook-entrypoint.log
- docs/evidence/invitation-relevance/hook-nextest.log
- docs/evidence/invitation-relevance/hook-tests-final.log
- docs/evidence/invitation-relevance/hook-tests-recheck.log
- docs/evidence/invitation-relevance/hook-tests.log
- docs/evidence/invitation-relevance/inbox-warning-green.log
- docs/evidence/invitation-relevance/independent-review.md
- docs/evidence/invitation-relevance/leak-check.log
- docs/evidence/invitation-relevance/leak-run-id
- docs/evidence/invitation-relevance/red.log
- docs/evidence/invitation-relevance/source-inventory.json
- docs/evidence/invitation-relevance/tool-delivery-green.log
- docs/evidence/invitation-relevance/tool-delivery-red.log
- docs/evidence/invitation-relevance/verification.md
- docs/evidence/invitation-relevance/warning-attention-focused.log
- docs/evidence/invitation-relevance/warning-audit.md
- docs/evidence/invitation-relevance/warning-checkin.log
- docs/evidence/invitation-relevance/warning-final-green.log
- docs/evidence/invitation-relevance/warning-green-complete.log
- docs/evidence/invitation-relevance/warning-green.log
- docs/evidence/invitation-relevance/warning-manifest-red.log
- docs/evidence/invitation-relevance/warning-projection-cleanup-all-green.log
- docs/evidence/invitation-relevance/warning-projection-cleanup-cooperative.log
- docs/evidence/invitation-relevance/warning-projection-cleanup-green.log
- docs/evidence/invitation-relevance/warning-projection-cleanup-red.log
- docs/evidence/invitation-relevance/warning-projection-cleanup-schema.log
- docs/evidence/invitation-relevance/warning-red.log
- docs/evidence/invitation-relevance/warning-schema-focused.log
- docs/evidence/invitation-relevance/warning-schema-module.log
- docs/evidence/invitation-relevance/warning-wake-cost.log
- docs/evidence/invitation-relevance/warning-wake-focused.log
- docs/evidence/invitation-relevance/warning-wake-red.log
- integrations/skill/SKILL.md
- migrations/0025_warning_notice_delivery.sql
- src/cli/mod.rs
- src/client/local.rs
- src/harness/bridge.rs
- src/harness/mod.rs
- src/ports.rs
- src/protocol/capabilities.rs
- src/protocol/commands.rs
- src/protocol/output_compact.rs
- src/protocol/results.rs
- src/service/dispatch.rs
- src/store/attention.rs
- src/store/materialization.rs
- src/store/mod.rs
- src/store/queries.rs
- src/store/schema.rs
- src/store/wake.rs
- src/test_support/counting_client.rs
- tests/cli/hook.rs
- tests/harness/bridge.rs
- tests/hook_entrypoint.rs
- tests/protocol/capabilities.rs
- tests/protocol/output.rs
- tests/store/attention.rs
- tests/store/cooperative_checkin.rs
- tests/store/queries.rs
- tests/store/schema.rs

## Owned verification and TDD evidence

Fresh HT_LEAK_RUN_ID: DE7D7B83-DB25-48C4-9074-5E61078CE469. Persistent raw logs under task worktree .tmp/ht-big.9. Fixtures used existing isolated short /tmp test paths; no real config/shared-server/pane changes. No helpers retained.

- RED: nice cargo test --locked --all-features handoff_actual22_upgrade_imports_already_committed_create_then_replays_without_unattached_fence, before8265, exit101; expected assertion at411 actual25 versus expected24.
- GREEN: same command after exact8265, exit0, 1/1 passed; all original import/attachment/replay assertions preserved. Immutable historical RED/GREEN and independent successor APPROVE also imported unchanged.
- nice cargo test --locked --all-features store::schema: exit0 but zero selected tests; actual schema module is store::connection::tests. Corrected focused command nice cargo test --locked --all-features store::connection::tests: exit0,115/115 passed.
- nice cargo test --locked --all-features store::cooperative_checkin: exit0,52/52 passed.
- nice cargo test --locked --all-features protocol::capabilities: sandbox socket bind denied in3 tests (11 passed); rerun with isolated-fixture CLI escalation exit0,14/14 passed.
- nice cargo test --locked --all-features lazy_: exit0,10/10 passed (2 library +8 combined); checks legacy digest/default omission, unsupported lazy dispatch and unadvertised v2 seam.
- cargo fmt and git diff --check: exit0.
- nice cargo clippy --locked --all-targets --all-features -- -D warnings: exit0,28.76s.
- Scoped scripts/check-no-leaked-processes --run-id RUN --root TASKWORKTREE: exit0, no leaked test processes (process-inspection escalation).

Sandbox nice invocation prints setpriority Operation not permitted but runs the cargo/script successfully; no Rust warnings. Focused socket rerun was clean. Full suite intentionally deferred under explicit worker constraint. Fresh independent composed task review remains coordinator-owned.

- nice scripts/check-default-features: exit0. Final cargo fmt --check and git diff --check: exit0.

Raw evidence SHA256:

- red.log: 0a2a3b546eb3bacb7babd6be5b33714d9b3611fafd2bdaea064992d62c8ed35b
- green.log: da3b485819a7bb9cfc4a9a1c4f7c8e8e24e1b4bcce8f916907cd2224598d5d98
- schema.log: dd205e712161e89d4ce7848b6ff104fd518f385914738f7678cdbbfe606951c4
- schema-green.log: 5a50f8d973070c7c875d2732a200b8bd4226b19969a1d305a33a4bd3161a2c43
- cooperative.log: 17d83060ed2ec33f438928fa655025730e408ea099ad5aa66e1b63c7753d22d7
- capabilities.log: b2f3633c2ea19ca4a5b99ec8c1ba703502b759f9342da3ebaccd0f9b4f027cf0
- capabilities-green.log: 60b5b9e0ddd362d0b9356ba5af840a4a47166778fa56856e358bc58c782d1626
- lazy-seam.log: d7ccf949155f77b8df2236685d1601057273367f82aed9892ad05ef844189778
- clippy.log: 8beb4a66aa5ca41ae11bdd533ba4de5a0e1c465dc1d9ed7d74922b991dd1407e
- default.log: 1ae2ce53f6c4a3265acd385e7e4a21d827a2b5594337d0d9ae68b92a856b30dc
- leaks.log: 87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e
