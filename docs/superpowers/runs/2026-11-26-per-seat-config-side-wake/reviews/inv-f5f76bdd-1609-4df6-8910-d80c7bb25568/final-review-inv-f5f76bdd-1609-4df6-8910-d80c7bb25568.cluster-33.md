# Final review, cluster 33 (tests-release)

Invocation inv-f5f76bdd-1609-4df6-8910-d80c7bb25568. Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Review source bound with dispatch-text.mjs bind-review-source (exit 0).

## Scope

The cluster holds one file, `tests/release/release_qualify_test.py` (slice 000182.diff), changed by d41408212 "Release tooling enables queue wake per seat (ht-z41e.5)". I opened `scripts/release-qualify` at head only to check what the test claims about it.

## Spec trace

Design §1 "Removing the node setting" says: "`scripts/release-qualify` selects queue proof only with an explicit `--queue-proof` (the node-settings probe goes)". This is the release-tooling part of baseline row B1 ("replace node level setting with seat level"). At head, `scripts/release-qualify` no longer has `_doctor_report`, `operator_programmatic_wake`, the `queue-mode` result row or the `mode_ok` block. `wake()` passes `--queue-proof` to `scripts/wake-e2e-herdr` only when `queue_proof` is set, and `main()` sets that only from the `--queue-proof` argument. AGENTS.md line 147 at head describes the same behaviour.

The test changes match:
- `QueueProofRouting.wake_command` drops the `install_enabled` mock now that the probe is gone. On Linux and macOS, the flagged and unflagged routing tests still assert the exact wake argv.
- The old `OperatorQueueMode` class tested the removed probe: JSON strictness, bounds, duplicate keys and doctor shape. `ExplicitOnlyQueueSelection` replaces it and asserts four things:
  - the probe symbols are absent;
  - an unflagged run on Linux and darwin produces exactly one wake command, without `--queue-proof` and without a `queue-mode` summary row, and opens no `settings.json` through `Path.open`, under a `Popen` tripwire;
  - the explicit flag selects queue proof without reading settings;
  - self-test skips the authenticated wake.
- A help-text test pins the new `--queue-proof` help wording.

B1's other half, the driver turning on `programmatic_wake` for each owned seat in `scripts/wake-e2e.py`, is in another cluster. Whether those two parts work together is for the seam reviewer.

## Verification run

I ran `python3 -m unittest tests/release/release_qualify_test.py -v` in the caller checkout. Before running, I confirmed HEAD is c3e9ab0b, the status is clean, and the test and script files are the same as at head. Result: 9 tests, all OK, in 0.29 s. No `/tmp/rq.*` scratch directory was left afterwards. All qualification subprocesses are mocked except the `--help` invocation, so nothing touched a real Herdr, daemon or config.

## Observations

- The settings guard patches only `Path.open`. A future probe that used builtin `open` or `os.open` would get past the guard. The behavioural assertion (no `--queue-proof` without the flag) would catch it only if the operator settings on the test machine enable queue wake. The roast r1 FYI (`2026-11-27-per-seat-config-side-wake-roast-pr-1.md:146`) raised this and it was rejected. The `hasattr` check together with the routing assertions is enough for the current code.
- `scripts/release-qualify` still has `import signal`, which only the removed probe used (`os.killpg(..., signal.SIGKILL)`). It is a dead import, harmless at runtime and outside this cluster's file.
- CI (`.github/workflows/ci.yml:407`) runs `scripts/tests/test_release_qualify.py`, not `tests/release/release_qualify_test.py`. That was already the case before this change, so the new explicit-only assertions run only when someone runs the file locally or a qualification run records it.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- tests/release/release_qualify_test.py is not run by CI (ci.yml:407 runs only scripts/tests/test_release_qualify.py), so the explicit-only selection checks are local-only; this predates the change.
- The settings-read guard covers only Path.open; a probe that read settings through builtin open or os.open would not trip it unless the host's operator settings enable programmatic_wake.

## Deferred OK
- Remove the now-unused `import signal` from scripts/release-qualify; only the deleted doctor probe used it.
- Consider adding tests/release/release_qualify_test.py to the CI release-tooling job beside scripts/tests/test_release_qualify.py.
