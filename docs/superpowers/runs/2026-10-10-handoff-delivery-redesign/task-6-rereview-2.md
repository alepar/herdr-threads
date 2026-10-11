Spec CLEAN
Quality CLEAN
I1 ADDRESSED

- `src/store/attention.rs:1077` adds the missing seventh existence probe pinned to `warning_conditions_invitation_origin`. Its canonical seats-PK instance lookup and frozen origin seat predicate match the partial index; it tests only still-open invitation conditions independently of the informational frontier. No current-member attribution or new migration is introduced.
- The actual upstream discovery gate now reaches this source: `src/store/mod.rs:1121` calls `seat_has_pending_rows`, which iterates all seven probes (`src/store/attention.rs:1086`), then invokes canonical wake attention and load_candidate (`src/store/mod.rs:1126`). The round1 bounded open-origin merge and offer bypass remain intact (`src/store/attention.rs:1206`; `src/store/wake.rs:197`), and mod fingerprints still use the shared wake-warning function (`src/store/seats.rs:4642`). Existing seat live/human filtering, continuity and admission checks remain in the same path.
- `tests/store/control.rs:10826` drains real warning-attribution work and asserts completion, then retains exact committed informational settlement and an empty informational pending set. `tests/store/control.rs:10894` invokes actual `discover_wake_candidates` before direct attention/load/reserve assertions and requires the departed, otherwise-idle original inviter as an actionable candidate. `tests/store/control.rs:10985` proves late explicit acceptance removes that candidate while the fresh clear remains passively deliverable. This closes the precise upstream shortcut missed in round1; original native/mod assertions remain present.
- `tests/store/wake_discovery_cost.rs:221` includes the new pinned probe in the nine-path inventory (seven pending sources, live seat walk, human exclusion). The existing query-plan pin/usage assertions and flat-cost budgets remain intact. Author EXPLAIN evidence reports the origin index search by instance/seat plus seats primary-key scalar lookup, without sorting or a retained-history scan.

Critical: None.
Important: None.
Minor: None.

- Mandatory fix-range test audit: `git diff --stat 50c397f4..19afa478 -- tests` and test diff preview capped400lines completed. Two files,70 insertions/6 deletions; preview fit within400lines without truncation. Removed lines only adapt the helper signature/calls and inventory count/comment to the additional path. No weakened assertion, deleted test, new skip/ignore or increased cost budget.
- Read the full11842-byte fix package once, prior I1/rereview and updated author report; Task6 brief constraints remain binding. Named outside-diff risk **upstream discovery bypasses the new source**: focused trace of `src/store/mod.rs:1121` through seven-probe dispatch, canonical attention and load_candidate confirms it is reachable. Named outside-diff risk **direct/mod offer gating still suppresses an already delivered open condition**: focused check of `src/store/attention.rs:1155`, `src/store/wake.rs:175` and `src/store/seats.rs:4642` confirms the round1 independent source and bypass remain used. No broader source crawl or runtime rerun.
- Author evidence distinguishes the initial test compile error from the meaningful discovery RED, then records16selected/16passed/0failed/0ignored: three post-offer regressions, three discovery cost tests, and ten existing draft, idle/working/blocked, human, host-unavailable pre-submit and recovery-hold guard checks. Recovered prior command evidence was not rerun. Final required lint exits0 in32.62s with zero diagnostics; fmt clean. No new process, config, thread, Cargo or test actions were performed by this reviewer.
- Scoped approval covers I1 and this fix range. Migration renumbering/schema seam review, final integration gates and downstream Task7/8/10 obligations remain future coordinator work. This review makes no global Task7 receipt-warning or shared-driver guarantee.

Spec CLEAN
Quality CLEAN
I1 ADDRESSED
