//! ht-kqrt.11: `LocateCaller` walks the subject's ancestry against each candidate's root.
use super::*;
use crate::test_support::RefusingHost;
use crate::{
    app::SystemClock,
    host::observation::HostTopology,
    ports::{
        AgentComposerState, ComposerObservation, HostCallContext, HostObservation, HostPort,
        HostSnapshot, NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest,
        ProcessRecord, PromptOutcome, SafeWakeTarget,
    },
    protocol::{
        authority::LivenessFingerprint,
        ids::{HostTargetId, SeatId},
        results::ApiError,
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A fake process table: `(pid, parent)` pairs.
pub(crate) struct Tree(Vec<(u32, Option<u32>)>);

impl ProcessInfo for Tree {
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        self.0
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(pid, parent)| ProcessRecord {
                pid: *pid,
                parent: *parent,
                start: 1,
                argv: Vec::new(),
            })
    }
}

/// A host port answering `target_root_process` from a table, counting its questions.
pub(crate) struct Roots {
    answers: Vec<(&'static str, Result<Option<u32>, ()>)>,
    asked: AtomicUsize,
    /// Answers terminal ids (a herdr-core host) rather than pane ids.
    terminals: bool,
}

impl Roots {
    pub(crate) fn port(answers: Vec<(&'static str, Result<Option<u32>, ()>)>) -> Arc<dyn HostPort> {
        Arc::new(Self {
            answers,
            asked: AtomicUsize::new(0),
            terminals: false,
        })
    }

    /// A herdr-core host answering terminal ids.
    pub(crate) fn hc_port(
        answers: Vec<(&'static str, Result<Option<u32>, ()>)>,
    ) -> Arc<dyn HostPort> {
        Arc::new(Self {
            answers,
            asked: AtomicUsize::new(0),
            terminals: true,
        })
    }
}

impl HostPort for Roots {
    fn answers_terminal_ids(&self) -> bool {
        self.terminals
    }
    fn target_root_process(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<Option<u32>, ApiError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.answers
            .iter()
            .find(|(id, _)| *id == target.as_str())
            .map_or(Ok(None), |(_, answer)| {
                answer.map_err(|()| ApiError::host_unavailable("fake host is down"))
            })
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

/// No core can be had for any socket.
fn no_core(_: &str) -> Result<Arc<dyn HostPort>, ApiError> {
    Err(ApiError::unsupported("no core"))
}

/// A `CorePort` answering `port` for every socket.
fn core_of(port: Arc<dyn HostPort>) -> impl Fn(&str) -> Result<Arc<dyn HostPort>, ApiError> {
    move |_| Ok(Arc::clone(&port))
}

fn pane(id: &str) -> LocateCandidate {
    LocateCandidate::Pane { id: id.into() }
}

fn terminal() -> LocateCandidate {
    LocateCandidate::Terminal {
        socket: "/hc.sock".into(),
        id: "t1".into(),
    }
}

fn request(candidates: Vec<LocateCandidate>, subject: Option<u32>) -> LocateCaller {
    LocateCaller {
        candidates,
        subject: subject.map(|pid| LivenessFingerprint { pid, start: 1 }),
    }
}

/// shell 10 -> harness 20 -> hook 30; an unrelated tree 99 -> 98.
pub(crate) fn tree() -> Arc<dyn ProcessInfo> {
    Arc::new(Tree(vec![
        (30, Some(20)),
        (20, Some(10)),
        (10, Some(1)),
        (1, None),
        (99, Some(98)),
        (98, None),
    ]))
}

// Kills: a walk that never reaches the pane's shell (Unverified for a real descendant) or one
// that verifies the wrong candidate index.
#[test]
fn herdr_candidate_verifies_nearest_ancestor() {
    let service = LocateCallerService::new(tree());
    let host = Roots::port(vec![("w1:p1", Ok(Some(10)))]);
    let reply = service.locate(
        &request(vec![terminal(), pane("w1:p1")], Some(30)),
        None,
        &[host],
        &no_core,
        &budget(),
    );
    assert_eq!(reply, LocateReply::Verified { index: 1, root: 10 });
}

// Kills: the transport peer pid being ignored when the claim carries no liveness pid.
#[test]
fn peer_pid_is_the_subject_without_a_fingerprint() {
    let service = LocateCallerService::new(tree());
    let host = Roots::port(vec![("w1:p1", Ok(Some(10)))]);
    let reply = service.locate(
        &request(vec![pane("w1:p1")], None),
        Some(30),
        &[host],
        &no_core,
        &budget(),
    );
    assert_eq!(reply, LocateReply::Verified { index: 0, root: 10 });
}

// Kills: falling back to the pane candidate when its root is not an ancestor, or when the
// terminal family cannot be checked at all.
#[test]
fn no_candidate_verifies_is_unverified() {
    let service = LocateCallerService::new(tree());
    let host = Roots::port(vec![("w1:p1", Ok(Some(10)))]);
    let reply = service.locate(
        &request(vec![pane("w1:p1"), terminal()], Some(99)),
        None,
        &[host],
        &no_core,
        &budget(),
    );
    assert_eq!(
        reply,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::NotAncestor, CandidateVerdict::Unsupported],
        }
    );
    // No subject at all verifies nothing either.
    let none = service.locate(
        &request(vec![pane("w1:p1")], None),
        None,
        &[],
        &no_core,
        &budget(),
    );
    assert!(matches!(none, LocateReply::Unverified { .. }));
}

// Kills: an unreachable host answering Verified or panicking, and an unreachable verdict
// outliving the outage (the next call must ask again once the host is back).
#[test]
fn unreachable_host_counts_unverified() {
    let service = LocateCallerService::new(tree());
    let down = Roots::port(vec![("w1:p1", Err(()))]);
    let reply = service.locate(
        &request(vec![pane("w1:p1")], Some(30)),
        None,
        std::slice::from_ref(&down),
        &no_core,
        &budget(),
    );
    assert_eq!(
        reply,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::Unreachable],
        }
    );
    let up = Roots::port(vec![("w1:p1", Ok(Some(10)))]);
    let again = service.locate(
        &request(vec![pane("w1:p1")], Some(30)),
        None,
        &[up],
        &no_core,
        &budget(),
    );
    assert_eq!(again, LocateReply::Verified { index: 0, root: 10 });
    // No hosts at all is the same refusal.
    let empty = service.locate(
        &request(vec![pane("w1:p1")], Some(98)),
        None,
        &[],
        &no_core,
        &budget(),
    );
    assert_eq!(
        empty,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::Unreachable],
        }
    );
}

// The real Herdr adapter against the fake server: `pane.process_info` names shell pid 1.
#[cfg(feature = "test-support")]
#[test]
fn native_cli_candidate_verifies_against_fake_herdr() {
    let dir = crate::test_support::scratch::temp_path("locate-fake-herdr");
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("h.sock");
    let _herdr = crate::test_support::fake_herdr::FakeHerdr::start("loc", &socket, &["w1:p1"]);
    let cli: Arc<dyn HostPort> = Arc::new(crate::host::native::NativeCli::new(
        socket,
        Arc::new(SystemClock::new()),
    ));
    let service = LocateCallerService::new(tree());
    let inside = service.locate(
        &request(vec![terminal(), pane("w1:p1")], Some(30)),
        None,
        std::slice::from_ref(&cli),
        &no_core,
        &budget(),
    );
    assert_eq!(inside, LocateReply::Verified { index: 1, root: 1 });
    let outside = service.locate(
        &request(vec![terminal(), pane("w1:p1")], Some(99)),
        None,
        &[cli],
        &no_core,
        &budget(),
    );
    assert!(matches!(outside, LocateReply::Unverified { .. }));
    std::fs::remove_dir_all(dir).unwrap();
}

/// A fake process table with start stamps and argv, replaceable between calls.
struct Table(Mutex<Vec<ProcessRecord>>);

impl Table {
    fn new(rows: &[(u32, Option<u32>, u64, &str)]) -> Arc<Self> {
        let table = Arc::new(Self(Mutex::default()));
        table.set(rows);
        table
    }
    fn set(&self, rows: &[(u32, Option<u32>, u64, &str)]) {
        *self.0.lock().unwrap() = rows
            .iter()
            .map(|(pid, parent, start, argv0)| ProcessRecord {
                pid: *pid,
                parent: *parent,
                start: *start,
                argv: if argv0.is_empty() {
                    Vec::new()
                } else {
                    vec![(*argv0).to_owned(), "x".to_owned()]
                },
            })
            .collect();
    }
}

impl ProcessInfo for Table {
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.pid == pid)
            .cloned()
    }
}

fn herdr_pane_at(shell: u32) -> Arc<dyn HostPort> {
    Roots::port(vec![("w1:p1", Ok(Some(shell)))])
}

fn herdr_verdict(table: Arc<Table>, subject: u32, shell: u32) -> LocateReply {
    LocateCallerService::new(table).locate(
        &request(vec![pane("w1:p1")], Some(subject)),
        None,
        &[herdr_pane_at(shell)],
        &no_core,
        &budget(),
    )
}

fn walled() -> LocateReply {
    LocateReply::Unverified {
        per_candidate: vec![CandidateVerdict::NotAncestor],
    }
}

// Kills: a walk that verifies the pane shell above a living `hc` client and its core.
#[test]
fn living_hc_client_is_a_wall() {
    let table = Table::new(&[
        (50, Some(40), 5, "/usr/bin/claude"),
        (40, Some(30), 5, "herdr-core"),
        (30, Some(20), 5, "/home/u/.cargo/bin/hc"),
        (20, Some(10), 5, "-bash"),
        (10, None, 5, "herdr"),
    ]);
    assert_eq!(herdr_verdict(table, 50, 20), walled());
}

// Kills: a wall that is only the `hc` client name (a core reparented to init has no client).
#[test]
fn reparented_core_is_a_wall() {
    let table = Table::new(&[
        (50, Some(40), 5, "claude"),
        (40, Some(1), 5, "/opt/herdr-core"),
        (1, None, 5, "init"),
    ]);
    // Even a root that is init itself, above the core, does not verify.
    assert_eq!(herdr_verdict(table, 50, 1), walled());
}

// Kills: a wall that rejects ordinary trees, or one that fires on unreadable argv.
#[test]
fn plain_pane_tree_verifies() {
    let plain = Table::new(&[
        (30, Some(20), 5, "claude"),
        (20, Some(10), 5, "node"),
        (10, Some(1), 5, "-bash"),
        (1, None, 5, "init"),
    ]);
    assert_eq!(
        herdr_verdict(plain, 30, 10),
        LocateReply::Verified { index: 0, root: 10 }
    );
    let unreadable = Table::new(&[
        (30, Some(20), 5, ""),
        (20, Some(10), 5, ""),
        (10, None, 5, ""),
    ]);
    assert_eq!(
        herdr_verdict(unreadable, 30, 10),
        LocateReply::Verified { index: 0, root: 10 }
    );
}

// Kills: a wall that hides a root nearer than itself (the hc candidate's own root, H1).
#[test]
fn root_below_the_wall_still_verifies() {
    let table = Table::new(&[
        (50, Some(40), 5, "claude"),
        (40, Some(30), 5, "herdr-core"),
        (30, None, 5, "hc"),
    ]);
    assert_eq!(
        herdr_verdict(table, 50, 40),
        LocateReply::Verified { index: 0, root: 40 }
    );
}

// Kills: any verdict cache: two identical calls must each ask the host, and a reused pid (same
// pid, new parent chain) or a subject that becomes readable must be seen by the next call.
#[test]
fn identical_calls_each_walk() {
    let table = Table::new(&[(30, Some(10), 100, "a"), (10, None, 1, "sh")]);
    let service = LocateCallerService::new(table.clone());
    let host = Arc::new(Roots {
        answers: vec![("w1:p1", Ok(Some(10)))],
        asked: AtomicUsize::new(0),
        terminals: false,
    });
    let port: Arc<dyn HostPort> = host.clone();
    let ask = || {
        service.locate(
            &request(vec![pane("w1:p1")], Some(30)),
            None,
            std::slice::from_ref(&port),
            &no_core,
            &budget(),
        )
    };
    let first = ask();
    assert_eq!(first, LocateReply::Verified { index: 0, root: 10 });
    assert_eq!(ask(), first);
    assert_eq!(host.asked.load(Ordering::SeqCst), 2, "walked both times");
    // The pid is reused by a process outside the pane.
    table.set(&[(30, None, 200, "b"), (10, None, 1, "sh")]);
    assert_eq!(ask(), walled());
    assert_eq!(host.asked.load(Ordering::SeqCst), 3);
}

fn hc_terminal_at(root: u32) -> Arc<dyn HostPort> {
    Roots::hc_port(vec![("t1", Ok(Some(root)))])
}

fn both_verdict(
    table: Arc<Table>,
    subject: u32,
    pane_shell: u32,
    terminal_root: u32,
) -> LocateReply {
    LocateCallerService::new(table).locate(
        &request(vec![pane("w1:p1"), terminal()], Some(subject)),
        None,
        &[herdr_pane_at(pane_shell)],
        &core_of(hc_terminal_at(terminal_root)),
        &budget(),
    )
}

fn unverified_both() -> LocateReply {
    LocateReply::Unverified {
        per_candidate: vec![CandidateVerdict::NotAncestor, CandidateVerdict::NotAncestor],
    }
}

// Kills: a family-ranked verdict (pane always, or terminal always) instead of the nearest
// ancestor, in either nesting order.
#[test]
fn nearest_ancestor_wins_in_both_nestings() {
    // An hc terminal run inside a Herdr pane: the terminal's root is nearer.
    let inside_pane = Table::new(&[
        (30, Some(20), 5, "claude"),
        (20, Some(10), 5, "sh"),
        (10, None, 5, "-bash"),
    ]);
    assert_eq!(
        both_verdict(inside_pane, 30, 10, 20),
        LocateReply::Verified { index: 1, root: 20 }
    );
    // A Herdr pane opened inside an hc terminal: the pane's shell is nearer.
    let inside_terminal = Table::new(&[
        (30, Some(20), 5, "claude"),
        (20, Some(10), 5, "-bash"),
        (10, None, 5, "sh"),
    ]);
    assert_eq!(
        both_verdict(inside_terminal, 30, 20, 10),
        LocateReply::Verified { index: 0, root: 20 }
    );
}

// Kills: asking every host about both families (a core handed a pane id, a Herdr server a
// terminal id), which would count a foreign host's error against a candidate.
#[test]
fn each_family_asks_only_its_own_hosts() {
    let table = Table::new(&[(30, Some(10), 5, "claude"), (10, None, 5, "sh")]);
    let herdr = Arc::new(Roots {
        answers: vec![("w1:p1", Ok(Some(10)))],
        asked: AtomicUsize::new(0),
        terminals: false,
    });
    let hc = Arc::new(Roots {
        answers: vec![("t1", Err(()))],
        asked: AtomicUsize::new(0),
        terminals: true,
    });
    let ports: Vec<Arc<dyn HostPort>> = vec![herdr.clone(), hc.clone()];
    let reply = LocateCallerService::new(table).locate(
        &request(vec![pane("w1:p1"), terminal()], Some(30)),
        None,
        &ports,
        &core_of(hc.clone()),
        &budget(),
    );
    assert_eq!(reply, LocateReply::Verified { index: 0, root: 10 });
    assert_eq!(herdr.asked.load(Ordering::SeqCst), 1, "one pane question");
    assert_eq!(hc.asked.load(Ordering::SeqCst), 1, "one terminal question");
}

// Kills: a terminal candidate with no hc host attached reported as anything but unsupported.
#[test]
fn terminal_without_an_hc_host_is_unsupported() {
    let table = Table::new(&[(30, Some(10), 5, "claude"), (10, None, 5, "sh")]);
    let reply = LocateCallerService::new(table).locate(
        &request(vec![terminal()], Some(30)),
        None,
        &[herdr_pane_at(10)],
        &no_core,
        &budget(),
    );
    assert_eq!(
        reply,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::Unsupported],
        }
    );
}

// Kills: a daemon-mode Codex (its process reparented to init, outside both roots' subtrees)
// verifying either family: with two families nothing is an ancestor, so the call is refused.
// H-C1 by fallback: the live real-core run is remaining evidence.
#[test]
fn daemon_mode_codex_reparented_two_families_refuse() {
    let table = Table::new(&[
        (50, Some(1), 5, "codex"),
        (1, None, 5, "init"),
        (20, Some(10), 5, "sh"),
        (10, None, 5, "-bash"),
    ]);
    assert_eq!(both_verdict(table, 50, 10, 20), unverified_both());
}

// Kills: the double-forked caller (reparented before it ran anything) being verified by a
// root it only shares a session with.
#[test]
fn double_forked_two_families_refuse() {
    let table = Table::new(&[(60, Some(1), 5, "sh"), (1, None, 5, "init")]);
    assert_eq!(both_verdict(table, 60, 10, 20), unverified_both());
}

// Kills: one-family verification. A single family is accepted unverified (TRUST-POLICY A1):
// the CLI never asks the daemon, so a reparented Codex with one family keeps working.
#[test]
fn one_family_accepted_unverified() {
    use crate::cli::caller_guard::{Guard, cli_pane};
    use crate::cli::caller_location::CallerEnv;
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        CallerEnv::from_lookup(move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| std::ffi::OsString::from(v))
        })
    };
    let never = |_: &[crate::cli::caller_location::CallerLocator]| -> Guard {
        panic!("a One caller must not be verified")
    };
    assert_eq!(
        cli_pane(&env(&[("HERDR_PANE_ID", "w1:p1")]), never, |_| {}).as_deref(),
        Some("w1:p1")
    );
    // A terminal alone names no pane, and is no refusal either.
    let reported = std::cell::Cell::new(false);
    assert_eq!(
        cli_pane(
            &env(&[("HC_TERMINAL_ID", "t1"), ("HC_TERMINAL_SOCKET", "/s")]),
            never,
            |_| reported.set(true),
        ),
        None
    );
    assert!(!reported.get());
}

// Kills: a wall that hides the hc candidate's own root, which sits below it (H1: the wall changes
// no outcome for an hc caller), and a Herdr root above the wall verifying for the same caller.
#[test]
fn hc_root_below_wall_found_first() {
    let table = Table::new(&[
        (50, Some(40), 5, "claude"),
        (40, Some(30), 5, "sh"),
        (30, Some(20), 5, "herdr-core"),
        (20, Some(10), 5, "-bash"),
        (10, None, 5, "herdr"),
    ]);
    // Both families: the hc root (40) is met before the wall (30) and the Herdr shell (20).
    assert_eq!(
        both_verdict(table.clone(), 50, 20, 40),
        LocateReply::Verified { index: 1, root: 40 }
    );
    // The Herdr candidate alone stays walled off, as before the hc host existed.
    assert_eq!(herdr_verdict(table, 50, 20), walled());
}

// The real adapters against both fakes: the Herdr pane's shell is pid 1 (fake_herdr) and the
// terminal's root is whatever pid fake_hc reports. The nearest ancestor wins.
#[cfg(feature = "test-support")]
#[test]
fn two_families_nearest_ancestor_wins() {
    use crate::test_support::fake_hc::{FakeHc, FakeTerminal};
    let dir = crate::test_support::scratch::temp_path("locate-two-families");
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("h.sock");
    let _herdr = crate::test_support::fake_herdr::FakeHerdr::start("loc", &socket, &["w1:p1"]);
    let cli: Arc<dyn HostPort> = Arc::new(crate::host::native::NativeCli::new(
        socket,
        Arc::new(SystemClock::new()),
    ));
    let core = FakeHc::start();
    core.add_terminal(FakeTerminal::live("t1"));
    let hc: Arc<dyn HostPort> = Arc::new(crate::host::hc::HcHost::new(
        crate::host::hc::HcClient::new(core.socket()),
    ));
    let ports = [cli];
    let ask = |service: &LocateCallerService| {
        service.locate(
            &request(vec![pane("w1:p1"), terminal()], Some(30)),
            None,
            &ports,
            &core_of(hc.clone()),
            &budget(),
        )
    };
    // tree: 30 -> 20 -> 10 -> 1. The terminal's root (10) is nearer than the pane's shell (1).
    core.set_pid("t1", Some(10));
    assert_eq!(
        ask(&LocateCallerService::new(tree())),
        LocateReply::Verified { index: 1, root: 10 }
    );
    // A terminal root outside the subject's ancestry leaves the pane the only ancestor root.
    core.set_pid("t1", Some(99));
    assert_eq!(
        ask(&LocateCallerService::new(tree())),
        LocateReply::Verified { index: 0, root: 1 }
    );
    // A terminal with no pid verifies nothing; the pane still does.
    core.set_pid("t1", None);
    assert_eq!(
        ask(&LocateCallerService::new(tree())),
        LocateReply::Verified { index: 0, root: 1 }
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// A `CorePort` selecting by socket: the port registered for it, else `Unsupported`.
fn cores(
    table: Vec<(&'static str, Arc<dyn HostPort>)>,
) -> impl Fn(&str) -> Result<Arc<dyn HostPort>, ApiError> {
    move |socket| {
        table
            .iter()
            .find(|(s, _)| *s == socket)
            .map(|(_, port)| Arc::clone(port))
            .ok_or_else(|| ApiError::unsupported("no core"))
    }
}

fn terminal_at(socket: &str) -> LocateCandidate {
    LocateCandidate::Terminal {
        socket: socket.into(),
        id: "t1".into(),
    }
}

// Kills: `candidate_root` taking the first core answering for `tN`: two cores both own `t1`,
// only the second one's root is the subject's ancestor.
#[test]
fn two_cores_sharing_tn_select_by_socket() {
    let table = Table::new(&[(30, Some(20), 5, "claude"), (20, None, 5, "sh")]);
    let first = hc_terminal_at(99);
    let second = hc_terminal_at(20);
    let core = cores(vec![("/a.sock", first), ("/b.sock", second)]);
    let service = LocateCallerService::new(table);
    let reply = service.locate(
        &request(vec![terminal_at("/b.sock")], Some(30)),
        None,
        &[],
        &core,
        &budget(),
    );
    assert_eq!(reply, LocateReply::Verified { index: 0, root: 20 });
    let other = service.locate(
        &request(vec![terminal_at("/a.sock")], Some(30)),
        None,
        &[],
        &core,
        &budget(),
    );
    assert_eq!(
        other,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::NotAncestor],
        }
    );
}

// Kills: a terminal candidate asking only the already attached hosts: `hosts` is empty, the
// core is attached on the call.
#[test]
fn terminal_candidate_attaches_its_core_on_a_miss() {
    let table = Table::new(&[(30, Some(20), 5, "claude"), (20, None, 5, "sh")]);
    let reply = LocateCallerService::new(table).locate(
        &request(vec![terminal()], Some(30)),
        None,
        &[],
        &core_of(hc_terminal_at(20)),
        &budget(),
    );
    assert_eq!(reply, LocateReply::Verified { index: 0, root: 20 });
}

// Kills: an attach failure reported as a final verdict: the next call, with the core attachable,
// verifies.
#[test]
fn unattachable_core_is_unreachable() {
    let table = Table::new(&[(30, Some(20), 5, "claude"), (20, None, 5, "sh")]);
    let service = LocateCallerService::new(table);
    let down = |_: &str| -> Result<Arc<dyn HostPort>, ApiError> {
        Err(ApiError::host_unavailable("attach failed"))
    };
    let first = service.locate(
        &request(vec![terminal()], Some(30)),
        None,
        &[],
        &down,
        &budget(),
    );
    assert_eq!(
        first,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::Unreachable],
        }
    );
    let second = service.locate(
        &request(vec![terminal()], Some(30)),
        None,
        &[],
        &core_of(hc_terminal_at(20)),
        &budget(),
    );
    assert_eq!(second, LocateReply::Verified { index: 0, root: 20 });
}

// Kills: a hint parser that reads a pane id as a terminal or loses the socket.
#[test]
fn hint_candidates_split_pane_and_terminals() {
    assert_eq!(hint_candidates("w1:p2"), vec![pane("w1:p2")]);
    assert_eq!(
        hint_candidates("w1:p2\n/run/hc/a.sock:t4"),
        vec![pane("w1:p2"), terminal_at_id("/run/hc/a.sock", "t4")]
    );
    assert_eq!(hint_candidates(""), Vec::<LocateCandidate>::new());
}

fn terminal_at_id(socket: &str, id: &str) -> LocateCandidate {
    LocateCandidate::Terminal {
        socket: socket.into(),
        id: id.into(),
    }
}
