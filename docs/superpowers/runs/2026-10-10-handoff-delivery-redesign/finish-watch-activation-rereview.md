# Watch activation scoped rereview — correction round 1

WA-R1: **ADDRESSED**
WA-R2: **ADDRESSED**
Spec: **PASS** for the prepared Watch test source and this correction delta; elected activation execution remains pending.
Quality: **PASS** for this correction delta, supported by the five executed helper regressions below.

Reviewed 2026-10-11 at supplied immutable helper checkpoint `0007ada15cdd131d4a358f8929ccd18cfe8dd6a0`, successor of `8cd4fc07`. Scope: the two prior findings and newly introduced breakage in `tests/integration/mod_delivery.rs`; the earlier unaffected coverage assessment carries forward. Source read from `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/finish-watch-activation`. Parent reports the equivalent formatted transfer on D `31e58797`; the inspected execution log identifies D's worktree. I did not independently resolve git HEAD or binary identity.

## WA-R1 disposition

The controller now snapshots diagnostics/results and drops its mutex guard before main-thread assertions in `wait_reply_for`, `release_reply`, `release_attention_after` and `assert_resumed_after_attention`. Worker invariants return recorded errors through `fail` instead of panicking while owning the controller. Mutex and condvar reacquisition recover poisoned inner state, including `stop`. Pull classification releases the exchange-log guard before acquiring the controller, eliminating the identified nested poisoning path.

Relay Drop now shuts down owned sockets before taking the controller lock, so a real forwarding write holding that lock can be interrupted. It then stops/notifies the controller, joins the worker and restores the private socket pathname/descriptor. The open-socket lock in Drop also recovers poison. A bounded downstream write timeout supplies a fallback. These changes address the prior double-panic abort and the additional lock-order requirement introduced by serialized output.

The new regression sources meaningfully exercise ordinary assertion unwinding, real relay destructor restoration/socket EOF/waiter completion, explicit controller poisoning with a condvar waiter, and observable worker failure. They are helper tests, not fabricated daemon-success fixtures.

## WA-R2 disposition

`forward_attention` holds the same mutex as Pull observation across the actual unchanged frame prefix/body write. It rechecks the expected observed request count before output. Permission does not create a marker; only complete successful output increments the forwarding order. A failed/partial write records failure, stops/notifies the controller and records no forwarding marker. The relay uses this method directly for actual daemon Attention frames and avoids writing those frames twice.

The regressions cover permission without output, a failure after prefix output, an extra observed Pull before forwarding, and successful framed output through an actual Unix socket pair. The successful-output test probes lock exclusion before both prefix/body socket writes, verifies the exact bytes at the peer, and then verifies the next observed Pull's order. This establishes the requested serialized observer ordering. It does not claim syscall-level client receipt, causal model uptake, or proof of an arbitrarily long quiet future.

No new source defect requiring correction was found in the scoped delta. No production changes, synthetic active capability, global drain cap or timing-based quiet oracle were introduced.

## Execution evidence inspected

I read `/private/tmp/ht-finish-capacity-successor-sweep.log`, including its initial run metadata, each covering helper result and final summary. Nextest run ID: `c02192e1-5457-45a1-a92f-d819a4a099e7`. The log shows **127 run / 127 passed**, two binaries, 4,425 skipped, runtime **8.847 s**. Compilation is separately reported as **1m 27s**; this multifile run is not a controlled incremental-build measurement.

All five exact covering tests passed:

- `mod_delivery::activation_capacity_flow_failed_or_unwritten_attention_has_no_marker` — 0.017 s.
- `mod_delivery::activation_capacity_flow_stop_recovers_poison_and_releases_waiter` — 0.021 s.
- `mod_delivery::activation_capacity_flow_oracle_assertions_do_not_poison` — 0.023 s.
- `mod_delivery::activation_capacity_flow_successful_frame_write_serializes_pull_observation` — 0.013 s.
- `mod_delivery::activation_capacity_flow_oracle_unwind_restores_relay` — 0.064 s.

These results support the helper corrections. They do not execute the five elected `activation_v2_*` tests. The actual early dormant-advertisement assertion, advertised same-child Watch chain and corrected full-capacity elected behavior still require parent execution. The earlier opaque four dormant failures remain unclassified; they are not retroactively meaningful RED. This log also does not independently establish final format/lint/default-feature checks, scoped leak-check completion, optional capture provenance or final active-source mapping.

**No Watch activation clearance is granted.** Production remains unadvertised per the supplied checkpoint. Parent retains all Cargo, activation, final readiness and leak-cleanup responsibilities.

I performed read-only source/report/log inspection and wrote only this report. No tests or other native execution, source/git/Herdr/tracker mutations, subprocess fixture starts, or subagents.

**COMPLETE + QUIESCENT.**
