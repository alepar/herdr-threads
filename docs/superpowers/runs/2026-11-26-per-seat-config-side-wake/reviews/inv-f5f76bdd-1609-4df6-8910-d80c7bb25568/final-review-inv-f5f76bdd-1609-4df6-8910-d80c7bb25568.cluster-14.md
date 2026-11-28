# Final review: cluster 14 (src-daemon), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified as commits; review source bound before this write). Cluster input: `src/daemon/settings.rs`, slice `slices/000079.diff`.

## What the cluster changes

`InstanceSettings.programmatic_wake` goes from `bool` (default false, carried into `ServiceConfig` via `with_programmatic_wake`) to `Option<bool>` (default `None`), documented as obsolete with no effect. The module header and field doc point at `ht human config seat set --seat S programmatic_wake on`. The unit test now asserts that an absent key parses to `None`, `true`/`false` to `Some(..)`, a non-boolean (`"on"`) is still refused, and `ServiceConfig::from_settings` still builds with the key set.

## Requirement baseline trace (rows touching this cluster)

- B1 ("replace node level setting with seat level") and B10 ("Remove, no migration"): at head, the only reader of `InstanceSettings.programmatic_wake` outside this file is `src/cli/doctor.rs:1171`, which reports `programmatic_wake_node_setting: "obsolete"` when the key is present. `src/service/config.rs` no longer has the flag (base had `with_programmatic_wake(settings.programmatic_wake)` at line 297). `src/notification/queue.rs:398` notes that the seat's own key is read inside the selection transaction. The remaining `programmatic_wake` identifiers in `src/ports.rs` and `src/scheduler/mod.rs` belong to the per-attention witness, not the node setting. The node key is therefore inert, and no migration onto seats exists. The cluster delivers its part of B1/B10. The per-seat read path (store/selection) and the doctor text line are outside this cluster and belong to the seam reviewer.
- Spec "Removing the node setting": the key stays parseable under `deny_unknown_fields`, so an existing file does not fail daemon start. Implemented as the spec says.

## Correctness notes

- `Option<bool>` under `#[serde(default)]` also accepts JSON `null`, which parses to `None`, so doctor does not flag it as obsolete. That is harmless: the key has no effect either way.
- A non-boolean value still fails daemon start, which matches base behaviour for the same malformed value. Keeping the strict parse is consistent with the rest of the file. It means a malformed obsolete key still blocks start, but that was already true at base and the spec does not ask to relax it.
- The file is local operator config, not the peer wire, so `scripts/check-wire-compat` rules (`deny_unknown_fields` ban) do not apply here.
- `Some(false)` is also reported as obsolete by doctor. That is reasonable, because the key is obsolete whatever its value.

## Findings

- none

## Concerns

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No daemon-start test in this cluster loads an on-disk settings.json that contains the obsolete key through `settings::load` and the elected-daemon path. Coverage is the `parse` unit test plus `ServiceConfig::from_settings`, which share the same schema, so the residual risk is low.

## Deferred OK
- none
