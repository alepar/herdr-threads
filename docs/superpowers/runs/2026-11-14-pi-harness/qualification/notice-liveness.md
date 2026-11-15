# Parked notice handoff and scheduled retry

A retained old presentation page does not replace the newer canonical warning wake frontier. After the first submitted wake is heard from, that same pending warning receives another automatic wake at the existing retry boundary, without a new publication or manual turn. This is a paired canonical-store and scheduler proof; the actual SDK after-settle callback ordering is covered separately.

No production source, policy, timing budget or authority changed. The canonical test uses a genuine **nonwaking unavailable-member Notice**, not a resolved clear Notice. Its counterpart is a genuine newer **origin receipt-overdue warning**. All Notice descriptors and warning metadata come from the existing producer/presentation paths; the test does not manufacture captured metadata or warnings with SQL.

`late_notice_successor_preserves_newer_canonical_warning_wake_frontier` in tests/store/notice_replay.rs:

- Creates the actual old unavailable Notice, removes the fixture's invitation with public Accept, and captures a deferred Watch page before the newer cause exists.
- Registers a real agent sink, sends to it with an explicit ACK requirement/deadline through the production service, then runs the canonical overdue writer and bounded warning work. The old Notice is verified nonwaking and the newer warning waking.
- Requires the source seat's candidate to have no pending invitation, receipt or admitted ordinary/lazy fallback. Its authoritative wake witness names the newer warning publication while Watch still replays the old token/body and omits the successor.
- Confirms the old token, checks that only its old projection ordinal advances, then pulls a different token containing the newer Notice. Buffering that successor does not advance its notice frontier. The canonical logical wake frontier remains exactly equal before confirmation, after confirmation and after successor allocation.

`late_notice_successor_retries_same_warning_frontier_after_contact` in tests/scheduler/wake_receipt.rs:

- Uses the real Scheduler, existing fake monotonic clock/durable-ladder fixture and a recording notifier. The candidate contains only actionable-warning publication key 9; no invitation, receipt or ordinary/lazy key can force retry.
- Uses real DeliveryPullState contact tracking, with an explicitly Nothing ordinary pull outlook, and a real live ModChannelRegistry registration using the adapter's declared IdleWake::Native policy. A live channel is asserted not to suppress native wake.
- Submits the first scheduler attempt, keeps targets/publication frontier unchanged, and advances only the existing fake clock. One millisecond before the 30-second retry boundary produces no new attempt.
- At the boundary, the silent negative control records unreceived and types nothing. The sole causal difference in the positive case is real record_contact after the first scheduler attempt; it produces reserve attempt-2 and typed, with the same warning frontier and no new message or user turn.

The scheduler fixture does not fabricate agent_settled or claim native callback timing. Its completed scheduler attempt is distinct from the native agent's settled event. The paired actual SDK regression holds confirmation stdout across real agent_settled and verifies successor uptake afterward; its manually stimulated second public prompt is not itself scheduler evidence. These two layers supply the separate ordering and automatic-retry proofs.

Causal RED deliberately omitted record_contact only: the positive case failed with 0 retries versus 1 expected, while the silent control passed (111 s build, 0.00 s test). GREEN added that real contact event only: both new tests passed (39.22 s build, 0.72 s tests). The allocated canonical ordinals are real and asserted increasing; the scheduler's fixed logical key 9 is a separate state-machine fixture, not a claim that canonical allocation happened to choose a literal 7/9 pair.

Focused compatibility gate: 14/14 existing and new wake-receipt/canonical presentation tests passed (0.02 s build, 0.534 s execution). All-target/all-feature clippy passed without warnings (17.37 s; real 17.42 s). Neutral and peer-wire guards passed with zero violations. Final format passed (7.89 s), git diff --check passed, and scoped host/builder process and scratch checks found no owned leftovers. The host checker consumed the clean scratch ledger before the builder repeat. Independent source review approved the frozen test diff without findings. No full suite, live run, real profile/service, credentials or production change was added.

Retained private evidence: `.superpowers/sdd/implementation-plan/notice-liveness-gates/` contains red.log, green.log, focused.log, clippy.log, neutral.log, wire.log, fmt.log, cleanup logs and the frozen test diff. Frozen three-file test diff SHA256: `4c6ebe33fc5abeae4a5e0573f2e9fe253fe52e301bd5969b06c31a50b42dcc5a`.
