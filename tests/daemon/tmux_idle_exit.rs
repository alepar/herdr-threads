//! N2-idle-exit: a wake-armed named binding (tmux hint verified at bind, live process) keeps the
//! daemon up beyond the 10 min idle window, and the daemon exits once none lives. The
//! consumer audit (`consumer_audit.rs`, ht-kqrt.6) stays the dialled-host half.

use super::consumer_audit::{AuditHost, Rig};
use crate::{
    daemon::idle_exit::host_inputs_active,
    harness::adapter::TurnEdge,
    host::named::{
        NamedHost,
        tmux_tests::{GenSeats, ManualClock, WakeRig},
    },
    ports::{HostId, HostPort, PromptOutcome},
    test_support::tmux::PrivateTmux,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn wait_until(mut probe: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if probe() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

const IDLE: &str = include_str!("../fixtures/screen_blocked/claude/idle.txt");

// Kills: an idle exit that ignores a wake-armed binding (the hint would be lost on the routine
// 10 min exit), a hint that expires with time (a p0 after 10 min must still type), a binding that
// outlives its process, and an unhinted or unverified binding that keeps the daemon up.
#[test]
fn wake_armed_binding_keeps_the_daemon_up_and_ends_with_its_process() {
    let Some(wake) = WakeRig::new("tmux_keep_alive", "claude", IDLE, "manual mode on") else {
        return;
    };
    wake.bindings(&[Some(TurnEdge::Idle)]);
    let rig = Rig::new("tmux-keep-alive");
    rig.builtin_port(wake.host.clone());
    assert!(
        host_inputs_active(&rig.sessions),
        "a wake-armed binding keeps it up"
    );
    // 11 minutes after the last hook: the wake still goes out.
    wake.clock.set(11 * 60_000);
    assert!(host_inputs_active(&rig.sessions));
    assert_eq!(wake.attempt().unwrap(), Some(PromptOutcome::Submitted));
    // The binding's process exits: it is pruned and the daemon may idle-exit.
    wake.tmux.run(&["kill-pane", "-t", &wake.pane.id]);
    assert!(
        wait_until(|| !host_inputs_active(&rig.sessions)),
        "a dead binding is pruned"
    );
    assert_eq!(rig.sessions.wake_armed_bindings(), 0);
}

fn named(seats: &Arc<GenSeats>) -> Arc<NamedHost> {
    Arc::new(NamedHost::new(
        "0123456789abcdef0123456789abcdef",
        HostId::from_row(9),
        seats.clone(),
        Arc::new(ManualClock::default()),
    ))
}

// Kills: any cell of dialled x named where the daemon's stay-up answer is not exactly "a dialled
// host holds a resolved seat, or a live wake-armed binding exists". A dialled host without a
// resolved local seat (a seatless core) is attached but does not count.
#[test]
fn combined_idle_exit_predicate_matrix() {
    let Some(tmux) = PrivateTmux::start("tmux_idle_exit_matrix") else {
        return;
    };
    let seats = Arc::new(GenSeats::default());
    let live_pane = tmux.new_pane("sleep 600");
    let dead_pane = tmux.new_pane("sleep 600");
    let unhinted = named(&seats);
    let live = named(&seats);
    assert!(live.record_tmux_hint(
        &crate::protocol::ids::HostTargetId::new("alice"),
        live_pane.pid,
        &tmux.hint(&live_pane)
    ));
    let dead = named(&seats);
    assert!(dead.record_tmux_hint(
        &crate::protocol::ids::HostTargetId::new("alice"),
        dead_pane.pid,
        &tmux.hint(&dead_pane)
    ));
    tmux.run(&["kill-pane", "-t", &dead_pane.id]);
    // A hint that fails verification (no such pane) leaves the host unhinted too.
    assert!(!unhinted.record_tmux_hint(
        &crate::protocol::ids::HostTargetId::new("alice"),
        live_pane.pid,
        "tmux:/nonexistent/sock:1:%99"
    ));
    for dialled in ["none", "seatless core", "resolved seat"] {
        for (name, host, armed) in [
            ("none", None, false),
            ("unhinted", Some(Arc::clone(&unhinted)), false),
            ("live wake-armed", Some(Arc::clone(&live)), true),
            ("dead wake-armed", Some(Arc::clone(&dead)), false),
        ] {
            let rig = Rig::new("tmux-matrix");
            match dialled {
                "resolved seat" => {
                    rig.dial("/herdr/a.sock", AuditHost::HERDR);
                }
                // Attached (a startup hint, a first claim) but holding no resolved seat.
                "seatless core" => {
                    rig.dial_seatless("/hc/core.sock", AuditHost::HC);
                }
                _ => {}
            }
            if let Some(host) = host {
                // Make sure the dead host's process is gone before it is counted.
                if name == "dead wake-armed" {
                    assert!(wait_until(|| host.wake_armed_bindings() == 0));
                }
                rig.builtin_port(host);
            }
            assert_eq!(
                host_inputs_active(&rig.sessions),
                dialled == "resolved seat" || armed,
                "dialled: {dialled}; named: {name}"
            );
        }
    }
}
