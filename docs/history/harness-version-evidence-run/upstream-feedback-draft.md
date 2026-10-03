# 2026-10-02-harness-version-evidence: round-2 roast counts punch-listed (never fixed) findings as "resolved"

Plugin: 6.4.2-alepar4.11 (session-loaded text 4.6; run followed the 4.11 cache files). Run: 23 beads under the epic, 2 design-roast rounds, 2 PR-roast rounds, 3 super-code launches (15 merges), autonomous, 2026-10-02.

## Defects
### 1. Round-N PR roast reports round-(N-1) punch-listed findings as "resolved"
- **Evidence:** scope-filter round 1: "scope-filter: 4 in-scope · 8 punch-listed"; only 5 fix beads filed. roast-pr-2: "delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 11 resolved · 1 regressed", listing punch-listed keys such as "src/cli/hook_evidence.rs:236 (single ok_sent_at_ms slot) — resolved" and "src/harness/state.rs:423 (newest_contract after downgrade) — resolved". The concurrent super-code final review still reports both as open. run.md needed a hand caveat: "round-2 'resolved' entries for punch-listed keys mean not re-surfaced, not fixed".
- **Premise to verify:** the round-N roast can receive the prior round's dispositions (in-scope / punch-list / rejected) from the caller, as it already receives the prior report.
- **Suggested fix shape:** split "resolved" into "fixed (a filed fix closed it)" and "not re-surfaced"; carry punch-listed keys as "punch-listed, open", outside the resolved count and the convergence test.

### 2. Scope filter can drop a whole step-back cluster with no clusterOverride
- **Evidence:** step-back cluster "hook-path-bounded: r1 [Nit] src/harness/attribution.rs:109; src/cli/hook.rs:2042 | rule: no file-system call on the hook path runs before the watchdog exists or outside the call budget …". Scope filter: `{"key":"[Nit] src/harness/attribution.rs:109; src/cli/hook.rs:2042","disposition":"punch-list","reason":"hook hardening; harness timeout caps damage"}`, no `clusterOverride` (four other cluster-splitting entries carry one). The fix-loop final review's top finding is on the same hook critical path.
- **Premise to verify:** a step-back cluster is meant to be overridden only with an explicit, recorded clusterOverride, including when all its members are punch-listed.
- **Suggested fix shape:** require a clusterOverride or a "cluster dropped: <name>" record when every member of a cluster is punch-listed, and surface dropped cluster rules in the report.

### 3. step-back-check splits list fields on "; " but multi-location keys contain "; "
- **Evidence:** keys such as "[Nit] src/harness/setup.rs:396; src/harness/setup.rs:404" and "[Should-fix] docs/install.md:184; docs/agent-usage.md:144". A `remains:` line joined with "; " was rejected; joining with ", " before each "r1 [" was accepted.
- **Premise to verify:** super-roast emits multi-location keys joined by "; ", and the checker splits on "; ".
- **Suggested fix shape:** use one list separator no key can contain (e.g. one key per line, or anchor on ", r<N> ["), consistently in the template, the checker and the docs.

## Run metrics
### Judge panel
- 2026-10-02-harness-version-evidence-roast-design-1.md: mode: design iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 34 · rr 0.71 · rg 0.79 · fg 0.50 · unanimous 0.50 · ground-loo 0.71 (n=24) · reproduce 13/21/0 · refute 3/31/0 · ground 20/14/0
- 2026-10-02-harness-version-evidence-roast-design-2.md: mode: design iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 12 · rr 0.42 · rg 0.92 · fg 0.33 · unanimous 0.33 · ground-loo 0.80 (n=5) · reproduce 8/3/1 · refute 2/10/0 · ground 9/2/1
- 2026-10-02-harness-version-evidence-roast-pr-1.md: mode: PR iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 25 · rr 0.64 · rg 0.76 · fg 0.64 · unanimous 0.52 · ground-loo 0.81 (n=16) · reproduce 13/12/0 · refute 8/17/0 · ground 15/10/0
- 2026-10-02-harness-version-evidence-roast-pr-2.md: mode: PR iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0

### Fix loop
- Metrics: completions — review clean 3 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 1 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
- Metrics: completions — review clean 10 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 3 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
- Metrics: completions — review clean 15 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 3 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
(three launches; counts are cumulative per launch)

### Merge-back
- Metrics: merges 3 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 1 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Metrics: merges 10 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 3 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Metrics: merges 15 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 4 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Merge: lines 15 · conflict 0 · seam-review fired 4 · check fail→fixed 0 · check fail 0 · → blocker 0

### Coverage
- coverage-round-1 · canonical R-list: R1-R12 (coverage-round-1-requirements.md; R13-R16 appended from r-new) · requirements: 12 · mapped: 12 · unmapped: 0
- coverage-round-2 · canonical R-list: R1-R16 (coverage-round-2-requirements.md) · requirements: 16 · mapped: 16 · unmapped: 0 · divergence: findings 11 → 8, novel 100%, widening: no
- scope-filter round 1: 4 in-scope · 8 punch-listed

### Bead graph
| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-xoc.22 | bug | Fix: loop_inventory test red on manifest.rs test-only wait_idle sleep | Covers: failing test cargo test --locked --all-features --lib loop_inventory (super-code final review, run ledger .superpowers/sdd/ht-xoc-pl |
| ht-xoc.19 | bug | Fix: recorder keeps the held SessionStart until it is stored (store-error-paths) | Covers: [Should-fix] src/daemon/harness_evidence.rs:162 |
| ht-xoc.18 | bug | Fix: Codex resumed sessions are never attributed (codex-resume-attribution) | Covers: [Should-fix] src/harness/attribution.rs:87 |
| ht-xoc.14 | bug | Fix: canary re-probes known_broken versions under main's contract | Covers: final review F3. In since-verified mode versions.candidates only probes versions above verified_max (taken across all contracts) and |
| ht-xoc.13 | bug | Fix: publish-manifest only from the default branch | Covers: final review F2. .github/workflows/harness-canary.yml publish-manifest job runs on schedule//workflow_dispatch without a ref check,  |
| ht-xoc.12 | bug | Fix: one settings.json schema for ServiceConfig and daemon::settings | Covers: final review F1 (Critical). src/daemon/settings.rs (ht-xoc.3) and src/service/config.rs (ServiceConfig::load, src/main.rs:31) both p |
| ht-xoc.21 | bug | Fix: canary last_working fallback names only a version below the broken one | Covers: [Nit] scripts/canary/manifest.py:306 |
| ht-xoc.20 | bug | Fix: user-visible docs match the foreign-session hook and Health behaviour (user-visible-docs-sync) | Covers: [Should-fix] docs/install.md:184; docs/agent-usage.md:144 |
| ht-xoc.17 | task | review: ht-xoc.5 | Review, fix and merge of ht-xoc.5, tracked separately so ht-xoc.5's dependents can start once it is implemented. The merge that lands ht-xoc |
| ht-xoc.16 | task | review: ht-xoc.4 | Review, fix and merge of ht-xoc.4, tracked separately so ht-xoc.4's dependents can start once it is implemented. The merge that lands ht-xoc |
| ht-xoc.15 | task | review: ht-xoc.2 | Review, fix and merge of ht-xoc.2, tracked separately so ht-xoc.2's dependents can start once it is implemented. The merge that lands ht-xoc |
| ht-xoc.11 | task | review: ht-xoc.6 | Review, fix and merge of ht-xoc.6, tracked separately so ht-xoc.6's dependents can start once it is implemented. The merge that lands ht-xoc |
| ht-xoc.10 | task | review: ht-xoc.1 | Review, fix and merge of ht-xoc.1, tracked separately so ht-xoc.1's dependents can start once it is implemented. The merge that lands ht-xoc |
| ht-xoc.9 | task | review: ht-xoc.3 | Review, fix and merge of ht-xoc.3, tracked separately so ht-xoc.3's dependents can start once it is implemented. The merge that lands ht-xoc |
| ht-xoc.8 | task | Spike: harness transcripts as the version source (Claude and Codex) | Redesigned after design roast r1 (docs/history/harness-version-evidence-run/2026-10-02-harness-version-evidence-roast-design |
| ht-xoc.7 | task | Integration sweep: harness version evidence end to end | End to end with a stand-in harness: an unlisted new version -> no Health line -> first lifecycle + tool payloads -> working; a payload missi |
| ht-xoc.6 | task | Canary manifest writer and harness-manifest branch publishing | Extend B6 canary: write rows to the harness-manifest branch by direct bot commit (contents: write for that branch only); only payload-contra |
| ht-xoc.5 | task | Contract-first state derivation and Health/doctor rendering | One pure function: local violation -> broken; ladder Refused -> broken; local verified -> working (manifest known_broken doctor-only); manif |
| ht-xoc.4 | task | Evidence store and verified-by-use recording | Migration adding harness_version_evidence(harness, version, contract_id, lifecycle_ok_at, tool_ok_at, violation_at, violation_event, violati |
| ht-xoc.3 | task | Manifest: schema 2, embedded copy, cache and fetch policy | Upgrade B6's in-repo harness-versions.json generator to schema_version 2 (canary-only fields null); embed at build (release: harness-manifes |
| ht-xoc.2 | task | Seam contract: transcript version reader | Attribution per spec (redesigned): read the running harness's version from the payload's transcript_path — Claude: 'version' of the newest J |
| ht-xoc.1 | task | Payload contract declarations, contract_id and contract-id CLI | Per-harness contract as data (event kinds consumed, required fields, JSON types) derived from the parsers in src/harness/claude.rs and codex |
| ht-xoc | epic | Harness version evidence: no noise on routine harness upgrades | Implements docs/design/herdr-threads/2026-10-02-harness-version-evidence-design.md (branch harness-version-evidence). Verified-by-use harness v |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-xoc.17 | ht-xoc.5 | unstated |
| ht-xoc.16 | ht-xoc.4 | unstated |
| ht-xoc.15 | ht-xoc.2 | unstated |
| ht-xoc.11 | ht-xoc.6 | unstated |
| ht-xoc.10 | ht-xoc.1 | unstated |
| ht-xoc.9 | ht-xoc.3 | unstated |
| ht-xoc.7 | ht-xoc.1 | blocked-by ht-xoc.1: consumes all leaves (integration sweep) |
| ht-xoc.7 | ht-xoc.6 | blocked-by ht-xoc.6: consumes all leaves (integration sweep) |
| ht-xoc.7 | ht-xoc.2 | blocked-by ht-xoc.2: consumes all leaves (integration sweep) |
| ht-xoc.7 | ht-xoc.3 | blocked-by ht-xoc.3: consumes all leaves (integration sweep) |
| ht-xoc.7 | ht-xoc.8 | blocked-by ht-xoc.8: consumes all leaves (integration sweep) |
| ht-xoc.7 | ht-xoc.4 | blocked-by ht-xoc.4: consumes all leaves (integration sweep) |
| ht-xoc.7 | ht-xoc.5 | blocked-by ht-xoc.5: consumes all leaves (integration sweep) |
| ht-xoc.6 | ht-xoc.3 | blocked-by ht-xoc.3: consumes manifest schema 2 |
| ht-xoc.6 | ht-xoc.1 | blocked-by ht-xoc.1: consumes contract-id CLI |
| ht-xoc.5 | ht-xoc.4 | blocked-by ht-xoc.4: consumes evidence rows |
| ht-xoc.5 | ht-xoc.1 | blocked-by ht-xoc.1: consumes contract_id |
| ht-xoc.5 | ht-xoc.3 | blocked-by ht-xoc.3: consumes manifest model |
| ht-xoc.4 | ht-xoc.1 | blocked-by ht-xoc.1: consumes contract classifier and contract_id |
| ht-xoc.4 | ht-xoc.2 | blocked-by ht-xoc.2: consumes attribution function |
| ht-xoc.4 | ht-xoc.3 | blocked-by ht-xoc.3: consumes manifest fetch entry point (ensure_manifest) |
| ht-xoc.2 | ht-xoc.8 | blocked-by ht-xoc.8: consumes transcript evidence note (timing, field location) |
| ht-xoc.2 | ht-xoc.1 | blocked-by ht-xoc.1: consumes normalize_version (canonical version string) |
| ht-xoc | ht-p03.71 | blocked-by ht-p03.71: consumes B6 admission ladder merged into main (every leaf builds on it) |
| ht-xoc | ht-p03.23 | blocked-by ht-p03.23: consumes B6 Health version rendering and parse-failure channel |
| ht-xoc | ht-p03.14 | blocked-by ht-p03.14: consumes B6 harness canary workflow |

## Design questions
### A. How should super-auto consume a fix-loop re-entry's super-code final review?
This run ended with the re-entry's final review at NOT READY. Its top item ("evidence step on the hook critical path before observe_harness_in") is one PR roast round 1 rejected 3-0 (roast-pr-1: "src/cli/hook.rs:2100 … Rejected 3-0"), while the final review rates it Important, arguing from the spec ("the hook never blocks on it"). Phase-3's first final review was consumed as fix beads; the re-entry's was not; nothing reconciles the two verdicts, and the report carries only a degraded qualifier. For feeding it back: it is fresh evidence, and dropping it ships a known "Important". Against: it re-litigates what a grounded panel already adjudicated, and re-opening on every final review threatens convergence; a tiebreak would be needed (e.g. re-panel only items the panel rejected or never saw).
If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### B. Stale session-loaded skill text vs a current cache
Pre-flight cannot distinguish "session text stale, cache current" from "cache behind", and its only remedy is a restart. Here the Skill tool loaded 4.6 while 4.11 was installed; the run followed the 4.11 files by absolute path, recorded skillSource, and passed the same skillsRoot to the coordinator, which worked end to end. For allowing it: no context-losing restart. Against: the orchestrator's in-context guidance is still the stale text, so drift is possible unless every followed file is re-read.
If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### C. Spike beads that need a privileged/live run on the critical chain
A spike (live Codex hook capture) on the main chain was refused by the permission layer (BLOCKED-AUTH) and drained the first super-code launch ("ready-drained; completed ht-xoc.1, .3, .6; escalated ht-xoc.8"), needing a relaunch. The graph pass had proposed taking it off the chain but parked both proposals ("graph-pass: depth 5→5 · applied 0 · parked 2"). Should such spikes declare a read-only fallback up front, and should the graph pass auto-apply the split for them? For: one refusal cost a whole launch. Against: auto-applying seam contracts on an autonomous run risks a wrong contract, which is why they are parked by default.
If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps
### Unreproduced "env-dependent" test-failure dismissals pass task review
Two implementer reports dismissed failing tests as "env-dependent under deep TMPDIR" without reproducing them; reviews passed clean (launch 1: "review clean 3 · after fix pass 0"); the final review then found a Critical there (two loaders rejecting each other's settings.json keys → fix bead). Suggested: an unreproduced "env-dependent / flaky / pre-existing" dismissal is a blocking review item until the reviewer re-runs the target or shows it failing identically on base.

## Already fixed — do not re-litigate
none

## Not established
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.
- Whether the hook critical-path item (Design question A) is a real defect: the panel and the final review disagree; no latency measurement was taken.
- Parallelism: the super-code cap of 4 bound below ready count (5) and runtime slots (10) in the fix loop; single observation, not filed as a finding.
- Analyst model: opus.

## Verification bar
- Defect 1: a 2-round PR roast fixture where round 1's scope filter punch-lists keys and round 2's scouts don't re-surface them; the delta line must not count them as resolved.
- Defect 2: a scope-filter output that punch-lists every member of a step-back cluster without clusterOverride must be rejected or recorded as a dropped cluster.
- Defect 3: step-back-check on a record whose remains: holds two multi-location keys.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
