//! The merged lazy-delivery model (2026-10-10): a hook-pull seat's lazy rows complete only
//! through `DeliveryConfirm` (`cooperative_hook_delivery`) or its own text inbox; the daemon
//! refuses `CompleteInboxDelivery { via: hook_context }` from a hook-pull binding, while a
//! Claude seat completes through that hook claim as in main v0.6.0. Same `DomainService` +
//! temp `SqliteStore` setup as `delivery_pull`.
use crate::{
    ports::{LocalService, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{CheckIn, CheckInMode, Command, CompleteInboxDelivery, LazyCompletionVia},
        delivery::{DeliveryConfirm, DeliveryConfirmOutcome, DeliveryPull, DeliveryPullReply},
        ids::*,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::{Connection, params};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

struct Fixture {
    service: DomainService,
    db: Connection,
    clock: Arc<TestClock>,
    directory: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn peer() -> PeerIdentity {
    PeerIdentity::from_kernel(501)
}
fn fourth() -> Harness {
    Harness::Agent(
        crate::harness::registry::builtins()
            .agent("synthetic_fourth")
            .unwrap(),
    )
}
fn claude() -> Harness {
    Harness::Agent(
        crate::harness::registry::builtins()
            .agent("claude")
            .unwrap(),
    )
}

/// Seats `s` (synthetic_fourth, hook pull) and `c` (Claude); thread `t` with both joined.
fn fixture() -> Fixture {
    use std::os::unix::fs::DirBuilderExt;
    let directory =
        crate::test_support::scratch::temp_path(format!("lazy-hook-pull-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let shared: Arc<dyn Clock> = clock.clone();
    let context = StoreContext::new(directory.join("store.db"), shared.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,3)",
        [],
    )
    .unwrap();
    for (seat, target) in [("s", "p"), ("c", "pc")] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,1,0,0)", [seat, target]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,1)", []).unwrap();
    for seat in ["s", "c"] {
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    let service = DomainService::new("i".into(), store, shared)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
    Fixture {
        service,
        db,
        clock,
        directory,
    }
}

/// A published lazy message addressed to `seat` (state `pending`).
fn lazy_message(f: &Fixture, seat: &str, id: &str, body: &str) {
    let sequence: i64 =
        f.db.query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let prep = format!("prep-{}", id.trim_start_matches("msg-"));
    f.db.execute(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES (?1,'i','actor',?1,zeroblob(32),'t',0,0,0,0,0,0,1,'building','lazy')",
        [&prep],
    )
    .unwrap();
    f.db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,'t',?3)", params![prep, id, seat]).unwrap();
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,delivery_mode) VALUES (?1,'i','t',?2,'ordinary',?3,0,?4,'lazy')", params![id, sequence, body, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'t',?3,0,?4,0,1,0)", params![prep, id, sequence + 2, sequence]).unwrap();
    f.db.execute(
        "UPDATE lazy_recipients SET decision_seq=?2 WHERE preparation_id=?1 AND decision_seq IS NULL",
        params![prep, sequence + 2],
    )
    .unwrap();
    f.db.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [sequence + 1],
    )
    .unwrap();
    f.db.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [sequence + 3],
    )
    .unwrap();
}

/// An ordinary message with a pending receipt for `seat`, receivable (a sealed preparation, a
/// prepared recipient and a manifest), at the next sequence: a cursor cannot pass it unshown.
fn receivable_ordinary(f: &Fixture, seat: &str, id: &str, body: &str) {
    let sequence: i64 =
        f.db.query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let prep = format!("prep-{id}");
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary',?3,0,?4)", params![id, sequence, body, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t',?2,'pending',300000)", params![id, seat]).unwrap();
    f.db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES (?1,'i','actor',?1,zeroblob(32),'t',0,0,0,0,0,0,1,'sealed')", [&prep]).unwrap();
    f.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES (?1,'t',?2,1,300000,1)", params![prep, seat]).unwrap();
    f.db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'t',?3,0,?4,0,1,0)", params![prep, id, sequence + 2, sequence]).unwrap();
    f.db.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [sequence + 1],
    )
    .unwrap();
    f.db.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [sequence + 3],
    )
    .unwrap();
}

fn lazy_state(f: &Fixture, seat: &str, id: &str) -> String {
    f.db.query_row(
        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
        [id, seat],
        |r| r.get(0),
    )
    .unwrap()
}

fn check_in(f: &Fixture, seat: &str, target: &str, harness: Harness) -> CallerClaim {
    // One harness process per pane: a seat's sessions share it.
    check_in_as(
        f,
        seat,
        target,
        harness,
        "n1",
        1000 + seat.bytes().map(u32::from).sum::<u32>(),
    )
}

/// A top-level check-in of session `session` on process `pid`. Two bindings of one seat need two
/// processes: a shared pid makes the second lifecycle claim supersede the first (a `/clear`).
fn check_in_as(
    f: &Fixture,
    seat: &str,
    target: &str,
    harness: Harness,
    session: &str,
    pid: u32,
) -> CallerClaim {
    let claim = CallerClaim {
        instance: "i".into(),
        seat: SeatId::new(seat),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new(format!("00000000-0000-4000-8000-{pid:012}")),
        target: HostTargetId::new(target),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint { pid, start: 1 }),
    };
    let command = Command::CheckIn(CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: 1,
        },
        claim,
        operation: OperationId::new(format!("ci-{seat}-{session}")),
        lineage_parent: None,
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    }
}

fn complete(
    f: &Fixture,
    claim: &CallerClaim,
    id: &str,
    operation: &str,
    via: Option<LazyCompletionVia>,
) -> Result<CommandResult, crate::protocol::results::ApiError> {
    f.service.handle(
        Command::CompleteInboxDelivery(CompleteInboxDelivery {
            messages: vec![MessageId::new(id)],
            operation: OperationId::new(operation),
            claim: claim.clone(),
            via,
        }),
        peer(),
        &budget(),
    )
}

fn offered(f: &Fixture, claim: &CallerClaim) -> Vec<MessageId> {
    let command = Command::DeliveryPull(DeliveryPull {
        claim: claim.clone(),
        seen_attention_version: None,
        marker_prefix: vec![],
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::DeliveryPulled(DeliveryPullReply::Page(page)) => page.offered,
        CommandResult::DeliveryPulled(
            DeliveryPullReply::Refused { .. } | DeliveryPullReply::Unchanged { .. },
        ) => vec![],
        other => panic!("unexpected {other:?}"),
    }
}

/// Kills: a hook-context completion accepted from a hook-pull binding (a second completion
/// source for the same row), a pull that re-offers a settled row, and a confirm that does not
/// settle the lazy row.
#[test]
fn hook_pull_lazy_row_is_delivered_exactly_once_through_pull() {
    let f = fixture();
    // A fresh binding's lazy cursor starts at the thread head (session-keyed bindings): the
    // session checks in before the row is published.
    let seat = check_in(&f, "s", "p", fourth());
    lazy_message(&f, "s", "msg-lz1", "passive text");
    assert_eq!(offered(&f, &seat), vec![MessageId::new("msg-lz1")]);

    let refused = complete(
        &f,
        &seat,
        "msg-lz1",
        "hc-1",
        Some(LazyCompletionVia::HookContext),
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidRequest);
    assert_eq!(
        refused.detail,
        "a hook-pull seat completes lazy rows through DeliveryConfirm"
    );
    assert_eq!(lazy_state(&f, "s", "msg-lz1"), "pending");

    let confirmed = f
        .service
        .handle(
            Command::DeliveryConfirm(DeliveryConfirm {
                claim: seat.clone(),
                ids: vec![MessageId::new("msg-lz1")],
                operation: OperationId::new("dc-1"),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryConfirmed(report) = confirmed else {
        panic!("unexpected {confirmed:?}");
    };
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].result, DeliveryConfirmOutcome::Settled);
    assert_eq!(lazy_state(&f, "s", "msg-lz1"), "displayed");

    assert!(
        offered(&f, &seat).is_empty(),
        "a settled row is never offered again"
    );
    let again = complete(
        &f,
        &seat,
        "msg-lz1",
        "hc-2",
        Some(LazyCompletionVia::HookContext),
    )
    .unwrap_err();
    assert_eq!(again.code, ErrorCode::InvalidRequest);
}

/// Kills: a guard that also refuses the text inbox display (`via` absent), which is the agent
/// asking and stays a completion source as in main.
#[test]
fn hook_pull_seat_text_inbox_completion_still_works() {
    let f = fixture();
    lazy_message(&f, "s", "msg-lz2", "passive text");
    let seat = check_in(&f, "s", "p", fourth());
    complete(&f, &seat, "msg-lz2", "inbox-1", None).unwrap();
    assert_eq!(lazy_state(&f, "s", "msg-lz2"), "displayed");
}

/// Kills: a guard keyed on anything but the claim's hook-pull policy (a Claude seat's hook
/// completion would then be refused and main v0.6.0 broken).
#[test]
fn claude_seat_hook_context_completion_per_main() {
    let f = fixture();
    lazy_message(&f, "c", "msg-lz3", "passive text");
    let seat = check_in(&f, "c", "pc", claude());
    complete(
        &f,
        &seat,
        "msg-lz3",
        "hc-1",
        Some(LazyCompletionVia::HookContext),
    )
    .unwrap();
    assert_eq!(lazy_state(&f, "c", "msg-lz3"), "displayed");
}

/// Kills: a hook-pull confirm that settles an ordinary message and a lazy row yet leaves the
/// binding's cursor before them (the lazy row then stays unread and is offered again).
#[test]
fn hook_pull_confirm_of_ordinary_and_lazy_advances_the_cursor_and_does_not_reoffer() {
    use crate::store::{lazy_delivery, read_cursors};
    let f = fixture();
    let seat = check_in(&f, "s", "p", fourth());
    receivable_ordinary(&f, "s", "msg-o1", "ordinary text");
    lazy_message(&f, "s", "msg-lz4", "passive text");
    let mut first = offered(&f, &seat);
    first.sort();
    assert_eq!(
        first,
        vec![MessageId::new("msg-lz4"), MessageId::new("msg-o1")]
    );

    let confirmed = f
        .service
        .handle(
            Command::DeliveryConfirm(DeliveryConfirm {
                claim: seat.clone(),
                ids: vec![MessageId::new("msg-o1"), MessageId::new("msg-lz4")],
                operation: OperationId::new("dc-both"),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryConfirmed(report) = confirmed else {
        panic!("unexpected {confirmed:?}");
    };
    assert_eq!(report.results.len(), 2);
    assert!(
        report
            .results
            .iter()
            .all(|r| r.result == DeliveryConfirmOutcome::Settled)
    );

    let binding = BindingId::new(
        f.db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    );
    let lazy_sequence: i64 =
        f.db.query_row(
            "SELECT sequence FROM messages WHERE id='msg-lz4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // The ordinary row and the lazy row are the thread's only receivable messages; the cursor
    // walks both and then the non-receivable ack events the settlement appended after them, so it
    // ends exactly at the thread's last sequence.
    let thread_tail: i64 =
        f.db.query_row(
            "SELECT max(sequence) FROM messages WHERE thread_id='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        thread_tail > lazy_sequence,
        "the settlement appended ack events after the lazy row"
    );
    assert_eq!(
        read_cursors::cursor_for(&f.db, &binding, &ThreadId::new("t")).unwrap(),
        Some(u64::try_from(thread_tail).unwrap())
    );
    assert!(!read_cursors::has_unread(&f.db, &binding).unwrap());
    assert_eq!(
        lazy_delivery::lazy_completion_state(
            &f.db,
            "i",
            &SeatId::new("s"),
            Some(&binding),
            &MessageId::new("msg-lz4")
        )
        .unwrap(),
        lazy_delivery::LazyCompletion::Completed
    );
    f.clock.0.fetch_add(
        crate::delivery::ledger::REOFFER_AFTER_MS + 1,
        Ordering::SeqCst,
    );
    assert!(
        offered(&f, &seat).is_empty(),
        "nothing is re-offered once the cursor passed both"
    );
}

fn confirm(f: &Fixture, claim: &CallerClaim, id: &str, operation: &str) -> DeliveryConfirmOutcome {
    let confirmed = f
        .service
        .handle(
            Command::DeliveryConfirm(DeliveryConfirm {
                claim: claim.clone(),
                ids: vec![MessageId::new(id)],
                operation: OperationId::new(operation),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryConfirmed(report) = confirmed else {
        panic!("unexpected {confirmed:?}");
    };
    assert_eq!(report.results.len(), 1);
    report.results[0].result
}

/// Seat `s` with two connected top-level bindings, `na` and `nb`.
fn two_bindings(f: &Fixture) -> (CallerClaim, CallerClaim) {
    let a = check_in_as(f, "s", "p", fourth(), "na", 2001);
    let b = check_in_as(f, "s", "p", fourth(), "nb", 2002);
    let connected: i64 =
        f.db.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='s' AND liveness='connected'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(connected, 2, "both bindings are connected");
    (a, b)
}

fn binding_of(f: &Fixture, claim: &CallerClaim) -> BindingId {
    BindingId::new(
        f.db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s' AND native_session=?1 AND ended_at IS NULL",
            [claim.native_session.as_str()],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
}

/// The receipt's state and who settled it, when and at which generation: a second settlement
/// would change any of them.
fn receipt_row(f: &Fixture, id: &str) -> (String, Option<i64>, Option<String>, Option<i64>) {
    f.db.query_row(
        "SELECT state, acked_at, ack_binding_id, ack_generation FROM receipt_state WHERE message_id=?1 AND seat_id='s'",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

/// Kills: a hook pull that reads the seat (`claim: None`), so a message one binding settled never
/// reaches its sibling, and a confirm of it that records a second settlement.
#[test]
fn hook_pull_ordinary_settled_by_a_sibling_still_reaches_the_other_binding() {
    use crate::store::read_cursors;
    let f = fixture();
    let (a, b) = two_bindings(&f);
    receivable_ordinary(&f, "s", "msg-o2", "x");
    assert_eq!(offered(&f, &a), vec![MessageId::new("msg-o2")]);
    assert_eq!(
        confirm(&f, &a, "msg-o2", "dc-a"),
        DeliveryConfirmOutcome::Settled
    );
    let before = receipt_row(&f, "msg-o2");
    assert_eq!(before.0, "acked");

    assert_eq!(
        offered(&f, &b),
        vec![MessageId::new("msg-o2")],
        "B has not read it: its own stream still carries the message A settled"
    );
    assert_eq!(
        confirm(&f, &b, "msg-o2", "dc-b"),
        DeliveryConfirmOutcome::AlreadySettled
    );
    let binding_b = binding_of(&f, &b);
    let tail: i64 =
        f.db.query_row(
            "SELECT max(sequence) FROM messages WHERE thread_id='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        read_cursors::cursor_for(&f.db, &binding_b, &ThreadId::new("t")).unwrap(),
        Some(u64::try_from(tail).unwrap())
    );
    assert!(!read_cursors::has_unread(&f.db, &binding_b).unwrap());
    assert_eq!(
        receipt_row(&f, "msg-o2"),
        before,
        "B's display records no second settlement"
    );
    f.clock.0.fetch_add(
        crate::delivery::ledger::REOFFER_AFTER_MS + 1,
        Ordering::SeqCst,
    );
    assert!(offered(&f, &b).is_empty());
}

/// Kills: a late first pull that finds an empty page because the seat-level receipt settled.
#[test]
fn hook_pull_late_first_pull_after_a_sibling_settled_delivers() {
    let f = fixture();
    let (a, b) = two_bindings(&f);
    receivable_ordinary(&f, "s", "msg-o2", "x");
    assert_eq!(offered(&f, &a), vec![MessageId::new("msg-o2")]);
    assert_eq!(
        confirm(&f, &a, "msg-o2", "dc-a"),
        DeliveryConfirmOutcome::Settled
    );
    f.clock.0.fetch_add(10 * 60 * 1000, Ordering::SeqCst);
    assert_eq!(offered(&f, &b), vec![MessageId::new("msg-o2")]);
}

/// Kills: a seat-union lazy page that re-shows B a row B completed while A still has it unread.
#[test]
fn hook_pull_lazy_row_completed_by_one_binding_is_not_reoffered_to_it() {
    let f = fixture();
    let (a, b) = two_bindings(&f);
    lazy_message(&f, "s", "msg-lz5", "p");
    assert_eq!(offered(&f, &b), vec![MessageId::new("msg-lz5")]);
    assert_eq!(
        confirm(&f, &b, "msg-lz5", "dc-lz-b"),
        DeliveryConfirmOutcome::Settled
    );
    f.clock.0.fetch_add(
        crate::delivery::ledger::REOFFER_AFTER_MS + 1,
        Ordering::SeqCst,
    );
    assert!(
        offered(&f, &b).is_empty(),
        "B completed it; A having it unread must not show it to B again"
    );
    assert_eq!(
        offered(&f, &a),
        vec![MessageId::new("msg-lz5")],
        "A's own stream still has it"
    );
}
