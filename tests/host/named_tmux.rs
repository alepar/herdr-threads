//! N2-hint: tmux hint verification and the per-target wake answer of `NamedHost`, against a
//! private tmux server (`tmux -L ht-test-<uuid>`, `kill-server` on Drop). A test skips, with the
//! recorded reason, when `tmux` is not installed.
use super::*;
use crate::{
    host::process_table::HostProcessTable,
    ports::ProcessInfo,
    protocol::time::{Cancellation, MonoInstant},
    test_support::tmux::{Pane, PrivateTmux},
};
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
fn outsider() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("600")
        .spawn()
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
