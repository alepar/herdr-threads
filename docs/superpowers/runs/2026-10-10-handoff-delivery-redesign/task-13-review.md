## Spec Compliance

❌ Issues found on head `7d432fe0f4ad3de85c96a029aedfb6e5b5ff5845`, complete base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`: historical recovery-result ceilings are enlarged unconditionally, and the new embedded-original response check does not preserve exact path spelling (Important findings R13-1 and R13-2 below).

⚠️ Verification limits: this is the complete Task13 source/test review, not approval of imported Task12 schema history, Task15 consumer behavior, composed B/D blobs, final default-feature validation, or whole-branch readiness. I read the package's stat and all 6,494 lines, including the first 400 lines of changed tests and every remaining test hunk. No Cargo commands or tests were rerun. The FINAL AUTHOR PACKAGE reports the exact final 32-test nextest selection, focused groups, clippy38.63s, fmt and harness-neutral success; its raw execution transcript is not duplicated in this artifact, so these are author-run evidence, not reviewer execution claims.

## Strengths

- `src/store/control.rs:761` derives modern selection from canonical membership and pending invitation state, preserving historical NULL confirmation/origin honestly instead of echoing requested policy. `tests/cli/direct_send.rs:1093`, `:1138`, `:1249` exercise immutable existing episodes, keyed replay, actual outcomes and historical serialization.
- `src/protocol/handoff/automatic_bootstrap.rs:559` checks actual nested future generated requests, response variants, startup/native argv and local shapes. Public boundary and maximum-escaped-body regressions at `tests/cli/topology_handoff.rs:6790` and `:7308` complement the independent canonical first-Begin refusal at `tests/handoff_topology_authority.rs:2374`.
- `src/cli/handoff.rs:2315` hashes full generated diagnostics separately from the bounded canonical summary; summary durability precedes optional artifact IO. `tests/cli/topology_handoff.rs:7150` verifies persisted summary, artifact failure, reopened retry and one native start.
- `src/cli/topology_handoff/automatic.rs:1140` validates modern Completed attachment and summary before permitting missing optional child information. `tests/cli/topology_handoff.rs:6920` explicitly forbids mutation/new-capability reconstruction and distinguishes absent from corrupt child progress.
- Changed legacy tests retain their meaningful checks: the consistently edited direct hint fixture edits the new selection hint too, and the historical report projection test still asserts exact full canonical summary retention. New transport tests cover real socket exchange, length refusal before body read, wrong result family and the unchanged default encoder.

## Issues

### Critical

None identified.

### Important

**R13-1 — Historical recovery retained-result bounds are widened without the required policy discriminator.** `src/store/topology_handoff/automatic.rs:177`, `:179`, `:956`, `:958`, and `:1076` now read/decode/write every recovery result with a 2MiB ceiling. The binding contract, repeated in `TRUST-POLICY.md:640`, retains a 256KiB complete result ceiling for absent-policy historical originals; only modern Some receives 2MiB. The conditional historical request limit at `src/protocol/handoff/automatic_bootstrap.rs:350` does not enforce the result limit: an attached cancellation can repeat the original in identity, attachment and recovery.attached_child, whereas its request has fewer copies. Consequently a historically refused aggregate can now be committed, and an oversized historical recovery blob can pass readers. Select the recovery-result read/decode/encode ceiling from the frozen original's policy in all three paths. Add a genuine historical result >256KiB negative paired with a modern positive; preserve exact historical replay below the ceiling.

**R13-2 — Embedded-original correlation uses normalized PathBuf equality rather than the promised exact serialized original.** `src/client/local.rs:229` compares the response identity using derived Eq. `AutomaticBootstrapIdentity` contains PathBuf fields and derives plain Deserialize (`src/protocol/handoff/automatic_bootstrap.rs:102`); Path equality normalizes component spelling such as trailing separators. The new response path does not call identity.validate or compare serialization. A response that changes only a namespace/cwd path spelling while retaining the original digest can therefore pass this guard and normal request/instance/boot correlation despite containing a different serialized original. This is the same representation concern that correctly withdrew the store typed-Eq optimization. Compare exact serialized identities (and reject invalid identity/digest if validating separately). Extend `tests/daemon/transport.rs:3647` with a path-spelling-only mismatch; its current execution-ID mismatch does not cover this case. This is cooperative corruption/correlation handling, not a request for adversarial caller verification.

### Minor

**R13-3 — Remaining permit-window serialization leaves little demonstrated headroom.** `src/store/topology_handoff/automatic.rs:122` validates/encodes the retained and supplied full originals during current-state reads, and child canonical checks likewise repeat full-envelope work. Retaining exact serialization avoids the PathBuf semantic regression, but the reported solo180–192ms store-entry-to-decision latency consumes most of the unchanged250ms permit and excludes earlier runtime work. The nextest group at `.config/nextest.toml:159` prevents proven test overlap; it does not establish production scheduling headroom. Keep this explicitly documented and investigate reuse of already validated canonical bytes within one transaction as a separate measured improvement, preserving strict decoding, request validation, exact byte identity and fresh authority. No timeout increase, cache authority, or guard relaxation is recommended. Send379/554ms measurements are total mutation durations, not pre-decision ages; the failing run's actual thread count is unknown.

## Assessment

**Spec verdict:** Issues found.

**Quality verdict:** Needs fixes.

**Task quality:** Needs fixes.

**Reasoning:** Modern policy composition, admission, no-relaunch progress and tests are substantial and generally coherent, but two boundary checks contradict the task's explicit historical/exact-original contracts. Correct R13-1 and R13-2 and obtain scoped fresh review of their fixes before this task is approved.

**Focused checks outside package context:** For the named historical-result risk, read the unchanged recovery flow around `src/store/topology_handoff/automatic.rs:970` to distinguish request and retained-result copies and verify write ordering. For the named exact-response-identity risk, read identity Deserialize and `src/protocol/wire.rs:242` result validation; the latter does not validate automatic identity/digest. Read the cut-off modern Completed presentation function to verify absence/corruption and summary-validation ordering. No broader repository crawl, git mutation, tracker action, Herdr action, or test execution occurred.
