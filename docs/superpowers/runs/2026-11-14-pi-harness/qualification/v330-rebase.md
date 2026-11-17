# v3.3.0 baseline integration audit

Rebased source `d642412ceeaa00218ad219279b6cdf28e49890d7` uses required main baseline `4f2a32d3a1fb9bd5991f6d1cb13fb345b99f1da5` (v3.3.0). Prior tip `7766d2986fa658918436d8fd298e13753df0f33d` used `4dabf06ef90c8c9e8c459b524b509af3a2e98b31`. [Exact mapping and source inventory](v330-rebase-evidence.json) record the independent accounting. Pi remains Unreleased, intended for future v3.4.0; this audit does not release it.

Independent range-diff verification found **85 patches: 82 equal and three append/context resolutions**. Exact byte comparisons verified both upstream and Pi additions survive:

- Entry 8: upstream queue-wake TRUST sections and Pi deferred-notice sections are both retained. Only their append location/context changed.
- Entry 60: the upstream `QueueNotifier` scheduler test and Pi `WarningContacts`/late-successor liveness group are both retained byte-for-byte from their respective inputs.
- Entry 74: the Pi Unreleased changelog entry remains above the complete v3.3.0 entry.

There are still **34 source paths outside `src/harness/pi/`** against the new base. The [core table](core-changes.md) retains the corresponding branch inventory; upstream queue/native source is baseline code, not a Pi-added migration or transport implementation. `src/store/schema.rs` and `migrations/` have no branch delta. Schema **18** comes entirely from v3.3.0's queue-wake migration.

## Shared queue/native integration

Upstream adds optional `WakeControl` and replay-scope adapter methods. Their neutral defaults are `None`; Pi does not override them. Queue selection returns the Native path when no control exists. Pi therefore retains composer wake, its canonical owed check and current host safety guards, without starting a programmatic queue helper. Programmatic versus composer receipt tracking remains the upstream scheduler contract; the preserved queue and notice-liveness groups cover distinct paths.

The initial source audit found one required compatibility correction: Pi's exhaustive `EventMetadata` initializer lacked upstream's new `native_turn_id` field. It must explicitly use `None`; Pi supplies no correlated native turn ID for the queue protocol. A scan of exhaustive initializers in `src/` and `tests/` found no other omission. The exact one-line adaptation is committed as `9f69b8d02b1244a7e21be8df9d368b380f887764`, independently approved by the Rust source reviewer. The auditor verified that commit adds only `native_turn_id: None`. This closes the source integration finding without asserting a completed build or runtime gate.

The AGENTS and TRUST wake notes are scoped to **composer submission, including fallback**. Their accepted check-to-send gap is not a claim about the programmatic queue transport. Upstream's stricter person/operator guard remains the v3.2.2 baseline behavior described in [the earlier audit](v322-rebase.md).

## Evidence boundary

This audit inspects conflict resolution and adapter integration; it does not independently requalify the upstream queue feature or execute builds/tests. Shared v3.3.0 checks are active on corrected source `9f69b8d02`; fresh native qualification and final suite results remain separate and are not claimed here. The integration branch and main handoff become ready for merge only after the coordinator records the required final gate disposition. [Historical d55/a8 paid evidence](final-live.md) retains provider and ordinary-path scope, not a new binary or queue-transport claim; no additional paid run is authorized. Prior failed native attempts remain failures. Current execution status belongs in the [qualification index](README.md).
