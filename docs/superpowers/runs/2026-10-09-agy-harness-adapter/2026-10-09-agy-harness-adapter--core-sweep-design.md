# Core sweep: every production harness-specific site behind the adapter

Parent: [root spec](2026-10-09-agy-harness-adapter-design.md) · Bead: ht-poh.2

## Goal

After this subtree, the core (cli, client, service, daemon, store, host, protocol, app and the neutral `src/harness/*.rs` files) names no harness except in permanent allowlist entries that give a frozen-wire, history or registration reason. `scripts/check-harness-neutral` reports zero `to-migrate` entries. Claude, Codex and Hermes behave as before: wire, JSON, hook output and setup/doctor reports stay byte-identical, and the existing tests guard them. The first leaf lands every new adapter hook with an inert, neutral default. That way the agy adapter (ht-poh.4) can be written entirely inside `src/harness/agy/` plus its `builtins()` row.

## Problem description

The adapter model (2026-10-03) moved admission, decode, encode, setup and launch behind `HarnessAdapter`, but much harness knowledge is still hard-coded in core:

- **Environment markers.** Two constant marker lists: `cli/mod.rs:264` `AGENT_ENV_MARKERS` and `cli/output.rs:173` `HARNESS_MARKERS`.
- **Config roots.** `cli/setup.rs:205-210,556-568` hard-codes Claude and Codex config roots.
- **Help text.** `cli/setup.rs:60-144`, `cli/launch.rs:74-132`, `cli/handoff.rs:26-58` and `cli/me.rs:38-45` hold per-harness help as constants.
- **Foreground advice.** `cli/setup/foreground.rs:7-8,55-121` holds the per-harness advice and settings parsers.
- **Doctor.** `cli/doctor.rs` holds the per-harness projections, text sections and repairs: `legacy_claude_projection` at :662 and `legacy_codex_projection` at :782, both called back from `harness/claude.rs:818` and `harness/codex.rs:1308`.
- **Settings kinds.** The settings-file engine's closed `SettingsKind {ClaudeUser, CodexUser}` is at `harness/setup.rs:108-142`, with Claude/Codex matches through its `legacy` module (:2047-2932).
- **Version and evidence parsing:**
  - version normalization and `contract-id` (`harness/contract.rs:266-323`);
  - transcript attribution (`harness/attribution.rs:86-205`);
  - evidence holding (`daemon/harness_evidence.rs:210`).
- **Hook runner.** `cli/hook.rs:247-382,1200-1212,2686-2703` keeps the `InstalledHarness` enum and the per-harness parse-failure report.
- **Health inputs.** `app.rs:255-263,1523-1524` and `daemon/health.rs:106-108,500-517,610-611` carry named Health inputs.
- **Store query.** `store/queries.rs:6315` has a SQL `harness IN ('claude','codex')`.
- **Codex-named vocabulary in neutral signatures:**
  - `CodexShellProbe` in `LaunchPolicy` (`adapter.rs:767,788,806`);
  - `InstallObservation::CodexWitness` (`adapter.rs:229`);
  - `codex::bounded_output`, `VERSION_TIMEOUT` and the admission labels used by `app.rs`, `manifest.rs` and `cli/setup.rs`.

A new harness would have to edit every one of these. That is exactly the out-of-adapter cost R13 measures.

## Main challenges

1. **Inventing hook shapes.** The root spec names the hooks (markers, help fragments, `foreground_advice`, the harness note, settings kinds) but not their shapes. They must be general enough for agy, which needs a settings kind with only a `permissions.allow` member in a file the harness rewrites itself. They must also stay byte-identical for Claude and Codex.
2. **Volume and overlap.** The sweep touches about 30 files. Several large files (`cli/doctor.rs`, `cli/setup.rs`, `harness/setup.rs`, `harness/claude.rs`, `harness/codex.rs`) would draw edits from several parallel leaves. Two sibling subtrees also edit `adapter.rs`, `cli/hook.rs`, `cli/commands.rs` and `harness/claude.rs`.
3. **Frozen sites must stay.** Some sites must keep naming a harness forever: v1 `HarnessHealth{codex,claude}`, v1 `HarnessEvidence`/`HookParseFailure` validation, the frozen V1 bootstrap grammar, schema-history checks, the generated `versions.json`, and the setup/launch JSON compatibility keys. They must be recognised and allowlisted, not "neutralised" into a wire change.
4. **The seam must be inert.** It must still give agy a working settings engine.

## Key decisions made

- **Where hooks live.**
  - One `AdapterMetadata` field (`agent_env_markers`, as the root spec names it).
  - Every other hook is a defaulted `HarnessAdapter` method. All static text fragments sit in one `AdapterText` struct that has a `NONE` constant, so adding a field later touches no adapter.
- **Settings kinds.** They become adapter-supplied `SettingsKindSpec` data that the shared JSON engine interprets. The engine gains an allow-member-only mode for files the native harness rewrites, which agy needs.
- **The predicate for "harnesses on the frozen v1 surfaces".** The existing `legacy_contract_id().is_some()` decides which harnesses Health always reports, report parse failures and trigger manifest fetches. No new flag is added.
- **Order of work.**
  - The seam also carries the inert, move-only neutral-vocabulary renames that sit in trait signatures or are imported by several areas: `PaneShellProbe`, process helpers, labels, `BinaryIdentity`, operational contract types, and `claude/doctor.rs`/`codex/doctor.rs` forwarder files.
  - After the seam, leaves are assigned by file, not by concern, with stated hunk exceptions.
- **Help text.** It may be reworded where a generic sentence named a harness. All wire, JSON and report text stays byte-identical.
- **Adapter-owned paths.** The sweep consumes from ht-poh.1 the rule that `src/harness/<id>.rs`, `src/harness/<id>/**` and `src/harness/<id>_*.rs` (id ∈ `builtins()`) count as adapter-owned.
- **Finding (V1 bootstrap).** The frozen V1 bootstrap grammar refuses every harness except Claude and Codex. R7's `handoff --kind agy` therefore needs a new bootstrap plan version. This sweep does not add one (D16).

## Decision points

### D1. Hook placement and shapes (`src/harness/adapter.rs`)

**Chosen:** one new metadata field plus defaulted trait methods. `Registration`, `ErasedAdapter` and `TypedAdapter` forward each method (`registry.rs:186-272`, `273-460`, `462-1080`).

```rust
// AdapterMetadata gains (required in every literal; 48 literals today, src+tests):
pub agent_env_markers: &'static [AgentEnvMarker],

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentEnvMarker {
    /// Exact env var name, [A-Z][A-Z0-9_]{0,63}. A non-empty value marks an agent process (output form).
    pub name: &'static str,
    /// Also TRUST-POLICY A4 person-pane refusal evidence (presence suffices).
    pub person_guard: bool,
}

// HarnessAdapter additions — all defaulted, all pure/local:
fn text(&self) -> &'static AdapterText { &AdapterText::NONE }
fn default_config_root(&self, environment: &SetupEnvironment) -> Option<std::path::PathBuf> { None }
fn settings_kind(&self) -> Option<&'static SettingsKindSpec> { None }
fn foreground_advice(&self) -> Option<&'static ForegroundAdvice> { None }
/// `setup-status` only (never doctor/launch status): adapter-local enrichments.
fn setup_status_extras(&self, _: &StatusRequest, _: &mut serde_json::Value) {}
/// Extra `setup` text lines rendered as `<id>: <line>` after the outcome line.
fn setup_report_lines(&self, _: &serde_json::Value) -> Vec<String> { Vec::new() }
/// Text for this adapter's `report.hooks.<id>` object; empty renders nothing.
fn doctor_text(&self, _: &serde_json::Value, _: DoctorTextForm) -> String { String::new() }
/// Prefixes of report-level limitations the Summary text already printed.
fn doctor_summary_claims(&self) -> &'static [&'static str] { &[] }
fn version_syntax(&self) -> Option<&'static VersionSyntax> { None }
fn transcript_version(&self) -> Option<&'static TranscriptVersionSpec> { None }

#[derive(Debug, Clone, Copy)]
pub struct AdapterText {
    pub setup_scope: &'static str,          // verbatim block under setup help "Scope:" (own indentation)
    pub setup_foreground: &'static str,     // sentence(s) in the "Foreground agents" paragraph
    pub setup_notes: &'static str,          // own paragraph(s) after it (Claude prompt suggestions)
    pub launch_notes: &'static str,         // paragraph(s) appended to launch help
    pub config_root_hint: &'static str,     // e.g. "CLAUDE_CONFIG_DIR"; used in launch/setup messages
    pub trust_reminder_label: &'static str, // e.g. "codex hook trust"; "" => "<id> setup trust"
    pub version_history_note: &'static str, // harness_states per-version note; "" => none
}
impl AdapterText { pub const NONE: Self = /* all "" */; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorTextForm { Debug, Summary }

pub struct ForegroundAdvice {
    pub settings_file: &'static str,                        // relative to the adapter's config root
    pub advice: &'static str,
    pub fixed_fields: &'static [(&'static str, &'static str)], // e.g. ("persistent_opt_out", "unavailable (Codex 0.160.1)")
    pub inspect: fn(&[u8], &mut serde_json::Value) -> Result<(), String>,
}

pub struct VersionSyntax {
    pub prefix: &'static str,   // stripped when present: Codex "codex-cli "
    pub suffix: &'static str,   // stripped when present: Claude " (Claude Code)"
    pub examples: &'static [(&'static str, &'static str)],
}

pub enum TranscriptRead { NewestEntry, HeadLine }
pub struct TranscriptVersionSpec {
    pub read: TranscriptRead,
    pub extract: fn(&serde_json::Value) -> Option<&str>,
    /// false: the transcript names only its creator (Codex) — payload attribution is
    /// `CreatorOnly`, a known resume `ResumedCreator`; true: resume is `ResumeBeforeFirstEntry`.
    pub current_runtime: bool,
}
```

**Registry validation in `Registry::new` (`registry.rs:1092`):**

- Marker names must be well formed and unique across all registrations. This adds `RegistryError::InvalidEnvMarker` and `RegistryError::DuplicateEnvMarker`.
- `settings_kind().harness` must equal `metadata().id`.
- Every `AdapterText` field must be at most 4096 bytes with no control characters other than `\n`.

**Registry helpers:**

- `Registry::env_markers()` iterates every marker in registry order.
- `Registry::person_guard_markers()` iterates only the markers with `person_guard: true`.

**Considered and discarded:**

- *Every hook as a metadata field.* It churns the 48 literals for each future hook, and metadata cannot hold behaviour (fn pointers work but read worse).
- *Each text fragment as its own trait method.* It needs seven forwarders in three layers each, where one `AdapterText` needs one.
- *Markers as a trait method.* Cheaper, but the root spec names `AdapterMetadata.agent_env_markers` and this subtree does not relitigate it.

**Values (seam):**

| Adapter | Markers | Other hooks |
|---|---|---|
| Claude | `CLAUDECODE` (guard) | `text`, `default_config_root`, `settings_kind`, `foreground_advice`, `setup_status_extras` (today's `claude::setup::complete_setup_status`), `setup_report_lines` (prompt-suggestion + mod lines), `doctor_text`, `version_syntax`, `transcript_version` (NewestEntry, `version`, true) |
| Codex | `CODEX_THREAD_ID` (not guard), `CODEX_SANDBOX` (guard), `CODEX_SANDBOX_NETWORK_DISABLED` (guard) | `text`, `default_config_root`, `settings_kind`, `foreground_advice`, `doctor_text`, `doctor_summary_claims` (`["Codex hook review required"]`), `version_syntax`, `transcript_version` (HeadLine, `session_meta.payload.cli_version`, false) |
| Hermes | none | `text` (its scope and launch paragraphs) only |
| synthetic_fourth | none | all defaults |

### D2. Environment markers (R8 rows: `cli/mod.rs:264`, `cli/output.rs:173`)

**Chosen:** both lists are derived from the registry.

- `output::harness_marked(env)` uses every registered marker with a non-empty value. Today that is the `HARNESS_MARKERS` set.
- `agent_env_marker(vars)` uses the `person_guard` markers by presence. Today that is the `AGENT_ENV_MARKERS` set. `CODEX_THREAD_ID` stays non-guard, as today.
- `ME_INIT_HELP` is assembled from the guard markers plus "a registered agent harness reported by Herdr". The kind check `authority::is_harness_agent_kind` is already registry-driven (`protocol/authority.rs:40`).
- TRUST-POLICY A4 (lines 194-195) is reworded in the same commit to "the person-guard environment markers registered adapters declare (today …)". The rule is unchanged.

**Discarded:** one shared list. It would either widen A4 to `CODEX_THREAD_ID`, which changes behaviour, or narrow output detection.

### D3. Help fragments and byte identity

**Chosen:** a new `src/cli/help.rs` provides `setup_help()`, `launch_help()`, `handoff_help()` and `me_init_help()`, each returning `String`. clap accepts `after_help = <expr>`, which `cli/commands.rs:852,1038` already use.

- Generic text stays in core.
- Per-adapter fragments are appended in registry order.
- The kind lists (`--kind a|b`) and the `HERDR_THREADS_*_OPTS` lists are generated:
  - the opts lists from `launch_policy().native_options_env()`;
  - the handoff and topology kind list from the frozen V1 bootstrap target set (D16), not from the whole registry, so the help never advertises a kind V1 refuses.

**Byte-identity policy:**

- Wire, JSON, hook stdout, setup and launch reports, and the doctor text and JSON must be byte-identical.
- `--help` text may change only where a generic sentence names a harness. Examples: "The Codex hook-trust reminder…" becomes "Hook-trust reminders…", and `commands.rs:1838` "see `herdr-threads setup codex`" becomes "see `herdr-threads setup <harness>`".
- Harness-specific help content moves into fragments verbatim.

**Discarded:**

- *A byte-identical help assembly.* It would force generic paragraphs to stay harness-named or be split mid-sentence into fragments.
- *A per-topic `help_fragment(HelpTopic)` method.* It is equivalent but needs a match in every adapter.

### D4. Setup config roots (`cli/setup.rs:205-210,360-361,399-420,481-493,556-568`)

**Chosen:**

- Claude adds `CLAUDE_CONFIG_DIR` to `setup_environment_inputs()`. Codex adds `CODEX_HOME` (Codex's list is empty today).
- `default_config_root(env)` resolves `declared[VAR]` when it is non-empty, else `home/.claude` or `home/.codex`. This is today's `claude_config_dir_from`/`codex_home_from` logic, moved into the adapters.
- `SetupEnv` drops `codex_home` and `claude_config_dir` and gains `config_roots: BTreeMap<String, PathBuf>`. It fills that map by iterating registrations in `from_process`/`for_context`.
- `snapshot()` copies `config_roots`. Explicit entries keep precedence (tests, `cli/launch.rs:770-773`, `harness/setup.rs:2180-2181`).
- The trait's default `resolve_setup_scope` (`adapter.rs:139-161`) is unchanged: it already reads `config_roots[id]`.
- `SetupEnv::claude_settings()`/`codex_file()` (`cli/setup.rs:481-493`) are replaced by `config_root(id)`, and the adapters join their own file names.

**Discarded:** a per-adapter `resolve_setup_scope` override with no map. `harness/launch.rs:850`, `registry.rs:932` and the tests read `config_roots` directly.

### D5. Foreground advice (`cli/setup/foreground.rs`)

**Chosen:**

- **What stays in core:** `EXPLANATION`, the bounded symlink-refusing read (`foreground.rs:12-46`), the report skeleton and `attach()`.
- **How `inspect(harness, env)` changes:** it looks up `foreground_advice()` through the registry. `None` returns `Value::Null`, which is what Hermes and every other adapter get today (`:66`).
- **How the report is assembled.** Key insertion order must match today's:
  1. the skeleton (`settings_file` = `config_root(id).join(spec.settings_file)`);
  2. `fixed_fields`;
  3. on a missing file, `user_config: missing`;
  4. otherwise `(spec.inspect)(bytes, &mut report)`, then `user_config: read`;
  5. on `Err`, `status: unknown` and `inspection_error`.
- **What moves to the adapters:** the Claude JSON parser (`:85-107`) and the Codex TOML parser (`:108-121`), as their `inspect` fns.

### D6. Settings kinds in the shared JSON engine (`harness/setup.rs:108-142`, used through `:430-1858`, `legacy` at `:1996-2932`)

**Chosen:**

```rust
pub struct SettingsKindSpec {
    pub harness: &'static str,                 // == adapter id; manifest `harness` and `<id>-user` manifest stem
    pub file_name: &'static str,               // final path component the engine may edit
    pub hook_groups: fn(&str) -> Vec<(&'static str, serde_json::Value)>, // empty => allow-member-only kind
    pub owned_allow: Option<OwnedAllowSpec>,   // the `permissions.allow` member this kind owns
    pub adoption_note: &'static str,           // Codex: ", so Codex's recorded trust for them stays valid"
    pub legacy_upgrade_warning: &'static str,  // Codex: CODEX_RETRUST_WARNING
}
pub struct OwnedAllowSpec {
    pub rule: &'static str,                    // declared member (Claude: HERDR_THREADS_ALLOW_RULE)
    pub retired: &'static [&'static str],      // Claude: [CALLER_CONTEXT_ALLOW_RULE]
    /// The native harness rewrites this file: a recorded member found missing is
    /// reported missing and re-added by setup instead of refusing (Claude: false).
    pub native_rewrites: bool,
}
#[derive(Clone, Copy)]
pub struct SettingsKind(&'static SettingsKindSpec); // Eq on (harness, file_name)
```

**What changes in the engine:**

- `SettingsKind::{harness, file_name, manages_permission (= owned_allow.is_some()), entries}` read the spec.
- `DECLARED_RULE` and `RETIRED_RULE` (`:151-154`) and `permission_valid`/`permission_current` (`:164-170`) take the rule from `kind.owned_allow`.
- The Claude hook groups (`claude::declared_hook_groups`) and the Codex builder (`codex_entries`, `:333-346`) become those adapters' `hook_groups`.
- The array stays at `permissions.allow`. Both Claude and agy use that path, so it is not generalised.
- Manifest bytes and paths are unchanged: `harness`, `permission{rule,fingerprint,pre_existing}`, and `manifest_path(state, "<id>-user", file)`.

**Allow-member-only mode** (when `hook_groups` is empty):

- **Install:** records `owned: []` plus `permission`. It creates `permissions`/`allow` when absent. A member the user already had is recorded `pre_existing: true` (the existing `permission_record`).
- **Ownership checks:** `recorded_entries_valid` and `shared_command` are not required for an empty `owned`.
- **Status:** `installed` means the member is present, whether owned or pre-existing. It is not gated on the whole-file fingerprint, because the harness rewrites the file.
- **Setup with a recorded member missing:**
  - with `native_rewrites`, the member is re-added and recorded again;
  - without it, setup refuses as today (`cli/setup.rs:137-138` exit 1).
- **Unsetup:** deletes only an owned member (`remove_allow`, `:223-256`). It prunes `allow`/`permissions` only when the recorded base lacked them.

This mirrors the owned-array-member semantics of `codex_config.rs`: pre-existing recorded, array created, owned member removed. It does so on the JSON engine Claude already uses.

**Discarded:**

- *An adapter-implemented settings engine for agy.* It duplicates the manifest, generation and atomic-replace machinery outside core, and would need core edits anyway to make doctor and status aware of it.
- *Generalising the array path.* No current or planned adapter needs it (YAGNI).
- *A trait method returning a list of kinds.* Each adapter manages one user settings file through the engine; agy's plugin directory uses owned assets, not this engine.

### D7. Doctor (`cli/doctor.rs`)

**What moves into the adapters.** The seam creates `src/harness/claude/doctor.rs` and `src/harness/codex/doctor.rs` as forwarders, and the doctor leaf moves the bodies into them:

- `legacy_claude_projection`/`legacy_codex_projection` (`:662-875`);
- `claude_observed_text`, `CLAUDE_NOT_OBSERVABLE` and `claude_verdict_line` (`:259-317`);
- `claude_path_line` (`:210-224`) and the `cfg(test)` installed helpers (`:105-248`);
- the Claude/Codex repair rules (`:1932-1943,1962-1989`).

**What stays in core:**

- `installed_declared` (`:191-207`) becomes neutral: `installed_declared(executable, recipe: Option<&str>, path)`.
- `repair_plan`/`add_manual_repairs` keep only the global items (context, state, daemon). Per-harness repairs come only from `DoctorProjection.safe_repairs`/`manual_repairs`, which the adapters already fill (`:759-772`).
- The summary's `fix:` line also fires when any `local_harnesses[id]` carries safe repairs.

**Text rendering:**

- `render_debug_text` (`:1624-1794`) calls `registration.doctor_text(&report.hooks[id], Debug)` in `adapter_order`.
- `render_text` (`:1829-1859`) calls `doctor_text(.., Summary)` and skips report limitations that start with a `doctor_summary_claims()` prefix. This replaces `:1888-1893`.
- A neutral helper `legacy_summary_lines(id, hooks)` produces today's `"{id}: {admission}, {setup}"` and runtime-metadata lines (`:1834-1845`). Claude and Codex call it from their Summary text.
- Hermes and others return `""`, which matches today: they are not in the `["claude","codex"]` loop.

**Frozen-wire allowlist:** the doctor JSON keys `daemon.harness_claude`/`daemon.harness_codex` (`:1316-1317`). They mirror v1 `HarnessHealth`. Their text rendering (`:1588-1589`) becomes neutral: it iterates `daemon` keys that start with `harness_`, in sorted order, which is the same order.

### D8. Setup reporting (`cli/setup.rs:745-754,956-966,1063-1110,1124-1147,1633-1659`)

- **Setup status:** the `if id == "claude" { complete_setup_status }` branch becomes `registration.setup_status_extras(&request, &mut projection)`.
- **Text lines:** `prompt_suggestion_line` and `mod_line` move behind Claude's `setup_report_lines`. Core prints each returned line as `{name}: {line}` at the same position.
- **Trust reminders:** the label prefix comes from `text().trust_reminder_label`.
- **Frozen compatibility (allowlisted):** the v1 setup JSON scalar `trust_reminder` (`:961-963`) and its text fallback (`:1108`). This scalar has always been Codex's note.
- **Re-exports:** the claude/codex re-exports (`:1639-1659`) are deleted. Importers, including tests, use the adapter paths.
- **Setup flags:** `--hooks-only` and the prompt-suggestion flags (`commands.rs:1684-1722`) are validated through `registration.setup_options()` with neutral error text.
- **Prompt-suggestion module:** `harness/prompt_suggestion.rs` (Claude's settings key editor) moves to `harness/claude/`.

### D9. Version syntax, contract-id, attribution, evidence (`harness/contract.rs`, `harness/attribution.rs`, `cli/hook_evidence.rs`, `daemon/harness_evidence.rs`)

- **Version normalization.** `normalize_version(harness, raw)` keeps its signature. That keeps callers at `protocol/commands.rs:192`, `daemon/harness_states.rs:479` and `attribution.rs:187` unchanged. Its body:
  1. looks up `version_syntax()`;
  2. trims;
  3. strips the suffix, then the prefix;
  4. parses with `Version::parse`.

  `None` is returned for an unknown harness or a harness with no syntax.
- **contract-id.** `normalize_examples`, `contract_ids_json` and `render_contract_ids` iterate the registry, keeping adapters with a native-payload contract (`contract_for`). They render in registry order, so the output is byte-identical today; an adapter added later only adds a key.
- **Attribution.**
  - `attribution.rs` keeps the bounded readers. `known_harness` and the `"codex"` branches (`:86-119,156-205`) are replaced by `transcript_version()`.
  - `attribute_native_runtime(harness: &str, input)` keeps its signature, so the adapter callers (`claude.rs:914`, `codex.rs:1423`) are unchanged.
  - The variants `CodexResumed`/`CodexCreatorOnly` are renamed `ResumedCreator`/`CreatorOnly`. Their `as_str` texts stay byte-identical and are allowlisted as frozen (they go on the wire and into doctor).
- **Evidence.**
  - `hook_evidence.rs:214` uses the contract descriptor's existing `AttributionHolding::SuppressResumed`.
  - `harness_evidence.rs:210` `holds_session_start` uses `AttributionHolding::UntilAttributed` on the harness's native-payload descriptor.
  - `codex_evidence::dir`/`prepare` (`hook_evidence.rs:355,394,628,741`) serve every harness's gate directory. They move to a neutral `harness::evidence` path.
- **Frozen (allowlisted):**
  - the v1 `HarnessEvidence::validate` and `HookParseFailure` checks for `"claude"|"codex"` (`protocol/commands.rs:187,1047`);
  - `admission::versions_document` (`:255-265`), the schema-2 `versions.json` generator byte-locked by `versions_json_guard`.

### D10. Hook runner (`cli/hook.rs`)

- **`InstalledHarness`** (`:247-252`) becomes `pub struct InstalledHarness(AgentHarnessId)`. It is a registered-contract handle that `observe_harness_in` returns for any `RegisteredContract` adapter. Other adapters keep today's `Err` path.
- **What is removed.** Today nothing in production builds the version-carrying variants: `InstallObservation::CodexWitness` has no producer outside `cli/hook.rs`. So the leaf deletes:
  - the `CodexWitness` arms: `installed_observation` (`:366-382`), `installed_compat` (`:2686-2703`), `observe_daemon_default` (`adapter.rs:924`) and `cli/launch.rs:865`;
  - `InstallObservation::CodexWitness` itself (`adapter.rs:229`).

  The leaf verifies that no producer exists before deleting. If one does, the variant is renamed `VersionWitness` instead.
- **`report_parse_failure`** (`:1200-1212`) sends `args.harness.as_str()` when that adapter has `legacy_contract_id().is_some()`. That keeps the v1 wire set.
- **`find_seat`** (`:1346`) uses the invoking hook's harness, not a hard-coded `Harness::Claude`. The leaf checks with `selected_generation` that this changes nothing for Claude and fixes Codex.
- **Variable rename:** `codex_start` becomes `empty_lifecycle_start` (`:544`).

### D11. Health, app, harness states (`app.rs`, `daemon/health.rs`, `daemon/harness_states.rs`)

- **Health inputs.** `HealthInputs.{codex,claude,additional_harnesses}` (`health.rs:106-109`) become one `harnesses: Vec<(String, HarnessStatus)>` in registry order.
  - Adapters with `legacy_contract_id().is_some()` are always present (`Unknown` until observed).
  - Others are present only when observed, as `app.rs:257-270` does today.
  - Limitation order and the `acceptable()`/cooperative folds (`:504-527,610-615`) iterate it. The order is unchanged: claude, codex, then the rest.
- **Frozen v1 projection.** `HarnessHealth{codex, claude}` (`health.rs:500-503`, `protocol/results.rs:691-696,731-734`) is the one permanent allowlisted projection.
- **Unseen versions.** `trigger_unseen_versions` input (`app.rs:1522-1525`) iterates registrations with a legacy contract.
- **Harness-state note.** The note at `harness_states.rs:602-606` becomes `text().version_history_note`, with no note when empty. Hermes stops receiving Codex's note, which fixes a latent mislabel; Hermes has no v1 evidence rows today.
- **Clean-up.** `COOPERATIVE_WAKE_LINE` (`health.rs:172`), which only tests use, moves under `cfg(test)`. The `cfg(test)` status helpers in `app.rs:332-430` move to the adapter test modules if the check flags them.

### D12. Store query (`store/queries.rs:6315`)

**Chosen:** `harness IN (SELECT value FROM json_each(?2))`, bound to a JSON array of `builtins()` ids. One extra bind and no string-built SQL. json1 is already used by the schema checks. This fixes the latent Hermes bug: Hermes bindings were not counted as the current agent.

**Discarded:** `harness <> 'human'`. It would count bindings of an unregistered harness.

### D13. Neutral vocabulary moved by the seam (move-only, inert)

- **Shell probe.** `CodexShellProbe`/`SystemShellProbe`/`SHELL_PROBE_TIMEOUT` move to `harness::launch` as `PaneShellProbe`/`SystemShellProbe`. These names appear in `LaunchPolicy` signatures that agy implements.
- **Process helpers.** `codex::bounded_output`, `version_output` and `VERSION_TIMEOUT` go to a new `harness::process`. Importers: `cli/setup.rs:254`, `harness/manifest.rs:932`, `app.rs:1495`, doctor.
- **Labels.** `SCHEMA_MATCHED_LABEL`/`OPTIMISTIC_LABEL` go to `harness::recipe`. Importers: `app.rs:454,458`, `doctor.rs:71-72`.
- **`BinaryIdentity`.** It moves to `harness::runtime`, and the `harness::BinaryIdentity` path is kept.
- **Operational contracts.** `ClaudeContract`/`CodexContract` (`harness/operational.rs`) move into their adapters.
- **Command guidance.** `cli/skill.rs:34` `CODEX_COMMAND_GUIDANCE` moves to the Codex adapter.
- **Doctor forwarders.** The empty forwarder modules `claude/doctor.rs` and `codex/doctor.rs` are created.

### D14. Adapter-owned paths and allowlist discipline

- **Consumed from ht-poh.1:** the check treats `src/harness/<id>.rs`, `src/harness/<id>/**` and `src/harness/<id>_*.rs` (`claude_mod.rs`, `codex_config.rs`, `codex_evidence.rs` after D9's move, `codex_schema.rs`) as adapter-owned for id ∈ `builtins()`. If ht-poh.1 lands narrower, the orchestrator amends it. This subtree does not relocate adapter files: relocation would churn every importer and collide with ht-poh.3/ht-poh.4 edits to `harness/claude.rs`.
- **Permanent registration entries:**
  - the `builtins()` rows and their asserts (`registry.rs:1219-1240`);
  - the `CODEX`/`CLAUDE` identities and `OccupantHarness`/`context::Harness` consts (`registry.rs:25-41`, `context.rs:31-32`), kept for tests and frozen V1 sites;
  - the `pub mod` declarations in `harness/mod.rs`.
- **Editing the allowlist.** Each leaf edits only the allowlist lines for paths it owns. The file stays sorted by path, so leaves never touch the same hunk.

### D15. Per-site disposition

| Site (verified) | Disposition | Leaf |
|---|---|---|
| `cli/mod.rs:264`, `cli/output.rs:173`, `cli/me.rs:38-45` | registry markers, assembled help, TRUST-POLICY A4 text | L2 |
| `cli/mod.rs:2311` `validate_actor_harness` | already `OccupantHarness::Agent(_)` vs `Human`; Hermes gap already closed; no change | — |
| `store/queries.rs:6315` | `json_each` over registry ids | L5 |
| `cli/setup.rs:205-210,356-583` roots | `default_config_root` and the `config_roots` map | L3 |
| `cli/setup.rs:60-144` `SETUP_HELP` | `cli/help.rs::setup_help()` | L2 |
| `cli/setup.rs:43,745,956-966,1063-1147,1633-1659` | D8; `trust_reminder` scalar allowlisted frozen | L3 |
| `cli/setup/foreground.rs` | D5 | L3 |
| `cli/launch.rs:70-71,74-132,949`; `cli/handoff.rs:26-58,1693-1705` | help, neutral text, opts list from registry (CLAUDE, CODEX, HERMES order; tests updated) | L2 |
| `cli/launch.rs:1122-1161` `"codex"` report/record key | permanent frozen (launch report/record v1 key, null for others) | L2 |
| `cli/launch.rs:865` `CodexWitness` arm | deleted with the variant | L6 (single-hunk exception) |
| `cli/topology_runtime.rs:242-246`, `cli/topology_handoff.rs:65-69` | `registry.agent(kind)`; eligibility decided by frozen V1 (D16) | L2 |
| `cli/commands.rs:1684-1722,1838` | `setup_options()` validation, neutral text | L2 |
| `cli/commands.rs:1063-1098,2930-2935` (watch) | out of scope: ht-poh.3 | — |
| `client/mod.rs:60` | neutral wording pointing at docs/install.md | L2 |
| `harness/launch.rs:14,170-173` re-exports | probe: L1 (D13); `CodexLaunchForm`/`scoped_codex_cwd` re-exports removed | L1 / L2 |
| `harness/launch.rs:255-460` V1 bootstrap grammar | permanent frozen compatibility record | L2 |
| `daemon/harness_states.rs:602` | `version_history_note` | L5 |
| `cli/doctor.rs` (all) | D7; `harness_claude`/`harness_codex` keys allowlisted frozen | L4 |
| `harness/contract.rs:266-323` | D9 | L6 |
| `harness/attribution.rs:44-205` | D9; reason strings allowlisted frozen | L6 |
| `daemon/harness_evidence.rs:210`; `cli/hook_evidence.rs:49,214,355,394,628,741` | D9 | L6 |
| `cli/hook.rs:247-382,544,1200-1212,1346,2686-2703` | D10 | L6 |
| `harness/adapter.rs:229,924` | `CodexWitness` removed | L6 |
| `harness/adapter.rs:767,788,806` | `PaneShellProbe` | L1 |
| `harness/setup.rs:108-142` | `SettingsKindSpec` (D6) | L1 |
| `harness/setup.rs:45-65,317-350,723-858,1773-1858` (Codex session plan, `plan_claude`/`uninstall_claude`) | Codex session-plan items move to `codex/setup.rs`; Claude-named functions become kind-generic | L3 |
| `harness/setup.rs` `legacy` (`:2047-2932`) | registry/kind-driven; `user_inspection(Harness, &SetupEnv)` and `installer_hooks_installed(&SetupEnv, Harness)` keep their signatures; the Codex allowance precheck (`:2063-2076`) moves to Codex's installer call site | L3 |
| `harness/prompt_suggestion.rs` | moved into the Claude adapter | L3 |
| `harness/registry.rs:25-41,1219-1240`, `context.rs:31-32`, `harness/mod.rs` mod decls | permanent registration | L1 |
| `harness/mod.rs:75`, `operational.rs`, `cli/skill.rs:34`, `manifest.rs:932`, `app.rs:454,458,1495` | D13 | L1 |
| `harness/admission.rs:255-265` | permanent frozen artifact | L6 |
| `harness/composer.rs:33` `CLAUDE_NOT_KNOWN_EMPTY` | moved to `harness/claude/composer.rs` | L5 |
| `app.rs:254-271,1522-1525`, `daemon/health.rs:106-109,172,426-427,500-527,610-615` | D11; v1 projection allowlisted frozen | L5 |
| `protocol/results.rs:691-696,731-734` | permanent frozen v1 `HarnessHealth` | L5 |
| `protocol/commands.rs:187,1047` | permanent frozen v1 validation | L6 |
| `store/schema.rs:1742` | permanent history (v9 shape check) | L5 |
| `store/receipts.rs:144-149` (mod ACK requires Claude) | out of scope: ht-poh.3 (PushChannel policy) | — |
| `service/dispatch.rs`, `service/mod_channels.rs`, `cli/watch.rs`, `protocol/watch.rs` | out of scope: ht-poh.3 | — |
| `ports.rs`, `protocol/handoff.rs`, `store/control.rs`, `host/native.rs`, `harness/cache.rs`, `protocol/authority.rs`, `protocol/service.rs` | matches are only in `cfg(test)` code or comments; no production site. The owning leaf deletes any `to-migrate` entries ht-poh.1 listed for them | L2 (ports, protocol/handoff), L5 (the rest) |

### D16. Boundaries and finding

- **ht-poh.3** owns:
  - the watch and delivery files;
  - `store/receipts.rs:144`;
  - the watch lines in `cli/commands.rs`;
  - `delivery_policy` in `adapter.rs`.
- **ht-poh.4** owns:
  - `local_evidence` in `adapter.rs`;
  - the runner's `local_evidence` call in `cli/hook.rs`;
  - its `builtins()` row.
- **Overlap.** These are additive edits to files this subtree also edits. Hunks are disjoint, and conflicts are resolved at merge.
- **Finding (R7, R13).** `compose_bootstrap_v1_argv` (`harness/launch.rs:266-281`) and `bootstrap_v1::retained_argv_matches` (`cli/handoff.rs:1585-1597`, `store/topology_handoff/attachment.rs:628`) admit only Claude and Codex, by design: they are a frozen compatibility record. So `handoff --pane|--new-tab --kind agy` cannot work without a new, adapter-neutral bootstrap plan version. Its retained composition would be the adapter's own `LaunchPolicy::compose_argv` result, frozen into the plan.
  - This sweep keeps V1 frozen and allowlisted, and does not add V2.
  - The orchestrator should open a bead, "Bootstrap plan V2: adapter-neutral retained composition", which ht-poh.4's handoff part depends on.
  - It is recorded as an R13 out-of-adapter cost.

## Acceptance

- `scripts/check-harness-neutral` passes and prints zero `to-migrate` entries. Every remaining entry is one of the permanent kinds in D14/D15 and carries a reason.
- These checks are green:
  - `nice cargo clippy --locked --all-targets --all-features -- -D warnings`;
  - `nice scripts/check-default-features`;
  - the focused tests of each leaf, including all existing Claude, Codex and Hermes setup, doctor, launch, hook, evidence, health and contract-id tests.
- The only test updates are:
  - import-path changes;
  - help-text wording;
  - the `env HERDR_THREADS_*_OPTS=` recovery list (now every registered key, in registry order);
  - registry-error variants.
- Extension proof: one test-only adapter, registered in a test `Registry`, declares all of these, and each shows up without any core edit:
  - an env marker;
  - `AdapterText` fragments;
  - a config root;
  - foreground advice;
  - an allow-member-only `SettingsKindSpec`;
  - doctor text;
  - version syntax.

  They appear respectively in:
  - `harness_marked` / `agent_env_marker`;
  - the assembled help;
  - `SetupEnv.config_roots`;
  - `setup-status`;
  - install, status and unsetup on a fixture file, including the pre-existing, array-absent, native-rewrite and member-removed cases;
  - doctor text;
  - `normalize_version` / `contract-id`.
- TRUST-POLICY A4 names markers as adapter-declared person-guard markers, in the same commit as L2.
- Every commit carries an `Outside-adapter:` trailer listing its files outside `src/harness/agy/`, which is all of them in this subtree.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
