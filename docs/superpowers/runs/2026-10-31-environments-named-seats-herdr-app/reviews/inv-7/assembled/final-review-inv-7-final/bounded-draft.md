# Final review, epic ht-kqrt (bounded captured-source review, inv-7)

Captured source: base de5af6736e2cd4f48593d7be4035d0565cb7fa85, head 8ae0c22ce774bd56f4d15bb6834a6e705e91522f. Both verified with `git rev-parse --verify`. The base is local `main`, so the branch is not behind. The caller checkout stayed clean at the head throughout. This review read captured Git and the ledger, and it ran no build or test.

Bounded because the selection sidecar is `status: unavailable` with `sourceAvailable: true`, `wholeRangeReady: false`, no omissions and four retained provenance gaps. The failed assembly and its sidecar are preserved.

## Summary

Every code item on the inv-6 must-fix list is closed in the source: `main` is absorbed and the delta renumbered to 0013, the site is decided once after the routing flags, the tmux gate takes the styled composer read, and journaled V3 recovery routes by the plan. The inv-6 suite item stays open.

Two product defects remain. The launch-gate shim does not apply the caller's config root on any path without an explicit profile directory, and no adapter implements `child_scrub_vars`, so a config root leaked through the core's environment wins. The named lane also remembers a stale publication as finished, which the design says it must not do. The second has a narrow trigger and a one-line fix.

No full-suite measurement of this head exists. The sweep is deferred to the caller, so nothing here treats the branch as tested.

## Step 1: own view (recorded before reading the ledger)

Seams read against the spec: the seven commits since the inv-6 round (`bc0a64584`, `959b6a555`, `82a9b9c7f`, `bfed0feed`, `dbe974dcc`, `6d9400f05` and the ledger-only ones), the named host and its tmux path, turn state, caller location and the guard, `LocateCaller`, the hc host, the launch gate, the daemon detach hop and idle exit, the hook entry, dispatch, the migration, and the Claude adapter's report-only events.

- Named lane skip (`dbe974dcc`). Binding liveness runs in the separate `LivenessWorker`, and nothing gates on snapshot age, so skipping a pass strands neither presence nor delivery. Ordering of `live_named` is deterministic (`ORDER BY ordinal`), so the slice comparison is sound. `published_live` is compared per tick and a failed capture leaves it behind, so a changed set republishes. One defect: `record_reconciliation_pass` promotes on any `Ok`, including `Ok(false)` (see Must fix).
- tmux hints outlive their binding. `NamedHost.tmux` is keyed by name and pruned only by anchor liveness, so retire leaves the hint. `wake_armed` therefore counts a binding that is no longer connected, against the spec's "connected top-level named bindings". The dispatcher's `composer_choice` over `foreground_pids` stops a wake into the wrong pane, so the cost is a daemon held up by a live process and a sender reading `unsafe` where `no_wake` is the truth.
- Hook order. The turn edge runs before `emit`, on the outer `deadline` rather than the bounded one (src/cli/hook.rs:4171), for every located hook including Herdr panes, where nothing reads the book. It stays inside the watchdog margin, so a stalled daemon delays output and loses nothing: `lazy.commit()` follows `emit`.
- Seats split. A line-multiset comparison of the 6685 lines removed from src/store/seats.rs against head's seats tree left 102 not found verbatim. Those sampled are import lists, `super::` path changes and the deliberate restore, retire and hold changes. Not each was judged individually.
- Migration. 0013 is the next free slot, the remote `name` backfill reads a validated value so the CHECK cannot abort it, and `seats_live_name` covers local rows only.
- Gate shim. `terminal_env` hides leaked `HERDR_*` so the terminal is found, and `exec` keeps the pid. The scrub and the config root are the gap recorded below.
- Named resolve, M6, M9, M10, the hc closure table, `HcBoot`, the two-family guard and the detach hop read as the spec states. `resolve_named_seat` keeps its idempotent key.

## Step 2: ledger triage

The ledger has 1606 lines: 366 `minor (deferred)`, no `parked finding` lines, six parked task completions, one BLOCKED task, seven `Recurring minor:` clusters, no `Recurring blocker:` and no BLOCKED-AUTH. The Finish metrics are UNAVAILABLE (malformed re-read), so absence in the ledger proves nothing.

- Clusters first. All seven are process classes, six `missing-evidence` and one `suite-not-run`. As grouped observations they say evidence was reported as prose or logs under tmpfs rather than bound to a source tree. Each finding keeps its identity. None names a product defect, and the deferred sweep answers them as a class.
- Parked completions. `.43` (hc selector wiring) closed by `ht-kqrt.99`. `.24` (readonly on every path) closed by `07ea44639`, `f1125872e` and `96146c3bc`. `.56` (V3 records) closed by the H3 writer. `.181` is an accepted limit in TRUST-POLICY. `.64` stays open (Must fix). Task 63 (`.60`, the header label conflict) reads as settled through `sender_label`, which is not re-verified end to end.
- Misgraded and must-fix: Task 57 `[config-root-optional]` names a known product defect. Task 125 `[missing-unit-test]` names the test that would have caught the promote defect.
- Skipped tests. Task 69 `[silent-skip]` is closed: `5f8152656` makes CI fail a tmux test that cannot run. Task 71's skipped shellcheck test is closed by `70f8e22df`. Task 37 `[silent-skip]` (beside-herdr tests return early without Herdr) is a repo convention with CI's Herdr install and job summary behind it, so it is answered only by reading that summary in the sweep.
- Closed since being deferred: Task 73 `[latent-panic]` (`270098ce2`), Task 37 `[known-unfixed-defect]` (`3d6b8f893`), Task 6 `[spec-approximation]` (`any_seated`), Task 112 `[follow-up-gap]` (`bc0a64584`).
- Ledger line 244 `landed≠completed` (13 vs 8) belongs to an early pass that predates reconciliation; it is not read as clean, and it changes nothing here.
- Ledger 1595 and 1596 together: of the `ht-kqrt.215` tests, only `an_idle_named_lane_writes_nothing` is red without the fix.

## Verdict
not ready (config root leak in the launch gate, stale publication promoted by the named lane, no green full suite, whole-range review outstanding)

## Must fix before landing
- Launch gate config root (ledger Task 57, misgraded minor): `gate_argv` in src/cli/topology_handoff/automatic.rs:395 adds `--config-root` only for an explicit `directory_context`, src/service/peer_launcher.rs:673 never does, src/cli/launch_new.rs passes `directory_context: None`, and no adapter overrides `child_scrub_vars`, so `prepare` in src/cli/launch_gate.rs leaves a leaked `CLAUDE_CONFIG_DIR` in place; a core autostarted under profile A runs a `launch --new-terminal` from profile B under A's account and hooks, against spec gate step 3; pass the caller's effective root on every gate argv and scrub the adapter's root variables, or record the limit in TRUST-POLICY with a human decision.
- Named lane promotes a stale publication (ledger Task 125 names the missing test): `record_reconciliation_pass` in src/identity/repair.rs:246-247 calls `lane.promote_live()` whatever the store answered, and the store's `Ok(false)` means the publication was stale and the marker did not move, so the lane then skips every later tick while the live set is unchanged and `reconciled_epoch` lags until a restart or a seat change, against the design's "a refused pass remembers nothing"; the trigger is narrow; promote only on `recorded == true` and add the `capture_if_due`-level test the brief listed.
- Parked `.64` and ledger 1136, 1519: get one green unfiltered suite of this head on a host with zsh and tmux (CI supplies both), read the job summary for skipped Herdr and tmux tests, and confirm the three zsh failures at base.

## Untested scope
- Whole-range review outstanding: 35 of 651 changed paths are listed in coverage, with 152 of 1766 hunks; every other path and hunk is unreviewed.
- Four provenance gaps remain, at base and at head: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 and 58, invocations inv-8759cc82-264d-4506-beaa-307c9600bb8d and inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f (invalid failed assembly archive: failed bounded archive contains successful assembly or split claims).
- No full-suite measurement of this head exists and the sweep is deferred to the caller, so nothing here treats the branch as tested.
- This review ran nothing; every finding comes from reading the head, and none was reproduced.
- Partly read and not listed in coverage: src/store/seats/resolution.rs (lines 700-1022), src/store/seats/observation.rs and continuity.rs (the reconciliation marker and its lag consumers), src/identity/repair.rs, src/ports.rs, src/cli/mod.rs, src/cli/topology_runtime.rs and src/cli/topology_handoff/automatic.rs (the newest commits' diffs only), src/cli/launch_new.rs, src/service/peer_launcher.rs (`gate_argv`), src/harness/claude/composer.rs (recognizers), TRUST-POLICY.md, AGENTS.md and the design; no conclusion covers their unread parts.
- Unreviewed: src/host/hc/rpc.rs, wire.rs, labels.rs, discovery.rs, src/host/probe.rs, src/service/host_sessions.rs, src/service/launch_gate.rs, src/store/topology_handoff/, src/store/seats/cooperative.rs and operator.rs, doctor, fake_hc, scripts/hc-smoke, scripts/wake-e2e.py, CI, and almost all new tests.
- Real herdr-core and macOS: `PendingRestart` keys, H-C1, L1 and live L3, Codex `--no-daemon`, `KERN_PROCARGS2` and herdr-app labels are not exercised here; run.md records L1 and live L3 as remaining evidence.
- Live wake e2e was not run on this head, including tmux wake against a real Claude with a prompt suggestion on screen.
- Metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing, and BLOCKED-AUTH absence is only as good as the ledger.

## Deferred OK
- Recurring clusters (seven groups, six `missing-evidence` and one `suite-not-run`): process findings, answered by the deferred sweep and a report template fix.
- tmux hints survive retire (src/host/named.rs:222, keyed by name, pruned only by anchor): the daemon stays up while the old process lives, and a reminted name reads `unsafe` where `no_wake` is true; `composer_choice` prevents a wrong-pane wake, so file a bead to drop hints on binding end.
- `report_turn_edge` runs before `emit` on the outer deadline for every located hook (src/cli/hook.rs:4171), 3 to 4 extra daemon round trips per turn edge on Herdr panes where nothing reads the book: latency only, carried from inv-5 and inv-6.
- `note_wake_hint` spawns `tmux display` on the request path: latency only, carried.
- `HostKind::wakes_dormant_target` is a fifth kind reader outside the spec's permitted list (ledger 1590): the tripwire test keeps it honest; amend the design's list or turn it into a port answer.
- Named `wake_gate` records `WakeBlock::Unknown`, the variant meant for a newer daemon's reason, for busy and draft refusals where native clears the block (ledger Task 88): only `doctor` reads it, so the model sees nothing.
- `.github`-side skip summaries and Task 37 `[silent-skip]`: repo convention, read in the sweep.
- Task 32 `[swallowed-error]`: `no_wake_reports_apply` failure falls back to the in-memory set, which repeats a report and loses none.
- Ledger 1567 `[weak-assertion]`, 1582 `[red-not-demonstrated]`, 1584 `[brief-deviation]`, 1588, 1589, 1596, 1505 `[flaky-test-risk]`, 1501, 1489, 1484, 1511, 1517: test-quality follow-ups.
- Ledger 1490 `[accepted-limit]`, 1467 `[ordering-change]`, 1479, 1494, 1578 `[doc-drift]`, 1577 `[capture-race]`: recorded limits or fail-safe.
- Stale header in src/host/hc/mod.rs:10 ("Labels land in a later bead"): doc drift.
- Full-suite wall clock over the 300 s budget in earlier runs: not a release gate; send it to a fixer.
- Run artifacts under `evidence/` and `docs/superpowers/runs`: bulk to prune as a follow-up; the head adds 74 files naming a home path (base has 6), no email and no credential pattern matched, and the path names the public handle, so this is tidiness rather than a scrub breach; decide before publishing.
