# Focused-pane poke skip (ht-yuz), Claude Code 2.1.288

This probe closes the gap the [summary smoke](../report.md) left open: the focused-pane poke skip, and a poke once focus moves away. It ran on 2026-10-03 (UTC) against the **real** Herdr session (0.9.1, with an attached TUI client), using a private herdr-threads instance built from main@3c22aa42 (schema v14, protocol 3).
- The earlier smoke used a private headless Herdr server, where `focused` never became true. This run confirms that hypothesis: on a session with an attached client, `herdr agent focus` sets `focused=true` (see `samples.txt`).

## Setup
- **Private instance:** `scripts/ht` pins `--state-dir /private/tmp/ht-focus-smoke/state` and the real Herdr socket.
- **Panes:** one scratch tab in the caller's workspace with three panes. S is a person seat (`me init`). X and Y are Claude agents started with `scripts/launch-claude.sh`.
  - They run model claude-haiku-4-5 in a project already trusted from the earlier smoke.
  - Their scratch settings carry the hooks plus `promptSuggestionEnabled: false`, so an idle composer reads empty (ht-1ip.46 rule; ht-6jt adds the setup prompt for this).
  - The project's `CLAUDE.md` forbids ACKing without an operator instruction, so receipts stay pending.
- **No real config touched:** nothing was written to `~/.claude`, `~/.codex` or `~/.aisw`.

## Probes
- **[probe2/](probe2/):** S sends one require-ACK message to X and Y, with a 120 s receipt window (soft point ≈ T+72). Y is focused from T+63 to T+114.
  - X was poked at about T+92 (`receipt due in 28s`).
  - Y received no poke while focused.
  - Focus returned with only 6 s left, so no after-release poke was possible.
- **[probe3/](probe3/):** same as probe2, but with a 300 s window (soft point ≈ T+180). Y is focused from T+151 to T+217.
  - **X, never focused by the probe:** poked at T+187 (`receipt due in 117s`).
  - **Y, focused across its soft point:** no poke between T+151 and T+217.
  - **Y after focus moved away:** poked at T+248 (`receipt due in 56s`), before the deadline.
  - The `due in 28s` line at T+2 in X's tail is scrollback from probe2. The `focused=true` on X near T+28 came from the operator's own client, not from the probe.

`samples.txt` records each pane's `focused` flag, its agent status and the poke and wake lines in its tail every 5 s. `wake_work.txt` holds the seats' wake rows at the end.

## Not established
- **Codex:** the focus skip on Codex was not run (the Codex account was out of quota; the scratch run used Claude only).
- **Skip reason not logged:** the daemon log does not record why a poke was skipped. The diagnostic label is dropped (code-roast punch-list item `src/notification/policy.rs:313`). The skip is inferred from X and Y being identical apart from focus.
- **Single run:** one run per probe; timings come from 5 s samples.
