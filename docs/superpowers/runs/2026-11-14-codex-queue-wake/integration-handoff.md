# Queue-wake integration handoff

Current frozen code: feat/codex-queue-control at7090d18af6996f7fdaed96717fef1f16c77883ba.
Main4dabf06ef90c8c9e8c459b524b509af3a2e98b31 is its ancestor; actual merge-tree
and source tree equal2de5287d691c34d7170969d006bc28457ae36f00. Astra273 accepts
the final narrow correction with no remaining findings. The authorized corrected
unfiltered suite passed9779/9779 with64profile skips and first scoped cleanup
clean. Implementation is ready for main integration disposition.
Main owns integration/minor release; implementor does not push, tag or publish.
`integration-commits.txt` lists the complete qualified branch lineage. Key final
corrections: bb0cc218c eligibility/resource fences;2bc6f9713 automatic transport
qualification;056d63893 bounded fallback/default-off;94b43c2b0 strict JSON;
6ca86d79a completed-result retry/legacy activity;7090d18af hint separation.
This evidence-only successor adds no runtime changes.

## Behavior and activation

Queue-backed wake is OFF by default by user decision relayed in main48. An absent
or false programmatic_wake keeps native typed wake. True opts in new queue adds
through existing node/settings.json startup semantics. Existing effect fences and
cleanup/recovery continue when off, including cleanup helpers. The22-byte content-
free wake marker is queued user input. Message bodies, invitations and receipt/ACK
attribution stay in ordinary hooks. Accepted is submission, never delivery.

Eligible foreground Linux queueing requires canonical connected top-level scope,
matched native Start/End baseline, cooperative loadedness/process/writer-lock proof,
captured receiver SQLite/executable identity, same-database helper and fresh final
idle/composer/focus guards. Busy is never deliberately queued. Unsupported, dormant
or ambiguous admission retains native behavior where genuine effect fences allow.
Version facts are evidence, not admission gates; no receiver load/resume is issued.

Accepted/Unknown may permit guarded native fallback after immutable arm30s plus
positive exact current-helper quiescence, under the existing spacing/backoff. Armed,
in-flight, uncertain cleanup and absent time evidence stay fenced. Every applicable
blocker is claimed atomically with exact owner/reservation/ticket and componentwise
publication high-water. Only typed host NoEffect restores prior coverage; submitted,
unknown, error, cancel, crash/missing/failed completion stays spent. Genuinely newer
attention can earn one effect; elapsed time/new reservation alone cannot. Physical
phase/IDs/logical state/programmatic fence remain; an old queued marker may still
run later and cause an extra turn. Reserved lazy p0 without ACK reaches the existing
lane through the same pure predicate; no wake_work/frontier/contact reset is used.

## Core changes

| Paths | Result/invariant |
| --- | --- |
| harness/wake.rs,adapter.rs,registry.rs | Optional neutral control/policy seams, exact scopes, typed outcomes and suppression-only replay cohorts. |
| harness/codex/wake/** | Receiver FD/inode/executable capture, private bounded helper add/list/delete and cooperative loadedness; nonUTF8 parents refused before spawn, vanished unrelated FDs handled narrowly. |
| ports.rs,ports/{wake_intents,native_turn}.rs,protocol | Canonical ledger/activity seams and backward-compatible optional native-turn metadata; hint eligibility is internal, not a wire field. |
| cli/hooks,harness decoders,service dispatch/store native_turn | Qualified current native IDs, matched baseline, new EndFirst purpose fulfillment without baseline/receipt proof; legacy activity preserved without canonical hint authority. |
| store/wake_intents*,migration/schema | Immutable scope/arm nonce/timestamp/pinned Add witness; independent purpose/effect/helper state, monotonic cleanup, bounded retention and exact fallback coverage. |
| notification queue/recovery/dispatch,scheduler | New-add opt-in, retained fences, actual transport/typed completion, existing-lane fallback and joined cleanup. Exact completed arm-ticket results retry metadata only after control returns;4096 capacity reserved before possible Add, no eviction/nonce replacement; total32-item fair tick. |
| store selection,daemon settings/service config/app | Empty ledger avoids binding census, off skips admission/canonical eligibility, real origin/cohort fences remain. Defaultfalse gates new adds only. |
| host native/named | Existing final idle/composer/foreground/focus guards reused; no input overwrite or weakened focused-empty rule. |
| adapter reap pool/node ownership | Fixed32 exact Child slots reserved before spawn, retained solely for bounded process-owned reaping; no saved-PID signals/queue I/O/callback authority. Ordinary exact process/group evidence alone proves quiescence. |
| store/schema.rs | Reviewed latest-only audit snapshot cache, historical upgrade capture unchanged; data/tamper regressions. |
| wake/release drivers | Bounded exact proof/ID-body association; setting-on OR explicit flag selects private proof, actual queue15s/native6s and strict Unknown failure; UI/resume gates unchanged. |

Paths in the table are under src/ unless clearly driver/test paths. Full branch
contains control/storage/activation together; no inactive partial slice is merged.

## Trust, migration and known limits

TRUST-POLICY records cooperative observed-idle/loadedness races, best-effort
cancellation and possible extra turns, bounded native ID observation-order limits,
helper/effect uncertainty independent of purpose, and exact owner recovery without
saved-PID signalling. Pinned Add evidence is historical, never current cleanup
proof. Queue acceptance/deletion/native activity is never a receipt. Legacy memory
refresh cannot publish a host hint without separately accepted canonical evidence;
unregistered/wronghost/held/subagent controls and harness-process attribution pass.

Soft-deadline pokes publish no new frontier, so they get no extra fallback allowance
while retained possible effect covers attention. New attention and sender overdue
warnings remain. P1 ht-d2jq.8 is an explicit separate follow-up: opted-in pre-arm
RefusedBeforeEffect plus transient refusal-metadata failure may leave a same-boot
Prepared fence until qualified scope/binding finality or owner restart. The current
armed-ticket broker does not fix it. Main accepts separate patch disposition because
queue ships off; this is not a claimed direct human waiver. CHANGELOG lists the
exact limit beside opt-in instructions. Hook delivery on a natural user turn remains.

Migration0018 remains next free on main4dab at run start: one-way node schema18,
including intent/owned-ID/current-native-turn state and bounded fallback metadata.
Released deltas are unchanged. Historical pre-fix unreleased18 fixture is preserved
and refused without repair/data loss; fresh and17->final18 audit/upgrade tests pass.
No real user/released store was upgraded in this feature qualification. Recheck
numbering if main advances. CHANGELOG discloses the one-way upgrade; minor release
versioning/pre-release wake gates remain main-owned.

## Qualification and review

Current code focused:59PASS4.091s/196compile, first e9d4713b scoped process/temp
CLEAN; Clippy23.52/default/fmt/neutral/wire/docs/diffPASS. Earlier exact-result/
legacy/selector slice:434PASS40.136s, first19617bf1 clean. Strict release parser:
16PASS0.218s, rejects duplicate keys and NaN/Infinity constants. Offline wake driver:
298PASS102.904s on its unchanged source. Immutable qualification-hint-authority.*,
qualification-main-review-edge-cases.* and qualification-json-constants.* pin
current source/log hashes and exact scopes; no old evidence is relabeled.

Historical formal round1 had3Shouldfix+1Nit, fixed; round2 found abandoned child
reap ownership, fixed. Formal round3 at a89dd07c0 vs4dab was clean16/16scouts,
0findings/0nits. Main's later independent integration review found additional issues;
bounded rereviews accepted corrected B1/S1-S5/N1-N4, default/routing, main54 edge
cases and final hint authority at7090. That current targeted acceptance is separate
from the historical formal verdict. No fourth broad review was requested.

Failed frozen6ca full:9778PASS/1FAIL/64skip925.402s execution,926.305s wall,
firsteedcb7d6 cleanupCLEAN. The sole real hint-authority coupling is fixed; original
fixed-tree-* and qualification-final-hint-failure.* remain immutable. Earlier15fd
static inventory failure is also retained, not qualification.

CURRENT CORRECTED FULL: exact7090/tree2de5287d, unfiltered alltargets/allfeatures,
profileci -j4, heavy/tb nice-n19/jobs/testthreads4/warmtarget/unchangedgroups.
HT_LEAK_RUN_ID93b01690-2f88-4c66-839a-1671542e08c0. Result9779PASS/0FAIL/64profile skips, execution1015.238s/compile0.03s/wall1017.390s,
exit0. First scoped process/temp cleanup exit0/CLEAN. Nextest2580a3d9-ad94-4104-978a-f2ca673ca69d.
Source stayed fixed; main4dab and next-free0018 rechecked at completion.
qualification-final-corrected.* pins raw hashes and changed non-document files;
final-suite-raw/ retains compressed original log/JUnit/time/firstcleanup/runner.

## Live and helper side-effect scope

Historical72unique authenticated Codex rows:30actual queue positives,6native
fallback compositional inferences,36silent/in-turn/no-wake. They retain original
SHAs, exact proof/accepted15s scopes and preserved failed6s/UI runs. No current-tree
live repeat, nonLinux queue proof or real-model named-tmux queue proof is claimed.
Owned live workspaces/daemons/auth copies were removed. Release owner runs the
required actual release wake matrix separately under the selected default/proof mode.

S4 populated isolated SQLite-copy evidence ran3helper cycles/15RPCs against the
captured executable, observed queue-only aggregate changes across6DB snapshots,
with real home masked/network disabled/no creds/model/load/resume. The helper also
opened other sibling DBs; this is not a universal future-version/migration guarantee.
Private copies/processes were removed. qualification-sqlite-effects.* retains exact
source/limits; no real-profile helper experiment is claimed.

Performance remains the separate nonblocking lane by the user's relayed main25/26
disposition. No five-minute/incremental budget pass or universal latency guarantee
is inferred. Warm worktree/build remains needed until main confirms integration;
owned cleanup follows landing. No push/shared-server restart/user-config write.
