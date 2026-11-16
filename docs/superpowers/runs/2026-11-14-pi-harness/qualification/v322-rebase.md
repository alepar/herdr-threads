# v3.2.2 integration audit

Audited source: `16b3812ae5c835d907a870c313ac4d0385443848` on main `4dabf06ef90c8c9e8c459b524b509af3a2e98b31`. Prior tip was `fc9ba7150db31a9af0e8f2130a6e8383309c5391` on `e60ec5eb5e9746f82b399127838dc9515c7cc712`. [Exact anchors, 84-entry mapping and source inventory](v322-rebase-evidence.json) accompany this read-only audit.

An independently regenerated range-diff has **82 equal patches and two context-only differences**:

- Entry 34 (`bbedc2ed7` → `47f7b2bbc`): TRUST-POLICY context incorporates upstream's person-session restriction. The Pi amendment is retained; inherited agent markers no longer imply an operator bypass.
- Entry 74 (`bad263b15` → `521570b5a`): changelog context changes from v3.2.1 to v3.2.2. The resolved file retains Pi under Unreleased above the complete v3.2.2 release entry. This was the coordinator-reported sole manual conflict.

Across source, Pi integration and qualification driver/fixture paths, old-tip to new-tip changes exactly match upstream additions/deletions in six files: `src/cli/commands.rs`, `src/cli/help.rs`, `src/cli/legacy.rs`, `src/cli/me.rs`, `src/cli/mod.rs` and `src/harness/registry.rs`. Pi adapter/extension and qualification driver/fixture bytes are unchanged by the rebase. The [core inventory](core-changes.md) remains **34 outside-Pi source paths** against the new base: 31 production paths and three test-only paths. The Native characterization remains test-only within already-listed `src/host/native.rs`.

## Person/operator integration

The upstream registry collects both registered agent environment markers and every declared caller session variable. Pi contributes `PI_CODING_AGENT` and its session/role/mode/PID witness variables through the existing registered adapter; its earlier removal from the unsupported-marker list does not create a person-attribution gap. `person_guard` refuses a present session marker before consulting the operator override or pane evidence. The presence check also covers empty values. A recorded operator mark cannot bypass this first check. Only a plain-shell caller without agent-session markers retains the upstream pane-evidence override.

This is source integration accounting, not runtime qualification of the new guard. The driver's explicit sender environment allowlist and its synthetic inherited-marker regression are described in the [main-review disposition](main-review-disposition.md); they do not authorize acting as a person from a real agent session. No Pi-specific bypass was introduced.

## Qualification boundary

The [main-review correction evidence](main-review-disposition.md), [historical R2](../roast-pr-2/report.md) and earlier integration reports retain their exact original anchors. The new tree maps the main-review correction commits to `052c25022` (N1/N3/N4), `e749c8ad2` (S1–S3) and `47f5a8997` (S4/N2). Characterization/terminal fixture source `25f3a8340` maps to `932cbd6dc`; the accepted-limit documentation maps to `16b3812ae`.

Historical paid [d55/a8 live evidence](final-live.md) remains bounded provider and ordinary-path evidence, not a run of a newly built v3.2.2 binary. Main authorized no new paid run. Fresh scripted native and full-suite execution are separate gates; shared checks/build were underway and no new full suite was claimed at this audit. The prior failed `notice_page` stays failed with its actual pre-paste composer gate unobserved. This audit ran no builds, tests, native sessions or provider requests. Current gate status belongs in the [qualification index](README.md).
