use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn manifest_matches_pinned_091_documented_argv_shape() {
    let parse = Command::new("python3")
        .args([
            "-c",
            "import json,sys,tomllib; print(json.dumps(tomllib.load(open(sys.argv[1], 'rb'))))",
        ])
        .arg(root().join("herdr-plugin.toml"))
        .output()
        .unwrap();
    assert!(
        parse.status.success(),
        "{}",
        String::from_utf8_lossy(&parse.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&parse.stdout).unwrap();
    assert_eq!(doc["id"], "herdr-threads");
    assert_eq!(doc["name"], "Threads");
    assert_eq!(doc["version"], "0.1.0");
    assert_eq!(doc["min_herdr_version"], "0.9.1");
    assert_eq!(doc["platforms"], serde_json::json!(["macos"]));
    assert_eq!(
        doc["build"][0]["command"],
        serde_json::json!(["./scripts/build.sh"])
    );
    assert_eq!(
        doc["startup"][0]["command"],
        serde_json::json!(["./scripts/view.sh", "ensure"])
    );
    assert_eq!(doc["panes"][0]["placement"], "overlay");
    assert_eq!(
        doc["panes"][0]["command"],
        serde_json::json!(["./scripts/view.sh", "view"])
    );
    let action_ids: Vec<_> = doc["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action["id"].as_str().unwrap())
        .collect();
    assert_eq!(action_ids, ["health", "doctor", "ensure", "stop", "view"]);
    for (id, argv) in [
        ("health", serde_json::json!(["./scripts/view.sh", "health"])),
        ("doctor", serde_json::json!(["./scripts/view.sh", "doctor"])),
        ("ensure", serde_json::json!(["./scripts/view.sh", "ensure"])),
        ("stop", serde_json::json!(["./scripts/view.sh", "stop"])),
        ("view", serde_json::json!(["./scripts/view.sh", "open"])),
    ] {
        let action = doc["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["id"] == id)
            .unwrap();
        assert_eq!(action["command"], argv, "action {id}");
    }
    for command in doc["build"]
        .as_array()
        .unwrap()
        .iter()
        .chain(doc["startup"].as_array().unwrap())
        .chain(doc["actions"].as_array().unwrap())
        .chain(doc["panes"].as_array().unwrap())
    {
        let argv = command["command"].as_array().unwrap();
        assert!(!argv.is_empty());
        for part in argv {
            assert!(part.as_str().unwrap().find('$').is_none());
        }
        let executable = root().join(argv[0].as_str().unwrap());
        assert!(executable.exists(), "{}", executable.display());
        assert_ne!(
            fs::metadata(&executable).unwrap().permissions().mode() & 0o111,
            0
        );
    }
}

fn installed_herdr() -> PathBuf {
    let which = Command::new("which").arg("herdr").output().unwrap();
    assert!(which.status.success(), "installed Herdr binary is required");
    let path = PathBuf::from(String::from_utf8(which.stdout).unwrap().trim());
    let path = fs::canonicalize(path).unwrap();
    let version = Command::new(&path).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "herdr 0.9.1"
    );
    let hash = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
    assert_eq!(
        hash, "5fc7a7e7adfaca56fa80aa89dcb025693357268dab8285b9ce2d08a2313c89de",
        "installed Herdr differs from the pinned 0.9.1 binary"
    );
    eprintln!(
        "installed Herdr: {}; version: herdr 0.9.1; sha256: {}",
        path.display(),
        hash
    );
    path
}

fn offline_link(
    binary: &Path,
    package: &Path,
    isolation: &Path,
    sentinel: &Path,
) -> std::process::Output {
    let config = isolation.join("config");
    let state = isolation.join("state");
    let runtime = isolation.join("runtime");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::create_dir_all(&runtime).unwrap();
    // macOS sockaddr_un has a short path limit; fixture() uses a short private /tmp path.
    let socket = isolation.join("s.sock");
    assert!(!socket.exists());
    let args = ["plugin", "link", package.to_str().unwrap(), "--disabled"];
    let fake_tools = sentinel.parent().unwrap().join("fake-tools");
    fs::create_dir_all(&fake_tools).unwrap();
    let cargo = fake_tools.join("cargo");
    fs::write(
        &cargo,
        "#!/bin/sh\nprintf 'build ran\\n' >> \"$CALL_LOG\"\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        fake_tools.display(),
        std::env::var("PATH").unwrap()
    );
    eprintln!("offline Herdr argv: {:?} {:?}", binary, args);
    let output = Command::new(binary)
        .args(args)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", &state)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("HERDR_CONFIG_PATH", isolation.join("absent-config.toml"))
        .env("HERDR_SOCKET_PATH", &socket)
        .env("CALL_LOG", sentinel)
        .env("PATH", path)
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_CLIENT_SOCKET_PATH")
        .env_remove("HERDR_SESSION")
        .env_remove("HERDR_WORKSPACE_ID")
        .env_remove("HERDR_TAB_ID")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_STARTUP_CWD")
        .env_remove("HERDR_BIN_PATH")
        .output()
        .unwrap();
    assert!(!socket.exists(), "link must not start a server");
    eprintln!(
        "offline Herdr exit: {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn installed_herdr_semantically_accepts_only_valid_disabled_package() {
    let binary = installed_herdr();
    let dir = fixture();
    let package = dir.clone();
    fs::copy(
        root().join("herdr-plugin.toml"),
        package.join("herdr-plugin.toml"),
    )
    .unwrap();
    eprintln!(
        "manifest sha256: {:x}",
        Sha256::digest(fs::read(package.join("herdr-plugin.toml")).unwrap())
    );
    let sentinel = dir.join("startup-or-build-ran");
    let isolated = dir.join("isolated valid home");
    let output = offline_link(&binary, &package, &isolated, &sentinel);
    assert!(
        output.status.success(),
        "valid package rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let registry_path = isolated.join("config/herdr/plugins.json");
    let registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    let entries = registry.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["plugin_id"], "herdr-threads");
    assert_eq!(entries[0]["enabled"], false);
    assert_eq!(
        entries[0]["manifest_path"],
        fs::canonicalize(package.join("herdr-plugin.toml"))
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(
        entries[0]["build"][0]["command"],
        serde_json::json!(["./scripts/build.sh"])
    );
    assert_eq!(
        entries[0]["startup"][0]["command"],
        serde_json::json!(["./scripts/view.sh", "ensure"])
    );
    assert_eq!(
        entries[0]["panes"][0]["command"],
        serde_json::json!(["./scripts/view.sh", "view"])
    );
    assert_eq!(entries[0]["panes"][0]["placement"], "overlay");
    for (id, argv) in [
        ("health", serde_json::json!(["./scripts/view.sh", "health"])),
        ("doctor", serde_json::json!(["./scripts/view.sh", "doctor"])),
        ("ensure", serde_json::json!(["./scripts/view.sh", "ensure"])),
        ("stop", serde_json::json!(["./scripts/view.sh", "stop"])),
        ("view", serde_json::json!(["./scripts/view.sh", "open"])),
    ] {
        let entry = entries[0]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == id)
            .unwrap();
        assert_eq!(entry["command"], argv, "normalized action {id}");
    }
    assert!(!sentinel.exists(), "disabled link ran startup or build");

    let invalid = dir.join("invalid package with spaces");
    fs::create_dir_all(&invalid).unwrap();
    let manifest = fs::read_to_string(root().join("herdr-plugin.toml")).unwrap();
    let duplicated = manifest.replacen("id = \"doctor\"", "id = \"health\"", 1);
    assert_ne!(duplicated, manifest);
    fs::write(invalid.join("herdr-plugin.toml"), duplicated).unwrap();
    let invalid_home = dir.join("isolated invalid home");
    let rejected = offline_link(&binary, &invalid, &invalid_home, &sentinel);
    assert!(
        !rejected.status.success(),
        "duplicate action ID was accepted"
    );
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("duplicate"),
        "unexpected rejection: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(
        !invalid_home.join("config/herdr/plugins.json").exists(),
        "invalid package entered registry"
    );
    assert!(!sentinel.exists(), "invalid link ran startup or build");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn view_action_opens_declared_pane_through_herdr_binary() {
    let dir = fixture();
    let log = dir.join("calls.txt");
    let herdr = dir.join("fake herdr");
    fs::write(
        &herdr,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$CALL_LOG\"\n",
    )
    .unwrap();
    fs::set_permissions(&herdr, fs::Permissions::from_mode(0o755)).unwrap();
    let status = Command::new(dir.join("scripts/view.sh"))
        .arg("open")
        .env("HERDR_BIN_PATH", &herdr)
        .env("HERDR_PLUGIN_STATE_DIR", dir.join("state with space"))
        .env("HERDR_SOCKET_PATH", dir.join("socket with space"))
        .env("CALL_LOG", &log)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "plugin",
            "pane",
            "open",
            "--plugin",
            "herdr-threads",
            "--entrypoint",
            "operator"
        ]
    );
    fs::remove_dir_all(dir).unwrap();
}

fn fixture() -> std::path::PathBuf {
    let dir = PathBuf::from(format!("/tmp/hp {}", uuid::Uuid::new_v4().simple()));
    let mut private = fs::DirBuilder::new();
    private.mode(0o700);
    private.create(&dir).unwrap();
    let metadata = fs::symlink_metadata(&dir).unwrap();
    assert!(metadata.file_type().is_dir());
    let uid = Command::new("id").arg("-u").output().unwrap();
    assert!(uid.status.success());
    assert_eq!(
        metadata.uid().to_string(),
        String::from_utf8(uid.stdout).unwrap().trim()
    );
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    fs::create_dir_all(dir.join("scripts")).unwrap();
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::copy(root().join("scripts/view.sh"), dir.join("scripts/view.sh")).unwrap();
    fs::copy(
        root().join("scripts/build.sh"),
        dir.join("scripts/build.sh"),
    )
    .unwrap();
    fs::set_permissions(
        dir.join("scripts/view.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::set_permissions(
        dir.join("scripts/build.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::write(
        dir.join("bin/herdr-threads"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$CALL_LOG\"\n",
    )
    .unwrap();
    fs::set_permissions(
        dir.join("bin/herdr-threads"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    dir
}

#[test]
fn fixture_root_is_private_under_permissive_umask() {
    if std::env::var_os("HERDR_PACKAGE_UMASK_CHILD").is_none() {
        let output = Command::new("sh")
            .args([
                "-c",
                "umask 000; exec \"$1\" --exact manifest::fixture_root_is_private_under_permissive_umask --nocapture",
                "sh",
            ])
            .arg(std::env::current_exe().unwrap())
            .env("HERDR_PACKAGE_UMASK_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "permissive-umask child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let dir = fixture();
    let metadata = fs::symlink_metadata(&dir).unwrap();
    assert!(metadata.file_type().is_dir());
    let uid = Command::new("id").arg("-u").output().unwrap();
    assert!(uid.status.success());
    assert_eq!(
        metadata.uid().to_string(),
        String::from_utf8(uid.stdout).unwrap().trim()
    );
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn build_is_locked_and_failure_preserves_prior_artifact_without_runtime_context() {
    let dir = fixture();
    fs::create_dir_all(dir.join("target/release")).unwrap();
    fs::create_dir_all(dir.join("fake-tools")).unwrap();
    fs::write(dir.join("target/release/herdr-threads"), b"new binary").unwrap();
    fs::write(dir.join("bin/herdr-threads"), b"old binary").unwrap();
    let cargo = dir.join("fake-tools/cargo");
    fs::write(
        &cargo,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CALL_LOG\"\nexit \"${FAKE_CARGO_STATUS:-0}\"\n",
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let log = dir.join("cargo.log");
    let path = format!(
        "{}:{}",
        dir.join("fake-tools").display(),
        std::env::var("PATH").unwrap()
    );
    let success = Command::new(dir.join("scripts/build.sh"))
        .env("PATH", &path)
        .env("CALL_LOG", &log)
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .status()
        .unwrap();
    assert!(success.success());
    assert_eq!(
        fs::read(dir.join("bin/herdr-threads")).unwrap(),
        b"new binary"
    );
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "build --release --locked\n"
    );
    fs::write(dir.join("bin/herdr-threads"), b"old binary").unwrap();
    let failure = Command::new(dir.join("scripts/build.sh"))
        .env("PATH", &path)
        .env("CALL_LOG", &log)
        .env("FAKE_CARGO_STATUS", "42")
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .status()
        .unwrap();
    assert!(!failure.success());
    assert_eq!(
        fs::read(dir.join("bin/herdr-threads")).unwrap(),
        b"old binary"
    );
    fs::remove_dir_all(dir).unwrap();
}

/// Kills: a build script that lets an inherited `CARGO_TARGET_DIR` redirect
/// cargo's output away from `target/`, so the installed `bin/herdr-threads`
/// is copied from a missing or stale `target/release` (the clean Herdr
/// install fails, or ships an old binary, whenever the user exports it).
#[test]
fn build_ignores_inherited_cargo_target_dir() {
    let dir = fixture();
    fs::create_dir_all(dir.join("fake-tools")).unwrap();
    let cargo = dir.join("fake-tools/cargo");
    // The fake writes the artifact wherever cargo would: the effective target dir.
    fs::write(
        &cargo,
        "#!/bin/sh\nprintf '%s\\n' \"${CARGO_TARGET_DIR:-unset}\" >> \"$CALL_LOG\"\n\
         out=\"${CARGO_TARGET_DIR:-target}/release\"\nmkdir -p \"$out\"\n\
         printf 'fresh binary' > \"$out/herdr-threads\"\n",
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir_all(dir.join("target/release")).unwrap();
    fs::write(dir.join("target/release/herdr-threads"), b"stale binary").unwrap();
    let foreign = dir.join("foreign target");
    let log = dir.join("cargo.log");
    let path = format!(
        "{}:{}",
        dir.join("fake-tools").display(),
        std::env::var("PATH").unwrap()
    );
    let status = Command::new(dir.join("scripts/build.sh"))
        .env("PATH", &path)
        .env("CALL_LOG", &log)
        .env("CARGO_TARGET_DIR", &foreign)
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_SOCKET_PATH")
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        fs::read(dir.join("bin/herdr-threads")).unwrap(),
        b"fresh binary",
        "installed artifact must come from this build"
    );
    let canonical = fs::canonicalize(&dir).unwrap();
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!("{}\n", canonical.join("target").display())
    );
    assert!(!foreign.exists(), "inherited target dir was used");
    fs::remove_dir_all(dir).unwrap();
}

/// Kills: a release archive (scripts/package-release.sh, no sources) whose
/// manifest build command runs cargo anyway and fails, or replaces the shipped
/// executable; and a PREBUILT package without an executable that "succeeds".
#[test]
fn build_keeps_the_prebuilt_release_executable() {
    let dir = fixture();
    fs::create_dir_all(dir.join("fake-tools")).unwrap();
    let cargo = dir.join("fake-tools/cargo");
    fs::write(&cargo, "#!/bin/sh\necho cargo >> \"$CALL_LOG\"\nexit 1\n").unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(dir.join("PREBUILT"), "prebuilt\n").unwrap();
    let shipped = fs::read(dir.join("bin/herdr-threads")).unwrap();
    let log = dir.join("cargo.log");
    let path = format!(
        "{}:{}",
        dir.join("fake-tools").display(),
        std::env::var("PATH").unwrap()
    );
    let build = || {
        Command::new(dir.join("scripts/build.sh"))
            .env("PATH", &path)
            .env("CALL_LOG", &log)
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .output()
            .unwrap()
    };
    let kept = build();
    assert!(kept.status.success(), "{kept:?}");
    assert_eq!(fs::read(dir.join("bin/herdr-threads")).unwrap(), shipped);
    assert!(!log.exists(), "cargo ran for a prebuilt package");

    fs::remove_file(dir.join("bin/herdr-threads")).unwrap();
    let missing = build();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing bin/herdr-threads"));
    assert!(!log.exists(), "cargo ran for a prebuilt package");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn startup_quotes_runtime_paths_and_exits_once() {
    let dir = fixture();
    let log = dir.join("calls.txt");
    let status = Command::new(dir.join("scripts/view.sh"))
        .arg("ensure")
        .env("HERDR_PLUGIN_STATE_DIR", dir.join("state with space"))
        .env("HERDR_SOCKET_PATH", dir.join("socket with space"))
        .env("CALL_LOG", &log)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "--state-dir",
            dir.join("state with space").to_str().unwrap(),
            "--host-endpoint",
            dir.join("socket with space").to_str().unwrap(),
            "daemon",
            "ensure"
        ]
    );
    fs::write(
        dir.join("bin/herdr-threads"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$CALL_LOG\"\nexit 47\n",
    )
    .unwrap();
    fs::set_permissions(
        dir.join("bin/herdr-threads"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let failure = Command::new(dir.join("scripts/view.sh"))
        .arg("ensure")
        .env("HERDR_PLUGIN_STATE_DIR", dir.join("state with space"))
        .env("HERDR_SOCKET_PATH", dir.join("socket with space"))
        .env("CALL_LOG", &log)
        .status()
        .unwrap();
    assert_eq!(failure.code(), Some(47), "ensure failure must propagate");
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter(|line| *line == "ensure")
            .count(),
        2
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn health_doctor_and_stop_use_exact_cli_argv() {
    let dir = fixture();
    for (action, tail) in [
        ("health", vec!["daemon", "health"]),
        ("doctor", vec!["doctor"]),
        ("stop", vec!["daemon", "stop"]),
    ] {
        let log = dir.join(format!("{action}.log"));
        let status = Command::new(dir.join("scripts/view.sh"))
            .arg(action)
            .env("HERDR_PLUGIN_STATE_DIR", dir.join("state with space"))
            .env("HERDR_SOCKET_PATH", dir.join("socket with space"))
            .env("CALL_LOG", &log)
            .status()
            .unwrap();
        assert!(status.success(), "{action}");
        let state = dir.join("state with space");
        let socket = dir.join("socket with space");
        let mut expected = vec![
            "--state-dir",
            state.to_str().unwrap(),
            "--host-endpoint",
            socket.to_str().unwrap(),
        ];
        expected.extend(tail);
        assert_eq!(
            fs::read_to_string(&log)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            expected,
            "{action}"
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn view_refreshes_on_enter_and_exits_on_q() {
    let dir = fixture();
    let log = dir.join("calls.txt");
    let mut child = Command::new(dir.join("scripts/view.sh"))
        .arg("view")
        .env("HERDR_PLUGIN_STATE_DIR", dir.join("state with space"))
        .env("HERDR_SOCKET_PATH", dir.join("socket with space"))
        .env("CALL_LOG", &log)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(b"\nq\n").unwrap();
    assert!(child.wait().unwrap().success());
    let calls = fs::read_to_string(&log).unwrap();
    assert_eq!(calls.lines().filter(|line| *line == "view").count(), 2);
    assert_eq!(calls.lines().filter(|line| *line == "--once").count(), 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unrelated_view_input_does_not_refresh() {
    let dir = fixture();
    let log = dir.join("calls.txt");
    let mut child = Command::new(dir.join("scripts/view.sh"))
        .arg("view")
        .env("HERDR_PLUGIN_STATE_DIR", dir.join("state with space"))
        .env("HERDR_SOCKET_PATH", dir.join("socket with space"))
        .env("CALL_LOG", &log)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"unknown\nq\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter(|line| *line == "view")
            .count(),
        1
    );
    fs::remove_dir_all(dir).unwrap();
}

/// ht-4is.11.1 N2: the release-absence proof for crash-boundary failpoints
/// runs as part of the package suite instead of as a manual script.
/// Kills: a failpoint hook, its registry or its error text reachable in an
/// ordinary (no-feature) release build, and a positive control that silently
/// stops covering every hook named in source.
#[test]
fn failpoints_are_compiled_out_of_release_builds() {
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("target directory")
        .to_path_buf();
    let output = Command::new("sh")
        .arg(root().join("tests/service/check_failpoints_absent.sh"))
        .arg(&target)
        .env_remove("CARGO_TARGET_DIR")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "status {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    let count = stdout
        .lines()
        .find_map(|line| line.strip_prefix("failpoint hooks in source: "))
        .and_then(|count| count.trim().parse::<usize>().ok())
        .expect("hook count");
    assert!(count >= 10, "{stdout}");
    assert!(
        stdout.contains(&format!(
            "positive control (test-support build) contains {count}/{count} hook names"
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("contains 0/{count} failpoint names")),
        "{stdout}"
    );
}
