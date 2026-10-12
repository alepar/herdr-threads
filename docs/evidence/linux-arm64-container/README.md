# Linux arm64 container install and link (2026-10-12)

Ubuntu 24.04 arm64 container (Docker Desktop, linux/arm64), no credentials, no authenticated
harness. Source `50d1197c` (`scripts/install.sh` is unchanged through `3ada57e3`).

- `build-release.log`: static `aarch64-unknown-linux-musl` 0.10.0 release built as `release.yml`
  does (Rust 1.94.0, `package-release.sh`, placeholder third-party licence page).
- `01-install.txt`: `install.sh --version v0.10.0 --setup` from that release served over `file://`
  (not a published GitHub download), linked into a headless Herdr 0.9.3 server (`herdr-linux-aarch64`, sha256 checked against
  `tests/herdr-releases.tsv`; the version is from the run notes, 01 does not print it); daemon ensured.
  It printed the old, since-corrected Linux warning.
- `02`/`03`: `doctor` `ok` outside and inside a Herdr pane with no harness installed.
- `04`: Claude Code 2.1.296 and Codex 0.162.1 installed from npm (Node 18; Claude Code wants >= 22).
- `05`/`06`: with those installed (never logged in), `--setup`
  installed hooks, skill and permissions; `doctor` was `degraded` (Codex hook review required, plus
  a transient right after daemon restart). No agent turn, wake or delivery ran. The "Codex 0.160.1"
  in 05's warning is a fixed advice string, not the installed version.
- `10-*`: `scripts/herdr-release-smoke` PASS against Herdr 0.9.1, 0.9.2 and 0.9.3. The `057d94f` in
  its header is the container's own copy of the source, not a commit in this repository. Its
  `LEAKS` line is the leak checker failing without `lsof` (traceback in the log); after installing
  `lsof` the check was clean (not captured here).

Not exercised on Linux: x86_64, a published-release download, upgrade, uninstall, the Herdr-down
cases, and any authenticated Claude Code or Codex session.
