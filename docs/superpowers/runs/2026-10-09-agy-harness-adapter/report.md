status: completed with 0 unresolved Blocking, 2 escalations [degraded: final review: not ready (invocation 8 final review: no Critical; Important S1 agy informational notices only at lifecycle check-ins; unmeasured branch; flaky ht-01s + two unclassified flakes) — code-final-review-8.md]
qualifier: final review: not ready (invocation 8 final review: no Critical; Important S1 agy informational notices only at lifecycle check-ins; unmeasured branch; flaky ht-01s + two unclassified flakes) — code-final-review-8.md
metrics: docs/superpowers/runs/2026-10-09-agy-harness-adapter/upstream-feedback-draft.md (parked for the hand-back; 4 defects, 2 design questions, 1 doc gap)

> Reader note: the phase-6 sweep ran after final review 8 and passed at the tip: 4904 passed, 0 failed, 44 skipped, 272 s, host quiet, `@ 417c534f`. That resolves the review's "unmeasured branch" item. S1 and the flakes are still open. — run.md `codeBuckets.sweep`

Branch `super-auto/agy-harness-adapter` @ `9ec7da52` · base `main` @ `7c40c83c` (the merge-base; main's v0.6.0 and v0.6.1/v0.6.2 were absorbed). Run dir: `docs/superpowers/runs/2026-10-09-agy-harness-adapter/`.

## Adapter-model accounting (R13): what changed outside the agy adapter

**Short answer.** The adapter model held for agy, but only because a large neutral core was built first. On the corrected diff, the run changed 104 `src/` files outside `src/harness/agy/` (+7673/−3137). The adapter's own source is 11 files (+3799). After the neutral core landed, the agy leaf beads made three small production edits outside the adapter that the R13 target did not allow (+19/−4 lines). The target was missed narrowly, and on those three edits only. The bigger costs, each recorded as an R13 finding, were the neutral seams agy needed: `hook_step`, `local_evidence`, bootstrap V2 and HookPull.

**Correcting for final review 8's S2.** `outside-adapter-report.md` measures `f3431b31..1eaf635b`. That range includes main's own v0.6.1/v0.6.2 work: 11 commits, 43 files, +1365/−25. The figures below use `git diff --numstat main...HEAD`. Its merge-base is `7c40c83c` (main tip), so both base absorbs (`d342add2`, `cc885360`) fall outside the range. They are partitioned with the tool's own adapter rule from `docs/dev/outside-adapter-report.md`. — git diff; code-final-review-8.md S2

| Bucket (main...HEAD) | Files | +/− |
|---|---|---|
| adapter (tool rule: `src/harness/agy/**`, `tests/fixtures/agy/**`, `tests/harness/agy_*.rs`) | 80 | +9822/−0 |
| — of which `src/harness/agy/**` | 11 | +3799/−0 |
| outside: `src/` trait files (`adapter.rs`, `registry.rs`) | 2 | +590/−12 |
| outside: other `src/` | 102 | +7083/−3125 |
| — of which the other adapters (`src/harness/{claude,codex,hermes}*`) | 13 | +1567/−465 |
| outside: tests (non-agy) | 270 | +13920/−481 |
| outside: agy test infra (`scripts/agy-*`, `docs/validation/agy-native.md`, `docs/evidence/agy-*`) | 156 | +18327/−0 |
| outside: other scripts (neutral check, report tool, golden generator) | 6 | +1660/−0 |
| outside: `integrations/` non-agy (Claude mod, SKILL) | 9 | +606/−263 |
| outside: `integrations/agy/README.md` | 1 | +158/−0 |
| outside: docs/manifest/CI | 12 | +326/−51 |
| outside: run dir | 40 | +6236/−0 |

— git diff main...HEAD (partitioned by drafter); tool's own figures (uncorrected): adapter 81 / outside 631 files, outside-adapter-report.md §Summary

**(1) Trait extensions: was the adapter's scope extended? Yes. 13 new `HarnessAdapter` methods, plus 1 on `DeliveryPolicy` and 1 on an existing trait.**
- `local_evidence` → `LocalEvidence`/`RoleWitness::decide` (fails toward child). Added for agy, which has no payload role. Recorded as an R13 finding. `8a29a8d3` (ht-poh.4.1) — agy-adapter-design D4; root spec §Refinements
- `hook_step` → `HookStep`/`HookStepContext`/`HookStepDaemon`. Added for agy: the delivery leaf had no core files. Recorded as an R13 finding. `8a29a8d3` — agy-adapter-design D6; root spec §Refinements
- `delivery_policy` → `DeliveryPolicy{mechanism, provenance, idle_wake}`, `DeliveryMechanism{PushChannel, HookPull}`, `IdleWake`. A general neutrality seam: Claude moved onto PushChannel, agy is the first HookPull user. `bd838410` (ht-poh.3.1) — root spec §2
- `DeliveryPolicy::wake_text` (R18 adapter-declared wake text). General: the user asked for it mid-run, and Claude and agy both use it. `9be243ee` (ht-poh.14) — run.md `resumeChange`; root spec R18
- `text()` → `AdapterText`, plus `AdapterMetadata.agent_env_markers` (`AgentEnvMarker`). General neutrality for the core sweep: help, marker and doctor text. `3319387a` (ht-poh.2.1) — root spec §1 table
- `default_config_root`, `settings_kind` (`SettingsKindSpec`), `foreground_advice`, `setup_status_extras`, `setup_report_lines`. General (core sweep); `settings_kind` also gives agy its `permissions.allow` member kind. `3319387a` — root spec §1 table
- `doctor_text` (`DoctorTextForm`), `doctor_summary_claims`, `version_syntax`, `transcript_version`. General (core sweep). `3319387a` — root spec §1 table
- `permission_backend`. General: migrates main v0.6.0's new permissions core behind the adapter. `1ab0ce9b` (ht-poh.16) — run.md `baseAbsorbed`
- Not extended: `contract::classify` has no payload discriminator for agy. The adapter injects a synthetic `$event` key instead, so core was not edited. — root spec §Refinements (R13 note)
- Of these, `local_evidence`, `hook_step` and HookPull (inside `delivery_policy`) exist because of agy. `wake_text` came from the user's R18. The rest is the planned total core sweep. — specs; git log -S on `src/harness/adapter.rs`

**(2) Reworked implementations: yes, substantially. All of it was planned, and none of it was triggered by an agy leaf.**
- Claude mod re-fit onto PushChannel: every Claude check became a policy lookup, and the wire kept `not_claude` (allowlisted). `e4d97f02` (ht-poh.3.3), 14 src files +334/−98. — ledger Task 17; outside-adapter-report §By category
- R9 "byte-identical" was relaxed on purpose: R17 changed the frame text, and R18 changed `register.js`/`delivery.test.ts` (the idle path appends, then submits `wake up`). — integration-sweep.md Guards DP10; run.md `resumeChange`
- Total core sweep (ht-poh.2.1–2.8): setup engine, CLI help and markers, doctor projections (`cli/doctor.rs` +172/−718), hook runner, contract, attribution and evidence. The largest commit is `3319387a`, 31 src files +1424/−543. — ledger; git show --numstat
- Bootstrap plan V2 for `handoff --kind agy`: frozen V1 admits only Claude and Codex. `b754b8d2` (ht-poh.8), 12 src files +194/−27; plus `handoff --new-tab` V2, `85549582` (ht-poh.22), 5 files +124/−19. — core-sweep-design D16; root spec §Refinements
- Permissions-core migration of absorbed main v0.6.0: `1ab0ce9b` (ht-poh.16), 11 files +145/−64. Main v0.6.0's lazy hook delivery was reconciled with HookPull in ht-poh.17. — run.md `baseAbsorbed`; ledger Tasks 40-41
- Core delivery fixes that agy's HookPull exposed: at-least-once after daemon restart, and no content-free wake. `2eac6f51` (ht-poh.31), 8 src files +190/−10. — ledger Task 49

**(3) Callers touched.** 31 `src/` files outside the adapter are add-only (+3191). Most are new neutral modules: `src/delivery/*`, `src/service/delivery_pull.rs` (+472), `src/protocol/delivery.rs`, `src/cli/wake_on.rs`, `src/harness/{claude,codex}/doctor.rs`. 73 files carry removals (+4482/−3137). — git diff main...HEAD; the tool's split was "callers 20 files +1881", outside-adapter-report.md §By category

**(4) Allowlist and to-migrate counts.**
- At ht-poh.1 (`656a700`): 237 entries, of which 206 `to-migrate`, 22 registration, 8 frozen-wire, 1 history. — `git show 656a700:scripts/harness-neutral-allowlist.txt`
- At HEAD: 51 entries (26 frozen-wire, 24 registration, 1 history, 0 to-migrate); 85 allowlisted hits (49/34/2/0); 0 violations. ht-poh.2.8 forbids `to-migrate`. — outside-adapter-report.md §Neutral check counts; code-final-review-8.md
- The report's "allowlist delta" (+26/+1/+24) is measured against zero: the script did not exist at `f3431b31`. The real before-count is the 206 to-migrate above. — code-final-review-8.md S2

**(5) Edits outside the adapter by agy leaves after the neutral core landed.**
- Allowed by the target: the `builtins()` row plus `pub mod agy;` (`df5133c1`); registry and adapter test-module list updates (`df5133c1`, `3fb72774`); TRUST-POLICY, README and docs rows. — git show
- Not allowed: `src/harness/adapter.rs` refine now sets `metadata.context_source` (+8, `040a8639`, ht-poh.4.3). A neutral-core change, justified by a resume e2e failure. — ledger line 182 `[neutral-core-change]`
- Not allowed: `src/harness/launch.rs` `owned_launch_hook` appends the repair text for a disabled plugin (+8/−1, `0902d2e3`, ht-poh.4.6). — ledger line 196 `[outside-adapter-source]`
- Not allowed: `src/harness/setup.rs` allow-member writes switched from `write_replacement` to `write_user_config` (+3/−3, `a32dc4f3`, ht-poh.18). Forced by main v0.6.0's backup model. — ledger line 250 `[core-change-outside-brief]`
- Test-infra (R12, not leakage): `scripts/agy-fake-gemini/scenario.py` `FRAME_HEADER` changed outside the brief (ht-poh.6.6). — ledger line 297; test-infra-design D13
- Reverse coupling: the adapter imports core internals `crate::service::delivery_pull::PAGE_MAX_BLOCKS` and `crate::cli::hook::resolve_on_path`. — code-final-review-6.md S5
- **Verdict.** The R13 target ("zero edits outside `src/harness/agy/` beyond builtins row + TRUST/docs once the neutral core landed") was **not met, narrowly**. After the core landed there were 3 production edits outside the adapter, +19/−4 lines in 3 shared files, plus one test-infra edit. Every one was flagged as a deferred minor. The heavier agy-driven core work went into dedicated core beads before or alongside the leaves, not into the leaves: `hook_step`, `local_evidence`, bootstrap V2, `handoff --new-tab` V2, the HookPull at-least-once fix. — ledger; git show; root spec §6
- Trailer accounting is unreliable: 25 non-merge commits lack an `Outside-adapter:` trailer, and the tool lists 70+ including design and bookkeeping commits. The diff is authoritative. — code-final-review-8.md Deferred; integration-sweep.md finding 5

## Implemented

- Epic ht-poh closed: 114 beads, 0 open; every task bead closed, stopReason root-closed. — run.md `codeBuckets.completed`; bd list
- Neutral rule seam (ht-poh.1, `4c9a763..656a700`); AGENTS.md neutral rule, delivery contract and audit (ht-poh.7, `42593b0..4c9a763`). — ledger
- Core sweep ht-poh.2.1–2.8 (`45cfa65..2e58110`, `8094aa1..875ed18`, `8dd62ba..5e41774`, `4c5a503..99ce322`, `2e58110..6fd939b`, `6fd939b..1204875`, `135c59b..ef1a40b`, `c9cb053..dace1a6`). — ledger
- Delivery policy ht-poh.3.1–3.7: `src/delivery/`, DeliveryPolicy, Claude PushChannel re-fit, HookPull handlers, `wake_on` (migration 0029, `herdr-threads wake-on`), SKILL section. — ledger Tasks 6, 11, 17, 18, 7, 19, 20
- agy adapter ht-poh.4.1–4.8: seams, skeleton, witnesses, plugin, skill and allow entry, composer, launch and handoff, doctor, docs. ht-poh.5: HookPull delivery with transcript confirm and Stop downgrade (`09ea7ab..b711754`). — ledger
- Bootstrap V2 (ht-poh.8, `5d47379..e8fab61`) and `handoff --new-tab` V2 (ht-poh.22, `6e4d5f3..ae1e951`). — ledger
- Main v0.6.0 reconciliation: ht-poh.16 (permissions core), ht-poh.17 (lazy hook delivery vs HookPull), ht-poh.18 (agy allow entry under consent and backups). — ledger Tasks 40-42
- R17 compact cooperative framing plus the AGENTS.md "Token-efficient communication" rule. Landed via ht-poh.41 (`0291ece..20ec16e`) after the human ran `scripts/gen-frame-goldens.mjs` (gate ht-poh.19); ht-poh.13 closed as superseded. — run.md `codeBuckets.escalated`; ledger Task 53
- R18 minimal `herdr-threads: wake up` wake text, adapter-declared; Claude mod and agy inject the bodies (ht-poh.14, `9ed04fe..9b78cd8`); ht-poh.27 closed as superseded. — ledger Task 43; run.md
- `wake_on: all` (R11), with the frontier keyed on publication order (ht-poh.48, `e9786a40`). — ledger Task 60; roast-pr-2
- The user's decisions:
  - DC-3: ActiveTurn poke and `poke_during_turn` removed (ht-poh.42).
  - `unsetup agy` keeps the v0.6 settings backups (ht-poh.43).
  - Harness and Herdr version stance in AGENTS.md (ht-poh.44).
  - Daemon-down lifecycle notice matches Claude and Codex (ht-poh.47).

  — ledger Tasks 54, 55, 56, 59; root spec Post-Implementation Notes
- Round-1 fix beads: ht-poh.45 (genuine post-tool-call confirm capture), 46 (flaky tests: WAIT raised), 49 (attention-block `marker_prefix`), 50 (probation ends after one turn), 51 (relay reserves worst-case cost), 52 (docs describe shipped behaviour). — ledger; handoff-continuation.md
- Native gate: all 14 flows passed on genuine agy 1.3.2 with Herdr 0.9.1 at `c2f51b51`, the first run with R17 and R18 together. — `docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/summary.txt`; code-final-review-8.md
- Live smoke passed at `4d2ec663`: google/gemini-2.5-flash via the relay, 7 requests, $0.022; wake, injection, ACK and reply, with `cooperative_hook_delivery` settled. — `docs/evidence/agy-live-smoke/2026-10-10-4d2ec663dbfa/report.md`
- R13 accounting tool `scripts/outside-adapter-report` (ht-poh.9, `46e2ad7..bd130dc`); final-SHA evidence task ht-poh.6.9 (`94e73c1..417c534`). — ledger

## Remaining

- **S1 (Important), final review:** agy never sees informational notices (invitation rejections, overdue/cleared) between lifecycle check-ins. TRUST-POLICY A7, the SKILL and the agy README promise "next hook/check-in". Needs a fix or a recorded accepted limit. — code-final-review-8.md
- **S2, final review:** the R13 report range includes main's changes and has no before-count. This report corrects for it above; the run-dir file itself is not regenerated. — code-final-review-8.md
- **S3, final review:** no SKILL fallback for a bare `wake up` with nothing injected (prior F1). — code-final-review-8.md
- **S4, final review:** `DowngradeRecord.misses` collides across conversations; fails safe. — code-final-review-8.md
- **Flakes, final review:**
  - ht-01s `golden_surfaces::handoff_bootstrap_plans` (new on the branch, open);
  - two unclassified, with no bead: `cooperative_overdue_warning_wakes_idle_native_agent_exactly_once` and `pending_public_retry_from_different_valid_caller_is_silent`;
  - ht-veq, ht-vrg, ht-2w9 open, to classify against main.

  — code-final-review-8.md; bd show
- 20 round-1 punch-list items, **out of scope (filtered)**, each with its recorded reason in run.md `scopeFilter-round-1`: `src/delivery/ledger.rs:137`, `:96`; `src/harness/agy/delivery.rs:873/880`, `:647/644`, `:830`, `:325`, `:909` + `tests/harness/agy_delivery.rs:794`; `src/cli/hook.rs:2960`, `:2878`; `scripts/agy-sandbox/fetch-agy:124`; `src/cli/topology_handoff.rs:192`; `src/service/delivery_pull.rs:384`, `:297`, `:176`; `src/harness/agy/decode.rs:98`; `src/harness/agy/plugin.rs:60`; `src/protocol/delivery.rs:1/3`; `TRUST-POLICY.md:211`; `src/store/schema.rs:411/409`; `src/harness/agy/setup.rs:379`. — run.md; roast-pr-1-punch-list.txt
- 2 round-2 punch-list items, **out of scope (filtered)**:
  - `src/harness/agy/delivery.rs:913`: a fix-regression from `1693fc18`; another conversation's Stop can cancel a probation it did not open.
  - `tests/service/delivery_pull.rs:723`: the test never reaches the `!seen_current` guard.

  — roast-pr-2.md; roast-pr-2-punch-list.txt
- Dropped clusters, shared causes left unfixed. Each dropped cluster's rule is in run.md (`cluster dropped:`):
  - `agy-state-files`: one locked, no-fsync write helper for agy state files, with a rotating prune.
  - `ledger-monotonic`: no ledger write may lose offer accounting.
  - `hook-silent-failures`: `HookStep::run` returns a diagnostic.
  - `stale-docs`: one pass making every doc describe shipped behaviour. ht-poh.52 later applied part of this.
- Unreviewed conflict hunk: the `integrations/skill/SKILL.md` resolution in base merge `cc885360` (main v0.6.1/v0.6.2, after the fix loop exited). No roast reviewed it. — run.md `baseAbsorbed`; `git show cc885360`
- Kept worktree `task-ht-poh.13`: uncommitted partial R17 work (10 modified files), superseded by ht-poh.41. Remove it. The `task-ht-poh.4.5` entry no longer exists (merged). — run.md `codeBuckets.worktreesKept`; git worktree list; code-final-review-8.md
- Parked escalation: Herdr may report agy idle while a subagent generates (herdr #2354). It was patched in-design: the composer reads a live `Agent(<name>)` row as Unreadable. The human still needs to confirm on Herdr 0.9.1. — run.md `parked`
- Parked graph changes:
  - ht-poh.5 <- ht-poh.4.5 drop (super-code invocation 1);
  - ht-poh.2.7 <- ht-poh.7 drop;
  - ht-poh.4.4 <- ht-poh.3.6 drop;
  - proposal: `delivery::doctor_lines` seam;
  - proposal: `src/cli/hook.rs` runner-helper split.

  Graph pass: depth 11→11, applied 2, parked 4. All tasks have since closed. — run.md `parked`, `graph-pass`
- resumeChange follow-ups: none. All four (autonomy, R17, R18, the token-efficiency rule) were delivered. — run.md `resumeChange`; AGENTS.md:62
- Never exercised for real: 24 h probation retry, legacy probation record without `probation_at`, real `proc_pidpath`, Linux `/proc/<ppid>/exe`, `--sandbox`; the wake flow has single live passes only. — code-final-review-8.md Untested scope
- Process survivors: none (stopped 0, survived 0). — run.md `codeBuckets.processSweep`

## Gotchas & surprises

- **Real-HOME incident.** ht-poh.6.6's implementer ran `herdr-threads setup agy` against the real HOME, creating `~/.gemini/config/plugins/herdr-threads`, then ran `unsetup`. The orchestrator verified cleanup: `~/.gemini` absent, no agy setup manifest, no agy state files. — friction.md; code-final-review-6.md
- **Stale-brief pipeline defect.** A bead description update (the HT_AGY_BIN path on ht-poh.4.5) never reached the implementer, which reads the materialized plan section. It filed false blockers ht-cos and ht-5mi, then escalated. Fixed by patching the plan sections. — friction.md; ledger Task 26
- **Permission-classifier refusal.** `node scripts/gen-frame-goldens.mjs` for the user-directed R17 change was refused twice as "Security Weaken" (BLOCKED-AUTH, ht-poh.13). It went to human gate ht-poh.19, then re-dispatch ht-poh.41. — friction.md; ledger line 221
- **Two base absorbs:**
  - `f3431b31 → d342add2`: 14 conflicted files, main v0.6.0, mid-code. Spawned ht-poh.16, 17 and 18.
  - `7c40c83c → cc885360`: 1 file, SKILL.md, after the fix loop.

  — run.md `baseAbsorbed`
- **Parser-differential fix.** The agy transcript confirm scanned raw JSONL bytes instead of the decoded injected steps (ht-poh.25, `7f048d77`). — bd ht-poh.25; ledger Task 45
- **Resume-rebinding defect found by the native gate.** On genuine agy, A → B → `--conversation A` stayed bound to B: `witness.rs` `known()` trusted the per-conversation state file. ht-poh.28 BLOCKED, fixed by ht-poh.30 (`219aab34`). — ledger lines 320, 338
- **Design roast round 1** changed three things:
  - one delivery signal, also checked at Stop (cluster C1);
  - the executable identity comes from the hook's parent process image;
  - a busy-subagent wake guard.

  stepBackDesign-round-1 decided to patch. — root spec §Design roast round 1 fixes; run.md
- **R13 findings forced into design:** the `hook_step` seam, bootstrap V2, the `$event` key. — root spec §Refinements
- **Genuine-agy facts that diverged from the design:**
  - `agy -c` is `--continue`, not `--conversation`;
  - TUI Stop reports `NO_TOOL_CALL`;
  - post-tool-call injections needed a genuine capture (ht-poh.45).

  — root spec §Refinements; code-final-review-6.md S2
- **stepBackCode-round-1: patch.** 24 findings, 5 clusters, no recurrence. Round 2 converged with 2 Nits. — run.md; roast-pr-1-step-back.md; roast-pr-2.md
- **`service::loop_inventory` was red on the integration branch** after the probe moves; fixed inline in the integration sweep. — integration-sweep.md finding 6
- **Slowness:**
  - round-1 hot-file cap raised to 4;
  - round 2 bound by the dependency graph, with an edge audit;
  - two human-gated six-deep chains (ht-poh.4.5 and ht-poh.19 → … → 6.9);
  - the planner re-added edge 14<-13 onto a human-gated bead, and the bead was split to unblock it.

  — run.md `codeBuckets.slowness`; ledger edge audits; friction.md
- **A loaded-host sweep timed out** on `manifest::failpoints_are_compiled_out_of_release_builds` at 340 s; it passed alone in 98 s. — run.md `codeBuckets.sweep`
- **Bookkeeping stall:** ht-poh.6.6 merged, but its task and review bead stayed open. — friction.md
- **super-roast `assemble-args`** mangled `$\``/`$&` sequences; worked around by building the args in Python. — friction.md
- Profile bottleneck/what-if lines: none in the ledger. — progress.md

## Entrypoints

1. `AGENTS.md` (Harness-neutral core, Delivery contract, version stance, token-efficient communication), then `scripts/check-harness-neutral` and `scripts/harness-neutral-allowlist.txt`. — ht-poh.1, ht-poh.7; diff
2. `src/harness/adapter.rs`: the trait extensions, `DeliveryPolicy`, `HookStep`, `RoleWitness`. Then `src/harness/registry.rs`. — ht-poh.2.1, 3.1, 4.1; diff
3. `src/delivery/` (`frame.rs`, `page.rs`, `ledger.rs`, `settle.rs`, `routing.rs`), then `src/service/delivery_pull.rs`. — ht-poh.3.2, 3.4; diff
4. `src/cli/hook.rs`: the runner's `local_evidence` and `hook_step` call sites. — ht-poh.2.6, 4.1; diff
5. `src/harness/agy/mod.rs` → `witness.rs`, `delivery.rs`, `setup.rs`/`plugin.rs`, `launch.rs`, `composer.rs`, `doctor.rs`. — ht-poh.4.x, ht-poh.5; diff
6. `TRUST-POLICY.md` (A2/A4/A7/A8 hook-pull text), `integrations/agy/README.md`, `integrations/skill/SKILL.md`. — diff
7. `scripts/agy-gate/`, `scripts/agy-relay/`, `docs/validation/agy-native.md`, `docs/evidence/agy-*`. — ht-poh.6.x; diff

## Smells

- `src/harness/agy/delivery.rs:913`: the probation fix (`1693fc18`) introduced a cross-conversation re-downgrade. The smell: one identity-wide record mutated by any conversation's Stop. — roast-pr-2.md
- `tests/service/delivery_pull.rs:723`: the guard test never reaches the guard. The smell: a regression pin that removing the guard leaves green. — roast-pr-2.md
- `migrations/0029_seat_wake_on.sql:5` keeps the column name `last_lazy_ordinal` while storing `decision_seq`. The smell: two orderings, easy to confuse, renamable only until 0029 ships. — roast-pr-2.md spot-checked Nit
- ht-poh.2.7 and ht-poh.3.4 merged their fix passes without a second review. The smell: hook-stdout goldens pinned only against a daemon with no Herdr host; a TRUST-POLICY bullet re-nested. — ledger lines 51, 150
- ht-poh.46 raised the `handoff_topology_cli` WAIT from 15 s to 60 s. The smell: it masks the flakes, against the AGENTS.md speed-budget rule. — code-final-review-7.md F4; code-final-review-8.md
- The adapter imports `service::delivery_pull::PAGE_MAX_BLOCKS` and `cli::hook::resolve_on_path`. The smell: an adapter coupled to core internals. — code-final-review-6.md S5
- The Claude idle path settles at append, before any turn runs (R18). The smell: items can settle unseen after a `/clear` (documented in TRUST-POLICY). — code-final-review-6.md S4
- Deferred out-of-brief edits: `context_source` refine (ht-poh.4.3), `launch.rs` repair text (ht-poh.4.6), `setup.rs` `write_user_config` (ht-poh.18), `codex.rs` launch_notes (ht-poh.2.2), `scenario.py` FRAME_HEADER (ht-poh.6.6), `mod_delivery.rs` (ht-poh.49). The smell: shared-code changes beyond each brief. — ledger lines 115, 182, 196, 250, 297, 426
- `[tdd-order]` and `no-red-run`-style labels across about 20 tasks. The smell: there is no red-first evidence, and label fragmentation hides the rate. — code-final-review-6.md, code-final-review-8.md
- Native-gate record: leak check `LEAKS` (a dead-owner `herdr server`, pid 25162), followed up in `1f103389`. The smell: a foreign leak in an evidence record. — `docs/evidence/agy-native-gate/2026-10-10-c2f51b5121b7/summary.txt`; code-final-review-8.md Deferred
- Parked findings, degraded-verdict records, DONE_WITH_CONCERNS reports, regressionPass and sweepFix fixes: none. — run.md; ledger Metrics (parked 0)
