# Inherited-name fixture currentness

The final unfiltered run exposed two test-fixture failures after the approved named-startup currentness gate: `inherited_name_is_refused_other_names_bind_and_restart_frees_it` and `no_hint_or_no_server_pid_skips_the_check` returned `Unsupported: refusing fixture host` when legitimate named resolution read its host. The fixture had used a deliberately refusing pane `EnvHost` as the named host and inserted boot/kind rows by hand. This was a fixture mismatch, not a production authority regression.

Commit `4b149c8eb716af6867f3210f25a164b0020db4ef` changes only `tests/daemon/inherited_name.rs`. It attaches the real built-in `NamedHost` through `HostSessions`, allowing ordinary coherent named observation/publication, and separately attaches the hinted Herdr pane's `EnvHost` for its server-environment read. Fixture Drop stops all owned sessions before directory cleanup. No production code, admission rule, golden, test body or assertion changes.

All four original test bodies and assertions are byte-identical, verified programmatically against the parent. They retain same-name M6 refusal and no allocation, the exact M6 golden, successful other-name allocation, binding after server restart without the variable, no-hint/no-server-PID behavior, hinted-core attachment/refusal, and first core attachment by LocateCaller.

## Evidence

- RED: the existing unfiltered run at `4dbdcd5f1` failed the two named tests with the refusing-host errors above. No second unfiltered run was performed for this fix.
- GREEN: `heavy tb env HT_LEAK_RUN_ID=inherited-fixture-20261115 cargo nextest run --locked --all-features --lib --status-level fail --final-status-level fail -E 'test(inherited_name_tests::) | test(named_startup_tests::)'`: 7/7 passed,7529 skipped; build1m13s, runtime1.494s. This executes all four inherited-name cases and all three named-startup regressions.
- `heavy tb rustfmt --edition 2024 --check tests/daemon/inherited_name.rs`: exit0.
- `scripts/check-harness-neutral`: passed,0 violations,90 allowlisted occurrences,55 entries.
- `scripts/check-wire-compat`: passed,0 violations,0 stale,456 fields,24 variants,274 baseline entries. Scoped diff whitespace check passed.
- `heavy tb scripts/check-no-leaked-processes --run-id inherited-fixture-20261115`: exit0; `no leaked test temp files` and `no leaked test processes` (0.31s wrapper call).

All builds/tests use the required wrappers, the existing target and default concurrency caps of four. The coordinator assigned one shared final clippy/default-feature/fmt gate to qualification after both coordinated test-only corrections; those final results are recorded by qualification, not claimed here. Other agents' files were neither staged nor committed by this fix.

The focused sample above is not a full-suite budget qualification. The triggering integrated unfiltered sample was878.113s; the coordinator's committed main baseline at the same `-j4` was951s. These figures do not establish an introduced regression or claim the five-minute budget is met. No stress or repeat run was added.
