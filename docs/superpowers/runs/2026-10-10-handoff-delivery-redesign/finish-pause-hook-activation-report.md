# Registered AGY Hook activation helper report

Prepared 2026-10-11. **Source preparation only: Cargo, formatting, lint, runtime/native execution and red/green observations are NOT RUN.** Parent owns the single Cargo lane, advertisement, reviews and integration. These tests do not establish readiness until parent executes and reviews them on the corrected active source.

Owned checkout: `.worktrees/handoff-delivery-redesign/.worktrees/finish-hook-activation`; branch `helper/handoff-finish-hook-activation`.

- Base: `4cb8aafe42906f5383aaaaa1531cb284a8287c5c` (production dormant4c).
- Tests-first checkpoint: `ef66bcdb` (`test: prepare elected registered AGY presentation activation proof`).
- Static source refinement: `88240e90` (`test: assert real hook receipt and deferred hint boundaries`).
- Changed paths: `tests/hook_entrypoint/agy.rs`, `tests/protocol/capabilities.rs` only. Both commits have truthful `Outside-adapter` trailers. No production advertisement or schema changes.

## New expected test names and assertions

All hook names below are in test target `hook_entrypoint`:

| Exact name | Expected active behavior, not yet observed |
| --- | --- |
| `agy::activation::activation_v2_registered_agy_system_hook_round_trip` | Real installed registered SessionStart enrolls against `continuity::Fixture` / `run_elected`; real child PreInvocation discovers actual Capabilities and selects PullV2; registered selector 0 produces exact native SystemMessage bytes; actual production HookStepCommit sends matching token and nonempty captured Attention controls; actual canonical reply is Confirmed with exact acknowledged controls; canonical outbox retires and emitted hints persist. |
| `agy::activation::activation_v2_registered_agy_user_hook_round_trip` | Same real elected/registered chain using selector 1 and exact UserMessage bytes after an accurately measured isolated executable identity is locally recorded as downgraded. Does not claim native AGY accepted the codec. |
| `agy::activation::activation_v2_registered_agy_failed_stdout_replays_without_completion` | Actual child stdout is a UnixStream whose reader is closed before spawn; actual PullV2 occurs but production completion sends no Confirm. Canonical token remains pending, local conversation hint bytes and downgrade/probation bytes stay unchanged. A successful subsequent registered child physically replays the same durable invitation/control descriptors and token and completes; passive descriptor replay does not spend ordinary-message probation. |
| `agy::activation::activation_v2_registered_agy_lost_completion_preserves_exact_recovery` | Test-local relay drops the first actual Confirm before forwarding, or drops its response after observing real canonical Confirmed. Both leave local emitted hints unchanged. Before-commit loss retains token and subsequent real child physically replays and confirms captured controls. After-commit response loss accepts actual exact request replay as AlreadyConfirmed with the same acknowledged controls. |
| `agy::activation::registered_agy_omitted_or_refused_capability_uses_historical_pull` | Real registered PreInvocation reaches a controlled historical peer (real daemon forwarding except V2 omission or capability refusal), emits historical SystemMessage, sends actual V1 DeliveryPull and no PullV2/ConfirmV2. This is exclusively negative/fallback evidence. |

Library test: `protocol::capabilities::tests::presentation_capability_routes_use_configured_domain`. It constructs real configured ControlService + DomainService + SqliteStore with cooperative owner/FairWriter, canonically checks in two seats and uses public CreateThread/RequireAck Invite producers. Actual Capabilities must advertise V2; PullV2 allocates a real nonempty-control page; exact Confirm and AlreadyConfirmed replies match its captured controls; canonical last_token is persisted and invitation stays pending. `every_advertised_capability_has_a_handler` calls this real route probe for V2. The stable advertised list now expects the activation addition; production list stays untouched.

System/User positives additionally create a real ordinary message through public CreateThread, Join and SendMessage. The emitted ordinary body retains its real pending ACK obligation; presentation never accepts the separate pending invitation. Loss/write tests deliberately compare durable nonordinary replay rather than falsely requiring ordinary message bodies inside the durable presentation outbox.

## Mechanism and source mapping

The hook fixture uses existing `continuity::Fixture` (`tests/hook_entrypoint.rs`), whose daemon runs actual `app::run_elected`, production DomainService, DeliveryPullState and registry composition. It does not reuse the older manually composed AGY Fixture as active evidence. Existing installed-argv contract, native payload shape and owned spawn conventions are reused. Both AGY and sender Claude registered child commands set isolated HOME and CLAUDE_CONFIG_DIR. A private measured `bin/agy` file establishes executable identity only; it is not executed or claimed to be native AGY.

The test-local observer renames only this fixture's private elected socket, binds the validated stable private pathname, and updates its test-owned endpoint descriptor's socket device/inode. It forwards exact request/response frames in positives and records the actual Commands/Results; it preserves canonical instance and boot fences. On Drop it joins its own observer thread and restores socket/descriptor before the existing elected fixture stops its daemon. Negative modes only omit/refuse capability discovery or drop an exchange; they synthesize no active capability, page, token or successful completion.

Frozen production input SHA-256 mapping (base4cb8aafe; parent must remap after separate corrections):

| Path | SHA-256 |
| --- | --- |
| `src/app.rs` | `df993fea67fdec3ba3590fa7f61a42bd22051ce12dfd3ad17ca19622ae137d61` |
| `src/cli/hook.rs` | `e9081b6af2078197d9ab93e615aa84617d1a4730cd3af9202ef1eb61f132469d` |
| `src/harness/agy/delivery.rs` | `6d9ece730b07273d57577b201a41f40e614d910da060de61066d3021198c8089` |
| `src/service/delivery_presentation.rs` | `3f4b8a06744f58895736c6c70d1eb746a4778732947756df17426c587c940242` |
| `src/protocol/capabilities.rs` | `1ed0b7340a421d8abfb28c8842ec5c1305fea16a060774d0b1cd8afd2f144f9d` |

Production path under test: actual ControlService Capabilities -> LocalSocketClient discovery -> registered AGY Step::pre -> configured DomainService PullV2 / registered adapter codec -> child stdout write + flush -> production HookStepCommit / complete_step -> original boot-fenced link -> canonical Confirm. No production diagnostic API was added.

## Actual evidence versus future commands

Actual evidence gathered: read AGENTS/TRUST/brief/proposal/review, inspected production and existing tests, wrote source-only tests, ran `git diff --check` (exit 0), recorded base/checkpoint SHAs and production fingerprints. No tests were executed, no assertion was observed passing or failing, and compilation/format/lint correctness is unverified. No helper-created process is live because no fixture or child has been executed. No Herdr write, tracker mutation, merge, push, stash, real-config write or subagent was used.

Parent command inventory (future only; use the existing shared absolute CARGO_TARGET_DIR/CARGO_BUILD_JOBS=4, nice, locked/all-features lane):

1. List `cargo test --locked --all-features --test hook_entrypoint agy::activation:: -- --list` and require all five exact names above, and list the exact library route probe before running it.
2. On dormant source, run selected actual capability assertions to observe intended red (advertisement omitted), then activate only after corrected cumulative source clears. A compile error is not the expected red.
3. On active corrected source, run `cargo test --locked --all-features --test hook_entrypoint agy::activation:: -- --test-threads=1` and `cargo test --locked --all-features --lib protocol::capabilities::tests::presentation_capability_routes_use_configured_domain -- --exact`; run the capability module and affected existing AGY seam selections.
4. Retain existing unconditional `harness::agy::delivery::tests::historical_system_message_scripted_page_preserves_native_bytes_and_oversize_geometry`, `...historical_downgraded_user_message_scripted_page_preserves_native_bytes_and_oversize_geometry` and `...output_shapes_are_byte_locked`. Existing canonical System/User allocations remain canonical codec seam coverage with hard-coded capability, not elected process proof.
5. Relevant existing negatives remain authoritative targeted seam coverage: `...postflush_confirmation_loss_keeps_state_replayable_after_restart`, `...stale_confirmation_and_expired_completion_never_save_injected_state`, `...invalid_acknowledged_control_subset_never_saves_hints_or_opens_probation`, `...completion_applies_only_the_daemon_acknowledged_captured_control_subset`, plus neutral adapter write/flush and canonical stale-owner/foreign-control tests. New process tests cover the missing actual installed bridge boundaries without replacing those controls.
6. Format, strict all-target/all-feature clippy, harness-neutral check and premerge default-feature check remain parent-owned. For any focused process sweep export a fresh HT_LEAK_RUN_ID, run scoped leak checks and stop only owned leaked children. No routine full suite.

## Remaining gaps and concerns

- Parent must compile/format/run every named test; the helper had no lane grant. Rust type-checking, fixture assumptions, timings, red/green and cleanup behavior are unexecuted.
- Separate private-loader and ordinary-offer corrections are outside this helper base. The helper's ordinary body assertion specifically requires actual active composition; any newly exposed source defect must return to independent source review.
- Stale owner, expired completion and invalid acknowledged subsets use retained adequate seam/canonical tests; this helper does not add redundant native process runners for those cases.
- Actual failed stdout exercises a real emission failure. Flush-specific rejection remains the existing neutral production emission helper control; no claim of an independently failed kernel flush is made.
- Native AGY codec uptake, native model uptake and external live Claude/plugin execution remain outside this registered bridge evidence. Parent owns required plugin execution and its truthful refusal reporting.
- Tests add no production model-visible strings. Test payload/diagnostic strings exist only inside isolated fixtures.

## Parent compile feedback correction

Parent compiled transferred `ef66bcdb` / `88240e90` onto D `df0463d7`; `/private/tmp/ht-finish-activation-preparation-red.log` reports E0277 at the origin binding query: rusqlite does not implement `FromSql` for `u64`. This is a compilation failure, **not** the intended inactive runtime RED.

Correction commit: `40ec88eb` (`test: decode canonical hook binding generation through i64`). The single changed expression reads the canonical SQLite generation as `i64`, then checks `u64::try_from(...)` with a fixture assertion that the canonical generation is nonnegative. It neither casts unchecked nor replaces the actual claim generation. Adjacent origin ID reads explicitly use String; the new pending/count reads use i64. No unrelated refactor or production change.

Helper validation: inspected the full parent compiler log and adjacent typed fields; `git diff --check` exited 0. Cargo, format and lint were forbidden for this correction and remain **NOT RUN** here. Parent recompilation and actual runtime inactive RED remain outstanding. No helper fixture, daemon or child process was started. Status: **COMPLETED QUIESCENT** (source correction only; no runtime success claim).

## Inactive-run diagnostic visibility correction

Parent reported application of `40ec88eb` as `54ae0dc7`, no formatting delta, then an actual inactive hook selection: compilation 6.49 s; runtime 3.846 s; five tests executed, historical omission/refusal PASS, four active tests FAIL; scoped leaked processes 0. The helper read `/private/tmp/ht-finish-hook-activation-inactive.log`. It contains no panic location/message for the four failed tests. **Their exact failure reasons are unproven; this report does not classify them as capability-assertion RED.**

Inspected diagnostic-loss path: `app::run_elected` uses `daemon::run_elected_with_diagnostics`; `ChildOwnedLogSink::install` in `src/daemon/mod.rs` duplicates/replaces both process fd 1 and fd 2 with its private diagnostic stream. `continuity::Fixture::spawn` installs only an `eprintln!` panic hook; the original Fixture::Drop joins the daemon and then removes its private root/log without showing it. `ChildOwnedLogSink::close` restores descriptors and joins/drains the diagnostic reader. `HT_MOD_DELIVERY_PANIC_LOG` is implemented by the separate mod_delivery Rig's panic hook, not this hook fixture, so that environment option alone cannot capture these hook failures. No existing equivalent hook capture environment was found.

Parent explicitly admitted `tests/hook_entrypoint.rs` for this diagnostic-only correction. Commit `33cad2b1` (`test: replay elected hook failure log after stdio restoration`) changes only `continuity::Fixture::Drop`: existing stop/join first restores stdio and drains diagnostic output; on unwind only, read the retained log and fallibly write its last at-most-8192 raw bytes to restored stderr, then retain existing private-root cleanup. Maximum added diagnostic output is **8218 bytes** (25-byte `elected daemon.log tail:\n` prefix + 8192-byte tail + one newline). Raw-byte writes avoid lossy UTF-8 expansion; write errors are ignored, preserving the original panic without a second panic. Passing tests get no additional output. The commit's truthful `Outside-adapter` trailer names `tests/hook_entrypoint.rs`.

This is a proposed visibility correction supported by source inspection, not a verified explanation of the four assertions. Parent must rerun and inspect the newly exposed exact location/message before treating failures as meaningful inactive runtime RED. Helper ran only `git diff --check` (exit 0) and read source/logs; no Cargo, format, lint, native or fixture execution. No processes were started. Source handoff status: **COMPLETED QUIESCENT**.

## First active run: discovery failure investigation and diagnostic checkpoint

Inspected actual parent log `/private/tmp/ht-finish-first-active.log` for active D `7c3edc2d5d6131258d4f0dfca7e1788ecbe3a4e4`: registered AGY System round trip and lost-completion recovery PASS; configured canonical presentation route and every-advertised handler probe PASS. Failed-stdout and User fail the `actual_page` assertion that this child's observer slice contains a V2-advertising actual Capabilities reply; historical omission/refusal fails the assertion that its slice contains any Capabilities command. These failures alone do **not** establish absent advertisement or a client cache defect. The active parent log has no same-child exchange summary, modes or child stderr at those assertions.

Static source finding for User/failed-stdout, still awaiting observed-mode reproduction: `Elected::downgrade` uses `fs::create_dir_all` for missing `adapter-state/agy`, which follows ordinary process permissions and can create both levels as 0755. Production `cli::hook::adapter_state_dir(create=true)` calls `daemon::paths::ensure_private_dir` on each existing level; `check_private_dir` requires exact 0700. Failed private-directory admission supplies state_dir=None; AGY `run_step_with_completion` immediately returns empty before Step::pre_invocation capability discovery. System and lost-completion tests do not seed this tree and allow production to create private directories. This source path plausibly explains the two downgrade-specific failures but actual modes/child traces must establish it; no fixture mode fix was made yet.

The fallback case does not call downgrade and remains independently unresolved. Each `Elected::run` spawns a new installed built-binary child; `LocalSocketClient` caches capabilities in its own per-instance OnceLock, not a cross-process cache. The observer is installed before the child and its take() slices are collected after child wait; no existing-source justification was found to relax same-child capability evidence or substitute another process's discovery.

Parent explicitly approved diagnostic checkpoint first. Commit `6ffe6d92` (`test: expose bounded registered hook discovery failure evidence`) changes `tests/hook_entrypoint/agy.rs` only. Existing capability, selected V2 page, native codec, flush-coupled completion/token, replay/hint/probation and historical no-V2 assertions are preserved. At existing capability-observation assertion failure only, a test helper reports actual parent/AGY directory modes or metadata error, same child's exit status/stdout byte count/elapsed time/last 512 stderr bytes and first eight observed command/result summaries plus total count. It includes neither message bodies nor full claims. The appended evidence is UTF-8-boundary truncated to **3072 bytes maximum** (plus existing assertion prefix; fallback includes bounded Fault enum spelling). No extra passing-test output or production text. Directory reads are fallible; there is no new assertion/panic in diagnostic formatting.

Helper inspected production active source and owned branch, read systematic-debugging instructions and actual parent log, and ran `git diff --check` (exit 0). **No Cargo/fmt/lint/native/runtime execution, directory mode correction, D mutation, tracker/Herdr/config mutation or process creation.** Parent should apply diagnostic checkpoint, reproduce User/failed-stdout/fallback, and return exact modes, child result and exchange summaries before fixture corrections. Source handoff: **COMPLETED QUIESCENT**.

## Observed private-directory fixture defect and narrow correction

Parent applied diagnostic checkpoint `6ffe6d92` as `83e90c52`. Actual `/private/tmp/ht-finish-hook-active-diag.log` compiled in 2.77 s and ran three selected cases in 4.025 s: User and failed-stdout failed; historical omission/refusal passed. Both failed children reported `adapter_modes(parent=0755,agy=0755)`, exit code 0, zero stdout bytes and zero observed exchanges. This establishes the downgrade-seeding defect: `create_dir_all` created nonprivate directories, production `adapter_state_dir` rejected them via exact-0700 private-directory admission, and AGY returned empty before capability discovery. The failure does not indicate missing actual daemon advertisement.

Correction commit **`1888adbb`**, `test: seed registered hook downgrade state privately`, changes only `tests/hook_entrypoint/agy.rs`. `Elected::downgrade` creates each owned level (`adapter-state`, then `agy`) with `DirBuilder::mode(0o700)`, accepts only AlreadyExists when creation fails, and asserts via symlink metadata that each is a real directory with exact 0700 permissions before writing the seed. Existing unsafe directories fail at the seed boundary; the fixture neither chmods them nor changes production permission checks. All registered-child capability, output-codec, flush-coupled completion/token, hints/probation, negative controls and bounded diagnostics are retained. No new product or passing-test output; only bounded failure assertion text for two fixture paths.

Fallback remains **unreproduced and unresolved**, separate from the proven downgrade defect. Parent reports both fallback iterations passed in the diagnostic selection and five additional exact isolated attempts passed with fresh tags and successful scoped leak checks (`/private/tmp/ht-finish-fallback-diag-{1..5}.log`). This is evidence of those passes, not a repair claim for the first active-run failure. Static source permits empty-before-discovery on missing state/payload, missing pane-context-derived caller claim, failed daemon connection, or exhausted deadline; `pane_context_journal` has bounded per-entry opening and ambiguity rejection. The fallback fixture does not seed downgrade directories, and each invocation is a fresh child with per-client capability cache. No source or runtime evidence selects one of those fail-open paths for its earlier failure, so no fallback behavior or proof assertion is changed. Retained bounded diagnostics will distinguish modes/status/output/exchanges if it recurs; no further repeat execution is justified absent new failure.

Helper validation: `git diff HEAD^ HEAD --check` exited 0, reviewed single-path commit/trailer and clean owned worktree. **Cargo/fmt/lint/native/runtime were NOT RUN here** under parent lane ownership. Parent's next actual full five-case active hook selection remains required to validate this correction. No helper child, daemon or other process was started; no D/Herdr/tracker/config mutation. **COMPLETED QUIESCENT**, source checkpoint ready for parent application and execution.
