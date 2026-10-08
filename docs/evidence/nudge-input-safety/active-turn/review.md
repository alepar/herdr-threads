# Active-turn follow-up review

/root/input_safety_review, read-only; no tests, edits or thread writes by reviewer.

No production blocker under the sampled Herdr-status contract. Dispatcher rejects ActiveTurn regardless of historical declarations before submitting. Both native entry points independently reject working. Ordinary/coalesced refusals preserve prior ladder; PokeOnly skips leave receipts unmarked.

Cached idle/done inference cannot guarantee physical turn completion or establish the report cause. Describe deferral as applying to reported active turns. Two stale port comments corrected to historical compatibility wording.

Final review of broadened poke_flow migrations: no blockers. SQLite receipts remain unpoked during active turns and drafts with historical capabilities. Active-turn test reuses the same scheduler/receipt, waits retry spacing and verifies later idle delivery. Updated port comments match policy. Review results are independent of worker test execution.
