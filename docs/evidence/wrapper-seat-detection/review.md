# Historical independent checkpoint review

Reviewed 2026-10-06 against base `cbd5b706` by independent agents `independent_review`
and, after the live Bash-tool probe, `checkpoint_review`. This is a draft checkpoint,
not approval to integrate the combined feature into main at that checkpoint.
The subsequent user decision adopts user-managed foreground configuration instead of
automatic TUI-pane recovery. The current scope is documented in TRUST-POLICY and
[the final feature review](final-review.md).

The checkpoint P1 was: removing forced embedded Codex execution permits a shared app-server,
while `HookEnv::from_process` still selects ambient `HERDR_PANE_ID`. Native Codex
0.160.1 SessionStart and PreToolUse hooks receive the server pane. A client in another
pane can therefore check in against the wrong seat. Explicit tool environment policy
corrects only the tool claim. Automatic TUI seat attribution is not implemented.

The reviewers found no additional defect in the argument change: caller arguments
remain in order, automatic `--no-daemon` injection and wrapper deduplication are removed,
and configuration export probes remain. The stale setup documentation was corrected.

The second reviewer validated all three actual native `exec_command` cases: each
scripted SSE call receives a matching native `function_call_output` containing the
capture; PreToolUse identifies `Bash`; hook sessions match tool `CODEX_THREAD_ID`.
Captured cleanup shows four owned process groups absent, leaders reaped and scratch
removed. This is an owned-process check, not a general detached-process audit. Evidence
uses scratch trust bypass and full access; it qualifies neither hook trust nor sandbox
behavior, and there is no live Claude daemon-tool capture.

The missing input is an explicit cooperative client-pane claim correlated with the
actual harness session and available to both lifecycle hooks and tools. An existing
wrapper or a supported upstream interface can provide it; this work installs no new
wrapper. The daemon must decide against its canonical view. Process names, foreground
argv, timestamps, focus and cached host sessions cannot replace that input under
TRUST-POLICY C1/C5/A2.

Verification retained from the implementation checkpoint: 61 launch tests, 80 setup
tests and one emitted-flag canary passed. Root reran the rejecting-wrapper regression;
the first reviewer ran two focused argv/wrapper tests. Required all-feature clippy,
default-feature check, formatting and diff guards passed. No full suite or release
operation was performed. At that checkpoint, later changes added probe evidence and documentation only.
The final feature also adds installer foreground advice and optional launch arguments.
