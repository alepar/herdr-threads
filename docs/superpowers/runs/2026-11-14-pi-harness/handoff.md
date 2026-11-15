# Pi integration handoff

Pi terminal sessions now support owned setup, launch/handoff, session enrollment,
in-turn message delivery and guarded native wake. Receipt confirmation requires
the exact current-branch and observed context evidence. The opt-in neutral manual
presentation seam keeps notices recoverable until confirmation; existing adapters
retain their default behavior. Shared changes and compatibility boundaries are
listed in the [33-path core inventory](qualification/core-changes.md).

The [second whole-branch review](roast-pr-2/report.md) is clean and converged, with
all four original findings resolved. Its frozen source predates v3.2.1. The
[integration audit](qualification/integration-rebase.md) verifies all 73 rebased
patches are equivalent and accounts for upstream changes. The tested integration
source is `bad263b15dbbca518ea0509d16f90f9d01c1c602` on
`e60ec5eb5e9746f82b399127838dc9515c7cc712`; later commits publish evidence only.

[Integration checks](qualification/integration-gates.md) passed, including 9,645
tests, with 66 skipped and clean scoped cleanup. The [qualification index](qualification/README.md)
links the 12-case native compatibility run, separate notice-recovery and automatic
successor-wake proof, canonical/SDK tests, and the single authorized live DeepSeek
run. Native/live evidence carries through the source-equivalent rebase; it is not
relabeled as a new binary run.

The suite exceeded five minutes under the four-worker training limit. Main accepted
that as nonblocking; `ht-n4xh` remains open. Clippy passed in 62.45 seconds, slightly
over its one-minute budget. Accepted screen-observability and downstream-context
limits remain in the [Pi guide](../../../../integrations/pi/README.md#delivery-and-wake).
The separate Claude/opencode notice-loss issue `ht-0w99` is unchanged.

Main owns independent integration review, merge and release. After merge, preserve
the private qualification/review evidence before removing this worktree, its build
output and probe dependencies, and the pre-reorder backup branch. No test daemons,
private hosts or live credential files remain from completed qualification runs.
