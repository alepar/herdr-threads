//! ht-hermes-wake: the Hermes composer reader in the native wake. Through the
//! real binary and daemon, stand-in Hermes seats whose panes show captured
//! Hermes 0.22.5 screens (`docs/evidence/hermes-composer/`) get p0 sends: the
//! empty composer (its italic placeholder) is woken; a typed draft, a busy turn
//! (Herdr still reports `idle`) and the first-run consent screen are never typed
//! into. The stand-in's binding process is the pane's foreground process; a real
//! Hermes 0.22 binding lives in its gateway, so the one-process choice refuses
//! before this reader (TRUST-POLICY).

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
