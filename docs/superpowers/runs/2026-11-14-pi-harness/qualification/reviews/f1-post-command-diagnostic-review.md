# Passive post-command diagnostic review

**Approved for the proposed diagnostic-only change.** Root retains execution GO. No production fix or accepted limitation is approved.

Reviewed patch SHA256 `19c85b3221a144cc00ced82a966cb516a7919a4f7692cee3cd7a08f906e143d9`. Independently reconstructed the three-line insertion in memory: driver SHA256 changes from `ce7567bb9bbba0b6f56a0eee2e883a64955c9590dfba111734eca0241d83b5f2` to `8c165a593b5a62403c9ba242c3c6a3c38fddbc515d41e4b044a2ac04f86f406d`.

The insertion follows the actual `notice_cause` event wait. `observe_state(sid=...)` requests a fresh nonce-tagged state observation without requiring an editor value (`scripts/pi-tui-qualify.py:562`). The provider only samples/logs in response to that observation request; it does not render, type, or update editor state. `capture` performs visible, visible ANSI, detection, and agent-info reads (`:192`). Attaching the state observation changes retained evidence only. Existing qualification assertions, wake eligibility, receipt deadline, editor writes, keys, and provider implementation are unchanged.

The extra observation/read latency is real. The capture occurs before the existing eight-second-deadline validation, which remains unchanged and can refuse if the added delay exhausts its precondition. This is not a simultaneous snapshot or a near-submit witness: API state precedes the four sequential host reads. The diagnostic can establish post-command internal/visible divergence without claiming the exact daemon input.

No source patch applied, tests/builds run, or runtime actions performed by this reviewer.
