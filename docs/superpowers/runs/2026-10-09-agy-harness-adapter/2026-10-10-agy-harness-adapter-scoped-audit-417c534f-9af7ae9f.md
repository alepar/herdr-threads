# Scoped post-cap audit — 417c534f..9af7ae9f

Two independent Opus seats (delivery correctness; store concurrency + test quality), read-only, over the
post-roast fix pass (ht-poh.53–58, SKILL N2 wording). Focused tests: 311/311 lib + 46/46 nextest.

verdict: clean — 0 Blocking, 0 Should-fix.

Minor, filed as follow-ups (outside the epic):
- ht-7jh — notices settled on pull are lost if a probation/systemMessage injection silently misses
  (new trigger of the settle-before-delivery limit, reviews 9 N5 / 10 B).
- ht-10p — notice blocks repeat fixed text per block, `notice` kind undocumented in SKILL, text not
  inlined (token-efficiency a/b, delivery rule 3; parity with the check-in path, not a regression).

Minor, accepted without a bead: refused-settle path untested at service level (store-level covered);
stale-snapshot trigger of N6 (attention block re-shown); journal byte-pin test cannot catch a revert of
the single write; `pull_notice_page` sub-queries map errors via `store_error`; SteppingClock seed budget
(~500 clock reads) is a deterministic tripwire for future store changes.

Verified correct: pull/check-in settle races (prefix check in IMMEDIATE txn), no `Unchanged` hiding a
pending notice, `notices_pending` never sticks, ht-poh.57 keeps notices out of the wake gate, mod channel
never sees notices, journal/handoff writes byte-identical with unchanged fsync/rename order, fixture
staging loops bounded, 15 s WAIT restored.
