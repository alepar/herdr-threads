//! `HcDiscovery` against scratch `core.json` state and `fake_hc`.

use super::*;
use crate::test_support::fake_hc::FakeHc;
use std::collections::BTreeMap;

/// A scratch XDG layout; the lookup closure never touches the process env.
struct Layout {
    root: PathBuf,
    extra: BTreeMap<String, OsString>,
}

impl Layout {
    fn new() -> Self {
        let root = crate::test_support::scratch::temp_path(format!(
            "ht-disc-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(root.join("run/herdr-core")).unwrap();
        std::fs::create_dir_all(root.join("state/herdr-core")).unwrap();
        Self {
            root,
            extra: BTreeMap::new(),
        }
    }

    fn set(&mut self, name: &str, value: impl Into<OsString>) {
        self.extra.insert(name.into(), value.into());
    }

    fn core_json(&self, session: &str, contents: &str) {
        let dir = self.root.join("state/herdr-core").join(session);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("core.json"), contents).unwrap();
    }

    fn scan(&self) -> Vec<DiscoveredCore> {
        let env = |name: &str| -> Option<OsString> {
            match name {
                "XDG_STATE_HOME" => Some(self.root.join("state").into()),
                "XDG_RUNTIME_DIR" => Some(self.root.join("run").into()),
                "XDG_CONFIG_HOME" => Some(self.root.join("config").into()),
                "HOME" => Some(self.root.join("home").into()),
                other => self.extra.get(other).cloned(),
            }
        };
        HcDiscovery::scan(&env)
    }
}

fn identity(id: &str) -> String {
    format!(r#"{{"id":"{id}","name":"x","machine_id":"m"}}"#)
}

#[test]
fn finds_fake_core_from_core_json() {
    let layout = Layout::new();
    let fake = FakeHc::start();
    layout.core_json("alpha", &identity("core-0001"));
    // The runtime-dir rule: <run>/herdr-core/<session>.sock.
    // A hard link: the same socket inode at the path the core's rules name.
    std::fs::hard_link(fake.socket(), layout.root.join("run/herdr-core/alpha.sock")).unwrap();

    let found = layout.scan();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "core-0001");
    assert_eq!(found[0].session, "alpha");
    assert_eq!(
        found[0].socket,
        layout.root.join("run/herdr-core/alpha.sock")
    );
    assert_eq!(found[0].boot.id().as_str(), "hc-core:core-0001");
    // A scan only says hello.
    assert!(fake.calls().iter().all(|c| c.method == "hello"));
}

#[test]
fn socket_comes_from_config_when_no_env_override() {
    let layout = Layout::new();
    let fake = FakeHc::start();
    layout.core_json("alpha", &identity("core-0001"));
    std::fs::create_dir_all(layout.root.join("config/herdr-core")).unwrap();
    std::fs::write(
        layout.root.join("config/herdr-core/config.toml"),
        format!("socket = {:?}\n", fake.socket().to_str().unwrap()),
    )
    .unwrap();
    let found = layout.scan();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].socket, fake.socket());
}

#[test]
fn ignores_stale_dir_without_socket() {
    let layout = Layout::new();
    layout.core_json("gone", &identity("core-0001"));
    layout.core_json("broken", "{not json");
    layout.core_json("anonymous", r#"{"name":"x"}"#);
    assert!(layout.scan().is_empty());
}

#[test]
fn never_spawns_hc() {
    let mut layout = Layout::new();
    let fake = FakeHc::start();
    let shim_dir = layout.root.join("shim");
    let marker = layout.root.join("hc-was-run");
    std::fs::create_dir_all(&shim_dir).unwrap();
    let shim = shim_dir.join("hc");
    std::fs::write(
        &shim,
        format!("#!/bin/sh\ntouch {}\nexit 1\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    layout.set("PATH", shim_dir.as_os_str());
    layout.set("HC_SOCKET", fake.socket().as_os_str());
    layout.core_json("alpha", &identity("core-0001"));
    layout.core_json("stale", &identity("core-9999"));

    assert_eq!(layout.scan().len(), 1);
    assert!(!marker.exists(), "discovery ran hc");
}

#[test]
fn id_mismatch_drops_core() {
    let mut layout = Layout::new();
    let fake = FakeHc::start();
    layout.set("HC_SOCKET", fake.socket().as_os_str());
    layout.core_json("alpha", &identity("core-other"));
    assert!(layout.scan().is_empty());
    // The same socket under the matching id is kept.
    layout.core_json("alpha", &identity("core-0001"));
    assert_eq!(layout.scan().len(), 1);
}

#[test]
fn duplicates_dedupe_by_id() {
    let mut layout = Layout::new();
    let fake = FakeHc::start();
    layout.set("HC_SOCKET", fake.socket().as_os_str());
    layout.core_json("beta", &identity("core-0001"));
    layout.core_json("alpha", &identity("core-0001"));
    let found = layout.scan();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].session, "alpha");
}
