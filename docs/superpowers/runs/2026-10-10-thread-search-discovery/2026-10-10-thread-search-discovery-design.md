# Thread list search discovery

## Goal

Make `herdr-threads thread list --search TEXT` discover threads whose optional name or topic contains TEXT as a case-sensitive literal substring. A thread named `psa-global` must be discoverable even when its topic contains only descriptive announcement text, while pagination remains bounded and detects search membership changes after rename.

## Problem description

The interactive picker filters both names and topics, whereas the CLI maps `--search` to the legacy `DirectoryQuery.topic_contains` field and `src/store/queries.rs::directory` evaluates only `topic.contains(needle)`. Names already appear in directory summaries but do not participate in this predicate. Consequently a user can see `psa-global` in the picker and still obtain no CLI search match. The older adopted shared contract explicitly specified topics-only search; this change intentionally amends that discovery contract without changing request field names or exact execution selectors.

## Main challenges

Adding the name predicate changes the set traversed by filtered directory cursors. Current filtered queries bind `topic/all` revision, while `set_thread_name` bumps only `directory/all`; non-recent filtered cursors therefore need a new rename dependency. The fix must preserve indexed candidate traversal, high waters, row/byte/work budgets, zero-match continuation, output framing, instance and membership scope, archival behavior, and recent versus ordinal ordering. Unrelated directory mutations must not acquire new invalidation effects merely to detect names.

## Key decisions made

Keep case-sensitive literal matching and extend its target to name OR topic. Preserve `DirectoryQuery.topic_contains` and all serialized request/result/cursor shapes. Read optional name alongside each existing bounded directory candidate, and apply the predicate before summary admission. Use the existing generic `filter_revisions` table with a dedicated `directory/name/all` revision (`scope_kind = directory`, `scope_key = name/all`), incremented transactionally only on actual name changes. Bind this revision to filtered cursor identity alongside the existing topic revision; retain existing non-search cursor behavior. Update public help, discovery prose, and the historical contract's topics-only statement. No schema or protocol version change is needed.

## Decision points

### Discovery matching

For `Some(needle)`, include a candidate when `topic.contains(needle)` OR its non-null `name.contains(needle)`. `None` leaves candidates unfiltered. An unnamed thread can still match its topic. Empty search text continues matching every candidate because every topic contains the empty string. Whitespace and punctuation remain literal; `%`, `_`, and regex characters have no special meaning. Mixed case and Unicode retain Rust UTF-8 `str::contains` behavior, with no normalization, lowercase conversion or locale rules. A row matching both fields appears once. Case-insensitive matching was considered but rejected because the existing literal case-sensitive contract already supports scripts and this bug only requires restoring missing name discovery. Fuzzy matching was rejected because the picker is an interactive UI, while CLI pagination and scripted discovery require a simple deterministic predicate. Exact name-to-ID execution resolution remains unchanged.

### Candidate traversal and result shape

Extend both directory candidate SELECT projections (ordinal and recent-index traversal) with nullable `name`. Reuse the existing loop and candidate counter; do not add an inventory-wide SQL `LIKE`, extra scan, separate name results, or per-candidate name lookup. Evaluate the OR predicate once for each candidate before its existing summary/encoded-budget handling. Keep existing ordering, high-water snapshot, work limit, max-byte fit oracle, stop reasons, continuation argv and output summaries. The query remains selected-instance scoped and applies the existing membership predicate independently. No membership defaults, `--joined`/`--invited`/`--all`, archive inclusion rules, or picker behavior change.

### Rename consistency

On every committed actual name transition in `set_thread_name` (unnamed to named, named to another name, or named to unnamed), bump `filter_revisions(instance, 'directory', 'name/all')` in the same transaction as the name write and existing event/revision updates. Preserve the existing `directory/all` bump. The existing scope-kind CHECK accepts `directory`; the dedicated `name/all` scope key requires no migration. Same-name no-ops and exact operation replays must not cause another bump. Managed/service-owned name mutation paths must either use the same bump or be shown unable to rename an existing thread; inventory their name-write sites during implementation. New-thread creation remains governed by the existing high water and needs no name-revision bump merely for its initial name.

Read the dedicated `directory/name/all` revision only when `topic_contains.is_some()`, treating missing revision rows as zero. Preserve `topic/all` as the existing cursor filter revision. Add the name revision to the opaque `last_examined_key` string for filtered queries, for both recent and ordinal order, while retaining every current lifecycle/member/activity component. Non-filtered query keys remain byte-for-byte unchanged. A filtered continuation issued before any committed rename in the selected instance must return `CursorStale`, with a restart argv preserving search, scope, ordering and output context. Renames into or out of the result set are covered even when the renamed row was already examined, was not emitted, or lies later in traversal. This is conservatively instance-wide across names: renaming a nonmatching thread still stales a filtered cursor. It does not introduce invalidation on unrelated membership changes beyond the existing query-specific member/recent revisions.

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

2026-10-10: Production mutation regression exposed that `filter_revisions.scope_kind` has `CHECK (scope_kind IN ('directory', 'inbox', 'topic'))`; the planned new `name` kind was rejected. Use existing kind `directory` with dedicated key `name/all`, distinct from key `all`. This preserves the atomic, selected-instance rename dependency and search-only cursor binding without a schema migration; no public shape or trust invariant changes.

2026-10-10: Integration sweep verified combined candidate `65a67dcf6f7bd63d0b0fd8b2665af896bc8ca95f`, including refreshed main `effa14a2` invitation rejection behavior. Fresh task-private builds ran 42 directory regressions, the actual isolated CLI `thread list --search psa-global` regression with an unrelated announcement topic, 16 invitation-rejection seam regressions, and three public help/argv/legacy-wire checks, all passing. Existing regressions cover both bounded traversal orders, zero-match work pages, selected-instance isolation, membership/archive behavior, production set/rename/clear staleness, replay/no-op stability, and legacy filtered cursor rejection; no additional integration gap or source fix was identified. Formatting, all-target/all-feature clippy, default-feature guard and final owned-process leak check passed. No full suite was run; exact commands and private build provenance are recorded in the task-3 report.
