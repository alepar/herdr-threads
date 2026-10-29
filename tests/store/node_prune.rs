//! `store::node_prune`: the per-node keep reasons, the island rule, the
//! opaque-entry guard, and the views that leave forgotten nodes out (N16).
use std::collections::BTreeSet;

use herdr_threads::{
    node::NodeUuid,
    store::{
        node_prune::{self, KeepReason, PruneScope},
        sync_status,
    },
};
use rusqlite::{Connection, params};

const NOW: i64 = 100 * 24 * 3_600_000;
const OLD: i64 = NOW - 30 * 24 * 3_600_000;
const WEEK: i64 = 7 * 24 * 3_600_000;

fn db() -> Connection {
    super::channel_archival::fixture()
}

fn node(n: u128) -> NodeUuid {
    NodeUuid::from_u128(n)
}

fn local(db: &Connection) -> NodeUuid {
    db.query_row("SELECT id FROM local_node", [], |r| r.get::<_, String>(0))
        .unwrap()
        .parse()
        .unwrap()
}

fn dir(db: &Connection, uuid: NodeUuid, tag: &str, host: &str) {
    db.execute(
        "INSERT INTO node_directory(node_uuid,tag,tag_k,host_label,first_seen_at) VALUES (?1,?2,0,?3,?4)",
        params![uuid.to_string(), tag, host, OLD],
    )
    .unwrap();
}

/// A log of `uuid` holding one applied entry per `(creates, needs)`.
fn log(db: &Connection, uuid: NodeUuid, entries: &[(Option<&str>, &[&str])]) -> i64 {
    let incarnation: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(incarnation),0)+1 FROM sync_logs",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter, \
         head_hash,applied_through,first_seen_at) VALUES (?1,?2,?1=(SELECT id FROM local_node),zeroblob(32),zeroblob(32),?3,zeroblob(32),?3,?4)",
        params![uuid.to_string(), incarnation, entries.len() as i64, OLD],
    )
    .unwrap();
    let log_ix = db.last_insert_rowid();
    for (i, (creates, needs)) in entries.iter().enumerate() {
        db.execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,signature,raw,hlc,kind,kind_version, \
             deps,creates,needs,payload,ingested_at,apply_state,applied_seq) \
             VALUES (?1,?2,zeroblob(32),zeroblob(32),zeroblob(64),zeroblob(68),1,'test.kind',1, \
             x'',?3,?4,x'',?5,'applied',(SELECT COALESCE(MAX(applied_seq),0)+1 FROM journal))",
            params![log_ix, i as i64 + 1, creates, needs.join("\n"), OLD],
        )
        .unwrap();
    }
    log_ix
}

/// One more entry of `log_ix`, of an unknown signed layout (no projection).
fn opaque(db: &Connection, log_ix: i64, counter: i64) {
    db.execute(
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,signature,raw,needs,ingested_at,apply_state) \
         VALUES (?1,?2,zeroblob(32),zeroblob(32),zeroblob(64),zeroblob(68),NULL,?3,'parked_layout')",
        params![log_ix, counter, OLD],
    )
    .unwrap();
}

fn scope(live: &[NodeUuid]) -> PruneScope {
    PruneScope {
        now_ms: NOW,
        host: "box".into(),
        older_than_ms: WEEK,
        live: live.iter().copied().collect(),
    }
}

fn verdicts(db: &Connection, scope: &PruneScope) -> Vec<(String, Option<KeepReason>)> {
    node_prune::evaluate(db, scope)
        .unwrap()
        .into_iter()
        .map(|c| (c.tag, c.keep))
        .collect()
}

// Kills: each per-node keep reason dropped, an island pruned around a kept
// member, a node off the host or unknown to the directory not blocking its
// island, and an off-host node listed or pruned.
#[test]
fn every_keep_reason_and_the_island_rule() {
    let db = db();
    let me = local(&db);
    log(&db, me, &[(Some("sLOC"), &[])]);
    let nodes: Vec<(u128, &str, &str)> = vec![
        (1, "AAA", "box"), // a lone dead island
        (2, "BBB", "box"), // B and C, a dead island of two
        (3, "CCC", "box"),
        (4, "DDD", "box"),   // references this node
        (5, "EEE", "box"),   // recent contact
        (6, "FFF", "box"),   // an enabled dial target learned it
        (7, "GGG", "box"),   // open clone evidence
        (8, "HHH", "box"),   // a peer-requested launch
        (9, "III", "other"), // off the host, J references it
        (10, "JJJ", "box"),
        (12, "LLL", "box"), // references a log owner the directory lacks
        (13, "MMM", "box"), // linked now
    ];
    for (n, tag, host) in &nodes {
        dir(&db, node(*n), tag, host);
    }
    log(&db, node(1), &[(Some("sA"), &["sA"])]);
    log(&db, node(2), &[(Some("sB"), &["sC"])]);
    log(&db, node(3), &[(Some("sC"), &[])]);
    log(&db, node(4), &[(None, &["sLOC"])]);
    log(&db, node(9), &[(Some("sI"), &[])]);
    log(&db, node(10), &[(None, &["sI"])]);
    log(&db, node(11), &[(Some("sK"), &[])]); // no directory row
    log(&db, node(12), &[(None, &["sK"])]);
    db.execute(
        "INSERT INTO sync_node_contact(node_uuid,last_contact_at,via,updated_at) VALUES (?1,?2,'relayed',?2)",
        params![node(5).to_string(), NOW - 1_000],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sync_dial_targets(ssh_dest,source,enabled,learned_node_uuid,created_at,updated_at) \
         VALUES ('f@h','explicit',1,?1,1,1)",
        params![node(6).to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sync_clone_evidence(subject,kind,detail,detected_at,last_seen_at) \
         VALUES (?1,'handshake','x',1,1)",
        params![node(7).to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO peer_requests(id,node_uuid,kind,plan,state,created_at,updated_at) \
         VALUES ('00000000-0000-0000-0000-000000000001',?1,'pane','{}','failed',1,1)",
        params![node(8).to_string()],
    )
    .unwrap();

    let got = verdicts(&db, &scope(&[node(13)]));
    let want: Vec<(String, Option<KeepReason>)> = vec![
        ("AAA".into(), None),
        ("BBB".into(), None),
        ("CCC".into(), None),
        ("DDD".into(), Some(KeepReason::ReferencedBy(me))),
        ("EEE".into(), Some(KeepReason::Recent)),
        ("FFF".into(), Some(KeepReason::DialTarget)),
        ("GGG".into(), Some(KeepReason::ForkEvidence)),
        ("HHH".into(), Some(KeepReason::PeerRequest)),
        ("JJJ".into(), Some(KeepReason::ReferencedBy(node(9)))),
        ("LLL".into(), Some(KeepReason::ReferencedBy(node(11)))),
        ("MMM".into(), Some(KeepReason::Linked)),
    ];
    assert_eq!(got, want);

    // `apply` writes exactly the prunable ones, and a second pass finds
    // nothing new to prune.
    let tx = db.unchecked_transaction().unwrap();
    node_prune::apply(&tx, &scope(&[node(13)]), "operator:local-user:1").unwrap();
    tx.commit().unwrap();
    let forgotten: BTreeSet<NodeUuid> = node_prune::forgotten(&db).unwrap();
    assert_eq!(forgotten, [node(1), node(2), node(3)].into_iter().collect());
    let again = verdicts(&db, &scope(&[node(13)]));
    assert!(again.iter().all(|(_, keep)| keep.is_some()), "{again:?}");
}

// Kills: an opaque log kept only through the island rule not blocking the
// prune, and a forgotten opaque log blocking every later prune for good.
#[test]
fn opaque_entries_block_unless_their_log_is_pruned_or_forgotten() {
    let db = db();
    let me = local(&db);
    log(&db, me, &[(Some("sLOC"), &[])]);
    dir(&db, node(1), "OPQ", "box");
    dir(&db, node(2), "NNN", "box");
    let opaque_log = log(&db, node(1), &[(Some("sO"), &[])]);
    opaque(&db, opaque_log, 2);
    log(&db, node(2), &[(Some("sN"), &[])]);

    // The opaque log is itself pruned: nothing blocks.
    assert_eq!(
        verdicts(&db, &scope(&[])),
        vec![("NNN".into(), None), ("OPQ".into(), None)]
    );

    // Kept through its island (it references this node): it blocks the rest.
    log(&db, node(1), &[(None, &["sLOC"])]);
    assert_eq!(
        verdicts(&db, &scope(&[])),
        vec![
            ("NNN".into(), Some(KeepReason::OpaqueEntries)),
            ("OPQ".into(), Some(KeepReason::ReferencedBy(me))),
        ]
    );

    // Forgotten earlier: it no longer blocks.
    db.execute(
        "INSERT INTO sync_forgotten_nodes(node_uuid,tag,host_label,forgotten_at,provenance) \
         VALUES (?1,'OPQ','box',1,'operator:local-user:1')",
        params![node(1).to_string()],
    )
    .unwrap();
    assert_eq!(verdicts(&db, &scope(&[])), vec![("NNN".into(), None)]);
}

// Kills: a forgotten node's logs, parked or failed entries or directory row
// still shown by `sync status`, `sync unapplied`, `peer list` or `@MACHINE`.
#[test]
fn views_leave_forgotten_nodes_out() {
    let db = db();
    dir(&db, node(1), "GON", "box");
    dir(&db, node(2), "KEP", "box");
    for n in [1, 2] {
        let log_ix = log(&db, node(n), &[(Some(&format!("s{n}")), &[])]);
        db.execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,signature,raw,hlc,kind,kind_version, \
             deps,creates,needs,payload,ingested_at,apply_state,apply_error) \
             VALUES (?1,2,zeroblob(32),zeroblob(32),zeroblob(64),zeroblob(68),1,'test.kind',1, \
             x'',NULL,'',x'',1,'failed','bad')",
            params![log_ix],
        )
        .unwrap();
    }
    let gone = node(1).to_string();
    db.execute(
        "INSERT INTO sync_forgotten_nodes(node_uuid,tag,host_label,forgotten_at,provenance) \
         VALUES (?1,'GON','box',1,'operator:local-user:1')",
        params![gone],
    )
    .unwrap();

    let (logs, omitted) = sync_status::per_log(&db, None, 64).unwrap();
    assert!(logs.iter().all(|(key, _)| key.node != node(1)), "{logs:?}");
    assert!(logs.iter().any(|(key, _)| key.node == node(2)));
    assert_eq!(omitted, 0, "forgotten logs are not counted as omitted");
    let page = sync_status::unapplied_page(&db, None, 32).unwrap();
    assert_eq!(page.failed, 1, "only the kept node's failed entry");
    assert!(
        page.rows.iter().all(|row| !row.log.starts_with("GON/")),
        "{:?}",
        page.rows
    );
    let nodes = sync_status::nodes(&db, NOW as u64, &[]).unwrap();
    assert_eq!(
        nodes.iter().map(|n| n.node.as_str()).collect::<Vec<_>>(),
        vec!["KEP"]
    );
    let machines = sync_status::machine_rows(&db).unwrap();
    assert!(machines.iter().all(|row| row.uuid != gone), "{machines:?}");
    assert!(machines.iter().any(|row| row.tag == "KEP"));
    assert_eq!(sync_status::forgotten_count(&db).unwrap(), 1);
}
