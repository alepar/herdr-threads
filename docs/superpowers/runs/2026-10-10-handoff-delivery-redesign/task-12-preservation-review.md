# Task 12 A independent preservation review

Reviewed final immutable HEAD `fd7fb6c9fc3182da9572720dedb299a96f036c37` against approved `53426ab6dc726d3406ca4b46654914cd817aa7d9` and released incoming `02041b93aa974cc9c736699f7b79c9d13557d884`. This is the supplemental literal/runtime preservation review, not the separate whole-task or schema verdict.

Spec CLEAN

Quality CLEAN

No Critical or Important preservation defect found in the reviewed scope. No source, Git, tracker, Herdr, or Cargo mutation was performed; this report is the sole file written. Tests were inspected, not rerun.

## Independently checked evidence

- Recomputed immutable Git tree comparisons, rather than trusting author summaries: all 3,157 entries in the unchanged-upstream proof have identical incoming and final blobs. All 368 entries in the actual-composition inventory match the final tree.
- All 31 published migration blobs are exact incoming blobs. All seven approved unpublished migration bodies are exact approved blobs after the explicit 29..35 to 32..38 rename. This includes the native-NULL author attribution fix in receipt-warning-origin SQL; no literal replacement altered its six trigger SELECTs.
- Independently scanned changed source, tests, documentation and migration text for numeric-only line changes against both approved and incoming trees, including strings/comments. Reviewed the complete numeric hunk inventory and relevant migration/include/cutpoint changes. No surviving digest-array-width, deadline, timestamp, batch-limit or fixture-cardinality substitution was found. The digest width remains 32. Numeric additions from exact upstream imports are distinguished from composition changes. Additive 39 capacity changes and fresh aggregate-capacity fixtures are deliberate separate changes, not renumbering.
- The final follow-up changes only `tests/store/topology_attachment_guard.rs` (82 insertions, 7 deletions); production/schema source is identical to c928. Read the entire follow-up and the actual final capacity test. Parent and child argv/body copies and their digests are now updated together; retained report digests are recomputed and retained validators run. These are serializer/storage fixtures, not evidence that public semantic validators currently admit 64 KiB automatic bodies. The separate formal review found that the retained Complete report argv omits the actual startup prompt; its pending correction and revised aggregate measurements are outside this fd7 preservation verdict. Aggregate Complete/Recover capacity proof is therefore explicitly not certified here.

## Runtime preservation

The finite due/backfill runtime is preserved through actual source comparison: `src/scheduler/deadlines.rs`, `src/store/invitation_due.rs`, `src/store/materialization.rs`, `src/store/catch_up.rs` and `src/store/effective.rs` have exact approved blobs. Changes in receipts.rs are confined to the imported delivery-settlement producer at its head; original repair/scan functions retain the captured high waters, shared repair allowance, total 100-row bound, all-visit accounting, lending/end-page logic and transactional reset behavior. Their associated historical fixtures retain the original non-version inputs and assertions after cutpoint remapping.

Canonical original-sender/inviter warning attribution and wake classification remain in the canonical store producers. Exact SQL preservation retains native NULL attribution constrained by actor seat and absent service attribution; it does not infer service authors or rewrite history. Attention changes add incoming pull-notice and wake-on-all support while retaining original warning classification/origin logic. Incoming scheduler hook-pull eligibility gates are a known D-owned successor seam, not certified by this review.

For moved producers, inspection went beyond line-presence evidence:

- The Lazy invitation branch is in neutral `Assembler::push`; it yields InformationalNotice, while timed/historical invitations still yield Attention. The watch regression calls that actual assembler and the drain. Canonical pending invitation metadata remains stored and exposed. Complete new hook-pull invitation context is D-owned and is not claimed here.
- Stable-event proposals run through `plan_settings(kind)` and adapter-owned Codex `plan_codex`, using the same complete event argv producer. Recorded ownership still requires exact manifest groups/fingerprints; canonical native route recognition does not adopt an unrecorded identical registration. Legacy v1/v2 migration and recovery remain separate from stable v3 commands, with installation identity in the manifest. Dynamic SettingsKind lookup replaces fixed adapter enums without widening removal authority to arbitrary commands or permission rules.
- Journal origin-bound validation remains before ordinal reservation/file creation, while incoming one-buffer durable writes are retained. Hermes diagnostic/advisory handling is preserved: the Hermes runtime file is exact approved source, and launch differences are only the incoming neutral PaneShellProbe name. Actually used configuration and launch-input fingerprint checks remain.
- Help moves into the neutral template with body, frozen retry and independent invitation/message deadline explanations retained. Incoming automatic-route semantics are not certified as completed A work.

## Tests reviewed

Inspected test diff statistics and complete applicable preservation diffs. In particular, read the full 736-line incoming-to-final setup test diff in first-400 and complete-remainder chunks, the complete final capacity follow-up and test body, the watch changes, and the entire approved-to-final effective/invitation_due/receipts/schema/topology_handoff_schema composition diff. Changed historical cutpoints reflect the inserted released 29..31 prefix; genuine <=28 versions remain unchanged. Rootpage exclusions/normalization are limited to newly introduced tables or the four explicitly rebuilt capacity tables and their implicit indexes; retained values, catalog definitions and other rootpage assertions stay compared. Foreign-registration, drift, exact ownership, interrupted migration, rollback, retained-row and finite-work assertions were not weakened. The scheduler test case-count 40-to-20 change is exact incoming behavior, not an A change.

## Non-blocking evidence and documentation notes

`task-12-literal-audit.json` is a historical 58-function supplemental inventory, without final_head, and its initialize token sequence reflects an earlier composition (latest 37). It must not be presented alone as exact fd7 proof. The final complete-literal-hunks inventory names fd7 and the immutable-tree checks above were performed independently. Refreshing or explicitly labeling the historical inventory would remove ambiguity.

Two test comments (`tests/integration/channel_activation.rs:214`, `tests/store/handoff_fences.rs:411`) still call the current schema 38; their actual assertions correctly require 39. This is cosmetic and does not invalidate the literal/runtime preservation verdict.

This review does not certify B's automatic policy/body/capability/aggregate preflight or scoped wire response implementation, D's full invitation context/pull warning eligibility, C's combined behavior, the whole epic, or final merge readiness. It does not convert author-recorded passing tests into independently rerun tests. No final source verdict is inferred from raw line matching alone.

## Round 1 final follow-up: e196ae11

Reviewed the complete exact `fd7fb6c9..e196ae1140499612d63bddc43a1624b70d004c62` delta and the revised report. Independently compared immutable trees: the only changed path is `tests/store/topology_attachment_guard.rs` (101 insertions, 54 deletions); production/schema is unchanged. Final tree is `6ab6831b7972af18ce600b553647db7185c31ccc`. The 368-entry composition map has zero final-blob mismatches; authoritative complete literal hunks name this exact HEAD/tree. The historical 58-function inventory is now explicitly PRELIMINARY_SUPERSEDED, resolving the evidence ambiguity above.

The new retained-report helper constructs actual startup argv using frozen retained_prompts and either the stored template or frozen bootstrap composition. Its relation assertion matches the real canonical store `validate_report` branches, including the same child context, instance and retained report argv. It separately asserts that startup prompt composition differs from bare caller argv, verifies parent/child body and argv equality, recomputes and checks parent/child digests, and validates the retained report digest. The small-body positive control additionally runs full AutomaticBootstrapMutation::Complete.validate. Escaped argv values now use grammar-admitted --model= prefixes; 4,088 control bytes plus the 8-byte prefix preserve the original 4,096-byte per-argument limit and the 16-argument maximum. Near-boundary serialization recalculates all nested copies and uses the same helper. Existing wire/SQL limit assertions remain; no assertion is relaxed to hide the previous incomplete retained argv.

Spec CLEAN

Quality CLEAN

These are scoped preservation verdicts for final e196ae11, with no Critical or Important finding in this follow-up. The earlier fd7 aggregate-proof exclusion is historical: the missing retained startup prompt has been corrected. The revised measurements are coherent canonical argv-relation serializer evidence. Extended-body public admission still intentionally bypasses B-owned limits, so this review does not certify B preflight, scoped response transport, actual accepted public 64 KiB flows, C combined proof, or whole-task merge readiness. The formal reviewer owns that wider verdict. Capacity 1/1, canonical store Complete 1/1 and lint/fmt/neutral/leak passes are author-recorded evidence, not tests independently rerun here.
