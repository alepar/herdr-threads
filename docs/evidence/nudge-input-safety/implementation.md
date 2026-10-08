# Approved attention input guard

2026-10-08, worker w4:pE5 / sBdSTaoct; development thread taQXwBH7X.
Product parent/diagnosis checkpoint: 116f5b175e0241fe7fe6fd0059567754c201e4e0.
Original main base: ada2f3767867096d9821db450bd0f6e8eb633d6e.
Original report mFeB1pLcR and diagnosis artifacts remain unchanged. The prior diagnosis document is a historical checkpoint; its proposed strict hook-only design was superseded by the user-approved sampled policy below.

## Approval and behavior

User accepted focus/input races and sampling limits, then approved focused empty composer observed for one minute, unfocused recognized empty immediately, nonempty/unreadable deferred indefinitely (approval relayed mDQ5llAvq). No stash/clear/retype or additional Enter. Focus is pane selection, not telemetry for every physical user action.

Native final admission now reads focus/composer after identity/status checks and before prompt delivery. The process-local 1024-window map uses monotonic time and seat/target/server/terminal/harness identity; target observation sequence is not identity. Failed admission, observed draft/unreadable read, successful send, identity change, clock reversal and gaps over one minute restart qualification. Ordinary status-only idle reads preserve continuity. Restart discards all samples.

Dispatcher refuses HumanInput without stashing even if a historical recipe declares it. Pre-submit refusal restores the existing ladder and preserves pending attention/receipt obligations. Post-submit uncertainty stays OutcomeUnknown; read-only verification never presses Enter. PokeOnly retains unconditional focus exclusion. Native adapter stash helpers remain unused by notification dispatch.

Lazy owner coordination: m9mtAxCw5 confirms default Lazy stays quiet; --nudge/ACK freeze Ordinary; pre-submit refusal restores prior ladder without downgrading mode or settling obligations. No schema/protocol/port changes or peer source imported.

Final coordination m37rf2dVA reports reviewed Lazy publication44876c0f, isolated merge4a581f0e: Lazy produces no receipts/warnings/send_attention/outstanding attention. This is the owner-reported reviewed producer contract, not a worker execution claim; its full feature remains unfinished and was not imported.

## Evidence and verification

Causal RED logs show three original unsafe admissions/extra Enter, HumanInput stash admission, draft-observation continuity bug, status-only reset bug, and an affected integration fixture timeout. Sandbox socket/ps permission failures are not causal evidence and are excluded from the final causal set.

Final passing checks:
- Native host regressions: 70 passed.
- Scheduler regressions: 91 passed, including canonical receipt/ladder seam.
- Trust integration filter: 12 passed.
- Slow-host history/health latency: 1 passed (17.55s); existing assertions unchanged.
- Sweep filter: 6 passed, including three summary-flow cases matched by the filter.
- cargo fmt and git diff --check passed.
- nice cargo clippy --locked --all-targets --all-features -- -D warnings: exit 0, final incremental 1.06s.
- nice scripts/check-default-features: exit 0. Sandbox denied renice for these two checks, but both commands ran successfully; no Rust warning suppression.
- UUID-scoped process check 1867184b-1672-4898-9f65-677cd50b2d68: exit 0, no leaked test processes.

Independent review found and verified corrections to timer reset behavior; final rereview and fixture review had no blockers. Review source conclusions are separate from this worker's executed tests. Logs, final product patch and SHA256 manifest are in implementation/. No routine full suite was run, per current repository policy; main owns final composition/integrated gates.

## Limits and release handoff

This is an approximate wake-preserving guard, not an atomic draft guarantee. A user can type/change focus after the screen read, or type/delete between samples. Detection can hide whitespace-only or captured-placeholder-identical drafts. Parked drafts, suggestions or unreadable screens may defer indefinitely. Existing setup controls prompt suggestions only with explicit user permission; this change edits no real configuration.

No shared host, account/config, native model, external Herdr source, install, main branch, push, or release action occurred. Worker owns isolated product implementation; main owns reviewed absorption, fresh composition checks against current main and release. Warning-only empty inbox is a separate lane. Preserve unrelated holds/approval gates; only the completed account-migration pause was cleared.
