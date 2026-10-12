# Composed 3511fb84: green source gates and interrupted 51-test baseline

**C remains NOT READY; source code-roast counters remain 0.** This checkpoint preserves the completed green source gates and full failed/interrupted baseline, with the owned-stop record and raw cancellation log. Earlier actual failed histories remain unchanged. No product success, repaired GREEN or complete qualification is asserted.

Candidate HEAD `3511fb84b6eb775e28a4bd4832468cb4bc10317c`, tree `578eb115ff596b94365f987c3f2b893d45c1e239`, scoped leak run ID `bdd0e0c7-6938-469e-b383-19db93367101`. Exactly 51 cases expected and selected. Listing/build exit 0; runtime exit 100; scoped leak check exit 0.

## Green source gates

Staged tree `578eb115ff596b94365f987c3f2b893d45c1e239`, index SHA-256 `7eeb9c73a5548c026fe99533c59eb4e76420e1984fed0624f0378f138f3fa9c3`. Every gate and aggregate exit 0. The execution file has no HEAD field; parent associates these gates with the same 3511fb84 candidate.

- fmt: 4.352287124958821 seconds, exit 0.
- neutral: 1.2997932080179453 seconds, exit 0.
- clippy: 37.08582620800007 seconds, exit 0.
- default: 26.239569625002332 seconds, exit 0.

All embedded command-log SHA-256 values were matched before copying.

## Baseline result and speed evidence

- Raw nextest summary: 51 tests run, 33 passed, 18 failed, 4835 skipped, 142.083 seconds. The 18 comprise **12 completed FAIL statuses and six owned SIGINT interruptions**. The six interruptions are not six additional product assertion failures; their incomplete runtime outcome remains separate.
- Wrapper runtime duration 142.30885566596407 seconds. Listing/build duration 124.43153370800428 seconds; compiler stderr reports 2m 02s. This is a multi-file composed test build/listing measurement, not a controlled one-file incremental production benchmark. Actual durations are preserved; no speed budget was raised.
- Thirty-three passes include Legacy/mode authority, bounded real-store reader work, pending-hint concurrency/rollback and the shared captured-body ACK control (1.537 seconds). These are scoped outcomes, not evidence that every pending defect is repaired.

## Causal failures versus fixture failures (parent diagnosis)

- Four reached causal REDs: two active final-send ownership gaps across replacement modes, where old frame/health reaches replacement; independent ordinary-publication scanner observes 0 instead of 1; deterministic virtual Notice same-token replay loses the Notice.
- Seven store-fixture failures: four duplicate execution UUID setups, two sparse-only receipt reads, and one foreign-instance foreign-key fixture. One prospective SQL-work control has a schema-fixture failure. These eight setup/oracle failures must be corrected and rerun; they do not uniquely prove additional production defects.
- Parent diagnosed the six hung CLI tests as a helper ownership cycle: a cancellation clone retained the endpoint open. This diagnosis is parent-supplied; this evidence task did not execute or alter a helper.

## Owned stop and interruptions

The copied `owned-stop.json` records only wrapper PID 65539 and its owned nextest child PID 65698, SIGINT, after six scoped CLI controls exceeded 60 seconds. Parent reports no shared-process action. Scoped leak verification completed with exit 0. This task performed no signal, process scan or runtime action.

Six raw SIGINT results:

- `cli::watch::tests::capable_watch_streams_ordered_v2_then_explicitly_confirms_captured_controls`
- `cli::watch::tests::q3_missing_watch_mode_capability_keeps_old_wire_and_consumer`
- `cli::watch::tests::q3_all_watch_mode_capabilities_send_true_and_use_the_accepted_lease`
- `cli::watch::tests::q3_missing_binding_cursors_capability_keeps_old_wire_and_consumer`
- `cli::watch::tests::presentation_watch_requires_both_capability_and_valid_accepted_lease`
- `cli::watch::tests::q3_missing_generic_presentation_capability_keeps_old_wire_and_consumer`

## Continuation

Source was unfrozen after the baseline; fix authors are active. The active final fence, ordinary scanner, Notice replay, helper-cycle and fixture corrections remain pending new GREEN. Root code-roast counter remains 0; C is not ready. Parent retains all Cargo/runtime/source ownership.

## Byte-exact copies

Every destination was verified byte-equal to its preserved source and SHA-256 equal. Full supplied packages are copied, including raw inventory, execution/binary provenance, `owned-stop.json` and cancelled run log. Destinations below are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q3-composed-lint-corrected-gates/clippy.log` | `ht-C-v090-q3-composed-lint-corrected-gates/clippy.log` | 219 | `83fbbb0c7b2ba3c117787c37e537baf908a7622c3d1208d61f28d321d8430697` |
| `/private/tmp/ht-C-v090-q3-composed-lint-corrected-gates/default.log` | `ht-C-v090-q3-composed-lint-corrected-gates/default.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-composed-lint-corrected-gates/execution.json` | `ht-C-v090-q3-composed-lint-corrected-gates/execution.json` | 1641 | `91f39d77fab90389ea9d974a4b6cd35d128b3ac82e17f993837515aad1f76406` |
| `/private/tmp/ht-C-v090-q3-composed-lint-corrected-gates/fmt.log` | `ht-C-v090-q3-composed-lint-corrected-gates/fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-composed-lint-corrected-gates/neutral.log` | `ht-C-v090-q3-composed-lint-corrected-gates/neutral.log` | 130 | `7561605afc0af5f27527d99451825bc26c55fa2f7083bb392ad4244e406b0536` |
| `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/execution.json` | `ht-C-v090-q3-composed-3511fb84-baseline51/execution.json` | 18537 | `d7a67c587b02ee3e55051b843adbb2ea40b110cca15eb9936426b42de2ab6546` |
| `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/leaks.log` | `ht-C-v090-q3-composed-3511fb84-baseline51/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/list.json` | `ht-C-v090-q3-composed-3511fb84-baseline51/list.json` | 865092 | `d1e1cf527deec8bc8107b412fb34084cf5efa4811cac487fddc82ed5b31a9570` |
| `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/list.stderr` | `ht-C-v090-q3-composed-3511fb84-baseline51/list.stderr` | 220 | `2112b5c5a5d8c00b6dfa2b063bbe9f733a843355c21f82a951462ab49295b939` |
| `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/owned-stop.json` | `ht-C-v090-q3-composed-3511fb84-baseline51/owned-stop.json` | 5582 | `794042ac60191ba4dcfca7f08f570b7cd2f0a5a64c3ae698d6d3555d7fe42dfb` |
| `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/run.log` | `ht-C-v090-q3-composed-3511fb84-baseline51/run.log` | 22649 | `db02cb30462b240365140ffca9be319311a589cbbb52b1003b81f37b8ca98c9c` |

Only new evidence/documentation files were written. Historical reports and sources remain unchanged. No source/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. QUIESCENT.
