# V090 final matrix preparation audit

Verdict: REVISE the preparation helpers before final sealing; the observed source selection itself preserves all required historical cases. This is a static preparation audit, not final source clearance or a runtime verdict. No Cargo, product runtime, Git mutation, source edit, or final seal was performed.

The source enumeration was stable at commit `e12e90df6ac0d45f8b7183a598066b5e555c8484`, tree `bba5b70a9f8f6eadc07a10978852fc6bcbd1fa21` (HEAD checked before and after). It used the actual preparation parser and selection algorithm in memory against immutable Git objects. The candidate had advanced since the source68 assignment. Q5/query work remains pending, so 737 is an observed interim count, never the required final count.

## Historical inputs and exact inclusion

| Input | Exact artifact | SHA-256 | Selection |
| --- | --- | --- | --- |
| Original | `resume-composed-matrix-preparation-f330bdab.json` | `d568da428d23aaa0b517d0ab6a656e0f1684634fdf18f4281761b12a35546fad` | 391 = 385 focused + 6 separate |
| Expanded historical floor | `resume-v090-9a573c3f-final-inventory.json` | `c433673ebf52ce5d64b4352f8b9f6d512dcad60edb3a66d5034a7b22f699f3a8` | 657 = 651 focused + 6 separate |
| Observed current source | commit/tree above | immutable source objects | 737 = 731 focused + 6 separate |

All original391 exact keys and all expanded657 exact keys occur in the observed current selection. No selected key lacks a unique source registration. Current target counts are lib651, combined41, hook_entrypoint9, integration35, service1. The parser discovered 5918 registered cases across its five roots but selected only the explicit composed matrix; this is not a routine full suite.

Preparation preserves all 391 individual historical trace rows, including original source path and file/function hashes, while resolving current source registrations. The two reviewed key dispositions remain separate requirements: their historical names were not members of the frozen391, so there is no fabricated subtraction. Main244, the source-difference author set, the restored Main availability control, and the imported negative AutoPeer control remain in the selection. Missing required negative source blocks final sealing.

## Actual gap and proposed repair

The existing helpers enforce original391, Main244 and currently discoverable author cases, but neither independently pins the expanded657 artifact nor requires every one of its keys. Removing a historical author-only case from current source removes it from `actual_author`; unrelated additions can keep the total above657 while the historical case silently disappears. A total-count lower bound does not close this gap.

Scratch successor patch: `resume-v090-matrix-historical-floor.patch`, SHA-256 `79865e314cd3f8ed752a94c003e86359a4002bcb60a02037cfc6239b7775a5ff`. It pins the actual expanded657 file above, unions its normalized exact keys into the existing source selection, preserves 657 individual historical source traces, and makes the runner independently require complete trace/key membership and the historical execution class. It preserves the existing source parser, author discovery, Main keys, five targets, original391 map and six separate Cargo invocations. The parent alone applies it when quiescent. No existing helper was edited in this audit.

## New source registrations

The 80 additions relative to expanded657 are accounted for by these source files. They are selected by current source identity, not by a wildcard runtime expansion.

| Source | Added cases |
| --- | ---: |
| `tests/service/mod_channels.rs` | 15 |
| `tests/cli/watch.rs` | 12 |
| `tests/service/delivery_pull.rs` | 14 |
| `tests/service/q4_accept_controls.rs` | 4 |
| `tests/cli/q5_continuation_public.rs` | 6 |
| `tests/store/direct_lazy_writer_floors.rs` | 5 |
| `tests/store/direct_lazy_readers.rs` | 6 |
| `tests/service/mod_ack_dispatch.rs` | 5 |
| `tests/service/q4_body_controls.rs` | 4 |
| `tests/store/materialization.rs` | 1 |
| `tests/cli/direct_send.rs` | 2 |
| `tests/store/queries.rs` | 2 |
| `tests/store/notice_replay.rs` | 2 |
| `tests/service/cooperative.rs` | 2 |

The child modules resolve through their actual registration chains:

- lib `service::delivery_pull::tests::q4_accept_controls::*`: 4 via `tests/service/delivery_pull.rs:34`.
- lib `service::delivery_pull::tests::q4_body_controls::*`: 4 via `tests/service/delivery_pull.rs:14466`, including the old-episode private lazy-tail retirement control.
- lib `store::control::tests::direct_threads::reader_controls::*`: 6 via `tests/store/direct_threads.rs:1006`.
- lib `store::control::tests::direct_threads::writer_floor_controls::*`: 5 via `tests/store/direct_threads.rs:1009`.
- combined `lazy_markers::q5_continuation_public::*`: 6 via `tests/combined.rs:103` and `tests/cli/lazy_markers.rs:756`.
- lib `store::invitation_content::notice_replay_tests::*`: 2 via `src/store/invitation_content.rs:3081`.

These stars describe namespaces only; generated execution predicates enumerate every full test name exactly. Final preparation must be refreshed after all Q3/Q4/Q5 source is finalized. New pending query/Q5 tests will enter via the same recursive source registration logic and may increase the count.

## Six mandatory recovery cases

All six remain target `combined`, source `tests/cli/lazy_recovery_context.rs`, class `mandatory_cargo_separate`:

```text
lazy_recovery_context::lazy_recovery_agent_mixed_submission_pins_ambient_journal
lazy_recovery_context::lazy_recovery_agent_mixed_submission_pins_journal
lazy_recovery_context::lazy_recovery_human_submission_pins_ambient_journal
lazy_recovery_context::lazy_recovery_human_submission_pins_journal
lazy_recovery_context::lazy_recovery_prior_durable_ref_on_second_record_failure_pins_ambient_journal
lazy_recovery_context::lazy_recovery_prior_durable_ref_on_second_record_failure_pins_journal
```

The runner lists all selected cases first, executes only the focused set with nextest, then invokes each mandatory case separately as `nice cargo test --locked --all-features --test combined <exact-name> -- --exact`. Its source checks retain the 30-second/20-millisecond projection wait settings. The proposed historical floor additionally checks every historical classification, so no mandatory case can quietly move into the focused class.

## Parent commands after finalized source review

Only after Q5/query work, helper patch review/application, final source review and a clean committed candidate, capture a unique fresh label and refresh both artifacts together. These commands are instructions; they were not executed in this audit.

```sh
matrix_source='/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090'
matrix_ws='/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish'
matrix_commit=$(git -C "$matrix_source" rev-parse HEAD)
matrix_tree=$(git -C "$matrix_source" rev-parse 'HEAD^{tree}')
matrix_label="final-$(git -C "$matrix_source" rev-parse --short=12 HEAD)-$(uuidgen)"
python3 "$matrix_ws/resume-v090-prepare.py" --source "$matrix_source" --tree "$matrix_tree" --label "$matrix_label" --reviewed-final-commit "$matrix_commit"
```

Final mode refuses a different HEAD/tree, dirty tracked/index state, merge state or pending required negative. Review the new inventory and trace maps; do not reuse the historical final seal or force a737 count. The final selection must retain all651 historical focused cases plus the6 mandatory cases and every current registered composed addition.

When the sole warmed Cargo lane is available, run the refreshed exact matrix once with a fresh absolute evidence directory:

```sh
matrix_evidence="/private/tmp/ht-C-v090-${matrix_label}"
python3 "$matrix_ws/resume-v090-run.py" --source "$matrix_source" --inventory "$matrix_ws/resume-v090-${matrix_label}-inventory.json" --selection-seal "$matrix_ws/resume-v090-${matrix_label}-selection-seal.json" --evidence-dir "$matrix_evidence" --run
```

`--run` already performs exact nextest enumeration before execution. Omitting it still invokes Cargo listing/build and requires a distinct fresh evidence directory; it is not a read-only metadata audit. The runner uses only lib/combined/hook_entrypoint/integration/service, with exact binary/name predicates and exact-list comparison. It retains the sole warmed target, isolated config paths, automatic-envelope serialization, source/index/executable stamps, scoped leak checks and separate six-case accounting. Final PASS requires execution and cleanup evidence; metadata preparation supplies none.

Reviewed original helper hashes: prepare `eb83ebd3e8cf7f68fdc08cfb53cca12096c896296b32191dced950bd3c556484`; runner `dce287cba7abcd89e0a0fce0ad63af50ac2bf1e333deff5391f7c4d09cbc267f`. An inherited historical `parent_runner_requirements` prose count can remain historical, but must not be presented as the refreshed count; actual command selection and counts come from current rows.
