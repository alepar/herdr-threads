//! N2-hint: tmux hint verification and the per-target wake answer of `NamedHost`, against a
//! private tmux server (`tmux -L ht-test-<uuid>`, `kill-server` on Drop). A test skips, with the
//! recorded reason, when `tmux` is not installed.
use super::*;
use crate::harness::adapter::TurnEdge;
use crate::service::turn_state::{TurnKey, TurnState, TurnStateBook};
use crate::{
    host::process_table::HostProcessTable,
    ports::ProcessInfo,
    protocol::time::{Cancellation, MonoInstant},
    test_support::tmux::{Pane, PrivateTmux},
};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

struct NoSeats;
impl NamedSeatSource for NoSeats {
    fn live_named(&self, _: &HostId, _: &CallBudget) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
        Ok(Vec::new())
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(5)
    }
    fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
        crate::protocol::time::MonoInstant(7)
    }
}

fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    }
}

fn host() -> NamedHost {
    NamedHost::new(
        "0123456789abcdef0123456789abcdef",
        HostId::from_row(9),
        Arc::new(NoSeats),
        Arc::new(FixedClock),
    )
}

fn target(name: &str) -> HostTargetId {
    HostTargetId::new(name)
}

/// A pane running `sh -c 'sleep 600; true'`: the shell stays the pane process and `sleep` is its
/// child, the shape of a harness started by a shell.
fn shell_pane(tmux: &PrivateTmux) -> (Pane, u32) {
    let pane = tmux.new_pane("sleep 600; true");
    let child = wait_for(|| children(pane.pid).into_iter().next());
    (pane, child)
}

fn children(pid: u32) -> Vec<u32> {
    let out = std::process::Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|word| word.parse().ok())
        .collect()
}

fn wait_for<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "condition not reached in 5 s");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn composer() -> WakeCapability {
    WakeCapability::Composer(ComposerTraits::TMUX)
}

/// A process outside every tmux pane: a child of the test that nothing in tmux started.
fn outsider() -> crate::test_support::spawn::OwnedChild {
    use crate::test_support::spawn::SpawnOwned;
    crate::test_support::spawn::command("sleep")
        .arg("600")
        .spawn_owned()
        .expect("spawn sleep")
}

// Kills: Composer for a binding whose hint verifies in the wrong way (any hint, no walk), and
// Composer for other targets of the host.
#[test]
fn verified_binding_answers_composer() {
    let Some(tmux) = PrivateTmux::start("verified_binding_answers_composer") else {
        return;
    };
    let (pane, child) = shell_pane(&tmux);
    let host = host();
    assert!(host.record_tmux_hint(&target("alice"), child, &tmux.hint(&pane)));
    assert_eq!(host.target_wake_capability(&target("alice")), composer());
    assert_eq!(
        host.target_wake_capability(&target("bob")),
        WakeCapability::None
    );
    // The pane's own process is a valid subject too (a harness that is the pane command).
    assert!(host.record_tmux_hint(&target("carol"), pane.pid, &tmux.hint(&pane)));
    assert_eq!(host.target_wake_capability(&target("carol")), composer());
}

// Kills: Composer without a verified hint; a hint that was never sent; each failing half of the
// verification (server pid, pane existence, ancestry, malformed text).
#[test]
fn unverified_binding_answers_none() {
    let Some(tmux) = PrivateTmux::start("unverified_binding_answers_composer") else {
        return;
    };
    let (pane, child) = shell_pane(&tmux);
    let good = tmux.hint(&pane);
    let host = host();
    assert_eq!(
        host.target_wake_capability(&target("alice")),
        WakeCapability::None,
        "no hint at all"
    );
    let socket = tmux.socket_path();
    let server = tmux.server_pid();
    for (what, hint) in [
        (
            "wrong server pid",
            format!("tmux:{}:{}:{}", socket.display(), server + 1, pane.id),
        ),
        (
            "no such pane",
            format!("tmux:{}:{server}:%9999", socket.display()),
        ),
        (
            "no such server",
            format!("tmux:{}.gone:{server}:{}", socket.display(), pane.id),
        ),
        ("garbled", "tmux:nonsense".to_owned()),
        ("empty", String::new()),
    ] {
        assert!(
            !host.record_tmux_hint(&target("alice"), child, &hint),
            "{what}"
        );
        assert_eq!(
            host.target_wake_capability(&target("alice")),
            WakeCapability::None,
            "{what}"
        );
    }
    // The right hint from a process that is not in the pane.
    let mut outside = outsider();
    assert!(!host.record_tmux_hint(&target("alice"), outside.id(), &good));
    assert_eq!(
        host.target_wake_capability(&target("alice")),
        WakeCapability::None,
        "a process outside the pane"
    );
    let _ = outside.kill();
    let _ = outside.wait();
}

// Kills: a failed check that moves, rebinds or un-observes the seat (T4); a stale hint that
// replaces or clears the verified binding of another process.
#[test]
fn stale_tmux_pane_wrong_server_pid_disables_wake_and_never_moves_seat() {
    let Some(tmux) = PrivateTmux::start("stale_tmux_pane_wrong_server_pid") else {
        return;
    };
    let (pane, child) = shell_pane(&tmux);
    let host = host();
    let alice = target("alice");
    let before = host.observe_current_target(&alice, &context()).unwrap();
    // A pane id reused on a restarted server: the right socket and pane, another server pid.
    let stale = format!(
        "tmux:{}:{}:{}",
        tmux.socket_path().display(),
        tmux.server_pid() + 1,
        pane.id
    );
    assert!(!host.record_tmux_hint(&alice, child, &stale));
    assert_eq!(host.target_wake_capability(&alice), WakeCapability::None);
    let after = host.observe_current_target(&alice, &context()).unwrap();
    assert_eq!(
        (
            &before.target,
            &before.terminal,
            before.generation,
            &before.incarnation
        ),
        (
            &after.target,
            &after.terminal,
            after.generation,
            &after.incarnation
        ),
        "the seat was neither moved nor re-observed elsewhere"
    );
    // A verified binding survives a later stale hint from another process.
    assert!(host.record_tmux_hint(&alice, child, &tmux.hint(&pane)));
    assert!(!host.record_tmux_hint(&alice, child, &stale));
    assert_eq!(host.target_wake_capability(&alice), composer());
}

// Kills: Composer for a TMUX_PANE inherited into a nested Herdr session (a pane that is not an
// ancestor of the caller), with a perfectly valid server pid and a pane that exists.
#[cfg(feature = "test-support")]
#[test]
fn stale_tmux_pane_nested_in_isolated_herdr() {
    use crate::test_support::isolated_herdr::IsolatedHerdr;
    let Some(tmux) = PrivateTmux::start("stale_tmux_pane_nested_in_isolated_herdr") else {
        return;
    };
    let Some(herdr) = IsolatedHerdr::new("stale_tmux_pane_nested_in_isolated_herdr") else {
        return;
    };
    std::fs::create_dir_all(herdr.root().join("cfg")).unwrap();
    std::fs::write(
        herdr.root().join("cfg/herdr.toml"),
        "default_shell = '/bin/sh'\n",
    )
    .unwrap();
    herdr.start();
    let created = herdr
        .command("herdr")
        .args([
            "workspace",
            "create",
            "--label",
            "ht-nested",
            "--no-focus",
            "--cwd",
        ])
        .arg(herdr.root().join("home"))
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let server = herdr.pid().expect("the isolated Herdr server pid");
    // The pane's shell (`default_shell`), not the short-lived helpers Herdr also starts.
    let pane_process = wait_for(|| {
        descendants(server).into_iter().find(|pid| {
            HostProcessTable.process(*pid).is_some_and(|record| {
                record
                    .argv
                    .first()
                    .is_some_and(|argv0| argv0.trim_start_matches('-').ends_with("sh"))
            })
        })
    });
    let (pane, child) = shell_pane(&tmux);
    let host = host();
    // Control: the same hint verifies for a process that is in the pane.
    assert!(host.record_tmux_hint(&target("control"), child, &tmux.hint(&pane)));
    // The outer tmux pane exists and its server pid is right; the caller is a Herdr pane process.
    assert!(!host.record_tmux_hint(&target("alice"), pane_process, &tmux.hint(&pane)));
    assert_eq!(
        host.target_wake_capability(&target("alice")),
        WakeCapability::None
    );
}

#[cfg(feature = "test-support")]
fn descendants(pid: u32) -> Vec<u32> {
    let mut all = Vec::new();
    let mut frontier = vec![pid];
    while let Some(next) = frontier.pop() {
        for child in children(next) {
            all.push(child);
            frontier.push(child);
        }
    }
    all
}

// Kills: a stale Composer after the binding's process exited; a verified entry that outlives its
// process in daemon memory.
#[test]
fn verified_binding_ends_with_its_process() {
    let Some(tmux) = PrivateTmux::start("verified_binding_ends_with_its_process") else {
        return;
    };
    let (pane, child) = shell_pane(&tmux);
    let host = host();
    let alice = target("alice");
    assert!(host.record_tmux_hint(&alice, child, &tmux.hint(&pane)));
    assert_eq!(host.target_wake_capability(&alice), composer());
    tmux.run(&["kill-pane", "-t", &pane.id]);
    wait_for(|| HostProcessTable.process(child).is_none().then_some(()));
    assert_eq!(host.target_wake_capability(&alice), WakeCapability::None);
}

// Kills: a binding anchored on the short-lived hook that carried a ResolveSeat hint and left
// dead by the next turn edge, instead of on the harness the edge fingerprints: the harness here
// is the pane process itself (no process below it), so the edge's record anchors on it.
#[test]
fn harness_as_pane_process_keeps_its_binding() {
    let Some(tmux) = PrivateTmux::start("harness_as_pane_process_keeps_its_binding") else {
        return;
    };
    // The pane process (the harness) runs a short-lived hook child first, then a long one.
    let pane = tmux.new_pane("sleep 1; sleep 600; true");
    let hook = wait_for(|| children(pane.pid).into_iter().next());
    let host = host();
    let alice = target("alice");
    let hint = tmux.hint(&pane);
    // The ResolveSeat path: the hook is the subject and the anchor.
    assert!(host.record_tmux_hint(&alice, hook, &hint));
    // The turn edge path: the harness is the subject; it is the pane process itself.
    assert!(host.record_tmux_hint(&alice, pane.pid, &hint));
    wait_for(|| HostProcessTable.process(hook).is_none().then_some(()));
    assert!(host.has_live_tmux_binding(&alice));
    assert_eq!(host.target_wake_capability(&alice), composer());
}

// Kills: answering from one binding only (the first, the last), and an unverified binding that
// cancels a verified sibling in the same pane.
#[test]
fn two_bindings_one_pane_one_verified_is_composer() {
    let Some(tmux) = PrivateTmux::start("two_bindings_one_pane_one_verified") else {
        return;
    };
    let (pane, child) = shell_pane(&tmux);
    let hint = tmux.hint(&pane);
    let mut outside = outsider();
    let alice = target("alice");
    // Unverified first, then verified: the last one decides nothing.
    let host_a = host();
    assert!(!host_a.record_tmux_hint(&alice, outside.id(), &hint));
    assert!(host_a.record_tmux_hint(&alice, child, &hint));
    assert_eq!(host_a.target_wake_capability(&alice), composer());
    // Verified first, then unverified.
    let host_b = host();
    assert!(host_b.record_tmux_hint(&alice, child, &hint));
    assert!(!host_b.record_tmux_hint(&alice, outside.id(), &hint));
    assert_eq!(host_b.target_wake_capability(&alice), composer());
    // Two verified bindings in the pane: either may end and the other still answers Composer.
    let host_c = host();
    assert!(host_c.record_tmux_hint(&alice, child, &hint));
    assert!(host_c.record_tmux_hint(&alice, pane.pid, &hint));
    assert_eq!(host_c.target_wake_capability(&alice), composer());
    let _ = outside.kill();
    let _ = outside.wait();
}

#[test]
fn neither_verified_is_none() {
    let Some(tmux) = PrivateTmux::start("neither_verified_is_none") else {
        return;
    };
    let (pane, _child) = shell_pane(&tmux);
    let hint = tmux.hint(&pane);
    let mut one = outsider();
    let mut two = outsider();
    let host = host();
    let alice = target("alice");
    assert!(!host.record_tmux_hint(&alice, one.id(), &hint));
    assert!(!host.record_tmux_hint(&alice, two.id(), &hint));
    assert_eq!(host.target_wake_capability(&alice), WakeCapability::None);
    for child in [&mut one, &mut two] {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// A source with one live named seat and chosen top-level binding harnesses (or a failing read).
struct HarnessSeats {
    harnesses: Vec<&'static str>,
    fail: bool,
}
impl NamedSeatSource for HarnessSeats {
    fn live_named(&self, _: &HostId, _: &CallBudget) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
        Ok(vec![(
            SeatId::new("seat-alice"),
            SeatName::parse("alice").unwrap(),
        )])
    }
    fn top_level_bindings(
        &self,
        _: &SeatId,
        _: &CallBudget,
    ) -> Result<Vec<crate::store::named_seats::TopLevelBinding>, ApiError> {
        if self.fail {
            return Err(ApiError::new(ErrorCode::HostUnavailable, "store read"));
        }
        Ok(self
            .harnesses
            .iter()
            .enumerate()
            .map(|(i, harness)| crate::store::named_seats::TopLevelBinding {
                generation: 1,
                harness: (*harness).to_owned(),
                native_session: crate::protocol::ids::NativeSessionId::new(format!("s{i}")),
            })
            .collect())
    }
}

// Kills: Composer for a seat whose harness has no blocked-screen recognizer (the gate refuses it
// forever, so the sender would never see `wake=no_wake`), None for a mixed seat, and a store
// transient flipping the answer.
#[test]
fn no_recognizer_harness_answers_none() {
    let Some(tmux) = PrivateTmux::start("no_recognizer_harness_answers_none") else {
        return;
    };
    let (pane, child) = shell_pane(&tmux);
    let hint = tmux.hint(&pane);
    let alice = target("alice");
    for (harnesses, fail, expected) in [
        (vec!["codex"], false, WakeCapability::None),
        (vec!["claude"], false, composer()),
        (vec!["codex", "claude"], false, composer()),
        (vec!["codex"], true, composer()),
        (vec![], false, composer()),
    ] {
        let host = NamedHost::new(
            "0123456789abcdef0123456789abcdef",
            HostId::from_row(9),
            Arc::new(HarnessSeats {
                harnesses: harnesses.clone(),
                fail,
            }),
            Arc::new(FixedClock),
        );
        assert!(host.record_tmux_hint(&alice, child, &hint));
        assert_eq!(
            host.target_wake_capability(&alice),
            expected,
            "{harnesses:?} fail={fail}"
        );
    }
}

// Kills: a gate refusal that leaves no block (the sender's overdue line has no `wake=`), a block
// that survives a clean wake, and one that never expires.
#[test]
fn every_gate_refusal_records_a_block() {
    let blocked = |rig: &WakeRig, expected: WakeBlock, case: &str| {
        assert_eq!(rig.host.wake_block(&rig.seat), Some(expected), "{case}");
    };
    // (case, harness, screen, marker, binding states, expected)
    let draft = IDLE.replacen("\n❯\n", "\n❯ half-written draft\n", 1);
    for (case, screen, marker, states, expected) in [
        (
            "rb_draft",
            draft.as_str(),
            "half-written draft",
            vec![Some(TurnEdge::Idle)],
            WakeBlock::Unknown,
        ),
        (
            "rb_busy",
            IDLE,
            "manual mode on",
            vec![Some(TurnEdge::Busy)],
            WakeBlock::Unknown,
        ),
        (
            "rb_unknown",
            IDLE,
            "manual mode on",
            vec![None],
            WakeBlock::Unknown,
        ),
        (
            "rb_blocked",
            CLAUDE_APPROVAL,
            "",
            vec![Some(TurnEdge::Idle)],
            WakeBlock::Unknown,
        ),
        (
            "rb_unrecognised",
            "not a claude screen\n",
            "not a claude",
            vec![Some(TurnEdge::Idle)],
            WakeBlock::Unreadable,
        ),
        (
            "rb_no_binding",
            IDLE,
            "manual mode on",
            vec![],
            WakeBlock::Unknown,
        ),
    ] {
        let Some(rig) = WakeRig::new(case, "claude", screen, marker) else {
            return;
        };
        rig.bindings(&states);
        assert_eq!(rig.host.wake_block(&rig.seat), None, "{case}: before");
        refused(rig.attempt());
        blocked(&rig, expected, case);
        assert_eq!(rig.typed(), "", "{case}");
    }
    let Some(rig) = WakeRig::new("rb_modes", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let id = rig.pane.id.clone();
    // Input off, then synchronized panes.
    rig.tmux.run(&["select-pane", "-d", "-t", &id]);
    refused(rig.attempt());
    blocked(&rig, WakeBlock::Unknown, "input off");
    rig.tmux.run(&["select-pane", "-e", "-t", &id]);
    rig.tmux
        .run(&["set-window-option", "-t", &id, "synchronize-panes", "on"]);
    refused(rig.attempt());
    blocked(&rig, WakeBlock::Unknown, "synchronized");
    rig.tmux
        .run(&["set-window-option", "-t", &id, "synchronize-panes", "off"]);
    // pane_in_mode.
    rig.tmux.run(&["copy-mode", "-t", &id]);
    refused(rig.attempt());
    blocked(&rig, WakeBlock::Unknown, "pane in mode");
    rig.tmux.run(&["send-keys", "-t", &id, "-X", "cancel"]);
    // A stale epoch.
    let (safe, mut context) = rig.target_and_context();
    context.expected_epoch = Some(safe.epoch + 1);
    refused(
        rig.host
            .submit_prompt_if_owed(&safe, WAKE, &context, &|| Ok(true)),
    );
    blocked(&rig, WakeBlock::Unknown, "stale epoch");
    // Two panes for one seat.
    let second = rig.tmux.new_pane("sleep 600");
    assert!(
        rig.host
            .record_tmux_hint(&target("seat-wake"), second.pid, &rig.tmux.hint(&second))
    );
    refused(rig.attempt());
    blocked(&rig, WakeBlock::Unknown, "two panes");
    // An expired block reads None.
    rig.clock.set(10 * 60_000 + 1);
    assert_eq!(rig.host.wake_block(&rig.seat), None, "expired");

    // A clean wake clears the block.
    let Some(rig) = WakeRig::new("rb_clear", "claude", IDLE, "manual mode on") else {
        return;
    };
    let keys = rig.bindings(&[Some(TurnEdge::Busy)]);
    refused(rig.attempt());
    blocked(&rig, WakeBlock::Unknown, "busy");
    rig.book.record(&keys[0], TurnEdge::Idle, 0);
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    assert_eq!(rig.host.wake_block(&rig.seat), None, "cleared by a wake");
}

// Kills: a server a test starts outliving the fixture (the leak check's other half is the
// Python test of scripts/check-no-leaked-processes).
#[test]
fn dropping_the_private_server_stops_it() {
    let Some(tmux) = PrivateTmux::start("dropping_the_private_server_stops_it") else {
        return;
    };
    let pid = tmux.server_pid();
    assert!(HostProcessTable.process(pid).is_some(), "the server runs");
    drop(tmux);
    wait_for(|| HostProcessTable.process(pid).is_none().then_some(()));
}

// ---- N2-wake: the composer wake of a verified tmux binding ----

const IDLE: &str = include_str!("../fixtures/screen_blocked/claude/idle.txt");
const AGY_IDLE: &str = include_str!("../fixtures/screen_blocked/agy/idle.txt");
const CLAUDE_APPROVAL: &str = include_str!("../fixtures/screen_blocked/claude/approval.txt");
const WAKE: &str = "wake-text";
// Claude Code 2.1.309 captures (docs/evidence/ansi-composer): a dim prompt suggestion, a typed
// draft, and the suggestion's detection text (what `capture-pane -p` shows).
const SUGGESTION_ANSI: &str = include_str!(
    "../../docs/evidence/ansi-composer/captures/claude-2.1.309-suggestion.recent-ansi.txt"
);
const SUGGESTION_PLAIN: &str = include_str!(
    "../../docs/evidence/ansi-composer/captures/claude-2.1.309-suggestion.read-detection.txt"
);
const DRAFT_ANSI: &str =
    include_str!("../../docs/evidence/ansi-composer/captures/claude-2.1.309-draft.recent-ansi.txt");

/// A clock the test moves (milliseconds).
#[derive(Default)]
pub(crate) struct ManualClock(AtomicU64);
impl ManualClock {
    pub(crate) fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}
impl Clock for ManualClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

/// The seat's connected top-level binding generations, set by the test.
#[derive(Default)]
pub(crate) struct GenSeats(
    Mutex<Vec<crate::store::named_seats::TopLevelBinding>>,
    /// Run by every `top_level_bindings` read, which the wake gate makes before its capture.
    Mutex<Option<Box<dyn Fn() + Send>>>,
);
impl NamedSeatSource for GenSeats {
    fn live_named(&self, _: &HostId, _: &CallBudget) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
        Ok(Vec::new())
    }
    fn top_level_bindings(
        &self,
        _: &SeatId,
        _: &CallBudget,
    ) -> Result<Vec<crate::store::named_seats::TopLevelBinding>, ApiError> {
        if let Some(hook) = self.1.lock().unwrap().as_ref() {
            hook();
        }
        Ok(self.0.lock().unwrap().clone())
    }
}

/// A named host with a verified tmux binding in a pane that shows `screen` and delivers what is
/// typed to a file.
pub(crate) struct WakeRig {
    pub(crate) tmux: PrivateTmux,
    pub(crate) host: Arc<NamedHost>,
    pub(crate) clock: Arc<ManualClock>,
    pub(crate) book: Arc<TurnStateBook>,
    seats: Arc<GenSeats>,
    pub(crate) seat: SeatId,
    pub(crate) pane: Pane,
    typed: PathBuf,
    harness: &'static str,
}

fn scratch_file(name: &str, contents: &str) -> PathBuf {
    let path = crate::test_support::scratch::temp_path(format!(
        "ht-wake-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&path, contents).unwrap();
    path
}

fn screen_size(screen: &str) -> (usize, usize) {
    // An ANSI screen is sized by what it shows, not by its escapes.
    let rows: Vec<String> = match crate::harness::sgr::parse(screen) {
        Some(lines) => lines.into_iter().map(|line| line.text).collect(),
        None => screen.lines().map(str::to_owned).collect(),
    };
    let width = rows.iter().map(|l| l.chars().count()).max().unwrap_or(80);
    (width.max(40), (rows.len() + 4).max(24))
}

/// The script of a pane: wait for the window to be resized to the screen's size, show the screen,
/// then run `tail`.
fn pane_script(screen: &str, shown: &std::path::Path, tail: &str) -> String {
    let (width, _) = screen_size(screen);
    format!(
        "until [ \"$(stty size | cut -d' ' -f2)\" -ge {width} ]; do sleep 0.05; done; cat {}; {tail}",
        shown.display()
    )
}

impl WakeRig {
    /// `None` (skipped) when tmux is missing. The pane's screen is `screen`; its keys go to a file.
    pub(crate) fn new(
        case: &str,
        harness: &'static str,
        screen: &str,
        marker: &str,
    ) -> Option<Self> {
        let typed = scratch_file(&format!("{case}-typed"), "");
        let tail = format!("stty -echo; exec cat > {}", typed.display());
        Self::with_tail(case, harness, screen, marker, typed, &tail)
    }

    fn with_tail(
        case: &str,
        harness: &'static str,
        screen: &str,
        marker: &str,
        typed: PathBuf,
        tail: &str,
    ) -> Option<Self> {
        let tmux = PrivateTmux::start(case)?;
        let shown = scratch_file(&format!("{case}-screen"), screen);
        let pane = tmux.new_pane(&pane_script(screen, &shown, tail));
        let (width, rows) = screen_size(screen);
        let resized = tmux.run(&[
            "resize-window",
            "-t",
            &pane.id,
            "-x",
            &width.to_string(),
            "-y",
            &rows.to_string(),
        ]);
        assert!(resized.status.success(), "resize-window");
        // An empty marker waits for the screen's own last row, never for nothing.
        let last_row = screen.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("");
        let marker = if marker.is_empty() {
            last_row.trim()
        } else {
            marker
        };
        wait_for(|| {
            let out = tmux.run(&["capture-pane", "-p", "-t", &pane.id]);
            String::from_utf8_lossy(&out.stdout)
                .contains(marker)
                .then_some(())
        });
        let clock = Arc::new(ManualClock(AtomicU64::new(0)));
        let seats = Arc::new(GenSeats::default());
        let book = Arc::new(TurnStateBook::default());
        let host = Arc::new(NamedHost::new(
            "0123456789abcdef0123456789abcdef",
            HostId::from_row(9),
            seats.clone(),
            clock.clone(),
        ));
        host.set_turn_state(Arc::clone(&book));
        let seat = SeatId::new("seat-wake");
        // The pane's own process (a `sh` that became `cat`) is a valid subject.
        assert!(host.record_tmux_hint(&target("seat-wake"), pane.pid, &tmux.hint(&pane)));
        Some(Self {
            tmux,
            host,
            clock,
            book,
            seats,
            seat,
            pane,
            typed,
            harness,
        })
    }

    /// The connected top-level bindings the pane holds, with their turn state.
    pub(crate) fn bindings(&self, states: &[Option<TurnEdge>]) -> Vec<TurnKey> {
        let keys: Vec<TurnKey> = (1..=states.len() as u64)
            .map(|generation| TurnKey {
                seat: self.seat.clone(),
                generation: 1,
                harness: self.harness.to_owned(),
                native_session: crate::protocol::ids::NativeSessionId::new(format!(
                    "s{generation}"
                )),
            })
            .collect();
        *self.seats.0.lock().unwrap() = keys
            .iter()
            .map(|k| crate::store::named_seats::TopLevelBinding {
                generation: k.generation,
                harness: k.harness.clone(),
                native_session: k.native_session.clone(),
            })
            .collect();
        for (key, edge) in keys.iter().zip(states) {
            if let Some(edge) = edge {
                self.book.record(key, *edge, 0);
            }
        }
        keys
    }

    /// One wake attempt as the dispatcher makes it.
    pub(crate) fn attempt(&self) -> Result<Option<PromptOutcome>, ApiError> {
        let (safe, context) = self.target_and_context();
        self.host
            .submit_prompt_if_owed(&safe, WAKE, &context, &|| Ok(true))
    }

    fn target_and_context(&self) -> (SafeWakeTarget, HostCallContext) {
        let observation = self
            .host
            .observe_current_target(&target("seat-wake"), &context())
            .unwrap();
        let mut safe = self
            .host
            .safe_wake_target(&self.seat, &observation)
            .expect("a safe target for a verified binding");
        safe.bound_harness = Some(self.harness.to_owned());
        let context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(u64::MAX / 2),
                cancellation: Cancellation::default(),
            },
            expected_boot: Some(observation.host_boot.clone()),
            expected_epoch: Some(observation.epoch),
        };
        (safe, context)
    }

    /// Everything the pane's program received.
    pub(crate) fn typed(&self) -> String {
        std::fs::read_to_string(&self.typed).unwrap()
    }

    pub(crate) fn capture(&self) -> String {
        let out = self.tmux.run(&["capture-pane", "-p", "-t", &self.pane.id]);
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn refused(result: Result<Option<PromptOutcome>, ApiError>) -> String {
    let error = result.expect_err("the attempt must be refused");
    assert_eq!(
        error.code,
        ErrorCode::TargetUnsafe,
        "a pre-send refusal is retried, never no_wake: {error:?}"
    );
    error.detail
}

fn busy_edges(edges: &[TurnEdge]) -> Vec<Option<TurnEdge>> {
    edges.iter().copied().map(Some).collect()
}

// Kills: a stale busy cleared by one read, by reads under 5 s apart, before 60 s of busy, or by a
// recognizer that cannot tell (None) or sees a running turn (Some(true)); and a clear that is
// never recorded (the wake stays refused forever).
#[test]
fn stale_busy_cleared_only_by_screen_idle_rule() {
    let Some(rig) = WakeRig::new("stale_busy_idle", "claude", IDLE, "manual mode on") else {
        return;
    };
    let keys = rig.bindings(&busy_edges(&[TurnEdge::Busy]));
    let busy = || rig.book.state(&keys[0]) == TurnState::Busy;
    // Busy for 30 s: a Some(false) read alone is not enough.
    rig.clock.set(30_000);
    refused(rig.attempt());
    assert!(busy(), "busy under 60 s");
    // 61 s: the first read of the pair.
    rig.clock.set(61_000);
    refused(rig.attempt());
    assert!(busy(), "one read");
    // Under 5 s later: still one effective read.
    rig.clock.set(63_000);
    refused(rig.attempt());
    assert!(busy(), "reads under 5 s apart");
    assert_eq!(rig.typed(), "", "nothing typed while busy");
    // 6 s after the first read: two reads at least 5 s apart clear it, and the wake goes out.
    rig.clock.set(67_000);
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    assert_eq!(rig.book.state(&keys[0]), TurnState::Idle);
    wait_for(|| (rig.typed() == format!("{WAKE}\n")).then_some(()));

    // A recognizer that cannot tell, and a running turn, never clear it.
    for (case, screen, marker) in [
        (
            "stale_busy_none",
            "not a claude screen\n".to_owned(),
            "not a claude",
        ),
        (
            "stale_busy_running",
            format!("{}  esc to interrupt\n", IDLE.trim_end()),
            "esc to interrupt",
        ),
    ] {
        let Some(rig) = WakeRig::new(case, "claude", &screen, marker) else {
            return;
        };
        let keys = rig.bindings(&busy_edges(&[TurnEdge::Busy]));
        for at in [61_000, 70_000, 90_000, 400_000] {
            rig.clock.set(at);
            refused(rig.attempt());
        }
        assert_eq!(rig.book.state(&keys[0]), TurnState::Busy, "{case}");
        assert_eq!(rig.typed(), "", "{case}");
    }
}

// Kills: a wake typed into a running turn whose Busy edge was missed (every entry Idle, the
// screen shows the running footer).
#[test]
fn idle_entry_with_a_running_screen_refuses() {
    let screen = format!("{}  esc to interrupt\n", IDLE.trim_end());
    let Some(rig) = WakeRig::new("idle_running_screen", "claude", &screen, "esc to interrupt")
    else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let detail = refused(rig.attempt());
    assert!(detail.contains("running turn"), "{detail}");
    assert_eq!(rig.typed(), "");
}

// Kills: pane idleness keyed on seat and generation only, where one session's Stop marks the
// shared entry Idle while a sibling session of the same generation is mid-turn.
#[test]
fn sibling_binding_mid_turn_holds_the_wake() {
    let Some(rig) = WakeRig::new("sib_same_generation", "claude", IDLE, "manual mode on") else {
        return;
    };
    let keys = rig.bindings(&[Some(TurnEdge::Busy), Some(TurnEdge::Busy)]);
    assert_eq!(keys[0].generation, keys[1].generation);
    rig.clock.set(1_000);
    rig.book.record(&keys[0], TurnEdge::Idle, 1_000);
    refused(rig.attempt());
    assert_eq!(rig.typed(), "");
    assert_eq!(rig.book.state(&keys[1]), TurnState::Busy);
}

// Kills: typing into copy-mode (keys go to the mode, not the program), into a pane with input off
// or with synchronized panes (keys reach other panes), a mode treated as no_wake instead of busy,
// and a gate that moves the copy-mode scroll position.
#[test]
fn pane_in_mode_input_off_synchronized_no_typing() {
    let Some(rig) = WakeRig::new("pane_modes", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let id = rig.pane.id.clone();
    let scroll = || {
        let out = rig.tmux.run(&[
            "display",
            "-p",
            "-t",
            &id,
            "#{pane_in_mode} #{scroll_position}",
        ]);
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    // A shorter window pushes the top of the screen into history, so there is something to scroll.
    assert!(
        rig.tmux
            .run(&["resize-window", "-t", &id, "-y", "20"])
            .status
            .success()
    );
    // copy-mode, scrolled up a page.
    assert!(
        rig.tmux
            .run(&["copy-mode", "-u", "-t", &id])
            .status
            .success()
    );
    let before = scroll();
    assert!(
        before.starts_with("1 ") && before != "1 0",
        "scrolled up: {before}"
    );
    let detail = refused(rig.attempt());
    assert!(detail.contains("mode"), "{detail}");
    assert_eq!(scroll(), before, "the scroll position is untouched");
    rig.tmux.run(&["send-keys", "-t", &id, "-X", "cancel"]);
    // input off.
    assert!(
        rig.tmux
            .run(&["select-pane", "-d", "-t", &id])
            .status
            .success()
    );
    refused(rig.attempt());
    rig.tmux.run(&["select-pane", "-e", "-t", &id]);
    // synchronized panes.
    rig.tmux
        .run(&["set-window-option", "-t", &id, "synchronize-panes", "on"]);
    refused(rig.attempt());
    rig.tmux
        .run(&["set-window-option", "-t", &id, "synchronize-panes", "off"]);
    assert_eq!(rig.typed(), "", "nothing was typed in any mode");
    // Control: with the modes cleared the same attempt goes out once.
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    wait_for(|| (rig.typed() == format!("{WAKE}\n")).then_some(()));
}

// Kills: an idle sibling authorising a wake into a pane whose shared TUI runs another binding's
// turn; an Unknown sibling read as idle; a wake typed twice once both are idle.
#[test]
fn sibling_bindings_aggregate_per_pane() {
    for (case, states) in [
        (
            "sib_idle_busy",
            vec![Some(TurnEdge::Idle), Some(TurnEdge::Busy)],
        ),
        (
            "sib_busy_idle",
            vec![Some(TurnEdge::Busy), Some(TurnEdge::Idle)],
        ),
        ("sib_unknown_idle", vec![None, Some(TurnEdge::Idle)]),
        ("sib_idle_unknown", vec![Some(TurnEdge::Idle), None]),
    ] {
        let Some(rig) = WakeRig::new(case, "claude", IDLE, "manual mode on") else {
            return;
        };
        rig.bindings(&states);
        // Not stale: busy for under a minute.
        rig.clock.set(1_000);
        refused(rig.attempt());
        assert_eq!(rig.typed(), "", "{case}");
    }
    let Some(rig) = WakeRig::new("sib_both_idle", "claude", IDLE, "manual mode on") else {
        return;
    };
    let keys = rig.bindings(&[Some(TurnEdge::Idle), Some(TurnEdge::Idle)]);
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    wait_for(|| (rig.typed() == format!("{WAKE}\n")).then_some(()));
    // Both idle again after the sibling's turn: the second wake is the retry's, typed once more.
    rig.book.record(&keys[1], TurnEdge::Busy, 0);
    refused(rig.attempt());
    assert_eq!(rig.typed(), format!("{WAKE}\n"), "exactly one wake so far");
}

// Kills: typing over a person's draft (T3), and approval screens read as wakeable.
#[test]
fn draft_present_no_typing() {
    let draft = IDLE.replacen("\n❯\n", "\n❯ half-written draft\n", 1);
    assert_ne!(draft, IDLE, "the fixture has an empty composer row to fill");
    let Some(rig) = WakeRig::new("draft_present", "claude", &draft, "half-written draft") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let detail = refused(rig.attempt());
    assert!(detail.contains("composer"), "{detail}");
    assert_eq!(rig.typed(), "");
    let Some(rig) = WakeRig::new("approval_screen", "claude", CLAUDE_APPROVAL, "") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    refused(rig.attempt());
    assert_eq!(rig.typed(), "");
}

/// An agy-shaped pane that keeps its wake in the composer until it is sent `C-u`, then shows
/// the empty composer.
fn agy_held_screen(held: &str) -> String {
    let row = format!("> {held}");
    let footer = AGY_IDLE.lines().last().unwrap().to_owned();
    let blanked = format!(
        "{}{}",
        " ".repeat("? for shortcuts".len()),
        &footer["? for shortcuts".len()..]
    );
    let held_screen = AGY_IDLE
        .replacen("\n>\n", &format!("\n{row}\n"), 1)
        .replace(&footer, &blanked);
    assert_ne!(held_screen, AGY_IDLE);
    held_screen
}

fn held_rig(case: &str, held: &str) -> Option<(WakeRig, String)> {
    let held_screen = agy_held_screen(held);
    let held_file = scratch_file(&format!("{case}-held"), &held_screen);
    let idle_file = scratch_file(&format!("{case}-idle"), AGY_IDLE);
    let tail = format!(
        "stty -echo -icanon min 1 time 0; while :; do k=$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' '); [ \"$k\" = 15 ] && {{ printf '\\033c'; cat {}; }}; done",
        idle_file.display()
    );
    let _ = held_file.clone();
    // The held screen is what the pane shows first.
    let rig = WakeRig::with_tail(
        case,
        "agy",
        &held_screen,
        held,
        scratch_file(&format!("{case}-typed"), ""),
        &tail,
    )?;
    Some((rig, held_screen))
}

// Kills: a wake left unsubmitted in the composer that is never read back (HoldingPrompt) or never
// cleared (it would block every later wake); and a clear that deletes a person's text (the
// composer holds more than the wake) or sends a key when the harness does not opt in.
#[test]
fn readback_mismatch_clears_no_submit() {
    let Some((rig, _)) = held_rig("readback_held", WAKE) else {
        return;
    };
    let (safe, context) = rig.target_and_context();
    assert_eq!(
        rig.host.pane_agent_state(&safe, &context).unwrap(),
        AgentComposerState::HoldingPrompt,
        "the wake is held in the composer"
    );
    assert!(rig.host.clear_own_wake_text(&safe, WAKE, &context).unwrap());
    assert_eq!(
        rig.host.pane_agent_state(&safe, &context).unwrap(),
        AgentComposerState::Submitted,
        "the composer is empty after the clear"
    );
    let Some((rig, _)) = held_rig("readback_draft", &format!("{WAKE} and more")) else {
        return;
    };
    let (safe, context) = rig.target_and_context();
    assert!(!rig.host.clear_own_wake_text(&safe, WAKE, &context).unwrap());
    assert!(
        rig.capture().contains("and more"),
        "the draft was left alone"
    );
    // Claude does not opt in: even an exact wake is left in place.
    let Some(rig) = WakeRig::new("readback_claude", "claude", IDLE, "manual mode on") else {
        return;
    };
    let (safe, context) = rig.target_and_context();
    assert!(!rig.host.clear_own_wake_text(&safe, WAKE, &context).unwrap());
    assert_eq!(rig.typed(), "");
}

// T5 (recorded): tmux consumes an OSC 7501 sequence a program writes. It reaches neither the
// screen nor any pane variable, and passthrough to an outer terminal is off by default, so a
// tmux seat has no self-reported status: the hook turn state plus the screen recognizer decide.
#[test]
fn osc_7501_passthrough_recorded() {
    let Some(tmux) = PrivateTmux::start("osc_7501_passthrough_recorded") else {
        return;
    };
    let pane = tmux.new_pane("printf '\\033]7501;status=idle\\007visible-after'; sleep 600");
    let screen = wait_for(|| {
        let out = tmux.run(&["capture-pane", "-p", "-t", &pane.id]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.contains("visible-after").then_some(text)
    });
    assert!(
        !screen.contains("7501") && !screen.contains("status=idle"),
        "{screen:?}"
    );
    let vars = tmux.run(&[
        "display",
        "-p",
        "-t",
        &pane.id,
        "#{pane_title}|#{pane_current_command}|#{window_name}",
    ]);
    assert!(
        !String::from_utf8_lossy(&vars.stdout).contains("status=idle"),
        "the payload reached a pane variable"
    );
    let passthrough = tmux.run(&["show-options", "-gv", "allow-passthrough"]);
    assert_eq!(String::from_utf8_lossy(&passthrough.stdout).trim(), "off");
}

// T1 (recorded): a screen replayed into a tmux pane and read back with `capture-pane` gives the
// same composer read, blocked and running answers as the Herdr detection text the harness
// policies were captured against.
#[test]
fn capture_pane_vs_herdr_detection_fixtures() {
    let registry = crate::harness::registry::builtins();
    for (harness, name, screen, marker) in [
        ("claude", "claude-idle", IDLE.to_owned(), "manual mode on"),
        ("claude", "claude-approval", CLAUDE_APPROVAL.to_owned(), ""),
        (
            "claude",
            "claude-draft",
            IDLE.replacen("\n❯\n", "\n❯ half-written draft\n", 1),
            "half-written draft",
        ),
        ("agy", "agy-idle", AGY_IDLE.to_owned(), "for shortcuts"),
    ] {
        let Some(rig) = WakeRig::new(&format!("t1_{name}"), harness, &screen, marker) else {
            return;
        };
        let policy = registry
            .by_id(registry.agent(harness).unwrap())
            .unwrap()
            .composer_policy()
            .unwrap();
        let (width, _) = screen_size(&screen);
        let captured = rig.capture();
        let width = Some(u16::try_from(width).unwrap());
        assert_eq!(
            policy.read(&captured, width),
            policy.read(&screen, width),
            "{name} composer"
        );
        assert_eq!(
            policy.screen_blocked(&captured),
            policy.screen_blocked(&screen),
            "{name} blocked"
        );
        assert_eq!(
            policy.screen_turn_running(&captured),
            policy.screen_turn_running(&screen),
            "{name} running"
        );
    }
}

// ---- the guards of the wake path, each decided alone ----

impl WakeRig {
    /// One wake attempt with the target and context changed by `mutate` first.
    fn attempt_with(
        &self,
        mutate: impl FnOnce(&mut SafeWakeTarget, &mut HostCallContext),
    ) -> Result<Option<PromptOutcome>, ApiError> {
        let (mut safe, mut context) = self.target_and_context();
        mutate(&mut safe, &mut context);
        self.host
            .submit_prompt_if_owed(&safe, WAKE, &context, &|| Ok(true))
    }

    /// Types a marker line after a refused attempt and returns everything the pane received.
    /// The pane reads in order, so a wake the refused attempt wrongly typed shows before it.
    fn typed_after_marker(&self) -> String {
        for keys in [&["-l", "Z"][..], &["Enter"][..]] {
            let mut args = vec!["send-keys", "-t", &self.pane.id];
            args.extend_from_slice(keys);
            assert!(self.tmux.run(&args).status.success(), "send-keys");
        }
        wait_for(|| self.typed().ends_with("Z\n").then_some(()));
        self.typed()
    }

    /// The hex of every key a `keylog_rig` pane received, in order.
    fn keys(&self) -> Vec<String> {
        self.typed().split_whitespace().map(str::to_owned).collect()
    }

    /// Sends one `Z` key after a refused clear and returns the keys the pane received.
    fn keys_after_marker(&self) -> Vec<String> {
        assert!(
            self.tmux
                .run(&["send-keys", "-t", &self.pane.id, "-l", "Z"])
                .status
                .success()
        );
        wait_for(|| (self.keys().last().map(String::as_str) == Some("5a")).then_some(()));
        self.keys()
    }
}

/// A pane showing `screen` that logs the hex of every key it receives, one per line, and never
/// changes what it shows: a clear that reaches it is visible as `15`, however often it is sent.
fn keylog_rig(case: &str, harness: &'static str, screen: &str, marker: &str) -> Option<WakeRig> {
    let typed = scratch_file(&format!("{case}-keys"), "");
    let tail = format!(
        "stty -echo -icanon min 1 time 0; while :; do k=$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' '); [ -n \"$k\" ] && echo \"$k\" >> {}; done",
        typed.display()
    );
    WakeRig::with_tail(case, harness, screen, marker, typed, &tail)
}

// Kills: `type_wake` typing after `still_owed` answers no (the send was settled while the wake
// was being prepared): the text and Enter go out anyway.
#[test]
fn still_owed_false_types_nothing() {
    let Some(rig) = WakeRig::new("still_owed_false", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let (safe, context) = rig.target_and_context();
    assert_eq!(
        rig.host
            .submit_prompt_if_owed(&safe, WAKE, &context, &|| Ok(false))
            .unwrap(),
        None
    );
    assert_eq!(
        rig.typed_after_marker(),
        "Z\n",
        "nothing typed before the marker"
    );
    assert!(!rig.capture().contains(WAKE), "the composer is untouched");
    // Control: the same attempt with the wake still owed goes out.
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    wait_for(|| (rig.typed() == format!("Z\n{WAKE}\n")).then_some(()));
}

// Kills: the wake gate's `f.server_pid == hint.server_pid` filter removed. The held hint names
// another server than the one the pane answers from; everything else about the binding holds.
#[test]
fn server_pid_recheck_at_wake_refuses() {
    let Some(rig) = WakeRig::new("server_pid_recheck", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let rewrite = |delta: i64| {
        let mut map = rig.host.tmux.lock().unwrap();
        for held in map.get_mut("seat-wake").unwrap() {
            held.hint.server_pid = u32::try_from(i64::from(held.hint.server_pid) + delta).unwrap();
        }
    };
    rewrite(1);
    let detail = refused(rig.attempt());
    assert!(detail.contains("pane unreadable"), "{detail}");
    assert_eq!(rig.typed_after_marker(), "Z\n", "nothing typed");
    rewrite(-1);
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    wait_for(|| (rig.typed() == format!("Z\n{WAKE}\n")).then_some(()));
}

// Kills: the wake gate's `basis != CooperativeAgent` refusal removed.
#[test]
fn wake_gate_refuses_a_non_cooperative_basis() {
    let Some(rig) = WakeRig::new("gate_basis", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let detail = refused(rig.attempt_with(|safe, _| {
        safe.basis = WakeTargetBasis::VerifiedOccupant {
            session: crate::protocol::ids::NativeSessionId::new("s1"),
            execution: crate::protocol::ids::ExecutionId::new("e1"),
        };
    }));
    assert!(
        detail.contains("native execution is unverified"),
        "{detail}"
    );
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: the wake gate's `expected_boot` comparison removed; the epochs and basis all hold.
#[test]
fn wake_gate_refuses_a_stale_boot() {
    let Some(rig) = WakeRig::new("gate_boot", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let detail = refused(rig.attempt_with(|_, context| {
        context.expected_boot = Some(HostBootId::new("another-boot"));
    }));
    assert!(detail.contains("host context changed"), "{detail}");
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: the wake gate's `expected_epoch != target.epoch` comparison removed; the boot and the
// host's own epoch match the target.
#[test]
fn wake_gate_refuses_a_stale_context_epoch() {
    let Some(rig) = WakeRig::new("gate_context_epoch", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let detail = refused(rig.attempt_with(|safe, context| {
        context.expected_epoch = Some(safe.epoch + 1);
    }));
    assert!(detail.contains("host context changed"), "{detail}");
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: the wake gate's comparison of the host's current epoch with the target's removed; the
// context and the target agree with each other (an old observation), the host has moved on.
#[test]
fn wake_gate_refuses_a_stale_host_epoch() {
    let Some(rig) = WakeRig::new("gate_host_epoch", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let detail = refused(rig.attempt_with(|safe, context| {
        safe.epoch += 1;
        context.expected_epoch = Some(safe.epoch);
    }));
    assert!(detail.contains("host context changed"), "{detail}");
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: the wake gate's cancellation check removed (a cancelled call types, or is reported as
// anything but `Cancelled`).
#[test]
fn wake_gate_refuses_a_cancelled_call() {
    let Some(rig) = WakeRig::new("gate_cancelled", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let error = rig
        .attempt_with(|_, context| context.budget.cancellation.cancel())
        .expect_err("a cancelled call is refused");
    assert_eq!(error.code, ErrorCode::Cancelled, "{error:?}");
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: the wake gate's composer-policy lookup treating an unknown bound harness as wakeable.
#[test]
fn wake_gate_refuses_a_harness_without_a_composer_policy() {
    let Some(rig) = WakeRig::new("gate_policy", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    for bound in [None, Some("no-such-harness".to_owned())] {
        let detail = refused(rig.attempt_with(|safe, _| safe.bound_harness = bound.clone()));
        assert!(detail.contains("no composer policy"), "{bound:?}: {detail}");
    }
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: the gate's budget checks removed. The cancellation is not set; only the deadline has
// passed (the floor refuses before any read).
#[test]
fn type_wake_with_an_exhausted_budget_types_nothing() {
    let Some(rig) = WakeRig::new("budget_exhausted", "claude", IDLE, "manual mode on") else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    // Control: with the clock before the deadline the wake goes out.
    assert_eq!(
        rig.attempt_with(|_, context| context.budget.deadline = MonoInstant(1_000))
            .unwrap(),
        Some(PromptOutcome::Submitted)
    );
    wait_for(|| (rig.typed() == format!("{WAKE}\n")).then_some(()));
    rig.clock.set(1_100);
    let error = rig
        .attempt_with(|_, context| {
            context.budget.deadline = MonoInstant(1_000);
        })
        .expect_err("an exhausted budget is refused");
    assert_eq!(error.code, ErrorCode::DeadlineExceeded, "{error:?}");
    assert_eq!(
        rig.typed_after_marker(),
        format!("{WAKE}\nZ\n"),
        "only the first wake was typed"
    );
}

/// `clear_own_wake_text` on an agy pane that holds exactly the wake, after `mutate` changed the
/// target or context. Control first: the unchanged call presses `C-u` (twice, the screen never
/// changes), so the key log works. Then the mutated call must press nothing and return `false`.
fn assert_clear_refused(
    case: &str,
    mutate: impl FnOnce(&mut SafeWakeTarget, &mut HostCallContext),
) {
    let screen = agy_held_screen(WAKE);
    let Some(rig) = keylog_rig(case, "agy", &screen, WAKE) else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let (safe, context) = rig.target_and_context();
    assert!(!rig.host.clear_own_wake_text(&safe, WAKE, &context).unwrap());
    wait_for(|| (rig.keys() == ["15", "15"]).then_some(()));
    let (mut safe, mut context) = rig.target_and_context();
    mutate(&mut safe, &mut context);
    assert!(!rig.host.clear_own_wake_text(&safe, WAKE, &context).unwrap());
    assert_eq!(
        rig.keys_after_marker(),
        ["15", "15", "5a"],
        "{case}: no key reached the pane"
    );
}

// Kills: `clear_own_wake_text` without its basis check; boot, epochs and the held text all hold.
#[test]
fn clear_own_wake_text_refuses_other_basis() {
    assert_clear_refused("clear_basis", |safe, _| {
        safe.basis = WakeTargetBasis::VerifiedOccupant {
            session: crate::protocol::ids::NativeSessionId::new("s1"),
            execution: crate::protocol::ids::ExecutionId::new("e1"),
        };
    });
}

// Kills: `clear_own_wake_text` without its `expected_boot` check.
#[test]
fn clear_own_wake_text_refuses_stale_boot() {
    assert_clear_refused("clear_boot", |_, context| {
        context.expected_boot = Some(HostBootId::new("another-boot"));
    });
}

// Kills: `clear_own_wake_text` without its `expected_epoch` check.
#[test]
fn clear_own_wake_text_refuses_stale_epoch() {
    assert_clear_refused("clear_epoch", |safe, context| {
        context.expected_epoch = Some(safe.epoch + 1);
    });
}

// Kills: `clear_own_wake_text` without its comparison of the host's current epoch.
#[test]
fn clear_own_wake_text_refuses_a_stale_host_epoch() {
    assert_clear_refused("clear_host_epoch", |safe, context| {
        safe.epoch += 1;
        context.expected_epoch = Some(safe.epoch);
    });
}

// Kills: `clear_own_wake_text` pressing for a target none of whose bindings is live.
#[test]
fn clear_own_wake_text_refuses_a_target_without_a_live_binding() {
    assert_clear_refused("clear_unbound", |safe, _| {
        safe.target = target("nobody");
    });
}

// Kills: the opt-in check of `clear_own_wake_text` removed. Codex's composer holds exactly the
// wake text (its policy reads it as `Text`, so a clear would press), boot, epoch and basis hold,
// and the harness does not opt in: no key is pressed.
#[test]
fn clear_own_wake_text_leaves_a_harness_that_does_not_opt_in() {
    const CODEX_IDLE: &str = include_str!("../fixtures/screen_turn_running/codex/idle.txt");
    let screen = CODEX_IDLE.replacen("› Ask Codex to do anything", &format!("› {WAKE}"), 1);
    assert_ne!(
        screen, CODEX_IDLE,
        "the fixture has an empty composer row to fill"
    );
    let registry = crate::harness::registry::builtins();
    let policy = registry
        .by_id(registry.agent("codex").unwrap())
        .unwrap()
        .composer_policy()
        .unwrap();
    let width = u16::try_from(screen_size(&screen).0).unwrap();
    assert_eq!(
        policy.read(&screen, Some(width)),
        ComposerRead::Text(WAKE.into())
    );
    assert!(!policy.clears_unsubmitted_wake() && policy.clear_key() == "ctrl+u");
    let Some(rig) = keylog_rig("clear_no_opt_in", "codex", &screen, WAKE) else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let (safe, context) = rig.target_and_context();
    assert!(!rig.host.clear_own_wake_text(&safe, WAKE, &context).unwrap());
    assert_eq!(rig.keys_after_marker(), ["5a"], "no key reached the pane");
}

// ---- the focused-composer hold (A4 Wake): a focused pane needs a minute of empty samples ----

/// A control-mode client attached to the private server, stdin held open until it is dropped: it
/// raises `#{session_attached}` without a pty. Returns once the session counts `clients`.
fn attach(tmux: &PrivateTmux, clients: u32) -> crate::test_support::spawn::OwnedChild {
    use crate::test_support::spawn::SpawnOwned;
    let mut command = crate::test_support::spawn::command("tmux");
    command
        .arg("-S")
        .arg(tmux.socket_path())
        .args(["-C", "attach"])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let client = command.spawn_owned().expect("spawn tmux -C attach");
    wait_for(|| {
        let out = tmux.run(&["display", "-p", "#{session_attached}"]);
        let count: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        (count == clients).then_some(())
    });
    client
}

/// Makes the rig's pane the one a client looks at: its window current, a client attached.
fn focus(rig: &WakeRig) -> crate::test_support::spawn::OwnedChild {
    assert!(
        rig.tmux
            .run(&["select-window", "-t", &rig.pane.id])
            .status
            .success()
    );
    attach(&rig.tmux, 1)
}

/// A rig whose single idle binding is ready to be woken.
fn idle_rig(case: &str) -> Option<WakeRig> {
    let rig = WakeRig::new(case, "claude", IDLE, "manual mode on")?;
    rig.bindings(&[Some(TurnEdge::Idle)]);
    Some(rig)
}

// Kills: a focused pane woken at once (the hard-coded `focused: false`), a hold shorter than the
// minute, and a refusal recorded as anything but the `focused` block.
#[test]
fn focused_pane_holds_the_wake_for_one_minute() {
    let Some(rig) = idle_rig("focused_hold") else {
        return;
    };
    let _client = focus(&rig);
    for at in [0, 30_000] {
        rig.clock.set(at);
        let detail = refused(rig.attempt());
        assert!(detail.contains("focused composer"), "{at}: {detail}");
        assert_eq!(rig.host.wake_block(&rig.seat), Some(WakeBlock::Focused));
        assert_eq!(rig.typed(), "", "nothing typed at {at}");
    }
    rig.clock.set(60_000);
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    wait_for(|| (rig.typed() == format!("{WAKE}\n")).then_some(()));
}

// Kills: samples more than a minute apart counting as one continuous minute.
#[test]
fn focused_hold_restarts_after_a_sample_gap() {
    let Some(rig) = idle_rig("focused_gap") else {
        return;
    };
    let _client = focus(&rig);
    for at in [0, 60_001] {
        rig.clock.set(at);
        refused(rig.attempt());
    }
    rig.clock.set(120_001);
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
}

// Kills: the sample taken after the turn check, so an attempt refused for a busy turn keeps the
// minute running.
#[test]
fn focused_hold_restarts_on_a_refused_attempt() {
    let Some(rig) = idle_rig("focused_refusal") else {
        return;
    };
    let _client = focus(&rig);
    rig.clock.set(0);
    refused(rig.attempt());
    rig.clock.set(30_000);
    rig.bindings(&busy_edges(&[TurnEdge::Busy]));
    let detail = refused(rig.attempt());
    assert!(!detail.contains("focused composer"), "{detail}");
    rig.bindings(&[Some(TurnEdge::Idle)]);
    rig.clock.set(60_000);
    refused(rig.attempt());
    rig.clock.set(120_000);
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
}

// Kills: a focus that reads only the client count, or leaves out the window or the pane.
#[test]
fn an_attached_pane_out_of_focus_wakes_at_once() {
    // The window is not the session's current one.
    let Some(rig) = idle_rig("focus_other_window") else {
        return;
    };
    let _client = attach(&rig.tmux, 1);
    let out = rig.tmux.run(&[
        "display",
        "-p",
        "-t",
        &rig.pane.id,
        "#{window_active}#{session_attached}",
    ]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "01");
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    // The window is current but another pane of it is.
    let Some(rig) = idle_rig("focus_other_pane") else {
        return;
    };
    let _client = focus(&rig);
    let (_, rows) = screen_size(IDLE);
    let width = screen_size(IDLE).0 + 40;
    for args in [
        vec![
            "resize-window".to_owned(),
            "-t".to_owned(),
            rig.pane.id.clone(),
            "-x".to_owned(),
            (width + 21).to_string(),
            "-y".to_owned(),
            rows.to_string(),
        ],
        vec![
            "split-window".to_owned(),
            "-h".to_owned(),
            "-d".to_owned(),
            "-l".to_owned(),
            "20".to_owned(),
            "-t".to_owned(),
            rig.pane.id.clone(),
            "sleep 600".to_owned(),
        ],
    ] {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        assert!(rig.tmux.run(&args).status.success(), "{args:?}");
    }
    let panes = rig.tmux.run(&[
        "list-panes",
        "-t",
        &rig.pane.id,
        "-F",
        "#{pane_id} #{pane_active}",
    ]);
    let listing = String::from_utf8_lossy(&panes.stdout).into_owned();
    let other = listing
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(id, _)| *id != rig.pane.id)
        .map(|(id, _)| id.to_owned())
        .expect("a second pane");
    assert!(
        rig.tmux
            .run(&["select-pane", "-t", &other])
            .status
            .success()
    );
    let out = rig.tmux.run(&[
        "display",
        "-p",
        "-t",
        &rig.pane.id,
        "#{pane_active}#{window_active}#{session_attached}",
    ]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "011");
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
}

// Kills: `#{session_attached}` parsed as a 0/1 flag, which turns two clients into an unreadable
// pane that fails the gate with the wrong reason.
#[test]
fn two_attached_clients_still_read_as_focused() {
    let Some(rig) = idle_rig("focused_two_clients") else {
        return;
    };
    let _first = focus(&rig);
    let _second = attach(&rig.tmux, 2);
    let out = rig.tmux.run(&["display", "-p", "#{session_attached}"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "2");
    let detail = refused(rig.attempt());
    assert!(detail.contains("focused composer"), "{detail}");
    assert_eq!(rig.host.wake_block(&rig.seat), Some(WakeBlock::Focused));
}

// Kills: a hard-coded `focused` in the observation, and a focus reported for a seat with no
// verified binding.
#[test]
fn observation_reports_the_panes_focus() {
    let Some(rig) = idle_rig("focused_observation") else {
        return;
    };
    let observe = |name: &str| {
        rig.host
            .observe_current_target(&target(name), &context())
            .unwrap()
            .focused
    };
    assert!(!observe("seat-wake"));
    let _client = focus(&rig);
    assert!(observe("seat-wake"));
    assert!(!observe("no-binding"));
}

// ---- the gate matches native's pre-send path (ht-kqrt.214) ----

/// A rig over a captured Claude screen, ready to be woken. The marker is the status row's text:
/// the default (the last row) carries escapes.
fn captured_rig(case: &str, screen: &str) -> Option<WakeRig> {
    let rig = WakeRig::new(case, "claude", screen, "auto mode on")?;
    rig.bindings(&[Some(TurnEdge::Idle)]);
    Some(rig)
}

// Kills: a composer read from `capture-pane -p` alone. A default hooks-only Claude seat shows a
// dim prompt suggestion after a turn; the plain text cannot tell it from a draft, so every wake
// was refused with `composer is nonempty`.
#[test]
fn wake_types_over_a_dim_prompt_suggestion() {
    let Some(rig) = captured_rig("suggestion", SUGGESTION_ANSI) else {
        return;
    };
    assert_eq!(rig.attempt().unwrap(), Some(PromptOutcome::Submitted));
    wait_for(|| (rig.typed() == format!("{WAKE}\n")).then_some(()));
}

// Kills: a styled read that vouches for any composer (dim and unstyled text told apart wrongly).
#[test]
fn styled_read_keeps_refusing_a_typed_draft() {
    let Some(rig) = captured_rig("styled_draft", DRAFT_ANSI) else {
        return;
    };
    let detail = refused(rig.attempt());
    assert!(detail.contains("composer is nonempty"), "{detail}");
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: a styled capture without SGR (or one that fails) replacing the detection read: the
// plain screen stays a refusal, as in `native`.
#[test]
fn a_screen_with_no_styles_keeps_the_detection_refusal() {
    let Some(rig) = captured_rig("suggestion_plain", SUGGESTION_PLAIN) else {
        return;
    };
    let detail = refused(rig.attempt());
    assert!(detail.contains("composer is nonempty"), "{detail}");
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: the read-back and the clear loop using the plain read: a suggestion would read as text.
#[test]
fn the_read_back_sees_the_styled_composer() {
    let Some(rig) = captured_rig("suggestion_readback", SUGGESTION_ANSI) else {
        return;
    };
    let (safe, context) = rig.target_and_context();
    assert_eq!(
        rig.host.pane_agent_state(&safe, &context).unwrap(),
        AgentComposerState::Submitted
    );
}

// Kills: a non-empty read-back leaving the focused window running (native resets it on every
// read path). Sampled empty at 0, a draft appears and is read back, the draft goes: without the
// reset the minute passes and the gate at 60 s types.
#[test]
fn a_nonempty_read_back_restarts_the_focused_window() {
    let typed = scratch_file("window_reset-typed", "");
    let draft = scratch_file("window_reset-draft", DRAFT_ANSI);
    let suggestion = scratch_file("window_reset-suggestion", SUGGESTION_ANSI);
    let clear = "printf '\\033[2J\\033[H'";
    let tail = format!(
        "stty -echo; read a; {clear}; cat {}; read b; {clear}; cat {}; exec cat > {}",
        draft.display(),
        suggestion.display(),
        typed.display()
    );
    let Some(rig) = WakeRig::with_tail(
        "window_reset",
        "claude",
        SUGGESTION_ANSI,
        "auto mode on",
        typed,
        &tail,
    ) else {
        return;
    };
    rig.bindings(&[Some(TurnEdge::Idle)]);
    let _client = focus(&rig);
    let enter = || {
        assert!(
            rig.tmux
                .run(&["send-keys", "-t", &rig.pane.id, "Enter"])
                .status
                .success()
        );
    };
    rig.clock.set(0);
    let detail = refused(rig.attempt());
    assert!(detail.contains("focused composer"), "{detail}");
    enter();
    wait_for(|| rig.capture().contains("hello draft").then_some(()));
    let (safe, context) = rig.target_and_context();
    assert_eq!(
        rig.host.pane_agent_state(&safe, &context).unwrap(),
        AgentComposerState::HoldingPrompt
    );
    enter();
    wait_for(|| {
        let screen = rig.capture();
        (!screen.contains("hello draft") && screen.contains("auto mode on")).then_some(())
    });
    rig.clock.set(60_000);
    let detail = refused(rig.attempt());
    assert!(detail.contains("focused composer"), "{detail}");
    assert_eq!(rig.typed(), "");
}

// Kills: no cancellation check after the gate's reads: `is_exhausted` alone reports a
// cancellation that lands during the gate as `DeadlineExceeded`, or types.
#[test]
fn cancel_during_the_gate_refuses_before_typing() {
    let Some(rig) = captured_rig("gate_cancel_mid", SUGGESTION_ANSI) else {
        return;
    };
    let (safe, context) = rig.target_and_context();
    let token = context.budget.cancellation.clone();
    *rig.seats.1.lock().unwrap() = Some(Box::new(move || token.cancel()));
    let error = rig
        .host
        .submit_prompt_if_owed(&safe, WAKE, &context, &|| Ok(true))
        .expect_err("a call cancelled mid-gate is refused");
    assert_eq!(error.code, ErrorCode::Cancelled, "{error:?}");
    *rig.seats.1.lock().unwrap() = None;
    assert_eq!(rig.typed_after_marker(), "Z\n");
}

// Kills: no budget floor (native refuses `left <= 2 * MIN_PROMPT_MILLIS` before any read): a
// gate that cannot finish must not start, and must leave the seat's wake block alone.
#[test]
fn a_budget_under_the_floor_refuses_before_any_read() {
    let Some(rig) = captured_rig("gate_floor", SUGGESTION_ANSI) else {
        return;
    };
    assert_eq!(rig.host.wake_block(&rig.seat), None);
    let error = rig
        .attempt_with(|_, context| context.budget.deadline = MonoInstant(400))
        .expect_err("a budget under the floor is refused");
    assert_eq!(error.code, ErrorCode::DeadlineExceeded, "{error:?}");
    assert_eq!(rig.host.wake_block(&rig.seat), None);
    assert_eq!(rig.typed_after_marker(), "Z\n");
}
