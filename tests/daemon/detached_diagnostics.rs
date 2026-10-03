use super::*;
use crate::daemon::ownership::OwnerLock;
use crate::daemon::paths::{InstancePaths, RuntimeContext};
use crate::ports::LocalService;
use crate::protocol::{
    authority::PeerIdentity,
    commands::Command,
    results::{ApiError, CapabilityState, CommandResult, ComponentState, Health, HealthState},
    time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
};
use crate::test_support::spawn::SpawnOwned;
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

/// A spawned process with every inherited HERDR_/CLAUDE/CODEX variable removed
/// (ht-p03.24); a test sets the variables it needs after this call.
fn scrubbed_process(program: impl AsRef<std::ffi::OsStr>) -> ProcessCommand {
    let mut command = ProcessCommand::new(program);
    crate::test_support::isolation::scrub_env(&mut command);
    command
}

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

struct InertService;
impl LocalService for InertService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        unreachable!("fixture stops before admitting a request")
    }
}

struct HealthService(String, String);
impl LocalService for HealthService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        command: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        assert!(matches!(command, Command::Health));
        let mut health = Health::unknown(
            self.0.clone(),
            self.1.clone(),
            env!("CARGO_PKG_VERSION").into(),
            crate::protocol::wire::PROTOCOL_VERSION,
        );
        health.database.state = ComponentState::Ready;
        health.schema.state = ComponentState::Ready;
        health.host.current_execution = CapabilityState::Supported;
        health.host.coherent_enumeration = CapabilityState::Supported;
        health.state = HealthState::Healthy;
        Ok(CommandResult::Health(health))
    }
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < timeout,
            "timed out waiting for detached fixture"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fixture_paths(root: PathBuf) -> InstancePaths {
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    InstancePaths::resolve(&context).unwrap()
}

struct ContinuousFixtureGuard {
    root: PathBuf,
    owner: crate::test_support::spawn::OwnedChild,
}

impl Drop for ContinuousFixtureGuard {
    fn drop(&mut self) {
        let _ = fs::write(self.root.join("stop-owner"), b"");
        let _ = fs::write(self.root.join("stop-writer"), b"");
        if self.owner.try_wait().ok().flatten().is_none() {
            let _ = self.owner.kill();
            let _ = self.owner.wait();
        }
        let started = Instant::now();
        while self.root.join("writer-started").exists()
            && !self.root.join("writer-ended").exists()
            && started.elapsed() < Duration::from_secs(2)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Run only in its own process: the production sink owns process stdout/stderr.
#[test]
#[ignore]
fn elected_output_fixture() {
    let root = PathBuf::from(std::env::var_os("HERDR_TASK23_ROOT").unwrap());
    let paths = fixture_paths(root);
    let shutdown = Cancellation::default();
    let ready_shutdown = shutdown.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(run_elected_with_diagnostics(
            &paths,
            Arc::new(TestClock),
            shutdown,
            |_, _, _| {
                if std::env::var_os("HERDR_TASK23_FACTORY_ERROR").is_some() {
                    Err(std::io::Error::other("factory-startup-sentinel"))
                } else {
                    Ok(Arc::new(InertService))
                }
            },
            move |_| {
                if std::env::var_os("HERDR_TASK23_STARTUP_ERROR").is_some() {
                    return Err(std::io::Error::other("startup-retained-sentinel"));
                }
                let mut input = [0; 1];
                assert_eq!(std::io::stdin().read(&mut input).unwrap(), 0);
                std::io::stdout().write_all(&vec![b'o'; 2_300_000]).unwrap();
                std::io::stdout().write_all(b"stdout-survived\n").unwrap();
                std::io::stderr().write_all(&vec![b'e'; 900_000]).unwrap();
                std::io::stderr().write_all(b"stderr-survived\n").unwrap();
                ready_shutdown.cancel();
                Ok(())
            },
            || async {
                if std::env::var_os("HERDR_TASK23_DRAIN_ERROR").is_some() {
                    Err(std::io::Error::other("post-rotation-owner-sentinel"))
                } else {
                    Ok(())
                }
            },
        ))
        .unwrap();
}

#[test]
fn elected_child_output_rotates_without_retaining_a_terminal() {
    let root = std::env::temp_dir().join(format!("herdr-task23-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let started = Instant::now();
    let status = scrubbed_process(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::elected_output_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "elected owner fixture exited {status}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "oversized output drain stalled"
    );
    let logs = fixture_paths(root.clone()).instance_dir;
    let mut names = fs::read_dir(&logs)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("daemon.log"))
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["daemon.log", "daemon.log.1"]);
    let bytes = names
        .iter()
        .flat_map(|name| {
            let content = fs::read(logs.join(name)).unwrap();
            assert!(content.len() <= 1_048_576);
            content
        })
        .collect::<Vec<_>>();
    assert!(
        bytes
            .windows(b"stdout-survived".len())
            .any(|w| w == b"stdout-survived")
    );
    assert!(
        bytes
            .windows(b"stderr-survived".len())
            .any(|w| w == b"stderr-survived")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_error_is_retained_in_elected_child_log() {
    let root = std::env::temp_dir().join(format!("herdr-task23-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let status = scrubbed_process(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::elected_output_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .env("HERDR_TASK23_STARTUP_ERROR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    let content = fs::read(fixture_paths(root.clone()).instance_dir.join("daemon.log")).unwrap();
    assert!(
        content
            .windows(b"daemon owner error: startup-retained-sentinel".len())
            .any(|w| w == b"daemon owner error: startup-retained-sentinel")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepublication_factory_error_is_logged_without_a_ready_endpoint() {
    let root = std::env::temp_dir().join(format!("herdr-task23-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let status = scrubbed_process(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::elected_output_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .env("HERDR_TASK23_FACTORY_ERROR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    let paths = fixture_paths(root.clone());
    assert!(!paths.descriptor_path.exists());
    assert!(
        !fs::read_dir(&paths.instance_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .any(|name| name.to_string_lossy().ends_with(".sock"))
    );
    let content = fs::read(paths.instance_dir.join("daemon.log")).unwrap();
    assert!(
        content
            .windows(b"daemon owner error: factory-startup-sentinel".len())
            .any(|w| w == b"daemon owner error: factory-startup-sentinel")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn owner_error_after_oversized_output_survives_final_drain() {
    let root = std::env::temp_dir().join(format!("herdr-task23-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let status = scrubbed_process(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::elected_output_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .env("HERDR_TASK23_DRAIN_ERROR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    let logs = fixture_paths(root.clone()).instance_dir;
    let active = fs::read(logs.join("daemon.log")).unwrap();
    let previous = fs::read(logs.join("daemon.log.1")).unwrap();
    assert!(active.len() <= 1_048_576 && previous.len() <= 1_048_576);
    assert!(
        active
            .windows(b"daemon owner error: post-rotation-owner-sentinel".len())
            .any(|w| w == b"daemon owner error: post-rotation-owner-sentinel")
    );
    fs::remove_dir_all(root).unwrap();
}

/// The descendant retains the redirected writer after the elected owner
/// restores its descriptors. It stops on read-side close or its fixture flag.
#[test]
#[ignore]
fn continuous_writer_fixture() {
    let root = PathBuf::from(std::env::var_os("HERDR_TASK23_ROOT").unwrap());
    fs::write(root.join("writer-started"), b"").unwrap();
    let chunk = [b'w'; 4096];
    while !root.join("stop-writer").exists() {
        if std::io::stdout().write_all(&chunk).is_err() {
            break;
        }
        if std::env::var_os("HERDR_TASK23_SLOW_WRITER").is_some() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fs::write(root.join("writer-ended"), b"").unwrap();
}

#[test]
#[ignore]
fn continuous_owner_fixture() {
    let root = PathBuf::from(std::env::var_os("HERDR_TASK23_ROOT").unwrap());
    let paths = fixture_paths(root.clone());
    let shutdown = Cancellation::default();
    let ready_shutdown = shutdown.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(run_elected_with_diagnostics(
        &paths,
        Arc::new(TestClock),
        shutdown,
        |_, _, _| Ok(Arc::new(InertService)),
        move |_| {
            let mut writer = scrubbed_process(std::env::current_exe().unwrap());
            writer
                .arg("--ignored")
                .arg("--exact")
                .arg("daemon::tests::continuous_writer_fixture")
                .env("HERDR_TASK23_ROOT", &root)
                .stdin(Stdio::null());
            if std::env::var_os("HERDR_TASK23_SLOW_WRITER").is_some() {
                writer.env("HERDR_TASK23_SLOW_WRITER", "1");
            }
            // The writer is deliberately left running (and unreaped) past this
            // owner process: the fixture tests output from a detached writer.
            // Tagged so the reaper still stops it once this fixture process is gone.
            crate::test_support::spawn::tag(&mut writer);
            #[allow(clippy::zombie_processes)]
            writer.spawn().unwrap(); // leak-guard: deliberately outlives its owner (detached writer fixture); tagged above
            wait_until(Duration::from_secs(2), || {
                root.join("writer-started").exists()
            });
            wait_until(Duration::from_secs(5), || root.join("stop-owner").exists());
            ready_shutdown.cancel();
            Ok(())
        },
        || async { Err(std::io::Error::other("continuous-final-error-sentinel")) },
    ));
    assert!(result.is_err());
}

#[test]
#[ignore]
fn reacquired_owner_fixture() {
    let root = PathBuf::from(std::env::var_os("HERDR_TASK23_ROOT").unwrap());
    let paths = fixture_paths(root);
    let shutdown = Cancellation::default();
    let ready_shutdown = shutdown.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let owned = runtime
        .block_on(run_elected_with_diagnostics(
            &paths,
            Arc::new(TestClock),
            shutdown,
            |_, _, _| Ok(Arc::new(InertService)),
            move |_| {
                ready_shutdown.cancel();
                Ok(())
            },
            || async { Ok(()) },
        ))
        .unwrap();
    assert!(
        owned,
        "owner lease was not reacquired after reader completion"
    );
}

#[test]
fn continuous_descendant_cannot_hold_owner_drain_or_erase_final_error() {
    run_continuous_case(false, false);
}

#[test]
fn slow_descendant_hits_monotonic_drain_deadline() {
    run_continuous_case(true, false);
}

#[test]
fn contender_is_excluded_at_real_reader_drain_entry() {
    run_continuous_case(true, true);
}

fn run_continuous_case(slow: bool, probe_drain: bool) {
    let root = std::env::temp_dir().join(format!("herdr-task23-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut owner_command = scrubbed_process(&executable);
    owner_command
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::continuous_owner_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if slow {
        owner_command.env("HERDR_TASK23_SLOW_WRITER", "1");
    }
    if probe_drain {
        owner_command.env("HERDR_TASK23_DRAIN_PROBE_DIR", &root);
    }
    let mut guard = ContinuousFixtureGuard {
        root: root.clone(),
        owner: owner_command.spawn_owned().unwrap(),
    };
    wait_until(Duration::from_secs(5), || {
        root.join("writer-started").exists()
    });
    let loser = scrubbed_process(&executable)
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::losing_owner_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(loser.success(), "second owner entered before drain");
    fs::write(root.join("stop-owner"), b"").unwrap();
    if probe_drain {
        wait_until(Duration::from_secs(2), || {
            root.join("drain-entered").exists()
        });
        assert!(
            !root.join("reader-completed").exists(),
            "reader completed before lease probe"
        );
        let paths = fixture_paths(root.clone());
        let attempt = OwnerLock::acquire(&paths);
        assert!(
            matches!(&attempt, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "contender acquired owner lease during reader drain: {attempt:?}"
        );
        assert!(
            !root.join("reader-completed").exists(),
            "reader completed during lease probe"
        );
        fs::write(root.join("release-reader"), b"").unwrap();
    }
    let started = Instant::now();
    let status = loop {
        if let Some(status) = guard.owner.try_wait().unwrap() {
            break Some(status);
        }
        if started.elapsed() > Duration::from_secs(3) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // The watchdog releases only this fixture's descendant if old code hangs.
    let writer_ended_before_stop = {
        let wait_start = Instant::now();
        loop {
            if root.join("writer-ended").exists() {
                break true;
            }
            if wait_start.elapsed() > Duration::from_secs(1) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    fs::write(root.join("stop-writer"), b"").unwrap();
    if status.is_none() {
        let _ = guard.owner.kill();
        let _ = guard.owner.wait();
    }
    wait_until(Duration::from_secs(3), || {
        root.join("writer-ended").exists()
    });
    assert!(
        status.is_some(),
        "continuous writer held owner drain past watchdog"
    );
    assert!(
        writer_ended_before_stop,
        "raw reader did not close its owned read side"
    );
    assert!(
        status.unwrap().success(),
        "owner fixture failed unexpectedly"
    );
    if probe_drain {
        assert!(
            root.join("reader-resumed").exists(),
            "real reader did not pass checkpoint"
        );
        assert!(
            root.join("reader-completed").exists(),
            "owner exited before reader completion"
        );
        assert!(
            !root.join("probe-timeout").exists(),
            "reader checkpoint exceeded live deadline"
        );
    }
    let paths = fixture_paths(root.clone());
    assert!(!paths.descriptor_path.exists());
    let active = fs::read(paths.instance_dir.join("daemon.log")).unwrap();
    let previous = fs::read(paths.instance_dir.join("daemon.log.1")).unwrap_or_default();
    assert!(active.len() <= 1_048_576 && previous.len() <= 1_048_576);
    assert!(
        active
            .windows(b"daemon owner error: continuous-final-error-sentinel".len())
            .any(|window| window == b"daemon owner error: continuous-final-error-sentinel")
    );
    let retained = [previous, active.clone()].concat();
    let cutoff = if slow {
        b"diagnostic raw-output drain cutoff (deadline)".as_slice()
    } else {
        b"diagnostic raw-output drain cutoff".as_slice()
    };
    assert!(
        retained
            .windows(cutoff.len())
            .any(|window| window == cutoff)
    );
    let reacquired = scrubbed_process(&executable)
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::reacquired_owner_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        reacquired.success(),
        "lease unavailable after reader joined"
    );
}

/// The launcher gives ensure_running its actual daemon-run argv while
/// executing the test fixture in a dedicated detached child.
#[test]
#[ignore]
fn detached_owner_fixture() {
    let root = PathBuf::from(std::env::var_os("HERDR_TASK23_ROOT").unwrap());
    let paths = fixture_paths(root.clone());
    let shutdown = Cancellation::default();
    let monitor_shutdown = shutdown.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::spawn(async move {
            let started = Instant::now();
            loop {
                if root.join("go").exists() {
                    break;
                }
                if started.elapsed() > Duration::from_secs(10) {
                    monitor_shutdown.cancel();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            std::io::stdout().write_all(&vec![b'o'; 2_300_000]).unwrap();
            std::io::stdout()
                .write_all(b"stdout-after-ensure-exit\n")
                .unwrap();
            std::io::stderr().write_all(&vec![b'e'; 900_000]).unwrap();
            std::io::stderr()
                .write_all(b"stderr-after-ensure-exit\n")
                .unwrap();
            loop {
                if root.join("stop").exists() || started.elapsed() > Duration::from_secs(12) {
                    monitor_shutdown.cancel();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        run_elected_with_diagnostics(
            &paths,
            Arc::new(TestClock),
            shutdown,
            |instance, boot, _| {
                Ok(Arc::new(HealthService(
                    instance.to_string(),
                    boot.to_string(),
                )))
            },
            move |_| {
                let mut input = [0; 1];
                assert_eq!(std::io::stdin().read(&mut input).unwrap(), 0);
                std::io::stderr().write_all(b"owner-startup\n").unwrap();
                Ok(())
            },
            || async { Ok(()) },
        )
        .await
        .unwrap();
    });
}

#[test]
#[ignore]
fn ensure_parent_fixture() {
    let root = PathBuf::from(std::env::var_os("HERDR_TASK23_ROOT").unwrap());
    let launcher = PathBuf::from(std::env::var_os("HERDR_TASK23_LAUNCHER").unwrap());
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(crate::daemon::lifecycle::ensure_running(
            &context,
            &launcher,
            Arc::new(TestClock),
        ))
        .unwrap();
}

#[test]
#[ignore]
fn losing_owner_fixture() {
    let root = PathBuf::from(std::env::var_os("HERDR_TASK23_ROOT").unwrap());
    let paths = fixture_paths(root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let owned = runtime
        .block_on(run_elected_with_diagnostics(
            &paths,
            Arc::new(TestClock),
            Cancellation::default(),
            |_, _, _| -> std::io::Result<Arc<dyn LocalService>> {
                panic!("losing owner constructed a service")
            },
            |_| Ok(()),
            || async { Ok(()) },
        ))
        .unwrap();
    assert!(!owned);
}

#[test]
fn detached_output_survives_ensure_parent_and_loser_does_not_touch_logs() {
    let root = std::env::temp_dir().join(format!("herdr-task23-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let launcher = root.join("launcher.sh");
    let current = std::env::current_exe().unwrap();
    let script = format!(
        "#!/bin/sh\nexec '{}' --ignored --exact daemon::tests::detached_owner_fixture\n",
        current.display()
    );
    fs::write(&launcher, script).unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    // The ensure callers have a readable stdin. The detached owner must
    // actively replace it with EOF rather than inherit this descriptor.
    let inherited_input = fs::File::open("/dev/zero").unwrap();
    let mut ensure_one = scrubbed_process(&current)
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::ensure_parent_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .env("HERDR_TASK23_LAUNCHER", &launcher)
        .stdin(Stdio::from(inherited_input.try_clone().unwrap()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn_owned()
        .unwrap();
    let mut ensure_two = scrubbed_process(&current)
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::ensure_parent_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .env("HERDR_TASK23_LAUNCHER", &launcher)
        .stdin(Stdio::from(inherited_input))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn_owned()
        .unwrap();
    let first = ensure_one.wait().unwrap();
    let second = ensure_two.wait().unwrap();
    assert!(
        first.success() && second.success(),
        "ensure callers exited {first} and {second}"
    );
    let paths = fixture_paths(root.clone());
    wait_until(Duration::from_secs(2), || paths.descriptor_path.exists());
    wait_until(Duration::from_secs(2), || {
        fs::read(paths.instance_dir.join("daemon.log"))
            .ok()
            .is_some_and(|bytes| {
                bytes
                    .windows(b"owner-startup".len())
                    .any(|window| window == b"owner-startup")
            })
    });
    let before = fs::read(paths.instance_dir.join("daemon.log")).unwrap();
    assert_eq!(
        before
            .windows(b"owner-startup".len())
            .filter(|window| *window == b"owner-startup")
            .count(),
        1
    );
    let loser = scrubbed_process(&current)
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::tests::losing_owner_fixture")
        .env("HERDR_TASK23_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(loser.success(), "loser exited {loser}");
    assert_eq!(
        fs::read(paths.instance_dir.join("daemon.log")).unwrap(),
        before
    );
    assert!(!paths.instance_dir.join("daemon.log.1").exists());
    fs::write(root.join("go"), b"").unwrap();
    wait_until(Duration::from_secs(5), || {
        fs::read(paths.instance_dir.join("daemon.log"))
            .ok()
            .is_some_and(|bytes| {
                bytes
                    .windows(b"stderr-after-ensure-exit".len())
                    .any(|w| w == b"stderr-after-ensure-exit")
            })
    });
    let active = fs::read(paths.instance_dir.join("daemon.log")).unwrap();
    let previous = fs::read(paths.instance_dir.join("daemon.log.1")).unwrap();
    assert!(active.len() <= 1_048_576 && previous.len() <= 1_048_576);
    let retained = [previous, active].concat();
    assert!(
        retained
            .windows(b"stdout-after-ensure-exit".len())
            .any(|w| w == b"stdout-after-ensure-exit")
    );
    assert!(
        retained
            .windows(b"stderr-after-ensure-exit".len())
            .any(|w| w == b"stderr-after-ensure-exit")
    );
    fs::write(root.join("stop"), b"").unwrap();
    wait_until(Duration::from_secs(5), || !paths.descriptor_path.exists());
    fs::remove_dir_all(root).unwrap();
}
