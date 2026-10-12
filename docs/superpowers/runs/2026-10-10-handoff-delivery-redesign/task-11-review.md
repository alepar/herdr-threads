Spec CLEAN
Quality CLEAN

### Spec compliance

- Approved against BASE dd1a53d43c6adaffdcbbe3b14080969660e2788f and HEAD 50e6a20b8212cf6d917d20934f52f476ba87e346. Read the complete package, test stat, capped first 400 lines and the empty remainder: two mapped test files, 203 additions and 6 deletions; no production change, skips, weakened timeouts or new test target.
- The new guarded canonical handoff exercises both acceptance→ACK and ACK→accept. Message expiry at 7001 precedes invitation expiry at 11001; absence of recipient registration is asserted, and each actual canonical mutation clears only its own condition (tests/cli/handoff_delivery.rs:3414, 3483, 3540). Original sender attention persists until final settlement, followed by passive clear and unchanged frozen receipt deadline/duration (3550–3585).
- Correcting the obsolete sender-passivity assertions matches TRUST-POLICY.md:451–462. The new sender assertions preserve unrelated observer passivity, actual actionable-warning discovery, informational offer independence, recipient offer deduplication and passive final clear (tests/integration/notice_wake.rs:308, 316, 345, 352, 417). These changes do not weaken the settled contract.
- The brief explicitly permits reusing existing producer proofs and limits new coverage to missing interfaces (task-11-brief.md:344). Its filesTouched list is permitted ownership rather than a mandate to edit every listed file. The two changed paths satisfy that scope.

### Strengths and focused risk checks

- Risk: a fake mutation or manually injected receipt overlay could make settlement pass without canonical authority. Focused inspection of existing CanonicalInvites::canonical (tests/cli/handoff_delivery.rs:569) confirms real preparation, cooperative permit issuance and store mutation. GuardedCanonical delegates guarded delivery to the canonical store; its counts read the same database (2053, 2072). The new test changes only the deciding store clock to 11002 for late mutations and reads committed state through its independent connection.
- Risk: sparse publication might be bypassed to manufacture discoverability. Existing handoff_policy_advance_work (2811) drains actual pending durable jobs using bounded materialization::advance_work; it inserts no synthetic receipt state. The added test invokes this helper before actual receipt and invitation scanners.
- Risk: changed notice expectations might mask loss of native or observer guards. Existing notice fixture candidate() uses StorePort::wake_candidates (tests/integration/notice_wake.rs:221); both new sender checks require actionable warnings without unrelated pending receipts. Existing bystander and affected-seat guard assertions remain. The diff does not itself claim native key submission; separately selected existing native/coalesced tests supply that seam evidence.

### Validation evidence inspected

- /private/tmp/ht-task11-focused-lib.log: 34 selected, 34 passed, including the new two-order settlement test, all three maximum-body paths, readiness/capability controls, actual NULL-native ordinary author, bounded H/D/I discovery and genuine upgrade tests; build 45.51s, runtime 34.345s.
- /private/tmp/ht-task11-focused-integration.log: 17 selected, 17 passed, covering stable setup, installed canary consumers and actual elected daemon delivery/new-tab/replay; build 21.67s, runtime 10.301s. The notice filter selected no cases in this target and is not counted as notice coverage.
- /private/tmp/ht-task11-notice-wake-final.log: the actual combined target ran 2 selected, 2 passed; build 2.18s, runtime 0.296s. Total final distinct coverage is 53, with no ignored tests. Filter-excluded unrelated tests are correctly distinguished from ignored tests.
- /private/tmp/ht-task11-clippy.log: exact-source all-target/all-feature lint completed without warnings in 19.17s. Final fmt log is silent, consistent with the reported successful fmt check; its exit status and the six UUID-scoped cleanup results are recorded in task-11-report.md, not independently reproduced by this review. No tests, git operations or process inspection were run by this reviewer.

### Limits and assessment

- Cross-task historical compatibility and native guard implementations are unchanged by this diff; the named producer tests in the inspected sweep provide regression evidence, not a fresh whole-branch correctness proof. No accepted-limit mismatch was found in the examined invitation/receipt policy seams.
- Final default-feature validation and expensive exact-tip readiness remain explicitly deferred to the parent after PR roast. This task review does not assert whole-branch merge readiness or full-suite budget compliance.
- Critical: none. Important: none. Minor: none. Task quality: Approved.
