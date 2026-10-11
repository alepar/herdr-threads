#[test]
fn released_v39_invitation_epoch_alter_records_nested_work_and_preserves_state()
-> rusqlite::Result<()> {
    use rusqlite::{params, types::Value};

    fn snapshot(db: &Connection, sql: &str) -> rusqlite::Result<Vec<Vec<Value>>> {
        let mut statement = db.prepare(sql)?;
        let width = statement.column_count();
        let mapped = statement.query_map([], |row| (0..width).map(|i| row.get(i)).collect())?;
        mapped.collect()
    }
    // Exact released migrations in order. Empty-schema SQL setup does not
    // substitute for the production migration runner or public invite path.
    let released = [
        include_str!("../../migrations/0001_initial.sql"),
        include_str!("../../migrations/0002_service_substrate.sql"),
        include_str!("../../migrations/0003_invitation_cancellations.sql"),
        include_str!("../../migrations/0004_voluntary_membership.sql"),
        include_str!("../../migrations/0005_service_notifications.sql"),
        include_str!("../../migrations/0006_retirement_health.sql"),
        include_str!("../../migrations/0007_attention_digest.sql"),
        include_str!("../../migrations/0008_digest_pending_paths.sql"),
        include_str!("../../migrations/0009_human_occupant.sql"),
        include_str!("../../migrations/0010_b5_trust_guards.sql"),
        include_str!("../../migrations/0011_cooperative_only.sql"),
        include_str!("../../migrations/0012_harness_version_evidence.sql"),
        include_str!("../../migrations/0013_thread_summaries.sql"),
        include_str!("../../migrations/0014_catch_up_release.sql"),
        include_str!("../../migrations/0015_preparation_retention.sql"),
        include_str!("../../migrations/0016_human_receipt_waivers.sql"),
        include_str!("../../migrations/0017_wake_batches.sql"),
        include_str!("../../migrations/0018_warning_conditions.sql"),
        include_str!("../../migrations/0019_thread_names.sql"),
        include_str!("../../migrations/0020_recent_activity.sql"),
        include_str!("../../migrations/0021_invitation_rejections.sql"),
        include_str!("../../migrations/0022_user_message_intent.sql"),
        include_str!("../../migrations/0023_channel_archival.sql"),
        include_str!("../../migrations/0024_harness_contract_diagnostics.sql"),
        include_str!("../../migrations/0025_warning_notice_delivery.sql"),
        include_str!("../../migrations/0026_lazy_message_delivery.sql"),
        include_str!("../../migrations/0027_harness_adapters.sql"),
        include_str!("../../migrations/0028_handoff_topology.sql"),
        include_str!("../../migrations/0029_direct_threads.sql"),
        include_str!("../../migrations/0030_automatic_handoff.sql"),
        include_str!("../../migrations/0031_seat_wake_on.sql"),
        include_str!("../../migrations/0032_invitation_confirmation.sql"),
        include_str!("../../migrations/0033_receipt_send_time.sql"),
        include_str!("../../migrations/0034_handoff_body_bounds.sql"),
        include_str!("../../migrations/0035_receipt_deadline_backfill.sql"),
        include_str!("../../migrations/0036_invitation_lifecycle.sql"),
        include_str!("../../migrations/0037_invitation_warning_origin.sql"),
        include_str!("../../migrations/0038_receipt_warning_origins.sql"),
        include_str!("../../migrations/0039_automatic_handoff_body_bounds.sql"),
    ];
    let epoch_alter = include_str!("../../migrations/0040_invitation_presentation.sql")
        .lines()
        .find(|line| line.starts_with("ALTER TABLE invitations ADD COLUMN command_context_epoch "))
        .expect("exact V40 epoch ALTER");
    for rows in [1_i64, 32, 128] {
        let db = Connection::open_in_memory()?;
        // Some released migrations rebuild tables. Enable FK enforcement
        // after the empty historical schema is assembled, before any data.
        db.execute_batch("PRAGMA foreign_keys=OFF;")?;
        for migration in released {
            db.execute_batch(migration)?;
        }
        db.execute_batch("PRAGMA user_version=39; PRAGMA foreign_keys=ON;")?;
        let foreign_keys: i64 = db.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        assert_eq!(foreign_keys, 1);
        db.execute_batch(
            "INSERT INTO host_instances(id,created_at,decision_seq) VALUES('instance',0,1);
             INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('origin','instance','resolved','native',1,0);
             INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('thread','instance','retained topic','retained goal',0,0);
             INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,joined_at) VALUES('thread','origin',1,'joined','joined',0);
             INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('thread','origin',1,1);",
        )?;
        for n in 0..rows {
            let seat = format!("recipient-{n:04}");
            let invitation = format!("invitation-{n:04}");
            db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES(?1,'instance','resolved','native',1,0)", [&seat])?;
            db.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES('thread',?1,1,'invited','invited')", [&seat])?;
            db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES(?1,'thread',?2,1,'pending',0,?3,1000,1000,'require_ack','origin')", params![invitation,seat,n+2])?;
        }
        db.execute(
            "UPDATE host_instances SET decision_seq=?1 WHERE id='instance'",
            [rows + 1],
        )?;
        assert!(snapshot(&db, "PRAGMA foreign_key_check")?.is_empty());
        let names = {
            let mut statement = db.prepare("PRAGMA table_info(invitations)")?;
            let mapped = statement.query_map([], |row| row.get::<_, String>(1))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?.join(",")
        };
        let old_query = format!("SELECT {names} FROM invitations ORDER BY ordinal");
        let invitations_before = snapshot(&db, &old_query)?;
        assert_eq!(invitations_before.len(), rows as usize);
        let digest_query = "SELECT * FROM digest_pending_invitations ORDER BY ordinal";
        let digest_before = snapshot(&db, digest_query)?;
        assert_eq!(digest_before.len(), rows as usize);
        let catalog_query = "SELECT type,name,tbl_name,rootpage,sql FROM sqlite_schema WHERE name<>'invitations' ORDER BY type,name";
        let catalog_before = snapshot(&db, catalog_query)?;
        let fk_query = "PRAGMA foreign_key_list(invitations)";
        let fk_before = snapshot(&db, fk_query)?;
        let rootpage: i64 = db.query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name='invitations'",
            [],
            |row| row.get(0),
        )?;
        let version: String = db.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
        let (outer, progress) = presentation_sqlite_measured(&db, epoch_alter)?;
        assert!(
            outer > 0 && progress > 0,
            "both linked counters must be active"
        );
        // Record actual nested work; neither constant nor linear complexity
        // is assumed from the absence of a CHECK on the new column.
        eprintln!(
            "sqlite_version={version} released39_invitations={rows} exact40_epoch_alter_outer_vm={outer} including_nested_progress={progress}"
        );
        assert_eq!(snapshot(&db, &old_query)?, invitations_before);
        assert_eq!(snapshot(&db, digest_query)?, digest_before);
        assert_eq!(snapshot(&db, catalog_query)?, catalog_before);
        assert_eq!(snapshot(&db, fk_query)?, fk_before);
        assert!(snapshot(&db, "PRAGMA foreign_key_check")?.is_empty());
        let after_rootpage: i64 = db.query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name='invitations'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(after_rootpage, rootpage);
        let defaults: i64 = db.query_row(
            "SELECT COUNT(*) FROM invitations WHERE command_context_epoch=0",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(defaults, rows);
        let new_column: (String, i64, String) = db.query_row("SELECT type,\"notnull\",dflt_value FROM pragma_table_info('invitations') WHERE name='command_context_epoch'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        assert_eq!(new_column, ("INTEGER".into(), 1, "0".into()));
        let user_version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        assert_eq!(
            user_version, 39,
            "this probe applies one ALTER, not full migration40"
        );
    }
    Ok(())
}
