# Capability advertisement activation — source-only handoff

Status: COMPLETE + QUIESCENT for this narrow source task. Actual active-path validation is NOT RUN by this helper and remains parent-owned.

Helper branch: `helper/handoff-finish-advertisement`.
Base: `31e5879781904b9ec20ee5ea3cec7517856260f3`.
Commit: `9f2ee80f5d997906dd8cd5b452c3eb3fa4ce4abe`.
Only committed path: `src/protocol/capabilities.rs`.
Trailer: `Outside-adapter: src/protocol/capabilities.rs`.

Parent explicitly released the read-only hold after reporting inactive cumulative whole/schema/interface production audits Spec PASS / Quality PASS and independent I4 correction reviews ADDRESSED Spec PASS / Quality PASS. Parent's mapped successor is `1cf377e9a890fedccbe5b070eacac6f6928e8444`, tree `b5e5a19727a4997b79b161123e8aac7cc19fc376`; the parent reports its only difference from the helper base is the Watch test correction. This helper did not independently execute those audits or import that unrelated test change.

## Exact change

Append `DELIVERY_PRESENTATION_V2` after `LAZY_HOOK_DELIVERY` in the actual `ADVERTISED` slice. Replace the stale `absent from ALL` comment with:

```rust
/// Bounded ordered delivery through registered codecs and explicit presentation confirmation.
/// Consumers retain historical delivery when capability discovery omits this name.
```

Prepared `tests/protocol/capabilities.rs` is unchanged: stable V2 spelling, expected advertised list, configured-domain real Pull/Confirm probe, and the `every_advertised_capability_has_a_handler` probe arm already exist. No compatibility omission test, wire shape, migration, trust invariant, core harness branch, codec, or consumer implementation changed.

The existing discovery name is 24 UTF-8 bytes; adding it to the JSON capability list adds 27 bytes including its quotes and separator. No per-message framing or wake text changed.

## Fresh source-only verification

- Before staging, `git diff --check`: exit 0.
- Exact diff inspected: one source file, one advertised entry plus the comment replacement.
- After commit, `git diff HEAD^ HEAD --check`: exit 0.
- `git diff HEAD^ HEAD --name-only`: exactly `src/protocol/capabilities.rs`.
- `git show --format=full --stat HEAD`: required trailer present; 1 file, 3 insertions, 1 deletion.
- `git status --short --branch`: clean `helper/handoff-finish-advertisement`.

Parent-provided RED logs were read directly before implementation: `/private/tmp/ht-finish-hook-activation-inactive-diagnostic.log` contains four actual elected AGY failures at missing advertisement and one historical omission/refusal PASS; `/private/tmp/ht-finish-watch-actual-inactive.log` contains five actual elected Watch failures at missing advertisement. These demonstrate the intended inactive failure but are not active GREEN evidence.

## Parent-owned validation selection — NOT RUN here

Expected exact capability filters, each statically present in the unchanged test source:

```text
protocol::capabilities::tests::capability_constants_are_stable
protocol::capabilities::tests::every_advertised_capability_has_a_handler
protocol::capabilities::tests::presentation_capability_routes_use_configured_domain
```

Discover nonzero test matches before executing these exact filters with locked/all-feature lib tests. Also execute the prepared elected AGY activation selection, elected Watch activation selection, unchanged historical omission controls, and affected existing consumer selections under the parent's sole authorized warmed Cargo lane. Required format, strict clippy, harness-neutral, default-feature, active-tip source/schema/interface review, scoped leak checks and production mod consumer execution remain parent-owned.

This helper ran no Cargo, fmt, lint, native harness/plugin runner, target preparation, Herdr command, tracker mutation, or real user configuration write. It started no daemon, helper process or test child and leaves none running. The run-local report is outside the source commit and is for parent promotion. No active-path PASS, downstream harness acceptance, or model uptake is claimed.

## Tests-only follow-up after the first actual active run

Follow-up commit: `5535b6ffb041262caa0146739852a58dbcdb6fe2`, parent `9f2ee80f5d997906dd8cd5b452c3eb3fa4ce4abe`, same helper branch. Only changed path: `tests/protocol/capabilities.rs`; exactly three insertions in `bare_daemon_advertises_legacy_capabilities_without_v2_recorder`. Trailer: `Outside-adapter: tests/protocol/capabilities.rs`.

Parent explicitly admitted this tests-only follow-up after its first actual active run. The actual log `/private/tmp/ht-finish-first-active.log` was read directly: nextest run `232a3171-f0e4-4cbb-92d8-96fd70b9a548`, 32 tests, 23 PASS / 9 FAIL, 42.445s execution. The configured production presentation route, every-advertised handler probe, and stable capability spelling/list passed in that parent run. This helper does not claim the entire active path passed; other Hook/Watch findings are outside this ownership.

The exact RED is `protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder`, failure at line554, 0.023s nextest duration. The actual 23-name response contains `seat.wake_on_v1`, `delivery.pull_v1` and `delivery.presentation_v2`; its old literal expectation omitted those three names. Source inspection traces the response to `ControlService::handle_with_output(Command::Capabilities)`: its bare fixture has no evidence recorder or health provider, so it filters only `HARNESS_EVIDENCE_V2` and `HARNESS_HEALTH_V2`, retains the advertised wake/pull/presentation names, and appends invitation confirmation. Bootstrap and automatic additions stay disabled in that fixture.

The correction adds the three missing strings in their exact handler order. All explicit no-recorder evidence/health negatives, bootstrap runtime exclusion, nonexistent capability assertion, and old-peer close/error controls remain unchanged. No production change was made. No new production/model-visible text is introduced (0 bytes).

Fresh source-only evidence: a read-only Python comparison resolved the actual advertised constants, removed only the two provider-gated names, appended invitation confirmation, and confirmed the corrected literal expected list equals both that 23-name contract and the actual RED response. It also checked that the evidence, health and bootstrap negative assertions remain present. This is source/log comparison, not Rust test execution. Pre-commit `git diff --check` and post-commit `git diff HEAD^ HEAD --check` exited0; exact diff/stat/trailer inspected; helper worktree clean after commit.

Parent-owned regression selection, NOT RUN by helper:

```text
protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder
protocol::capabilities::tests::
```

Require a nonzero exact test match and execute the full capability module as the affected selection in the sole authorized parent Cargo lane. No Cargo/fmt/lint/native/subagent/Herdr/tracker/config activity, D checkout edit, target creation or child process occurred in this follow-up. COMPLETE + QUIESCENT for source-only follow-up; parent GREEN/review remains required.
