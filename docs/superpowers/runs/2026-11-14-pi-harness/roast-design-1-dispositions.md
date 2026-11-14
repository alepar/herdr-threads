# Design review 1 dispositions

The frozen first design received four Blocking and five Should-fix findings. All nine are addressed in the revised design; this is the author's disposition, pending review 2, not a claim of reviewer clearance.

| Finding | Revision |
|---|---|
| Role admission | D2 names the bundled extension as cooperative role producer, captures inherited ownership before writes, refuses explicit/unknown children, requires an explicit stamped top-level role, and documents marker-stripping interactive-child indistinguishability. |
| Native queue survival | D3/D5 eliminate all message bodies from native nextTurn/steer/follow-up queues. Bodies stay locally owned until persistent boundary drafts; reload reconciles accepted unacknowledged entries against the current branch. |
| Aggregate capacity | D4 specifies 256 records/2 MiB, priority-based displacement without settlement, independent wake-control state, and one replay/refill after progress when displacement occurred. Pressure qualification is mandatory. |
| Failed wake observation | D5 adds a 30-second unreceived transition and suspends automatic retries for unchanged attention; later authentic contact/new attention/replacement must requalify. This explicitly accepts bounded degraded progress. |
| First idle wake | D3 initializes idle from successful lifecycle admission plus fresh isIdle at startup/reload, with no preceding settled-event requirement. |
| Lifecycle guidance/provenance | D3 makes mail watch-only and proposes one neutral output-policy setting: lifecycle registration emits guidance/routing as ObserverOnly. Native boundary insertion carries guidance; it grants no receipt. |
| Shutdown ownership | D3 withdraws only still-owned env values, fences async work, stops/reaps watch/timers, and reports existing session-end liveness disconnection without retiring a seat. |
| Same-session tree | D3 adds branch revision, current-branch reconciliation and guidance restoration; prior valid receipts are not reversed. |
| Cross-batch ordering | D4 sorts the retained pending union at admission, and D5 uses one sorted boundary page. No passive native FIFO body queue exists. |

The revision uses a newly investigated API already present in the captured upstream docs: `before_agent_start` persistent messages and `turn_end`/`agent_before_settle` boundary drafts. The credential-free real-SDK follow-up asserts nine cases and confirms completed boundary continuation within one outer run. It also exposes non-completed pre-settle behavior differing from the documentation, so D5 explicitly guards `outcome === completed` and never treats a returned draft as receipt evidence. See `research/candidates/probe/results.md` and `boundary-trace.jsonl`.

Review 1's report remains authoritative for its frozen input. Its wrapper-separator format check failed and the skill's single format repair was rejected; findings and panel evidence remain intact. That limitation is preserved in its friction artifact rather than edited out of the old report.
