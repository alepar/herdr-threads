# CLI names, recent picker and durable handoff evidence

Recorded 2026-10-04 on branch `cli-human-targets`. Product implementation was independently reviewed through `3f7f62cd14423400d19b51bfced01ecb15903a08` (Task 4), after reviewed scoped targets (`70b7568a`), optional names (`a9957290`) and picker/nickname repairs (`e7909191`). The original documentation/help/demo snapshot is `1cc77ce2`, with source-claim corrections in `4420c09d`. Coordinator integration/full-suite/release checks remain separate gates; this report does not claim a published release.

Final runtime review and fresh scoped fix review **PASS** at `a423fe15cd26ef5865fc3e5ece42f0399dcc5f88`; coordinator Task 5 walkthrough/source-claim review is **COMPLETE/PASS**, including the docs-only corrections. Accepted fix verification reports **50 distinct targeted tests passed**, all-target/all-feature Clippy, default-feature gate, format/diff checks, a narrow owned empty/cancel PTY exercise and scoped no-leak audit passed. These reported results were not rerun for this metadata update; the reviewed frozen handoff is ready, with coordinator integration, final full sweep and release pending.

This package implements the [approved design](../../superpowers/specs/2026-10-03-cli-names-handoff-design.md). Labels and names select requests; exact IDs freeze before durable submission. They grant no seat continuity or caller authority. The daemon still decides against its canonical view. There is no fuzzy execution selection, first-match ambiguity fallback, implicit acceptance, permission expansion or new receipt provenance.

## Public contract checked

- Pane IDs, exact labels and live agent names resolve within explicit parents or the caller's live tab/workspace, never focus. Outside Herdr, supply parents or an exact pane ID. Conflicts show escaped candidates and qualification guidance. Reads never allocate seats; guarded recipient resolution retains restoration holds.
- Names are optional, separate from topics/goals, exact case-sensitive UTF-8 (1–128 bytes, no controls), and nonunique. Lookup includes archives and nonmembers in the selected instance; existing exact IDs win. Every public thread selector/filter uses the same resolver, while invitation/message/job/recovery namespaces remain exact-only.
- Bare `read` progressively browses recent paged all-instance channels, including archives; fuzzy name/topic matching is confined to the human picker. All three TTYs, usable TERM and no agent/cooperative caller or machine/JSON flag are required. Enter selects a canonical ID; Esc/Ctrl-C cancel with exit 0. Explicit history/follow is read-only. Human author/event-recipient nicknames are relative to the live caller and bounded/escaped; missing host labels do not break history. Machine/recovery identifiers remain canonical.
- Handoff requires one explicit pane, exactly one new/existing thread form and one 1–1024-byte durable body after `--`. Repeated `--agent-arg=VALUE` preserves one native argv element each. Repeated `--require-ack-pane PANE` preserves one recipient each. Create/invite/send precede guarded launch; existing channels require joined sender membership. Startup contains only a fixed canonical inbox/read bootstrap, not the task body.
- Exact keyed durable steps survive failure. Private synced progress and a possible-start fence prevent automatic duplicate launch. A typed confirmed non-submission can be repaired/retried; unknown errors, early harness exit and crashes across possible-start stay fenced. Recovery reports completed IDs, phase and inspect/manual-launch commands without the body. Successful launch by itself records neither invitation acceptance nor ACK/check-in and does not establish task completion.
- Validated default agent text inbox submits only fully displayed canonical pending agent receipts after the whole page is written and flushed. Machine/JSON/explicit-seat/pane inbox selection stays read-only. Humans owe no ACK; waiver records no ACK. A live pane lookup does not rebase a stale lifecycle claim. ACK means receipt, never agreement or completion.

Wire protocol **4** refuses older peers before dispatch because the shipped v0.2.1 result types reject unknown fields; upgrade CLI and daemon together using the installer lifecycle. Optional absent fields preserve historical stored payload/digest compatibility. Schema **19** adds names and schema **20** adds indexed recent activity; final schema is **20**. Handoff uses private intent/progress files and adds no SQLite migration.

## Reviewed implementation checks

These are selected final results from the Task 1–4 implementer reports, accepted by their independent reviews. They were not rerun merely to assemble this report. Every command below used `HT_LEAK_RUN_ID=cli-names-handoff-20261003` and all features. Commands are repository-relative; private roots and personal paths are omitted.

| Exact command | Reported result and coverage |
| --- | --- |
| `nice cargo test --locked --all-features --lib cli::panes` | 19 passed, 0 failed; scoped exact matching, ID/parent validation, escaped ambiguity and no unresolved name dispatch |
| `nice cargo test --locked --all-features --lib cli::cooperative_tests -- --nocapture` | Task 2 final: 36 passed, 0 failed; recipient/caller separation, read-only foreign inbox and private wire routes |
| `nice cargo test --locked --all-features --lib thread_names -- --nocapture` | Task 2 final: 16 passed, 0 failed; 23 selector positions, indexed all-instance uniqueness, migration/authorization/replay and wire compatibility |
| `nice cargo test --locked --all-features --lib recent_picker` | 9 passed, 0 failed; recent index/backfill/activity publication and 206-row archive-inclusive paged traversal |
| `nice cargo test --locked --all-features --lib cli::follow::read_cost_names` | Final repair: 10 passed, 0 failed; relative nicks, caller movement/recovery, machine IDs and bounded host lookup |
| `nice cargo test --locked --all-features --lib cli::irc::tests` | Final repair: 14 passed, 0 failed; escaped component budgets retain the pane component |
| `nice cargo test --locked --all-features --lib store::connection::tests` | Final repair: 104 passed, 0 failed; includes rejection of a malformed recent-activity column default |
| `nice cargo test --locked --all-features --lib handoff` | Task 4 final: 20 passed, 0 failed; parser, exact keys, lost responses, durable crash/output recovery, frozen IDs/scope and typed native refusal |
| `nice cargo test --locked --all-features --lib codex_early_exit_inside_the_observation_window_is_reported` | 1 passed, 0 failed; submitted early exit remains Possible |
| `nice cargo test --locked --all-features --lib launch::tests` | 59 passed, 0 failed before the final additional launcher case (included in final handoff group); ordinary launch regression coverage |
| `nice cargo test --locked --all-features --lib host::native::tests::guarded_start` | 7 passed, 0 failed; native correlation, refusals, name fallback and cancellation |

Fault evidence used controlled sockets/hosts plus real synced journal files. Mutation probes detected regenerated durable keys and removal of the possible-start replay guard; the restored source passed the final covering checks. This is not power-loss or hardware-failure testing.

## Owned terminal and native exercises

The Task 3 private terminal driver seeded 205 channels, with the sole matching name beyond the first 200 rows. Late-page selection, Esc/Ctrl-C cancellation, empty filter, SIGTERM restoration, agent/dumb-TERM/machine/JSON/nonTTY refusal and query-error restoration passed. UI stayed on stderr, history on stdout and receipts unchanged. Its invocation was `HT_LEAK_RUN_ID=cli-names-handoff-20261003 bash <owned-private>/cli-picker-exercise.sh`; only the temporary path is sanitized. It used three private PTYs, isolated harness directories/state and the repository isolated-Herdr helper. All owned processes were stopped.

Task 4 ran its retained private native driver twice, including against final compiled `3f7f62cd`. It used an owned named Herdr session, private HOME/CODEX_HOME/state/socket and real **Codex 0.160.0**, with `-a on-request`, no credentials and no full-access/network flags. Final exit was 0 with correlated `started` and a `managed_launch` binding. The sender was joined; the recipient remained invited with no accepted invitation. Exactly one durable message, one pending invitation and one pending receipt existed; native argv retained `-a`, `on-request`, ordinary Codex `--no-daemon`, and a canonical inbox/read bootstrap with no task body. No local intent remained after flushed success.

This establishes native startup and durable composition. It does **not** establish an authenticated model turn, autonomous persona conversation, receipt by the model or work completion. Controlled fixtures establish refusal/uncertainty/crash behavior. Neither exercise touched the shared Herdr server or real user configuration. Both ended with scoped `no leaked test processes` results.

## Task 5 fresh checks

All Cargo commands below were run with `HT_LEAK_RUN_ID=cli-names-handoff-20261003`. No product runtime behavior or grammar changed in this task: Rust edits add help text only. Existing parser/skill tests and meaningful script checks cover those changes; no tests merely mirroring prose were added.

| Exact command | Result |
| --- | --- |
| `nice cargo test --locked --all-features --lib cli::commands::tests` | 55 passed, 0 failed; build 12.80 s, tests 0.05 s |
| `nice cargo test --locked --all-features --lib cli::skill` | Final: 8 passed, 0 failed; build 6.17 s, tests 0.02 s; embedded guide 157 lines, strict `<200` bound unchanged with eight-line communication headroom |
| `nice cargo clippy --locked --all-targets --all-features -- -D warnings` | Exit 0, 10.94 s |
| `nice scripts/check-default-features` | Exit 0, no compiler diagnostics |
| `nice cargo build --locked --all-features --bin herdr-threads` | Exit 0, 6.19 s |
| `target/debug/herdr-threads read --help` | Exit 0, 48 lines; recent picker/terminal/cancel/relative nick/read-only contract |
| `target/debug/herdr-threads send --help` | Exit 0, 48 lines; exact scoped pane/agent selection and repeat-one-recipient semantics |
| `target/debug/herdr-threads invite --help` | Exit 0, 40 lines; shared target help |
| `target/debug/herdr-threads handoff --help` | Exit 0, 67 lines; shared target help and existing durable/native/retry contract |
| `bash -n scripts/demo-tea-party.sh` | Exit 0 |
| `python3 .superpowers/sdd/2026-10-03-cli-names-handoff/task-5-script-check.py` | Exit 0; all dry-run/mock watcher checks below passed; retained readiness driver is private scratch, not a shipped test |
| `cargo fmt --check` | Exit 0 |
| `git diff --check` | Exit 0 |
| `scripts/check-no-leaked-processes --run-id cli-names-handoff-20261003` | Exit 0: `no leaked test processes` |

The script check dry-ran `--label tea-check --cwd '/tmp/project with spaces' --codex-home '/tmp/profile with spaces' --claude-arg '--setting=value with spaces;$(never-run)' --codex-model 'model with spaces' --codex-profile 'profile with spaces'`, then parsed generated commands through Bash into an owned mock CLI. It verified the frozen host pane ID, channel creation before four handoffs, one durable body per handoff (740/732/726/713 UTF-8 bytes), exact native option/value elements including spaces/metacharacters, repeated separate recipients, host `--no-allowed-tools`, and early refusal of an ASCII-invalid label. It did not execute the substituted text. Watcher success chose `data.summary.thread` over a conflicting incidental row; exact NotFound retried, while Conflict, HostUnavailable and malformed success failed visibly without a read. Owned mock children exited and fixtures were removed.

The root separately audited unchanged compiled `thread name`, `thread rename`, and `thread list` help at the Task 4 source, all exit 0. README examples retain the exact repeated flag grammar; the install-guide support backlink is already incorporated. The updated demo uses durable named-channel handoffs and native independent approvals. Historical tea-party media is captioned as the earlier manual flow; no new recording or live demo was run.

The scoped process audit required read-only escalation because sandboxed `ps` is denied; its final result was exit 0, `no leaked test processes`. Sandboxed `nice` reported `setpriority: Operation not permitted` but Cargo completed successfully. Incremental builds and lint remained under one minute. No full suite/performance gate, fresh native exercise, shared-server operation, release, merge, push, stash or worktree cleanup was performed. Final reviews passed and the frozen readiness handoff is ready; coordinator owns the integrated full sweep and publication.
