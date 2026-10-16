# v2.0.0 candidate evidence (2026-10-16)

| Check | SHA | Result |
|---|---|---|
| Full suite + leak + default-features + clippy + harness-neutral + fmt | 8ee926127 | 7442/7442, 233 s, all gates clean (`full-suite-8ee926127.txt`) |
| Real ssh, two Linux nodes (`scripts/linux-e2e/run real-ssh`) | 8ee926127 | 11/11 (`real-ssh-report.md`) |
| `peer --stdio` relay on Linux, red then green | 8ee926127 | old io::copy pump: test blocks; fixed pump: 9/9 + dialer 15/15 (`linux-pump-red-green.log`) |
| Linux process table (`host::process_table`), pid 1 root-owned | 8ee926127 | 11/11 (`linux-process-table.log`) |
| Pre-release wake e2e, Herdr 0.9.5 (`scripts/wake-e2e-herdr 0.9.5`) | 8ee926127 binary | 113/114; the one FAIL was the driver missing a `[p0]` marker Claude's renderer wrapped (`wake-e2e-report.md`) |
| Wake e2e re-run of that case after the driver fix (claude, mixed-order/group) | 78d6ac8e3 driver, same product code | 3/3 (`wake-e2e-mixed-rerun-report.md`) |

Final release candidate: **a10b1cda9** (then this evidence commit).

| Check | SHA | Result |
|---|---|---|
| Full suite + leak + default-features + clippy + harness-neutral + fmt | a10b1cda9 | 7445/7445, 315 s (over the 300 s budget, not a release gate), all gates clean (`full-suite-a10b1cda9.txt`) |
| Real ssh, two Linux nodes | a10b1cda9 | 11/11 (`real-ssh-a10b1cda9.txt`) |
| Post-cap scoped roast (f2dd1f788..7f0269782) | — | Should-fix (4) [converged]; the two Should-fix and the docs Nit fixed in a10b1cda9; report `../2026-10-16-cross-node-sync-roast-pr-post-cap-audit.md` |

Product code changed by these checks: `src/cli/peer.rs` (relay deadlocked sshd over real ssh on Linux) and `src/sync/dialer.rs` (attempt outcomes logged), commit 483f9e3ff; `src/main.rs` and `src/cli/instance.rs` (a fresh node creates its default state dir, owner-only, never replacing an explicit one), commits 7f0269782 and a10b1cda9. Between 8ee926127 and a10b1cda9 only `peer --stdio` state-dir resolution changed (`git diff --stat 8ee926127 a10b1cda9 -- src`: src/cli/instance.rs, src/main.rs), so the wake e2e at 8ee926127 (single-node wake path, which these files do not reach) is reused rather than re-run, and the Linux relay/process-table runs at 8ee926127 stand for unchanged code.
