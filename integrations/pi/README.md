# Pi native extension

`extension/herdr-threads.js` is a dependency-free native pi extension. Its default
export registers callbacks; the named exports support focused qualification. Pi's
recorded native loader imports the default factory, so helper exports are inert.
Setup owns the extension directory and replaces the marked `LAUNCH` constant with
its CLI argv prefix. Without that replacement, the executable is `herdr-threads`
from PATH. The Rust adapter/setup task is required before this is an integrated
installation; these assets alone do not establish daemon admission.

Only an explicit native `tui` context enrolls. The extension captures inherited pi
ownership markers before writing its own session, top-level role, mode and PID.
An inherited foreign/malformed PID, child/unknown role, nonterminal mode, or session
without same-process ownership prevents enrollment. Other harnesses' variables
are not role evidence. One process owner prevents duplicate global/project loads.

Lifecycle commands stage at most 4096 bytes of guidance, separately from mail.
Only successful admission starts `watch --harness pi --session SESSION`. The CLI
negotiates presentation leases and confirms nonordinary output after flushing it;
the extension neither duplicates those confirmations nor accepts invitations.
Watch loss reconnects with 1, 2, 5, 10 and then 30 second backoff. A terminal watch
refusal waits for a later eligible native callback to retry lifecycle admission.

Mail stays in a local buffer with a combined limit of 256 pending/proposed records
and 2 MiB of UTF-8 rendered text, including proposed frames and staged guidance.
Metadata and JavaScript allocation overhead are additional. Every admission sorts
the whole retained union by priority, deadline with none last, then stable first
observation. WatchLine has no creation timestamp: exact age across reconnects is
not reconstructed. Better arrivals can displace worse pending records; proposed
records cannot be displaced. Every dropped incoming or displaced record creates
replay debt. Capacity release replaces the watch to request canonical pending
replay, independently of exhausted logical reoffers. Failures keep that debt and
use bounded reconnect backoff. Saturation itself does not loop reconnects.

Bodies enter only native `before_agent_start` or completed `turn_end` /
`agent_before_settle` draft results. Each page has at most 32 items / 64 KiB. Other
extensions' entries and continuation are preserved. Each retained item gets at
most one proposal per outer run; rejected drafts wait for another run. No body is
put into pi's steer, follow-up or next-turn queues.

A receipt requires an exact custom-type nonce and full unchanged frame both in
the finalized current native branch and in a later model-bound `context` callback
for the admitted session. Only then does `watch ack --via context` name complete
ordinary/lazy IDs. Truncated mail retains its existing explicit-inbox continuation
and never ACKs. Guidance, invitations, warnings and attention are never mail ACKs.
Failed ACKs retry only exact previously evidenced IDs, fenced by owner and branch.
Tree/compaction callbacks invalidate old context evidence; reload reconciles only
this integration's bounded known unacknowledged proposals. A void API return,
preview, message-end event or accepted entry alone supplies no receipt evidence.
Reload retains accepted frames but clears their old owner's waking witnesses.
A current canonical urgent replay can then wake the successor once, without a
duplicate body proposal or an ACK before successor context observation.

An admitted waking item can submit only `herdr-threads: wake up`. The final owner,
channel, fresh idle, UI span and editor checks run synchronously with submission.
Nonempty drafts block wake without being edited; missing/unreadable editor APIs
disable wake while boundary delivery remains available. Wakes never select a
model or provider. A single flight times out after 30 seconds without native
progress and suspends unchanged attention. Pending passive mail starts no run.
Rejected/altered drafts, error/aborted boundaries and attempted mail left without
context at settlement also block unchanged attention from restarting itself. A
later user run, new attention or channel replacement can retry; receipts stay pending.
Already known urgent keys share their attention fence, including alternating
reoffers of different retained IDs. A bounded cache remembers up to 256 waking
keys in addition to the witnesses on retained records. It never evicts a key to
infer new attention. If rejection or no-progress suspension reaches that history
bound, ambiguous identities cannot start automatic runs until an authentic user
run, new owner or successful channel transition resets the fence. This degrades
only wake; boundary delivery and canonical pending mail remain available. Repeated
connected status on the same live channel is not such a transition.
Wake is rechecked on new watch attention/connection, final settlement and extension
UI prompt completion. There is no editor-change polling; merely deleting a draft
may leave attention pending until one of those events or a manual turn.

Shutdown fences old callbacks first, cancels retries, aborts owned command children
and stops/reaps the watch. Its best-effort old-owner SessionEnd reports liveness;
it never retires a seat.
Detached watches remain owned until their replacement/stop promises settle;
shutdown and reload join those promises before releasing the process owner or
admitting a successor.
Environment cleanup compares exact old values so a
successor's markers survive. Every child has bounded output; one-shot commands
have a deadline and owned processes receive TERM then KILL if necessary.

## Focused qualification

Use the repository's heavy/toolbox wrappers:

```sh
/var/home/alepar/.cache/herdr-threads-heavy/heavy /var/home/alepar/.cache/herdr-threads-heavy/tb node --test integrations/pi/extension/tests/extension.test.mjs
```

The real-SDK fixture uses the existing locked research probe dependencies, or an
explicit `HT_PI_SDK_MODULES` pointing at that `node_modules` directory. Install the
research lockfile with `npm ci --ignore-scripts` through the same wrapper, then:

```sh
/var/home/alepar/.cache/herdr-threads-heavy/heavy /var/home/alepar/.cache/herdr-threads-heavy/tb node --test integrations/pi/extension/tests/sdk.test.mjs
```

It uses private HOME/agent/session directories, a scripted provider and controlled
CLI transport. It runs the real SDK callbacks, persistence, context and native
continuation; it makes no authenticated provider request. Its explicit native
shutdown event exercises cleanup; SDK `dispose()` alone does not emit that event.

Real TUI draft/modal behavior remains a qualification gate. Extension UI spans
are not a universal built-in modal detector. This implementation does not attest
disk flush, provider receipt or model consumption; a later extension may transform
context after observation, and downstream transport may fail. Real tools,
credentialed providers, native reload/resume/fork and integrated daemon replay
need separate real TUI/integrated qualification. The fixture does exercise actual
SDK `reload()` and confirms predecessor shutdown and successor admission. Native
branch inspection uses pi's
read-only current-branch API, not a transcript-wide history search.
