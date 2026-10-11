# Whole-epic review — ht-akx

CLEAN

Reviewed base `effa14a2d6e6ccd406ec020230003f44272ce17a` through head `fe464af5abcfc75d7c15ad2ec354c77e6353c5f8` against the binding thread-search-discovery design. Read the ledger first and all three task reports. The two deferred nice-priority warnings are environment noise; no parked finding or authorization-refused scope remains. Historical shared-cache results are excluded from acceptance evidence.

## Strengths

- `src/store/queries.rs:1626`: filtered ordinal/recent opaque keys add the selected-instance name revision while retaining lifecycle, membership, activity and topic dependencies. Missing rows map to zero; legacy filtered keys mismatch and receive normal restart guidance. Unfiltered key spellings are preserved.
- `src/store/queries.rs:1725`: all ordinal/recent candidate projections include nullable name. The literal name OR topic predicate runs inside the existing bounded loop before summary admission, preserving index traversal, high waters, candidate accounting, deduplication, byte fitting and zero-match continuations.
- `src/store/control.rs:1357`: the dedicated directory/name/all revision publishes in the existing accountable transaction and actual-change branch. Production mutation dispatch reaches this setter; replay/no-op behavior remains intact. Repository-wide writer inventory confirms that creation only inserts new IDs and managed ensure returns existing threads unchanged or inserts without a name.
- Public flag help, README, legacy wire-field comment and adopted-contract amendments consistently describe the intentional name-or-topic expansion. No schema, authority, picker or message-search scope expansion was introduced.

## Issues

Critical: none. Important: none. Minor product findings: none.

Process artifacts have two trailing blank-line warnings (`task-1-brief.md:35`, `task-2-brief.md:32`) from range-wide diff checking; these do not affect the implementation or its contract. The substantive source/test/public-document diff passes whitespace checking.

## Test changes check

Ran `git diff --stat effa14a2..fe464af5 -- tests`: five files, 544 insertions and 3 deletions. Ran the full test diff capped at 400 lines; the diff contains 615 lines, so the initial display was truncated. Read the remaining complete `tests/store/queries.rs` diff separately. No changed inline test modules exist in the source diff.

No assertion was unjustifiably loosened, deleted or skipped. The three replaced parser lines strengthen the same continuation test with recent ordering and literal punctuation. New coverage exercises real store queries, cooperative-permit production mutations and an isolated actual CLI invocation. It covers both traversal orders, name/topic matching, nullable names, case/UTF-8/literal punctuation, membership/archive/instance scopes, empty work-page continuation, rename/set/clear invalidation, replay/no-op stability, existing unfiltered semantics, unrelated membership changes, legacy filtered keys and generated argv/wire compatibility. The refreshed-main invitation-rejection implementation and tests remain intact in the reviewed range.

## Verification evidence and limitations

Authoritative combined-source evidence is task 3's private build at source `65a67dcf`; later changes through reviewed head are documentation/process artifacts. Its report records directory42, CLI1, rejection16, three contract checks, formatting, all-target/all-feature clippy, default-feature guard and final process-leak check passing. Inspected directory, CLI, rejection and clippy logs directly; counts and private source path agree. Prior private leaf evidence additionally covers name24/recent24. No tests were rerun because review identified no unanswered behavior doubt requiring a duplicate execution.

The parent-owned post-roast branch sweep remains pending. This review does not claim a full suite or completed branch sweep. Concurrent controller changes in durable run/progress artifacts were observed and left untouched.

## Assessment

Spec alignment and code quality: satisfactory. The approved directory-kind/name/all-key adjustment matches schema and helper validation and avoids a migration while preserving the intended independent revision dependency. No confirmed blocking defect or meaningful testing gap was found.

Ready for the parent's remaining finish gates: yes. Merge readiness still depends on its explicitly deferred post-roast sweep; this is a clean code review, not a substitute for that gate.
