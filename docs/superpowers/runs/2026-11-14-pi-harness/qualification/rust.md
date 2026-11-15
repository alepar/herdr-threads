# Pi Rust adapter verification

Implementation snapshot: `ca4474c06ae263c8bc253aea623c029ba0a31801`. The initial independent task review found setup duplication and malformed trust-section placement. Both were repaired and the scoped re-review approved them; see [initial review](rust-review.md), [fix evidence and SHA map](rust-fix1.md) and [re-review](rust-rereview-1.md). The accepted refactor source is `4a784faa9`; native runtime qualification remains separate.

The shared changes are split into `64f087e4b` (optional selected-process validation), `3e1e6664f` (fallible local evidence before orchestration) and `564bd13a9` (channel-owned lifecycle selection/output). Pi activation, trust amendments and public fixtures are in `ca4474c06`.

## Completed checks

| Check | Outcome |
|---|---|
| Focused Rust nextest, library plus combined/integration targets | 1,232 passed; 7,999 unrelated tests excluded; 17.433 s runtime, 56.98 s build |
| Extension callback tests including refusal/no-watch/retry | 37 passed; production extension unchanged |
| Locked all-target/all-feature clippy, warnings denied | Passed, 42.30 s |
| Default-feature gate | Passed |
| Formatting at the committed revision | Passed |
| Harness-neutral and wire compatibility guards | Passed |
| Run-scoped process and temporary-file leak checks | Passed for `pi-task2-20261114` |

Tests first demonstrated failures for missing pi registration/process attribution, then passed after implementation. A new compatibility assertion caught an accidental inversion in the existing hook-pull continuation predicate; the original behavior was restored and verified. Separate refusal regressions failed against successful-empty hook refusal behavior before the channel-owned sentinel correction, then passed through both the runner and actual binary paths. Public golden changes were regenerated, inspected, then verified without blessing.

The fixed refusal signal is `herdr-threads: check-in unavailable (hook refused).` followed by a newline, 52 UTF-8 bytes. It contains no rejected payload or arguments and carries no completion evidence. Successful empty output remains valid. Existing adapters retain quiet refusals.

Initial cold/check-to-test codegen took 7m13s; intermediate changed-target builds took 3m33s and 1m41s under the host's four-job training-priority cap. A later incremental source build took 23.01s. No concurrency increase, duplicate target tree or unrelated build optimization was used.

## Shared change accounting

The following is the implementer's compatibility accounting, subject to independent review.

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
| `src/harness/mod.rs` | Export pi adapter module | Existing exports retained | Registry activation and neutral guard |
| `src/harness/unsupported.rs` | Transfer pi marker ownership to registered metadata | Agent/person guard still sees the same marker | Marker/caller/public CLI tests and env-marker golden |
| `src/host/native.rs` | Replace pi-as-unknown test fixtures | Production code unchanged | Existing unknown-agent wake/UI refusal tests |
| `src/cli/help.rs` | Spell out process IDs, avoiding new pi token false positive in neutral scanner | Same operational guidance | Neutral guard and help golden |

## Remaining qualification boundary

These checks do not establish actual native pi process admission, native TUI draft/picker safety, full lifecycle/reconnect behavior, authenticated provider/model continuity or the final unfiltered integrated suite. Those remain the next qualification task. No credential was read or used for these checks. No real harness configuration or shared server was modified. The warm worktree and retained SDK dependencies remain owned by the coordinator for qualification and are removed after integration.
