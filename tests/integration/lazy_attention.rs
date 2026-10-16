//! Passive delivery must remain absent from every actionable projection.
//! These tests use real canonical store mutations; no daemon or global state.
use herdr_threads::{
    ports::{
        DueScanRequest, DueScanState, DurableWorkAdmission, ReadContext, RegisterAvailableRequest,
        SendPreparationProgress, StorePort,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::*,
        ids::*,
        output::OutputSpec,
        pagination::PageRequest,
        priority::DeliveryPriority,
        results::{CheckInResult, CommandResult, ErrorCode, InboxBatchV2Item},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    store::{self, SqliteStore, StoreSettings, attention, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use rusqlite::{Connection, params};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct FixedClock(AtomicI64);
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::Relaxed))
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
fn read_context() -> ReadContext {
    ReadContext {
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    }
}
struct Fixture {
    // SQLite owners drop before the isolation directory.
    store: SqliteStore,
    db: Connection,
    clock: Arc<FixedClock>,
    _iso: TestIsolation,
}
impl Fixture {
    fn new() -> Self {
        let iso = TestIsolation::new("lazy-attention");
        let clock = Arc::new(FixedClock(AtomicI64::new(1000)));
        let context = StoreContext::new(iso.path("store.db"), clock.clone()).with_test_host();
        let db = context.open_writer().unwrap();
        db.execute_batch("UPDATE herdr_hosts SET host_boot='b',host_epoch=1; UPDATE local_node SET decision_seq=1;
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0);").unwrap();
        let store = SqliteStore::new(
            StoreContext::new(iso.path("store.db"), clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap();
        let f = Self {
            store,
            db,
            clock,
            _iso: iso,
        };
        f.member("author", "resolved", "joined", Harness::Codex);
        f.member("agent", "resolved", "joined", Harness::Codex);
        f
    }
    fn member(&self, seat: &str, state: &str, membership: &str, harness: Harness) {
        self.db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,retired_at,retired_seq,home_node,is_local,host_id) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b',?2,'native',?1,1,0,0,CASE WHEN ?2='retired' THEN 0 END,CASE WHEN ?2='retired' THEN 1 END,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",params![seat,state]).unwrap();
        self.db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch,ui_state) VALUES ((SELECT min(host_id) FROM herdr_hosts),?1,'b',1,0,1,'fresh',0,'term-'||?1,'inc','coherent_enumeration',1,'idle')",[seat]).unwrap();
        self.db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES(?1,1,?1,'b',1,0,?2,'session-'||?1,?3,?4,0,0,'term-'||?1,'inc')",params![seat,harness.as_str(), execution(seat), if harness==Harness::Human {"operator_human"} else {"cooperative_top_level"}]).unwrap();
        self.db
            .execute(
                "INSERT INTO memberships(thread_id,seat_id,state) VALUES('t',?1,?2)",
                params![seat, membership],
            )
            .unwrap();
        if membership == "joined" {
            self.db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t',?1,1,1)",[seat]).unwrap();
            // A binding open while its seat joins holds a cursor for the thread (join seeds it).
            self.db.execute("INSERT INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t',0 FROM occupant_bindings WHERE seat_id=?1",[seat]).unwrap();
        }
    }
    fn claim(&self, seat: &str) -> CallerClaim {
        let (generation, harness, saved_execution): (i64,String,String) = self.db.query_row("SELECT generation,harness,execution_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",[seat],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        CallerClaim {
            host: herdr_threads::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: SeatId::new(seat),
            binding_generation: generation as u64,
            role: CallerRole::TopLevel,
            harness: if harness == "human" {
                Harness::Human
            } else {
                Harness::Codex
            },
            native_session: NativeSessionId::new(format!("session-{seat}")),
            execution: ExecutionId::new(saved_execution),
            target: HostTargetId::new(seat),
            agent_id: None,
            liveness: None,
        }
    }
    fn mutate(
        &self,
        mutation: PermitMutation,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        let permit = self
            .store
            .issue_cooperative_permit(store::cooperative_permit_request(&mutation)?, &budget())?;
        self.store.mutate(mutation, permit, &budget())
    }
    fn send(&self, op: &str, mode: DeliveryMode) -> MessageId {
        self.send_as("author", op, mode)
    }
    fn send_as(&self, seat: &str, op: &str, mode: DeliveryMode) -> MessageId {
        let request = SendMessage {
            priority: None,
            delivery_mode: mode,
            thread: ThreadId::new("t"),
            body: format!("announcement {op}"),
            direct_recipient: None,
            // Only an explicit ACK recipient owes a receipt (D9): an ordinary send names `agent`.
            invited_recipients: if mode == DeliveryMode::Ordinary && seat != "agent" {
                vec![SeatId::new("agent")]
            } else {
                vec![]
            },
            deadline_millis: None,
            operation: OperationId::new(op),
            claim: self.claim(seat),
            relays_user: false,
            user_intent: None,
        };
        self.publish(request)
    }
    fn publish(&self, request: SendMessage) -> MessageId {
        for _ in 0..100 {
            if matches!(
                self.store
                    .prepare_send_step(&request, DurableWorkAdmission { max_units: 1 }, &budget())
                    .unwrap(),
                SendPreparationProgress::Ready { .. }
            ) {
                let CommandResult::MessageSent(id) =
                    self.mutate(PermitMutation::SendMessage(request)).unwrap()
                else {
                    panic!("send")
                };
                return id;
            }
        }
        panic!("preparation did not converge")
    }
    fn check(&self, seat: &str, op: &str, startup: bool) -> CheckInResult {
        self.check_claim(self.claim(seat), op, startup)
    }
    fn check_claim(&self, mut claim: CallerClaim, op: &str, startup: bool) -> CheckInResult {
        claim.liveness = Some(herdr_threads::protocol::authority::LivenessFingerprint::TEST);
        if startup {
            claim.execution = ExecutionId::new(uuid::Uuid::new_v4().to_string());
        }
        let command = CheckIn {
            lineage_parent: None,
            mode: if startup {
                CheckInMode::Lifecycle {
                    expected_binding_generation: claim.binding_generation,
                }
            } else {
                CheckInMode::Current
            },
            claim,
            operation: OperationId::new(op),
        };
        let permit = self
            .store
            .issue_cooperative_permit(
                store::cooperative_permit_request(&PermitMutation::CheckIn(command.clone()))
                    .unwrap(),
                &budget(),
            )
            .unwrap();
        let CommandResult::CheckedIn(result) = self
            .store
            .register_available(
                RegisterAvailableRequest {
                    command,
                    read: read_context(),
                    operator: None,
                },
                permit,
                &budget(),
            )
            .unwrap()
        else {
            panic!("check")
        };
        result
    }
    fn query(&self, command: Command) -> CommandResult {
        self.store
            .query(&command, &read_context(), &budget())
            .unwrap()
    }
    fn inbox(&self, seat: &str) -> Vec<MessageId> {
        let CommandResult::InboxBatchV2(page) = self.query(Command::InboxBatchV2(InboxQuery {
            pushed: false,
            thread: None,
            claim: None,
            seat: Some(SeatId::new(seat)),
            page: PageRequest::default(),
        })) else {
            panic!("inbox")
        };
        page.items
            .into_iter()
            .filter_map(|item| {
                if let InboxBatchV2Item::LazyMessage { message, .. } = item {
                    Some(message)
                } else {
                    None
                }
            })
            .collect()
    }
    fn reopen(&mut self) {
        self.store = SqliteStore::new(
            StoreContext::new(self._iso.path("store.db"), self.clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap();
    }
    fn count(&self, table: &str) -> i64 {
        self.db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    fn assert_passive(&self) {
        for table in [
            "receipts",
            "receipt_state",
            "prepared_recipients",
            "prepared_unavailable_warnings",
            "warning_conditions",
            "warning_jobs",
            "warning_recipients",
            "wake_work",
            "wake_batches",
            "delivery_observations",
        ] {
            assert_eq!(
                self.count(table),
                0,
                "unexpected attention producer {table}"
            );
        }
        assert_eq!(
            self.db
                .query_row(
                    "SELECT count(*) FROM work_jobs WHERE kind='send_attention'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let candidates = self
            .store
            .wake_candidates(
                herdr_threads::test_support::test_host_id(),
                PageRequest::default(),
                &budget(),
            )
            .unwrap();
        assert!(
            candidates.items.iter().all(|c| !c.has_actionable_work()),
            "{candidates:?}"
        );
        assert!(
            self.store
                .poke_candidates(herdr_threads::test_support::test_host_id(), 100, &budget())
                .unwrap()
                .is_empty()
        );
        assert!(!attention::seat_has_pending_rows(&self.db, "agent").unwrap());
        let digest = attention::wake_seat_attention(&self.db, "agent")
            .unwrap()
            .attention;
        assert!(
            !digest.has_pending_receipt
                && !digest.has_pending_invitation
                && digest.latest_warning_seq.is_none()
        );
    }
}
fn execution(seat: &str) -> &'static str {
    if seat == "author" {
        "00000000-0000-4000-8000-000000000001"
    } else {
        "00000000-0000-4000-8000-000000000002"
    }
}
fn offer_attention(offer: &CheckInResult) -> serde_json::Value {
    // Binding generations may advance on startup; actionable offer must not.
    serde_json::json!({"count":offer.warning_count,"more":offer.warning_count_has_more,"inbox":offer.inbox,"warnings":offer.warnings,"notices":offer.notices})
}

fn parsed_send(f: &Fixture, words: Vec<String>, operation: &str) -> SendMessage {
    use herdr_threads::cli::commands::{CliAction, parse_argv_exact_seats};
    let parsed = parse_argv_exact_seats(words).unwrap();
    let CliAction::Mutation(spec) = parsed.action else {
        panic!("expected send mutation")
    };
    let Command::SendMessage(mut send) = spec
        .into_command(Some(f.claim("author")), OperationId::new(operation))
        .unwrap()
    else {
        panic!("expected send command")
    };
    // `--nudge` alone is a priority-0 delivery row now; the receipt-bearing shape these controls
    // exercise names its recipient (D9). The CLI/guide wording is a later task's.
    if send.priority == Some(DeliveryPriority::p0()) && send.invited_recipients.is_empty() {
        send.invited_recipients = vec![SeatId::new("agent")];
    }
    send
}

/// Catches a shipped coordination-reply recipe selecting passive delivery
/// despite promising hook notification. Run the compiled guide, parse its
/// actual recipe, and publish it through the canonical store, not a prose snapshot.
#[test]
fn guide_daily_loop_reply_creates_notification_attention() {
    let mut output = Vec::new();
    herdr_threads::cli::run_in_pane(["herdr-threads", "skill"], None, &mut output).unwrap();
    let guide = String::from_utf8(output).unwrap();
    let daily = guide
        .split_once("## Daily loop (top-level agent)")
        .unwrap()
        .1;
    let recipe = daily
        .split_once("```bash\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0
        .lines()
        .find(|line| line.starts_with("herdr-threads send "))
        .expect("coordination reply recipe");
    let words = shlex::split(recipe.split('#').next().unwrap()).unwrap();
    let words = words
        .into_iter()
        .map(|word| if word == "THREAD" { "t".into() } else { word })
        .collect();
    let f = Fixture::new();
    let request = parsed_send(&f, words, "guide-reply");
    let mode = request.delivery_mode;
    let id = f.publish(request);
    assert!(
        attention::wake_seat_attention(&f.db, "agent")
            .unwrap()
            .attention
            .has_pending_receipt,
        "compiled daily-loop recipe selected {mode:?} and created no recipient receipt attention: {recipe}"
    );
    assert!(
        f.store
            .wake_candidates(
                herdr_threads::test_support::test_host_id(),
                PageRequest::default(),
                &budget()
            )
            .unwrap()
            .items
            .iter()
            .any(|candidate| candidate.has_actionable_work()),
        "coordination reply must be eligible for ordinary wake"
    );
    assert!(f.inbox("agent").is_empty(), "{id:?} must not be lazy mail");
}

/// Catches the parser adding attention to a bare announcement or failing
/// to promote an explicit receipt request through the real publication path.
#[test]
fn guide_send_passive_and_explicit_receipt_controls() {
    for (options, ordinary) in [
        (vec![], false),
        (vec!["-p", "1"], false),
        (vec!["--nudge"], true),
        (vec!["--require-ack", "agent"], true),
    ] {
        let f = Fixture::new();
        let mut words = vec!["herdr-threads", "send", "t", "--body", "announcement"];
        words.extend(options);
        let request = parsed_send(
            &f,
            words.into_iter().map(str::to_owned).collect(),
            "control",
        );
        let id = f.publish(request);
        if ordinary {
            assert!(
                attention::wake_seat_attention(&f.db, "agent")
                    .unwrap()
                    .attention
                    .has_pending_receipt
            );
            assert!(
                f.store
                    .wake_candidates(
                        herdr_threads::test_support::test_host_id(),
                        PageRequest::default(),
                        &budget()
                    )
                    .unwrap()
                    .items
                    .iter()
                    .any(|candidate| candidate.has_actionable_work())
            );
        } else {
            f.assert_passive();
            assert_eq!(f.inbox("agent").as_slice(), std::slice::from_ref(&id));
            f.mutate(PermitMutation::CompleteInboxDelivery(
                CompleteInboxDelivery {
                    receipt_candidates: Vec::new(),
                    via: None,
                    messages: vec![id],
                    operation: OperationId::new("display-complete"),
                    claim: f.claim("agent"),
                },
            ))
            .unwrap();
            assert!(f.inbox("agent").is_empty());
            f.assert_passive();
        }
    }
}

/// Catches lazy publication making a quiet joined thread Recent, replacing
/// ordinary last_activity on pending attention, or hiding explicit content.
#[test]
fn lazy_publication_does_not_create_recovery_recency() {
    use herdr_threads::{
        cli::hook::encode_native,
        harness::{Capability, LifecycleEvent, RecoveryRows, context},
        protocol::{
            results::HotReason,
            summary::{SummaryOutcome, SummaryRequest},
        },
    };

    let mut f = Fixture::new();
    let old = f.send("old-ordinary", DeliveryMode::Ordinary);
    let hot = |f: &Fixture, seat| {
        let CommandResult::HotThreads(hot) = f.query(Command::HotThreads(HotThreadsQuery {
            seat: SeatId::new(seat),
            limit: 8,
        })) else {
            panic!("hot threads")
        };
        hot
    };
    assert_eq!(hot(&f, "author").hot[0].reason, HotReason::Recent);
    // The ordinary message is more than the default 24-hour hot window old.
    f.clock.0.store(86_402_000, Ordering::Relaxed);
    assert!(hot(&f, "author").hot.is_empty());
    let lazy = f.send_as("agent", "lazy-arrival", DeliveryMode::Lazy);

    assert_eq!(f.inbox("author").as_slice(), std::slice::from_ref(&lazy));
    let CommandResult::History(history) = f.query(Command::History(HistoryQuery {
        thread: ThreadId::new("t"),
        page: PageRequest::default(),
        initial: None,
        full_bodies: true,
    })) else {
        panic!("history")
    };
    assert_eq!(
        history
            .items
            .iter()
            .map(|row| &row.message)
            .collect::<Vec<_>>(),
        [&lazy, &old]
    );
    let SummaryOutcome::Ready(summary) = f
        .store
        .summary(
            &SummaryRequest {
                thread: ThreadId::new("t"),
                claim: f.claim("author"),
            },
            &budget(),
        )
        .unwrap()
    else {
        panic!("small explicit summary should be ready")
    };
    assert_eq!(
        summary
            .tail
            .iter()
            .map(|row| &row.message)
            .collect::<Vec<_>>(),
        [&old, &lazy]
    );
    assert_eq!(f.inbox("author").as_slice(), std::slice::from_ref(&lazy));

    for reopened in [false, true] {
        if reopened {
            f.reopen();
        }
        let quiet = hot(&f, "author");
        assert!(
            quiet.hot.is_empty() && quiet.overflow.is_empty(),
            "{quiet:?}"
        );
        let recovery = RecoveryRows::from_hot_threads(&quiet);
        assert!(
            recovery.is_none(),
            "lazy mail must create no recovery block"
        );
        assert!(!attention::seat_has_pending_rows(&f.db, "author").unwrap());
        let attention = hot(&f, "agent");
        assert_eq!(attention.hot.len(), 1);
        assert_eq!(attention.hot[0].reason, HotReason::PendingReceipt);
        assert_eq!(attention.hot[0].last_activity, UtcMillis(1000));
        assert!(attention.overflow.is_empty());

        for harness in [context::Harness::Claude, context::Harness::Codex] {
            for (kind, source) in [
                (context::EventKind::Compact, "compact"),
                (context::EventKind::Resume, "resume"),
                (context::EventKind::Clear, "clear"),
            ] {
                let event = LifecycleEvent {
                    harness,
                    kind,
                    source: source.into(),
                    native_session: Some("session-author".into()),
                    role: context::Role::TopLevel,
                    event_id: uuid::Uuid::new_v4().to_string(),
                    capability: Capability::ObservedInput,
                    agent_id: None,
                };
                let output = encode_native(&event, b"", &[], None, None, None, recovery.as_ref());
                let output = String::from_utf8(output).unwrap();
                assert!(!output.contains(&herdr_threads::harness::recovery_instruction()));
                assert!(!output.contains("hot threads:"));
            }
        }
    }

    // A fresh ordinary publication still creates Recent recovery for its
    // author, without relying on a pending recipient receipt to make it hot.
    f.send("ordinary-control", DeliveryMode::Ordinary);
    let recent = hot(&f, "author");
    assert_eq!(recent.hot.len(), 1);
    assert_eq!(recent.hot[0].thread, ThreadId::new("t"));
    assert_eq!(recent.hot[0].reason, HotReason::Recent);
    assert_eq!(recent.hot[0].last_activity, UtcMillis(86_402_000));
    assert!(RecoveryRows::from_hot_threads(&recent).is_some());
}

/// Catches accidental inclusion of passive rows in hook attention/count/token.
#[test]
fn lazy_attention_current_startup_stable_across_restart() {
    let mut f = Fixture::new();
    let initial = f.check("agent", "baseline", false);
    let no_mail = f.check("agent", "no-mail-control", false);
    assert_ne!(
        initial.offered_through, no_mail.offered_through,
        "offer boundary advances even without any mail"
    );
    let baseline = offer_attention(&no_mail);
    assert_eq!(offer_attention(&initial), baseline);
    let digest = attention::seat_digest(
        &f.db,
        "34750f98bd59fcfc946da45aaabe933b",
        &SeatId::new("agent"),
        &|| Ok(()),
    )
    .unwrap()
    .digest;
    let id = f.send("lazy", DeliveryMode::Lazy);
    for round in 0..4 {
        if round == 2 {
            f.reopen();
        }
        for startup in [false, true] {
            assert_eq!(
                offer_attention(&f.check("agent", &format!("callback-{round}-{startup}"), startup)),
                baseline
            );
            f.assert_passive();
            assert_eq!(
                attention::seat_digest(
                    &f.db,
                    "34750f98bd59fcfc946da45aaabe933b",
                    &SeatId::new("agent"),
                    &|| Ok(())
                )
                .unwrap()
                .digest,
                digest
            );
            assert_eq!(f.inbox("agent").as_slice(), std::slice::from_ref(&id));
        }
    }
}

/// Lazy hook delivery (user clarification 2026-10-09): a lazy publication
/// moves the digest a hook asks for with lazy rows (its change marker), so
/// the next standard hook presents it, while every wake input stays
/// untouched: no attention producer, no wake candidate, no wake frontier
/// change. Catches lazy left out of the hook marker, and lazy leaking into
/// wake (the nudge must stay skipped).
#[test]
fn lazy_attention_moves_the_lazy_digest_marker_but_never_wake() {
    let f = Fixture::new();
    let lazy_digest = || {
        let mut digest = attention::seat_digest(
            &f.db,
            "34750f98bd59fcfc946da45aaabe933b",
            &SeatId::new("agent"),
            &|| Ok(()),
        )
        .unwrap()
        .digest;
        attention::add_lazy(&f.db, "34750f98bd59fcfc946da45aaabe933b", &mut digest).unwrap();
        digest.validate().unwrap();
        digest
    };
    let wake = || {
        format!(
            "{:?}",
            attention::wake_seat_attention(&f.db, "agent")
                .unwrap()
                .attention
        )
    };
    let before = lazy_digest();
    assert_eq!(before.lazy.as_ref().unwrap().count, 0);
    assert!(before.token.lazy.is_none() && before.is_empty());
    let wake_before = wake();
    let first = f.send("lazy", DeliveryMode::Lazy);
    let after = lazy_digest();
    assert!(after.token.advanced_beyond(&before.token));
    assert!(!after.is_empty());
    let class = after.lazy.as_ref().unwrap();
    assert_eq!(
        (class.count, class.items[0].id.as_str()),
        (1, first.as_str())
    );
    assert!(
        after
            .summary()
            .ends_with(&format!("; messages=1 [{}@t]", first.as_str()))
    );
    // The default digest (wake fingerprint inputs, old clients) is unchanged.
    let plain = attention::seat_digest(
        &f.db,
        "34750f98bd59fcfc946da45aaabe933b",
        &SeatId::new("agent"),
        &|| Ok(()),
    )
    .unwrap()
    .digest;
    assert!(plain.lazy.is_none() && plain.token.lazy.is_none() && plain.is_empty());
    assert_eq!(wake(), wake_before);
    assert!(
        !f.store
            .wake_candidates(
                herdr_threads::test_support::test_host_id(),
                PageRequest::default(),
                &budget()
            )
            .unwrap()
            .items
            .iter()
            .any(|c| c.has_actionable_work())
    );
    f.assert_passive();
    // A second arrival moves it again; the joined mark never regresses.
    let second = f.send("lazy-2", DeliveryMode::Lazy);
    let later = lazy_digest();
    assert!(
        later
            .token
            .advanced_beyond(&after.token.join(&before.token))
    );
    assert_eq!(later.lazy.as_ref().unwrap().count, 2);
    assert_eq!(later.lazy.as_ref().unwrap().items[0].id, second.as_str());
    assert_eq!(wake(), wake_before);
    f.assert_passive();
}

/// Catches indirect materialization of receipts/warnings by outstanding work.
#[test]
fn lazy_attention_no_indirect_outstanding_send_work() {
    let f = Fixture::new();
    f.member("unavailable", "unresolved", "joined", Harness::Codex);
    f.send("lazy", DeliveryMode::Lazy);
    for _ in 0..32 {
        let page = f
            .store
            .pending_work(PageRequest::default(), &budget())
            .unwrap();
        for job in page.items {
            f.store
                .advance_work(&job.id, DurableWorkAdmission { max_units: 1 }, &budget())
                .unwrap();
        }
        f.assert_passive();
    }
    assert_eq!(f.count("lazy_recipients"), 2);
}

/// Catches delayed attention work and deadline/poke candidates, not just an
/// empty send return. Ordinary control below proves the candidate path is live.
#[test]
fn lazy_attention_no_wake_poke_prompt_retry_deadline_warning() {
    let mut f = Fixture::new();
    f.send("lazy", DeliveryMode::Lazy);
    let mut state = DueScanState::default();
    for round in 0..8 {
        if round == 4 {
            f.reopen();
        }
        f.check("agent", &format!("current-{round}"), false);
        let progress = f
            .store
            .due_obligations(
                DueScanRequest {
                    state,
                    max_candidates: 16,
                    run_invitations: true,
                    run_receipts: true,
                },
                &budget(),
            )
            .unwrap();
        state = progress.state;
        f.assert_passive();
    }
    assert_eq!(f.count("lazy_recipients"), 1);
    assert_eq!(f.db.query_row("SELECT count(*) FROM receipts WHERE deadline_at IS NOT NULL OR soft_poked_at IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

/// Catches passive rows becoming receipt obligations, and a vacuous empty
/// attention fixture: the same real publication route must signal ordinary mail.
#[test]
fn lazy_attention_mixed_ordinary_positive_controls() {
    let f = Fixture::new();
    f.member("unavailable", "unresolved", "joined", Harness::Codex);
    let lazy = f.send("lazy", DeliveryMode::Lazy);
    f.assert_passive();
    f.send("ordinary", DeliveryMode::Ordinary);
    assert!(
        f.store
            .wake_candidates(
                herdr_threads::test_support::test_host_id(),
                PageRequest::default(),
                &budget()
            )
            .unwrap()
            .items
            .iter()
            .any(|c| c.has_actionable_work())
    );
    assert!(attention::seat_has_pending_rows(&f.db, "agent").unwrap());
    assert!(
        attention::wake_seat_attention(&f.db, "agent")
            .unwrap()
            .attention
            .has_pending_receipt
    );
    assert!(
        f.db.query_row(
            "SELECT count(*) FROM work_jobs WHERE kind='send_attention'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            > 0
    );
    assert!(
        f.count("warning_conditions") > 0,
        "unavailable ordinary audience warns"
    );
    f.member("outsider", "resolved", "left", Harness::Codex);
    f.mutate(PermitMutation::Invite(Invite {
        confirmation: None,
        thread: ThreadId::new("t"),
        seat: SeatId::new("outsider"),
        deadline_millis: None,
        operation: OperationId::new("invite"),
        claim: f.claim("author"),
    }))
    .unwrap();
    let digest = |seat| {
        attention::seat_digest(
            &f.db,
            "34750f98bd59fcfc946da45aaabe933b",
            &SeatId::new(seat),
            &|| Ok(()),
        )
        .unwrap()
        .digest
    };
    assert!(
        digest("outsider").invitations.count > 0,
        "ordinary invitation remains actionable"
    );
    assert!(
        digest("author").warnings.count > 0,
        "ordinary unavailable warning remains actionable"
    );
    let before = [digest("agent"), digest("author"), digest("outsider")];
    let second = f.send("lazy-after-ordinary", DeliveryMode::Lazy);
    assert_eq!(
        [digest("agent"), digest("author"), digest("outsider")],
        before
    );
    assert_eq!(f.inbox("agent"), [lazy, second]);
}

/// Catches audience expansion on later joins, disappearance on leave/archive,
/// synthetic settlement on retirement, or readonly content settling delivery.
#[test]
fn lazy_addressed_survives_leave_retire_archive_and_excludes_postjoin() {
    let mut f = Fixture::new();
    f.member("human", "resolved", "joined", Harness::Human);
    f.member("unavailable", "unresolved", "joined", Harness::Codex);
    f.member("invitee", "resolved", "invited", Harness::Codex);
    f.member("retired", "retired", "joined", Harness::Codex);
    let id = f.send("lazy", DeliveryMode::Lazy);
    f.member("later", "resolved", "joined", Harness::Codex);
    let seats =
        f.db.prepare("SELECT seat_id FROM lazy_recipients ORDER BY seat_id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    assert_eq!(seats, ["agent", "human", "unavailable"]);
    assert!(f.inbox("later").is_empty());
    f.mutate(PermitMutation::Leave(Leave {
        thread: ThreadId::new("t"),
        operation: OperationId::new("leave-agent"),
        claim: f.claim("agent"),
    }))
    .unwrap();
    f.mutate(PermitMutation::Archive(ThreadMutation {
        thread: ThreadId::new("t"),
        operation: OperationId::new("archive"),
        claim: f.claim("author"),
    }))
    .unwrap();
    f.reopen();
    assert_eq!(f.inbox("agent").as_slice(), std::slice::from_ref(&id));
    f.query(Command::History(HistoryQuery {
        thread: ThreadId::new("t"),
        page: PageRequest::default(),
        initial: None,
        full_bodies: true,
    }));
    f.store
        .summary(
            &herdr_threads::protocol::summary::SummaryRequest {
                thread: ThreadId::new("t"),
                claim: f.claim("author"),
            },
            &budget(),
        )
        .unwrap();
    f.query(Command::Message(MessageQuery {
        message: id.clone(),
        body: BodyReadRequest {
            cursor: None,
            offset: None,
            max_bytes: 16384,
        },
    }));
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM lazy_recipients WHERE seat_id='agent'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    f.mutate(PermitMutation::CompleteInboxDelivery(
        CompleteInboxDelivery {
            receipt_candidates: Vec::new(),
            via: None,
            messages: vec![id.clone()],
            operation: OperationId::new("display-human"),
            claim: f.claim("human"),
        },
    ))
    .unwrap();
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM lazy_recipients WHERE seat_id='human'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "displayed"
    );
    let retired_claim = f.claim("agent");
    let retirement = f
        .store
        .begin_retirement(
            SeatId::new("agent"),
            herdr_threads::ports::ClosureEvidence {
                host_boot: HostBootId::new("b"),
                epoch: 1,
                target: HostTargetId::new("agent"),
                generation: 0,
            },
            &budget(),
        )
        .unwrap();
    let mut complete = false;
    for _ in 0..100 {
        if f.store
            .advance_retirement(
                retirement.id.clone(),
                herdr_threads::ports::WorkAdmission::Background,
                &budget(),
            )
            .unwrap()
            .complete
        {
            complete = true;
            break;
        }
    }
    assert!(complete, "retirement must converge");
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM lazy_recipients WHERE seat_id='agent'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    let error = f
        .mutate(PermitMutation::CompleteInboxDelivery(
            CompleteInboxDelivery {
                receipt_candidates: Vec::new(),
                via: None,
                messages: vec![id],
                operation: OperationId::new("retired"),
                claim: retired_claim,
            },
        ))
        .unwrap_err();
    assert!(matches!(
        error.code,
        ErrorCode::Conflict | ErrorCode::CallerUnverified | ErrorCode::NotFound
    ));
    f.mutate(PermitMutation::Reopen(ThreadMutation {
        thread: ThreadId::new("t"),
        operation: OperationId::new("reopen"),
        claim: f.claim("author"),
    }))
    .unwrap();
    let next = f.send("after-retire", DeliveryMode::Lazy);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM lazy_recipients WHERE message_id=?1 AND seat_id='agent'",
            [next.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

/// Catches lazy backlog being added to archival receipt blockers; idle is
/// qualified through actual repeated composer observations, never assumed.
#[test]
fn lazy_attention_backlog_allows_qualified_quiet_archive() {
    use herdr_threads::{
        ports::HostUiState,
        store::archival::{self, Runtime},
    };
    let mut f = Fixture::new();
    let id = f.send("lazy", DeliveryMode::Lazy);
    for at in (0..=120_000).step_by(30_000) {
        let rt = Runtime {
            boot: "test-runtime".into(),
            mono: at,
            utc: UtcMillis(at),
            after_ms: 60_000,
            hosts: herdr_threads::test_support::test_host_evidence(0, true),
            legacy_source: Some("covered".into()),
        };
        archival::advance(&f.db, "34750f98bd59fcfc946da45aaabe933b", &rt).unwrap();
        for seat in ["author", "agent"] {
            if let Some(ticket) =
                archival::observation_ticket(&f.db, "34750f98bd59fcfc946da45aaabe933b", seat, &rt)
                    .unwrap()
            {
                let mut sample = crate::channel_archival::sample(at, HostUiState::Idle);
                sample.0.observation_sequence = (at / 30_000 + 2) as u64;
                sample.0.target = HostTargetId::new(seat);
                sample.0.terminal = Some(TerminalId::new(format!("term-{seat}")));
                assert!(archival::record_sample(&mut f.db, &ticket, &rt, &sample).unwrap());
            }
        }
        for _ in 0..20 {
            if !archival::advance(&f.db, "34750f98bd59fcfc946da45aaabe933b", &rt)
                .unwrap()
                .has_more
            {
                break;
            }
        }
        if at == 0 {
            assert!(
                !f.db
                    .query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
                        .get::<_, bool>(0))
                    .unwrap()
            );
        }
    }
    assert!(
        f.db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
            .get::<_, bool>(0))
            .unwrap(),
        "qualified quiet channel with passive backlog must archive"
    );
    assert_eq!(f.inbox("agent").as_slice(), std::slice::from_ref(&id));
    f.mutate(PermitMutation::CompleteInboxDelivery(
        CompleteInboxDelivery {
            receipt_candidates: Vec::new(),
            via: None,
            messages: vec![id],
            operation: OperationId::new("after-archive"),
            claim: f.claim("agent"),
        },
    ))
    .unwrap();
    assert!(f.inbox("agent").is_empty());
    assert_eq!(f.count("receipts"), 0);
}

/// ht-705: a nudge shown whole inside the recipient's turn (a tool-boundary hook page, which
/// sends `AdvanceCursors { in_turn: true }` after its output is flushed) was served in-turn, so
/// the idle wake after that turn is owed to no one (delivery contract rule 4). The receipt stays
/// pending (presentation is not an ACK). Kills: the post-turn duplicate wake on Codex; a display
/// outside a turn (lifecycle hook, CLI) suppressing the wake; one shown receipt hiding a newer
/// unshown one; a new occupant (lifecycle check-in) inheriting the old one's presentation.
#[test]
fn in_turn_display_of_a_nudge_owes_no_idle_wake() {
    let f = Fixture::new();
    let wake_owed = || {
        f.store
            .wake_candidates(
                herdr_threads::test_support::test_host_id(),
                PageRequest::default(),
                &budget(),
            )
            .unwrap()
            .items
            .iter()
            .any(|c| c.seat.as_str() == "agent" && c.has_actionable_work())
    };
    let display = |op: &str, displayed: Vec<MessageId>, in_turn: bool| {
        f.store
            .advance_cursors(
                &AdvanceCursors {
                    claim: f.claim("agent"),
                    operation: OperationId::new(op),
                    displayed,
                    in_turn,
                    invitations: Vec::new(),
                },
                &budget(),
            )
            .unwrap();
    };
    let receipt_pending = || {
        attention::wake_seat_attention(&f.db, "agent")
            .unwrap()
            .attention
            .has_pending_receipt
    };
    let first = f.send("nudge-1", DeliveryMode::Ordinary);
    assert!(
        wake_owed(),
        "control: an unshown nudge wakes an idle session"
    );
    display("shown-idle", vec![first.clone()], false);
    assert!(wake_owed(), "a display outside a turn leaves the wake owed");
    display("shown-in-turn", vec![first.clone()], true);
    assert!(!wake_owed(), "served in-turn: no idle wake");
    assert!(receipt_pending(), "presentation settles nothing");
    let second = f.send("nudge-2", DeliveryMode::Ordinary);
    assert!(wake_owed(), "a newer unshown nudge still wakes");
    display("shown-2", vec![second], true);
    assert!(!wake_owed());
    f.check("agent", "current", false);
    assert!(!wake_owed(), "a Current check-in keeps the presentation");
    f.check("agent", "new-session", true);
    assert!(wake_owed(), "a new occupant was not served");
}

/// ht-705: an invitation shown inside the invitee's turn (tool-boundary hook page) owes no idle
/// wake after the turn; it stays pending (nothing is accepted). (Warnings a Current check-in
/// carried are already spared by `warning_offer`: `scheduler::policy` tests.) Kills: the
/// post-turn invitation wake on Codex; an in-turn display accepting the invitation; a new
/// occupant inheriting it.
#[test]
fn in_turn_display_of_an_invitation_owes_no_idle_wake() {
    let f = Fixture::new();
    f.member("outsider", "resolved", "left", Harness::Codex);
    let wake_owed = |seat: &str| {
        f.store
            .wake_candidates(
                herdr_threads::test_support::test_host_id(),
                PageRequest::default(),
                &budget(),
            )
            .unwrap()
            .items
            .iter()
            .any(|c| c.seat.as_str() == seat && c.has_actionable_work())
    };
    f.mutate(PermitMutation::Invite(Invite {
        confirmation: None,
        thread: ThreadId::new("t"),
        seat: SeatId::new("outsider"),
        deadline_millis: None,
        operation: OperationId::new("invite"),
        claim: f.claim("author"),
    }))
    .unwrap();
    let invitation: String =
        f.db.query_row(
            "SELECT id FROM invitations WHERE seat_id='outsider'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        wake_owed("outsider"),
        "control: an unshown invitation wakes"
    );
    f.store
        .advance_cursors(
            &AdvanceCursors {
                claim: f.claim("outsider"),
                operation: OperationId::new("shown-in-turn"),
                displayed: Vec::new(),
                in_turn: true,
                invitations: vec![InvitationId::new(invitation.clone())],
            },
            &budget(),
        )
        .unwrap();
    assert!(!wake_owed("outsider"), "served in-turn: no idle wake");
    assert!(
        attention::wake_seat_attention(&f.db, "outsider")
            .unwrap()
            .attention
            .has_pending_invitation,
        "presentation accepts nothing"
    );
    f.check("outsider", "new-session", true);
    assert!(wake_owed("outsider"), "a new occupant was not served");
}

/// ht-705: in-turn service is per binding. Seat `agent` holds two live top-level sessions, older
/// `a` and newer `b` (distinct processes). Both served a nudge in their turns: nothing is owed. A
/// lifecycle check-in by `b` (a new session in that process slot) starts only `b` over: `a` keeps
/// its record, and the seat owes `b` its wake again. Kills: a lifecycle reset that clears the
/// sibling's service, and one that leaves the reset binding counted as served.
#[test]
fn a_sibling_lifecycle_check_in_resets_only_its_own_in_turn_service() {
    let f = Fixture::new();
    f.db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,liveness_pid,liveness_start) VALUES('agent',1,'agent','b',1,0,'codex','session-agent-b','00000000-0000-4000-8000-0000000000b2','cooperative_top_level',0,0,'term-agent','inc',20,1)",[]).unwrap();
    f.db.execute("INSERT INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t',0 FROM occupant_bindings WHERE native_session='session-agent-b'",[]).unwrap();
    let binding = |session: &str| -> String {
        f.db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE native_session=?1 AND ended_at IS NULL",
            [session],
            |r| r.get(0),
        )
        .unwrap()
    };
    let (a, b) = (binding("session-agent"), binding("session-agent-b"));
    let claim_a = f.claim("agent");
    assert_eq!(claim_a.native_session.as_str(), "session-agent");
    let claim_b = CallerClaim {
        host: herdr_threads::test_support::test_host_locator(),
        native_session: NativeSessionId::new("session-agent-b"),
        execution: ExecutionId::new("00000000-0000-4000-8000-0000000000b2"),
        ..claim_a.clone()
    };
    let wake_owed = || {
        f.store
            .wake_candidates(
                herdr_threads::test_support::test_host_id(),
                PageRequest::default(),
                &budget(),
            )
            .unwrap()
            .items
            .iter()
            .any(|c| c.seat.as_str() == "agent" && c.has_actionable_work())
    };
    let served = |binding: &str| -> i64 {
        f.db.query_row(
            "SELECT count(*) FROM in_turn_offer WHERE binding_id=?1",
            [binding],
            |r| r.get(0),
        )
        .unwrap()
    };
    let nudge = f.send("nudge", DeliveryMode::Ordinary);
    let show = |claim: &CallerClaim, op: &str| {
        f.store
            .advance_cursors(
                &AdvanceCursors {
                    claim: claim.clone(),
                    operation: OperationId::new(op),
                    displayed: vec![nudge.clone()],
                    in_turn: true,
                    invitations: Vec::new(),
                },
                &budget(),
            )
            .unwrap();
    };
    show(&claim_b, "shown-b");
    assert!(
        wake_owed(),
        "a was not served: the newer session's turn never hides a's wake"
    );
    show(&claim_a, "shown-a");
    assert!(!wake_owed(), "both live sessions were served in-turn");
    f.check_claim(claim_b, "b-new-session", true);
    assert_eq!(
        served(&a),
        1,
        "b's lifecycle check-in leaves a's service alone"
    );
    assert_eq!(served(&b), 0, "b starts over");
    assert_eq!(binding("session-agent"), a, "a stays open");
    assert!(wake_owed(), "the reset sibling is owed its wake again");
}
