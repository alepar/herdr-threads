//! Backend-neutral owned permission contracts. No native grants are written here.
//!
//! Inventory fingerprints must come from the consumer's explicit installer ownership
//! record or its verified current executable, never discovery of another PATH entry.
//! Backend observations attest exact native rule/file bytes; this module does not
//! render policies or infer matching scope. Consent is settled before acquiring a
//! guard, then backend writers refresh plans under that guard before publication.
pub mod claude;
pub mod codex;

use crate::cli::commands::{
    OrdinaryCatalog, OutputPosition, PermissionCliInputs, RoutingToken, ordinary_catalog,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_PATH_BYTES: usize = 4096;
// The full finite Claude policy must fit while publication remains bounded.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RESOURCES: usize = 32768;
/// Shared finite Claude native settings ceiling, including hook and prompt writers.
pub const CLAUDE_SETTINGS_MAX_BYTES: usize = 16 * 1024 * 1024;
/// Legacy hook manifests encode two settings baselines as JSON byte arrays.
/// Bounded separately from the independent permission manifest (16 MiB).
pub const CLAUDE_HOOK_MANIFEST_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const PERMISSION_MANIFEST_VERSION: u32 = 1;

/// Versioned cooperative grammar attestation, independent of versions/help or integration support.
/// Unknown fields/versions and partial attestations cannot preserve executable-wide grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallerPermissionBoundary {
    version: u32,
    immediate_human: bool,
    root_person_operator: String,
    inferred_human: String,
    retry_actor: String,
}
impl InstallerPermissionBoundary {
    pub fn current() -> Self {
        Self {
            version: 1,
            immediate_human: true,
            root_person_operator: "refused".into(),
            inferred_human: "refused".into(),
            retry_actor: "immutable_original".into(),
        }
    }
    pub fn validate(bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > 4096 {
            return Err(refusal(
                "incoming permission boundary contract exceeds bound",
            ));
        }
        let contract: Self = serde_json::from_slice(bytes).map_err(|_| {
            refusal("incoming permission boundary contract is malformed or unsupported")
        })?;
        if contract != Self::current() {
            return Err(refusal(
                "incoming permission boundary contract is incompatible",
            ));
        }
        Ok(())
    }
}

fn refusal(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail)
}
fn safe_absolute(path: &Path) -> io::Result<()> {
    let Some(text) = path.to_str() else {
        return Err(refusal("non-UTF8 permission path"));
    };
    if !path.is_absolute()
        || text.len() > MAX_PATH_BYTES
        || text.chars().any(char::is_control)
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(refusal(
            "permission paths must be bounded absolute paths without controls or traversal",
        ));
    }
    Ok(())
}
fn no_existing_symlinks(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(refusal("symlink permission path"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn no_symlink_ancestors(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if metadata.file_type().is_symlink() {
            return Err(refusal("symlink permission path"));
        }
    }
    Ok(())
}
fn owned(metadata: &fs::Metadata) -> bool {
    metadata.uid() == unsafe { libc::geteuid() }
}
fn same_inode(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}
fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    safe_absolute(path)?;
    no_symlink_ancestors(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || !owned(&metadata) || metadata.len() > limit {
        return Err(refusal("foreign or oversized permission file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit || !same_inode(&metadata, &fs::symlink_metadata(path)?) {
        return Err(refusal("permission file changed during read"));
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionBackend {
    Claude,
    Codex,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableSpelling {
    pub spelling: String,
    pub canonical_target: Option<PathBuf>,
}
/// Verified, bounded ownership inventory. Preserves exact link spelling.
#[derive(Debug, Clone)]
pub struct VerifiedExecutableInventory {
    canonical: PathBuf,
    links: Vec<PathBuf>,
    fingerprint: String,
}
impl VerifiedExecutableInventory {
    pub fn verify(canonical: &Path, links: &[PathBuf], expected_sha256: &str) -> io::Result<Self> {
        if links.len() > 2 || !valid_fingerprint(expected_sha256) {
            return Err(refusal("invalid executable ownership evidence"));
        }
        safe_absolute(canonical)?;
        no_symlink_ancestors(canonical)?;
        if canonical.canonicalize()? != canonical || fs::metadata(canonical)?.mode() & 0o111 == 0 {
            return Err(refusal("owned binary must be canonical and executable"));
        }
        if format!(
            "{:x}",
            Sha256::digest(read_bounded(canonical, MAX_EXECUTABLE_BYTES)?)
        ) != expected_sha256
        {
            return Err(refusal("owned executable fingerprint changed"));
        }
        for link in links {
            safe_absolute(link)?;
            no_symlink_ancestors(link.parent().ok_or_else(|| refusal("link has no parent"))?)?;
            let metadata = fs::symlink_metadata(link)?;
            if !metadata.file_type().is_symlink()
                || !owned(&metadata)
                || fs::read_link(link)? != canonical
            {
                return Err(refusal("foreign executable link or target"));
            }
        }
        Ok(Self {
            canonical: canonical.into(),
            links: links.into(),
            fingerprint: expected_sha256.into(),
        })
    }
    pub fn canonical(&self) -> &Path {
        &self.canonical
    }
    pub fn links(&self) -> &[PathBuf] {
        &self.links
    }
    pub fn revalidate(&self) -> io::Result<()> {
        Self::verify(&self.canonical, &self.links, &self.fingerprint).map(|_| ())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PinnedRouting {
    pub state_directory: Option<PathBuf>,
    pub host_endpoint: Option<PathBuf>,
}
#[derive(Debug, Clone)]
pub struct PermissionInputs {
    executables: Vec<ExecutableSpelling>,
    routing: PinnedRouting,
}
impl PermissionInputs {
    pub fn validate(
        cli: &PermissionCliInputs,
        inventory: &VerifiedExecutableInventory,
        routing: PinnedRouting,
    ) -> io::Result<Self> {
        inventory.revalidate()?;
        for path in [&routing.state_directory, &routing.host_endpoint]
            .into_iter()
            .flatten()
        {
            safe_absolute(path)?;
        }
        if (cli.permission_link_path.is_some() || cli.permission_alias_path.is_some())
            && cli.permission_installed_binary.is_none()
        {
            return Err(refusal("links require installed binary inventory"));
        }
        let mut executables = vec![
            ExecutableSpelling {
                spelling: "herdr-threads".into(),
                canonical_target: None,
            },
            ExecutableSpelling {
                spelling: "ht".into(),
                canonical_target: None,
            },
        ];
        // Direct setup admits its independently verified current canonical binary.
        executables.push(ExecutableSpelling {
            spelling: inventory
                .canonical
                .to_str()
                .ok_or_else(|| refusal("non-UTF8 executable"))?
                .into(),
            canonical_target: Some(inventory.canonical.clone()),
        });
        for (index, value) in [
            &cli.permission_installed_binary,
            &cli.permission_link_path,
            &cli.permission_alias_path,
        ]
        .into_iter()
        .enumerate()
        {
            let Some(value) = value else {
                continue;
            };
            let path = Path::new(value);
            safe_absolute(path)?;
            if (index == 0 && value != inventory.canonical.to_str().unwrap_or_default())
                || (index != 0
                    && !inventory
                        .links
                        .iter()
                        .any(|p| p.to_str() == Some(value.as_str())))
            {
                return Err(refusal(
                    "executable spelling lacks exact ownership evidence",
                ));
            }
            if !executables.iter().any(|e| e.spelling == *value) {
                executables.push(ExecutableSpelling {
                    spelling: value.clone(),
                    canonical_target: Some(inventory.canonical.clone()),
                });
            }
        }
        Ok(Self {
            executables,
            routing,
        })
    }
    pub fn executables(&self) -> &[ExecutableSpelling] {
        &self.executables
    }
    pub fn routing(&self) -> &PinnedRouting {
        &self.routing
    }
    pub fn catalog(&self) -> OrdinaryCatalog {
        ordinary_catalog()
    }
    /// The catalog's leading routing forms with pinned values bound. A form whose value is
    /// absent, or that `accept` cannot express, is withheld.
    pub(crate) fn routing_tokens(&self, accept: impl Fn(&str) -> bool) -> Vec<Vec<&str>> {
        fn bind<'a>(path: &'a Option<PathBuf>, accept: &impl Fn(&str) -> bool) -> Option<&'a str> {
            path.as_deref()
                .and_then(|p| p.to_str())
                .filter(|value| accept(value))
        }
        self.catalog()
            .routing_forms
            .iter()
            .filter_map(|form| {
                form.iter()
                    .map(|token| match token {
                        RoutingToken::Literal(value) => Some(*value),
                        RoutingToken::StateDirectory => {
                            bind(&self.routing.state_directory, &accept)
                        }
                        RoutingToken::HostEndpoint => bind(&self.routing.host_endpoint, &accept),
                    })
                    .collect()
            })
            .collect()
    }
}

/// Every finite leading arrangement of `command` for one executable spelling: each routing
/// form, and each output flag before the command in both orders around the routing.
pub(crate) fn arrangements<'a>(
    spelling: &'a str,
    routing: &[Vec<&'a str>],
    command: &[&'a str],
) -> Vec<Vec<&'a str>> {
    let catalog = ordinary_catalog();
    let mut found = Vec::new();
    for route in routing {
        let mut plain = vec![spelling];
        plain.extend(route.iter().copied());
        plain.extend(command.iter().copied());
        found.push(plain);
        if catalog
            .output_positions
            .contains(&OutputPosition::BeforeFamily)
        {
            for output in catalog.output_flags {
                let mut after = vec![spelling];
                after.extend(route.iter().copied());
                after.push(output);
                after.extend(command.iter().copied());
                found.push(after);
                // Another finite leading order, never an arbitrary middle glob.
                if !route.is_empty() {
                    let mut before = vec![spelling, output];
                    before.extend(route.iter().copied());
                    before.extend(command.iter().copied());
                    found.push(before);
                }
            }
        }
    }
    found
}
/// Exact file/rule ownership. A pre-existing entry is never removable owned data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedPermissionResource {
    pub file: PathBuf,
    pub rule: Option<String>,
    pub fingerprint: String,
    pub pre_existing: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionComponentManifest {
    pub version: u32,
    pub backend: PermissionBackend,
    pub config_root: PathBuf,
    pub resources: Vec<OwnedPermissionResource>,
    pub executable_spellings: Vec<String>,
}
impl PermissionComponentManifest {
    pub fn validate(&self, identity: &VerifiedConfigIdentity) -> io::Result<()> {
        if self.version != PERMISSION_MANIFEST_VERSION
            || self.backend != identity.backend
            || self.config_root != identity.root
            || self.resources.len() > MAX_RESOURCES
            || self.executable_spellings.len() > 5
        {
            return Err(refusal("invalid permission manifest identity or bounds"));
        }
        let mut seen = BTreeSet::new();
        for resource in &self.resources {
            validate_resource(resource, identity)?;
            if !seen.insert((&resource.file, &resource.rule)) {
                return Err(refusal("duplicate permission resource"));
            }
        }
        for (index, spelling) in self.executable_spellings.iter().enumerate() {
            if spelling != "herdr-threads" && spelling != "ht" {
                safe_absolute(Path::new(spelling))?;
            }
            if self.executable_spellings[..index].contains(spelling) {
                return Err(refusal("duplicate executable spelling"));
            }
        }
        Ok(())
    }
}
fn validate_resource(
    resource: &OwnedPermissionResource,
    identity: &VerifiedConfigIdentity,
) -> io::Result<()> {
    safe_absolute(&resource.file)?;
    if !resource.file.starts_with(&identity.root)
        || !valid_fingerprint(&resource.fingerprint)
        || resource
            .rule
            .as_ref()
            .is_some_and(|r| r.is_empty() || r.len() > 65536 || r.chars().any(char::is_control))
    {
        return Err(refusal("invalid permission resource"));
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionInspectionState {
    Missing,
    ExactHistoricalOwned,
    CurrentOwned,
    Conflict,
    Foreign,
}
/// Exact full-file freshness evidence, separate from permission ownership evidence.
/// Backends must still report native permission files/rules in `observed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionFileBaseline {
    pub file: PathBuf,
    pub fingerprint: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionInspection {
    pub state: PermissionInspectionState,
    pub manifest: Option<PermissionComponentManifest>,
    pub historical: Option<OwnedPermissionResource>,
    pub manifest_fingerprint: Option<String>,
    /// Exact refreshed permission resources used for ownership classification.
    pub observed: Vec<OwnedPermissionResource>,
    /// Publication freshness only; cannot prove ownership or make a component foreign.
    pub file_baselines: Vec<PermissionFileBaseline>,
    pub identity: VerifiedConfigIdentity,
}
impl PermissionInspection {
    /// Read-only: observed entries are permission evidence from bounded exact-byte inspection.
    /// Baselines capture full-file freshness, never substitute for native permission evidence.
    /// Historical proof is accepted only when it exactly matches an observed non-pre-existing entry.
    pub fn inspect(
        identity: &VerifiedConfigIdentity,
        manifest_path: &Path,
        observed: &[OwnedPermissionResource],
        file_baselines: &[PermissionFileBaseline],
        historical: Option<&OwnedPermissionResource>,
    ) -> io::Result<Self> {
        identity.revalidate()?;
        safe_absolute(manifest_path)?;
        no_existing_symlinks(manifest_path)?;
        if observed.len().saturating_add(file_baselines.len()) > MAX_RESOURCES {
            return Err(refusal("too many permission observations"));
        }
        for entry in observed {
            validate_resource(entry, identity)?;
        }
        for baseline in file_baselines {
            safe_absolute(&baseline.file)?;
            if !baseline.file.starts_with(&identity.root)
                || !valid_fingerprint(&baseline.fingerprint)
            {
                return Err(refusal("invalid permission file baseline"));
            }
        }
        if let Some(proof) = historical {
            validate_resource(proof, identity)?;
        }
        let mut result = Self {
            state: PermissionInspectionState::Missing,
            manifest: None,
            historical: None,
            manifest_fingerprint: None,
            observed: observed.to_vec(),
            file_baselines: file_baselines.to_vec(),
            identity: identity.clone(),
        };
        match fs::symlink_metadata(manifest_path) {
            Ok(_) => {
                let bytes = read_bounded(manifest_path, MAX_MANIFEST_BYTES)?;
                let manifest: PermissionComponentManifest = serde_json::from_slice(&bytes)
                    .map_err(|_| refusal("invalid permission manifest"))?;
                manifest.validate(identity)?;
                let observed_resources: BTreeSet<_> = observed
                    .iter()
                    .map(|r| (&r.file, &r.rule, &r.fingerprint, r.pre_existing))
                    .collect();
                result.state = if manifest.resources.iter().all(|r| {
                    observed_resources.contains(&(&r.file, &r.rule, &r.fingerprint, r.pre_existing))
                }) {
                    PermissionInspectionState::CurrentOwned
                } else {
                    PermissionInspectionState::Conflict
                };
                result.manifest_fingerprint = Some(format!("{:x}", Sha256::digest(&bytes)));
                result.manifest = Some(manifest);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                result.state = if let Some(proof) = historical {
                    if !proof.pre_existing && observed.contains(proof) {
                        result.historical = Some(proof.clone());
                        PermissionInspectionState::ExactHistoricalOwned
                    } else if proof.pre_existing {
                        PermissionInspectionState::Foreign
                    } else {
                        PermissionInspectionState::Conflict
                    }
                } else if observed.is_empty() {
                    PermissionInspectionState::Missing
                } else {
                    PermissionInspectionState::Foreign
                };
            }
            Err(e) => return Err(e),
        }
        identity.revalidate()?;
        Ok(result)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionConsent {
    Undecided,
    Granted,
    Declined,
}
/// Native matching scope is decided by the backend, never inferred from executable ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionGrantChange {
    OwnedNarrowing,
    Expansion,
}
#[derive(Debug, Clone)]
pub struct PermissionPlan {
    inspection: PermissionInspection,
    desired: PermissionComponentManifest,
    consent: PermissionConsent,
    requires_consent: bool,
}
impl PermissionPlan {
    pub fn inspection(&self) -> &PermissionInspection {
        &self.inspection
    }
    pub fn desired(&self) -> &PermissionComponentManifest {
        &self.desired
    }
    pub fn consent(&self) -> PermissionConsent {
        self.consent
    }
    pub fn requires_consent(&self) -> bool {
        self.requires_consent
    }
    pub fn prepare(
        inspection: PermissionInspection,
        desired: PermissionComponentManifest,
        consent: PermissionConsent,
        change: PermissionGrantChange,
    ) -> io::Result<Self> {
        desired.validate(&inspection.identity)?;
        let existing_spellings = inspection
            .manifest
            .as_ref()
            .map(|m| m.executable_spellings.as_slice())
            .unwrap_or(&[]);
        let path_expansion = desired.executable_spellings.iter().any(|s| {
            !(existing_spellings.contains(s)
                || inspection.state == PermissionInspectionState::ExactHistoricalOwned
                    && s == "herdr-threads")
        });
        let requires_consent = inspection.state == PermissionInspectionState::Missing
            || change == PermissionGrantChange::Expansion
            || path_expansion;
        Ok(Self {
            inspection,
            desired,
            consent,
            requires_consent,
        })
    }
    pub fn authorize(&self) -> io::Result<()> {
        if matches!(
            self.inspection.state,
            PermissionInspectionState::Conflict | PermissionInspectionState::Foreign
        ) {
            return Err(refusal("conflicting or foreign permission component"));
        }
        if self.requires_consent && self.consent != PermissionConsent::Granted {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "ordinary read/write permissions require explicit component consent; human/operator forms remain withheld",
            ));
        }
        Ok(())
    }
    /// Backend calls after refreshing exact observations under the supplied guard.
    pub fn revalidate(
        &self,
        guard: &OwnedConfigWriteGuard,
        refreshed: &PermissionInspection,
    ) -> io::Result<()> {
        guard.revalidate()?;
        if guard.identity() != &self.inspection.identity || refreshed != &self.inspection {
            return Err(refusal("permission plan changed; inspect and plan again"));
        }
        self.authorize()
    }
}
/// No implementation is installed by this seam. Backend writers share one guard.
pub trait PermissionComponent {
    fn inspect(&self) -> io::Result<PermissionInspection>;
    fn plan(
        &self,
        inputs: &PermissionInputs,
        consent: PermissionConsent,
    ) -> io::Result<PermissionPlan>;
    fn install(
        &self,
        guard: &OwnedConfigWriteGuard,
        plan: &PermissionPlan,
    ) -> io::Result<PermissionInspection>;
    fn remove(
        &self,
        guard: &OwnedConfigWriteGuard,
        inspection: &PermissionInspection,
    ) -> io::Result<PermissionInspection>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedConfigIdentity {
    root: PathBuf,
    backend: PermissionBackend,
    device: u64,
    inode: u64,
}
impl VerifiedConfigIdentity {
    pub fn verify(root: &Path, backend: PermissionBackend) -> io::Result<Self> {
        safe_absolute(root)?;
        no_symlink_ancestors(root)?;
        let metadata = fs::metadata(root)?;
        if !metadata.is_dir()
            || !owned(&metadata)
            || metadata.mode() & 0o022 != 0
            || root.canonicalize()? != root
        {
            return Err(refusal(
                "config root must be a verified owned effective directory",
            ));
        }
        Ok(Self {
            root: root.into(),
            backend,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn backend(&self) -> PermissionBackend {
        self.backend
    }
    pub fn revalidate(&self) -> io::Result<()> {
        if Self::verify(&self.root, self.backend)? != *self {
            return Err(refusal("effective config identity changed"));
        }
        Ok(())
    }
    fn lock_path(&self) -> PathBuf {
        self.root.join(match self.backend {
            PermissionBackend::Claude => ".herdr-threads-claude.lock",
            PermissionBackend::Codex => ".herdr-threads-codex.lock",
        })
    }
    fn marker(&self) -> Vec<u8> {
        format!(
            "herdr-threads owned config lock v1\n{:?}\n{}:{}\n{}\n",
            self.backend,
            self.device,
            self.inode,
            self.root.display()
        )
        .into_bytes()
    }
}
/// Stable owned inode, no unlink. Advisory exclusion covers cooperating writers;
/// backend final-check/rename still cannot detect a noncooperating editor in that interval.
pub struct OwnedConfigWriteGuard {
    file: File,
    identity: VerifiedConfigIdentity,
}
/// Operation-local observations for causal initialization tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigLockInitialization {
    Created,
    ReadyToPublish,
}
impl OwnedConfigWriteGuard {
    pub fn acquire(identity: &VerifiedConfigIdentity, timeout: Duration) -> io::Result<Self> {
        Self::acquire_observed(identity, timeout, |_, _| Ok(()))
    }
    #[cfg(feature = "test-support")]
    pub fn acquire_with_observer(
        identity: &VerifiedConfigIdentity,
        timeout: Duration,
        observer: impl FnMut(ConfigLockInitialization, &Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        Self::acquire_observed(identity, timeout, observer)
    }
    fn acquire_observed(
        identity: &VerifiedConfigIdentity,
        timeout: Duration,
        mut observer: impl FnMut(ConfigLockInitialization, &Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        if timeout > Duration::from_secs(10) {
            return Err(refusal("config lock timeout exceeds bound"));
        }
        let deadline = Instant::now() + timeout;
        identity.revalidate()?;
        let path = identity.lock_path();
        let options = || {
            let mut o = OpenOptions::new();
            o.read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW);
            o
        };
        let file = match options().open(&path) {
            Ok(file) => {
                check_lock_metadata(&file, &path)?;
                acquire_config_lock(&file, deadline)?;
                file
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // No incomplete inode is ever exposed at the stable name. A terminated
                // creator may leave this private name; it is never adopted or repaired.
                let private = identity
                    .root
                    .join(format!(".herdr-threads-lock-init-{}", uuid::Uuid::new_v4()));
                let mut file = options().create_new(true).open(&private)?;
                let mut cleanup = UnpublishedLockName::new(&private, &file)?;
                check_lock_metadata(&file, &private)?;
                observer(ConfigLockInitialization::Created, &private)?;
                file.write_all(&identity.marker())?;
                file.sync_all()?;
                acquire_config_lock(&file, deadline)?;
                observer(ConfigLockInitialization::ReadyToPublish, &private)?;
                identity.revalidate()?;
                check_lock_metadata(&file, &private)?;
                match publish_config_lock(&private, &path) {
                    Ok(()) => {
                        cleanup.path = None;
                        File::open(&identity.root)?.sync_all()?;
                        file
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        // The winning creator published a complete, already locked inode.
                        // Discard only our unpublished inode, then use the same deadline.
                        drop(file);
                        drop(cleanup);
                        let file = options().open(&path)?;
                        check_lock_metadata(&file, &path)?;
                        acquire_config_lock(&file, deadline)?;
                        file
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };
        let guard = Self {
            file,
            identity: identity.clone(),
        };
        guard.revalidate()?;
        Ok(guard)
    }
    pub fn identity(&self) -> &VerifiedConfigIdentity {
        &self.identity
    }
    pub fn revalidate(&self) -> io::Result<()> {
        self.identity.revalidate()?;
        check_lock_metadata(&self.file, &self.identity.lock_path())?;
        // Read from a separately opened descriptor without acquiring/releasing another lock.
        if read_bounded(&self.identity.lock_path(), MAX_PATH_BYTES as u64 + 128)?
            != self.identity.marker()
        {
            return Err(refusal("foreign or edited config lock"));
        }
        Ok(())
    }
}
fn acquire_config_lock(file: &File, deadline: Instant) -> io::Result<()> {
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "owned config writer busy; retry after it completes",
                    ));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(2)));
            }
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}
// Drop cleanup is scoped to the exact private inode this operation created.
struct UnpublishedLockName {
    path: Option<PathBuf>,
    device: u64,
    inode: u64,
}
impl UnpublishedLockName {
    fn new(path: &Path, file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            path: Some(path.into()),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}
impl Drop for UnpublishedLockName {
    fn drop(&mut self) {
        if let Some(path) = &self.path
            && let Ok(metadata) = fs::symlink_metadata(path)
            && metadata.is_file()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(path);
        }
    }
}
fn publish_config_lock(private: &Path, stable: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let private = CString::new(private.as_os_str().as_bytes())
            .map_err(|_| refusal("invalid private config lock path"))?;
        let stable = CString::new(stable.as_os_str().as_bytes())
            .map_err(|_| refusal("invalid stable config lock path"))?;
        // SAFETY: both C strings are live and NUL terminated. These primitives
        // atomically move one name without replacing a winner or adding a link.
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(private.as_ptr(), stable.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                private.as_ptr(),
                libc::AT_FDCWD,
                stable.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (private, stable);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace config lock publication is unavailable",
        ))
    }
}
fn check_lock_metadata(file: &File, path: &Path) -> io::Result<()> {
    let metadata = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || !owned(&metadata)
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || !same_inode(&metadata, &named)
        || named.file_type().is_symlink()
    {
        return Err(refusal("foreign, replaced or unsafe config lock"));
    }
    Ok(())
}
impl Drop for OwnedConfigWriteGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("ht-permissions-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(p.canonicalize().unwrap())
        }
        fn identity(&self) -> VerifiedConfigIdentity {
            VerifiedConfigIdentity::verify(&self.0, PermissionBackend::Claude).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn resource(root: &Path) -> OwnedPermissionResource {
        OwnedPermissionResource {
            file: root.join("settings.json"),
            rule: Some("Bash(herdr-threads *)".into()),
            fingerprint: "a".repeat(64),
            pre_existing: false,
        }
    }
    fn manifest(root: &Path) -> PermissionComponentManifest {
        PermissionComponentManifest {
            version: 1,
            backend: PermissionBackend::Claude,
            config_root: root.into(),
            resources: vec![resource(root)],
            executable_spellings: vec!["herdr-threads".into()],
        }
    }
    #[test]
    fn permission_inspection_does_not_create_files() {
        let f = Fixture::new();
        let proof = resource(&f.0);
        let inspection = PermissionInspection::inspect(
            &f.identity(),
            &f.0.join("manifest.json"),
            std::slice::from_ref(&proof),
            &[],
            Some(&proof),
        )
        .unwrap();
        assert_eq!(
            inspection.state,
            PermissionInspectionState::ExactHistoricalOwned
        );
        assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 0);
    }
    #[test]
    fn permission_plan_requires_component_consent() {
        let f = Fixture::new();
        let inspection = PermissionInspection::inspect(
            &f.identity(),
            &f.0.join("manifest.json"),
            &[],
            &[],
            None,
        )
        .unwrap();
        let plan = PermissionPlan::prepare(
            inspection,
            manifest(&f.0),
            PermissionConsent::Undecided,
            PermissionGrantChange::Expansion,
        )
        .unwrap();
        assert!(
            plan.authorize().is_err(),
            "ownership does not authorize missing grants"
        );
    }
    #[test]
    fn permission_inputs_reject_invalid_and_changed_inventory() {
        let f = Fixture::new();
        let binary = f.0.join("binary");
        fs::write(&binary, b"owned").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let digest = "f5e6d024c05c9cc2746a3e127408b91a8b7a7f2a30da0c259bc54265502ddef4";
        assert!(VerifiedExecutableInventory::verify(&binary, &[], &"0".repeat(64)).is_err());
        let inventory = VerifiedExecutableInventory::verify(&binary, &[], digest).unwrap();
        for value in [
            "relative",
            "/bad\npath",
            "/bad/../path",
            &format!("/{}", "x".repeat(4096)),
        ] {
            let cli = PermissionCliInputs {
                permission_installed_binary: Some(value.into()),
                ..Default::default()
            };
            assert!(
                PermissionInputs::validate(&cli, &inventory, PinnedRouting::default()).is_err(),
                "accepted {value:?}"
            );
        }
        let routing = PinnedRouting {
            state_directory: Some("relative".into()),
            host_endpoint: None,
        };
        assert!(
            PermissionInputs::validate(&PermissionCliInputs::default(), &inventory, routing)
                .is_err()
        );
        let input = PermissionInputs::validate(
            &PermissionCliInputs::default(),
            &inventory,
            PinnedRouting::default(),
        )
        .unwrap();
        assert_eq!(
            input
                .executables()
                .iter()
                .map(|e| e.spelling.as_str())
                .collect::<Vec<_>>(),
            vec!["herdr-threads", "ht", binary.to_str().unwrap()]
        );
        assert!(
            input
                .catalog()
                .families
                .iter()
                .any(|f| f.prefix == ["send"])
        );
        fs::write(&binary, b"edited").unwrap();
        assert!(
            PermissionInputs::validate(
                &PermissionCliInputs::default(),
                &inventory,
                PinnedRouting::default()
            )
            .is_err()
        );
    }
    #[test]
    fn permission_inputs_preserve_spelling_deduplicate_and_refuse_link_races() {
        let f = Fixture::new();
        let binary = f.0.join("binary");
        fs::write(&binary, b"owned").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let link = f.0.join("ht space");
        symlink(&binary, &link).unwrap();
        let inventory = VerifiedExecutableInventory::verify(
            &binary,
            std::slice::from_ref(&link),
            "f5e6d024c05c9cc2746a3e127408b91a8b7a7f2a30da0c259bc54265502ddef4",
        )
        .unwrap();
        let cli = PermissionCliInputs {
            permission_installed_binary: Some(binary.display().to_string()),
            permission_link_path: Some(link.display().to_string()),
            permission_alias_path: Some(link.display().to_string()),
            ..Default::default()
        };
        let inputs =
            PermissionInputs::validate(&cli, &inventory, PinnedRouting::default()).unwrap();
        assert_eq!(inputs.executables().len(), 4);
        assert_eq!(
            inputs.executables()[3].canonical_target.as_deref(),
            Some(binary.as_path())
        );
        assert_eq!(inputs.executables()[3].spelling, link.to_str().unwrap());
        fs::remove_file(&link).unwrap();
        symlink(f.0.join("foreign"), &link).unwrap();
        assert!(PermissionInputs::validate(&cli, &inventory, PinnedRouting::default()).is_err());
    }
    #[test]
    fn permission_manifest_bounds_and_exact_duplicates_remain_refused() {
        let f = Fixture::new();
        let id = f.identity();
        let proof = resource(&f.0);
        let mut m = manifest(&f.0);
        m.resources = (0..=MAX_RESOURCES)
            .map(|index| {
                let mut resource = proof.clone();
                resource.rule = Some(format!("Bash(ht inbox --fixture {index})"));
                resource
            })
            .collect();
        assert!(m.validate(&id).is_err());
        m.resources = vec![proof.clone(), proof.clone()];
        m.resources[1].fingerprint = "b".repeat(64);
        assert!(
            m.validate(&id).is_err(),
            "same file/rule with changed fingerprint is still duplicate"
        );
        m.resources = vec![proof];
        m.resources[0].rule = Some("x".repeat(65537));
        assert!(m.validate(&id).is_err(), "individual rule bound changed");
        let path = f.0.join("manifest.json");
        let mut padded = serde_json::to_vec(&manifest(&f.0)).unwrap();
        // Valid JSON with trailing whitespace: absent the byte limit, inspection succeeds.
        padded.resize(MAX_MANIFEST_BYTES as usize + 1, b' ');
        fs::write(&path, padded).unwrap();
        assert!(
            PermissionInspection::inspect(&id, &path, &[], &[], None).is_err(),
            "oversized manifest accepted"
        );
    }
    #[test]
    fn permission_inspection_classifies_exact_edited_missing_and_foreign() {
        let f = Fixture::new();
        let id = f.identity();
        let path = f.0.join("manifest.json");
        let proof = resource(&f.0);
        assert_eq!(
            PermissionInspection::inspect(&id, &path, &[], &[], None)
                .unwrap()
                .state,
            PermissionInspectionState::Missing
        );
        assert_eq!(
            PermissionInspection::inspect(&id, &path, std::slice::from_ref(&proof), &[], None)
                .unwrap()
                .state,
            PermissionInspectionState::Foreign
        );
        assert_eq!(
            PermissionInspection::inspect(&id, &path, &[], &[], Some(&proof))
                .unwrap()
                .state,
            PermissionInspectionState::Conflict
        );
        let mut foreign = proof.clone();
        foreign.pre_existing = true;
        assert_eq!(
            PermissionInspection::inspect(
                &id,
                &path,
                std::slice::from_ref(&foreign),
                &[],
                Some(&foreign)
            )
            .unwrap()
            .state,
            PermissionInspectionState::Foreign
        );
        fs::write(&path, serde_json::to_vec(&manifest(&f.0)).unwrap()).unwrap();
        assert_eq!(
            PermissionInspection::inspect(&id, &path, std::slice::from_ref(&proof), &[], None)
                .unwrap()
                .state,
            PermissionInspectionState::CurrentOwned
        );
        let mut edited = proof.clone();
        edited.fingerprint = "b".repeat(64);
        assert_eq!(
            PermissionInspection::inspect(&id, &path, &[edited], &[], None)
                .unwrap()
                .state,
            PermissionInspectionState::Conflict
        );
        let mut bad_manifest = manifest(&f.0);
        bad_manifest.version = 99;
        fs::write(&path, serde_json::to_vec(&bad_manifest).unwrap()).unwrap();
        assert!(PermissionInspection::inspect(&id, &path, &[], &[], None).is_err());
        fs::write(&path, b"malformed").unwrap();
        assert!(PermissionInspection::inspect(&id, &path, &[], &[], None).is_err());
        fs::remove_file(&path).unwrap();
        symlink(f.0.join("missing"), &path).unwrap();
        assert!(PermissionInspection::inspect(&id, &path, &[], &[], None).is_err());
    }
    #[test]
    fn permission_baseline_only_missing_requires_consent_and_freshness() {
        let f = Fixture::new();
        let id = f.identity();
        let path = f.0.join("manifest.json");
        fs::write(f.0.join("settings.json"), b"unrelated settings").unwrap();
        let mut baseline = PermissionFileBaseline {
            file: f.0.join("settings.json"),
            fingerprint: format!("{:x}", Sha256::digest(b"unrelated settings")),
        };
        let inspection =
            PermissionInspection::inspect(&id, &path, &[], std::slice::from_ref(&baseline), None)
                .unwrap();
        assert_eq!(inspection.state, PermissionInspectionState::Missing);
        for consent in [PermissionConsent::Undecided, PermissionConsent::Declined] {
            assert!(
                PermissionPlan::prepare(
                    inspection.clone(),
                    manifest(&f.0),
                    consent,
                    PermissionGrantChange::Expansion,
                )
                .unwrap()
                .authorize()
                .is_err()
            );
        }
        let plan = PermissionPlan::prepare(
            inspection.clone(),
            manifest(&f.0),
            PermissionConsent::Granted,
            PermissionGrantChange::Expansion,
        )
        .unwrap();
        assert!(plan.authorize().is_ok());
        let guard = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        assert!(plan.revalidate(&guard, &inspection).is_ok());
        fs::write(&baseline.file, b"changed unrelated settings").unwrap();
        baseline.fingerprint = format!("{:x}", Sha256::digest(fs::read(&baseline.file).unwrap()));
        let refreshed = PermissionInspection::inspect(&id, &path, &[], &[baseline], None).unwrap();
        assert_eq!(refreshed.state, PermissionInspectionState::Missing);
        assert!(plan.revalidate(&guard, &refreshed).is_err());
    }
    #[test]
    fn permission_baselines_do_not_prove_native_file_ownership() {
        let f = Fixture::new();
        let id = f.identity();
        let path = f.0.join("manifest.json");
        let mut native_file = resource(&f.0);
        native_file.rule = None;
        let baseline = PermissionFileBaseline {
            file: native_file.file.clone(),
            fingerprint: native_file.fingerprint.clone(),
        };
        let inspect = |observed: &[OwnedPermissionResource], historical| {
            PermissionInspection::inspect(
                &id,
                &path,
                observed,
                std::slice::from_ref(&baseline),
                historical,
            )
            .unwrap()
        };
        let foreign = inspect(std::slice::from_ref(&native_file), None);
        assert_eq!(foreign.state, PermissionInspectionState::Foreign);
        assert!(
            PermissionPlan::prepare(
                foreign,
                manifest(&f.0),
                PermissionConsent::Granted,
                PermissionGrantChange::Expansion,
            )
            .unwrap()
            .authorize()
            .is_err()
        );
        assert_eq!(
            inspect(&[], Some(&native_file)).state,
            PermissionInspectionState::Conflict
        );
        let mut desired = manifest(&f.0);
        desired.resources = vec![native_file.clone()];
        fs::write(&path, serde_json::to_vec(&desired).unwrap()).unwrap();
        assert_eq!(
            inspect(&[], None).state,
            PermissionInspectionState::Conflict
        );
        assert_eq!(
            inspect(std::slice::from_ref(&native_file), None).state,
            PermissionInspectionState::CurrentOwned
        );
        let invalid = PermissionFileBaseline {
            file: "relative".into(),
            fingerprint: "a".repeat(64),
        };
        assert!(PermissionInspection::inspect(&id, &path, &[], &[invalid], None).is_err());
        let invalid = PermissionFileBaseline {
            file: f.0.join("settings.json"),
            fingerprint: "bad".into(),
        };
        assert!(PermissionInspection::inspect(&id, &path, &[], &[invalid], None).is_err());
        assert!(
            PermissionInspection::inspect(
                &id,
                &path,
                &[],
                &vec![baseline; MAX_RESOURCES + 1],
                None,
            )
            .is_err()
        );
    }
    #[test]
    fn permission_plan_separates_narrowing_expansion_and_refreshed_base() {
        let f = Fixture::new();
        let id = f.identity();
        let path = f.0.join("manifest.json");
        let proof = resource(&f.0);
        let historical = PermissionInspection::inspect(
            &id,
            &path,
            std::slice::from_ref(&proof),
            &[],
            Some(&proof),
        )
        .unwrap();
        let narrow = PermissionPlan::prepare(
            historical.clone(),
            manifest(&f.0),
            PermissionConsent::Declined,
            PermissionGrantChange::OwnedNarrowing,
        )
        .unwrap();
        assert!(narrow.authorize().is_ok());
        let mut expanded = manifest(&f.0);
        expanded.executable_spellings.push("ht".into());
        let declined = PermissionPlan::prepare(
            historical.clone(),
            expanded.clone(),
            PermissionConsent::Declined,
            PermissionGrantChange::OwnedNarrowing,
        )
        .unwrap();
        assert!(declined.authorize().is_err());
        let mut native_expansion = manifest(&f.0);
        native_expansion.resources[0].rule = Some("Bash(ht send *)".into());
        assert!(
            PermissionPlan::prepare(
                historical.clone(),
                native_expansion,
                PermissionConsent::Declined,
                PermissionGrantChange::Expansion
            )
            .unwrap()
            .authorize()
            .is_err()
        );
        let granted = PermissionPlan::prepare(
            historical.clone(),
            expanded,
            PermissionConsent::Granted,
            PermissionGrantChange::Expansion,
        )
        .unwrap();
        assert!(granted.authorize().is_ok());
        let guard = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        assert!(granted.revalidate(&guard, &historical).is_ok());
        let refreshed = PermissionInspection::inspect(&id, &path, &[], &[], Some(&proof)).unwrap();
        assert!(granted.revalidate(&guard, &refreshed).is_err());
        let foreign =
            PermissionInspection::inspect(&id, &path, std::slice::from_ref(&proof), &[], None)
                .unwrap();
        assert!(
            PermissionPlan::prepare(
                foreign,
                manifest(&f.0),
                PermissionConsent::Granted,
                PermissionGrantChange::Expansion
            )
            .unwrap()
            .authorize()
            .is_err()
        );
    }
    #[test]
    fn owned_config_guard_refuses_foreign_symlink_edited_and_replaced_inode() {
        let f = Fixture::new();
        let id = f.identity();
        let path = id.lock_path();
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::from_secs(11)).is_err());
        assert!(!path.exists());
        for bytes in [b"".as_slice(), b"herdr-threads owned config lock v1\n"] {
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let inode = fs::metadata(&path).unwrap().ino();
            assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
            fs::remove_file(&path).unwrap();
        }
        fs::write(&path, b"foreign").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign");
        fs::remove_file(&path).unwrap();
        symlink(f.0.join("missing"), &path).unwrap();
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
        fs::remove_file(&path).unwrap();
        let guard = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(guard.revalidate().is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&path, f.0.join("extra-link")).unwrap();
        assert!(guard.revalidate().is_err());
        fs::remove_file(f.0.join("extra-link")).unwrap();
        guard.revalidate().unwrap();
        fs::write(&path, b"edited").unwrap();
        assert!(guard.revalidate().is_err());
        drop(guard);
        assert!(OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).is_err());
        fs::remove_file(&path).unwrap();
        let guard = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        fs::rename(&path, f.0.join("old-lock")).unwrap();
        fs::write(&path, id.marker()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(guard.revalidate().is_err());
    }
    // Child test re-entry uses per-command environment only, never parent-global state.
    #[cfg(feature = "test-support")]
    #[test]
    fn guard_child() {
        let Some(root) = std::env::var_os("HT_PERMISSION_GUARD_CHILD_ROOT") else {
            return;
        };
        let id =
            VerifiedConfigIdentity::verify(Path::new(&root), PermissionBackend::Claude).unwrap();
        let barrier = PathBuf::from(std::env::var_os("HT_PERMISSION_GUARD_CHILD_BARRIER").unwrap());
        match OwnedConfigWriteGuard::acquire(&id, Duration::ZERO) {
            Ok(_guard) => {
                fs::write(barrier.join("result.tmp"), b"acquired").unwrap();
                fs::rename(barrier.join("result.tmp"), barrier.join("result")).unwrap();
                let end = Instant::now() + Duration::from_secs(5);
                while !barrier.join("release").exists() {
                    assert!(Instant::now() < end, "child release timed out");
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                fs::write(barrier.join("result.tmp"), b"busy").unwrap();
                fs::rename(barrier.join("result.tmp"), barrier.join("result")).unwrap();
            }
            Err(e) => panic!("unexpected child guard error: {e}"),
        }
    }
    #[cfg(feature = "test-support")]
    #[test]
    fn owned_config_guard_excludes_other_process() {
        use crate::test_support::spawn::{SpawnOwned, command};
        let f = Fixture::new();
        let id = f.identity();
        let barrier = f.0.join("barrier");
        fs::create_dir(&barrier).unwrap();
        let lock = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        let inode = fs::metadata(id.lock_path()).unwrap().ino();
        let child = || {
            let mut command = command(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "harness::permissions::tests::guard_child",
                    "--nocapture",
                ])
                .env("HT_PERMISSION_GUARD_CHILD_ROOT", &f.0)
                .env("HT_PERMISSION_GUARD_CHILD_BARRIER", &barrier)
                .env("HOME", f.0.join("home"))
                .env("CLAUDE_CONFIG_DIR", &f.0)
                .env("CODEX_HOME", &f.0)
                .env("TMPDIR", &f.0);
            command.spawn_owned().unwrap()
        };
        let wait_result = || {
            let end = Instant::now() + Duration::from_secs(5);
            while !barrier.join("result").exists() {
                assert!(Instant::now() < end, "child result timed out");
                std::thread::sleep(Duration::from_millis(2));
            }
            fs::read(barrier.join("result")).unwrap()
        };
        let mut blocked = child();
        assert_eq!(wait_result(), b"busy");
        assert!(blocked.wait().unwrap().success());
        drop(blocked);
        drop(lock);
        fs::remove_file(barrier.join("result")).unwrap();
        let mut holder = child();
        assert_eq!(wait_result(), b"acquired");
        assert_eq!(
            OwnedConfigWriteGuard::acquire(&id, Duration::ZERO)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        fs::write(barrier.join("release"), b"release").unwrap();
        assert!(holder.wait().unwrap().success());
        drop(holder);
        let _next = OwnedConfigWriteGuard::acquire(&id, Duration::ZERO).unwrap();
        assert_eq!(fs::metadata(id.lock_path()).unwrap().ino(), inode);
    }
}
