# Task 2 fix round 1 report

Scope: review findings I1 and M1 only. Reviewed HEAD `ca4474c06ae263c8bc253aea623c029ba0a31801` remains historical. Fix-round starting HEAD was `f2f0032d7`; shared base is `cf93b1bef`. No Task 3 work, extension production edit, new behavioral adapter interface or wake-policy change belongs to this round.

## Changes and compatibility

I1 extracts the duplicated bounded reads, digest ownership/planning, pending-digest crash recovery, publication, status inspection and conservative removal into `src/harness/owned_files.rs`. The module contains no harness names. Callers supply rendered bytes, local paths/directories, an adapter diagnostic prefix and opaque serialized metadata. Opencode retains its permission decision and `permissions_changed` report; flattened metadata occupies precisely the existing permission field position. Pi supplies empty metadata, retaining its five-field manifest and two owned files. Rendering, output projections, diagnostics and permission behavior stay adapter-owned. No opencode golden is edited.

The helper/opencode commit is independent of Pi source and is ordered immediately after the shared base; Pi conversion is separate. Existing pending-upgrade, symlink, foreign/modified-file, directory cleanup, permission and reinstallation tests exercise the shared engine through the real callers. An added Pi assertion pins its manifest field count/two-file scope and unchanged reinstallation bytes; it passed against the original implementation before conversion.

M1 restores the original inline Accepted limits reference and relocates the unchanged Pi trust amendment to a real section boundary immediately before Accepted limits. Its normative text is unchanged.

## Actual historical fixture provenance

The opt-in test `historical_v320_manifest_upgrades_without_changing_owned_bytes_or_projection` invokes an explicitly supplied installed executable in isolated HOME, state, configuration, cwd and empty PATH. It verifies `herdr-threads 3.2.0` before generation. No reconstructed manifest is substituted. The gate is ignored normally with an explicit reason, and selecting it without `HT_SETUP_V320_BINARY` fails.

Producer binary SHA256: `4e67207b708a807ab50e757628101c3ac3dd584d5e4f173eb8b805e60a1f4bb3`. Tag `v3.2.0`: `24231d9478cdbf482c4927bc53b1e9c3dd5eb2bc`. Its setup source SHA256 is `d558233443a4d99a0d1fd1d6799d69e97f5b036cf3d07fb701d01eaaf9406480`; that source is identical at the shared base and before extraction. The embedded plugin and shared skill also have no diff from the tag. Version is historical fixture provenance only, never an operational admission/setup gate.

For both permission variants, old code actually writes the manifest and plugin/skill. New deserialization/serialization equals the old raw manifest byte-for-byte; stored digests match old-produced files. New setup classifies the installation already installed and preserves all raw bytes. Setup/status/doctor/unsetup projections are compared with the old executable's actual JSON; modified and foreign files survive subsequent new setup/unsetup. Existing portable tests remain enabled normally; the historical gate is reported separately and is not implied by a normal/unfiltered suite.

## Complete core-change accounting

The original three approved behavior seams retain their defaults; this fix adds concrete shared ownership support, not a fourth policy seam.

| Changed core file outside `src/harness/pi/` | Seam / purpose | Existing/default behavior | Evidence pinning compatibility |
|---|---|---|---|
| `src/harness/adapter.rs` | Optional process-claim validator; channel-owned OutputPolicy; fallible existing local evidence | None / false / Ok(default) | Adapter, caller-env and all shipped adapter tests; hook-pull continuation regression |
| `src/harness/caller_env.rs` | Check selected ancestor before fingerprint | Absent callback preserves prior path | Existing causal caller-env tests and new pi CLI/hook process-table matrix |
| `src/harness/registry.rs` | Forward fallible evidence, neutral policy lookup, pi registration | Unregistered/human policy false; existing registration order retained | Registry, adapter-seam, builtin capability/delivery tests |
| `src/harness/claude.rs` | Compile-consistent explicit default fields | Validator None, channel policy false | Claude decode/setup/caller/composer tests |
| `src/harness/codex.rs` | Compile-consistent explicit default fields | Validator None, channel policy false | Codex decode/setup/caller/composer tests |
| `src/harness/hermes.rs` | Compile-consistent explicit default fields | Validator None, channel policy false | Hermes lifecycle/admission/setup/caller tests |
| `src/harness/agy/mod.rs` | Explicit default fields; wrap existing evidence in Ok | Existing agy evidence and hook delivery preserved | Agy witness, admission, delivery, setup tests |
| `src/harness/opencode/mod.rs` | Compile-consistent explicit default fields | Validator None, channel policy false | Opencode contract/decode/caller/setup/channel tests |
| `src/cli/hook.rs` | Propagate evidence refusal before orchestration; channel-owned refusal sentinel; omit hook mail and preserve legacy coverage | Successful existing evidence, non-channel-owned quiet output/composition and success-empty output unchanged | Real socket-backed hook tests, SessionEnd no-binding-mutation regression, legacy hook-pull continuation regression |
| `src/harness/bridge.rs` | Complete lifecycle operation without composing channel mail | Branch taken only for channel-owned adapter | Canonical CheckIn tests and pi socket-backed hook test |
| `src/store/mod.rs` | Skip channel-owned CheckIn inbox hydration and notice selection | Existing query/offer path remains in else branch | Existing cooperative CheckIn/notice tests and pi canonical state snapshot |
| `src/store/seats/cooperative.rs` | Skip warning/notice presentation frontiers for channel-owned offers | Existing lifecycle registration, rotation, history/cursor seeding and other-adapter frontiers retained | Existing cooperative lifecycle/replay/notice tests; pi repeated Current/lifecycle checks |
| `src/harness/mod.rs` | Export shared ownership support and pi adapter | Existing exports retained | Registry activation, neutral guard, standalone-helper ordering |
| `src/harness/unsupported.rs` | Transfer pi marker ownership to registered metadata | Agent/person guard still sees the same marker | Marker/caller/public CLI tests and env-marker golden |
| `src/host/native.rs` | Replace pi-as-unknown test fixtures | Production code unchanged | Existing unknown-agent wake/UI refusal tests |
| `src/cli/help.rs` | Spell out process IDs, avoiding new pi token false positive in neutral scanner | Same operational guidance | Neutral guard and help golden |
| `src/harness/owned_files.rs` | Extract existing concrete file ownership engine; no new adapter policy | Callers retain paths/rendering/metadata meaning/output; existing bounded reads and digest recovery preserved | 26 portable opencode tests, explicit actual-3.2.0 gate, Pi manifest/preservation tests |
| `src/harness/opencode/setup.rs` | Delegate ownership operations; local permission metadata/projections retained | Same manifest field order/path/digests, permissions, setup/status/doctor/unsetup bytes | Actual-3.2.0 manifest round-trip plus all portable ownership/permission tests and unchanged goldens |

## Commit reorder and integrity

The authorized own-branch interactive reorder replayed all 19 commits without conflict. Standalone helper `7db3f97d3` is immediately after `cf93b1bef`, ahead of all extension/docs/Pi commits, and its patch contains no Pi path or name. Pi conversion follows separately. No branch except `pi-harness` was changed, and no file/commit was discarded.

Pre-reorder source HEAD: `15489685251297853bf56ada94883e5c04a65568`. Final replayed source HEAD: `4a784faa9270e4f5fe7971264bb0b1a4fa3f0233`. Both have identical tree `ee1a61cb713e149d5ae2a107e36d0121462d4f86`; `git diff --exit-code` passed. Range-diff reports only expected module-declaration context differences for helper/activation; all other patches are equal. No behavioral conflict ruling was needed. The coordinator had pre-approved preserving only the helper declaration at base and both declarations when activation replays; Git applied this automatically.

| Old SHA | New SHA | Commit |
|---|---|---|
| `c50340a6b` | `385c8ed81` | docs: research and design pi harness integration |
| `74e115f5a` | `a3c117456` | docs: record pi integration base and model continuity check |
| `228502ec0` | `69b96a187` | docs: freeze pi extension adapter contract for implementation |
| `dc581e95f` | `a0d373413` | Add native pi extension boundary delivery |
| `4643aab03` | `f3fa476e7` | docs: record pi runtime retry and ordering constraints |
| `6bcff9047` | `a557bf760` | Fence pi attention reoffers and join detached watch cleanup |
| `8ba848027` | `9dbeb2f4a` | docs: define bounded pi wake novelty fallback |
| `aa744e7a4` | `c04eb1ee9` | Rearm recovered pi wake witnesses for the successor owner |
| `568a69579` | `9904c2423` | docs: record accepted pi extension evidence and integration seams |
| `2efdd555f` | `15b869dae` | docs(pi): require fail-closed hook owner evidence |
| `2e76982e2` | `24bd953f6` | docs(pi): distinguish hook refusal from empty success |
| `64f087e4b` | `9b0ad41a4` | Add optional process claim consistency validation |
| `3e1e6664f` | `2060c2639` | Refuse failed local evidence before hook orchestration |
| `564bd13a9` | `39882395a` | Allow channel-owned lifecycle output without mail offers |
| `ca4474c06` | `e61d0e355` | Register the pi TUI adapter and owned extension setup |
| `f2f0032d7` | `da32307b4` | docs(pi): record Rust adapter verification and core seams |
| `6b08f4b21` | `d120a3582` | Repair the pi trust amendment section boundary |
| `c73f56ca1` | `7db3f97d3` | Share digest-owned file operations without changing installed state |
| `154896852` | `4a784faa9` | Use shared owned-file operations for pi setup |

## Validation evidence

All Rust/test/build commands use `heavy tb` with the existing seeded target and default job/test caps of four. No full suite, stress/multi-run gate, credentialed call, new target or second concurrent build was run. Scoped process/scratch tag: `pi-task2-fix1-20261114`. The extraction is a refactor under existing green behavioral coverage; test harness mistakes below are not claimed as production RED evidence.

- Historical harness initial executions exposed old CLI grammar (`setup` must precede options), then the doctor's legitimate nonzero exit with JSON. Corrected the fixture invocation/doctor expectation. Earlier builds took 1m30s and 39.97s. The first real historical compatibility pass after extraction compiled in 2m39s and ran 1/1 in 0.07s.
- `heavy tb env HT_LEAK_RUN_ID=<tag> cargo test --locked --all-features --lib harness::opencode::setup::tests`: 26 passed, 1 explicit historical test ignored; build 1m26s, run 0.01s. This includes pending-digest recovery across two interrupted upgrades, permissions, symlinks, modified/foreign preservation, install/reinstall and cleanup.
- Pi pre-conversion baseline: an obsolete `harness::pi_tests` filter selected zero tests after 1m12s compile and is not evidence. Corrected `heavy tb env HT_LEAK_RUN_ID=<tag> cargo test --locked --all-features --lib harness::pi::tests`: 9/9 passed, including the added manifest assertion, 0.04s build / effectively 0s runtime.
- Final focused source selection: `heavy tb env HT_LEAK_RUN_ID=<tag> cargo nextest run --locked --all-features --lib --status-level fail --final-status-level fail -E 'test(harness::pi::) | test(harness::opencode::) | test(harness::setup::) | test(local_evidence_tests::) | test(hook_pull_lazy_tests::)'`: 101/101 passed, build 3m10s / run 0.248s.
- Required historical gate on final source: `heavy tb env HT_SETUP_V320_BINARY=<verified-producer> HT_LEAK_RUN_ID=<tag> cargo test --locked --all-features --lib historical_v320_manifest -- --ignored --nocapture`: 1/1 passed, 0.05s build / 0.06s run. This is separate from normally enabled coverage.
- Missing-producer negative check: `heavy tb env -u HT_SETUP_V320_BINARY cargo test --locked --all-features --lib historical_v320_manifest -- --ignored --nocapture`: expected exit101, 1 failure with `HT_SETUP_V320_BINARY is required for this explicit historical gate`; build0.02s / run effectively0s. The ignored test cannot silently pass when explicitly requested without its producer.
- After reorder: `heavy tb env HT_LEAK_RUN_ID=<tag> cargo nextest run --locked --all-features --test combined --status-level fail --final-status-level fail -E 'test(golden_surfaces::) | test(adapter_smoke::)'`: 20/20 passed, build1m47s / run6.993s. No blessing flag or golden edit. Setup/status/doctor/unsetup, help, hook output and other public snapshots remain byte-identical.
- `scripts/check-harness-neutral`: passed, 0 violations,90 allowlisted occurrences,55 entries. Shared ownership source and standalone commit contain no Pi names.
- `scripts/check-wire-compat`: passed, 0 violations,0 stale,456 fields,24 variants,274 baseline entries. `git diff --check` passed.

Cold/check-to-test or replay rebuilds above are recorded separately from test runtime; no new target, upward job override or repeat gate was used to pursue timing. This round does not claim an incremental speed qualification.

- Initial extraction lint caught one needless borrow at removal; it was corrected without changing semantics. Final `heavy tb cargo clippy --locked --all-targets --all-features -- -D warnings`: exit0, `Finished dev profile ... in 1m11s`.
- `heavy tb scripts/check-default-features`: exit0, no output or warnings. Observed between wrapper start18:09:57 and the next gate at18:10:56 (under59s; the script emits no own elapsed time).
- `heavy tb cargo fmt --check`: exit0, quiet success on replayed source HEAD. Final report-only HEAD is checked again before handoff.
- `heavy tb scripts/check-no-leaked-processes --run-id pi-task2-fix1-20261114`: exit0, `no leaked test temp files` and `no leaked test processes` (0.28s wrapper call).

## Self-review and handoff

I1 and M1 are addressed. Final focused/public selections passed121 normally enabled tests, plus the separately selected real historical compatibility test. The expected missing-producer failure is negative gate evidence, not a passing product test. No model-visible production string is added; existing setup/status/doctor/unsetup bytes and goldens are retained. The helper owns no permission decision and introduces no HarnessAdapter trait, wire shape, provenance, identity rule or delivery/wake behavior.

Self-review checked manifest serialization order and path, pending-digest ownership/finalization, conflict-before-write ordering, symlink/size bounds, created-directory cleanup, edited/unowned preservation, adapter error prefixes and projection retention. Historical fixtures and owned children are removed by the fixture lifecycle; scoped leak checks confirm cleanup. The short-lived reorder editor and old-commit scratch JSON were removed after transferring the SHA map here. The existing seeded build directory is retained for coordinator/reviewer reuse; this branch is not merged.

The replayed source HEAD is `4a784faa9270e4f5fe7971264bb0b1a4fa3f0233` with tree `ee1a61cb713e149d5ae2a107e36d0121462d4f86`. The report-only commit follows it; its exact clean HEAD is supplied in the final handoff (a commit cannot contain its own hash). Full-suite, real native TUI/provider qualification and any later wake strategy remain Task3/coordinator scope; none is claimed here. No unresolved issue within this fix scope remains.
