//! `HcHost`, `HcBoot` and the label vocabulary against `fake_hc`.

use super::labels::{HcAddress, TerminalId};
use super::*;
use crate::protocol::time::{Cancellation, MonoInstant};
use crate::test_support::fake_hc::{FakeHc, FakeTerminal};

fn hello(core_id: &str) -> HelloReply {
    HelloReply {
        core: wire::CoreInfo {
            id: core_id.into(),
            name: String::new(),
        },
        ..HelloReply::default()
    }
}

fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(10_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    }
}

fn host_with_t1() -> (FakeHc, HcHost) {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let host = HcHost::new(HcClient::new(fake.socket()));
    (fake, host)
}

#[test]
fn hc_boot_from_hello() {
    let boot = HcBoot::from_hello(&hello("core-0001")).unwrap();
    assert_eq!(boot.id().as_str(), "hc-core:core-0001");
    // 8-byte prefix: a 120-byte id fits exactly, 121 does not.
    let fits = "a".repeat(128 - BOOT_PREFIX.len());
    assert_eq!(
        HcBoot::from_hello(&hello(&fits))
            .unwrap()
            .id()
            .as_str()
            .len(),
        128
    );
    assert!(HcBoot::from_hello(&hello(&format!("{fits}a"))).is_none());
    assert!(HcBoot::from_hello(&hello("cöre")).is_none());
    assert!(HcBoot::from_hello(&hello("")).is_none());
    assert!(HcBoot::from_hello(&hello("a b")).is_none());
}

#[test]
fn hc_in_implemented_set() {
    assert!(HostKind::Hc.is_implemented());
    assert_eq!(HostKind::from_stored("hc"), HostKind::Hc);
    assert_eq!(HostKind::Hc.stored_name(), Some("hc"));
    let (_fake, host) = host_with_t1();
    assert_eq!(host.kind(), HostKind::Hc);
}

#[test]
fn target_root_process_is_terminal_get_pid() {
    let (fake, host) = host_with_t1();
    let target = HostTargetId::new("t1");
    assert_eq!(
        host.target_root_process(&target, &context()).unwrap(),
        Some(4242)
    );
    fake.set_pid("t1", Some(777));
    assert_eq!(
        host.target_root_process(&target, &context()).unwrap(),
        Some(777)
    );
    fake.set_pid("t1", None);
    assert_eq!(host.target_root_process(&target, &context()).unwrap(), None);
    assert!(
        host.target_root_process(&HostTargetId::new("t99"), &context())
            .is_err()
    );
    let methods: Vec<String> = fake.calls().into_iter().map(|c| c.method).collect();
    assert!(methods.iter().all(|m| m == "terminal.get"), "{methods:?}");
}

#[test]
fn target_root_process_works_inside_a_runtime() {
    let (_fake, host) = host_with_t1();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let pid = rt.block_on(async { host.target_root_process(&HostTargetId::new("t1"), &context()) });
    assert_eq!(pid.unwrap(), Some(4242));
}

#[test]
fn answers_wake_none_closing_no_evidence() {
    let (_fake, host) = host_with_t1();
    assert_eq!(host.wake_capability(), WakeCapability::None);
    assert_eq!(host.closure_authority(), ClosureAuthority::Closing);
    assert_eq!(host.archival_evidence(), ArchivalEvidence::NoEvidence);
    assert_eq!(
        host.native_launch_capability(),
        NativeLaunchCapability::Unsupported
    );
}

#[test]
fn observation_sequence_is_its_own_counter() {
    let (_fake, a) = host_with_t1();
    let (_fake2, b) = host_with_t1();
    assert_eq!(a.next_observation_sequence(), 1);
    assert_eq!(a.next_observation_sequence(), 2);
    assert_eq!(b.next_observation_sequence(), 1);
}

#[test]
fn enumerate_targets_failed_hello_closes_nothing() {
    let dead = HcHost::new(HcClient::new("/nonexistent/hc.sock"));
    let snapshot = dead.enumerate_targets(&context()).unwrap();
    assert_eq!(
        snapshot.enumeration,
        EnumerationEvidence::CompleteUnverified
    );
    assert!(snapshot.targets.is_empty());
    assert!(!snapshot.authorizes_absence_closure());
    // A live core with a terminal enumerates it (the capture rule is
    // tests/host/hc_closure.rs): one connection, no `terminal.get`.
    let (fake, host) = host_with_t1();
    let snapshot = host.enumerate_targets(&context()).unwrap();
    assert_eq!(snapshot.enumeration, EnumerationEvidence::CoherentVerified);
    assert_eq!(snapshot.targets.len(), 1);
    assert_eq!(snapshot.boot.as_str(), "hc-core:core-0001");
    assert!(fake.calls_to("terminal.get").is_empty());
    assert_eq!(fake.calls_to("events.subscribe").len(), 1);
}

#[test]
fn env_vars_are_the_scrub_set() {
    assert_eq!(
        ENV_VARS,
        [
            "HC_TERMINAL_ID",
            "HC_TERMINAL_SOCKET",
            "HC_SCOPE",
            "HC_SESSION"
        ]
    );
    let list = crate::host::child_env_scrub_list();
    assert!(ENV_VARS.iter().all(|name| list.contains(name)));
}

#[test]
fn address_format() {
    let t = TerminalId::parse("t12").unwrap();
    let a = HcAddress::new("main", t);
    assert_eq!(a.to_string(), "main:t12");
    assert_eq!(HcAddress::parse("main:t12"), Some(a));
    let bare = HcAddress::parse("t3").unwrap();
    assert_eq!(bare.session, None);
    assert_eq!(bare.to_string(), "t3");
    for bad in [
        "",
        "t",
        "tx",
        "t1x",
        ":t1",
        "a:b:t1",
        "main:",
        "main:1",
        "machine/t1",
    ] {
        assert_eq!(HcAddress::parse(bad), None, "{bad}");
    }
}
