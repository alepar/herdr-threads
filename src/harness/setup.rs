//! Scoped hook proposals and explicit user-settings installation (Claude `settings.json`, Codex
//! `hooks.json`). No process launch or trust change.
use super::{codex, launch::compose_native_argv_with};
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
    /// The launch line with no caller arguments: `--no-daemon` once, then the session `-c`
    /// overrides ([`CodexSetupPlan::launch_argv_for`] composes it for caller arguments).
    pub launch_argv: Vec<String>,
    /// The session `-c hooks.*=...` overrides alone (no `--no-daemon`).
    pub session_config: Vec<String>,
    pub owned: Vec<OwnedEntry>,
}

impl CodexSetupPlan {
    /// The launch argv for `caller` arguments, composed by the one launch rule
    /// ([`compose_native_argv_with`]): `--no-daemon` exactly once at the top level, none when
    /// `shell_passes_no_daemon` (the pane's `codex` function or alias supplies it; Codex
    /// refuses the flag twice), and a caller's own single `--no-daemon` kept in place.
    pub fn launch_argv_for(
        &self,
        caller: Vec<String>,
        shell_passes_no_daemon: bool,
    ) -> Result<Vec<String>, SetupError> {
        compose_native_argv_with(
            crate::protocol::authority::Harness::Codex,
            caller,
            self.session_config.clone(),
            shell_passes_no_daemon,
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
fn permission_record(pre_existing: bool) -> OwnedPermission {
    OwnedPermission {
        rule: DECLARED_RULE.into(),
        fingerprint: permission_fingerprint(DECLARED_RULE),
        pre_existing,
    }
}
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
fn root(bytes: &[u8]) -> Result<Value, SetupError> {
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
        let base = if recorded.is_empty() && rules_present.is_empty() {
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
/// Internal composition only: production callers go through `plan_codex_for_version`.
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
    plan.launch_argv = plan.launch_argv_for(Vec::new(), false)?;
    Ok(plan)
}

/// The production setup boundary. The witness exists only after observing the
/// installed binary report a version some recipe covers, so this cannot plan
/// unmeasured. The witness's recipe must still be a registered recipe.
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
    /// Claude project only: the recorded permission allow rule and whether it is present now.
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

/// Who holds the declared allow rule, as recorded by the ownership manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowRuleOwnership {
    /// Setup added the rule and unsetup removes it.
    Owned,
    /// The user's settings held the identical rule before setup; unsetup leaves it.
    PreExisting,
    /// The manifest records no rule (installed before the rule was declared): re-run setup.
    NotRecorded,
    /// The manifest records only the retired `claude::CALLER_CONTEXT_ALLOW_RULE`: re-run setup
    /// to replace it (an owned copy is removed) with the declared rule.
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
        let rule_count = allow_count(&root(&current)?, DECLARED_RULE)?;
        // A manifest recorded before the declared rule (none, or only the retired export rule)
        // gains it now. Earlier setups never wrote the declared rule, so an identical rule
        // already present is the user's own. A legacy unmarked installation cannot record the
        // intent safely: remove and set up again.
        // A kind without the allow rule has nothing to record: its permission is always current.
        let recorded_current = !kind.manages_permission()
            || manifest.permission.as_ref().is_some_and(permission_current);
        let permission = match &manifest.permission {
            _ if !kind.manages_permission() => None,
            Some(permission) if recorded_current => Some(permission.clone()),
            _ if manifest.installation_id.is_empty() => return Err(SetupError::Conflict),
            _ => Some(permission_record(rule_count > 0)),
        };
        // An owned retired rule is replaced: record it so an interruption still removes it.
        let retiring = manifest
            .permission
            .as_ref()
            .filter(|p| !permission_current(p) && !p.pre_existing)
            .cloned()
            .or_else(|| manifest.superseded_permission.clone());
        if manifest.phase == InstallPhase::Installed {
            // Every recorded owned group must still be exact before anything is claimed, and a
            // recorded declared rule must still be present (setup never silently re-adds a
            // removed rule). A retired rule already gone is fine: the upgrade removes it anyway.
            uninstall_json(&current, &manifest.owned)?;
            if kind.manages_permission() && recorded_current && rule_count == 0 {
                return Err(SetupError::Conflict);
            }
            if manifest.owned == expected && recorded_current {
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
            intent.permission = permission;
            intent.superseded_permission = retiring;
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
            manifest.permission = permission;
            manifest.superseded_permission = retiring;
            replace_manifest(manifest_path, &manifest)?;
        } else if !recorded_current {
            manifest.permission = permission;
            manifest.superseded_permission = retiring;
            replace_manifest(manifest_path, &manifest)?;
        }
        let resume = plan_resume(&current, &manifest, &unmarked_command)?;
        if resume.publish {
            if config_bytes(config)? != current {
                return Err(SetupError::Conflict);
            }
            write_replacement(config, &resume.bytes, false)?;
            if fault == InstallFault::InterruptAfterPublication {
                action.take().expect("one fault action")();
                return Err(SetupError::Io);
            }
        }
        manifest.phase = InstallPhase::Installed;
        manifest.installed_fingerprint = fingerprint(&resume.bytes);
        manifest.installation_base_bytes = resume.base;
        manifest.superseded.clear();
        manifest.superseded_permission = None;
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
    // Never duplicate the user's identical allow rule: record it as pre-existing instead.
    let permission = if kind.manages_permission() {
        Some(permission_record(
            allow_count(&baseline, DECLARED_RULE)? > 0,
        ))
    } else {
        None
    };
    let installation_id = uuid::Uuid::new_v4().to_string();
    let plan = compose_json(
        expected_base,
        &kind.entries(&owned_command(hook_argv, &installation_id)?)?,
        &permission
            .as_ref()
            .filter(|p| !p.pre_existing)
            .map(|p| p.rule.as_str())
            .into_iter()
            .collect::<Vec<_>>(),
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
        permission,
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
    write_replacement(config, &plan.proposed_bytes, false)?;
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
            let rule_present = root(&current)
                .and_then(|value| allow_count(&value, DECLARED_RULE))
                .is_ok_and(|count| count > 0);
            let installed = !kind.manages_permission() || rule_present;
            return Ok(HookInspection {
                installed,
                observed,
                configured_hook: installed.then(|| ConfiguredHook {
                    scope: kind.scope().into(),
                    path: config.to_string_lossy().into_owned(),
                    fingerprint: fingerprint(&current),
                }),
                allow_rule: kind.manages_permission().then_some(AllowRuleInspection {
                    rule: DECLARED_RULE,
                    ownership: if rule_present {
                        AllowRuleOwnership::PreExisting
                    } else {
                        AllowRuleOwnership::NotRecorded
                    },
                    present: rule_present,
                }),
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
    let rule = DECLARED_RULE;
    let rule_present = serde_json::from_slice::<Value>(&current)
        .ok()
        .and_then(|v| allow_count(&v, rule).ok())
        .is_some_and(|count| count > 0);
    let allow_rule = AllowRuleInspection {
        rule,
        ownership: match &manifest.permission {
            Some(p) if !permission_current(p) => AllowRuleOwnership::Superseded,
            Some(p) if p.pre_existing => AllowRuleOwnership::PreExisting,
            Some(_) => AllowRuleOwnership::Owned,
            None => AllowRuleOwnership::NotRecorded,
        },
        present: rule_present,
    };
    // Installed also means the declared allow rule (Claude) is recorded and present.
    let installed = manifest.phase == InstallPhase::Installed
        && declaration_valid
        && (!kind.manages_permission()
            || (manifest
                .permission
                .as_ref()
                .is_some_and(|p| permission_valid(p) && permission_current(p))
                && rule_present))
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
        allow_rule: kind.manages_permission().then_some(allow_rule),
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
    let removed = if manifest.phase == InstallPhase::Installed
        && fingerprint(&current) == manifest.installed_fingerprint
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
        write_replacement(config, &removed, false)?;
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
    let permission =
        if kind.manages_permission() && allow_count(&root(&current)?, DECLARED_RULE)? > 0 {
            Some(permission_record(true))
        } else {
            None
        };
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
        permission,
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
