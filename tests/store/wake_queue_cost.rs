//! Empty retained-ledger fence reads must not pay for binding history.
use super::*;
use crate::ports::{WakeIntentPreparation, WakeQueueSelection, WakeReservation};
use crate::test_support::isolation::{CostCounter, count_vm_units};
mod fixture {
    use crate as herdr_threads;
    include!("../fixtures/wake_intent.rs");
}
#[test]
fn empty_ledger_fence_cost_is_flat_in_valid_open_cohorts() {
    let (store, _path, r, p) = fixture::fixture();
    for edge in [
        crate::harness::adapter::TurnEdge::Busy,
        crate::harness::adapter::TurnEdge::Idle,
    ] {
        crate::ports::StorePort::record_native_turn(
            &store,
            &fixture::native_report(&store, &r, &p, "baseline", edge),
            &fixture::budget(),
        )
        .unwrap();
    }
    let db = store.context().open_writer().unwrap();
    let mut costs = Vec::new();
    for (prefix, count) in [("short", 128), ("long", 1024)] {
        db.execute(&format!("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<?1) INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id) SELECT 's',1,1,'pane','host',0,'codex',printf('00000000-0000-0000-0000-%012d',x+{offset}),'execution','cooperative_top_level',0,0,'term-pane','inc',?2||x FROM n",offset=if prefix=="short" {10000}else{20000}),rusqlite::params![count,prefix]).unwrap();
        let counter = CostCounter::default();
        count_vm_units(&db, &counter, || {
            let blocked = retained_fences(&db, fixture::INSTANCE, &r.seat).unwrap();
            assert!(!blocked.programmatic && !blocked.native);
        });
        costs.push(counter.units());
    }
    assert!(
        costs[1] <= costs[0] + 2,
        "empty-ledger work grew with binding history: {costs:?}"
    );
}

fn qualify(store: &SqliteStore, r: &WakeReservation, p: &WakeIntentPreparation) {
    for edge in [
        crate::harness::adapter::TurnEdge::Busy,
        crate::harness::adapter::TurnEdge::Idle,
    ] {
        crate::ports::StorePort::record_native_turn(
            store,
            &fixture::native_report(store, r, p, "qualified", edge),
            &fixture::budget(),
        )
        .unwrap();
    }
}
fn select(store: &SqliteStore, r: &WakeReservation) -> WakeQueueSelection {
    queue_wake_selection(store, r, true, &fixture::budget()).unwrap()
}
#[test]
fn per_seat_flag_selects_queue_only_for_the_flagged_seat() {
    let (store, _path, r, p) = fixture::fixture();
    qualify(&store, &r, &p);
    store.context().open_writer().unwrap().execute_batch(&format!(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('b','{}','resolved','native','pane-b',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));",
        fixture::INSTANCE
    )).unwrap();
    let mut other = r.clone();
    other.seat = crate::protocol::ids::SeatId::new("b");
    fixture::set_programmatic_wake(&store, "s", true);
    let on = select(&store, &r);
    assert!(on.programmatic_enabled && on.binding.is_some());
    let off = select(&store, &other);
    assert!(!off.programmatic_enabled && off.binding.is_none());
    // The flag of seat b alone does not enable seat s.
    fixture::set_programmatic_wake(&store, "s", false);
    fixture::set_programmatic_wake(&store, "b", true);
    let s = select(&store, &r);
    assert!(!s.programmatic_enabled && s.binding.is_none());
}

#[test]
fn flag_toggle_applies_on_the_next_selection() {
    let (store, _path, r, p) = fixture::fixture();
    qualify(&store, &r, &p);
    for on in [false, true, false, true] {
        fixture::set_programmatic_wake(&store, "s", on);
        let selection = select(&store, &r);
        assert_eq!(selection.programmatic_enabled, on);
        assert_eq!(selection.binding.is_some(), on);
    }
}

#[test]
fn seat_config_read_failure_selects_native() {
    let (store, _path, r, p) = fixture::fixture();
    qualify(&store, &r, &p);
    fixture::set_programmatic_wake(&store, "s", true);
    assert!(select(&store, &r).programmatic_enabled);
    store
        .context()
        .open_writer()
        .unwrap()
        .execute_batch("ALTER TABLE seat_config RENAME TO seat_config_unreadable")
        .unwrap();
    let selection = select(&store, &r);
    assert!(!selection.programmatic_enabled);
    assert!(selection.binding.is_none());
    assert!(!selection.blockers.programmatic && !selection.blockers.native);
}

const SESSION_ROOT: &str = "00000000-0000-0000-0000-000000000001";
const SIDE_ONE: &str = "00000000-0000-0000-0000-0000000000a1";
const SIDE_TWO: &str = "00000000-0000-0000-0000-0000000000a2";

/// Real Codex hook payloads (`tests/fixtures/codex-0.163.0-side`); `enroll` swaps in the session.
macro_rules! payload {
    ($name:literal) => {
        include_bytes!(concat!("../fixtures/codex-0.163.0-side/", $name)) as &[u8]
    };
}
/// The root process of the fixture is pid 10; another pid is another process.
const ROOT_PID: u32 = 10;

/// Enrolls `session` through its hook event, as the hook would (decode, check-in mode, evidence on
/// the claim), never by writing a binding flag.
fn enroll(store: &SqliteStore, payload: &[u8], session: &str, pid: u32) {
    fixture::enroll_payload(
        store,
        payload,
        session,
        pid,
        &format!("op-{}", uuid::Uuid::new_v4()),
    )
    .unwrap();
}
/// `(concurrent_session, auxiliary_session)` of the binding of `session`.
fn bits(store: &SqliteStore, session: &str) -> (bool, bool) {
    store
        .context()
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT concurrent_session,auxiliary_session FROM occupant_bindings WHERE native_session=?1",
            [session],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}
/// Ends the fixture's root binding, leaving only what was enrolled beside it.
fn disconnect_root(store: &SqliteStore) {
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=1,disconnect_reason='process_exit' WHERE binding_id='binding'",
            [],
        )
        .unwrap();
}
/// Another top-level Codex binding of seat `s` for the 64-binding census bound only; the bound is
/// independent of which bits a binding carries, so this one is not a selection fixture.
fn add_binding(store: &SqliteStore, id: &str, session: &str) {
    let db = store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id,liveness_pid,liveness_start,liveness_boot,concurrent_session,auxiliary_session) VALUES ('s',1,1,'pane','host',0,'codex',?1,'execution','cooperative_top_level',0,0,'term-pane','inc',?2,10,1,(SELECT boot FROM node_kernel_boot WHERE id=1),1,1)",
        rusqlite::params![session, id],
    )
    .unwrap();
}
fn selected(selection: &WakeQueueSelection) -> (String, Vec<String>) {
    let binding = selection.binding.as_ref().expect("queue binding selected");
    (
        binding.scope.native_session.as_str().to_owned(),
        binding
            .side_sessions
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect(),
    )
}
fn native(selection: &WakeQueueSelection) {
    assert!(selection.programmatic_enabled && selection.binding.is_none());
}

/// The fixture with `events` enrolled (session, payload, pid) through their hooks before the
/// wake is reserved (a check-in advances the decision sequence, which stales an earlier
/// reservation), then qualified: ready for `select`.
fn seat_with(
    events: &[(&str, &[u8], u32)],
    after: impl FnOnce(&SqliteStore),
) -> (SqliteStore, WakeReservation) {
    let (store, _path, r, p) = fixture::fixture_with(|store| {
        for (session, payload, pid) in events {
            enroll(store, payload, session, *pid);
        }
        after(store);
    });
    qualify(&store, &r, &p);
    (store, r)
}

#[test]
fn queue_targets_the_root_with_a_fork_session_start_side() {
    let (store, r) = seat_with(
        &[(SIDE_ONE, payload!("session-start-fork.json"), ROOT_PID)],
        |_| {},
    );
    assert_eq!(bits(&store, SIDE_ONE), (true, true));
    let (root, sides) = selected(&select(&store, &r));
    assert_eq!(root, SESSION_ROOT);
    assert_eq!(sides, [SIDE_ONE]);
}

#[test]
fn queue_targets_the_root_with_a_side_first_seen_by_user_prompt_submit_or_pre_tool_use() {
    for (label, first) in [
        (
            "user-prompt-submit",
            payload!("user-prompt-submit-side.json"),
        ),
        ("pre-tool-use", payload!("pre-tool-use-side.json")),
    ] {
        let (store, r) = seat_with(&[(SIDE_ONE, first, ROOT_PID)], |_| {});
        assert_eq!(bits(&store, SIDE_ONE), (false, true), "{label}");
        let (root, sides) = selected(&select(&store, &r));
        assert_eq!(
            (root.as_str(), sides),
            (SESSION_ROOT, vec![SIDE_ONE.to_owned()]),
            "{label}"
        );
    }
}

#[test]
fn a_later_non_null_event_of_the_side_keeps_the_mark_and_the_root_targeted() {
    let (store, r) = seat_with(
        &[
            (SIDE_ONE, payload!("pre-tool-use-side.json"), ROOT_PID),
            (
                SIDE_ONE,
                payload!("pre-tool-use-side-nonnull.json"),
                ROOT_PID,
            ),
        ],
        |_| {},
    );
    assert_eq!(bits(&store, SIDE_ONE), (false, true));
    assert_eq!(selected(&select(&store, &r)).0, SESSION_ROOT);
}

#[test]
fn queue_targets_the_root_with_two_side_bindings() {
    let (store, r) = seat_with(
        &[
            (SIDE_ONE, payload!("user-prompt-submit-side.json"), ROOT_PID),
            (SIDE_TWO, payload!("session-start-fork.json"), ROOT_PID),
        ],
        |_| {},
    );
    let (root, mut sides) = selected(&select(&store, &r));
    sides.sort();
    assert_eq!(root, SESSION_ROOT);
    assert_eq!(sides, [SIDE_ONE, SIDE_TWO]);
}

#[test]
fn lone_root_selects_with_no_side_sessions() {
    let (store, r) = seat_with(&[], |_| {});
    let (root, sides) = selected(&select(&store, &r));
    assert_eq!(root, SESSION_ROOT);
    assert!(sides.is_empty());
}

#[test]
fn a_side_with_no_hook_event_yet_is_invisible_and_the_root_is_targeted() {
    // The accepted limit (TRUST-POLICY): only the root has a binding, so it is the root.
    let (store, r) = seat_with(&[], |_| {});
    let open: i64 = store
        .context()
        .open_writer()
        .unwrap()
        .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(open, 1);
    assert_eq!(selected(&select(&store, &r)).0, SESSION_ROOT);
}

/// Records a qualified native turn (Busy then Idle) of `session`'s own binding, as [`qualify`] does
/// for the root: the report names the session, its execution and its process.
fn qualify_session(
    store: &SqliteStore,
    r: &WakeReservation,
    p: &WakeIntentPreparation,
    session: &str,
    pid: u32,
) {
    let execution: String = store
        .context()
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT execution_id FROM occupant_bindings WHERE native_session=?1",
            [session],
            |r| r.get(0),
        )
        .unwrap();
    for edge in [
        crate::harness::adapter::TurnEdge::Busy,
        crate::harness::adapter::TurnEdge::Idle,
    ] {
        let mut report = fixture::native_report(store, r, p, "qualified-session", edge);
        report.claim.native_session = crate::protocol::ids::NativeSessionId::new(session);
        report.claim.execution = crate::protocol::ids::ExecutionId::new(&execution);
        report.claim.liveness =
            Some(crate::protocol::authority::LivenessFingerprint { pid, start: 1 });
        crate::ports::StorePort::record_native_turn(store, &report, &fixture::budget()).unwrap();
    }
}
/// A lone session of the seat, its root gone, with its own native turn qualified: what makes a
/// binding selectable other than its side marks.
fn lone_qualified(first: &[u8]) -> (SqliteStore, WakeReservation) {
    let (store, _path, r, p) = fixture::fixture_with(|store| {
        enroll(store, first, SIDE_ONE, ROOT_PID);
        disconnect_root(store);
    });
    qualify_session(&store, &r, &p, SIDE_ONE, ROOT_PID);
    (store, r)
}

/// The provenance the lone session's binding was enrolled with.
fn provenance(store: &SqliteStore, session: &str) -> String {
    store
        .context()
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT observation_provenance FROM occupant_bindings WHERE native_session=?1",
            [session],
            |r| r.get(0),
        )
        .unwrap()
}

// Kills: the `[only]` arm of `process_root` selecting a lone session that is concurrent or
// auxiliary (`!only.concurrent && !only.auxiliary` dropped). The side's own native turn is
// qualified and its binding is hook-registered, so nothing but that rule keeps it from selecting;
// the control below selects the same shape without the bits. A fork enrolls with the
// `cooperative_top_level` provenance `canonical` demands, which a side first seen by a prompt or
// tool event does not (see the next test).
#[test]
fn a_lone_side_whose_root_is_gone_is_never_a_target() {
    for (label, first, bits_are) in [
        (
            "fork-null-path",
            payload!("session-start-fork.json"),
            (true, true),
        ),
        // A real fork (a path) carries only the concurrent bit and is never a target either.
        (
            "fork-nonnull",
            payload!("session-start-fork-nonnull.json"),
            (true, false),
        ),
    ] {
        let (store, r) = lone_qualified(first);
        assert_eq!(bits(&store, SIDE_ONE), bits_are, "{label}");
        assert_eq!(
            provenance(&store, SIDE_ONE),
            "cooperative_top_level",
            "{label}: selectable but for the root rule"
        );
        native(&select(&store, &r));
    }
}

// A side first seen by a prompt or tool event is enrolled as `cooperative_env_session`, which
// `canonical` refuses on its own: native whatever the root rule says (so this test is not the
// guard's kill, the one above is). It pins that no such side becomes a queue target.
#[test]
fn a_lone_side_first_seen_by_a_prompt_or_tool_event_stays_native() {
    for (label, first) in [
        (
            "user-prompt-submit",
            payload!("user-prompt-submit-side.json"),
        ),
        ("pre-tool-use", payload!("pre-tool-use-side.json")),
    ] {
        let (store, r) = lone_qualified(first);
        assert_eq!(bits(&store, SIDE_ONE), (false, true), "{label}");
        assert_eq!(provenance(&store, SIDE_ONE), "cooperative_env_session");
        native(&select(&store, &r));
    }
}

// Kills: dropping only `!only.auxiliary` from the `[only]` arm: the fixture root itself, marked
// auxiliary by a null-path event, is a lone hook-registered top-level binding with a qualified
// turn and no concurrent bit.
#[test]
fn a_lone_root_binding_marked_auxiliary_is_never_a_target() {
    let (store, r) = seat_with(
        &[(
            SESSION_ROOT,
            payload!("pre-tool-use-root-null.json"),
            ROOT_PID,
        )],
        |_| {},
    );
    assert_eq!(bits(&store, SESSION_ROOT), (false, true));
    native(&select(&store, &r));
}

// The control of the tests above: the same steps with a session that carries neither bit select
// it, so they turn red when the `[only]` arm stops refusing a side.
#[test]
fn a_lone_non_auxiliary_session_whose_root_is_gone_is_selected() {
    let (store, r) = lone_qualified(payload!("session-start-root.json"));
    assert_eq!(bits(&store, SIDE_ONE), (false, false));
    let (root, sides) = selected(&select(&store, &r));
    assert_eq!(root, SIDE_ONE);
    assert!(sides.is_empty());
}

#[test]
fn a_non_side_fork_beside_its_connected_original_stays_native() {
    let (store, r) = seat_with(
        &[(
            SIDE_ONE,
            payload!("session-start-fork-nonnull.json"),
            ROOT_PID,
        )],
        |_| {},
    );
    assert_eq!(bits(&store, SIDE_ONE), (true, false));
    native(&select(&store, &r));
    // Even with a real /side beside both, the fork keeps the process rootless.
    let (store, r) = seat_with(
        &[
            (
                SIDE_ONE,
                payload!("session-start-fork-nonnull.json"),
                ROOT_PID,
            ),
            (SIDE_TWO, payload!("user-prompt-submit-side.json"), ROOT_PID),
        ],
        |_| {},
    );
    native(&select(&store, &r));
}

#[test]
fn a_root_that_emits_a_null_transcript_path_makes_the_process_rootless() {
    // The root's own event reports a null path: it reads auxiliary, so no root is named.
    let (store, r) = seat_with(
        &[
            (SIDE_ONE, payload!("user-prompt-submit-side.json"), ROOT_PID),
            (
                SESSION_ROOT,
                payload!("pre-tool-use-root-null.json"),
                ROOT_PID,
            ),
        ],
        |_| {},
    );
    assert_eq!(bits(&store, SESSION_ROOT), (false, true));
    native(&select(&store, &r));
}

#[test]
fn bindings_across_two_processes_stay_native() {
    let (store, r) = seat_with(
        &[(SIDE_ONE, payload!("pre-tool-use-side.json"), ROOT_PID + 1)],
        |_| {},
    );
    native(&select(&store, &r));
}

#[test]
fn plain_call_bindings_carry_no_evidence_and_stay_native() {
    // A session enrolled by a plain call whose payload carries no evidence (what a hook-less CLI
    // caller sends) reads as a second plain binding, so no root is named: /side routing needs
    // hook-registered Codex.
    let (store, r) = seat_with(
        &[(SIDE_ONE, payload!("pre-tool-use-root.json"), ROOT_PID)],
        |_| {},
    );
    assert_eq!(bits(&store, SIDE_ONE), (false, false));
    native(&select(&store, &r));
}

#[test]
fn a_non_naming_adapter_stays_native_beside_a_second_binding() {
    let (store, _path, r, p) = fixture::fixture();
    qualify(&store, &r, &p);
    {
        let db = store.context().open_writer().unwrap();
        db.execute("UPDATE occupant_bindings SET harness='claude'", [])
            .unwrap();
        db.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id,liveness_pid,liveness_start,liveness_boot,concurrent_session) VALUES ('s',1,1,'pane','host',0,'claude',?1,'execution','cooperative_top_level',0,0,'term-pane','inc','c2',10,1,(SELECT boot FROM node_kernel_boot WHERE id=1),1)",
            [SIDE_ONE],
        )
        .unwrap();
    }
    assert!(select(&store, &r).binding.is_none());
}

#[test]
fn census_above_the_bound_stays_native() {
    let (store, _path, r, p) = fixture::fixture();
    qualify(&store, &r, &p);
    for n in 0..64 {
        add_binding(
            &store,
            &format!("side{n}"),
            &format!("00000000-0000-0000-0000-{:012}", n + 100),
        );
    }
    // 65 connected bindings: over the 64-binding census bound.
    assert!(select(&store, &r).binding.is_none());
}
