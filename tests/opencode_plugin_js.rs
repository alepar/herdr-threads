//! The opencode plugin's model-free logic tests
//! (`integrations/opencode/plugin/tests/plugin.test.mjs`, a fake opencode client, runner and
//! clock) under `node --test`. opencode runs the plugin under its embedded Bun; the logic tests
//! need only a JavaScript runtime, so this is skipped when `node` is not on PATH, as
//! `scripts/test-opencode-plugin` is.
use herdr_threads::test_support::spawn;
use std::path::{Path, PathBuf};

fn node() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("node"))
        .find(|path| path.is_file())
}

#[test]
fn plugin_logic_tests_pass() {
    let Some(node) = node() else {
        eprintln!("skipped: node not found on PATH");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = spawn::command(node)
        .arg("--test")
        .arg(root.join("integrations/opencode/plugin/tests/plugin.test.mjs"))
        .current_dir(root)
        .output()
        .expect("run node --test");
    assert!(
        output.status.success(),
        "node --test failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
