# Qualification timing and cancellation correction

The second integrated native attempt on `ae6626ff8` recorded eight passing rows
(setup, safety, threshold, busy, receipts, ordering, invitations and negative),
then stopped at an invalid cancellation assertion. Its full failed result and
clean owned-resource cleanup remain preserved privately. It is not a passing
14-case run. Earlier multiline/passive negative observations that could be masked
by the previous native wake's cooldown receive no qualification credit.

The qualification driver now waits for eligibility using the tested session's
latest actual native wake start before multiline, threshold passive, named
passive and live passive sends. It requires a witnessed native session and records
the calculated deadline and observation time. A session with no prior wake does
not acquire an invented delay. Deterministic clock regressions failed before the
correction and pass afterward, including a named-session versus foreign-session
case. Pre-send run baselines, fresh editor observations, exact styled captures
and selector/model preservation remain required.

The old cancellation scenario delivered mail at a legitimate completed turn
boundary before aborting a later settle boundary: context at `1794727144676`,
before-settle at `1794727144679`, aborted settlement at `1794727144700`. Its
whole-run absence assertion incorrectly rejected that completed delivery.
The corrected fixture installs a barrier after production has seen an empty
completed-turn proposal. Only then does the driver send a unique requested-ACK
item. Such requests are p0, so the driver holds an owned unsent draft while busy
to prevent a legitimate Native wake after abort. It requires production's settle
proposal to contain that exact ID/body, aborts through the public API, checks no
context/provider request contained that target and requires its receipt to remain
pending. The same draft is then submitted directly for recovery and exact receipt.
There is no empty-idle interval between draft protection and user recovery.

The actual fixture-order regression failed without the barrier and passes with
it. All 14 driver self-tests pass in 1.323 s. Independent review approved frozen
patch SHA-256 `485406dcebb5704293c860e2450a09c2930bd8f991e7283e9c87190116b9cc37`.
Only the qualification driver and fixture changed; production stayed unchanged.

| Supplemental native case | Result | Seconds |
| --- | --- | ---: |
| safety | PASS | 182.537 |
| threshold | PASS | 101.877 |
| failures | PASS | 4.436 |
| named/tmux | PASS | 66.120 |

The supplemental safety/threshold/named observations replace the masked negative
controls. The named case exercises the integrated startup-readiness core through
an actual verified private tmux process: connected startup binding, p0 context
receipt, passive p1 next-turn delivery, and busy-tool p0 delivery with no trailing
wake over 31 seconds. It remains distinct from Herdr native qualification.
Cancellation retained 16 outer starts before and after abort, equal exact styled
draft rows, separate fresh observation nonces, zero target-bearing context/provider
requests, and pending then acknowledged receipt after direct user recovery.
Cleanup verified helper exit 0 and reaping, removed scratch root, process/temp
scan status 0 and no cleanup errors.

[Selected machine-readable evidence](task-3-timing-settle-evidence.json) records
source/driver/fixture/candidate hashes, actual cooldown eligibility, cancellation
proof and cleanup. Full private manifests/captures retain the failed attempts and
native traces. Lifecycle, compaction, handoff, channel fallback, live and full-suite
results are separate final evidence. External custom overlays and spaces-only
cursor-home drafts remain the explicitly accepted limits; neither receives a
preservation claim from these tests.
