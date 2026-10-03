//! Stub `claude` / `codex` executables for tests that resolve a harness on a
//! scratch `PATH`.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// Write `<dir>/<name>`, a `#!/bin/sh` script whose `--version` prints the
/// line `name`'s real executable prints for `version` (`<version> (Claude
/// Code)` or `codex-cli <version>`), mode 0755. Returns its path.
pub(crate) fn write_stub_harness(dir: &Path, name: &str, version: &str) -> PathBuf {
    let line = match name {
        "claude" => format!("{version} (Claude Code)"),
        "codex" => format!("codex-cli {version}"),
        other => panic!("no stub version line for harness {other:?}"),
    };
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{line}'\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}
