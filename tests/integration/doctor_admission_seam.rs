//! ht-p03.48: the doctor -> canary seam. The installed executable's
//! `doctor --json` (ht-p03.49) reports the Claude admission string for a stub
//! `claude` on a scratch PATH, and the canary's t0.admission verdict
//! (`harness-canary.sh --check-admission`, ht-p03.14.6) parses that same
//! output against a canary-probe.json written by the canary itself.
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
        let path = self.root.join("bin/claude");
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

    fn canary(&self, args: &[&std::ffi::OsStr]) -> (i32, String) {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/harness-canary.sh");
        let mut command = Command::new("bash");
        command.arg(script).args(args);
        let output = command.output().unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    /// The canary's own expected-admission file for `version`, against `versions`
    /// (None: the shipped table).
    fn probe(&self, version: &str, versions: Option<&Path>) -> PathBuf {
        let dir = self.root.join("probe");
        let mut args: Vec<&std::ffi::OsStr> = Vec::new();
        if let Some(v) = versions {
            args.extend(["--versions-json".as_ref(), v.as_os_str()]);
        }
        let bin = self.root.join("bin/claude");
        args.extend([
            "--write-probe-files".as_ref(),
            dir.as_os_str(),
            "claude".as_ref(),
            version.as_ref(),
            bin.as_os_str(),
        ]);
        let (code, _) = self.canary(&args);
        assert_eq!(code, 0, "--write-probe-files failed");
        dir.join("canary-probe.json")
    }

    fn admission(doctor: &Path) -> String {
        let doc: serde_json::Value = serde_json::from_slice(&fs::read(doctor).unwrap()).unwrap();
        doc["doctor"]["hooks"]["claude"]["installed"]["admission"]
            .as_str()
            .unwrap_or_else(|| panic!("no admission string in {doc}"))
            .to_owned()
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

/// Kills: doctor naming a listed version anything but `listed`, or the canary
/// failing to accept the string doctor printed.
#[test]
fn listed_claude_is_listed_and_the_canary_passes() {
    let case = Case::new("listed");
    case.stub(LISTED);
    let doctor = case.doctor(None);
    assert_eq!(Case::admission(&doctor), "listed");
    let probe = case.probe(LISTED, None);
    let (code, out) = case.verdict(&doctor, &probe);
    assert_eq!((code, out.split('\t').next()), (0, Some("pass")), "{out}");
}

/// Kills: a version above the verified maximum reported as listed or refused.
#[test]
fn newer_claude_is_optimistic_and_the_canary_passes() {
    let case = Case::new("newer");
    case.stub(NEWER);
    let doctor = case.doctor(None);
    assert_eq!(Case::admission(&doctor), "optimistic");
    let probe = case.probe(NEWER, None);
    let (code, out) = case.verdict(&doctor, &probe);
    assert_eq!((code, out.split('\t').next()), (0, Some("pass")), "{out}");
}

/// Kills: a known-broken range planted through HT_TEST_RECIPES_JSON that doctor
/// ignores (admission stays listed) or a canary that expects anything but refused.
#[test]
fn known_broken_claude_is_refused_and_the_canary_passes() {
    let case = Case::new("broken");
    case.stub(LISTED);
    let versions = case.plant_broken_range();
    let doctor = case.doctor(Some(&versions));
    assert_eq!(Case::admission(&doctor), "refused");
    let probe = case.probe(LISTED, Some(&versions));
    let (code, out) = case.verdict(&doctor, &probe);
    assert_eq!((code, out.split('\t').next()), (0, Some("pass")), "{out}");
    // The probe's expectation is what is measured: a listed expectation fails.
    let listed_probe = case.probe(LISTED, None);
    let (code, out) = case.verdict(&doctor, &listed_probe);
    assert_eq!((code, out.split('\t').next()), (1, Some("fail")), "{out}");
}

/// Kills: a missing claude reported as an admitted or refused version, and a
/// canary that calls it a version break instead of an infra error.
#[test]
fn missing_claude_is_not_found_and_the_canary_reports_infra() {
    let case = Case::new("missing");
    let doctor = case.doctor(None);
    assert_eq!(Case::admission(&doctor), "not_found");
    let probe = case.probe(LISTED, None);
    let (code, out) = case.verdict(&doctor, &probe);
    assert_eq!((code, out.split('\t').next()), (2, Some("infra")), "{out}");
}
