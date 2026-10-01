//! Isolated clean-install lifecycle gate. Runs `scripts/validate-package.sh`,
//! which publishes the committed HEAD to a private repository and drives the
//! pinned Herdr 0.9.1 install/build/startup/action/pane/uninstall/link path
//! against a private Herdr server, which it restarts and live-hands-off (never
//! the shared one) to exercise the `[[startup]]` entry and its rerun against a
//! serving daemon. Ignored by default: it performs several
//! locked release builds and needs the installed Rust toolchain and permission to bind Unix
//! sockets under /private/tmp. See docs/validation/package.md for the
//! mutations each step kills.

use std::{path::Path, process::Command};

#[test]
#[ignore = "clean release installs through a private Herdr server (several minutes)"]
fn clean_package_install_lifecycle() {
    let validator = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/validate-package.sh");
    let output = Command::new(validator)
        .output()
        .expect("start package validator");
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{stdout}");
    assert!(
        output.status.success(),
        "package validation failed: stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("PACKAGE_VALIDATION_PASS"));
}
