# Task15 ordinary-Lazy passive reset refinement review

Spec Issues found — affected reset clauses of checkpoint48daad.
Quality Needs fixes — affected reset clauses of checkpoint48daad.

This report qualifies and supersedes the RESET wording in `task-15-ordinary-lazy-signal-review.md` for proposal SHA256 `48daad34d8b371ec8781404e4f6e4ddac788f01f0d976946e3a66028391a5cb7`. Its other source-backed publication, boundedness, serialization and continuation assessments remain intact. My initial CLEAN verdict incorrectly endorsed replacing ModRefresh load_raw with presentation binding-aware load. The implementer identified the concrete contradiction before making that source change. No disputed edit or new tests were performed for this review.

## Important findings

1. **ModRefresh must not own presentation binding reset.** Actual `src/store/invitation_content.rs:158-181` constructs fresh presentation state on binding mismatch, including clearing its outbox, ordinary traversal/carry, presentation frontiers and old control episode while retaining the monotonic epoch counter and independent mod state. Refresh currently uses load_raw at `:691` and separately updates only its mod header/frontiers at `:693-705`. The actual regression `tests/service/delivery_pull.rs:2578`, `presentation_refresh_preserves_live_presentation_header_and_outbox_across_rebind`, snapshots presentation headers/frontiers/allocation/item display counters, rotates the binding, calls refresh, and requires the snapshot to remain exact until a real Pull. The proposal's load_raw→load direction violates this governing M invariant. Retain load_raw in refresh; correct only passive-field ownership/reset.

2. **Independent unconditional zeroing by both owners would reset a binding twice.** Merely zeroing the four new fields on mod identity mismatch while retaining unconditional zeroing in actual Pull's fresh-state construction is insufficient for the existing each-manifest-once/per-binding contract. Refresh-first can scan/increment for the new binding, then Pull clears its signal cursor; Pull-first can scan, then the first refresh clears it. Those are bounded individual calls but duplicated retained-prefix work and potentially duplicate passive generations. Existing main and mod identity headers already distinguish these orders. No extra identity column/cache or accepted duplicate-catchup limit is necessary.

## Smallest precise replacement rule

The following is one concrete rule, not an implementation menu. Parent must select it in a revised exact checkpoint before source authorization.

Define, from the loaded PRE-MUTATION row and the already canonically admitted claim:

- `presentation_matches`: instance_id equals claim.instance AND generation equals claim.binding_generation AND execution equals claim.execution.
- `mod_matches`: instance_id equals claim.instance AND mod_generation equals claim.binding_generation AND mod_execution equals claim.execution.

The row's four passive publication fields belong to the admitted current binding iff EITHER identity matches. They have not been initialized for this binding iff NEITHER matches. Evaluate these predicates before installing/changing either header. The canonical seat/consumer row key remains unchanged; a nonempty foreign instance is corrupt private state and is not a matching scope. Native-session/Watch-lease/current-binding admission remains in the existing canonical service/store boundary, not these private predicates.

**ModRefresh (Watch only):** retain load_raw and existing admission. If neither identity matches, zero ONLY ordinary_publication_after, ordinary_publication_high, ordinary_passive_generation and ordinary_scan_passive_generation. If either matches, preserve all four. Independently retain the existing mod-header mismatch branch: reset its existing mod invitation maintenance frontiers, then install current mod_generation/mod_execution. Preserve existing presentation header, outbox/token/descriptors/masks, invitation presentation counters/frontiers, ordinary cursor/body/carry/version, render selection and control episode exactly. Empty/new row initialization retains its existing behavior and zero defaults. Advance passive publication maintenance and save through the existing shared State transaction; this save is what commits the matching mod header with its passive progress. Do not alter main presentation identity merely to mark the passive cursor current.

**Actual presentation load/Pull:** retain the existing presentation_matches fast path. On presentation mismatch, perform the existing lazy fresh presentation reset and control-counter retention. Preserve all existing mod state as before. If the PRE-RESET row has mod_matches, copy its FOUR passive fields into the fresh State, because refresh already initialized/scanned them for this same admitted binding. Otherwise keep their zero defaults. Install current presentation identity as usual. Do not preserve old presentation cursor/body/carry or native ordinary consumed version merely because passive fields are preserved. At the NEW ordinary traversal START, snapshot the latest native version and passive generation; copied scan_generation is not evidence that the newly reset presentation traversal has consumed that generation.

**Other operations:** no additional durable reset site. State::default supplies zero fields; load_raw validates scalars/relationships without reassigning scope; shared save validates and preserves them on every save. Render selection is read-only. Confirm currently calls load, but a binding-mismatched old token becomes NotCurrentPage and returns without save (`:253-256`); keep this no-effects behavior. Successful current/last-token confirmation cannot reset passive scope, and must preserve the four fields through shared save. Carry serialization and ordinary item cleanup must not encode or zero these fields. Retired rows/seat cleanup retain their existing ownership semantics. Never reset mod_version as part of the new passive reset.

## Interleaving proof

| New-binding order | First operation | Second operation | Result |
|---|---|---|---|
| Refresh then Pull | Neither header matches: zero passive fields once, install mod header, advance cursor | Main header mismatches but mod matches: fresh presentation reset copies four passive fields | No lost/duplicated publication progress; old presentation state survives refresh until Pull |
| Pull then Refresh | Neither matches: fresh presentation reset zeros passive fields, installs main header, advances cursor | Main matches though mod does not: preserve passive fields, update mod header/frontiers | No second passive reset or duplicate retained-prefix scan |
| Repeated refresh before first Pull | First refresh installs matching mod header | Later refresh sees mod_matches and preserves fields | Cursor/generation advances monotonically |
| Hook consumer | No ModRefresh touches its separate row | First binding-aware Pull resets once; subsequent Pull matches | No cross-consumer identity or progress sharing |
| Restart or transaction failure | Persisted matching header and four fields reload together | Failed transaction commits neither | No restart reset or lost signal |
| Another binding arrives before both owners caught up | Neither old header matches the newly admitted claim | First successful owner initializes new passive scope; second recognizes its header | Exactly one reset per actually visited binding scope |

The two owners are serialized by existing canonical deciding transactions; stale claims cannot legitimately revisit a previous binding. Scope generation/execution facts are canonical admission inputs, not observed availability or authority from local hints. A newly current matching main header implies passive scope was initialized by the revised load rule; a matching mod header implies initialization by the revised refresh rule. This induction is why the OR test is sufficient without another stored identity. On initial migration the four defaults are zero, so an already matching header starts valid bounded historical catch-up.

This refinement adds no SQL field, persisted bytes, scan/probe/seek work, output or source sweep beyond proposal48. Existing bounds100/200/3, four-column203-byte diagnostic accounting, carry155731 cap and parent-required retained-prefix/startup cost selections remain unchanged. The shared load/save invariants high>=after and scan_generation<=passive_generation remain checked; generation increments and overflow rollback are unchanged.

## Required proof before code approval

Keep the existing exact presentation snapshot regression. Add both new-binding operation orders with a partially consumed >100-row publication snapshot and nonzero generation, asserting only one passive reset, preserved after/high/generation after the second owner transition, unchanged presentation allocation/counters after refresh, and fresh ordinary START after actual Pull. Include refresh-only repetition, Pull-only Hook row, actual Watch row, initial defaulted migration row, restart between owners, rollback, and stale old-token Confirm no-effects. Mid-walk ordinary arrival must retain the originally selected freeze-until-cursor/body/carry-exhaustion semantics. These are future implementation evidence obligations; no tests were rerun here.

The exact replacement rule above is source-backed and internally coherent. The old48 checkpoint and my affected CLEAN wording must not authorize the load_raw→load change or unconditional dual reset. A revised selected document/hash and fresh scoped verdict are needed; this report itself is design advice, not source authorization. Only this review artifact was written. No Cargo/source/git/tracker/Herdr mutations or subagents were used.
