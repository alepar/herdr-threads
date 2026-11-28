# Final review: cluster 12 (scripts-tests), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify <sha>^{commit}`). Caller checkout observed at HEAD c3e9ab0be, clean, before and after the test run.

## Scope

The cluster is one file, `scripts/tests/test_release_qualify.py` (slice 000065): three lines removed, namely the comment and the `patch.object(module, 'operator_programmatic_wake', lambda: False)` stub in `run_driver`.

## Analysis

- Spec link: design section "Removing the node setting" says `scripts/release-qualify` selects queue proof only with an explicit `--queue-proof` and that the node-settings probe goes. At head, `scripts/release-qualify` no longer defines `operator_programmatic_wake` or `_doctor_report`. `wake()` appends `--queue-proof` only when `queue_proof` is set, and `main()` still refuses the flag outside `QUEUE_PLATFORMS`. The stub would now raise `AttributeError` (patch.object on a missing attribute), so removing it is required, and the test edit matches the script.
- Without the stub, the `self_test=False` paths (`test_live_wake_uses_newest_after_self_test`, `test_failed_candidate_blocks_live_wake_and_binary_installers`) would reach the real candidate if any probe were left in the script. No `Popen` or doctor call remains. Every command goes through the patched `subprocess.run`, so the tests stay hermetic.
- Ran `python3 -m unittest discover -s scripts/tests -p 'test_release_qualify.py' -v` (the CI spelling, `.github/workflows/ci.yml:407`) at head with `PYTHONDONTWRITEBYTECODE=1`: 7 tests, OK, in 0.05 s. The caller tree stayed clean afterwards.
- Docs agree with the script at head: AGENTS.md line 147 and RELEASE.md lines 38-51 both describe explicit-flag-only selection. That belongs to another cluster and is noted here only as a consistency check.
- Baseline row B1 ("replace node level setting with seat level"): this cluster delivers only the test-side follow-up for the release tooling. Delivering the seat-level replacement in the daemon, CLI and wake driver is outside this cluster and is for the seat and other cluster reviewers to trace. Rows B2-B10 are not touched by this file.

## Findings

- No defect found in the cluster's diff. The removal is correct and required, and the suite passes at head.

## Concerns

- `scripts/release-qualify` at head still has `import signal`, but `signal.` is no longer used (its only use was the removed `os.killpg(..., signal.SIGKILL)`). It is a harmless dead import, and that file is another cluster's.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No test in `scripts/tests/test_release_qualify.py` checks the explicit-flag contract the design now requires: `qualify(evidence, False, queue_proof=True)` should pass `--queue-proof` to `scripts/wake-e2e-herdr`, and the default `queue_proof=False` should not. A regression that drops or always adds the flag would pass all 7 tests. This gap was there at base and is now the whole selection rule.
- No test checks that `release-qualify` reads no operator state (no `doctor --json`, no node `settings.json`) on the live wake path. Today that holds only because the script has no such code. Nothing asserts it.
- The `main()` refusal of `--queue-proof` outside Linux and macOS has no unit test.

## Deferred OK
- Remove the now-unused `import signal` from `scripts/release-qualify` (cosmetic, another cluster's file).
