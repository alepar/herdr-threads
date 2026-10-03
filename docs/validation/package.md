# Package installation validation

Status: **passing** (see Results). The platform is Herdr `0.9.1` (the pinned binary, sha256 `5fc7a7e7…c89de`) on macOS arm64. Run the gate with:

```sh
cargo test --locked --features test-support --test package install::clean_package_install_lifecycle -- --ignored --exact --nocapture
# or directly, optionally against another repository/ref:
scripts/validate-package.sh [SOURCE_REPOSITORY [REF]]
```

The gate is ignored by default. It performs four locked release builds (failed install, clean install, failed rebuild, update install) plus one local development build for the link step, so it takes several minutes. It needs the installed Rust toolchain and permission to bind Unix sockets under `/private/tmp`.

## Isolation

The validator validates only committed source. It pushes `REF` (default `HEAD`) as `main` to a private bare repository inside one fresh `/private/tmp/htpv-*` directory. It also creates two derived refs there: `broken` appends a `compile_error!` and `update` increments the package patch version in `Cargo.toml` and `Cargo.lock`. All Herdr commands then run with that directory as `HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_RUNTIME_DIR`, with a private `HERDR_CONFIG_PATH` and `HERDR_SOCKET_PATH`. Inherited `HERDR_*` and `GIT_*` variables are removed. `RUSTUP_HOME`/`CARGO_HOME` point to the user's toolchain and crate cache, which the build only uses. A private global git `url.file://…/repos/.insteadOf https://github.com/` rule makes `herdr plugin install alepar/herdr-threads` clone the private repository, so Herdr itself performs the checkout, runs the manifest build, and registers or rolls back the plugin, with the network remote replaced.

The validator starts a private `herdr server`, checks that it reports the private API socket, and stops it only by signalling that child process. To exercise the `[[startup]]` entry it restarts **only that private server** twice (after the clean install and after the update install): it signals the child it started, waits for it to exit, and launches `herdr server` again with the same private environment. Between those restarts it sends `herdr server live-handoff` to the private socket (the environment names only the private socket). Herdr then spawns a detached `herdr server --handoff-import` successor; the validator finds it by argv (`--handoff-import` plus the private config directory) and later stops it by signalling that pid. It never runs `herdr server stop`, and it never contacts, stops or restarts the shared server. Before and after the run, it snapshots the hash of the user's `~/.config/herdr/plugins.json` and the plugin directory listing; any change fails the gate. (`HT_PACKAGE_SHARED_HERDR_CONFIG` points that snapshot at another directory. It exists only for the demonstration in Results; the gate never sets it.) Daemons started from the private state are stopped by the `stop` action. A cleanup path also signals any that remain, matching only processes whose argv contains the private root. The private directory is deleted unless `HT_PACKAGE_KEEP=1`.

## Claims and the mutations that kill them

Every row below is a claim about the package. Each mutation listed was committed alone to a scratch clone of the final validator's commit (`52b0554`), run through that validator, and failed the gate (exit 1) inside that row's step, at the check named. Results lists the runs, including one mutation that survived.

| Step | Package claim asserted by the gate | Mutation → first failing check |
| --- | --- | --- |
| Failed build | `install --ref broken` exits nonzero with the compiler error and "Plugin was not installed."; no registration; no managed or temporary checkout left | `build.sh` without `set -e` (a failed cargo build still exits 0) → "failed build: install exits nonzero" |
| Clean install | exit 0; exactly one registration; `bin/herdr-threads` executable and reports the source version; the build ignored the inherited `CARGO_TARGET_DIR` (the validator exports a hostile one); action list = {health, doctor, ensure, stop, view} | `build.sh` without the `CARGO_TARGET_DIR` pin → "clean install exits zero" |
| Startup entry | after restarting only the private server, the one `startup` log for `herdr-threads` finished with exit 0 and a healthy/degraded `state`; exactly one daemon, whose argv is exactly `<installed>/bin/herdr-threads daemon run --state-dir <Herdr plugin state> --host-endpoint <private socket>` | startup `["./scripts/view.sh", "view"]` and startup `["./scripts/view.sh", "health"]` → "startup entry exits 0"; startup `["/usr/bin/true"]` → "startup started exactly one daemon"; `ensure` passing `$HERDR_PLUGIN_STATE_DIR/alt` → "startup daemon is the shipped executable with Herdr's state and socket" |
| Host/state context | the `ensure` action exits 0 with the startup daemon's boot id and still exactly one daemon; `health` shows the same boot; `doctor` exits 0 and shows the same state dir, socket (present), version and `daemon.version_matches: true` | `doctor` action passing `$HERDR_PLUGIN_STATE_DIR/alt` → "doctor action succeeds" (exit 3) |
| Live handoff | with the startup daemon serving, the rerun startup entry exits 0 with the **unchanged** boot id; the daemon set is still exactly `{startup pid: exact argv}`; `health` reaches the same boot; within 30 s the daemon reports a `last_reconciliation_at` later than the handoff (it has re-observed the successor) | `view.sh ensure` running `daemon stop` first when `HERDR_PLUGIN_EVENT=startup` → "handoff keeps exactly the startup daemon" (a new daemon pid; the fix2 run of this mutation failed one check earlier, at "handoff startup reuses the serving daemon", so which of the two fires depends on shutdown timing); `IdentityRepair::capture_if_due` capturing the host only once per daemon boot → "daemon reconciles against the handoff successor" |
| Mail data | after the handoff, in a private workspace pane: operator `seat resolve --new-seat` returns a seat, cooperative `check-in` exits 0, and `thread create --topic <unique>` returns a thread, all through the shipped CLI, which finds the daemon from Herdr's plugin environment (`HERDR_PLUGIN_STATE_DIR`, `HERDR_SOCKET_PATH`). The gate does **not** show that `--new-seat` creates a fresh seat rather than resolving one (see the surviving mutation in Results) | the CLI reading `HERDR_THREADS_STATE_DIR` instead of `HERDR_PLUGIN_STATE_DIR` → the operator `seat resolve` call (exit 2, `state directory missing`; a `run` failure, not a named check) |
| Operator view | the `view` action opens the `operator` pane through Herdr; the pane renders the thread topic and the prompt; it stays open after 3 s; `x`+Enter prints the hint and stays open; Enter re-renders; `q`+Enter closes the pane | `view.sh view` exiting after one render → the step's first pane wait (the pane closed without showing the prompt; a `fail`, not a named check) |
| Failed rebuild | `install --ref broken` over the working install exits nonzero; registry byte-identical; installed executable sha256 unchanged; the daemon still serves the same boot | `build.sh` reusing the installed executable when cargo fails → "failed rebuild exits nonzero" |
| Source update | `install --ref update` while the old daemon runs gives a new executable reporting the new version. After restarting only the private server, the startup log exits 3 with `(daemon_version_mismatch)` naming the old version and the old process stays the only daemon (no second writer). `ensure` exits 3 the same way and starts no daemon; `doctor` exits 3 with `result: daemon_version_mismatch`. `stop` exits 0 with `stop_accepted` and the **old** boot id, and the old process exits. `ensure` then starts the new version with the same instance id and a new boot, and the thread and seat are still listed | handshake version check disabled → "updated startup reports the version mismatch" (the startup `ensure` exited 0 against the old daemon); `stop` action wired to `daemon health` → "updated stop reaches the older owner"; database file name scoped by package version → "thread survives the source update" |
| Local link | clone the published ref and `herdr plugin link` it **before** building: `health` and `ensure` fail loudly (exit neither 0 nor 3, stderr names `bin/herdr-threads`) and start no daemon. After `scripts/build.sh` in the checkout (the hostile `CARGO_TARGET_DIR` still exported): the build ignored it; `ensure` exits 0 with exactly one daemon whose argv is `<checkout>/bin/herdr-threads daemon run --state-dir <plugin state> --host-endpoint <private socket>` and the same instance id as before; `health` reaches the same boot; the thread written before the uninstall is readable; `stop` exits the daemon | `view.sh` reporting a missing executable as "not running" (exit 3) → "unbuilt link fails loudly" (exit 3); instance directory keyed by the executable path as well as the host socket → "linked daemon keeps the instance" |
| Shared state | the shared registry hash and plugin directory listing are unchanged | the watched registry rewritten during a run (the `HT_PACKAGE_SHARED_HERDR_CONFIG` demonstration) → "shared Herdr registry and plugin directories unchanged" |

**Preconditions, not claims.** The gate also stops on the following, so that the claims above are attributable. They assert Herdr's own behaviour or the fixture, or the state a later claim needs. This document claims nothing about them, and none of the mutation runs in Results failed at one:
- the private server (and each restarted one) owns the private socket; the private registry starts empty; the update fixture edits the locked root package;
- Herdr resolves the published commit and registers the plugin enabled under the private config; `health` exits 3 right after registration (no daemon exists before the first restart);
- each server start or handoff logs exactly one `startup` entry; the handoff leaves exactly one private `--handoff-import` successor, which answers on the private socket, and the original server exits within 15 s (a `fail`, not a named check);
- the update install replaces the single registration and leaves the older daemon running;
- after the update step, the `stop` action exits 0 with `stop_accepted`, the daemon exits, `health` exits 3 and no private daemon remains (the link step needs a stopped daemon); `herdr plugin link` registers the checkout in place and does not build.

`herdr plugin uninstall` (before the link step) and `herdr plugin unlink` (at the end) run as transitions and must exit 0; the gate asserts nothing else about them.

The deterministic `package` suite covers the rest of the package surface without Herdr:
- `manifest::manifest_matches_pinned_091_documented_argv_shape` pins the literal manifest commands, including the startup entry `["./scripts/view.sh", "ensure"]`. The gate checks what the startup entry does, not its literal text.
- `manifest::build_ignores_inherited_cargo_target_dir` kills a build that follows an inherited `CARGO_TARGET_DIR`. Without the fix it installs a stale `target/release` binary.
- Other tests cover the argv shape of each action, a failed build preserving the prior artifact, quoting, and the view loop.
Results lists the mutation run for the first test; the second was demonstrated in the original round (commit `1eaf06f`).

## Limits

- The install source is a private `file://` repository behind an `insteadOf` rule, not GitHub. The GitHub fetch itself, marketplace indexing and `--ref` resolution of remote tags are not exercised.
- Native harness hooks are not installed here, so the daemon reports `degraded` and `doctor` reports `result: degraded`. Hook readiness is owned by the native validation beads (ht-4is.11.x).
- Server starts. The gate starts the startup entry in two ways only: a clean SIGTERM restart of its private server (twice) and one `herdr server live-handoff`. A server started implicitly by an attaching client, a restart after an unclean server crash, and `herdr update --handoff` with a real updated executable are not exercised. From the Herdr 0.9.1 source, not from the gate: both a normal server start and the handoff-import server (`run_handoff_import_server` in `src/server/headless/bootstrap.rs`, which calls `server.app.run_plugin_startup_hooks()` after `report_owned`) run every enabled plugin's startup commands, and `herdr update --handoff` sends the same `server.live_handoff` request with the updated executable as `import_exe`.
- Live handoff and an existing daemon. The gate exercises exactly one handoff, before any package update, with the startup daemon serving, and asserts only the Live handoff row. It does not exercise a handoff after a package update. It does not assert that host-backed calls fail during the window before the daemon re-observes the successor; the adapter source (`check_context`/`check_boot` in `src/host/native.rs`, `stale_host_observation`) is why the gate waits for a reconciliation before its first host-backed call. It asserts nothing about seats that existed before a handoff. Seat continuity across a handoff belongs to identity recovery (the seat-identity design and adopted decision (b) of the wave-2 fix1 adoption), not to this package.
- The linked development checkout is cloned from the published private ref; linking a dirty, uncommitted working tree is not exercised.
- Only macOS arm64 was exercised.

## Results (2026-09-29)

### Claim audit (adopted item (c) of the wave-2 fix2 adoption), validator `52b0554`

- Gate `install::clean_package_install_lifecycle` on source commit `52b0554` (run through `cargo test --locked --all-features --test package ... --ignored --exact --test-threads=1`): **passed**, 1 test, 265.99 s, `PACKAGE_VALIDATION_PASS`, **89** named checks. The handoff rerun exited 0 with the unchanged boot id, the daemon set was unchanged, and the daemon reconciled against the successor 3115 ms after the handoff. The updated startup entry exited 3.
- The count fell from 95 to 89. Six named checks were removed: the one that could never fail ("live handoff: the original private server exited", reached only after `wait` had returned, so `returncode` was always set; a server that does not exit now fails the gate through an explicit precondition `fail`), the literal startup-command check (pinned by the manifest test instead), and four uninstall/unlink checks that assert only Herdr behaviour no package mutation reaches.
- Mutation runs. Each mutation was committed alone to a scratch clone of `52b0554` and run through the `52b0554` validator. The table lists the exit code and the check at which each run failed.

| Row | Mutation | Exit | First failing check | Seconds |
| --- | --- | --- | --- | --- |
| Failed build | `build.sh` with `set -u` instead of `set -eu` | 1 | failed build: install exits nonzero (Herdr printed "Installed herdr-threads") | 78 |
| Clean install | `build.sh` without the `CARGO_TARGET_DIR` pin | 1 | clean install exits zero | 150 |
| Startup entry | startup `["./scripts/view.sh", "view"]` | 1 | startup entry exits 0 (exit 3) | 190 |
| Startup entry | startup `["./scripts/view.sh", "health"]` | 1 | startup entry exits 0 (exit 3) | 192 |
| Startup entry | startup `["/usr/bin/true"]` | 1 | startup started exactly one daemon (none) | 189 |
| Startup entry | `ensure` passing `$HERDR_PLUGIN_STATE_DIR/alt` | 1 | startup daemon is the shipped executable with Herdr's state and socket | 187 |
| Host/state context | `doctor` passing `$HERDR_PLUGIN_STATE_DIR/alt` | 1 | doctor action succeeds (exit 3) | 168 |
| Live handoff | startup `ensure` runs `daemon stop` first | 1 | handoff keeps exactly the startup daemon (new pid) | 181 |
| Live handoff | `capture_if_due` captures once per daemon boot | 1 | daemon reconciles against the handoff successor | 216 |
| Mail data | CLI reads `HERDR_THREADS_STATE_DIR` | 1 | operator `seat resolve` exited 2, `state directory missing` | 195 |
| Mail data | `seat resolve --new-seat` dispatched as plain `resolve` | **0** | none: **survived**, `PACKAGE_VALIDATION_PASS`, 89 checks | 464 |
| Operator view | `view.sh view` exits after one render | 1 | first pane wait (pane closed, prompt never shown) | 207 |
| Failed rebuild | `build.sh` reuses the installed executable when cargo fails | 1 | failed rebuild exits nonzero | 304 |
| Source update | handshake version check disabled | 1 | updated startup reports the version mismatch (exit 0) | 399 |
| Source update | `stop` wired to `daemon health` | 1 | updated stop reaches the older owner | 387 |
| Source update | database file name scoped by package version | 1 | thread survives the source update | 400 |
| Local link | `view.sh` reports a missing executable as not running (exit 3) | 1 | unbuilt link fails loudly | 401 |
| Local link | instance directory keyed by executable path and host socket | 1 | linked daemon keeps the instance | 506 |
| Shared state | unmutated source; `HT_PACKAGE_SHARED_HERDR_CONFIG` pointed at a scratch directory whose `plugins.json` a background job rewrote 90 s into the run | 1 | shared Herdr registry and plugin directories unchanged (every earlier check passed) | 506 |

- The surviving mutation means the gate does not pin `--new-seat` semantics: on an unbound pane a plain `resolve` also returns a seat. The Mail data row therefore claims only that the call returns a seat. Fresh-seat semantics belong to the seat tests, not this package gate.
- Deterministic manifest test: with the startup entry changed to `["./scripts/view.sh", "health"]`, `manifest::manifest_matches_pinned_091_documented_argv_shape` failed (`left: ["./scripts/view.sh","health"]`, `right: ["./scripts/view.sh","ensure"]`).
- An earlier round of the same mutations on validator `e09ed2a` (identical except that its reconciliation probe found the daemon through `HERDR_PLUGIN_STATE_DIR`) gave the same outcome for all 18 it ran: 17 failed at the same check as above, and the `seat resolve --new-seat` mutation survived there too. Because of that survivor, the probe was changed so that the Mail data row has its own kill, and every mutation was rerun on `52b0554`.
- The runs went four at a time. After every run no `htpv-*` directory, private daemon, private server or `--handoff-import` successor remained. The shared `~/.config/herdr/plugins.json` sha256 prefix `0e2c5ef0` was the same before and after the gate.

### Earlier rounds

These runs used earlier validators. The claims they supported are demonstrated again above on `52b0554`.
- Live-handoff coverage (validator `8151b73`): gate passed, 95 checks, 245.46 s. The capture-once mutation passed the pre-handoff validator `66fafc7`, so the handoff step is the only gate step that kills it.
- Startup and link coverage (validator `8087583`): gate passed, 86 checks, 318.19 s.
- Original package gate (validator `1eaf06f`): gate passed, 60 checks, 417.41 s. The same validator on base `08c55ca`, before the `build.sh` target-dir pin, failed at "clean install exits zero": cargo built into the inherited target dir, then `cp: target/release/herdr-threads: No such file or directory`, and Herdr printed "Plugin was not installed." `manifest::build_ignores_inherited_cargo_target_dir` failed before the `build.sh` fix and passes after it.
