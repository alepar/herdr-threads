# Configuration smoke (ht-p03.21)

Tree: `46f25709` (task branch, ht-p03.1 isolated Herdr fixture merged in). Host: macOS arm64, herdr 0.9.1,
claude 2.1.287, codex 0.159.3. Every Herdr state ran in an isolated named Herdr session
(`test_support::isolated_herdr`), never the shared server. This record classifies outcomes; it fixes no bucket behaviour.

Command: `HT_CONFIG_SMOKE_OUT=<dir> nice cargo test --locked --all-features --test integration config_smoke -- --test-threads=1`
Result: `5 passed; 0 failed`. The tests assert only that each command completes with an exit code (all exited 0);
the classification below reads the captured `daemon health --json` / `doctor --json` output.

## Release targets

| Target | Outcome |
|---|---|
| `aarch64-apple-darwin` | PASS: `cargo build --locked --release --target aarch64-apple-darwin` finished in 2m00s. Full serial suite on the host: FAIL (3 tests), see "Full suite" below. |
| `x86_64-apple-darwin` | PASS: `cargo build --locked --release --target x86_64-apple-darwin` (cross) finished in 1m31s. |
| `x86_64-unknown-linux-musl` | NOT_EXERCISED: Docker CLI is installed but its daemon is not running (`failed to connect to the docker API at unix://~/.docker/run/docker.sock`), and starting Docker Desktop is outside this task. Follow-on: first CI run (`build-targets` job). |
| `aarch64-unknown-linux-musl` | NOT_EXERCISED: same reason. Follow-on: first CI run. |

Both Apple builds emit one pre-existing `dead_code` warning in the lib (not caused by this task).

## Herdr up/down states (ensure + Health + doctor, isolated session)

All three commands exited 0 in every state. Health `state` is `degraded` in every state, including the healthy
one, because of the harness limitations `harness claude: claude 2.1.287: optimistic: assumed recipe claude-hooks-2.1.283`
and `harness codex unknown: the installed version has not been observed yet`.

| State | Outcome | Evidence (health / doctor) |
|---|---|---|
| Herdr up, plugin linked | PASS | `host.reachability=ready`; limitations are only the two harness lines above. |
| Herdr stopped while daemon runs | PASS with observation | Health and doctor completed. Right after `h.stop()` (socket removed, doctor `host_endpoint_present:false`) both still reported `host_reachability: ready` with only the harness limitations: the daemon's host view had not yet degraded. Whether this is scheduler-tick lag or a Health gap is not established here; B3 Health degraded wording (ht-p03.9 family) owns the check. No bead filed: a single immediate read cannot distinguish the two. |
| Herdr never started (no socket) | PASS | `host.reachability=unavailable`; `host unavailable: latest host capture failed: StaleHostObservation: host endpoint witness unavailable: EndpointUnavailable`, `scheduler degraded: host observation invalidated: HostUnavailable`; doctor `host_endpoint_present:false`. |
| Stale socket file, server dead | PASS | `host.reachability=unavailable`; `host unavailable: latest host capture failed: HostUnavailable: host API transport: Connection refused (os error 61)`; doctor `host_endpoint_present:true`. Distinct from never-started wording by the transport error. |
| Herdr restarted under a running daemon | PASS | After `h.restart()` (starts=2, state up) health and doctor report `host_reachability: ready`, harness limitations only. Recovery-to-normal cadence is B2's, not measured here. |

## Harness version set

Not exercised here.

| Row | Owner |
|---|---|
| Claude Code 2.1.286 (live + canary tier 0) | live: ht-p03.20; canary tier 0: ht-p03.14.x |
| Claude Code 2.1.283, 2.1.284, 2.1.285 | canary tier 0: ht-p03.14.x |
| Claude Code newest published > 2.1.286 (must classify `Optimistic`) | admission table: ht-p03.13; canary tier 0: ht-p03.14.x |
| Codex 0.159.3 (live + canary tier 0) | live: ht-p03.20; canary tier 0: ht-p03.14.x |
| Codex 0.157.1, 0.158.0, 0.159.2 | canary tier 0: ht-p03.14.x |
| Codex newest published > 0.159.3 | canary tier 0: ht-p03.14.x |
| Either, older than every recipe (`Refused`) | admission table test: ht-p03.13 |

Note: the live run shows `claude 2.1.287: optimistic: assumed recipe claude-hooks-2.1.283`, so the installed
Claude (unlisted) is admitted optimistically today, as the spec expects.

## Native matrix

Owned elsewhere: early per-cell run ht-p03.38; on-tip rerun ht-p03.20.

## Installer x Herdr states

Owned elsewhere: ht-p03.31 (`install_test.sh`).

## Full suite (aarch64-apple-darwin host)

Command: `nice cargo test --locked --all-features --no-fail-fast --tests -- --test-threads=1` (debug profile, host arm64).

| Target | Result |
|---|---|
| lib unit tests | 1304 passed, 0 failed, 16 ignored |
| contracts / hook_entrypoint / host_adapter / integration / lifecycle_ux / local_endpoint / package / service / view | all ok (integration: 17 passed, including the 5 `config_smoke` tests) |
| `setup_cli` | FAIL: 16 passed, 3 failed |

Failing: `unsupported_harness_version_is_refused_with_status_four` (exit 0, expected 4), `codex_setup_installs_user_hooks_and_unsetup_restores_bytes` (exit 0, expected 4, line 584),
`bare_setup_skips_missing_and_reports_refused_harnesses` (line 1124: claude now `installed: claude 2.1.287 (recipe claude-hooks-2.1.283)`).
Cause: these tests depend on the host's installed Claude being refused, but 2.1.287 is now admitted optimistically. This is harness-admission behaviour,
not caused by this task; filed as a comment on the owning bead **ht-p03.13** (admission table test; the host-dependent tests should move to fake-version fixtures).
Doc tests were not run (`--tests`); a separate `--doc` run is left to the end-of-epic suite.
