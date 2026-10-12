# Publication CHECK and released39 epoch proofs: round 2 source review

Spec CLEAN

Quality CLEAN

Exact reviewed source artifacts:

- Complete publication block, including shared measurement helper: 10,929 bytes, SHA-256 `89500827eebe29a0c7187dd89a9c02413259b55c5ce788af23f0c9ef81710d87`.
- Complete new released39 epoch block: 8,901 bytes, SHA-256 `e657761a07a33a6ebe755e423c175eb660ec29d0115751c2e69dd33b877ef718`.
- Complete round-2 delta: 6,426 bytes, SHA-256 `8daafbcb45ee0f010638573bc64f4c87443380cce6228b6ece0d896725b0e73a`.

Read both complete cumulative blocks, the entire refreshed report, the complete delta, current D target prerequisites, migration selection and the actual schema/trigger seams used by the new fixture. No Cargo, test execution, live source application, Git mutation, or substitute system-SQLite experiment was performed. These are source-only verdicts; current corrected tests have no execution approval or passing claim.

## Findings and disposition

The original mixed hook/watch exact-affine finding remains ADDRESSED: independent homogeneous hook and watch series are retained. The subsequently observed runtime failure of the no-CHECK constant-work assumption is ADDRESSED in the proposed assertions and report. My round-1 source approval did not establish that assumption; the actual runtime result supersedes it. This correction is to test assumptions, not a production behavior fix.

Both checked and matched unchecked series must now have positive retained-row growth. For each, the increment from 1 to 128 rows must fall between four and five times the increment from 1 to 32 rows, bracketing the count ratio 127/31 while permitting overhead. This is an explicit fixture acceptance bound, not a universal SQLite complexity theorem. It catches constant work and substantial superlinear growth at the measured sizes without asserting exact whole-series affine ticks.

Separately, checked-minus-control must be positive at one row and scale by 32/128 from that measured one-row difference. The coefficient is not hardcoded as 3/6/9/12. Both databases use the same actual schema, homogeneous values, columns added in the same order, and paired DDL differing only by the added scalar CHECKs; through successive ALTERs, the difference therefore includes cumulative additional checks. All null choices, JSON values, masks, range/length inputs and consumer branches are identical within each series. Unique non-NULL fixed-width seat IDs are not subject to a varying content CHECK. No source-level inconsistency remains in that differential measurement. Its exact scaling is still an assertion for D's linked runtime to validate; failure must be investigated, not relabeled as success.

No blocking findings from the complete review. One nonblocking wording nit remains in the publication block: the inherited comment saying the control “isolates fixed DDL from validation of retained rows” is imprecise now that baseline validation is known to scale too. The code and authoritative report handle that correctly. Prefer “matches baseline DDL and existing validation; subtraction isolates added CHECK work” when next editing that comment. This does not change the reviewed behavior or authorize an unreviewed source change.

## Full test-change assessment

Publication preservation, defaults, rejection and counter checks remain intact: full actual V40 consumer table and its token indexes; exact four V41 ALTERs; SQL-schema-valid retained rows of both supported kinds; complete old-column snapshots after ALTER and failed update; zero defaults on every row; independent negative scalar refusals; positive active outer counters and checked work exceeding matched control. Removing known-false constant/whole-affine assertions is justified by the disclosed actual failure and replacement with meaningful growth and differential assertions. It does not remove the migration invariants under test. SQL-valid empty control objects/generic state strings remain explicitly excluded from canonical runtime-decoder or full-upgrade proof.

The new epoch test uses exact include_str paths for every migration 1 through 39, in exact numeric order, with no omissions, substitutions or duplicate entries. Each connection executes those real migrations on an empty database, explicitly enables/verifies foreign keys before populating data, and selects the exact epoch ALTER from actual migration40. No reduced invitation relation, renamed shadow table, writable_schema, or dropped constraint/trigger is used. Setting user_version to39 labels this private SQL fixture; it does not impersonate the production migration runner.

Actual parent/schema shapes support the fixture: host instance defaults, native resolved seats, a same-instance thread, joined origin membership and open interval, invited recipient memberships with episode1, and pending timed invitations with positive unique decision sequences, nonzero frozen duration/deadline and immutable original inviter. The modern confirmation-origin insert guard is satisfied. Initial rejection projection is the permitted zero default; pending invitation triggers maintain the digest. Final host decision_seq covers all seeded decisions. No required service episode is needed for these native timed invitations. The absence of a public creation-info event is disclosed: this is a real-relation SQL witness, not proof of a public invite transaction.

Before/after checks preserve every old invitation column, complete pending digest rows, unrelated catalog SQL/rootpages, invitation rootpage and foreign-key declarations. They require clean foreign_key_check before and after, zero epochs for all rows, exact INTEGER/NOT NULL/default0 column shape, and unchanged user_version39 after applying only the single ALTER. The epoch probe requires both counters active and prints actual version/counters for 1/32/128 rows, with no assumed constant or linear cost. It cannot yet support a migration-cost conclusion; D must retain actual execution output and reconcile production documentation with it.

## Shared helper and API safety

Extracting `presentation_sqlite_measured` to module scope allows both tests to call one implementation. Current module imports provide Connection and StatementStatus; all other imports are local. There is no existing function-name collision. Relative include paths are correct for the eventual `tests/store/invitation_content.rs` location. Iterating the released [&str;39] array for each connection is valid because the array is Copy. Existing three tests and target registration remain untouched.

The unchanged measurement implementation uses the project's rusqlite0.39 public ffi/unsafe handle and StatementStatus API. A stable Box<Cell<u64>> outlives its registered callback. Connection-local interval1 registration occurs after prepare; the synchronous callback performs saturating arithmetic without allocation, SQLite reentry or unwinding. The guard unregisters before the counter is freed on success, returned error or panic. Each private connection is used serially; no competing handler/global mutation is introduced. Outer VmStep and nested progress remain distinct, and neither is claimed to account for all C/page/CPU costs. No new feature, dependency, process or product/model-visible string is added.

## Exact baseline and remaining gates

At read time the active D target is 16,523 bytes, SHA-256 `a3452caf1072e9a070ec949024b777fe37f54d87e3917ab8c9702c88a99745b6`. It contains the round-1 block exactly once. Its preceding 6,060 original bytes still hash to `1a5bd5d0580e8ffd611a806fda9c07b214c992529c8449e3ea126912bf9c1c0f`, followed by one separator newline. Actual migrations40/41 remain respectively `83e7348a425907dc7ad55021960812e933f4126cdaacefbb7f90d28382118326` and `0142aa5a67146fbc5bd77380609a86b7f892b32704fb4a2f32f36ba35f3523c7`.

Verified archived round-1 block remains 10,462 bytes/hash `23038740284131385450df6c739fbc386c230cbbf606466804fdf648f50b8ff5`; its archived report hash is `f352e0bdc3ea8c7876036dfb25a6c55c286fe4d5700c417dfcaded4350da7adf`. Original archives/reviews remain historical evidence. The refreshed report explicitly supersedes their constant-control assumptions. Prior runtime numbers are attributed to coordinator-supplied execution; this review did not independently run those measurements or receive a raw-log path in this assignment.

D is active: NO application is approved while it writes. Require a new positive quiescent ACK and exact current target check before replacing only the old round-1 appended block with these two blocks together, defining the helper once. Then D must compile and execute both actual selected tests under the required locked/all-features gate, retain nonzero selection counts and raw --nocapture measurements, and investigate any failures. Whole D schema, interface, behavior and package gates remain separate. No green, completed or merge-ready claim is made.
