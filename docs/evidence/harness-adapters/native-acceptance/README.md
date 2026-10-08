# Hermes native acceptance (2026-10-08)

Live runs of real Hermes against this branch, each in an isolated root: private
HOME/HERMES_HOME, a cloned Hermes checkout, an isolated herdr-threads state dir and
a private Herdr server. The model was deepseek-v4.1-flash via OpenRouter, with the
operator's existing credentials copied into the isolated home. `manifest.json` holds
the binary identities and measured rows. `run-2/` holds run 2's store and Hermes
`state.db` extracts (no credentials, no model output beyond the child's context).

## Result

Run 1 measured the cooperative flow; run 2 repeated it on a fresh root, isolated
context-only delivery, measured the child stage and the reset, and checked a bare
start outside `launch`.

| Stage | Verdict | Observation |
|---|---|---|
| plugin setup | PASS | `setup hermes --profile default` installed the three owned files after the probe-limit fix below |
| enablement | PASS | `hermes plugins enable herdr-threads` (native command); `setup-status` reports configured_enabled=true |
| guarded launch | PASS | `launch --kind hermes`: outcome started, identity captured before launch, binding `managed_launch` gen 2 |
| recognition | PASS | private Herdr `agent get` reports agent `hermes`, idle, with the native session id |
| callback context | PASS | context journal: completed `Startup`, then `Tool` turns; native `state.db` user row `api_content` carries the injected guide, routing and seat (user message, not system prompt) |
| context-only turn | PASS (run 2) | with an invitation and an ACK-required message pending and the daemon wake delayed 180 s, the neutral prompt "What is the capital of France? One line." led the model to accept, reply and ACK 9 s later; no wake batch was ever reserved |
| binding | PASS | gen 3 `cooperative_top_level`, native_session = real Hermes session id, registered and live |
| model accept | PASS | invitation accepted by the Hermes seat at gen 3, provenance `cooperative_top_level` |
| model inbox / read | PASS | model ran `inbox` and `read`, then replied in the thread |
| model ACK | PASS | run 1: explicit `ack`, gen 3; run 2: text inbox ACK, `action_provenance` `cooperative_inbox_display`, gen 3 |
| relaunch | PASS (run 2) | managed relaunch on the same pane ended the old binding and bound the next generation to the new native session |
| child | PASS (run 2, after fix) | `delegate_task` child session (`source` subagent, parent set) user row carries the subagent restriction (685-byte `api_content`); the child made no threads writes; the journal holds no child entry |
| reset | PASS | `/new` consumed as a declared reset: `Clear` lifecycle, the old generation ended, the next bound to the new native session, same seat, context delivered |
| bare start | LIMIT (run 2) | `hermes` started in a pane without `launch`: Herdr lists no agent for the pane and the session gets no context; see TRUST-POLICY accepted limits |

Each run's teardown stopped Hermes, the daemon and the private Herdr server; no
process referencing the isolated root survived, the root (with its credential
copies) was deleted and the operator's real Hermes launchers verified unchanged.

## Honest limits

- Two runs, one model, one profile, macOS only.
- The Herdr host is the private modified build with the guarded process-hint
  capability, not a released Herdr.
- Run 2's first attempt at the child stage found the defect below; the PASS row is
  the rerun after the fix, upgraded in place by `setup`.
- An earlier rerun saw one managed session receive no context on its first turn.
  Run 2 on a fresh root did not reproduce it; that rerun had first hit a launch
  refused for a mode-0644 settings file, which run 2 avoided. Unexplained.

## Defects found and fixed

`src/harness/hermes/runtime.rs` capped every Hermes probe at 2 s. Real Hermes
bootstrap takes about 2.0 s on a warm install, so setup failed with `Deadline`
against every real install. The cap is now `PROBE_LIMIT` (15 s), still inside the
caller's 30 s budget. Regression test `capture_tolerates_bootstrap_slower_than_two_seconds`
fails on the old cap and passes on the new one.

Hermes runs `delegate_task` children with `platform="subagent"`. The bridge
dropped every callback whose platform was not `cli`, so the subagent restriction
never reached a real child. The bridge now answers a parented `subagent`
`pre_llm_call` locally with the restriction and ignores every other subagent
callback. The child fixture now uses the native platform; the bridge test fails
on the old bridge and passes on the new one.

## Operational hazard (Hermes, not this repo)

Running Hermes with an isolated HERMES_HOME against a shared source checkout lets
Hermes' bootstrap re-mint the checkout's `.hermes/bin` launchers to point at the
isolated home's Python. An earlier session's temp root left the operator's real
launcher broken this way. These runs used their own APFS clone of the checkout and
verified the real launchers unchanged before and after.
