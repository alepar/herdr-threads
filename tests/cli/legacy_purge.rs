//! `legacy purge` through the built executable, in a scrubbed environment:
//! the dry run lists and removes nothing; `--confirm` removes only
//! `instances/`; the refusals are distinct one-line exit-2 errors.
//! Each test names the mutation it kills.

use herdr_threads::test_support::isolation::TestIsolation;
use std::{fs, path::Path, process::Output};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Fixture {
    iso: TestIsolation,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let fixture = Self {
            iso: TestIsolation::new(tag),
        };
        fs::create_dir_all(fixture.instances().join("a")).unwrap();
        fs::write(fixture.instances().join("a/threads.sqlite3"), [0u8; 2048]).unwrap();
        fs::write(fixture.instances().join("a/settings.json"), b"{}").unwrap();
        fs::create_dir_all(fixture.node()).unwrap();
        fs::write(fixture.node().join("herdr-threads.db"), b"keep").unwrap();
        fixture
    }

    fn state(&self) -> std::path::PathBuf {
        let state = self.iso.path("state");
        fs::create_dir_all(&state).unwrap();
        // The CLI reports the canonical path (macOS /tmp is /private/tmp).
        state.canonicalize().unwrap()
    }
    fn instances(&self) -> std::path::PathBuf {
        self.state().join("instances")
    }
    fn node(&self) -> std::path::PathBuf {
        self.state().join("node")
    }

    fn run(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut command = self.iso.command(BIN);
        command.arg("--state-dir").arg(self.state());
        command.args(["legacy", "purge"]).args(args);
        for (key, value) in envs {
            command.env(key, value);
        }
        command.output().unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Kills: a dry run that removes, or one that prints no inventory.
#[test]
fn dry_run_lists_and_removes_nothing() {
    let fixture = Fixture::new("legacy-dry");
    let out = fixture.run(&[], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains(&format!(
            "legacy data: {} (1 instances, 2.0 KiB) not used; remove: herdr-threads legacy purge",
            fixture.instances().display()
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("legacy settings not carried over:"),
        "{stdout}"
    );
    assert!(fixture.instances().join("a/threads.sqlite3").exists());
    assert!(fixture.node().join("herdr-threads.db").exists());
}

/// Kills: a purge that removes `node/` or the whole state dir, or one that
/// does not print the audit label.
#[test]
fn confirm_removes_only_instances_keeps_node() {
    let fixture = Fixture::new("legacy-confirm");
    let out = fixture.run(&["--confirm"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.starts_with(&format!(
            "purged {} (1 instances, 2.0 KiB) by operator:local-user:",
            fixture.instances().display()
        )),
        "{stdout}"
    );
    assert!(!fixture.instances().exists());
    assert_eq!(
        fs::read(fixture.node().join("herdr-threads.db")).unwrap(),
        b"keep"
    );
    // Nothing left to purge.
    let again = fixture.run(&["--confirm"], &[]);
    assert_eq!(again.status.code(), Some(0));
    assert!(text(&again.stdout).starts_with("no legacy data:"));
}

/// Kills: a purge that ignores agent evidence, or an `--operator` that does
/// not override it.
#[test]
fn confirm_refuses_agent_evidence() {
    let fixture = Fixture::new("legacy-agent");
    let out = fixture.run(&["--confirm"], &[("CLAUDECODE", "1")]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("--operator"), "{stderr}");
    assert!(stderr.contains("CLAUDECODE"), "{stderr}");
    assert_eq!(stderr.trim().lines().count(), 1, "{stderr}");
    assert!(fixture.instances().exists());

    let out = fixture.run(&["--confirm", "--operator"], &[("CLAUDECODE", "1")]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(!fixture.instances().exists());
}

/// Kills: following a symlinked `instances` (deleting its target or the link).
#[test]
fn confirm_refuses_symlinked_instances() {
    let fixture = Fixture::new("legacy-symlink");
    let target = fixture.iso.path("elsewhere");
    fs::rename(fixture.instances(), &target).unwrap();
    std::os::unix::fs::symlink(&target, fixture.instances()).unwrap();
    let out = fixture.run(&["--confirm"], &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("instances is a symlink"), "{stderr}");
    assert!(target.join("a/threads.sqlite3").exists());
    assert!(
        fs::symlink_metadata(fixture.instances())
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// Kills: a purge that ignores a held legacy `owner.lock`, or one that does
/// not name the holder's pid from `endpoint.json`.
#[test]
fn confirm_refuses_held_lock_naming_pid() {
    let fixture = Fixture::new("legacy-lock");
    let dir = fixture.instances().join("a");
    fs::write(dir.join("endpoint.json"), br#"{"pid": 31337}"#).unwrap();
    let held = fs::File::create(dir.join("owner.lock")).unwrap();
    held.lock().unwrap();
    let out = fixture.run(&["--confirm"], &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("legacy daemon running: pid 31337"),
        "{stderr}"
    );
    assert!(Path::new(&dir).join("threads.sqlite3").exists());

    held.unlock().unwrap();
    let out = fixture.run(&["--confirm"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(!fixture.instances().exists());
}
