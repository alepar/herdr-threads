use super::*;
use crate::daemon::paths::{NodePaths, RuntimeContext};
use crate::test_support::spawn::SpawnOwned;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// A private 0700 state root plus a short private socket base, both removed
/// on drop (ht-p03.24: no fixed endpoints).
struct Fixture(crate::test_support::isolation::TestIsolation);

impl Fixture {
    fn sock(&self, name: &str) -> PathBuf {
        self.0.socket_path(name)
    }
}

impl std::ops::Deref for Fixture {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        self.0.state_root()
    }
}

impl AsRef<std::path::Path> for Fixture {
    fn as_ref(&self) -> &std::path::Path {
        self.0.state_root()
    }
}

fn fixture() -> Fixture {
    Fixture(crate::test_support::isolation::TestIsolation::new("owner"))
}

fn paths(root: &std::path::Path, endpoint: impl AsRef<std::path::Path>) -> NodePaths {
    let context =
        RuntimeContext::explicit(root.to_path_buf(), endpoint.as_ref().to_path_buf(), None)
            .unwrap();
    NodePaths::resolve(&context.state_dir).unwrap()
}

#[test]
fn node_hint_read_treats_missing_and_corrupt_as_unknown() {
    let root = fixture();
    let paths = paths(&root, root.sock("namespace.sock"));
    assert_eq!(read_node_uuid_hint(&paths).unwrap(), None);
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
    assert_eq!(
        read_node_uuid_hint(&paths).unwrap(),
        Some(owner.node_uuid().unwrap())
    );
    for bad in [vec![b'a'; 8193], vec![0xff], b"{\"format\":2}".to_vec()] {
        fs::write(&paths.node_json_path, &bad).unwrap();
        assert_eq!(read_node_uuid_hint(&paths).unwrap(), None);
    }
    fs::remove_file(&paths.node_json_path).unwrap();
    assert_eq!(read_node_uuid_hint(&paths).unwrap(), None);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_lease_without_a_hint_cannot_publish() {
    let root = fixture();
    let paths = paths(&root, root.sock("no-hint.sock"));
    let owner = OwnerLock::acquire(&paths).unwrap();
    assert_eq!(
        owner.node_uuid().unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    let bound = owner.bind_socket().unwrap();
    assert_eq!(
        owner
            .publish_endpoint(&bound, "test", crate::protocol::wire::PROTOCOL_VERSION)
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(!paths.descriptor_path.exists());
    drop(bound);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_lock_probe_requires_same_validated_inode() {
    let root = fixture();
    let paths = paths(&root, root.sock("probe.sock"));
    assert_eq!(
        owner_lock_identity(&paths).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert!(!paths.lock_path.exists());
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
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
    let paths = paths(&root, root.sock("release.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
    let identity = owner_lock_identity(&paths).unwrap();
    let hint = fs::read(&paths.node_json_path).unwrap();
    drop(owner);
    assert!(previous_owner_released(&paths, identity).unwrap());
    assert_eq!(fs::read(&paths.node_json_path).unwrap(), hint);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn elected_owner_removes_only_its_exact_unpublished_bound_socket() {
    let root = fixture();
    let paths = paths(&root, root.sock("unpublished-cleanup.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
    let listener = owner.bind_socket().unwrap();
    let socket = listener.path().to_path_buf();
    assert!(socket.exists());
    assert!(!paths.descriptor_path.exists());
    owner.remove_unpublished_bound_socket(&listener).unwrap();
    assert!(!socket.exists());
    assert!(!paths.descriptor_path.exists());
    assert_eq!(
        OwnerLock::acquire_with_test_hint(&paths)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    drop(listener);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unpublished_cleanup_preserves_substituted_path() {
    let root = fixture();
    let paths = paths(&root, root.sock("unpublished-substitute.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
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
    let paths = paths(&root, root.sock("failed-publication.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
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
fn absent_explicit_state_directory_is_created_privately() {
    let root = fixture();
    let state = root.join("new-state");
    let instance = paths(&state, root.sock("new-state.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(!owner.node_uuid().unwrap().is_nil());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn detached_runtime_rejects_relative_host_binary() {
    let root = fixture();
    assert!(
        RuntimeContext::explicit(
            root.to_path_buf(),
            root.sock("bin.sock"),
            Some(PathBuf::from("bin/herdr")),
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn symlinked_node_directory_is_rejected() {
    let root = fixture();
    let instance = paths(&root, root.sock("host-c.sock"));
    symlink(&root, &instance.node_dir).unwrap();
    assert!(OwnerLock::acquire_with_test_hint(&instance).is_err());
    fs::remove_file(&instance.node_dir).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn permissive_node_directory_is_rejected() {
    let root = fixture();
    let instance = paths(&root, root.sock("permissions.sock"));
    fs::create_dir_all(&instance.node_dir).unwrap();
    fs::set_permissions(&instance.node_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
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
    let instance = paths(&long, root.sock("host-long.sock"));
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
    let left = paths(&first, root.sock("same-host.sock"));
    let right = paths(&second, root.sock("same-host.sock"));
    assert_ne!(left.socket_path, right.socket_path);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn held_lock_elects_one_owner_and_releases_after_exit() {
    let root = fixture();
    let instance = paths(&root, root.sock("lock.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let error = OwnerLock::acquire_with_test_hint(&instance).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    drop(owner);
    assert!(OwnerLock::acquire_with_test_hint(&instance).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bound_listener_keeps_owner_lock_until_listener_drops() {
    let root = fixture();
    let instance = paths(&root, root.sock("listener-lock.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    drop(owner);
    assert_eq!(
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    let socket = bound.path().to_owned();
    drop(bound);
    assert!(OwnerLock::acquire_with_test_hint(&instance).is_ok());
    fs::remove_file(socket).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn async_accept_listener_keeps_owner_lock_until_server_stops() {
    let root = fixture();
    let instance = paths(&root, root.sock("clone-lock.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
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
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
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
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(async_bound);
    let next = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    assert_eq!(descriptor.endpoint, socket);
    next.remove_stale_endpoint().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_descriptor_pid_does_not_authorize_unlink_or_signal() {
    let root = fixture();
    let instance = paths(&root, root.sock("stale.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let published = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    let mut unrelated = root
        .0
        .command("/bin/sleep")
        .arg("30")
        .spawn_owned()
        .unwrap();
    let descriptor = EndpointDescriptor {
        software_version: "0.1.0".into(),
        protocol_version: 1,
        node_uuid: owner.node_uuid().unwrap(),
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
        read_descriptor(&instance, owner.node_uuid().unwrap())
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
    let Ok(root) = std::env::var("HT_OWNER_CRASH_ROOT") else {
        return;
    };
    let mode = std::env::var("HT_OWNER_CRASH_MODE").unwrap();
    let endpoint = std::env::var("HT_OWNER_CRASH_SOCKET").unwrap();
    let instance = paths(std::path::Path::new(&root), endpoint);
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
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
            file.write_all(b"complete-socket-path").unwrap();
        }
    });
    created_rx.recv().unwrap();
    assert!(marker.exists());
    assert!(fs::read(&marker).unwrap().is_empty());
    finish_tx.send(()).unwrap();
    writer.join().unwrap();
    assert_eq!(fs::read(&marker).unwrap(), b"complete-socket-path");
    fs::remove_dir_all(root).unwrap();
}

fn crashed_owner_recovers(mode: &str) {
    let root = fixture();
    let instance = paths(&root, root.sock("crash-socket.sock"));
    let mut child = root
        .0
        .command(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::ownership::tests::child_binds_for_crash_case",
            "--nocapture",
        ])
        .env("HT_OWNER_CRASH_ROOT", &*root)
        .env("HT_OWNER_CRASH_SOCKET", root.sock("crash-socket.sock"))
        .env("HT_OWNER_CRASH_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn_owned()
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
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    owner.remove_stale_endpoint().unwrap();
    assert_eq!(old_path.exists(), mode == "unpublished");
    // The pathname is stable: the next owner reclaims a dead unpublished
    // socket and binds the same name; only the boot identity changes.
    let next = owner.bind_socket().unwrap();
    assert_eq!(next.path(), old_path);
    assert_eq!(next.path(), instance.socket_path);
    let descriptor = owner.publish_endpoint(&next, "0.1.0", 1).unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.node_uuid().unwrap())
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
    let instance = paths(&root, root.sock("symlink.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
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
    let Ok(root) = std::env::var("HT_OWNER_TEST_ROOT") else {
        return;
    };
    let endpoint = std::env::var("HT_OWNER_TEST_SOCKET").unwrap();
    let instance = paths(std::path::Path::new(&root), endpoint);
    let _owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    fs::write(std::path::Path::new(&root).join("ready"), b"held").unwrap();
    std::thread::sleep(Duration::from_secs(30));
}

#[test]
fn crashed_child_releases_kernel_lock() {
    let root = fixture();
    let instance = paths(&root, root.sock("child-lock.sock"));
    let mut child = root
        .0
        .command(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::ownership::tests::child_holds_lock",
            "--nocapture",
        ])
        .env("HT_OWNER_TEST_ROOT", &*root)
        .env("HT_OWNER_TEST_SOCKET", root.sock("child-lock.sock"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn_owned()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !root.join("ready").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(root.join("ready").exists(), "child did not acquire lock");
    assert_eq!(
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(OwnerLock::acquire_with_test_hint(&instance).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_and_wrong_instance_descriptors_are_rejected() {
    let root = fixture();
    let instance = paths(&root, root.sock("descriptor.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    fs::write(&instance.descriptor_path, b"{corrupt").unwrap();
    fs::set_permissions(&instance.descriptor_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(read_descriptor(&instance, owner.node_uuid().unwrap()).is_err());
    assert!(owner.remove_stale_endpoint().is_err());
    assert!(instance.descriptor_path.exists());
    let wrong = EndpointDescriptor {
        software_version: "0.1.0".into(),
        protocol_version: 1,
        node_uuid: crate::node::NodeUuid::new_v4(),
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
    assert!(read_descriptor(&instance, owner.node_uuid().unwrap()).is_err());
    assert!(owner.remove_stale_endpoint().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn descriptor_is_published_only_for_private_bound_socket() {
    let root = fixture();
    let instance = paths(&root, root.sock("publish.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let descriptor = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.node_uuid().unwrap()).unwrap(),
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
    let instance = paths(&root, root.sock("substitution.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
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
    let instance = paths(&root, root.sock("foreign.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
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
    let instance = paths(&root, root.sock("stable.sock"));
    let again = paths(&root, root.sock("stable.sock"));
    assert_eq!(instance.socket_path, again.socket_path);
    assert_eq!(
        instance.socket_path,
        crate::daemon::paths::stable_socket_path(&instance.node_dir).unwrap()
    );
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
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
    let instance = paths(&root, root.sock("endpoint-name.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let mut descriptor = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    descriptor.endpoint = instance
        .node_dir
        .join(format!("daemon-{}.sock", descriptor.boot_id));
    fs::write(
        &instance.descriptor_path,
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.node_uuid().unwrap())
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
    let instance = paths(&root, root.sock("reclaim.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
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

/// A descriptor is accepted only for this node's exact socket pathname: the
/// same file name in another directory, and another name in the same one,
/// are both a mismatch. Kills: acceptance by file name or by directory.
#[test]
fn descriptor_naming_another_socket_is_a_mismatch() {
    use std::os::unix::fs::MetadataExt;
    let root = fixture();
    let instance = paths(&root, root.sock("exact.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let published = owner
        .publish_endpoint(&bound, "0.9.0", crate::protocol::wire::PROTOCOL_VERSION)
        .unwrap();
    assert_eq!(
        read_descriptor(&instance, owner.node_uuid().unwrap()).unwrap(),
        published
    );
    let other_dir = root.sock("rt-other");
    fs::create_dir(&other_dir).unwrap();
    fs::set_permissions(&other_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let elsewhere = other_dir.join(instance.socket_path.file_name().unwrap());
    let sibling = instance.socket_path.with_file_name("other.sock");
    for endpoint in [elsewhere, sibling] {
        let live = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
        let meta = fs::metadata(&endpoint).unwrap();
        let mut moved = published.clone();
        moved.endpoint = endpoint.clone();
        moved.socket_device = meta.dev();
        moved.socket_inode = meta.ino();
        fs::write(
            &instance.descriptor_path,
            serde_json::to_vec(&moved).unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_descriptor(&instance, owner.node_uuid().unwrap())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData,
            "{endpoint:?}"
        );
        drop(live);
        fs::remove_file(&endpoint).unwrap();
    }
    drop(bound);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}
/// The published endpoint must be a real socket owned by this user with
/// mode 0600. A symlink, a wrong-mode socket or a foreign-owned socket at the
/// socket pathname is refused by read and by stale cleanup, and nothing is
/// removed. Kills: descriptor acceptance bypassing `validate_socket`.
#[test]
fn published_endpoint_must_be_a_private_owned_socket() {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let root = fixture();
    let mut instance = paths(&root, root.sock("published-neg.sock"));
    let dir = root.sock("pn-dir");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let at = dir.join("0123456789abcdef.sock");
    instance.socket_path = at.clone();
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let publish = |instance: &NodePaths, meta: &fs::Metadata| {
        let descriptor = EndpointDescriptor {
            software_version: "0.9.0".into(),
            protocol_version: 1,
            node_uuid: owner.node_uuid().unwrap(),
            boot_id: uuid::Uuid::new_v4(),
            endpoint: instance.socket_path.clone(),
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
    };
    let refused = |instance: &NodePaths, what: &str| {
        let error = read_descriptor(instance, owner.node_uuid().unwrap()).unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::PermissionDenied,
            "{what}: {error}"
        );
        let error = owner.remove_stale_endpoint().unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::PermissionDenied,
            "{what}: {error}"
        );
        assert!(instance.descriptor_path.exists(), "{what}");
    };

    // A real owned socket elsewhere, reached through a symlink at the name.
    let target = root.sock("pn-target.sock");
    let live = std::os::unix::net::UnixListener::bind(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, &at).unwrap();
    publish(&instance, &fs::metadata(&target).unwrap());
    refused(&instance, "symlink");
    assert!(fs::symlink_metadata(&at).unwrap().file_type().is_symlink());
    assert!(target.exists());
    fs::remove_file(&at).unwrap();
    drop(live);
    fs::remove_file(&target).unwrap();

    // An owned socket at the name with a group/other-accessible mode.
    let wrong = std::os::unix::net::UnixListener::bind(&at).unwrap();
    fs::set_permissions(&at, fs::Permissions::from_mode(0o666)).unwrap();
    publish(&instance, &fs::metadata(&at).unwrap());
    refused(&instance, "wrong mode");
    assert!(at.exists());
    drop(wrong);
    fs::remove_file(&at).unwrap();

    // A socket owned by another user (any system socket not ours) as the
    // node's socket. Prefer a 0600 one, so only the owner check can refuse it.
    let sockets: Vec<(std::path::PathBuf, u32)> = ["/var/run", "/run", "/private/var/run"]
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            (meta.file_type().is_socket() && meta.uid() != crate::daemon::paths::effective_uid())
                .then(|| (entry.path(), meta.permissions().mode() & 0o777))
        })
        .collect();
    let foreign = sockets
        .iter()
        .find(|(_, mode)| *mode == 0o600)
        .or(sockets.first())
        .map(|(path, _)| path.clone());
    match foreign {
        Some(foreign) => {
            instance.socket_path = foreign.clone();
            publish(&instance, &fs::metadata(&foreign).unwrap());
            let error = read_descriptor(&instance, owner.node_uuid().unwrap()).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::PermissionDenied,
                "foreign: {error}"
            );
            assert!(foreign.exists());
        }
        None => eprintln!("no foreign-owned socket on this host; foreign case skipped"),
    }
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

/// Releasing the lease releases the lock even while another copy of its
/// descriptor exists, as when a process spawned concurrently by another
/// thread still holds the transient copy it inherited until its exec
/// completes. Kills: relying on close alone (the copy kept the lock held and
/// the immediate re-election saw WouldBlock, ht-w7n).
#[test]
fn dropping_the_lease_releases_it_despite_a_transient_descriptor_copy() {
    use std::os::fd::AsRawFd;
    let root = fixture();
    let paths = paths(&root, root.sock("lease.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
    // SAFETY: duplicates a descriptor this test owns; closed below.
    let copy = unsafe { libc::dup(owner.lock_file.0.as_raw_fd()) };
    assert!(copy >= 0);
    drop(owner);
    let next = OwnerLock::acquire_with_test_hint(&paths);
    // SAFETY: closes the duplicate made above.
    unsafe { libc::close(copy) };
    drop(next.expect("the dropped lease was released despite the descriptor copy"));
}

/// Write the published descriptor, private (0600) as the owner publishes it.
fn rewrite_descriptor(paths: &NodePaths, descriptor: &EndpointDescriptor) {
    fs::write(
        &paths.descriptor_path,
        serde_json::to_vec(descriptor).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&paths.descriptor_path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// macOS renumbers an APFS volume's st_dev across reboots while the socket
/// file keeps its inode (ht-tyt7). Under the owner lock, a dead boot's socket
/// whose device alone changed is stale, not substituted: the elected owner
/// removes it and binds a new boot. Kills: failing closed on every reboot
/// that renumbers the device (a manual rename of endpoint.json was needed).
#[test]
fn dead_boot_socket_with_renumbered_device_is_reclaimed() {
    let root = fixture();
    let instance = paths(&root, root.sock("renumbered.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let bound = owner.bind_socket().unwrap();
    let published = owner.publish_endpoint(&bound, "0.1.0", 1).unwrap();
    // The boot dies: the socket file stays, nothing listens on it.
    drop(bound);
    let renumbered = EndpointDescriptor {
        socket_device: published.socket_device ^ 1,
        ..published.clone()
    };
    rewrite_descriptor(&instance, &renumbered);
    // Clients keep the strict check and are told how to recover.
    let error = read_descriptor(&instance, owner.node_uuid().unwrap()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(is_socket_substituted(&error), "{error}");
    assert_eq!(
        error.to_string(),
        "socket pathname was substituted; run `herdr-threads daemon ensure` to reclaim it"
    );
    owner.remove_stale_endpoint().unwrap();
    assert!(fs::symlink_metadata(&instance.socket_path).is_err());
    assert!(!instance.descriptor_path.exists());
    let next = owner.bind_socket().unwrap();
    let descriptor = owner.publish_endpoint(&next, "0.1.0", 1).unwrap();
    assert_eq!(descriptor.endpoint, published.endpoint);
    owner.remove_owned_endpoint(descriptor.boot_id).unwrap();
    drop(next);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

/// The renumbered-device relaxation (ht-tyt7) removes only a dead socket that
/// is the descriptor's own inode: a live listener, a different inode (with
/// the device matching or not), a symlink, a regular file and a socket of
/// another uid are refused and left in place, with the descriptor kept.
/// Kills: accepting any inode mismatch (including weakening the inode guard
/// to apply only when the device differs too), skipping the connect probe,
/// following the pathname, or dropping the uid check.
#[test]
fn renumbered_device_never_reclaims_live_or_unsafe_artifacts() {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let root = fixture();
    let instance = paths(&root, root.sock("renumbered-neg.sock"));
    let owner = OwnerLock::acquire_with_test_hint(&instance).unwrap();
    let at = instance.socket_path.clone();
    let describe = |device: u64, inode: u64| {
        let descriptor = EndpointDescriptor {
            software_version: "0.1.0".into(),
            protocol_version: 1,
            node_uuid: owner.node_uuid().unwrap(),
            boot_id: uuid::Uuid::new_v4(),
            endpoint: at.clone(),
            pid: 4242,
            socket_device: device,
            socket_inode: inode,
        };
        rewrite_descriptor(&instance, &descriptor);
        descriptor
    };
    let refused = |kind: io::ErrorKind, what: &str| {
        let error = owner.remove_stale_endpoint().unwrap_err();
        assert_eq!(error.kind(), kind, "{what}: {error}");
        assert!(instance.descriptor_path.exists(), "{what}");
        assert!(
            fs::symlink_metadata(&at).is_ok(),
            "{what}: artifact removed"
        );
        error
    };

    // A live listener at the descriptor's inode: it answers, so it stays.
    let bound = owner.bind_socket().unwrap();
    let meta = fs::symlink_metadata(&at).unwrap();
    describe(meta.dev() ^ 1, meta.ino());
    refused(io::ErrorKind::AddrInUse, "live listener");
    // A dead socket whose inode differs is substituted, not renumbered,
    // whether or not the device matches too.
    drop(bound);
    let meta = fs::symlink_metadata(&at).unwrap();
    for (device, what) in [
        (meta.dev(), "inode mismatch, same device"),
        (meta.dev() ^ 1, "inode and device mismatch"),
    ] {
        describe(device, meta.ino() + 1);
        let error = refused(io::ErrorKind::PermissionDenied, what);
        assert!(is_socket_substituted(&error), "{what}: {error}");
        assert_eq!(
            error.to_string(),
            format!(
                "socket pathname was substituted; no daemon holds the owner lock: remove {}, then run `herdr-threads daemon ensure`",
                instance.descriptor_path.display()
            )
        );
    }
    // A socket of another uid: refused before any probe or unlink. The uid
    // is injected, so this runs on every host (no foreign socket is needed).
    let descriptor = describe(meta.dev() ^ 1, meta.ino());
    let error = remove_dead_boot_socket_as(&descriptor, &instance.descriptor_path, meta.uid() + 1)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    assert!(fs::symlink_metadata(&at).unwrap().file_type().is_socket());
    fs::remove_file(&at).unwrap();

    // A symlink to a dead socket elsewhere, described by the target's inode.
    let target = root.sock("renumbered-target.sock");
    drop(std::os::unix::net::UnixListener::bind(&target).unwrap());
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, &at).unwrap();
    let meta = fs::metadata(&target).unwrap();
    describe(meta.dev() ^ 1, meta.ino());
    refused(io::ErrorKind::PermissionDenied, "symlink");
    assert!(fs::symlink_metadata(&at).unwrap().file_type().is_symlink());
    assert!(
        fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_socket()
    );
    fs::remove_file(&at).unwrap();
    fs::remove_file(&target).unwrap();

    // A private regular file at the name.
    fs::write(&at, b"must survive").unwrap();
    fs::set_permissions(&at, fs::Permissions::from_mode(0o600)).unwrap();
    let meta = fs::symlink_metadata(&at).unwrap();
    describe(meta.dev() ^ 1, meta.ino());
    refused(io::ErrorKind::PermissionDenied, "regular file");
    assert_eq!(fs::read(&at).unwrap(), b"must survive");
    fs::remove_file(&at).unwrap();
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}
