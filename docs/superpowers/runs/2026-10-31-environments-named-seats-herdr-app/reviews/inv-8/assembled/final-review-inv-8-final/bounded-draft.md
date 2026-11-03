# Final review, epic ht-kqrt (bounded captured-source review, inv-8)

Captured source: base de5af6736e2cd4f48593d7be4035d0565cb7fa85, head 573c76b5e607f0a72d4d8554b24473c5b28e6c5c. Both verified with `git rev-parse --verify`. The base equals local `main`, so the branch is not behind and migration 0013 is still the next free slot. The caller checkout was on the captured head and clean at the start and when this draft was written; no caller commit or status change was observed.

Bounded because the selection sidecar is `status: unavailable` with `sourceAvailable: true`, `wholeRangeReady: false`, no omissions and four retained provenance gaps. The failed assembly and its sidecar are preserved. `bind-review-source` succeeded before any artifact was written.

## Summary

One new product defect, two carried. The read-only rule (D3) was implemented only on the V1 pull. The V2 presentation path, which serves a Claude mod's watch and agy's hooks, has no read-only filter, so TRUST-POLICY's statement about it is false for those harnesses and a read-only mod session retries and then stalls. The launch-gate config root and the stale-publication promotion from inv-7 are both still present at this head: no bead was filed for either, and `ht-kqrt.219` did not change the line in question.

`ht-kqrt.219` and `.220` address the sweep-1 failures as far as reading can tell. Sweep-1 ran at `9296918db`: 8903 of 8906 passed, 54 skipped, 341 s, leaks clean. Its three failures are those two beads. The three zsh tests behind inv-7's parked item passed there, so that item reduces to a green run of this head.

No full-suite measurement of this head exists. The sweep is deferred to the caller, so nothing here treats the branch as tested.

## Step 1: own view

Order, stated plainly: the epic spec and the inv-7 verdict and must-fix list were read first, in order to re-verify that list at this head and to aim at paths no earlier invocation had inspected. The ledger was grepped once during step 1, to learn whether the read-only limit was already recorded. It was otherwise read in step 2.

Coverage was chosen from the union of the seven earlier bounded coverage files: 80 of the 176 product paths had never been listed, so this pass went there first.

- Read-only delivery (new finding). `binding.readonly` is read in exactly one delivery site, src/service/delivery_pull.rs:605, inside the V1 `delivery_pull`. `presentation_pull` in src/service/delivery_presentation.rs:85 (a file this epic did not change, which is why no task review saw it) reads no such flag, and neither do its store readers. It is the path of `PresentationConsumer::Watch` (src/cli/watch.rs:689, chosen whenever the daemon advertises presentation) and of `PresentationConsumer::Hook` from agy (src/harness/agy/delivery.rs:812, :1089). The historical `watch_loop` fallback pages `InboxBatchV2`, unfiltered too. Confirmation still reaches `settle_pending`: `presentation_confirm` handles controls and invitations only, and message receipts settle through `AckModDelivered` or `DeliveryConfirm`. So no ACK is wrongly discharged. The defect is what happens next, set out under Must fix.
- `settle_pending` read-only branch. It returns `newly` as the skipped set, which matches the function's contract, and `ack_displayed` then reports nothing acknowledged. `SettleHow::Acked` is untouched, so an explicit `ack` works, as TRUST-POLICY says.
- Named lane, `ba6a96daa`. `begin_pass` and the `failing` check do what the commit says, and the skip path's unset `available` is harmless (`is_available` has no production caller). `promote_live` is still unconditional, under Must fix.
- Seat store split. Lines in src/store/seats/*.rs that are absent from base src/store/seats.rs were read: 644 of about 7,900, the rest moved verbatim. `guard_live_holder`, `self_retire`, `refuse_cross_kind_rebind`, `hand_back_name`, `restore_own_seat`, `named_seat_apply` and `record_locator_hint` read as the spec states.
- Retirement. `begin_retirement_authored` refuses `Closed` on a named seat. `end_local_seat` and `settle_local_seat` move the fence's body unchanged in effect, and the replayed `seat.retired` takes the same transition.
- Restore and parking. `PARKED_READY` agrees with the materializer per park, and the attempted-set guard stops a spin. The held-for-operator case has no operator verb, under Deferred OK.
- V3 handoff protocol and store. The untagged `AutomaticCreation` decodes V2 bytes to `Tab` first, V2 digests are preserved by the skipped `placement`, `creation_scope` and `validate_guard` bind a V3 creation to its witness, and created-pane recovery is refused. Operator recovery of a V3 attempt works with `--host-endpoint` naming the core, and tests cover it.
- Wire and migration. Base ignores a non-null `display_name`, so a new sender is safe beside an old peer. `LaunchTarget::NewTerminal` carries its cap and is sent only to a peer that advertised it. The 0013 backfill cannot trip its CHECK. `local_launch_gates` prunes itself at each open.
- Daemon seams. `new_guarded` now answers capabilities per call, `request_bound` clamps before validation, the kernel peer pid is plumbed, and `archival` filters on evidence. `probe_kind` stays inside its budget, with a nonblocking connect and a re-armed read timeout. `HcAddress` resolves as specified.
- Repo gates run here: `scripts/check-harness-neutral` ok (0 violations) and `scripts/check-wire-compat` ok (0 violations, 0 stale), both at head.

## Step 2: ledger triage

The ledger has 1652 lines and 369 `minor (deferred)`, no `parked finding` lines beyond the parked completions inv-7 closed out, seven `Recurring minor:` clusters, no `Recurring blocker:` and no BLOCKED-AUTH. The Finish metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing.

- Clusters first. All seven are process classes, six `missing-evidence` and one `suite-not-run`. As grouped observations they say evidence was reported as prose or tmpfs logs rather than bound to a source tree. Each finding keeps its identity and scope; none changes a product answer, and the deferred sweep answers them.
- Misgraded and must-fix: Task 25 `[unverified-mod-confirm]` names a known product defect, confirmed here and widened by the V2 finding. Task 57 `[config-root-optional]` and Task 125 `[missing-unit-test]` stay misgraded as inv-7 found.
- New since inv-7: Task 125 `[suite-not-run]`, Task 129 `[missing-evidence]`, Task 130 `[missing-base-red-run]` and `[rig-daemon-herdr-calls]`. None is must-fix. The last is test-rig only and noted below.
- Parked completions: unchanged since inv-7's triage, and no new parked item this invocation.

## Verdict
not ready (read-only filter missing on the V2 delivery path, launch gate config root, stale publication promoted by the named lane, no full-suite measurement of this head, whole-range review outstanding)

## Must fix before landing
- Read-only bindings on the V2 delivery path (new; ledger Task 25 misgraded minor): the D3 filter exists only in the V1 pull (src/service/delivery_pull.rs:605), while `presentation_pull` (src/service/delivery_presentation.rs:85) offers ACK-required rows to a read-only Claude mod watch and to read-only agy hooks, against TRUST-POLICY.md:2049-2051; for the mod, `settle_pending` skips (src/store/receipts.rs:741) so the row reports `Retryable/Busy` (:465), stays in `rec.unacked` and is retried at least every 30 s for as long as it is pending (register.js:18, :527, :1035), `record_ack` never runs (`counts_as_mod_ack`, src/protocol/watch.rs:593), and once the seat's receipt is 10 min old `stalled_judgement` (src/service/mod_channels.rs:556, via `mod_stall_oldest`) closes the channel `Stalled` and starts the cooldown, repeatedly; apply the filter in the presentation reader, exclude read-only bindings from the stall judgement, and add a test through the Watch and Hook consumers.
- Launch gate config root (carried from inv-7, ledger Task 57, re-verified at this head): `gate_argv` in src/cli/topology_handoff/automatic.rs:395 adds `--config-root` only for an explicit `directory_context`, src/service/peer_launcher.rs:672 never does, src/cli/launch_new.rs:132 passes `directory_context: None`, and no adapter overrides `child_scrub_vars` (src/harness/adapter.rs:54), so `prepare` in src/cli/launch_gate.rs leaves a leaked `CLAUDE_CONFIG_DIR` in place; a core autostarted under profile A runs a `launch --new-terminal` from profile B under A's account and hooks, against spec gate step 3 (design line 839), and TRUST-POLICY records no limit; pass the caller's effective root on every gate argv and scrub the adapter's root variables, or record the limit with a human decision.
- Named lane promotes a stale publication (carried from inv-7, ledger Task 125, re-verified at this head; `ht-kqrt.219` did not touch it): `record_reconciliation_pass` in src/identity/repair.rs:256-257 calls `lane.promote_live()` whatever the store answered, and the store's `Ok(false)` means the publication was stale and the marker did not move (src/store/seats/observation.rs:1966), so the lane then skips every later tick while the live set is unchanged and `reconciled_epoch` lags until the set changes; the fix is one line, `if recorded { lane.promote_live(); }`, plus the test Task 125 names.

## Untested scope
- whole-epic review incomplete; bounded inspected hunks only; all remaining hunks and provenance gaps remain unreviewed
- Whole-range review outstanding: this invocation lists 55 of 661 changed paths and 191 of 1782 hunks; the union of all eight invocations' coverage files is 145 of 661 paths and 141 of 176 product paths; every other path and hunk is unreviewed.
- Four provenance gaps remain, at base and at head: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 and 58, invocations inv-8759cc82-264d-4506-beaa-307c9600bb8d and inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f (invalid failed assembly archive: failed bounded archive contains successful assembly or split claims).
- No full-suite measurement of this head exists and the sweep is deferred to the caller; sweep-1 at `9296918db` predates `ht-kqrt.219` and `.220`, its 54 skips are unnamed in the log, and a green unfiltered run of this head on a host with zsh and tmux is still owed, which is what remains of inv-7's parked `.64` item.
- This review ran only `scripts/check-harness-neutral` and `scripts/check-wire-compat`; clippy, `scripts/check-default-features` and tests were not run, and every finding comes from reading the head, with none reproduced.
- Partly read and not listed in coverage: src/store/seats/*.rs (new lines only), src/identity/repair.rs and src/identity/reconcile.rs (`ba6a96daa` hunks and the lane code), src/service/workers.rs (lane loop), src/service/mod_channels.rs and src/service/delivery_presentation.rs (the second is outside the changed set), src/cli/launch_gate.rs, src/service/launch_gate.rs (header), src/cli/instance.rs, TRUST-POLICY.md, the design doc (lines 1-720 and greps), scripts/hc-smoke (first 120 lines).
- Unreviewed product paths: src/cli/topology_handoff/automatic.rs (the newest commits' diffs were read by inv-7 only), src/test_support/fake_hc.rs, src/test_support/tmux.rs, scripts/wake-e2e.py, scripts/wake_e2e_selftest.py, integrations/claude/mod/tests, and almost all new tests.
- Real herdr-core and macOS: `PendingRestart` keys, H-C1, L1 and live L3, Codex `--no-daemon`, `KERN_PROCARGS2` and herdr-app labels are not exercised here; run.md records L1 and live L3 as remaining evidence.
- Live wake e2e was not run on this head, including tmux wake against a real Claude with a prompt suggestion on screen, and a read-only session with a live mod has never been exercised.
- Metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing, and BLOCKED-AUTH absence is only as good as the ledger.

## Deferred OK
- Recurring clusters (seven groups, six `missing-evidence` and one `suite-not-run`): process findings, answered by the deferred sweep and a report template fix.
- Held-for-the-operator unnamed restore (new): on a node with Herdr and hc host rows the `seat.created` parks on `wait:host` (src/store/seats/publication.rs:312, src/store/apply.rs:1004) as spec and TRUST-POLICY say, but no operator verb places it, `sync unapplied replay` re-parks it and `doctor` lists only held named seats (src/cli/doctor.rs:248); no data is lost and the journal keeps the fact, so file a bead for a doctor line and a placement verb.
- M10 on Herdr panes compares the bare pane id (src/store/seats/resolution.rs:718-726), so with two Herdr hosts the same pane id can refuse falsely (new): the outcome is M10 naming a live seat, never a wrong bind; the spec's "host plus target" wording overstates it for Herdr, so amend the wording or the hint.
- V3 operator recovery from a shell that selects a Herdr host fails with "recovery selected namespace differs from original" and does not name the core endpoint it wants (src/cli/topology_runtime.rs, `recovery_origin_host_matches`) (new): `--host-endpoint` works and is tested; a message fix.
- src/cli/lazy_display.rs:154 decides read-only from the process env where the daemon's binding flag is the authority (new): both mismatches are safe (no ACK sent, or the daemon skips it), so tidiness only.
- tmux hints survive retire (src/host/named.rs:222, keyed by name, pruned only by anchor): the daemon stays up while the old process lives, and a reminted name reads `unsafe` where `no_wake` is true; `composer_choice` prevents a wrong-pane wake, so file a bead to drop hints on binding end.
- `report_turn_edge` runs before `emit` on the outer deadline for every located hook (src/cli/hook.rs:4171), 3 to 4 extra daemon round trips per turn edge on Herdr panes where nothing reads the book: latency only.
- `note_wake_hint` spawns `tmux display` on the request path: latency only.
- `HostKind::wakes_dormant_target` is a fifth kind reader outside the spec's permitted list (ledger 1590): the tripwire test keeps it honest; amend the design's list or turn it into a port answer.
- Named `wake_gate` records `WakeBlock::Unknown` for busy and draft refusals where native clears the block (ledger Task 88): only `doctor` reads it, so the model sees nothing.
- Task 32 `[swallowed-error]`: `no_wake_reports_apply` failure falls back to the in-memory set, which repeats a report and loses none.
- Task 37 `[silent-skip]` and `.github`-side skip summaries: repo convention, read in the sweep.
- Task 125 `[suite-not-run]`, Task 129 `[missing-evidence]`, Task 130 `[missing-base-red-run]`: process findings on merged sweep fixes; the next sweep answers them.
- Task 130 `[rig-daemon-herdr-calls]`: rig daemon health and stop still run whatever `herdr` is on PATH; test-rig only, and the tests stay isolated, so a follow-up.
- Ledger 1567 `[weak-assertion]`, 1582 `[red-not-demonstrated]`, 1584 `[brief-deviation]`, 1588, 1589, 1596, 1505 `[flaky-test-risk]`, 1501, 1489, 1484, 1511, 1517: test-quality follow-ups.
- Ledger 1490 `[accepted-limit]`, 1467 `[ordering-change]`, 1479, 1494, 1578 `[doc-drift]`, 1577 `[capture-race]`: recorded limits or fail-safe.
- Stale header in src/host/hc/mod.rs:10 ("Labels land in a later bead"): doc drift.
- Full-suite wall clock of 341 s at sweep-1, over the 300 s budget: not a release gate; send it to a fixer.
- Run artifacts under `evidence/` and `docs/superpowers/runs`: bulk to prune as a follow-up, including the 9,008-line sweep log; the head names a home path in many of them, so decide before publishing.
- Cleanup: `stop-run-processes` and `remove-task-worktree` exited 4 on `.219` and `.220` because six pids could not be inspected, so task worktrees and /tmp/sp-503081fe are retained; a human or the caller removes them after landing, and /tmp is at 7% so nothing is urgent.
