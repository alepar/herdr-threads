Spec CLEAN
Quality CLEAN

Reviewed BASE a95623dc6a6f5a84d48b2d54e09d8701bc978b25 through HEAD d132e79a35915d4558f79b48fb190a4d1ff5804d against task-8-brief.md and task-8-report.md. Read the supplied package once in sequential passes, including its stat, the first 400 test-diff lines and all remaining test changes. No Cargo, source mutation, Git mutation, subagents or Herdr actions.

Strengths and verified requirements:
- src/cli/commands.rs:1082,2152 selects explicit Lazy for ordinary agent invitations, supports RequireAck/deadline implication and rejects conflicting options. Positive/signed millisecond validation precedes runtime effects; human policy flags refuse and human confirmation remains absent at src/cli/commands.rs:1723,1791. tests/cli/commands.rs:2457,2488,2503 exercise materialization, conflicts/overflow and human compatibility.
- src/cli/mod.rs:2073 refuses unsupported modern invitation policy before recipient allocation. src/cli/mod.rs:2933 preserves that selected policy into the existing semantic freeze. tests/cli/cooperative.rs:4563,4590 verify both explicit-seat and actual caller-scoped pane routes refuse before intent/allocation effects; existing allocation/caller-separation assertions at tests/cli/cooperative.rs:2015 remain intact.
- tests/cli/cooperative.rs:4460 uses the public parser/runtime with canonical store and verifies creation-based timing, original inviter and unchanged duplicate episode in both policy directions. tests/cli/cooperative.rs:4660,4749 preserve historical omitted fields, bytes/digest/key/caller and exact modern frozen retry requests through a real retained transport-failure boundary.
- src/cli/watch.rs:550 exempts only explicit Lazy invitations from attention while retaining timed/historical behavior; tests/cli/watch.rs:1715 covers the real drain output. src/protocol/output_compact.rs:173 preserves identity/goal/acceptance guidance and honestly distinguishes Lazy from acceptance-required policy without fabricating historical deadlines. tests/cli/output.rs:222,294 cover actual Text/JSON encoding and fully recoverable Lazy hook display without settlement.
- README.md:113 and integrations/skill/SKILL.md:111 document requested semantics; the guide preserves the established strict line budget by folding text rather than changing its test. Unchanged hook completeness, operator and required-membership implementations need no speculative edits.

Focused checks outside the diff, each tied to a concrete risk:
- Risk: the new early pane guard might leave direct-seat creation or replay unguarded. Checked existing run_cooperative fresh/retry call sites at src/cli/mod.rs:2775,2878 and helper at3039: modern semantic policy negotiates before recording/submission, while absent historical policy remains exempt. No guard duplication or journal format change.
- Risk: human namespace normalization might still emit agent policy. Checked parse_argv_in_registry actor extraction at src/cli/commands.rs:1643 and its surrounding validation: actor is determined before policy flag rejection and final human confirmation clearing.
- Risk: suppressing Lazy watch attention could drop ordinary messages or interrupt drain. Checked the enclosing drain match at src/cli/watch.rs:545: only the attention predicate changes; ordinary/lazy message handling and page continuation remain in the existing path.

Test-change assessment: additions assert real parser/runtime/canonical storage, journal contents, rendering and watch behavior. The existing fake capability addition and expected command-count adjustment retain its independent allocation/read/caller assertions. No weakened assertion, ignored/skipped test, relaxed budget or test-only bypass found. The report explicitly excludes wrong JSON selection, zero-selection and unknown-flag refusal from behavioral RED; corrected runtime RED precedes the production changes. Reported focused GREEN and lint/fmt/leak checks are task evidence, not a claim of independent rerun. Initial 158.72s/65.11s build budget overruns are disclosed; final 51.74s build and42.03s clippy are within budget.

Critical: none.
Important: none.
Minor: none.

Cannot verify from this consumer diff: inherited invitation lifecycle/origin wake correctness, combined driver fairness and final integrated/default-feature readiness. These remain the existing Task6/Task7/parent gates, not missing Task8 implementation.

Task quality: Approved.
