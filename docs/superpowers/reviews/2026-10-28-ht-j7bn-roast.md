super-roast verdict: clean (2 nits) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (a local multi-agent messaging daemon) with a strict test-discipline culture. The change touches only a test and the nextest schedule; the blast radius is CI signal quality and suite wall time.
inputs: bead/ht-j7bn@8b692df13 vs dc6ef629f
coverage: unavailable — inline run, no coverage object; lanes covered inline: correctness, test-causality-lifecycle, test-suite-cost
independence: none (inline) · rung: inline

## Change under review

- `.config/nextest.toml`: `hermes_enrollment_smoke::hermes_bare_enrollment_cold_daemon_configurations`
  runs alone at the front (`threads-required = "num-test-threads"`, priority 100), like the
  cap-asserting `registered_resume` Hermes tests; it stays in the `hermes-smoke` group.
- `tests/integration/hermes_enrollment_smoke.rs`: configuration 8's seat assertion prints the
  bridge step (`elapsed_ms`, raw hook answers); configuration 10 accepts a `current` second turn
  when no ack of the cold first turn reached the bridge.

## Confirmed findings

None in the inline round. The independent review afterwards confirmed one (fixed, below).

## Independent review follow-up (fixed)

- [Should-fix] tests/integration/hermes_enrollment_smoke.rs, configuration 10 — the relaxation
  accepted a `current` second turn whenever the first turn's last answer had no ack, not only
  when the cap ran out (`ack_mode` reads the last spy entry; a cap timeout appends nothing). A
  fast first turn (under the driver's 1 s slow mark) answering without a `lifecycle_ack` after
  the daemon committed the startup check-in, the ht-nzzg gap, passed although it used to fail.
  Fix: the branch is gated on cap exhaustion (no answer reached the bridge, or the turn took
  >= 1000 ms); otherwise the second turn must be a seated `startup` one. Shown with a temporary
  driver edit stripping the first turn's `lifecycle_ack`: when that turn had checked in
  (116 ms), the test fails at `step 0: the first turn delivered no context ... mode current`;
  runs whose first turn failed open (no check-in, second turn `startup`) pass, as before.

## Rejected (with reason)

- [Should-fix] "Running alone hides a real product race (ht-jhcl-style HostUnavailable on the
  first claim)". Rejected: the daemon attaches the startup hint host before the listener serves
  (`src/app.rs`, `attach_startup` before serving, comments at ~1969/2123), so a cold daemon's
  first `ResolveSeat` finds the session and its index entry already published; ht-jhcl's window
  needs a concurrent attach, which a cold start does not have. Daemon log of a local run: store
  open to `herdr host attached` in 12 ms.
- [Should-fix] "Configuration 8 becomes vacuous". Rejected: it is unchanged and strict (exactly
  one seat after the start hook). Shown to fail with a temporary driver edit making the start
  hook's answer never reach the bridge (daemon pre-started): `the start hook did not enroll w1:p1
  inside its cap: {"callback":"on_session_start","child":[],...}`; with no hook answers at all it
  fails at `the start hook did not bring the daemon up`.
- [Should-fix] "Configuration 10's new branch is unreachable or masks a wrong mode". Rejected: it
  was reproduced (1 of 16 runs under two stress loops): the first turn returned no ack and the
  second turn's own check-in came back `current` with empty context, which only a startup
  check-in committed for that binding by the first turn can produce. The final one-seat,
  one-open-binding, one-binding-ever checks still run on every path.
- [Nit] "threads-required and the hermes-smoke group conflict". Rejected: the same combination
  is already in use (`registered_resume::startup_enrollment_hermes_qualified_unowned_never_resolves`
  matches both a `threads-required` override and the `hermes-smoke` group override);
  `cargo nextest list` parses the config.

## Unverified nits (spot-checked)

- [FYI] The CI classification is inferred, not observed: the bridge discards the hook's stderr
  diagnostic, and the CI log has only the panic. Evidence: failure at 7.0 s against 3.5 s alone
  locally (start hook 0.6-1.3 s idle, up to 2.8 s under two loops), daemon up but no seat, and no
  pre-commit fast-fail path left once the attach race is excluded. The configuration-8 message
  now prints the bridge step, so a recurrence shows whether the hook used its cap.
- [FYI] Suite cost: about 4 s locally (about 7 s on a CI runner) with the shard's other slots
  idle, as for the precedent tests. Within the per-shard budget; noted for the ht-zo4 accounting.

## Escalations (need human)

- Product follow-up, not a defect of this diff: when a cold Hermes turn runs out of its cap
  after the daemon committed the startup check-in, the startup context never reaches the model,
  and the next turn is a `current` check-in (no restated startup). Whether a committed but
  undelivered lifecycle check-in should be re-offered is a delivery-contract decision; worth a
  bead.
