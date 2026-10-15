use super::{identity, namespace};
use herdr_threads::{protocol::time::UtcMillis, store::schema};
use rusqlite::Connection;

// Keep this module's identity helpers tied to the same reviewed consumer API.
#[test]
fn registered28_identity_lookup_is_absent_without_a_begin() {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    assert_eq!(
        herdr_threads::store::topology_handoff::current(&db, &namespace(), &identity()).unwrap(),
        None
    );
}

#[test]
fn legacy_key_index_retains_foreign_namespaces_with_bounded_lookup_after_1000_attempts() {
    use herdr_threads::{protocol::handoff::BootstrapAttempt, store::topology_handoff};
    let mut db = super::fixture();
    let id = identity();
    let mut other = id.clone();
    other.payload.handoff.namespace.state_dir = "/other-state".into();
    other.digest = other.semantic_digest().unwrap();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    topology_handoff::begin_pending(&tx, &other.payload.handoff.namespace, &other, UtcMillis(0))
        .unwrap();
    tx.execute(
        "UPDATE bootstrap_attempts SET state='not_submitted' WHERE parent_id=1 AND attempt=1",
        [],
    )
    .unwrap();
    for n in 2..=1001 {
        let attempt = BootstrapAttempt::new(n).unwrap();
        tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key,not_submitted_key) VALUES(1,?1,?2,?3,?4,?5,?6)",rusqlite::params![n,if n==1001{"prepared"}else{"not_submitted"},attempt.operation(&id.compound,"reserve").unwrap().as_str(),attempt.operation(&id.compound,"record").unwrap().as_str(),attempt.operation(&id.compound,"check").unwrap().as_str(),attempt.operation(&id.compound,"not_submitted").unwrap().as_str()]).unwrap();
    }
    tx.execute(
        "UPDATE bootstrap_handoffs SET current_attempt=1001 WHERE id=1",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    let sql = "SELECT state_dir,host_endpoint FROM bootstrap_child_keys INDEXED BY bootstrap_keys_legacy_lookup WHERE instance_id='i' AND actor_scope='seat:s' AND operation_key='invite' LIMIT 2";
    let plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|p| p.contains("instance_id=? AND actor_scope=? AND operation_key=?")),
        "{plan:?}"
    );
    let mut statement = db.prepare(sql).unwrap();
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "unscoped lookup must retain both namespaces for conservative refusal, not impose uniqueness or choose authority"
    );
    assert!(rows.contains(&("/state".into(), "/host.sock".into())));
    assert!(rows.contains(&("/other-state".into(), "/host.sock".into())));
    assert!(
        statement.get_status(rusqlite::StatementStatus::VmStep) < 100,
        "lookup must not scan all historical attempt keys"
    );
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id)
            .unwrap()
            .unwrap()
            .attempt
            .get(),
        1001
    );
}
