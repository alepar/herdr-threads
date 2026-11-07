# Final review, cluster 5 (root): TRUST-POLICY.md

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7, both verified with `git rev-parse --verify`. Caller checkout observed at the head and clean, before and after reading. Input: `slices/000001.diff`, the only file in this cluster. Line numbers are of `TRUST-POLICY.md` at the head.

## Method

The policy is normative, so each new claim was read against the production code it describes, not against the spec alone. Source beyond the diff was opened only for that, and for the baseline rows. Nothing was executed; no worktree was created and no test was run.

## Policy text that matches the head

- The key outcome table and lag rule (lines 233-250) matches `store::seats::key_verdict::judge` row for row, including the lag rule and the retired row being lag-independent. `key_decision` ignores a key on any host kind but Herdr, as line 231 says.
- Refusal text and codes match: `key_conflict_cause`, `TargetAlreadyOwned` for the alert-only kinds, definitive `TargetUnresolved` for the hold-bearing ones, `ContinuityKeyPending` for a claimless caller. The hold check sits in `current_resolution_owner`, which C1 and operator repair do not use.
- The clearing list at lines 258-261 matches the callers: `rebind_unresolved_seat`, the fresh-seat branch of `mutate_operator` (the task 2 gap is closed), `clear_conflicts_absent_from`, `delete_key`. Line 261's "clears at the check-in" is what `cooperative.rs:1053` does; it is narrower than spec 4.2, and here the policy is the accurate text.
- *Reversible retirement* (lines 448-483) matches `begin_retirement_authored`, `reversible::{unretire, unretire_by_fact, finalize, finalize_by_fact}`, `retire_row`, and the fragment users in `poke.rs`, `receipts.rs`, `invitation_due.rs`, `archival.rs`, `schema.rs`, `queries.rs`, `observation.rs` and `resolution.rs`. Authorization order gives what line 459 states.
- The client paragraph (lines 267-274) matches `caller_continuity_key`, `presented_continuity_key`, `own_pane_continuity_key`, `SemanticMutation::ContinuityCheckIn.continuity_key`, `continuity_intent` reuse on key equality, `api_failure` and `pending_failure`. The watch claim goes through `hook::watch_continuity`, so it shares `PaneCall::continuity_key`.
- The provenance rows, the N4 edit and the amendment counts for .8, .2 and .3 are correct. The scrub claim in *Key inheritance* matches `src/host/mod.rs:40` and the shim's blanking of all `HERDR_*`.

## Baseline rows, as far as this file reaches

- B1: the policy states the requested behaviour and its limits. The Hermes gateway failure is cluster 3's item; if a human scope change takes that route rather than a rename, line 2186's limit must state it.
- B2: the policy states an immediate fence and service undo, and the code it names does both. The end-to-end path is the seam reviewer's.
- B3: lines 454-455 are true of the code; no path turns an agent exit into a close. See the untested list for the test's gap.
- B4: the policy adds no Herdr change, and the diff has none.
- The accepted limit at line 2173 promises operator finalization of a restored seat. Cluster 4's `seat inspect` panic follows that exact path; fixing the read needs no policy edit.

## Verdict

not ready (spec 4.6's G2 and G5 policy text is absent with line 257 naming a path that does not exist, C1's count is false, and the key inheritance limit promises a recovery the code refuses)

## Must fix before landing

- `TRUST-POLICY.md:2186-2189` says an inherited key leaves the pane "seatless until the variable is unset", and that is false: `current_resolution_owner` refuses every ordinary resolution while `hold_kind` finds the row, whatever key is presented, and `conflict_hold_is_definitive_and_not_kept` (`tests/store/continuity_key_decisions.rs:722`) pins the refusal for a keyless resolve; a row clears only on a rebind, an operator fresh seat, a coherent publication without the target or release of the key, so after the variable is unset the pane stays seatless, and `api_failure`'s prefix match makes a keyless hook there quiet, so the agent gets no delivery and no explanation. Restate the limit with the real exits (operator `seat rebind` or `seat resolve --new-seat`, pane close, key release) and qualify line 264's "behave exactly as before", which the hold also contradicts; or change the behaviour if the unset recovery is what is wanted.
- Spec 4.6 and the spec Goal require a shell continuity row in C1, a G5 rule beside *Restored-pane retirement* and the G2 extension of the ht-hsyg decision record, and none exists at this head (`structural_shell_continuity` and `restored_target_absent` have zero occurrences), while line 257 already says "shell continuity" does not pass through the hold, naming a path with no definition and no code. Beads ht-ma6i.1 and ht-ma6i.4 own this text, list `TRUST-POLICY.md` in their files, and are open with no human scope change dropping them; these are not baseline rows. Land them, or, if a human drops them, remove the term from line 257.
- `TRUST-POLICY.md:197` says "Exactly four ways" above a table of seven rows, in the invariant that defines how a seat may move; spec 4.6 names this sentence for change and the ht-ma6i.11 sweep checks it, but both say "six", which the head already exceeds and G2 would make eight. Correct the count or drop the number.

## Untested scope

- This review ran nothing; every claim above is from reading the head. The executable pin of this file, `tests/integration/trust_policy.rs`, was not run here.
- `agent_exit_with_pane_open_keeps_the_seat` (`tests/store/restored_pane.rs:671`), the named B3 test, binds a variable `memberships` to `SELECT count(*) FROM retirements`, and nothing in the test reads the `memberships` table, so B3's "and its memberships" is pinned there only by the absence of a retirement job; the behaviour follows from no job running, but the test name and alias claim more than it checks.
- A keyless hook on a held pane has no hook-level case: the `key-conflict.json` fixture in `tests/cli/hook.rs:8282` answers a keyed claim from a scripted daemon, so the quiet outcome the first must-fix item describes is traced from `api_failure`, not observed.
- The cross-node paragraph (line 473) and the mixed-version limit (line 2176) rest on store and two-registry suites; no daemon cluster runs them, and ht-ma6i.11 has not run.

## Deferred OK

- N5 at line 1978 still says a remote seat is retired "only by its home node's `seat.retired`", and lines 504 and 509 say a retirement is "also a `seat.retired` fact", though `seat.retired_reversible` now retires a remote copy and a key holder's restored-pane close writes the reversible row; N4 was updated and these were not. The home-node rule itself holds, so this is rewording for the ht-ma6i.11 policy pass.
- The amendment log (lines 102-111) has no entry for ht-ma6i.6 (two accepted limits) or ht-ma6i.7 (the N4 change and one limit), while every earlier change that added a limit has one.
- Line 220 defines `bound_by` `check_in` as the agent presenting the key "on its own check-in", and line 2169 says a seat gains a key "at keyed check-in or through a reservation", but `settle_key` also binds with `CheckIn` at an enrollment or a plain CLI resolve, as the table row at line 243 admits; reword the definition so the recorded evidence reads honestly.
- *Reversible retirement* omits two agreed behaviours: home-node window mail leaves the seat out of the audience and is unread history with no delivery row on reattach (spec 4.3, task 10's note), and a fact applied during the window has its deadline shifted by the whole window, which the task 8 ledger entry says is in the policy text and is not. One sentence each.
- Line 355's "while nonretired seats exist" and line 361's lift rule read as if a waiting seat counted; line 464 states the exception and the code follows it, so the reader only has to find the later sentence.
