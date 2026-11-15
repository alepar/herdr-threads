# Task 3 driver review fixes (I1/I2)

Reviewed base: f3112fa24df8314c0f5d730798caba99d840cd06. This evidence covers
credential-free driver self-tests, not final native or live-provider qualification.

I1: Negative wake checks now capture their run baseline before the triggering
send. Every quiet assertion takes an explicit baseline. Passive sends share a
pre-send helper; blocked draft/selector cases also compare the exact styled pane
and native editor, session, provider and model after the observation interval.
Busy and failed-boundary checks retain their original pre-send baseline through
the trailing quiet window.

RED: fake sends that synchronously incremented the native-start count passed the
old threshold/live quiet interval and reached the manual-turn step. The new
checks reject that run. Separate tests reject changed picker/editor rendering or
changed native editor/session/provider/model even when no model run starts.

I2: The private helper starts in its own process group, with signal traps for its
existing owned-root teardown. Cleanup attempts each resource independently.
A graceful timeout/interruption triggers bounded TERM, then KILL if needed, pipe
closure and reaping. An ownership-checked invocation of the existing isolated-host
teardown handles a surviving root. Exact-run leak cleanup and a final rescan
cover remaining tagged descendants; root removal requires the matching owner.
Timeout/interruption remains a qualification failure even after recovery.

RED: timeout, KeyboardInterrupt and TERM-ignoring fake helpers all remained
unreaped with the old cleanup. GREEN failure injection verifies status 1, helper
reaping, child reaping, removed scratch/credential files and a clean exact-run
process scan. An out-of-run control process survives each cleanup, then the test
explicitly terminates and reaps that control. These tests start no Herdr server,
no native harness and no provider, and read no real credentials.

Validation (authorized heavy/toolbox wrappers, elevated host namespace, cap4):

- `python3 scripts/pi-tui-qualify.py --self-test`: 8 passed.
- `scripts/check-no-leaked-processes --run-id 1a110b1d-c7b3-4966-b2c1-97e8e5950ea3`: clean processes and temp files.
- `git diff --check -- scripts/pi-tui-qualify.py`: clean.
- Read-only historical npm PID check outside the sandbox: absent; no signal sent.

The fix changes only the dedicated driver and its embedded self-tests. Final
native/live/full-suite gates remain pending the integrated startup fix and review.

Driver SHA-256: `8cff2c5d049bf97473278189c505cfec7b4ac04d81052521aa5c6fb39b3682fa`.
