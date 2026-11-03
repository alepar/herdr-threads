# Final review, epic ht-kqrt (bounded captured-source review, inv-5)

Captured source: base f4ddaeffa9fa13549f514669391e84a7fc8d5e5f, head 81b092894a25565f2ffdfd6abaa6f35a43c76f2c. Both verified. The caller checkout stayed clean at the head throughout. This review only read captured Git, plus the read-only herdr-core checkout at dc276b7. It ran no build, no test and no reproduction, and no full-suite measurement of this head exists: the sweep is deferred to the caller, so this branch is not treated as tested.

Bounded because the selection sidecar is `status: unavailable` with `sourceAvailable: true`, `wholeRangeReady: false`, no omissions and four retained provenance gaps.

## Summary

The inv-4 must-fix list is largely closed: a tmux Codex seat now answers `no_wake`, `NamedHost` records wake blocks, turn edges carry the hint, `launch` has its new flags, a named `handoff --existing` dials the Herdr endpoint, and `HostBootId` no longer panics. The merge with `main` is still not done.

The new finding is a daemon composition defect. The capability that `handoff` onto a herdr-core needs is advertised only when the daemon's startup primary is a Herdr guarded-start host. So H3 `handoff` cannot run on a machine without Herdr, and a daemon first started by a named seat or a herdr-core terminal takes guarded handoff away from Herdr panes until it restarts.

## Step 1: own view (recorded before reading the ledger)

Seams checked against the spec: caller location and the two-family guard, `LocateCaller`, the hc host and client, the named host and tmux wake, turn state, `wake_status`, `HostSessions` and the built-in slot, idle exit, the detach hop, instance discovery, launch and the launch gate, and the named resolve, restore, retire and live-holder paths.

### 1. `handoff` onto a herdr-core needs a capability a core-only daemon never advertises (Critical)

- `try_run_automatic_new_with_ports` gates both placements of a V3 plan on `handoff.automatic_v1`: `automatic_before_topology` for `NewTab` and `require_automatic_capability` for `NewTerminal` (src/cli/topology_runtime.rs:346-358). The `run_target` retry arm does the same (:527).
- `AUTOMATIC_HANDOFF_V1` is not in `ADVERTISED` (tests/contracts.rs:2715). Only `ControlService::new_guarded` turns it on (src/daemon/control.rs:173-189, :346).
- `new_guarded` runs only when `guarded_host` is `Some`, and that is `primary.filter(|session| session.bootstrap().is_some())` (src/app.rs:2629-2632).
- An hc session never has `bootstrap`: the slot requires `HostGuardedStart`, and its runtime goes to `target_bootstrap` (`HostSessions::start_session`, `HostSession::new`).
- With only a core attached, `startup_primary` is `None`, which `startup_primary_two_host` pins (tests/daemon/multi_host.rs). So `ControlService::new` is used, and `handoff --new-terminal` from an hc terminal fails with `daemon lacks automatic handoff capability`.
- The epic goal says handoff can create targets in herdr-core terminals, and use case 1 is a machine without Herdr. No test crosses this seam: `tests/hc_handoff.rs` drives the shim and `CreateTargetPort` directly, and the CLI tests use fake clients that list the capability. `bootstrap_runtime_gate_two_host` asserts only the two-host answer.
- `launch --new-terminal` is not affected: `LOCAL_LAUNCH_GATE_V1` is in `ADVERTISED`.

### 2. `startup_primary` returns the hinted session unfiltered, so a non-Herdr first caller disables guarded handoff for Herdr panes (Critical)

- Failure path: a named caller's context has `host_endpoint = /.herdr-threads/named-host` (`resolve_host_endpoint`, src/cli/instance.rs), and a Terminal site's has the core socket. `spawn_detached` passes that as `--host-endpoint` (src/daemon/lifecycle.rs), and `main` turns it into the startup hint.
- `attach_startup` then answers from the built-in slot or the core, and `startup_primary(hint)` returns that session, because only the dialled fallback is filtered to `HostGuardedStart` (src/service/host_sessions.rs:1081-1092). The doc comment says "never the built-in host"; the hint branch contradicts it.
- The remembered branch has the same gap: `attach_remembered_live` keeps the most recently attached row as `first` with no kind filter (:1106-1124), so an hc row can win it too.
- Effect on a mixed machine: if a named process or hc terminal is the first caller, `guarded_host` is `None` for the daemon's life. Herdr panes lose `handoff.automatic_v1` and `handoff.bootstrap_guarded_resolution_v1` (`daemon lacks automatic handoff capability`, `daemon lacks guarded handoff capability`) until the daemon restarts. This is a regression against the base, where a start on a Herdr machine carried a Herdr hint.
- Second effect: `ElectedHostEvidence::of_sessions` reads `incarnation_witness` and `safe_prompt` from the named port, the same skew `health_entries` was changed to avoid.
- `startup_primary_two_host` passes `None` over lazily attached sessions, so it reaches neither branch. The ledger line 445 `[helper-only-test]` is the same gap.

### 3. tmux wake has no focused-composer hold (Important)

- Design line 573 reads: "Focus = `pane_active && window_active && session_attached`, under the existing focused-composer hold." `git grep` finds none of those formats in src. `PANE_FORMAT` does not read them, `NamedHost` observations carry `focused: false`, and `wake_gate` has no focus step.
- TRUST-POLICY A4 Wake says "Focused panes require a minute of observed empty composer samples". The N2-wake paragraph neither applies that nor lists it as a limit, so an operator attached to a tmux pane with an empty composer gets a wake typed and submitted immediately, and a keystroke racing it can be submitted with it.
- AGENTS.md requires a weakened invariant to update TRUST-POLICY.md in the same commit. No task covered this line. Implement the hold, or record the accepted limit with a human decision.

### 4. `launch --new-tab` from a named seat dials the reserved locator (Important)

- `hc_target::select_wanted` answers `Ok(None)` for a Named site, so `launch_new::on_herdr` runs, and it takes `context.require_host()`, which is `/.herdr-threads/named-host` (src/cli/launch_new.rs:166-171). The topology read then fails on a socket that does not exist.
- `named_handoff` (src/cli/mod.rs) and `topology_runtime` go through `handoff::herdr_endpoint`, which is the lazy rule SA1 sets for non-Pane sites. `LaunchNew` was added after that rule and does not use it. It is the same defect class as `.187`, on the new entry point.
- A named seat on a Herdr-less machine with one core also never reaches discovery here, because `Some(_)` returns before the discovered branch.

### 5. Smaller seams (not blocking)

- `run_admitted_hook_since` reports the turn edge with the outer `HookEnv`, after `admitted_hook_outcome` has dropped the verified-terminal scope. A two-family hook, verified or refused with M4, then looks up the foreign pane's seat and sends `TurnState` for it. The effect is one in-memory entry nobody reads, since only `NamedHost` reads turn state, and it binds nothing. Skip when `env.ambiguous` is set.
- `report_turn_edge` runs for every located hook, so each Herdr pane pays connect, capabilities, a seat page and `TurnState` twice per turn for a book only named seats read. That cuts against "the no-news hook path is one cheap round trip". Gate it on `env.named`.
- `note_wake_hint` runs `tmux display` synchronously on the request path, up to 2 s per seat-acting named call or turn edge when a tmux server hangs.
- `locate_caller`'s epoch hashes host ids only, so the doc's "changes when a host restarts" does not hold. Subjects are short-lived, so the effect is nil.
- For a named seat the hook's `CommandRouting` still carries `host_endpoint: /.herdr-threads/named-host` (44 bytes), which `pinned_host` removes from CLI `next:` lines. If a model copies it as `--host-endpoint`, `lazy_host_endpoint` reads that flag as the Herdr server and a named handoff fails.

Checked with nothing to report: `HcClosure` against `Registry::finish` as the design states it, socket derivation against herdr-core paths.rs and platform/unix.rs, `parent` beside `no_parent` over RPC (run.rs lets `parent` win; only the CLI refuses the pair), named continuity, the held-name hand-back, the live-holder guard, the wake gate order, turn-state edges, the detach hop, gate release order and the migration backfill.

## Step 2: ledger triage

The ledger has 328 `minor (deferred)` lines, 20 lines naming `parked`, 7 `Recurring minor:` clusters and no `BLOCKED-AUTH` line. Metrics are UNAVAILABLE (malformed Finish re-read), so absence in the ledger proves nothing.

Recurring clusters: all seven are `missing-evidence` or `suite-not-run` process findings. The first six were triaged at inv-4. The seventh (ht-kqrt.175, .174, .177, ledger 1305) is the same class. They need no code; the deferred sweep supplies the evidence they lack.

Parked:

- `.43`, `.24`, `.56`: superseded by `.99`, `.181` and the V3 work; no longer open.
- `.60` (BLOCKED, hc label in `from` headers): landed at ledger 1163 under a bead amendment that keeps the seat id for hc senders; `sender_label` names only named seats. Ledger 1165 notes the stale test comment.
- `.181` (ledger 1336): a fresh plain-only named seat's first readonly call is refused until a retry. It is recorded in TRUST-POLICY and it fails toward not settling. Acceptable.
- `.64` (ledger 1196): the three zsh tests. These name failing tests, so they are triaged as must-fix until a green run exists. CI installs zsh, so the first CI run answers it.

Minors reassessed:

- Ledger 1136 `[silent-skip]`, beside-Herdr tests: `.189` closed the tmux half, and CI lists the remaining skips in its job summary. It stays in the must-fix item for a green CI run, because a skip there is unmeasured coverage.
- Ledger 445 `[helper-only-test]`: misgraded, because it hid findings 1 and 2. It belongs with them.
- Ledger 1031 `[missing-e2e-test]`: no live tmux wake run exists; recorded under Untested scope.
- Ledger 1414 `[bare-lookup-remaining]`, 1416 `[guard-ignores-state-dir]`, 1401 `[old-hint-degrade]`, 1281 `[wire-skew-refusal]`: each fails closed or degrades only an unreleased skew. Deferred.
- Ledger 1406 `[orphan-seat-on-conflict]`: an accepted limit in TRUST-POLICY.

Inherited from inv-4: `main` is still 65 commits ahead of the merge base, the base itself, so run constraint 6 was never met. A dry-run `git merge-tree` reports 23 conflicted files, including src/store/schema.rs, src/store/seats.rs, src/identity/reconcile.rs and TRUST-POLICY.md. `0012_host_environments.sql` collides with `main`'s `0012_process_continuity` pair.

## Verdict
not ready (handoff.automatic_v1 is tied to a Herdr startup primary, so core-only handoff fails and a non-Herdr first caller disables guarded handoff; main is unmerged)

## Must fix before landing
- `handoff` onto a herdr-core fails on a core-only daemon: the V3 path requires `handoff.automatic_v1`, advertised only through `new_guarded` from a Herdr primary (src/app.rs:2629, src/daemon/control.rs:173, src/cli/topology_runtime.rs:346); gate V3 on what the target core's session can do and add a spawned-daemon test with only a core attached.
- `startup_primary` returns a hinted or remembered built-in or hc session unfiltered (src/service/host_sessions.rs:1081), so a named or hc first caller removes guarded capabilities from Herdr panes until restart and gives Health the named port's answers; keep the primary to `HostGuardedStart` hosts, or decide guarded capabilities per call so a Herdr host attached later is served, and test with a reserved and a core hint.
- tmux wake has no focused-composer hold (design line 573, TRUST-POLICY A4 Wake): read `pane_active`, `window_active` and `session_attached` and apply the one-minute hold, or record the accepted limit with a human decision.
- `launch --new-tab` from a named seat dials `/.herdr-threads/named-host` (src/cli/launch_new.rs:166): use `handoff::herdr_endpoint`, as `named_handoff` does.
- Merge `main`: 65 commits behind, 23 conflicted files in a dry run, and `0012_host_environments.sql` collides with `main`'s `0012_process_continuity` pair; renumber as the spec requires and check `structural_process_continuity` against the cross-kind refusal.
- Parked `.64` and ledger 1136: get a green unfiltered suite on a host with zsh and tmux (CI supplies it), and read the job summary for skipped Herdr and tmux tests.

## Untested scope
- whole-epic review incomplete; bounded inspected hunks only; all remaining hunks and provenance gaps remain unreviewed
- Whole-range review outstanding: 27 of 624 changed paths are listed in coverage, with 220 of about 1710 hunks; every other path and hunk is unreviewed.
- Four provenance gaps remain, at base and at head: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 and 58, invocations inv-8759cc82-264d-4506-beaa-307c9600bb8d and inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f (invalid failed assembly archive).
- No full-suite measurement of this head exists and the sweep is deferred to the caller, so nothing here treats the branch as tested.
- This review ran nothing; findings are from reading the head, and none was reproduced, including finding 1, which is traced through the code and one pinned test.
- Partly read and not listed in coverage: src/store/seats/resolution.rs, cooperative.rs, publication.rs, operator.rs, src/daemon/control.rs, src/cli/topology_runtime.rs, src/cli/topology_handoff.rs, src/cli/handoff.rs, the last three hunks of src/service/peer_launcher.rs, TRUST-POLICY.md and the design; no conclusion covers their unread parts.
- Unreviewed: src/cli/seat_resolution.rs, src/cli/watch.rs, src/cli/doctor.rs, src/cli/setup.rs and setup and install, the `seats.rs` split as a pure move, observation.rs, hc labels and wire, fake_hc, sync and name@node, the Claude mod and Hermes bridge, wake-e2e changes, and almost all new tests.
- Metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing, and BLOCKED-AUTH absence is only as good as the ledger.
- Real herdr-core and macOS: `PendingRestart` keys, H-C1, L1-L3, Codex `--no-daemon`, `KERN_PROCARGS2` and herdr-app labels are not exercised here.
- Live wake e2e was not run, including the opt-in tmux-named cases; ledger 1031 leaves the dispatcher-to-`NamedHost` path without an end-to-end test.

## Deferred OK
- Recurring clusters (seven `missing-evidence` or `suite-not-run` groups): process findings, answered by the deferred sweep and a report template fix.
- `report_turn_edge` runs for two-family hooks after the verified scope ends: one in-memory entry nobody reads; skip it when `env.ambiguous` is set.
- `report_turn_edge` adds round trips to every Herdr pane's turn edges: latency only; gate it on `env.named` in a follow-up.
- `note_wake_hint` spawns `tmux display` on the request path, bounded at 2 s.
- `locate_caller` epoch ignores host restarts: short-lived subjects make the cache nearly unreachable; fix the doc.
- Hook `CommandRouting` keeps the reserved locator for a named seat: 44 bytes on check-ins, and a failure only if copied into a handoff.
- A Named site with `--new-tab` never reaches `HcDiscovery`: refused, not misrouted; revisit with the `launch_new` fix.
- Ledger 1336 (`.181` parked): accepted limit in TRUST-POLICY, and it fails toward not settling.
- Ledger 1414 `[bare-lookup-remaining]`: `me init` lookup from a Terminal site; file a bead.
- Ledger 1416 `[guard-ignores-state-dir]`: the guard fails closed with M4; file a bead.
- Ledger 1401 `[old-hint-degrade]` and 1281 `[wire-skew-refusal]`: only an unreleased skew is affected.
- Ledger 1406 `[orphan-seat-on-conflict]`: accepted limit in TRUST-POLICY.
- Doc placement, weak-assertion, missing-red-run and out-of-list-file minors, for example ledger 1269, 1283, 1293, 1309, 1320, 1335, 1363 and 1371: cosmetic or test-quality follow-ups.
- Committed run artifacts under `evidence/` and `docs/superpowers/runs` are bulk to prune as a follow-up.
