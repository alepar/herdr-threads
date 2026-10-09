---
super-roast verdict: clean (0 nits) [converged]
mode: pr        iteration: 2 of 3
profile (assumed): herdr-threads is an internal developer tool: a same-user local Rust daemon plus CLI plugin for the Herdr terminal multiplexer, released publicly (v0.1.x) but run by individual developers on their own machines. The trust model is cooperative same-user (TRUST-POLICY.md is normative): no adversarial caller verification; the correctness bar is seat continuity, honest provenance and never moving or merging seats on heuristic evidence. Hermes support is a harness adapter driven by a Python plugin bridge. Blast radius is low: single operator, no external data, cheap rollback.
inputs: super-auto/hermes-global-enrollment@124277c3 vs main@e0f4c261 (merge-base)
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 2 resolved · 0 regressed (0 Blocking) · 2 punch-listed (open)
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, observability, testing, hygiene-docs, deploy-safety) · raw 1 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 1 · rr 1.00 · rg 0.00 · fg 0.00 · unanimous 0.00 · ground-loo 0.00 (n=1) · reproduce 0/1/0 · refute 0/1/0 · ground 1/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 1/0/0/1 · deploy-safety 0/0/0/0

## Confirmed findings
- none

Prior-report tracking (iteration 1 confirmed findings):
- [Should-fix] .config/nextest.toml:101 — punch-listed (open). Not re-surfaced this round; the caller's scope filter left it unfixed. It stays open and does not count as resolved.
- [Should-fix] docs/superpowers/runs/2026-10-09-hermes-global-enrollment/run.md:27 — resolved. The two sub-items the prior panel verified in the tree are fixed: docs/compatibility/README.md:38-41 now says a bare Hermes CLI session enrolls through the Hermes hooks with `launch --kind hermes` as the other path, and the "Nothing produces it yet" comment is gone from src/harness/adapter.rs (commit 6e5626bb). No scout re-raised the record this round. Note for the caller: run.md (rewritten in 124277c3) still reads "review: not ready", now naming the one-shot seat item (rejected this round, below), the concurrent start/first-turn UnknownOutcome retry (spot-rejected in iteration 1) and "branch unmeasured by full suite". That line is the workflow's own bookkeeping; it is not a finding against the tree.
- [Nit] tests/cli/hook.rs:8260 — punch-listed (open). Not re-surfaced this round; the caller chose not to fix it.
- [Nit] docs/compatibility/README.md:40 — resolved. The page now states the hook enrollment path (verified at HEAD, docs/compatibility/README.md:38-41).

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- TRUST-POLICY.md:497 — The Hermes enrollment rule and design doc never decide or record what happens to headless one-shot Hermes runs (`hermes -z`, `chat -q` non-TTY); they carry `platform == "cli"`, so a one-shot in an Unowned pane allocates a lasting seat with no accepted-limit entry. Rejected 2/3 (reproduce REJECT, refute REJECT, ground CONFIRM at Nit). Default route stands. The refute seat's evidence answers the minority: TRUST-POLICY.md:491-496 ("Global hooks enroll supported top-level Herdr sessions ... including sessions started outside `launch`") already makes seat creation for any top-level CLI session the stated contract, and the same happens today for `claude -p` / `codex exec`. Design D7 (design.md:221-223) scopes "global" to every Hermes CLI session whose profile has the plugin enabled; a one-shot is a top-level CLI session. The design roast (roast-design-1.md:42-43) already examined this exact case and rejected it, because telling headless from interactive would need TTY/ancestry heuristics the trust policy forbids, concluding "At most a documentation note that headless runs enroll too." Managed launch refusing oneshot (integrations/hermes/README.md:32-34) is a wrapper-support limit, not a conflict with global enrollment. The ground seat's CONFIRM concedes "There is no functional defect" and that what remains is an optional note; it cites nothing the majority's evidence does not address, so there is no material dissent. The external premise (oneshot.py:583 `platform="cli"`) was checked only on the local install (v0.21.4+canary), not the pinned 0.21.5, and no seat ran the isolated-Herdr spike; this does not change the outcome because rejection rests on the internal contract. The related iteration-1 rejection at src/cli/hook.rs:2476 (nested Hermes replacing another harness's binding) is not reopened: this finding explicitly does not re-argue it, and no fix changed the text that rejection rested on. Optional follow-up outside this review: one sentence in the Hermes bullet of TRUST-POLICY.md noting that headless/one-shot CLI runs enroll like any other top-level CLI session.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none
---