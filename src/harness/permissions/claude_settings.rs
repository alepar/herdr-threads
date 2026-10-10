//! The Claude permission component: exact owned `permissions.allow` / `permissions.ask` rules in
//! the user's `settings.json`, recorded in a manifest of their own, independent of the hook
//! installation.
//!
//! Every publication first records its intent (the target component plus the settings
//! fingerprints before and after) in the manifest, then replaces the settings file through the
//! backed user-config writer, then finishes: a historical broad rule an earlier hook setup
//! recorded has its record retired, and the intent is cleared. An interrupted publication is
//! settled by the next operation: settings still `before` restore the previous component,
//! settings already `after` finish, anything else refuses.
//!
//! Without consent the component only keeps or narrows what it owns. An owned historical
//! `Bash(herdr-threads *)` grant becomes the `herdr-threads` rules (the same allow plus the human
//! and escalating ask rules) automatically; the `ht` rules need consent. A rule the user already
//! held is recorded as pre-existing and never removed.
use super::{
    OwnedConfigWriteGuard, OwnedPermissionResource, PERMISSION_MANIFEST_VERSION,
    PendingPublication, PermissionBackend, PermissionComponentManifest, PermissionConsent,
    VerifiedConfigIdentity, claude::render,
};
use crate::harness::setup::{
    self, RecordedPermission, SetupError, config_bytes, recorded_hook_permission,
    retire_hook_permission, write_replacement, write_user_config,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const LISTS: [&str; 2] = ["allow", "ask"];
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the component stands, as `setup-status` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudePermissionState {
    /// Nothing is owned or recorded.
    Missing,
    /// The component owns (or records as pre-existing) its rules.
    Installed,
    /// An earlier hook setup recorded an owned broad rule; the next setup narrows it.
    Historical,
    /// A publication was interrupted; the next setup or unsetup settles it.
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudePermissionStatus {
    pub state: ClaudePermissionState,
    /// Recorded rules present now, per list, with whether each was pre-existing.
    pub allow: Vec<(String, bool)>,
    pub ask: Vec<(String, bool)>,
    /// Owned historical rules still present (state `Historical`).
    pub historical: Vec<String>,
    /// Broad `herdr-threads` / `ht` allow rules nobody here owns: they still grant everything.
    pub foreign_broad: Vec<String>,
}

pub struct ClaudePermissions {
    settings: PathBuf,
    manifest: PathBuf,
    hooks: PathBuf,
}

fn rule_fingerprint(list: &str, rule: &str) -> String {
    hex(&serde_json::to_vec(&json!({ list: rule })).expect("string serialization"))
}
fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn resource_list(resource: &OwnedPermissionResource) -> Option<&'static str> {
    let rule = resource.rule.as_deref()?;
    LISTS
        .into_iter()
        .find(|list| resource.fingerprint == rule_fingerprint(list, rule))
}

/// One `permissions.<list>` array; absent is empty. A non-object `permissions` or non-array
/// list is not a settings file the component can compose.
fn native<'a>(value: &'a Value, list: &str) -> Result<Vec<&'a str>, SetupError> {
    let Some(permissions) = value.get("permissions") else {
        return Ok(Vec::new());
    };
    let permissions = permissions.as_object().ok_or(SetupError::Invalid)?;
    match permissions.get(list) {
        None => Ok(Vec::new()),
        Some(rules) => Ok(rules
            .as_array()
            .ok_or(SetupError::Invalid)?
            .iter()
            .filter_map(Value::as_str)
            .collect()),
    }
}
fn present(value: &Value, list: &str, rule: &str) -> Result<bool, SetupError> {
    Ok(native(value, list)?.contains(&rule))
}
/// Remove one occurrence of `rule` from `permissions.<list>`, if present.
fn remove_one(value: &mut Value, list: &str, rule: &str) {
    if let Some(rules) = value
        .get_mut("permissions")
        .and_then(|p| p.get_mut(list))
        .and_then(Value::as_array_mut)
        && let Some(index) = rules.iter().position(|r| r.as_str() == Some(rule))
    {
        rules.remove(index);
    }
}
/// Append `rule` to `permissions.<list>`, recording each container this creates.
fn append(
    value: &mut Value,
    list: &str,
    rule: &str,
    created: &mut BTreeSet<String>,
) -> Result<(), SetupError> {
    let map = value.as_object_mut().ok_or(SetupError::Invalid)?;
    if !map.contains_key("permissions") {
        created.insert("permissions".into());
    }
    let permissions = map
        .entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(SetupError::Invalid)?;
    if !permissions.contains_key(list) {
        created.insert(format!("permissions.{list}"));
    }
    permissions
        .entry(list)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or(SetupError::Invalid)?
        .push(json!(rule));
    Ok(())
}
/// Drop each container the component created that is now empty; the rest stay recorded.
fn prune(value: &mut Value, created: &mut BTreeSet<String>) {
    for list in LISTS {
        let name = format!("permissions.{list}");
        if created.contains(&name)
            && let Some(permissions) = value.get_mut("permissions").and_then(Value::as_object_mut)
            && permissions
                .get(list)
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        {
            permissions.remove(list);
            created.remove(&name);
        }
    }
    if created.contains("permissions")
        && value
            .get("permissions")
            .and_then(Value::as_object)
            .is_some_and(serde_json::Map::is_empty)
        && let Some(map) = value.as_object_mut()
    {
        map.remove("permissions");
        created.remove("permissions");
    }
}
fn covers_spelling(rule: &str, spelling: &str) -> bool {
    rule.strip_prefix("Bash(")
        .and_then(|rest| rest.strip_prefix(spelling))
        .is_some_and(|rest| rest == ")" || rest.starts_with(' '))
}

#[cfg(test)]
thread_local! {
    static FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) };
}
/// Test-only interruption points of one publication.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    AfterIntent,
    AfterSettings,
}
#[cfg(test)]
pub(crate) fn arm(fault: Option<Fault>) {
    FAULT.with(|f| f.set(fault));
}
fn interrupt(_at: &'static str) -> Result<(), SetupError> {
    #[cfg(test)]
    {
        let hit = FAULT.with(|f| match (f.get(), _at) {
            (Some(Fault::AfterIntent), "intent") | (Some(Fault::AfterSettings), "settings") => {
                f.set(None);
                true
            }
            _ => false,
        });
        if hit {
            return Err(SetupError::Io);
        }
    }
    Ok(())
}

impl ClaudePermissions {
    /// `settings` is the user's Claude `settings.json`; `manifest` this component's private
    /// manifest; `hooks` the hook installation's manifest, which may hold a historical record.
    pub fn new(settings: PathBuf, manifest: PathBuf, hooks: PathBuf) -> Self {
        Self {
            settings,
            manifest,
            hooks,
        }
    }

    pub fn manifest(&self) -> &Path {
        &self.manifest
    }

    /// The settings file as recorded in the manifest: under the canonical config directory.
    fn recorded_file(&self) -> Result<PathBuf, SetupError> {
        let parent = self.settings.parent().ok_or(SetupError::Invalid)?;
        let name = self.settings.file_name().ok_or(SetupError::Invalid)?;
        Ok(parent
            .canonicalize()
            .map_err(|_| SetupError::Io)?
            .join(name))
    }

    fn current(&self) -> Result<Option<Vec<u8>>, SetupError> {
        match self.settings.symlink_metadata() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            _ => config_bytes(&self.settings).map(Some),
        }
    }

    fn read(&self) -> Result<Option<PermissionComponentManifest>, SetupError> {
        let bytes = match fs::read(&self.manifest) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SetupError::Io),
            Ok(bytes) => bytes,
        };
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(SetupError::TooLarge);
        }
        let manifest: PermissionComponentManifest =
            serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
        let file = self.recorded_file()?;
        // A manifest can never direct removal of a rule it does not fingerprint.
        if manifest.version != PERMISSION_MANIFEST_VERSION
            || manifest.backend != PermissionBackend::Claude
            || manifest
                .resources
                .iter()
                .any(|r| r.file != file || resource_list(r).is_none())
        {
            return Err(SetupError::Conflict);
        }
        Ok(Some(manifest))
    }

    fn write(&self, manifest: &PermissionComponentManifest) -> Result<(), SetupError> {
        let bytes = serde_json::to_vec(manifest).map_err(|_| SetupError::Invalid)?;
        write_replacement(&self.manifest, &bytes, true)
    }

    fn remove_manifest(&self) -> Result<(), SetupError> {
        match fs::remove_file(&self.manifest) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(SetupError::Io),
            _ => Ok(()),
        }
    }

    fn lock(&self) -> Result<OwnedConfigWriteGuard, SetupError> {
        let root = self
            .recorded_file()?
            .parent()
            .ok_or(SetupError::Invalid)?
            .to_path_buf();
        let identity = VerifiedConfigIdentity::verify(&root, PermissionBackend::Claude)
            .map_err(|_| SetupError::Invalid)?;
        OwnedConfigWriteGuard::acquire(&identity, LOCK_TIMEOUT).map_err(|_| SetupError::Io)
    }

    /// Read-only: what the component owns and what an earlier hook setup recorded.
    pub fn status(&self) -> Result<ClaudePermissionStatus, SetupError> {
        let manifest = self.read()?;
        let hook = recorded_hook_permission(&self.settings, &self.hooks)?;
        let settings = self
            .current()
            .map(|bytes| bytes.unwrap_or_else(|| b"{}".to_vec()));
        let value = if manifest.is_none() && hook.is_none() {
            // Nothing recorded: an unreadable file is the hook installer's to report.
            settings
                .and_then(|bytes| setup::root(&bytes))
                .unwrap_or_else(|_| json!({}))
        } else {
            setup::root(&settings?)?
        };
        let mut status = ClaudePermissionStatus {
            state: ClaudePermissionState::Missing,
            allow: Vec::new(),
            ask: Vec::new(),
            historical: Vec::new(),
            foreign_broad: Vec::new(),
        };
        let mut owned_broad = BTreeSet::new();
        if let Some(manifest) = &manifest {
            status.state = if manifest.pending.is_some() {
                ClaudePermissionState::Pending
            } else {
                ClaudePermissionState::Installed
            };
            for resource in &manifest.resources {
                let (Some(list), Some(rule)) = (resource_list(resource), &resource.rule) else {
                    continue;
                };
                if present(&value, list, rule)? {
                    let entry = (rule.clone(), resource.pre_existing);
                    if list == "allow" {
                        status.allow.push(entry);
                    } else {
                        status.ask.push(entry);
                    }
                }
            }
        } else if let Some(hook) = &hook {
            for rule in &hook.owned {
                if present(&value, "allow", rule)? {
                    status.historical.push(rule.clone());
                    owned_broad.insert(rule.clone());
                }
            }
            if !status.historical.is_empty() {
                status.state = ClaudePermissionState::Historical;
            }
        }
        status.foreign_broad = native(&value, "allow")?
            .into_iter()
            .filter(|rule| {
                ["Bash(herdr-threads *)", "Bash(ht *)", "Bash(*)"].contains(rule)
                    && !owned_broad.contains(*rule)
            })
            .map(str::to_owned)
            .collect();
        Ok(status)
    }

    /// Settle an interrupted publication under `_guard`: finish one whose settings were
    /// published, restore the previous component when they were not, refuse anything else.
    fn settle(&self, _guard: &OwnedConfigWriteGuard) -> Result<(), SetupError> {
        let Some(mut manifest) = self.read()? else {
            return Ok(());
        };
        let Some(pending) = manifest.pending.take() else {
            return Ok(());
        };
        let current = hex(&self.current()?.ok_or(SetupError::Conflict)?);
        if current == pending.after {
            if let Some(retired) = &pending.retired {
                match recorded_hook_permission(&self.settings, &self.hooks)? {
                    None => {}
                    Some(record) if record.owned == *retired => {
                        retire_hook_permission(&self.hooks, &record)?
                    }
                    Some(_) => return Err(SetupError::Conflict),
                }
            }
            if manifest.resources.is_empty() && manifest.created.is_empty() {
                self.remove_manifest()
            } else {
                self.write(&manifest)
            }
        } else if current == pending.before {
            match pending.previous {
                Some(previous) => self.write(&previous),
                None => self.remove_manifest(),
            }
        } else {
            Err(SetupError::Conflict)
        }
    }

    /// Bring the owned rules to the rendered set as far as `consent` allows (see the module
    /// docs). Without consent and with nothing owned or recorded, nothing is touched.
    pub fn apply(&self, consent: PermissionConsent) -> Result<ClaudePermissionStatus, SetupError> {
        if consent != PermissionConsent::Granted
            && self.read()?.is_none()
            && recorded_hook_permission(&self.settings, &self.hooks)?.is_none()
        {
            return self.status();
        }
        let guard = self.lock()?;
        self.settle(&guard)?;
        let manifest = self.read()?;
        let hook = recorded_hook_permission(&self.settings, &self.hooks)?;
        if manifest.is_some() && hook.is_some() {
            // Two owners of one grant: never guess which one to trust.
            return Err(SetupError::Conflict);
        }
        let current = self.current()?.ok_or(SetupError::Conflict)?;
        let mut value = setup::root(&current)?;
        let rendered = render();
        let file = self.recorded_file()?;
        let mut target: Vec<(&str, String)> = LISTS
            .into_iter()
            .zip([rendered.allow, rendered.ask])
            .flat_map(|(list, rules)| rules.into_iter().map(move |rule| (list, rule)))
            .collect();
        if consent != PermissionConsent::Granted {
            match (&manifest, &hook) {
                // An existing component keeps or narrows exactly what is still there.
                (Some(manifest), _) => {
                    let kept: BTreeSet<(&str, &str)> = manifest
                        .resources
                        .iter()
                        .filter_map(|r| Some((resource_list(r)?, r.rule.as_deref()?)))
                        .filter(|(list, rule)| present(&value, list, rule).unwrap_or(false))
                        .collect();
                    target.retain(|(list, rule)| kept.contains(&(*list, rule.as_str())));
                }
                // The historical broad grant keeps exactly the spelling it already covered.
                (None, Some(hook)) if !hook.owned.is_empty() => {
                    target.retain(|(_, rule)| covers_spelling(rule, "herdr-threads"));
                }
                _ => target.clear(),
            }
        }
        let mut created: BTreeSet<String> = match (&manifest, &hook) {
            (Some(manifest), _) => manifest.created.iter().cloned().collect(),
            (
                None,
                Some(RecordedPermission {
                    created_permissions,
                    created_allow,
                    ..
                }),
            ) => [
                created_permissions.then(|| "permissions".to_owned()),
                created_allow.then(|| "permissions.allow".to_owned()),
            ]
            .into_iter()
            .flatten()
            .collect(),
            _ => BTreeSet::new(),
        };
        // Retire what is no longer wanted: owned rules outside the target, historical rules.
        let wanted: BTreeSet<(&str, &str)> = target
            .iter()
            .map(|(list, rule)| (*list, rule.as_str()))
            .collect();
        if let Some(manifest) = &manifest {
            for resource in manifest.resources.iter().filter(|r| !r.pre_existing) {
                let (Some(list), Some(rule)) = (resource_list(resource), &resource.rule) else {
                    continue;
                };
                if !wanted.contains(&(list, rule.as_str())) {
                    remove_one(&mut value, list, rule);
                }
            }
        }
        // A historical rule the target keeps changes owner in place; any other is removed.
        for rule in hook.iter().flat_map(|h| &h.owned) {
            if !wanted.contains(&("allow", rule.as_str())) {
                remove_one(&mut value, "allow", rule);
            }
        }
        let owned: BTreeSet<(&str, &str)> = manifest
            .iter()
            .flat_map(|m| &m.resources)
            .filter(|r| !r.pre_existing)
            .filter_map(|r| Some((resource_list(r)?, r.rule.as_deref()?)))
            .chain(
                hook.iter()
                    .flat_map(|h| &h.owned)
                    .map(|r| ("allow", r.as_str())),
            )
            .collect();
        let mut resources = Vec::new();
        for (list, rule) in &target {
            let held = present(&value, list, rule)?;
            // A rule already there that this component did not add is the user's own.
            let pre_existing = held && !owned.contains(&(*list, rule.as_str()));
            if !held {
                append(&mut value, list, rule, &mut created)?;
            }
            resources.push(OwnedPermissionResource {
                file: file.clone(),
                rule: Some(rule.clone()),
                fingerprint: rule_fingerprint(list, rule),
                pre_existing,
            });
        }
        prune(&mut value, &mut created);
        let after = if value == setup::root(&current)? {
            current.clone()
        } else {
            serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)?
        };
        if after.len() > 1_048_576 {
            return Err(SetupError::TooLarge);
        }
        let desired = PermissionComponentManifest {
            version: PERMISSION_MANIFEST_VERSION,
            backend: PermissionBackend::Claude,
            config_root: file.parent().ok_or(SetupError::Invalid)?.into(),
            resources,
            created: created.into_iter().collect(),
            pending: None,
        };
        if after == current && manifest.as_ref() == Some(&desired) && hook.is_none() {
            drop(guard);
            return self.status();
        }
        self.publish(&guard, &current, &after, desired, manifest, hook)?;
        drop(guard);
        self.status()
    }

    /// Remove every rule the component added; pre-existing rules stay. A historical hook
    /// record is left to the hook removal, which strips its owned rule.
    pub fn remove(&self) -> Result<ClaudePermissionStatus, SetupError> {
        if self.read()?.is_none() {
            return self.status();
        }
        let guard = self.lock()?;
        self.settle(&guard)?;
        let Some(manifest) = self.read()? else {
            drop(guard);
            return self.status();
        };
        let current = self.current()?.ok_or(SetupError::Conflict)?;
        let mut value = setup::root(&current)?;
        for resource in manifest.resources.iter().filter(|r| !r.pre_existing) {
            if let (Some(list), Some(rule)) = (resource_list(resource), &resource.rule) {
                remove_one(&mut value, list, rule);
            }
        }
        let mut created: BTreeSet<String> = manifest.created.iter().cloned().collect();
        prune(&mut value, &mut created);
        let after = if value == setup::root(&current)? {
            current.clone()
        } else {
            serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)?
        };
        let desired = PermissionComponentManifest {
            resources: Vec::new(),
            created: Vec::new(),
            pending: None,
            ..manifest.clone()
        };
        self.publish(&guard, &current, &after, desired, Some(manifest), None)?;
        drop(guard);
        self.status()
    }

    fn publish(
        &self,
        guard: &OwnedConfigWriteGuard,
        current: &[u8],
        after: &[u8],
        desired: PermissionComponentManifest,
        previous: Option<PermissionComponentManifest>,
        hook: Option<RecordedPermission>,
    ) -> Result<(), SetupError> {
        let mut intent = desired;
        intent.pending = Some(PendingPublication {
            before: hex(current),
            after: hex(after),
            previous: previous.map(Box::new),
            retired: hook.map(|h| h.owned),
        });
        self.write(&intent)?;
        interrupt("intent")?;
        write_user_config(&self.settings, current, after)?;
        interrupt("settings")?;
        self.settle(guard)
    }
}

#[cfg(test)]
#[path = "../../../tests/harness/claude_permissions.rs"]
mod tests;
