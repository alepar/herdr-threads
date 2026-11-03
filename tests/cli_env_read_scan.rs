//! `src/cli/caller_location.rs` is the CLI's only reader of caller-location
//! environment: no other `src/cli` file names those variables as a string
//! literal (or as a `NAME=` assignment word in a format string).

use std::fs;
use std::path::{Path, PathBuf};

const NAMES: [&str; 8] = [
    "HERDR_PANE_ID",
    "HERDR_ENV",
    "HC_TERMINAL_ID",
    "HC_TERMINAL_SOCKET",
    "HERDR_THREADS_SEAT_NAME",
    "HERDR_THREADS_SEAT_READONLY",
    "TMUX",
    "TMUX_PANE",
];

/// Lines of `source` holding a string literal that is exactly a caller-location
/// variable name, or that begins with `NAME=` (an assignment word). Prose that
/// merely mentions a name inside a longer message is not an env read.
fn offending_lines(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        for name in NAMES {
            if line.contains(&format!("\"{name}\"")) || line.contains(&format!("\"{name}=")) {
                found.push((index + 1, line.trim().to_owned()));
                break;
            }
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

#[test]
fn only_caller_location_reads_caller_env() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(files.len() > 10, "scan found no sources under {root:?}");
    let mut offenders = Vec::new();
    for file in files {
        if file.file_name().is_some_and(|n| n == "caller_location.rs") {
            continue;
        }
        let source = fs::read_to_string(&file).unwrap();
        for (line, text) in offending_lines(&source) {
            offenders.push(format!("{}:{line}: {text}", file.display()));
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

#[test]
fn matcher_fails_on_a_planted_literal() {
    for name in NAMES {
        let planted = format!("let v = std::env::var(\"{name}\");");
        assert_eq!(offending_lines(&planted).len(), 1, "{name}");
    }
    let planted = "format!(\"HERDR_PANE_ID={}\", x)";
    assert_eq!(offending_lines(planted).len(), 1);
    // Prose and comments are not reads.
    assert!(offending_lines("\"run it inside the pane (HERDR_PANE_ID)\"").is_empty());
    assert!(offending_lines("// \"HERDR_ENV\"").is_empty());
}

/// A hook reads the caller environment once, at its entry (`HookEnv::from_process`), and swaps it
/// once when the guard verifies a terminal; every later use goes through `HookEnv::caller`. This
/// process's env cannot be set in a test (clippy.toml bans it), so a source check holds the rule.
#[test]
fn hook_reads_caller_env_at_its_entry_only() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli/hook.rs")).unwrap();
    let count = |needle: &str| {
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains(needle))
            .count()
    };
    assert_eq!(
        count("CallerEnv::from_process()"),
        1,
        "src/cli/hook.rs reads the process caller env once (HookEnv::from_process)"
    );
    assert_eq!(
        count("InstanceInputs::from_process"),
        0,
        "the hook's selector detection reads HookEnv::caller (InstanceInputs::from_caller)"
    );
}
