//! Last-writer-wins registers for replicated thread metadata.
//!
//! Responsibility: this module owns the `thread_registers` table and the
//! `(HlcStamp, NodeUuid)` tie-break that decides which write to a
//! `(thread, field)` register wins, whatever order writes arrive in. It hides
//! the `ON CONFLICT ... WHERE` form and the `thread_registers_lww` monotone
//! trigger behind [`upsert`] and [`current`]. It never writes `threads`
//! (the `thread.*` handlers of ht-5fr.4 copy winners there), never references
//! `threads` by foreign key, and never reads the local clock. A new LWW field
//! is a new [`RegisterField`] variant plus a CHECK entry; handlers never write
//! the table or compare `(hlc, node)` themselves.
//!
//! Losing is [`RegisterOutcome::Lost`], not an error. A trigger abort means a
//! broken caller and surfaces as the `rusqlite::Error`.

use rusqlite::{Connection, OptionalExtension, params};

use crate::node::NodeUuid;
use crate::protocol::ids::ThreadId;
use crate::sync::hlc::HlcStamp;

/// The closed set of LWW register fields; matches the table's `field` CHECK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterField {
    Topic,
    Name,
    Archived,
}

impl RegisterField {
    pub fn as_str(self) -> &'static str {
        match self {
            RegisterField::Topic => "topic",
            RegisterField::Name => "name",
            RegisterField::Archived => "archived",
        }
    }
}

/// Whether an [`upsert`] changed the register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// The write was stored: a first write or a strictly higher stamp.
    Won,
    /// The register already holds an equal or higher `(hlc, node)`.
    Lost,
}

/// Writes `value` to the register when `(hlc, node)` is strictly above the
/// stored stamp (or the register is empty). `ingest_seq` is the `journal` row
/// that carried the write. Node order is the 32-hex text order, which equals
/// the byte order of [`NodeUuid`].
pub fn upsert(
    db: &Connection,
    thread: &ThreadId,
    field: RegisterField,
    value: Option<&str>,
    hlc: HlcStamp,
    node: NodeUuid,
    ingest_seq: i64,
) -> rusqlite::Result<RegisterOutcome> {
    let changed = db.execute(
        "INSERT INTO thread_registers(thread_id, field, value, hlc, node_uuid, src_ingest_seq) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(thread_id, field) DO UPDATE SET \
           value = excluded.value, hlc = excluded.hlc, \
           node_uuid = excluded.node_uuid, src_ingest_seq = excluded.src_ingest_seq \
         WHERE (excluded.hlc, excluded.node_uuid) > (thread_registers.hlc, thread_registers.node_uuid)",
        params![
            thread.as_str(),
            field.as_str(),
            value,
            hlc.as_u64() as i64,
            node.to_string(),
            ingest_seq
        ],
    )?;
    Ok(if changed == 1 {
        RegisterOutcome::Won
    } else {
        RegisterOutcome::Lost
    })
}

/// The stamp of the register's current winner: the floor an author must
/// exceed (`author_above`) for its write to win.
pub fn current(
    db: &Connection,
    thread: &ThreadId,
    field: RegisterField,
) -> rusqlite::Result<Option<(HlcStamp, NodeUuid)>> {
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT hlc, node_uuid FROM thread_registers WHERE thread_id = ?1 AND field = ?2",
            params![thread.as_str(), field.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(hlc, node)| {
        let node = node.parse::<NodeUuid>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok((HlcStamp::from_u64(hlc as u64), node))
    })
    .transpose()
}

#[cfg(test)]
#[path = "../../tests/store/thread_registers.rs"]
mod tests;
