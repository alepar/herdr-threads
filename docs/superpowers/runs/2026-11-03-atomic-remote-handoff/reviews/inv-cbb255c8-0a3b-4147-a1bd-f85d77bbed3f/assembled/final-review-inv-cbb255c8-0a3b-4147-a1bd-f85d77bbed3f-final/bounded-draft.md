# Final review (bounded): epic ht-8lem, atomic remote handoff

Branch `super-auto/atomic-remote-handoff`, base `1e620fdb570ab6e8a254cee334fca7a484d79354`, head
`63e8cae92b423800bac4c0de60de8b8883371ecf` (both verified with `git rev-parse --verify <sha>^{commit}`; caller
checkout at head, clean status at start, not switched).

## Route and scope

Bounded captured-source review. `review-inputs.json` is `status: unavailable`, `sourceAvailable: true`,
`wholeRangeReady: false`, base/head match, 226 changed paths, 0 omitted, 19 provenance gaps retained (listed
under Untested scope). No whole-range package was built or loaded; the failed assembly and the original sidecar
are preserved untouched. Hunks were read from captured Git (`git diff base..head -- <path>`, `git show head:<path>`).

Inspected (see `bounded-coverage.json`, 40 paths, 176 hunks): the sender block and its signing domain, the
`message.sent` authority rule and deadline owner (materializer, migration 0015, receipt due paths, schema
overdue), the sender ledger (`store::handoff_obligations`) and the requester service (`service::peer_handoff`),
the target's decide checks, worker and staging transaction (`service::peer_launcher` non-test hunks,
`store::peer_handoff`, `store::peer_requests`), the coordinator caps and bounds, the CLI parse and run of a remote
handoff (`cli/commands.rs`, `cli/peer_launch.rs`, `cli/mod.rs` run_selected), seat-addressed alerts (attention,
inbox v2, control, cooperative view, output), `sync_admin` and the docs (TRUST-POLICY, SKILL, help, environments).
Read for reference but not claimed as covered: design doc sections 1-7 and Q4 (other sections not read),
`tests/multinode/handoff_atomic.rs` lines 936-1076, test names in `tests/store/handoff_obligations.rs`, and the
test module of `src/sync/handoff_block.rs` past line 330.

## Step 1: own view (spec vs. composed branch)

The composed design holds together on the paths read: the sender signs once and freezes `request_bytes`
(`insert_or_get`, every send replays the stored row), the target checks link node, author, request id, key and
signature before seat placement and writes nothing on refusal, staging writes `thread.direct`, the recipient's
join and the `message.sent` built only through `SenderBlock::expected_payload` in one transaction (hc gate record
in the same commit, `gate_view` releases a handoff row only once the message row exists), every node authorizes
the fact from the fact plus replicated identity (`HandoffRecipientHome`, whole-payload `restates`), and the
deadline owner is `COALESCE(deadline_node, origin_node)` in every due path and both rewritten
`receipt_due_sparse` triggers (existing rows keep NULL `deadline_node`, so the trigger rewrite changes nothing for
pre-0015 data). Ledger transitions are guarded, monotonic and idempotent; the handoff overdue that already fired
stands in for the linked message's receipt overdue only while its condition is open. No Critical or Important
defect was found in the inspected hunks. Findings, all Minor:

- M1 `src/service/peer_handoff.rs:387`: the handoff message id is minted by `public_ids::fresh` checking only
  `messages`; a pending ledger row's `message` is a live id held in no slot, so a later ordinary send (slots
  `send_preparations`, `messages`) can mint the same id before the handoff message replicates, and a second
  handoff hitting a pending row's id fails `insert_or_get` on `UNIQUE(message)` instead of regenerating.
  Probability is about pending-rows x 2^-47.6 per draw, inside the Probabilistic identifiers accepted limit, but
  the allocator's doc ("live ids are checked for collisions in every slot") no longer holds. Fix: add
  `("handoff_obligations","message",prefix::MESSAGE)` to the handoff mint and to `SEND_PREPARATION_SLOTS`.
- M2 `src/service/peer_handoff.rs` `poll`: a terminal row is answered by `ledger_answer` before `record` runs the
  claim check, and `ledger_answer` marks a pending seat-addressed alert shown. Q4 Clearing (a) says only a
  top-level, non-readonly binding clears an alert; the daemon here clears it for any local caller that polls the
  id (the CLI side requires a top-level caller, so only a direct or readonly-seat caller can reach it). Cooperative
  model, low impact; either check the claim (top-level, not readonly) before marking, or record it as an accepted
  limit.
- M3 `src/scheduler/deadlines.rs:412`: `due_handoffs` runs only inside `drive_due` when an invitation or receipt
  phase is due, and its error is propagated with `?`, aborting the rest of the deadline tick (writer acquisition or
  the first `due_rows` read; per-row errors are already logged and skipped). A handoff deadline is therefore
  skipped while both receipt and invitation phases are in failure backoff, and a handoff page read error costs the
  tick's other classes. Fix: log and continue instead of `?`, and run it independent of the two phases' retry gates.
- M4 `src/sync/coordinator/remote_launch.rs` `missing_cap`/`LaunchPlan::required_cap`: a handoff
  `--new-terminal` plan is checked for `peer.handoff.v1` only, not `peer.launch.terminal.v1`. Unreachable in
  production (every node advertising the handoff cap also advertises the terminal cap; only
  `HT_TEST_WITHHOLD_CAPS` can split them), so it is a latent assumption, not a defect today.
- M5 `src/protocol/output_compact.rs` HandoffOverdue: an unseen `failed` row (a refusal or pre-staging failure the
  re-send task learned, with a deadline) renders `not confirmed by that machine`, although that machine did answer;
  `peer list` shows the true `failed: <detail>`. Minor UX; the spec's Q4 framing defines only the two lines.
- M6 `src/protocol/capabilities.rs:14`: the `HANDOFF_ATOMIC_REMOTE_V1` doc says "not advertised until its feature
  lands", but `daemon/control.rs` advertises it. Stale comment.
- M7 `src/cli/peer_launch.rs` handoff `Failed{unconfirmed: true}`: the ledger stores it as terminal `failed`, the
  CLI keeps the request file (unconfirmed), so every rerun for up to 6 days prints the stored failure, then the
  stale-file path asks `resume_only` and gets the same row until it is pruned (7 days after it ended). Consistent
  with the bare-launch rule (never a second agent in a pane that may run one) but not stated for handoffs in the
  spec's "terminal rows" list, which says the CLI settles the file on each terminal answer.

## Step 2: ledger triage (`progress.md`)

Metrics lines are UNAVAILABLE (malformed Finish re-read); absence of findings in the ledger is not read as clean.
No `BLOCKED-AUTH` lines.

- Recurring minor, `[missing-evidence]`, x17 across 17 tasks (and the x3 clusters at lines 85 and 235): process
  class, not a product defect. Reassessed as a group: it means per-task test claims are prose; it raises the weight
  of the outstanding full-suite run at head, nothing else.
- Parked (Task 16, ht-8lem.15, line 157): three parts. (1) the partitioned-overdue test asserted only
  `<= 1` warning and named a possible product gap (receipt overdue filtered on `origin_node`): resolved by later
  tasks; the due filters now use `deadline_owner_sql!`, `overdue_while_partitioned` asserts exactly one warning on
  the sender and none on the target, and `linked_then_overdue_on_the_sender_only` covers the link-first order.
  (2) the staged-message CLI line: now asserted by `fail_after_staging_prints_the_staged_message`. (3) older third
  node: now `older_third_node_fails_then_replays_after_upgrade` via `HT_TEST_LEGACY_MESSAGE_SENT`. Remaining part:
  "recipient ACK clears" is asserted at store level only (`ack_clears_handoff_warning`,
  `cleared_prelink_warning_lets_the_linked_message_warn`), not multinode; untested scope, not must-fix.
- `migration_0015_handoff_obligations` (tests/store/schema.rs:7365) named as failing at several task bases with
  an attribution gap (lines 56, 89, 101, 110, 117, 129, 139, 146, 231): a named failing test is must-fix-class by
  rule, re-checked: it PASSES in the retained sweeps at 1ba75ec1a and 3058a90df
  (`evidence/ht-8lem/full-suite-*.log`), so resolved on the branch; it must be re-confirmed by the full suite at
  head.
- Task 14 `[edge-case-untested]` handoff_warning_of reusing a cleared warning (line 120): fixed, the query now
  joins `clear_warning_id IS NULL`, with a store test.
- Task 14 `[starvation-unbounded-retry]` (line 116): fixed by `due_pass` paging past unmovable rows.
- Task 7 `[idempotent-skips-seat-check]` (line 62): partly fixed (a recorded `seat` is checked first); with no
  recorded seat a repeat naming another seat still gets the staged ids. Deferred, cooperative.
- Task 7 `[idempotent-skips-gate]` (line 61): every hc caller stages with `Some(gate)` in the same commit, the
  Herdr child with `None`; no caller mixes them. Deferred.
- Task 1 `[unmeasured-flake]` codex_schema_tests (line 33): unrelated to the epic, passed on rerun; deferred to the
  flake side quest.
- Task 10 `[dead-code-left]` RemoteLaunch.then / AfterHandoff path (line 136): `then` is now always `None` for
  remote launches from this parse; dead code, follow-up cleanup.
- Task 16 `[slow-test]` overdue_while_partitioned 76 s (line 156): last sweep 238 s < 300 s budget; deferred.
- Remaining minors (doc wrap, comment placement, test seams, duplicated mapping, wall-clock assertions,
  per-call Pacer, FakeHerdr-only env assertion, weak test oracles): deferred, none names a skipped test, a product
  defect or a data-loss path.

## Unreviewed paths (186 of 226)

Source not inspected: scripts/check-wire-compat, scripts/tests/test_check_wire_compat.py, src/app.rs (re-send task
wiring), src/cli/accept_delivery.rs, src/cli/handoff.rs (prompt v2 `render_v2`, `StagingAuthor`),
src/cli/handoff_delivery.rs, src/cli/launch.rs, src/cli/launch_new.rs, src/cli/peer_handoff_launch.rs (the Herdr
staging child, 527 lines), src/cli/topology_handoff.rs, src/cli/topology_handoff/automatic.rs, src/cli/watch.rs,
src/ports.rs, src/protocol/authority.rs, src/protocol/commands.rs, src/protocol/handoff.rs,
src/protocol/handoff/automatic.rs, src/protocol/handoff/automatic_bootstrap.rs, src/protocol/results.rs,
src/service/delivery_pull.rs, src/service/dispatch.rs (PeerHandoffStage / PeerRequestView dispatch),
src/service/mod.rs, src/store/control/direct.rs (`open_pair`, `author_thread_direct`), src/store/mod.rs,
src/store/topology_handoff/automatic.rs, src/store/topology_handoff/runtime.rs, src/sync/mod.rs,
src/test_support/multinode.rs. Partially inspected (test-module and other hunks not listed in coverage):
src/service/peer_launcher.rs, src/sync/coordinator/remote_launch.rs, src/cli/commands.rs (hunks before -2481),
src/cli/mod.rs (hunks before -3262), src/cli/peer_launch.rs (test hunks). All tests/** changes, all
docs/superpowers/runs/** run artifacts (including the design doc), and all evidence/** logs are unreviewed as
diffs.

## Verdict
not ready (bounded review only: 40 of 226 paths inspected, 19 provenance gaps, no full-suite run at head; no Critical/Important defect found in inspected hunks)

## Must fix before landing
- Whole-range final review of 1e620fdb5..63e8cae92 is still outstanding: this bounded pass inspected 40 of 226 paths and cannot clear the branch.
- Run the full suite (`nice cargo nextest run --locked --all-targets --all-features`) at 63e8cae92 plus `scripts/check-no-leaked-processes --run-id`: the last green sweep is at 3058a90df, before the ht-8lem.49, .50, .51, .52 and .53 merges, so no full-suite measurement of this head exists.

## Untested scope
- No full-suite measurement of head 63e8cae92 exists; the caller runs the sweep after this invocation, and the branch is not treated as tested.
- Unreviewed source (no hunk inspected): src/cli/peer_handoff_launch.rs, src/cli/handoff.rs, src/service/dispatch.rs, src/store/control/direct.rs, src/store/topology_handoff/runtime.rs, src/app.rs, src/protocol/results.rs, src/protocol/commands.rs, src/protocol/handoff.rs, src/ports.rs and the other 18 non-test source/script paths listed in "Unreviewed paths".
- Partially inspected source: test hunks of src/service/peer_launcher.rs, src/sync/coordinator/remote_launch.rs and src/cli/peer_launch.rs; src/cli/commands.rs hunks before @@ -2481; src/cli/mod.rs hunks before @@ -3262.
- All tests/** diffs, all docs/superpowers/runs/** artifacts (including the design doc beyond sections 1-7 and Q4) and evidence/** logs are unreviewed as diffs.
- Recipient ACK clearing the in-thread overdue:handoff warning is tested at store level only, not in a multinode run (parked finding of ht-8lem.15).
- The hc worker's handoff branch and the Herdr tab profile env are exercised against fakes (FakeHc, FakeHerdr) only; no live Herdr or herdr-core run, and the pre-release wake e2e with real harnesses has not been run for this branch.
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:69 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:97 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:100 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:102 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:103 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:107 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:108 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: 1e620fdb5 and 63e8cae92 docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-264d-4506-beaa-307c9600bb8d invalid failed assembly archive (failed bounded archive contains successful assembly or split claims).
- Provenance gap: 1e620fdb5 and 63e8cae92 docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58 inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f invalid failed assembly archive (failed bounded archive contains successful assembly or split claims).
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:54 inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb incomplete claimed collection, incomplete claimed final report, missing bounded evidence final-review-inv-632e6c0e-7964-4e4d-99dc-6a1aa9e38efb.md, incomplete assembled bundle digest.md.
- Provenance gap: 63e8cae92:docs/superpowers/runs/2026-11-03-atomic-remote-handoff/run.md:56 inv-583f17bf-39ba-4698-8687-9047316fd37a incomplete claimed collection, incomplete claimed final report, missing bounded evidence final-review-inv-583f17bf-39ba-4698-8687-9047316fd37a.md, incomplete assembled bundle digest.md.
- Provenance gap: the selection's slices directory is empty and digest.md and clusters.json were not written for this invocation.
- Ledger metrics are UNAVAILABLE (malformed Finish re-read), so ledger completeness is unverified.

## Deferred OK
- M1 handoff message id minted against `messages` only (src/service/peer_handoff.rs:387); collision is within the Probabilistic identifiers accepted limit; follow-up: add the `handoff_obligations.message` slot to the handoff mint and to SEND_PREPARATION_SLOTS.
- M2 a terminal-row poll marks the seat-addressed alert shown before any claim check (src/service/peer_handoff.rs poll/ledger_answer); cooperative model, the CLI requires a top-level caller; follow-up or accepted-limit note.
- M3 due_handoffs runs only when a receipt or invitation phase is due and its error aborts the deadline tick (src/scheduler/deadlines.rs:412); follow-up: log and continue, decouple from the phase retry gates.
- M4 a handoff --new-terminal plan is not checked for peer.launch.terminal.v1 (remote_launch.rs missing_cap); unreachable while every handoff-capable node advertises the terminal cap.
- M5 an unseen failed row's inbox alert reads "not confirmed by that machine" (output_compact.rs); peer list shows the true failure.
- M6 stale HANDOFF_ATOMIC_REMOTE_V1 doc comment ("not advertised until its feature lands") in src/protocol/capabilities.rs.
- M7 a handoff Failed{unconfirmed} keeps the request file, so reruns repeat the stored failure until the row is pruned; matches the bare-launch rule, spec wording should say so.
- Recurring [missing-evidence] (x17 across 17 tasks plus the x3 clusters): process class, no product impact; compensated only by the full-suite run at head.
- migration_0015_handoff_obligations attribution-gap minors: test passes in the retained sweeps at 1ba75ec1a and 3058a90df; re-confirm at head with the full suite.
- Task 7 idempotent-skips-seat-check (partly fixed) and idempotent-skips-gate: no caller mixes gate and gate-less staging; cooperative model.
- Task 10 dead RemoteLaunch.then / AfterHandoff path for remote handoffs: cleanup follow-up.
- Task 16 slow overdue_while_partitioned (76 s): suite at 238 s, under the 300 s budget.
- Task 1 unmeasured codex_schema_tests flake: unrelated to this epic, belongs to the flake side quest.
- Remaining task minors (doc wrap, misplaced comment, test seams in product files, duplicated answer mapping, wall-clock assertions, per-call Pacer, FakeHerdr-only env assertion, weak request_key oracle, duplicate startup prune, Task 11 sender-seat state check by is_local/home_node only).
