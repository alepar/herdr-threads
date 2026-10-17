# Verification

Base 82786da05; branch fix/v2-flakes; verified code commit 97755c280c089eda3b7e43249efabcf7c21ce31e. Independent review found no must-fix issues. No merge or push performed.

| Check | Result | Wall time |
|---|---|---|
| Focused nextest stress, ten relevant tests × five iterations | 50/50 passed; final iterations overlapped full-suite startup | 37.925s |
| Full `nice cargo nextest run --locked --all-targets --all-features` | 7,602 passed, 48 existing ignored/skipped, zero failures | 289.180s runner / 289.45s command |
| `scripts/check-no-leaked-processes --run-id` | No leaked test processes or temp files | 0.76s |
| `nice cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed | 58.65s |
| `nice cargo clippy --locked --all-targets -- -D warnings` | Passed | 54.72s |
| `nice scripts/check-default-features` | Passed | 40.93s |
| `cargo fmt -- --check` | Passed | — |
| `scripts/check-harness-neutral` | Zero violations | — |
| `git diff --check` | Passed | — |

Leak run ID: `1bda9736-c1df-478e-9097-874a32bf96ec`. The full run stayed inside the strict 300s budget. Its slowest test was the release failpoint proof (259.337s); neither test timeout nor suite budget was raised. Both clippy configurations stayed below one minute. Structured stage results: verification-results.json.

Controlled failing-first evidence is preserved in [reproductions](reproductions.md). The current V2 product watch defect is ht-x0b9. Historical ht-7ykh remains open because its older v1.2.0 path/schedule was not reconstructed. Broader static candidates remain tracked under ht-fmww.

Research checks: coverage structural validator passed (three rounds, explicit budget-exhausted stop); claim-support checker passed all thirteen factual ledger entries; report surface and package validators passed; external citation URL validator confirmed all nine external URLs accessible. Local execution evidence was checked by reading the retained record and local link validation, not counted as an independent external document. Retrieval saturation and dynamic confirmation of the broader audit candidates are not claimed.

All suite fixtures and helpers completed; run-scoped cleanup found no leaks. Compiler/lint helpers exited when their commands completed. No authenticated release wake gate was run for this non-release patch task.
