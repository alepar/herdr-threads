# Final review (bounded): ht-z41e per-seat config and Codex /side queue wake

Base `236c77a660c3bb19fb31472007d8b0c670f3b907`, head `4707c14b3b041fccb804766e6177f189291a50eb` (both verified with
`git rev-parse --verify`; the caller checkout was clean at the head throughout and was not switched). Invocation
`inv-50681948-cd78-495e-807c-0e8c5a897c00`; the review source was bound in the ledger before any write.

## Scope of this bounded pass

The review-inputs sidecar reads `status: unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, 904 selected,
0 omitted, with two provenance gaps (run.md:128, inv-f5f76bdd: incomplete claimed collection and incomplete split bundle).
The failed assembly and the sidecar were preserved. The range is 904 files, +187,643/-550; 688 of them (173,865 added
lines) are run evidence under `docs/superpowers/runs/`. This pass read the full changed-path inventory and inspected the
production source, migrations, the user-facing docs and the release tooling hunks listed in `bounded-coverage.json`, from
captured Git only. Tests were inspected by name only. Read-only checks run at the head: `scripts/check-harness-neutral`
ok (0 violations), `scripts/check-wire-compat --base <base>` ok (0 violations, 0 stale). No build, clippy or test run was
made here, and no full-suite measurement of this branch exists yet.

## Requirement baseline trace

- B1 (node setting replaced by seat level): `InstanceSettings.programmatic_wake` is `Option<bool>`, parsed and inert;
  `ServiceConfig`/`QueueWakeRuntime` lost the flag; `queue_wake_selection` and `wake_intents::arm` read
  `seat_config::programmatic_wake_on` in their transactions, failing closed. `doctor` flags the obsolete key. Delivered.
- B2/S1 (generic key=value, closed registry): `src/protocol/seat_config.rs` REGISTRY with type, default, tier, backing;
  table-backed keys need no schema change. Delivered.
- B3/B4 (read/write CLI, list with descriptions): `config seat list|get|set|unset`, `human config seat set|unset`;
  `list` static with values/default/tier/description plus a trailer naming both set spellings; `--json`. Delivered.
- B5 (prose instruction works): skill and agent-usage text name the commands; the live `config-prose/group` verdict
  recorded `config seat list` then `human config seat set --seat <seat> programmatic_wake on` and store value `on`
  (row failed only on a native-mode transport check; see untested scope). Delivered on that evidence.
- B6/B7 (tiers; nudge threshold writable by non-human path): agent tier is the caller's own top-level, non-read-only
  binding through the cooperative permit (`write_threshold` shared with the aliases, audit row, `note_new_attention`);
  human tier is an operator decision of the elected owner's uid, any local seat; reads open to any caller incl.
  subagents; Claude/Codex ask rules cover `human config seat` in both spellings. Delivered.
- B8 (wakes reach the background root with /side open; spikes): process-root selection via
  `HarnessAdapter::wake_process_root` (Codex: one non-concurrent, non-auxiliary binding, all others auxiliary), sticky
  `auxiliary_session` from null `transcript_path` or a `tui_side_chat` fork rollout, root-scoped admission
  (`admit_queue_wake`), root-scoped receipt. Live `side-queue/group` PASS (private, and shared after ht-z41e.53).
  Delivered for `programmatic_wake on`.
- B9 (no native typing into a side chat): `CODEX_PLACEHOLDERS` holds only the main placeholder; split-pane reads keep a
  side placeholder as text for native; live `side-native-defer/group` PASS (ht-z41e.51 run-2). Delivered.
- B10 decisions (remove with no migration; own seat, human any; key over existing column; codex-2 spikes): matched.
- No baseline row was found undelivered.

## Cross-task integration observations

- Agent-tier set/unset, operator set/unset and the `nudge-threshold`/`wake-on` aliases share one column write
  (`seats::write_column`) with the audit upsert and wake re-judgement; table-backed changes also re-judge. Final
  retirement deletes rows in both retirement-job inserts; operator writes refuse a finally retired seat.
- The dispatcher's queue-candidate exemption (`queue_capable` + `one_queue_process`) is gated on the reserved witness's
  `programmatic_wake` and a wake control, so default-off seats keep the early busy/typed refusals. Native fallback still
  goes through `check_idle_wake`/`submit_prompt_if_owed`.
- Commit a7d1d92e0 (ht-z41e.43) widens lazy-wake reservation targets to every connected binding at both reservation
  sites (`load_candidate`, `reserve`). This is neutral core and changes native-wake target sets for every harness, not
  only Codex; it was exercised live on Codex only.
- `CallerClaim.auxiliary_session` (deny_unknown_fields struct) is stripped for an older daemon only on the hook send
  paths (`cached_event`, `tool_boundary_check_in`); the journal keeps the unstripped claim, and the CLI retry path
  (`src/cli/retry.rs`, `run_retry_inner`) re-sends the frozen claim without `drop_unadvertised_auxiliary`.
- `DeliveryPullState.session_contacts` keeps one entry per (seat, native session) for the daemon's lifetime with no
  pruning; `QueueWakeRuntime.roots` clears wholesale at 256.
- `refresh` of the native dwell (`reset_empty`) in `observe_target_with_composer` was simplified; `read_composer` still
  resets on any non-Empty read, so behavior is equivalent; the queue admission reads via `read_composer_text` and never
  touches the windows, as designed.
- All 131 range commits carry an `Outside-adapter:` trailer.

## Ledger triage

- Recurring minor `missing-evidence` x10 (9 tasks) and `cleanup-inspection` x3: evidence and process hygiene; no
  product effect. `stop-run-processes` inventory failed throughout; several task worktrees and `/tmp/sp-6cd888ee` dirs
  are recorded as kept or unknown.
- BLOCKED-AUTH ht-z41e.53: the merge's `git reset --hard a9ec9544...` was refused and its merge check did not run; the
  Git history nonetheless contains 048595590 and merge 2e2759226 (ledger: `git-only [ht-z41e.53]`). ht-z41e.54 merged
  with `check none`. The last product change (composer.rs) therefore has no recorded merge-time check on the tip.
- Task 23 `confirmed-live-fail` (graded minor, names live FAILs): re-filed as ht-z41e.51..54; side-native-defer,
  resume-idle, side-queue shared, side-nudge, side-draft and flag-flip have recorded PASS runs; config-prose has none.
- Task 11 `retry-journal-downgrade` names a known defect path; triaged as must-fix (cheap, one call on the retry path).
- Task 7 marker/mis-enrolled minors are superseded by ht-z41e.21/.33; Task 28 and Task 1 doc-drift are resolved at
  the head (TRUST-POLICY states typed text refuses a queue wake; the ansi-composer README states the side placeholder
  reads as text).
- Task 33 `existing-test-changed` (side-draft queue expectation accepted -> none) matches the shipped rule that typed
  text refuses a queue wake; the human confirmation of that rule is still parked per the design's post-implementation
  notes.

## Verdict

not ready (bounded pass only; no full suite, clippy or merge check on the tip; retry path leaks auxiliary bit)

## Must fix before landing

- Run the pre-merge gates on 4707c14 that no record shows: `cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-default-features`, and the full `cargo nextest run --locked --all-targets --all-features` with `scripts/check-no-leaked-processes --run-id`; the ht-z41e.53 merge check was lost to BLOCKED-AUTH and ht-z41e.54 merged with `check none`, so the composer.rs change on the tip has no recorded merge check (check-harness-neutral and check-wire-compat pass here).
- src/cli/retry.rs `run_retry_inner`: a journaled Codex check-in whose frozen claim carries `auxiliary_session: true` is re-sent without `drop_unadvertised_auxiliary`, so with a new CLI against a daemon lacking `seat.auxiliary_session_v1` (daemon not restarted after upgrade) a retried /side check-in is refused under `CallerClaim`'s `deny_unknown_fields` (PLAUSIBLE, not reproduced; Task 11 deferred minor, misgraded as it names a defect path); strip on the retry send path as the hook paths do.

## Untested scope

- Whole-range review outstanding: provenance gaps at run.md:128 (inv-f5f76bdd incomplete claimed collection and split bundle); not inspected here: TRUST-POLICY.md, every tests/** file (about 130), scripts/wake-e2e.py (+1168), scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/wake_e2e_selftest.py, scripts/tests/test_release_qualify.py, tests/release/release_qualify_test.py, scripts/wake-e2e-herdr, the last two hunks of scripts/release-qualify, the test hunks of src/host/native.rs and src/harness/registry.rs, src/protocol/results.rs, src/protocol/handoff.rs, src/protocol/mod.rs, src/protocol/service.rs, src/store/queries.rs, src/store/invitation_content.rs, src/store/seats/continuity.rs, src/store/schema.rs beyond its first hunk, the one-line harness decode changes (agy, opencode, pi, hermes, claude.rs, harness/mod.rs), src/cli/peer_launch.rs, src/test_support, docs/install.md, docs/validation, docs/superpowers/specs/INDEX.md, docs/evidence/ansi-composer/README.md hunks, evidence/** and all 688 docs/superpowers/runs/** paths.
- No full-suite measurement of this branch exists; the sweep is deferred to the caller.
- Required default wake e2e (Claude and Codex) not run at the tip; a7d1d92e0 widened lazy-wake reservation targets for every harness in neutral core (src/store/wake.rs), exercised live only on Codex.
- The queue-proof matrix (59/60 plus busy-invite rerun) ran at 715d3a8, before ht-z41e.53's composer.rs change; side-queue-focused/group was last run before ht-z41e.53, and side-native-defer/group only at ht-z41e.51 run-2, before the split-pane reader changed for native reads too.
- config-prose/group has no PASS row at any SHA (native-mode row failed on queue_transport_ok), and the live run bypassed the harness ask prompt (`--approve-for-me`), so the person-approval flow for `ht human config seat set` was not observed live.
- tmux Codex seats stay native under --queue-proof (named host refuses `admit_queue_wake` by default); queue routing for them is untested by design.
- Ledger/Git identity gap for ht-z41e.53 (git-only), and process cleanup left task worktrees and /tmp/sp-6cd888ee dirs kept or unknown; a human should inspect and remove them.

## Deferred OK

- src/service/delivery_pull.rs `session_contacts` grows by one entry per (seat, native session) for the daemon's lifetime with no pruning; small, bounded by sessions seen, reset on daemon exit; worth a follow-up bead.
- src/notification/queue.rs `roots` clears all entries at 256 retained roots; dropping a root only loses root-scoped receipt for that attempt.
- Human confirmation of the queue draft rule (typed text in root or side refuses a queue wake) is parked; it is a safety tightening that keeps B8 working for empty and placeholder composers.
- Task 26 extra host round trip for a lone Herdr-working root with hook Idle (capture/prepare before arm-time refusal): per-retry cost only.
- Task 37 queue_text_ok accepts any WAKE_UP target rather than the root specifically; correct because queue selection names only a cooperative_top_level root.
- Daemon-side agent-tier refusal of a human-tier `unset` names `set <key> <value>`; the CLI refuses first with the exact `unset` spelling, so only non-CLI clients see it.
- Recurring missing-evidence (x10) and cleanup-inspection (x3) minors: evidence and process hygiene, no product effect.
- Task 23 evidence bulk: run evidence adds 173,865 lines under docs/superpowers/runs (partly xz-compressed in 24692f775); repo size only.
- Accepted limit recorded in TRUST-POLICY: agy, opencode, Hermes and Pi have no ask rule on `ht human`, so their agents can write human-tier keys unasked (cooperative model).
