# Linux e2e runner (Docker)

`scripts/linux-e2e/run` runs the herdr-threads e2e checks in a disposable Linux container on any Docker host
(Docker Desktop on macOS included), natively on arm64 or amd64. Python 3.9+ and Docker on the host; nothing else.

```sh
scripts/linux-e2e/run self-test                       # wake-e2e driver --self-test in Linux (~1-2 min warm)
scripts/linux-e2e/run release-smoke [--versions 0.9.3]  # scripts/herdr-release-smoke, every listed Herdr by default
scripts/linux-e2e/run doctor [--hide-harnesses]       # static musl release -> install.sh -> doctor in/out of a pane
scripts/linux-e2e/run wake-e2e [creds] [--live] [-- DRIVER ARGS]   # the live wake matrix (see Credentials)
scripts/linux-e2e/run clean [--keep-image]            # remove every labelled container, volume and image
scripts/linux-e2e/run --self-test                     # this script's own tests, fake docker, no Docker needed
```

Common options: `--evidence-dir DIR` (default: a new directory under the system temp dir, printed at the end),
`--platform arm64|amd64` (default: the Docker server's architecture; the other one runs emulated, slowly), and
`--no-cache` (no named volumes).

## What a run does

1. Builds the image `ht-linux-e2e-runner:<arch>` from [Dockerfile](Dockerfile), with `git archive HEAD
   scripts/linux-e2e tests/herdr-releases.tsv` as the build context: ubuntu 24.04, Rust `1.94.0` (CI's
   `RUST_TOOLCHAIN`) plus the musl target, Herdr from `tests/herdr-releases.tsv` (newest listed release,
   SHA-256 checked for the container's architecture), Node 22 (official tarball, SHA-256 pinned), Claude Code and
   the Codex CLI at pinned versions (npm integrity pinned; installed, never logged in), python3, git, procps and
   lsof; a non-root user `tester` without sudo. Update the pins in the `ARG` lines.
2. Starts one container with `docker run --init` (PID 1 reaps the helpers' zombies), labelled
   `dev.herdr-threads.linux-e2e=1`, no bind mounts, no extra privileges.
3. Streams `git archive HEAD` into it, checks that the exported tree's id equals the host's `HEAD^{tree}` and
   rebuilds HEAD's commit object over it, so inside the container `git rev-parse HEAD` is the host's SHA (a
   one-commit shallow checkout) and `git status` is clean. Only committed content is tested: the run says so when
   the worktree is dirty.
4. Runs the subcommand's steps ([in-container.sh](in-container.sh)); each step's output is shown and saved.
5. Copies the container's evidence out and removes the container (`docker rm -f -v`) on every exit path,
   including Ctrl-C, SIGTERM and SIGHUP.

Evidence layout: `runner.json` (SHA, tree, image id, steps with exit codes and times, credential kinds,
container removal), `host/NN-step.log` (each step's output), `container/` (what the steps wrote: Herdr server log,
release smoke report and leak check, doctor reports, pane screens, harness process snapshots).

Exit codes: 0 passed, 1 failed, 2 refused (usage, preflight, missing credentials), 128+N interrupted by signal N.

## Subcommands

- `self-test`: `python3 scripts/wake-e2e.py --self-test`.
- `release-smoke`: `scripts/herdr-release-smoke` (each Herdr in its own private IsolatedHerdr root inside the
  container) with a short `TMPDIR`, then `scripts/check-no-leaked-processes --run-id`. The report is
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
  before and after are in the evidence. Driver arguments follow `--`; the runner sets `--bin`,
  `--codex-auth-home` and the default `--evidence-dir` (`container/wake-e2e/`) itself.

## Credentials (wake-e2e only)

No credential is ever baked into the image, bind-mounted, or written to the evidence; no real profile directory
(`~/.claude`, `~/.codex`, `~/.aisw`, `~/.gemini`) is mounted or copied. Credentials are accepted only together
with `--live` (otherwise the run is refused before Docker is touched):

| Input | Becomes | Notes |
| --- | --- | --- |
| `--codex-auth-file PATH` | streamed into the container as `~/.ht-creds/codex/auth.json` (dir 0700, file 0600); the driver gets `--codex-auth-home ~/.ht-creds/codex` and copies it 0600 into its scratch `CODEX_HOME` | must be JSON (a Codex `auth.json`) |
| `--claude-oauth-token-file PATH` | one line, `CLAUDE_CODE_OAUTH_TOKEN` | from `claude setup-token` (long-lived, no refresh) |
| `--anthropic-api-key-file PATH` | one line, `ANTHROPIC_API_KEY` | alternative to the token |

The Claude value goes into a mode-0600 env-file in a private 0700 host temp dir, handed to the single
`docker exec --env-file` that starts the headless Herdr server; Herdr's panes (the driver and the agents it
launches) inherit it. It is not in the container config (`docker inspect`), not in the image, and the env-file is
deleted when the run ends. Evidence files record only which kind of credential was used. The launcher checks
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
  `ht-linux-e2e-runner-{cargo,target,herdr}-<arch>` keep the registry, `target/` and Herdr downloads warm between
  runs; `clean` removes them). `release-smoke` takes ~30 s per Herdr version after the build.
- Quota: `wake-e2e --live` drives real Claude Code and Codex agents (~80-100 short turns, ~10 min for the full
  matrix; pin cheap models with `-- --claude-model ... --codex-model ...`, narrow with `--harness`/`--cases`).
  It runs locally, person-initiated, never in CI (AGENTS.md "Pre-release wake e2e").

## Not done yet (live mode on Linux)

The driver's harness setup is still the macOS one: on Linux it needs a scratch Claude config with seeded
onboarding and folder trust (`.claude.json`: `hasCompletedOnboarding`, theme, `projects[cwd]`), the first-run theme
picker in its `DIALOG` pattern, and a decision on the Codex sandbox inside Docker (Landlock/seccomp/bubblewrap;
possibly `--security-opt seccomp=unconfined` or no sandbox). Until then `--live` gets as far as the driver.
