# Final review (bounded) — epic ht-z41e, branch super-auto/per-seat-config-side-wake

Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head f5253690f6c8a7cbd008f444bad24dce1c96d714 (both verified with `git rev-parse --verify`). Invocation inv-ac7d9dd6-02d5-4d73-a42d-e52b0d6b316f, bound with `bind-review-source` before any write.

## Review route

The selection sidecar `review-inputs.json` reads `status: unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, no omissions, and keeps two provenance gaps (run.md:114, inv-f5f76bdd: incomplete claimed collection; incomplete split bundle). That allows only a bounded captured-source review. I read the full changed-path inventory (588 paths, 90,753 insertions). I then inspected the product paths that carry the epic's behavior: the registry, store, CLI, protocol, dispatch, queue, selection, scheduler receipt, hook evidence, composer, doctor, settings, migrations, and the governing docs (TRUST-POLICY, AGENTS, SKILL, design). `bounded-coverage.json` lists exactly the hunks I inspected. Tests, the wake e2e driver scripts, evidence logs, the remaining product files and the run-doc corpus were not inspected (see the untested-scope list). The full test suite has not been run on this branch: the caller runs it after this invocation, so nothing here says the branch is tested.

## Requirement baseline trace (from production entry points at head)

- B1 (replace the node setting with a seat-level one): `InstanceSettings.programmatic_wake` is now `Option<bool>` and inert (src/daemon/settings.rs). `ServiceConfig` and `QueueWakeRuntime` no longer carry the flag (src/service/config.rs, src/app.rs). The flag is read per seat in `queue_wake_selection` and again at `arm` (src/store/wake_intents/selection.rs, src/store/wake_intents.rs). `doctor` flags the obsolete key. Delivered.
- B2/S1 (generic key = value, closed registry): `REGISTRY` in src/protocol/seat_config.rs. Table-backed keys need no migration. Unknown keys are refused with the list of keys. Delivered.
- B3 (CLI read/write): `config seat get|set|unset` and `human config seat set|unset` (src/cli/config_seat.rs, src/cli/commands.rs, src/cli/mod.rs `config_seat_need`). Delivered.
- B4 (list with descriptions): `render_list` / `render_list_json`, static, no daemon. Delivered.
- B5 (prose instruction works): the SKILL.md "Per-seat settings" line and docs/agent-usage.md name the commands. An agent told in prose can run `ht config seat set nudge_threshold N`. A human-tier key is refused with the exact `ht human` spelling (`refuse_agent_config_write`). Delivered.
- B6 (tiered write, all readable): `GetSeatConfig` has no claim, and any caller can read, subagents included. The agent tier is `PermitMutation::SetSeatConfig`, own seat only, never a subagent or a read-only binding. The human tier is `seat_config_write` → `OperatorActor` → `set_operator`. The CLI picks the tier from an immediate `human` argv[1]. Ask rules cover `human config seat` for Claude and Codex under both spellings. Delivered; the accepted limit is recorded in TRUST-POLICY.
- B7 (nudge threshold settable through the non-human path): `nudge_threshold` is agent tier and column-backed. `set_agent` reuses `write_threshold` with `refuse_readonly = true`. Delivered.
- B8 (wake routed to the top-level session in the background while `/side` is open; spikes): spike evidence exists under the run dir. Root selection is `process_root` plus Codex `wake_process_root` over the `auxiliary_session` bit, set from null `transcript_path` on SessionStart, UserPromptSubmit, PreToolUse and Stop (src/harness/codex.rs, src/store/bindings.rs `mark_auxiliary`, gated by `seat.auxiliary_session_v1`). Queue admission is root-scoped (`admit_queue_wake`). Receipt is root-session scoped (`take_root`, `contacted_since_session`). The code path exists from production entry points, with two gaps. First, the live qualification at the final SHA (ht-z41e.10, ht-z41e.43) is still open, so no real Codex has exercised side-queue, side-native-defer, or the focused-pane and busy-side observation. Second, admission refuses a typed draft in the side composer, which narrows the design's own §Selection "Queue admission" and §Testing ("a displayed side draft arms"). That narrowing is recorded only as an orchestrator note in Post-Implementation Notes.
- B9 (no native typing into the side chat): `CODEX_PLACEHOLDERS` drops `Ask a follow-up question`, and the side composer reads `Text`, so native admission refuses and retries. Delivered.
- B10 (brainstorm decisions): no migration of the node key (CHANGELOG states it). Own seat for the agent, any local seat for the human (`--seat` only on the human verbs). Key over the existing column (`ConfigBacking::NudgeThresholdColumn`). Delivered.

## Cross-task integration analysis

- Dispatcher seam (src/notification/dispatch.rs:332-336, 376-381, 467-474). `queue_candidate` is `self.queue.is_some() && poke.is_none() && for_seat_attention() && one_queue_process(targets)`. The queue runtime is wired whenever a guarded state dir exists (src/app.rs:2132-2156), regardless of any seat's `programmatic_wake` and regardless of harness. So since ht-z41e.42, every one-process seat-attention wake skips the early `ActiveTurn | HumanInput` refusal. That includes default-off seats and Claude seats. These wakes go on to `pane_foreground`, `owes_wake`, a queue selection transaction (which returns `Native` for an off seat) and finally `submit_prompt_if_owed` → `idle_wake_gate`, whose fresh status and composer read still refuse. So rule 4 holds today, but only through the final gate. On the default path the early defense is gone, and every busy or typed retry pays extra host and store round trips. The design's §Unchanged behavior promises the native path is unchanged. The reservation already carries `attention_witness.programmatic_wake()` (the scheduler reads it), so gating `queue_candidate` on it, and on the adapter having a `wake_control`, is a small change that restores the native default.
- Composition of selection, admission and receipt checks out. `process_root` requires one adapter, pid, start stamp and boot. Codex's `wake_process_root` names a root only when every other candidate is auxiliary. A lone concurrent or auxiliary binding is never a target. `arm` re-reads the seat flag (stale authority → NoEffect → native fallback). `take_root` is filled only on Submitted/Unknown, so a native fallback stays seat-scoped. `AwaitingReceipt` lifts suppression on a `programmatic_wake` flip.
- Hook evidence gating. `drop_unadvertised_auxiliary` runs on the cached-event dispatch (bridge.rs:657) and on the tool-boundary check-in (bridge.rs:1315). `decode_request` keeps the frozen bit. `exact_check_in_replay` clears it before comparing. The journal retry path (src/cli/retry.rs) is not gated: a ledger minor that matters only after a daemon downgrade.
- Seat config store. `apply` re-judges wake (`note_new_attention`) on a Table change. `write_column` does the same for the threshold and upserts the audit row on every write. Final retirement deletes rows in both the local and the replicated retirement-job inserts. An operator write to a finally retired seat is refused with `seat retired: <seat>`. `seats_with_value` feeds doctor read-only.
- `DeliveryPullState.session_contacts` (src/service/delivery_pull.rs:109, 164) gains one entry per (seat, native session) ever seen and is never evicted. The per-seat `contacts` map is bounded by seats; this one grows with the number of sessions over the daemon's lifetime. It is small per entry, but unbounded.
- `QueueWakeRuntime.roots` clears the whole map at `ROOTS_MAX` (256). That only happens if the dispatcher stops calling `take_queue_root`, and then a pending queued receipt loses its root and gets no `AwaitingReceipt`. That is benign: before this run, queue attempts had none either.

## Ledger triage

- `Recurring minor: [process-class: missing-evidence]` (6 tasks): process hygiene. Test logs live only in scratch `/tmp/sp-6cd888ee`, and no structured evidence arrays were written. This does not block landing on its own. It does mean per-task test claims cannot be re-verified from the repo, which strengthens the need for the caller's full-suite run.
- Line 18 (ht-z41e.1, ignored `herdr_release_wake` side-close path and inverted `/side` expectation never executed): a minor that names an ignored test, so it was misgraded. Triaged as must fix; it folds into the open live qualification ht-z41e.10.
- Line 69 (ht-z41e.8, side-queue and side-native-defer untested against real Codex), line 298 (ht-z41e.30, I/O paths covered only by pure helpers): the same gap, ht-z41e.10 and ht-z41e.43 open. Must fix.
- Lines 60 and 61 (ht-z41e.7 marker deviation, mis-enrolled lone side): superseded by redesign cycle 1 (`auxiliary_session`; a lone auxiliary binding is never a target, selection.rs `[only]` arm). Resolved at head.
- Line 17 (ansi-composer README drift): fixed at head (README rows now say the side capture reads `Text`).
- Line 153 (retry journal not stripped against an older daemon): downgrade only. Acceptable as a follow-up.
- Line 480 (extra host round trip for a lone working root): accepted by the amended design. Related to the dispatcher seam above, which widens the same cost to default-off seats.
- Line 481 (`scripts/check-default-features` not run): merge-time gate. Untested.
- Lines 37, 44, 45, 52, 64, 75, 76, 232, 239 (seam-only or weak integration tests, missing armed-unknown variant, tmux seats native under `--queue-proof`): coverage depth only, no defect named. Acceptable.

## Findings

- [important] src/notification/dispatch.rs:332 — `queue_candidate` ignores the seat's `programmatic_wake` and the harness's queue support, so every one-process seat-attention wake (default-off seats and Claude included) bypasses the early ActiveTurn/HumanInput refusal and relies only on the final native gate. The default path's behavior and cost change, against design §Unchanged behavior.
- [important] B8 qualification — live side-queue/group, side-native-defer/group, the focused and busy-side run, and the ignored `herdr_release_wake` side-close path have not run at the final SHA (ht-z41e.10 and ht-z41e.43 open).
- [important] design/TRUST-POLICY mismatch — design §Selection "Queue admission" and §Testing say a typed side draft admits and arms a queue wake. The code (src/host/native.rs:2357-2367) and TRUST-POLICY refuse typed text. The narrowing was an orchestrator decision, not a human one, and it reduces what B8 covers (a side draft blocks the root wake until it is cleared or the side closes).
- [minor] src/service/delivery_pull.rs:164 — `session_contacts` is never evicted: unbounded growth over the daemon's lifetime.
- [minor] src/cli/retry.rs — a journaled check-in carrying `auxiliary_session` is not gated against an older daemon (ledger line 153).
- [nit] src/protocol/seat_config.rs — the description length test allows < 130 bytes, while the field doc says under 110.
- [nit] src/store/seat_config.rs `apply` — an explicit Table set to the default value (`off` with no row) reports `changed` and `is_default: false` and re-judges wake. Harmless, but `get` then shows `off` without `(default)`.

## Verdict
not ready (bounded review only; live B8 qualification and full suite not run; dispatcher queue-candidate exemption ungated by the seat flag; side-draft narrowing needs human sign-off)

## Must fix before landing
- Gate `queue_candidate` in src/notification/dispatch.rs:332 on `reservation.attention_witness.programmatic_wake()` and on the adapter having a `wake_control`, so default-off and non-queue seats keep the early ActiveTurn/HumanInput refusal and its cost (design §Unchanged behavior); add a test that a default-off busy single-target seat is refused before the queue lane.
- Run the open live qualification at the final SHA (ht-z41e.10, ht-z41e.43): side-queue/group under --queue-proof with a focused pane and a busy side turn, side-native-defer/group, side-nudge and side-draft, plus the ignored herdr_release_wake side-close path (ledger line 18 is a misgraded minor naming an ignored test); B8 is not shown delivered on a real Codex until then.
- Reconcile the side-draft rule: design §Selection "Queue admission" and §Testing say a typed side draft admits, while code (src/host/native.rs:2357) and TRUST-POLICY refuse it; get the human's decision on this narrowing of B8 and make the spec body match.

## Untested scope
- No full-suite measurement exists for this branch (ht-z41e.27 open); the caller runs it after this invocation.
- scripts/check-default-features was not run (ledger line 481).
- Bounded review: whole-range review is still outstanding; provenance gaps retained: run.md:114 inv-f5f76bdd incomplete claimed collection; incomplete split bundle (missing cluster report or slices).
- Not inspected: every tests/** path (including tests/cli/config_seat.rs, tests/store/seat_config.rs, tests/service/seat_config.rs, tests/scheduler/queue_side_dispatch.rs, tests/notification/queue_runtime.rs, tests/scheduler/wake_receipt.rs, tests/integration/*, tests/fixtures/**).
- Not inspected: scripts/wake-e2e.py, scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/tests/test_release_qualify.py, tests/release/release_qualify_test.py.
- Not inspected: src/harness/registry.rs, src/ports.rs, src/protocol/results.rs, src/store/schema.rs, src/store/queries.rs, src/service/kicks.rs, src/harness/mod.rs, src/cli/retry.rs, and the src/host/native.rs test hunks (@@ -4991, -5977, -6630).
- Not inspected: README.md, RELEASE.md, docs/install.md, docs/validation/herdr-release-matrix.md, integrations/codex/README.md, evidence/** logs, and the docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/** corpus other than the design doc.
- Not checked: whether `mark_auxiliary` runs on every check-in path (only `resolve_claim_with_hooks` was read).

## Deferred OK
- src/service/delivery_pull.rs `session_contacts` unbounded growth: small per entry; file a follow-up to evict on binding end or with a TTL.
- `QueueWakeRuntime.roots` clear-all at ROOTS_MAX: reachable only if the dispatcher stops taking roots; no worse than before this run.
- src/cli/retry.rs journaled auxiliary bit not gated against a downgraded daemon (ledger line 153): downgrade-only.
- Recurring missing-evidence cluster (6 tasks): process hygiene; covered by the caller's full-suite run.
- Seam-only and weak-integration test notes (ledger lines 37, 44, 45, 52, 64, 75, 76, 232, 239): coverage depth, no named defect.
- Extra host round trip for a lone working root (ledger line 480): accepted by the amended design.
- Registry description length test bound (130) vs. doc (110), and explicit `off` reading as non-default: cosmetic.
- observe_target_with_composer no longer resets dwell windows for a non-idle composer_ui observation without a composer read (no policy / unknown kind): such panes have no composer typing.
