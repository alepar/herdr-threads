super-roast verdict: clean (2 nits) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal release tooling. The wake e2e driver is a local, pre-release gate run by the releasing agent; a defect mislabels or delays a qualification row, it never reaches users or real data.
inputs: bead/ht-b8jm@7ddbc1b33 vs main@6e918785b
coverage: unavailable: manual run, no coverage object. One fresh correctness/test scout over the diff; findings verified inline against the code and the self-test
independence: partial (subagent scouts, inline verification) · rung: manual fan-out

## Confirmed findings
- [Should-fix] scripts/wake-e2e.py codex_throttle: `feedback_log_body LIKE '%429%'` also matched a non-429 retry whose hex thread or turn id, or a jittered delay, held `429` by chance. Such a retry would widen the wait and mislabel a real failure as "provider throttled".
  verdict: confirmed (inline: real bodies carry full hex UUIDs in their span prefix)
  evidence: the release-2.5.0 retry rows and the live-log sample show `thread_id=01a10f34-...` span prefixes; the status text is `last status: 429 Too Many Requests`
  fix-shape hint: match `%status: 429%`; the fixture's non-429 retry now carries `4291` in its turn id. Fixed in the follow-up commit.

## Unverified nits (spot-checked)
- [FYI] A throttled-then-entered row PASSed with no trace of the throttle. Fixed: `wait_entered` returns the cause, and busy/eot rows record `checks.provider_throttled`.
- [Nit] test_throttled_wait_runs_on_until_the_tool_enters skipped `late.join()` if `wait_entered` raised. Fixed with try/finally.
- [Nit] `{delay:.0f}s` prints a sub-second 429 backoff as `0s`. Cosmetic. Kept.
- [Nit] The log is probed once, at 60 s. Codex logs the retry before it sleeps, so a 429 is visible by then. A later batched write is unverified against Codex. Accepted.

## Rejected (with reason)
- Clock skew between `since` and Codex `ts`: same host clock; `int(since)` floors and at most admits rows from 1 s before the prompt.
- Read-only open of a live WAL db: verified locally with an open writer mid-transaction; any sqlite3.Error yields the plain failure.
- Watchdog interaction: the held tool's 240 s watchdog counts from the tool's own start and later turn waits from publication, so a 180 s entry wait shortens neither.
- Two-dot diff shows the v2.6.0 prep reverted: the branch base predates 398f6da16; the three-dot diff touches only the two scripts.

## Escalations (need human)
none
