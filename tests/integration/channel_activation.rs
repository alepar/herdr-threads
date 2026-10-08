//! Production loader, detached daemon and CLI recovery across the frozen relay seam.
use super::sweep::{FakeHost, Scratch, pane};
use herdr_threads::{
    app::SystemClock,
    cli::{
        handoff::{HandoffPlan, HandoffRequest},
        journal::{IntentScope, Journal, SemanticMutation},
        launch::LaunchRequest,
    },
    client::local::LocalSocketClient,
    daemon::{
        ownership::EndpointDescriptor,
        paths::{InstancePaths, RuntimeContext},
    },
    ports::LocalClient,
    protocol::{
        authority::CallerClaim,
        ids::*,
        output::ContinuationContext,
        results::CommandResult,
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
struct World {
    root: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    paths: InstancePaths,
    _host: FakeHost,
    _scratch: Scratch,
}
impl World {
    fn new(version: i64, disabled: bool) -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/htch-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let socket = root.join("host.sock");
        let mut sender = pane("w1:p1", "terminal-a");
        sender["label"] = json!("sender");
        let host = FakeHost::start(&socket, vec![sender, pane("w1:p2", "terminal-b")]);
        let state = root.join("custom-state");
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(state.clone(), socket.clone(), None).unwrap(),
        )
        .unwrap();
        paths.prepare_instance_dir().unwrap();
        if version != 0 {
            let db = rusqlite::Connection::open(&paths.database_path).unwrap();
            let mut migrations = fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect::<Vec<_>>();
            migrations.sort();
            for path in migrations {
                let n = path.file_name().unwrap().to_str().unwrap()[..4]
                    .parse::<i64>()
                    .unwrap();
                if n <= version {
                    db.execute_batch(&fs::read_to_string(path).unwrap())
                        .unwrap();
                }
            }
            db.pragma_update(None, "user_version", version).unwrap();
            let owner = herdr_threads::daemon::ownership::OwnerLock::acquire(&paths).unwrap();
            let instance = owner.instance_uuid().to_string();
            db.execute(
                "INSERT INTO host_instances(id,created_at) VALUES(?1,0)",
                [&instance],
            )
            .unwrap();
            db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('historical',?1,'old channel','preserve',-9000000,-9000000)", [&instance]).unwrap();
        }
        if disabled {
            let path = paths.instance_dir.join("settings.json");
            fs::write(&path, r#"{"auto_archive_after_ms":0}"#).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let world = Self {
            root: root.clone(),
            state,
            socket,
            paths,
            _host: host,
            _scratch: Scratch(root),
        };
        world.ok(None, &["daemon", "ensure"]);
        world
    }
    fn command(&self, claim: Option<&CallerClaim>, args: &[&str]) -> std::process::Command {
        let mut cmd = crate::scrubbed_command(BIN);
        cmd.args(["--json", "--state-dir"])
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.socket)
            .env("HOME", self.root.join("home"))
            .env("CODEX_HOME", self.root.join("codex"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"));
        if let Some(c) = claim {
            cmd.args([
                "--cooperative-seat",
                c.seat.as_str(),
                "--cooperative-target",
                c.target.as_str(),
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
            ]);
        }
        cmd.args(args);
        cmd
    }
    fn ok(&self, claim: Option<&CallerClaim>, args: &[&str]) -> Value {
        let out = self.command(claim, args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"]["data"].clone()
    }
    fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(&self.paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(3)).unwrap();
        db
    }
    fn check_in(&self) -> CallerClaim {
        let seat = self.ok(None, &["seat", "resolve", "--pane", "w1:p1"]);
        let out = self
            .command(
                None,
                &[
                    "--cooperative-seat",
                    seat.as_str().unwrap(),
                    "--cooperative-target",
                    "w1:p1",
                    "--cooperative-harness",
                    "claude",
                    "--cooperative-role",
                    "top-level",
                    "check-in",
                    "--lifecycle-event",
                    "initial",
                ],
            )
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        serde_json::from_value(
            serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"]["data"]["context"]
                .clone(),
        )
        .unwrap()
    }
    fn call(&self, command: herdr_threads::protocol::commands::Command) -> CommandResult {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let descriptor: EndpointDescriptor =
            serde_json::from_slice(&fs::read(&self.paths.descriptor_path).unwrap()).unwrap();
        let client = LocalSocketClient::new(
            descriptor.endpoint,
            clock.clone(),
            descriptor.instance_uuid,
            Some(descriptor.boot_id),
        );
        client
            .call(
                command,
                &CallBudget {
                    deadline: MonoInstant(clock.monotonic_now().0 + 5000),
                    cancellation: Cancellation::default(),
                },
            )
            .unwrap()
    }
}
impl Drop for World {
    fn drop(&mut self) {
        self.state = self
            .paths
            .instance_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let _ = self.command(None, &["daemon", "stop"]).output();
    }
}
fn wait(what: &str, mut check: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(Instant::now() < until, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn channel_activation_real_daemon_loads_fresh_21_and_22_and_owns_archival_lane() {
    for version in [0, 21, 22] {
        let w = World::new(version, false);
        // The boot archival pass may precede canonical host admission, then
        // park until its 60s safety tick. Resolve a real fixture target before
        // expecting prompt initialization: this admits qualified host evidence
        // and the seat_archival insertion kicks the existing archival lane.
        w.ok(None, &["seat", "resolve", "--pane", "w1:p1"]);
        let db = w.db();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            26
        );
        wait("archival worker initialization", || {
            db.query_row(
                "SELECT count(*) FROM archival_instances WHERE policy_ms=3600000",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        });
        let old_boot: String = db
            .query_row("SELECT runtime_boot FROM archival_instances", [], |r| {
                r.get(0)
            })
            .unwrap();
        w.ok(None, &["daemon", "stop"]);
        w.ok(None, &["daemon", "ensure"]);
        wait("fresh runtime grace", || {
            db.query_row("SELECT runtime_boot FROM archival_instances", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
                != old_boot
        });
        assert_eq!(
            db.query_row("SELECT count(*) FROM threads WHERE archived=1", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0,
            "migration and restart cannot archive historical channels immediately"
        );
        if version != 0 {
            wait("historical channel discovery", || {
                db.query_row(
                    "SELECT count(*) FROM channel_archival WHERE thread_id='historical'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
                    == 1
            });
            let (quiet, due): (Option<i64>, i64) = db
                .query_row(
                    "SELECT quiet_mono,due_mono FROM channel_archival WHERE thread_id='historical'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert!(
                quiet.is_none_or(|at| at >= 0) && due >= 0,
                "historical timestamps cannot backdate monotonic grace"
            );
        }
    }
    let w = World::new(0, true);
    let claim = w.check_in();
    w.ok(Some(&claim), &["thread", "create", "--topic", "disabled"]);
    assert_eq!(
        w.db()
            .query_row("SELECT count(*) FROM archival_instances", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "zero policy parks the owned lane without decisions"
    );
}

#[test]
fn channel_activation_cli_complete_removal_failure_archived_binding_retry_is_cleanup_only() {
    let mut w = World::new(22, false);
    let claim = w.check_in();
    let recipient = SeatId::new(
        w.ok(None, &["seat", "resolve", "--pane", "w1:p2"])
            .as_str()
            .unwrap(),
    );
    let journal_root = w.paths.instance_dir.join("intents");
    let journal = Journal::open(&journal_root).unwrap();
    let key = || OperationId::new(uuid::Uuid::new_v4().to_string());
    // This is a historical journal spelling, not merely a later CLI selector.
    // Both frozen paths use an alias that the new daemon canonicalizes at startup.
    let frozen_alias = w.root.join("legacy-alias");
    std::os::unix::fs::symlink(&w.root, &frozen_alias).unwrap();
    let plan = HandoffPlan {
        request: HandoffRequest {
            thread: None,
            thread_name: Some("legacy handoff".into()),
            topic: Some("topic".into()),
            goal: Some("goal".into()),
            body: "durable assignment".into(),
            launch: LaunchRequest {
                target: HostTargetId::new("w1:p2"),
                harness: herdr_threads::harness::context::Harness::Claude,
                harness_binary: None,
                argv: vec![],
                name: None,
                pane_label: None,
            },
        },
        context: ContinuationContext {
            state_dir: Some(
                frozen_alias
                    .join("custom-state")
                    .to_string_lossy()
                    .into_owned(),
            ),
            host: Some(
                frozen_alias
                    .join("host.sock")
                    .to_string_lossy()
                    .into_owned(),
            ),
        },
        recipient: recipient.clone(),
        create_key: key(),
        invite_key: key(),
        send_key: key(),
    };
    let reference = journal
        .record(
            IntentScope::Cooperative {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
            },
            SemanticMutation::freeze(
                SemanticMutation::Handoff(Box::new(plan.clone())),
                claim.clone(),
            )
            .unwrap(),
            0,
        )
        .unwrap();
    // Legacy crash fixture: CREATE was decided before Begin existed. The production
    // retry must discover this exact recorded result and never leave an unattached fence.
    let created = w.call(
        SemanticMutation::CreateThread {
            name: plan.request.thread_name.clone(),
            topic: "topic".into(),
            goal: "goal".into(),
        }
        .to_command(plan.create_key.clone(), Some(claim.clone()))
        .unwrap(),
    );
    let CommandResult::ThreadCreated(thread) = created else {
        panic!("expected thread")
    };
    let invitation = w.call(
        SemanticMutation::Invite {
            thread: thread.clone(),
            seat: recipient,
            deadline_millis: None,
        }
        .to_command(plan.invite_key.clone(), Some(claim.clone()))
        .unwrap(),
    );
    let message = w.call(
        SemanticMutation::SendMessage {
            delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
            thread: thread.clone(),
            body: plan.request.body.clone(),
            invited_recipients: vec![plan.recipient.clone()],
            deadline_millis: None,
            relays_user: false,
            user_intent: None,
        }
        .to_command(plan.send_key.clone(), Some(claim.clone()))
        .unwrap(),
    );
    // Frozen native success is the on-disk recovery boundary; no native harness runs.
    fs::write(journal_root.join(format!("handoff-{}.progress",reference.operation.as_str())),serde_json::to_vec(&json!({"thread":thread,"invitation":invitation,"message":message,"possible_start":true,"launch":{"outcome":"started"}})).unwrap()).unwrap();
    fs::write(
        journal_root.join(format!("handoff-{}.lock", reference.operation.as_str())),
        b"",
    )
    .unwrap();
    struct Mode(PathBuf);
    impl Drop for Mode {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }
    let restore = Mode(journal_root.clone());
    fs::set_permissions(&journal_root, fs::Permissions::from_mode(0o500)).unwrap();
    let out = w
        .command(
            None,
            &[
                "--cooperative-seat",
                claim.seat.as_str(),
                "--cooperative-target",
                "sender",
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
                "retry",
                &reference.recovery_ref(),
            ],
        )
        .env("HERDR_PANE_ID", "w1:p1")
        .output()
        .unwrap();
    drop(restore);
    assert!(!out.status.success(), "cleanup must fail: {out:?}");
    let db = w.db();
    assert_eq!(
        db.query_row(
            "SELECT state,thread_id FROM channel_handoff_fences WHERE compound=?1",
            [reference.operation.as_str()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        )
        .unwrap(),
        ("completed".into(), thread.as_str().to_owned()),
        "Complete must commit before failed local removal: {out:?}"
    );
    assert!(journal.load(&reference).is_ok());
    w.ok(Some(&claim), &["archive", thread.as_str()]);
    w.ok(
        Some(&claim),
        &["check-in", "--lifecycle-event", "replacement"],
    );
    let snapshot = || {
        db.query_row("SELECT (SELECT count(*) FROM operations),(SELECT count(*) FROM messages),(SELECT count(*) FROM invitations),(SELECT count(*) FROM channel_handoff_fences WHERE state='live')",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap()
    };
    let before = snapshot();
    w.ok(None, &["daemon", "stop"]);
    w.ok(None, &["daemon", "ensure"]);
    wait("retained completed intent coverage", || {
        db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
            == 0
    });
    // A copied UUID/database in a distinct root cannot authorize historical cleanup.
    let original_state = w.state.clone();
    let copied_state = w.root.join("copied-state");
    let copied_paths = InstancePaths::resolve(
        &RuntimeContext::explicit(copied_state.clone(), w.socket.clone(), None).unwrap(),
    )
    .unwrap();
    copied_paths.prepare_instance_dir().unwrap();
    db.execute(
        "VACUUM INTO ?1",
        [copied_paths.database_path.to_str().unwrap()],
    )
    .unwrap();
    fs::copy(&w.paths.namespace_path, &copied_paths.namespace_path).unwrap();
    let copied_journal = copied_paths.instance_dir.join("intents");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&copied_journal)
        .unwrap();
    for entry in fs::read_dir(&journal_root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), copied_journal.join(entry.file_name())).unwrap();
        }
    }
    w.state = copied_state;
    struct Stop(std::process::Command);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = self.0.output();
        }
    }
    let copied_stop = Stop(w.command(None, &["daemon", "stop"]));
    w.ok(None, &["daemon", "ensure"]);
    let rejected = w
        .command(Some(&claim), &["retry", &reference.recovery_ref()])
        .output()
        .unwrap();
    assert!(
        !rejected.status.success(),
        "copied UUID must not authorize cleanup"
    );
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("exact canonical state directory"),
        "{rejected:?}"
    );
    assert!(
        Journal::open(&copied_journal)
            .unwrap()
            .load(&reference)
            .is_ok()
    );
    drop(copied_stop);
    w.state = original_state;
    // Path aliases must retain the frozen canonical namespace and digest.
    let alias = w.root.join("state-alias");
    std::os::unix::fs::symlink(&w.state, &alias).unwrap();
    w.state = alias;
    let out = w
        .command(Some(&claim), &["retry", &reference.recovery_ref()])
        .output()
        .unwrap();
    assert!(out.status.success(), "historical cleanup: {out:?}");
    assert!(journal.load(&reference).is_err());
    assert_eq!(
        snapshot(),
        before,
        "terminal replay must create no new operations or protection"
    );
}
