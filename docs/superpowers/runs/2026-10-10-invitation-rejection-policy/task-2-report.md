# Task 2 — ht-xms.2 runtime implementation report

Status: implementation complete, focused verification passed, committed for independent review.
Commit: `3932108e` — Deliver native invitation refusals as passive warning notices
Branch/worktree: `task-ht-xms.2`, `/Users/alepar/AleCode/herdr-threads/.worktrees/invitation-rejection-runtime`

## Change

New native refusals append the original retained `reject:<invitation>` event as `warn`, preserving payload, source invitation and native actor. The same deciding transaction enqueues the existing bounded warning attribution/work jobs with frozen membership high water and recipient decision cutoff. The audience remains decision-time members plus the rejecting seat. No migration or parallel event was added.

One indexed canonical classifier requires the retained rejection, exact source/thread/key, native actor and matching retained payload. It is shared by effective actionability, informational pending detection, projected backlog suppression and passive wake classification. Worker attribution writes the native notice into the existing `digest_programmatic_warnings` table inside the same savepoint as recipient/cursor progress. No warning attention bump is generated. The existing current-binding notice frontier settles only carried bounded prefixes. Historical info-event replay remains unchanged and creates no fanout.

TRUST-POLICY A7 now documents native rejection notices, frozen audience and passive semantics. Existing exact-ID, required membership, binding, provenance and receipt rules are preserved.

## Verification

All Cargo commands resolved the package from the runtime task worktree and reused `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/invitation-rejection-policy/target`.

- RED: `nice cargo test --locked --all-features --lib store::control::tests::invitation_rejection_delivers_reason_once_as_a_thread_warning` failed against old production code at the expected assertion (`info` vs `warn`). Compile 2m55s, test 0.11s.
- Final GREEN: `nice cargo test --locked --all-features --lib store::control::tests::invitation_rejection` passed 13/13. Final incremental test-only rebuild 15.92s, tests 0.76s.
- Runtime library binary `target/debug/deps/herdr_threads-a1eddcbd246c5614 store::materialization::tests::` passed 21/21.
- Same runtime binary with filters `store::receipts::tests::`, `store::queries::tests::inbox_batch_keeps_open_and_clear`, `store::queries::tests::active_warnings_union`, `store::attention::tests::saturated_other_seat_transitions`, and `store::control::tests::rejection_retains` passed 69/69.
- Same binary with filters `store::cooperative_checkin_tests::builtin_warning_dedup`, `store::cooperative_checkin_tests::programmatic_notices_settle_page_by_page`, `store::connection::tests::delayed_warning_attribution`, `store::connection::tests::invitation_warning_clear`, and `cli::journal::tests::invitation_rejection` passed 12/12.
- `cargo fmt --check` and `git diff --check` passed before commit.

New tests cover actual member inbox/system reason/source/native author; pre/post one-unit attribution visibility and canonical wake suppression; same/fresh-operation replay without redelivery; invalid reason, wrong thread and wrong invitation without effects; projection failure rollback/retry with no lost or duplicate notice; delayed projection after an older global offer watermark; frozen recipients despite later join/leave; independent second invitation rejection; bounded carried prefix and stale execution frontier; historical info replay; and classifier negatives for wrong key/source/actor/JSON, built-in author and absent rejection ledger. Existing focused rejection race/required/current-binding/receipt/history/provenance tests pass.

## Self-review and limitations

Self-reviewed all seven owned-file diffs and acceptance requirements; no remaining issue found. Runtime checkout is clean after commit. The deliberately transferred baseline tests are included in this commit.

Root owns independent review, combined all-target/all-feature clippy and default-feature checks, and serial merge. No full suite was run per AGENTS.md. No daemons or helper children were started, no Herdr writes were made, and no shared configuration was changed. `nice` reported sandbox inability to adjust process priority but the verification commands executed normally. The first worktree commit attempt encountered the filesystem boundary; the authorized isolated-branch commit then succeeded through the escalation reviewer.
