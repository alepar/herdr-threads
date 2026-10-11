//! Daemon-side hook-pull delivery (delivery-policy DP4-DP6, ht-poh.3.4): the
//! `DeliveryPull` page, the offered ledger and `DeliveryConfirm`, driven through
//! `DomainService` over a temp `SqliteStore` with the test-support
//! `synthetic_fourth` adapter (`HookPull`, `cooperative_hook_delivery`) as the
//! pull seat.
use super::*;
use crate::{
    delivery::{
        HEADER,
        ledger::{MAX_OFFERS_PER_GENERATION, REOFFER_AFTER_MS},
    },
    ports::{HookPullOutlook, LocalService, NextPull, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{CheckIn, CheckInMode, Command},
        delivery::{
            DeliveryConfirm, DeliveryConfirmOutcome as Outcome, DeliveryConfirmReason as Reason,
            DeliveryPull, DeliveryPullReply, DeliveryRefusalReason as Refusal,
        },
        ids::*,
        results::CommandResult,
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
        watch::WATCH_BODY_LIMIT_BYTES,
    },
    service::{delivery_pull::DeliveryPullState, dispatch::DomainService, fair_writer::FairWriter},
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
    store: Arc<dyn StorePort>,
    /// The offered ledger the service and (in production) the wake lane share.
    state: Arc<DeliveryPullState>,
    clock: Arc<TestClock>,
    db: Connection,
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

/// Seats `s` (synthetic_fourth), `x` (Codex), `c` (Claude); thread `t` with all
/// three joined. `s` has pending ordinary messages `m1`..`m<ordinary>`.
fn fixture(ordinary: usize) -> Fixture {
    use std::os::unix::fs::DirBuilderExt;
    let directory = std::env::temp_dir().join(format!("delivery-pull-{}", uuid::Uuid::new_v4()));
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
    for (seat, target) in [("s", "p"), ("x", "px"), ("c", "pc")] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,0,0,0)", [seat, target]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,1)", []).unwrap();
    for seat in ["s", "x", "c"] {
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    let state = Arc::new(DeliveryPullState::default());
    let service = DomainService::new("i".into(), Arc::clone(&store), shared)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(Arc::clone(&state));
    let f = Fixture {
        service,
        store,
        state,
        clock,
        db,
        directory,
    };
    for n in 1..=ordinary {
        ordinary_message(&f, &format!("m{n}"), "hello");
    }
    f
}

/// A daemon restart: a fresh service (and so a fresh process-local ledger) over the same store.
fn restart(f: &mut Fixture) {
    let shared: Arc<dyn Clock> = f.clock.clone();
    f.state = Arc::new(DeliveryPullState::default());
    f.service = DomainService::new("i".into(), Arc::clone(&f.store), shared)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(Arc::clone(&f.state));
}
/// The seat's wake attention counter (the store bumps it for every new pending source; 0 without a row).
fn wake_attention(f: &Fixture) -> u64 {
    use rusqlite::OptionalExtension;
    f.db.query_row(
        "SELECT attention_version FROM wake_work WHERE seat_id='s'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .unwrap()
    .unwrap_or(0) as u64
}
/// What the store does for a new pending source: move the seat's wake attention counter (the fixture inserts
/// messages directly, so it must be moved by hand).
fn bump_wake_attention(f: &Fixture) {
    f.db.execute(
        "INSERT INTO wake_work(seat_id,attention_version) VALUES ('s',1) ON CONFLICT(seat_id) DO UPDATE SET attention_version=attention_version+1",
        [],
    )
    .unwrap();
}

/// Next free message sequence (also used for the decision sequence).
fn next_sequence(db: &Connection) -> i64 {
    db.query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn bump(db: &Connection, sequence: i64) {
    db.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [sequence + 1],
    )
    .unwrap();
    db.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [sequence + 3],
    )
    .unwrap();
}
fn ordinary_message(f: &Fixture, id: &str, body: &str) {
    let sequence = next_sequence(&f.db);
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary',?3,0,?4)", params![id, sequence, body, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t','s','pending',300000)", [id]).unwrap();
    bump(&f.db, sequence);
}
/// A published lazy message addressed to `s` (state `pending`).
fn lazy_message(f: &Fixture, id: &str, body: &str) {
    let sequence = next_sequence(&f.db);
    let prep = format!("prep-{}", id.trim_start_matches("msg-"));
    f.db.execute(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES (?1,'i','actor',?1,zeroblob(32),'t',0,0,0,0,0,0,1,'building','lazy')",
        [&prep],
    )
    .unwrap();
    f.db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,'t','s')", params![prep, id]).unwrap();
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,delivery_mode) VALUES (?1,'i','t',?2,'ordinary',?3,0,?4,'lazy')", params![id, sequence, body, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'t',?3,0,?4,0,1,0)", params![prep, id, sequence + 2, sequence]).unwrap();
    bump(&f.db, sequence);
}

fn claim(
    seat: &str,
    target: &str,
    harness: Harness,
    session: &str,
    generation: u64,
) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new(seat),
        binding_generation: generation,
        role: CallerRole::TopLevel,
        harness,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new(target),
    }
}
fn fresh_execution(mut claim: CallerClaim, n: u8) -> CallerClaim {
    claim.execution = ExecutionId::new(format!("00000000-0000-4000-8000-0000000000{n:02}"));
    claim
}
/// Lifecycle check-in; returns the claim the daemon hands back.
fn check_in(f: &Fixture, claim: CallerClaim, operation: &str) -> CallerClaim {
    let command = Command::CheckIn(CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: claim.binding_generation,
        },
        claim,
        operation: OperationId::new(operation),
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    }
}
/// The pull seat `s`, checked in as `synthetic_fourth` in native session `n1`.
fn pull_seat(f: &Fixture) -> CallerClaim {
    check_in(f, claim("s", "p", fourth(), "n1", 0), "ci-1")
}
/// Rotate `s` to a new native session (a `/clear`) or the same one (a resume).
fn rotate(f: &Fixture, previous: &CallerClaim, session: &str, n: u8) -> CallerClaim {
    check_in(
        f,
        fresh_execution(
            claim("s", "p", fourth(), session, previous.binding_generation),
            n,
        ),
        &format!("ci-{n}"),
    )
}

fn pull(f: &Fixture, claim: &CallerClaim, seen: Option<u64>) -> DeliveryPullReply {
    let command = Command::DeliveryPull(DeliveryPull {
        claim: claim.clone(),
        seen_attention_version: seen,
        marker_prefix: vec![],
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::DeliveryPulled(reply) => reply,
        other => panic!("unexpected {other:?}"),
    }
}
fn page(reply: DeliveryPullReply) -> crate::protocol::delivery::DeliveryPage {
    match reply {
        DeliveryPullReply::Page(page) => page,
        other => panic!("expected a page, got {other:?}"),
    }
}
fn refusal(reply: DeliveryPullReply) -> Refusal {
    match reply {
        DeliveryPullReply::Refused { reason } => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}
fn ids(list: &[&str]) -> Vec<MessageId> {
    list.iter().map(|id| MessageId::new(*id)).collect()
}
fn confirm(
    f: &Fixture,
    claim: &CallerClaim,
    list: &[&str],
    operation: &str,
) -> Vec<(String, Outcome, Option<Reason>)> {
    let command = Command::DeliveryConfirm(DeliveryConfirm {
        claim: claim.clone(),
        ids: ids(list),
        operation: OperationId::new(operation),
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::DeliveryConfirmed(report) => report
            .results
            .into_iter()
            .map(|item| (item.id.as_str().to_owned(), item.result, item.reason))
            .collect(),
        other => panic!("unexpected {other:?}"),
    }
}
fn receipt_state(f: &Fixture, id: &str) -> String {
    f.db.query_row(
        "SELECT state FROM receipts WHERE message_id=?1 AND seat_id='s'",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
fn lazy_state(f: &Fixture, id: &str) -> String {
    f.db.query_row(
        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='s'",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
fn advance(f: &Fixture, ms: i64) {
    f.clock.0.fetch_add(ms, Ordering::SeqCst);
}

#[test]
fn pull_returns_a_framed_page_whose_ordinary_ids_settle_on_confirm() {
    let f = fixture(2);
    let context = pull_seat(&f);
    let page = page(pull(&f, &context, None));
    assert!(page.text.starts_with(HEADER), "{}", page.text);
    assert!(!page.text.starts_with('/'));
    assert!(page.text.contains("[herdr-threads] message m1 in "));
    assert_eq!(page.offered, ids(&["m1", "m2"]));
    assert!(page.truncated.is_empty());
    assert!(!page.more);
    assert_eq!(receipt_state(&f, "m1"), "pending", "a pull settles nothing");

    let report = confirm(&f, &context, &["m1", "m2"], "op-1");
    assert_eq!(
        report,
        vec![
            ("m1".to_owned(), Outcome::Settled, None),
            ("m2".to_owned(), Outcome::Settled, None)
        ]
    );
    assert_eq!(receipt_state(&f, "m1"), "acked");
    let observation: String =
        f.db.query_row(
            "SELECT ack_observation FROM receipts WHERE message_id='m1' AND seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let observation: serde_json::Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(
        observation["action_provenance"],
        "cooperative_hook_delivery"
    );
    assert_eq!(observation["via"], "transcript");
    assert_eq!(observation["provenance"], "cooperative_top_level");
}

#[test]
fn a_lazy_row_is_offered_framed_and_completes_on_confirm() {
    let f = fixture(0);
    lazy_message(&f, "msg-lz1", "passive text");
    let context = pull_seat(&f);
    let page = page(pull(&f, &context, None));
    assert!(
        page.text.contains("[herdr-threads] lazy msg-lz1 in "),
        "{}",
        page.text
    );
    assert_eq!(page.offered, ids(&["msg-lz1"]));
    assert_eq!(lazy_state(&f, "msg-lz1"), "pending");
    let report = confirm(&f, &context, &["msg-lz1"], "op-1");
    assert_eq!(report, vec![("msg-lz1".to_owned(), Outcome::Settled, None)]);
    assert_eq!(lazy_state(&f, "msg-lz1"), "displayed");
    // A repeat confirm is answered from the store: already settled.
    let again = confirm(&f, &context, &["msg-lz1"], "op-2");
    assert_eq!(
        again[0].1,
        Outcome::NotOffered,
        "settled ids left the ledger"
    );
}

#[test]
fn a_second_pull_with_the_returned_version_is_unchanged() {
    let f = fixture(2);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    let second = pull(&f, &context, Some(first.attention_version));
    assert_eq!(
        second,
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
    // Without the version the unconfirmed items are not shown again either
    // (the 60 s re-offer bound), but the version is not trusted blindly: a
    // stale version gets the same answer only once nothing is due.
    let stale = pull(&f, &context, Some(first.attention_version ^ 1));
    assert_eq!(
        stale,
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        },
        "offered items are skipped until due"
    );
    // New pending work moves the version.
    ordinary_message(&f, "m3", "later");
    let third = page(pull(&f, &context, Some(first.attention_version)));
    assert_ne!(third.attention_version, first.attention_version);
    assert_eq!(third.offered, ids(&["m3"]));
}

#[test]
fn more_than_a_page_of_blocks_sets_more_and_the_next_pull_continues() {
    let f = fixture(40);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert!(first.more);
    assert_eq!(first.offered.len(), 32);
    assert_eq!(first.offered[0], MessageId::new("m1"));
    // Same version, but `more_pending` forbids the Unchanged short-circuit.
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(!second.more);
    assert_eq!(
        second.offered.len(),
        8,
        "the remaining blocks, oldest first"
    );
    assert_eq!(second.offered[0], MessageId::new("m33"));
    let done = pull(&f, &context, Some(second.attention_version));
    assert!(matches!(done, DeliveryPullReply::Unchanged { .. }));
}

#[test]
fn an_unconfirmed_id_is_reoffered_after_sixty_seconds_at_most_three_times() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert_eq!(first.offered, ids(&["m1"]));
    let seen = Some(first.attention_version);

    advance(&f, REOFFER_AFTER_MS - 1);
    assert!(matches!(
        pull(&f, &context, seen),
        DeliveryPullReply::Unchanged { .. }
    ));
    advance(&f, 1);
    let second = page(pull(&f, &context, seen));
    assert_eq!(second.offered, ids(&["m1"]), "due after 60 s");
    // Not again straight away: the 60 s runs from the last showing.
    assert!(matches!(
        pull(&f, &context, seen),
        DeliveryPullReply::Unchanged { .. }
    ));
    advance(&f, REOFFER_AFTER_MS);
    let third = page(pull(&f, &context, seen));
    assert_eq!(third.offered, ids(&["m1"]));
    // Parked after the third showing.
    advance(&f, 10 * REOFFER_AFTER_MS);
    assert!(matches!(
        pull(&f, &context, seen),
        DeliveryPullReply::Unchanged { .. }
    ));
    assert_eq!(receipt_state(&f, "m1"), "pending", "it waits for inbox");
}

#[test]
fn a_rotated_generation_discards_offers() {
    let f = fixture(1);
    let first = pull_seat(&f);
    page(pull(&f, &first, None));
    // /clear: a new native session rotates the generation.
    let second = rotate(&f, &first, "n2", 2);
    assert!(second.binding_generation > first.binding_generation);
    // The new generation never saw the offer.
    assert_eq!(
        confirm(&f, &second, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
    // The old generation's claim is stale (a different native session).
    assert_eq!(
        confirm(&f, &first, &["m1"], "op-2"),
        vec![("m1".to_owned(), Outcome::StaleGeneration, None)]
    );
    // The pending item is offered afresh to the new generation.
    assert_eq!(page(pull(&f, &second, None)).offered, ids(&["m1"]));
}

#[test]
fn a_resume_rotates_the_generation_and_an_older_claim_is_stale_generation() {
    let f = fixture(1);
    let first = pull_seat(&f);
    page(pull(&f, &first, None));
    let resumed = rotate(&f, &first, "n1", 2);
    assert_eq!(resumed.binding_generation, first.binding_generation + 1);
    // Same native session, older generation: the offer set was discarded, and
    // the answer names the generation rather than "never offered".
    assert_eq!(
        confirm(&f, &first, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::StaleGeneration, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
}

#[test]
fn a_truncated_id_is_listed_truncated_and_refused_on_confirm() {
    let f = fixture(0);
    ordinary_message(&f, "big", &"x".repeat(WATCH_BODY_LIMIT_BYTES + 100));
    ordinary_message(&f, "small", "short");
    let context = pull_seat(&f);
    let page = page(pull(&f, &context, None));
    assert_eq!(page.truncated, ids(&["big"]));
    assert_eq!(page.offered, ids(&["small"]));
    assert!(page.text.contains("herdr-threads"), "{}", page.text);
    let report = confirm(&f, &context, &["big", "small"], "op-1");
    assert_eq!(
        report,
        vec![
            (
                "big".to_owned(),
                Outcome::RefusedTerminal,
                Some(Reason::Truncated)
            ),
            ("small".to_owned(), Outcome::Settled, None),
        ]
    );
    assert_eq!(receipt_state(&f, "big"), "pending");
}

#[test]
fn claude_and_codex_bindings_are_refused_no_hook_pull() {
    let f = fixture(1);
    let codex = check_in(&f, claim("x", "px", Harness::Codex, "nx", 0), "ci-x");
    assert_eq!(refusal(pull(&f, &codex, None)), Refusal::NoHookPull);
    let claude = check_in(&f, claim("c", "pc", Harness::Claude, "nc", 0), "ci-c");
    assert_eq!(refusal(pull(&f, &claude, None)), Refusal::NoHookPull);
    assert_eq!(
        confirm(&f, &codex, &["m1"], "op-1"),
        vec![(
            "m1".to_owned(),
            Outcome::RefusedTerminal,
            Some(Reason::NoHookPull)
        )]
    );
}

#[test]
fn session_mismatch_subagent_held_and_unresolved_seats_are_refused() {
    let f = fixture(1);
    let context = pull_seat(&f);

    let mut other_session = context.clone();
    other_session.native_session = NativeSessionId::new("someone-else");
    assert_eq!(
        refusal(pull(&f, &other_session, None)),
        Refusal::SessionMismatch
    );

    let mut subagent = context.clone();
    subagent.role = CallerRole::Subagent;
    assert_eq!(refusal(pull(&f, &subagent, None)), Refusal::SessionMismatch);

    let mut other_instance = context.clone();
    other_instance.instance = "elsewhere".into();
    assert_eq!(
        refusal(pull(&f, &other_instance, None)),
        Refusal::SessionMismatch
    );

    // A seat that never checked in has no binding.
    let none = claim("x", "px", fourth(), "nx", 0);
    assert_eq!(refusal(pull(&f, &none, None)), Refusal::NoBinding);

    // An unknown seat is unresolved.
    let ghost = claim("ghost", "pg", fourth(), "ng", 0);
    assert_eq!(refusal(pull(&f, &ghost, None)), Refusal::Unresolved);
    f.db.execute("UPDATE seats SET state='unresolved' WHERE id='s'", [])
        .unwrap();
    assert_eq!(refusal(pull(&f, &context, None)), Refusal::Unresolved);
    f.db.execute("UPDATE seats SET state='resolved' WHERE id='s'", [])
        .unwrap();

    f.db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','p','b',1,'repair')", []).unwrap();
    assert_eq!(refusal(pull(&f, &context, None)), Refusal::Held);
}

#[test]
fn confirm_of_an_id_never_offered_is_not_offered_and_settles_nothing() {
    let f = fixture(1);
    let context = pull_seat(&f);
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
    // Only what a pull showed is confirmable: m9 arrived after the page.
    page(pull(&f, &context, None));
    ordinary_message(&f, "m9", "unseen");
    assert_eq!(
        confirm(&f, &context, &["m1", "m9", "ghost"], "op-2"),
        vec![
            ("m1".to_owned(), Outcome::Settled, None),
            ("m9".to_owned(), Outcome::NotOffered, None),
            ("ghost".to_owned(), Outcome::NotOffered, None),
        ]
    );
    assert_eq!(receipt_state(&f, "m9"), "pending");
}

#[test]
fn settled_elsewhere_ids_leave_the_ledger_and_an_empty_page_is_unchanged() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    // The seat reads the item with `ack` outside the hook.
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    advance(&f, REOFFER_AFTER_MS);
    // Nothing is pending any more: the version moved or the page is empty, and
    // the empty page answers Unchanged.
    let reply = pull(&f, &context, Some(first.attention_version));
    assert!(
        matches!(reply, DeliveryPullReply::Unchanged { .. }),
        "{reply:?}"
    );
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)],
        "the dropped offer is gone"
    );
}

#[test]
fn already_settled_ids_report_already_settled_and_leave_the_ledger() {
    let f = fixture(1);
    let context = pull_seat(&f);
    page(pull(&f, &context, None));
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::AlreadySettled, None)]
    );
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-2"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
}

#[test]
fn the_marker_prefix_reaches_the_truncation_marker() {
    let f = fixture(0);
    ordinary_message(&f, "big", &"y".repeat(WATCH_BODY_LIMIT_BYTES + 10));
    let context = pull_seat(&f);
    let command = Command::DeliveryPull(DeliveryPull {
        claim: context,
        seen_attention_version: None,
        marker_prefix: vec!["/opt/ht".into(), "--state-dir".into(), "/s".into()],
    });
    let CommandResult::DeliveryPulled(reply) =
        f.service.handle(command, peer(), &budget()).unwrap()
    else {
        panic!("not a pull reply");
    };
    assert!(page(reply).text.contains("/opt/ht --state-dir /s body big"));
}

/// One pending invitation for seat `s` (what wakes a seat without a message).
fn pending_invitation(f: &Fixture) {
    let decision: i64 =
        f.db.query_row(
            "SELECT decision_seq+1 FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    f.db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t2','i','topic','goal',0,0,1)", []).unwrap();
    f.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES('inv','t2','s',1,'pending',0,?1,300,300)", [decision]).unwrap();
    bump_wake_attention(f);
}
fn pull_with_prefix(
    f: &Fixture,
    claim: &CallerClaim,
    seen: Option<u64>,
    prefix: &[&str],
) -> DeliveryPullReply {
    let command = Command::DeliveryPull(DeliveryPull {
        claim: claim.clone(),
        seen_attention_version: seen,
        marker_prefix: prefix.iter().map(|part| (*part).to_owned()).collect(),
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::DeliveryPulled(reply) => reply,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn attention_block_is_present_with_a_pending_invitation_and_follows_the_marker_prefix() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let first = page(pull_with_prefix(
        &f,
        &context,
        None,
        &["/opt/ht", "--state-dir", "/s"],
    ));
    assert!(
        first.text.contains(&format!(
            "[herdr-threads] attention {}:",
            first.attention_version
        )),
        "{}",
        first.text
    );
    assert!(
        first.text.contains("run /opt/ht --state-dir /s inbox"),
        "{}",
        first.text
    );
    // The default prefix keeps the wake marker byte for byte.
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let default = page(pull(&f, &context, None));
    assert!(
        default.text.contains(crate::notification::policy::MARKER),
        "{}",
        default.text
    );
}

#[test]
fn attention_block_is_absent_when_the_version_was_seen() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert!(first.text.contains("attention"), "{}", first.text);
    assert_eq!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
}

#[test]
fn an_attention_only_page_answers_page() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let only = page(pull(&f, &context, None));
    assert!(only.offered.is_empty(), "{:?}", only.offered);
    assert!(only.truncated.is_empty(), "{:?}", only.truncated);
    assert!(only.text.contains("attention"), "{}", only.text);
    assert!(
        !only.text.contains("[herdr-threads] message"),
        "{}",
        only.text
    );
}

#[test]
fn attention_version_moves_with_every_fingerprint_field() {
    use crate::ports::ModFingerprint;
    let base = ModFingerprint::default();
    let version = |fingerprint: &ModFingerprint| attention_version(fingerprint, None);
    let moved = [
        ModFingerprint {
            attention_version: 1,
            ..base.clone()
        },
        ModFingerprint {
            pending_receipts: 1,
            ..base.clone()
        },
        ModFingerprint {
            max_pending_ordinal: 1,
            ..base.clone()
        },
        ModFingerprint {
            lazy_pending: 1,
            ..base.clone()
        },
        ModFingerprint {
            max_lazy_rowid: 1,
            ..base.clone()
        },
        ModFingerprint {
            other_pending: 1,
            ..base.clone()
        },
        ModFingerprint {
            binding_generation: Some(0),
            ..base.clone()
        },
    ];
    let mut seen = std::collections::BTreeSet::from([version(&base)]);
    for fingerprint in &moved {
        assert!(seen.insert(version(fingerprint)), "{fingerprint:?}");
    }
    assert_eq!(version(&base), version(&base.clone()), "deterministic");
    // The notice term: absent, ordinal 1 and ordinal 2 are three versions.
    let with_notice = |notice| attention_version(&base, notice);
    let mut seen = std::collections::BTreeSet::new();
    for notice in [None, Some(1), Some(2)] {
        assert!(seen.insert(with_notice(notice)), "{notice:?}");
    }
}

#[test]
fn delivery_pull_is_advertised() {
    assert!(crate::protocol::capabilities::ADVERTISED.contains(&"delivery.pull_v1"));
}

// Kills: a pull answered `Unchanged` before this process read the seat's whole
// pending set, which strands offered-unconfirmed items after a daemon restart.
#[test]
fn a_restarted_daemon_offers_pending_items_again_to_a_hook_holding_its_last_version() {
    let mut f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert_eq!(first.offered, ids(&["m1"]));
    restart(&mut f);
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
    let again = page(pull(&f, &context, Some(first.attention_version)));
    assert_eq!(again.offered, ids(&["m1"]));
    assert_eq!(
        pull(&f, &context, Some(again.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: again.attention_version
        },
        "the short-circuit works again after a whole read"
    );
}

// Kills: an outlook that ignores offers, due re-offers, parking, new attention
// or a rotated generation.
#[test]
fn the_hook_pull_outlook_follows_offers_reoffers_and_parking() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let seat = SeatId::new("s");
    let generation = context.binding_generation;
    let now = |f: &Fixture| UtcMillis(f.clock.0.load(Ordering::SeqCst));
    let outlook = |f: &Fixture| {
        f.state
            .next_pull(&seat, generation, wake_attention(f), now(f))
    };
    assert_eq!(outlook(&f), NextPull::Unknown, "before any pull");
    page(pull(&f, &context, None));
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(now(&f).0 + REOFFER_AFTER_MS))
    );
    assert!(!f.state.holds_parked(&seat, generation));
    for _ in 1..MAX_OFFERS_PER_GENERATION {
        advance(&f, REOFFER_AFTER_MS);
        assert_eq!(outlook(&f), NextPull::Delivers, "the re-offer is due");
        page(pull(&f, &context, None));
    }
    assert_eq!(outlook(&f), NextPull::Nothing);
    assert!(f.state.holds_parked(&seat, generation));
    ordinary_message(&f, "m2", "later");
    bump_wake_attention(&f);
    assert_eq!(outlook(&f), NextPull::Delivers, "new attention");
    let rotated = rotate(&f, &context, "n2", 2);
    assert_eq!(
        f.state.next_pull(
            &seat,
            rotated.binding_generation,
            wake_attention(&f),
            now(&f)
        ),
        NextPull::Unknown,
        "a rotated generation starts over"
    );
    assert!(!f.state.holds_parked(&seat, rotated.binding_generation));
}

// ---- informational notices on the pull (ht-poh.53) ----

// Kills: leftover notices counted as wake work (final review 9 N1). Notices never wake: an offered id
// shown 10 s ago holds the wake until its re-offer is due, even with notices left over; the next pull
// still carries those notices and does not re-show the id.
#[test]
fn leftover_notices_do_not_bring_a_held_wake_forward() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let seat = SeatId::new("s");
    let generation = context.binding_generation;
    let total = crate::protocol::results::MAX_NOTICE_PAGE_ITEMS + 2;
    for n in 0..total {
        service_notice(&f, &format!("w-{n:02}"));
    }
    let offered_at = UtcMillis(f.clock.0.load(Ordering::SeqCst));
    let first = page(pull(&f, &context, None));
    assert_eq!(first.offered, ids(&["m1"]));
    assert_eq!(
        first.text.matches("] notice w-").count(),
        16,
        "{}",
        first.text
    );
    advance(&f, 10_000);
    let outlook = |f: &Fixture| {
        f.state.next_pull(
            &seat,
            generation,
            wake_attention(f),
            UtcMillis(f.clock.0.load(Ordering::SeqCst)),
        )
    };
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(offered_at.0 + REOFFER_AFTER_MS)),
        "leftover notices are not wake work"
    );
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(
        second.text.contains("notice w-16") && second.text.contains("notice w-17"),
        "{}",
        second.text
    );
    assert!(second.offered.is_empty(), "m1's re-offer is not due yet");
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(offered_at.0 + REOFFER_AFTER_MS))
    );
    assert!(matches!(
        pull(&f, &context, Some(second.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    advance(&f, REOFFER_AFTER_MS - 10_000);
    assert_eq!(outlook(&f), NextPull::Delivers, "the re-offer is due");
}

/// A projected informational notice for `s`: an overdue transition about
/// another seat (`x`), which never wakes `s` (TRUST-POLICY A7).
fn service_notice(f: &Fixture, id: &str) {
    let sequence = next_sequence(&f.db);
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES (?1,'i','t',?2,'warn','{}',0,?3,1)", params![id, sequence, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('receipt','t',?1,'x',?1,?2)", params![id, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s',?1,'t',?2,1)", params![id, sequence + 2]).unwrap();
    bump(&f.db, sequence);
}
fn overdue_notice_about_x(f: &Fixture, id: &str) {
    service_notice(f, id);
}
/// A projected invitation-rejection notice for `s`, authored by `x`.
fn rejection_notice(f: &Fixture, id: &str) {
    let sequence = next_sequence(&f.db);
    let inv = format!("inv-{id}");
    f.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES(?1,'t','x',9,'pending',0,1,300,300)", [&inv]).unwrap();
    f.db.execute("INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) VALUES (?1,'outside role',100,'x',1,'cooperative_top_level:test')", [&inv]).unwrap();
    let json =
        format!(r#"{{"action":"reject","seat":"x","invitation":"{inv}","reason":"outside role"}}"#);
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,author_kind,actor_seat_id,source_invitation_id,event_key,event_json,decision_at,decision_seq,event_offset) VALUES (?1,'i','t',?2,'warn','native','x',?3,'reject:'||?3,?4,100,?5,1)", params![id, sequence, inv, json, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s',?1,'t',?2,1)", params![id, sequence + 2]).unwrap();
    bump(&f.db, sequence);
}
fn notice_frontier(f: &Fixture) -> i64 {
    f.db.query_row(
        "SELECT COALESCE(MAX(offered_ordinal),0) FROM digest_notice_offer WHERE seat_id='s'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn current_check_in(
    f: &Fixture,
    claim: CallerClaim,
    operation: &str,
) -> crate::protocol::results::CheckInResult {
    let command = Command::CheckIn(CheckIn {
        mode: CheckInMode::Current,
        claim,
        operation: OperationId::new(operation),
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_service_notice_reaches_the_next_pull_once_as_a_notice_block() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-rej");
    let first = page(pull_with_prefix(&f, &context, None, &["ht"]));
    assert!(
        first.text.contains("[herdr-threads] notice w-rej in "),
        "{}",
        first.text
    );
    assert!(first.text.contains("ht body w-rej"), "{}", first.text);
    assert!(
        first.text.contains("informational, no ACK"),
        "{}",
        first.text
    );
    assert!(first.offered.is_empty() && first.truncated.is_empty());
    assert_eq!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
    // Shown once: even without the version it is not shown again.
    assert_eq!(
        pull(&f, &context, None),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
}

#[test]
fn a_rejection_notice_reaches_the_next_pull_once_as_a_notice_block() {
    let f = fixture(0);
    let context = pull_seat(&f);
    rejection_notice(&f, "w-rej");
    assert!(
        !crate::store::attention::warning_wakes_seat(&f.db, "s", "w-rej").unwrap(),
        "a rejection notice is informational"
    );
    let first = page(pull(&f, &context, None));
    assert!(
        first.text.contains("[herdr-threads] notice w-rej in "),
        "{}",
        first.text
    );
    assert!(first.text.contains("from x"), "{}", first.text);
    assert!(first.text.contains("body w-rej"), "{}", first.text);
    assert!(first.offered.is_empty() && first.truncated.is_empty());
    assert!(matches!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
}

#[test]
fn an_overdue_transition_about_another_seat_reaches_the_next_pull() {
    let f = fixture(0);
    let context = pull_seat(&f);
    overdue_notice_about_x(&f, "w-over");
    let first = page(pull(&f, &context, None));
    assert!(
        first.text.contains("[herdr-threads] notice w-over in "),
        "{}",
        first.text
    );
    assert!(first.offered.is_empty());
    assert!(matches!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
}

#[test]
fn a_pending_notice_reserves_no_wake() {
    let f = fixture(0);
    let context = pull_seat(&f);
    let before = wake_attention(&f);
    overdue_notice_about_x(&f, "w-over");
    let view = f
        .store
        .mod_seat_view(&SeatId::new("s"), &budget())
        .unwrap()
        .unwrap();
    assert_eq!(
        view.fingerprint.other_pending, 0,
        "a notice is not mod attention"
    );
    assert_eq!(wake_attention(&f), before);
    let candidates: i64 =
        f.db.query_row(
            "SELECT count(*) FROM wake_batches WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(candidates, 0);
    let _ = page(pull(&f, &context, None));
    assert_eq!(
        wake_attention(&f),
        before,
        "carrying it moves no wake counter either"
    );
}

#[test]
fn a_notice_carried_by_a_pull_is_not_carried_by_a_later_lifecycle_check_in() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let _ = page(pull(&f, &context, None));
    let result = current_check_in(&f, context.clone(), "ci-2");
    assert!(result.notices.items.is_empty(), "{:?}", result.notices);
}

#[test]
fn a_notice_carried_by_a_check_in_is_not_pulled() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let result = current_check_in(&f, context.clone(), "ci-2");
    assert_eq!(result.notices.items.len(), 1);
    let context = result.context;
    assert_eq!(
        pull(&f, &context, None),
        DeliveryPullReply::Unchanged {
            attention_version: attention_version(
                &f.store
                    .mod_seat_view(&SeatId::new("s"), &budget())
                    .unwrap()
                    .unwrap()
                    .fingerprint,
                None
            )
        }
    );
}

#[test]
fn a_new_notice_moves_the_version_past_unchanged() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert!(matches!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    service_notice(&f, "w-late");
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(second.text.contains("notice w-late"), "{}", second.text);
    assert!(second.offered.is_empty(), "the message is not re-offered");
}

#[test]
fn notices_beyond_one_page_continue_on_the_next_pull() {
    let f = fixture(0);
    let context = pull_seat(&f);
    let total = crate::protocol::results::MAX_NOTICE_PAGE_ITEMS + 2;
    for n in 0..total {
        service_notice(&f, &format!("w-{n:02}"));
    }
    let first = page(pull(&f, &context, None));
    assert_eq!(
        first.text.matches("] notice w-").count(),
        16,
        "{}",
        first.text
    );
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert_eq!(
        second.text.matches("] notice w-").count(),
        2,
        "{}",
        second.text
    );
    assert!(second.text.contains("notice w-16") && second.text.contains("notice w-17"));
    assert!(matches!(
        pull(&f, &context, Some(second.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
}

// Kills: notices waiting behind a message backlog (main review Important 2 knock-on).
#[test]
fn notices_ride_a_full_message_page() {
    let f = fixture(40);
    let context = pull_seat(&f);
    service_notice(&f, "w-wait");
    let first = page(pull(&f, &context, None));
    assert!(first.more);
    assert_eq!(
        first.offered.len(),
        PAGE_MAX_BLOCKS - 1,
        "one block slot is held for the notice"
    );
    assert!(first.text.contains("notice w-wait"), "{}", first.text);
    assert!(notice_frontier(&f) > 0, "settled by the frontier rule");
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(
        !second.text.contains("notice w-wait"),
        "shown once: {}",
        second.text
    );
    assert_eq!(second.offered.len(), 40 - (PAGE_MAX_BLOCKS - 1));
    assert!(!second.more);
}

#[test]
fn an_invitation_attention_block_is_not_repeated_after_notices_settle() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let first = page(pull(&f, &context, None));
    assert!(first.text.contains("attention pending"), "{}", first.text);
    assert!(first.text.contains("notice w-1"), "{}", first.text);
    assert_eq!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
}

#[test]
fn confirm_of_a_notice_id_is_not_offered() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let _ = page(pull(&f, &context, None));
    assert_eq!(
        confirm(&f, &context, &["w-1"], "op-n"),
        vec![("w-1".to_owned(), Outcome::NotOffered, None)]
    );
}

// ---- a backlog over the offer table (ht-poh.59, main review Important 2) ----

/// One pull as a hook makes it: it sends the last version it saw and keeps the reply's. Records when each id was
/// shown and returns the page text (None for `Unchanged`).
fn hook_pull_once(
    f: &Fixture,
    context: &CallerClaim,
    seen: &mut Option<u64>,
    shown: &mut std::collections::BTreeMap<MessageId, Vec<i64>>,
) -> Option<String> {
    match pull(f, context, *seen) {
        DeliveryPullReply::Page(page) => {
            *seen = Some(page.attention_version);
            let at = f.clock.0.load(Ordering::SeqCst);
            for id in page.offered.iter().chain(&page.truncated) {
                shown.entry(id.clone()).or_default().push(at);
            }
            Some(page.text)
        }
        DeliveryPullReply::Unchanged { attention_version } => {
            *seen = Some(attention_version);
            None
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// Pending items the seat's pulls have read so far (the drain's store walk).
fn items_read(f: &Fixture) -> usize {
    f.state.probe.items_read.load(Ordering::SeqCst)
}

// Kills: a full offer table evicting still-pending ids (re-offered afresh: shown again < 60 s later, more than
// three times), a full table answering Delivers forever, notices starving behind the endless `more` backlog
// (main review Important 2), and a full-table pull walking the whole pending set when the last whole read
// already saw it (final review 11 S2: 70 s+ here; a large backlog overran the hook's budget).
#[test]
fn a_backlog_over_the_offer_table_keeps_the_reoffer_bounds_and_parks_for_inbox() {
    use crate::delivery::ledger::MAX_OFFERS_PER_SEAT;
    // The smallest backlog that overfills the table, plus a few. The first four bodies are over the limit
    // (truncated, never confirmable); the rest are short, so a store page holds 32 of them.
    let backlog = MAX_OFFERS_PER_SEAT + 4;
    let f = fixture(0);
    let big = "x".repeat(WATCH_BODY_LIMIT_BYTES + 100);
    for n in 0..backlog {
        let body = if n < 4 { big.as_str() } else { "hello" };
        ordinary_message(&f, &format!("b{n:03}"), body);
    }
    let context = pull_seat(&f);
    // After the check-in, which would carry a notice already pending.
    service_notice(&f, "w-1");
    let seat = SeatId::new("s");
    let generation = context.binding_generation;
    let now = |f: &Fixture| UtcMillis(f.clock.0.load(Ordering::SeqCst));
    let outlook = |f: &Fixture| {
        f.state
            .next_pull(&seat, generation, wake_attention(f), now(f))
    };
    let mut shown = std::collections::BTreeMap::new();
    let mut seen = None;
    let bounded = |shown: &std::collections::BTreeMap<MessageId, Vec<i64>>| {
        for (id, times) in shown {
            assert!(
                times.len() <= usize::from(MAX_OFFERS_PER_GENERATION),
                "{id:?} shown {} times: {times:?}",
                times.len()
            );
            assert!(
                times
                    .windows(2)
                    .all(|pair| pair[1] - pair[0] >= REOFFER_AFTER_MS),
                "{id:?} shown again under 60 s: {times:?}"
            );
        }
    };

    // A hook pull every 100 ms for 1.6 s: the table fills in the first nine.
    let mut texts = Vec::new();
    for _ in 0..16 {
        texts.push(hook_pull_once(&f, &context, &mut seen, &mut shown));
        advance(&f, 100);
    }
    bounded(&shown);
    let first = texts[0].as_deref().expect("the first pull shows a page");
    assert!(
        first.contains("notice w-1"),
        "the notice rides the backlog page: {first}"
    );
    assert!(notice_frontier(&f) > 0, "the carried notice settled");
    assert_eq!(
        texts
            .iter()
            .flatten()
            .filter(|text| text.contains("notice w-1"))
            .count(),
        1,
        "the notice is shown once"
    );
    assert_eq!(
        shown.len(),
        MAX_OFFERS_PER_SEAT,
        "the table filled, then took no new id"
    );
    for n in MAX_OFFERS_PER_SEAT..backlog {
        let id = format!("b{n:03}");
        assert!(
            !shown.contains_key(&MessageId::new(&id)),
            "{id} waits for inbox"
        );
        assert_eq!(receipt_state(&f, &id), "pending");
    }
    let first_shown = shown.values().map(|times| times[0]).min().unwrap();
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(first_shown + REOFFER_AFTER_MS)),
        "a full table is not Delivers-forever"
    );
    assert!(
        f.state.holds_parked(&seat, generation),
        "a full table parks new ids for inbox"
    );

    // Past 60 s, 120 s and 180 s: 1.2 s of pulls every 100 ms each time. The last whole read saw this same
    // pending set, so a re-offer pull reads only as far as its due ids.
    for round in 1..=3 {
        f.clock
            .0
            .store(first_shown + round * REOFFER_AFTER_MS, Ordering::SeqCst);
        for index in 0..12 {
            let before = items_read(&f);
            hook_pull_once(&f, &context, &mut seen, &mut shown);
            if round == 1 && index == 0 {
                let read = items_read(&f) - before;
                assert!(
                    read <= PAGE_MAX_BLOCKS,
                    "the first re-offer pull stops after its due ids; it read {read} of {backlog}"
                );
            }
            advance(&f, 100);
        }
        bounded(&shown);
    }
    assert_eq!(shown.len(), MAX_OFFERS_PER_SEAT);
    assert!(
        shown
            .values()
            .all(|times| times.len() == usize::from(MAX_OFFERS_PER_GENERATION)),
        "every offered id was re-offered up to the bound, then parked"
    );
    assert_eq!(outlook(&f), NextPull::Nothing);
    assert!(f.state.holds_parked(&seat, generation));

    // A new notice on the parked full table rides a page without reading the pending set: nothing in it could be
    // shown or freed.
    service_notice(&f, "w-2");
    let before = items_read(&f);
    let text = hook_pull_once(&f, &context, &mut seen, &mut shown).expect("the notice is shown");
    assert!(text.contains("notice w-2"), "{text}");
    assert_eq!(items_read(&f), before, "the drain was skipped");
    assert!(
        shown
            .values()
            .all(|times| times.len() == usize::from(MAX_OFFERS_PER_GENERATION)),
        "no message rides along"
    );
    // New attention still wakes; the parked table makes that wake name `inbox`.
    bump_wake_attention(&f);
    assert_eq!(outlook(&f), NextPull::Delivers);
}

// Kills: an id withheld at capacity stuck behind `Unchanged` after the same whole read freed a slot.
#[test]
fn an_id_withheld_at_capacity_is_shown_once_a_whole_read_frees_a_slot() {
    use crate::delivery::ledger::MAX_OFFERS_PER_SEAT;
    let f = fixture(MAX_OFFERS_PER_SEAT + 1);
    let context = pull_seat(&f);
    let mut seen = None;
    for _ in 0..MAX_OFFERS_PER_SEAT / PAGE_MAX_BLOCKS {
        seen = Some(page(pull(&f, &context, seen)).attention_version);
    }
    let last = format!("m{}", MAX_OFFERS_PER_SEAT + 1);
    assert!(
        matches!(
            pull(&f, &context, seen),
            DeliveryPullReply::Unchanged { .. }
        ),
        "the table is full: {last} waits"
    );
    assert!(
        f.state
            .holds_parked(&SeatId::new("s"), context.binding_generation)
    );
    // `inbox` settles m1 outside the hook: the version moves and the whole read drops m1 from the table.
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    let version = match pull(&f, &context, seen) {
        DeliveryPullReply::Unchanged { attention_version } => attention_version,
        other => panic!("nothing fits before the whole read frees the slot: {other:?}"),
    };
    let freed = page(pull(&f, &context, Some(version)));
    assert_eq!(
        freed.offered,
        ids(&[last.as_str()]),
        "no time passed: the freed slot shows it"
    );
}

// Kills: a pull that settled its notices and then lost a rotation race before recording, answering Busy so the
// settled notices are never shown (final review 11 S3).
#[test]
fn notices_settled_by_a_pull_that_lost_a_rotation_race_are_still_returned() {
    let f = fixture(2);
    let context = pull_seat(&f);
    service_notice(&f, "w-race");
    let newer = context.binding_generation + 1;
    *f.state.probe.before_record.lock().unwrap() =
        Some(Box::new(move |ledger: &mut OfferedLedger| {
            ledger
                .seat_mut(&SeatId::new("s"), newer, "n2")
                .expect("a newer generation replaces the entry");
        }));
    let page = page(pull(&f, &context, None));
    assert!(
        page.text.contains("[herdr-threads] notice w-race in "),
        "{}",
        page.text
    );
    assert!(
        notice_frontier(&f) > 0,
        "the notice settled before the race"
    );
    assert!(
        page.offered.is_empty() && page.truncated.is_empty() && !page.more,
        "no message is shown to a superseded generation: {page:?}"
    );
    assert!(!page.text.contains("message m1"), "{}", page.text);
    let ledger = f.state.lock();
    let entry = ledger.get(&SeatId::new("s")).unwrap();
    assert_eq!(entry.generation, newer);
    assert!(
        entry.offers.is_empty(),
        "nothing recorded into the newer entry"
    );
}

// Kills: dropping informational invitations from the actual service page, or
// treating a displayed invitation as a message receipt / accepted membership.
#[test]
fn a_lazy_invitation_reaches_hook_pull_without_attention_or_acceptance() {
    use crate::protocol::commands::{CreateThread, InvitationConfirmation, Invite};
    let f = fixture(0);
    let context = pull_seat(&f);
    let DeliveryPullReply::Unchanged {
        attention_version: quiet_version,
    } = pull(&f, &context, None)
    else {
        panic!("quiet pull must be unchanged")
    };
    let inviter = check_in(&f, claim("x", "px", fourth(), "inviter", 0), "ci-inviter");
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "lazy invitation topic".into(),
                goal: "review the handoff".into(),
                operation: OperationId::new("create-lazy-invite"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread not created")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::Lazy),
                thread: thread.clone(),
                seat: SeatId::new("s"),
                deadline_millis: None,
                operation: OperationId::new("lazy-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        wake_attention(&f),
        0,
        "Lazy invitation never schedules ordinary wake work"
    );
    let first = page(pull(&f, &context, Some(quiet_version)));
    assert!(
        first.text.contains("lazy invitation topic"),
        "{}",
        first.text
    );
    assert!(first.text.contains(thread.as_str()), "{}", first.text);
    assert!(first.text.contains("accept"), "{}", first.text);
    assert!(!first.text.contains("] attention "), "{}", first.text);
    assert!(
        first.offered.is_empty(),
        "invitations are not confirmable message ids"
    );
    assert!(first.truncated.is_empty());
    assert!(
        first.text.contains("review the handoff"),
        "invitation goal reaches context: {}",
        first.text
    );
    assert!(
        matches!(
            pull(&f, &context, Some(first.attention_version)),
            DeliveryPullReply::Unchanged { .. }
        ),
        "whole passive display is not repeatedly injected"
    );
    let (state, mode): (String, String) =
        f.db.query_row(
            "SELECT state,confirmation FROM invitations WHERE thread_id=?1 AND seat_id='s'",
            [thread.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((state.as_str(), mode.as_str()), ("pending", "lazy"));
    let attention = crate::store::attention::wake_seat_attention(&f.db, "s").unwrap();
    assert!(!attention.attention.has_pending_invitation);
    assert!(!attention.attention.has_pending_receipt);
    assert!(attention.attention.latest_warning_seq.is_none());
}

// Kills: caching an offered informational frontier as if it settled the
// original sender's independent open-condition attention.
#[test]
fn departed_receipt_origins_survive_pull_offers_late_association_and_partial_ack() {
    use crate::protocol::commands::{Ack, Leave};
    let mut f = fixture(0);
    let a = pull_seat(&f);
    let b = check_in(&f, claim("x", "px", fourth(), "origin-b", 0), "ci-origin-b");
    let recipient = check_in(
        &f,
        claim("c", "pc", fourth(), "recipient", 0),
        "ci-recipient",
    );
    for (n, author) in [(1, "s"), (2, "s"), (3, "x")] {
        let id = format!("origin-{n}");
        f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,author_kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','native',?3,'origin body',0,?2)", params![id,n,author]).unwrap();
        if n < 3 {
            f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','c','pending',1,0,500)", [&id]).unwrap();
        } else {
            f.db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('origin-p','i','actor','origin-p',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed')", []).unwrap();
            f.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('origin-p','t','c',1,1001000,0)", []).unwrap();
            f.db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('origin-p',?1,'i','t',3,0,3,0,1,0)", [&id]).unwrap();
            f.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,deadline_at,ack_required) VALUES (?1,'c','pending',1001000,1)", [&id]).unwrap();
        }
    }
    f.db.execute("UPDATE threads SET next_sequence=1000 WHERE id='t'", [])
        .unwrap();
    f.db.execute(
        "UPDATE host_instances SET decision_seq=1000 WHERE id='i'",
        [],
    )
    .unwrap();
    for (who, operation) in [(&a, "leave-a"), (&b, "leave-b")] {
        f.service
            .handle(
                Command::Leave(Leave {
                    thread: ThreadId::new("t"),
                    operation: OperationId::new(operation),
                    claim: who.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let shared: Arc<dyn Clock> = f.clock.clone();
    let context = StoreContext::new(f.directory.join("store.db"), shared);
    crate::store::receipts::scan_due(
        &context,
        &mut f.db,
        100,
        &mut crate::store::receipts::ReceiptDueCursor::default(),
    )
    .unwrap();
    let warning: String =
        f.db.query_row(
            "SELECT open_warning_id FROM warning_conditions WHERE condition_kind='receipt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    while crate::store::materialization::advance_work(
        &mut f.db,
        &format!("work:{warning}"),
        crate::ports::DurableWorkAdmission::new(16).unwrap(),
        &budget(),
        context.clock(),
    )
    .unwrap()
    .has_more
    {}
    let first_a = page(pull(&f, &a, None));
    assert!(first_a.text.contains("] attention "), "{}", first_a.text);
    assert!(
        matches!(
            pull(&f, &a, Some(first_a.attention_version)),
            DeliveryPullReply::Unchanged { .. }
        ),
        "whole warning offer does not force repeated context"
    );
    struct OriginNotifier(std::sync::Mutex<Vec<SeatId>>);
    impl crate::ports::NotificationPort for OriginNotifier {
        fn attempt_wake(
            &self,
            reservation: crate::ports::WakeReservation,
            _: &crate::ports::HostCallContext,
        ) -> Result<crate::ports::WakeOutcome, ApiError> {
            self.0.lock().unwrap().push(reservation.seat);
            Ok(crate::ports::WakeOutcome::Unavailable)
        }
    }
    f.db.execute(
        "UPDATE observed_targets SET ui_state='active_turn',observed_at=?1",
        [f.clock.utc_now().0],
    )
    .unwrap();
    f.db.execute(
        "UPDATE observed_targets SET ui_state='idle' WHERE target_id='p'",
        [],
    )
    .unwrap();
    let elected_boot = uuid::Uuid::new_v4();
    let wake_store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            StoreContext::new(f.directory.join("store.db"), f.clock.clone()),
            "i",
            StoreSettings {
                daemon_boot: Some(elected_boot),
                ..StoreSettings::default()
            },
        )
        .unwrap(),
    );
    let scheduled =
        crate::service::workers::ScheduledStore::new(wake_store, Arc::new(FairWriter::new(32)));
    let notifier = OriginNotifier(std::sync::Mutex::new(Vec::new()));
    let scheduler = crate::scheduler::Scheduler::new(
        "i".into(),
        &scheduled,
        &scheduled,
        &notifier,
        crate::notification::policy::RetryConfig::default(),
        elected_boot,
    )
    .with_hook_pull(f.state.as_ref());
    scheduler.drive_wakes(&budget()).unwrap();
    assert!(
        notifier.0.lock().unwrap().contains(&a.seat),
        "actual canonical scheduler must retain an open origin warning after whole offer despite NextPull::Nothing"
    );

    let first_b = pull(&f, &b, None);
    let seen_b = match first_b {
        DeliveryPullReply::Page(p) => p.attention_version,
        DeliveryPullReply::Unchanged { attention_version } => attention_version,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipt_warning_origins WHERE seat_id='x'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    f.clock.0.store(1_002_000, Ordering::SeqCst);
    crate::store::receipts::scan_due(
        &context,
        &mut f.db,
        100,
        &mut crate::store::receipts::ReceiptDueCursor::default(),
    )
    .unwrap();
    let later_b = page(pull(&f, &b, Some(seen_b)));
    assert!(later_b.text.contains("] attention "), "{}", later_b.text);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE condition_kind='receipt'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    for (n, remaining) in [(1, 2), (2, 1), (3, 0)] {
        f.service
            .handle(
                Command::Ack(Ack {
                    messages: vec![MessageId::new(format!("origin-{n}"))],
                    operation: OperationId::new(format!("ack-origin-{n}")),
                    claim: recipient.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let a_view = f.store.mod_seat_view(&a.seat, &budget()).unwrap().unwrap();
        let b_view = f.store.mod_seat_view(&b.seat, &budget()).unwrap().unwrap();
        assert_eq!(a_view.fingerprint.other_pending, u64::from(remaining > 0));
        assert_eq!(b_view.fingerprint.other_pending, u64::from(remaining > 0));
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM receipt_warning_origins WHERE warning_id=?1",
                [&warning],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            if remaining > 0 { 2 } else { 0 }
        );
    }
    let before_final_pass = notifier.0.lock().unwrap().len();
    scheduler.drive_wakes(&budget()).unwrap();
    assert_eq!(
        notifier.0.lock().unwrap().len(),
        before_final_pass,
        "passive final clear creates no warning-only wake"
    );
    assert_eq!(f.db.query_row("SELECT count(*) FROM warning_conditions WHERE condition_kind='receipt' AND clear_warning_id IS NOT NULL", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
}

// Kills: a real canonical lazy arrival never triggering the live watch consumer.
#[test]
fn canonical_lazy_invitation_signals_a_passive_channel_drain_without_attention_history() {
    use crate::{
        ports::ModChannelSink,
        protocol::{
            commands::{CreateThread, InvitationConfirmation, Invite},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let f = fixture(0);
    let recipient = check_in(
        &f,
        claim("s", "p", Harness::Claude, "channel", 0),
        "channel-ci",
    );
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "inviter", 0),
        "channel-inviter-ci",
    );
    let reads = Arc::new(
        SqliteStore::new(
            StoreContext::new(f.directory.join("store.db"), f.clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    let service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink(std::sync::Mutex::new(Vec::new())));
    service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(f.clock.utc_now()).unwrap();
    assert!(sink.0.lock().unwrap().is_empty());
    let before = f
        .store
        .mod_seat_view(&recipient.seat, &budget())
        .unwrap()
        .unwrap();
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "channel invitation".into(),
                goal: "review channel".into(),
                operation: OperationId::new("channel-create"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread not created")
    };
    service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::Lazy),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: None,
                operation: OperationId::new("channel-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let after = f
        .store
        .mod_seat_view(&recipient.seat, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(
        before.fingerprint, after.fingerprint,
        "passive content never changes wake eligibility"
    );
    registry.pass(f.clock.utc_now()).unwrap();
    assert_eq!(
        sink.0.lock().unwrap().len(),
        1,
        "actual channel receives a drain signal"
    );
    registry.pass(f.clock.utc_now()).unwrap();
    assert_eq!(
        sink.0.lock().unwrap().len(),
        1,
        "unchanged canonical content does not flood the channel"
    );
}

#[test]
fn canonical_lazy_invitation_content_changes_on_accept_reject_and_retirement_only() {
    use crate::protocol::commands::{
        Accept, CreateThread, InvitationConfirmation, Invite, OperatorRetire, Reject,
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "version-inviter", 0),
        "version-inviter-ci",
    );
    let create = |suffix: &str, invite: bool| {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: suffix.into(),
                    goal: "review".into(),
                    operation: OperationId::new(format!("version-create-{suffix}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("not created")
        };
        let invitation = if invite {
            let CommandResult::Invitation(id) = f
                .service
                .handle(
                    Command::Invite(Invite {
                        confirmation: Some(InvitationConfirmation::Lazy),
                        thread: thread.clone(),
                        seat: recipient.seat.clone(),
                        deadline_millis: None,
                        operation: OperationId::new(format!("version-invite-{suffix}")),
                        claim: inviter.clone(),
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap()
            else {
                panic!("not invited")
            };
            Some(id)
        } else {
            None
        };
        (thread, invitation)
    };
    let version = || {
        f.store
            .mod_seat_view(&recipient.seat, &budget())
            .unwrap()
            .unwrap()
            .pending_invitation_version
    };
    assert_eq!(version(), 0);
    let (accepted, _) = create("accepted", true);
    let (rejected, rejection) = create("rejected", true);
    let (newest, _) = create("newest", true);
    let initial = version();
    let shown = page(pull(&f, &recipient, None));
    create("unrelated", false);
    assert_eq!(
        version(),
        initial,
        "unrelated real decisions do not change recipient content"
    );
    assert!(matches!(
        pull(&f, &recipient, Some(shown.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    f.service
        .handle(
            Command::Accept(Accept {
                thread: accepted,
                operation: OperationId::new("version-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let after_accept = version();
    assert_ne!(
        after_accept, initial,
        "older acceptance changes content despite unchanged newest pending invitation"
    );
    f.service
        .handle(
            Command::Reject(Reject {
                thread: rejected,
                invitation: rejection.unwrap(),
                reason: "outside remit".into(),
                operation: OperationId::new("version-reject"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_ne!(
        version(),
        after_accept,
        "older rejection removes canonical content"
    );
    let before_topic = version();
    f.service
        .handle(
            Command::SetTopic(crate::protocol::commands::SetTopic {
                thread: newest,
                topic: "updated invitation decision context".into(),
                operation: OperationId::new("version-topic"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_ne!(
        version(),
        before_topic,
        "canonical topic revision invalidates a previously whole-offered invitation"
    );
    let changed = page(pull(&f, &recipient, Some(shown.attention_version)));
    assert!(changed.text.contains("updated invitation decision context"));
    assert!(
        !changed.text.contains("accepted"),
        "settled/unchanged invitation contexts do not flood the next page"
    );
    f.store
        .mutate_operator(
            crate::ports::OperatorRequest::Retire(OperatorRetire {
                seat: recipient.seat.clone(),
                operation: OperationId::new("version-retire"),
            }),
            crate::protocol::authority::OperatorActor::from_peer(peer(), 501).unwrap(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        version(),
        0,
        "retirement removes recipient-visible pending content"
    );
}

fn canonical_lazy_invitation_backlog(
    f: &Fixture,
    recipient: &CallerClaim,
    inviter: &CallerClaim,
    count: usize,
    prefix: &str,
) -> ThreadId {
    use crate::protocol::commands::{CreateThread, InvitationConfirmation, Invite};
    let mut oldest = None;
    for n in 0..count {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: format!("{prefix}-topic-{n}"),
                    goal: "review".into(),
                    operation: OperationId::new(format!("{prefix}-create-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("thread not created")
        };
        oldest.get_or_insert_with(|| thread.clone());
        f.service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::Lazy),
                    thread,
                    seat: recipient.seat.clone(),
                    deadline_millis: None,
                    operation: OperationId::new(format!("{prefix}-invite-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    oldest.expect("nonempty canonical backlog")
}

// These saturation controls intentionally exercise the capable route. The
// historical V1 REDs remain evidence of the unsupported old cache limit.
fn presentation_test_pull(
    f: &Fixture,
    claim: &CallerClaim,
) -> crate::protocol::delivery::DeliveryPullReplyV2 {
    let result = f
        .service
        .handle(
            Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                claim: claim.clone(),
                consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                render: crate::protocol::delivery::PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: Vec::new(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryPulledV2(reply) = result else {
        panic!("wrong presentation result {result:?}")
    };
    reply
}
fn presentation_invitations(
    page: &crate::protocol::delivery::DeliveryPageV2,
) -> Vec<crate::protocol::delivery::PresentationInvitation> {
    page.items
        .iter()
        .filter_map(|v| match v {
            crate::protocol::delivery::PresentationItemV2::Invitation { id, content } => {
                Some(crate::protocol::delivery::PresentationInvitation {
                    id: id.clone(),
                    content: content.clone(),
                })
            }
            _ => None,
        })
        .collect()
}
fn presentation_watch_frozen(
    f: &Fixture,
    claim: &CallerClaim,
) -> Option<crate::delivery::presentation::FrozenPresentationRender> {
    f.store
        .presentation_render_selection(
            claim,
            crate::protocol::delivery::PresentationConsumer::Watch,
            &budget(),
        )
        .unwrap()
        .map(
            |identity| crate::delivery::presentation::FrozenPresentationRender {
                identity,
                codec: None,
            },
        )
}
fn presentation_test_confirm(
    f: &Fixture,
    claim: &CallerClaim,
    page: &crate::protocol::delivery::DeliveryPageV2,
) {
    if let Some(token) = &page.presentation_token {
        let result = f
            .service
            .handle(
                Command::DeliveryPresentationConfirmV2(
                    crate::protocol::delivery::DeliveryPresentationConfirmV2 {
                        claim: claim.clone(),
                        consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                        token: token.clone(),
                        emitted_controls: page.emitted_controls.clone(),
                        presentation_lease: None,
                    },
                ),
                peer(),
                &budget(),
            )
            .unwrap();
        assert!(matches!(
            result,
            CommandResult::DeliveryPresentationConfirmedV2(
                crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 {
                    status:
                        crate::protocol::delivery::DeliveryPresentationConfirmationV2::Confirmed,
                    ..
                }
            )
        ));
    }
}

#[test]
fn an_old_visible_invitation_topic_change_survives_a_saturated_newest_window() {
    use crate::protocol::commands::SetTopic;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "saturated-inviter", 0),
        "saturated-inviter-ci",
    );
    // The smallest backlog with a visible invitation outside WINDOW1001.
    let oldest = canonical_lazy_invitation_backlog(
        &f,
        &recipient,
        &inviter,
        crate::store::attention::WINDOW + 1,
        "window",
    );

    let mut exhausted = false;
    let mut saw_oldest = false;
    for _ in 0..40 {
        let crate::protocol::delivery::DeliveryPullReplyV2::Page(shown) =
            presentation_test_pull(&f, &recipient)
        else {
            panic!("expected finite presentation page")
        };
        saw_oldest |= shown.text.contains("window-topic-0");
        presentation_test_confirm(&f, &recipient, &shown);
        if !shown.more {
            exhausted = true;
            break;
        }
    }
    assert!(
        exhausted,
        "finite1002 invitation display must make progress"
    );
    assert!(saw_oldest, "the old context was actually displayed");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook' AND complete_count=1",[],|r|r.get::<_,i64>(0)).unwrap(),(crate::store::attention::WINDOW+1) as i64,"every canonical context completed before quiet");
    assert!(matches!(
        presentation_test_pull(&f, &recipient),
        crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. }
    ));
    f.service
        .handle(
            Command::SetTopic(SetTopic {
                thread: oldest,
                topic: "changed oldest visible invitation context".into(),
                operation: OperationId::new("window-change-oldest"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(changed) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("expected changed canonical context")
    };
    assert!(
        changed
            .text
            .contains("changed oldest visible invitation context"),
        "{}",
        changed.text
    );
    assert!(changed.offered.is_empty());
    assert!(changed.truncated.is_empty());
    presentation_test_confirm(&f, &recipient, &changed);
    assert!(matches!(
        presentation_test_pull(&f, &recipient),
        crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. }
    ));
}

#[test]
fn a_finite_invitation_backlog_progresses_beyond_the_whole_display_cache() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "capacity-inviter", 0),
        "capacity-inviter-ci",
    );
    let count = crate::delivery::ledger::MAX_INVITATION_DISPLAYS_PER_SEAT + 1;
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, count, "capacity");

    let mut saw_final = false;
    let mut reached_quiet = false;
    for _ in 0..40 {
        match presentation_test_pull(&f, &recipient) {
            crate::protocol::delivery::DeliveryPullReplyV2::Page(shown) => {
                saw_final |= shown
                    .text
                    .contains(&format!("capacity-topic-{}", count - 1));
                presentation_test_confirm(&f, &recipient, &shown);
                assert!(shown.offered.is_empty());
                assert!(shown.truncated.is_empty());
            }
            crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. } => {
                reached_quiet = true;
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        saw_final,
        "finite1025 backlog must display its last invitation before quiet"
    );
    assert!(reached_quiet, "whole context is not repeatedly flooded");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook' AND complete_count=1",[],|r|r.get::<_,i64>(0)).unwrap(),count as i64,"every canonical context completed before quiet");
    assert_eq!(f.db.query_row(
        "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending' AND confirmation='lazy'",
        [], |r| r.get::<_, i64>(0),
    ).unwrap(), count as i64);
    assert_eq!(wake_attention(&f), 0);
}

#[test]
fn a_departed_inviter_remains_actionable_after_whole_deadline_warning_until_acceptance() {
    use crate::protocol::commands::{Accept, CreateThread, InvitationConfirmation, Invite, Leave};
    let mut f = fixture(0);
    let inviter = pull_seat(&f);
    let recipient = check_in(
        &f,
        claim("x", "px", fourth(), "invited-recipient", 0),
        "invited-recipient-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "inviter deadline origin".into(),
                goal: "review".into(),
                operation: OperationId::new("inviter-origin-create"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("not created")
    };
    let CommandResult::Invitation(invitation) = f
        .service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread: thread.clone(),
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("inviter-origin-invite"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("not invited")
    };
    f.service
        .handle(
            Command::Leave(Leave {
                thread: thread.clone(),
                operation: OperationId::new("inviter-origin-leave"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let _ = pull(&f, &inviter, None);
    f.clock.0.store(1_001_001, Ordering::SeqCst);
    let context = StoreContext::new(f.directory.join("store.db"), f.clock.clone());
    let expired =
        crate::store::invitation_due::scan_invitation_due_batch(&context, &mut f.db, None, 100)
            .unwrap();
    assert_eq!(expired.warnings_added, 1);
    let warning: String = f.db.query_row(
        "SELECT open_warning_id FROM warning_conditions WHERE condition_kind='invitation' AND condition_id=?1",
        [invitation.as_str()], |r| r.get(0),
    ).unwrap();
    while crate::store::materialization::advance_work(
        &mut f.db,
        &format!("work:{warning}"),
        crate::ports::DurableWorkAdmission::new(16).unwrap(),
        &budget(),
        context.clock(),
    )
    .unwrap()
    .has_more
    {}
    let first = page(pull(&f, &inviter, None));
    assert!(first.text.contains("] attention "), "{}", first.text);
    assert!(matches!(
        pull(&f, &inviter, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    assert_eq!(
        f.store
            .mod_seat_view(&inviter.seat, &budget())
            .unwrap()
            .unwrap()
            .fingerprint
            .other_pending,
        1,
        "whole display does not consume the original departed inviter's condition"
    );
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("inviter-origin-accept"),
                claim: recipient,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        f.store
            .mod_seat_view(&inviter.seat, &budget())
            .unwrap()
            .unwrap()
            .fingerprint
            .other_pending,
        0
    );
    assert_eq!(f.db.query_row(
        "SELECT count(*) FROM warning_conditions WHERE condition_kind='invitation' AND condition_id=?1 AND open_warning_id=?2 AND clear_warning_id IS NOT NULL",
        params![invitation.as_str(), warning], |r| r.get::<_, i64>(0),
    ).unwrap(), 1, "one immutable warning lifetime clears on actual acceptance");
}

#[test]
fn presentation_v2_replays_one_canonical_invitation_until_emit_confirmation() {
    use serde_json::json;
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "v2-inviter", 0),
        "v2-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "durable-v2");
    let wire = json!({"kind":"delivery_pull_v2","args":{
        "claim":recipient,"consumer":"hook",
        "render":{"kind":"hook","confirmable_selector":0,"passive_selector":0}
    }});
    let command = serde_json::from_value::<Command>(wire.clone());
    assert!(
        command.is_ok(),
        "actual canonical service needs the V2 route: {command:?}"
    );
    let result = f
        .service
        .handle(command.unwrap(), peer(), &budget())
        .unwrap();
    let first = serde_json::to_value(result).unwrap();
    assert_eq!(first["kind"], "delivery_pulled_v2");
    assert_eq!(first["data"]["kind"], "page");
    let page = &first["data"]["args"];
    assert!(
        page["text"]
            .as_str()
            .unwrap()
            .contains("durable-v2-topic-0")
    );
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "invitation");
    // This ordered invitation-only page emits no Attention or Cleared controls.
    let expected_controls = json!([]);
    assert_eq!(page["emitted_controls"], expected_controls);
    let token = page["presentation_token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 32);
    assert!(crate::protocol::delivery::valid_presentation_token(&token));
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    // A lost reply and service restart cannot consume the invitation offer.
    restart(&mut f);
    let replay = f
        .service
        .handle(
            serde_json::from_value(wire.clone()).unwrap(),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), first);
    let wrong = json!({"kind":"delivery_presentation_confirm_v2","args":{
        "claim":recipient,"consumer":"hook","token":"ffffffffffffffffffffffffffffffff",
        "emitted_controls":expected_controls
    }});
    let wrong = f
        .service
        .handle(serde_json::from_value(wrong).unwrap(), peer(), &budget())
        .unwrap();
    let wrong = serde_json::to_value(wrong).unwrap();
    assert_eq!(wrong["data"]["status"], "not_current_page");
    assert_eq!(wrong["data"]["acknowledged_controls"], expected_controls);
    let confirmation = json!({"kind":"delivery_presentation_confirm_v2","args":{
        "claim":recipient,"consumer":"hook","token":token,
        "emitted_controls":expected_controls
    }});
    let settled = f
        .service
        .handle(
            serde_json::from_value(confirmation.clone()).unwrap(),
            peer(),
            &budget(),
        )
        .unwrap();
    let settled = serde_json::to_value(settled).unwrap();
    assert_eq!(settled["data"]["status"], "confirmed");
    assert_eq!(settled["data"]["acknowledged_controls"], expected_controls);
    let duplicate = f
        .service
        .handle(
            serde_json::from_value(confirmation).unwrap(),
            peer(),
            &budget(),
        )
        .unwrap();
    let duplicate = serde_json::to_value(duplicate).unwrap();
    assert_eq!(duplicate["data"]["status"], "already_confirmed");
    assert_eq!(
        duplicate["data"]["acknowledged_controls"],
        expected_controls
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "presentation never accepts or ACKs"
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0,
        "presentation confirmation creates no receipt ACK"
    );
}

#[test]
fn canonical_invitation_context_coalesces_topic_changes_without_recipient_fanout() {
    use crate::protocol::commands::SetTopic;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "context-inviter", 0),
        "context-inviter-ci",
    );
    let thread = canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "context-source");
    let original_wake = wake_attention(&f);
    for (n, topic) in [(1, "first latest topic"), (2, "second latest topic")] {
        f.service
            .handle(
                Command::SetTopic(SetTopic {
                    thread: thread.clone(),
                    topic: topic.into(),
                    operation: OperationId::new(format!("context-topic-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let projection = f.db.prepare("SELECT c.context_revision,c.last_change_seq,t.topic,t.topic_revision FROM invitation_context_latest c JOIN threads t ON t.id=c.thread_id WHERE c.instance_id='i' AND c.thread_id=?1");
    assert!(
        projection.is_ok(),
        "canonical mutations need a durable latest-context projection: {:?}",
        projection.as_ref().err()
    );
    let (revision, seq, topic, topic_revision): (i64, i64, String, i64) = projection
        .unwrap()
        .query_row([thread.as_str()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(topic, "second latest topic");
    assert_eq!(topic_revision, 2);
    assert!(revision >= 2);
    assert!(seq > 0);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitation_context_latest WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "coalesced state stays one row per thread"
    );
    assert_eq!(
        wake_attention(&f),
        original_wake,
        "passive topic changes do not wake"
    );
    let before: (i64, i64) = (revision, seq);
    // Exact operation replay must not invent a new canonical change sequence.
    f.service
        .handle(
            Command::SetTopic(SetTopic {
                thread: thread.clone(),
                topic: "second latest topic".into(),
                operation: OperationId::new("context-topic-2"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(f.db.query_row("SELECT context_revision,last_change_seq FROM invitation_context_latest WHERE thread_id=?1", [thread.as_str()], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(), before);
}

#[test]
fn presentation_v2_resumes_control_heavy_ordinary_partial_without_changing_body_limit() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    ordinary_message(&f, "partial-prefix", &"a".repeat(20000));
    ordinary_message(&f, "partial-controls", &"\0".repeat(9000));
    let first = presentation_test_pull(&f, &recipient);
    assert!(matches!(
        first,
        crate::protocol::delivery::DeliveryPullReplyV2::Page(_)
    ));
    let partial:String=f.db.query_row("SELECT ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let partial: std::collections::HashMap<String, String> =
        serde_json::from_str(&partial).unwrap();
    let bytes = partial
        .get("partial-controls")
        .expect("real page split retained control bytes");
    assert!(bytes.len() > 16384 / 6 && bytes.len() < WATCH_BODY_LIMIT_BYTES);
    assert!(bytes.bytes().all(|b| b == 0));
    let mut completed = false;
    for _ in 0..4 {
        if let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
            presentation_test_pull(&f, &recipient)
            && page.truncated.contains(&MessageId::new("partial-controls"))
        {
            assert!(page.text.contains(&"\0".repeat(WATCH_BODY_LIMIT_BYTES)));
            completed = true;
            break;
        }
    }
    assert!(completed, "partial resumes to inherited8KiB truncation");
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipts WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2,
        "no synthetic ordinary ACK"
    );
}

#[test]
fn presentation_v2_keeps_truncated_ordinary_cursor_progress_across_quanta() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    ordinary_message(&f, "multipage-ordinary", &"z".repeat(65536));
    ordinary_message(&f, "after-multipage", "following ordinary body");
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(first) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("first ordinary body missing")
    };
    assert_eq!(first.truncated, vec![MessageId::new("multipage-ordinary")]);
    assert!(first.text.contains(&"z".repeat(WATCH_BODY_LIMIT_BYTES)));
    let mut quiet = false;
    let mut following = false;
    for _ in 0..5 {
        let next = presentation_test_pull(&f, &recipient);
        if let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) = &next {
            assert!(
                !page
                    .truncated
                    .contains(&MessageId::new("multipage-ordinary")),
                "no repeated truncated body"
            );
            if page.offered.contains(&MessageId::new("after-multipage")) {
                assert!(page.text.contains("following ordinary body"));
                following = true;
            }
        }
        if matches!(
            next,
            crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. }
        ) {
            quiet = true;
            break;
        }
    }
    assert!(
        following || first.offered.contains(&MessageId::new("after-multipage")),
        "following fetched item retained"
    );
    assert!(
        quiet,
        "already displayed truncated body remainder advances without reassembly failure or repeated display"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM receipts WHERE message_id='multipage-ordinary'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
}

#[test]
fn presentation_refresh_observes_lazy_context_and_removal_without_consuming_output() {
    use crate::protocol::commands::{Accept, SetTopic};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "refresh-inviter", 0),
        "refresh-inviter-ci",
    );
    let thread = canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 3, "refresh");
    let wake = wake_attention(&f);
    let refresh = || {
        f.store
            .presentation_refresh(
                &recipient.seat,
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .expect("canonical bounded passive refresh available")
    };
    let mut initial = refresh();
    for _ in 0..8 {
        assert!(initial.context_visits <= 40 && initial.lifecycle_visits <= 20);
        if !initial.more {
            break;
        }
        initial = refresh();
    }
    assert!(initial.version > 0);
    assert!(!initial.more);
    let before = initial.version;
    assert_eq!(refresh().version, before, "quiet content does not churn");
    assert_eq!(f.db.query_row("SELECT initial_after FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"maintenance never consumes presentation discovery");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND (complete_count!=0 OR partial_count!=0)",[],|r|r.get::<_,i64>(0)).unwrap(),0,"maintenance creates no display offers");
    f.service
        .handle(
            Command::SetTopic(SetTopic {
                thread: thread.clone(),
                topic: "latest passive refresh topic".into(),
                operation: OperationId::new("refresh-topic"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let changed = refresh();
    assert!(changed.version > before);
    assert_eq!(refresh().version, changed.version);
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("refresh-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let removed = refresh();
    assert!(
        removed.version > changed.version,
        "removal invalidates passive content"
    );
    assert_eq!(refresh().version, removed.version);
    assert_eq!(
        wake_attention(&f),
        wake,
        "passive maintenance and lazy acceptance do not manufacture wake attention"
    );
}

#[test]
fn presentation_refresh_preserves_live_presentation_header_and_outbox_across_rebind() {
    use crate::{
        ports::PresentationInput,
        protocol::delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer},
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "refresh-preserve-inviter", 0),
        "refresh-preserve-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "refresh-preserve");
    let pull = |claim: CallerClaim| {
        f.store
            .presentation_pull(
                PresentationInput {
        ordinary_due_snapshot: None,
                    frozen_render: presentation_watch_frozen(&f, &claim),
                    render_profile:
                        crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
                    pull: DeliveryPullV2 {
                        claim,
                        consumer: PresentationConsumer::Watch,
                        render:
                            crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        marker_prefix: vec![],
                    },
                    attention_version: 0,
                    ordinary_version: 0,
                    with_attention: false,
                    retry_messages: false,
                    blocked_messages: vec![],
                    known_messages: vec![],
                    new_message_slots: 256,
                },
                &budget(),
            )
            .unwrap()
            .reply
    };
    let DeliveryPullReplyV2::Page(first) = pull(recipient.clone()) else {
        panic!("initial full context")
    };
    assert_eq!(presentation_invitations(&first).len(), 1);
    assert!(!presentation_invitations(&first)[0].content.truncated);
    let snapshot = || {
        f.db.query_row("SELECT json_object('generation',generation,'execution',execution,'initial_after',initial_after,'initial_high',initial_high,'context_after_seq',context_after_seq,'revisit_after_bytes',revisit_after_bytes,'rotation',rotation,'outbox_token',outbox_token,'outbox_text',outbox_text,'outbox_descriptors',outbox_descriptors,'counters',(SELECT json_group_array(json_array(generation,execution,context_key,partial_count,complete_count,output_class)) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='watch')) FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|r.get::<_,String>(0)).unwrap()
    };
    let before = snapshot();
    f.store
        .presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot(),
        before,
        "same-binding maintenance preserves allocation and replay"
    );
    let next = rotate(&f, &recipient, "refresh-new-native", 2);
    assert_eq!(
        snapshot(),
        before,
        "canonical rebind itself does not rewrite presentation rows"
    );
    f.store
        .presentation_refresh(
            &next.seat,
            next.binding_generation,
            next.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot(),
        before,
        "maintenance must not perform presentation binding reset"
    );
    let DeliveryPullReplyV2::Page(fresh) = pull(next) else {
        panic!("actual presentation owns reset and redisplays full context")
    };
    assert_eq!(presentation_invitations(&fresh).len(), 1);
    assert!(!presentation_invitations(&fresh)[0].content.truncated);
    assert_ne!(fresh.presentation_token, first.presentation_token);
}

#[test]
fn presentation_refresh_stub_allows_actual_watch_full_offer_and_replaced_owner_replay() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            delivery::{
                DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer,
            },
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        claim("c", "pc", Harness::Claude, "watch-full-context", 0),
        "watch-full-ci",
    );
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "watch-full-inviter", 0),
        "watch-full-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "watch-full");
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let refresh = f
        .store
        .presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert!(refresh.version > 0);
    let (first, _) = f
        .service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            Arc::new(Sink),
            &budget(),
        )
        .unwrap();
    let old = registry.presentation_lease(first).unwrap();
    let pull = |lease: String| match f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Watch,
                render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some(lease),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    {
        CommandResult::DeliveryPulledV2(reply) => reply,
        other => panic!("{other:?}"),
    };
    let DeliveryPullReplyV2::Page(full) = pull(old.clone()) else {
        panic!("mod-only stub must not hide first full output")
    };
    assert_eq!(presentation_invitations(&full).len(), 1);
    assert!(!presentation_invitations(&full)[0].content.truncated);
    assert!(
        presentation_invitations(&full)[0]
            .content
            .body
            .contains("review")
    );
    let (second, _) = f
        .service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            Arc::new(Sink),
            &budget(),
        )
        .unwrap();
    let new = registry.presentation_lease(second).unwrap();
    let DeliveryPullReplyV2::Page(replay) = pull(new.clone()) else {
        panic!("replacement reuses current outbox")
    };
    assert_eq!(replay, full);
    let confirm = |lease: String| {
        f.service
            .handle(
                Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    token: full.presentation_token.clone().unwrap(),
                    emitted_controls: full.emitted_controls.clone(),
                    presentation_lease: Some(lease),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    };
    assert!(matches!(
        confirm(old),
        CommandResult::DeliveryPresentationConfirmedV2(
            crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 {
                status: DeliveryPresentationConfirmationV2::StaleBinding,
                ..
            }
        )
    ));
    assert!(matches!(
        confirm(new),
        CommandResult::DeliveryPresentationConfirmedV2(
            crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 {
                status: DeliveryPresentationConfirmationV2::Confirmed,
                ..
            }
        )
    ));
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE seat_id='c' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "no invitation acceptance or fake ACK"
    );
}

#[test]
fn presentation_final_watch_codec_counts_escaped_canonical_invitation_batch() {
    use crate::{
        ports::PresentationInput,
        protocol::{
            commands::{CreateThread, InvitationConfirmation, Invite},
            delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer},
            watch::{WatchItem, WatchLine},
        },
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "watch-frame-inviter", 0),
        "watch-frame-ci",
    );
    let topic = "\0".repeat(300);
    let goal = "\0".repeat(300);
    for n in 0..24 {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: topic.clone(),
                    goal: goal.clone(),
                    operation: OperationId::new(format!("watch-frame-create-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("canonical thread")
        };
        f.service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::Lazy),
                    thread,
                    seat: recipient.seat.clone(),
                    deadline_millis: None,
                    operation: OperationId::new(format!("watch-frame-invite-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let wake = wake_attention(&f);
    let mut displayed = std::collections::HashSet::new();
    for _ in 0..24 {
        let result = f
            .store
            .presentation_pull(
                PresentationInput {
        ordinary_due_snapshot: None,
                    frozen_render: presentation_watch_frozen(&f, &recipient),
                    render_profile:
                        crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
                    pull: DeliveryPullV2 {
                        claim: recipient.clone(),
                        consumer: PresentationConsumer::Watch,
                        render:
                            crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        marker_prefix: vec![],
                    },
                    attention_version: 0,
                    ordinary_version: 0,
                    with_attention: false,
                    retry_messages: false,
                    blocked_messages: vec![],
                    known_messages: vec![],
                    new_message_slots: 256,
                },
                &budget(),
            )
            .unwrap();
        let DeliveryPullReplyV2::Page(page) = result.reply else {
            break;
        };
        let mut final_bytes = 0;
        for item in &presentation_invitations(&page) {
            assert!(!item.content.truncated, "standalone full context fits");
            assert_eq!(
                item.content.body.matches(&topic).count(),
                2,
                "exact canonical topic and goal preserved"
            );
            final_bytes += serde_json::to_vec(&WatchLine::new(
                format!("invitation:{}", item.id.as_str()),
                WatchItem::Invitation(item.content.clone()),
            ))
            .unwrap()
            .len()
                + 1;
            displayed.insert(item.id.clone());
        }
        assert!(
            final_bytes <= 65536,
            "final Watch JSON-lines batch is {final_bytes} bytes"
        );
        if let Some(token) = page.presentation_token {
            let result = f
                .store
                .presentation_confirm(
                    &crate::protocol::delivery::DeliveryPresentationConfirmV2 {
                        claim: recipient.clone(),
                        consumer: PresentationConsumer::Watch,
                        token,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        emitted_controls: page.emitted_controls,
                    },
                    &budget(),
                )
                .unwrap();
            assert!(matches!(result.status,crate::protocol::delivery::DeliveryPresentationConfirmationV2::Confirmed|crate::protocol::delivery::DeliveryPresentationConfirmationV2::AlreadyConfirmed));
        }
    }
    assert_eq!(
        displayed.len(),
        24,
        "every canonical invitation eventually full"
    );
    assert_eq!(
        wake_attention(&f),
        wake,
        "lazy output never creates wake work"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        24
    );
}

#[test]
fn presentation_final_cleanup_preserves_passive_removal_observation() {
    use crate::{
        ports::PresentationInput,
        protocol::{
            commands::Accept,
            delivery::{DeliveryPullV2, PresentationConsumer},
        },
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "cleanup-inviter", 0),
        "cleanup-ci",
    );
    let thread = canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "cleanup-order");
    let refresh = || {
        f.store
            .presentation_refresh(
                &recipient.seat,
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap()
    };
    let initial = refresh();
    assert!(initial.version > 0);
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("cleanup-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    // Presentation cleanup wins the race with passive maintenance.
    f.store
        .presentation_pull(
            PresentationInput {
        ordinary_due_snapshot: None,
                frozen_render: presentation_watch_frozen(&f, &recipient),
                render_profile:
                    crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
                pull: DeliveryPullV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                    marker_prefix: vec![],
                },
                attention_version: 0,
                ordinary_version: 0,
                with_attention: false,
                retry_messages: false,
                blocked_messages: vec![],
                known_messages: vec![],
                new_message_slots: 256,
            },
            &budget(),
        )
        .unwrap();
    let removed = refresh();
    assert!(
        removed.version > initial.version,
        "removal must invalidate passive content even after presentation cleanup"
    );
    assert_eq!(refresh().version, removed.version, "removal observed once");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='watch'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"settled current row is reclaimed");
}

#[test]
fn presentation_registry_pass_drives_bounded_passive_refresh_before_full_pull() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        claim("c", "pc", Harness::Claude, "registry-refresh", 0),
        "registry-refresh-ci",
    );
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "registry-refresh-inviter", 0),
        "registry-refresh-inviter-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(UtcMillis(0)).unwrap();
    sink.0.lock().unwrap().clear();
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "registry-refresh");
    registry.pass(UtcMillis(1)).unwrap();
    let version:i64=f.db.query_row("SELECT COALESCE((SELECT mod_version FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'),0)",[],|r|r.get(0)).unwrap();
    assert!(
        version > 0,
        "actual registry worker must drive independent canonical maintenance"
    );
    let frames = sink.0.lock().unwrap().clone();
    assert!(
        matches!(frames.as_slice(), [WatchFrame::Attention { .. }]),
        "passive arrival requests exactly one in-turn drain: {frames:?}"
    );
    registry.pass(UtcMillis(2)).unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        frames,
        "quiet maintenance does not flood context"
    );
    assert_eq!(f.db.query_row("SELECT initial_after FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"worker has not consumed presentation discovery");
    let lease = registry.presentation_lease(channel).unwrap();
    let result = f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient,
                consumer: PresentationConsumer::Watch,
                render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some(lease),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = result else {
        panic!("actual full pull")
    };
    assert_eq!(presentation_invitations(&page).len(), 1);
    assert!(!presentation_invitations(&page)[0].content.truncated);
    assert!(
        presentation_invitations(&page)[0]
            .content
            .body
            .contains("goal: review")
    );
}

#[test]
fn presentation_private_partial_guards_reject_corrupt_supported_shapes() {
    use crate::protocol::results::ErrorCode;
    let cases = [
        (
            "cardinality",
            serde_json::json!({"one":"a","two":"b"}).to_string(),
        ),
        (
            "raw-limit",
            serde_json::json!({"raw":"\0".repeat(8192)}).to_string(),
        ),
        ("invalid-id", serde_json::json!({"é":"x"}).to_string()),
        (
            "serialized-limit",
            serde_json::json!({"raw":"\0".repeat(9000)}).to_string(),
        ),
        ("invalid-json", "not-json".into()),
    ];
    for (label, partial) in cases {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        presentation_test_pull(&f, &recipient);
        // Deliberate local corruption exercises canonical checked reads even
        // when an operator/older writer bypassed the schema's storage guard.
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_partial=?1,ordinary_done=0,ordinary_cursor=NULL WHERE seat_id='s' AND consumer='hook'",[partial]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let error = f
            .service
            .handle(
                Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                    claim: recipient,
                    consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                    render: crate::protocol::delivery::PresentationRenderRequest::Hook {
                        confirmable_selector: 0,
                        passive_selector: 0,
                    },
                    presentation_lease: None,
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreCorrupt, "{label}: {error:?}");
        assert!(f.db.query_row("SELECT outbox_token IS NULL FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,bool>(0)).unwrap(),"failed allocation has no token");
    }
}

#[test]
fn presentation_private_skipped_body_requires_exact_retained_cursor() {
    use crate::protocol::results::ErrorCode;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    presentation_test_pull(&f, &recipient);
    f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_skip_body='missing-body',ordinary_done=0,ordinary_cursor=NULL WHERE seat_id='s' AND consumer='hook'",[]).unwrap();
    let error = f
        .service
        .handle(
            Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                claim: recipient,
                consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                render: crate::protocol::delivery::PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
    assert!(error.detail.contains("skipped body cursor"));
}

#[test]
fn presentation_ordinary_carry_survives_completed_control_partial_and_invitation_priority() {
    use crate::protocol::delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    ordinary_message(&f, "carry-prefix", &"a".repeat(20000));
    ordinary_message(&f, "carry-controls", &"\0".repeat(9000));
    for n in 0..6 {
        ordinary_message(&f, &format!("carry-after-{n}"), &"b".repeat(8192));
    }
    let first = presentation_test_pull(&f, &recipient);
    assert!(matches!(first, DeliveryPullReplyV2::Page(_)));
    let partial:String=f.db.query_row("SELECT ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let partial: std::collections::HashMap<String, String> =
        serde_json::from_str(&partial).unwrap();
    assert!(
        partial.contains_key("carry-controls"),
        "actual query split retains control-heavy partial"
    );
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "carry-inviter", 0),
        "carry-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 24, "carry-priority");
    let prefix = vec!["p".repeat(4096)];
    let mut seen = std::collections::HashSet::new();
    for _ in 0..24 {
        let result=f.service.handle(Command::DeliveryPullV2(DeliveryPullV2{claim:recipient.clone(),consumer:PresentationConsumer::Hook,render:crate::protocol::delivery::PresentationRenderRequest::Hook{confirmable_selector:0,passive_selector:0},presentation_lease:None,marker_prefix:prefix.clone()}),peer(),&budget()).expect("valid bounded ordinary query plus prior partial must persist withheld ordinary items");
        let CommandResult::DeliveryPulledV2(reply) = result else {
            panic!("pull result")
        };
        let DeliveryPullReplyV2::Page(page) = reply else {
            break;
        };
        for id in page.offered.iter().chain(&page.truncated) {
            seen.insert(id.as_str().to_owned());
        }
        presentation_test_confirm(&f, &recipient, &page);
    }
    assert!(
        seen.contains("carry-controls"),
        "completed partial eventually presents inherited truncated body"
    );
    for n in 0..6 {
        assert!(
            seen.contains(&format!("carry-after-{n}")),
            "following fetched ordinary item retained"
        );
    }
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipts WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        8,
        "presentation is no ACK"
    );
}

#[test]
fn presentation_raw_carry_serializer_accounting_witness() {
    use crate::{
        delivery::{Assembled, Assembler},
        protocol::{
            ids::{MessageId, SeatId, ThreadId},
            results::InboxBatchV2Item,
        },
    };
    let prefix = vec!["\0".repeat(4096)];
    assert_eq!(serde_json::to_vec(&prefix).unwrap().len(), 24580);
    // Eight arguments maximize independent quote/comma overhead at fixed raw bytes.
    let max_prefix = vec!["\0".repeat(512); 8];
    assert_eq!(serde_json::to_vec(&max_prefix).unwrap().len(), 24601);
    let mut max_delta = 0isize;
    for topic in [
        "".into(),
        "x".repeat(121),
        "é".repeat(121),
        "\0".repeat(121),
        "\"".repeat(121),
        "\u{2028}".repeat(121),
    ] {
        for lazy in [false, true] {
            let message = MessageId::parse("\"".repeat(128)).unwrap();
            let sender = Some(SeatId::parse("\"".repeat(128)).unwrap());
            let item = if lazy {
                InboxBatchV2Item::LazyMessage {
                    thread: ThreadId::parse("t").unwrap(),
                    topic_data: topic.clone(),
                    message: message.clone(),
                    sequence: 1,
                    sender: sender.clone(),
                    author_role: None,
                    relays_user: false,
                    user_intent: None,
                    author_role_backfilled: false,
                    body: "\0".repeat(8192),
                    body_start: 0,
                    body_end: 8192,
                    body_len: 8193,
                    wakes: false,
                }
            } else {
                InboxBatchV2Item::Message {
                    thread: ThreadId::parse("t").unwrap(),
                    topic_data: topic.clone(),
                    message: message.clone(),
                    sequence: 1,
                    sender: sender.clone(),
                    author_role: None,
                    relays_user: false,
                    user_intent: None,
                    author_role_backfilled: false,
                    body: "\0".repeat(8192),
                    body_start: 0,
                    body_end: 8192,
                    body_len: 8193,
                    ack_candidate: None,
                }
            };
            let source_len = serde_json::to_vec(&item).unwrap().len();
            let mut assembler = Assembler::new(None).with_command_prefix(prefix.clone());
            let Assembled::Item(mut assembled) = assembler.push(item).unwrap() else {
                panic!("completed supported chunk")
            };
            let marker = crate::protocol::watch::truncation_marker_for(&prefix, &message, lazy);
            assert!(assembled.message.truncated);
            assembled.message.body = assembled
                .message
                .body
                .strip_suffix(&marker)
                .expect("producer-owned exact generated marker")
                .to_owned();
            assert_eq!(assembled.message.body, "\0".repeat(8192));
            let raw = serde_json::json!({"marker_lazy":lazy,"content":{"id":message,"lazy":lazy,"content":assembled.message}});
            let delta = serde_json::to_vec(&raw).unwrap().len() as isize - source_len as isize;
            max_delta = max_delta.max(delta);
            assert!(delta <= 512, "metadata delta {delta}");
        }
    }
    // Page bytes + one prior raw partial + per-item metadata + one prefix + shell.
    let derived = 65536 + 6 * (8192 - 1) + 32 * 512 + 24601 + 64;
    assert_eq!(derived, 155731);
    eprintln!("raw carry metadata witness max_delta={max_delta}, derived_cap={derived}");
}

#[test]
fn presentation_current_handler_derives_timed_invitation_attention() {
    use crate::protocol::commands::{CreateThread, InvitationConfirmation, Invite};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "relevance-inviter", 0),
        "relevance-inviter-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "timed invitation".into(),
                goal: "review".into(),
                operation: OperationId::new("relevance-thread"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("relevance-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("actual pending invitation page")
    };
    assert!(
        page.text.contains("[herdr-threads] attention "),
        "actual handler must derive canonical confirming invitation relevance, not digest-free placeholder metadata: {}",
        page.text
    );
}

#[test]
fn presentation_promoted_lazy_raw_carry_survives_restart_with_frozen_prefix() {
    use crate::protocol::delivery::{
        DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationItemV2,
        PresentationRenderRequest,
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    f.db.execute("UPDATE seats SET wake_on='all' WHERE id='s'", [])
        .unwrap();
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "promoted-carry-inviter", 0),
        "promoted-carry-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "promoted-baseline");
    let DeliveryPullReplyV2::Page(baseline) = presentation_test_pull(&f, &recipient) else {
        panic!("baseline invitation")
    };
    presentation_test_confirm(&f, &recipient, &baseline);
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 24, "promoted-priority");
    lazy_message(&f, "msg-promoted-carry", &"\0".repeat(9000));
    let old_prefix = vec!["p".repeat(4096)];
    let pull = |f: &Fixture, prefix: Vec<String>| match f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                render: PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: prefix,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    {
        CommandResult::DeliveryPulledV2(reply) => reply,
        other => panic!("{other:?}"),
    };
    let DeliveryPullReplyV2::Page(first) = pull(&f, old_prefix.clone()) else {
        panic!("priority page")
    };
    let json:String=f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let carry: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(carry["prefix"], serde_json::json!(old_prefix));
    let row = carry["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["content"]["id"] == "msg-promoted-carry")
        .expect("actual promoted lazy withheld carry");
    assert_eq!(row["marker_lazy"], true);
    assert_eq!(
        row["content"]["lazy"], false,
        "promotion changes output type only"
    );
    assert_eq!(row["content"]["content"]["body"], "\0".repeat(8192));
    presentation_test_confirm(&f, &recipient, &first);
    restart(&mut f);
    let mut emitted = None;
    for _ in 0..24 {
        let DeliveryPullReplyV2::Page(page) = pull(&f, vec!["new-prefix".into()]) else {
            break;
        };
        for item in &page.items {
            if let PresentationItemV2::Message { id, lazy, content } = item
                && id.as_str() == "msg-promoted-carry"
            {
                assert!(!lazy);
                emitted = Some(content.body.clone());
            }
        }
        presentation_test_confirm(&f, &recipient, &page);
        if emitted.is_some() {
            break;
        }
    }
    let expected = format!(
        "{}{}",
        "\0".repeat(8192),
        crate::protocol::watch::truncation_marker_for(
            &old_prefix,
            &MessageId::new("msg-promoted-carry"),
            true
        )
    );
    assert_eq!(
        emitted,
        Some(expected),
        "restart and new request prefix preserve original presentation bytes"
    );
    assert_eq!(
        lazy_state(&f, "msg-promoted-carry"),
        "pending",
        "presentation is no lazy completion"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipts WHERE message_id='msg-promoted-carry'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

// Private fixed-state transition coverage; actual canonical Attention allocation
// is separately exercised by the current-handler relevance regression.
#[test]
fn presentation_confirmation_preserves_original_slots_and_exact_subsets_on_restart() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2 as Status,
        PresentationConsumer, PresentationControlStampV2 as Stamp, PresentationItemV2 as Item,
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "confirm-slot-inviter", 0),
        "confirm-slot-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "confirm-slot");
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("canonical invitation page")
    };
    let token = page.presentation_token.clone().unwrap();
    let mut descriptors:serde_json::Value=serde_json::from_str(&f.db.query_row("SELECT outbox_descriptors FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap()).unwrap();
    let attention = Stamp::Attention {
        epoch: "1".into(),
        version: 9,
    };
    descriptors.as_array_mut().unwrap().push(serde_json::json!({"item":Item::Attention{epoch:"1".into(),attention_version:9,text:"ht inbox".into()},"context_key":null}));
    let controls = serde_json::json!({"counter":"1","last_exact":true,"active_epoch":"1","allocated_attention":attention});
    f.db.execute("UPDATE invitation_presentation_consumers SET outbox_descriptors=?1,control_state=?2 WHERE seat_id='s' AND consumer='hook'",rusqlite::params![descriptors.to_string(),controls.to_string()]).unwrap();
    let confirm = |f: &Fixture, stamps: Vec<Stamp>| {
        f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                token: token.clone(),
                presentation_lease: None,
                emitted_controls: stamps,
            },
            &budget(),
        )
    };
    let before=f.db.query_row("SELECT outbox_descriptors||control_state||outbox_retired_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap();
    for stamps in [
        vec![Stamp::Attention {
            epoch: "1".into(),
            version: 10,
        }],
        vec![attention.clone(), attention.clone()],
    ] {
        assert!(confirm(&f, stamps).is_err());
        assert_eq!(f.db.query_row("SELECT outbox_descriptors||control_state||outbox_retired_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap(),before,"invalid subset has no effects");
    }
    let first = confirm(&f, vec![]).unwrap();
    assert_eq!(first.status, Status::Confirmed);
    assert!(first.acknowledged_controls.is_empty());
    let retained=f.db.query_row("SELECT outbox_token,outbox_descriptors,outbox_retired_mask,outbox_confirmed_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap();
    assert_eq!(
        retained,
        (token.clone(), descriptors.to_string(), 1, 1),
        "slot1 survives original slot0 retirement without compaction"
    );
    restart(&mut f);
    assert_eq!(
        confirm(&f, vec![]).unwrap().status,
        Status::AlreadyConfirmed,
        "old empty completion cannot consume newly visible control"
    );
    let exact = confirm(&f, vec![attention.clone()]).unwrap();
    assert_eq!(exact.status, Status::Confirmed);
    assert_eq!(exact.acknowledged_controls, vec![attention.clone()]);
    let retry = confirm(&f, vec![attention.clone()]).unwrap();
    assert_eq!(retry.status, Status::AlreadyConfirmed);
    assert_eq!(retry.acknowledged_controls, vec![attention.clone()]);
    f.db.execute("UPDATE invitation_presentation_consumers SET control_state=?1 WHERE seat_id='s' AND consumer='hook'",[serde_json::json!({"counter":"2","last_exact":true,"active_epoch":"2"}).to_string()]).unwrap();
    assert!(
        confirm(&f, vec![attention])
            .unwrap()
            .acknowledged_controls
            .is_empty(),
        "last-token retry cannot resurrect obsolete epoch"
    );
}

#[test]
fn presentation_actual_candidate_sizing_and_rpc_wrapper_work_remain_bounded() {
    use crate::{
        delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
        ports::PresentationInput,
        protocol::delivery::{
            DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        },
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "sizing-bound-inviter", 0),
        "sizing-bound-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 100, "sizing-bound");
    let registry = crate::harness::registry::builtins();
    let adapter = registry
        .by_id(registry.agent(recipient.harness.as_str()).unwrap())
        .unwrap();
    let codec = adapter.presentation_codec(0).unwrap();
    let profile = PresentationRenderProfile::Hook {
        confirmable_selector: 0,
        passive_selector: 0,
        confirmable: codec,
        passive: codec,
    };
    let mut all = std::collections::HashSet::new();
    let mut max_passes = 0;
    for _ in 0..20 {
        let frozen_render = f
            .store
            .presentation_render_selection(&recipient, PresentationConsumer::Hook, &budget())
            .unwrap()
            .map(|identity| FrozenPresentationRender {
                identity,
                codec: Some(codec),
            });
        let output = f
            .store
            .presentation_pull(
                PresentationInput {
        ordinary_due_snapshot: None,
                    frozen_render,
                    render_profile: profile.clone(),
                    pull: DeliveryPullV2 {
                        claim: recipient.clone(),
                        consumer: PresentationConsumer::Hook,
                        render: PresentationRenderRequest::Hook {
                            confirmable_selector: 0,
                            passive_selector: 0,
                        },
                        presentation_lease: None,
                        marker_prefix: vec![],
                    },
                    attention_version: u64::MAX,
                    ordinary_version: 0,
                    with_attention: false,
                    retry_messages: false,
                    blocked_messages: vec![],
                    known_messages: vec![],
                    new_message_slots: 256,
                },
                &budget(),
            )
            .unwrap();
        let work = &output.work;
        assert!(
            work.initial <= 40
                && work.context <= 80 - work.initial
                && work.revisit <= 100 - work.initial - work.context
                && work.initial + work.context + work.revisit <= 100
                && work.ordinary <= 100
                && work.notices <= 17
                && work.notice_validations <= 16
                && work.replay <= 32,
            "{work:?}"
        );
        assert!(
            work.output_passes <= 352
                && work.output_bytes <= 352 * 65537
                && work.rpc_passes == 1
                && work.rpc_bytes <= crate::delivery::presentation::RPC_RESULT_LIMIT + 1
        );
        max_passes = max_passes.max(work.output_passes);
        let result = CommandResult::DeliveryPulledV2(output.reply.clone());
        let encoded_result = serde_json::to_vec(&result).unwrap();
        assert_eq!(work.rpc_bytes, encoded_result.len());
        let response = crate::protocol::wire::WireResponse {
            version: crate::protocol::wire::PROTOCOL_VERSION,
            request_id: "\"".repeat(128),
            instance: uuid::Uuid::from_u128(1).to_string(),
            daemon_boot: uuid::Uuid::from_u128(2).to_string(),
            result: Ok(result),
        };
        let encoded = crate::protocol::wire::encode_wire_response(&response).unwrap();
        assert!(
            serde_json::from_slice::<crate::protocol::wire::WireResponse>(&encoded[4..]).is_ok()
        );
        assert_eq!(
            u32::from_be_bytes(encoded[..4].try_into().unwrap()) as usize,
            encoded.len() - 4
        );
        assert!(
            encoded.len() - encoded_result.len() <= 512
                && encoded.len() <= crate::protocol::wire::MAX_WIRE_FRAME_BYTES
        );
        let DeliveryPullReplyV2::Page(page) = output.reply else {
            break;
        };
        let (_, actual) = crate::delivery::presentation::measure(&profile, &page.items).unwrap();
        assert_eq!(actual, page.output_bytes as usize);
        for invitation in presentation_invitations(&page) {
            if !invitation.content.truncated {
                all.insert(invitation.id);
            }
        }
        presentation_test_confirm(&f, &recipient, &page);
        if all.len() == 100 {
            break;
        }
    }
    assert_eq!(all.len(), 100);
    assert!(
        max_passes > 32,
        "actual candidate sizing, not only one final serializer"
    );
    eprintln!(
        "actual_candidate_max_output_passes={max_passes}; declared_bound=352; wrapper_reserve=512"
    );
}

#[test]
fn presentation_handler_clears_unconfirmed_epoch_then_reopens_new_episode() {
    use crate::protocol::{
        commands::{Accept, CreateThread, InvitationConfirmation, Invite},
        delivery::{DeliveryPullReplyV2, PresentationControlStampV2 as Stamp},
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "episode-inviter", 0),
        "episode-ci",
    );
    let invite = |suffix: &str| {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: "episode".into(),
                    goal: "review".into(),
                    operation: OperationId::new(format!("episode-thread-{suffix}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("thread")
        };
        f.service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::RequireAck),
                    thread: thread.clone(),
                    seat: recipient.seat.clone(),
                    deadline_millis: Some(1000),
                    operation: OperationId::new(format!("episode-invite-{suffix}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        thread
    };
    let first_thread = invite("first");
    let DeliveryPullReplyV2::Page(first) = presentation_test_pull(&f, &recipient) else {
        panic!("first")
    };
    let first_stamp = first
        .emitted_controls
        .iter()
        .find(|v| matches!(v, Stamp::Attention { .. }))
        .unwrap()
        .clone();
    let Stamp::Attention {
        epoch: first_epoch, ..
    } = &first_stamp
    else {
        unreachable!()
    };
    // Acceptance closes the condition before any emission confirmation. Clear
    // must still retire the possibly emitted epoch, retaining original slots.
    f.service
        .handle(
            Command::Accept(Accept {
                thread: first_thread.clone(),
                operation: OperationId::new("episode-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    // Retained canonical warning-job rows whose publications have not arrived
    // force bounded proof progress before exact absence. They are fixture
    // source rows, not a claim that public commands created 301 warnings.
    f.db.execute("WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<301) INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'episode-unpublished-'||x,20,?1,1000000,'s','invitation',?2 FROM n",rusqlite::params![first_thread.as_str(),presentation_invitations(&first)[0].id.as_str()]).unwrap();
    let DeliveryPullReplyV2::Page(unknown) = presentation_test_pull(&f, &recipient) else {
        panic!("bounded Unknown replay")
    };
    assert_eq!(
        unknown.presentation_token, first.presentation_token,
        "Unknown must preserve original outbox identity"
    );
    assert!(
        unknown.emitted_controls.is_empty() && unknown.more,
        "Unknown withholds Attention while progressing proof"
    );
    let mut clear = None;
    let mut diagnostics = Vec::new();
    // Two indexed sources each retain301 rows. Allow one source and one
    // fallback microstep per row, each scheduled within13 eligible calls,
    // plus13 startup slots. This is a finite test guard, not a delivery cap.
    let convergence_guard = 13 * (2 * 2 * 301 + 13);
    for quantum in 0..convergence_guard {
        let reply = presentation_test_pull(&f, &recipient);
        let work = f.service.presentation.last_work.lock().unwrap().unwrap();
        assert_eq!(
            work.relevance_calls, 1,
            "each successful handler call advances exactly once"
        );
        assert!(
            work.relevance_sources + work.relevance_fallback <= 100 && work.relevance_seeks <= 26
        );
        let fence: i64 =
            f.db.query_row(
                "SELECT version FROM presentation_warning_fence WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let proof:String=f.db.query_row("SELECT proof_json FROM presentation_warning_proofs WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
        let diagnostic = serde_json::json!({"quantum":quantum,"work":format!("{work:?}"),"fence":fence,"proof":serde_json::from_str::<serde_json::Value>(&proof).unwrap()});
        if diagnostics.len() < 32 {
            diagnostics.push(diagnostic);
        } else {
            diagnostics[31] = diagnostic;
        }
        match reply {
            DeliveryPullReplyV2::Page(page) => {
                if page
                    .emitted_controls
                    .iter()
                    .any(|v| matches!(v,Stamp::Cleared{epoch,..} if epoch==first_epoch))
                {
                    clear = Some(page);
                    break;
                }
                assert!(
                    !page.emitted_controls.contains(&first_stamp),
                    "closed Attention cannot replay"
                );
                if !page.items.is_empty() {
                    presentation_test_confirm(&f, &recipient, &page);
                }
            }
            DeliveryPullReplyV2::Unchanged { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    let clear = clear.unwrap_or_else(|| {
        panic!(
            "unconfirmed allocated Attention receives exact scoped clear; diagnostics={}",
            serde_json::to_string(&diagnostics).unwrap()
        )
    });
    assert!(!clear.emitted_controls.contains(&first_stamp));
    presentation_test_confirm(&f, &recipient, &clear);
    invite("second");
    let mut reopened = None;
    for _ in 0..32 {
        if let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) {
            reopened = page
                .emitted_controls
                .iter()
                .find(|v| matches!(v, Stamp::Attention { .. }))
                .cloned();
            presentation_test_confirm(&f, &recipient, &page);
            if reopened.is_some() {
                break;
            }
        }
    }
    let Stamp::Attention { epoch, .. } = reopened.expect("new canonical open receives new episode")
    else {
        unreachable!()
    };
    assert_ne!(
        &epoch, first_epoch,
        "presentation epoch never reuses a retired epoch"
    );
}

#[test]
fn presentation_handler_published_warn_reopens_at_unchanged_native_version() {
    use crate::{
        ports::{
            ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionGuard,
            ServiceWriteTransactionProof,
        },
        protocol::{
            commands::{Accept, CreateThread, InvitationConfirmation, Invite},
            delivery::{DeliveryPullReplyV2, PresentationControlStampV2 as Stamp},
            results::ApiError,
            service::{NotificationSeverity, ServiceNotify, ServiceOperation, ServiceResult},
        },
    };
    struct Gate(ServiceAuthorId);
    struct Guard(ServiceAuthorId);
    impl ServiceDecisionGuard for Guard {
        fn author(&self) -> &ServiceAuthorId {
            &self.0
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            assert_eq!(connection.author(), &self.0);
            Ok(Box::new(Guard(self.0.clone())))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            true
        }
    }
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "warn-epoch-inviter", 0),
        "warn-epoch-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "first epoch".into(),
                goal: "review".into(),
                operation: OperationId::new("warn-epoch-thread"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread: thread.clone(),
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("warn-epoch-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let DeliveryPullReplyV2::Page(first) = presentation_test_pull(&f, &recipient) else {
        panic!("first Attention")
    };
    let first_epoch = first
        .emitted_controls
        .iter()
        .find_map(|v| match v {
            Stamp::Attention { epoch, .. } => Some(epoch.clone()),
            _ => None,
        })
        .unwrap();
    presentation_test_confirm(&f, &recipient, &first);
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("warn-epoch-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let mut clear_version = None;
    for _ in 0..32 {
        if let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) {
            if page
                .emitted_controls
                .iter()
                .any(|v| matches!(v,Stamp::Cleared{epoch,..} if epoch==&first_epoch))
            {
                clear_version = Some(page.attention_version);
            }
            if !page.items.is_empty() {
                presentation_test_confirm(&f, &recipient, &page);
            }
            if clear_version.is_some() {
                break;
            }
        }
    }
    let prior_version = clear_version.expect("first exact epoch cleared");
    let native_version = || {
        f.db.query_row(
            "SELECT attention_version FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = native_version();
    let tx = f.db.unchecked_transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    let gate = Gate(author.clone());
    let connection = ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, author);
    let ServiceResult::Notification(published) = f
        .store
        .service_operation(
            ServiceOperation::Notify(ServiceNotify {
                thread: ThreadId::new("t"),
                severity: NotificationSeverity::Warn,
                event_json: serde_json::json!({"text":"new warning before materialization"}),
                operation: OperationId::new("warn-epoch-notify"),
            }),
            &connection,
            &gate,
            &budget(),
            None,
        )
        .unwrap()
    else {
        panic!("published Warn")
    };
    assert_eq!(
        native_version(),
        before,
        "canonical publication precedes its native materializer bump"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE id=?1 AND kind='warn'",
            [published.summary.message.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(f.db.query_row("SELECT count(*) FROM work_jobs WHERE kind='send_attention' AND subject_id LIKE 'service-notify:%' AND status='pending'",[],|r|r.get::<_,i64>(0)).unwrap(),1,"actual materialization remains pending");
    let mut reopened = None;
    for _ in 0..32 {
        if let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) {
            assert_eq!(
                page.attention_version, prior_version,
                "actual handler native metadata remains unchanged"
            );
            reopened = page.emitted_controls.iter().find_map(|v| match v {
                Stamp::Attention { epoch, .. } => Some(epoch.clone()),
                _ => None,
            });
            if !page.items.is_empty() {
                presentation_test_confirm(&f, &recipient, &page);
            }
            if reopened.is_some() {
                break;
            }
        }
    }
    assert_ne!(
        reopened.expect("published Warn reopens presentation relevance before materialization"),
        first_epoch
    );
    assert_eq!(
        native_version(),
        before,
        "V2 pull/Confirm never runs native materialization"
    );
}

#[test]
fn presentation_quiet_channel_observes_new_lazy_ordinary_without_wake() {
    use crate::{
        ports::{ModChannelSink, ModStoreReads},
        protocol::{
            commands::{DeliveryMode, SendMessage},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        claim("c", "pc", Harness::Claude, "quiet-lazy-recipient", 0),
        "quiet-lazy-ci",
    );
    let sender = check_in(
        &f,
        claim("x", "px", fourth(), "quiet-lazy-sender", 0),
        "quiet-lazy-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (_channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(UtcMillis(0)).unwrap();
    sink.0.lock().unwrap().clear();
    let native = || {
        f.db.query_row(
            "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='c'),0)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = native();
    f.service
        .handle(
            Command::SendMessage(SendMessage {
                delivery_mode: DeliveryMode::Lazy,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "new lazy content after quiet registration".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("quiet-lazy-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(f.db.query_row("SELECT count(*) FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id='c' AND r.state='pending'",[],|r|r.get::<_,i64>(0)).unwrap(),1,"actual canonical lazy publication");
    assert_eq!(
        native(),
        before,
        "ordinary Lazy does not create native attention"
    );
    registry.pass(UtcMillis(1)).unwrap();
    let frames = sink.0.lock().unwrap().clone();
    assert!(
        matches!(frames.as_slice(), [WatchFrame::Attention { .. }]),
        "new recipient-visible Lazy content needs one passive in-turn drain after quiet registration: {frames:?}"
    );
    registry.pass(UtcMillis(2)).unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        frames,
        "unchanged lazy content does not repeat passive drains"
    );
    assert_eq!(
        native(),
        before,
        "passive observation preserves native eligibility"
    );
}

#[test]
fn presentation_confirm_rejects_oversized_private_json_before_decoding() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for (field, limit) in [
        ("last_descriptors", 4096),
        ("control_state", 4096),
        ("outbox_descriptors", 262144),
        ("outbox_prefix", 24601),
        ("outbox_selection", 128),
        ("outbox_text", 65536),
        ("ordinary_pending", 155731),
        ("ordinary_partial", 49415),
        ("ordinary_version", 20),
        ("outbox_version", 20),
        ("ordinary_cursor", 8192),
        ("ordinary_skip_body", 128),
        ("execution", 128),
        ("mod_execution", 128),
        ("render_key", 64),
        ("outbox_render_key", 64),
        ("outbox_codec_key", 64),
    ] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            claim("x", "px", fourth(), "private-bound-inviter", 0),
            "private-bound-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "private-bound");
        let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
            panic!("canonical page")
        };
        presentation_test_confirm(&f, &recipient, &page);
        let original=f.db.query_row(&format!("SELECT {field} FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'"),[],|r|r.get::<_,Option<String>>(0)).unwrap().unwrap_or_default();
        let corrupted = original + &" ".repeat(limit + 1);
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='hook'"),[corrupted]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let result = f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient,
                consumer: PresentationConsumer::Hook,
                token: page.presentation_token.unwrap(),
                emitted_controls: vec![],
                presentation_lease: None,
            },
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "{field} must enforce declared{limit}byte bound before allocation/parsing/effects: {result:?}"
        );
    }
}

#[test]
fn presentation_last_token_control_serializer_has_exact_bounded_witness() {
    use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
    let stamps = (0..32)
        .map(|n| Stamp::Attention {
            epoch: (u64::MAX - n).to_string(),
            version: u64::MAX,
        })
        .collect::<Vec<_>>();
    crate::protocol::delivery::validate_control_stamps(&stamps).unwrap();
    for stamp in &stamps {
        assert_eq!(serde_json::to_vec(stamp).unwrap().len(), 82);
    }
    let serialized = serde_json::to_vec(&stamps).unwrap();
    assert_eq!(serialized.len(), 32 * 82 + 31 + 2);
    assert_eq!(serialized.len(), 2657);
    assert!(serialized.len() <= 4096);
    assert_eq!(
        serde_json::from_slice::<Vec<Stamp>>(&serialized).unwrap(),
        stamps
    );
}

#[test]
fn presentation_handler_partial_confirmation_retains_canonical_slots_across_restart() {
    use crate::protocol::{
        commands::{CreateThread, InvitationConfirmation, Invite},
        delivery::{
            DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2 as Status,
            DeliveryPullReplyV2, PresentationConsumer, PresentationControlStampV2 as Stamp,
            PresentationItemV2 as Item,
        },
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "public-subset-inviter", 0),
        "public-subset-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "canonical partial confirmation".into(),
                goal: "review".into(),
                operation: OperationId::new("public-subset-thread"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("public-subset-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("canonical page")
    };
    assert_eq!(page.emitted_controls.len(), 1);
    assert!(
        page.items
            .iter()
            .any(|v| matches!(v, Item::Invitation { .. }))
    );
    let token = page.presentation_token.clone().unwrap();
    let stamp = page.emitted_controls[0].clone();
    let row = |f: &Fixture| {
        f.db.query_row("SELECT outbox_descriptors,outbox_retired_mask,outbox_confirmed_mask,control_state,last_descriptors,last_confirmed_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?))).unwrap()
    };
    let original = row(&f);
    let descriptors: Vec<serde_json::Value> = serde_json::from_str(&original.0).unwrap();
    let control_slot = descriptors
        .iter()
        .position(|v| v["item"]["kind"] == "attention")
        .unwrap();
    let allowed = (1i64 << descriptors.len()) - 1;
    let noncontrols = allowed & !(1i64 << control_slot);
    let confirm = |f: &Fixture, emitted_controls: Vec<Stamp>| {
        f.service.handle(
            Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                token: token.clone(),
                presentation_lease: None,
                emitted_controls,
            }),
            peer(),
            &budget(),
        )
    };
    let CommandResult::DeliveryPresentationConfirmedV2(empty) = confirm(&f, vec![]).unwrap() else {
        panic!("confirm")
    };
    assert_eq!(empty.status, Status::Confirmed);
    assert!(empty.acknowledged_controls.is_empty());
    let partial = row(&f);
    assert_eq!(partial.0, original.0);
    assert_eq!((partial.1, partial.2), (noncontrols, noncontrols));
    restart(&mut f);
    let DeliveryPullReplyV2::Page(replay) = presentation_test_pull(&f, &recipient) else {
        panic!("canonical replay")
    };
    assert_eq!(replay.presentation_token, Some(token.clone()));
    assert_eq!(replay.emitted_controls, vec![stamp.clone()]);
    assert!(
        replay
            .items
            .iter()
            .all(|v| matches!(v, Item::Attention { .. }))
    );
    assert_eq!(
        row(&f),
        partial,
        "replay preserves original slots and masks"
    );
    let foreign = match &stamp {
        Stamp::Attention { epoch, version } => Stamp::Attention {
            epoch: epoch.clone(),
            version: version + 1,
        },
        _ => panic!("attention"),
    };
    for invalid in [vec![foreign], vec![stamp.clone(), stamp.clone()]] {
        assert!(confirm(&f, invalid).is_err());
        assert_eq!(
            row(&f),
            partial,
            "invalid capture cannot mutate slots or controls"
        );
    }
    let CommandResult::DeliveryPresentationConfirmedV2(reply) =
        confirm(&f, vec![stamp.clone()]).unwrap()
    else {
        panic!("confirm")
    };
    assert_eq!(reply.status, Status::Confirmed);
    assert_eq!(reply.acknowledged_controls, vec![stamp.clone()]);
    restart(&mut f);
    let CommandResult::DeliveryPresentationConfirmedV2(retry) =
        confirm(&f, vec![stamp.clone()]).unwrap()
    else {
        panic!("retry")
    };
    assert_eq!(retry.status, Status::AlreadyConfirmed);
    assert_eq!(
        retry.acknowledged_controls,
        vec![stamp],
        "lost reply retry acknowledges the original still-applicable capture"
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0,
        "presentation confirmation is not message ACK"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE accepted_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "presentation confirmation is not invitation acceptance"
    );
}

#[test]
fn presentation_private_state_rejects_typed_corruption_without_confirmation_effects() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for (field, value) in [
        ("last_descriptors", "null"),
        ("last_descriptors", "7"),
        ("last_descriptors", "["),
        ("control_state", "null"),
        ("control_state", "{\"unknown\":true}"),
        ("control_state", "{\"counter\":\"-1\"}"),
        ("last_confirmed_mask", "-1"),
        ("last_confirmed_mask", "1"),
    ] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            claim("x", "px", fourth(), "typed-corrupt-inviter", 0),
            "typed-corrupt-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "typed-corrupt");
        let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
            panic!("canonical page")
        };
        presentation_test_confirm(&f, &recipient, &page);
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='hook'"),[value]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let snapshot = || {
            f.db.query_row("SELECT last_token,last_descriptors,last_confirmed_mask,control_state FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient,
                consumer: PresentationConsumer::Hook,
                token: page.presentation_token.unwrap(),
                emitted_controls: vec![],
                presentation_lease: None,
            },
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "typed {field}={value} must refuse before effects: {result:?}"
        );
        assert_eq!(snapshot(), before);
        assert!(f.db.execute("UPDATE invitation_presentation_consumers SET last_descriptors=NULL WHERE seat_id='s' AND consumer='hook'",[]).is_err(),"STRICT NOT NULL rejects SQL NULL at storage boundary");
    }
}

#[test]
fn presentation_watch_fresh_confirmation_records_exact_owner_publication_cutoff_once() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            commands::{
                Ack, CreateThread, DeliveryMode, InvitationConfirmation, Invite, SendMessage,
            },
            delivery::{
                DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2 as Status,
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer,
                PresentationControlStampV2 as Stamp, PresentationRenderRequest,
            },
            watch::{MOD_STALL_AFTER_MS, ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        claim("c", "pc", Harness::Claude, "stall-v2-recipient", 0),
        "stall-v2-ci",
    );
    let sender = check_in(
        &f,
        claim("x", "px", fourth(), "stall-v2-sender", 0),
        "stall-v2-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            Arc::new(Sink),
            &budget(),
        )
        .unwrap();
    let lease = registry.presentation_lease(channel).unwrap();
    let send = |f: &Fixture, key: &str| {
        let CommandResult::MessageSent(id) = f
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Ordinary,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: key.into(),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(key),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("message")
        };
        id
    };
    let first_receipt = send(&f, "stall-v2-first");
    // Timed invitation supplies an independently canonical NUDGE condition;
    // this fixture proves mod health bookkeeping, not initial receipt relevance.
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "stall proof".into(),
                goal: "review".into(),
                operation: OperationId::new("stall-v2-thread"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("stall-v2-invite"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    f.clock
        .0
        .store(1_000_000 + MOD_STALL_AFTER_MS as i64 + 1, Ordering::Relaxed);
    let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Watch,
                render: PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some(lease.clone()),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("Watch page")
    };
    assert_eq!(
        page.emitted_controls
            .iter()
            .filter(|v| matches!(v, Stamp::Attention { .. }))
            .count(),
        1,
        "single-control producer dependency of fresh status"
    );
    assert_eq!(page.emitted_controls.len(), 1);
    assert!(
        !registry.stalled(&recipient.seat, f.clock.utc_now()),
        "allocation/replay is not physical emission evidence; no legacy observer pass in this fixture"
    );
    let request = DeliveryPresentationConfirmV2 {
        claim: recipient.clone(),
        consumer: PresentationConsumer::Watch,
        token: page.presentation_token.unwrap(),
        presentation_lease: Some(lease),
        emitted_controls: page.emitted_controls,
    };
    let CommandResult::DeliveryPresentationConfirmedV2(first) = f
        .service
        .handle(
            Command::DeliveryPresentationConfirmV2(request.clone()),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("confirm")
    };
    assert_eq!(first.status, Status::Confirmed);
    assert_eq!(first.acknowledged_controls, request.emitted_controls);
    assert!(
        registry.stalled(&recipient.seat, f.clock.utc_now()),
        "fresh exact Watch emission confirmation records receipt-publication cutoff, not a new countdown"
    );
    // Drop the first response as a lost-reply simulation. Isolate the original
    // qualifying receipt via a REAL ACK before testing the later cutoff.
    f.service
        .handle(
            Command::Ack(Ack {
                messages: vec![first_receipt],
                operation: OperationId::new("stall-v2-real-ack"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    f.clock.0.fetch_add(1000, Ordering::Relaxed);
    let _later_receipt = send(&f, "stall-v2-later");
    f.clock
        .0
        .fetch_add(MOD_STALL_AFTER_MS as i64 + 1, Ordering::Relaxed);
    assert!(
        !registry.stalled(&recipient.seat, f.clock.utc_now()),
        "later receipt is after original push cutoff; prior receipt settled and real ACK is now old"
    );
    let CommandResult::DeliveryPresentationConfirmedV2(retry) = f
        .service
        .handle(
            Command::DeliveryPresentationConfirmV2(request),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("retry")
    };
    assert_eq!(retry.status, Status::AlreadyConfirmed);
    assert!(!retry.acknowledged_controls.is_empty());
    assert!(
        !registry.stalled(&recipient.seat, f.clock.utc_now()),
        "AlreadyConfirmed lost-reply retry must not move publication cutoff"
    );
}

#[test]
fn presentation_publication_scalar_maximum_roundtrips_without_changing_carry() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "scalar-inviter", 0),
        "scalar-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "scalar");
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("page")
    };
    let carry:String=f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let fields = [
        "ordinary_publication_after",
        "ordinary_publication_high",
        "ordinary_passive_generation",
        "ordinary_scan_passive_generation",
    ];
    for field in fields {
        assert_eq!(f.db.query_row(&format!("SELECT {field} FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_publication_after=?1,ordinary_publication_high=?1,ordinary_passive_generation=?1,ordinary_scan_passive_generation=?1 WHERE seat_id='s' AND consumer='hook'",[i64::MAX]).unwrap();
    presentation_test_confirm(&f, &recipient, &page);
    let mut map = serde_json::Map::new();
    for field in fields {
        let(value,kind)=f.db.query_row(&format!("SELECT {field},typeof({field}) FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'"),[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap();
        assert_eq!((value, kind), (i64::MAX, "integer".into()));
        map.insert(field.into(), serde_json::json!(value));
    }
    assert_eq!(serde_json::to_vec(&map).unwrap().len(), 203);
    let with_other = serde_json::json!({"existing":0});
    let old = serde_json::to_vec(&with_other).unwrap().len();
    let mut with_other = with_other.as_object().unwrap().clone();
    with_other.extend(map);
    assert_eq!(serde_json::to_vec(&with_other).unwrap().len() - old, 202);
    assert_eq!(f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap(),carry,"four scalars add zero carry bytes");
}

#[test]
fn presentation_publication_walk_counts_foreign_prefix_before_filter_and_progresses() {
    use crate::protocol::commands::{CreateThread, DeliveryMode, SendMessage};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let sender = check_in(
        &f,
        claim("x", "px", fourth(), "prefix-sender", 0),
        "prefix-sender-ci",
    );
    let CommandResult::ThreadCreated(foreign) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "foreign prefix".into(),
                goal: "bounded walk".into(),
                operation: OperationId::new("prefix-thread"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    let send = |thread: ThreadId, n: usize| {
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread,
                    body: format!("prefix-body-{n}"),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("prefix-send-{n}")),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    };
    for n in 0..101 {
        send(foreign.clone(), n);
    }
    send(ThreadId::new("t"), 101);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM send_manifests WHERE instance_id='i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        102
    );
    let native = wake_attention(&f);
    let refresh = || {
        f.store
            .presentation_refresh(
                &recipient.seat,
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap()
    };
    let first = refresh();
    assert_eq!(
        (
            first.ordinary_publication_visits,
            first.ordinary_publication_probes,
            first.ordinary_publication_seeks
        ),
        (100, 100, 2)
    );
    assert!(
        first.more,
        "foreign rows still require bounded maintenance continuation"
    );
    assert_eq!(
        first.version, 0,
        "no addressed publication in first quantum"
    );
    let second = refresh();
    assert_eq!(
        (
            second.ordinary_publication_visits,
            second.ordinary_publication_probes,
            second.ordinary_publication_seeks
        ),
        (2, 3, 2)
    );
    assert!(
        second.version > first.version,
        "addressed final row advances passive content"
    );
    let quiet = refresh();
    assert_eq!(
        (
            quiet.ordinary_publication_visits,
            quiet.ordinary_publication_probes,
            quiet.ordinary_publication_seeks
        ),
        (0, 0, 1)
    );
    assert_eq!(quiet.version, second.version);
    assert!(!quiet.more);
    assert_eq!(
        wake_attention(&f),
        native,
        "maintenance never adds native wake eligibility"
    );
    let mut bodies = Vec::new();
    for _ in 0..32 {
        match presentation_test_pull(&f, &recipient) {
            crate::protocol::delivery::DeliveryPullReplyV2::Page(page) => {
                bodies.push(page.text.clone());
                presentation_test_confirm(&f, &recipient, &page);
                if !page.more {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(bodies.join("\n").contains("prefix-body-101"));
    assert!(!bodies.join("\n").contains("prefix-body-100"));
}

#[test]
fn presentation_publication_invalid_scalars_refuse_before_confirmation_effects() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for (after, high, passive, scan) in [(2, 1, 0, 0), (0, 0, 1, 2)] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            claim("x", "px", fourth(), "invalid-scalar-inviter", 0),
            "invalid-scalar-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "invalid-scalar");
        let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
            panic!("page")
        };
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_publication_after=?1,ordinary_publication_high=?2,ordinary_passive_generation=?3,ordinary_scan_passive_generation=?4 WHERE seat_id='s' AND consumer='hook'",rusqlite::params![after,high,passive,scan]).unwrap();
        let snapshot = || {
            f.db.query_row("SELECT outbox_token,outbox_retired_mask,outbox_confirmed_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient,
                consumer: PresentationConsumer::Hook,
                token: page.presentation_token.unwrap(),
                emitted_controls: page.emitted_controls,
                presentation_lease: None,
            },
            &budget(),
        );
        assert!(matches!(
            result,
            Err(crate::protocol::results::ApiError {
                code: crate::protocol::results::ErrorCode::StoreCorrupt,
                ..
            })
        ));
        assert_eq!(snapshot(), before);
    }
}

#[test]
fn presentation_publication_refresh_and_pull_rebind_preserve_each_others_progress() {
    use crate::{
        ports::PresentationInput,
        protocol::{
            commands::{DeliveryMode, SendMessage},
            delivery::{DeliveryPullV2, PresentationConsumer},
        },
    };
    for (refresh_first, same_generation_repair) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let sender = check_in(
            &f,
            claim("x", "px", fourth(), "reset-publication-sender", 0),
            "reset-publication-ci",
        );
        for n in 0..201 {
            f.service
                .handle(
                    Command::SendMessage(SendMessage {
                        delivery_mode: DeliveryMode::Lazy,
                        user_intent: None,
                        thread: ThreadId::new("t"),
                        body: format!("reset body {n}"),
                        direct_recipient: None,
                        invited_recipients: vec![],
                        deadline_millis: None,
                        operation: OperationId::new(format!("reset-publication-{n}")),
                        claim: sender.clone(),
                        relays_user: false,
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap();
        }
        let pull = |claim: &CallerClaim| {
            f.store.presentation_pull(PresentationInput{ordinary_due_snapshot:None,frozen_render:presentation_watch_frozen(&f,claim),render_profile:crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,pull:DeliveryPullV2{claim:claim.clone(),consumer:PresentationConsumer::Watch,render:crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,presentation_lease:Some("0123456789abcdef0123456789abcdef".into()),marker_prefix:vec![]},attention_version:0,ordinary_version:0,with_attention:false,retry_messages:false,blocked_messages:vec![],known_messages:vec![],new_message_slots:256},&budget()).unwrap()
        };
        let refresh = |claim: &CallerClaim| {
            f.store
                .presentation_refresh(
                    &claim.seat,
                    claim.binding_generation,
                    claim.native_session.as_str(),
                    &budget(),
                )
                .unwrap()
                .unwrap()
        };
        let initial = refresh(&recipient);
        assert_eq!(
            (
                initial.ordinary_publication_visits,
                initial.ordinary_publication_probes,
                initial.ordinary_publication_seeks
            ),
            (100, 200, 2),
            "all addressed candidates reach both indexed probes"
        );
        let scalar = || {
            f.db.query_row("SELECT ordinary_publication_after,ordinary_publication_high,ordinary_passive_generation FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
        };
        assert_eq!(scalar().2, 1);
        assert!(scalar().0 > 0 && scalar().0 < scalar().1);
        let next = if same_generation_repair {
            // Explicit canonical repair fixture: preserve generation/session,
            // replace execution, and prove private ownership compares BOTH.
            let next = fresh_execution(recipient.clone(), 2);
            f.db.execute("UPDATE occupant_bindings SET execution_id=?1 WHERE seat_id='s' AND generation=?2 AND ended_at IS NULL",rusqlite::params![next.execution.as_str(),next.binding_generation as i64]).unwrap();
            assert_eq!(next.binding_generation, recipient.binding_generation);
            assert_ne!(next.execution, recipient.execution);
            next
        } else {
            let next = rotate(&f, &recipient, "publication-reset-native", 2);
            assert!(next.binding_generation > recipient.binding_generation);
            next
        };
        if refresh_first {
            refresh(&next);
        } else {
            pull(&next);
        }
        let first = scalar();
        assert!(first.0 > 0 && first.0 < first.1);
        assert_eq!(first.2, 1, "first new owner initializes exactly once");
        if refresh_first {
            pull(&next);
        } else {
            refresh(&next);
        }
        let second = scalar();
        assert!(
            second.0 > first.0,
            "second owner keeps positive cursor, continues instead of resetting"
        );
        assert_eq!(second.1, first.1);
        assert_eq!(second.2, 2);
        // Reopen the canonical store and continue the same finite frontier.
        let context = StoreContext::new(f.directory.join("store.db"), f.clock.clone());
        let _reopened = context.open_writer().unwrap();
        refresh(&next);
        let done = scalar();
        assert_eq!(done.0, done.1);
        assert_eq!(done.2, 3);
        let before = scalar();
        refresh(&next);
        assert_eq!(
            scalar(),
            before,
            "quiet refresh preserves completed progress"
        );
    }
}

#[test]
fn presentation_private_context_cursors_refuse_invalid_thread_ids_before_effects() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for field in [
        "context_after_thread",
        "context_thread",
        "mod_context_after_thread",
        "mod_context_thread",
    ] {
        for bad in ["x".repeat(129), "bad thread".into(), "λ".into()] {
            let f = fixture(0);
            let recipient = pull_seat(&f);
            let inviter = check_in(
                &f,
                claim("x", "px", fourth(), "cursor-bound-inviter", 0),
                "cursor-bound-ci",
            );
            canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "cursor-bound");
            let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
                panic!("page")
            };
            f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='hook'"),[bad]).unwrap();
            let snapshot = || {
                f.db.query_row("SELECT outbox_token,outbox_retired_mask,outbox_confirmed_mask,context_after_thread,context_thread,mod_context_after_thread,mod_context_thread FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?))).unwrap()
            };
            let before = snapshot();
            let result = f.store.presentation_confirm(
                &DeliveryPresentationConfirmV2 {
                    claim: recipient,
                    consumer: PresentationConsumer::Hook,
                    token: page.presentation_token.unwrap(),
                    emitted_controls: page.emitted_controls,
                    presentation_lease: None,
                },
                &budget(),
            );
            assert!(
                matches!(
                    result,
                    Err(crate::protocol::results::ApiError {
                        code: crate::protocol::results::ErrorCode::StoreCorrupt,
                        ..
                    })
                ),
                "{field} invalid cursor must refuse before effects: {result:?}"
            );
            assert_eq!(snapshot(), before);
        }
    }
}

#[test]
fn presentation_private_item_refresh_text_bounds_refuse_before_effects() {
    for (field, bad) in [
        ("mod_key", "x".repeat(4097)),
        ("mod_execution", "x".repeat(129)),
    ] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            claim("x", "px", fourth(), "item-bound-inviter", 0),
            "item-bound-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "item-bound");
        f.store
            .presentation_refresh(
                &recipient.seat,
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_items SET {field}=?1 WHERE seat_id='s' AND consumer='watch'"),[bad]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let snapshot = || {
            f.db.query_row("SELECT mod_version,mod_cleanup_after,mod_cleanup_high,mod_initial_after FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "private {field} must refuse before effects: {result:?}"
        );
        assert_eq!(snapshot(), before);
    }
}

#[test]
fn presentation_publication_signal_preserves_active_capture_and_replays_across_restart() {
    use crate::protocol::{
        commands::{DeliveryMode, SendMessage},
        delivery::DeliveryPullReplyV2,
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let sender = check_in(
        &f,
        claim("x", "px", fourth(), "capture-lazy-sender", 0),
        "capture-lazy-ci",
    );
    let send = |f: &Fixture, key: &str, body: String| {
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body,
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(key),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    };
    canonical_lazy_invitation_backlog(&f, &recipient, &sender, 1, "capture-outbox");
    send(&f, "capture-large", "z".repeat(65536));
    let DeliveryPullReplyV2::Page(first) = presentation_test_pull(&f, &recipient) else {
        panic!("large body page")
    };
    let scalar = |f: &Fixture| {
        f.db.query_row("SELECT ordinary_scan_passive_generation,ordinary_passive_generation,ordinary_version,ordinary_cursor,ordinary_pending,ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))).unwrap()
    };
    let frozen_text:String=f.db.query_row("SELECT outbox_text FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    assert!(
        first.presentation_token.is_some(),
        "canonical nonordinary invitation owns replay token"
    );
    let captured = scalar(&f);
    assert_eq!((captured.0, captured.1), (1, 1));
    assert!(
        captured.3.is_some(),
        "actual bounded large body capture remains active"
    );
    send(&f, "capture-following", "later-publication-body".into());
    let DeliveryPullReplyV2::Page(replay) = presentation_test_pull(&f, &recipient) else {
        panic!("immutable pending output replay")
    };
    assert_eq!(replay.presentation_token, first.presentation_token);
    assert_eq!(
        replay.text, frozen_text,
        "only frozen nonordinary projection is replayed; ordinary uses its existing display ledger"
    );
    let newer = scalar(&f);
    assert_eq!((newer.0, newer.1), (1, 2));
    assert_eq!(
        (&newer.2, &newer.3, &newer.4, &newer.5),
        (&captured.2, &captured.3, &captured.4, &captured.5),
        "new signal must not reset active START tuple/body/carry"
    );
    restart(&mut f);
    let DeliveryPullReplyV2::Page(restarted) = presentation_test_pull(&f, &recipient) else {
        panic!("restart replay")
    };
    assert_eq!(restarted.text, frozen_text);
    presentation_test_confirm(&f, &recipient, &restarted);
    let mut later = false;
    for _ in 0..32 {
        match presentation_test_pull(&f, &recipient) {
            DeliveryPullReplyV2::Page(page) => {
                later |= page.text.contains("later-publication-body");
                presentation_test_confirm(&f, &recipient, &page);
                if later && !page.more {
                    break;
                }
            }
            DeliveryPullReplyV2::Unchanged { .. } => break,
            other => panic!("unexpected active capture reply: {other:?}"),
        }
    }
    assert!(
        later,
        "after frozen capture finishes, changed passive tuple starts next capture without losing following publication"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2,
        "no synthetic lazy completion"
    );
}

fn finish_presentation_attention_job(f: &Fixture, job: &str) {
    for _ in 0..16 {
        let progress = f
            .store
            .advance_work(
                job,
                crate::ports::DurableWorkAdmission::new(1).unwrap(),
                &budget(),
            )
            .unwrap();
        assert!(progress.processed_this_turn <= 1);
        assert_eq!(progress.last_error, None);
        if !progress.has_more {
            return;
        }
    }
    panic!("fixture attention job did not finish within its finite recipient bound");
}

#[test]
fn presentation_registered_channel_materializes_each_published_warn_before_drain() {
    use crate::{
        ports::{
            ModChannelSink, ModChannels, ModStoreReads, ServiceAuthorityGate,
            ServiceConnectionAuthority, ServiceDecisionGuard, ServiceWriteTransactionProof,
        },
        protocol::{
            delivery::{
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer,
                PresentationControlStampV2, PresentationRenderRequest,
            },
            results::ApiError,
            service::{NotificationSeverity, ServiceNotify, ServiceOperation, ServiceResult},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Gate(ServiceAuthorId);
    struct Guard(ServiceAuthorId);
    impl ServiceDecisionGuard for Guard {
        fn author(&self) -> &ServiceAuthorId {
            &self.0
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            assert_eq!(connection.author(), &self.0);
            Ok(Box::new(Guard(self.0.clone())))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            true
        }
    }

    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        claim("c", "pc", Harness::Claude, "warn-channel-recipient", 0),
        "warn-channel-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    let lease = registry.presentation_lease(channel).unwrap();
    registry.pass(f.clock.utc_now()).unwrap();
    sink.0.lock().unwrap().clear();
    let native = || {
        f.db.query_row(
            "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='c'),0)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let tx = f.db.unchecked_transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    let gate = Gate(author.clone());
    let connection = ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, author);
    for n in 0..2 {
        sink.0.lock().unwrap().clear();
        let before = native();
        let text = format!("distinct channel warning {n}");
        let ServiceResult::Notification(notification) = f
            .store
            .service_operation(
                ServiceOperation::Notify(ServiceNotify {
                    thread: ThreadId::new("t"),
                    severity: NotificationSeverity::Warn,
                    event_json: serde_json::json!({"text":text}),
                    operation: OperationId::new(format!("warn-channel-notify-{n}")),
                }),
                &connection,
                &gate,
                &budget(),
                None,
            )
            .unwrap()
        else {
            panic!("Warn")
        };
        assert_eq!(
            native(),
            before,
            "publication precedes bounded native projection"
        );
        let job:String=f.db.query_row("SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT 'service-notify:'||preparation_id FROM service_notification_publications WHERE message_id=?1)",[notification.summary.message.as_str()],|r|r.get(0)).unwrap();
        finish_presentation_attention_job(&f, &job);
        assert!(
            native() > before,
            "each distinct Warn projects a native change even while generic relevance remains Open"
        );
        registry.mark_dirty();
        registry.pass(f.clock.utc_now()).unwrap();
        assert!(
            sink.0
                .lock()
                .unwrap()
                .iter()
                .any(|frame| matches!(frame, WatchFrame::Attention { .. })),
            "healthy bounded materialization causes channel drain"
        );
        let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = f
            .service
            .handle(
                Command::DeliveryPullV2(DeliveryPullV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some(lease.clone()),
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("drained page")
        };
        assert!(page.items.iter().any(|item|matches!(item,crate::protocol::delivery::PresentationItemV2::Notice{warning,..} if warning==&notification.summary.message)),"each published Warn delivers its distinct Notice");
        if n == 0 {
            assert!(
                page.emitted_controls
                    .iter()
                    .any(|v| matches!(v, PresentationControlStampV2::Attention { .. }))
            );
        }
        if let Some(token) = page.presentation_token {
            f.service
                .handle(
                    Command::DeliveryPresentationConfirmV2(
                        crate::protocol::delivery::DeliveryPresentationConfirmV2 {
                            claim: recipient.clone(),
                            consumer: PresentationConsumer::Watch,
                            token,
                            emitted_controls: page.emitted_controls,
                            presentation_lease: Some(lease.clone()),
                        },
                    ),
                    peer(),
                    &budget(),
                )
                .unwrap();
        }
    }
    registry.unregister(channel, f.clock.utc_now());
}

// These fixtures issue through the real V2 service and settle through the real
// canonical ACK writers. No offers or terminal receipt states are preseeded.
struct OfferWindowFixture {
    f: Fixture,
    recipient: CallerClaim,
    sender: CallerClaim,
    watch: Option<(
        Arc<crate::service::mod_channels::ModChannelRegistry>,
        crate::ports::ModChannelId,
        String,
    )>,
}

impl OfferWindowFixture {
    fn new(watch: bool) -> Self {
        use crate::{
            ports::{ModChannelSink, ModChannels, ModStoreReads},
            protocol::watch::{ModDeliverySetting, WatchFrame, WatchRequest},
            service::mod_channels::ModChannelRegistry,
        };
        struct Sink;
        impl ModChannelSink for Sink {
            fn push(&self, _: WatchFrame) -> bool {
                true
            }
        }
        let mut f = fixture(0);
        let recipient = if watch {
            check_in(
                &f,
                claim("c", "pc", Harness::Claude, "offer-window-recipient", 0),
                "offer-window-recipient-ci",
            )
        } else {
            pull_seat(&f)
        };
        let sender = check_in(
            &f,
            claim("x", "px", fourth(), "offer-window-sender", 0),
            "offer-window-sender-ci",
        );
        let watch = watch.then(|| {
            let reads: Arc<dyn ModStoreReads> = f.store.clone();
            let registry = Arc::new(ModChannelRegistry::new(
                f.clock.clone(),
                reads,
                "i".into(),
                Arc::new(|| {}),
                ModDeliverySetting::On,
            ));
            f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
                .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
                .with_delivery_pull(f.state.clone())
                .with_mod_channels(registry.clone());
            let (channel, _) = f
                .service
                .watch_register(
                    &WatchRequest {
                        claim: recipient.clone(),
                    },
                    Arc::new(Sink),
                    &budget(),
                )
                .unwrap();
            let lease = registry.presentation_lease(channel).unwrap();
            (registry, channel, lease)
        });
        Self {
            f,
            recipient,
            sender,
            watch,
        }
    }

    fn send(
        &self,
        n: usize,
        mode: crate::protocol::commands::DeliveryMode,
        body: &str,
    ) -> MessageId {
        use crate::protocol::commands::SendMessage;
        let CommandResult::MessageSent(id) = self
            .f
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: mode,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: body.into(),
                    // This fixture's existing thread has joined memberships;
                    // it is not an automatic direct-pair channel.
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("offer-window-send-{n}")),
                    claim: self.sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("real send result")
        };
        id
    }

    fn pull(&self) -> crate::protocol::delivery::DeliveryPullReplyV2 {
        use crate::protocol::delivery::{
            DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        };
        let Some((_, _, lease)) = &self.watch else {
            let reply = presentation_test_pull(&self.f, &self.recipient);
            self.confirm_nonmessage_presentation(&reply);
            return reply;
        };
        let CommandResult::DeliveryPulledV2(reply) = self
            .f
            .service
            .handle(
                Command::DeliveryPullV2(DeliveryPullV2 {
                    claim: self.recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some(lease.clone()),
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("real V2 pull result")
        };
        self.confirm_nonmessage_presentation(&reply);
        reply
    }

    fn confirm_nonmessage_presentation(
        &self,
        reply: &crate::protocol::delivery::DeliveryPullReplyV2,
    ) {
        use crate::protocol::delivery::{
            DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
        };
        if let DeliveryPullReplyV2::Page(page) = reply
            && let Some(token) = &page.presentation_token
        {
            self.f
                .service
                .handle(
                    Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                        claim: self.recipient.clone(),
                        consumer: if self.watch.is_some() {
                            PresentationConsumer::Watch
                        } else {
                            PresentationConsumer::Hook
                        },
                        token: token.clone(),
                        presentation_lease: self.watch.as_ref().map(|(_, _, lease)| lease.clone()),
                        emitted_controls: page.emitted_controls.clone(),
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap();
        }
    }

    fn offers(&self) -> std::collections::BTreeMap<MessageId, crate::delivery::ledger::Offer> {
        self.f
            .state
            .lock()
            .get(&self.recipient.seat)
            .unwrap()
            .offers
            .clone()
    }

    fn drain(&self, wanted: usize) -> std::collections::BTreeSet<MessageId> {
        use crate::protocol::delivery::DeliveryPullReplyV2;
        let mut shown = std::collections::BTreeSet::new();
        for _ in 0..256 {
            match self.pull() {
                DeliveryPullReplyV2::Page(page) => {
                    for id in page.offered.iter().chain(&page.truncated) {
                        assert!(shown.insert(id.clone()), "early duplicate offer {id:?}");
                    }
                }
                DeliveryPullReplyV2::Unchanged { .. } => {}
                other => panic!("admitted V2 drain refused: {other:?}"),
            }
            let work = self
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap();
            assert!(
                work.initial + work.context + work.revisit + work.ordinary + work.notices <= 433,
                "fresh discovery bound stays independent of ledger reconciliation: {work:?}"
            );
            if shown.len() == wanted {
                return shown;
            }
        }
        panic!(
            "bounded V2 drain showed {} of {wanted} messages",
            shown.len()
        );
    }

    fn ack(&self, messages: &[MessageId], mod_ack: bool, lazy: bool, operation: &str) {
        use crate::protocol::{
            commands::{Ack, AckModDelivered},
            watch::{ModAckOutcome, ModDeliveryVia},
        };
        for (n, batch) in messages.chunks(100).enumerate() {
            let operation = OperationId::new(format!("{operation}-{n}"));
            let command = if mod_ack {
                Command::AckModDelivered(AckModDelivered {
                    via: if lazy {
                        ModDeliveryVia::Append
                    } else {
                        ModDeliveryVia::Context
                    },
                    messages: batch.to_vec(),
                    operation,
                    claim: self.recipient.clone(),
                })
            } else {
                Command::Ack(Ack {
                    messages: batch.to_vec(),
                    operation,
                    claim: self.recipient.clone(),
                })
            };
            let result = self.f.service.handle(command, peer(), &budget()).unwrap();
            if mod_ack {
                let CommandResult::ModDeliveryAcked(report) = result else {
                    panic!("actual Watch ACK result")
                };
                assert_eq!(report.results.len(), batch.len());
                assert!(
                    report
                        .results
                        .iter()
                        .all(|v| v.result == ModAckOutcome::Settled),
                    "real canonical delivery settlement: {report:?}"
                );
            }
        }
    }
}

impl Drop for OfferWindowFixture {
    fn drop(&mut self) {
        use crate::ports::ModChannels;
        if let Some((registry, channel, _)) = &self.watch {
            registry.unregister(*channel, self.f.clock.utc_now());
        }
    }
}

fn assert_v2_offer_window_reclaimed(watch: bool, lazy: bool) {
    use crate::{
        delivery::ledger::MAX_OFFERS_PER_SEAT,
        protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2},
    };
    let window = OfferWindowFixture::new(watch);
    let mode = if lazy {
        DeliveryMode::Lazy
    } else {
        DeliveryMode::Ordinary
    };
    let sent = (0..MAX_OFFERS_PER_SEAT)
        .map(|n| window.send(n, mode, "complete canonical offer window"))
        .collect::<Vec<_>>();
    assert_eq!(window.drain(256), sent.iter().cloned().collect());
    assert_eq!(
        window.offers().len(),
        256,
        "real V2 issuance filled live capacity"
    );
    window.ack(&sent, watch, lazy, "offer-window-settle");
    for id in &sent {
        let state: String = window
            .f
            .db
            .query_row(
                if lazy {
                    "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2"
                } else {
                    "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2"
                },
                params![id.as_str(), window.recipient.seat.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, if lazy { "displayed" } else { "acked" });
    }
    let successor = window.send(256, mode, "new identity after cumulative capacity");
    let mut emitted = false;
    for n in 0..16 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            assert!(page.truncated.is_empty());
            assert!(page.offered.iter().all(|id| id == &successor));
            emitted |= page.offered.contains(&successor);
        }
        if n == 0 {
            let work = window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap();
            assert!(work.ordinary_due_key_probes <= 1303);
    assert!(work.ordinary_due_seeks <= 100);
    assert!(work.ordinary_due_hydrated <= 32);
    assert!(work.output_passes <= 352);
            assert_eq!(work.offer_reconciliation_visits, 256);
            assert_eq!(
                work.offer_reconciliation_probes,
                if lazy { 768 } else { 1024 }
            );
        }
        if emitted {
            break;
        }
    }
    assert!(
        emitted,
        "canonical terminal offers must free capacity in the same V2 binding"
    );
    assert_eq!(
        window.offers().keys().cloned().collect::<Vec<_>>(),
        vec![successor]
    );
    let ledger = window.f.state.lock();
    let entry = ledger.get(&window.recipient.seat).unwrap();
    assert_eq!(entry.generation, window.recipient.binding_generation);
    assert_eq!(
        entry.native_session,
        window.recipient.native_session.as_str()
    );
}

// Kills: treating MAX_OFFERS_PER_SEAT as cumulative lifetime throughput when
// Watch settles actual receipt-state rows through AckModDelivered.
#[test]
fn presentation_watch_settled_receipts_reclaim_live_offer_capacity() {
    assert_v2_offer_window_reclaimed(true, false);
}

// Kills: reconciling only the mod ACK route, leaving actual manual ACKs stuck
// in a capable Hook consumer's live-offer table.
#[test]
fn presentation_hook_manual_ack_reclaims_live_offer_capacity() {
    assert_v2_offer_window_reclaimed(false, false);
}

// Kills: checking only ordinary receipts and retaining displayed published
// lazy rows forever even after the canonical Watch completion writer runs.
#[test]
fn presentation_watch_completed_lazy_rows_reclaim_live_offer_capacity() {
    assert_v2_offer_window_reclaimed(true, true);
}

// Kills: cap eviction, reclaiming truncated/pending offers as presentation
// success, and resetting an unsettled id's count/timestamps during reclamation.
#[test]
fn presentation_reclamation_preserves_pending_truncated_and_reoffer_history() {
    use crate::protocol::{
        commands::{AckModDelivered, DeliveryMode},
        delivery::{
            DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        },
        watch::{ModAckOutcome, ModAckReason, ModDeliveryVia},
    };
    let window = OfferWindowFixture::new(true);
    let big = "x".repeat(WATCH_BODY_LIMIT_BYTES + 1);
    let mut sent = (0..256)
        .map(|n| {
            window.send(
                n,
                DeliveryMode::Ordinary,
                if n == 0 { &big } else { "pending" },
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(window.drain(256), sent[..256].iter().cloned().collect());
    let first = window.offers();
    assert!(first[&sent[0]].truncated);
    for _ in 0..4 {
        window.pull();
    }
    assert_eq!(
        window.offers(),
        first,
        "pending offers neither evict nor reoffer early"
    );
    let CommandResult::ModDeliveryAcked(report) = window
        .f
        .service
        .handle(
            Command::AckModDelivered(AckModDelivered {
                via: ModDeliveryVia::Context,
                messages: vec![sent[0].clone()],
                operation: OperationId::new("offer-window-truncated-refusal"),
                claim: window.recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("truncated ACK result")
    };
    assert_eq!(report.results[0].result, ModAckOutcome::RefusedTerminal);
    assert_eq!(report.results[0].reason, Some(ModAckReason::Truncated));
    window.pull();
    assert_eq!(
        window.offers(),
        first,
        "refused delivery ACK is not settlement"
    );
    advance(&window.f, REOFFER_AFTER_MS);
    assert_eq!(window.drain(256), sent[..256].iter().cloned().collect());
    let second = window.offers();
    assert!(second.values().all(|v| v.count == 2));
    // Add the withheld fresh identity after observing the due reoffers. A
    // zero-slot fresh carry otherwise stops ordinary discovery
    // before it can revisit the older IDs in this same finite walk.
    sent.push(window.send(256, DeliveryMode::Ordinary, "waiting for a reclaimed slot"));
    for _ in 0..4 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            assert!(!page.offered.contains(&sent[256]));
        }
    }
    assert_eq!(
        window.offers(),
        second,
        "the fresh identity cannot evict pending offers at capacity"
    );
    // Manual ACK is allowed after the body continuation was read, including
    // the previously truncated identity. Keep one short offer canonically pending.
    window.ack(&sent[..255], false, false, "offer-window-manual-settle");
    let mut emitted = false;
    for _ in 0..16 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            emitted |= page.offered.contains(&sent[256]);
            assert!(
                !page.offered.contains(&sent[255]),
                "surviving pending offer is not due"
            );
        }
        if emitted {
            break;
        }
    }
    assert!(
        emitted,
        "manual ACK of a truncated offer must reclaim its slot"
    );
    let retained = window.offers();
    assert_eq!(retained.len(), 2);
    assert_eq!(
        retained[&sent[255]], second[&sent[255]],
        "reconciliation preserves live reoffer history"
    );
    let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused {
        reason: Refusal::SessionMismatch,
    }) = window
        .f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: window.recipient.clone(),
                consumer: PresentationConsumer::Watch,
                render: PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some("ffffffffffffffffffffffffffffffff".into()),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("stale owner must refuse")
    };
    assert_eq!(
        window.offers(),
        retained,
        "refused owner cannot reconcile another owner's ledger"
    );
    advance(&window.f, REOFFER_AFTER_MS);
    assert_eq!(
        window.drain(2),
        [sent[255].clone(), sent[256].clone()].into_iter().collect()
    );
    assert_eq!(window.offers()[&sent[255]].count, MAX_OFFERS_PER_GENERATION);
    advance(&window.f, REOFFER_AFTER_MS);
    assert_eq!(window.drain(1), [sent[256].clone()].into_iter().collect());
    let parked = window.offers();
    assert!(
        parked
            .values()
            .all(|v| v.count == MAX_OFFERS_PER_GENERATION)
    );
    advance(&window.f, REOFFER_AFTER_MS);
    for _ in 0..4 {
        window.pull();
    }
    assert_eq!(
        window.offers(),
        parked,
        "pending offers remain parked at the original generation limit"
    );

    // A real canonical check-in rotates the binding. The old owner/claim must
    // refuse before it can drop the prior generation's retained offers.
    let next = check_in(
        &window.f,
        fresh_execution(
            claim(
                "c",
                "pc",
                Harness::Claude,
                "offer-window-next",
                window.recipient.binding_generation,
            ),
            2,
        ),
        "offer-window-next-ci",
    );
    assert!(next.binding_generation > window.recipient.binding_generation);
    assert!(matches!(window.pull(), DeliveryPullReplyV2::Refused { .. }));
    assert_eq!(
        window.offers(),
        parked,
        "stale generation cannot reconcile ledger state"
    );
}

fn full_pending_offer_window_with_fresh_carry() -> (OfferWindowFixture, Vec<MessageId>) {
    use crate::protocol::commands::DeliveryMode;
    let window = OfferWindowFixture::new(true);
    let big = "x".repeat(WATCH_BODY_LIMIT_BYTES + 1);
    let sent = (0..257)
        .map(|n| {
            window.send(
                n,
                DeliveryMode::Ordinary,
                if n == 0 { &big } else { "pending" },
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(window.drain(256), sent[..256].iter().cloned().collect());
    let offers = window.offers();
    assert_eq!(offers.len(), 256);
    assert!(offers.values().all(|offer| offer.count == 1));
    assert!(offers[&sent[0]].truncated);
    assert!(!offers.contains_key(&sent[256]));
    assert_eq!(window.f.db.query_row(
        "SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.seat_id='c' AND COALESCE(rs.state,'pending')='pending' AND pr.ack_required=1",
        [], |r| r.get::<_,i64>(0),
    ).unwrap(), 257, "real sends remain pending; presentation does not ACK");
    (window, sent)
}

fn capacity_normal_state(window: &OfferWindowFixture) -> String {
    window.f.db.query_row("SELECT json_array(ordinary_version,ordinary_done,ordinary_cursor,ordinary_pending,ordinary_partial,ordinary_skip_body,ordinary_scan_passive_generation) FROM invitation_presentation_consumers WHERE seat_id=?1 AND consumer=?2",
        rusqlite::params![window.recipient.seat.as_str(), if window.watch.is_some() { "watch" } else { "hook" }], |r| r.get(0)).unwrap()
}

fn assert_full_pending_offer_work(window: &OfferWindowFixture) {
    let work = window
        .f
        .service
        .presentation
        .last_work
        .lock()
        .unwrap()
        .unwrap();
    assert!(
        work.initial + work.context + work.revisit + work.ordinary + work.notices <= 433,
        "full pending carry must respect fresh discovery: {work:?}"
    );
    assert_eq!(work.offer_reconciliation_visits, 256);
    assert_eq!(work.offer_reconciliation_probes, 1024);
}

// Kills: an unoffered fresh carry at zero slots driving endless actionable
// page.more continuations despite every known offer being blocked until due.
#[test]
fn presentation_full_pending_carry_reaches_quiet_without_evicting_offers() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let (window, sent) = full_pending_offer_window_with_fresh_carry();
    let original = window.offers();
    let mut quiet = false;
    for _ in 0..16 {
        match window.pull() {
            DeliveryPullReplyV2::Unchanged { .. } => quiet = true,
            DeliveryPullReplyV2::Page(page) => {
                assert!(
                    page.offered.is_empty() && page.truncated.is_empty(),
                    "no offer is due and the fresh identity must wait at capacity"
                );
                quiet = !page.more;
            }
            other => panic!("live full-capacity owner refused: {other:?}"),
        }
        assert_full_pending_offer_work(&window);
        assert_eq!(
            window.offers(),
            original,
            "quieting cannot evict or reset pending offers"
        );
        if quiet {
            break;
        }
    }
    assert!(
        quiet,
        "full pending carry must stop actionable continuation within bounded pulls"
    );
    assert!(!window.offers().contains_key(&sent[256]));
}

// Kills: quieting the full carry by permanently parking it, which also starves
// already known pending offers once their original 60-second reoffer is due.
#[test]
fn presentation_full_pending_carry_serves_due_known_offers_without_admitting_fresh_id() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let (window, sent) = full_pending_offer_window_with_fresh_carry();
    let original = window.offers();
    // Let ordinary discovery encounter the retained fresh ID before the clock
    // advances, without requiring the separate quiet-continuation behavior.
    for _ in 0..4 {
        window.pull();
        assert_full_pending_offer_work(&window);
        assert_eq!(window.offers(), original);
    }
    let parked = capacity_normal_state(&window);
    advance(&window.f, REOFFER_AFTER_MS);
    let due_at = window.f.clock.utc_now();
    let mut shown = std::collections::BTreeSet::new();
    for _ in 0..16 {
        match window.pull() {
            DeliveryPullReplyV2::Page(page) => {
                for id in page.offered.iter().chain(&page.truncated) {
                    assert!(
                        original.contains_key(id),
                        "only due known IDs fit at capacity"
                    );
                    assert_ne!(id, &sent[256]);
                    assert!(
                        shown.insert(id.clone()),
                        "same-time duplicate reoffer {id:?}"
                    );
                }
            }
            DeliveryPullReplyV2::Unchanged { .. } => {}
            other => panic!("live due-offer owner refused: {other:?}"),
        }
        assert_full_pending_offer_work(&window);
        assert_eq!(capacity_normal_state(&window), parked);
        let retained = window.offers();
        assert_eq!(retained.len(), 256);
        for (id, before) in &original {
            let after = retained[id];
            assert_eq!(after.first_at, before.first_at);
            assert_eq!(after.truncated, before.truncated);
            assert_eq!(after.count, if shown.contains(id) { 2 } else { 1 });
            assert_eq!(
                after.last_at,
                if shown.contains(id) {
                    due_at
                } else {
                    before.last_at
                }
            );
        }
        if shown.len() == 256 {
            break;
        }
    }
    assert!(
        !shown.is_empty(),
        "due known offers must make progress past a retained fresh ID"
    );
    assert_eq!(
        shown,
        sent[..256].iter().cloned().collect(),
        "the finite due set must progress within bounded pulls"
    );
    assert!(!window.offers().contains_key(&sent[256]));
    let twice = window.offers();
    for _ in 0..4 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            assert!(
                page.offered.is_empty() && page.truncated.is_empty(),
                "due progress cannot reset the original reoffer spacing"
            );
        }
        assert_full_pending_offer_work(&window);
        assert_eq!(window.offers(), twice);
    }
    advance(&window.f, REOFFER_AFTER_MS);
    let mut third = std::collections::BTreeSet::new();
    for _ in 0..16 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            third.extend(page.offered.into_iter().chain(page.truncated));
        }
        assert_eq!(capacity_normal_state(&window), parked);
        if third.len() == 256 { break; }
    }
    assert_eq!(third, sent[..256].iter().cloned().collect());
    assert!(window.offers().values().all(|stamp| stamp.count == 3));
    advance(&window.f, REOFFER_AFTER_MS);
    assert!(matches!(window.pull(), DeliveryPullReplyV2::Unchanged { .. }));
    window.ack(std::slice::from_ref(&sent[1]), false, false, "capacity-resume");
    let DeliveryPullReplyV2::Page(resumed) = window.pull() else { panic!("same-call reclaimed slot"); };
    assert!(resumed.offered.contains(&sent[256]));

}

fn offer_reconciliation_input(
    f: &Fixture,
    claim: &CallerClaim,
    known: Vec<MessageId>,
) -> crate::ports::PresentationInput {
    use crate::protocol::delivery::{
        DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
    };
    crate::ports::PresentationInput {
        ordinary_due_snapshot: None,
        frozen_render: presentation_watch_frozen(f, claim),
        render_profile: crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
        pull: DeliveryPullV2 {
            claim: claim.clone(),
            consumer: PresentationConsumer::Watch,
            render: PresentationRenderRequest::WatchJsonLinesV1,
            presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
            marker_prefix: vec![],
        },
        attention_version: 0,
        ordinary_version: 0,
        with_attention: false,
        retry_messages: false,
        blocked_messages: known.clone(),
        new_message_slots: 256usize.saturating_sub(known.len()),
        known_messages: known,
    }
}

// Kills: a full pending walk instead of exact bounded snapshot keys, skipping
// reconciliation on Unchanged, and omitting the legacy authority fallback.
#[test]
fn presentation_reclamation_legacy_terminal_delta_has_exact_bounded_work_on_unchanged() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(257);
    let recipient = pull_seat(&f);
    f.db.execute("UPDATE receipts SET state='acked' WHERE seat_id='s'", [])
        .unwrap();
    // A new consumer must finish its finite ordinary scan before it can be
    // quiet. Prime that real walk without any ledger hints or reconciliation.
    let mut quiet = false;
    for _ in 0..16 {
        let primed = f
            .store
            .presentation_pull(
                offer_reconciliation_input(&f, &recipient, vec![]),
                &budget(),
            )
            .unwrap();
        assert_eq!(primed.work.offer_reconciliation_visits, 0);
        assert_eq!(primed.work.offer_reconciliation_probes, 0);
        assert!(primed.reclaimed_messages.is_empty());
        assert!(
            primed.work.initial
                + primed.work.context
                + primed.work.revisit
                + primed.work.ordinary
                + primed.work.notices
                <= 433
        );
        match primed.reply {
            DeliveryPullReplyV2::Unchanged { .. } => {
                quiet = true;
                break;
            }
            DeliveryPullReplyV2::Page(_) => {}
            other => panic!("finite consumer prime refused: {other:?}"),
        }
    }
    assert!(
        quiet,
        "finite empty consumer walk must reach quiet within bounded pulls"
    );
    let known = (1..=256)
        .map(|n| MessageId::new(format!("m{n}")))
        .collect::<Vec<_>>();
    let output = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, known.clone()),
            &budget(),
        )
        .unwrap();
    assert!(matches!(
        output.reply,
        DeliveryPullReplyV2::Unchanged { .. }
    ));
    assert_eq!(
        output
            .reclaimed_messages
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        known.into_iter().collect()
    );
    assert_eq!(output.work.offer_reconciliation_visits, 256);
    assert_eq!(output.work.offer_reconciliation_probes, 1280);
    assert_eq!(output.work.ordinary_publication_visits, 0);
}

// Kills: deleting pending/missing snapshot entries, probing duplicate hints
// repeatedly, and reading receipt authority for an absent message.
#[test]
fn presentation_reclamation_keeps_pending_missing_and_deduplicates_snapshot_work() {
    let f = fixture(2);
    let recipient = pull_seat(&f);
    f.db.execute(
        "UPDATE receipts SET state='recipient_retired' WHERE message_id='m2' AND seat_id='s'",
        [],
    )
    .unwrap();
    let output = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, ids(&["m1", "m2", "missing", "m2"])),
            &budget(),
        )
        .unwrap();
    assert_eq!(output.reclaimed_messages, ids(&["m2"]));
    assert_eq!(output.work.offer_reconciliation_visits, 3);
    assert_eq!(output.work.offer_reconciliation_probes, 11);
    assert_eq!(receipt_state(&f, "m1"), "pending");
}

// Kills: a contradictory legacy row overriding manifest authority, a missing
// sparse row being treated as terminal, or explicit materialized waivers being
// needlessly retained until the offer cap becomes a lifetime throughput cap.
#[test]
fn presentation_reclamation_preserves_logical_precedence_and_explicit_waiver_flags() {
    use crate::protocol::commands::DeliveryMode;
    for (state, waiver, legacy_required, want) in [
        (None, "", 1, false),
        (Some("pending"), "", 1, false),
        (Some("acked"), "", 1, true),
        (Some("recipient_retired"), "", 1, true),
        (None, "prepared", 1, true),
        (Some("pending"), "sparse", 1, true),
        (None, "", 0, false),
    ] {
        let window = OfferWindowFixture::new(false);
        let message = window.send(0, DeliveryMode::Ordinary, "logical authority");
        let f = &window.f;
        f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,ack_required) VALUES (?1,'t','s','acked',300000,?2)",params![message.as_str(),legacy_required]).unwrap();
        if let Some(state) = state {
            f.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_required) VALUES (?1,'s',?2,?3)",params![message.as_str(),state,if waiver=="sparse" {0} else {1}]).unwrap();
        }
        if waiver == "prepared" {
            f.db.execute("UPDATE prepared_recipients SET ack_required=0 WHERE seat_id='s' AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)",[message.as_str()]).unwrap();
        }
        let output = f
            .store
            .presentation_pull(
                offer_reconciliation_input(f, &window.recipient, vec![message.clone()]),
                &budget(),
            )
            .unwrap();
        assert_eq!(
            output.reclaimed_messages,
            if want { vec![message] } else { vec![] },
            "state={state:?} waiver={waiver} legacy_required={legacy_required}"
        );
        assert_eq!(output.work.offer_reconciliation_visits, 1);
        assert_eq!(output.work.offer_reconciliation_probes, 4);
    }
    // Legacy waiver flags are themselves canonical materialized terminal
    // evidence when no manifest addressed authority exists.
    let f = fixture(1);
    let recipient = pull_seat(&f);
    f.db.execute(
        "UPDATE receipts SET ack_required=0 WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    let output = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, ids(&["m1"])),
            &budget(),
        )
        .unwrap();
    assert_eq!(output.reclaimed_messages, ids(&["m1"]));
    assert_eq!(output.work.offer_reconciliation_probes, 5);
}

// Kills: confusing staged or uncompleted lazy rows with canonically published
// displayed progress. Migration26 introduced only manifest-published lazy mail.
#[test]
fn presentation_reclamation_requires_published_addressed_lazy_completion() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    for id in [
        "msg-old-pending",
        "msg-old-displayed",
        "msg-unpublished-displayed",
    ] {
        lazy_message(&f, id, "retained lazy body");
    }
    f.db.execute("UPDATE lazy_recipients SET state='displayed' WHERE seat_id='s' AND message_id!='msg-old-pending'", []).unwrap();
    f.db.execute(
        "DELETE FROM send_manifests WHERE message_id='msg-unpublished-displayed'",
        [],
    )
    .unwrap();
    let output = f
        .store
        .presentation_pull(
            offer_reconciliation_input(
                &f,
                &recipient,
                ids(&[
                    "msg-old-pending",
                    "msg-old-displayed",
                    "msg-unpublished-displayed",
                ]),
            ),
            &budget(),
        )
        .unwrap();
    assert_eq!(output.reclaimed_messages, ids(&["msg-old-displayed"]));
    assert_eq!(output.work.offer_reconciliation_visits, 3);
    assert_eq!(output.work.offer_reconciliation_probes, 9);
    assert_eq!(lazy_state(&f, "msg-old-pending"), "pending");
}

// Kills: suppressing the terminal delta on recovery replay, or running its
// keyed reads before exact canonical scope/frozen-render admission.
#[test]
fn presentation_reclamation_replay_returns_delta_and_refusals_do_no_reconciliation() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(1);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "reclamation-inviter", 0),
        "reclamation-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "reclamation-replay");
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    let first = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, vec![]),
            &budget(),
        )
        .unwrap();
    assert!(
        matches!(first.reply, DeliveryPullReplyV2::Page(ref page) if page.presentation_token.is_some())
    );
    let input = offer_reconciliation_input(&f, &recipient, ids(&["m1"]));
    assert!(input.frozen_render.is_some());
    let replay = f.store.presentation_pull(input.clone(), &budget()).unwrap();
    assert!(replay.work.replay > 0);
    assert_eq!(replay.reclaimed_messages, ids(&["m1"]));
    assert_eq!(replay.work.offer_reconciliation_visits, 1);
    assert_eq!(replay.work.offer_reconciliation_probes, 5);
    for scope_refusal in [true, false] {
        let mut refused = input.clone();
        if scope_refusal {
            refused.pull.claim = fresh_execution(recipient.clone(), 3);
        } else {
            refused.frozen_render = None;
        }
        let output = f.store.presentation_pull(refused, &budget()).unwrap();
        assert!(matches!(output.reply, DeliveryPullReplyV2::Refused { .. }));
        assert!(output.reclaimed_messages.is_empty());
        assert_eq!(output.work.offer_reconciliation_visits, 0);
        assert_eq!(output.work.offer_reconciliation_probes, 0);
    }
}

// Kills: silent truncation/cap raising on unbounded internal snapshots. Refuse
// before private presentation progress is changed or terminal delta is applied.
#[test]
fn presentation_reclamation_refuses_oversize_hints_before_effects() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let count = || {
        f.db.query_row(
            "SELECT count(*) FROM invitation_presentation_consumers",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = count();
    for field in ["known", "blocked", "slots"] {
        let mut input = offer_reconciliation_input(&f, &recipient, vec![]);
        match field {
            "known" => {
                input.known_messages = (0..257)
                    .map(|n| MessageId::new(format!("unknown-{n}")))
                    .collect()
            }
            "blocked" => {
                input.blocked_messages = (0..257)
                    .map(|n| MessageId::new(format!("unknown-{n}")))
                    .collect()
            }
            _ => input.new_message_slots = 257,
        }
        let error = f.store.presentation_pull(input, &budget()).unwrap_err();
        assert_eq!(
            error.code,
            crate::protocol::results::ErrorCode::InvalidRequest
        );
        assert_eq!(
            count(),
            before,
            "{field} overflow must not mutate presentation state"
        );
    }
}

#[test]
fn presentation_publication_foreign_refresh_refuses_without_touching_hook_or_watch() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "foreign-refresh-inviter", 0),
        "foreign-refresh-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "foreign-refresh");
    let _ = presentation_test_pull(&f, &recipient);
    let hook = || {
        f.db.query_row("SELECT outbox_token,outbox_text,ordinary_publication_after,ordinary_passive_generation FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap()
    };
    let before_hook = hook();
    f.store
        .presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        hook(),
        before_hook,
        "Watch maintenance is independent of Hook presentation"
    );
    f.db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('foreign-instance',0)",
        [],
    )
    .unwrap();
    f.db.execute("UPDATE invitation_presentation_consumers SET instance_id='foreign-instance' WHERE seat_id='s' AND consumer='watch'",[]).unwrap();
    let watch = || {
        f.db.query_row("SELECT instance_id,mod_generation,mod_execution,ordinary_publication_after,ordinary_passive_generation FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap()
    };
    let before = watch();
    let result = f.store.presentation_refresh(
        &recipient.seat,
        recipient.binding_generation,
        recipient.native_session.as_str(),
        &budget(),
    );
    assert!(matches!(
        result,
        Err(crate::protocol::results::ApiError {
            code: crate::protocol::results::ErrorCode::StoreCorrupt,
            ..
        })
    ));
    assert_eq!(watch(), before);
    assert_eq!(hook(), before_hook);
}

#[test]
fn presentation_handler_initial_ordinary_receipt_delivers_body_without_invented_attention() {
    use crate::protocol::{
        commands::{DeliveryMode, SendMessage},
        delivery::{DeliveryPullReplyV2, PresentationControlStampV2},
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let sender = check_in(
        &f,
        claim("x", "px", fourth(), "initial-ordinary-sender", 0),
        "initial-ordinary-ci",
    );
    f.service
        .handle(
            Command::SendMessage(SendMessage {
                delivery_mode: DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "ordinary bounded response needed".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("initial-ordinary-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("actual ordinary body")
    };
    assert!(page.text.contains("ordinary bounded response needed")); // M Attention relevance is confirming invitations plus warnings;
    // ordinary ACK obligations retain their own nudge-class message output.
    assert!(
        !page
            .emitted_controls
            .iter()
            .any(|v| matches!(v, PresentationControlStampV2::Attention { .. }))
    );
    assert!(!page.offered.is_empty());
    assert_eq!(f.db.query_row("SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.seat_id='s' AND COALESCE(rs.state,'pending')='pending' AND pr.ack_required=1",[],|r|r.get::<_,i64>(0)).unwrap(),1,"body presentation does not ACK ordinary obligation");
}

#[test]
fn presentation_publication_watch_reconnect_keeps_positive_frontier_without_native_wake() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            commands::{DeliveryMode, SendMessage},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        claim(
            "c",
            "pc",
            Harness::Claude,
            "publication-reconnect-recipient",
            0,
        ),
        "publication-reconnect-ci",
    );
    let sender = check_in(
        &f,
        claim("x", "px", fourth(), "publication-reconnect-sender", 0),
        "publication-reconnect-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let request = WatchRequest {
        claim: recipient.clone(),
    };
    let (first, _) = f
        .service
        .watch_register(&request, Arc::new(Sink), &budget())
        .unwrap();
    let old = registry.presentation_lease(first).unwrap();
    for n in 0..201 {
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: format!("reconnect body {n}"),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("publication-reconnect-{n}")),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let native = wake_attention(&f);
    registry.pass(f.clock.utc_now()).unwrap();
    let state = || {
        f.db.query_row("SELECT ordinary_publication_after,ordinary_publication_high,ordinary_passive_generation FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
    };
    let before = state();
    assert!(before.0 > 0 && before.0 < before.1);
    assert_eq!(before.2, 1);
    registry.unregister(first, f.clock.utc_now());
    assert!(!registry.presentation_owner_live(
        &recipient.seat,
        recipient.binding_generation,
        recipient.native_session.as_str(),
        &old
    ));
    assert_eq!(
        state(),
        before,
        "grace transition does not rewrite canonical cursor"
    );
    let (second, _) = f
        .service
        .watch_register(&request, Arc::new(Sink), &budget())
        .unwrap();
    let new = registry.presentation_lease(second).unwrap();
    assert_ne!(old, new);
    registry.pass(f.clock.utc_now()).unwrap();
    let continued = state();
    assert!(continued.0 > before.0);
    assert_eq!(continued.1, before.1);
    assert_eq!(continued.2, 2);
    assert_eq!(
        wake_attention(&f),
        native,
        "passive reconnect maintenance never creates native eligibility"
    );
    registry.unregister(second, f.clock.utc_now());
}

#[test]
fn presentation_publication_generation_overflow_rolls_back_frontier_and_version() {
    use crate::protocol::commands::{DeliveryMode, SendMessage};
    for field in ["ordinary_passive_generation", "mod_version"] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let sender = check_in(
            &f,
            claim("x", "px", fourth(), "publication-overflow-sender", 0),
            "publication-overflow-ci",
        );
        f.store
            .presentation_refresh(
                &recipient.seat,
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='watch'"),[i64::MAX]).unwrap();
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: "overflow addressed body".into(),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new("publication-overflow-send"),
                    claim: sender,
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let snapshot = || {
            f.db.query_row("SELECT ordinary_publication_after,ordinary_publication_high,ordinary_passive_generation,ordinary_scan_passive_generation,mod_version FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::SequenceExhausted,
                    ..
                })
            ),
            "{field} overflow must refuse: {result:?}"
        );
        assert_eq!(
            snapshot(),
            before,
            "no consumed publication or partial version write on overflow"
        );
    }
}

#[test]
fn presentation_quiet_channel_observes_new_ordinary_ack_message() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            commands::{DeliveryMode, SendMessage},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        claim("c", "pc", Harness::Claude, "quiet-ordinary-recipient", 0),
        "quiet-ordinary-ci",
    );
    let sender = check_in(
        &f,
        claim("x", "px", fourth(), "quiet-ordinary-sender", 0),
        "quiet-ordinary-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(f.clock.utc_now()).unwrap();
    sink.0.lock().unwrap().clear();
    let native = || {
        f.db.query_row(
            "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='c'),0)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = native();
    let CommandResult::MessageSent(message) = f
        .service
        .handle(
            Command::SendMessage(SendMessage {
                delivery_mode: DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "new ordinary ACK content after quiet registration".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("quiet-ordinary-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("ordinary message")
    };
    assert_eq!(native(), before, "native materializer has not yet run");

    let job:String=f.db.query_row("SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)",[message.as_str()],|r|r.get(0)).unwrap();
    finish_presentation_attention_job(&f, &job);
    assert!(native() > before);
    registry.mark_dirty();
    registry.pass(f.clock.utc_now()).unwrap();
    let frames = sink.0.lock().unwrap().clone();
    assert!(
        matches!(frames.as_slice(), [WatchFrame::Attention { .. }]),
        "new ordinary ACK message needs an admitted channel drain after quiet registration: {frames:?}"
    );
    registry.pass(f.clock.utc_now()).unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        frames,
        "unchanged ordinary publication does not repeat drains"
    );

    let CommandResult::DeliveryPulledV2(crate::protocol::delivery::DeliveryPullReplyV2::Page(page)) =
        f.service
            .handle(
                Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                    claim: recipient.clone(),
                    consumer: crate::protocol::delivery::PresentationConsumer::Watch,
                    render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: registry.presentation_lease(channel),
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    else {
        panic!("ordinary admitted drain")
    };
    assert!(page.items.iter().any(|v|matches!(v,crate::protocol::delivery::PresentationItemV2::Message{id,lazy:false,content} if id==&message && content.body=="new ordinary ACK content after quiet registration" && content.ack_required)),"ordered Watch item delivers exact ordinary body and ACK requirement");
    assert!(
        page.emitted_controls.is_empty(),
        "ordinary receipt alone does not invent generic Attention"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id='c'",
            [message.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending",
        "display does not ACK"
    );
    f.clock.0.store(
        1_000_000 + crate::protocol::watch::MOD_STALL_AFTER_MS as i64 + 1,
        Ordering::Relaxed,
    );
    assert!(
        registry.stalled(&recipient.seat, f.clock.utc_now()),
        "successful native observer push preserves ordinary-only legacy publication cutoff"
    );
    registry.unregister(channel, f.clock.utc_now());
}

// These fixtures catch unbounded/private ID reads and unchecked constructors in
// each independently reachable cleanup, dirty-sizing and revisit branch.
fn private_invitation_id_fixture(branch: &str) -> (Fixture, CallerClaim, i64) {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "private-id-inviter", 0),
        "private-id-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "private-id");
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("admitted Hook page")
    };
    assert_eq!(presentation_invitations(&page).len(), 1);
    presentation_test_confirm(&f, &recipient, &page);
    let ordinal = f.db.query_row(
        "SELECT revisit_ordinal FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap();
    let cleanup_after = if branch == "cleanup" { 0 } else { ordinal };
    let dirty_after = if branch == "dirty" { 0 } else { ordinal };
    let required_bytes = if branch == "revisit" { 1 } else { i64::MAX };
    f.db.execute(
        "UPDATE invitation_presentation_items SET required_render_bytes=?1 WHERE seat_id='s' AND consumer='hook'",
        [required_bytes],
    ).unwrap();
    f.db.execute(
        "UPDATE invitation_presentation_consumers SET initial_after=?1,initial_high=?1,context_after_seq=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),context_high=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),context_after_thread=?2,context_thread='',context_selected_seq=0,context_invitation_after=0,context_invitation_high=0,cleanup_after=?3,dirty_after=?4,dirty_high=?1,revisit_after_bytes=0,revisit_after_ordinal=0,revisit_high_bytes=?5,revisit_high_ordinal=?6 WHERE seat_id='s' AND consumer='hook'",
        params![ordinal, "\u{10ffff}", cleanup_after, dirty_after, i64::from(branch == "revisit"), if branch == "revisit" { ordinal } else { 0 }],
    ).unwrap();
    assert!(f.db.query_row(
        "SELECT outbox_token IS NULL FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",
        [], |r| r.get::<_, bool>(0),
    ).unwrap(), "confirmed allocation cannot bypass private reads through replay");
    let selected = f.db.query_row(
        "SELECT (SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer=s.consumer AND i.revisit_ordinal>s.cleanup_after),(SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer=s.consumer AND i.revisit_ordinal>s.dirty_after AND i.revisit_ordinal<=s.dirty_high),(SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer=s.consumer AND i.generation=s.generation AND (i.required_render_bytes,i.revisit_ordinal)>(s.revisit_after_bytes,s.revisit_after_ordinal) AND (i.required_render_bytes,i.revisit_ordinal)<=(s.revisit_high_bytes,s.revisit_high_ordinal) AND i.required_render_bytes<=65536) FROM invitation_presentation_consumers s WHERE s.seat_id='s' AND s.consumer='hook'",
        [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    ).unwrap();
    assert_eq!(
        selected,
        match branch {
            "cleanup" => (1, 0, 0),
            "dirty" => (0, 1, 0),
            "revisit" => (0, 0, 1),
            _ => panic!("unknown branch"),
        },
        "one private reader is independently reachable"
    );
    (f, recipient, ordinal)
}

// Preserve raw SQLite type and bytes, including malformed UTF-8. Snapshot every
// table so private frontiers, canonical invitations, receipts and warnings all
// participate in the no-effect assertion; row sorting also supports WITHOUT ROWID.
fn private_invitation_id_snapshot(db: &Connection) -> Vec<(String, Vec<Vec<u8>>)> {
    use rusqlite::types::ValueRef;
    let mut tables = db.prepare(
        "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ).unwrap();
    let names = tables
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut query = db
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let columns = query.column_count();
            let mut rows = query
                .query_map([], |row| {
                    let mut bytes = Vec::new();
                    for column in 0..columns {
                        let (tag, raw) = match row.get_ref(column)? {
                            ValueRef::Null => (0, Vec::new()),
                            ValueRef::Integer(v) => (1, v.to_le_bytes().to_vec()),
                            ValueRef::Real(v) => (2, v.to_bits().to_le_bytes().to_vec()),
                            ValueRef::Text(v) => (3, v.to_vec()),
                            ValueRef::Blob(v) => (4, v.to_vec()),
                        };
                        bytes.push(tag);
                        bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
                        bytes.extend_from_slice(&raw);
                    }
                    Ok(bytes)
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}

fn repair_private_invitation_id(
    f: &Fixture,
    value: rusqlite::types::Value,
    cast_text: bool,
) -> rusqlite::Result<usize> {
    repair_private_invitation_id_for_consumer(f, "hook", value, cast_text)
}

fn repair_private_invitation_id_for_consumer(
    f: &Fixture,
    consumer: &str,
    value: rusqlite::types::Value,
    cast_text: bool,
) -> rusqlite::Result<usize> {
    // Deliberately corrupt only the private ID through a short-lived repair
    // connection. Never relax the service/writer connection's foreign keys.
    let repair = Connection::open(f.directory.join("store.db")).unwrap();
    repair.pragma_update(None, "foreign_keys", true).unwrap();
    let before = repair
        .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
        .unwrap();
    repair.pragma_update(None, "foreign_keys", false).unwrap();
    assert!(
        !repair
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    let sql = if cast_text {
        "UPDATE invitation_presentation_items SET invitation_id=CAST(?1 AS TEXT) WHERE seat_id='s' AND consumer=?2"
    } else {
        "UPDATE invitation_presentation_items SET invitation_id=?1 WHERE seat_id='s' AND consumer=?2"
    };
    let result = repair.execute(sql, params![value, consumer]);
    repair.pragma_update(None, "foreign_keys", before).unwrap();
    assert!(
        repair
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    assert!(
        f.db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    result
}

fn pull_private_invitation_id(
    f: &Fixture,
    recipient: &CallerClaim,
) -> Result<crate::ports::PresentationOutput, crate::protocol::results::ApiError> {
    use crate::{
        delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
        ports::PresentationInput,
        protocol::delivery::{DeliveryPullV2, PresentationConsumer, PresentationRenderRequest},
    };
    let registry = crate::harness::registry::builtins();
    let adapter = registry
        .by_id(registry.agent(recipient.harness.as_str()).unwrap())
        .unwrap();
    let codec = adapter.presentation_codec(0).unwrap();
    let frozen_render = f
        .store
        .presentation_render_selection(recipient, PresentationConsumer::Hook, &budget())
        .unwrap()
        .map(|identity| FrozenPresentationRender {
            identity,
            codec: Some(codec),
        });
    f.store.presentation_pull(
        PresentationInput {
        ordinary_due_snapshot: None,
            frozen_render,
            render_profile: PresentationRenderProfile::Hook {
                confirmable_selector: 0,
                passive_selector: 0,
                confirmable: codec,
                passive: codec,
            },
            pull: DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                render: PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: vec![],
            },
            attention_version: 0,
            ordinary_version: 0,
            with_attention: false,
            retry_messages: false,
            blocked_messages: vec![],
            known_messages: vec![],
            new_message_slots: 256,
        },
        &budget(),
    )
}

#[test]
fn presentation_private_invitation_ids_refuse_corruption_in_each_reader_without_effects() {
    use crate::protocol::results::{ApiError, ErrorCode};
    use rusqlite::types::Value;
    let mut failures = Vec::new();
    for branch in ["cleanup", "dirty", "revisit"] {
        for (kind, value) in [
            ("empty", Value::Text(String::new())),
            ("overlong", Value::Text("x".repeat(129))),
            ("space", Value::Text("bad id".into())),
            ("tab", Value::Text("bad\tid".into())),
            ("control", Value::Text("bad\0id".into())),
            ("non_ascii", Value::Text("λ".into())),
            ("invalid_utf8", Value::Blob(vec![0x80])),
        ] {
            let (f, recipient, _) = private_invitation_id_fixture(branch);
            assert_eq!(repair_private_invitation_id(&f, value, true).unwrap(), 1);
            let before = private_invitation_id_snapshot(&f.db);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pull_private_invitation_id(&f, &recipient)
            }));
            if !matches!(
                result,
                Ok(Err(ApiError {
                    code: ErrorCode::StoreCorrupt,
                    ..
                }))
            ) {
                failures.push(format!(
                    "{branch}/{kind}: expected StoreCorrupt without panic, got {result:?}"
                ));
            }
            if private_invitation_id_snapshot(&f.db) != before {
                failures.push(format!(
                    "{branch}/{kind}: private or canonical rows changed after refusal"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn presentation_private_invitation_ids_allow_exact_128_byte_ids_in_each_reader() {
    for branch in ["cleanup", "dirty", "revisit"] {
        let (f, recipient, _) = private_invitation_id_fixture(branch);
        assert_eq!(
            repair_private_invitation_id(&f, rusqlite::types::Value::Text("x".repeat(128)), true)
                .unwrap(),
            1
        );
        let output = pull_private_invitation_id(&f, &recipient).unwrap();
        assert_eq!(output.work.initial, 0);
        assert_eq!(output.work.context, 0);
        assert_eq!(
            output.work.revisit, 1,
            "{branch}: valid boundary ID reaches its private reader"
        );
        assert_eq!(
            f.db.query_row(
                "SELECT COUNT(*) FROM invitations WHERE seat_id='s' AND state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            f.db.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn presentation_private_invitation_id_wrong_types_refuse_at_strict_storage_boundary() {
    for value in [
        rusqlite::types::Value::Blob(vec![0x80]),
        rusqlite::types::Value::Null,
    ] {
        let (f, _, _) = private_invitation_id_fixture("cleanup");
        let before = private_invitation_id_snapshot(&f.db);
        assert!(
            repair_private_invitation_id(&f, value, false).is_err(),
            "STRICT TEXT NOT NULL blocks constructing wrong-type corruption"
        );
        assert_eq!(private_invitation_id_snapshot(&f.db), before);
    }
}

fn private_watch_cleanup_id_fixture() -> (Fixture, CallerClaim) {
    // Keep a genuine Hook allocation/owner alongside independently initialized
    // passive Watch state; the corruption fixture must not manufacture a binding.
    let (f, recipient, ordinal) = private_invitation_id_fixture("cleanup");
    f.store
        .presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    f.db.execute(
        "UPDATE invitation_presentation_consumers SET mod_initial_after=?1,mod_initial_high=?1,mod_context_after_seq=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),mod_context_high=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),mod_context_after_thread=?2,mod_context_thread='',mod_context_selected_seq=0,mod_context_invitation_after=0,mod_context_invitation_high=0,mod_cleanup_after=0,mod_cleanup_high=?1 WHERE seat_id='s' AND consumer='watch'",
        params![ordinal, "\u{10ffff}"],
    ).unwrap();
    let selected = f.db.query_row(
        "SELECT (SELECT COUNT(*) FROM digest_pending_invitations d WHERE d.seat_id=s.seat_id AND d.ordinal>s.mod_initial_after AND d.ordinal<=s.mod_initial_high),(SELECT COUNT(*) FROM invitation_context_latest c WHERE c.instance_id=s.instance_id AND (c.last_change_seq,c.thread_id)>(s.mod_context_after_seq,s.mod_context_after_thread) AND c.last_change_seq<=s.mod_context_high),(SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer='watch' AND i.revisit_ordinal>s.mod_cleanup_after AND i.revisit_ordinal<=s.mod_cleanup_high AND i.mod_key!='' AND i.mod_generation=s.mod_generation AND i.mod_execution=s.mod_execution) FROM invitation_presentation_consumers s WHERE s.seat_id='s' AND s.consumer='watch'",
        [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    ).unwrap();
    assert_eq!(
        selected,
        (0, 0, 1),
        "only the owned passive Watch cleanup row is reachable"
    );
    assert!(f.db.query_row(
        "SELECT s.mod_generation=b.generation AND s.mod_execution=b.execution_id AND b.native_session=?1 FROM invitation_presentation_consumers s JOIN occupant_bindings b ON b.seat_id=s.seat_id AND b.ended_at IS NULL WHERE s.seat_id='s' AND s.consumer='watch'",
        [recipient.native_session.as_str()], |r| r.get::<_, bool>(0),
    ).unwrap(), "passive cleanup retains the genuine canonical owner");
    (f, recipient)
}

#[test]
fn presentation_private_watch_cleanup_ids_refuse_malformed_identity_without_effects() {
    use crate::protocol::results::{ApiError, ErrorCode};
    let mut failures = Vec::new();
    for (kind, id) in [
        ("empty", ""),
        ("space", "bad id"),
        ("tab", "bad\tid"),
        ("control", "bad\0id"),
        ("non_ascii", "λ"),
    ] {
        let (f, recipient) = private_watch_cleanup_id_fixture();
        assert_eq!(
            repair_private_invitation_id_for_consumer(
                &f,
                "watch",
                rusqlite::types::Value::Text(id.into()),
                true,
            )
            .unwrap(),
            1
        );
        let before = private_invitation_id_snapshot(&f.db);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.store.presentation_refresh(
                &recipient.seat,
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
        }));
        if !matches!(
            result,
            Ok(Err(ApiError {
                code: ErrorCode::StoreCorrupt,
                ..
            }))
        ) {
            failures.push(format!(
                "passive_watch_cleanup/{kind}: expected StoreCorrupt without panic, got {result:?}"
            ));
        }
        if private_invitation_id_snapshot(&f.db) != before {
            failures.push(format!(
                "passive_watch_cleanup/{kind}: private or canonical rows changed after refusal"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn presentation_private_watch_cleanup_ids_enforce_borrowed_text_bounds_without_effects() {
    use crate::protocol::results::{ApiError, ErrorCode};
    for (kind, value) in [
        ("overlong", rusqlite::types::Value::Text("x".repeat(129))),
        ("invalid_utf8", rusqlite::types::Value::Blob(vec![0x80])),
    ] {
        let (f, recipient) = private_watch_cleanup_id_fixture();
        assert_eq!(
            repair_private_invitation_id_for_consumer(&f, "watch", value, true).unwrap(),
            1
        );
        let before = private_invitation_id_snapshot(&f.db);
        let result = f.store.presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(ApiError {
                    code: ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "{kind}: {result:?}"
        );
        assert_eq!(private_invitation_id_snapshot(&f.db), before);
    }
}

#[test]
fn presentation_private_watch_cleanup_ids_allow_128_byte_id_without_canonical_owner_changes() {
    let (f, recipient) = private_watch_cleanup_id_fixture();
    let owner = || {
        f.db.query_row(
        "SELECT generation,native_session,execution_id,observation_provenance FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
        [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
    ).unwrap()
    };
    let before_owner = owner();
    assert_eq!(
        repair_private_invitation_id_for_consumer(
            &f,
            "watch",
            rusqlite::types::Value::Text("x".repeat(128)),
            true,
        )
        .unwrap(),
        1
    );
    let output = f
        .store
        .presentation_refresh(
            &recipient.seat,
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(output.context_visits, 0);
    assert_eq!(
        output.lifecycle_visits, 1,
        "only passive cleanup is visited"
    );
    assert_eq!(output.ordinary_publication_visits, 0);
    assert_eq!(owner(), before_owner);
    assert_eq!(f.db.query_row("SELECT COUNT(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='watch'", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "valid orphan private row is reclaimed");
    assert_eq!(f.db.query_row("SELECT COUNT(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook'", [], |r| r.get::<_, i64>(0)).unwrap(), 1, "Hook owner state remains intact");
    assert_eq!(
        f.db.query_row(
            "SELECT COUNT(*) FROM invitations WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

// Kills: retrying the same absent first 100 IDs forever, or treating misses as settlement.
#[test]
fn presentation_capacity_missing_due_keys_finish_a_finite_pass() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let window = OfferWindowFixture::new(true);
    // Prime independent controls/publication before installing conservative hints.
    for _ in 0..4 {
        window.pull();
    }
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .seat_mut(
                &window.recipient.seat,
                window.recipient.binding_generation,
                window.recipient.native_session.as_str(),
            )
            .unwrap();
        for n in 0..256 {
            entry.record(
                &MessageId::new(format!("absent-{n:03}")),
                window.f.clock.utc_now(),
                false,
            );
        }
    }
    advance(&window.f, REOFFER_AFTER_MS);
    let original = window.offers();
    let mut visits = Vec::new();
    for _ in 0..3 {
        let reply = window.pull();
        visits.push(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
        );
        assert_eq!(
            window.offers(),
            original,
            "a missing row is not terminal evidence"
        );
        assert!(
            matches!(reply, DeliveryPullReplyV2::Unchanged { .. })
                || matches!(reply, DeliveryPullReplyV2::Page(ref page) if page.offered.is_empty() && page.truncated.is_empty())
        );
    }
    assert_eq!(visits, vec![100, 100, 56]);
    for _ in 0..3 {
        assert!(matches!(
            window.pull(),
            DeliveryPullReplyV2::Unchanged { .. }
        ));
        assert_eq!(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
            0
        );
        assert_eq!(window.offers(), original);
    }
}

// Kills CDR-1: conservative missing due hints perpetuating a completed normal scan.
#[test]
fn presentation_capacity_reclaimed_or_initially_free_with_missing_due_keys_quiets() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for reclaim in [false, true] {
        let window = OfferWindowFixture::new(true);
        let terminal = reclaim.then(|| window.send(0, DeliveryMode::Ordinary, "settled"));
        if let Some(id) = &terminal {
            assert_eq!(
                window.drain(1),
                std::collections::BTreeSet::from([id.clone()])
            );
            window.ack(std::slice::from_ref(id), false, false, "capacity-terminal");
        }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .seat_mut(
                    &window.recipient.seat,
                    window.recipient.binding_generation,
                    window.recipient.native_session.as_str(),
                )
                .unwrap();
            for n in 0..255 {
                entry.record(
                    &MessageId::new(format!("absent-{n:03}")),
                    window.f.clock.utc_now(),
                    false,
                );
            }
        }
        advance(&window.f, REOFFER_AFTER_MS);
        let original = window.offers();
        // Only one source row exists; four quanta allow control and publication work.
        let mut quiet = false;
        for _ in 0..4 {
            quiet = match window.pull() {
                DeliveryPullReplyV2::Unchanged { .. } => true,
                DeliveryPullReplyV2::Page(page) => !page.more,
                other => panic!("admitted fixture refused: {other:?}"),
            };
            if quiet {
                break;
            }
        }
        assert!(
            quiet,
            "missing due hints cannot restart a finished positive-capacity pass"
        );
        let retained = window.offers();
        assert_eq!(retained.len(), 255);
        for (id, stamp) in retained {
            assert_eq!(original[&id], stamp);
        }
    }
}


// Kills: final continuation using input slots instead of remaining capacity.
#[test]
fn presentation_capacity_last_slot_parks_unknown_carry_and_only_continues_for_due_keys() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for missing_due in [false, true] {
        let window = OfferWindowFixture::new(true);
        for _ in 0..4 { window.pull(); }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger.seat_mut(&window.recipient.seat, window.recipient.binding_generation, window.recipient.native_session.as_str()).unwrap();
            for n in 0..255 {
                let now = window.f.clock.utc_now();
                entry.record(&MessageId::new(format!("missing-{n:03}")),
                    if missing_due && n == 0 { UtcMillis(now.0 - REOFFER_AFTER_MS) } else { now }, false);
            }
        }
        let first = window.send(0, DeliveryMode::Ordinary, "last slot");
        let parked_id = window.send(1, DeliveryMode::Ordinary, "must wait");
        let mut allocated = false;
        for _ in 0..4 {
            if let DeliveryPullReplyV2::Page(page) = window.pull() && page.offered.contains(&first) {
                assert_eq!(page.more, missing_due, "only a distinct runnable due key justifies continuation");
                allocated = true;
                break;
            }
        }
        assert!(allocated);
        let parked = capacity_normal_state(&window);
        assert!(matches!(window.pull(), DeliveryPullReplyV2::Unchanged { .. }));
        assert_eq!(capacity_normal_state(&window), parked);
        let work = window.f.service.presentation.last_work.lock().unwrap().unwrap();
        assert_eq!(work.ordinary, usize::from(missing_due));
        assert_eq!(window.offers().len(), 256);
        assert!(!window.offers().contains_key(&parked_id));
        window.ack(std::slice::from_ref(&first), false, false, "last-slot-ack");
        let DeliveryPullReplyV2::Page(page) = window.pull() else { panic!("resume retained sibling"); };
        assert!(page.offered.contains(&parked_id));
    }
}
