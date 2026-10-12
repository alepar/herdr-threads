# Main v0.9.0 neutral seams author report

Implemented the approved four-file lane in shared `ROOT/.worktrees/upstream-composition-v090`, during the uncommitted composition of `f330bdab` and Main `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`. This is an author report, not independent Quality review or a runtime PASS. Prior source reviews remain historical.

Owned changes:

- `src/protocol/capabilities.rs`: retain the union, including `delivery.presentation_v2`, `inbox.binding_cursors_v1`, and `attention.binding_digest_v1`. Existing send-time/Ensure capabilities remain.
- `tests/protocol/capabilities.rs`: retain both constant assertions and independent full ordered literal expectations. The base list contains exactly 27 distinct capabilities. The bare-daemon expectation retains binding capabilities before the dynamically appended invitation-confirmation capability; both branches' handler probes remain.
- `tests/service/mod_channels.rs`: retain the D presentation domain/registry cases and all Main binding-channel cases. Adapt D registry calls and entry lookups to the exact BindingId API agreed with `impl_watch_recovery`. Add a wrong-binding lease check, preservation of a sibling binding's owner after replacement, and a wrong-binding presentation-health refusal. Keep original generation/session/lease/replacement/grace and owner-serialization assertions.
- `src/store/attention.rs`: review the clean merge semantically and replace `settle_pulled_notices`' newest generation/session selection with an exact admitted BindingId argument immediately after seat_id. The deciding query checks matching instance/seat/session/generation, current seat mapping generation, resolved seat, open connected top-level binding with empty agent id, and no unreleased target recovery hold. The bounded exact oldest-first carried-prefix comparison and binding-owned frontier upsert are unchanged. It never chooses a sibling or enrolls a caller. The admitted ID remains a product of canonical caller admission, not client-local authority. D confirming-invitation/open-origin logic and Main binding frontier/mail additions remain in the merged file.

The exact settlement API was agreed with `impl_watch_recovery` and `compose_v090_store_binding` before editing. Watch owns ports/service forwarding; store-binding owns store/mod forwarding and tests/store/attention.rs adaptation, including the binding-isolation regression. Those files were not edited by this lane. D's epoch/physical-token confirmation logic remains under its existing owners; this legacy pulled-notice helper does not replace those masks or grant token authority.

Static validation completed: file-specific `rustfmt --edition 2024 --config skip_children=true`, subsequent `rustfmt --check` on all four files, and scoped `git diff --check` passed. A source inventory verifies the union of Main/f330 channel-test/function declarations, with no duplicate root functions, after accounting for Main's intentional `mod_seat_view`→`mod_binding_view` and three generation-to-binding test renames. A separate read-only comparison verifies that the full independent capability literal list exactly matches the composed base advertisement, with no duplicates. All four files are free of merge markers.

During editing, an initially unanchored Python end-marker search matched an earlier domain-module test and duplicated a suffix. The author identified and removed precisely that mechanical duplicate, reapplied the edits with an anchored search, and performed the declaration inventory and parser/format checks above. No concurrent writer was involved; the initial concurrency concern was withdrawn.

Final owned-file SHA-256 values:

| Path | SHA-256 |
|---|---|
| src/protocol/capabilities.rs | `152f3f66b034ac1893e1f417d1e8485fd5a828266f9de57301e47c20d6baf9ff` |
| tests/protocol/capabilities.rs | `290ef6e02d851e36e464bf6e119e446b5a1643d1cb72677615929a6c22a6f766` |
| tests/service/mod_channels.rs | `22be16a921f3998b4c5f728a08e517479aaacea92613cace36756586a13205fb` |
| src/store/attention.rs | `6736940dd971638d32506c6169a2e267e08c34ffc4d78f412dc1f91b7b6ec95e` |

No Cargo, runtime tests, Git staging/commit, Herdr operations, native configuration writes, or subagents. Parent-owned compilation, focused tests/gates and fresh independent review remain required. Author lane complete; quiescent.
