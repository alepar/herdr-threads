# V090 Popen registration correction — preparation only

Independent review R1 reproduced SIGTERM/SIGINT after an owned group existed but before `Popen` returned and assigned `child`. The earlier corrected runner hash `58232505ee8acf9edcf93fa03497f2f749cea399b1ae565527f8b37db87f4bc9` and its report/validation remain historical evidence; that version had the ownership gap.

The new runner wraps Popen creation and assignment in `cancellation.defer()`. A catchable signal records cancellation without raising inside the constructor. Once the child reference is assigned, leaving the deferral immediately raises into the existing finally block, which now has the owned child and runs its group cleanup. This closes the reproduced catchable-signal creation/registration gap. Exact selection, recovery semantics and log bounds are unchanged.

Replayable `python3 resume-v090-popen-race-mock.py` uses the actual runner main/control flow with subprocess, signals, selectors, source checks, binaries and selection transports mocked. It creates no OS child and sends no signal. Both SIGTERM and SIGINT are injected after fake group creation but before fake Popen returns. Negative controls remove only the registration guard in memory and reproduce the unowned focused group. Corrected cases clean up that focused group, return143/130, reach focused and final scoped scans, restore both handlers and skip mandatory execution. Other fake cleanup calls are recorded too; they are not real leak scans. Four cases passed. Actual process cleanup verification remains parent-owned.

| Artifact | SHA-256 |
| --- | --- |
| `resume-v090-run.py` | `dce287cba7abcd89e0a0fce0ad63af50ac2bf1e333deff5391f7c4d09cbc267f` |
| `resume-v090-popen-race-mock.py` | `cd679fd30c192020385e960239f933cc61e9e7b668c5f57932ddfed6c4924f98` |
| `resume-v090-popen-race-validation.json` | `8ee161ecc2f95248529330f57f795b5c5c9734ccbbf3c48b305f415b9323c7a7` |

No Cargo, project runtime, OS signal, process termination, Git mutation, project source edit, plugin or Herdr write occurred. No final source seal or execution success is asserted. Historical artifacts remain byte exact. QUIESCENT for independent follow-up review.
