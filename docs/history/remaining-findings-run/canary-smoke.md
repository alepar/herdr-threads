# Canary configuration smoke (ht-p03.14.7)

Scheduled-job configuration exercised before merge on Linux. Container: `docker run --rm --platform linux/amd64
ubuntu:24.04` (Docker Desktop 29.8.0 on an Apple Silicon host, amd64 emulation). **Actual arch: `uname -m` = `x86_64`,
Ubuntu 24.04.5 LTS, Python 3.12 (system), node v22.23.3 / npm 10.9.9 (official tarball), rustc 1.94.0 (rustup, the
workflow's `RUST_TOOLCHAIN`), `cargo build --locked` of herdr-threads in the container.** Source mounted read-only and
copied to `/work`; fresh container `HOME=/root` (no `~/.claude`, no `~/.codex`). No push, no live `gh`.

All runs: `--model-tier off --keep`. Harness set `both`. Registry = the real npm registry on the run date (2026-10-02).

## First pass: Linux-only failure found and fixed

Both configurations 1 and 2 first ended **exit 2 (infra_error)** on Linux: `t0.hook-fires` failed with
`the local 401 stub printed no port ... File "/usr/lib/python3.12/http/server.py" ... import random ...
ImportError: cannot import name 'bisect' from 'bisect' (/work/scripts/canary/bisect.py)`, and `t0.payload-parse`,
`t0.schema`, `t0.admission` (codex) followed from it. Cause: `scripts/canary/stub_api.py` is run by path, so
`scripts/canary/` is `sys.path[0]` and `scripts/canary/bisect.py` shadows the stdlib `bisect` that `random` imports;
on Python 3.12 `http.server` pulls in `email.utils` -> `random` at import time (on the macOS Python 3.14 used for
every earlier local run it does not, so it was invisible). Fix in the owning leaf's file: `stub_api.py` drops its own
directory from `sys.path` before any import (the same guard `file_issues.py` already carries), plus
`scripts/canary/test_stub_api.py` (stub starts with a poisoned `bisect.py` next to it). Re-run of every container
configuration after the fix is what is recorded below.

## Results

| # | Configuration | Result | Evidence |
|---|---|---|---|
| 1 | ubuntu-24.04 linux/amd64 (`x86_64`), `scripts/harness-canary.sh --harness both --versions latest --model-tier off` | **PASS** (exit 0) | claude 2.1.287 all_pass, codex 0.160.0 all_pass; summary header `runner linux/x86_64`; tier-0 rows pass except expected skip/warn rows |
| 2 | since-verified on the real registry: `--harness both --versions since-verified --bisect --model-tier off` (in the container) | **PASS** (exit 0) | candidates asserted equal to the expected list, below |
| 3 | job tail in the container: report -> `file_issues.py --dry-run --existing-issues none.json` -> step exits with the canary's code | **PASS** | `file_issues rc=0`; tail step `exit=0 (canary code 0)` for both reports. Propagation of 0/1/2 shown with the real break report: step exit 0 -> 0, 1 -> 1, 2 -> 2, unset -> 2 (`exit "${CANARY_CODE:-2}"`). Before the fix the same tail returned 2 for the infra_error report, as the workflow would |
| 3b | live-gh `File issues` step (`gh label create` / `gh issue create`) | **NOT_EXERCISED** | needs `GH_TOKEN` and writes to the repo; first real run is the post-merge `workflow_dispatch` (root §Follow-on item 2). Dry-run covers the gh command shapes |
| 4 | `t0.isolation` tripwire with no `~/.claude/settings.json` (fresh container home) | **PASS** | `t0.isolation pass "real harness config untouched"` for claude 2.1.287 and codex 0.160.0 in both runs (ht-p03.14.2's absent-file outcome is pass) |

## since-verified candidate assertion (configuration 2)

Expected list computed separately with `versions.py` (`candidates(npm_list, "since-verified", doc, h)` minus
`known_broken` ranges from `docs/compatibility/harness-versions.json`) over the run's own `npm view ... versions --json`
output, compared with `candidates` in `canary-report.json`:

| harness | verified_max | known_broken ranges | expected | report candidates | equal |
|---|---|---|---|---|---|
| claude | 2.1.286 | none | `2.1.287` | `2.1.287` | yes |
| codex | 0.158.0 | none | `0.159.0, 0.159.1, 0.159.2, 0.159.3, 0.160.0` | `0.159.0, 0.159.1, 0.159.2, 0.159.3, 0.160.0` | yes |

`excluded` was empty for both. Bisect probed only the newest candidate (it passed, so no further search): codex
`0.160.0` role `newest`, 1 attempt. The known_broken subtraction is a no-op on today's data (the file has none), so the
equality does not exercise it; `versions.py`/`bisect.py` unit and self-tests cover that.

## Notes

- Warnings in the build (`unused imports LaneSet/Lane`, `IdleHook` never used) are pre-existing and unrelated.
- The canary commit is reported as `unknown` in the container because `.git` was not copied.
- Reproduce: mount the repo at `/src:ro`, copy to `/work`, install node 22 + rustup 1.94.0, `cargo build --locked`, run the
  commands in the table with `--keep --out /out/<name>`.
