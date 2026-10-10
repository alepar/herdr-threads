# SDD ledger — plan: ht-akx-plan.md

Spec: docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md (read; reachable; binding).

## Preflight consistency table

| Tasks | Producer / consumer or self-consistency check | Finding and ruling |
|---|---|---|
| 1 | Bounded predicate, transactional name/all publication and filtered opaque key; production mutation/pagination tests verify those behaviors. | Consistent. Preserve unfiltered keys and topic/all; inventory managed writes. No trust change or new target. |
| 2 | Public help, adopted-contract amendment and legacy field comment; parser/serialization checks verify unchanged compatibility. | Consistent. No dependency on implementation for these checks. Reuse existing assertions; add missing coverage only. |
| 3 | Integrated behavior and contract consumed after both leaves; uncovered tests and design notes produced. | Consistent. Conditional source declarations allow small integration fixes only; larger gaps return to coordinator. |
| 1 ↔ 2 | Task 1 implements DirectoryQuery.topic_contains semantics; Task 2 documents them and verifies serde/argv compatibility. | Consistent shared interface. Optional protocol comments in Task 1 bead are assigned exclusively to Task 2. Leaves have no write-file overlap. |
| 1 ↔ 3 | Task 1 supplies query/control behavior and store/CLI tests; Task 3 consumes and may repair those same files. | Consistent; serialized by blocking dependency. Reuse leaf evidence and inspect exact integrated tree. |
| 2 ↔ 3 | Task 2 supplies public help/wire contract; Task 3 checks against production behavior and may repair CLI/protocol source. | Consistent; serialized by blocking dependency. Legacy topic_contains spelling is retained throughout. |

Preflight ruling: no unresolved conflicts. Spec is binding. Task 3 conditional repair declarations do not mandate edits. Only the parent coordinator updates this ledger after initialization.

Task 1 (ht-akx.1): planned; ready.
Task 2 (ht-akx.2): planned; ready.
Task 3 (ht-akx.3): planned; blocked on ht-akx.1 and ht-akx.2.

Launch: native collaboration fallback; config.concurrency 2; hotFileCap 3; base 871521af; caller owns finish; config.sweep deferred to super-auto post-roast phase.
Provenance: both task worktrees resolve their own Cargo.toml and source/test paths; no workspace path dependencies; shared CARGO_TARGET_DIR is build artifacts only.
Detector: round 1 — parallelism: 2 ready · cap 2 · peak in-flight 2 · hot-file deferrals none
Ruling: Task 1 uses filter_revisions(instance, 'directory', 'name/all') rather than ('name', 'all') — schema CHECK allows directory/inbox/topic only, contradicting design's generic-kind assumption; dedicated key preserves search-only instance-wide rename dependency without migration; cost if wrong is representation adjustment, no public shape or semantics change. Integration task must amend spec Post-Implementation Notes.
Verification invalidated: first docs gate reused shared-target behavior-only binary despite docs manifest path; 41 tests absent from source prove stale artifact. Re-running after touching task-local src/lib.rs timestamp forces package compilation; gate accepts only source-matching expected test inventory.
Ruling: root requires private CARGO_TARGET_DIR per task/integration, optional APFS clone with package-clean restricted to private clone. Forced timestamp rebuild aborted (exit130); source content unchanged. Prior shared-cache validation potentially contaminated; both tasks revalidate focused tests/clippy/default against private artifacts. config.gate target substituted to private path by root authorization; test selection unchanged.
Merge: ht-akx.2 — rebase clean · seam-review none · gate pass
Task 2 (ht-akx.2): complete (commits 7a0d9253..7d458036, private docs validations and integration gate38/38; integration package compiled2m18s, tests1.39s)
Task 2 (ht-akx.2): minor (deferred): sandbox nice priority adjustment denied; cargo checks succeeded, environment noise only.
Verification risk identified: behavior rebase might have overlapped revalidation; later inactive-worker audit disproved overlap. No affected result exists. Stable rebased SHA combined42 verification starts through followup_task.
Correction: behavior private revalidation was not active during rebase; send_message did not restart completed implementer. followup_task now running stable combined-source private verification. No compiler/rebase overlap occurred.
Merge: ht-akx.1 — rebase clean · seam-review none · gate pass
Task 1 (ht-akx.1): complete (commits e7592f5a..d61b4720, private source-proven tests/checks; integration gate42/42, compile38.96s execution1.26s)
Task 1 (ht-akx.1): minor (deferred): sandbox nice priority adjustment denied; cargo checks succeeded, environment noise only.
Candidate: main effa14a2 merged as65a67dcf before integration sweep; INDEX append conflict kept both independent rows. Clean auto-merges inspected in control.rs (reject vs name setter), commands.rs (Reject vs list search help), tests/store/control.rs (both test families), INDEX.
Detector: round 2 — parallelism: 1 ready · cap 2 · peak in-flight 1 · hot-file deferrals none · dependency-limited integration leaf
Merge: ht-akx.3 — rebase clean · seam-review none · gate pass
Task 3 (ht-akx.3): complete (commits 65a67dcf..35bf2f84, review clean; integration-private v0.6.1 gate42/42 compile2m01s execution1.50s; final leak check no leaked test processes)
Sweep: declared per-branch sweep deferred to super-auto post-code-roast; .3 focused combined integration verification is recorded separately and does not claim full-suite coverage.
Metrics: merges 3 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 0 (fixed 0) · gate-fails 0
Metrics: fix-loop round 1: 0 addressed / 0 entered · round 2: 0 addressed / 0 entered · round 3: 0 addressed / 0 entered · round 4: 0 addressed / 0 entered · round 5: 0 addressed / 0 entered
Metrics: fix-loop breaker-tripped: 0
Metrics: ledger-check ok · append-failed 0 · append-retried 0
Final review: CLEAN — fresh whole-epic review effa14a2..fe464af5; no product findings, deferred nice warnings classified environment-only. Parent post-roast sweep remains pending.
Finish: stopReason root-closed; completed ht-akx.1,ht-akx.2,ht-akx.3; escalated/pendingRetry/parked none; stalled false; review CLEAN; caller owns finish, integration worktree retained.
