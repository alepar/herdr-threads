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
pub(crate) struct GenSeats(Mutex<Vec<u64>>);
impl NamedSeatSource for GenSeats {
    fn live_named(&self, _: &HostId, _: &CallBudget) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
        Ok(Vec::new())
    }
    fn top_level_generations(&self, _: &SeatId, _: &CallBudget) -> Result<Vec<u64>, ApiError> {
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
    let width = screen
        .lines()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(80);
    (width.max(40), (screen.lines().count() + 4).max(24))
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
                generation,
            })
            .collect();
        *self.seats.0.lock().unwrap() = keys.iter().map(|k| k.generation).collect();
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
fn held_rig(case: &str, held: &str) -> Option<(WakeRig, String)> {
    let held_screen = {
        let row = format!("> {held}");
        let footer = AGY_IDLE.lines().last().unwrap().to_owned();
        let blanked = format!(
            "{}{}",
            " ".repeat("? for shortcuts".len()),
            &footer["? for shortcuts".len()..]
        );
        AGY_IDLE
            .replacen("\n>\n", &format!("\n{row}\n"), 1)
            .replace(&footer, &blanked)
    };
    assert_ne!(held_screen, AGY_IDLE);
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
