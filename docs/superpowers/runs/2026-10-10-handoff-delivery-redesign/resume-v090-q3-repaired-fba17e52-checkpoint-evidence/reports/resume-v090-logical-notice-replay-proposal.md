# Logical Notice replay proposal — unapplied

Source/index were frozen for the parent's Q3 run. This turn changed only ignored WS scratch artifacts. No Cargo, test/daemon process, Git, Herdr, real harness/model/config, or nested agent was used. Scoped rustfmt parser/format checks passed for both complete proposed Rust files and the standalone test addition; type-check and behavioral execution remain the parent's next gates.

## Actual RED and cause

Parent evidence: `/private/tmp/ht-C-v090-notice-replay-diagnostic-33c74793/{run.log,execution.json}`. HEAD `33c74793a369fad377036bf182b832c8cb05bac7`, tree `0c1922d5432308ce41e15d277211892f4d4ded32`, leak run `b36e4e14-d9e6-4144-b6b9-77e58b4ad1e0`, nextest UUID `8c1926cf-9284-4e27-ad1e-075fb7e71271`. List/compile exit 0; one selected test failed (run exit 100, 1.905 s); scoped leak check exit 0.

The before/after diagnostic proves the same published warning `w0af6e0a6-63f5-813d-9e84-7a63938466f3`, exact thread `tgN8gyP6y`, preparation `pXEZM7Lo1`, source `mXEZM7Lo1`, instance `75275848-e753-4f77-81c8-6d52d9bc82e0`, decision 8, warning offset/count 1/1, base sequence 3, interval high 3, recipient generation 8, digest event `(8,1)`, digest ordinal 2 and offered frontier 2. The physical warning row is absent before and after. The exact same binding ID, generation and execution survive the permitted process-exit reconnect; there was no retry CheckIn. Retry retains Invitation and Attention exactly but drops Notice.

At `src/store/invitation_content.rs:1655`, `replay` decides Notice applicability solely from `messages(id,thread_id,kind='warn')`, so it marks the original virtual Notice slot retired despite intact publication and recipient authority. Fresh delivery already supports published virtual warning recipes in `src/store/attention.rs:1182`; `migrations/0025_warning_notice_delivery.sql` canonically projects those recipes without requiring a physical warning message. The offer frontier advances when a page is allocated (`invitation_content.rs:2601`), so asking for another unoffered notice is not a recovery mechanism.

## Scratch artifacts (revision 2; authoritative)

The earlier production-v1 and service-file test patches are superseded scratch history. Do not apply `resume-v090-logical-notice-replay-tests-proposed.patch`: parent requested source ownership isolation from active Q3 work. The current proposal has three independently staged patches:

1. `resume-v090-logical-notice-replay-store-tests-proposed.patch`: tests-only baseline RED. Adds `tests/store/notice_replay.rs` and the private test module declaration at the end of `src/store/invitation_content.rs`. No `tests/service/delivery_pull.rs` hunk. Fixture helper implementations use the existing service fixture patterns locally, with no process or ancestor.
2. `resume-v090-logical-notice-replay-production-v2-proposed.patch`: the same replay predicate as v1, with the single production SQL literal named by a private constant so child tests can measure that exact statement. No public visibility expansion or changed predicate.
3. `resume-v090-logical-notice-replay-query-tests-proposed.patch`: post-production tests-only addition to the new store test file. Depends on the production SQL constant; do not include it in the baseline RED compilation before patch 2.

`resume-v090-logical-notice-replay-store-tests-proposed.rs`, `...-store-all-tests-proposed.rs`, and `...-production-v2-proposed.rs` are full scratch files for inspection, not whole-file overwrite instructions. All pass scoped rustfmt parsing/format checks. The earlier artifacts below remain historical:


- `resume-v090-logical-notice-replay-production-proposed.patch`: only `src/store/invitation_content.rs`; 36-line private helper plus replacement of the Notice applicability arm (rustfmt expands the adjacent match formatting). Does not change Confirm or any producer.
- `resume-v090-logical-notice-replay-production-proposed.rs`: complete production candidate for parser/format inspection, not an installation source.
- `resume-v090-logical-notice-replay-tests-proposed.patch`: only additive test in `tests/service/delivery_pull.rs`; applying it is tests-only, independently of the production patch.
- `resume-v090-logical-notice-replay-test-addition.rs`: the standalone test addition.
- `resume-v090-logical-notice-replay-tests-proposed.rs`: complete test candidate for parser/format inspection. Do not overwrite the shared service file with this scratch copy; apply only the additive hunk to preserve Q3 ownership.

## Applicability and fixed bound

The physical warning branch retains its original exact id/thread/kind test. The new logical branch preserves the captured item verbatim only when all of these canonical keys agree:

| Relation | Lookup / predicate | Existing bounded index or constraint |
|---|---|---|
| messages | captured warning id; captured thread and warn kind | PRIMARY KEY(id) |
| digest_programmatic_warnings | canonical claim seat + captured warning id; captured thread | UNIQUE(seat_id,warning_id), migration 0008 |
| send_manifests | claim instance + digest decision; captured thread; digest offset in 1..warning_count | UNIQUE INDEX send_manifests_instance_decision, migration 0001 |
| prepared_unavailable_warnings | exact published preparation + digest offset; captured warning id | UNIQUE(preparation_id,warning_offset); alternate UNIQUE(preparation_id,warning_id), migration 0001 |
| warning_recipients | captured warning id + claim seat; generation equals digest event sequence | UNIQUE(warning_id,seat_id), migration 0001 |

`CROSS JOIN` fixes digest -> manifest -> prepared offset -> recipient lookup order. Every outer relation is already at most one row before the next lookup; no LIMIT-after-filter scan, warning-id history walk, membership walk, queue scan, body hydration or new schema/index is introduced. The manifest lookup explicitly selects the existing unique instance/decision index. The theoretical upper bound is five exact-key relation probes per replayed Notice, no more than 160 for the existing maximum of 32 descriptors (normally fewer because non-Notice descriptors share the slots). Existing `work.replay` counts every visited descriptor. This is a structural source/index argument, not a claim that five SQL statements or five measured engine instructions establish a cost bound. No EXPLAIN or runtime counters have been executed by this agent. The separate post-production query control measures the exact production SQL on the linked SQLite: all five relations must appear as indexed SEARCH, the manifest must use its named unique index, relation FullscanStep and Sort must be zero, VM steps must remain <=256, and each case's VM range must stay within 16 steps as same-ID unpublished recipes and unrelated digest rows grow from 0 to 64 to 4096. The test prints version, query plan and counters for parent evidence. Those are proposed assertions pending the parent's actual run, not measured results. It also checks wrong warning/thread/recipient/instance and missing manifest/offset/recipient authority, with rollback between mutation controls.

The helper intentionally does not consult the advanced notice frontier or present condition actionability. A frozen Notice is an informational historical event; its original body/id/thread/order/token/codec remains intact even after another reader handles the underlying condition. The exact binding and consumer remain admitted by the existing canonical `binding_scope`/presentation row path before this predicate.

## Confirm and trust

Read TRUST-POLICY, especially the capable presentation section at 1544–1551: “explicit confirmation performs no discovery, fallback or revalidation walk.” Current Confirm validates exact binding/current-or-last token and captured control subsets, then consumes retained non-control slots without any physical warning lookup (`invitation_content.rs:833–938`). That behavior is already applicable to virtual Notices and should remain unchanged. Adding a second physical/logical discovery query to Confirm would be unnecessary and contrary to the stated policy.

The tests-only proposal proves partial Confirm consumes the original Notice/Invitation slot mask while omitted Attention stays in its original slot; subsequent exact control Confirm and duplicate Confirm preserve applicable acknowledged stamps and canonical last-token state. Foreign token returns NotCurrentPage without touching the frozen row. No receipt ACK, invitation acceptance, lazy display, frontier reset, wake, waiter, new materialization requirement, or provenance/admission change is proposed. No TRUST-POLICY weakening or new accepted limit is needed.

## Deterministic regression proposal

`store::invitation_content::notice_replay_tests::presentation_virtual_notice_replays_and_confirms_original_slots_without_materializing`

The in-process real service fixture registers stable synthetic reader/inviter claims; member c has no registered binding. A real RequireAck invitation supplies an independent continuing Attention witness. The reader authors a real ordinary message to joined thread t, producing c's published unavailable-warning recipe while avoiding a self-receipt on the reader. Bounded existing send-attention and warning-attribution fixture steps establish the canonical notice projection; they do not write a physical warning. The test explicitly asserts the physical row is absent before Pull, after restart/replay and after Confirm.

It requires exact entire-page equality (no Notice filtering or replacing descriptors), exact frozen binding/consumer/generation/execution/token/descriptor/selection/render/codec/prefix/masks, and unchanged full warning publication+digest+recipient data. It keeps an exact ordinary receipt-state/invitation-state snapshot, checks the offer frontier remains unchanged after its first allocation, and checks partial/full/duplicate confirmation masks and stamps. The short-lived registered shell fixture plays no role in this service regression. The existing registered failed-stdout test still supplies the physical writer/flush/codec failure boundary.

Expected RED against current production: full page equality loses the virtual Notice after service restart. This expectation is supported by the actual registered diagnostic RED; the new deterministic test has not yet been compiled or run. Recommended parent sequence: revision-2 store tests-only patch -> compile/list and RED -> production-v2 patch plus query-test patch -> fresh focused GREEN and actual EXPLAIN/VM evidence -> controls. No edits to the persistent parent fixture or DirectSend proposal are bundled here.

## Exact parent selectors

New test, lib:
`test(=store::invitation_content::notice_replay_tests::presentation_virtual_notice_replays_and_confirms_original_slots_without_materializing)`

Post-production exact-query/index/VM control, lib:
`test(=store::invitation_content::notice_replay_tests::retained_virtual_notice_lookup_uses_indexed_constant_vm_work)`

Existing registered physical-output RED:
`binary(hook_entrypoint) & test(=agy::activation::activation_v2_registered_agy_failed_stdout_replays_without_completion)`

Existing registered lost-completion control:
`binary(hook_entrypoint) & test(=agy::activation::activation_v2_registered_agy_lost_completion_preserves_exact_recovery)`

Existing lib controls:
- `test(=service::delivery_pull::tests::presentation_hook_notice_settles_current_execution_without_reoffer)`
- `test(=service::delivery_pull::tests::presentation_watch_notice_settles_current_execution_without_reoffer)`
- `test(=service::delivery_pull::tests::presentation_watch_notice_continuation_preserves_remaining_confirming_condition)`
- `test(=service::delivery_pull::tests::presentation_confirmation_preserves_original_slots_and_exact_subsets_on_restart)`
- `test(=service::delivery_pull::tests::presentation_handler_partial_confirmation_retains_canonical_slots_across_restart)`
- `test(=service::delivery_pull::tests::presentation_final_watch_codec_counts_escaped_canonical_invitation_batch)`

Production scope deliberately excludes the unrelated DirectSend liveness loss and ephemeral registered-hook process fingerprints. Those remain separately proposed/reviewed work.

## Quiescent hashes (revision 2)

No candidate file was written by this agent. The Q3-owned service test file changed externally from the earlier read; it is excluded from every current proposed hunk.

- Current observed candidate `src/store/invitation_content.rs`: `1d0239ee6ea4422f927273d5c21c4fff1820446c77bf0268986544cd380478fb`.
- Current observed candidate `tests/service/delivery_pull.rs`: `7a3d88fc5fb5af1d6e794b38b3e7190abfc0cb1e70e114d703efbb8b9452ade2`.
- Scratch `resume-v090-logical-notice-replay-production-v2-proposed.patch`: `83df5a30c117abd34d82c7fa675e35ea79b5aa5b3c0bb2495130fe7537fcd733`.
- Scratch `resume-v090-logical-notice-replay-store-tests-proposed.patch`: `9976fe5c9d01d5af81809f20f93881dcce15ae64f0106f6b4917fc9ac8aa70f7`.
- Scratch `resume-v090-logical-notice-replay-query-tests-proposed.patch`: `6505e5dbc2b8e695a97c063894da6a9302a5e1da6e5dc987bd5ef455dface4de`.
