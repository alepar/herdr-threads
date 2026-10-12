# ht-54r.88 triage: agy native gate, base vs branch

Environment: genuine agy 1.3.3 (darwin_arm64, version recorded as evidence only, no pin), fake upstream (no `--live`,
zero spend), isolated HOME and isolated Herdr test session per flow. Base = 43cc19cd (temp worktree, own
`scripts/agy-native-gate`, own build, removed afterwards). Branch = integration tip f7341e91 before this task's edits.
Reports: `base-report.json`, `branch-report.json` (unmodified tip), `branch-fix1-report.json` (clear, resume),
`branch-fix2-report.json` (delivery, downgrade, wake), `branch-final-report.json` (all 15 after the edits).

| flow | base | branch (tip, before edits) | failing check on branch | cause | decision |
|---|---|---|---|---|---|
| allow-rewrite, boot, enroll, handoff, launch, permission, setup, subagent, unsetup | PASS | PASS | | | none |
| run_command_env | N/A (branch-only flow) | PASS | | | none |
| clear | PASS | FAIL | same seat after /clear; generation advanced by one; native session is the new id | `await_binding(generation != before)` and "generation +1" encode one binding per seat. Under session-keyed bindings `/clear` enrolls a new row (`generation` is the seat's mapping generation at enrollment, unchanged), so the predicate never matched and `after=None`. The product enrolled correctly: `bindings.json` shows old row `disconnected`, new row `connected`, same seat. | gate assumption; flow updated to session-keyed semantics (new connected top_level binding keyed by the new presence-lock id, same seat, old binding no longer connected, no new seat). No product change. |
| resume | PASS | FAIL | `--conversation A` rebinds the same seat to native session A; `-c` without rotation | `live_binding` returned the highest-ordinal open row, which is B's (disconnected) row. A's own row (ordinal 1) is reconnected by the resume, as the spec says (a claim matching an existing key reconnects that row). Product behaviour is correct (`bindings.json`: A connected, B disconnected, 2 rows total). | gate assumption; helpers now select the connected open top-level row, and the flow asserts A's own `binding_id` is connected again, B's is not, no new row on `-c`, no new seat. No product change. |
| delivery | FAIL (wake-join not served; stop phase consumes nothing) | FAIL | the sender invited the agy seat and it accepted, then every later check cascades | `_l6.join` ran `accept` from the gate process: no `ANTIGRAVITY_CONVERSATION_ID`, no agy ancestor, so `caller::active()` is not `Agent` and the CLI falls to the retired pane-context path: `caller_not_located ... has no lifecycle check-in context yet` (src/cli/mod.rs ~2690). The product refuses an unattributable caller as designed (TRUST-POLICY: agent seats have no pane-context fallback). Base failure is a different cause: accept ran first and removed the reason to wake (ht-poh.46), so `wake-join` was never served. | gate assumption (both). The invite's idle wake is now answered by a scripted `run_command` `herdr-threads accept <thread>` (same mechanism `run_command_env` proved attributes), so the accept runs as the agy session and the wake is served. Asserted: output "exited with code 0", no refusal code, membership `joined`. No product change. |
| downgrade | FAIL (abort: doctor reports the downgrade (abort) for the identity) | FAIL | same accept failure, cascading | same as delivery (accept from outside agy). The base-only failure (`doctor` downgrade report) is not reproduced on the branch once the accept is real: see final run. | gate assumption; accept via scripted `run_command` (shared `_l6.join`). No product change. |
| wake | PASS | FAIL | accept; then b/c/d/e/f cascade; and `e: the seat's wake preference is now all` | accept from outside agy (cascade); and `wake-on` read from the gate process hit the same refusal. | gate assumption; accept via `join`; the preference is read from `seats.wake_on` in the daemon database (the model already set it through `run_command`). No product change. |

## Product regression?

No. In every failing flow the product refused or recorded exactly what the spec says; the flows impersonated the
agy seat from outside agy (pre-binding pane-context model) or assumed one binding row per seat. The TRUST-POLICY
rule "do not restore the agent pane-context path" was respected: no Rust changed.

## Result after the edits (branch tip + this task)

- `branch-final-report.json`: 14/15 PASS (clear, delivery, downgrade, resume and the ten unchanged flows); `wake`
  failed only `e: the seat's wake preference is now all` (my first query matched the sender's seat too).
- `branch-fix3-report.json`: `wake` PASS after scoping the query to the agy seat. The only change between the two
  runs is that one query in `flows/wake.py`.
- Gate leak check clean in every run. 15/15 flows pass on the branch.
- Offline: `/usr/bin/python3 -I -B -m unittest discover -s scripts/agy-gate -p 'test_*.py'` -> Ran 137 tests OK.
  RED runs before the helper/scenario changes: `red-offline-l6.txt`, `red-offline-l5.txt`.
- Base-only failure for downgrade ("doctor reports the downgrade (abort)") did not occur on the branch after the fix;
  it is recorded for the base for completeness and needs no action in this epic (base is not the merge target).
