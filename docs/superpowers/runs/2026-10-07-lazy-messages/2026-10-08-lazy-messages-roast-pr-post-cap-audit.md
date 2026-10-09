super-roast verdict: Should-fix (6 confirmed) [converged]
mode: pr        iteration: post-cap audit
profile (assumed): Durable local coordination software runs in real developer sessions. Trust is cooperative and same-user, with canonical daemon authority and honest receipt and actor provenance; real data loss, migration corruption and forced attention violations matter, and no network-adversarial caller model is assumed. Main owns the final integrated suite and release/install; this is isolated source review.
inputs: super-auto/lazy-messages@b22e7d1abdd0661520dece8b2f90e8d69a1f7e41 vs main@4f7cad2ddadf0f3e9bf917a36917624821b3be77; separately authorized post-cap follow-up
delta vs prior: 3 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 3 resolved · 0 regressed (0 Blocking) · 3 punch-listed (open)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 3 → deduped 3 → panel 3 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 3 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=3) · reproduce 3/0/0 · refute 3/0/0 · ground 3/0/0
lane-yield (found/confirmed/unique/refuted): correctness 1/1/1/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 1/1/1/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 1/1/1/0

## Confirmed findings
_Count includes 3 new findings from this round and 3 prior confirmed findings that remain punch-listed (open); the panel verified 3 findings this round._
- [Should-fix] src/cli/output.rs:335 — Text history and body reads fail with NotFound when a selected result contains a published unavailable-recipient warning represented by its send manifest rather than a physical messages row. (new) [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: Source inspection shows ReadModes::lookup includes all selected history and message IDs (src/cli/output.rs:334-336), while recorded_mode searches physical messages only (src/store/lazy_delivery.rs:47-54). Published warnings can be canonical without a physical row (src/store/effective.rs:1566-1580), and the new metadata handler maps the missing mode to NotFound (src/store/queries.rs:242-244), which the text lookup propagates (:357-370) before output (src/cli/mod.rs:941,1053; src/cli/follow.rs:723-742). The existing logical-warning test was read, not run; no executable reproduction or test run is claimed.
  fix-shape hint: Restrict mode lookup to ordinary content, or make it recognize canonical logical warning IDs.

- [Should-fix] src/cli/mod.rs:940 — Legacy human inbox pages can fail with InvalidBudget because fitting runs before the topic column is added. (new) [lanes: regression] [fix-regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: fit_inbox_read measures the page before topics are fetched and installed (src/cli/mod.rs:940,954-968). Human rendering adds a padded topic column (src/cli/human.rs:328-372), while the final output check rejects an oversized human Inbox before writing (src/cli/output.rs:295-305), without selecting a smaller page. The seats supplied source-derived byte examples, not an executed application reproduction; no test run is claimed.
  fix-shape hint: Include the same topic context in fitting that final human rendering uses.

- [Should-fix] src/cli/commands.rs:1823 — The mandatory Human invocation route makes the shipped README quickstart fail at initialization and its later handoff/send steps, while corresponding test commands use the new syntax. (new) [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: README.md:31 still says `herdr-threads me init`; the parser requires Human for MeInit (src/cli/commands.rs:1810-1824), and actor routing selects Human only when argv[1] is `human` (src/cli/actor_route.rs:23-33). After correcting init, documented root handoff/send still conflict with the Human context (src/cli/mod.rs:1714-1733,2111-2138). The updated tryout uses the Human prefix (tests/integration/readme_tryout.rs:349,356,374,486). This is source proof, not an executed walkthrough.
  fix-shape hint: Update the README quickstart commands to use the Human invocation route throughout.

- [Should-fix] src/protocol/output_compact.rs:77; src/protocol/results.rs:896 — Missing invitation goal payload. (punch-listed (open), see iteration 1)
  evidence: The prior report lists this confirmed finding as deliberately left unfixed by the caller; the current packets do not re-surface it. It remains open by caller punch-list status.

- [Should-fix] src/cli/follow.rs:813 — Serial per-message delivery-mode RPCs. (punch-listed (open), see iteration 1)
  evidence: The prior report lists this confirmed finding as deliberately left unfixed by the caller; the current packets do not re-surface it. It remains open by caller punch-list status.

- [Nit] src/cli/journal.rs:688 — Retained abandoned lazy display proofs and allocation scan growth. (punch-listed (open), see iteration 1)
  evidence: The prior report lists this confirmed finding as deliberately left unfixed by the caller; the current packets do not re-surface it. It remains open by caller punch-list status.

Prior confirmed status: the iteration 2 Blocking finding at src/store/queries.rs:5720 (lazy arrivals add recovery summary work), Should-fix finding at src/cli/mod.rs:908 (legacy inbox cursor upgraded to v2), and Should-fix finding at src/cli/commands.rs:2191 (embedded daily-loop guide contradicts lazy reply notification behavior) are resolved. The guide correction is independently reviewed in .superpowers/sdd/post-cap-follow-up-plan/task-3-review.md and task-3-report.md; the current compiled guide teaches the --nudge recipe. None was re-surfaced by current scouts or panels.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [FYI] src/store/schema.rs:513 — Opening an existing store raises its schema version to 26, preventing binary-only rollback to the exact base daemon against the same store. (previously rejected; rejection preserved)
  verdict: rejected in iteration 1 (reproduce REJECT / refute CONFIRM / ground REJECT); not re-judged this round.
  reason: Prior reproduce and ground evidence established the expected forward-only migration boundary, not a violated rollback requirement. The approved design requires additive migration26; docs/operations.md:151 documents forward-only startup migrations. No current packet supplies materially changed evidence, data loss or failed migration.
  pre-existing on base: src/store/schema.rs:403-408,482-510; src/store/connection.rs:125-130.
  dissent: The prior refute seat confirmed the factual binary-only rollback limit at FYI and explicitly called it pre-existing. The majority addressed that evidence; the rejection stands.

## Unverified nits (spot-checked)
- [FYI] src/cli/journal.rs:697 — record_lazy_displayed_chunk duplicates the existing journal's durable contiguous-display algorithm and file transaction instead of sharing their implementation. (prior spot history retained; not re-judged this round)
  spot outcome: REJECT in iteration 1; retained as an unverified-nit entry, not promoted to panel verification.
  reason: The prior refute evidence at src/cli/journal.rs:742-747,995-1003 identifies duplication but no incorrect advancement, missing durability barrier or failed recovery. Full-claim identity, different read caps and the ordinary complete-body shortcut are semantic differences. Shared implementation remains optional refactoring; the approved reuse requirement concerns send preparation and publication.

## Escalations (need human)
- none
