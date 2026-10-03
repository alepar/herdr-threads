# B6 Harness version canary: script, tiers, bisection and scheduled workflow (design)

Parent spec: [2026-10-01-remaining-herdr-threads-findings-design.md](./2026-10-01-remaining-herdr-threads-findings-design.md)
(§B6 Decision 4, §Configurations to exercise "Harness version set", §Explicit deferrals, §Follow-on) ·
Bead: `ht-p03.14` (epic, promoted at the root split) · Root epic: `ht-p03` · Mode B (autonomous,
caller-invoked).

## Goal

`scripts/harness-canary.sh` installs any published Claude Code / Codex version into a throwaway prefix
and isolated home, runs a no-model compatibility tier (plus a model tier when API keys exist), and,
when the newest version breaks, bisects to the exact first failing version with a confirmed
predecessor; a daily GitHub Actions workflow runs it and opens one deduplicated issue per
`(harness, first_bad)` naming the range and the evidence. An offline `--self-test` proves the bisection
(planted break found exactly; contradictions reported `inconclusive`), and every tier-0 check is
defined by an exact command decided here from the installed harnesses' real behavior.

Promotion rationale (seed): install harness, tier 0 (5 checks), tier 1 capture + gated Rust test,
bisect, report/exit codes, self-test stub, workflow + issue dedupe; the per-harness "model-free command"
and the launch-flag list were undefined in the parent and are decided here from evidence.

Ancestor goal (root `## Goal`, verbatim prefix): *herdr-threads at the integration tip is ready to tag
v0.1.0: … admits newer Claude Code / Codex versions honestly with a scheduled canary that pins down the
first breaking version …*

Owner intent (verbatim gist): versions keep updating continuously; optimistically assume new versions
work; a build-time or periodic job pulls fresh versions, tests, and if it breaks, figures out the exact
breaking version so an adapter can be added for that version range.

## Problem description

The parent fixed the canary's shape (inputs, tiers, bisect, outputs, workflow) but left four things
undefined that an implementer cannot guess: the model-free command that proves a harness "loads the
written config", the launch-flag list tier 0 checks, how the canary reads the admission classification,
and how a found break turns into an actionable record without automated code changes. It also assumed
the npm packages expose the harness binary directly, which is false for Codex (below).

## Evidence gathered (2026-10-01, this design pass)

All probes ran in a throwaway prefix under the session scratchpad with `HOME`, `CLAUDE_CONFIG_DIR`,
`CODEX_HOME` and `XDG_*` pointing inside it; nothing touched `~/.claude`, `~/.codex`, aisw profiles or
the shared Herdr server.

| # | Observation | Consequence |
|---|---|---|
| E1 | `npm view`: packages are `@anthropic-ai/claude-code` (bin `bin/claude.exe`, optionalDependencies `@anthropic-ai/claude-code-<platform>`, dist-tags `latest` 2.1.287, `stable` 2.1.285) and `@openai/codex` (bin `bin/codex.js`, optionalDependencies aliased to `@openai/codex@<ver>-<platform>`, dist-tags `latest` 0.159.3, `alpha` 0.161.0-alpha.13). | Version source is `npm view <pkg> versions --json`; Codex's list has 5,212 entries of which 197 are plain `X.Y.Z` (platform-suffixed and alpha builds must be filtered). |
| E2 | Claude: `bin/claude.exe` is replaced by the native Mach-O by the package's `postinstall` (`node install.cjs`), hard-linked to the platform package's `claude`. npm 11.19 printed an `allowScripts` warning but still ran it. | Install must verify the binary runs and fall back to running `install.cjs` once; a future npm that blocks scripts would otherwise leave a placeholder. |
| E3 | Codex: `.bin/codex` → `bin/codex.js` (Node shim); the native binary is `node_modules/@openai/codex-<platform>/vendor/<triple>/bin/codex` (same layout in 0.157.1 and 0.159.3). | The canary runs the native vendor binary (the layout the standalone installer gives users, and what the owner runs). |
| E4 | **Product defect:** `herdr-threads doctor` with the npm shim first on `PATH` reports codex 0.159.3 `refused` — "embedded hook schemas could not be fingerprinted (no embedded hook command schemas found)" — because `codex_schema` canonicalizes `codex` to `codex.js`; with the vendor binary first on `PATH` it reports `schema-matched, live-unverified` (`sha256:86858f24…`, recipe `codex-hooks-v1`). | npm-installed Codex users never get `SchemaMatched` (after ht-p03.13 they fall to `Optimistic`). Out of this epic's scope (product code owned by ht-p03.13's files); recorded in §Explicit deferrals for the coordinator and surfaced by a warn-level canary check. |
| E5 | Codex 0.159.3 `exec --dangerously-bypass-hook-trust` with no credentials fires the `$CODEX_HOME/hooks.json` SessionStart hook (payload: `session_id`, `cwd`, `hook_event_name`, `model`, `permission_mode`, `source:"startup"`) *before* any model request, then fails with 401 (default provider) or retries forever (`waiting for network`, local provider). | A model-free "harness loads the written config" check exists: start a headless run, wait for the SessionStart hook, kill the process group. |
| E6 | Claude 2.1.287 `-p` with `ANTHROPIC_BASE_URL` pointed at a local 401 stub and a dummy `ANTHROPIC_API_KEY` fires the `$CLAUDE_CONFIG_DIR/settings.json` SessionStart hook in about 1 s, then retries the stub. No request leaves the machine; no keychain or OAuth credential is used. | Same fire-and-kill check for Claude, fully offline after install. |
| E7 | `herdr-threads setup claude --harness-binary <npm claude 2.1.286>` with `--state-dir`/`--host-endpoint` pointing at canary paths writes the owned hooks into the isolated `settings.json`, never contacting Herdr; the hook command is the absolute path of the `herdr-threads` executable that ran setup. Replacing that file with a witness shell script after setup, a headless `claude -p` invoked the setup-written command (`… hook claude`) with the SessionStart payload on stdin. `unsetup claude` removed it and deleted the created `settings.json`. | Tier 0 proves *the setup-written* configuration loads and fires, without product changes: a witness shim at the hook command path records argv and stdin, then execs the real binary. |
| E8 | The owner's interactive shell defines `claude`/`codex` functions that call `aisw workspace check` and add flags. A test that typed `claude` inside a zsh tool call ran the function, not the binary, and the hook never fired. | The canary runs every harness by absolute path under `env -i` in non-interactive `bash`, never through a user shell. |
| E9 | `codex doctor --json` (0.159.3) reports `config.load ok` but has no hooks check (a malformed `hooks.json` still reports ok); `codex debug prompt-input` does not run hooks; `codex features list` fails with "failed to load bootstrap configuration" on an invalid `config.toml`. `--strict-config` exists in 0.159.3. | `config.toml` acceptance = `codex features list`; `hooks.json` acceptance = the fire-and-kill hook witness (E5). |
| E10 | `codex --help` 0.159.3 lists `--no-daemon`, `-c/--config`, `--dangerously-bypass-hook-trust` (exec), subcommands `exec` (alias `e`), `resume`, and 26 others; `codex exec --help` lists `resume`, `fork`, `review`, `help` and `-c/--config`. `claude --help` lists `-p/--print`, `--settings`, `--setting-sources`, `--output-format`, `--max-budget-usd`. | Launch-flag list (§D4). Codex's subcommand/value-option tables in `src/harness/launch.rs` mirror 0.159.2's help and drift with new releases — a warn-level check compares them. |
| E11 | Release cadence: Claude Code published 2.1.280–2.1.287 one per day (2026-09-22 → 10-01); Codex 0.157.0–0.159.3, seven stable releases in six days. Install sizes ≈ 228 MB (claude), ≈ 133 MB tarball (codex); `npm install` 17 s / 31 s locally. | A daily canary has 1–7 candidates per harness; bisection costs ≤ ⌈log₂ n⌉ + 3 probes. |
| E12 | Installed today: Claude Code **2.1.287** (newer than the newest listed recipe 2.1.286) and Codex 0.159.3. | The parent's Configurations row "newest published > 2.1.286" already exists (2.1.287) and is what the owner runs. |
| E13 | Evidence procedures for model runs: `docs/evidence/claude-286-hook-capture/run.sh` (`-p … --model claude-haiku-4-5-20251001 --max-budget-usd 0.10 --setting-sources project,local --output-format json`), `docs/evidence/codex-158-live-hook-capture/run1.sh` (`--no-daemon exec --ephemeral --ignore-user-config --dangerously-bypass-hook-trust --json --skip-git-repo-check -s read-only -m gpt-6-luna -c model_reasoning_effort="low" -c hooks.<Event>=…`) and `inject.py` (`hookSpecificOutput.additionalContext` marker). | Tier 1 reuses these exact, already-proven invocations. |

## Main challenges

- **No model in the always-on tier**, yet it must prove the harness actually loads the configuration
  setup writes. Solved by E5–E7: both harnesses fire SessionStart before the first model request.
- **Isolation on the owner's machine and in CI** with one code path: a fresh `env -i` environment per
  probe, every home variable inside the probe directory, and no user shell (E8).
- **Bisection over a noisy oracle**: installs and model runs fail for reasons unrelated to the version;
  the algorithm must retry, confirm, and say `inconclusive` instead of guessing.
- **Feeding a break back without CI writing code** (parent deferral): an issue carrying the exact
  range, the failing checks, the evidence and a paste-ready `known_broken` suggestion.

## Key decisions made

1. The canary is one bash orchestrator plus small Python 3 helpers (JSON, version math, bisection,
   report) — Python is already a CI and repo dependency (validators, fixtures); no `jq`.
2. Tier 0 has **seven failing checks and three warn-only checks** per probe (§D3), every one an exact
   command; "loads the written config" = the setup-written hook fires model-free under a witness shim.
3. The canary reads admission from `herdr-threads doctor --json`, `hooks.<harness>.installed.admission`
   (Codex today; Claude via ht-p03.23's doctor PATH check — a consumed contract, §Consumed contracts).
4. One gated Rust test, `tests/harness/canary_payloads.rs`, parses every captured payload (tier 0 and
   tier 1) through the hook's own `observe_harness_in` + `parse_event` path and reports the Codex
   schema fingerprint and launch-table drift; it is inert unless `HT_CANARY_CAPTURE_DIR` is set.
5. Bisection = newest-first, binary search assuming monotonicity, then a fresh-install confirmation of
   `first_bad` (fails) and its predecessor (passes); any contradiction → `inconclusive` with every probe.
6. Exit codes 0/1/2 as the parent; `inconclusive` exits 1. The workflow files issues from the report
   (not the exit code), one per `(harness, first_bad)` title, commenting only when the evidence digest
   changes; never auto-closes, never opens PRs.

## Scope

`scripts/harness-canary.sh`, `scripts/canary/*` (helpers, witness shim, capture hook, local stub,
launch-flag table), `scripts/harness-canary-selftest/*`, `tests/harness/canary_payloads.rs` (+ its
`#[path]` mount in `src/harness/mod.rs`), `.github/workflows/harness-canary.yml`, the canary section of
`docs/compatibility/harnesses.md`, and the committed tier-0 evidence for the listed versions with the
recipes' evidence levels.

## Non-goals

- Fixing E4 (npm-shim fingerprinting) — product code in ht-p03.13's files; deferred to the coordinator.
- A fake-model stub that drives PreToolUse without a model (see deferrals).
- Auto-generated adapter PRs; auto-closing issues; Renovate-style bump PRs (parent decisions).
- Running Herdr. The canary never starts, contacts or needs Herdr — not even the ht-p03.1 isolated
  session: setup/doctor take `--host-endpoint <probe>/herdr.sock` (a path that never exists) and every
  `HERDR_*` variable is absent, so the hook stays silent after the witness has recorded it.
- macOS runners in the scheduled job (ubuntu-24.04 per parent; `workflow_dispatch` input can pick
  `macos-15`).

## Consumed contracts

| From | Artifact | How the canary uses it |
|---|---|---|
| ht-p03.13 | `docs/compatibility/harness-versions.json` (fields `harness`, `version`, `recipe`, `evidence`, `known_broken`) | `verified_max` = greatest listed version per harness; `known_broken` ranges; expected admission (§D3 `t0.admission`). |
| ht-p03.13 (Amended by coverage r2, 2026-10-01) | Exact shape of the file: `{"schema_version": 1, "generated_from": …, "rows": [{"harness", "version", "recipe", "evidence", "known_broken": [{"min": "X.Y.Z"\|null, "max": "X.Y.Z"\|null}]}]}`; recipes hold `known_broken: &'static [VersionSet]` | `known_broken` is a list of inclusive `{min,max}` ranges, `null` = open end; §D7 and §D8 read and emit exactly this. |
| ht-p03.13 | `Admission::Optimistic` and its doctor state string `optimistic` | Expected admission for versions above `verified_max`. |
| ht-p03.15 | Setup/unsetup behavior on a fresh isolated home | `t0.setup`, `t0.unsetup`. |
| ht-p03.28 | Launch flag set and argv handling (`--no-daemon` once, `-c hooks.*` at the subcommand level; nothing added for Claude) | `t0.launch-flags` list (§D4). |
| ht-p03.23 | `doctor --json` → `doctor.hooks.claude.installed.{binary, version, admission, recipe}`, mirroring today's `doctor.hooks.codex.installed` | `t0.admission` for Claude. Field names are this design's request of ht-p03.23 (comment added on that bead). |
| ht-p03.16 | SHA-pinning convention `uses: <owner>/<action>@<40-hex sha> # <tag>`, actionlint | The canary workflow (no edge; convention, not artifact). |

## Decision points

### D1. Version source and candidate list

- `npm view <pkg> versions --json` (registry `https://registry.npmjs.org/`, 3 retries, 60 s each).
  Packages: `claude` → `@anthropic-ai/claude-code`, `codex` → `@openai/codex` (E1).
- Stable filter: `^(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})$` — the product's own
  canonical `Version::parse` rule — sorted numerically. This drops `-alpha.*`, `-<platform>` and
  pre-release builds.
- `--versions latest`: the single greatest stable version (not `dist-tags.latest`, which can lag or
  point at a tagged channel). `--versions since-verified`: every stable version `> verified_max`
  (from harness-versions.json). `--versions <list>`: comma-separated exact versions, each must exist in
  the registry list (else exit 2). No candidates → harness status `no_candidates`, exit 0.
- **`--versions-json <path>`** (Amended by coverage r2, 2026-10-01, ht-p03.14.2/.14.9): overrides the harness-versions.json read for `verified_max` and `known_broken` (default `docs/compatibility/harness-versions.json`), so a known_broken refusal can be exercised before any real break exists. It feeds `canary-probe.json` and `versions.py`; it never moves the bisect baseline (only `--baseline` does).
- **known_broken exclusion** (Amended by coverage r2, 2026-10-01, ht-p03.14.1): candidates inside a `known_broken` range are excluded from the search and listed in the report; §D7 gives the moving-baseline rule.

### D2. Install and isolation (per probe)

Probe directory `P = <out>/work/<harness>-<version>-<attempt>/` (deleted after the probe unless
`--keep`); everything a probe touches is under `P`:

- `npm install --prefix P/npm --no-audit --no-fund --no-save <pkg>@<version>` with
  `npm_config_cache=<out>/npm-cache` (shared across probes in one run, never `~/.npm`),
  `npm_config_update_notifier=false`, under the isolated environment below.
- Binary resolution: Claude `P/npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe`; if
  `--version` fails, run `node install.cjs` in that package once (E2), then re-check. Codex: the single
  file matching `P/npm/node_modules/@openai/codex-*/vendor/*/bin/codex` (E3); zero or several matches →
  infra error. The resolved binary is symlinked as `P/bin/<harness>` — `P/bin` is the only harness
  directory on `PATH`.
- Environment: every harness, `herdr-threads` and `npm` invocation runs as
  `env -i HOME=P/home CLAUDE_CONFIG_DIR=P/home/.claude CODEX_HOME=P/home/.codex XDG_CONFIG_HOME=P/home/.config XDG_STATE_HOME=P/home/.local/state XDG_DATA_HOME=P/home/.local/share XDG_CACHE_HOME=P/home/.cache TMPDIR=P/tmp PATH=P/bin:<node dir>:/usr/bin:/bin LANG=C.UTF-8 TERM=dumb`
  plus, only for tier 1, the one API key that harness needs. All directories are created first
  (Codex warns when `CODEX_HOME` is missing, E5). No `HERDR_*`, `CLAUDE*`, `CODEX_*` or `BASH_ENV`
  variable crosses in (E8); harnesses are executed by absolute path from non-interactive `bash`.
- The canary refuses to start (exit 2) if `--out` resolves inside `$HOME/.claude`, `$HOME/.codex`, an
  aisw profile directory, or the repository's tracked tree, and asserts after every probe that the real
  `$HOME/.claude/settings.json` and `$HOME/.codex/hooks.json` mtimes are unchanged (a cheap tripwire,
  check id `t0.isolation`, failing → infra error, never a version break).
- Process control: every harness run is started in its own process group (`python3 scripts/canary/run.py
  --timeout S --until-file F -- argv…`), killed as a group at the deadline or when the awaited file
  appears; no orphan survives a probe.
- `herdr-threads` under test: `--herdr-threads <path>` or, by default, `cargo build --locked` once per
  run; each probe copies it to `P/ht/herdr-threads` so the setup-written hook command points inside `P`.

### D3. Tier 0 (no model, always) — exact checks

Run in this order per probe; `S = --state-dir P/state --host-endpoint P/herdr.sock --json`. Status per
check: `pass | fail | warn | skip`. Any `fail` fails the probe.

| Check id | Level | Command / assertion |
|---|---|---|
| `t0.version` | fail | `P/bin/<h> --version` exits 0 within 15 s; stdout matches `^X.Y.Z \(Claude Code\)$` (claude) or `^codex-cli X.Y.Z$` (codex) and X.Y.Z equals the installed version. |
| `t0.setup` | fail | `P/ht/herdr-threads S setup <h> --harness-binary P/bin/<h>` exits 0 with `setup.action == "installed"`, `setup.harness_version.version == X.Y.Z`; Claude: `P/home/.claude/settings.json` has the owned `SessionStart` and `PreToolUse`(`Bash`) groups; Codex: `P/home/.codex/hooks.json` has `SessionStart`, `SubagentStart`, `PreToolUse` groups. |
| `t0.admission` | fail | `P/ht/herdr-threads S doctor` → `doctor.hooks.<h>.installed.admission` is, evaluated in the root §B6 D2 ladder's first-match order (amended by design roast round 1): `refused` if inside a `known_broken` range (even when also listed); `listed` if the version is listed in harness-versions.json; for a version `> verified_max`, `optimistic` (claude) or `optimistic` / `schema-matched, live-unverified` (codex; must be `schema-matched…` iff `t0.schema` reports `match`); otherwise recorded, not asserted. (Amended by coverage r2, 2026-10-01: for Codex above `verified_max` the verdict is finalized after `t0.schema` runs — `t0.admission` resolves `expected_admission = schema-matched-or-optimistic` against `t0.schema`'s `match`/`drift`/`unextractable`, so `t0.schema` is evaluated first for the verdict even though it is a warn-level check; the closed `expected_admission` set in `canary-probe.json` is `listed \| optimistic \| schema-matched-or-optimistic \| refused \| unasserted`, ht-p03.14.8.) |
| `t0.config-load` | fail | Codex: `P/bin/codex features list` exits 0 (config.toml as setup left it, E9). Claude: covered by `t0.hook-fires` (settings.json is only read by a session). |
| `t0.hook-fires` | fail | After setup, `P/ht/herdr-threads` is replaced by `scripts/canary/witness.sh` (records argv + stdin into `P/capture/tier0/<n>.{argv,stdin}`, then `exec`s the real binary). A local stub (`scripts/canary/stub_api.py`, answers every request 401) is started on 127.0.0.1. Claude: `ANTHROPIC_BASE_URL=http://127.0.0.1:<port> ANTHROPIC_API_KEY=canary-invalid CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 P/bin/claude -p canary </dev/null` (cwd `P/proj`). Codex: `P/bin/codex --no-daemon exec --ephemeral --dangerously-bypass-hook-trust --skip-git-repo-check -s read-only -c 'model_providers.canary={name="canary",base_url="http://127.0.0.1:<port>/v1",wire_api="responses"}' -c model_provider=canary canary </dev/null`. Pass when a capture whose argv ends `hook <h>` and whose stdin is a JSON object with `hook_event_name == "SessionStart"` and `source == "startup"` appears within 30 s; the run is then killed. |
| `t0.payload-parse` | fail | `HT_CANARY_CAPTURE_DIR=P cargo test --locked --lib canary_payloads -- --nocapture` (§D5) passes for every captured payload. |
| `t0.launch-flags` | fail | Every flag in `scripts/canary/launch-flags.tsv` (§D4) appears in the named help text. |
| `t0.unsetup` | fail | `P/ht/herdr-threads S unsetup <h>` exits 0 with `setup.action == "removed"`; Claude `settings.json` absent again (setup created it); Codex `hooks.json` contains no `herdr-threads-owner:` command and `config.toml` equals its pre-setup bytes (absent, on a fresh home). |
| `t0.schema` | warn (codex) | The gated test's `schema` result for `P/bin/codex`: `match` (fingerprint equals a recipe's), `drift` (differs — the leading signal that an adapter is needed) or `unextractable`. `drift`/`unextractable` → warn. |
| `t0.npm-shim-admission` | warn (codex) | `doctor` again with `P/npm/node_modules/.bin` first on `PATH`; warn when its admission differs from `t0.admission`'s (E4 visibility until fixed). |
| `t0.launch-tables` | warn (codex) | The gated test's `launch_tables` result: top-level subcommands/aliases in `codex --help` absent from `launch.rs`'s known tables, and value-taking options (`<…>` in help) absent from `CODEX_VALUE_OPTIONS` (E10). |

A probe also records `t0.isolation` (§D2). The parent's "5 checks" map as: version+admission →
`t0.version`/`t0.admission`; setup/unsetup → `t0.setup`/`t0.unsetup`; loads config →
`t0.config-load`/`t0.hook-fires`/`t0.payload-parse`; fingerprint → `t0.schema`; launch flags →
`t0.launch-flags`.

**Why the Codex fingerprint is warn-only.** Under the ladder, drift does not refuse — it moves Codex from
`SchemaMatched` to `Optimistic`, which `t0.admission` already asserts. Drift with every failing check
passing is a reason to look, not a break; the summary lists it under "Signals" and the workflow puts it in
the job summary but files no issue.

### D4. Launch-flag list

`scripts/canary/launch-flags.tsv`, columns `harness  help-command  flag  source`:

| harness | help command | flag | why (source) |
|---|---|---|---|
| codex | `codex --help` | `--no-daemon` | launch adds it once at top level (`harness/launch.rs` compose, `setup.rs` launch_argv) |
| codex | `codex --help` | `--config` | owned `-c hooks.*` for the interactive form |
| codex | `codex --help` | `exec` | handled launch form |
| codex | `codex --help` | `resume` | handled launch form |
| codex | `codex exec --help` | `--config` | owned `-c hooks.*` at exec level (0.159.2 ignores root-level hooks for exec) |
| codex | `codex exec --help` | `resume` | handled `exec resume` form |
| codex | `codex exec --help` | `--dangerously-bypass-hook-trust` | canary tier 0/1 depend on it |
| codex | `codex resume --help` | `--config` | owned `-c hooks.*` at resume level |
| claude | `claude --help` | `--settings` | the launch proposal `setup` prints (`JsonSetupPlan::launch_argv`) |
| claude | `claude --help` | `--print` | tier 0/1 headless run |
| claude | `claude --help` | `--setting-sources` | documented user/project layering in setup's note |

Claude launch adds no harness flag (owned configuration is on disk; the Herdr agent name is a Herdr
argument, ht-p03.28). An ungated unit test in `canary_payloads.rs` asserts every flag the Rust
launch/setup code emits (`--no-daemon`, `-c`, `--settings`) has a TSV row, so the list cannot silently
lag the code.

### D5. The gated Rust test

`tests/harness/canary_payloads.rs`, mounted from `src/harness/mod.rs` with
`#[cfg(test)] #[path = "../../tests/harness/canary_payloads.rs"] mod canary_payloads;` like its
siblings. With `HT_CANARY_CAPTURE_DIR` unset, the gated test returns immediately (passes); the
ungated launch-flag-table test always runs.

Capture-directory contract (written by the script, read by the test):

```
<P>/canary-probe.json        {"harness":"codex","version":"0.159.3","binary":"<P>/bin/codex"}
<P>/capture/tier0/*.stdin    raw hook stdin (witness)          (+ .argv)
<P>/capture/tier1/*.json     {"event":..., "stdin":<raw string>, "stdout":...} (capture hook)
<P>/help/<cmd>.txt           help texts for launch-tables (codex --help, codex exec --help)
→ <P>/canary-rust.json       written by the test: per-payload parse results, schema, launch_tables
```

For each payload: observe the harness exactly as the hook does — `cli::hook::observe_harness_in(harness,
PATH=dirname(binary), budget, None)` — then `cli::hook::parse_event(&installed, bytes)`. Any parse error,
or an observation refusal for a version the canary expected to be admitted, fails the test (and so
`t0.payload-parse` / `t1.payload-parse`). For Codex it also computes the production
`codex_schema` fingerprint of `binary` and compares it with every recipe's `schema_fingerprint`, and
reads `help/*.txt` for `launch_tables`; both are reported in `canary-rust.json`, never asserted.

### D6. Tier 1 (model) — when keys exist

`--model-tier auto` runs tier 1 for a harness only when its key is non-empty (`ANTHROPIC_API_KEY` /
`OPENAI_API_KEY`); `off` never; `required` makes a missing key an infra error (exit 2). Tier 1 runs on a
**fresh** probe home without `herdr-threads setup` (so outputs are not confounded by the product hook),
with capture hooks supplied per invocation exactly as the evidence runs did (E13):

- Capture hook `scripts/canary/capture_hook.py <dir> <nonce-file>`: writes `{event, stdin, stdout}` to
  `<dir>`, and prints `{"hookSpecificOutput":{"hookEventName":<event>,"additionalContext":"canary-nonce-<event>: <N>"}}`
  with a per-probe random nonce for SessionStart and PreToolUse.
- Claude: settings file in `P/proj/.claude/settings.local.json` with SessionStart and `PreToolUse`
  (`Bash`) capture hooks and `permissions.allow: ["Bash(echo canary-ok)"]`; run
  `claude -p "<prompt>" --model ${HT_CANARY_CLAUDE_MODEL:-claude-haiku-4-5-20251001} --max-budget-usd 0.10 --setting-sources project,local --output-format json`.
- Codex: `printenv OPENAI_API_KEY | codex login --with-api-key` into `P/home/.codex`, then
  `codex --no-daemon exec --ephemeral --ignore-user-config --dangerously-bypass-hook-trust --json --skip-git-repo-check -s read-only -m ${HT_CANARY_CODEX_MODEL:-gpt-6-luna} -c model_reasoning_effort="low" -c hooks.SessionStart=… -c hooks.PreToolUse=… "<prompt>"`.
- Prompt: "Run the shell command `echo canary-ok`. Then reply with every line that starts with
  `canary-nonce-` from your context, verbatim, and nothing else."
- Checks: `t1.session-start` (SessionStart payload captured), `t1.pre-tool-use` (Bash PreToolUse payload
  whose command contains `echo canary-ok`), `t1.context-delivery` (final output contains both nonces),
  `t1.payload-parse` (the gated test over `capture/tier1`). Timeout 180 s per run. A tier-1 failure is
  retried twice (parent) before the probe counts as failed.
- Cost bound: one prompt, small model, `--max-budget-usd 0.10` (Claude) / low effort (Codex); a daily
  run with one new version per harness costs cents; worst case (bisect over 7 candidates, all retries)
  ≈ 10 probes × 3 attempts × 2 harnesses.

### D7. Bisection

Implemented in `scripts/canary/bisect.py` against an abstract probe command
(`--probe-cmd '<cmd> {version}'`, exit 0 pass / 1 fail / 2 infra, JSON result on stdout) so the
self-test drives the same code with a stub.

1. Candidates `C[0..n-1]` ascending (§D1); baseline `B = verified_max` (assumed good, not yet probed).
2. Probe `C[n-1]`. Pass → status `all_pass` (intermediate versions are not probed: optimistic by owner
   intent). Fail → continue. Without `--bisect`, every candidate is probed in descending order and
   reported; no `first_bad` is claimed.
3. Binary search with `lo = -1` (B), `hi = n-1`: `mid = (lo+hi)//2`; pass → `lo = mid`, fail → `hi = mid`;
   until `hi - lo == 1`. `first_bad = C[hi]`, `last_good = C[lo]` or `B`.
4. Confirm with fresh installs: `first_bad` must fail and `last_good` must pass (this is the first probe
   of `B` when `lo = -1`).
5. Retry policy per probe: a fail is retried once (a tier-1 fail twice); pass on retry → pass, marked
   `flaky: true`. An infra result is retried once; still infra → harness status `infra_error`, bisection
   stops, exit 2 (no `first_bad` claimed).
6. `inconclusive` when any observed result contradicts monotonicity: the confirmation of `first_bad`
   passes, the confirmation of `last_good` fails, or `B` itself fails (environment or registry problem,
   not a version break). The report lists every probe with its attempts and checks.
7. Honest limit (recorded in the report as `assumes_monotone: true`): non-monotone regions the search
   never probes are not detected; the probe list shows exactly which versions were tested.

Probe budget: ≤ 1 + ⌈log₂ n⌉ + 2 probes per harness, each ≤ 2 attempts (tier 1: 3). (Amended by coverage r2, 2026-10-01: plus 1 probe per crossed `known_broken` range — see rule 8.)

8. **known_broken exclusion and moving baseline** (Amended by coverage r2, 2026-10-01, ht-p03.14.1; `known_broken` is the ht-p03.13 list of inclusive `{min,max}` ranges, `null` = open). Candidates inside any `known_broken` range are excluded from the bisect search and listed in the report (`excluded`). Once the version just above the newest `known_broken` range passes, the baseline `B` moves past that range, so a second break above it surfaces with its own `first_bad`; the move costs **+1 probe per crossed range**. An open-ended range (`max` null) never excludes the newest stable version: it is probed anyway. If the version just above a range still fails, the harness status is `known_broken_persists` (exit 0, no `first_bad`, nothing filed, no new issue) instead of a new break. The probe-count bound the self-test asserts is therefore ≤ 1 + ⌈log₂ n⌉ + 2, plus 1 per crossed `known_broken` range.

### D8. Report, summary and exit codes

`<out>/canary-report.json` (`schema_version: 1`):

```json
{"schema_version":1,"generated_at":"…","canary_commit":"<git sha>","herdr_threads":"<version>",
 "runner":{"os":"linux","arch":"x86_64"},
 "inputs":{"harness":"both","versions":"since-verified","bisect":true,"model_tier":"auto"},
 "harnesses":[{"harness":"claude","package":"@anthropic-ai/claude-code","verified_max":"2.1.286",
   "candidates":["2.1.287"],"status":"all_pass|break|inconclusive|infra_error|no_candidates|known_broken_persists",
   "first_bad":null,"last_good":null,"failing_checks":[],"signals":[],
   "suggested_action":null,
   "probes":[{"version":"2.1.287","role":"newest|search|confirm","attempts":[{"result":"pass|fail|infra",
      "tier1":false,"duration_ms":0,"checks":[{"id":"t0.version","status":"pass","detail":"…"}]}],
      "result":"pass","flaky":false}],
   "assumes_monotone":true}],
 "exit_code":0}
```

`suggested_action` on `break`: `{"kind":"adapter_or_known_broken","range":">= <first_bad>",
"known_broken_snippet":"known_broken: Some(VersionSet::Interval { min: Version::new(a,b,c), max: … })",
"text":"add a recipe for >= <first_bad> (adapter), or mark [<first_bad>, …) known_broken until one exists"}`
— the snippet's `max` is the newest failing version probed. `summary.md` renders one table per harness
(version × check), the verdict line, signals, and the suggested action.

(~~the snippet above~~ superseded, Amended by coverage r2, 2026-10-01: recipes hold `known_broken: &'static [VersionSet]` (ht-p03.13), so the paste-ready snippet is `known_broken: &[VersionSet::Interval { min: Some(Version::new(a, b, c)), max: None }]`, with `max: Some(Version::new(…))` set to the newest failing version probed)

Exit: `0` every harness `all_pass`, `no_candidates` or `known_broken_persists` (Amended by coverage r2, 2026-10-01; warns allowed); `1` any `break` or
`inconclusive` (and no infra error); `2` any `infra_error`, bad arguments, or isolation refusal. The
report is written in every case where arguments parsed.

### D9. Workflow `.github/workflows/harness-canary.yml`

- Triggers: `schedule: cron '17 6 * * *'` (daily, off the hour), `workflow_dispatch` (inputs `harness`
  default `both`, `versions` default `since-verified`, `model_tier` default `auto`, `runner` default
  `ubuntu-24.04`), and `pull_request` with `paths:` the canary files — **self-test job only**.
- Top-level `permissions: {contents: read}`. Jobs:
  - `selftest` (all triggers): checkout, setup-python, `scripts/harness-canary.sh --self-test`.
  - `canary` (schedule/dispatch only; `needs: selftest`; `permissions: {contents: read, issues: write}`;
    `timeout-minutes: 90`; `concurrency: {group: harness-canary, cancel-in-progress: false}`):
    checkout, setup-node (Node 22), rustup (repo `RUST_TOOLCHAIN`), `cargo build --locked`, run
    `scripts/harness-canary.sh --harness ${{inputs.harness||'both'}} --versions … --bisect
    --model-tier … --out "$RUNNER_TEMP/canary"` with `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` from secrets
    (empty when absent → tier 1 skipped, parent deferral), `continue-on-error` captured via `$?`;
    upload `canary-report.json`, `summary.md` and probe logs (`if: always()`, 30-day retention); append
    `summary.md` to `$GITHUB_STEP_SUMMARY`; run `scripts/canary/file_issues.py --report … --repo
    "$GITHUB_REPOSITORY" --run-url …` with `GH_TOKEN: ${{ github.token }}` when the report exists; finally `exit` with the canary's code.
- Every `uses:` pinned `<owner>/<action>@<40-hex sha> # <tag>` (ht-p03.16 convention); actionlint
  clean.
- **Schedule liveness** (amended by design roast round 1). In a public repository GitHub disables
  scheduled workflows after 60 days without repository activity. A disabled canary files no issues, so
  it looks exactly like a passing one. Nothing in the workflow can detect its own disablement, so the
  check is documented instead. The canary section of `docs/compatibility/harnesses.md` and the
  `docs/release.md` checklist both state the rule. The check is `gh workflow view harness-canary.yml`:
  the state is `active` and the latest scheduled run is recent. The remedy is `gh workflow enable
  harness-canary.yml`. An automated keepalive is deferred (below).

### D10. Issue filing and deduplication (`scripts/canary/file_issues.py`)

- One issue per harness whose status is `break` or `inconclusive`. Titles (the dedupe key, exact match):
  `harness-canary: <harness> <first_bad> breaks herdr-threads` and
  `harness-canary: <harness> inconclusive above <last_good|verified_max>`.
- Lookup: `gh issue list --repo R --state open --label harness-canary --search "<title> in:title"
  --json number,title,body` then exact title comparison.
- Body: verdict, range (`>= first_bad`; last good version), failing checks with their `detail`
  excerpts (≤ 40 lines each), the probe table, run URL and artifact name, suggested action with the
  `known_broken` snippet, and a hidden marker `<!-- canary-digest: <sha256 of harness block minus timings> -->`.
- Existing open issue: comment with the new run only if its latest digest differs (no daily noise);
  else no-op. New: `gh label create harness-canary --force` then `gh issue create --label harness-canary`.
- Never closes or edits labels of existing issues; never opens PRs. `--dry-run` prints the `gh`
  commands instead of running them (used by tests and local exercise).

### D11. Self-test (offline)

`scripts/harness-canary.sh --self-test` execs `python3 scripts/harness-canary-selftest/run.py`, which
needs no network, npm, Rust build or harness, and finishes in seconds. It drives `bisect.py` with
`scripts/harness-canary-selftest/stub_probe.py` (a stub installer + checks whose per-version results
come from a case file; stateful cases count calls in a temp dir) and asserts the report:

| Case | Planted | Expected |
|---|---|---|
| all-pass | 9 candidates, all pass | `all_pass`, exactly 1 probe |
| break-mid | first fail at index 5 of 9 | `break`, `first_bad = C[5]`, `last_good = C[4]`, ≤ 1+4+2 probes |
| break-first | first fail at index 0 | `first_bad = C[0]`, `last_good = B` (B probed once, passes) |
| flip | `C[k]` fails during search, passes on confirmation | `inconclusive`, every probe listed |
| baseline-broken | B fails on confirmation | `inconclusive` |
| flaky | a version fails once then passes | treated as pass, `flaky: true` |
| infra | stub installer exits 2 twice for one version | `infra_error`, exit 2, no `first_bad` |
| tier1-retry | tier-1 fail, fail, pass | pass after 3 attempts |
| known-broken-then-new-break (Amended by coverage r2, 2026-10-01, ninth case, ht-p03.14.1) | a `known_broken` range in the fixture harness-versions.json, the version above it passes, then a later version fails | the range's versions are excluded, the baseline moves past it (+1 probe), `break` with its own `first_bad`; a variant where the version above the range still fails reports `known_broken_persists`, exit 0 |

Plus offline unit checks of the helpers: candidate filtering on a recorded `npm view` fixture (a trimmed
copy of today's Codex list, platform/alpha entries included), `verified_max` from a fixture
harness-versions.json, and report → `summary.md` rendering.

`file_issues.py --dry-run` against planted reports (new issue; same digest → no-op; changed digest →
comment) is checked by `scripts/canary/test_file_issues.py`, owned by C4 and run by the workflow's
`selftest` job right after `--self-test`. `--self-test` itself does not call it: it execs C1's `run.py`,
and calling C4's file would add a C1 → C4 dependency.

### D12. How the script is exercised locally before merge

Each leaf's acceptance names its part; together:

1. `scripts/harness-canary.sh --self-test` — offline.
2. `scripts/harness-canary.sh --harness both --versions latest --model-tier off --out "$TMPDIR/hc-latest"`
   — newest Claude (2.1.287 today) classifies `optimistic`, newest Codex `schema-matched…` or
   `optimistic`; exit 0.
3. Real-registry bisect with a planted break: `HT_CANARY_PLANT_BREAK=claude@2.1.285` (test-only
   variable; adds a failing `t0.planted` check for versions ≥ the given one) with
   `--harness claude --baseline 2.1.283 --versions 2.1.284,2.1.285,2.1.286,2.1.287 --bisect --model-tier off` must
   report `first_bad 2.1.285`, `last_good 2.1.284`, exit 1 — the full install/tier-0/bisect path
   end to end (`--baseline` overrides `verified_max`).
4. Listed-version evidence run (C5): Claude 2.1.283–2.1.286, Codex 0.157.1, 0.158.0 and 0.159.2/0.159.3.
5. `actionlint .github/workflows/harness-canary.yml`; `file_issues.py --dry-run` on the step-3 report.
6. Tier 1 only if the implementer's environment already has API keys exported (never read from aisw
   profiles or the owner's harness homes); otherwise the first CI run after secrets are added is the
   proof (parent deferral; §Follow-on).

The scheduled workflow's first real run is post-merge follow-on (no push in this run).

## Leaves (decomposition)

| Leaf | Bead | Delivers | Blocked by |
|---|---|---|---|
| C1 Bisect engine, version/report helpers, offline self-test | `ht-p03.14.1` | `scripts/canary/{versions,bisect,report}.py`, `scripts/harness-canary-selftest/*` (stub probe, cases, fixtures, `run.py`); D1 filtering, D7, D8 writer, D11 | — |
| C2 Canary script skeleton, `--probe`, isolation, capture contract, payload parse path | `ht-p03.14.2` | `scripts/harness-canary.sh`, `scripts/canary/run.py`, `tests/harness/canary_payloads.rs` (parse path) + test mount in `src/harness/mod.rs`; D1, D2, D5, D8 wiring; checks t0.version/config-load/payload-parse/isolation/planted | `ht-p03.14.1` |
| C2b Tier-0 dependency-gated and warn checks, acceptance runs | `ht-p03.14.6` | `scripts/canary/{witness.sh,stub_api.py,launch-flags.tsv}`, check call sites, Rust fingerprint/launch_tables/flag-table test; t0.setup/unsetup/admission/hook-fires/launch-flags, warn t0.schema/npm-shim-admission/launch-tables; D3, D4, D12 steps 2–3 | `ht-p03.14.2`, ht-p03.13, ht-p03.15, ht-p03.28, ht-p03.23 |
| C3 Tier 1 model run | `ht-p03.14.3` | `scripts/canary/{tier1.sh,capture_hook.py}` + the tier-1 call site; D6 | `ht-p03.14.2` |
| C4 Workflow, issue filing, docs | `ht-p03.14.4` | `.github/workflows/harness-canary.yml`, `scripts/canary/{file_issues,test_file_issues}.py` + `scripts/canary/testdata/reports/`, canary section of `docs/compatibility/harnesses.md`; D9–D10 | `ht-p03.14.2` (selftest job calls `harness-canary.sh`; report schema decided in §D8) |
| C5 Listed-version tier-0 evidence | `ht-p03.14.5` | `docs/evidence/canary-tier0-2026-10/`, recipe evidence levels `no_model` where no live run, regenerated harness-versions.json | `ht-p03.14.6` |

Edges carry `blocked-by` reason lines in each dependent's description. C2b takes the epic's external
deps (ht-p03.13/.15/.28) and adds ht-p03.23 (doctor `hooks.claude.installed.admission`, the only
Claude admission surface; a comment on ht-p03.23 names the fields). C1 consumes nothing from
outside the epic; C2 only C1. Longest chain inside the epic: C1 → C2 → {C3, C4, C2b → C5}; C2b also waits
on its external deps. ht-p03.19 and ht-p03.20 take C2b as well as the epic.

## Acceptance (epic level, distributed to leaves)

- `--self-test` passes offline, all D11 cases (C1; the `--self-test` flag is wired by C2).
- `--harness both --versions latest --model-tier off` passes locally; newest classifies as expected (C2b).
- Planted-break real-registry bisect reports exactly `first_bad`/`last_good` (C2).
- Tier-0 runs of the listed older versions recorded; evidence levels `no_model` where no live run (C5).
- Tier 0 setup/unsetup and launch-flag checks pass for the installed versions (C2b; needs ht-p03.15/.28).
- actionlint clean; every `uses:` pinned to a full SHA with tag comment; `file_issues.py --dry-run`
  cases pass (C4).
- Tier 1: `auto` with no keys → `t1.*` `skip` and exit unaffected; `required` with no keys → exit 2;
  with keys, one recorded run per harness or recorded as follow-on (C3).

## Configurations exercised

The parent's Harness version set, canary rows: newest Claude (> 2.1.286; 2.1.287 today) and newest
Codex → C2 (step 2); Claude 2.1.283–2.1.286 and Codex 0.157.1, 0.158.0, 0.159.2, 0.159.3 → C5. Install
layouts: npm-native (both) → C2; npm shim vs native Codex → `t0.npm-shim-admission` (warn) in every
Codex probe. Model tiers `off` / `auto`-without-keys / `required`-without-keys → C3; `auto` with keys →
follow-on unless keys are present at implementation time.

## Explicit deferrals and deviations from the parent

| Item | Reason |
|---|---|
| E4: npm-installed Codex (`codex.js` shim) cannot be fingerprinted, so it is `refused` today and would be `Optimistic` instead of `SchemaMatched` after ht-p03.13 | Product code in `src/harness/codex_schema.rs`/`codex.rs` (ht-p03.13's files), outside a canary epic. Filed as ht-p03.35 (fix: when the resolved `codex` canonicalizes to `@openai/codex/bin/codex.js`, fingerprint the sibling `@openai/codex-<platform>/vendor/<triple>/bin/codex`). The canary's warn check keeps it visible. |
| Fake-model stub driving PreToolUse without a model | Would make tier 0 cover PreToolUse offline; Codex hung "waiting for network" against a local provider in this pass and the Responses/Messages streaming stubs are new tooling. Tier 1 covers PreToolUse when keys exist. |
| macOS in the scheduled job | Parent chose ubuntu-24.04; dispatch input allows `macos-15`. |
| Parent "Tier 0 … the harness loads the written config using a model-free command" | Realized as `t0.config-load` + `t0.hook-fires` + `t0.payload-parse` (stronger: the setup-written hook actually fires). |
| Parent "inconclusive on a planted non-monotone sequence" | Detection is defined as an *observed* contradiction (D7.6); unobserved non-monotone regions are not detectable without probing every version, and the report says so (`assumes_monotone`). |
| Parent: tier 1 hooks are capture hooks | Kept, and tier 1 runs on a home without herdr-threads setup so the nonce delivery is attributable; tier 0 covers the setup-written hooks. |
| Automated keepalive for the 60-day scheduled-workflow disable (D9) | A keepalive commit needs `contents: write` on the scheduled job, against D9's least-privilege permissions. An external monitor lives outside the repo. The liveness check is documented instead (D9, release checklist). |

## Follow-on (post-merge, human)

- Add `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` repository secrets to enable tier 1 in CI.
- Watch the first scheduled run; confirm the self-test PR job runs on the first canary-touching PR.
- Keep the schedule alive: check `gh workflow view harness-canary.yml` periodically, and run
  `gh workflow enable harness-canary.yml` after any 60-day quiet period (D9).
- Triage the first `harness-canary` issue: add an adapter recipe or a `known_broken` range.

## Post-Implementation Notes

(empty)
