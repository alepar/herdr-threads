//! Seam integration ht-p03.42: wake outcomes across scheduler, dispatcher,
//! SQLite store and a scripted fake host. Only the host is fake; discovery,
//! reservation, the final fence, verification and completion are real.
use super::*;
use crate::{
    ports::AgentComposerState,
    scheduler::{SubmissionVerification, WakeDriveOutcome},
};
use std::collections::VecDeque;

/// Fake host with scripted pre-send refusal and scripted composer states.
struct SeamHost {
    clock: Arc<JumpClock>,
    refuse: AtomicBool,
    observes: AtomicU64,
    prompts: AtomicU64,
    submit_keys: AtomicU64,
    pane_states: Mutex<VecDeque<AgentComposerState>>,
}
impl SeamHost {
    fn sends(&self) -> u64 {
        self.prompts.load(Ordering::SeqCst) + self.submit_keys.load(Ordering::SeqCst)
    }
}
impl HostPort for SeamHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.observes.fetch_add(1, Ordering::SeqCst);
        if self.refuse.load(Ordering::SeqCst) {
            return Err(crate::scheduler::error(
                ErrorCode::HostUnavailable,
                "scripted refusal",
            ));
        }
        let now = self.clock.monotonic_now();
        let mut observation = fresh_observation();
        observation.host_boot = HostBootId::new("host");
        observation.observed_at_mono = now;
        observation.started_at_mono = now;
        observation.completed_at_mono = now;
        Ok(observation)
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        FakeNativeHost {
            observation: observation.clone(),
            submitted: AtomicU64::new(0),
            pane_states: Default::default(),
            submit_keys: AtomicU64::new(0),
        }
        .safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        assert_eq!(text, crate::notification::policy::MARKER);
        self.prompts.fetch_add(1, Ordering::SeqCst);
        Ok(PromptOutcome::Submitted)
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        let mut states = self.pane_states.lock().unwrap();
        Ok(if states.len() > 1 {
            states.pop_front().unwrap()
        } else {
            states
                .front()
                .copied()
                .unwrap_or(AgentComposerState::Submitted)
        })
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        self.submit_keys.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

const START_MS: u64 = 1_000_000;
/// A restarted runner waits the retained 30 s minimum before its first wake.
const FIRST_DUE_MS: u64 = START_MS + 30_000;

struct Seam {
    path: std::path::PathBuf,
    context: StoreContext,
    clock: Arc<JumpClock>,
    host: Arc<SeamHost>,
}
impl Seam {
    fn new(states: Vec<AgentComposerState>) -> Self {
        let path = std::env::temp_dir().join(format!(
            "herdr-wake-outcome-seam-{}.db",
            uuid::Uuid::new_v4()
        ));
        let clock = Arc::new(JumpClock {
            mono: AtomicU64::new(0),
            utc: AtomicI64::new(0),
        });
        let context = StoreContext::new(path.clone(), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute_batch("\
            INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('seat','i','resolved','native','target',1,1,0);\
            INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','target','host',1,1,0,'fresh','occupied','idle','execution',1,'term-'||'target','inc','coherent_enumeration',1);\
            INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('seat',1,1,'target','host',1,'codex','session','execution','fresh',0,0,'term-'||'target','inc');\
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('thread','i','topic','goal',0,0);\
            INSERT INTO memberships(thread_id,seat_id,state) VALUES ('thread','seat','invited');\
            INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES ('invite','thread','seat',1,'pending',0,100000000,100000000,1);\
        ").unwrap();
        // A prior wake already happened (ladder at step 1, 60 s effective
        // delay, no recorded frontier), so the next
        // reservation climbs exactly one rung; the first-ever one stays at 0.
        db.execute("INSERT INTO wake_work(seat_id,reason_bits,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot) VALUES ('seat',1,1,30000,60000,'prior',?1)", [old_daemon_boot().to_string()]).unwrap();
        drop(db);
        clock.mono.store(START_MS, Ordering::SeqCst);
        let host = Arc::new(SeamHost {
            clock: clock.clone(),
            refuse: AtomicBool::new(false),
            observes: AtomicU64::new(0),
            prompts: AtomicU64::new(0),
            submit_keys: AtomicU64::new(0),
            pane_states: Mutex::new(states.into()),
        });
        Self {
            path,
            context,
            clock,
            host,
        }
    }

    /// (retry_step, effective_delay_ms, last_outcome, last_reservation_id) of the seat.
    fn ladder(&self) -> (i64, i64, Option<String>, Option<String>) {
        let db = self.context.open_writer().unwrap();
        db.query_row(
            "SELECT retry_step,effective_delay_ms,last_outcome,last_reservation_id FROM wake_work WHERE seat_id='seat'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    }

    /// Run `body` with a scheduler over the real store and this fake host.
    fn run<T>(&self, body: impl FnOnce(&dyn Fn() -> WakeDriveOutcome) -> T) -> T {
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(self.path.clone(), self.clock.clone()),
                "i",
                StoreSettings {
                    daemon_boot: Some(daemon_boot()),
                    wake_batch_delay_ms: 0,
                    ..StoreSettings::default()
                },
            )
            .unwrap(),
        );
        let ports = ScheduledStore::new(store, Arc::new(FairWriter::new(32)));
        let dispatch = NativeWakeDispatcher::new(self.host.as_ref(), &ports, self.clock.as_ref());
        let due = FakeDeadlinePort {
            clock: Arc::new(FakeClock(AtomicU64::new(0))),
            due_calls: AtomicU64::new(0),
        };
        let scheduler = Scheduler::new(
            "i".into(),
            &due,
            &ports,
            &dispatch,
            RetryConfig::default(),
            daemon_boot(),
        );
        let budget = CallBudget {
            deadline: MonoInstant(10_000_000),
            cancellation: Cancellation::default(),
        };
        // One drive scans one bounded page; keep driving until it settles or
        // an attempt happens, so a test sees the attempt's own outcome.
        let drive = || {
            let mut last = scheduler.drive_wakes(&budget).unwrap();
            for _ in 0..5 {
                if last.attempted > 0 || !last.has_more {
                    break;
                }
                last = scheduler.drive_wakes(&budget).unwrap();
            }
            last
        };
        body(&drive)
    }
}
impl Drop for Seam {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn refused_pre_send_backs_off_without_advancing_the_ladder() {
    let seam = Seam::new(vec![]);
    seam.host.refuse.store(true, Ordering::SeqCst);
    seam.run(|drive| {
        seam.clock.mono.store(FIRST_DUE_MS, Ordering::SeqCst);
        let before = seam.ladder();
        let refused = drive();
        assert_eq!(refused.attempted, 1);
        assert_eq!(seam.host.sends(), 0, "a refusal sends nothing");
        assert!(
            refused.verification.is_empty(),
            "a refusal sent nothing to verify: {:?}",
            refused.verification
        );
        let after = seam.ladder();
        assert_eq!(after.0, before.0, "retry_step must not move on a refusal");
        assert_eq!(
            after.1, before.1,
            "effective delay must not move on a refusal"
        );
        assert_eq!(
            after.3, before.3,
            "the fenced restore must put the prior reservation back"
        );
        let due = refused
            .next_due_at
            .expect("refusal backoff schedules a retry");
        assert!(
            due.0 > FIRST_DUE_MS,
            "retry is later than the refusal instant"
        );

        // Before the refusal's next_due_at the seat is not retried.
        let observes = seam.host.observes.load(Ordering::SeqCst);
        seam.clock.mono.store(due.0 - 1, Ordering::SeqCst);
        assert_eq!(drive().attempted, 0);
        assert_eq!(seam.host.observes.load(Ordering::SeqCst), observes);

        // At next_due_at the host has recovered: the attempt goes through.
        seam.host.refuse.store(false, Ordering::SeqCst);
        seam.clock.mono.store(due.0, Ordering::SeqCst);
        let retried = drive();
        assert_eq!(retried.attempted, 1);
        assert_eq!(seam.host.prompts.load(Ordering::SeqCst), 1);
        assert_eq!(seam.ladder().0, before.0 + 1);
    });
}

#[test]
fn sent_and_verified_advances_one_step() {
    let seam = Seam::new(vec![AgentComposerState::Submitted]);
    seam.run(|drive| {
        seam.clock.mono.store(FIRST_DUE_MS, Ordering::SeqCst);
        let before = seam.ladder();
        let outcome = drive();
        assert_eq!(outcome.attempted, 1);
        assert_eq!(
            outcome.verification,
            vec![(SeatId::new("seat"), SubmissionVerification::Verified)]
        );
        assert_eq!(seam.host.prompts.load(Ordering::SeqCst), 1);
        assert_eq!(seam.host.submit_keys.load(Ordering::SeqCst), 0);
        let after = seam.ladder();
        assert_eq!(after.0, before.0 + 1, "exactly one ladder step");
        assert_eq!(after.2.as_deref(), Some("submitted"));
    });
}

#[test]
fn unsubmitted_after_retry_reports_unsubmitted_and_advances_exactly_one_step() {
    let seam = Seam::new(vec![
        AgentComposerState::HoldingPrompt,
        AgentComposerState::HoldingPrompt,
    ]);
    seam.run(|drive| {
        seam.clock.mono.store(FIRST_DUE_MS, Ordering::SeqCst);
        let before = seam.ladder();
        let outcome = drive();
        assert_eq!(outcome.attempted, 1);
        assert_eq!(
            outcome.verification,
            vec![(SeatId::new("seat"), SubmissionVerification::Unsubmitted)]
        );
        let after = seam.ladder();
        // The durable string stays the existing OutcomeUnknown disposition
        // (inline literal in src/store/wake.rs; no constant exists).
        assert_eq!(after.2.as_deref(), Some("outcome_unknown"));
        assert_eq!(after.0, before.0 + 1, "exactly one ladder step");
        assert_eq!(seam.host.prompts.load(Ordering::SeqCst), 1);
        assert_eq!(
            seam.host.submit_keys.load(Ordering::SeqCst),
            1,
            "single submit-key retry"
        );

        // Within the ladder delay nothing is sent again.
        let sends = seam.host.sends();
        for at in [1, 1_000, 29_000] {
            seam.clock.mono.store(FIRST_DUE_MS + at, Ordering::SeqCst);
            assert_eq!(drive().attempted, 0);
        }
        assert_eq!(seam.host.sends(), sends, "no re-send loop");
        assert_eq!(seam.ladder().0, after.0);
    });
}
