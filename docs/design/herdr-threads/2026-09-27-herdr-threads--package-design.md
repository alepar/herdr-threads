# Native Herdr package and release preparation

## Goal

Produce an installable native Herdr plugin with working runtime entrypoints, a compact operator view and accurate setup/recovery/release instructions, ready for public repository publication after integration.

Parent: [root design](2026-09-27-herdr-threads-design.md). Bead: ht-4is.10. Earlier siblings: caller-attribution, store, seat-identity, scheduler, daemon, CLI and harness designs. Root ht-4is.9 owns the composed executable.

## Native package

Use herdr-plugin.toml id `herdr-threads`, name `Threads`, initial version `0.1.0` and min_herdr_version `0.9.1`, subject to actual native validation before release. Initially declare macos only, since the working host is Darwin arm64 and no Linux native evidence exists yet. Document Unix-oriented implementation separately from supported platforms; never infer Linux/Windows support from compilation alone.

The manifest build entry calls a repository build script with argv-array syntax. The script performs `cargo build --release --locked`, creates bin/, and atomically installs the resulting single executable at bin/herdr-threads. Source checkout retains Cargo.lock, scripts and manifest, not durable state. Runtime entries invoke that built executable or small checked-in wrappers. Build steps require a documented Rust toolchain and do not rely on Herdr runtime variables, which are absent during install build.

Startup invokes daemon ensure once and exits. Actions expose health/doctor, ensure, stop and open the operator view. A manifest pane entrypoint opens the compact view. Its small wrapper renders `view --once`, then waits for Enter to refresh or q to exit so a short-lived popup does not disappear before it can be read; this is a line-oriented wrapper, not a custom navigation framework. It invokes only bounded read queries and preserves native terminal behavior. No pane in the operator view is treated as an agent seat merely because it was opened by the plugin.

Avoid event-command process storms: production daemon uses the host adapter subscription plus periodic snapshots, so no per-message manifest event spawning is required. Never copy the prior mail plugin's pane.exited retirement behavior: process exit is occupant unavailable, pane close is seat retirement. No shell environment placeholders in manifest argv; runtime wrappers read env explicitly and quote paths.

## Installation and activation behavior

Clean installation builds and validates the manifest in a temporary isolated Herdr configuration, then verifies action list and pane entrypoint against the installed binary. Local development link happens only after building, because link does not run build commands. Linking/enabling alone may not run startup; docs instruct explicit ensure/doctor and supported harness setup. Never infer daemon/hook readiness from registration success.

Runtime inherits HERDR_BIN_PATH, HERDR_SOCKET_PATH and plugin state/config/root paths. CLI invocation outside a plugin action uses documented explicit context/environment; installation must provide an agent-usable absolute executable path or clearly documented PATH setup without editing global shell configuration. Harness setup records that executable path through its owned config helper. Reinstall does not replace state. Updating the binary while a daemon is running reports a version/boot mismatch and instructs explicit stop/ensure; it does not silently run two incompatible writers or migrate a live database concurrently.

Uninstall/disable does not imply shutdown because Herdr has no shutdown hook. Document stop, remove owned harness entries, then unlink/uninstall. Preserve mail state by default. Do not provide an automatic data deletion command in this release. Capture clean-install and build-failure behavior without changing global hooks, existing plugin registrations or unrelated sessions.

## Documentation and release artifacts

README supplies the brief purpose, exact receipt semantics and quick start: resolve empty seat, create/invite, send with explicit invited recipient, launch, read/ACK/accept. Include manual-launch equivalent, topic update/archive and optional cheap summary delegation. Install docs cover prerequisites, scoped hook setup, configuration/deadline overrides, CLI discovery and clean rollback. Operations docs cover health, unknown send outcome/retry, deadlines, daemon crash, offline seat, retired versus left, ambiguous rebind and native input race.

Compatibility documentation lists exact validated Herdr/Codex/Claude and platform versions plus fixture support limits. Evidence links distinguish deterministic tests from native receipt observations. State no inference-interruption guarantee or measured savings percentage without matched data. Diagnostics redact credentials and native transcript content.

A build/test CI workflow runs the Rust formatting, lint and deterministic suites on the supported platform and validates manifest structure/artifact paths. Native model tests require a local authenticated isolated environment and are a separate documented script; CI must not fake their result. Include a release checklist with reproducible build, reviewed SHA, validation report, archive/install contents and marketplace requirements.

Public follow-on: push the approved repository to alepar/herdr-threads, make it public if appropriate, add `herdr-plugin` topic and retain parseable manifest on the indexed branch for herdr.dev/plugins/. These are prepared instructions, not remote publication during this epic. No marketing or separate website is required.

## Decomposition and validation

Split manifest/build/runtime/operator wrappers, installation/operations/release docs with CI, and isolated clean-install verification. Manifest and docs consume established command contracts; install verification consumes actual composed executable/package and proves install failure does not register a broken plugin. The validation subtree independently tests native communication through these packaged entrypoints.

Acceptance: every manifest command exists and is executable, locked build from clean checkout succeeds, runtime uses correct state/host context, operator view remains readable until closed, startup exits, stop works, scoped hook install/remove preserves unrelated configuration, source reinstall retains state, failed build aborts registration, and public prerequisites are accurately documented. Native installation tests use private config/server state only.

## Final support reconciliation

ht-4is.10.4 consumes the completed validation support matrix and initial manifest/docs, then reconciles their release-facing claims. Initial packaging remains runnable before native validation. Final reconciliation verifies exact versions, platforms and evidence links and blocks release for unsupported required configurations. Runtime/build-affecting corrections require refreshed affected validation; documentation alone never grants support.

## Revised public recovery and query contracts

Documentation and packaged command help must cover bounded pagination for every collection, exact returned continuation arguments, `pending-receipts`, search pages that make progress with zero matches, and explicit cursor-stale restarts. Explain that `read --recent N` selects the first descending page, with continuation available for older history. Topics and compact startup statistics remain required recovery data; optional summaries use a separately requested inexpensive subagent reading the public API.

Document the three explicit local administrator actions: `seat rebind SEAT --pane ADDRESS --operator`, orphan-only `invite THREAD --seat SEAT --operator`, and `seat resolve --pane ADDRESS --new-seat --operator`. The daemon checks the kernel peer UID against its owner; this identifies the local account, not a human. Administrative mode grants no ACK, acceptance, message-send or checkpoint authority. Recovery-baseline holds and the deliberate fresh-role choice must be visible in the compact view and doctor output. An ordinary empty-pane resolve remains possible before launching its future recipient.

Operations instructions distinguish transaction decision time from physical commit, UTC warning deadlines from monotonic wake spacing, and safe unregistered recovery hints from verified registration. Explain the immediate terminal retirement fence and frozen cutover, effective status/orphan recovery before cleanup, bounded boot-resumed cleanup and seat-inspect pending/complete/error fields. Cutover-owed warning/audit history materializes incrementally; retirement warnings have already-settled conditions and add no new wake fanout. Preserve all history without a backlog cap. Explain how to run ensure when the unsupervised daemon is stopped. Keep the native observation/commit race and uncertain prompt delivery limits explicit; neither an RPC response nor paste+Enter establishes model receipt. Required native recovery or freshness failures block corresponding release support claims.

The restore-allocation clarification responds to the still-open F6 escalation and must be carried into the next design review. Packaging does not declare that prior dispute resolved.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
