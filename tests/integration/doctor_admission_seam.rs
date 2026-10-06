//! Actual doctor -> shell canary core-admission seam, separate from historical ladders.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

/// Version the shipped table lists, one above its verified maximum, and one
/// planted as known broken.
const LISTED: &str = "2.1.286";
const NEWER: &str = "2.1.299";

struct Case {
    root: PathBuf,
}

impl Case {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ht-adm-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in ["bin", "home", "state"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        Self { root }
    }

    fn stub(&self, version: &str) {
        self.stub_harness("claude", version);
    }

    fn stub_harness(&self, harness: &str, version: &str) {
        let path = self.root.join("bin").join(harness);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{version} (Claude Code)'\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A versions document that lists `LISTED` as known broken (the range the
    /// canary reads and, through HT_TEST_RECIPES_JSON, the code under test).
    fn plant_broken_range(&self) -> PathBuf {
        let path = self.root.join("versions.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"schema_version": 1, "rows": [
                {"harness": "claude", "version": "2.1.285", "recipe": "claude-hooks-2.1.283",
                 "evidence": "live", "known_broken": []},
                {"harness": "claude", "version": LISTED, "recipe": "claude-hooks-2.1.283",
                 "evidence": "live", "known_broken": [{"min": LISTED, "max": LISTED}]},
            ]}))
            .unwrap(),
        )
        .unwrap();
        path
    }

    /// `doctor --json` under an isolated HOME and a PATH of only the stub dir
    /// plus the system dirs. Returns the file holding its stdout.
    fn doctor(&self, recipes: Option<&Path>) -> PathBuf {
        let mut command = Command::new(BIN); // leak-guard: tagged below via spawn::tag after env_clear
        command
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .args(["doctor", "--json", "--state-dir"])
            .arg(self.root.join("state"))
            .arg("--host-endpoint")
            .arg(self.root.join("herdr.sock"));
        herdr_threads::test_support::spawn::tag(&mut command);
        if let Some(recipes) = recipes {
            command.env("HT_TEST_RECIPES_JSON", recipes);
        }
        let output = command.output().unwrap();
        let out = self.root.join("doctor.json");
        fs::write(&out, &output.stdout).unwrap();
        out
    }

    fn admission(doctor: &Path) -> String {
        let doc: serde_json::Value = serde_json::from_slice(&fs::read(doctor).unwrap()).unwrap();
        doc["doctor"]["hooks"]["claude"]["installed"]["admission"]
            .as_str()
            .unwrap_or_else(|| panic!("no admission string in {doc}"))
            .to_owned()
    }

    fn canary(&self, args: &[&std::ffi::OsStr]) -> (i32, String) {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/harness-canary.sh");
        let mut command = Command::new("bash");
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("home/.claude"))
            .env("CODEX_HOME", self.root.join("home/.codex"))
            .arg(script)
            .args(args);
        herdr_threads::test_support::spawn::tag(&mut command);
        let output = command.output().unwrap();
        (
            output.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }

    fn probe(&self, harness: &str, version: &str, versions: Option<&Path>) -> PathBuf {
        let dir = self.root.join("probe");
        let mut args: Vec<&std::ffi::OsStr> = Vec::new();
        if let Some(versions) = versions {
            args.extend(["--versions-json".as_ref(), versions.as_os_str()]);
        }
        let binary = self.root.join("bin").join(harness);
        args.extend([
            "--write-probe-files".as_ref(),
            dir.as_os_str(),
            harness.as_ref(),
            version.as_ref(),
            binary.as_os_str(),
        ]);
        let (code, out) = self.canary(&args);
        assert_eq!(code, 0, "--write-probe-files: {out}");
        dir.join("canary-probe.json")
    }

    fn verdict(&self, doctor: &Path, probe: &Path) -> (i32, String) {
        self.canary(&[
            "--check-admission".as_ref(),
            doctor.as_os_str(),
            probe.as_os_str(),
        ])
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// Catches diagnostic version output or ladder overrides controlling doctor admission.
#[test]
fn doctor_declares_contract_for_listed_newer_and_known_broken_metadata() {
    for (tag, version, broken) in [
        ("listed", LISTED, false),
        ("newer", NEWER, false),
        ("broken", LISTED, true),
    ] {
        let case = Case::new(tag);
        case.stub(version);
        let recipes = broken.then(|| case.plant_broken_range());
        let doctor = case.doctor(recipes.as_deref());
        assert_eq!(Case::admission(&doctor), "contract_declared");
    }
}

// Genuine consumer regression: current doctor output goes through the shell's
// generated historical probe and actual saved-admission evaluator, without a mock.
#[test]
fn task3_r1_current_doctor_to_canary_pass_is_core_only() {
    for (tag, version, broken, historical) in [
        ("r1-listed", LISTED, false, "listed"),
        ("r1-newer", NEWER, false, "optimistic"),
        ("r1-broken", LISTED, true, "refused"),
    ] {
        let case = Case::new(tag);
        case.stub(version);
        let recipes = broken.then(|| case.plant_broken_range());
        let doctor = case.doctor(recipes.as_deref());
        assert_eq!(Case::admission(&doctor), "contract_declared");
        let probe = case.probe("claude", version, recipes.as_deref());
        let probe_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&probe).unwrap()).unwrap();
        assert_eq!(probe_json["expected_admission"], historical);
        let (code, out) = case.verdict(&doctor, &probe);
        assert_eq!((code, out.split('\t').next()), (0, Some("pass")), "{out}");
        assert!(out.contains("no exact-runtime/native proof"), "{out}");
    }
}

#[test]
fn task3_r1_codex_core_consumer_needs_no_historical_schema_result() {
    let case = Case::new("r1-codex");
    case.stub_harness("codex", "0.160.0");
    let doctor = case.doctor(None);
    let doc: serde_json::Value = serde_json::from_slice(&fs::read(&doctor).unwrap()).unwrap();
    assert_eq!(
        doc["doctor"]["hooks"]["codex"]["installed"]["admission"],
        "contract_declared"
    );
    assert!(doc["doctor"]["hooks"]["codex"]["installed"]["version"].is_null());
    let probe = case.probe("codex", "0.160.0", None);
    let probe_json: serde_json::Value = serde_json::from_slice(&fs::read(&probe).unwrap()).unwrap();
    assert_eq!(
        probe_json["expected_admission"],
        "schema-matched-or-optimistic"
    );
    let (code, out) = case.verdict(&doctor, &probe);
    assert_eq!((code, out.split('\t').next()), (0, Some("pass")), "{out}");
    assert!(out.contains("no exact-runtime/native proof"), "{out}");
}

#[test]
fn task3_r1_current_doctor_to_canary_keeps_fail_and_infra() {
    let case = Case::new("r1-errors");
    let doctor = case.doctor(None);
    let probe = case.probe("claude", LISTED, None);
    let (code, out) = case.verdict(&doctor, &probe);
    assert_eq!((code, out.split('\t').next()), (2, Some("infra")), "{out}");

    case.stub(LISTED);
    let doctor = case.doctor(None);
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&doctor).unwrap()).unwrap();
    for admission in [
        serde_json::Value::Null,
        serde_json::json!("unknown"),
        serde_json::json!("listed"),
    ] {
        let mut mutated = original.clone();
        mutated["doctor"]["hooks"]["claude"]["installed"]["admission"] = admission;
        fs::write(&doctor, serde_json::to_vec(&mutated).unwrap()).unwrap();
        let (code, out) = case.verdict(&doctor, &probe);
        assert_eq!((code, out.split('\t').next()), (1, Some("fail")), "{out}");
    }
    for bad in ["{}", "not json"] {
        fs::write(&doctor, bad).unwrap();
        let (code, out) = case.verdict(&doctor, &probe);
        assert_eq!((code, out.split('\t').next()), (1, Some("fail")), "{out}");
    }
}

#[test]
fn missing_claude_is_not_found() {
    let case = Case::new("missing");
    assert_eq!(Case::admission(&case.doctor(None)), "not_found");
}

// Catches ordinary doctor probing either harness and repairing metadata absence.
#[test]
fn task3_versionless_doctor_keeps_wrapper_metadata_optional_and_config_actionable() {
    let case = Case::new("task3-no-probes");
    let marker = case.root.join("invocations");
    for name in ["claude", "codex"] {
        let binary = case.root.join("bin").join(name);
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\n' \"$*\" >> '{}'\nexit 71\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(binary, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = case.doctor(None);
    let doc: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert!(!marker.exists(), "doctor invoked diagnostic flags");
    for name in ["claude", "codex"] {
        let installed = &doc["doctor"]["hooks"][name]["installed"];
        assert_eq!(installed["admission"], "contract_declared");
        assert!(installed["version"].is_null());
    }
    let limitations = doc["doctor"]["limitations"].to_string();
    assert!(
        limitations.contains("hooks are not installed"),
        "{limitations}"
    );
    assert!(!limitations.contains("not admitted"), "{limitations}");
}
