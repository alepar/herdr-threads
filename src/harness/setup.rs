//! Scoped hook proposals and explicit user-settings installation (Claude `settings.json`, Codex
//! `hooks.json`). No process launch or trust change.
use super::{codex, launch::compose_native_argv};
use crate::ports::ConfiguredHook;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{borrow::Cow, fs, io::Write, path::Path};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    Invalid,
    TooLarge,
    Conflict,
    Io,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedEntry {
    pub event: String,
    pub group: Value,
    pub fingerprint: String,
}
/// The owned permission allow rule of a Claude project installation. Tracked like an owned hook
/// group: fingerprinted, removed exactly by unsetup. A rule the user's settings already held
/// before installation is recorded as `pre_existing` and is never added or removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedPermission {
    pub rule: String,
    pub fingerprint: String,
    pub pre_existing: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonSetupPlan {
    pub proposed_bytes: Vec<u8>,
    pub base_fingerprint: String,
    pub owned: Vec<OwnedEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventGroups {
    pub event: String,
    pub groups: Vec<Value>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSetupPlan {
    pub base_fingerprint: String,
    pub events: Vec<EventGroups>,
    /// The owned session overrides for a launch with no caller arguments.
    pub launch_argv: Vec<String>,
    /// The session `-c hooks.*=...` overrides alone.
    pub session_config: Vec<String>,
    pub owned: Vec<OwnedEntry>,
}

impl CodexSetupPlan {
    /// Compose owned overrides with caller arguments using the launch rule.
    pub fn launch_argv_for(&self, caller: Vec<String>) -> Result<Vec<String>, SetupError> {
        compose_native_argv(
            crate::protocol::authority::Harness::Codex,
            caller,
            self.session_config.clone(),
        )
        .map_err(|_| SetupError::Invalid)
    }
}
/// Quote only at the command-hook shell boundary; native launch remains an argv vector.
pub fn shell_command(argv: &[String]) -> Result<String, SetupError> {
    if argv.is_empty() || argv.len() > 32 {
        return Err(SetupError::Invalid);
    }
    let mut words = Vec::new();
    for word in argv {
        if word.is_empty() || word.len() > 4096 || word.chars().any(char::is_control) {
            return Err(SetupError::Invalid);
        }
        words.push(format!("'{}'", word.replace('\'', "'\\''")));
    }
    Ok(words.join(" "))
}
/// The owner marker suffix a marked hook command carries.
const OWNER_MARKER: &str = " # herdr-threads-owner:";

/// The hook command registered for `event`: the base command with the two words
/// `'--event' '<event>'` inserted after the harness word, which is before the owner
/// marker when there is one (else appended).
pub(crate) fn event_command(base: &str, event: &str) -> String {
    let pair = format!(" '--event' '{event}'");
    match base.find(OWNER_MARKER) {
        Some(at) => format!("{}{pair}{}", &base[..at], &base[at..]),
        None => format!("{base}{pair}"),
    }
}

/// Inverse of [`event_command`]: the base command without its `--event <event>` pair. A command
/// that does not carry the pair (a legacy registration) is returned unchanged.
pub(crate) fn base_command<'a>(command: &'a str, event: &str) -> Cow<'a, str> {
    let pair = format!(" '--event' '{event}'");
    let end = command.find(OWNER_MARKER).unwrap_or(command.len());
    match command[..end].strip_suffix(pair.as_str()) {
        Some(head) => Cow::Owned(format!("{head}{}", &command[end..])),
        None => Cow::Borrowed(command),
    }
}

/// The user-level hook file an owned installation edits. Both are JSON objects whose `hooks` map
/// event names to arrays of matcher groups; only Claude's also carries the owned allow rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsKind {
    /// `$CLAUDE_CONFIG_DIR/settings.json` (default `~/.claude/settings.json`).
    ClaudeUser,
    /// `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`).
    CodexUser,
}
impl SettingsKind {
    pub fn harness(self) -> &'static str {
        match self {
            Self::ClaudeUser => "claude",
            Self::CodexUser => "codex",
        }
    }
    pub fn scope(self) -> &'static str {
        "user"
    }
    /// The only file name an installation of this kind may edit.
    pub fn file_name(self) -> &'static str {
        match self {
            Self::ClaudeUser => "settings.json",
            Self::CodexUser => "hooks.json",
        }
    }
    /// Whether the installation owns the permission allow rule.
    fn manages_permission(self) -> bool {
        self == Self::ClaudeUser
    }
    /// Owned entries derived from the adapter declaration, in declaration order.
    fn entries(self, command: &str) -> Result<Vec<OwnedEntry>, SetupError> {
        match self {
            Self::ClaudeUser => claude_entries(command),
            Self::CodexUser => codex_entries(command),
        }
    }
}

pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn permission_fingerprint(rule: &str) -> String {
    fingerprint(&serde_json::to_vec(&json!({ "allow": rule })).unwrap_or_default())
}
/// The allow rule current setup installs.
const DECLARED_RULE: &str = super::claude::HERDR_THREADS_ALLOW_RULE;
/// The allow rule earlier setups installed; recognized in their manifests only, so re-setup can
/// replace and unsetup can remove an owned copy.
const RETIRED_RULE: &str = super::claude::CALLER_CONTEXT_ALLOW_RULE;
/// A recorded permission is owned only for a rule this adapter declares or once declared, with a
/// matching fingerprint: a manifest can never direct removal of an arbitrary user rule.
fn permission_valid(permission: &OwnedPermission) -> bool {
    (permission.rule == DECLARED_RULE || permission.rule == RETIRED_RULE)
        && permission.fingerprint == permission_fingerprint(&permission.rule)
}
fn permission_current(permission: &OwnedPermission) -> bool {
    permission.rule == DECLARED_RULE
}
/// The retired owned rule an in-flight (prepared) upgrade removes, if any.
fn superseded_rule(manifest: &OwnershipManifest) -> Option<&str> {
    manifest
        .superseded_permission
        .as_ref()
        .map(|p| p.rule.as_str())
}
/// The rule this installation added and therefore removes: absent for a pre-existing user rule.
fn owned_rule(manifest: &OwnershipManifest) -> Option<&str> {
    manifest
        .permission
        .as_ref()
        .filter(|p| !p.pre_existing)
        .map(|p| p.rule.as_str())
}
/// Occurrences of `rule` in `permissions.allow`. A non-object `permissions` or non-array
/// `allow` is not a settings file this setup can compose.
fn allow_count(value: &Value, rule: &str) -> Result<usize, SetupError> {
    let Some(permissions) = value.get("permissions") else {
        return Ok(0);
    };
    let permissions = permissions.as_object().ok_or(SetupError::Invalid)?;
    let Some(allow) = permissions.get("allow") else {
        return Ok(0);
    };
    Ok(allow
        .as_array()
        .ok_or(SetupError::Invalid)?
        .iter()
        .filter(|entry| entry.as_str() == Some(rule))
        .count())
}
/// Append `rule` to `permissions.allow` unless an identical rule is already there.
fn ensure_allow(value: &mut Value, rule: &str) -> Result<(), SetupError> {
    if allow_count(value, rule)? > 0 {
        return Ok(());
    }
    let map = value.as_object_mut().ok_or(SetupError::Invalid)?;
    map.entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(SetupError::Invalid)?
        .entry("allow")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or(SetupError::Invalid)?
        .push(json!(rule));
    Ok(())
}
/// Remove one occurrence of the owned `rule` (none present is not an error: the rule is not user
/// data). An `allow` array or `permissions` object left empty is dropped only when the recorded
/// base did not hold it; a legacy empty or unparsable base proves nothing, so nothing is pruned.
fn remove_allow(bytes: &[u8], rule: &str, recorded_base: &[u8]) -> Result<Vec<u8>, SetupError> {
    let mut value = root(bytes)?;
    if allow_count(&value, rule)? == 0 {
        return Ok(bytes.to_vec());
    }
    let base = root(recorded_base).ok();
    let base_held = |key: &str| {
        base.as_ref().is_none_or(|base| match key {
            "permissions" => base.get("permissions").is_some(),
            _ => base["permissions"].get("allow").is_some(),
        })
    };
    let permissions = value["permissions"]
        .as_object_mut()
        .ok_or(SetupError::Invalid)?;
    let allow = permissions["allow"]
        .as_array_mut()
        .ok_or(SetupError::Invalid)?;
    let index = allow
        .iter()
        .position(|entry| entry.as_str() == Some(rule))
        .ok_or(SetupError::Invalid)?;
    allow.remove(index);
    if allow.is_empty() && !base_held("allow") {
        permissions.remove("allow");
    }
    if permissions.is_empty() && !base_held("permissions") {
        value
            .as_object_mut()
            .ok_or(SetupError::Invalid)?
            .remove("permissions");
    }
    serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)
}
/// Strip exact owned hook groups and the given owned allow rules (each one occurrence, if present).
fn uninstall_claude(
    bytes: &[u8],
    entries: &[OwnedEntry],
    rules: &[&str],
    recorded_base: &[u8],
) -> Result<Vec<u8>, SetupError> {
    let mut stripped = uninstall_json(bytes, entries)?;
    for rule in rules {
        stripped = remove_allow(&stripped, rule, recorded_base)?;
    }
    Ok(stripped)
}
fn owned(event: &str, group: Value) -> Result<OwnedEntry, SetupError> {
    let bytes = serde_json::to_vec(&group).map_err(|_| SetupError::Invalid)?;
    Ok(OwnedEntry {
        event: event.into(),
        group,
        fingerprint: fingerprint(&bytes),
    })
}
pub(crate) fn root(bytes: &[u8]) -> Result<Value, SetupError> {
    if bytes.len() > 1_048_576 {
        return Err(SetupError::TooLarge);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| SetupError::Invalid)?;
    if !v.is_object() {
        return Err(SetupError::Invalid);
    }
    Ok(v)
}
fn contains_command(group: &Value, command: &str) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks
                .iter()
                .any(|hook| hook.get("command").and_then(Value::as_str) == Some(command))
        })
}
fn merge(groups: &mut Vec<Value>, entry: &OwnedEntry) -> Result<(), SetupError> {
    let command = entry.group["hooks"][0]["command"]
        .as_str()
        .ok_or(SetupError::Invalid)?;
    let matches: Vec<_> = groups
        .iter()
        .filter(|g| contains_command(g, command))
        .collect();
    if matches.is_empty() {
        groups.push(entry.group.clone());
        return Ok(());
    }
    if matches.len() == 1 && matches[0] == &entry.group {
        return Ok(());
    }
    Err(SetupError::Conflict)
}
/// Compose every hook group the Claude adapter declares (SessionStart and Bash PreToolUse).
/// Declaring a hook claims neither native observation nor model-visible delivery.
pub fn plan_claude(bytes: &[u8], hook_argv: &[String]) -> Result<JsonSetupPlan, SetupError> {
    let command = shell_command(hook_argv)?;
    plan_claude_command(bytes, &command)
}

/// Owned entries derived from the adapter declaration, in declaration order.
fn claude_entries(command: &str) -> Result<Vec<OwnedEntry>, SetupError> {
    super::claude::declared_hook_groups(command)
        .into_iter()
        .map(|(event, group)| owned(event, group))
        .collect()
}

/// Codex's declared context-only hook groups (SessionStart, SubagentStart, Bash PreToolUse) in the
/// `hooks.json` form. The 10 s timeout bounds Codex's wait (its default is 600 s); the hook's own
/// watchdog ends it far earlier.
fn codex_entries(command: &str) -> Result<Vec<OwnedEntry>, SetupError> {
    codex::DECLARATION
        .owned_hooks
        .iter()
        .map(|hook| {
            let command = event_command(command, hook.event);
            let mut group = json!({"hooks":[{"type":"command","command":command,"timeout":10}]});
            if let Some(matcher) = hook.matcher {
                group["matcher"] = json!(matcher);
            }
            owned(hook.event, group)
        })
        .collect()
}

fn plan_claude_command(bytes: &[u8], command: &str) -> Result<JsonSetupPlan, SetupError> {
    compose_json(bytes, &claude_entries(command)?, &[])
}

/// Compose hook groups and ensure each given allow rule is present (never duplicated).
fn compose_json(
    bytes: &[u8],
    entries: &[OwnedEntry],
    rules: &[&str],
) -> Result<JsonSetupPlan, SetupError> {
    let mut v = root(bytes)?;
    for rule in rules {
        ensure_allow(&mut v, rule)?;
    }
    let map = v.as_object_mut().ok_or(SetupError::Invalid)?;
    let hooks = map
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(SetupError::Invalid)?;
    for entry in entries {
        let event = hooks
            .entry(entry.event.clone())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or(SetupError::Invalid)?;
        merge(event, entry)?;
    }
    let proposed_bytes = serde_json::to_vec(&v).map_err(|_| SetupError::Invalid)?;
    if proposed_bytes.len() > 1_048_576 {
        return Err(SetupError::TooLarge);
    }
    Ok(JsonSetupPlan {
        proposed_bytes,
        base_fingerprint: fingerprint(bytes),
        owned: entries.to_vec(),
    })
}

/// The entry's own registered command.
fn entry_command(entry: &OwnedEntry) -> Option<&str> {
    entry.group["hooks"][0]["command"].as_str()
}

/// The single base hook command shared by every owned entry, if the entries agree on one: each
/// entry's command with its own `--event` pair removed. Entries must also be uniformly evented
/// or uniformly legacy (no `--event` anywhere).
pub fn shared_command(entries: &[OwnedEntry]) -> Option<String> {
    let first = entries.first()?;
    let base = base_command(entry_command(first)?, &first.event).into_owned();
    let evented = entry_command(first)? != base;
    entries
        .iter()
        .all(|entry| {
            let Some(command) = entry_command(entry) else {
                return false;
            };
            entry.group["hooks"]
                .as_array()
                .is_some_and(|hooks| hooks.len() == 1)
                && base_command(command, &entry.event) == base.as_str()
                && (command != base) == evented
        })
        .then_some(base)
}

/// `entries` with every `--event` pair removed: the registration form of an installation made
/// before per-event registration.
fn legacy_entries(entries: &[OwnedEntry]) -> Result<Vec<OwnedEntry>, SetupError> {
    entries
        .iter()
        .map(|entry| {
            let command = entry_command(entry).ok_or(SetupError::Invalid)?;
            let mut group = entry.group.clone();
            group["hooks"][0]["command"] = json!(base_command(command, &entry.event));
            owned(&entry.event, group)
        })
        .collect()
}

/// Whether the entries are exactly what the adapter declares today. Only inspection's
/// `installed` and the install/upgrade target ask this question.
fn is_current_declaration(kind: SettingsKind, entries: &[OwnedEntry]) -> bool {
    shared_command(entries)
        .and_then(|base| kind.entries(&base).ok())
        .is_some_and(|declared| declared == entries)
}

/// Whether the entries are the current declaration as an installation made before per-event
/// registration wrote it: no `--event` on any command. Still an installed (working) hook set;
/// `setup` upgrades it.
fn is_legacy_declaration(kind: SettingsKind, entries: &[OwnedEntry]) -> bool {
    shared_command(entries)
        .and_then(|base| kind.entries(&base).ok())
        .and_then(|declared| legacy_entries(&declared).ok())
        .is_some_and(|legacy| legacy == entries)
}

/// The manifest's own recorded entries are self-consistent: a nonempty list sharing one command
/// with exactly one hook per group, unique events, and fingerprints that match their groups.
/// Removal trusts these (plus `uninstall_json` exactness), never the current declaration.
fn recorded_entries_valid(entries: &[OwnedEntry]) -> bool {
    shared_command(entries).is_some()
        && entries.iter().enumerate().all(|(i, entry)| {
            !entry.event.is_empty()
                && entries[..i].iter().all(|prior| prior.event != entry.event)
                && owned(&entry.event, entry.group.clone())
                    .is_ok_and(|actual| actual.fingerprint == entry.fingerprint)
        })
}

/// Marker plus recorded-entry validation for owned entries and any in-flight superseded entries.
fn recorded_ownership_valid(manifest: &OwnershipManifest) -> bool {
    valid_owner_marker(manifest)
        && recorded_entries_valid(&manifest.owned)
        && manifest.permission.as_ref().is_none_or(permission_valid)
        // A superseded rule exists only in a prepared upgrade: an owned retired rule being
        // replaced by the declared one.
        && manifest.superseded_permission.as_ref().is_none_or(|old| {
            manifest.phase == InstallPhase::Prepared
                && permission_valid(old)
                && !permission_current(old)
                && !old.pre_existing
                && manifest.permission.as_ref().is_some_and(permission_current)
        })
        && (manifest.superseded.is_empty()
            || (manifest.phase == InstallPhase::Prepared
                && recorded_entries_valid(&manifest.superseded)
                && shared_command(&manifest.superseded) == shared_command(&manifest.owned)))
}

/// Entries whose exact recorded group is present under their event.
fn exact_present(value: &Value, entries: &[OwnedEntry]) -> Vec<OwnedEntry> {
    entries
        .iter()
        .filter(|entry| {
            value["hooks"][&entry.event]
                .as_array()
                .is_some_and(|groups| groups.contains(&entry.group))
        })
        .cloned()
        .collect()
}

/// Keep a recorded base when the current bytes are exactly that base plus the given groups
/// (unchanged since it was recorded); otherwise strip the groups structurally.
fn restoration_base(
    base: &[u8],
    current: &[u8],
    entries: &[OwnedEntry],
    rule: Option<&str>,
) -> Result<Vec<u8>, SetupError> {
    let rules: Vec<&str> = rule.into_iter().collect();
    let stripped = uninstall_claude(current, entries, &rules, base)?;
    if !base.is_empty()
        && compose_json(base, entries, &rules).is_ok_and(|plan| plan.proposed_bytes == current)
    {
        return Ok(base.to_vec());
    }
    Ok(stripped)
}

/// `current` without the permission component's rendered rules, dropping a list or
/// `permissions` object they alone left when the recorded base held none: the file as the hook
/// installation would see it with no grant in place. `None` when either is not composable.
fn without_permission_rules(current: &[u8], recorded_base: &[u8]) -> Option<Vec<u8>> {
    let mut value = root(current).ok()?;
    let base = root(recorded_base).ok()?;
    let rendered = super::permissions::claude::render();
    if let Some(permissions) = value.get_mut("permissions").and_then(Value::as_object_mut) {
        for (list, rules) in [("allow", &rendered.allow), ("ask", &rendered.ask)] {
            if let Some(entries) = permissions.get_mut(list).and_then(Value::as_array_mut) {
                entries.retain(|entry| {
                    entry
                        .as_str()
                        .is_none_or(|rule| !rules.iter().any(|r| r == rule))
                });
                if entries.is_empty() && base["permissions"].get(list).is_none() {
                    permissions.remove(list);
                }
            }
        }
        if permissions.is_empty() && base.get("permissions").is_none() {
            value.as_object_mut()?.remove("permissions");
        }
    }
    serde_json::to_vec(&value).ok()
}

struct Resume {
    bytes: Vec<u8>,
    base: Vec<u8>,
    publish: bool,
}

/// Plan the step from `current` to a file holding exactly `manifest.owned`: superseded groups from
/// an earlier installation are removed only when exact, edited ones conflict, and an independent
/// unmarked copy of the hook command refuses publication.
fn plan_resume(
    current: &[u8],
    manifest: &OwnershipManifest,
    unmarked_command: &str,
) -> Result<Resume, SetupError> {
    let value = root(current)?;
    shared_command(&manifest.owned).ok_or(SetupError::Invalid)?;
    let present = exact_present(&value, &manifest.owned);
    let superseded = exact_present(&value, &manifest.superseded);
    let rule = owned_rule(manifest);
    let rule_count = match manifest.permission.as_ref() {
        Some(permission) => allow_count(&value, &permission.rule)?,
        None => 0,
    };
    // A pre-existing user rule is never added by setup: its absence cannot be resumed.
    if manifest.permission.as_ref().is_some_and(|p| p.pre_existing) && rule_count == 0 {
        return Err(SetupError::Conflict);
    }
    let rule_present = rule.filter(|_| rule_count > 0);
    let retired_present = match superseded_rule(manifest) {
        Some(old) if allow_count(&value, old)? > 0 => Some(old),
        _ => None,
    };
    let rules_present: Vec<&str> = rule_present.into_iter().chain(retired_present).collect();
    if present.len() == manifest.owned.len()
        && superseded.is_empty()
        && retired_present.is_none()
        && (rule.is_none() || rule_present.is_some())
    {
        let leftover = manifest
            .superseded
            .iter()
            .any(|entry| has_entry_command(&value, entry));
        if !leftover {
            return Ok(Resume {
                bytes: current.to_vec(),
                base: restoration_base(
                    &manifest.installation_base_bytes,
                    current,
                    &manifest.owned,
                    rule,
                )?,
                publish: false,
            });
        }
    }
    if manifest.owned.iter().any(|entry| {
        event_has_command(&value, &entry.event, unmarked_command)
            || event_has_command(
                &value,
                &entry.event,
                &event_command(unmarked_command, &entry.event),
            )
    }) {
        return Err(SetupError::Conflict);
    }
    let mut recorded = superseded.clone();
    recorded.extend(present);
    let recorded_base = &manifest.installation_base_bytes;
    let target_rules: Vec<&str> = rule.into_iter().collect();
    let unchanged_since_base = (!recorded.is_empty() || !rules_present.is_empty())
        && !recorded_base.is_empty()
        && compose_json(recorded_base, &recorded, &rules_present)
            .is_ok_and(|plan| plan.proposed_bytes == current);
    let (base, bytes) = if unchanged_since_base {
        // The file is exactly the recorded base plus recorded groups: publish exactly the base
        // plus the target, so later removal restores the base byte-for-byte.
        (
            recorded_base.clone(),
            compose_json(recorded_base, &manifest.owned, &target_rules)?.proposed_bytes,
        )
    } else {
        // Apart from the permission component's own rules, the file may still be exactly the
        // recorded base plus recorded groups: keep that base, so removal (permissions first,
        // then hooks) still restores it byte for byte.
        let base_plus_grant = !recorded.is_empty()
            && !recorded_base.is_empty()
            && without_permission_rules(current, recorded_base).is_some_and(|plain| {
                compose_json(recorded_base, &recorded, &rules_present)
                    .is_ok_and(|plan| plan.proposed_bytes == plain)
            });
        let base = if base_plus_grant {
            recorded_base.clone()
        } else if recorded.is_empty() && rules_present.is_empty() {
            current.to_vec()
        } else {
            prune_emptied_recorded_events(
                &uninstall_claude(current, &recorded, &rules_present, recorded_base)?,
                manifest,
                recorded_base,
            )?
        };
        let retiring: Vec<&str> = retired_present.into_iter().collect();
        let stripped = if superseded.is_empty() && retiring.is_empty() {
            current.to_vec()
        } else {
            prune_emptied_recorded_events(
                &uninstall_claude(current, &superseded, &retiring, recorded_base)?,
                manifest,
                recorded_base,
            )?
        };
        (
            base,
            compose_json(&stripped, &manifest.owned, &target_rules)?.proposed_bytes,
        )
    };
    let resumed = root(&bytes)?;
    // An edited superseded group under an event the target no longer owns must not survive.
    if manifest.superseded.iter().any(|entry| {
        !manifest.owned.iter().any(|o| o.event == entry.event) && has_entry_command(&resumed, entry)
    }) {
        return Err(SetupError::Conflict);
    }
    Ok(Resume {
        bytes,
        base,
        publish: true,
    })
}

/// Drop an event array, for any event this installation owns or supersedes, that stripping recorded
/// groups left empty when the recorded base did not hold that event: the array existed only for
/// our groups, so neither the published file nor the restoration base keeps an `"Event":[]`
/// residue. An array the base held is user state and stays; a legacy empty (or unparsable) base
/// cannot prove which arrays the user held, so nothing is pruned. Pruning a still-owned event is
/// harmless for publication because `compose_json` recreates it with the owned group.
fn prune_emptied_recorded_events(
    bytes: &[u8],
    manifest: &OwnershipManifest,
    recorded_base: &[u8],
) -> Result<Vec<u8>, SetupError> {
    let base_hooks = root(recorded_base).ok().map(|base| base["hooks"].clone());
    let mut value = root(bytes)?;
    let Some(hooks) = value.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(bytes.to_vec());
    };
    for entry in manifest.owned.iter().chain(&manifest.superseded) {
        let base_held = base_hooks
            .as_ref()
            .is_none_or(|hooks| hooks.get(&entry.event).is_some());
        if !base_held
            && hooks
                .get(&entry.event)
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        {
            hooks.remove(&entry.event);
        }
    }
    serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)
}

/// Whether the file holds, under the entry's event, a group running the entry's own command.
fn has_entry_command(value: &Value, entry: &OwnedEntry) -> bool {
    entry_command(entry).is_some_and(|command| event_has_command(value, &entry.event, command))
}

fn event_has_command(value: &Value, event: &str, command: &str) -> bool {
    value["hooks"][event]
        .as_array()
        .is_some_and(|groups| groups.iter().any(|group| contains_command(group, command)))
}

fn owned_command(hook_argv: &[String], installation_id: &str) -> Result<String, SetupError> {
    if installation_id.is_empty() {
        return shell_command(hook_argv);
    }
    if !installation_id
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        return Err(SetupError::Invalid);
    }
    Ok(format!(
        "{} # herdr-threads-owner:{installation_id}",
        shell_command(hook_argv)?
    ))
}
/// Remove exact fingerprint-matching owned groups only. Other groups and keys survive.
pub fn uninstall_json(bytes: &[u8], entries: &[OwnedEntry]) -> Result<Vec<u8>, SetupError> {
    let mut v = root(bytes)?;
    for entry in entries {
        if owned(&entry.event, entry.group.clone())?.fingerprint != entry.fingerprint {
            return Err(SetupError::Conflict);
        }
        let command = entry.group["hooks"][0]["command"]
            .as_str()
            .ok_or(SetupError::Invalid)?;
        let Some(hooks) = v.get_mut("hooks") else {
            return Err(SetupError::Conflict);
        };
        let hooks = hooks.as_object_mut().ok_or(SetupError::Invalid)?;
        let Some(groups) = hooks.get_mut(&entry.event) else {
            return Err(SetupError::Conflict);
        };
        let groups = groups.as_array_mut().ok_or(SetupError::Invalid)?;
        if groups
            .iter()
            .any(|g| contains_command(g, command) && g != &entry.group)
            || groups.iter().filter(|g| *g == &entry.group).count() != 1
        {
            return Err(SetupError::Conflict);
        }
        groups.retain(|g| g != &entry.group);
    }
    serde_json::to_vec(&v).map_err(|_| SetupError::Invalid)
}
/// `existing` is the session-flags layer the printed launch line owns (the
/// public CLI passes it empty): Codex discovers the hooks of config.toml,
/// hooks.json and trusted project layers separately and runs them alongside
/// these `-c` values (hooks/list on 0.157.1, 0.158.0 and 0.159.2), so copying
/// them in would register them twice. Each session override includes the
/// supplied existing groups. hooks.state is untouched and must be measured separately.
/// Internal composition shared by contract planning and historical diagnostics.
fn plan_codex(
    existing: &[EventGroups],
    hook_argv: &[String],
) -> Result<CodexSetupPlan, SetupError> {
    let command = shell_command(hook_argv)?;
    if existing.len() > 64 {
        return Err(SetupError::TooLarge);
    }
    let mut events = existing.to_vec();
    let mut seen = std::collections::BTreeSet::new();
    for event in &events {
        if !valid_event(&event.event)
            || !seen.insert(event.event.clone())
            || event.groups.len() > 128
        {
            return Err(SetupError::Invalid);
        }
    }
    let mut owned_entries = Vec::new();
    // Exactly the adapter's declared context-only hook groups. The Bash
    // PreToolUse group carries tool-boundary attention context; the gated
    // invocation rewrite is never part of any installed group.
    for hook in codex::DECLARATION.owned_hooks {
        let name = hook.event;
        let mut group =
            json!({"hooks":[{"type":"command","command":event_command(&command, name)}]});
        if let Some(matcher) = hook.matcher {
            group["matcher"] = json!(matcher);
        }
        let entry = owned(name, group)?;
        if let Some(event) = events.iter_mut().find(|event| event.event == name) {
            merge(&mut event.groups, &entry)?;
        } else {
            events.push(EventGroups {
                event: name.into(),
                groups: vec![entry.group.clone()],
            });
        }
        owned_entries.push(entry);
    }
    let mut session_config: Vec<String> = Vec::new();
    for entry in &owned_entries {
        let event = events
            .iter()
            .find(|e| e.event == entry.event)
            .ok_or(SetupError::Invalid)?;
        session_config.push("-c".into());
        session_config.push(format!(
            "hooks.{}={}",
            event.event,
            toml_inline(&json!(event.groups))?
        ));
    }
    if session_config.iter().map(String::len).sum::<usize>() > 65536 {
        return Err(SetupError::TooLarge);
    }
    let mut plan = CodexSetupPlan {
        base_fingerprint: groups_fingerprint(existing)?,
        events,
        launch_argv: Vec::new(),
        session_config,
        owned: owned_entries,
    };
    plan.launch_argv = plan.launch_argv_for(Vec::new())?;
    Ok(plan)
}

/// Compose the registered Codex setup contract without executable metadata.
/// The handle declares registration, never runtime support or native delivery.
pub fn plan_codex_for_contract(
    existing: &[EventGroups],
    hook_argv: &[String],
    _: &super::operational::CodexContract,
) -> Result<CodexSetupPlan, SetupError> {
    plan_codex(existing, hook_argv)
}

/// Diagnostic/fixture compatibility for historical installed-version callers.
pub fn plan_codex_for_version(
    existing: &[EventGroups],
    hook_argv: &[String],
    version: &codex::InstalledVersion,
) -> Result<CodexSetupPlan, SetupError> {
    if !codex::DECLARATION.recipes.contains(version.recipe()) {
        return Err(SetupError::Invalid);
    }
    plan_codex(existing, hook_argv)
}
fn valid_event(s: &str) -> bool {
    !s.is_empty() && s.len() < 64 && s.chars().all(|c| c.is_ascii_alphanumeric())
}
/// Serialize the bounded JSON-compatible hook schema into TOML inline values; null is rejected.
fn toml_inline(v: &Value) -> Result<String, SetupError> {
    match v {
        Value::String(s) => serde_json::to_string(s).map_err(|_| SetupError::Invalid),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Array(items) => {
            let values: Result<Vec<_>, _> = items.iter().map(toml_inline).collect();
            Ok(format!("[{}]", values?.join(",")))
        }
        Value::Object(map) => {
            let values: Result<Vec<_>, _> = map
                .iter()
                .map(|(k, v)| {
                    Ok(format!(
                        "{}={}",
                        serde_json::to_string(k).map_err(|_| SetupError::Invalid)?,
                        toml_inline(v)?
                    ))
                })
                .collect();
            Ok(format!("{{{}}}", values?.join(",")))
        }
        Value::Null => Err(SetupError::Invalid),
    }
}

/// Check exact supplied configuration bytes again immediately before a future installer publishes.
pub fn verify_base(plan: &JsonSetupPlan, current: &[u8]) -> Result<(), SetupError> {
    if fingerprint(current) == plan.base_fingerprint {
        Ok(())
    } else {
        Err(SetupError::Conflict)
    }
}
/// Typed Codex counterpart of exact-entry uninstall. Missing/edited entries are conservatively refused.
pub fn uninstall_codex(
    existing: &[EventGroups],
    entries: &[OwnedEntry],
) -> Result<Vec<EventGroups>, SetupError> {
    let mut root = serde_json::Map::new();
    let mut seen = std::collections::BTreeSet::new();
    for event in existing {
        if !valid_event(&event.event) || !seen.insert(event.event.clone()) {
            return Err(SetupError::Invalid);
        }
        root.insert(event.event.clone(), json!(event.groups));
    }
    let bytes = serde_json::to_vec(&json!({"hooks":root})).map_err(|_| SetupError::Invalid)?;
    let removed: Value = serde_json::from_slice(&uninstall_json(&bytes, entries)?)
        .map_err(|_| SetupError::Invalid)?;
    existing
        .iter()
        .map(|event| {
            Ok(EventGroups {
                event: event.event.clone(),
                groups: removed["hooks"][&event.event]
                    .as_array()
                    .ok_or(SetupError::Invalid)?
                    .clone(),
            })
        })
        .collect()
}

impl JsonSetupPlan {
    /// Settings path stays a native argv value. This is a proposal, not a launch or installation.
    pub fn launch_argv(&self, settings_path: &str) -> Result<Vec<String>, SetupError> {
        if settings_path.is_empty()
            || settings_path.len() > 4096
            || settings_path.chars().any(char::is_control)
        {
            return Err(SetupError::Invalid);
        }
        Ok(vec!["--settings".into(), settings_path.into()])
    }
}
fn groups_fingerprint(groups: &[EventGroups]) -> Result<String, SetupError> {
    let bytes = serde_json::to_vec(groups).map_err(|_| SetupError::Invalid)?;
    if bytes.len() > 1_048_576 {
        return Err(SetupError::TooLarge);
    }
    Ok(fingerprint(&bytes))
}

/// Installation and observation are independent. Only a native callback can mark observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeObservation {
    Unknown,
    Observed,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookInspection {
    pub installed: bool,
    pub observed: NativeObservation,
    /// Present only for an exact, owned installation; suitable for managed launch.
    pub configured_hook: Option<ConfiguredHook>,
    /// Claude only: the allow rule an earlier setup recorded and whether it is present now.
    pub allow_rule: Option<AllowRuleInspection>,
    /// The owned groups belong to another setup's installation (they carry its owner marker),
    /// adopted for this file: recorded by a manifest, or found by inspection with none yet.
    pub adopted: Option<Adoption>,
    /// The installed hooks predate per-event registration (no `--event` on their commands): they
    /// still work, and `setup` rewrites them. True only for that legacy form.
    pub legacy_event_registration: bool,
}

/// Hook groups of another herdr-threads setup (same hook command, another owner marker) that this
/// file holds exactly, e.g. a `hooks.json` copied into a second `CODEX_HOME` with its trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoption {
    /// The owner marker (installation id) the adopted groups carry.
    pub owner: String,
    /// A manifest for this file records the adoption (`setup` ran); otherwise inspection found
    /// the groups with no manifest of this file's own.
    pub recorded: bool,
}

/// Who holds the allow rule an earlier hook setup recorded, until the permission component
/// retires the record (current hook setup records none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowRuleOwnership {
    /// Setup added the rule and unsetup removes it.
    Owned,
    /// The user's settings held the identical rule before setup; unsetup leaves it.
    PreExisting,
    /// The manifest records the retired `claude::CALLER_CONTEXT_ALLOW_RULE`.
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowRuleInspection {
    pub rule: &'static str,
    pub ownership: AllowRuleOwnership,
    /// The rule currently appears in `permissions.allow`.
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipManifest {
    pub version: u8,
    pub harness: String,
    pub scope: String,
    pub path: String,
    pub original_bytes: Vec<u8>,
    #[serde(default)]
    pub installation_base_bytes: Vec<u8>,
    #[serde(default)]
    pub installation_id: String,
    pub owned: Vec<OwnedEntry>,
    pub installed_fingerprint: String,
    #[serde(default)]
    pub phase: InstallPhase,
    /// Prepared upgrade only: recorded groups of the earlier installation that the in-flight
    /// upgrade replaces. Removal and resumption remove them only when still exact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded: Vec<OwnedEntry>,
    /// Claude project only: the owned `permissions.allow` rule. Absent in manifests recorded
    /// before the rule was declared; re-running setup adds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<OwnedPermission>,
    /// Prepared upgrade only: the owned retired rule (`claude::CALLER_CONTEXT_ALLOW_RULE`) that
    /// the in-flight upgrade removes while `permission` records the declared rule. Removal and
    /// resumption remove it when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_permission: Option<OwnedPermission>,
    /// The owned groups were not written by this installation: they were already in the file,
    /// carrying the owner marker `installation_id` of another setup (a copied hook file), and
    /// were adopted in place so the file's bytes (and Codex's trust hashes) stay unchanged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adopted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InstallPhase {
    Prepared,
    #[default]
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallFault {
    None,
    PartialManifest,
    AfterManifest,
    BeforeReplacement,
    /// Upgrade: run the action, then stop after intent is durable and before settings publication.
    InterruptAfterIntent,
    /// Resumed or upgraded publication: run the action, then stop before the manifest is finalized.
    InterruptAfterPublication,
}

pub(crate) fn config_bytes(path: &Path) -> Result<Vec<u8>, SetupError> {
    if path
        .symlink_metadata()
        .map_err(|_| SetupError::Io)?
        .file_type()
        .is_symlink()
    {
        return Err(SetupError::Invalid);
    }
    let bytes = fs::read(path).map_err(|_| SetupError::Io)?;
    if bytes.len() > 1_048_576 {
        return Err(SetupError::TooLarge);
    }
    Ok(bytes)
}

/// Replace a user configuration file whose bytes the caller just validated as `current`.
/// An unchanged result writes nothing; otherwise `current` is first kept as a private sibling
/// backup (see [`backup_user_config`]), whatever it holds. A backup stays even when
/// the replacement then fails. herdr-threads state and manifests use
/// [`write_replacement`] directly and are never backed up.
pub(crate) fn write_user_config(
    path: &Path,
    current: &[u8],
    bytes: &[u8],
) -> Result<(), SetupError> {
    if config_bytes(path)? != current {
        return Err(SetupError::Conflict);
    }
    if current == bytes {
        return Ok(());
    }
    back_up_before_change(path, current)?;
    write_replacement(path, bytes, false)
}

thread_local! {
    static BACKUP_SESSION: std::cell::RefCell<Option<std::collections::HashSet<std::path::PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

/// One command run: while a session is open, each user configuration file is backed up once,
/// before its first change, so one run leaves one pre-image per file however many components
/// write it. Without a session every change is backed up.
pub struct BackupSession(());
impl BackupSession {
    pub fn start() -> Self {
        BACKUP_SESSION.with(|session| *session.borrow_mut() = Some(Default::default()));
        Self(())
    }
}
impl Drop for BackupSession {
    fn drop(&mut self) {
        BACKUP_SESSION.with(|session| *session.borrow_mut() = None);
    }
}

/// This run created `path` (setup's own placeholder): it has no pre-image to keep, so its
/// later changes in the same session are not backed up.
pub(crate) fn note_created_user_config(path: &Path) {
    BACKUP_SESSION.with(|session| {
        if let Some(seen) = session.borrow_mut().as_mut() {
            seen.insert(path.to_path_buf());
        }
    });
}

/// Back up `current` unless this session already kept a pre-image of `path` or created it.
fn back_up_before_change(path: &Path, current: &[u8]) -> Result<(), SetupError> {
    let first = BACKUP_SESSION.with(|session| {
        session
            .borrow_mut()
            .as_mut()
            .is_none_or(|seen| seen.insert(path.to_path_buf()))
    });
    if first {
        backup_user_config(path, current)?;
    }
    Ok(())
}

/// Delete a user configuration file whose bytes the caller just validated as `current`, keeping
/// them first as a backup (see [`backup_user_config`]).
pub(crate) fn remove_user_config(path: &Path, current: &[u8]) -> Result<(), SetupError> {
    if config_bytes(path)? != current {
        return Err(SetupError::Conflict);
    }
    back_up_before_change(path, current)?;
    fs::remove_file(path).map_err(|_| SetupError::Io)?;
    fs::File::open(path.parent().ok_or(SetupError::Invalid)?)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| SetupError::Io)
}

/// Keep `current` beside `path` as `<name>.<UTC timestamp>-<uuid>.herdr-threads`: mode 0600,
/// never overwriting, synced with its directory before the caller mutates `path`. Backups are
/// retained; herdr-threads never restores from, prunes or trusts them.
pub(crate) fn backup_user_config(
    path: &Path,
    current: &[u8],
) -> Result<std::path::PathBuf, SetupError> {
    let parent = path.parent().ok_or(SetupError::Invalid)?;
    let file_name = path
        .file_name()
        .ok_or(SetupError::Invalid)?
        .to_string_lossy();
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| SetupError::Io)?
        .as_millis();
    let stamp: String =
        super::manifest::format_rfc3339_utc(i64::try_from(millis).map_err(|_| SetupError::Io)?)
            .chars()
            .filter(|c| !matches!(c, '-' | ':'))
            .collect();
    let backup = parent.join(format!(
        "{file_name}.{stamp}-{}.herdr-threads",
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&backup).map_err(|_| SetupError::Io)?;
        file.write_all(current).map_err(|_| SetupError::Io)?;
        file.sync_all().map_err(|_| SetupError::Io)?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| SetupError::Io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&backup);
    }
    result.map(|()| backup)
}

/// Test oracle: the retained backups of `config`, as sorted byte contents.
#[cfg(test)]
pub(crate) fn user_config_backups(config: &Path) -> Vec<Vec<u8>> {
    let prefix = format!("{}.", config.file_name().unwrap().to_string_lossy());
    let mut found: Vec<Vec<u8>> = fs::read_dir(config.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.ends_with(".herdr-threads")
        })
        .map(|entry| fs::read(entry.path()).unwrap())
        .collect();
    found.sort();
    found
}

pub(crate) fn write_replacement(
    path: &Path,
    bytes: &[u8],
    private: bool,
) -> Result<(), SetupError> {
    let parent = path.parent().ok_or(SetupError::Invalid)?;
    let file_name = path
        .file_name()
        .ok_or(SetupError::Invalid)?
        .to_string_lossy();
    let temp = parent.join(format!(".{file_name}.herdr-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temp).map_err(|_| SetupError::Io)?;
        if !private {
            file.set_permissions(
                fs::metadata(path)
                    .map_err(|_| SetupError::Io)?
                    .permissions(),
            )
            .map_err(|_| SetupError::Io)?;
        }
        file.write_all(bytes).map_err(|_| SetupError::Io)?;
        file.sync_all().map_err(|_| SetupError::Io)?;
        fs::rename(&temp, path).map_err(|_| SetupError::Io)?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| SetupError::Io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Whether a recorded manifest belongs to this kind of installation of exactly `config`. A kind
/// that does not own the allow rule never records one.
fn manifest_matches(
    kind: SettingsKind,
    manifest: &OwnershipManifest,
    config: &Path,
) -> Result<bool, SetupError> {
    Ok(manifest.harness == kind.harness()
        && manifest.scope == kind.scope()
        && manifest.path == config.to_str().ok_or(SetupError::Invalid)?
        && (kind.manages_permission()
            || (manifest.permission.is_none() && manifest.superseded_permission.is_none())))
}

/// Explicitly install the owned hook groups (and, for Claude, the owned allow rule) into one
/// user-level hook file. The caller supplies the exact baseline read during planning; a changed
/// file is refused before publication.
pub fn install_user_settings(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    hook_argv: &[String],
    expected_base: &[u8],
) -> Result<OwnershipManifest, SetupError> {
    install_user_settings_inner(
        kind,
        config,
        manifest_path,
        hook_argv,
        expected_base,
        InstallFault::None,
        || {},
    )
}

#[cfg(test)]
pub(crate) fn install_user_settings_with_fault<F: FnOnce()>(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    hook_argv: &[String],
    expected_base: &[u8],
    fault: InstallFault,
    action: F,
) -> Result<OwnershipManifest, SetupError> {
    install_user_settings_inner(
        kind,
        config,
        manifest_path,
        hook_argv,
        expected_base,
        fault,
        action,
    )
}

fn install_user_settings_inner<F: FnOnce()>(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    hook_argv: &[String],
    expected_base: &[u8],
    fault: InstallFault,
    action: F,
) -> Result<OwnershipManifest, SetupError> {
    let mut action = Some(action);
    if config.file_name().and_then(|s| s.to_str()) != Some(kind.file_name())
        || manifest_path == config
    {
        return Err(SetupError::Invalid);
    }
    let unmarked_entries = kind.entries(&shell_command(hook_argv)?)?;
    let unmarked_plan = compose_json(expected_base, &unmarked_entries, &[])?;
    let unmarked_legacy = legacy_entries(&unmarked_entries)?;
    if read_manifest(manifest_path)?.is_none() {
        // A file that already holds exactly this command's groups under another setup's owner
        // marker is adopted, not given a second set: only a manifest is recorded, and the
        // recorded-manifest path below then finds the installation complete.
        if config_bytes(config)? != expected_base {
            return Err(SetupError::Conflict);
        }
        adopt_user_settings(kind, config, manifest_path, hook_argv)?;
    }
    if let Some(mut manifest) = read_manifest(manifest_path)? {
        let expected_command = owned_command(hook_argv, &manifest.installation_id)?;
        let expected = kind.entries(&expected_command)?;
        // Ownership is validated against the manifest's own records. A different hook command
        // (argv) is not an upgrade: remove the installation first, then set up again.
        if !manifest_matches(kind, &manifest, config)?
            || !recorded_ownership_valid(&manifest)
            || shared_command(&manifest.owned).as_deref() != Some(expected_command.as_str())
            || manifest.original_bytes != expected_base
        {
            return Err(SetupError::Conflict);
        }
        if manifest.installation_id.is_empty()
            && (manifest.phase == InstallPhase::Prepared || manifest.owned != expected)
        {
            return Err(SetupError::Conflict);
        }
        let unmarked_command = shell_command(hook_argv)?;
        let current = config_bytes(config)?;
        // The permission rule is not part of the hook installation: a manifest that still records
        // one is first settled by the permission component, which retires the record.
        if manifest.permission.is_some() || manifest.superseded_permission.is_some() {
            return Err(SetupError::Conflict);
        }
        if manifest.phase == InstallPhase::Installed {
            // Every recorded owned group must still be exact before anything is claimed.
            uninstall_json(&current, &manifest.owned)?;
            if manifest.owned == expected {
                return Ok(manifest);
            }
            // Upgrade to the current declaration. Prove the target composes before durably
            // recording the intent, so a conflict leaves both files untouched.
            let mut intent = manifest.clone();
            intent.superseded = manifest
                .owned
                .iter()
                .filter(|entry| !expected.contains(entry))
                .cloned()
                .collect();
            intent.owned = expected.clone();
            intent.phase = InstallPhase::Prepared;
            plan_resume(&current, &intent, &unmarked_command)?;
            replace_manifest(manifest_path, &intent)?;
            if fault == InstallFault::InterruptAfterIntent {
                action.take().expect("one fault action")();
                return Err(SetupError::Io);
            }
            manifest = intent;
        } else if manifest.owned != expected {
            // A prepared installation recorded under an earlier declaration: its groups become
            // superseded by the current target.
            for entry in std::mem::take(&mut manifest.owned) {
                if !expected.contains(&entry) && !manifest.superseded.contains(&entry) {
                    if manifest.superseded.iter().any(|s| s.event == entry.event) {
                        return Err(SetupError::Conflict);
                    }
                    manifest.superseded.push(entry);
                }
            }
            manifest.owned = expected.clone();
            replace_manifest(manifest_path, &manifest)?;
        }
        let resume = plan_resume(&current, &manifest, &unmarked_command)?;
        if resume.publish {
            if config_bytes(config)? != current {
                return Err(SetupError::Conflict);
            }
            write_user_config(config, &current, &resume.bytes)?;
            if fault == InstallFault::InterruptAfterPublication {
                action.take().expect("one fault action")();
                return Err(SetupError::Io);
            }
        }
        manifest.phase = InstallPhase::Installed;
        manifest.installed_fingerprint = fingerprint(&resume.bytes);
        manifest.installation_base_bytes = resume.base;
        manifest.superseded.clear();
        replace_manifest(manifest_path, &manifest)?;
        return Ok(manifest);
    }
    if config_bytes(config)? != expected_base {
        return Err(SetupError::Conflict);
    }
    let baseline: Value = root(expected_base)?;
    if unmarked_plan
        .owned
        .iter()
        .chain(&unmarked_legacy)
        .any(|entry| {
            baseline["hooks"][&entry.event]
                .as_array()
                .is_some_and(|groups| groups.contains(&entry.group))
        })
    {
        return Err(SetupError::Conflict);
    }
    let installation_id = uuid::Uuid::new_v4().to_string();
    let plan = compose_json(
        expected_base,
        &kind.entries(&owned_command(hook_argv, &installation_id)?)?,
        &[],
    )?;
    let manifest = OwnershipManifest {
        version: 2,
        harness: kind.harness().into(),
        scope: kind.scope().into(),
        path: config.to_str().ok_or(SetupError::Invalid)?.into(),
        original_bytes: expected_base.to_vec(),
        installation_base_bytes: expected_base.to_vec(),
        installation_id,
        owned: plan.owned,
        installed_fingerprint: fingerprint(&plan.proposed_bytes),
        phase: InstallPhase::Prepared,
        superseded: Vec::new(),
        permission: None,
        superseded_permission: None,
        adopted: false,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SetupError::Invalid)?;
    if manifest_bytes.len() > 16_777_216 {
        return Err(SetupError::TooLarge);
    }
    publish_manifest(
        manifest_path,
        &manifest_bytes,
        fault == InstallFault::PartialManifest,
    )?;
    if fault == InstallFault::AfterManifest {
        action.take().expect("one fault action")();
    }
    if config_bytes(config)? != expected_base {
        return Err(SetupError::Conflict);
    }
    if fault == InstallFault::BeforeReplacement {
        action.take().expect("one fault action")();
    }
    write_user_config(config, expected_base, &plan.proposed_bytes)?;
    let mut installed = manifest;
    installed.phase = InstallPhase::Installed;
    replace_manifest(manifest_path, &installed)?;
    Ok(installed)
}

pub(crate) fn publish_manifest(path: &Path, bytes: &[u8], partial: bool) -> Result<(), SetupError> {
    let parent = path.parent().ok_or(SetupError::Invalid)?;
    let temp = parent.join(format!(".herdr-manifest-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temp).map_err(|_| SetupError::Io)?;
        if partial {
            file.write_all(&bytes[..bytes.len() / 2])
                .map_err(|_| SetupError::Io)?;
            return Err(SetupError::Io);
        }
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| SetupError::Io)?;
        fs::hard_link(&temp, path).map_err(|_| SetupError::Conflict)?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| SetupError::Io)
    })();
    let _ = fs::remove_file(&temp);
    result
}

fn replace_manifest(path: &Path, manifest: &OwnershipManifest) -> Result<(), SetupError> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| SetupError::Invalid)?;
    write_replacement(path, &bytes, true)
}

fn read_manifest(path: &Path) -> Result<Option<OwnershipManifest>, SetupError> {
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
    let bytes = fs::read(path).map_err(|_| SetupError::Io)?;
    if bytes.len() > 16_777_216 {
        return Err(SetupError::TooLarge);
    }
    let manifest: OwnershipManifest =
        serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
    if !matches!(manifest.version, 1 | 2) {
        return Err(SetupError::Invalid);
    }
    Ok(Some(manifest))
}

/// The permission rules an earlier Claude hook setup recorded in its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedPermission {
    /// The rules that setup added, which are owned: never a rule the user already held.
    pub owned: Vec<String>,
    /// Whether the user's settings had no `permissions` / `permissions.allow` before that setup
    /// (unknown for a legacy manifest without a recorded base: then both are kept).
    pub created_permissions: bool,
    pub created_allow: bool,
    /// The exact manifest bytes the record was read from.
    manifest: Vec<u8>,
}

/// The permission record of the Claude hook manifest at `manifest_path` for `config`, validated
/// like removal: only a declared or retired rule with its fingerprint is ever owned.
pub fn recorded_hook_permission(
    config: &Path,
    manifest_path: &Path,
) -> Result<Option<RecordedPermission>, SetupError> {
    let Some(manifest) = read_manifest(manifest_path)? else {
        return Ok(None);
    };
    if manifest.permission.is_none() && manifest.superseded_permission.is_none() {
        return Ok(None);
    }
    if !manifest_matches(SettingsKind::ClaudeUser, &manifest, config)?
        || !recorded_ownership_valid(&manifest)
    {
        return Err(SetupError::Conflict);
    }
    let owned = owned_rule(&manifest)
        .into_iter()
        .chain(superseded_rule(&manifest))
        .map(str::to_owned)
        .collect();
    let base = root(&manifest.installation_base_bytes).ok();
    Ok(Some(RecordedPermission {
        owned,
        created_permissions: base
            .as_ref()
            .is_some_and(|base| base.get("permissions").is_none()),
        created_allow: base
            .as_ref()
            .is_some_and(|base| base["permissions"].get("allow").is_none()),
        manifest: fs::read(manifest_path).map_err(|_| SetupError::Io)?,
    }))
}

/// Drop the permission record [`recorded_hook_permission`] read, once the permission component
/// owns its outcome. A manifest that changed since is a conflict; one already without a record
/// is done.
pub fn retire_hook_permission(
    manifest_path: &Path,
    recorded: &RecordedPermission,
) -> Result<(), SetupError> {
    let Some(mut manifest) = read_manifest(manifest_path)? else {
        return Err(SetupError::Conflict);
    };
    if manifest.permission.is_none() && manifest.superseded_permission.is_none() {
        return Ok(());
    }
    if fs::read(manifest_path).map_err(|_| SetupError::Io)? != recorded.manifest {
        return Err(SetupError::Conflict);
    }
    manifest.permission = None;
    manifest.superseded_permission = None;
    // The installation's own published state no longer includes the rule, so a later removal
    // of a file back to exactly the base plus the hooks still restores the base byte for byte.
    if manifest.phase == InstallPhase::Installed
        && !manifest.installation_base_bytes.is_empty()
        && let Ok(plan) = compose_json(&manifest.installation_base_bytes, &manifest.owned, &[])
    {
        manifest.installed_fingerprint = fingerprint(&plan.proposed_bytes);
    }
    replace_manifest(manifest_path, &manifest)
}

/// The recorded ownership manifest, if one exists. The setup CLI passes its
/// `original_bytes` back as the install baseline on a re-run and shows its
/// recorded command; ownership decisions stay in install/inspect/remove.
pub fn read_settings_manifest(
    manifest_path: &Path,
) -> Result<Option<OwnershipManifest>, SetupError> {
    read_manifest(manifest_path)
}

fn valid_owner_marker(manifest: &OwnershipManifest) -> bool {
    if manifest.version < 2 {
        return manifest.installation_id.is_empty();
    }
    let Ok(id) = uuid::Uuid::parse_str(&manifest.installation_id) else {
        return false;
    };
    if id.to_string() != manifest.installation_id {
        return false;
    }
    shared_command(&manifest.owned)
        .is_some_and(|command| command.ends_with(&format!(" # herdr-threads-owner:{id}")))
}

pub fn inspect_user_settings(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    observed: NativeObservation,
) -> Result<HookInspection, SetupError> {
    inspect_user_settings_for(kind, config, manifest_path, observed, None)
}

/// [`inspect_user_settings`], and, given the current hook argv, a file with no manifest of its own
/// whose groups are another setup's exact groups for this command is reported installed
/// (adopted, not yet recorded). Claude also needs the declared allow rule present.
pub fn inspect_user_settings_for(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    observed: NativeObservation,
    hook_argv: Option<&[String]>,
) -> Result<HookInspection, SetupError> {
    let Some(manifest) = read_manifest(manifest_path)? else {
        if let Some(argv) = hook_argv
            // Best effort: a file that cannot be read or parsed is simply not installed here,
            // as it was before adoption existed.
            && let Ok(Some(adoptable)) = adoptable_user_settings(kind, config, argv)
            && let Ok(current) = config_bytes(config)
        {
            return Ok(HookInspection {
                installed: true,
                observed,
                configured_hook: Some(ConfiguredHook {
                    scope: kind.scope().into(),
                    path: config.to_string_lossy().into_owned(),
                    fingerprint: fingerprint(&current),
                }),
                allow_rule: None,
                adopted: Some(Adoption {
                    owner: adoptable.installation_id,
                    recorded: false,
                }),
                legacy_event_registration: false,
            });
        }
        return Ok(HookInspection {
            installed: false,
            observed,
            configured_hook: None,
            allow_rule: None,
            adopted: None,
            legacy_event_registration: false,
        });
    };
    if !manifest_matches(kind, &manifest, config)? {
        return Err(SetupError::Conflict);
    }
    let current = config_bytes(config)?;
    // Installed means every hook the adapter currently declares; an older subset is stale.
    let legacy = is_legacy_declaration(kind, &manifest.owned);
    let declaration_valid = recorded_ownership_valid(&manifest)
        && shared_command(&manifest.owned).is_some_and(|s| !s.is_empty())
        && (is_current_declaration(kind, &manifest.owned) || legacy);
    // A rule recorded by an earlier setup stays reported until the permission component
    // retires the record; current hook setup records none.
    let allow_rule = manifest.permission.as_ref().map(|p| {
        let rule = if permission_current(p) {
            DECLARED_RULE
        } else {
            RETIRED_RULE
        };
        AllowRuleInspection {
            rule,
            ownership: match p {
                _ if !permission_current(p) => AllowRuleOwnership::Superseded,
                p if p.pre_existing => AllowRuleOwnership::PreExisting,
                _ => AllowRuleOwnership::Owned,
            },
            present: serde_json::from_slice::<Value>(&current)
                .ok()
                .and_then(|v| allow_count(&v, rule).ok())
                .is_some_and(|count| count > 0),
        }
    });
    let installed = manifest.phase == InstallPhase::Installed
        && declaration_valid
        && manifest.owned.iter().all(|entry| {
            serde_json::from_slice::<Value>(&current)
                .ok()
                .and_then(|v| v["hooks"][&entry.event].as_array().cloned())
                .is_some_and(|groups| groups.contains(&entry.group))
                && owned(&entry.event, entry.group.clone())
                    .is_ok_and(|actual| actual.fingerprint == entry.fingerprint)
        });
    Ok(HookInspection {
        installed,
        observed,
        configured_hook: installed.then(|| ConfiguredHook {
            scope: kind.scope().into(),
            path: manifest.path.clone(),
            fingerprint: fingerprint(&current),
        }),
        allow_rule,
        adopted: manifest.adopted.then(|| Adoption {
            owner: manifest.installation_id.clone(),
            recorded: true,
        }),
        legacy_event_registration: installed && legacy,
    })
}

/// Removal refuses edited or missing owned groups, and keeps other current settings.
pub fn remove_user_settings(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
) -> Result<(), SetupError> {
    let manifest = read_manifest(manifest_path)?.ok_or(SetupError::Conflict)?;
    if !manifest_matches(kind, &manifest, config)? {
        return Err(SetupError::Conflict);
    }
    let current = config_bytes(config)?;
    // Removal trusts the manifest's own recorded entries, never the current declaration, so an
    // installation made under an earlier declaration stays removable.
    if !recorded_ownership_valid(&manifest) {
        return Err(SetupError::Conflict);
    }
    if manifest.phase == InstallPhase::Prepared && manifest.installation_id.is_empty() {
        return Err(SetupError::Conflict);
    }
    let rule = owned_rule(&manifest);
    // Owned rules this removal strips: the recorded one and, for an interrupted upgrade, the
    // retired one it was replacing.
    let rules: Vec<&str> = rule.into_iter().chain(superseded_rule(&manifest)).collect();
    let base = &manifest.installation_base_bytes;
    // A file exactly the recorded base plus the owned groups restores the base byte for byte,
    // even when its fingerprint changed in between (a permission grant added and removed).
    let removed = if manifest.phase == InstallPhase::Installed
        && !manifest.installation_base_bytes.is_empty()
    {
        let structurally_removed = uninstall_claude(&current, &manifest.owned, &rules, base)?;
        if compose_json(&manifest.installation_base_bytes, &manifest.owned, &rules)
            .is_ok_and(|plan| plan.proposed_bytes == current)
        {
            manifest.installation_base_bytes.clone()
        } else {
            structurally_removed
        }
    } else if manifest.phase == InstallPhase::Prepared {
        // Remove the exact recorded groups that are present (owned or superseded); an edited copy
        // of the owned command under any recorded event conflicts.
        shared_command(&manifest.owned).ok_or(SetupError::Invalid)?;
        let value = root(&current)?;
        let mut recorded = manifest.owned.clone();
        recorded.extend(manifest.superseded.iter().cloned());
        let present = exact_present(&value, &recorded);
        // The owned rule may or may not have been published before the interruption, and an
        // upgrade's retired rule may or may not have been removed yet.
        let mut rule_present = false;
        for rule in &rules {
            rule_present |= allow_count(&value, rule)? > 0;
        }
        let removed = if present.is_empty() && !rule_present {
            current.clone()
        } else {
            uninstall_claude(&current, &present, &rules, base)?
        };
        let remaining = root(&removed)?;
        if recorded
            .iter()
            .any(|entry| has_entry_command(&remaining, entry))
        {
            return Err(SetupError::Conflict);
        }
        removed
    } else {
        uninstall_claude(&current, &manifest.owned, &rules, base)?
    };
    if config_bytes(config)? != current {
        return Err(SetupError::Conflict);
    }
    if removed != current {
        write_user_config(config, &current, &removed)?;
    }
    fs::remove_file(manifest_path).map_err(|_| SetupError::Io)
}

/// Another setup's installation found in a hook file: its owner marker and the exact groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoptable {
    pub installation_id: String,
    pub owned: Vec<OwnedEntry>,
}

/// Whether `bytes` hold, under every declared event, exactly one group running this hook command
/// registered for that event (`--event <event>`)
/// apart from its owner marker, all with one valid marker, each exactly the declared group for
/// `<command> # herdr-threads-owner:<id>`. An unmarked copy, a second or edited copy, two markers,
/// or a missing event is not adoptable (those keep their existing install/refusal behavior).
pub fn find_adoptable(
    kind: SettingsKind,
    bytes: &[u8],
    hook_argv: &[String],
) -> Result<Option<Adoptable>, SetupError> {
    let base = shell_command(hook_argv)?;
    let value = root(bytes)?;
    let mut id: Option<String> = None;
    for template in kind.entries(&base)? {
        let evented = event_command(&base, &template.event);
        let prefix = format!("{evented}{OWNER_MARKER}");
        let Some(groups) = value["hooks"][&template.event].as_array() else {
            return Ok(None);
        };
        let mut found = 0;
        for hook in groups
            .iter()
            .filter_map(|group| group.get("hooks").and_then(Value::as_array))
            .flatten()
        {
            let Some(command) = hook.get("command").and_then(Value::as_str) else {
                continue;
            };
            if command == base || command == evented {
                return Ok(None);
            }
            let Some(marker) = command.strip_prefix(&prefix) else {
                continue;
            };
            if !uuid::Uuid::parse_str(marker).is_ok_and(|uuid| uuid.to_string() == marker) {
                return Ok(None);
            }
            if id.get_or_insert_with(|| marker.to_owned()) != marker {
                return Ok(None);
            }
            found += 1;
        }
        if found != 1 {
            return Ok(None);
        }
    }
    let Some(installation_id) = id else {
        return Ok(None);
    };
    let owned = kind.entries(&owned_command(hook_argv, &installation_id)?)?;
    if exact_present(&value, &owned).len() != owned.len() {
        return Ok(None);
    }
    Ok(Some(Adoptable {
        installation_id,
        owned,
    }))
}

/// [`find_adoptable`] over the current file; `None` when the file does not exist.
pub fn adoptable_user_settings(
    kind: SettingsKind,
    config: &Path,
    hook_argv: &[String],
) -> Result<Option<Adoptable>, SetupError> {
    if config.file_name().and_then(|s| s.to_str()) != Some(kind.file_name()) {
        return Err(SetupError::Invalid);
    }
    if fs::symlink_metadata(config).is_err() {
        return Ok(None);
    }
    find_adoptable(kind, &config_bytes(config)?, hook_argv)
}

/// The restoration base of adopted groups: the file without them, dropping an event array they
/// alone filled, kept only when composing the groups back reproduces the file exactly (as the
/// original setup wrote it). Otherwise empty: removal then strips the groups structurally.
fn adoption_base(current: &[u8], owned: &[OwnedEntry]) -> Vec<u8> {
    let Ok(stripped) = uninstall_json(current, owned) else {
        return Vec::new();
    };
    let Ok(mut value) = root(&stripped) else {
        return Vec::new();
    };
    if let Some(hooks) = value.get_mut("hooks").and_then(Value::as_object_mut) {
        for entry in owned {
            if hooks
                .get(&entry.event)
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
            {
                hooks.remove(&entry.event);
            }
        }
    }
    match serde_json::to_vec(&value) {
        Ok(base)
            if compose_json(&base, owned, &[]).is_ok_and(|plan| plan.proposed_bytes == current) =>
        {
            base
        }
        _ => Vec::new(),
    }
}

/// Record a manifest for this file that adopts another setup's exact groups in place (see
/// [`find_adoptable`]). The hook file is never written, so Codex's trust hashes stay valid. A
/// Claude allow rule already present is recorded as pre-existing (never removed); an absent one
/// is left unrecorded so setup adds it as owned. `None` when a manifest exists or nothing is
/// adoptable.
pub fn adopt_user_settings(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    hook_argv: &[String],
) -> Result<Option<OwnershipManifest>, SetupError> {
    if manifest_path == config || read_manifest(manifest_path)?.is_some() {
        return Ok(None);
    }
    let Some(adoptable) = adoptable_user_settings(kind, config, hook_argv)? else {
        return Ok(None);
    };
    let current = config_bytes(config)?;
    let manifest = OwnershipManifest {
        version: 2,
        harness: kind.harness().into(),
        scope: kind.scope().into(),
        path: config.to_str().ok_or(SetupError::Invalid)?.into(),
        original_bytes: current.clone(),
        installation_base_bytes: adoption_base(&current, &adoptable.owned),
        installation_id: adoptable.installation_id,
        owned: adoptable.owned,
        installed_fingerprint: fingerprint(&current),
        phase: InstallPhase::Installed,
        superseded: Vec::new(),
        permission: None,
        superseded_permission: None,
        adopted: true,
    };
    if !recorded_ownership_valid(&manifest) {
        return Err(SetupError::Invalid);
    }
    let bytes = serde_json::to_vec(&manifest).map_err(|_| SetupError::Invalid)?;
    publish_manifest(manifest_path, &bytes, false)?;
    if config_bytes(config)? != current {
        let _ = fs::remove_file(manifest_path);
        return Err(SetupError::Conflict);
    }
    Ok(Some(manifest))
}

/// Codex hooks are session `-c` arguments; this records ownership without a global write.
pub fn codex_session_manifest(plan: &CodexSetupPlan) -> Result<OwnershipManifest, SetupError> {
    Ok(OwnershipManifest {
        version: 1,
        harness: "codex".into(),
        scope: "session".into(),
        path: "codex:-c".into(),
        original_bytes: Vec::new(),
        installation_base_bytes: Vec::new(),
        installation_id: String::new(),
        owned: plan.owned.clone(),
        installed_fingerprint: groups_fingerprint(&plan.events)?,
        phase: InstallPhase::Installed,
        superseded: Vec::new(),
        permission: None,
        superseded_permission: None,
        adopted: false,
    })
}

pub fn inspect_codex_session(
    events: &[EventGroups],
    manifest: &OwnershipManifest,
    observed: NativeObservation,
) -> Result<HookInspection, SetupError> {
    if manifest.version != 1
        || manifest.harness != "codex"
        || manifest.scope != "session"
        || manifest.path != "codex:-c"
    {
        return Err(SetupError::Invalid);
    }
    let owned_hooks = codex::DECLARATION.owned_hooks;
    let declaration_valid = manifest.owned.len() == owned_hooks.len()
        && owned_hooks
            .iter()
            .zip(&manifest.owned)
            .all(|(hook, entry)| {
                entry.event == hook.event
                    && entry.group["hooks"]
                        .as_array()
                        .is_some_and(|hooks| hooks.len() == 1)
                    && entry.group["hooks"][0]["type"] == "command"
                    && entry.group["hooks"][0]["command"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                    && entry.group.get("matcher").and_then(Value::as_str) == hook.matcher
                    && entry.group.get("matcher").is_none_or(Value::is_string)
            });
    let installed = declaration_valid
        && manifest.owned.iter().all(|entry| {
            events
                .iter()
                .find(|event| event.event == entry.event)
                .is_some_and(|event| event.groups.contains(&entry.group))
                && owned(&entry.event, entry.group.clone())
                    .is_ok_and(|actual| actual.fingerprint == entry.fingerprint)
        });
    let current_fingerprint = if installed {
        Some(groups_fingerprint(events)?)
    } else {
        None
    };
    Ok(HookInspection {
        installed,
        observed,
        configured_hook: current_fingerprint.map(|fingerprint| ConfiguredHook {
            scope: "session".into(),
            path: "codex:-c".into(),
            fingerprint,
        }),
        allow_rule: None,
        adopted: None,
        legacy_event_registration: false,
    })
}
/// Check the typed reader's exact structural input before future session overrides are published.
pub fn verify_codex_base(plan: &CodexSetupPlan, current: &[EventGroups]) -> Result<(), SetupError> {
    if groups_fingerprint(current)? == plan.base_fingerprint {
        Ok(())
    } else {
        Err(SetupError::Conflict)
    }
}

/// Validate local selectors before any adapter resolver or filesystem write.
pub fn validate_local_request(
    registration: Option<&super::registry::Registration>,
    install: bool,
    scope: &super::adapter::SetupScopeRequest,
    options: &super::adapter::SetupOptions,
) -> Result<(), super::adapter::SetupFailure> {
    use super::adapter::*;
    if let SetupScopeRequest::Profile(name) = scope {
        let registration = registration.ok_or_else(|| {
            SetupFailure::Invalid("--profile requires an explicitly selected adapter".into())
        })?;
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(SetupFailure::Invalid("invalid profile name".into()));
        }
        if !registration
            .metadata()
            .setup_scopes
            .iter()
            .any(|scope| matches!(scope, SetupScopeKind::Profile))
        {
            return Err(SetupFailure::Invalid(format!(
                "{}: named profile is unsupported",
                registration.metadata().id
            )));
        }
    }
    for (name, enabled) in options {
        if !install {
            return Err(SetupFailure::Invalid(
                "adapter options apply to setup only".into(),
            ));
        }
        let descriptor = registration
            .and_then(|registration| {
                registration
                    .setup_options()
                    .iter()
                    .find(|option| option.name == name)
            })
            .ok_or_else(|| SetupFailure::Invalid(format!("undeclared adapter option: {name}")))?;
        if !enabled {
            continue;
        }
        if descriptor
            .conflicts
            .iter()
            .any(|name| options.get(*name) == Some(&true))
        {
            return Err(SetupFailure::Invalid(format!(
                "conflicting adapter option: {name}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod adoption_tests {
    use super::*;

    fn argv() -> Vec<String> {
        ["/x/herdr-threads", "--state-dir", "/s", "hook", "codex"]
            .map(String::from)
            .to_vec()
    }

    fn marked(id: &str) -> Value {
        let entries = codex_entries(&owned_command(&argv(), id).unwrap()).unwrap();
        let mut hooks = serde_json::Map::new();
        for entry in entries {
            hooks.insert(entry.event, json!([entry.group]));
        }
        json!({ "hooks": hooks })
    }

    fn adoptable(value: &Value) -> Option<Adoptable> {
        find_adoptable(
            SettingsKind::CodexUser,
            &serde_json::to_vec(value).unwrap(),
            &argv(),
        )
        .unwrap()
    }

    /// Kills: adopting an unmarked copy, two owner markers, a duplicated or
    /// edited group, a missing event, or a different command; and an
    /// adoption base that does not restore the file without the groups.
    #[test]
    fn setup_adoption_requires_one_marker_and_exact_groups_for_every_event() {
        let id = uuid::Uuid::new_v4().to_string();
        let value = marked(&id);
        let found = adoptable(&value).expect("exact marked copy is adoptable");
        assert_eq!(found.installation_id, id);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(adoption_base(&bytes, &found.owned), b"{\"hooks\":{}}");

        let mut two = value.clone();
        two["hooks"]["PreToolUse"] =
            marked(&uuid::Uuid::new_v4().to_string())["hooks"]["PreToolUse"].clone();
        assert!(adoptable(&two).is_none());

        let mut duplicated = value.clone();
        let group = duplicated["hooks"]["SessionStart"][0].clone();
        duplicated["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(group);
        assert!(adoptable(&duplicated).is_none());

        let mut edited = value.clone();
        edited["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = json!(99);
        assert!(adoptable(&edited).is_none());

        let mut missing = value.clone();
        missing["hooks"]
            .as_object_mut()
            .unwrap()
            .remove("SubagentStart");
        assert!(adoptable(&missing).is_none());

        let mut unmarked = value.clone();
        unmarked["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(json!({"hooks":[{"type":"command","command":shell_command(&argv()).unwrap()}]}));
        assert!(adoptable(&unmarked).is_none());

        let other: Value = serde_json::from_str(
            &serde_json::to_string(&value)
                .unwrap()
                .replace("'/s'", "'/t'"),
        )
        .unwrap();
        assert!(adoptable(&other).is_none());
    }
}

/// Shared legacy-format transactions and compatibility projections.
pub(crate) mod legacy {
    use crate::cli::{
        RunError, hook,
        setup::{PromptSuggestionPolicy, SetupEnv, SetupRequest, SetupVerb},
    };
    use crate::{
        daemon::paths::{ensure_owned_state_root, ensure_private_dir},
        harness::{
            claude,
            context::Harness,
            setup::{
                self as lib, NativeObservation, SettingsKind, SetupError, adopt_user_settings,
                adoptable_user_settings, inspect_user_settings, inspect_user_settings_for,
                install_user_settings, read_settings_manifest, shared_command, shell_command,
            },
        },
        protocol::results::{ApiError, ErrorCode},
    };
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::{self, Write},
        path::{Path, PathBuf},
    };
    pub(crate) fn legacy_request(
        harness: Harness,
        verb: SetupVerb,
        native_binary: Option<&Path>,
        prompt_suggestions: PromptSuggestionPolicy,
    ) -> Result<SetupRequest, crate::harness::adapter::SetupFailure> {
        Ok(SetupRequest {
            scope: Default::default(),
            harness,
            verb,
            harness_binary: native_binary
                .map(|p| {
                    p.to_str().map(str::to_owned).ok_or_else(|| {
                        crate::harness::adapter::SetupFailure::Invalid(
                            "native binary path is not UTF-8".into(),
                        )
                    })
                })
                .transpose()?,
            prompt_suggestions,
            hooks_only: false,
            permissions: Default::default(),
        })
    }
    fn installer_failure(detail: impl Into<String>) -> crate::cli::RunError {
        crate::cli::RunError::Io(std::io::Error::other(detail.into()))
    }
    pub(crate) fn installer_hooks_installed(
        env: &SetupEnv,
        harness: Harness,
    ) -> Result<bool, crate::cli::RunError> {
        use crate::{
            cli::{installer_skill::read_optional, setup},
            harness::{
                codex_config,
                setup::{self as owned, NativeObservation, SettingsKind},
            },
        };
        let (kind, path, manifest) = match harness {
            Harness::Claude => {
                let (path, manifest) = setup::claude_paths(env)?;
                (SettingsKind::ClaudeUser, path, manifest)
            }
            Harness::Codex => {
                let paths = setup::codex_paths(env)?;
                let allowance = codex_config::inspect(&paths.config, &paths.config_manifest)
                    .map_err(|error| {
                        installer_failure(format!("invalid Codex allowance ownership: {error:?}"))
                    })?;
                if allowance.recorded.as_ref().is_some_and(|manifest| {
                    manifest.phase != owned::InstallPhase::Installed || !allowance.present
                }) {
                    return Err(installer_failure(
                        "recorded Codex allowance is partial or edited; preserved",
                    ));
                }
                (SettingsKind::CodexUser, paths.hooks, paths.hooks_manifest)
            }
            Harness::Human => return Err(installer_failure("human panes have no hooks")),
            _ => return Err(installer_failure("installer policy unavailable")),
        };
        let bytes = read_optional(&path)?;
        let argv = env.hook_argv(harness)?;
        let recorded = owned::read_settings_manifest(&manifest).map_err(|error| {
            installer_failure(format!("invalid hook ownership manifest: {error:?}"))
        })?;
        if let Some(recorded) = recorded {
            let current = bytes.as_deref().unwrap_or_default();
            let ownership = (|| -> Result<(), super::SetupError> {
                if recorded.phase != owned::InstallPhase::Installed
                    || !super::manifest_matches(kind, &recorded, &path)?
                    || !super::recorded_ownership_valid(&recorded)
                    || super::shared_command(&recorded.owned).as_deref()
                        != Some(super::owned_command(&argv, &recorded.installation_id)?.as_str())
                {
                    return Err(super::SetupError::Conflict);
                }
                super::uninstall_json(current, &recorded.owned)?;
                if kind.manages_permission()
                    && recorded
                        .permission
                        .as_ref()
                        .is_some_and(super::permission_current)
                    && super::allow_count(&super::root(current)?, super::DECLARED_RULE)? == 0
                {
                    return Err(super::SetupError::Conflict);
                }
                Ok(())
            })();
            ownership.map_err(|error| {
                installer_failure(format!(
                    "recorded hooks are partial or edited; preserved: {error:?}"
                ))
            })?;
            // Ownership of an older declaration permits a canonical upgrade; it grants no readiness.
            return Ok(true);
        }
        let inspection = owned::inspect_user_settings_for(
            kind,
            &path,
            &manifest,
            NativeObservation::Unknown,
            Some(&argv),
        )
        .map_err(|error| {
            installer_failure(format!("hook ownership inspection failed: {error:?}"))
        })?;
        if inspection.installed {
            return Ok(true);
        }
        if let Some(bytes) = bytes {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| installer_failure("invalid hook settings; preserved"))?;
            if !value.is_object() {
                return Err(installer_failure(
                    "hook settings are not an object; preserved",
                ));
            }
            if let Some(hooks) = value.get("hooks") {
                let map = hooks
                    .as_object()
                    .ok_or_else(|| installer_failure("invalid hooks map; preserved"))?;
                if map.values().any(|groups| !groups.is_array()) {
                    return Err(installer_failure("invalid hook groups; preserved"));
                }
            }
            // This conservative conflict check grants no ownership; canonical setup handles all writes.
            if value.get("hooks").is_some_and(foreign_hooks) {
                return Err(installer_failure(
                    "unowned or partial herdr-threads hooks; preserved (use setup-status to inspect)",
                ));
            }
        }
        Ok(false)
    }

    fn foreign_hooks(value: &Value) -> bool {
        match value {
            Value::Object(object) => object.iter().any(|(key, value)| {
                (key == "command" && value.as_str().is_some_and(|s| s.contains("herdr-threads")))
                    || foreign_hooks(value)
            }),
            Value::Array(array) => array.iter().any(foreign_hooks),
            _ => false,
        }
    }

    pub(crate) fn scoped_legacy_environment(
        harness: Harness,
        scope: &crate::harness::adapter::ResolvedSetupScope,
        snapshot: &crate::harness::adapter::SetupEnvironment,
    ) -> Result<SetupEnv, crate::harness::adapter::SetupFailure> {
        let crate::harness::adapter::ResolvedSetupScope::ConfigRoot(root) = scope else {
            return Err(crate::harness::adapter::SetupFailure::Invalid(format!(
                "{}: named profile is unsupported",
                harness.as_str()
            )));
        };
        let mut env = SetupEnv::from_snapshot(snapshot);
        match harness {
            Harness::Claude => env.claude_config_dir = Some(root.clone()),
            Harness::Codex => env.codex_home = Some(root.clone()),
            _ => {}
        }
        Ok(env)
    }
    pub(crate) fn legacy_diagnostics(
        projection: &Value,
    ) -> Vec<crate::harness::adapter::SetupDiagnostic> {
        use crate::harness::adapter::{DiagnosticSeverity, SetupDiagnostic};
        let mut diagnostics: Vec<_> = projection["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .take(16)
            .filter_map(Value::as_str)
            .map(|text| {
                SetupDiagnostic::new("legacy_setup_warning", DiagnosticSeverity::Warning, text)
            })
            .collect();
        if let Some(text) = projection["prompt_suggestions"]["note"].as_str() {
            diagnostics.push(SetupDiagnostic::new(
                "prompt_suggestions_advice",
                DiagnosticSeverity::Info,
                text,
            ));
        }
        if let Some(text) = projection["trust"]["note"].as_str() {
            diagnostics.push(SetupDiagnostic::new(
                "native_trust_manual",
                DiagnosticSeverity::Info,
                text,
            ));
        }
        diagnostics
    }

    pub(crate) fn legacy_adapter_setup(
        harness: Harness,
        request: &crate::harness::adapter::SetupRequest,
        install: fn(&SetupRequest, &SetupEnv) -> Result<Value, RunError>,
        prompt_suggestions: PromptSuggestionPolicy,
    ) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
        let mut env = scoped_legacy_environment(harness, &request.scope, &request.environment)?;
        env.executable = request.executable.clone();
        let mut legacy = legacy_request(
            harness,
            SetupVerb::Install,
            request.native_binary.as_deref(),
            prompt_suggestions,
        )?;
        // Only an adapter that declares the option (Claude) can receive it.
        legacy.hooks_only = request
            .options
            .get(crate::harness::claude::setup::HOOKS_ONLY_OPTION)
            == Some(&true);
        legacy.permissions = crate::cli::setup::PermissionPolicy::from_options(&request.options);
        harness_binary(&legacy, &env).map_err(adapter_failure)?;
        if let Some(leftover) = env.instance_source["state_dir_leftover"].as_str() {
            return Err(adapter_failure(invalid(format!(
                "{leftover}; nothing was changed"
            ))));
        }
        let mut projection = install(&legacy, &env).map_err(adapter_failure)?;
        crate::cli::setup::foreground::attach(&mut projection, harness, &env);
        let actions = vec![match projection["action"].as_str() {
            Some("installed") => crate::harness::adapter::SetupAction::InstalledOwned,
            Some("adopted") => crate::harness::adapter::SetupAction::AdoptedOwned,
            _ => crate::harness::adapter::SetupAction::Unchanged,
        }];
        let diagnostics = legacy_diagnostics(&projection);
        Ok(crate::harness::adapter::SetupOutcome {
            actions,
            diagnostic: String::new(),
            diagnostics,
            projection,
        })
    }
    pub(crate) fn legacy_adapter_unsetup(
        harness: Harness,
        request: &crate::harness::adapter::UnsetupRequest,
        remove: fn(&SetupEnv) -> Result<Value, RunError>,
    ) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure>
    {
        let env = scoped_legacy_environment(harness, &request.scope, &request.environment)?;
        let projection = remove(&env).map_err(adapter_failure)?;
        let actions = vec![if projection["action"] == "removed" {
            crate::harness::adapter::SetupAction::RemovedOwned
        } else {
            crate::harness::adapter::SetupAction::Unchanged
        }];
        let diagnostics = legacy_diagnostics(&projection);
        Ok(crate::harness::adapter::RemovalOutcome {
            actions,
            diagnostic: String::new(),
            residue: vec![],
            diagnostics,
            projection,
        })
    }
    pub(crate) fn legacy_adapter_status(
        harness: Harness,
        request: &crate::harness::adapter::StatusRequest,
        status: impl FnOnce(&SetupRequest, &SetupEnv) -> Result<Value, RunError>,
    ) -> crate::harness::adapter::SetupStatus {
        use crate::harness::adapter::*;
        let result = (|| {
            let env = scoped_legacy_environment(harness, &request.scope, &request.environment)?;
            let legacy = legacy_request(
                harness,
                SetupVerb::Status,
                request.native_binary.as_deref(),
                PromptSuggestionPolicy::Ask,
            )?;
            harness_binary(&legacy, &env).map_err(adapter_failure)?;
            let mut projection = status(&legacy, &env).map_err(adapter_failure)?;
            crate::cli::setup::foreground::attach(&mut projection, harness, &env);
            let configured_hook = user_inspection(harness, &env)
                .ok()
                .and_then(|(_, inspection)| inspection.configured_hook);
            let fingerprint = configured_hook
                .as_ref()
                .map(|hook| hook.fingerprint.clone());
            Ok(LocalSetupStatus {
                scope: request.scope.clone(),
                installed: projection["installed"].as_bool().unwrap_or(false),
                enabled: None,
                admitted: projection["harness_version"]["admission"]
                    .as_str()
                    .map(|admission| admission == "contract_declared"),
                observed: None,
                configured_hook,
                fingerprint,
                diagnostics: {
                    let mut diagnostics = legacy_diagnostics(&projection);
                    diagnostics.push(SetupDiagnostic::new(
                "native_enablement_unknown",
                DiagnosticSeverity::Info,
                "Native enablement and runtime observation are separate from local installation",
                ));
                    diagnostics
                },
                repairs: vec![
                    LocalRepair::InstallOwned,
                    LocalRepair::RepairOwned,
                    LocalRepair::RemoveOwned,
                ],
                projection,
            })
        })();
        match result {
            Ok(status) => SetupStatus::Detailed(Box::new(status)),
            Err(error) => SetupStatus::Failed(error),
        }
    }
    pub(crate) fn harness_binary(
        request: &SetupRequest,
        env: &SetupEnv,
    ) -> Result<Option<PathBuf>, RunError> {
        match &request.harness_binary {
            Some(binary) => {
                let binary = PathBuf::from(binary);
                if !binary.is_absolute() {
                    return Err(invalid("--harness-binary must be an absolute path"));
                }
                Ok(Some(binary))
            }
            None => Ok(hook::resolve_on_path(
                harness_name(request.harness),
                env.path.as_deref(),
            )),
        }
    }

    /// An accepted installed-version observation.
    #[derive(Debug, Clone)]
    pub(crate) struct Observed {
        pub(crate) binary: PathBuf,
        pub(crate) version: Option<String>,
        pub(crate) recipe: &'static str,
    }

    pub(crate) fn observation_json(
        result: &Result<(Observed, Option<crate::harness::operational::CodexContract>), String>,
    ) -> Value {
        match result {
            Ok((observed, _)) => json!({
                "admission": "contract_declared",
                "binary": observed.binary.display().to_string(),
                "version": observed.version,
                "recipe": observed.recipe,
            }),
            Err(refusal) => json!({"admission":"unavailable", "version":null, "refusal":refusal}),
        }
    }

    pub(crate) fn refuse_executable(refusal: String) -> RunError {
        api(ErrorCode::UnsupportedHarness, refusal)
    }

    // ------------------------------------------------------------ owned files

    /// Private manifest path for one legacy settings file.
    pub fn manifest_path(state_dir: &Path, kind: &str, file: &Path) -> PathBuf {
        let digest = format!("{:x}", Sha256::digest(file.as_os_str().as_encoded_bytes()));
        state_dir
            .join("setup")
            .join(format!("{kind}-{}.json", &digest[..32]))
    }

    /// Records files (and their directory) setup created, so unsetup can delete
    /// exactly those when they are back to their created state. Kept beside the
    /// manifest.
    pub(crate) fn created_marker_path(manifest: &Path) -> PathBuf {
        manifest.with_extension("created.json")
    }

    /// One file setup may create: its path, the bytes it is created with, and
    /// whether this invocation created it or its directory.
    pub(crate) struct OwnedFile {
        path: PathBuf,
        manifest: PathBuf,
        initial: &'static [u8],
        created_dir: bool,
        pub(crate) created_file: bool,
    }

    impl OwnedFile {
        pub(crate) fn new(path: PathBuf, manifest: PathBuf, initial: &'static [u8]) -> Self {
            Self {
                path,
                manifest,
                initial,
                created_dir: false,
                created_file: false,
            }
        }

        /// Create the file (and one missing directory level) with its initial
        /// bytes when absent. A non-directory parent refuses.
        pub(crate) fn prepare(&mut self) -> Result<(), RunError> {
            let dir = self.path.parent().expect("owned file parent");
            match fs::symlink_metadata(dir) {
                Ok(meta) if !meta.is_dir() && !fs::metadata(dir).is_ok_and(|m| m.is_dir()) => {
                    return Err(invalid(format!(
                        "{} exists but is not a directory",
                        dir.display()
                    )));
                }
                Ok(_) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(dir).map_err(|error| {
                        invalid(format!("could not create {}: {error}", dir.display()))
                    })?;
                    self.created_dir = true;
                }
                Err(error) => return Err(error.into()),
            }
            match fs::symlink_metadata(&self.path) {
                Ok(_) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&self.path)?
                        .write_all(self.initial)?;
                    self.created_file = true;
                    crate::harness::setup::note_created_user_config(&self.path);
                }
                Err(error) => return Err(error.into()),
            }
            Ok(())
        }

        /// Undo what [`prepare`](Self::prepare) created, unless a manifest now
        /// refers to it (a recorded preparation resumes).
        pub(crate) fn undo(&self) {
            if self.manifest.exists() {
                return;
            }
            if self.created_file && fs::read(&self.path).is_ok_and(|b| b == self.initial) {
                let _ = fs::remove_file(&self.path);
            }
            if self.created_dir
                && let Some(dir) = self.path.parent()
            {
                let _ = fs::remove_dir(dir);
            }
        }

        /// Record what was created; a warning when the record cannot be kept.
        pub(crate) fn record(&self, warnings: &mut Vec<String>) {
            if !(self.created_file || self.created_dir) {
                return;
            }
            let marker =
                json!({"settings_created": self.created_file, "dir_created": self.created_dir});
            let result = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode_private()
                .open(created_marker_path(&self.manifest))
                .and_then(|mut file| file.write_all(marker.to_string().as_bytes()));
            if result.is_err() {
                warnings.push(format!(
                    "could not record that setup created {}; unsetup will leave it",
                    self.path.display()
                ));
            }
        }
    }

    /// After removal: delete a file setup created once it is back to its
    /// created bytes (and its directory when setup created it and it is empty).
    pub(crate) fn delete_created(path: &Path, manifest: &Path, initial: &[u8]) -> bool {
        let marker_path = created_marker_path(manifest);
        let mut deleted = false;
        if let Ok(bytes) = fs::read(&marker_path) {
            let marker: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            if marker["settings_created"] == json!(true)
                && fs::read(path).is_ok_and(|b| b == initial)
                && fs::remove_file(path).is_ok()
            {
                deleted = true;
                if marker["dir_created"] == json!(true)
                    && let Some(dir) = path.parent()
                {
                    // Only an empty directory is removed.
                    let _ = fs::remove_dir(dir);
                }
            }
            let _ = fs::remove_file(&marker_path);
        }
        deleted
    }

    pub(crate) trait PrivateMode {
        fn mode_private(&mut self) -> &mut Self;
    }
    impl PrivateMode for fs::OpenOptions {
        fn mode_private(&mut self) -> &mut Self {
            use std::os::unix::fs::OpenOptionsExt;
            self.mode(0o600)
        }
    }

    /// The state directory, made ready to hold setup manifests.
    pub(crate) fn prepare_state(env: &SetupEnv) -> Result<&Path, RunError> {
        let state = env.state_dir()?;
        // A detected state directory follows Herdr's plugin state layout, whose
        // parents Herdr creates on the plugin's first action: create them if
        // setup runs first. An explicit one must already have its parent.
        if env.instance_source["state_dir"]
            .as_str()
            .is_some_and(|source| source.starts_with("herdr plugin list"))
            && let Some(parent) = state.parent()
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        ensure_owned_state_root(state)?;
        ensure_private_dir(&state.join("setup"))?;
        Ok(state)
    }

    pub(crate) fn settings_error(
        error: SetupError,
        verb: SetupVerb,
        kind: SettingsKind,
        settings: &Path,
        manifest: &Path,
    ) -> RunError {
        let settings = settings.display();
        let harness = kind.harness();
        match (error, verb) {
            (SetupError::Invalid, _) => invalid(format!(
                "{settings} is not a JSON object with an object `hooks` (and, for Claude, \
             `permissions` with an array `allow`), or it or the ownership manifest is a symlink \
             or damaged; nothing was changed"
            )),
            (SetupError::TooLarge, _) => invalid(format!(
                "{settings} or its ownership manifest exceeds the setup size bound; nothing was changed"
            )),
            (SetupError::Conflict, SetupVerb::Remove) => api(
                ErrorCode::Conflict,
                format!(
                    "the owned herdr-threads hook groups in {settings} were edited or removed, or the \
                 file changed during removal; nothing was changed. Restore the groups or remove \
                 them by hand, then delete the manifest {}",
                    manifest.display()
                ),
            ),
            (SetupError::Conflict, _) => api(
                ErrorCode::Conflict,
                format!(
                    "refused to install into {settings}: it changed during setup, already holds an \
                 unowned identical herdr-threads hook, holds an edited owned hook{rule}, or an \
                 existing installation uses a different hook command (executable, state \
                 directory or Herdr instance). Run `herdr-threads unsetup {harness}` (or remove \
                 the unowned hook) and set up again; nothing was changed",
                    rule = if kind == SettingsKind::ClaudeUser {
                        format!(
                            ", lacks the recorded permission allow rule `{}` (removed by hand)",
                            claude::HERDR_THREADS_ALLOW_RULE
                        )
                    } else {
                        String::new()
                    }
                ),
            ),
            (SetupError::Io, _) => failed(format!(
                "could not read or replace {settings} or its ownership manifest; if a manifest was \
             recorded, re-run the same command to resume"
            )),
        }
    }

    pub(crate) fn adopted_warning(kind: SettingsKind, file: &Path, owner: &str) -> String {
        format!(
            "{} already held the herdr-threads hook groups of another setup (owner marker {owner}, \
         e.g. a copied {}); they were adopted in place and the file was not changed{}. \
         `herdr-threads unsetup {}` here removes only these groups from this file",
            file.display(),
            kind.file_name(),
            if kind == SettingsKind::CodexUser {
                ", so Codex's recorded trust for them stays valid"
            } else {
                ""
            },
            kind.harness()
        )
    }

    pub(crate) fn adoption_json(adoption: Option<&lib::Adoption>) -> Value {
        match adoption {
            None => Value::Null,
            Some(adoption) => json!({
                "owner": adoption.owner,
                "recorded": adoption.recorded,
                "note": if adoption.recorded {
                    "the owned groups carry another setup's owner marker; this file's manifest \
                     adopted them in place"
                } else {
                    "the groups carry another setup's owner marker and match this command exactly \
                     (a copied hook file); `setup` adopts them without changing the file"
                },
            }),
        }
    }

    /// With no manifest for this file, adopt another setup's exact groups so `unsetup` can remove
    /// them; `None` when nothing is adoptable (or the hook command cannot be resolved).
    pub(crate) fn adopt_for_removal(
        kind: SettingsKind,
        env: &SetupEnv,
        file: &Path,
        manifest: &Path,
    ) -> Result<Option<lib::OwnershipManifest>, RunError> {
        let map = |e| settings_error(e, SetupVerb::Remove, kind, file, manifest);
        let Ok(argv) = env.hook_argv(match kind {
            SettingsKind::ClaudeUser => Harness::Claude,
            SettingsKind::CodexUser => Harness::Codex,
        }) else {
            return Ok(None);
        };
        // An unreadable or unparsable file has nothing adoptable: unsetup reports not installed.
        if !matches!(adoptable_user_settings(kind, file, &argv), Ok(Some(_))) {
            return Ok(None);
        }
        prepare_state(env)?;
        adopt_user_settings(kind, file, manifest, &argv).map_err(map)
    }

    /// The recorded hook command shared by every owned group: the base command, without the
    /// `--event <event>` pair each group registers it with.
    pub(crate) fn recorded_command(manifest: &Path) -> Option<String> {
        shared_command(&read_settings_manifest(manifest).ok().flatten()?.owned)
    }

    /// Setup output when a recorded registration without `--event` is rewritten to the per-event form.
    pub(crate) const EVENT_DOWNGRADE_WARNING: &str = "the hook commands now carry --event; a herdr-threads build \
     from before per-event registration rejects them, so to downgrade herdr-threads first run \
     `herdr-threads unsetup <harness>` with this build";

    /// Codex trusts hooks by hash, so rewritten commands need review again.
    pub(crate) const CODEX_RETRUST_WARNING: &str = "Codex trusts hooks by hash: the rewritten hook commands need \
     review again (the next interactive `codex` start, or /hooks) before Codex runs them";

    /// Install the owned hook groups into one user-level hook file.
    pub(crate) fn install_settings(
        kind: SettingsKind,
        verb: SetupVerb,
        env: &SetupEnv,
        file: &mut OwnedFile,
        warnings: &mut Vec<String>,
    ) -> Result<(lib::OwnershipManifest, bool, bool), RunError> {
        let argv = env.hook_argv(match kind {
            SettingsKind::ClaudeUser => Harness::Claude,
            SettingsKind::CodexUser => Harness::Codex,
        })?;
        let (path, manifest) = (file.path.clone(), file.manifest.clone());
        let map = |error| settings_error(error, verb, kind, &path, &manifest);
        let recorded = read_settings_manifest(&file.manifest).map_err(map)?;
        let before = recorded
            .is_some()
            .then(|| {
                inspect_user_settings(kind, &file.path, &file.manifest, NativeObservation::Unknown)
                    .ok()
            })
            .flatten();
        let already = before
            .as_ref()
            .is_some_and(|inspection| inspection.installed);
        let legacy = before
            .as_ref()
            .is_some_and(|inspection| inspection.legacy_event_registration);
        if recorded.is_none() {
            file.prepare()?;
        }
        let base = match &recorded {
            Some(recorded) => recorded.original_bytes.clone(),
            None => fs::read(&file.path).map_err(|error| {
                failed(format!("could not read {}: {error}", file.path.display()))
            })?,
        };
        match install_user_settings(kind, &file.path, &file.manifest, &argv, &base) {
            Ok(installed) => {
                file.record(warnings);
                if legacy {
                    let harness = kind.harness();
                    warnings.push(EVENT_DOWNGRADE_WARNING.replace("<harness>", harness));
                    if kind == SettingsKind::CodexUser {
                        warnings.push(CODEX_RETRUST_WARNING.to_owned());
                    }
                }
                // Adopted now: this run recorded another setup's groups in place, writing no hook.
                let adopted = recorded.is_none() && installed.adopted;
                if adopted {
                    warnings.push(adopted_warning(
                        kind,
                        &file.path,
                        &installed.installation_id,
                    ));
                }
                Ok((installed, already, adopted))
            }
            Err(error) => {
                file.undo();
                if error == SetupError::Conflict
                    && let Some(moved) =
                        moved_binary_conflict(kind, &path, recorded.as_ref(), &argv)
                {
                    return Err(moved);
                }
                Err(map(error))
            }
        }
    }

    /// The first word of a hook command built by `shell_command` (single-quoted, an embedded
    /// quote written `'\''`).
    pub(crate) fn first_shell_word(command: &str) -> Option<String> {
        let mut chars = command.strip_prefix('\'')?.chars().peekable();
        let mut word = String::new();
        while let Some(c) = chars.next() {
            if c != '\'' {
                word.push(c);
            } else if chars.clone().take(3).eq("\\''".chars()) {
                chars.nth(2);
                word.push('\'');
            } else {
                return Some(word);
            }
        }
        None
    }

    /// A re-setup refused because the recorded hook command names another executable than this
    /// one (the binary was moved, reinstalled elsewhere or run from a copy): the generic conflict
    /// message lists causes such as hand removal, which misleads here. Names both paths and the fix.
    pub(crate) fn moved_binary_conflict(
        kind: SettingsKind,
        file: &Path,
        recorded: Option<&lib::OwnershipManifest>,
        argv: &[String],
    ) -> Option<RunError> {
        let recorded_command = recorded?.owned.first()?.group["hooks"][0]["command"].as_str()?;
        let recorded_exe = first_shell_word(recorded_command)?;
        let current_exe = argv.first()?;
        if &recorded_exe == current_exe {
            return None;
        }
        let harness = kind.harness();
        Some(api(
            ErrorCode::Conflict,
            format!(
                "{} already holds the herdr-threads hooks of an installation that runs \
             `{recorded_exe}`, but this is `{current_exe}` (the binary moved, or this is another \
             copy). Run `herdr-threads unsetup {harness}` (from either binary: it removes the \
             recorded groups whatever executable they name), then `herdr-threads setup \
             {harness}` from the binary you want to keep; nothing was changed",
                file.display()
            ),
        ))
    }

    // ------------------------------------------------------------------ claude

    pub(crate) fn instance_json(env: &SetupEnv) -> Value {
        json!({
            "state_dir": env.state_dir.as_ref().map(|p| p.display().to_string()),
            "host_endpoint": env.host_endpoint().ok().map(|p| p.display().to_string()),
            "source": env.instance_source,
        })
    }

    pub(crate) fn settings_status(
        kind: SettingsKind,
        env: &SetupEnv,
        settings: &Path,
        report: &mut Value,
    ) -> Result<(), RunError> {
        let Ok(state) = env.state_dir() else {
            report["installed"] = json!(false);
            report["error"] = json!(format!(
                "state directory unknown: {}",
                env.instance_source["state_dir_error"]
                    .as_str()
                    .unwrap_or("use --state-dir or HERDR_PLUGIN_STATE_DIR")
            ));
            return Ok(());
        };
        let manifest = manifest_path(state, &format!("{}-user", kind.harness()), settings);
        report["manifest"] = json!(manifest.display().to_string());
        let argv = env
            .hook_argv(match kind {
                SettingsKind::ClaudeUser => Harness::Claude,
                SettingsKind::CodexUser => Harness::Codex,
            })
            .ok();
        let inspection = inspect_user_settings_for(
            kind,
            settings,
            &manifest,
            NativeObservation::Unknown,
            argv.as_deref(),
        )
        .map_err(|e| settings_error(e, SetupVerb::Status, kind, settings, &manifest))?;
        report["installed"] = json!(inspection.installed);
        report["adopted"] = adoption_json(inspection.adopted.as_ref());
        if kind == SettingsKind::ClaudeUser {
            report["allow_rule"] = claude::setup::allow_rule_json(inspection.allow_rule.as_ref());
        }
        report["observed"] = json!("unknown");
        if let Some(recorded) = read_settings_manifest(&manifest).ok().flatten() {
            report["recorded_phase"] = json!(match recorded.phase {
                lib::InstallPhase::Prepared => "prepared",
                lib::InstallPhase::Installed => "installed",
            });
        }
        if let Some(command) = recorded_command(&manifest) {
            report["command"] = json!(command);
        } else if let Some(adoption) = &inspection.adopted
            && let Some(argv) = &argv
            && let Ok(command) = shell_command(argv)
        {
            report["command"] = json!(format!(
                "{command} # herdr-threads-owner:{}",
                adoption.owner
            ));
        }
        if let Ok(argv) = env.hook_argv(match kind {
            SettingsKind::ClaudeUser => Harness::Claude,
            SettingsKind::CodexUser => Harness::Codex,
        }) && let Ok(expected) = shell_command(&argv)
        {
            report["current_command_matches"] = json!(
                report["command"]
                    .as_str()
                    .is_some_and(|recorded| recorded.starts_with(&format!("{expected} # ")))
            );
        }
        Ok(())
    }

    pub(crate) fn user_inspection(
        harness: Harness,
        env: &SetupEnv,
    ) -> Result<(PathBuf, lib::HookInspection), String> {
        let (kind, file) = match harness {
            Harness::Claude => (
                SettingsKind::ClaudeUser,
                env.claude_settings().map_err(|e| e.to_string())?,
            ),
            Harness::Codex => (
                SettingsKind::CodexUser,
                env.codex_file("hooks.json").map_err(|e| e.to_string())?,
            ),
            Harness::Human => return Err(NO_HUMAN_SETUP.to_owned()),
            _ => {
                return Err(format!(
                    "{}: setup inspection is unsupported",
                    harness.as_str()
                ));
            }
        };
        let state = env.state_dir().map_err(|e| e.to_string())?;
        let manifest = manifest_path(state, &format!("{}-user", kind.harness()), &file);
        let argv = env.hook_argv(harness).ok();
        let inspection = inspect_user_settings_for(
            kind,
            &file,
            &manifest,
            NativeObservation::Unknown,
            argv.as_deref(),
        )
        .map_err(|error| {
            format!(
                "the owned {} installation in {} cannot be inspected ({error:?}); run \
                 `herdr-threads setup-status {}`",
                kind.harness(),
                file.display(),
                kind.harness()
            )
        })?;
        Ok((file, inspection))
    }

    pub(crate) fn api(code: ErrorCode, detail: impl Into<String>) -> RunError {
        RunError::Api(ApiError::new(code, detail))
    }

    pub(crate) fn invalid(detail: impl Into<String>) -> RunError {
        api(ErrorCode::InvalidRequest, detail)
    }

    pub(crate) fn failed(detail: impl Into<String>) -> RunError {
        RunError::Io(io::Error::other(detail.into()))
    }

    pub(crate) fn adapter_failure(error: RunError) -> crate::harness::adapter::SetupFailure {
        use crate::harness::adapter::SetupFailure;
        match error {
            RunError::Api(e) => SetupFailure::Api(e),
            RunError::Io(e) => SetupFailure::Io(e),
            other => SetupFailure::Io(io::Error::other(other.to_string())),
        }
    }
    pub(crate) const NO_HUMAN_SETUP: &str =
        "setup manages agent hooks only; a person uses `herdr-threads human me init`";
    pub(crate) fn harness_name(harness: Harness) -> &'static str {
        harness.as_str()
    }
}
