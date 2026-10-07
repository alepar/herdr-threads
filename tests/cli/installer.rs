use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture {
    root: PathBuf,
    env: SetupEnv,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ht-installer-prompts-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("bin")).unwrap();
        for (name, version) in [
            ("claude", "2.1.284 (Claude Code)"),
            ("codex", "codex-cli 0.158.0"),
        ] {
            let file = root.join("bin").join(name);
            fs::write(&file, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = SetupEnv {
            executable: root.join("herdr-threads"),
            state_dir: Some(root.join("state")),
            cwd: root.clone(),
            path: Some(root.join("bin").into_os_string()),
            codex_home: Some(root.join("codex")),
            claude_config_dir: Some(root.join("claude")),
            host_endpoint: Some(root.join("herdr.sock")),
            instance_source: Value::Null,
            home: Some(root.clone().into()),
            declared_environment: std::collections::BTreeMap::new(),
        };
        Self { root, env }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn confirmation_requires_yes_and_keeps_eof_or_decline() {
    for (answer, expected) in [
        ("y\n", true),
        ("YES\n", true),
        ("\n", false),
        ("n\n", false),
        ("", false),
        ("maybe\n", false),
    ] {
        let mut output = Vec::new();
        assert_eq!(
            confirm("Install?", &mut io::Cursor::new(answer), &mut output).unwrap(),
            expected
        );
        assert!(String::from_utf8(output).unwrap().ends_with("[y/N] "));
    }
}

// Kills a single bundled confirmation, ignoring decline, or prompting for owned skills.
#[test]
fn separate_missing_decisions_and_owned_updates_have_independent_side_effects() {
    let f = Fixture::new();
    let mut prompts = Vec::new();
    let report = execute(&f.env, false, true, |question| {
        prompts.push(question.to_owned());
        Ok(question.contains("skill"))
    });
    assert_eq!(report["exit_status"], 0, "{report}");
    assert_eq!(
        prompts,
        [
            "Install herdr-threads hooks for claude?",
            "Install herdr-threads skill for claude?",
            "Install herdr-threads hooks for codex?",
            "Install herdr-threads skill for codex?"
        ]
    );
    assert!(!f.root.join("claude/settings.json").exists());
    assert!(!f.root.join("codex/hooks.json").exists());
    for h in ["claude", "codex"] {
        assert!(
            f.root
                .join(h)
                .join("skills/herdr-threads/SKILL.md")
                .exists()
        );
    }
    prompts.clear();
    let report = execute(&f.env, false, true, |question| {
        prompts.push(question.to_owned());
        Ok(false)
    });
    assert_eq!(report["exit_status"], 0, "{report}");
    assert_eq!(
        prompts,
        [
            "Install herdr-threads hooks for claude?",
            "Install herdr-threads hooks for codex?"
        ]
    );
    for entry in report["integrations"].as_array().unwrap() {
        assert_eq!(
            entry["outcome"],
            if entry["component"] == "skill" {
                "updated"
            } else {
                "declined"
            }
        );
    }
}

// Kills adopting a skill directory created by another writer while the user considers consent.
#[test]
fn skill_directory_created_during_prompt_is_preserved() {
    let f = Fixture::new();
    let report = execute(&f.env, false, true, |question| {
        if question == "Install herdr-threads skill for claude?" {
            fs::create_dir_all(f.root.join("claude/skills/herdr-threads")).unwrap();
            fs::write(f.root.join("claude/skills/herdr-threads/README"), "foreign").unwrap();
            Ok(true)
        } else {
            Ok(false)
        }
    });
    assert_eq!(report["exit_status"], 1);
    assert!(!f.root.join("claude/skills/herdr-threads/SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(f.root.join("claude/skills/herdr-threads/README")).unwrap(),
        "foreign"
    );
}

// Kills bypassing the foreign-hook precheck when configuration appears during consent.
#[test]
fn foreign_hooks_created_during_prompt_are_preserved() {
    let f = Fixture::new();
    let bytes = br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/other/herdr-threads hook claude"}]}]}}"#;
    let report = execute(&f.env, false, true, |question| {
        if question == "Install herdr-threads hooks for claude?" {
            fs::create_dir_all(f.root.join("claude")).unwrap();
            fs::write(f.root.join("claude/settings.json"), bytes).unwrap();
            Ok(true)
        } else {
            Ok(false)
        }
    });
    assert_eq!(report["exit_status"], 1);
    assert_eq!(
        fs::read(f.root.join("claude/settings.json")).unwrap(),
        bytes
    );
}

struct FourthInstaller;
impl crate::harness::adapter::HarnessAdapter for FourthInstaller {
    type Admission = ();
    fn metadata(&self) -> &'static crate::harness::adapter::AdapterMetadata {
        use crate::harness::adapter::*;
        static META: AdapterMetadata = AdapterMetadata {
            id: "fourth",
            display_label: "Fourth",
            context_spelling: "Fourth",
            context_aliases: &[],
            executable: ExecutableLookup::Path("fourth-cli"),
            host_kinds: &[],
            setup_scopes: &[SetupScopeKind::ConfigRoot],
            runtime_sources: &[],
            budget: EventBudgetPolicy {
                lifecycle_ms: 100,
                observer_ms: 50,
            },
        };
        &META
    }
    fn installer_policy(&self) -> Option<&dyn crate::harness::adapter::InstallerPolicy> {
        Some(self)
    }
    fn resolve_setup_scope(
        &self,
        _: &crate::harness::adapter::SetupScopeRequest,
        env: &crate::harness::adapter::SetupEnvironment,
    ) -> Result<crate::harness::adapter::ResolvedSetupScope, crate::harness::adapter::SetupFailure>
    {
        if env.declared.contains_key("FIXTURE_SCOPE_ERROR") {
            return Err(crate::harness::adapter::SetupFailure::Invalid(
                "fixture scope unavailable".into(),
            ));
        }
        if let Some(counter) = env.declared.get("FIXTURE_SCOPE_CALLS") {
            let call = fs::read_to_string(counter).unwrap().parse::<u32>().unwrap() + 1;
            fs::write(counter, call.to_string()).unwrap();
            if call >= 4 {
                return Ok(crate::harness::adapter::ResolvedSetupScope::ConfigRoot(
                    PathBuf::from(env.declared.get("FIXTURE_NEXT_ROOT").unwrap()),
                ));
            }
        }
        Ok(crate::harness::adapter::ResolvedSetupScope::ConfigRoot(
            PathBuf::from(
                env.declared
                    .get("FIXTURE_INSTALL_ROOT")
                    .expect("captured fixture root"),
            ),
        ))
    }
    fn contracts(&self) -> &'static [crate::harness::adapter::ContractDescriptor] {
        &[]
    }
    fn observe_install(
        &self,
        _: &crate::harness::adapter::InstallEnvironment,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::InstallObservation {
        panic!("installer must not observe runtime")
    }
    fn admit(
        &self,
        _: &crate::harness::adapter::AdmissionRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::AdmissionDecision<()> {
        panic!("installer grants no runtime admission")
    }
    fn version_ladder(
        &self,
        _: &crate::harness::runtime::RuntimeIdentity,
    ) -> crate::harness::adapter::Ladder {
        panic!("no runtime evidence")
    }
    fn classify(
        &self,
        _: &crate::harness::adapter::HookInput,
    ) -> crate::harness::adapter::ContractObservation {
        panic!("no hook input")
    }
    fn decode(
        &self,
        _: &(),
        _: &crate::harness::adapter::HookInput,
    ) -> Result<crate::harness::adapter::DecodedEvent, crate::harness::adapter::DecodeFailure> {
        panic!("no hook input")
    }
    fn encode(
        &self,
        _: &(),
        _: &crate::harness::adapter::DecodedEvent,
        _: &crate::harness::adapter::NeutralOffer,
    ) -> Result<crate::harness::adapter::EncodedOutput, crate::harness::adapter::EncodeFailure>
    {
        panic!("no output")
    }
    fn attribute_runtime(
        &self,
        _: &crate::harness::adapter::HookInput,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::RuntimeAttribution {
        panic!("no runtime")
    }
    fn setup(
        &self,
        r: &crate::harness::adapter::SetupRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
        let crate::harness::adapter::ResolvedSetupScope::ConfigRoot(root) = &r.scope else {
            panic!("fixture config scope")
        };
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("owned-hooks"), "owned").unwrap();
        Ok(crate::harness::adapter::SetupOutcome {
            actions: vec![crate::harness::adapter::SetupAction::InstalledOwned],
            diagnostic: "fixture owned".into(),
            projection: json!({"action":"installed"}),
            diagnostics: vec![],
        })
    }
    fn status(
        &self,
        _: &crate::harness::adapter::StatusRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::SetupStatus {
        panic!("installer uses declared ownership policy")
    }
    fn unsetup(
        &self,
        _: &crate::harness::adapter::UnsetupRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure>
    {
        panic!("no removal")
    }
}
impl crate::harness::adapter::InstallerPolicy for FourthInstaller {
    fn inspect_hooks(
        &self,
        r: &crate::harness::adapter::StatusRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::harness::adapter::InstallerHookState, crate::harness::adapter::SetupFailure>
    {
        use crate::harness::adapter::*;
        let ResolvedSetupScope::ConfigRoot(root) = &r.scope else {
            panic!("fixture scope")
        };
        match fs::read_to_string(root.join("owned-hooks")) {
            Ok(s) if s == "owned" => Ok(InstallerHookState::Owned),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(InstallerHookState::Missing),
            _ => Err(SetupFailure::Invalid("foreign hooks preserved".into())),
        }
    }
    fn skill_destination(
        &self,
        s: &crate::harness::adapter::ResolvedSetupScope,
    ) -> Option<crate::harness::adapter::InstallerSkillDestination> {
        let crate::harness::adapter::ResolvedSetupScope::ConfigRoot(root) = s else {
            return None;
        };
        // A lawful hooks-only registration declares no optional skill facility.
        if root.file_name().is_some_and(|name| name == "hooks-only") {
            return None;
        }
        if root
            .file_name()
            .is_some_and(|name| name == "escaping-skill")
        {
            return Some(crate::harness::adapter::InstallerSkillDestination {
                root: root.clone(),
                file: root.parent().unwrap().join("outside/SKILL.md"),
            });
        }
        Some(crate::harness::adapter::InstallerSkillDestination {
            root: root.clone(),
            file: root.join("skills/herdr-threads/SKILL.md"),
        })
    }
}

#[test]
fn absorption_installer_real_generic_consumer_uses_fourth_declared_provider() {
    use crate::harness::registry::{Registration, Registry};
    let mut f = Fixture::new();
    for name in ["claude", "codex"] {
        fs::remove_file(f.root.join("bin").join(name)).unwrap();
    }
    let binary = f.root.join("bin/fourth-cli");
    fs::write(&binary, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let root = f.root.join("fourth-home");
    f.env
        .declared_environment
        .insert("FIXTURE_INSTALL_ROOT".into(), root.clone().into_os_string());
    let registry = Registry::new(Box::leak(
        vec![Registration::new(&FourthInstaller)].into_boxed_slice(),
    ))
    .unwrap();
    let skipped = execute_for_registry(&registry, &f.env, false, false, |_| panic!("no terminal"));
    assert_eq!(skipped["integrations"].as_array().unwrap().len(), 2);
    assert!(
        skipped["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["harness"] == "fourth" && e["outcome"] == "skipped")
    );
    assert!(!root.exists());
    let mut prompts = Vec::new();
    let installed = execute_for_registry(&registry, &f.env, false, true, |q| {
        prompts.push(q.to_owned());
        Ok(true)
    });
    assert_eq!(prompts.len(), 2, "separate consent per missing component");
    assert_eq!(installed["exit_status"], 0, "{installed}");
    assert_eq!(
        fs::read_to_string(root.join("owned-hooks")).unwrap(),
        "owned"
    );
    assert_eq!(
        fs::read_to_string(root.join("skills/herdr-threads/SKILL.md")).unwrap(),
        crate::cli::skill::SKILL_MD
    );
    let updated = execute_for_registry(&registry, &f.env, false, false, |_| {
        panic!("owned components never prompt")
    });
    assert!(
        updated["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["outcome"] == "updated")
    );
    fs::write(root.join("skills/herdr-threads/SKILL.md"), "foreign edit").unwrap();
    let preserved = execute_for_registry(&registry, &f.env, true, false, |_| {
        panic!("explicit consent")
    });
    assert_eq!(preserved["exit_status"], 1);
    assert_eq!(
        fs::read_to_string(root.join("skills/herdr-threads/SKILL.md")).unwrap(),
        "foreign edit"
    );
}

#[test]
fn absorption_installer_destination_rejects_control_paths() {
    let registration = crate::harness::registry::Registration::new(&FourthInstaller);
    for root in ["/tmp/fourth\nroot", "/tmp/fourth\0root"] {
        assert!(
            registration
                .installer_skill_destination(
                    &crate::harness::adapter::ResolvedSetupScope::ConfigRoot(root.into())
                )
                .is_err(),
            "control path accepted: {root:?}"
        );
    }
}

#[test]
fn absorption_installer_provider_refuses_edited_owned_hooks_before_consent() {
    let f = Fixture::new();
    let installed = execute(&f.env, true, false, |_| panic!("explicit consent"));
    assert_eq!(installed["exit_status"], 0, "{installed}");
    let path = f.root.join("claude/settings.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["hooks"] = json!({});
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(
        crate::harness::setup::legacy::installer_hooks_installed(
            &f.env,
            crate::harness::context::Harness::Claude
        )
        .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn absorption_installer_absent_hermes_policy_reports_without_guessed_writes() {
    let f = Fixture::new();
    for name in ["claude", "codex"] {
        fs::remove_file(f.root.join("bin").join(name)).unwrap();
    }
    let binary = f.root.join("bin/hermes");
    let marker = f.root.join("hermes-invoked");
    fs::write(
        &binary,
        format!("#!/bin/sh\nprintf x > '{}'\nexit 99\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let report = execute(&f.env, true, false, |_| {
        panic!("undeclared policy cannot prompt")
    });
    let entries = report["integrations"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{report}");
    assert_eq!(entries[0]["harness"], "hermes");
    assert_eq!(entries[0]["component"], "installer");
    assert_eq!(entries[0]["outcome"], "unavailable");
    assert!(
        entries[0]["detail"]
            .as_str()
            .unwrap()
            .contains("no installer policy")
    );
    assert!(!marker.exists());
    assert!(!f.root.join("home/.hermes").exists());
    assert!(!f.root.join("hermes/skills").exists());
}

// Kills a canonical final resolution writing outside the inspected and consented scope.
#[test]
fn installer_final_scope_change_preserves_unconsented_root_and_independent_skill() {
    use crate::harness::registry::{Registration, Registry};
    let (f, root, next_root, counter) = changing_scope_fixture();
    let registry = Registry::new(Box::leak(
        vec![Registration::new(&FourthInstaller)].into_boxed_slice(),
    ))
    .unwrap();
    let mut prompts = Vec::new();
    let report = execute_for_registry(&registry, &f.env, false, true, |question| {
        prompts.push(question.to_owned());
        Ok(true)
    });
    assert!(
        !next_root.exists(),
        "unconsented final scope was written: {report}"
    );
    assert!(!root.join("owned-hooks").exists());
    assert_eq!(
        prompts,
        [
            "Install herdr-threads hooks for fourth?",
            "Install herdr-threads skill for fourth?"
        ]
    );
    assert_eq!(report["exit_status"], 1, "{report}");
    assert_eq!(report["integrations"][0]["outcome"], "failed");
    assert!(
        report["integrations"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("scope")
    );
    assert_eq!(report["integrations"][1]["outcome"], "installed");
    assert_eq!(
        fs::read_to_string(root.join("skills/herdr-threads/SKILL.md")).unwrap(),
        crate::cli::skill::SKILL_MD
    );
    assert_eq!(fs::read_to_string(counter).unwrap(), "4");
}

// Kills applying installer capture constraints to an ordinary canonical setup caller.
#[test]
fn ordinary_registered_setup_uses_current_resolved_scope() {
    use crate::harness::adapter::*;
    let (f, root, next_root, counter) = changing_scope_fixture();
    fs::write(counter, "3").unwrap();
    let registration = crate::harness::registry::Registration::new(&FourthInstaller);
    setup::execute_registered(
        &registration,
        SetupVerb::Install,
        &SetupScopeRequest::Default,
        None,
        SetupOptions::new(),
        &f.env.snapshot(),
    )
    .unwrap();
    assert!(!root.exists());
    assert_eq!(
        fs::read_to_string(next_root.join("owned-hooks")).unwrap(),
        "owned"
    );
}

fn changing_scope_fixture() -> (Fixture, PathBuf, PathBuf, PathBuf) {
    let mut f = Fixture::new();
    let binary = f.root.join("bin/fourth-cli");
    fs::write(&binary, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let root = f.root.join("scope-a");
    let next_root = f.root.join("scope-b");
    let counter = f.root.join("scope-calls");
    fs::write(&counter, "0").unwrap();
    for (key, value) in [
        ("FIXTURE_INSTALL_ROOT", &root),
        ("FIXTURE_NEXT_ROOT", &next_root),
        ("FIXTURE_SCOPE_CALLS", &counter),
    ] {
        f.env
            .declared_environment
            .insert(key.into(), value.clone().into_os_string());
    }
    // Only the declared fourth provider participates in the generic consumer.
    for name in ["claude", "codex"] {
        fs::remove_file(f.root.join("bin").join(name)).unwrap();
    }
    (f, root, next_root, counter)
}

// Kills treating a provider's absent optional skill as installer failure or inspecting a guessed path.
#[test]
fn optional_skill_none_is_unavailable_and_hooks_succeed() {
    use crate::harness::registry::{Registration, Registry};
    for (confirm_missing, interactive) in [(false, true), (true, false)] {
        let mut f = Fixture::new();
        for name in ["claude", "codex"] {
            fs::remove_file(f.root.join("bin").join(name)).unwrap();
        }
        let binary = f.root.join("bin/fourth-cli");
        let marker = f.root.join("native-invoked");
        fs::write(
            &binary,
            format!("#!/bin/sh\nprintf x > '{}'\nexit 99\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(binary, fs::Permissions::from_mode(0o755)).unwrap();
        let root = f.root.join("hooks-only");
        f.env
            .declared_environment
            .insert("FIXTURE_INSTALL_ROOT".into(), root.clone().into_os_string());
        // Skill inspection requires state; hooks-only setup does not. None must bypass inspection.
        f.env.state_dir = None;
        let registry = Registry::new(Box::leak(
            vec![Registration::new(&FourthInstaller)].into_boxed_slice(),
        ))
        .unwrap();
        let mut prompts = Vec::new();
        let report = execute_for_registry(&registry, &f.env, confirm_missing, interactive, |q| {
            prompts.push(q.to_owned());
            Ok(true)
        });
        assert_eq!(report["exit_status"], 0, "{report}");
        assert_eq!(
            prompts,
            if interactive {
                vec!["Install herdr-threads hooks for fourth?"]
            } else {
                vec![]
            }
        );
        assert_eq!(report["integrations"].as_array().unwrap().len(), 2);
        assert_eq!(report["integrations"][0]["component"], "hooks");
        assert_eq!(report["integrations"][0]["outcome"], "installed");
        assert_eq!(
            report["integrations"][1],
            json!({"harness":"fourth","component":"skill","outcome":"unavailable",
                   "detail":"registered adapter declares no skill destination"})
        );
        assert_eq!(
            fs::read_to_string(root.join("owned-hooks")).unwrap(),
            "owned"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert!(!f.root.join("state").exists());
        assert!(!marker.exists());
        let updated = execute_for_registry(&registry, &f.env, false, true, |_| {
            panic!("owned hooks and unavailable skill cannot prompt")
        });
        assert_eq!(updated["exit_status"], 0, "{updated}");
        assert_eq!(updated["integrations"][0]["outcome"], "updated");
        assert_eq!(updated["integrations"][1]["outcome"], "unavailable");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);

        // A second provider's genuine failures remain independent and ordered.
        let codex = f.root.join("bin/codex");
        fs::copy(f.root.join("bin/fourth-cli"), codex).unwrap();
        let mixed = Registry::new(Box::leak(
            vec![
                Registration::new(&FourthInstaller),
                Registration::new(&crate::harness::codex::CodexAdapter),
            ]
            .into_boxed_slice(),
        ))
        .unwrap();
        let report = execute_for_registry(&mixed, &f.env, true, false, |_| panic!("explicit"));
        assert_eq!(report["exit_status"], 1, "{report}");
        assert_eq!(report["integrations"][0]["harness"], "fourth");
        assert_eq!(report["integrations"][0]["outcome"], "updated");
        assert_eq!(report["integrations"][1]["outcome"], "unavailable");
        assert_eq!(report["integrations"][2]["harness"], "codex");
        assert_eq!(report["integrations"][2]["outcome"], "failed");
        assert_eq!(report["integrations"][3]["outcome"], "failed");
        assert!(!f.root.join("codex").exists());
        assert!(!marker.exists());
    }
}

fn installer_provider_fixture(scope: &str) -> (Fixture, PathBuf) {
    let mut f = Fixture::new();
    for name in ["claude", "codex"] {
        fs::remove_file(f.root.join("bin").join(name)).unwrap();
    }
    let binary = f.root.join("bin/fourth-cli");
    fs::write(&binary, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(binary, fs::Permissions::from_mode(0o755)).unwrap();
    let root = f.root.join(scope);
    f.env
        .declared_environment
        .insert("FIXTURE_INSTALL_ROOT".into(), root.clone().into_os_string());
    (f, root)
}

// Kills conflating unavailable with declared-but-missing, declined, invalid, or failed scope.
#[test]
fn optional_skill_present_and_invalid_keep_their_distinct_verdicts() {
    use crate::harness::registry::{Registration, Registry};
    let registry = Registry::new(Box::leak(
        vec![Registration::new(&FourthInstaller)].into_boxed_slice(),
    ))
    .unwrap();
    let (f, root) = installer_provider_fixture("present-skill");
    let skipped = execute_for_registry(&registry, &f.env, false, false, |_| panic!("no tty"));
    assert_eq!(skipped["exit_status"], 0);
    assert_eq!(skipped["integrations"][1]["outcome"], "skipped");
    assert!(!root.exists());
    let mut prompts = Vec::new();
    let declined = execute_for_registry(&registry, &f.env, false, true, |q| {
        prompts.push(q.to_owned());
        Ok(false)
    });
    assert_eq!(declined["exit_status"], 0);
    assert_eq!(declined["integrations"][1]["outcome"], "declined");
    assert_eq!(prompts.len(), 2);
    assert!(!root.exists());
    let (f, root) = installer_provider_fixture("escaping-skill");
    let invalid = execute_for_registry(&registry, &f.env, true, false, |_| panic!("explicit"));
    assert_eq!(invalid["exit_status"], 1, "{invalid}");
    assert_eq!(invalid["integrations"][0]["outcome"], "installed");
    assert_eq!(invalid["integrations"][1]["outcome"], "failed");
    assert!(!f.root.join("outside").exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    let (mut f, root) = installer_provider_fixture("hooks-only");
    f.env
        .declared_environment
        .insert("FIXTURE_SCOPE_ERROR".into(), "yes".into());
    let failed = execute_for_registry(&registry, &f.env, true, false, |_| panic!("bad scope"));
    assert_eq!(failed["exit_status"], 1, "{failed}");
    assert_eq!(failed["integrations"][0]["outcome"], "failed");
    assert_eq!(failed["integrations"][1]["outcome"], "failed");
    assert!(!root.exists());
}

// Kills turning a refused declared skill into optional absence or overwriting its bytes.
#[test]
fn declared_skill_ownership_refusals_preserve_bytes_and_hooks() {
    use crate::harness::registry::{Registration, Registry};
    let registry = Registry::new(Box::leak(
        vec![Registration::new(&FourthInstaller)].into_boxed_slice(),
    ))
    .unwrap();
    for kind in [
        "foreign",
        "edited",
        "partial",
        "malformed",
        "symlink",
        "oversized",
    ] {
        let (f, root) = installer_provider_fixture("present-skill");
        let report = execute_for_registry(&registry, &f.env, true, false, |_| panic!("explicit"));
        assert_eq!(report["exit_status"], 0, "{report}");
        let skill = root.join("skills/herdr-threads/SKILL.md");
        let manifest =
            setup::manifest_path(f.env.state_dir.as_ref().unwrap(), "fourth-skill", &skill);
        match kind {
            "foreign" => {
                fs::remove_file(&manifest).unwrap();
                fs::write(&skill, "foreign skill").unwrap();
            }
            "edited" => fs::write(&skill, "edited skill").unwrap(),
            "partial" => {
                let mut value: Value =
                    serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
                value["complete"] = json!(false);
                fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "malformed" => fs::write(&manifest, "invalid manifest").unwrap(),
            "symlink" => {
                fs::remove_file(&skill).unwrap();
                let target = f.root.join("foreign-target");
                fs::write(&target, "foreign target bytes").unwrap();
                std::os::unix::fs::symlink(target, &skill).unwrap();
            }
            "oversized" => fs::write(&skill, vec![b'x'; 1_048_577]).unwrap(),
            _ => unreachable!(),
        }
        let before = fs::read(&skill).unwrap();
        let manifest_before = fs::read(&manifest).ok();
        let report = execute_for_registry(&registry, &f.env, false, true, |_| {
            panic!("ownership refusal or owned hooks must not prompt")
        });
        assert_eq!(report["exit_status"], 1, "{kind}: {report}");
        assert_eq!(report["integrations"][0]["outcome"], "updated", "{kind}");
        assert_eq!(report["integrations"][1]["outcome"], "failed", "{kind}");
        assert_eq!(fs::read(&skill).unwrap(), before, "{kind}");
        assert_eq!(fs::read(&manifest).ok(), manifest_before, "{kind}");
        assert_eq!(
            fs::read_to_string(root.join("owned-hooks")).unwrap(),
            "owned"
        );
        if kind == "symlink" {
            assert!(
                fs::symlink_metadata(skill)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }
}
