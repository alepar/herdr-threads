//! The node directory: one row per node this store has heard of, itself
//! included.
//!
//! Responsibility
//! - Owns: the `node_directory` table and nothing else. It is the only module
//!   that writes the table: the self row at `schema::initialize`
//!   ([`insert_self`]), the peer rows that genesis and retag fact handlers
//!   upsert later ([`upsert`], ht-5fr.3/.4), and the tag lookup that finds
//!   nodes sharing a tag ([`colliding`]).
//! - Hides: the table shape and the `node_directory_tag` index.
//! - Must not: decide a retag. That stays in `node::tag::on_collision`; the
//!   apply handler calls [`colliding`], hands the result to `on_collision` and
//!   acts on its answer. Callers never write the table directly.
//! - Role: storage seam for peer-node identity facts. New directory behaviour
//!   (labels, tag history) goes here, behind these functions.

use crate::node::{NodeTag, NodeUuid};
use crate::store::schema::NodeSeed;
use rusqlite::{Connection, OptionalExtension, params};

/// One `node_directory` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDirectoryRow {
    pub node_uuid: NodeUuid,
    pub tag: NodeTag,
    pub tag_k: u32,
    pub host_label: Option<String>,
    pub first_seen_at: i64,
    /// The HLC of the fact that set the tag; `None` for the self row at init.
    pub tag_hlc: Option<i64>,
}

fn parse_column<T: std::str::FromStr>(text: String, index: usize) -> rusqlite::Result<T> {
    text.parse().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            "malformed node_directory column".into(),
        )
    })
}

/// Inserts the self row of a node store being created, in the creating
/// transaction (`first_seen_at` is the creation time, `tag_hlc` NULL).
pub fn insert_self(conn: &Connection, node: &NodeSeed, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO node_directory(node_uuid,tag,tag_k,first_seen_at) VALUES (?1,?2,?3,?4)",
        params![node.uuid.to_string(), node.tag.to_string(), node.tag_k, now],
    )?;
    Ok(())
}

/// Inserts a peer's row, or updates its tag, `tag_k`, label and `tag_hlc` when
/// the node is already known; `first_seen_at` keeps its first value. A `None`
/// label keeps the stored label.
pub fn upsert(conn: &Connection, row: &NodeDirectoryRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO node_directory(node_uuid,tag,tag_k,host_label,first_seen_at,tag_hlc) \
         VALUES (?1,?2,?3,?4,?5,?6) \
         ON CONFLICT(node_uuid) DO UPDATE SET tag=excluded.tag, tag_k=excluded.tag_k, \
         host_label=COALESCE(excluded.host_label,host_label), tag_hlc=excluded.tag_hlc",
        params![
            row.node_uuid.to_string(),
            row.tag.to_string(),
            row.tag_k,
            row.host_label,
            row.first_seen_at,
            row.tag_hlc
        ],
    )?;
    Ok(())
}

/// The row of `uuid`, if the directory knows the node.
pub fn get(conn: &Connection, uuid: NodeUuid) -> rusqlite::Result<Option<NodeDirectoryRow>> {
    conn.query_row(
        "SELECT node_uuid,tag,tag_k,host_label,first_seen_at,tag_hlc FROM node_directory \
         WHERE node_uuid=?1",
        [uuid.to_string()],
        |r| {
            Ok(NodeDirectoryRow {
                node_uuid: parse_column(r.get(0)?, 0)?,
                tag: parse_column(r.get(1)?, 1)?,
                tag_k: r.get(2)?,
                host_label: r.get(3)?,
                first_seen_at: r.get(4)?,
                tag_hlc: r.get(5)?,
            })
        },
    )
    .optional()
}

/// Every node that currently carries `tag`, ordered by uuid. The caller (a
/// fact handler) passes the result to `on_collision`; this function only
/// reports who shares the tag.
pub fn colliding(conn: &Connection, tag: &NodeTag) -> rusqlite::Result<Vec<NodeUuid>> {
    let mut statement = conn
        .prepare("SELECT node_uuid FROM node_directory INDEXED BY node_directory_tag WHERE tag=?1 ORDER BY node_uuid")?;
    statement
        .query_map([tag.to_string()], |r| parse_column(r.get(0)?, 0))?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::tag_for;
    use crate::protocol::time::UtcMillis;
    use crate::store::schema::initialize;

    fn store() -> (Connection, NodeSeed) {
        let conn = Connection::open_in_memory().unwrap();
        let seed = crate::test_support::test_node_seed();
        initialize(&conn, || UtcMillis(7), &seed).unwrap();
        (conn, seed)
    }

    #[test]
    fn initialize_writes_self_row_through_node_directory() {
        let (conn, seed) = store();
        let row = get(&conn, seed.uuid).unwrap().expect("self row");
        assert_eq!(row.node_uuid, seed.uuid);
        assert_eq!(row.tag, seed.tag);
        assert_eq!(row.tag_k, 0);
        assert_eq!(row.first_seen_at, 7);
        assert_eq!(row.tag_hlc, None);
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM node_directory", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn colliding_returns_other_uuids_with_tag() {
        let (conn, seed) = store();
        let peer = NodeUuid::from_u128(0x1111);
        let other = NodeUuid::from_u128(0x2222);
        let (peer_tag, other_tag) = (seed.tag, tag_for(other, 5));
        assert_ne!(other_tag, seed.tag);
        for (uuid, tag) in [(peer, peer_tag), (other, other_tag)] {
            upsert(
                &conn,
                &NodeDirectoryRow {
                    node_uuid: uuid,
                    tag,
                    tag_k: 0,
                    host_label: None,
                    first_seen_at: 9,
                    tag_hlc: Some(3),
                },
            )
            .unwrap();
        }
        let mut expected = vec![seed.uuid, peer];
        expected.sort_by_key(|u| u.to_string());
        assert_eq!(colliding(&conn, &seed.tag).unwrap(), expected);
        assert_eq!(colliding(&conn, &other_tag).unwrap(), vec![other]);
        let unused: NodeTag = "zzz".parse().unwrap();
        assert_ne!(unused, seed.tag);
        assert!(colliding(&conn, &unused).unwrap().is_empty());
    }

    #[test]
    fn upsert_updates_tag_and_keeps_first_seen_and_label() {
        let (conn, _) = store();
        let peer = NodeUuid::from_u128(0x1111);
        let mut row = NodeDirectoryRow {
            node_uuid: peer,
            tag: "abc".parse().unwrap(),
            tag_k: 0,
            host_label: Some("laptop".into()),
            first_seen_at: 9,
            tag_hlc: Some(3),
        };
        upsert(&conn, &row).unwrap();
        row.tag = "xyz".parse().unwrap();
        row.tag_k = 1;
        row.host_label = None;
        row.first_seen_at = 99;
        row.tag_hlc = Some(4);
        upsert(&conn, &row).unwrap();
        let stored = get(&conn, peer).unwrap().unwrap();
        assert_eq!(stored.tag.as_str(), "xyz");
        assert_eq!(stored.tag_k, 1);
        assert_eq!(stored.host_label.as_deref(), Some("laptop"));
        assert_eq!(stored.first_seen_at, 9);
        assert_eq!(stored.tag_hlc, Some(4));
    }
}
