# Coverage ledger — ht-p03
# <ledger-id> · r<N> · <TYPE> · <subject> · <disposition> — <one line>
r1-01 · r1 · flag-sweep · ht-p03.12.3 · kept-leaf — size-only PROMOTE overruled (coordinator 2026-10-01): D6 split into .12.3/.12.7/.12.8 at that level, spec already decides; leaf stays
r1-02 · r1 · flag-sweep · ht-p03.19 · kept-leaf — PROMOTE overruled: bounded review-debt scope fixed by spec §B8 D2; runs after every B1–B4/B6/B7/B10 bead
r1-03 · r1 · flag-sweep · ht-p03.17 · kept-leaf — size-only PROMOTE overruled; already split at root into .17/.33/.34
r1-04 · r1 · flag-sweep · ht-p03.16 · kept-leaf — size-only PROMOTE overruled; already split at root into .16/.31/.32
r1-05 · r1 · flag-sweep · ht-p03.15 · kept-leaf — size-only PROMOTE overruled; already split at root into .15/.28/.29/.30
r1-06 · r1 · flag-sweep · ht-p03.11 · kept-leaf — size-only PROMOTE overruled; already split at root into .11/.27 (r1 adds edges/contract but scope stays leaf-sized)
r1-07 · r1 · flag-sweep · ht-p03.8 · kept-leaf — size-only PROMOTE overruled; already split at root into .8/.24/.25/.26
r1-08 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14<-ht-p03.13 · applied — epic acceptance restated with leaf attribution [ht-p03.14.6 cites .13]; no epic (needs:)
r1-09 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14<-ht-p03.15 · applied — same restatement [ht-p03.14.6 cites .15]
r1-10 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14<-ht-p03.28 · applied — same restatement [ht-p03.14.6 cites .28]
r1-11 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12<-ht-p03.6 · applied — epic acceptance restated [ht-p03.12.4/.12.5/.12.9 cite .6]
r1-12 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.9<-ht-p03.1 · applied — epic acceptance restated [ht-p03.9.5 cites .1]
r1-13 · r1 · GAP · R4 · applied — root spec §B1 D2 + Explicit deferrals record warnings retained (B1 nested D4) and work_jobs_live; R4 noted amended in run.md (id kept)
r1-14 · r1 · GAP · ht-p03.12 · applied — epic description and acceptance rewritten to the nested spec D1–D6 and deviations table
r1-15 · r1 · GAP · operator docs · applied — new leaf ht-p03.36 (docs/operations.md logs, degraded wording, retry suffix, remedy table)
r1-16 · r1 · GAP · design docs for new mechanisms · applied — same leaf ht-p03.36 (design-doc sections for Pacer, retention, taxonomy, optimistic ladder)
r1-17 · r1 · GAP · R46 · applied — new leaf ht-p03.37 findings closure ledger + B5 non-interference; ht-p03.20 confirms its rows
r1-18 · r1 · GAP · R41 · applied — ht-p03.20 acceptance: unpinned 10x on the tip SHA; ht-p03.1 fixture parallel-safety criterion
r1-19 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.20 · applied — ht-p03.20 adds Listed recipe rows for 2.1.287 / 0.159.3 when cells PASS; root Configurations row corrected
r1-20 · r1 · UNEXERCISED-CONFIGURATION · native matrix · applied — new leaf ht-p03.38 "Configuration smoke: native matrix…" (deps .1, .18); .20 consumes its cell entry points; .21 text repointed
r1-21 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.35<-ht-p03.14.6 · applied — criterion restated to t0.npm-shim-admission (needs: ht-p03.14.6) + edge
r1-22 · r1 · UNOWNED-SEAM · doctor --json Claude admission fields · applied — seam ht-p03.47 contract / ht-p03.48 integration; .23 owns line + shape test; .14.6 consumes
r1-23 · r1 · UNOWNED-SEAM · full_bodies wire compatibility · applied — seam ht-p03.43/ht-p03.44: handshake capability flag, no protocol_version bump; .12.8 fallback restated
r1-24 · r1 · UNOWNED-SEAM · daemon lane wiring · applied — seam ht-p03.39/ht-p03.40 (CommitKicks::register, origin rule, WorkerStatus success/failure + LaneErrorLog hook, retention in Health); .11 wires logger after lanes
r1-25 · r1 · UNOWNED-SEAM · B3 operator text · applied — seam ht-p03.45/ht-p03.46 (remedy() signature + log-path accessors); .10 implements, .11/.27 consume
r1-26 · r1 · UNOWNED-SEAM · ht-p03.5<-ht-p03.2 · applied — .2 owns b4-removed-symbols.txt; edge .5←.2; .5 acceptance widened (applied as producer/consumer edge, single producer)
r1-27 · r1 · UNOWNED-SEAM · wake outcome for unsent-after-retry · applied — seam ht-p03.41/ht-p03.42 (OutcomeUnknown 'unsubmitted', HostPort::pane_agent_state on post-collapse surface)
r1-28 · r1 · UNOWNED-SEAM · CI corpus step · applied — .26 owns the CI step (edge .26←.18), .18 drops ci.yml and owns the entry point; .26 adds SHA-pin check (single owner, reviewer fix)
r1-29 · r1 · GAP · ht-p03.11 · applied — startup fallback (piped stderr) defined; tests split into writable-but-failing and unwritable cases
r1-30 · r1 · NARRATIVE-EDGE · ht-p03.6<-ht-p03.2 · applied — edge dropped, blocked-by line removed; 10x flake criterion moved to .24
r1-31 · r1 · NARRATIVE-EDGE · ht-p03.10<-ht-p03.2 · applied — edge dropped, blocked-by line removed
r1-32 · r1 · INSUFFICIENT-INPUT · root pack · reported — findings doc and bead comments missing from pack; not re-dispatched (autonomous r1 instruction); ht-p03.37 now owns the per-finding check
r1-33 · r1 · GAP · R7 · applied — new leaf ht-p03.9.6 admission-observer tick on the Pacer; .23 ← .9.6
r1-34 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.6 · applied — .6 criterion restated (concurrency-independent counter); 10x parallel flake runs moved to .24
r1-35 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.32 · applied — edges .32 ← .17/.33/.34/.23 with (needs:) README/CHANGELOG criterion
r1-36 · r1 · UNOWNED-SEAM · ht-p03.33<-ht-p03.17 · applied — edge .33←.17 (golden layout producer); .12.7 golden baseline stated (producer/consumer edge, reviewer fix)
r1-37 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.25<-ht-p03.12.5 · applied — edge + (needs: ht-p03.12.5) Wave 18 criterion
r1-38 · r1 · NARRATIVE-EDGE · ht-p03.20<-ht-p03.21 · applied — line restated: consumes config-smoke.md outcomes
r1-39 · r1 · UNOWNED-SEAM · known_broken JSON encoding · applied — encoding pinned in .13 owns line ({min,max} ranges), .14.1 text follows it (spec-decided seam, no edge)
r1-40 · r1 · GAP · ht-p03.23 · applied — transport stated (hook CLI → daemon report → .11 logger), edge .23←.11 + test
r1-41 · r1 · GAP · R22 · applied — .23 renders known_broken Refused text in Health/doctor + test
r1-42 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.11<-ht-p03.9.4/ht-p03.9.5 · applied — edges .11 ← .9.4/.9.5/.12.6, criterion cites ht-p03.9.5
r1-43 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.24<-ht-p03.9.4/ht-p03.9.5 · applied — edges + (needs:) on the service-timing flake
r1-44 · r1 · UNOWNED-SEAM · ht-p03.34<-ht-p03.7 · applied — edge .34←.7 + (needs:) on Ctrl-C test (producer/consumer edge, reviewer fix)
r1-45 · r1 · UNOWNED-SEAM · CI Herdr availability · applied — .1 owns require/HT_SKIP_HERDR_TESTS policy; .26 provisions or reports skips; edge .26←.1
r1-46 · r1 · NARRATIVE-EDGE · ht-p03.24<-ht-p03.3, ht-p03.25<-ht-p03.2 · applied — reason lines restated as concrete artifacts (post-collapse test surface; surviving test file set)
r1-47 · r1 · NARRATIVE-EDGE · ht-p03.19/ht-p03.20<-ht-p03.9/.12/.14 · no-change — all reviewers mark these epic edges justified (whole-epic merged diff / tip)
r1-48 · r1 · GAP · R32 · applied — ht-p03.20 tip-wide actionlint + SHA-pin rg check (needs: ht-p03.16)
r1-49 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.9.4<-ht-p03.1 · applied — edge + latency/idle criteria run on the isolated session (needs: ht-p03.1)
r1-50 · r1 · GAP · ht-p03.9 retention idle bound · applied — clause moved out of ht-p03.9 epic; owned by ht-p03.12.6 (already tested there)
r1-51 · r1 · GAP · ht-p03.9.4 idle bound · applied — .9.4 idle criterion restated (≤ 1 wake pass per observation publish, zero commits); .7 consumer-contract wording corrected
r1-52 · r1 · UNOWNED-SEAM · CommitKicks lane registration · applied — folded into seam ht-p03.39 (CommitKicks::register)
r1-53 · r1 · UNOWNED-SEAM · ht-p03.9.5<-ht-p03.2 · applied — edge .9.5←.2 with consumes line (producer/consumer edge, all three reviewers' fix)
r1-54 · r1 · GAP · 20 ms sleeps in src/daemon, src/client · applied — .9.2 rg widened to (10|20)
r1-55 · r1 · GAP · ht-p03.9.5 cancellation · applied — < 20 ms cancellation from a 30 s backoff wait (needs: ht-p03.7)
r1-56 · r1 · INSUFFICIENT-INPUT · ht-p03.9 pack · reported — external excerpt truncated; tree text of ht-p03.7 does carry the appended consumer contract
r1-57 · r1 · UNOWNED-SEAM · ht-p03.9.4/ht-p03.9.5<-ht-p03.7 · applied — direct edges .9.4←.7, .9.5←.7 with consumes lines
r1-58 · r1 · GAP · send latency end to end · applied — .9.4 latency through the daemon request path; .9.1 scoped origin guard + no-leak test
r1-59 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.9.5 · applied — rg restated to the observation lane's own scope
r1-60 · r1 · GAP · daemon loop inventory · applied — folded into new leaf ht-p03.9.6 (rg inventory + allowlist)
r1-61 · r1 · NARRATIVE-EDGE · ht-p03.9.3<-ht-p03.7 · rejected — ht-p03.7's description (appended consumer contract item 2) names the standalone Backoff, wait_blocking and Cancellation; pack excerpt was truncated
r1-62 · r1 · UNOWNED-SEAM · non-lane commit origin · applied — folded into seam ht-p03.39 (request origin for pool/background threads); .9.2 consumes
r1-63 · r1 · UNOWNED-SEAM · ht-p03.12/ht-p03.11<-ht-p03.9.1 · rejected — edges ht-p03.12.6←ht-p03.9.1 and ht-p03.11←ht-p03.9.1 already exist in the tree
r1-64 · r1 · GAP · negative kick test · applied — .9.1 criterion: commit to T kicks exactly lanes_for_table(T); .9.4 asserts send-path tables map to Wakes
r1-65 · r1 · UNOWNED-SEAM · wake/work cursor shape · applied — seam ht-p03.12.10/ht-p03.12.11 (participants .12.2, .12.4, .12.5)
r1-66 · r1 · GAP · page-fit linear time · applied — .12.2 total encoded-bytes bound criterion
r1-67 · r1 · GAP · completed_at stamping · applied — .12.1 per-kind production-path tests + rg pairing check
r1-68 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.6<-ht-p03.3 · applied — edge + consumes line
r1-69 · r1 · UNOWNED-SEAM · retention Health · applied — folded into seam ht-p03.39 (retention feeds existing summary, no new line); .12.6 text updated
r1-70 · r1 · UNOWNED-SEAM · Retention kick row · applied — .9.1 states it leaves the row empty; .12.6 asserts it (single owner)
r1-71 · r1 · UNOWNED-SEAM · no-op transaction counting · applied — .9.1 already excludes no-op txns; .12.6 now also opens no write txn when nothing qualifies
r1-72 · r1 · UNOWNED-SEAM · counting fake LocalClient · applied — seam ht-p03.12.12/ht-p03.12.13 (contract ← .3)
r1-73 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.5/ht-p03.12.3 · applied — oracle moves to test_support; collect_pane_seats deleted if uncalled
r1-74 · r1 · UNOWNED-SEAM · ht-p03.12.4<-ht-p03.3 · applied — edge + consumes line (spec lists .3 work discovery as consumed)
r1-75 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.6 fair writer · applied — FairWriter is existing code moved by ht-p03.3 (root spec R18); covered by new edge .12.6←.3, criterion cites it
r1-76 · r1 · UNEXERCISED-CONFIGURATION · ht-p03.12.6 Herdr down · applied — extended .12.6 acceptance (fake-host capture failures → pruned, bounded)
r1-77 · r1 · GAP · ht-p03.9 amendments · rejected — ht-p03.9 acceptance already carried the coordinator's retention amendment (tree text); clause now moved to .12.6 per r1-50
r1-78 · r1 · GAP · wake recovery walk · applied — new leaf ht-p03.12.9 + wake_work_reserved partial index in .12.1
r1-79 · r1 · GAP · ht-p03.12.5 flatness population · applied — flatness grows live seats with wake_work and settled attention
r1-80 · r1 · GAP · ht-p03.12.5 probe indexes · applied — .12.5 INDEXED BY criterion; .12.1 verifies/creates the six probe paths
r1-81 · r1 · GAP · ht-p03.12.6 spawn · applied — production-startup criterion
r1-82 · r1 · GAP · ht-p03.12.6 pins · applied — active + previous published pins and previous-pointer reader test
r1-83 · r1 · GAP · ht-p03.12.6 shutdown · applied — cancellation during backlog drain criterion
r1-84 · r1 · GAP · ht-p03.12.6 triggers · applied — published→discarded under existing triggers criterion
r1-85 · r1 · UNEXERCISED-CONFIGURATION · canary scheduled configuration · applied — new leaf ht-p03.14.7 "Configuration smoke: canary on ubuntu-24.04 (latest and since-verified --bisect)"
r1-86 · r1 · GAP · canary report→issue chain · applied — .14.6 dry-run file_issues.py on the real step-3 report; edge .14.6←.14.4
r1-87 · r1 · UNOWNED-SEAM · canary capture-directory files · applied — seam ht-p03.14.8/ht-p03.14.9 (canary-probe.json + expected_admission, help/*.txt)
r1-88 · r1 · UNOWNED-SEAM · canary tier-1 failure marker · applied — folded into seam ht-p03.14.8 (failed_tier field); .14.3 test
r1-89 · r1 · GAP · E4 npm-installed Codex · no-change — already filed as ht-p03.35 (canary spec deferral row now cites it)
r1-90 · r1 · GAP · ht-p03.14.2 t0.config-load · applied — acceptance probes codex and claude (claude skip by design)
r1-91 · r1 · UNOWNED-SEAM · t0.payload-parse vacuous · applied — .14.6 owns: empty tier-0 capture fails when hook-fires ran
r1-92 · r1 · GAP · known_broken masking · applied — .14.1 excludes known_broken candidates, baseline moves past; ninth self-test case
r1-93 · r1 · INSUFFICIENT-INPUT · ht-p03.14 pack · reported — parent §B6/§Configurations and external owns lines missing from pack; not re-dispatched
r1-94 · r1 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14.3<-ht-p03.13 · applied — edge + (needs: ht-p03.13) on the keyed run
r1-95 · r1 · NARRATIVE-EDGE · ht-p03.14.6<-ht-p03.13 · applied — line restated (harness-versions.json + Admission::Optimistic); 'optimistic' string attributed to .23
r2-01 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.11<-ht-p03.9.6 · applied — edge + blocked-by line; criterion cites (needs: ht-p03.9.6)
r2-02 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.9.6<-ht-p03.9.2/ht-p03.9.4/ht-p03.9.5 · applied — three edges; inventory criterion (needs:) and converted loops barred from the allowlist
r2-03 · r2 · GAP · epic sleep rg owner · applied — ht-p03.9.6 owns the epic (10|20) ms rg over daemon/client/service; inventory pattern widened (\bsleep\(, recv/park/wait_timeout) and scope to src/ minus client/cli; epic attribution updated
r2-04 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.39 · applied — criterion restated to register + direct CommitKicks::kick; mapped-commit kick stays with .9.1/.40; .9.1 implements the contract's types (no "(new)")
r2-05 · r2 · UNOWNED-SEAM · wire compatibility (old CLI vs hello capabilities) · applied — adopted seam pair ht-p03.43/.44: .43 decides old-decoder tolerance (reply-field iff pre-change type tolerant, else client opt-in or separate request) + fixture test; .44 old-CLI→new-daemon case; .10 owner unchanged
r2-06 · r2 · UNOWNED-SEAM · hook parse-failure report wire compatibility · applied — folded into ht-p03.43 (capability hook.parse_failure_report, send only when advertised); .23 participant (edge .23←.43, boundary pointer); .44←.23 with old-daemon case
r2-07 · r2 · UNOWNED-SEAM · counting fake LocalClient · applied — ht-p03.12.12 scripts hello capabilities via .43's accessor; edge .12.12←.43 + criterion
r2-08 · r2 · UNOWNED-SEAM · doctor --json Claude admission fields · applied — .47 closed set fixed: listed | schema-matched, live-unverified (Codex only) | optimistic | refused | not_found; supersedes the .23 comment; .14.6 t0.admission treats not_found as infra
r2-09 · r2 · UNOWNED-SEAM · known_broken test injection · applied — ht-p03.13 owns HT_TEST_RECIPES_JSON (test-support builds only); ht-p03.14.2 owns --versions-json; consumed by .23, .49, .48, .14.9
r2-10 · r2 · UNOWNED-SEAM · admission-observer commit origin · applied — .39 decides Lane::AdmissionObserver (std thread, empty row, own counter key); .9.1 implements; .9.6 uses it; .40 asserts the all-lane idle per-origin invariant
r2-11 · r2 · GAP · admission-observer idle bound · applied — .9.6 idle criterion (zero commits with unchanged binary, ≤ 1 pass per tick, tick ≥ 5 s); R59 recorded
r2-12 · r2 · GAP · startup fallback stderr pipe · applied — .11 criterion: daemon re-points stderr after election; starter-exit survival test; R58 recorded
r2-13 · r2 · GAP · native evidence SHA · applied — .20: after the recipe-row/fix commit, full suite + ≥ 1 core cell per harness + doctor/Health admission rerun on the new SHA, recorded as the evidence SHA; R60 recorded
r2-14 · r2 · UNEXERCISED-CONFIGURATION · ht-p03.38 Claude version · applied (extended existing bead) — .38 runs Claude cells on listed 2.1.286 from an isolated npm prefix (exercises the 2.1.286 live row); 2.1.287 NOT_EXERCISED-pending-admission there, run by .20
r2-15 · r2 · GAP · R52 · applied — new leaf ht-p03.50 "Spec reconciliation" (root §B1 Closes/D1, §B2 D3, Retention kick row, R2 wording; rg check); no deps, runs first round
r2-16 · r2 · GAP · nested spec reconciliation · applied — folded into ht-p03.50 (B1 nested D1/D2/D4/D6/Acceptance, pacer L6/D1/D2/D5, canary §D3/§D7/§D11)
r2-17 · r2 · GAP · R55 · applied — .1 shell helper honours HT_SKIP_HERDR_TESTS; .31 install_test.sh reports skipped cases by name; .26 runs install_test.sh in CI (edge .26←.31); ledger r1-45 superseded
r2-18 · r2 · UNOWNED-SEAM · wake outcome for unsent-after-retry · applied — .41 restated: last_outcome keeps the existing OutcomeUnknown string (pacer D5), 'unsubmitted' only in WakeDriveOutcome + daemon.log; .9.3 owns the store write + criterion; .42 restated
r2-19 · r2 · UNOWNED-SEAM · admission spawn site · applied — spawn-failure rule added to .39; edge .27←.9.6 (check applied to the Pacer lane spawn)
r2-20 · r2 · UNOWNED-SEAM · retention-origin kicks · applied — .39 rule: Retention-origin commits kick no lane; .9.1 implements + test; .12.6 criterion; .40 idle assertion with retention running
r2-21 · r2 · UNOWNED-SEAM · recovery page cursor · applied — folded into seam ht-p03.12.10/.12.11 (recovery cursor, seat-ordinal order, longest_cursor_bytes); .12.9 participant (edges .12.9←.12.10, .12.9←.3), .12.11←.12.9; .12.2 owns only the ~595 fit tail
r2-22 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.6 baseline pins · applied — pin criterion conditional on tables surviving ht-p03.2 (b4-removed-symbols.txt via .12.1 migration test); .12.1 consumes the list
r2-23 · r2 · UNOWNED-SEAM · ht-p03.34<-ht-p03.33 · applied — edge + blocked-by line; .34 criterion every follow notice goes through escape_for_terminal
r2-24 · r2 · UNOWNED-SEAM · canary expected_admission domain · applied — .14.8 closed set (listed | optimistic | schema-matched-or-optimistic | refused | unasserted), --baseline never affects it; .14.6 resolves codex verdict after t0.schema + test
r2-25 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.37<-seam integrations · applied — edges .37←.40/.42/.44/.46/.48 + criterion naming seam-closed rows
r2-26 · r2 · GAP · R22 · applied — .13 ladder row major_version_change; .23 label test (needs: ht-p03.13)
r2-27 · r2 · UNEXERCISED-CONFIGURATION · ht-p03.14.7 ubuntu route · applied (extended existing bead) — branch-push option removed; --platform linux/amd64 required (arch recorded); job-tail configuration (file_issues --dry-run + exit propagation); NOT_EXERCISED points to §Follow-on item 2; .32 checklist line
r2-28 · r2 · UNOWNED-SEAM · INDEX status lines · applied — .36 is the final INDEX owner; edge .36←.5
r2-29 · r2 · GAP · daemon stop under skew · applied — .10 criterion: daemon stop terminates a fake old-version daemon without decoding; ensure starts the new one; R61 recorded
r2-30 · r2 · NARRATIVE-EDGE · ht-p03.14.6<-ht-p03.23 · applied — split per §Splitting a Bead: doctor PATH check + doctor --json fill moved to new leaf ht-p03.49 (deps .13, .47); .14.6 and .48 repointed to .49; .23←.49; .47 participant .23→.49
r2-31 · r2 · GAP · ht-p03.19 exclusion set · applied — .19←.40/.42/.44/.46/.48/.49 and exclusion set names seam-integration inline fixes
r2-32 · r2 · GAP · Wakes-mapped commits per observation cycle · applied — .9.5 criterion ≤ 1 Wakes-mapped commit per idle cycle via the kick sink; ledger r1-51 superseded
r2-33 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.9.5 Herdr-stopped bound · applied — restated: after the first failure step, ≤ 1 commit per further step; first-failure kicks recorded
r2-34 · r2 · UNEXERCISED-CONFIGURATION · Herdr stopped with pending wakes · applied (extended ht-p03.9.4) — ≤ 2 commits per seat per refusal step to the 30 s cap, one Submitted per seat after restart
r2-35 · r2 · UNOWNED-SEAM · Pacer consumer contract · applied — .7 owns wait_blocking, Backoff, Notify-backed Cancellation (src/protocol/time.rs), tick reset; four tests; ".9 child" reference removed; ledger r1-61/r1-56 superseded
r2-36 · r2 · GAP · ht-p03.9.6 Health suffix · applied — .9.6 retry-suffix criterion (needs: ht-p03.9.1)
r2-37 · r2 · GAP · ht-p03.9.4 backoff qualifier · applied — .9.4 backs off only on unclassified Err; Refused seat does not bump lane attempts
r2-38 · r2 · GAP · mapped-table hook coverage · applied — .9.1 every-table insert/update/delete kick test + no WITHOUT ROWID + no unqualified DELETE on mapped tables
r2-39 · r2 · GAP · WriterTurn exclusivity · applied — .9.1 criterion: writer guard private to WriterTurn
r2-40 · r2 · GAP · cross-lane kick · applied — .9.4 deadline-commit → wake attempt < 100 ms criterion
r2-41 · r2 · GAP · ht-p03.9.2 fallback and drain · applied — .9.2 no-runtime revoke and drain-deadline criteria
r2-42 · r2 · INSUFFICIENT-INPUT · ht-p03.9 pack · reported — ht-p03.12.6 absent from the ht-p03.9 pack; aggregator confirmed from the root pack that .12.6 carries the retention idle bound and the empty Retention kick row; not re-dispatched (final round)
r2-43 · r2 · NARRATIVE-EDGE · ht-p03.9.2<-ht-p03.39 · kept-as-is — reviewer offered drop-edge or add-criterion with no stated preference; seam-contract edge kept, recorded
r2-44 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.5 flatness population · applied — live non-human seats held fixed, settled history grows; separate linear-in-live criterion; ledger r1-79 superseded
r2-45 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.10 · applied — .12.10 rejection ships switched off; .12.5 switches it on with the emission change (criterion moved)
r2-46 · r2 · GAP · retention pin stall · applied — .12.6 candidate query excludes pins (or cursors past them) + criterion
r2-47 · r2 · GAP · work_jobs_retention scan · applied — .12.1 index keyed (kind, completed_at); .12.6 retention-pass flatness criterion + edge .12.6←.6
r2-48 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.2 bytes bound · applied — bound restated c·(n·base + page_bytes·(1+⌈log₂ n⌉))
r2-49 · r2 · GAP · occupant_bindings_current probe · applied — .12.1 verifies seven access paths; .12.5 prepares all seven
r2-50 · r2 · GAP · work_jobs FK/trigger audit · applied — .12.6 criterion
r2-51 · r2 · GAP · retention lane reporting · applied — .12.6 record_failure/record_success/retry-suffix criterion (needs: ht-p03.39); D4 deviation recorded by .50; ledger r1-69 superseded
r2-52 · r2 · GAP · ht-p03.12.4 flatness population · applied — superseded generations + targets grow 10× too
r2-53 · r2 · UNOWNED-SEAM · transcript goldens · applied — .12.7/.12.8 goldens restated as differential (change reverted on the current tip); ledger r1-36 superseded for these leaves
r2-54 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.12.13 · applied — per-invocation counts
r2-55 · r2 · UNOWNED-SEAM · page_fit.rs creator · applied — .12.10 creates page_fit.rs with the PageFit stub; .12.2 implements it
r2-56 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14.8 · applied — criterion restated to a hand-written sample; .14.8 creates the stub files and the src/harness/mod.rs mount; .14.1 validates stub_probe.py; .14.1/.14.2 Files "extends"
r2-57 · r2 · GAP · known_broken bisect rule · applied — .14.1: +1 probe per crossed range, open-ended range still probes newest, known_broken_persists (no issue); .14.4 files nothing for it; .14.7 candidates minus ranges; spec text via .50; ledger r1-92 superseded
r2-58 · r2 · GAP · tier-1 failure plant · applied — .14.3 owns HT_CANARY_PLANT_TIER1_FAIL; .14.9 consumes; ledger r1-88 superseded
r2-59 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14.6 second dry run · applied — .14.4 owns --existing-issues; .14.6 criterion restated; ledger r1-86 superseded
r2-60 · r2 · GAP · t0.isolation absent file · applied — .14.2 defines absent→absent pass / created → infra + test; .14.7 asserts it
r2-61 · r2 · UNOWNED-SEAM · known_broken snippet constructor · applied — .13 pins VersionSet::Interval { min, max } (Option<Version>) + constructor test; no edge
r2-62 · r2 · GAP · ht-p03.14 epic acceptance · applied — tier-1 [ht-p03.14.3] and file_issues dry-run [ht-p03.14.4] bullets restored; ledger r1-08 superseded
r2-63 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14.6 newest classifies optimistic · applied — "classifies as harness-versions.json expects at run time" in .14.6 and the epic
r2-64 · r2 · GAP · D12 step-3 full rerun · applied — step 3 owned by .14.2; .14.6 reruns it with the full tier-0 set to feed the dry run + criterion
r2-65 · r2 · GAP · witness restore before unsetup · applied — .14.6 restores the real binary after hook-fires; payload-parse limited to 'hook <h>' argv + test
r2-66 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-p03.14.2 fixture version · applied — committed fixture under tests/harness/testdata/canary/ naming a listed version
r2-67 · r2 · sweep · ht-p03.51 · created — Integration sweep leaf; depends on all 74 non-epic beads except ht-p03.20; ht-p03.20←ht-p03.51 ("consumes integration sweep result")
g-01 · graph · GRAPH-EDGE · ht-p03.44 <- ht-p03.23 · kept — capability-gated hook parse-failure report (old-daemon criterion)
g-02 · graph · GRAPH-EDGE · ht-p03.41 <- ht-p03.3 · parked — drop judged safe no: shared src/ports.rs, src/scheduler/mod.rs; depth 13→12 alone
g-03 · graph · GRAPH-EDGE · ht-p03.9.6 <- ht-p03.9.4 · kept — loop inventory + epic rg run after the converted lanes (split proposal parked)
g-04 · graph · GRAPH-EDGE · ht-p03.37 <- ht-p03.19 · kept — review-debt.md dispositions feed closure-ledger rows
g-05 · graph · GRAPH-EDGE · ht-p03.36 <- ht-p03.12 · applied — narrowed to .12.1/.12.4/.12.5/.12.9/.12.6/.12.8; blocked-by, consumes and needs lines rewritten
g-06 · graph · GRAPH-EDGE · ht-p03.36 <- ht-p03.9 · applied — narrowed to .9.1/.9.4/.9.5/.9.6 (no leaf wait removed); blocked-by and consumes lines rewritten
g-07 · graph · GRAPH-EDGE · ht-p03.9.4 <- ht-p03.9.3 · kept — WakeDriveOutcome::next_due_at (seam-contract proposal parked)
g-08 · graph · GRAPH-EDGE · ht-p03.32 <- ht-p03.23 · kept — shipped Optimistic wording the CHANGELOG must match
g-09 · graph · GRAPH-EDGE · ht-p03.27 <- ht-p03.9.6 · kept — admission-observer Pacer spawn site
g-10 · graph · GRAPH-EDGE · ht-p03.23 <- ht-p03.27 · kept — Health line budget and folding rule
g-11 · graph · GRAPH-EDGE · ht-p03.23 <- ht-p03.11 · kept — rate-limited logger for hook parse failures
g-12 · graph · GRAPH-EDGE · ht-p03.20 <- ht-p03.9 · kept — whole-tip native rerun (r1-47)
g-13 · graph · GRAPH-EDGE · ht-p03.20 <- ht-p03.14 · kept — whole-tip native rerun (r1-47)
g-14 · graph · GRAPH-EDGE · ht-p03.20 <- ht-p03.51 · kept — integration-sweep edge, exempt
g-15 · graph · GRAPH-EDGE · ht-p03.20 <- ht-p03.12 · kept — whole-tip native rerun (r1-47)
g-16 · graph · GRAPH-EDGE · ht-p03.19 <- ht-p03.32 · parked — drop judged safe no: .19 consumes every B7 bead's merged diff; no depth change alone
g-17 · graph · GRAPH-EDGE · ht-p03.19 <- ht-p03.9 · kept — review exclusion set = whole-epic merged diff (r1-47)
g-18 · graph · GRAPH-EDGE · ht-p03.19 <- ht-p03.44 · kept — seam-integration inline fixes in the exclusion set (coverage r2)
g-19 · graph · GRAPH-EDGE · ht-p03.19 <- ht-p03.14 · kept — review exclusion set = whole-epic merged diff (r1-47)
g-20 · graph · GRAPH-EDGE · ht-p03.19 <- ht-p03.12 · kept — review exclusion set = whole-epic merged diff (r1-47)
g-21 · graph · GRAPH-EDGE · ht-p03.11 <- ht-p03.9.6 · kept — admission-observer lane failure hook to LaneErrorLog
g-22 · graph · GRAPH-EDGE · ht-p03.3 <- ht-p03.2 · kept — post-deletion ports/store surface; same files
g-23 · graph · GRAPH-EDGE · ht-p03.9.6 (split) · parked — proposal: admission-observer half vs inventory half; repoint .27/.11/.23; depth 13→12
g-24 · graph · GRAPH-EDGE · ht-p03.9.4 <- ht-p03.9.3 (seam) · parked — proposal: next_due_at into contract ht-p03.41; with g-23, depth 13→11
