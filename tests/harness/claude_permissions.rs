use super::*;
use crate::harness::setup::{
    OwnedPermission, OwnershipManifest, SettingsKind, install_user_settings,
};
use serde_json::{Value, json};
use std::fs;

const BROAD: &str = "Bash(herdr-threads *)";

struct Scope {
    dir: PathBuf,
    settings: PathBuf,
    hooks: PathBuf,
}
impl Scope {
    fn new(settings: &[u8]) -> Self {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ht-claude-permissions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join(".claude")).unwrap();
        let settings_path = dir.join(".claude/settings.json");
        fs::write(&settings_path, settings).unwrap();
        Self {
            hooks: dir.join("hooks.json"),
            settings: settings_path,
            dir,
        }
    }
    fn component(&self) -> ClaudePermissions {
        ClaudePermissions::new(
            self.settings.clone(),
            self.dir.join("permissions.json"),
            self.hooks.clone(),
        )
    }
    fn value(&self) -> Value {
        serde_json::from_slice(&fs::read(&self.settings).unwrap()).unwrap()
    }
    fn backups(&self) -> Vec<Vec<u8>> {
        setup::user_config_backups(&self.settings)
    }
    /// A hook installation an earlier setup made, recording `rule` (added unless pre-existing).
    fn historical(&self, rule: &str, pre_existing: bool) {
        let base = fs::read(&self.settings).unwrap();
        let argv = vec!["/private/tmp/herdr-threads".to_owned(), "hook".to_owned()];
        install_user_settings(
            SettingsKind::ClaudeUser,
            &self.settings,
            &self.hooks,
            &argv,
            &base,
        )
        .unwrap();
        let mut manifest: OwnershipManifest =
            serde_json::from_slice(&fs::read(&self.hooks).unwrap()).unwrap();
        manifest.permission = Some(OwnedPermission {
            rule: rule.into(),
            fingerprint: format!(
                "sha256:{}",
                hex(&serde_json::to_vec(&json!({ "allow": rule })).unwrap())
            ),
            pre_existing,
        });
        fs::write(&self.hooks, serde_json::to_vec(&manifest).unwrap()).unwrap();
        if !pre_existing {
            let mut value = self.value();
            append(&mut value, "allow", rule, &mut BTreeSet::new()).unwrap();
            fs::write(&self.settings, serde_json::to_vec(&value).unwrap()).unwrap();
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn rules(value: &Value, list: &str) -> Vec<String> {
    native(value, list)
        .unwrap()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

// Plain setup with nothing owned or recorded must not grant anything, take a lock or back up.
#[test]
fn without_consent_or_record_nothing_is_touched() {
    let scope = Scope::new(br#"{"hooks":{}}"#);
    let status = scope
        .component()
        .apply(PermissionConsent::Undecided)
        .unwrap();
    assert_eq!(status.state, ClaudePermissionState::Missing);
    assert_eq!(fs::read(&scope.settings).unwrap(), br#"{"hooks":{}}"#);
    assert!(scope.backups().is_empty());
    assert!(!scope.dir.join("permissions.json").exists());
    assert!(
        !scope
            .dir
            .join(".claude/.herdr-threads-claude.lock")
            .exists()
    );
}

// The user's decision: an owned historical broad grant is kept without consent, now owned by
// the component, and the human and escalating commands start asking; `ht` needs consent.
#[test]
fn owned_historical_broad_rule_is_taken_over_without_consent() {
    let scope = Scope::new(br#"{"permissions":{"deny":["Bash(rm *)"]},"other":1}"#);
    scope.historical(BROAD, false);
    let before = fs::read(&scope.settings).unwrap();
    let backups = scope.backups().len();
    assert_eq!(
        scope.component().status().unwrap().state,
        ClaudePermissionState::Historical
    );
    let status = scope
        .component()
        .apply(PermissionConsent::Undecided)
        .unwrap();
    assert_eq!(status.state, ClaudePermissionState::Installed);
    let value = scope.value();
    assert_eq!(
        rules(&value, "allow"),
        vec![BROAD.to_owned(), "Bash(herdr-threads)".to_owned()]
    );
    let ask = rules(&value, "ask");
    for asked in [
        "human",
        "setup",
        "unsetup",
        "doctor fix",
        "internal installer-integrations",
    ] {
        assert!(
            ask.contains(&format!("Bash(herdr-threads {asked} *)")),
            "{asked}"
        );
        assert!(
            ask.contains(&format!("Bash(herdr-threads {asked})")),
            "{asked}"
        );
    }
    assert!(
        ask.iter().all(|r| covers_spelling(r, "herdr-threads")),
        "{ask:?}"
    );
    assert_eq!(value["permissions"]["deny"], json!(["Bash(rm *)"]));
    assert_eq!(value["other"], 1);
    // The taken-over rule is owned, not pre-existing: unsetup removes it.
    assert!(status.allow.contains(&(BROAD.to_owned(), false)));
    // One backed publication of the exact prior bytes; the hook record is retired.
    assert_eq!(scope.backups().len(), backups + 1);
    assert!(scope.backups().contains(&before));
    assert_eq!(
        recorded_hook_permission(&scope.settings, &scope.hooks).unwrap(),
        None
    );
    // A re-run changes nothing.
    let settled = fs::read(&scope.settings).unwrap();
    scope
        .component()
        .apply(PermissionConsent::Undecided)
        .unwrap();
    assert_eq!(fs::read(&scope.settings).unwrap(), settled);
    assert_eq!(scope.backups().len(), backups + 1);
    // Consent adds the `ht` rules.
    scope.component().apply(PermissionConsent::Granted).unwrap();
    assert!(rules(&scope.value(), "allow").contains(&"Bash(ht *)".to_owned()));
    assert!(rules(&scope.value(), "ask").contains(&"Bash(ht human *)".to_owned()));
}

// The retired export rule allowed nothing the plugin runs: it is removed, and only the bare
// spelling's rules take its place.
#[test]
fn owned_retired_export_rule_is_replaced() {
    let retired = crate::harness::claude::CALLER_CONTEXT_ALLOW_RULE;
    let scope = Scope::new(b"{}");
    scope.historical(retired, false);
    scope
        .component()
        .apply(PermissionConsent::Undecided)
        .unwrap();
    let allow = rules(&scope.value(), "allow");
    assert!(!allow.contains(&retired.to_owned()));
    assert!(allow.contains(&BROAD.to_owned()));
    assert!(!allow.contains(&"Bash(ht *)".to_owned()));
}

// A broad rule the user held before setup is theirs: the record goes, the rule stays.
#[test]
fn pre_existing_historical_rule_is_kept_and_reported_foreign() {
    let scope = Scope::new(br#"{"permissions":{"allow":["Bash(herdr-threads *)"]}}"#);
    scope.historical(BROAD, true);
    let before = fs::read(&scope.settings).unwrap();
    let backups = scope.backups();
    let status = scope
        .component()
        .apply(PermissionConsent::Undecided)
        .unwrap();
    assert_eq!(status.state, ClaudePermissionState::Missing);
    assert_eq!(status.foreign_broad, vec![BROAD.to_owned()]);
    assert_eq!(fs::read(&scope.settings).unwrap(), before);
    assert_eq!(scope.backups(), backups);
    assert!(!scope.dir.join("permissions.json").exists());
    assert_eq!(
        recorded_hook_permission(&scope.settings, &scope.hooks).unwrap(),
        None
    );
}

// Consent grants the full rendering; an identical rule already there is recorded pre-existing
// and survives removal, which otherwise restores the settings exactly.
#[test]
fn granted_install_records_pre_existing_rules_and_removal_restores() {
    let original = br#"{"permissions":{"allow":["Bash(ht *)"]},"x":1}"#;
    let scope = Scope::new(original);
    let status = scope.component().apply(PermissionConsent::Granted).unwrap();
    assert_eq!(status.state, ClaudePermissionState::Installed);
    assert!(status.allow.contains(&("Bash(ht *)".to_owned(), true)));
    assert_eq!(
        rules(&scope.value(), "allow")
            .iter()
            .filter(|r| *r == "Bash(ht *)")
            .count(),
        1
    );
    let status = scope.component().remove().unwrap();
    assert_eq!(status.state, ClaudePermissionState::Missing);
    assert_eq!(
        scope.value(),
        serde_json::from_slice::<Value>(original).unwrap()
    );
    assert!(!scope.dir.join("permissions.json").exists());
}

// Without consent an existing component never expands: a rule the user removed stays removed.
#[test]
fn without_consent_existing_component_only_keeps_or_narrows() {
    let scope = Scope::new(b"{}");
    scope.component().apply(PermissionConsent::Granted).unwrap();
    let mut value = scope.value();
    remove_one(&mut value, "allow", "Bash(ht *)");
    fs::write(&scope.settings, serde_json::to_vec(&value).unwrap()).unwrap();
    let status = scope
        .component()
        .apply(PermissionConsent::Undecided)
        .unwrap();
    assert!(!rules(&scope.value(), "allow").contains(&"Bash(ht *)".to_owned()));
    assert!(!status.allow.iter().any(|(rule, _)| rule == "Bash(ht *)"));
    // Removal then leaves no residue of containers the component created.
    scope.component().remove().unwrap();
    assert_eq!(scope.value(), json!({}));
}

// Interrupted after the intent: settings unchanged, so the next run restores and replans.
// Interrupted after the settings: the next run finishes, retiring the hook record.
#[test]
fn interrupted_publication_settles_on_the_next_run() {
    for fault in [Fault::AfterIntent, Fault::AfterSettings] {
        let scope = Scope::new(b"{}");
        scope.historical(BROAD, false);
        arm(Some(fault));
        assert_eq!(
            scope.component().apply(PermissionConsent::Undecided),
            Err(SetupError::Io)
        );
        arm(None);
        assert_eq!(
            scope.component().status().unwrap().state,
            ClaudePermissionState::Pending
        );
        let status = scope
            .component()
            .apply(PermissionConsent::Undecided)
            .unwrap();
        assert_eq!(status.state, ClaudePermissionState::Installed, "{fault:?}");
        assert!(rules(&scope.value(), "ask").contains(&"Bash(herdr-threads human *)".to_owned()));
        assert_eq!(
            rules(&scope.value(), "allow")
                .iter()
                .filter(|r| *r == BROAD)
                .count(),
            1
        );
        assert_eq!(
            recorded_hook_permission(&scope.settings, &scope.hooks).unwrap(),
            None
        );
    }
}

// Settings edited while a publication was interrupted are neither the before nor the after
// state: refuse rather than guess.
#[test]
fn interrupted_publication_with_edited_settings_refuses() {
    let scope = Scope::new(b"{}");
    scope.historical(BROAD, false);
    arm(Some(Fault::AfterSettings));
    assert!(
        scope
            .component()
            .apply(PermissionConsent::Undecided)
            .is_err()
    );
    arm(None);
    let mut value = scope.value();
    value["edited"] = json!(true);
    fs::write(&scope.settings, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        scope.component().apply(PermissionConsent::Undecided),
        Err(SetupError::Conflict)
    );
    assert_eq!(scope.component().remove(), Err(SetupError::Conflict));
}

// A historical grant the user already deleted is not re-added without consent: the record is
// only retired.
#[test]
fn deleted_historical_rule_is_not_regranted() {
    let scope = Scope::new(b"{}");
    scope.historical(BROAD, false);
    let mut value = scope.value();
    remove_one(&mut value, "allow", BROAD);
    fs::write(&scope.settings, serde_json::to_vec(&value).unwrap()).unwrap();
    let status = scope
        .component()
        .apply(PermissionConsent::Undecided)
        .unwrap();
    assert_eq!(status.state, ClaudePermissionState::Missing);
    // Nothing granted; only the empty list the earlier setup created is tidied away.
    value.as_object_mut().unwrap().remove("permissions");
    assert_eq!(scope.value(), value);
    assert_eq!(
        recorded_hook_permission(&scope.settings, &scope.hooks).unwrap(),
        None
    );
}

// With settings.json (or its whole directory) gone, removal and setup forget the component
// instead of refusing, even mid-publication.
#[test]
fn missing_settings_forget_the_component() {
    for gone in ["file", "dir", "pending"] {
        let scope = Scope::new(b"{}");
        scope.component().apply(PermissionConsent::Granted).unwrap();
        if gone == "pending" {
            arm(Some(Fault::AfterIntent));
            let _ = scope.component().apply(PermissionConsent::Granted);
            arm(None);
        }
        if gone == "dir" {
            fs::remove_dir_all(scope.dir.join(".claude")).unwrap();
        } else {
            fs::remove_file(&scope.settings).unwrap();
        }
        assert_eq!(
            scope.component().remove().unwrap().state,
            ClaudePermissionState::Missing,
            "{gone}"
        );
        assert!(!scope.component().manifest().exists(), "{gone}");
        assert_eq!(
            scope
                .component()
                .apply(PermissionConsent::Undecided)
                .unwrap()
                .state,
            ClaudePermissionState::Missing,
            "{gone}"
        );
    }
}
