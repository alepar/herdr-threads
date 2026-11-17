---
super-roast verdict: clean (17 nits)
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling: a local multi-agent messaging daemon. The trust model is cooperative and same-user. A wake delivery misfire types text into a terminal.
inputs: fix/codex-resume-dormant-wake@472dada00 vs main@a686a6684
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, testing, test-causality-lifecycle, test-isolation-environment, observability, hygiene-docs, api-contract) · raw 48 → deduped 28 → panel 11 · spot 17 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "git diff main..472dada00 in the worktree"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 11 · rr 1.00 · rg 0.64 · fg 0.64 · unanimous 0.64 · ground-loo 0.64 (n=11) · reproduce 0/11/0 · refute 0/11/0 · ground 4/7/0
lane-yield (found/confirmed/unique/refuted): correctness 5/0/0/5 · security 2/0/0/2 · premortem 4/0/0/4 · simplicity-design 6/0/0/5 · hot-path-perf 3/0/0/3 · concurrency-async 4/0/0/4 · testing 7/0/0/7 · test-causality-lifecycle 2/0/0/2 · test-isolation-environment 2/0/0/2 · observability 5/0/0/5 · hygiene-docs 3/0/0/3 · api-contract 5/0/0/5

## Confirmed findings            ← consumed by super-design, one task per finding
- none

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- src/store/lazy_delivery/mod.rs:257; :248 — The dormant predicate is widened in harness-neutral core to every harness's end event with no adapter policy, while rationale, tests and e2e cover only Codex `/quit`.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✗), 3 of 3 REJECT.
  reason: Dormancy was already harness-neutral on base (`harness<>'human'` and host kind only), so the diff extends an existing neutral predicate by one disconnect reason and adds no harness branch.
  reason: The reversal of "a clean quit gets no wake" is the stated intent. TRUST-POLICY A4, AGENTS.md and the doc comment are amended in the same commit, and the widening is recorded as an accepted limit.
  reason: The Hermes and Claude exposure is asserted, not demonstrated. The native recheck (agent kind equals the harness, awaiting input, composer empty) still gates every typed wake.
- src/store/lazy_delivery/mod.rs:248; TRUST-POLICY.md:1268 — After a deliberate `/quit`, a fresh unrelated session of the same harness in that pane gets the wake, with no time bound.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✗), 3 of 3 REJECT.
  reason: The diff states this as an accepted limit in TRUST-POLICY ("so does a pane whose session ended for good and that holds another session of the harness").
  reason: A resumed session and a fresh session of a harness that runs no hook before its first turn are indistinguishable to the daemon. A time bound would be heuristic evidence, which the trust policy forbids for binding decisions.
  reason: The same limit, with no time bound, already exists on base for process-exit dormancy. The cost is one content-free wake turn in a same-user model.
- src/scheduler/mod.rs:999 — A dormant wake recorded Submitted in the end-event window is marked `unreceived` and not retyped, so the resumed Codex gets no p0 wake.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✗), 3 of 3 REJECT.
  reason: A pane at a shell yields Refused, which goes to `record_dormant_refusal` and never creates an AwaitingReceipt. Only a wake typed into a still-exiting TUI remains.
  reason: The AwaitingReceipt closed loop is documented behaviour in AGENTS.md and already applies to process-exit dormant targets on base.
  reason: The suppression clears on hook contact, a newer item or a target change. Not reproduced; the seats read code only.
- src/store/lazy_delivery/mod.rs:257 — An end-evented binding becomes a wake target while the Codex process is still alive, so the wake marker can be typed into a pane that is tearing down (external).
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✗), 3 of 3 REJECT.
  reason: The window is at most about 3 s (SessionEnd clamp, src/harness/codex.rs:1267-1268) and needs a wake attempt to fall inside it.
  reason: AGENTS.md accepts that composer admission is sampled and not atomic. A connected idle Codex already had the same sampled wake up to the disconnect commit on base.
  reason: The worst case is one `herdr-threads: wake up` line discarded by the dying TUI or read by the shell. No state is touched.
  note: The external premise (what Herdr's `agent.get` reports during Codex teardown) was not grounded by any seat. All three seats rejected on materiality, which does not depend on it. The proposed spike was not run.
- src/store/lazy_delivery/mod.rs:257; TRUST-POLICY.md:1268 — End events sent while the harness process keeps running (Claude `/clear` or `/resume`, Hermes `on_session_finalize`) open a dormant window before the successor session checks in (external).
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✗), 3 of 3 REJECT.
  reason: TRUST-POLICY A4 already accepts "another session of that harness in the pane, which the seat would enroll anyway, gets the wake". An in-process switch is that case.
  reason: The dormant authority grants only a content-free wake. The successor's turn-start hook delivers the pending p0, which was owed a wake anyway.
  reason: The finding itself calls the Claude and Hermes event ordering unestablished. No seat researched it.
- src/store/lazy_delivery/mod.rs:137; :133 — `seat_owes_display_sql!` was not extended to `liveness_end_event`, so a row displayed by another top-level binding drops out of an end-evented dormant binding's wake walk; no test covers it.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✓ at Nit), 2 of 3 REJECT.
  reason: The trigger needs a second top-level binding of the seat that displays the row and then disconnects. That row was shown to the seat, so skipping a wake for it loses nothing.
  reason: Rows nothing displays stay pending, and the new test `an_end_event_leaves_a_dormant_binding_like_a_process_exit` covers that path.
  dissent: The ground seat confirmed the same facts at Nit (stale comment, missing test). The majority's evidence addresses them, so the tally stands. The stale-comment part is listed under Unverified nits.
- src/notification/dispatch.rs:1012; src/store/lazy_delivery/mod.rs:257 — A cleanly quit seat whose pane stays at a shell now retries refused wakes with no end condition (300 s spacing), and the commit does not acknowledge it.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✓ at Nit), 2 of 3 REJECT.
  reason: "Not acknowledged" is false. The commit message and the TRUST-POLICY A4 edit both state that the refused wake "retries on the dormant ladder".
  reason: The retry loop and its 300 s cap (`DORMANT_REFUSAL_CAP_MS`) exist on base for process-exit seats and were accepted in the ht-unmk review. The diff does not touch dispatch.rs.
  reason: The cost is about 12 attempts per hour per seat, and only while an admitted pending row exists and no successor binding connects.
  dissent: The ground seat confirmed at Nit and conceded both points above. A possible follow-up is an age or contact bound on end-event dormancy.
- scripts/wake-e2e.py:3111; :3096 — The `resume-quit` case requires `disconnect_reason == 'liveness_end_event'` from one unpolled read, so a slow SessionEnd hook records `liveness_process_exit` and the case FAILs although the wake works (external).
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✗), 3 of 3 REJECT.
  reason: The assertion is the case's proof that the end-event path ran. `resume-idle` already covers the process-exit path, so a FAIL on the wrong reason is deliberate strictness.
  reason: The failure can only be a false FAIL, never a false PASS. It was not reproduced and rests on an unverified external Codex fact.
  reason: The `reasons[0]` ordering concern shows no scenario with more than one open top-level row for the session.
- src/store/schema.rs:1503 — A sender's overdue warning for an end-evented seat left at a shell changes from `wake=none` to a refusal outcome, and the diff does not state the model-visible change.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✗), 3 of 3 REJECT.
  reason: No new model-visible string is added. The refusal tokens already exist and are already shown for process-exit dormant seats. AGENTS.md rule (f) covers new strings.
  reason: The refused-and-retried behaviour is stated in the TRUST-POLICY edit and the commit message. Nothing documents `wake=none` as a contract for ended seats.
  reason: The finding names no consumer that breaks. `side-draft` concerns a typed draft, not an ended session at a shell.
- TRUST-POLICY.md:1268 — The new policy sentence, commit message and store test rely on `empty_shell` occupancy, which production never assigns.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✓ at Nit), 2 of 3 REJECT.
  reason: The premise is true: `EmptyShell` is assigned only in a test closure (src/host/native.rs:6573).
  reason: The sentence is a disjunction ("no foreground agent, or `empty_shell` occupancy") and the first limb is the enforced guard, so a pane at a shell is still never typed into.
  dissent: The ground seat confirmed at Nit and agreed the policy statement is not false. A possible follow-up is to drop the dead clause from the policy text and test the foreground-recheck refusal instead.
- tests/store/binding_lazy.rs:986 — The `db_with_subagent_only` fixture parent was switched from EndEvent to Superseded, so the real `end_event_session` shape (parent and child both end-evented) is asserted nowhere.
  verdict: rejected (reproduce ✗ / refute ✗ / ground ✓ at Nit), 2 of 3 REJECT.
  reason: The fixture exists to isolate "a subagent is never woken", which needs a non-dormant parent. An EndEvent parent is dormant under the new rule.
  reason: The new rule is asserted by `an_end_event_leaves_a_dormant_binding_like_a_process_exit`. `dormant_top_level` filters `role='top_level'`, so an end-evented child is excluded structurally.
  dissent: The ground seat confirmed at Nit: no test pins the combined parent-plus-child shape. The majority's evidence addresses it as the composition of two tested rules.

## Unverified nits (spot-checked)
- [Nit] src/store/lazy_delivery/mod.rs:126 — Unchanged doc comments (mod.rs:126-132, wake.rs:41-48) still say only a process-exited binding keeps its rows pending and that `seat_owes_display_sql!` keeps the dormant walk's rows pending; this is now incomplete for an end-evented dormant binding.
  spot check: CONFIRM at Nit. Only the wake.rs:41-48 comment misdescribes the dormant binding. No behavioural effect.
- [Nit] scripts/wake-e2e.py:3094 — If `/quit` does not end Codex within QUIT_EXIT_S, the driver still types the resume command line into the running Codex composer and leaves the shared recipient pane dirty for later cases.
  spot check: REJECT. The case already records FAIL (`quit_exited` gates `passed`), so it cannot yield a false PASS. A SIGKILL fallback would be optional hardening.
- [FYI] src/store/lazy_delivery/mod.rs:257 — Cleanly ended seats now reach the bounded DORMANT_WINDOW (4096-row) pending scan in every wake discovery walk.
  spot check: REJECT. The scan is index-ordered and capped by the existing window. Not measured.
- [Nit] src/host/native.rs:1416 — A dormant wake refused for a shell or non-matching agent records no `WakeBlock`, so senders and `doctor` see no reason.
  spot check: REJECT. The process-exit dormant path behaves the same. A new `WakeBlock` variant would be a separate feature.
- [Nit] scripts/wake-e2e.py:229 — No hermetic test drives the real SessionEnd hook path into a dormant wake target; only the opt-in real-model `resume-quit` case does.
  spot check: REJECT. The hook write (tests/hook_entrypoint.rs:8511) and the predicate (new store tests) are covered separately.
- [FYI] src/store/lazy_delivery/mod.rs:257 — Every new test uses a Codex binding; non-Codex end-evented seats as dormant targets, including channel-woken harnesses, are untested.
  spot check: REJECT. Wake classification (`classify_wake_rows`) is untouched and does not key on `disconnect_reason`.
- [FYI] src/store/lazy_delivery/mod.rs:257 — "Which disconnect reasons leave a binding resumable" is encoded in three places with three different reason sets, as raw SQL strings.
  spot check: REJECT. The divergence is documented, and the raw-string sets existed on base. No misdelivery was demonstrated.
- [Nit] scripts/wake-e2e.py:3112 — The `disconnect_reason` check takes `reasons[0]` from an unordered query.
  spot check: REJECT. The query is scoped to one seat, one native session and the open top-level binding. No path to a second row was found.
- [FYI] src/store/lazy_delivery/mod.rs:257 — Speculative: end-event dormant seats left at a shell may keep consuming the discovery walk's 100-seat examination cap.
  spot check: REJECT. pre-existing on base: src/store/mod.rs:1247 (`discover_wake_candidates`) already examines process-exit dormant seats the same way on a686a6684.
- [Nit] TRUST-POLICY.md:1268 — A wake typed into another session of the harness in the pane leaves no record distinguishing it from a correct dormant wake.
  spot check: REJECT. The limit is documented as accepted, and no requirement asks for per-session wake attribution.
- [FYI] scripts/wake-e2e.py:3110 — No operator-facing command shows that a seat is dormant or why; the e2e reads `disconnect_reason` from the SQLite file.
  spot check: REJECT. pre-existing on base: `git grep disconnect_reason -- src/cli src/protocol src/service` on a686a6684 finds only src/service/delivery_pull.rs:334.
- [Nit] scripts/wake-e2e.py:3095 — The `resume-quit` driver uses a fixed 1 s sleep between typing `/quit` and Enter, and the Enter send is `check=False`.
  spot check: REJECT. The same idiom is at scripts/wake-e2e.py:2977, 3167 and 3180. It can only produce a false FAIL, recorded as `quit_exited=False`.
- [Nit] tests/store/wake.rs:3126 — The end-event branch does not assert `composer_choice(&[])` → `NoMatch` or `validate_wake_reservation`, which the process-exit branch does.
  spot check: REJECT. The seat rejected on the finding's own concession that `composer_choice` does not branch on the disconnect reason, and did not read that source.
- [Nit] tests/store/binding_lazy.rs:927; :913 — The successor test forces its ordering with a raw SQL UPDATE and `disconnected_at=i64::MAX` instead of `disconnect_now`.
  spot check: REJECT. The test still exercises the binding choice in `dormant_top_level`. The raw UPDATE matches the file's existing helpers.
- [Nit] src/store/lazy_delivery/mod.rs:254 — One wake examination evaluates `dormant_top_level` and the dormant window several times per seat, and more seats now reach that path.
  spot check: REJECT. The diff adds no calls, and the finding concedes the pattern predates it. Not measured.
- [Nit] tests/store/binding_lazy.rs:942 — The negative tests hand-copy the complement of the dormant reason set, so a new `DisconnectReason` variant is covered by neither.
  spot check: REJECT. The SQL is an allowlist, so a new variant defaults to not dormant.
- [Nit] scripts/wake-e2e.py:3094 — `resume_case` branches its exit mechanics on `case.startswith("resume-quit")` and adds a module-level `_pid_alive` helper.
  spot check: REJECT. RESUME_CASES has two entries, so a two-way branch is proportionate.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none
---