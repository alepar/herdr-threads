//! F6: labels come from the daemon's per-host read, keyed `(host, target)`.
//! Mounted from `tests/cli/peer_locations.rs`; the daemon side is covered by
//! `tests/store/remote_seats.rs` and `tests/service/host_routing.rs`.
use super::*;
use crate::protocol::results::SeatSummary;

fn located(target: &str, labels: Option<(&str, &str, &str)>) -> ParticipantLocation {
    ParticipantLocation {
        seat: SeatId::new("s"),
        continuity: ContinuityStatus::Resolved,
        target: Some(HostTargetId::new(target)),
        terminal: Some("term".into()),
        incarnation: None,
        remote: labels.map(|(w, t, p)| entry(&(w.to_owned(), t.to_owned(), p.to_owned()))),
    }
}

fn summary(seat: &str, labels: Option<(&str, &str, &str)>) -> SeatSummary {
    SeatSummary {
        seat: SeatId::new(seat),
        continuity: ContinuityStatus::Resolved,
        target: Some(HostTargetId::new("t1")),
        generation: 1,
        created_at: UtcMillis(0),
        retired_at: None,
        active: false,
        bindings: Vec::new(),
        availability: None,
        directory: labels
            .map(|(w, t, p)| Box::new(entry(&(w.to_owned(), t.to_owned(), p.to_owned())))),
    }
}

fn seat_list(rows: Vec<SeatSummary>) -> String {
    let page = Page {
        items: rows,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 2,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    };
    crate::cli::human::render(
        &CommandResult::Seats(page),
        &crate::protocol::output::OutputSpec::default(),
    )
    .expect("seat list has a human form")
}

// Kills: labels looked up by target alone. Two hosts both own `t1`; each
// seat shows its own host's labels in `seat list` and in its participant
// location, and neither borrows the other's.
#[test]
fn two_hosts_same_target_distinct_labels() {
    let text = seat_list(vec![
        summary("sa", Some(("alpha", "tab-a", "pane-a"))),
        summary("sb", Some(("beta", "tab-b", "pane-b"))),
    ]);
    let line = |seat: &str| {
        text.lines()
            .find(|l| l.starts_with(seat))
            .unwrap()
            .to_owned()
    };
    assert!(line("sa").contains("alpha") && line("sa").contains("pane-a"));
    assert!(!line("sa").contains("beta"));
    assert!(line("sb").contains("beta") && line("sb").contains("pane-b"));
    assert!(!line("sb").contains("alpha"));
    let a = format_location(&located("t1", Some(("alpha", "tab-a", "pane-a"))));
    let b = format_location(&located("t1", Some(("beta", "tab-b", "pane-b"))));
    assert_ne!(a, b);
    assert!(a.contains("alpha") && !a.contains("beta"));
    assert!(b.contains("beta") && !b.contains("alpha"));
}

// Kills: a CLI that fails or prints stale host data against a daemon with no
// label read: every label is `-` and the commands still render.
#[test]
fn cli_without_host_topologies_prints_dash() {
    let text = seat_list(vec![summary("sa", None)]);
    let row = text.lines().find(|l| l.starts_with("sa")).unwrap();
    assert!(row.contains("t1 (unavailable)"), "{row}");
    assert_eq!(
        format_location(&located("t1", None)),
        "- / - / t1 (-) [advisory]"
    );
}

// Kills: `name` required on the wire: an older daemon's entry has none.
#[test]
fn old_seat_directory_decodes_name_none() {
    let old = serde_json::json!({"node": "n", "state": "idle", "workspace": "w"});
    let decoded: SeatDirectory = serde_json::from_value(old).unwrap();
    assert_eq!(decoded.name, None);
    assert_eq!(decoded.workspace.as_deref(), Some("w"));
    // An unset name is not sent, so an older CLI sees the same bytes as before.
    assert!(!serde_json::to_string(&decoded).unwrap().contains("name"));
}
