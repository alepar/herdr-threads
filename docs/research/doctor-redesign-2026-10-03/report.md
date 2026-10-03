# Doctor, status, and repair interfaces for Herdr Threads

## Executive Summary

The existing Herdr Threads doctor collects useful information but puts many inventory lines ahead of its result. An operator must scan version, path, and hook details before seeing whether the system is healthy or which action matters. The redesign should print the result and actionable issues first, retain the prior inventory under `--debug`, and preserve JSON field meanings. These are interface choices drawn from the local baseline and cross-checked against diagnostic tools that separate ordinary summaries from detail or repair [1][2][3][7][11].

Repair must be narrower than detection. Beads exposes `doctor --fix`, but also distinguishes preview, confirmation, and options for more invasive repairs [1][2]. Cargo applies only compiler-proposed fixes and then shows remaining warnings [7]. Homebrew tells operators to correct relevant warnings and cautions against broad ownership or permission changes based on old advice [8]. Herdr Threads has a stronger continuity constraint: its daemon decides seat and receipt changes against canonical state; a doctor that guessed a seat from local hints would cross the trust policy [12]. The practical safe set is owned Claude hook setup for an admitted harness and daemon ensure when its endpoint is simply absent. Codex setup can also write an allowance into global sandbox configuration, so it remains an explicit operator action. Unsafe state, version mismatch, hook trust, and seat recovery likewise need explicit operator action.

Codex hook trust illustrates why the wording must distinguish evidence from conclusion. OpenAI says installed non-managed hooks remain disabled until reviewed [5]. The local setup code can read whether Codex recorded a per-hook `trusted_hash`; it cannot prove that the hash matches the current hook or that the current TUI will execute it [6][11]. Doctor should call missing keys “review required,” present recorded keys as “recorded, current hashes unverified,” and use “unknown” when files cannot be interpreted. `doctor fix` must never write trust. The installer should place the one-time interactive review step before routine suggestions when it has just set up Codex hooks, without claiming to know current trust.

## Introduction

This research compares four concrete surfaces: Beads `bd doctor`, the installed Herdr and Herdr Threads command/help contract, Claude Code diagnostics, and Codex diagnostics and hook trust. It also checks repair and presentation practices in Cargo, Homebrew, and npm. The implementation target is the Herdr Threads checkout at `0d70765e`, with its `doctor` report, setup library, installer, and trust policy as the local baseline [11][12]. The question is how to make default output concise while retaining evidence and adding a repair verb that cannot silently change identity or trust.

Sources were inspected on 2026-10-03. External documentation and the installed CLI help were read as product behavior descriptions; local code and isolated tests were used for Herdr Threads behavior. Product versions and documentation can change, so the comparison supports design choices rather than a promise about future CLI syntax. The neighboring tab reported Codex 0.160.0 no-model and scoped native exec/TUI sandbox checks; those private session overrides do not establish the effective policy of all user and project sessions. This report does not turn that observation into a global support claim.

## Main Analysis

### 1. Put the judgment before the inventory

Beads' installed `bd doctor --help` says the default view shows warnings and errors while `-v/--verbose` shows all checks; `--agent` adds observed and expected state, explanation, commands, sources, and severity. Its docs also document `bd doctor --fix` for a specific metadata refresh [1][2]. This is a useful two-level model: a person sees what needs attention, while an agent or investigator can request evidence. The installed Codex `doctor --help` similarly offers `--summary` for grouped rows and `--json` for a machine report. That observation is version-specific and is recorded in the evidence ledger rather than asserted as a stable public API.

Claude Code documents terminal `claude doctor` as a read-only installation and settings diagnostic and distinguishes it from in-session `/doctor`, which can apply fixes [3]. Its getting-started guide uses doctor after installation to check installation type and version [4]. The comparison suggests a simple front page for Herdr Threads: overall result, daemon state, harness version judgments, hook status, and actionable issues. It should link to a detailed form without making the user read the details first [1][3][4][11].

The baseline Herdr Threads text renderer begins with version, protocol, state path and source, host endpoint, instance path, and harness verdict blocks before the daemon state and final `result`. Its JSON is the structured report the text renderer consumes [11]. Keeping that renderer behind `--debug` retains the evidence operators already use. A new compact renderer can use the same report and lead with `doctor: ok|degraded|unavailable|...`; this avoids another diagnostic truth source. The detailed and compact text may evolve, while machine readers keep the existing JSON fields [11].

### 2. Check first, repair explicitly, recheck afterward

Beads separates inspection and mutation with `--fix`; it exposes `--dry-run`, interaction flags, and additional opt-in repairs for cases with greater consequences [1][2]. Cargo's `fix` command applies known compiler suggestions, runs checking, and displays warnings that remain. It also guards dirty work unless the user chooses the relevant allowance [7]. npm doctor, by contrast, is described as checking the environment and displaying recommended changes, including external requirements such as registry reachability and writable directories [10]. These examples support an explicit repair command with per-action outcomes and a final fresh diagnostic result [1][7][10].

Herdr Threads has existing actions with appropriate boundaries: `daemon ensure` and owned `setup claude` [11]. The repair planner can use those when the local context is valid, the state directory passes ownership/mode checks, the daemon is merely not running, and the admitted Claude binary has missing hooks whose ownership is inspectable. Setup itself uses ownership manifests and refuses a changed or foreign installation. Doctor must report that refusal instead of overriding it [11][12]. `setup codex` may also change global sandbox configuration, so doctor names it as a manual step. If a daemon is unreachable or has version skew, restarting or stopping it could affect the shared server and cannot be inferred from a failed read. A clear manual outcome is safer than a speculative process action [8][12].

Rechecking matters. The daemon might start but still be degraded, or setup may complete while the hook still needs user trust. The post-repair report must therefore supply the exit status and status lines, and each attempted action must say whether it was attempted, failed, refused, or still manual. A successful `setup codex` cannot be presented as proof of a working Codex hook [5][11]. Idempotence follows from planning against the current report and invoking the existing setup and ensure operations, then planning no setup action on a second run once ownership is installed [11].

### 3. Treat hook trust and sandbox policy as separate evidence

OpenAI's plugin documentation says enabling a plugin does not automatically trust its hooks; non-managed hooks wait for user review [5]. Codex's source persists a reviewed hook hash under `hooks.state` [6]. The Herdr Threads setup documentation describes the user-level Codex `hooks.json` and its recorded per-hook keys, while its code explicitly says the presence of a stored hash is not verified [11]. The strongest local assertion is therefore that a review record exists, not that the current hook is trusted or has run. A missing key in a readable, installed hook configuration calls for the manual TUI review. A malformed or unreadable config is unknown. This distinction should be present in JSON as well as default text, and `doctor fix` should only name the interactive step [5][6][11].

Sandbox socket policy has a different evidence ladder. The existing documentation ties an allowance to a measured Codex version and warns about configurations not measured for default-deny [11]. An isolated no-model `codex sandbox` probe on installed 0.160.0, reported by the adjacent tab, observed the allowed Unix socket connect and controlled other Unix, loopback, and external TCP connects fail with `EPERM`; unsandboxed controls connected. The same tab later reported scoped native exec/TUI checks with exact socket and journal-root behavior under private session overrides. Those observations are useful for those binary and invocation paths, but do not establish the effective policy of all user/project sessions or future versions. Doctor should relay “global socket policy unvalidated for the effective Codex configuration” or “probe inconclusive/failed” where appropriate and avoid collapsing these into “Codex unsupported.” A fetch failure or an absent probe is not a successful default-deny measurement. This is an inference from the reported scope and local policy, not a product claim [11][12].

### 4. Keep installer guidance prominent and terminal aware

The Herdr Threads installer already ends with next steps and a final status line, and its offline harness checks exit codes and cleanup across install, upgrade, partial setup, and uninstall [11]. The important addition is the manual Codex review immediately after a successful hook setup, ahead of routine “check it” and “try it” guidance. Because the installer cannot verify current trust, the instruction should say to review if prompted or open `/hooks`, and should not say that trust is already satisfied [5][11]. A failure still needs its established exit code and follow-up command.

Terminal color can make the one-time action and final judgment easier to spot, but logs and pipelines must remain clean. Cargo's public CLI explicitly makes color automatic by default, with `always` and `never` choices [7]. In this installer, the narrower rule is enough: emit ANSI only to a TTY with `TERM` usable and `NO_COLOR` unset; keep redirected output exact plain text. The offline installer test can exercise both a pseudo terminal and `NO_COLOR` without touching the user's real Herdr or harness configuration. Color is presentation only: wording and exit statuses remain the operational contract [7][11].

### 5. The trust policy rules out clever identity repair

Homebrew's current troubleshooting guide warns against unexamined destructive ownership or permission changes [8]. Its tap trust guide treats code execution trust as an explicit user decision, with doctor warnings rather than automatic trust [9]. These are analogous to Herdr Threads' local invariants: an operator can deliberately rebind or retire a seat, but a doctor cannot infer an identity transition from a pane name, local file, or old endpoint [12]. The daemon's canonical view and receipt provenance must remain the decision point [12].

No proposed repair touches seat bindings, receipts, wake behavior, or hook trust. A doctor is allowed to diagnose these conditions and give exact manual guidance. This is the central safety boundary for `doctor fix`: automatic actions use existing owner-aware operations; ambiguous or security-relevant actions stay manual. It makes failures reviewable and avoids a new provenance class [8][9][12].

## Synthesis & Insights

The examples converge on a two-layer interface. The ordinary status page is optimized for decisions; a detailed or machine form exposes evidence; a separate repair action applies a limited set of known operations and reports what remains. Beads offers the clearest explicit fix verb [1][2], Claude separates read-only terminal diagnosis from an in-session repair surface [3], Cargo demonstrates recheck after applied fixes [7], and the Herdr Threads trust policy supplies a much narrower mutation boundary [12]. The resulting design is a local adaptation, not a copy of any one tool.

A second pattern is that “configured,” “trusted,” “observed working,” and “empirically measured” are different states. The Codex trust record and the sandbox probe each answer one question only [5][6][11]. Doctor becomes more useful when it names the exact evidence level rather than forcing all incomplete states into an unsupported verdict. This also keeps the JSON report honest for automation.

## Limitations & Caveats

The Beads and Codex installed help captures are local observations from 2026-10-03; their output may change independently of the cited web documentation. The Herdr Threads source links point at the baseline commit and may require repository access. The Codex trust status derived from `config.toml` does not recompute Codex's own current hook hash and cannot prove hook execution. The 0.160.0 sandbox observations include a controlled `codex sandbox` invocation and scoped native exec/TUI checks reported by a neighboring tab; this report does not include their raw captures and does not elevate them to global effective-configuration evidence.

Repair tests use isolated `HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, state directories, and private daemon context. They do not exercise a live user's hooks or change the shared Herdr server. A recheck can still race with an external edit; the existing setup ownership checks are the write boundary [11][12].

## Recommendations

Implement a verdict-first default text renderer over the existing report, retain the previous renderer under `doctor --debug`, and leave existing JSON fields intact. Add `doctor fix` as an explicit operation that only ensures an absent daemon and runs existing owned Claude setup when its hooks are missing and ownership is inspectable. Keep Codex setup explicit because it can change global sandbox configuration. After every run, recheck and list attempted, failed, refused, manual, or no-op outcomes. Keep unsafe paths, version skew, foreign hook ownership, seat changes, and Codex trust manual [1][3][7][11][12].

Expose Codex hook trust as `review_required`, `recorded_unverified`, `unknown`, or `not_installed`; treat only the first as confirmed pending review and never write `hooks.state` from doctor. Put the interactive one-time review step first in installer next steps after Codex setup, with plain output in pipes and color only in supporting terminals [5][6][7][11]. When binary-specific sandbox probes are integrated later, name unknown, inconclusive, and positive measurements separately; do not present an unprobed version or failed probe as unsupported or default-deny [11][12].

## Methodology Appendix

The run used standard-mode scope, retrieval, cross-source comparison, synthesis, and claim review. Sources were restricted to product documentation and maintained source repositories, plus the local checked-out baseline and installed CLI help. The evidence ledger stores short excerpts or exact code locators with stable source IDs; the claim ledger records the conclusions the design uses. Product-specific facts generally have one authoritative publisher rather than three independent witnesses, while cross-product recommendations compare at least three sources. The report does not infer facts from search snippets alone where a primary page or installed command was available. The outline was refined after source review to separate Codex trust from sandbox policy, since both were initially grouped under “hooks.”

Artifact placement differs from the research skill's default `~/Documents` path: this workspace permits writes to the repository and temporary roots, so the report and ledgers live under `docs/research/doctor-redesign-2026-10-03/`. HTML and PDF copies are generated from this Markdown source. No copy is written to real user configuration directories.
## Bibliography

[1] Beads maintainers (2026). [Beads troubleshooting](https://github.com/gastownhall/beads/blob/main/docs/reference/troubleshooting.md). Retrieved 2026-10-03.
[2] Beads maintainers (2026). [Beads repository and CLI](https://github.com/gastownhall/beads). Retrieved 2026-10-03; installed `bd doctor --help` also inspected.
[3] Anthropic (2026). [Claude Code CLI reference](https://code.claude.com/docs/en/cli-reference). Retrieved 2026-10-03.
[4] Anthropic (2025). [Claude Code installation](https://docs.anthropic.com/en/docs/claude-code/getting-started). Retrieved 2026-10-03.
[5] OpenAI (2026). [Package your plugin](https://developers.openai.com/plugins/build/plugins). Retrieved 2026-10-03.
[6] OpenAI (2026). [Codex hook trust persistence source](https://github.com/openai/codex/blob/main/codex-rs/tui/src/hooks_rpc.rs). Retrieved 2026-10-03.
[7] Rust project (2026). [cargo fix](https://doc.rust-lang.org/cargo/commands/cargo-fix.html). Retrieved 2026-10-03.
[8] Homebrew (2026). [Troubleshooting](https://docs.brew.sh/Troubleshooting). Retrieved 2026-10-03.
[9] Homebrew (2026). [Tap Trust](https://docs.brew.sh/Tap-Trust). Retrieved 2026-10-03.
[10] npm (2021). [npm doctor](https://docs.npmjs.com/cli/v7/commands/npm-doctor/). Retrieved 2026-10-03.
[11] Herdr Threads (2026). [Doctor source at baseline](https://github.com/alepar/herdr-threads/blob/0d70765e/src/cli/doctor.rs). Read locally at `0d70765e`, 2026-10-03; installer and setup files also inspected.
[12] Herdr Threads (2026). [Trust policy at baseline](https://github.com/alepar/herdr-threads/blob/0d70765e/TRUST-POLICY.md). Read locally at `0d70765e`, 2026-10-03.
