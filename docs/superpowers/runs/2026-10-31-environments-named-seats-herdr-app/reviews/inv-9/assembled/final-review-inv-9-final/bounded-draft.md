# Final review, epic ht-kqrt (bounded captured-source review, inv-9)

Captured source: base de5af6736e2cd4f48593d7be4035d0565cb7fa85, head ad2ee8dc99f379fb0926a14d64e2f3114a64c699. Both verified with `git rev-parse --verify`. The base equals local `main`, so the branch is not behind; `main` ends at delta 12 and this branch adds 13 and 14. The caller checkout was on the captured head and clean at the start and when this draft was written; no caller commit or status change was observed.

Bounded because the selection sidecar is `status: unavailable` with `sourceAvailable: true`, `wholeRangeReady: false`, no omissions and four retained provenance gaps. The failed assembly and its sidecar are preserved. `bind-review-source` succeeded before any artifact was written.

## Summary

One new product defect. `handoff --new-terminal` from a herdr-core terminal can never succeed: the flag parses, selects a core and is then refused by a guard that requires `--new-tab`, which clap forbids beside it. The feature's end-to-end test passes because it asserts only that one other error is absent. The task report had already seen exit 2 and filed it as a test gap.

All three inv-8 must-fix items landed and read correct at this head: the read-only filter on the V2 presentation path (`f9ef4a1c1`), the launching caller's config root on the gate argv (`39a8e797b`), and promotion of the named lane's live set only after a recorded marker (`de40100de`).

No full-suite measurement of this head exists. The sweep is deferred to the caller, so nothing here treats the branch as tested.

## Step 1: own view

Order: the epic spec was read first (design lines 1-900), then the host layer and the caller seam from captured Git. The inv-8 report and the ledger's newest round were read after that, and the rest of this pass was aimed at the paths no earlier invocation had listed. `src/cli/topology_handoff/automatic.rs` was one of those; reading it, and the deferred note beside the newest commit, led to the finding below.

- `handoff --new-terminal` (new finding, under Must fix). Parse: `--new-terminal` conflicts with `--new-tab` (src/cli/commands.rs:2097, and the test at tests/hc_handoff.rs:144-147 pins `new_tab.is_none()`), and src/cli/commands.rs:3947-3960 keeps `new_tab: None`. Selection: src/cli/mod.rs:939-946 runs `hc_target::select`, which yields `Placement::NewTerminal{parent}` from the caller's terminal. Then the automatic branch at src/cli/topology_runtime.rs:426 calls `prepare_automatic`, whose first check (src/cli/topology_handoff.rs:229-241) returns `invalid automatic bootstrap target or channel` when `request.new_tab.is_none()`, an invalid-request exit 2, and line 242 unwraps that same field as the label. Nothing rewrites `new_tab` between parse and that check. So no plan is published, no terminal is created, and the V3 `NewTerminal` handoff path in src/cli/topology_handoff/automatic.rs and src/store/topology_handoff/automatic.rs has no production caller from the CLI. `launch --new-terminal` takes src/cli/launch_new.rs and works, and `handoff --new-terminal --space @MACHINE --cwd DIR` takes the remote launch branch and is unaffected. Not reproduced here: the review did not build, and this rests on the code path above plus ledger line 1784, where the task implementer observed exit 2 on this command.
- Why every gate missed it. tests/hc_handoff.rs:192 `new_terminal_no_app_labels_parent_caller` calls `create()` directly and skips `prepare_automatic`. tests/hc_handoff.rs:1554-1560 runs the real command and asserts only that stderr lacks the capability refusal, with no exit-code check. run.md:536 lists `--new-terminal` as covered by `hc_handoff tests`, and TRUST-POLICY.md:3052 states the path exists.
- Inherited-name check beside an unattached Herdr server (new, plausible, under Deferred OK). `refuse_inherited_name` (src/service/dispatch.rs:266) asks only `all_for_reads()` for a pane candidate; `core_port` attaches a core on a miss, a pane candidate attaches nothing, and the `pane-host=` part added by `ht-kqrt.234` is filtered out of `hint_candidates` (src/service/locate.rs:166-181) and read only for M10. A named hook's claim host is the reserved locator (src/cli/hook.rs:4701-4716), so a daemon that hook autostarts gets no Herdr `--host-endpoint`. On a node with no remembered row for that socket, the first pane of a Herdr server started with the name binds it unchecked, and a later pane folds into the same seat once the first agent exits. D19 still refuses a concurrent holder and M9 still reports the second locator. TRUST-POLICY.md:705-722 records "a pane no host answers for" as an accepted limit, so this is not a policy break, but the limit covers the scenario the check exists for, on a fresh server.
- `ht-kqrt.237` placeholder (new, minor, under Deferred OK). `resolve_and_attach` (src/cli/topology_handoff/automatic.rs:455-503) sends `Attach` on every resume, and the daemon runs `record_placeholder` (src/service/dispatch.rs:734-756) before the idempotent mutation, with no check of a saved attachment. A retry of a V3 handoff past its Send enrolls a further `launch:` binding on the same process, after the agent's lifecycle check-in has passed. The binding is authority-free and ends with the process, and a failed placeholder on a finished terminal is rescued by the `status` fallback in `resolve_and_attach`.
- Host layer, read in full with no defect found. `HcHost` capture, `HcClosure` and the `create_target` witness pinning agree with herdr-core at `dc276b7` (`Registry::finish`, `platform::endpoints`, `Paths::from_env`). Named host wake gate, `tmux.rs`, the anchor walk and `TurnStateBook` agree with D21: a binding with no reported state, a stale busy and a tmux mode all refuse typing. `LocateCaller` with the wall, `caller_guard` and `caller_location` agree with the precedence rules. `guard_live_holder` reads as SA16 states.
- Launch gate. `prepare` scrubs before it applies `--config-root`, `terminal_env` hides `HERDR_*` from the shim, and `ht-kqrt.239` decides the shim before the two-family guard.
- Repo gates run here at head: `scripts/check-harness-neutral` ok (0 violations) and `scripts/check-wire-compat` ok (0 violations, 0 stale).

## Step 2: ledger triage

The ledger has 1794 lines, 395 `minor (deferred)`, the parked completions inv-7 and inv-8 closed out, nine `Recurring minor:` clusters, no `Recurring blocker:` and no BLOCKED-AUTH. The Finish metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing.

- Clusters first. All nine are process classes: eight `missing-evidence`, one `suite-not-run`. Two are new this round (ledger 1714, 1774). As grouped observations they say evidence was reported as prose or tmpfs logs, and that RED runs were skipped. Each finding keeps its identity and scope. The second new cluster matters more than the label suggests: Task 135, 143 and 147 admit their "fails without the fix" tests were never run red, and a weak assertion in that style is how the finding above survived.
- Misgraded and must-fix: Task 145 `[missing-edge-case-test]` (ledger 1784) says the `--new-terminal` handoff exits 2 and is untested. It names a known product defect, so it is must-fix, under the finding above.
- Other new minors of the fix round, all deferred: Task 132 `[agy-root-uncovered]`, `[peer-limit-documented]`; Task 131 `[bounded-empty-page-progress]`, `[seat-only-readonly-no-wake]`; Task 140 `[serial-probe-budget]`, `[no-red-run]`; Task 139 `[ambient-env-gap]`, `[unit-only-gate-test]`; Task 138 `[unwired-call-site-test]`; Task 135 `[sibling-call-site]`; Task 143 `[doc-drift]`; Task 147 `[double-parse]`; Task 145 `[fixture-adjustment]`.
- Parked completions: unchanged since inv-7's triage, with no new parked item in this invocation. Task 68 (`.64`, the zsh tests) still reduces to a green run of this head.
- The earlier deferred list from inv-8 was reread, not re-verified item by item. One moved: the M10 false refusal on two Herdr hosts is fixed by `ht-kqrt.234`. The stale header it names at src/host/hc/mod.rs:10 is still there.

## Verdict
not ready (`handoff --new-terminal` can never succeed; whole-range review, provenance gaps and the full-suite sweep remain outstanding)

## Must fix before landing
- `handoff --new-terminal` is dead (new; ledger 1784 Task 145 misgraded minor): `prepare_automatic` (src/cli/topology_handoff.rs:229-242) refuses `request.new_tab.is_none()` with `invalid automatic bootstrap target or channel` and unwraps it as the label, while clap makes `--new-terminal` exclusive with `--new-tab` (src/cli/commands.rs:2097) and nothing fills the field, so every local `handoff --new-terminal` exits 2 before any plan or terminal exists, against the design's H3 CLI list (line 866), TRUST-POLICY.md:3052 and the `run.md:536` claim; let a `NewTerminal` target pass the guard with a derived label (the harness kind, as `launch` and the remote branch use), and replace the assertion at tests/hc_handoff.rs:1554-1560 with `assert_routed` plus `assert_handoff_recorded_its_placeholder(&rig, &["--new-terminal"])`, which also closes Task 145's gap.

## Untested scope
- Whole-range review outstanding: this invocation lists 21 of 695 changed paths and 124 of 1889 hunks; every other path and hunk is unreviewed.
- Four provenance gaps remain, at base and at head: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 and 58, invocations inv-8759cc82-264d-4506-beaa-307c9600bb8d and inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f (invalid failed assembly archive: failed bounded archive contains successful assembly or split claims).
- No full-suite measurement of this head exists and the sweep is deferred to the caller; the last sweep predates the whole fix round of `ht-kqrt.222` to `.239`, and a green unfiltered run on a host with zsh and tmux is still owed, which is what remains of inv-7's parked `.64` item.
- This review ran only `scripts/check-harness-neutral` and `scripts/check-wire-compat`; clippy, `scripts/check-default-features` and tests were not run, and every finding comes from reading the head, with none reproduced.
- Partly read and not listed in coverage: src/service/dispatch.rs, src/service/launch_gate.rs, src/store/seats/cooperative.rs, src/store/seats/resolution.rs, src/store/invitation_content.rs, src/cli/topology_handoff.rs, src/cli/topology_runtime.rs, src/cli/commands.rs, src/cli/mod.rs, src/app.rs, tests/hc_handoff.rs, TRUST-POLICY.md, and the design doc past line 900. The product diff `573c76b5e..ad2ee8dc9` was read in full, but its hunks do not map onto base..head hunks, so those files stay unlisted.
- Never listed by any invocation and not read here: src/store/journal.rs, src/host/observation.rs, src/host/transport.rs, src/harness/codex/setup.rs, src/harness/agy/mod.rs, src/test_support/fake_hc.rs, src/test_support/tmux.rs, scripts/hc-smoke, scripts/wake-e2e.py, scripts/wake_e2e_selftest.py, integrations/claude/mod/tests, and almost all new tests.
- The weak-assertion class is not swept: other end-to-end tests that assert only the absence of one error string were not audited, and this finding suggests they should be.
- Real herdr-core and macOS: `PendingRestart` keys, H-C1, L1 and live L3, Codex `--no-daemon`, `KERN_PROCARGS2` and herdr-app labels are not exercised here; run.md records L1 and live L3 as remaining evidence.
- Live wake e2e was not run on this head, including tmux wake against a real Claude, and a read-only session with a live mod has never been exercised.
- Metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing, and BLOCKED-AUTH absence is only as good as the ledger.

## Deferred OK
- Inherited-name check with the pane's Herdr server unattached (new, plausible): recorded as an accepted limit at TRUST-POLICY.md:705-722 and bounded by D19 and M9, but it covers a fresh server whose first hook autostarted the daemon; attach `pane-host=` on a miss as `core_port` does for a core, or have the human confirm the limit as written, and file a bead either way.
- `record_placeholder` runs on every `Attach`, including replays past the Send (new): a further authority-free `launch:` binding on the same process until it exits; guard it on `status.attachment.is_none()` in a follow-up.
- Recurring clusters (nine groups, eight `missing-evidence` and one `suite-not-run`): process findings, answered by the deferred sweep and a report template fix; the RED-run cluster should become a required step, not a note.
- Task 132 `[agy-root-uncovered]`: agy's root is HOME, so a core autostarted under another HOME is not scrubbed; exotic, and TRUST-POLICY's "root variables" wording stays true.
- Task 131 `[seat-only-readonly-no-wake]` and `[bounded-empty-page-progress]`: documented and asserted behaviour.
- Task 135 `[sibling-call-site]`: the launch path at src/cli/mod.rs still passes `parsed.output` to `herdr_endpoint_io`; safe today, route it through `herdr_output()` when next touched.
- Task 143 `[doc-drift]`: the design still says `terminal.run` always sends `no_parent: true`; the code and `ht-kqrt.235` are right.
- Task 140 `[serial-probe-budget]`, Task 147 `[double-parse]`: latency only.
- Task 139 `[ambient-env-gap]`, `[unit-only-gate-test]`, Task 138 `[unwired-call-site-test]`, Task 145 `[fixture-adjustment]`, Task 140 `[no-red-run]`: test-quality follow-ups.
- Held-for-the-operator unnamed restore (carried from inv-8): no data lost; file a bead for a doctor line and a placement verb.
- V3 operator recovery from a shell that selects a Herdr host fails without naming the core endpoint (carried from inv-8): a message fix.
- src/cli/lazy_display.rs:154 decides read-only from process env (carried from inv-8): both mismatches are safe.
- tmux hints survive retire (carried from inv-8): file a bead to drop hints on binding end.
- `report_turn_edge` round trips on every Herdr hook and `note_wake_hint` spawning `tmux display` on the request path (carried from inv-8): latency only.
- `HostKind::wakes_dormant_target` as a fifth `kind` reader, and named `wake_gate` recording `WakeBlock::Unknown` for busy refusals (carried from inv-8): amend the list; only `doctor` reads the block.
- Migration 0014 beside the design's "one migration" rule: the design records it at line 1775, and no peer wire depends on it.
- Stale header at src/host/hc/mod.rs:10 ("Labels land in a later bead"): doc drift.
- Full-suite wall clock of 341 s at sweep-1, over the 300 s budget (carried from inv-8): not a release gate; send it to a fixer.
- Run artifacts under `evidence/` and `docs/superpowers/runs` (bulk of the diff): prune as a follow-up, and decide on the home paths they name before publishing.
- Cleanup: `stop-run-processes` and `remove-task-worktree` exited 4 on the newest tasks because six pids could not be inspected, so task worktrees and /tmp/sp-503081fe are retained; a human or the caller removes them after landing, and /tmp is at 4%.
