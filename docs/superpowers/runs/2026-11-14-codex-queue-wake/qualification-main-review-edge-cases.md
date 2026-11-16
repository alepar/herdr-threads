# Main targeted-review edge cases

Basis94b43c2b0f83227a38bd2b88fbd637dedc219850, main4dabf06ef unchanged.
Authorization: main54 and Astra253/255/257; follow-up disposition Astra259/260.
This supplements the immutable056d/94b evidence. No full/live run is claimed.

## Changes

Completed Add results survive a transient metadata failure in a node-owned retry
collection. Capacity for4096 unique attempts is reserved before prepare/helper/Add;
there is no effectful-result eviction or nonce replacement. Only after control.add
returns is the typed result published in its exact existing owner/arm ticket slot.
The normal record_result CAS remains the sole database authority. A successful CAS
settles the slot; false CAS requires a canonical terminal/inapplicable read. Failed
writes/reads and an exact still-Armed ticket remain pending. No helper, Add,
cancellation or speculative same-boot Armed conversion is performed. Positive
helper cleanup while the callback still runs cannot publish a result.

The existing five-second recovery tick examines at most16 metadata slots plus the
remaining ledger rows, preserving a TOTAL32-item quantum and fair cursors. Pending
slots keep activity. Node ownership/shutdown remains bounded; restart uses prior
owner recovery. No new lane, runtime configuration or schema field is added.

Same-instance top-level non-subagent legacy observations now refresh the local
activity book when canonical queue qualification is unavailable. They create no
queue state, fulfill no purpose and grant no queue authority. Canonical current-ID
retry/mismatched-End suppression remains. Service peer/instance checks are unchanged.
NativeTurnUpdate.changed describes a fresh memory observation, not necessarily a
durable queue-state write. Existing tests now distinguish these claims explicitly.

Selection takes a neutral eligibility-needed flag. Actual retained origin/cohort
fences are always read; empty unresolved-ledger EXISTS avoids cohort enumeration.
When new adds are disabled, binding/canonical/reservation admission is skipped.
Real fence-read errors remain closed. Native-only cost stays flat with valid open
binding history; opted-in binding selection and retained cross-origin fences remain.

TRUST heading uses2026-11-15. The soft-poke nit is an explicit conservative limit:
no new publication frontier means no additional fallback allowance while retained
possible effects cover the attention. New attention and sender warnings remain.

## Verification

Three causal tests failed before the fixes (1.487s): lost result write stayed Armed
after the real recovery tick, canonical-absent legacy activity was dropped, and
default-off native wake failed on corrupt queue-admission state. They passed after
implementation (1.733s). Initial invalid Harness constant/extracted-variable compile
logs are setup/compile failures, not behavioral RED. Expanded34 ran32PASS/2FAIL:
the old tests still asserted edge=None for canonical absence; only legacy-edge
expectations changed, with queue-state/intent/identity invariants retained.

Final affected qualification:434PASS40.136s,134s compile, all targets/features,
profileci -j4 under existing heavy/tb cap4/nice19. Fresh19617bf1 scoped processes
and temp files CLEAN; first check output and JUnit retained. Controls include real
runtime/control/recovery: repeated failed writes, quiescent helper before callback
return, lost arm reply with NoEffect/no Add, lost successful reply, stale/deduplicated
nonce, capacity refusal before helper and no effectful-result eviction. The empty
ledger cost test compares128 then1152 valid open cohorts. Existing Armed/uncertain
cleanup, frontier/guard, old schema and default-off controls also pass.

Clippy all-targets/all-features PASS50.48s; default-feature, format, neutral, wire,
docs and whitespace checks PASS. Python code is unchanged from the16PASS strict
JSON correction; no redundant Python/live rerun. No recurring delivery/wake text
was added. New infrastructure refusal details are at most34 bytes, on the existing
error path. No speed-budget pass is inferred from the compile time.

## Explicit follow-up and integration boundary

P1 ht-d2jq.8 is separate and intentionally unfixed: when opted-in Add returns
RefusedBeforeEffect BEFORE an arm ticket exists and its subsequent refuse_prepared
metadata write fails transiently, the same-boot Prepared row can keep that seat/
cohort fenced after contention ends until qualified scope/binding finality or owner
restart. The exact armed-ticket broker does not cover it or fabricate a nonce.
Bodies remain available through ordinary user-turn hooks. Main accepted this
opt-in-only creation path as a documented follow-up because queue ships off by
default; this is integration disposition, not a direct human waiver. CHANGELOG lists
the exact limit beside opt-in instructions and discloses the one-way schema18 upgrade.
The P1 bug records trigger/path, recovery boundary and a proposed causal regression.

This delta remains held for Astra's narrow source/evidence acceptance. Main54
requires the single final frozen-tree unfiltered suite plus fresh scoped process/
temp checks after that acceptance; no further broad review/live run was requested.
Historical15fd FAIL,72live rows and earlier qualification source identities remain
unchanged. This report and JSON pin only the current correction and its actual tests.
