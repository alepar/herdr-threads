//! ht-p03.51: the root integration sweep for the remaining herdr-threads
//! findings. Cross-bucket paths no per-seam bead owns, driven through the
//! production elected daemon on a private named Herdr session (the
//! `lane_wiring::Session` fixture, never the shared server) and the built
//! `herdr-threads` executable:
//!
//! - a send's commit reaching a wake attempt within 100 ms with all five lanes
//!   registered, with and without a large settled history (B1 x B4);
//! - retention draining a backlog in bounded batches while the other lanes
//!   stay idle (B4 x B1);
//! - every lane failing at once surfacing through `remedy()`/Health within the
//!   Health line budget (B2 x B3);
//! - startup failure, lane failure and version skew each reaching the operator
//!   as their own `remedy()` text, a skewed daemon never showing the lane
//!   pointer (B3 x B6).
//!
//! The existing per-seam tests that cover the remaining flows are listed with
//! their outcomes in `docs/history/remaining-findings-run/integration-sweep-notes.md`.
use crate::lane_wiring::{
    Session, kicks_since, run_idle_window, settle_setup, wait_lanes_settled, wait_until,
};
use crate::lanes_latency;
use herdr_threads::{
    daemon::{
        health::HEALTH_LINE_BUDGET,
        paths::{InstancePaths, RuntimeContext},
        remedy::{RemedyContext, remedy},
    },
    protocol::{
        commands::{Command, HOOK_PARSE_DETAIL_BYTES, HookParseFailure, bounded_hook_detail},
        results::{ErrorClass, ErrorCode},
        wire::PROTOCOL_VERSION,
    },
    service::kicks::Lane,
    store::retention::RETENTION_BATCH_ROWS,
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::{
    fs,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// A sender and `recipients` recipients, each in a thread of its own, on the
/// `lanes_latency` fixture (a recipient has had no wake attention yet, so the
/// first wake it receives is the one the test provokes; a second send to the
/// same plain-shell seat would sit on the 30 s ladder, so each measured send
/// goes to a fresh recipient), the lanes quiet.
struct Scene(lanes_latency::Scene);

impl Scene {
    fn new(case: &str, recipients: usize) -> Option<Self> {
        let scene = lanes_latency::Scene::new(case, recipients)?;
        keep_panics_visible();
        wait_lanes_settled_latency(&scene.session);
        Some(Self(scene))
    }

    fn session(&self) -> &lanes_latency::Session {
        &self.0.session
    }

    /// Measures from the send's first request commit to the wake producer
    /// after writer release, before host handling. Its bounded read proves the
    /// first canonical active reservation, with full physical proof required
    /// finally for the same publication. The endpoint precedes the read and excludes
    /// observer scheduling. Each recipient must be fresh.
    fn send_to_wake_attempt(&self, recipient: usize, body: &str) -> Duration {
        let s = self.session();
        s.wait_commits_quiet("wake", Duration::from_millis(1500));
        let (caller, thread) = &self.0.recipients[recipient];
        let db = s.db();
        let target = WakeTarget::fresh(&db, &caller.seat, thread, &self.0.sender.seat, body);
        let db = rusqlite::Connection::open_with_flags(
            db.path().expect("isolated database path"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.busy_timeout(Duration::from_millis(50)).unwrap();
        let db = Mutex::new(db);
        let attempted = Arc::new(Mutex::new(AttemptTrace::new()));
        let slot = attempted.clone();
        let after_seq = target.after_seq;
        let observer = s
            .probe
            .observe_lane_commits(
                Lane::Wakes,
                Box::new(move |at| {
                    let entered = Instant::now();
                    let mut trace = slot.lock().unwrap();
                    let queried = trace.attempt.is_none();
                    let mut matched = false;
                    let mut snapshot = None;
                    if queried {
                        match target.read_snapshot(&db.lock().unwrap()) {
                            Ok(observed) => {
                                if trace.first.is_none() {
                                    trace.first = observed.canonical_attempt(at);
                                }
                                if let Some(attempt) = observed.attempt(at) {
                                    matched = true;
                                    trace.attempt = Some(Ok(attempt));
                                }
                                snapshot = Some(observed);
                            }
                            Err(error) => trace.attempt = Some(Err(error)),
                        }
                    }
                    let ended = Instant::now();
                    if trace.samples.len() < TRACE_CAP {
                        trace.samples.push(CallbackSample {
                            at,
                            entered,
                            ended,
                            queried,
                            matched,
                            snapshot,
                        });
                    } else {
                        trace.overflow = true;
                    }
                }),
            )
            .expect("exclusive measured wake observer");
        let diagnostics = AttemptDiagnostics {
            observer: Some(observer),
            trace: attempted.clone(),
            probe: s.probe.clone(),
            seat: caller.seat.clone(),
            body: body.into(),
            thread: thread.clone(),
            after_seq,
            bridge_before: SystemTime::now(),
            bridge: (Instant::now(), SystemTime::now()),
        };
        // Inspect the actual fixture command builder without invoking a child.
        let mut command = s.herdr.command(env!("CARGO_BIN_EXE_herdr-threads"));
        herdr_threads::test_support::spawn::tag(&mut command);
        eprintln!(
            "SWEEP_COMMAND_BUILDER_ENV (construction-only, actual CLI child not observed here) pid={} scale={:?}",
            std::process::id(),
            command
                .get_envs()
                .find(|(key, _)| *key == herdr_threads::protocol::time::TEST_TIMEOUT_SCALE_ENV)
        );
        let sent_after = Instant::now();
        let sent = self.0.send(recipient, body, &[]);
        let message = sent.as_str().expect("sent message ID");
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let kicks: Vec<_> = s
                .probe
                .kick_log()
                .into_iter()
                .filter(|(_, _, at)| *at >= sent_after)
                .collect();
            let latency = {
                let trace = attempted.lock().unwrap();
                trace.attempt.as_ref().and_then(|attempt| {
                    let attempt = attempt
                        .as_ref()
                        .unwrap_or_else(|error| panic!("target proof read: {error}"));
                    fenced_first_send_measurement(
                        &kicks,
                        trace.first.as_ref(),
                        Some(attempt),
                        message,
                    )
                })
            };
            if let Some(latency) = latency {
                drop(diagnostics);
                // Settle this round's attention. An unacknowledged receipt
                // keeps a refused wake on its retry ladder, and a retry's host
                // call would hold the single wake lane during a later round's
                // measurement (seen under suite load as Refused(Unsafe)).
                s.ok(Some(caller), &["ack", message]);
                return latency;
            }
            assert!(
                Instant::now() < until,
                "no correlated recipient attempt and materialization kick within 3 s: recipient {}, message {message}, proof {:?}, kicks {kicks:?}",
                caller.seat,
                attempted.lock().unwrap().attempt
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

const TRACE_CAP: usize = 64;

#[derive(Debug)]
struct CallbackSample {
    at: Instant,
    entered: Instant,
    ended: Instant,
    queried: bool,
    matched: bool,
    snapshot: Option<WakeSnapshot>,
}

struct AttemptTrace {
    attempt: Option<Result<WakeAttempt, rusqlite::Error>>,
    first: Option<CanonicalAttempt>,
    samples: Vec<CallbackSample>,
    overflow: bool,
}
impl AttemptTrace {
    fn new() -> Self {
        Self {
            attempt: None,
            first: None,
            samples: Vec::with_capacity(TRACE_CAP),
            overflow: false,
        }
    }
}

struct AttemptDiagnostics {
    observer: Option<herdr_threads::app::LaneCommitObserverGuard>,
    trace: Arc<Mutex<AttemptTrace>>,
    probe: herdr_threads::app::LaneProbe,
    seat: String,
    body: String,
    thread: String,
    after_seq: i64,
    bridge_before: SystemTime,
    bridge: (Instant, SystemTime),
}
impl Drop for AttemptDiagnostics {
    fn drop(&mut self) {
        drop(self.observer.take());
        let trace = self.trace.lock().unwrap_or_else(|e| e.into_inner());
        let kicks: Vec<_> = self
            .probe
            .kick_log()
            .into_iter()
            .filter(|(_, _, at)| *at >= self.bridge.0)
            .collect();
        let stages: Vec<_> = trace
            .samples
            .iter()
            .map(|sample| {
                (
                    sample.at,
                    sample.entered,
                    sample.ended,
                    sample.entered.saturating_duration_since(sample.at),
                    sample.ended.saturating_duration_since(sample.entered),
                    sample.queried,
                    sample.matched,
                    &sample.snapshot,
                )
            })
            .collect();
        let report = format!(
            "SWEEP_TRACE pid={} seat={} body={:?} thread={} cutoff={} bridge_utc_before={:?} bridge={:?} first_canonical={:?} final_full30={:?} overflow={} stages=(producer_at, callback_enter, read_end_including_slot_and_db_lock, dispatcher_delay, callback_elapsed, queried, exact_match, raw_snapshot) {:?} kicks={:?}\n",
            std::process::id(),
            self.seat,
            self.body,
            self.thread,
            self.after_seq,
            self.bridge_before,
            self.bridge,
            trace.first.as_ref().map(|first| (
                &first.identity,
                first.producer_at,
                &first.reservation
            )),
            trace.attempt,
            trace.overflow,
            stages,
            kicks
        );
        eprint!("{report}");
        if let Some(dir) = std::env::var_os("HT_FOUR_DIAGNOSTICS_DIR") {
            let path = std::path::PathBuf::from(dir).join(format!(
                "sweep-{}-{}.log",
                std::process::id(),
                self.bridge.1.duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            if let Err(error) = fs::write(path, report) {
                eprintln!("wake diagnostics were not flushed: {error}");
                assert!(
                    std::thread::panicking(),
                    "failed to persist wake diagnostic trace"
                );
            }
        }
        assert!(
            !trace.overflow || std::thread::panicking(),
            "wake diagnostic exceeded its 64-callback cap"
        );
    }
}

#[derive(Debug, Clone)]
struct WakeAttempt {
    message: String,
    // Captured on the producer after writer release, before exact-target read.
    proved_at: Instant,
    identity: Option<AttemptIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttemptIdentity {
    message: String,
    preparation: String,
    job: String,
    generation: i64,
    harness: String,
    wake_generation: i64,
    daemon_boot: String,
    receipt_seq: i64,
    receipt_offset: i64,
}
#[derive(Debug, Clone)]
struct CanonicalAttempt {
    identity: AttemptIdentity,
    producer_at: Instant,
    reservation: String,
}

// Verbatim pre-projection proof query: differential oracle, never called by observer.
const ORIGINAL_WAKE_PROOF_SQL: &str = r#"SELECT m.id FROM messages m
             JOIN send_manifests sm ON sm.message_id=m.id
             JOIN prepared_recipients r ON r.preparation_id=sm.preparation_id AND r.seat_id=?1
             JOIN work_jobs j ON j.subject_id=sm.preparation_id AND j.kind='send_attention' AND j.status='complete'
             JOIN receipt_state rs ON rs.message_id=m.id AND rs.seat_id=r.seat_id
             JOIN occupant_bindings b ON b.seat_id=r.seat_id AND b.ended_at IS NULL AND b.harness!='human'
             JOIN seats s ON s.id=b.seat_id AND s.generation=b.generation AND s.state='resolved'
             JOIN wake_work w ON w.seat_id=r.seat_id AND w.binding_generation=b.generation
             WHERE m.thread_id=?2 AND m.actor_seat_id=?3 AND m.body=?4 AND m.decision_seq>?5
               AND m.kind='ordinary' AND m.delivery_mode='ordinary'
               AND sm.recipient_count=1 AND sm.warning_count=0
               AND r.ack_required=1 AND r.eligible_at_snapshot=1 AND r.frozen_duration_ms=300000
               AND rs.state='pending' AND rs.ack_required=1 AND rs.acked_at IS NULL
               AND w.reservation_id IS NOT NULL AND w.reservation_boot IS NOT NULL
               AND w.last_receipt_seq=m.decision_seq AND w.last_receipt_offset=m.event_offset
               AND (SELECT count(*) FROM prepared_recipients pr JOIN send_manifests pm ON pm.preparation_id=pr.preparation_id WHERE pr.seat_id=?1)=1
               AND (SELECT count(*) FROM receipt_state WHERE seat_id=?1 AND state='pending')=1
               AND NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=?1)
               AND NOT EXISTS(SELECT 1 FROM digest_open_warning_recipients WHERE seat_id=?1)
               AND NOT EXISTS(SELECT 1 FROM digest_programmatic_warnings WHERE seat_id=?1)"#;

const WAKE_PROJECTION_SQL: &str = r#"WITH candidates AS MATERIALIZED (
    SELECT id,decision_seq,event_offset,kind,delivery_mode FROM messages
    WHERE thread_id=?2 AND actor_seat_id=?3 AND body=?4 AND decision_seq>?5
    LIMIT 2
), projection AS MATERIALIZED (
    SELECT m.id AS message,
           m.decision_seq AS decision_seq,
           m.event_offset AS event_offset,
           sm.preparation_id AS preparation,
           r.ack_required AS recipient_ack,
           r.eligible_at_snapshot AS eligible,
           r.frozen_duration_ms AS frozen_duration_ms,
           j.id AS job,
           j.status AS job_status,
           rs.state AS receipt_state,
           rs.ack_required AS receipt_ack,
           rs.acked_at AS acked_at,
           b.generation AS binding_generation,
           b.harness AS harness,
           s.generation AS seat_generation,
           s.state AS seat_state,
           w.binding_generation AS wake_generation,
           w.reservation_id AS reservation,
           w.reservation_boot AS reservation_boot,
           w.last_receipt_seq AS receipt_seq,
           w.last_receipt_offset AS receipt_offset,
           m.kind='ordinary' AS p0,
           m.delivery_mode='ordinary' AS p1,
           sm.preparation_id IS NOT NULL AS p2,
           sm.recipient_count=1 AS p3,
           sm.warning_count=0 AS p4,
           r.preparation_id IS NOT NULL AS p5,
           r.ack_required=1 AS p6,
           r.eligible_at_snapshot=1 AS p7,
           r.frozen_duration_ms=300000 AS p8,
           j.status='complete' AS p9,
           rs.message_id IS NOT NULL AS p10,
           rs.state='pending' AS p11,
           rs.ack_required=1 AS p12,
           CASE WHEN rs.message_id IS NULL THEN NULL ELSE rs.acked_at IS NULL END AS p13,
           b.seat_id IS NOT NULL AS p14,
           b.harness!='human' AS p15,
           s.id IS NOT NULL AS p16,
           s.generation=b.generation AS p17,
           s.state='resolved' AS p18,
           w.seat_id IS NOT NULL AS p19,
           w.binding_generation=b.generation AS p20,
           CASE WHEN w.seat_id IS NULL THEN NULL ELSE w.reservation_id IS NOT NULL END AS p21,
           CASE WHEN w.seat_id IS NULL THEN NULL ELSE w.reservation_boot IS NOT NULL END AS p22,
           w.last_receipt_seq=m.decision_seq AS p23,
           w.last_receipt_offset=m.event_offset AS p24,
           (SELECT count(*) FROM prepared_recipients pr JOIN send_manifests pm ON pm.preparation_id=pr.preparation_id WHERE pr.seat_id=?1)=1 AS p25,
           (SELECT count(*) FROM receipt_state WHERE seat_id=?1 AND state='pending') AS p26,
           NOT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=?1) AS p27,
           NOT EXISTS(SELECT 1 FROM digest_open_warning_recipients WHERE seat_id=?1) AS p28,
           NOT EXISTS(SELECT 1 FROM digest_programmatic_warnings WHERE seat_id=?1) AS p29,
           r.availability_provenance AS recipient_provenance,
           hw.through_decision_seq AS human_waiver_cutoff,
           s.retired_at AS retired_at
    FROM candidates m
    LEFT JOIN send_manifests sm ON sm.message_id=m.id
    LEFT JOIN prepared_recipients r ON r.preparation_id=sm.preparation_id AND r.seat_id=?1
    LEFT JOIN work_jobs j ON j.subject_id=sm.preparation_id AND j.kind='send_attention'
    LEFT JOIN receipt_state rs ON rs.message_id=m.id AND rs.seat_id=r.seat_id
    LEFT JOIN occupant_bindings b ON b.seat_id=?1 AND b.ended_at IS NULL
    LEFT JOIN seats s ON s.id=b.seat_id
    LEFT JOIN wake_work w ON w.seat_id=?1
    LEFT JOIN human_receipt_waivers hw ON hw.seat_id=?1
)
SELECT *, COALESCE(p0 AND p1 AND p2 AND p3 AND p4 AND p5 AND p6 AND p7 AND p8 AND p9 AND p10 AND p11 AND p12 AND p13 AND p14 AND p15 AND p16 AND p17 AND p18 AND p19 AND p20 AND p21 AND p22 AND p23 AND p24 AND p25 AND p26=1 AND p27 AND p28 AND p29,0) AS accepted FROM projection"#;

#[derive(Debug, Clone, Copy)]
#[repr(usize)]
enum WakePredicate {
    Ordinary,
    OrdinaryDelivery,
    ManifestPresent,
    OneRecipient,
    NoManifestWarnings,
    FrozenRecipientPresent,
    FrozenAck,
    Eligible,
    FrozenDuration,
    Materialized,
    PhysicalReceiptPresent,
    PendingReceipt,
    ReceiptAck,
    Unacked,
    CurrentBindingPresent,
    AgentBinding,
    SeatPresent,
    CanonicalGeneration,
    ResolvedSeat,
    WakePresent,
    WakeGeneration,
    ActiveReservation,
    ReservationBoot,
    ReceiptSequence,
    ReceiptOffset,
    SolePublication,
    SolePendingReceipt,
    NoInvitation,
    NoOpenWarning,
    NoProgrammaticWarning,
}
const WAKE_PREDICATES: [WakePredicate; 30] = [
    WakePredicate::Ordinary,
    WakePredicate::OrdinaryDelivery,
    WakePredicate::ManifestPresent,
    WakePredicate::OneRecipient,
    WakePredicate::NoManifestWarnings,
    WakePredicate::FrozenRecipientPresent,
    WakePredicate::FrozenAck,
    WakePredicate::Eligible,
    WakePredicate::FrozenDuration,
    WakePredicate::Materialized,
    WakePredicate::PhysicalReceiptPresent,
    WakePredicate::PendingReceipt,
    WakePredicate::ReceiptAck,
    WakePredicate::Unacked,
    WakePredicate::CurrentBindingPresent,
    WakePredicate::AgentBinding,
    WakePredicate::SeatPresent,
    WakePredicate::CanonicalGeneration,
    WakePredicate::ResolvedSeat,
    WakePredicate::WakePresent,
    WakePredicate::WakeGeneration,
    WakePredicate::ActiveReservation,
    WakePredicate::ReservationBoot,
    WakePredicate::ReceiptSequence,
    WakePredicate::ReceiptOffset,
    WakePredicate::SolePublication,
    WakePredicate::SolePendingReceipt,
    WakePredicate::NoInvitation,
    WakePredicate::NoOpenWarning,
    WakePredicate::NoProgrammaticWarning,
];

#[derive(Clone)]
struct WakeProjection {
    message: String,
    decision_seq: i64,
    event_offset: i64,
    preparation: Option<String>,
    recipient_ack: Option<bool>,
    eligible: Option<bool>,
    frozen_duration_ms: Option<i64>,
    job: Option<String>,
    job_status: Option<String>,
    receipt_state: Option<String>,
    receipt_ack: Option<bool>,
    acked_at: Option<i64>,
    binding_generation: Option<i64>,
    harness: Option<String>,
    seat_generation: Option<i64>,
    seat_state: Option<String>,
    wake_generation: Option<i64>,
    reservation: Option<String>,
    reservation_boot: Option<String>,
    receipt_seq: Option<i64>,
    receipt_offset: Option<i64>,
    predicates: [Option<bool>; 30],
    accepted: bool,
    pending_receipts: i64,
    recipient_provenance: Option<String>,
    human_waiver_cutoff: Option<i64>,
    retired_at: Option<i64>,
}
impl std::fmt::Debug for WakeProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let predicates: Vec<_> = WAKE_PREDICATES.iter().zip(self.predicates).collect();
        f.debug_struct("WakeProjection")
            .field("message", &self.message)
            .field("decision_seq", &self.decision_seq)
            .field("event_offset", &self.event_offset)
            .field("preparation", &self.preparation)
            .field("recipient_ack", &self.recipient_ack)
            .field("eligible", &self.eligible)
            .field("frozen_duration_ms", &self.frozen_duration_ms)
            .field("job", &self.job)
            .field("job_status", &self.job_status)
            .field("receipt_state", &self.receipt_state)
            .field("receipt_ack", &self.receipt_ack)
            .field("acked_at", &self.acked_at)
            .field("binding_generation", &self.binding_generation)
            .field("harness", &self.harness)
            .field("seat_generation", &self.seat_generation)
            .field("seat_state", &self.seat_state)
            .field("wake_generation", &self.wake_generation)
            .field("reservation", &self.reservation)
            .field("reservation_boot", &self.reservation_boot)
            .field("receipt_seq", &self.receipt_seq)
            .field("receipt_offset", &self.receipt_offset)
            .field("accepted", &self.accepted)
            .field("pending_receipts", &self.pending_receipts)
            .field("recipient_provenance", &self.recipient_provenance)
            .field("human_waiver_cutoff", &self.human_waiver_cutoff)
            .field("retired_at", &self.retired_at)
            .field("predicates", &predicates)
            .finish()
    }
}

impl WakeProjection {
    // Evidence of the first canonical reservation. Physical projection and
    // job completion remain mandatory at the separate FINAL strong barrier.
    fn canonical_attempt(&self, producer_at: Instant) -> Option<CanonicalAttempt> {
        for predicate in WAKE_PREDICATES {
            if matches!(
                predicate,
                WakePredicate::Materialized
                    | WakePredicate::PhysicalReceiptPresent
                    | WakePredicate::PendingReceipt
                    | WakePredicate::ReceiptAck
                    | WakePredicate::Unacked
                    | WakePredicate::SolePendingReceipt
            ) {
                continue;
            }
            if self.predicate(predicate) != Some(true) {
                return None;
            }
        }
        if self.recipient_provenance.as_deref() == Some("operator_human")
            || self
                .human_waiver_cutoff
                .is_some_and(|through| through >= self.decision_seq)
            || self.retired_at.is_some()
        {
            return None;
        }
        match self.predicate(WakePredicate::PhysicalReceiptPresent) {
            Some(true)
                if self.pending_receipts == 1
                    && self.predicate(WakePredicate::PendingReceipt) == Some(true)
                    && self.predicate(WakePredicate::ReceiptAck) == Some(true)
                    && self.predicate(WakePredicate::Unacked) == Some(true) => {}
            Some(false) if self.pending_receipts == 0 => {}
            _ => return None,
        }
        Some(CanonicalAttempt {
            identity: self.identity()?,
            producer_at,
            reservation: self.reservation.clone()?,
        })
    }
    fn identity(&self) -> Option<AttemptIdentity> {
        Some(AttemptIdentity {
            message: self.message.clone(),
            preparation: self.preparation.clone()?,
            job: self.job.clone()?,
            generation: self.binding_generation?,
            harness: self.harness.clone()?,
            wake_generation: self.wake_generation?,
            daemon_boot: self.reservation_boot.clone()?,
            receipt_seq: self.receipt_seq?,
            receipt_offset: self.receipt_offset?,
        })
    }
    fn predicate(&self, predicate: WakePredicate) -> Option<bool> {
        self.predicates[predicate as usize]
    }
}

#[derive(Debug, Clone)]
enum WakeSnapshot {
    MissingCandidate,
    Unique(Box<WakeProjection>),
    // Exactly two sampled candidates means AT LEAST two, not an exact total.
    Ambiguous(Box<[WakeProjection; 2]>),
}
impl WakeSnapshot {
    fn message(&self) -> Option<&str> {
        match self {
            Self::Unique(row) => Some(&row.message),
            Self::Ambiguous(rows) => {
                assert_eq!(rows.len(), 2);
                None
            }
            Self::MissingCandidate => None,
        }
    }
    fn attempt(&self, proved_at: Instant) -> Option<WakeAttempt> {
        match self {
            Self::Unique(row) if row.accepted => Some(WakeAttempt {
                message: row.message.clone(),
                proved_at,
                identity: row.identity(),
            }),
            _ => None,
        }
    }
    fn canonical_attempt(&self, at: Instant) -> Option<CanonicalAttempt> {
        self.unique()?.canonical_attempt(at)
    }

    fn unique(&self) -> Option<&WakeProjection> {
        match self {
            Self::Unique(row) => Some(row),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct WakeTarget {
    seat: String,
    thread: String,
    sender: String,
    body: String,
    after_seq: i64,
}

impl WakeTarget {
    fn fresh(
        db: &rusqlite::Connection,
        seat: &str,
        thread: &str,
        sender: &str,
        body: &str,
    ) -> Self {
        let competing: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM wake_work WHERE seat_id=?1 AND
                (reserved_at_utc IS NOT NULL OR last_reserved_at_utc IS NOT NULL OR completed_at_utc IS NOT NULL))
             OR EXISTS(SELECT 1 FROM prepared_recipients r JOIN send_manifests m ON m.preparation_id=r.preparation_id WHERE r.seat_id=?1)
             OR EXISTS(SELECT 1 FROM receipt_state WHERE seat_id=?1 AND state='pending')
             OR EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=?1)
             OR EXISTS(SELECT 1 FROM digest_open_warning_recipients WHERE seat_id=?1)
             OR EXISTS(SELECT 1 FROM digest_programmatic_warnings WHERE seat_id=?1)",
            [seat], |row| row.get(0)).unwrap();
        assert!(
            !competing,
            "measured recipient {seat} has prior attempts or competing attention"
        );
        let after_seq = db.query_row("SELECT decision_seq FROM host_instances WHERE id=(SELECT instance_id FROM seats WHERE id=?1)", [seat], |row| row.get(0)).unwrap();
        Self {
            seat: seat.into(),
            thread: thread.into(),
            sender: sender.into(),
            body: body.into(),
            after_seq,
        }
    }

    fn read_snapshot(&self, db: &rusqlite::Connection) -> rusqlite::Result<WakeSnapshot> {
        // One statement/snapshot retains every original conjunct. Unique
        // indexed joins cannot multiply candidate rows; two candidates reject.
        let mut statement = db.prepare(WAKE_PROJECTION_SQL)?;
        let rows = statement
            .query_map(
                rusqlite::params![
                    self.seat,
                    self.thread,
                    self.sender,
                    self.body,
                    self.after_seq
                ],
                |row| {
                    let mut predicates = [None; 30];
                    for (index, predicate) in predicates.iter_mut().enumerate() {
                        *predicate = if index == WakePredicate::SolePendingReceipt as usize {
                            Some(row.get::<_, i64>(47)? == 1)
                        } else {
                            row.get(21 + index)?
                        };
                    }
                    Ok(WakeProjection {
                        message: row.get(0)?,
                        decision_seq: row.get(1)?,
                        event_offset: row.get(2)?,
                        preparation: row.get(3)?,
                        recipient_ack: row.get(4)?,
                        eligible: row.get(5)?,
                        frozen_duration_ms: row.get(6)?,
                        job: row.get(7)?,
                        job_status: row.get(8)?,
                        receipt_state: row.get(9)?,
                        receipt_ack: row.get(10)?,
                        acked_at: row.get(11)?,
                        binding_generation: row.get(12)?,
                        harness: row.get(13)?,
                        seat_generation: row.get(14)?,
                        seat_state: row.get(15)?,
                        wake_generation: row.get(16)?,
                        reservation: row.get(17)?,
                        reservation_boot: row.get(18)?,
                        receipt_seq: row.get(19)?,
                        receipt_offset: row.get(20)?,
                        predicates,
                        pending_receipts: row.get(47)?,
                        recipient_provenance: row.get(51)?,
                        human_waiver_cutoff: row.get(52)?,
                        retired_at: row.get(53)?,
                        accepted: row.get(54)?,
                    })
                },
            )?
            .take(3)
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(match rows.len() {
            0 => WakeSnapshot::MissingCandidate,
            1 => WakeSnapshot::Unique(Box::new(rows.into_iter().next().unwrap())),
            2 => WakeSnapshot::Ambiguous(Box::new(rows.try_into().unwrap())),
            _ => return Err(rusqlite::Error::InvalidQuery),
        })
    }

    fn read_attempt(
        &self,
        db: &rusqlite::Connection,
        proved_at: Instant,
    ) -> rusqlite::Result<Option<WakeAttempt>> {
        Ok(self.read_snapshot(db)?.attempt(proved_at))
    }

    fn read_original_attempt(&self, db: &rusqlite::Connection) -> rusqlite::Result<Option<String>> {
        use rusqlite::OptionalExtension;
        db.query_row(
            ORIGINAL_WAKE_PROOF_SQL,
            rusqlite::params![
                self.seat,
                self.thread,
                self.sender,
                self.body,
                self.after_seq
            ],
            |row| row.get(0),
        )
        .optional()
    }
}

// Full30 evidence is FINAL; it must identify the same canonical publication
// as the first producer record. A refused retry may have another reservation
// ID. Never claim the physical/full proof already held at the first endpoint.
fn fenced_first_send_measurement(
    kicks: &[herdr_threads::app::KickRecord],
    first: Option<&CanonicalAttempt>,
    final_proof: Option<&WakeAttempt>,
    message: &str,
) -> Option<Duration> {
    let final_proof = final_proof?;
    fenced_send_measurement(kicks, Some(final_proof), message)?;
    let first = first?;
    if final_proof.identity.as_ref() != Some(&first.identity)
        || first.producer_at > final_proof.proved_at
    {
        return None;
    }
    // Anchor on the publication commit: the foreground commit that schedules
    // send_attention on Deadlines. Hidden send preparation also updates archival
    // fencing in an earlier foreground commit (an Archival-only kick); timing
    // from that one would add request processing between the two commits
    // (2-5 ms alone, 139 ms seen under suite load) to commit-to-wake.
    let (_, _, anchor) = kicks
        .iter()
        .find(|(lanes, origin, _)| origin.is_none() && lanes.contains(Lane::Deadlines))?;
    if first.producer_at < *anchor
        || *anchor > final_proof.proved_at
        || !kicks.iter().any(|(lanes, origin, at)| {
            *origin == Some(Lane::Deadlines)
                && lanes.contains(Lane::Wakes)
                && *at <= final_proof.proved_at
        })
    {
        return None;
    }
    Some(first.producer_at.duration_since(*anchor))
}

fn fenced_send_measurement(
    kicks: &[herdr_threads::app::KickRecord],
    attempted: Option<&WakeAttempt>,
    message: &str,
) -> Option<Duration> {
    let attempted = attempted?;
    assert_eq!(
        attempted.message, message,
        "producer proof must identify the returned send"
    );
    // Publication schedules send_attention on Deadlines. Its writer's kick
    // must be flushed, as must the materializer's Deadlines -> Wakes kick.
    if !kicks
        .iter()
        .any(|(lanes, origin, _)| origin.is_none() && lanes.contains(Lane::Deadlines))
        || !kicks.iter().any(|(lanes, origin, _)| {
            *origin == Some(Lane::Deadlines) && lanes.contains(Lane::Wakes)
        })
    {
        return None;
    }
    let (_, _, committed_at) = kicks.iter().find(|(_, origin, _)| origin.is_none())?;
    // The preparation commit may precede publication: retaining the original
    // earliest-request anchor makes the unchanged 100 ms check stricter.
    Some(attempted.proved_at.saturating_duration_since(*committed_at))
}

// Contract-only literal timeline, not a fabricated preparation commit.
#[test]
fn measurement_retains_earliest_flushed_request_anchor() {
    use herdr_threads::service::kicks::LaneSet;
    let start = Instant::now();
    let deadlines = LaneSet::EMPTY.with(Lane::Deadlines);
    let wakes = LaneSet::EMPTY.with(Lane::Wakes);
    let kicks = [
        (deadlines, None, start),
        (deadlines, None, start + Duration::from_millis(90)),
        (
            wakes,
            Some(Lane::Deadlines),
            start + Duration::from_millis(95),
        ),
    ];
    let attempt = WakeAttempt {
        message: "literal-target".into(),
        proved_at: start + Duration::from_millis(120),
        identity: None, // Contract-only literal, not canonical producer evidence.
    };
    let measured = fenced_send_measurement(&kicks, Some(&attempt), "literal-target").unwrap();
    assert_eq!(measured, Duration::from_millis(120));
    assert!(
        std::panic::catch_unwind(|| assert!(measured < Duration::from_millis(100))).is_err(),
        "selecting the later90ms request would incorrectly accept30ms"
    );
}

// Ordering control: real ordinary publication/materialization, no daemon or host.
#[test]
fn retention_observer_waits_for_correlated_materialization_kick() {
    producer_ordering_control(false, false, None);
}

#[test]
fn logical_reservation_before_projection_is_not_a_completed_target_proof() {
    producer_ordering_control(true, false, None);
}

#[test]
fn wake_snapshot_preserves_logical_frontier_before_physical_projection() {
    producer_ordering_control(true, true, None);
}

fn assert_projection_plan(db: &rusqlite::Connection, target: &WakeTarget) {
    let sql = format!("EXPLAIN QUERY PLAN {WAKE_PROJECTION_SQL}");
    let mut statement = db.prepare(&sql).unwrap();
    let plan: Vec<String> = statement
        .query_map(
            rusqlite::params![
                target.seat,
                target.thread,
                target.sender,
                target.body,
                target.after_seq
            ],
            |row| row.get(3),
        )
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    println!("PROJECTION_QUERY_PLAN {plan:?}");
    for table in ["sm", "r", "j", "rs", "b", "s", "w", "hw"] {
        assert!(plan.iter().any(|line| line.starts_with(&format!("SEARCH {table} ")) && line.contains("INDEX")),
            "unique join {table} must use indexed exact keys: {plan:?}");
    }
    assert!(
        plan.iter()
            .any(|line| line.contains("SEARCH messages") && line.contains("INDEX")),
        "candidate selection must use indexed thread key"
    );
    // Original seat-global scalar subqueries remain unchanged, including
    // receipt-state count cost; do not mislabel those as new indexed probes.
}

// Pure identity/timeline contract negatives, supplied by the real two-stage
// fixture. Mutated keys are test inputs, never claimed runtime state changes.
fn first_final_negative_controls(
    snapshot: WakeSnapshot,
    at: Instant,
    final_proof: &WakeAttempt,
    kicks: &[herdr_threads::app::KickRecord],
    message: &str,
) {
    let first = snapshot.canonical_attempt(at).unwrap();
    assert!(fenced_first_send_measurement(kicks, None, Some(final_proof), message).is_none());
    assert!(fenced_first_send_measurement(kicks, Some(&first), None, message).is_none());
    for mutated in 0..10 {
        let mut wrong = first.clone();
        match mutated {
            0 => wrong.identity.message.push_str("other"),
            1 => wrong.identity.preparation.push_str("other"),
            2 => wrong.identity.job.push_str("other"),
            3 => wrong.identity.generation += 1,
            4 => wrong.identity.harness = "human".into(),
            5 => wrong.identity.wake_generation += 1,
            6 => wrong.identity.daemon_boot.push_str("other"),
            7 => wrong.identity.receipt_seq += 1,
            8 => wrong.identity.receipt_offset += 1,
            _ => wrong.producer_at = final_proof.proved_at + Duration::from_millis(1),
        }
        assert!(
            fenced_first_send_measurement(kicks, Some(&wrong), Some(final_proof), message)
                .is_none(),
            "identity/timeline mutation {mutated}"
        );
    }
    let no_publication: Vec<_> = kicks
        .iter()
        .copied()
        .filter(|(lanes, origin, _)| !(origin.is_none() && lanes.contains(Lane::Deadlines)))
        .collect();
    let no_materializer: Vec<_> = kicks
        .iter()
        .copied()
        .filter(|(lanes, origin, _)| {
            !(*origin == Some(Lane::Deadlines) && lanes.contains(Lane::Wakes))
        })
        .collect();
    for missing in [no_publication, no_materializer] {
        assert!(
            fenced_first_send_measurement(&missing, Some(&first), Some(final_proof), message)
                .is_none()
        );
    }
    let wrong_return = std::panic::catch_unwind(|| {
        fenced_first_send_measurement(
            kicks,
            Some(&first),
            Some(final_proof),
            "wrong-returned-message",
        )
    });
    assert!(wrong_return.is_err());
    let anchor = kicks
        .iter()
        .find(|(lanes, origin, _)| origin.is_none() && lanes.contains(Lane::Deadlines))
        .unwrap()
        .2;
    // An earlier foreground send-preparation commit that only updates archival
    // fencing is not the publication commit: it moves no endpoint.
    let mut early_foreground = vec![(
        herdr_threads::service::kicks::LaneSet::EMPTY.with(Lane::Archival),
        None,
        anchor - Duration::from_millis(150),
    )];
    early_foreground.extend_from_slice(kicks);
    assert_eq!(
        fenced_first_send_measurement(&early_foreground, Some(&first), Some(final_proof), message),
        fenced_first_send_measurement(kicks, Some(&first), Some(final_proof), message)
    );
    let mut before_anchor = first.clone();
    before_anchor.producer_at = anchor - Duration::from_millis(1);
    assert!(
        fenced_first_send_measurement(kicks, Some(&before_anchor), Some(final_proof), message)
            .is_none()
    );
    let mut after_final_kicks = kicks.to_vec();
    for (_, origin, at) in &mut after_final_kicks {
        if *origin == Some(Lane::Deadlines) {
            *at = final_proof.proved_at + Duration::from_millis(1);
        }
    }
    assert!(
        fenced_first_send_measurement(&after_final_kicks, Some(&first), Some(final_proof), message)
            .is_none()
    );
    // Logical first may precede the materializer fence; final producer barrier
    // is later. This literal ordering contract does not invent a real kick.
    let mut between = kicks.to_vec();
    for (lanes, origin, at) in &mut between {
        if *origin == Some(Lane::Deadlines) && lanes.contains(Lane::Wakes) {
            *at = first.producer_at + Duration::from_millis(1);
        }
    }
    assert_eq!(
        fenced_first_send_measurement(&between, Some(&first), Some(final_proof), message),
        Some(first.producer_at.duration_since(anchor))
    );
    let mut late = first.clone();
    late.producer_at = anchor + Duration::from_millis(150);
    let mut late_final = final_proof.clone();
    late_final.proved_at = late.producer_at;
    let actual_late =
        fenced_first_send_measurement(kicks, Some(&late), Some(&late_final), message).unwrap();
    assert_eq!(actual_late, Duration::from_millis(150));
    assert!(
        std::panic::catch_unwind(|| assert!(actual_late < Duration::from_millis(100))).is_err()
    );
    let mut delayed_final = final_proof.clone();
    delayed_final.proved_at += Duration::from_millis(150);
    assert_eq!(
        fenced_first_send_measurement(kicks, Some(&first), Some(&delayed_final), message),
        fenced_first_send_measurement(kicks, Some(&first), Some(final_proof), message),
        "later proof/consumer delay must not change first producer endpoint"
    );
    assert!(matches!(snapshot, WakeSnapshot::Unique(_)));
}

fn first_eligibility_boundary_controls(db: &rusqlite::Connection, target: &WakeTarget) {
    let before = target.read_snapshot(db).unwrap();
    let original = before.unique().unwrap();
    assert!(before.canonical_attempt(Instant::now()).is_some());
    // The genuine preparation above records cooperative_top_level. These
    // nullable/legacy provenance cases exercise the typed eligibility contract;
    // they do not claim that the current preparation API produced either value.
    for (label, provenance, expected) in [
        ("NULL provenance", None, true),
        ("human provenance", Some("operator_human".to_owned()), false),
    ] {
        let mut row = original.clone();
        row.recipient_provenance = provenance;
        assert_eq!(row.canonical_attempt(Instant::now()).is_some(), expected);
        println!("FIRST_ELIGIBILITY_TYPED_CONTRACT {label} expected={expected} {row:?}");
    }
    // SQLite forbids a resolved seat with a retirement marker. Independently
    // exercise this defensive classifier guard without disabling that CHECK.
    let mut malformed_retirement = original.clone();
    malformed_retirement.retired_at = Some(1);
    assert!(
        malformed_retirement
            .canonical_attempt(Instant::now())
            .is_none()
    );
    println!("FIRST_ELIGIBILITY_TYPED_CONTRACT malformed retired marker {malformed_retirement:?}");
    for (label, sql, expected) in [
        (
            "missing atomic job",
            "DELETE FROM work_jobs WHERE kind='send_attention'".to_owned(),
            false,
        ),
        (
            "retryable failed job",
            "UPDATE work_jobs SET status='failed' WHERE kind='send_attention'".to_owned(),
            true,
        ),
        (
            "older waiver",
            format!(
                "INSERT INTO human_receipt_waivers VALUES('b',{},1,0)",
                original.decision_seq - 1
            ),
            true,
        ),
        (
            "equal waiver",
            format!(
                "INSERT INTO human_receipt_waivers VALUES('b',{},1,0)",
                original.decision_seq
            ),
            false,
        ),
        (
            "newer waiver",
            format!(
                "INSERT INTO human_receipt_waivers VALUES('b',{},1,0)",
                original.decision_seq + 1
            ),
            false,
        ),
    ] {
        db.execute_batch("SAVEPOINT first_eligibility").unwrap();
        db.execute_batch(&sql).unwrap();
        let snapshot = target.read_snapshot(db).unwrap();
        assert_eq!(
            snapshot.canonical_attempt(Instant::now()).is_some(),
            expected,
            "first eligibility {label}: {snapshot:?}"
        );
        assert_eq!(
            snapshot
                .attempt(Instant::now())
                .as_ref()
                .map(|a| a.message.as_str()),
            target.read_original_attempt(db).unwrap().as_deref(),
            "final30 unchanged {label}"
        );
        println!("FIRST_ELIGIBILITY_BOUNDARY {label} expected={expected} {snapshot:?}");
        db.execute_batch("ROLLBACK TO first_eligibility; RELEASE first_eligibility")
            .unwrap();
    }
    // A different pending physical row must defeat an absent target row even
    // though the boolean old 'count=1' alone could not identify count0 versus2.
    db.execute_batch("SAVEPOINT physical_competition; DELETE FROM receipt_state WHERE seat_id='b'; INSERT INTO receipt_state(message_id,seat_id,state,ack_required) VALUES('other-message','b','pending',1)").unwrap();
    let competing = target.read_snapshot(db).unwrap();
    let row = competing.unique().unwrap();
    assert_eq!(row.pending_receipts, 1);
    assert_eq!(
        row.predicate(WakePredicate::PhysicalReceiptPresent),
        Some(false)
    );
    assert!(competing.canonical_attempt(Instant::now()).is_none());
    db.execute_batch("ROLLBACK TO physical_competition; RELEASE physical_competition")
        .unwrap();
}

fn projection_differential_controls(db: &rusqlite::Connection, target: &WakeTarget, message: &str) {
    let compare = |label: &str| {
        let snapshot = target.read_snapshot(db).unwrap();
        let original = target.read_original_attempt(db).unwrap();
        assert_eq!(
            snapshot
                .attempt(Instant::now())
                .as_ref()
                .map(|a| a.message.as_str()),
            original.as_deref(),
            "unique differential {label}: {snapshot:?}"
        );
        println!("PROJECTION_DIFFERENTIAL {label} {snapshot:?}");
        snapshot
    };
    assert!(
        compare("complete canonical target")
            .unique()
            .unwrap()
            .accepted
    );
    first_eligibility_boundary_controls(db, target);
    let mut unrelated = WakeTarget {
        seat: "c".into(),
        thread: target.thread.clone(),
        sender: target.sender.clone(),
        body: "no such target".into(),
        after_seq: target.after_seq,
    };
    assert!(matches!(
        unrelated.read_snapshot(db).unwrap(),
        WakeSnapshot::MissingCandidate
    ));
    unrelated.body = target.body.clone();
    let wrong_recipient = unrelated.read_snapshot(db).unwrap();
    assert_eq!(wrong_recipient.message(), Some(message));
    assert!(!wrong_recipient.unique().unwrap().accepted);
    assert!(wrong_recipient.canonical_attempt(Instant::now()).is_none());
    assert_eq!(unrelated.read_original_attempt(db).unwrap(), None);
    // Each mutation is an isolated test snapshot, never a production write.
    // Constraints remain enabled; rollback restores the full canonical proof.
    let mutations = [
        (
            "wake row missing",
            "DELETE FROM wake_work WHERE seat_id='b'",
            WakePredicate::WakePresent,
        ),
        (
            "physical projection missing",
            "DELETE FROM receipt_state WHERE seat_id='b'",
            WakePredicate::PhysicalReceiptPresent,
        ),
        (
            "human binding",
            "UPDATE occupant_bindings SET harness='human' WHERE seat_id='b'",
            WakePredicate::AgentBinding,
        ),
        (
            "competing invitation",
            "INSERT INTO digest_pending_invitations VALUES('b','diagnostic','t',1,1)",
            WakePredicate::NoInvitation,
        ),
        (
            "competing open warning",
            "INSERT INTO digest_open_warning_recipients VALUES('b','diagnostic','t','job',1)",
            WakePredicate::NoOpenWarning,
        ),
        (
            "competing programmatic warning",
            "INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES('b','diagnostic','t',1,1)",
            WakePredicate::NoProgrammaticWarning,
        ),
        (
            "job incomplete",
            "UPDATE work_jobs SET status='pending' WHERE kind='send_attention'",
            WakePredicate::Materialized,
        ),
        (
            "no active reservation",
            "UPDATE wake_work SET reservation_id=NULL WHERE seat_id='b'",
            WakePredicate::ActiveReservation,
        ),
        (
            "missing boot",
            "UPDATE wake_work SET reservation_boot=NULL WHERE seat_id='b'",
            WakePredicate::ReservationBoot,
        ),
        (
            "wake generation",
            "UPDATE wake_work SET binding_generation=binding_generation+1 WHERE seat_id='b'",
            WakePredicate::WakeGeneration,
        ),
        (
            "frontier sequence",
            "UPDATE wake_work SET last_receipt_seq=last_receipt_seq+1 WHERE seat_id='b'",
            WakePredicate::ReceiptSequence,
        ),
        (
            "frontier offset",
            "UPDATE wake_work SET last_receipt_offset=last_receipt_offset+1 WHERE seat_id='b'",
            WakePredicate::ReceiptOffset,
        ),
        (
            "canonical generation",
            "UPDATE seats SET generation=generation+1 WHERE id='b'",
            WakePredicate::CanonicalGeneration,
        ),
        (
            "unresolved seat",
            "UPDATE seats SET state='unresolved' WHERE id='b'",
            WakePredicate::ResolvedSeat,
        ),
        (
            "ended binding",
            "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='b'",
            WakePredicate::CurrentBindingPresent,
        ),
        (
            "recipient ack waived",
            "UPDATE prepared_recipients SET ack_required=0 WHERE seat_id='b'",
            WakePredicate::FrozenAck,
        ),
        (
            "recipient ineligible",
            "UPDATE prepared_recipients SET eligible_at_snapshot=0 WHERE seat_id='b'",
            WakePredicate::Eligible,
        ),
        (
            "recipient duration",
            "UPDATE prepared_recipients SET frozen_duration_ms=299999 WHERE seat_id='b'",
            WakePredicate::FrozenDuration,
        ),
        (
            "receipt not pending",
            "UPDATE receipt_state SET state='acked' WHERE seat_id='b'",
            WakePredicate::PendingReceipt,
        ),
        (
            "receipt ack waived",
            "UPDATE receipt_state SET ack_required=0 WHERE seat_id='b'",
            WakePredicate::ReceiptAck,
        ),
        (
            "receipt acked",
            "UPDATE receipt_state SET acked_at=1 WHERE seat_id='b'",
            WakePredicate::Unacked,
        ),
        (
            "manifest recipient count",
            "UPDATE send_manifests SET recipient_count=2",
            WakePredicate::OneRecipient,
        ),
        (
            "manifest warnings",
            "UPDATE send_manifests SET warning_count=1",
            WakePredicate::NoManifestWarnings,
        ),
    ];
    for (label, sql, predicate) in mutations {
        db.execute_batch("SAVEPOINT projection_control").unwrap();
        match db.execute_batch(sql) {
            Ok(()) => {
                let snapshot = compare(label);
                let row = snapshot.unique().unwrap();
                assert!(!row.accepted);
                assert_eq!(row.predicate(predicate), Some(false), "{label}");
                let canonical_allowed = matches!(
                    predicate,
                    WakePredicate::Materialized | WakePredicate::PhysicalReceiptPresent
                );
                assert_eq!(
                    snapshot.canonical_attempt(Instant::now()).is_some(),
                    canonical_allowed,
                    "canonical first differential {label}: {snapshot:?}"
                );

                if label == "physical projection missing" {
                    assert_eq!(row.predicate(WakePredicate::Unacked), None);
                    assert_eq!(row.pending_receipts, 0);
                    assert!(snapshot.canonical_attempt(Instant::now()).is_some());
                }
                if label == "wake row missing" {
                    assert_eq!(row.predicate(WakePredicate::ActiveReservation), None);
                    assert_eq!(row.predicate(WakePredicate::ReservationBoot), None);
                }
            }
            Err(error)
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) =>
            {
                println!("PROJECTION_CONSTRAINT_FORBIDS {label}: {error}");
            }
            Err(error) => panic!("{label}: {error}"),
        }
        db.execute_batch("ROLLBACK TO projection_control; RELEASE projection_control")
            .unwrap();
        assert!(compare("restored").unique().unwrap().accepted);
    }
}

#[test]
fn wake_snapshot_partial_projection_refusal_selects_later_retry() {
    // Controlled ordering contrast, not historical/native latency reproduction.
    producer_ordering_control(false, false, Some(false));
    producer_ordering_control(false, false, Some(true));
}

fn partial_projection_refusal_control(
    store: &Arc<herdr_threads::store::SqliteStore>,
    db: &rusqlite::Connection,
    target: &WakeTarget,
    job: &str,
    message: &str,
    kicks: &Arc<Mutex<Vec<herdr_threads::app::KickRecord>>>,
    case: (bool, uuid::Uuid),
) {
    let (partial, daemon_boot) = case;
    use herdr_threads::{
        notification::{
            dispatch::DispatchState,
            policy::{DurableRetry, RetryConfig},
        },
        ports::{DurableWorkAdmission, PriorLadder, RefusalCause, StorePort, WakeOutcome},
        protocol::{
            ids::SeatId,
            pagination::PageRequest,
            time::{CallBudget, Cancellation, MonoInstant},
        },
        service::kicks::enter_lane,
    };
    let clock = StorePort::clock(&**store);
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 3000),
        cancellation: Cancellation::default(),
    };
    let anchor = kicks
        .lock()
        .unwrap()
        .iter()
        .find(|(_, origin, _)| origin.is_none())
        .unwrap()
        .2;
    {
        let _origin = enter_lane(Lane::Deadlines);
        let progress = StorePort::advance_work(
            &**store,
            job,
            DurableWorkAdmission::new(if partial { 1 } else { 16 }).unwrap(),
            &budget(),
        )
        .unwrap();
        assert_eq!(progress.has_more, partial);
    }
    let before = target.read_snapshot(db).unwrap();
    let row = before.unique().unwrap();
    assert_eq!(
        row.predicate(WakePredicate::PhysicalReceiptPresent),
        Some(true)
    );
    assert_eq!(row.predicate(WakePredicate::Materialized), Some(!partial));
    let readonly = Mutex::new(
        rusqlite::Connection::open_with_flags(
            db.path().unwrap(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap(),
    );
    let samples = Arc::new(Mutex::new(Vec::with_capacity(TRACE_CAP)));
    let slot = samples.clone();
    let watched = target.clone();
    store.set_kick_pause(Box::new(move || {
        if herdr_threads::service::kicks::current_origin() != Some(Lane::Wakes) {
            return;
        }
        let at = Instant::now();
        let snapshot = watched.read_snapshot(&readonly.lock().unwrap());
        let mut samples = slot.lock().unwrap();
        assert!(samples.len() < TRACE_CAP, "controlled observer cap");
        samples.push((at, snapshot));
    }));
    let candidate = || {
        StorePort::wake_candidates(&**store, PageRequest::default(), &budget())
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.seat.as_str() == target.seat)
            .unwrap()
    };
    let first_candidate = candidate();
    let prior = PriorLadder::from_candidate(&first_candidate);
    let seat = SeatId::new(target.seat.clone());
    let mut state = DispatchState::new(RetryConfig::default(), clock.monotonic_now(), daemon_boot)
        .with_refusal_seed(1);
    state
        .restore(
            seat.clone(),
            DurableRetry {
                retry_step: first_candidate.retry_step.try_into().unwrap(),
                minimum_delay_ms: first_candidate.minimum_delay_ms,
                effective_delay_ms: first_candidate.effective_delay_ms,
                ever_reserved: first_candidate.last_reservation_id.is_some(),
            },
        )
        .unwrap();
    assert!(state.can_reserve(&seat, clock.monotonic_now()));
    let first = {
        let _origin = enter_lane(Lane::Wakes);
        StorePort::reserve_wake(&**store, &first_candidate, &budget())
            .unwrap()
            .unwrap()
    };
    state
        .reserved(
            seat.clone(),
            first.attempt.clone(),
            first.daemon_boot,
            clock.monotonic_now(),
        )
        .unwrap();
    let first_at;
    let first_evidence;
    {
        let records = samples.lock().unwrap();
        assert_eq!(records.len(), 1);
        first_at = records[0].0;
        let snapshot = records[0].1.as_ref().unwrap();
        let row = snapshot.unique().unwrap();
        assert_eq!(row.message, message);
        assert_eq!(row.reservation.as_deref(), Some(first.attempt.as_str()));
        first_evidence = snapshot
            .canonical_attempt(first_at)
            .expect("first canonical evidence");

        for predicate in WAKE_PREDICATES {
            assert_eq!(
                row.predicate(predicate),
                Some(predicate as usize != WakePredicate::Materialized as usize || !partial),
                "first {predicate:?}: {snapshot:?}"
            );
        }
        assert_eq!(row.accepted, !partial);
        assert_eq!(
            target.read_original_attempt(db).unwrap().is_some(),
            !partial
        );
        println!(
            "CONTROL_FIRST_CANONICAL partial={partial} producer={first_at:?} elapsed={:?} {snapshot:?}",
            first_at.duration_since(anchor)
        );
    }
    assert!(
        first_at.duration_since(anchor) < Duration::from_millis(100),
        "first canonical reservation must actually be early in this control"
    );
    if !partial {
        let attempt = samples.lock().unwrap()[0]
            .1
            .as_ref()
            .unwrap()
            .attempt(first_at)
            .unwrap();
        assert!(
            fenced_send_measurement(&kicks.lock().unwrap(), Some(&attempt), message).unwrap()
                < Duration::from_millis(100)
        );
        let _origin = enter_lane(Lane::Wakes);
        StorePort::complete_wake(
            &**store,
            first.attempt,
            WakeOutcome::Cancelled,
            None,
            &budget(),
        )
        .unwrap();
        store.set_kick_pause(Box::new(|| {}));
        return;
    }
    // Actual scheduler records refusal before its completion writer, then
    // restores the guard only after the store matched that exact attempt.
    let outcome = WakeOutcome::Refused(RefusalCause::Unsafe);
    let completed = clock.monotonic_now();
    assert!(
        state
            .finish(&seat, &first.attempt, &first.daemon_boot, completed)
            .unwrap()
    );
    state.record_outcome(&seat, outcome, completed);
    let matched = {
        let _origin = enter_lane(Lane::Wakes);
        StorePort::complete_wake(
            &**store,
            first.attempt.clone(),
            outcome,
            Some(&prior),
            &budget(),
        )
        .unwrap()
    };
    assert!(matched);
    state.restore_prior_guard(&seat);
    let restored = target.read_snapshot(db).unwrap();
    let row = restored.unique().unwrap();
    assert!(row.reservation.is_none() && row.reservation_boot.is_none());
    assert!(row.receipt_seq.is_none() && row.receipt_offset.is_none());
    assert!(!row.accepted);
    assert!(!state.can_reserve(&seat, completed));
    let due = state.next_due_at(completed).unwrap();
    assert!((80..=120).contains(&(due.0 - completed.0)));
    assert!(state.can_reserve(&seat, due));
    let prior_kicks = kicks.lock().unwrap().len();
    {
        let _origin = enter_lane(Lane::Deadlines);
        assert!(
            !StorePort::advance_work(
                &**store,
                job,
                DurableWorkAdmission::new(1).unwrap(),
                &budget()
            )
            .unwrap()
            .has_more
        );
    }
    assert!(
        kicks.lock().unwrap()[prior_kicks..]
            .iter()
            .all(|(lanes, _, _)| !lanes.contains(Lane::Wakes)),
        "final job-only unit must not emit a Wakes kick"
    );
    println!(
        "CONTROL_REFUSAL restored={restored:?} actual_guard_due_ms={} injected_wait_ms=120 (not historical/native delay)",
        due.0 - completed.0
    );
    std::thread::sleep(Duration::from_millis(120));
    assert!(state.can_reserve(&seat, clock.monotonic_now()));
    let second_candidate = candidate();
    let second = {
        let _origin = enter_lane(Lane::Wakes);
        StorePort::reserve_wake(&**store, &second_candidate, &budget())
            .unwrap()
            .unwrap()
    };
    assert_ne!(first.attempt, second.attempt);
    let attempt = samples
        .lock()
        .unwrap()
        .iter()
        .find_map(|(at, snapshot)| snapshot.as_ref().unwrap().attempt(*at))
        .unwrap();
    let latency = fenced_send_measurement(&kicks.lock().unwrap(), Some(&attempt), message).unwrap();
    assert!(latency >= Duration::from_millis(100));
    let red = std::panic::catch_unwind(|| {
        assert!(
            latency < Duration::from_millis(100),
            "send commit to wake attempt took {latency:?}"
        )
    });
    assert!(red.is_err());
    let repaired = fenced_first_send_measurement(
        &kicks.lock().unwrap(),
        Some(&first_evidence),
        Some(&attempt),
        message,
    )
    .unwrap();
    assert_eq!(repaired, first_at.duration_since(anchor));
    first_final_negative_controls(
        target.read_snapshot(db).unwrap(),
        first_at,
        &attempt,
        &kicks.lock().unwrap(),
        message,
    );
    assert!(
        repaired < Duration::from_millis(100),
        "first canonical endpoint must be measured despite later strong retry proof: {repaired:?}"
    );

    println!(
        "CONTROL_STRICT_RETRY_CAUSAL_RED first_early={:?} later_strict={latency:?} distinct_reservation={:?} samples={:?}; original accepted conjunction unchanged, injected ordering not historical attribution",
        first_at.duration_since(anchor),
        second.attempt,
        samples.lock().unwrap()
    );
    {
        let _origin = enter_lane(Lane::Wakes);
        StorePort::complete_wake(
            &**store,
            second.attempt,
            WakeOutcome::Cancelled,
            None,
            &budget(),
        )
        .unwrap();
    }
    store.set_kick_pause(Box::new(|| {}));
}

fn producer_ordering_control(
    logical_first: bool,
    inspect_snapshot: bool,
    partial_retry: Option<bool>,
) {
    use herdr_threads::{
        app::SystemClock,
        ports::{DurableWorkAdmission, SendPreparationProgress, StorePort},
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            commands::{DeliveryMode, PermitMutation, SendMessage},
            ids::*,
            pagination::PageRequest,
            results::CommandResult,
            time::{CallBudget, Cancellation, Clock, MonoInstant},
        },
        service::kicks::enter_lane,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let iso = TestIsolation::new("retention-observer-ordering");
    let clock = Arc::new(SystemClock::new());
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 3000),
        cancellation: Cancellation::default(),
    };
    let context = StoreContext::new(iso.path("store.db"), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES('i',0,'b',1,1);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','ordering','ordering',0,0);").unwrap();
    for seat in ["a", "b", "c"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES(?1,'i','resolved','native',?2,1,1,0)", rusqlite::params![seat,format!("p-{seat}")]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i',?1,'b',1,1,1,'fresh',0,?2,'inc','coherent_enumeration',1)",rusqlite::params![format!("p-{seat}"),format!("term-{seat}")]).unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES(?1,1,?2,'b',1,1,'codex',?3,?5,'cooperative_top_level',0,0,?4,'inc')",rusqlite::params![seat,format!("p-{seat}"),format!("session-{seat}"),format!("term-{seat}"),format!("00000000-0000-4000-8000-0000000000{seat}1")]).unwrap();
        if seat != "c" {
            db.execute(
                "INSERT INTO memberships(thread_id,seat_id,state) VALUES('t',?1,'joined')",
                [seat],
            )
            .unwrap();
            db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t',?1,1,1)",[seat]).unwrap();
        }
    }
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES('unrelated','t','c',1,'pending',0,1,300000,300000);").unwrap();
    let control_boot = uuid::Uuid::new_v4();
    let store = Arc::new(
        SqliteStore::new(
            context,
            "i",
            StoreSettings {
                daemon_boot: Some(control_boot),
                wake_batch_delay_ms: 0,
                ..StoreSettings::default()
            },
        )
        .unwrap(),
    );
    let kicks: Arc<Mutex<Vec<herdr_threads::app::KickRecord>>> = Arc::default();
    let recorded = kicks.clone();
    store.set_kick_sink(Box::new(move |lanes, origin| {
        recorded
            .lock()
            .unwrap()
            .push((lanes, origin, Instant::now()))
    }));
    let target = WakeTarget::fresh(&db, "b", "t", "a", "ordering target");
    let request = SendMessage {
        delivery_mode: DeliveryMode::Ordinary,
        user_intent: None,
        thread: ThreadId::new("t"),
        body: "ordering target".into(),
        invited_recipients: vec![SeatId::new("b")],
        deadline_millis: None,
        relays_user: false,
        operation: OperationId::new("send"),
        claim: CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("a"),
            target: HostTargetId::new("p-a"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("session-a"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000a1"),
        },
    };
    while !matches!(
        StorePort::prepare_send_step(
            &*store,
            &request,
            DurableWorkAdmission::new(16).unwrap(),
            &budget()
        )
        .unwrap(),
        SendPreparationProgress::Ready { .. }
    ) {}
    let mut second_request = request.clone();
    second_request.operation = OperationId::new("second-send");
    let mutation = PermitMutation::SendMessage(request);
    let permit = StorePort::issue_cooperative_permit(
        &*store,
        herdr_threads::store::cooperative_permit_request(&mutation).unwrap(),
        &budget(),
    )
    .unwrap();
    let CommandResult::MessageSent(message) =
        StorePort::mutate(&*store, mutation, permit, &budget()).unwrap()
    else {
        panic!("message")
    };
    let job:String = db.query_row("SELECT w.id FROM work_jobs w JOIN send_manifests m ON m.preparation_id=w.subject_id WHERE m.message_id=?1 AND w.kind='send_attention'",[message.as_str()],|r|r.get(0)).unwrap();
    assert!(
        target.read_attempt(&db, Instant::now()).unwrap().is_none(),
        "publication without materialization is not an attempt"
    );
    if let Some(partial) = partial_retry {
        partial_projection_refusal_control(
            &store,
            &db,
            &target,
            &job,
            message.as_str(),
            &kicks,
            (partial, control_boot),
        );
        return;
    }

    if logical_first {
        let candidate = StorePort::wake_candidates(&*store, PageRequest::default(), &budget())
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.seat.as_str() == "b")
            .unwrap();
        let reservation = {
            let _origin = enter_lane(Lane::Wakes);
            StorePort::reserve_wake(&*store, &candidate, &budget())
                .unwrap()
                .expect("logical manifest is actionable before projection")
        };
        let first_at = Instant::now();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM receipt_state WHERE message_id=?1",
                [message.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT reservation_id FROM wake_work WHERE seat_id='b'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            reservation.attempt.as_str()
        );
        assert!(
            target.read_attempt(&db, first_at).unwrap().is_none(),
            "an active logical reservation still lacks the required completed physical proof"
        );
        if inspect_snapshot {
            let snapshot = target.read_snapshot(&db).unwrap();
            assert_eq!(
                snapshot.message(),
                Some(message.as_str()),
                "diagnostics must retain the exact logical publication/frontier before physical projection; proof stays rejected"
            );
            let row = snapshot.unique().unwrap();
            assert_eq!(
                row.reservation.as_deref(),
                Some(reservation.attempt.as_str())
            );
            assert_eq!(row.receipt_seq, Some(row.decision_seq));
            assert_eq!(row.receipt_offset, Some(row.event_offset));
            assert_eq!(row.predicate(WakePredicate::ActiveReservation), Some(true));
            assert_eq!(row.predicate(WakePredicate::Materialized), Some(false));
            assert_eq!(
                row.predicate(WakePredicate::PhysicalReceiptPresent),
                Some(false)
            );
            assert!(row.receipt_state.is_none());
            assert_eq!(row.pending_receipts, 0);
            assert!(snapshot.canonical_attempt(first_at).is_some());
            assert_eq!(row.predicate(WakePredicate::Unacked), None);
            assert!(!row.accepted);
            assert_eq!(target.read_original_attempt(&db).unwrap(), None);
            assert!(snapshot.attempt(first_at).is_none());
            assert_projection_plan(&db, &target);
        }
        if inspect_snapshot {
            let _origin = enter_lane(Lane::Deadlines);
            let partial = StorePort::advance_work(
                &*store,
                &job,
                DurableWorkAdmission::new(1).unwrap(),
                &budget(),
            )
            .unwrap();
            assert!(
                partial.has_more,
                "one recipient unit precedes final job completion"
            );
            let snapshot = target.read_snapshot(&db).unwrap();
            let row = snapshot.unique().unwrap();
            assert_eq!(
                row.predicate(WakePredicate::PhysicalReceiptPresent),
                Some(true)
            );
            assert_eq!(row.predicate(WakePredicate::Materialized), Some(false));
            assert_eq!(row.predicate(WakePredicate::ActiveReservation), Some(true));
            assert_eq!(
                row.reservation.as_deref(),
                Some(reservation.attempt.as_str())
            );
            assert!(!row.accepted);
            assert_eq!(target.read_original_attempt(&db).unwrap(), None);
            assert!(
                kicks
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(lanes, origin, _)| *origin == Some(Lane::Deadlines)
                        && lanes.contains(Lane::Wakes)),
                "real recipient prefix emitted Deadline-origin Wakes kick while job remained pending"
            );
            println!("PROJECTION_REAL_PARTIAL_WITH_KICK {snapshot:?}");
        }
        {
            let _origin = enter_lane(Lane::Deadlines);
            assert!(
                !StorePort::advance_work(
                    &*store,
                    &job,
                    DurableWorkAdmission::new(16).unwrap(),
                    &budget()
                )
                .unwrap()
                .has_more
            );
        }
        assert!(
            target.read_attempt(&db, Instant::now()).unwrap().is_some(),
            "projection makes the same active frontier provable later"
        );
        assert_eq!(
            db.query_row(
                "SELECT reservation_id FROM wake_work WHERE seat_id='b'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            reservation.attempt.as_str()
        );
        if inspect_snapshot {
            projection_differential_controls(&db, &target, message.as_str());
            while !matches!(
                StorePort::prepare_send_step(
                    &*store,
                    &second_request,
                    DurableWorkAdmission::new(16).unwrap(),
                    &budget()
                )
                .unwrap(),
                SendPreparationProgress::Ready { .. }
            ) {}
            let second_mutation = PermitMutation::SendMessage(second_request);
            let second_permit = StorePort::issue_cooperative_permit(
                &*store,
                herdr_threads::store::cooperative_permit_request(&second_mutation).unwrap(),
                &budget(),
            )
            .unwrap();
            let CommandResult::MessageSent(second_message) =
                StorePort::mutate(&*store, second_mutation, second_permit, &budget()).unwrap()
            else {
                panic!("second genuine publication")
            };
            let ambiguous = target.read_snapshot(&db).unwrap();
            let WakeSnapshot::Ambiguous(rows) = &ambiguous else {
                panic!("two genuine same-body publications must be ambiguous: {ambiguous:?}")
            };
            assert!(rows.iter().any(|row| row.message == message.as_str()));
            assert!(
                rows.iter()
                    .any(|row| row.message == second_message.as_str())
            );
            assert!(ambiguous.attempt(Instant::now()).is_none());
            assert!(ambiguous.canonical_attempt(Instant::now()).is_none());
            assert_eq!(target.read_original_attempt(&db).unwrap(), None);
            println!("PROJECTION_AMBIGUOUS_AT_LEAST_TWO {ambiguous:?}");
        }
        println!(
            "CONTROL logical-before-projection message={message:?} first-at={first_at:?} original active attempt={:?}; first strict proof rejected, same frontier becomes provable after Deadline projection; historical callback ordinal remains unknown",
            reservation.attempt
        );
        return;
    }
    let baseline = store.commit_counts()["wake"];
    let (paused_tx, paused_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let resume_rx = Mutex::new(resume_rx);
    let armed = AtomicBool::new(true);
    store.set_kick_pause(Box::new(move || {
        if herdr_threads::service::kicks::current_origin() == Some(Lane::Deadlines)
            && armed.swap(false, Ordering::SeqCst)
        {
            paused_tx.send(()).unwrap();
            resume_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
        }
    }));
    struct PausedWriter {
        resume: Option<std::sync::mpsc::SyncSender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for PausedWriter {
        fn drop(&mut self) {
            if let Some(tx) = self.resume.take() {
                let _ = tx.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    let worker_store = store.clone();
    let worker_job = job.clone();
    let thread = std::thread::spawn(move || {
        let _origin = enter_lane(Lane::Deadlines);
        let clock = SystemClock::new();
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 3000),
            cancellation: Cancellation::default(),
        };
        let progress = StorePort::advance_work(
            &*worker_store,
            &worker_job,
            DurableWorkAdmission::new(16).unwrap(),
            &budget,
        )
        .unwrap();
        assert!(!progress.has_more);
    });
    let paused = PausedWriter {
        resume: Some(resume_tx),
        thread: Some(thread),
    };
    paused_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM receipt_state WHERE message_id=?1 AND seat_id='b' AND ack_required=1",[message.as_str()],|r|r.get::<_,i64>(0)).unwrap(),1);
    // Use the real admission primitive with its guard held across the store
    // call, exactly as ScheduledStore::reserve_wake does. SQLite release alone
    // does not release this enclosing FairWriter turn.
    let writer = Arc::new(herdr_threads::service::fair_writer::FairWriter::new(16));
    let readonly = rusqlite::Connection::open_with_flags(
        iso.path("store.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let readonly = Mutex::new(readonly);
    let traced = Arc::new(Mutex::new(AttemptTrace::new()));
    let callback_trace = traced.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release_rx = Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    store.set_kick_pause(Box::new(move || {
        if herdr_threads::service::kicks::current_origin() != Some(Lane::Wakes) {
            return;
        }
        let at = Instant::now();
        let entered = Instant::now();
        let prior = first.swap(false, Ordering::SeqCst);
        if !prior {
            // Current callback cost is after its producer endpoint, before
            // proof; the active frontier remains stable without a host here.
            std::thread::sleep(Duration::from_millis(120));
        }
        let result = target.read_attempt(&readonly.lock().unwrap(), at);
        let ended = Instant::now();
        let matched = result.as_ref().is_ok_and(|attempt| attempt.is_some());
        let mut trace = callback_trace.lock().unwrap();
        trace.samples.push(CallbackSample {
            at,
            entered,
            ended,
            queried: true,
            matched,
            snapshot: None,
        });
        if let Some(attempt) = result.unwrap() {
            trace.attempt = Some(Ok(attempt));
        }
        drop(trace);
        if prior {
            assert!(!matched, "unrelated c must be an unmatched prior callback");
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            callback_trace.lock().unwrap().samples[0].ended = Instant::now();
        }
    }));
    let prior_store = store.clone();
    let prior_writer = writer.clone();
    let prior = std::thread::spawn(move || {
        let clock = SystemClock::new();
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 3000),
            cancellation: Cancellation::default(),
        };
        let candidate = StorePort::wake_candidates(&*prior_store, PageRequest::default(), &budget)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.seat.as_str() == "c")
            .unwrap();
        let _origin = enter_lane(Lane::Wakes);
        let _turn = prior_writer.enter_background(&budget, &clock).unwrap();
        assert!(
            StorePort::reserve_wake(&*prior_store, &candidate, &budget)
                .unwrap()
                .is_some()
        );
    });
    let mut prior_guard = PausedWriter {
        resume: Some(release_tx),
        thread: Some(prior),
    };
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(store.commit_counts()["wake"] > baseline);
    let before = kicks.lock().unwrap().clone();
    assert!(traced.lock().unwrap().attempt.is_none());
    assert!(fenced_send_measurement(&before, None, message.as_str()).is_none());
    let producer_store = store.clone();
    let producer_writer = writer.clone();
    let producer = std::thread::spawn(move || {
        let clock = SystemClock::new();
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 3000),
            cancellation: Cancellation::default(),
        };
        let candidate =
            StorePort::wake_candidates(&*producer_store, PageRequest::default(), &budget)
                .unwrap()
                .items
                .into_iter()
                .find(|c| c.seat.as_str() == "b")
                .unwrap();
        let _origin = enter_lane(Lane::Wakes);
        let _turn = producer_writer.enter_background(&budget, &clock).unwrap();
        assert!(
            StorePort::reserve_wake(&*producer_store, &candidate, &budget)
                .unwrap()
                .is_some()
        );
    });
    let mut producer_guard = PausedWriter {
        resume: None,
        thread: Some(producer),
    };
    let until = Instant::now() + Duration::from_secs(3);
    while writer.waiting().1 != 1 {
        assert!(
            Instant::now() < until,
            "later target writer did not queue behind callback"
        );
        std::thread::yield_now();
    }
    // A prior callback holds real admission after SQLite's writer release.
    std::thread::sleep(Duration::from_millis(120));
    prior_guard.resume.take().unwrap().send(()).unwrap();
    prior_guard.thread.take().unwrap().join().unwrap();
    producer_guard.thread.take().unwrap().join().unwrap();
    let mut trace = traced.lock().unwrap();
    assert_eq!(trace.samples.len(), 2);
    assert!(!trace.samples[0].matched && trace.samples[1].matched);
    assert!(trace.samples[1].at.duration_since(trace.samples[0].at) >= Duration::from_millis(100));
    assert!(
        trace.samples[1].ended.duration_since(trace.samples[1].at) >= Duration::from_millis(100)
    );
    println!(
        "CONTROL prior unmatched callback blocks FairWriter; current matched callback read delayed120ms AFTER endpoint: {:?}",
        trace.samples
    );
    let current_read_end = trace.samples[1].ended;
    let attempted = trace.attempt.take().unwrap().unwrap();
    drop(trace);
    assert!(
        fenced_send_measurement(&before, Some(&attempted), message.as_str()).is_none(),
        "even target proof cannot bypass the paused materialization kick"
    );
    drop(paused);
    let after = kicks.lock().unwrap().clone();
    let latency = fenced_send_measurement(&after, Some(&attempted), message.as_str()).unwrap();
    let request_times: Vec<_> = after
        .iter()
        .filter(|(_, origin, _)| origin.is_none())
        .map(|(_, _, at)| *at)
        .collect();
    assert_eq!(
        request_times.len(),
        1,
        "preparation is unmapped; publication is this real fixture's only request kick"
    );
    let first_request = request_times[0];
    let after_callback = current_read_end.duration_since(first_request);
    assert!(
        after_callback.saturating_sub(latency) >= Duration::from_millis(100),
        "current callback delay is after producer endpoint and remains excluded"
    );
    assert_eq!(
        fenced_send_measurement(&after, Some(&attempted), message.as_str()),
        Some(latency)
    );
    println!(
        "CONTROL phase split earliest-publication-anchor={latency:?} after-current-read={after_callback:?}; prior admission delay retained, current callback cost excluded; preparation has no mapped kick"
    );
    let without_publication: Vec<_> = after
        .iter()
        .copied()
        .filter(|(_, origin, _)| origin.is_some())
        .collect();
    assert!(
        fenced_send_measurement(&without_publication, Some(&attempted), message.as_str()).is_none(),
        "a flushed publication request kick is also required"
    );
    assert!(
        latency >= Duration::from_millis(100),
        "control must really delay the target attempt: {latency:?}"
    );
    assert!(
        std::panic::catch_unwind(|| assert!(
            latency < Duration::from_millis(100),
            "commit to wake attempt took {latency:?}"
        ))
        .is_err(),
        "real >100 ms must remain rejected"
    );
    // Deliberately delay the observer after the producer transferred its
    // proof. The producer endpoint, hence measured duration, stays identical.
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(
        fenced_send_measurement(&after, Some(&attempted), message.as_str()),
        Some(latency)
    );
    assert!(attempted.proved_at.elapsed() >= Duration::from_millis(100));
    println!(
        "CONTROL target={message:?}/b exact materialization/reservation; released deadline kick={after:?}; producer upper bound={latency:?}; delayed observer leaves endpoint unchanged"
    );
}

/// Waits until every lane has finished a pass (the registry's Pacers).
fn wait_lanes_settled_latency(s: &lanes_latency::Session) {
    let until = Instant::now() + Duration::from_secs(20);
    while !Lane::ALL
        .iter()
        .all(|lane| s.probe.registered_idle_events(*lane) >= 1)
    {
        assert!(Instant::now() < until, "a lane never finished a pass");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The in-process daemon points the process stderr at its daemon.log, so a
/// failed assertion's message would vanish with the test's scratch directory.
/// With `HT_SWEEP_PANIC_FILE` set, panics are also appended there.
fn keep_panics_visible() {
    let Some(path) = std::env::var_os("HT_SWEEP_PANIC_FILE") else {
        return;
    };
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{info}\n");
        }
    }));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// `count` completed `send_attention` jobs `hist1..histN`, completed at
/// `completed_at` (ms since the epoch).
fn seed_completed_jobs(db: &rusqlite::Connection, count: u64, completed_at: u64) {
    db.execute_batch(&format!(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{count}) \
             INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) \
             SELECT 'hist'||x,'send_attention','hist'||x,0,'complete',{completed_at} FROM n"
    ))
    .unwrap();
}

fn count(db: &rusqlite::Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Every lane is registered with the commit-kick registry and a `WorkerStatus`
/// before any traffic, and then a send's commit is attempted by the wake lane
/// in under 100 ms, for three different recipients (the production worker
/// set, no tick wait).
/// Kills: a lane missing from the production worker set (the registration
/// check names it), and a send chain that falls back to the wake lane's 5 s
/// safety tick on a later send (one send alone would hide a tick that
/// happened to land).
#[test]
fn send_commit_wake_under_100ms_with_all_five_lanes() {
    let Some(scene) = Scene::new("sweep_send_commit_wake", 3) else {
        return;
    };
    for lane in Lane::ALL {
        assert!(
            scene.session().probe.registered(lane) && scene.session().probe.status_has_pacer(lane),
            "{} is not registered with both the kick registry and its status",
            lane.name()
        );
    }
    for round in 0..3 {
        let latency = scene.send_to_wake_attempt(round, &format!("sweep probe {round}"));
        assert!(
            latency < Duration::from_millis(100),
            "send {round}: commit to wake attempt took {latency:?}"
        );
    }
}

/// 6,000 completed jobs younger than the 24 h retention age are left alone;
/// retention deletes them once they age, in more than one bounded batch, and
/// keeps the `preparation_cleanup` marker row. The 100 ms send checks under
/// and after this history are `send_stays_under_100ms_with_and_after_settled_history`. The flat-cost counters themselves are the store tests
/// `work_discovery_is_flat_in_completed_jobs`,
/// `wake_discovery_is_flat_in_settled_history` and
/// `observation_walk_is_flat_in_retired_seats_and_superseded_generations`
/// (recorded in the sweep notes); this is the end-to-end form through the
/// daemon.
/// Kills: a retention pass that deletes in one unbounded transaction (fewer
/// than ceil(6000/batch) retention commits), one that prunes the kept marker
/// kind, and superseded snapshot generations that
/// accumulate.
#[test]
fn retention_keeps_tables_bounded_while_discovery_stays_flat() {
    let Some(scene) = Scene::new("sweep_retention_bounded", 2) else {
        return;
    };
    let s = scene.session();
    let db = s.db();
    const HISTORY: u64 = 6_000;
    seed_completed_jobs(&db, HISTORY, now_ms());
    db.execute(
        "INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) \
             VALUES ('keep-prep','preparation_cleanup','keep-prep',0,'complete',0)",
        [],
    )
    .unwrap();
    let hist = || count(&db, "SELECT count(*) FROM work_jobs WHERE id LIKE 'hist%'");
    assert_eq!(hist(), HISTORY as i64);
    // The 100 ms send checks under and after this history are
    // `send_stays_under_100ms_with_and_after_settled_history`, which runs alone.
    // Recent completions are not yet prunable: retention leaves them alone.
    s.probe.kick_registered(Lane::Retention);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(hist(), HISTORY as i64, "retention pruned unexpired jobs");

    // Age them, then let retention drain.
    s.db()
        .execute(
            "UPDATE work_jobs SET completed_at = 0 WHERE id LIKE 'hist%'",
            [],
        )
        .unwrap();
    let retention_from = s.commits("retention");
    let deadline_from = s.commits("deadline");
    let until = Instant::now() + Duration::from_secs(60);
    while hist() > 0 {
        assert!(
            Instant::now() < until,
            "retention left {} of {HISTORY} jobs after 60 s",
            hist()
        );
        s.probe.kick_registered(Lane::Retention);
        std::thread::sleep(Duration::from_millis(50));
    }
    let batches = s.commits("retention") - retention_from;
    let least = HISTORY.div_ceil(RETENTION_BATCH_ROWS as u64);
    assert!(
        batches >= least,
        "{HISTORY} rows need at least {least} bounded batches of {RETENTION_BATCH_ROWS}; \
         saw {batches} retention commits"
    );
    assert_eq!(
        s.commits("deadline"),
        deadline_from,
        "retention's prune commits woke the deadline lane into committing"
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM work_jobs WHERE id = 'keep-prep'"),
        1,
        "preparation_cleanup completions are never pruned"
    );
    // A send after the drain (its latency is asserted in the split-out test)
    // also gives the observation lane a chance to publish below.
    scene.send_to_wake_attempt(1, "after retention");
    // Snapshot generations stay bounded: the observation lane publishes a new
    // one every 5 s (kicks do not publish sooner) and retention keeps the
    // active, the previous and the in-flight stage only. One publication after
    // the drain, then a retention pass, is the end-to-end form; the prune
    // rules themselves are `tests/store/retention.rs`.
    let generations = || count(&db, "SELECT count(*) FROM snapshot_generations");
    let before = generations();
    wait_until(
        "an observation publication after the drain",
        Duration::from_secs(20),
        || generations() > before,
    );
    // Wait for passes, not commits: a pass with nothing left to prune only
    // reads and commits nothing (earlier drain passes already pruned, the new
    // row is an in-flight 'building' stage, or a publication freed nothing).
    // Two kicked passes in turn: the second starts after a new generation row
    // appeared. A pass that errors also counts as a pass here.
    for _ in 0..2 {
        let passes = s.probe.registered_idle_events(Lane::Retention);
        s.probe.kick_registered(Lane::Retention);
        wait_until("a retention pass", Duration::from_secs(20), || {
            s.probe.registered_idle_events(Lane::Retention) > passes
        });
    }
    let generations = count(&db, "SELECT count(*) FROM snapshot_generations");
    assert!(
        generations <= 6,
        "{generations} snapshot generations survive retention"
    );
}

/// The latency half of `retention_keeps_tables_bounded_while_discovery_stays_flat`:
/// a send under 6,000 settled jobs, and another after retention drains them,
/// are each attempted under 100 ms from commit. Split out so it can run alone
/// (`.config/nextest.toml`): beside the suite's process-heavy tests the first
/// send measured 333 ms. Kills: a discovery path that follows settled history.
#[test]
fn send_stays_under_100ms_with_and_after_settled_history() {
    let Some(scene) = Scene::new("sweep_retention_latency", 2) else {
        return;
    };
    let s = scene.session();
    let db = s.db();
    const HISTORY: u64 = 6_000;
    seed_completed_jobs(&db, HISTORY, now_ms());
    let hist = || count(&db, "SELECT count(*) FROM work_jobs WHERE id LIKE 'hist%'");
    assert_eq!(hist(), HISTORY as i64);
    let latency = scene.send_to_wake_attempt(0, "with history");
    assert!(
        latency < Duration::from_millis(100),
        "send under {HISTORY} settled jobs: commit to wake attempt took {latency:?}"
    );
    s.db()
        .execute(
            "UPDATE work_jobs SET completed_at = 0 WHERE id LIKE 'hist%'",
            [],
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(60);
    while hist() > 0 {
        assert!(
            Instant::now() < until,
            "retention left {} of {HISTORY} jobs after 60 s",
            hist()
        );
        s.probe.kick_registered(Lane::Retention);
        std::thread::sleep(Duration::from_millis(50));
    }
    let latency = scene.send_to_wake_attempt(1, "after retention");
    assert!(
        latency < Duration::from_millis(100),
        "send after the drain: commit to wake attempt took {latency:?}"
    );
}

/// The idle bound with a retention backlog. An idle window spanning a wake
/// safety tick and an observation cadence (`lane_wiring::IDLE_WINDOW`) with
/// all five lanes running and 3,000 expired jobs to prune: no deadline, wake, request or
/// admission-observer commit; the wake lane makes at most one pass per 5 s
/// window; retention drains the backlog; and its prune commits kick no lane.
/// The short window's pass bound only catches a wake lane woken more often
/// than every ~2.7 s (see `lane_wiring::idle_five_lanes_30s`).
/// Kills: a retention prune whose kick wakes the wake or deadline lane (the
/// wake lane would pass more than once per window and commit), and a backlog
/// that makes retention hold the writer past the idle bound instead of
/// finishing in batches.
#[test]
fn retention_runs_alongside_the_wake_idle_bound() {
    // Long real-time waits: runs beside, not behind, ONE_DAEMON.
    if herdr_threads::test_support::spawn::ran_in_own_process(
        module_path!(),
        "retention_runs_alongside_the_wake_idle_bound",
    ) {
        return;
    }
    let Some(s) = Session::new("sweep_retention_idle_bound") else {
        return;
    };
    keep_panics_visible();
    let pane = s.first_pane();
    s.seat(&pane);
    settle_setup(&s);
    seed_completed_jobs(&s.db(), 3_000, 0);
    let counts = s.probe.commit_counts();
    let kicks_from = s.probe.kick_log().len();
    let wake_from = s.probe.registered_idle_events(Lane::Wakes);
    let observation_from = counts.get("observation").copied().unwrap_or(0);
    let elapsed = run_idle_window(&s, wake_from, observation_from);
    let after = s.probe.commit_counts();
    for origin in ["deadline", "wake", "request", "admission-observer"] {
        assert_eq!(
            after.get(origin),
            counts.get(origin),
            "an idle daemon with a retention backlog committed from the {origin} origin: \
             {counts:?} -> {after:?}"
        );
    }
    assert_eq!(
        count(
            &s.db(),
            "SELECT count(*) FROM work_jobs WHERE id LIKE 'hist%'"
        ),
        0,
        "retention drained the backlog"
    );
    assert!(
        after["retention"] >= counts.get("retention").copied().unwrap_or(0) + 12,
        "3,000 rows are at least 12 batches: {counts:?} -> {after:?}"
    );
    let new_kicks = kicks_since(&s, kicks_from);
    assert!(
        new_kicks.is_empty(),
        "retention pruning kicked a lane: {new_kicks:?}"
    );
    let ticks = elapsed.as_secs() / 5 + 1;
    let wake_passes = s.probe.registered_idle_events(Lane::Wakes) - wake_from;
    assert!(
        (1..=ticks).contains(&wake_passes),
        "the wake lane made {wake_passes} passes in {elapsed:?} (at most {ticks})"
    );
}

/// Every lane fails at once, as a store outage would make them. Health stays
/// within its line budget (the docs' fold: more than two degraded lanes become
/// one summary line, not a line per lane), that one line names every lane and
/// the daemon log path, each lane wrote exactly one first-occurrence line to
/// daemon.log, and a later good pass clears Health back to its healthy lines.
/// Kills: a fold that is not applied through the daemon (five scheduler lines
/// or a pointer per lane), a summary that drops a lane or the log path, Health
/// lines beyond `HEALTH_LINE_BUDGET`, and a lane whose failure never reaches
/// the shared rate-limited log.
#[test]
fn lane_failure_surfaces_through_remedy_text_within_the_health_line_budget() {
    let Some(s) = Session::new("sweep_all_lanes_fail") else {
        return;
    };
    keep_panics_visible();
    let pane = s.first_pane();
    s.seat(&pane);
    wait_lanes_settled(&s, Duration::from_millis(1500));
    let (state, healthy) = s.health();
    assert_eq!(state, "healthy", "{healthy:?}");
    let log_before: Vec<usize> = Lane::ALL
        .iter()
        .map(|lane| s.lane_log_lines(*lane).len())
        .collect();
    for lane in Lane::ALL {
        s.probe.fail_lane(lane, ErrorCode::StoreBusy);
    }
    let until = Instant::now() + Duration::from_secs(40);
    while !Lane::ALL
        .iter()
        .all(|lane| s.probe.lane_health(*lane).is_some())
    {
        assert!(
            Instant::now() < until,
            "not every lane reported its failure: {:?}",
            Lane::ALL
                .iter()
                .map(|lane| (lane.name(), s.probe.lane_health(*lane)))
                .collect::<Vec<_>>()
        );
        for lane in Lane::ALL {
            s.probe.kick_registered(lane);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let (state, lines) = s.health();
    assert_eq!(state, "degraded", "{lines:?}");
    assert!(
        lines.len() <= HEALTH_LINE_BUDGET,
        "{} Health lines exceed the budget of {HEALTH_LINE_BUDGET}: {lines:?}",
        lines.len()
    );
    let scheduler: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("scheduler degraded"))
        .collect();
    assert_eq!(scheduler.len(), 1, "one folded summary line: {lines:?}");
    assert!(
        scheduler[0].contains(&format!("{} lanes degraded", Lane::ALL.len()))
            && Lane::ALL
                .iter()
                .all(|lane| scheduler[0].contains(lane.name()))
            && scheduler[0].contains(&format!("see {}", s.log.display())),
        "{scheduler:?}"
    );
    assert!(
        lines.iter().all(|line| !line.starts_with("degraded: ")),
        "a folded summary carries the log path itself, not a pointer per lane: {lines:?}"
    );
    for (lane, before) in Lane::ALL.iter().zip(&log_before) {
        assert_eq!(
            s.lane_log_lines(*lane).len(),
            before + 1,
            "{}: one first-occurrence daemon.log line however many passes failed: {:?}; log: {}",
            lane.name(),
            s.lane_log_lines(*lane),
            s.daemon_log()
        );
    }
    for lane in Lane::ALL {
        s.probe.heal_lane(lane);
    }
    wait_until("every lane to clear", Duration::from_secs(60), || {
        for lane in Lane::ALL {
            s.probe.kick_registered(lane);
        }
        Lane::ALL
            .iter()
            .all(|lane| s.probe.lane_health(*lane).is_none())
    });
    let (state, cleared) = s.health();
    assert_eq!(state, "healthy", "{cleared:?}");
    assert_eq!(cleared, healthy, "Health returns to its healthy lines");
}

/// One operator, three failure classes, each its own `remedy()` text: a start
/// attempt that cannot open its database prints that attempt's startup-log
/// remedy (exit 3), a degraded lane prints the `degraded: <remedy naming daemon.log>`
/// pointer, and a daemon whose descriptor is one protocol version behind
/// prints the stop-then-ensure skew remedy and does not print the lane pointer
/// (a skewed daemon is not read at all) -- then, with the skew removed, the
/// lane pointer is back, so neither class hid the other.
/// Kills: a skew path that decodes or reads Health from the old daemon, a
/// lane pointer that survives into the skew report, a skew that makes the
/// degraded state forget itself, and a startup failure that prints the
/// daemon.log path instead of its own attempt log.
#[test]
fn startup_failure_lane_failure_and_skew_reach_the_operator() {
    let Some(s) = Session::new("sweep_three_classes") else {
        return;
    };
    keep_panics_visible();
    // 1. Startup failure, in its own state directory.
    {
        let root = std::path::PathBuf::from(format!(
            "/private/tmp/hosw-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        let state = root.join("st");
        fs::create_dir_all(&state).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let host = root.join("no-host.sock");
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(state.clone(), host.clone(), None).unwrap(),
        )
        .unwrap();
        paths.prepare_instance_dir().unwrap();
        fs::write(&paths.database_path, b"this is not a sqlite database").unwrap();
        let ensure = crate::scrubbed_command(env!("CARGO_BIN_EXE_herdr-threads"))
            .arg("--state-dir")
            .arg(&state)
            .arg("--host-endpoint")
            .arg(&host)
            .args(["daemon", "ensure"])
            .env("HOME", root.join("home"))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&ensure.stderr).into_owned();
        assert_eq!(ensure.status.code(), Some(3), "{stderr}");
        let logs: Vec<_> = fs::read_dir(paths.instance_dir.join("logs"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(logs.len(), 1, "one per-attempt startup log: {logs:?}");
        let expected = remedy(
            None,
            &RemedyContext::StartupFailure {
                log: logs[0].path(),
            },
        );
        assert!(
            stderr.lines().any(|line| line.contains(&expected)),
            "no line has {expected:?}: {stderr}"
        );
        assert!(
            !stderr.contains(&s.log.display().to_string()),
            "a startup failure names its own attempt log, not another daemon's: {stderr}"
        );
    }

    // 2. A degraded lane, on the running daemon.
    let pane = s.first_pane();
    s.seat(&pane);
    wait_lanes_settled(&s, Duration::from_millis(1500));
    let lane = Lane::Wakes;
    s.probe.fail_lane(lane, ErrorCode::StoreBusy);
    wait_until(
        "the wake lane to report its failure",
        Duration::from_secs(20),
        || {
            s.probe.kick_registered(lane);
            s.probe.lane_health(lane).is_some()
        },
    );
    let pointer = format!(
        "degraded: {}",
        remedy(
            Some(ErrorClass::Transient),
            &RemedyContext::LaneDegraded { log: s.log.clone() }
        )
    );
    let (state, lines) = s.health();
    assert_eq!(state, "degraded", "{lines:?}");
    assert!(lines.contains(&pointer), "{pointer:?} not in {lines:?}");

    // 3. Skew: the descriptor says the daemon speaks the previous protocol.
    let paths = InstancePaths::resolve(
        &RuntimeContext::explicit(s.state.clone(), s.herdr.socket_path(), None).unwrap(),
    )
    .unwrap();
    let original = fs::read(&paths.descriptor_path).unwrap();
    let published: Value = serde_json::from_slice(&original).unwrap();
    let software = published["software_version"].as_str().unwrap().to_owned();
    let mut skewed = published.clone();
    skewed["protocol_version"] = json!(PROTOCOL_VERSION - 1);
    fs::write(&paths.descriptor_path, serde_json::to_vec(&skewed).unwrap()).unwrap();
    let restore = |bytes: &[u8]| fs::write(&paths.descriptor_path, bytes).unwrap();
    let (code, _, stderr) = s.cli(None, &["daemon", "health"]);
    restore(&original);
    assert_eq!(code, 3, "{stderr}");
    let skew = remedy(
        Some(ErrorClass::VersionSkew),
        &RemedyContext::VersionSkew {
            daemon: format!("{software} (protocol {})", PROTOCOL_VERSION - 1),
            cli: format!(
                "{} (protocol {PROTOCOL_VERSION})",
                env!("CARGO_PKG_VERSION")
            ),
        },
    );
    assert!(stderr.contains(&skew), "{skew:?} not in {stderr}");
    assert!(
        !stderr.contains(&pointer),
        "the skew report is not the lane pointer: {stderr}"
    );

    // With the skew gone the degraded state, which lives in the daemon, is
    // still reported.
    let (state, lines) = s.health();
    assert_eq!(state, "degraded", "{lines:?}");
    assert!(lines.contains(&pointer), "{pointer:?} not in {lines:?}");
    s.probe.heal_lane(lane);
    wait_until("the wake lane to clear", Duration::from_secs(40), || {
        s.probe.kick_registered(lane);
        s.probe.lane_health(lane).is_none()
    });
}

/// Unwired-value sweep finding (ht-p03.51): `HOOK_PARSE_DETAIL_BYTES` was a
/// constant nothing read, and the CLI cut the detail to 256 characters while
/// the wire rejects more than 256 bytes, so a multi-byte diagnostic (the
/// `{error:?}` of a payload that carried non-ASCII text) was dropped by
/// validation instead of reported. The CLI's cut now is the wire's byte bound.
/// Kills: a character-count truncation (300 two-byte characters would be 600
/// bytes and fail `validate`), a cut inside a character (a panic or invalid
/// text), and a bound that drifts from the validation limit.
#[test]
fn hook_parse_detail_cut_is_the_wire_bound_for_multibyte_text() {
    let long = "\u{e9}".repeat(300);
    let detail = bounded_hook_detail(&long);
    assert!(
        detail.len() <= HOOK_PARSE_DETAIL_BYTES,
        "{} bytes",
        detail.len()
    );
    assert_eq!(
        detail.len(),
        HOOK_PARSE_DETAIL_BYTES,
        "two-byte characters fill the bound exactly"
    );
    let report = |detail: String| {
        Command::HookParseFailure(HookParseFailure {
            harness: "claude".into(),
            detail,
        })
        .validate()
    };
    report(detail).expect("the CLI's own cut passes the wire validation");
    assert!(
        report("\u{e9}".repeat(HOOK_PARSE_DETAIL_BYTES / 2 + 1)).is_err(),
        "one byte over the bound is rejected"
    );
    // A three-byte character straddling the bound is cut whole.
    let straddle = format!("{}{}", "a".repeat(HOOK_PARSE_DETAIL_BYTES - 1), "\u{20ac}");
    assert_eq!(
        bounded_hook_detail(&straddle),
        "a".repeat(HOOK_PARSE_DETAIL_BYTES - 1)
    );
    assert_eq!(bounded_hook_detail("short"), "short");
}
