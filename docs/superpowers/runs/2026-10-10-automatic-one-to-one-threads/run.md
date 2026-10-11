# super-auto run — 2026-10-10-automatic-one-to-one-threads

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: roast-code

idea: Use $super-auto, fully autonomous end to end with both roasts on, to implement this feature in this repo: automatic 1:1 threads. An agent can send a message to another agent by giving only a target seat ID, without specifying a thread. The plugin automatically creates (or reuses an existing) "1:1" thread with exactly two participants, the sender and the recipient, which otherwise behaves like a normal thread. The handoff command gets the same treatment: if no thread is specified, the handoff goes through the 1:1 thread. When the integration branch is ready for merge, run `herdr-threads join dev-herdr-threads` (thread taQXwBH7X) and send a --nudge message there stating that this feature is ready for integration into main.
branch: super-auto/automatic-one-to-one-threads
base: main
spec: 2026-10-10-automatic-one-to-one-threads-design.md
epic: ht-e99

approvals:
- top-split · auto under explicit fully-autonomous delegation · ht-e99.1 PROMOTE, ht-e99.2 LEAF, ht-e99.3 PROMOTE, ht-e99.4 LEAF, ht-e99.5 LEAF (post-coverage mandatory sweep added auto), ht-e99.6 LEAF (design-only roast repair added auto)
- coverage-round-1 · canonical R-list: R1: Target-seat-only send accepts --seat with body/file/stdin; positional ordinary send stays unchanged and mutual-exclusion/no-target validation is explicit.; R2: Direct recipient qualification rejects self/missing/retired/foreign/unresolved/held recipients while supporting resolved unbound recipients and ordinary human send.; R3: Direct lazy/nudge/ACK/deadline/relay/user-intent behavior freezes exactly the addressed target, rejects extra ACK seats/panes, and retains invitation consent.; R4: Omitted handoff channel selects automatic routing for existing-seat, existing-pane, launched-pane and new-tab destinations; explicit forms and metadata constraints remain.; R5: Indexed canonical unordered seat-pair registry has additive migration, uniqueness/FKs, deterministic defaults, no ordinary-thread adoption and stable retention/identity.; R6: Accountable EnsureDirectThread atomically decides A2/recipient eligibility and create/invite/reuse, converges reversed concurrency and replays historical saved results.; R7: Ordinary create/invite events, membership/attention projections and actor provenance honestly distinguish reservation from joined/accepted state; TRUST-POLICY A5 is updated.; R8: Immutable pair enrollment guards every ordinary/service/operator path and forbids third seats and service ownership while preserving ordinary unmarked channels.; R9: Archive/leave/reject/retirement lifecycle refuses resurrection/remapping, reports explicit recovery and reuses the same channel after ordinary repair.; R10: Send preparation/publication validates pair identity and audience, including corrupt membership/raw protocol/ensure-to-send races.; R11: DirectSend intent freezes original actor/caller/namespace/target/body/options/ensure key/send key before effects, with bounded progress and locked atomic synced publication.; R12: Pending-ops/retry resumes the exact ensure/send and selected thread after loss/restart, retains staged failures, never duplicates or replaces intent and preserves historical completed output/cleanup.; R13: Preflight checks syntax/flags/caller/target/capability before publication/effects where supported; authoritative validation stays transactional and final output names ordinary thread/message IDs.; R14: New automatic handoff variants/versions preserve V1 bytes/hashes/reports/keys and explicit forms, and negotiate additive capability before journal/effects without unsafe fallback.; R15: Existing-recipient automatic handoff freezes recipient/ensure key and canonically registers/attaches selected thread before restricted invite/send, enforcing original caller/payload/namespace/fence equality.; R16: New-tab auto routing occurs after guarded recipient attachment and retains bootstrap/child fence, downstream/outer recovery and at-most-one native submission without new topology/start authority.; R17: Startup remains managed_launch without acceptance/ACK/completion; staged archive protections survive until canonical completion and completed history does not revive them.; R18: Focused store tests exercise migration, identity/index, concurrency, replay, invalid bindings/namespaces, corrupt membership, every enrollment path, lifecycle, human/unbound recipients and ordinary isolation/history.; R19: Focused CLI tests exercise both send forms/body/options and pending retry at every stage with historical journals preserved.; R20: Handoff tests run each of four destination configurations before the terminal gate, retain V1 golden fixtures and exercise child equality, lost replies/restart and topology completion recovery.; R21: README, embedded skill/manual/help document syntax, lazy default, invitations, immutable pair/lifecycle and exact recovery examples without editing real config.; R22: Focused verification follows fmt, locked all-target/all-feature clippy and relevant tests; merge adds default-feature checks, owned helpers stop and scoped leak checks follow full/integration sweeps within stated budgets. · requirements: 22 · mapped: 21 · unmapped: 1 (R22) · auto capability activation ownership → adopt/amend existing .3.1/.3.2/.3.3; auto unbound/Human configuration → .1.2; auto unsupported-capability configuration → .2/.3.2/.3.3; auto R22 → verification acceptance all leaves; auto promotion flag .2 → retained with recorded cohesion rationale.
- coverage-round-2 · requirements: 22 · mapped: 21 · unmapped: 1 (R22) · auto COV2-01/COV1-04 → terminal sweep ht-e99.5 owns exact default-feature/leak/budget gate; auto COV2-02 parser candidate rejected against nested explicit new-tab ownership, shared grammar boundary clarified without new edge; trajectory 6→2, novel 1/2, count shrank. No third coverage round.

roastDesignRound: 2
roast-design: 2026-10-10-automatic-one-to-one-threads-roast-design-1.md, 2026-10-10-automatic-one-to-one-threads-roast-design-2.md

parked:
- 2026-10-10-automatic-one-to-one-threads-roast-design-2.md · degraded-verdict · "clean (0 nits) [low coverage] — proceeded, not re-roasted; auto under explicit fully-autonomous delegation. All9scouts valid/completed; zero raw findings triggers reporter literal rule; no failed stages or missing applicable judges. Prior Blocking resolved, no new/carried/regressed; no escalations/beyond caps."

Design hand-off: ht-e99 ready for caller-owned code phase; two design-roast rounds, one design-only repair ht-e99.6 closed at ce11790f. No implementation code delivered.

codeBuckets:
  completed: ht-e99.1.1, ht-e99.1.2, ht-e99.2, ht-e99.3.1, ht-e99.3.2, ht-e99.3.3, ht-e99.4, ht-e99.5, ht-e99.6, ht-e99.7
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: CLEAN

Code phase started: ht-e99.6 is closed design-only repair at ce11790f, with no implementation merge.

roastCodeRound: 1

Whole-epic successor: `.superpowers/sdd/ht-e99-plan/whole-epic-review-2.md` is CLEAN at 75d6fb5f662d4f10ccdc540accd909fc935e6969 against main6107075f; original NEEDS_FIX review preserved. Task9 repair independently verified. Full suite remains unmeasured. PR roast round 1 begins at the checkpoint containing this state.
