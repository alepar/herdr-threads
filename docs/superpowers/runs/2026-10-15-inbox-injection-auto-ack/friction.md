# Friction log

- [2026-10-15 pre-flight] native worktree landed outside writable roots; moved the same worktree into repository .worktrees before edits. Git metadata requires CLI-only escalation in this sandbox.
- [2026-10-15 design] Named opus/sonnet/fable model overrides unavailable; used inherited GPT model for fresh-context agents — semantic phase/role separation retained, vendor tiering unavailable.
- [2026-10-15 design baseline] Seeded main target lacked matching debug artifacts; first focused hook_inbox baseline rebuilt for 11m11s then passed 13/13 tests in 0.02s — cold build only, not an incremental-budget measurement; target now warm.
- [2026-10-15 design-roast-1] Workflow tool unavailable; canonical roast engine ran through manual file-backed fresh-agent fan-out — same-family GPT seats, assembled prompts and engine coverage/routes preserved.
- [2026-10-15 design-roast-2] Workflow unavailable; retained canonical engine manual fan-out, fresh late-round+regression+domain scouts, same-family GPT model substitution.

- [2026-10-15] phase code: task ht-794l.1 first focused cargo test used filter without --test combined and triggered all-target crate links on fresh task-worktree path (>2m despite seeded dependencies). Narrow subsequent commands to --test combined / --lib; initial build kept running for honest verdict.

- [2026-10-15] phase code: fresh task-local Rust links exceeded 5m amid many peer compiler processes despite seeded dependencies; task .3 --lib watch compile also slow. Agent process inspection confirms progress. No stopped builds or fabricated failures.

- [2026-10-15] phase code: stop-run-processes sandbox process inspection denied ps/kill yet emitted stopped for active compiler PIDs; initial cargo actually finished after12m25s. Final hygiene must use approved escalation and record actual findings. No test daemon spawned by the protocol test.

- [2026-10-16] phase code: sandboxed socket tests fail fixture UnixListener bind EPERM before behavior; exact focused reruns via scoped escalation passed (.3 integration6, .2 native6/golden2, .4 EPIPE1). Sandbox process reaper left one owned sleep stand-in, stopped by escalated scoped helper; final .3 check clean.

- [2026-10-16] phase code: .2 broad single-libtest hook/handoff batch exited0 without final summary after some failures due process-wide lifecycle interactions; not counted as PASS. Switched final focused library verification to nextest process-per-test and repaired exposed stale fixture assumptions/shared1024-byte guidance budget.

- [2026-10-16] phase code: .1 early checkpoint initializer adaptation mistakenly added receipt_candidates to SemanticMutation variants in three files; .2 found compiler E0559. Upstream explicit followup commits fixed sites; final canonical/source tests and clippy passed before review.

- [2026-10-16] phase code: ordinary stacked merge paused after first --onto rebase for seam review. On seam-cleared redispatch refreshed base to returned mergeBasefc791495; original stackbase214d160f is no longer ancestor and repeating --onto original base risks replaying integration commits. Explicit guard added to dispatch, no code loss/replay occurred.

- [2026-10-16] Code roast used manual fan-out, inherited GPT family substitutions. Initial judges inherited orchestration context; excluded and replaced with fork_turns=none blind judges before verdicts. All 11 scouts and seven valid judge seats completed.

- [2026-10-16] Code roast round2 manual fan-out with inherited GPT substitutions: all 12 scouts returned no candidates, no judges needed; no degradation.
