# Final review, epic ht-kqrt (bounded captured-source review, inv-4)

Captured source: base f4ddaeffa9fa13549f514669391e84a7fc8d5e5f, head 6dfce0d93d21d88bd96ae83689511e9bc875db5f. Both verified. The caller checkout was clean at the head throughout; nothing in it changed. This review is reading only: no build, no test, no reproduction was run, so every finding below comes from source at the head and is unexecuted.

## Summary

The inv-3 blockers that stopped named seats from working end to end are gone: hooks and the named CLI now reach the daemon, `NamedSeatResolved` is accepted, the tmux wake lane landed, V3 records landed, and no child bead of the epic is open. What remains is the herdr-core half and the honesty of tmux wake. Five inv-3 must-fix items are still present at the head with no bead owning them, including the spec's common nested hc case. Two new defects came in with `.69`. The branch is also 65 commits behind `main`, a dry-run merge conflicts in 23 files, and the migration slot collides.

## Step 1: own view (recorded before reading the ledger)

Seams checked against the spec: caller location, `LocateCaller`, the two-family guard, the named and hc hosts, tmux hint and wake gate, turn state, no-wake reports, built-in session and lanes, idle exit, double fork and scrub, launch gate, target creator, peer launch placement.

- Two-family caller in an hc terminal gets no seat. `caller_guard::cli_pane` maps `Guard::Terminal` to no pane, `run()` takes its site from `non_pane_locator`, which is `None` for `Ambiguous`, and `two_family_guard` returns the default outcome. 04 B records that a core autostarted from a Herdr pane gives every terminal `HERDR_PANE_ID` and `HERDR_ENV=1`, so this is the normal hc path, not a corner.
- `candidate_root` drops the candidate's socket (src/service/locate.rs:376) and `locate_caller` reads only attached hosts, so a first call in a new core is `Unsupported`, and two cores with the same `tN` answer from the first.
- `inherited_name_host` drops the session of `<session>:tN` (src/service/locate.rs:196-203): any core with that `tN` whose server holds the name refuses with M6.
- Hint refresh. Hooks send `tmux_hint` only through `ResolveSeat`, which turn hooks skip for a resolved seat (src/cli/hook.rs:2080). After a daemon restart a live tmux seat regains turn state on its next hook but no hint.
- Codex in tmux. `wake_gate` requires `screen_blocked == Some(false)` (src/host/named.rs:299) and Codex keeps the `None` default (src/harness/codex/composer.rs:174), so it is never typed into, while `target_wake_capability` answers `Composer` from the hint alone (src/host/named.rs:446). No `wake=no_wake`, and `scripts/wake-e2e.py:189` lists Codex in `TMUX_HARNESSES`.
- `NamedHost` has no `wake_block` or `note_wake_block`, so every gate refusal reaches senders as no `wake=` reason at all (src/ports.rs:4255 default, src/service/dispatch.rs:2659).
- Otherwise inspected and matching the spec: precedence, wall handling, double fork, the single scrub list, built-in slot and `Option` lane plumbing, `kind()` gate on `any_reachable`, turn-state edges including continuation, no-wake episode rows, gate shim env handling, witness checks in `authorize`.

## Step 2: ledger triage

Metrics lines are UNAVAILABLE (malformed Finish re-read), and ledger line 244 records an older `landed≠completed 13 vs 8`, so absence in the ledger is not read as clean. No `Recurring blocker:` and no `BLOCKED-AUTH` line.

Six `Recurring minor:` clusters, each ×3 across 3 tasks, keep their identity: `missing-evidence` (.2, .12, .3), (.10, .53, .75), (.6, .14, .9), (.23, .21, .26), (.136, .141, .69), and `suite-not-run` (.30, .139, .71). As a group they are process findings: reports carry no structured evidence bound to a source tree. Judged separately they are not code defects, and they mean no per-task pass substitutes for the deferred sweep.

Parked records: `.43` settled by `.99`. `.56` superseded by `.133`, `.134` and `.57`, by commit subjects, not independently traced. `.24` still needs a human decision. `.57` (launch flags) is a spec deviation left for review and stays open below. `.30` (delivery `from` golden) closed by `.164` on commit subject. `.64` (zsh trio) is environmental; CI installs zsh, so it needs a CI run.

Comparing with this review's Step 1: the first three Step 1 items and the hint item are the inv-3 must-fix items, unfixed, and repeating them independently raises confidence. The Codex and `wake_block` items are new since inv-3.

Inv-3 items checked at head and cleared: named hook context, named CLI, `NamedSeatResolved`, per-target `Composer` without a lane, 16 open beads. Inv-3 items still present: verified terminal, `candidate_root`, `HcHost` epoch, `map_hc_pane`, read-only caller, hint refresh, `.24`, main merge. No bead matches any of these by title or description.

## Verdict
not ready (hc nested callers get no seat, hc restart and allocation defects open, Codex tmux wake unreachable and unreported, 65 commits behind main; no full-suite measurement of this head; whole-range review outstanding)

## Must fix before landing
- A verified terminal binds nothing: `Guard::Terminal` yields no pane and no site (src/cli/caller_guard.rs:98, src/cli/mod.rs:348, src/cli/hook.rs `two_family_guard`), so an agent in any core autostarted from a Herdr pane has no seat despite the spec Goal and 04 B; acting as the Terminal site is the fix, or a human accepts the limit, because the TRUST-POLICY text alone is not that decision.
- `LocateCaller` never attaches the core and ignores the socket (src/service/locate.rs:376, :396, src/service/dispatch.rs `locate_caller`): a first call in a new core prints a false M4 forever since nothing else attaches it, and two cores sharing `tN` print a false M4 for the second core's callers.
- `HcHost::resume_after_epoch` is an inherent method (src/host/hc/mod.rs:320), so `port.resume_after_epoch` at src/service/host_sessions.rs:740 hits the no-op trait default: after a daemon restart `src/store/seats/observation.rs:2838` refuses hc snapshots until the sequence passes the persisted one, blinding D10 closure for about as long as the previous run published; tests/identity/reconcile.rs:4309 calls the concrete type and masks it.
- `map_hc_pane` drops the core on Allocate (src/cli/seat_resolution.rs:1198-1213): `alpha:t2` in an allocate slot resolves on the caller's own host, or under a named caller mints a named seat `t2`; carry the host on `Deferred`.
- `guard_live_holder` has no caller-side readonly (src/store/seats/cooperative.rs:718, called at :827): a `HERDR_THREADS_SEAT_READONLY=1` caller beside a live holder is refused M7, against spec line 1225 and TRUST-POLICY N; tests/store/live_holder.rs covers only a readonly holder row.
- tmux Codex seats are never typed and never reported: `screen_blocked` is `None` so `wake_gate` always refuses, yet `target_wake_capability` says `Composer`, which suppresses `wake=no_wake` and deletes its report row; make the per-target answer `None` for a harness with no blocked recognizer, or ship a Codex recognizer from a captured fixture, and fix `TMUX_HARNESSES`, because `--harness codex --cases tmux-named/send-p0` cannot pass today.
- `NamedHost` records no wake block: N2 acceptance "the sender gets the existing `wake=` overdue rendering" fails for draft, busy, blocked, unknown and tmux-mode refusals, since `wake_block` is the default `None`; `wake_gate` refusals need `note_wake_block` and a test, because no tmux test asserts `wake=`.
- The tmux hint is not refreshed by hooks: `CheckIn` has no field and steady turns skip the resolver (src/cli/hook.rs:2080), so spec line 557 ("every hook call") and TRUST-POLICY's "until that binding's next hook" are both false after a restart; carry the hint or amend both.
- `launch` has no `--space`, `--new-tab` or `--new-terminal` (`LaunchArgs`, src/cli/commands.rs:2036) although spec lines 849 and 1446 and the epic Goal require them; parked on `.57` as a spec deviation, so implement it or get a human decision.
- Parked `.24`, needs a human decision: a session enrolled only by plain commands stays writable, so its display discharges ACKs under `HERDR_THREADS_SEAT_READONLY=1`.
- Merge `main`: 65 commits behind, dry-run conflicts in 23 files including src/store/schema.rs, src/store/seats.rs, src/identity/reconcile.rs and TRUST-POLICY.md, and `0012_host_environments.sql` collides with main's `0012_process_continuity` pair; renumber on merge as the spec requires, and verify `structural_process_continuity` against the cross-kind refusal.
- Ledger 1137 `[known-unfixed-defect]`: `handoff --existing --seat PEER` from a named seat fails dialing the reserved locator, no bead; reported by `.34`, not reproduced here.
- Ledger 1050 `[latent-panic]`: `HostBootId::new` at src/host/hc/mod.rs:678 panics on a long core id, since the composed string can exceed 128 bytes while `HcBoot` admits 120 of core id; use `parse` and return `OutcomeUnknown`.
- Ledger 1114 and 1179 `[transient-failure-no-retry]`: one failed `terminal.get` after the Send is staged refuses the gate as `WitnessMismatch`, which kills a launch that was fine; retry until the deadline.
- Ledger 839 `[undocumented-accepted-limit]`: the session of a `<session>:tN` hint is ignored, so a core that is not the caller's can refuse with M6; pass the core or document the limit.
- Ledger 1202 and 1136 `[silent-skip]`: `named_tmux_idle_recipient_wakes_for_p0_not_p1` and the beside-Herdr tests pass with nothing exercised when tmux or Herdr is absent, and `.github/workflows/ci.yml` has no step that installs tmux, so whether CI ever runs the only daemon-level tmux wake test is unconfirmed.
- Ledger 213 `[unverified-live-path]`: the shellcheck test of scripts/lib/isolated-core.sh is skipped when shellcheck is absent and its result is unrecorded; run it once where shellcheck exists.
- Ledger 1204 `[environmental-test-failures]`, with parked `.64`: the unfiltered suite is red on this host for the zsh trio; a green run on a host with zsh is required, which CI supplies.

## Untested scope
- whole-epic review incomplete; bounded inspected hunks only; all remaining hunks and provenance gaps remain unreviewed
- Whole-range review outstanding: 21 of 588 changed paths are listed in coverage, with 126 hunks; every other path and hunk is unreviewed.
- Four provenance gaps remain, at base and at head: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 and 58, invocations inv-8759cc82-264d-4506-beaa-307c9600bb8d and inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f (invalid failed assembly archive).
- No full-suite measurement of this head exists and the sweep is deferred to the caller; the run's gate (248 s, 3 zsh failures, leak check clean) was on `.64`'s tree before `.66` merged, and is not treated as testing this head.
- This review ran nothing; findings are from reading the head, and no finding was reproduced.
- Partly read and not listed in coverage: src/host/hc/mod.rs, src/cli/seat_resolution.rs, src/cli/mod.rs, src/store/seats/cooperative.rs, src/store/seats/observation.rs, src/harness/claude/composer.rs, src/harness/codex/composer.rs, TRUST-POLICY.md, the design, and the unlisted hunks of src/service/host_sessions.rs and src/service/peer_launcher.rs; no conclusion covers their unread parts.
- Unreviewed: src/store/seats restore, retire, held-name and publication paths; hc rpc, wire, labels and discovery; fake_hc; sync and name@node; Hermes bridge and Claude mod; setup, install and doctor; self-retire; handoff and topology_handoff rewrites; almost all new tests; the `seats.rs` split as a pure move.
- Metrics are UNAVAILABLE (ledger re-read malformed), so ledger absence proves nothing, and BLOCKED-AUTH absence is only as good as the ledger.
- Real herdr-core and macOS: `PendingRestart` keys, H-C1, L1-L3, Codex `--no-daemon`, `KERN_PROCARGS2` and herdr-app labels are not exercised; ledger 1174 notes hc-smoke was not rerun at the H1 gate.
- Live wake e2e was not run, including the opt-in tmux-named cases (ledger 1107), whose status regex and glyph checks are unexercised; ledger 1115 and 1122 leave `authorize_launch_gate` and the shim `exec` without a spawned-daemon test; ledger 1184 leaves mod tests unrun (no bun on this host).

## Deferred OK
- The six recurring clusters (`missing-evidence` .2/.12/.3, .10/.53/.75, .6/.14/.9, .23/.21/.26, .136/.141/.69; `suite-not-run` .30/.139/.71) are process findings: fix the report template, and let the deferred sweep replace the evidence they lack.
- Ledger 808 `[ambiguous-acceptance]`: a bound no-wake named seat stays a poke candidate, but `safe_wake_target` answers `None` without a live binding, so nothing is typed; the cost is a spent `soft_poked_at`, traced no further.
- Ledger 226 `[wire-form-deferred]`: `SeatWakeStatus.host` is `serde(skip)`; it is consumed in the daemon only, so no client needs it yet.
- Ledger 742 `[swallowed-error]`: `no_wake_reports_apply` falls back to the in-memory set, so the worst case is a repeated warning.
- Ledger 1100 `[doc-drift]`: seatless-host idle exit is spec-mandated and undocumented outside it; add a line when next touched.
- Ledger 1142 `[over-broad-suppression]`, 478 and 480: lane log and startup diagnostics only, bounded by the startup deadline.
- Ledger 1178 `[orphan-binding-on-write-failure]`: the shim times out and the terminal finish retires the seat, so it heals itself.
- Ledger 1193 `[load-flake]`: tracked by bead ht-62tr, and flakes do not block.
- Ledger 1135 and 1172 `[suite-over-budget]`: load on a shared host; the last gate was 248 s, and budget is not a landing gate.
- `ht-tyil` (unused `CallerSite` answers) is filed; the unused answers are harmless and it can follow.
- Doc comment, comment placement, weak-assertion and scope-note minors (for example ledger 431, 1088, 1090, 1092, 1099, 1101, 1165, 1205) are cosmetic or test-quality follow-ups.
- Committed run artifacts under `evidence/` and `docs/superpowers/runs` (ledger 1030) are bulk to prune as a follow-up, and retained cleanup records (exit 4) are a run-finish chore.
