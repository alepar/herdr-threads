super-roast verdict: Nit (2 confirmed) [converged]
mode: pr        iteration: 2 of 3
profile (assumed): Open-source developer tooling (herdr-threads, a Rust CLI/daemon, v0.6.0 public) used locally by a single user to coordinate coding agents in Herdr panes. The trust model is same-user and cooperative (TRUST-POLICY). Correctness of seat continuity, receipts, delivery settlement and user-config ownership matters; writes to user config must be owned and reversible (v0.6 backup model). No network-exposed surface, no multi-tenant data, easy local rollback.
inputs: super-auto/agy-harness-adapter@73ed0b06 vs main@f3431b31
delta vs prior: 1 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 3 resolved · 1 regressed (0 Blocking) · 20 punch-listed (open)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 5 → deduped 3 → panel 2 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0
lane-yield (found/confirmed/unique/refuted): correctness 1/1/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 1/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 1/1/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 1/1/1/0 · hygiene-docs 1/0/0/0

## Confirmed findings
- [Nit] src/harness/agy/delivery.rs:913 — The new inconclusive-probation-end branch in agy `stop()` re-downgrades the identity-wide record on any conversation's Stop that injected this turn (including userMessage-only turns), so another conversation can cancel a probation it did not open and the identity stays on userMessage for another 24 h even when systemMessage delivery succeeded. [lanes: correctness, regression] [fix-regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  new this iteration: introduced by 1693fc18, the fix for the prior Nit at delivery.rs:842 (probation never ended after an inconclusive turn). The prior defect is gone; this is a different defect the fix added.
  evidence: The downgrade record is one file per identity (`load_downgrade`/`save_downgrade(self.state_dir)`) and `probation` lives on it. `injected_this_turn` is per conversation (ConversationState, line 283) and `record_injection` (line 338) sets it for userMessage injections too. The new branch at line 913 is `else if existed && record.probation && state.injected_this_turn { record.downgrade(&reason, now); save_downgrade(..) }` with no check that this conversation opened the probation or injected a `!e.user` entry. Trace: conversation B injects a userMessage page; conversation A's pre_invocation opens probation (lines 862-867) and injects systemMessage; B's Stop runs: the confirm clear needs `!e.user` (line 736) so it does not fire, `is_miss` needs a `!e.user` entry (line 946) so it is false, the new branch matches and `record.downgrade` sets probation=false and downgraded_at=now. A's later confirm finds its systemMessage frame but the clear is gated on `downgrade.probation` (line 736), now false, so `step()` returns UserMessage for another 24 h. Contradicts the doc comment at lines 476-479 (probation ends at that turn's own Stop). Before 1693fc18, B's Stop matched no branch and A's confirm cleared the record. The ground seat notes the trigger is wider than the finding states: B can inject a userMessage while A's probation is open with no need to straddle the 24 h mark. Tests added in 1693fc18 (tests/harness/agy_delivery.rs) exercise a single conversation only.
  severity note: all three seats rated Nit. Delivery still works through userMessage, so the cost is a delayed systemMessage retry (self-correcting degraded mode), not data loss. No floor applies.
  fix-shape hint: gate the branch on this conversation having made a non-user injection this turn (`state.injected.iter().any(|e| e.turn == state.turn && !e.user)`), or record which conversation opened the probation (`probation_conversation`) and apply the inconclusive-end and miss branches only in that conversation's `stop()`; add a two-conversation test.

- [Nit] tests/service/delivery_pull.rs:723 — The fix round's test for 'attention block absent when the version was seen' never reaches the `!seen_current` guard on `with_attention`; every seen-version pull with only an invitation pending returns early through the Unchanged fast path, so deleting `!seen_current &&` from delivery_pull.rs:294 leaves every test green and the case the guard exists for (seen version with a due or new message) is unpinned. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  regressed (fixed incompletely, see iteration 1): the prior Should-fix at src/service/delivery_pull.rs:294 asked for three cases (present, absent on `seen_current`, attention-only page is `Page`). The fix added tests for all three, but the absent-on-`seen_current` test does not exercise the guard. The present and attention-only cases are covered, so the residue is rated Nit.
  evidence: src/service/delivery_pull.rs:222-232 returns `Unchanged` before any page work when `seen_current && entry.read_attention.is_some() && !entry.more_pending && !entry.any_due(now)`. The test `attention_block_is_absent_when_the_version_was_seen` (tests/service/delivery_pull.rs ~723-750) seeds only `pending_invitation` with no pending message, pulls once with `None`, then pulls with `Some(first.attention_version)`; after the first pull read_attention is Some, more_pending is false and no offers exist, so the second pull takes the early return at :224 and never evaluates `let with_attention = !seen_current && view.fingerprint.other_pending > 0;` at :294. `pending_invitation` is used only in the new attention tests (lines 701/703, 726, 739, 754); none combines an invitation with a due re-offer (`advance(REOFFER_AFTER_MS)`) or a new message after the version was seen. The older seen-version tests (lines 383-435, 620, 828) create no invitation, so `other_pending` stays 0 there. If the guard were removed, the attention block would repeat on every 60 s re-offer page, which DP5 step 5 ('when the version moved') forbids. Production code is correct; only the regression pin is missing.
  fix-shape hint: in the absent-on-seen test, seed a pending message alongside the invitation and either `advance(REOFFER_AFTER_MS)` or add a new message after the first pull, then pull with the seen version and assert the page has no `attention` frame.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this iteration. Every rejection from iteration 1 stands; no scout re-surfaced a previously rejected finding.

## Unverified nits (spot-checked)
- [Nit] migrations/0029_seat_wake_on.sql:5 — Commit e9786a40 changed the lazy wake frontier to `decision_seq` but left the unreleased migration 0029 column, WakeRow field, schema verification, test name and DP8 design doc naming it `last_lazy_ordinal`; rename now while 0029 is unreleased. (spot: CONFIRM Nit — migrations/0029:5 still adds `last_lazy_ordinal`; src/store/wake.rs:120/141/167/522/694, src/store/schema.rs:606 and tests/store/wake_on_wake.rs use that name while `wake_frontier` in src/store/lazy_delivery.rs stores `max(decision_seq)` and its doc says the staging ordinal would be wrong; `lazy_recipients.ordinal` still exists so the two orders can be confused; the seat did not check the design-doc line claims (lines 65, 340-342). Residue of the resolved prior Should-fix at src/store/lazy_delivery.rs:249; a rename is an in-place edit only until 0029 ships.)

## Prior confirmed findings not re-surfaced
- resolved: [Should-fix] src/store/lazy_delivery.rs:249 — frontier now keyed on publication order (e9786a40); only the naming residue above remains.
- resolved: [Nit] src/harness/agy/delivery.rs:842 — probation now ends after one inconclusive turn (1693fc18); the new branch introduced the fix-regression confirmed above.
- resolved: [Nit] scripts/agy-relay/budget.py:71 — worst-case cost is reserved in reserve() (d48950b2); no scout re-surfaced it.
- punch-listed (open), 20 entries, unchanged from the caller's list: src/delivery/ledger.rs:137; src/delivery/ledger.rs:96; src/harness/agy/delivery.rs:873;880; src/harness/agy/delivery.rs:647;644; src/harness/agy/delivery.rs:830; src/cli/hook.rs:2960; scripts/agy-sandbox/fetch-agy:124; src/cli/hook.rs:2878; src/cli/topology_handoff.rs:192; src/service/delivery_pull.rs:384; src/harness/agy/decode.rs:98; src/harness/agy/plugin.rs:60; src/protocol/delivery.rs:1;3; src/harness/agy/delivery.rs:325; src/harness/agy/delivery.rs:909 + tests/harness/agy_delivery.rs:794; TRUST-POLICY.md:211; [FYI] src/store/schema.rs:411;409; [FYI] src/harness/agy/setup.rs:379; [FYI] src/service/delivery_pull.rs:297; [FYI] src/service/delivery_pull.rs:176. Nobody fixed these; the caller chose not to. They are sub-Blocking and do not affect convergence.

## Escalations (need human)
- none

