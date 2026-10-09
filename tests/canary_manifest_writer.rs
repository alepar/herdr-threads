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
    cmd.env("PYTHONDONTWRITEBYTECODE", "1");
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

#[test]
fn runtime_writer_roundtrips_through_the_real_rust_reader() {
    let dir = scratch("runtime");
    let script = r#"
import importlib.util,json,pathlib,sys
root,binary,directory=sys.argv[1:]
spec=importlib.util.spec_from_file_location('writer',pathlib.Path(root)/'scripts/canary/manifest.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
a=next(a for a in m.discovery(binary)['adapters'] if a['id']=='claude')
w=pathlib.Path(directory)/'work/claude-try1';w.mkdir(parents=True)
i={'key':'release:2.1.288','release_version':'2.1.288','source':'npm','base_version':None,'derived_version':None,'commit':None,'dirty':False,'distance':None}
r={'schema_version':1,'harness':'claude','attempt':'try1','identity':i,'evidence_stage':'no_model','outcome':'complete','reason':None,'domains':[{'domain':c['domain'],'origin':c['origin'],'contract_id':c['id'],'successful_milestones':c['required_milestones'],'violations':[],'outcome':'compatible'} for c in a['contracts']]}
(w/'result.json').write_text(json.dumps(r))
index={'schema_version':1,'attempts':[{'harness':'claude','attempt':'try1','identity_key':i['key'],'evidence_stage':'no_model','result_path':'work/claude-try1/result.json','capture_paths':[]}]}
p=pathlib.Path(directory)/'artifact-index.json';p.write_text(json.dumps(index))
results=m.indexed_results(p,{'schema_version':1,'adapters':[a]})
b={'schema_version':2,'generated_at':None,'rows':[],'contracts':{}}
print(m.render(m.add_runtime(b,b,{'schema_version':1,'adapters':[a]},results,'2026-10-02T06:00:00Z')),end='')
"#;
    let Some(out) = python(&[
        "-c".as_ref(),
        script.as_ref(),
        root().as_os_str(),
        BIN.as_ref(),
        dir.as_os_str(),
    ]) else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed = herdr_threads::harness::manifest::parse(&out.stdout).unwrap();
    let rows = parsed.runtime_rows();
    assert!(
        !rows.is_empty(),
        "actual Claude discovery has required domains"
    );
    let registration = herdr_threads::harness::registry::builtins()
        .registrations()
        .iter()
        .find(|r| r.metadata().id == "claude")
        .unwrap();
    for row in rows {
        let descriptor = registration
            .contracts()
            .iter()
            .find(|d| d.domain_id == row.domain)
            .unwrap();
        assert!(
            parsed
                .runtime_row("claude", &row.identity, descriptor)
                .is_some()
        );
        assert_eq!(row.last_seen_at, 1_790_920_800_000);
        assert_eq!(
            row.evidence_stage,
            herdr_threads::harness::manifest::RuntimeStage::NoModel
        );
        assert_eq!(row.identity.key, "release:2.1.288");
    }
    assert!(
        parsed.rows.is_empty(),
        "runtime identities never enter semver rows"
    );
}
