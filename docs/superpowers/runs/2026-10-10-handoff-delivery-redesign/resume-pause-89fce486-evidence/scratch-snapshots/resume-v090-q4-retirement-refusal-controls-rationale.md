# Q4 modern Lazy retirement refusal controls — scratch proposal

2026-10-11. Tests-only work against the frozen nested candidate. No candidate
file, production source, Cargo/test runtime, Git/index, Herdr/config, or
nested-agent action occurred. Only ignored outer-workspace scratch was written.
Parent reported current candidate7242; no independent Git attestation is claimed.

Patch `resume-v090-q4-retirement-refusal-controls.patch` proposes a footer child
mount in `tests/service/q4_body_controls.rs` and a new
`tests/service/q4_retirement_refusal_controls.rs`. It preserves BodyFixture and
all existing tests. The footer mount is disjoint from the separately authored
printed-CLI-next correction to the earlier body loop; parent owns applying both.

## Existing coverage and the actual gap

The inspected `tests/store/invitation_content.rs` Q1 controls already prove:

- `continued_body_retirement_is_private_and_charged_once` (1290): private
  retirement versus strict public continuation, one charged continued candidate.
- `continued_body_refuses_corrupt_accumulator_and_missing_authority_without_progress`
  (1468): corrupt private accumulator and missing prepared ordinary recipient.
- `continued_body_retirement_rolls_back_when_final_state_write_fails` (1501),
  plus cancellation controls: rollback/retry after retirement and final save.
- Pending capture and sibling-unread controls (1351/1383): receipt and reader
  protections remain separate from private retirement.

Those fixtures capture `Source::Unread` and use ordinary receipt authority.
They do not enter the new `Source::Lazy` modern scope-loss branch after an
actual Reject/Invite transition. Q4's existing
`private_old_episode_lazy_tail_retires_and_reaches_fresh_preaccept_body` supplies
the positive counterpart for that new branch; this proposal does not repeat it.

The source branch in `queries.rs` admits retirement only for private normal
ordinary-source traversal with legacy_pending0, after immutable captured
publication/address/range and accumulator validation. Public and legacy tails
remain strict. Missing authority, corrupt continuation state, and scope loss
must not be conflated.

## New controls and independent producer witnesses

The nested child reuses the existing actual BodyFixture. Each setup uses its
public Ensure/Send and actual private presentation to capture a long UTF-8 Lazy
prefix, serializes the actual production Watch lines and confirms only physical
output. No display cursor is created. It canonically Rejects the old invitation,
Invites a new episode, and publishes a fresh body. It verifies old publication
decision < new invitation creation < fresh publication, actual legacy_pending0,
both own cursors absent, and the actual production continuation-scope query now
false before any fault. The expected false result comes from those independently
established episode facts, not a copied predicate.

Five tests fill the missing Lazy-specific coverage:

1. `modern_lazy_scope_loss_cannot_hide_corrupt_accumulator_or_utf8_offset`
   has five isolated fault cases: invalid accumulator ID, another valid message's
   accumulator ID, wrong accumulator byte length, mismatched skipped body ID,
   and a structurally valid cursor whose byte offset falls inside UTF-8. It
   requires the existing corruption/boundary refusal and precise guard detail,
   rather than allowing the now-false audience scope to retire malformed state.

2. `modern_lazy_retirement_requires_intact_captured_publication_and_recipient_authority`
   has five cases: missing manifest, mismatched manifest publication decision,
   message+manifest sharing a foreign instance that differs from the captured
   instance, missing exact Lazy recipient, and recipient thread mismatch. The
   foreign-instance case deliberately preserves message/manifest agreement,
   isolating captured-instance validation instead of succeeding merely because
   those two rows disagree. Each case requires InvalidCursor at the captured
   addressed-content guard and exact unchanged state bookends.

3. `public_lazy_continuation_refuses_real_old_episode_audience_loss` sends the
   real captured cursor through the public InboxBatchV2 query with its actual
   claim. It requires the strict audience refusal, never private retirement.
   Q1's public ordinary control does not replace this new Lazy-scope witness.

4. `legacy_lazy_tail_keeps_strict_refusal_when_modern_reader_scope_is_lost`
   marks only the isolated recipient's legacy_pending1 compatibility flag.
   All captured publication/body/addressing authority remains real. It requires
   strict continuation refusal instead of the modern-only retirement exception.
   This is a compatibility-state fixture, not a claim that a new public producer
   creates legacy rows or that a historical migration was executed.

5. `modern_lazy_scope_retirement_rolls_back_when_final_private_save_fails`
   faults the exact continued Watch consumer's final save only when its cursor
   changes. That requires tentative modern retirement rather than a prior
   passive/context-only save. The expected trigger error identifies the final
   write failure, then all private and canonical bookends must be restored.
   Q1's Source::Unread final-save control exercises a different retirement guard.

The five parameterized tests represent thirteen isolated fixture cases, not
thirteen newly registered test functions. No new process target or process is
introduced. They exercise actual store/query paths, not a fake retirement model.

## Fault boundaries and rollback proof

Canonical producers establish all pair, invitation, body, addressing and cursor
state. Only impossible-by-writer corrupt records use explicit fixture SQL.
For recipient identity/deletion and immutable publication changes, the helper
temporarily removes one named guard in the isolated database, applies one row
mutation, then restores its exact original catalog definition before any
production reader call. Foreign instance authority gets an isolated host row
for FK-valid corruption. No foreign-key/check disabling, real user config,
heuristic enrollment or real server mutation is used.

Every error control compares all rows in consumer/item/private proof/fence
tables, binding cursors/bindings, Lazy recipients, receipt_state/receipts,
invitations/memberships/intervals, wake_work, warning/digest notice frontiers,
messages/manifests and host_instances. It separately requires both reader
cursors absent and the current invitation still pending/invited with no receipt
or native wake. These bookends catch discarded partial/skip/cursor state,
publication/context/private counter progress accidentally committed before an
error, and canonical display/ACK/read/notice effects. The snapshot is taken
after intentional fault setup, so refusal must preserve even the corrupt record
instead of silently repairing it.

## Verification limits

Source inspection found the gap; it is not a runtime result. rustfmt parsed and
formatted the scratch child with edition2024 and skip_children=true. Unified
patch generation read the frozen parent file. No compile, SQL trigger/query
execution, actual refusal, actual rollback or GREEN is claimed. Parent must
compile and run the new selectors, distinguish setup faults from guard outcomes,
and rerun the existing Q1/Q4 positive counterparts as needed. The tests make
no new physical-query/probe bound or production-budget claim.

Quiescent hashes:

```
29401c48f920c6296971166ef9a25c4722907e1e5ea48ae651c4d8812524313f  q4_body_controls.rs mount preimage
515743f2d49f225bcd45693af904e5e7c87ff28d6233876ecc5601a5ef419587  scratch q4_retirement_refusal_controls.rs
282a560ccb99083c60b1b63e83621bf23b9ee6886b52d68e07f378ce64f2da5d  resume-v090-q4-retirement-refusal-controls.patch
```
