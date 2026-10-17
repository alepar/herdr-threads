# Auto ACK integration test repair for the v2 candidate

Base: `a5f00b37f` on `super-auto/cross-node-sync` (cross-node sync, main v1.2.2 and inbox injection auto ACK). Repair branch: `fix/auto-ack-v2-tests`. Product code is unchanged. This branch is for the p2p release owner to integrate; it is not merged to main and does not release anything.

## Failure causes and retained checks

- Four hook entrypoint tests assumed receipts remained pending after complete injection or that messages displayed at startup would be delivered again after acceptance. Startup can display and ACK opening mail while its invitation stays pending. The tests now verify actual message rows, receipt settlement, explicit invitation acceptance, hook provenance, and quiet subsequent hooks. The three-thread startup follows its exact inbox continuation when the context budget leaves bodies undisplayed.
- The hook race test accounts for arrivals between digest, check-in and inbox reads being displayed in the same hook. A new race after the inbox snapshot proves the unseen receipt remains canonically pending and is delivered by the next hook. It uses the public pending query because the receipt projection can lag publication.
- The generated truncation marker now directs both ordinary and lazy rows to inbox. Its inverse test checks that shared marker, while retaining rejection of a different command prefix or a missing generated suffix and preserving body text that resembles a marker.
- Deadline and poke tests inspect read-only attention digests while receipts must remain pending, then verify that actual hook display settles them and stops pokes. A separate real-hook hold/release test retains the frozen summary frontier check, including the initial display ACK's added timeline event.
- Session binding tests verify the displaying Claude binding and hook provenance without an explicit ACK. Codex resume injects complete bodies and advances its cursor. Past a history gap, the retained small receipt body is displayed and settled by that same binding. Reconnection may append clear events before the page snapshot, so the cursor must cover at least the pre-resume head.

## Verification

- Baseline: all ten reported tests reproduced as failures, with no passing tests in that selection.
- Final focused run: ten passed, zero failed; 8.766 seconds test runtime. Build: 2.77 seconds.
- Independent read-only review: no remaining findings after final overflow-path correction.
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: passed (59.43 seconds).
- `scripts/check-harness-neutral`, `nice scripts/check-default-features`, `cargo fmt --check`, and `git diff --check`: passed.
- `HT_LEAK_RUN_ID=0cbaae8c-32c6-4fc9-929f-78c56cc3bde7 nice cargo nextest run --locked --all-targets --all-features`: passed, 7,591 tests, zero failures, 47 skipped; 279.760 seconds test runtime (under the five-minute budget). The release failpoint build check took 248.965 seconds. The reported cross-node text-inbox display flake passed in this full run.
- Scoped process and scratch-file checks passed for the baseline, every focused run and the full suite; no cleanup kills were needed.

Every test run has a unique `HT_LEAK_RUN_ID`; scoped process and scratch-file checks follow each run. The initial isolated build took 2m30s after warm-target seeding; later test edits rebuilt in seconds.
