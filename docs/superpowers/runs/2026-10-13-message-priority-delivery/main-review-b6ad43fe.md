# Main independent review: super-auto/message-priority-delivery @ b6ad43fe (composed on main a0f93109 as dee56dba)

Four read-only reviewers, one per area. No Critical findings. Composed-source checks clean (fmt, harness-neutral, clippy -D, default-features, workflows/docs tests, wake-e2e --self-test). For each Important item: reproduce it (failing test first) or show it cannot happen, then fix. Minor items: fix or answer.

## Important

I1. Hook header contradicts the ACK instruction (src/cli/hook_inbox.rs:45 vs :214). INBOX_HEADER now says "messages shown in full..." (was "lazy messages ... no receipt or reply obligation"), so it covers every message, while the same block can say ACK-required messages are not ACKed yet. A model may skip the ACK on a --require-ack or handoff message, producing an overdue warning. This text is injected on every non-quiet hook. Qualify it, e.g. "messages without an ACK request".

I2. Old invitation and handoff flags hard-fail with a bare clap error (src/cli/commands.rs:1170 InviteArgs, :1406 HandoffArgs). `invite --lazy`, `invite --require-ack` and `handoff --require-invite-ack` were removed outright, while `send --lazy` stays hidden and returns a clear removal message (:1657). Agents with the previous SKILL.md loaded fail right after the upgrade. Accept them as hidden no-ops (invitations always confirm now) or give the same explicit message.

I3. CHANGELOG (CHANGELOG.md:5) leaves out breaking behaviour changes:
- `--nudge` used to create receipts, deadlines and overdue warnings for every joined agent; now it creates only p0 rows, so senders relying on overdue warnings lose them silently;
- `--nudge --deadline N` is now refused (validate_delivery_options);
- every new invitation is confirming (deadline, wakes, overdue warning); a requested Lazy confirmation is ignored, and an explicit human RequireAck is now accepted;
- the v47 store upgrade is one-way: v0.11 refuses the store afterwards, so a downgrade loses the state dir;
- also worth release-note lines: `wake-on` output shape change (src/cli/wake_on.rs:39) and ServiceMessageSent.recipient_count can now be 0 ("no recipients" error text).

I4. A sibling binding's display can block a process wake, with an endless retry (src/store/wake.rs:679-694, src/notification/dispatch.rs:~400-415, src/ports.rs:2182).
- With `lazy` now a seat-attention reason, a wake to a one-process group (ht-327) reaches every binding of the process, and dispatch refuses unless owes_wake holds for ALL of them.
- LAZY_FRONTIER_UNREAD_SQL checks only the given binding, but delivery rows are per-binding streams.
- Scenario: /side binding X has displayed p0 frontier row R; the idle main binding Y has not. The frontier stays at R (Y's view), owes_wake(X) is false, so the wake is Refused(Unavailable), the frontier is restored, and it retries on refusal backoff indefinitely. Y's p0 message never wakes Y, and every retry costs a host foreground read.
- This contradicts the SQL's own doc comment and Delivery contract rules 4/6.
- Suggested fix: `lazy` owedness = ANY reached binding still has the frontier unread. Please cover it in the wake e2e side cases as well (side-nudge/group).

I5. Inbox: a partly displayed head body of a newly collected deadline window skips the rest of that window (src/store/queries/inbox_v2/mod.rs:449-451 next=before + pin_partial; order.rs:510; deadline_window.rs:339 re-collect, :278 retain(key > last)).
- The re-collect assumes everything <= `last` was shown, but `last` can come from an earlier window W1 whose keys sort above W2's: windows are filled in walk-ordinal order, not key order.
- Scenario: more than 64 pre-upgrade unread rows. W1 ends with (p0, None, ds=500); W2 holds pending-receipt rows (p0, Some(D), ...) that sort below `last`. W2's head body overflows the page, so a chunk is emitted.
- On the next page the head resumes, `retain` drops every other deadline item of W2, and persist moves past them. They are never shown in this traversal, and the page reports Complete.
- Likely right after the upgrade.

I6. Inbox: deadline-class keys can change between pages, so a shown item is shown again (deadline_window.rs:200 builds the Unread key deadline from a Pending receipt only; :278 assumes stable keys).
- Scenario: explicit-ACK M (deadline D) is shown and display-ACKed on page 1, but its thread cursor can't pass M because a lower-seq row in that thread hasn't been shown yet. That is common now that p0 sorts ahead of an older p5 row.
- Its re-collected key becomes (p0, None, ...) > last, so page 2 shows M again, without an ACK candidate.
- Related: mark_displayed_above (read_cursors.rs:488) only marks rows that have a lazy_recipients entry, so receipt messages shown above a blocked cursor are offered again on fresh inbox calls until the cursor catches up.
- Suggested direction for I5 and I6: store the window-start key (or one per source) in the cursor, and freeze each item's deadline when the window is captured.

## Minor
- M1 invitation.rs:111: stored pre-upgrade lazy invitations keep wakes:false, but SKILL.md:23 says invitations always wake. Note the exception or treat old lazy invitations as p0.
- M2 output.rs:403: a `[p0]` marker row on every pre-upgrade ordinary message in history/search/read/follow, and ` [p0]` on ACK rows (duplicates the ack line). That is model-visible growth; state its size (token rule f) or drop the redundant cases.
- M3 integrations/claude/mod/README.md:50 calls `lazy` legacy, but the daemon still emits DeliveryKind::Lazy (assemble.rs:353); :123 says acks are for "message items only", but register.js acks lazy too.
- M4 invitation.rs:151: FrozenInvitation::watch drops deadline_at, so replayed invitations sort differently than live ones.
- M5 commands.rs:1975: a human handoff sets a dead InvitationConfirmation::Lazy; remove it or document why.
- M6 TRUST-POLICY: the A8 disconnected-binding limit and the D7 pair auto-join consent are documented inline but not added to the "Accepted limits" list or the Decision record.
- M7 messages.rs:~313/~718: a v0.11 send interrupted mid-build and resumed after the upgrade can end with a mixed audience (some receipts, some p0 rows).
- M8 0047: lazy_recipient_source does not check NEW.priority = the preparation's priority (only stage_recipient guarantees it); message_priority_immutable is redundant with messages_immutable.
- M9 ports.rs:2182 / dispatch.rs:268: lazy-only multi-target wakes now skip the idle gate; only the adapter's final composer read prevents a busy nudge. Note it in the delivery audit.
- M10 0047:94-101: view rows for invited/declined ordinary threads are never unread and pile up, so wake/inbox seeks skip them (a cost against the O(1) goal).
- M11 lazy_delivery/mod.rs:182: reconnect re-seed is O(thread history) for threads without a cursor.
- M12 subagent binding views grow per publication until disconnect, and no wake path reads them.
- M13 deadline_window.rs:~197/~247: Unread and Receipt share SourceRank::Receipt but use different ordinal sequences, so equal keys can make retain drop an item (rare).
- M14 deadline-class ordering is only correct within one window (needs a spec answer; it is the precondition for I5).

Verified clean, so you don't need to redo it: priority convention and every threshold comparison; kick-after-commit; threshold re-read at reserve; busy refusal; ACK-required always nudge-class; dead-session view cleanup; hook re-seed; migration transactionality and idempotence; v0.11 refusing v47; protocol-8 refusal; serde defaults; format-1 cursor error; mod ordering/dedup/busy guard; row pagination and cursor codec; page bounds; staging dedup.
