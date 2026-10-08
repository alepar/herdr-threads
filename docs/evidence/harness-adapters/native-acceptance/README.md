# Hermes native acceptance (2026-10-08)

One live run of real Hermes against this branch, in an isolated root: private
HOME/HERMES_HOME, a cloned Hermes checkout, an isolated herdr-threads state dir and
a private Herdr server. The model was deepseek-v4.1-flash via OpenRouter, with the
operator's existing credentials copied into the isolated home. `manifest.json` holds
the binary identities and measured rows.

## Result

| Stage | Verdict | Observation |
|---|---|---|
| plugin setup | PASS | `setup hermes --profile default` installed the three owned files after the probe-limit fix below |
| enablement | PASS | `hermes plugins enable herdr-threads` (native command); `setup-status` reports configured_enabled=true |
| guarded launch | PASS | `launch --kind hermes`: outcome started, identity captured before launch, binding `managed_launch` gen 2 |
| recognition | PASS | private Herdr `agent get` reports agent `hermes`, idle, with the native session id |
| callback context | PASS | context journal: completed `Startup`, then `Tool` turns; native `state.db` user row `api_content` carries the injected guide, routing and seat (user message, not system prompt) |
| binding | PASS | gen 3 `cooperative_top_level`, native_session = real Hermes session id, registered and live |
| model accept | PASS | invitation accepted by the Hermes seat at gen 3, provenance `cooperative_top_level` |
| model inbox / read | PASS | model ran `inbox` and `read`, then replied in the thread |
| model ACK | PASS | explicit `ack` of the first message: receipt_state acked, gen 3, `cooperative_top_level`; second message ACKed by text inbox |
| reset | PASS | `/new` consumed as a declared reset: `Clear` lifecycle, gen 3 ended, gen 4 bound to the new native session, same seat |
| child | SKIPPED | this profile exposes no delegation toolset; child delivery unmeasured |

Teardown stopped Hermes, the daemon and the private Herdr server; no process
referencing the isolated root survived.

## Honest limits

- The first model turn was started by the daemon's wake prompt
  (`attention pending; run herdr-threads inbox`), which is the product's designed
  path. A turn driven by the injected context alone, with a prompt that never
  mentions threads, was not isolated: the wake raced the neutral prompt.
- One run, one model, one profile, macOS only.
- The Herdr host is the private modified build with the guarded process-hint
  capability, not a released Herdr.

## Defect found and fixed

`src/harness/hermes/runtime.rs` capped every Hermes probe at 2 s. Real Hermes
bootstrap takes about 2.0 s on a warm install, so setup failed with `Deadline`
against every real install. The cap is now `PROBE_LIMIT` (15 s), still inside the
caller's 30 s budget. Regression test `capture_tolerates_bootstrap_slower_than_two_seconds`
fails on the old cap and passes on the new one.

## Operational hazard (Hermes, not this repo)

Running Hermes with an isolated HERMES_HOME against a shared source checkout lets
Hermes' bootstrap re-mint the checkout's `.hermes/bin` launchers to point at the
isolated home's Python. An earlier session's temp root left the operator's real
launcher broken this way. This run used its own APFS clone of the checkout and
verified the real launchers unchanged before and after.
