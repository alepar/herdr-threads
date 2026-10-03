# Harness canary tier 0, 2026-10 (listed and newest versions)

Two runs of `scripts/harness-canary.sh --model-tier off` (claude, then codex), merged in canary-report.json. Per-harness summaries follow.


canary commit `b52eb7dbc7c1b6adbcaffda54de4c7fa8603041c`, herdr-threads 0.1.0, runner darwin/arm64

## claude (@anthropic-ai/claude-code) — verdict: all_pass

verified_max 2.1.286; candidates: 2.1.283, 2.1.284, 2.1.285, 2.1.286, 2.1.287

| version | role | result | attempts | t0.version | t0.setup | t0.config-load | t0.hook-fires | t0.payload-parse | t0.schema | t0.admission | t0.launch-flags | t0.npm-shim-admission | t0.launch-tables | t0.unsetup | t1.session-start | t1.pre-tool-use | t1.context-delivery | t1.payload-parse | t0.isolation |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2.1.287 | newest | pass | 1 | pass | pass | skip | pass | pass | skip | pass | pass | skip | skip | pass | skip | skip | skip | skip | pass |
| 2.1.286 | search | pass | 1 | pass | pass | skip | pass | pass | skip | pass | pass | skip | skip | pass | skip | skip | skip | skip | pass |
| 2.1.285 | search | pass | 1 | pass | pass | skip | pass | pass | skip | pass | pass | skip | skip | pass | skip | skip | skip | skip | pass |
| 2.1.284 | search | pass | 1 | pass | pass | skip | pass | pass | skip | pass | pass | skip | skip | pass | skip | skip | skip | skip | pass |
| 2.1.283 | search | pass | 1 | pass | pass | skip | pass | pass | skip | pass | pass | skip | skip | pass | skip | skip | skip | skip | pass |

Assumes a monotone break (versions never probed are not checked).

exit code: 0

## codex (@openai/codex) — verdict: all_pass

verified_max 0.158.0; candidates: 0.157.1, 0.158.0, 0.159.2, 0.159.3, 0.160.0

| version | role | result | attempts | t0.version | t0.setup | t0.config-load | t0.hook-fires | t0.payload-parse | t0.schema | t0.admission | t0.launch-flags | t0.npm-shim-admission | t0.launch-tables | t0.unsetup | t1.session-start | t1.pre-tool-use | t1.context-delivery | t1.payload-parse | t0.isolation |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0.160.0 | newest | pass | 1 | pass | pass | pass | pass | pass | pass | pass | pass | warn | pass | pass | skip | skip | skip | skip | pass |
| 0.159.3 | search | pass | 1 | pass | pass | pass | pass | pass | pass | pass | pass | warn | pass | pass | skip | skip | skip | skip | pass |
| 0.159.2 | search | pass | 1 | pass | pass | pass | pass | pass | pass | pass | pass | warn | pass | pass | skip | skip | skip | skip | pass |
| 0.158.0 | search | pass | 1 | pass | pass | pass | pass | pass | pass | pass | pass | pass | pass | pass | skip | skip | skip | skip | pass |
| 0.157.1 | search | pass | 1 | pass | pass | pass | pass | pass | pass | pass | pass | pass | pass | pass | skip | skip | skip | skip | pass |

Assumes a monotone break (versions never probed are not checked).

exit code: 0
