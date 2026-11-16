//! Empty retained-ledger fence reads must not pay for binding history.
use super::*;
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
