//! ht-kqrt.32 (SA11): the host-side no-wake tripwire. Every implemented host kind is built
//! through its test-only constructor; a kind whose host answers wake `None` makes
//! `wake=no_wake` reachable, so SKILL.md must explain it (K2). The harness half is
//! `idle_wake_classification_of_the_builtins` (src/harness/registry.rs).
//!
//! Keyed on `target_wake_capability` for a target with no verified wake path: after the tmux wake
//! path (N2) a named host answers `Composer` at host level and `None` only per target.

use herdr_threads::{
    host::{hc::HcHost, hc::rpc::HcClient, named::NamedHost, native::NativeCli},
    ports::{HostId, HostKind, HostPort, WakeCapability},
    protocol::{
        ids::{HostTargetId, SeatId, SeatName},
        time::CallBudget,
    },
    store::named_seats::NamedSeatSource,
};
use std::sync::Arc;

/// The K2 sentence SKILL.md must carry once any host can have no wake.
const K2: &str = "`no_wake`: the seat's host cannot wake it; it sees the item on its next turn.";

struct NoSeats;
impl NamedSeatSource for NoSeats {
    fn live_named(
        &self,
        _: &HostId,
        _: &CallBudget,
    ) -> Result<Vec<(SeatId, SeatName)>, herdr_threads::protocol::results::ApiError> {
        Ok(Vec::new())
    }
}

/// The host of `kind` through its test-only constructor, no process or socket started. The match
/// is exhaustive on purpose: a new kind in `IMPLEMENTED` must be added here.
fn build(kind: HostKind) -> Box<dyn HostPort> {
    let clock = Arc::new(herdr_threads::app::SystemClock::new());
    match kind {
        HostKind::Herdr => Box::new(NativeCli::new(
            "/nonexistent/herdr-no-wake-tripwire.sock".into(),
            clock,
        )),
        HostKind::Hc => Box::new(HcHost::new(HcClient::new(
            "/nonexistent/hc-no-wake-tripwire.sock",
        ))),
        HostKind::Named => Box::new(NamedHost::new(
            "00000000000000000000000000000001",
            herdr_threads::test_support::host_id_for_test(0),
            Arc::new(NoSeats),
            clock,
        )),
        HostKind::Unknown => unreachable!("Unknown is never implemented"),
    }
}

/// A target no host has a verified wake path for.
fn unverified() -> HostTargetId {
    HostTargetId::new("unverified")
}

/// Err names the kinds whose host has no wake path while `skill` does not explain `no_wake`.
fn tripwire(skill: &str) -> Result<(), String> {
    let silent: Vec<_> = HostKind::IMPLEMENTED
        .iter()
        .filter(|kind| build(**kind).target_wake_capability(&unverified()) == WakeCapability::None)
        .map(|kind| kind.stored_name().unwrap_or("?"))
        .collect();
    if silent.is_empty() || skill.contains(K2) {
        return Ok(());
    }
    Err(format!(
        "host kind(s) {silent:?} have no wake path (`wake=no_wake` is reachable); \
         integrations/skill/SKILL.md must explain it in Warnings (K2): {K2}"
    ))
}

fn skill() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/skill/SKILL.md"
    ))
    .unwrap()
}

// Kills: a host kind with no wake path (named, hc) while SKILL.md says nothing about `no_wake`.
#[test]
fn host_no_wake_tripwire_passes_with_k2_in_the_skill() {
    tripwire(&skill()).unwrap();
}

// Kills: a tripwire that passes whatever the skill says (test of the test, on a copy without K2),
// or that no longer sees any kind with a `None` answer.
#[test]
fn host_no_wake_tripwire_fails_without_k2() {
    let without = skill().replace(K2, "");
    assert!(!without.contains(K2));
    let error = tripwire(&without).unwrap_err();
    assert!(error.contains("named"), "{error}");
    assert!(error.contains("hc"), "{error}");
    assert!(!error.contains("herdr"), "Herdr hosts wake: {error}");
}

// Kills: `HostKind::wakes_dormant_target` drifting from the host's own answer for a target with no
// verified wake path: the store would then keep a dormant ladder for a host that refuses (or drop
// one a host could run).
#[test]
fn dormant_wake_matches_each_hosts_target_capability() {
    for kind in HostKind::IMPLEMENTED {
        let composer = matches!(
            build(*kind).target_wake_capability(&HostTargetId::new("w9:p9")),
            WakeCapability::Composer(_)
        );
        assert_eq!(kind.wakes_dormant_target(), composer, "{kind:?}");
    }
}
