# C elected Hook startup diagnostic author handoff

Status: **COMMIT QUIESCENT**. Source is frozen for parent transfer and the exact failed-case diagnostic run. No Cargo, build, test, Herdr CLI, daemon/background helper or subagent was run by this author. This is diagnostic instrumentation, not a behavioral fix, runtime pass or explanation of the failure.

- Checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/finish-composed-hook`
- Branch: `helper/handoff-composed-public-ensure`
- Immediate base: `cc686d29416522d1405e79df5c857c95438e819d`; base tree: `bd278a7c987e13b6a0436495b2e2d0a9cc089060`
- Diagnostic commit: `f8073334f2fb9227a1ada322c2f5927b2cebad37`
- Tree: `720d79ee8e625bb32f3684361c7255de5932e7bd`
- Only changed path in this commit: `tests/hook_entrypoint/agy.rs` (+136/-6).
- Source blob: `d97f882cb3c6086f71760e9b94a4fa4cf3cfdf1a`
- Source SHA256: `ed404b59c821f22bb65665456821d6da65062d36d3ae33e3843c3113e06dedcd`
- `Elected::start` declaration: line821; startup snapshot and missing-CheckedIn assertion: lines856–869; new pure diagnostic helper: lines1097–1216.
- Exact failed-case selector remains `hook_entrypoint` / `agy::activation::activation_v2_registered_agy_lost_completion_preserves_exact_recovery` (declaration now1895).
- Trailer: `Outside-adapter: tests/hook_entrypoint/agy.rs`
- New production model-visible text: **0 bytes**. New test failure diagnostic including the assertion prefix: at most **6 KiB**, emitted only on missing canonical CheckedIn.

Read all of `resume-C-hook-startup-triage.md` and the actual saved failure at `/private/tmp/ht-C-composed-b046c123/nextest-run.log:94–112`. The observed failure establishes code0, nonempty startup output and no successful typed CheckedIn in the snapshot; it does not establish why startup failed. The actual run's record is historical evidence for source `b046c123`, not execution of this changed source. Used systematic-debugging guidance to gather the missing evidence without a semantic repair.

Implementation retains the existing single `observer.take()` at exactly the same stage, inspects its vector by reference, and clones only a genuine CheckedIn context when present. Existing code/empty-output assertions, Fault::None startup observer, registered command, invocation, budgets and teardown remain unchanged. The missing-CheckedIn panic alone now calls the diagnostic helper. No retry, extra check-in, synthetic claim, fallback-positive oracle, sleep, timeout, synchronization or caller API/call-site change was added.

Diagnostic contents and bounds:

- Escaped isolated SessionStart stdout and stderr, each at most1536 encoded bytes, after lossy UTF-8 conversion and exact fixture-root replacement with `<fixture>`. Each snippet reports omitted redacted-text byte count, including zero when complete. Snippet accounting includes quotes and truncation annotation.
- At most12 indexed exchanges: all when at most12; otherwise first6 plus last6. The record includes total and omitted counts. Typed startup/delivery command/result variants are projected to short static tags; unrelated/unmapped variants appear as `other`. Errors show only ErrorCode; no full API detail or payload is dumped. No request Debug, caller claims, contexts, operation keys, tokens, messages, transcript/journal/database contents, environment or full argv appears.
- Setup stage `Elected::start/SessionStart`, actual startup fault `None`, code, elapsed milliseconds, original captured stdout byte count, captured stderr UTF-8 byte count, tracked source caller and bounded escaped current test-thread name. Iteration is explicitly `unrecorded`, because retaining the existing zero-argument setup call and callers cannot establish which later-fault loop iteration is active. No attribution to DropConfirm/LoseConfirmReply is guessed.
- The helper caps its whole record to6016 bytes, reserving128 bytes for the existing assertion prefix under6KiB. If final framing grows past this bound, it keeps a UTF-8-boundary suffix plus an omitted-leading-byte annotation, retaining stage and final exchanges toward the end. Original full trace count remains in the pre-cap record.

One API limit is reported honestly: `Hook.stderr` is already a lossy UTF-8 String, so its raw pre-conversion byte length is unavailable from the authorized captured values. The diagnostic explicitly labels `stderr_captured_utf8_bytes`; it does not claim original raw stderr length or change the shared Hook struct/invokers. Stdout remains a raw Vec and its byte count is exact. Existing separate code/empty-output assertion stderr behavior was retained; the new6KiB bound applies to the missing-CheckedIn record.

Static author checks: direct rustfmt format and `--check` exit0; precommit diff check exit0; postcommit diff check exit0; postcommit status empty. Inspected the source diff and static budget arithmetic. No standalone diagnostic test or runtime evidence was generated; parent remains sole Cargo/execution lane and owns source gates, leak proof and causal diagnosis. Transfer only `f8073334f2fb9227a1ada322c2f5927b2cebad37` after the separately reviewed A7 correction as planned.
