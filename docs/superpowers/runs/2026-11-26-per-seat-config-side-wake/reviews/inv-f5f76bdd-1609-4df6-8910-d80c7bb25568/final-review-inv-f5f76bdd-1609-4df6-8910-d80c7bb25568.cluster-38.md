# Final review: cluster 38 (tests-sync), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`; caller checkout HEAD equals the head SHA, working tree clean at review time). Review source bound via `bind-review-source` (exit 0).

## Scope

Slices 000243 and 000244, one line each:

- `tests/sync/facts_message.rs:979`: `Sender`'s `CallerClaim` literal gains `auxiliary_session: false`.
- `tests/sync/facts_receipt.rs:156`: `Node::completion_claim` gains `auxiliary_session: false`.

Both files are `#[path]` modules of `tests/combined.rs`. They change because of commit 136484396 (ht-z41e.21), which added `CallerClaim.auxiliary_session: bool` in `src/protocol/authority.rs` with `#[serde(default, skip_serializing_if = "std::ops::Not::not")]`.

## Analysis

- Compile coverage: these are the only `CallerClaim { .. }` struct literals under `tests/sync/` at head (git grep), and both now set the new field. Leaving one out would break the build, and `clippy --all-targets --all-features` would catch it.
- Behaviour preservation: `false` is skipped on serialization, so these claims put the same bytes on the wire as before the change. The sync fact tests therefore exercise exactly what they did at base. `false` is also the right value: these fixtures model ordinary top-level and person claims, not Codex `/side` sessions.
- Spec fit: the design states that seat config is node-local, with no fact kind and no sync. The auxiliary bit is a binding attribute that only queue root selection reads. Neither feature adds anything to replicate, so no new sync-fact test is owed in this cluster.
- Requirement baseline: rows B1-B10 and S1 are delivered by production code outside this cluster. This cluster is fixture upkeep and neither delivers nor weakens any row. The end-to-end tracing of the rows belongs to the clusters that own them and to the seam reviewer.
- Observation outside the cluster, for the seam reviewer: `CallerClaim` keeps its pre-existing `#[serde(deny_unknown_fields)]`, and the capability comment relies on that. The hook sends the field only to a daemon that advertises `seat.auxiliary_session_v1`. The claim travels on the local CLI-to-daemon protocol, not the peer wire that `scripts/check-wire-compat` governs, so this cluster takes no position on it.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- none

## Deferred OK
- none
