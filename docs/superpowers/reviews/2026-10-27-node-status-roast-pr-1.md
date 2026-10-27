super-roast verdict: Should-fix (17 confirmed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling run by one user across their own machines (cooperative, same-user trust model); a daemon that runs for days, so log volume, write amplification and shutdown hygiene matter; no network-exposed surface beyond authenticated peer links.
inputs: integration/ht-seat-directory@0231b07ac vs main@f9dd1944d (phase 1 of ht-xg88, node status; ht-b3k6)
coverage: lanes correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, testing (8 scouts, all returned; no triage pass, lanes chosen by the orchestrator) · 61 raw → 30 deduped → 17 confirmed, 9 accepted as-is, 4 rejected · judges: none dispatched — the orchestrator verified each finding against the code itself
independence: partial (subagent scouts, inline verification by the author) · rung: manual fan-out
seat-agreement: unavailable — no judge panels ran

## Confirmed findings
- [Should-fix] src/service/sync_admin.rs PeerList — a wedged coordinator spends the whole budget on the snapshot, then the store read fails: `peer list` breaks instead of falling back to stored rows [lanes: correctness, concurrency-async, premortem]
  fix: the snapshot gets its own 2 s deadline inside the budget (PEER_LIST_SNAPSHOT_MS).
- [Should-fix] src/store/sync_status.rs node_state — stored, throttled contact overrides a live link: a just-linked node renders `unreachable since <now>` [correctness]
  fix: a live active link decides linked/stalled first; reach only without one.
- [Should-fix] src/sync/coordinator/peer_status.rs step/stuck — the progress mark is not keyed to the log: after an incarnation switch held < old mark and stalls are never reported [correctness]
  fix: marks are (LogKey, held); a log change counts as progress.
- [Should-fix] src/daemon/mod.rs — daemon-source writes (owner errors, drain note, final replay) bypass stamping; "every line" was false [correctness, testing]
  fix: StampedLog wraps the shared sink; every write path stamps with one shared line state.
- [Should-fix] src/sync/coordinator/peer_status.rs flush — a failing write retried every 1 s tick, logged every time [concurrency-async, premortem, security]
  fix: 10 s backoff, one log line per failure run and one on recovery; JoinError logged too.
- [Should-fix] src/sync/coordinator.rs shutdown — peer-status write neither joined nor finally flushed [data-migrations, concurrency-async, premortem]
  fix: final_peer_status_flush joins the write in flight and writes the differing rows, each bounded by CLOSE_WRITE_TIMEOUT.
- [Should-fix] src/store/sync_peer_status.rs ms() — a peer counter > i64::MAX fails every peer-status write [security]
  fix: values stored clamped.
- [Should-fix] src/sync/coordinator/peer_status.rs routine filter — app_seen_at changes every `seen`, so every linked node is rewritten every 10 s [hot-path-perf, simplicity-design]
  fix: app contact rewritten only after it advanced 30 s.
- [Should-fix] stored linked_since survives a crash or shutdown and claims an old link; caps/last_linked_at/linked_since written but never read [data-migrations, simplicity-design]
  fix: columns dropped (migration 0006 unreleased); linked state is live-only.
- [Should-fix] src/store/sync_peer_status.rs merge — a new boot kept the old boot's positions [data-migrations]
  fix: a new boot replaces every value.
- [Should-fix] spec §9 — "a downgrade leaves the tables unused" is false: an older binary refuses a newer schema [data-migrations]
  fix: spec corrected.
- [Should-fix] on_node_info — accepted repeatedly (urgent write per flip), only C0/C1 filtered [security]
  fix: once per link, `[0-9A-Za-z.+-]{1,32}`.
- [Should-fix] app_rx_wall set at link creation: a peer that never sent a coordinator frame shows an app age [correctness, testing]
  fix: `None` until the first coordinator frame.
- [Should-fix] src/test_support/multinode.rs relay — node_info consumed the 4th FREE slot, holding a held node's first full have [correctness]
  fix: node_info passes uncounted.
- [Should-fix] tests/multinode/node_status.rs — `stalled tx` phase flaky: app threshold 15 s vs 10 s `seen` under load [testing]
  fix: test app threshold 25 s.
- [Nit] output_compact peers — `empty` printed above node rows [correctness]
  fix: `empty` only with neither targets nor nodes.
- [Nit] store layer rule took presentation strings; renderer re-matched state literals; daemon log formatter duplicated [simplicity-design]
  fix: typed Stall/NodeState; reuse harness::manifest::format_rfc3339_utc.
- Tests added: partial tx progress, version validation and once-per-link, SQLite merge rule (boot reset, monotone app contact, NULL kept, clamp), node_state with a live link, failure backoff and log-once.

## Rejected / accepted as-is (with reason)
- N+1 queries in `nodes()` and per-tick `peer_rows()` rebuild [hot-path-perf] — bounded by known nodes, operator-only or microseconds; same pattern as `peers()`.
- Extra lookup and clock read in on_frame [hot-path-perf] — marginal next to decode and ingest.
- Unbounded budget on the write in flight [premortem] — mirrors flush_contacts; now reported once after 60 s and bounded at shutdown.
- Shrinking daemon.log retention from the 25-byte stamp [premortem] — accepted, noted in spec §4.6.
- `app` stall when the two ends disagree on the active link (speculative) [concurrency-async] — ranking is deterministic on both sides; not shown to persist; display only.
- vector_known vs round.full_have [simplicity-design] — different lifetimes (rounds restart, activation does not); both kept.
- LinkRow.since as the new-daemon sentinel [simplicity-design] — kept; the older daemon's reply prints the row as before.
- Redundant len() asserts [testing] — they record byte sizes per AGENTS.md (f).
