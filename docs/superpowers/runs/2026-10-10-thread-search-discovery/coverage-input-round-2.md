# Root coverage review round 2

Fresh-context, input-bounded review. The contents of this packet are the entire permitted review window. No repository exploration or tracker queries.

## Goal
Make `herdr-threads thread list --search TEXT` discover threads whose optional name or topic contains TEXT as a case-sensitive literal substring. A thread named `psa-global` must be discoverable even when its topic contains only descriptive announcement text, while pagination remains bounded and detects search membership changes after rename.

## Parent goal chain
Empty (root)

## Spec
# Thread list search discovery

## Goal

Make `herdr-threads thread list --search TEXT` discover threads whose optional name or topic contains TEXT as a case-sensitive literal substring. A thread named `psa-global` must be discoverable even when its topic contains only descriptive announcement text, while pagination remains bounded and detects search membership changes after rename.

## Problem description

The interactive picker filters both names and topics, whereas the CLI maps `--search` to the legacy `DirectoryQuery.topic_contains` field and `src/store/queries.rs::directory` evaluates only `topic.contains(needle)`. Names already appear in directory summaries but do not participate in this predicate. Consequently a user can see `psa-global` in the picker and still obtain no CLI search match. The older adopted shared contract explicitly specified topics-only search; this change intentionally amends that discovery contract without changing request field names or exact execution selectors.

## Main challenges

Adding the name predicate changes the set traversed by filtered directory cursors. Current filtered queries bind `topic/all` revision, while `set_thread_name` bumps only `directory/all`; non-recent filtered cursors therefore need a new rename dependency. The fix must preserve indexed candidate traversal, high waters, row/byte/work budgets, zero-match continuation, output framing, instance and membership scope, archival behavior, and recent versus ordinal ordering. Unrelated directory mutations must not acquire new invalidation effects merely to detect names.

## Key decisions made

Keep case-sensitive literal matching and extend its target to name OR topic. Preserve `DirectoryQuery.topic_contains` and all serialized request/result/cursor shapes. Read optional name alongside each existing bounded directory candidate, and apply the predicate before summary admission. Use the existing generic `filter_revisions` table with a dedicated `name/all` revision, incremented transactionally only on actual name changes. Bind this revision to filtered cursor identity alongside the existing topic revision; retain existing non-search cursor behavior. Update public help, discovery prose, and the historical contract's topics-only statement. No schema or protocol version change is needed.

## Decision points

### Discovery matching

For `Some(needle)`, include a candidate when `topic.contains(needle)` OR its non-null `name.contains(needle)`. `None` leaves candidates unfiltered. An unnamed thread can still match its topic. Empty search text continues matching every candidate because every topic contains the empty string. Whitespace and punctuation remain literal; `%`, `_`, and regex characters have no special meaning. Mixed case and Unicode retain Rust UTF-8 `str::contains` behavior, with no normalization, lowercase conversion or locale rules. A row matching both fields appears once. Case-insensitive matching was considered but rejected because the existing literal case-sensitive contract already supports scripts and this bug only requires restoring missing name discovery. Fuzzy matching was rejected because the picker is an interactive UI, while CLI pagination and scripted discovery require a simple deterministic predicate. Exact name-to-ID execution resolution remains unchanged.

### Candidate traversal and result shape

Extend both directory candidate SELECT projections (ordinal and recent-index traversal) with nullable `name`. Reuse the existing loop and candidate counter; do not add an inventory-wide SQL `LIKE`, extra scan, separate name results, or per-candidate name lookup. Evaluate the OR predicate once for each candidate before its existing summary/encoded-budget handling. Keep existing ordering, high-water snapshot, work limit, max-byte fit oracle, stop reasons, continuation argv and output summaries. The query remains selected-instance scoped and applies the existing membership predicate independently. No membership defaults, `--joined`/`--invited`/`--all`, archive inclusion rules, or picker behavior change.

### Rename consistency

On every committed actual name transition in `set_thread_name` (unnamed to named, named to another name, or named to unnamed), bump `filter_revisions(instance, 'name', 'all')` in the same transaction as the name write and existing event/revision updates. Preserve the existing `directory/all` bump. The generic revision table accepts this scope without migration. Same-name no-ops and exact operation replays must not cause another bump. Managed/service-owned name mutation paths must either use the same bump or be shown unable to rename an existing thread; inventory their name-write sites during implementation. New-thread creation remains governed by the existing high water and needs no name-revision bump merely for its initial name.

Read `name/all` only when `topic_contains.is_some()`, treating missing revision rows as zero. Preserve `topic/all` as the existing cursor filter revision. Add the name revision to the opaque `last_examined_key` string for filtered queries, for both recent and ordinal order, while retaining every current lifecycle/member/activity component. Non-filtered query keys remain byte-for-byte unchanged. A filtered continuation issued before any committed rename in the selected instance must return `CursorStale`, with a restart argv preserving search, scope, ordering and output context. Renames into or out of the result set are covered even when the renamed row was already examined, was not emitted, or lies later in traversal. This is conservatively instance-wide across names: renaming a nonmatching thread still stales a filtered cursor. It does not introduce invalidation on unrelated membership changes beyond the existing query-specific member/recent revisions.

Using `directory/all` instead of a dedicated name revision was considered and rejected because it includes unrelated membership, lifecycle and control activity. Combining arithmetic revision counters was rejected because separate dependencies are clearer and avoid overflow/collision choices. A schema migration or new public cursor field was rejected because the existing generic revision storage and opaque key are sufficient. No semantic-version marker is required: old filtered cursor keys lack the new name component and therefore fail comparison with a normal restart hint; old unfiltered cursors keep their previous semantics.

### Documentation and compatibility

Document `thread list --search` as case-sensitive literal substring matching of names or topics in `src/cli/commands.rs` flag help and the README discovery paragraph; update other actively maintained command documentation if its existing text claims topics-only behavior. Amend `docs/design/herdr-threads/shared-contract-amendment-adopted.md` section 4 to state that its earlier topics-only directory-search rule is superseded by this design, while the separate top-level topic/body `search` command remains unchanged. Document `DirectoryQuery.topic_contains` as a legacy field name whose directory semantics now cover both fields; retain serde spelling, validation, continuation argv and wire version. Previously excluded name-only matches are an intentional behavior expansion. No public permission, caller attribution, receipt provenance or trust invariant changes; `TRUST-POLICY.md` therefore requires no amendment.

### Validation

Use focused regressions in the existing explicit test targets. Store-query tests cover name-only `psa-global`, topic-only, both-field deduplication, unnamed/no-match, case sensitivity, UTF-8 and literal punctuation. Exercise ordinary and recent ordering, selected-instance isolation, default/joined/invited/all membership behavior, and archived rows according to current behavior. A bounded pagination case must put the name-only hit beyond at least one zero-match work-limited page, then traverse continuations without duplicates or skips. Keep existing row/byte budget and continuation round-trip tests relevant to the modified directory loop.

Exercise actual name mutations through production store dispatch, rather than only manually updating revision rows: obtain a filtered continuation and rename a thread into the match set, out of it, or clear its matching name; each returns `CursorStale` with the right restart command. Cover recent and non-recent queries, a previously skipped and a later candidate, unchanged-name no-op/replay stability, instance isolation, and a non-search continuation whose existing semantics are preserved. Keep the existing topic-edit stale regression and prove unrelated membership activity does not newly stale an all-membership, non-recent filtered cursor. Parser/contract tests confirm the legacy `topic_contains` request field and regenerated search argv round-trip unchanged.

Required verification is `cargo fmt`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and relevant `nice cargo test --locked --all-features <filter>` runs. Before integration run `nice scripts/check-default-features`. Do not run the full suite routinely while ht-zo4 remains open. Tests use isolated storage, user configuration and private servers only; any owned processes must stop before reporting completion, and a full-suite run if separately required must use the prescribed leak-run ID/check.

### Scope and implementation boundaries

One store-focused leaf can own candidate projection, the predicate, dedicated name revision publication/binding, and focused store regressions because those changes share a correctness invariant. A documentation/CLI-contract leaf may follow that seam for help, protocol field comments, normative amendment and parser/contract verification. Shared `src/store/queries.rs` edits must remain serialized. Do not refactor the picker, add fuzzy CLI search, search message bodies/goals/IDs, alter name resolution, change membership/archival policy, restart the shared daemon, or deploy/install the branch as part of this run.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

## Task tree (complete unabridged bulk tracker dump)
[
  {
    "id": "ht-akx.2",
    "title": "Document literal thread-name discovery in CLI help and contract",
    "description": "Update CLI --search help and README examples to state case-sensitive literal substring matching against thread name OR topic, rather than topic-only behavior. Amend docs/design/herdr-threads/shared-contract-amendment-adopted.md section 4 explicitly while preserving bounds and generic message topic/body search. Document picker fuzzy search as distinct only where needed for clarity; no broad redesign. Keep wire DirectoryQuery.topic_contains field and APIs unchanged. Files-touched: src/cli/commands.rs (--search declaration around line 1056), README.md, docs/design/herdr-threads/shared-contract-amendment-adopted.md, focused existing CLI argument tests if needed. Acceptance: --help clearly names both fields and literal case sensitivity, docs have no current contradictory discovery assertion, docs identify actual existing scopes without promising automatic all-page CLI traversal. This task consumes the already-decided spec search semantics; it can proceed independently of behavior code.\nMandatory protocol field comment: document DirectoryQuery.topic_contains in src/protocol/commands.rs (or its actual defining file) as the retained legacy wire name for literal name-or-topic matching, preserving serde spelling. owns: public CLI and protocol discovery documentation; consumes: decided literal name-or-topic specification.\nMandatory focused parser/contract verification: serialized DirectoryQuery keeps topic_contains and regenerated search continuation argv round-trip unchanged, preserving search text, scope, ordering and output context. Reuse existing tests where they already prove these assertions; add missing meaningful checks.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T19:37:49Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T19:41:10Z",
    "labels": [
      "sp:ht-akx"
    ],
    "dependencies": [
      {
        "issue_id": "ht-akx.2",
        "depends_on_id": "ht-akx",
        "type": "parent-child",
        "created_at": "2026-10-10T19:37:49Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-akx"
  },
  {
    "id": "ht-akx.1",
    "title": "Implement bounded name-or-topic directory matching and rename-safe cursors",
    "description": "Implement case-sensitive literal substring OR matching across canonical optional thread name and topic in existing DirectoryQuery.topic_contains. Preserve bounded per-candidate traversal, output/page/high-water behavior and existing membership/archive/recent semantics; no canonical-ID matching and no picker changes. Add dedicated name/all filter revision on actual SetThreadName changes (set, rename, clear), no-op changes do not invalidate; bind name revision alongside existing topic/all only for filtered directory cursors, with legacy cursor/filter safety. Own focused meaningful store and CLI regression tests covering name-only/topic-only hits, None/empty needles, unnamed threads, both fields deduped, case-sensitive misses, metacharacters literal, matching/nonmatching pagination, membership/recent/archive scopes, rename entering/leaving match set after first page, rename no-op, topic changes, and unrelated directory writes preserving filtered continuation. Preserve all protocol field/shape compatibility and message-search behavior. Files-touched: src/store/queries.rs, src/store/control.rs, tests/store/queries.rs, tests/store/cooperative_controls.rs or current name mutation suite, tests/cli/read_cost_names.rs (reuse isolated harness), protocol comments if needed. owns: canonical directory search predicate and name revision/cursor binding; consumes: existing DirectoryQuery.topic_contains and SetThreadName canonical transaction. Acceptance: focused regressions pass with unchanged limits and exact CLI --search name discovery; cargo fmt and per-change clippy pass. Read TRUST-POLICY before control.rs changes; update policy only if changing invariant, which is outside intended scope.\nInventory every canonical and managed/service thread-name SQL write site. Every actual existing-thread name change must publish name/all in its deciding transaction, or demonstrate the path cannot rename an existing thread. New-thread initial names remain high-water controlled.\nMandatory verification: run nice scripts/check-default-features before integration. Required regression detail: place a name-only hit beyond at least one work-limited zero-match page; follow cursors without duplicates/skips. Obtain mutation-driven stale results through production store dispatch, including set, rename and clear. Verify exact-operation replay stability as well as unchanged-name no-op stability.\nAlso verify selected-instance isolation, clearing a matching name, and unfiltered continuation behavior remains unchanged.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T19:37:39Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T19:41:31Z",
    "labels": [
      "sp:ht-akx"
    ],
    "dependencies": [
      {
        "issue_id": "ht-akx.1",
        "depends_on_id": "ht-akx",
        "type": "parent-child",
        "created_at": "2026-10-10T19:37:39Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-akx"
  },
  {
    "id": "ht-akx",
    "title": "Make thread list search discover names and topics",
    "description": "Fix CLI discovery mismatch where --search psa-global misses named threads because only topics are searched. Deliver bounded literal name-or-topic discovery, cursor correctness, compatibility and documented CLI contract. Spec will be committed in docs/superpowers/runs/2026-10-10-thread-search-discovery.",
    "spec_id": "docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md",
    "status": "open",
    "priority": 2,
    "issue_type": "epic",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T19:37:09Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T19:38:58Z",
    "labels": [
      "sp:ht-akx"
    ],
    "dependency_count": 0,
    "dependent_count": 0,
    "comment_count": 0
  }
]

## Flagged tasks
None

## Changes since previous round
ht-akx.1 amended for C1 required default-feature check and C3 explicit cursor/production/replay/pagination/isolation regressions; ht-akx.2 amended for C2 required legacy field/continuation parser contract checks.

## Rejected-findings ledger
# Coverage ledger

Round 1: 3/3 valid fresh-context root reviews; all mapped R1-R6, R7 partial.
C1 GAP verification/default-features — reported A/B/C; verified spec command omitted from task acceptance; auto applied explicit ownership to ht-akx.1.
C2 GAP parser/contract round-trip — reported A; verified optional parser assignment omitted mandatory contract assertions; auto applied explicit ownership to ht-akx.2.
C3 GAP cursor regression specificity — reported B/C; verified spec required work-limited zero-match, production-dispatch set/rename/clear, replay stability, instance isolation and unfiltered continuation assertions omitted from task description; auto extended ht-akx.1.
No edge/seam/orphan/configuration/flag findings; no promotions; no rejected findings.
requirements: 7 · mapped: 7 · unmapped: 0



## Requirements (canonical)
R1: Directory --search uses case-sensitive literal substring across optional name OR topic; deduplicate dual hits, retain None/empty behavior and literal UTF-8 punctuation semantics.
R2: Candidate traversal, high water, row/byte/work bounds, zero-match continuation, ordering, instance and existing membership/archive semantics remain intact.
R3: Every actual existing-thread name mutation publishes dedicated name/all atomically; no-op/replay and initial creation keep correct semantics.
R4: Filtered recent and ordinal cursors bind name and existing topic revisions, stale and restart correctly after relevant changes, preserve unfiltered/legacy safety and avoid unrelated invalidation.
R5: Serialized request/result/cursor shapes and legacy field names remain compatible; top-level search, picker, trust/identity semantics unchanged.
R6: CLI help, README, adopted contract amendment and protocol field comment explain exact new contract.
R7: Focused production-path regressions exercise relevant literal/scoped/pagination/rename cases and required fmt, clippy and default-feature checks validate change without routine full-suite use.
