super-roast verdict: Should-fix (14 confirmed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling, one user's own nodes (cooperative, same-user); agent-facing CLI output where every byte is model context; the hook path is latency-budgeted.
inputs: integration/ht-seat-directory@6d41b47b2 vs main@fffa62793 (phase 3 of ht-xg88, seat directory)
coverage: lanes correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, api-contract, testing (8 scouts, all returned; no triage) · ~75 raw → 30 deduped → 14 confirmed, 9 accepted, 7 rejected · judges: none dispatched — the orchestrator verified each finding against the code
independence: partial (subagent scouts, inline verification by the author) · rung: manual fan-out
seat-agreement: unavailable — no judge panels ran

## Confirmed findings
- [Should-fix] queries.rs seats — every Seats query, including the hook's pane lookup (collect_pane_seats) and caller resolution, built a directory entry it discards (~7 statements per row on every hook event) [hot-path-perf, premortem]
  fix: entries only for listings (no target); `--local` rejects before building one.
- [Should-fix] human.rs seat table — a remote seat's presence word printed unescaped (terminal control from a peer) [security]
  fix: escaped; and presence is restricted on receipt.
- [Should-fix] seat_status sanitize — a peer could send a display word (`unreachable`, `stale`) as a fresh presence, or row punctuation in the harness, and mislead every surface [security, correctness]
  fix: presence ∈ busy/idle/offline else `-`; harness `[A-Za-z0-9._-]{1,16}` else dropped.
- [Should-fix] cli/mod.rs filter gate — any probe failure (Ctrl-C, deadline) became "needs a newer daemon, stop it"; uncached extra round trip [concurrency, premortem, simplicity, api-contract, correctness]
  fix: `client.supports_capability` (cached), `invalid_request` like the sibling gate; comment corrected (an older daemon refuses the field, it does not ignore it).
- [Should-fix] queries.rs seats — `--local` judged by the state word, so a retired remote seat passed [correctness]
  fix: the `is_local` column.
- [Should-fix] seats_argv — a filter value starting with `-` broke the `next:` command [correctness]
  fix: `--flag=VALUE`.
- [Should-fix] compact seat row — continuity (unresolved/retired) no longer shown; SKILL still promised timestamps in machine text [api-contract, correctness]
  fix: ` unresolved` / ` retired` markers; SKILL now says `--json` keeps every field.
- [Should-fix] participant row — the advisory suffix sat between `remote` and the flags [api-contract]
  fix: at the end of the row; grammar doc updated.
- [Should-fix] labels joined before escaping [simplicity, correctness]
  fix: each part escaped on its own (`SeatDirectory::labels(escape)`).
- [Nit] three renderers repeated the row head [simplicity] — fix: `entry_head`/`labels_tail`.
- [Nit] human `unreachable` without a contact differed from compact [api-contract] — fix: `unreachable never`.
- [Nit] module header of local_seat_status stale; ParticipantLocation doc said labels are client-local [simplicity, api-contract] — fixed.
- [Nit] SKILL omitted that `--label` matches remote seats only [api-contract] — fixed.
- Tests added: `matches` per criterion (incl. `--on` = tag, negative cases), local row and continuity markers, participant suffix for every state with byte size, per-part label escaping, filtered continuation (`--node=zq7`, cursor resumes), negative `--node` in the e2e.

## Rejected / accepted as-is (with reason)
- Older CLI with `deny_unknown_fields` result structs vs a newer daemon [api-contract] — same as `home_unreachable` before; CLI and daemon ship as one binary.
- Unfiltered end-of-page probe can emit a `next:` that is empty under a sparse filter; up to 100 examined rows per page [correctness, premortem] — designed work bound (spec 6.1); accepted.
- `reach` / node lookups per remote row, not memoized [hot-path-perf, premortem] — user-invoked listings, bounded by the page.
- Directory adds bytes per row to byte-bounded pages [premortem] — pane lookups now carry none.
- Typed state enum instead of strings [simplicity, api-contract] — the wire stays tolerant of new words; one producer, renderers match literals.
- Multi-token STATE in the compact row [api-contract] — documented in the skill; `--json` for parsing.
- Duplicate seat_active in presence and directory [hot-path-perf] — listings only now.
- Render tests in the sync_admin suite [simplicity] — kept beside the store tests they share fixtures with.
- `seat inspect` fills only the top-level directory [api-contract] — the summary's would duplicate it.
