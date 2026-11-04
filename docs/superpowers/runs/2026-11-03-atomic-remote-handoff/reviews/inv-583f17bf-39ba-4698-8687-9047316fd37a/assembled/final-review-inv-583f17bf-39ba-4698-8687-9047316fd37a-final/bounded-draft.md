# Final review (bounded): epic ht-8lem, atomic remote handoff

Branch `super-auto/atomic-remote-handoff`, captured source base `1e620fdb570ab6e8a254cee334fca7a484d79354`, head `aaefcd47f280fdf72bfba3a03550a02b05e48610` (both verified with `git rev-parse --verify <sha>^{commit}`). Caller checkout observed at HEAD `aaefcd47f`, working tree clean; it was not switched or modified. The review-source binding was appended to the ledger by `bind-review-source` (exit 0).

This is a BOUNDED review. The review-input sidecar reads `status: unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, 183 changed paths, 0 omitted, 11 provenance gaps. No whole-range package was built or loaded. Only the hunks listed in `bounded-coverage.json` (52 hunks in 11 paths) were inspected from captured Git; everything else is unreviewed. A whole-range review remains outstanding. No full-suite measurement of this branch exists (the sweep is deferred to the caller), and this review ran no tests.

## Scope inspected

Focus: the fixes landed since the previous bounded review (inv-632e6c0e at head 40283c947) and since code roast round 2 (67053aeb2), plus the seams they touch. Read from captured Git: the requester and re-send task (`src/service/peer_handoff.rs`, lines ~76-110 and ~200-700; the file is one new-file hunk, its head and tail were not read), the deadline-owner switch (`src/store/receipts.rs`, migration 0015 triggers, `src/store/messages.rs`, `src/store/materialize.rs`, `src/sync/facts/message.rs`, `src/sync/facts/mod.rs`), the overdue writer and `handoff_warning_of` (`src/store/schema.rs` hunks listed in coverage), the RemoteLaunch handler (`src/service/sync_admin.rs`) and transport bound (`src/daemon/transport.rs`, unchanged), the CLI loop (`src/cli/peer_launch.rs` non-test hunks), and TRUST-POLICY.md. Read for context but not claimed as covered: the 0c5c27e1a diff of `src/service/peer_launcher.rs`, `resend_set`/`due_rows`/`ALERT_PENDING_SQL` in `src/store/handoff_obligations.rs`, `restates` in `src/sync/handoff_block.rs`, the new tests of 09d27f3ef, the roast round 2 report and punch list, and the previous bounded draft.

## Prior must-fix items: status at head

- Receipt due scans keyed on `origin_node` (previous Must fix 1, roast Blocking receipts.rs:1019): fixed by e125902ae. `PHYSICAL_DUE_SELECTION`, the `receipt_due_sparse` repair insert, `scan_extension_lapses` and both `receipt_due_sparse` triggers (migration 0015) now use `COALESCE(deadline_node, origin_node)` via `deadline_owner_sql!`. `messages.deadline_node` is set from `payload.handoff.sender_node` on every node that applies the fact, and the sender's materializer links the ledger row in the same transaction. No remaining `origin_node` deadline filter found in `src/` or `migrations/` for receipts (invitations keep `origin_node`, correctly).
- `migration_0015_handoff_obligations` failing deterministically (previous Must fix 2): fixed by 061851453 per the roast round 2 tracking; the fixture was not re-read here.
- Retired sender seat accepted by the target (previous Must fix 3): the target half is fixed by 0c5c27e1a (`SENDER_SEAT_RETIRED` refusal in decide, with a test). The sender half is not (see Important finding below).
- Roast round 2 Should-fix `Requester::poll` reusing a stale send window: fixed by 09d27f3ef. `send_window` is recomputed after the slot wait and capped at what the budget leaves after `REPLICA_WAIT_MS + SETTLE_MARGIN_MS`; a window used up by the slot wait answers Timeout without sending. With the 17 s RemoteLaunch bound: slot wait <= 10 s, then send <= ~1 s, then replica wait 5 s, inside the bound. The new real-time test covers both the slot-never-freed and slot-freed-late paths.
- Roast round 2 Should-fix `handoff_warning_of` reusing a cleared pre-link warning: fixed by 9987863fd (join on `warning_conditions.clear_warning_id IS NULL`), with a store test.

## Findings (own view, step 1)

### Important: the sender keeps re-sending a retired author's handoff, so the target-side retired check loses the race it exists for
- `ledger::resend_set` (`src/store/handoff_obligations.rs` ~l.488) selects every `sent|launched` row with no author-state filter, and `Requester::resend_pass` (`src/service/peer_handoff.rs` ~l.597-640) replays it immediately when the target becomes newly linked (`newly.contains(&node)`). Only the CLI path (`record`) and `open_pair`/`ALERT_PENDING_SQL` check retirement.
- The target's new `SENDER_SEAT_RETIRED` refusal only fires if the author's `seat.retired` fact has already replicated there. In the scenario the previous review named (target offline, author retired, target links later), the re-send frame goes out on the first pass after link-up, concurrently with journal catch-up, so the target can still verify the block, stage a task message from a retired seat and launch an agent.
- Failure scenario: seat A runs `handoff --pane P@M` while M is offline; A is retired; M links; the re-send reaches M before the retirement fact; M launches an agent whose task names a retired sender, and the sender never opens the pair thread (`open_pair` skips a retired author). The ledger row also stays `sent|launched` until the 6-day expiry, re-sent every 30 s.
- Fix shape: filter retired authors out of `resend_set` (or expire/fail rows when their author retires) and add a store or service test.

### Minor (own view)
- After 9987863fd, `handoff_warning_of` is close to redundant: an open handoff warning has `condition_kind='receipt'`, `condition_id=<recipient seat>` in the same pair thread, so the `active` query already finds it. The only case where it now changes the outcome is a condition covered by a `warning_close_sweeps` row (which `active` excludes and `handoff_warning_of` does not), where it returns a swept warning instead of letting a new one open. Narrow; worth a comment or aligning the two predicates.
- `settle` reports `Timeout { secs: wait.as_secs() }`; with the new budget-cut window a sub-second send reports `no answer in 0s`. Cosmetic.
- `wait_slot` uses wall-clock `Instant` while `send_window` uses the injected clock (already deferred as `[clock-bypass]` at ht-8lem.41); the new test works around it with a wall-following test clock.
- TRUST-POLICY N15 *Remote handoff* lists the checks the target makes ("the link's node and the sender seat's replicated home"); the new retired-seat refusal is not named there. Doc-only.

## Ledger triage (step 2)

- Recurring minor `[missing-evidence]` (ht-8lem.40, .37, .41; earlier .1, .7, .3 and others): process-only, Deferred OK. Note that ht-8lem.42 also records `[cleanup-inspection] leak check could not run`, so test hygiene of the last two fix tasks is unverified: untested scope, not a code finding.
- ht-8lem.37 `[out-of-scope-file]` migration 0015 changed for the triggers: correct and needed (the triggers are part of the N5 fix); Deferred OK.
- ht-8lem.37 `[duplicated-sql-fragment]` trigger copies of the deadline-owner fragment: guarded by `deadline_owner_triggers_use_the_fragment`; Deferred OK.
- ht-8lem.37 `[removed-assertion]` handoff_pane_atomic lost the target-records-no-overdue assertion, said to be covered by `linked_then_overdue_on_the_sender_only`: not verified here; untested scope.
- ht-8lem.37 `[missing-evidence]` on the migration_0015 failure: resolved by 061851453.
- ht-8lem.39 `[comment-formatting]`, ht-8lem.40 `[missing-evidence]`, ht-8lem.41 `[clock-bypass]`, ht-8lem.42 `[missing-evidence]`: Deferred OK.
- ht-8lem.15 parked finding (overdue acceptance "recipient ACK clears" not asserted; link-before-Launched in-thread warning): the product defect behind it is fixed by e125902ae; the "recipient ACK clears" clause remains unasserted at multinode level. Untested scope.
- Roast round 2 open Nit: `overdue_while_partitioned` (tests/multinode/handoff_atomic.rs ~l.886-937) never forces the Launched-before-link order, so its claimed kill of a broken re-send is vacuous and its header overclaims. No commit after 67053aeb2 touches it. Untested scope (store-level tests cover the Launched-first path).
- Roast round 2 unverified Nit: `linked_then_overdue_on_the_sender_only` races a fixed 8 s deadline across the cross-process pipeline; a starvation flake risk. Deferred OK, track.
- Roast punch list (6 Nits: handoff_obligations.rs:503, :339; peer_launch.rs:291; SKILL.md:64; peer_handoff.rs:606; handoff.rs:840): open, Deferred OK.
- Earlier items carried from the previous bounded review (resend_pass aborts the whole pass on one settle error; `stage` relies on `debug_assert!` for `restates`; `required_cap` checks only `peer.handoff.v1` for a new-terminal handoff; `ledger_answer` marks a shown failure only when already overdue; due_rows starvation by `launched` rows without a held pair thread): not re-verified at head beyond resend_pass (still `?` inside the loop); Deferred OK, except that the starvation item shares its fix with the retired-author item above.
- Ledger cleanup lines for ht-8lem.37 to .43 record `stop-run-processes` exit 4 (pids 1950, 2008, 6582, 976133 not inspectable) and kept task worktrees under `.worktrees/super-auto-atomic-remote-handoff--task-ht-8lem.*` and `/tmp/sp-e3317cf8/ht-8lem.*`: a cleanup the caller still owes (the /tmp copies hold RAM), not a code finding.
- Metrics lines are UNAVAILABLE (ledger re-read malformed), so absence of a record in the ledger is not read as clean. No BLOCKED-AUTH lines.

## Unreviewed (bounded route)

- Provenance gap: `aaefcd47:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md` lines 44, 72, 75: malformed collection records (outside approvals or invalid prefix).
- Provenance gap: `docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51` (inv-8759cc82-264d-4506-beaa-307c9600bb8d) and `:58` (inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f): invalid failed assembly archives, at base and head.
- Provenance gap: `run.md:33` inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb: incomplete claimed collection, incomplete claimed final report, invalid failed assembly archive (missing bounded evidence final-review-inv-632e6c0e-...md), incomplete assembled bundle (digest.md).
- Unreviewed product code at this invocation: `src/store/handoff_obligations.rs` (read only in parts), `src/store/peer_handoff.rs`, `src/sync/handoff_block.rs` (only `restates` read), `src/service/peer_launcher.rs` (only the 0c5c27e1a diff read), `src/sync/frame.rs`, `src/sync/coordinator.rs`, `src/sync/coordinator/remote_launch.rs`, `src/store/attention.rs`, `src/scheduler/deadlines.rs`, `src/cli/handoff.rs`, `src/cli/commands.rs`, `src/cli/mod.rs`, `src/cli/peer_handoff_launch.rs`, `src/cli/topology_handoff.rs`, `src/protocol/*`, `src/store/control.rs`, `src/store/control/direct.rs`, `src/store/topology_handoff/runtime.rs`, `src/service/dispatch.rs`, `src/app.rs`, `src/daemon/*`, `src/ports.rs`, the remaining `src/store/schema.rs` and `src/service/sync_admin.rs` hunks, and every other one-line change. Several of these were covered by the previous bounded review at 40283c947 but are not re-claimed here.
- Unreviewed hunks in inspected files: `src/service/peer_handoff.rs` lines ~1-75, ~110-200 and ~700-729 of its single new-file hunk; the test hunks of `src/cli/peer_launch.rs`.
- Unreviewed tests: all of `tests/`, including `tests/multinode/handoff_atomic.rs`, `tests/service/peer_handoff.rs` (only the 09d27f3ef test read), `tests/store/handoff_obligations.rs`, `tests/daemon/peer_handoff_decide.rs`, `tests/store/schema.rs`, and the golden handoff fixtures.
- Unreviewed docs: the design doc, roast/coverage artifacts beyond roast round 2 and its punch list, `docs/environments.md`, `integrations/skill/SKILL.md`, `evidence/ht-8lem.4/lib-tests.log`.

## Verdict
not ready (bounded review only; retired-author handoffs are still re-sent and can race the target's new refusal; no full-suite run)

## Must fix before landing
- resend_set in src/store/handoff_obligations.rs re-sends sent|launched rows of a retired author, and resend_pass fires on link-up before the retirement fact replicates, so the target's SENDER_SEAT_RETIRED refusal can be bypassed in the offline-target case; filter retired authors from the re-send (or fail/expire their rows) and add a test.
- Whole-range review still outstanding: this bounded review covered 52 hunks in 11 of 183 paths, with 11 provenance gaps unresolved.

## Untested scope
- No full-suite run of this branch exists; the caller's sweep (cargo nextest run --locked --all-targets --all-features, then scripts/check-no-leaked-processes) is still owed, so the branch is not treated as tested.
- ht-8lem.42 leak check could not run, so process and temp-file hygiene of the latest fix tasks is unverified.
- overdue_while_partitioned never forces the Launched-before-link order (roast round 2 Nit, open); its re-send coverage is vacuous at multinode level.
- The overdue acceptance clause "recipient ACK clears" is not asserted at multinode level (ht-8lem.15 parked).
- handoff_pane_atomic lost its target-records-no-overdue assertion (ht-8lem.37); claimed covered by linked_then_overdue_on_the_sender_only, not verified here.
- No dispatch-level tests for PeerHandoffStage / PeerRequestView routing and elected_writer authority (ht-8lem.7, .8), no in-process test of the hc worker handoff branch (ht-8lem.9), no coordinator test that a handoff to a peer without peer.handoff.v1 sends no frame (ht-8lem.4), no CLI test that inbox requests and advances seat alerts (ht-8lem.21): carried from the previous bounded review, not re-checked.
- All unreviewed paths, hunks and provenance gaps listed under "Unreviewed (bounded route)".

## Deferred OK
- handoff_warning_of is now nearly redundant with the active-condition query and differs from it only on swept conditions; align or comment.
- Timeout answers report "no answer in 0s" for a sub-second budget-cut send window.
- wait_slot uses wall-clock Instant rather than the injected Clock (ht-8lem.41 clock-bypass).
- TRUST-POLICY N15 Remote handoff does not name the new retired-sender refusal among the target's checks.
- linked_then_overdue_on_the_sender_only races a fixed 8 s deadline (starvation flake risk; track).
- Six open roast punch-list Nits (handoff_obligations.rs:503 and :339, peer_launch.rs:291, SKILL.md:64, peer_handoff.rs:606, handoff.rs:840).
- resend_pass aborts the whole pass on one row's settle error; stage relies on debug_assert for restates; required_cap checks only peer.handoff.v1 for a new-terminal handoff; ledger_answer marks a shown failure only when already overdue.
- Duplicated deadline-owner SQL in the migration 0015 triggers (guarded by a pinning test).
- Recurring missing-evidence process findings and the comment-formatting nit.
- Kept task worktrees and /tmp/sp-e3317cf8/ht-8lem.* directories from incomplete cleanup (caller cleanup, not code).
