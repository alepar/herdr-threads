//! ht-kqrt.72 (E1): the server-environ reader and the inherited-name check. Included from
//! `src/service/locate.rs` (it needs the crate-private parsers).
use super::*;
use crate::{
    app::SystemClock,
    host::observation::HostTopology,
    ports::{
        AgentComposerState, ComposerObservation, HostObservation, HostSnapshot,
        NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest, PromptOutcome,
        SafeWakeTarget,
    },
    protocol::{
        ids::SeatId,
        results::ApiError,
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    test_support::{
        RefusingHost,
        spawn::{OwnedChild, SpawnOwned, command},
    },
};
use std::sync::atomic::{AtomicU32, Ordering};

const NAME_VAR: &str = crate::host::named::ENV_SEAT_NAME;

/// A refusing host that owns one pane and serves from a settable server pid (0 = no pid).
pub(crate) struct EnvHost {
    pane: &'static str,
    server: AtomicU32,
}

impl EnvHost {
    pub(crate) fn new(pane: &'static str, server: Option<u32>) -> Arc<Self> {
        Arc::new(Self {
            pane,
            server: AtomicU32::new(server.unwrap_or(0)),
        })
    }
    /// The server restarted as `pid`.
    pub(crate) fn restart_as(&self, pid: u32) {
        self.server.store(pid, Ordering::SeqCst);
    }
}

impl HostPort for EnvHost {
    fn target_root_process(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<Option<u32>, ApiError> {
        Ok((target.as_str() == self.pane).then_some(1))
    }
    fn server_process(&self) -> Option<u32> {
        Some(self.server.load(Ordering::SeqCst)).filter(|pid| *pid != 0)
    }
    fn live_topology(&self, budget: &CallBudget) -> Result<HostTopology, ApiError> {
        RefusingHost.live_topology(budget)
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        RefusingHost.observe_current_target(target, context)
    }
    fn observe_current_target_for_archival(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        RefusingHost.observe_current_target_for_archival(target, context)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        RefusingHost.enumerate_targets(context)
    }
    fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        RefusingHost.submit_prompt(target, text, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        RefusingHost.launch_native(request, context)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        RefusingHost.pane_agent_state(target, context)
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        RefusingHost.send_submit_key(target, context)
    }
}

fn budget() -> CallBudget {
    let clock = SystemClock::new();
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
        cancellation: Cancellation::default(),
    }
}

fn context() -> HostCallContext {
    HostCallContext {
        budget: budget(),
        expected_boot: None,
        expected_epoch: None,
    }
}

/// A long-lived owned child whose environment may carry the seat name; returns once `exec` has
/// replaced the forked copy of the parent's environment (the variable is then visible, or, for
/// a child without it, the child's own `/proc` entry is the post-exec one).
pub(crate) fn server_child(name: Option<&str>) -> OwnedChild {
    let mut cmd = command("sleep");
    cmd.arg("60");
    cmd.env_remove(NAME_VAR);
    if let Some(name) = name {
        cmd.env(NAME_VAR, name);
    }
    let child = cmd.spawn_owned().expect("sleep starts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let environ = read_server_environ(child.id());
        let ready = environ.as_ref().is_some_and(|environ| {
            environ
                .iter()
                .any(|(key, _)| key.as_os_str() == "HT_TEST_OWNER")
        });
        if ready {
            return child;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the child's environment never became readable"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

// Kills: a reader that returns `None` for a readable child, drops or mangles entries, or reads
// the parent's environment instead of the child's.
#[cfg(target_os = "linux")]
#[test]
fn linux_reader_reads_known_env() {
    let child = server_child(Some("x"));
    let environ = read_server_environ(child.id()).expect("a same-user child is readable");
    assert!(
        environ
            .iter()
            .any(|(key, value)| key.as_os_str() == NAME_VAR && value.as_os_str() == "x"),
        "the child's seat name is read back: {environ:?}"
    );
    let plain = server_child(None);
    let environ = read_server_environ(plain.id()).expect("readable");
    assert!(
        environ.iter().all(|(key, _)| key.as_os_str() != NAME_VAR),
        "a child started without the variable does not carry it"
    );
}

// Kills: a block parser that splits on the wrong byte, keeps an entry with no `=`, or cuts a
// value at its own `=`.
#[test]
fn environ_block_splits_on_the_first_equals() {
    let parsed = parse_environ_block(b"A=1\0B=x=y\0NOEQ\0C=\0");
    let text: Vec<(&str, &str)> = parsed
        .iter()
        .map(|(k, v)| (k.to_str().unwrap(), v.to_str().unwrap()))
        .collect();
    assert_eq!(text, [("A", "1"), ("B", "x=y"), ("C", "")]);
}

// Kills: a parser that mistakes argv for the environment, stops at the executable path's
// padding, drops an empty value, or reads past the terminating empty string.
#[test]
fn macos_procargs2_parser_on_fixture() {
    let fixture = include_bytes!("../fixtures/procargs2/sample.bin");
    let parsed = parse_procargs2(fixture).expect("the captured buffer parses");
    let text: Vec<(&str, &str)> = parsed
        .iter()
        .map(|(k, v)| (k.to_str().unwrap(), v.to_str().unwrap()))
        .collect();
    assert_eq!(
        text,
        [
            ("HOME", "/Users/x"),
            ("HERDR_THREADS_SEAT_NAME", "main"),
            ("EMPTY", ""),
            ("PATH", "/usr/bin:/bin"),
        ]
    );
    // Malformed buffers are `None`, never a partial environment taken for a whole one.
    assert_eq!(parse_procargs2(&[]), None, "empty");
    assert_eq!(parse_procargs2(&fixture[..3]), None, "short argc");
    assert_eq!(
        parse_procargs2(&fixture[..30]),
        None,
        "cut inside the padding and argv"
    );
    let mut negative = fixture.to_vec();
    negative[..4].copy_from_slice(&(-1_i32).to_ne_bytes());
    assert_eq!(parse_procargs2(&negative), None, "negative argc");
    let mut greedy = fixture.to_vec();
    greedy[..4].copy_from_slice(&200_i32.to_ne_bytes());
    assert_eq!(parse_procargs2(&greedy), None, "argc past the strings");
}

// Kills: a check that fails closed (refuses) when the server environment cannot be read, one
// that refuses on a different value, one that asks a host that does not own the pane, and one
// that ignores a matching name.
#[test]
fn unreadable_env_skips_check() {
    let ctx = context();
    let reads = |env: Option<Environ>| move |_: u32| env.clone();
    let carries = |name: &str| Some(vec![(OsString::from(NAME_VAR), OsString::from(name))]);
    let host: Arc<dyn HostPort> = EnvHost::new("w1:p1", Some(42));
    let hosts = [host];
    assert!(inherited_name(
        "x",
        "w1:p1",
        &hosts,
        &ctx,
        reads(carries("x"))
    ));
    assert!(
        !inherited_name("x", "w1:p1", &hosts, &ctx, reads(carries("y"))),
        "a different name is honoured"
    );
    assert!(
        !inherited_name("x", "w1:p1", &hosts, &ctx, reads(None)),
        "an unreadable environment skips the check"
    );
    assert!(
        !inherited_name("x", "w1:p1", &hosts, &ctx, reads(Some(Vec::new()))),
        "an environment without the variable"
    );
    assert!(
        !inherited_name("x", "w9:p9", &hosts, &ctx, reads(carries("x"))),
        "no host owns the pane"
    );
    let no_pid: Arc<dyn HostPort> = EnvHost::new("w1:p1", None);
    assert!(
        !inherited_name("x", "w1:p1", &[no_pid], &ctx, reads(carries("x"))),
        "a server with no pid skips the check"
    );
    // The real reader on a pid that cannot exist.
    assert_eq!(read_server_environ(u32::MAX - 1), None);
}

// Kills: an M6 text that drifts from the golden bytes or the stated 129-byte fixed part.
#[test]
fn m6_matches_the_golden() {
    assert_eq!(
        format!(
            "herdr-threads: {}\n",
            inherited_name_detail("reviewer", "Herdr")
        ),
        include_str!("../cli/golden/m6.txt")
    );
    assert_eq!(
        format!("herdr-threads: {}", inherited_name_detail("", "")).len(),
        129,
        "the fixed bytes of M6"
    );
}

// Kills: a NativeCli that answers no server pid, or the connecting process's own pid, for the
// process serving its socket.
#[test]
fn native_cli_server_process_is_the_socket_peer() {
    let dir = crate::test_support::scratch::temp_path("environ-native-server");
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("h.sock");
    // The listener lives in this process, so the peer pid is this pid.
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let cli = crate::host::native::NativeCli::new(socket.clone(), Arc::new(SystemClock::new()));
    assert_eq!(cli.server_process(), Some(std::process::id()));
    drop(_listener);
    std::fs::remove_file(&socket).unwrap();
    assert_eq!(cli.server_process(), None, "no listener, no server pid");
    std::fs::remove_dir_all(dir).unwrap();
}
