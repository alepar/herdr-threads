### Spec Compliance

- ✅ Spec compliant for the scoped post-rebase Task 1/landed Task 2 seam at `e70091bf8f6dbe03a4d811d1feceda4f4bac8de5`, base `d2f62e647aaa18ede5e894bc326cc196a4399cd5`. **Spec: CLEAN.** All five overlap files were inspected: `src/cli/mod.rs:2761`, `src/protocol/capabilities.rs:31`, `src/store/schema.rs:581`, `tests/protocol/capabilities.rs:1665`, and `tests/store/schema.rs:6266`.
- ✅ `src/store/schema.rs:172` commits fresh migrations through invitation29 before calling receipt30 at `:601`; historical28 commits29 at `:582`, historical29 goes directly to30, and current30 audits both without rerunning the cutover. `:649` requires autocommit, disables FK enforcement before BEGIN IMMEDIATE, validates the FK graph, commits version30 with the rebuild/marker, and restores the original FK setting after success or rollback. No nested transaction or historical SQL rewrite was introduced.
- ✅ `migrations/0030_receipt_send_time.sql:4` preserves receipt rows/ordinals/sparse rowids and sequence high-water while removing only receipt availability/deadline coupling; `:161` creates and captures the cutover in that same transaction. `src/store/schema.rs:693` audits every installed non-temporary CREATE object, including the exact receipt table constraints and marker. `src/store/effective.rs:1806` bounds legacy reconstruction by the captured sequence and `:1814` preserves stored deadlines and terminal absent deadlines.
- ✅ Invitation confirmation definition survives at `src/protocol/capabilities.rs:31` and remains absent from ADVERTISED (`:59`); SEND_TIME_RECEIPTS_V1 remains advertised (`:63`). Both new-send and retry invitation guards survive beside receipt guards at `src/cli/mod.rs:2757` and `:2857`; the unchanged helper at `:3020` refuses explicit invitation policy without its capability. The invitation legacy-byte/digest/modern-policy regression survives at `tests/protocol/capabilities.rs:1665`, immediately before the receipt public-status regression at `:1706`.
- ⚠️ Runtime validation for this rebased source is pending. The original approved 317 focused passes and warning-free 70s clippy apply to the old candidate, not this tip. The seam author's Python actual-SQL proof is reported passing, but does not establish Rust compilation or runtime behavior. Require the controller's actual GREEN connection/effective/capabilities seam selections, then the mandatory exact-candidate lint and premerge default-feature gate before merge.

### Strengths

- `tests/store/effective.rs:982` runs genuine migration0001..0028 and migration0001..0029 precursors with populated receipt/FK graphs; it compares retained rows, canonical receipt indexes/triggers, ordinals, sparse rowids and sequence high-water. Its modern invitation checks retain original origin/policy, reject mutation, and force the invitation due index to exclude lazy rows after receipt upgrade/restart.
- `tests/store/effective.rs:1108` covers both old versions and original FK OFF/ON: receipt failure leaves autocommit, committed invitation29, old paired-null CHECKs and no marker, then repairs the child and retries/reopens30 with the original FK setting intact. `:921` verifies one-time cutover capture and running-clock restart stability; `:1188` retains real acknowledged NULL/NULL provenance through later registration and restart.
- `src/store/schema.rs:607` still audits invitation confirmation/origin columns plus exact immutable/origin triggers and due index before the new receipt audit. `tests/store/schema.rs:6266` honestly retains expected catalog totals75/140/183; `:5699` excludes only the additive cutover marker from the existing old-root comparison.

### Issues

#### Critical (Must Fix)

- None found.

#### Important (Should Fix)

- None found in the scoped seam.

#### Minor (Nice to Have)

- No new seam findings. Existing timing-budget concern remains: `task-1-report.md` reports source rebuilds6m08s/4m15s, CLI compile4m26s and lint70s, exceeding one-minute budgets. This review does not establish attribution or resolve those overruns.

### Checks

- Read the brief, original implementation report/review, seam-fix report and supplied119KB review package. Read the package in portions; an initial combined output truncated its beginning, recovered with the1..270 portion. No broad second epic review was performed.
- Mandatory test-diff commands ran in the exact candidate worktree: `git diff --stat d2f62e647aaa18ede5e894bc326cc196a4399cd5..e70091bf8f6dbe03a4d811d1feceda4f4bac8de5 -- tests` and the same test diff capped with `head -n 400`. Stat:7files,645insertions,26deletions. The400-line output is intentionally truncated; the full test hunks were inspected in the supplied package. No test deletion, skip or unauthorized assertion weakening found. Changed deadline assertions implement the requested semantics; bounded drain loops add termination/error assertions. Schema fixture changes restore canonical FK prerequisites and retain old history/root/catalog checks.
- Named risk — invitation policy accidentally lost through clean automerge: focused reads of the unchanged invitation capability definition/list (`src/protocol/capabilities.rs:31`), CLI helper (`src/cli/mod.rs:3020`), and legacy/modern wire regression (`tests/protocol/capabilities.rs:1665`) confirm retention and non-advertisement. This is the bounded inspection of each of the three separate overlap contracts, not a repository crawl.
- Named risk — fresh/historical transaction nesting or lost invitation audit: read the adjacent initialization context at `src/store/schema.rs:122` and `:545`; both fresh and historical29 transactions commit before the30 wrapper. Read the previously unchanged invitation audit at `:607` against migration0029's exact columns/index/triggers. A read-only Python byte comparison confirms candidate migration0029 equals the landed integration file; the supplied base-to-head package contains no modification to it.
- Named risk — capability served after a failed schema upgrade: checked unchanged writer initialization at `src/store/connection.rs:167`; schema initialization must succeed at `:175` before returning a usable writer. No new bypass is introduced by this seam.
- No Cargo, Rust tests, Herdr thread operations, process starts, branch/index mutations, commits or rebases were performed. Only this requested report was written. Runtime validation remains controller-owned and unclaimed.

### Assessment

**Task quality: CLEAN — conditionally approved code.**

**Reasoning:** The rebase composes invitation29 and receipt30 in the correct transaction order, preserves both capability contracts, and adds behavioral preservation/rollback coverage rather than merely renaming SQL. No code fixes are required by this scoped review; merge remains gated on actual GREEN focused seam validation and exact-tip mandatory checks.
