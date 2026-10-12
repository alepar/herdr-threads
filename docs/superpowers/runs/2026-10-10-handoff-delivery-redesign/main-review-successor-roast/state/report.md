super-roast verdict: clean (5 nits) [converged]
mode: pr        iteration: post-cap audit
profile (assumed): Internal same-user cooperative local-agent service with durable SQLite and immutable request, receipt and binding provenance, and native launch and wake effects. Preserve authority, bytes and deadline/evidence separation. This source-only audit supplies no real-config or shared-server mutation, live model uptake or release clearance claim.
inputs: main-review-successor@ab2d0171c97d2046461eba14c496df9a9f93b6db vs reviewed-Main-composition@7ea79d3d8e598b01729b61be9c0faa31a6e046f4 (source-only; RUN/scratch excluded)
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 2 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 11/11 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, observability, testing, hygiene-docs) · raw 0 → deduped 0 → panel 0 · spot 0 · promoted 0 · judge completion n/a (no panels) · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 0/0/0/0

Same-family review limits method independence. All fresh scouts returned no findings; this run required no dedupe or judge panels. The empty later round supports convergence under the supplied rule, within source-only coverage.

Prior confirmed finding status (immediately prior report):
- [Should-fix] scripts/wake-e2e.py:766 — Incorrect idle-wake invitation oracle (resolved).
  Current scripts/wake-e2e.py:773-774 makes direct Lazy invitation and send-lazy cases passive under wake_on=nudge. The person-driver group invitation remains unchanged at :785-786; the direct opening remains Lazy at :790. This resolves the selected narrow direct-Lazy oracle scope, without changing production delivery policy.
- [Should-fix] docs/agent-usage.md:159 — Universal head-seeding and seat-wide settlement guidance for direct acceptance (resolved).
  Current docs/agent-usage.md:161 confines generic settlement to join/nonpair acceptance. At :163 active direct acceptance preserves existing cursors, seeds missing open-binding cursors at the invitation event, and preserves independent sibling unread streams and normal fresh-binding-after-acceptance seeding.

Historical iteration 1 status:
- [Should-fix] integrations/claude/mod/hooks/register.js:289 — Late old-run refusal deleting replacement invitation context (resolved previously; historical status retained).
- [Should-fix] src/store/invitation_content.rs:2442 — Unbounded membership-history work under an outer warning charge (resolved previously; historical status retained).
- [Should-fix] tests/store/effective.rs:2136 — Receipt-upgrade catalog assertion rejecting migration-45 triggers (resolved previously; historical status retained).
- [Should-fix] src/service/delivery_presentation.rs:62 — Owner-mutex waiting ignoring cancellation/deadlines (resolved previously; historical status retained).
- [Should-fix] integrations/claude/mod/hooks/register.js:296 — Unbounded retained invitation revisions (resolved by newly user-authorized queue repair).
  Current register.js:317 retains the exact inflight object while replacing pending revisions. Refusal at :349-354 coalesces pending contexts; successful whole completion at :364-365 removes only identical noninflight replay context. Append selection at :67-72 bounds whole batches to 32 items and 65,536 framed UTF-8 bytes. The old intentionally-unfixed queue waiver is not carried forward. This historical repair is separate from the two immediately prior confirmed entries counted in the delta.

## Confirmed findings

- none

## Not verified (beyond panel cap)

- none

## Not verified (dedupe failed or judge lost)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

Historical rejections remain standing. No current packet supplies materially changed evidence; their cited accepted limits remain normative.

- [FYI] src/cli/watch.rs:365 — Upgrading the executable and daemon while retaining the published Claude mod makes Watch select V2 output whose invitation and notice items that installed mod silently discards.
  reason: Overruled default confirmed to rejected on the reproduce seat's concrete contract evidence. The selected task-15-framing-refinement-proposal.md:138 explicitly states that older mod consumers ignore additive Notice and do not acquire upgraded completeness guarantees. protocol/watch.rs:491-492 explicitly states that older consumers ignore additive Invitation and require upgrade for its content. Q3:25-29 prescribes this CLI capability election and forwarding. The two confirming seats' claim that no accepted limit covers the installed parser does not address those exact additive-consumer provisions. The source mechanism is real, but demanding additional mod-origin negotiation strengthens the selected contract. This does not qualify current consumers or pass final C.

- [FYI] src/harness/agy/delivery.rs:1152 — The new V2 hook delivery path silently discards persistent presentation errors and codec refusals, so stalled delivery produces the same empty successful hook outcome as a quiet inbox.
  reason: Reproduce and refute seats distinguish the mandatory received-page sizing-mismatch diagnostic at framing proposal:46 from Refused/ApiError. Removed/drifted codecs are an explicitly accepted refusal/repair limit. The ground seat confirms the visibility issue as pre-existing on base src/harness/agy/delivery.rs:818-840 and cli/hook.rs:3131-3145, and concedes the accepted V2 refusal. Its evidence does not rebut the majority's scoped-contract reasoning. Default rejected stands.

- [FYI] integrations/claude/mod/hooks/register.js:246 — A queued timed invitation can wake the session after its attention episode has been cleared because the clear retracts only Attention items and leaves the invitation's wake eligibility intact.
  reason: Reproduce and refute seats cite the selected framing proposal:277's epoch-scoped Attention-only clear and :299's no-retraction limit for already emitted downstream content. Watch has already written/flushed the invitation before it enters this JS queue. The minority treats unappended JS content as un-emitted, but the majority explicitly addresses that distinction using the selected FULL OFFER boundary at TRUST-POLICY.md:1524-1528. A general invitation wake-revocation guarantee is not selected. Default rejected stands.

- [FYI] src/store/invitation_content.rs:2632 — The new first-discovery normal render-deferral path has no regression control proving that a failed final private save rolls back its traversal progress.
  reason: The exact overflow-arm fault control is absent, as all seats acknowledge. The refute seat identifies tests/store/invitation_content.rs:1501-1520,1619-1712 and tests/service/q4_retirement_refusal_controls.rs:483-500 exercising the shared final-save/decision rollback mechanism. The new arm changes only local State; store/mod.rs:1297-1312 and connection.rs:350-384 retain the same transaction boundary, with service progress unreachable on error. The ground seat concedes correct transactional ordering and those common controls; its branch-specific coverage preference establishes no distinct unprotected mechanism. Default rejected stands.

- [FYI] src/store/invitation_content.rs:2945 — Small presentation metadata reads and passive maintenance are coupled to loading and rewriting the full retained presentation payload through the monolithic State accessor.
  reason: All seats verify bounded extra copying, but no incorrect output, changed authority/frontier, unbounded retained scan or measured speed-budget failure. Individual retained-field caps total at most 532826 bytes for the four cited fields. Q3:95 explicitly acknowledges that every refresh saves this table; the narrow reader-operation contract applies elsewhere. Payload-independent metadata cost is not selected. Default rejected stands; final speed evidence remains deferred.

- [FYI] src/store/schema.rs:689 — A binary rollback to the published build cannot reopen an instance after this build commits its first new database migration.
  reason: All seats distinguish real unknown-schema refusal from the selected forward migration contract. resume-main-v090-schema-impact.md:19,75-81 requires per-migration atomicity and forward repair/reopen; tests/store/schema.rs:8190-8226 explicitly expects committed V36 after a failing V37 and resumes with the new initializer. No reverse-migration or executable-only downgrade guarantee is selected. Retained data loss is not demonstrated. Default rejected stands.

- [FYI] src/store/invitation_content.rs:2774 — Presentation allocation and replay render and persist a duplicate outbox_text payload that no output or recovery path reads.
  reason: The redundancy is real. Reproduce and refute seats cite the selected framing proposal:84,144-146 expressly retaining bounded frozen plaintext and descriptors, with separate 65536/262144 byte ceilings. The ground seat's Nit rests on avoidable bounded work and concedes no output, authority or measured speed regression; it does not rebut that selected representation/storage contract. Default rejected stands; optimization remains optional.

## Unverified nits (spot-checked)

These five historical entries retain their original single-seat status. This audit dispatched no spot checks and supplies no documented resolution evidence; their current resolution remains unverified.

- [Nit] integrations/skill/SKILL.md:23 — The embedded agent skill still classifies invitations generally as idle-waking nudges after ordinary agent invitations become lazy by default.
  evidence: Refute spot check confirms the generic classification conflicts with the correctly updated :76/:120 and commands.rs:2260-2265 Lazy default. Single-seat verification only.
- [Nit] src/harness/setup.rs:59 — The stable-hook ownership change leaves the current installation guide instructing users that every owned command contains the removed per-install owner marker.
  evidence: Refute spot check cites docs/install.md:183,187 against marker-free commands at setup.rs:59-76 and version-3 manifest ownership. Adapter notes are correct but do not repair the linked guide. Single-seat verification only.
- [Nit] src/cli/help.rs:74 — The compiled launch help still presents installed owned hooks as a mandatory preflight condition even though this change allows launch without them.
  evidence: Refute spot check cites LAUNCH_HEAD:69-78, golden launch help:49-58 and positive tests/cli/launch.rs:466-523 for launch without ready hooks. Single-seat verification only.
- [Nit] src/cli/help.rs:141 — The expanded public handoff body API leaves the linked current command reference describing only a positional body capped at 1024 bytes.
  evidence: Refute spot check cites docs/agent-usage.md:339-340 against the exclusive body/file/stdin group at commands.rs:1362,1408-1415 and protocol/handoff/automatic.rs:117's 65536-byte bound. Single-seat verification only.
- [Nit] src/delivery/invitation.rs:119 — The new invitation module exposes an unused second frozen-outbox representation and rendering API alongside the representation that production actually uses.
  evidence: Refute spot check finds FrozenInvitation/block_text/join_blocks/render_key disconnected from production; private FrozenPresentation at invitation_content.rs:1560 carries actual replay authority. The public offset format creates low-impact maintenance ambiguity. Single-seat verification only.

## Escalations (need human)

- none

The original Goal, accepted limits and readiness gates remain unchanged. Queue, generic/direct-reader documentation and the narrow person-driver oracle repairs are now explicitly authorized; no prior queue waiver is inherited. Final C, actual current-consumer qualification, controlled speed measurements and authenticated pre-release wake qualification remain deferred. This source-only report claims no runtime or native PASS, production activation, live model uptake or release clearance.

