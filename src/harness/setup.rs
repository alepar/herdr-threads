//! Scoped hook proposals and explicit user-settings installation (Claude `settings.json`, Codex
//! `hooks.json`). No process launch or trust change.
use crate::ports::ConfiguredHook;
use crate::store::connection::DurableSync;
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
/// Quote one complete stable event argv produced by
/// [`crate::cli::hook::installed_event_argv`]. This never appends an event,
/// canonicalizes an executable spelling, or adds an installation owner marker.
pub fn native_event_command(event_argv: &[String]) -> Result<String, SetupError> {
    let words = event_argv
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let parsed = crate::cli::hook::parse_hook_argv(&words)
        .ok_or(SetupError::Invalid)?
        .map_err(|_| SetupError::Invalid)?;
    if parsed.event.is_none()
        || event_argv.len() < 4
        || event_argv[event_argv.len() - 3] != crate::cli::hook::EVENT_SUBCOMMAND
    {
        return Err(SetupError::Invalid);
    }
    shell_command(event_argv)
}

/// The owner marker suffix a marked hook command carries.
const OWNER_MARKER: &str = " # herdr-threads-owner:";

/// Recognize only a canonical UUID in an actual trailing shell comment. Quoted path text
/// can contain the marker spelling without carrying installation identity.
fn owner_marker_index(command: &str) -> Option<usize> {
    let at = command.rfind(OWNER_MARKER)?;
    let suffix = &command[at + OWNER_MARKER.len()..];
    let id = uuid::Uuid::parse_str(suffix).ok()?;
    if id.to_string() != suffix {
        return None;
    }
    let mut quote = None;
    let mut escaped = false;
    for ch in command[..at].chars() {
        if escaped {
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            escaped = true;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        }
    }
    (quote.is_none() && !escaped).then_some(at)
}

/// The hook command registered for `event`: the base command with the two words
/// `'--event' '<event>'` inserted after the harness word, which is before the owner
/// marker when there is one (else appended).
pub(crate) fn event_command(base: &str, event: &str) -> String {
    let pair = format!(" '--event' '{event}'");
    match owner_marker_index(base) {
        Some(at) => format!("{}{pair}{}", &base[..at], &base[at..]),
        None => format!("{base}{pair}"),
    }
}

/// Inverse of [`event_command`]: the base command without its `--event <event>` pair. A command
/// that does not carry the pair (a legacy registration) is returned unchanged.
pub(crate) fn base_command<'a>(command: &'a str, event: &str) -> Cow<'a, str> {
    let pair = format!(" '--event' '{event}'");
    let end = owner_marker_index(command).unwrap_or(command.len());
    match command[..end].strip_suffix(pair.as_str()) {
        Some(head) => Cow::Owned(format!("{head}{}", &command[end..])),
        None => Cow::Borrowed(command),
    }
}

/// The `permissions.allow` member a settings kind owns.
#[derive(Debug, Clone, Copy)]
pub struct OwnedAllowSpec {
    /// The declared member (Claude: `HERDR_THREADS_ALLOW_RULE`).
    pub rule: &'static str,
    /// Members earlier setups installed; recognized in their manifests only, so re-setup can
    /// replace and unsetup can remove an owned copy (Claude: `[CALLER_CONTEXT_ALLOW_RULE]`).
    pub retired: &'static [&'static str],
    /// The native harness rewrites this file itself: a recorded member found missing is
    /// reported missing and re-added by setup instead of refusing (Claude: false).
    pub native_rewrites: bool,
}

/// The user-level settings file one adapter's installation edits through the shared JSON engine.
/// Both are JSON objects whose `hooks` map event names to arrays of matcher groups; a kind may
/// also own one `permissions.allow` member. A kind with no hook groups and an owned member is
/// allow-member-only.
#[derive(Debug)]
pub struct SettingsKindSpec {
    /// The adapter id: the manifest `harness` and the `<id>-user` manifest stem.
    pub harness: &'static str,
    /// The only file name an installation of this kind may edit.
    pub file_name: &'static str,
    /// Hook groups for a hook command, in declaration order; empty means allow-member-only.
    pub hook_groups: fn(&str) -> Vec<(&'static str, Value)>,
    pub owned_allow: Option<OwnedAllowSpec>,
    /// Appended to the adopted-groups warning (Codex: trust note).
    pub adoption_note: &'static str,
    /// Pushed when setup rewrites a pre-per-event registration (Codex: the re-trust warning).
    pub legacy_upgrade_warning: &'static str,
    /// Rules a separate permission component owns in this file, per `permissions` list (`allow`, `ask`), stripped when
    /// deciding whether the file is still the recorded base plus groups. Empty for a kind whose file holds none.
    pub permission_rules: fn() -> Vec<(&'static str, Vec<String>)>,
}

/// The `permission_rules` of a kind whose file holds no rules of a permission component.
pub fn no_permission_rules() -> Vec<(&'static str, Vec<String>)> {
    Vec::new()
}

/// A settings kind, compared by `(harness, file_name)`.
#[derive(Debug, Clone, Copy)]
pub struct SettingsKind(&'static SettingsKindSpec);
impl PartialEq for SettingsKind {
    fn eq(&self, other: &Self) -> bool {
        self.0.harness == other.0.harness && self.0.file_name == other.0.file_name
    }
}
impl Eq for SettingsKind {}
impl SettingsKind {
    pub const fn new(spec: &'static SettingsKindSpec) -> Self {
        Self(spec)
    }
    pub fn spec(self) -> &'static SettingsKindSpec {
        self.0
    }
    pub fn harness(self) -> &'static str {
        self.0.harness
    }
    pub fn scope(self) -> &'static str {
        "user"
    }
    /// The only file name an installation of this kind may edit.
    pub fn file_name(self) -> &'static str {
        self.0.file_name
    }
    /// Whether the installation owns a permission allow member.
    pub(crate) fn manages_permission(self) -> bool {
        self.0.owned_allow.is_some()
    }
    /// No hook groups, one owned allow member: the native harness owns the rest of the file.
    pub(crate) fn allow_member_only(self) -> bool {
        self.manages_permission() && (self.0.hook_groups)("").is_empty()
    }
    /// The declared allow member. Only meaningful when [`Self::manages_permission`].
    fn declared_rule(self) -> &'static str {
        self.0.owned_allow.as_ref().map_or("", |allow| allow.rule)
    }
    /// The retired allow member `rule` names, if the kind declares it.
    fn retired_rule(self, rule: &str) -> Option<&'static str> {
        self.0
            .owned_allow
            .as_ref()
            .and_then(|allow| allow.retired.iter().copied().find(|r| *r == rule))
    }
    fn native_rewrites(self) -> bool {
        self.0
            .owned_allow
            .as_ref()
            .is_some_and(|allow| allow.native_rewrites)
    }
    /// Owned entries derived from the adapter declaration, in declaration order.
    fn entries(self, command: &str) -> Result<Vec<OwnedEntry>, SetupError> {
        (self.0.hook_groups)(command)
            .into_iter()
            .map(|(event, group)| owned(event, group))
            .collect()
    }
    /// The settings file's declared allow rule, for inspection reports.
    pub(crate) fn allow_rule(self) -> Option<&'static str> {
        self.0.owned_allow.as_ref().map(|allow| allow.rule)
    }
}

/// Build complete native commands from the recognized setup route, using the registered
/// producer for each event. Legacy proposal callers with no hook argv retain their grammar.
pub(super) fn native_entries(
    kind: SettingsKind,
    argv: &[String],
) -> Result<Option<Vec<OwnedEntry>>, SetupError> {
    let words = argv
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let parsed = match crate::cli::hook::parse_hook_argv(&words) {
        Some(Ok(parsed)) => parsed,
        Some(Err(_)) if argv.iter().any(|a| a == crate::cli::hook::EVENT_SUBCOMMAND) => {
            return Err(SetupError::Invalid);
        }
        _ => return Ok(None),
    };
    if parsed.harness.as_str() != kind.harness() {
        return Err(SetupError::Invalid);
    }
    let state = parsed
        .state_dir
        .as_ref()
        .map(|p| p.to_str().ok_or(SetupError::Invalid))
        .transpose()?;
    let host = parsed
        .host_endpoint
        .as_ref()
        .map(|p| p.to_str().ok_or(SetupError::Invalid))
        .transpose()?;
    let entries = kind
        .entries(&shell_command(argv)?)?
        .into_iter()
        .map(|entry| {
            let event_argv = crate::cli::hook::installed_event_argv(
                &argv[0],
                state,
                host,
                parsed.harness,
                &entry.event,
            )
            .map_err(|_| SetupError::Invalid)?;
            let mut group = entry.group;
            group["hooks"][0]["command"] = json!(native_event_command(&event_argv)?);
            owned(&entry.event, group)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(entries))
}

/// Decode only our canonical shell encoding and the dedicated native grammar. Never infer a
/// route from arbitrary shell text or strip a suffix from an unrecognized command.
fn native_route(entry: &OwnedEntry) -> Option<String> {
    let command = entry_command(entry)?;
    native_command_route(command, &entry.event)
}

fn native_command_route(command: &str, event: &str) -> Option<String> {
    let argv = shlex::split(command)?;
    if native_event_command(&argv).ok()?.as_str() != command {
        return None;
    }
    let words = argv
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let parsed = crate::cli::hook::parse_hook_argv(&words)?.ok()?;
    if parsed.event.as_deref() != Some(event) {
        return None;
    }
    shell_command(&crate::cli::hook::installed_argv(
        &argv[0],
        parsed.state_dir.as_ref().and_then(|p| p.to_str()),
        parsed.host_endpoint.as_ref().and_then(|p| p.to_str()),
        parsed.harness,
    ))
    .ok()
}

/// Historical shared/evented registrations remain executable alongside stable hooks. Accept
/// only our canonical legacy encoding, optionally followed by a real UUID owner comment.
/// Recognition is diagnostic conflict evidence; exact manifest groups alone prove ownership.
fn legacy_command_route(command: &str, event: &str) -> Option<String> {
    let command = owner_marker_index(command).map_or(command, |at| &command[..at]);
    let argv = shlex::split(command)?;
    let words = argv
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let parsed = crate::cli::hook::parse_hook_argv(&words)?.ok()?;
    if parsed.event.as_deref().is_some_and(|name| name != event) {
        return None;
    }
    let route = shell_command(&crate::cli::hook::installed_argv(
        &argv[0],
        parsed.state_dir.as_ref().and_then(|p| p.to_str()),
        parsed.host_endpoint.as_ref().and_then(|p| p.to_str()),
        parsed.harness,
    ))
    .ok()?;
    let expected = if parsed.event.is_some() {
        event_command(&route, event)
    } else {
        route.clone()
    };
    (command == expected).then_some(route)
}

/// Exact per-event command strings for diagnostics. This is never an ownership grant.
pub(crate) fn commands_json(entries: &[OwnedEntry]) -> Value {
    Value::Object(
        entries
            .iter()
            .filter_map(|e| Some((e.event.clone(), json!(entry_command(e)?))))
            .collect(),
    )
}

fn ownership_route(entries: &[OwnedEntry]) -> Option<String> {
    let command = shared_command(entries)?;
    Some(
        owner_marker_index(&command).map_or_else(|| command.clone(), |at| command[..at].to_owned()),
    )
}

/// A recognized registration for our event would execute alongside this installation. Exact
/// manifest records are the only exceptions; unrelated native commands remain untouched.
fn conflicting_native_registration(
    value: &Value,
    kind: SettingsKind,
    target: &[OwnedEntry],
    recorded: &[OwnedEntry],
) -> bool {
    target.iter().any(|entry| {
        value["hooks"][&entry.event]
            .as_array()
            .is_some_and(|groups| {
                groups.iter().any(|group| {
                    !recorded
                        .iter()
                        .any(|owned| owned.event == entry.event && &owned.group == group)
                        && group["hooks"].as_array().is_some_and(|hooks| {
                            hooks.iter().any(|hook| {
                                hook["command"].as_str().is_some_and(|command| {
                                    native_command_route(command, &entry.event)
                                        .or_else(|| legacy_command_route(command, &entry.event))
                                        .and_then(|route| shlex::split(&route))
                                        .is_some_and(|argv| {
                                            argv.last()
                                                .is_some_and(|harness| harness == kind.harness())
                                        })
                                })
                            })
                        })
                })
            })
    })
}

pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// `path` with every symbolic link resolved: the deepest existing ancestor is canonicalized and
/// the rest of the path joined to it. Read-only.
pub(crate) fn canonical_path(path: &Path) -> std::io::Result<std::path::PathBuf> {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(mut canonical) => {
                canonical.extend(rest.iter().rev());
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let (Some(up), Some(name)) = (existing.parent(), existing.file_name()) else {
                    return Err(error);
                };
                rest.push(name);
                existing = up;
            }
            Err(error) => return Err(error),
        }
    }
}

/// The canonical location of `file` when it lies outside its config root once symbolic links
/// are resolved: a directory below `root` is a link to elsewhere, so the file is shared with
/// whatever root holds the real one. `None` when the resolved parent is still under the resolved
/// root (no link, or only `root` itself is a link, as with a dotfile manager). Read-only.
pub(crate) fn shared_through_symlink(
    root: &Path,
    file: &Path,
) -> std::io::Result<Option<std::path::PathBuf>> {
    let (Some(parent), Some(name)) = (file.parent(), file.file_name()) else {
        return Ok(None);
    };
    let linked = parent
        .ancestors()
        .take_while(|dir| dir.starts_with(root))
        .any(|dir| fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()));
    if !linked {
        return Ok(None);
    }
    let canonical_parent = canonical_path(parent)?;
    if canonical_parent.starts_with(canonical_path(root)?) {
        return Ok(None);
    }
    Ok(Some(canonical_parent.join(name)))
}

/// The ownership manifests of `kind` in the state directory's `setup/` (at most 512 of that
/// kind), for finding which recorded path owns a shared file. Read-only.
pub(crate) fn manifests_of_kind(state_dir: &Path, kind: &str) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(state_dir.join("setup")) else {
        return Vec::new();
    };
    let prefix = format!("{kind}-");
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                name.strip_prefix(&prefix)
                    .and_then(|rest| rest.strip_suffix(".json"))
                    .is_some_and(|digest| {
                        digest.len() == 32 && digest.bytes().all(|b| b.is_ascii_hexdigit())
                    })
            })
        })
        .map(|entry| entry.path())
        .take(512)
        .collect()
}
fn permission_fingerprint(rule: &str) -> String {
    fingerprint(&serde_json::to_vec(&json!({ "allow": rule })).unwrap_or_default())
}
fn permission_record(kind: SettingsKind, pre_existing: bool) -> OwnedPermission {
    let rule = kind.declared_rule();
    OwnedPermission {
        rule: rule.into(),
        fingerprint: permission_fingerprint(rule),
        pre_existing,
    }
}
/// A recorded permission is owned only for a rule this adapter declares or once declared, with a
/// matching fingerprint: a manifest can never direct removal of an arbitrary user rule.
fn permission_valid(kind: SettingsKind, permission: &OwnedPermission) -> bool {
    kind.0.owned_allow.as_ref().is_some_and(|allow| {
        (permission.rule == allow.rule || allow.retired.contains(&permission.rule.as_str()))
            && permission.fingerprint == permission_fingerprint(&permission.rule)
    })
}
fn permission_current(kind: SettingsKind, permission: &OwnedPermission) -> bool {
    permission.rule == kind.declared_rule()
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
fn uninstall_settings(
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
pub(crate) fn owned(event: &str, group: Value) -> Result<OwnedEntry, SetupError> {
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
pub(crate) fn merge(groups: &mut Vec<Value>, entry: &OwnedEntry) -> Result<(), SetupError> {
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
/// Compose every hook group a settings kind's adapter declares, in declaration order.
/// Declaring a hook claims neither native observation nor model-visible delivery.
pub fn plan_settings(
    kind: SettingsKind,
    bytes: &[u8],
    hook_argv: &[String],
) -> Result<JsonSetupPlan, SetupError> {
    if let Some(entries) = native_entries(kind, hook_argv)? {
        return compose_json(bytes, &entries, &[]);
    }
    let command = shell_command(hook_argv)?;
    compose_json(bytes, &kind.entries(&command)?, &[])
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
    if let Some(route) = native_route(first) {
        return entries
            .iter()
            .all(|entry| {
                entry.group["hooks"]
                    .as_array()
                    .is_some_and(|h| h.len() == 1)
                    && native_route(entry).as_deref() == Some(route.as_str())
            })
            .then_some(route);
    }
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
    if let Some(route) = entries.first().and_then(native_route) {
        return shlex::split(&route)
            .and_then(|argv| native_entries(kind, &argv).ok().flatten())
            .is_some_and(|declared| declared == entries);
    }
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
fn recorded_ownership_valid(kind: SettingsKind, manifest: &OwnershipManifest) -> bool {
    if kind.allow_member_only() {
        return member_manifest_valid(kind, manifest);
    }
    valid_owner_marker(manifest)
        && recorded_entries_valid(&manifest.owned)
        && manifest
            .permission
            .as_ref()
            .is_none_or(|p| permission_valid(kind, p))
        // A superseded rule exists only in a prepared upgrade: an owned retired rule being
        // replaced by the declared one.
        && manifest.superseded_permission.as_ref().is_none_or(|old| {
            manifest.phase == InstallPhase::Prepared
                && permission_valid(kind, old)
                && !permission_current(kind, old)
                && !old.pre_existing
                && manifest
                    .permission
                    .as_ref()
                    .is_some_and(|p| permission_current(kind, p))
        })
        && (manifest.superseded.is_empty()
            || (manifest.phase == InstallPhase::Prepared
                && recorded_entries_valid(&manifest.superseded)
                && ownership_route(&manifest.superseded) == ownership_route(&manifest.owned)))
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
    let stripped = uninstall_settings(current, entries, &rules, base)?;
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
fn without_permission_rules(
    kind: SettingsKind,
    current: &[u8],
    recorded_base: &[u8],
) -> Option<Vec<u8>> {
    let mut value = root(current).ok()?;
    let base = root(recorded_base).ok()?;
    let rendered = (kind.spec().permission_rules)();
    if let Some(permissions) = value.get_mut("permissions").and_then(Value::as_object_mut) {
        for (list, rules) in &rendered {
            if let Some(entries) = permissions.get_mut(*list).and_then(Value::as_array_mut) {
                entries.retain(|entry| {
                    entry
                        .as_str()
                        .is_none_or(|rule| !rules.iter().any(|r| r == rule))
                });
                if entries.is_empty() && base["permissions"].get(*list).is_none() {
                    permissions.remove(*list);
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
    kind: SettingsKind,
    current: &[u8],
    manifest: &OwnershipManifest,
    unmarked_command: &str,
) -> Result<Resume, SetupError> {
    let value = root(current)?;
    let present = exact_present(&value, &manifest.owned);
    let superseded = exact_present(&value, &manifest.superseded);
    for entry in manifest.owned.iter().chain(&manifest.superseded) {
        if has_entry_command(&value, entry) {
            // Declaration upgrades can change timeout/matcher while retaining a command.
            // Validate the exact recorded generation actually present, not the absent target.
            let exact = present
                .iter()
                .chain(&superseded)
                .find(|known| {
                    known.event == entry.event && entry_command(known) == entry_command(entry)
                })
                .ok_or(SetupError::Conflict)?;
            uninstall_json(current, std::slice::from_ref(exact))?;
        }
    }
    if manifest.version == 3
        && present
            .iter()
            .any(|entry| superseded.iter().any(|old| old.event == entry.event))
    {
        return Err(SetupError::Conflict);
    }
    shared_command(&manifest.owned).ok_or(SetupError::Invalid)?;
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
        value["hooks"][&entry.event]
            .as_array()
            .is_some_and(|groups| {
                groups.iter().any(|group| {
                    (contains_command(group, unmarked_command)
                        || contains_command(group, &event_command(unmarked_command, &entry.event)))
                        && !superseded
                            .iter()
                            .any(|old| old.event == entry.event && &old.group == group)
                })
            })
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
            && without_permission_rules(kind, current, recorded_base).is_some_and(|plain| {
                compose_json(recorded_base, &recorded, &rules_present)
                    .is_ok_and(|plan| plan.proposed_bytes == plain)
            });
        let base = if base_plus_grant {
            recorded_base.clone()
        } else if recorded.is_empty() && rules_present.is_empty() {
            current.to_vec()
        } else {
            prune_emptied_recorded_events(
                &uninstall_settings(current, &recorded, &rules_present, recorded_base)?,
                manifest,
                recorded_base,
            )?
        };
        let retiring: Vec<&str> = retired_present.into_iter().collect();
        let stripped = if superseded.is_empty() && retiring.is_empty() {
            current.to_vec()
        } else {
            prune_emptied_recorded_events(
                &uninstall_settings(current, &superseded, &retiring, recorded_base)?,
                manifest,
                recorded_base,
            )?
        };
        (
            base,
            compose_json(&stripped, &manifest.owned, &target_rules)?.proposed_bytes,
        )
    };
    let bytes = if manifest.version == 3 && !superseded.is_empty() {
        // Replace exact superseded groups in their slots. Unrelated groups retain their native
        // trust-key indices; an equivalent reinstall never reaches this publication path.
        let mut replaced = value.clone();
        for old in &superseded {
            let groups = replaced["hooks"][&old.event]
                .as_array_mut()
                .ok_or(SetupError::Invalid)?;
            let index = groups
                .iter()
                .position(|group| group == &old.group)
                .ok_or(SetupError::Conflict)?;
            if let Some(target) = manifest.owned.iter().find(|entry| entry.event == old.event) {
                groups[index] = target.group.clone();
            } else {
                groups.remove(index);
            }
        }
        let replaced = serde_json::to_vec(&replaced).map_err(|_| SetupError::Invalid)?;
        compose_json(&replaced, &manifest.owned, &target_rules)?.proposed_bytes
    } else {
        bytes
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
/// Check exact supplied configuration bytes again immediately before a future installer publishes.
pub fn verify_base(plan: &JsonSetupPlan, current: &[u8]) -> Result<(), SetupError> {
    if fingerprint(current) == plan.base_fingerprint {
        Ok(())
    } else {
        Err(SetupError::Conflict)
    }
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
pub(crate) fn groups_fingerprint(groups: &[EventGroups]) -> Result<String, SetupError> {
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
    /// This file retains an installation identity adopted from another setup's legacy marked
    /// groups: recorded by a manifest, or found by inspection with none yet.
    pub adopted: Option<Adoption>,
    /// The installed hooks use legacy shared or UUID-marked commands: they still work, and
    /// explicit `setup` migrates them to stable per-event commands.
    pub legacy_event_registration: bool,
    /// Required hook events (`required_hooks`) the recorded installation does not hold: a stale
    /// install, fixed by re-running `herdr-threads setup`. Empty for a current install and for
    /// one that is not installed at all.
    pub missing_required_events: Vec<String>,
}

/// The `required_hooks` events of `harness` that none of `installed` names.
pub fn missing_required_events<'a>(
    harness: &str,
    installed: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let installed: Vec<&str> = installed.into_iter().collect();
    super::registry::builtins()
        .registrations()
        .iter()
        .find(|registration| registration.metadata().id == harness)
        .map(|registration| registration.required_hook_events())
        .unwrap_or_default()
        .iter()
        .filter(|event| !installed.contains(event))
        .map(|event| (*event).to_owned())
        .collect()
}

/// Installation identity adopted from exact legacy UUID-marked groups, retained across migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoption {
    /// The retained installation id, originally carried by the legacy owner marker.
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
    /// The manifest records no rule (installed before the rule was declared): re-run setup.
    NotRecorded,
    /// The manifest records only a retired rule of the kind: re-run setup to replace it (an
    /// owned copy is removed) with the declared rule.
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowRuleInspection {
    pub rule: &'static str,
    pub ownership: AllowRuleOwnership,
    /// The rule currently appears in `permissions.allow`.
    pub present: bool,
}

/// Report form of the recorded allow rule of a settings kind; `null` when no manifest is recorded.
pub fn allow_rule_json(kind: SettingsKind, allow_rule: Option<&AllowRuleInspection>) -> Value {
    match allow_rule {
        None => Value::Null,
        Some(rule) => {
            let mut report = json!({
                "rule": rule.rule,
                "ownership": match rule.ownership {
                    AllowRuleOwnership::Owned => "owned",
                    AllowRuleOwnership::PreExisting => "pre_existing",
                    AllowRuleOwnership::NotRecorded => "not_recorded",
                    AllowRuleOwnership::Superseded => "superseded",
                },
                "present": rule.present,
            });
            if rule.ownership == AllowRuleOwnership::Superseded {
                let retired = kind
                    .spec()
                    .owned_allow
                    .as_ref()
                    .and_then(|allow| allow.retired.first())
                    .copied()
                    .unwrap_or_default();
                report["note"] = json!(format!(
                    "recorded with the retired rule `{retired}`; re-run `herdr-threads setup {}` \
                     to replace it with `{}`",
                    kind.harness(),
                    rule.rule
                ));
            }
            report
        }
    }
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
    /// The installation identity originated in exact legacy marked groups adopted in place.
    /// A later explicit stable migration retains this provenance while rewriting the commands.
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
        .and_then(|dir| dir.durable_sync())
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
        file.durable_sync().map_err(|_| SetupError::Io)?;
        fs::File::open(parent)
            .and_then(|dir| dir.durable_sync())
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
        file.durable_sync().map_err(|_| SetupError::Io)?;
        fs::rename(&temp, path).map_err(|_| SetupError::Io)?;
        fs::File::open(parent)
            .and_then(|dir| dir.durable_sync())
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
    if kind.allow_member_only() {
        return install_allow_member(
            kind,
            config,
            manifest_path,
            expected_base,
            fault,
            action.take(),
        );
    }
    let native = native_entries(kind, hook_argv)?;
    let unmarked_entries = native
        .clone()
        .unwrap_or(kind.entries(&shell_command(hook_argv)?)?);
    let unmarked_plan = compose_json(expected_base, &unmarked_entries, &[])?;
    let unmarked_legacy = legacy_entries(&unmarked_entries)?;
    if read_manifest(manifest_path)?.is_none() {
        // A file that already holds exactly this command's groups under another setup's owner
        // marker is adopted, not given a second set: only a manifest is recorded, and the
        // recorded-manifest path below then finds the installation complete.
        if config_bytes(config)? != expected_base {
            return Err(SetupError::Conflict);
        }
        // Another setup's groups for this command that are not adoptable (edited, duplicated)
        // refuse: a second owned set beside them would run the hook twice per event.
        if adopt_user_settings(kind, config, manifest_path, hook_argv)?.is_none()
            && holds_registrations(kind, expected_base, hook_argv)?
        {
            return Err(SetupError::Conflict);
        }
    }
    if let Some(mut manifest) = read_manifest(manifest_path)? {
        let expected_command = owned_command(hook_argv, &manifest.installation_id)?;
        let expected = native.clone().unwrap_or(kind.entries(&expected_command)?);
        // Ownership is validated against the manifest's own records. A different hook command
        // (argv) is not an upgrade: remove the installation first, then set up again.
        if !manifest_matches(kind, &manifest, config)?
            || !recorded_ownership_valid(kind, &manifest)
            || ownership_route(&manifest.owned).as_deref()
                != Some(shell_command(hook_argv)?.as_str())
            || manifest.original_bytes != expected_base
        {
            return Err(SetupError::Conflict);
        }
        if native.is_none()
            && manifest.installation_id.is_empty()
            && (manifest.phase == InstallPhase::Prepared || manifest.owned != expected)
        {
            return Err(SetupError::Conflict);
        }
        let unmarked_command = shell_command(hook_argv)?;
        let current = config_bytes(config)?;
        if native.is_some() {
            let mut recorded = manifest.owned.clone();
            recorded.extend(manifest.superseded.iter().cloned());
            if conflicting_native_registration(&root(&current)?, kind, &expected, &recorded) {
                return Err(SetupError::Conflict);
            }
        }
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
            if expected.iter().any(|entry| {
                !manifest.owned.contains(entry)
                    && !manifest.owned.iter().any(|old| {
                        old.event == entry.event && entry_command(old) == entry_command(entry)
                    })
                    && has_entry_command(&root(&current).unwrap_or(Value::Null), entry)
            }) {
                return Err(SetupError::Conflict);
            }
            let mut intent = manifest.clone();
            if native.is_some() {
                intent.version = 3;
                if intent.installation_id.is_empty() {
                    intent.installation_id = uuid::Uuid::new_v4().to_string();
                }
            }
            intent.superseded = manifest
                .owned
                .iter()
                .filter(|entry| !expected.contains(entry))
                .cloned()
                .collect();
            intent.owned = expected.clone();
            intent.phase = InstallPhase::Prepared;
            plan_resume(kind, &current, &intent, &unmarked_command)?;
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
            if native.is_some() {
                manifest.version = 3;
            }
            replace_manifest(manifest_path, &manifest)?;
        }
        let resume = plan_resume(kind, &current, &manifest, &unmarked_command)?;
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
    if native.is_some() && conflicting_native_registration(&baseline, kind, &unmarked_entries, &[])
    {
        return Err(SetupError::Conflict);
    }
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
        &native
            .clone()
            .unwrap_or(kind.entries(&owned_command(hook_argv, &installation_id)?)?),
        &[],
    )?;
    let manifest = OwnershipManifest {
        version: if native.is_some() { 3 } else { 2 },
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

/// A manifest of an allow-member-only installation: no owned hook groups, one recorded declared
/// member with a matching fingerprint, and a well-formed installation id.
fn member_manifest_valid(kind: SettingsKind, manifest: &OwnershipManifest) -> bool {
    manifest.owned.is_empty()
        && manifest.superseded.is_empty()
        && manifest.superseded_permission.is_none()
        && !manifest.adopted
        && uuid::Uuid::parse_str(&manifest.installation_id)
            .is_ok_and(|id| id.to_string() == manifest.installation_id)
        && manifest
            .permission
            .as_ref()
            .is_some_and(|p| permission_valid(kind, p) && permission_current(kind, p))
}

/// Install the owned allow member into a file the native harness otherwise owns. The recorded
/// base is the file as it was before the member was added, so removal prunes `allow` and
/// `permissions` only when that base lacked them. A recorded member that is missing again (the
/// harness rewrote the file) is re-added when the kind declares `native_rewrites`, else refused.
fn install_allow_member<F: FnOnce()>(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    expected_base: &[u8],
    fault: InstallFault,
    action: Option<F>,
) -> Result<OwnershipManifest, SetupError> {
    let mut action = action;
    let rule = kind.declared_rule();
    let current = config_bytes(config)?;
    if let Some(mut manifest) = read_manifest(manifest_path)? {
        if !manifest_matches(kind, &manifest, config)?
            || !recorded_ownership_valid(kind, &manifest)
            || manifest.original_bytes != expected_base
        {
            return Err(SetupError::Conflict);
        }
        let mut value = root(&current)?;
        let present = allow_count(&value, rule)? > 0;
        if present && manifest.phase == InstallPhase::Installed {
            return Ok(manifest);
        }
        if !present && manifest.phase == InstallPhase::Installed && !kind.native_rewrites() {
            return Err(SetupError::Conflict);
        }
        let bytes = if present {
            current.clone()
        } else {
            // The member was never published (interrupted install) or the harness rewrote the
            // file without it: this run adds it, so this run owns it and the file as it stands
            // now is the base removal restores.
            manifest.permission = Some(permission_record(kind, false));
            manifest.installation_base_bytes = current.clone();
            ensure_allow(&mut value, rule)?;
            let bytes = serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)?;
            if bytes.len() > 1_048_576 {
                return Err(SetupError::TooLarge);
            }
            write_user_config(config, &current, &bytes)?;
            bytes
        };
        manifest.phase = InstallPhase::Installed;
        manifest.installed_fingerprint = fingerprint(&bytes);
        replace_manifest(manifest_path, &manifest)?;
        return Ok(manifest);
    }
    if current != expected_base {
        return Err(SetupError::Conflict);
    }
    let mut value = root(expected_base)?;
    // Never duplicate the user's identical member: record it as pre-existing instead.
    let permission = permission_record(kind, allow_count(&value, rule)? > 0);
    if !permission.pre_existing {
        ensure_allow(&mut value, rule)?;
    }
    let proposed = serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)?;
    if proposed.len() > 1_048_576 {
        return Err(SetupError::TooLarge);
    }
    let manifest = OwnershipManifest {
        version: 2,
        harness: kind.harness().into(),
        scope: kind.scope().into(),
        path: config.to_str().ok_or(SetupError::Invalid)?.into(),
        original_bytes: expected_base.to_vec(),
        installation_base_bytes: expected_base.to_vec(),
        installation_id: uuid::Uuid::new_v4().to_string(),
        owned: Vec::new(),
        installed_fingerprint: fingerprint(if permission.pre_existing {
            expected_base
        } else {
            &proposed
        }),
        phase: InstallPhase::Prepared,
        superseded: Vec::new(),
        permission: Some(permission.clone()),
        superseded_permission: None,
        adopted: false,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SetupError::Invalid)?;
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
    if !permission.pre_existing {
        write_user_config(config, expected_base, &proposed)?;
    }
    let mut installed = manifest;
    installed.phase = InstallPhase::Installed;
    replace_manifest(manifest_path, &installed)?;
    Ok(installed)
}

/// Inspect an allow-member-only installation: installed means the recorded member is present now,
/// whether owned or pre-existing. The whole-file fingerprint is not consulted: the native harness
/// rewrites the file.
fn inspect_allow_member(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    observed: NativeObservation,
) -> Result<HookInspection, SetupError> {
    let Some(manifest) = read_manifest(manifest_path)? else {
        return Ok(HookInspection {
            installed: false,
            observed,
            configured_hook: None,
            allow_rule: None,
            adopted: None,
            legacy_event_registration: false,
            missing_required_events: Vec::new(),
        });
    };
    if !manifest_matches(kind, &manifest, config)? {
        return Err(SetupError::Conflict);
    }
    let current = config_bytes(config)?;
    let rule = kind.declared_rule();
    let rule_present = serde_json::from_slice::<Value>(&current)
        .ok()
        .and_then(|value| allow_count(&value, rule).ok())
        .is_some_and(|count| count > 0);
    let valid = recorded_ownership_valid(kind, &manifest);
    let installed = valid && manifest.phase == InstallPhase::Installed && rule_present;
    Ok(HookInspection {
        installed,
        observed,
        configured_hook: installed.then(|| ConfiguredHook {
            scope: kind.scope().into(),
            path: manifest.path.clone(),
            fingerprint: fingerprint(&current),
        }),
        allow_rule: Some(AllowRuleInspection {
            rule,
            ownership: match &manifest.permission {
                Some(p) if p.pre_existing => AllowRuleOwnership::PreExisting,
                Some(_) => AllowRuleOwnership::Owned,
                None => AllowRuleOwnership::NotRecorded,
            },
            present: rule_present,
        }),
        adopted: None,
        legacy_event_registration: false,
        missing_required_events: Vec::new(),
    })
}

/// Remove only the owned member (never a pre-existing one), pruning `allow` and `permissions`
/// only when the recorded base lacked them; then the manifest.
fn remove_allow_member(
    kind: SettingsKind,
    manifest: &OwnershipManifest,
    config: &Path,
    manifest_path: &Path,
) -> Result<(), SetupError> {
    if !recorded_ownership_valid(kind, manifest) {
        return Err(SetupError::Conflict);
    }
    let current = config_bytes(config)?;
    let removed = match owned_rule(manifest) {
        Some(rule) => remove_allow(&current, rule, &manifest.installation_base_bytes)?,
        None => current.clone(),
    };
    if config_bytes(config)? != current {
        return Err(SetupError::Conflict);
    }
    if removed != current {
        write_user_config(config, &current, &removed)?;
    }
    fs::remove_file(manifest_path).map_err(|_| SetupError::Io)
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
            .and_then(|_| file.durable_sync())
            .map_err(|_| SetupError::Io)?;
        fs::hard_link(&temp, path).map_err(|_| SetupError::Conflict)?;
        fs::File::open(parent)
            .and_then(|dir| dir.durable_sync())
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
    if !matches!(manifest.version, 1..=3) {
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

/// The permission record of `kind`'s hook manifest at `manifest_path` for `config`, validated
/// like removal: only a declared or retired rule with its fingerprint is ever owned.
pub fn recorded_hook_permission(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
) -> Result<Option<RecordedPermission>, SetupError> {
    let Some(manifest) = read_manifest(manifest_path)? else {
        return Ok(None);
    };
    if manifest.permission.is_none() && manifest.superseded_permission.is_none() {
        return Ok(None);
    }
    if !manifest_matches(kind, &manifest, config)? || !recorded_ownership_valid(kind, &manifest) {
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
    let marked = |entries: &[OwnedEntry]| {
        shared_command(entries)
            .is_some_and(|command| command.ends_with(&format!(" # herdr-threads-owner:{id}")))
    };
    if manifest.version == 3 {
        manifest
            .owned
            .iter()
            .all(|entry| native_route(entry).is_some())
            && (manifest.superseded.is_empty()
                || marked(&manifest.superseded)
                || manifest
                    .superseded
                    .iter()
                    .all(|entry| native_route(entry).is_some())
                // v1 manifests recorded exact groups before UUID identity existed. Their
                // canonical legacy route remains valid evidence during a prepared migration;
                // the new identity belongs to the stable target, not to the historical group.
                || shared_command(&manifest.superseded).is_some_and(|command| {
                    owner_marker_index(&command).is_none()
                        && shlex::split(&command).is_some_and(|argv| {
                            shell_command(&argv).is_ok_and(|canonical| canonical == command)
                                && argv.last().and_then(|id| legacy::registered_kind_in(crate::harness::registry::builtins(), id)).is_some_and(|kind| native_entries(kind, &argv).is_ok_and(|entries| entries.is_some()))
                        })
                }))
    } else {
        marked(&manifest.owned)
    }
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
    if kind.allow_member_only() {
        return inspect_allow_member(kind, config, manifest_path, observed);
    }
    let Some(manifest) = read_manifest(manifest_path)? else {
        if let Some(argv) = hook_argv
            // Best effort: a file that cannot be read or parsed is simply not installed here,
            // as it was before adoption existed.
            && let Ok(Some(adoptable)) = adoptable_user_settings(kind, config, argv)
            // An older copy lacking events declared since is not installed: setup upgrades it.
            && shell_command(argv)
                .and_then(|base| kind.entries(&base))
                .is_ok_and(|declared| declared.len() == adoptable.owned.len())
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
                missing_required_events: missing_required_events(
                    kind.harness(),
                    adoptable.owned.iter().map(|entry| entry.event.as_str()),
                ),
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
            missing_required_events: Vec::new(),
        });
    };
    if !manifest_matches(kind, &manifest, config)? {
        return Err(SetupError::Conflict);
    }
    let current = config_bytes(config)?;
    // Installed means every hook the adapter currently declares; an older subset is stale.
    let legacy = is_legacy_declaration(kind, &manifest.owned)
        || (manifest.version < 3
            && ownership_route(&manifest.owned)
                .and_then(|s| shlex::split(&s))
                .is_some_and(|argv| {
                    native_entries(kind, &argv).is_ok_and(|entries| entries.is_some())
                }));
    let declaration_valid = recorded_ownership_valid(kind, &manifest)
        && shared_command(&manifest.owned).is_some_and(|s| !s.is_empty())
        && (is_current_declaration(kind, &manifest.owned) || legacy);
    // A rule recorded by an earlier setup stays reported until the permission component
    // retires the record; current hook setup records none.
    let allow_rule = manifest.permission.as_ref().map(|p| {
        let current_rule = permission_current(kind, p);
        let rule = if current_rule {
            kind.declared_rule()
        } else {
            kind.retired_rule(&p.rule).unwrap_or_default()
        };
        AllowRuleInspection {
            rule,
            ownership: match p {
                _ if !current_rule => AllowRuleOwnership::Superseded,
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
        && uninstall_json(&current, &manifest.owned).is_ok()
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
        missing_required_events: missing_required_events(
            kind.harness(),
            manifest.owned.iter().map(|entry| entry.event.as_str()),
        ),
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
    if kind.allow_member_only() {
        return remove_allow_member(kind, &manifest, config, manifest_path);
    }
    let current = config_bytes(config)?;
    // Removal trusts the manifest's own recorded entries, never the current declaration, so an
    // installation made under an earlier declaration stays removable.
    if !recorded_ownership_valid(kind, &manifest) {
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
        let structurally_removed = uninstall_settings(&current, &manifest.owned, &rules, base)?;
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
            uninstall_settings(&current, &present, &rules, base)?
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
        uninstall_settings(&current, &manifest.owned, &rules, base)?
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

/// Whether `bytes` hold, under each declared event they register at all, exactly one group
/// running this hook command registered for that event (`--event <event>`)
/// apart from its owner marker, all with one valid marker, each exactly the declared group for
/// `<command> # herdr-threads-owner:<id>`. A copy made by an older setup lacks the events declared
/// since: it is adopted with the events it has (setup then upgrades it like a recorded older
/// declaration; `owned` lists only those). Any registration of the command that is unmarked or
/// edited ([`registrations`]), without `--event`, with another event's argument or under an
/// undeclared event, a second copy, two markers, or no event at all is not adoptable (those keep
/// their existing install/refusal behavior).
pub fn find_adoptable(
    kind: SettingsKind,
    bytes: &[u8],
    hook_argv: &[String],
) -> Result<Option<Adoptable>, SetupError> {
    let base = shell_command(hook_argv)?;
    let value = root(bytes)?;
    let declared: Vec<String> = kind
        .entries(&base)?
        .into_iter()
        .map(|entry| entry.event)
        .collect();
    let mut id: Option<String> = None;
    let mut events: Vec<String> = Vec::new();
    for registration in registrations(&value, &routes(kind, hook_argv)?) {
        // Every registration of this command must be an authentic marked per-event group under
        // its own declared event. An unmarked or edited one (another argument, extra words,
        // other spacing, a prefix), one without `--event`, or one under an event this adapter
        // does not declare would be left beside a second set.
        // A native registration is never another setup's adoptable legacy installation.
        let Some(marker) = registration
            .marker
            .filter(|_| registration.canonical && !registration.native)
        else {
            return Ok(None);
        };
        if registration.argument.as_deref() != Some(registration.event.as_str())
            || !declared.contains(&registration.event)
            || events.contains(&registration.event)
            || !uuid::Uuid::parse_str(&marker).is_ok_and(|uuid| uuid.to_string() == marker)
            || id.get_or_insert_with(|| marker.clone()) != &marker
        {
            return Ok(None);
        }
        events.push(registration.event);
    }
    let Some(installation_id) = id else {
        return Ok(None);
    };
    let owned: Vec<OwnedEntry> = kind
        .entries(&owned_command(hook_argv, &installation_id)?)?
        .into_iter()
        .filter(|entry| events.contains(&entry.event))
        .collect();
    if exact_present(&value, &owned).len() != owned.len() {
        return Ok(None);
    }
    Ok(Some(Adoptable {
        installation_id,
        owned,
    }))
}

/// The hook commands registered under one event array.
fn hook_commands<'a>(value: &'a Value, event: &str) -> impl Iterator<Item = &'a str> {
    value["hooks"][event]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(Value::as_str))
}

/// One registration of this hook command, wherever it sits.
struct Registration {
    /// The event array it is registered under.
    event: String,
    /// One of the spellings setup writes, nothing else: legacy `<base>[ '--event' '<X>']`,
    /// optionally followed by exactly ` # herdr-threads-owner:<marker>`, or the native
    /// `<native> '<X>'`.
    canonical: bool,
    /// The native per-event spelling (no owner marker).
    native: bool,
    /// Its event argument, when canonical with one.
    argument: Option<String>,
    /// Its owner marker, when canonical legacy with one.
    marker: Option<String>,
}

impl Registration {
    /// Exactly a bare spelling setup writes with no owner marker (legacy, or native for its own
    /// event): the unowned identical hook setup's own conflict handling already refuses.
    fn exact_unmarked(&self) -> bool {
        self.canonical
            && self.marker.is_none()
            && match self.argument.as_deref() {
                Some(argument) => argument == self.event,
                // The pre-per-event legacy spelling runs for any event; native always names one.
                None => !self.native,
            }
    }
}

/// The two command families of this hook: the legacy `base` (`... 'hook' '<harness>'`) and the
/// native per-event prefix (`... 'hook-event' '<harness>'`, without its event word).
struct Routes {
    base: String,
    native: Option<String>,
}

fn routes(kind: SettingsKind, hook_argv: &[String]) -> Result<Routes, SetupError> {
    let base = shell_command(hook_argv)?;
    let native = native_entries(kind, hook_argv)
        .ok()
        .flatten()
        .and_then(|entries| {
            let entry = entries.first()?;
            let suffix = format!(
                " {}",
                shell_command(std::slice::from_ref(&entry.event)).ok()?
            );
            entry_command(entry)?
                .strip_suffix(&suffix)
                .map(str::to_owned)
        });
    Ok(Routes { base, native })
}

/// Every registration of this hook under any event array: any command containing the full
/// quoted legacy command or native prefix, canonical or edited (a prefix, extra words, other
/// spacing, an owner comment missing or changed). Another executable, state directory or instance
/// has other routes and is not listed. Accepted limit: a copy whose argv was re-quoted (unquoted
/// or double-quoted words) no longer contains them and is not recognized.
fn registrations(value: &Value, routes: &Routes) -> Vec<Registration> {
    let Some(hooks) = value["hooks"].as_object() else {
        return Vec::new();
    };
    let base = routes.base.as_str();
    let mut found = Vec::new();
    for event in hooks.keys() {
        for command in hook_commands(value, event) {
            let edited = Registration {
                event: event.clone(),
                canonical: false,
                native: false,
                argument: None,
                marker: None,
            };
            if let Some(native) = routes.native.as_deref()
                && command.contains(native)
            {
                let argument = command
                    .strip_prefix(native)
                    .and_then(|rest| rest.strip_prefix(" '"))
                    .and_then(|rest| rest.strip_suffix('\''))
                    .filter(|argument| !argument.contains('\''));
                found.push(match argument {
                    Some(argument) => Registration {
                        canonical: true,
                        native: true,
                        argument: Some(argument.to_owned()),
                        ..edited
                    },
                    None => edited,
                });
                continue;
            }
            if !command.contains(base) {
                continue;
            }
            let parsed = command.strip_prefix(base).and_then(|rest| {
                let (argument, rest) = match rest.strip_prefix(" '--event' '") {
                    Some(tail) => {
                        let (argument, rest) = tail.split_once('\'')?;
                        (Some(argument.to_owned()), rest)
                    }
                    None => (None, rest),
                };
                let marker = match rest {
                    "" => None,
                    _ => Some(rest.strip_prefix(OWNER_MARKER)?.to_owned()),
                };
                Some((argument, marker))
            });
            found.push(match parsed {
                Some((argument, marker)) => Registration {
                    canonical: true,
                    argument,
                    marker,
                    ..edited
                },
                None => edited,
            });
        }
    }
    found
}

/// The event arrays under which `bytes` register this hook command more than once (for example a
/// second owned set written beside a copied older one, or an edited copy's group beside the
/// canonical one): every such registration runs on each event.
pub(crate) fn doubled_events(
    kind: SettingsKind,
    bytes: &[u8],
    hook_argv: &[String],
) -> Result<Vec<String>, SetupError> {
    let value = root(bytes)?;
    let mut doubled: Vec<String> = Vec::new();
    let found = registrations(&value, &routes(kind, hook_argv)?);
    for entry in &found {
        if !doubled.contains(&entry.event)
            && found
                .iter()
                .filter(|other| other.event == entry.event)
                .count()
                > 1
        {
            doubled.push(entry.event.clone());
        }
    }
    Ok(doubled)
}

/// The warning for [`doubled_events`].
pub(crate) fn doubled_warning(file: &Path, events: &[String]) -> String {
    format!(
        "{} registers the herdr-threads hook more than once for {} (marked groups of more \
         than one setup, e.g. left by an earlier release's setup over a copied file): each runs \
         on every event. Keep one group per event (the recorded owner's, when setup recorded \
         one) and remove the others by hand",
        file.display(),
        events.join(", ")
    )
}

/// Whether `bytes` register this hook command anywhere other than as the exact unmarked command
/// (which setup's conflict handling refuses on its own): another setup's marked groups or an edited
/// copy, adoptable or not.
pub(crate) fn holds_registrations(
    kind: SettingsKind,
    bytes: &[u8],
    hook_argv: &[String],
) -> Result<bool, SetupError> {
    if bytes.is_empty() {
        return Ok(false);
    }
    Ok(registrations(&root(bytes)?, &routes(kind, hook_argv)?)
        .iter()
        .any(|registration| !registration.exact_unmarked()))
}

/// `bytes` without the groups exactly equal to `entries` (a recorded installation's own groups),
/// so the rest can be checked for other registrations of the hook.
pub(crate) fn without_groups(bytes: &[u8], entries: &[OwnedEntry]) -> Result<Vec<u8>, SetupError> {
    let mut value = root(bytes)?;
    if let Some(hooks) = value.get_mut("hooks").and_then(Value::as_object_mut) {
        for (event, groups) in hooks.iter_mut() {
            if let Some(groups) = groups.as_array_mut() {
                groups.retain(|group| {
                    !entries
                        .iter()
                        .any(|entry| &entry.event == event && &entry.group == group)
                });
            }
        }
    }
    serde_json::to_vec(&value).map_err(|_| SetupError::Invalid)
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
    let manifest = adoption_manifest(kind, config, &current, adoptable)?;
    let bytes = serde_json::to_vec(&manifest).map_err(|_| SetupError::Invalid)?;
    publish_manifest(manifest_path, &bytes, false)?;
    if config_bytes(config)? != current {
        let _ = fs::remove_file(manifest_path);
        return Err(SetupError::Conflict);
    }
    Ok(Some(manifest))
}

/// The installed manifest that adopts `adoptable` in `config`, whose bytes are `current`.
fn adoption_manifest(
    kind: SettingsKind,
    config: &Path,
    current: &[u8],
    adoptable: Adoptable,
) -> Result<OwnershipManifest, SetupError> {
    let manifest = OwnershipManifest {
        version: 2,
        harness: kind.harness().into(),
        scope: kind.scope().into(),
        path: config.to_str().ok_or(SetupError::Invalid)?.into(),
        original_bytes: current.to_vec(),
        installation_base_bytes: adoption_base(current, &adoptable.owned),
        installation_id: adoptable.installation_id,
        owned: adoptable.owned,
        installed_fingerprint: fingerprint(current),
        phase: InstallPhase::Installed,
        superseded: Vec::new(),
        permission: None,
        superseded_permission: None,
        adopted: true,
    };
    if !recorded_ownership_valid(kind, &manifest) {
        return Err(SetupError::Invalid);
    }
    Ok(manifest)
}

/// A stale record: `manifest` records an installation of `config` none of whose recorded hook
/// commands (owned or superseded) is left anywhere in the file, which instead holds another
/// setup's groups for this hook command that [`find_adoptable`] accepts (one valid owner marker,
/// each group exactly the declared group for its own declared event, an older event subset
/// allowed). That is a hook file copied over ours: the recorded groups are gone and the groups
/// present are recognisably ours, so they are adopted in place of the record and setup upgrades
/// them. Anything else is not stale: a recorded command still present (exact or edited), a
/// record that owns a permission rule, unmarked, edited or duplicated copies, another
/// executable, state directory or instance, or an undeclared event keep their refusal.
pub fn stale_record_successor(
    kind: SettingsKind,
    manifest: &OwnershipManifest,
    config: &Path,
    bytes: &[u8],
    hook_argv: &[String],
) -> Result<Option<Adoptable>, SetupError> {
    if kind.allow_member_only()
        || !manifest_matches(kind, manifest, config)?
        || manifest.permission.is_some()
        || manifest.superseded_permission.is_some()
    {
        return Ok(None);
    }
    let recorded: Vec<&str> = manifest
        .owned
        .iter()
        .chain(&manifest.superseded)
        .filter_map(entry_command)
        .collect();
    if recorded.is_empty() {
        return Ok(None);
    }
    let value = root(bytes)?;
    let still_present = value["hooks"].as_object().is_some_and(|hooks| {
        hooks
            .keys()
            .any(|event| hook_commands(&value, event).any(|command| recorded.contains(&command)))
    });
    if still_present {
        return Ok(None);
    }
    find_adoptable(kind, bytes, hook_argv)
}

/// A stale record superseded in place by [`readopt_stale_record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readoption {
    /// The installation identity of the superseded record.
    pub stale_owner: String,
    /// Where the superseded record was kept (beside the manifest).
    pub stale_backup: std::path::PathBuf,
    /// The adopting manifest now recorded.
    pub manifest: OwnershipManifest,
}

/// Replace a stale record ([`stale_record_successor`]) with a manifest adopting the groups the
/// file holds. The hook file is not written here; the superseded record is kept beside the
/// manifest as `<manifest>.<UTC timestamp>-<uuid>.herdr-threads` and the manifest is replaced
/// atomically. `None` (nothing written) when there is no manifest or it is not stale.
pub fn readopt_stale_record(
    kind: SettingsKind,
    config: &Path,
    manifest_path: &Path,
    hook_argv: &[String],
) -> Result<Option<Readoption>, SetupError> {
    if manifest_path == config || fs::symlink_metadata(config).is_err() {
        return Ok(None);
    }
    let Some(stale) = read_manifest(manifest_path)? else {
        return Ok(None);
    };
    let current = config_bytes(config)?;
    let Some(adoptable) = stale_record_successor(kind, &stale, config, &current, hook_argv)? else {
        return Ok(None);
    };
    let manifest = adoption_manifest(kind, config, &current, adoptable)?;
    let stale_bytes = fs::read(manifest_path).map_err(|_| SetupError::Io)?;
    let stale_backup = backup_user_config(manifest_path, &stale_bytes)?;
    replace_manifest(manifest_path, &manifest)?;
    if config_bytes(config)? != current {
        // The file changed meanwhile: put the stale record back and refuse.
        write_replacement(manifest_path, &stale_bytes, true)?;
        return Err(SetupError::Conflict);
    }
    Ok(Some(Readoption {
        stale_owner: stale.installation_id,
        stale_backup,
        manifest,
    }))
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
    use crate::harness::codex;

    fn argv() -> Vec<String> {
        ["/x/herdr-threads", "--state-dir", "/s", "hook", "codex"]
            .map(String::from)
            .to_vec()
    }

    fn marked(id: &str) -> Value {
        let entries = codex::settings_kind()
            .entries(&owned_command(&argv(), id).unwrap())
            .unwrap();
        let mut hooks = serde_json::Map::new();
        for entry in entries {
            hooks.insert(entry.event, json!([entry.group]));
        }
        json!({ "hooks": hooks })
    }

    fn adoptable(value: &Value) -> Option<Adoptable> {
        find_adoptable(
            codex::settings_kind(),
            &serde_json::to_vec(value).unwrap(),
            &argv(),
        )
        .unwrap()
    }

    /// Kills: the native (`hook-event`) spelling invisible to the refusal and doubled
    /// diagnostics: an edited native command, one under another event's array, or a legacy
    /// marked group left beside a native one.
    #[test]
    fn native_registrations_are_classified() {
        let kind = codex::settings_kind();
        let native = native_entries(kind, &argv())
            .unwrap()
            .expect("native route");
        let mut hooks = serde_json::Map::new();
        for entry in &native {
            hooks.insert(entry.event.clone(), json!([entry.group]));
        }
        let value = json!({ "hooks": hooks });
        let bytes = |value: &Value| serde_json::to_vec(value).unwrap();
        // Exact native groups: the unowned identical case setup's own handling refuses.
        assert!(!holds_registrations(kind, &bytes(&value), &argv()).unwrap());
        assert!(
            doubled_events(kind, &bytes(&value), &argv())
                .unwrap()
                .is_empty()
        );
        assert!(adoptable(&value).is_none());
        let command = |value: &Value, event: &str| {
            value["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let mut edited = value.clone();
        edited["hooks"]["SessionStart"][0]["hooks"][0]["command"] =
            json!(format!("{} '--verbose'", command(&value, "SessionStart")));
        assert!(holds_registrations(kind, &bytes(&edited), &argv()).unwrap());
        let mut moved = value.clone();
        moved["hooks"]["SessionStart"][0]["hooks"][0]["command"] =
            json!(command(&value, "PreToolUse"));
        assert!(holds_registrations(kind, &bytes(&moved), &argv()).unwrap());
        // A bare legacy command moved under another event's array is not the unowned identical
        // hook setup's own handling refuses: it is counted.
        let mut moved_legacy = value.clone();
        moved_legacy["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(
                json!({"hooks": [{"type": "command", "command": event_command(
                    &shell_command(&argv()).unwrap(),
                    "SessionStart",
                )}]}),
            );
        assert!(holds_registrations(kind, &bytes(&moved_legacy), &argv()).unwrap());
        let mut own_legacy = value.clone();
        own_legacy["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(
                json!({"hooks": [{"type": "command", "command": event_command(
                    &shell_command(&argv()).unwrap(),
                    "PreToolUse",
                )}]}),
            );
        assert!(!holds_registrations(kind, &bytes(&own_legacy), &argv()).unwrap());
        // A recorded installation's own groups are excluded before checking the rest.
        let rest = without_groups(&bytes(&moved_legacy), &native).unwrap();
        assert!(holds_registrations(kind, &rest, &argv()).unwrap());
        assert!(
            !holds_registrations(
                kind,
                &without_groups(&bytes(&value), &native).unwrap(),
                &argv()
            )
            .unwrap()
        );
        let mut beside = value.clone();
        let legacy = marked(&uuid::Uuid::new_v4().to_string())["hooks"]["SessionStart"][0].clone();
        beside["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(legacy);
        assert_eq!(
            doubled_events(kind, &bytes(&beside), &argv()).unwrap(),
            ["SessionStart"]
        );
    }

    /// Kills: adopting an unmarked copy, two owner markers, a duplicated or
    /// edited group, a missing event, or a different command; and an
    /// adoption base that does not restore the file without the groups.
    #[test]
    fn setup_adoption_requires_one_marker_and_exact_groups_for_every_registered_event() {
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
        // An older copy lacking an event declared since is adopted with the events it has.
        let older = adoptable(&missing).expect("older marked copy is adoptable");
        assert_eq!(older.installation_id, id);
        assert!(!older.owned.is_empty());
        assert!(
            older
                .owned
                .iter()
                .all(|entry| entry.event != "SubagentStart")
        );
        assert_eq!(older.owned.len() + 1, found.owned.len());
        assert!(adoptable(&json!({"hooks": {}})).is_none());

        // An older copy whose group runs the hook with another event's argument, or with any
        // other edit before its marker, is an edit, not a missing event; so is a marked group
        // under an event this adapter does not declare.
        let kind = codex::settings_kind();
        let swap =
            |from: &'static str, to: &'static str| move |command: &str| command.replace(from, to);
        type Edit = Box<dyn Fn(&str) -> String>;
        let edits: Vec<(&str, Edit)> = vec![
            (
                "extra argument after",
                Box::new(swap("'PreToolUse' #", "'PreToolUse' '--verbose' #")),
            ),
            (
                "extra argument before",
                Box::new(swap("'--event'", "'--verbose' '--event'")),
            ),
            (
                "doubled space",
                Box::new(swap("'PreToolUse' #", "'PreToolUse'  #")),
            ),
            ("tab", Box::new(swap("'PreToolUse' #", "'PreToolUse'\t#"))),
            (
                "leading space",
                Box::new(|command: &str| format!(" {command}")),
            ),
            (
                "environment prefix",
                Box::new(|command: &str| format!("FOO=1 {command}")),
            ),
            (
                "unmarked extra argument",
                Box::new(|command: &str| {
                    let (head, _) = command.split_once(" # ").unwrap();
                    format!("{head} '--verbose'")
                }),
            ),
            (
                "unmarked trailing space",
                Box::new(|command: &str| format!("{} ", command.split_once(" # ").unwrap().0)),
            ),
        ];
        for (edit, apply) in &edits {
            let mut edited = missing.clone();
            let original = edited["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_owned();
            let command = apply(&original);
            assert_ne!(command, original, "{edit}");
            edited["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = json!(command);
            assert!(adoptable(&edited).is_none(), "{edit:?}");
            let bytes = serde_json::to_vec(&edited).unwrap();
            assert!(
                holds_registrations(kind, &bytes, &argv()).unwrap(),
                "{edit:?}"
            );
            // Beside the canonical group, the edited one is a second registration.
            let mut beside = value.clone();
            let group = edited["hooks"]["PreToolUse"][0].clone();
            beside["hooks"]["PreToolUse"]
                .as_array_mut()
                .unwrap()
                .push(group);
            let bytes = serde_json::to_vec(&beside).unwrap();
            assert_eq!(
                doubled_events(kind, &bytes, &argv()).unwrap(),
                ["PreToolUse"],
                "{edit:?}"
            );
        }
        let mut wrong = missing.clone();
        let text = serde_json::to_string(&wrong["hooks"]["PreToolUse"]).unwrap();
        wrong["hooks"]["PreToolUse"] = serde_json::from_str(
            &text.replace("'--event' 'PreToolUse'", "'--event' 'SessionStart'"),
        )
        .unwrap();
        assert!(adoptable(&wrong).is_none());
        let bytes = serde_json::to_vec(&wrong).unwrap();
        assert!(holds_registrations(kind, &bytes, &argv()).unwrap());
        let mut undeclared = missing.clone();
        undeclared["hooks"]["SomeOtherEvent"] = value["hooks"]["SubagentStart"].clone();
        assert!(adoptable(&undeclared).is_none());
        // A second registration under one array, whatever its argument, is doubled.
        let mut doubled = value.clone();
        let extra = wrong["hooks"]["PreToolUse"][0].clone();
        doubled["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(extra);
        let bytes = serde_json::to_vec(&doubled).unwrap();
        assert_eq!(
            doubled_events(kind, &bytes, &argv()).unwrap(),
            ["PreToolUse"]
        );
        assert!(
            doubled_events(kind, &serde_json::to_vec(&value).unwrap(), &argv())
                .unwrap()
                .is_empty()
        );
        // Another state directory's groups are not this command's.
        let foreign: Value = serde_json::from_str(
            &serde_json::to_string(&value)
                .unwrap()
                .replace("'/s'", "'/t'"),
        )
        .unwrap();
        let bytes = serde_json::to_vec(&foreign).unwrap();
        assert!(!holds_registrations(kind, &bytes, &argv()).unwrap());

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
            context::Harness,
            setup::{
                self as lib, NativeObservation, SettingsKind, SetupError, adopt_user_settings,
                adoptable_user_settings, allow_rule_json, inspect_user_settings,
                inspect_user_settings_for, install_user_settings, read_settings_manifest,
                shared_command, shell_command,
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
            pane_env: Default::default(),
        })
    }
    fn installer_failure(detail: impl Into<String>) -> crate::cli::RunError {
        crate::cli::RunError::Io(std::io::Error::other(detail.into()))
    }
    /// The settings kind `harness`'s adapter registers, if it registers one.
    pub(crate) fn registered_kind(harness: Harness) -> Option<SettingsKind> {
        registered_kind_in(crate::harness::registry::builtins(), harness.as_str())
    }

    /// The settings kind adapter `id` of `registry` registers, if it registers one.
    pub(crate) fn registered_kind_in(
        registry: &crate::harness::registry::Registry,
        id: &str,
    ) -> Option<SettingsKind> {
        let registration = registry.by_id(registry.agent(id).ok()?).ok()?;
        registration.settings_kind().map(SettingsKind::new)
    }

    /// The user settings file of `kind` (under its adapter's config root) and its manifest.
    pub(crate) fn settings_paths(
        env: &SetupEnv,
        kind: SettingsKind,
    ) -> Result<(PathBuf, PathBuf), RunError> {
        let file = env.config_root(kind.harness())?.join(kind.file_name());
        let manifest = manifest_path(env.state_dir()?, &format!("{}-user", kind.harness()), &file);
        Ok((file, manifest))
    }

    pub(crate) fn installer_hooks_installed(
        env: &SetupEnv,
        harness: Harness,
    ) -> Result<bool, crate::cli::RunError> {
        use crate::{
            cli::installer_skill::read_optional,
            harness::setup::{self as owned, NativeObservation},
        };
        let kind = match harness {
            Harness::Human => return Err(installer_failure("human panes have no hooks")),
            _ => registered_kind(harness)
                .ok_or_else(|| installer_failure("installer policy unavailable"))?,
        };
        let (path, manifest) = settings_paths(env, kind)?;
        // An adapter's own pre-checks (for example historical ownership records beside the
        // hook file) run at its installer call site, before this generic inspection.
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
                    || !super::recorded_ownership_valid(kind, &recorded)
                    || super::ownership_route(&recorded.owned).as_deref()
                        != Some(super::shell_command(&argv)?.as_str())
                {
                    return Err(super::SetupError::Conflict);
                }
                super::uninstall_json(current, &recorded.owned)?;
                if kind.manages_permission()
                    && recorded
                        .permission
                        .as_ref()
                        .is_some_and(|p| super::permission_current(kind, p))
                    && super::allow_count(&super::root(current)?, kind.declared_rule())? == 0
                {
                    return Err(super::SetupError::Conflict);
                }
                Ok(())
            })();
            if let Err(error) = ownership {
                // A stale record whose groups a copied hook file replaced with recognisably
                // owned ones: setup re-adopts and upgrades them (see `stale_record_successor`).
                if matches!(
                    owned::stale_record_successor(kind, &recorded, &path, current, &argv),
                    Ok(Some(_))
                ) {
                    return Ok(true);
                }
                return Err(installer_failure(format!(
                    "recorded hooks are partial or edited; preserved: {error:?}"
                )));
            }
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
        // Another setup's groups copied here by an older release: setup adopts and upgrades them.
        if inspection.installed
            || matches!(
                owned::adoptable_user_settings(kind, &path, &argv),
                Ok(Some(_))
            )
        {
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
        env.config_roots
            .insert(harness.as_str().to_owned(), root.clone());
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
        hooks_only: bool,
    ) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
        let mut env = scoped_legacy_environment(harness, &request.scope, &request.environment)?;
        env.executable = request.executable.clone();
        let mut legacy = legacy_request(
            harness,
            SetupVerb::Install,
            request.native_binary.as_deref(),
            prompt_suggestions,
        )?;
        legacy.hooks_only = hooks_only;
        legacy.permissions = crate::cli::setup::PermissionPolicy::from_options(&request.options);
        legacy.pane_env = crate::cli::setup::PaneEnvPolicy::from_options(&request.options);
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

    pub(crate) fn observation_json<C>(result: &Result<(Observed, Option<C>), String>) -> Value {
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
                "{settings} is not a JSON object with an object `hooks`{permissions}, or it or the \
             ownership manifest is a symlink or damaged; nothing was changed",
                permissions = if kind.manages_permission() {
                    format!(
                        " (and, for {}, `permissions` with an array `allow`)",
                        display_label(kind)
                    )
                } else {
                    String::new()
                }
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
                 unowned identical herdr-threads hook, holds a conflicting herdr-threads \
                 registration for the same event, holds an edited owned hook{rule}, or an \
                 existing installation uses a different hook command (executable, state \
                 directory or Herdr instance). Run `herdr-threads unsetup {harness}` (or remove \
                 the unowned hook) and set up again; nothing was changed",
                    rule = if let Some(rule) = kind.allow_rule() {
                        format!(
                            ", lacks the recorded permission allow rule `{rule}` (removed by hand)"
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

    /// The display label of the adapter a settings kind belongs to.
    fn display_label(kind: SettingsKind) -> &'static str {
        let registry = crate::harness::registry::builtins();
        registry
            .agent(kind.harness())
            .ok()
            .and_then(|id| registry.by_id(id).ok())
            .map_or(kind.harness(), |registration| {
                registration.metadata().display_label
            })
    }

    pub(crate) fn adopted_warning(kind: SettingsKind, file: &Path, owner: &str) -> String {
        format!(
            "{} already held the herdr-threads hook groups of another setup (owner marker {owner}, \
         e.g. a copied {}); they were adopted in place and the file was not changed{}. \
         `herdr-threads unsetup {}` here removes only these groups from this file",
            file.display(),
            kind.file_name(),
            kind.spec().adoption_note,
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
                    "this file's manifest retains an installation identity adopted from another \
                     setup's legacy UUID-marked groups"
                } else {
                    "the groups carry another setup's owner marker and match this command exactly \
                     (a copied hook file); `setup` adopts them without changing the file"
                },
            }),
        }
    }

    /// The installation `unsetup` removes from `file`: its recorded manifest, a stale record
    /// replaced by the groups the file holds ([`lib::readopt_stale_record`]), or, with no manifest,
    /// another setup's adoptable groups ([`adopt_for_removal`]). A superseded stale record is
    /// reported under `report["superseded_record"]`.
    pub(crate) fn recorded_for_removal(
        kind: SettingsKind,
        env: &SetupEnv,
        file: &Path,
        manifest: &Path,
        report: &mut Value,
    ) -> Result<Option<lib::OwnershipManifest>, RunError> {
        let map = |e| settings_error(e, SetupVerb::Remove, kind, file, manifest);
        if let Ok(argv) = env.hook_argv(kind_harness(kind))
            && let Some(readopted) =
                lib::readopt_stale_record(kind, file, manifest, &argv).map_err(map)?
        {
            report["superseded_record"] = json!({
                "owner": readopted.stale_owner,
                "kept_at": readopted.stale_backup.display().to_string(),
                "note": "the file no longer held the recorded groups; its herdr-threads groups \
                         (owner marker in `adopted`) were adopted in place of that record",
            });
            return Ok(Some(readopted.manifest));
        }
        match read_settings_manifest(manifest).map_err(map)? {
            Some(recorded) => Ok(Some(recorded)),
            None => adopt_for_removal(kind, env, file, manifest),
        }
    }

    /// The setup warning for a stale record superseded by [`lib::readopt_stale_record`].
    pub(crate) fn readopted_warning(file: &Path, readoption: &lib::Readoption) -> String {
        format!(
            "{} no longer held the hook groups recorded for installation {} (the file was \
             replaced, e.g. copied from another setup); its herdr-threads groups with owner marker \
             {} were adopted in place of that record and upgraded. Other hooks were kept; the \
             superseded record is kept at {}",
            file.display(),
            readoption.stale_owner,
            readoption.manifest.installation_id,
            readoption.stale_backup.display()
        )
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
        let Ok(argv) = env.hook_argv(kind_harness(kind)) else {
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
    pub(crate) const EVENT_DOWNGRADE_WARNING: &str = "the hook commands now use hook-event; a herdr-threads build \
     from before per-event registration rejects them, so to downgrade herdr-threads first run \
     `herdr-threads unsetup <harness>` with this build";

    /// Install the owned hook groups into one user-level hook file.
    pub(crate) fn install_settings(
        kind: SettingsKind,
        verb: SetupVerb,
        env: &SetupEnv,
        file: &mut OwnedFile,
        warnings: &mut Vec<String>,
    ) -> Result<(lib::OwnershipManifest, bool, bool), RunError> {
        let argv = env.hook_argv(kind_harness(kind))?;
        let (path, manifest) = (file.path.clone(), file.manifest.clone());
        let map = |error| settings_error(error, verb, kind, &path, &manifest);
        // A stale record whose groups a copied file replaced with recognisably owned ones is
        // superseded first; the upgrade below then replaces those groups.
        let readopted =
            lib::readopt_stale_record(kind, &file.path, &file.manifest, &argv).map_err(map)?;
        if let Some(readoption) = &readopted {
            warnings.push(readopted_warning(&file.path, readoption));
        }
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
            .is_some_and(|inspection| inspection.legacy_event_registration)
            || readopted.is_some()
            || (recorded.is_none()
                && matches!(
                    adoptable_user_settings(kind, &file.path, &argv),
                    Ok(Some(_))
                ));
        if recorded.is_none() {
            file.prepare()?;
        }
        let base = match &recorded {
            Some(recorded) => recorded.original_bytes.clone(),
            None => fs::read(&file.path).map_err(|error| {
                failed(format!("could not read {}: {error}", file.path.display()))
            })?,
        };
        // Another setup's groups for this command that cannot be adopted: unsetup cannot remove
        // them either, so name the hand edit instead of the generic conflict advice.
        // A symlinked file keeps its own refusal (setup never follows the link).
        if recorded.is_none()
            && !fs::symlink_metadata(&file.path).is_ok_and(|meta| meta.file_type().is_symlink())
            && lib::holds_registrations(kind, &base, &argv).unwrap_or(false)
            && !matches!(lib::find_adoptable(kind, &base, &argv), Ok(Some(_)))
        {
            file.undo();
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} holds herdr-threads hook registrations setup cannot adopt: another \
                     setup's groups or copies of the hook command that are edited, duplicated, \
                     unmarked or registered without `--event`. \
                     Setup does not add a second set beside them, and unsetup does not own them: \
                     remove those groups from the file by hand, then set up again; nothing was \
                     changed",
                    file.path.display()
                ),
            ));
        }
        // Beside a recorded installation: any other registration of the hook that setup's own
        // conflict handling does not refuse (edited, moved, doubled) is refused the same way
        // before the recorded groups are upgraded or migrated next to it.
        if let Some(recorded) = &recorded
            && !fs::symlink_metadata(&file.path).is_ok_and(|meta| meta.file_type().is_symlink())
            && let Ok(current) = fs::read(&file.path)
            && let Ok(rest) = lib::without_groups(
                &current,
                &[recorded.owned.as_slice(), recorded.superseded.as_slice()].concat(),
            )
            && lib::holds_registrations(kind, &rest, &argv).unwrap_or(false)
        {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} holds herdr-threads hook registrations beside the recorded installation \
                     that setup does not own: copies of the hook command that are edited, \
                     moved to another event or duplicated (or a recorded group whose command was \
                     edited). Setup does not add or migrate groups beside them: remove or restore \
                     them by hand, then set up again; nothing was changed",
                    file.path.display()
                ),
            ));
        }
        let previous_bytes = fs::read(&file.path)
            .map_err(|error| failed(format!("could not read {}: {error}", file.path.display())))?;
        match install_user_settings(kind, &file.path, &file.manifest, &argv, &base) {
            Ok(installed) => {
                file.record(warnings);
                let unchanged = fs::read(&file.path).is_ok_and(|bytes| bytes == previous_bytes);
                if legacy && !unchanged {
                    let harness = kind.harness();
                    warnings.push(EVENT_DOWNGRADE_WARNING.replace("<harness>", harness));
                    if !kind.spec().legacy_upgrade_warning.is_empty() {
                        warnings.push(kind.spec().legacy_upgrade_warning.to_owned());
                    }
                }
                // Adopted now: this run recorded another setup's groups in place, writing no hook.
                let adopted = recorded.is_none() && installed.adopted && unchanged;
                if adopted {
                    warnings.push(adopted_warning(
                        kind,
                        &file.path,
                        &installed.installation_id,
                    ));
                }
                if let Ok(bytes) = fs::read(&file.path)
                    && let Ok(doubled) = lib::doubled_events(kind, &bytes, &argv)
                    && !doubled.is_empty()
                {
                    warnings.push(lib::doubled_warning(&file.path, &doubled));
                }
                Ok((installed, already && unchanged, adopted))
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
        let argv = env.hook_argv(kind_harness(kind)).ok();
        let inspection = inspect_user_settings_for(
            kind,
            settings,
            &manifest,
            NativeObservation::Unknown,
            argv.as_deref(),
        )
        .map_err(|e| settings_error(e, SetupVerb::Status, kind, settings, &manifest))?;
        report["installed"] = json!(inspection.installed);
        if let Some(stale) = crate::harness::required_hooks::stale_install_json(
            kind.harness(),
            &inspection.missing_required_events,
        ) {
            report["stale_install"] = stale;
        }
        report["adopted"] = adoption_json(inspection.adopted.as_ref());
        if kind.manages_permission() {
            report["allow_rule"] = allow_rule_json(kind, inspection.allow_rule.as_ref());
        }
        report["observed"] = json!("unknown");
        let recorded = read_settings_manifest(&manifest).ok().flatten();
        let target = recorded
            .as_ref()
            .map(|m| m.owned.clone())
            .or_else(|| {
                argv.as_ref()
                    .and_then(|argv| super::native_entries(kind, argv).ok().flatten())
            })
            .unwrap_or_default();
        let owned = recorded
            .as_ref()
            .map(|m| {
                m.owned
                    .iter()
                    .chain(&m.superseded)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        report["registration_conflict"] = super::config_bytes(settings)
            .and_then(|bytes| super::root(&bytes))
            .map(|value| {
                json!(super::conflicting_native_registration(
                    &value, kind, &target, &owned
                ))
            })
            .unwrap_or(Value::Null);
        if let Some(recorded) = recorded {
            report["commands"] = super::commands_json(&recorded.owned);
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
        if let Ok(argv) = env.hook_argv(kind_harness(kind))
            && let Ok(expected) = shell_command(&argv)
        {
            report["current_command_matches"] = json!(
                report["command"]
                    .as_str()
                    .is_some_and(|recorded| recorded == expected
                        || recorded.starts_with(&format!("{expected} # ")))
            );
        }
        Ok(())
    }

    pub(crate) fn user_inspection(
        harness: Harness,
        env: &SetupEnv,
    ) -> Result<(PathBuf, lib::HookInspection), String> {
        let kind = match harness {
            Harness::Human => return Err(NO_HUMAN_SETUP.to_owned()),
            _ => registered_kind(harness)
                .ok_or_else(|| format!("{}: setup inspection is unsupported", harness.as_str()))?,
        };
        let file = env
            .config_root(kind.harness())
            .map(|root| root.join(kind.file_name()))
            .map_err(|e| e.to_string())?;
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
    /// The `Harness` a settings kind belongs to (its adapter id is a registered builtin).
    pub(crate) fn kind_harness(kind: SettingsKind) -> Harness {
        use crate::harness::registry::{OccupantHarness, builtins};
        Harness::from(OccupantHarness::Agent(
            builtins()
                .agent(kind.harness())
                .expect("a settings kind names a registered harness"),
        ))
    }
    pub(crate) fn harness_name(harness: Harness) -> &'static str {
        harness.as_str()
    }
}
