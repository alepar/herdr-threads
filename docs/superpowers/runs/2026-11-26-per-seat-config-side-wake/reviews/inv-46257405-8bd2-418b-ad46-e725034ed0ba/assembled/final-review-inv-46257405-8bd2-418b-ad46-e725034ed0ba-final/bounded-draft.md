# Final review (bounded) — epic ht-z41e, super-auto/per-seat-config-side-wake

Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head 6cda40813ebfe5655a52f293888a9791c88d676b (both verified with `git rev-parse --verify <sha>^{commit}`). Caller checkout was at head 6cda40813 with a clean status when read; it was never switched or restored.

## Scope of this pass

Bounded route: review-inputs.json reports sourceAvailable true, wholeRangeReady false, status unavailable, no omissions, and two retained provenance gaps (inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 at run.md:110: incomplete claimed collection; incomplete split bundle, missing cluster report or slices). No whole-range package was built or loaded. The full changed-path inventory (571 paths, 363 of them under docs/superpowers) was read from captured Git. Inspected from captured Git, every hunk: all of `src/**`, `migrations/node/0020..0022`, `integrations/codex/README.md`, `integrations/skill/SKILL.md`, `AGENTS.md`, `RELEASE.md`, `scripts/release-qualify`, `tests/hook_entrypoint.rs` and the run's design doc. Read beyond the diff to trace the baseline: `src/notification/dispatch.rs`, `src/host/native.rs`, `src/harness/composer.rs`, `src/ports.rs` (`for_seat_attention`), `src/cli/commands.rs` (actor checks), `src/cli/mod.rs` (`exact_check_in_replay`) at head, plus the added lines of `TRUST-POLICY.md` (context lines not read, so it is not counted as inspected). Everything else in the inventory (the rest of `tests/**`, `scripts/wake-e2e.py` and its selftests, `scripts/wake_e2e_queue*.py`, `evidence/**`, `CHANGELOG.md`, `README.md`, `docs/**` other than the design doc) is unreviewed in this pass.

## Requirement baseline trace

Each row was traced statically from a production entry point at head. No row was exercised live in this pass.

- B1 (node setting replaced by a seat setting): `InstanceSettings.programmatic_wake` is now `Option<bool>` and is read only by doctor, which prints an "obsolete" line. `QueueWakeRuntime` lost its flag. `queue_wake_selection` reads `seat_config::programmatic_wake_on` inside its transaction, and `wake_intents::arm` reads it again (a flip before arm goes stale-authority NoEffect, then native). Delivered.
- B2/S1 (generic key=value, closed typed registry): `protocol::seat_config::REGISTRY` holds two keys, and unknown keys and values are refused with the valid set. A `Table` key needs no schema change. Delivered.
- B3/B4 (CLI to read and write; list with descriptions): `ht config seat list` is static, renders key, values, default, tier and description plus a trailer, and has a `--json` form; `get` and `set|unset` exist. Delivered.
- B5 (telling an agent in prose works): SKILL.md gains one "Per-seat settings" sentence that names list, get, set and the human spelling. When an agent tries a human-tier key, the refusal names the exact `ht human config seat set KEY VALUE`. Delivered in text. There is no live prose case run (`config_prose_case` in wake-e2e.py is unexercised, ledger Task 22).
- B6/B7 (all keys readable by non-human; low-risk keys such as nudge_threshold writable by non-human; gated keys only via human): `GetSeatConfig` needs no claim. Agent-tier writes go through `PermitMutation::SetSeatConfig`, then `seat_config::set_agent`, then `write_threshold`/accountable transaction on the caller's own seat. The daemon refuses an agent-tier write of a human-tier key (`agent_write`), and so does the CLI (`refuse_agent_config_write`). The human tier is an operator decision checked against the elected owner uid, and the harness ask rule gates it (Claude, Codex). The accepted limit for the other harnesses is recorded in TRUST-POLICY. Delivered.
- B8 (wakes still reach the top-level session in the background while /side is open): chain at head, with root and side both owing:
  - the observation reads the side placeholder as `Text` (HumanInput);
  - `queue_candidate` lets it pass (all reasons are seat attention);
  - gated several-target composer choice gives `Process`;
  - `use_queue` takes the root-text proxy (ht-z41e.37) and `one_queue_process`;
  - `queue_wake_selection` reaches `process_root` and `CodexAdapter::wake_process_root`, which picks the binding that is neither concurrent nor auxiliary;
  - `guard(GuardFor::Queue)` calls `check_programmatic_wake`, which admits the exact side placeholder (ht-z41e.40);
  - arm completes and the receipt is root-session scoped.

  The auxiliary bit reaches the store from real Codex hook payloads on SessionStart fork, UserPromptSubmit, PreToolUse and Stop (`tests/hook_entrypoint.rs` codex_auxiliary_entrypoint). Delivered in code, but proven only by unit and fake-peer tests. The design's live `side-queue/group` qualification (ht-z41e.10, open) has not run. One condition is open: the guard refuses a pane whose Herdr status is `working`; see the Concerns section.
- B9 (no native wake typed into a side chat): `CODEX_PLACEHOLDERS` no longer lists the side placeholder. The native final gate `idle_wake_gate(Admission::Composer)` needs `Empty`, so the wake refuses and retries on the backoff. Queue admission keeps separate focused-dwell windows. Delivered. Live `side-native-defer/group` not run.
- B10 (brainstorm decisions): node key removed with no migration (doctor points at the seat command). The write target is the caller's own seat, and the human tier may name any seat (`SeatSlot::ConfigSeat`, Global tiering). The threshold is a key over the existing column (`write_column`, audit row, `note_new_attention`). Delivered. The live spikes were run under the codex-2 profile (spike-side-identity.md). This pass did not check them against the `--codex-auth-home` rule.

## Cross-task integration observations

- The seams compose: ht-z41e.4 (per-seat flag), ht-z41e.21/.33 (auxiliary evidence on every hook path, gated by `seat.auxiliary_session_v1` in both `cached_event` and `tool_boundary_check_in`), ht-z41e.35 (root-scoped receipt through `take_queue_root`), ht-z41e.37 (multi-target `use_queue`) and ht-z41e.40 (programmatic composer admission). Together they close the two earlier final-review blockers recorded in run.md, inv-5e65b241 and inv-d7c1c3bb (single-target gate) and inv-e401fcce (guard refusing the side composer).
- `observe_target_with_composer` no longer resets the empty windows on `composer_ui && ui != Idle`. I checked `composer::observed_ui`: `ActiveTurn` arises only from status `working`, which still resets, and non-admitted reads reset their own admission's window inside `read_composer`. So the change is behaviour-preserving for native admission.
- The dispatcher's HumanInput pass-through (`queue_candidate`) does not look at the seat's `programmatic_wake`. With the key off, a draft-holding Codex pane now reaches `queue.attempt`, which returns Native, and is then refused by the native final gate ("composer is nonempty") instead of the earlier "ui human input or active turn". Nothing is typed. The cost is one extra selection transaction per refused attempt and a different refusal detail.
- The queue guard keeps the focused-pane rule: a person looking at the `/side` chat gets the root's queue wake only after 60 s of continuous admitted reads. This is within the design and TRUST-POLICY text, but it is worth knowing for the live case.
- `QueueWakeRuntime.roots` clears everything at 256 retained entries. Entries leak only when an attempt is Submitted but its transport is not taken (recovery paths). The insert-take window is one dispatcher call, so dropping a live root is unlikely.
- Wire: new commands and results have no `deny_unknown_fields`. `CallerClaim.auxiliary_session` is `serde(default, skip_serializing_if = Not::not)` and is stripped for daemons that do not advertise it. Neither `scripts/check-wire-compat` nor `scripts/check-harness-neutral` was run in this pass.

## Ledger triage

The ledger was read after the observations above.

- Recurring minor `missing-evidence` (6 occurrences across ht-z41e.1, .2, .4, .7, .33, .37): logs live only under `/tmp/sp-6cd888ee`. This is process hygiene. It does not block landing, but it means no retained per-task test evidence exists, which adds weight to the outstanding full-suite and live runs.
- Task 1 (ht-z41e.1), `unexecuted-test`: the `herdr_release_wake` side-close path and its inverted `/side` expectation are compiled but `#[ignore]`d and never run. A minor that names an ignored test was misgraded, so it is regraded to land-blocking. It belongs to the ht-z41e.10 qualification (herdr-release-smoke).
- Task 7 (ht-z41e.7), `marker-deviation` and `mis-enrolled-side`: superseded by ht-z41e.21. The marker is now `auxiliary_session`, and a lone concurrent or auxiliary binding is never a target (`process_root` `[only]` arm). Resolved.
- Task 8 (ht-z41e.8), `no-live-run` (`side_view_changed` may need loosening): open until ht-z41e.10.
- Task 11 (ht-z41e.21), `retry-journal-downgrade`: a journaled check-in carrying `auxiliary_session` is replayed through `decode_request`. On the `cached_event` path it is stripped. The remaining exposure is a daemon downgrade between record and retry. Acceptable as is.
- Task 24 (ht-z41e.37), `root-text-proxy`: `queue_text_ok` is `any` target. It is correct because `select_binding` requires a `cooperative_top_level` root. Acceptable, but a later refactor of selection provenance could silently break it.
- Task 25 (ht-z41e.40) and others, `missing-red-run` and stop-run-processes `--check` failing with "git worktree inventory failed": process gaps. The leak checks were manual.
- No `parked` completion lines, no `BLOCKED-AUTH` lines and no `Recurring blocker` lines are in the ledger. run.md's own `parked:` block holds a design escalation (null `transcript_path` sets only `auxiliary_session`), and the shipped code implements that conservative shape. Its "human to confirm" request is still open, and so is the inv-f5f76bdd cluster-3 collection gap that this pass also inherits.

## Concerns

- Busy side turn vs queue guard: `check_programmatic_wake` reuses the native status gate, and the native.rs test `check_programmatic_wake_refuses_a_side_draft_and_unreadable` asserts that a pane with agent status `working` refuses the queue wake. AGENTS.md (side cases paragraph), TRUST-POLICY and the design say instead that a busy side binding does not refuse the root (spike S2) and that under `--queue-proof` a hooked recipient is queue-woken "even while a side turn is busy". If Herdr reports the pane `working` while a side turn runs, the root wake waits for the side turn to end, and the `side-nudge`/`side-draft` queue-mode expectations may fail live. Verify in ht-z41e.10, then fix either the docs or the expectation. (PLAUSIBLE, not reproduced.)
- `side-draft` under `--queue-proof`: the queue guard admits only the exact side placeholder text, so a draft typed into the side composer refuses the queue wake as well. The AGENTS.md text "side-draft then needs only the intact draft and the delivered item" holds only if the draft sits somewhere other than the displayed side composer. Not traced in `scripts/wake-e2e.py` (unreviewed here).

## Verdict

not ready (bounded review only; live qualification ht-z41e.10 incl. the ignored herdr_release_wake path never ran, no full-suite measurement exists, whole-range coverage gap inv-f5f76bdd remains)

## Must fix before landing

- Run ht-z41e.10 at the final SHA and record it: wake-e2e Codex `--queue-proof` matrix with per-seat enablement plus `side-queue/group`, `side-native-defer/group`, `side-nudge/group`, `side-draft/group`; it is the design's §Qualification for B8/B9 and the only proof the root is queue-woken beside a displayed /side chat (all current B8 evidence is unit/fake-peer).
- Run the `#[ignore]`d `herdr_release_wake` side-close path (tests/integration/herdr_release_smoke.rs, via scripts/herdr-release-smoke); ledger Task 1 graded it minor, but an ignored, never-executed test of changed behaviour is land-blocking.
- Resolve the busy-side contradiction found during ht-z41e.10: `check_programmatic_wake` refuses pane status `working` (native.rs test asserts it) while AGENTS.md/TRUST-POLICY/design promise a busy side turn does not refuse the root; align docs or behaviour with the live result.

## Untested scope

- No full-suite (`cargo nextest run --locked --all-targets --all-features`) measurement of this branch exists; the caller runs it after this invocation, so the branch is not yet tested as a whole.
- Whole-range review remains outstanding: provenance gaps inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 (run.md:110 incomplete claimed collection; incomplete split bundle, missing cluster report or slices).
- Unreviewed paths in this bounded pass: every changed path outside src/**, migrations/node/0020-0022, integrations/codex/README.md, integrations/skill/SKILL.md, AGENTS.md, RELEASE.md, scripts/release-qualify, tests/hook_entrypoint.rs and the design doc — notably tests/** (other than hook_entrypoint.rs), scripts/wake-e2e.py, scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, tests/release/release_qualify_test.py, scripts/tests/test_release_qualify.py, evidence/**, CHANGELOG.md, README.md, docs/install.md, docs/agent-usage.md, docs/evidence/ansi-composer/README.md, docs/validation/herdr-release-matrix.md and the run's docs/superpowers files.
- TRUST-POLICY.md was read only for its added lines; its hunks' context was not reviewed.
- scripts/check-wire-compat, scripts/check-harness-neutral, clippy and scripts/check-default-features were not run in this pass.
- Live B5 prose case (`config_prose_case`) and flag-flip case are covered only by pure-helper selftests (ledger Task 22).
- Run-wide process-leak sweep is incomplete: stop-run-processes --check failed with "git worktree inventory failed" across tasks; leak checks were manual ps.
- run.md parked design escalation (null transcript_path sets only auxiliary_session) still asks for a human confirmation that is not recorded.

## Deferred OK

- Recurring missing-evidence (logs only under /tmp/sp-6cd888ee) across ht-z41e.1/.2/.4/.7/.33/.37: process hygiene, not a product defect.
- ht-z41e.21 retry-journal-downgrade: auxiliary bit not stripped only on a daemon downgrade between journal record and retry.
- ht-z41e.37 root-text-proxy: queue_text_ok uses any target's WAKE_UP; correct while select_binding requires a cooperative_top_level root.
- Dispatcher HumanInput pass-through ignores the seat's programmatic_wake: nothing is typed (native final gate refuses), only an extra selection transaction and a different refusal detail.
- QueueWakeRuntime.roots clears all entries at 256; only leaked (untaken) roots accumulate.
- Queue admission keeps the 60 s focused-pane dwell, so a person viewing the /side chat sees the root wake after a minute; within the focused rule.
- ht-z41e.24 inline final-retirement SQL in seat_config.rs instead of a shared helper.
- ht-z41e.5 TmuxRecipient stays native under --queue-proof; documented in RELEASE.md/AGENTS.md as Herdr-pane seats only.
- ht-z41e.6 spike method deviations (owned PTY, single trials, raw screen text); conclusions consumed and cross-checked by hook_entrypoint fixtures.
- Test-quality minors (seam-only tests, redundant assertions, regression-guard-only test, file placement, missing RED runs) in ht-z41e.3/.11/.33/.36/.37/.40.
- Doc drift minors (ansi-composer README line 22, wake-e2e bindings_idle docstring, REFUSED_OUTCOMES timed_out assumption).
