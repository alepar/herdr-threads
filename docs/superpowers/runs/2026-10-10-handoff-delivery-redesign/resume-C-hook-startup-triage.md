# C composed hook startup triage

Read-only diagnosis, 2026-10-11. No code fix, Cargo invocation, source/Git mutation, Herdr operation, or test process was performed. The only write is this report. Read systematic-debugging/SKILL.md and the applicable TRUST-POLICY startup enrollment, C1/C2, A2, attribution, local witness, and hook-output rules. Canonical CheckIn remains required; local output must not be substituted for its authority.

## Source and run identity

Candidate: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-478ec4a9`.

The preserved `/private/tmp/ht-C-composed-b046c123/execution.json` records clean source before/after execution at commit `b046c123be452957d3f7f10d8fbdc60b18a4533e`, tree `ecc9977a535a932b5514ab8820650d1c832691a5`; run ID `28b15c51-3941-483f-83b0-2ef768028f03`. The candidate files read for this investigation match the execution record's SHA256 values:

| Path | SHA256 |
| --- | --- |
| tests/hook_entrypoint/agy.rs | b7c16cdf2d80f11b71fc47378e915b683de8a9bd98d185518851cdd2824af2cd |
| tests/hook_entrypoint.rs | 9d89233c9c796506749e1cb5eadb3cf4806bea9fa6c6b440e6b68ad978b27000 |
| src/cli/hook.rs | e9081b6af2078197d9ab93e615aa84617d1a4730cd3af9202ef1eb61f132469d |

Actual C: 385 focused cases, 377 PASS / 8 FAIL; six mandatory cases pass; recorded scoped leak checks all exit zero. This report addresses only `hook_entrypoint / agy::activation::activation_v2_registered_agy_lost_completion_preserves_exact_recovery`.

The parent reports the unchanged test blob passed in the earlier 314-case D final run. That is comparison evidence, not a waiver of actual C. This investigation directly inspected actual C's log/record; it did not independently locate/revalidate the earlier D run artifact.

## Observed failure and exact limit of the evidence

`nextest-run.log:94-112`: FAIL at 1.371 s (libtest 1.34 s), panic `tests/hook_entrypoint/agy.rs:861`, `installed SessionStart reaches canonical CheckIn`. The replayed daemon log tail contains only `harness manifest: auto`, startup evidence counts (both zero), and this panic.

`Elected::start` at agy.rs:819-872:

1. Starts the private elected continuity fixture with two NoSession panes and waits for canonical reconciliation.
2. Creates a private agy home with one conversation, empty transcript, regular presence lock, and isolated executable identity.
3. Installs an `Observer` with `Fault::None`.
4. Invokes the actual installed SessionStart command through `/bin/sh`, supplying native JSON, temporary HOME and pane routing.
5. Successfully asserts exit code `Some(0)` and nonempty stdout.
6. Takes all completed observed exchanges, searches their results for `Ok(CommandResult::CheckedIn(_))`, and panics if absent.

Therefore the actual result proves: child exited zero, emitted at least one byte, and the observer snapshot contained no typed successful CheckedIn. It does **not** prove successful enrollment or even that a CheckIn command was submitted. Hook fail-open policy deliberately exits zero; top-level lifecycle service failures produce nonempty native unavailable context (`src/cli/hook.rs:3380-3425`). Local subagent lifecycle guidance can also be nonempty without a CheckIn (`:3245+`). Both are distinguishable from the captured stdout, but that output is discarded at this assertion.

The failing test loops over DropConfirm and LoseConfirmReply (`agy.rs:1765+`). In each iteration, `Elected::start()` completes before invite setup and installation of the faulted observer. Thus the failure precedes the **failing iteration's** fault injection and recovery assertions. The existing panic does not identify whether the first or second iteration failed; do not claim neither iteration ran.

The same actual C run passed other uses of this setup: failed_stdout recovery (4.430 s), system round trip (2.794 s), user round trip (1.846 s), modern direct ensure (2.172 s), and omitted/refused capability fallback (4.364 s). These show startup can succeed at this candidate, not why this invocation failed.

## Observer and process lifetime

`Observer::start` (`agy.rs:689-797`) renames the real listener to a private upstream pathname, binds the descriptor's stable pathname, updates device/inode, and forwards requests on a single worker. Fault::None changes no protocol bytes. A response is decoded and appended to `exchanges` **before** it is written to the child. Consequently, a successful CheckedIn response consumed by this child cannot merely be waiting for an after-write record insertion. The snapshot does not prove there are no in-flight requests: requests are recorded only after an upstream response is available. Failed client frame reads are silently skipped, and forward-write errors are ignored; those are explicit secondary evidence gaps.

On drop, Observer stops and joins the worker, removes the proxy socket, restores the upstream pathname and original endpoint descriptor. On unwinding from this startup assertion, it drops before the local fixture. The fixture stops/joins its private daemon, thereby restoring process stdio, replays the final 8 KiB of daemon.log, then deletes its private root (`tests/hook_entrypoint.rs:4881+`). This explains why the panic appears in the saved bounded tail and why child output/private files cannot be recovered from the surviving log. No observer-worker panic is present in the actual tail; no evidence supports asserting one occurred.

Process invocation (`agy.rs:942-988`) pipes stdin/stderr and, for this startup, stdout. It writes the small native payload then drops stdin, allowing the hook's read-to-end to finish; `wait_with_output` collects stdout/stderr. There is no explicit parent pipe timeout here. The production hook owns the wall-clock guard. The fixture's readiness channel has 5 s and reconciliation loop 15 s; neither corresponding assertion failed.

## Budgets read, without proposing any change

- Core production tool budget: 1500 ms; lifecycle: 5000 ms; watchdog margin: 150 ms (`src/cli/hook.rs:67-75`). Agy declares the same caps (`src/harness/agy/mod.rs:45`). The native plugin registration is SessionStart 10 s, PreInvocation 10 s, Stop 5 s (`plugin.rs:71-73`); this test invokes the installed command directly, without a native agy timer.
- The watchdog starts at the tool bound before filesystem/stdin work, then switches to the refined event bound, measured from the same start (`hook.rs:3548-3694`). Local witness refinement receives half the provisional lifecycle bound (`:2852-2890`). Unknown/ineligible roles can return quietly; subagent lifecycle context is distinct.
- Existing test support tags the child with timeout scale 10 unless explicitly overridden; this Elected path does not remove it (`src/test_support/spawn.rs:59-74`). The external wrapper therefore ordinarily uses 15 s initially / 50 s for lifecycle. `Hook.timeout_scale: None` means this helper did not record the effective setting, not that production bounds were selected. Do not change that setting. Some inner budgets remain independently bounded (for example ordinary bridge observed CheckIn span 5000 ms).
- Observer socket client read/write timeouts are 2 s each; upstream read 5 s and write 2 s (`agy.rs:723-748`). These per-I/O settings are not a full test or frame deadline. The production local client caps connection wait at min(remaining, 2 s), and fences writes/response reads with its call budget (`src/client/local.rs`).
- The entire failed case took 1.371 s, including fixture setup/shutdown. This does not establish any of the above multi-second deadlines expired, and specifically provides no evidence of exhausting the lifecycle cap. Shorter service/admission failures remain possible. No environmental or scheduler explanation is established.

## Hypothesis and smallest useful diagnostic

Working hypothesis, **unconfirmed**: SessionStart followed a fail-open startup path before obtaining a successful canonical CheckIn, and the test discarded the stage/reason already available in its child output and exchange results. A local role downgrade is an alternative distinguishable by the same evidence. We cannot name the refusing component or propose a behavioral repair yet.

Smallest first diagnostic is test-only in `Elected::start`: retain `startup` and the single `observer.take()` vector, inspect the vector by reference, and on missing CheckedIn panic with a compact diagnostic derived from those already captured values. Keep the same assertion, fault placement, snapshot timing, budgets, cleanup and production command. Do not replace startup with a synthetic canonical claim or call it again until successful.

Required failure-only record:

- Stage `SessionStart`, fault `None`, child code, elapsed milliseconds, stdout/stderr original byte lengths.
- Bounded escaped actual stdout and stderr snippets; at most 1536 encoded bytes each, with explicit truncation/omitted-byte indication. Replace the fixture root with `<fixture>` before rendering; never print environment, full argv, transcript content, database/journal contents, executable identity, or arbitrary request payloads. This isolated startup has no user-supplied thread/message bodies; do not extend raw-output logging to later message-delivery hooks.
- At most 12 indexed exchanges (first six + last six if longer), total count and omitted count. Each has command **variant only**, then result variant / `Err(ErrorCode)` / no response. Include at most 128 escaped bytes of an API error detail only if needed after the child stderr, with the same root redaction. No `Debug` dump of whole Command, CheckedIn context, caller claim, operation key, page, token or body. Use small test-local tag projection, not a new production protocol helper.
- Cap the entire emitted diagnostic at 6 KiB including escape expansion/framing, leaving space for the panic location in the existing 8 KiB daemon tail. Render only on failure and keep the decisive stage/error/last exchanges toward the end. No full log dump or additional logging daemon.

An optional tiny test-only label passed from the two-iteration test (`DropConfirm` / `LoseConfirmReply`, explicitly labeled as the **later fault**) identifies which setup failed. It must not change Observer::start(Fault::None). This is useful but not necessary to identify the startup failure stage.

Do not add sleeps, retry loops, serialization groups, budget increases, environment changes, or new observer synchronization as the diagnostic. A green rerun supplies no missing causal evidence and does not waive actual C. The parent owns any authorized diagnostic implementation and Cargo run.

Interpretation of a reproduced record:

1. `check-in unavailable` plus an error exchange localizes the canonical/service or transport boundary; inspect that exact stage/code.
2. Subagent guidance with no CheckIn localizes role refinement; inspect the already-created witness files and the refined role via a separate targeted diagnostic, preserving cooperative trust semantics.
3. No useful error exchange with child transport failure means the current observer's unrecorded in-flight/frame-error gap matters. Only then add a bounded test-only observer stage trace (command tag, stage, relative time, I/O ErrorKind) recorded before upstream forwarding and after response decode/write. No payloads or protocol changes.
4. If child output claims successful startup yet the vector has no CheckedIn, trace endpoint routing and result types with explicit evidence; the existing append-before-write ordering alone rules out the simple recording race.

Status: actual failure remains unresolved. The evidence gap and minimal diagnostic are concrete; no fix or timeout change is justified yet.
