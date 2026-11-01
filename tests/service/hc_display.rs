//! `<session>:tN` in the daemon's label reads: two fake cores both owning `t1`.

use super::*;
use crate::host::hc::{HcClient, HcHost};
use crate::identity::repair::OrdinaryIdentity;
use crate::protocol::ids::HostLocator;
use crate::protocol::results::HostPaneLabels;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::service::host_sessions::HostSession;
use crate::store::{SqliteStore, StoreSettings, connection::StoreContext};
use crate::test_support::fake_hc::{FakeHc, FakeTerminal};

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

struct Rig {
    sessions: Arc<HostSessions>,
    locators: [HostLocator; 2],
    _fakes: [FakeHc; 2],
    dir: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `alpha` and `beta`, each owning `t1`; `beta_name` names beta's terminal.
fn rig(beta_name: Option<&str>) -> Rig {
    use std::os::unix::fs::DirBuilderExt;
    let dir = crate::test_support::scratch::temp_path_canonical(format!(
        "herdr-hc-display-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(TestClock);
    let instance = crate::node::NodeUuid::new_v4().to_string();
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(dir.join("db"), Arc::clone(&clock))
                .with_test_host()
                .with_node_seed(crate::test_support::node_seed_for(&instance)),
            instance.clone(),
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let writer = Arc::new(crate::service::fair_writer::FairWriter::new(32));
    let mut built = Vec::new();
    let mut locators = Vec::new();
    let mut fakes = Vec::new();
    for (index, (session, name)) in [("alpha", None), ("beta", beta_name)]
        .into_iter()
        .enumerate()
    {
        let fake = FakeHc::start();
        fake.state().set_core_name(session);
        let mut terminal = FakeTerminal::live("t1");
        terminal.name = name.map(str::to_owned);
        fake.add_terminal(terminal);
        let port = Arc::new(HcHost::new(HcClient::new(fake.socket())));
        let identity = Arc::new(OrdinaryIdentity::new(
            instance.clone(),
            crate::test_support::host_id_for_test(index as i64 + 1),
            store.clone(),
            port,
            Arc::clone(&clock),
            Arc::clone(&writer),
        ));
        // One canonical directory per session: distinct locators.
        let sub = dir.join(session);
        std::fs::create_dir(&sub).unwrap();
        let locator = HostLocator::parse(sub.to_string_lossy().into_owned()).unwrap();
        locators.push(locator.clone());
        built.push(HostSession::prebuilt(
            locator,
            identity,
            Arc::clone(&clock),
            None,
        ));
        fakes.push(fake);
    }
    let sessions = HostSessions::prebuilt_many(built, store, clock, instance);
    let [a, b] = <[HostLocator; 2]>::try_from(locators).unwrap();
    let [fa, fb] = <[FakeHc; 2]>::try_from(fakes).ok().unwrap();
    Rig {
        sessions,
        locators: [a, b],
        _fakes: [fa, fb],
        dir,
    }
}

fn pane_of(map: &LabelMap, session_index: usize, rig: &Rig) -> String {
    let host = rig
        .sessions
        .all_for_reads()
        .into_iter()
        .map(|s| s.host_id())
        .nth(session_index)
        .unwrap();
    map[&(host, "t1".to_owned())].pane.clone()
}

#[test]
fn read_labels_shows_session_qualified_pane_per_host() {
    let rig = rig(None);
    let map = read_labels(&rig.sessions, &budget(), &Default::default());
    assert_eq!(map.len(), 2);
    let mut panes = [pane_of(&map, 0, &rig), pane_of(&map, 1, &rig)];
    panes.sort();
    assert_eq!(panes, ["alpha:t1", "beta:t1"]);
}

#[test]
fn named_hc_terminal_keeps_its_name() {
    let rig = rig(Some("worker"));
    let map = read_labels(&rig.sessions, &budget(), &Default::default());
    let mut panes = [pane_of(&map, 0, &rig), pane_of(&map, 1, &rig)];
    panes.sort();
    assert_eq!(panes, ["alpha:t1", "worker"]);
}

#[test]
fn host_topologies_rows_carry_session_and_locator() {
    let rig = rig(None);
    let rows = host_topologies(&rig.sessions, &budget());
    assert_eq!(rows.len(), 2);
    for (session, locator) in [("alpha", &rig.locators[0]), ("beta", &rig.locators[1])] {
        let row = rows
            .iter()
            .find(|r| r.session.as_deref() == Some(session))
            .unwrap();
        assert_eq!(row.locator.as_ref(), Some(locator));
        assert_eq!(row.target.as_str(), "t1");
    }
}

#[test]
fn herdr_shaped_row_serializes_without_session_keys() {
    let row = HostPaneLabels {
        host: "1".into(),
        target: crate::protocol::ids::HostTargetId::new("w1:p1"),
        terminal: "t".into(),
        incarnation: None,
        workspace_id: "w1".into(),
        workspace_label: None,
        tab_id: "tab".into(),
        tab_label: None,
        pane_label: None,
        session: None,
        locator: None,
    };
    let json = serde_json::to_string(&row).unwrap();
    assert_eq!(
        json,
        r#"{"host":"1","target":"w1:p1","terminal":"t","incarnation":null,"workspace_id":"w1","workspace_label":null,"tab_id":"tab","tab_label":null,"pane_label":null}"#
    );
}

#[test]
fn older_daemon_row_decodes() {
    let old = r#"{"host":"1","target":"w1:p1","terminal":"t","workspace_id":"w1","tab_id":"tab"}"#;
    let row: HostPaneLabels = serde_json::from_str(old).unwrap();
    assert_eq!((row.session, row.locator), (None, None));
}
