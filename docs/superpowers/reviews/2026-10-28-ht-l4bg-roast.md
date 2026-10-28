super-roast verdict: clean (2 nits)
mode: PR        iteration: 1 of 3
profile (assumed): Local same-user CLI plugin. The intent journal is crash-safe client-side state that every hook turn and every send touches. A wrong dedupe would put a duplicate check-in on the daemon. A long critical section fails an agent's send outright.
inputs: bead/ht-l4bg@5c0c869f9 vs main@94414e6d3
coverage: correctness/concurrency, test-quality and remaining-O(N) lanes, all from one scout · 9 raw → 6 deduped → spot-checked inline by the author · no full panels
independence: partial (subagent scout, inline verification) · rung: manual fan-out

## Confirmed findings
- [Nit] commit 5c0c869f9 message: "the only O(N) work left under the lock is the names-only allocator check in record". This overstates the change. `locked_check_in`, `pending_continuity` and `complete_operation` still list every filename under the lock; they no longer open the headers.
  verdict: confirmed (inline: `scan_headers(high, …)` and `complete_operation` both call `read_dir` while the lock is held)
  evidence: src/cli/journal.rs `locked_check_in` / `pending_continuity` / `complete_operation`. A listing takes 1-3 ms at 3000-6000 entries (measured).
  fix-shape hint: correct the commit text. A listing costs nothing like a header read, so no code change is needed.
- [Nit] tests/cli/journal.rs `populate_foreign_intents` writes its fixture files with `fs::write`, so they get umask mode rather than 0600.
  verdict: confirmed, harmless (only the directory's permissions are checked)
  fix-shape hint: none needed.

## Rejected (with reason)
- Two-phase scan misses a check-in, so a duplicate gets recorded. Rejected: every `next-ordinal` write and every `.intent` publish rename happens in one locked critical section (`record_locked`; `reserve(0)` at init is locked too). So every entry at or below the captured high water is published, removed or a crash gap, and readdir returns every entry that survives the whole scan. The locked pass covers everything above the high water. An early match that disappears triggers a full locked rescan.
- Unlocked `Journal::open` races with first-time init or with counter validation. Rejected: a partially written marker fails the `== "1\n"` check and falls through to the locked path. `counter()` lists the directory before it reads the monotonic allocator; `read_only` already validated without the lock in the same way.
- Suffix match in `complete_operation` misses entries the old header match found. Rejected: filenames are always `path(header.reference)`, and the header-equality check filters out suffix collisions.

## Unverified nits (spot-checked)
- FYI: `matching_check_in` now skips a same-scope entry whose `load` returns NotFound; before, it returned the error. Only a vanished or mislabeled file hits this, so the change is benign.
- FYI: `dedup_by_key(ordinal)` in `pending_continuity` would collapse two files that share an ordinal. Only a corrupt journal can contain them.
- FYI: each check-in lookup now takes the lock twice. That is more lock handoffs, but each hold is short.

## Escalations (need human)
- None. Follow-up outside this change: about 2795 pending `operator_orphan_invite` intents for one thread and seat, one every 2 minutes from 10-19 to 10-23, all retained by design ("add no discard or expiry policy"). Whether operator first-submission rejections should be discarded the way cooperative and service-allocation ones are is a retention-policy decision.
