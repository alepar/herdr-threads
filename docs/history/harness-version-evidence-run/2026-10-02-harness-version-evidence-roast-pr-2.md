super-roast verdict: Nit (1 confirmed) [converged]
mode: PR        iteration: 2 of 3
profile (assumed): herdr-threads is a pre-release (v0.1.0, "not yet published") Herdr plugin that lets Claude Code and Codex agents in local terminal panes message each other. It is a single-maintainer, local-only developer tool: the daemon, hooks and SQLite store all run as the invoking user, and the spec's trust model is explicitly cooperative, same-user and advisory (the manifest is unsigned; local evidence always wins). There are no external users, no money and no irreversible data at stake; the evidence this feature records is advisory Health/doctor text, and rollback is a reinstall. Resilience, observability and cost findings are therefore down-weighted; correctness of the recorded verdicts and user-facing promises (docs, isolation claims) are what matter.
inputs: super-auto/harness-version-evidence@6ac6028e vs main@a7255713
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 11 resolved · 1 regressed (0 Blocking)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 1 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0

## Confirmed findings
- [Nit] integrations/claude/README.md:32 — The fix for the confirmed stale-docs finding (ht-xoc.20) missed integrations/claude/README.md:32. It still says the hook "exits 0 at once with no output" in any Claude session that is not a pane of the instance, but this diff makes that hook read stdin for up to 200 ms, write per-session gate files under <state>/harness/evidence and send an evidence note to a running daemon. (regressed: incomplete fix of the iteration-1 Should-fix docs finding at docs/install.md:184; docs/agent-usage.md:144 — those locations, the src/cli/setup.rs help text and the CHANGELOG entry were fixed; this one sentence was missed)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: integrations/claude/README.md:32 on the branch: "in any Claude session that is not a pane of that instance it exits 0 at once with no output." The same sentence exists on base a7255713 and the diff touches nothing under integrations/, so the diff made it stale rather than editing it. docs/install.md:184 was rewritten on this branch to say the foreign-pane hook "still reads the payload (for at most 200 ms) and ... sends one best-effort harness evidence note (300 ms budget) ...; its gate files live under `<state>/harness/evidence`". The behaviour change is in src/cli/hook.rs plus the new src/cli/hook_evidence.rs. The round-1 step-back note (docs/history/harness-version-evidence-run/2026-10-02-harness-version-evidence-roast-pr-1-step-back.md:6) prescribed grepping "exits 0 at once" across the docs; `grep -rn "at once" integrations` still matches this line. All three seats rate it Nit: one stale sentence, no functional effect. The gate re-ran the grep and confirmed the README line is the only remaining stale occurrence (docs/install.md:249 matches "at once" in an unrelated launch sentence).
  fix-shape hint: rewrite the README sentence to match docs/install.md:184 (foreign session: reads the payload for at most 200 ms, sends one best-effort evidence note, touches gate files under `<state>/harness/evidence`, prints nothing, starts no daemon, exits 0).

Prior confirmed findings now resolved (no scout re-surfaced them; 13/13 scouts returned, none dead):
- src/harness/attribution.rs:87 (Codex resume attribution) — resolved
- src/daemon/harness_evidence.rs:162 (held SessionStart lost on failed store write) — resolved
- src/cli/hook_evidence.rs:236 (single ok_sent_at_ms slot) — resolved
- src/harness/state.rs:423 (newest_contract after downgrade) — resolved
- scripts/canary/manifest.py:306 (last_working fallback not < v) — resolved
- scripts/canary/release_contract.sh:58; .github/workflows/harness-canary.yml:157 (infra failure collapsed to unsupported) — resolved
- src/harness/setup.rs:396; :404 (downgrade note for --event format) — resolved
- src/harness/attribution.rs:109; src/cli/hook.rs:2042 (blocking open on transcript_path, no watchdog) — resolved
- src/app.rs:1199; :1201 (evidence store read error hides Health limitation) — resolved
- src/harness/setup.rs:756; src/cli/doctor.rs:850 (Codex re-trust caveat) — resolved
- src/harness/attribution.rs:193; :209 (scan_tail re-parse cost) — resolved

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this iteration. The iteration-1 rejections stand unchanged; no scout re-surfaced any of them and no `previouslyRejected` packet arrived.

## Unverified nits (spot-checked)
- none this iteration (no spot checks ran: spot 0).

## Escalations (need human)
- none