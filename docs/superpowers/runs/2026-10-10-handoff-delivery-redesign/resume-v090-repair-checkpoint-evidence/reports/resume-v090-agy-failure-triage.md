# Registered AGY / native-codec triage (actual 657 selection)

Read-only inspection of the supplied v090 composition and `/private/tmp/ht-C-v090-final-composed-657/nextest-run.log`. No Cargo, tests, Git, Herdr commands, service calls, source edits or process launches were performed. This report is the only written artifact. TRUST-POLICY.md A2/A3 and the physical-presentation contract were consulted. Supplied revision labels are parent-provided, not independently checked with Git.

## Six registered-hook failures: confirmed stale process fixture

The failing tests are the system/user round trips, failed stdout replay, lost completion recovery, historical-capability fallback, and modern direct-Ensure lazy pair. Each recorded failure is `herdr-threads hook: no agy, claude, codex, hermes or other registered agent harness process among this process's ancestors`. The reported lines 1340/1435/1861/1925/1977 are calls to `Elected::start()`; track_caller attribution does not mean delivery reached those test bodies.

`tests/hook_entrypoint/agy.rs:968` implements a private `Elected::run_process`, still starting `scrubbed_command("/bin/sh")` with the registered command as `-c`. It supplies neither a harness ancestor nor a test harness stand-in. The fake `bin/agy` written at line 859 is an executable-identity fixture used by `resolve_identity`; putting it on PATH does not make it an ancestor. Its `exit 1` body is intentionally not a native model invocation.

The current production path requires `hook_fingerprint(event.harness)` in `src/cli/hook.rs:2582` before constructing the lifecycle session journal. Non-test binary code at line 1922 uses `caller_env::harness_fingerprint`, which calls `walk(inputs, Some(harness))`. `src/harness/caller_env.rs:170` walks actual parent records, respecting the test ancestry floor; PATH is not used. AGY declares process names `agy`, `antigravity`, `antigravity-cli` in `src/harness/agy/mod.rs:78`. The failure is therefore expected canonical liveness admission, before the intended CheckIn, V2 Pull, native encoding or Confirm assertions.

The current shared helper already demonstrates the migration: `tests/hook_entrypoint.rs:522-535` runs installed commands beneath `test_support::harness_shim::under_harness`. `src/test_support/harness_shim.rs` uses a Bash parent with a harness argv0 and waits for its child. This is a test process fixture, not production ancestor guessing.

Proposed scoped repair: give the private process helper an explicit harness argument and use the existing shim around the unchanged registered shell command. AGY invoke passes `agy`; both sender SessionStart call sites pass `claude`. Keep environment scrubbing, isolated HOME/config/state, registered argv, real binary, real daemon, observer forwarding, exact stdout codecs, broken stdout FD, fault modes and production completion path. Do not set fabricated claims or loosen hook_fingerprint.

There is a lifecycle concern to verify after this first repair: a per-invocation shim exits with its child, so the daemon can mark its fingerprint disconnected; the canonical Hook reconnect path supports that, but stored claims used between hooks must not accidentally depend on a dead fixture ancestor. If a persistent synthetic session is needed, own it for the Elected lifetime using existing test ownership facilities and stop it on drop. Keep the sender and recipient sessions distinct. Do not create a real harness process or use the user's session/config.

## Additional direct-Ensure fixture mismatch behind the first failure

`agy.rs:1480` uses the shared in-process `cooperative()` helper. That helper now installs `agent_origin("claude", "sess-1")` at `tests/hook_entrypoint.rs:681`, with `LivenessFingerprint::TEST`. The test actually registers its sender through installed SessionStart as `claude/origin-session` (agy.rs:1450), captures the resulting claim, and later requires exact equality `ensure.claim == origin` and `send.claim == origin`. Those session identities cannot match. The current failure precedes this point, so this is a statically established latent fixture mismatch, not a separately observed runtime failure.

Repair the in-process invocation locally to select the captured canonical sender's harness/native session/role and honest fixture fingerprint. Preserve actual CLI parsing and modern Ensure/Send RPCs and all exact-result/no-settlement assertions. Avoid changing the global cooperative helper's fixed session for unrelated tests. If a native child CLI is chosen instead, it must share the sender fixture session/environment/process rather than borrowing the model's environment. Check exact claim equality with the canonical admission response after any liveness reconnect.

## Two actual-codec tests: recorded symptom is insufficient for a root-cause claim

`canonical_v2_system_allocated_pages_fit_actual_native_codec` and `canonical_v2_downgraded_user_allocated_pages_fit_actual_native_codec` both fail at `tests/harness/agy_delivery.rs:1882`, observed full invitations 0, expected 24. They are in-process DomainService tests and do not use Elected or process ancestry. Applying the hook shim cannot address them.

The fixture checks in the recipient, checks in a distinct origin, and produces 24 lazy invitations through real CreateThread/Invite calls before the failure. Those commands all unwrap, so their admission/producer stages succeeded in the recorded run. The loop invokes the real adapter and DomainService. However, `Canonical::call` at line 1738 uses `self.service.handle(...)?`; any actual service error propagates to `src/harness/agy/delivery.rs:1155`, whose wildcard returns empty stdout without setting a diagnostic. A V2 Refused reply similarly yields empty output without a diagnostic. The fixture captures only successful Page variants and silently continues when none is captured (line 1818). Consequently its `prepared.diagnostic.is_none()` assertion does not prove Pull admission/allocation succeeded, and 40 repeats of an error/refusal produce exactly the observed terminal 0/24 assertion.

No deciding error, refusal reason, page count or command trace is present in the supplied log. Static inspection therefore does not establish whether these failures are canonical admission, a store error, or successful pages without invitations. It would be incorrect to claim a codec, materialization, session or schema defect solely from this aggregate assertion. The pending 565 materialization repair is not evidence for these two tests: invitations use the trigger-maintained digest_pending_invitations projection, and this fixture is an in-process service without a daemon materialization loop.

Smallest next experiment, parent-controlled: in this fixture's Canonical::call, identify the command kind, assert the actual `service.handle` result is Ok with its ApiError on failure, and explicitly fail on V2 Refused with its reason. Record bounded Page metadata (item counts by kind/token presence/more, not payloads) if all requests succeed. Add an assertion that the canonical pending/digest invitation counts are 24 before first pull. Run only the two codec tests. This exposes the first failed boundary without changing production behavior, prepopulating offers, inventing claims or weakening any assertion. Parent authorization is needed for source changes under this read-only assignment; no user permission issue is implied.

## Validation that must remain intact

Retain actual registered-plugin stdout, exact SystemMessage/UserMessage shapes and byte equality, explicit physical Confirm token/control checks, no invitation acceptance/message ACK/read-cursor settlement, failed-output replay, missing/response-lost completion, and omitted/refused-capability historical V1 fallback. Do not reduce the 24 canonical escaped invitations or the aggregate output-size assertion merely to pass. Once diagnostics identify the codec cause, repair the fixture or production component at that boundary and rerun focused cases under the parent's sole-Cargo/process ownership. No verification success is claimed here.

## Authorized fixture/diagnostic author pass

Parent subsequently authorized edits to exactly `tests/hook_entrypoint/agy.rs` and `tests/harness/agy_delivery.rs`; no production changes. Both files are now quiescent for parent review/commit/testing.

- The private registered-hook launcher now accepts explicit `Harness` and calls the existing `under_harness` with the unchanged `/bin/sh -c <registered command>`, then scrubs environment and restores isolated fixture values. AGY calls pass their registered harness; sender SessionStart calls pass Claude. Spawn ownership, stdout-failure socket, native command construction and assertions remain intact.
- The direct-Ensure test now runs its actual in-process parsed CLI inside a scoped `fix_origin` derived from the captured installed sender CheckIn claim, including origin-session and its captured liveness fingerprint. The shared sess-1 helper is untouched. Exact Ensure/Send claim equality remains asserted.
- The two actual-codec tests now assert canonical pending/projected invitation counts `(24,24)` under their actual recipient/instance keys. Their Canonical boundary stops on the first actual service error or V2 refusal instead of allowing adapter quiet-output handling to hide it. Error detail is capped at 512 Unicode scalars (at most 2048 raw UTF-8 bytes, debug escaping can expand that); fixed enum/status/count framing is small. Only failure output includes diagnostics, never success-path model-visible text. A fixed counter summary records pulls/pages/unchanged/invitation items and last-page item count/token-presence/more. No body, token value, request or claim is printed. All existing output-size, native-codec, 24-full-invitation and no-settlement assertions remain.
- No production diagnosis/fix for the two codec failures was attempted. Per-invocation shim fingerprints remain a specific pending validation concern; exact claim equality has not been weakened to conceal it. Parent should run the focused tests before choosing a longer-lived session fixture.

Formatting: `rustfmt --edition 2024 --config skip_children=true` on only the two owned files, followed by `--check`, exited 0. No Cargo, Git, Herdr, test/process execution or source outside those two files was used. Runtime success remains unverified.

Quiescent SHA-256:

```
07dda0b3f6329f4e2c64acab15fc2f807d4d1be394d3289c0544aec8749d1c32  tests/hook_entrypoint/agy.rs
02749ef05d72ff253a96d2a983ff801a0a9461c1e650d1f2caa57cce1d16ea78  tests/harness/agy_delivery.rs
```

## First focused diagnostic result and exact codec fixture correction

Parent ran affected lib57 at actual94 (run UUID `cc503c42-9f70-4c13-81bd-804701732232`; `/private/tmp/ht-C-v090-repaired-lib57-first/run.log`), reporting 53 pass / 4 fail and no leaks. Both codec tests reached the new first-Pull diagnostic: `SessionMismatch; pulls=1,pages=0`, after canonical/projected invitation counts `(24,24)` passed. Registered-hook cases were not run in that batch.

The precise cause is shared synthetic process identity, not allocation or codec behavior. `claim()` at tests/harness/agy_delivery.rs:55 creates an AGY claim with `LivenessFingerprint::TEST` (pid 1, start 1). The origin was made from `claim(0)`, changing seat/target/session/execution but keeping the same harness and fingerprint. Its lifecycle CheckIn invokes `src/store/bindings.rs:759` `same_pid_supersede`: the query at lines 779-793 disconnects every other open connected top-level binding in the same **instance**, harness and process fingerprint, including another seat. Thus origin's CheckIn canonically disconnects the recipient as `Superseded`. `src/service/delivery_pull.rs:285` explicitly returns SessionMismatch for that state; a plain Hook pull cannot reconnect it. TRUST-POLICY A4 explicitly specifies this same-process rule. Neither session_execution nor a guessed generation correction is needed.

Authorized minimal correction: the codec fixture's origin now has a distinct synthetic fingerprint `{pid: 2, start: 1}`. This is appropriate for its existing in-process DomainService without a real-process liveness worker, matching the independent-session model used in store binding tests. A narrow precondition reads each actual canonical binding by seat/harness/native-session/agent/generation/execution and requires connected state with no disconnect reason after both CheckIns. All diagnostics and 24-invitation/output-codec/token/no-ACK assertions remain. No production file changed and no source redefinition of supersession was introduced.

Scoped rustfmt and `--check` passed again; no Cargo/Git/Herdr/runtime was run by this agent. Files are quiescent; runtime verification is still the parent's next step. Updated hashes (supersede the previous author-pass codec hash):

```
07dda0b3f6329f4e2c64acab15fc2f807d4d1be394d3289c0544aec8749d1c32  tests/hook_entrypoint/agy.rs
6cc369ff3d447b357ae911156b57cd5be98f97bd66773314c0523044c32ec192  tests/harness/agy_delivery.rs
```
