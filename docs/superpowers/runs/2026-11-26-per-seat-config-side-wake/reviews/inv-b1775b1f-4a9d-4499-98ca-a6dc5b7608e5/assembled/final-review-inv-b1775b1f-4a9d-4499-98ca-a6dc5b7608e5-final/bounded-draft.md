# Final review (bounded) — epic ht-z41e, super-auto/per-seat-config-side-wake

## Scope and method

- Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head fbc2321c41f29405211b13f6ee5452beec9cf55b (both verified with `git rev-parse --verify <sha>^{commit}`). The caller checkout was at fbc2321c4 with a clean `git status` before and after the review.
- This is a bounded review. `review-inputs.json` reports status unavailable, sourceAvailable true, wholeRangeReady false, 596 selected, 0 omitted, and 2 gaps, both at `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/run.md:115` for inv-f5f76bdd (incomplete claimed collection; incomplete split bundle). No whole-range package was built or loaded.
- I inspected the production diff hunks listed in bounded-coverage.json: the seat-config registry, store, CLI and daemon routing; queue selection, root naming and arm; the dispatcher's queue-candidate gate; the queue runtime and its admission; the native host's queue admission and dwell code; scheduler receipt scope; auxiliary-session evidence from the Codex decode through the bridge, the capability gate and the binding store; the three migrations; doctor; the settings change; permission rules; the skill text; and release-qualify. I also read the head text of TRUST-POLICY.md ("Queue wake target with /side sessions present") and AGENTS.md lines 109 and 147.
- In a temporary detached worktree at head (since removed), `scripts/check-harness-neutral` passed (0 violations, 90 allowlisted) and `scripts/check-wire-compat` passed (0 violations). Nothing was built and no tests were run.

## Requirement trace (baseline B1–B10, S1)

- B1 (replace the node setting with a seat setting): `InstanceSettings.programmatic_wake` is now `Option<bool>` and has no effect (`src/daemon/settings.rs`). `ServiceConfig` and `QueueWakeRuntime` lose the process-wide flag. Queue selection (`selection.rs:174`), arm (`wake_intents.rs:285`) and the attention witness (`store/wake.rs:272`) read `seat_config::programmatic_wake_on`, which fails closed. Doctor reports the obsolete node key and the seats that have the key on. Delivered.
- B2/S1 (generic key=value capability, closed typed registry): `src/protocol/seat_config.rs` `REGISTRY` holds two keys, each with values, default, tier and backing. Unknown keys and invalid values are refused with the allowed set. Delivered.
- B3/B4 (a read/write CLI and a list with descriptions): `ht config seat list|get|set|unset` and `ht human config seat set|unset` are wired through `caller_need`/`run_selected` to `config_seat::run`. `list` runs without a daemon (`src/cli/config_seat.rs`). Delivered.
- B5 (an agent told in prose can set the setting): the trailer of `list` names both spellings, an agent-path write of a human-tier key is refused with the exact `ht human config seat set KEY VALUE` line, and SKILL.md has one line on per-seat settings. Delivered.
- B6/B7 (every caller reads; agents write low-risk keys such as nudge_threshold; gated keys need a human): `nudge_threshold` is agent tier. Agent writes go through `cooperative_mutation`, so they write only the caller's own seat and refuse subagents and read-only bindings. The human tier needs `operator_owner_uid`, and Claude and Codex have ask rules for `human config seat`. The `nudge-threshold`/`wake-on` aliases share `write_column`. Delivered, with the accepted cooperative-model limit that a harness without an ask rule does not prompt.
- B8 (wakes reach the top-level session while /side is open; spikes): spikes S1–S4 are recorded. The production path is: a Codex hook with null `transcript_path` sets `auxiliary_session`; `process_root` with `CodexAdapter::wake_process_root` names the root; the dispatcher's `queue_candidate` lets the attempt through; `admit_queue_wake` accepts the side placeholder; the intent targets the root UUID; and receipt is scoped to the root session. This path exists for an empty side composer. Two gaps: a typed side draft refuses the queue wake, a narrowing parked for a human decision with no scope-change row; and there is no live run at the final SHA (ht-z41e.10/.31/.43 are open).
- B9 (stop native wake typing into a side chat): the side placeholder is removed from `CODEX_PLACEHOLDERS`, so native admission refuses a displayed side composer. Delivered.
- B10 (brainstorm decisions): no migration; agents write their own seat and a human may name any seat; the key sits over the existing column; live spikes ran under codex-2. Delivered as designed.

## Ledger triage notes

- The recurring `missing-evidence` cluster (×6 across 6 tasks) is process hygiene: logs were left in scratch and no structured evidence was kept. It is not a product defect, but it means no retained per-task test evidence exists at head.
- Task 1 line 18 (`herdr_release_smoke` side-close path compiled but ignored, needs a private Herdr) names an ignored test, so under the triage rule I regrade it to must-fix. It belongs to the open live-qualification gate.
- Task 7 lines 60–61 (concurrent_session used as the marker; a lone mis-enrolled side gets targeted) were superseded by ht-z41e.21. The lone-binding rule in `process_root` now refuses concurrent or auxiliary bindings.
- Task 11 line 153 (a retry journal can carry `auxiliary_session` to a downgraded daemon) affects only a downgrade between record and retry. Deferred OK.
- Task 24 line 359 (`queue_text_ok` accepts any target with `WAKE_UP`): this holds today because queue selection names only a `cooperative_top_level` root. Deferred OK.
- Task 26 lines 480–481: the extra host round trip was accepted by the amended design; `check-default-features` has not been run and is outstanding before merge.
- Task 28 line 542 (TRUST-POLICY/AGENTS may still say a side draft admits a queue wake): checked at head. TRUST-POLICY and AGENTS.md:109 both state that typed text refuses, so this is resolved.
- Two `parked` escalations sit in run.md. One is the roast-design-3 null-transcript rule (conservative, so it is a concern only). The other is the final-review side-draft rule (`final-review-inv-ac7d9dd6`), which narrows B8 and needs a human decision.

## Findings

- [minor] `src/host/native.rs:2534` (`observe_target_with_composer`): `reset_empty` lost the base clause `|| (composer_ui && ui != HostUiState::Idle)`. The new comment says the composer read inside `observed_ui` already resets the window, but `observed_ui` returns `Unknown` without any read when the pane has no agent kind, an unrecognized kind, or no composer policy. In those cases an idle/done observation no longer resets the native focus-dwell window. Example: a focused Codex pane starts its dwell, then Herdr briefly reports no agent at t=30s. At t=61s the first empty read admits typing, although the composer was not observed empty throughout. This contradicts design §Unchanged behavior, which keeps the native focus dwell as it was. The fix is one line: restore the clause.
- [minor] `src/service/delivery_pull.rs` `session_contacts` is never pruned. It gains one entry per (seat, native session) that makes a claim-bearing call over the daemon's lifetime, so memory grows slowly with session churn.
- [minor] `src/notification/queue.rs` `ROOTS_MAX`: when 256 roots are retained, the whole map is cleared. That can drop the root of an in-flight attempt, and that attempt then gets no `AwaitingReceipt` (no `wake=unreceived` tracking). This is diagnostic loss only.
- [nit] `src/protocol/commands.rs` places `use crate::protocol::seat_config::ConfigTier;` between `enum PermitMutation` and its `TryFrom` impl.

## Concerns

- The human tier is enforced only by the harness permission policy. The agy, opencode, Hermes and Pi adapters have no ask rule, so an agent there can write `programmatic_wake` without a prompt. SKILL.md and TRUST-POLICY record this as an accepted limit of the cooperative model; B6 holds only under that model.
- The parked roast-design-3 escalation: a null `transcript_path` sets only `auxiliary_session`, so a root that emits null (`codex exec --ephemeral`) silently loses queue routing. The behaviour is conservative, but the human has not confirmed it.
- `one_queue_process` in the dispatcher compares only `fingerprint_pid` and harness. It now lets a working or human-input pane through to the queue lane whenever the seat has `programmatic_wake` on. Safety then rests on the queue admission and on the native fallback's `check_idle_wake` final gate refusing. I read both and they do refuse, but the composition has no live proof at the final SHA.

## Verdict

not ready (bounded review; live qualification and herdr_release_wake not run at final SHA; parked side-draft B8 narrowing needs a human decision; no full-suite measurement)

## Must fix before landing

- B8: run the open live qualification (ht-z41e.10, ht-z41e.31, ht-z41e.43) at the final SHA. That means `side-queue/group` under `--queue-proof` with the recipient pane focused and a busy side turn, plus `side-native-defer`, `side-nudge` and `side-draft`. No real-Codex evidence yet covers the ht-z41e.42/.45 queue admission and dispatcher gate.
- Ledger Task 1 (ht-z41e.1), regraded: the `herdr_release_wake` side-close path and the inverted /side expectation in `tests/integration/herdr_release_smoke.rs` (ignored, needs a private Herdr) have never run. Run them through `scripts/herdr-release-smoke` before landing.
- B8 narrowing with no human scope-change row: the queue lane refuses any typed side draft (orchestrator decision, parked as `final-review-inv-ac7d9dd6`), so a seat with `programmatic_wake on` and a side draft is not woken in the root. Get the human's decision and record it as a scope-change row, or implement side-draft admission.

## Untested scope

- No full-suite measurement of this branch exists (gate ht-z41e.27 is open; the caller runs the sweep after this invocation). `scripts/check-default-features` and clippy were not run by this review.
- The unbounded whole-epic review (gate ht-z41e.28) is outstanding. Provenance gaps: run.md:115 inv-f5f76bdd incomplete claimed collection; run.md:115 inv-f5f76bdd incomplete split bundle (missing cluster report or slices).
- Paths not inspected in this bounded pass include all tests/** changes, scripts/wake-e2e.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue_selftest.py, scripts/tests/test_release_qualify.py, tests/release/release_qualify_test.py, src/store/queries.rs, src/store/wake.rs (beyond line 272), src/service/kicks.rs, src/store/archival.rs, src/store/invitation_content.rs, src/store/seats/continuity.rs, src/ports/wake_intents.rs, src/protocol/results.rs, src/protocol/handoff.rs, src/protocol/service.rs, src/harness/mod.rs, the harness decode one-liners, src/test_support/**, the remaining src/host/native.rs and src/cli/commands.rs hunks, CHANGELOG.md, README.md, RELEASE.md, docs/** (beyond the design doc and run.md), integrations/codex/README.md, and the evidence logs.
- No test covers an armed queued `OutcomeUnknown` attempt with a root (ledger Task 20); only the unarmed case is tested.
- The wake e2e driver's new cases (config-prose, flag-flip, side-queue, side-native-defer) have only pure-helper selftest coverage and no live run (ledger Tasks 8 and 22).

## Deferred OK

- `src/host/native.rs:2534` dwell reset narrowing (minor): an edge that needs a Herdr agent-kind flicker; fix as a one-line follow-up.
- Unpruned `session_contacts` map in `src/service/delivery_pull.rs`: slow growth bounded by session churn; follow-up.
- `ROOTS_MAX` wholesale clear in `src/notification/queue.rs`: loses unreceived tracking only, never delivery.
- Retry journal carrying `auxiliary_session` to a downgraded daemon (ledger Task 11): needs a daemon downgrade between record and retry.
- `queue_text_ok` accepting any WAKE_UP target (ledger Task 24): correct while selection names only a cooperative_top_level root.
- Recurring missing-evidence cluster (ledger, ×6): process hygiene, not product behaviour.
- `use` placement nit in `src/protocol/commands.rs`.
