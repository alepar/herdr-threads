super-roast verdict: Should-fix (4 confirmed, fixed in round-1 follow-up) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling; the change is test-only (tests/setup_cli.rs), so blast radius is suite reliability and the RED power of the task51 FIFO refusal tests, not users.
inputs: ci/setup-neighbors@d6ca64f93 vs main@870a53455 (merge base 94414e6d3)
coverage: scouts correctness, premortem, concurrency-async, simplicity-design, test-causality-lifecycle, test-suite-cost (6/6 returned; triage skipped, test lanes chosen by hand for a test-only diff) · 33 raw → 9 deduped · judge panels not run: each finding was verified inline by the orchestrator against code and reproductions · remainder-capped: 0
independence: partial (subagent scouts, inline verification) · rung: manual fan-out

## Confirmed findings

- [Should-fix] tests/setup_cli.rs `task51_fifo_has_reader` — a probe hit opens the FIFO's write end, which completes a consumer's blocking open(2); the fd was dropped at once, so the released consumer read EOF and ran on until `child.stop()`, and a regression could fail as "mutated on X" instead of "blocked" [lanes: correctness, premortem, concurrency-async, test-causality-lifecycle, simplicity-design]
  verdict: confirmed (test-causality scout reproduced the release with a FIFO demo; Linux fifo_open semantics)
  fix: the probe returns the write end, and it is held until the child is stopped, so a released consumer blocks on its read instead of acting on EOF.
- [Should-fix] tests/setup_cli.rs wait loop — with no FIFO (neighbors, lock probe) the wait had no bound: 300 s nextest kill without a label, forever under plain `cargo test` [lanes: all six]
  verdict: confirmed (code read)
  fix: a labelled `HANG_GUARD` of 60 s (four times the product's 15 s probe limit) on every call; proved by stopping the helper-regular child 65 s: `consumer helper-regular still running after 60s`.
- [Should-fix] tests/setup_cli.rs wait loop — in a FIFO case, a consumer blocked elsewhere (e.g. a blocking write-side open of the FIFO, which adds no reader) was never judged [lanes: correctness, premortem, test-causality-lifecycle, test-suite-cost]
  verdict: confirmed (scout demo: a blocked writer gives ENXIO)
  fix: covered by the same labelled hang guard.
- [Nit] tests/setup_cli.rs probe — ENOENT/ELOOP panicked and a regular file at the path counted as a reader, misattributing a consumer that removed or replaced the FIFO [lanes: correctness, premortem, concurrency-async]
  verdict: confirmed (code read)
  fix: O_NOFOLLOW, ENOENT/ELOOP and non-FIFO results are "no reader"; the snapshot assertions name the mutation.

## Accepted residual (documented, not fixed)

- tests/setup_cli.rs probe — a correct O_NONBLOCK consumer holds the FIFO between open and its fstat refusal; a probe landing in that window reads as blocked [lanes: correctness, premortem, concurrency-async, test-causality-lifecycle, simplicity-design]. No portable non-destructive way tells a blocked opener from one in that window (a write-open releases the opener; /proc is Linux-only). The probe only runs once a child has been alive 3 s, then every 3 s, and the window is a few syscalls of CPU, so a false hit needs a consumer preempted inside it at the probe instant. The probe's doc comment states the window.

## Rejected / out of scope

- Rename `task51_bounded`, split the return type — with the hang guard the function is bounded again; `None` only for FIFO calls is documented. Nit, not taken.
- Lock probe re-executes the whole combined binary ~42 times (test-suite-cost) — true cost, but pre-existing design and out of this bead's scope; follow-up candidate.
- The product's own 15 s helper `PROBE_LIMIT` is also load-dependent (correctness, speculative) — a product bound, would fail as a different assertion; not observed; out of scope.
- macOS FIFO reader counting unverified (premortem) — XNU's fifo_open counts a reader before sleeping as far as known; if not, the hang guard still fails the case, labelled, at 60 s.
