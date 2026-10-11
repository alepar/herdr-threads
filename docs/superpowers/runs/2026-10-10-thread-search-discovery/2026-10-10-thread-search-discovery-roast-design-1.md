---
super-roast verdict: clean (0 nits)
mode: design        iteration: 1 of 3
profile (assumed): Internal local same-user cooperative CLI/daemon with bounded SQLite directory queries. The design introduces no new permissions, trust or wire changes and has limited operational blast radius. Violation of its stated core purpose remains a Blocking severity floor.
inputs: docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md; actual settled tree below
coverage: premortem, completeness, yagni, failure-mode, feasibility, domain:database-query-pagination, domain:transactional-consistency, domain:api-compatibility (8/8 scouts returned; triage and dedupe completed) · 1 raw → 1 deduped → 1 panel / 0 spot / 0 promoted · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 0/1/0 · refute 0/1/0 · ground 0/1/0

## Confirmed findings

- none

## Not verified (beyond panel cap)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

- Rename consistency; Candidate traversal and result shape; Validation — The claimed gap would allow candidate names from an older state to be combined with a newer name revision, admitting a continuation that skips a newly matching thread.
  verdict: rejected (reproduce REJECT / refute REJECT / ground REJECT).
  evidence: All three seats identify the existing read transaction as the mechanism that defeats the premise. `src/store/queries.rs::query_with_output_for_caller` starts `BEGIN DEFERRED` at line 214, before `Command::Directory` dispatch at line 277, and keeps that transaction open through candidate traversal, revision reads and cursor creation. The reproduce seat additionally identifies the separate read-only connection from `src/store/connection.rs::open_query`. The seats ground the snapshot behavior in [SQLite isolation documentation](https://www.sqlite.org/isolation.html): writes committed during an active read transaction remain invisible to that transaction. Because the design preserves the existing query loop and snapshot, candidate names and the captured name revision share a snapshot; a later rename leaves the old revision in the cursor and makes continuation stale. No seat leaves the external premise unresolved. Restating the transaction boundary would be a documentation suggestion, not a confirmed defect.

## Unverified nits (spot-checked)

- none

## Escalations (need human)

- none
---

## Appendix — Actual settled task tree

The following is the full unabridged bulk JSON from `settled-tree.json`, captured after the task acceptance and file-hint corrections. The spec reviewed by scouts was unchanged by those corrections.

```json
[
  {
    "id": "ht-akx.3",
    "title": "Integration sweep: bounded CLI thread-name discovery",
    "description": "Verify the combined CLI name-or-topic search main flow and docs against the settled spec. Use isolated existing harnesses, never shared Herdr service/config writes. Exercise actual CLI thread list --search psa-global on an isolated named thread with unrelated announcement topic, and production rename/set/clear cursor staleness; inspect missing integration coverage not already owned by the leaf regressions, add meaningful tests only for gaps, fix small integration errors inline and file blockers for larger gaps. Sweep for unwired name revisions, candidate projections or help/legacy field semantics. Review exact combined tree, run focused relevant tests, cargo fmt, prescribed clippy and scripts/check-default-features as needed; final expensive once-per-branch checks belong to super-auto after code roast, do not run full suite routinely. Files-touched: focused existing tests/store/queries.rs and tests/cli/read_cost_names.rs if uncovered seams need tests, implementation files only for small integration gaps, spec Post-Implementation Notes and verification evidence. owns: integrated goal verification and uncovered integration tests; consumes: implemented bounded discovery behavior and public CLI/protocol contract. Acceptance: exact CLI discovery and bounded rename-safe continuation satisfy spec using behavior (needs: ht-akx.1) and public help/contracts (needs: ht-akx.2). blocked-by ht-akx.1: consumes all leaves (integration sweep). blocked-by ht-akx.2: consumes all leaves (integration sweep).",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T19:42:11Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T19:42:11Z",
    "labels": [
      "sp:ht-akx"
    ],
    "dependencies": [
      {
        "issue_id": "ht-akx.3",
        "depends_on_id": "ht-akx.1",
        "type": "blocks",
        "created_at": "2026-10-10T19:42:23Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-akx.3",
        "depends_on_id": "ht-akx.2",
        "type": "blocks",
        "created_at": "2026-10-10T19:42:24Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      },
      {
        "issue_id": "ht-akx.3",
        "depends_on_id": "ht-akx",
        "type": "parent-child",
        "created_at": "2026-10-10T19:42:11Z",
        "created_by": "Alexey Parfenov",
        "metadata": "{}"
      }
    ],
    "dependency_count": 2,
    "dependent_count": 0,
    "comment_count": 0,
    "parent": "ht-akx"
  },
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
    "dependent_count": 1,
    "comment_count": 0,
    "parent": "ht-akx"
  },
  {
    "id": "ht-akx.1",
    "title": "Implement bounded name-or-topic directory matching and rename-safe cursors",
    "description": "Implement case-sensitive literal substring OR matching across canonical optional thread name and topic in existing DirectoryQuery.topic_contains. Preserve bounded per-candidate traversal, output/page/high-water behavior and existing membership/archive/recent semantics; no canonical-ID matching and no picker changes. Add dedicated name/all filter revision on actual SetThreadName changes (set, rename, clear), no-op changes do not invalidate; bind name revision alongside existing topic/all only for filtered directory cursors, with legacy cursor/filter safety. Own focused meaningful store and CLI regression tests covering name-only/topic-only hits, None/empty needles, unnamed threads, both fields deduped, case-sensitive misses, metacharacters literal, matching/nonmatching pagination, membership/recent/archive scopes, rename entering/leaving match set after first page, rename no-op, topic changes, and unrelated directory writes preserving filtered continuation. Preserve all protocol field/shape compatibility and message-search behavior. Files-touched: src/store/queries.rs, src/store/control.rs, tests/store/queries.rs, tests/store/control.rs (name mutation suite), tests/cli/read_cost_names.rs (reuse isolated harness), protocol comments if needed. owns: canonical directory search predicate and name revision/cursor binding; consumes: existing DirectoryQuery.topic_contains and SetThreadName canonical transaction. Acceptance: focused regressions pass with unchanged limits and exact CLI --search name discovery; cargo fmt and per-change clippy pass. Read TRUST-POLICY before control.rs changes; update policy only if changing invariant, which is outside intended scope.\nInventory every canonical and managed/service thread-name SQL write site. Every actual existing-thread name change must publish name/all in its deciding transaction, or demonstrate the path cannot rename an existing thread. New-thread initial names remain high-water controlled.\nMandatory verification: run nice scripts/check-default-features before integration. Required regression detail: place a name-only hit beyond at least one work-limited zero-match page; follow cursors without duplicates/skips. Obtain mutation-driven stale results through production store dispatch, including set, rename and clear. Verify exact-operation replay stability as well as unchanged-name no-op stability.\nAlso verify selected-instance isolation, clearing a matching name, and unfiltered continuation behavior remains unchanged.",
    "status": "open",
    "priority": 2,
    "issue_type": "task",
    "owner": "235553+alepar@users.noreply.github.com",
    "created_at": "2026-10-10T19:37:39Z",
    "created_by": "Alexey Parfenov",
    "updated_at": "2026-10-10T19:43:29Z",
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
    "dependent_count": 1,
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
```
