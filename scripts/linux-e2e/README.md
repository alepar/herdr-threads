# Linux e2e runner (Docker)

`scripts/linux-e2e/run` runs the herdr-threads e2e checks in a disposable Linux container on any Docker host
(Docker Desktop on macOS included), natively on arm64 or amd64. Python 3.9+ and Docker on the host; nothing else.

```sh
scripts/linux-e2e/run self-test                       # wake-e2e driver --self-test in Linux (~1-2 min warm)
scripts/linux-e2e/run release-smoke [--versions 0.9.3]  # scripts/herdr-release-smoke, every listed Herdr by default
scripts/linux-e2e/run doctor [--hide-harnesses]       # static musl release -> install.sh -> doctor in/out of a pane
scripts/linux-e2e/run wake-e2e [creds] [--live] [-- DRIVER ARGS]   # the live wake matrix (see Credentials)
scripts/linux-e2e/run clean [--all [--keep-image]]    # leftovers of this worktree (--all: every labelled object)
scripts/linux-e2e/run --self-test                     # this script's own tests, fake docker, no Docker needed
```

Common options: `--evidence-dir DIR` (default: a new directory under the system temp dir, printed at the end),
`--platform arm64|amd64` (default: the Docker server's architecture; the other one runs emulated, slowly), and
`--no-cache` (no named volumes).

## What a run does

1. Builds the image from [Dockerfile](Dockerfile) and starts the container from the immutable image id that build
   wrote (`--iidfile`, recorded in `runner.json` with the container's own image); the tag `ht-linux-e2e-runner:<arch>`
   is shared by every worktree and is never what `docker run` uses. The build uses `git archive <HEAD sha>
   scripts/linux-e2e tests/herdr-releases.tsv` as its context. Every `RUN` is `bash -euo pipefail`, and each
   download goes through [image-fetch.sh](image-fetch.sh) (download, SHA-256, version check, each fatal, then the
   artifact is moved into place). Contents: ubuntu 24.04 (pinned by its multi-arch index
   digest), Rust `1.94.0` (CI's `RUST_TOOLCHAIN`) plus the musl target, Herdr from `tests/herdr-releases.tsv`
   (newest listed release, SHA-256 checked for the container's architecture), Node 22 (official tarball, SHA-256
   pinned), Claude Code and the Codex CLI via `npm ci` from [npm/package-lock.json](npm/package-lock.json), which
   pins the sha512 integrity of every tarball including the per-platform native packages (installed, never logged
   in; `npm ci --ignore-scripts`, then Claude Code's reviewed postinstall run explicitly, see the Dockerfile),
   python3, git, procps and lsof; a non-root user `tester` without sudo. Update the pins in the `ARG` lines
   and, for the harnesses, with `npm install --package-lock-only` in `npm/`.
2. Starts one container with `docker run --init` (PID 1 reaps the helpers' zombies), labelled
   `dev.herdr-threads.linux-e2e=1` plus this worktree and run, no bind mounts, `--cap-drop ALL`,
   `--security-opt no-new-privileges` (W6 may have to relax this for the Codex sandbox).
3. Streams `git archive <sha>` (the SHA resolved once at start) into it, checks that the exported tree's id equals the host's `HEAD^{tree}` and
   rebuilds HEAD's commit object over it, so inside the container `git rev-parse HEAD` is the host's SHA (a
   one-commit shallow checkout) and `git status` is clean. Only committed content is tested: the run says so when
   the worktree is dirty.
4. Runs the subcommand's steps ([in-container.sh](in-container.sh)); each step's output is shown and saved.
5. Stops the container (`docker stop -t 5`: agents die first), copies its evidence out of the stopped container and
   removes it (`docker rm -f -v`) on every exit path, including Ctrl-C, SIGTERM and SIGHUP; a second signal during
   cleanup is reported and ignored.

The container bounds its own life. Its main process is a watchdog; the runner renews a lease file in it every
15 s (`docker exec touch`). If the runner is SIGKILLed or the host dies, the lease expires after 60 s; at that
point, after `--timeout` + 30 min for `wake-e2e` (2 h for the others), or on `docker stop`, the watchdog stops every
other process in the container, deletes everything that may hold a credential (`~/.ht-creds`, the driver's scratch
`codex-home` and `claude-config` dirs under `/tmp/ht-wake-e2e.*`, any `auth.json`, `.credentials.json` or
`.claude.json` there, and the harness homes' credential files), checks that none of it remains, and exits: 0 or 3
after a verified scrub, 4 if anything survived. Exiting stops the container. A killed runner
still leaves a stopped container (and, if it held one, its env-file dir `$TMPDIR/ht-linux-e2e-cred.<pid>.*`): the
next run removes this worktree's stopped containers itself (their evidence saved under `leftovers/`; the watchdog
has already scrubbed their credentials), except one whose runner is still alive (it holds its run lock
`$TMPDIR/ht-linux-e2e-run.<run>.lock`) or that stopped less than 60 s ago; it warns about everything else.
`clean` removes stale env-file dirs and run locks: older than 60 s and unlocked (a live runner locks its dir before
writing to it, so pid reuse cannot hide a stale one). `clean` (no flag) touches only this worktree's containers and
volumes, its stale env-file dirs and the unused dangling runner images it built (builds carry the worktree
label); `clean --all` removes every object with the
shared label, including containers someone started by hand from the image (they inherit its label). Images built
before a5fb8d75 carry no worktree label, so only `clean --all` removes them (or `docker rmi <id>`). This is fail-safe, not resumable: a host that sleeps, or a runner suspended with Ctrl-Z, for longer than the
lease (60 s) stops a live run too. Lease renewal failures are counted in `runner.json` (`lease`) and logged in
`host/lease.log`.

Evidence layout: `runner.json` (SHA, tree, image id, steps with exit codes and times, credential kinds,
container removal), `host/NN-step.log` (each step's output), `container/` (what the steps wrote: Herdr server log,
release smoke report and leak check, doctor reports, pane screens, harness process snapshots).

Exit codes: 0 passed, 1 failed, 2 refused (usage, preflight, missing credentials), 3 the evidence could not be
copied out (overrides 0 and 1: an unreviewable run never passes), 128+N interrupted by signal N. On exit 3 the
stopped container is kept, and its name and a `docker cp` recovery command printed (`runner.json`:
`evidence_copy`, `kept_container`, `container_removed: false`), when the watchdog's exit code (0 from its stop trap,
3 from a lapsed lease) proves it stopped everything else and verified the scrub first; otherwise the container is removed and the
evidence is reported lost. A container that vanished after it started (removed or killed outside the run) is
also exit 3: its evidence is gone. A later run of the worktree saves a kept container's evidence under `leftovers/`.

## Subcommands

- `self-test`: `python3 scripts/wake-e2e.py --self-test`.
- `release-smoke`: `scripts/herdr-release-smoke` (each Herdr in its own private IsolatedHerdr root inside the
  container; one smoke at a time per Herdr cache volume, under `flock`) with a short `TMPDIR`, then `scripts/check-no-leaked-processes --run-id`. The report is
  `container/release-smoke.md`.
- `doctor`: starts a headless `herdr server` in the container, builds the static musl release as `release.yml`
  does (with a marked placeholder `THIRD_PARTY_LICENSES.html` instead of cargo-about), lays out a `file://`
  release tree with `SHA256SUMS`, installs it with `scripts/install.sh --version vX.Y.Z --setup`
  (`HERDR_THREADS_RELEASE_URL=file://...`), waits for the daemon's first reconcile, then runs `herdr-threads doctor`
  and `doctor --debug` outside a pane (`container/doctor-outside.txt`) and inside a Herdr pane via `herdr pane run`
  (`container/doctor-pane.txt`). Passes when both report `ok` or `degraded` with `daemon: healthy`.
  `--hide-harnesses` removes `claude`/`codex` from PATH (a Linux user without them).
- `wake-e2e`: starts the headless Herdr server, builds `bin/herdr-threads` (`scripts/build.sh`), creates one
  workspace and types the launcher into its pane with `herdr pane run` (the driver needs `HERDR_ENV=1` and a
  pane). The launcher's preflight refuses (exit 2) before starting any harness unless every selected harness
  (`--harness`, default `claude,codex`) has a credential and `--live` was given; harness process snapshots
  before and after are in the evidence. Driver arguments follow `--`; the runner sets `--bin`, `--codex-auth-home`
  and `--evidence-dir` (`container/wake-e2e/`) itself and refuses them, and any abbreviation of a driver option
  (`--harn`), there; it passes `--harness` on in canonical form. `--live` refuses a dirty worktree unless
  `--allow-dirty` (untracked files count as dirty too, deliberately: `git status --porcelain`).

## Credentials (wake-e2e only)

No credential is ever baked into the image, bind-mounted, or written to the evidence; no real profile directory
(`~/.claude`, `~/.codex`, `~/.aisw`, `~/.gemini`) is mounted or copied. Credentials are accepted only together
with `--live` (otherwise the run is refused before Docker is touched):

| Input | Becomes | Notes |
| --- | --- | --- |
| `--codex-auth-file PATH` | streamed into the container as `~/.ht-creds/codex/auth.json` (dir 0700, file 0600); the driver gets `--codex-auth-home ~/.ht-creds/codex` and copies it 0600 into its scratch `CODEX_HOME` | must be JSON (a Codex `auth.json`) |
| `--claude-oauth-token-file PATH` | one bare token, `CLAUDE_CODE_OAUTH_TOKEN` | from `claude setup-token` (long-lived, no refresh); must start with `sk-ant-oat` (the prefix Claude Code itself checks); JSON, `NAME=value` and multi-field files are refused |
| `--anthropic-api-key-file PATH` | one bare token, `ANTHROPIC_API_KEY` | alternative to the token; warns unless it starts with `sk-ant-`. Claude Code may ask once whether to use a found API key: answering that dialog is W6's |

The Claude value goes into a mode-0600 env-file in a private 0700 host temp dir, handed to the single
`docker exec --env-file` that starts the headless Herdr server; Herdr's panes (the driver and the agents it
launches) inherit it. It is not in the container config (`docker inspect`), not in the image, and the env-file is
created just before that step and deleted right after it (and again at exit). Evidence files record only which
kind of credential was used. Inside the container the secret is visible to every process of the container user
(the environment of the Herdr server, its panes and agents; the staged `auth.json`): treat the container as
holding it until it is stopped, which the watchdog bounds (see "What a run does"). The launcher checks
that the server's environment reaches the panes (`server environment reaches panes: yes` in
`container/wake-e2e.txt`).

Refresh-token rotation: a ChatGPT-login Codex `auth.json` carries a refresh token. If Codex refreshes during the
run (the driver records `codex_auth_refreshed_in_run`), the new tokens land only in the container's copy, which
is destroyed with the container, and the provider may have rotated, and so invalidated, the refresh token in your
source file: you may have to log in again on the host. Prefer a dedicated account, or an API-key login
(`codex login --with-api-key`, whose `auth.json` has no refresh token). The same holds for any copied Claude
`.credentials.json`, which is why only a `setup-token` token or an API key is accepted for Claude.

## Costs

- Docker only (no quota): `self-test`, `release-smoke`, `doctor`, and `wake-e2e` without credentials or
  without `--live`. First image build ~2-3 min; a cold release or test build several minutes (the named volumes
  `ht-linux-e2e-runner-{cargo,target,herdr}-<arch>-<worktree>` keep the registry, `target/` and Herdr downloads
  warm between runs of one worktree; `clean` removes this worktree's, `clean --all` everyone's). `release-smoke` takes ~30 s per Herdr version after the build.
- Quota: `wake-e2e --live` drives real Claude Code and Codex agents (~80-100 short turns, ~10 min for the full
  matrix; pin cheap models with `-- --claude-model ... --codex-model ...`, narrow with `--harness`/`--cases`).
  It runs locally, person-initiated, never in CI (AGENTS.md "Pre-release wake e2e").

## Not done yet (live mode on Linux)

The driver's harness setup is still the macOS one: on Linux it needs a scratch Claude config with seeded
onboarding and folder trust (`.claude.json`: `hasCompletedOnboarding`, theme, `projects[cwd]`), the first-run theme
picker in its `DIALOG` pattern, and a decision on the Codex sandbox inside Docker (Landlock/seccomp/bubblewrap;
possibly `--security-opt seccomp=unconfined` or no sandbox). Until then `--live` gets as far as the driver.
