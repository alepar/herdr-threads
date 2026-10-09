use herdr_threads::{
    archival_legacy::Source,
    daemon::paths::{InstancePaths, RuntimeContext},
    protocol::output::ContinuationContext,
};
struct Temp(std::path::PathBuf);
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
fn source() -> (Temp, InstancePaths, Source) {
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
fn compound(paths: &InstancePaths) -> herdr_threads::cli::journal::IntentRef {
    compound_in(paths, context(paths))
}
fn compound_in(
    paths: &InstancePaths,
    context: ContinuationContext,
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
            thread: Some(ThreadId::new("t")),
            thread_name: None,
            topic: None,
            goal: None,
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
    assert_eq!(scan.hints[0].identity.compound, reference.operation);
    assert_eq!(
        scan.hints[0].identity.thread.as_ref().unwrap().as_str(),
        "t"
    );
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
        assert_eq!(scan.hints[0].identity.digest, header.semantic_digest);
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
    let hint = scan.hints.pop().unwrap();
    let mut db = super::channel_archival::fixture();
    super::channel_archival::thread(&db);
    super::channel_archival::joined_agent(&db);
    let tx = db.transaction().unwrap();
    herdr_threads::store::handoff::begin_pending(
        &tx,
        &hint.identity,
        herdr_threads::protocol::time::UtcMillis(0),
    )
    .unwrap();
    herdr_threads::store::handoff::complete_pending(
        &tx,
        &hint.identity,
        herdr_threads::protocol::time::UtcMillis(1),
    )
    .unwrap();
    tx.commit().unwrap();
    let tx = db.transaction().unwrap();
    let result = herdr_threads::store::handoff::import_hint(
        &tx,
        &hint.identity,
        hint.progress_thread.as_ref(),
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
