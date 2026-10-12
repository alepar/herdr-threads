# Controlled capacity-fixture experiment

Status: proposed, not executed. Only this report is written. Parent owns any scratch source change, Cargo invocation, cleanup and acceptance decision.

Base: composed C `f330bdab3abee8535213228e5465b085b65be750`, tree `5576d25210259cada7b4b00c484c33693369f228`. File: `tests/integration/mod_delivery.rs`; line anchors below are from the current read-only snapshot. Scope: `activation_v2_watch_full_pending_capacity_parks_until_real_channel_input` only.

The original b046 full-union failure remains unexplained: the real Watch did not emit `sent[255]` after the terminal reply was released. The isolated diagnostic pass and the parent's comparable 391-test diagnostic pass did not capture a failure. This experiment can establish a concrete fixture lifecycle defect and validate its removal. It cannot establish that this defect caused the historical failure without matching historical runtime evidence.

## Mechanism under test

The real Watch submits DeliveryPullV2 with a 5000 ms whole-call budget (`src/cli/mod.rs:3050`, `src/cli/watch.rs:619`). LocalClient applies the remaining budget while awaiting reply bytes (`src/client/local.rs:183–205`). The fixture observes a completed daemon reply but holds its downstream bytes in `CapacityFlow::hold_reply`. After observing the terminal false/Unchanged reply, it runs a separate completion barrier of up to STEP (20 seconds) before releasing those bytes. The client deadline remains active during that barrier. Watch closes with an error if that call fails (`src/cli/watch.rs:703`). Frozen UTC does not freeze this deadline.

The controlled variable is a single real six-second delay immediately after observing the terminal reply and verifying the exact 256 offered IDs. It is strictly a temporary experiment, never a final test requirement. No RPC budget, STEP, production clock, page result, attention frame or assertion changes.

## RED scratch variant

Apply this insertion to the frozen diagnostic base. Retain all current code and diagnostics after it:

```diff
@@
     assert_eq!(offered, sent[..256].iter().cloned().collect());
+    // TEMPORARY controlled experiment: an observed terminal RPC is still held.
+    std::thread::sleep(Duration::from_secs(6));
     // Keep the terminal response held and Attention credits at zero while
```

The insertion is after current line 2718, before the existing paced canonical work barrier. The measurement `terminal_held_at` already starts at terminal observation (line 2708), so `hold_ms` includes the imposed delay. The daemon and child continue running during the sleep. Parent executes the exact case once with the existing failure-only retained diagnostic and scoped leak check. Do not rerun until a desired result appears.

Accept a supported RED only when the actual output diagnostic identifies the timeout mechanism: held duration greater than 5000 ms; terminal `target_item=true` tying the missing `sent[255]` to the held reply; missing target output; and a closed/error child or equivalent closing/error status. The expected failure site is the unchanged `wait_item(&sent[255])`. A failed terminal write strengthens the evidence; a successful write does not disprove expiry because a socket write is not proof of client consumption. Preserve the exact observed write result.

If the terminal contains no target, the child stays live, or the run fails first at a different stage, this planned RED has not established the requested causal chain. Stop and interpret the actual metadata; do not add delays or weaken assertions. A target absent from terminal may still permit proving RPC expiry, but does not reproduce the missing-target symptom by the proposed direct path.

## GREEN scratch variant, conditional on supported RED

Keep the same six-second delay, same fixture data, same STEP, same client and daemon. Change only lifecycle ordering: release the terminal response and finish the existing observed `wait_item(&sent[255])` before the delay and before the paced canonical work barrier. Attention credits remain zero until the original explicit `release_attention_after(terminal)`.

Exact block-movement recipe against the diagnostic base (use source anchors, not shifted line numbers):

1. Immediately after `assert_eq!(offered, sent[..256].iter().cloned().collect());`, move the intact block beginning `let terminal_exchange = relay.snapshot()[terminal_index].clone();` (current 2821) through the end of the `if let Err(failure) = output_wait` block (current 2889). This contains terminal metadata, `held_before_release`, `flow.release_reply(terminal)`, bounded output diagnostics and the original output assertion. It must appear exactly once.
2. Place the same temporary `std::thread::sleep(Duration::from_secs(6));` immediately after that moved output block. Thus the imposed delay now occurs after actual target stdout observation, while Attention is still blocked.
3. Leave the canonical completion barrier and its assertions after the delay. Leave `diagnostic_before`, `capacity_normal_snapshot`, carry-ID257/body assertions and receipt/noACK checks after this barrier. The baseline must continue to represent completed canonical work, before forwarding Attention.
4. Since the moved output diagnostic precedes the barrier, remove its `barrier_ms={}` field and `barrier_elapsed.as_millis()` argument; replace the label with literal `phase=terminal_output_before_barrier`. Remove now-unused `barrier_started_at` and `barrier_elapsed` bindings. Do not invent a zero barrier duration. All terminal, child, output and relay evidence remains available.
5. Rewrite the barrier comment and wait description to say that the terminal reply has been delivered, Attention remains blocked, and canonical jobs finish before forwarding the real queued frame. Update the snapshot comment to reflect that snapshots are before Attention, rather than claiming the initial response is still held.

The insertion location in GREEN is therefore:

```rust
assert_eq!(offered, sent[..256].iter().cloned().collect());
// Existing terminal metadata, release and observed-output block goes here.
// TEMPORARY controlled experiment: terminal output is now observed.
std::thread::sleep(Duration::from_secs(6));
// Existing paced canonical work barrier, snapshots and carry assertions follow.
```

Parent executes this exact case once and retains the result and scoped leak check. A GREEN requires the entire original test to pass, including a real queued daemon Attention, exactly one forwarded stimulus preceding the resumed Pull, quiet resumed reply, exact unchanged normal carry/cursor tuple, pending receipts and no model ACK. An isolated pass at only the former output stage is insufficient.

The two variants deliberately place an identical bounded stall on opposite sides of a real RPC completion. They do not simulate a daemon response or solve expiry by raising a deadline. If RED meets its evidence criteria and GREEN passes all oracles, the supported conclusion is that withholding the terminal RPC across unrelated setup work is a demonstrated fixture hazard, and completing the RPC first removes that hazard under the same stall. The historical b046 failure remains an unconfirmed instance.

## Why this preserves the original oracle

Both terminal Page.more=false and Unchanged send the real Watch into stream waiting after output/any required confirmation (`src/cli/watch.rs:703–710`). The barrier's ordinary lane kicks create no client Pull themselves. Attention forwarding remains blocked during the entire delay and barrier. `CapacityFlow::release_attention_after` checks `requests == terminal` and zero forwarded frames; `forward_attention` rechecks the request count under its mutex before writing an actual frame; `assert_resumed_after_attention` checks the order of actual successful frame output and the next request. Any accidental extra Pull during the new interval still fails these existing assertions.

Do not infer that target stdout alone proves every optional Confirm has finished. The terminal metadata records the token, and the final request/frame order and whole-case result remain required. No ownership, receipt, presentation-authority or production behavior changes are proposed.

## Final candidate boundary

Only after the controlled comparison is assessed, remove the temporary six-second sleep and its experiment comment. Keep the ordering correction if accepted as a mechanism-supported fixture safety fix. No injected delay remains in the final suite. Parent performs the ordinary exact-case and original-pressure verification on that undelayed candidate and all required checks. Retain the original failure and both controlled outcomes; report the evidence boundary explicitly rather than claiming a proven explanation of b046.
