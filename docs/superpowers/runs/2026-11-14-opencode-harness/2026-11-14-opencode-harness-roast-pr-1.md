super-roast verdict: Should-fix (8 confirmed)
mode: PR        iteration: 1 of 3
profile (assumed): herdr-threads is a locally installed Rust CLI and daemon that carries durable message threads between coding agents; this PR adds an opencode harness (a Rust adapter plus one dependency-free JS plugin). The trust model is cooperative and same-user, with no network-exposed surface and no customer data or money at stake. It ships to other users through tagged releases and an installer, and rollback is a reinstall. The Delivery contract and TRUST-POLICY are normative, so wake, ACK and sender-warning correctness weigh more than resilience, cost or doc hygiene.
inputs: opencode-harness@1219ce5f9 vs main@bd7492d79
coverage: scouts 15/15 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, deploy-safety, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, dependency, hygiene-docs) · raw 88 → deduped 75 → panel 28 · spot 46 · promoted 1 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "git diff main...HEAD in the repo worktree at the captured head; all commits local"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 29 · rr 0.83 · rg 0.72 · fg 0.69 · unanimous 0.62 · ground-loo 0.75 (n=24) · reproduce 7/19/3 · refute 7/20/2 · ground 14/12/3
lane-yield (found/confirmed/unique/refuted): correctness 6/3/1/2 · security 5/0/0/5 · premortem 8/0/0/5 · simplicity-design 8/0/0/8 · hot-path-perf 5/0/0/4 · concurrency-async 7/1/0/4 · deploy-safety 5/0/0/5 · api-contract 5/0/0/5 · observability 8/2/0/5 · testing 11/3/3/4 · test-causality-lifecycle 3/0/0/3 · test-isolation-environment 3/0/0/3 · test-suite-cost 2/0/0/2 · dependency 2/1/1/1 · hygiene-docs 10/1/1/9

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] integrations/opencode/plugin/herdr-threads.js:820; integrations/opencode/plugin/herdr-threads.js:822 — When opencode lacks `session.promptAsync` the plugin degrades the wake but keeps its watch channel live, so the daemon still classifies the seat as channel-woken: p0 sends, ACK requests and invitations never wake it and the sender never gets `wake=no_wake`, contrary to the design and the doctor text. [lanes: correctness, observability]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `maybeWake` (herdr-threads.js:820-824) runs `degrade('wake'); S.wakePending = null; return`.
  `degrade()` (246-253) only writes a ledger line and the heartbeat `degraded` field; `goChannelless` (745), which stops the child, is called only for `append` and `append_row` (786-787).
  The watch child is spawned as `watch --harness opencode --session <sid>` (477) with no capability flag; `OpencodeDelivery::idle_wake` returns `IdleWake::Channel` unconditionally.
  Design line 148: "a missing `promptAsync` loses the wake only (`wake=no_wake`)"; src/harness/opencode/doctor.rs:197 renders "no idle wake (wake=no_wake)".
  Delivery contract rule 5 (an ACK request wakes, or the sender is told there is no wake) is not met in this degraded case.
  Limits: the reproduce and ground seats took the daemon-side `classify_wake_rows` path from the finding and did not trace it themselves. The case needs an opencode build without `promptAsync`.
  severity: seats voted Should-fix, Nit, Should-fix.
  fix-shape hint: on a missing `promptAsync`, make the daemon see no wake path (stop the channel as `goChannelless` does, or report the lost capability at watch registration); otherwise correct design line 148 and doctor.rs:197 to say the sender gets no warning.
- [Should-fix] docs/superpowers/runs/2026-11-14-opencode-harness/2026-11-14-opencode-harness-design.md:114 — The design calls opencode's startup dependency install irrelevant because the plugin has no npm dependency, but the branch's own research says plugin loading waits for a network install of `@opencode-ai/plugin` once any plugin is configured, and offline behaviour was never confirmed. [lanes: dependency]
  verdict: confirmed (reproduce ✗ (seat voted REJECT, outvoted 2-1) / refute ✗-survived / ground ✓)
  evidence: Design line 114: "No npm dependency, so opencode's dependency install is irrelevant to it."
  research-opencode-surface.md:219: "plugin loading waits for it when plugins are configured (`config.waitForDependencies()`). Offline behaviour UNCONFIRMED."
  research-opencode-surface.md:234 lists "Offline startup with plugins configured" as an open question.
  native-gate.md:21 and :50: without the mock registry the install "retries for about 70 s (observed)" and blocks the instance bootstrap; tests/opencode_native.rs:18-21 and :514 point `NPM_CONFIG_REGISTRY` at a 404 mock to avoid it.
  src/harness/opencode/setup.rs:49 writes `plugins/herdr-threads.js`, which is the state that turns the install into a wait.
  Dissent (reproduce seat): line 114 is true as written, the wait is opencode behaviour for any plugin, and the design states no startup-latency requirement. The confirming seats address this: the wait is triggered by a plugin being configured, which setup does, and no design row or doc bounds or mentions it.
  Limits: the 70 s figure was observed against a refused registry; the delay for a real offline user is unmeasured.
  fix-shape hint: run the finding's spike (offline start with only the plugin configured, time to first hook), then correct line 114 and record the result or a mitigation in integrations/opencode/README.md.
- [Nit] src/harness/opencode/setup.rs:470 — When setup updates a file that holds the manifest's previous pending digest, it records the old `digest` and not the file's current digest; if that write then fails, the file matches neither recorded digest and later setup/unsetup reports it as edited or not owned. [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `Entry::owns` accepts bytes equal to `entry.pending`; the action is `Updated`, and `held = entry.digest.clone()` (D0) is written with `pending = T2` before `write_replacement`, while the file still holds P1.
  A failed or interrupted write leaves P1, which `owns` rejects on the next install ("was edited or is not owned"), and `remove` leaves it as residue.
  This contradicts the code's own comment "a crash mid-write leaves files the manifest still recognizes".
  It needs two interrupted setups in a row with different render targets, and is recovered by moving the file aside.
  fix-shape hint: record the fingerprint of the current file bytes as `held`.
- [Nit] integrations/opencode/plugin/herdr-threads.js:1190; integrations/opencode/plugin/herdr-threads.js:1107; src/harness/opencode/doctor.rs:205 — The plugin heartbeat is one file per state dir, written by every opencode process from its own in-memory base with no read-merge, and doctor checks no pid liveness, so the last writer replaces another process's degraded/denied record and a dead process's record still reads as loaded. [lanes: correctness, concurrency-async, observability]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: The path is `<stateDir>/opencode/plugin-heartbeat.json` (HEARTBEAT_FILE, l.40, l.1147), with no pid or session in it.
  Each `makePlugin` calls `setHeartbeatBase` with a fresh base and writes `loaded_at` by tmp+rename (l.1107-1126, 1190-1200).
  The inert guard is per process, so two opencode panes on one state dir are both `active` and overwrite each other.
  doctor.rs `heartbeat_json` (205-240) marks `stale` only from `loaded_at` against the plugin file mtime and copies `pid` unchecked.
  The effect is limited to diagnostics; the "installed, never loaded" proof (design line 121) still holds.
  fix-shape hint: write one heartbeat file per pid, or read-merge on write and check pid liveness in doctor.
- [Nit] integrations/opencode/plugin/herdr-threads.js:690 — The plugin's ACK error branches have no test: every test's fake `ackReply` returns `settled` with exit 0, so the retryable retention, the 30 s `ACK_RETRY_MS` re-ack, the exit-2 `rowRefused` fallback that re-acks without `--row`, and the `refused_terminal`/`stale_generation` handling never run. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: plugin.test.mjs:66-69 defines `ackReply` once and nothing overrides it; the file has no `retryable`, `row_refused`, `stale_generation`, `refused_terminal` or `code: 2`.
  The two 30 000 ms advances (lines 449, 567) test append and wake backoff, not the ACK retry.
  In `doAck` (about 673-718) the untested branches include the re-ack without `--row` (receipt provenance) and `delete rec.delivered[id]` on `stale_generation` (re-append path).
  severity: seats voted Nit, Nit, Should-fix; it is a coverage gap, not a demonstrated defect.
  fix-shape hint: override `h.ackReply` per test to return exit 2, a non-zero exit, a missing id, `refused_terminal` and `stale_generation`, and assert the resulting argv and ledger state.
- [Nit] integrations/opencode/plugin/herdr-threads.js:520 — No plugin test emits an `attention` or `attention_cleared` watch line, so attention queueing, the per-run `runAttention` dedup, the `presentation_epoch` canonical check and the retraction of queued attention items are untested. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `grep -i attention` over integrations/opencode/plugin/tests/, tests/opencode_plugin_js.rs and tests/opencode_native.rs finds nothing.
  The untested code is the epoch filter (herdr-threads.js:529), the `attention_cleared` retract (531-538), the `runAttention` dedup (543-546) and `attentionVersions` (623-627).
  Design line 96 puts "attention lines, presentation_epoch" in the plugin's scope, and attention items are actionable warnings under the Delivery contract.
  severity: seats voted Nit, Should-fix, Nit (promoted from a spot check).
  fix-shape hint: add tests that send `attention` and `attention_cleared` lines with canonical and non-canonical epochs and assert what is appended, deduped and retracted.
- [Nit] tests/harness/opencode_setup.rs:688 — The doctor test pins the `append_row` degraded text as "appends are acked without the stored message id", but the plugin handles that degradation by going channelless with no acks, and the plugin's own test asserts zero acks. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/harness/opencode/doctor.rs:196 maps `"append_row"` to "appends are acked without the stored message id"; tests/harness/opencode_setup.rs:687 pins that string.
  herdr-threads.js:787 does `if (r.noId) return goChannelless('append_row')`, which stops the channel before any ack.
  plugin.test.mjs:646-657 asserts `h.acks().length === 0` and `channelless === true`.
  Design line 94 requires the stored id for an ack, so the operator-facing text states the opposite of the behaviour.
  fix-shape hint: reword the doctor effect to match `append` (channel stopped, turn hook delivers, no idle wake) and update the pinned test string.
- [Nit] AGENTS.md:66 — Delivery contract rule 5 and the matching "What we do today" no_wake bullet still name only "a named seat without a live mod channel" as the no-wake case, though this PR adds an opencode seat in a Herdr pane whose plugin channel is down. [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗ (seat voted REJECT, outvoted 2-1) / ground ✓)
  evidence: AGENTS.md:66 reads "except on a host with no wake (a named seat without a live mod channel)"; AGENTS.md:105 reads "(a named seat, no live unstalled mod channel)". The diff changes only rule 6's Channel example.
  src/harness/registry.rs:1780 has `("opencode", [NoWake, Channel, NoWake, Channel])`; TRUST-POLICY.md was amended for the same case.
  Dissent (refute seat): the parenthetical is an example, and hc terminal seats were already outside it on base. The ground seat concedes both points and still finds the new case missing; no behaviour is affected.
  fix-shape hint: widen the two parentheticals to name a channel-only harness whose channel is down (or drop the example list).

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- integrations/opencode/plugin/herdr-threads.js:798 — ACK-request receipts settle on a successful append, before any model turn. Rejected 3-0: design line 94 specifies the ack after the append returns the stored message id ("the host evidence of entry"), the same class as the Claude mod; a failed wake keeps `wakePending` and retries after a 30 s backoff.
- integrations/opencode/plugin/herdr-threads.js:847 — The plugin's idle wake is not closed-loop. Rejected 3-0: design lines 90 and 184 state "the append is the delivery, the wake only guarantees it a turn"; the AGENTS.md closed-loop clause covers the native composer wake, which opencode does not use; a refused `promptAsync` is retried.
- integrations/opencode/plugin/herdr-threads.js:783 — An append that succeeds after a channel generation change is dropped without `complete()`. Rejected 3-0: not acking is the conservative at-least-once path (design line 94), the re-offer reaches the newly bound session, and completing under the old generation would be the worse outcome.
- integrations/opencode/plugin/herdr-threads.js:787 — One `noId` append response puts the plugin into channelless mode for the process's life. Rejected 3-0: design D8 and the `goChannelless` comment make this the intended degrade; transient failures take the `error` branch with backoff; without a stored id nothing can be acked.
- integrations/opencode/plugin/herdr-threads.js:152 — The `herdr-threads *` allow rule may auto-approve compound or substituted commands. Rejected 3-0: opencode's permissions docs (fetched) say rules apply to every extracted subcommand and unanalyzable commands return a scanner error; research-opencode-surface.md:176 records the same as confirmed. The `$(...)` case was not tested live.
- src/harness/opencode/setup.rs:72 — `setup opencode --with-permissions` has no agent-evidence guard. Rejected 3-0: the consent-gated grant is an accepted limit (design line 185), no harness's setup path has such a guard, opencode's own bash `ask` is the consent gate, and TRUST-POLICY forbids adversarial caller verification.
- integrations/opencode/plugin/herdr-threads.js:164 — The consented grant replaces an explicit `"herdr-threads *": "ask"` rule with `allow`. Rejected 3-0: design line 119 names an exact-key `deny` as the only honoured override; plugin.test.mjs:719 pins the behaviour.
- integrations/opencode/plugin/herdr-threads.js:41 — The `ht *` grant auto-approves an unrelated `ht` binary. Rejected 2-1: docs/install.md:25 says "`ht` is installed by default and runs the same CLI", which disproves the premise that no `ht` executable ships; design line 119 specifies the key. The one CONFIRM (Nit) marked it pre-existing on base: src/harness/claude/permissions.rs:374.
- integrations/opencode/plugin/herdr-threads.js:330 — Every hook is serialized through one `S.hookChain`, each allowed 8 s. Rejected 3-0: design line 83 requires only fail-open ("the plugin injects nothing and the turn proceeds"); the stacking needs a hung daemon plus an unverified assumption about opencode's hook concurrency.
- integrations/opencode/plugin/herdr-threads.js:378 — The bound/lifecycle decision in `onChatMessage` is read before the hook waits on the chain. Rejected 2-1: the effect is at most a duplicate SessionStart check-in or a wrong `startup`/`clear` label, with no delivery or wake rule broken. The dissenting CONFIRM (Nit) cites the same lines and concedes the low impact.
- src/harness/registry.rs:1528 — `wake=no_wake` now also reports a transient channel-down state. Rejected 3-0: design lines 143, 146, 148 and 157 specify `wake=no_wake` for an opencode seat with no live channel; the status is accurate at read time.
- integrations/opencode/plugin/herdr-threads.js:115 — The recovery line uses the absolute executable and does not match the `herdr-threads *` allow rule. Rejected 2-1: the line mirrors the Claude mod on base (register.js:90) and design line 114; the cost is one permission prompt on the truncation path. The dissenting CONFIRM (Nit) grounded only the pattern mismatch, which the majority already granted.
- integrations/opencode/plugin/herdr-threads.js:340 — The plugin detects a failed check-in by substring-matching core's prose. Rejected 2-1: plugin.test.mjs:746 reads src/cli/hook.rs and asserts the marker, and core tests and golden fixtures pin the same string; this answers the dissenting seat's claim that no test pairs the two.
- integrations/opencode/plugin/herdr-threads.js:1060; integrations/opencode/plugin/herdr-threads.js:1017; integrations/opencode/plugin/herdr-threads.js:329 — Plugin failures leave no trace in production. Rejected 2-1: design lines 121 and 148 scope the heartbeat to load facts and degraded capabilities; the env-gated ledger is the Claude mod's convention; a dead channel surfaces as `wake=no_wake`. The dissenting CONFIRM (Nit) agrees the spec sets no production-logging requirement.
- src/scheduler/mod.rs:735 — The channel-only exclusion is all-or-nothing, not per target. Rejected 3-0: the `no_native_wake` doc comment states the rule as intended, a target without a composer policy is refused safely by the host, and the finding traced no wrong outcome.
- tests/opencode_plugin_js.rs:17; tests/opencode_plugin_js.rs:16; tests/opencode_plugin_js.rs:15 — The plugin test passes when node is not on PATH. Rejected 3-0: the skip is stated in the file header and follows repo precedent (tests/canary_manifest_writer.rs:54, :95 for python3); the CI-lacks-node premise was not shown.
- integrations/opencode/plugin/herdr-threads.js:730 — Peer bodies are stored as user-role messages. Rejected 3-0: design lines 25 and 87 choose the `noReply` append with `synthetic: true`; AGENTS.md (e) keeps only the invariant that peer messages never override the user, carried by the framing and the skill.

## Unverified nits (spot-checked)
- integrations/opencode/plugin/herdr-threads.js:794 — An item that alone exceeds the 64 KiB budget is offered only as a recovery line and stays in `S.queue` for the run. (spot: REJECT; same pattern as the Claude mod, the inbox recovery delivers it)
- integrations/opencode/plugin/herdr-threads.js:497 — A watch child that stops with a stop reason parks the channel and nothing in the heartbeat or doctor shows it. (spot: REJECT; design line 96 specifies the stop, and a later check-in clears it)
- integrations/opencode/plugin/herdr-threads.js:395 — `onCompacted`'s append bypasses the `appendInflight` guard. (spot: REJECT; no ordering requirement between the re-entry text and a delivery page)
- integrations/opencode/plugin/herdr-threads.js:489 — After a rollback to a pre-opencode binary the plugin respawns `watch --harness opencode` every 30 s. (spot: REJECT; one failing spawn per 30 s, no state harmed)
- integrations/opencode/plugin/herdr-threads.js:601 — Every watch line calls `retryAcks()` unthrottled; only the tick path honours `ACK_RETRY_MS`. (spot: CONFIRM Nit; only on a persistent ack failure, copied from the Claude mod)
- src/harness/registry.rs:1644 — The new `opencode` harness id reaches an older daemon with no capability negotiation. (spot: REJECT; same pattern as Hermes and agy on base, the refusal is loud)
- src/scheduler/mod.rs:716 — `no_native_wake` records no attempt for a channel-down opencode seat. (spot: REJECT; design D1/D8, nothing is retried so there is no ladder to observe)
- src/harness/opencode/decode.rs:103 — The hook decode refuses unknown `event` or `source` values. (spot: REJECT; plugin and binary ship together, fail-closed matches design line 183)
- src/harness/opencode/setup.rs:489 — A rolled-back binary cannot remove the opencode integration. (spot: REJECT; docs/install.md:239 already says to run `unsetup <harness>` with the newer build first)
- integrations/opencode/plugin/herdr-threads.js:103 — The `appendBatch` page bounds (32 items, 64 KiB, `deferred`, `recoveryOffered`) are untested. (spot: CONFIRM Nit; bodies are capped at 8 KiB upstream)
- scripts/wake-e2e-herdr:39 — `CALLER_ENV` does not strip `OPENCODE_PID` while validate-package.sh does. (spot: REJECT; opencode overwrites `OPENCODE_PID` at startup per research-opencode-surface.md line 35)
- scripts/wake-e2e.py:1918 — The wake e2e counts ledger `submit` entries, written before `promptAsync`. (spot: REJECT; `woke` is checked separately, a submit in a no-wake case should fail)
- scripts/wake-e2e.py:2092 — `opencode_ready` falls through its timeout and then sleeps a fixed 5 s. (spot: REJECT; `start()` raises if the standing prompt's report never shows)
- tests/setup_cli.rs:1564 — The opencode-absent expectations depend on no `opencode` in /usr/bin or /bin. (spot: REJECT; same shape as the agy expectations, not observed)
- integrations/opencode/plugin/herdr-threads.js:636 — `rec.delivered` and the per-session maps grow without bound. (spot: REJECT; copied from register.js:449, entries are small)
- tests/setup_cli.rs:1561 — Bare `setup`/`unsetup`/`setup-status` now cover opencode and the CHANGELOG does not say so. (spot: REJECT; same as the agy precedent, a docs suggestion)
- src/harness/registry.rs:1499 — `has_idle_wake` still returns `Some(true)` for opencode and its doc calls it the oracle. (spot: REJECT; the return value is correct for its stated question, one stale doc sentence)
- src/scheduler/mod.rs:1214 — The `no_native_wake` check inside `WakeRunner::try_poke` has no test that fails without it. (spot: CONFIRM Nit; `drive_pokes` filters first)
- integrations/opencode/plugin/tests/plugin.test.mjs:27 — Tests settle async work with a fixed five-round `setImmediate` flush. (spot: REJECT; the fakes are microtask-only, one round drains the chain)
- tests/opencode_native.rs:744 — The native gate checks "a p1 append starts no turn" with a fixed 5 s sleep. (spot: REJECT; later context-index and idle checks would catch a late wake)
- integrations/opencode/plugin/herdr-threads.js:575 — Invitation dedup by context version, the named-seat `no_pane` exemption and the `CONNECTED_RESET_MS` reset have no plugin test. (spot: CONFIRM Nit)
- integrations/opencode/plugin/herdr-threads.js:23 — The plugin carries no version or contract identifier. (spot: REJECT; versionless by design D6/D8, doctor reports a stale plugin)
- src/scheduler/mod.rs:302 — The native wake/poke exclusion has no switch to fall back to a composer wake. (spot: REJECT; design lines 47 and 214 exclude a typed fallback from this epic)
- integrations/opencode/plugin/herdr-threads.js:934 — `shell.env` repeats the `session.get` walk when the parent lookup fails. (spot: REJECT; not caching a failure lets the lookup recover)
- src/harness/opencode/launch.rs:210 — Config-directory discovery is encoded separately in setup.rs, launch.rs and doctor.rs. (spot: REJECT; agy and hermes also write their own `resolve_scope`)
- src/harness/opencode/setup.rs:270 — setup.rs copies about 250 lines of agy/plugin.rs manifest machinery. (spot: REJECT; design D6 says "as the agy plugin directory is")
- integrations/opencode/plugin/herdr-threads.js:43 — The plugin re-implements the Claude mod's watch-consumer state machine. (spot: REJECT; design line 96 documents the third copy and its parity golden)
- docs/superpowers/runs/2026-11-14-opencode-harness/wake-e2e/report.md:1 — The committed wake e2e evidence came from a dirty tree at 8eb398aab and does not cover the head. (spot: REJECT; the opencode run is opt-in, not a release gate)
- src/scheduler/mod.rs:716 — `no_native_wake` has no AGENTS.md "What we do today" entry and no delivery-contract audit entry. (spot: REJECT; TRUST-POLICY's Wake paragraph documents it)
- docs/superpowers/runs/2026-11-14-opencode-harness/2026-11-14-opencode-harness-design.md:161 — Design D10's "zero behaviour change in core" was not updated for the scheduler and health changes, and `docs/compatibility/harnesses.md` was not touched. (spot: REJECT; design line 169 anticipates a neutral core change)
- AGENTS.md:137 — The wake e2e Isolation paragraph says nothing about opencode recipients or the changed credential handoff. (spot: REJECT; line 126 names the credential and option)
- AGENTS.md:10 — The never-write list omits opencode's data directory `~/.local/share/opencode`. (spot: REJECT; nothing in the diff writes there, the scratch-XDG isolation is the guard; the default data path was not confirmed externally)
- AGENTS.md:6 — The `Outside-adapter` trailer rule still names `src/harness/agy/`. (spot: REJECT; the trailer only annotates the diff)
- tests/opencode_native.rs:573; tests/opencode_native.rs:574 — Outside its network namespace the native gate picks the serve port by bind-and-drop. (spot: REJECT; only the no-netns fallback, a false failure at worst)
- src/scheduler/mod.rs:722 — `no_native_wake` calls the process-global `builtins()` instead of an injected source. (spot: REJECT; the same call is used across core store code)
- integrations/opencode/plugin/herdr-threads.js:1103 — With `HERDR_THREADS_MOD_LEDGER` set, the ledger grows without a cap. (spot: REJECT; opt-in debug file, the Claude cap is a by-product of its rewrite design)
- integrations/opencode/plugin/herdr-threads.js:127 — `appendBatch` re-frames the whole batch for each candidate item. (spot: REJECT; bounded at 32 items and 64 KiB, once per append)
- Cargo.toml:92 — The opt-in native gate gets its own `[[test]]` binary. (spot: REJECT; it re-executes itself under `unshare -rn`, link time not measured)
- .config/nextest.toml:233 — The native gate is put in the `named-seats` nextest group. (spot: REJECT; the group's rationale is load, the cost appears only in opted-in runs)
- src/harness/registry.rs:1528 — The `IdleWake::Channel` predicate is spelled out in three Registry methods. (spot: REJECT; a one-line predicate that already had two copies on base)
- tests/integration/session_claims.rs:434 — The rewritten refusal test asserts `text.contains("opencode")`, which both branches satisfy. (spot: REJECT; the same assert also checks the discriminating marker text)
- integrations/opencode/plugin/herdr-threads.js:185 — `createCore` is one roughly 800-line closure over about 40 state fields. (spot: REJECT; design line 9 asks for one file, register.js is the precedent)
- src/scheduler/mod.rs:716 — The PR is one 223-file unit that also changes core scheduling, health text and agy e2e credential handling. (spot: REJECT; packaging preference, no size rule in the repo)
- src/harness/opencode/setup.rs:155 — `Launch { argv }` and `launch()` are shallow pass-throughs. (spot: REJECT; the struct is the serialized shape the plugin embeds)
- integrations/opencode/plugin/tests/plugin.test.mjs:581 — The priority-order test has a dead `h.core.onEvent({ type: 'noop' })` under a comment that describes a hold it does not perform. (spot: CONFIRM Nit)
- Cargo.toml:92 — The diff adds no third-party Rust crate or npm package. (spot: REJECT; a negative result, nothing to fix)

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- integrations/opencode/plugin/herdr-threads.js:763 — `refreshStatus()` can overwrite the plugin's own `busy` mark with a server snapshot, so a wake prompt is sent into a session that is about to start a turn.
  reason: UNVERIFIED external. The reproduce and ground seats confirmed the plugin path but could not fetch opencode's session/prompt.ts (404) to confirm that `session.status()` reports idle during the `chat.message` hook.
  The refute seat rejected on materiality (research-opencode-surface.md l.126: a prompt sent while busy is queued, nothing is lost).
  check: the finding's spike (slow `chat.message` hook, deliver a `wakes=true` line, count `promptAsync` calls).
- integrations/opencode/plugin/herdr-threads.js:914 — `wakePending` is cleared by any assistant message whose id sorts after the append, even when that step read the message list before the append landed, so a waking item can get no wake.
  reason: UNVERIFIED external. The reproduce and refute seats could not fetch opencode source.
  The ground seat CONFIRMED at Should-fix from sst/opencode v1.2.0 prompt.ts (messages read at line 298, assistant message created at 561-564, model context built from the earlier snapshot at 623 and 660), but only against that old tag and not the version the plugin targets.
  check: read the targeted release's prompt loop order, or probe an append between a step's read and its assistant-message creation.
- integrations/opencode/plugin/herdr-threads.js:779; integrations/opencode/plugin/herdr-threads.js:730; integrations/opencode/plugin/herdr-threads.js:261 — No `client.session.*` SDK call has a timeout, so a hung append or wake wedges `appendInflight`/`wakeInflight` and a hung `session.get` blocks `chat.message` and `shell.env`.
  reason: UNVERIFIED external. The refute and ground seats confirmed the internal facts but could not establish whether opencode's in-process `prompt` (noReply), `promptAsync` or `session.get` can stall.
  The reproduce seat rejected (the design has no SDK-latency requirement; a wedged channel falls back to turn-hook delivery).
  check: the finding's spike (time an append during a long tool call and during `/compact`).
- integrations/opencode/plugin/herdr-threads.js:790 — A failed append is retried every 30 s although the append is not idempotent, so a store-then-error would store the page twice.
  reason: UNVERIFIED external. The reproduce and ground seats confirmed the retry path but could not determine whether `session.prompt` (noReply) can store the message and then return an error or throw.
  The refute seat rejected on design line 94 ("A failed append acks nothing and the item is offered again") and at-least-once delivery.
  check: read `createUserMessage`/`prompt()` in the targeted opencode release for anything that can throw after the message is stored.

## Escalations the reporter omitted (engine-added — escalate routes are final)
- integrations/opencode/plugin/herdr-threads.js:779; integrations/opencode/plugin/herdr-threads.js:730; integrations/opencode/plugin/herdr-threads.js:261 — No client.session.* SDK call (prompt, promptAsync, status, get) has a timeout: a hung append or wake leaves appendInflight/wakeInflight set and stops channel delivery and wakes with no watchdog, and a hung session.get blocks every chat.message and shell.env. (external-unverified)
