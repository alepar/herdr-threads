# Final review: cluster 30 (tests-integration), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify <sha>^{commit}`). The caller checkout was at HEAD c3e9ab0be with a clean status at the start and at the end of the review. The bind-review-source guard exited 0.

Cluster inputs: slices 000169-000175 covering tests/integration/{herdr_release_smoke,lazy_attention,notice_wake,seat_config_sweep,summary_flow,sweep_remaining,topology_activation}.rs, plus the digest. To check named risks I also opened tests/fixtures/herdr-matrix/codex.js (the stand-in the smoke test drives), tests/fixtures/wake_intent.rs, tests/fixtures/codex-0.163.0-side/session-start-fork.json, tests/combined.rs, tests/integration/lazy_config_smoke.rs (World::raw), docs/evidence/poke-spike/captures/codex-q6-workers.read-detection.txt, the run's roast and findings ledger, and the ht-z41e.10 and ht-z41e.9 bead status.

## What the cluster changes

- seat_config_sweep.rs is new and registered in tests/combined.rs. Flow 1 drives the public CLI against an isolated daemon. It checks that `config seat list` names every registry key and the human spelling, then sets `human config seat set --seat S programmatic_wake on` with no caller seat. Afterwards `get` reads `on` for S and `off (default)` for the other seat, `doctor --json` lists only S, and the store's `effective_value` agrees. Flow 2 has an agent set its own `nudge_threshold` through `config seat set`. The wake frontier honours it (p3 wakes at T=3, p4 does not), `unset` returns it to `0 (default)`, and an agent write of `programmatic_wake` is refused with the `ht human config seat set ...` spelling. Flow 3 enrols a `/side` fork (real SessionStart payload, null transcript_path, same pid) beside the root. It checks that `queue_wake_selection` targets the root and lists the side in `side_sessions`, and that `read_composer` reads the side placeholder as non-Empty.
- herdr_release_smoke.rs `herdr_release_wake` now expects the opposite under `/side`. A p0 sent while the side placeholder is shown must type nothing for 10 s. After Ctrl-C closes the side view (the new `side-close` branch in codex.js), the agent must wake within 60 s. A 32 s spacing sleep comes before the send, so the earlier p3 wake's spacing does not make the negative window pass vacuously.
- The other five files add only `auxiliary_session: false` to CallerClaim and LifecycleEvent literals. This follows the new `#[serde(default)]` field from ht-z41e.21 and changes no behaviour.

## Requirement baseline trace (cluster scope)

- B3, B4, B6 and B7 are exercised through the real CLI binary and a real isolated daemon: list, get, set and unset on both tiers, the agent-writable `nudge_threshold`, the human-only `programmatic_wake`, and reads of every key from the agent path. B5 is covered by `list`, which carries the descriptions and the human spelling, and by the agent-path refusal, which names the exact human command.
- B1 (seat-level programmatic_wake) is covered down to the store value that queue selection reads. Which seat a queue selection picks is proved at the seam level in tests/store/wake_queue_cost.rs. The production selection path belongs to the seam reviewer.
- B8 (queue routing to the root beside `/side`) is covered through the library `queue_wake_selection` port, with a real hook payload for the side session. End-to-end live evidence is the ht-z41e.10 row, which is still open.
- B9 (no native typing into a side chat) is covered at the composer-read level in flow 3, and on a real Herdr host only by the ignored `herdr_release_wake`, which has not been executed (see the untested items below).

## Verification run by this review

- In a review-owned detached worktree at c3e9ab0be (target reflinked from the caller's debug build, removed afterwards), `cargo test --locked --all-features --test combined -- seat_config_sweep lazy_attention notice_wake` passed 18 of 18. That includes all three seat_config_sweep flows at head, re-run after the ht-z41e.21 and ht-z41e.22 changes; the retained evidence/ht-z41e.11 log predates them. The log is at /tmp/claude-1001/-var-home-<user>-AleCode-herdr-threads/57269490-82ff-4bd7-a4a5-c012f2ee2266/scratchpad/c30-test.log. No process referencing the review worktree survived.
- summary_flow, sweep_remaining, topology_activation and herdr_release_smoke are separate targets. They were not executed here, because their change is a mechanical field addition that clippy type-checks (evidence/ht-z41e.11/checks.log, rc=0).

## Findings

- [Nit] tests/integration/seat_config_sweep.rs:7 — the module doc says `per_seat_flag_selects_queue_only_for_the_flagged_seat` belongs to `store::wake_intents::selection`. It is actually in tests/store/wake_queue_cost.rs:57, so a reader following the reference will not find it.
- [Nit] tests/integration/seat_config_sweep.rs flow 2 — `w.text(1, false, &["nudge-threshold"]).contains('3')` is a weak check. Any '3' in the output (a seat id, a path) satisfies it. The adjacent `config seat get` assertion carries the real proof.
- [Nit] tests/integration/herdr_release_smoke.rs:1415 — the `/side` negative check sleeps a fixed 10 s and has no witness that a wake attempt was made and refused. The ht-z41e.1 roast already recorded and rejected this as a Nit, and the positive wake after close shows the item was owed.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `herdr_release_wake` (tests/integration/herdr_release_smoke.rs:1175, ignored) has never been executed. That leaves unrun its inverted `/side` expectation, the codex.js Ctrl-C `side-close` branch, and whether Herdr `pane send-text` delivers a bare `\x03` (no other call in the file sends a control byte). It is scheduled in ht-z41e.10 (open, gated behind ht-z41e.9) through scripts/herdr-release-smoke at the final SHA.
- B8 queue routing to the root beside a displayed `/side` chat has no live evidence. Flow 3 covers it only through the library selection port, and live side-queue and side-native-defer runs are the ht-z41e.10 `live-qualification` row (requiredLive, open).
- Flow 3's native half checks only `read_composer` on a side placeholder, not a deferred wake through the host. The per-seam test is `host::native` `cooperative_wake_defers_on_a_side_composer_and_prompts_once_it_reads_empty`.
- summary_flow, sweep_remaining and topology_activation were not executed by this review after their `auxiliary_session: false` literal additions; only compile-checked by clippy.

## Deferred OK
- Fix the seat_config_sweep.rs module doc's location for `per_seat_flag_selects_queue_only_for_the_flagged_seat` (it lives in tests/store/wake_queue_cost.rs).
- Tighten the flow 2 `nudge-threshold` output check from `contains('3')` to the exact threshold line.
- In herdr_release_smoke, replace the fixed 10 s `/side` sleep with a wait on the wake lane's refusal record (Nit already recorded by the ht-z41e.1 roast).
- The `wakes_on` helper in flow 2 calls `wake()` twice with a near-tautological second assertion (ledger Task 9 `redundant-assertion`).
