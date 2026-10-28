super-roast verdict: Nit (1 confirmed) [converged]
mode: pr        iteration: 2 of 3
profile (assumed): Local developer tool (Rust CLI + daemon) used by coding agents on one user's machine. Internal blast radius: no external users, no real money or data at stake, easy rollback. Help and output text is read by models, so neutral wording matters. CI checks (check-harness-neutral and the help-page test) guard the neutral core, so a hole in a guard is a real gap rather than cosmetics, weighed against whether any current text passes through it.
inputs: harness-neutral-cli-text@de90d00ce vs main@71b3bc0c1
delta vs prior: 1 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 5 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs, observability) · raw 3 → deduped 2 → panel 1 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "full diff de90d00ce vs 71b3bc0c1 captured by assemble-args; all changed files in scope"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0
lane-yield (found/confirmed/unique/refuted): correctness 1/1/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 1/1/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · api-contract 0/0/0/0 · testing 1/0/0/0 · test-causality-lifecycle 0/0/0/0 · test-isolation-environment 0/0/0/0 · test-suite-cost 0/0/0/0 · hygiene-docs 0/0/0/0 · observability 0/0/0/0

## Confirmed findings            ← consumed by super-design, one task per finding
- [Nit] tests/cli/commands.rs:4776; tests/cli/commands.rs:4672 — The round-2 help test removes every bare occurrence of each adapter's config-root hint (CLAUDE_CONFIG_DIR, CODEX_HOME, HOME) and each HERDR_THREADS_<ID>_OPTS key from the whole rendered help page before scanning, so a core-authored help line whose only harness token is one of these identifiers now passes unflagged; round 1's whole-line verbatim exemption would have flagged it. (new this iteration; introduced by the round-2 fix for the prior per-line exemption finding) [lanes: correctness, simplicity-design]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: tests/cli/commands.rs ~4672 pushes each registration's bare `text.config_root_hint` ("CLAUDE_CONFIG_DIR" src/harness/claude.rs:884, "CODEX_HOME" src/harness/codex.rs:1346, "HOME" src/harness/agy/mod.rs:58) and every `registered_options_keys(registry, |_| true)` key into `supplied`. check_page (~4776) runs `text = text.replace(exact, " ")` over the whole page for each entry. Worked case from the seats: core line "Set CODEX_HOME first" becomes "Set   first"; `hits` finds nothing, although `names_harness` would have split CODEX_HOME into CODEX/HOME and flagged it. In b66fec8ff `adapter_owned` exempted a line only when the whole trimmed line occurred verbatim in adapter source. The in-test self-checks do not cover the shape: "codex: CODEX_HOME" and "HERDR_THREADS_CLAUDE_OPTS prepend; then Claude runs SessionStart" are each flagged by another token (codex, Claude, SessionStart). The awk scan cannot see `///` doc comments (script header), so this test is the only guard on clap help text. All three seats read code; none ran a probe. Today's goldens hold these identifiers only in adapter-supplied text, so no current violation exists.
  severity note: seats unanimous Nit. Kept at Nit: unlike the prior bead-id finding, no in-use text shape passes through the hole, and the composed ` / ` joins remain exempted correctly. The refute seat notes src/cli/help.rs:295 builds a filtered options list that differs from the full ` / ` join, so a fix cannot simply drop the bare keys without computing that filtered join.
  fix-shape hint: remove only the composed adapter-supplied strings (the `config_root_hints` ` / ` join and each options list as help.rs `options_list` renders it, including the filtered variant) instead of each bare identifier, and add a self-check input whose only harness token is a bare identifier (e.g. "Set CODEX_HOME first") that must be flagged.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- none this iteration. Prior rejections (iteration 1: `--agent-id` hide as surface change; adapter_owned substring provenance / KEYED table; duplicated MECHANISMS lists) were not re-surfaced and stand.

## Unverified nits (spot-checked)
- [Nit] tests/cli/commands.rs:4786 — The new in-test self-check in every_help_page_is_harness_neutral does not test any detector on its own: seven of eight inputs contain a registered harness id (so names_harness alone flags them), and the eighth ("see C1 and spec §2 (ht-xe3v)") trips design_ref, '§' and the bead rule together. Deleting the MECHANISMS check, the 3-5 character bead rule (the fix for the prior Should-fix), the design-reference check, the section-sign check, the C0/C1 control exemption (widening `.replace("C1 control", "")` to `.replace("C1", "")`) or the new camelCase splitting in names_harness, one at a time, leaves every self-check passing; today's clean help pages mean the full scan catches none of it either. Spot seat CONFIRM Nit: read the `hits` closure and all eight inputs; no CamelCase input, no negative case for the control exemption; scripts/check-harness-neutral has per-detector ProseLiteralTests, so the gap is only in the Rust help test; introduced by this change. Fix shape: one single-trigger input per detector ("see ht-xe3v", "spec §2", "per C1 rules", "the on_session_start hook", "ClaudeCode env") plus a negative input for the C0/C1 exemption.

## Prior confirmed findings (iteration 1) — status
- [Should-fix] scripts/check-harness-neutral:356; scripts/check-harness-neutral:354; tests/cli/commands.rs:4727 (bead ids only in three-character form) — resolved: the spot seat's evidence records the 3-5 character bead rule in both the awk script (test_bead_ids_of_three_to_five_characters) and the help test, with the four-character self-check ht-xe3v added. Its isolation weakness is the Unverified nit above.
- [Nit] src/cli/commands.rs:1288 (invented `harness-version normalize` examples) — resolved: not re-surfaced by any of 14 scouts over the full diff.
- [Nit] src/cli/commands.rs:962; spec A1 (hidden `--agent-id` text unreachable by the help test) — resolved: not re-surfaced.
- [Nit] tests/cli/commands.rs:4652 (one KEYED substring exempted a whole paragraph) — resolved: round 2 replaced line skipping with exact-string span removal (de90d00ce). The over-broad span set is the new confirmed finding above, a different defect, not a regression of this one.
- [Nit] scripts/check-harness-neutral:332 (prose test per physical line, not per literal) — resolved: not re-surfaced.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none