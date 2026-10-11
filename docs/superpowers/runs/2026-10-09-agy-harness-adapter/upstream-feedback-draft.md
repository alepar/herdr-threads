# 2026-10-09-agy-harness-adapter: frozen plan briefs never see bead amendments, so re-dispatched tasks act on stale requirements

Plugin: superpowers 6.4.2-alepar4.20. Run: 114 beads under one epic (39 leaves at settle, grown by roast/fix/user-directed tasks), 8 super-code invocations, 62 coordinator merges, 2 design-roast + 2 code-roast rounds, ~2 days (2026-10-09/10), autonomous after interactive root design.

## Defects

### 1. super-code freezes plan sections; bead-description amendments made after materialization never reach the implementer or triage
- **Evidence:** friction.md: "a bead description updated after plan materialization (HT_AGY_BIN path added to ht-poh.4.5) never reached the implementer … filed a not-real blocker twice (ht-cos, ht-5mi)". Ledger: "The task bead ht-poh.4.5 was updated at 12:01Z, before blocker ht-5mi was filed at 12:29Z"; the second RESOLVE escalated under the one-retry bound. A later bead (ht-poh.14) amended with two extra requirements before dispatch landed "review clean" without them; both resurfaced as the next final review's Important findings. coordinator-workflow.md §Plan materialization: "an already-assigned ordinal or section is never renumbered or rewritten"; implementer-prompt.md points the implementer only at the brief and `bd comments`; the triage prompt pastes the frozen `## Task <N>` section. Workaround: the orchestrator hand-patched plan sections (6 times).
- **Premise to verify:** bead `updated_at` (or a description hash) reliably signals a change after the section was written; freezing exists only to avoid disturbing in-flight tasks, and a (re-)dispatch is not in flight.
- **Suggested fix shape:** at dispatch/re-dispatch compare the current bead description with what was materialized and append a dated "Amendments (current bead description)" block to the brief — or instruct implementer and triage to `bd show <id>` and treat the current description as authoritative where it differs.

### 2. A merged task can be logged complete while its task and review beads stay open; the next invocation stops "stalled"
- **Evidence:** ledger: "Merge: ht-poh.6.6 — rebase clean · seam-review none · check pass" / "Task 34 (ht-poh.6.6): complete (commits 3c6dea0..b548b5d, review clean)" — the only one of 62 `Merge:` lines with no matching `Cleanup:` line; later edge audits still call ht-poh.6.6 "the one agent-ready leaf"; detector "1 ready · peak in-flight 1"; the invocation returned `stopReason: stalled`; the orchestrator closed both beads by hand ("merged as b548b5de"). coordinator.js:1765 tells the merge agent to append the ledger only after `bd close` succeeded.
- **Premise to verify:** the coordinator counted completion from the merge agent's report alone; `scripts/already-merged` (close-only) and the existing close-check prompt can verify cheaply.
- **Suggested fix shape:** after every merge report verify the task and review beads are closed (close-only when not), and before declaring a stall run already-merged close-only on any ready id already in `completed`.

### 3. super-design `scripts/coverage-inputs` matches a subepic spec to the wrong epic when its first 15 lines name sibling bead ids
- **Evidence:** coverage-inputs:68-79 tests `head -n 15` against `sorted_ids` (sorted by length) and breaks on the first hit; its comment says "first tree id its header names". All four subspecs here had a `Bead: <id>` header but also named siblings early (core-sweep: ht-poh.2, ht-poh.4; agy-adapter: ht-poh.4, ht-poh.5; delivery-policy: ht-poh.3, ht-poh.5; test-infra: ht-poh.4–6) → goals were shown under the wrong epics; two reviewers each spent their NEEDS-SPEC on it (coverage ledger c15). Round 2 (after a manual header workaround) widened 14 → 26 findings, 85% novel.
- **Premise to verify:** the §Conventions header (`Parent: … · Bead: <id>`) is reliably present; otherwise the earliest id by position is the owner.
- **Suggested fix shape:** match an explicit `Bead: <id>` token first, fall back to the earliest id by text position, and warn `multi-id-spec:` when several tree ids appear unmarked.

### 4. super-roast `assemble-args.mjs` substitutes report/caller text with `String.replace(str, replacementString)`, so `$&`, `` $` ``, `$'`, `$n` in that text corrupt the engine script
- **Evidence:** friction.md: round-2 PR roast engine failed to parse ("Unterminated string constant") because the round-1 report contained a regex `^[A-Za-z0-9._-]+$`` ` ``; assemble-args.mjs:296 ``engine.replace(hook, `const A = ${JSON.stringify(args)}`)``; same pattern at :227, :182, :118. Workaround: embedded the JSON from Python.
- **Premise to verify:** every dynamic substitution can be a function replacer (super-roast-workflow.md:277 already uses one).
- **Suggested fix shape:** `s.replace(x, () => value)` at every site taking caller- or report-derived text; add a test whose prior report contains `` $` `` and `$&`.

## Run metrics
### Judge panel
- roast-design-1: mode: design iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 39 · rr 0.87 · rg 0.82 · fg 0.79 · unanimous 0.74 · ground-loo 0.85 (n=34) · reproduce 6/33/0 · refute 1/37/1 · ground 9/30/0
- roast-design-2: mode: design iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 8 · rr 0.50 · rg 0.75 · fg 0.38 · unanimous 0.38 · ground-loo 0.75 (n=4) · reproduce 4/4/0 · refute 1/6/1 · ground 6/2/0
- roast-pr-1: mode: pr iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 47 · rr 0.74 · rg 0.81 · fg 0.77 · unanimous 0.66 · ground-loo 0.89 (n=35) · reproduce 23/24/0 · refute 17/30/0 · ground 28/19/0
- roast-pr-2: mode: pr iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0
### Fix loop
Metrics: completions — review clean 60 · after fix pass 2 · parked 0 · re-entry closes 0 · dispatched early 18 · cancelled 0
Metrics: fix-pass — entered 3 · FIXED 2 · BLOCKED 1
### Merge-back
Metrics: merges 62 · merge-failed 0 · rebase-conflicts 2 · seam-reviews 11 (fixed 0) · check-fails 0 (fixed 0)
Metrics: ledger-check ok · append-failed 0 · append-retried 0
- 64 first-parent merge commits vs 62 `Merge:` lines: the 2 extra are orchestrator base absorbs (main v0.6.0 at d342add2, 14 conflicted files; main v0.6.1/v0.6.2 at cc885360, 1 file), not coordinator merges.
### Coverage
- scope-filter: 4 in-scope · 20 punch-listed
- requirements: 13 · mapped: 12 · unmapped: 1 (R13)
- requirements: 16 · mapped: 16 · unmapped: 0 · divergence: findings 14 → 26, novel 85%, widening: yes (round 1 inputs hid owns lines and mis-matched epic goals; fixed for round 2) · integration sweep ht-poh.11
### Timing
- `Profile: unavailable — no Profile: lines in the ledger (run-profile not run per invocation)`.
- Edge audits: 9 `Edge audit:` lines, 0 `Edge cut:` lines (config edgeCuts apply-safe); detector peaks after invocation 1: 1–4 in flight vs cap 8.

### Bead graph
| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-poh.52 | task | Docs and comments describe shipped behaviour (final-review docs item, stale-docs rule) | Covers: r1 [Must-fix] docs: agent-usage.md handoff kinds, TRANSCRIPT_WINDOW doc comment, claude/permissions.rs:37 doc comment, sweep DP10 'u |
| ht-poh.51 | task | agy-relay hard budget holds under concurrent requests | Covers: [Nit] scripts/agy-relay/budget.py:71 |
| ht-poh.50 | task | agy downgrade probation must clear after one confirmed turn | Covers: [Nit] src/harness/agy/delivery.rs:842 |
| ht-poh.49 | task | DeliveryPull attention block: marker_prefix command and tests (cluster attention-block) | Covers: [Should-fix] src/service/delivery_pull.rs:294, [FYI] src/service/delivery_pull.rs:297 |
| ht-poh.48 | task | wake_on lazy wake frontier keyed on publication order | Covers: [Should-fix] src/store/lazy_delivery.rs:249 |
| ht-poh.47 | task | agy lifecycle hook with the daemon down: match Claude/Codex check-in-unavailable behaviour and document it | Covers: r1 [Must-fix] parked: daemon-down lifecycle hooks print the check-in-unavailable notice (R4 output discipline). |
| ht-poh.46 | task | Root-cause the flaky tests seen during the run | Covers: r1 [Must-fix] untested: flaky tests golden_surfaces::handoff_bootstrap_plans, handoff_topology_cli::explicit_pane_preserves_both_har |
| ht-poh.45 | task | Genuine-agy check: delivery confirm after a tool call (wake flow phase c) | Covers: r1 [Must-fix] untested: post-tool-call delivery confirm on genuine agy (final review S2). |
| ht-poh.44 | task | AGENTS.md: harness and Herdr version stance (assume backward compatibility, no exact-version gates) | Covers: user direction 2026-10-10 (not a roast finding). |
| ht-poh.43 | task | agy unsetup keeps v0.6 settings backups: document and pin with a test | Covers: r1 [Must-fix] parked: unsetup agy leaves v0.6 settings backups (human decision 2026-10-10: keep main v0.6's backup-retention model,  |
| ht-poh.42 | task | Remove the ActiveTurn poke branch and the exact-version poke_during_turn capability (DC-3) | Covers: r1 [Must-fix] parked: DC-3 ActiveTurn poke branch contradicts contract rule 4 (human decision 2026-10-10: remove; park the idea). |
| ht-poh.41 | task | R17 compact cooperative framing + AGENTS.md token-efficiency rule (re-dispatch with human-generated goldens) | Re-dispatch of ht-poh.13 on the current branch (ht-poh.13's worktree predates main v0.6.0 and later fixes). The human ran the frame-golden g |
| ht-poh.40 | task | review: ht-poh.11 | Review, fix and merge of ht-poh.11, tracked separately so ht-poh.11's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.39 | task | review: ht-poh.35 | Review, fix and merge of ht-poh.35, tracked separately so ht-poh.35's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.38 | task | review: ht-poh.36 | Review, fix and merge of ht-poh.36, tracked separately so ht-poh.36's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.37 | task | TRUST-POLICY wording: wake_on push-channel lazy completion stores no provenance string | Final review inv 5 item 6 (Task 20 policy/impl drift). The wake_on push-channel paragraph in TRUST-POLICY.md says the daemon 'records the cl |
| ht-poh.36 | task | agy transcript confirm: drop stale tracked ids so the scan window cannot stall | Final review inv 5 item 7 (Important). src/harness/agy/delivery.rs confirm() scans from the smallest recorded offset of all tracked ids. An  |
| ht-poh.35 | task | Gate scenarios: route wake-join on 'herdr-threads: wake up' and seed agy version pins | Final review inv 5 item 5. scripts/agy-gate/scenarios/delivery-stop.json (~line 27) and downgrade.json route the wake-join step on 'attentio |
| ht-poh.34 | task | review: ht-poh.28 | Review, fix and merge of ht-poh.28, tracked separately so ht-poh.28's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.33 | task | review: ht-poh.31 | Review, fix and merge of ht-poh.31, tracked separately so ht-poh.31's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.32 | task | review: ht-poh.30 | Review, fix and merge of ht-poh.30, tracked separately so ht-poh.30's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.31 | task | HookPull at-least-once after daemon restart, and no content-free wake for undeliverable items | Final review inv 4 findings 2, 3 and 6a (requirements that were dropped from ht-poh.14 because its brief predated them). |
| ht-poh.30 | task | agy resume rebinding: local conversation-state file must not suppress Resume (R3) | Final review inv 4 finding 1, reproduced on genuine agy 1.3.2 by ht-poh.28. src/harness/agy/witness.rs local_evidence treats a conversation  |
| ht-poh.6.15 | task | review: ht-poh.6.7 | Review, fix and merge of ht-poh.6.7, tracked separately so ht-poh.6.7's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.29 | task | review: ht-poh.14 | Review, fix and merge of ht-poh.14, tracked separately so ht-poh.14's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.28 | task | Fix native gate flows L2/L4/L5 against genuine agy 1.3.2 and herdr-threads v0.6 rules | Final review (inv 3) / Task 34 evidence: the gate's boot/setup/permission/launch (L4) and enroll/subagent/clear/resume (L5) flows, and the L |
| ht-poh.27 | task | Claude mod idle path: append frames, then submit minimal 'wake up' prompt | Split from ht-poh.14 (R18 item 3). Claude mod (PushChannel, IdleWake::Channel) idle path: instead of $.prompt.submit of the framed batch, $. |
| ht-poh.26 | task | review: ht-poh.25 | Review, fix and merge of ht-poh.25, tracked separately so ht-poh.25's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.6.14 | task | review: ht-poh.6.6 | Review, fix and merge of ht-poh.6.6, tracked separately so ht-poh.6.6's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.6.13 | task | review: ht-poh.6.8 | Review, fix and merge of ht-poh.6.8, tracked separately so ht-poh.6.8's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.25 | task | agy transcript confirm: parse injected steps, not raw JSONL bytes (parser-differential) | Security review finding on ht-poh.5 (commit 3fb72774, src/harness/agy/delivery.rs header_ids/scan_transcript): the confirm scan searches the |
| ht-poh.24 | task | review: ht-poh.5 | Review, fix and merge of ht-poh.5, tracked separately so ht-poh.5's dependents can start once it is implemented. The merge that lands ht-poh |
| ht-poh.4.16 | task | review: ht-poh.4.5 | Review, fix and merge of ht-poh.4.5, tracked separately so ht-poh.4.5's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.23 | task | review: ht-poh.22 | Review, fix and merge of ht-poh.22, tracked separately so ht-poh.22's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.22 | task | handoff --new-tab for V2 bootstrap kinds (agy, Hermes): place the startup prompt via LaunchPolicy::prepare_startup_input | Final review (phase 3 inv 2): preflight_launch_request in src/cli/topology_handoff.rs appends the V1 bare prompt argument for every kind; ag |
| ht-poh.21 | task | review: ht-poh.18 | Review, fix and merge of ht-poh.18, tracked separately so ht-poh.18's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.20 | task | review: ht-poh.17 | Review, fix and merge of ht-poh.17, tracked separately so ht-poh.17's dependents can start once it is implemented. The merge that lands ht-p |
| ht-poh.19 | task | Gate: human runs or allows scripts/gen-frame-goldens.mjs for ht-poh.13 | The permission classifier refused 'node scripts/gen-frame-goldens.mjs' twice for ht-poh.13 (R17 compact cooperative framing) as Security Wea |
| ht-poh.18 | task | agy allow entry under main v0.6.0 permission consent and backup model | Base absorb: main v0.6.0 adds a permission core with consent and config backups. agy's owned permissions.allow member (ht-poh.4.4, SettingsK |
| ht-poh.17 | task | Reconcile main v0.6.0 lazy hook delivery with HookPull and redo the delivery-contract audit | Base absorb: main v0.6.0 delivers lazy bodies at standard hooks under the claim cooperative_hook_context. This branch adds DeliveryPolicy::H |
| ht-poh.16 | task | Harness-neutral migration of main v0.6.0 permissions core | Base absorb (main v0.6.0 merged into the run branch): main added src/harness/permissions{,/claude,/claude_settings,/codex_rules}.rs and setu |
| ht-poh.6.12 | task | review: ht-poh.6.4 | Review, fix and merge of ht-poh.6.4, tracked separately so ht-poh.6.4's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.4.15 | task | review: ht-poh.4.8 | Review, fix and merge of ht-poh.4.8, tracked separately so ht-poh.4.8's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.4.14 | task | review: ht-poh.4.7 | Review, fix and merge of ht-poh.4.7, tracked separately so ht-poh.4.7's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.4.13 | task | review: ht-poh.4.6 | Review, fix and merge of ht-poh.4.6, tracked separately so ht-poh.4.6's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.6.11 | task | review: ht-poh.6.5 | Review, fix and merge of ht-poh.6.5, tracked separately so ht-poh.6.5's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.4.12 | task | review: ht-poh.4.3 | Review, fix and merge of ht-poh.4.3, tracked separately so ht-poh.4.3's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.4.11 | task | review: ht-poh.4.4 | Review, fix and merge of ht-poh.4.4, tracked separately so ht-poh.4.4's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.4.10 | task | review: ht-poh.4.2 | Review, fix and merge of ht-poh.4.2, tracked separately so ht-poh.4.2's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.2.16 | task | review: ht-poh.2.8 | Review, fix and merge of ht-poh.2.8, tracked separately so ht-poh.2.8's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.3.14 | task | review: ht-poh.3.4 | Review, fix and merge of ht-poh.3.4, tracked separately so ht-poh.3.4's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.15 | task | review: ht-poh.8 | Review, fix and merge of ht-poh.8, tracked separately so ht-poh.8's dependents can start once it is implemented. The merge that lands ht-poh |
| ht-poh.2.15 | task | review: ht-poh.2.4 | Review, fix and merge of ht-poh.2.4, tracked separately so ht-poh.2.4's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.3.13 | task | review: ht-poh.3.6 | Review, fix and merge of ht-poh.3.6, tracked separately so ht-poh.3.6's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.4.9 | task | review: ht-poh.4.1 | Review, fix and merge of ht-poh.4.1, tracked separately so ht-poh.4.1's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.2.14 | task | review: ht-poh.2.2 | Review, fix and merge of ht-poh.2.2, tracked separately so ht-poh.2.2's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.3.12 | task | review: ht-poh.3.7 | Review, fix and merge of ht-poh.3.7, tracked separately so ht-poh.3.7's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.2.13 | task | review: ht-poh.2.3 | Review, fix and merge of ht-poh.2.3, tracked separately so ht-poh.2.3's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.3.11 | task | review: ht-poh.3.3 | Review, fix and merge of ht-poh.3.3, tracked separately so ht-poh.3.3's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.2.12 | task | review: ht-poh.2.6 | Review, fix and merge of ht-poh.2.6, tracked separately so ht-poh.2.6's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.2.11 | task | review: ht-poh.2.5 | Review, fix and merge of ht-poh.2.5, tracked separately so ht-poh.2.5's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.2.10 | task | review: ht-poh.2.1 | Review, fix and merge of ht-poh.2.1, tracked separately so ht-poh.2.1's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.3.10 | task | review: ht-poh.3.2 | Review, fix and merge of ht-poh.3.2, tracked separately so ht-poh.3.2's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.3.9 | task | review: ht-poh.3.5 | Review, fix and merge of ht-poh.3.5, tracked separately so ht-poh.3.5's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.14 | task | Minimal wake prompt (core + agy): adapter-declared wake text, agy 'wake up', HookPull restart and offer-exhausted safety | User direction 2026-10-10 (R18): the nudge should say something minimal like 'wake up' and let the hook inject the actual message. |
| ht-poh.13 | task | Compact cooperative message framing: drop per-delivery boilerplate and 'untrusted' wording | User direction 2026-10-10 (R17). Delivered peer messages are too verbose and framed as hostile. Example the user flagged (Claude mod submit) |
| ht-poh.2.9 | task | review: ht-poh.2.7 | Review, fix and merge of ht-poh.2.7, tracked separately so ht-poh.2.7's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.3.8 | task | review: ht-poh.3.1 | Review, fix and merge of ht-poh.3.1, tracked separately so ht-poh.3.1's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.6.10 | task | review: ht-poh.6.2 | Review, fix and merge of ht-poh.6.2, tracked separately so ht-poh.6.2's dependents can start once it is implemented. The merge that lands ht |
| ht-poh.12 | task | review: ht-poh.9 | Review, fix and merge of ht-poh.9, tracked separately so ht-poh.9's dependents can start once it is implemented. The merge that lands ht-poh |
| ht-poh.11 | task | Integration sweep: agy harness adapter on a harness-neutral core | Root integration sweep. Verify the goal's main flows end to end on the integrated branch: (1) bare agy enrollment, child vs /clear, resume,  |
| ht-poh.2.8 | task | Strict neutral check: forbid to-migrate entries | R8 enforcement after the sweep: scripts/check-harness-neutral fails on any 'to-migrate' allowlist entry (category removed from the format, o |
| ht-poh.6.9 | task | Final-SHA evidence: full native gate + live smoke, commit records | R12 evidence at the final code SHA. Run scripts/agy-native-gate with every flow, using genuine agy (HT_AGY_BIN) in the isolated world, then  |
| ht-poh.10 | task | Gate: fix loop exited | Released by super-auto when the phase-5 code fix loop exits (final code SHA). Holds final-SHA evidence work. |
| ht-poh.2.7 | task | Golden snapshots of per-harness surfaces before the sweep and re-fit | Byte-identity guard for R8 and R9. Before any move, capture golden snapshots, via tests that run today's code, for Claude, Codex and Hermes: |
| ht-poh.9 | task | Outside-adapter accounting tool: scripts/outside-adapter-report | R13. A script that, given base and head SHAs, partitions the diff into src/harness/agy/** (plus the builtins() row) versus everything else.  |
| ht-poh.6.8 | task | Live smoke runner (`--dry-run` / `--live`) and evidence README | `RelayBackend` (in its own module), `live_smoke.py` and `scripts/agy-live-smoke` per D11: clean-SHA rule, CI guard, a flow of enroll, then w |
| ht-poh.6.7 | task | Gate flow: idle wake into the composer | `flows/wake.py`, D7 row `wake` (a)–(e): wake on an ACK-required message into an empty composer with in-request delivery; no wake for lazy-on |
| ht-poh.6.6 | task | Gate flows: delivery with transcript confirm, and the forced downgrade | `flows/delivery.py` (lazy, ordinary, ACK-required and truncated items; a held inv0 to prove nothing settles before confirm; ACK via `run_com |
| ht-poh.6.5 | task | Gate flows: enroll, subagent, `/clear`, resume | Flow modules and scenarios for the D7 rows `enroll`, `subagent` (the `invoke_subagent` shape from research B, with match-routed child and pa |
| ht-poh.6.4 | task | Gate flows: setup, permission, allow-rewrite, unsetup, launch | Flow modules and scenarios for the D7 rows `setup`, `permission`, `allow-rewrite`, `unsetup` and `launch`, with flow-local DB/CLI queries. |
| ht-poh.6.3 | task | Gemini↔OpenAI relay with enforced budget, plus the OpenAI fake upstream | `translate.py` (D9: joined system parts, `functionResponse` to `tool` with id pairing, preserved or aliased Gemini ids, fresh unique emitted |
| ht-poh.6.2 | task | Isolated agy world, native gate runner, `boot` flow, `fetch-agy` | Implement `agyenv.py` per D4/D5/D8: root via `ih_root_new`, seed settings, sandbox profile, binary staging (`cp -c`) and wrapper, `restage_a |
| ht-poh.6.1 | task | Fake Gemini server, scenario format `agy-fake-gemini/1`, CI line for offline agy-script tests | Grow `harness-fake_server.py` into `server.py` plus `scenario.py` per D2: routes, reply kinds, `delay_ms` and chunked SSE, match routing, `{ |
| ht-poh.4.8 | task | Docs, README rows, handoff check | README harness table row and registry sentence (`README.md:114-128`); `integrations/agy/README.md` (operator guide: setup, plugin enable, al |
| ht-poh.4.7 | task | Doctor projection and text | `doctor.rs` per D10: projection from L3 status, witness-health scan (bounded, read-only), sandbox note, limitations, safe/manual repairs, `d |
| ht-poh.4.6 | task | Launch and handoff policy (agy LaunchPolicy, V2 handoff plan) and HERDR_THREADS_AGY_OPTS | `AgyLaunch` per D8: flag table from the `agy --help` 1.3.2 fixture (`tests/fixtures/agy/help/agy-1.3.2.txt`), refusals, `-c`/`--conversation |
| ht-poh.4.5 | task | Composer policy from model-free TUI captures | Capture `agent read --source detection` for agy states (empty, draft, multi-line, busy, permission prompt). Use genuine agy (`HT_AGY_BIN`) a |
| ht-poh.4.4 | task | Owned plugin dir, skill, allow entry: setup/status/unsetup | `plugin.rs` + `setup.rs` per D7: render plugin.json, hooks.json and SKILL.md (`SKILL_MD` + `AGY_SKILL_GUIDANCE`); owned-asset manifest, conf |
| ht-poh.4.3 | task | Witness evidence, lifecycle reading (startup, /clear, resume via C1), enrollment with startup check-in context, TRUST-POLICY startup text | `witness.rs`: D5 `local_evidence` (presence lock, the listing over bound parent and bounded glob of 65,536 entries plus deadline, K hint, li |
| ht-poh.4.2 | task | agy module skeleton, metadata, registration, contract, decode/encode, hook argv | D1 skeleton: `mod.rs` with every trait method forwarding to stub submodules (setup/status/unsetup refuse or report not installed; `launch_po |
| ht-poh.4.1 | task | Neutral hook seams: `local_evidence` + witness rule, `hook_step` | Lands D4 and D6, inert by default. **What it adds:** `Witness`, `RoleWitness::decide`, `LocalEvidence::refine`, `LocalEvidenceRequest`, `Har |
| ht-poh.8 | task | Bootstrap plan V2: adapter-neutral retained composition for handoff | Finding from ht-poh.2 D16 (R7, R13): the frozen V1 bootstrap grammar (harness/launch.rs compose_bootstrap_v1_argv ~:266-281; cli/handoff.rs  |
| ht-poh.2.6 | task | Hook runner, contract, attribution and evidence neutral | D9, D10 and the L6 rows of D15. `InstalledHarness` handle; `CodexWitness` removed from `adapter.rs`, `cli/hook.rs` and the single arm at `cl |
| ht-poh.2.5 | task | Daemon, store, app and health neutral | D11, D12 and the L5 rows of D15. `HealthInputs.harnesses` with the legacy-contract predicate and a single allowlisted v1 projection; registr |
| ht-poh.2.4 | task | Doctor via adapter projections and text | D7. Moves the Claude and Codex projection bodies, observed/verdict/path helpers, `cfg(test)` installed helpers and per-harness repair rules  |
| ht-poh.2.3 | task | Setup engine and setup CLI de-harnessed | D4, D5, D6 (callers), D8 and the L3 rows of D15. `SetupEnv.config_roots` via `default_config_root`; foreground via `foreground_advice`; the  |
| ht-poh.2.2 | task | CLI help, env markers, launch/handoff surface | D2, D3 and the L2 rows of D15. Registry-derived markers in `cli/mod.rs`/`cli/output.rs` (picker unchanged); new `cli/help.rs` assembling set |
| ht-poh.2.1 | task | Adapter trait hooks seam | Lands every D1 hook with a neutral default, plus the `agent_env_markers` metadata field (all 48 literals in src and tests). Adds registry va |
| ht-poh.3.7 | task | Push-channel promotion of `wake_on: all` lazy rows | DP8 promotion. `InboxBatchV2Item::LazyMessage.wakes` (skip-if-false, the `informational` precedent) is set by the store's InboxBatchV2 query |
| ht-poh.3.6 | task | Core wake decision honours `wake_on: all`, plus SKILL section | DP8 core wake. `wake_seat_attention` reads `wake_on`; `lazy_delivery::pending_summary` is shared with `mod_seat_view`. `LogicalAttentionFron |
| ht-poh.3.5 | task | `wake_on` preference: migration, store, wire and `herdr-threads wake-on` | DP8 storage, wire, store and CLI. The next free migration adds `seats.wake_on` and `wake_work.last_lazy_ordinal`, with schema.rs lists updat |
| ht-poh.3.4 | task | HookPull handlers: `DeliveryPull`/`DeliveryConfirm`, offered ledger, pull-seat routing | Replace L1's inert arms with DP5/DP6: `src/delivery/ledger.rs` (pure bounded ledger) and `src/service/delivery_pull.rs` (Mutex owner, store- |
| ht-poh.3.3 | task | Claude `PushChannel` re-fit: every Claude check becomes a policy lookup | DP3 and the DP7 idle-wake part. The Claude adapter declares `(PushChannel, cooperative_mod_delivery, Channel)`. Replace dispatch.rs:531/557/ |
| ht-poh.3.2 | task | Extract `src/delivery/` from `watch` and port the D4 frame to Rust | Move-only extraction (DP2). Add `delivery/{mod,assemble,page,frame}.rs`. `cli/watch.rs` keeps a facade with the same names and signatures (` |
| ht-poh.3.1 | task | Delivery seam: `DeliveryPolicy` trait, registry lookup, `DeliveryPull`/`DeliveryConfirm` wire, inert handlers | **Seam for ht-poh.5.** Add `DeliveryMechanism`, `IdleWake`, the `DeliveryPolicy` trait and `HarnessAdapter::delivery_policy()` (default `Non |
| ht-poh.7 | task | AGENTS.md harness-neutral rule + delivery contract, and delivery-contract audit with inline fixes | Spec §1 and §Delivery contract (R8 text, R10). AGENTS.md sections 'Harness-neutral core' and 'Delivery contract' (six rules, token-efficienc |
| ht-poh.6 | epic | Test infrastructure: agy fake Gemini server + native gate, Gemini↔OpenAI relay + live smoke script | Spec §5 (R12). scripts/agy-fake-gemini/ (Python scripted Gemini-native server grown from the probe harness at ~/Documents/Agy_Integration_Re |
| ht-poh.5 | task | agy HookPull delivery: transcript confirm, pull, systemMessage inject, Stop downgrade | Spec §3 Delivery/Downgrade/Output discipline (R4). agy adapter declares HookPull/cooperative_hook_delivery/Native; PreInvocation confirm (bo |
| ht-poh.4 | epic | agy adapter (minus delivery): metadata, owned plugin + skill, owned allow entry, local evidence + decode/witness, context, composer, launch, doctor | Spec §3 except the HookPull delivery part (R1, R2, R3, R5, R6, R7). src/harness/agy/ module + builtins() row; local_evidence trait step (pre |
| ht-poh.3 | epic | Delivery policy: src/delivery extraction, DeliveryPolicy trait, Claude PushChannel re-fit, HookPull requests, wake_on + SKILL section | Spec §2 (R4 core side, R9, R11). Deliver: neutral src/delivery/ (D4 framing, truncation, pages, dedupe, settlement, offered ledger); Deliver |
| ht-poh.2 | epic | Core sweep: move every production harness-specific site behind the adapter (empty all to-migrate allowlist entries) | Spec §1 table (R8). Every site in the table moves behind HarnessAdapter/registry or becomes a permanent frozen-wire/history allowlist entry  |
| ht-poh.1 | task | Neutral rule seam: check-harness-neutral script + allowlist (format, to-migrate entries, D14 path rule), CI and per-change wiring | Spec §1 (R8) — seam: scripts/check-harness-neutral (shell) failing on harness id literals / Harness::<Name> / harness-named types or fields  |
| ht-poh | epic | agy harness adapter on a harness-neutral core | Root epic for super-auto run 2026-10-09-agy-harness-adapter. Spec: docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harne |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-poh.41 | ht-poh.3.2 | consumes Rust delivery::frame |
| ht-poh.41 | ht-poh.14 | consumes Claude mod idle path in register.js |
| ht-poh.40 | ht-poh.11 | unstated |
| ht-poh.39 | ht-poh.35 | unstated |
| ht-poh.38 | ht-poh.36 | unstated |
| ht-poh.34 | ht-poh.28 | unstated |
| ht-poh.33 | ht-poh.31 | unstated |
| ht-poh.32 | ht-poh.30 | unstated |
| ht-poh.6.15 | ht-poh.6.7 | unstated |
| ht-poh.29 | ht-poh.14 | unstated |
| ht-poh.28 | ht-poh.30 | consumes resume rebinding fix the gate's resume flow asserts |
| ht-poh.26 | ht-poh.25 | unstated |
| ht-poh.6.14 | ht-poh.6.6 | unstated |
| ht-poh.6.13 | ht-poh.6.8 | unstated |
| ht-poh.25 | ht-poh.5 | consumes src/harness/agy/delivery.rs confirm scan |
| ht-poh.24 | ht-poh.5 | unstated |
| ht-poh.4.16 | ht-poh.4.5 | unstated |
| ht-poh.23 | ht-poh.22 | unstated |
| ht-poh.21 | ht-poh.18 | unstated |
| ht-poh.20 | ht-poh.17 | unstated |
| ht-poh.18 | ht-poh.16 | consumes neutral permissions-core seam |
| ht-poh.6.12 | ht-poh.6.4 | unstated |
| ht-poh.4.15 | ht-poh.4.8 | unstated |
| ht-poh.4.14 | ht-poh.4.7 | unstated |
| ht-poh.4.13 | ht-poh.4.6 | unstated |
| ht-poh.6.11 | ht-poh.6.5 | unstated |
| ht-poh.4.12 | ht-poh.4.3 | unstated |
| ht-poh.4.11 | ht-poh.4.4 | unstated |
| ht-poh.4.10 | ht-poh.4.2 | unstated |
| ht-poh.2.16 | ht-poh.2.8 | unstated |
| ht-poh.3.14 | ht-poh.3.4 | unstated |
| ht-poh.15 | ht-poh.8 | unstated |
| ht-poh.2.15 | ht-poh.2.4 | unstated |
| ht-poh.3.13 | ht-poh.3.6 | unstated |
| ht-poh.4.9 | ht-poh.4.1 | unstated |
| ht-poh.2.14 | ht-poh.2.2 | unstated |
| ht-poh.3.12 | ht-poh.3.7 | unstated |
| ht-poh.2.13 | ht-poh.2.3 | unstated |
| ht-poh.3.11 | ht-poh.3.3 | unstated |
| ht-poh.2.12 | ht-poh.2.6 | unstated |
| ht-poh.2.11 | ht-poh.2.5 | unstated |
| ht-poh.2.10 | ht-poh.2.1 | unstated |
| ht-poh.3.10 | ht-poh.3.2 | unstated |
| ht-poh.3.9 | ht-poh.3.5 | unstated |
| ht-poh.14 | ht-poh.5 | consumes agy turn-start injection of pulled bodies |
| ht-poh.14 | ht-poh.4.5 | consumes agy composer policy used by Native wake |
| ht-poh.14 | ht-poh.3.6 | consumes core wake decision (wake_on) |
| ht-poh.14 | ht-poh.3.3 | consumes policy-based idle wake (wake_suppressed, IdleWake on the channel) |
| ht-poh.13 | ht-poh.3.6 | consumes SKILL wake-on section (same SKILL.md) |
| ht-poh.13 | ht-poh.3.2 | consumes Rust delivery::frame and golden parity fixtures |
| ht-poh.13 | ht-poh.3.4 | consumes HookPull handlers (DP10 guard) |
| ht-poh.13 | ht-poh.19 | Gate: human runs or allows scripts/gen-frame-goldens.mjs |
| ht-poh.13 | ht-poh.3.7 | consumes push-channel promotion (DP10 guard; delivery::page) |
| ht-poh.13 | ht-poh.2.7 | consumes golden snapshots to update |
| ht-poh.13 | ht-poh.3.3 | consumes Claude PushChannel re-fit (DP10 byte-identity guard) |
| ht-poh.2.9 | ht-poh.2.7 | unstated |
| ht-poh.3.8 | ht-poh.3.1 | unstated |
| ht-poh.6.10 | ht-poh.6.2 | unstated |
| ht-poh.12 | ht-poh.9 | unstated |
| ht-poh.11 | ht-poh.17 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.25 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.22 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.3.3 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.3.2 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.2 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.2 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.7 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.8 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.36 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.7 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.37 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.9 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.18 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.5 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.1 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.7 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.8 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.3.1 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.28 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.5 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.2 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.3 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.3.6 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.5 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.6 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.16 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.1 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.4 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.14 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.4 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.6 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.6 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.8 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.3 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.35 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.30 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.3.5 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.6.4 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.3.7 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.3.4 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.7 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.5 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.2.1 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.8 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.4.3 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.31 | consumes all leaves (integration sweep) |
| ht-poh.11 | ht-poh.1 | consumes all leaves (integration sweep) |
| ht-poh.2.8 | ht-poh.2.6 | consumes removal of its to-migrate allowlist entries |
| ht-poh.2.8 | ht-poh.2.3 | consumes removal of its to-migrate allowlist entries |
| ht-poh.2.8 | ht-poh.3.3 | consumes removal of its to-migrate allowlist entries |
| ht-poh.2.8 | ht-poh.2.5 | consumes removal of its to-migrate allowlist entries |
| ht-poh.2.8 | ht-poh.2.2 | consumes removal of its to-migrate allowlist entries |
| ht-poh.2.8 | ht-poh.2.4 | consumes removal of its to-migrate allowlist entries |
| ht-poh.2.8 | ht-poh.3.4 | consumes removal of its to-migrate allowlist entries |
| ht-poh.6.9 | ht-poh.11 | consumes integrated branch after the integration sweep |
| ht-poh.6.9 | ht-poh.6.5 | consumes gate flow / runner exercised at the final SHA |
| ht-poh.6.9 | ht-poh.10 | consumes Gate: fix loop exited (final code SHA) |
| ht-poh.6.9 | ht-poh.6.8 | consumes gate flow / runner exercised at the final SHA |
| ht-poh.6.9 | ht-poh.2.8 | consumes strict neutral check (zero to-migrate) |
| ht-poh.6.9 | ht-poh.6.6 | consumes gate flow / runner exercised at the final SHA |
| ht-poh.6.9 | ht-poh.35 | consumes working delivery-stop/downgrade gate scenarios and pins |
| ht-poh.6.9 | ht-poh.6.4 | consumes gate flow / runner exercised at the final SHA |
| ht-poh.6.9 | ht-poh.6.7 | consumes gate flow / runner exercised at the final SHA |
| ht-poh.6.9 | ht-poh.9 | consumes scripts/outside-adapter-report |
| ht-poh.6.9 | ht-poh.28 | consumes working gate flows for the final-SHA run |
| ht-poh.2.7 | ht-poh.7 | consumes delivery-contract audit fixes (snapshots capture post-audit behaviour) |
| ht-poh.9 | ht-poh.1 | consumes Outside-adapter trailer format |
| ht-poh.6.8 | ht-poh.6.2 | consumes World/ModelBackend API |
| ht-poh.6.8 | ht-poh.4.4 | consumes the behaviour this flow exercises (Owned plugin dir, skill, allow entry: setup/status/unsetup) |
| ht-poh.6.8 | ht-poh.3.4 | consumes DeliveryPull/Confirm handlers for the delivery the smoke exercises |
| ht-poh.6.8 | ht-poh.4.3 | consumes the behaviour this flow exercises (Witness evidence, lifecycle reading, enrollment, TRUST-POLICY startup text) |
| ht-poh.6.8 | ht-poh.3.6 | consumes core wake decision for the smoke's idle wake |
| ht-poh.6.8 | ht-poh.5 | consumes the behaviour this flow exercises (agy HookPull delivery: transcript confirm, pull, systemMessage inject, Stop  |
| ht-poh.6.8 | ht-poh.6.3 | consumes relay CLI and fake upstream |
| ht-poh.6.8 | ht-poh.4.5 | consumes the behaviour this flow exercises (Composer policy from model-free TUI captures) |
| ht-poh.6.7 | ht-poh.4.5 | consumes the behaviour this flow exercises (Composer policy from model-free TUI captures) |
| ht-poh.6.7 | ht-poh.6.2 | consumes World/Flow API |
| ht-poh.6.7 | ht-poh.14 | consumes minimal wake text for agy |
| ht-poh.6.7 | ht-poh.5 | consumes the behaviour this flow exercises (agy HookPull delivery: transcript confirm, pull, systemMessage inject, Stop  |
| ht-poh.6.7 | ht-poh.3.4 | consumes the behaviour this flow exercises (HookPull handlers: `DeliveryPull`/`DeliveryConfirm`, offered ledger, pull-se |
| ht-poh.6.7 | ht-poh.3.6 | consumes the behaviour this flow exercises (Core wake decision honours `wake_on: all`, plus SKILL section) |
| ht-poh.6.7 | ht-poh.6.1 | consumes `delay_ms` busy step |
| ht-poh.6.6 | ht-poh.3.4 | consumes the behaviour this flow exercises (HookPull handlers: `DeliveryPull`/`DeliveryConfirm`, offered ledger, pull-se |
| ht-poh.6.6 | ht-poh.4.7 | consumes the behaviour this flow exercises (Doctor projection and text) |
| ht-poh.6.6 | ht-poh.6.1 | consumes scenario hold/bind |
| ht-poh.6.6 | ht-poh.6.2 | consumes World/Flow API, `restage_agy` |
| ht-poh.6.6 | ht-poh.25 | consumes corrected transcript confirm for the delivery gate flow |
| ht-poh.6.6 | ht-poh.5 | consumes the behaviour this flow exercises (agy HookPull delivery: transcript confirm, pull, systemMessage inject, Stop  |
| ht-poh.6.5 | ht-poh.4.2 | consumes the behaviour this flow exercises (agy module skeleton, metadata, registration, contract, decode/encode, hook a |
| ht-poh.6.5 | ht-poh.6.1 | consumes scenario format and match routing |
| ht-poh.6.5 | ht-poh.4.3 | consumes the behaviour this flow exercises (Witness evidence, lifecycle reading, enrollment, TRUST-POLICY startup text) |
| ht-poh.6.5 | ht-poh.4.4 | consumes the behaviour this flow exercises (Owned plugin dir, skill, allow entry: setup/status/unsetup) |
| ht-poh.6.5 | ht-poh.6.2 | consumes World/Flow API |
| ht-poh.6.4 | ht-poh.8 | consumes bootstrap plan V2 for the handoff --kind agy flow |
| ht-poh.6.4 | ht-poh.4.6 | consumes the behaviour this flow exercises (Launch policy and `HERDR_THREADS_AGY_OPTS`) |
| ht-poh.6.4 | ht-poh.6.2 | consumes World/Flow API |
| ht-poh.6.4 | ht-poh.4.4 | consumes the behaviour this flow exercises (Owned plugin dir, skill, allow entry: setup/status/unsetup) |
| ht-poh.6.4 | ht-poh.6.1 | consumes scenario format |
| ht-poh.6.4 | ht-poh.4.7 | consumes the behaviour this flow exercises (Doctor projection and text) |
| ht-poh.6.2 | ht-poh.6.1 | consumes server CLI and scenario format |
| ht-poh.4.8 | ht-poh.8 | consumes bootstrap V2 |
| ht-poh.4.8 | ht-poh.4.4 | consumes setup docs |
| ht-poh.4.8 | ht-poh.4.6 | consumes launch policy |
| ht-poh.4.7 | ht-poh.2.3 | consumes setup status projection used by doctor |
| ht-poh.4.7 | ht-poh.2.4 | consumes doctor rendering via doctor_text/doctor_projection |
| ht-poh.4.7 | ht-poh.4.4 | consumes status API |
| ht-poh.4.7 | ht-poh.4.3 | consumes witness helpers |
| ht-poh.4.6 | ht-poh.4.4 | consumes `plugin::status` |
| ht-poh.4.6 | ht-poh.4.2 | consumes launch stub |
| ht-poh.4.6 | ht-poh.8 | consumes bootstrap plan V2 (agy handoff produces a V2 plan) |
| ht-poh.4.6 | ht-poh.2.2 | consumes registry-assembled launch/handoff help and opts-env list |
| ht-poh.4.5 | ht-poh.4.2 | consumes composer stub/forwarder |
| ht-poh.4.4 | ht-poh.4.2 | consumes paths, argv builder, stubs |
| ht-poh.4.4 | ht-poh.2.3 | consumes setup engine and setup CLI driven by settings_kind/setup hooks (kind-generic legacy helpers) |
| ht-poh.4.4 | ht-poh.3.6 | consumes SKILL_MD with the wake-on section (owned by ht-poh.3.6) rendered into the agy skill |
| ht-poh.4.3 | ht-poh.4.2 | consumes module, decode, paths |
| ht-poh.4.3 | ht-poh.4.1 | consumes trait + refine + runner call |
| ht-poh.4.2 | ht-poh.2.1 | consumes artifact |
| ht-poh.4.2 | ht-poh.1 | consumes artifact |
| ht-poh.4.1 | ht-poh.1 | consumes artifact |
| ht-poh.4.1 | ht-poh.2.1 | consumes artifact |
| ht-poh.4.1 | ht-poh.2.6 | consumes neutral hook runner (InstalledHarness handle, find_seat) in cli/hook.rs |
| ht-poh.8 | ht-poh.2.2 | consumes neutral launch/handoff surface (help, opts list, topology kind lookup) |
| ht-poh.8 | ht-poh.1 | consumes check-harness-neutral |
| ht-poh.8 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.2.6 | ht-poh.2.1 | consumes `VersionSyntax`, `TranscriptVersionSpec` |
| ht-poh.2.6 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.2.5 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.2.5 | ht-poh.2.1 | consumes `AdapterText` |
| ht-poh.2.4 | ht-poh.2.1 | consumes doctor hooks and forwarder modules |
| ht-poh.2.4 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.2.3 | ht-poh.2.1 | consumes `SettingsKindSpec`, setup hooks |
| ht-poh.2.3 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.2.2 | ht-poh.2.1 | consumes `AgentEnvMarker`, `AdapterText` |
| ht-poh.2.2 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.2.1 | ht-poh.1 | consumes artifact |
| ht-poh.2.1 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.3.7 | ht-poh.3.5 | consumes wake_on accessor |
| ht-poh.3.7 | ht-poh.3.2 | consumes delivery::page |
| ht-poh.3.7 | ht-poh.3.3 | consumes PushChannel re-fit of the watch/dispatch path |
| ht-poh.3.6 | ht-poh.3.3 | consumes wake_suppressed and policy-based idle wake |
| ht-poh.3.6 | ht-poh.7 | consumes audit |
| ht-poh.3.6 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.3.6 | ht-poh.3.5 | consumes wake_on column and accessor, last_lazy_ordinal |
| ht-poh.3.5 | ht-poh.1 | consumes check |
| ht-poh.3.4 | ht-poh.3.1 | consumes wire types |
| ht-poh.3.4 | ht-poh.3.3 | consumes `ack_policy_delivered`, policy lookups |
| ht-poh.3.4 | ht-poh.3.2 | consumes src/delivery API |
| ht-poh.3.4 | ht-poh.7 | consumes audit |
| ht-poh.3.3 | ht-poh.7 | consumes audit |
| ht-poh.3.3 | ht-poh.3.2 | consumes src/delivery module (delivery::settle lands inside it) |
| ht-poh.3.3 | ht-poh.1 | consumes allowlist |
| ht-poh.3.3 | ht-poh.3.1 | consumes DeliveryPolicy trait, `Registry::delivery_policy` |
| ht-poh.3.3 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.3.2 | ht-poh.7 | consumes contract audit before the refactor |
| ht-poh.3.2 | ht-poh.2.7 | consumes golden snapshot fixtures for per-harness surfaces (byte-identity baseline) |
| ht-poh.3.2 | ht-poh.1 | consumes check-harness-neutral (src/delivery must name no harness) |
| ht-poh.3.1 | ht-poh.1 | consumes check-harness-neutral |
| ht-poh.5 | ht-poh.4.1 | consumes hook_step seam (HookStep/HookStepContext/HookStepDaemon) and runner branch |
| ht-poh.5 | ht-poh.4.5 | consumes agy ComposerPolicy used by Native idle wake |
| ht-poh.5 | ht-poh.3.1 | consumes DeliveryPolicy trait, registry lookup and HookPull wire types (DeliveryPull/DeliveryConfirm); end-to-end daemon |
| ht-poh.5 | ht-poh.3.2 | consumes D4 frame text and src/delivery API |
| ht-poh.5 | ht-poh.4.4 | consumes plugin hooks.json registering PreInvocation/Stop |
| ht-poh.5 | ht-poh.3.4 | consumes DeliveryPull/Confirm handlers and offered-ledger semantics |
| ht-poh.5 | ht-poh.4.2 | consumes agy module skeleton, InjectionStep encodings, paths::conversation_state location |
| ht-poh.5 | ht-poh.4.3 | consumes top-level role on PreInvocation (witness mapping) |
| ht-poh.5 | ht-poh.4.7 | consumes agy doctor.rs projection the downgrade line extends |
| ht-poh.5 | ht-poh.17 | consumes merged lazy-delivery model (which path delivers lazy rows for HookPull seats) |

## Design questions
### Should the edge audit be skipped when every critical-path head is human-gated and the graph is unchanged?
9 audits, 0 cuts; 6 reported "changes: none"; 5 concluded the fastest gain was clearing a human gate; each re-checked the same parked safe-no edge. For skipping: saves an opus-class dispatch per invocation and points straight at the only actionable lever (a one-line "critical path held by human gate X" escalation). For keeping: the audit is the only place safe-no candidates get written down for a later operator, and it is cheap relative to a stalled run.
If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### Should a BLOCKED-AUTH step behind a user-directed change route to the top-level orchestrator, and should planner edge-adds flag a BLOCKED/human-gated blocker?
A user-directed wording change had its golden regeneration (`node scripts/gen-frame-goldens.mjs`) refused twice as "Security Weaken"; the only path was BLOCKED-AUTH → human gate bead → a new bead re-implementing the task (the original superseded, its worktree left with uncommitted work). Meanwhile the planner added `ht-poh.14 <- ht-poh.13` onto the refused bead, deepening a six-deep human-gated chain until the orchestrator split the dependent. For: saves a re-implementation and the chain depth. Against: moves permission-sensitive commands out of the isolated task worktree, and later audits judged the edge itself genuine.
If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps
### super-design says `owns:`/`consumes:` live inline in the short description, but coverage-inputs only extracts them at line start
SKILL.md §Decomposition: "it lives inside the short description … The literal tokens `owns:` and `consumes:` are required"; coverage-inputs:106 matches `^\s*(-\s*)?(owns:|consumes:|boundary contract:)`. Round-1 inputs carried 2 `owns:` lines; after the orchestrator moved inline (bold) declarations onto their own lines in 31 beads, round 2 carried 30. Round 1's ten UNOWNED-SEAM findings were rejected as an inputs defect (coverage ledger c12). Fix shape: match the tokens anywhere in a line (stripping `**`/`_`), or require one declaration per line and warn when the token appears mid-line.

## Already fixed — do not re-litigate
none

## Not established
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.
- The Metrics lines are the last invocation's; per-invocation totals across 8 invocations were not summed.
- Defect 1's "two Important findings resurfaced" is attributed from the final review text and the brief contents; a controlled re-run was not done.
- No Profile: lines, so no critical-path timing is claimed.
- Dropped as below the bar (not reported): one implementer ran the product's setup against the real HOME once (the rule already exists at coordinator.js:1543; cleanup verified); the orchestrator's own table-parsing tooling; the merge-count difference (base absorbs).

## Verification bar
- Defect 1: re-dispatch a task after amending its bead description and check the implementer's brief/report reflects the amendment; same for a triage RESOLVE.
- Defect 2: a dryRun/replay where a merge agent reports success but `bd close` fails; the coordinator must close-only (or re-check) instead of returning `stalled`.
- Defect 3: coverage-inputs over a fixture tree whose subspec headers name sibling ids before their own `Bead:` id.
- Defect 4: assemble-args with a prior report containing `` $` `` and `$&`; the generated engine must parse and carry the text verbatim.
- Doc gap: coverage-inputs over a bead with an inline `**owns:**` declaration.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
