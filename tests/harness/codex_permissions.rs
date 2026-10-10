use super::*;
use crate::harness::setup::user_config_backups;

struct Scope {
    dir: PathBuf,
}
impl Scope {
    fn new() -> Self {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ht-codex-permissions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("codex")).unwrap();
        Self { dir }
    }
    fn component(&self) -> CodexPermissions {
        CodexPermissions::new(&self.dir.join("codex"), self.dir.join("permissions.json"))
    }
    fn rules(&self) -> PathBuf {
        self.dir.join("codex/rules/herdr-threads.rules")
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// Without consent nothing is written; with it the rendered rules are installed, a re-run changes
// nothing, and removal deletes the file, keeping it as a backup.
#[test]
fn codex_rules_install_only_with_consent_and_remove_cleanly() {
    let scope = Scope::new();
    let component = scope.component();
    assert_eq!(
        component.apply(PermissionConsent::Undecided),
        Ok(CodexPermissionState::Missing)
    );
    assert!(!scope.dir.join("codex/rules").exists());
    assert_eq!(
        component.apply(PermissionConsent::Granted),
        Ok(CodexPermissionState::Installed)
    );
    assert_eq!(fs::read(scope.rules()).unwrap(), render().into_bytes());
    assert_eq!(
        component.apply(PermissionConsent::Undecided),
        Ok(CodexPermissionState::Installed)
    );
    assert_eq!(component.remove(), Ok(CodexPermissionState::Missing));
    assert!(!scope.rules().exists());
    assert!(!scope.dir.join("permissions.json").exists());
    // The backup keeps the directory, as backups keep every config directory.
    assert_eq!(
        user_config_backups(&scope.rules()),
        vec![render().into_bytes()]
    );
}

// A file at the owned name herdr-threads did not write, or an owned file someone edited, is never
// overwritten or deleted.
#[test]
fn codex_rules_never_touch_foreign_or_edited_files() {
    let scope = Scope::new();
    fs::create_dir_all(scope.dir.join("codex/rules")).unwrap();
    fs::write(scope.rules(), b"# mine\n").unwrap();
    let component = scope.component();
    assert_eq!(component.status(), Ok(CodexPermissionState::Foreign));
    assert_eq!(
        component.apply(PermissionConsent::Granted),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(scope.rules()).unwrap(), b"# mine\n");
    fs::remove_file(scope.rules()).unwrap();
    component.apply(PermissionConsent::Granted).unwrap();
    fs::write(scope.rules(), b"# edited\n").unwrap();
    assert_eq!(component.status(), Ok(CodexPermissionState::Edited));
    // Plain setup reports it; an explicit grant refuses; removal leaves it and forgets it.
    assert_eq!(
        component.apply(PermissionConsent::Undecided),
        Ok(CodexPermissionState::Edited)
    );
    assert_eq!(
        component.apply(PermissionConsent::Granted),
        Err(SetupError::Conflict)
    );
    assert_eq!(component.remove(), Ok(CodexPermissionState::Edited));
    assert_eq!(fs::read(scope.rules()).unwrap(), b"# edited\n");
    assert_eq!(component.status(), Ok(CodexPermissionState::Foreign));
}

// A deleted owned file is not re-granted without consent: the record is forgotten.
#[test]
fn codex_rules_deleted_by_the_person_stay_deleted() {
    let scope = Scope::new();
    let component = scope.component();
    component.apply(PermissionConsent::Granted).unwrap();
    fs::remove_file(scope.rules()).unwrap();
    assert_eq!(
        component.apply(PermissionConsent::Undecided),
        Ok(CodexPermissionState::Missing)
    );
    assert!(!scope.rules().exists());
    assert!(!scope.dir.join("permissions.json").exists());
}

// An outdated owned file is refreshed in place, keeping the previous bytes as a backup.
#[test]
fn codex_rules_refresh_backs_up_the_previous_rendering() {
    let scope = Scope::new();
    let component = scope.component();
    component.apply(PermissionConsent::Granted).unwrap();
    let old = b"# herdr-threads owned execpolicy v0\n".to_vec();
    fs::write(scope.rules(), &old).unwrap();
    let mut manifest = component.read().unwrap().unwrap();
    manifest.resources[0].fingerprint = hex(&old);
    component.write(&manifest).unwrap();
    assert_eq!(
        component.apply(PermissionConsent::Undecided),
        Ok(CodexPermissionState::Installed)
    );
    assert_eq!(fs::read(scope.rules()).unwrap(), render().into_bytes());
    assert_eq!(user_config_backups(&scope.rules()), vec![old]);
}

// An interrupted publication settles on the next run: a recorded intent whose file is still the
// "before" state restores the previous record.
#[test]
fn codex_rules_interrupted_intent_restores_the_previous_record() {
    let scope = Scope::new();
    let component = scope.component();
    component.apply(PermissionConsent::Granted).unwrap();
    let installed = component.read().unwrap().unwrap();
    let mut intent = installed.clone();
    intent.resources.clear();
    intent.pending = Some(PendingPublication {
        before: hex(&fs::read(scope.rules()).unwrap()),
        after: ABSENT.into(),
        previous: Some(Box::new(installed.clone())),
        retired: None,
    });
    component.write(&intent).unwrap();
    assert_eq!(component.status(), Ok(CodexPermissionState::Pending));
    assert_eq!(
        component.apply(PermissionConsent::Undecided),
        Ok(CodexPermissionState::Installed)
    );
    assert_eq!(component.read().unwrap(), Some(installed));
}

// A rules file that appears after the absent baseline is never replaced: the exclusive publish
// refuses, the foreign bytes stay, nothing is recorded as owned and nothing is pending.
#[test]
fn codex_rules_first_publish_never_replaces_a_file_created_meanwhile() {
    let scope = Scope::new();
    let component = scope.component();
    let rules = scope.rules();
    BEFORE_RULES_WRITE.with(|hook| {
        let rules = rules.clone();
        hook.set(Some(Box::new(move || {
            fs::write(&rules, b"# foreign\n").unwrap();
        })));
    });
    assert_eq!(
        component.apply(PermissionConsent::Granted),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&rules).unwrap(), b"# foreign\n");
    assert!(user_config_backups(&rules).is_empty());
    assert!(!scope.dir.join("permissions.json").exists());
    assert_eq!(component.status(), Ok(CodexPermissionState::Foreign));
}
