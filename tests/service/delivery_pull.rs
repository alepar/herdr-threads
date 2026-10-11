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

#[test]
fn notices_wait_while_the_message_page_is_full() {
    let f = fixture(40);
    let context = pull_seat(&f);
    service_notice(&f, "w-wait");
    let first = page(pull(&f, &context, None));
    assert!(first.more);
    assert!(!first.text.contains("notice w-wait"), "{}", first.text);
    assert_eq!(notice_frontier(&f), 0);
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(second.text.contains("notice w-wait"), "{}", second.text);
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
