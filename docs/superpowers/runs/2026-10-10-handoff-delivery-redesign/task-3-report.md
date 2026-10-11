# Task 3: Stable native event dispatch and command generation

Status: DONE_WITH_CONCERNS — behavior/format/lint checks passed; measured first-build and lint budget concern recorded below. Committed and clean: `70d8b778` (`feat(hooks): add stable registered native event entrypoints`).

Worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.4.1`
Branch: `task-ht-0lo.4.1`
Base: `37c894ab944c516007735c60285761d179c79a39` (own HEAD and clean status confirmed before edits).

## Preflight

Read task brief/execution amendment, settled root/hooks contracts, base-composition.md, fixture-preflight.md, repository guidelines and trust policy. Inspected real main interceptor, registered adapter contract/decoder path, setup ownership grammar and bundled mod route consumer before editing. `cargo metadata --no-deps --format-version 1` confirmed own workspace_root/private target directory; no shared target used. No native configuration, thread/inbox/ACK or shared Herdr actions performed.

## Observed RED

Command (own worktree, exact scoped CLI escalation for nice):

```sh
CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.4.1/target nice cargo test --locked --all-features --lib native_event_entrypoints_
```

Source provenance: unchanged production at base, only new tests/cli/hook.rs parser regression. Source frozen throughout compile. Cargo reported `Finished test profile ... in 7m 50s`; execution 0.00s. Result: 0 passed, 1 failed, 3595 filtered. Expected panic: `dedicated event must select hook entrypoint` at tests/cli/hook.rs:192. Existing parser returned None for hook-event, proving missing dispatch rather than a fixture/compiler failure. Released lane immediately after observed failure. Host contention/monolithic owned first test build exceeded the project incremental budget; coordinator confirmed prior similar first builds and retained unchanged profiles/timeouts. One authorized process inspection coincided with completion and produced no useful own-rustc progress measurement; no process stopped/restarted.

## Prepared implementation and seam

- `cli::hook::installed_event_argv(executable: &str, state_dir: Option<&str>, host_endpoint: Option<&str>, harness: Harness, event: &str) -> Result<Vec<String>, String>` produces exactly one complete registered harness/event argv. Routes supplied by setup must already be canonical; executable spelling stays exact.
- `harness::setup::native_event_command(event_argv: &[String]) -> Result<String, SetupError>` validates a complete dedicated event argv then shell-quotes exactly once. It rejects legacy/invalid pairs and adds no owner/random suffix.
- Example stable command: `'/opt/ht' '--state-dir' '/state' '--host-endpoint' '/host' 'hook-event' 'claude' 'SessionStart'`.
- Parser accepts stable hook-event HARNESS EVENT and existing hook HARNESS [--event EVENT]. Explicit pairs must exist in the registration's declared contract before the main interceptor calls the hook runner.
- Existing install ownership, manifest installation IDs and legacy event_command remain untouched for Task9.
- Approved scope expansion: src/harness/claude_mod.rs and tests/harness/claude_mod.rs. Its launch_prefix consumes known stable/evented-legacy/unevented-legacy Claude argv through the parser and returns only executable/global route for watch.
- Prepared built-binary fixture exercises all five native events, child malformed-payload codec refusal, and invalid-pair zero check-in/digest/parse-report counter changes. Uses existing isolated fixture and spawn_owned helpers.

## Pending verification

Focused unit parser/producer, legacy installed argv, mod, Hermes callback, actual native entrypoint, format and all-target/all-feature clippy. No completion or GREEN claim yet.

## First GREEN and compatibility adjustment

With coordinator-assigned jobs4, own private target, `nice cargo test --locked --all-features --lib native_event_`: 2/2 passed, build51.41s, execution0.00s,3596filtered. Production/source frozen from compilation start. Then `nice cargo test --locked --all-features --lib hook_argv_`: build0.04s, execution0.00s,1passed/1failed. Setup installed-hook argv roundtrip passed; old optional-event test unwrapped an arbitrary63-byte undeclared event. This is obsolete under explicit invalid-pair refusal; changed it to assert undeclared refusal while retaining valid Claude/Codex legacy forms and invalid syntax/bound checks. Preserved legacy usage `--event NAME` phrasing. Lane released immediately for this bounded fix.

## Legacy assertion contract trace (coordinator accepted; fresh review requested)

Coordinator challenged the long-name assertion change as possible legacy installation-identity compatibility. Checked the BASE producer, not an inferred new grammar: base src/harness/setup.rs:86-91 event_command emits literal `--event {event}`; base:703-716 owned_command emits installation UUID only as the separate `# herdr-threads-owner:{installation_id}` comment. Unmodified src/harness/claude.rs:721-730 declares SessionStart/PreToolUse; base setup.rs:327-336 uses codex DECLARATION hook.event. Unmodified Claude decoder src/harness/claude.rs:894-901 and Codex decoder src/harness/codex.rs:1403-1411 require registered_event==payload.hook_event_name. Unknown event strings were syntactically accepted but could never be a valid installed codec pair; they formerly reached daemon parse-report effects. Normative settled root design line48 expressly keeps legacy syntax and requires invalid pair refusal before daemon effects; line50 separately identifies UUID shell comments. Coordinator accepted the narrow assertion refinement on that concrete trace. Flag this assertion change for independent review. Valid legacy native events, unevented registrations and malformed/empty/63-vs64-byte syntax bounds remain covered; no identity token reinterpreted.

## Focused GREEN after reviewed legacy assertion refinement

All commands used own worktree, `CARGO_BUILD_JOBS=4`, absolute own private `CARGO_TARGET_DIR` recorded above, `nice cargo test --locked --all-features`, scoped CLI escalation; executed sequentially in coordinator-granted exclusive lane. No source changes during builds:

| Selection | Result | Cargo build | Execution |
| --- | --- | --- | --- |
| `--lib hook_argv_` |2passed,0failed,3596filtered|34.47s|0.00s|
| `--lib installed_argv_and_hook_parser_are_one_contract` |1passed,0failed,3597filtered|0.03s|0.00s|
| `--lib launch_prefix_` |2passed,0failed,3596filtered|0.03s|0.00s|
| `--lib hermes_codec_keeps_canonical_routing_and_immutable_prepared_kind` |1passed,0failed,3597filtered|0.03s|0.00s|
| `--test hook_entrypoint native_event_entrypoints_` |1passed,0failed,60filtered|58.49s|1.11s|

Actual binary test covers Claude SessionStart/PreToolUse and Codex SessionStart/PreToolUse/SubagentStart through stable producer command and shell boundary; malformed child field proves child codec actually runs; unknown/cross-harness pairs fail before check-in/digest/parse-report daemon counters change. Existing Fixture owns isolated UDS/service thread and private paths; every spawned shell/hook uses spawn_owned and wait_with_output; Fixture Drop calls stop/cancel and joins its worker before removing root. No owned process retained after test return.

Then mandated `nice cargo clippy --locked --all-targets --all-features -- -D warnings` exited101 on exactly one style issue (`clippy::collapsible_if`, src/cli/hook.rs:226), with no other compiler/test diagnostics. Released lane after all clippy jobs exited, applied suggested equivalent let-chain and cargo fmt. No semantic change. Corrected final lint/format plus explicit Hermes actual-process facade compatibility remain queued; do not repeat expensive native test absent a new risk.

## Final verification and self-review

On final source (only equivalent collapsible-if let-chain change since native GREEN), sequential own-target/jobs4 commands:

```sh
CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.4.1/target nice cargo clippy --locked --all-targets --all-features -- -D warnings
CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.4.1/target nice cargo test --locked --all-features --lib actual_process_entrypoint_reaches_startup_callback_admission
cargo fmt --check
```

Results: clippy exit0, no diagnostics, Cargo `Finished dev profile ... in 1m10s`. Explicit actual Hermes process-entrypoint compatibility:1passed/0failed/3597filtered, build32.67s, execution0.07s. Format exit0, pristine output. git diff --check exit0. Total10 unique focused tests passed across recorded selections; no full suite or default-feature run (parent-owned by sequencing). No repeat real native dispatch needed for an equivalent lint-only expression rewrite; actual binary native evidence remains the58.49s build/1.11s execution above. Final Hermes child uses isolated HOME/config and spawn_owned/wait_with_output; no owned process remains.

Self-reviewed full production diff and changed tests. Changes stay within declared ownership plus approved Claude mod consumer expansion. No ownership/migration/adoption logic changed, no trust policy/invariant change, no daemon receipt/identity change. Existing hook argv producer remains for legacy installations; Task9 must explicitly adopt the new per-event producer API into its installation ownership/migration logic. No executable/state/endpoint heuristic stripping. Format/helper validations are covered at shell/parser boundaries, not by mirrored expected builders.

### Files changed

- src/cli/hook.rs: dedicated parser surface, registered-pair refusal and complete stable event argv producer.
- src/harness/setup.rs: complete stable event argv validation/shell formatter; existing installation ownership remains untouched.
- src/harness/claude_mod.rs: recognized stable/legacy event route extraction for watch.
- tests/cli/hook.rs: stable pairs/producer route identity and quoting, actual legacy roundtrip and reviewed undeclared-event assertion refinement. Restricted cross-harness codex setup loop to Codex harness, since invalid native pairs now correctly refuse.
- tests/harness/claude_mod.rs: stable/legacy event route compatibility and invalid forms.
- tests/hook_entrypoint.rs: actual all-five-event native codec callbacks and invalid-pair no-effects fixture in existing required-feature test target.

### Remaining concerns / integration seam

Behavior checks pass, but final per-change lint measured70s,10s above the normative60s budget, and first owned lib-test compile with jobs1 measured7m50s under known host contention. Subsequent jobs4 task test compilations measured51.41s/34.47s/58.49s/32.67s. No timeout/profile/assertion weakening, no cache manipulation, no repeated passing check solely for presentation. Coordinator notified; timing is a measured limitation for final readiness, not an increased budget. No correctness concern identified in self-review.

Task9 owns exact installation IDs/groups, repeat reuse, migration, native trust reporting, canary/tooling and setup command activation. Producer assumes canonical route values supplied by setup resolver; executable spelling remains part of exact command identity. Legacy arbitrary-event assertion refinement was explicitly reviewed by coordinator against actual producer/decoder/spec evidence and should receive independent reviewer attention. Parent owns fresh review, integration/default-feature readiness, bead closure and worktree removal. No push/stash/merge performed.

## Commit / final state

Commit: `70d8b778` — `feat(hooks): add stable registered native event entrypoints`.
Final own branch HEAD differs from base and own git status --short is empty. Commit contains exactly six declared/approved source/test files; report deliberately lives outside own worktree for coordinator ownership. No source edits after verification; normal git add hit sandbox index.lock denial and exact scoped git add/commit escalation succeeded. No automatic approval rejection. No remaining owned process or private server. No merge/bead closure/worktree removal.
