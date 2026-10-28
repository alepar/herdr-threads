super-roast verdict: clean (0 nits, 2 FYI) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling with a durable local store; data-loss in the store is floor-class. The change is a daemon-start ordering fix for `sync resync`.
inputs: bead/ht-la3c@ff6a84166 vs main@6e918785b
coverage: unavailable — inline run, no coverage object; lanes reviewed by hand: correctness, failure-mode, test-isolation
independence: none (inline) · rung: inline

## Change under review

`resume_at_start` (resync step 0: VACUUM backup, rename db/-wal/-shm into
`backups/<ts>/orig/`) ran in `run_elected_impl` before `OwnerLock::acquire`,
so a contender that loses the election could still move the store. It now
runs inside the elected factory. Regression test:
`tests/daemon/sync_resync.rs::election_loser_leaves_the_running_store_in_place`
(fails on main: the loser moves the running store; passes on the branch).

## Confirmed findings

none

## Rejected (with reason)

- [Should-fix] src/app.rs pre-election `load_node_identity` — "other pre-election
  code still touches the store". Rejected: between function entry and the factory
  only `prepare_node_dir`, the key read, `database_path.exists()`, settings and
  `publish_node_json` run; no SQLite open remains before election (grep of
  `database_path`/`Connection` in that span).
- [Should-fix] resync phases — "a second elected daemon re-runs the move".
  Rejected: elected starts are serialized by the owner lease and the first one
  writes `moved` before releasing it; `moved`/`salvaging` never move files
  (`resume_at_start`, existing `crash_after_moved_resumes_without_second_backup`).

## Unverified nits (spot-checked)

- [FYI] src/app.rs — `load_node_identity` now sees the old store present during
  a pending resync, so a missing node key is refused (`KeyMissingForStore`)
  where before step 0 had already moved the store and a new key was minted.
  The node that requested the resync had a key; refusing is the conservative
  outcome (no silent new identity).
- [FYI] src/app.rs — step 0's `resync: rebuilding ...` line and a step-0 start
  failure now go to `daemon.log` (post-election stderr) rather than the
  launching ensure's pre-election stderr, the same place every other factory
  start failure (e.g. journal open) already reports.

## Escalations (need human)

none
