# Pi integration handoff

**Ready for main merge under main’s accepted disposition `mnSGlopMCqRw`.** The local full suite remains failed; CI’s sharded full gate is outstanding. This handoff does not perform a merge or release.

Pi terminal sessions now support owned setup, launch/handoff, session enrollment,
in-turn message delivery and guarded native wake. Receipt confirmation requires
the exact current-branch and observed context evidence. The opt-in neutral manual
presentation seam keeps notices recoverable until confirmation; existing adapters
retain their default behavior. Shared changes and compatibility boundaries are
listed in the [34-path core inventory](qualification/core-changes.md).

Current production source is `9f69b8d02b1244a7e21be8df9d368b380f887764` on required main `4f2a32d3a1fb9bd5991f6d1cb13fb345b99f1da5` (v3.3.0, schema 18). Pi adds no migration. [The current rebase audit](qualification/v330-rebase.md) accounts for integration and the reviewed optional native-turn metadata correction. [Main-review disposition](qualification/main-review-disposition.md) records all eight addressed findings and their source/causal evidence.

[Shared checks/build](qualification/v330-final-gates.md) passed with clean scans. [Ten selected fresh native cases](qualification/v330-native.md) passed on the exact development candidate `a63ac7a7…`, with clean owned and independent same-ID cleanup scans. The failed HOL attempt remains recorded alongside the reviewed two-sink fixture correction. **The local full-suite attempt failed (9,853 passed, two failed, 67 skipped).** [The reviewed two-file correction and focused follow-up](qualification/v330-full-suite.md) at `55401e27c56a9e3f990206c550ec6f42ea61e510` passed all 19 selected checks, every required guard and both same-run scans; production source remains unchanged. Test execution took 927.245 seconds, compilation 7m18s and total wall time 1,366.38 seconds; both scans were clean. Main accepted no second local full run for these test-only corrections (`mnSGlopMCqRw`). The full sharded merge-CI gate remains outstanding and was not executed locally; main owns merge. The suite's test-profile artifact is distinct from the native development binary. The [qualification index](qualification/README.md) is the canonical status record.

The [second whole-branch review](roast-pr-2/report.md), earlier gates and live evidence retain their historical anchors. They are not relabeled as reviews or runs of this tree. Main authorized no new paid run; [d55/a8 live evidence](qualification/final-live.md) retains provider and ordinary-path scope. The RPC-only composer-race characterization and subsequent driver controls are separate from native proof.

Historical suites exceeded five minutes under the four-worker training limit; main accepted that suite miss as nonblocking, with `ht-n4xh` open. Initial v3.3.0 clippy passed functionally in 100.21 seconds, missing its 60-second budget, and the development build took 66.63 seconds. Corrective final clippy also passed in 20.22 seconds. Main explicitly accepted clippy/build timing as nonblocking for merge/release too. These fresh version/rebase rebuild observations under cap four are not incremental-build measurements; `ht-n4xh` and raw budget misses remain recorded. Accepted screen-observability and downstream-context
limits remain in the [Pi guide](../../../../integrations/pi/README.md#delivery-and-wake).
The separate Claude/opencode notice-loss issue `ht-0w99` is unchanged.

Open composer-path follow-ups are `ht-gkvv` (upstream conditional prompt) and `ht-1g1c` (reread narrowing only); they include composer fallback and do not characterize programmatic queue transport.

This task prepares the integration branch and main handoff for merge; it performs no release. Main owns the later merge/release decision. After merge, preserve
the private qualification/review evidence before removing this worktree, its build
output and probe dependencies, and the pre-reorder backup branch. All requested local runs completed, and owned runtime cleanup was verified.
