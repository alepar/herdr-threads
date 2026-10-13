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
            config_roots: [
                ("codex".into(), root.join("codex")),
                ("claude".into(), root.join("claude")),
            ]
            .into(),
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
            match entry["component"].as_str() {
                Some("skill") => "updated",
                // Permissions follow the hooks: never granted where the hooks were declined.
                Some("permissions") => "skipped",
                _ => "declined",
            }
        );
    }
}

// The permissions component is asked separately, only once the hooks are in place, and a no
// leaves the hooks installed without a grant; --without-permissions never asks.
#[test]
fn permissions_are_a_separate_decision_after_the_hooks() {
    let f = Fixture::new();
    let mut prompts = Vec::new();
    let report = execute(&f.env, false, true, |question| {
        prompts.push(question.to_owned());
        Ok(!question.contains("claude run"))
    });
    assert_eq!(report["exit_status"], 0, "{report}");
    assert!(
        prompts.contains(&super::permission_question("claude")),
        "{prompts:?}"
    );
    assert!(
        prompts.contains(&super::permission_question("codex")),
        "{prompts:?}"
    );
    let outcome = |harness: &str| {
        report["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["harness"] == harness && e["component"] == "permissions")
            .map(|e| e["outcome"].clone())
    };
    assert_eq!(outcome("claude"), Some(json!("declined")));
    assert_eq!(outcome("codex"), Some(json!("installed")));
    assert!(f.root.join("codex/rules/herdr-threads.rules").exists());
    let settings = fs::read_to_string(f.root.join("claude/settings.json")).unwrap();
    assert!(!settings.contains("Bash(herdr-threads *)"), "{settings}");
    let again = super::execute_with(&f.env, &[], false, true, true, |question| {
        assert!(!question.contains("without prompting"), "{question}");
        Ok(true)
    });
    let claude = again["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["harness"] == "claude" && e["component"] == "permissions")
        .unwrap()
        .clone();
    assert_eq!(claude["outcome"], "skipped");
    assert_eq!(claude["detail"], "--without-permissions");
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
            agent_env_markers: &[],
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

// ---------------------------------------------------------------- shared roots and extra roots

fn extra(harness: &str, root: &Path) -> ExtraConfigRoot {
    ExtraConfigRoot {
        harness: harness.into(),
        root: root.to_path_buf(),
    }
}

fn entry<'a>(report: &'a Value, harness: &str, component: &str, root: &Path) -> &'a Value {
    report["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["harness"] == harness
                && e["component"] == component
                && e["config_root"] == json!(root.display().to_string())
        })
        .unwrap_or_else(|| {
            panic!(
                "no {harness} {component} entry for {}: {report}",
                root.display()
            )
        })
}

/// A profile root whose `skills` (and, for Codex, `rules`) directories are symbolic links to
/// `target`'s, like an aisw profile.
fn linked_profile(root: &Path, target: &Path, rules: bool) {
    fs::create_dir_all(root).unwrap();
    std::os::unix::fs::symlink(target.join("skills"), root.join("skills")).unwrap();
    if rules {
        std::os::unix::fs::symlink(target.join("rules"), root.join("rules")).unwrap();
    }
}

fn no_removal_advice(detail: &str) {
    assert!(detail.contains("shared through a symlink to"), "{detail}");
    assert!(
        detail.contains("set up the root that holds the real file first"),
        "{detail}"
    );
    assert!(
        !detail.contains("remove it") && !detail.contains("move it aside"),
        "{detail}"
    );
}

// Kills writing through a shared symlink, asking for it, failing on it, or recording the lexical
// path: an owned, unedited real file is `shared`, and the profile's own hooks still install.
#[test]
fn shared_skill_and_rules_owned_at_the_real_root_are_reported_not_written() {
    let f = Fixture::new();
    let installed = execute(&f.env, true, false, |_| panic!("explicit consent"));
    assert_eq!(installed["exit_status"], 0, "{installed}");
    let codex_profile = f.root.join("profiles/codex/p1");
    let claude_profile = f.root.join("profiles/claude/c1");
    linked_profile(&codex_profile, &f.root.join("codex"), true);
    linked_profile(&claude_profile, &f.root.join("claude"), false);
    let skill = fs::read(f.root.join("codex/skills/herdr-threads/SKILL.md")).unwrap();
    let rules = fs::read(f.root.join("codex/rules/herdr-threads.rules")).unwrap();
    let mut prompts = Vec::new();
    let report = execute_for_registry_with(
        crate::harness::registry::builtins(),
        &f.env,
        &[
            extra("codex", &codex_profile),
            extra("claude", &claude_profile),
        ],
        false,
        false,
        true,
        |question| {
            prompts.push(question.to_owned());
            Ok(true)
        },
    );
    assert_eq!(report["exit_status"], 0, "{report}");
    assert_eq!(
        prompts,
        [
            format!(
                "Install herdr-threads hooks for claude (CLAUDE_CONFIG_DIR={})?",
                claude_profile.display()
            ),
            permission_question(&format!(
                "claude (CLAUDE_CONFIG_DIR={})",
                claude_profile.display()
            )),
            format!(
                "Install herdr-threads hooks for codex (CODEX_HOME={})?",
                codex_profile.display()
            ),
        ]
    );
    for (harness, profile) in [("codex", &codex_profile), ("claude", &claude_profile)] {
        let shared = entry(&report, harness, "skill", profile);
        assert_eq!(shared["outcome"], "shared", "{report}");
        let detail = shared["detail"].as_str().unwrap();
        assert!(
            detail.starts_with("shared via symlink; managed at ")
                && detail.ends_with(&format!("{harness}/skills/herdr-threads/SKILL.md")),
            "{detail}"
        );
        assert_eq!(
            entry(&report, harness, "hooks", profile)["outcome"],
            "installed"
        );
        let state = f.env.state_dir.as_ref().unwrap();
        let lexical = profile.join("skills/herdr-threads/SKILL.md");
        assert!(!setup::manifest_path(state, &format!("{harness}-skill"), &lexical).exists());
        assert!(
            fs::symlink_metadata(profile.join("skills"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // The effective roots are labelled too once a harness has more than one root.
        let default = f.root.join(harness);
        assert_eq!(
            entry(&report, harness, "skill", &default)["outcome"],
            "updated"
        );
        assert_eq!(
            entry(&report, harness, "skill", &default)["root_label"],
            json!(format!(
                "{}={}",
                if harness == "codex" {
                    "CODEX_HOME"
                } else {
                    "CLAUDE_CONFIG_DIR"
                },
                default.display()
            ))
        );
    }
    let permissions = entry(&report, "codex", "permissions", &codex_profile);
    assert_eq!(permissions["outcome"], "shared", "{report}");
    assert!(
        permissions["detail"]
            .as_str()
            .unwrap()
            .ends_with("codex/rules/herdr-threads.rules"),
        "{permissions}"
    );
    assert_eq!(
        entry(&report, "claude", "permissions", &claude_profile)["outcome"],
        "installed"
    );
    assert_eq!(
        fs::read(f.root.join("codex/skills/herdr-threads/SKILL.md")).unwrap(),
        skill
    );
    assert_eq!(
        fs::read(f.root.join("codex/rules/herdr-threads.rules")).unwrap(),
        rules
    );
    assert!(codex_profile.join("hooks.json").is_file());
    // Status and doctor report the same shared state for the profile's rules.
    let mut profile_env = f.env.clone();
    profile_env
        .config_roots
        .insert("codex".into(), codex_profile.clone());
    let status = crate::harness::codex::setup::permission_status_json(&profile_env);
    assert_eq!(status["state"], "shared", "{status}");
    // A rerun converges without questions.
    let again = execute_for_registry_with(
        crate::harness::registry::builtins(),
        &f.env,
        &[
            extra("codex", &codex_profile),
            extra("claude", &claude_profile),
        ],
        false,
        false,
        true,
        |question| panic!("owned or shared: {question}"),
    );
    assert_eq!(again["exit_status"], 0, "{again}");
}

// Kills adopting, overwriting or advising removal of a shared file herdr-threads does not own at
// its real location (foreign or edited); an absent real file is skipped with the same advice.
#[test]
fn shared_files_not_owned_at_the_real_root_fail_without_removal_advice() {
    let f = Fixture::new();
    let foreign = f.root.join("foreign");
    fs::create_dir_all(foreign.join("skills/herdr-threads")).unwrap();
    fs::create_dir_all(foreign.join("rules")).unwrap();
    fs::write(
        foreign.join("skills/herdr-threads/SKILL.md"),
        "foreign skill",
    )
    .unwrap();
    fs::write(foreign.join("rules/herdr-threads.rules"), "foreign rules").unwrap();
    let empty = f.root.join("empty");
    fs::create_dir_all(empty.join("skills")).unwrap();
    fs::create_dir_all(empty.join("rules")).unwrap();
    let foreign_profile = f.root.join("profiles/codex/foreign");
    let empty_profile = f.root.join("profiles/codex/empty");
    linked_profile(&foreign_profile, &foreign, true);
    linked_profile(&empty_profile, &empty, true);
    let report = execute_for_registry_with(
        crate::harness::registry::builtins(),
        &f.env,
        &[
            extra("codex", &foreign_profile),
            extra("codex", &empty_profile),
        ],
        true,
        false,
        false,
        |_| panic!("explicit consent"),
    );
    assert_eq!(report["exit_status"], 1, "{report}");
    for component in ["skill", "permissions"] {
        let failed = entry(&report, "codex", component, &foreign_profile);
        assert_eq!(failed["outcome"], "failed", "{report}");
        no_removal_advice(failed["detail"].as_str().unwrap());
        let skipped = entry(&report, "codex", component, &empty_profile);
        assert_eq!(skipped["outcome"], "skipped", "{report}");
        let detail = skipped["detail"].as_str().unwrap();
        assert!(detail.contains("which is not installed"), "{detail}");
        no_removal_advice(detail);
    }
    // Failure isolation: the effective root and the profiles' own hooks still converge.
    assert_eq!(
        entry(&report, "codex", "skill", &f.root.join("codex"))["outcome"],
        "installed"
    );
    assert_eq!(
        entry(&report, "codex", "hooks", &foreign_profile)["outcome"],
        "installed"
    );
    assert_eq!(
        fs::read_to_string(foreign.join("skills/herdr-threads/SKILL.md")).unwrap(),
        "foreign skill"
    );
    assert_eq!(
        fs::read_to_string(foreign.join("rules/herdr-threads.rules")).unwrap(),
        "foreign rules"
    );
    assert!(!empty.join("skills/herdr-threads").exists());
    assert!(!empty.join("rules/herdr-threads.rules").exists());

    // Edited at the real root: the owner's record no longer matches, so the profile fails too.
    let owned_profile = f.root.join("profiles/codex/owned");
    linked_profile(&owned_profile, &f.root.join("codex"), true);
    fs::write(f.root.join("codex/skills/herdr-threads/SKILL.md"), "edited").unwrap();
    fs::write(f.root.join("codex/rules/herdr-threads.rules"), "edited").unwrap();
    let edited = execute_for_registry_with(
        crate::harness::registry::builtins(),
        &f.env,
        &[extra("codex", &owned_profile)],
        true,
        false,
        false,
        |_| panic!("explicit consent"),
    );
    assert_eq!(edited["exit_status"], 1, "{edited}");
    for component in ["skill", "permissions"] {
        let failed = entry(&edited, "codex", component, &owned_profile);
        assert_eq!(failed["outcome"], "failed", "{edited}");
        no_removal_advice(failed["detail"].as_str().unwrap());
    }
    assert_eq!(
        fs::read_to_string(f.root.join("codex/rules/herdr-threads.rules")).unwrap(),
        "edited"
    );
}

// Kills a changed default: with no symlink on the path, a foreign skill keeps today's refusal.
#[test]
fn unshared_foreign_skill_keeps_the_existing_refusal() {
    let f = Fixture::new();
    let profile = f.root.join("profiles/codex/plain");
    fs::create_dir_all(profile.join("skills/herdr-threads")).unwrap();
    fs::write(profile.join("skills/herdr-threads/SKILL.md"), "foreign").unwrap();
    let report = execute_for_registry_with(
        crate::harness::registry::builtins(),
        &f.env,
        &[extra("codex", &profile)],
        true,
        false,
        false,
        |_| panic!("explicit consent"),
    );
    let failed = entry(&report, "codex", "skill", &profile);
    assert_eq!(failed["outcome"], "failed");
    assert!(
        failed["detail"]
            .as_str()
            .unwrap()
            .starts_with("unowned skill at "),
        "{failed}"
    );
}

// Kills reconciling a root twice, ignoring consent per root, letting one root's failure stop
// another, or reconciling a profile before the default root that holds its shared files.
#[test]
fn extra_roots_are_ordered_deduplicated_and_consented_per_root() {
    let mut f = Fixture::new();
    // The adapters' default roots (their variables unset) are under HOME. HOME is not the
    // working directory here: Codex reads `<cwd>/.codex` as a project layer, and a hook there
    // is refused as a duplicate.
    fs::create_dir_all(f.root.join("home")).unwrap();
    f.env.home = Some(f.root.join("home").into_os_string());
    let home_codex = f.root.join("home/.codex");
    let second = f.root.join("codex-2");
    let broken = f.root.join("codex-broken");
    fs::create_dir_all(broken.join("skills/herdr-threads")).unwrap();
    fs::write(broken.join("skills/herdr-threads/SKILL.md"), "foreign").unwrap();
    fs::create_dir_all(f.root.join("codex")).unwrap();
    let alias = f.root.join("codex-alias");
    std::os::unix::fs::symlink(f.root.join("codex"), &alias).unwrap();
    let claude_second = f.root.join("claude-2");
    let mut prompts = Vec::new();
    let report = execute_for_registry_with(
        crate::harness::registry::builtins(),
        &f.env,
        &[
            extra("codex", &second),
            extra("codex", &broken),
            extra("codex", &alias),
            extra("codex", &second),
            extra("codex", &home_codex),
            extra("claude", &claude_second),
        ],
        false,
        false,
        true,
        |question| {
            prompts.push(question.to_owned());
            // Declines only the second Codex root's hooks.
            Ok(!(question.contains("hooks") && question.contains("codex-2")))
        },
    );
    assert_eq!(report["exit_status"], 1, "{report}");
    let codex_roots: Vec<String> = report["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["harness"] == "codex" && e["component"] == "hooks")
        .map(|e| e["config_root"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        codex_roots,
        [&home_codex, &f.root.join("codex"), &second, &broken]
            .map(|root| root.display().to_string()),
        "{report}"
    );
    assert_eq!(
        entry(&report, "codex", "hooks", &second)["outcome"],
        "declined"
    );
    assert_eq!(
        entry(&report, "codex", "skill", &second)["outcome"],
        "installed"
    );
    assert_eq!(
        entry(&report, "codex", "permissions", &second)["outcome"],
        "skipped"
    );
    assert_eq!(
        entry(&report, "codex", "skill", &broken)["outcome"],
        "failed"
    );
    assert_eq!(
        entry(&report, "codex", "hooks", &broken)["outcome"],
        "installed",
        "{report}"
    );
    for root in [&home_codex, &f.root.join("codex")] {
        assert_eq!(
            entry(&report, "codex", "hooks", root)["outcome"],
            "installed",
            "{report}"
        );
        assert_eq!(
            entry(&report, "codex", "permissions", root)["outcome"],
            "installed",
            "{report}"
        );
    }
    assert_eq!(
        entry(&report, "claude", "hooks", &claude_second)["outcome"],
        "installed"
    );
    assert!(
        prompts.contains(&format!(
            "Install herdr-threads skill for codex (CODEX_HOME={})?",
            second.display()
        )),
        "{prompts:?}"
    );
    assert!(home_codex.join("hooks.json").is_file());
    assert!(!second.join("hooks.json").exists());
    assert!(
        !report.to_string().contains(&alias.display().to_string()),
        "a symlinked alias of a reconciled root is the same root: {report}"
    );
}

// Kills a `setup codex` that writes through, removes through, or advises removing a shared
// rules file: the canonical setup reports it like the installer does.
#[test]
fn setup_codex_reports_shared_rules_and_never_writes_through_the_link() {
    use crate::cli::setup::{PermissionPolicy, SetupRequest};
    let f = Fixture::new();
    let installed = execute(&f.env, true, false, |_| panic!("explicit consent"));
    assert_eq!(installed["exit_status"], 0, "{installed}");
    let profile = f.root.join("profiles/codex/p1");
    linked_profile(&profile, &f.root.join("codex"), true);
    let mut env = f.env.clone();
    env.config_roots.insert("codex".into(), profile.clone());
    let request = |verb, permissions| SetupRequest {
        scope: Default::default(),
        verb,
        harness: crate::harness::context::Harness::Codex,
        harness_binary: None,
        prompt_suggestions: Default::default(),
        hooks_only: false,
        permissions,
        pane_env: Default::default(),
    };
    let rules = f.root.join("codex/rules/herdr-threads.rules");
    let bytes = fs::read(&rules).unwrap();
    let report = setup::execute(&request(SetupVerb::Install, PermissionPolicy::Grant), &env)
        .expect("shared grant is satisfied");
    assert_eq!(report["permissions"]["state"], "shared", "{report}");
    let removed =
        setup::execute(&request(SetupVerb::Remove, PermissionPolicy::Ask), &env).expect("unsetup");
    assert_eq!(removed["permissions"]["state"], "shared", "{removed}");
    assert_eq!(fs::read(&rules).unwrap(), bytes);
    fs::write(&rules, "edited").unwrap();
    let error = setup::execute(&request(SetupVerb::Install, PermissionPolicy::Grant), &env)
        .unwrap_err()
        .to_string();
    no_removal_advice(&error);
    let advised = setup::execute(&request(SetupVerb::Install, PermissionPolicy::Ask), &env)
        .expect("advice only");
    assert_eq!(
        advised["permissions"]["state"], "shared_unmanaged",
        "{advised}"
    );
    no_removal_advice(advised["permissions"]["note"].as_str().unwrap());
    assert_eq!(fs::read_to_string(&rules).unwrap(), "edited");
}

#[test]
fn config_root_values_are_registered_installer_harnesses_and_absolute_dirs() {
    let registry = crate::harness::registry::builtins();
    assert_eq!(
        parse_config_root("codex=/a/b/", registry).unwrap(),
        extra("codex", Path::new("/a/b"))
    );
    for (value, wanted) in [
        ("codex", "expected HARNESS=DIR"),
        ("codex=relative/dir", "absolute path"),
        ("codex=/a/../b", "absolute path"),
        ("codex=/a\nb", "absolute path"),
        ("nope=/a", "nope"),
        ("human=/a", "human"),
        ("=/a", "\"\""),
    ] {
        let error = parse_config_root(value, registry).unwrap_err();
        assert!(error.contains(wanted), "{value:?}: {error}");
    }
    // A registered agent without an installer policy is refused by name, never by a branch.
    for registration in registry.registrations() {
        if registration.installer_policy().is_none() {
            let id = registration.metadata().id;
            let error = parse_config_root(&format!("{id}=/a"), registry).unwrap_err();
            assert!(error.contains("does not reconcile"), "{id}: {error}");
        }
    }
}

// Kills the $HOME-cwd false positive: `<cwd>/.codex` that resolves to the CODEX_HOME being set up
// is that home, not a project layer (as Codex treats it). The user-level home read as a project
// layer for another root stays refused (Codex loads it there), without removal advice; a genuine
// project `.codex` with the hook keeps its refusal and advice.
#[test]
fn codex_project_layer_scan_skips_the_home_being_set_up() {
    use crate::cli::setup::{PermissionPolicy, SetupRequest};
    let mut f = Fixture::new();
    let home = f.root.join("home");
    fs::create_dir_all(&home).unwrap();
    let alias = f.root.join("home-alias");
    std::os::unix::fs::symlink(&home, &alias).unwrap();
    f.env.home = Some(home.clone().into_os_string());
    // The working directory reaches HOME through a symlink, so `<cwd>/.codex` is lexically not
    // CODEX_HOME but resolves to it.
    f.env.cwd = alias.clone();
    f.env
        .config_roots
        .insert("codex".into(), home.join(".codex"));
    let request = SetupRequest {
        scope: Default::default(),
        verb: SetupVerb::Install,
        harness: crate::harness::context::Harness::Codex,
        harness_binary: None,
        prompt_suggestions: Default::default(),
        hooks_only: false,
        permissions: PermissionPolicy::Grant,
        pane_env: Default::default(),
    };
    setup::execute(&request, &f.env).expect("default root from HOME");
    setup::execute(&request, &f.env).expect("default root rerun from HOME");
    let installer = execute(&f.env, true, false, |_| panic!("explicit consent"));
    let hooks = installer["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["harness"] == "codex" && e["component"] == "hooks")
        .unwrap();
    assert_eq!(hooks["outcome"], "updated", "{installer}");

    // A profile root set up from HOME: Codex would read ~/.codex as a project layer there.
    let profile = f.root.join("profiles/codex/p1");
    fs::create_dir_all(&profile).unwrap();
    let mut profile_env = f.env.clone();
    profile_env
        .config_roots
        .insert("codex".into(), profile.clone());
    let error = setup::execute(&request, &profile_env)
        .unwrap_err()
        .to_string();
    assert!(error.contains("is the user-level Codex home"), "{error}");
    assert!(
        error.contains("run setup from another working directory"),
        "{error}"
    );
    assert!(!error.contains("Remove it"), "{error}");
    assert!(!profile.join("hooks.json").exists());
    let report = execute_for_registry_with(
        crate::harness::registry::builtins(),
        &f.env,
        &[extra("codex", &profile)],
        true,
        false,
        false,
        |_| panic!("explicit consent"),
    );
    let failed = entry(&report, "codex", "hooks", &profile);
    assert_eq!(failed["outcome"], "failed", "{report}");
    assert!(
        failed["detail"]
            .as_str()
            .unwrap()
            .contains("user-level Codex home"),
        "{failed}"
    );
    assert_eq!(
        entry(&report, "codex", "hooks", &home.join(".codex"))["outcome"],
        "updated",
        "{report}"
    );
    // From another working directory the profile root installs.
    profile_env.cwd = f.root.clone();
    setup::execute(&request, &profile_env).expect("profile root from elsewhere");

    // A genuine project `.codex` holding the hook in a git repository is still refused.
    let repo = f.root.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join(".codex")).unwrap();
    fs::copy(
        home.join(".codex/hooks.json"),
        repo.join(".codex/hooks.json"),
    )
    .unwrap();
    let mut project_env = f.env.clone();
    project_env.cwd = repo.join("sub");
    fs::create_dir_all(&project_env.cwd).unwrap();
    let error = setup::execute(&request, &project_env)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Remove it from that file"), "{error}");
    assert!(
        error.contains(&repo.join(".codex/hooks.json").display().to_string()),
        "{error}"
    );
}
