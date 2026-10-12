Spec NEEDS_FIX
Quality NEEDS_FIX

### Spec compliance

- Reviewed integration BASE `58da51c13aa630fa512b6bdd20209468ae0c1adc` → Task4 HEAD `d23ecbf7576912d603b168158a11dbbca9593426`. Read the full 2,947-line / 127,605-byte review package once in four sequential sections, plus task brief, report, initial review, I1 re-review, seam-fix report and base-composition contract. This is a scoped rebase/seam review, not a new whole-branch review.
- The production source and migration composition inspected are sound, but two latest-version fixture guards remain at provisional29. They prevent the required focused topology validation from reaching its behavioral assertions (Important S1).

### Strengths and overlap checks

- `src/store/schema.rs:583`, `src/store/schema.rs:609`, `src/store/schema.rs:611`: old versions audit the genuine bootstrap28 shape, invitation29 is verified, receipt30 runs and commits separately, then handoff31 runs separately. Current31 audits the widened shape. Fresh0 first commits ordinary0001..0029/version29 and then follows the same receipt30/handoff31 ordering; neither rebuild is nested in the fresh transaction.
- `src/store/schema.rs:658`, `src/store/schema.rs:4078`: both rebuild wrappers require autocommit, disable FK outside BEGIN, audit their resulting DDL and complete FK graph before updating version/commit, roll back failures and restore the caller's FK setting. Receipt cutover capture remains exclusively in immutable30;31 neither invokes30 nor captures its marker again.
- `migrations/0031_handoff_body_bounds.sql:4`:31 copies the parent/recovery blobs at original names without rename-based FK rewriting. All columns, operation keys and dependent-table relations survive; unchanged trigger/index contracts are audited against28 with only the two declared bounds substituted (`src/store/schema.rs:4123`). Read-only byte comparison against the integration checkout found all30 migration files0001..0030 identical. No historical SQL mutation is present in the diff.
- `tests/store/topology_handoff_schema.rs:334`, `tests/store/topology_handoff_schema.rs:426`, `tests/store/topology_handoff_schema.rs:599`: new fixtures use true28/29/30 prefixes, preserve schema cutpoints while seeding, compare six populated bootstrap tables and canonical completed/recovery history, physically reopen the populated database, retain non-rebuilt roots and existing30 cutover, and exercise31 failure/retry across FK OFF/ON without recapturing cutover. Fresh/older current-DDL fixtures compare against actual31. The exact1MiB/max+1 recovery test retains its assertions (`tests/store/topology_attempts.rs:218`).
- All six overlapping files were checked: `TRUST-POLICY.md:516` documents the approved body-bearing bounds without altering receipt/attribution policy; `src/cli/commands.rs:1291` and `src/cli/commands.rs:2570` retain the exclusive source/read-once parser; `src/cli/handoff.rs:114` preserves body validation separately from topic/goal validation; `src/cli/journal.rs:1355` retains the I1 Handoff|HandoffDelivery exact512KiB cap before allocator reservation/publication; `src/store/schema.rs:583` composes invitation29/receipt30/handoff31; `tests/cli/handoff_delivery.rs:2010` and `tests/cli/handoff_delivery.rs:2638` keep guarded canonical delivery and real wire/retry tests. No semantic loss was found in the five clean automerges.
- `src/cli/journal.rs:1355`, `tests/cli/handoff.rs:4060`, `tests/cli/handoff.rs:3907`: the approved I1 prepublication match arm and actual oversized public-producer regression survive alongside normal maximum-body frozen retry. Generic historical Journal::load/decode_reader acceptance, global wire1MiB, metadata-only128KiB and non-handoff archival caps are unchanged.
- `src/cli/handoff_delivery.rs:604` preflights the exact nested terminal before canonical completion; retained exact original bytes, digests and keys are preserved. Body-bearing bound increases remain scoped to the previously approved serializers and loaders.

### Issues

#### Critical

- None found.

#### Important

- **S1 — `tests/store/topology_handoff.rs:70` and `tests/store/topology_handoff.rs:1334`: latest-schema assertions still expect29 after public `schema::initialize` now upgrades to31.** The first assertion is in shared `setup`, used to produce the cached fixture image for the topology suites; every such consumer panics before its behavioral test. The second breaks the standalone fresh/canonical26 upgrade regression. The imports directly use `herdr_threads::store::schema` (`tests/store/topology_handoff.rs:5`), so there is no facade or historical cutpoint that makes29 correct. Update these two post-initialize guards to31, preserving actual historical28/29/30 cutpoint assertions in the schema fixtures. Then run the already queued focused connection/topology selections on the corrected exact tip. This is an obvious guard contradiction; no Cargo invocation is needed to establish it.

#### Minor

- No additional code finding. Earlier production-source rebuild overruns remain honestly recorded in `task-4-report.md`; the warm I1 lint16.41s does not establish that every production-source incremental build meets the one-minute budget.

### Named focused checks and limits

- Source/effect ordering risk: checked unchanged `src/cli/mod.rs:346` and `src/cli/input.rs:35`; parsing/acquisition precedes execution, and file/stdin reads stop at65,537 bytes with nonempty validUTF-8 validation. Retry freezes the acquired String, not a source path.
- Legacy publication/launch durability risk: checked unchanged producer ordering at `src/cli/handoff.rs:650`; native preflight precedes Journal::record, and canonical mutations/native execution occur only through resume after record. The cap in the reviewed writer therefore refuses before the required durable/effect boundary.
- Receipt capability/API preservation risk: checked unchanged `src/cli/mod.rs:3063` and `src/protocol/capabilities.rs:19`; send-time discovery still refuses unsupported peers and preserves the named capability. Public wire command/confirmation fields remain byte-identical to the integration checkout in `src/protocol/commands.rs`; the existing legacy invitation adaptation is still `confirmation: None` at `src/cli/handoff.rs:229`.
- Migration-before-use risk: checked unchanged `StoreContext::open_writer` at `src/store/connection.rs:168`; schema initialization must succeed before a writer is returned. This supports fail-closed migration ordering; it is not a runtime GREEN claim.
- Discovery/control composition risk: checked the directory path at `src/cli/commands.rs:2101`, literal name/topic matching at `src/store/queries.rs:1753`, filtered cursor name revision at `src/store/queries.rs:1626`, and name mutation revision bump at `src/store/control.rs:1357`. `queries.rs`, `control.rs`, `protocol/commands.rs`, `protocol/capabilities.rs` and `cli/mod.rs` are byte-identical to the integration checkout. Upstream name/topic search and control revision composition survive.
- Changed-file context exceptions: read missing fresh-initialization and unchanged receipt-wrapper context in schema.rs to evaluate the explicitly requested transaction seam, the topology test imports to resolve S1, and the explicit legacy producer body to verify the named durability boundary. No unrelated source crawl or source mutation was performed.
- Mandatory test review exception: ran only the specifically required test-only `git diff --stat BASE..HEAD -- tests` and `git diff BASE..HEAD -- tests | head -400`; eight files,1073 insertions/24 deletions. The second output was deliberately truncated at400 lines. All complete test hunks were reviewed in the full package. No unauthorized skip, deletion, assertion weakening or timeout increase was found. Whitespace-invalid becomes empty-invalid under the intended message contract; the exact recovery expectation follows approved1MiB; staged-prefix setup preserves its original budget and strengthens progress checks. S1 is an incorrect retained provisional version guard, not an unauthorized relaxation.
- The reported SQLite proof covers populated cyclic topology, opaque blobs/history, physical/sparse receipts and dependents, invitation origin/policy, cutover atomicity and75/140/183 catalog counts; it is supplemental implementer evidence, not independently rerun Rust validation. No Cargo/test reruns, Herdr operations, source/index/branch changes, Git mutations or real-config writes were performed. Only this report was written.
- Focused Rust connection/topology GREEN and exact-tip mandatory merge gates remain pending. The prior original-task/I1 GREEN and static SQL proof do not establish execution GREEN for this rebased tip. Re-review S1 after correction, then obtain focused GREEN and the coordinator-owned exact-tip checks before merge.

### Assessment

Spec NEEDS_FIX
Quality NEEDS_FIX

The rebase retains the approved production semantics and composes the two FK-off rebuilds correctly. Fix the two stale latest-version fixture guards before validating or integrating this tip.
