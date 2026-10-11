# Task 2 report: invitation confirmation protocol/schema

Status: DONE. Verified and committed on the assigned isolated branch.

## Contract and implementation

- Public wire type: `protocol::commands::InvitationConfirmation::{Lazy, RequireAck}` serialized `lazy` / `require_ack`.
- Public request field: `Invite.confirmation: Option<InvitationConfirmation>`, default/skip-if-none. `None` is historical timed behavior; `deadline_millis` remains the requested duration.
- Saved semantic field: `cli::journal::SemanticMutation::Invite.confirmation` with the same omission behavior. Frozen intents retain mode through command construction. `has_invitation_confirmation()` recurses through `Frozen`.
- Capability: `protocol::capabilities::INVITATION_CONFIRMATION_V1 = "invitation.confirmation_v1"`. Defined, deliberately **not advertised** until Task 6 canonical lifecycle serves it. Modern new/replayed intents negotiate before journal creation/submission; legacy intents bypass the new capability.
- Schema migration `0029_invitation_confirmation.sql` adds nullable `confirmation` and `inviter_seat_id` FK. NULL preserves historical timed interpretation and unknown original attribution. Existing `frozen_duration_ms` / `deadline_at` are retained unchanged. Modern insert requires original inviter; policy/origin update is guarded.
- New `invitations_confirmation_due` index uses state pending, no warning, reject_recorded=0, and `(confirmation IS NULL OR confirmation='require_ack')`. Existing indexes are preserved for old lifecycle query audits. Task 6 must select the new index/predicate and honor mode in all lifecycle/projections.
- Explicit Lazy plus deadline conflicts. Human wire claims cannot explicitly require agent ACK.
- All historical constructors mechanically set `confirmation: None`; no CLI default/lifecycle changes are made here.

## Coordination seam

Coordinator approved additional migration file and protected store/control fail-closed guard. Task 6 must remove the temporary `confirmation.is_some()` Unsupported guard only when lifecycle support is active, and then advertise the capability. Task 1 and Task 4 also have local v29 migrations. Coordinator will assign versions29/30/31 in serial completion order during bounded integration and fresh schema seam review. This invitation candidate is presently first ready and likely remains29; later candidates will be renumbered. This isolated worktree correctly remains at29.

Output projection fields are explicitly deferred to Task 6 by coordinator ruling. `protocol::results::InboxBatchItem::Invitation` and `InboxBatchV2Item::Invitation` currently have no confirmation/timing metadata. Task 6 must add optional confirmation/deadline metadata as needed for Task 8, preserving historical `None` and skipped bytes. Task 2 does not preempt query/result lifecycle ownership.

## TDD evidence

First RED command (own worktree; shared warmed `CARGO_TARGET_DIR`):

```text
CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/target nice cargo test --locked --all-features --lib invitation_confirmation_
nice: setpriority: Operation not permitted
Finished test profile in 4m 13s
running 2 tests
protocol::capabilities::tests::invitation_confirmation_wire_preserves_legacy_and_distinguishes_modern_requests ... FAILED
store::connection::tests::invitation_confirmation_schema_migrates_legacy_and_retains_frozen_origin ... FAILED
wire: unknown field `confirmation`, expected one of `thread`, `seat`, `deadline_millis`, `operation`, `claim`
schema: no such column: confirmation
result: FAILED. 0 passed; 2 failed; 3589 filtered out. exit 101
```

Both failures were expected missing functionality. Build exceeded incremental budget under measured concurrent external compiler contention; coordinator independently observed the compiler active, not stalled. Following commands use escalated `nice cargo` to avoid priority restriction. Second queued run also includes activation refusal regression before its guard implementation.

Second shared-cache command (escalated `nice cargo`) stopped with compiler error E0046: the newly added fake Client omitted the required `call_with_output` method. This test harness error was corrected before the next command.

Second runtime RED command (private warmed task target; escalation retained):

```text
CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.2.1/target nice cargo test --locked --all-features --lib invitation_confirmation_
Finished test profile in 10m 31s
running 6 tests
saved intent ... ok
capability producer ... FAILED (actual 4 calls, old captured assertion expected 2)
wire compatibility ... ok
schema missing guard ... ok
activation refusal ... FAILED (unwrap_err got Ok Invitation)
schema retention ... ok
result: FAILED. 4 passed; 2 failed; 3589 filtered out. exit 101
```

The activation failure demonstrated the missing canonical refusal. Only after that failure was observed, the temporary store guard was implemented. The capability call-count assertion was already corrected in source from 2 to 4 while this compiler was running; this run had captured the old version. No production policy change was needed for that assertion.

An independent Python/SQLite in-memory check executed all 29 SQL migrations and verified new catalog object text against the migration definitions: exit 0, `All 29 SQL migrations execute; new schema object audits match SQLite catalog.`

## Verification

All final Rust commands use `CARGO_BUILD_JOBS=1` and `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.2.1/target` with scoped `nice cargo` escalation.

```text
nice cargo test --locked --all-features --lib invitation_confirmation_
Finished test profile in 4m 53s
running 6 tests
capability producer ... ok
saved intent ... ok
wire compatibility ... ok
schema missing guard ... ok
schema retention ... ok
canonical activation refusal ... ok
result: ok. 6 passed; 0 failed; 3589 filtered out; finished in 0.36s. exit 0

nice cargo test --locked --all-features --lib protocol::capabilities::tests
Finished test profile in 0.07s
result: ok. 18 passed; 0 failed; 3577 filtered out; finished in 1.16s. exit 0

nice cargo test --locked --all-features --lib store::connection::tests
Finished test profile in 0.04s
result: ok. 130 passed; 0 failed; 3465 filtered out; finished in 2.15s. exit 0

nice cargo test --locked --all-features --lib store::control::tests
Finished test profile in 0.19s
result: ok. 111 passed; 0 failed; 3484 filtered out; finished in 18.84s. exit 0
```

The store regression selection includes pending reinvite/first-deadline precedence, explicit required membership flows, human rejection provenance, operator orphan invitation duration, and historical invitation rejection replay. No full suite was run, per project instructions. No private Herdr server, daemon child, or other helper process was started by this task. Socket capability tests and all Cargo commands exited normally.

```text
nice cargo clippy --locked --all-targets --all-features -- -D warnings
Finished dev profile [unoptimized + debuginfo] in 6m 26s. exit 0; no diagnostics.

cargo fmt -- --check
exit 0; no output.

git diff --check
exit 0; no output.
```

Measured final compile and lint durations exceed the project one-minute budget in this run. The coordinator observed active compiler contention earlier; these measurements are disclosed rather than treated as evidence of a source regression. No timeout or budget was weakened. Build lane released after lint; all owned Cargo/formatter processes have exited.

## Self review

Completed source diff review against the brief: no modern CLI default or invitation lifecycle is implemented; every mechanical old producer remains None; unsupported modern wire guards run before transaction work; no seat continuity, receipt attribution, required service or operator invitation semantics change. No correctness concern identified in the reviewed source. Final focused regression, lint, formatting and whitespace verification passed. Commit/status evidence follows below.

## Files changed

- `src/cli/commands.rs`
- `src/cli/handoff.rs`
- `src/cli/journal.rs`
- `src/cli/mod.rs`
- `src/protocol/capabilities.rs`
- `src/protocol/commands.rs`
- `src/store/control.rs`
- `src/store/schema.rs`
- `src/test_support/history.rs`
- `tests/cli/cooperative.rs`
- `tests/cli/handoff_delivery.rs`
- `tests/handoff_topology_authority.rs`
- `tests/integration/channel_activation.rs`
- `tests/integration/lazy_attention.rs`
- `tests/integration/topology_activation.rs`
- `tests/protocol/capabilities.rs`
- `tests/service/cooperative.rs`
- `tests/service/crash_matrix.rs`
- `tests/store/control.rs`
- `tests/store/cooperative_checkin.rs`
- `tests/store/facade.rs`
- `tests/store/retention.rs`
- `tests/store/schema.rs`
- `migrations/0029_invitation_confirmation.sql`

## Final commit and cleanliness

- Branch: `task-ht-0lo.2.1`.
- BASE: `957544e8849a36c5a211820aa5bcd3c4b0c92ea5`.
- HEAD: `30c1f07c387ce0de6fcb562be75e8d9e6c5e1772` — `Add invitation confirmation wire and schema contract`.
- HEAD differs from BASE. `git status --short` exited 0 with empty output after commit.
- All owned test, lint and formatter processes exited; zero owned child processes remain.
- Integration seams: serial migration version assignment; Task 6 removes canonical temporary guard, advertises capability, and produces query metadata. No unresolved implementation correctness concern.
