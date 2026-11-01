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
}

impl Roots {
    pub(crate) fn port(answers: Vec<(&'static str, Result<Option<u32>, ()>)>) -> Arc<dyn HostPort> {
        Arc::new(Self {
            answers,
            asked: AtomicUsize::new(0),
        })
    }
}

impl HostPort for Roots {
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
        guard_key: None,
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
        0,
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
        0,
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
        0,
        &budget(),
    );
    assert_eq!(
        reply,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::NotAncestor, CandidateVerdict::Unsupported],
            first_guard: false,
        }
    );
    // No subject at all verifies nothing either.
    let none = service.locate(&request(vec![pane("w1:p1")], None), None, &[], 0, &budget());
    assert!(matches!(none, LocateReply::Unverified { .. }));
}

// Kills: an unreachable host answering Verified or panicking, and an unreachable verdict being
// cached (the next call must ask again once the host is back).
#[test]
fn unreachable_host_counts_unverified() {
    let service = LocateCallerService::new(tree());
    let down = Roots::port(vec![("w1:p1", Err(()))]);
    let reply = service.locate(
        &request(vec![pane("w1:p1")], Some(30)),
        None,
        std::slice::from_ref(&down),
        0,
        &budget(),
    );
    assert_eq!(
        reply,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::Unreachable],
            first_guard: false,
        }
    );
    let up = Roots::port(vec![("w1:p1", Ok(Some(10)))]);
    let again = service.locate(
        &request(vec![pane("w1:p1")], Some(30)),
        None,
        &[up],
        0,
        &budget(),
    );
    assert_eq!(again, LocateReply::Verified { index: 0, root: 10 });
    // No hosts at all is the same refusal.
    let empty = service.locate(
        &request(vec![pane("w1:p1")], Some(98)),
        None,
        &[],
        0,
        &budget(),
    );
    assert_eq!(
        empty,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::Unreachable],
            first_guard: false,
        }
    );
}

// Kills: a cache that never hits (the host is asked twice), one that ignores the epoch, and one
// that keys on the candidates alone.
#[test]
fn cache_hits_same_key() {
    let service = LocateCallerService::new(tree());
    let host = Arc::new(Roots {
        answers: vec![("w1:p1", Ok(Some(10)))],
        asked: AtomicUsize::new(0),
    });
    let port: Arc<dyn HostPort> = host.clone();
    let ask = |subject: u32, epoch: u64| {
        service.locate(
            &request(vec![pane("w1:p1")], Some(subject)),
            None,
            std::slice::from_ref(&port),
            epoch,
            &budget(),
        )
    };
    let first = ask(30, 7);
    assert_eq!(ask(30, 7), first);
    assert_eq!(host.asked.load(Ordering::SeqCst), 1);
    ask(30, 8);
    assert_eq!(
        host.asked.load(Ordering::SeqCst),
        2,
        "a new epoch asks again"
    );
    ask(20, 8);
    assert_eq!(
        host.asked.load(Ordering::SeqCst),
        3,
        "a new subject asks again"
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
        0,
        &budget(),
    );
    assert_eq!(inside, LocateReply::Verified { index: 1, root: 1 });
    let outside = service.locate(
        &request(vec![terminal(), pane("w1:p1")], Some(99)),
        None,
        &[cli],
        0,
        &budget(),
    );
    assert!(matches!(outside, LocateReply::Unverified { .. }));
    std::fs::remove_dir_all(dir).unwrap();
}

// Kills: a guard memory that answers `first` more than once per key, one shared across keys, and
// one that survives the service (a restarted daemon is a new service and arms it again).
#[test]
fn guard_memory_is_once_per_key_and_per_daemon() {
    let ask = |service: &LocateCallerService, key: &str| {
        let mut req = request(vec![pane("w1:p1")], Some(99));
        req.guard_key = Some(key.into());
        match service.locate(&req, None, &[], 0, &budget()) {
            LocateReply::Unverified { first_guard, .. } => first_guard,
            other => panic!("{other:?}"),
        }
    };
    let service = LocateCallerService::new(tree());
    assert!(ask(&service, "hermes:s1"));
    assert!(!ask(&service, "hermes:s1"));
    assert!(!ask(&service, "hermes:s1"));
    assert!(ask(&service, "hermes:s2"));
    assert!(ask(&LocateCallerService::new(tree()), "hermes:s1"));
    // A verified caller is never remembered: it owes no guard line.
    let host = Roots::port(vec![("w1:p1", Ok(Some(10)))]);
    let mut verified = request(vec![pane("w1:p1")], Some(30));
    verified.guard_key = Some("hermes:s3".into());
    assert!(matches!(
        service.locate(&verified, None, &[host], 0, &budget()),
        LocateReply::Verified { .. }
    ));
    assert!(ask(&service, "hermes:s3"));
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
        0,
        &budget(),
    )
}

fn walled() -> LocateReply {
    LocateReply::Unverified {
        per_candidate: vec![CandidateVerdict::NotAncestor],
        first_guard: false,
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

// Kills: a cache keyed on the pid alone (a reused pid hit another process's verdict).
#[test]
fn reused_pid_with_new_start_misses_cache() {
    let table = Table::new(&[(30, Some(10), 100, "a"), (10, None, 1, "sh")]);
    let service = LocateCallerService::new(table.clone());
    let host = Arc::new(Roots {
        answers: vec![("w1:p1", Ok(Some(10)))],
        asked: AtomicUsize::new(0),
    });
    let port: Arc<dyn HostPort> = host.clone();
    let ask = || {
        service.locate(
            &request(vec![pane("w1:p1")], Some(30)),
            None,
            std::slice::from_ref(&port),
            0,
            &budget(),
        )
    };
    let first = ask();
    assert_eq!(first, LocateReply::Verified { index: 0, root: 10 });
    assert_eq!(ask(), first);
    assert_eq!(host.asked.load(Ordering::SeqCst), 1);
    // The pid is reused by a process outside the pane.
    table.set(&[(30, None, 200, "b"), (10, None, 1, "sh")]);
    assert_eq!(ask(), walled());
    assert_eq!(host.asked.load(Ordering::SeqCst), 2);
}

// Kills: caching a subject with no readable record (no start stamp to tell it from a reuse).
#[test]
fn unstamped_subject_never_cached() {
    let table = Table::new(&[]);
    let service = LocateCallerService::new(table.clone());
    let host = Arc::new(Roots {
        answers: vec![("w1:p1", Ok(Some(10)))],
        asked: AtomicUsize::new(0),
    });
    let port: Arc<dyn HostPort> = host.clone();
    let ask = || {
        service.locate(
            &request(vec![pane("w1:p1")], Some(30)),
            None,
            std::slice::from_ref(&port),
            0,
            &budget(),
        )
    };
    assert_eq!(ask(), walled());
    assert_eq!(ask(), walled());
    assert_eq!(host.asked.load(Ordering::SeqCst), 2, "walked both times");
    // Once the process is readable the walk sees it, not a cached miss.
    table.set(&[(30, Some(10), 7, "a"), (10, None, 1, "sh")]);
    assert_eq!(ask(), LocateReply::Verified { index: 0, root: 10 });
}
