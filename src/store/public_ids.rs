//! The one place the store mints user-facing public IDs.
//!
//! IDs are `<prefix>-<8 base62 chars>` (`protocol::ids::generate_public_id`).
//! Every caller runs inside the store's write transaction, so a candidate is
//! checked against every table it (or an ID derived from its suffix) will be
//! written to before use; a taken candidate is regenerated in the same
//! transaction. The tables' UNIQUE constraints remain the final backstop.

use rusqlite::{Connection, params};

use crate::protocol::{
    ids::{generate_public_id, prefix},
    results::{ApiError, ErrorCode},
};

use super::connection::{api_error, store_error};

/// Regeneration attempts before giving up. With 62^8 suffixes, needing even a
/// second attempt is already astronomically unlikely.
const ATTEMPTS: usize = 16;

/// Where a candidate must be absent: `(table, column, prefix)` — the candidate
/// suffix is checked as `<prefix>-<suffix>` in `table.column`.
pub(crate) type Slot = (&'static str, &'static str, &'static str);

/// A send preparation `prep-X` publishes its ordinary message as `msg-X`.
pub(crate) const SEND_PREPARATION_SLOTS: &[Slot] = &[
    ("send_preparations", "id", prefix::SEND_PREPARATION),
    ("messages", "id", prefix::MESSAGE),
];
/// A notification preparation `notify-prep-X` publishes `notify-X`.
pub(crate) const NOTIFY_PREPARATION_SLOTS: &[Slot] = &[
    (
        "service_notification_preparations",
        "id",
        prefix::NOTIFY_PREPARATION,
    ),
    ("messages", "id", prefix::NOTIFY),
];

/// A fresh public ID with `prefix`, absent from every slot.
///
/// The first slot is normally `(table, "id", prefix)` for the row about to be
/// inserted; extra slots cover IDs derived from the same suffix (a send
/// preparation `prep-X` publishes message `msg-X`).
pub(crate) fn fresh(conn: &Connection, prefix: &str, slots: &[Slot]) -> Result<String, ApiError> {
    fresh_with(
        prefix,
        || generate_public_id(prefix),
        |candidate| {
            let suffix = &candidate[prefix.len() + 1..];
            for (table, column, slot_prefix) in slots {
                let value = format!("{slot_prefix}-{suffix}");
                let taken: bool = conn
                    .query_row(
                        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column}=?1)"),
                        params![value],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                if taken {
                    return Ok(true);
                }
            }
            Ok(false)
        },
    )
}

fn fresh_with(
    prefix: &str,
    mut generate: impl FnMut() -> String,
    mut taken: impl FnMut(&str) -> Result<bool, ApiError>,
) -> Result<String, ApiError> {
    for _ in 0..ATTEMPTS {
        let candidate = generate();
        debug_assert!(candidate.starts_with(prefix));
        if !taken(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(api_error(
        ErrorCode::StoreCorrupt,
        "public ID space exhausted: every generated candidate collided",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ids::is_short_public_id;

    #[test]
    fn a_colliding_candidate_is_regenerated() {
        let mut queue = vec!["seat-BBBBBBBB", "seat-AAAAAAAA"];
        let mut checked = Vec::new();
        let id = fresh_with(
            "seat",
            || queue.pop().unwrap().to_owned(),
            |candidate| {
                checked.push(candidate.to_owned());
                Ok(candidate == "seat-AAAAAAAA")
            },
        )
        .unwrap();
        assert_eq!(id, "seat-BBBBBBBB");
        assert_eq!(checked, ["seat-AAAAAAAA", "seat-BBBBBBBB"]);
    }

    #[test]
    fn persistent_collisions_fail_closed() {
        let error = fresh_with("seat", || "seat-AAAAAAAA".into(), |_| Ok(true)).unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreCorrupt);
    }

    #[test]
    fn fresh_checks_every_slot_including_derived_ids() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE a(id TEXT UNIQUE); CREATE TABLE b(id TEXT UNIQUE);")
            .unwrap();
        let id = fresh(&conn, "prep", &[("a", "id", "prep"), ("b", "id", "msg")]).unwrap();
        assert!(is_short_public_id("prep", &id), "{id}");
    }
}
