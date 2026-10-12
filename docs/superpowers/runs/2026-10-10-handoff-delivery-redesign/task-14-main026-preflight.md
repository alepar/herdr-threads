# Main026 composition and Task14 fixture delta

Read-only preparation, 2026-10-11. No application, source edits, builds or tracker changes. Exact upstream range inspected: `43cc19cd60c78306efc6c9cb16b7c46963c01355..026ad3ab444779216748cc5733e2241e2834ebaf`. B candidate `3ad07776` composes43+B. Live D remains its own frozen/review candidate based on ee1e; this document does not alter or replace that review snapshot. Parent owns composition execution, final checks and roast.

## Exact delta and preserved catalog

Two commits: `58ad5634f7fe82b27d1b38c199beb49e7a44ad2b` fixes ht-clj; `026ad3ab` prepares v0.8.1. Ten paths,455 insertions/21 deletions:

| Paths | Actual change |
| --- | --- |
| `src/store/service_controls.rs` | Required attachment refuses to reuse any invitation ever referenced by a requirement episode. Ordinary lookup preserves its prior rule. Existing effective requirement remains no-op. Otherwise existing allocator creates a fresh requirement-owned invitation/requirement. |
| `src/store/control.rs`, `src/store/seats.rs` | Shared `pub(crate) fn current_invitation_for_accept(&Connection,&ThreadId,&SeatId)->Result<Option<InvitationId>,ApiError>` first selects newest pending/noncancelled/nonrejected invitation, then falls back to newest noncancelled historical invitation. BOTH native accept and cooperative AcceptCurrent permit preparation use it. |
| `tests/store/control.rs` | Expands/renames the real managed reattachment regression through replacement, replay/release/history, explicit required acceptance, then native acceptance of preserved ordinary offer without second join interval. |
| `TRUST-POLICY.md`, `docs/evidence/ht-clj-required-invite-reuse.md` | Clarifies immutable invitation→one requirement lifetime linkage, fresh reattachment, pending-first acceptance, no service acceptance/receipt settlement. Evidence preserves the original D reporter regression and its historical expectation. |
| `Cargo.toml`, `Cargo.lock`, `herdr-plugin.toml`, `CHANGELOG.md` | Package/plugin version0.8.0→0.8.1 and release note only; no dependency update. |

**No migration file, schema catalog, wire shape, capability or released SQL changes.** Released1..31, redesign32..38, capacity39 and D40/41 retain the currently selected layout. No migration renumbering, historical SQL rewrite, old-schema relabeling or new upgrade fixture is warranted by this delta. Genuine existing schema proof remains required on the final composed tip, but consume producer evidence unless the composed blob actually changes schema.

## Minimal composition and ownership

After D's frozen source is committed/reviewed, compose Main026 in the coordinator-owned isolated composition branch, preserving complete B and D ancestry/provenance. No provisional merge into root integration. Independent composed-blob/source review is necessary even for clean automatic hunks; this audit neither runs nor predicts a clean merge result.

The only semantic production overlap requiring careful resolution is `service_controls::invite`:

- Main's pending query excludes historical requirement linkage for `InvitationConstraint::Required` via `NOT EXISTS(requirement_episodes WHERE invitation_id=i.id AND (created_decision_seq=i.created_decision_seq OR required_parameter))`.
- D's query also selects fourth tuple field `COALESCE(i.confirmation='lazy',0)` and creates a fresh timed required episode for a pending modern Lazy invitation, preserving the independent Lazy offer. Keep this tuple/guard **and** Main's historical linkage exclusion. Do not adopt Main's three-field tuple wholesale or drop the new constraint parameter.
- Preserve D's `record_command_change` calls after successful requirement attach/release and canonical `record_context_change`/monotonic command epoch behavior. Fresh replacement applies command mutation to the NEW invitation; it must not rewrite the original ordinary invitation or released requirement. Existing replay/no-op must not invent command/context mutations.
- Ordinary service Invite remains able to select the preserved independent ordinary invitation during/after a fresh required episode, with retained timing/policy/origin. Main's lifetime linkage rule is a necessary reuse restriction; it does not override D's additional Lazy/timed distinction.

Control/seat helper hunks may merge automatically, but verify both paths call the exact shared helper. Selecting the newest accepted requirement while an older ordinary invitation remains pending was the independently reviewed Main bug; never restore separate highest-episode queries during conflict resolution. Preserve D acceptance cancellation/rejection/requirement revision/confirmation projection semantics around the new selection. B direct pair Ensure lifecycle is separate: this repair does not authorize reinviting a rejected/left pair or overriding retained initial creation policy.

Merge TRUST additively: retain Main's fourteen-line lifetime-linkage/pending-first clarification and all B/D body, delivery, framing, A7 and capability limits. Upstream version0.8.1 wins package metadata consistently; no test-specific version gates. Preserve the evidence file as actual released timed-only history; it explicitly says the redesign's Lazy/context columns were absent from released43, so do not reinterpret its old outcomes as current composed evidence.

## Exact stale fixture and semantic delta

Main's registered lib test is now `store::control::tests::managed_required_reattach_preserves_upgraded_ordinary_invitation_and_history` (formerly `managed_release_preserves_upgraded_ordinary_invitation_and_its_deadline`). It uses `bound_fixture(100)`, real service authority/operations, full stored-row snapshots and actual cooperative permit issuance. Preserve all expanded assertions, not merely the new query. Upstream reported136 control/15 requirement/graph_d2 focused PASS is historical Main evidence, not evidence on composed B+D+026.

D's `tests/store/control.rs::same_invitation_requirement_attach_release_replacement_has_distinct_context_epochs` currently asserts third attach returns SQLite UNIQUE Conflict and unchanged context. That assumption is **actively stale after026**:

1. Keep the first supported timed ordinary reuse on identical invitation ID, exact operation replay without mutation, and attach/release monotonic command epochs (None→Required→None).
2. Replace the obsolete third Conflict expectation with successful fresh requirement-owned invitation/requirement identity and episode/timing, retaining original invitation and released requirement history. Never relax UNIQUE or attach another requirement to the same old invitation.
3. Its current `state()` tuple includes original invitation command epoch **and thread-level context revision/last_change_seq**. A fresh replacement legitimately advances thread context; do not assert the whole tuple remains equal. Assert old invitation command epoch/stored fields remain unchanged, new invitation has its own command epoch/current required command, and thread coalesced revision advances once for actual mutation. Exact replay/no-op does not advance either.
4. Retain modern Lazy sibling `service_requirement_uses_an_independent_episode_without_mutating_native_lazy_invitation`: first Required already uses a new timed episode and preserves Lazy policy/origin/deadline-none. Main's correction must not make that Lazy invitation first-reusable.

No third **same-ID** requirement attachment becomes supported. A third operation is supported through a fresh ID; bounded presentation source, hydration and revisit must discover it as a new canonical pending invitation. Old ordinary presentation remains its own current command/revision, not an alias for the replacement's required acceptance.

## Focused C additions, without new suite or scope

C retains Task14's admitted paths and existing hosts. Add/adjust only true combined seams; coordinator admits any necessary producer-test path before C edits. `tests/store/control.rs` is existing Main/D producer ownership, not automatically newly granted to C by this document.

- Consume composed Main regression plus D epoch/Lazy sibling proof. C's public handoff/Direct Ensure→delivery fixture can use preserved modern ordinary Lazy invitation, Required attach/release/fresh reattach, then actual full Hook/Watch pending context. Require two distinct canonical invitation identities and correct per-item commands; no stale command epoch, phantom acceptance, receipt ACK or Lazy wake. Reuse final D whole/partial/replay/format proofs rather than clone saturation suites.
- Native public acceptance after accepting the newer requirement must still settle the older pending ordinary invitation. Drive actual permit preparation and native Accept against the same canonical target; require one original join interval and unchanged historic requirement acceptance/provenance. Old exact required acceptance revision refuses; exact historical operation replay returns old result without reactivation. No direct SQL “accept” shortcut.
- B modern Ensure existing pair selection must report the ACTUAL selected episode/policy/outcome consistently with retained main lifecycle. Fresh service requirement is not a new direct-pair invitation or an override of Ensure policy. AlreadyJoined remains no new invitation obligation; rejected/left direct pair behavior remains unchanged.

Run exact nonzero existing registered producer selections only as needed: lib `store::control::tests::managed_required_reattach_preserves_upgraded_ordinary_invitation_and_history`, D same-invitation epoch test (updated oracle), D independent-Lazy required episode test, and the existing required/ordinary acceptance interval controls. Existing `--test service graph_d2::` is Main's integration host; no new binary/private accessor. C's mandatory `--test combined lazy_recovery_context::` six cases and30s/20ms send-projection wait remain unchanged. Full/expensive final/default-feature checks remain parent-owned after roast.

## Review/readiness checklist

Independent composition review checks the complete ten-path upstream delta plus final resolved service query, shared accept helper, D command producer calls/projection triggers and B policy/wire preservation. Exact clean automatic merge is an outcome to inspect, not acceptance evidence by itself. No new migration/DDL/capability/model-visible text; version0.8.1 is metadata, never admission authority. Required service acceptance remains exact revision and native/cooperative actor attribution; no service ACK/acceptance or heuristic binding repair.

Before C executes, refresh against actual merged B/D/026 reports and source; inventory final test names/hosts and obsolete third-Conflict assertions across admitted tests/docs. D74-path frozen review and current capability-withheld status remain intact until their own review gates clear. New upstream correction is bounded source composition, not a reopened delivery architecture or permission for unreviewed provisional integration.
