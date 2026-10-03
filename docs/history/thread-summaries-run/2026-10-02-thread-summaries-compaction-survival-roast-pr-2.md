super-roast verdict: Should-fix (1 confirmed) [converged]
mode: PR        iteration: 2 of 3
profile (assumed): Internal, local single-user developer tooling: a Rust daemon plus CLI coordinating coding agents in terminal panes. Same-user cooperative trust model per TRUST-POLICY.md, no network exposure. The SQLite store holds real thread history, so data loss matters. Reviewers ran focused tests, cargo check and clippy; the full suite runs once in phase 6.
inputs: super-auto/thread-summaries-compaction-survival@f4a513b2 vs main@826a8804
delta vs prior: 1 new confirmed (0 Blocking) · 10 carried (0 Blocking) · 9 resolved · 0 regressed (0 Blocking)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, dependency, hygiene-docs) · raw 2 → deduped 2 → panel 1 · spot 0 · promoted 1 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 2 · rr 1.00 · rg 0.50 · fg 0.50 · unanimous 0.50 · ground-loo 0.50 (n=2) · reproduce 1/1/0 · refute 1/1/0 · ground 2/0/0

**Prior-report tracking (iteration 1 confirmed findings).** Fix round r1 landed ht-1ip.46 to .52 (commits 01ebc0d6, f64f809a, c58fb12c, 1e969cab, e75fd9ac, c8c46586, 100d0a8e). The caller's r1 scope filter put 8 findings in scope and 11 on the punch list. No scout re-raised any prior finding this round, so no seat verified the punch-listed ones; they are tracked here, not re-entered under Confirmed findings.
- resolved: src/harness/composer.rs:122 (ht-1ip.46; Cargo.toml:27 `unicode-width = "=0.2.2"`, classification by display width).
- resolved: src/summary/identifiers.rs:105 (ht-1ip.48; identifiers.rs:10 `MAX_IDENTIFIER_BYTES = 160`, line 108 rejects longer tokens).
- resolved: src/protocol/wire.rs:15 (ht-1ip.49; wire.rs:20 `PROTOCOL_VERSION: u16 = 3`; docs/operations.md:82 protocol-3 skew entry).
- resolved: docs/agent-usage.md:73 (ht-1ip.50; agent-usage.md:75 documents `send --relays-user`, line 85 documents `summary THREAD` / `summary job` / `summary submit` and the catch-up hold, line 133 names the compact event). The fix left a sibling contradiction at line 126; see the new confirmed finding below.
- resolved: src/store/summary.rs:1548 (ht-1ip.51; summary.rs:1554 `m.kind == MessageKind::Ordinary && is_priority(...)`).
- resolved: src/summary/fold.rs:202 (ht-1ip.48; fold.rs:204 adds identifiers to rendered_bytes; test `rendered_bytes_counts_entries_and_identifiers_and_is_stable`).
- resolved: src/scheduler/mod.rs:259 (ht-1ip.47; mod.rs:279-283 filters `poke_admissible` before `.take(POKE_SEAT_LIMIT)`).
- resolved: tests/store/schema.rs:3278 (ht-1ip.52; commit subject "author_role backfill test covers the binding start bound and two bindings").
- resolved: src/protocol/output_compact.rs:336 (docs sweep in ht-1ip.50; agent-usage.md:5 now lists `[human] [relays user]` and the `deferred: recipient catching up (until HH:MMZ)` suffix in the row grammar).
- still-open (punch-listed by the r1 scope filter, not re-surfaced this round): src/store/wake.rs:657 — store/poke.rs:95 still has `(?3 IS NULL OR seat_id=?3)` on one shared statement.
- still-open (punch-listed): src/summary/render.rs:142 — protocol/summary.rs:222 still carries `text_ref` only on the instruction body.
- still-open (punch-listed): src/store/mod.rs:1909 — not re-checked this round.
- still-open (punch-listed): src/notification/policy.rs:313; src/notification/dispatch.rs:222 — dispatch.rs:225 still `(_, true) => return outcome(WakeOutcome::Unsafe)` with the Skip label dropped.
- still-open (punch-listed): src/cli/hook.rs:1406 — not re-checked this round.
- still-open (punch-listed): docs/evidence/summary-smoke/captures/state/codex-scratch-config.final.toml:11 — the file still holds 5 `[projects."..."]` tables.
- still-open (punch-listed): tests/integration/summary_flow.rs:36 — `ACK_DEADLINE_SECONDS` is still 3.
- still-open (punch-listed): tests/integration/summary_flow.rs:1186 — not re-checked this round.
- still-open (punch-listed): docs/operations.md:131 — grep for poke, catch-up or effective deadline in docs/operations.md still finds nothing (the protocol-3 skew entry was added, which is a different section).
- still-open (punch-listed, FYI, pre-existing on base): src/store/schema.rs:270 — one-way migrations; no fix was required for this change.

## Confirmed findings
- [Should-fix] README.md:149; docs/agent-usage.md:126 — The branch adds the Claude recipe `claude-hooks-2.1.287` (with compaction recovery) and the `summary` command family, but README.md's Supported configurations row and Summaries bullet, and the hook-admission sentence in docs/agent-usage.md:126, still describe the pre-branch state. docs/agent-usage.md now contradicts itself: line 126 says the hook parses only Claude 2.1.283 to 2.1.286, while line 133 says Claude SessionStart `compact` is admitted by the 2.1.287 recipe. (new this iteration; adjacent to the resolved docs/agent-usage.md:73 finding, whose fix commit f64f809a edited line 133 and left line 126 stale)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `git show 826a8804:src/harness/claude.rs` has no "2.1.287"; HEAD src/harness/claude.rs:87 defines `id: "claude-hooks-2.1.287"`, so the recipe is new on this branch. README.md is absent from `git diff 826a8804...HEAD`. README.md:149 reads "Recipe `claude-hooks-2.1.283` admits 2.1.283 to 2.1.286"; README.md:11 lists only 2.1.285 and 2.1.286; the Summaries bullet near README.md:135 still describes only a subagent reading `read THREAD --recent N` and does not mention `herdr-threads summary`. docs/agent-usage.md:126 says the payload is parsed only for "Claude Code 2.1.283 to 2.1.286 inclusive, recipe `claude-hooks-2.1.283`" (unchanged from the base file). docs/agent-usage.md:133 says the compact event is "admitted only by the Claude 2.1.287 recipe, `claude-hooks-2.1.287`". docs/operations.md:84 calls agent-usage.md authoritative. docs/install.md:12, docs/release.md:19 and integrations/claude/README.md:39 were updated to list 2.1.287, so the omission is an oversight, not a deliberate split. Reproduce seat rated it Nit (README support row may track natively validated versions); refute and ground seats rated it Should-fix on the self-contradiction in the authoritative page. Severity kept at Should-fix: the prior round rated the same page's drift Should-fix under this profile, and a reader on Claude 2.1.287 concludes from line 126 or the README that their version is not admitted.
  fix-shape hint: update the version range at docs/agent-usage.md:126 to name both Claude recipes, update README.md:149 and the Summaries bullet to mention the 2.1.287 recipe, `herdr-threads summary` and compaction recovery.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- src/harness/composer.rs:146 — The ht-1ip.46 fix hard-codes a Claude-only `Unsafe` branch in the generic composer parser while the Claude 2.1.287 recipe still declares `composer_stash: Supported`, so the clear/poke/retype path is unreachable in production and the native tests reach `clear_composer` directly. Rejected 2-1 (reproduce REJECT FYI, refute REJECT FYI, ground CONFIRM Nit). Facts verified by all three seats: composer.rs ~155-160 returns `Unsafe { CLAUDE_NOT_KNOWN_EMPTY }` for any non-empty Claude text; stash_composer (native.rs ~885-915) returns Failed before clear_composer; claude.rs:105 declares Supported; policy.rs:341-344 yields Stash only where Supported. The majority cites TRUST-POLICY.md:192, which states the limit in writing ("Claude's `composer_stash` declaration currently never stashes, because no Claude composer text is known to be a typed draft"), updated in the same change as AGENTS.md requires; the composer.rs comment gives the reason (prompt suggestions cannot be told from a typed draft without captured styling, ht-jf3); the outcome is identical to the proposed recipe change (poke skipped, WakeOutcome::Unsafe), and the refusal is fail-safe (no draft is typed over). Ground seat's Nit CONFIRM calls it a layering and dead-code smell and concedes it is "not a safety or correctness defect" and "a defensible design choice", so the dissent is answered by the majority's evidence. Remaining ask is a layering preference: move the decision to the recipe (`composer_stash: Unsupported`) and save one fence plus one read per skipped poke. New this iteration (arises from the ht-1ip.46 fix); not previously listed.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none
