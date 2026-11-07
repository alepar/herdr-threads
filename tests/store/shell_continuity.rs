//! TRUST-POLICY C1 shell continuity (ht-ma6i.1) against the real store: the gaps, the fingerprint
//! write, the candidates and the decision. Mounted from `src/store/seats/continuity.rs`. The lane
//! around them is in `tests/service/observation_lane.rs`, the deletes at un-retire and at final
//! retirement of a reversible seat in `tests/store/reversible_retirement.rs`.
use crate::{
    identity::{reconcile::plan_page, shell_continuity::ShellFingerprint},
    ports::{
        DurableWorkAdmission, EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostId,
        HostObservation, HostSnapshot, HostUiState, IncarnationEvidence, ObservationProvenance,
        OrdinaryResolutionAttempt, OrdinaryResolutionGuard, OrdinaryResolutionOutcome,
        PublishedSnapshot, ReconciliationOutcome, ShellContinuityCandidate, ShellContinuityGuard,
        ShellContinuityRequest, ShellFingerprintGap, SnapshotHeader, StructuralOccupancy,
    },
    protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::{OperatorRetire, ResolveSeat},
        ids::{
            ContinuityKey, HostBootId, HostCallId, HostTargetId, OperationId, SeatId, TerminalId,
        },
        results::{ApiError, ContinuityConflictKind, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{connection::StoreContext, continuity_keys, seats},
};
use rusqlite::{Connection, params};
use std::sync::Arc;

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
/// The kernel boot id the fixture's shells are recorded under.
const KERNEL_BOOT: &str = "kernel-boot-a";

fn shell() -> ShellFingerprint {
    ShellFingerprint {
        pid: 4242,
        start: 7,
        session: 4242,
        tty: 34817,
        kernel_boot: KERNEL_BOOT.into(),
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

struct Fx {
    path: std::path::PathBuf,
    context: StoreContext,
    conn: Connection,
    host: HostId,
    sequence: u64,
    operation: u64,
    boot: &'static str,
    incarnation: &'static str,
}

impl Fx {
    /// A published coherent snapshot of `panes`, its pass recorded.
    fn new(panes: &[&str]) -> Self {
        let path = crate::test_support::scratch::temp_path(format!(
            "herdr-shell-continuity-{}.db",
            uuid::Uuid::new_v4()
        ));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host();
        let mut conn = context.open_writer().unwrap();
        crate::store::journal::open(
            &context,
            &mut conn,
            Arc::new(crate::node::NodeIdentity::from_seed(
                crate::test_support::TEST_NODE_SEED,
            )),
            "test",
            crate::sync::facts::FactRegistry::builtins(),
        )
        .unwrap();
        conn.execute("UPDATE herdr_hosts SET host_boot='b1',host_epoch=1", [])
            .unwrap();
        let mut fx = Self {
            path,
            context,
            conn,
            host: crate::test_support::test_host_id(),
            sequence: 0,
            operation: 0,
            boot: "b1",
            incarnation: "inc-1",
        };
        fx.publish_and_pass(panes);
        fx
    }

    fn observation(&mut self, pane: &str, provenance: ObservationProvenance) -> HostObservation {
        self.sequence += 1;
        HostObservation {
            focused: false,
            target: HostTargetId::new(pane),
            host_boot: HostBootId::new(self.boot),
            epoch: 1,
            generation: 1,
            observed_at_utc: UtcMillis(100),
            observed_at_mono: MonoInstant(100),
            provenance,
            occupant: None,
            ui: HostUiState::Idle,
            // A new incarnation gives each terminal a new id.
            terminal: Some(TerminalId::new(format!("term-{pane}-{}", self.incarnation))),
            occupancy: StructuralOccupancy::EmptyShell,
            incarnation: IncarnationEvidence::Verified {
                identity: self.incarnation.into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("call-{pane}-{}", self.sequence)),
            connection_epoch: 1,
            observation_sequence: self.sequence,
            started_at_mono: MonoInstant(100),
            completed_at_mono: MonoInstant(100),
        }
    }

    fn publish(&mut self, panes: &[&str]) -> PublishedSnapshot {
        let targets: Vec<_> = panes
            .iter()
            .map(|pane| self.observation(pane, ObservationProvenance::CoherentEnumeration))
            .collect();
        let capture = HostSnapshot {
            boot: HostBootId::new(self.boot),
            epoch: 1,
            observation_sequence: self.sequence,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: self.incarnation.into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets,
        };
        let budget = budget();
        let admission = seats::begin_host_observation(
            &self.context,
            &mut self.conn,
            INSTANCE,
            self.host,
            &budget,
        )
        .unwrap();
        let header = SnapshotHeader::from_captured(admission, &capture).unwrap();
        let stage =
            seats::begin_snapshot_stage(&self.context, &mut self.conn, header, &budget).unwrap();
        let (context, conn) = (&self.context, &mut self.conn);
        crate::test_support::snapshot::stage_all(&capture.targets, |offset, targets| {
            seats::stage_snapshot_targets(
                context,
                conn,
                &stage.id,
                offset,
                targets,
                DurableWorkAdmission::new(16).unwrap(),
                &budget,
            )
        })
        .unwrap();
        seats::seal_snapshot_stage(&self.context, &mut self.conn, &stage.id, &budget).unwrap();
        seats::publish_snapshot_stage(&self.context, &mut self.conn, &stage.id, &budget).unwrap()
    }

    /// One reconciliation pass over the publication, its marker recorded; every outcome.
    fn publish_and_pass(&mut self, panes: &[&str]) -> Vec<ReconciliationOutcome> {
        let published = self.publish(panes);
        let budget = budget();
        let page = seats::saved_seats_page(
            &self.context,
            &self.conn,
            &published.id,
            0,
            None,
            16,
            &budget,
        )
        .unwrap();
        let outcomes = plan_page(&page)
            .unwrap()
            .into_iter()
            .map(|transition| {
                seats::apply_reconciliation_transition(
                    &self.context,
                    &mut self.conn,
                    transition,
                    &budget,
                )
                .unwrap()
            })
            .collect();
        seats::record_reconciliation_pass(&self.context, &mut self.conn, &published, &budget)
            .unwrap();
        outcomes
    }

    /// The live handoff as the store sees it: a new server and terminal ids, the same panes.
    fn handoff(&mut self, panes: &[&str]) {
        self.boot = "b2";
        self.incarnation = "inc-2";
        self.publish_and_pass(panes);
    }

    /// One explicit current-target read of `pane`, published.
    fn read(&mut self, pane: &str) -> (HostObservation, crate::ports::HostObservationAdmission) {
        let admission = seats::begin_host_observation(
            &self.context,
            &mut self.conn,
            INSTANCE,
            self.host,
            &budget(),
        )
        .unwrap();
        let fresh = self.observation(pane, ObservationProvenance::FreshCurrentTarget);
        assert!(
            seats::publish_current_target_observation(
                &self.context,
                &mut self.conn,
                &admission,
                &fresh,
                &budget(),
            )
            .unwrap()
        );
        (fresh, admission)
    }

    fn resolve(&mut self, pane: &str) -> SeatId {
        self.operation += 1;
        let request = ResolveSeat {
            host: crate::test_support::test_host_locator(),
            target: HostTargetId::new(pane),
            operation: OperationId::new(format!("op-{}", self.operation)),
            locator_hint: None,
            tmux_hint: None,
            continuity_key: None,
        };
        let (fresh, admission) = self.read(pane);
        let guard = OrdinaryResolutionGuard::try_new(&request, fresh, &admission).unwrap();
        match seats::resolve_seat(
            &self.context,
            &mut self.conn,
            INSTANCE,
            request,
            OrdinaryResolutionAttempt::Observed(guard),
            &budget(),
        )
        .unwrap()
        {
            OrdinaryResolutionOutcome::Resolved(seat) => seat,
            OrdinaryResolutionOutcome::NeedsObservation => panic!("observed attempt"),
        }
    }

    fn gaps(&self) -> Vec<ShellFingerprintGap> {
        seats::shell_fingerprint_gaps(&self.context, INSTANCE, self.host, None, &budget()).unwrap()
    }

    fn candidates(&self) -> Vec<ShellContinuityCandidate> {
        seats::shell_continuity_candidates(&self.context, INSTANCE, self.host, None, &budget())
            .unwrap()
    }

    /// The seat's gap, recorded with `shell`.
    fn record(&self, seat: &SeatId, shell: &ShellFingerprint) {
        let gap = self
            .gaps()
            .into_iter()
            .find(|gap| gap.seat == *seat)
            .expect("the seat has a gap");
        assert!(seats::record_shell_fingerprint(&self.conn, &gap, shell, UtcMillis(100)).unwrap());
    }

    /// A bare seat at `pane` with its fingerprint recorded.
    fn seat_with_row(&mut self, pane: &str, shell: &ShellFingerprint) -> SeatId {
        let seat = self.resolve(pane);
        self.record(&seat, shell);
        seat
    }

    /// The decision for the listed candidate of `seat`, against a fresh read.
    fn decide(&mut self, seat: &SeatId) -> Result<(), ApiError> {
        let candidate = self
            .candidates()
            .into_iter()
            .find(|candidate| candidate.seat == *seat)
            .expect("the seat is a candidate");
        let (fresh, _) = self.read(candidate.target.as_str());
        let guard =
            ShellContinuityGuard::try_new(INSTANCE, self.host, &candidate.target, fresh).unwrap();
        seats::decide_shell_continuity(
            &self.context,
            &mut self.conn,
            INSTANCE,
            ShellContinuityRequest {
                seat: candidate.seat,
                seat_generation: candidate.seat_generation,
                fingerprint: candidate.fingerprint,
                guard,
            },
        )
        .map(|_| ())
    }

    fn scalar(&self, sql: &str) -> i64 {
        self.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn state(&self, seat: &SeatId) -> (String, i64) {
        self.conn
            .query_row(
                "SELECT state,generation FROM seats WHERE id=?1",
                [seat.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn kinds(&self, seat: &SeatId) -> Vec<String> {
        self.conn
            .prepare("SELECT kind FROM allocation_decisions WHERE seat_id=?1 ORDER BY ordinal")
            .unwrap()
            .query_map([seat.as_str()], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn binding(&self, seat: &SeatId, harness: &str, provenance: &str) {
        self.conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,ended_at) VALUES (?1,1,1,'old-pane','b1',1,?2,'session-'||?1,'exec-'||?1,?3,10,10,50)",
            params![seat.as_str(), harness, provenance],
        )
        .unwrap();
    }
}

fn gap_seats(fx: &Fx) -> Vec<SeatId> {
    fx.gaps().into_iter().map(|gap| gap.seat).collect()
}

/// Kills: listing a seat with a current row, an unresolved seat, a seat proved on a dead
/// incarnation; never listing one whose terminal moved past its row; a write past a moved seat.
#[test]
fn gaps_list_only_resolved_herdr_seats_with_no_current_row() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.resolve("p1");
    assert_eq!(gap_seats(&fx), vec![seat.clone()]);
    let gap = fx.gaps().remove(0);
    assert_eq!(gap.target.as_str(), "p1");
    assert_eq!(gap.terminal.as_str(), "term-p1-inc-1");

    // A stale listing writes nothing once the seat moved on.
    fx.conn
        .execute(
            "UPDATE seats SET generation=generation+1 WHERE id=?1",
            [seat.as_str()],
        )
        .unwrap();
    assert!(!seats::record_shell_fingerprint(&fx.conn, &gap, &shell(), UtcMillis(100)).unwrap());
    assert_eq!(fx.scalar("SELECT count(*) FROM seat_shell_fingerprints"), 0);

    fx.record(&seat, &shell());
    assert_eq!(
        fx.conn
            .query_row(
                "SELECT pid,start,session,tty,incarnation,terminal_id,kernel_boot FROM seat_shell_fingerprints",
                [],
                |r| Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, u32>(2)?,
                    r.get::<_, i64>(3)? as u64,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                )),
            )
            .unwrap(),
        (
            shell().pid,
            shell().start,
            shell().session,
            shell().tty,
            "inc-1".to_owned(),
            "term-p1-inc-1".to_owned(),
            KERNEL_BOOT.to_owned()
        )
    );
    assert!(gap_seats(&fx).is_empty(), "a current row is never listed");

    // A new terminal under the same seat: the row is no longer current, and is replaced.
    fx.conn
        .execute(
            "UPDATE seats SET structural_terminal_id='term-next' WHERE id=?1",
            [seat.as_str()],
        )
        .unwrap();
    assert_eq!(gap_seats(&fx), vec![seat.clone()]);
    fx.record(&seat, &shell());
    assert_eq!(fx.scalar("SELECT count(*) FROM seat_shell_fingerprints"), 1);
    assert!(gap_seats(&fx).is_empty());

    // A resolved seat a refused transition left on a dead incarnation is not listed.
    let other = fx.resolve("p2");
    assert_eq!(gap_seats(&fx), vec![other.clone()]);
    fx.conn
        .execute(
            "UPDATE seats SET structural_incarnation='inc-dead' WHERE id=?1",
            [other.as_str()],
        )
        .unwrap();
    assert!(gap_seats(&fx).is_empty());

    // An unresolved seat has nothing to read.
    fx.conn
        .execute(
            "UPDATE seats SET structural_incarnation='inc-1',state='unresolved',unresolved_reason='other' WHERE id=?1",
            [other.as_str()],
        )
        .unwrap();
    assert!(gap_seats(&fx).is_empty());
}

/// The page is bounded and the cursor skips what is behind it. Kills: an unbounded page, which
/// makes a Herdr read per seat per pass; a cursor that is ignored, which starves later seats.
#[test]
fn gaps_are_bounded_and_page_from_the_cursor() {
    let panes: Vec<String> = (0..18).map(|n| format!("p{n:02}")).collect();
    let refs: Vec<&str> = panes.iter().map(String::as_str).collect();
    let mut fx = Fx::new(&refs);
    for pane in &refs {
        fx.resolve(pane);
    }
    let first = gap_seats(&fx);
    assert_eq!(first.len(), crate::ports::SHELL_FINGERPRINT_READS_PER_PASS);
    let rest =
        seats::shell_fingerprint_gaps(&fx.context, INSTANCE, fx.host, first.last(), &budget())
            .unwrap();
    assert_eq!(rest.len(), 2);
    assert!(rest.iter().all(|gap| gap.seat > *first.last().unwrap()));
}

/// Kills: a fingerprint for a named seat, or on a herdr-core host, whose continuity is not a pane.
#[test]
fn named_and_hc_never_get_a_row() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.resolve("p1");
    assert_eq!(gap_seats(&fx).len(), 1);
    fx.conn
        .execute(
            "UPDATE seats SET name='reviewer' WHERE id=?1",
            [seat.as_str()],
        )
        .unwrap();
    assert!(gap_seats(&fx).is_empty(), "a named seat");
    fx.conn
        .execute("UPDATE seats SET name=NULL WHERE id=?1", [seat.as_str()])
        .unwrap();
    assert_eq!(gap_seats(&fx).len(), 1);
    for kind in ["hc", "named"] {
        fx.conn
            .execute("UPDATE herdr_hosts SET kind=?1", [kind])
            .unwrap();
        assert!(gap_seats(&fx).is_empty(), "a host of kind {kind}");
    }
}

/// Kills: opening a binding; leaving the hold; a kind other than the shell's; no proof; a seat that
/// stays unresolved. The generation moves by one and the decision is recorded once.
#[test]
fn decision_resolves_releases_hold_opens_no_binding_and_records_kind() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.seat_with_row("p1", &shell());
    fx.handoff(&["p1"]);
    let (state, generation) = fx.state(&seat);
    assert_eq!(state, "unresolved");
    fx.conn
        .execute(
            "INSERT OR IGNORE INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,'p1','b2',1,'unresolved saved seat continuity')",
            [fx.host.get()],
        )
        .unwrap();
    assert!(
        fx.scalar(
            "SELECT count(*) FROM recovery_holds WHERE target_id='p1' AND released_at IS NULL"
        ) > 0
    );
    let bindings = fx.scalar("SELECT count(*) FROM occupant_bindings");
    fx.decide(&seat).unwrap();
    assert_eq!(fx.state(&seat), ("resolved".to_owned(), generation + 1));
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM recovery_holds WHERE target_id='p1' AND released_at IS NULL"
        ),
        0,
        "the pane's hold is released"
    );
    assert_eq!(
        fx.scalar("SELECT count(*) FROM occupant_bindings"),
        bindings,
        "no binding opened"
    );
    assert_eq!(
        fx.scalar("SELECT count(*) FROM occupant_bindings WHERE ended_at IS NULL"),
        0
    );
    assert_eq!(
        fx.kinds(&seat).last().map(String::as_str),
        Some("structural_shell_continuity")
    );
    let (terminal, incarnation): (String, String) = fx
        .conn
        .query_row(
            "SELECT structural_terminal_id,structural_incarnation FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(terminal, "term-p1-inc-2");
    assert_eq!(incarnation, "inc-2");
    assert!(fx.candidates().is_empty(), "no longer a candidate");
    assert_eq!(
        gap_seats(&fx),
        vec![seat.clone()],
        "the new incarnation's fingerprint is due, for the next handoff"
    );
}

/// Kills: resolving a seat with no structural proof, which the next pass would unresolve again.
#[test]
fn next_pass_reconfirms_rather_than_unresolves() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.seat_with_row("p1", &shell());
    fx.handoff(&["p1"]);
    fx.decide(&seat).unwrap();
    let decided = fx.state(&seat);
    assert_eq!(decided.0, "resolved");
    for _ in 0..2 {
        let outcomes = fx.publish_and_pass(&["p1"]);
        assert!(
            outcomes.iter().all(|outcome| !matches!(
                outcome,
                ReconciliationOutcome::Stale(_) | ReconciliationOutcome::RetirementStarted(_)
            )),
            "{outcomes:?}"
        );
        assert_eq!(fx.state(&seat), decided, "the pass moved the seat");
    }
}

/// Kills: deciding past a kept claim, which is a session's own word and decided first.
#[test]
fn kept_claim_blocks_the_decision() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.seat_with_row("p1", &shell());
    fx.handoff(&["p1"]);
    fx.conn.execute(
        "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) VALUES (?1,'p1','claude','kept','{}',100)",
        [fx.host.get()],
    )
    .unwrap();
    let refused = fx.decide(&seat).unwrap_err();
    assert_eq!(refused.code, ErrorCode::ServiceBusy, "{refused:?}");
    assert_eq!(fx.state(&seat).0, "unresolved");
    assert!(
        fx.kinds(&seat)
            .iter()
            .all(|k| k != "structural_shell_continuity")
    );
}

/// Kills: taking a pane that has a resolved owner.
#[test]
fn resolved_owner_blocks_the_decision() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.seat_with_row("p1", &shell());
    let owner = fx.resolve("p2");
    fx.handoff(&["p1", "p2"]);
    fx.conn
        .execute(
            "UPDATE seats SET state='resolved',unresolved_reason=NULL,target_id='p1' WHERE id=?1",
            [owner.as_str()],
        )
        .unwrap();
    let refused = fx.decide(&seat).unwrap_err();
    assert_eq!(refused.code, ErrorCode::TargetAlreadyOwned, "{refused:?}");
    assert_eq!(fx.state(&seat).0, "unresolved");
}

/// Kills: reading a person seat as a bare one, or a person seat's older agent binding as its last.
#[test]
fn person_seat_is_no_candidate() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let person = fx.seat_with_row("p1", &shell());
    fx.binding(&person, "human", "cooperative_top_level");
    let later_agent = fx.seat_with_row(
        "p2",
        &ShellFingerprint {
            pid: 5151,
            session: 5151,
            ..shell()
        },
    );
    fx.binding(&later_agent, "human", "cooperative_top_level");
    fx.binding(&later_agent, "codex", "cooperative_top_level");
    fx.handoff(&["p1", "p2"]);
    let candidates: Vec<SeatId> = fx.candidates().into_iter().map(|c| c.seat).collect();
    assert_eq!(candidates, vec![later_agent], "the newest binding decides");
}

/// An exited agent and a launch that never checked in are the shell path's own cases. Kills: a
/// binding filter copied from process continuity, which would hide them.
#[test]
fn an_exited_agent_or_managed_launch_only_seat_is_a_candidate() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let exited = fx.seat_with_row("p1", &shell());
    fx.binding(&exited, "codex", "cooperative_top_level");
    let launched = fx.seat_with_row(
        "p2",
        &ShellFingerprint {
            pid: 5151,
            session: 5151,
            ..shell()
        },
    );
    fx.binding(&launched, "codex", "managed_launch");
    fx.handoff(&["p1", "p2"]);
    fx.decide(&exited).unwrap();
    fx.decide(&launched).unwrap();
    assert_eq!(fx.state(&exited).0, "resolved");
    assert_eq!(fx.state(&launched).0, "resolved");
}

/// Same-incarnation unresolve: nothing proves anything until a new incarnation. Kills: dropping the
/// incarnation filter, which makes any unresolved seat with a live shell move back on its own.
#[test]
fn a_seat_unresolved_in_the_same_incarnation_is_no_candidate() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.seat_with_row("p1", &shell());
    fx.conn
        .execute(
            "UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id=?1",
            [seat.as_str()],
        )
        .unwrap();
    assert!(fx.candidates().is_empty());
}

/// Kills: a conflict row that blocks this path, or one left standing after it.
#[test]
fn key_conflict_row_does_not_block_and_is_cleared() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.seat_with_row("p1", &shell());
    fx.handoff(&["p1"]);
    continuity_keys::record_conflict(
        &fx.conn,
        &continuity_keys::ConflictFacts {
            host: fx.host,
            target: "p1".into(),
            key: ContinuityKey::new("another"),
            kind: ContinuityConflictKind::KeyLiveElsewhere,
            seat: None,
            owner: None,
        },
        UtcMillis(5),
    )
    .unwrap();
    assert_eq!(
        fx.scalar("SELECT count(*) FROM continuity_key_conflicts"),
        1
    );
    fx.decide(&seat).unwrap();
    assert_eq!(fx.state(&seat).0, "resolved");
    assert_eq!(
        fx.scalar("SELECT count(*) FROM continuity_key_conflicts"),
        0
    );
}

/// Kills: a decision on evidence that fits two seats, or the listing's check alone.
#[test]
fn a_rival_fingerprint_blocks_the_decision() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.seat_with_row("p1", &shell());
    let rival = fx.seat_with_row("p2", &shell());
    fx.handoff(&["p1", "p2"]);
    assert!(seats::shell_rival_exists(&fx.conn, &seat, &shell()).unwrap());
    let later = ShellFingerprint {
        start: shell().start + 1,
        ..shell()
    };
    assert!(
        !seats::shell_rival_exists(&fx.conn, &seat, &later).unwrap(),
        "another start stamp is another process"
    );
    let refused = fx.decide(&seat).unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict, "{refused:?}");
    assert_eq!(fx.state(&seat).0, "unresolved");
    assert_eq!(fx.state(&rival).0, "unresolved");
}

/// Kills: a row that outlives a final retirement, by the local writer and the replicated one.
#[test]
fn row_goes_at_final_retirement_and_unretire() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.seat_with_row("p1", &shell());
    retire(&mut fx, &seat, "op-retire");
    assert_eq!(
        fx.scalar("SELECT count(*) FROM seat_shell_fingerprints"),
        0,
        "the local writer"
    );

    let other = fx.seat_with_row("p2", &shell());
    let tx = fx.conn.transaction().unwrap();
    crate::store::control::insert_replicated_retirement_job(&tx, &other, UtcMillis(100)).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        fx.scalar("SELECT count(*) FROM seat_shell_fingerprints"),
        0,
        "the replicated writer"
    );
    // Un-retire: `row_goes_at_unretire` in `tests/store/reversible_retirement.rs`.
}

fn retire(fx: &mut Fx, seat: &SeatId, operation: &str) {
    seats::mutate_operator(
        &fx.context,
        &mut fx.conn,
        INSTANCE,
        crate::ports::OperatorRequest::Retire(OperatorRetire {
            seat: seat.clone(),
            operation: OperationId::new(operation),
        }),
        OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap(),
        None,
    )
    .unwrap();
}

/// A retired row, whatever its kind, names no rival. Kills: a rival query with no seat state, which
/// would strand the live seat in the same shell. A reversible seat keeps its row:
/// `reversible_seat_row_is_no_rival` in `tests/store/reversible_retirement.rs`.
#[test]
fn retired_seat_row_is_no_rival() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.seat_with_row("p1", &shell());
    let retired = fx.seat_with_row("p2", &shell());
    assert!(seats::shell_rival_exists(&fx.conn, &seat, &shell()).unwrap());
    // Retiring deletes the row; one left behind by any other way must still not count.
    retire(&mut fx, &retired, "op-retire");
    fx.conn
        .execute(
            "INSERT INTO seat_shell_fingerprints(seat_id,host_id,target_id,host_boot,incarnation,terminal_id,pid,start,session,tty,kernel_boot,recorded_at) \
             SELECT ?1,host_id,'p2','b1','inc-1','t',?2,?3,?2,?4,?5,1 FROM herdr_hosts",
            params![
                retired.as_str(),
                shell().pid,
                shell().start as i64,
                shell().tty as i64,
                KERNEL_BOOT
            ],
        )
        .unwrap();
    assert!(!seats::shell_rival_exists(&fx.conn, &seat, &shell()).unwrap());
}

/// `--replace` and `seat retire` leave the pane's shell to a new seat. Kills: the old seat's row
/// standing as a rival, or the old seat itself coming back.
#[test]
fn replaced_seat_in_a_surviving_shell_recovers_on_next_handoff() {
    let mut fx = Fx::new(&["p1"]);
    let old = fx.seat_with_row("p1", &shell());
    retire(&mut fx, &old, "op-retire");
    let new = fx.seat_with_row("p1", &shell());
    fx.handoff(&["p1"]);
    assert_eq!(
        fx.candidates()
            .into_iter()
            .map(|c| c.seat)
            .collect::<Vec<_>>(),
        vec![new.clone()]
    );
    fx.decide(&new).unwrap();
    assert_eq!(fx.state(&new).0, "resolved");
    assert_eq!(fx.state(&old).0, "retired");
}

/// A shell recorded under another kernel boot names no process of this one. Kills: a rival query
/// without the boot, which would strand a seat behind a row a reboot left (its pid and start can
/// come back); a listing or write that drops the recorded boot.
#[test]
fn a_rival_fingerprint_of_another_boot_does_not_block() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.seat_with_row("p1", &shell());
    let before_reboot = ShellFingerprint {
        kernel_boot: "kernel-boot-old".into(),
        ..shell()
    };
    let other = fx.seat_with_row("p2", &before_reboot);
    assert!(!seats::shell_rival_exists(&fx.conn, &seat, &shell()).unwrap());
    assert!(
        seats::shell_rival_exists(&fx.conn, &other, &before_reboot).is_ok_and(|rival| !rival),
        "nor the other way round"
    );
    fx.handoff(&["p1", "p2"]);
    let candidate = fx
        .candidates()
        .into_iter()
        .find(|candidate| candidate.seat == seat)
        .expect("the seat is a candidate");
    assert_eq!(candidate.fingerprint, shell(), "the boot round-trips");
    fx.decide(&seat).unwrap();
    assert_eq!(fx.state(&seat).0, "resolved");
}

/// A process table that knows only its kernel boot id.
struct Booted(&'static str);
impl crate::ports::ProcessInfo for Booted {
    fn process(&self, _pid: u32) -> Option<crate::ports::ProcessRecord> {
        None
    }
    fn kernel_boot(&self) -> Option<String> {
        Some(self.0.to_owned())
    }
}

/// ht-ma6i.44: the store records the daemon's kernel boot id at start, every binding statement
/// that writes a liveness fingerprint stamps it (a NULL fingerprint stamps none), and the C1
/// process continuity listing returns it. Kills: a binding fingerprint with no boot, a refresh
/// that skips a row whose only change is the boot (the collision after a reboot), a boot kept
/// beside a cleared fingerprint, or a listing that drops it.
#[test]
fn bindings_stamp_the_kernel_boot_the_store_recorded() {
    use crate::protocol::authority::LivenessFingerprint;
    use crate::store::bindings;
    let mut fx = Fx::new(&["p1"]);
    drop(
        crate::store::SqliteStore::new(
            StoreContext::new(fx.path.clone(), Arc::new(FixedClock))
                .with_test_host()
                .with_processes(Arc::new(Booted("boot-a"))),
            INSTANCE,
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let recorded: String = fx
        .conn
        .query_row("SELECT boot FROM node_kernel_boot WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(recorded, "boot-a");

    let seat = fx.resolve("p1");
    fx.conn
        .execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,liveness,disconnected_at,disconnect_reason) \
             SELECT id,generation,target_generation,target_id,structural_terminal_id,structural_incarnation,structural_host_boot,1,'claude','sess-1','exec-1','cooperative_top_level',100,100,'disconnected',100,'liveness_process_exit' FROM seats WHERE id=?1",
            [seat.as_str()],
        )
        .unwrap();
    let binding = crate::protocol::ids::BindingId::new(
        fx.conn
            .query_row(
                "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1",
                [seat.as_str()],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
    );
    let boot = |fx: &Fx| -> Option<String> {
        fx.conn
            .query_row(
                "SELECT liveness_boot FROM occupant_bindings WHERE seat_id=?1",
                [seat.as_str()],
                |r| r.get(0),
            )
            .unwrap()
    };
    let disconnect = |fx: &Fx| {
        fx.conn
            .execute(
                "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=100,disconnect_reason='liveness_process_exit' WHERE seat_id=?1",
                [seat.as_str()],
            )
            .unwrap();
    };
    let fingerprint = LivenessFingerprint {
        pid: 4242,
        start: 7,
    };
    let trigger = bindings::ConnectTrigger::Lifecycle;
    assert_eq!(boot(&fx), None);
    assert!(bindings::connect(&fx.conn, &binding, None, trigger, UtcMillis(100)).unwrap());
    assert_eq!(boot(&fx), None, "no fingerprint, no boot");
    disconnect(&fx);
    assert!(
        bindings::connect(
            &fx.conn,
            &binding,
            Some(fingerprint),
            trigger,
            UtcMillis(100)
        )
        .unwrap()
    );
    assert_eq!(boot(&fx).as_deref(), Some("boot-a"), "the reconnect");
    fx.conn
        .execute(
            "UPDATE occupant_bindings SET liveness_boot='boot-old' WHERE seat_id=?1",
            [seat.as_str()],
        )
        .unwrap();
    assert!(bindings::touch(&fx.conn, &binding, Some(fingerprint), UtcMillis(100)).unwrap());
    assert_eq!(
        boot(&fx).as_deref(),
        Some("boot-a"),
        "the same pid and start under another boot are rewritten"
    );
    disconnect(&fx);
    assert!(bindings::connect(&fx.conn, &binding, None, trigger, UtcMillis(100)).unwrap());
    assert_eq!(boot(&fx), None, "a cleared fingerprint clears its boot");
    disconnect(&fx);
    assert!(
        bindings::connect(
            &fx.conn,
            &binding,
            Some(fingerprint),
            trigger,
            UtcMillis(100)
        )
        .unwrap()
    );

    // The shape the latest unresolve leaves (as `tests/store/control.rs` builds it).
    fx.handoff(&["p1"]);
    fx.conn
        .execute(
            "UPDATE seats SET state='unresolved',unresolved_reason='other',target_id='p1' WHERE id=?1",
            [seat.as_str()],
        )
        .unwrap();
    fx.conn
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected',disconnect_reason='seat_unresolved',disconnected_at=100,ended_at=100,incarnation='inc-1',generation=(SELECT generation-1 FROM seats WHERE id=?1) WHERE seat_id=?1",
            [seat.as_str()],
        )
        .unwrap();
    let listing = seats::process_continuity_candidates(
        &fx.context,
        INSTANCE,
        fx.host,
        &crate::ports::ProcessContinuityPage::default(),
        &budget(),
    )
    .unwrap();
    let listed: Vec<_> = listing
        .candidates
        .iter()
        .map(|c| (c.seat.clone(), c.liveness, c.liveness_boot.clone()))
        .collect();
    assert_eq!(
        listed,
        vec![(seat.clone(), fingerprint, Some("boot-a".to_owned()))]
    );
}
