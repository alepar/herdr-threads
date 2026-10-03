# Canary tier 0 evidence, 2026-10

- Date: 2026-10-02 (runs at 10:4x UTC)
- Runner: darwin/arm64 (macOS 26.6.2, Apple silicon), node v26.9.0, npm 11.19.1
- Canary commit: `b52eb7dbc7c1b6adbcaffda54de4c7fa8603041c` (task branch with ht-p03.14.6 merged; the full tier-0 check set)
- Commands (scratch output directories outside the checkout, redacted to `<scratch>` in the committed files):
  - `scripts/harness-canary.sh --model-tier off --harness claude --versions 2.1.283,2.1.284,2.1.285,2.1.286,2.1.287 --out <scratch>/hc-claude`
  - `scripts/harness-canary.sh --model-tier off --harness codex --versions 0.157.1,0.158.0,0.159.2,0.159.3,0.160.0 --out <scratch>/hc-codex --herdr-threads target/debug/herdr-threads`
- Newest as of that day (`npm view`): Claude Code 2.1.287 (latest/next tag), Codex 0.160.0 (greatest stable).
- `canary-report.json` merges the two reports (harness blocks concatenated); `summary.md` holds both per-harness tables.

## Result

Every probe passed on the first attempt, both harnesses verdict `all_pass`, exit 0. No tier-0 check failed on any listed or newest version, so no bd bug was filed (none needed).

One non-pass outcome: `t0.npm-shim-admission` is `warn` on Codex 0.159.2, 0.159.3 and 0.160.0 (pass on 0.157.1 and 0.158.0). Detail: with the npm shim first on PATH admission is `optimistic`, with the native binary `schema-matched, live-unverified` (E4). These three versions are not listed in the Codex recipe, so this is the designed unlisted-version behavior, not a failure. `t0.config-load`, `t0.schema` and `t0.launch-tables` are `skip` for Claude by design (Codex-only checks).

Admission notes: Codex 0.159.2/0.159.3/0.160.0 schema fingerprint equals the `codex-hooks-v1` recipe's (`sha256:86858f24...`), so hook schemas did not drift.

## What tier 0 proves

For each version in a throwaway HOME / config dir: the package installs and reports its version; `setup` writes the owned hooks and `unsetup` restores the pre-setup bytes; the admission ladder classifies the version as expected; a hook fires from a real harness launch (against a local stub, no key) and the capture parses with the herdr-threads payload parser; the launch flags herdr-threads uses are present in the harness help and the launch tables know every subcommand; the real `~/.claude`, `~/.codex` and aisw profiles were untouched (`t0.isolation`).

## What tier 0 does not prove

No model turn ran (`--model-tier off`; tier-1 checks skipped). So no context delivery to the model, no receipt, no transport behavior, no hook output application. A pass means the integration surface still installs and parses, not that a thread message reaches a model.

## Recipe evidence levels

No change to `src/harness/{claude,codex}.rs` was needed. Levels already match the evidence: Claude 2.1.284 and Codex 0.157.1 are `no_model` (hook input captured, no live receipt run), and 2.1.283, 2.1.285, 2.1.286 and Codex 0.158.0 are `live` backed by existing native evidence under `docs/evidence/` and `docs/validation/report.md`. This tier-0 run raises nothing to `live`; live levels are left to the native rerun (ht-p03.20). `docs/compatibility/harness-versions.json` regenerates byte-identical and its guard test `versions_json_matches_recipes` passes.
