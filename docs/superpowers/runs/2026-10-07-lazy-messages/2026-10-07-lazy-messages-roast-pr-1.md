super-roast verdict: Blocking (5 confirmed)
mode: pr        iteration: 1 of 3
profile (assumed): Durable local coordination software used on real developer sessions. Trust is cooperative and same-user, with canonical daemon authority and honest receipt and actor provenance. Real data loss, migration corruption and forced attention violations matter; no network-adversarial caller model is assumed. Main owns the final integrated suite and release/install; this is isolated source review.
inputs: super-auto/lazy-messages@72a2ba6aa5df04f14870a0661aa16213efa1df78 vs main@4f7cad2ddadf0f3e9bf917a36917624821b3be77
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 10 → deduped 7 → panel 6 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 6 · rr 0.50 · rg 0.67 · fg 0.83 · unanimous 0.50 · ground-loo 1.00 (n=3) · reproduce 3/3/0 · refute 6/0/0 · ground 5/1/0
lane-yield (found/confirmed/unique/refuted): correctness 3/3/0/0 · security 0/0/0/0 · premortem 1/1/1/0 · simplicity-design 2/1/0/1 · hot-path-perf 1/1/1/0 · concurrency-async 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 1/0/0/1 · api-contract 2/2/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 0/0/0/0

## Confirmed findings
- [Blocking] src/store/queries.rs:5959 — V2 inbox continuations omit explicit-seat and machine read-only selectors, so following them can mutate the caller’s inbox or fail against a different seat. [lanes: correctness, api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm that run_wire upgrades passive reads at src/cli/mod.rs:885-891, while inbox_batch_argv at src/store/queries.rs:4880-4886 drops both selectors. Following an own-seat text continuation enters run_display_inbox at src/cli/mod.rs:1051-1077 and settles complete lazy deliveries or ordinary ACKs. A foreign-seat continuation instead fails the cursor seat comparison at src/protocol/pagination.rs:231-243. JSON format alone remains read-only, but does not preserve foreign-seat selection.
  severity floor: Settling a traversal requested as read-only violates the lazy-delivery boundary stated in docs/superpowers/specs/2026-10-07-lazy-messages-design.md:29 and TRUST-POLICY.md:364-365. Preserving explicit read-only discovery without settlement is a core purpose of this artifact; the Blocking floor overrides the seats’ Should-fix labels.
  fix-shape hint: Carry seat selection and read-only presentation through every generated continuation, and verify following those commands preserves both.

- [Should-fix] src/cli/mod.rs:2057; src/store/queries.rs:5959 — Paginated human inbox continuations omit the required immediate human namespace, preventing the originating human caller from completing the displayed body. [lanes: correctness, api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm that page_v2 builds a root inbox command through src/store/queries.rs:4880-4886. src/cli/actor_route.rs:24-34 requires immediate argv[1]=human, so following the supplied command selects Agent and src/cli/mod.rs:2057-2059 rejects the existing Human context before the next chunk. The approved design at lines 29-33 supports bounded body continuations and human own-text settlement. Manually inserting human works; the supplied command does not.
  fix-shape hint: Preserve the invocation actor in continuation context and insert human immediately after the executable before routing flags.

- [Should-fix] src/protocol/output_compact.rs:77; src/protocol/results.rs:896 — The v2 invitation representation omits canonical goal data, causing the default text inbox to report available goals as unavailable. [lanes: correctness, simplicity-design]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm that InboxBatchV2Item::Invitation has no goal_data at src/protocol/results.rs:896-901. The scan loads only topic at src/store/queries.rs:6138-6143, and the compact adapter supplies goal_data: None at src/protocol/output_compact.rs:77. Even a short canonical goal with ample output budget renders as unavailable at lines 178-190. Base src/store/queries.rs:5108-5119 carried the goal, omitting it only for output fitting. The approved design at line 31 requires preserving existing invitation handling. Canonical goal data remains accessible through thread show.
  fix-shape hint: Preserve the existing invitation goal payload in v2 and apply bounded fitting instead of unconditional omission.

- [Should-fix] src/cli/follow.rs:813 — Human follow adds one serial delivery-mode RPC per displayed message instead of batching the IDs from each fetched history page. [lanes: hot-path-perf]
  verdict: confirmed (reproduce ✗ — REJECT / refute ✗-survived / ground ✓)
  evidence: Refute and ground confirm singleton History construction at src/cli/follow.rs:802-818 and synchronous MessageDeliveryModes lookup at src/cli/output.rs:203-242. Opening and live loops at follow.rs:1146-1153 and 1186-1198 call this separately for each message. A supported 100-record page therefore adds 100 serial metadata exchanges, including ordinary messages. Each LocalSocketClient::call creates a runtime and opens an exchange at src/client/local.rs:267-272 and 173-179. The same diff batches full history pages at follow.rs:722-727.
  dissent: Reproduce agrees the introduced 100-call amplification exists, but rejects because design.md:47 bounds each request rather than promising follow latency or one request per page. Refute and ground address that distinction: no measured latency violation is asserted; the demonstrated avoidable serial amplification occurs repeatedly on the existing user-facing page boundary. The dissent does not identify an unresolved factual premise.
  fix-shape hint: Fetch delivery modes once per fetched history page and pass those annotations to the per-message renderer.

- [Nit] src/cli/journal.rs:688 — Abandoned lazy display proofs accumulate permanently across binding changes, growing the shared intent directory and the directory scans performed by every subsequent journal allocation. [lanes: premortem]
  verdict: confirmed (reproduce ✗ — REJECT / refute ✗-survived / ground ✓)
  evidence: Refute and ground confirm that src/cli/journal.rs:688-768 persists proofs keyed by the complete claim and message, including partial displays. A partial display creates no completion intent at src/cli/lazy_display.rs:73-83 and 135-138. Successful successor settlement removes only the successor’s proof through src/cli/retry.rs:449-465. No production sweeper or count/age bound was found. Journal::counter at src/cli/journal.rs:880-892 scans all retained entries while allocation holds the shared lock at lines 1199-1213.
  dissent: Reproduce agrees obsolete files persist, but notes that design.md:35,39 requires claim invalidation and successful-set cleanup, not physical deletion of old hints. Refute and ground address this: the claim concerns additional retained files and scan work, not incorrect identity reuse. The ordinary base implementation has analogous partial-proof retention; the new lazy namespace adds another source and also persists complete first chunks.
  severity: The refute seat assigns Nit because only retained files and extra scans are established. The local same-user profile and nonauthoritative client-hint impact support Nit rather than the ground seat’s Should-fix; no delivery error, lost canonical data or significant latency regression is demonstrated.
  fix-shape hint: Add bounded cleanup for abandoned claim-specific lazy proofs while preserving proofs required by outstanding frozen completion intents.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [FYI] src/store/schema.rs:513 — Opening an existing store with this build permanently raises its schema version to 26, preventing binary-only rollback to the exact base daemon against the same store.
  verdict: rejected (reproduce REJECT / refute CONFIRM / ground REJECT).
  reason: Reproduce and ground confirm the version boundary but show no violated rollback requirement. The approved design expressly requires an additive migration26. Base src/store/schema.rs:482-510 already uses committed forward-only startup migrations and rejects future schemas at lines 403-408; docs/operations.md:151 documents that convention. No data loss or failed migration is established.
  pre-existing on base: src/store/schema.rs:403-408,482-510; src/store/connection.rs:125-130.
  dissent: Refute confirms the factual binary-only rollback limitation at FYI and explicitly calls it pre-existing. The majority addresses that same evidence and rejects treating the expected migration boundary as a new defect. No new downgrade guarantee is inferred.

## Unverified nits (spot-checked)
- [FYI] src/cli/journal.rs:697 — record_lazy_displayed_chunk duplicates the existing journal’s durable contiguous-display algorithm and file transaction instead of sharing their implementation.
  spot outcome: REJECT; this remains an unverified-nit entry under the fixed spot-tier route.
  reason: The refute seat confirms duplication at src/cli/journal.rs:742-747 and 995-1003, but identifies no wrong advancement, missing durability barrier or failed recovery. Full-claim identity, different read caps and the ordinary complete-body shortcut are real semantic differences. Shared implementation is an optional refactoring; the approved reuse requirement concerns send preparation and publication, not this local helper.

## Escalations (need human)
- none
