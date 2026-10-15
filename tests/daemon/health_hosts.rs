//! ht-5fr.2.10: Health per attached Herdr server. Each test names the
//! mutation it kills.

use super::*;
use crate::protocol::results::{HEALTH_ENDPOINT_MAX_BYTES, bound_endpoint_tail};

fn host(id: i64, endpoint: &str, status: ComponentStatus) -> HostHealthInput {
    HostHealthInput {
        host_id: id,
        endpoint: endpoint.into(),
        status,
        version: Some(format!("0.{id}.0")),
        current_execution: CapabilityState::Unsupported,
        coherent_enumeration: CapabilityState::Supported,
        safe_prompt: CapabilityState::Supported,
        last_reconciliation_at: Some(UtcMillis(100 + id)),
    }
}

fn hosted(hosts: Vec<HostHealthInput>) -> Health {
    let mut inputs = health_budget::ready_inputs();
    // The single-host fields must not matter once entries exist.
    inputs.host = ComponentStatus::Unknown;
    inputs.coherent_enumeration = CapabilityState::Unknown;
    inputs.hosts = hosts;
    inputs.assemble()
}

/// Kills: an aggregate that takes the first entry (or the last) instead of
/// the weakest capability and the earliest reconciliation, a missing entry,
/// and a `healthy` that ignores the second server.
#[test]
fn aggregate_over_two_hosts() {
    let a = host(1, "/s/a.sock", ComponentStatus::Ready);
    let mut b = host(2, "/s/b.sock", ComponentStatus::Ready);
    b.coherent_enumeration = CapabilityState::Unknown;
    b.last_reconciliation_at = Some(UtcMillis(50));
    let health = hosted(vec![a.clone(), b]);
    assert_eq!(health.hosts.len(), 2);
    assert_eq!(health.hosts[0].host_id, 1);
    assert_eq!(health.hosts[1].endpoint, "/s/b.sock");
    assert_eq!(health.host.reachability, ComponentState::Ready);
    assert_eq!(health.host.coherent_enumeration, CapabilityState::Unknown);
    assert_eq!(health.host.safe_prompt, CapabilityState::Supported);
    assert_eq!(health.last_reconciliation_at, Some(UtcMillis(50)));
    assert_ne!(health.state, HealthState::Healthy, "B is not coherent yet");

    let both = hosted(vec![a, host(2, "/s/b.sock", ComponentStatus::Ready)]);
    assert_eq!(both.state, HealthState::Healthy, "{:?}", both.limitations);
    assert_eq!(both.last_reconciliation_at, Some(UtcMillis(101)));
    assert!(both.validate().is_ok());
}

/// Kills: a store-only aggregate that reads ready, and a missing-time host
/// hidden by the others' times.
#[test]
fn empty_aggregate_is_unknown_and_unreconciled_host_wins_the_minimum() {
    let none = hosted(Vec::new());
    assert!(none.hosts.is_empty());
    assert_eq!(none.host.reachability, ComponentState::Unknown);
    let mut fresh = host(2, "/s/b.sock", ComponentStatus::Ready);
    fresh.last_reconciliation_at = None;
    let health = hosted(vec![host(1, "/s/a.sock", ComponentStatus::Ready), fresh]);
    assert_eq!(health.last_reconciliation_at, None);
}

/// Kills: an unreachable server that leaves Health `healthy`, a missing or
/// reworded line, and an aggregate that hides the outage.
#[test]
fn one_unreachable_host_degrades_with_line() {
    let health = hosted(vec![
        host(1, "/s/a.sock", ComponentStatus::Unavailable("gone".into())),
        host(2, "/s/b.sock", ComponentStatus::Ready),
    ]);
    assert_eq!(health.state, HealthState::Degraded);
    assert_eq!(health.host.reachability, ComponentState::Unavailable);
    assert!(
        health
            .limitations
            .contains(&"Herdr server /s/a.sock unreachable".to_owned()),
        "{:?}",
        health.limitations
    );
    assert!(
        !health
            .limitations
            .iter()
            .any(|line| line.contains("b.sock")),
        "the reachable server adds no line: {:?}",
        health.limitations
    );
    assert_eq!(health.hosts[0].reachability, ComponentState::Unavailable);
    assert_eq!(health.hosts[1].reachability, ComponentState::Ready);
}

/// Kills: one line per unreachable server past the fold threshold (it would
/// crowd the 16-line wire bound) and a fold that miscounts.
#[test]
fn many_unreachable_hosts_fold() {
    let down = |count: i64| {
        hosted(
            (1..=count)
                .map(|id| {
                    host(
                        id,
                        &format!("/s/{id}.sock"),
                        ComponentStatus::Unavailable("gone".into()),
                    )
                })
                .collect(),
        )
    };
    let at_threshold = down(LANE_LINES_BEFORE_FOLD as i64);
    assert_eq!(
        at_threshold
            .limitations
            .iter()
            .filter(|line| line.ends_with(" unreachable"))
            .count(),
        LANE_LINES_BEFORE_FOLD
    );
    let folded = down(LANE_LINES_BEFORE_FOLD as i64 + 1);
    let unreachable: Vec<_> = folded
        .limitations
        .iter()
        .filter(|line| line.contains("unreachable"))
        .collect();
    assert_eq!(unreachable, ["3 Herdr servers unreachable"]);
    let sixteen = down(16);
    assert_eq!(sixteen.hosts.len(), 16);
    assert!(
        sixteen
            .limitations
            .contains(&"16 Herdr servers unreachable".to_owned())
    );
    assert!(sixteen.validate().is_ok());
}

/// Kills: a head-keeping cut (the tail is the distinguishing part), a cut
/// that splits a character, a missing mark, and an unbounded endpoint.
#[test]
fn endpoint_truncation_keeps_tail() {
    let short = "/s/herdr.sock";
    assert_eq!(bound_endpoint_tail(short), short);
    let exact = "x".repeat(HEALTH_ENDPOINT_MAX_BYTES);
    assert_eq!(bound_endpoint_tail(&exact), exact);

    let long = format!("/{}/tail.sock", "d".repeat(300));
    let cut = bound_endpoint_tail(&long);
    assert!(cut.starts_with('…'));
    assert!(cut.ends_with("/tail.sock"));
    assert!(cut.len() <= HEALTH_ENDPOINT_MAX_BYTES, "{}", cut.len());

    // Multi-byte characters at the cut stay whole.
    let wide = format!("/{}end.sock", "é".repeat(200));
    let cut = bound_endpoint_tail(&wide);
    assert!(cut.starts_with('…') && cut.ends_with("end.sock"));
    assert!(cut.len() <= HEALTH_ENDPOINT_MAX_BYTES);

    let health = hosted(vec![host(
        1,
        &long,
        ComponentStatus::Unavailable("x".into()),
    )]);
    assert_eq!(health.hosts[0].endpoint, bound_endpoint_tail(&long));
    assert!(
        health
            .limitations
            .iter()
            .any(|line| line == &format!("Herdr server {} unreachable", health.hosts[0].endpoint))
    );
    assert!(health.validate().is_ok());
}

/// Kills: a degraded server that reads as ready, and an aggregate that takes
/// the best server's status instead of the worst: the aggregate `host` line
/// carries the worst server's detail and no per-server unreachable line
/// appears for a server that merely degraded.
#[test]
fn degraded_host_is_the_aggregate_line() {
    let health = hosted(vec![
        host(1, "/s/a.sock", ComponentStatus::Ready),
        host(
            2,
            "/s/b.sock",
            ComponentStatus::Degraded("capture partial".into()),
        ),
    ]);
    assert_eq!(health.state, HealthState::Degraded);
    assert_eq!(health.host.reachability, ComponentState::Degraded);
    assert!(
        health
            .limitations
            .contains(&"host degraded: capture partial".to_owned()),
        "{:?}",
        health.limitations
    );
    assert!(
        !health
            .limitations
            .iter()
            .any(|line| line.contains("unreachable")),
        "{:?}",
        health.limitations
    );
}
