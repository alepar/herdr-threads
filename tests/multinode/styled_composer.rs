//! ht-ansi-composer: the native fallback wake reads the composer with styles.
//! Through the real binary and daemon, a stand-in Claude seat whose composer
//! shows a dim prompt suggestion (captured from Claude Code 2.1.309) is woken
//! by a p0 send; a typed draft is never typed over; and a Herdr that ignores
//! `--format ansi` keeps today's refusal of any composer text.

use crate::support::{self, P1, P2, P3};

const P4: &str = "w1:p4";

macro_rules! capture {
    ($name:literal) => {
        include_str!(concat!(
            "../../docs/evidence/ansi-composer/captures/claude-2.1.309-",
            $name
        ))
    };
}

#[test]
fn styled_composer_read_decides_the_fallback_wake() {
    let cluster = support::cluster_with_panes("mn-styled", 1, &[], &[0], &[P1, P2, P3, P4]);
    let host = cluster.host(0);
    let suggestion = capture!("suggestion.read-detection.txt");
    host.set_screen(P1, suggestion, Some(capture!("suggestion.recent-ansi.txt")));
    host.set_screen(
        P2,
        capture!("draft.read-detection.txt"),
        Some(capture!("draft.recent-ansi.txt")),
    );
    host.set_screen(P3, suggestion, None);
    let suggested = cluster.seat(0, P1);
    let drafting = cluster.seat(0, P2);
    let unstyled = cluster.seat(0, P3);
    let sender = cluster.seat(0, P4);
    support::wait_seats(&cluster, 0, 4);

    for seat in [&drafting, &unstyled, &suggested] {
        support::send(
            &cluster,
            &sender,
            &["--seat", &seat.id, "--body", "styled", "--nudge"],
        );
    }
    support::wait_wakes(&cluster, 0, P1, 1);
    // Both refusals were decided on a styled read attempt, not skipped.
    cluster.wait_until("styled reads of P2 and P3", crate::WAIT, || {
        (host.reads_for(P2, "recent") >= 1 && host.reads_for(P3, "recent") >= 1).then_some(())
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(host.prompts_for(P2), 0, "a draft was typed over");
    assert_eq!(
        host.prompts_for(P3),
        0,
        "unstyled composer text woke: {:?}",
        host.prompts()
    );
    assert_eq!(host.prompts_for(P4), 0, "the sender is not woken");
}
