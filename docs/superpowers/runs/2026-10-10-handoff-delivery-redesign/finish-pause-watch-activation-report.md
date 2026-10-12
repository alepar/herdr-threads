# Watch activation tests-only checkpoint

Status: SOURCE ONLY / NOT EXECUTED. No activation, Cargo, fmt, lint, native runner, Herdr write, tracker mutation or merge was performed. `git diff 4cb8aafe..HEAD --check` passed. No test processes or daemon processes were started by this helper.

Owned checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/finish-watch-activation`.
Branch: `helper/handoff-finish-watch-activation`.
Base: `4cb8aafe42906f5383aaaaa1531cb284a8287c5c` (tests-only successor of frozen4c).
Commits: `c7d610f2562465f471aee75dc257e27fa686248c`, `dbbdf495ca91da8b8fa19ddd442fc5ed4b15ee16`.
Transfer: `4cb8aafe..dbbdf495ca91da8b8fa19ddd442fc5ed4b15ee16`. Only the three assigned test files changed. Each commit has a truthful Outside-adapter trailer. Production remains dormant4c, unchanged by this helper. Parent must map tests onto corrected source and execute RED before activation.

## Implemented assertions, not execution evidence

| Exact test name | Location | Intended break caught |
|---|---|---|
| `activation_v2_elected_watch_capabilities_lease_output_confirm` | `tests/integration/mod_delivery.rs` | Actual elected Capabilities must select V2 in the real CLI child. Same child's real Accepted lease is valid lowercase 32hex and matches its claim/Pull/Confirm. The mixed canonical page contains lazy/ordinary/invitation/Warn notice/nonempty Attention. Real stdout order and newline byte count match actual captured items and `output_bytes` (32 items/65536 bytes). Pause the original nonempty Confirm before forwarding; observe stdout and still-pending canonical outbox; reject a foreign subset without retirement; then forward unchanged and check exact acknowledged stamps and canonical retirement. Confirm itself does not ACK, accept, consume lazy state or wake. Later explicit ACK/accept yields actual scoped clear. |
| `activation_v2_watch_lazy_only_page_is_passive` | same | Actual selected V2 lazy-only page has one item, empty control set, exact successful Confirm, no receipt ACK/event/native prompt/Attention. Crossing the stall timeout leaves the passive channel live. |
| `activation_v2_watch_replacement_replays_uncommitted_output` | same | Drop real nonempty Confirm before dispatch. Canonical outbox stays replayable. Replacement real CLI obtains a different Accepted lease and physically re-emits invitation/control before its own exact Confirm. Previous owner Confirm is StaleBinding. |
| `activation_v2_watch_lost_confirm_response_replays_exact_subset` | same | Forward actual Confirm and hold its actual response so the child/owner stays live. Separate recovery requests get AlreadyConfirmed with exact captured stamps or empty submitted subset. Foreign stamps are rejected and canonical progress stays unchanged. Then lose original response; consumer exits error honestly. |
| `presentation_watch_requires_both_capability_and_valid_accepted_lease` | `tests/cli/watch.rs` | Real run_session consumer retains historical output/InboxBatchV2 and issues no V2 request for omitted capability + valid lease, advertised capability + omitted lease, and advertised capability + malformed lease. Valid advertised + valid lease is already covered by existing capable consumer test and the new actual elected tests. |
| `presentation_owner_accepted_wire_carries_the_registered_lease` | `tests/daemon/watch_connection.rs` | Existing transport seam retained. Historical inactive Accepted omission bytes are now asserted unconditionally even after advertisement becomes active. |

All six rows above are NOT RUN. No actual stdout capture artifact has been created. Do not count tests' presence as an advertised-path success.

## Observer and capture contract

The test-local relay reuses the existing elected `Rig` (`run_elected_probed`, injected ShiftedClock, actual installed SessionStart, real owned CLI children, private stand-in host, isolated state/HOME/config). It moves only that rig's socket pathname, listens at the original pathname, and updates only isolated descriptor socket device/inode to match its listener; original descriptor/socket are restored on drop. Instance/boot/claim and all request/reply frames are unchanged. It forwards both directions/EOF and joins all threads. It can pause the first real nonempty Confirm, drop it before dispatch, or hold then lose its real response. It manufactures no capability, lease, page, success or model-visible content. Separate direct recovery requests are expressly not used as selection positives.

Evidence buffers are purpose-bounded: <=256 observed exchanges, each frame <=1MiB wire limit, <=131072 captured stdout bytes, <=32 items/65536 bytes asserted for the mixed page. Existing process helpers own Watch/CLI children; relay helpers are threads/sockets, all shut down/joined on drop. No product diagnostics API, runner framework, Cargo target, config policy, migration or production source was added. New strings are test diagnostics/fixture bodies; production model-visible byte delta is zero.

`HT_ACTIVATION_CAPTURE_PATH` is optional. When set for the elected positive, it writes the real child's exact observed stdout JSONLines (including initial connected status, mixed page and later scoped clear) after correctness assertions; the test never reads that file. IDs/epochs/bodies come from actual elected production output, not handwritten success fixtures. Runtime bodies include `activation passive lazy`, `activation ordinary ACK candidate`; Warn comes from actual service Notify on thread `activation-warn`; invitation topic is `activation invitation`. Parent must attach capture SHA256, executed source SHA, built binary identity/SHA256 and command identity. The downstream mod helper has this contract. Captured-stream replay proves mod consumer handling, not a live native engine spawning this child or model uptake.

## Retained sufficient seams and gaps

Do not duplicate these existing targeted controls merely to inflate elected test count:

- `presentation_watch_failed_flush_never_confirms_or_writes_success_hint` already exercises both write and flush failures against the actual V2 loop, with no Confirm.
- `presentation_watch_close_wins_before_next_more_quantum`, `presentation_watch_foreign_acknowledgement_is_error_not_success`, `presentation_watch_refused_confirmation_never_records_truncation_hint`, `presentation_watch_new_owner_physically_reemits_before_own_confirmation` retain consumer failure/hint gates.
- `presentation_watch_fresh_confirmation_records_exact_owner_publication_cutoff_once` covers canonical healthy Attention cutoff, passive/stale/refused/replay negatives and non-reset behavior. `presentation_attention_cutoff_requires_exact_live_owner`, `passive_invitation_drain_does_not_arm_attention_stall_history`, `presentation_owner_lease_is_exact_live_and_replacement_fenced` cover registry health/owner boundaries. The new elected tests do not expose private registry health through a new API.
- `presentation_confirmation_preserves_original_slots_and_exact_subsets_on_restart`, `presentation_handler_partial_confirmation_retains_canonical_slots_across_restart` retain durable exact-subset/slot behavior.

Registered AGY System/User hook proof, capability handler probes and production advertisement are sibling/parent scope. Actual plugin engine execution/capture replay is sibling scope. Stronger live native model uptake remains an external evidence boundary. No seam result, V1 fallback, missing match, zero-test exit or unexecuted native path closes active completion.

## Parent execution handoff

Every command below is UNEXECUTED by this helper; parent owns the single warm Cargo lane, absolute target dir, CARGO_BUILD_JOBS=4 and fresh leak run ID.

1. List integration tests with `nice cargo test --locked --all-features --test integration activation_v2_ -- --list`; require the four exact elected names above before execution.
2. List unit selections with `nice cargo test --locked --all-features --lib presentation_watch_requires_both_capability_and_valid_accepted_lease -- --list` and the existing watch transport/consumer selections. Require nonzero exact matches.
3. Before production activation, run the elected positive and record RED. Expected first assertion: `elected daemon must advertise presentation V2 before activation proof` in `activation_owner`, because the actual route returns the dormant advertised set. Earlier compile/setup errors are not that RED and must be corrected.
4. After corrected source review + parent activation, run all four elected selections and all `mod_delivery` integration tests (existing real-watch tests will select V2). Set `HT_ACTIVATION_CAPTURE_PATH` on the elected positive for the actual downstream replay artifact, then record provenance/digests.
5. Run relevant unit/service seams named above, parent formatting/clippy/harness-neutral/default-feature gates, and scoped leaked-process check. No routine full suite expansion.

## Exact inspected source selection

Git blob IDs below were resolved from base `4cb8aafe42906f5383aaaaa1531cb284a8287c5c`. Parent must remap every corrected production path and final active tip; these hashes are not clearance of later fixes. The helper's two commits change test blobs only.

| Path | Base git blob ID |
|---|---|
| `src/protocol/capabilities.rs` | `94cb2c9e9d4e0b8efe9cf8292bc775b362765118` |
| `src/app.rs` | `6c5cc878b9524fbc3ed181979c269762730e7407` |
| `src/cli/watch.rs` | `5cc0c2bfe387fbd39a2ce05a44844565b3e9abbd` |
| `src/client/local.rs` | `35903163325acf5c5f8e2d5b77e238c744551f8c` |
| `src/daemon/control.rs` | `a2b1d23110b7c459d08114c20e95eb99c3c69dc4` |
| `src/daemon/transport/watch_connection.rs` | `ae1316bc17a47e65a3bfc766fd04850bd11feb25` |
| `src/service/dispatch.rs` | `73a8d39135062dc384833d8f7b4f182668cda326` |
| `src/service/mod_channels.rs` | `e192631257c243bd55b42136814e1475d82ae16a` |
| `src/service/delivery_presentation.rs` | `4172d41451c4dc61c2bbad7b1e08e59d00653025` |
| `src/store/invitation_content.rs` | `2cc2ab287ab8756853faaf83fd6880abaae8871a` |
| `src/store/presentation_relevance.rs` | `76406381467fe3c416e70d80e24e80f2502ba4c5` |
| `src/delivery/presentation.rs` | `b254b485e76fb8297a9ab3fb826f0ce122425fb0` |
| `src/protocol/delivery.rs` | `ae0cb5471c9266dec6c93e41c5dc5e9e7e0ffa83` |
| `src/protocol/watch.rs` | `a4cc46f36d6c3c9efc3d5172bb692acf6d7acdbe` |
| `src/protocol/results.rs` | `ba934dc1fbb8944285b4d3574b508a31b85d56cd` |
| `src/daemon/ownership.rs` | `30c576a749bf345241a627b1d06aaf5ef6bf0c28` |
| `migrations/0040_invitation_presentation.sql` | `e5b0667838b2fa914a2d1213194cc30fac68b2c7` |
| `migrations/0041_presentation_relevance.sql` | `b0cfc959a9ce2339d8bbab7e1c5b085004c46dba` |


# Additive capacity and diagnostic checkpoint — 2026-10-11

Status: SOURCE-ONLY COMPLETE + QUIESCENT. Commit `8cd4fc07be3af71f42f7d590fc5ed21576e6f89a`, parent `dbbdf495ca91da8b8fa19ddd442fc5ed4b15ee16`. Only `tests/integration/mod_delivery.rs` changed; truthful Outside-adapter trailer. `git diff dbbdf495..HEAD --check` passed. No Cargo, fmt, lint, native engine, Herdr write, tracker mutation, source/core change, subagent or test process execution. Parent owns compilation, listing, RED/GREEN and source remapping. No production capacity implementation or activation is authorized or performed here.

Read `offer-capacity-spin-assessment.md` and `capacity-design.md`, preserving their distinction between parked unknown work, runnable due-known work, and independently runnable passive/control work. This checkpoint pins the actual CLI anti-spin consequence; exact-key due traversal, private offer stamp/cache assertions and work budgets remain in parent service/production scope.

## Added test and deterministic oracle

Exact registered integration test: `mod_delivery::activation_v2_watch_full_pending_capacity_parks_until_real_channel_input`.

1. Use the elected isolated Rig and actual installed enrollment as before. Freeze only UTC after setup; monotonic request budgets keep advancing, so no offer can accidentally become due because the machine is slow.
2. Run one real sender CLI SendMessage. Replay its actual recorded request as 256 additional canonical elected RPCs, changing only request/operation IDs. These are actual admitted SendMessage operations, not SQL/ledger fixtures. Successful real results provide all 257 message IDs. Each body is 6000 plain bytes, below the ordinary 8192 limit. Roughly ten fit a final 64KiB page; the final source slice therefore crosses the 256-slot boundary, and the test explicitly requires ID257's original body in actual durable carry rather than assuming that crossing occurred.
3. Extend only the test-local relay with a Condvar controller. Each real Pull reply is recorded and held for one explicit test permit. Every actual daemon Attention frame is held separately; none is synthesized, discarded or released during the initial drain. Request/frame events have a shared monotonic causal order. The ordinary first registration still gets the elected daemon's real capability response and same child's canonical Accepted lease, checked after the sender-seeding log boundary.
4. Step actual replies. Require 256 distinct real offered IDs and no early duplicate/truncation/ID257 offer, actual <=32 items/65536 bytes, then an actual Unchanged or Page.more=false reply. A fixture observation guard allows at most64 drain quanta (~26 output pages plus finite publication/maintenance); this guard fails the regression rather than capping any production/client drain. The source-reachable infinite empty Page.more loop fails this count oracle without a wall-time throughput benchmark or a sleep-based quiet window.
5. Before releasing channel input, assert actual durable normal carry retains ID257's exact 6000-byte body and all257 receipts remain pending, with no mod ACK event. Release the terminal real reply; observe actual stdout for ID256. Verify the child has not started another Pull before any Attention was forwarded. Release exactly one actual queued daemon frame, and require the next actual Pull's event order to follow that forwarding event. Its real response must be terminal and contain no ordinary offer/truncation/output. The complete normal carry/partial/cursor/skip/done/version/scan-generation tuple stays byte-for-byte unchanged.
6. Pending receipts stay pending. Stop the owned child and require actual canonical reconnect_grace, retaining upstream EOF and joined relay thread/socket cleanup. The relay's added state is bounded to65 request/reply observations (64 drain plus one explicitly stimulated retry); existing256-exchange/1MiB-frame bounds remain. Original optional actual stdout capture contract is unchanged and is not used as a fabricated positive for this regression.

The positive oracle is the real finite terminal response plus the causal ordering of the next actual request after released real input. It is not an elapsed no-RPC measurement, a global client drain cap, a synthetic capability/page, or direct service Pull fixture. No syscall-level native model uptake is claimed. Offer history preservation here is observed as 256 unique physical offers with no early repeat/reset/eviction across frozen-UTC retry and exact retained carry; private count/timestamp equality remains covered by the parent's service regressions, without a new diagnostics API.

UNEXECUTED parent commands: list the exact name with `nice cargo test --locked --all-features --test integration activation_v2_watch_full_pending_capacity_parks_until_real_channel_input -- --list`, require a nonzero match, then run on activated corrected source in the sole warm Cargo lane. Dormant source must fail the early actual capability assertion; activated pre-capacity-fix source must fail finite capacity parking. Missing match, setup timeout or compiler failure is not the intended capacity RED.

## Inactive execution diagnostic correction

Parent supplied `/private/tmp/ht-finish-watch-activation-inactive.log`: the two unit compatibility selections passed; the configured capability route test failed at its expected actual-advertisement assertion; all four elected Watch selections failed after about21.4s without readable panic detail. Those four failures are **not** meaningful advertised-path RED, and this helper does not reclassify them as such. Source inspection alone cannot identify their runtime cause from that log.

Added `Rig::activation()` for the five activation-dependent elected tests. Immediately after the actual elected descriptor publishes, before seat/enrollment/invitation setup, send a real boot-correlated Capabilities request to the actual endpoint. Dormant source now has a narrow intended first assertion: `actual elected Capabilities must advertise presentation V2 before activation setup`. This guard does not replace the same-child selecting-consumer evidence after activation.

The existing elected rig redirects process stdio while running. Its panic hook now saves at most8 UTF-8 messages of8192 bytes each and emits them after `stop_daemon` restores stdio during unwinding. This makes actual later setup/relay failures visible without requiring `HT_MOD_DELIVERY_PANIC_LOG`; the existing optional file path still works. No runtime validation of either correction was performed by this helper.

## Source mapping for additive handoff

Helper source remains dormant4c through its tests-only base; parent source at supplied7db6f13c is a different corrected mapping and must own final comparison. The table gives exact git blob IDs, not claims of compiling or executing either revision.

| Path | Helper base dbbdf495 blob | Parent supplied 7db6f13c blob |
|---|---|---|
| `src/cli/watch.rs` | `5cc0c2bfe387fbd39a2ce05a44844565b3e9abbd` | `5cc0c2bfe387fbd39a2ce05a44844565b3e9abbd` |
| `src/service/delivery_presentation.rs` | `4172d41451c4dc61c2bbad7b1e08e59d00653025` | `7f1740bba743a7ef244cac8cafd47df191f70b0c` |
| `src/store/invitation_content.rs` | `2cc2ab287ab8756853faaf83fd6880abaae8871a` | `b1c568cb8e717927049cc2c1111a1d8aaf0c0931` |
| `src/delivery/ledger.rs` | `66df729e5f1a84d14a8ee1ff3bd5966153c8313d` | `66df729e5f1a84d14a8ee1ff3bd5966153c8313d` |
| `src/protocol/capabilities.rs` | `94cb2c9e9d4e0b8efe9cf8292bc775b362765118` | `94cb2c9e9d4e0b8efe9cf8292bc775b362765118` |
| `tests/integration/mod_delivery.rs` | `c706bc5df22fe5235527b4b1ab0448b883d7012a` | `033dd1c679cbca2abd7d93ea64dee2cca29ab339` |

Parent's prior c7/dbb transfers were applied as6f911d02/b305e9a5 plus formatting df0463; this additive commit is based on the isolated unformatted helper tip and may require normal test-only conflict resolution when transferred. The original source mapping above and all original execution states remain historical. Final active production SHA, built binary identity, actual capture SHA256 and runtime evidence still belong to the parent.


## Review correction round 1 — WA-R1 and WA-R2 (source-only)

Checkpoint: `0007ada15cdd131d4a358f8929ccd18cfe8dd6a0`, parent `8cd4fc07be3af71f42f7d590fc5ed21576e6f89a`, isolated branch `helper/handoff-finish-watch-activation`. Only `tests/integration/mod_delivery.rs` changed (261 additions, 46 deletions), blob `66c5675729cce3c66a01ddd7c5b69d53c37dd50e`. Commit trailer truthfully lists that sole path. Parent's corresponding prior transfer was reported as `3f74131a`; this helper checkpoint is based on its original unformatted `8cd4fc07` source, so the parent owns normal formatting/transfer reconciliation and final executed-source mapping.

Read the full exact `watch-activation-task-review.md`. WA-R1 is addressed in source by taking diagnostic/result snapshots, dropping controller guards before ordinary assertions, recovering poisoned mutex/condvar state for shutdown, and recording worker invariant/write failures for the main oracle to report. Pull identification also releases the exchange-log guard before entering the ordering controller. Relay Drop shuts down its owned socket clones before acquiring the controller lock, then stops/wakes waiters, joins the worker and restores socket pathname/descriptor; the socket write has a bounded timeout as a fallback. This preserves the existing bidirectional child EOF path.

WA-R2 is addressed in source by moving the actual unchanged Attention frame prefix/body write into `forward_attention`, holding the same ordering mutex used for Pull observations throughout that write. Permission creates no forwarding marker. The controller rechecks the request count immediately before output, refusing an extra already-observed Pull. Only a completely successful write increments the forwarding order; partial/failed writes record an observable worker error and create no forwarding marker. The elected capacity oracle still requires a real daemon frame and an actual child Pull ordered after that successful forwarding. This is a serialized observer oracle, not a timing-based quiet window, a global client drain cap, or a claim about downstream model uptake.

Five new helper regression sources do not require an elected daemon, advertised capability, native engine or product-success fixtures. Exact Rust test names (integration target):

- `mod_delivery::activation_capacity_flow_oracle_unwind_restores_relay` — induces an immediate missing-Pull assertion, catches the original readable diagnostic, and checks non-poisoned state, joined/released worker, owned socket EOF, original descriptor bytes and restored socket pathname.
- `mod_delivery::activation_capacity_flow_oracle_assertions_do_not_poison` — induces invalid release, stimulus and ordering assertions and checks that each releases the controller guard before unwinding.
- `mod_delivery::activation_capacity_flow_stop_recovers_poison_and_releases_waiter` — deliberately poisons the controller and exercises poison-safe stop plus condvar reacquisition; a scoped worker and stop-on-drop guard bound failure cleanup.
- `mod_delivery::activation_capacity_flow_failed_or_unwritten_attention_has_no_marker` — checks permission alone, a body-write failure after accepted prefix bytes, observable failure diagnostics, and an extra Pull inserted between permission and output; none creates a forwarding marker.
- `mod_delivery::activation_capacity_flow_successful_frame_write_serializes_pull_observation` — writes actual framed helper bytes through a Unix socket pair, checks that both prefix/body writes exclude Pull observation under the controller lock, reads the exact framed bytes from the peer, and checks the subsequent Pull's order. These local helper bytes are not an activation stdout capture.

Parent execution filters, not run here:

```sh
nice cargo test --locked --all-features --test integration mod_delivery::activation_capacity_flow_ -- --nocapture
nice cargo test --locked --all-features --test integration mod_delivery::activation_v2_watch_full_pending_capacity_parks_until_real_channel_input -- --exact --nocapture
```

The first filter has exactly the five named new sources above; parent must confirm a nonzero selected/executed count. Each listed full helper name also supports individual execution with `-- --exact --nocapture`. The second remains the actual selected-consumer capacity regression and still requires the reviewed actual advertisement/activation source. The earlier four opaque dormant failures remain unclassified; this correction does not retroactively turn them into meaningful advertisement RED. Optional actual stdout capture contract is unchanged, and no capture artifact was produced by this helper.

Fresh source verification: `git diff HEAD^..HEAD --check` exited 0; `git status --short` was empty after the commit; exact HEAD/parent/blob resolved as above. No Cargo, formatting, lint, native execution, elected daemon, subprocess or helper process was started here. No compilation, runtime cleanup success, GREEN, RED or activation clearance is claimed. Parent owns five helper executions, elected dormant/active proof, scoped leak checks and independent scoped rereview.

**COMPLETE + QUIESCENT — source checkpoint only.**


## Cumulative interface audit correction — tokenless lazy-only page

Source-only checkpoint `c4f0eb00cb7b5b943aae5fbd85172c3dc382f7a0`, parent `0007ada15cdd131d4a358f8929ccd18cfe8dd6a0`, isolated helper branch unchanged. Sole changed path `tests/integration/mod_delivery.rs` (33 additions, 14 deletions), blob `2edc9a22a5a2c7ef0f58006163c7c007ecebe88e`; truthful Outside-adapter trailer lists that path.

Parent's new cumulative interface audit identified a prepared-test defect: the lazy-only elected case waited for a Confirm that expected production behavior never sends. I read the current D test at lines 2548 onward, current D `src/cli/watch.rs` token-gated Confirm at line 673, `src/store/invitation_content.rs` descriptor-gated outbox allocation (current line 2404 onward), and the `DeliveryPageV2` optional-token serialization contract. This correction changes no production source, D checkout, capability, token or confirmation behavior.

The unchanged exact test `mod_delivery::activation_v2_watch_lazy_only_page_is_passive` now observes the actual selected `delivery_pull_v2` Page containing its real lazy ID, checks the selected child's actual lease, exactly one item, omitted `presentation_token`, empty controls and absent canonical Watch outbox token. The page search includes already in-flight initial Pulls, because one may select the newly admitted lazy row; it does not incorrectly assume the request must have started after send. Actual stdout still must contain the lazy line with no required ACK. Pending lazy state, no mod ACK, no Attention, no native wake, a live passive channel after the existing 11-minute injected clock advance, and child liveness remain required. Explicit owned-child termination/reconnect-grace establishes the observed route's EOF boundary before inspecting its actual requests for absence of Confirm, ACK, acceptance and lazy completion. Pending state, absent outbox and no wake are checked again afterward. No invitation or Notice was added to the lazy-only fixture, and no successful stream fixture/capture was synthesized.

Parent filter (not executed here):

```sh
nice cargo test --locked --all-features --test integration mod_delivery::activation_v2_watch_lazy_only_page_is_passive -- --exact --nocapture
```

Parent reported all five elected dormant cases now fail specifically at the actual Capabilities missing-V2 assertion (`mod_delivery.rs:556`) in a total .547 seconds, log `/private/tmp/ht-finish-watch-actual-inactive.log`, scoped leaks zero. This is parent-reported meaningful advertisement RED for that parent source; it is not an active behavior result or execution of this new helper checkpoint. It supersedes the earlier unclassified opaque failures only for the new explicitly diagnosed parent run.

Fresh helper verification: `git diff HEAD^..HEAD --check` exited 0, post-commit `git status --short` empty, exact HEAD/parent/blob resolved above. No Cargo, fmt, lint, native execution, Herdr/tracker writes, subprocesses, elected daemon, or subagents were started. Parent owns transfer reconciliation, final exact-source mapping, compilation, active test execution and independent review. Optional actual stdout capture remains unchanged and unproduced here.

**COMPLETE + QUIESCENT — source checkpoint only.**


## Shared bootstrap native-wake investigation — diagnostic checkpoint

Checkpoint `8df87a36069333a47c54e7ca2a6f05a8918905df`, parent `c4f0eb00cb7b5b943aae5fbd85172c3dc382f7a0`, sole changed path `tests/integration/mod_delivery.rs` (69 additions, 9 deletions), blob `f3f0e96967ac381b1299306e916a2d04ac40a5cd`. Tests-only Outside-adapter trailer lists that path. Source frozen and no fix applied in this checkpoint.

Used the systematic-debugging skill, read parent logs `/private/tmp/ht-finish-first-active.log` and `/private/tmp/ht-finish-active-existing-mod.log`, and traced the current activated D wake/host/CLI paths read-only. Parent first active run at `7c3edc2d5d6131258d4f0dfca7e1788ecbe3a4e4` selected 32 tests (23 PASS, 9 FAIL); all five elected Watch cases passed the early advertisement guard but failed in common Rig bootstrap waiting for its native invitation wake. Parent's three existing controls (`send_attention_drain_ack_settles_with_mod_delivery_provenance`, `lazy_row_appended_then_completed`, `no_mod_or_env_off_keeps_native_wake_and_exits_3`) likewise all failed at the same bootstrap, before their bodies. Neither run establishes active Watch behavior. Parent reports scoped cleanup checks; this helper did not execute them.

Static findings: the private host record contains ping/pane.get/snapshot calls but no agent.get/prompt. The actual native submission path requires the agent.get recheck before composer read/prompt, so the observed stop precedes that final submission path. SessionStart exits 0 on fail-open paths, which makes canonical registration state relevant. More specifically, current D `src/cli/commands.rs:2228` chooses `InvitationConfirmation::Lazy` unless `--require-ack` or a deadline was explicit; the shared Rig bootstrap currently issues a plain invite and then waits for its native wake. The default-lazy/wake-oracle mismatch is a source-supported hypothesis, not a claimed runtime root cause. Per parent direction, this checkpoint preserves the original invitation producer, timeout, real wake requirement, canonical contracts and failing behavior to obtain the missing runtime evidence first.

Diagnostics are emitted only on the existing wake timeout. The Rig saves its exact own initial installed SessionStart exit code plus UTF-8-safe stdout/stderr prefixes (2048 bytes each, with original byte lengths and truncation flags). Timeout opens only its own SQLite database read-only with a 100ms busy timeout and reports bounded canonical B seat identity, last two occupant bindings, last two invitations including confirmation mode, and its wake_work row. Query/open errors become diagnostic values rather than secondary assertions. Recent private stand-in host calls include the last eight actual parameter objects (512-byte prefixes with lengths/flags), replacing the previous target-only formatting that hid pane.get arguments. Host guards are released before the original timeout panic; no production diagnostic interface or environment mutation was introduced.

Smallest parent reproduction (one existing control, unchanged name):

```sh
nice cargo test --locked --all-features --test integration mod_delivery::no_mod_or_env_off_keeps_native_wake_and_exits_3 -- --exact --nocapture
```

Parent should confirm a nonzero selected count and inspect bootstrap_invitation confirmation, binding registration/provenance and matching seat/terminal/incarnation identities alongside the own SessionStart output. If this demonstrates a registered healthy binding plus a lazy bootstrap invitation with no wake work, the narrow proposed fixture correction is an explicit `--require-ack` on that bootstrap invitation, preserving the default-lazy product contract and the native wake assertion. No such fix is included or runtime conclusion claimed here. No new minimal mode assertion is needed to run this unchanged existing selector; its actual canonical stored mode is included in the failure diagnostic.

Fresh source checks: exact HEAD/parent/blob resolved above, `git diff HEAD^..HEAD --check` exited 0, post-commit `git status --short` empty. No Cargo/fmt/lint/native execution, processes, production source, D writes, Herdr/tracker/config writes, foreign process inspection, or subagents. No capture produced, no runtime cleanup/active Watch GREEN claim. Parent owns reproduction and root-cause determination before any fix round.

**COMPLETE + QUIESCENT — diagnostic checkpoint ready for parent.**


## Shared bootstrap fixture correction after diagnostic reproduction

Source-only checkpoint `581decde600941a783d7d5a746d332c43488734e`, parent `8df87a36069333a47c54e7ca2a6f05a8918905df`, sole changed path `tests/integration/mod_delivery.rs` (3 additions, 1 deletion), blob `062febc86db140d342cad5a2145430df5c651091`. Truthful Outside-adapter trailer lists that sole path.

Read actual parent diagnostic log `/private/tmp/ht-finish-watch-bootstrap-diag.log`. Parent executed the one existing native-only control on transferred diagnostic source `b7787ac1920148a8c51a596536cca2a2e6a8c533`; one test selected, original wake timeout FAIL, total runtime 21.246 seconds. The diagnostic shows resolved B seat generation 2, matching current Claude cooperative_top_level binding/session SB/terminal/incarnation, non-null registered_at, SessionStart exit 0 and actual context stdout, bootstrap invitation pending with canonical confirmation `lazy`, and no wake_work row. This establishes the setup producer mismatch: its plain invitation correctly follows the new default-lazy contract and does not create the native wake that this setup oracle waits for. The earlier five new and three old failures are setup failures before their own behavior bodies, not active Watch evidence.

Per explicit parent instruction, the narrow correction adds `--require-ack` only to the shared pre-Watch bootstrap invitation whose existing oracle deliberately requires native wake. Two nearby comment lines state this intent. Its real wake wait, explicit later acceptance, wake-lane completion and injected spacing behavior stay intact; timeout unchanged. Existing diagnostics are retained. All later lazy/passive invitations/messages and the real default-lazy production contract remain unchanged. No production/D/config/Herdr writes were made.

The corrected source has not been executed here. Parent plans all five exact elected Watch cases plus three existing controls:

- `mod_delivery::activation_v2_elected_watch_capabilities_lease_output_confirm`
- `mod_delivery::activation_v2_watch_lazy_only_page_is_passive`
- `mod_delivery::activation_v2_watch_replacement_replays_uncommitted_output`
- `mod_delivery::activation_v2_watch_lost_confirm_response_replays_exact_subset`
- `mod_delivery::activation_v2_watch_full_pending_capacity_parks_until_real_channel_input`
- `mod_delivery::send_attention_drain_ack_settles_with_mod_delivery_provenance`
- `mod_delivery::lazy_row_appended_then_completed`
- `mod_delivery::no_mod_or_env_off_keeps_native_wake_and_exits_3`

Use the existing integration target and confirm nonzero selected/executed counts. Parent owns actual active behavior, capture production/digests, cleanup and review; no new test or production gate is introduced by this one-flag fixture change.

Fresh helper verification: exact HEAD/parent/blob resolved above, `git diff HEAD^..HEAD --check` exited 0, post-commit `git status --short` empty. No Cargo/fmt/lint/native execution, processes, subagents, tracker/Herdr/config writes, or D edits. This source checkpoint does not claim fixture GREEN, active Watch GREEN, successful cleanup at runtime, model uptake or activation clearance. No capture produced here.

**COMPLETE + QUIESCENT — source correction ready for parent execution.**


## Active fixture follow-up round 2 — real recovery attention and retirement diagnostics

Checkpoint `3e63da5b455f89f5cb5edfc5b1302ef6251e6d97`, parent `581decde600941a783d7d5a746d332c43488734e`, sole changed path `tests/integration/mod_delivery.rs` (60 additions, 5 deletions), blob `8dcf05b78e5e1d94aa06f824411c3b1ce7e2a2c6`. Outside-adapter trailer lists that sole path. This is a tests-only source checkpoint; no parent D writes or runtime performed here.

Read actual parent `/private/tmp/ht-finish-second-active.log` and traced current helper plus active D invitation, confirmation and retirement implementation. Parent executed 35 selected tests on actual `f8c25103`: 32 PASS, 3 FAIL, total 25.091 seconds. The elected lazy-only and full-pending-capacity cases and three existing mod controls passed; the three remaining Watch failures now reach their own behavior after corrected shared bootstrap. Parent reports all five AGY and 22 protocol tests pass in the same selection; these remain parent execution facts, not helper execution claims. Parent scoped cleanup was pending when this follow-up began.

Recovery root cause is source- and runtime-supported: both tests called the shared default-lazy `activation_invitation` and had no independent nudge-class producer. The actual lost-response stdout showed the invitation with `wakes:false` and no Attention; replacement waited for a nonexistent nonempty-control Confirm. The narrow correction leaves both invitations lazy and adds one actual `rig.send_ordinary` require-ACK message in each case, supplying the independent Attention condition needed for loss/replay. No blanket invitation mode change or synthesized capability/control was made. Both tests retain actual same-child lease/Pull/Confirm and owner replay/recovery assertions. Additional assertions require the invitation's actual stdout to remain `wakes:false`, the ordinary receipt to remain pending, zero mod-ACK events, and the canonical invitation to remain pending after presentation recovery. Default-lazy invitation/passivity and no-accept/no-ACK contracts stay explicit.

The mixed positive reached actual Confirmed with its exact acknowledged control subset before timing out on the global condition `(current is None, last token equals captured token, last mask nonzero)`. Source shows confirmation retires its specific token, while later independent Pull/Confirm pages can replace `last_token`/mask. This is a hypothesis for that remaining failure, not a concluded runtime root cause. Per parent direction, its exact failure condition and STEP timeout remain unchanged in this checkpoint. On failure only, the test now reports canonical current token, last token and last-confirmed mask, plus six newest actual Pull/Confirm exchanges summarized by submitted token/controls, actual page token/kind/more/item kinds+IDs, response status and acknowledged controls. Bodies and claim dumps are omitted; capture is bounded by the relay and six-exchange limit. No gating or weakened canonical oracle has been applied.

Parent should execute these exact three integration selectors (nonzero counts required):

```sh
nice cargo test --locked --all-features --test integration mod_delivery::activation_v2_watch_replacement_replays_uncommitted_output -- --exact --nocapture
nice cargo test --locked --all-features --test integration mod_delivery::activation_v2_watch_lost_confirm_response_replays_exact_subset -- --exact --nocapture
nice cargo test --locked --all-features --test integration mod_delivery::activation_v2_elected_watch_capabilities_lease_output_confirm -- --exact --nocapture
```

If actual diagnostics confirm successor token overwrite, the proposed later correction is to hold the original actual canonical Confirm reply after daemon commitment and before returning it to the child, assert retirement of that specific token and exact controls under that causal gate, then forward the unchanged actual response and permit later pages. This preserves canonical evidence and same-child output-before-Confirm; it does not forbid successor pages or drop retirement assertions. No such correction is included before parent reproduction.

Fresh source checks: exact HEAD/parent/blob above, `git diff HEAD^..HEAD --check` exited 0, `git status --short` empty after commit. No Cargo/fmt/lint/native execution, processes, subagents, production/D/Herdr/tracker/config writes. No actual capture produced here; optional capture contract unchanged. No active GREEN, cleanup success, model uptake or activation clearance is claimed for this new helper checkpoint.

**COMPLETE + QUIESCENT — source checkpoint frozen for parent's exact three selections.**
