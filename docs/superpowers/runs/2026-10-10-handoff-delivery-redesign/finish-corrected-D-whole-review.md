# Corrected D cumulative whole review

**Spec: PASS for inactive production-source readiness. Quality: FAIL for cumulative activation-test readiness because of Important D-WHOLE-4 (I4).** No new blocking production defect was found. The inactive production implementation is ready for the separately controlled narrow activation stage once the prepared-test correction is reviewed and integrated. This is not activation completion or merge clearance.

## Pin, scope and independence

Reviewed base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218` through HEAD `31e5879781904b9ec20ee5ea3cec7517856260f3`, tree `f165279e4acd8c43c3edc7cb0e22ab2e9bb08f34`, at `.worktrees/finish-corrected-D-review`. Read-only review: no source/index/HEAD writes, Cargo/native execution, tracker/Herdr writes or subprocess test children. Only the two review artifacts were written for this initial review.

All **80 cumulative paths** are assigned in `corrected-D-whole-coverage.json`. The actual Git cumulative path set, HEAD, tree and every current blob were checked against the inventory. All **61 unchanged paths** match both frozen-4c Git blobs and that audit's file SHA-256 values. Their exact earlier review levels/notes and provenance are retained, rather than relabeled as a fresh exhaustive audit. All **19 changed paths** received fresh semantic source/diff or changed-test-body review with their interactions. Large test files retain the previous depth for unchanged/moved fixture code.

The actual bodies of all **14 restored attention tests** were read directly in `tests/store/attention.rs`, including membership highs, exact witnesses, canonical/legacy precedence, fallback progress, stale candidates, receipt sweeps, overflow rollback, unavailable episodes and corruption. The additional actual thirteen-source SQL/VM matrix (`:3020`) and due-key matrix (`:3604`) were also read. This is body-level inspection, not an inference from test names or earlier summaries.

Normative inputs included AGENTS/TRUST, workspace constraints, ROOT's finish plan and main/four subsystem designs, accepted bounded dispositions, relevant later scope/reset/framing refinements, and ROOT's newer capacity design/reviews. The prior frozen-4c whole review/coverage and current capacity, Watch and leak-scope rereviews were consulted. The newer ROOT bounded cache/reclamation limits govern this assessment even where D's historical design prose is older. Accepted topic-only continuation for standalone unrenderable ordinary output is not reopened without new evidence.

## Finding

### D-WHOLE-4 / I4 — Important: lazy-only activation test waits for an impossible Confirm

`tests/integration/mod_delivery.rs:2559` waits for `delivery_presentation_confirm_v2`, then uses that Confirm to locate the lazy-only page and assert `confirmed` through `:2579`. A pure ordinary/lazy message page has no frozen nonordinary descriptors. The store creates an outbox token only when descriptors exist (`src/store/invitation_content.rs:2405`, assignment at `:2437`), and the actual Watch consumer sends Confirm only when the page has a token (`src/cli/watch.rs:671`). The correct lazy-only flow therefore cannot satisfy this wait after advertisement is enabled.

Current inactive execution fails earlier at capability discovery, so that RED does not expose or excuse this additional assertion defect. Change the test to identify the actual Pull page containing the lazy item, assert absent token/empty controls and no corresponding Confirm, and retain the passive delivery/no-ACK/no-nudge/channel-health assertions. Do not change production to create a token merely to satisfy this test. The author has a separate pending narrow correction; it is outside this pinned review.

No Critical findings. No additional Important production findings identified.

## Earlier whole-review findings

- **D-WHOLE-1 addressed:** terminal offer reclamation is bounded and conservative. Store-side `reclaim_terminal_offers` (`src/store/invitation_content.rs:1663`) uses canonical ACK/retirement/waiver or published lazy completion; missing keys are retained. Service applies the exact reclaimed delta to the in-memory ledger (`src/service/delivery_presentation.rs:213`). Real 256/257-message tests cover ACK, explicit/manual ACK and lazy completion releasing capacity. Deferred/consumed caches are bounded and fenced by incarnation, generation, native session, exact offer stamps, eligibility/passive versions and render key.
- **D-WHOLE-2 addressed:** the three private carry/state ID readers validate borrowed text length (128-byte cap) and fallible ID parsing before effects; adjacent cleanup also parses. Relevant code is in `src/store/invitation_content.rs` around `:1148`, `:1183`, `:1229` and `:2665`. Changed service tests cover all readers, malformed classes, wrong types, boundary size and table snapshots on refusal. This is not adversarial caller verification.
- **D-WHOLE-3 addressed:** `tests/service/delivery_pull.rs:2425` now builds the current render-selector request, checks structured confirmation status/control subsets and exercises exact-token restart replay while proving no invitation acceptance or ordinary receipt transition.

The independently inspected successor-sweep log names the affected passing tests supporting these closures. That does not extend the evidence to every cumulative path or active consumer.

## Source and test assessment

The capacity implementation preserves the intended separation between normal continuation and due-only work at a full offer window. It bounds keyed eligibility, attempts and hydration, carries fixed-size suppression state, fences stale application and keeps failed or partial output from falsely advancing an offer. Real-body tests cover cache convergence with missing keys, render failures, partial retained carry, zero remaining capacity, post-reclamation progress and a reachable fourth offer count. Reclamation is based on positive canonical terminal evidence; neither absent rows nor ledger age authorize removal.

The new SQL tests trace actual executed statements and compare work across materially larger irrelevant histories, rather than relying only on hand-written EXPLAIN stand-ins. Restored relevance tests again exercise concrete mutations and rollback boundaries. Migration tests use real file-backed v39/v40 close/reopen flows, preserve old table values/catalog properties and test late failure rollback/retry.

Prepared Watch/native-hook tests exercise elected capabilities and real output/confirmation boundaries. Relay attention bookkeeping records successful physical forwarding before causal pull assertions, and helper teardown releases blocked work and owned sockets. These are useful preparations, but preparation is not successful active execution. D-WHOLE-4 must be fixed before treating the prepared Watch suite as activation-ready.

## Execution evidence and outstanding gates

Independently read `/private/tmp/ht-finish-capacity-successor-sweep.log`: **127 passed, 0 failed, 4425 skipped**, two binaries, runtime **8.847 s**, nextest internal run ID `c02192e1-5457-45a1-a92f-d819a4a099e7`. SHA-256: `b24cd7eb1fbc87fad2e086dd30035e6ce83dec0eae3d6800408d1ad69010eb03`. The preceding compilation was about 87 seconds; it is not a controlled incremental timing measurement. This log covers the affected capacity/successor scope only.

Parent-reported evidence, not independently executed or log-audited here: scoped leaks zero under HT_LEAK_RUN_ID 6fe9d557-d954-40c9-83e4-688cab992cd7 (distinct from nextest’s internal run ID; W/capacity-successor-evidence.json) and neutral check PASS; broad strict clippy PASS at **89 s** (above the 60-second budget), followed by a controlled one-source-touch incremental strict-clippy PASS at **21.89 s**; default-feature merge guard PASS at **20.40 s**. Preserve both timing records. The controlled run addresses the incremental concern without rewriting the broad run as under budget.

Parent also reports all five elected Watch activation tests RED at the real missing advertisement (`/private/tmp/ht-finish-watch-actual-inactive.log`, 0.547 s, scoped leaks zero), and four AGY tests similarly RED at real discovery. These are meaningful inactive-baseline failures, not active-path successes. D still intentionally omits `DELIVERY_PRESENTATION_V2` from `ADVERTISED`; the capability-set assertion and configured production-route test likewise require the separate activation change.

Before completion/merge: integrate and verify the narrow D-WHOLE-4 correction; perform the separately reviewed advertisement change; run the real elected Watch/native AGY and configured-route assertions against that exact active source; obtain the required actual production-consumer evidence via a captured stream; and complete the plan's full affected-path inventory and final checks. The 127-test scope is not an all-D test pass. Do not routinely substitute a full-suite run for the required focused evidence while the repository's flakiness restriction remains in force.

**Review status: COMPLETE + QUIESCENT.** No live processes were started. The pinned source is unchanged. The cumulative quality finding remains open at this HEAD; any later scoped correction must carry its own exact delta and verdict.

