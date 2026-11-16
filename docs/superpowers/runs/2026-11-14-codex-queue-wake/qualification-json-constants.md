# Strict release-mode JSON constants

Astra250 bounded review accepted the storage/accounting and runtime/scheduler
material at056d63893. The sole remaining release-routing Minor was reproduced:
Python JSON accepts NaN, Infinity and -Infinity in otherwise malformed operator
settings or canonical doctor output. A valid queue-mode boolean could therefore
select a mode instead of rejecting the containing non-JSON document.

Both json.loads calls in scripts/release-qualify now use a rejecting parse_constant
callback, retaining the existing duplicate-key hook and sanitized invalid-JSON
errors. No schema validator, routing redesign, configuration write or new
model-visible text is added. The change is limited to the release parser and its
tests; core Rust/storage/runtime bytes remain exactly056d63893.

Two new regression tests exercise all three constants in doctor output and in
minimum_wake_delay_ms with programmatic_wake both false and true. Behavioral RED:
2 tests/9 subcase failures,0.041s. Final routing suite:16PASS0.218s. Scoped process
check clean; Python run has no temp ledger. Clippy0.09s, default-feature guard,
format, neutral, wire, docs, Python3.9 syntax and whitespace checks all pass.

The parent qualification-native-fallback-default.md/json remains immutable evidence
for056d63893. This supplement pins the two changed source hashes and new raw logs;
its overlay is the current source view. No new Rust test run, authenticated live
run or unfiltered full suite is claimed. Integration remains held pending Astra's
narrow acceptance/main review settlement and the required final frozen-tree suite
with fresh scoped process/temp evidence.
