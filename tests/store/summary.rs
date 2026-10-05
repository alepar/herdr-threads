//! Real-store tests for the summary job service (ht-1ip.5): plan, lease,
//! bundle, submit and the p99 lease length. Path-included from
//! `src/store/summary.rs`, so private items are in scope.
//!
//! The catch-up hooks (`enter_or_keep`, `on_ready`, `on_progress`) are inert
//! stubs until ht-1ip.6; what these tests can observe of them is that the
//! handlers call them without error and use the frontier `enter_or_keep`
//! returns.
use super::*;
use crate::{
    protocol::{
        authority::{CallerRole, Harness},
        ids::{ExecutionId, HostTargetId, NativeSessionId},
        time::{Clock, MonoInstant},
    },
    store::connection::StoreContext,
};
use serde_json::json;
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

const T: &str = "t";
const T0: i64 = 1_000_000;
/// Body bytes of a standard message: about 200 bytes rendered, so three fill a 512-byte chunk.
const BODY: usize = 170;

struct Fx {
    db: Connection,
    context: StoreContext,
    settings: SummarySettings,
    now: i64,
    next_seq: u64,
}

impl Fx {
    /// Instance `i` with seats `a` and `b` (open bindings, executions `ea` and
    /// `eb`) and thread `t`; instance `o` with seat `x`.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("summary-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path, Arc::new(FixedClock));
        let db = context.open_writer().unwrap();
        db.execute_batch(
            "\
            INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,100000),('o',0,'host',1,100000);\
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('a','i','resolved','native','pane-a',1,1,0),('b','i','resolved','native','pane-b',1,1,0),('x','o','resolved','native','pane-x',1,1,0);\
            INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('a',1,1,'pane-a','term-a','inc-a','host',1,'codex','na','ea','cooperative_top_level',0,0),('b',1,1,'pane-b','term-b','inc-b','host',1,'codex','nb','eb','cooperative_top_level',0,0);\
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,1);",
        )
        .unwrap();
        Self {
            db,
            context,
            settings: SummarySettings {
                chunk_bytes: 512,
                ..SummarySettings::default()
            },
            now: T0,
            next_seq: 1,
        }
    }

    fn add(&mut self, role: &str, relays: bool, body: &str) -> u64 {
        self.add_intent(role, relays, None, body)
    }

    fn add_intent(
        &mut self,
        role: &str,
        relays: bool,
        intent: Option<UserIntent>,
        body: &str,
    ) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.db
            .execute(
                "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,\
                 decision_at,decision_seq,author_role,relays_user,user_intent) \
                 VALUES (?1,'i','t',?2,'ordinary','a',?3,?4,?2,?5,?6,?7)",
                params![
                    format!("m{seq}"),
                    seq as i64,
                    body,
                    60_000 * seq as i64,
                    role,
                    relays,
                    intent.map(UserIntent::as_str)
                ],
            )
            .unwrap();
        self.db
            .execute(
                "UPDATE threads SET next_sequence=?1 WHERE id='t'",
                [self.next_seq as i64],
            )
            .unwrap();
        seq
    }

    /// A message of the standard size whose text starts with `text`.
    fn add_sized(&mut self, role: &str, text: &str) -> u64 {
        let body = format!("{text} {}", "x".repeat(BODY.saturating_sub(text.len() + 1)));
        self.add(role, false, &body)
    }

    /// `n` agent messages of about 200 rendered bytes: three fill a 512-byte chunk.
    fn add_plain(&mut self, n: usize) {
        for _ in 0..n {
            let body = "x".repeat(BODY);
            let seq = self.add("agent", false, &body);
            let size = rendered_size(&physical_message(&self.db, &format!("m{seq}"), seq).unwrap());
            assert!((171..=255).contains(&size), "message renders {size} bytes");
        }
    }

    fn claim(&self, seat: &str, role: CallerRole) -> CallerClaim {
        CallerClaim {
            instance: "i".into(),
            seat: SeatId::new(seat),
            binding_generation: 1,
            role,
            harness: Harness::Codex,
            native_session: NativeSessionId::new(format!("n{seat}")),
            execution: ExecutionId::new(format!("e{seat}")),
            target: HostTargetId::new(format!("pane-{seat}")),
        }
    }

    /// One handler call in its own transaction, committed only on success (as
    /// the store does).
    fn run<R>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>, &SummarySettings, UtcMillis) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let now = UtcMillis(self.now);
        let tx = self.db.transaction().unwrap();
        let result = f(&tx, &self.settings, now);
        if result.is_ok() {
            tx.commit().unwrap();
        }
        result
    }

    fn summary_for(&mut self, claim: CallerClaim) -> Result<SummaryOutcome, ApiError> {
        let request = SummaryRequest {
            thread: ThreadId::new(T),
            claim,
        };
        self.run(|tx, settings, now| summary(tx, "i", &request, settings, now))
    }

    fn summary_as(&mut self, seat: &str) -> SummaryOutcome {
        let claim = self.claim(seat, CallerRole::TopLevel);
        self.summary_for(claim).unwrap()
    }

    fn work(&mut self, seat: &str) -> SummaryWork {
        match self.summary_as(seat) {
            SummaryOutcome::Work(work) => work,
            other => panic!("expected Work, got {other:?}"),
        }
    }

    fn ready(&mut self, seat: &str) -> SummaryReady {
        match self.summary_as(seat) {
            SummaryOutcome::Ready(ready) => ready,
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    fn fetch(&mut self, seat: &str, ticket: &JobTicket) -> Result<SummaryJobOutcome, ApiError> {
        let request = SummaryJobRequest {
            job_id: ticket.job_id.clone(),
            lease_token: ticket.lease_token.clone(),
            claim: self.claim(seat, CallerRole::TopLevel),
        };
        self.run(|tx, settings, now| summary_job(tx, "i", &request, settings, now))
    }

    fn bundle(&mut self, seat: &str, ticket: &JobTicket) -> JobBundle {
        match self.fetch(seat, ticket).unwrap() {
            SummaryJobOutcome::Bundle(bundle) => bundle,
            other => panic!("expected Bundle, got {other:?}"),
        }
    }

    fn submit_as(
        &mut self,
        claim: CallerClaim,
        ticket: &JobTicket,
        submission: serde_json::Value,
    ) -> Result<SubmitOutcome, ApiError> {
        let request = SummarySubmitRequest {
            job_id: ticket.job_id.clone(),
            lease_token: ticket.lease_token.clone(),
            submission,
            claim,
        };
        self.run(|tx, settings, now| summary_submit(tx, "i", &request, settings, now))
    }

    fn submit(
        &mut self,
        seat: &str,
        ticket: &JobTicket,
        submission: serde_json::Value,
    ) -> Result<SubmitOutcome, ApiError> {
        let claim = self.claim(seat, CallerRole::TopLevel);
        self.submit_as(claim, ticket, submission)
    }

    /// Fetch and validly submit every ticket of `seat`'s Work until Ready.
    fn finish_all(&mut self, seat: &str) -> SummaryReady {
        for _ in 0..40 {
            match self.summary_as(seat) {
                SummaryOutcome::Ready(ready) => return ready,
                SummaryOutcome::Work(work) => {
                    assert!(
                        !work.jobs.is_empty(),
                        "Work without tickets would never end"
                    );
                    for ticket in &work.jobs {
                        self.bundle(seat, ticket);
                        self.now += 1_000;
                        let stored = self.submit(seat, ticket, valid("n".repeat(100))).unwrap();
                        assert!(matches!(
                            stored,
                            SubmitOutcome::Stored {
                                fallback: false,
                                ..
                            }
                        ));
                    }
                }
            }
        }
        panic!("summary never became Ready");
    }

    fn count(&self, sql: &str) -> i64 {
        self.db.query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

fn valid(narrative: String) -> serde_json::Value {
    json!({
        "submission_schema": SUBMISSION_SCHEMA,
        "narrative": narrative,
        "prompt_version": "p1",
        "model": "m1",
    })
}

fn ids(tickets: &[JobTicket]) -> Vec<(String, String)> {
    tickets
        .iter()
        .map(|t| {
            (
                t.job_id.as_str().to_string(),
                t.lease_token.as_str().to_string(),
            )
        })
        .collect()
}

// ---- Step 1: p99 and durations ----

fn seed_durations(fx: &Fx, ms: impl IntoIterator<Item = u64>) {
    fx.db
        .execute(
            "INSERT OR IGNORE INTO summary_jobs(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,created_at) \
             VALUES ('sj-d','i','t','cv',0,0,1,1,0)",
            [],
        )
        .unwrap();
    for ms in ms {
        record_duration(&fx.db, "i", "sj-d", ms, UtcMillis(1)).unwrap();
    }
}

#[test]
fn p99_is_cold_below_twenty_samples() {
    let fx = Fx::new();
    let mut settings = SummarySettings::default();
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 90_000);
    seed_durations(&fx, std::iter::repeat_n(500_000, 19));
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 90_000);
    // The cold value is returned as configured, not clamped.
    settings.p99_cold_ms = 5_000;
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 5_000);
    // The twentieth sample switches to the measured, clamped value.
    seed_durations(&fx, [500_000]);
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 500_000);
}

#[test]
fn p99_nearest_rank_and_clamp() {
    let fx = Fx::new();
    let settings = SummarySettings::default();
    seed_durations(&fx, (1..=200).map(|s| s * 1_000));
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 198_000);
    fx.db
        .execute("DELETE FROM summary_job_durations", [])
        .unwrap();
    seed_durations(&fx, std::iter::repeat_n(1_000, 200));
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 30_000);
    fx.db
        .execute("DELETE FROM summary_job_durations", [])
        .unwrap();
    seed_durations(&fx, std::iter::repeat_n(20 * 60_000, 200));
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 600_000);
}

#[test]
fn only_latest_200_count() {
    let fx = Fx::new();
    let settings = SummarySettings::default();
    // Fifty old slow samples would put the 99th percentile at the 10 minute cap.
    seed_durations(&fx, std::iter::repeat_n(20 * 60_000, 50));
    seed_durations(&fx, std::iter::repeat_n(40_000, 200));
    assert_eq!(p99_job_duration(&fx.db, "i", &settings).unwrap(), 40_000);
}

#[test]
fn other_instances_samples_do_not_count() {
    let fx = Fx::new();
    let settings = SummarySettings::default();
    seed_durations(&fx, std::iter::repeat_n(200_000, 30));
    assert_eq!(p99_job_duration(&fx.db, "o", &settings).unwrap(), 90_000);
}

// ---- Step 2: plan, Ready and Work ----

#[test]
fn non_reader_is_refused() {
    let mut fx = Fx::new();
    fx.add_plain(3);
    // A seat of another instance.
    let mut claim = fx.claim("x", CallerRole::TopLevel);
    let err = fx.summary_for(claim.clone()).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unauthorized);
    // An unknown seat.
    claim.seat = SeatId::new("nobody");
    assert_eq!(
        fx.summary_for(claim).unwrap_err().code,
        ErrorCode::Unauthorized
    );
    // Nothing was planned for the refused callers.
    assert_eq!(fx.count("SELECT count(*) FROM summary_jobs"), 0);
}

#[test]
fn unknown_thread_is_not_found() {
    let mut fx = Fx::new();
    let request = SummaryRequest {
        thread: ThreadId::new("missing"),
        claim: fx.claim("a", CallerRole::TopLevel),
    };
    let err = fx
        .run(|tx, settings, now| summary(tx, "i", &request, settings, now))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn foreign_instance_claim_is_refused() {
    let mut fx = Fx::new();
    let mut claim = fx.claim("a", CallerRole::TopLevel);
    claim.instance = "o".into();
    let err = fx.summary_for(claim).unwrap_err();
    assert_eq!(err.code, ErrorCode::CallerUnverified);
}

#[test]
fn work_lists_level0_jobs_in_ascending_chunk_order_and_enters_catch_up() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    assert_eq!(work.frontier, 10);
    assert!(work.leased_elsewhere.is_empty());
    let ranges: Vec<(u64, u64)> = work
        .jobs
        .iter()
        .map(|t| (t.range.first_seq, t.range.last_seq))
        .collect();
    assert_eq!(ranges, vec![(1, 3), (4, 6), (7, 9)]);
    for (i, ticket) in work.jobs.iter().enumerate() {
        assert_eq!((ticket.level, ticket.index), (0, i as u64));
        assert_eq!(ticket.lease_until, UtcMillis(T0 + 60_000));
        assert_eq!(
            ticket.budget_bytes,
            submission_budget_bytes(0, fx.settings.narrative_bytes)
        );
        assert!(!ticket.lease_token.as_str().is_empty());
    }
    // Reservations are rows of seat `a`, unfetched.
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM summary_jobs WHERE lease_seat_id='a' AND fetched_at IS NULL \
             AND reserved_at=1000000 AND attempts=1"
        ),
        3
    );
}

#[test]
fn repoll_returns_the_same_tickets() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let first = fx.work("a");
    fx.now += 5_000;
    let second = fx.work("a");
    assert_eq!(ids(&first.jobs), ids(&second.jobs));
    assert_eq!(
        first.jobs, second.jobs,
        "re-return keeps the original lease_until"
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM summary_jobs WHERE attempts=1"),
        3
    );
}

#[test]
fn max_new_leases_caps_new_reservations() {
    let mut fx = Fx::new();
    fx.settings.max_new_leases = 2;
    fx.add_plain(10);
    let first = fx.work("a");
    assert_eq!(first.jobs.len(), 2);
    assert_eq!(
        first.jobs.iter().map(|t| t.index).collect::<Vec<_>>(),
        vec![0, 1]
    );
    for ticket in &first.jobs {
        fx.bundle("a", ticket);
    }
    let next = fx.work("a");
    // The two fetched leases are re-returned and the third chunk is reserved.
    assert_eq!(
        next.jobs.iter().map(|t| t.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(ids(&next.jobs)[..2], ids(&first.jobs)[..]);
}

#[test]
fn other_seat_sees_leased_elsewhere() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let a = fx.work("a");
    fx.now += 1_000;
    let b = fx.work("b");
    assert!(b.jobs.is_empty());
    assert_eq!(b.leased_elsewhere.len(), 3);
    for (theirs, ours) in b.leased_elsewhere.iter().zip(&a.jobs) {
        assert_eq!(theirs.job_id, ours.job_id);
        assert_eq!(theirs.lease_until, ours.lease_until);
        assert_eq!(theirs.range, ours.range);
    }
}

#[test]
fn frontier_is_the_active_rows_f() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    fx.db
        .execute(
            "INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,state) \
             VALUES ('a','t',7,1,'ea',0,'active')",
            [],
        )
        .unwrap();
    let a = fx.work("a");
    assert_eq!(a.frontier, 7);
    // Only the chunks that end at or below F are planned for it.
    assert_eq!(
        a.jobs.iter().map(|t| t.range.last_seq).collect::<Vec<_>>(),
        vec![3, 6]
    );
    // Another seat is not bound by that row.
    let b = fx.work("b");
    assert_eq!(b.frontier, 10);
    assert_eq!(b.jobs.len(), 1, "the chunk 7..9 is free; 1..6 are A's");
    assert_eq!(b.jobs[0].range.last_seq, 9);
    assert_eq!(b.leased_elsewhere.len(), 2);
}

#[test]
fn ready_assembles_cover_fold_and_tail() {
    let mut fx = Fx::new();
    fx.add_plain(1);
    fx.add_sized("human", "please do the thing");
    fx.add_plain(8);
    let ready = fx.finish_all("a");
    assert_eq!(ready.frontier, 10);
    assert_eq!(
        ready
            .cover
            .iter()
            .map(|b| (b.header.level, b.header.index))
            .collect::<Vec<_>>(),
        vec![(0, 0), (0, 1), (0, 2)]
    );
    for block in &ready.cover {
        assert_eq!(block.narrative, "n".repeat(100));
        assert_eq!(block.header.provenance, BlockProvenance::DerivedSummary);
        assert_eq!(block.header.author_seat, SeatId::new("a"));
    }
    assert_eq!(
        ready.tail.iter().map(|m| m.sequence).collect::<Vec<_>>(),
        vec![10]
    );
    assert!(ready.tail_complete);
    assert!(!ready.over_budget);
    // The human instruction is an open fold entry shown in full.
    let entry = ready
        .fold
        .entries
        .iter()
        .find(|e| e.item.id == "i.2")
        .expect("prefilled instruction");
    assert_eq!(entry.status, ItemStatus::Open);
    assert_eq!(entry.display, fold_display_full());
    assert_eq!(ready.sizes.narrative_bytes, 300);
    assert_eq!(ready.sizes.display_bytes, fx.settings.display_bytes);
    assert_eq!(ready.sizes.fold_bytes, ready.fold.rendered_bytes);
}

fn fold_display_full() -> crate::protocol::summary::FoldDisplay {
    crate::protocol::summary::FoldDisplay::Full
}

#[test]
fn over_budget_when_narratives_exceed_display_bytes_with_no_run_of_eight() {
    let mut fx = Fx::new();
    fx.settings.display_bytes = 250;
    fx.add_plain(10);
    let ready = fx.finish_all("a");
    assert_eq!(ready.sizes.narrative_bytes, 300);
    assert!(ready.over_budget);
}

#[test]
fn ready_on_an_empty_thread_has_no_cover() {
    let mut fx = Fx::new();
    let ready = fx.ready("a");
    assert_eq!(ready.frontier, 0);
    assert!(ready.cover.is_empty());
    assert!(ready.tail.is_empty());
    assert!(ready.tail_complete);
    assert_eq!(fx.count("SELECT count(*) FROM summary_jobs"), 0);
}

#[test]
fn ready_on_first_call_enters_nothing() {
    let mut fx = Fx::new();
    fx.add_plain(2);
    // No full chunk: Ready at once, with the raw tail.
    let ready = fx.ready("a");
    assert!(ready.cover.is_empty());
    assert_eq!(ready.tail.len(), 2);
    assert_eq!(fx.count("SELECT count(*) FROM catch_up"), 0);
    assert_eq!(fx.count("SELECT count(*) FROM summary_jobs"), 0);
}

#[test]
fn shared_reuse_by_second_seat() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let first = fx.finish_all("a");
    let leases = fx.count("SELECT count(*) FROM summary_jobs WHERE lease_token IS NOT NULL");
    let second = fx.ready("b");
    assert_eq!(
        first.cover.iter().map(|b| &b.block_id).collect::<Vec<_>>(),
        second.cover.iter().map(|b| &b.block_id).collect::<Vec<_>>()
    );
    // Seat a's own Work -> Ready exit leaves its row ended (ht-1ip.6); the
    // reader b never enters catch-up, and nothing stays active.
    assert_eq!(
        fx.count("SELECT count(*) FROM catch_up WHERE seat_id='b'"),
        0,
        "a reader of stored blocks enters no catch-up"
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM catch_up WHERE state='active'"),
        0
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM catch_up WHERE seat_id='a' AND state='ended' AND end_reason='ready'"),
        1
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM summary_jobs WHERE lease_token IS NOT NULL"),
        leases,
        "a reader of stored blocks reserves nothing"
    );
}

#[test]
fn stale_claim_gets_work_without_entry() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let mut claim = fx.claim("a", CallerRole::TopLevel);
    claim.binding_generation = 99;
    match fx.summary_for(claim).unwrap() {
        SummaryOutcome::Work(work) => assert_eq!(work.jobs.len(), 3),
        other => panic!("expected Work, got {other:?}"),
    }
    assert_eq!(fx.count("SELECT count(*) FROM catch_up"), 0);
}

#[test]
fn current_binding_is_generation_and_execution() {
    let fx = Fx::new();
    let thread = ThreadId::new(T);
    let claim = fx.claim("a", CallerRole::TopLevel);
    assert!(check_caller(&fx.db, "i", &claim, &thread).unwrap().current);
    let mut stale = claim.clone();
    stale.execution = ExecutionId::new("other-execution");
    assert!(!check_caller(&fx.db, "i", &stale, &thread).unwrap().current);
    let mut old = claim;
    old.binding_generation = 2;
    assert!(!check_caller(&fx.db, "i", &old, &thread).unwrap().current);
}

// ---- Step 3: fetch, lapse, expiry ----

#[test]
fn fetch_starts_the_lease() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    fx.now += 5_000;
    let bundle = fx.bundle("a", &work.jobs[0]);
    assert_eq!(bundle.job_id, work.jobs[0].job_id);
    // Cold p99 is 90 s, so the lease is 2 x 90 s from the fetch.
    let (fetched, until): (i64, i64) = fx
        .db
        .query_row(
            "SELECT fetched_at, lease_until FROM summary_jobs WHERE id=?1",
            [work.jobs[0].job_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(fetched, T0 + 5_000);
    assert_eq!(until, T0 + 5_000 + 180_000);
    // A second fetch with the same token returns the bundle without moving the lease.
    fx.now += 10_000;
    assert_eq!(fx.bundle("a", &work.jobs[0]), bundle);
    let until_again: i64 = fx
        .db
        .query_row(
            "SELECT lease_until FROM summary_jobs WHERE id=?1",
            [work.jobs[0].job_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(until_again, until);
}

#[test]
fn fetched_lease_follows_p99_and_is_capped() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    seed_durations(&fx, std::iter::repeat_n(400_000, 20));
    let work = fx.work("a");
    fx.bundle("a", &work.jobs[0]);
    let until: i64 = fx
        .db
        .query_row(
            "SELECT lease_until FROM summary_jobs WHERE id=?1",
            [work.jobs[0].job_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    // 2 x 400 s is above the 10 minute cap.
    assert_eq!(until, T0 + 600_000);
}

#[test]
fn lapsed_reservation_fetch_is_honoured_when_free() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    fx.now += 61_000;
    let bundle = fx.bundle("a", &work.jobs[0]);
    assert_eq!(bundle.range, work.jobs[0].range);
    let until: i64 = fx
        .db
        .query_row(
            "SELECT lease_until FROM summary_jobs WHERE id=?1",
            [work.jobs[0].job_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(until, fx.now + 180_000);
}

#[test]
fn lapsed_reservation_taken_by_other_returns_reservation_lapsed() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let a = fx.work("a");
    fx.now += 61_000;
    let b = fx.work("b");
    assert_eq!(b.jobs.len(), 3, "B takes the lapsed reservations");
    match fx.fetch("a", &a.jobs[0]).unwrap() {
        SummaryJobOutcome::ReservationLapsed { leased_elsewhere } => {
            assert_eq!(leased_elsewhere.job_id, a.jobs[0].job_id);
            assert_eq!(leased_elsewhere.lease_until, b.jobs[0].lease_until);
        }
        other => panic!("expected ReservationLapsed, got {other:?}"),
    }
}

#[test]
fn expired_lease_returns_to_pool() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let a = fx.work("a");
    fx.bundle("a", &a.jobs[0]);
    fx.now += 180_001;
    let b = fx.work("b");
    assert_eq!(b.jobs.len(), 3, "B gets the expired job again");
    assert_ne!(b.jobs[0].lease_token, a.jobs[0].lease_token);
    assert_eq!(b.jobs[0].job_id, a.jobs[0].job_id);
    // A's old token no longer opens the job.
    let err = fx
        .submit("a", &a.jobs[0], valid("late".into()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unauthorized);
}

#[test]
fn wrong_token_is_refused() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let a = fx.work("a");
    let mut forged = a.jobs[0].clone();
    forged.lease_token = LeaseToken::new("forged");
    let err = fx.fetch("a", &forged).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unauthorized);
    // The right token presented by another seat opens nothing: the job is
    // simply leased elsewhere, as far as that seat can tell.
    assert!(matches!(
        fx.fetch("b", &a.jobs[0]),
        Ok(SummaryJobOutcome::ReservationLapsed { .. })
    ));
    // And the holder still gets its bundle.
    assert!(matches!(
        fx.fetch("a", &a.jobs[0]),
        Ok(SummaryJobOutcome::Bundle(_))
    ));
}

#[test]
fn level0_bundle_contents() {
    let mut fx = Fx::new();
    fx.add_plain(1);
    let human = fx.add_sized("human", "ship the release");
    assert_eq!(human, 2);
    fx.add_plain(1); // seq 3 closes chunk 0
    fx.add_plain(3); // chunk 1: 4..6
    fx.add_sized("human", "second instruction"); // seq 7
    fx.add_plain(5);
    let work = fx.work("a");
    let version = render::chunking_version(&fx.settings);

    // Chunk 0, fetched first: its own prefill and messages.
    fx.now += 1_000;
    let b0 = fx.bundle("a", &work.jobs[0]);
    assert_eq!(b0.level, 0);
    assert_eq!(
        b0.messages.iter().map(|m| m.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(b0.chunking_version, version);
    assert_eq!(b0.submission_schema, SUBMISSION_SCHEMA);
    assert_eq!(b0.narrative_bytes, fx.settings.narrative_bytes);
    assert_eq!(b0.budget_bytes, work.jobs[0].budget_bytes);
    assert!(b0.children.is_empty() && b0.pinned.is_empty());
    assert_eq!(
        b0.fold
            .entries
            .iter()
            .map(|e| e.item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["i.2"]
    );

    // Store chunk 0 with a decision and a transition that closes the instruction.
    let submission = json!({
        "submission_schema": SUBMISSION_SCHEMA,
        "narrative": "chunk zero",
        "new_decisions": [{"ref": "d1", "seq": 1, "by_seat": "a", "text": "use sqlite"}],
        "transitions": [{"target": "i.2", "new_status": "done", "cite_seq": 3}],
        "prompt_version": "p1",
        "model": "m1",
    });
    fx.now += 1_000;
    assert!(matches!(
        fx.submit("a", &work.jobs[0], submission).unwrap(),
        SubmitOutcome::Stored { .. }
    ));

    // Chunk 1 (4..6), fetched after: the stored decision is in its fold, the
    // instruction closed in chunk 0 is not (closed before this chunk), and the
    // prefill of chunk 1 (no priority message there) adds nothing.
    fx.now += 1_000;
    let b1 = fx.bundle("a", &work.jobs[1]);
    assert_eq!(
        b1.fold
            .entries
            .iter()
            .map(|e| e.item.id.clone())
            .collect::<Vec<_>>(),
        vec![format!("{version}.0.1")]
    );
    assert_eq!(b1.fold.entries[0].status, ItemStatus::Active);

    // Chunk 2 (7..9): carries its own priority message as prefill, plus the
    // decision of chunk 0; the unstored chunk 1 contributes only its prefill.
    let b2 = fx.bundle("a", &work.jobs[2]);
    let mut got: Vec<String> = b2.fold.entries.iter().map(|e| e.item.id.clone()).collect();
    got.sort();
    let mut want = vec!["i.7".to_string(), format!("{version}.0.1")];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn rollup_bundle_contents() {
    let mut fx = Fx::new();
    fx.settings.display_bytes = 600;
    // Seq 1 is a long human instruction: a chunk of its own, spilled to text_ref.
    let long = "L".repeat(3_000);
    fx.add("human", false, &long);
    fx.add_plain(21); // seven more chunks of three
    fx.add_plain(1); // tail
    // Level 0: eight blocks of 100 bytes exceed the 600 display bytes.
    for _ in 0..8 {
        let work = fx.work("a");
        let ticket = work.jobs[0].clone();
        fx.bundle("a", &ticket);
        fx.now += 1_000;
        fx.submit("a", &ticket, valid("n".repeat(100))).unwrap();
    }
    let work = fx.work("a");
    assert_eq!(work.jobs.len(), 1);
    let ticket = work.jobs[0].clone();
    assert_eq!((ticket.level, ticket.index), (1, 0));
    assert_eq!(ticket.range.first_seq, 1);
    assert_eq!(ticket.range.last_seq, 22);
    assert_eq!(
        ticket.budget_bytes,
        submission_budget_bytes(1, fx.settings.narrative_bytes)
    );
    fx.now += 1_000;
    let bundle = fx.bundle("a", &ticket);
    assert_eq!(bundle.level, 1);
    assert!(bundle.messages.is_empty());
    assert_eq!(bundle.children.len(), 8);
    assert_eq!(
        bundle.children.iter().map(|c| c.index).collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );
    assert!(bundle.children.iter().all(|c| c.level == 0 && !c.fallback));
    assert!(
        bundle
            .children
            .iter()
            .all(|c| c.narrative == "n".repeat(100))
    );
    // The open long instruction is pinned with its raw body.
    assert_eq!(bundle.pinned.len(), 1);
    assert_eq!(bundle.pinned[0].item_id, "i.1");
    let seq1_id: String = fx
        .db
        .query_row(
            "SELECT id FROM messages WHERE thread_id='t' AND sequence=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let spilled = bundle
        .fold
        .entries
        .iter()
        .find(|e| e.item.id == "i.1")
        .expect("i.1 in the bundle fold");
    let ItemBody::UserInstruction { message_id, .. } = &spilled.item.body else {
        panic!("not an instruction");
    };
    assert_eq!(
        message_id.as_ref().map(|m| m.as_str()),
        Some(seq1_id.as_str())
    );
    assert_eq!(bundle.pinned[0].text.as_deref(), Some(long.as_str()));
    assert_eq!(bundle.pinned[0].text_ref, None);
    assert!(!bundle.oversized);

    // Compatible settings leave a fetched lease unchanged. A replacement
    // lease captures the smaller budget and spills its raw pinned text.
    fx.settings.bundle_bytes = 1_000;
    assert_eq!(fx.bundle("a", &ticket), bundle);
    fx.now = snapshot_state(&fx, &ticket).1.unwrap();
    let ticket = fx.work("a").jobs[0].clone();
    let small = fx.bundle("a", &ticket);
    assert_eq!(small.pinned[0].text, None);
    assert_eq!(small.pinned[0].text_ref, Some(1));
    assert!(small.oversized);
    fx.settings.bundle_bytes = SummarySettings::default().bundle_bytes;

    // A rollup submission carries narrative and labels only; its block records
    // the children and adds no ledger rows.
    let items_before = fx.count("SELECT count(*) FROM summary_items");
    fx.now += 1_000;
    let stored = fx.submit("a", &ticket, valid("rolled up".into())).unwrap();
    let SubmitOutcome::Stored { block_id, fallback } = stored else {
        panic!("expected Stored");
    };
    assert!(!fallback);
    assert_eq!(fx.count("SELECT count(*) FROM summary_items"), items_before);
    let children: String = fx
        .db
        .query_row(
            "SELECT children_json FROM summary_blocks WHERE id=?1",
            [block_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let children: Vec<String> = serde_json::from_str(&children).unwrap();
    assert_eq!(children.len(), 8);
    // The cover is now the single rollup plus the raw tail.
    let ready = fx.ready("a");
    assert_eq!(
        ready
            .cover
            .iter()
            .map(|b| (b.header.level, b.header.index))
            .collect::<Vec<_>>(),
        vec![(1, 0)]
    );
    assert_eq!(
        ready.tail.iter().map(|m| m.sequence).collect::<Vec<_>>(),
        vec![23]
    );
}

// ---- Step 4: submit ----

#[test]
fn valid_level0_submit_stores_block_items_and_transitions() {
    let mut fx = Fx::new();
    fx.add_plain(1);
    fx.add_sized("human", "see src/lib.rs and ht-12"); // seq 2
    fx.add_plain(8);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    let version = render::chunking_version(&fx.settings);
    fx.now += 2_000;
    fx.bundle("a", &ticket);
    fx.now += 7_000;
    let submission = json!({
        "submission_schema": SUBMISSION_SCHEMA,
        "narrative": "they talked",
        "new_decisions": [{"ref": "d1", "seq": 1, "by_seat": "a", "text": "use sqlite", "quote": "xxx"}],
        "new_open_items": [{"ref": "o1", "seq": 2, "kind": "ask", "from_seat": "a", "text": "who?"}],
        "transitions": [{"target": "o1", "new_status": "resolved", "cite_seq": 3}],
        "prompt_version": "pv-7",
        "model": "model-x",
    });
    let SubmitOutcome::Stored { block_id, fallback } = fx.submit("a", &ticket, submission).unwrap()
    else {
        panic!("expected Stored");
    };
    assert!(!fallback);
    let row: (
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        String,
        i64,
    ) = fx
        .db
        .query_row(
            "SELECT provenance, author_seat_id, model, prompt_version, chunking_version, level, \
             first_seq, last_seq, narrative, fallback FROM summary_blocks WHERE id=?1",
            [block_id.as_str()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "derived_summary".into(),
            "a".into(),
            "model-x".into(),
            "pv-7".into(),
            version.clone(),
            0,
            1,
            3,
            "they talked".into(),
            0
        )
    );
    let items: Vec<(String, String, i64)> = {
        let mut stmt = fx
            .db
            .prepare(
                "SELECT kind, item_id, seq FROM summary_items WHERE block_id=?1 ORDER BY rowid",
            )
            .unwrap();
        stmt.query_map([block_id.as_str()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    let (identifiers, ledger): (Vec<_>, Vec<_>) = items
        .into_iter()
        .partition(|(kind, _, _)| kind == "identifier");
    assert_eq!(
        ledger,
        vec![
            ("user_instruction".to_string(), "i.2".to_string(), 2),
            ("decision".to_string(), format!("{version}.0.1"), 1),
            ("open_item".to_string(), format!("{version}.0.2"), 2),
        ]
    );
    let mut found: Vec<String> = identifiers.into_iter().map(|(_, id, _)| id).collect();
    found.sort();
    assert_eq!(found, vec!["ht-12".to_string(), "src/lib.rs".to_string()]);
    // The transition's ref is rewritten to the assigned id.
    let transition: (String, String, i64) = fx
        .db
        .query_row(
            "SELECT target_id, new_status, cite_seq FROM summary_transitions WHERE block_id=?1",
            [block_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        transition,
        (format!("{version}.0.2"), "resolved".to_string(), 3)
    );
    // The job points at its block; its fetch-to-submit duration is recorded.
    let job: (String, i64, i64) = fx
        .db
        .query_row(
            "SELECT block_id, last_submit_token IS NOT NULL, rejections FROM summary_jobs WHERE id=?1",
            [ticket.job_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(job, (block_id.as_str().to_string(), 1, 0));
    let duration: i64 = fx
        .db
        .query_row(
            "SELECT duration_ms FROM summary_job_durations WHERE job_id=?1",
            [ticket.job_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(duration, 7_000);
}

#[test]
fn human_seat_event_cannot_supersede_an_instruction() {
    let mut fx = Fx::new();
    fx.add_plain(1);
    fx.add_sized("human", "keep ht-45 safe"); // seq 2: the instruction
    // seq 3: an info event stamped with the human role, as a human-bound seat's
    // join or accept event is.
    fx.db
        .execute(
            "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,event_json,\
             decision_at,decision_seq,author_role,relays_user) \
             VALUES ('m3','i','t',3,'info','a','{\"action\":\"accept_required\"}',180000,3,'human',0)",
            [],
        )
        .unwrap();
    fx.next_seq = 4;
    fx.add_plain(7);
    // The loader and the fold guard see the same set: the event is not in it.
    let priority = priority_sequences(&fx.db, &ThreadId::parse("t").unwrap(), 9).unwrap();
    assert!(priority.contains(&2), "{priority:?}");
    assert!(!priority.contains(&3), "{priority:?}");
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    fx.bundle("a", &ticket);
    fx.now += 1_000;
    let submission = json!({
        "submission_schema": SUBMISSION_SCHEMA,
        "narrative": "n",
        "transitions": [{"target": "i.2", "new_status": "superseded", "cite_seq": 3}],
        "prompt_version": "pv",
        "model": "m",
    });
    let outcome = fx.submit("a", &ticket, submission).unwrap();
    let SubmitOutcome::Rejected { reasons } = &outcome else {
        panic!("expected Rejected, got {outcome:?}");
    };
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("only by a priority message; 3 is not one")),
        "{reasons:?}"
    );
}

#[test]
fn stored_records_round_trip_into_the_fold() {
    let mut fx = Fx::new();
    fx.add_plain(1);
    fx.add_sized("human", "need ht-12 done");
    fx.add_plain(8);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    fx.bundle("a", &ticket);
    fx.now += 1_000;
    let submission = json!({
        "submission_schema": SUBMISSION_SCHEMA,
        "narrative": "n",
        "new_decisions": [{"ref": "d1", "seq": 1, "by_seat": "a", "text": "go"}],
        "transitions": [{"target": "i.2", "new_status": "done", "cite_seq": 3}],
        "prompt_version": "p", "model": "m",
    });
    fx.submit("a", &ticket, submission).unwrap();
    let thread = ThreadId::new(T);
    let version = render::chunking_version(&fx.settings);
    let loaded = load_level0_records(&fx.db, &thread, &version, 10).unwrap();
    assert_eq!(loaded.len(), 1);
    let (range, records) = &loaded[0];
    assert_eq!((range.first_seq, range.last_seq), (1, 3));
    assert_eq!(records.items.len(), 2);
    assert_eq!(records.transitions.len(), 1);
    assert_eq!(records.transitions[0].target_id, "i.2");
    assert!(records.identifiers.iter().any(|i| i.value == "ht-12"));
}

#[test]
fn resubmit_same_token_and_body_returns_the_same_result() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    fx.bundle("a", &ticket);
    fx.now += 1_000;
    let body = valid("one".into());
    let first = fx.submit("a", &ticket, body.clone()).unwrap();
    fx.now += 1_000;
    let again = fx.submit("a", &ticket, body).unwrap();
    assert_eq!(first, again);
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 1);
}

#[test]
fn second_valid_submit_for_stored_job_returns_existing_block_without_progress() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    fx.bundle("a", &ticket);
    fx.now += 1_000;
    let first = fx.submit("a", &ticket, valid("one".into())).unwrap();
    fx.now += 1_000;
    let second = fx
        .submit("a", &ticket, valid("a different one".into()))
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 1);
    assert_eq!(
        fx.count("SELECT count(*) FROM summary_job_durations"),
        1,
        "no second duration"
    );
    let narrative: String = fx
        .db
        .query_row("SELECT narrative FROM summary_blocks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(narrative, "one");
}

#[test]
fn first_rejection_then_fallback() {
    let mut fx = Fx::new();
    fx.add_plain(1);
    fx.add_sized("human", "keep ht-45 safe"); // seq 2
    fx.add_plain(8);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    let version = render::chunking_version(&fx.settings);
    fx.bundle("a", &ticket);
    fx.now += 1_000;

    let bad = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "", "model": "m"});
    let first = fx.submit("a", &ticket, bad.clone()).unwrap();
    let SubmitOutcome::Rejected { reasons } = &first else {
        panic!("expected Rejected, got {first:?}");
    };
    assert_eq!(reasons.len(), 1);
    assert!(reasons[0].contains("prompt_version"), "{reasons:?}");
    let rejections = |fx: &Fx| -> i64 {
        fx.db
            .query_row(
                "SELECT rejections FROM summary_jobs WHERE id=?1",
                [ticket.job_id.as_str()],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(rejections(&fx), 1);
    // The same body again is the same answer, not a second rejection.
    assert_eq!(fx.submit("a", &ticket, bad).unwrap(), first);
    assert_eq!(rejections(&fx), 1);
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 0);

    // A different invalid body is the second rejection: a final fallback block.
    let worse = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": ""});
    let SubmitOutcome::Stored { block_id, fallback } = fx.submit("a", &ticket, worse).unwrap()
    else {
        panic!("expected Stored");
    };
    assert!(fallback);
    let row: (String, String, String, i64, String) = fx
        .db
        .query_row(
            "SELECT narrative, model, prompt_version, fallback, author_seat_id FROM summary_blocks WHERE id=?1",
            [block_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            String::new(),
            "none".into(),
            "daemon-fallback-v2".into(),
            1,
            "a".into()
        )
    );
    // Ledger only: the prefill and the identifiers, nothing model-proposed.
    let kinds: Vec<(String, String)> = {
        let mut stmt = fx
            .db
            .prepare("SELECT kind, item_id FROM summary_items WHERE block_id=?1 ORDER BY rowid")
            .unwrap();
        stmt.query_map([block_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        kinds,
        vec![
            ("user_instruction".to_string(), "i.2".to_string()),
            ("identifier".to_string(), "ht-45".to_string())
        ]
    );
    assert_eq!(fx.count("SELECT count(*) FROM summary_transitions"), 0);
    assert!(!kinds.iter().any(|(_, id)| id.starts_with(&version)));
    // No fetch-to-submit duration is recorded for a fallback.
    assert_eq!(fx.count("SELECT count(*) FROM summary_job_durations"), 0);

    // The fallback is final: a later valid submit gets it back.
    fx.now += 1_000;
    let later = fx.submit("a", &ticket, valid("too late".into())).unwrap();
    assert_eq!(
        later,
        SubmitOutcome::Stored {
            block_id,
            fallback: true
        }
    );
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 1);
}

#[test]
fn expired_token_submit_is_refused() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    fx.bundle("a", &ticket);
    fx.now += 180_000;
    let err = fx.submit("a", &ticket, valid("late".into())).unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(err.detail, "summary lease expired");
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 0);
}

#[test]
fn submit_before_fetch_is_refused() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    let err = fx
        .submit("a", &work.jobs[0], valid("early".into()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(err.detail, "summary job was not fetched");
}

#[test]
fn submit_with_foreign_seat_or_token_is_refused() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    fx.bundle("a", &ticket);
    let err = fx.submit("b", &ticket, valid("mine".into())).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unauthorized);
    let mut forged = ticket.clone();
    forged.lease_token = LeaseToken::new("forged");
    let err = fx.submit("a", &forged, valid("mine".into())).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unauthorized);
}

#[test]
fn subagent_claim_may_fetch_and_submit_the_seats_lease() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    let ticket = work.jobs[0].clone();
    let child = fx.claim("a", CallerRole::Subagent);
    let request = SummaryJobRequest {
        job_id: ticket.job_id.clone(),
        lease_token: ticket.lease_token.clone(),
        claim: child.clone(),
    };
    fx.run(|tx, settings, now| summary_job(tx, "i", &request, settings, now))
        .unwrap();
    fx.now += 1_000;
    let stored = fx
        .submit_as(child, &ticket, valid("child wrote".into()))
        .unwrap();
    assert!(matches!(
        stored,
        SubmitOutcome::Stored {
            fallback: false,
            ..
        }
    ));
    // The block is stamped with the seat, not a role.
    let author: String = fx
        .db
        .query_row("SELECT author_seat_id FROM summary_blocks", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(author, "a");
}

#[test]
fn stale_version_submit_is_refused() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    // Chunk 0 is stored under the current chunking_version.
    fx.bundle("a", &work.jobs[0]);
    fx.now += 1_000;
    fx.submit("a", &work.jobs[0], valid("old".into())).unwrap();
    // Chunk 1 is fetched, then the chunking changes.
    fx.bundle("a", &work.jobs[1]);
    fx.settings.chunk_bytes = 700;
    let err = fx
        .submit("a", &work.jobs[1], valid("stale".into()))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(
        err.detail,
        "summary job was planned under an older chunking_version"
    );
    assert_eq!(
        fx.fetch("a", &work.jobs[1]).unwrap_err().detail,
        "summary job was planned under an older chunking_version"
    );
    // The old block is kept, never pruned; the new version plans from scratch
    // and the old block is not in its cover.
    let fresh = fx.work("a");
    assert_eq!(
        fresh.jobs.len(),
        2,
        "chunks of 700 bytes: 4 messages per chunk"
    );
    assert_eq!(fresh.jobs[0].index, 0);
    assert_ne!(fresh.jobs[0].job_id, work.jobs[0].job_id);
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 1);
    assert_eq!(
        fx.count("SELECT count(DISTINCT chunking_version) FROM summary_jobs"),
        2
    );
}

#[test]
fn blocks_are_unique_per_key() {
    let mut fx = Fx::new();
    fx.add_plain(10);
    let work = fx.work("a");
    fx.bundle("a", &work.jobs[0]);
    fx.now += 1_000;
    fx.submit("a", &work.jobs[0], valid("one".into())).unwrap();
    let err = fx.db.execute(
        "INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at) \
         SELECT 'dup',instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,'x',author_seat_id,model,prompt_version,created_at FROM summary_blocks",
        [],
    );
    assert!(
        err.is_err(),
        "a second block for the same key must be refused"
    );
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 1);
}

#[test]
fn rollup_fallback_has_an_empty_narrative_and_no_records() {
    let mut fx = Fx::new();
    fx.settings.display_bytes = 600;
    fx.add_plain(25); // eight chunks of three, plus one tail message
    for _ in 0..8 {
        let work = fx.work("a");
        let ticket = work.jobs[0].clone();
        fx.bundle("a", &ticket);
        fx.now += 1_000;
        fx.submit("a", &ticket, valid("n".repeat(100))).unwrap();
    }
    let ticket = fx.work("a").jobs[0].clone();
    assert_eq!(ticket.level, 1);
    fx.bundle("a", &ticket);
    fx.now += 1_000;
    // A rollup that tries to add ledger records is invalid.
    let with_records = json!({
        "submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": "m",
        "new_decisions": [{"ref": "d", "seq": 1, "by_seat": "a", "text": "t"}],
    });
    assert!(matches!(
        fx.submit("a", &ticket, with_records).unwrap(),
        SubmitOutcome::Rejected { .. }
    ));
    let still_bad = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "", "model": "m"});
    let SubmitOutcome::Stored { block_id, fallback } = fx.submit("a", &ticket, still_bad).unwrap()
    else {
        panic!("expected Stored");
    };
    assert!(fallback);
    let row: (String, i64, String) = fx
        .db
        .query_row(
            "SELECT narrative, fallback, children_json FROM summary_blocks WHERE id=?1",
            [block_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((row.0.as_str(), row.1), ("", 1));
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&row.2).unwrap().len(),
        8
    );
    let rows: i64 = fx
        .db
        .query_row(
            "SELECT count(*) FROM summary_items WHERE block_id=?1",
            [block_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

fn intent_entry(fold: &Fold, seq: u64) -> &crate::protocol::summary::FoldEntry {
    let id = format!("i.{seq}");
    let entries: Vec<_> = fold.entries.iter().filter(|e| e.item.id == id).collect();
    assert_eq!(entries.len(), 1, "source {seq}: {fold:?}");
    entries[0]
}

/// Two deliberately sized chunks: sources A #1..2, answer/completion B #3.
fn classified_two_chunks() -> (Fx, Vec<JobTicket>) {
    let mut fx = Fx::new();
    fx.add_intent(
        "human",
        false,
        Some(UserIntent::Query),
        &format!("What is progress? {}", "q".repeat(280)),
    );
    fx.add_intent(
        "agent",
        true,
        Some(UserIntent::Request),
        &format!("Cut a build. {}", "r".repeat(280)),
    );
    fx.add(
        "agent",
        false,
        &format!("Progress is complete; build shipped. {}", "a".repeat(520)),
    );
    let jobs = fx.work("a").jobs;
    assert_eq!(
        jobs.iter().map(|j| j.range).collect::<Vec<_>>(),
        vec![
            SeqRange {
                first_seq: 1,
                last_seq: 2
            },
            SeqRange {
                first_seq: 3,
                last_seq: 3
            }
        ]
    );
    (fx, jobs)
}

fn close_classified() -> serde_json::Value {
    let mut submission = valid("Progress answered and build completed".into());
    submission["transitions"] = json!([
        {"target":"i.1", "new_status":"resolved", "cite_seq":3},
        {"target":"i.2", "new_status":"resolved", "cite_seq":3}
    ]);
    submission
}

#[test]
fn user_intent_crosschunk_query_request_b_before_a() {
    let (mut fx, jobs) = classified_two_chunks();
    let b = fx.bundle("a", &jobs[1]);
    for seq in [1, 2] {
        assert_eq!(intent_entry(&b.fold, seq).status, ItemStatus::Open);
    }
    fx.now += 1_000;
    assert!(matches!(
        fx.submit("a", &jobs[1], close_classified()).unwrap(),
        SubmitOutcome::Stored {
            fallback: false,
            ..
        }
    ));
    let a = fx.bundle("a", &jobs[0]);
    assert_eq!(
        intent_entry(&a.fold, 1).status,
        ItemStatus::Open,
        "B's later citation cannot enter A's frontier"
    );
    fx.submit("a", &jobs[0], valid("Original human work".into()))
        .unwrap();
    let ready = fx.ready("a");
    for seq in [1, 2] {
        let entry = intent_entry(&ready.fold, seq);
        assert_eq!(
            (entry.status, entry.closed_at_seq),
            (ItemStatus::Resolved, Some(3))
        );
    }
    assert_eq!(
        fx.count("SELECT count(*) FROM summary_items WHERE kind='user_instruction'"),
        2
    );
    assert_eq!(fx.count("SELECT count(*) FROM summary_items i JOIN summary_blocks b ON b.id=i.block_id WHERE i.seq NOT BETWEEN b.first_seq AND b.last_seq"), 0);
}

#[test]
fn user_intent_crosschunk_missing_a_fallback_and_unanswered() {
    let mut fx = Fx::new();
    fx.settings.chunk_bytes = 1_600;
    for (intent, text) in [
        (UserIntent::Query, "progress"),
        (UserIntent::Request, "cut a build"),
        (UserIntent::Query, "unanswered"),
        (UserIntent::Request, "incomplete"),
        (UserIntent::Rule, "always test"),
    ] {
        fx.add_intent(
            "human",
            false,
            Some(intent),
            &format!("{text} {}", "x".repeat(300)),
        );
    }
    fx.add(
        "agent",
        false,
        &format!("Progress answered and build shipped. {}", "x".repeat(1_600)),
    );
    let jobs = fx.work("a").jobs;
    assert_eq!(
        jobs.iter().map(|j| j.range).collect::<Vec<_>>(),
        vec![
            SeqRange {
                first_seq: 1,
                last_seq: 5
            },
            SeqRange {
                first_seq: 6,
                last_seq: 6
            },
        ]
    );
    fx.bundle("a", &jobs[1]);
    fx.now += 1_000;
    let mut closure = close_classified();
    for transition in closure["transitions"].as_array_mut().unwrap() {
        transition["cite_seq"] = json!(6);
    }
    fx.submit("a", &jobs[1], closure).unwrap();
    fx.bundle("a", &jobs[0]);
    let mut bad = valid("invalid first".into());
    bad["model"] = json!("");
    assert!(matches!(
        fx.submit("a", &jobs[0], bad.clone()).unwrap(),
        SubmitOutcome::Rejected { .. }
    ));
    bad["narrative"] = json!("invalid second");
    assert!(matches!(
        fx.submit("a", &jobs[0], bad).unwrap(),
        SubmitOutcome::Stored { fallback: true, .. }
    ));
    let ready = fx.finish_all("a");
    for seq in [1, 2] {
        assert_eq!(
            (
                intent_entry(&ready.fold, seq).status,
                intent_entry(&ready.fold, seq).closed_at_seq
            ),
            (ItemStatus::Resolved, Some(6))
        );
    }
    for (seq, status) in [
        (3, ItemStatus::Open),
        (4, ItemStatus::Open),
        (5, ItemStatus::Active),
    ] {
        assert_eq!(intent_entry(&ready.fold, seq).status, status);
    }
    assert_eq!(
        fx.count("SELECT count(*) FROM summary_items WHERE kind='user_instruction'"),
        5
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM summary_items WHERE kind='open_item'"),
        0
    );
    assert_eq!(fx.count("SELECT count(*) FROM summary_items i JOIN summary_blocks b ON b.id=i.block_id WHERE i.seq NOT BETWEEN b.first_seq AND b.last_seq"), 0);
}

#[test]
fn user_intent_rule_change_store_replay() {
    for (change, role, relay) in [
        (RuleChange::Withdrawn, "human", false),
        (RuleChange::Replaced, "agent", true),
    ] {
        let mut fx = Fx::new();
        fx.settings.chunk_bytes = 1_000;
        fx.add_intent(
            "human",
            false,
            Some(UserIntent::Rule),
            &format!("Always test {}", "x".repeat(280)),
        );
        fx.add(
            role,
            relay,
            &format!(
                "Withdraw that rule; use the new rule instead. {}",
                "x".repeat(680)
            ),
        );
        let ticket = fx.work("a").jobs[0].clone();
        assert_eq!(
            ticket.range,
            SeqRange {
                first_seq: 1,
                last_seq: 2
            }
        );
        fx.bundle("a", &ticket);
        let mut submission = valid("Rule changed".into());
        submission["transitions"] = json!([{"target":"i.1", "new_status":"superseded", "cite_seq":2, "rule_change":change, "quote":"Withdraw that rule"}]);
        assert!(matches!(
            fx.submit("a", &ticket, submission).unwrap(),
            SubmitOutcome::Stored {
                fallback: false,
                ..
            }
        ));
        let records = load_level0_records(
            &fx.db,
            &ThreadId::new(T),
            &render::chunking_version(&fx.settings),
            2,
        )
        .unwrap();
        assert_eq!(records[0].1.transitions[0].rule_change, Some(change));
        let ready = fx.ready("a");
        assert_eq!(
            (
                intent_entry(&ready.fold, 1).status,
                intent_entry(&ready.fold, 1).closed_at_seq
            ),
            (ItemStatus::Superseded, Some(2))
        );
    }
}

#[test]
fn user_intent_active_rule_pinned_and_budgeted() {
    let mut fx = Fx::new();
    fx.settings.display_bytes = 1;
    fx.settings.bundle_bytes = 512;
    for intent in [UserIntent::Query, UserIntent::Request, UserIntent::Rule] {
        fx.add_intent("human", false, Some(intent), &"long source ".repeat(200));
    }
    fx.add_plain(16); // five more full chunks plus tail: FAN_IN level-0 blocks.
    loop {
        let work = fx.work("a");
        if work.jobs.iter().any(|j| j.level > 0) {
            let ticket = work.jobs.iter().find(|j| j.level > 0).unwrap();
            let rollup = fx.bundle("a", ticket);
            assert_eq!(
                rollup.children.len(),
                crate::protocol::summary::FAN_IN as usize
            );
            for (seq, intent, status) in [
                (1, UserIntent::Query, ItemStatus::Open),
                (2, UserIntent::Request, ItemStatus::Open),
                (3, UserIntent::Rule, ItemStatus::Active),
            ] {
                let entry = intent_entry(&rollup.fold, seq);
                assert_eq!(entry.status, status);
                assert!(
                    matches!(&entry.item.body, ItemBody::UserInstruction { user_intent: Some(i), text_ref: Some(s), message_id: Some(id), .. } if *i == intent && *s == seq && id.as_str() == format!("m{seq}"))
                );
                let pinned = rollup
                    .pinned
                    .iter()
                    .find(|p| p.seq == seq)
                    .expect("live source pinned");
                assert_eq!(
                    (&pinned.item_id, pinned.text_ref, &pinned.text),
                    (&format!("i.{seq}"), Some(seq), &None)
                );
            }
            assert!(rollup.oversized);
            assert_eq!(
                rollup.size_bytes as usize,
                serde_json::to_vec(&rollup).unwrap().len()
            );
            fx.submit("a", ticket, valid("rollup".into())).unwrap();
            break;
        }
        for ticket in work.jobs {
            let bundle = fx.bundle("a", &ticket);
            assert_eq!(
                bundle.size_bytes as usize,
                serde_json::to_vec(&bundle).unwrap().len()
            );
            fx.submit("a", &ticket, valid("n".repeat(100))).unwrap();
        }
    }
    let ready = fx.finish_all("a");
    assert!(ready.over_budget);
    for (seq, status) in [
        (1, ItemStatus::Open),
        (2, ItemStatus::Open),
        (3, ItemStatus::Active),
    ] {
        assert_eq!(intent_entry(&ready.fold, seq).status, status);
    }
}

#[test]
fn user_intent_generation2_and_schema1_refusal() {
    assert_eq!(SUBMISSION_SCHEMA, 2);
    assert_eq!(render::RENDERER_VERSION, 2);
    assert_eq!(
        crate::cli::skill::SUMMARY_PROMPT_VERSION,
        "thread-summary-v2"
    );
    let (mut fx, jobs) = classified_two_chunks();
    let bundle = fx.bundle("a", &jobs[0]);
    assert_eq!(bundle.submission_schema, 2);
    let mut old = valid("old schema first".into());
    old["submission_schema"] = json!(1);
    let SubmitOutcome::Rejected { reasons } = fx.submit("a", &jobs[0], old.clone()).unwrap() else {
        panic!("schema1 must reject")
    };
    assert_eq!(reasons, vec!["unknown submission_schema 1"]);
    old["narrative"] = json!("old schema second");
    let SubmitOutcome::Stored {
        block_id,
        fallback: true,
    } = fx.submit("a", &jobs[0], old).unwrap()
    else {
        panic!("second distinct rejection must fallback")
    };
    let block = block_by_id(&fx.db, block_id.as_str()).unwrap();
    assert_eq!(block.header.prompt_version, "daemon-fallback-v2");
}

#[test]
fn user_intent_partial_and_ack_do_not_complete() {
    use crate::{
        protocol::{
            authority::{MutationPermit, ObligationRef},
            commands::Ack,
            ids::OperationId,
            time::CallBudget,
        },
        store::{receipts, schema},
    };
    let mut fx = Fx::new();
    for (intent, text) in [
        (UserIntent::Query, "Question"),
        (UserIntent::Request, "Build requested"),
        (UserIntent::Rule, "Always test"),
    ] {
        fx.add_intent(
            "human",
            false,
            Some(intent),
            &format!("{text} {}", "x".repeat(520)),
        );
    }
    fx.add(
        "agent",
        false,
        &format!(
            "Partial answer; I will build tomorrow; this release complied. {}",
            "x".repeat(520)
        ),
    );
    fx.db.execute_batch("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m1','t','b','pending',800); INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane-b','host',1,1,0,'fresh','term-b','inc-b','coherent_enumeration',1);").unwrap();
    let receipt_before: String = fx
        .db
        .query_row(
            "SELECT state FROM receipts WHERE message_id='m1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for ticket in fx.work("a").jobs {
        fx.bundle("a", &ticket);
        let submission = valid(
            "Partial answer; promised build; compliant release. No completion transition.".into(),
        );
        assert!(matches!(
            fx.submit("a", &ticket, submission).unwrap(),
            SubmitOutcome::Stored {
                fallback: false,
                ..
            }
        ));
    }
    let ready = fx.ready("a");
    for (seq, status) in [
        (1, ItemStatus::Open),
        (2, ItemStatus::Open),
        (3, ItemStatus::Active),
    ] {
        assert_eq!(intent_entry(&ready.fold, seq).status, status);
    }
    assert_eq!(
        fx.db
            .query_row(
                "SELECT state FROM receipts WHERE message_id='m1'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        receipt_before
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM receipts WHERE acked_at IS NOT NULL"),
        0
    );
    fx.db.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-0000000000bb' WHERE seat_id='b'", []).unwrap();
    let mut claim = fx.claim("b", CallerRole::TopLevel);
    claim.execution = ExecutionId::new("00000000-0000-4000-8000-0000000000bb");
    let request = Ack {
        messages: vec![MessageId::new("m1")],
        operation: OperationId::new("intent-ack"),
        claim,
    };
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let mut permit = MutationPermit::cooperative(
        request.claim.clone(),
        request.operation.clone(),
        ObligationRef::CheckIn(request.claim.seat.clone()),
        schema::canonical_digest(&receipts::ack_payload(&request)).unwrap(),
        MonoInstant(100),
        (1, 0),
        budget.clone(),
    );
    receipts::ack(&fx.context, &mut fx.db, &budget, &request, &mut permit).unwrap();
    assert_eq!(
        fx.db
            .query_row(
                "SELECT state FROM receipts WHERE message_id='m1'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "acked"
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM receipts WHERE acked_at IS NOT NULL"),
        1
    );
    assert_eq!(fx.count("SELECT count(*) FROM summary_transitions"), 0);
    assert_eq!(
        fx.ready("a").fold,
        ready.fold,
        "ACK events do not change intent state"
    );
}

#[test]
fn user_intent_old_generation_readable_but_not_mixed() {
    let mut fx = Fx::new();
    // Genuine old renderer digest; legacy source carries no classification.
    let old_input = format!(
        "renderer=1\nchunk_bytes={}\ntracker_prefixes={}\n",
        fx.settings.chunk_bytes,
        fx.settings.tracker_prefixes.join(",")
    );
    let old_version = format!("cv1-{}", hex(&Sha256::digest(old_input.as_bytes())[..8]));
    fx.add_sized("human", "old unclassified work");
    fx.add_plain(24);
    let jobs = fx.work("a").jobs;
    let mut old_bundle = fx.bundle("a", &jobs[0]);
    old_bundle.chunking_version = old_version.clone();
    old_bundle.submission_schema = 1;
    let mut old_hasher = Sha256::new();
    for message in &old_bundle.messages {
        old_hasher.update(render::render_message(message).as_bytes());
    }
    let old_hash = hex(&old_hasher.finalize()); // renderer1 hashed raw rendered sources only
    fx.bundle("a", &jobs[1]);
    let mut bad = valid("cached old rejection".into());
    bad["model"] = json!("");
    assert!(matches!(
        fx.submit("a", &jobs[1], bad.clone()).unwrap(),
        SubmitOutcome::Rejected { .. }
    ));
    fx.db
        .execute(
            "UPDATE summary_jobs SET chunking_version=?1",
            [&old_version],
        )
        .unwrap();
    let old_id = SummaryBlockId::new("historical-block");
    fx.db.execute("INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at,job_id) VALUES (?1,'i','t',?2,0,0,1,3,?3,'historical closure','a','old-model','thread-summary-v1',0,?4)", params![old_id.as_str(), old_version, old_hash, jobs[0].job_id.as_str()]).unwrap();
    let old_body = format!(
        r#"{{"type":"user_instruction", "author_seat":"a", "author_role":"human", "relays_user":false, "text":{}, "text_ref":null}}"#,
        serde_json::to_string(&old_bundle.messages[0].text).unwrap()
    );
    fx.db.execute("INSERT INTO summary_items(block_id,kind,item_id,seq,body_json,thread_id,chunking_version) VALUES (?1,'user_instruction','i.1',1,?2,'t',?3)", params![old_id.as_str(), &old_body, old_version]).unwrap();
    fx.db.execute("INSERT INTO summary_transitions(block_id,ordinal,target_id,new_status,cite_seq,thread_id,chunking_version) VALUES (?1,0,'i.1','done',2,'t',?2)", params![old_id.as_str(), old_version]).unwrap();
    fx.db
        .execute(
            "UPDATE summary_jobs SET block_id=?1 WHERE id=?2",
            params![old_id.as_str(), jobs[0].job_id.as_str()],
        )
        .unwrap();
    let closure = valid("historical retry".into());
    let old_block = block_by_id(&fx.db, old_id.as_str()).unwrap();
    let old_records = load_level0_records(&fx.db, &ThreadId::new(T), &old_version, 25).unwrap();
    let old_inputs: Vec<_> = old_records
        .iter()
        .map(|(range, records)| BlockRecords {
            range: *range,
            records,
        })
        .collect();
    assert_eq!(
        fold::compute(&old_inputs, 25, &|_| false).entries[0].status,
        ItemStatus::Done
    );
    let old_json: String = fx
        .db
        .query_row(
            "SELECT body_json FROM summary_items WHERE kind='user_instruction'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let claims_before: Vec<(Option<String>, i64, Option<String>)> = fx
        .db
        .prepare("SELECT author_role,relays_user,user_intent FROM messages ORDER BY sequence")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (ticket, body) in [
        (&jobs[0], closure),
        (&jobs[1], bad),
        (&jobs[2], valid("never fetched".into())),
    ] {
        assert_eq!(fx.fetch("a", ticket).unwrap_err().code, ErrorCode::Conflict);
        let err = fx.submit("a", ticket, body).unwrap_err();
        assert_eq!(err.code, ErrorCode::Conflict);
        assert!(err.detail.contains("older chunking_version"));
    }
    assert_eq!(
        load_level0_records(
            &fx.db,
            &ThreadId::new(T),
            &render::chunking_version(&fx.settings),
            25
        )
        .unwrap()
        .len(),
        0
    );
    fx.settings.display_bytes = 1; // require rollup and its current-generation children
    let ready = fx.finish_all("a");
    assert_eq!(intent_entry(&ready.fold, 1).status, ItemStatus::Open);
    assert!(
        ready
            .cover
            .iter()
            .all(|b| b.header.chunking_version != old_version)
    );
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks b, json_each(b.children_json) c JOIN summary_blocks child ON child.id=c.value WHERE b.level>0 AND b.chunking_version!=child.chunking_version"), 0);
    let new_id: String = fx
        .db
        .query_row(
            "SELECT id FROM summary_blocks WHERE level=0 AND idx=0 AND chunking_version!=?1",
            [&old_version],
            |r| r.get(0),
        )
        .unwrap();
    let new_block = block_by_id(&fx.db, &new_id).unwrap();
    assert_ne!(new_block.header.source_hash, old_block.header.source_hash);
    assert_eq!(block_by_id(&fx.db, old_id.as_str()).unwrap(), old_block);
    assert_eq!(
        fx.db
            .query_row(
                "SELECT body_json FROM summary_items WHERE block_id=?1 AND kind='user_instruction'",
                [old_id.as_str()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        old_json
    );
    let claims_after: Vec<(Option<String>, i64, Option<String>)> = fx
        .db
        .prepare("SELECT author_role,relays_user,user_intent FROM messages ORDER BY sequence")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(claims_after, claims_before);
    assert_eq!(fx.count("SELECT count(*) FROM receipts"), 0);
}

fn snapshot_state(fx: &Fx, ticket: &JobTicket) -> (Option<i64>, Option<i64>, Option<String>) {
    fx.db
        .query_row(
            "SELECT fetched_at,lease_until,fetched_bundle_json FROM summary_jobs WHERE id=?1",
            [ticket.job_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[test]
fn user_intent_snapshot_repeat_fetch_and_settings() {
    let mut fx = Fx::new();
    fx.add_sized("human", "keep this source");
    fx.add_plain(3);
    let ticket = fx.work("a").jobs[0].clone();
    let bundle = fx.bundle("a", &ticket);
    let bytes = serde_json::to_string(&bundle).unwrap();
    let state = snapshot_state(&fx, &ticket);
    assert_eq!(state.2.as_deref(), Some(bytes.as_str()));
    fx.now += 1_000;
    assert_eq!(
        serde_json::to_string(&fx.bundle("a", &ticket)).unwrap(),
        bytes
    );
    assert_eq!(snapshot_state(&fx, &ticket), state);
    fx.settings.narrative_bytes = 8;
    fx.settings.bundle_bytes = 1;
    fx.settings.display_bytes = 1;
    assert_eq!(
        serde_json::to_string(&fx.bundle("a", &ticket)).unwrap(),
        bytes
    );
    assert_eq!(snapshot_state(&fx, &ticket), state);
    let SubmitOutcome::Stored { block_id, fallback } =
        fx.submit("a", &ticket, valid("n".repeat(100))).unwrap()
    else {
        panic!("frozen budget must accept submission")
    };
    assert!(!fallback);
    assert_eq!(
        block_by_id(&fx.db, block_id.as_str())
            .unwrap()
            .header
            .source_hash,
        source_hash(&bundle)
    );
}

fn snapshot_intervening_commit(rollback: bool) {
    let mut fx = Fx::new();
    fx.settings.chunk_bytes = 1_000;
    fx.settings.max_new_leases = 1;
    fx.add_intent(
        "human",
        false,
        Some(UserIntent::Query),
        &format!("Progress? {}", "x".repeat(300)),
    );
    fx.add(
        "agent",
        false,
        &format!("First answer. {}", "x".repeat(650)),
    );
    fx.add(
        "agent",
        false,
        &format!("More complete answer. {}", "x".repeat(1_000)),
    );
    let a = fx.work("a").jobs[0].clone();
    let b = fx.work("b").jobs[0].clone();
    assert_eq!((a.range.last_seq, b.range.first_seq), (2, 3));
    let before = fx.bundle("b", &b);
    assert_eq!(intent_entry(&before.fold, 1).status, ItemStatus::Open);
    let bytes = serde_json::to_string(&before).unwrap();
    fx.bundle("a", &a);
    if rollback {
        fx.now -= 1_000;
    }
    let mut closure = valid("first answer".into());
    closure["transitions"] = json!([{"target":"i.1","new_status":"resolved","cite_seq":2}]);
    closure["new_open_items"] =
        json!([{"ref":"ordinary","seq":2,"kind":"question","from_seat":"a","text":"Other work?"}]);
    assert!(matches!(
        fx.submit("a", &a, closure).unwrap(),
        SubmitOutcome::Stored {
            fallback: false,
            ..
        }
    ));
    let repeat = fx.bundle("b", &b);
    assert_eq!(serde_json::to_string(&repeat).unwrap(), bytes);
    assert_eq!(snapshot_state(&fx, &b).2.as_deref(), Some(bytes.as_str()));
    assert!(
        !repeat
            .fold
            .entries
            .iter()
            .any(|e| matches!(e.item.body, ItemBody::OpenItem { .. }))
    );
    let mut closure = valid("second answer".into());
    closure["transitions"] = json!([{"target":"i.1","new_status":"resolved","cite_seq":3}]);
    assert!(matches!(
        fx.submit("b", &b, closure).unwrap(),
        SubmitOutcome::Stored {
            fallback: false,
            ..
        }
    ));
    let ready = fx.ready("b");
    assert_eq!(ready.cover.len(), 2);
    assert_eq!(
        (
            intent_entry(&ready.fold, 1).status,
            intent_entry(&ready.fold, 1).closed_at_seq
        ),
        (ItemStatus::Resolved, Some(2))
    );
    assert_eq!(
        ready
            .fold
            .entries
            .iter()
            .filter(|e| e.item.id == "i.1")
            .count(),
        1
    );
    assert!(
        ready
            .fold
            .entries
            .iter()
            .any(|e| matches!(e.item.body, ItemBody::OpenItem { .. }))
    );
}

#[test]
fn user_intent_snapshot_equal_timestamp_intervening_commit() {
    snapshot_intervening_commit(false);
}
#[test]
fn user_intent_snapshot_clock_rollback_intervening_commit() {
    snapshot_intervening_commit(true);
}

#[test]
fn user_intent_snapshot_capture_rolls_back() {
    let mut fx = Fx::new();
    fx.add_plain(4);
    let ticket = fx.work("a").jobs[0].clone();
    let before = snapshot_state(&fx, &ticket);
    fx.now += 5_000;
    let request = SummaryJobRequest {
        job_id: ticket.job_id.clone(),
        lease_token: ticket.lease_token.clone(),
        claim: fx.claim("a", CallerRole::TopLevel),
    };
    let err=fx.run(|tx,settings,now| {
        let version=render::chunking_version(settings);
        tx.execute("INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at,job_id) VALUES ('malformed','i','t',?1,0,0,1,3,'hash','bad','a','model','prompt',0,?2)", params![version,ticket.job_id.as_str()]).unwrap();
        tx.execute("INSERT INTO summary_items(block_id,kind,item_id,seq,body_json,thread_id,chunking_version) VALUES ('malformed','open_item','bad',1,'{bad','t',?1)", [&version]).unwrap();
        summary_job(tx,"i",&request,settings,now)
    }).unwrap_err();
    assert_eq!(err.code, ErrorCode::StoreCorrupt);
    assert_eq!(snapshot_state(&fx, &ticket), before);
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 0);
    let bundle = fx.bundle("a", &ticket);
    let mut wrong_job = serde_json::to_value(&bundle).unwrap();
    wrong_job["job_id"] = json!("another-job");
    for value in [
        Some("{malformed".to_owned()),
        None,
        Some(wrong_job.to_string()),
    ] {
        fx.db
            .execute(
                "UPDATE summary_jobs SET fetched_bundle_json=?1 WHERE id=?2",
                params![value, ticket.job_id.as_str()],
            )
            .unwrap();
        for err in [
            fx.fetch("a", &ticket).unwrap_err(),
            fx.submit("a", &ticket, valid("no rebuild".into()))
                .unwrap_err(),
        ] {
            assert_eq!(err.code, ErrorCode::StoreCorrupt);
            assert!(err.detail.len() < 160);
        }
    }
    assert_eq!(fx.count("SELECT count(*) FROM summary_blocks"), 0);
}

#[test]
fn user_intent_snapshot_lease_reset_and_fences() {
    let mut fx = Fx::new();
    fx.add_plain(7);
    let jobs = fx.work("a").jobs;
    let ticket = &jobs[1];
    assert_eq!(
        fx.submit("a", ticket, valid("unfetched".into()))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let first = fx.bundle("a", ticket);
    let first_state = snapshot_state(&fx, ticket);
    assert!(matches!(
        fx.fetch("b", ticket).unwrap(),
        SummaryJobOutcome::ReservationLapsed { .. }
    ));
    assert_eq!(
        fx.submit("b", ticket, valid("foreign".into()))
            .unwrap_err()
            .code,
        ErrorCode::Unauthorized
    );
    let mut foreign = ticket.clone();
    foreign.lease_token = LeaseToken::new("foreign-token");
    assert_eq!(
        fx.fetch("a", &foreign).unwrap_err().code,
        ErrorCode::Unauthorized
    );
    assert_eq!(
        fx.submit("a", &foreign, valid("foreign".into()))
            .unwrap_err()
            .code,
        ErrorCode::Unauthorized
    );
    let mut bad = valid("cached rejection".into());
    bad["model"] = json!("");
    let rejected = fx.submit("a", ticket, bad.clone()).unwrap();
    assert!(matches!(rejected, SubmitOutcome::Rejected { .. }));
    assert_eq!(fx.submit("a", ticket, bad.clone()).unwrap(), rejected);
    // Invalid callers and expired/stale leases are fenced before snapshot decode
    // or cached rejection replay, even when persistence is corrupt.
    fx.db
        .execute(
            "UPDATE summary_jobs SET fetched_bundle_json='{bad' WHERE id=?1",
            [ticket.job_id.as_str()],
        )
        .unwrap();
    for sql in [
        "UPDATE occupant_bindings SET generation=2 WHERE seat_id='a'",
        "UPDATE occupant_bindings SET execution_id='changed' WHERE seat_id='a'",
    ] {
        fx.db.execute(sql, []).unwrap();
        assert_eq!(
            fx.fetch("a", ticket).unwrap_err().code,
            ErrorCode::CallerUnverified
        );
        assert_eq!(
            fx.submit("a", ticket, bad.clone()).unwrap_err().code,
            ErrorCode::CallerUnverified
        );
        fx.db
            .execute(
                "UPDATE occupant_bindings SET generation=1,execution_id='ea' WHERE seat_id='a'",
                [],
            )
            .unwrap();
    }
    fx.settings.chunk_bytes += 1;
    assert_eq!(fx.fetch("a", ticket).unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        fx.submit("a", ticket, bad.clone()).unwrap_err().code,
        ErrorCode::Conflict
    );
    fx.settings.chunk_bytes -= 1;
    fx.now = first_state.1.unwrap();
    assert_eq!(fx.fetch("a", ticket).unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        fx.submit("a", ticket, bad).unwrap_err().code,
        ErrorCode::Conflict
    );
    let new = fx
        .work("b")
        .jobs
        .into_iter()
        .find(|j| j.job_id == ticket.job_id)
        .unwrap();
    assert_ne!(new.lease_token, ticket.lease_token);
    let cleared = snapshot_state(&fx, &new);
    assert_eq!((cleared.0, cleared.2), (None, None));
    // A new lease captures a changed current fold, while the old snapshot was stable.
    let other = fx
        .work("b")
        .jobs
        .into_iter()
        .find(|j| j.job_id == jobs[0].job_id)
        .unwrap();
    fx.bundle("b", &other);
    let mut item = valid("new work".into());
    item["new_open_items"] =
        json!([{"ref":"new","seq":1,"kind":"ask","from_seat":"a","text":"New work?"}]);
    fx.submit("b", &other, item).unwrap();
    let second = fx.bundle("b", &new);
    assert_ne!(second, first);
    assert_eq!(fx.bundle("b", &new), second);
    let stored = fx.submit("b", &new, valid("stored".into())).unwrap();
    assert_eq!(fx.submit("b", &new, valid("retry".into())).unwrap(), stored);
    fx.db
        .execute(
            "UPDATE occupant_bindings SET execution_id='changed' WHERE seat_id='b'",
            [],
        )
        .unwrap();
    assert_eq!(
        fx.submit("b", &new, valid("stored retry".into()))
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
    fx.db
        .execute(
            "UPDATE occupant_bindings SET execution_id='eb' WHERE seat_id='b'",
            [],
        )
        .unwrap();
    fx.settings.chunk_bytes += 1;
    assert_eq!(
        fx.submit("b", &new, valid("stored retry".into()))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    fx.settings.chunk_bytes -= 1;
    fx.now = snapshot_state(&fx, &new).1.unwrap();
    assert_eq!(
        fx.submit("b", &new, valid("stored retry".into()))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}
