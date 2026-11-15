# Codex queue wake: live spike

Date: 2026-11-14. Installed binary exercised: `codex-cli 0.167.0`.
Version is an observation, not an admission gate. No production integration or wake policy changed.

**A queued content-free marker can wake a loaded embedded `--no-daemon` TUI. Its normal turn-start hook runs, injects controlled mail, and the model receives it. A bare CLI switch is not ready for production: stale markers, duplicate submissions and unloaded sessions need explicit handling.**

Queue content remains user input. These tests use it only for the existing wake marker; regular message bodies remain on the hook path.

## What was exercised

Real authenticated Codex TUI processes ran in owned PTYs with an isolated HOME, CODEX_HOME, XDG directories and working directory. Each run used a private mode-0600 regular-file copy of credentials from a permitted aisw profile. The source file stayed unchanged, and none of the copies refreshed credentials. The receiver used `--no-daemon`, `CODEX_EXEC_SERVER_URL=` and the normal workspace-write sandbox. Only the test-owned hook definition bypassed hook trust, using the CLI's dedicated hook-trust flag; sandbox and tool approval protections were not bypassed.

The controlled hooks log session/turn IDs and inject tokens via `additionalContext`. They deliberately do not call herdr-threads. This isolates queue transport and hook invocation; it does not qualify real seat attribution, canonical receipt ACKs or the complete Herdr wake matrix.

## Outcomes

| Probe | Observation | Evidence |
|---|---|---|
| Loaded idle TUI | Queue CLI returned an entry ID; a new turn ran and the normal hook injected a token echoed by the model. End-to-end response was 6.8 s and 8.0 s in two samples, including model time. | [Initial run](run-1/results.json), [matrix](run-3/results.json) |
| Draft | The queued wake ran; the pre-existing unsubmitted draft survived and was subsequently submitted intact. | `draft_is_preserved` in the matrix |
| Duplicate CLI calls | Two identical submissions started two turns. | `duplicate_submissions_are_not_deduplicated` in the matrix |
| Busy receiver | Queue did not start a second turn during the held tool. Mail arrived through a hook in the busy turn, but the retained marker then caused an extra turn. | `busy_deferral_and_stale_marker` in the matrix |
| Interrupted receiver | An accepted marker remained pending after interruption. An explicit user-style continuation allowed it to drain. | `interrupted_queue_is_held`, `explicit_turn_releases_interrupted_queue` |
| Wrong storage | A sender using a different scratch Codex home could not reach the target; its queue command failed. | `wrong_storage_does_not_reach_target` |
| Model picker | A queued turn started while the picker was open. This is an observation, not a pass for the current modal-hold policy. | `modal_picker_queue_behavior` |
| Root with `/side` displayed | Queueing to the captured root UUID started the root turn; it did not retarget the displayed side chat. | [Lifecycle run](run-6-lifecycle/results.json) |
| Unloaded embedded target | Queue accepted the marker, but no process existed to receive it and no wake occurred during the observation window. | `unloaded_embedded_target_does_not_wake` |
| Explicit resume | Resuming that exact root UUID consumed its retained marker and the hook-injected token reached the model. | `explicit_resume_consumes_queued_marker` |

The duplicate, stale-marker, interruption and unloaded results are limits, even though the characterization cases correctly observed them. A `passed` observation in a raw file is not a declaration that the behavior satisfies herdr-threads policy.

## Queue RPC works without attaching to the receiver

A separate private `codex app-server --listen stdio://` process, using the same scratch storage, could add, list and delete queue entries for the embedded receiver. Its `thread/loaded/list` remained empty: it did not resume or take ownership of the target. The target's own watcher received the added marker and its own hook delivered the token.

This refines the earlier app-server distinction: **we cannot externally attach to the embedded TUI for live context injection, but a separate app-server can operate on its persistent queue.** This is a queue control plane, not a replacement for the receiving runtime or its hooks.

The generated schema and actual RPC calls expose:

- `thread/queue/add`: accepts `threadId`, `clientUserMessageId` and user input; returns a distinct `queuedSubmission.id`.
- `thread/queue/list`: lists pending entries, including both identifiers.
- `thread/queue/delete`: removes the exact queued-submission ID. First delete returned `deleted: true`; repeating it returned `false`.
- `thread/queue/changed`: a thread-level queue-change notification, not a model-delivery receipt.

The experiment initialized its private RPC client with `experimentalApi: true`; it did not establish that every queue method requires that capability. Capability status must be observed rather than inferred from the experimental schema export.

### Retry identity is not idempotency

The first RPC probe intentionally tested the hypothesis that repeating `clientUserMessageId` deduplicates an add. It failed: the same client ID created two different pending submission IDs. Replaying that client ID after consumption also caused another turn.

The original [RPC run](run-4-rpc/results.json) is retained with its failures. Its first cancellation probe deleted one of the two entries, leaving the duplicate; the resulting extra wake was therefore not a failure of exact-ID deletion. The corrected [reconciliation run](run-5-reconciled/results.json) explicitly deleted both entries. The held busy turn finished, and no subsequent marker turn appeared during the 20-second observation window.

This establishes that pending-marker cancellation is available. It does not establish a race-free cancellation guarantee once the receiving runtime has already consumed an entry. Neither an absent row nor a failed repeated delete proves the model received the marker.

## What signal do we get back?

There are distinct observations:

1. **Accepted:** CLI/RPC returns a queue entry ID. This proves enqueue acceptance only.
2. **Pending or removed:** queue list/delete observes the persistent queue. Removal can mean consumption or explicit cancellation; it is not an ACK.
3. **Turn reached:** the receiving `UserPromptSubmit` hook sees its native session ID, turn ID and wake prompt. This happened in the live tests.
4. **Model received controlled mail:** the model echoed a token available only from the hook. The sanitized transcript records this separately.

Thus the queue does not supply an end-to-end delivered receipt, but it does cause the ordinary hook to run. The existing hook/contact path can provide wake-arrival evidence. Body delivery and its canonical flush-based receipt semantics should remain there.

## Consequences for implementation

Do not replace Native wake with an unconditional `codex queue` call.

- Submit only for an eligible idle, loaded binding. Codex's queue has no herdr-threads priority or threshold knowledge. No priority adapter was implemented here, so p1/T=0 and p0/T=0 integration are not claimed tested.
- Keep hooks for regular bodies, busy-turn delivery and receipt handling. Queue only the existing content-free marker.
- Reconcile queue entries and cancel stale ones before permitting a second wake transport. CLI retries are not idempotent; the RPC client ID is useful for correlation but does not deduplicate.
- Preserve interruption and modal policy. Never force queue dispatch merely to defeat an interruption hold. The picker observation shows queue alone does not implement the current modal gate.
- Preserve a dormant-target path. An unloaded embedded session does not wake itself; explicit resume is a separate operation. Native fallback must be chosen before enqueue, or after safely accounting for a retained marker.
- Keep capability selection and queue command/RPC details inside the Codex adapter, with neutral policy seams and no harness-name branches in core.

The remaining qualification gaps are an actual herdr-threads adapter, canonical priority/receipt behavior, crash/lost-response recovery, cancellation-versus-consumption races, multiple processes sharing one native session, and timing/soak coverage across installed configurations. The spike does not authorize or implement those changes.

## Reproduction and retained evidence

Run only with a user-authorized live credential copy source. Set `SPIKE_AUTH_FILE` to an aisw profile's `auth.json`, not a real Codex home. The scripts create their own private copies and remove them during cleanup.

```sh
python3 docs/validation/codex-queue-spike-2026-11-14/spike.py \
  --auth-file "$SPIKE_AUTH_FILE" --output "$SPIKE_OUTPUT" --matrix
python3 docs/validation/codex-queue-spike-2026-11-14/rpc_spike.py \
  --auth-file "$SPIKE_AUTH_FILE" --output "$SPIKE_RPC_OUTPUT" --reconcile-duplicates
python3 docs/validation/codex-queue-spike-2026-11-14/lifecycle_spike.py \
  --auth-file "$SPIKE_AUTH_FILE" --output "$SPIKE_LIFECYCLE_OUTPUT"
```

The scripts use normal Codex sandboxing, owned process groups and a unique per-run process tag. They do not start or stop the shared Herdr or herdr-threads services. They are manual spike drivers, not unattended CI tests. Do not point their output at an existing evidence run.

[Selected generated protocol fields](protocol-schema.json), sanitized hook/model observations and RPC replies are retained. Full credential files, runtime databases and complete developer-context transcripts are not retained. The terminal captures contain controlled test prompts and UI diagnostics, with scratch paths normalized and terminal control sequences stripped. These are text captures, not reconstructed screen recordings.

One startup attempt, [run 2](run-2/results.json), failed because a scratch-only MCP configuration override lacked a transport. Removing that override allowed subsequent runs to start; this is a driver setup failure, not a queue result. The original failure is retained rather than omitted.

[Cleanup verification](cleanup-check.json) found no remaining tagged spike processes or scratch roots. The lifecycle run had one detached descendant, which cleanup stopped; the follow-up check confirmed its PID no longer existed. Every recorded source credential hash remained unchanged and no private copy refreshed credentials.

Validation: Python drivers and JSON evidence checked; `cargo fmt --check`, locked all-target/all-feature Clippy with warnings denied, harness-neutral and wire-compat guards passed. The cold worktree crate lint took 1m42s; this was not an incremental edit benchmark. Default-feature guard status is recorded in `validation.json`. No full Rust suite or release wake matrix was run for this evidence-only change.
