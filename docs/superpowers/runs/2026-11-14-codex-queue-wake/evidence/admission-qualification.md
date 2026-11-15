# Queue admission qualification

**Verdict: do not enable automatic queue wake through the tested hook-veto route.**
The queue accepts ordinary user input. UserPromptSubmit can prevent model sampling,
but it runs after runtime turn admission, produces a visible blocked/completed turn,
and receives no queue or client correlation ID. It cannot safely suppress only our
obsolete markers. Production code and wake policy are unchanged.

This is the bounded negative qualification for ht-d2jq.1, following the conditional
design at 151c10e98. Version 0.167.0 is a historical observation, not an admission gate.
Earlier behavior, including interrupted markers and extra turns after busy delivery,
is in [the initial report](report.md). The new evidence uses the same isolated PTY,
private configuration and copied credential discipline; it is not an integration or
release wake-e2e pass.

## Observed results

[Run 7](run-7-admission/results.json) completed all five characterization cases.
A passing characterization means the recorded behavior was reproduced, not that the
behavior meets the delivery contract.

| Probe | Observation | Consequence |
|---|---|---|
| Model request witness | Initial ordinary turn emitted PreModelRequest | Witness hooks were active before testing veto |
| Explicit admission block | No PreModelRequest for either blocked queued turn; terminal displayed Blocked by hook and Worked for <1s | Prevents inference, but does not prevent an extra visible turn |
| Identity capture | Complete UserPromptSubmit payload has cwd, hook_event_name, model, permission_mode, prompt, session_id, transcript_path, turn_id | No queuedSubmission.id or clientUserMessageId for exact owned-marker attribution |
| Held admission hook | Queue already empty at hook entry; delete exact accepted row returned deleted:false while hook was held | Admission hook is too late to recover that pending row or cancel it by deletion |
| Hook exit 1 | PreModelRequest and model completion occurred | Hook failure is fail-open; not a correctness fence |
| Lost add reply | While a busy barrier held, list found the pending row by exact client correlation; deletion prevented its later turn | Recovery demonstrated only while the row remains pending; absence is ambiguous |

The full [terminal capture](run-7-admission/terminal.txt) preserves the blocked turn
UI. Run 7 did not retain a separate runtime-events file; runtime start ordering below
comes from upstream source, not an invented runtime trace. The test hook recognizes
marker text solely to exercise veto. Text equality is not proposed as production
ownership evidence: a human can submit the same text.

[Run 8](run-8-receivers/results.json) exited with the startup timeout
`second same-session receiver`. Its second TUI remained at the
[conversation ownership screen](run-8-receivers/terminal.txt), without a second
SessionStart. The planned contention schedule and private SQLite lock stage were
**not exercised**. This establishes an observed same-home ownership gate, not universal
single-receiver exclusivity. The failed probe is preserved rather than counted as a
contention pass.

## Causal boundaries

The following references use official tagged source. Selected passages, exact line
ranges and source hashes are retained in [source inspection](admission-source-inspection.json).

1. Queue service takes its own dispatch mutex, peeks a pending item, calls
   start_turn_if_idle with the copied input, then deletes the row after Started:
   [service.rs, lines 347–444](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/ext/queue/src/service.rs#L347).
   Its mutex is a service-instance field, not a cross-process storage claim
   (lines 236–266). The queue store uses ordinary SELECT and DELETE; deletion returns
   rows_affected > 0:
   [queued_items.rs, lines 109–164](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/state/src/runtime/queued_items.rs#L109).
2. Start-if-idle reserves active_turn and starts the regular task before returning
   Started: [turn_input.rs, lines 479–551 and 633–649](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/session/turn_input.rs#L479).
   That task emits TurnStarted before run_turn:
   [regular.rs, lines 60–93](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/tasks/regular.rs#L60).
   The spawned task and queue deletion can race; this is not a universal total order
   between TurnStarted and deletion. In the observed run, deletion preceded hook entry.
3. UserPromptSubmit then decides whether run_turn continues to model sampling:
   [turn.rs, lines 404–415 and 1004–1041](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/session/turn.rs#L404).
   Blocking input returns without inference, after the runtime task has started.
   The hook builder discards additional UserInput fields when constructing the
   request: [hook_runtime.rs, lines 707–731](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/hook_runtime.rs#L707).
   The [request and outcome handling](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/hooks/src/events/user_prompt_submit.rs#L24)
   defines no queue identity; explicit block sets stop, while a hook error records
   failure without that stop (lines 179–210 and 245–275).

A source-permitted interleaving is receiver A peeks a row, separate controller B
deletes it successfully, then A starts its already copied input. The separate
service mutexes do not serialize A and B. This exact successful-delete race was not
forced in the live test; it is an inference from the inspected implementation.
Thus neither deleted:true nor absence proves that no turn can run. Deleting before
any receiver copies the row removes the pending work; existing pending-deletion
experiments do not establish which side of that boundary an arbitrary delete hit.

For an unknown add result, exact correlation can recover a still-pending row, as
observed. Once it disappears, list cannot distinguish consumption, deletion or a
missed enqueue. Earlier duplicate tests show clientUserMessageId is not idempotent.
Neither disappearance nor enqueue acceptance is message delivery or an ACK.
Interrupted turns remain an existing negative limit: markers wait for explicit
continuation; this extension did not force continuation or override interruption.

## Target and ownership evidence

Hook payload supplied native session and turn IDs. Separately, the test captured
hook parent PID, process start ticks, executable basename and ancestry; the parent
matched the owned foreground receiver. **CODEX_HOME came from process environment,
not the hook payload.** CODEX_SESSION_ID and CODEX_THREAD_ID were absent from that
environment in this isolated run; CODEX_EXEC_SERVER_URL was empty. These observations
identify this test process, not a general supported effective-storage contract.

Config permits sqlite_home and CODEX_SQLITE_HOME, with CODEX_HOME as a fallback:
[config_toml.rs, lines 372–384](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/config/src/config_toml.rs#L372).
Consequently CODEX_HOME alone cannot establish queue storage identity. Querying the
separate control helper's config does not prove the receiver's effective config.

The upstream writer lock is scoped to CODEX_HOME/thread-writer-locks/session.lock:
[writer_lock.rs, lines 19–85](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/rollout/src/writer_lock.rs#L19).
Local creation and resume acquire it:
[live_writer.rs, lines 54–82](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/thread-store/src/local/live_writer.rs#L54).
That supports the observed ownership screen. Queue storage may have a different
namespace through sqlite_home, so this does not establish exclusivity for all
shared-storage/profile configurations. No heuristic seat or binding changes follow.

## Validation and cleanup

Drivers: [admission_spike.py](admission_spike.py), [receiver_spike.py](receiver_spike.py).
Both parse as Python; run 7's five characterizations passed; run 8's ownership gate
prevented its planned experiment. No Rust or production model-visible string changed.
Run 7/8 results record unchanged source authentication and no private-token refresh.
All owned subprocesses, scratch homes, credentials and SQLite files were removed;
[cleanup verification](admission-cleanup-check.json) records no remaining tagged
processes or scratch roots and no sensitive-pattern matches in the new artifacts.

Repository checks: all-target/all-feature Clippy with warnings denied passed (58.70s),
format check passed, harness-neutral check passed, wire-compat check passed.
Default-feature guard passed.
No full suite was run for this evidence-only change. Build artifacts are removed
before handoff. Astra owns final design disposition; the activation gates remain closed.
