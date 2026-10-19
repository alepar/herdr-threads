---
super-roast verdict: Nit (1 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 2 of 3
profile (assumed): Internal developer tool that runs on same-user machines. It performs cross-node sync of signed journals across a mixed-version mesh, so mixed-version safety matters. Store migrations run on real user data, so data loss is a floor. No external users, no network-exposed surface beyond cooperative peers.
inputs: super-auto/bytes-first-journal@5cae6c302 vs main@e6815f803
delta vs prior: 1 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 6 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs, dependency) · raw 1 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 1/1/1/0 · dependency 0/0/0/0

Readiness note: the engine supplied no review-readiness basis (coverage unknown, provenance unknown). Stage coverage itself is complete (14/14 scouts, 100% judge completion, no seat lost, no panel cap). The `[low coverage]` qualifier is carried for readiness-incompleteness only. `[converged]` is not set because the engine marked this round convergence-ineligible (readiness not affirmed); no Blocking of any provenance is confirmed this round.

Prior-report tracking (iteration 1 confirmed findings, none re-surfaced by this round's packets; fix commits in 479c7376e..5cae6c302):
- [Should-fix] CHANGELOG.md:5 — resolved (3f2d1f81b "Docs: unreleased changelog section, 'before 2.3.0' wording").
- [Should-fix] src/store/journal.rs:2334 promote_parked full scan — resolved (3b349325a "Fix: promote_parked indexed and no-op without registered layouts").
- [Should-fix] src/store/incarnations.rs:401 fork candidate never ready — resolved (8d8e633ec "Fix: opaque forked predecessor is a terminal cut for re-issue"). The chosen remedy introduces this round's one new finding (doc sync), which is a new entry, not a regression: the pass now drains and `pending` reaches 0 as asked.
- [Nit] src/sync/envelope.rs:888 UNDERSTOOD registry race — resolved (3ad326546, 109943464, 0601a91fc "serialize the understood-layout registry" / "hold the layout registry").
- [Nit] docs/operations.md:311 "2.1.1 and older" wording — resolved (3f2d1f81b).
- [FYI] src/store/schema.rs:395 user_version check-then-act — resolved (21274cd6c "Fix: re-check user_version inside the delta transaction").

## Confirmed findings            ← consumed by super-design, one task per finding
- [Nit] src/store/incarnations.rs:275 — Fix commit 8d8e633ec adds a new accepted limit to the N7 fork-repair invariant: known (HTJRNL01) fork-branch facts chained on an opaque `journal_forked` row are never re-issued and are counted as `cannot_reissue`, but TRUST-POLICY.md, docs/operations.md and the CHANGELOG still describe `cannot_reissue` as covering only foreign-layout or opaque entries. (new this iteration; introduced by the fix for prior finding incarnations.rs:401) [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `git show --stat 8d8e633ec` touches only src/store/incarnations.rs and tests/store/incarnations.rs. Its `fork_cut_cte` doc comment: known descendants of an opaque `journal_forked` row "could only follow a re-issued predecessor, which never exists. All count as `cannot_reissue`; none is a candidate or pending." TRUST-POLICY.md N7 (~line 1424) still promises the owner re-issues "every fact past the frozen counter from all branches ... so every node ends with the union of facts". The only related accepted limit, TRUST-POLICY.md:2035, says "Re-issue never copies a foreign-layout entry, promoted or not (`cannot_reissue`)" and says nothing about understood entries being dropped. docs/operations.md:315: "A node cannot re-issue an own-log opaque entry it does not understand. The repair row shows ` cannot_reissue=<n>`". The CHANGELOG Unreleased bullet describes the counter the same way. Grep of TRUST-POLICY.md and docs/operations.md for "chained on", "fork_cut", "terminal cut" finds nothing. AGENTS.md Trust policy: "A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit." All three seats CONFIRM at Nit: the limit is narrow (an opaque forked row with known descendants), the code and commit message state it, and no data loss occurs beyond the deliberate cut. Distinct from the prior rejected 0005_after.sql:16 same-commit-rule finding: that rejection rested on no invariant being weakened, which is not the case here.
  fix-shape hint: Add one sentence to the TRUST-POLICY.md:2035 accepted-limit block (and a matching clause at operations.md:315 and the CHANGELOG bullet) stating that known fork-branch facts whose predecessor on the branch is an opaque forked row are also counted under `cannot_reissue` and are left out of the re-issued union; no code change.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- none this iteration. The iteration-1 rejections (see the prior report: coordinator.rs:1724 Have frames, journal.rs:888 StoreCorrupt abort, schema.rs:406 no free-space preflight, apply.rs:915 mark_opaque_heads, 0005 column order, journal.rs:1122 KNOWN_SQL, journal.rs:764 INNER JOIN, sync_status.rs:451 J3 raw hash, journal.rs:355 promotion budget, journal.rs:2279 content-to-signature binding, lifecycle.rs:101 startup latency, ingest.rs:169 authenticate_oversize, 0005_after.sql:16 same-commit rule) all stand; none was re-surfaced.

## Unverified nits (spot-checked)
- none this iteration (spot 0). The iteration-1 spot-confirmed nits were not re-surfaced and are not tracked as resolved here; they remain advisory.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---