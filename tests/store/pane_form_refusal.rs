//! N4-paneform (SA9): pane-form allocation of a pane a live named seat's binding hinted is refused
//! with M10; any other pane allocates as before.
use super::*;
use crate::{
    ports::{EvidenceKind, HostKind},
    protocol::{
        commands::ResolveSeat,
        ids::{HostLocator, NAMED_HOST_LOCATOR, OperationId, TerminalId},
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
};
use rusqlite::Connection;
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

struct Fixture {
    context: StoreContext,
    conn: Connection,
    host: HostId,
}

fn fixture() -> Fixture {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-pane-form-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path, Arc::new(FixedClock)).with_test_host();
    let mut conn = context.open_writer().unwrap();
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let (host, _) =
        crate::store::hosts::attach_host(&mut conn, &locator, 0, HostKind::Named).unwrap();
    Fixture {
        context,
        conn,
        host,
    }
}

impl Fixture {
    /// A named seat `reviewer` with one open top-level binding hinting `pane`.
    fn named_seat_on(&mut self, pane: &str) -> SeatId {
        let request = ResolveSeat {
            target: HostTargetId::new("reviewer"),
            host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
            operation: OperationId::new("op-named"),
            locator_hint: Some(pane.to_owned()),
            tmux_hint: None,
        };
        let CommandResult::NamedSeatResolved { seat, .. } = resolve_named_seat(
            &self.context,
            &mut self.conn,
            INSTANCE,
            self.host,
            &request,
            &budget(),
        )
        .unwrap() else {
            panic!("wrong result")
        };
        self.conn
            .execute(
                "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) \
                 VALUES (?1,1,'reviewer','b',1,'codex','s1',?2,'cooperative_top_level',0,'term','inc')",
                rusqlite::params![seat.as_str(), uuid::Uuid::new_v4().to_string()],
            )
            .unwrap();
        // The next named resolve from the pane records the hint on that binding.
        let again = ResolveSeat {
            operation: OperationId::new("op-hint"),
            ..request
        };
        resolve_named_seat(
            &self.context,
            &mut self.conn,
            INSTANCE,
            self.host,
            &again,
            &budget(),
        )
        .unwrap();
        seat
    }

    /// Pane-form allocation of `pane` through the shared ordinary recipe.
    fn allocate(&mut self, pane: &str, operation: &str) -> Result<CommandResult, ApiError> {
        let target = HostTargetId::new(pane);
        let proof = DurableStructuralProof::from_persisted(
            target.clone(),
            TerminalId::new(format!("t-{pane}")),
            "inc-1".to_owned(),
            EvidenceKind::NativeCurrentTarget,
            HostBootId::new("boot-1"),
            1,
            1,
            1,
            1,
        )
        .unwrap();
        let host = self.host;
        schema::execute_idempotent_transaction(
            &self.context,
            &mut self.conn,
            &format!("service-allocation:{INSTANCE}"),
            operation,
            schema::canonical_digest(&("pane_form", pane)).unwrap(),
            |_| Ok(()),
            |tx, at| {
                ordinary_resolution_apply(tx, at, INSTANCE, host, &target, &proof)
                    .map(|(result, _)| result)
            },
        )
    }
}

// Kills: no refusal of a named agent's pane (a second ordinary seat minted for it), the wrong
// text, and a refusal of unrelated panes.
#[test]
fn pane_form_allocation_of_named_agents_pane_refused() {
    let mut f = fixture();
    f.named_seat_on("w1:p2");
    let refused = f.allocate("w1:p2", "alloc-1").unwrap_err();
    assert_eq!(refused.code, ErrorCode::TargetAlreadyOwned);
    let rendered = format!("herdr-threads: {}\n", refused.detail);
    assert_eq!(rendered, include_str!("../cli/golden/m10.txt"));
    assert_eq!(
        "herdr-threads: pane  runs named seat ; address it by name".len(),
        57,
        "the fixed bytes of M10"
    );
    let seats: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM seats WHERE target_id='w1:p2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seats, 0, "the refused allocation minted nothing");
    assert!(
        f.allocate("w1:p3", "alloc-2").is_ok(),
        "another pane still allocates"
    );
}

// Kills: refusing on a binding that ended or a seat that retired (the pane is then free).
#[test]
fn ended_binding_or_retired_seat_no_longer_refuses() {
    let mut f = fixture();
    f.named_seat_on("w1:p2");
    f.conn
        .execute("UPDATE occupant_bindings SET ended_at=5", [])
        .unwrap();
    assert!(f.allocate("w1:p2", "alloc-1").is_ok(), "ended binding");

    let mut g = fixture();
    g.named_seat_on("w1:p2");
    g.conn
        .execute(
            "UPDATE seats SET state='retired',retired_at=5 WHERE name='reviewer'",
            [],
        )
        .unwrap();
    assert!(g.allocate("w1:p2", "alloc-1").is_ok(), "retired seat");
}
