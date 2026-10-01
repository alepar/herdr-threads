# Codex 0.159.3 sandbox default-deny probe (ht-4is.8.15)

Codex auto-updated from 0.159.2 to 0.159.3 on 2026-09-30. Its hooks are still admitted by schema fingerprint. The `config.toml` sandbox socket allowance was measured on 0.159.2 only, so `setup-status` and `doctor` warned that `network_access=true` was installed for an unmeasured version. This run repeats the no-model probe from `native-codex-demo-3/sandbox-socket-probe/` (archived in git tag `archive/herdr-threads-run-2026-09-26` under `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/`) on 0.159.3.

- Binary: `~/.local/bin/codex` -> `~/.codex/packages/standalone/releases/0.159.3-aarch64-apple-darwin/bin/codex`, `codex-cli 0.159.3`, sha256 `4d210f7c5a18fd0386434df23b5bdbb8c0e7257d3e8a2b30b0769c8bbe99a878`.
- No model was launched. Only `codex sandbox` ran, each time with a fresh scratch `CODEX_HOME`: `codex-home-empty`, or `codex-home-allowance` holding only the three allowance keys ([codex-home-allowance-config.toml](codex-home-allowance-config.toml)). The real `~/.codex` was never used as `CODEX_HOME` and nothing under it was written.
- Daemon: an isolated scratch daemon (`--state-dir $SCRATCH/state`, host endpoint a fake listening socket `$SCRATCH/fakehost.sock`, so the shared Herdr server was not involved). Its stable socket was `/private/tmp/herdr-threads-501/87d2592e90e0b831.sock`. It was stopped afterwards.
- Negative targets: another Unix socket we own (`neg.sock`), the Herdr server socket (connect then close only), a loopback TCP listener we own (`127.0.0.1:47812`), external TCP (`1.1.1.1:53`, `93.184.215.14:443`), and HTTPS through the proxy (`curl https://example.com`) and around it (`--noproxy '*'`).

Script: [run-probe.sh](run-probe.sh), with [conn.py](conn.py), [listen.py](listen.py) and [net.sh](net.sh) copied from demo 3. Full output: [probe-results.txt](probe-results.txt). Paths are redacted to `$SCRATCH`, `$TARGET` and `~`.

## Results

| Phase | Allowance | Daemon socket | Other targets | Proxied HTTPS | Product CLI `daemon health` |
| --- | --- | --- | --- | --- | --- |
| P0 unsandboxed | n/a | CONNECTED | all Unix and loopback CONNECTED (external TCP times out on this network) | 200 | rc=0 |
| P1 workspace-write | none | EPERM | all EPERM | DNS fails | rc=4 `transport_denied` |
| P2 workspace-write | `-c` overrides | CONNECTED | all EPERM | 403 (curl 56); `--noproxy` DNS fails | health returned |
| P3 workspace-write | `config.toml` only (user-level setup form) | CONNECTED | all EPERM | 403 (curl 56); `--noproxy` DNS fails (curl 6) | health returned |

The result matches 0.159.2 (demo 3) target for target. The proxy starts (`HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` set), only the allowlisted daemon socket connects, and all other network access stays denied. So 0.159.3 joins `CODEX_SANDBOX_MEASURED_VERSIONS` in `src/cli/setup.rs`.

## Longer-term: measure per binary identity

Exact version pins send a warning after every Codex auto-update, even a patch release, until someone repeats this probe. A follow-up design could measure the allowance per binary identity and cache the result, like the hook schema fingerprint (`src/harness/codex_schema.rs`, keyed by binary sha256). Setup or doctor would run this same no-model probe (`codex sandbox` with a scratch `CODEX_HOME` and its own listeners) against the installed binary, then admit the allowance for that binary hash only when every negative target is denied and the allowlisted socket connects. The cost is a slower first setup, plus scratch listeners and a short-lived daemon socket. A failed or unavailable probe must keep the current behavior: hooks only, allowance withheld, loud warning when one is already installed. This change does not do that. It keeps the exact-version gate and adds 0.159.3 on this evidence.
