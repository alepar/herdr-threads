# super-code final review (post-hand-back fix round 2)

**Final review: epic ht-54r, branch `super-auto/session-keyed-seat-bindings` at b35cad5d (fork point 43cc19cd)**

The full suite has never been run on this tip. The last green sweep was at abe1bf03 (5471/5471), before all 14 fix-round tasks (ht-54r.76 to .91), and ht-54r.87 changed the core liveness ranking. I ran no builds or tests; this review is read-only. Treat the branch as untested until the caller's sweep passes.

Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/.superpowers/sdd/ht-54r-plan/review-43cc19cd..b35cad5d.diff`

## Step 1: my own review against the spec

Fix round 2 (ht-54r.87 to .91) closes final review 5's must-fix items 1 to 5 and its item 4:
- **Plain calls (ht-54r.87):** a plain call no longer reconnects a superseded, ended or retired binding. The liveness worker ranks a binding that only plain calls touched below a lifecycle-claimed one, so it can lose but never win. `supersede_plain_child_row` disconnects the stray top-level row of an early Hermes child call.
- **agy gate (ht-54r.88):** 15 of 15 flows pass. All five failures were gate flows written for the old model, not product regressions (triage table in `evidence/ht-54r.88/triage.md`).
- **Composer wake (ht-54r.89):** the wake now matches the binding's fingerprint against the whole set of foreground processes. Live probes showed the old single-PID rule never fired for Claude (its MCP servers and `watch` process share the foreground group) or for Codex installed via npm.
- **Tests (ht-54r.90):** the fix-round tests were proven red on the old code, and the `>=` cursor assertions were tightened.
- **Continuity and doc drift (ht-54r.91):** a continuity reattach now fast-forwards, `continuity_candidates` prefers rows ended `seat_unresolved`, the poke path re-checks liveness, and the stale text was fixed.

I checked the Hermes lineage path that the ht-54r.87 ranking could have broken. A parented `pre_llm_call` for a session the journal has not seen is a lifecycle Startup check-in. So a compression successor claims its binding through a lifecycle check-in, and a successor that started from a CLI call is promoted at its first turn. It does not lose to its predecessor.

New findings:
1. **The policy and usage docs contradict ht-54r.87 (normative drift).**
   - `TRUST-POLICY.md:327-329` still says "An ACK by a disconnected binding reconnects it first (any call carrying its fingerprint does)".
   - `docs/agent-usage.md:141` still says "a later resume, or any call of that session, reconnects it".
   - The code now refuses both after a supersede, end event or retirement. AGENTS.md requires the policy text to change in the same commit as the behaviour.
2. **The config9 flake is probably a race in the test.** `config9_foreground_and_background_agents_in_one_pane` (`tests/integration/session_bindings_smoke.rs:2060`) asserts two things after the one-shot's shim process has already exited:
   - the shimmed one-shot `sess-p` is still `connected`;
   - it is one of the three wake targets.

   The liveness worker runs every 5 s and can disconnect it first. The failure point matches Task 41's report ("before pid assertions"), and the test failed again under load in Task 55. The fix is to give the one-shot a long-lived stand-in process, or to wait on the disconnect.
3. **Main has moved to v0.8.1 (026ad3ab, the ht-clj invite-reuse fix) and was not absorbed.**
   - `git merge-tree` shows a conflict in CHANGELOG.md.
   - `src/store/control.rs`, `src/store/seats.rs` and `tests/store/control.rs` merge automatically. They touch invitation selection, not bindings, so the risk is low, but the result is untested.
4. **The managed-launch placeholder still uses the strict single-foreground rule** (`started_process_pid`). Codex installed via npm, and possibly Claude once it has spawned its MCP servers, record no placeholder. Launch continues and prints a note, but the wake that recovers a lost first prompt is gone. The spec says launch is refused in this case. It does not block landing; it needs a follow-up bead or a decision.
5. **A residual effect of ht-54r.87 (no test covers it).** If a new session's own lifecycle check-in is lost and never replayed from the journal, the session enrolls through plain calls. It then loses permanently to its superseded predecessor in the same process: no ACK, no delivery gate, no wake, until the next lifecycle event. Replaying the pending journal entry covers the usual causes (a timeout, the daemon being down).

## Step 2: ledger triage

**Metrics.**
- "landed≠completed 55 vs 54" is the same one-off from the ht-54r.1 re-dispatch, present in every Metrics line since round 1. Every merge in fix round 2 has a matching `complete` line, so the ledger is complete for that round.
- "check-fails 3 (fixed 2)": the one not fixed is ht-54r.70's block on harness-neutral violations. ht-54r.71 cleared them, and ht-54r.70 then merged with the check passing.

**Recurring clusters.**
- **[doc-drift], 5 times across 5 tasks (ht-54r.2, .21, .80, .78, .91):** when a later task reverses a rule, nobody updates the normative text. It is still live as finding 1. ht-54r.91's own deferred note flagged agent-usage.md:141 and it was not fixed.
- **[missing-red-run], 3 times (ht-54r.6, .21, .83), plus ht-54r.77 [no-red-run] and ht-54r.82 [red-step-skipped]:** a pipeline defect. The RED step is not enforced, and the merge gate only builds. ht-54r.90 retrofitted red runs for .76 to .83, but did not repeat them on the base after .87 landed.
- **[doc-formatting], 3 times:** TRUST-POLICY line wrapping. Cosmetic.

**Minors re-graded to must-fix.**
- ht-54r.91 [doc-drift] in agent-usage.md: this is finding 1.
- ht-54r.91 [flaky-test-unverified] and ht-54r.75 [unexplained-flake], both config9: a flaky test is a must-fix. This is finding 2.

**Parked lines.**
- **ht-54r.5** (11 Hermes/continuity hook tests time out under full-lib load, failing the same way at base): deferred, but it becomes must-fix if the sweep reproduces it.
- **ht-54r.12 → ht-ozo** (the `/clear` wake model was chosen autonomously and the mod_delivery tests were rewritten to match): this needs the human's sign-off.
- **ht-54r.19** (the ignored subagent-cursor test): resolved. The branch adds no `#[ignore]` anywhere.

**BLOCKED-AUTH, ht-54r.7:** its conflict was resolved outside the pipeline and never seam-reviewed. That is untested scope, not a finding.

## Verdict
**Not ready.** I found no open product defect. What remains is doc drift, one test race, absorbing the new base, a human sign-off, and a sweep that has not been run.

## Must fix before landing
1. Absorb main v0.8.1 (resolve the CHANGELOG conflict) before the sweep.
2. Run on the merged tip:
   - the full suite: `nice cargo nextest run --locked --all-targets --all-features`, with `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"` afterwards;
   - clippy with `-D warnings`, `scripts/check-default-features` and `scripts/check-harness-neutral`;
   - Linux CI.
3. Fix `TRUST-POLICY.md:327-329` and `docs/agent-usage.md:141` to match ht-54r.87: only a binding disconnected by process exit or migration reconnects on a plain call.
4. Fix the config9 race: give the one-shot a long-lived stand-in, or assert its disconnect instead of `connected`. Then run it repeatedly, for example with `scripts/flake-hunt`.
5. Human decision: sign off the ht-ozo `/clear` wake model (parked ht-54r.12).

## Untested scope
- **Full suite:** never run on b35cad5d or on anything after abe1bf03.
- **Linux:** the `/proc` fingerprint paths have never been compiled locally.
- **Real harness bindings:** ht-54r.89 derived every fingerprint because no harness could log in, so no real binding row exists for any harness. Also unverified:
  - Codex: the SubagentStop and SessionEnd payloads, and `CODEX_THREAD_ID` across `/new`, fork and resume;
  - Hermes: started by `herdr-threads launch`, `/resume`, and compression lineage.
- **Hermes `subagent_start` dropped by the bridge's single-flight lock:** the child stays a top-level row. After ht-54r.87 that row is superseded, but only ACK checks liveness, so the child can still send, accept, leave or invite. That breaks decision 13 (subagents are read-only).
- **Hermes `/background`:** if it runs in-process turns under its own session, its lifecycle claim supersedes the foreground session. The foreground's later turns are Current-mode check-ins, which cannot reconnect it, so it loses ACK and delivery until `/new`.
- **ht-54r.87 residual:** finding 5.
- **ht-54r.7:** the conflict resolution done outside the pipeline (BLOCKED-AUTH).
- **agy gate:** "15/15" combines a 14/15 run with a separate single-flow wake run after a one-line query fix. There is no single full run on the tip.
- **`continuity_candidates` filter:** it only helps when the seat becomes unresolved before the worker's process-exit pass. After a Herdr restart the worker usually runs first, and the filter falls back to the old behaviour.
- **Other gaps:**
  - passive reads do not reconnect a binding;
  - two hook-pull bindings share one per-seat offer ledger;
  - sandboxes may hide the ancestor PID;
  - a sibling PreToolUse hook can drop the `--agent-id` rewrite.

## Deferred OK
- **Managed-launch placeholder:** the strict single-foreground read (finding 4). File a follow-up bead or decide to accept it.
- **TRUST-POLICY.md:825**, the Hermes "no C1 resume continuity" wording: probably still accurate.
- **[doc-formatting] cluster:** the line wrapping.
- **Code smells:**
  - the `"superseded"` literal at delivery_pull.rs:229;
  - the redundant `LivenessFingerprintRequired` branch;
  - the dropped `checked_host_number` range check;
  - the duplicated `run_wire` fallback.
- **Tests:**
  - the brittle source-grep test in test_scenarios_l6.py;
  - the weak `a_no_binding_refusal_keeps_the_state_file` (it only pins the client);
  - red runs not repeated on the base after .87;
  - the loose agy ancestor check in run_command_env.py.
- **Repository size:** `evidence/` adds 3.4 MB on this branch. Committing evidence is existing repo practice, but ht-54r.88 commits five near-duplicate artifact sets, which could be pruned.
- **Performance:** the full-scan unread probes in read_cursors, and the per-seat hook-pull ledger.
- **Parked ht-54r.5:** the Hermes load timeouts, unless the sweep reproduces them.
