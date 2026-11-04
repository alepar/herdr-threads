# super-auto report — 2026-11-03-atomic-remote-handoff

status: clean [degraded: whole-epic final review bounded only (omission provenance unavailable, review-collection gaps); coverage round 2 widened; 1 parked test finding] · sweep green @ 3058a90df
metrics: pending (upstream-feedback not yet run)

Branch `super-auto/atomic-remote-handoff`, base `main@1e620fdb5`, epic `ht-8lem` (left open: root acceptance is the coordinator's merge).

## Coordinator review fixes (review @ cb122a5ca)

- BLOCKING 1 fixed (ht-8lem.49): remote `--new-tab`/`--new-terminal` carry the profile env into the target create; a
  remote `--pane` with `--profile-dir` is refused (CLI and target); a test asserts the environment the started tab gets.
- SHOULD-FIX 2 and 3 fixed (ht-8lem.50): a launched-but-unreplicated row keeps its seat-addressed alert until the pair
  thread is held, `peer list` shows `overdue unlinked: seat <seat>`; `due_rows` skips rows it cannot advance; per-row
  errors in due/expiry/re-send passes are logged and skipped.
- Optional items (ht-8lem.51): request_key tag v3 kept for plain launch, periodic prune, one `node_key` helper, stage()
  seat check before the idempotent return, no empty `message=`, non-UTF-8 `--profile-dir`/`--cwd` refused.
- Roast of the fix (`2026-11-04-atomic-remote-handoff-review-fix-roast-pr-post-cap-audit.md`): converged, 0 Blocking; its
  regressions fixed (ht-8lem.52: truthful inbox line for a launched-overdue alert, the stale alert-shown guard, the
  timing-margin test; ht-8lem.53: SKILL.md, docs/environments.md, schema.rs comment and spec wording).
- Final sweep found one log assertion (the handoff task logged a Stop's cancellation); fixed in 56929bd24.

Checks at 56929bd24: full suite unfiltered 9152 passed, 0 failed, 54 skipped, 268 s;
`HT_LEAK_RUN_ID=f68eaf6b-8101-4ac7-926d-691036f3d2d4`, check-no-leaked-processes clean; clippy -D warnings,
check-default-features, check-harness-neutral, check-wire-compat, fmt all pass. Log `evidence/ht-8lem/full-suite-56929bd24.log`.

Still open (punch list): `overdue_while_partitioned` does not force the Launched-before-link ordering; the retired-author
re-send filter (target refuses them) and the round-1 Nits listed below.

## What landed

A remote handoff (`handoff --pane P@M`, `--new-tab --space S@M`, `--new-terminal --space @M`) is one signed request
(cap `peer.handoff.v1`). The sender's node signs a `SenderBlock` (domain `HTHOFF01`); the target validates it, allocates
the seat, writes `thread.direct`, the recipient join and the `message.sent` (block embedded) in one transaction, then
launches (Herdr child `internal peer-handoff-launch`; hc releases its gate on the staged Send). Every node verifies the
embedded signature from the fact alone (`AuthorRule::HandoffRecipientHome`). The sender's node owns the deadline
(`messages.deadline_node`) through a monotonic obligation ledger (`handoff_obligations`: frozen signed bytes,
sent/launched/linked/failed/expired, 6-day TTL, re-send on link-up). A target that never answers raises a seat-addressed
overdue alert (Q4). Prompt v2 is one line on every host: `<summary> — from: <name> (<id>)`. TRUST-POLICY N4, N5, N15
and the coordinator-approved extras (provenance `peer_handoff:<node_uuid>`, reworded and new accepted limits incl. the
6-day expiry) are in.

Beads: 29 merges (ht-8lem.1, .3–.11, .13–.15, .17, .18, .20, .21, .37–.48; .16 split into .20/.21); .2 superseded by .17;
gates .12, .19 closed on coordinator answers.

## Checks at the tip

- Full suite, unfiltered: `heavy tb nice cargo nextest run --locked --all-targets --all-features` @ 3058a90df —
  9129 passed, 0 failed, 54 skipped, 238 s (budget 300 s). `HT_LEAK_RUN_ID=439cc0b6-4153-4677-9575-534918071d8d`;
  `scripts/check-no-leaked-processes --run-id` clean (no processes, no temp files). Log:
  `evidence/ht-8lem/full-suite-3058a90df.log`.
- `tb nice cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
- `tb nice scripts/check-default-features`: pass. `scripts/check-harness-neutral`: ok (0 violations).
  `scripts/check-wire-compat`: ok (0 violations, baseline grew by new types only). `cargo fmt --check`: clean.
- Earlier sweeps: bd5a92402 had 4 failures (2 scans, 1 golden, 1 multinode race) → ht-8lem.44–47; re-run 1ba75ec1a had
  1 (same race in a sibling test) → ht-8lem.48; 3058a90df green.
- Docs-only commits after 3058a90df (run records, reports); no code changed after the measured tip.

## Reviews

- Design roast: `2026-11-03-atomic-remote-handoff-roast-design-1.md` (Should-fix 7, step-back redesign of the sender
  ledger, amend record `amend-design-1.md`), `…-roast-design-2.md` (converged, punch list applied to the spec).
- Code roast: `…-roast-pr-1.md` (2 Blocking: post-link receipt overdue never fired; failing migration test — both fixed,
  ht-8lem.37/.38), `…-roast-pr-2.md` (converged, 0 Blocking), `2026-11-04-atomic-remote-handoff-roast-pr-post-cap-audit.md`
  (post-loop changes: 3 Nits, 0 Blocking).
- Whole-epic final reviews (per invocation, `reviews/`): bounded only — the review-input assembler reported omission
  provenance unavailable, so no whole-range final review exists. The code roasts reviewed the whole branch diff.

## Remaining (punch list, not fixed)

- Final review (latest): the re-send loop does not filter retired authors; a re-send to a target now gets Refused
  (ht-8lem.39), so it ends as `failed`, but the loop could stop earlier.
- Audit Nits: the "one in-thread warning per handoff" wording (spec "Overdue across linking", schema.rs:2178 comment)
  predates ht-8lem.43, which allows a second warning after the first one's condition was cleared; the
  `cli_poll_after_a_slow_resend_stays_inside_the_bound` test relies on ~50 ms wall-clock margins; `overdue_while_partitioned`
  does not force the Launched-before-link ordering it claims to cover.
- Round-1 punch list: due-batch starvation on a stuck row (handoff_obligations.rs:503), plain remote launch across an
  upgrade (peer_launch.rs:291), stale SKILL.md:64 text, a missing log line (peer_handoff.rs:606), seat-known-no-pair
  due_step silence (handoff_obligations.rs:339), branch history hygiene (handoff.rs:840).
- Parked (ht-8lem.15): the multinode overdue test does not assert recipient-ACK clearing (asserted at store/service level).
- History: ~230 commits including no-op and non-compiling intermediates; the final reviews recommend a squash or cleaned
  history on merge.
- Task worktrees and the run temp root were removed after the run (all merged; the PIDs the Finish sweep could not inspect were systemd --user, sd-pam and two sshd sessions). The integration worktree stays for your review; remove it and its target after the merge.

## New model-visible strings (sizes)

Prompt v2 ` — from: ` 11 bytes (23 + id overhead, 26 + name with one, max 290); `--summary is required for a handoff to
another machine` 55; `--deadline for a handoff to another machine is at most 6 days` 61; refusal details 55/44/60;
`sender seat not replicated here yet` 35; `sender node key not replicated here yet` 39; upgrade error 94 + label/version;
seat-addressed alert row 68 + tag; peer list `expired unconfirmed` 32, `expired unlinked: seat` 36, `failed:` 21 (plus
values); `; message <m> stands in thread <t>` 31; ledger linked result 31; overdue payload delta 49. Each is also stated in
its commit message.

## Proposed CHANGELOG lines (3.0.0)

- Breaking: a remote handoff is one signed request (`peer.handoff.v1`). The target writes the task message before the agent
  starts; `--summary` (required) and `--profile-dir` (absolute, on the target) work remotely. A peer without the
  capability is refused with an upgrade error; there is no two-step fallback. `--thread`, `--new-thread`, `--goal`,
  `--thread-name`, `--topic` and `--invite-deadline` are refused for remote handoffs; `--deadline` is capped at 6 days.
- The sender's node owns a remote handoff's ACK deadline and warns even when the target is offline (a seat-addressed
  alert when the target never answered). Unanswered requests are re-sent on link-up and expire after 6 days.
- Handoff prompts (local and remote) are one line: `<summary> — from: <name> (<id>)`.
- A 2.x node marks a 3.0 handoff message `failed` until it is upgraded and the fact is replayed (`sync unapplied`).
