//! TRUST-POLICY C1 host session continuity (ht-iygh D1) against the real store: the candidates,
//! the decision and its guards, the restored binding as a dormant wake target, and the session's own
//! check-in reconnecting it. Mounted from `src/store/seats/continuity.rs`. The lane around them is in
//! `tests/service/observation_lane.rs`.
use crate::{
    identity::reconcile::plan_page,
    ports::{
        DurableWorkAdmission, EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostId,
        HostObservation, HostSessionCandidate, HostSessionContinuityRequest, HostSnapshot,
        HostUiState, IncarnationEvidence, ObservationProvenance, OrdinaryResolutionAttempt,
        OrdinaryResolutionGuard, OrdinaryResolutionOutcome, PublishedSnapshot,
        ReconciliationOutcome, ShellContinuityGuard, SnapshotHeader, StructuralOccupancy,
    },
    protocol::{
        commands::ResolveSeat,
        ids::{
            ContinuityKey, HostBootId, HostCallId, HostTargetId, OperationId, SeatId, TerminalId,
        },
        results::{ApiError, CommandResult, ContinuityConflictKind, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{connection::StoreContext, continuity_keys, lazy_delivery, seats},
};
use rusqlite::{Connection, params};
use std::sync::Arc;

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

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
            "herdr-host-session-continuity-{}.db",
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

    fn candidates(&self) -> Vec<HostSessionCandidate> {
        seats::host_session_continuity_candidates(
            &self.context,
            INSTANCE,
            self.host,
            None,
            &budget(),
        )
        .unwrap()
    }

    fn listed(&self) -> Vec<SeatId> {
        self.candidates().into_iter().map(|c| c.seat).collect()
    }

    /// An open connected top-level binding of `seat`, placed where the seat is now, as a hook
    /// check-in leaves it.
    fn agent(&self, seat: &SeatId, harness: &str, session: &str, provenance: &str) {
        self.conn
            .execute(
                "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) \
                 SELECT id,generation,target_generation,target_id,structural_terminal_id,structural_incarnation,structural_host_boot,1,?2,?3,'exec-'||?3,?4,100,100 FROM seats WHERE id=?1",
                params![seat.as_str(), harness, session, provenance],
            )
            .unwrap();
    }

    /// A codex seat at `pane` whose session is `session`.
    fn codex_seat(&mut self, pane: &str, session: &str) -> SeatId {
        let seat = self.resolve(pane);
        self.agent(&seat, "codex", session, "cooperative_top_level");
        seat
    }

    /// The request for `seat`'s listed candidate and `session`, against a fresh read.
    fn request(&mut self, seat: &SeatId, session: &str) -> HostSessionContinuityRequest {
        let candidate = self
            .candidates()
            .into_iter()
            .find(|candidate| candidate.seat == *seat)
            .expect("the seat is a candidate");
        let (harness, session) = candidate
            .sessions
            .iter()
            .find(|(_, bound)| bound == session)
            .cloned()
            .expect("the session is listed");
        self.request_for(
            seat,
            candidate.seat_generation,
            &candidate.target,
            harness,
            session,
        )
    }

    fn request_for(
        &mut self,
        seat: &SeatId,
        generation: u64,
        target: &HostTargetId,
        harness: String,
        session: String,
    ) -> HostSessionContinuityRequest {
        let (fresh, _) = self.read(target.as_str());
        HostSessionContinuityRequest {
            seat: seat.clone(),
            seat_generation: generation,
            harness,
            native_session: session,
            guard: ShellContinuityGuard::try_new(INSTANCE, self.host, target, fresh).unwrap(),
        }
    }

    fn submit(&mut self, request: HostSessionContinuityRequest) -> Result<CommandResult, ApiError> {
        seats::decide_host_session_continuity(&self.context, &mut self.conn, INSTANCE, request)
    }

    fn decide(&mut self, seat: &SeatId, session: &str) -> Result<CommandResult, ApiError> {
        let request = self.request(seat, session);
        self.submit(request)
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

    fn decisions(&self, seat: &SeatId) -> i64 {
        self.conn
            .query_row(
                "SELECT count(*) FROM allocation_decisions WHERE seat_id=?1 AND kind='host_session_continuity'",
                [seat.as_str()],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// The seat's open bindings: (binding, liveness, reason, provenance, registered, fingerprinted).
    fn open(&self, seat: &SeatId) -> Vec<(String, String, Option<String>, String, bool, bool)> {
        self.conn
            .prepare("SELECT binding_id,liveness,disconnect_reason,observation_provenance,registered_at IS NOT NULL,liveness_pid IS NOT NULL FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal")
            .unwrap()
            .query_map([seat.as_str()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

/// Kills: listing a keyed, named or person seat, a seat whose only binding is a launch
/// placeholder, one whose binding had disconnected before the unresolve, or one whose saved target
/// the publication does not show; a listing that drops a session or the seat's newest first.
#[test]
fn candidates_list_only_unkeyed_agent_seats_at_a_published_target() {
    let panes = ["p1", "p2", "p3", "p4", "p5", "p6", "p7"];
    let mut fx = Fx::new(&panes);
    let eligible = fx.codex_seat("p1", "sess-a");
    fx.agent(&eligible, "claude", "sess-b", "cooperative_env_session");
    let keyed = fx.codex_seat("p2", "sess-keyed");
    let person = fx.resolve("p3");
    fx.agent(&person, "human", "person", "operator_human");
    let launched = fx.resolve("p4");
    fx.agent(&launched, "codex", "launch:x", "managed_launch");
    let exited = fx.codex_seat("p5", "sess-exited");
    fx.conn
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=90,disconnect_reason='liveness_process_exit' WHERE seat_id=?1",
            [exited.as_str()],
        )
        .unwrap();
    let named = fx.codex_seat("p6", "sess-named");
    let gone = fx.codex_seat("p7", "sess-gone");
    let author = {
        let tx = fx.conn.transaction().unwrap();
        let author =
            crate::store::service_substrate::ensure_reserved_author(&tx, INSTANCE, UtcMillis(0))
                .unwrap();
        tx.commit().unwrap();
        author
    };
    fx.conn
        .execute(
            "INSERT INTO continuity_keys(key,owner_author_id,seat_id,reserved_at,bound_at,bound_by) VALUES ('k',?1,?2,1,1,'service_reservation')",
            params![author.as_str(), keyed.as_str()],
        )
        .unwrap();
    fx.handoff(&["p1", "p2", "p3", "p4", "p5", "p6"]);
    fx.conn
        .execute(
            "UPDATE seats SET name='reviewer' WHERE id=?1",
            [named.as_str()],
        )
        .unwrap();
    for seat in [
        &eligible, &keyed, &person, &launched, &exited, &named, &gone,
    ] {
        assert_eq!(fx.state(seat).0, "unresolved", "{seat:?}");
    }
    let candidates = fx.candidates();
    assert_eq!(
        candidates.iter().map(|c| &c.seat).collect::<Vec<_>>(),
        vec![&eligible]
    );
    assert_eq!(candidates[0].target.as_str(), "p1");
    // Only hook-registered sessions: the plain-CLI `cooperative_env_session` binding has no hook
    // to connect a restored binding (roast diff 1, finding 2).
    assert_eq!(
        candidates[0].sessions,
        vec![("codex".to_owned(), "sess-a".to_owned())]
    );
    // Before the new incarnation nothing is listed: the unresolve's binding must be of an earlier one.
    let mut same = Fx::new(&["p1"]);
    let seat = same.codex_seat("p1", "sess-a");
    same.conn
        .execute(
            "UPDATE occupant_bindings SET ended_at=100,liveness='disconnected',disconnect_reason='seat_unresolved' WHERE seat_id=?1",
            [seat.as_str()],
        )
        .unwrap();
    same.conn
        .execute(
            "UPDATE seats SET state='unresolved',unresolved_reason='other',generation=generation+1 WHERE id=?1",
            [seat.as_str()],
        )
        .unwrap();
    assert!(same.listed().is_empty());
}

/// Kills: a decision that leaves the seat unresolved or the pane held, records another kind, opens
/// a connected, registered or fingerprinted binding, more than one, or one the wake walk misses.
#[test]
fn decision_resolves_and_opens_one_dormant_binding() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.codex_seat("p1", "sess-a");
    fx.handoff(&["p1"]);
    let (state, generation) = fx.state(&seat);
    assert_eq!(state, "unresolved");
    fx.conn
        .execute(
            "INSERT OR IGNORE INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,'p1','b2',1,'unresolved saved seat continuity')",
            [fx.host.get()],
        )
        .unwrap();
    let result = fx.decide(&seat, "sess-a").unwrap();
    let CommandResult::ContinuityReattached(reattached) = result else {
        panic!("{result:?}")
    };
    assert_eq!(reattached.seat, seat);
    assert_eq!(reattached.binding_generation as i64, generation + 1);
    assert_eq!(fx.state(&seat), ("resolved".to_owned(), generation + 1));
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM recovery_holds WHERE target_id='p1' AND released_at IS NULL"
        ),
        0,
        "the pane's hold is released"
    );
    assert_eq!(fx.decisions(&seat), 1);
    let open = fx.open(&seat);
    assert_eq!(open.len(), 1, "{open:?}");
    let (binding, liveness, reason, provenance, registered, fingerprinted) = &open[0];
    assert_eq!(liveness, "disconnected");
    assert_eq!(reason.as_deref(), Some("host_session_restore"));
    assert_eq!(provenance, "host_session_continuity");
    assert!(!*registered && !*fingerprinted);
    let placed: (String, String, i64, String, String) = fx
        .conn
        .query_row(
            "SELECT harness,native_session,generation,terminal_id,incarnation FROM occupant_bindings WHERE binding_id=?1",
            [binding],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        placed,
        (
            "codex".to_owned(),
            "sess-a".to_owned(),
            generation + 1,
            "term-p1-inc-2".to_owned(),
            "inc-2".to_owned()
        )
    );
    assert_eq!(
        lazy_delivery::dormant_top_level(&fx.conn, seat.as_str()).unwrap(),
        Some(binding.clone())
    );
    assert!(fx.listed().is_empty(), "no longer a candidate");
    // The next pass reconfirms the seat rather than unresolving it.
    let outcomes = fx.publish_and_pass(&["p1"]);
    assert!(
        outcomes.iter().all(|outcome| !matches!(
            outcome,
            ReconciliationOutcome::Stale(_) | ReconciliationOutcome::RetirementStarted(_)
        )),
        "{outcomes:?}"
    );
    assert_eq!(fx.state(&seat).0, "resolved");
}

/// A second cold restart before the session's first turn: the restored binding, never connected,
/// is ended by that unresolve and names the session again. Kills: a listing that takes only
/// `seat_unresolved`, which strands the seat after the second restart.
#[test]
fn a_second_restart_before_the_first_turn_restores_again() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.codex_seat("p1", "sess-a");
    fx.handoff(&["p1"]);
    fx.decide(&seat, "sess-a").unwrap();
    fx.boot = "b3";
    fx.incarnation = "inc-3";
    fx.publish_and_pass(&["p1"]);
    assert_eq!(fx.state(&seat).0, "unresolved");
    assert_eq!(fx.listed(), vec![seat.clone()]);
    fx.decide(&seat, "sess-a").unwrap();
    assert_eq!(fx.state(&seat).0, "resolved");
    assert_eq!(fx.decisions(&seat), 2);
    assert_eq!(fx.open(&seat).len(), 1);
}

/// Kills: a decision on a session that fits two seats.
#[test]
fn another_seats_binding_naming_the_session_blocks() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.codex_seat("p1", "sess-a");
    let other = fx.codex_seat("p2", "sess-other");
    // The other seat once held the same session (a `/resume` into it), ended long ago.
    fx.agent(&other, "codex", "sess-a", "cooperative_top_level");
    fx.conn
        .execute(
            "UPDATE occupant_bindings SET ended_at=50 WHERE seat_id=?1 AND native_session='sess-a'",
            [other.as_str()],
        )
        .unwrap();
    fx.handoff(&["p1", "p2"]);
    let refused = fx.decide(&seat, "sess-a").unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict, "{refused:?}");
    assert_eq!(fx.state(&seat).0, "unresolved");
    // A retired seat's binding is no rival.
    seats::mutate_operator(
        &fx.context,
        &mut fx.conn,
        INSTANCE,
        crate::ports::OperatorRequest::Retire(crate::protocol::commands::OperatorRetire {
            seat: other.clone(),
            operation: OperationId::new("op-retire"),
        }),
        crate::protocol::authority::OperatorActor::from_peer(
            crate::protocol::authority::PeerIdentity::from_kernel(501),
            501,
        )
        .unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(fx.state(&other).0, "retired");
    fx.decide(&seat, "sess-a").unwrap();
    assert_eq!(fx.state(&seat).0, "resolved");
}

/// Kills: choosing between two unresolved seats saved at one pane.
#[test]
fn another_unresolved_seat_saved_at_the_target_blocks() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.codex_seat("p1", "sess-a");
    let other = fx.codex_seat("p2", "sess-other");
    fx.handoff(&["p1", "p2"]);
    fx.conn
        .execute(
            "UPDATE seats SET target_id='p1' WHERE id=?1",
            [other.as_str()],
        )
        .unwrap();
    let refused = fx.decide(&seat, "sess-a").unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict, "{refused:?}");
    assert_eq!(fx.state(&seat).0, "unresolved");
    assert_eq!(fx.decisions(&seat), 0);
}

/// Kills: deciding past a kept claim, on the target or for one of the seat's sessions anywhere.
#[test]
fn a_kept_claim_blocks_the_decision() {
    for (target, session) in [("p1", "kept"), ("p9", "sess-a")] {
        let mut fx = Fx::new(&["p1"]);
        let seat = fx.codex_seat("p1", "sess-a");
        fx.handoff(&["p1"]);
        fx.conn.execute(
            "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) VALUES (?1,?2,'codex',?3,'{}',100)",
            params![fx.host.get(), target, session],
        )
        .unwrap();
        let refused = fx.decide(&seat, "sess-a").unwrap_err();
        assert_eq!(refused.code, ErrorCode::ServiceBusy, "{target} {refused:?}");
        assert_eq!(fx.state(&seat).0, "unresolved");
    }
}

/// Kills: taking a pane that has a resolved owner.
#[test]
fn a_resolved_owner_blocks_the_decision() {
    let mut fx = Fx::new(&["p1", "p2"]);
    let seat = fx.codex_seat("p1", "sess-a");
    let owner = fx.resolve("p2");
    fx.handoff(&["p1", "p2"]);
    fx.conn
        .execute(
            "UPDATE seats SET state='resolved',unresolved_reason=NULL,target_id='p1' WHERE id=?1",
            [owner.as_str()],
        )
        .unwrap();
    let refused = fx.decide(&seat, "sess-a").unwrap_err();
    assert_eq!(refused.code, ErrorCode::TargetAlreadyOwned, "{refused:?}");
    assert_eq!(fx.state(&seat).0, "unresolved");
}

/// Kills: deciding over a hold-bearing key conflict on the target, which a key's owner decides.
#[test]
fn a_holding_key_conflict_row_blocks() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.codex_seat("p1", "sess-a");
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
    let refused = fx.decide(&seat, "sess-a").unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict, "{refused:?}");
    assert_eq!(fx.state(&seat).0, "unresolved");
}

/// Kills: a replay that rebinds or enrolls twice, or a session or seat the listing no longer names.
#[test]
fn replay_is_idempotent_and_a_stale_request_moves_nothing() {
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.codex_seat("p1", "sess-a");
    fx.handoff(&["p1"]);
    let (_, generation) = fx.state(&seat);
    let generation = generation as u64;
    let target = HostTargetId::new("p1");
    let first = fx.request_for(&seat, generation, &target, "codex".into(), "sess-a".into());
    let decided = fx.submit(first).unwrap();
    let again = fx.request_for(&seat, generation, &target, "codex".into(), "sess-a".into());
    assert_eq!(fx.submit(again).unwrap(), decided);
    assert_eq!(fx.decisions(&seat), 1);
    assert_eq!(fx.open(&seat).len(), 1);
    // A session the seat's ended bindings do not name.
    let mut other = Fx::new(&["p1"]);
    let seat = other.codex_seat("p1", "sess-a");
    other.handoff(&["p1"]);
    let (_, generation) = other.state(&seat);
    let stale = other.request_for(
        &seat,
        generation as u64,
        &target,
        "codex".into(),
        "sess-unknown".into(),
    );
    let refused = other.submit(stale).unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict, "{refused:?}");
    assert_eq!(other.state(&seat).0, "unresolved");
}

/// The resumed session's own first turn checks in at its pane: the seat is resolved there, so it
/// is an ordinary check-in. Kills: a second binding for the session, or one left dormant.
#[test]
fn the_sessions_own_check_in_connects_the_restored_binding() {
    use crate::{
        ports::{CooperativePermitRequest, ReadContext, RegisterAvailableRequest, StorePort},
        protocol::{
            authority::{CallerClaim, CallerRole, Harness, ObligationRef},
            commands::{CheckIn, CheckInMode},
            ids::{ExecutionId, NativeSessionId},
        },
    };
    let mut fx = Fx::new(&["p1"]);
    let seat = fx.codex_seat("p1", "sess-a");
    fx.handoff(&["p1"]);
    fx.decide(&seat, "sess-a").unwrap();
    let restored = fx.open(&seat)[0].0.clone();
    let (_, generation) = fx.state(&seat);
    fx.read("p1");
    let store = crate::store::SqliteStore::new(
        StoreContext::new(fx.path.clone(), Arc::new(FixedClock)).with_test_host(),
        INSTANCE,
        crate::store::StoreSettings::default(),
    )
    .unwrap();
    let claim = CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: INSTANCE.into(),
        seat: seat.clone(),
        binding_generation: generation as u64,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("sess-a"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("p1"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    };
    let command = CheckIn {
        readonly: false,
        lineage_parent: None,
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: generation as u64,
        },
        claim: claim.clone(),
        operation: OperationId::new("first-turn"),
        continuity_key: None,
    };
    let permit = store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim: claim.clone(),
                operation: command.operation.clone(),
                obligation: ObligationRef::CheckIn(seat.clone()),
                payload_hash: crate::store::schema::canonical_digest(&(
                    "check_in",
                    &command.mode,
                    &command.claim,
                ))
                .unwrap(),
                check_in_mode: Some(command.mode),
            },
            &budget(),
        )
        .unwrap();
    store
        .register_available(
            RegisterAvailableRequest {
                command,
                read: ReadContext {
                    instance: INSTANCE.into(),
                    output: Default::default(),
                    operation_scope: None,
                },
                operator: None,
            },
            permit,
            &budget(),
        )
        .unwrap();
    drop(store);
    let open = fx.open(&seat);
    assert_eq!(open.len(), 1, "no second binding: {open:?}");
    let (binding, liveness, reason, provenance, registered, fingerprinted) = &open[0];
    assert_eq!(binding, &restored);
    assert_eq!(liveness, "connected");
    assert_eq!(reason, &None);
    assert_eq!(provenance, "cooperative_top_level");
    assert!(*registered && *fingerprinted);
    assert_eq!(
        lazy_delivery::dormant_top_level(&fx.conn, seat.as_str()).unwrap(),
        None
    );
}

/// Roast diff 1, finding 1: a restored binding's wake is content-free (R18), as for the
/// hook-registered binding it succeeds, and it keeps turn delivery.
#[test]
fn a_restored_binding_gets_the_content_free_wake_and_turn_delivery() {
    let registry = crate::harness::registry::builtins();
    let restored = Some(crate::protocol::authority::HOST_SESSION_CONTINUITY_PROVENANCE);
    for harness in ["codex", "claude"] {
        assert_eq!(
            registry.binding_wake_text(harness, restored),
            crate::notification::policy::WAKE_UP,
            "{harness}"
        );
    }
    assert!(registry.binding_turn_delivery("codex", restored));
}
