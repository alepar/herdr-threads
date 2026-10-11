# Source/cache transfer audit

A restored B command completed in0.03s with zero selected tests and4204 filtered: it used D's stale lib binary despite correct Cargo metadata. INVALID, excluded from evidence. Source-preserving freshness invalidation then actual B crate compilation selected its nine unique modern tests (five pass/four genuine RED); later real recompilations underpin reported B GREEN results. Historical dirty-diff hashes were not recorded at earlier transfers and cannot be reconstructed honestly.

Coordinator independently inspected these retained raw D logs; each contains actual `Compiling herdr-threads` at the warm absolute package path and a uniquely introduced D regression with nonzero selection:

| Log under /private/tmp | Build | Selected result |
| --- | --- | --- |
| ht-task15-rust-red.log | 2m13s | 1 RED,4197 filtered |
| ht-task15-stable-red.log | 2m25s | 1 RED,4202 filtered |
| ht-task15-origin-scheduler-red2.log | 1m50s | 1 RED,4203 filtered |
| ht-task15-origin-scheduler-green.log | 21.19s | 1 GREEN,4203 filtered |

D common-name cached checks followed its actual recompilation anchor. No uncertain nonzero prior D result was identified. B's actual compilation anchors and unique selected public tests are recorded in task-13-report.md; zero-selection stale result never counts. This is source-identity evidence, not a claim that transfer build times meet the incremental budget.

Every future source/cache transfer now invalidates all relevant owned Rust/Cargo source roots, records exact HEAD/tree/dirty-diff hash and target provenance, and requires actual crate rebuild plus a unique expected test with selected count>0. Metadata alone is insufficient. Third D transfer invalidated486 owned files; exact current provenance is task-15-restored-source-provenance-3.json. Target was quiescent and only owned cache moved; no foreign cache/process modified.
