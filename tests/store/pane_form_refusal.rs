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
        self.allocate_on(self.host, pane, operation)
    }

    /// A herdr-core host at `socket`.
    fn core(&mut self, socket: &str) -> HostId {
        let locator = HostLocator::parse(socket).unwrap();
        crate::store::hosts::attach_host(&mut self.conn, &locator, 0, HostKind::Hc)
            .unwrap()
            .0
    }

    /// A Herdr TUI host at `socket`.
    fn herdr(&mut self, socket: &str) -> HostId {
        let locator = HostLocator::parse(socket).unwrap();
        crate::store::hosts::attach_host(&mut self.conn, &locator, 0, HostKind::Herdr)
            .unwrap()
            .0
    }

    /// [`Self::allocate`] on `host`.
    fn allocate_on(
        &mut self,
        host: HostId,
        pane: &str,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
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

// Kills: M10 comparing the bare allocation target `tN` with a stored hint that a terminal writes
// as `<stem>:tN`, so it never fires for a named seat in a herdr-core terminal.
#[test]
fn terminal_hint_refuses_allocation_on_its_core() {
    let mut f = fixture();
    f.named_seat_on("/run/cores/alpha.sock:t1");
    let alpha = f.core("/run/cores/alpha.sock");
    let refused = f.allocate_on(alpha, "t1", "alloc-1").unwrap_err();
    assert_eq!(refused.code, ErrorCode::TargetAlreadyOwned);
    assert_eq!(
        refused.detail,
        "pane t1 runs named seat reviewer; address it by name"
    );
}

// Kills: M10 matching the terminal by target alone, which refuses the same `tN` of another core.
#[test]
fn same_tn_on_another_core_still_allocates() {
    let mut f = fixture();
    f.named_seat_on("/run/cores/alpha.sock:t1");
    let beta = f.core("/run/cores/beta.sock");
    assert!(
        f.allocate_on(beta, "t1", "alloc-1").is_ok(),
        "t1 of core beta is not the named seat's terminal"
    );
}

// Kills: a refusal by a named binding whose process exited (liveness `disconnected`), which holds
// the pane until the seat is retired.
#[test]
fn exited_named_agent_no_longer_refuses() {
    let mut f = fixture();
    f.named_seat_on("w1:p2");
    f.conn
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected', disconnected_at=5, disconnect_reason=?1",
            [crate::store::bindings::DisconnectReason::ProcessExit.as_str()],
        )
        .unwrap();
    assert!(f.allocate("w1:p2", "alloc-1").is_ok(), "exited named agent");
}

// Kills: a liveness filter that is not reversible (a reconnected binding must refuse again).
#[test]
fn reconnected_named_agent_refuses_again() {
    let mut f = fixture();
    f.named_seat_on("w1:p2");
    f.conn
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected', disconnected_at=5, disconnect_reason='process_exit'",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE occupant_bindings SET liveness='connected', disconnected_at=NULL, disconnect_reason=NULL",
            [],
        )
        .unwrap();
    let refused = f.allocate("w1:p2", "alloc-1").unwrap_err();
    assert_eq!(refused.code, ErrorCode::TargetAlreadyOwned);
}

// Kills: M10 comparing the bare pane id, which refuses the same pane id of another Herdr server.
#[test]
fn same_pane_id_on_another_herdr_still_allocates() {
    let mut f = fixture();
    f.named_seat_on("w1:p2\npane-host=/run/herdr-a.sock");
    let a = f.herdr("/run/herdr-a.sock");
    let b = f.herdr("/run/herdr-b.sock");
    assert!(
        f.allocate_on(b, "w1:p2", "alloc-1").is_ok(),
        "w1:p2 of server b is not the named seat's pane"
    );
    let refused = f.allocate_on(a, "w1:p2", "alloc-2").unwrap_err();
    assert_eq!(refused.code, ErrorCode::TargetAlreadyOwned);
}

// Pins the accepted limit: a pane hint with no reported server matches that pane id on every host.
#[test]
fn pane_hint_with_no_server_matches_any_host() {
    let mut f = fixture();
    f.named_seat_on("w1:p2");
    let a = f.herdr("/run/herdr-a.sock");
    let refused = f.allocate_on(a, "w1:p2", "alloc-1").unwrap_err();
    assert_eq!(refused.code, ErrorCode::TargetAlreadyOwned);
}
