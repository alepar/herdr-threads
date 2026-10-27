super-roast verdict: Should-fix (16 confirmed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling run by one user across their own machines (cooperative, same-user trust model); long-running daemons, small meshes (2-4 nodes) today; display-only data from authenticated peer links.
inputs: integration/ht-seat-directory@17964c5cc vs main@4f639d7f6 (phase 2 of ht-xg88, seat status)
coverage: lanes correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, testing (8 scouts, all returned; no triage, lanes chosen by the orchestrator) · ~85 raw → 31 deduped → 16 confirmed, 10 accepted, 5 rejected · judges: none dispatched — the orchestrator verified each finding against the code
independence: partial (subagent scouts, inline verification by the author) · rung: manual fan-out
seat-agreement: unavailable — no judge panels ran

## Confirmed findings
- [Should-fix] seat_status.rs on_publish_seats — rev compared with activity ages: every publish bumped rev and the refresh path was dead; with relays losing transit time an unchanged rev could also circulate a loop [lanes: correctness, hot-path-perf, simplicity, premortem]
  fix: every publish is the next rev; same boot accepts only a higher rev, so loop copies die.
- [Should-fix] seat_status.rs pending/retry — a failed content write replaced by a content-less refresh: older rows shown Fresh [correctness, concurrency, data-migrations, simplicity, testing]
  fix: every update carries the whole snapshot (no refresh-only update); the store skips unchanged rows.
- [Should-fix] remote_seat_status.rs record — a seat learned after its snapshot stayed Unknown [correctness, data-migrations]
  fix: whole snapshots on every refresh re-apply the guard.
- [Should-fix] sanitize — empty presence passed and failed the column CHECK, poisoning every batch forever [all lanes]
  fix: empty presence becomes `-`.
- [Should-fix] seat_status.rs writes — retried every 1 s, no stuck report [concurrency, premortem]
  fix: 10 s backoff, log once per run and on recovery, stuck write logged once after 60 s.
- [Should-fix] clone suspicion — every plain restart read as clone_suspected for 180 s; stale relayed old-boot copies re-armed it [correctness, premortem, concurrency, testing]
  fix: boots tracked by build times; another boot is live only if built after the held boot's first build.
- [Should-fix] unbounded origins — any origin (nil included) held, stored and relayed [security, premortem]
  fix: only origins this node knows (a held log or a contact, direct or by seen).
- [Should-fix] publisher — a publish dropped by a full queue was recorded as sent [correctness, concurrency, premortem]
  fix: publish_seat_status returns whether it was handed over; retried next check.
- [Should-fix] local_seat_status — oldest-first LIMIT left the newest seats out past 512 [premortem]
  fix: newest first.
- [Should-fix] publisher labels — a transient host failure stripped that host's labels for 60 s and caused two rev bumps [premortem]
  fix: a host that does not answer keeps its last labels; one 3 s budget for all hosts.
- [Should-fix] harness ids over 16 bytes cut into another id [correctness]
  fix: dropped, never cut.
- [Should-fix] frame halving dropped up to half of what fits [correctness, premortem]
  fix: cut to the share that fits.
- [Should-fix] last_transition_at lost a disconnect when registered_at was NULL [correctness]
  fix: coalesce both; one cached query for harness and transition.
- [Should-fix] publisher read failures silent [concurrency]
  fix: logged once per run.
- [Nit] busy UI set duplicated from the wake path [simplicity]
  fix: shared `effective::turn_in_progress`, used by both.
- [Nit] `get` returned rows for seats retired since [data-migrations]
  fix: joins seats with state='remote'.
- TRUST-POLICY: the accepted limit now states that snapshot origin/boot/rev are unsigned link data (any linked node can misdescribe another's seats; a too-high rev hides that boot until restart), in-contract (A1) [security]. N13 states the display rule for every reader.
- Tests added: clone vs restart (unit), sanitize, ordering (`newer`), rev per publish, unknown origin dropped, late seat taken by the refresh, change propagates within the check period (e2e, refresh 60 s).

## Rejected / accepted as-is (with reason)
- Gossip fan-out O(N²) per change [hot-path-perf, premortem] — small meshes today; follow-up if the node count grows.
- Per-seat queries in local_seats (effective_observation, seat_active) [hot-path-perf, simplicity] — kept to reuse the canonical rules; the two binding queries are now one cached statement.
- Encode per destination link to measure size [hot-path-perf] — cold path, a handful per publish.
- Control-queue crowding on a new link [premortem, speculative] — one frame per known origin; not shown at current sizes.
- Ordering state not reloaded after restart [data-migrations] — first frame after restart is the current one in practice; short window.
- Receiver wall-clock step back >1 s delays a boot switch [premortem] — wall clock is decoration elsewhere; accepted.
- No retention for origins that vanished [data-migrations] — bounded by known nodes; `get` filters retired seats.
- Labels leave the machine [security, external] — documented in N13; same-user nodes.
- Read API with no production caller [simplicity, testing, correctness] — phase 3 by design.
- Shutdown adds bounded steps [concurrency] — each bounded; aborted publisher's blocking reads carry the cancelled budget.
