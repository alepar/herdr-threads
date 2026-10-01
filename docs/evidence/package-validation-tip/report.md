# ht-4is.10.3 package lifecycle validation at integration tip d635aca

**Verdict: PASS.** `scripts/validate-package.sh /private/tmp/ht-pkg-tip d635aca` exited 0 and printed `PACKAGE_VALIDATION_PASS` after 89 named checks (`result: pass`). All of it ran against a private Herdr server, config and state under `/private/tmp/htpv-8jalxh8p`. The shared server was never contacted and was not restarted.

## Environment
- Source: detached worktree `/private/tmp/ht-pkg-tip` at `d635aca724b36a07693dd5fd1ee00b8d904222ff`. It has been removed (worktree remove, then prune).
- Herdr: `~/.local/bin/herdr` 0.9.1, sha256 `5fc7a7e7adfaca56fa80aa89dcb025693357268dab8285b9ce2d08a2313c89de`. This matches the pinned hash.
- Shared server pid **before: 80553** (started Sun Sep 27 22:31:23 2026). **After: 80553**, same start time, so it was not restarted.
- Shared `~/.config/herdr/plugins.json` sha256 `0e2c5ef0f44f842ee448f7d2f527b1d6c652ac795dedf3d14a8467bcce23eed5` both before and after. The `~/.config/herdr/plugins/{config,github}` listing did not change.
- Binary sha256:
  - The linked checkout executable built from d635aca (`scripts/build.sh`, release, `herdr-threads 0.1.0`) is `278c83791becf9c430ad24354e8e08cfdd523f6b4fad7b21897eb8536aa7536c`.
  - I have no hash for the Herdr-installed executables (0.1.0 install and 0.1.1 update). The gate's uninstall step deleted them before I could hash them, and the script does not record them.
- Versions: 0.1.0 installed, then updated to 0.1.1 (update fixture commit `ac3d2c7`).

## Acceptance items

| Acceptance item | Result | Evidence (named gate checks) |
|---|---|---|
| Clean build/install from a private checkout: locked release build, one registration, shipped executable, hostile `CARGO_TARGET_DIR` ignored | PASS | clean install exits zero; registers exactly this plugin; resolves the published commit; build installed the shipped executable; build ignored the inherited CARGO_TARGET_DIR |
| Actions list | PASS | installed action list = {health, doctor, ensure, stop, view} |
| Installed native commands run the shipped executable with the correct host/state context | PASS | startup daemon argv is `<installed>/bin/herdr-threads daemon run --state-dir <private st>/herdr/plugins/herdr-threads --host-endpoint <private>/h.sock`; ensure reuses the startup daemon; doctor reports state_dir, host_endpoint, version and `version_matches` |
| Startup entry | PASS | after a private server restart, the startup entry ran once, exited 0 and started exactly one daemon (state `degraded`) |
| Live handoff | PASS | startup exit 0, boot id unchanged, daemon pid unchanged, reconciled 2403 ms after the handoff |
| Mail data through the shipped CLI | PASS | operator resolves a fresh seat; seat creates a durable thread |
| Operator view stays readable until quit | PASS | view renders the thread topic; pane still open after 3 s; unrelated input keeps it open; Enter refreshes; `q` closes |
| Failed build cannot register (fresh install) | PASS | install exits nonzero; Herdr reports the build failure; no registry entry; no managed checkout left |
| Failed rebuild over a working install | PASS | exits nonzero; registry byte-identical; shipped executable unchanged; daemon still serving |
| Source update preserves mail data, with an older daemon still running | PASS | updated startup exits 3 with version mismatch and starts no second writer; ensure and doctor report the mismatch; stop reaches the older owner, which exits; new ensure keeps the instance id with a new boot; thread and seat survive the update |
| Explicit shutdown | PASS | stop action succeeds; daemon exited; health reports not running; no private daemon remains; linked stop succeeds |
| Local link | PASS | the unbuilt link fails loudly and starts nothing. After a build: linked daemon is the checkout executable, keeps the instance, and reads the preserved thread |
| Shared server and global plugins untouched | PASS | the gate's shared-registry check passed. My independent pid, hash and listing snapshots were also unchanged. No private or `--handoff-import` processes were left after the run |

I also ran the deterministic `cargo test --locked --all-features --test package`: 11 passed, 1 ignored (the gate), 0 failed, in 102 s.

## Product defects
None found.

## Validator/tooling notes (not product source)
- `scripts/validate-package.sh --help` is not supported. `scripts/validate-package.sh:19` (`source_repository=${1:-...}`) treats `--help` as the source path, so the embedded Python fails with a traceback from `git -C /private/tmp/ht-pkg-tip/--help rev-parse`. The usage text exists only as the header comment (line 4).
- The evidence JSON does not record the installed or updated executable sha256. Since uninstall removes those files, the hashes of the installed binaries cannot be recovered after a run.
- The daemon is `degraded` and doctor reports `degraded` because native hooks are not installed here. That is expected, and the package doc's Limits section assigns it to ht-4is.11.x.
- These known limits still apply: the `--new-seat` fresh-seat semantics are not pinned by this gate (a documented surviving mutation), and there is no GitHub fetch or marketplace path.

## Cleanup
- Private root `/private/tmp/htpv-8jalxh8p` deleted after copying its artifacts.
- Detached worktree removed.
- No leftover private daemons or servers. Bead not closed.

## Evidence: /private/tmp/ht-pkg-tip-evidence
- `run.log`: full validator output and JSON summary (checks, startup/handoff/update data), ending with `exit=0`.
- `pre.txt` / `post.txt`: Herdr path, sha and version; shared server pid/start time before and after; shared `plugins.json` sha; plugin directory listing; leftover-process scan; linked binary sha256.
- `package-suite.log`: deterministic package test results.
- `private-root/`: private `server-{1,2,3}.out`, private `plugins.json`, and the private instance `daemon.log`.