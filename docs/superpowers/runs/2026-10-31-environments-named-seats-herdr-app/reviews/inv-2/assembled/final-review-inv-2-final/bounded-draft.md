# Final review (bounded): epic ht-kqrt on super-auto/environments-named-seats-herdr-app, invocation inv-2

Captured source: base f4ddaeffa9fa13549f514669391e84a7fc8d5e5f, head 04df0e77a843fdd5ff35fea77dfc77b10162a414 (both verified with `git rev-parse --verify`). The inv-2 review source was bound to the ledger before any artifact write (bind-review-source exit 0).

## Scope and route

Bounded captured-source review, not a whole-range review. The sidecar `assembled/final-review-inv-2-final/review-inputs.json` is readable, its base/head match, sourceAvailable true, wholeRangeReady false, status unavailable, 197 changed paths, 0 omitted, 0 slices, 4 provenance gaps (listed under Untested scope). No whole-range package was built or loaded; the failed assembly outputs and the original sidecar are untouched. A whole-range review is still outstanding.

Range delta since inv-1: head 04df0e77a adds exactly one commit over inv-1's head 124759cc8 ("Phase 3 invocation inv-1 settled"), and `git diff 124759cc8 04df0e77a` touches only `docs/superpowers/runs/2026-10-31-environments-named-seats-herdr-app/**` (inv-1 review archive, profile, friction, run.md). No source, test, migration or script changed since inv-1, so every code finding below is checked against the same code inv-1 saw. I re-derived each one from captured Git myself rather than copying inv-1's draft.

What I read: the full changed-path inventory (197 paths, 50279+/406-), the full-range hunks of the 19 product paths listed in bounded-coverage.json (store hosts/schema/queries, migration 0012, host_sessions, dispatch, ports, protocol commands/handoff/capabilities/results, cli site_capability/doctor, the Claude/agy/adapter composer `screen_blocked` seam, hc `wire.rs`, fake_herdr, the fake-hc binary), the end of `src/test_support/fake_hc.rs` (start/listen/halt/rebind/drop), the hc 0.1.15 OpenRPC schema's Status/Foreground/Terminal/HelloResult/CoreInfo nullability (to check `wire.rs`), main's `src/store/schema.rs` NODE_DELTAS, the epic bead tree (`bd list --parent ht-kqrt --all`: 93 children, 35 closed, 58 open), and the ledger. I ran no cargo build, clippy, check-default-features or tests. check-harness-neutral and check-wire-compat were run by inv-1 on code identical to this head and are not rerun here.

## Step 1: my own findings

### Epic completeness (blocking)

- 58 of 93 children of ht-kqrt are open: every named-seat leaf, tmux wake, every hc host leaf, launch/handoff through CreateTargetPort, the phase gates, the host-neutrality scan ht-kqrt.65 and the integration sweep ht-kqrt.66. The seams on the branch (HostKind, WakeCapability, CreateTargetPort, `attach_builtin`, `caller_location`, the hc RPC client and wire types, migration 0012, the launch-gate command) have no production consumer. `Command::AuthorizeLaunchGate` is answered `Unsupported` and `LAUNCH_GATE_V1` is not advertised. The branch cannot land as the completed epic.
- ht-kqrt.6, .9, .11 and .14 are closed but their task branches are not in head (the ledger profile lists them as dispatched and never landed; .14's merge was held for a missing worktree). Their closed status overstates delivery.

### Migration collision with main (blocking, data-shape)

- Head's `NODE_DELTAS` ends with `version: 12, sql: 0012_host_environments.sql, rebuild: None`. Main (25 commits ahead, `version = "2.8.2"`) ends with its own `version: 12, sql: 0012_process_continuity.sql, rebuild: Some(copy_allocation_decisions + 0012_process_continuity_after.sql)`. After the main merge two deltas claim user_version 12. A store already at 12 from a 2.8.2 install never gets `herdr_hosts.kind`/`kind_probed_at`, and `attach_host` and `read_row` (`ROW_COLUMNS` now selects `kind,kind_probed_at`) then fail on every host attach and remembered-row read, i.e. the daemon attaches no Herdr server. The host-environments delta has to be renumbered after main's (13), its `ALTER TABLE seats ADD COLUMN name ...` / `seats_live_name` and `occupant_bindings` columns checked against main's seats rebuild, and a migration test added from a 2.8.2-shaped store.

### Cross-task seam: the built-in host stored and swept as Herdr (ht-kqrt.4 x ht-kqrt.5)

- `HostSessions::attach_builtin` persists through `store.attach_host`, whose upsert hardcodes `kind='herdr', kind_probed_at=now` (`HostKind::stored_name` exists but is unused there). Consequences once ht-kqrt.15 wires `attach_builtin` at daemon start: (a) the named host row is kind `herdr`, so `herdr_hosts_one_named` never applies and `HostRow::attachable()` treats it as dialable; (b) `sweep_tick` filters against `attached()`, which excludes the built-in, so each tick re-picks that row and probes `factory.exists` on its locator; (c) `attach_remembered_live` iterates most recent first, `attach_builtin` just stamped `last_attached_at`, `by_locator` returns the built-in, so `first` becomes the built-in's id; `startup_primary` resolves it with `get()`, which reads only `sessions`, and silently falls back to the lowest `host_id` instead of the most recently attached Herdr server. That primary feeds Health, archival and the advertised bootstrap capability. The unit tests (`attach_builtin_is_never_in_attached`, `builtin_found_by_locator_and_resolve`) cover neither the stored kind nor startup ordering with a remembered built-in row.

### Composer `screen_blocked` fallback (ht-kqrt.37, latent product defect)

- Claude's and agy's `screen_blocked` return `Some(true)` only for the captured approval layouts (`Esc to cancel` + `❯ N.`; `↑/↓ Navigate` + `> N.` within the last 8 non-empty rows). Any other readable screen returns `Some(false)`, which the trait defines as "a recognized idle or busy composer". An uncaptured question dialog (AskUserQuestion) or any dialog with a different footer is read as typeable, and the planned consumer (tmux composer wake, ht-kqrt.69) would type a wake that answers the dialog. No production caller exists at head; the consumer must not merge on this fallback.

### hc wire types (ht-kqrt.75), checked

- `wire.rs` matches the 0.1.15 schema's nullability: every schema `[T, null]` field is an `Option`, and every non-Option `String`/integer field is non-nullable in the schema (Status.state/app/msg/title, Foreground.*, Terminal.address/state/title/created_ms/turn). The module comment "Every field a reply may omit or null carries `#[serde(default)]`" overstates what `default` does: it tolerates omission, not an explicit `null`, so a future core that nulls e.g. `Terminal.title` would fail the whole record's decode. Fine for 0.1.15; note for the phase that consumes it.
- `fake-hc` binary: its doc says it exits on SIGTERM; it installs no handler, so SIGTERM kills it with the socket file left behind (cleanup happens only on the stdin-EOF path). `FakeHc` (in-process) uses `scratch::temp_path` and removes its socket on drop; its per-connection threads are not joined on halt.

### Smaller observations

- `src/cli/doctor.rs` limitation line formats serde_json `Value`s (`host["endpoint"]`, `host["kind"]`), so strings keep JSON quotes (`kind '"future"'`). The test checks `contains` only.
- `classify_wake_rows` (`src/service/dispatch.rs`) is passed `wake_suppressed && !stalled` inline while its doc names `ModChannels::mod_suppressed`; two copies of one formula.
- `SeatsQuery.host` scoping (`src/store/queries.rs`) runs one extra `seat_on_host` query per scanned row and skips rows like a directory mismatch; correct, and the cursor digest includes the host so a scoped cursor cannot be reused unscoped. Pane-site bytes are pinned unchanged by tests.
- `unattachable_hosts_report` opens the store read-only from the CLI for diagnostics only; on a pre-0012 store the `kind` select fails and it reports nothing, which is the documented behaviour.

## Step 2: ledger triage

- Recurring minor `[missing-evidence]` / `[stale-evidence]` (ht-kqrt.2, .3, .10, .12, .53, .75, .8 and others): a reporting-process cluster. Its effect is untested scope (task clippy/default-features evidence pre-rebase or not sourceTree-bound), not a code finding.
- ht-kqrt.37 `[missing-question-screen]`: misgraded as minor; it names a known product-defect path once consumed. Must fix before the tmux wake consumer merges (see above).
- ht-kqrt.10 `[hook-gate-behavior-change]`: `HookEnv.pane` now requires `HERDR_ENV=1`; stricter, and real Herdr panes set it. Deferred OK; pin with a test.
- ht-kqrt.12 `[deferred-wiring]`: no CLI sets `host`/`caller_host` yet; planned for the site phases. Deferred OK.
- ht-kqrt.76 `[unverified-live-path]`: the real `hc run --bg` / `server.stop` path of `scripts/lib/isolated-core.sh` and its shellcheck were never exercised. Untested scope.
- Other deferred minors (ht-kqrt.53 wire-compat comment and visibility widening; .7 visibility drift; .5 missing edge-case test and unverified claim; .8 wire form; .75 comment placement; .76 process-match and leak-rule scope; .10 empty-value filtering; .12 test location; .1 trailer attribution): Deferred OK.
- Parked: 0. BLOCKED-AUTH: none. Since inv-1 the ledger gained only the inv-2 launch, review-source binding, profile and metrics lines; no new findings.
- Metrics: inv-2's ledger-check reports ok, but inv-1's recorded `landed≠completed 13 vs 8` and the read-ledger gaps (malformed `# ledger-digest:` header, Git landing lookup output withheld, ledger/Git identities differ) mean the completion record is incomplete; absence of findings there is not evidence of a clean range.

## Verdict
not ready (epic 58/93 children open; v12 migration collides with main's v12; built-in host stored and swept as Herdr; bounded review, no full-suite run on head)

## Must fix before landing
- Renumber `migrations/node/0012_host_environments.sql` / `NodeDelta { version: 12 }` after main's own version-12 delta (`0012_process_continuity.sql`, seats rebuild) when merging main; verify the rebuild keeps `seats.name`, `seats_live_name`, `occupant_bindings.locator_hint`/`readonly`, and add a migration test from a 2.8.2-shaped store.
- Merge main (25 commits ahead, v2.8.2) into the run branch and resolve version, CHANGELOG and schema conflicts before landing.
- The epic goal is not delivered: 58 of 93 children are open, including every named, tmux and hc leaf, the phase gates, ht-kqrt.65 and the integration sweep ht-kqrt.66; do not land this as the completed epic.
- Merge or reopen ht-kqrt.6, .9, .11 and .14: closed, but their branches are not in head (.14's merge held for a missing worktree).
- Make `attach_host` write the attaching host's kind instead of hardcoding `'herdr'`, and keep the built-in row out of `attach_remembered_live`/`startup_first` and `sweep_tick`, before ht-kqrt.15 wires `attach_builtin`; otherwise the startup primary falls back to the lowest host_id and the sweep re-probes the named row every tick.
- Claude/agy `screen_blocked` returns `Some(false)` (typeable) for any readable uncaptured dialog such as AskUserQuestion; add captures and recognizers or return `None` for unrecognized selection dialogs before the tmux composer wake (ht-kqrt.69) consumes it (ht-kqrt.37 minor misgraded).

## Untested scope
- No full-suite measurement of head 04df0e77a exists; the sweep is deferred to the caller, so the branch is not tested as merged.
- This review ran no cargo build, clippy, check-default-features or tests; check-harness-neutral and check-wire-compat ran only in inv-1 on code-identical head 124759cc8.
- Task evidence for clippy and check-default-features is pre-rebase or not sourceTree-bound for most tasks (recurring `[missing-evidence]` cluster).
- The real `hc run --bg` / `server.stop` path of `scripts/lib/isolated-core.sh` and its shellcheck test were never exercised.
- No Claude/agy question-screen capture exists, so `screen_blocked` behaviour on those screens is unknown.
- The hook pane gate change (`HookEnv.pane` requires `HERDR_ENV=1`) is not pinned for `pane_context_journal` and `record_observer_reset`.
- No test migrates a 2.8.2-shaped (main v12) store through this branch's deltas.
- Bounded review, hunks not inspected: `src/protocol/handoff.rs` test hunks `@@ -1053,6 +1146,7 @@` and `@@ -2118,6 +2212,7 @@`; `src/test_support/fake_hc.rs` beyond its start/listen/halt/rebind/drop section.
- Bounded review, paths not inspected in this invocation: `src/host/hc/rpc.rs`, `src/host/hc/mod.rs`, `src/cli/caller_location.rs`, `src/cli/hook.rs`, `src/cli/handoff.rs`, `src/cli/handoff_delivery.rs`, `src/cli/topology_handoff.rs`, `src/cli/topology_handoff/automatic.rs`, `src/cli/mod.rs`, `src/cli/commands.rs`, `src/cli/instance.rs`, `src/cli/follow.rs`, `src/cli/peer_launch.rs`, `src/cli/wake_warning.rs`, `src/app.rs`, `src/daemon/control.rs`, `src/daemon/lifecycle.rs`, `src/harness/codex/composer.rs`, `src/harness/registry.rs`, `src/host/mod.rs`, `src/host/native.rs`, `src/host/observation.rs`, `src/host/transport.rs`, `src/notification/dispatch.rs`, `src/protocol/handoff/automatic.rs`, `src/protocol/handoff/automatic_bootstrap.rs`, `src/store/topology_handoff/automatic.rs`, `src/test_support/mod.rs`, `scripts/**`, `Cargo.toml`, `Cargo.lock`, `herdr-plugin.toml`, `.github/workflows/ci.yml`, `AGENTS.md`, `TRUST-POLICY.md`, `CHANGELOG.md`, `integrations/**`, every `tests/**` path (including golden fixtures and `tests/fixtures/hc/hc-0.1.15.openrpc.json` beyond the schema types named above) and every `docs/**` path.
- Provenance gap: `f4ddaeffa:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51` inv-8759cc82-264d-4506-beaa-307c9600bb8d invalid failed assembly archive (failed bounded archive contains successful assembly or split claims).
- Provenance gap: `f4ddaeffa:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58` inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f invalid failed assembly archive (same cause).
- Provenance gap: `04df0e77a:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51` inv-8759cc82-264d-4506-beaa-307c9600bb8d invalid failed assembly archive (same cause).
- Provenance gap: `04df0e77a:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58` inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f invalid failed assembly archive (same cause).
- Ledger completion record incomplete (inv-1 `landed≠completed 13 vs 8`, malformed ledger-digest header, ledger/Git identity mismatch).

## Deferred OK
- Recurring `[missing-evidence]` process cluster (a reporting practice; its coverage effect is under Untested scope).
- ht-kqrt.10 hook pane gate is stricter than before and real Herdr panes set `HERDR_ENV=1`; pin it with a test later.
- ht-kqrt.10 empty `HERDR_PANE_ID` is now filtered to `None`.
- ht-kqrt.12 `host`/`caller_host` are not yet set by any CLI (planned in the site phases); tests live in `tests/store/queries.rs`.
- ht-kqrt.3 shallow budget test; both submit paths use `submit_budget_millis()`.
- ht-kqrt.7 `has_idle_wake` is still `pub`.
- ht-kqrt.5 first-attach hook skip untested and health_hosts coverage claim unconfirmed.
- ht-kqrt.53 `BootstrapProgress` widened to `pub` and the `AuthorizeLaunchGate` capability cited in prose.
- ht-kqrt.8 `SeatWakeStatus.host` is `serde(skip)`.
- ht-kqrt.75 Cargo.toml `[[bin]] fake-hc` placement detaches the required-features comment.
- ht-kqrt.76 `ic_pids` ps field layout assumption and the core-test leak rule not requiring PPID 1.
- ht-kqrt.1 commit trailer model name.
- The doctor unattachable-host limitation line prints JSON-quoted endpoint and kind.
- `classify_wake_rows` caller duplicates the `mod_suppressed` formula inline.
- `wire.rs` module comment claims null tolerance that `serde(default)` does not give; matches the 0.1.15 schema today.
- `fake-hc` binary leaves its socket on SIGTERM despite the doc's exit claim.
- The hc rpc response id is not matched to the request id, and an oversize line is reported as a closed connection (from inv-1; not re-inspected here).
- `caller_location` `Ambiguous` uses the pane unverified until ht-kqrt.11's guard merges (from inv-1; not re-inspected here).
- Duplicate closed beads ht-kqrt.40 and ht-kqrt.75 (same title; .75 merged).
