# Inherited-name fixture currentness

The final unfiltered run exposed two test-fixture failures after the approved named-startup currentness gate: `inherited_name_is_refused_other_names_bind_and_restart_frees_it` and `no_hint_or_no_server_pid_skips_the_check` returned `Unsupported: refusing fixture host` when legitimate named resolution read its host. The fixture had used a deliberately refusing pane `EnvHost` as the named host and inserted boot/kind rows by hand. This was a fixture mismatch, not a production authority regression.

Initial commit `4b149c8eb716af6867f3210f25a164b0020db4ef` changed only `tests/daemon/inherited_name.rs`, attaching a `NamedHost` and a separate hinted pane `EnvHost`. Review found that it incorrectly constructed the named port with host ID 0, so its complete enumeration read an empty store scope. Its seven passing tests were insufficient evidence of publication fidelity and did not make that fixture repair acceptable. Fixture Drop still stops all owned sessions before directory cleanup.

All four original test bodies and assertions are byte-identical, verified programmatically against the parent. They retain same-name M6 refusal and no allocation, the exact M6 golden, successful other-name allocation, binding after server restart without the variable, no-hint/no-server-PID behavior, hinted-core attachment/refusal, and first core attachment by LocateCaller.

## Review correction

The corrected first fixture calls `sessions.reserve_host_id(&locator, HostKind::Named)` before constructing `NamedHost`, passing the returned ID exactly as production `src/app.rs` does. A new assertion in the shared successful-resolution helper requires real complete enumeration to contain the resolved name. Current-target lookup alone cannot satisfy this assertion. The assertion failed with ID 0 before the ID correction, so it directly detects the review defect.

All original test bodies/assertions and M6 bytes remain unchanged; only the shared fixture/helper and this new guard change. No production or authority code changes. The pre-existing separate `with_sessions` helper for HC-only refusal/LocateCaller cases is unchanged: those cases allocate no name and do not need nonempty named enumeration. This scope was explicitly reported to the coordinator.

## Evidence

- RED: the existing unfiltered run at `4dbdcd5f1` failed the two named tests with the refusing-host errors above. No second unfiltered run was performed for this fix.
- Initial, insufficient GREEN before the host-ID review finding: `heavy tb env HT_LEAK_RUN_ID=inherited-fixture-20261115 cargo nextest run --locked --all-features --lib --status-level fail --final-status-level fail -E 'test(inherited_name_tests::) | test(named_startup_tests::)'`: 7/7 passed,7529 skipped; build1m13s, runtime1.494s. This executes all four inherited-name cases and all three named-startup regressions.
- `heavy tb rustfmt --edition 2024 --check tests/daemon/inherited_name.rs`: exit0.
- `scripts/check-harness-neutral`: passed,0 violations,90 allowlisted occurrences,55 entries.
- `scripts/check-wire-compat`: passed,0 violations,0 stale,456 fields,24 variants,274 baseline entries. Scoped diff whitespace check passed.
- `heavy tb scripts/check-no-leaked-processes --run-id inherited-fixture-20261115`: exit0; `no leaked test temp files` and `no leaked test processes` (0.31s wrapper call).

All builds/tests use the required wrappers, the existing target and default concurrency caps of four. The coordinator assigned one shared final clippy/default-feature/fmt gate to qualification after both coordinated test-only corrections; those final results are recorded by qualification, not claimed here. Other agents' files were neither staged nor committed by this fix.

The focused sample above is not a full-suite budget qualification. The triggering integrated unfiltered sample was 878.113 s; the coordinator's committed main baseline at the same `-j4` was 951 s. These figures do not establish an introduced regression or claim the five-minute budget is met. No stress or repeat run was added.

Review-regression RED: `heavy tb env HT_LEAK_RUN_ID=inherited-fixture-r1-20261115 cargo nextest run --locked --all-features --lib --status-level fail --final-status-level fail -E 'test(inherited_name_tests::)'`: 2 passed, 2 failed, 7,532 skipped; build 57.53 s, runtime 1.016 s. Both successful-allocation tests failed at `complete named enumeration must include the resolved name` before the ID correction. This is separate from the original refusing-host RED.

Review-correction GREEN: the same focused seven-test command above with `HT_LEAK_RUN_ID=inherited-fixture-r1-20261115` passed 7/7, 7,529 skipped; build 46.10 s, runtime 1.422 s. It includes the new complete-enumeration guard as well as all four unchanged test bodies and all three named-startup cases. Scoped rustfmt, neutral/wire and whitespace guards passed. The scoped leak checker returned `no leaked test temp files` and `no leaked test processes` (0.28 s wrapper call) after both the intentional RED and final GREEN. No production change or second full suite was run. This correction awaits independent scoped re-review; earlier approval is not assumed.
