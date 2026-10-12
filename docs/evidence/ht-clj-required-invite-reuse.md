# ht-clj: required reattachment after ordinary invitation reuse

2026-10-11. Independent repair based on released Main `43cc19cd`.

## Original evidence retained

`bd show ht-clj` records the reporter's canonical producer test
`same_invitation_requirement_attach_release_replacement_has_distinct_context_epochs`
on `task-ht-0lo.8.4`, based on `ee1e598c` plus its context-epoch changes. That test
intentionally records the third attach as a Conflict with unchanged state. Its source
and result are unchanged by this repair; no unfinished handoff redesign was imported.
The reported ordinaryLazy control belongs to that independent lane. Released
`43cc19cd` invitations have no confirmation-policy/origin/context-epoch columns;
this repair exercises the released timed invitation implementation and preserves
all its stored columns and existing attribution.

## Independent reproduction

An isolated worktree at `43cc19cd` first passed the existing
`managed_release_preserves_upgraded_ordinary_invitation_and_its_deadline` test
(1 passed). Extend that real SQLite service-control test through another required
Invite after release. Before changing production code:

```text
nice cargo test --locked --all-features --lib managed_release_preserves_upgraded_ordinary_invitation_and_its_deadline
0 passed; 1 failed
ApiError { code: Conflict, detail: "SQLite: UNIQUE constraint failed: requirement_episodes.invitation_id", ... }
```

The regression is now named
`managed_required_reattach_preserves_upgraded_ordinary_invitation_and_history`.
The failing source was the same test through its first replacement operation,
against unchanged production source at the released baseline.

## Cause and decision

The pending-invitation query excluded requirement-owned invitations (same invitation
and requirement creation sequence), but allowed an independently created ordinary
invitation even when a released requirement already referenced it. Release correctly
preserved that ordinary invitation. A later required Invite selected it again and
attempted an INSERT forbidden by the ledger's UNIQUE invitation linkage.

For required attachment only, exclude every invitation already referenced by any
requirement episode. A still-effective requirement remains an idempotent no-op.
An unreferenced pending ordinary invitation can still be reused on first attachment.
Otherwise create a fresh requirement-owned invitation through the existing monotonic
episode allocator. Ordinary invite lookup retains its independent-invitation behavior.

An early refusal would strand otherwise supported reattachment. Relaxing UNIQUE or
rewriting a historical link would change the ledger contract. Fresh invitation
creation uses the existing replacement mechanism and requires no migration or new
wire shape. TRUST-POLICY.md documents the clarified lifetime linkage in the same
change. It preserves explicit native acceptance, service attribution, canonical
transaction authority, release history and exact historical operation replay.

## Regression coverage

The real service-control regression verifies initial ordinary reuse without a deadline
reset, release without cancellation of that independent invitation, fresh replacement
identity/episode/timing, required no-op and exact-key replay, ordinary lookup during
replacement, refusal of stale exact-revision acceptance, cancellation of only the
fresh pending requirement-owned invitation, and historical replay after release
without reactivation. A further attachment is explicitly accepted under the existing
`cooperative_top_level` provenance. Full-row snapshots protect the original invitation
and released requirement across these transitions.

No new model-visible string, wake behavior, service capability or schema is introduced.

## Review correction: pending ordinary acceptance

Independent review found that a newer accepted requirement invitation could shadow
an older preserved ordinary invitation in native `accept THREAD`. Both permit
preparation and acceptance selected the highest episode even if already accepted.
An extended regression using real cooperative permit issuance reproduced the wrong
accepted invitation ID before the correction (0 passed; 1 failed).

Both paths now share selection of the newest live pending invitation, falling back
to the existing latest noncancelled invitation only when none is pending. The test
explicitly accepts the preserved ordinary invitation after the new requirement,
checks its settled state and empty invitation inbox, and verifies the original join
interval is retained. Exact historical acceptance replay and terminal diagnostics
retain their existing fallback. No acceptance or provenance is invented by service
reattachment itself.

## Final validation and review

All checks on the final source passed:

| Check | Result |
|---|---|
| `nice cargo test --locked --all-features --lib store::control::tests::` | 136 passed; 2.13 s runtime; incremental build 35.05 s |
| `nice cargo test --locked --all-features --lib requirement` | 15 passed; 0.17 s runtime |
| `nice cargo test --locked --all-features --test service graph_d2::` | 1 passed; 0.64 s runtime |
| `nice cargo clippy --locked --all-targets --all-features -- -D warnings` | exit 0; 41.31 s including build-lock wait |
| `nice scripts/check-default-features` | exit 0 |
| `scripts/check-harness-neutral` | exit 0; 0 violations |
| `cargo fmt --check`, `git diff --check` | exit 0 |
| `scripts/check-no-leaked-processes --run-id 82163c57-70a3-4a2f-b8aa-127dc16071e9` | no leaked test processes |

The sandbox denied process-priority adjustment (`nice` still ran Cargo), the
integration daemon's private Unix socket setup, and the leak checker's process
enumeration. The isolated integration test and leak check were rerun with approved
permissions and passed. The initial socket-denied test did not reach the product
flow. No shared server or real user configuration was changed.

Independent read-only review by `review_required_reuse` first found the native
acceptance shadowing issue above. After its failing regression and correction,
re-review returned no Critical, Important or Minor findings and **Ready to merge:
Yes**, conditional on the checks now completed. The full suite was intentionally
not run, following AGENTS.md's focused-test requirement while ht-zo4 remains open.
Main owns integration, release, installation and worktree cleanup.
