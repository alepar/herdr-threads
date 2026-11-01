//! `NativeCli::new` in `src/cli` appears only at the kept sites of the F
//! table: pane observation of the caller's own Herdr host, and launch and
//! handoff. Labels come from the daemon (`Command::HostTopologies`). Sites are
//! keyed by file and enclosing fn with an occurrence count, never a line.

use std::fs;
use std::path::{Path, PathBuf};

/// (file under src/cli, enclosing fn, occurrences, why kept).
const KEPT: &[(&str, &str, usize, &str)] = &[
    ("mod.rs", "pane_agent", 1, "pane observation"),
    ("mod.rs", "located_pane_seat", 1, "pane observation"),
    ("hook.rs", "locate_seat_for", 1, "pane observation"),
    // Lazy claim-host adapter for pane selectors (topology, current_pane); no label read (spec F6 amended, ht-kqrt.9).
    ("mod.rs", "run_parsed_in_pane", 1, "pane observation"),
    ("mod.rs", "run_launch", 1, "launch"),
    ("handoff.rs", "run", 1, "handoff"),
    ("peer_launch.rs", "create_tab", 1, "launch"),
    ("topology_runtime.rs", "run", 1, "handoff"),
    ("topology_runtime.rs", "run_operator", 2, "handoff"),
];

/// `(enclosing fn, line)` of every `NativeCli::new` in `source`.
fn sites(source: &str) -> Vec<String> {
    let mut current = String::from("<none>");
    let mut found = Vec::new();
    for line in source.lines() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        if let Some(at) = code.find("fn ") {
            let head = &code[..at];
            let head = head.trim();
            if head.is_empty() || head.starts_with("pub") || head == "async" {
                let name: String = code[at + 3..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    current = name;
                }
            }
        }
        if line.contains("NativeCli::new") {
            found.push(current.clone());
        }
    }
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn violations(files: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, source) in files {
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for function in sites(source) {
            *counts.entry(function).or_default() += 1;
        }
        for (function, count) in counts {
            let allowed = KEPT
                .iter()
                .find(|(file, f, _, _)| name == file && *f == function)
                .map_or(0, |(_, _, n, _)| *n);
            if count > allowed {
                bad.push(format!(
                    "{name}: fn {function} has {count}, allowed {allowed}"
                ));
            }
        }
    }
    bad
}

#[test]
fn native_cli_new_only_at_kept_sites() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli");
    let mut paths = Vec::new();
    rust_files(&root, &mut paths);
    assert!(paths.len() > 10, "scan found no sources under {root:?}");
    let files: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            (
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read_to_string(p).unwrap(),
            )
        })
        .collect();
    assert_eq!(violations(&files), Vec::<String>::new());
}

// Kills: a scan that matches nothing, or one that lets a label read in a new
// fn (or an extra one in a kept fn) through.
#[test]
fn scan_flags_an_unlisted_site() {
    let extra = "fn labels() {\n    let h = NativeCli::new(a, b);\n}\n";
    assert_eq!(sites(extra), ["labels"]);
    let v = violations(&[("follow.rs".into(), extra.into())]);
    assert_eq!(v.len(), 1, "{v:?}");
    let twice = "fn run_launch() {\n NativeCli::new(a);\n NativeCli::new(b);\n}\n";
    assert_eq!(violations(&[("mod.rs".into(), twice.into())]).len(), 1);
    let kept = "fn run_launch() {\n NativeCli::new(a);\n}\n";
    assert!(violations(&[("mod.rs".into(), kept.into())]).is_empty());
}
