# Frozen6ca final-suite failure

The authorized unfiltered suite on6ca86d79a853350410534617013e7379e7deccda
(tree060578bc929650187130289b3523636391cbb5c3, equal to main4dab merge-tree)
finished exit100:9779run,9778PASS/1FAIL/64profile skips,925.402s execution,
0.03s compile,926.305s wall.19binaries, profileci, cap4/jobs4/testthreads4,
nice19/warmtarget/unchangedgroups. Fresh eedcb7d6 scoped process/temp checks were
CLEAN on their first invocation; cleanup exit0. JUnit has9779executed cases,
1failure/0errors; profile skips are reported by the log rather than JUnit cases.

Sole failure: service::dispatch::launch_gate_guards_tests::
turn_state_hands_the_hint_to_the_host_for_the_harness_process, launch_gate_guards.rs252.
The old recorded:false assertion masks a real coupling: service treated any
accepted legacy edge as permission to publish a host wake hint. Restoring legacy
activity therefore let an unregistered claim reach note_wake_hint. The fix must
preserve cooperative memory activity while separately requiring canonical hint
eligibility, retaining unregistered/wronghost/held/subagent negatives and the
registered harness-pid positive. This is not a test-only failure or qualification.

Source stayed fixed throughout the run. Astra268 approved an internal explicit
hint_eligible field after completion; main's corrected-run disposition relayed in
Astra269 permits exactly one more full run after focused guards and narrow
acceptance. No blind repeat, live run or broader review is authorized by this
failure record. The original raw log, timing, JUnit and first cleanup remain under
fixed-tree-*; future corrected evidence must use distinct names. No speed-budget
pass is claimed. The separate documented P1 ht-d2jq.8 is unchanged.
