# friction log — 2026-10-07-installer-human-permissions

- [2026-10-07 preflight] Workflow tool is unavailable; ordinary-subagents mechanism selected — supported capability fallback.
- [2026-10-07 preflight] User requires existing installer-permissions branch naming and coordinator-owned full suite/landing window — preserve these explicit constraints rather than stock branch/sweep/finish defaults.

- [2026-10-07 design roast] Workflow unavailable; canonical assembled prompts and engine accounting use manual fan-out with fresh GPT agents. No native model or product execution.

- code launch: parent owns ordinary-subagent scheduler using spawn_agent/wait_agent; one task chain at a time. Design418fea96 converged after2 roasts. Full-suite sweep DEFERRED to coordinator w4:p1 by explicit user scope; focused scope join remains ht-uwd.12. MergeCheck: nice cargo clippy --locked --all-targets --all-features -- -D warnings && nice scripts/check-default-features. Cross-target release builds/default-feature clippy remain coordinator CI evidence, not local parity claims. No early-unblock/background edge audit in this mechanism.
