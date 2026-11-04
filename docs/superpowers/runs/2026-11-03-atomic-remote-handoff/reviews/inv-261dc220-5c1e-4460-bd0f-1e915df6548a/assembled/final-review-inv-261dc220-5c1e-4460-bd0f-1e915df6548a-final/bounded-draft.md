# Final review (bounded): ht-8lem atomic remote handoff

Invocation inv-261dc220-5c1e-4460-bd0f-1e915df6548a. Base 1e620fdb570ab6e8a254cee334fca7a484d79354, head a5e201c53feb8626ec86ba905fb72bd3ec7cedcb (both verified as commits; caller checkout at a5e201c53, clean, never switched). Bounded route: review-inputs.json has status unavailable, sourceAvailable true, wholeRangeReady false, 217 selected, 0 omitted, 18 gaps. No whole-range package was built or loaded.

## Scope of this pass

The range has 217 changed paths and 239 commits. Earlier bounded passes (inv-632e6c0e, inv-583f17bf, inv-b56f0808, inv-dd32bbd0, inv-88ad8555) and the coordinator's independent review at cb122a5ca came before this one. This pass focused on the fix round cb122a5ca..a5e201c53 (ht-8lem.49, .50 and .51) and on the carried must-fix items. It read these whole from captured Git: the new-file hunks of src/store/handoff_obligations.rs, src/service/peer_handoff.rs, src/store/peer_handoff.rs and migrations/node/0015_atomic_remote_handoff.sql, plus the named hunks in bounded-coverage.json. To check the env path it also read, outside the changed range, native.rs create_tab, transport.rs tab.create and launch_directories.rs validate_bootstrap.

## Step 1: own view

- ht-8lem.49 (BLOCKING 1, --profile-dir on Herdr targets) looks fixed as specified. The remote --pane path with --profile-dir is refused by the CLI, which now applies the guarded-pane check to remote targets too, and by the target: plan_refusal's PANE_PROFILE_DIR, 49 bytes. A remote new tab now carries the frozen LaunchDirectoryContext through `internal peer-target-create --directory-context`, and create_with puts context.env into CreateTargetRequest.env. The Herdr transport sends env in tab.create, and NativeCli::create_tab checks env against the context with validate_bootstrap. The worker freezes context.cwd from plan.cwd and passes the same plan.cwd as --cwd, so the cwd agreement check holds. hc is unchanged (context None, root in gate argv). LaunchPlan.profile_dir is new on this branch, so the new pane refusal changes no released behaviour.
- ht-8lem.50 (SHOULD-FIX 2 and 3) looks fixed in the store. ALERT_PENDING_SQL, the partial index and the WAKE_PROBES probe now agree on `overdue_seq IS NOT NULL AND thread_warned_at IS NULL AND alert_shown_at IS NULL AND state!='linked'`. A late Launched{seat} without a held pair thread therefore keeps the alert, and ledger_rows shows launched rows that are overdue but not yet warned. due_pass pages with a (deadline_at, request) cursor and counts only advanced rows against the limit, so rows it cannot advance no longer starve the batch. Per-row errors are isolated in due_pass, in expire_due (one savepoint per row) and in resend_pass.
- ht-8lem.51 optional items look done: v3 request key for plain launch, hourly prune in resend_pass, the shared node_keys helper (newest incarnation first; the materializer now accepts any incarnation's key), the stage() seat-mismatch check before the idempotent return, no empty message=, and a non-UTF-8 --cwd or --profile-dir refused for a remote target.
- The earlier must-fix "poll after wait_slot recomputes the send window" is fixed: poll calls send_window(budget) again after the slot wait.
- New finding (should-fix, model-visible text): the seat-addressed alert now also covers launched rows, where the target did confirm and returned a seat, but output_compact.rs still renders every InboxBatchV2Item::HandoffOverdue as `warning overdue_handoff <req8> to <tag>: not confirmed by that machine`. A sender whose agent did start is told the target never confirmed it, which invites a duplicate handoff with new intent text. The alert row carries no state or seat, so the renderer cannot tell the two cases apart. The text needs a launched variant (the peer list already has `overdue unlinked: seat <seat>`), or the item needs to carry the state.
- New finding (minor): Requester::ledger_answer still marks an alert shown only when `row.seat.is_none()`, the old predicate. A poll that shows an expired-unlinked row (one with a seat) leaves its alert pending, so inbox shows it again. The condition should read the row's alert_pending().
- New observation (minor): the in-place edit of migration 0015 changes the partial index handoff_obligations_alert. The migration is unreleased, so no shipped DB is affected. A dev store migrated by an earlier branch build keeps the old index definition, though, and the WAKE_PROBES query uses `INDEXED BY handoff_obligations_alert` with the new WHERE clause. SQLite rejects that with "no query solution" when the partial index does not cover the query. Such stores need re-creating.
- Carried must-fix, still open at a5e201c53: resend_set and resend_pass replay the sent|launched rows of a retired author. resend_set has no author-state filter, and nothing on retirement touches handoff_obligations (the only users outside the module are listed in this review's grep). The target refuses only once the retirement fact has replicated there (SENDER_SEAT_RETIRED). A link-up re-send that wins the race against the replication of the retirement fact therefore starts an agent and authors a message for a retired author. The alert predicate does filter retired authors, but the re-send path does not. TRUST-POLICY records no accepted limit for this.
- expire_due retries a row whose due step keeps erroring on every pass (every 2 s), with one daemon.log line each time: log noise, not a correctness problem.

## Step 2: ledger triage

- Recurring minor [missing-evidence] (ht-8lem.40, .37 and .41, recurring again in .42, .44, .47, .50 and .51) is process-class and does not block landing. Code-level it is covered only by the requirement for an unfiltered suite at head.
- ht-8lem.15 parked (recipient ACK clearing not asserted at multinode level) is deferred OK, as in earlier passes. Clearing is covered at store and service level.
- ht-8lem.49 minor [fake-only-assertion]: the tab env is asserted at FakeHerdr tab.create, not in a live Herdr shell. This is untested scope, not a must-fix: the wire mapping (transport.rs tab.create env) is unchanged code shared with local --new-tab.
- ht-8lem.51 minor [weak-test-oracle]: request_key_v3_for_a_plain_launch recomputes the hash from the same serialization, so it does not prove byte identity with a 2.x key. Deferred OK; a golden 2.x key fixture would close it.
- ht-8lem.51 minor [duplicate-work] (prune at startup and on the first pass) is deferred OK and harmless.
- ht-8lem.44 minor [per-call-pacer] / ht-8lem.41 [clock-bypass]: wait_slot still uses SystemClock and Instant. Deferred OK.
- ht-8lem.37 minors (duplicated deadline-owner trigger SQL, out-of-scope migration edit, removed assertion now covered elsewhere) are deferred OK.
- There are no BLOCKED-AUTH lines. The Metrics lines are UNAVAILABLE (malformed ledger), so absence there is not read as clean.
- Cleanup lines for .49, .50 and .51 report stop-run-processes exit 4 on uninspectable non-run pids 1950, 2008, 6582 and 976133. The task worktrees and /tmp/sp-e3317cf8/ht-8lem.49, .50 and .51 were kept, so caller cleanup is still to do.
- resumeChange planned a scoped PR roast of the fix round. run.md at head records none.

## Unreviewed (bounded route)

- Every changed path and hunk not listed in bounded-coverage.json is unreviewed by this pass. That includes the other hunks of src/cli/commands.rs, src/cli/peer_launch.rs and src/service/peer_launcher.rs, all of src/cli/peer_handoff_launch.rs, src/sync/handoff_block.rs, src/sync/coordinator*.rs, src/sync/frame.rs, src/store/schema.rs, src/store/receipts.rs, src/store/peer_requests.rs, src/store/control*.rs, src/store/messages.rs, src/service/sync_admin.rs, src/cli/handoff.rs and src/cli/topology_handoff.rs, TRUST-POLICY.md (read only as fix-round diff text), every test file, and the docs, evidence and run artifacts.
- Provenance gaps (18): run.md@a5e201c53 lines 61, 89, 92, 94, 95 and 99 have a malformed collection record. The inv-8759cc82 and inv-b6e4e8fd failed assembly archives (2026-10-17 run.md:51 and :58, at base and head) are invalid. inv-632e6c0e (run.md:47) and inv-583f17bf (run.md:49) each have an incomplete claimed collection, an incomplete final report, missing bounded evidence and an incomplete digest.md bundle.

## Verdict
not ready (retired-author re-send still open, misleading launched-alert text, no full-suite run at head, whole-range review incomplete)

## Must fix before landing
- src/store/handoff_obligations.rs resend_set / src/service/peer_handoff.rs resend_pass still re-send the sent|launched rows of a retired author. There is no author-state filter and no retirement hook into the ledger, so a link-up re-send can reach the target before the retirement fact does and start an agent for a retired author. Filter retired authors in resend_set, or fail or expire their rows at retirement, and add a test, or record an accepted limit in TRUST-POLICY. Carried from inv-583f17bf, inv-dd32bbd0 and inv-88ad8555.
- src/protocol/output_compact.rs:94 renders every seat-addressed handoff alert as "not confirmed by that machine". Since ht-8lem.50 the alert also covers launched rows whose target confirmed and named a seat, so the sender is told something false. Give the launched case its own text, for example with the seat, as the peer list's `overdue unlinked: seat <seat>` does, or carry the state in HandoffOverdue. State the byte size.
- Run the unfiltered full suite on a5e201c53 with HT_LEAK_RUN_ID, then scripts/check-no-leaked-processes --run-id. The caller deferred it, so this head has no full-suite measurement; the last green sweep was at 3058a90df, before the ht-8lem.49 to .51 fix round.
- Complete a whole-range final review: this bounded pass covered 11 of 217 paths, and 18 provenance gaps are open.
- Land through a squash or a cleaned history. The branch carries duplicated no-op and non-compiling intermediate commits (roast nit src/cli/handoff.rs:840), which break bisect on main.

## Untested scope
- whole-epic review incomplete; bounded inspected hunks only; all remaining hunks and provenance gaps remain unreviewed
- Full suite, leak check and temp-file check are unmeasured on a5e201c53; ht-8lem.50 kept only gate2.log (2 multinode tests), and ht-8lem.49 and .51 have prose-only evidence.
- The remote --new-tab profile env is asserted at FakeHerdr tab.create, never in a live Herdr shell (ht-8lem.49 minor).
- No test covers a re-send for an author who retired while the target was offline.
- The scoped PR roast of the ht-8lem.49 to .51 fix round named in resumeChange has no record in run.md at head.
- No dispatch-level test of PeerHandoffStage and PeerRequestView routing (ht-8lem.7, .8), no in-process test of the hc worker handoff branch (ht-8lem.9), no coordinator test that a peer without peer.handoff.v1 receives no frame (ht-8lem.4), and no CLI test of inbox seat alerts (ht-8lem.21); carried, not re-verified.
- Recipient ACK clearing of a handoff warning is asserted at store and service level only, not multinode (ht-8lem.15 parked).
- Process and worktree cleanup for ht-8lem.49 to .51 is incomplete (stop-run-processes exit 4 on uninspectable non-run pids); task worktrees and /tmp/sp-e3317cf8/ht-8lem.49 to .51 were kept.

## Deferred OK
- Requester::ledger_answer marks an alert shown only when row.seat is None, the pre-ht-8lem.50 predicate. A polled expired-unlinked row keeps its alert, and inbox shows it once more.
- Migration 0015 was edited in place (the handoff_obligations_alert partial index). It is unreleased, but a dev store migrated by an earlier branch build fails the INDEXED BY wake probe and must be re-created.
- expire_due retries a row whose step keeps erroring every 2 s with one daemon.log line each time (log noise only).
- request_key_v3_for_a_plain_launch recomputes from the same serialization rather than a golden 2.x key (ht-8lem.51 minor).
- Prune runs at startup and again on the first re-send pass (ht-8lem.51 minor, harmless).
- wait_slot uses SystemClock and Instant instead of the injected clock (ht-8lem.41 and .44 minors).
- Recurring missing-evidence minors across tasks are process-class.
- The HANDOFF_ATOMIC_REMOTE_V1 doc comment says "not advertised until its feature lands", but the daemon advertises it unconditionally (stale comment, carried; not re-inspected).
- Seat-addressed alerts reach hooks and channels only as the generic attention count and need one inbox call (spec Q4 accepted consequence).
- Earlier deferred items, dispositions unchanged and not re-inspected here: duplicated deadline-owner trigger SQL, dead RemoteLaunch.then paths, stale SKILL.md launch-prompt text, and no dated Amended entry for TRUST-POLICY N4, N5 and N15.
