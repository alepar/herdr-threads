# Final review: cluster 7 (integrations-claude), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified with `git rev-parse --verify`. The caller checkout read head 1b0583074 and a clean status before this file was written; nothing in it was edited.

Files: `integrations/claude/mod/hooks/register.js`, `integrations/claude/mod/tests/frame_golden.test.ts`, `integrations/claude/mod/tests/named_watch_env.test.ts`.

## Requirement baseline trace

- B1 ("Continuity key: approved."): the design (§4.2 Client side) gives this cluster one duty, "The Claude mod forwards it to its watch child". Traced at head: launcher env, `session.start` in `register()` reads `$.env.get('HERDR_THREADS_CONTINUITY_KEY')`, sets it in `watchEnv`, `buildIo` passes `watchEnv` to `$.process.spawn` at register.js:1146. This is the production entry point, and it has no separate manual step. Delivered for this cluster's segment.
- Beyond the spawn, the path leaves the cluster: `caller_continuity_key`, `CallerEnv::for_site`, `watch_continuity`, and the daemon's judgment. That is the seam reviewer's to trace. For orientation only: `src/cli/watch.rs:1291` passes `CallerEnv::for_site(..)` into `watch_continuity`, and `tests/cli/hook.rs:3857` (`watch_claim_carries_the_key`) pins that the claim carries the key.
- B2, B3 and B4: no code in this cluster bears on them. The mod holds no retirement or membership logic, and Herdr is not touched.

## Analysis of the register.js change

Env overlay. The Task 7 review left open whether `process.spawn` overlays or replaces `env`. This matters more for a keyed pane seat than for a named one: a replace would drop `HERDR_PANE_ID` and `PATH`, the child would answer `no_pane`, and the mod would stop permanently, so a key would disable delivery. The engine's own API contract settles it for the installed build (2.1.318): `ProcessSpawnRequest` reads "the host's environment with `env` over it", and its `env` field reads "Variables set over the host process's own environment". Overlay is the documented behaviour, so the forward is safe. This rests on the contract and was not executed (see the last section's list).

Pane-seat classification. The new truthy branch makes `watchEnv` non-null for a keyed pane seat, so I checked every reader of `io.watchEnv`. There are two: the spawn at register.js:1146, and the `named` test at register.js:311, which reads only `HERDR_THREADS_SEAT_NAME` and `HC_TERMINAL_ID`. A key therefore does not turn a pane seat into a named one, and `no_pane` still stops the mod. The new test covers this.

Argv-only path. A session with no name, terminal or key still gets `watchEnv = null` and the bare `{ argv }` event. The existing argv-only tests still pass.

Absent and invalid values. An empty value is falsy and is not forwarded; `CallerEnv::from_lookup` also filters empty values, so inheriting one is harmless. An invalid non-empty value is forwarded and `ContinuityKey::parse` in the child treats it as absent with no diagnostic, as §4.2 requires. The mod does no parsing of its own, which keeps one owner for the syntax.

Scope. The mod forwards a key beside a seat name or terminal id too. Per §4.2 such a key is ignored; that decision is made outside the mod, so it is a seam matter.

Mod packaging. `register.js` is embedded and installed by content (`src/harness/claude_mod.rs`), and `plugin.json` stays at the fixed `0.0.0`, so no version bump is needed for the change to reach users.

Golden list. `sender-label` was already in `tests/fixtures/frame-goldens.ts` at base (added by 857cc19a1), while the cover list in `frame_golden.test.ts` lacked it. The edit corrects a stale list and loses no coverage. Inferred from the two files: the cover test would have failed at base; it was not run there.

## Evidence

- `scripts/test-claude-mod` semantics (isolated `CLAUDE_CONFIG_DIR`, `claude plugin validate --strict`, then `claude plugin test`) were run on a `git archive` export of the head in the session scratchpad with claude 2.1.318: 182 pass, 0 fail across 5 files, including both new tests and the `sender-label` golden. Validation has to pass before the tests run in that script.
- A probe test was added to that export to run a real `$.process.spawn` and read back an overlaid variable. It could not execute: inside `claude plugin test`, `$.process` is undefined with no hook answering it. The probe recorded nothing about overlay and was deleted with the export.
- The `plugin-authoring` skill was loaded only to read the engine's API types. No mod was written, so no enable prompt should have been raised by this review.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The overlay of `watchEnv` on a real spawn is not executed by any test: the mod tests intercept `process.spawn`, and `claude plugin test` offers no real `$.process` (probe attempted); the evidence is the engine's API contract for `ProcessSpawnRequest`, so a keyed pane seat's child keeping `HERDR_PANE_ID` should be confirmed in the real-Herdr qualification that design §6 places outside this run.
- A real Claude Code session launched with the key, carrying it through the mod's watch child to a claim on a live daemon, is not exercised in this cluster; `tests/clone_identity_sweep.rs` and `tests/cli/hook.rs` cover the CLI side, and the join is the seam reviewer's to trace.
- No mod-level test combines a key with a seat name or terminal id; the mod's forwarding there is the same one line, and the ignore rule of design §4.2 sits outside this cluster.

## Deferred OK
- The comment at `integrations/claude/mod/hooks/register.js:1103` still says "The named-seat variables" though `watchEnv` now also carries terminal and key variables; comment only, no behaviour.
- `integrations/claude/mod/README.md:25` lists the watch child's environment without the forwarded variables; the name and terminal variables were already absent from it, so the key follows an existing gap.
