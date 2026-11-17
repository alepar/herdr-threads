# v3.3.0 final shared checks and development build

The shared checks and development build **passed**, with clean same-run builder and host process/temp scans. Source: `9f69b8d02b1244a7e21be8df9d368b380f887764`, on required base `4f2a32d3a1fb9bd5991f6d1cb13fb345b99f1da5`. Completion HEAD `64bcfabee6a45f0e86f4fdcd07c994e1d37ed759` differs only by the four audited documentation paths. [Exact identities, commands, timings and log hashes](v330-final-gates.json) accompany this report.

| Check | Result | Wall seconds |
| --- | --- | ---: |
| Format check | PASS | 7.56 |
| All-target/all-feature clippy, warnings denied | PASS; 60-second budget missed | 100.21 |
| Default-feature check | PASS | 68.79 |
| Harness-neutral guard | PASS | 1.36 |
| Wire compatibility guard | PASS | 0.10 |
| Development binary build | PASS | 66.63 |
| Builder process/temp scan | PASS, exit 0 | — |
| Host process/temp scan | PASS, exit 0 | — |

The existing warm target and capped `heavy tb` runner were used with four workers and no overrides. Timings are separate command observations, not a paired regression benchmark. Clippy's functional pass does **not** meet its 60-second budget; the previously accepted full-suite timing disposition does not imply a clippy-budget waiver.

## Exact native candidate

- Binary version: `herdr-threads 3.3.0` (the baseline version, not a Pi release).
- Development binary SHA256: `a63ac7a7629f29c117d1968523d65f8253aaaa1a1bfbe5e66b29e0b81ac06ea4`.
- Stable same-inode artifact: `target/debug/deps/herdr_threads-ee429b642d86e164`; common build path: `target/debug/herdr-threads`.
- Cargo profile: **dev**, fingerprint profile `4923293383976553682`, features `["test-support"]`; exact fingerprint and source hashes are in the JSON.

Fresh scripted native qualification was authorized and active at publication; **no native result is claimed here**. The final full suite remains pending separate coordinator authorization after native. Its test-profile binary will be recorded independently; a shared path or source tree does not make it the same development artifact. No paid run was performed for these checks, and [historical d55/a8 live evidence](final-live.md) retains its original provider/ordinary-path scope.

The [v3.3.0 source audit](v330-rebase.md) records the reviewed `native_turn_id: None` adaptation, retained conflict-resolution groups, 34-path inventory and inherited schema 18. Pi declares no programmatic wake control; upstream shared canonical queue fences still execute before native-only selection. Queue disabled/no Pi control does not mean upstream dispatch has no effect.

Private complete logs remain in `.superpowers/sdd/implementation-plan/final-gates/v330-final-gates/`; their hashes were checked for this publication. Run ID: `efdec623-9303-48f6-9e45-d24bb6d7a761`. This document archives completed shared checks, not overall merge readiness. The coordinator owns final native/full-suite disposition and the subsequent main handoff; release is outside this task.
