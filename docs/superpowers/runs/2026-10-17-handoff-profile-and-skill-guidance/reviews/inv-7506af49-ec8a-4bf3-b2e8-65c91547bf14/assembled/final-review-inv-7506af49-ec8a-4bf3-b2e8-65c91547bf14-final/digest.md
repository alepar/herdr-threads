## diffstat

CHANGELOG.md                                       |    4 +
 README.md                                          |    6 +
 docs/agent-usage.md                                |   40 +-
 ...17-handoff-profile-and-skill-guidance-design.md |   86 +
 .../code-profile-original.json                     | 2426 ++++++++++++++++++++
 .../code-profile-original.md                       |  259 +++
 .../coverage-changes-round-2.md                    |    8 +
 .../coverage-findings-round-1.md                   |    7 +
 .../coverage-findings-round-2.md                   |    0
 .../coverage-inputs-round-1.md                     |   40 +
 .../coverage-inputs-round-2.md                     |   48 +
 .../coverage-ledger.md                             |   13 +
 .../coverage-precheck-round-1.md                   |    1 +
 .../coverage-precheck-round-2.md                   |    1 +
 .../coverage-round-1-requirements.md               |   12 +
 .../coverage-round-1-result.md                     |    3 +
 .../coverage-round-1-review-1.md                   |   38 +
 .../coverage-round-1-review-2.md                   |   28 +
 .../coverage-round-2-prompt.md                     |   88 +
 .../coverage-round-2-requirements.md               |   12 +
 .../coverage-round-2-result.md                     |    7 +
 .../coverage-round-2-review-1.md                   |   27 +
 .../coverage-round-2-review-2.md                   |   26 +
 .../design-handoff.md                              |   13 +
 .../design-tree.json                               |  463 ++++
 .../evidence/baseline-nextest.log                  |  500 ++++
 .../evidence/baseline.md                           |   11 +
 .../evidence/profile-directories.md                |   78 +
 .../friction.md                                    |   14 +
 .../graph-after.md                                 |    7 +
 .../graph-before.md                                |    7 +
 .../graph-pass-prompt.md                           |   50 +
 .../graph-review.md                                |    7 +
 .../promotion-review.md                            |   13 +
 .../review-intent-recovery.md                      |  130 ++
 ...iew-inv-8759cc82-264d-4506-beaa-307c9600bb8d.md |  126 +
 ...iew-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f.md |  138 ++
 .../run.md                                         |   56 +
 .../task-1.md                                      |   18 +
 .../task-10.md                                     |   21 +
 .../task-2.md                                      |   14 +
 .../task-3.md                                      |   14 +
 .../task-4.md                                      |   14 +
 .../task-5.md                                      |   14 +
 .../task-6.md                                      |   18 +
 .../task-7.md                                      |   13 +
 .../task-8.md                                      |   14 +
 .../task-9.md                                      |   14 +
 .../task-ids.json                                  |   12 +
 docs/superpowers/specs/INDEX.md                    |    1 +
 docs/validation/herdr-release-matrix.md            |   58 +-
 integrations/skill/SKILL.md                        |    6 +-
 scripts/herdr-release-smoke                        |   15 +-
 src/cli/commands.rs                                |   59 +-
 src/cli/handoff_delivery.rs                        |    2 +
 src/cli/help.rs                                    |   16 +-
 src/cli/launch.rs                                  |   57 +-
 src/cli/setup.rs                                   |    1 +
 src/cli/topology_handoff.rs                        |  134 +-
 src/cli/topology_handoff/automatic.rs              |   31 +-
 src/cli/topology_runtime.rs                        |   11 +
 src/daemon/control.rs                              |    5 +
 src/harness/adapter.rs                             |   15 +
 src/harness/claude.rs                              |   13 +
 src/harness/codex.rs                               |   13 +
 src/harness/hermes.rs                              |   99 +-
 src/harness/hermes/launch.rs                       |   26 +
 src/harness/launch.rs                              |   10 +-
 src/host/native.rs                                 |    7 +-
 src/host/transport.rs                              |    2 +-
 src/ports.rs                                       |    2 +
 src/protocol/capabilities.rs                       |    4 +
 src/protocol/handoff.rs                            |  188 +-
 src/protocol/handoff/automatic.rs                  |    3 +
 src/protocol/handoff/automatic_bootstrap.rs        |   10 +-
 src/protocol/handoff/launch_directories.rs         |  174 ++
 tests/cli/follow_retry.rs                          |    1 +
 tests/cli/handoff.rs                               |    1 +
 tests/cli/launch.rs                                |    1 +
 tests/cli/topology_handoff.rs                      |  501 +++-
 tests/daemon/transport.rs                          |    1 +
 tests/fixtures/golden/help/handoff.txt             |   32 +-
 tests/fixtures/herdr-matrix/codex.js               |   11 +-
 tests/handoff_agy.rs                               |   48 +
 tests/handoff_topology_authority.rs                |   10 +
 tests/handoff_topology_cli.rs                      |  100 +-
 tests/harness/agy_launch.rs                        |    1 +
 tests/harness/launch.rs                            |  291 +++
 tests/host/topology_creation.rs                    |  101 +-
 tests/integration/channel_activation.rs            |    1 +
 tests/integration/herdr_release_smoke.rs           |  258 ++-
 tests/integration/hermes_enrollment_smoke.rs       |   87 +-
 tests/integration/topology_activation.rs           |    1 +
 tests/lifecycle_ux.rs                              |    1 +
 tests/service/resolution.rs                        |    1 +
 tests/store/archival_legacy.rs                     |    2 +
 tests/store/topology_attachment_guard.rs           |   37 +-
 tests/store/topology_handoff.rs                    |   37 +-
 98 files changed, 7384 insertions(+), 61 deletions(-)

## generated-input provenance

## Generated-input provenance
Captured range: b4d5f9628d17f869ab5b139b7f4ae3c933d02841..89b60fcd07b71888608f0b1d67dadb6de38c3ff4
Selection: complete; selected product/input paths: 98; omitted historical payloads: 12.
diffstat.txt describes selected product/input paths. Reports, findings and collection metadata remain ordinary review inputs.
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/assembled/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d-final/bounded-original-draft.md","sha256":"3e95b9346deda8956adefbd62333b67d381e28eb522071278793358838e0bc0b","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":35,"invocationId":"inv-8759cc82-264d-4506-beaa-307c9600bb8d","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/assembled/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d-final/coverage.json","sha256":"57737e3dd8ae4dc3d47cc76f614c7ddca330dd180bb64ca18952b0854bc664ba","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":35,"invocationId":"inv-8759cc82-264d-4506-beaa-307c9600bb8d","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/assembled/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d-final/diffstat.txt","sha256":"6d0e0b9d6d24d4bec04418078d19f99565158189c84a00a83ec83590a23d7a89","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":35,"invocationId":"inv-8759cc82-264d-4506-beaa-307c9600bb8d","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/assembled/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d-final/digest.md","sha256":"4eb3f3df8b2bb88945b3431665893e9c5c1aeff0d3e86b47e46b7bf46b3bbf3f","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":35,"invocationId":"inv-8759cc82-264d-4506-beaa-307c9600bb8d","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/assembled/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d-final/generated-input-provenance.md","sha256":"36eb6457c7100233d00b582fb41983ec091a6d8e4bab4a287c827f6d1f80d12b","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":35,"invocationId":"inv-8759cc82-264d-4506-beaa-307c9600bb8d","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/assembled/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d-final/review-inputs.json","sha256":"1a41bf538b43254bee5684b3e82a2ce05256b64bbef5391d977e14e44c744509","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":35,"invocationId":"inv-8759cc82-264d-4506-beaa-307c9600bb8d","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-8759cc82-264d-4506-beaa-307c9600bb8d/final-review-inv-8759cc82-264d-4506-beaa-307c9600bb8d.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/assembled/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f-final/bounded-coverage.json","sha256":"171e924c711f045dda3466a143c72685fc64987f850387afa2d115f40a81b07c","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":42,"invocationId":"inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/assembled/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f-final/bounded-draft.md","sha256":"65f29c97477956f8c183bd61e5a98d330e90fa0dfd6516ce3b8e7f315a49e4a4","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":42,"invocationId":"inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/assembled/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f-final/diffstat.txt","sha256":"04f9782919ade98f3377348588c0e35818108e6a31ff0f4ca2ab7caf9b247ff8","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":42,"invocationId":"inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/assembled/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f-final/digest.md","sha256":"0c69a5996894474a58d5c88624cd7bb35434b4b4845374c05bbc817c474662d6","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":42,"invocationId":"inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/assembled/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f-final/generated-input-provenance.md","sha256":"e2bf6e81470eb0adecf7775d7f6ac7ae971d5f1047e1ec74b422fb97c0a63aed","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":42,"invocationId":"inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f.md"}
- {"path":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/assembled/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f-final/review-inputs.json","sha256":"b11e675492e2f1ca076869120b688baf45091b9c482b1db8089786526f187a9d","revision":"89b60fcd07b71888608f0b1d67dadb6de38c3ff4","run":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md","recordLine":42,"invocationId":"inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","archive":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f","report":"docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/reviews/inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/final-review-inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f.md"}
Open each report/archive at its recorded revision. Omission supplies no finding disposition or verdict.

## report summaries

task-1
Status: none
Assumptions:
No bead amendments/comments existed. Directory context is named directory_context consistently at transport boundaries; the projection seam is explicit_directory_env. New cwd-only selection may retain existing historical cwd behavior without a profile context. Present-child execution tests remain de
Concerns:
None blocking. Clippy's reported 67s includes lock contention; this run does not isolate a standalone lint performance measurement.

task-10
Status: none
Assumptions:
none
Concerns:
none

task-11
Status: IMPLEMENTED. Final authoritative result and checks are appended below.
Assumptions:
none
Concerns:
Task9 matrix remains owned by its existing agent and must run against merged fix; no matrix files taken. Matrix GREEN is not claimed here. Parent owns full-suite/final integration. No account switch, thread mutation, push, stash, shared server or real user config operations.

task-2
Status: IMPLEMENTED; completionKind: implementation.
Assumptions:
Current bead and frozen brief agree; no comments/amendments. This leaf leaves launch-option/profile examples to the separate public CLI documentation leaf as explicitly directed by its brief. Static prose uses the requested existing Herdr skill as recovery mechanism; does not execute it or send thre
Concerns:
Remaining skill byte/line allowance must be honored by later profile-example change. Build timing and sandbox failure details above are limitations of this check configuration; no baseline attribution is asserted.

task-3
Status: IMPLEMENTED; completionKind: implementation; stacked: true.
Assumptions:
none
Concerns:
Initial warm-seeded focused-test compilation took 3m40s; initial GREEN and final clippy took 1m02s. Final test recompilation took 41.86s. The over-budget build/lint measurements are recorded without claiming they are pre-existing or attributing them to another task; no baseline timing comparison was

task-4
Status: IMPLEMENTED; completionKind: implementation.
Assumptions:
none
Concerns:
The initial new-worktree lib test artifact rebuild took 3m31s despite the required warm seed; subsequent measured incremental builds were 55.16s, 29.35s, 27.59s, 26.96s and 19.46s, and final Clippy was 59.59s. No build-configuration changes were made.

The environment projection is declared child in

task-5
Status: none
Assumptions:
none
Concerns:
- All-target clippy took 1m03s, exceeding the repository one-minute lint budget by three seconds. No task-base lint timing was measured, so regression attribution is not established.
- First-checkout builds took 2m10s (--lib RED) and 1m27s (combined); the measured incremental GREEN --lib build took 36.15s.
- Synthetic owned Python/host fixtures exercise mapping, scope, prelaunch and refusal behavior; no real native authenticated harness was invoked.

task-6
Status: none
Assumptions:
Read current bead description/acceptance and comments before implementation; no comments or amendments existed. Current bead and frozen brief agree. Read trust policy before examining canonical authority paths. Existing exact canonical retention validators already bind optional context through compl
Concerns:
Initial task-local seeded rebuilds took 2m39s (`--lib` RED) and 2m24s (normal-library/combined RED). Incremental GREEN builds were 52.66s (`--lib`) and 14.91s (combined). Required clippy passed but took 1m01s, one second over the stated per-change budget; no measured baseline supports attribution, a

task-7
Status: none
Assumptions:
none
Concerns:
none

task-8
Status: none
Assumptions:
- Current bead and comments read before implementation: no amendments/comments. Applied compatible frozen brief and current acceptance together.
- Task7 is landed; preserved its guidance and counted this example against its shared growth budget.
- Help points to adapter mapping instead of hardcoding harness names in neutral core, consistent with repository architecture.
- The initial fresh-worktree build/lint timings exceeded the budget as listed in RESULT concerns. Final rebased builds/lint were within1minute. No causal baseline performance attribution claimed.
- No behavioral-evaluation agents: explicitly excluded by leaf scope.
Concerns:
none

task-9
Status: IMPLEMENTED (implementation). Branch: task-ht-1urq.9.
Assumptions:
none
Concerns:
none

## interfaces and seams

src/cli/launch.rs: pub directory_context: Option<crate::protocol::handoff::LaunchDirectoryContext>,
src/cli/topology_handoff.rs: pub(crate) fn require_directory_capability<C: crate::ports::LocalClient + ?Sized>( client: &C, clock: &dyn crate::protocol::time::Clock, ) -> Result<(), RunError>
src/cli/topology_handoff.rs: pub(crate) fn check_directories( context: Option<&crate::protocol::handoff::LaunchDirectoryContext>, ) -> Result<(), RunError>
src/cli/topology_handoff.rs: pub(crate) fn check_bootstrap_directories( cwd: &std::path::Path, context: Option<&crate::protocol::handoff::LaunchDirectoryContext>, ) -> Result<(), RunError>
src/cli/topology_handoff.rs: pub profile_dir: Option<std::path::PathBuf>,
src/harness/adapter.rs: fn explicit_directory_env( &self, _: &std::path::Path, ) -> Result<std::collections::BTreeMap<String, String>, crate::protocol::results::ApiError>
src/harness/adapter.rs: pub explicit_profile_directory: Option<std::path::PathBuf>,
src/harness/claude.rs: fn explicit_directory_env( &self, directory: &std::path::Path, ) -> Result<std::collections::BTreeMap<String, String>, crate::protocol::results::ApiError>
src/harness/codex.rs: fn explicit_directory_env( &self, directory: &std::path::Path, ) -> Result<std::collections::BTreeMap<String, String>, crate::protocol::results::ApiError>
src/harness/hermes/launch.rs: fn explicit_directory_env( &self, directory: &std::path::Path, ) -> Result<BTreeMap<String, String>, ApiError>
src/ports.rs: pub directory_context: Option<crate::protocol::handoff::LaunchDirectoryContext>,
src/protocol/capabilities.rs: pub const HANDOFF_LAUNCH_DIRECTORIES_V1: &str = "handoff.launch_directories_v1";
src/protocol/handoff.rs: pub use launch_directories::LaunchDirectoryContext;
src/protocol/handoff.rs: pub directory_context: Option<LaunchDirectoryContext>,
src/protocol/handoff/automatic_bootstrap.rs: pub directory_context: Option<LaunchDirectoryContext>,
src/protocol/handoff/launch_directories.rs: pub struct LaunchDirectoryContext
src/protocol/handoff/launch_directories.rs: pub fn validate(&self) -> Result<(), &'static str>
src/protocol/handoff/launch_directories.rs: pub fn validate_bootstrap( &self, cwd: &Path, env: &BTreeMap<String, String>, ) -> Result<(), &'static str>
src/protocol/handoff/launch_directories.rs: pub profile_dir: PathBuf,
src/protocol/handoff/launch_directories.rs: pub cwd: PathBuf,
src/protocol/handoff/launch_directories.rs: pub env: BTreeMap<String, String>,
new public file: src/protocol/handoff/launch_directories.rs
ht-1urq.1: Seam contract: frozen launch directory context

## deferred minors

Task 2 (ht-1urq.7): minor (deferred): [check-command-warning] nice reported setpriority: Operation not permitted; clippy completed successfully
Task 6 (ht-1urq.5): minor (deferred): [check-duration] clippy61s, no measured baseline attribution; retain final integration timing
Task 7 (ht-1urq.6): minor (deferred): [attribution-gap] concurrent topology test failure unestablished; exact task base/final libtest+nextest controls pass; topology-failure-investigation.md
Task 10 (ht-1urq.10): minor (deferred): [execution-warning] nice could not set priority; checks passed
Metrics: completions — review clean 10 · after fix pass 1 · parked 0 · re-entry closes 0 · dispatched early 8 · cancelled 0

## amendments

none
