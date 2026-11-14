# Rules for every opencode-epic implementer (ht-vxdp)

Read `AGENTS.md` first; it is binding. These add to it.

## Work area

- Work only in the worktree and on the branch your brief names. Commit there. Never commit on `main` or on another agent's branch.
- Design: `docs/superpowers/runs/2026-11-14-opencode-harness/2026-11-14-opencode-harness-design.md`. It is decided; do not redesign. If the code contradicts it, or you need to change anything outside your task's scope (especially `src/harness/adapter.rs` traits, `registry.rs` beyond the registration row and its tests, the `setup.rs` engine, `launch.rs`, `src/cli`, `src/service`, `src/store`, `src/host`, `src/protocol`), stop and report it in your final report instead of writing it. Another coordinator is changing Hermes launch and setup plumbing at the same time.
- Evidence about opencode: `research-opencode-surface.md` and `probe-real-model.md` in the same directory. Confirm against them; never assume opencode behaviour, and never pin to an exact opencode version (AGENTS.md "Harness and Herdr versions").

## Build and test

- The host has no C toolchain. Every cargo command runs as:
  `/var/home/alepar/.cache/herdr-threads-heavy/heavy /var/home/alepar/.cache/herdr-threads-heavy/tb <cmd>`
  for example `… tb nice cargo clippy --locked --all-targets --all-features -- -D warnings`.
- The worktree's `target/` is already seeded. Never set `CARGO_TARGET_DIR`, never create another target directory, never build under `/tmp`.
- Per change: clippy as above, `scripts/check-harness-neutral`, `scripts/check-wire-compat`, `cargo fmt`. Run the focused tests for what you changed (`nice cargo test --locked --all-features <filter>`). Do not run the full suite; the coordinator runs it at integration.
- Tests use isolated HOME, XDG dirs, Herdr and state, through the existing test helpers (`test_support::spawn`, `test_support::scratch::temp_path`).

## Never

- `git push`, `git stash`, `git rebase` of shared history, or a commit on `main`.
- `herdr plugin link` / `unlink`; restarting, stopping or killing the shared Herdr server or the real herdr-threads daemon; closing panes or workspaces you did not create.
- `herdr-threads setup` / `unsetup` / `install.sh` against real config. Any setup run needs an isolated HOME and config root.
- Writing to `~/.claude`, `~/.codex`, `~/.aisw`, `~/.gemini`, `~/.config/opencode`, `~/.local/share/opencode`, `~/.opencode`, `~/.agents`, `~/.hermes`.
- Running the real `opencode` binary without `HOME`, the four `XDG_*_HOME` variables and `TMPDIR` pointing into a scratch directory.
- Reading credentials. Only a brief that says so explicitly may use the OpenRouter key, and then only through a mode-0600 env file in a scratch root, never printed, logged, committed or kept in evidence.
- Any `herdr-threads` write (send, ack, accept, join, invite, leave, check-in): you are a subagent of the coordinator's seat.

## Commits

- Small, focused commits with a straight message. Trailers, in this order at the end:
  `Outside-adapter: <none | comma-separated repo-relative paths or globs changed outside src/harness/opencode/ and integrations/opencode/>`
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`
- A change that adds an accepted limit or touches attribution updates `TRUST-POLICY.md` in the same commit.
- A new model-visible string states its size in bytes in the commit message (AGENTS.md "Token-efficient communication" (f)).

## Finish

- Stop every process you started; remove scratch files.
- Final report: commits (hash and subject), what was verified and how (commands and results), anything skipped or failing, every deviation from the design, and every edit outside your scope that you found necessary but did not make.
