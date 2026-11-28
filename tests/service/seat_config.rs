//! Per-seat config through the daemon (ht-z41e.2; included from `src/service/dispatch.rs`): the
//! two tiers' authority, parity of `SetSeatConfig nudge_threshold` with the `SetNudgeThreshold`
//! alias per seat class, the refusals that reach the client, and the any-caller reads.
use crate::{
    ports::{LocalService, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{
            CheckIn, CheckInMode, Command, GetSeatConfig, SetNudgeThreshold, SetSeatConfig,
            UnsetSeatConfig, WakeStatusQuery,
        },
        ids::*,
        priority::NudgeThreshold,
        results::{ApiError, CommandResult, ErrorCode, SeatConfigWrite},
        seat_config::ConfigTier,
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::dispatch::DomainService,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::{os::unix::fs::DirBuilderExt, sync::Arc};

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const EXEC_1: &str = "00000000-0000-4000-8000-000000000001";

struct PrivateDirectory(std::path::PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn owner() -> PeerIdentity {
    PeerIdentity::from_kernel(501)
}

/// The seat classes the daemon tells apart by the claim's host: a Herdr pane seat (a hook-registered
/// or plain-CLI claim is the same claim to the daemon, so the harness varies instead), a named
/// seat on the reserved named host, and a herdr-core terminal seat (a host row of kind `hc`).
#[derive(Clone, Copy, Debug)]
enum Class {
    Pane(Harness),
    Named,
    Terminal,
}
const CLASSES: [Class; 4] = [
    Class::Pane(Harness::Claude),
    Class::Pane(Harness::Codex),
    Class::Named,
    Class::Terminal,
];

struct Fixture {
    // Drop order: the service and connection release the store before its directory goes.
    service: DomainService,
    db: Connection,
    class: Class,
    _directory: PrivateDirectory,
}

/// A service over seat `s` of `class`, plus a second seat `o` with no binding.
fn fixture(class: Class) -> Fixture {
    let path = crate::test_support::scratch::temp_path(format!(
        "service-seat-config-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let directory = PrivateDirectory(path.clone());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let named = matches!(class, Class::Named);
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let context = if named {
        StoreContext::new(path.join("store.db"), clock.clone()).with_attached_host(&locator)
    } else {
        StoreContext::new(path.join("store.db"), clock.clone()).with_test_host()
    };
    let db = context.open_writer().unwrap();
    let kind = match class {
        Class::Named => "named",
        Class::Terminal => "hc",
        Class::Pane(_) => "herdr",
    };
    let endpoint = if named {
        locator.as_str().to_owned()
    } else {
        crate::test_support::test_host_locator().as_str().to_owned()
    };
    db.execute(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1,kind=?1 WHERE endpoint=?2",
        rusqlite::params![kind, endpoint],
    )
    .unwrap();
    let host: i64 = db
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE kind=?1",
            [kind],
            |r| r.get(0),
        )
        .unwrap();
    let (target, name) = match class {
        Class::Named => ("reviewer", Some("reviewer")),
        Class::Terminal => ("t1", None),
        Class::Pane(_) => ("p", None),
    };
    for (seat, target, name) in [("s", target, name), ("o", "q", None)] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,name) VALUES (?1,?2,'resolved','native',?3,0,0,0,(SELECT id FROM local_node),1,?4,?5)",
            rusqlite::params![seat, INSTANCE, target, host, name]).unwrap();
        db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,?2,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?2,'inc','coherent_enumeration',1)",
            rusqlite::params![host, target]).unwrap();
    }
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, INSTANCE, StoreSettings::default()).unwrap());
    let service = DomainService::new(INSTANCE.into(), store, clock)
        .with_operator_owner(501)
        .with_cooperative_owner(
            501,
            Arc::new(crate::service::fair_writer::FairWriter::new(32)),
        );
    Fixture {
        service,
        db,
        class,
        _directory: directory,
    }
}

impl Fixture {
    fn claim(&self) -> CallerClaim {
        let (host, target, harness) = match self.class {
            Class::Named => (
                HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
                "reviewer",
                Harness::Codex,
            ),
            Class::Terminal => (
                crate::test_support::test_host_locator(),
                "t1",
                Harness::Claude,
            ),
            Class::Pane(harness) => (crate::test_support::test_host_locator(), "p", harness),
        };
        CallerClaim {
            auxiliary_session: false,
            host,
            instance: INSTANCE.into(),
            seat: SeatId::new("s"),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness,
            native_session: NativeSessionId::new("session-n"),
            execution: ExecutionId::new(EXEC_1),
            target: HostTargetId::new(target),
            agent_id: None,
            liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
        }
    }
    /// Checks the seat's top-level binding in and returns the context the daemon answers with.
    fn check_in(&self, readonly: bool) -> CallerClaim {
        let claim = self.claim();
        let command = Command::CheckIn(CheckIn {
            lineage_parent: None,
            readonly,
            mode: CheckInMode::Lifecycle {
                expected_binding_generation: claim.binding_generation,
            },
            claim,
            operation: OperationId::new(format!("in-{}", uuid::Uuid::new_v4())),
            continuity_key: None,
        });
        match self.service.handle(command, owner(), &budget()).unwrap() {
            CommandResult::CheckedIn(result) => result.context,
            other => panic!("unexpected {other:?}"),
        }
    }
    fn handle(&self, command: Command) -> Result<CommandResult, ApiError> {
        self.handle_as(command, owner())
    }
    fn handle_as(&self, command: Command, peer: PeerIdentity) -> Result<CommandResult, ApiError> {
        command
            .validate()
            .map_err(|e| ApiError::new(ErrorCode::InvalidRequest, e))?;
        self.service.handle(command, peer, &budget())
    }
    fn agent_set(
        &self,
        claim: &CallerClaim,
        key: &str,
        value: &str,
    ) -> Result<CommandResult, ApiError> {
        self.handle(Command::SetSeatConfig(set(
            Some(claim),
            None,
            ConfigTier::Agent,
            key,
            value,
        )))
    }
    fn threshold_and_mirror(&self, seat: &str) -> (i64, String) {
        self.db
            .query_row(
                "SELECT nudge_threshold,wake_on FROM seats WHERE id=?1",
                [seat],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
    fn wake_work(&self, seat: &str) -> Option<(i64, i64)> {
        self.db
            .query_row(
                "SELECT reason_bits,attention_version FROM wake_work WHERE seat_id=?1",
                [seat],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }
    fn wake_status(&self) -> CommandResult {
        self.handle(Command::WakeStatus(WakeStatusQuery {
            all: false,
            seats: vec![SeatId::new("s")],
            message: None,
            observer: None,
        }))
        .unwrap()
    }
    fn writer(&self, seat: &str, key: &str) -> Option<String> {
        self.db
            .query_row(
                "SELECT writer_provenance FROM seat_config WHERE seat_id=?1 AND key=?2",
                [seat, key],
                |r| r.get(0),
            )
            .ok()
    }
}

fn set(
    claim: Option<&CallerClaim>,
    seat: Option<&str>,
    tier: ConfigTier,
    key: &str,
    value: &str,
) -> SetSeatConfig {
    SetSeatConfig {
        seat: seat.map(SeatId::new),
        key: key.into(),
        value: value.into(),
        tier,
        operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
        claim: claim.cloned(),
    }
}
fn human_set(
    f: &Fixture,
    peer: PeerIdentity,
    seat: Option<&str>,
    key: &str,
    value: &str,
) -> Result<CommandResult, ApiError> {
    f.handle_as(
        Command::SetSeatConfig(set(None, seat, ConfigTier::Human, key, value)),
        peer,
    )
}
fn written(result: Result<CommandResult, ApiError>) -> SeatConfigWrite {
    match result.unwrap() {
        CommandResult::SeatConfigWritten(write) => write,
        other => panic!("unexpected {other:?}"),
    }
}
fn view(f: &Fixture, seat: &str, key: Option<&str>) -> crate::protocol::results::SeatConfigView {
    match f
        .handle(Command::GetSeatConfig(GetSeatConfig {
            seat: SeatId::new(seat),
            key: key.map(str::to_owned),
        }))
        .unwrap()
    {
        CommandResult::SeatConfig(view) => view,
        other => panic!("unexpected {other:?}"),
    }
}

/// Kills: a `SetSeatConfig nudge_threshold` that decides differently from `SetNudgeThreshold` for
/// some class of seat (a second authority check, a different wake re-judgement or a different
/// no-wake answer), per class of seat and for a stale claim too.
#[test]
fn set_seat_config_and_the_alias_leave_identical_state_for_every_seat_class() {
    for class in CLASSES {
        let generic = fixture(class);
        let me = generic.check_in(false);
        let result = written(generic.agent_set(&me, "nudge_threshold", "3"));
        assert_eq!(
            (result.value.as_str(), result.changed, result.is_default),
            ("3", true, false),
            "{class:?}"
        );
        let alias = fixture(class);
        let me = alias.check_in(false);
        let result = alias
            .handle(Command::SetNudgeThreshold(SetNudgeThreshold {
                value: NudgeThreshold::new(3).unwrap(),
                operation: OperationId::new("alias"),
                claim: me.clone(),
            }))
            .unwrap();
        assert!(
            matches!(result, CommandResult::NudgeThreshold(ref s) if s.changed),
            "{class:?}"
        );
        assert_eq!(
            generic.threshold_and_mirror("s"),
            alias.threshold_and_mirror("s"),
            "{class:?}"
        );
        assert_eq!(
            generic.threshold_and_mirror("s"),
            (3, "nudge".into()),
            "{class:?}"
        );
        assert_eq!(generic.wake_work("s"), alias.wake_work("s"), "{class:?}");
        assert!(
            generic.wake_work("s").is_some(),
            "{class:?}: the change re-judged the wake"
        );
        // The same subsequent WakeStatus answer, a no-wake one included (named and terminal).
        assert_eq!(generic.wake_status(), alias.wake_status(), "{class:?}");
        // A claim for a binding that is not current is refused by both, alike.
        let mut stale = me.clone();
        stale.binding_generation += 5;
        let a = generic
            .agent_set(&stale, "nudge_threshold", "4")
            .unwrap_err();
        let b = alias
            .handle(Command::SetNudgeThreshold(SetNudgeThreshold {
                value: NudgeThreshold::new(4).unwrap(),
                operation: OperationId::new("alias-stale"),
                claim: stale,
            }))
            .unwrap_err();
        assert_eq!(a.code, b.code, "{class:?}");
        assert_eq!(generic.threshold_and_mirror("s").0, 3, "{class:?}");
        // The agent audit row records the cooperative provenance; the alias writes it too.
        assert_eq!(
            generic.writer("s", "nudge_threshold").as_deref(),
            Some("cooperative_top_level")
        );
        assert_eq!(
            alias.writer("s", "nudge_threshold").as_deref(),
            Some("cooperative_top_level")
        );
    }
}

#[test]
fn agent_unset_returns_the_threshold_to_its_default() {
    let f = fixture(Class::Pane(Harness::Claude));
    let me = f.check_in(false);
    f.agent_set(&me, "nudge_threshold", "-1").unwrap();
    let write = written(f.handle(Command::UnsetSeatConfig(UnsetSeatConfig {
        seat: None,
        key: "nudge_threshold".into(),
        tier: ConfigTier::Agent,
        operation: OperationId::new("unset"),
        claim: Some(me),
    })));
    assert_eq!(
        (write.value.as_str(), write.is_default, write.changed),
        ("0", true, true)
    );
    assert_eq!(f.threshold_and_mirror("s"), (0, "nudge".into()));
}

#[test]
fn agent_tier_refusals_reach_the_client() {
    let f = fixture(Class::Pane(Harness::Claude));
    let me = f.check_in(false);
    // A human-tier key: the exact line.
    let error = f.agent_set(&me, "programmatic_wake", "on").unwrap_err();
    assert_eq!(
        error.detail,
        "seat config key `programmatic_wake` needs `ht human config seat set programmatic_wake <value>`"
    );
    // Another seat.
    let other = f
        .handle(Command::SetSeatConfig(set(
            Some(&me),
            Some("o"),
            ConfigTier::Agent,
            "nudge_threshold",
            "2",
        )))
        .unwrap_err();
    assert!(
        other.detail.contains("only the caller's own seat"),
        "{other:?}"
    );
    // A subagent never mutates.
    let mut child = me.clone();
    child.role = CallerRole::Subagent;
    // Client-side validation already refuses the claim shape; the daemon refuses it on its own too.
    let command = Command::SetSeatConfig(set(
        Some(&child),
        None,
        ConfigTier::Agent,
        "nudge_threshold",
        "2",
    ));
    assert!(command.validate().is_err());
    assert_eq!(
        f.service
            .handle(command, owner(), &budget())
            .unwrap_err()
            .code,
        ErrorCode::SubagentReadOnly
    );
    // Unknown key and bad values reach the client unchanged.
    assert_eq!(
        f.agent_set(&me, "nope", "1").unwrap_err().detail,
        "unknown seat config key `nope`; keys: nudge_threshold, programmatic_wake"
    );
    assert_eq!(
        f.agent_set(&me, "nudge_threshold", "-5")
            .unwrap_err()
            .detail,
        "invalid value `-5` for nudge_threshold; values: -1..9"
    );
    // An agent write needs a claim.
    let no_claim = f.handle(Command::SetSeatConfig(set(
        None,
        None,
        ConfigTier::Agent,
        "nudge_threshold",
        "2",
    )));
    assert_eq!(no_claim.unwrap_err().code, ErrorCode::InvalidRequest);
    // A non-owner peer is refused as for every cooperative mutation.
    let wrong = f.handle_as(
        Command::SetSeatConfig(set(
            Some(&me),
            None,
            ConfigTier::Agent,
            "nudge_threshold",
            "2",
        )),
        PeerIdentity::from_kernel(502),
    );
    assert_eq!(wrong.unwrap_err().code, ErrorCode::Unauthorized);
    assert_eq!(
        f.threshold_and_mirror("s"),
        (0, "nudge".into()),
        "every refusal wrote nothing"
    );
    assert_eq!(f.writer("s", "nudge_threshold"), None);
}

#[test]
fn a_read_only_binding_cannot_write_seat_config() {
    let f = fixture(Class::Pane(Harness::Codex));
    let me = f.check_in(true);
    let error = f.agent_set(&me, "nudge_threshold", "2").unwrap_err();
    assert_eq!(error.detail, "read-only binding cannot write seat config");
    assert_eq!(f.threshold_and_mirror("s"), (0, "nudge".into()));
}

#[test]
fn human_tier_is_an_operator_decision_on_any_local_seat() {
    let f = fixture(Class::Pane(Harness::Claude));
    // No caller seat at all: seat `o` has no binding.
    let write = written(human_set(&f, owner(), Some("o"), "programmatic_wake", "on"));
    assert_eq!((write.value.as_str(), write.changed), ("on", true));
    assert_eq!(
        f.writer("o", "programmatic_wake").as_deref(),
        Some("operator:local-user:501")
    );
    assert_eq!(
        view(&f, "o", Some("programmatic_wake")).entries[0].value,
        "on"
    );
    // The A5 exception: the operator sets another seat's nudge threshold.
    let write = written(human_set(&f, owner(), Some("o"), "nudge_threshold", "4"));
    assert_eq!(write.value, "4");
    assert_eq!(f.threshold_and_mirror("o").0, 4);
    assert_eq!(
        f.writer("o", "nudge_threshold").as_deref(),
        Some("operator:local-user:501")
    );
    assert!(
        f.wake_work("o").is_some(),
        "the human path re-judges the wake as well"
    );
    // Unset through the same tier.
    let unset = written(f.handle(Command::UnsetSeatConfig(UnsetSeatConfig {
        seat: Some(SeatId::new("o")),
        key: "programmatic_wake".into(),
        tier: ConfigTier::Human,
        operation: OperationId::new("h-unset"),
        claim: None,
    })));
    assert!(unset.is_default && unset.changed);
    assert_eq!(f.writer("o", "programmatic_wake"), None, "the row is gone");
    // A peer that is not the elected owner is refused; no seat is refused; a stranger seat is not found.
    assert_eq!(
        human_set(
            &f,
            PeerIdentity::from_kernel(502),
            Some("o"),
            "programmatic_wake",
            "on"
        )
        .unwrap_err()
        .code,
        ErrorCode::Unauthorized
    );
    assert_eq!(
        human_set(&f, owner(), None, "programmatic_wake", "on")
            .unwrap_err()
            .detail,
        "human seat config needs --seat or a caller seat"
    );
    assert_eq!(
        human_set(&f, owner(), Some("nobody"), "programmatic_wake", "on")
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        human_set(&f, owner(), Some("o"), "programmatic_wake", "true")
            .unwrap_err()
            .detail,
        "invalid value `true` for programmatic_wake; values: on|off"
    );
}

/// Kills: a read that demands a claim or refuses a seat other than the caller's. The read carries no
/// claim at all, so a declared subagent (which only ever holds a claim to mutate) can issue it.
#[test]
fn reads_need_no_claim_and_cover_any_local_seat() {
    let f = fixture(Class::Pane(Harness::Claude));
    let command = Command::GetSeatConfig(GetSeatConfig {
        seat: SeatId::new("o"),
        key: None,
    });
    assert!(
        command.accountable_claim().is_none(),
        "no claim, so no subagent refusal applies"
    );
    let me = f.check_in(false);
    f.agent_set(&me, "nudge_threshold", "5").unwrap();
    human_set(&f, owner(), Some("o"), "programmatic_wake", "on").unwrap();
    let mine = view(&f, "s", None);
    assert_eq!(mine.entries.len(), 2);
    assert_eq!(mine.entries[0].value, "5");
    assert_eq!(
        mine.entries[0].writer.as_deref(),
        Some("cooperative_top_level")
    );
    let theirs = view(&f, "o", None);
    assert_eq!(
        theirs
            .entries
            .iter()
            .map(|e| (e.key.as_str(), e.value.as_str(), e.is_default))
            .collect::<Vec<_>>(),
        [
            ("nudge_threshold", "0", true),
            ("programmatic_wake", "on", false)
        ]
    );
    let missing = f.handle(Command::GetSeatConfig(GetSeatConfig {
        seat: SeatId::new("nobody"),
        key: None,
    }));
    assert_eq!(missing.unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn a_remote_seat_is_refused_with_one_line() {
    let f = fixture(Class::Pane(Harness::Claude));
    f.db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,host_id,published) \
         VALUES ('r',?2,'remote','native',0,0,?1,0,'agent',NULL,1)",
        rusqlite::params!["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", INSTANCE],
    )
    .unwrap();
    let want_prefix = "seat r is homed on node aaaaaaaa; its seat config is set there";
    let read = f.handle(Command::GetSeatConfig(GetSeatConfig {
        seat: SeatId::new("r"),
        key: None,
    }));
    assert_eq!(read.unwrap_err().detail, want_prefix);
    assert_eq!(
        human_set(&f, owner(), Some("r"), "programmatic_wake", "on")
            .unwrap_err()
            .detail,
        want_prefix
    );
    assert_eq!(f.writer("r", "programmatic_wake"), None);
}

/// Kills: a setting that is refused, or that changes anything beyond its own row, on a seat whose
/// harness has no programmatic wake transport: it is accepted and inert.
#[test]
fn programmatic_wake_on_a_seat_with_no_transport_is_accepted_and_inert() {
    let f = fixture(Class::Pane(Harness::Claude));
    let _ = f.check_in(false);
    let before = f.threshold_and_mirror("s");
    let write = written(human_set(&f, owner(), Some("s"), "programmatic_wake", "on"));
    assert_eq!((write.value.as_str(), write.changed), ("on", true));
    // Only the wake is re-judged (ht-z41e.4); the threshold and its mirror stay.
    assert_eq!(f.threshold_and_mirror("s"), before);
}

fn human_unset(f: &Fixture, seat: &str, key: &str, op: &str) -> Result<CommandResult, ApiError> {
    f.handle(Command::UnsetSeatConfig(UnsetSeatConfig {
        seat: Some(SeatId::new(seat)),
        key: key.into(),
        tier: ConfigTier::Human,
        operation: OperationId::new(op),
        claim: None,
    }))
}

/// Kills: a human-tier write that skips the subagent refusal because the operator path takes no
/// claim: a claim naming a subagent (by role, or by agent id alone) on either verb is refused with
/// the stable code and detail before it writes a row or touches the threshold.
#[test]
fn a_subagent_claim_on_a_human_tier_write_is_refused_and_writes_nothing() {
    let f = fixture(Class::Pane(Harness::Claude));
    let me = f.check_in(false);
    let mut by_role = me.clone();
    by_role.role = CallerRole::Subagent;
    let mut by_id = me.clone();
    by_id.agent_id = Some(AgentId::new("agent-7"));
    for (name, claim) in [("role", by_role), ("agent-id", by_id)] {
        for (key, value) in [("nudge_threshold", "3"), ("programmatic_wake", "on")] {
            let mut human = set(Some(&claim), Some("s"), ConfigTier::Human, key, value);
            human.operation = OperationId::new(format!("sub-set-{name}-{key}"));
            let refused = f
                .service
                .handle(Command::SetSeatConfig(human), owner(), &budget())
                .unwrap_err();
            assert_eq!(
                refused.code,
                ErrorCode::SubagentReadOnly,
                "{name} set {key}"
            );
            assert_eq!(
                refused.detail,
                crate::protocol::authority::SUBAGENT_READ_ONLY_DETAIL,
                "{name} set {key}"
            );
            let refused = f
                .service
                .handle(
                    Command::UnsetSeatConfig(UnsetSeatConfig {
                        seat: Some(SeatId::new("s")),
                        key: key.into(),
                        tier: ConfigTier::Human,
                        operation: OperationId::new(format!("sub-unset-{name}-{key}")),
                        claim: Some(claim.clone()),
                    }),
                    owner(),
                    &budget(),
                )
                .unwrap_err();
            assert_eq!(
                refused.code,
                ErrorCode::SubagentReadOnly,
                "{name} unset {key}"
            );
        }
    }
    assert_eq!(f.threshold_and_mirror("s"), (0, "nudge".into()));
    assert_eq!(f.writer("s", "nudge_threshold"), None, "no audit row");
    assert_eq!(f.writer("s", "programmatic_wake"), None, "no config row");
}

/// Kills: a human unset of `nudge_threshold` that leaves the column or its `wake_on` mirror at the
/// set value, drops the audit row, records another provenance or skips the wake re-judgement.
#[test]
fn a_human_unset_of_nudge_threshold_resets_column_mirror_audit_and_wake() {
    let f = fixture(Class::Pane(Harness::Claude));
    let write = written(human_set(&f, owner(), Some("o"), "nudge_threshold", "-1"));
    assert_eq!((write.value.as_str(), write.is_default), ("-1", false));
    assert_eq!(f.threshold_and_mirror("o"), (-1, "all".into()));
    // Forget the set's wake work so only the unset can create it.
    f.db.execute("DELETE FROM wake_work WHERE seat_id='o'", [])
        .unwrap();
    assert_eq!(f.wake_work("o"), None);

    let write = written(human_unset(&f, "o", "nudge_threshold", "h-unset-threshold"));
    assert_eq!(
        (write.value.as_str(), write.is_default, write.changed),
        ("0", true, true)
    );
    assert_eq!(
        f.threshold_and_mirror("o"),
        (0, "nudge".into()),
        "the column and its wake_on mirror are back at the default"
    );
    assert_eq!(
        f.writer("o", "nudge_threshold").as_deref(),
        Some("operator:local-user:501"),
        "the audit row is kept with operator provenance"
    );
    assert!(f.wake_work("o").is_some(), "the unset re-judged the wake");
    assert_eq!(view(&f, "o", Some("nudge_threshold")).entries[0].value, "0");
}

/// Kills: an operator write that applies twice on a replayed operation id (the recorded result is
/// returned, the row is untouched by the replay) and one that accepts the same id for another value.
#[test]
fn a_replayed_human_write_returns_the_recorded_result_and_a_changed_payload_conflicts() {
    let f = fixture(Class::Pane(Harness::Claude));
    let mut first = set(
        None,
        Some("o"),
        ConfigTier::Human,
        "programmatic_wake",
        "on",
    );
    first.operation = OperationId::new("op-replay");
    let recorded = written(f.handle(Command::SetSeatConfig(first.clone())));
    assert_eq!((recorded.value.as_str(), recorded.changed), ("on", true));

    // Another operation moves the value on. A real second apply of `first` would put `on` back.
    written(human_set(
        &f,
        owner(),
        Some("o"),
        "programmatic_wake",
        "off",
    ));
    let replay = written(f.handle(Command::SetSeatConfig(first.clone())));
    assert_eq!(replay, recorded, "the recorded result, `changed` included");
    assert_eq!(
        view(&f, "o", Some("programmatic_wake")).entries[0].value,
        "off",
        "the replay wrote nothing"
    );

    // The same id for another payload (value, then verb) is a conflict, and writes nothing.
    let mut other = first.clone();
    other.value = "off".into();
    assert_eq!(
        f.handle(Command::SetSeatConfig(other)).unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(
        human_unset(&f, "o", "programmatic_wake", "op-replay")
            .unwrap_err()
            .code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(
        view(&f, "o", Some("programmatic_wake")).entries[0].value,
        "off"
    );
}
