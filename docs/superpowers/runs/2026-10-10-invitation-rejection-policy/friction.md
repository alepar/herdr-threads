# Friction

- 2026-10-10: startup inbox lacked lifecycle context; actual top-level session cooperative check-in resolved it.
- 2026-10-10: sandbox blocked Git ref creation; approved scoped git worktree escalation resolved it.
- 2026-10-10: upstream version lookup failed inside sandbox; approved gh api lookup confirmed 6.4.2-alepar4.20 matches loaded version.
- 2026-10-10: initial isolated cold optimized test build takes substantially longer than incremental budget; focused test is queued behind baseline build.

- [2026-10-10 code] Concurrent isolated task builds shared one warm target and guidance waited on Cargo artifact lock for several minutes; separate target avoids lock but repeats cold optimized compile.
- [2026-10-10 code] Guidance parser invocation queued in shared Cargo target returned runtime artifact (test count differed), despite task cwd. Agent disclosed provenance caveat; exact combined parser check remains mandatory.
- [2026-10-10 code] review-package script lacked executable permission; invoking it with bash generated the expected package unchanged.
