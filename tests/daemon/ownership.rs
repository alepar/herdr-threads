use super::*;
use crate::daemon::paths::{InstancePaths, RuntimeContext};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("herdr-owner-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn paths(root: &std::path::Path, endpoint: &str) -> InstancePaths {
    let context =
        RuntimeContext::explicit(root.to_path_buf(), PathBuf::from(endpoint), None).unwrap();
    InstancePaths::resolve(&context).unwrap()
}

#[test]
fn existing_namespace_read_is_bounded_and_private() {
    let root = fixture();
    let paths = paths(&root, "/tmp/herdr-test-namespace.sock");
    assert_eq!(read_existing_namespace(&paths).unwrap(), None);
    let owner = OwnerLock::acquire(&paths).unwrap();
    assert_eq!(
        read_existing_namespace(&paths).unwrap(),
        Some(owner.instance_uuid())
    );
    fs::write(&paths.namespace_path, vec![b'a'; 8193]).unwrap();
    assert_eq!(
        read_existing_namespace(&paths).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    fs::write(&paths.namespace_path, [0xff]).unwrap();
    assert_eq!(
        read_existing_namespace(&paths).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    fs::set_permissions(&paths.namespace_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        read_existing_namespace(&paths).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    fs::remove_file(&paths.namespace_path).unwrap();
    symlink(&paths.lock_path, &paths.namespace_path).unwrap();
    assert_eq!(
        read_existing_namespace(&paths).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_lock_probe_requires_same_validated_inode() {
    let root = fixture();
    let paths = paths(&root, "/tmp/herdr-test-probe.sock");
    assert_eq!(
        owner_lock_identity(&paths).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert!(!paths.lock_path.exists());
    let owner = OwnerLock::acquire(&paths).unwrap();
    let identity = owner_lock_identity(&paths).unwrap();
    assert!(!previous_owner_released(&paths, identity).unwrap());
    fs::remove_file(&paths.lock_path).unwrap();
    assert_eq!(
        previous_owner_released(&paths, identity)
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    fs::write(&paths.lock_path, b"").unwrap();
    fs::set_permissions(&paths.lock_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        previous_owner_released(&paths, identity)
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_lock_probe_observes_release_without_modifying_state() {
    let root = fixture();
    let paths = paths(&root, "/tmp/herdr-test-release.sock");
    let owner = OwnerLock::acquire(&paths).unwrap();
    let identity = owner_lock_identity(&paths).unwrap();
    let namespace = fs::read(&paths.namespace_path).unwrap();
    drop(owner);
    assert!(previous_owner_released(&paths, identity).unwrap());
    assert_eq!(fs::read(&paths.namespace_path).unwrap(), namespace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn elected_owner_removes_only_its_exact_unpublished_bound_socket() {
    let root = fixture();
    let paths = paths(&root, "/tmp/herdr-test-unpublished-cleanup.sock");
    let owner = OwnerLock::acquire(&paths).unwrap();
    let listener = owner.bind_socket().unwrap();
    let socket = listener.path().to_path_buf();
    assert!(socket.exists());
    assert!(!paths.descriptor_path.exists());
    owner.remove_unpublished_bound_socket(&listener).unwrap();
    assert!(!socket.exists());
    assert!(!paths.descriptor_path.exists());
    assert_eq!(
        OwnerLock::acquire(&paths).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(listener);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unpublished_cleanup_preserves_substituted_path() {
    let root = fixture();
    let paths = paths(&root, "/tmp/herdr-test-unpublished-substitute.sock");
    let owner = OwnerLock::acquire(&paths).unwrap();
    let listener = owner.bind_socket().unwrap();
    let socket = listener.path().to_path_buf();
    fs::remove_file(&socket).unwrap();
    fs::write(&socket, b"foreign-file").unwrap();
    let error = owner
        .remove_unpublished_bound_socket(&listener)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read(&socket).unwrap(), b"foreign-file");
    drop(listener);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_publication_cleanup_removes_only_matching_bound_descriptor() {
    let root = fixture();
    let paths = paths(&root, "/tmp/herdr-test-failed-publication.sock");
    let owner = OwnerLock::acquire(&paths).unwrap();
    let listener = owner.bind_socket().unwrap();
    let socket = listener.path().to_path_buf();
    owner
        .publish_endpoint(&listener, "test", crate::protocol::wire::PROTOCOL_VERSION)
        .unwrap();
    owner.remove_failed_bound_publication(&listener).unwrap();
    assert!(!socket.exists());
    assert!(!paths.descriptor_path.exists());
    drop(listener);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_locator_isolates_instances_and_converges_across_worktrees() {
    let root = fixture();
    let first = paths(&root, "/tmp/herdr-test-host-a.sock");
    let again = paths(&root, "/tmp/herdr-test-host-a.sock");
    let second = paths(&root, "/tmp/herdr-test-host-b.sock");
    assert_eq!(first.instance_dir, again.instance_dir);
    assert_ne!(first.instance_dir, second.instance_dir);
    let owner = OwnerLock::acquire(&first).unwrap();
    let uuid = owner.instance_uuid();
    drop(owner);
    assert_eq!(OwnerLock::acquire(&again).unwrap().instance_uuid(), uuid);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn absent_explicit_state_directory_is_created_privately() {
    let root = fixture();
    let state = root.join("new-state");
    let instance = paths(&state, "/tmp/herdr-test-new-state.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(!owner.instance_uuid().is_nil());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn detached_runtime_rejects_relative_host_binary() {
    let root = fixture();
    assert!(
        RuntimeContext::explicit(
            root.clone(),
            PathBuf::from("/tmp/herdr-test-bin.sock"),
            Some(PathBuf::from("bin/herdr")),
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_locator_and_symlink_are_rejected() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-host-c.sock");
    fs::create_dir_all(instance.instance_dir.parent().unwrap()).unwrap();
    fs::create_dir(&instance.instance_dir).unwrap();
    fs::set_permissions(&instance.instance_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(instance.instance_dir.join("locator"), b"wrong host").unwrap();
    assert!(OwnerLock::acquire(&instance).is_err());
    fs::remove_dir_all(&instance.instance_dir).unwrap();
    symlink(&root, &instance.instance_dir).unwrap();
    assert!(OwnerLock::acquire(&instance).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn permissive_instance_directory_is_rejected() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-permissions.sock");
    fs::create_dir_all(&instance.instance_dir).unwrap();
    fs::set_permissions(&instance.instance_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        OwnerLock::acquire(&instance).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn long_state_path_uses_private_short_socket_path() {
    let root = fixture();
    let long = root.join("x".repeat(80)).join("y".repeat(80));
    fs::create_dir_all(&long).unwrap();
    fs::set_permissions(&long, fs::Permissions::from_mode(0o700)).unwrap();
    let instance = paths(&long, "/tmp/herdr-test-host-long.sock");
    assert!(instance.socket_path.as_os_str().len() < 100);
    assert!(!instance.socket_path.starts_with(&long));
    let parent = instance.socket_path.parent().unwrap();
    assert_eq!(
        fs::metadata(parent).unwrap().permissions().mode() & 0o777,
        0o700
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn isolated_long_state_roots_do_not_share_fallback_socket() {
    let root = fixture();
    let first = root.join("a".repeat(80));
    let second = root.join("b".repeat(80));
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    fs::set_permissions(&first, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&second, fs::Permissions::from_mode(0o700)).unwrap();
    let left = paths(&first, "/tmp/herdr-test-same-host.sock");
    let right = paths(&second, "/tmp/herdr-test-same-host.sock");
    assert_ne!(left.socket_path, right.socket_path);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn held_lock_elects_one_owner_and_releases_after_exit() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-lock.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let error = OwnerLock::acquire(&instance).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    drop(owner);
    assert!(OwnerLock::acquire(&instance).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bound_listener_keeps_owner_lock_until_listener_drops() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-listener-lock.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    drop(owner);
    assert_eq!(
        OwnerLock::acquire(&instance).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let socket = bound.path().to_owned();
    drop(bound);
    assert!(OwnerLock::acquire(&instance).is_ok());
    fs::remove_file(socket).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn async_accept_listener_keeps_owner_lock_until_server_stops() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-clone-lock.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let socket = bound.path().to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let async_bound = runtime.block_on(async { bound.into_async().unwrap() });
    let descriptor = owner
        .publish_async_endpoint(&async_bound, "0.1.0", 1)
        .unwrap();
    drop(owner);
    assert_eq!(
        OwnerLock::acquire(&instance).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    runtime.block_on(async {
        let client = tokio::net::UnixStream::connect(&socket);
        let accepted = async_bound.accept();
        let (client, server) = tokio::join!(client, accepted);
        drop(client.unwrap());
        drop(server.unwrap());
    });
    assert_eq!(
        OwnerLock::acquire(&instance).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(async_bound);
    let next = OwnerLock::acquire(&instance).unwrap();
    assert_eq!(descriptor.endpoint, socket);
    next.remove_stale_endpoint().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_descriptor_pid_does_not_authorize_unlink_or_signal() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-stale.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let published = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    let mut unrelated = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let descriptor = EndpointDescriptor {
        software_version: "0.1.0".into(),
        protocol_version: 1,
        instance_uuid: owner.instance_uuid(),
        boot_id: published.boot_id,
        endpoint: published.endpoint.clone(),
        pid: unrelated.id(),
        socket_device: published.socket_device,
        socket_inode: published.socket_inode,
    };
    fs::write(
        &instance.descriptor_path,
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&instance.descriptor_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(owner.remove_owned_endpoint(uuid::Uuid::new_v4()).is_err());
    assert!(instance.descriptor_path.exists());
    assert_eq!(
        read_descriptor(&instance, owner.instance_uuid())
            .unwrap()
            .pid,
        unrelated.id()
    );
    owner.remove_owned_endpoint(published.boot_id).unwrap();
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "descriptor PID was signaled"
    );
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn child_binds_for_crash_case() {
    let Ok(root) = std::env::var("HERDR_OWNER_CRASH_ROOT") else {
        return;
    };
    let mode = std::env::var("HERDR_OWNER_CRASH_MODE").unwrap();
    let instance = paths(
        std::path::Path::new(&root),
        "/tmp/herdr-test-crash-socket.sock",
    );
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    if mode == "published" {
        owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    }
    let temporary = std::path::Path::new(&root).join("socket-path.tmp");
    fs::write(&temporary, bound.path().to_string_lossy().as_bytes()).unwrap();
    fs::rename(&temporary, std::path::Path::new(&root).join("socket-path")).unwrap();
    std::thread::sleep(Duration::from_secs(30));
}

#[test]
fn marker_creation_can_precede_marker_contents() {
    let root = fixture();
    let marker = root.join("socket-path");
    let (created_tx, created_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn({
        let marker = marker.clone();
        move || {
            let mut file = fs::File::create(&marker).unwrap();
            created_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            use std::io::Write;
            file.write_all(b"/tmp/complete.sock").unwrap();
        }
    });
    created_rx.recv().unwrap();
    assert!(marker.exists());
    assert!(fs::read(&marker).unwrap().is_empty());
    finish_tx.send(()).unwrap();
    writer.join().unwrap();
    assert_eq!(fs::read(&marker).unwrap(), b"/tmp/complete.sock");
    fs::remove_dir_all(root).unwrap();
}

fn crashed_owner_recovers(mode: &str) {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-crash-socket.sock");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::ownership::tests::child_binds_for_crash_case",
            "--nocapture",
        ])
        .env("HERDR_OWNER_CRASH_ROOT", &root)
        .env("HERDR_OWNER_CRASH_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let marker = root.join("socket-path");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let marker_bytes = fs::read(&marker).unwrap_or_default();
    let old_path = String::from_utf8(marker_bytes).ok().map(PathBuf::from);
    let bound_before_crash = old_path.as_ref().is_some_and(|path| path.exists());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(marker.exists(), "child did not publish socket marker");
    assert!(
        bound_before_crash,
        "child socket missing before crash or marker incomplete"
    );
    let old_path = old_path.unwrap();
    let owner = OwnerLock::acquire(&instance).unwrap();
    owner.remove_stale_endpoint().unwrap();
    assert_eq!(old_path.exists(), mode == "unpublished");
    // The pathname is stable: the next owner reclaims a dead unpublished
    // socket and binds the same name; only the boot identity changes.
    let next = owner.bind_socket().unwrap();
    assert_eq!(next.path(), old_path);
    assert_eq!(next.path(), instance.socket_path);
    let descriptor = owner.publish_endpoint(&next, "0.1.0", 1).unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.instance_uuid())
            .unwrap()
            .boot_id,
        descriptor.boot_id
    );
    owner.remove_owned_endpoint(descriptor.boot_id).unwrap();
    if old_path.exists() {
        fs::remove_file(old_path).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn crash_before_publication_does_not_block_restart() {
    crashed_owner_recovers("unpublished");
}

#[test]
fn crash_after_publication_cleans_verified_stale_socket() {
    crashed_owner_recovers("published");
}

#[test]
fn stale_cleanup_preserves_unknown_socket_without_descriptor() {
    let root = fixture();
    let endpoint = format!("/tmp/herdr-test-symlink-{}.sock", uuid::Uuid::new_v4());
    let instance = paths(&root, &endpoint);
    let owner = OwnerLock::acquire(&instance).unwrap();
    let protected = root.join("protected");
    fs::write(&protected, b"must survive").unwrap();
    symlink(&protected, &instance.socket_path).unwrap();
    owner.remove_stale_endpoint().unwrap();
    assert!(instance.socket_path.exists());
    // Binding the stable pathname refuses (never follows or removes) a symlink.
    assert_eq!(
        owner.bind_socket().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(
        fs::symlink_metadata(&instance.socket_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&protected).unwrap(), b"must survive");
    fs::remove_file(&instance.socket_path).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn child_holds_lock() {
    let Ok(root) = std::env::var("HERDR_OWNER_TEST_ROOT") else {
        return;
    };
    let instance = paths(
        std::path::Path::new(&root),
        "/tmp/herdr-test-child-lock.sock",
    );
    let _owner = OwnerLock::acquire(&instance).unwrap();
    fs::write(std::path::Path::new(&root).join("ready"), b"held").unwrap();
    std::thread::sleep(Duration::from_secs(30));
}

#[test]
fn crashed_child_releases_kernel_lock() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-child-lock.sock");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::ownership::tests::child_holds_lock",
            "--nocapture",
        ])
        .env("HERDR_OWNER_TEST_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !root.join("ready").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(root.join("ready").exists(), "child did not acquire lock");
    assert_eq!(
        OwnerLock::acquire(&instance).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(OwnerLock::acquire(&instance).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_and_wrong_instance_descriptors_are_rejected() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-descriptor.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    fs::write(&instance.descriptor_path, b"{corrupt").unwrap();
    fs::set_permissions(&instance.descriptor_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(read_descriptor(&instance, owner.instance_uuid()).is_err());
    assert!(owner.remove_stale_endpoint().is_err());
    assert!(instance.descriptor_path.exists());
    let wrong = EndpointDescriptor {
        software_version: "0.1.0".into(),
        protocol_version: 1,
        instance_uuid: uuid::Uuid::new_v4(),
        boot_id: uuid::Uuid::new_v4(),
        endpoint: instance.socket_path.clone(),
        pid: 1,
        socket_device: 1,
        socket_inode: 1,
    };
    fs::write(
        &instance.descriptor_path,
        serde_json::to_vec(&wrong).unwrap(),
    )
    .unwrap();
    assert!(read_descriptor(&instance, owner.instance_uuid()).is_err());
    assert!(owner.remove_stale_endpoint().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn descriptor_is_published_only_for_private_bound_socket() {
    let root = fixture();
    let endpoint = format!("/tmp/herdr-test-publish-{}.sock", uuid::Uuid::new_v4());
    let instance = paths(&root, &endpoint);
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let descriptor = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.instance_uuid()).unwrap(),
        descriptor
    );
    assert_eq!(
        fs::metadata(&instance.descriptor_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(owner.remove_owned_endpoint(uuid::Uuid::new_v4()).is_err());
    assert!(bound.path().exists());
    owner.remove_owned_endpoint(descriptor.boot_id).unwrap();
    assert!(!bound.path().exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn substituted_listener_cannot_be_published_or_removed() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-substitution.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let descriptor = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    fs::remove_file(bound.path()).unwrap();
    let _replacement = std::os::unix::net::UnixListener::bind(bound.path()).unwrap();
    fs::set_permissions(bound.path(), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(owner.publish_endpoint(&bound, "0.1.0", 1).is_err());
    assert!(owner.remove_owned_endpoint(descriptor.boot_id).is_err());
    assert!(bound.path().exists());
    fs::remove_file(bound.path()).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn foreign_listener_cannot_be_advertised() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-foreign.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    fs::remove_file(bound.path()).unwrap();
    let _foreign = std::os::unix::net::UnixListener::bind(bound.path()).unwrap();
    fs::set_permissions(bound.path(), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(owner.publish_endpoint(&bound, "0.1.0", 1).is_err());
    assert!(!instance.descriptor_path.exists());
    fs::remove_file(bound.path()).unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// The daemon socket pathname is a pure function of the instance (state
/// root + host endpoint): every boot binds the same name, so a sandbox
/// allowlist naming it stays valid across restarts, while each boot still
/// publishes a distinct boot ID. Kills: reintroducing a boot-specific name.
#[test]
fn socket_pathname_is_stable_across_boots_with_distinct_boot_ids() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-stable.sock");
    let again = paths(&root, "/tmp/herdr-test-stable.sock");
    assert_eq!(instance.socket_path, again.socket_path);
    assert_eq!(
        instance.socket_path,
        crate::daemon::paths::stable_socket_path(&instance.instance_dir).unwrap()
    );
    let owner = OwnerLock::acquire(&instance).unwrap();
    let first = owner.bind_socket().unwrap();
    let published = owner.publish_endpoint(&first, "0.1.0", 1).unwrap();
    assert_eq!(published.endpoint, instance.socket_path);
    owner.remove_owned_endpoint(published.boot_id).unwrap();
    drop(first);
    let second = owner.bind_socket().unwrap();
    let next = owner.publish_endpoint(&second, "0.1.0", 1).unwrap();
    assert_eq!(next.endpoint, published.endpoint);
    assert_ne!(next.boot_id, published.boot_id);
    owner.remove_owned_endpoint(next.boot_id).unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// A descriptor naming any other pathname than the instance's stable one is
/// rejected, so a descriptor cannot redirect clients to a foreign socket.
#[test]
fn descriptor_endpoint_must_be_the_stable_pathname() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-endpoint-name.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let mut descriptor = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    descriptor.endpoint = instance
        .instance_dir
        .join(format!("daemon-{}.sock", descriptor.boot_id));
    fs::write(
        &instance.descriptor_path,
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.instance_uuid())
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    drop(bound);
    fs::remove_dir_all(root).unwrap();
}

/// Only a dead socket owned by this user is reclaimed at the stable name; a
/// live listener there (not holding the owner lock) or a regular file is
/// refused and left in place.
#[test]
fn stable_pathname_reclaims_only_dead_owned_sockets() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-reclaim.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    // A live foreign listener at the pathname: refused, not unlinked.
    let live = std::os::unix::net::UnixListener::bind(&instance.socket_path).unwrap();
    assert_eq!(
        owner.bind_socket().unwrap_err().kind(),
        io::ErrorKind::AddrInUse
    );
    assert!(instance.socket_path.exists());
    // The same socket once its listener is gone is dead and reclaimed.
    drop(live);
    let bound = owner.bind_socket().unwrap();
    assert_eq!(bound.path(), instance.socket_path);
    drop(bound);
    fs::remove_file(&instance.socket_path).unwrap();
    // A regular file is never removed.
    fs::write(&instance.socket_path, b"not a socket").unwrap();
    assert_eq!(
        owner.bind_socket().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(fs::read(&instance.socket_path).unwrap(), b"not a socket");
    fs::remove_dir_all(root).unwrap();
}

/// Upgrade: a descriptor published by a pre-ht-910 daemon names its per-boot
/// socket (`<stem>-<boot>.sock`). It is still read (an older running daemon
/// stays reachable and stoppable), and after a crash the next owner removes
/// that verified stale socket and binds the stable pathname. Kills: a new
/// daemon that can never start over an old crash's descriptor.
#[test]
fn legacy_per_boot_descriptor_is_read_and_cleaned_up() {
    let root = fixture();
    let instance = paths(&root, "/tmp/herdr-test-legacy.sock");
    let owner = OwnerLock::acquire(&instance).unwrap();
    let boot = uuid::Uuid::new_v4();
    let legacy = instance.legacy_boot_socket_path(boot).unwrap();
    assert_ne!(legacy, instance.socket_path);
    assert!(
        legacy
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(&format!("-{boot}.sock"))
    );
    let old = std::os::unix::net::UnixListener::bind(&legacy).unwrap();
    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o600)).unwrap();
    let meta = fs::metadata(&legacy).unwrap();
    use std::os::unix::fs::MetadataExt;
    let descriptor = EndpointDescriptor {
        software_version: "0.0.9".into(),
        protocol_version: 1,
        instance_uuid: owner.instance_uuid(),
        boot_id: boot,
        endpoint: legacy.clone(),
        pid: 1,
        socket_device: meta.dev(),
        socket_inode: meta.ino(),
    };
    fs::write(
        &instance.descriptor_path,
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&instance.descriptor_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.instance_uuid()).unwrap(),
        descriptor
    );
    // Any other per-boot name (wrong boot) is still rejected.
    let mut other = descriptor.clone();
    other.boot_id = uuid::Uuid::new_v4();
    fs::write(
        &instance.descriptor_path,
        serde_json::to_vec(&other).unwrap(),
    )
    .unwrap();
    assert!(read_descriptor(&instance, owner.instance_uuid()).is_err());
    fs::write(
        &instance.descriptor_path,
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    drop(old); // the old daemon crashed
    owner.remove_stale_endpoint().unwrap();
    assert!(!legacy.exists());
    assert!(!instance.descriptor_path.exists());
    let bound = owner.bind_socket().unwrap();
    assert_eq!(bound.path(), instance.socket_path);
    drop(bound);
    fs::remove_file(&instance.socket_path).unwrap();
    fs::remove_dir_all(root).unwrap();
}
