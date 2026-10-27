//! ht-hermes-wake: the Hermes composer reader in the native wake. Through the
//! real binary and daemon, stand-in Hermes seats whose panes show captured
//! Hermes 0.22.5 screens (`docs/evidence/hermes-composer/`) get p0 sends: the
//! empty composer (its italic placeholder) is woken; a typed draft, a busy turn
//! (Herdr still reports `idle`) and the first-run consent screen are never typed
//! into. A real Hermes 0.22 binding lives in its gateway, never the pane's
//! foreground process; the composer match is pane-scoped for it (ht-f0xn,
//! TRUST-POLICY A4), which the gateway test below drives.

use crate::support::{self, P1, P2, P3};

const P4: &str = "w1:p4";
const P5: &str = "w1:p5";

macro_rules! capture {
    ($name:literal) => {
        (
            include_str!(concat!(
                "../../docs/evidence/hermes-composer/captures/hermes-0.22.5-",
                $name,
                ".read-detection.txt"
            )),
            include_str!(concat!(
                "../../docs/evidence/hermes-composer/captures/hermes-0.22.5-",
                $name,
                ".recent-ansi.txt"
            )),
        )
    };
}

#[test]
fn hermes_idle_wake_types_only_into_an_empty_composer() {
    let cluster = support::cluster_with_panes("mn-hermes", 1, &[], &[0], &[P1, P2, P3, P4, P5]);
    let host = cluster.host(0);
    let panes = [
        (P1, capture!("idle-empty")),
        (P2, capture!("draft")),
        (P3, capture!("busy")),
        (P4, capture!("telemetry-modal")),
    ];
    for (pane, (detection, styled)) in panes {
        host.set_agent(pane, "hermes");
        host.set_screen(pane, detection, Some(styled));
    }
    let seats: Vec<_> = panes
        .iter()
        .map(|(pane, _)| cluster.seat_as(0, pane, "hermes"))
        .collect();
    let sender = cluster.seat(0, P5);
    support::wait_seats(&cluster, 0, 5);

    for seat in seats.iter().rev() {
        support::send(
            &cluster,
            &sender,
            &["--seat", &seat.id, "--body", "hermes", "--nudge"],
        );
    }
    support::wait_wakes(&cluster, 0, P1, 1);
    // The draft is decided on its styled read; the busy and consent screens on detection.
    cluster.wait_until("composer reads of P2, P3 and P4", crate::WAIT, || {
        (host.reads_for(P2, "recent") >= 1
            && [P3, P4]
                .iter()
                .all(|pane| host.reads_for(pane, "detection") >= 1))
        .then_some(())
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(host.prompts_for(P2), 0, "a draft was typed over");
    assert_eq!(host.prompts_for(P3), 0, "a busy Hermes was nudged");
    assert_eq!(host.prompts_for(P4), 0, "the consent screen was typed into");
    assert_eq!(host.prompts_for(P5), 0, "the sender is not woken");
}

/// ht-f0xn: from Hermes 0.22.1 one gateway process runs the sessions of every pane, so each
/// binding's process is the gateway and each pane's foreground process is its own TUI. A p0
/// send to one pane's seat wakes that pane only (the composer match is pane-scoped for a process
/// that spans seats); the sibling pane of the same gateway is never typed into.
#[test]
fn hermes_gateway_wake_types_only_into_the_recipient_pane() {
    let cluster = support::cluster_with_panes("mn-hermes-gw", 1, &[], &[0], &[P1, P2, P3]);
    let host = cluster.host(0);
    let (detection, styled) = capture!("idle-empty");
    for pane in [P1, P2] {
        host.set_agent(pane, "hermes");
        host.set_screen(pane, detection, Some(styled));
    }
    let a = cluster.seat_in_process(0, P1, "hermes", "gateway");
    let b = cluster.seat_in_process(0, P2, "hermes", "gateway");
    let sender = cluster.seat(0, P3);
    support::wait_seats(&cluster, 0, 3);

    support::send(
        &cluster,
        &sender,
        &["--seat", &a.id, "--body", "for-a", "--nudge"],
    );
    support::wait_wakes(&cluster, 0, P1, 1);
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(host.prompts_for(P2), 0, "the sibling pane was woken for a");
    support::send(
        &cluster,
        &sender,
        &["--seat", &b.id, "--body", "for-b", "--nudge"],
    );
    support::wait_wakes(&cluster, 0, P2, 1);
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(host.prompts_for(P1), 1, "the first pane was woken again");
    assert_eq!(host.prompts_for(P2), 1, "the sibling pane was woken twice");
    assert_eq!(host.prompts_for(P3), 0, "the sender is not woken");
}
