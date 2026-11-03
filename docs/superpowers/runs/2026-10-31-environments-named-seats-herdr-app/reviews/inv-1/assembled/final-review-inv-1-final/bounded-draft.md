# Final review (bounded): epic ht-kqrt on super-auto/environments-named-seats-herdr-app

Captured source: base f4ddaeffa9fa13549f514669391e84a7fc8d5e5f, head 124759cc8a49763e5cfe1600b68892c532d7f3f1 (both verified with `git rev-parse --verify`). Invocation inv-1 was bound to the ledger before any artifact write.

## Scope and route

This is a bounded captured-source review, not a whole-range review. The selection sidecar `assembled/final-review-inv-1-final/review-inputs.json` reports status unavailable, sourceAvailable true, wholeRangeReady false, 189 changed paths, 0 omitted, 0 slices and 5 provenance gaps (listed under Untested scope). No whole-range package was built or loaded. A whole-range review is still outstanding.

What I read: the full changed-path inventory (189 paths, 43153+/406-), the source, script and config diff (`git diff base..head` over src, migrations, scripts, Cargo.toml, herdr-plugin.toml, .github, AGENTS.md, TRUST-POLICY.md, integrations), the epic-only part of the source diff (`git diff 140ce59f8..124759cc8`, i.e. after the ht-kqrt.1 main merge), the head versions of `src/host/hc/{mod,rpc}.rs`, the hook-side callers of `HookEnv.pane` in `src/cli/hook.rs`, `HostSessions::{resolve,attach_startup,attach_remembered_live,sweep_tick,startup_primary}`, the epic bead tree (`bd list --parent ht-kqrt`), and the ledger. In a review-owned detached worktree at head I ran `scripts/check-harness-neutral` (ok, 0 violations) and `scripts/check-wire-compat --base f4ddaeffa` (ok, 0 violations, 0 stale). I ran no cargo build, clippy or test.

Range composition: 13 task merges (ht-kqrt.1, .2, .3, .4, .5, .7, .8, .10, .12, .37, .53, .75, .76) plus the run's design docs. ht-kqrt.1 brought in main up to 2.8.1 (the handoff `--summary` prompt, the herdr-plugin.toml `id` removals, the hermes README and skill text). Those hunks were already reviewed on main; I did not re-review them here.

## Step 1: my own findings

### Epic completeness (blocking)

- The epic goal is not delivered on this branch. Of 93 children, 58 are open. These include every named-seat leaf (ht-kqrt.15 to .34), tmux wake (.38, .69, .70), every hc host leaf (.41 to .52, .55, .60, .67, .68), launch and handoff through CreateTargetPort (.54 to .58), the phase gates (.13, .62, .63, .64), the host-neutrality scan (.65) and the integration sweep (.66). The seams that are on the branch (HostKind, WakeCapability, CreateTargetPort, attach_builtin, caller_location, the hc RPC client, migration 0012) have no production consumer yet. Example: no non-test caller of `attach_builtin`, `create_target` or `target_root_process` exists at head.
- Four tasks are closed as "implemented; review and merge tracked by" open review beads, and are not merged: ht-kqrt.6 (ht-kqrt.91), ht-kqrt.9 (.92), ht-kqrt.11 (.93) and ht-kqrt.14 (.84). The ledger records ht-kqrt.14's merge as held because its task worktree is absent. Their branches (task-ht-kqrt.6/9/11/14) are not in head.

### Main drift and migration collision (blocking)

- At head, main is 25 commits ahead (`git log 124759cc8..main`: v2.8.2 is prepared). Main has its own `NodeDelta { version: 12, sql: 0012_process_continuity.sql, rebuild: Some(copy_allocation_decisions + 0012_process_continuity_after.sql) }` in `src/store/schema.rs`. This branch adds a different `NodeDelta { version: 12, sql: 0012_host_environments.sql }`. Once main is merged, two deltas claim user_version 12. A store already at 12 from either side would never apply the other side's delta. A 2.8.2 store would then lack `herdr_hosts.kind`, and `attach_host` and `read_row` select `kind`/`kind_probed_at`, so every host attach and remembered-row read would fail. Main's delta also rebuilds `seats`, so the order matters for the branch's `seats.name` CHECK column and the `seats_live_name` index. The branch delta has to be renumbered after main's (13), the rebuild has to be checked to keep the added `seats` and `occupant_bindings` columns, and a test has to migrate a 2.8.2-shaped store.
- The version (Cargo.toml and herdr-plugin.toml 2.8.1) and CHANGELOG also need that main merge before landing.

### Cross-task seam: built-in host persisted as a Herdr row (ht-kqrt.4 x ht-kqrt.5)

- `HostSessions::attach_builtin` (ht-kqrt.5) persists the built-in host through `store.attach_host`. That function (ht-kqrt.4) always upserts `kind='herdr', kind_probed_at=now`. When ht-kqrt.15 wires `attach_builtin` at daemon start for the named host, three things go wrong. (a) The named host row is stored as kind `herdr`, so `herdr_hosts_one_named` never applies, and `HostRow::attachable()` treats the row as a dialable Herdr host. (b) `sweep_tick` filters candidates against `attached()`, which excludes the built-in, so every tick picks the built-in row up again and probes `factory.exists` on its locator. (c) `attach_remembered_live` visits remembered rows most recent first. Because `attach_builtin` just stamped `last_attached_at`, its row comes first, and `by_locator` returns the built-in session, so `startup_first` is set to the built-in's id. `startup_primary` resolves that id through `get()`, which only reads `sessions` and never sees the built-in, so the primary silently falls back to the lowest attached `host_id` instead of the most recently attached Herdr server. The primary feeds Health, archival and the advertised bootstrap capability. Fix before ht-kqrt.15 lands: make the kind write follow the caller's kind, and keep the built-in row out of the remembered-row attach and sweep (or out of `startup_first`).

### Composer screen_blocked fallback (ht-kqrt.37, latent)

- Claude's and agy's `screen_blocked` return `Some(true)` only for the captured approval layouts (`Esc to cancel` + `❯ N.`, `↑/↓ Navigate` + `> N.`). Any other screen that `read()` does not report as `Unreadable` returns `Some(false)`, which the trait documents as "a recognized idle or busy composer", i.e. typeable. A question dialog (AskUserQuestion) or any other uncaptured selection dialog whose footer differs falls into `Some(false)`. The tmux composer wake (ht-kqrt.69, open) is the planned consumer: a wake typed there would answer the dialog. No production caller exists at head, so nothing breaks today, but the consumer must not merge on this fallback.

### Smaller observations

- `src/cli/doctor.rs` limitation line: `format!("host {} {} kind '{}' ...", host["host_id"], host["endpoint"], host["kind"])` formats serde_json `Value`s, so strings keep their JSON quotes (`host 2 "/herdr/far.sock" kind '"future"': ...`). The test only checks `contains`.
- `src/service/dispatch.rs` `wake_status` inlines `wake_suppressed && !stalled`, which is the same formula as `scheduler::mod_suppressed`. The doc comment names `mod_suppressed`. The two copies can drift.
- `src/host/hc/rpc.rs`: the response is taken from the first line with a non-null id, without matching it to the request id (harmless with one connection per call). A reply line over 16 MiB is truncated by `take` and reported as "connection closed before a response".
- `src/cli/caller_location.rs`: when both a pane and a terminal family are present (`Ambiguous`), the pane is used unverified. This is documented, and the TUI leak guard M1/M3 is in ht-kqrt.11, which is not merged. An `hc` terminal started from inside a Herdr pane inherits `HERDR_PANE_ID` and would act as that pane's seat until then.
- Checked and fine: wire compatibility. New `SeatsQuery.host` and `ResolveThreadQuery.caller_host` are `serde(default, skip_serializing_if)` and byte-identical for Pane sites (tests pin the bytes). `SeatWakeStatus.host` is `serde(skip)`. `AuthorizeLaunchGate` is refused client-side unless `launch.gate_v1` is advertised (it is not) and refused daemon-side as Unsupported. The `HostPort` defaults reproduce the Herdr behaviour (wake budget 2000 ms, so replacing `PROMPT_MILLIS` with `submit_budget_millis()` keeps both submit paths unchanged). `classify_wake_rows` agrees with `has_idle_wake` on a TUI host (table test). The wake lane is skipped only for `WakeCapability::None`.

## Step 2: ledger triage

- Recurring minor `[missing-evidence]` / `[stale-evidence]` (ht-kqrt.2, .3, .4, .5, .8, .10, .12, .37, .53, .75): a process class. Clippy and check-default-features were reported pre-rebase only for ht-kqrt.4, .5, .8, .37 and .53. Together with no full-suite run on head, this is untested scope, not a code finding.
- ht-kqrt.37 `[missing-question-screen]`: misgraded as minor. It names a known product-defect path once consumed (see the screen_blocked section). Must fix before the tmux wake consumer merges.
- ht-kqrt.10 `[hook-gate-behavior-change]`: `HookEnv.pane` is now `None` without `HERDR_ENV=1`, and `pane_context_journal` and `record_observer_reset` read `env.pane` without their own gate. Real Herdr panes set both variables, and the new behaviour is stricter, not looser. Deferred OK, but pin it with a test.
- ht-kqrt.12 `[deferred-wiring]`: no CLI sets `host`/`caller_host` yet. This is planned for the site phases (ht-kqrt.16, .46). Deferred OK.
- ht-kqrt.3 `[shallow-test]`: I verified that both submit paths in `notification/dispatch.rs` use `submit_budget_millis()`. Deferred OK.
- ht-kqrt.76 `[unverified-live-path]`: the real `hc run --bg` / `server.stop` path and shellcheck were never exercised. Untested scope until ht-kqrt.52 or an hc-equipped host runs it.
- ht-kqrt.53 `[wire-compat-comment]`, `[visibility-widening]`; ht-kqrt.7 `[visibility-drift]`; ht-kqrt.5 `[missing-edge-case-test]`, `[unverified-claim]`; ht-kqrt.8 `[wire-form-deferred]`; ht-kqrt.75 `[comment-misplacement]`; ht-kqrt.76 `[fragile-process-match]`, `[weak-leak-rule-scope]`; ht-kqrt.10 `[empty-value-behavior-change]`; ht-kqrt.12 `[test-location-deviation]`; ht-kqrt.1 `[trailer-attribution-mismatch]`: Deferred OK.
- Parked findings: none (0 parked). BLOCKED-AUTH lines: none.
- Metrics: `landed≠completed 13 vs 8`, with ledger/completed and ledger/Git identity gaps. The ledger is incomplete, so a clean completion record is not evidence of a clean range.

## Verdict
not ready (epic 58/93 children open; v12 migration collides with main's v12; built-in host seam; no full-suite run on head)

## Must fix before landing
- Renumber `migrations/node/0012_host_environments.sql` / `NodeDelta { version: 12 }` after main's own version-12 delta (`0012_process_continuity.sql`, seats rebuild) when merging main. Verify the rebuild keeps `seats.name`, `seats_live_name`, `occupant_bindings.locator_hint`/`readonly`, and add a migration test from a 2.8.2-shaped store.
- Merge main (25 commits ahead, v2.8.2) into the run branch before landing, and resolve the version, CHANGELOG and schema conflicts.
- The epic goal is not delivered: 58 of 93 children are open, including every named, tmux and hc leaf, the phase gates ht-kqrt.13/.62/.63/.64, the host-neutrality scan ht-kqrt.65 and the integration sweep ht-kqrt.66. Do not land as the completed epic.
- Merge or reopen ht-kqrt.6, .9, .11 and .14: they are closed as implemented, but their review beads (.91, .92, .93, .84) are open, their branches are not in head, and the ht-kqrt.14 merge is held because its worktree is missing.
- `attach_builtin` (ht-kqrt.5) goes through `store.attach_host` (ht-kqrt.4), which hardcodes `kind='herdr'`. Make the stored kind follow the attaching host, and keep the built-in row out of `attach_remembered_live`/`startup_first` and `sweep_tick` before ht-kqrt.15 wires it. Otherwise the startup primary falls back to the lowest host_id and the sweep re-probes the named row on every tick.
- Claude/agy `screen_blocked` returns `Some(false)` (typeable) for any readable uncaptured dialog, such as an AskUserQuestion screen. Add a capture and recognizer, or return `None` for unrecognized selection dialogs, before the tmux composer wake (ht-kqrt.69) consumes it. This was graded minor in ht-kqrt.37 but names a product-defect path.

## Untested scope
- No full-suite measurement of head 124759cc8 exists. The sweep is deferred to the caller, so the branch is not tested as merged.
- This review ran no cargo build, clippy, check-default-features or tests on head. Only check-harness-neutral and check-wire-compat ran (both ok).
- Task evidence for clippy and check-default-features is pre-rebase only for ht-kqrt.4, .5, .8, .37 and .53. Most task reports carry no sourceTree-bound evidence records.
- The real `hc run --bg` / `server.stop` path of `scripts/lib/isolated-core.sh` and its shellcheck test were never exercised (no hc, no shellcheck).
- No Claude/agy question-screen capture exists, so the `screen_blocked` behaviour there is unknown.
- The hook pane gate change (`HookEnv.pane` requires `HERDR_ENV=1`) is not pinned for `pane_context_journal` and `record_observer_reset`.
- Bounded review, hunks not inspected: `src/host/hc/wire.rs`, `src/test_support/fake_hc.rs`, `src/test_support/fake_herdr.rs`, `src/test_support/mod.rs`, and the main-merge hunks in `src/cli/handoff.rs` (33), `src/cli/topology_handoff.rs` (7), `src/cli/instance.rs` (3), `src/protocol/handoff.rs` (3), `src/protocol/handoff/automatic_bootstrap.rs` (2), `src/store/topology_handoff/automatic.rs` (2) and one each in `src/cli/{handoff_delivery,hook,mod,peer_launch}.rs`, `src/cli/topology_handoff/automatic.rs`, `src/daemon/{control,lifecycle}.rs`, `src/host/{mod,native,observation,transport}.rs`, `src/protocol/capabilities.rs` and `src/protocol/handoff/automatic.rs`.
- Bounded review, paths not inspected: every `tests/**` path (67 paths, including the golden fixtures and `tests/fixtures/hc/hc-0.1.15.openrpc.json`), every `docs/**` path, `CHANGELOG.md`, `Cargo.lock` and `docs/release.md`.
- Provenance gap: `124759cc8:docs/superpowers/runs/2026-10-31-environments-named-seats-herdr-app/run.md:39` malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: `f4ddaeffa:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51` inv-8759cc82-264d-4506-beaa-307c9600bb8d invalid failed assembly archive (failed bounded archive contains successful assembly or split claims).
- Provenance gap: `f4ddaeffa:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58` inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f invalid failed assembly archive (failed bounded archive contains successful assembly or split claims).
- Provenance gap: `124759cc8:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51` inv-8759cc82-264d-4506-beaa-307c9600bb8d invalid failed assembly archive (same cause).
- Provenance gap: `124759cc8:docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58` inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f invalid failed assembly archive (same cause).
- Ledger metrics `landed≠completed 13 vs 8`, with ledger/completed and ledger/Git identity gaps, so the completion record is incomplete.

## Deferred OK
- Recurring `[missing-evidence]` process cluster across ht-kqrt.2, .3, .10, .12, .53, .75 and .8 (a reporting practice; its test-coverage effect is listed under Untested scope).
- ht-kqrt.10 hook pane gate: stricter than before, and real Herdr panes set `HERDR_ENV=1`. Pin it with a test later.
- ht-kqrt.10 empty `HERDR_PANE_ID` is now filtered to `None`.
- ht-kqrt.12: `host`/`caller_host` are never set by the CLI yet (planned in the site phases). Its tests live in `tests/store/queries.rs`.
- ht-kqrt.3 shallow budget test: I verified that both submit paths use `submit_budget_millis()`.
- ht-kqrt.7: `has_idle_wake` is still `pub`.
- ht-kqrt.5: the first-attach hook skip is untested, and the health_hosts coverage claim is unconfirmed.
- ht-kqrt.53: `BootstrapProgress` widened to `pub`, and the `AuthorizeLaunchGate` capability is cited in prose (check-wire-compat passes; it is the local protocol, not the peer wire).
- ht-kqrt.8: `SeatWakeStatus.host` is `serde(skip)` (no client needs it yet).
- ht-kqrt.75: the Cargo.toml `[[bin]] fake-hc` placement detaches the required-features comment.
- ht-kqrt.76: the `ic_pids` ps field layout assumption, and the core-test leak rule does not require PPID 1.
- ht-kqrt.1: the commit trailer model name.
- The doctor unattachable-host limitation line prints JSON-quoted endpoint and kind.
- `wake_status` duplicates the `scheduler::mod_suppressed` formula instead of sharing it.
- The hc rpc response id is not matched to the request id, and an oversize line is misreported as a closed connection.
- `caller_location` `Ambiguous` uses the pane unverified until ht-kqrt.11's M1/M3 guard merges.
- Duplicate closed beads ht-kqrt.40 and ht-kqrt.75 (same title; .75 merged).
