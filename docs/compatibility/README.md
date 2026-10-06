# Runtime manifest compatibility

The schema-2 manifest keeps `rows` and the scalar `contracts` map for legacy
Claude/Codex semver admission. Exact runtimes use separate `runtime_rows` and
`runtime_contracts` collections. Build identities never become semver rows.
Runtime descriptors are the binary's discovery DomainContract projection;
historical declarations do not imply current-registry usability.

The writer consumes `artifact-index.json` beside the report, or an explicit
`--artifact-index`. The index joins exact harness, attempt, identity and measured
stage to bounded result/capture paths. Traversal, symlink escape, ambiguous
attempts and mismatched identities fail cleanly. Complete compatible domains
need every declared required milestone. Declared violations alone can become
`known_broken`; incomplete, unsupported, infrastructure and flaky legacy setup
results remain artifacts. Bridge success cannot credit native-shape evidence.

`source_captured`, `no_model` and `live` retain their distinct meanings. Capture
only results never verify; a complete model-free domain can verify schema
compatibility without proving model consumption. `last_seen_at` is nonnegative
integer UTC milliseconds in the Rust store range; top-level `generated_at` is
RFC3339. Manual rows retain their source, stage and exact scope.

Release replay asks the built release for its own discovery, with explicit
legacy `contract-id` fallback for old releases. Only indexed representable
Claude/Codex native payload captures use the legacy gated parser. Domains or
exact builds without a release replay evaluator are reported unsupported;
metadata alone cannot qualify them. Replay retains the original stage and
requires the exact indexed source attempt. It never promotes model-free evidence
to live evidence, or infers a build from a version-looking string.

Missing/infrastructure evidence preserves every baseline collection. Runtime
history retains the newest 50 identities per harness by observation time, plus
manual, known-broken and identities named by existing baseline recipe rows.
Output is deterministic and written atomically only after validation, within
80% of the reader's 262144-byte limit; oversized protected history fails without
replacing the previous output. Publication remains a separate coordinator action.

Hermes is implemented experimental/source-tested, with native acceptance unverified.
`native_callback`/`native_shape_observation` and `bridge_envelope`/`bridge_envelope`
remain independent domains; `qualified_turn` and `qualified_post_tool` must be
measured separately for each exact runtime/contract. Native loader gate and timely
client replies establish plumbing only. Startup-captured identity survives later
source edits; it is neither continuous loaded-byte attestation nor canonical seat
identity. Runtime metadata is optional for registered Claude/Codex operational
contracts and cannot be fabricated from setup or callback success.

The [bounded Hermes driver](../../integrations/hermes/README.md#measurement-contract)
labels synthetic dry runs and keeps source, recognition, callback/domain, context,
model and canonical receipt stages separate. Its sanitized result is not a manifest
row and cannot promote a fixture or unknown native mode to live evidence.
