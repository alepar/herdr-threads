# v2.0.0 candidate evidence (2026-10-16)

| Check | SHA | Result |
|---|---|---|
| Full suite + leak + default-features + clippy + harness-neutral + fmt | 8ee926127 | 7442/7442, 233 s, all gates clean (`full-suite-8ee926127.txt`) |
| Real ssh, two Linux nodes (`scripts/linux-e2e/run real-ssh`) | 8ee926127 | 11/11 (`real-ssh-report.md`) |
| `peer --stdio` relay on Linux, red then green | 8ee926127 | old io::copy pump: test blocks; fixed pump: 9/9 + dialer 15/15 (`linux-pump-red-green.log`) |
| Linux process table (`host::process_table`), pid 1 root-owned | 8ee926127 | 11/11 (`linux-process-table.log`) |
| Pre-release wake e2e, Herdr 0.9.5 (`scripts/wake-e2e-herdr 0.9.5`) | 8ee926127 binary | 113/114; the one FAIL was the driver missing a `[p0]` marker Claude's renderer wrapped (`wake-e2e-report.md`) |
| Wake e2e re-run of that case after the driver fix (claude, mixed-order/group) | 78d6ac8e3 driver, same product code | 3/3 (`wake-e2e-mixed-rerun-report.md`) |

Product code changed by these checks: `src/cli/peer.rs` (relay deadlocked sshd over real ssh on Linux) and `src/sync/dialer.rs` (attempt outcomes now logged), commit 483f9e3ff. Everything else changed was test tooling. The product code is identical between 8ee926127 and 78d6ac8e3.
