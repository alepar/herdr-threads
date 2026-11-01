//! `<session>:tN` labels, the `herdr-app` doc names and the ambiguous bare `tN`
//! refusal, against `fake_hc`.

use super::labels::{
    APP_DOC, AddressRefusal, HcAddress, HcCore, LABEL_SPACE, LABEL_TAB, TerminalId,
};
use super::*;
use crate::protocol::time::{Cancellation, MonoInstant};
use crate::test_support::fake_hc::{FakeHc, FakeTerminal};
use serde_json::json;

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

fn core(session: &str, space: &str, tab: &str, name: Option<&str>) -> (FakeHc, HcHost) {
    let fake = FakeHc::start();
    fake.state().set_core_name(session);
    let mut t = FakeTerminal::live("t1");
    t.name = name.map(str::to_owned);
    t.labels.insert(LABEL_SPACE.into(), space.into());
    t.labels.insert(LABEL_TAB.into(), tab.into());
    fake.add_terminal(t);
    let host = HcHost::new(HcClient::new(fake.socket()));
    (fake, host)
}

fn app_doc(fake: &FakeHc, version: u64, tab_name: &str) {
    fake.set_doc(
        APP_DOC,
        json!({"spaces": [{"id": "s1", "name": "Work", "folder": "/x", "future": [1],
            "tabs": [{"id": "a", "name": "first", "root": {}},
                     {"id": "b", "name": tab_name, "root": {}}]}]}),
        version,
    );
}

fn cores_of(hosts: &[(&str, &HcHost)]) -> Vec<HcCore> {
    hosts
        .iter()
        .map(|(session, _)| HcCore {
            session: (*session).into(),
            terminals: vec![TerminalId::parse("t1").unwrap()],
        })
        .collect()
}

#[test]
fn two_cores_t1_labels_never_cross() {
    let (fa, a) = core("alpha", "s1", "a", Some("agent-a"));
    let (fb, b) = core("beta", "s9", "z", None);
    app_doc(&fa, 3, "");
    let la = a.seat_labels(&budget()).unwrap();
    let lb = b.seat_labels(&budget()).unwrap();
    assert_eq!(la.len(), 1);
    assert_eq!(la[0].workspace_label.as_deref(), Some("Work"));
    assert_eq!(la[0].tab_label.as_deref(), Some("first"));
    assert_eq!(la[0].pane_label.as_deref(), Some("agent-a"));
    // beta's doc is unwritten: nothing of alpha's names reaches it.
    assert_eq!(lb[0].workspace_id, "s9");
    assert_eq!(lb[0].workspace_label, None);
    assert_eq!(lb[0].tab_label, None);
    assert_eq!(lb[0].pane_label, None);
    let t1 = HostTargetId::new("t1");
    assert_eq!(a.display_address(&t1).unwrap().to_string(), "alpha:t1");
    assert_eq!(b.display_address(&t1).unwrap().to_string(), "beta:t1");
    drop((fa, fb));
}

#[test]
fn bare_t1_ambiguous_refused_with_both_candidates() {
    let (_fa, a) = core("alpha", "s1", "a", None);
    let (_fb, b) = core("beta", "s1", "a", None);
    let cores = cores_of(&[("alpha", &a), ("beta", &b)]);
    let bare = HcAddress::parse("t1").unwrap();
    let refusal = bare.resolve(&cores).unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "ambiguous terminal, use one of: alpha:t1 beta:t1"
    );
    // The qualified forms resolve, each to its own core.
    for session in ["alpha", "beta"] {
        let address = HcAddress::parse(&format!("{session}:t1")).unwrap();
        assert_eq!(address.resolve(&cores).unwrap().0, session);
    }
    assert_eq!(
        HcAddress::parse("gamma:t1").unwrap().resolve(&cores),
        Err(AddressRefusal::NoSuchTarget)
    );
}

#[test]
fn bare_tn_ok_with_one_core() {
    let (_fa, a) = core("alpha", "s1", "a", None);
    let cores = cores_of(&[("alpha", &a)]);
    let (session, terminal) = HcAddress::parse("t1").unwrap().resolve(&cores).unwrap();
    assert_eq!((session.as_str(), terminal.as_str()), ("alpha", "t1"));
    assert_eq!(
        HcAddress::parse("t2").unwrap().resolve(&cores),
        Err(AddressRefusal::NoSuchTarget)
    );
}

#[test]
fn doc_unknown_fields_tolerated() {
    let names = labels::AppNames::parse(&json!({
        "spaces": [{"id": "s1", "name": "Work", "mystery": {"x": 1},
            "tabs": [{"id": "a", "name": "first", "extra": null}, 7, {"name": "no id"}]},
            "junk", {"id": 5}],
        "unknown_top": true
    }));
    assert_eq!(names.space_name("s1"), Some("Work"));
    assert_eq!(names.tab_label("a").as_deref(), Some("first"));
    assert_eq!(labels::AppNames::parse(&json!("nope")), Default::default());
    assert_eq!(labels::AppNames::parse(&json!(null)), Default::default());
}

#[test]
fn doc_cache_invalidated_by_version() {
    let (fake, host) = core("alpha", "s1", "b", None);
    app_doc(&fake, 5, "old");
    let label = |h: &HcHost| h.seat_labels(&budget()).unwrap()[0].tab_label.clone();
    assert_eq!(label(&host).as_deref(), Some("old"));
    // Same version, new content: the cache wins (the version is the key).
    app_doc(&fake, 5, "changed-without-bump");
    assert_eq!(label(&host).as_deref(), Some("old"));
    // A bumped version re-parses.
    app_doc(&fake, 6, "new");
    assert_eq!(label(&host).as_deref(), Some("new"));
}

#[test]
fn title_never_used() {
    let fake = FakeHc::start();
    let mut t = FakeTerminal::live("t1");
    t.title = "Claude Code - secret title".into();
    t.labels.insert(LABEL_SPACE.into(), "s1".into());
    t.labels.insert(LABEL_TAB.into(), "a".into());
    fake.add_terminal(t);
    let host = HcHost::new(HcClient::new(fake.socket()));
    let labels = host.seat_labels(&budget()).unwrap();
    assert_eq!(labels[0].pane_label, None);
    let topology = host.live_topology(&budget()).unwrap();
    assert_eq!(topology.panes[0].label, None);
    let all = format!("{labels:?}{topology:?}");
    assert!(!all.contains("secret title"), "{all}");
}

#[test]
fn unnamed_tab_shows_position() {
    let (fake, host) = core("alpha", "s1", "b", None);
    app_doc(&fake, 2, "");
    let labels = host.seat_labels(&budget()).unwrap();
    assert_eq!(labels[0].tab_label.as_deref(), Some("2"));
    let topology = host.live_topology(&budget()).unwrap();
    assert_eq!(topology.tabs[0].label.as_deref(), Some("2"));
    assert_eq!(topology.spaces[0].label.as_deref(), Some("Work"));
    assert_eq!(topology.panes[0].target.as_str(), "t1");
}
