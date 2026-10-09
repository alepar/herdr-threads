use herdr_threads::{
    archival_legacy::Source,
    daemon::paths::{InstancePaths, RuntimeContext},
    protocol::output::ContinuationContext,
};
pub(super) struct Temp(std::path::PathBuf);
impl Temp {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub(super) fn source() -> (Temp, InstancePaths, Source) {
    let dir = Temp(std::env::temp_dir().join(format!("ht-archive-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&dir.0).unwrap();
    let context =
        RuntimeContext::explicit(dir.path().to_owned(), dir.path().join("host.sock"), None)
            .unwrap();
    let paths = InstancePaths::resolve_read_only(&context).unwrap();
    std::fs::create_dir_all(&paths.instance_dir).unwrap();
    let source = Source::new(
        &paths,
        "i".into(),
        ContinuationContext {
            state_dir: Some(context.state_dir.to_string_lossy().into()),
            host: Some(context.host_endpoint.to_string_lossy().into()),
        },
    );
    (dir, paths, source)
}
// Complete a bounded scan, without treating a partial page as absence or a
// veto. Tests of the page boundary below continue to call scan directly.
fn complete_scan(source: &mut Source) -> herdr_threads::archival_legacy::Scan {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut hints = Vec::new();
    loop {
        let mut page = source.scan(|| false).unwrap();
        hints.append(&mut page.hints);
        if !page.pending {
            page.hints = hints;
            return page;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "legacy scan did not finish"
        );
    }
}
#[test]
fn archival_legacy_absence_and_orphan_progress_are_covered_read_only() {
    let (_dir, paths, mut source) = source();
    let first = complete_scan(&mut source)
        .coverage
        .expect("absence is covered");
    assert!(source.validate(&first));
    assert!(!paths.instance_dir.join("intents").exists());
    std::fs::create_dir(paths.instance_dir.join("intents")).unwrap();
    std::fs::write(
        paths.instance_dir.join("intents/handoff-orphan.progress"),
        b"broken",
    )
    .unwrap();
    assert!(!source.validate(&first));
    let calls = std::cell::Cell::new(0);
    let next = source
        .scan(|| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        })
        .unwrap();
    assert!(next.pending, "expired page must remain partial");
    assert!(
        next.coverage.is_none(),
        "partial traversal cannot claim coverage"
    );
    assert!(next.hints.is_empty());
    let next = complete_scan(&mut source);
    assert!(next.coverage.is_some());
    assert!(next.hints.is_empty());
}
#[test]
fn archival_legacy_noncompound_and_malformed_veto_until_fresh_absence() {
    let (_dir, paths, mut source) = source();
    let root = paths.instance_dir.join("intents");
    std::fs::create_dir(&root).unwrap();
    let file = root.join("00000000000000000001-op.intent");
    for bytes in [
        b"broken".as_slice(),
        br#"{"kind":"send_message"}
malformed tail"#,
    ] {
        std::fs::write(&file, bytes).unwrap();
        assert!(complete_scan(&mut source).coverage.is_none());
    }
    std::fs::remove_file(file).unwrap();
    assert!(complete_scan(&mut source).coverage.is_some());
}
fn context(paths: &InstancePaths) -> ContinuationContext {
    ContinuationContext {
        state_dir: Some(
            paths
                .instance_dir
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_string_lossy()
                .into(),
        ),
        host: Some(paths.locator.clone()),
    }
}
pub(super) fn compound(paths: &InstancePaths) -> herdr_threads::cli::journal::IntentRef {
    compound_in(paths, context(paths))
}
fn compound_in(
    paths: &InstancePaths,
    context: ContinuationContext,
) -> herdr_threads::cli::journal::IntentRef {
    legacy_compound(paths, context, false)
}
fn legacy_compound(
    paths: &InstancePaths,
    context: ContinuationContext,
    new_thread: bool,
) -> herdr_threads::cli::journal::IntentRef {
    use herdr_threads::{
        cli::{
            handoff::{HandoffPlan, HandoffRequest},
            journal::{IntentScope, Journal, SemanticMutation},
            launch::LaunchRequest,
        },
        protocol::ids::*,
    };
    let id = super::handoff_fences::identity();
    let plan = HandoffPlan {
        request: HandoffRequest {
            thread: (!new_thread).then(|| ThreadId::new("t")),
            thread_name: None,
            topic: new_thread.then(|| "literal-topic".into()),
            goal: new_thread.then(|| "literal-goal".into()),
            body: "body".into(),
            launch: LaunchRequest {
                target: HostTargetId::new("p"),
                harness: serde_json::from_str("\"Codex\"").unwrap(),
                harness_binary: None,
                argv: vec![],
                name: None,
                pane_label: None,
            },
        },
        context,
        recipient: id.recipient,
        create_key: id.create_key,
        invite_key: id.invite_key,
        send_key: id.send_key,
    };
    Journal::open(paths.instance_dir.join("intents"))
        .unwrap()
        .record(
            IntentScope::Cooperative {
                instance: "i".into(),
                seat: id.claim.seat.clone(),
            },
            SemanticMutation::freeze(SemanticMutation::Handoff(Box::new(plan)), id.claim).unwrap(),
            0,
        )
        .unwrap()
}
#[test]
fn archival_legacy_verified_compound_has_only_exact_veto_identity() {
    let (_dir, paths, mut source) = source();
    let reference = compound(&paths);
    let scan = complete_scan(&mut source);
    assert!(scan.coverage.is_some());
    assert_eq!(scan.hints.len(), 1);
    let herdr_threads::archival_legacy::Hint::Handoff { identity, .. } = &scan.hints[0] else {
        panic!("expected legacy handoff")
    };
    assert_eq!(identity.compound, reference.operation);
    assert_eq!(identity.thread.as_ref().unwrap().as_str(), "t");
}
#[test]
fn archival_legacy_aliases_preserve_frozen_bytes_and_digest() {
    use herdr_threads::cli::journal::Journal;
    let (dir, paths, mut source) = source();
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(dir.path(), &alias).unwrap();
    let aliased = ContinuationContext {
        state_dir: Some(alias.to_string_lossy().into()),
        host: Some(alias.join("host.sock").to_string_lossy().into()),
    };
    let reference = compound_in(&paths, aliased.clone());
    let file = paths.instance_dir.join("intents").join(format!(
        "{:020}-{}.intent",
        reference.ordinal,
        reference.operation.as_str()
    ));
    let original = std::fs::read(&file).unwrap();
    let header = Journal::open(paths.instance_dir.join("intents"))
        .unwrap()
        .load(&reference)
        .unwrap()
        .header;
    // Exercise normalization with canonical and aliased current contexts.
    for scanner in [&mut source, &mut Source::new(&paths, "i".into(), aliased)] {
        let scan = complete_scan(scanner);
        assert!(scan.coverage.is_some());
        assert_eq!(scan.hints.len(), 1);
        let herdr_threads::archival_legacy::Hint::Handoff { identity, .. } = &scan.hints[0] else {
            panic!("expected legacy handoff")
        };
        assert_eq!(identity.digest, header.semantic_digest);
    }
    assert_eq!(std::fs::read(file).unwrap(), original);
}
#[test]
fn archival_legacy_unknown_or_distinct_full_namespaces_veto() {
    use herdr_threads::cli::journal::Journal;
    let (dir, paths, mut source) = source();
    let current = context(&paths);
    let foreign = dir.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    let loop_path = dir.path().join("loop");
    std::os::unix::fs::symlink(&loop_path, &loop_path).unwrap();
    let inaccessible = dir.path().join("inaccessible");
    std::fs::create_dir(&inaccessible).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut cases = vec![
        ContinuationContext {
            state_dir: None,
            host: current.host.clone(),
        },
        ContinuationContext {
            state_dir: current.state_dir.clone(),
            host: None,
        },
        ContinuationContext {
            state_dir: current.state_dir.clone(),
            host: Some(dir.path().join("other.sock").to_string_lossy().into()),
        },
        ContinuationContext {
            state_dir: current.state_dir.clone(),
            host: Some(foreign.join("host.sock").to_string_lossy().into()),
        },
    ];
    for state in [
        foreign,
        dir.path().join("missing"),
        loop_path,
        inaccessible,
        "relative".into(),
    ] {
        cases.push(ContinuationContext {
            state_dir: Some(state.to_string_lossy().into()),
            host: current.host.clone(),
        });
    }
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    for frozen in cases {
        let reference = compound_in(&paths, frozen.clone());
        let scan = complete_scan(&mut source);
        assert!(scan.coverage.is_none(), "{frozen:?}");
        assert!(scan.hints.is_empty(), "{frozen:?}");
        journal.complete(&reference).unwrap();
        let reference = compound(&paths);
        // Current uncertainty cannot be repaired by the frozen record either.
        let mut unknown_current = Source::new(&paths, "i".into(), frozen);
        assert!(complete_scan(&mut unknown_current).coverage.is_none());
        journal.complete(&reference).unwrap();
    }
    assert!(complete_scan(&mut source).coverage.is_some());
}
#[test]
fn archival_legacy_valid_noncompound_is_a_transient_coverage_veto() {
    use herdr_threads::cli::journal::{IntentScope, Journal, SemanticMutation};
    let (_dir, paths, mut source) = source();
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let id = super::handoff_fences::identity();
    let reference = journal
        .record(
            IntentScope::Cooperative {
                instance: "i".into(),
                seat: id.claim.seat.clone(),
            },
            SemanticMutation::freeze(
                SemanticMutation::SendMessage {
                    delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
                    user_intent: None,
                    thread: herdr_threads::protocol::ids::ThreadId::new("t"),
                    body: "work".into(),
                    invited_recipients: vec![],
                    deadline_millis: None,
                    relays_user: false,
                },
                id.claim,
            )
            .unwrap(),
            0,
        )
        .unwrap();
    let scan = complete_scan(&mut source);
    assert!(scan.coverage.is_none());
    assert!(scan.hints.is_empty());
    assert!(
        journal.load(&reference).is_ok(),
        "importer must not remove intent"
    );
    journal.complete(&reference).unwrap();
    assert!(complete_scan(&mut source).coverage.is_some());
}
#[test]
fn archival_legacy_partial_pages_and_cancel_never_claim_coverage() {
    let (_dir, paths, mut source) = source();
    let root = paths.instance_dir.join("intents");
    std::fs::create_dir(&root).unwrap();
    for n in 0..100 {
        std::fs::write(root.join(format!("handoff-{n}.progress")), b"orphan").unwrap();
    }
    let first = source.scan(|| false).unwrap();
    assert!(first.pending);
    assert!(first.coverage.is_none());
    assert!(source.scan(|| true).is_err());
    let mut finished = false;
    for _ in 0..10 {
        let page = source.scan(|| false).unwrap();
        if page.coverage.is_some() {
            finished = true;
            break;
        }
    }
    assert!(finished, "resumed bounded traversal must exhaust");
}
#[test]
fn archival_legacy_unsafe_roots_files_and_caps_veto() {
    use std::os::unix::fs::symlink;
    let (dir, paths, mut source) = source();
    let root = paths.instance_dir.join("intents");
    symlink(dir.path(), &root).unwrap();
    assert!(source.scan(|| false).is_err());
    std::fs::remove_file(&root).unwrap();
    std::fs::create_dir(&root).unwrap();
    let intent = root.join("00000000000000000001-op.intent");
    symlink(dir.path().join("missing"), &intent).unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::remove_file(&intent).unwrap();
    std::fs::write(
        &intent,
        vec![b'x'; herdr_threads::archival_legacy::RECORD_CAP + 1],
    )
    .unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::remove_file(&intent).unwrap();
    std::fs::create_dir(&intent).unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
}
#[test]
fn archival_legacy_false_kind_digest_and_malformed_compound_tails_veto() {
    let (_dir, paths, mut source) = source();
    let reference = compound(&paths);
    let path = paths.instance_dir.join("intents").join(format!(
        "{:020}-{}.intent",
        reference.ordinal,
        reference.operation.as_str()
    ));
    let original = std::fs::read_to_string(&path).unwrap();
    let header_end = original.find('\n').unwrap();
    let mut header: serde_json::Value = serde_json::from_str(&original[..header_end]).unwrap();
    header["kind"] = serde_json::json!("send_message");
    std::fs::write(
        &path,
        format!("{}\n{}", header, &original[header_end + 1..]),
    )
    .unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::write(&path, format!("{original}malformed trailing JSON")).unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::write(
        &path,
        original.replace("\"body\":\"body\"", "\"body\":\"changed\""),
    )
    .unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
}
#[test]
fn archival_legacy_captured_hint_cannot_revive_completion() {
    let (_dir, paths, mut source) = source();
    compound(&paths);
    let mut scan = complete_scan(&mut source);
    let herdr_threads::archival_legacy::Hint::Handoff {
        identity,
        progress_thread,
        ..
    } = scan.hints.pop().unwrap()
    else {
        panic!("expected legacy handoff")
    };
    let mut db = super::channel_archival::fixture();
    super::channel_archival::thread(&db);
    super::channel_archival::joined_agent(&db);
    let tx = db.transaction().unwrap();
    herdr_threads::store::handoff::begin_pending(
        &tx,
        &identity,
        herdr_threads::protocol::time::UtcMillis(0),
    )
    .unwrap();
    herdr_threads::store::handoff::complete_pending(
        &tx,
        &identity,
        herdr_threads::protocol::time::UtcMillis(1),
    )
    .unwrap();
    tx.commit().unwrap();
    let tx = db.transaction().unwrap();
    let result = herdr_threads::store::handoff::import_hint(
        &tx,
        &identity,
        progress_thread.as_ref(),
        "source",
        herdr_threads::protocol::time::UtcMillis(2),
    )
    .unwrap();
    assert_eq!(
        result.state,
        herdr_threads::protocol::handoff::HandoffState::Completed
    );
    tx.commit().unwrap();
}

pub(super) fn modern_compound(
    paths: &InstancePaths,
    bootstrap: bool,
    new_thread: bool,
) -> herdr_threads::cli::journal::IntentRef {
    use herdr_threads::cli::journal::{BootstrapPlan, DeliveryPlan, Journal, SemanticMutation};
    use herdr_threads::protocol::handoff::{HandoffChannel, HandoffNamespace};
    let mut id = super::topology_handoff::identity();
    let current = context(paths);
    id.payload.handoff.namespace = HandoffNamespace {
        instance: "i".into(),
        state_dir: current.state_dir.unwrap().into(),
        host_endpoint: current.host.unwrap().into(),
    };
    if new_thread {
        id.payload.handoff.channel = HandoffChannel::New {
            name: Some("literal-name".into()),
            topic: "literal-topic".into(),
            goal: "literal-goal".into(),
        };
    }
    let mutation = if bootstrap {
        SemanticMutation::HandoffBootstrap(Box::new(BootstrapPlan {
            version: 1,
            payload: id.payload,
        }))
    } else {
        SemanticMutation::HandoffDelivery(Box::new(DeliveryPlan {
            version: 1,
            payload: id.payload.handoff,
            recipient: herdr_threads::protocol::ids::SeatId::new("recipient"),
        }))
    };
    Journal::open(paths.instance_dir.join("intents"))
        .unwrap()
        .record(
            id.scope,
            SemanticMutation::freeze(mutation, id.claim).unwrap(),
            0,
        )
        .unwrap()
}

#[test]
fn archival_legacy_bootstrap_existing_and_new_thread_are_bounded_readonly_hints() {
    for new_thread in [false, true] {
        let (_dir, paths, mut source) = source();
        let reference = modern_compound(&paths, true, new_thread);
        let file = paths.instance_dir.join("intents").join(format!(
            "{:020}-{}.intent",
            reference.ordinal,
            reference.operation.as_str()
        ));
        let original = std::fs::read(&file).unwrap();
        let scan = complete_scan(&mut source);
        assert!(
            scan.coverage.is_some(),
            "valid bootstrap must establish coverage"
        );
        assert_eq!(
            scan.hints.len(),
            1,
            "bootstrap must reach canonical deciding import"
        );
        assert_eq!(std::fs::read(&file).unwrap(), original);
    }
}

#[test]
fn archival_legacy_delivery_uses_its_own_frozen_identity_without_launch() {
    let (_dir, paths, mut source) = source();
    modern_compound(&paths, false, false);
    let scan = complete_scan(&mut source);
    assert!(
        scan.coverage.is_some(),
        "valid delivery must establish coverage"
    );
    assert_eq!(
        scan.hints.len(),
        1,
        "delivery must protect its exact channel"
    );
}

pub(super) fn scanner_store(
    paths: &InstancePaths,
) -> (herdr_threads::store::SqliteStore, rusqlite::Connection) {
    use herdr_threads::store::{SqliteStore, StoreSettings, connection::StoreContext};
    let store = SqliteStore::new(
        StoreContext::new(
            paths.database_path.clone(),
            std::sync::Arc::new(herdr_threads::app::SystemClock::default()),
        ),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    let db = rusqlite::Connection::open(&paths.database_path).unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES('i',0)",
        [],
    )
    .unwrap();
    super::channel_archival::thread(&db);
    super::channel_archival::joined_agent(&db);
    (store, db)
}
fn import_scan(
    store: &herdr_threads::store::SqliteStore,
    scan: &herdr_threads::archival_legacy::Scan,
) -> Result<herdr_threads::store::archival::Progress, herdr_threads::protocol::results::ApiError> {
    use herdr_threads::{
        ports::StorePort,
        protocol::time::{CallBudget, MonoInstant, UtcMillis},
        store::archival::Runtime,
    };
    let budget = CallBudget {
        deadline: MonoInstant(store.clock().monotonic_now().0 + 5000),
        cancellation: Default::default(),
    };
    let rt = Runtime {
        boot: "archive-test".into(),
        mono: 0,
        utc: UtcMillis(0),
        after_ms: 60000,
        host_generation: 0,
        coherent: false,
        valid_until_mono: None,
        legacy_source: scan.coverage.clone(),
    };
    store.archival_pass(&rt, &scan.hints, &budget)
}
fn bootstrap_hint(
    scan: &herdr_threads::archival_legacy::Scan,
) -> herdr_threads::protocol::handoff::BootstrapIdentity {
    let herdr_threads::archival_legacy::Hint::Bootstrap { identity, .. } = &scan.hints[0] else {
        panic!("bootstrap expected")
    };
    identity.as_ref().clone()
}
fn cancel_bootstrap(
    db: &mut rusqlite::Connection,
    id: &herdr_threads::protocol::handoff::BootstrapIdentity,
) {
    use herdr_threads::{
        protocol::{handoff::*, time::UtcMillis},
        store::topology_handoff,
    };
    let ns = &id.payload.handoff.namespace;
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, ns, id, UtcMillis(0)).unwrap();
    let mut request = RecoverBootstrap {
        inspection: Some(
            BootstrapRecoveryInspection::from_status(
                &topology_handoff::current(&tx, &id.payload.handoff.namespace, id)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap(),
        ),
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::first(),
        operation: id.compound.clone(),
        disposition: BootstrapRecoveryDisposition::Cancelled {
            reason: "inspected quiescent fixture".into(),
            quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
            child_guard: BootstrapCancellationGuard {
                attached_child: None,
            },
        },
    };
    request.operation = request.decision_operation().unwrap();
    topology_handoff::attempts::recover(&tx, ns, &request, 501, UtcMillis(1), None).unwrap();
    tx.commit().unwrap();
}
#[test]
fn archival_legacy_cancelled_bootstrap_dominates_delayed_scan_and_restart() {
    let (_dir, paths, mut source) = source();
    modern_compound(&paths, true, false);
    let scan = complete_scan(&mut source);
    let id = bootstrap_hint(&scan);
    let (store, mut db) = scanner_store(&paths);
    cancel_bootstrap(&mut db, &id);
    for active in [
        &store,
        &herdr_threads::store::SqliteStore::new(
            herdr_threads::store::connection::StoreContext::new(
                paths.database_path.clone(),
                std::sync::Arc::new(herdr_threads::app::SystemClock::default()),
            ),
            "i",
            Default::default(),
        )
        .unwrap(),
    ] {
        import_scan(active, &scan).unwrap();
        assert!(
            !db.query_row(
                "SELECT bootstrap_veto FROM archival_instances WHERE instance_id='i'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap(),
            "canonical cancellation must dominate retained local bootstrap"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn archival_legacy_selected_source_cannot_import_into_another_database() {
    let (_dir, paths, mut source) = source();
    modern_compound(&paths, false, false);
    let scan = complete_scan(&mut source);
    let (_other_dir, other_paths, _other_source) = self::source();
    let (store, db) = scanner_store(&other_paths);
    assert!(
        import_scan(&store, &scan).is_err(),
        "same instance UUID must not canonize foreign source"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn delivery_terminal(
    paths: &InstancePaths,
    reference: &herdr_threads::cli::journal::IntentRef,
) -> (std::path::PathBuf, Vec<u8>) {
    use herdr_threads::protocol::{
        handoff::{HandoffResult, HandoffState},
        ids::*,
        results::CommandResult,
    };
    use sha2::{Digest, Sha256};
    let original_path = paths.instance_dir.join("intents").join(format!(
        "{:020}-{}.intent",
        reference.ordinal,
        reference.operation.as_str()
    ));
    let original = std::fs::read_to_string(&original_path).unwrap();
    let invitation = CommandResult::Invitation(InvitationId::new("invitation"));
    let message = CommandResult::MessageSent(MessageId::new("message"));
    let report = serde_json::json!({"compound":"bootstrap","thread":"t","recipient":"recipient","participation":"invited_pending","outcome":"staged","invitation":invitation,"message":message,"recovery_ref":reference.recovery_ref()});
    let report_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&report).unwrap()));
    let progress = serde_json::json!({"staged":{"thread":"t","invitation":invitation,"message":message,"invitation_attempted":true},"report":report,"report_digest":report_digest});
    // Typed producer field order is staged, report, report_digest (not JSON map sort order).
    let progress_bytes = format!(
        "{{\"staged\":{{\"thread\":\"t\",\"invitation\":{},\"message\":{},\"invitation_attempted\":true}},\"report\":{},\"report_digest\":{}}}",
        serde_json::to_string(&invitation).unwrap(),
        serde_json::to_string(&message).unwrap(),
        serde_json::to_string(&report).unwrap(),
        serde_json::to_string(&report_digest).unwrap()
    );
    let terminal = serde_json::json!({"version":1,"original":original,"completed":HandoffResult {compound:OperationId::new("bootstrap"),thread:Some(ThreadId::new("t")),state:HandoffState::Completed},"progress":progress,"progress_digest":format!("{:x}",Sha256::digest(progress_bytes.as_bytes()))});
    let path = paths.instance_dir.join("intents").join(format!(
        "delivery-{:020}-{}.terminal",
        reference.ordinal,
        reference.operation.as_str()
    ));
    let bytes = serde_json::to_vec(&terminal).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    std::fs::remove_file(original_path).unwrap();
    (path, bytes)
}
#[test]
fn archival_legacy_delivery_terminal_after_cleanup_requires_canonical_completion() {
    let (_dir, paths, mut source) = source();
    let reference = modern_compound(&paths, false, false);
    let (path, bytes) = delivery_terminal(&paths, &reference);
    let scan = complete_scan(&mut source);
    assert!(
        scan.coverage.is_some(),
        "strict retained delivery origin must remain readable after cleanup"
    );
    assert_eq!(scan.hints.len(), 1);
    let (store, mut db) = scanner_store(&paths);
    import_scan(&store, &scan).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "terminal file cannot manufacture completion or live fence"
    );
    assert!(
        db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    let herdr_threads::archival_legacy::Hint::Handoff { identity, .. } = &scan.hints[0] else {
        panic!("delivery expected")
    };
    let tx = db.transaction().unwrap();
    herdr_threads::store::handoff::begin_pending(
        &tx,
        identity,
        herdr_threads::protocol::time::UtcMillis(0),
    )
    .unwrap();
    herdr_threads::store::handoff::complete_pending(
        &tx,
        identity,
        herdr_threads::protocol::time::UtcMillis(1),
    )
    .unwrap();
    tx.commit().unwrap();
    let scan = complete_scan(&mut source);
    import_scan(&store, &scan).unwrap();
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

// Pace the cancellation check after one consumed entry beyond the scanner's
// 10ms page deadline. This tests the actual bounded continuation independently
// of filesystem enumeration order: every record page is pending, EOF is empty.
fn one_entry_page(source: &mut Source) -> herdr_threads::archival_legacy::Scan {
    let calls = std::cell::Cell::new(0);
    source
        .scan(|| {
            let call = calls.get();
            calls.set(call + 1);
            if call == 2 {
                std::thread::sleep(std::time::Duration::from_millis(12));
            }
            false
        })
        .unwrap()
}

#[test]
fn archival_legacy_missing_bootstrap_cannot_be_forgotten_between_pages() {
    let (_dir, paths, mut source) = source();
    modern_compound(&paths, true, false);
    for n in 0..2 {
        std::fs::write(
            paths
                .instance_dir
                .join("intents")
                .join(format!("handoff-orphan-{n}.progress")),
            b"orphan",
        )
        .unwrap();
    }
    let (store, db) = scanner_store(&paths);
    let mut final_page = false;
    for _ in 0..30 {
        let page = one_entry_page(&mut source);
        import_scan(&store, &page).unwrap();
        if !page.pending {
            assert!(
                page.coverage.is_none(),
                "earlier absent parent must veto final source coverage"
            );
            assert!(
                db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
                    r.get::<_, bool>(0)
                })
                .unwrap(),
                "absent canonical bootstrap must remain a veto at deciding coverage"
            );
            final_page = true;
            break;
        }
    }
    assert!(final_page);
    for table in [
        "bootstrap_handoffs",
        "bootstrap_attempts",
        "bootstrap_child_keys",
        "channel_handoff_fences",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "scan must not manufacture authority"
        );
    }
}

#[test]
fn archival_legacy_modern_frozen_claim_namespace_digest_and_progress_are_strict() {
    use std::os::unix::fs::PermissionsExt;
    for bootstrap in [false, true] {
        let (dir, paths, mut source) = source();
        let reference = modern_compound(&paths, bootstrap, false);
        let path = paths.instance_dir.join("intents").join(format!(
            "{:020}-{}.intent",
            reference.ordinal,
            reference.operation.as_str()
        ));
        let original = std::fs::read_to_string(&path).unwrap();
        let (header, body) = original.split_once('\n').unwrap();
        for pointer in ["/claim", "/mutation", "/mutation/payload"] {
            let mut value: serde_json::Value = serde_json::from_str(body).unwrap();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), true.into());
            std::fs::write(&path, format!("{header}\n{value}")).unwrap();
            assert!(complete_scan(&mut source).coverage.is_none());
        }
        let mut changed: serde_json::Value = serde_json::from_str(body).unwrap();
        changed["claim"]["native_session"] = serde_json::json!("changed");
        std::fs::write(&path, format!("{header}\n{changed}")).unwrap();
        assert!(complete_scan(&mut source).coverage.is_none());
        let mut changed: serde_json::Value = serde_json::from_str(body).unwrap();
        let pointer = if bootstrap {
            "/mutation/payload/handoff/namespace/state_dir"
        } else {
            "/mutation/payload/namespace/state_dir"
        };
        let foreign = dir.path().join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        *changed.pointer_mut(pointer).unwrap() = serde_json::json!(foreign);
        let semantic: herdr_threads::cli::journal::SemanticMutation =
            serde_json::from_value(changed.clone()).unwrap();
        use sha2::{Digest, Sha256};
        let mut h: serde_json::Value = serde_json::from_str(header).unwrap();
        h["semantic_digest"] = serde_json::json!(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&semantic).unwrap())
        ));
        std::fs::write(&path, format!("{h}\n{changed}")).unwrap();
        assert!(
            complete_scan(&mut source).coverage.is_none(),
            "valid recomputed digest with copied instance UUID cannot establish foreign namespace"
        );
        std::fs::write(&path, &original).unwrap();
        let progress = paths
            .instance_dir
            .join("intents")
            .join(format!("handoff-{}.progress", reference.operation.as_str()));
        for bytes in [
            b"malformed".to_vec(),
            vec![b'x'; herdr_threads::archival_legacy::RECORD_CAP + 1],
        ] {
            std::fs::write(&progress, bytes).unwrap();
            assert!(complete_scan(&mut source).coverage.is_none());
        }
        std::fs::set_permissions(&progress, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(complete_scan(&mut source).coverage.is_none());
        std::fs::remove_file(&progress).unwrap();
        std::os::unix::fs::symlink(&path, &progress).unwrap();
        assert!(complete_scan(&mut source).coverage.is_none());
        std::fs::remove_file(&progress).unwrap();
        assert!(complete_scan(&mut source).coverage.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
#[test]
fn archival_legacy_delivery_terminal_malformed_or_contradictory_records_veto() {
    let (_dir, paths, mut source) = source();
    let reference = modern_compound(&paths, false, false);
    let (path, original) = delivery_terminal(&paths, &reference);
    for pointer in ["", "/progress/staged", "/completed"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), true.into());
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(complete_scan(&mut source).coverage.is_none());
    }
    for (pointer, value) in [
        ("/version", serde_json::json!(2)),
        ("/progress_digest", serde_json::json!("0".repeat(64))),
        ("/progress/report/recipient", serde_json::json!("other")),
        ("/completed/state", serde_json::json!("live")),
    ] {
        let mut terminal: serde_json::Value = serde_json::from_slice(&original).unwrap();
        *terminal.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&path, serde_json::to_vec(&terminal).unwrap()).unwrap();
        assert!(complete_scan(&mut source).coverage.is_none());
    }
    std::fs::write(&path, &original).unwrap();
    let progress = paths
        .instance_dir
        .join("intents")
        .join(format!("handoff-{}.progress", reference.operation.as_str()));
    std::fs::write(&progress, b"{}").unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::remove_file(&progress).unwrap();
    std::fs::write(&path, vec![b'x'; 128 * 1024 + 1]).unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(paths.database_path.clone(), &path).unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(complete_scan(&mut source).coverage.is_some());
}

fn linked_archival_fixture(
    paths: &InstancePaths,
    source: &mut Source,
    new_thread: bool,
) -> (
    herdr_threads::store::SqliteStore,
    rusqlite::Connection,
    herdr_threads::archival_legacy::Scan,
    herdr_threads::protocol::handoff::BootstrapIdentity,
    herdr_threads::protocol::handoff::BootstrapAttachment,
) {
    use herdr_threads::{
        cli::journal::{BootstrapPlan, Journal, SemanticMutation},
        protocol::{handoff::*, time::UtcMillis},
        store::topology_handoff,
    };
    let child_reference = legacy_compound(paths, context(paths), new_thread);
    let child_scan = complete_scan(source);
    let herdr_threads::archival_legacy::Hint::Handoff {
        identity: child, ..
    } = &child_scan.hints[0]
    else {
        panic!("child")
    };
    let mut id = super::topology_handoff::identity();
    let ctx = context(paths);
    id.payload.handoff.namespace = HandoffNamespace {
        instance: "i".into(),
        state_dir: ctx.state_dir.unwrap().into(),
        host_endpoint: ctx.host.unwrap().into(),
    };
    if new_thread {
        id.payload.handoff.channel = HandoffChannel::New {
            name: None,
            topic: "literal-topic".into(),
            goal: "literal-goal".into(),
        };
    }
    id.payload.handoff_key = child_reference.operation.clone();
    id.digest = id.semantic_digest().unwrap();
    Journal::open(paths.instance_dir.join("intents"))
        .unwrap()
        .record(
            id.scope.clone(),
            SemanticMutation::freeze(
                SemanticMutation::HandoffBootstrap(Box::new(BootstrapPlan {
                    version: 1,
                    payload: id.payload.clone(),
                })),
                id.claim.clone(),
            )
            .unwrap(),
            0,
        )
        .unwrap();
    let scan = complete_scan(source);
    let (store, mut db) = scanner_store(paths);
    let tx = db.transaction().unwrap();
    let ns = &id.payload.handoff.namespace;
    topology_handoff::begin_pending(&tx, ns, &id, UtcMillis(0)).unwrap();
    let attempt = BootstrapAttempt::first();
    topology_handoff::attempts::reserve_attempt(
        &tx,
        ns,
        &ReserveBootstrapAttempt {
            identity: id.clone(),
            expected_attempt: attempt,
            operation: attempt.operation(&id.compound, "reserve").unwrap(),
        },
    )
    .unwrap();
    let mut created = super::topology_handoff::created();
    created.witness.endpoint = ns.host_endpoint.clone();
    topology_handoff::attempts::record_created(
        &tx,
        ns,
        &RecordBootstrapCreated {
            identity: id.clone(),
            expected_attempt: attempt,
            operation: attempt.operation(&id.compound, "record").unwrap(),
            evidence: created.clone(),
        },
    )
    .unwrap();
    let attachment = BootstrapAttachment {
        attempt,
        created,
        resolve_operation: id.payload.resolve_key.clone(),
        resolved_seat: child.recipient.clone(),
        handoff: child.as_ref().clone(),
    };
    // Already-reviewed attachment state is seeded here; actual reserve/record,
    // cancellation, namespace Begin and linked completion writers stay real.
    tx.execute(
        "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(1,1,?1)",
        [serde_json::to_vec(&attachment).unwrap()],
    )
    .unwrap();
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='attached' WHERE id=1",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    (store, db, scan, id, attachment)
}
fn cancel_attached(
    db: &mut rusqlite::Connection,
    id: &herdr_threads::protocol::handoff::BootstrapIdentity,
    a: &herdr_threads::protocol::handoff::BootstrapAttachment,
) -> Result<(), herdr_threads::protocol::results::ApiError> {
    use herdr_threads::{
        protocol::{handoff::*, time::UtcMillis},
        store::topology_handoff,
    };
    let tx = db.transaction().unwrap();
    let mut request = RecoverBootstrap {
        inspection: Some(
            BootstrapRecoveryInspection::from_status(
                &topology_handoff::current(&tx, &id.payload.handoff.namespace, id)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap(),
        ),
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::first(),
        operation: id.compound.clone(),
        disposition: BootstrapRecoveryDisposition::Cancelled {
            reason: "inspected retained creation".into(),
            quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
            child_guard: BootstrapCancellationGuard {
                attached_child: Some(a.handoff.clone()),
            },
        },
    };
    request.operation = request.decision_operation().unwrap();
    topology_handoff::attempts::recover(
        &tx,
        &id.payload.handoff.namespace,
        &request,
        501,
        UtcMillis(2),
        None,
    )?;
    tx.commit().unwrap();
    Ok(())
}
#[test]
fn archival_legacy_cancelled_attached_child_suppresses_delayed_not_yet_live_hint() {
    let (_dir, paths, mut source) = source();
    let (store, mut db, scan, id, a) = linked_archival_fixture(&paths, &mut source, false);
    cancel_attached(&mut db, &id, &a).unwrap();
    import_scan(&store, &scan)
        .expect("exact cancelled attachment must suppress stale not-yet-live child hint");
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM bootstrap_attachments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(store);
    drop(db);
    let store = herdr_threads::store::SqliteStore::new(
        herdr_threads::store::connection::StoreContext::new(
            paths.database_path.clone(),
            std::sync::Arc::new(herdr_threads::app::SystemClock::default()),
        ),
        "i",
        Default::default(),
    )
    .unwrap();
    import_scan(&store, &complete_scan(&mut source)).unwrap();
    let db = rusqlite::Connection::open(&paths.database_path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn archival_legacy_live_attached_child_hint_is_protected_and_refuses_cancel() {
    let (_dir, paths, mut source) = source();
    let (store, mut db, scan, id, a) = linked_archival_fixture(&paths, &mut source, false);
    import_scan(&store, &scan).expect("exact linked local journal may install only a veto hint");
    assert_eq!(
        db.query_row("SELECT origin FROM channel_handoff_fences", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "legacy_local_journal_hint"
    );
    assert!(
        cancel_attached(&mut db, &id, &a).is_err(),
        "live exact hint must block cancellation"
    );
    import_scan(&store, &scan).unwrap();
    assert_eq!(
        herdr_threads::store::handoff::current(&db, &a.handoff)
            .unwrap()
            .unwrap()
            .state,
        herdr_threads::protocol::handoff::HandoffState::Live
    );
}

#[test]
fn archival_legacy_bootstrap_known_progress_is_bounded_exact_and_never_authority() {
    let (_dir, paths, mut source) = source();
    let reference = modern_compound(&paths, true, false);
    let id = bootstrap_hint(&complete_scan(&mut source));
    let path = paths
        .instance_dir
        .join("intents")
        .join(format!("handoff-{}.progress", reference.operation.as_str()));
    let mut created = super::topology_handoff::created();
    created.witness.endpoint = id.payload.handoff.namespace.host_endpoint.clone();
    let request = herdr_threads::ports::CreateTabRequest {
        correlation: created.correlation.clone(),
        workspace: id.payload.workspace.clone(),
        cwd: id.payload.cwd.clone(),
        label: id.payload.label.clone(),
        focus: false,
        env: Default::default(),
        expected_witness: created.witness.clone(),
    };
    let records = [
        serde_json::json!({"version":1,"identity":id,"attempt":1,"possible_creation":false,"request":null,"creation":null,"not_submitted":false}),
        serde_json::json!({"version":1,"identity":id,"attempt":1,"possible_creation":true,"request":request,"creation":null,"not_submitted":false}),
        serde_json::json!({"version":1,"identity":id,"attempt":1,"possible_creation":true,"request":request,"creation":created,"not_submitted":false}),
        serde_json::json!({"version":1,"identity":id,"attempt":1,"possible_creation":true,"request":request,"creation":null,"not_submitted":true}),
    ];
    let (store, db) = scanner_store(&paths);
    for record in &records {
        let bytes = serde_json::to_vec(record).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let scan = complete_scan(&mut source);
        assert!(
            scan.coverage.is_some(),
            "known producer progress must establish structural coverage: {record}"
        );
        import_scan(&store, &scan).unwrap();
        assert!(
            db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
                r.get::<_, bool>(0)
            })
            .unwrap()
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            db.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "local possible_creation must never become submission authority"
        );
    }
    for (pointer, value) in [
        ("/version", serde_json::json!(2)),
        ("/attempt", serde_json::json!(0)),
        ("/possible_creation", serde_json::json!(false)),
        ("/not_submitted", serde_json::json!(true)),
        ("/identity/digest", serde_json::json!("0".repeat(64))),
        ("/request/label", serde_json::json!("other")),
        ("/creation/correlation", serde_json::json!("other")),
        ("/creation/terminal", serde_json::json!("")),
    ] {
        let mut bad = records[2].clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(
            complete_scan(&mut source).coverage.is_none(),
            "bad progress {pointer}"
        );
    }
    let mut bad = records[2].clone();
    bad["request"]["expected_witness"]["unknown"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
    std::fs::write(&path, vec![b'x'; 128 * 1024 + 1]).unwrap();
    assert!(complete_scan(&mut source).coverage.is_none());
}

fn linked_completion(
    id: &herdr_threads::protocol::handoff::BootstrapIdentity,
    a: &herdr_threads::protocol::handoff::BootstrapAttachment,
) -> herdr_threads::protocol::handoff::CompleteLinkedBootstrap {
    use herdr_threads::protocol::{handoff::*, ids::ThreadId};
    use sha2::{Digest, Sha256};
    let ns = &id.payload.handoff.namespace;
    let prompt = format!(
        "Expected handoff command routing (JSON data): null Prefer a startup hook command group only when its instance UUID, canonical state directory and canonical host endpoint exactly match every expected routing field above. Missing (null), different or ambiguous routing cannot supersede this handoff's target. Open your durable inbox using that matching group. Otherwise use the exact fallback: `herdr-threads --state-dir {} --host-endpoint {} inbox`. The task for thread t is stored in inbox; follow its printed next: commands for complete bodies. Do not reread it with read/body. When waiting for replies, finish your turn and let hooks notify you of new mail; do not poll or run follow. Launch does not accept invitations or ACK messages. Accept invitations separately; default text inbox ACKs fully displayed messages.",
        ns.state_dir.display(),
        ns.host_endpoint.display()
    );
    let mut argv = id.payload.launch.argv.clone();
    argv.push(prompt);
    let argv = herdr_threads::harness::launch::compose_native_argv(
        id.payload.launch.harness,
        argv,
        Vec::new(),
    )
    .unwrap();
    let report = serde_json::json!({"outcome":"started","pane":a.created.root_pane,"seat":a.resolved_seat,"harness":"codex","argv":argv});
    CompleteLinkedBootstrap {
        identity: id.clone(),
        attachment: a.clone(),
        operation: id.payload.linked_complete_key.clone(),
        legacy_completion: HandoffMutation {
            identity: a.handoff.clone(),
            operation: id.payload.handoff.keys.complete.clone(),
        },
        retained: LinkedBootstrapReport {
            thread: ThreadId::new("t"),
            recipient: a.resolved_seat.clone(),
            pane: a.created.root_pane.clone(),
            kind: "launch".into(),
            launch: id.payload.launch.clone(),
            report_digest: format!("{:x}", Sha256::digest(serde_json::to_vec(&report).unwrap())),
            report,
            terminal: a.created.terminal.clone(),
            host_incarnation: a.created.host_incarnation.clone(),
        },
    }
}
#[test]
fn archival_legacy_atomic_completed_parent_and_child_dominate_delay_restart_and_corruption() {
    use herdr_threads::{
        protocol::{handoff::HandoffState, time::UtcMillis},
        store::{handoff, topology_handoff},
    };
    let (_dir, paths, mut source) = source();
    let (store, mut db, scan, id, a) = linked_archival_fixture(&paths, &mut source, false);
    let tx = db.transaction().unwrap();
    handoff::begin_linked_pending(&tx, &id.payload.handoff.namespace, &a.handoff, UtcMillis(1))
        .unwrap();
    topology_handoff::complete_linked_pending(
        &tx,
        &id.payload.handoff.namespace,
        &linked_completion(&id, &a),
        UtcMillis(2),
    )
    .unwrap();
    tx.commit().unwrap();
    // Captured before the atomic terminal transaction; no current binding or
    // membership revalidation may revive those historical fences on import.
    db.execute_batch("UPDATE memberships SET state='left'; UPDATE threads SET archived=1; UPDATE seats SET generation=2;").unwrap();
    import_scan(&store, &scan).unwrap();
    drop(store);
    drop(db);
    let store = herdr_threads::store::SqliteStore::new(
        herdr_threads::store::connection::StoreContext::new(
            paths.database_path.clone(),
            std::sync::Arc::new(herdr_threads::app::SystemClock::default()),
        ),
        "i",
        Default::default(),
    )
    .unwrap();
    let db = rusqlite::Connection::open(&paths.database_path).unwrap();
    import_scan(&store, &complete_scan(&mut source)).unwrap();
    assert_eq!(
        handoff::current(&db, &a.handoff).unwrap().unwrap().state,
        HandoffState::Completed
    );
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    db.execute_batch("DROP TRIGGER bootstrap_report_immutable")
        .unwrap();
    db.execute(
        "UPDATE bootstrap_reports SET completed_json=?1",
        [b"{}".as_slice()],
    )
    .unwrap();
    assert!(
        import_scan(&store, &scan).is_err(),
        "corrupt canonical terminal presentation must refuse rather than restore either fence"
    );
    assert_eq!(
        handoff::current(&db, &a.handoff).unwrap().unwrap().state,
        HandoffState::Completed
    );
    assert_eq!(
        db.query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "completed"
    );
}
#[test]
fn archival_legacy_new_thread_hint_resolves_only_exact_atomic_created_key() {
    use herdr_threads::{
        protocol::ids::ThreadId,
        store::{handoff, topology_handoff},
    };
    let (_dir, paths, mut source) = source();
    let (store, mut db, scan, id, a) = linked_archival_fixture(&paths, &mut source, true);
    import_scan(&store, &scan).unwrap();
    assert!(
        handoff::current(&db, &a.handoff)
            .unwrap()
            .unwrap()
            .thread
            .is_none()
    );
    let tx = db.transaction().unwrap();
    topology_handoff::attach_created(
        &tx,
        &id.payload.handoff.namespace,
        "i",
        "seat:s",
        "unrelated",
        &ThreadId::new("t"),
    )
    .unwrap();
    assert!(
        handoff::current(&tx, &a.handoff)
            .unwrap()
            .unwrap()
            .thread
            .is_none()
    );
    topology_handoff::attach_created(
        &tx,
        &id.payload.handoff.namespace,
        "i",
        "seat:s",
        id.payload.handoff.keys.create.as_str(),
        &ThreadId::new("t"),
    )
    .unwrap();
    assert_eq!(
        handoff::current(&tx, &a.handoff)
            .unwrap()
            .unwrap()
            .thread
            .unwrap()
            .as_str(),
        "t"
    );
    assert_eq!(
        tx.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "t",
        "both fences attach inside the same deciding transaction"
    );
    tx.rollback().unwrap();
    assert!(
        handoff::current(&db, &a.handoff)
            .unwrap()
            .unwrap()
            .thread
            .is_none()
    );
    assert_eq!(
        db.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
}
#[test]
fn archival_legacy_many_bootstraps_have_no_cardinality_cap_and_terminal_during_scan_is_current() {
    let (_dir, paths, mut source) = source();
    for _ in 0..33 {
        modern_compound(&paths, true, false);
    }
    let scan = complete_scan(&mut source);
    assert!(
        scan.coverage.is_some(),
        "scanner cardinality does not invent uncertainty"
    );
    assert_eq!(
        scan.hints
            .iter()
            .filter(|hint| matches!(hint, herdr_threads::archival_legacy::Hint::Bootstrap { .. }))
            .count(),
        33
    );
    let id = bootstrap_hint(&scan);
    let (store, mut db) = scanner_store(&paths);
    cancel_bootstrap(&mut db, &id);
    for _ in 0..20 {
        let page = source.scan(|| false).unwrap();
        assert!(page.hints.len() <= herdr_threads::archival_legacy::ENTRIES_PER_PAGE);
        import_scan(&store, &page).unwrap();
        if !page.pending {
            assert!(page.coverage.is_some());
            break;
        }
    }
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap(),
        "more than32 exact canonical terminal journals remain recognizable"
    );
    let (_dir, paths, mut source) = self::source();
    modern_compound(&paths, true, false);
    for n in 0..100 {
        std::fs::write(
            paths
                .instance_dir
                .join("intents")
                .join(format!("handoff-{n}.progress")),
            b"orphan",
        )
        .unwrap();
    }
    let first = source.scan(|| false).unwrap();
    assert!(first.pending);
    assert!(first.coverage.is_none());
    let original =
        herdr_threads::cli::journal::Journal::open(paths.instance_dir.join("intents")).unwrap();
    let reference = original.resolve_recovery_ref("local:1").unwrap();
    let pending = original.load(&reference).unwrap();
    let herdr_threads::cli::journal::SemanticMutation::Frozen { claim, mutation } =
        pending.semantic
    else {
        panic!("frozen")
    };
    let herdr_threads::cli::journal::SemanticMutation::HandoffBootstrap(plan) = *mutation else {
        panic!("bootstrap")
    };
    let id = herdr_threads::protocol::handoff::BootstrapIdentity {
        compound: plan.payload.handoff.keys.compound.clone(),
        scope: pending.header.scope,
        claim,
        digest: pending.header.semantic_digest,
        payload: plan.payload,
    };
    let (store, mut db) = scanner_store(&paths);
    cancel_bootstrap(&mut db, &id);
    let scan = complete_scan(&mut source);
    assert!(scan.coverage.is_some());
    import_scan(&store, &scan).unwrap();
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
}

#[test]
fn archival_legacy_completed_bootstrap_requires_its_exact_canonical_completed_child() {
    use herdr_threads::{
        protocol::time::UtcMillis,
        store::{handoff, topology_handoff},
    };
    let (_dir, paths, mut source) = source();
    let (store, mut db, _scan, id, a) = linked_archival_fixture(&paths, &mut source, false);
    let tx = db.transaction().unwrap();
    handoff::begin_linked_pending(&tx, &id.payload.handoff.namespace, &a.handoff, UtcMillis(1))
        .unwrap();
    topology_handoff::complete_linked_pending(
        &tx,
        &id.payload.handoff.namespace,
        &linked_completion(&id, &a),
        UtcMillis(2),
    )
    .unwrap();
    tx.commit().unwrap();
    let journal =
        herdr_threads::cli::journal::Journal::open(paths.instance_dir.join("intents")).unwrap();
    let child = journal.resolve_recovery_ref("local:1").unwrap();
    journal.complete(&child).unwrap();
    let scan = complete_scan(&mut source);
    assert_eq!(scan.hints.len(), 1);
    // Corruption injection is confined to this owned database; normal terminal
    // triggers prevent this state and are unchanged by the archival importer.
    db.execute_batch("DROP TRIGGER channel_handoff_retained; DELETE FROM channel_handoff_fences")
        .unwrap();
    assert!(
        import_scan(&store, &scan).is_err(),
        "parent report cannot fabricate a missing canonical child completion"
    );
    assert_eq!(
        db.query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "completed"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn archival_legacy_unconfirmed_delivery_terminal_cannot_be_forgotten_between_pages() {
    let (_dir, paths, mut source) = source();
    let reference = modern_compound(&paths, false, false);
    delivery_terminal(&paths, &reference);
    for n in 0..2 {
        std::fs::write(
            paths
                .instance_dir
                .join("intents")
                .join(format!("handoff-orphan-{n}.progress")),
            b"orphan",
        )
        .unwrap();
    }
    let (store, db) = scanner_store(&paths);
    let mut done = false;
    for _ in 0..30 {
        let page = one_entry_page(&mut source);
        import_scan(&store, &page).unwrap();
        if !page.pending {
            assert!(
                page.coverage.is_none(),
                "earlier unconfirmed terminal must veto final source coverage"
            );
            assert!(
                db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
                    r.get::<_, bool>(0)
                })
                .unwrap()
            );
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn archival_legacy_ordinary_import_error_stays_vetoed_across_pages() {
    let (_dir, paths, mut source) = source();
    compound(&paths);
    let baseline = complete_scan(&mut source);
    let herdr_threads::archival_legacy::Hint::Handoff { identity, .. } = &baseline.hints[0] else {
        panic!("handoff")
    };
    let mut mismatched = identity.as_ref().clone();
    mismatched.digest = "a".repeat(64);
    let (store, mut db) = scanner_store(&paths);
    let tx = db.transaction().unwrap();
    herdr_threads::store::handoff::begin_pending(
        &tx,
        &mismatched,
        herdr_threads::protocol::time::UtcMillis(0),
    )
    .unwrap();
    tx.commit().unwrap();
    for n in 0..2 {
        std::fs::write(
            paths
                .instance_dir
                .join("intents")
                .join(format!("handoff-orphan-{n}.progress")),
            b"orphan",
        )
        .unwrap();
    }
    let mut failed = false;
    let mut done = false;
    for _ in 0..30 {
        let page = one_entry_page(&mut source);
        failed |= import_scan(&store, &page).is_err();
        if !page.pending {
            assert!(failed);
            assert!(
                page.coverage.is_none(),
                "earlier canonical identity mismatch cannot disappear on final page"
            );
            done = true;
            break;
        }
    }
    assert!(done);
}

#[test]
fn archival_legacy_queued_final_coverage_observes_earlier_deciding_veto() {
    let (_dir, paths, mut source) = source();
    let reference = modern_compound(&paths, false, false);
    delivery_terminal(&paths, &reference);
    for n in 0..2 {
        std::fs::write(
            paths
                .instance_dir
                .join("intents")
                .join(format!("handoff-orphan-{n}.progress")),
            b"orphan",
        )
        .unwrap();
    }
    let mut pages = Vec::new();
    for _ in 0..30 {
        let page = one_entry_page(&mut source);
        let done = !page.pending;
        pages.push(page);
        if done {
            break;
        }
    }
    assert!(
        pages[..pages.len() - 1]
            .iter()
            .any(|page| page.hints.iter().any(|hint| matches!(
                hint,
                herdr_threads::archival_legacy::Hint::Handoff {
                    retained_completion: Some(_),
                    ..
                }
            ))),
        "fixture must place terminal before final page"
    );
    assert!(
        pages.last().unwrap().coverage.is_some(),
        "sample final coverage before deciding earlier hint"
    );
    let (store, db) = scanner_store(&paths);
    for page in &pages {
        import_scan(&store, page).unwrap();
    }
    assert!(
        db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap(),
        "queued final coverage must observe the earlier deciding veto"
    );
    std::fs::remove_dir_all(paths.instance_dir.join("intents")).unwrap();
    let fresh_absence = source.scan(|| false).unwrap();
    assert!(fresh_absence.coverage.is_some());
    import_scan(&store, &fresh_absence).unwrap();
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    import_scan(&store, pages.last().unwrap()).unwrap();
    assert!(
        db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap(),
        "fresh absent traversal cannot clear a queued old page's marker"
    );
}

#[test]
fn archival_legacy_fresh_traversal_rechecks_terminal_and_generation_without_clearing_old_veto() {
    let (_dir, paths, mut source) = source();
    modern_compound(&paths, true, false);
    let stale = complete_scan(&mut source);
    let id = bootstrap_hint(&stale);
    let generation = stale.coverage.clone().unwrap();
    let (store, mut db) = scanner_store(&paths);
    import_scan(&store, &stale).unwrap();
    cancel_bootstrap(&mut db, &id);
    import_scan(&store, &stale).unwrap();
    assert!(
        db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    let fresh = complete_scan(&mut source);
    assert_eq!(fresh.coverage.as_ref(), Some(&generation));
    import_scan(&store, &fresh).unwrap();
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    import_scan(&store, &stale).unwrap();
    assert!(
        db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap(),
        "old samples can only veto"
    );
    std::fs::remove_dir_all(paths.instance_dir.join("intents")).unwrap();
    let absent = complete_scan(&mut source);
    assert_ne!(absent.coverage, Some(generation));
    import_scan(&store, &absent).unwrap();
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    assert_eq!(
        db.query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "cancelled"
    );
}

#[test]
fn archival_legacy_budget_admission_error_vetoes_queued_coverage_but_fresh_scan_recovers() {
    use herdr_threads::{
        ports::StorePort,
        protocol::time::{CallBudget, MonoInstant, UtcMillis},
        store::archival::Runtime,
    };
    let (_dir, paths, mut source) = source();
    compound(&paths);
    let sample = complete_scan(&mut source);
    let (store, db) = scanner_store(&paths);
    let rt = Runtime {
        boot: "archive-test".into(),
        mono: 0,
        utc: UtcMillis(0),
        after_ms: 60000,
        host_generation: 0,
        coherent: false,
        valid_until_mono: None,
        legacy_source: sample.coverage.clone(),
    };
    let budget = CallBudget {
        deadline: MonoInstant(store.clock().monotonic_now().0 + 5000),
        cancellation: Default::default(),
    };
    budget.cancellation.cancel();
    assert!(store.archival_pass(&rt, &sample.hints, &budget).is_err());
    import_scan(&store, &sample).unwrap();
    assert!(
        db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
    let fresh = complete_scan(&mut source);
    import_scan(&store, &fresh).unwrap();
    assert!(
        !db.query_row("SELECT bootstrap_veto FROM archival_instances", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap()
    );
}

#[test]
fn archival_legacy_modern_header_rejects_nonjournal_reference() {
    for bootstrap in [false, true] {
        for malformed in ["ordinal", "operation"] {
            let (_dir, paths, mut source) = source();
            let reference = modern_compound(&paths, bootstrap, false);
            let original = paths.instance_dir.join("intents").join(format!(
                "{:020}-{}.intent",
                reference.ordinal,
                reference.operation.as_str()
            ));
            let bytes = std::fs::read(&original).unwrap();
            let split = bytes.iter().position(|b| *b == b'\n').unwrap();
            let mut header: serde_json::Value = serde_json::from_slice(&bytes[..split]).unwrap();
            if malformed == "ordinal" {
                header["reference"]["ordinal"] = serde_json::json!(0);
            } else {
                header["reference"]["operation"] = serde_json::json!("not-a-journal-uuid");
            }
            let mut changed = serde_json::to_vec(&header).unwrap();
            changed.push(b'\n');
            changed.extend_from_slice(&bytes[split + 1..]);
            std::fs::remove_file(original).unwrap();
            let filename = format!(
                "{:020}-{}.intent",
                header["reference"]["ordinal"].as_u64().unwrap(),
                header["reference"]["operation"].as_str().unwrap()
            );
            std::fs::write(paths.instance_dir.join("intents").join(filename), changed).unwrap();
            let scan = complete_scan(&mut source);
            assert!(
                scan.coverage.is_none(),
                "invalid journal reference cannot establish modern coverage: {bootstrap} {malformed}"
            );
            assert!(scan.hints.is_empty());
        }
    }
}

#[test]
fn archival_legacy_main539_join_intent_vetoes_without_authorizing_or_rewriting() {
    use herdr_threads::cli::journal::{IntentScope, Journal, SemanticMutation};
    let (_dir, paths, mut source) = source();
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let id = super::handoff_fences::identity();
    let reference = journal
        .record(
            IntentScope::Cooperative {
                instance: "i".into(),
                seat: id.claim.seat.clone(),
            },
            SemanticMutation::freeze(
                SemanticMutation::Join {
                    thread: herdr_threads::protocol::ids::ThreadId::new("t"),
                },
                id.claim,
            )
            .unwrap(),
            0,
        )
        .unwrap();
    let path = paths.instance_dir.join("intents").join(format!(
        "{:020}-{}.intent",
        reference.ordinal,
        reference.operation.as_str()
    ));
    let before = std::fs::read(&path).unwrap();
    let scan = complete_scan(&mut source);
    assert!(scan.coverage.is_none());
    assert!(scan.hints.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    journal.complete(&reference).unwrap();
    assert!(complete_scan(&mut source).coverage.is_some());
}
