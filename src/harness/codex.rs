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
use super::codex_schema::{self, Unextractable};
use super::context::{ContextError, EventKind, Harness, Role};
pub use super::recipe::NativeSupport;
use super::recipe::{self, LookupError, Recipe, Version, VersionSet};
use super::{Capability, LifecycleEvent, declared_role, field, input};
use crate::protocol::results::CapabilityState;
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
    versions: VersionSet::Exact(&[Version::new(0, 157, 1), Version::new(0, 158, 0)]),
    evidence: &[
        "docs/compatibility/codex-probe.md",
        "docs/evidence/codex-158-hook-capture/report.md",
        "docs/evidence/codex-158-live-hook-capture/report.md",
    ],
    scope: "0.157.1: native root/child PreToolUse input captured and source-read hook contract; \
            0.158.0: embedded hook input/output schemas byte-identical to 0.157.1, live \
            SessionStart startup/resume, SubagentStart and root/child Bash PreToolUse input \
            captured, and context-only additionalContext delivery to the model observed for \
            SessionStart and PreToolUse (not SubagentStart). permission_mode is always \
            bypassPermissions under exec and is not a sandbox signal. SessionStart fork is \
            known-unsupported (never captured). Transport and receipt are not qualified for \
            either",
    profile: CodexProfile {
        input_schema: InputSchema::HooksV1,
        schema_fingerprint: HOOKS_V1_SCHEMA_FINGERPRINT,
        input_mapping: Capability::ObservedInput,
        invocation_transport: NativeSupport::Unsupported,
        model_receipt: NativeSupport::Unsupported,
    },
}];

/// The recipe covering an installed version string, if any.
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
    /// A well-formed version no recipe lists whose binary's embedded hook
    /// schemas were fingerprinted and match no recipe's captured schemas.
    SchemaUnmatched {
        version: String,
        fingerprint: String,
    },
    /// A well-formed version no recipe lists whose binary's embedded hook
    /// schemas could not be fingerprinted within the observation deadline.
    SchemaUnextractable {
        version: String,
        reason: Unextractable,
    },
}

const UNLISTED_REMEDY: &str = "Install a supported version, or capture this version's hook \
                               payloads and add a recipe backed by that evidence";

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
            Self::SchemaUnmatched {
                version,
                fingerprint,
            } => write!(
                f,
                "codex {version} has no adapter recipe and its embedded hook schemas \
                 ({fingerprint}) match no recipe's captured schemas; supported recipes: {}. \
                 {UNLISTED_REMEDY}",
                recipe::describe(RECIPES)
            ),
            Self::SchemaUnextractable { version, reason } => write!(
                f,
                "codex {version} has no adapter recipe and its embedded hook schemas could \
                 not be fingerprinted ({reason}); supported recipes: {}. {UNLISTED_REMEDY}",
                recipe::describe(RECIPES)
            ),
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
}

/// The fixed evidence label for a schema-matched admission.
pub const SCHEMA_MATCHED_LABEL: &str = "schema-matched, live-unverified";

pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_OUTPUT_LIMIT: u64 = 256;

/// The version line parsed, with its listed recipe if one lists it.
enum Reported {
    Listed(String, &'static CodexRecipe),
    Unlisted(String),
}

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
        let deadline = Instant::now() + timeout.min(VERSION_TIMEOUT);
        // Taken before the run, so an unlisted version is only admitted when
        // the binary fingerprinted is the binary that reported it.
        let ran = codex_schema::BinaryIdentity::observe(binary);
        let stdout = version_output_by(binary, deadline)?;
        match Self::reported(&stdout)? {
            Reported::Listed(version, recipe) => Ok(Self {
                version,
                recipe,
                admission: Admission::Listed,
            }),
            Reported::Unlisted(version) => {
                Self::admit_by_schema(binary, ran, version, deadline, cache)
            }
        }
    }

    fn admit_by_schema(
        binary: &Path,
        ran: Option<codex_schema::BinaryIdentity>,
        version: String,
        deadline: Instant,
        cache: codex_schema::FingerprintCache<'_>,
    ) -> Result<Self, VersionError> {
        let measured =
            codex_schema::fingerprint_binary_with(binary, deadline, cache).map_err(|reason| {
                VersionError::SchemaUnextractable {
                    version: version.clone(),
                    reason,
                }
            })?;
        if ran.as_ref() != Some(&measured.identity) {
            return Err(VersionError::SchemaUnextractable {
                version,
                reason: Unextractable::Changed,
            });
        }
        match RECIPES
            .iter()
            .find(|recipe| recipe.profile.schema_fingerprint == measured.fingerprint)
        {
            Some(recipe) => Ok(Self {
                version,
                recipe,
                admission: Admission::SchemaMatched {
                    fingerprint: measured.fingerprint,
                    binary_sha256: measured.binary_sha256,
                },
            }),
            None => Err(VersionError::SchemaUnmatched {
                version,
                fingerprint: measured.fingerprint,
            }),
        }
    }

    fn reported(stdout: &[u8]) -> Result<Reported, VersionError> {
        if stdout.len() as u64 > VERSION_OUTPUT_LIMIT {
            return Err(VersionError::Unrecognized);
        }
        let text = std::str::from_utf8(stdout).map_err(|_| VersionError::Unrecognized)?;
        let line = text.strip_suffix('\n').unwrap_or(text);
        let version = line
            .strip_prefix("codex-cli ")
            .ok_or(VersionError::Unrecognized)?;
        match recipe_for(version) {
            Ok(recipe) => Ok(Reported::Listed(version.to_owned(), recipe)),
            Err(LookupError::Unrecognized) => Err(VersionError::Unrecognized),
            Err(LookupError::Unsupported(_)) => Ok(Reported::Unlisted(version.to_owned())),
        }
    }

    /// Listed-recipe parse of `--version` output with no binary to
    /// fingerprint: an unlisted version is `Unsupported`.
    #[cfg(test)]
    fn from_output(stdout: &[u8]) -> Result<Self, VersionError> {
        match Self::reported(stdout)? {
            Reported::Listed(version, recipe) => Ok(Self {
                version,
                recipe,
                admission: Admission::Listed,
            }),
            Reported::Unlisted(version) => Err(VersionError::Unsupported(version)),
        }
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
    /// `codex 0.158.0: listed recipe codex-hooks-v1` or
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
    version_output_by(binary, Instant::now() + timeout)
}

/// Run `<absolute binary> --version` and return its stdout before
/// `deadline`. A process that exits but leaves a descendant holding stdout is
/// `Unavailable`; its process group is killed.
fn version_output_by(binary: &Path, deadline: Instant) -> Result<Vec<u8>, VersionError> {
    bounded_output_by(binary, &["--version"], VERSION_OUTPUT_LIMIT, deadline)
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
    bounded_output_by(binary, args, limit, Instant::now() + timeout)
}

fn bounded_output_by(
    binary: &Path,
    args: &[&str],
    limit: u64,
    deadline: Instant,
) -> Result<Vec<u8>, VersionError> {
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
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
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
            Self::SchemaUnmatched {
                version,
                fingerprint,
            } => format!(
                "codex {version}: refused: embedded hook schemas {fingerprint} match no recipe"
            ),
            Self::SchemaUnextractable { version, reason } => format!(
                "codex {version}: refused: embedded hook schemas not fingerprinted ({reason})"
            ),
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
        let Some(binary) = resolve_on_path(path) else {
            return Self {
                binary: None,
                result: Err(InstalledRefusal::NotFound),
            };
        };
        Self::observe_binary(binary, timeout, codex_schema::FingerprintCache::Memory)
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

    /// `listed`, `schema-matched, live-unverified`, `refused` or `not_found`.
    pub fn state(&self) -> &'static str {
        match &self.result {
            Ok(version) => match version.admission() {
                Admission::Listed => "listed",
                Admission::SchemaMatched { .. } => SCHEMA_MATCHED_LABEL,
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
