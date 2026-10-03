use super::*;
use crate::protocol::{ids::ThreadId, pagination::PageRequest};

fn query(full_bodies: bool) -> HistoryQuery {
    HistoryQuery {
        thread: ThreadId::new("t"),
        page: PageRequest {
            cursor: None,
            limit: 19,
            max_bytes: 65_536,
        },
        initial: Some(HistoryRange::Recent { count: 5 }),
        full_bodies,
    }
}

// Golden frozen from the wire before `full_bodies` existed.
// Kills: serializing `"full_bodies":false`, which changes every request byte.
#[test]
fn history_query_without_full_bodies_is_byte_identical() {
    let golden = r#"{"thread":"t","page":{"cursor":null,"limit":19,"max_bytes":65536},"initial":{"kind":"recent","count":5}}"#;
    assert_eq!(serde_json::to_string(&query(false)).unwrap(), golden);
    // An older request (no field) decodes with the field off.
    assert_eq!(
        serde_json::from_str::<HistoryQuery>(golden).unwrap(),
        query(false)
    );
}

// Kills: skipping the field when true, or decoding it as always false.
#[test]
fn full_bodies_true_round_trips() {
    let wire = serde_json::to_string(&query(true)).unwrap();
    assert!(wire.ends_with(r#","full_bodies":true}"#), "{wire}");
    let back: HistoryQuery = serde_json::from_str(&wire).unwrap();
    assert!(back.full_bodies);
    assert_eq!(back, query(true));
}
