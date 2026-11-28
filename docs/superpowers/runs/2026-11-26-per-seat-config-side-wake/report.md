# super-auto report — 2026-11-26-per-seat-config-side-wake

status: ready for your merge decision — full suite green at the tip, live qualification passing, 0 Blocking open · qualifiers: review collection gaps 1 · finding evidence gaps 10 · root acceptance waived by the user (ht-z41e.28)
metrics: pending (upstream-feedback not yet run)

branch: super-auto/per-seat-config-side-wake @ ca50d7a37 (base main, fork 4e4f628a4; main absorbed at d520ee3b5 = v3.7.0 seat restore) · epic ht-z41e

## What landed (39 implementation beads, all reviewed and merged)

- Per-seat config: `seat_config` table (node deltas 21-23 after main's v20), a closed typed registry (`nudge_threshold` agent tier over the existing column; `programmatic_wake` human tier, default off), `ht config seat list|get|set|unset` and `ht human config seat set|unset [--seat S]`, operator provenance, capability `seat.config_v1`, TRUST-POLICY write rules and the human-tier accepted limit. The node-level `programmatic_wake` is obsolete (parsed, ignored, doctor line).
- Release tooling: queue proof only with `--queue-proof`; the wake e2e driver enables queue wake per seat through the human CLI.
- Native wake defers on a displayed `/side` chat instead of typing into it.
- Codex `/side` queue routing to the root session. The live path needed seven links fixed across the fix loop and live runs: the every-event root marker (`transcript_path` null, plus a fork rollout's `thread_source: tui_side_chat`), the marker in the shared claim builder, the one-process multi-target queue gate, root-only wake-text eligibility, a root-scoped queue admission checked once at arm (no pane-wide `working`, no focus dwell, drafts refuse), lazy-wake reservation with a displayed sibling, and side-by-side pane composer reads.

## Evidence

- Live (Codex 0.172.0, Herdr 0.9.5, codex-2 credential copy): side-queue on a focused pane with a busy side turn (Accepted on the root, armed 0.49 s, nothing typed into the side), side-queue on the shared server, side-native-defer, side-nudge, side-draft, flag-flip, resume-idle, root-draft refusal, the `--queue-proof` Codex matrix (59/60, the one row a busy-invite flake that passed 3/3 on rerun), config-prose (an agent told in prose found the key and ran the human command), `herdr_release_wake`. See qualification.md and evidence/qualification/.
- Full suite at c693861ce: all 10078 pass (the sweep-fix beads ht-z41e.59/.60 fixed the first sweep's 2 failures; 4 human-actor failures in the re-run were an environment leak in the new tb wrapper, fixed by main and re-verified). Clippy, check-default-features, check-harness-neutral, check-wire-compat and the leak checks clean.
- Roasts: design converged (r2, amend r3); code loop capped at round 5 (3 + 2 redesign cycles) with 0 Blocking; post-cap audit 0 Blocking.

## Remaining (needs you)

1. ht-z41e.28 (whole-epic review with complete provenance) waived by the user; epic closed.
2. —
3. Decisions parked for you:
   - Side-draft rule: the queue lane refuses any typed composer text (side or root draft). Confirm, or ask for side-draft admission.
   - Root marker: a null `transcript_path` sets only the queue-only `auxiliary_session` bit (`concurrent_session` stays fork-only). Confirm, or ask for `source: fork` corroboration.
   - Fork rollout read race: the `tui_side_chat` marker is read once on the fork's SessionStart. If Codex hasn't flushed the first line yet, routing beside that side falls back to native (fails closed). Unverified Codex ordering.
4. Punch list (sub-Blocking): the queue pre-check before helper spawn (refused queue retries spawn a helper), a busy-root refusal reason in the daemon log, a native dwell test that can't tell its two cases apart, duplicated admission prologue in native.rs, and two wake-e2e Nits from the audit.
5. Review collection gap: invocation 2's cluster-3 report (this run's own docs) was never written, so every later roast ran bounded with provenance unavailable.
6. Merge: handed to the main coordinator for a squash merge at the user's request (message mnSGk2Tr6pWJ); one CHANGELOG.md conflict against main 7bda0e1a5.

## Cleanup

All task worktrees and branches are removed and the run's processes are stopped. The run worktree and its target remain until you choose how to merge.
