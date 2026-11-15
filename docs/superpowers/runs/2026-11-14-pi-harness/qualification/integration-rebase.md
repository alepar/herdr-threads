# Integration rebase accounting

The integration source preserves all **73 branch patches** across the rebase. The [exact anchors, commit mapping and 33-path inventory](integration-rebase-evidence.json) record this read-only audit.

| Boundary | Commit |
| --- | --- |
| Original base | `91ce0cdeb8e87dcc16c79f4a556fabffb7946967` |
| R2 reviewed source | `1dbdce20b59cfcc7f6c36c7a5085a0119b9dd874` |
| Original tip, including archived R2 report | `dc4c8173b182458fa15b45a71f0d8b2ba6ada7d0` |
| New main base | `e60ec5eb5e9746f82b399127838dc9515c7cc712` |
| Rebased equivalent tip | `bce7e6cc63ee42099a80e4d7845a9ef064fd7c94` |
| Audited integration tip | `bad263b15dbbca518ea0509d16f90f9d01c1c602` |

`git range-diff` reports `=` for every one of the 73 commits from original base to original tip against new base to rebased tip. The original reviewed source to original tip adds only the archived [R2 report](../roast-pr-2/report.md). The rebased tip to audited integration tip adds only the six-line Unreleased Pi entry in `CHANGELOG.md`.

Comparing the original tip with the integration tip yields exactly the 14 paths changed by upstream main, with the additional Pi changelog entry in the same changelog. Upstream changes comprise release/version metadata, Codex command guidance and its tests/goldens, and peer-wire fixtures. Among `src/`, the Pi integration and the two qualification scripts, the only change from the R2-reviewed source is upstream `src/harness/codex.rs` command guidance. Its changed text matches upstream; Pi production code, the neutral core behavior added by this branch, the extension and the driver/fixture are unchanged. Other upstream file contents match the new base, except the combined changelog and Codex file, which also retain branch changes.

Against the new base, **33 changed source paths outside `src/harness/pi/`** remain: 30 production paths and three test-only source paths. Their set exactly matches [the consolidated core table](core-changes.md); there are eight additional adapter source paths under `src/harness/pi/`.

## Evidence boundary

[R2](../roast-pr-2/report.md) concluded clean, zero findings, converged on the original reviewed source and original base above. It did **not** review the new base. This audit establishes patch equivalence and accounts for the integration delta; it does not constitute another whole-branch review or a post-rebase test run.

Main permits carrying the [native HOL](f1-native-hol.md), [compatibility](f1-final-compatibility.md) and [scoped live](final-live.md) evidence for unchanged Pi source. Those results retain their original source anchors and exact `a8aef2df…` binary identity. Upstream version and Cargo changes mean this accounting does not assert an identical newly built binary. [The full-suite result](full-suite-attempt-2.md) is likewise historical candidate evidence; separate integration checks are coordinator-owned. This audit ran no builds, tests, native sessions or provider requests.
