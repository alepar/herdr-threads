# Corrected frozen-tree full-suite qualification

Exact source `7090d18af6996f7fdaed96717fef1f16c77883ba`, clean before/after the
run, tree `2de5287d691c34d7170969d006bc28457ae36f00`. Main remains
`4dabf06ef90c8c9e8c459b524b509af3a2e98b31`; its actual merge-tree equals the
source tree. Migration0018 is still next free on main at completion.

Astra273 accepted the final hint-authority delta. Under main28/Astra269, exactly
one corrected unfiltered all-target/all-feature suite ran with the same cap4,
profileci, warm target and unchanged nextest groups. Command: `heavy tb nice -n 19
cargo nextest run --locked --all-targets --all-features --profile ci -j 4`;
CARGO_BUILD_JOBS/NEXTEST_TEST_THREADS/RUST_TEST_THREADS all4. Source was fixed
throughout; no live run or broader review was added.

Result: **9779 passed, 0 failed, 64 profile skips**, exit0. Execution1015.238s,
compile0.03s, command wall1017.390s. One slow release failpoint proof135.107s.
JUnit has9779executed/0failures/0errors; the64 profile skips appear in the log,
not as executed JUnit cases. Nextest2580a3d9-ad94-4104-978a-f2ca673ca69d.
Fresh HT_LEAK_RUN_ID93b01690-2f88-4c66-839a-1671542e08c0; the first automatic
scoped cleanup returned0: no leaked test temp files, no leaked test processes.
No duplicate cleanup was run before preserving this result.

`qualification-final-corrected.json` pins source, configuration, all changed
non-document files and raw artifacts. `final-suite-raw/` preserves deterministic
gzip copies of the original log, JUnit, 504 host samples/time record, first cleanup
and runner. Global host maxima cargo2/rustc3/nextest1 show concurrent host activity;
no timing causation or five-minute budget pass is inferred. Performance remains
the separate nonblocking lane by relayed user main25/26 disposition.

The prior6ca failed full suite remains immutable in qualification-final-hint-failure.*.
This corrected PASS does not rewrite it or extend historical live scope. Existing
P1 ht-d2jq.8 remains open, with the exact opt-in pre-arm Prepared-fence limit in
CHANGELOG and integration handoff. Main owns merge/release; implementation/build
worktree cleanup follows confirmed landing. The successor commit preserves only
documentation/evidence; runtime qualification remains attributed to7090.
