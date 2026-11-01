//! ht-kqrt.72 (N4-inherit): a per-command seat name equal to the one the host server's own
//! environment carries is refused with M6 and never bound; any other name is honoured, and the
//! refused name binds once the server restarts without the variable.
use crate::ports::{LocalService, StorePort};
use crate::{
    ports::HostId,
    protocol::{
        commands::{Command, ResolveSeat},
        ids::*,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::{
        dispatch::{DomainService, PeerIdentity},
        fair_writer::FairWriter,
        locate::environ_reader_tests::EnvHost,
        locate::environ_reader_tests::server_child,
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::{os::unix::fs::DirBuilderExt, sync::Arc};

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const PANE: &str = "w1:p1";

struct PrivateDirectory(std::path::PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}

struct Fixture {
    service: DomainService,
    db: Connection,
    host: Arc<EnvHost>,
    _directory: PrivateDirectory,
}

fn fixture(server: Option<u32>) -> Fixture {
    let path = crate::test_support::scratch::temp_path(format!(
        "daemon-inherited-name-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let directory = PrivateDirectory(path.clone());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let context =
        StoreContext::new(path.join("store.db"), clock.clone()).with_attached_host(&locator);
    let db = context.open_writer().unwrap();
    db.execute(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1,kind='named' WHERE endpoint=?1",
        [locator.as_str()],
    )
    .unwrap();
    let named_host: i64 = db
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE kind='named'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, INSTANCE, StoreSettings::default()).unwrap());
    let host = EnvHost::new(PANE, server);
    let service = DomainService::with_host(
        INSTANCE.into(),
        HostId::from_row(named_host),
        store,
        clock,
        host.clone(),
        Arc::new(FairWriter::new(32)),
    );
    Fixture {
        service,
        db,
        host,
        _directory: directory,
    }
}

fn resolve(
    f: &Fixture,
    name: &str,
    operation: &str,
    hint: Option<&str>,
) -> Result<CommandResult, ApiError> {
    f.service.handle(
        Command::ResolveSeat(ResolveSeat {
            target: HostTargetId::new(name),
            host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
            operation: OperationId::new(operation),
            locator_hint: hint.map(str::to_owned),
        }),
        PeerIdentity::from_kernel(501),
        &budget(),
    )
}

fn seats_named(db: &Connection, name: &str) -> i64 {
    db.query_row("SELECT count(*) FROM seats WHERE name=?1", [name], |r| {
        r.get(0)
    })
    .unwrap()
}

// Kills: no check (X binds), a check that compares the wrong side (Y refused), a refusal that
// still allocates the seat, a refusal that outlives the server's restart, and an M6 detail that
// drifts from the golden bytes.
#[test]
fn inherited_name_is_refused_other_names_bind_and_restart_frees_it() {
    let server = server_child(Some("reviewer"));
    let f = fixture(Some(server.id()));

    let refused = resolve(&f, "reviewer", "op-x", Some(PANE)).expect_err("X is the server's own");
    assert_eq!(refused.code, ErrorCode::InheritedSeatName);
    assert_eq!(
        format!("herdr-threads: {}\n", refused.detail),
        include_str!("../cli/golden/m6.txt")
    );
    assert_eq!(
        seats_named(&f.db, "reviewer"),
        0,
        "a refused name is never bound"
    );

    let CommandResult::NamedSeatResolved { created, .. } =
        resolve(&f, "builder", "op-y", Some(PANE)).expect("Y is honoured")
    else {
        panic!("a named resolve answers NamedSeatResolved");
    };
    assert!(created);
    assert_eq!(seats_named(&f.db, "builder"), 1);

    // The server restarts without the variable.
    let restarted = server_child(None);
    f.host.restart_as(restarted.id());
    let CommandResult::NamedSeatResolved { created, .. } =
        resolve(&f, "reviewer", "op-x2", Some(PANE)).expect("X binds after the restart")
    else {
        panic!("a named resolve answers NamedSeatResolved");
    };
    assert!(created);
    assert_eq!(seats_named(&f.db, "reviewer"), 1);
}

// Kills: a check that runs without a host candidate (a plain named shell has no server env to
// inherit), and one that fails closed when the server has no readable pid.
#[test]
fn no_hint_or_no_server_pid_skips_the_check() {
    let server = server_child(Some("reviewer"));
    let f = fixture(Some(server.id()));
    resolve(&f, "reviewer", "op-a", None).expect("no host candidate: nothing to inherit from");
    assert_eq!(seats_named(&f.db, "reviewer"), 1);

    let f = fixture(None);
    resolve(&f, "reviewer", "op-b", Some(PANE)).expect("no server pid: the check is skipped");
    assert_eq!(seats_named(&f.db, "reviewer"), 1);
}
