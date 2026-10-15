//! The one place the store mints user-facing public IDs.
//!
//! IDs are `<prefix><tag><8 base62 chars>` (`protocol::ids::generate_public_id`),
//! the tag being this node's `local_node.tag`.
//! Every caller runs inside the store's write transaction, so a candidate is
//! checked against every table it (or an ID derived from its suffix) will be
//! written to before use; a taken candidate is regenerated in the same
//! transaction. The tables' UNIQUE constraints remain the final backstop.

use rusqlite::{Connection, params};

use std::str::FromStr;

use crate::node::NodeTag;
use crate::protocol::{
    ids::{generate_public_id, prefix},
    results::{ApiError, ErrorCode},
};

use super::connection::{api_error, store_error};

/// Regeneration attempts before giving up. With 62^8 suffixes, needing even a
/// second attempt is already astronomically unlikely.
const ATTEMPTS: usize = 16;

/// Where a candidate must be absent: `(table, column, prefix)` — the candidate
/// `<tag><suffix>` is checked as `<prefix><suffix>` in `table.column`.
pub(crate) type Slot = (&'static str, &'static str, &'static str);

/// A send preparation `pX` publishes its ordinary message as `mX`.
pub(crate) const SEND_PREPARATION_SLOTS: &[Slot] = &[
    ("send_preparations", "id", prefix::SEND_PREPARATION),
    ("messages", "id", prefix::MESSAGE),
];
/// A notification preparation `npX` publishes `nX`.
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
/// preparation `pX` publishes message `mX`).
///
/// Suffixes are drawn from 62^8 ≈ 2^47.6 values. A retired id is only reused
/// if a fresh draw hits it and nothing references it any more: the probability
/// per draw is about (retired ids) x 2^-47.6, and such a reuse is harmless
/// (TRUST-POLICY Accepted limits, Probabilistic identifiers). Live ids are
/// checked for collisions in every slot.
///
/// Owns: minting, with the node tag. The tag is read from the `local_node` row
/// through `conn`, i.e. inside the caller's transaction, never from
/// `NodeIdentity`'s cache: a retag commits on that row, and the next mint in
/// any transaction must see exactly the committed value. Must not: write, or
/// decide the tag (that is `node::tag_for` at node creation / retag).
pub(crate) fn fresh(conn: &Connection, prefix: &str, slots: &[Slot]) -> Result<String, ApiError> {
    let tag: String = conn
        .query_row("SELECT tag FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    let tag = NodeTag::from_str(&tag).map_err(|error| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("local_node.tag is not a node tag: {error}"),
        )
    })?;
    fresh_with(
        prefix,
        || generate_public_id(prefix, &tag),
        |candidate| {
            let suffix = &candidate[prefix.len()..];
            for (table, column, slot_prefix) in slots {
                let value = format!("{slot_prefix}{suffix}");
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
    use crate::protocol::ids::{is_public_id, public_id_parts};

    #[test]
    fn a_colliding_candidate_is_regenerated() {
        let mut queue = vec!["sQ7aBBBBBBBB", "sQ7aAAAAAAAA"];
        let mut checked = Vec::new();
        let id = fresh_with(
            "s",
            || queue.pop().unwrap().to_owned(),
            |candidate| {
                checked.push(candidate.to_owned());
                Ok(candidate == "sQ7aAAAAAAAA")
            },
        )
        .unwrap();
        assert_eq!(id, "sQ7aBBBBBBBB");
        assert_eq!(checked, ["sQ7aAAAAAAAA", "sQ7aBBBBBBBB"]);
    }

    #[test]
    fn persistent_collisions_fail_closed() {
        let error = fresh_with("s", || "sQ7aAAAAAAAA".into(), |_| Ok(true)).unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreCorrupt);
    }

    fn conn_with_tag(tag: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE a(id TEXT UNIQUE); CREATE TABLE b(id TEXT UNIQUE);
             CREATE TABLE local_node(tag TEXT NOT NULL);",
        )
        .unwrap();
        conn.execute("INSERT INTO local_node(tag) VALUES (?1)", params![tag])
            .unwrap();
        conn
    }

    #[test]
    fn fresh_checks_every_slot_including_derived_ids() {
        let conn = conn_with_tag("Q7a");
        let id = fresh(&conn, "p", &[("a", "id", "p"), ("b", "id", "m")]).unwrap();
        assert!(is_public_id("p", &id), "{id}");
    }

    #[test]
    fn fresh_reads_tag_from_local_node() {
        let conn = conn_with_tag("Q7a");
        for prefix in ["s", "t", "np"] {
            let id = fresh(&conn, prefix, &[]).unwrap();
            let (tag, _) = public_id_parts(prefix, &id).expect(&id);
            assert_eq!(tag.as_str(), "Q7a", "{id}");
        }
    }

    #[test]
    fn fresh_follows_updated_tag() {
        let conn = conn_with_tag("Q7a");
        let before = fresh(&conn, "s", &[]).unwrap();
        conn.execute("UPDATE local_node SET tag='Zz9'", []).unwrap();
        let after = fresh(&conn, "s", &[]).unwrap();
        assert!(before.starts_with("sQ7a"), "{before}");
        assert!(after.starts_with("sZz9"), "{after}");
    }

    #[test]
    fn fresh_rejects_a_malformed_tag_row() {
        let conn = conn_with_tag("a-c");
        let error = fresh(&conn, "s", &[]).unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreCorrupt);
    }
}
