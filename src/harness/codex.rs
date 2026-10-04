//! Codex hook adapter, selected by an evidence-backed recipe registry
//! ([`RECIPES`]). Input mapping is supported for the installed versions a
//! recipe covers; live lifecycle coverage remains unqualified.
//!
//! Native invocation transport (an approving `permissionDecision` plus
//! `updatedInput` rewrite) and model-issued receipt are UNSUPPORTED in every
//! recipe: no isolated native run has demonstrated that such a rewrite
//! preserves ordinary deny/approval decisions or reaches the daemon socket
//! under the intended policy. This adapter therefore emits context only. Its
//! setup declaration installs context hooks only: lifecycle, child start, and
//! a Bash `PreToolUse` group whose output is `additionalContext` and never a
//! permission decision or `updatedInput`.
pub mod setup;

use super::adapter::*;
use super::admission::{self, OptimisticAdmission, Refusal, Row};
use super::codex_schema::{self, Unextractable};
use super::context::{ContextError, EventKind, Harness, Role};
use super::contract::{
    EventClass, EventContract, HarnessContract,
    JsonType::{String as Str, StringOrNull as StrOrNull},
    field as f,
};
pub use super::recipe::NativeSupport;
use super::recipe::{self, Evidence, LookupError, Recipe, Version, VersionSet};
use super::{Capability, LifecycleEvent, declared_role, field, input};
use crate::protocol::results::CapabilityState;
use crate::protocol::time::CallBudget;
use crate::protocol::time::{Cancellation, external_bound};
use serde_json::{Value, json};
use std::{
    fmt,
    io::Read,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// The native hook payload contract `check_hooks_v1`, `parse_shape` and
/// `parse_tool_invocation` consume. Kept in step with the parsers by the drift
/// tests in `tests/harness/contract.rs`: a parser that requires an undeclared
/// field fails them. Changing it changes the contract id, which must be
/// deliberate.
pub const CONTRACT: HarnessContract = HarnessContract {
    harness: "codex",
    discriminator: "hook_event_name",
    events: &[
        EventContract {
            event: "SessionStart",
            class: EventClass::Lifecycle,
            fields: &[
                f("hook_event_name", Str, true),
                f("session_id", Str, true),
                f("source", Str, true),
                f("turn_id", Str, false),
                f("agent_id", Str, false),
                f("agent_type", Str, false),
            ],
        },
        EventContract {
            event: "SubagentStart",
            class: EventClass::Other,
            fields: &[
                f("hook_event_name", Str, true),
                f("session_id", Str, true),
                f("turn_id", Str, true),
                f("cwd", Str, true),
                f("model", Str, true),
                f("permission_mode", Str, true),
                f("agent_id", Str, true),
                f("agent_type", Str, true),
                f("transcript_path", StrOrNull, true),
            ],
        },
        EventContract {
            event: "PreToolUse",
            class: EventClass::Tool,
            fields: &[
                f("hook_event_name", Str, true),
                f("session_id", Str, true),
                f("turn_id", Str, true),
                f("tool_name", Str, true),
                f("tool_use_id", Str, true),
                f("tool_input.command", Str, true),
                f("agent_id", Str, false),
                f("agent_type", Str, false),
            ],
        },
    ],
};

/// Native hook-input schema a recipe parses. One variant per distinct
/// captured schema; recipes whose payloads are identical share a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSchema {
    /// The `*.command.input` hook schemas embedded in Codex 0.155.1, 0.157.1
    /// and 0.158.0, which are byte-identical across those releases.
    HooksV1,
}

/// Per-recipe Codex behaviour and evidence-backed capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexProfile {
    pub input_schema: InputSchema,
    /// `sha256:<hex>` fingerprint ([`super::codex_schema::fingerprint`]) of
    /// the embedded hook command schemas captured for this recipe. An
    /// installed version the recipe does not list is admitted under this
    /// recipe, as schema-matched and live-unverified, only when the schemas
    /// embedded in its binary produce exactly this fingerprint.
    pub schema_fingerprint: &'static str,
    pub input_mapping: Capability,
    pub invocation_transport: NativeSupport,
    pub model_receipt: NativeSupport,
    /// Poke capabilities (spec §10). The spike tested Codex 0.160.0 only, and
    /// no recipe covers it, so every recipe leaves both unsupported.
    pub composer_stash: NativeSupport,
    pub poke_during_turn: NativeSupport,
}

pub type CodexRecipe = Recipe<CodexProfile>;

/// Fingerprint of the 23 `*.command.{input,output}` schemas embedded in Codex
/// 0.157.1 and 0.158.0 (and 0.155.1), computed from the committed extraction
/// `codex-158-hook-capture/schemas-0.158.0/`, whose file digests equal the
/// 0.157.1 extraction's `schemas-0.157.1.SHA256SUMS`. The recipe tests
/// recompute it from those files.
pub const HOOKS_V1_SCHEMA_FINGERPRINT: &str =
    "sha256:86858f2456c999030224a92d8dfb535183fe0edf8601690d8941978fadbb066d";

/// Evidence-backed Codex recipes. An installed version outside every recipe
/// is refused by [`InstalledVersion::observe`]; adding a version requires
/// captured evidence of its hook payloads.
pub const RECIPES: &[CodexRecipe] = &[Recipe {
    id: "codex-hooks-v1",
    versions: VersionSet::Exact(&[
        Version::new(0, 157, 1),
        Version::new(0, 158, 0),
        Version::new(0, 159, 3),
    ]),
    evidence: &[
        "docs/compatibility/codex-probe.md",
        "docs/evidence/codex-158-hook-capture/report.md",
        "docs/evidence/codex-158-live-hook-capture/report.md",
        "docs/validation/report.md",
    ],
    evidence_levels: &[
        (Version::new(0, 157, 1), Evidence::NoModel),
        (Version::new(0, 158, 0), Evidence::Live),
        (Version::new(0, 159, 3), Evidence::Live),
    ],
    known_broken: &[],
    scope: "0.157.1: native root/child PreToolUse input captured and source-read hook contract; \
            0.158.0: embedded hook input/output schemas byte-identical to 0.157.1, live \
            SessionStart startup/resume, SubagentStart and root/child Bash PreToolUse input \
            captured, and context-only additionalContext delivery to the model observed for \
            SessionStart and PreToolUse (not SubagentStart). permission_mode is always \
            bypassPermissions under exec and is not a sandbox signal. SessionStart fork is \
            known-unsupported (never captured). Transport and receipt are not qualified for \
            either; 0.159.3: the ht-p03.20 native matrix manual and managed core-flow cells, run \
            from the fixed install path on the evidence SHA in docs/validation/report.md",
    profile: CodexProfile {
        input_schema: InputSchema::HooksV1,
        schema_fingerprint: HOOKS_V1_SCHEMA_FINGERPRINT,
        input_mapping: Capability::ObservedInput,
        invocation_transport: NativeSupport::Unsupported,
        model_receipt: NativeSupport::Unsupported,
        composer_stash: NativeSupport::Unsupported,
        poke_during_turn: NativeSupport::Unsupported,
    },
}];

/// The table every admission entry point classifies against: [`RECIPES`],
/// except that test builds honor `HT_TEST_RECIPES_JSON` (see
/// [`admission::override_table`]).
pub fn admission_table() -> &'static [CodexRecipe] {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(table) = admission::override_table("codex", RECIPES, |key| std::env::var_os(key)) {
        return table;
    }
    RECIPES
}

/// The recipe LISTING an installed version string, if any.
pub fn recipe_for(installed: &str) -> Result<&'static CodexRecipe, LookupError> {
    recipe::lookup(RECIPES, installed)
}

/// Why setup owns a native hook group. Every purpose is context-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPurpose {
    /// `SessionStart` lifecycle check-in context.
    Lifecycle,
    /// `SubagentStart` child context; children never gain root authority.
    ChildStart,
    /// Bash `PreToolUse` bounded attention at tool boundaries. Output is only
    /// `hookSpecificOutput.{hookEventName, additionalContext}`; 0.157.1 accepts
    /// that shape without a decision (source-read), and a live 0.158.0 run
    /// delivered it to the model without blocking the tool call.
    ToolBoundaryContext,
}

/// One hook group setup owns, exactly as it is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedHook {
    pub event: &'static str,
    pub matcher: Option<&'static str>,
    pub purpose: HookPurpose,
}

/// What this adapter claims about the native harness. Health and setup read
/// this instead of restating support decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration {
    /// The recipe registry every versioned entry selects from.
    pub recipes: &'static [CodexRecipe],
    /// Hook groups setup owns, in install order. Setup and inspection read
    /// this list; none of them depends on the gated invocation rewrite.
    pub owned_hooks: &'static [OwnedHook],
    pub limitation: &'static str,
}

pub const DECLARATION: Declaration = Declaration {
    recipes: RECIPES,
    owned_hooks: &[
        OwnedHook {
            event: "SessionStart",
            matcher: None,
            purpose: HookPurpose::Lifecycle,
        },
        OwnedHook {
            event: "SubagentStart",
            matcher: None,
            purpose: HookPurpose::ChildStart,
        },
        OwnedHook {
            event: "PreToolUse",
            matcher: Some("^Bash$"),
            purpose: HookPurpose::ToolBoundaryContext,
        },
    ],
    limitation: "codex invocation transport unsupported: approving updatedInput rewrite, \
                 approval preservation and scoped socket reachability are unproven; \
                 model receipt unsupported",
};

impl Declaration {
    /// Whether setup installs a context-only tool-boundary hook.
    pub fn tool_boundary_context(&self) -> bool {
        self.owned_hooks
            .iter()
            .any(|hook| hook.purpose == HookPurpose::ToolBoundaryContext)
    }

    /// Health cannot know which installed version a future session runs, so
    /// Codex is `supported` only if every recipe proves transport and receipt.
    pub fn health_capability(&self) -> CapabilityState {
        if !self.recipes.is_empty()
            && self.recipes.iter().all(|recipe| {
                recipe.profile.invocation_transport == NativeSupport::Supported
                    && recipe.profile.model_receipt == NativeSupport::Supported
            })
        {
            CapabilityState::Supported
        } else {
            CapabilityState::Unsupported
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// The binary could not be run, timed out, or exited unsuccessfully.
    Unavailable,
    /// The output was not exactly one `codex-cli <version>` line.
    Unrecognized,
    /// A well-formed version no recipe covers, refused without consulting a
    /// binary's embedded schemas (no binary was observed).
    Unsupported(String),
    /// A version inside a recipe's `known_broken` range (ladder row 2),
    /// refused even when a recipe lists it or it is newer than every max.
    KnownBroken {
        version: String,
        range: VersionSet,
        newest_working: Option<Version>,
    },
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str(&recipe::unavailable_message(
                "codex",
                "not an absolute runnable binary, failed, or timed out",
                "Install a supported version and point the hook at its absolute executable",
                RECIPES,
            )),
            Self::Unrecognized => write!(
                f,
                "codex --version output is not `codex-cli X.Y.Z`; supported recipes: {}",
                recipe::describe(RECIPES)
            ),
            Self::Unsupported(version) => {
                f.write_str(&recipe::unsupported_message("codex", version, RECIPES))
            }
            Self::KnownBroken {
                version,
                range,
                newest_working,
            } => f.write_str(&admission::known_broken_message(
                "codex",
                version,
                range,
                *newest_working,
            )),
        }
    }
}

/// A hook entry that cannot obtain a version witness refuses with the same
/// actionable, recipe-naming text as the Claude entries
/// ([`ContextError::UnsupportedVersion`]). Every variant, including
/// `Unavailable` (the binary could not be observed at all), names the
/// supported recipes.
impl From<VersionError> for ContextError {
    fn from(error: VersionError) -> Self {
        Self::UnsupportedVersion(error.to_string())
    }
}

/// How an observed installed version was admitted to its recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// The recipe lists the version: exact, evidence-backed behaviour.
    Listed,
    /// The recipe does not list the version, but the hook schemas embedded
    /// in the observed binary hash-match the recipe's captured schemas. The
    /// hook contract is statically identical; no live capture exists for
    /// this version (schema-matched, live-unverified).
    SchemaMatched {
        /// `sha256:<hex>` fingerprint of the embedded schemas.
        fingerprint: String,
        /// Hex SHA-256 of the fingerprinted binary.
        binary_sha256: String,
    },
    /// No recipe lists the version and no recipe's captured schemas match the
    /// binary's, but the version is not older than every recipe: admitted
    /// under an assumed recipe, live-unverified (ladder row 6).
    Optimistic {
        admission: OptimisticAdmission,
        /// What the binary's embedded hook schemas showed.
        schema: SchemaObservation,
    },
}

/// What fingerprinting an optimistically admitted binary's embedded hook
/// schemas showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaObservation {
    /// The schemas were read and match no recipe's captured schemas.
    Drift { fingerprint: String },
    /// The schemas could not be fingerprinted.
    Unreadable { reason: Unextractable },
}

/// Witness that a specific installed Codex binary reported a version some
/// recipe covers — listed by the recipe, or schema-matched to it — together
/// with that recipe and how it was admitted. The only production constructor
/// runs that binary; there is no public string-based entry that skips the
/// observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVersion {
    version: String,
    recipe: &'static CodexRecipe,
    admission: Admission,
    /// The file whose embedded schemas were fingerprinted: the native vendor
    /// binary when the observed `codex` is the npm JS wrapper. `None` when no
    /// fingerprint was taken (a listed version never reads the binary).
    fingerprinted: Option<std::path::PathBuf>,
}

/// The fixed evidence label for a schema-matched admission.
pub const SCHEMA_MATCHED_LABEL: &str = "schema-matched, live-unverified";

/// The fixed state label for an optimistic admission. The operator-facing
/// wording rendered from [`OptimisticAdmission`] is the doctor's.
pub const OPTIMISTIC_LABEL: &str = "optimistic";

/// Codex versions on which setup's sandbox allowance (default-deny of the
/// network proxy) was measured: measurement data about versions, not an
/// admission claim, so it lives beside the recipe tables rather than in
/// `cli::setup`. See `cli::setup::codex_unmeasured_allowance_warning` for why
/// setup writes the allowance only for these.
pub const SANDBOX_MEASURED_VERSIONS: &[&str] = &["0.159.2", "0.159.3"];

pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_OUTPUT_LIMIT: u64 = 256;

impl InstalledVersion {
    /// Run `<absolute binary> --version` and accept a version a recipe lists,
    /// or an unlisted version whose embedded hook schemas hash-match a
    /// recipe. One [`VERSION_TIMEOUT`] deadline bounds the whole observation.
    pub fn observe(binary: &Path) -> Result<Self, VersionError> {
        Self::observe_cached(binary, VERSION_TIMEOUT, None)
    }

    /// [`Self::observe`] under a caller deadline (capped at
    /// [`VERSION_TIMEOUT`]).
    pub fn observe_within(binary: &Path, timeout: Duration) -> Result<Self, VersionError> {
        Self::observe_cached(binary, timeout, None)
    }

    /// Observe `binary` within `timeout` (capped at [`VERSION_TIMEOUT`]). One
    /// deadline bounds the whole observation: `--version` process exit,
    /// reading its stdout to EOF and, for an unlisted version only, reading,
    /// hashing and schema-fingerprinting the binary. A listed version never
    /// reads the binary. `cache_file` is an optional persistent fingerprint
    /// cache inside a caller-owned private directory.
    pub fn observe_cached(
        binary: &Path,
        timeout: Duration,
        cache_file: Option<&Path>,
    ) -> Result<Self, VersionError> {
        Self::observe_with(
            binary,
            timeout,
            cache_file.map_or(
                codex_schema::FingerprintCache::Memory,
                codex_schema::FingerprintCache::ReadWrite,
            ),
        )
    }

    /// [`Self::observe_cached`] with an explicit fingerprint cache mode.
    pub fn observe_with(
        binary: &Path,
        timeout: Duration,
        cache: codex_schema::FingerprintCache<'_>,
    ) -> Result<Self, VersionError> {
        Self::observe_with_cancel(binary, timeout, cache, None)
    }

    /// [`Self::observe_with`] whose `--version` run is killed (process group
    /// included) once `cancel` fires. The schema fingerprint stays
    /// deadline-bounded only: it runs after a version was reported, so a hung
    /// binary never reaches it.
    pub(crate) fn observe_with_cancel(
        binary: &Path,
        timeout: Duration,
        cache: codex_schema::FingerprintCache<'_>,
        cancel: Option<&Cancellation>,
    ) -> Result<Self, VersionError> {
        let deadline = Instant::now() + external_bound(timeout.min(VERSION_TIMEOUT));
        // Taken before the run, so an unlisted version is only admitted when
        // the binary fingerprinted is the binary that reported it.
        let target = codex_schema::fingerprint_target(binary);
        let ran = target
            .as_deref()
            .ok()
            .and_then(codex_schema::BinaryIdentity::observe);
        let stdout = version_output_by(binary, deadline, cancel)?;
        let version = Self::reported(&stdout)?;
        let table = admission_table();
        // Row 4's lazy closure: fingerprint the binary only for a version
        // that is not listed, not known broken and not older than every min.
        let mut matched: Option<(String, String)> = None;
        let mut observation: Option<SchemaObservation> = None;
        let mut fingerprinted: Option<std::path::PathBuf> = None;
        let row = admission::classify(table, &version, || {
            let target = match &target {
                Ok(target) => target,
                Err(reason) => {
                    observation = Some(SchemaObservation::Unreadable {
                        reason: reason.clone(),
                    });
                    return None;
                }
            };
            fingerprinted = Some(target.clone());
            let measured = match codex_schema::fingerprint_binary_with(target, deadline, cache) {
                Ok(measured) if ran.as_ref() == Some(&measured.identity) => measured,
                Ok(_) => {
                    observation = Some(SchemaObservation::Unreadable {
                        reason: Unextractable::Changed,
                    });
                    return None;
                }
                Err(reason) => {
                    observation = Some(SchemaObservation::Unreadable { reason });
                    return None;
                }
            };
            let recipe = table
                .iter()
                .find(|recipe| recipe.profile.schema_fingerprint == measured.fingerprint);
            match recipe {
                Some(_) => {
                    matched = Some((measured.fingerprint, measured.binary_sha256));
                }
                None => {
                    observation = Some(SchemaObservation::Drift {
                        fingerprint: measured.fingerprint,
                    });
                }
            }
            recipe
        });
        match row {
            Row::Listed(recipe) => Ok(Self {
                version,
                recipe,
                admission: Admission::Listed,
                fingerprinted,
            }),
            Row::SchemaMatched(recipe) => {
                let (fingerprint, binary_sha256) =
                    matched.expect("a schema match records its fingerprint");
                Ok(Self {
                    version,
                    recipe,
                    admission: Admission::SchemaMatched {
                        fingerprint,
                        binary_sha256,
                    },
                    fingerprinted,
                })
            }
            Row::Optimistic { recipe, admission } => Ok(Self {
                version,
                recipe,
                admission: Admission::Optimistic {
                    admission,
                    schema: observation.unwrap_or(SchemaObservation::Unreadable {
                        reason: Unextractable::NoSchemas,
                    }),
                },
                fingerprinted,
            }),
            Row::Refused(Refusal::Unparsable) => Err(VersionError::Unrecognized),
            Row::Refused(Refusal::OlderThanSupported(_)) => Err(VersionError::Unsupported(version)),
            Row::Refused(Refusal::KnownBroken {
                range,
                newest_working,
            }) => Err(VersionError::KnownBroken {
                version,
                range,
                newest_working,
            }),
        }
    }

    /// The version text of one `codex-cli <version>` line (not yet parsed).
    fn reported(stdout: &[u8]) -> Result<String, VersionError> {
        if stdout.len() as u64 > VERSION_OUTPUT_LIMIT {
            return Err(VersionError::Unrecognized);
        }
        let text = std::str::from_utf8(stdout).map_err(|_| VersionError::Unrecognized)?;
        let line = text.strip_suffix('\n').unwrap_or(text);
        let version = line
            .strip_prefix("codex-cli ")
            .ok_or(VersionError::Unrecognized)?;
        Ok(version.to_owned())
    }

    /// Listed-recipe parse of `--version` output with no binary to
    /// fingerprint: an unlisted version is `Unsupported`.
    #[cfg(test)]
    fn from_output(stdout: &[u8]) -> Result<Self, VersionError> {
        let version = Self::reported(stdout)?;
        match recipe_for(&version) {
            Ok(recipe) => Ok(Self {
                version,
                recipe,
                admission: Admission::Listed,
                fingerprinted: None,
            }),
            Err(LookupError::Unrecognized) => Err(VersionError::Unrecognized),
            Err(LookupError::Unsupported(_)) => Err(VersionError::Unsupported(version)),
        }
    }

    /// The file whose embedded schemas were fingerprinted, if any.
    pub fn fingerprinted(&self) -> Option<&Path> {
        self.fingerprinted.as_deref()
    }

    pub fn as_str(&self) -> &str {
        &self.version
    }

    /// The recipe the observed version selected.
    pub fn recipe(&self) -> &'static CodexRecipe {
        self.recipe
    }

    /// How the observed version was admitted.
    pub fn admission(&self) -> &Admission {
        &self.admission
    }

    /// One bounded line (at most 256 bytes) of admission evidence for
    /// doctor, Health and the hook's stored evidence, e.g.
    /// `codex 0.158.0: listed recipe codex-hooks-v1`,
    /// `codex 0.159.3: optimistic (newer-than-verified): assumed recipe
    /// codex-hooks-v1; schema drift sha256:…` or
    /// `codex 0.159.2: schema-matched, live-unverified: recipe
    /// codex-hooks-v1 hook schemas sha256:…; binary sha256 <16 hex>` (the
    /// full binary digest stays in [`Admission::SchemaMatched`]).
    pub fn evidence(&self) -> String {
        let line = match &self.admission {
            Admission::Listed => {
                format!("codex {}: listed recipe {}", self.version, self.recipe.id)
            }
            Admission::SchemaMatched {
                fingerprint,
                binary_sha256,
            } => format!(
                "codex {}: {SCHEMA_MATCHED_LABEL}: recipe {} hook schemas {fingerprint}; \
                 binary sha256 {}",
                self.version,
                self.recipe.id,
                &binary_sha256[..binary_sha256.len().min(16)]
            ),
            Admission::Optimistic { admission, schema } => {
                let schema = match schema {
                    SchemaObservation::Drift { fingerprint } => format!("drift {fingerprint}"),
                    SchemaObservation::Unreadable { reason } => {
                        format!("unreadable ({reason})")
                    }
                };
                format!(
                    "codex {}: {OPTIMISTIC_LABEL} ({}): assumed recipe {}; schema {schema}",
                    self.version,
                    admission.placement.label(),
                    admission.assumed_recipe
                )
            }
        };
        line.chars().take(256).collect()
    }

    #[cfg(test)]
    pub(crate) fn pinned_for_test() -> Self {
        Self::recipe_version_for_test("0.157.1")
    }

    /// Test witness for a version string, through the same registry lookup
    /// and output parser as `observe`.
    #[cfg(test)]
    pub(crate) fn recipe_version_for_test(version: &str) -> Self {
        Self::from_output(format!("codex-cli {version}\n").as_bytes()).expect("recipe version")
    }
}

/// Run `<absolute binary> --version` under one deadline covering process exit
/// and reading stdout to EOF; the process group is killed on expiry. Shared by
/// every adapter that gates on an observed installed version.
pub(crate) fn version_output(binary: &Path, timeout: Duration) -> Result<Vec<u8>, VersionError> {
    version_output_by(binary, Instant::now() + external_bound(timeout), None)
}

/// [`version_output`] that also kills the process group once `cancel` fires.
pub(crate) fn version_output_cancellable(
    binary: &Path,
    timeout: Duration,
    cancel: &Cancellation,
) -> Result<Vec<u8>, VersionError> {
    version_output_by(
        binary,
        Instant::now() + external_bound(timeout),
        Some(cancel),
    )
}

/// Run `<absolute binary> --version` and return its stdout before
/// `deadline`. A process that exits but leaves a descendant holding stdout is
/// `Unavailable`; its process group is killed.
fn version_output_by(
    binary: &Path,
    deadline: Instant,
    cancel: Option<&Cancellation>,
) -> Result<Vec<u8>, VersionError> {
    bounded_output_by(
        binary,
        &["--version"],
        VERSION_OUTPUT_LIMIT,
        deadline,
        cancel,
    )
}

/// Run `<absolute binary> ARGS...` with no stdin and return at most `limit`
/// bytes of its stdout, only for a successful exit before `timeout`; the
/// process group is killed on expiry. Used for read-only CLI queries.
pub(crate) fn bounded_output(
    binary: &Path,
    args: &[&str],
    limit: u64,
    timeout: Duration,
) -> Result<Vec<u8>, VersionError> {
    bounded_output_by(
        binary,
        args,
        limit,
        Instant::now() + external_bound(timeout),
        None,
    )
}

fn bounded_output_by(
    binary: &Path,
    args: &[&str],
    limit: u64,
    deadline: Instant,
    cancel: Option<&Cancellation>,
) -> Result<Vec<u8>, VersionError> {
    let cancelled = || cancel.is_some_and(Cancellation::is_cancelled);
    if !binary.is_absolute() {
        return Err(VersionError::Unavailable);
    }
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| VersionError::Unavailable)?;
    let group = child.id() as libc::pid_t;
    let kill_group = || {
        // SAFETY: signals only the fresh process group created for this
        // observation (leader pid == group id).
        unsafe { libc::killpg(group, libc::SIGKILL) };
    };
    let Some(stdout) = child.stdout.take() else {
        kill_group();
        let _ = child.wait();
        return Err(VersionError::Unavailable);
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline && !cancelled() => {
                std::thread::sleep(Duration::from_millis(5))
            }
            _ => {
                kill_group();
                let _ = child.kill();
                let _ = child.wait();
                return Err(VersionError::Unavailable);
            }
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let stdout = match receiver.recv_timeout(remaining) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return Err(VersionError::Unavailable),
        Err(_) => {
            // A descendant still holds stdout past the deadline.
            kill_group();
            return Err(VersionError::Unavailable);
        }
    };
    if !status.success() {
        return Err(VersionError::Unavailable);
    }
    Ok(stdout)
}

/// First absolute, executable `codex` on `path` (a `PATH` value).
pub fn resolve_on_path(path: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("codex"))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// Why no admitted `codex` was found on `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledRefusal {
    /// No absolute executable `codex` on `PATH`.
    NotFound,
    /// The binary found was observed and refused.
    Refused(VersionError),
}

impl InstalledRefusal {
    /// One short line (Health bound) naming the refusal.
    pub fn summary(&self) -> String {
        match self {
            Self::NotFound => "codex: no absolute executable `codex` on PATH".into(),
            Self::Refused(error) => error.summary(),
        }
    }
}

impl fmt::Display for InstalledRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str(&self.summary()),
            Self::Refused(error) => error.fmt(f),
        }
    }
}

impl VersionError {
    /// One short line (Health bound); [`fmt::Display`] is the full
    /// actionable refusal.
    pub fn summary(&self) -> String {
        match self {
            Self::Unavailable => "codex: --version could not be observed; refused".into(),
            Self::Unrecognized => "codex: --version output unrecognized; refused".into(),
            Self::Unsupported(version) => format!("codex {version}: no adapter recipe; refused"),
            Self::KnownBroken {
                version,
                range,
                newest_working,
            } => crate::harness::known_broken_label("codex", version, range, *newest_working),
        }
    }
}

/// The admission of the `codex` found on a `PATH`, for doctor and Health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledAdmission {
    /// The absolute executable observed, if one was found.
    pub binary: Option<std::path::PathBuf>,
    pub result: Result<InstalledVersion, InstalledRefusal>,
}

impl InstalledAdmission {
    /// Observe the `codex` found on `path` (a `PATH` value) within
    /// `timeout`. Read-only: no persistent cache is written.
    pub fn observe_on_path(path: Option<&std::ffi::OsStr>, timeout: Duration) -> Self {
        Self::observe_on_path_with(path, timeout, None)
    }

    /// [`Self::observe_on_path`] whose `--version` run is killed once
    /// `cancel` fires.
    pub(crate) fn observe_on_path_cancellable(
        path: Option<&std::ffi::OsStr>,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> Self {
        Self::observe_on_path_with(path, timeout, Some(cancel))
    }

    fn observe_on_path_with(
        path: Option<&std::ffi::OsStr>,
        timeout: Duration,
        cancel: Option<&Cancellation>,
    ) -> Self {
        let Some(binary) = resolve_on_path(path) else {
            return Self {
                binary: None,
                result: Err(InstalledRefusal::NotFound),
            };
        };
        let result = InstalledVersion::observe_with_cancel(
            &binary,
            timeout,
            codex_schema::FingerprintCache::Memory,
            cancel,
        )
        .map_err(InstalledRefusal::Refused);
        Self {
            binary: Some(binary),
            result,
        }
    }

    /// Observe the resolved absolute `binary` within `timeout`, using `cache`
    /// for an unlisted version's schema fingerprint.
    pub fn observe_binary(
        binary: std::path::PathBuf,
        timeout: Duration,
        cache: codex_schema::FingerprintCache<'_>,
    ) -> Self {
        let result = InstalledVersion::observe_with(&binary, timeout, cache)
            .map_err(InstalledRefusal::Refused);
        Self {
            binary: Some(binary),
            result,
        }
    }

    /// One bounded line: the admitted version's
    /// [`InstalledVersion::evidence`] or the refusal summary.
    pub fn line(&self) -> String {
        match &self.result {
            Ok(version) => version.evidence(),
            Err(refusal) => refusal.summary(),
        }
    }

    /// `listed`, `schema-matched, live-unverified`, `optimistic`, `refused` or
    /// `not_found`.
    pub fn state(&self) -> &'static str {
        match &self.result {
            Ok(version) => match version.admission() {
                Admission::Listed => "listed",
                Admission::SchemaMatched { .. } => SCHEMA_MATCHED_LABEL,
                Admission::Optimistic { .. } => OPTIMISTIC_LABEL,
            },
            Err(InstalledRefusal::NotFound) => "not_found",
            Err(InstalledRefusal::Refused(_)) => "refused",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The native rewrite path is gated off by `DECLARATION`.
    Unsupported(&'static str),
    Context(ContextError),
}

/// A parsed native Bash `PreToolUse` input together with the recipe of the
/// observed installed version that produced it.
///
/// Every field is private: the recipe carries the transport authority that
/// [`ToolInvocation::scoped_command`] consults, so the only constructor is
/// [`parse_tool_invocation`], whose recipe comes from an [`InstalledVersion`]
/// witness. Code outside this module cannot build one with its own recipe:
///
/// ```compile_fail
/// use herdr_threads::harness::codex::ToolInvocation;
/// // Kills: making the authority-bearing `recipe` field public again.
/// fn forge(invocation: &ToolInvocation) -> &'static herdr_threads::harness::codex::CodexRecipe {
///     invocation.recipe
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInvocation {
    event: LifecycleEvent,
    /// The recipe of the installed version that produced this input.
    recipe: &'static CodexRecipe,
    turn_id: String,
    tool_use_id: String,
    command: String,
}
impl ToolInvocation {
    pub fn event(&self) -> &LifecycleEvent {
        &self.event
    }

    /// The recipe of the observed installed version that produced this input.
    pub fn recipe(&self) -> &'static CodexRecipe {
        self.recipe
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub fn tool_use_id(&self) -> &str {
        &self.tool_use_id
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    /// Explicit unsupported gate: no scoped command is produced until the
    /// selected recipe is a registered one ([`DECLARATION`]`.recipes`) that
    /// records proven native invocation transport. Registration is checked by
    /// value, so a recipe that is not in the registry never opens the gate,
    /// whatever its transport flag says.
    pub fn scoped_command(&self) -> Result<String, TransportError> {
        if self.event.role != Role::TopLevel {
            return Err(TransportError::Context(ContextError::Child));
        }
        if !DECLARATION.recipes.contains(self.recipe)
            || self.recipe.profile.invocation_transport != NativeSupport::Supported
        {
            return Err(TransportError::Unsupported(DECLARATION.limitation));
        }
        scoped_bash_command(&self.command, &self.tool_use_id).map_err(TransportError::Context)
    }

    /// Test-only forgery seam: an invocation carrying an arbitrary recipe, to
    /// prove the transport gate refuses recipes outside the registry.
    #[cfg(test)]
    pub(crate) fn forged_for_test(
        event: LifecycleEvent,
        recipe: &'static CodexRecipe,
        tool_use_id: &str,
        command: &str,
    ) -> Self {
        Self {
            event,
            recipe,
            turn_id: "turn-forged".into(),
            tool_use_id: tool_use_id.into(),
            command: command.into(),
        }
    }
}

pub fn parse_tool_invocation(
    bytes: &[u8],
    event_id: &str,
    version: &InstalledVersion,
) -> Result<ToolInvocation, ContextError> {
    let event = parse_event_for_version(bytes, event_id, version)?;
    if event.kind != EventKind::Tool {
        return Err(ContextError::Invalid);
    }
    let value = input(bytes, event_id)?;
    let turn_id = field(&value, "turn_id")?.ok_or(ContextError::Invalid)?;
    let tool_use_id = field(&value, "tool_use_id")?.ok_or(ContextError::Invalid)?;
    let command = value["tool_input"]["command"]
        .as_str()
        .ok_or(ContextError::Invalid)?
        .to_owned();
    Ok(ToolInvocation {
        event,
        recipe: version.recipe(),
        turn_id,
        tool_use_id,
        command,
    })
}

fn required(value: &Value, key: &str) -> Result<String, ContextError> {
    field(value, key)?.ok_or(ContextError::Invalid)
}

/// The sole parser entry. The version witness can only come from observing
/// the installed binary, so no caller can parse native input unmeasured. The
/// witness's recipe selects the input schema.
pub fn parse_event_for_version(
    bytes: &[u8],
    event_id: &str,
    version: &InstalledVersion,
) -> Result<LifecycleEvent, ContextError> {
    let recipe = version.recipe();
    let value = input(bytes, event_id)?;
    match recipe.profile.input_schema {
        InputSchema::HooksV1 => check_hooks_v1(&value)?,
    }
    let mut event = parse_shape(&value, event_id)?;
    event.capability = match version.admission() {
        Admission::Listed => recipe.profile.input_mapping,
        // The stored evidence of a schema-matched parse never claims the
        // recipe's live-observed input mapping.
        Admission::SchemaMatched { .. } => Capability::SchemaMatchedInput,
        Admission::Optimistic { .. } => Capability::OptimisticInput,
    };
    Ok(event)
}

/// Adapter-side checks for [`InputSchema::HooksV1`] owned events.
fn check_hooks_v1(value: &Value) -> Result<(), ContextError> {
    let name = required(value, "hook_event_name")?;
    required(value, "session_id")?;
    field(value, "turn_id")?;
    match name.as_str() {
        "PreToolUse" => {
            required(value, "turn_id")?;
            // HooksV1 PreToolUse carries child identity as a pair or not at all.
            if field(value, "agent_id")?.is_some() != field(value, "agent_type")?.is_some() {
                return Err(ContextError::Invalid);
            }
            let command = value
                .get("tool_input")
                .and_then(|v| v.as_object())
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str())
                .ok_or(ContextError::Invalid)?;
            if command.len() > 65_536 || command.contains('\0') {
                return Err(ContextError::Invalid);
            }
        }
        "SubagentStart" => {
            // Complete HooksV1 SubagentStartCommandInput.
            for key in [
                "turn_id",
                "cwd",
                "model",
                "permission_mode",
                "agent_id",
                "agent_type",
            ] {
                required(value, key)?;
            }
            if !value
                .as_object()
                .is_some_and(|o| o.contains_key("transcript_path"))
            {
                return Err(ContextError::Invalid);
            }
            field(value, "transcript_path")?;
        }
        "SessionStart" => (),
        _ => return Err(ContextError::Invalid),
    }
    Ok(())
}

/// Return only native model context. No change produces no stdout; this never
/// returns a receipt decision, tool permission decision, or command rewrite.
pub fn encode_context(
    kind: EventKind,
    context: &str,
    changed: bool,
) -> Result<Vec<u8>, ContextError> {
    if !changed || context.is_empty() {
        return Ok(Vec::new());
    }
    if context.len() > 4096 || context.contains('\0') {
        return Err(ContextError::TooLarge);
    }
    let event = match kind {
        EventKind::Startup | EventKind::Resume | EventKind::Clear | EventKind::Compact => {
            "SessionStart"
        }
        EventKind::Tool => "PreToolUse",
        EventKind::Restart => return Err(ContextError::Invalid),
    };
    encode_native_context(event, context)
}

pub fn encode_event_context(
    event: &LifecycleEvent,
    context: &str,
    changed: bool,
) -> Result<Vec<u8>, ContextError> {
    if event.harness != Harness::Codex {
        return Err(ContextError::Invalid);
    }
    if !changed || context.is_empty() {
        return Ok(Vec::new());
    }
    if context.len() > 4096 || context.contains('\0') {
        return Err(ContextError::TooLarge);
    }
    let name = match event.source.as_str() {
        "SubagentStart" if event.role == Role::Subagent => "SubagentStart",
        "PreToolUse" if event.kind == EventKind::Tool => "PreToolUse",
        "startup" | "resume" | "clear" | "compact" => "SessionStart",
        _ => return Err(ContextError::Invalid),
    };
    encode_native_context(name, context)
}

fn encode_native_context(event: &str, context: &str) -> Result<Vec<u8>, ContextError> {
    let bytes = serde_json::to_vec(&json!({"hookSpecificOutput": {
        "hookEventName": event,
        "additionalContext": context,
    }}))
    .map_err(|_| ContextError::Invalid)?;
    if bytes.len() > 4608 {
        return Err(ContextError::TooLarge);
    }
    Ok(bytes)
}

/// Scoped command construction, reachable only through the gated
/// `ToolInvocation::scoped_command`. It is not a native hook decision.
fn scoped_bash_command(command: &str, tool_use_id: &str) -> Result<String, ContextError> {
    if command.is_empty()
        || command.len() > 65_536
        || command.contains('\0')
        || tool_use_id.is_empty()
        || tool_use_id.len() > 128
        || !tool_use_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ContextError::Invalid);
    }
    Ok(format!(
        "export HERDR_HOOK_INVOCATION='{tool_use_id}'\n{command}"
    ))
}

fn parse_shape(value: &Value, event_id: &str) -> Result<LifecycleEvent, ContextError> {
    let name = required(value, "hook_event_name")?;
    let role = declared_role(value)?;
    let (source, kind, role) = match name.as_str() {
        "SessionStart" => {
            let source = required(value, "source")?;
            let kind = match source.as_str() {
                "startup" => EventKind::Startup,
                "resume" => EventKind::Resume,
                "clear" => EventKind::Clear,
                "compact" => EventKind::Compact,
                // `fork` is valid HooksV1 input but known-unsupported: no
                // live fork payload has been captured, so whether it carries
                // a fresh session or the parent's history (startup-like or
                // resume-like check-in) is uncharacterized.
                _ => return Err(ContextError::Invalid),
            };
            (source, kind, role)
        }
        "SubagentStart" => (name, EventKind::Startup, Role::Subagent),
        "PreToolUse" if field(value, "tool_name")?.as_deref() == Some("Bash") => {
            required(value, "tool_use_id")?;
            (name, EventKind::Tool, role)
        }
        _ => return Err(ContextError::Invalid),
    };
    Ok(LifecycleEvent {
        harness: Harness::Codex,
        source,
        kind,
        native_session: field(value, "session_id")?,
        role,
        event_id: event_id.into(),
        capability: Capability::SourceSupported,
    })
}

pub(crate) struct CodexAdapter;

struct CodexCanary;
impl super::adapter::CanaryStrategy for CodexCanary {
    fn descriptor(&self) -> super::adapter::CanaryDescriptor {
        super::adapter::CanaryDescriptor {
            kind: super::adapter::CanaryKind::NpmRelease,
            candidate_kind: super::adapter::CandidateKind::StableRelease,
            npm_package: Some("@openai/codex".into()),
            model_key_env: Some("OPENAI_API_KEY".into()),
            companion: "scripts/canary/adapters/codex.py".into(),
            artifact_schema_version: 1,
        }
    }
}
impl HarnessAdapter for CodexAdapter {
    fn receipt_admission_summary(&self) -> Option<String> {
        Some(
            RECIPES
                .iter()
                .map(|recipe| recipe.versions.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
    fn observation_fingerprint(&self, env: &InstallEnvironment) -> Option<String> {
        super::adapter::executable_observation_fingerprint(env, "codex")
    }
    fn observe_daemon(
        &self,
        env: &InstallEnvironment,
        budget: &CallBudget,
    ) -> super::adapter::DaemonObservation {
        let (status, version) = observe_daemon_install(
            env.path.as_deref(),
            super::adapter::adapter_timeout(env, budget),
            &budget.cancellation,
        );
        super::adapter::DaemonObservation {
            status,
            identity: version
                .and_then(|v| RuntimeIdentity::stable_release(&v, "installed_probe").ok()),
            receipt_basis: Some(
                crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE.into(),
            ),
            ..Default::default()
        }
    }

    type Admission = InstalledVersion;
    fn metadata(&self) -> &'static AdapterMetadata {
        static METADATA: AdapterMetadata = AdapterMetadata {
            id: "codex",
            display_label: "Codex",
            context_spelling: "Codex",
            context_aliases: &[],
            executable: ExecutableLookup::Path("codex"),
            host_kinds: &["codex"],
            setup_scopes: &[SetupScopeKind::ConfigRoot],
            runtime_sources: &["installed_probe", "native_transcript"],
            budget: EventBudgetPolicy {
                lifecycle_ms: 5000,
                observer_ms: 1500,
            },
        };
        &METADATA
    }
    fn doctor_projection(
        &self,
        request: &StatusRequest,
        daemon: &serde_json::Value,
        budget: &CallBudget,
    ) -> Option<DoctorProjection> {
        Some(crate::cli::doctor::legacy_codex_projection(
            request, daemon, budget,
        ))
    }
    fn legacy_contract_id(&self) -> Option<String> {
        Some(super::contract::contract_id(&CONTRACT))
    }
    fn canary_strategy(&self) -> Option<&dyn super::adapter::CanaryStrategy> {
        Some(&CodexCanary)
    }
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        Some(self)
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        static CONTRACTS: [ContractDescriptor; 1] = [ContractDescriptor {
            domain: ContractDomain::Native,
            domain_id: "native_payload",
            origin: super::evidence::EvidenceOrigin::NativePayload,
            events: &[
                super::evidence::LEGACY_EVENTS[0],
                super::evidence::LEGACY_EVENTS[1],
                super::evidence::EvidenceEvent {
                    native_event: "SubagentStart",
                    milestone: None,
                    always_send: false,
                },
            ],
            required_milestones: &["lifecycle", "tool"],
            qualifications: &[],
            holding: super::evidence::AttributionHolding::SuppressResumed,
            resumed_unavailable_reason: Some("codex resume: rollout version is the creating CLI's"),
            contract: &CONTRACT,
        }];
        &CONTRACTS
    }
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget) -> InstallObservation {
        let Some(binary) = resolve_on_path(env.path.as_deref()) else {
            return InstallObservation::Unavailable {
                diagnostic: "installed codex executable not found on PATH".into(),
            };
        };
        let private = env
            .state_dir
            .as_deref()
            .and_then(|state| super::codex_evidence::prepare(state).ok());
        let cache = private.as_deref().map(super::codex_evidence::cache_path);
        let admission = InstalledAdmission::observe_binary(
            binary,
            super::adapter::adapter_timeout(env, budget),
            cache.as_deref().map_or(
                super::codex_schema::FingerprintCache::Memory,
                super::codex_schema::FingerprintCache::ReadWrite,
            ),
        );
        if let Some(private) = &private {
            let now = env.clock.utc_now().0.max(0) as u64;
            let _ = super::codex_evidence::record(
                &super::codex_evidence::admission_path(private),
                &admission,
                now,
            );
        }
        match admission.result {
            Ok(version) => InstallObservation::CodexWitness(version),
            Err(error) => InstallObservation::Unavailable {
                diagnostic: format!("installed codex version: {}", error.summary()),
            },
        }
    }
    fn admit(
        &self,
        request: &AdmissionRequest,
        _: &CallBudget,
    ) -> AdmissionDecision<Self::Admission> {
        let InstallObservation::CodexWitness(version) = &request.installed else {
            return AdmissionDecision::Refused {
                diagnostic: "installed codex witness unavailable".into(),
            };
        };
        match version.admission() {
            Admission::Listed => AdmissionDecision::Listed {
                state: version.clone(),
                recipe: version.recipe().id,
            },
            Admission::SchemaMatched { .. } => AdmissionDecision::SchemaMatched {
                state: version.clone(),
                recipe: version.recipe().id,
            },
            Admission::Optimistic { admission, .. } => AdmissionDecision::Optimistic {
                state: version.clone(),
                recipe: version.recipe().id,
                diagnostic: super::optimistic_label(admission, false),
            },
        }
    }
    fn version_ladder(&self, identity: &RuntimeIdentity) -> Ladder {
        identity.release().map_or(Ladder::Admitted, |version| {
            super::state::table_ladder(admission_table(), version)
        })
    }
    fn classify(&self, input: &HookInput) -> ContractObservation {
        ContractObservation {
            domain: ContractDomain::Native,
            classification: super::contract::classify(
                &CONTRACT,
                input.registered_event.as_deref(),
                &input.bytes,
            ),
        }
    }
    fn output_policy(&self) -> OutputPolicy {
        OutputPolicy {
            child_requires_endpoint: true,
            extra_guidance: crate::cli::skill::CODEX_COMMAND_GUIDANCE,
            empty_lifecycle: true,
            session_start_hint: true,
        }
    }
    fn decode(
        &self,
        admitted: &Self::Admission,
        input: &HookInput,
    ) -> Result<DecodedEvent, DecodeFailure> {
        parse_event_for_version(&input.bytes, &uuid::Uuid::new_v4().to_string(), admitted)
            .map(DecodedEvent::from_native)
            .map_err(DecodeFailure::Native)
    }
    fn encode(
        &self,
        _: &Self::Admission,
        event: &DecodedEvent,
        offer: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        super::adapter::encode_context(event, offer)
    }
    fn attribute_runtime(&self, input: &HookInput, _: &CallBudget) -> RuntimeAttribution {
        super::attribution::attribute_native_runtime("codex", input)
    }
    fn setup(&self, request: &SetupRequest, _: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
        setup::setup(request)
    }
    fn status(&self, request: &StatusRequest, _: &CallBudget) -> SetupStatus {
        setup::status(request)
    }
    fn unsetup(
        &self,
        request: &UnsetupRequest,
        _: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure> {
        setup::unsetup(request)
    }
}

fn observe_daemon_install(
    path: Option<&std::ffi::OsStr>,
    timeout: std::time::Duration,
    cancel: &crate::protocol::time::Cancellation,
) -> (super::adapter::HarnessStatus, Option<String>) {
    use crate::harness::codex::{InstalledRefusal, VersionError};
    let admission = crate::harness::codex::InstalledAdmission::observe_on_path_cancellable(
        path, timeout, cancel,
    );
    let version = match &admission.result {
        Ok(version) => Some(version.as_str()),
        Err(InstalledRefusal::Refused(
            VersionError::Unsupported(version) | VersionError::KnownBroken { version, .. },
        )) => Some(version.as_str()),
        Err(_) => None,
    };
    let version = version.and_then(|raw| crate::harness::contract::normalize_version("codex", raw));
    (
        crate::app::codex_status(
            &admission,
            crate::harness::codex::DECLARATION.health_capability(),
        ),
        version,
    )
}

/// Codex native launch grammar, environment and wrapper policy.
pub mod launch {
    use crate::cli::setup::SetupEnv;
    use crate::protocol::results::{ApiError, ErrorCode};
    use crate::protocol::time::CallBudget;
    use serde_json::{Value, json};
    use std::{
        fs,
        io::Read,
        process::{Command as Process, Stdio},
        time::{Duration, Instant},
    };
    fn error(code: ErrorCode, detail: &str) -> ApiError {
        ApiError::new(code, detail)
    }
    /// Codex options (root, `exec` and `resume` levels) that consume the next
    /// argument as their value. Any other `-`/`--` option is taken as a switch;
    /// `--flag=value` spellings never consume the next argument. `-i/--image`
    /// takes one or more values, so its separated spelling is refused
    /// ([`CODEX_MULTI_VALUE_OPTIONS`]) rather than guessing its arity.
    pub(crate) const CODEX_VALUE_OPTIONS: &[&str] = &[
        "-c",
        "--config",
        "--enable",
        "--disable",
        "-i",
        "--image",
        "--remote",
        "--remote-auth-token-env",
        "--thread-source",
        "-m",
        "--model",
        "--local-provider",
        "-p",
        "--profile",
        "-s",
        "--sandbox",
        "-a",
        "--ask-for-approval",
        "-C",
        "--cd",
        "--add-dir",
        "--output-schema",
        "--color",
        "-o",
        "--output-last-message",
    ];

    /// Codex options taking a variable number of values (codex-cli 0.159.2
    /// `-i, --image <FILE>...`): the separated spelling would swallow a
    /// following subcommand or prompt, so only `--image=FILE` (or the option
    /// after `--`) is accepted.
    const CODEX_MULTI_VALUE_OPTIONS: &[&str] = &["-i", "--image"];

    /// Codex subcommands a managed launch cannot configure: refused rather than
    /// started without the owned hooks. `exec` is a handled form; `resume` is refused until captured.
    /// Covers every top-level subcommand and alias `codex --help` lists for
    /// codex-cli 0.159.2 (plus older names), so a bare first positional naming a
    /// Codex subcommand is never mistaken for an interactive prompt; a prompt
    /// that is such a word goes after `--`.
    pub(crate) const CODEX_UNSUPPORTED_SUBCOMMANDS: &[&str] = &[
        "agents",
        "e",
        "review",
        "login",
        "logout",
        "mcp",
        "mcp-server",
        "app-server",
        "app",
        "completion",
        "sandbox",
        "debug",
        "apply",
        "a",
        "fork",
        "cloud",
        "cloud-tasks",
        "features",
        "help",
        "plugin",
        "remote-control",
        "update",
        "doctor",
        "queue",
        "archive",
        "delete",
        "migrate-rollouts",
        "unarchive",
        "exec-server",
        "responses-api-proxy",
        "stdio-to-uds",
        "execpolicy",
        "generate-ts",
    ];

    /// `codex exec` subcommands other than `resume` (codex-cli 0.159.2
    /// `codex exec --help`): refused, since no evidence shows they read the
    /// exec-level owned hooks.
    pub(crate) const CODEX_EXEC_UNSUPPORTED_SUBCOMMANDS: &[&str] = &["fork", "review", "help"];

    /// Where a managed Codex launch places the owned `-c` configuration. Codex
    /// 0.159.2 `exec` ignores root-level `hooks.*` overrides
    /// (`native-codex-matrix-1/hook-placement-probe`), so each subcommand form
    /// carries them at its own level; `--no-daemon` is a top-level flag and
    /// always precedes the subcommand.
    ///
    /// Evidence per form:
    /// - `Interactive`: `codex --no-daemon -c hooks.* [PROMPT]`, the launch line
    ///   `setup codex` prints (root-level session overrides);
    /// - `Exec`: `codex --no-daemon exec -c hooks.* ... PROMPT`
    ///   (`hook-placement-probe/exec.jsonl`, `codex-158-live-hook-capture/run1.sh`);
    /// - `ExecResume`: `codex --no-daemon exec ... resume ... -c hooks.* ID PROMPT`
    ///   (`codex-158-live-hook-capture/run3.sh`, SessionStart resume captured);
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CodexLaunchForm {
        Interactive,
        Exec,
        ExecResume,
    }

    /// The explicit working directory whose project configuration a scoped
    /// sandbox probe can reproduce. An implicit pane cwd is not inferred from
    /// the coordinator process or saved pane paths.
    pub fn scoped_codex_cwd(argv: &[String]) -> Result<std::path::PathBuf, ApiError> {
        let options_end = argv
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(argv.len());
        let mut found = None;
        let mut index = 0;
        while index < options_end {
            let arg = argv[index].as_str();
            let cwd = if arg == "-C" || arg == "--cd" {
                argv.get(index + 1).map(String::as_str)
            } else {
                arg.strip_prefix("--cd=")
                    .or_else(|| arg.strip_prefix("-C="))
            };
            if let Some(cwd) = cwd {
                let path = std::path::PathBuf::from(cwd);
                if found.is_some() || !path.is_absolute() {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "scoped Codex sandbox validation needs one absolute -C directory",
                    ));
                }
                found = Some(path);
                index += if arg == "-C" || arg == "--cd" { 2 } else { 1 };
            } else {
                index += if CODEX_VALUE_OPTIONS.contains(&arg) {
                    2
                } else {
                    1
                };
            }
        }
        found.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "scoped Codex sandbox validation needs an explicit -C /absolute/project/path to match project policy",
        )
    })
    }

    /// The next positional argument at or after `start`: its index and whether
    /// it follows `--` (and so is a prompt, never a subcommand).
    fn next_positional(argv: &[String], start: usize) -> Option<(usize, bool)> {
        let mut index = start;
        while index < argv.len() {
            let arg = argv[index].as_str();
            if arg == "--" {
                return (index + 1 < argv.len()).then_some((index + 1, true));
            }
            if arg.len() > 1 && arg.starts_with('-') {
                index += if CODEX_VALUE_OPTIONS.contains(&arg) {
                    2
                } else {
                    1
                };
                continue;
            }
            return Some((index, false));
        }
        None
    }

    /// The caller's Codex form and the index at which the owned configuration is
    /// inserted (right after the subcommand that must carry it).
    fn codex_form(argv: &[String]) -> Result<(CodexLaunchForm, usize, Option<usize>), ApiError> {
        let Some((first, after_separator)) = next_positional(argv, 0) else {
            return Ok((CodexLaunchForm::Interactive, 0, None));
        };
        if after_separator {
            return Ok((CodexLaunchForm::Interactive, 0, None));
        }
        match argv[first].as_str() {
            "exec" => match next_positional(argv, first + 1) {
                Some((second, false)) if argv[second] == "resume" => {
                    Ok((CodexLaunchForm::ExecResume, second + 1, Some(first)))
                }
                // Only `exec resume` has evidence of reading its own hook level;
                // every other exec subcommand (0.159.2: `fork`, `review`, `help`)
                // is refused rather than started with unverified hook placement.
                Some((second, false))
                    if CODEX_EXEC_UNSUPPORTED_SUBCOMMANDS.contains(&argv[second].as_str()) =>
                {
                    Err(error(
                        ErrorCode::InvalidRequest,
                        "managed launch supports Codex `exec` and `exec resume` only; this exec \
                     subcommand cannot carry the owned hook configuration",
                    ))
                }
                _ => Ok((CodexLaunchForm::Exec, first + 1, Some(first))),
            },
            "resume" => Err(error(
                ErrorCode::InvalidRequest,
                "managed launch refuses the Codex `resume` form: no live capture shows it loading the owned hooks (TRUST-POLICY Accepted limits); run `codex resume` by hand in the pane, or use `exec resume`",
            )),
            word if CODEX_UNSUPPORTED_SUBCOMMANDS.contains(&word) => Err(error(
                ErrorCode::InvalidRequest,
                "managed launch supports Codex interactive, `exec` and `exec resume` only; \
             this subcommand cannot carry the owned hook configuration",
            )),
            _ => {
                // Interactive with a prompt: Codex takes one prompt, so a second
                // positional means the arguments were misread (an unknown option
                // taking a value before a subcommand). Refuse rather than place
                // the owned hooks where the subcommand would ignore them.
                if next_positional(argv, first + 1).is_some() {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "ambiguous Codex arguments: more than one positional argument before a \
                     recognised subcommand; put the prompt after `--`",
                    ));
                }
                Ok((CodexLaunchForm::Interactive, 0, None))
            }
        }
    }

    /// The native argument array a managed launch submits: for Claude the owned
    /// arguments then the caller's; for Codex `--no-daemon` exactly once at the
    /// top level, then the caller's arguments byte for byte and in order with the
    /// owned configuration inserted at the level of the caller's subcommand
    /// ([`CodexLaunchForm`]). An owned `--no-daemon` is dropped, never
    /// duplicated. Unsupported subcommands, conflicting daemon modes, a
    /// misplaced `--no-daemon` and caller `hooks.*` overrides are refused.
    /// [`compose_native_argv`](crate::harness::launch::compose_native_argv) for a pane whose shell wrapper may already pass
    /// `--no-daemon`. With `shell_passes_no_daemon` the composed Codex argv
    /// carries no `--no-daemon` at all (the wrapper supplies the single one), so
    /// a caller's own top-level `--no-daemon` is dropped too; every other check
    /// and placement is unchanged.
    pub fn compose_native_argv_with(
        caller: Vec<String>,
        owned: Vec<String>,
        shell_passes_no_daemon: bool,
    ) -> Result<Vec<String>, ApiError> {
        let options_end = caller
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(caller.len());
        let options = &caller[..options_end];
        if options.iter().any(|arg| {
            arg == "--daemon" || arg.starts_with("--daemon=") || arg.starts_with("--no-daemon=")
        }) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "conflicting Codex daemon mode",
            ));
        }
        // Codex applies repeated `-c` values in order within the session layer,
        // so a caller hook override (a `hooks.*` key or the whole `hooks` table)
        // would silently replace the owned hook.
        let scoped_socket_policy = owned
            .iter()
            .any(|arg| arg == "sandbox_workspace_write.network_access=true");
        let mut previous_is_config = false;
        let mut previous_takes_value = false;
        for arg in options {
            let value = if previous_is_config {
                Some(arg.as_str())
            } else {
                arg.strip_prefix("--config=").or_else(|| {
                    arg.strip_prefix("-c")
                        .filter(|rest| !rest.is_empty())
                        .map(|rest| rest.strip_prefix('=').unwrap_or(rest))
                })
            };
            if value.is_some_and(overrides_hooks) {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "caller Codex hooks override would replace the owned hook configuration",
                ));
            }
            if scoped_socket_policy
                && (value.is_some()
                    || (!previous_takes_value
                        && matches!(
                            arg.as_str(),
                            "-c" | "--config"
                                | "-p"
                                | "--profile"
                                | "-s"
                                | "--sandbox"
                                | "--enable"
                                | "--disable"
                                | "--add-dir"
                                | "--remote"
                                | "--remote-auth-token-env"
                                | "--worktree"
                                | "--approve-for-me"
                                | "--dangerously-bypass-approvals-and-sandbox"
                                | "--yolo"
                                | "--dangerously-bypass-hook-trust"
                                | "--full-auto"
                                | "--search"
                        ))
                    || (!previous_takes_value
                        && [
                            "--profile=",
                            "--sandbox=",
                            "--enable=",
                            "--disable=",
                            "--add-dir=",
                            "--remote=",
                            "--remote-auth-token-env=",
                        ]
                        .iter()
                        .any(|prefix| arg.starts_with(prefix)))
                    || (!previous_takes_value && (arg.starts_with("-s") || arg.starts_with("-p"))))
            {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "caller Codex policy or profile override would differ from the measured scoped sandbox policy",
                ));
            }
            if !previous_takes_value && CODEX_MULTI_VALUE_OPTIONS.contains(&arg.as_str()) {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "ambiguous Codex arguments: put images as --image=FILE or before --",
                ));
            }
            previous_is_config = !previous_takes_value && (arg == "-c" || arg == "--config");
            previous_takes_value =
                !previous_takes_value && CODEX_VALUE_OPTIONS.contains(&arg.as_str());
        }
        let (_, insert_at, subcommand) = codex_form(&caller)?;
        let no_daemon: Vec<usize> = options
            .iter()
            .enumerate()
            .filter(|(_, arg)| *arg == "--no-daemon")
            .map(|(index, _)| index)
            .collect();
        if no_daemon.len() > 1 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "duplicate Codex --no-daemon",
            ));
        }
        if let (Some(&at), Some(subcommand)) = (no_daemon.first(), subcommand)
            && at > subcommand
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "Codex --no-daemon is a top-level flag; it must precede the subcommand",
            ));
        }
        let owned = owned.into_iter().filter(|arg| arg != "--no-daemon");
        let mut argv = Vec::with_capacity(caller.len() + 8);
        let (caller, insert_at) = match (shell_passes_no_daemon, no_daemon.first()) {
            (true, Some(&at)) => {
                // The wrapper's flag is the single one; removing a top-level
                // switch never changes the subcommand form.
                let mut caller = caller;
                caller.remove(at);
                (
                    caller,
                    if at < insert_at {
                        insert_at - 1
                    } else {
                        insert_at
                    },
                )
            }
            (true, None) => (caller, insert_at),
            (false, _) => {
                if no_daemon.is_empty() {
                    argv.push("--no-daemon".to_owned());
                }
                (caller, insert_at)
            }
        };
        let mut caller = caller.into_iter();
        argv.extend(caller.by_ref().take(insert_at));
        argv.extend(owned);
        argv.extend(caller);
        Ok(argv)
    }

    /// Whether a Codex `-c` value sets the `hooks` table or a key under it: the
    /// key is the text before the first `=`, trimmed.
    fn overrides_hooks(value: &str) -> bool {
        let key = value.split('=').next().unwrap_or("").trim();
        key == "hooks" || key.starts_with("hooks.")
    }

    /// How the pane's interactive shell resolves `codex`. Herdr starts the
    /// agent by name inside that shell, so a user function or alias wrapping
    /// `codex` runs first and may already pass `--no-daemon` (Codex refuses the
    /// flag twice). Injected so tests never run a real shell.
    pub trait CodexShellProbe {
        /// The shell's description of `codex` (stdout only), or why it could
        /// not be obtained.
        fn resolve_codex(&self) -> Result<String, String>;
        fn resolve_codex_bounded(
            &self,
            clock: &dyn crate::protocol::time::Clock,
            budget: &CallBudget,
        ) -> Result<String, String> {
            if budget.is_exhausted(clock) {
                return Err("launch budget exhausted".into());
            }
            self.resolve_codex()
        }
        fn pane_shell_env_bounded(
            &self,
            var: &str,
            clock: &dyn crate::protocol::time::Clock,
            budget: &CallBudget,
        ) -> Option<String> {
            if budget.is_exhausted(clock) {
                return None;
            }
            self.pane_shell_env(var)
        }

        /// The value the pane's interactive shell itself gives `var` (an `export`
        /// in its startup files), without the launcher's own value; `None` when
        /// the shell sets none or cannot be asked. Herdr's `agent.start` carries
        /// no environment, so the agent inherits whatever the pane shell has.
        fn pane_shell_env(&self, _var: &str) -> Option<String> {
            None
        }
    }

    /// The bound on the shell probe; on timeout launch keeps adding `--no-daemon`.
    pub const SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

    /// Runs the user's `$SHELL` (else `/bin/zsh`) interactively, as the pane
    /// does: zsh `whence -f codex 2>/dev/null || type codex`, otherwise
    /// `type codex`. Stdout only; stdin and stderr are null.
    pub struct SystemShellProbe {
        pub shell: std::path::PathBuf,
        pub timeout: Duration,
    }

    impl SystemShellProbe {
        pub fn from_process() -> Self {
            let shell = std::env::var_os("SHELL")
                .filter(|shell| !shell.is_empty())
                .map_or_else(|| "/bin/zsh".into(), std::path::PathBuf::from);
            Self {
                shell,
                timeout: crate::protocol::time::external_bound(SHELL_PROBE_TIMEOUT),
            }
        }
    }

    impl SystemShellProbe {
        /// Runs `script` in the interactive shell (`-ic`), stdout only, bounded by
        /// the probe timeout. `unset` removes one inherited variable first.
        fn run_script(&self, script: &str, unset: Option<&str>) -> Result<String, String> {
            let mut command = Process::new(&self.shell);
            command.arg("-ic").arg(script);
            if let Some(var) = unset {
                command.env_remove(var);
            }
            let mut child = command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| format!("{}: {error}", self.shell.display()))?;
            let mut stdout = child.stdout.take().ok_or("no shell stdout")?;
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = stdout.read_to_end(&mut bytes).map(|_| bytes);
                let _ = sender.send(result);
            });
            let deadline = Instant::now() + self.timeout;
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Ok(None) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("the shell probe timed out".into());
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            if !status.success() {
                return Err(format!("the shell probe exited with {status}"));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let bytes = receiver
                .recv_timeout(remaining.max(Duration::from_millis(100)))
                .map_err(|_| "the shell probe output was not closed".to_owned())?
                .map_err(|error| error.to_string())?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
    }

    impl CodexShellProbe for SystemShellProbe {
        fn resolve_codex_bounded(
            &self,
            clock: &dyn crate::protocol::time::Clock,
            budget: &CallBudget,
        ) -> Result<String, String> {
            let remaining = budget.deadline.0.saturating_sub(clock.monotonic_now().0);
            if remaining == 0 {
                return Err("launch budget exhausted".into());
            }
            Self {
                shell: self.shell.clone(),
                timeout: self.timeout.min(Duration::from_millis(remaining)),
            }
            .resolve_codex()
        }
        fn pane_shell_env_bounded(
            &self,
            var: &str,
            clock: &dyn crate::protocol::time::Clock,
            budget: &CallBudget,
        ) -> Option<String> {
            let remaining = budget.deadline.0.saturating_sub(clock.monotonic_now().0);
            if remaining == 0 {
                return None;
            }
            Self {
                shell: self.shell.clone(),
                timeout: self.timeout.min(Duration::from_millis(remaining)),
            }
            .pane_shell_env(var)
        }

        fn resolve_codex(&self) -> Result<String, String> {
            let is_zsh = self
                .shell
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("zsh"));
            let script = if is_zsh {
                "whence -f codex 2>/dev/null || type codex"
            } else {
                "type codex"
            };
            self.run_script(script, None)
        }

        fn pane_shell_env(&self, var: &str) -> Option<String> {
            const BEGIN: &str = "HT_PANE_ENV_BEGIN";
            const END: &str = "HT_PANE_ENV_END";
            // Only a plain variable name is ever interpolated into the script.
            if var.is_empty() || !var.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                return None;
            }
            let script = format!("printf '%s' {BEGIN}\"${{{var}-}}\"{END}");
            let output = self.run_script(&script, Some(var)).ok()?;
            let value = output.split(BEGIN).nth(1)?.split(END).next()?;
            (!value.is_empty()).then(|| value.to_owned())
        }
    }

    /// Whether a shell's description of `codex` (a function body or alias)
    /// passes `--no-daemon` as a word of its own. Comment lines are ignored;
    /// `--no-daemon=...` is not the flag.
    pub fn wrapper_passes_no_daemon(description: &str) -> bool {
        description
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(|line| {
                line.split(|c: char| {
                    c.is_whitespace() || matches!(c, '\'' | '"' | '`' | ';' | '(' | ')' | '|' | '&')
                })
            })
            .any(|word| word == "--no-daemon")
    }

    /// The report text when the pane shell's `codex` wrapper already passes
    /// `--no-daemon` and launch therefore adds none.
    pub const CODEX_WRAPPER_NO_DAEMON: &str =
        "shell function or alias already passes --no-daemon; launch added none";

    /// The Codex profile Codex applies: `-p/--profile` before `--` (the last one
    /// wins), else the top-level `profile` key of `config.toml`, else none.
    pub fn codex_profile(argv: &[String], config: Option<&str>) -> (String, &'static str) {
        let options_end = argv.iter().position(|a| a == "--").unwrap_or(argv.len());
        let options = &argv[..options_end];
        let mut chosen = None;
        for (index, arg) in options.iter().enumerate() {
            let value = match arg.as_str() {
                "-p" | "--profile" => options.get(index + 1).map(String::as_str),
                other => other
                    .strip_prefix("--profile=")
                    .or_else(|| other.strip_prefix("-p").filter(|rest| !rest.is_empty())),
            };
            if let Some(value) = value {
                chosen = Some(value.to_owned());
            }
        }
        if let Some(profile) = chosen {
            return (profile, "argv");
        }
        let from_config = config
            .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
            .and_then(|doc| doc.get("profile")?.as_str().map(str::to_owned));
        match from_config {
            Some(profile) => (profile, "config.toml"),
            None => ("default".to_owned(), "none"),
        }
    }

    /// The effective Codex home, its `config.toml` and the selected profile,
    /// which answers "why was my Codex profile not applied?".
    pub(crate) fn codex_report(env: &SetupEnv, argv: &[String]) -> Value {
        let home = env.codex_home.as_deref();
        let config_path = home.map(|home| home.join("config.toml"));
        let config = config_path
            .as_deref()
            .and_then(|path| fs::read_to_string(path).ok());
        let (profile, profile_source) = codex_profile(argv, config.as_deref());
        json!({
            "codex_home": home.map(|home| home.display().to_string()),
            "config_path": config_path.as_ref().map(|path| path.display().to_string()),
            "config_present": config.is_some(),
            "profile": profile,
            "profile_source": profile_source,
            "command_execution": "approved_outside_sandbox",
            "command_guidance": "Run herdr-threads commands outside the sandbox through Codex approval; if denied, report the policy refusal without bypassing it",
        })
    }
}

impl LaunchPolicy for CodexAdapter {
    fn resolve_scope(
        &self,
        request: &LaunchRequest,
        probe: &dyn super::launch::CodexShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchScope, crate::protocol::results::ApiError> {
        let mut scope = super::launch::native_scope(request, "codex", "CODEX_HOME", probe, budget)?;
        // -C remains in native argv. It changes project lookup, never setup profile selection.
        let options_end = request
            .argv
            .iter()
            .position(|a| a == "--")
            .unwrap_or(request.argv.len());
        let mut index = 0;
        while index < options_end {
            let arg = &request.argv[index];
            let value = if arg == "-C" || arg == "--cd" {
                request.argv.get(index + 1).map(String::as_str)
            } else {
                arg.strip_prefix("--cd=")
                    .or_else(|| arg.strip_prefix("-C="))
            };
            if let Some(value) = value {
                scope.working_directory = request.environment.cwd.join(value);
            }
            index += if launch::CODEX_VALUE_OPTIONS.contains(&arg.as_str()) {
                2
            } else {
                1
            };
        }
        Ok(scope)
    }
    fn validate_native_argv(
        &self,
        argv: &[String],
    ) -> Result<(), crate::protocol::results::ApiError> {
        launch::compose_native_argv_with(argv.to_vec(), Vec::new(), false).map(|_| ())
    }
    fn compose_argv(
        &self,
        caller: Vec<String>,
        owned: Vec<String>,
        shell: bool,
    ) -> Result<Vec<String>, crate::protocol::results::ApiError> {
        launch::compose_native_argv_with(caller, owned, shell)
    }
    fn prepare_launch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        admitted: &super::registry::AdmittedHandle,
        status: &LocalSetupStatus,
        probe: &dyn super::launch::CodexShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchPreparation, crate::protocol::results::ApiError> {
        if admitted.metadata().id != "codex" || status.scope != scope.setup {
            return Err(crate::protocol::results::ApiError::new(
                crate::protocol::results::ErrorCode::InvalidRequest,
                "launch admission or scope mismatch",
            ));
        }
        let hook = super::launch::owned_launch_hook(status)?;
        if hook != super::launch::native_configuration_hook(request, scope, Harness::Codex)? {
            return Err(crate::protocol::results::ApiError::new(
                crate::protocol::results::ErrorCode::Conflict,
                "selected native setup status changed before preparation",
            ));
        }
        let shell = probe
            .resolve_codex_bounded(request.environment.clock.as_ref(), budget)
            .is_ok_and(|text| launch::wrapper_passes_no_daemon(&text));
        let env = crate::harness::setup::legacy::scoped_legacy_environment(
            Harness::Codex,
            &scope.setup,
            &request.environment,
        )
        .map_err(|err| {
            crate::protocol::results::ApiError::new(
                crate::protocol::results::ErrorCode::InvalidRequest,
                err.to_string(),
            )
        })?;
        Ok(LaunchPreparation {
            argv: self.compose_argv(request.argv.clone(), Vec::new(), shell)?,
            hook,
            working_directory: scope.working_directory.clone(),
            environment_overrides: Default::default(),
            report: json!({"codex": launch::codex_report(&env, &request.argv)}),
            wrapper_warning: shell.then_some(launch::CODEX_WRAPPER_NO_DAEMON),
        })
    }
    fn configuration_fingerprint(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
    ) -> Result<String, crate::protocol::results::ApiError> {
        super::launch::native_configuration_fingerprint(
            request,
            scope,
            Harness::Codex,
            &["hooks.json", "config.toml"],
        )
    }
    fn expected_host_kinds(&self) -> &'static [&'static str] {
        self.metadata().host_kinds
    }
}
