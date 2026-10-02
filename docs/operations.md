# Operations and recovery

Commands here were checked against the built binary's `--help` at `61869bc` (including `setup`, `unsetup`, `setup-status`, `launch` and the top-level `participants` alias of `thread participants`). Behaviour statements come from the design contracts, [agent-usage.md](agent-usage.md) and the deterministic test suite unless marked as observed. Native Claude Code and Codex behaviour is summarized in the [validation report](validation/report.md) (verdict PASS_WITH_GAPS; native rows describe the code at each run's SHA) and is a release claim only once re-run on the release SHA (see [release.md](release.md)). Add `--state-dir STATE_DIR --host-endpoint HOST_SOCKET` when running outside a plugin action, and `--json` for structured output. On a terminal, text output defaults to a human-readable form; use `--machine` for the stable `key: value` form (the default when stdout is not a terminal).

## Exit statuses

| Status | Meaning |
| --- | --- |
| 0 | Success. `daemon ensure` also returns 0 for a reachable **degraded** daemon and prints its limitations. |
| 1 | The request failed (not found, conflict, unauthorized, stale state, store full or corrupt, ...). |
| 2 | Invalid arguments or invalid local context: no caller located, an unsafe state directory or private subdirectory, an invalid cursor or byte budget. |
| 3 | Daemon or host unavailable, daemon version mismatch, store busy, deadline exceeded or cancelled. Run `daemon ensure` and retry. |
| 4 | Unsupported capability in this build or environment, including an unverified caller, a harness version no recipe admits (`setup`, `launch`), and `transport_denied`: a sandbox refused the daemon socket (see [the Codex sandbox socket allowance](install.md#codex-sandbox-socket-allowance)). |
| 5 | Outcome unknown. Inspect `pending-ops` and `retry` the local reference. |

Errors print one line on stderr: `herdr-threads: DETAIL (error_code)`, followed by a `restart:` line when the error carries a restart command.

Lifecycle UX, from [agent-usage.md](agent-usage.md#lifecycle-diagnostics-and-exit-status) and the `lifecycle_ux` tests:

- `herdr-threads --help` and `--version` print to stdout and exit 0. `herdr-threads` with no command prints usage on stderr and exits 2 (observed). An unrecognized subcommand such as `frobnicate` exits 2 with `(invalid_request)` (observed).
- `daemon health`, `doctor` and read commands never start the daemon. When the daemon is not running, every command that needs it exits 3 with `(host_unavailable)` and the `daemon ensure` hint.
- A "no caller located" outcome (no `HERDR_PANE_ID`, a pane with no resolved seat, a seat with no lifecycle check-in context, or `inbox`/`pending-receipts` with neither a pane nor `--seat`) is invalid local context: `(invalid_request)`, exit 2.
- `herdr-threads hook claude|codex` is the exception: it **always exits 0**, whatever fails. The table above does not apply to it.

## Harness setup and launch checks

Full steps are in [install.md](install.md#harness-hooks); these are the operational checks.

- `setup-status claude` and `setup-status codex` report whether the owned user-level hooks are installed (separately from `observed`), which Herdr instance they serve (`instance`), whether the harness version on `PATH` is admitted, and for Codex the sandbox allowance and the hook trust keys. They write nothing. `doctor` repeats the Claude and Codex installation results and the Codex admission, including `hooks.codex.installed: codex X.Y.Z: schema-matched, live-unverified` for a version admitted by schema fingerprint.
- `setup claude` exits 1 when an owned hook or the recorded allow rule `Bash(herdr-threads *)` was edited or removed by hand: run `unsetup claude` and set up again. Exit 4 means the installed harness version is not admitted.
- A user-level installation serves one Herdr instance (state directory and server socket); its hook is silent in panes of any other. To move it, run `unsetup claude|codex`, then `setup` with the new context: setup refuses an installation recorded for another hook command or socket (exit 1).
- Codex runs the installed hooks only once trusted: if `setup-status codex` shows `trust_recorded: false`, start `codex` interactively and trust the herdr-threads hooks in its review (or `/hooks`).
- A sandboxed agent command that fails with `herdr-threads: Operation not permitted (os error 1)` could not write the instance's client journals (`intents/`, `contexts/`): the Codex allowance predates its writable roots. `setup-status codex` shows `sandbox.writable_roots_present: false` with a warning, and `doctor` lists it as a limitation. Run `herdr-threads setup codex` to upgrade it.
- A sandboxed agent command that exits 4 with `transport_denied` was refused the daemon socket by the Codex sandbox: check `setup-status codex` (`sandbox.present`), or, on a Codex version other than 0.159.2 or 0.159.3 (where setup withholds it), choose the sandbox deliberately.
- `launch` exit 5 means the start may have happened: inspect the pane (`herdr agent get` / `herdr agent read`) before launching again. Every submitted start is appended to `STATE_DIR/instances/<digest>/launches.jsonl`.

## Daemon health and lifecycle

- `daemon health` reports the running owner. It never starts a daemon, and exits 3 (`host_unavailable`) when none is running.
- `doctor` is read-only. It reports the version, state directory safety, instance directory, daemon reachability and health, the Claude hook installation for the current project (`installed` separate from `observed`) both recipe registries (`hooks.claude.recipes`, `hooks.codex.recipes`), the admission of the `codex` on PATH (`hooks.codex.installed`: listed, schema-matched live-unverified, refused or not found) and the admission evidence the latest Codex hook stored (`hooks.codex.last_hook`). `hooks.claude.observed` says whether native Claude hook runs are verified; in the cooperative mode it reads `not observable: Claude hook runs are not recorded (cooperative mode)`, which is not a failure. Doctor also checks the environment it runs in (the one the harnesses run in): a `claude` or `codex` on PATH whose hooks are not installed in the files this environment resolves (`$CLAUDE_CONFIG_DIR/settings.json`, `$CODEX_HOME/hooks.json`), a `codex` on PATH that no recipe admits, or a Codex sandbox warning is printed as a `limitation:` line and makes the result `degraded`. `result: ok` means the daemon is `healthy` and doctor found no such problem. It exits 0 when the daemon is reachable (`ok` or `degraded`), 3 when it is not running, unreachable or a different version, and 2 for a missing or unsafe context (`result: unsafe_state_dir`, with no `daemon ensure` suggestion).
- `view --once` prints the compact operator view (health, thread directory, overdue obligations). The plugin's overlay pane repeats it on Enter and quits on `q`.
- `daemon ensure` starts the daemon if none is running and waits up to five seconds for it to become ready.
- `daemon stop` asks the daemon to cancel gracefully and waits for accepted work to drain. It deletes no state.

The daemon is not supervised. Herdr runs the startup entry when its server starts (including a live-handoff successor). If the daemon stops or crashes, commands that need it exit 3 until someone runs `daemon ensure` (for example, through the plugin's `ensure` action). A crashed owner is replaced once its lock is released; the new owner verifies and cleans up the old endpoint itself. Missing or unsafe endpoint evidence is an error, never permission to delete a socket.

Health `state` is `healthy` in the designed cooperative mode: database and schema ready, host reachable with coherent enumeration, no worker (scheduler) failure, a wake path (the cooperative safe prompt or native current execution), and every harness the daemon finds on its `PATH` `cooperative` or `supported`. A harness absent from the daemon's `PATH` does not degrade Health.

`harness.claude` and `harness.codex` (one daemon boot observation of `claude --version` / `codex --version` on the daemon's `PATH`):

- `cooperative`: a recipe admits the installed version; model-issued accept/ACK is recorded as `cooperative_top_level` provenance, not as a native-verified receipt. This is the designed mode.
- `supported`: native-verified model receipt. No recipe declares it in this build.
- `unsupported`: not usable: no executable on the daemon's `PATH` (a note) or a version no recipe admits (a limitation).
- `unknown`: the bounded boot observation has not finished yet.

Hook installation is per harness environment, so `doctor` (run in that environment) checks it, not the daemon.

`notes` are informational facts about the cooperative mode and never degrade Health:

- `harness claude: claude X: listed: recipe ...`, the admitted Claude version;
- `harness NAME not installed: no executable ... on the daemon's PATH`;
- `receipt cooperative: harness cooperative means an admitted recipe without native-verified receipt; ...`, while a harness is `cooperative` (receipts are cooperative, demonstrated live only on Claude 2.1.285-2.1.286 and Codex 0.159.2, schema-matched, live-unverified);
- `wake cooperative: ...`, while wakes use the cooperative safe prompt instead of native current execution.

`limitations` are problems; a `degraded` state names at least one. Read them rather than the state word alone:

- `harness codex: codex X: schema-matched, live-unverified: ...` for a Codex admitted only by schema fingerprint. It is still `cooperative` and does not by itself degrade Health;
- `harness NAME unsupported: ...` for an installed version no recipe admits, and `harness NAME unknown: ...` before the boot observation finishes;
- `host unavailable: ...` when the host socket is unreachable;
- `wake unavailable: ...` when the host offers neither native current execution nor the safe cooperative prompt;
- `scheduler degraded: ...` for a background worker failure (typed, redacted);
- `retirement cleanup pending` / `retirement cleanup degraded`;
- `unresolved seats: N; ...` followed by up to a few `unresolved seat SEAT on PANE (REASON)` lines, while any seat is unresolved (health also reports `unresolved_seats`);
- `binding evidence: backfilled N at store startup, still lacking M now; ...` when older bindings lacked reconfirmation evidence at writer startup. The still-lacking count is rechecked on every health read.

`last_scheduler_tick_at` is the completion time of the deadline scheduler's last error-free tick (about every 100 ms while the daemon runs); `null` only until the first tick of this boot. `last_reconciliation_at` is the completion time of the last reconciliation over a verified coherent host publication.

`notes` and the `cooperative` harness value were added to the Health result in protocol 1 without a version change. A newer CLI reads an older daemon's Health (no `notes`); an older CLI cannot decode a newer daemon's Health, so after upgrading run `daemon stop` and `daemon ensure` with the new executable.

Protocol 2 (B5) added the request's expected daemon boot and the B5 commands. An older daemon cannot decode a protocol-2 request, so `daemon ensure`, `doctor`, `daemon stop` and every command from the newer executable stop with `unknown_wire_version` and the stop-then-ensure hint instead of reaching it. Stop the old daemon with the old executable first.

## Attention digest and notices

[agent-usage.md](agent-usage.md) is authoritative; this is a summary.

- **Pending counts, not history.** The inbox per-thread `warnings`, `invitations` and `pending_receipts` counts, the check-in `warning_count`, and the digest classes count only what is still open. That means invitations not accepted, declined or cancelled; receipts not ACKed; warnings whose condition is still open (overdue invitation or receipt, open unavailability episode); and programmatic service warnings not yet offered to the seat's current occupant. A settled warning leaves the counts but stays in history. The full history, settled or not, is always `warnings --seat SEAT`.
- **Saturation.** Every pending count is read by bounded walks and saturates at 1,000. The matching `*_has_more` flag (`count_has_more`, `invitations_has_more`, `pending_receipts_has_more`, `warnings_has_more`, `warning_count_has_more`) is true when more may be pending. The digest prints `1000+`.
- **Hook digest line.** At SessionStart the hook prints pending attention and `attention digest: invitations=N [ID@THREAD, …]; receipts=N [...]; warnings=N [...]` (at most 4 IDs per class, `+more` beyond). At a tool boundary it compares only the digest token (newest pending invitation, addressed receipt, actionable warning and unavailability episode) with the last token shown to this execution. It prints nothing and makes no check-in when nothing is newer. An ACK or an accepted invitation alone prints nothing. A digest failure fails open: the offer is printed anyway.
- **Notice settlement.** Each check-in offer carries at most 16 of the seat's programmatic notices not yet offered to its current occupant, oldest first, in `notices` (`has_more` when more remain). The hook shows them on an `offered notices: N [NOTICE@THREAD, …]` line. The offering transaction advances that occupant's offered frontier to the last notice carried: one row written, nothing deleted. Both the durable lifecycle check-in and the tool-boundary Current check-in settle their page **once the transaction commits, even if the hook output is later lost**. Such a notice can settle unseen, and it stays in `warnings` history. The read-only `warnings` command never settles anything.
- **Replacement.** The frontier belongs to one occupant (binding generation and execution). A replacement, including each hook lifecycle check-in (startup, clear, resume), starts an empty frontier, so retained notices are offered again from the oldest, one page per check-in.
- None of this is a receipt. Reading, searching, viewing, check-in and hook output never ACK or accept.

## Crash and reconnect recovery

The composed crash matrix (`tests/service/crash_matrix.rs`, ht-4is.11.1) drives the production service over a real SQLite file with test-only failpoints. It separates what the caller observed, the authoritative rows, and pending work. Failpoints are compiled only under `cfg(test)` or the `test-support` feature. `tests/service/check_failpoints_absent.sh` (run by the package suite) checks that an ordinary release build contains none of their names. What the matrix establishes deterministically:

- **Commit and response boundaries.** A crash around send, ACK or wake commit and response gives either an accepted transition or none. A failed or crashed commit creates no accepted transition and no wake. A lost response is recovered through `pending-ops` and `retry` (below), replaying the original message or ACK once. A crash after send but before any wake reservation wakes from the durable rows after restart. A crash after a prompt was written but before the attempt was recorded never becomes an ACK.
- **Downtime.** A late ACK or acceptance after downtime records exactly one overdue warning, then settles. A timely ACK paused across its deadline while the due scan waits never warns. Archiving during pending work still allows late settlement, retirement during pending work never produces an ACK, and a rebind invalidates the old context.
- **Storage faults.** A full disk maps to `store_full` (exit 1): free space and retry; nothing was accepted and history is intact. A locked database gives `store_busy` (exit 3). A corrupt database or an unknown schema fails with `store_corrupt` or `incompatible_schema` (exit 1) and history is never rewritten.
- **Host socket denial and lost host events.** Seats whose host observation is invalidated are **reconfirmed structurally**. The next coherent host snapshot must show the same terminal in the same verified host boot and incarnation as the evidence stored on the seat's latest binding. There is no operator step and no reconcile error. This holds for a resolved seat that never had an occupant binding too, from its own verified structural proof; the agent's first fresh lifecycle check-in then registers it. A seat that cannot be reconfirmed becomes **unresolved** and does not block other seats. Its lifecycle check-ins and mutations are refused with a bounded line saying why and how to repair. Health reports it as described above. Repair a seat that stays unresolved with `seat rebind SEAT --pane PANE_ADDRESS --operator`. The repair fences any context from before it.
- **Binding evidence.** Every binding stores the terminal and incarnation from the observation that proved it. Writer startup backfills older bindings only from matching verified structural proof. The rest are reported as still lacking evidence and need the same operator repair after a host invalidation.
- **Fresh lifecycle check-in after recovery.** A new lifecycle event is admitted at the service's current generation. A retried check-in keeps exact replay. Every other mutation through a stale context is still refused.

The isolated host-recovery validation (`scripts/validate-host-recovery.sh`, ht-4is.11.5) drives a private Herdr 0.9.1 server with stand-in pane processes, no model: rename and reorder, CLI exit, host socket denial and unavailability with structural reconfirmation, daemon SIGKILL across deadlines, missed events across downtime, an observed move, pane-close retirement, host stop/restore with operator repair, and separate instance namespaces. It passed 12 of 12 scenarios on source `fcb37c8` ([report](evidence/host-recovery-validation-tip/report.md)). Recovery with a real model agent in the pane (a killed native agent, a restore under a live session) has not been exercised.

## Bounded reads and continuations

Every collection (`thread list`, `thread show`, `thread participants`, `inbox`, `warnings`, `pending-receipts`, `read`, `search`, `seat list`, `seat inspect`, `seat retirements`, `delivery inspect`, `delivery recipients`, `overdue`, `diagnostics`, `pending-ops`, `view`) takes `--limit`, `--max-bytes` and `--cursor`. `body MESSAGE` returns bounded chunks with `--offset` and `--max-bytes`.

- Each page reports `has_more`, `stop_reason` (`complete`, `rows`, `bytes` or `work`) and `next_argv`. **Run the exact returned `next_argv`**; do not build a cursor yourself.
- A `search` page may contain no matches and still advance; keep following it until it completes.
- A filtered or mutable view can return `cursor_stale`; restart with the returned restart command.
- `read THREAD --recent N` takes the newest N as the first descending page; its continuation walks older history. `--recent` cannot be combined with `--limit`, and a cursor cannot be combined with `--recent`, `--after` or `--before`.
- `read THREAD --follow` is the live view: the recent tail (`--recent N`, default 20), then a poll of `read --after LAST_SEQUENCE` about every second (backing off to 4 s while idle), following each page continuation, so every committed message is printed exactly once. It does not take `--limit`, `--cursor` or `--before`. It reconnects after a daemon restart (it re-reads the published endpoint with backoff up to 5 s), keeps following an archived thread, stops with status 0 when the thread disappears or on Ctrl-C, and never ACKs or accepts. For nicks it reads each author seat's mapping (`seat inspect`), one Herdr pane-name snapshot (cached for 15 s) and the seat's existing private binding context for the harness, cached for a minute; clipped previews are completed with one bounded `body` read.
- An error for a pending or degraded read, or a too-small byte budget, is not an empty result.
- `thread list` without `--seat`, `--joined`, `--invited` or `--all` needs a caller seat. Observed on the earlier draft: outside a pane it exits 2; `thread list --all` works without one.

## Unknown mutation outcome and the local journal

Each mutation gets a durable operation key. The CLI records the intent in a private local journal (`STATE_DIR/instances/<digest>/intents/`) before submitting it.

1. If the response is lost, the command exits 5 (`unknown_outcome`).
2. Run `pending-ops` to list unresolved intents and their `local:` references.
3. Run `retry LOCAL_REF` from the same pane, or with the same `--cooperative-*` flags. It resubmits the original key and payload. A retry for another seat's intent is refused.
4. Confirm the result with `read`, `delivery inspect MESSAGE` and `pending-receipts`.

The local reference is not a message ID and cannot be ACKed. The same key with a different payload is rejected (`operation_payload_mismatch`). To send a deliberate second copy, run a new `send`.

The hook's tool-boundary check-in is non-durable: it writes nothing to the local journals and never replays. The lifecycle (SessionStart) check-in is durable and replayed exactly if its response is lost. A pending lifecycle request that the service definitively rejects is recorded as terminal and cleared, so the next SessionStart registers against the current generation. A resumed session's reattachment is decided in one daemon transaction that also opens the new binding; if its reply is lost the seat is already bound and the next SessionStart registers normally.

## Deadlines, decision time and wake spacing

- **Invitation deadline** starts at the invitation's transaction decision and ends only at explicit acceptance. Launching, binding, reading and ACKing messages do not settle it. Repeating an invitation while one is pending reuses it and does not reset its timer.
- **Receipt deadline** for a required recipient starts at the send decision if an eligible top-level occupant is available. Otherwise the duration is frozen at send and the timer starts at the first eligible-availability decision (for a prelaunch seat, its first successful check-in). Once started, a timer never resets on restart, retry, leave, archive or replacement.
- **Decision time** is one trusted UTC sample taken inside the successful deciding write transaction, after queue and lock waits. It classifies lateness. A pause after it, including COMMIT or fsync delay, does not reclassify the decision. A failed or crashed commit creates nothing.
- Each missed deadline produces one durable warning. A late ACK or acceptance still counts and settles the obligation without erasing the warning. Time alone never deletes anything or removes a member.
- Persisted UTC governs classification: a forward clock jump can make work due, a backward jump delays it.
- **Wake spacing** is separate: a process-local monotonic delay (minimum 30 s). After a restart a prior reservation waits the full delay again; downtime does not count.
- A prompt submission is only a terminal write, never proof that a model received anything. An offline seat keeps its obligations. A blocked UI or an active turn can delay observation; no ACK is fabricated.

## Seats, leaving, retirement

- `leave THREAD` stops future fanout for that thread only. Earlier pending obligations stay. A left seat can be invited again as a new membership episode.
- The last joined seat leaving, or being retired, leaves the thread **orphaned**, not deleted.
- **Retirement** happens when Herdr verifiably closes the seat's actual pane. A durable terminal fence and frozen cutover take effect immediately across all threads, and a reused pane label does not revive the seat. Process exit is only "occupant unavailable", not retirement. Host unavailability, denied access, event loss or unexplained identity loss alone do not prove closure; they lead to reconfirmation or an unresolved seat (above).
- Rows, owed warnings and per-thread audits for a retirement materialize in bounded background cleanup that resumes after a restart. Effective membership and orphan recovery apply before cleanup finishes. `seat inspect SEAT` shows cleanup as pending, complete or error; views can be temporarily pending or degraded meanwhile. There is no history or backlog cap.
- Retirement warnings record conditions that were already settled at cutover, so they create no new wake.

## A person's own pane (`me init`)

`herdr-threads me init`, run in a shell pane, makes that pane's seat the person's identity: harness `human`, provenance `operator_human` on the binding, its availability and every accountable decision (send, invite, accept, ACK). It is the person's ordinary seat, not operator repair, and has none of the operator forms' powers. Inspect it with `seat inspect SEAT`; in SQLite the current `occupant_bindings` row has `harness='human'`. A person's seat is never sent a wake prompt (a wake goes only to an agent of the seat's bound harness, so a `human` binding has none); required receipts addressed to it wait for a manual `ack` and go overdue like any other. After a daemon restart, re-run `me init` in the pane to mark it available again. If `me init` is refused (agent environment markers `CLAUDECODE`, `CODEX_SANDBOX` or `CODEX_SANDBOX_NETWORK_DISABLED`, a Herdr-reported Claude or Codex agent in the pane, or a seat bound to an agent), use your own shell pane, give this pane a fresh seat with `seat resolve --pane PANE --new-seat --operator`, or override as the local account with `me init --operator` (later commands in that pane then run as you).

`--pane` accepts a Herdr pane ID or a unique pane/tab label; `herdr pane current` inside a pane prints its ID when a name is ambiguous or unknown.

## Ambiguous restore or move, and operator repair

A restored or moved pane whose identity is ambiguous is **held** for inspection. The plugin does not silently hand the prior role to it. Inspect with `seat list`, `seat inspect SEAT`, `diagnostics`, `daemon health` (unresolved seats) and `view --once`, then choose deliberately among the three operator actions:

```text
herdr-threads seat rebind SEAT --pane PANE_ADDRESS --operator
herdr-threads seat resolve --pane PANE_ADDRESS --new-seat --operator
herdr-threads invite THREAD --seat SEAT --operator
```

- `seat rebind` moves an existing seat to a new pane; `seat resolve --new-seat` gives a pane a deliberately fresh seat.
- `invite --operator` works only for a thread with zero joined seats (orphaned). It creates or reuses an ordinary pending invitation, does not join the seat, reopen an archive or revive a retired seat, and loses with `thread_not_orphaned` to a concurrent acceptance.
- Authority is the kernel peer UID matching the daemon owner: a local account, not proof of a present human.
- Operator mode cannot ACK, accept, send or check in, and cannot be combined with those commands. A person who needs to send or ACK uses their own pane identity (`me init`, above).
- Plain `seat resolve --pane PANE_ADDRESS` (no `--operator`) allocates an unclaimed empty pane, for example before launching its recipient.
- The hook never allocates a seat. At SessionStart in a pane whose seat is unresolved it prints `herdr-threads: check-in unavailable (pane seat mapping is Unresolved) …` and the diagnose argv; on tool calls it prints nothing.

## Restore holds and repair

Restore holds follow the [trust policy](../TRUST-POLICY.md) (C1 to C3, F6 resolved 2026-10-01). After a Herdr restart the daemon holds every unowned target in the restored baseline, and ordinary resolution (`seat resolve`, `launch`, `me init`) refuses it with inspect, rebind and fresh-seat guidance.

**When a hold lifts.**

- Per target: an operator `seat rebind` or `seat resolve --new-seat` on that target, or cooperative continuity on it (below).
- Instance-wide: automatically once no unresolved nonretired seat remains and the restored baseline is reconciled. Nothing is left to protect, so panes created before the restart but never claimed become ordinary again. No command is needed.

**Retire a seat.** `herdr-threads seat retire SEAT --operator` abandons the seat. It retires and its pending obligations settle as recipient-retired. Seats are never merged.

**Rebind over a collision.** `seat rebind OLD --pane PANE --operator` refuses with `target_already_owned` when PANE is owned by another live seat NEW. The refusal names exactly two resolutions as ready argv:

```text
herdr-threads seat retire OLD --operator
herdr-threads seat rebind OLD --pane PANE --replace NEW --operator
```

The first abandons the old seat. The second abandons the new role: it retires NEW and rebinds OLD to PANE in one decision, so nobody can claim the target in between. NEW's pending obligations settle as recipient-retired; nothing moves from NEW to OLD.

**Cooperative continuity.** A resumed top-level agent session (SessionStart source `resume`) whose harness session id uniquely matches an unresolved seat's last binding reattaches that seat by itself, with no operator step. It never applies to `startup`, `/clear` or `/new`, which carry a new id, and it never merges seats. Claude does this with `claude --resume`. Codex does it when you run `codex resume` by hand in the pane. Managed `launch` of the Codex `resume` form stays refused (no captured hook evidence), so reattachment is by running it yourself in the pane. If the id matches no unresolved seat, or matches more than one, nothing happens and the seat waits for an operator choice. A resume that arrives before the daemon has reconciled the restored Herdr is retried within the hook; if the hook gives up, the pane stays held and the next `resume` in the pane (or `herdr-threads retry`) finishes it. Evidence: [cooperative continuity](compatibility/cooperative-continuity-resume.md).

**Reading the repair history.** `seat inspect SEAT` shows how the seat's latest repair was decided:

| Row kind | Meaning |
|---|---|
| `operator_rebind` | `seat rebind` (including `--replace`), decided as `operator:local-user:<uid>` |
| `operator_retire` | `seat retire`, or the retire half of `--replace` |
| `cooperative_continuity` | reattached by a resumed session. The row carries a Herdr `agent_session` diagnostic: `match`, `mismatch`, `absent` or `read_error`. It is a diagnostic only and never decides anything |
| `operator_human_override` | a person's `me init --operator` replaced an agent binding |

**Daemon restart.** A request carries the daemon boot the client addressed. If the daemon restarted in between, it refuses before dispatch with `daemon_boot_changed` (`DaemonBootChanged`, exit 3, transient) and nothing was applied. Retry the command; the client addresses the daemon's current boot on the next attempt. A joined seat whose pane mapping is structurally reconfirmed after the restart stays available: its open binding carries forward to the new host epoch with no new check-in. The carry lands in the first reconciliation pass, not at `daemon ensure`; a send in between does not wait for it. A recipient whose binding is about to be carried (same Herdr boot and incarnation, a cooperative or operator binding) is staged as not yet available but draws no `recipient_unavailable` warning, and its receipt timer starts when the carry lands, exactly as for a fresh check-in. If the pass does not carry the seat, the ordinary unavailable paths open the outage and later sends warn as usual. A native `verified_current_target` binding is never carried; it re-registers. Availability still ends when the mapping becomes unresolved, the seat retires, or a check-in replaces the binding.

## Native replacement race

Herdr has no atomic "check native session and commit" operation. A replacement of the agent that happens after the plugin's fresh observation but before the SQLite commit can go unseen. The record keeps the observed caller and decision and is never reattributed to the successor; no physical-commit atomicity is claimed.

## Programmatic service connections

`service inspect` shows the registered programmatic client. `service disconnect --expected-boot BOOT_UUID --expected-generation N`, with the exact values just observed, recovers a connected but unresponsive service; a stale request cannot disconnect a successor. These are same-user cooperative operator commands. Service `info` notices remain discoverable without waking an agent; `warn` notices use native attention and settle as described under [notice settlement](#attention-digest-and-notices).
