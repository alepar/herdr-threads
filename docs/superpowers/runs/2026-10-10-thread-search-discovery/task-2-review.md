### Spec Compliance

- ✅ Spec compliant. CLEAN. CLI help explicitly specifies both fields and case-sensitive literal substring matching (`src/cli/commands.rs:1058`); the retained protocol field changes only by a comment (`src/protocol/commands.rs:360`). README documents selected-instance/membership scopes and manual bounded continuation (`README.md:81`). Amendment section 4 expressly supersedes topic-only directory discovery while preserving generic topic/body search and bounds (`docs/design/herdr-threads/shared-contract-amendment-adopted.md:74`). All six brief-listed files have corresponding changes; the three approved maintained-document corrections remain narrowly scoped.
- ⚠️ Runtime name matching/name-revision behavior belongs to the separate behavior task and is not verified by this documentation diff. No runtime behavior or serialized shape changes appear here.

### Strengths

- Real continuation coverage calls the isolated SQLite store and parses returned `next_argv`, checks output context, cursor and full request equality after removing only the cursor (`tests/protocol/capabilities.rs:1529`). This checks literal text, scope, recent ordering and bounds through the actual producer rather than merely parsing hand-authored argv.
- Legacy nonempty `topic_contains` serialization and deserialization are asserted (`tests/protocol/capabilities.rs:1489`); the existing parser assertions retain their previous checks and add recent ordering and punctuation (`tests/cli/commands.rs:947`).

### Issues

- Critical: none.
- Important: none.
- Minor: the implementer report's Verification block records `nice: setpriority: Operation not permitted`. This is environmental command noise, not a compiler/test warning or evidence of failed validation; arrange an environment permitting the prescribed priority adjustment when practical. No rerun requested.

## Test changes

- Ran `git diff --stat 871521afbe22fd60ad09e67cf0a532e04a5647e2..9002066c72b2fcd58345e8ef4aa05148b504c1e0 -- tests`: two files, 86 insertions and 3 deletions.
- Ran `git diff 871521afbe22fd60ad09e67cf0a532e04a5647e2..9002066c72b2fcd58345e8ef4aa05148b504c1e0 -- tests | head -400`: full test diff fit below the 400-line cap; no truncation. No deleted, skipped or loosened assertions; replaced assertions preserve existing requirements and strengthen inputs/order coverage.
- Named external risk checked: whether the new continuation test reaches the production continuation generator. `src/protocol/capabilities.rs:98` includes this test module under `cfg(test)`; `StorePort::query` at `src/store/mod.rs:1422` routes through `query_with_output_for_caller` at line 1493; directory dispatch is `src/store/queries.rs:277`, and real continuations use `directory_argv` at lines 1788/1832, defined at 2656. The test's `page.has_more`, successful `next_argv` parse, output equality and request equality are meaningful and reachable with its two matching rows and limit one. No type-cannot-fail assertion is treated as evidence.
- No tests rerun; reviewed reported focused test, formatting, lint and default-feature results. Initial combined tool output truncated part of the review package; recovered that omitted documentation section without separately rereading changed files.

### Assessment

**Task quality:** Approved.

**Reasoning:** The change fulfills the documentation and compatibility scope with meaningful production-generated continuation verification, while preserving the wire field and runtime code. No blocking spec or quality findings.
