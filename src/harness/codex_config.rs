//! The owned Codex sandbox allowance in the user's `$CODEX_HOME/config.toml`.
//!
//! Three keys let Codex's default `workspace-write` sandbox reach exactly one Unix socket (the
//! herdr-threads daemon's stable socket) and nothing else:
//! `sandbox_workspace_write.network_access = true`, `features.network_proxy.enabled = true` and
//! `features.network_proxy.unix_sockets."<socket>" = "allow"`. Since manifest version 2 the
//! allowance also adds this instance's client-side journal directories (`<instance>/intents`
//! and `<instance>/contexts`) to `sandbox_workspace_write.writable_roots`: mutating commands
//! record a pending-operation intent and the caller's context there, and workspace-write
//! otherwise refuses those writes (EPERM). Only those two directories are added, never the
//! instance directory, the SQLite database or the daemon's own files. Each root is one owned
//! array member: a member the user already had is pre-existing, the array is created when
//! absent, and removal deletes only owned members (and the array when setup created it and it
//! is empty again). A version-1 manifest (socket keys only) is upgraded in place.
//!
//! Editing is structural (`toml_edit`, which keeps every other byte, comment and layout), and
//! ownership is recorded in a private manifest like the JSON hook installations: a key the user
//! already had with the same value is recorded as pre-existing and never removed; a key with a
//! different value refuses installation; removal deletes only owned keys whose value is still
//! ours (an edited one refuses) and restores the file byte for byte when nothing else changed.
//! Codex's own later writes (for example `hooks.state` trust hashes) are kept.
use super::setup::{
    InstallPhase, SetupError, config_bytes, fingerprint, publish_manifest, write_replacement,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike, Value};

const MAX_CONFIG: usize = 1_048_576;

/// One owned (or pre-existing) key and the value the allowance needs there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowanceKey {
    pub path: Vec<String>,
    pub value: AllowanceValue,
    /// The user's file already held this exact key and value: never added, never removed.
    pub pre_existing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AllowanceValue {
    Bool(bool),
    String(String),
    /// One string member of the array at the key path (`writable_roots`).
    Member(String),
}

/// The array key the writable roots are members of.
pub const WRITABLE_ROOTS: [&str; 2] = ["sandbox_workspace_write", "writable_roots"];

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowanceManifest {
    pub version: u8,
    pub harness: String,
    pub kind: String,
    pub path: String,
    pub socket: String,
    /// The writable roots the allowance adds (version 2); empty in version 1.
    #[serde(default)]
    pub writable_roots: Vec<String>,
    pub original_bytes: Vec<u8>,
    pub installed_fingerprint: String,
    pub keys: Vec<AllowanceKey>,
    /// Tables (and the roots array) setup created (deepest last); removed again only when empty.
    pub created: Vec<Vec<String>>,
    pub phase: InstallPhase,
    /// Removal may restore `original_bytes` when the file still has the installed fingerprint.
    /// False after an upgrade of a file that had changed since the first installation (its
    /// fingerprint then covers later edits that `original_bytes` lacks).
    #[serde(default = "default_true")]
    pub restore_exact: bool,
    /// A prepared upgrade: the file bytes it was composed from.
    #[serde(default)]
    pub upgrade_base: Option<Vec<u8>>,
}

/// Why an allowance could not be composed, with the offending dotted key where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowanceError {
    Setup(SetupError),
    /// The key exists with another value (or a non-table where a table is needed).
    Foreign(String),
}

impl From<SetupError> for AllowanceError {
    fn from(error: SetupError) -> Self {
        Self::Setup(error)
    }
}

/// The keys the allowance for `socket` and `roots` needs, in a fixed order: the three socket
/// keys, then one `writable_roots` member per root.
pub fn allowance_keys(socket: &str, roots: &[String]) -> Vec<(Vec<String>, AllowanceValue)> {
    let path = |parts: &[&str]| parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    let mut keys = vec![
        (
            path(&["sandbox_workspace_write", "network_access"]),
            AllowanceValue::Bool(true),
        ),
        (
            path(&["features", "network_proxy", "enabled"]),
            AllowanceValue::Bool(true),
        ),
        (
            path(&["features", "network_proxy", "unix_sockets", socket]),
            AllowanceValue::String("allow".into()),
        ),
    ];
    keys.extend(
        roots
            .iter()
            .map(|root| (path(&WRITABLE_ROOTS), AllowanceValue::Member(root.clone()))),
    );
    keys
}

/// The manifest version for an allowance with `roots`.
fn version_for(roots: &[String]) -> u8 {
    if roots.is_empty() { 1 } else { 2 }
}

fn dotted(path: &[String]) -> String {
    path.iter()
        .map(|part| {
            if !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                part.clone()
            } else {
                serde_json::to_string(part).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn parse(bytes: &[u8]) -> Result<DocumentMut, SetupError> {
    if bytes.len() > MAX_CONFIG {
        return Err(SetupError::TooLarge);
    }
    std::str::from_utf8(bytes)
        .map_err(|_| SetupError::Invalid)?
        .parse::<DocumentMut>()
        .map_err(|_| SetupError::Invalid)
}

fn render(doc: &DocumentMut) -> Result<Vec<u8>, SetupError> {
    let text = doc.to_string();
    // The result must itself be a valid Codex config file.
    text.parse::<DocumentMut>()
        .map_err(|_| SetupError::Invalid)?;
    if text.len() > MAX_CONFIG {
        return Err(SetupError::TooLarge);
    }
    Ok(text.into_bytes())
}

fn lookup<'a>(doc: &'a DocumentMut, path: &[String]) -> Option<&'a Item> {
    let mut item = doc.as_item();
    for part in path {
        item = item.as_table_like()?.get(part)?;
    }
    Some(item)
}

fn container_mut<'a>(doc: &'a mut DocumentMut, path: &[String]) -> Option<&'a mut dyn TableLike> {
    let mut item: &mut Item = doc.as_item_mut();
    for part in path {
        item = item.as_table_like_mut()?.get_mut(part)?;
    }
    item.as_table_like_mut()
}

fn matches(item: &Item, value: &AllowanceValue) -> bool {
    match value {
        AllowanceValue::Bool(want) => item.as_bool() == Some(*want),
        AllowanceValue::String(want) => item.as_str() == Some(want.as_str()),
        AllowanceValue::Member(want) => item
            .as_array()
            .is_some_and(|array| array.iter().any(|v| v.as_str() == Some(want.as_str()))),
    }
}

fn to_value(value: &AllowanceValue) -> Value {
    match value {
        AllowanceValue::Bool(b) => Value::from(*b),
        AllowanceValue::String(s) | AllowanceValue::Member(s) => Value::from(s.as_str()),
    }
}

/// Insert `value` at `path`, creating missing containers. The socket map is created as an
/// inline table (its key is a path); other containers as tables, implicit when they only hold
/// sub-tables. Records every container it creates.
fn insert(
    doc: &mut DocumentMut,
    path: &[String],
    value: &AllowanceValue,
    created: &mut Vec<Vec<String>>,
) -> Result<(), AllowanceError> {
    let (last, parents) = path.split_last().ok_or(SetupError::Invalid)?;
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    // Inside an inline (or dotted) table only inline values can be added.
    let mut inline = false;
    for (depth, part) in parents.iter().enumerate() {
        let here = &path[..=depth];
        if table.get(part).is_none() {
            let inline_map = here.last().is_some_and(|p| p == "unix_sockets");
            let item = if inline_map || inline || table.is_dotted() {
                Item::Value(Value::InlineTable(InlineTable::new()))
            } else {
                let mut new = Table::new();
                new.set_implicit(depth + 1 < parents.len());
                Item::Table(new)
            };
            table.insert(part, item);
            created.push(here.to_vec());
        }
        inline = matches!(table.get(part), Some(Item::Value(_)));
        table = table
            .get_mut(part)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| AllowanceError::Foreign(dotted(here)))?;
    }
    if let AllowanceValue::Member(_) = value {
        match table.get_mut(last) {
            Some(item) => item
                .as_array_mut()
                .ok_or_else(|| AllowanceError::Foreign(dotted(path)))?
                .push(to_value(value)),
            None => {
                let mut array = toml_edit::Array::new();
                array.push(to_value(value));
                table.insert(last, Item::Value(Value::Array(array)));
                created.push(path.to_vec());
            }
        }
        return Ok(());
    }
    table.insert(last, Item::Value(to_value(value)));
    Ok(())
}

/// Add each key to `doc` (`pre_existing` when already there with its value). A key holding
/// another value refuses; a roots array without the member gets it appended.
fn compose(
    doc: &mut DocumentMut,
    wanted: Vec<(Vec<String>, AllowanceValue)>,
    created: &mut Vec<Vec<String>>,
) -> Result<Vec<AllowanceKey>, AllowanceError> {
    let mut keys = Vec::new();
    for (path, value) in wanted {
        match lookup(doc, &path) {
            Some(item) if matches(item, &value) => keys.push(AllowanceKey {
                path,
                value,
                pre_existing: true,
            }),
            Some(item)
                if !matches!(value, AllowanceValue::Member(_)) || item.as_array().is_none() =>
            {
                return Err(AllowanceError::Foreign(dotted(&path)));
            }
            _ => {
                insert(doc, &path, &value, created)?;
                keys.push(AllowanceKey {
                    path,
                    value,
                    pre_existing: false,
                });
            }
        }
    }
    Ok(keys)
}

/// A composed allowance: the new file bytes, the key records and the
/// containers created.
pub type AllowancePlan = (Vec<u8>, Vec<AllowanceKey>, Vec<Vec<String>>);

/// Compose the allowance for `socket` and `roots` into `base`: the new bytes, the key records
/// and the containers created. A key already holding another value refuses.
pub fn plan_install(
    base: &[u8],
    socket: &str,
    roots: &[String],
) -> Result<AllowancePlan, AllowanceError> {
    let mut doc = parse(base)?;
    let mut created = Vec::new();
    let keys = compose(&mut doc, allowance_keys(socket, roots), &mut created)?;
    Ok((render(&doc)?, keys, created))
}

/// Re-add the owned keys missing from `base` (a prepared installation or upgrade publishing).
fn reapply(base: &[u8], keys: &[AllowanceKey]) -> Result<Vec<u8>, AllowanceError> {
    let mut doc = parse(base)?;
    let mut created = Vec::new();
    for key in keys.iter().filter(|key| !key.pre_existing) {
        if !lookup(&doc, &key.path).is_some_and(|item| matches(item, &key.value)) {
            insert(&mut doc, &key.path, &key.value, &mut created)?;
        }
    }
    Ok(render(&doc)?)
}

/// Every owned key is present with its value.
fn owned_present(doc: &DocumentMut, keys: &[AllowanceKey]) -> bool {
    keys.iter()
        .all(|key| lookup(doc, &key.path).is_some_and(|item| matches(item, &key.value)))
}

/// Strip the owned keys (an edited one refuses; a missing one is not user data) and the
/// containers setup created once they are empty. Byte-exact when nothing else changed.
pub fn plan_remove(current: &[u8], manifest: &AllowanceManifest) -> Result<Vec<u8>, SetupError> {
    if manifest.phase == InstallPhase::Installed
        && manifest.restore_exact
        && fingerprint(current) == manifest.installed_fingerprint
    {
        return Ok(manifest.original_bytes.clone());
    }
    if manifest.phase == InstallPhase::Prepared && current == manifest.original_bytes.as_slice() {
        return Ok(current.to_vec());
    }
    let mut doc = parse(current)?;
    for key in manifest.keys.iter().filter(|key| !key.pre_existing) {
        let Some(item) = lookup(&doc, &key.path) else {
            continue;
        };
        if !matches(item, &key.value) {
            // A member removed by hand is simply gone; any other edit is the user's.
            if matches!(key.value, AllowanceValue::Member(_)) && item.as_array().is_some() {
                continue;
            }
            return Err(SetupError::Conflict);
        }
        let (last, parents) = key.path.split_last().ok_or(SetupError::Invalid)?;
        let container = container_mut(&mut doc, parents).ok_or(SetupError::Conflict)?;
        match &key.value {
            AllowanceValue::Member(member) => {
                let array = container
                    .get_mut(last)
                    .and_then(Item::as_array_mut)
                    .ok_or(SetupError::Conflict)?;
                array.retain(|value| value.as_str() != Some(member.as_str()));
            }
            _ => {
                container.remove(last);
            }
        }
    }
    for path in manifest.created.iter().rev() {
        let Some((last, parents)) = path.split_last() else {
            continue;
        };
        if let Some(table) = container_mut(&mut doc, parents)
            && table.get(last).is_some_and(|item| {
                item.as_table_like().is_some_and(TableLike::is_empty)
                    || item.as_array().is_some_and(toml_edit::Array::is_empty)
            })
        {
            table.remove(last);
        }
    }
    render(&doc)
}

fn manifest_valid(manifest: &AllowanceManifest, config: &Path) -> Result<bool, SetupError> {
    let expected = allowance_keys(&manifest.socket, &manifest.writable_roots);
    Ok(manifest.version == version_for(&manifest.writable_roots)
        && manifest.harness == "codex"
        && manifest.kind == "sandbox-allowance"
        && manifest.path == config.to_str().ok_or(SetupError::Invalid)?
        && manifest.keys.len() == expected.len()
        && manifest
            .keys
            .iter()
            .zip(&expected)
            .all(|(key, (path, value))| &key.path == path && &key.value == value)
        // A created container is always a prefix of an owned key path (or the roots array).
        && manifest.created.iter().all(|created| {
            manifest.keys.iter().any(|key| {
                (key.path.len() > created.len() && key.path.starts_with(created))
                    || (&key.path == created && matches!(key.value, AllowanceValue::Member(_)))
            })
        }))
}

pub fn read_manifest(path: &Path) -> Result<Option<AllowanceManifest>, SetupError> {
    if !path.exists() {
        return Ok(None);
    }
    if path
        .symlink_metadata()
        .map_err(|_| SetupError::Io)?
        .file_type()
        .is_symlink()
    {
        return Err(SetupError::Invalid);
    }
    let bytes = std::fs::read(path).map_err(|_| SetupError::Io)?;
    if bytes.len() > 4 * MAX_CONFIG {
        return Err(SetupError::TooLarge);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| SetupError::Invalid)
}

fn replace(path: &Path, manifest: &AllowanceManifest) -> Result<(), SetupError> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| SetupError::Invalid)?;
    write_replacement(path, &bytes, true)
}

/// Install outcome: whether this call changed the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowanceInstall {
    pub manifest: AllowanceManifest,
    pub changed: bool,
}

/// Install the allowance for `socket` and `roots` into `config` (an existing regular file),
/// recording ownership in `manifest_path` before the file is replaced. Re-running is
/// idempotent; a version-1 installation for the same socket is upgraded with the roots; an
/// installation for another socket or other roots, or an owned key edited by hand, refuses.
pub fn install(
    config: &Path,
    manifest_path: &Path,
    socket: &str,
    roots: &[String],
) -> Result<AllowanceInstall, AllowanceError> {
    if config.file_name().and_then(|s| s.to_str()) != Some("config.toml") || manifest_path == config
    {
        return Err(SetupError::Invalid.into());
    }
    let current = config_bytes(config)?;
    if let Some(mut manifest) = read_manifest(manifest_path)? {
        if !manifest_valid(&manifest, config)? || manifest.socket != socket {
            return Err(SetupError::Conflict.into());
        }
        let doc = parse(&current)?;
        if manifest.writable_roots != roots {
            // Only a settled version-1 installation (socket keys only) is upgraded.
            if !manifest.writable_roots.is_empty()
                || manifest.phase != InstallPhase::Installed
                || !owned_present(&doc, &manifest.keys)
            {
                return Err(SetupError::Conflict.into());
            }
            return upgrade(config, manifest_path, manifest, &current, roots);
        }
        if owned_present(&doc, &manifest.keys) {
            let changed = false;
            if manifest.phase == InstallPhase::Prepared {
                manifest.phase = InstallPhase::Installed;
                manifest.installed_fingerprint = fingerprint(&current);
                manifest.upgrade_base = None;
                replace(manifest_path, &manifest)?;
            }
            return Ok(AllowanceInstall { manifest, changed });
        }
        // A prepared installation (or upgrade) whose file was never replaced publishes now.
        let base = manifest
            .upgrade_base
            .clone()
            .unwrap_or_else(|| manifest.original_bytes.clone());
        if manifest.phase == InstallPhase::Prepared && current == base {
            let bytes = reapply(&current, &manifest.keys)?;
            if config_bytes(config)? != current {
                return Err(SetupError::Conflict.into());
            }
            write_replacement(config, &bytes, false)?;
            manifest.phase = InstallPhase::Installed;
            manifest.installed_fingerprint = fingerprint(&bytes);
            manifest.upgrade_base = None;
            replace(manifest_path, &manifest)?;
            return Ok(AllowanceInstall {
                manifest,
                changed: true,
            });
        }
        return Err(SetupError::Conflict.into());
    }
    let (bytes, keys, created) = plan_install(&current, socket, roots)?;
    let manifest = AllowanceManifest {
        version: version_for(roots),
        harness: "codex".into(),
        kind: "sandbox-allowance".into(),
        path: config.to_str().ok_or(SetupError::Invalid)?.into(),
        socket: socket.into(),
        writable_roots: roots.to_vec(),
        original_bytes: current.clone(),
        installed_fingerprint: fingerprint(&bytes),
        keys,
        created,
        phase: InstallPhase::Prepared,
        restore_exact: true,
        upgrade_base: None,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SetupError::Invalid)?;
    publish_manifest(manifest_path, &manifest_bytes, false)?;
    let changed = bytes != current;
    if changed {
        if config_bytes(config)? != current {
            return Err(SetupError::Conflict.into());
        }
        write_replacement(config, &bytes, false)?;
    }
    let mut installed = manifest;
    installed.phase = InstallPhase::Installed;
    replace(manifest_path, &installed)?;
    Ok(AllowanceInstall {
        manifest: installed,
        changed,
    })
}

/// Add `roots` to a settled version-1 installation: the manifest is prepared first (with the
/// file bytes the upgrade starts from), then the file is replaced, then the manifest settles.
fn upgrade(
    config: &Path,
    manifest_path: &Path,
    mut manifest: AllowanceManifest,
    current: &[u8],
    roots: &[String],
) -> Result<AllowanceInstall, AllowanceError> {
    let mut doc = parse(current)?;
    let mut created = manifest.created.clone();
    let wanted = allowance_keys(&manifest.socket, roots)
        .into_iter()
        .skip(manifest.keys.len())
        .collect();
    let added = compose(&mut doc, wanted, &mut created)?;
    let bytes = render(&doc)?;
    // `original_bytes` restores exactly only while the file is the first installation's.
    manifest.restore_exact =
        manifest.restore_exact && fingerprint(current) == manifest.installed_fingerprint;
    manifest.version = version_for(roots);
    manifest.writable_roots = roots.to_vec();
    manifest.keys.extend(added);
    manifest.created = created;
    manifest.installed_fingerprint = fingerprint(&bytes);
    manifest.phase = InstallPhase::Prepared;
    manifest.upgrade_base = Some(current.to_vec());
    replace(manifest_path, &manifest)?;
    let changed = bytes != current;
    if changed {
        if config_bytes(config)? != current {
            return Err(SetupError::Conflict.into());
        }
        write_replacement(config, &bytes, false)?;
    }
    manifest.phase = InstallPhase::Installed;
    manifest.upgrade_base = None;
    replace(manifest_path, &manifest)?;
    Ok(AllowanceInstall { manifest, changed })
}

/// What setup-status reports for the allowance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowanceInspection {
    pub recorded: Option<AllowanceManifest>,
    /// Every owned and pre-existing key is present with its value.
    pub present: bool,
}

impl AllowanceInspection {
    /// The recorded allowance is present and covers exactly `roots` as writable roots (a
    /// version-1 installation covers none: mutations from the sandbox then fail with EPERM).
    pub fn roots_present(&self, roots: &[String]) -> bool {
        self.present
            && self
                .recorded
                .as_ref()
                .is_some_and(|manifest| manifest.writable_roots == roots)
    }
}

pub fn inspect(config: &Path, manifest_path: &Path) -> Result<AllowanceInspection, SetupError> {
    let recorded = read_manifest(manifest_path)?;
    let present = match &recorded {
        Some(manifest) if manifest_valid(manifest, config)? => match config_bytes(config) {
            Ok(bytes) => owned_present(&parse(&bytes)?, &manifest.keys),
            Err(_) => false,
        },
        Some(_) => return Err(SetupError::Conflict),
        None => false,
    };
    Ok(AllowanceInspection { recorded, present })
}

/// Remove the owned allowance keys and the manifest. `Ok(false)`: nothing was recorded.
pub fn remove(config: &Path, manifest_path: &Path) -> Result<bool, SetupError> {
    let Some(manifest) = read_manifest(manifest_path)? else {
        return Ok(false);
    };
    if !manifest_valid(&manifest, config)? {
        return Err(SetupError::Conflict);
    }
    let current = match config_bytes(config) {
        Ok(bytes) => bytes,
        // The file is gone: nothing of ours is left in it.
        Err(SetupError::Io) if !config.exists() => {
            std::fs::remove_file(manifest_path).map_err(|_| SetupError::Io)?;
            return Ok(true);
        }
        Err(error) => return Err(error),
    };
    let removed = plan_remove(&current, &manifest)?;
    if config_bytes(config)? != current {
        return Err(SetupError::Conflict);
    }
    if removed != current {
        write_replacement(config, &removed, false)?;
    }
    std::fs::remove_file(manifest_path).map_err(|_| SetupError::Io)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ht-codex-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    const SOCK: &str = "/s d/instances/abc/daemon.sock";

    /// Kills: a write that loses comments or unrelated tables, an allowance under the wrong
    /// table, re-defining an existing `[features]` table (invalid TOML), or a non-exact restore.
    #[test]
    fn install_keeps_user_config_and_removal_restores_bytes() {
        let dir = tmp();
        let config = dir.join("config.toml");
        let manifest = dir.join("m.json");
        let original = "# mine\nmodel = \"x\"\n\n[features]\nhooks = true\n\n[projects.\"/p\"]\ntrust_level = \"trusted\"\n";
        std::fs::write(&config, original).unwrap();
        let done = install(&config, &manifest, SOCK, &[]).unwrap();
        assert!(done.changed);
        let text = std::fs::read_to_string(&config).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert!(text.starts_with("# mine\nmodel = \"x\"\n"), "{text}");
        assert_eq!(doc["features"]["hooks"].as_bool(), Some(true));
        assert_eq!(
            doc["features"]["network_proxy"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            doc["features"]["network_proxy"]["unix_sockets"][SOCK].as_str(),
            Some("allow")
        );
        assert_eq!(
            doc["sandbox_workspace_write"]["network_access"].as_bool(),
            Some(true)
        );
        assert_eq!(
            doc["projects"]["/p"]["trust_level"].as_str(),
            Some("trusted")
        );
        // Idempotent.
        assert!(!install(&config, &manifest, SOCK, &[]).unwrap().changed);
        assert!(inspect(&config, &manifest).unwrap().present);
        // Another socket is another installation: refused.
        assert_eq!(
            install(&config, &manifest, "/other.sock", &[]),
            Err(AllowanceError::Setup(SetupError::Conflict))
        );
        assert!(remove(&config, &manifest).unwrap());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(!manifest.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Kills: removing a key the user already had, keeping Codex's own later edits out of the
    /// result, or deleting an owned key the user changed.
    #[test]
    fn pre_existing_keys_stay_and_later_edits_survive_structural_removal() {
        let dir = tmp();
        let config = dir.join("config.toml");
        let manifest = dir.join("m.json");
        std::fs::write(
            &config,
            "[sandbox_workspace_write]\nnetwork_access = true\nwritable_roots = []\n",
        )
        .unwrap();
        let done = install(&config, &manifest, SOCK, &[]).unwrap();
        assert!(done.manifest.keys[0].pre_existing);
        assert!(!done.manifest.keys[1].pre_existing);
        // Codex records hook trust after setup.
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str(
            "\n[hooks.state.\"/h/hooks.json:session_start:0:0\"]\ntrusted_hash = \"sha256:1\"\n",
        );
        std::fs::write(&config, &text).unwrap();
        assert!(!install(&config, &manifest, SOCK, &[]).unwrap().changed);
        assert!(remove(&config, &manifest).unwrap());
        let doc: DocumentMut = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(
            doc["sandbox_workspace_write"]["network_access"].as_bool(),
            Some(true)
        );
        assert!(doc.get("features").is_none(), "{doc}");
        assert_eq!(
            doc["hooks"]["state"]["/h/hooks.json:session_start:0:0"]["trusted_hash"].as_str(),
            Some("sha256:1")
        );
        // An edited owned key refuses removal.
        install(&config, &manifest, SOCK, &[]).unwrap();
        let edited = std::fs::read_to_string(&config)
            .unwrap()
            .replace("enabled = true", "enabled = false");
        std::fs::write(&config, &edited).unwrap();
        assert_eq!(remove(&config, &manifest), Err(SetupError::Conflict));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), edited);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Kills: overwriting a user's conflicting value, or editing an unparsable file.
    #[test]
    fn foreign_values_and_invalid_files_refuse_without_writing() {
        let dir = tmp();
        let config = dir.join("config.toml");
        let manifest = dir.join("m.json");
        for (text, want) in [
            (
                "[sandbox_workspace_write]\nnetwork_access = false\n",
                AllowanceError::Foreign("sandbox_workspace_write.network_access".into()),
            ),
            ("features = 1\n", AllowanceError::Foreign("features".into())),
            ("[[x]\n", AllowanceError::Setup(SetupError::Invalid)),
        ] {
            std::fs::write(&config, text).unwrap();
            assert_eq!(install(&config, &manifest, SOCK, &[]), Err(want), "{text}");
            assert_eq!(std::fs::read_to_string(&config).unwrap(), text);
            assert!(!manifest.exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Kills: an allowance that cannot be read back by a TOML parser when the user configures
    /// features through dotted keys or an inline table.
    #[test]
    fn dotted_and_inline_feature_tables_compose_valid_toml() {
        for text in [
            "features.hooks = true\n",
            "features = { hooks = true }\n",
            "[features]\nnetwork_proxy.enabled = true\n",
        ] {
            let (bytes, keys, _) = plan_install(text.as_bytes(), SOCK, &[]).unwrap();
            let doc: DocumentMut = String::from_utf8(bytes).unwrap().parse().unwrap();
            assert_eq!(
                doc["features"]["network_proxy"]["unix_sockets"][SOCK].as_str(),
                Some("allow"),
                "{text}"
            );
            assert_eq!(keys.len(), 3);
        }
    }

    fn roots() -> Vec<String> {
        vec![
            "/s d/instances/abc/intents".to_owned(),
            "/s d/instances/abc/contexts".to_owned(),
        ]
    }

    fn members(doc: &DocumentMut) -> Vec<String> {
        doc["sandbox_workspace_write"]["writable_roots"]
            .as_array()
            .map(|array| {
                array
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Kills: an allowance without the client journal roots (sandboxed mutations then fail
    /// with EPERM), a root wider than the two journal directories, or a removal that leaves
    /// the created array behind or loses the user's own roots.
    #[test]
    fn writable_roots_are_owned_array_members() {
        let dir = tmp();
        let config = dir.join("config.toml");
        let manifest = dir.join("m.json");
        std::fs::write(&config, "model = \"x\"\n").unwrap();
        let done = install(&config, &manifest, SOCK, &roots()).unwrap();
        assert_eq!(done.manifest.version, 2);
        let doc: DocumentMut = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(members(&doc), roots());
        assert!(inspect(&config, &manifest).unwrap().roots_present(&roots()));
        assert!(!install(&config, &manifest, SOCK, &roots()).unwrap().changed);
        assert!(remove(&config, &manifest).unwrap());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "model = \"x\"\n");

        // The user's own roots array: ours are appended, a shared member is pre-existing and
        // stays, theirs stay, the array stays.
        let mine = "[sandbox_workspace_write]\nwritable_roots = [\"/mine\", \"/s d/instances/abc/contexts\"]\n";
        std::fs::write(&config, mine).unwrap();
        let done = install(&config, &manifest, SOCK, &roots()).unwrap();
        assert!(!done.manifest.keys[3].pre_existing);
        assert!(done.manifest.keys[4].pre_existing);
        let doc: DocumentMut = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(
            members(&doc),
            [
                "/mine",
                "/s d/instances/abc/contexts",
                "/s d/instances/abc/intents"
            ]
        );
        // Codex edits the file later: removal is structural.
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("\n[hooks.state.\"k\"]\ntrusted_hash = \"sha256:1\"\n");
        std::fs::write(&config, &text).unwrap();
        assert!(remove(&config, &manifest).unwrap());
        let doc: DocumentMut = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(members(&doc), ["/mine", "/s d/instances/abc/contexts"]);
        assert!(
            doc["sandbox_workspace_write"]
                .get("network_access")
                .is_none()
        );
        assert_eq!(
            doc["hooks"]["state"]["k"]["trusted_hash"].as_str(),
            Some("sha256:1")
        );

        // A non-array writable_roots is the user's: refused untouched.
        std::fs::write(
            &config,
            "[sandbox_workspace_write]\nwritable_roots = \"/x\"\n",
        )
        .unwrap();
        assert_eq!(
            install(&config, &manifest, SOCK, &roots()),
            Err(AllowanceError::Foreign(
                "sandbox_workspace_write.writable_roots".into()
            ))
        );
        assert!(!manifest.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Kills: an existing version-1 installation that refuses (or silently keeps lacking the
    /// roots) on re-setup, an upgrade that marks the earlier socket keys pre-existing (so
    /// unsetup would leave network_access behind), or an exact restore that drops Codex's
    /// edits made between the first setup and the upgrade.
    #[test]
    fn version_one_installation_upgrades_with_roots() {
        let dir = tmp();
        let config = dir.join("config.toml");
        let manifest = dir.join("m.json");
        let original = "model = \"x\"\n";
        std::fs::write(&config, original).unwrap();
        install(&config, &manifest, SOCK, &[]).unwrap();
        let v1 = inspect(&config, &manifest).unwrap();
        assert!(v1.present && !v1.roots_present(&roots()));
        // Codex records trust after the first setup.
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("\n[hooks.state.\"k\"]\ntrusted_hash = \"sha256:1\"\n");
        std::fs::write(&config, &text).unwrap();
        let done = install(&config, &manifest, SOCK, &roots()).unwrap();
        assert!(done.changed);
        assert_eq!(done.manifest.version, 2);
        assert!(done.manifest.keys.iter().all(|key| !key.pre_existing));
        assert!(!done.manifest.restore_exact);
        assert!(inspect(&config, &manifest).unwrap().roots_present(&roots()));
        assert!(!install(&config, &manifest, SOCK, &roots()).unwrap().changed);
        assert!(remove(&config, &manifest).unwrap());
        let doc: DocumentMut = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert!(doc.get("sandbox_workspace_write").is_none(), "{doc}");
        assert!(doc.get("features").is_none(), "{doc}");
        assert_eq!(
            doc["hooks"]["state"]["k"]["trusted_hash"].as_str(),
            Some("sha256:1")
        );

        // Untouched since the first setup: the upgrade still restores byte for byte.
        std::fs::write(&config, original).unwrap();
        install(&config, &manifest, SOCK, &[]).unwrap();
        assert!(
            install(&config, &manifest, SOCK, &roots())
                .unwrap()
                .manifest
                .restore_exact
        );
        assert!(remove(&config, &manifest).unwrap());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Kills: a crash between the prepared upgrade manifest and the file replacement that
    /// leaves setup refusing forever, or removal refusing over the never-written members.
    #[test]
    fn prepared_upgrade_resumes_and_removes() {
        let dir = tmp();
        let config = dir.join("config.toml");
        let manifest = dir.join("m.json");
        std::fs::write(&config, "").unwrap();
        install(&config, &manifest, SOCK, &[]).unwrap();
        let before = std::fs::read(&config).unwrap();
        let saved = std::fs::read(&manifest).unwrap();
        install(&config, &manifest, SOCK, &roots()).unwrap();
        // Simulate the crash: prepared manifest, file not yet replaced.
        let mut prepared: AllowanceManifest =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        prepared.phase = InstallPhase::Prepared;
        prepared.upgrade_base = Some(before.clone());
        std::fs::write(&manifest, serde_json::to_vec(&prepared).unwrap()).unwrap();
        std::fs::write(&config, &before).unwrap();
        // Removal from that state strips the socket keys and skips the absent members.
        assert!(remove(&config, &manifest).unwrap());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
        // Resume: the prepared upgrade publishes.
        std::fs::write(&config, &before).unwrap();
        std::fs::write(&manifest, &saved).unwrap();
        install(&config, &manifest, SOCK, &roots()).unwrap();
        std::fs::write(&manifest, serde_json::to_vec(&prepared).unwrap()).unwrap();
        std::fs::write(&config, &before).unwrap();
        let resumed = install(&config, &manifest, SOCK, &roots()).unwrap();
        assert!(resumed.changed);
        assert_eq!(resumed.manifest.phase, InstallPhase::Installed);
        assert!(inspect(&config, &manifest).unwrap().roots_present(&roots()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
