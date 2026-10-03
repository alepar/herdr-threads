//! ht-p03.10: protocol version skew, both directions, against a fake
//! other-version daemon. The fake is this test binary re-exec'd (so it is a
//! real process holding the real owner lock, socket and descriptor) with a
//! `daemon run --state-dir <state>` command line, which is what the skew-tolerant
//! `daemon stop` identifies an owner by. "Old" is the real release skew pair:
//! a protocol-1 daemon or CLI against this protocol-2 build (protocol 2 is
//! B5's `expected_boot`, ht-rzi.23).
//! Mounted from tests/integration.rs.

use herdr_threads::test_support::spawn::SpawnOwned;
use herdr_threads::{
    app::SystemClock,
    client::local::LocalSocketClient,
    daemon::{
        control::request_stop,
        lifecycle::ensure_running,
        ownership::{OwnerLock, read_descriptor},
        paths::{InstancePaths, RuntimeContext},
        remedy::{RemedyContext, remedy},
    },
    protocol::{
        results::{ErrorClass, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
        wire::{PROTOCOL_VERSION, WireResponse},
    },
};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::DirBuilderExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const FAKE_ENV: &str = "HT_FAKE_OTHER_VERSION_DAEMON";
const FAKE_SOFTWARE: &str = "0.0.1";
/// The protocol of the last release before this build's (B5 moved to 2).
const OLD_PROTOCOL: u16 = 1;

/// Kills: a protocol bump that leaves the skew tests on a synthetic pair.
#[test]
fn skew_tests_use_the_real_release_pair() {
    assert_eq!(PROTOCOL_VERSION, 2);
    assert_eq!(OLD_PROTOCOL, PROTOCOL_VERSION - 1);
}

/// The re-exec entry point. A normal test run returns at once; the helper
/// process (env set) owns the instance until it is signalled.
#[test]
fn fake_other_version_daemon_main() {
    let Ok(spec) = std::env::var(FAKE_ENV) else {
        return;
    };
    let (state, host) = spec.split_once('|').expect("state|host");
    // Never outlive a crashed parent.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        std::process::exit(3);
    });
    let context =
        RuntimeContext::explicit(PathBuf::from(state), PathBuf::from(host), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let lock = OwnerLock::acquire(&paths).unwrap();
    let listener = lock.bind_socket().unwrap();
    lock.publish_endpoint(&listener, FAKE_SOFTWARE, OLD_PROTOCOL)
        .unwrap();
    // An other-version daemon that cannot decode this CLI's hello just closes.
    loop {
        let _ = listener.accept();
    }
}

struct Scratch {
    root: PathBuf,
    state: PathBuf,
    host: PathBuf,
}
impl Scratch {
    fn new() -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/htsk-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let state = root.join("st");
        fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
        let host = root.join("host.sock");
        Self { root, state, host }
    }
    fn context(&self) -> RuntimeContext {
        RuntimeContext::explicit(self.state.clone(), self.host.clone(), None).unwrap()
    }
    fn paths(&self) -> InstancePaths {
        InstancePaths::resolve(&self.context()).unwrap()
    }
    /// This CLI with the scratch context (isolated HOME and harness config).
    fn cli(&self, args: &[&str]) -> Output {
        crate::scrubbed_command(BIN)
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host)
            .args(args)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .output()
            .unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // Stop whatever daemon this scratch ended up with, then clean up.
        let _ = self.cli(&["daemon", "stop"]);
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Fake(herdr_threads::test_support::spawn::OwnedChild);
impl Fake {
    fn start(scratch: &Scratch) -> Self {
        let child = crate::scrubbed_command(std::env::current_exe().unwrap())
            .args([
                "daemon_skew::fake_other_version_daemon_main",
                "--exact",
                "--nocapture",
                "--",
                "daemon",
                "run",
                "--state-dir",
            ])
            .arg(&scratch.state)
            .arg("--host-endpoint")
            .arg(&scratch.host)
            .env(
                FAKE_ENV,
                format!("{}|{}", scratch.state.display(), scratch.host.display()),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn_owned()
            .unwrap();
        let fake = Self(child);
        let paths = scratch.paths();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(Some(instance)) =
                herdr_threads::daemon::ownership::read_existing_namespace(&paths)
                && let Ok(descriptor) = read_descriptor(&paths, instance)
                && descriptor.pid == fake.0.id()
            {
                return fake;
            }
            assert!(Instant::now() < deadline, "fake daemon never published");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn pid(&self) -> u32 {
        self.0.id()
    }
    fn alive(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
    fn wait_exit(&mut self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if !self.alive() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock::new())
}

fn ensure(
    scratch: &Scratch,
) -> Result<
    herdr_threads::daemon::ownership::EndpointDescriptor,
    herdr_threads::protocol::results::ApiError,
> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(ensure_running(&scratch.context(), Path::new(BIN), clock()))
}

fn lock_free(scratch: &Scratch) -> bool {
    OwnerLock::acquire(&scratch.paths()).is_ok()
}

/// Kills: decoding the other version's descriptor or reply before comparing
/// versions (a 5 s timeout or a deny_unknown_fields error would surface).
#[test]
fn new_cli_reports_skew_before_decode() {
    let scratch = Scratch::new();
    let mut fake = Fake::start(&scratch);
    let started = Instant::now();
    let error = ensure(&scratch).unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(error.code, ErrorCode::UnknownWireVersion);
    let expected = remedy(
        Some(ErrorClass::VersionSkew),
        &RemedyContext::VersionSkew {
            daemon: format!("{FAKE_SOFTWARE} (protocol {OLD_PROTOCOL})"),
            cli: format!(
                "{} (protocol {PROTOCOL_VERSION})",
                env!("CARGO_PKG_VERSION")
            ),
        },
    );
    assert_eq!(error.detail, expected);
    for banned in ["unknown field", "deny_unknown", "timed out", "decode"] {
        assert!(!error.detail.contains(banned), "{}", error.detail);
    }
    // The CLI's other commands report the same line, not a decode failure.
    let health = scratch.cli(&["daemon", "health"]);
    let stderr = text(&health.stderr);
    assert_eq!(health.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains(&expected), "{stderr}");
    assert!(!stderr.contains("unknown field"), "{stderr}");
    let doctor = scratch.cli(&["doctor"]);
    assert!(
        text(&doctor.stdout).contains(&expected),
        "{}",
        text(&doctor.stdout)
    );
    assert!(fake.alive(), "reporting skew must not touch the daemon");
}

/// Kills: closing the socket on a foreign-version request (the old decoder
/// would then wait out its deadline) instead of answering decodably.
#[test]
fn old_cli_gets_a_decodable_skew_error() {
    let scratch = Scratch::new();
    let descriptor = ensure(&scratch).expect("this version starts");
    assert_eq!(descriptor.protocol_version, PROTOCOL_VERSION);
    // A request in the protocol-1 wire shape: no `output`, no `expected_boot`.
    let body = format!(
        r#"{{"version":1,"request_id":"old-cli-1","expected_instance":"{}","command":{{"kind":"health"}}}}"#,
        descriptor.instance_uuid
    );
    let mut stream = UnixStream::connect(&descriptor.endpoint).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(body.as_bytes()).unwrap();
    let mut prefix = [0_u8; 4];
    stream
        .read_exact(&mut prefix)
        .expect("a reply, not a close");
    let mut reply = vec![0_u8; u32::from_be_bytes(prefix) as usize];
    stream.read_exact(&mut reply).unwrap();
    // The reply decodes with the response type an older CLI uses.
    let response: WireResponse = serde_json::from_slice(&reply).expect("decodable");
    assert_eq!(
        response.version, OLD_PROTOCOL,
        "echoes the sender's version"
    );
    assert_eq!(response.request_id, "old-cli-1");
    assert_eq!(response.instance, descriptor.instance_uuid.to_string());
    let error = response.result.unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownWireVersion);
    assert!(error.detail.contains("daemon stop"), "{}", error.detail);
}

/// Pulls the backticked commands out of a remedy line, in order.
fn commands(remedy_text: &str) -> Vec<Vec<String>> {
    remedy_text
        .split('`')
        .skip(1)
        .step_by(2)
        .map(|command| {
            command
                .strip_prefix("herdr-threads ")
                .expect("remedy commands start with the CLI name")
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

/// Kills: a skew remedy that cannot run from this CLI (stop refusing on the
/// protocol mismatch), a stop that leaves the lock held, or an ensure that
/// does not then start this version.
#[test]
fn remedy_round_trip_stops_the_old_daemon() {
    let scratch = Scratch::new();
    let mut fake = Fake::start(&scratch);
    let remedy_line = ensure(&scratch).unwrap_err().detail;
    println!("VersionSkew remedy: {remedy_line}");
    let steps = commands(&remedy_line);
    assert_eq!(steps.len(), 2, "{remedy_line}");
    assert_eq!(steps[0], ["daemon", "stop"]);
    assert_eq!(steps[1], ["daemon", "ensure"]);
    assert!(!lock_free(&scratch), "the old daemon holds the owner lock");

    let stop = scratch.cli(&steps[0].iter().map(String::as_str).collect::<Vec<_>>());
    assert!(
        stop.status.success(),
        "{}{}",
        text(&stop.stdout),
        text(&stop.stderr)
    );
    assert!(
        fake.wait_exit(Duration::from_secs(10)),
        "the old daemon exited"
    );
    assert!(lock_free(&scratch), "the owner lock was released");

    let ensured = scratch.cli(&["--json", "daemon", "ensure"]);
    assert!(ensured.status.success(), "{}", text(&ensured.stderr));
    let health: Value = serde_json::from_slice(&ensured.stdout).unwrap();
    assert_eq!(
        health["result"]["data"]["protocol_version"],
        PROTOCOL_VERSION
    );
    assert_eq!(
        health["result"]["data"]["software_version"],
        env!("CARGO_PKG_VERSION")
    );
}

/// Kills: signalling a pid on the descriptor's say-so alone. Two refusals,
/// each with the old daemon still alive afterwards: the descriptor this call
/// holds names a boot that is not the published one, and a published
/// descriptor whose pid is an unrelated live process (a reused pid).
#[test]
fn stop_refuses_a_pid_whose_boot_id_does_not_match() {
    let scratch = Scratch::new();
    let mut fake = Fake::start(&scratch);
    let paths = scratch.paths();
    let instance = herdr_threads::daemon::ownership::read_existing_namespace(&paths)
        .unwrap()
        .unwrap();
    let published = read_descriptor(&paths, instance).unwrap();
    let client = LocalSocketClient::new(
        published.endpoint.clone(),
        clock(),
        instance,
        Some(published.boot_id),
    );
    let c = clock();
    let budget = CallBudget {
        deadline: MonoInstant(c.monotonic_now().0 + 5_000),
        cancellation: Cancellation::default(),
    };

    // (a) A descriptor from a previous boot (other boot_id and socket inode).
    let previous_boot = herdr_threads::daemon::ownership::EndpointDescriptor {
        boot_id: uuid::Uuid::new_v4(),
        socket_inode: published.socket_inode + 1,
        ..published.clone()
    };
    let error = request_stop(&client, &paths, &previous_boot, &budget).unwrap_err();
    assert!(
        error.detail.contains("refusing to signal"),
        "{}",
        error.detail
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(fake.alive(), "no signal was sent");

    // (b) The published pid is some other live process.
    let mut bystander = crate::scrubbed_command("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .spawn_owned()
        .unwrap();
    let mut planted: Value =
        serde_json::from_slice(&fs::read(&paths.descriptor_path).unwrap()).unwrap();
    planted["pid"] = Value::from(bystander.id());
    fs::write(
        &paths.descriptor_path,
        serde_json::to_vec(&planted).unwrap(),
    )
    .unwrap();
    let stop = scratch.cli(&["daemon", "stop"]);
    let stderr = text(&stop.stderr);
    assert!(!stop.status.success(), "{stderr}");
    assert!(stderr.contains("refusing to signal"), "{stderr}");
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "the bystander was not signalled"
    );
    assert!(fake.alive(), "the old daemon was not signalled");
    assert_ne!(fake.pid(), bystander.id());
    let _ = bystander.kill();
    let _ = bystander.wait();
}
