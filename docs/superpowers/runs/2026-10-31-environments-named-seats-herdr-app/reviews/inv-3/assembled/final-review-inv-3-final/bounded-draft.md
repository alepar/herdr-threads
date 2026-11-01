# Final review, epic ht-kqrt (bounded captured-source review, inv-3)

Range: f4ddaeffa9fa13549f514669391e84a7fc8d5e5f..6fd338f6e45a40c1e9bdc26d1742beb938d0ee4b, 446 changed paths, 169 commits. Branch `super-auto/environments-named-seats-herdr-app`.

This is a bounded review: `review-inputs.json` reports `sourceAvailable: true`, `wholeRangeReady: false`, no omissions and four retained provenance gaps. It inspected a chosen set of production seams, not the whole range. A whole-range review remains outstanding. No full-suite measurement of this branch exists: the sweep is deferred to the caller, so nothing here treats the branch as tested.

Caller state observed at the end of the review: HEAD 6fd338f6e, clean status, branch unchanged. The ledger gained the `review-source:inv-3` binding this dispatch authorized.

## Summary

Named seats, the headline use case, do not work end to end. All three entry points fail before a seat binds, and no open task or in-flight branch fixes them. Separately, 16 of 132 child beads are open (including every phase gate), the branch sits 65 commits behind `main` with a migration slot collision, and owner-less test failures are on record. Beyond those, the hc path has a restart defect and a wrong-core allocation, and a two-family caller whose terminal verifies gets no seat.

The foundations held up where inspected: the `NonClosing` gating in reconcile, the single scrub list, the double fork and the D10 closure mapping matched the spec.

## Step 1: own view (recorded before reading the ledger)

### Reproduction

Run with `env -i`, a scratch HOME and state dir, no `herdr` on PATH, no Herdr variables, `HERDR_THREADS_SEAT_NAME=reviewer` and `CLAUDE_CODE_SESSION_ID` set. Binary: the `task-ht-kqrt.49` build at 22436a8fe, the newest binary carrying the named site. It is 25 commits behind head, not head. `src/daemon/paths.rs` lines 60-122 and `hook_runtime_context_in` are byte-identical at head by hash, and later edits to `run()` only added the Terminal mapping. Every line below is therefore confirmed by reading head as well. No daemon was left running.

- `inbox` and `thread create`: `this command acts as a seat: run it inside the agent's own Herdr pane (HERDR_PANE_ID) ...`, exit 2.
- `seat resolve`: `invalid state/host context: host endpoint missing: use --host-endpoint or HERDR_SOCKET_PATH`, exit 2.
- claude `SessionStart` hook, twice: `herdr-threads hook: runtime context: No such file or directory (os error 2)`, exit 0, no daemon started.

### Why each entry point fails

The hook: `hook_runtime_context_in` (src/cli/hook.rs:4580) passes the reserved locator `/.herdr-threads/named-host` to `RuntimeContext::explicit`, which runs `host.parent().unwrap().canonicalize()?` (src/daemon/paths.rs:79). `/.herdr-threads` exists on no machine. SA15's premise was that `HostLocator::parse` never stats, which is true, but the runtime context does. The only test of this function uses a terminal socket, whose parent exists (tests/daemon/terminal_site.rs:108).

The CLI: `run()` maps only a Terminal site onto the caller pane (src/cli/mod.rs:350), so a Named site reaches `derive_selection` with `caller_pane = None` and is refused (:3954). `instance.rs:98` sets `site_host` only for Terminal, so a named `seat resolve` has no claim host. `CallerSite::enrolls_on_first_use` has no caller.

Behind those two, `launch::resolve_seat` (src/cli/launch.rs:326) and `allocate_recipient_seat` (src/cli/mod.rs:3518) accept only `CommandResult::SeatResolved`, while a named resolve answers `NamedSeatResolved`. Once the context is fixed, a hook mints the seat and then fails with `service returned no resolved seat`, and it fails again on every lifecycle event because `Enroll::Lifecycle` always runs the resolver (src/cli/hook.rs:2080). Every test that sees `NamedSeatResolved` drives the command directly.

### hc seams

`HcHost::resume_after_epoch` is an inherent method (src/host/hc/mod.rs:317). `impl HostPort for HcHost` does not override it, so the daemon's `port.resume_after_epoch` (src/service/host_sessions.rs:672) reaches the no-op default (src/ports.rs:4284). After a daemon restart the same boot `hc-core:<id>` returns with epoch 1 and sequence 1, and `publish_snapshot_stage` refuses each snapshot (`old_epoch == epoch && old_sequence >= sequence`) until the counter passes the persisted sequence. At the spec's 5 s poll that blinds reconcile, and so D10 closure, for about as long as the previous daemon lived.

`map_hc_pane` (src/cli/seat_resolution.rs:1060) scopes the lookup to the core but drops `host` on `SeatMapping::Allocate`. `allocate_deferred_seats` (:425) then allocates on the caller's claim host. From a terminal on core `beta`, inviting `alpha:t2` where `t2` has no seat binds `beta`'s `t2`, the K4 class of cross-host mix-up. From a named caller it mints a named seat called `t2`, then errors.

`two_family_guard` and `cli_pane` treat a verified terminal as no seat (src/cli/caller_guard.rs:98, src/cli/hook.rs:4021): the hook exits silently and the CLI falls through to `caller_not_located`. The spec calls the stale `HERDR_PANE_ID` in an hc terminal the common nested case, and its goal is a seat for that agent. TRUST-POLICY.md:805 records this as an accepted limit, but the spec's H1 limits do not include it.

`candidate_root` (src/service/locate.rs:376) ignores the terminal candidate's `socket` and returns the first host that answers `tN`. Cores number terminals from `t1`, so a second core's caller gets the wrong root and a false M4. A core nothing has attached answers `Unsupported` (:396), and the `Ambiguous` path never attaches it, so the first call in a fresh core prints M4's `neither is an ancestor`, which is false.

### tmux seams

`NamedHost::target_wake_capability` answers `Composer` for a verified hint (src/host/named.rs:222), while host-level `wake_capability` is `None`, `safe_wake_target` is `None` and no typing path exists. `classify_wake_rows` (src/service/dispatch.rs:2552) reads the per-target answer, and `wake_block` defaults to `None`, so the row carries no warning. In this merged state a tmux-hinted seat loses `wake=no_wake`, its report row is deleted (`WakeEpisode::Composer`) and nothing wakes it: a p0 sender is told nothing. `.38` merged ahead of `.69`.

`CheckIn` has no `tmux_hint`. The hint rides only `ResolveSeat`, and a hook resolves only on lifecycle events or the first unowned turn (src/cli/hook.rs:2080). The spec says every hook call refreshes it, and its accepted limit promises recovery at the binding's next hook. Steady turns make no resolver call, so after a restart the seat stays unwoken until its next lifecycle event or CLI resolve.

### Store seam

`guard_live_holder` (src/store/seats/cooperative.rs:718) exempts a read-only holder row but never sees the caller's `command.readonly`, which is applied only after the guard (:827, :1017). A `HERDR_THREADS_SEAT_READONLY=1` session beside a live holder is refused with M7. Spec N acceptance requires it to be admitted, and use case 3 depends on it. `descendant_subagent_readonly_admitted` flips the holder row, not the caller.

### Integration

`main` is de5af6736, 65 commits past the merge base, which is the run's base. Run constraint 6 expected `main` to be brought in; 5e19e9bce, a617faf90 and cb70a2339 are not ancestors of head, and `scrub_env` arrived as a reimplementation (6f4b8311c). `main` has `0012_process_continuity.sql` (with an `_after` step and an `allocation_decisions` rebuild) in the slot this branch's `0012_host_environments.sql` holds, and 390 changed lines in `src/store/seats.rs`, a file this branch split into modules (d1daa8513). Its new `structural_process_continuity` rebind path has to compose with the cross-kind refusal and named continuity. That is design work, not a renumber.

## Step 2: ledger triage

There is no `Recurring blocker:` line and no `BLOCKED-AUTH` line. The Metrics lines are UNAVAILABLE (the Finish ledger re-read was malformed), so absence in the ledger is not read as clean.

Four `Recurring minor:` clusters, all `[process-class: missing-evidence]`, each ×3 across 3 tasks: (ht-kqrt.2, .12, .3), (.10, .53, .75), (.6, .14, .9), (.23, .21, .26). Each keeps its identity and scope. Judged as a group they are process findings, not code defects. Read with `[stale-check-evidence]`, `[stale-evidence]` and `[suite-not-run]`, they mean no per-task pass is bound to a source tree, so no task-level result substitutes for the deferred sweep.

Parked records: `.43` (hc selector wiring) was filed as `.99` and is closed; head shows it wired, so it is settled. `.24` and `.56` stay open below and need a human decision.

Minors regraded to must-fix, because each names a known product defect, a skipped test or a failure on record:

- ledger 615 `[later-task-gap]`, 620 `[deferred-named-watch-binding]`: the named CLI defect, owned by no bead.
- ledger 722 `[deferred-dependency]`: the read-only caller; `.24` did not pick it up.
- ledger 534 `[unused-code]`: `resume_after_epoch`; `.44` wired attach and the override never came.
- ledger 852 `[accepted-limit]`: a verified terminal binds nothing.
- ledger 875 `[hook-path-unchecked]`: confirmed, hooks do not carry the hint on steady turns.
- ledger 787, 850, 874 `[pre-existing-failures]`, 812 `[unmeasured-base-failure]`: failures recorded with owner null.
- ledger 213 `[unverified-live-path]`: a shellcheck test skipped.
- ledger 808 `[ambiguous-acceptance]`: a bound no-wake named seat stays a poke candidate; `.32`'s suppression covers offline seats only and relies on there being no wake lane, which `.69` adds.
- ledger 839, 860 and 737, 774, 851: undocumented limits and contradictory normative text in TRUST-POLICY.md.

## Verdict

not ready (named seats fail at every entry point, 16 beads open including all phase gates, branch behind main with a migration collision, no full-suite measurement)

## Must fix before landing

- Every hook on a named site dies: `RuntimeContext::explicit` canonicalizes the parent of `/.herdr-threads/named-host` (src/daemon/paths.rs:79), which never exists. Reproduced. No bead owns it.
- Named CLI cannot act: `run()` gives a Named site no pane (src/cli/mod.rs:350) and no claim host (src/cli/instance.rs:98), so inbox, thread create and seat resolve are refused. Reproduced. Ledger 615, and 620 records `watch` reporting no_binding; no bead owns it.
- `launch::resolve_seat` and `allocate_recipient_seat` reject `NamedSeatResolved`: after the context fix, each lifecycle hook mints or finds the seat, then fails with no check-in (src/cli/launch.rs:326, src/cli/mod.rs:3518).
- A verified terminal gets no seat in the spec's common nested case (src/cli/caller_guard.rs:98, src/cli/hook.rs:4021); TRUST-POLICY.md:805 records a limit the spec never accepted. Bind it, or get a human decision.
- `candidate_root` ignores the socket and a fresh core is never attached (src/service/locate.rs:376, :396): two cores sharing `tN`, or a first call in a new core, print a false M4.
- `HcHost` never overrides `HostPort::resume_after_epoch`: after a daemon restart hc publications are refused until the counter passes the persisted sequence, blinding D10 closure. One-line fix plus a restart test.
- `map_hc_pane` drops the core on Allocate (src/cli/seat_resolution.rs:1060): `alpha:t2` allocates on the caller's own core, or mints a named seat `t2`. Needs host on the deferred entry and a Stage-level test.
- `guard_live_holder` refuses a read-only caller beside a live holder with M7 (src/store/seats/cooperative.rs:718), failing spec N acceptance and use case 3. Pass `readonly` in and test the caller side.
- Per-target `Composer` merged without its lane: a tmux-hinted seat loses `wake=no_wake`, its report row is deleted and nothing wakes it. Land `.69` first or gate the answer on the lane; `.69` must also suppress pokes for bound no-wake seats (ledger 808).
- The tmux hint is not refreshed by hooks: `CheckIn` has no field and steady turns skip the resolver, so the spec's every-hook refresh and the next-hook recovery limit are both false. Carry it, or amend spec and TRUST-POLICY.
- Parked `.24`, needs a human decision: a session enrolled only through plain commands stays writable, so its display discharges ACKs under `HERDR_THREADS_SEAT_READONLY=1`. Accept the limit or carry the flag at enroll.
- Parked `.56`, needs a human decision: V3 records have no writer or consumer (src/protocol/handoff.rs:384) and journal validation takes 1|2 only, so hc attempts have no journal record. Assign the writer to `.57` or amend the spec.
- Owner-less failures on record: golden_surfaces ×6 (doctor golden lacks keys doctor.rs emits), host_neutral_scan (probe.rs, main.rs, target_creator.rs unallowlisted), every_table_is_classified (no_wake_reports). Own and fix them; run the skipped shellcheck test.
- Merge `main` (65 commits): renumber `0012`, reconcile `src/store/seats.rs` with the module split, and verify `structural_process_continuity` against the cross-kind refusal. Then run the full suite and leak checks on the merged tree.
- 16 beads open: .30 .33 .34 .50 .57 .59 .60 .61 .62 .63 .64 .66 .68 .69 .71 .119, including the N2, H1 and final gates. Also fix TRUST-POLICY's undocumented limits and its `never CoherentVerified` contradiction (ledger 737, 839, 860).

## Untested scope

- Whole-range review outstanding: 27 of 446 changed paths were inspected, and only 65 hunks are claimed; every other path and hunk is unreviewed.
- Four provenance gaps remain, at base and head: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 and 58, invocations inv-8759cc82-264d-4506-beaa-307c9600bb8d and inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f (invalid failed assembly archive).
- No full-suite measurement of this branch exists, and no test or clippy run was made by this review; the sweep is deferred to the caller.
- The reproduction used the `.49` binary at 22436a8fe, 25 commits behind head; head itself was not built or run. Findings without a reproduction come from reading source and are unexecuted.
- Partly read and not listed in coverage: src/store/seats/cooperative.rs, observation.rs, resolution.rs, src/ports.rs, TRUST-POLICY.md and the unlisted hunks of the partly read files. No conclusion covers their unread parts.
- Unreviewed: src/cli/launch_gate.rs, hc rpc, wire, labels and discovery, fake_hc, sync and peer launch, Hermes bridge and Claude mod changes, setup and install flow, doctor, self-retire, no-wake reports and almost all new tests.
- Metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing, and BLOCKED-AUTH absence is only as good as the ledger.
- Real herdr-core behaviour: `HcClosure::PendingRestart` keys are unchecked against a real core (ledger 533), H-C1, L1-L3 and Codex `--no-daemon` live runs are remaining evidence, and production `HcBootstrapObserver` refuses every call pending `.57`.
- Wake e2e and macOS paths (E1 `KERN_PROCARGS2`, herdr-app labels) were not run. `dialled_hosts_active` approximates the spec's seat-holding rule (ledger 444); the seatless core case waits on `.50`.

## Deferred OK

- The four `missing-evidence` clusters (ht-kqrt.2, .12, .3; .10, .53, .75; .6, .14, .9; .23, .21, .26) are process findings: fix the report template, but only the deferred sweep can replace the evidence they lack.
- Doc comment and comment placement minors (ledger 220, 431, 443, 588, 614) and duplicate arms or small visibility drift (ledger 188, 207, 491, 750) are cosmetic.
- `[swallowed-error]` no_wake_reports_apply (ledger 742) falls back to the in-memory set, so the worst case is a repeated warning.
- `[sync-on-request-path]` (ledger 872): a 2 s tmux check on a named resolve is bounded and runs outside the writer turn.
- `[extra-round-trip]` (ledger 795): turn edges add connect, locate and call on every turn for pane seats too, and nothing reads them yet. Gate them when `.69` lands; not a blocker alone.
- `[silent-failure-relaunch]` and `[narrow-race-window]` (ledger 478, 480): startup diagnostics only, bounded by the startup deadline.
- `[config-root-optional]` (ledger 830) and `[test-only-stopgap]` (ledger 550, cfg-gated to test-support): carry to `.57`, which has to close both.
- Run artifacts committed under `evidence/` and the large `docs/superpowers/runs` dumps (ledger 810): prune as a follow-up.
- Cleanup records with `process cleanup incomplete (exit 4)` and kept `/tmp/sp-503081fe` paths: a run-finish chore, not a code defect.
