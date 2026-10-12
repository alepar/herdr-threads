# Leak scope task review

Spec: PASS. Quality: PASS. Findings: none.

Reviewed the supplied immutable helper snapshot at
`5a7bc0590c7dda3b86576c53c10be90a01d7bead`, the narrow two-file change described
against `026ad3ab444779216748cc5733e2241e2834ebaf`, `constraints.md`, and
`leak-scope-report.md`. This was a source-only independent review; no Git commands,
test execution, process collection, signals, Cargo, formatting, lint, Herdr,
tracker, or source mutations were performed.

The checker satisfies the requested isolation contract:

- `scripts/check-no-leaked-processes:86` preserves explicit PID exclusions first.
  Lines 88–89 reject every candidate lacking exact requested `HT_LEAK_RUN_ID`
  equality before owner checks or any leak rule. Foreign, missing, and prefix-only
  tags therefore cannot fall through to global detection rules.
- With `run_id=None`, the new guard does nothing and the existing dead-owner,
  artifact, and orphan-temp rule chain retains its behavior. Matching run tags
  still classify as `run-id`.
- `scan` combines the checker and its ancestor chain with explicit exclusions
  before classification (`scripts/check-no-leaked-processes:182`). Every grace
  rescan passes the same run scope. The `--kill` PID list comes only from the final
  classified leak list (lines 204–224), so the repair also scopes that path.
- The documentation at lines 4–6 accurately states exact-tag scope, required
  propagation into every child environment, the accepted exclusion of untagged
  children, and unchanged global behavior without `--run-id`.

The pure regression tests cover the actual failure mechanism:

- Six fixtures cover both owner keys, executable and cwd artifact evidence, and
  orphan server HOME and daemon state-dir evidence. Each is rejected under missing,
  foreign, and `r10` tags when requesting `r1`: 18 meaningful regression assertions
  (`scripts/tests/test_check_no_leaked_processes.py:38` and line 58).
- Matching tags remain positive, and all six global rule outcomes remain positive
  without a requested run ID for missing, foreign, and matching tags (lines 64–77).
- The ancestor helper and classifier are composed with tagged self, two ancestors,
  an explicit excluded PID, and one retained positive candidate (lines 135–147).
- The test module imports the checker under `leakcheck`, so its guarded `main` is
  not invoked. Test calls use in-memory `Proc` values, the pure classifier, and the
  ancestor helper; they do not collect or kill processes. The recorded RED with
  18 failures and two GREEN runs of 10 tests is consistent with these assertions;
  execution evidence is the author's report, not an independent rerun.

The two-line production guard is sufficient and keeps this repair narrow. No
blocking or nonblocking findings. Parent application and the actual scoped checks
of the completed parent run IDs remain outstanding; this review makes no claim
that those runs are leak-free.

Status: COMPLETEQUIESCENT. No persistent processes were started by this review.
