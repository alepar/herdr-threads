status: clean [degraded: final review: not ready (unreviewed: whole-epic coverage and omission provenance remain incomplete), sweep: 3be83dfe3 — 7858 passed, 2 failed, 0 errors, 48 skipped; failing: latency::history_and_health_stay_fast_under_a_party_with_a_slow_host (ht-1tl3), no_leaks::sigkill_killed_run_leaves_no_processes (ht-oxdk) — both load flakes, 2/2 isolated pass; command: nice cargo nextest run --locked --all-targets --all-features @ 3be83dfe3, outstanding findings: 46] …(full report-status output truncated to 600 chars; most of the rest is review-provenance bookkeeping)
metrics: pending (upstream-feedback not run)

# Bytes-first journal — run report

Branch `super-auto/bytes-first-journal` (base `main`, main absorbed at 07d7655cb) · epic ht-76u6 · spec [design](2026-10-18-bytes-first-journal-design.md) · run state [run.md](run.md). This branch carries the whole unreleased 2.3.0 peer-wire work with its storage reworked bytes-first. The report was written by the orchestrator, not a drafter subagent.

## Delivered (run.md codeBuckets, bead tree, git log)

14 beads landed: 13 with clean task reviews and 1 after its fix pass, 0 merge-check failures.

- **Migration.** `NodeDelta` rebuild support (.1): foreign keys are turned off outside the transaction, the row copy runs in Rust, `foreign_key_check` is enforced, and the open-time schema audit replays the rebuild.
- **Delta 0005 rewritten** (.2, `0005_bytes_first_journal.sql` + `_after.sql`): `journal`/`journal_forked` get `raw` with a nullable all-or-none projection; existing rows are copied with `raw = canonical bytes`, checked `SHA-256(raw) == hash`; the full trigger and index set is recreated; `journal_opaque` is gone.
- **One decoder and reader audit** (.9, .10): a single-table decoder classifies rows by `raw` magic; SQL classification uses the same rule; the `parked_layout` status count is added; `CHAIN_ROWS` and the union readers are deleted.
- **Apply and promotion** (.4): an apply stall at `parked_layout` (`blocked_on opaque_layout`); promotion re-derives the projection and unparks successors, at store open and in the apply pass; a failed promotion leaves the row parked and is counted under `cannot_promote`.
- **Resync salvage and re-issue** (.5): salvage reads both schema-4 and schema-5 backups; re-issue works on the new rows.
- **Docs and trust wording** (.7).
- **Tests** (.6): two-node relay and promotion integration tests.
- **Integration sweep** (.8).
- **Roast fixes** (.18–.22): CHANGELOG section and wording, indexed no-op `promote_parked`, the opaque-fork terminal cut for re-issue, the test layout-registry lock, and a `user_version` re-check inside the delta transaction.
- **ht-evlm** (promotion follow-up) is closed as superseded.

## Reviews

- **Design roast:** 3 rounds (round 1: 10 confirmed, 2 Blocking; round 2: 3 confirmed, 1 Blocking; round 3: 3 confirmed, 0 Blocking). All findings were fixed in the spec and beads; the loop exited at the cap with 0 Blocking.
- **Code roast:** 2 rounds. Round 1: 6 confirmed (3 Should-fix, 2 Nit, 1 FYI), all fixed. Round 2: 6/6 resolved, 1 new docs Nit fixed inline.

## Verification

- **Full suite:** `nice cargo nextest run --locked --all-targets --all-features` at 3be83dfe3: 7858 passed, 2 failed, 48 skipped, 465 s.
- **The 2 failures** are load-sensitive and pass 2/2 in isolation; I filed both as P3 flakes:
  - `latency::history_and_health_stay_fast_under_a_party_with_a_slow_host` (ht-1tl3);
  - `no_leaks::sigkill_killed_run_leaves_no_processes` (ht-oxdk).
- **Leak check:** clean.
- **Per-task checks:** each task ran clippy -D warnings, check-harness-neutral, check-wire-compat and its focused tests; the coordinator's merge check ran check plus check-default-features.

## Remaining

- **Coordinator final reviews stayed partial.** They hit the same provenance gap in the unrelated run `docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51,58`, and both code roasts reviewed the raw diff.
- **Process shortcuts** (see friction.md): the design phase skipped the promotion-review and coverage passes; the step-back and scope filter were decided inline by me.
- **Untested by design:** behaviour against a real 2.1.1 binary (covered by the shadow decoder and the fixtures).
