//! Real owner contention must observe budgets before canonical admission.
use herdr_threads::{
    ports::{LocalService, ModChannelSink, ModChannels, StorePort, WatchReplaySnapshot},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint, PeerIdentity},
        commands::{CheckIn, CheckInMode, Command, InvitationConfirmation, Invite},
        delivery::{
            DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2, DeliveryPullReplyV2,
            DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        },
        ids::{
            BindingId, ExecutionId, HostTargetId, NativeSessionId, OperationId, SeatId, ThreadId,
        },
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
        watch::{ModDeliverySetting, WatchFrame, WatchRequest},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter, mod_channels::ModChannelRegistry},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::{Connection, types::Value};
use std::{
    os::unix::fs::DirBuilderExt,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(2);

struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

// Instrument only permit lookup; owner identity, admission, canonical work and
// replay application are real. A signal proves the worker reached this owner,
// rather than merely being spawned before its budget expired.
struct ObservedChannels {
    registry: Arc<ModChannelRegistry>,
    requested: Mutex<Option<mpsc::Sender<()>>>,
}
impl ModChannels for ObservedChannels {
    fn presentation_owner(&self, seat: &SeatId) -> Result<Option<Arc<Mutex<()>>>, ApiError> {
        let owner = self.registry.presentation_owner(seat)?;
        if let Some(signal) = self.requested.lock().unwrap().take() {
            signal.send(()).unwrap();
        }
        Ok(owner)
    }
    fn presentation_owner_live(
        &self,
        seat: &SeatId,
        binding: &BindingId,
        generation: u64,
        native: &str,
        lease: &str,
    ) -> bool {
        self.registry
            .presentation_owner_live(seat, binding, generation, native, lease)
    }
    fn presentation_replay_snapshot(
        &self,
        claim: &CallerClaim,
        lease: &str,
        known: &herdr_threads::delivery::ledger::OrdinaryDueSnapshot,
    ) -> Option<WatchReplaySnapshot> {
        self.registry
            .presentation_replay_snapshot(claim, lease, known)
    }
    fn apply_presentation_replay(
        &self,
        snapshot: WatchReplaySnapshot,
        completed: &std::collections::BTreeMap<
            herdr_threads::protocol::ids::MessageId,
            herdr_threads::delivery::ledger::Offer,
        >,
        current: &herdr_threads::delivery::ledger::OrdinaryDueSnapshot,
    ) {
        let permit = self
            .registry
            .presentation_owner(&snapshot.scope.seat)
            .unwrap()
            .unwrap();
        assert!(
            matches!(permit.try_lock(), Err(TryLockError::WouldBlock)),
            "same owner must remain held through successful replay installation"
        );
        self.registry
            .apply_presentation_replay(snapshot, completed, current);
    }
}

struct Sink;
impl ModChannelSink for Sink {
    fn push(&self, _: WatchFrame) -> bool {
        true
    }
}

struct Fixture {
    service: Arc<DomainService>,
    channels: Arc<ObservedChannels>,
    clock: Arc<TestClock>,
    pull: DeliveryPullV2,
    confirm: DeliveryPresentationConfirmV2,
    owner: Arc<Mutex<()>>,
    db: Connection,
    root: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Default::default(),
    }
}
fn peer() -> PeerIdentity {
    herdr_threads::test_support::peer_identity(501)
}

impl Fixture {
    fn new(consumer: PresentationConsumer) -> Self {
        let root =
            std::env::temp_dir().join(format!("presentation-owner-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let clock = Arc::new(TestClock(AtomicU64::new(100)));
        let context = StoreContext::new(root.join("store.db"), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,3);
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',1,0,0);
            INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,0,1,'fresh','unknown','unknown',0,0,'term-p','inc','coherent_enumeration',1);
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,1);
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('x','i','resolved','native','px',1,0,0);
            INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','px','b',1,0,1,'fresh','unknown','unknown',0,0,'term-px','inc','coherent_enumeration',1);
            INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','x','joined');
            INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','x',1,1);").unwrap();
        let store: Arc<dyn StorePort> =
            Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
        let registry = Arc::new(ModChannelRegistry::new(
            clock.clone(),
            store.clone(),
            "i".into(),
            Arc::new(|| {}),
            ModDeliverySetting::On,
        ));
        let service = DomainService::new("i".into(), store.clone(), clock.clone())
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
            .with_mod_channels(registry.clone());
        let harness = match consumer {
            PresentationConsumer::Hook => Harness::Agent(
                herdr_threads::harness::registry::builtins()
                    .agent("synthetic_fourth")
                    .unwrap(),
            ),
            PresentationConsumer::Watch => Harness::Claude,
        };
        let claim = CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness,
            native_session: NativeSessionId::new("owner-session"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
            target: HostTargetId::new("p"),
            agent_id: None,
            liveness: Some(LivenessFingerprint::TEST),
        };
        let CommandResult::CheckedIn(checked) = service
            .handle(
                Command::CheckIn(CheckIn {
                    mode: CheckInMode::Lifecycle {
                        expected_binding_generation: 1,
                    },
                    claim,
                    operation: OperationId::new("owner-ci"),
                    lineage_parent: None,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("canonical check-in")
        };
        let claim = checked.context;
        let mut sender = claim.clone();
        sender.seat = SeatId::new("x");
        sender.target = HostTargetId::new("px");
        sender.native_session = NativeSessionId::new("owner-sender");
        sender.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
        sender.liveness = Some(LivenessFingerprint { pid: 2, start: 1 });
        let CommandResult::CheckedIn(sender) = service
            .handle(
                Command::CheckIn(CheckIn {
                    mode: CheckInMode::Lifecycle {
                        expected_binding_generation: 1,
                    },
                    claim: sender,
                    operation: OperationId::new("owner-sender-ci"),
                    lineage_parent: None,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("canonical sender check-in")
        };
        assert!(matches!(
            service
                .handle(
                    Command::Invite(Invite {
                        claim: sender.context,
                        thread: ThreadId::new("t"),
                        seat: claim.seat.clone(),
                        confirmation: Some(InvitationConfirmation::Lazy),
                        deadline_millis: None,
                        operation: OperationId::new("owner-invite"),
                    }),
                    peer(),
                    &budget()
                )
                .unwrap(),
            CommandResult::Invitation(_)
        ));
        let lease = if consumer == PresentationConsumer::Watch {
            let (channel, _) = service
                .watch_register(
                    &WatchRequest {
                        presentation_v2: true,
                        claim: claim.clone(),
                    },
                    Arc::new(Sink),
                    &budget(),
                )
                .unwrap();
            Some(registry.presentation_lease(channel).unwrap())
        } else {
            None
        };
        let channels = Arc::new(ObservedChannels {
            registry,
            requested: Mutex::new(None),
        });
        let owner = channels.presentation_owner(&claim.seat).unwrap().unwrap();
        let service = Arc::new(service.with_mod_channels(channels.clone()));
        let pull = DeliveryPullV2 {
            claim: claim.clone(),
            consumer,
            presentation_lease: lease.clone(),
            marker_prefix: vec![],
            render: match consumer {
                PresentationConsumer::Hook => PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                PresentationConsumer::Watch => PresentationRenderRequest::WatchJsonLinesV1,
            },
        };
        let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = service
            .handle(Command::DeliveryPullV2(pull.clone()), peer(), &budget())
            .unwrap()
        else {
            panic!("real context presentation")
        };
        let confirm = DeliveryPresentationConfirmV2 {
            claim,
            consumer,
            presentation_lease: lease,
            token: page
                .presentation_token
                .expect("real outstanding presentation token"),
            emitted_controls: page.emitted_controls,
        };
        Self {
            service,
            channels,
            clock,
            pull,
            confirm,
            owner,
            db,
            root,
        }
    }

    fn bookend(&self) -> Vec<Vec<Vec<Value>>> {
        [
            "invitation_presentation_consumers",
            "invitation_presentation_items",
            "presentation_warning_proofs",
            "presentation_warning_fence",
            "occupant_bindings",
            "binding_cursors",
            "receipts",
            "receipt_state",
            "lazy_recipients",
            "invitations",
            "memberships",
            "wake_work",
            "warning_offer",
            "digest_notice_offer",
            "host_instances",
        ]
        .into_iter()
        .map(|table| {
            let columns = self
                .db
                .prepare(&format!("SELECT * FROM {table}"))
                .unwrap()
                .column_count();
            let order = (1..=columns)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut statement = self
                .db
                .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
                .unwrap();
            statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|n| row.get(n))
                        .collect::<rusqlite::Result<Vec<Value>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
    }

    fn command(&self, confirm: bool) -> Command {
        if confirm {
            Command::DeliveryPresentationConfirmV2(self.confirm.clone())
        } else {
            Command::DeliveryPullV2(self.pull.clone())
        }
    }
}

fn exhausted_waiter(confirm: bool, cancelled: bool) {
    for consumer in [PresentationConsumer::Hook, PresentationConsumer::Watch] {
        let f = Fixture::new(consumer);
        let before = f.bookend();
        let held = f.owner.lock().unwrap();
        let (entered, requested) = mpsc::channel();
        *f.channels.requested.lock().unwrap() = Some(entered);
        let service = f.service.clone();
        let command = f.command(confirm);
        let waiter_budget = budget();
        let running_budget = waiter_budget.clone();
        let (completed, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            completed
                .send(service.handle(command, peer(), &running_budget))
                .unwrap();
        });
        let reached_owner = requested.recv_timeout(WAIT);
        if cancelled {
            waiter_budget.cancellation.cancel();
        } else {
            f.clock.0.store(1_000, Ordering::SeqCst);
        }
        let before_release = result.recv_timeout(WAIT);
        // Cleanup precedes every assertion, including the intentionally RED one.
        // The old blocking implementation is released and joined, never detached.
        drop(held);
        worker.join().unwrap();
        reached_owner.expect("worker requested the held exact owner");
        let outcome = before_release.expect("exhausted waiter must return BEFORE owner release");
        assert_eq!(
            outcome.unwrap_err().code,
            if cancelled {
                ErrorCode::Cancelled
            } else {
                ErrorCode::DeadlineExceeded
            }
        );
        assert_eq!(
            f.bookend(),
            before,
            "no private, reader, receipt or binding effects"
        );
        f.clock.0.store(100, Ordering::SeqCst);
        let CommandResult::DeliveryPresentationConfirmedV2(reply) = f
            .service
            .handle(f.command(true), peer(), &budget())
            .unwrap()
        else {
            panic!("confirmation after exhausted waiter")
        };
        assert_eq!(
            reply.status,
            DeliveryPresentationConfirmationV2::Confirmed,
            "exhausted waiter must not consume the outstanding token"
        );
    }
}

// Kills both uninterruptible owner locks and admission after budget exhaustion.
#[test]
fn presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects() {
    exhausted_waiter(false, true);
}
#[test]
fn presentation_pull_expired_owner_waiter_returns_before_release_without_effects() {
    exhausted_waiter(false, false);
}
#[test]
fn presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects() {
    exhausted_waiter(true, true);
}
#[test]
fn presentation_confirm_expired_owner_waiter_returns_before_release_without_effects() {
    exhausted_waiter(true, false);
}

// Kills try_lock-and-refuse and any owner bypass: a live waiter remains pending
// while held, then performs its real canonical operation after release.
#[test]
fn presentation_live_owner_waiters_serialize_and_succeed_after_release() {
    for consumer in [PresentationConsumer::Hook, PresentationConsumer::Watch] {
        for confirm in [false, true] {
            let f = Fixture::new(consumer);
            let before = f.bookend();
            let held = f.owner.lock().unwrap();
            let (entered, requested) = mpsc::channel();
            *f.channels.requested.lock().unwrap() = Some(entered);
            let service = f.service.clone();
            let command = f.command(confirm);
            let (completed, result) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                completed
                    .send(service.handle(command, peer(), &budget()))
                    .unwrap();
            });
            let reached_owner = requested.recv_timeout(WAIT);
            let while_held = result.recv_timeout(Duration::from_millis(50));
            let during = f.bookend();
            drop(held);
            let after_release = result.recv_timeout(WAIT);
            worker.join().unwrap();
            reached_owner.unwrap();
            assert!(
                matches!(while_held, Err(mpsc::RecvTimeoutError::Timeout)),
                "live waiter must serialize behind the held owner"
            );
            assert_eq!(during, before, "held owner fences canonical effects");
            match after_release.unwrap().unwrap() {
                CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) if !confirm => {
                    assert_eq!(
                        page.presentation_token.as_deref(),
                        Some(f.confirm.token.as_str())
                    );
                }
                CommandResult::DeliveryPresentationConfirmedV2(reply) if confirm => {
                    assert_eq!(reply.status, DeliveryPresentationConfirmationV2::Confirmed);
                }
                other => panic!("successful contended presentation: {other:?}"),
            }
        }
    }
}

#[test]
fn presentation_poisoned_owner_recovers_for_pull_and_confirm() {
    for consumer in [PresentationConsumer::Hook, PresentationConsumer::Watch] {
        let f = Fixture::new(consumer);
        let owner = f.owner.clone();
        assert!(
            std::thread::spawn(move || {
                let _held = owner.lock().unwrap();
                panic!("isolated owner poisoning control");
            })
            .join()
            .is_err()
        );
        assert!(matches!(
            f.service
                .handle(f.command(false), peer(), &budget())
                .unwrap(),
            CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(_))
        ));
        let CommandResult::DeliveryPresentationConfirmedV2(reply) = f
            .service
            .handle(f.command(true), peer(), &budget())
            .unwrap()
        else {
            panic!("poisoned owner confirmation")
        };
        assert_eq!(reply.status, DeliveryPresentationConfirmationV2::Confirmed);
    }
}
