use super::*;
#[test]
fn overdue_warning_names_degraded_queue_without_changing_native_result() {
    use crate::ports::TimeBasis;
    use crate::protocol::{
        authority::ObligationRef,
        ids::{MessageId, SeatId},
        time::UtcMillis,
    };
    for (degraded, logical, expected) in [
        (1100, "active", "queue_unreceived"),
        (900, "active", "submitted"),
        (1100, "fulfilled", "submitted"),
    ] {
        let mut db = fixture();
        db.execute_batch("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m','t',1,'ordinary','body',1000,10); UPDATE threads SET next_sequence=2 WHERE id='t'; INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('34750f98bd59fcfc946da45aaabe933b','p','m','t',10,1000,1,0,1,0); INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('s',11,1200,1,'verified'); UPDATE local_node SET decision_seq=11 WHERE id='34750f98bd59fcfc946da45aaabe933b'; INSERT INTO wake_work(seat_id,reason_bits,completed_at_utc,last_outcome) VALUES ('s',1,1200,'submitted');").unwrap();
        // Projection-only fixture: the claim tests separately prove when the
        // daemon may set this durable degradation marker.
        db.execute("INSERT INTO wake_intents(instance_id,attempt,adapter,replay_scope,origin_seat,origin_host,origin_host_boot,origin_host_epoch,origin_target,origin_target_generation,target_json,descriptor,correlation,phase,revision,owner_boot,arm_nonce,created_at,updated_at,cleanup_quarantined,diagnostic,armed_at_utc,native_degraded_at_utc,logical_status) VALUES ('34750f98bd59fcfc946da45aaabe933b','queued','codex','session:old','s',1,'host',0,'pane',1,x'7b7d',x'00','correlation','accepted',1,'boot','nonce',0,1100,0,'queue_unreceived',-30000,?1,?2)",rusqlite::params![degraded,logical]).unwrap();
        let tx = db.transaction().unwrap();
        let warning = schema::record_overdue_if_pending(
            &tx,
            &ObligationRef::Receipt {
                message: MessageId::new("m"),
                seat: SeatId::new("s"),
            },
            &TimeBasis::Decision,
            UtcMillis(1500),
        )
        .unwrap();
        tx.commit().unwrap();
        let payload: String = db
            .query_row(
                "SELECT event_json FROM messages WHERE id=?1",
                [warning.warning.unwrap().as_str()],
                |r| r.get(0),
            )
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["wake"], expected);
        assert_eq!(
            db.query_row(
                "SELECT last_outcome FROM wake_work WHERE seat_id='s'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "submitted"
        );
    }
}
