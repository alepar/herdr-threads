## Verdict: not ready

I reviewed the branch diff against the epic spec and the inbox-merge subspec, then triaged the ledger. I also ran a few focused tests at HEAD (99308304), listed below. The ledger's landed≠completed line (19 vs 16) means the ledger is incomplete, so I did not treat anything missing from it as clean.

**No full-suite run exists for this branch.** The caller runs the sweep after this review. Every merge in this run was gated by `cargo check` only, never by tests, so "review clean" says nothing about whether tests pass. The test failures below landed that way.

What I ran at HEAD:
- `cargo test --lib -- store::connection::tests store::effective`: 194 passed, 7 failed.
- `cargo test --test hook_entrypoint -- pair_accept_delivers_mixed_opening_mail_once`: failed.

## Step 1: my own findings (before reading the ledger)

**A. Important: the hook pull now reads the whole pending backlog on every pull.**
- Where: `src/delivery/page.rs` `drain_pages`, changed by ht-yha.9 and then ht-cbb (011c24d2), composing badly with `src/service/delivery_pull.rs`.
- What changed: `drain_pages` now holds back every item until the last page is read, sorts them, and only then hands them to the sink.
- Why it matters: delivery_pull's sink stops early when the 32-block or 64 KiB page budget is full (`PAGE_MAX_BLOCKS`, `PAGE_MAX_TEXT_BYTES`), or on the "full table, only due offers" path (`short`, the comment at delivery_pull.rs:482 "does not walk the backlog"). That early stop no longer saves any store reads.
- Effect: each hook pull with changed attention pages the entire pending set, up to `MAX_DRAIN_PAGES` = 4096 pages, under one `CallBudget`. A seat working off a large backlog 32 items per turn does O(n²) total work. Once one pull runs past its budget, the pull returns Busy and the hook delivers nothing.
- Why no test caught it: the guard test in tests/service/delivery_pull.rs (~3584) checks `probe.items_read`, which counts items the sink saw, not pages read. So it still passes while the bound is gone.
- The sort buys almost nothing: the inbox v2 merge already emits pages in `DeliveryOrderKey` order, apart from the accepted limit where a backlog over 64 deadline-class items is ordered only within each window.
- Side effect: items pushed into `assembler.emitted` are dropped if a later page errors. The ledger records this as a ht-yha.9 minor; I'm upgrading it to Important.
- Fix: emit in page order (or sort within a page) and keep the early stop working. Then make the probe count pages or rows read.

**B. Important, model-visible: stale guidance text.** `src/cli/handoff.rs:2448` `AUTOMATIC_PENDING_GUIDANCE` still says "Ensure creates the initial invitation once". After D7, new pairs auto-join and no invitation is created, so agents are told something false. Also flagged in the ledger as ht-yha.6 [stale-guidance-text].

**C. Minor, abstraction drift.**
- `src/store/lazy_delivery/wake.rs` `admitted()` re-implements the threshold rule (T < 0 means every priority, else 0..=T) outside `NudgeThreshold`.
- AGENTS.md says `wake_frontier` runs over `seats::THRESHOLD_ADMITS_SQL`, but only `note_published` uses that fragment.
- The test-only `commands::dispatch` / `MutationSpec::into_command` path always uses the legacy send shape, so `-p 2..9` silently becomes lazy even against a new daemon. Production goes through `run_cooperative`, which checks the daemon's capability.

**D. Minor, edge cases.**
- `first_cursor_sql!` in `read_cursors.rs` applies the join-time floor to every direct thread, including pre-upgrade pairs. A seat with no cursor in an old pair would be offered history since its join.
- `MARK_DISPLAYED_SQL` only counts connected bindings. An open but disconnected top-level binding of the same seat loses the row from the ordered seek (the index covers `state='pending'` only). This is the residue of ht-yha.4's [state-dependent-wake-seek] minor. The connected case was fixed in ht-yha.5.4.

**E. Minor, existing behaviour carried over.** The compact inbox renderer places `[pN]` after the first `" from "`. A thread topic containing " from " puts the marker in the wrong place. The old `[lazy]` placement had the same flaw.

Spec coverage otherwise looks complete: the migration backfills and triggers, the D3/D4 flag matrix, D5 alias, D9 staging, wake frontier with one seek per admitted priority, display marks with inherit and prune, pair auto-join with a floor for an offline peer, confirming invitations, old-client mapping in both directions, and the `[pN]` framing in Rust and JS.

## Step 2: ledger triage

**Recurring cluster, doc-drift (8 instances across ht-yha.13, .8, .11, .10, .16).** The ledger counts ×3 across 3 tasks; I found 8. This is a pipeline defect: each task brief restricts the file list and no task owns cross-file stale text, so drift outside a task's list can never be fixed by that task. Still present at HEAD:
- AGENTS.md:112 and scripts/linux-e2e/README.md:190,192 still use the removed case `send-nudge/group`.
- integrations/claude/mod/README.md:123 still says acks apply to "messages and lazy rows only".
- 8 `wake_on: all` comments in `src` (ports.rs, assemble.rs, policy.rs and others).
- tests/store/binding_lazy.rs still names the old `lazy_delivery.rs` path.
- `AUTOMATIC_PENDING_GUIDANCE` (finding B), the only model-visible one.

**Second class, test failures recorded as "pre-existing".** Two tasks recorded known failing tests as pre-existing minors. Both still fail at HEAD:
1. The 7 upgrade tests (ht-yha.5.2 and ht-yha.3 minors). The ht-yha.3 entry says they fail identically at base 0a24b6c3, but shows no base-run output. They fail at HEAD: `absorption_main24_and25_to26…`, `published_v35_upgrade…` and `published_v36_upgrade…` ("published35 retained lazy_recipients"), plus four `effective::dormant_upgrade_*` / `receipt_send_time_upgrade_*` tests. That base, 0a24b6c3, already contains ht-yha.2, which added migration v47. The assertion compares `lazy_recipients` rows before and after upgrade, and v47 adds columns to that table, so this is very likely the epic's own regression. It is not proven pre-existing on `main`.
2. `tests/hook_entrypoint.rs` `pair_accept_delivers_mixed_opening_mail_once` (ht-yha.6 minor). After accept, `lazy-middle` is shown 0 times instead of the expected 1. Either the per-binding display marks (ht-yha.5.2) suppress a row the test expects to be offered again, or the test's expectation is stale. One of the two must be decided and fixed.

**Must decide (human).** ht-yha.3 [service-send-deviation]: `ServiceSend` keeps receipts for its whole audience (`AckAudience::Everyone`). That contradicts user-approved D9 ("receipts only for explicit --require-ack recipients"). It is disclosed, but needs either a user decision recorded as an accepted limit, or a conversion.

**Flake claims without base evidence** (ht-yha.14, .5.3, .7, .5.4): `lazy_markers::q5_*_public_refits` fails with ReadBudgetExhausted. Reruns pass, but it was never run at base. It belongs to the full-suite sweep together with the ht-zo4 flakiness work. It is untested, not cleared.

**Parked findings:** none. The one BLOCKED fix pass (ht-yha.5.3) was about a report that lives in a git-excluded file, a pipeline artifact; the task later landed with a clean review.

## Must fix before landing
1. Restore bounded hook-pull reads in `drain_pages`, and fix the guard test so it counts pages or rows read (finding A).
2. Make the 7 schema/effective upgrade tests pass at HEAD, or prove on `main` that they fail there and give them an owner.
3. Resolve `pair_accept_delivers_mixed_opening_mail_once`: fix the product or rewrite the stale expectation, with the reason recorded.
4. Update `AUTOMATIC_PENDING_GUIDANCE`; it is model-visible and now false.
5. Get the user's decision on `ServiceSend` versus D9, and record it in TRUST-POLICY.md and the spec.
6. Do one doc-drift sweep: AGENTS.md:112, linux-e2e README, mod README:123, the `wake_on: all` comments.

## Untested scope
- **No full-suite run** of this branch. Merge gates were `cargo check` only.
- The pre-release wake e2e (`scripts/wake-e2e.py`) has not been run against this branch. Its `markers_ok` check passes when no frame header is visible on screen (ht-yha.11 minor), so the marker part of a live PASS can be vacuous.
- The q5 refit flake has never been run at base.
- Not covered by any test:
  - drain cost against a large backlog (finding A);
  - a disconnected top-level binding together with the seat-wide displayed flip (finding D);
  - the pair first send at p0 and p5 (only p1 per threshold is tested, ht-yha.12);
  - the passive/wake split by priority in the cross-surface mod fixture (ht-yha.15);
  - the `wake-on` deprecation and lossy-priority stderr lines (ht-yha.7);
  - the running daemon's capability reply (ht-yha.16 checks only the `ADVERTISED` constant).
- `BLOCKED-AUTH`: none recorded.

## Deferred OK
- ht-yha.2: test exemption scope and missing deadline assert (the column is NOT NULL).
- ht-yha.5.1: fragile header-byte lookup, duplicated codec recipe, `window_end` edge case, round-trip generator gap.
- ht-yha.5.3: garbled comment, trivial wrapper, report gaps.
- ht-yha.3: interim wake gap, now covered by priority_sweep flows 2 and 4 and priority_config_smoke; widened visibility of `plan_delivery_ack`; stale `delivery_mode` readers, which the ht-yha.16 audit cleared (only the redundant `seats.rs` `mod_stall_oldest` filter remains).
- ht-yha.4: weak capability probe; subagent display keeping `state` pending (intended and tested).
- ht-yha.7: receipts-capability probe skipped on priority-shaped plans (implied by `send.priority_v1`); handoff's legacy p0 spelling (justified for byte-identical retry).
- ht-yha.5.2: lowest-binding_id tiebreak when inheriting marks.
- ht-yha.6: `LEGACY_PAIR_MARKER` test-only branch (absent from release builds); operator invitation stores NULL confirmation (documented); test placement; tests not written red-first.
- ht-yha.5.4: out-of-scope `MARK_DISPLAYED_SQL` change (needed, and in TRUST-POLICY.md); unrelated codec bound removal.
- ht-yha.8: mod mirror pending (landed in ht-yha.9).
- ht-yha.9: one extra `effective_receipt` query per hydrated message; register.js sorting the whole queue.
- ht-yha.12: indirect wake check; implicit invitation-deadline ordering assumption.
- ht-yha.16: real-clock test; duplicate test coverage.
- My own minors C, D and E.

Files referenced:
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/delivery/page.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/service/delivery_pull.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/tests/service/delivery_pull.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/cli/handoff.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/tests/store/schema.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/tests/store/effective.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/tests/hook_entrypoint.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/service_send.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/lazy_delivery/wake.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/read_cursors.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-3a1203e0..99308304.diff`