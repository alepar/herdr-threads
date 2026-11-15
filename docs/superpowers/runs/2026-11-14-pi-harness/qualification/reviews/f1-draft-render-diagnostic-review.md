# Post-command draft diagnostic review

**Diagnosis supported; visible-draft refusal remains unqualified by this run.** Independently inspected raw `notice-wake-diagnostic/{captures.json,native.jsonl,manifest.json,results.json}`, rather than relying solely on its summary. Source HEAD `c045bdd742d1856f0d0e36cbd4d39197af6d351e`; binary SHA256 `a8aef2df56e6954fe3c12b7d02841f521712ceb403d13f9b93fabdabe2224289`; provider SHA256 `1bbb2f67a76a015beebeb055fb915389fb6aa50178c46d1e7c17e95d7d2380fa`.

## Observations

- Fresh API observation at `1794741845795` reports idle with the exact draft. Post-command capture at `1794741845.8872876` shows an empty editor in visible and detection text, and an inverse cursor at column zero with plain padding in ANSI.
- Fresh API observation at `1794741855847` still reports the draft. Capture `notice-only-pending-before-clear` at `1794741855.9087229` still shows that same empty visible/detection/styled editor. This independently reproduces sustained internal-versus-rendered divergence, not merely a single transient frame.
- The controller's clear is followed by terminal Ctrl+C at `1794741855916` and empty API state at `1794741855948`. Wake paste arrives at `1794741856386`, Enter at `1794741856687`, and `before_agent_start` at `1794741856724` contains exactly `herdr-threads: wake up`.
- The case reports PASS in 17.306 seconds. Cleanup records no errors, helper exit 0/reaped, leak exit 0, and scratch removal.

## Interpretation and limits

The reproduced divergence strongly supports the earlier source-backed explanation in `draft-render-source-audit.md`: fixture `ctx.ui.setEditorText` restored memory without repainting. The previous failed run's exact near-submit frame was not retained, so this diagnostic does not retrospectively prove that frame. It does establish that the fixture's API draft witness is insufficient evidence of a visible draft.

The observed empty screen satisfies Pi's existing screen-empty grammar; the shared native gate cannot distinguish it from an actually empty editor using those pixels. No alternate submission bypass or shared native-gate defect was identified by the source audit. No shared gate change is justified by this evidence. Codex, agy, and Hermes source behavior is unchanged; this statement does not claim fresh runtime coverage for them.

The diagnostic PASS establishes the exercised post-clear wake path, not refusal while a visible draft is held. The preceding quiet interval may depend on scheduling and does not establish hidden-draft safety. The prior whitespace/cursor-home screen limitation did not automatically authorize this distinct unrendered-editor limitation.

During review, the coordinator relayed main's explicit disposition `mnSGNESDIxnR`: accept the unrendered third-party `ctx.ui.setEditorText` case as its own documented screen limitation, tracked by `ht-1z1n`, and qualify a genuinely rendered draft without changing the shared gate. This review records that newly supplied disposition; it does not infer acceptance from previous limits. Normative documentation remains root-owned.

A fixture correction that clears/retypes only its exact owned draft through real terminal input, then requires fresh API state and literal styled/detection draft evidence before the receipt deadline, addresses the demonstrated qualification gap. Its implementation and causal tests need separate frozen-delta review. This review makes no approval claim for that forthcoming patch.

No runtime, builds, tests, or production edits were performed by this reviewer.
