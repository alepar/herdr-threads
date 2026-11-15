# Task 2 review

## Spec Compliance

- Functional requirements are implemented in the reviewed paths; the required trust amendment has a placement/formatting defect (M1). Reviewed frozen base `aa744e7a4` through head `ca4474c06ae263c8bc253aea623c029ba0a31801`.
- The three approved neutral seams are present, with no fourth interface seam or production pi-name branch in neutral core: selected-process validation (`src/harness/adapter.rs:330`, `src/harness/caller_env.rs:253`); fallible evidence with `Ok(default)` and propagation before orchestration (`src/harness/adapter.rs:238`, `src/cli/hook.rs:3820`); channel-owned output defaulting false (`src/harness/adapter.rs:2076`). Existing adapter literals retain None/false, and agy wraps its existing evidence in Ok.
- Cannot verify here: actual native TUI process shapes, installed native-loader execution, real reload/resume/fork, draft/modal safety, credentialed provider wake, shared herdr-core launch composition, or final integrated-suite qualification. These remain Task 3 gates; this review does not qualify or approve release.

## Strengths

- Process ownership checks bind the claim to the already selected actual ancestor, with pi leaving `pid_var` unset. Payload/session/role/mode/PID agreement fails before SessionEnd can disconnect anything (`src/harness/pi/mod.rs:46`, `src/harness/pi/witness.rs:56`, `src/harness/pi/witness.rs:79`). Causal process-table and refused-SessionEnd tests exercise the important distinction between classification and claimed identity (`tests/harness/pi.rs:115`, `tests/cli/hook_local_evidence.rs:907`).
- Mail suppression happens before canonical hydration and notice settlement, with warning frontiers also protected (`src/store/mod.rs:2539`, `src/store/mod.rs:2600`, `src/store/seats/cooperative.rs:1192`). The socket-backed hook regression and SQLite state snapshot check useful behavioral effects, including repeated Current/lifecycle calls (`tests/cli/hook_pull_lazy.rs:416`, `tests/store/notice_replay.rs:716`).
- Refusal normalization preserves valid empty success and default-adapter quiet refusal, while giving channel-owned callers a bounded ObserverOnly sentinel with completion evidence cleared (`src/cli/hook.rs:3996`). Both refined-decode entry paths, the foreign-site exit and watchdog have the policy applied (`src/cli/hook.rs:3778`, `src/cli/hook.rs:5089`, `src/cli/hook.rs:5230`, `src/cli/hook.rs:5308`).
- Pi declares channel-only wake, no composer, and no hook turn-delivery fallback; the extension source is embedded directly. Launch uses a closed grammar, rejects unknown/secret option forms without echoing values, preserves a single startup argument, and maps explicit profiles to the agent directory (`src/harness/pi/delivery.rs:7`, `src/harness/pi/mod.rs:221`, `src/harness/pi/setup.rs:16`, `src/harness/pi/launch.rs:22`, `src/harness/pi/launch.rs:94`).

## Issues

### Critical

None found.

### Important

**I1 — The new setup module copies an existing ownership state machine instead of sharing it.** `src/harness/pi/setup.rs:112`, `src/harness/pi/setup.rs:151`, `src/harness/pi/setup.rs:225`, `src/harness/pi/setup.rs:255`, and `src/harness/pi/setup.rs:377` reproduce opencode's manifest-entry ownership, bounded reads, manifest read/write, directory creation, install planning/pending-digest recovery, and removal logic (`src/harness/opencode/setup.rs:273`, `src/harness/opencode/setup.rs:314`, `src/harness/opencode/setup.rs:399`, `src/harness/opencode/setup.rs:429`, `src/harness/opencode/setup.rs:552`). The read/directory/removal functions contain verbatim logic blocks; install differs principally in configured directories and omission of opencode permission state. This adds hundreds of lines and a second independently maintained implementation of crash recovery and foreign/modified-file preservation. Future correctness fixes must be duplicated across adapters.

Extract narrowly shared digest-owned-file operations into neutral setup support, keeping adapter rendering, paths, diagnostics and permission behavior local. Preserve the existing opencode manifest schema/behavior and pi's two-file scope. Coordinate that scope adjustment before editing shared files; no additional adapter policy interface is inherently required. This is an Important maintainability defect under the task-review rubric, not a claim that the current preservation tests fail.

### Minor

**M1 — The pi trust section was inserted inside an existing inline section reference.** `TRUST-POLICY.md:334` now ends the continuity-key paragraph with an open backtick followed by `## Pi native extension ...`; `TRUST-POLICY.md:396` contains the displaced `## Accepted limits`).` suffix. The pi title is not a heading, and a malformed Accepted limits heading interrupts C1 before its remaining continuity prose. Restore the original inline reference and place the pi amendment at a proper section boundary with its own heading. The amendment's substantive cooperative-evidence and accepted-limit text is otherwise present.

## Checks and evidence

- Read the supplied diff package in sequential chunks; recovered tool-truncated passages separately. Used diff metadata for changed-file line references. No git commands, tests, builds, source/index/branch changes, threads writes, configuration or credential access. Only this review report was written.
- Inspected the complete task report's final validation claims: 1,232 focused Rust tests, 37 extension tests, clippy, format, default features, neutral/wire guards and scoped leak checks passed on the stated HEAD. Earlier compile/test failures have subsequent stated passing results. No suite was rerun and these reported results are not presented as independently executed checks.
- Named risk: validator refusal might fall back outward. Checked the cut-off process-walk context in `src/harness/caller_env.rs:176`: walk selects one ancestor, then the new fingerprint validation returns an error; it does not resume the walk.
- Named risk: channel suppression might leave settlement/presentation work in omitted surrounding logic. Checked cut-off lifecycle/orchestration context in `src/cli/hook.rs:3420` and `src/cli/hook.rs:4279`; fixed guidance and routing remain composed, while ObserverOnly output supplies no delivery completion. Checked the retained-result callback at `src/store/seats/cooperative.rs:1681`: it changes current/historical disposition only, with no new settlement/frontier work.
- Named risk: setup code duplicates ownership machinery. Compared the bounded opencode setup implementation at `src/harness/opencode/setup.rs:273`; this confirms I1 without a broader repository crawl.
- Named risk: launch grammar or destination diverges from native contracts. Compared the recorded native `dist/cli/args.js:14` parser under the research probe's dependency tree: supported arities, thinking values and `--` startup handling agree. The recorded extension-location evidence (`research/evidence.jsonl:9` in the pi run directory) explicitly includes `extensions/*/index.js`. Checked the unchanged extension command/payload construction at `integrations/pi/extension/herdr-threads.js:63` and `:76`; Rust's setup prefix and payload contract agree. This is source/contract inspection, not native-TUI qualification.
- Core-change accounting agrees with the report: adapter/registry/caller-env changes implement the three seams; existing adapter changes preserve defaults; hook/bridge/store/cooperative changes implement channel ownership and refusal; module/unsupported registration enables pi; native-host changes are test-fixture substitutions; help changes spell out process IDs to avoid the scanner false positive. No additional production behavior was found in those files outside this scope.

## Assessment

**Task quality: Needs fixes.** The process/evidence and canonical mail-suppression seams are carefully bounded and supported by causal tests. The duplicated ownership engine is substantial maintenance debt introduced by this task; repair it and the malformed normative-document insertion before approval.
