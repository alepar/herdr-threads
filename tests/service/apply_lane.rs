//! The Apply lane (journal-core design section 13): kicked passes, the apply
//! hint, backoff and Health, the post-commit hook, the WAL checkpoint and
//! shutdown. The store is real and the clock is fake; no daemon starts. A
//! second journal (the "source") authors entries that the lane's store ingests.
use super::*;
use crate::{
    node::{NodeIdentity, NodeUuid, tag_for},
    protocol::time::{MonoInstant, UtcMillis},
    service::kicks::{CommitKicks, LaneSet},
    store::{
        SqliteStore, StoreSettings,
        connection::{HoldSource, OWN_HISTORY_HOLD_MAX, OwnHistoryHold, StoreContext},
        journal::{self as jr, Authored},
    },
    sync::{
        BatchLimits, EntityKey, LogKey,
        facts::{ApplyContext, FactError, FactKind, FactRegistry},
        journal_port::JournalPort,
    },
    test_support::isolation::TestIsolation,
};
use rusqlite::{Connection, Transaction};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64};

/// One flushed commit kick: the lanes and the committing thread's origin.
type KickEntry = (LaneSet, Option<Lane>);

const HOUR_MS: i64 = 60 * 60 * 1000;

struct LaneClock {
    mono: AtomicU64,
    utc: AtomicI64,
}
impl Clock for LaneClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = std::time::Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

fn seed_bytes(n: u32) -> [u8; 32] {
    let mut seed = [7u8; 32];
    seed[..4].copy_from_slice(&n.to_le_bytes());
    seed
}

/// Two seeds whose nodes carry the same tag at `k = 0`, as `(lower, higher)`.
fn colliding_seeds() -> (u32, u32) {
    let uuid_of = |n: u32| NodeIdentity::from_seed(seed_bytes(n)).uuid();
    let mut seen = std::collections::HashMap::new();
    for n in 0..200_000 {
        let tag = tag_for(uuid_of(n), 0);
        if let Some(&earlier) = seen.get(&tag) {
            return if uuid_of(earlier) < uuid_of(n) {
                (earlier, n)
            } else {
                (n, earlier)
            };
        }
        seen.insert(tag, n);
    }
    panic!("no tag collision within 200000 seeds");
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Put {
    id: String,
}

/// A kind that bumps a local counter, so an applied entry is observable.
struct PutKind;
impl FactKind for PutKind {
    const NAME: &'static str = "test.put";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        EntityKey::new(format!("e:{}", p.id)).ok()
    }
    fn needs(_: &Put) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Put) -> Result<(), FactError> {
        tx.execute(
            "UPDATE local_node SET send_eligibility_revision=send_eligibility_revision+1",
            [],
        )
        .map_err(|e| FactError::from_api(crate::store::connection::store_error(e)))?;
        Ok(())
    }
}

/// One journal: its context, writer connection and active log.
struct Source {
    ctx: StoreContext,
    conn: Connection,
    log: LogKey,
}

impl Source {
    fn put_all(&mut self, from: usize, to: usize) {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| {
                let mut last: Option<Authored> = None;
                for n in from..to {
                    last = Some(jr::author::<PutKind>(
                        tx,
                        ctx,
                        d,
                        &Put {
                            id: format!("p{n}"),
                        },
                    )?);
                }
                Ok(last)
            },
        )
        .unwrap();
    }

    /// Every entry of the source's active log.
    fn entries(&self) -> Vec<crate::sync::envelope::Envelope> {
        let mut all = Vec::new();
        loop {
            let from = all.len() as u64 + 1;
            let batch = jr::serve(&self.conn, &self.log, from, BatchLimits::default()).unwrap();
            if batch.is_empty() {
                return all;
            }
            all.extend(batch);
        }
    }
}

fn open_journal(
    path: PathBuf,
    clock: Arc<LaneClock>,
    seed: u32,
    registry: &'static FactRegistry,
) -> (StoreContext, Connection, LogKey, NodeUuid) {
    let identity = Arc::new(NodeIdentity::from_seed(seed_bytes(seed)));
    let ctx = StoreContext::new(path, clock)
        .with_node_seed(crate::test_support::node_seed_for(identity.uuid()));
    ctx.seed_incarnations(u64::from(seed) + 1);
    let mut conn = ctx.open_writer().unwrap();
    let uuid = identity.uuid();
    let report = jr::open(&ctx, &mut conn, identity, "host", registry).unwrap();
    (ctx, conn, report.self_log, uuid)
}

/// The lane's store, a fake clock and the pieces of one Apply lane; the lane
/// thread starts with `start`.
struct LaneFx {
    iso: TestIsolation,
    clock: Arc<LaneClock>,
    registry: &'static FactRegistry,
    store: Arc<SqliteStore>,
    path: PathBuf,
    writer: Arc<FairWriter>,
    pacer: Arc<Pacer>,
    status: Arc<WorkerStatus>,
    cancel: Cancellation,
    handle: Option<thread::JoinHandle<()>>,
    calls: Arc<Mutex<Vec<ApplyProgress>>>,
    kicked: Arc<Mutex<Vec<KickEntry>>>,
}

impl LaneFx {
    fn new(label: &str, seed: u32) -> Self {
        let registry = FactRegistry::builtins_plus(|b| b.with::<PutKind>());
        let iso = TestIsolation::new(label);
        let path = iso.state_root().join("store.db");
        let clock = Arc::new(LaneClock {
            mono: AtomicU64::new(1),
            utc: AtomicI64::new(1_000 * HOUR_MS),
        });
        let (ctx, conn, _, uuid) = open_journal(path.clone(), clock.clone(), seed, registry);
        drop(conn);
        let kicks = Arc::new(CommitKicks::default());
        let store = Arc::new(
            SqliteStore::new(ctx, uuid.to_string(), StoreSettings::default())
                .unwrap()
                .with_commit_kicks(Arc::clone(&kicks)),
        );
        let kicked = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&kicked);
        store.set_kick_sink(Box::new(move |lanes, origin| {
            if !lanes.is_empty() {
                sink.lock().unwrap().push((lanes, origin));
            }
        }));
        let cancel = Cancellation::default();
        let pacer = Arc::new(Pacer::new("apply", clock.clone(), cancel.clone()));
        kicks.register(Lane::Apply, Arc::clone(&pacer));
        Self {
            iso,
            clock,
            registry,
            store,
            path,
            writer: Arc::new(FairWriter::new(32)),
            pacer,
            status: Arc::new(WorkerStatus::default()),
            cancel,
            handle: None,
            calls: Arc::default(),
            kicked,
        }
    }

    fn start(&mut self) {
        self.start_with(Box::new(|_| {}));
    }

    fn start_with(&mut self, post_commit: ApplyPostCommit) {
        let calls = Arc::clone(&self.calls);
        self.handle = Some(
            start_apply_worker(
                self.store.clone(),
                Box::new(move |progress| {
                    calls.lock().unwrap().push(*progress);
                    post_commit(progress);
                }),
                Arc::clone(&self.writer),
                Arc::clone(&self.pacer),
                self.cancel.clone(),
                Arc::clone(&self.status),
            )
            .unwrap(),
        );
    }

    /// A second journal on the same fake clock.
    fn source(&self, label: &str, seed: u32) -> Source {
        let path = self.iso.state_root().join(format!("{label}.db"));
        let (ctx, conn, log, _) = open_journal(path, self.clock.clone(), seed, self.registry);
        Source { ctx, conn, log }
    }

    /// Ingests every entry of `source` into the lane's store.
    fn ship(&self, source: &Source) {
        for chunk in source.entries().chunks(200) {
            let outcome = self
                .store
                .ingest(&source.log, chunk.to_vec(), &budget())
                .unwrap();
            assert!(
                matches!(
                    outcome,
                    crate::sync::journal_port::IngestOutcome::Stored { .. }
                ),
                "{outcome:?}"
            );
        }
    }

    /// Moves the fake clock forward and wakes the lane's pacer.
    fn advance(&self, ms: u64) {
        self.clock.mono.fetch_add(ms, Ordering::SeqCst);
        self.clock.utc.fetch_add(ms as i64, Ordering::SeqCst);
        self.pacer.clock_advanced();
    }

    fn wait_idle(&self, n: u64) {
        wait_until("the lane to wait", || self.pacer.idle_events() >= n);
    }

    fn apply_commits(&self) -> u64 {
        self.store
            .commit_counts()
            .get("apply")
            .copied()
            .unwrap_or(0)
    }

    fn count(&self, sql: &str) -> i64 {
        let db = Connection::open(&self.path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn hold(&self) -> Arc<OwnHistoryHold> {
        Arc::clone(self.store.context().sync().unwrap().own_history_hold())
    }

    fn applied_puts(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM journal WHERE kind='test.put' AND apply_state='applied'")
    }

    /// Injects a StoreBusy failure for Apply-origin store access while `gate` is set.
    fn fault_while(&self, gate: &Arc<std::sync::atomic::AtomicBool>) {
        let gate = Arc::clone(gate);
        self.store
            .set_lane_fault(Some(Arc::new(move |origin: Option<Lane>| {
                (origin == Some(Lane::Apply) && gate.load(Ordering::SeqCst))
                    .then(|| ApiError::new(ErrorCode::StoreBusy, "injected"))
            })));
    }

    fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

impl Drop for LaneFx {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn kicked_ingest_applied_within_one_pass() {
    let mut fx = LaneFx::new("apply-kicked", 1);
    fx.start();
    fx.wait_idle(1);
    let before = fx.apply_commits();
    let mut source = fx.source("src", 2);
    source.put_all(0, 3);
    fx.ship(&source);
    // genesis + three puts, in the kicked pass (well under the 64-entry limit).
    wait_until("the entries to apply", || fx.applied_puts() == 3);
    fx.wait_idle(2);
    assert_eq!(
        fx.apply_commits() - before,
        1,
        "one pass applied the whole batch"
    );
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM journal WHERE apply_state<>'applied'"),
        0
    );
    assert_eq!(fx.store.checkpoints_run(), 0, "far below 1024 entries");
    assert!(fx.status.health().is_none());
}

#[test]
fn empty_pass_commits_nothing_kicks_nothing() {
    let mut fx = LaneFx::new("apply-empty", 1);
    fx.start();
    fx.wait_idle(1);
    let commits = fx.apply_commits();
    let kicks = fx.kicked.lock().unwrap().len();
    // A hinted pass over an empty backlog.
    fx.store.set_apply_hint();
    let turns = fx.writer.turns();
    fx.pacer.kick();
    fx.wait_idle(2);
    assert!(
        fx.writer.turns() > turns,
        "the hinted pass did take a writer turn"
    );
    assert_eq!(fx.apply_commits(), commits, "nothing to apply, no commit");
    assert_eq!(
        fx.kicked.lock().unwrap().len(),
        kicks,
        "an empty pass kicks no lane"
    );
}

#[test]
fn clear_hint_skips_writer_turn() {
    let mut fx = LaneFx::new("apply-hint", 1);
    fx.start();
    fx.wait_idle(1);
    assert!(!fx.store.take_apply_hint(), "the first drain cleared it");
    // A local-only authoring commit applies inline and leaves the hint clear.
    let (ctx, conn, log, _) = open_journal(fx.path.clone(), fx.clock.clone(), 1, fx.registry);
    let mut local = Source { ctx, conn, log };
    local.put_all(0, 1);
    assert!(
        !fx.store.take_apply_hint(),
        "authoring leaves the hint clear"
    );
    let turns = fx.writer.turns();
    fx.pacer.kick();
    fx.wait_idle(2);
    assert_eq!(
        fx.writer.turns(),
        turns,
        "a kicked pass with a clear hint takes no writer turn"
    );
}

#[test]
fn ingest_sets_hint_only_for_stored_rows() {
    let fx = LaneFx::new("apply-ingest-hint", 1);
    assert!(fx.store.take_apply_hint(), "set at open, before any pass");
    assert!(!fx.store.take_apply_hint());
    let mut source = fx.source("src", 2);
    source.put_all(0, 2);
    fx.ship(&source);
    assert!(fx.store.take_apply_hint(), "stored pending rows set it");
    // The same entries again are duplicates: nothing stored, hint stays clear.
    fx.ship(&source);
    assert!(!fx.store.take_apply_hint());
}

#[test]
fn lane_fault_degrades_health_then_recovers() {
    let mut fx = LaneFx::new("apply-health", 1);
    let faulted = Arc::new(std::sync::atomic::AtomicBool::new(true));
    fx.fault_while(&faulted);
    let mut source = fx.source("src", 2);
    source.put_all(0, 2);
    fx.ship(&source);
    let (replayed_tx, replayed_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let first = std::sync::atomic::AtomicBool::new(true);
    // Hold the first successful callback after health/backoff publication,
    // before the worker advances from startup replay to pending entries.
    fx.start_with(Box::new(move |progress| {
        if first.swap(false, Ordering::SeqCst) {
            replayed_tx.send(*progress).unwrap();
            // Dropping the sender on an assertion failure also releases us.
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
        }
    }));
    wait_until("the failure", || {
        fx.pacer.attempts() == 1 && fx.status.health().is_some()
    });
    let summary = fx.status.health().unwrap().summary();
    assert!(summary.starts_with("lane apply failed:"), "{summary}");
    assert!(summary.contains("; retrying (attempt 1,"), "{summary}");
    assert!(!summary.contains('\n'), "Health keeps one line: {summary}");
    assert!(
        fx.calls.lock().unwrap().is_empty(),
        "post_commit never runs on a failed pass"
    );
    assert_eq!(fx.applied_puts(), 0);
    faulted.store(false, Ordering::SeqCst);
    let due = fx.pacer.next_retry_at().unwrap().0 - fx.pacer.now().0;
    fx.advance(due);
    let progress = replayed_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let replay_succeeded = fx.pacer.attempts() == 0 && fx.status.last_tick().is_some();
    let applied_at_replay = fx.applied_puts();
    release_tx.send(()).unwrap();
    assert!(
        replay_succeeded,
        "startup replay already cleared the failure"
    );
    assert_eq!(progress.applied, 0);
    assert_eq!(
        applied_at_replay, 0,
        "startup replay does not drain pending puts"
    );
    // Health/last_tick describe each successful pass, including startup replay.
    // Wait for the retry's actual effect before asserting recovery.
    wait_until("the retry to apply the backlog", || fx.applied_puts() == 2);
    assert!(fx.status.health().is_none());
}

#[test]
fn post_commit_called_with_self_tag_k_never_on_failure() {
    let (lo, hi) = colliding_seeds();
    let mut fx = LaneFx::new("apply-post-commit", hi);
    let source = fx.source("src", lo);
    let faulted = Arc::new(std::sync::atomic::AtomicBool::new(true));
    fx.fault_while(&faulted);
    fx.ship(&source);
    fx.start();
    wait_until("the failure", || fx.pacer.attempts() == 1);
    assert!(fx.calls.lock().unwrap().is_empty(), "no call on failure");
    faulted.store(false, Ordering::SeqCst);
    let due = fx.pacer.next_retry_at().unwrap().0 - fx.pacer.now().0;
    fx.advance(due);
    wait_until("the retag", || {
        fx.calls
            .lock()
            .unwrap()
            .iter()
            .any(|progress| progress.self_tag_k == Some(1))
    });
    assert_eq!(
        fx.count("SELECT tag_k FROM local_node"),
        1,
        "the retag committed before post_commit saw it"
    );
}

#[test]
fn checkpoint_after_1024_applied() {
    let mut fx = LaneFx::new("apply-checkpoint", 1);
    fx.start();
    fx.wait_idle(1);
    let mut source = fx.source("src", 2);
    source.put_all(0, 1100);
    fx.ship(&source);
    wait_until("the entries to apply", || fx.applied_puts() == 1100);
    wait_until("the checkpoint", || fx.store.checkpoints_run() >= 1);
    assert_eq!(
        fx.store.checkpoints_run(),
        1,
        "one checkpoint; the count restarts after it"
    );
}

#[test]
fn worker_stops_on_factory_stop() {
    let mut fx = LaneFx::new("apply-stop", 1);
    fx.start();
    fx.wait_idle(1);
    fx.cancel.cancel();
    let handle = fx.handle.take().unwrap();
    wait_until("the worker to finish", || handle.is_finished());
    handle.join().unwrap();
}

/// A held foreign log, started lane, nothing applied yet.
fn held_fx(label: &str) -> LaneFx {
    let mut fx = LaneFx::new(label, 1);
    fx.hold().set(HoldSource::Links, true, fx.pacer.now());
    let mut source = fx.source("src", 2);
    source.put_all(0, 2);
    fx.ship(&source);
    fx.start();
    fx.wait_idle(1);
    assert_eq!(
        fx.applied_puts(),
        0,
        "held: foreign log, flag set, under the cap"
    );
    fx
}

#[test]
fn held_foreign_entries_apply_at_the_hold_cap_without_new_ingest() {
    let fx = held_fx("apply-hold-cap");
    fx.advance(OWN_HISTORY_HOLD_MAX);
    wait_until("the cap to lift the hold", || fx.applied_puts() == 2);
    assert_eq!(fx.hold().expiry_logs(), 1);
}

fn epochs_of(fx: &LaneFx) -> (u64, u64) {
    let epochs = fx.store.context().sync().unwrap().apply_epochs().clone();
    (epochs.ingested(), epochs.drained())
}

// Kills: an ingest that does not move the epoch, and a drain that records
// before the rows are applied.
#[test]
fn ingest_then_drain_advances_the_drained_epoch() {
    let mut fx = LaneFx::new("apply-epochs", 1);
    assert_eq!(epochs_of(&fx), (1, 0), "startup epoch, nothing drained");
    let mut source = fx.source("src", 2);
    source.put_all(0, 3);
    fx.ship(&source);
    assert_eq!(epochs_of(&fx), (2, 0), "stored rows moved ingested only");
    fx.ship(&source);
    assert_eq!(epochs_of(&fx), (2, 0), "duplicates store nothing");
    fx.start();
    wait_until("the lane to record the drain", || epochs_of(&fx) == (2, 2));
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM journal WHERE apply_state<>'applied'"),
        0,
        "the drain was recorded with nothing pending"
    );
}

// Kills: a drain recorded by a pass that left foreign heads pending under the
// own-history hold.
#[test]
fn a_pass_under_the_own_history_hold_is_not_a_drain() {
    let fx = held_fx("apply-epochs-hold");
    let (ingested, drained) = epochs_of(&fx);
    assert!(
        drained < ingested,
        "held pass is not a drain: {ingested} {drained}"
    );
    fx.advance(OWN_HISTORY_HOLD_MAX);
    wait_until("the cap to lift the hold", || fx.applied_puts() == 2);
    wait_until("the drain to be recorded", || {
        let (i, d) = epochs_of(&fx);
        d == i
    });
}

#[test]
fn hold_wait_is_bounded_by_the_remaining_cap() {
    let hold = OwnHistoryHold::new(Arc::default());
    assert_eq!(hold.remaining(MonoInstant(5)), None);
    hold.set(HoldSource::Links, true, MonoInstant(1_000));
    assert_eq!(
        hold.remaining(MonoInstant(61_000)),
        Some(Duration::from_millis(OWN_HISTORY_HOLD_MAX - 60_000))
    );
    assert!(!hold.active(MonoInstant(1_000 + OWN_HISTORY_HOLD_MAX)));
    assert_eq!(
        hold.remaining(MonoInstant(1_000 + OWN_HISTORY_HOLD_MAX)),
        None
    );
    hold.set(HoldSource::Links, false, MonoInstant(400_000));
    hold.set(HoldSource::Branches, true, MonoInstant(500_000));
    assert_eq!(
        hold.remaining(MonoInstant(500_000)),
        Some(Duration::from_millis(OWN_HISTORY_HOLD_MAX))
    );
}
