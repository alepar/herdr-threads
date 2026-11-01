use super::*;
use crate::protocol::{
    ids::HostLocator,
    time::{Cancellation, MonoInstant},
};
use std::sync::Mutex;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(5)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(7)
    }
}

#[derive(Default)]
struct FakeSource {
    rows: Mutex<Vec<(SeatId, SeatName)>>,
    asked: Mutex<Vec<HostId>>,
}

impl NamedSeatSource for FakeSource {
    fn live_named(
        &self,
        host: &HostId,
        _budget: &CallBudget,
    ) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
        self.asked.lock().unwrap().push(*host);
        Ok(self.rows.lock().unwrap().clone())
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    }
}

fn context() -> HostCallContext {
    HostCallContext {
        budget: budget(),
        expected_boot: None,
        expected_epoch: None,
    }
}

fn name(text: &str) -> SeatName {
    SeatName::parse(text).unwrap()
}

fn host_with(rows: &[(&str, &str)]) -> (NamedHost, Arc<FakeSource>) {
    let source = Arc::new(FakeSource::default());
    *source.rows.lock().unwrap() = rows
        .iter()
        .map(|(id, n)| (SeatId::new(*id), name(n)))
        .collect();
    let host = NamedHost::new(
        "0123456789abcdef0123456789abcdef",
        HostId::from_row(9),
        source.clone(),
        Arc::new(FixedClock),
    );
    (host, source)
}

// Kills: a Herdr default leaking through (Composer wake, Closing, Supported
// archival, a server pid), which would make absence close named seats.
#[test]
fn answers_are_the_named_host_policy() {
    let (host, _) = host_with(&[]);
    assert_eq!(host.kind(), HostKind::Named);
    assert_eq!(
        host.wake_capability(),
        WakeCapability::Composer(ComposerTraits::TMUX)
    );
    assert_eq!(host.closure_authority(), ClosureAuthority::NonClosing);
    assert_eq!(host.archival_evidence(), ArchivalEvidence::NoEvidence);
    assert_eq!(host.server_process(), None);
    assert_eq!(
        host.native_launch_capability(),
        NativeLaunchCapability::Unsupported
    );
}

// Kills: a boot that varies per call or a missing terminal id.
#[test]
fn observe_current_target_is_present_generation_one() {
    let (host, _) = host_with(&[]);
    let first = host
        .observe_current_target(&HostTargetId::new("reviewer"), &context())
        .unwrap();
    let second = host
        .observe_current_target(&HostTargetId::new("reviewer"), &context())
        .unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(first.terminal.as_ref().unwrap().as_str(), "named:reviewer");
    assert_eq!(
        first.host_boot.as_str(),
        "named-host:node=0123456789abcdef0123456789abcdef"
    );
    assert_eq!(first.host_boot, second.host_boot);
    assert_eq!(first.incarnation, second.incarnation);
    assert_eq!(
        first.incarnation,
        IncarnationEvidence::Verified {
            identity: "named-host:node=0123456789abcdef0123456789abcdef".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        }
    );
    assert!(second.observation_sequence > first.observation_sequence);
}

// Kills: observing a target that is not a seat name.
#[test]
fn an_unparsable_name_is_never_observed() {
    let (host, _) = host_with(&[]);
    for bad in ["-x", "a b", ""] {
        if let Ok(target) = HostTargetId::parse(bad) {
            assert!(host.observe_current_target(&target, &context()).is_err());
        }
    }
    assert!(
        host.observe_current_target(&HostTargetId::new("-x"), &context())
            .is_err()
    );
}

// Kills: an enumeration derived from names rather than the store read, or one
// that claims coherent-verified authority.
#[test]
fn enumeration_is_the_store_read_and_not_coherent_verified() {
    let (host, source) = host_with(&[("sAAA00000001", "a"), ("sAAA00000002", "b.c")]);
    let snapshot = host.enumerate_targets(&context()).unwrap();
    let targets: Vec<_> = snapshot
        .targets
        .iter()
        .map(|t| t.target.as_str().to_owned())
        .collect();
    assert_eq!(targets, ["a", "b.c"]);
    assert_eq!(
        snapshot.enumeration,
        EnumerationEvidence::CompleteUnverified
    );
    assert!(!snapshot.authorizes_absence_closure());
    assert_eq!(*source.asked.lock().unwrap(), [HostId::from_row(9)]);
}

// Kills: a pane label that is not the name (displays would need a name branch).
#[test]
fn seat_labels_pane_is_the_name() {
    let (host, _) = host_with(&[("sAAA00000001", "reviewer")]);
    let labels = host.seat_labels(&budget()).unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].pane_label.as_deref(), Some("reviewer"));
    assert_eq!(labels[0].terminal, "named:reviewer");
    assert_eq!(labels[0].workspace_label, None);
    assert_eq!(labels[0].tab_label, None);
}

// Kills: a default `Unsupported` on a method a capability answer refuses.
#[test]
fn unreachable_methods_are_not_unsupported() {
    let (host, _) = host_with(&[]);
    let error = host.live_topology(&budget()).unwrap_err();
    assert_ne!(error.code, ErrorCode::Unsupported);
    let error = host
        .observe_current_target_for_archival(&HostTargetId::new("a"), &context())
        .unwrap_err();
    assert_ne!(error.code, ErrorCode::Unsupported);
}

// Kills: the reserved locator reaching the filesystem checks (it is not on disk)
// or the canonical-path rules rejecting the leading dot component.
#[test]
fn reserved_locator_parses_without_stat() {
    let locator = HostLocator::parse(crate::protocol::ids::NAMED_HOST_LOCATOR).unwrap();
    assert!(locator.is_named_host());
    assert!(!std::path::Path::new(locator.as_str()).exists());
    assert!(!HostLocator::parse("/herdr/a.sock").unwrap().is_named_host());
    assert!(HostLocator::parse("/.herdr-threads/named-host/").is_err());
    assert!(HostLocator::parse("/.herdr-threads/named-host/x").is_ok());
}

// Kills: any drift of the M11 text: 53 bytes plus the name.
#[test]
fn m11_golden() {
    let golden = include_str!("../cli/golden/m11.txt");
    let text = handoff_into_refusal(&name("reviewer"));
    assert_eq!(format!("{text}\n"), golden);
    assert_eq!(text.len(), 53 + "reviewer".len());
    assert_eq!(handoff_into_refusal(&name("a")).len(), 54);
}
