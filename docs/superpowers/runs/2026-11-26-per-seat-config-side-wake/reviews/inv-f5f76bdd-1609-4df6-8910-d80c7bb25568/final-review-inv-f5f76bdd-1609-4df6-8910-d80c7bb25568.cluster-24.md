# Final review: cluster 24 (src-test-support)

Epic ht-z41e, integration branch super-auto/per-seat-config-side-wake.
Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`; review source bound to the ledger, exit 0).

## Scope read

- slices/000126.diff: `src/test_support/history.rs`, `Actor::claim` adds `auxiliary_session: false` to its `CallerClaim` literal.
- slices/000127.diff: `src/test_support/mod.rs`, the `synthetic_fourth` test adapter's `decode` adds `auxiliary_session: false` to its `EventMetadata` literal.

## Analysis

- Both hunks complete struct literals after ht-z41e.21 added `CallerClaim.auxiliary_session` (`src/protocol/authority.rs:34`, `#[serde(default, skip_serializing_if = "std::ops::Not::not")]`) and `EventMetadata.auxiliary_session` (`src/harness/adapter.rs:768`). Without them the `test-support` build does not compile, so the change is required.
- `false` is the right value in both places. The history `Actor` builds claims for existing top-level bindings it reads back from `occupant_bindings`, and a `/side` session is not among them. The synthetic adapter models no auxiliary session. The bit is sticky on the binding and later `false` evidence never clears it (`src/harness/adapter.rs:622-624`), so a `false` claim from these helpers cannot reset a binding that a test marked auxiliary.
- Wire compatibility is unaffected. `false` is skipped on serialization, so these test claims stay byte-identical to the pre-change wire for daemons without `seat.auxiliary_session_v1`.
- In `history.rs` the field sits first in the literal rather than in declaration order (after `liveness`). This is cosmetic: rustfmt does not reorder fields, and nothing breaks.
- The `true` path is tested outside this cluster: `src/cli/site_capability.rs:408` sets `claim.auxiliary_session = true`, and the Codex `/side` fixtures `tests/fixtures/codex-0.163.0-side/*.json` drive `tests/cli/hook.rs` through null `transcript_path` payloads.

## Requirement baseline trace

- The spec's `## Requirement baseline` (B1 onward: seat-level config replacing the node setting, a generic key=value seat config, the read/write and list CLIs, a nudge threshold writable without the human path, `/side` wake handling) is delivered only by production code outside this cluster. `src/test_support` compiles only under the `test-support` feature and has no production entry point, so this cluster neither delivers nor regresses any row. Tracing the rows end to end belongs to the seam reviewer and the clusters that own `src/cli/config_seat.rs`, `src/store`, `src/harness/codex` and the wake path.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The `synthetic_fourth` adapter cannot emit `auxiliary_session: true`, so the neutral-core tests that use it never exercise the sticky auxiliary bit through a non-Codex adapter. The Codex fixtures and `site_capability.rs` cover the bit itself.

## Deferred OK
- In `src/test_support/history.rs`, `Actor::claim` places `auxiliary_session` first instead of in declaration order. Cosmetic.
