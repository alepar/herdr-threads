//! The canary's harness-manifest writer (scripts/canary/manifest.py) against the real binary: the contract
//! ids it reads from `herdr-threads contract-id --json` are the library's own, and `write` normalizes
//! versions through the binary's `harness-version normalize`. Skipped (with a printed reason) when
//! `python3` is absent.

use herdr_threads::harness::contract::{contract_for, contract_id};
use herdr_threads::test_support::spawn::{SpawnOwned, command};
use std::path::{Path, PathBuf};
use std::process::Output;

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn python(args: &[&std::ffi::OsStr]) -> Option<Output> {
    let mut cmd = command("python3");
    cmd.args(args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    match cmd.spawn_owned() {
        Ok(child) => Some(child.wait_with_output().expect("python3 output")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("spawning python3: {error}"),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canary-manifest-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn manifest_py() -> PathBuf {
    root().join("scripts/canary/manifest.py")
}

#[test]
fn writer_contract_ids_equal_the_binarys() {
    let script = manifest_py();
    let Some(out) = python(&[
        script.as_os_str(),
        "contract".as_ref(),
        "--binary".as_ref(),
        BIN.as_ref(),
    ]) else {
        eprintln!("skipped: python3 is not installed");
        return;
    };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for harness in ["claude", "codex"] {
        let want = contract_id(contract_for(harness).unwrap());
        assert_eq!(doc[harness], want, "{harness}");
    }
}

#[test]
fn writer_over_the_all_pass_case_records_the_binarys_contracts() {
    let dir = scratch("write");
    let case: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("scripts/harness-canary-selftest/manifest-cases/all-pass.json"))
            .unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("baseline.json"), case["baseline"].to_string()).unwrap();
    std::fs::write(dir.join("report.json"), case["report"].to_string()).unwrap();
    let script = manifest_py();
    let out_path = dir.join("out.json");
    let Some(out) = python(&[
        script.as_os_str(),
        "write".as_ref(),
        "--baseline".as_ref(),
        dir.join("baseline.json").as_os_str(),
        "--report".as_ref(),
        dir.join("report.json").as_os_str(),
        "--binary".as_ref(),
        BIN.as_ref(),
        "--generated-at".as_ref(),
        "2026-10-02T06:00:00Z".as_ref(),
        "--out".as_ref(),
        out_path.as_os_str(),
    ]) else {
        eprintln!("skipped: python3 is not installed");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{stderr}");
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&out_path).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let claude = contract_id(contract_for("claude").unwrap());
    let codex = contract_id(contract_for("codex").unwrap());
    assert_eq!(doc["contracts"]["claude"], claude);
    assert_eq!(doc["contracts"]["codex"], codex);
    // Every canary row of this run is keyed by the binary's id for its harness.
    let rows = doc["rows"].as_array().unwrap();
    let fresh: Vec<_> = rows.iter().filter(|r| r["source"] == "canary").collect();
    assert_eq!(fresh.len(), 3, "{rows:#?}");
    for row in fresh {
        let want = if row["harness"] == "claude" {
            &claude
        } else {
            &codex
        };
        assert_eq!(row["contract_id"], *want, "{row}");
    }
}
