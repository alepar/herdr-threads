**Whole-epic final review at 8e7eaa41, which contains main 88f5c69f.** Independent opus reviewer, read-only.

**Verdict: not ready.** There is one interaction defect with main's warning-wake refinement. Everything else checked at HEAD holds.

**Must fix**

1. **A live mod channel still gives attention prompts to members who owe nothing when another seat's overdue transition opens or clears.**
   - Main's 251b7dbd narrows native wake attention with `warning_wakes_seat` (src/store/attention.rs:1052-1066). The mod path does not:
     - `mod_seat_view` takes `other_pending` from the full digest warnings count (src/store/seats.rs ~4370).
     - `watch::drain` treats every `InboxBatchV2Item::Warning` as attention (src/cli/watch.rs:549-551).
   - So a bystander seat gets `attention:<v>` and then `attention:<v+1>`. The model of `tests/integration/notice_wake.rs::other_seat_overdue_transitions_never_wake_members_who_owe_nothing` shows this, where the author holds a Warning item.
   - Native wakes are correctly suppressed while a channel is live, because `mod_suppressed` in src/scheduler/mod.rs is per seat and does not depend on the candidate.
   - Fix: narrow both the fingerprint and the drain with `warning_wakes_seat`, and add a bystander integration case. Filed as ht-j16.34.

**Tests the reviewer ran**
- Focused nextest (`notice_wake`, `mod_routing`, `mod_digest_flag`, `notice_published_while_live`, `stale_attention_is_retracted`, `mod_channels::`, `mod_ack`): 75/75.
- Mod JS: 84/84.

**Untested scope**
- No test covered another seat's transition notice reaching a seat with a live mod channel.
- Live stress has not been re-run on the merged-main code.

**Deferred OK**
- ht-182, ht-22y, ht-oag.
- The roast-pr-2 punch list.
- The parked escalations.
- The accepted windows in the Post-Implementation Notes.
