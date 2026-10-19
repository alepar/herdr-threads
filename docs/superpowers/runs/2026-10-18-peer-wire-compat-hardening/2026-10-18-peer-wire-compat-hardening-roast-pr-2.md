super-roast verdict: clean (0 nits) [low coverage] [review readiness incomplete]
mode: pr        iteration: 2 of 3
profile (assumed): Internal developer tool running on same-user machines. It performs cross-node sync of signed journals between nodes that may run different releases. Mixed-version mesh safety matters: an old peer must keep working against a new one. No external users and no money at stake, so pure cost/observability findings are down-weighted; store-compatibility and wire-compatibility findings are not.
inputs: super-auto/peer-wire-compat-hardening@a597ef4fd vs main@f78f076ab
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 5 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 0 → deduped 0 → panel 0 · spot 0 · promoted 0 · judge completion n/a (no panels) · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 0/0/0/0

Readiness note: all 13 scouts returned (including the new `regression` lane) and raised zero candidates, so no packet reached the judge stage. The engine reports review readiness as coverage unknown, provenance unknown with an empty basis, and marks this an empty late round. Returning scouts and zero candidates do not establish source completeness, so the verdict carries `[low coverage]` and `[review readiness incomplete]`, and `[converged]` is withheld (`convergenceEligible` is false). No inspection counts beyond the engine's own are claimed here.

Prior-report tracking (iteration 1 confirmed findings; each checked against the fix commit between d6fa2d515 and a597ef4fd, not only against the absence of packets):
- [Should-fix] src/sync/resync.rs:488 — resync salvage read a schema-3 backup through `CHAIN_ROWS` and failed on `journal_opaque`. **resolved**: commit 28eae04b4 (ht-j87z.29) probes `sqlite_master` for `journal_opaque` (resync.rs:452) and creates an empty TEMP `journal_opaque` on the read-only backup connection (resync.rs:459) so the chain-row reader runs journal-only; tests at resync.rs:1212 and 1243 salvage a stripped schema-3 backup.
- [Should-fix] src/sync/coordinator/serve.rs:265 — parking a log in `on_served` returned without `push_link`, stalling the link's other armed logs. **resolved**: commit 7085706b0 (ht-j87z.30) calls `self.push_link(link)` right after `entry.withheld_logs.insert(log, from)` (serve.rs:272-275).
- [Should-fix] src/store/journal.rs:1962; migrations/node/0004_journal_opaque.sql:19,21 — frozen delete-on-promote surface (`journal_opaque_promoting`, exemption trigger, dead `promote_opaque`/`unpark`). **resolved**: commit dd3dbe1ee (ht-j87z.31). The 0004 delta now has an unconditional `journal_opaque_append_only` BEFORE DELETE trigger, no `journal_opaque_promoting` table, and a header that defers promoted storage to ht-evlm; `git grep` finds no `promote_opaque`, `unpark` or `journal_opaque_promoting` in src/ or migrations/ (the only remaining mention is tests/store/schema.rs:6139 asserting the table is absent).
- [Nit] src/store/journal.rs:206 — `verify_invariants` seeked `MAX(counter)` over the `CHAIN_ROWS` UNION ALL. **resolved**: commit 4c1ad669a (ht-j87z.32) takes `MAX(counter)` separately from `journal` and from `journal_opaque WHERE branch=0` and combines with `MAX(COALESCE(..),COALESCE(..))` (journal.rs:206-207); the head-hash lookup still reads `CHAIN_ROWS` but is an equality seek on `(log_ix, counter)`, which both halves index.
- [Nit] src/sync/coordinator.rs:2341; src/sync/coordinator/serve.rs:271; src/sync/coordinator.rs:5256 — `withheld` counted parked logs while docs said entries. **resolved**: commit 6bb2878bf (ht-j87z.33) renames the field to `withheld_logs` (coordinator.rs:514, sync_admin.rs:171-172 with `#[serde(alias = "withheld")]` for old readers), documents it as logs parked on the link (serve.rs:22, docs/operations.md:311, CHANGELOG.md:5), and adds `withheld_logs_counts_parked_logs_not_entries` (coordinator.rs:5271) asserting 2 for two parked logs.

## Confirmed findings            ← consumed by super-design, one task per finding
- none

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- none this iteration. The iteration 1 rejections (node delta version 4 clash with v2.2.0's 0004_sibling_delivery, full local heads to non-cap peers, discarded `insert_opaque` result, `handle_restore` paused own log, oversize own-log `AheadOfSelf`, gated branch reply penalty, mesh-wide opaque stall, R4 removals-only print, MANIFEST-pinned fixtures, journal signature domain, double parse in `WireItem::from_json`, missing `source_node`/`received_at` on branch rows, spaced `blocked=` token, in-memory oversize stall) stand unchanged; no scout re-surfaced any of them. Caller note from iteration 1 remains in force: the rebase onto main, 0004 file rename, `NODE_DELTAS` renumber, test start-point fix and release.md 8b wording update are mandatory before merge.

## Unverified nits (spot-checked)
- none this iteration (no spot checks ran). The iteration 1 spot-confirmed nits were not re-raised; the ones touched by this round's fixes are the 0004 header comment (removed with the promotion surface, ht-j87z.31) and the CHANGELOG "byte-identical traffic" wording (CHANGELOG.md:5 still says "2.1.1 peers see byte-identical traffic", so that nit stands as written in iteration 1). The others (393216 duplicated limit, `authenticate_oversize` duplication, vacuous relay assertions, duplicate reader/writer pairs, `IngestOutcome::Oversize` on the port contract, `WireItemError::TooLarge`, public `advertise_opaque` knob, `// cap: OPAQUE_CAP` comment, inbound `branch_entries` round trip) were not re-examined this round and remain as reported there.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none