//! Stub `claude` / `codex` executables for tests that resolve a harness on a
//! scratch `PATH`.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// Place `<dir>/<name>`, a `#!/bin/sh` script whose `--version` prints the
/// line `name`'s real executable prints for `version` (`<version> (Claude
/// Code)` or `codex-cli <version>`), mode 0755. Returns its path.
///
/// `<dir>/<name>` is a symlink to one shared, never-modified script per
/// (name, version) under the temp dir. macOS assesses the first exec of every
/// newly written file (syspolicyd/XProtect): ~0.2 s, and several seconds while
/// builds write new executables, past the production 5 s version-probe
/// bound. Executing a symlink to an already-executed file is not a first
/// exec. The probe's identity resolves the link to the shared script, whose
/// inode, size and times never change once published.
pub(crate) fn write_stub_harness(dir: &Path, name: &str, version: &str) -> PathBuf {
    let line = match name {
        "claude" => format!("{version} (Claude Code)"),
        "codex" => format!("codex-cli {version}"),
        other => panic!("no stub version line for harness {other:?}"),
    };
    let shared = shared_stub(
        name,
        version,
        &format!("#!/bin/sh\nprintf '%s\\n' '{line}'\n"),
    );
    let path = dir.join(name);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("replace {}: {error}", path.display()),
    }
    std::os::unix::fs::symlink(&shared, &path).unwrap();
    path
}

/// The shared script for (name, version): staged under a unique name, then
/// hard-linked into place without ever replacing a published one, and never
/// rewritten or relinked, so a test holding its identity never sees it change.
fn shared_stub(name: &str, version: &str, script: &str) -> PathBuf {
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let cache = std::env::temp_dir().join(format!("ht-stub-harness-v1-uid{uid}"));
    fs::create_dir_all(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let shared = cache.join(format!("{name}-{version}"));
    if !shared.exists() {
        let staged = cache.join(format!(".{name}-{version}.{}", uuid::Uuid::new_v4()));
        fs::write(&staged, script).unwrap();
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).unwrap();
        // The winner keeps its staged name as a second link: unlinking it
        // would change the published inode's ctime, part of the identity a
        // concurrent test may already hold.
        match fs::hard_link(&staged, &shared) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&staged);
            }
            Err(error) => panic!("publish {}: {error}", shared.display()),
        }
    }
    assert_eq!(
        fs::read_to_string(&shared).unwrap(),
        script,
        "{} holds a different script",
        shared.display()
    );
    shared
}
