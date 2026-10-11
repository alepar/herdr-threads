### Spec Compliance

- ❌ Needs fixes: the original inviter's open-condition wake is still removed by informational-offer settlement. `src/store/attention.rs:1129` adds the correct origin predicate, but `src/store/attention.rs:1180` applies it only to the offer-filtered pending-warning set. This misses the binding requirement that open invitation-condition wake be independent of the per-binding informational frontier.
- ✅ The reviewed owned scope otherwise implements immutable modern policy/origin and conditional event/wake creation (`src/store/control.rs:1530`, `src/store/control.rs:1671`), inert Lazy overdue timing (`src/store/schema.rs:3607`), finite indexed invitation scanning (`src/store/invitation_due.rs:54`), required-over-Lazy fresh episodes (`src/store/service_controls.rs:361`), passive inviter clears (`src/store/schema.rs:3466`), optional omitted legacy metadata in both result versions (`src/protocol/results.rs:1056`, `src/protocol/results.rs:1099`), capability activation (`src/daemon/control.rs:324`), and the temporary linked None guard (`src/store/topology_handoff/attachment.rs:285`). A7 is updated in the same commit (`TRUST-POLICY.md:413`).
- ⚠️ Downstream obligations remain explicit: Task7 shared driver/fairness/combined physical budget; Task8 client attention predicates at `src/cli/watch.rs:550` and `src/cli/hook_inbox.rs:105` plus final presentation/defaults; Task10 frozen linked policy replacement at `src/store/topology_handoff/attachment.rs:281`. Provisional migration30 needs integration renumbering and fresh overlapping schema review after rebase. These are not Task6 defects.

### Strengths

- `src/store/invitation_due.rs:54` bounds the ordinal range in SQL before LIMIT and excludes Lazy through a partial index; future rows continue the traversal, and exact-full slices honestly retain a cursor for one final empty probe. The physical-work regression checks both VM work and absence of sorting (`tests/store/invitation_due.rs:532`).
- `src/store/service_controls.rs:361` creates a real separate timed episode instead of rewriting the voluntary Lazy row. The service regression uses actual canonical operations and preserves the old policy and keyed replay (`tests/store/control.rs:10332`). Release's exact requirement/invitation creation-sequence check at `src/store/service_controls.rs:466` cancels only the service-owned episode.
- `src/store/effective.rs:1590` uses immutable recorded origin or exact legacy invitation-creation evidence, without choosing a current member. `src/store/schema.rs:3466` projects the original inviter's clear without adding wake work.
- `tests/store/lazy_inbox.rs:1192` exercises both V1 and V2 producers for Lazy, RequireAck and absent historical fields; output conversion carries the metadata (`src/protocol/output_compact.rs:76`).

### Issues

#### Critical (Must Fix)

- None.

#### Important (Should Fix)

- **Open inviter warnings cease waking after an informational offer.** `src/store/attention.rs:1180`: `wake_seat_attention` gets warnings through `seat_pending_warnings` before calling `wake_warnings`. The unchanged programmatic warning walk at `src/store/attention.rs:931` requires `ordinal > notice_frontier`; committing an offer advances that frontier (`src/store/attention.rs:791`). The new direct origin recipient insertion runs migration25's transition trigger, which removes the warning from legacy open-warning projections (`migrations/0025_warning_notice_delivery.sql:4`). Backlog/legacy judging also explicitly refuses an already projected transition (`src/store/attention.rs:595`). Consequently, after the original inviter receives one committed notice offer, the still-open warning drops out of actual native attention and mod `other_pending` (`src/store/seats.rs:4642`); `warning_wakes_seat` returning true when called directly cannot restore the missing candidate. Native offer gating additionally treats carried warnings as offered (`src/store/wake.rs:175`, `src/store/wake.rs:439`). Add a bounded origin-addressed open-condition wake projection independent of notice offers, and ensure actual native and mod narrowing/offer gating retains it until the condition clears. Preserve existing informational offer settlement, passive observers and passive clears. Add a regression that commits an exact inviter notice offer while the invitation remains pending, then checks actual wake candidate/admission and mod attention before and after explicit acceptance; current tests only check pre-offer waking and post-clear passivity (`tests/store/invitation_due.rs:489`).

#### Minor (Nice to Have)

- None.

### Checks and evidence

- Read the Task6 brief, report and full 114575-byte review package. Initial combined tool output was truncated; recovered the omitted package portions in bounded reads. No source, index or branch mutations; only this report was written.
- Required test-change audit executed `git diff --stat BASE..HEAD -- tests` and `git diff BASE..HEAD -- tests` capped at 400 lines. The latter was deliberately truncated at 400; full hunks were reviewed in the supplied package. Test stat: 8 files, 854 insertions, 26 deletions. The former activation-refusal test is replaced because canonical activation is this task's requirement; the interleaving test removes assertions that arrivals consume the old snapshot, matching the required SQL high-water exclusion. Neither is an unauthorized weakening. No new ignore/skip/deletion found. `tests/store/attention.rs:1696` retains the pre-existing ignored flat benchmark; that file is absent from the changed-test stat.
- Named unchanged-callsite risk: **informational settlement erases open inviter wake**. Checked the pending-warning projection, transition migration trigger, judged-warning exclusion, actual native warning-offer gating and mod warning narrowing (`src/store/attention.rs:595`, `:791`, `:931`; `migrations/0025_warning_notice_delivery.sql:4`; `src/store/wake.rs:175`, `:439`; `src/store/seats.rs:4642`). This establishes the Important finding without a test rerun.
- Named unchanged-callsite risk: **new service episode accidentally cancels or accepts the original Lazy episode**. Checked exact release cancellation and ordinary/required acceptance (`src/store/service_controls.rs:466`; `src/store/control.rs:520`, `:933`): cancellation targets only the retained requirement episode; acceptance updates the explicitly selected invitation and does not rewrite the old Lazy policy.
- Named API/projection risk: **stored Lazy physical timing leaks as an effective deadline**. Checked invitation deadline references in query/effective/seat projections alongside the changed central overdue and native batching paths (`src/store/queries.rs:3947`, `:5036`; `src/store/schema.rs:3607`; `src/store/wake.rs:911`). No additional effective Lazy timer found in those checked paths.
- Existing author validation is recorded in `task-6-report.md`: lifecycle 71 pass/1 pre-existing ignore, metadata 1 pass, output compatibility 31 pass; clippy exit0 in39.50s without diagnostics, fmt/check exit0, no owned persistent children. Some artifact builds exceeded the one-minute budget (66/88s and earlier runs), disclosed there; no comparative baseline establishes an incremental regression from this diff. No Cargo/tests, Herdr operations, full suite, default-feature checks or merge were rerun by this review.

### Assessment

**Spec Compliance:** Needs fixes.

**Task quality:** Needs fixes.

**Reasoning:** Creation, scanner, metadata and service episode handling are coherent and well covered, but the canonical wake candidate path still conflates informational delivery settlement with open-condition waking. Correct that bounded projection and test the post-offer state before approving Task6.
