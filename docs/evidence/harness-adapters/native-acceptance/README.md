# Hermes native acceptance (2026-10-08)

Live runs of real Hermes against this branch, each in an isolated root: private
HOME/HERMES_HOME, a cloned Hermes checkout, an isolated herdr-threads state dir and
a private Herdr server. The model was deepseek-v4.1-flash via OpenRouter, with the
operator's existing credentials copied into the isolated home and deleted with it.

**Run 3 is the record of this verdict.** It ran entirely on the final build
(`9039a866`, binary `56af8de8…`, bridge `9ed5e59a…`; `run-3/identities.txt`) and
`run-3/` holds its raw outputs: setup/launch JSON, Herdr agent views, the injected
`api_content` of each measured turn, and store/Hermes snapshots at every stage
(`snap-*/`, each with its own `at` time). Local paths are redacted to `<root>`/`<home>`;
there are no credentials and no model output except the thread reply in
`12-thread-read.json`. Runs 1 and 2 found the two defects below; see "Earlier runs".

## Result (run 3)

Times are seconds after the step's own prompt or send, from the recorded files.

| Stage | Verdict | Evidence |
|---|---|---|
| plugin setup | PASS | `01-setup.json` installed; `03-owned-files.txt` lists `plugin.yaml`, `__init__.py`, `bridge_config.json` with the shipped digests |
| enablement | PASS | `02-enable.out` (native `hermes plugins enable`); `03-setup-status.json` configured_enabled true |
| guarded launch | PASS | `04-launch.json`: started, `startup_captured_prelaunch_observation`, binding gen 2 `managed_launch` |
| recognition | PASS | `07-agent-get-after-turn.json`: agent `hermes`, idle, `agent_session` = native session `20261008_133346_b76595` |
| callback context | PASS | `08-turn1-api-content.txt`: the first neutral turn's user message `api_content` (2484 bytes) carries the guide; journal Startup check-in binds gen 3 `cooperative_top_level` to that session (`snap-after-turn1`). Hermes injects `pre_llm_call` context into the user message by contract; it does not persist the system prompt, so that half is cited, not measured |
| context-only turn | PASS | invitation and ACK-required message sent (`09`, `10`); the delayed wake is a reservation row with deadline +180 s (`snap-after-send/wake_batches.txt`). Neutral prompt "What is the capital of France? One line." (`11`): its `api_content` (`13`) carries `attention digest: invitations=1`. The model's text inbox ACKed at +2.0 s, it accepted at +19.5 s and replied at +22.1 s; the reservation row was removed when attention cleared (present in `snap-neutral-3`, gone in `snap-neutral-4`), long before its deadline, so no wake was ever due |
| model accept | PASS | `snap-neutral-10/invitations.txt`: accepted by the seat at gen 3, observation provenance `cooperative_top_level`, native session `…_b76595` |
| model inbox | PASS | receipt `action_provenance` `cooperative_inbox_display` (text inbox) |
| model ACK | PASS | `snap-neutral-10/receipts.txt`: acked, gen 3, `cooperative_top_level` |
| model reply | PASS | `12-thread-read.json`: the seat's ordinary message in the thread |
| relaunch | PASS | `14-relaunch.json`: launch left the open gen-3 binding unchanged; the new process's Startup check-in ended gen 3 and bound gen 4 to the new session `…_3e1d1f` (`snap-after-relaunch-launch` vs `-turn`) |
| child | PASS | `delegate_task` child session `…_7e4146` (source subagent, parent `…_3e1d1f`): its user `api_content` (`16`) is the goal plus the exact subagent restriction; no binding, receipt or membership changed across the child (`snap-before-child` vs `snap-after-child`, apart from the parent's own `registered_at` refresh) |
| reset | PASS | `/new`: declared reset consumed at gen 4, journal `Clear`, gen 4 ended and gen 5 bound to `…_06775f` on the same seat; its first turn carries context (3224 bytes) (`snap-after-reset`) |
| bare start | LIMIT | `hermes` started in a new pane without `launch`: `17-agent-get-bare.json` `agent_not_found`, `17-agent-list-bare.json` lists only the launched pane; session `…_d83e17` has no `api_content`; no seat or binding created (`snap-before-bare` vs `snap-after-bare`). TRUST-POLICY accepted limit |
| cleanup | PASS | `18-teardown.txt`: no process references the root after teardown, the operator's real Hermes launchers match their pre-run digests, root deleted |

Setup and launch report `native_acceptance: "unmet"`: that field is the tool's
own startup-qualification status and does not consume this evidence.

## Earlier runs

- Run 1 (build `82ba030c`) passed setup through reset with the model's first turn
  started by the wake prompt; it found the probe-cap defect. Its raw outputs were
  deleted with its root; only this summary remains.
- Run 2 started on the pre-fix build (`258868fb…`, bridge `b47623b7…`) through the
  first child attempt, which found the subagent defect, then upgraded in place
  (`09afd22a…`, bridge `9ed5e59a…`) for the child retry, reset and bare start.
  `run-2/` holds its store and Hermes extracts.
- An earlier rerun saw one managed session receive no context on its first turn;
  that root had first hit a launch refused for a mode-0644 settings file. Runs 2
  and 3 on fresh roots did not reproduce it. Unexplained.

## Honest limits

- One model, one profile, macOS only; three runs, one on the final build.
- The Herdr host is the private modified build with the guarded process-hint
  capability (`agent_start_process_hint_v1`), not a released Herdr.
- `api_content` is preserved for the measured turns, not for every turn.

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
