# First expanded 657-test run: preserved failed evidence

**Status: FAILED; investigation remains required.** This preserves the initial expanded run before any retry. The copied source reviews retain their original pending runtime cutoffs; the actual expanded-run failure governs readiness. Passing source gates, model-free mod checks and mandatory recovery controls do not make this run pass.

Recorded immutable candidate: HEAD `9a573c3f2a5420ff0c3fee07a3e836d189e88a83`, tree `5158b885c8988b1c3ebf5b96047751a6e4949212`. Runner status `run_failed`, exit 1. Started `2026-10-11T23:38:42.331724+00:00`; finished `2026-10-11T23:40:39.841404+00:00`.

- Expanded selection: 657 cases, split into 651 focused and six independently exact mandatory cases. Focused evidence is complete: no missing, extra or duplicate cases. Nextest reports 581 passed, 70 failed, 5331 skipped in 47.188 seconds; focused command exit 100 and measured command duration 48.11425125005189 seconds.
- Six exact mandatory recovery calls each exit 0, with six passed and no missing, extra, duplicate, ignored or erroneous results.
- Focused, mandatory and final scoped leak checks all exit 0 for `23ccc45f-5693-49d5-8f6e-dfa16e590868`. Final execution provenance records the expected HEAD/tree, empty tracked status and no unmerged entries. This task did not independently invoke Git or scan processes.
- Final composed source gates for tree `5158b885c8988b1c3ebf5b96047751a6e4949212`: fmt, neutral-core, all-feature clippy and default-feature checks each exit 0; aggregate exit 0.
- Final mod checks: exit 0, scoped leak exit 0 (`a5a62191-0830-4656-b34b-a946da1eddc0`), 1.6191561250016093 seconds. Log reports 121 tests passed, zero failed. Scope is model-free registered-mod validation/tests with isolated HOME/configs; no native model consumption or receipt claim.
- Parent-reported coordination context: usage-limit triage agents failed before investigation; the user has now resumed work. This is coordination history, not a failure diagnosis. No retry or investigation result is asserted here.
- Optional `watch-activation.jsonl` was absent from the run directory. The large `nextest-list.json` was not copied. Original inventory, selection seal and runner basenames are preserved; their hashes match the execution record.

Every listed copy is byte-equal to its source and verified by SHA-256. Originals are preserved. Destinations below are relative to the parent RUN directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-final-composed-657/execution.json` | `resume-v090-657-first-run-evidence/execution.json` | 595167 | `a1cdd5300ea5d61fa3eca8350b6d341c1417064aa35db5f6f9a297279a1058a9` |
| `/private/tmp/ht-C-v090-final-composed-657/nextest-run.log` | `resume-v090-657-first-run-evidence/nextest-run.log` | 153910 | `3c1a3bc0e31e71cab6d4dfc3e59f88f1373157d08652a74090f586c3b02a64bb` |
| `/private/tmp/ht-C-v090-final-composed-657/nextest-list.stderr.log` | `resume-v090-657-first-run-evidence/nextest-list.stderr.log` | 220 | `08b4aa1b5ed37be289136a5e4d7ea40d212b4654185aad9353d05beb957fdf50` |
| `/private/tmp/ht-C-v090-final-composed-657/focused-scoped-leaks.log` | `resume-v090-657-first-run-evidence/focused-scoped-leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-final-composed-657/mandatory-scoped-leaks.log` | `resume-v090-657-first-run-evidence/mandatory-scoped-leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-final-composed-657/final-scoped-leaks.log` | `resume-v090-657-first-run-evidence/final-scoped-leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-final-composed-657/mandatory-recovery-01.log` | `resume-v090-657-first-run-evidence/mandatory-recovery-01.log` | 495 | `2f56ee1d27b7ad379284b5c7e935075c9a16bd67a0e2f1a53f10e93c3f1034c7` |
| `/private/tmp/ht-C-v090-final-composed-657/mandatory-recovery-02.log` | `resume-v090-657-first-run-evidence/mandatory-recovery-02.log` | 487 | `a3419ce3f8c3ab8653bbee978b900e4e3dc0bc211d50b7b505cdf59613f71eef` |
| `/private/tmp/ht-C-v090-final-composed-657/mandatory-recovery-03.log` | `resume-v090-657-first-run-evidence/mandatory-recovery-03.log` | 489 | `4f805388709da7531a59602c58e9b551c48a0b733a327b0da086cc8a312d08e2` |
| `/private/tmp/ht-C-v090-final-composed-657/mandatory-recovery-04.log` | `resume-v090-657-first-run-evidence/mandatory-recovery-04.log` | 481 | `64c268b286cfa6d93c32647f1fc8023fa42ffc38d6f8d609aba48c0c226435c8` |
| `/private/tmp/ht-C-v090-final-composed-657/mandatory-recovery-05.log` | `resume-v090-657-first-run-evidence/mandatory-recovery-05.log` | 515 | `6f6e3b90cfadfedcfbcc48acfeadb2fe9d0c38799594b53c04e246c3dd70b937` |
| `/private/tmp/ht-C-v090-final-composed-657/mandatory-recovery-06.log` | `resume-v090-657-first-run-evidence/mandatory-recovery-06.log` | 507 | `0ad8665996afff1b87d7fa6387eda8d1a23a600454e7e8bad942c0da01e920d5` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-9a573c3f-final-inventory.json` | `resume-v090-657-first-run-evidence/resume-v090-9a573c3f-final-inventory.json` | 985010 | `c433673ebf52ce5d64b4352f8b9f6d512dcad60edb3a66d5034a7b22f699f3a8` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-9a573c3f-final-selection-seal.json` | `resume-v090-657-first-run-evidence/resume-v090-9a573c3f-final-selection-seal.json` | 1272372 | `f1fd34d5d6063aeedc3f06989caf5905afe9a863989c24070d51db3571a5186e` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-run.py` | `resume-v090-657-first-run-evidence/resume-v090-run.py` | 42412 | `dce287cba7abcd89e0a0fce0ad63af50ac2bf1e333deff5391f7c4d09cbc267f` |
| `/private/tmp/ht-C-v090-final-composed-source-gates/execution.json` | `v090-9a573c3f-final-composed-source-gates-execution.json` | 1622 | `6c8fd4cf9bdf9b3e3f86a8b97b309c92aa775f6e395e1407c2d0a8065daef411` |
| `/private/tmp/ht-C-v090-final-mod-tests/execution.json` | `v090-9a573c3f-final-mod-tests-execution.json` | 299 | `cabe4c6bf72b6a7cd3f2b6806578457be86a117b8ded6d387db44396b9cf5e69` |
| `/private/tmp/ht-C-v090-final-mod-tests/run.log` | `v090-9a573c3f-final-mod-tests-run.log` | 12246 | `b9adefc27d08bec6c02e0b89f5c8cc6ff30407878eb5537253ec153db9711d0c` |
| `/private/tmp/ht-C-v090-final-mod-tests/leaks.log` | `v090-9a573c3f-final-mod-tests-leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-final-composed-spec-review.md` | `resume-final-composed-spec-review.md` | 28056 | `507972f28e040c791d7e3a103dbdc7405b53c414bb331c9d53b87dcd706f2f05` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-final-composed-quality-review.md` | `resume-final-composed-quality-review.md` | 31236 | `9d9d5d77b58197eb0bb97b65ace11fdd589f962958dfd3c6bdde6b8384209312` |

Only new evidence paths were written. Existing documents, source files, index and historical evidence were untouched. No Git mutation, Cargo, project runtime, Herdr command or subordinate agent was used. No process was started by this task.
