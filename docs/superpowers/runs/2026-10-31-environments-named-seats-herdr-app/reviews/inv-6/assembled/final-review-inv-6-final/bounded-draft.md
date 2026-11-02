# Final review, epic ht-kqrt (bounded captured-source review, inv-6)

Captured source: base f4ddaeffa9fa13549f514669391e84a7fc8d5e5f, head 17eb653d1867573d6b8eaac0dd9fcf3385499ce2. Both verified. The caller checkout stayed clean at the head throughout. This review only read captured Git, the design and the ledger. It ran no build, no test and no reproduction, and no full-suite measurement of this head exists: the sweep is deferred to the caller, so this branch is not treated as tested.

Bounded because the selection sidecar is `status: unavailable` with `sourceAvailable: true`, `wholeRangeReady: false`, no omissions and four retained provenance gaps.

## Summary

Four of the six inv-5 must-fix items are closed in the source: `handoff` onto a core on a core-only daemon, the `startup_primary` filter, the tmux focused hold and `launch --new-tab` from a named seat. The other two stay open: the merge with `main` is not done, and the green unfiltered run is still missing (three zsh tests fail on this host).

Two new product defects. Fix-round beads .199 and .203 compose wrong: a verified two-family terminal now reads its core socket as a Herdr server. And the tmux wake reads the composer as plain text only, so a default Claude session that shows a prompt suggestion is refused every wake. One minor from the ledger is misgraded: operator recovery of a V3 record is refused on a core-only daemon.

## Step 1: own view (recorded before reading the ledger)

Seams checked against the spec: the fix-round diff from 81b092894 to the head across the guarded capabilities, the verified-terminal decision, the shared Herdr endpoint, the focused hold, retirement replay, the hc freeze and the time bounds; then surfaces inv-5 left unread: `seat_resolution.rs`, the Claude mod, the Hermes bridge, sync and `name@node`, the delivery header, the no-wake reports, and the blocked-screen and running-turn recognizers.

### 1. A verified two-family terminal dials its core socket as a Herdr server (Important)

- Composition of .199 and .203. `.199` added one shared point, `topology_runtime::herdr_endpoint_io` (src/cli/topology_runtime.rs:75), which builds `InstanceInputs::from_process(None, host)` and calls `herdr_endpoint(selected)`. `.203` removed the `VERIFIED_TERMINAL` thread-local that made `from_process` read a verified terminal as a lone site.
- Failure path: a caller with both `HERDR_PANE_ID` and `HC_TERMINAL_*` (for example `hc run` started inside a Herdr pane), verified as the terminal. `run` swaps its own `CallerEnv` value, not the process environment. `context` comes from `for_site`, so `selected` is the core socket (src/cli/mod.rs:719). `from_process` re-reads the unswapped environment, reaches `Located::Ambiguous`, and leaves `site_host` `None`. `InstanceInputs::herdr_endpoint` (src/cli/instance.rs:157) then returns `selected`, the core socket.
- Effect: every Herdr path from that caller takes the core as its Herdr server: the pane-selector `host` closure (src/cli/mod.rs:831), `recipient_host` (:3496), `run_launch` (:4500), `launch_new::on_herdr` (src/cli/launch_new.rs:171), `handoff::run` (src/cli/handoff.rs:2444) and the handoff `namespace`. `handoff --existing`, `handoff --pane`, `launch PANE` and pane-form `invite` fail on a Herdr server that is reachable. The caller's environment even names it. A lone Terminal site in the same position works.
- TRUST-POLICY now says every path from a non-Pane site "takes the endpoint from the one shared point" and that a verified terminal acts as that site at every hop. This path contradicts both, and it fails closed: a wrong dial, not a wrong bind. `pane_agent` (src/cli/mod.rs:457) has the same `from_process` read and is harmless only because a failed read is no evidence.
- No test crosses it: `two_family_terminal_acts_as_its_terminal_seat` covers messaging, and `tests/cli_env_read_scan.rs` counts reads in `hook.rs` only. Fix: pass `parsed.caller_site` into `herdr_endpoint_io` through `for_site`.

### 2. The tmux wake never takes a styled composer read (Important)

- `NamedHost::wake_gate` reads the composer with `read_composer_in` (src/host/named.rs:384) over `tmux::capture`, which is `capture-pane -p` with no `-e` (src/host/named/tmux.rs:183). `read_pane_composer` (:449) does the same. Nothing on the tmux path calls `wants_styled` or `read_styled`; the Herdr path does (src/host/native.rs:2386-2393).
- Claude's plain `read` cannot tell a suggestion from a draft and answers `Unsafe { CLAUDE_NOT_KNOWN_EMPTY }` by design, so the gate refuses with "composer is nonempty". An uncaptured placeholder answers `Unreadable`.
- Effect: TRUST-POLICY says Claude draws a prompt suggestion after a turn, and `setup` turns them off only on an interactive yes. A default hooks-only Claude named seat in tmux therefore gets no wake once a suggestion shows, and the sender reads a refused-wake warning. N2 can type into only two harnesses, and this is one: Codex and Hermes answer `None`, a mod-backed Claude wakes itself, and agy is the other. N2's acceptance is "an idle named seat with an empty composer wakes", and A4 defines dim-only text as empty.
- TRUST-POLICY's limit for suggestions covers a host that serves no styled read. tmux serves one. Neither the design nor the tests mention suggestions on this path. It fails safe, so nothing is typed into a draft. Fix: capture with `-e` and take the styled read when the policy asks, or record the limit with a human decision.

### 3. Inherited: `main` is unmerged (Critical)

- `main` is still 65 commits ahead of the merge base, which is the base itself. A dry-run `git merge-tree` reports 24 conflicted files, including src/store/schema.rs, src/store/seats.rs (now split on this branch), src/identity/reconcile.rs, src/daemon/lifecycle.rs and TRUST-POLICY.md.
- `migrations/node/0012_host_environments.sql` and `main`'s `0012_process_continuity.sql` pair share a number under different file names, so Git reports no conflict for it. The branch is at version 2.8.1 and `main` at 2.8.4.

### 4. Smaller seams (not blocking)

- `watch.rs:1285` and `:1393` still pass `CallerEnv::from_process()` although `parsed.caller_site` is at hand. For a verified terminal that affects only how selectors are printed. It should go with the fix to finding 1.
- A Terminal site's `next:` lines pin `--host-endpoint <core>`. Copied onto a pane-form command, `herdr_endpoint` reads the flag as the Herdr server. Same class as the reserved-locator item inv-5 deferred.
- `claim_recovery_env` has no Terminal case: a recovery hint for a core claim prints `HERDR_PANE_ID=tN`, and the Terminal arm of `env_assignments` is unreachable. Inside the terminal it goes through the guard and lands on the terminal, so the retry still works. Run elsewhere it is a pane caller with a bare `tN`.
- `handoff --existing` on a machine with no Herdr is refused twice over: no Herdr endpoint for the namespace and topology, and the node-wide capability. The spec only promises handoff "when a Herdr host is reachable", and the design note records it, but the skill's first routing rule is handoff. `send` works, and the refusal is clean.

Checked with nothing to report: `GuardedAnswers` and the per-call capability answers, `startup_primary` filtering, the V3 route through `target_runtime`, `request_bound` and the gate budgets, `connect_until`, `focused_empty_ready` shared by both hosts, the replayed `seat.retired` transition, M10 host matching, hc Err on an unreadable capture, `hc_selector`, `remote_named_seat`, `PEER_LAUNCH_TERMINAL_CAP` gating, `sender_label`, the poke filter, `no_wake_reports`, the hook's one-decision flow (`caller_env`, `report_turn_edge_on`), and the Hermes `PANE_ENV` gateway path, where a gateway's `TMUX` cannot reach a wake because Hermes has no blocked-screen recognizer.

The `seats.rs` split is not a pure move: a normalized line comparison shows 65 removed and about 600 added lines. Those are the feature's named-seat, self-retire and restore changes, spread across the split. Only `restore_own_seat`, the retirement replay and parts of resolution were read.

## Step 2: ledger triage

The ledger has 349 `minor (deferred)` lines, 7 `Recurring minor:` clusters, no `Recurring blocker:` line and no `BLOCKED-AUTH` line. This invocation added no parked completion. Metrics are UNAVAILABLE (malformed Finish re-read), so absence in the ledger proves nothing.

Recurring clusters: the same seven `missing-evidence` or `suite-not-run` process groups as inv-5. They need no code; the deferred sweep supplies what they lack.

Parked: unchanged from inv-5. `.43`, `.24` and `.56` are superseded, `.60` landed under its amendment, `.181` (ledger 1336) is an accepted limit that fails toward not settling, and `.64` (ledger 1196) names failing tests, so it stays must-fix until a green run exists. Ledger 1519 adds that those three failures were never shown failing at base.

Minors from this round reassessed:

- Ledger 1476 `[follow-up-gap]`, first half: `topology_recover::automatic::require_capability` (src/cli/topology_recover/automatic.rs:80, called at :199 and :235) gates on the node-wide flag, so an operator cannot recover a stuck V3 bootstrap on a core-only daemon. This is a known product defect, so it is misgraded and goes to must-fix. Whether recovery then works on a core is unverified: `.200`'s test covers the forward path only.
- Ledger 1476, second half: `handoff_delivery::require_automatic_capability`, delivery into an existing seat. Deferred, as finding 4 sets out.
- Ledger 1488 `[missing-end-to-end-test]`: the swapped `HookEnv` reaching `report_turn_edge` has unit coverage only. Finding 1 shows what unwired swaps cost; add the end-to-end test beside its fix.
- Ledger 1483 `[unaudited-claim-host-sites]`: judged by inspection; the sites at src/cli/mod.rs:1652, 4259 and 4373 were not re-read here.
- Ledger 1518 `[suite-not-run]`: the `.205` proof ran before the rebase. The caller's sweep answers it.

## Verdict
not ready (main is unmerged; a verified two-family terminal dials its core socket as a Herdr server; tmux wake refuses a Claude prompt suggestion)

## Must fix before landing
- Merge `main`: 65 commits behind, 24 conflicted files in a dry run, and `0012_host_environments.sql` silently shares its number with `main`'s `0012_process_continuity` pair; renumber as the spec requires and check `structural_process_continuity` against the cross-kind refusal.
- A verified two-family terminal uses its core socket as the Herdr server: `herdr_endpoint_io` builds `from_process` and ignores the decided site (src/cli/topology_runtime.rs:75, src/cli/instance.rs:157), so pane forms, `launch` and `handoff --pane` or `--existing` fail on a reachable Herdr; pass `parsed.caller_site` through `for_site`, fix `pane_agent` beside it, and add a two-family test through a pane-form command.
- tmux wake reads the composer as plain text only (src/host/named.rs:384, :449; src/host/named/tmux.rs:183), so a default hooks-only Claude seat showing a prompt suggestion is refused every wake; capture with `-e` and take the styled read, or record the limit with a human decision.
- Operator recovery of a V3 bootstrap gates on the node-wide flag (src/cli/topology_recover/automatic.rs:80, ledger 1476, misgraded minor), so a stuck attempt on a core-only daemon cannot be recovered; route it like `require_bootstrap_route_capability` and test recovery on a core.
- Parked `.64` and ledger 1136, 1519: get a green unfiltered suite on a host with zsh and tmux (CI supplies it), read the job summary for skipped Herdr and tmux tests, and confirm the three zsh failures at base.

## Untested scope
- whole-epic review incomplete; bounded inspected hunks only; all remaining hunks and provenance gaps remain unreviewed
- Whole-range review outstanding: 29 of 655 changed paths are listed in coverage, with 103 of 1830 hunks; every other path and hunk is unreviewed.
- Four provenance gaps remain, at base and at head: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 and 58, invocations inv-8759cc82-264d-4506-beaa-307c9600bb8d and inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f (invalid failed assembly archive).
- No full-suite measurement of this head exists and the sweep is deferred to the caller, so nothing here treats the branch as tested. The last recorded run, `.205`'s proof before its rebase, ran 8799 tests with 3 failures and 53 skipped.
- This review ran nothing; every finding comes from reading the head, and none was reproduced.
- Read through the 81b092894..head diff but not listed in coverage, because coverage records base..head hunks: src/app.rs, src/daemon/control.rs, src/daemon/transport.rs, src/main.rs, src/service/dispatch.rs, src/service/host_sessions.rs, src/service/launch_gate.rs, src/cli/hook.rs, src/cli/mod.rs, src/cli/handoff.rs, src/cli/topology_runtime.rs, src/cli/topology_handoff.rs, src/cli/watch.rs, src/cli/doctor.rs, src/cli/setup.rs, src/cli/me.rs, src/host/native.rs, src/host/probe.rs, src/host/hc/mod.rs, src/store/control/retirement.rs, src/store/seats/publication.rs, src/store/seats/resolution.rs; no conclusion covers their earlier hunks.
- Partly read and not listed in coverage: src/host/named.rs, src/host/named/tmux.rs, src/host/hc/rpc.rs, src/host/hc/labels.rs, src/host/hc/wire.rs, src/service/locate.rs, src/cli/launch_new.rs, src/cli/handoff_delivery.rs, src/cli/topology_recover/automatic.rs, TRUST-POLICY.md and the design; no conclusion covers their unread parts.
- Unreviewed: src/store/seats/observation.rs, cooperative.rs, operator.rs and continuity.rs beyond the line comparison, src/identity/reconcile.rs, the migration, doctor and setup before the fix round, install, hc discovery, fake_hc, wake-e2e changes, CI, and almost all new tests.
- Metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing, and BLOCKED-AUTH absence is only as good as the ledger.
- Real herdr-core and macOS: `PendingRestart` keys, H-C1, L1-L3, Codex `--no-daemon`, `KERN_PROCARGS2` and herdr-app labels are not exercised here.
- Live wake e2e was not run, including tmux wake against a real Claude, which is what finding 2 needs.

## Deferred OK
- Recurring clusters (seven `missing-evidence` or `suite-not-run` groups): process findings, answered by the deferred sweep and a report template fix.
- `handoff --existing` with no Herdr host, ledger 1476 second half: outside the spec's promise and refused cleanly; `send` works; file a bead and state it in the skill.
- `watch.rs` `from_process` reads for a verified terminal: printed selectors only; fix with the two-family item.
- A Terminal `next:` line pins the core as `--host-endpoint`: a failure only if copied onto a pane form.
- `claim_recovery_env` has no Terminal case: the retry works from the terminal; the Terminal arm of `env_assignments` is dead until then.
- `report_turn_edge` still runs for every located hook, and `note_wake_hint` still spawns `tmux display` on the request path: carried from inv-5, latency only.
- Full-suite wall clock: 354 s in the `.205` proof against the 300 s budget, with `failpoints_are_compiled_out_of_release_builds` at 312 s alone; not a release gate, send it to a fixer.
- Ledger 1336 (`.181` parked): accepted limit in TRUST-POLICY, and it fails toward not settling.
- Ledger 1490 `[accepted-limit]`, 1467 `[ordering-change]`, 1479 `[stale-startup-lane-status]`, 1494 `[doc-drift]`: recorded limits or harmless.
- Ledger 1489 `[regression-guard-only]`, 1484 `[test-replaced]`, 1501 `[late-duplicate-unchecked]`, 1505 `[flaky-test-risk]`, 1510, 1511, 1517: test-quality follow-ups.
- Roast r2 punch list: the tmux test helper's `TMUX_TMPDIR`, Health degraded on hc nodes, and label documentation.
- Kept task worktrees and /tmp/sp-503081fe from `process cleanup incomplete (exit 4)` for every task of this round: the caller removes them once the unknown pids are understood.
- Committed run artifacts under `evidence/` and `docs/superpowers/runs` are bulk to prune as a follow-up.
