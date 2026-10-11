# Task 15 — IN PROGRESS: regression preparation

Owned initial checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.8.4`, branch `task-ht-0lo.8.4`, base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`.
Native operation `eb22efb2-83a0-4a6b-9a85-00e2196cd989`; coordinator metadata `task-15-owned-metadata.json`. No Cargo or production changes before the first actual behavioral RED. Sibling B owns the exclusive Cargo lane; coordinator will move this owned checkout to the retained warm absolute slot before validation.

Read implementer, subagent-driven-development, TDD and good-tests instructions; actual AGENTS, trust-policy launch/receipt/invitation/origin rules, brief/preflight, upstream refinements and Task12 final report. No subagents, Herdr actions, integration/main merges, tracker/ledger mutations or real config writes.

Prepared actual regression hosts:

- `tests/service/delivery_pull.rs`: real canonical Invite after cached quiet pull; complete invitation context without attention, message offer, receipt or acceptance. Separate real departed A1/A2 physical and later B1 published sparse associations, actual service ACKs, coalesced single-condition lifetime and canonical per-origin views.
- `tests/hook_entrypoint/agy.rs`: built registered PreInvocation after quiet cache, canonical Lazy Invite through real CLI; pending episode and no receipt/acceptance/wake.
- `tests/harness/agy_launch.rs`: registry preparation with missing/stale/disabled/unreadable readiness; selected native argv remains valid and unsafe argv refused.
- `tests/cli/launch.rs`: guarded public launch equivalent readiness table, actual host submission and no fabricated configured hook.
- `tests/cli/watch.rs`: current actual watch drain and serializer must produce a passive invitation line with content and acceptance continuation, no message ACK semantics.

Direct rustfmt parsed all five changed test files. This is syntax/format evidence only, not compilation or behavioral RED/GREEN. No tests have been run yet; no completion claim.

Concrete production seams traced before changes: Assembler classifies Lazy invitations informational, drain_pages discards that item and DeliveryPull only consumes message Item. ModSeatView wake fingerprint excludes Lazy invitations, so cached quiet version hides arrival. AGY prepare_launch hard-refuses readiness; neutral core fingerprints status-selected optional hook before actual preparation. Watch supports only message/lazy/attention, and old Claude mod ignores additive invitation kind.

Coordinator admitted a narrow passive-content field in src/ports.rs and additive watch/protocol/mod consumers; requested existing test hosts are delivery.test.ts, cli/watch.rs, protocol/watch.rs if needed. Recipient-specific bounded signature proposal supersedes global decision_seq: hash the existing canonical pending invitation WINDOW (1001 candidates) IDs/publication keys and saturation. This proposal still awaits final coordinator ruling and actual mutation/bounds proof; no source exists yet.


## Real isolated mod TDD (Rust lane untouched)

Run command: `HT_LEAK_RUN_ID=<scoped UUID> HOME=/private/tmp/ht-task15-mod-home CLAUDE_CONFIG_DIR=/private/tmp/ht-task15-mod-config scripts/test-claude-mod`. Script creates a further private config directory, validates plugin strictly and runs actual proprietary mod test runner; no model/network/live agy invoked.

1. RED log `/private/tmp/ht-task15-mod-red.log`, UUID `49bd99b8-9ce4-4ff1-b3ac-0bfc998dc80e`: 95 pass/1 fail, passive invitation expected append count 1 but got 0. Existing createCore discarded invitation kinds.
2. Minimal mod-only production fix adds invitation ingestion, explicit no-ACK semantics, and uses existing passive append path without idle-submit eligibility. GREEN `/private/tmp/ht-task15-mod-green.log`, UUID `cc0671b9-c526-4d75-aec3-7d5b1daa7499`: 96/96, test runtime 0.71s.
3. RED `/private/tmp/ht-task15-mod-cut-red.log`, UUID `2144c3f3-c98f-43cb-aa28-974b09c2c006`: 97 pass/1 fail, truncated invitation incorrectly marked whole-delivered as append. Timed invitation wake/no-message-ACK control already passed.
4. Minimal whole-display fix excludes truncated invitations from delivered bookkeeping. GREEN `/private/tmp/ht-task15-mod-cut-green.log`, UUID `974b824a-f2e5-4f2e-96da-e891a845243f`: 98/98, runtime 0.62s. Existing stress and byte-for-byte frame goldens pass. Main's unprefixed delivered log unchanged.

All four UUID-scoped leak checks passed with no leaked test processes. Initial sandboxed checker could not execute ps; narrow approved read-only escalation succeeded, no processes killed. Rust production is unchanged; Rust tests have only been parsed/formatted, not built/run. No source commit yet.

Backward compatibility is explicit: old mod consumers ignore the additive invitation kind and lack new complete invitation context until updated. Existing message/lazy/attention wire shapes and frame goldens are unchanged. Invitation records never become message ACKs; timed invitation may wake through existing idle guards, passive invitation never submits a wake, and truncated display never claims completeness.


## Warm validation transfer

Coordinator transferred the unchanged dirty owned checkout to `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`, branch `task-ht-0lo.8.4`, HEAD ee1e598c3ee96566ddc9b397185cdc97c8bf6218. Old source path no longer exists. All seven modified paths preserved; all seven manifest target paths verified in `task-15-warm-metadata.json`. Exact retained target is `<new owned source>/target`; no cache clone or concurrent Cargo writer. All subsequent Rust validation uses explicit CARGO_TARGET_DIR there and CARGO_BUILD_JOBS=4.

Settled origin ruling: after full warning offer, repeated identical-version pull may be Unchanged. Canonical open-origin mod/native eligibility remains independently actionable. Do not force repeated context injection or a wake every pull. The presentation ledger cannot consume a condition; final actual ACK passively clears it. Native consumer must not use NextPull/items as authority for open-condition lifetime.

## First actual Rust RED selections and source-only phase

First warm source/cache selection compiled in 2m13s after B's partial-feature source switched to A-based D with five Rust fixture edits, CARGO_BUILD_JOBS=4. This is not a controlled single-edit speed probe. Command prefix was `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 HT_LEAK_RUN_ID=034741aa-41d6-4b99-bfb8-390215dd10ad nice cargo test --locked --all-features --lib <filter>`.

- `a_lazy_invitation_reaches_hook_pull_without_attention_or_acceptance`: 0/1, 4197 filtered, 0.18s. Expected Page after quiet pull, got Unchanged. Log `/private/tmp/ht-task15-rust-red.log`.
- `departed_receipt_origins_survive_pull_offers_late_association_and_partial_ack`: 1/1, 1.01s, compile0.17s. Existing canonical origin behavior passes; original fixture did not cover actual scheduler presentation gate. Log `/private/tmp/ht-task15-origin-red.log`.
- `advisory_plugin_readiness_keeps_native_launch_available_without_fabricated_hooks`: 0/1, .00s, compile.03s. Missing plugin returned MissingHook. Log `/private/tmp/ht-task15-agy-launch-red.log`.
- `agy_guarded_launch_keeps_plugin_readiness_advisory_and_native_guards`: 0/1, .10s, compile.15s. Public guarded launch returned MissingHook before host submission. Log `/private/tmp/ht-task15-public-launch-red.log`.
- `watch_carries_a_passive_invitation_as_invitation_content_never_a_message`: 0/1, .00s, compile.11s. Actual watch drain emitted0 lines, expected1. Log `/private/tmp/ht-task15-watch-red.log`.

Scoped process checker passed with no leaks. No hook_entrypoint rebuild. Released lane and froze all source before coordinator restored D to its original task-ht-0lo.8.4 source path (warm slot/cache loaned back to B). A final read-only status observed B in the old warm path during transition; no writes or builds ran there after lease release. All further D operations use restored owned source path.

Minimal production source prepared after those REDs: advisory AGY readiness (selected hook only for current explicitly enabled integration), neutral core fingerprint based on actual preparation hook, distinct typed invitation drain/watch context, canonical recipient-specific WINDOW1001 pending invitation key hash reused from seat_digest, and separate per-binding invitation whole-display set plus bounded/spaced truncated attempt history. Invitation caches bounded1024 per binding and share4096-seat ledger LRU; no message receipt ACK or acceptance, no native wake outlook membership. Direct rustfmt parses; diff whitespace check and harness-neutral gate pass (0 violations). No Rust GREEN yet.

New fixtures awaiting next actual lease: full goal/old omitted-goal inspect ordering, actual canonical Lazy Invite to live ModChannelRegistry discovery, passive registry stall history, canonical accept/reject/retire/unrelated decisions, actual installed stable AGY event argv, strengthened real ScheduledStore/Scheduler origin-warning path after whole offer and same-version Unchanged. Goal producer, registry, stable-entry and scheduler production remain unchanged pending those REDs.


## Second lease: actual producers and canonical scheduler

Coordinator moved the same D branch and source into the exact warm .3.2 source/cache slot again; provenance task-15-warm-metadata-2.json. Exclusive jobs4 Cargo lease. No edits during compilation. Initial multi-file branch-switch build2m25s; this is not a controlled single-edit measurement. Scoped run UUID d608e944-f4a8-4397-b762-590981143085.

Actual RED selections (registered lib target, nonzero counts): stable installed AGY command0/1 (.13s, legacy hook agy producer); canonical invitation lifecycle/channel0/2 (.21s, missing passive channel signal and unchanged topic signature); goal renderer0/1 (.00s, missing goal); passive registry0/1 (.00s, missing drain signal); real canonical goal pull0/1 (.11s, missing goal). Logs /private/tmp/ht-task15-{stable,canonical,goal,passive-registry,goal-pull}-red.log. Initial scheduler fixture0/1 was an elected-daemon-boot setup error and did not authorize production. Corrected elected-boot real ScheduledStore/Scheduler fixture then failed behavior0/1 (.24s), log /private/tmp/ht-task15-origin-scheduler-red2.log; multi-file compilation1m50s. Actual canonical departed origin received no wake after whole warning offer and same-version Unchanged.

After relevant REDs, implemented bounded topic_revision join, goal_data preservation and explicit pending-recipient thread show inspection before acceptance; independent passive channel drain signal preserves last_attention_push; installed AGY handlers use stable hook-event with exact route. All were compiled in the batched build. Cached prepared GREEN7/7: canonical lifecycle/channel2, actual lazy goal hook-pull1, renderer inspection1, passive registry1, installed stable command1, bounded V2 goal/revision query and usable pending inspection1. Each cached compilation .04-.20s; selected runtime .00-.62s.

After cached GREENs, ONLY src/scheduler/mod.rs changed (7 lines) to bypass presentation NextPull suppression for exactly actionable_warning_seq present AND not warning_offered_for_current_occupant. The same Cargo lib command/filter/features/jobs4/source/cache as the scheduler RED compiled21.19s, actual test1/1 passed .28s: /private/tmp/ht-task15-origin-scheduler-green.log. This is controlled one-source-file incremental evidence for this required check, not a universal timing claim. Canonical attention, witness, continuity, mod suppression, idle/presence, backoff/coalescing guards remain. Scoped leak checker passed. No live AGY, provider/network, full suite or real config writes.

Source froze32 dirty paths at HEADee1e before lease release. Coordinator restored own .8.4 path, all32 preserved; B now owns warm slot/cache and D is source-only.

## Additional precision/completeness controls (source-only Rust)

JS canonical revision RED98pass/1fail (same ID revised context suppressed), then GREEN99/99, logs /private/tmp/ht-task15-mod-revision-{red,green}.log; scoped UUIDs b4072f65-dc31-4530-b5b4-317ac5b66341 and ca016b54-fb61-469e-a250-b8f61d25f415, leak checks passed. Watch revision carries canonical decimal text to preserve u64 bits through JavaScript; original invitation ID unchanged, historical optional omission preserved.

Isolated mod controls now101/101 (.60s), /private/tmp/ht-task15-mod-precision-completeness.log, scoped UUID efdbd16e-76cd-4e1c-9b24-d7d3e797fc27, leak check passed. Controls assert distinct 9007199254740992 / 9007199254740993 / 18446744073709551615, duplicate suppression, unchanged original ID, no ACK/native submit, and incomplete omitted-goal offer followed by full same-revision goal. No additional JS production needed for these controls.

New Rust assembler regression an_omitted_invitation_goal_is_incomplete_until_full_context_at_the_same_revision prepared, not yet compiled/run: existing goal omission currently incorrectly counts whole and can suppress later full same-revision context. Producer fix withheld until actual RED. Concrete unresolved saturation audit: newest1001 canonical content signature versus oldest-first all-invitation V2 walk and1024 whole ledger can miss old context edits or fill capacity ahead of arrivals; canonical-window/full-inbox contract raised to coordinator. No completeness claim until resolved and tested.


## In-flight completeness regression

Actual isolated JS RED /private/tmp/ht-task15-mod-inflight-goal-red.log:101pass/1fail of102,1.16s, expected a second append for newly available full same-ID/revision goal but received1. Scoped60d475a5-c241-40f6-bfdf-4fbcad8b609e leak check passed. Parent authorized this independently of the canonical algorithm design hold. Minimal JS fix lets fuller queued context survive a partial duplicate and completes only the actual invitation queue object, preserving a newer item. GREEN102/102 .60s (/private/tmp/ht-task15-mod-inflight-goal-green.log, scoped5c885a6b-4414-4927-9a58-b2f25ab3cb53); strengthened exact two-item→one-item→empty queue and whole duplicate suppression controls also102/102 .62s (/private/tmp/ht-task15-mod-inflight-goal-final.log, scoped9dca852a-4ad0-4858-9acd-4dc5679aad24). Both leak checks passed. Message ACK, idle passive wake, existing stress/frame goldens and unprefixed log unchanged.

Prepared unrun Rust consumer counterexamples:1002 canonical dispatch invitations, exhaustive display/quiet, then actual SetTopic oldest outside newest1001 signature;1025 canonical dispatch invitations must show final context before quiet despite1024 display cache. MAX_DRAIN_PAGES4096 bounds one invocation; every next pull restarts cursor=None, so it is not persistent progress. New actual departed-inviter deadline fixture covers InviteRequireAck→Leave→bounded due/materialization→whole pull→Unchanged→canonical origin eligibility→actual Accept and immutable clear. No canonical progress/projection producer changes before persisted refinement and fresh boundedness review.

Updated mapped tests/integration/agy_sweep.rs bootstrap to explicit plain-invite --require-ack because it intentionally waits for a timed invitation idlewake. Existing prompt wait/timeouts preserved; --require-invite-ack is handoff-only. Source33dirtypaths, no Cargo in restored .8.4, no running test helpers. Additional extraction/store-query omission→full same-revision fixtures remain unrun pending lease. Existing extraction expected-body-drop sequences, advisory launch controls, actual hook_entrypoint and old omitted protocol exactbyte tests require focused validation before any completion claim.


## Model-visible text accounting (current draft, final algorithm still held)

New invitation body adds a seven-byte newline+goal prefix plus the canonical goal (maximum1024 UTF-8 bytes), or21 fixed bytes for omitted-goal/next framing plus one pinned thread show command. Acceptance uses the existing seven-byte newline+next prefix plus one pinned accept/accept-required command with original IDs. Requested deadline adds a short deadline line (roughly11-byte prefix plus decimal milliseconds). Typical full context is topic+goal at most about2KiB plus command/header bytes; no static delivery explanation is repeated. Goal omission is explicit and inspect precedes acceptance. Existing BODY_LIMIT_BYTES8KiB truncation, watch body bounds,32 blocks/64KiB pull frame and selected encoded page limits remain the verification boundaries; command length varies with exact pinned routing paths and IDs. Revision is additive metadata only, decimal text on watch, never new identity/provenance or repeated model-visible prose. Stable wake marker unchanged/content-free. No actionable pull hint string introduced by the scheduler fix.

Parent selected coalesced eventual-latest topic semantics after per-thread updates quiesce, subject to concrete persisted bounds/interface/duplicate proof and fresh review before algorithm code. No every-transient-revision promise; invitation/receipt due driver guarantees untouched. Novel canonical algorithm remains unimplemented; initial WINDOW1001 approach explicitly lacks full-pending completeness at saturation.


## Third lease: forced source rebuild and bounded counterexamples

Coordinator moved D to exact warm .3.2 slot, HEAD ee1e,33dirtypaths, provenance task-15-restored-source-provenance-3.json. All486 owned Rust/Cargo source roots touched to force crate rebuild; initial diff SHA256 b463b9d72fa1e07bf70be3ba274b35197339229295ace1624f4697277bec7a0f. Verified actual Compiling herdr-threads warm source line and unique selected tests. All commands explicit CARGO_TARGET_DIR warm/target, jobs4/nice/locked/allfeatures/lib, scoped27ef5726-d155-46c9-87d5-679f2a49c7a4. No edits during compile.

Initial compile-only error: new1025 SQL count fixture used unsupported usize FromSql; corrected to i64 after command drained. Not behavioral RED. Actual rebuilt omission RED0/1,4207filtered,.01s, build2m13s (/private/tmp/ht-task15-omission-red2.log): omitted goal falsely whole. Rebuild follows forced486-root invalidation, not a controlled one-source measurement.

Actual canonical saturation REDs: /private/tmp/ht-task15-window-red.log0/1,7.67s; real1002 dispatch-created pending invitations exhausted/displayed old context and reached quiet, then actual SetTopic on oldest led actual DeliveryPull to Unchanged. /private/tmp/ht-task15-a_finite_invitation_backlog_progresses_beyond_the_whole_display_cache.log0/1,7.68s;1025 canonical dispatch invitations reached quiet without last item displayed. No novel projection/progress producer changes; both failures deliberately remain until parent-approved M redesign.

Actual selected byte-budget query RED0/1,.04s: /private/tmp/ht-task15-v2_invitation_goal_and_revision_fit_or_yield_to_usable_pending_inspection.log. Actual2048-byte goal omission falsely whole before later65536-byte same-revision full goal. Departed inviter lifecycle1/1 PASS,.08s: /private/tmp/ht-task15-a_departed_inviter_remains_actionable_after_whole_deadline_warning_until_acceptance.log. It uses actual timed Invite/Leave/bounded due materialization/full offer/same-version Unchanged/canonical origin eligibility/actual Accept/one immutable clear.

Minimal admitted production fix src/delivery/assemble.rs: goal_data=None is incomplete/truncated independently of byte-cut condition, so explicit usable inspection remains before acceptance and no false whole cache entry. Byte limit truncation still cuts only when body exceeds limit. Actual omission-to-full assembler/query passed. Existing extraction host first15/18 then17/18 exposed old drop-body expectations and a real timed invitation retract/reactivate bug: complete-context dedupe suppressed still-pending timed attention after a retract. ActualRED /private/tmp/ht-task15-extraction-omission-green.log17pass/1fail (timed reactivation expected attention5, got Exhausted), multi-file build27.42s. Narrow assemble-only fix classifies waking before whole-display dedupe; waking duplicate returns Attention, passive duplicate remains Duplicate, no repeated full body. Exact same extraction command/features/jobs/cache rebuilt16.77s with ONLY assemble.rs changed: controlled required incremental evidence. Existing timed reactivation passed; new omission fixture then correctly needed explicit Lazy for passive Duplicate semantics. No performance-only repeat.

Actual broad cached failures audited before expected test updates: AGYlaunch19/21 (old missing/disabled veto expectations), watch41/44 (old invitation drop-body sequences), query duplicate expectation legacy timed. Modchannels40/40 already passed. Preserve/strengthen assertions: extraction/watch full-context helpers include goal, exact invitation/message/attention order and pinned accept command; explicit omission fixtures strip goal; Lazy omitted→full tests suppress duplicate full body; legacy timed query duplicate remains Attention. Missing/disabled launch tests now assert safe native argv, no fabricated hook, and setup/enable diagnostic through registry+policy; structural/native argv and actually-used configuration mutation negatives retained. Deliberate --print fixture asserts actual resolve_scope refusal instead of helper unwrap. One test-only diagnostic field error (warnings are report[agy][warnings]) was corrected after compile drained; excluded from behavioral evidence.

Final compact test-only multifile build55.34s, then actual focusedGREEN124/124:
- extraction18/18,.01s: /private/tmp/ht-task15-third-final-extraction2.log
- actual byte-budget goal/revision query1/1,.17s: /private/tmp/ht-task15-third-final-v2_invitation_goal_and_revision_fit_or_yield_to_usable_pending_inspection.log
- AGYlaunch21/21,.62s: /private/tmp/ht-task15-third-final-harness__agy__launch__tests__.log
- watch44/44,.31s: /private/tmp/ht-task15-third-final-cli__watch__tests__.log
- modchannels40/40,.10s: /private/tmp/ht-task15-third-final-service__mod_channels__tests__.log

Actual guarded public advisory launch separately1/1 PASS,.17s, /private/tmp/ht-task15-third-agy_guarded_launch_keeps_plugin_readiness_advisory_and_native_guards.log; passive watch separately1/1 PASS. Actual missing/stale/disabled/unknown adapter readiness table now passes within21-test module. Scoped leak check passed, all processes stopped. No full/default-feature suite or live AGY/network/provider calls. Final clippy/fmt/neutral remains after approved algorithm; no redundant partial build gate.

Third lease RELEASED frozen33dirtypaths, diffSHA256468a1197a7a9e1846b359decef7cf555fc1fab1e0cd495be5de1fb5c2ee3e888, no own commits. Await coordinator restored path before source operations. Remaining validation: service delivery_pull broader host excluding only held saturation cases until algorithm; cli launch actual-used fingerprint mutation controls; store attention/modview; actual registered hook_entrypoint AGY; combined historical omitted exactbyte codec; final gates and full exact-tip review. Initial Ensure Lazy producer coverage belongs composed B+D tip because A-based ensure_direct_thread still writes historical timed invitation; D has not altered B-owned producer.
