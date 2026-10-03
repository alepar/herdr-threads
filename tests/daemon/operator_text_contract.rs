use super::*;
use crate::daemon::paths::{InstancePaths, RuntimeContext};
use crate::daemon::remedy::{RemedyContext, remedy};
use crate::protocol::results::ErrorClass;
use std::path::PathBuf;

fn paths() -> (PathBuf, InstancePaths) {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("herdr-optext-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    (root, paths)
}

#[test]
fn path_accessors_have_the_contract_shape() {
    let (root, paths) = paths();
    assert_eq!(
        daemon_log_path(&paths),
        paths.instance_dir.join("daemon.log")
    );
    assert_eq!(startup_logs_dir(&paths), paths.instance_dir.join("logs"));
    let attempt = StartAttempt::new();
    let path = startup_log_path(&paths, &attempt);
    assert_eq!(path.parent().unwrap(), paths.instance_dir.join("logs"));
    let name = path.file_name().unwrap().to_str().unwrap();
    let middle = name
        .strip_prefix(&format!("startup-{}-", std::process::id()))
        .and_then(|rest| rest.strip_suffix(".log"))
        .expect("startup-<pid>-<nonce>.log");
    assert_eq!(middle.len(), 8);
    assert!(
        middle
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert_ne!(StartAttempt::new(), StartAttempt::new());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remedy_names_the_log_when_given_one() {
    let log = PathBuf::from("/x/daemon.log");
    for context in [
        RemedyContext::StartupFailure { log: log.clone() },
        RemedyContext::StartupTimeout { log: log.clone() },
        RemedyContext::LaneDegraded { log: log.clone() },
        RemedyContext::Doctor { log: log.clone() },
    ] {
        assert!(
            remedy(Some(ErrorClass::Unavailable), &context).contains("/x/daemon.log"),
            "{context:?}"
        );
    }
    // ht-p03.107: exit 3 names `ensure` for an unavailable daemon, `stop`
    // for version skew.
    let unavailable = remedy(Some(ErrorClass::Unavailable), &RemedyContext::Exit3);
    assert!(unavailable.contains("daemon ensure"), "{unavailable}");
    assert!(!unavailable.contains("daemon stop"), "{unavailable}");
    assert!(remedy(Some(ErrorClass::VersionSkew), &RemedyContext::Exit3).contains("daemon stop"));
}

#[test]
fn call_sites_use_the_accessors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "src/daemon/lifecycle.rs",
        "src/daemon/health.rs",
        "src/cli/doctor.rs",
    ] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            assert!(
                !(line.contains("daemon.log") || line.contains("startup-")),
                "{file}:{} hardcodes a log path; use daemon::logs accessors",
                number + 1
            );
        }
    }
}
