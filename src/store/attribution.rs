//! Local attribution of replicated rows.
//!
//! Responsibility
//! - Owns: `fact_attribution_staging` (the only reader and writer), the
//!   staging-key builders, and the fallback observation a row gets when no
//!   attribution was staged (`replicated_from:<uuid>` for another node's
//!   fact, `local_journal` for this node's own earlier fact).
//! - Hides: that attribution never crosses the wire, and the provenance
//!   strings TRUST-POLICY A3 defines.
//! - Must not: be keyed by an `EntityKey`, let a staged row outlive its
//!   transaction (a command stages and the materializer takes within one
//!   writer transaction; a leftover is a bug the WriteGuard suites catch),
//!   read bindings to attribute (materializers call [`take`] instead), or
//!   appear in a payload.
//! - Role: a command that authors an attributed fact calls [`stage`] before
//!   authoring; the kind's materializer calls [`take`] once and writes the
//!   returned binding columns and observation. A new attributed kind adds a
//!   staging-key builder here.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::protocol::results::ApiError;
use crate::store::connection::store_error;
use crate::store::control::AccountableActor;
use crate::store::materialize::MaterializeInput;

/// `send:<message>`: the sender's attribution of a `message.sent`.
pub(crate) fn send_key(message: &str) -> String {
    format!("send:{message}")
}

/// `ack:<message>/<seat>`: the acknowledging binding of a `receipt.settled`.
pub(crate) fn ack_key(message: &str, seat: &str) -> String {
    format!("ack:{message}/{seat}")
}

/// `answer:<invitation>`: the answering binding of an `invitation.answered`.
pub(crate) fn answer_key(invitation: &str) -> String {
    format!("answer:{invitation}")
}

/// `join:<thread>/<seat>/<n>`: the joining binding of a `membership.joined`
/// transition `n`.
pub(crate) fn join_key(thread: &str, seat: &str, n: u64) -> String {
    format!("join:{thread}/{seat}/{n}")
}

/// What a materializer writes into the local binding columns of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attribution {
    /// The local binding that acted; `None` for a row no local binding made.
    pub binding_id: Option<String>,
    /// The binding generation; 0 with no binding.
    pub generation: i64,
    /// The observation JSON text (at most 4096 bytes).
    pub observation: String,
}

/// Stages the command's attribution under `key`. `observation` is the JSON
/// text the command would store for a local decision (usually
/// `actor.observation(decided_at)`). Staging a key twice replaces the first.
pub(crate) fn stage(
    tx: &Transaction<'_>,
    key: &str,
    actor: &AccountableActor,
    observation: &str,
) -> Result<(), ApiError> {
    tx.execute(
        "INSERT INTO fact_attribution_staging(staging_key,binding_id,binding_generation,observation) \
         VALUES(?1,?2,?3,?4) \
         ON CONFLICT(staging_key) DO UPDATE SET binding_id=excluded.binding_id, \
         binding_generation=excluded.binding_generation, observation=excluded.observation",
        params![
            key,
            actor.binding_id.as_str(),
            i64::try_from(actor.binding_generation).unwrap_or(i64::MAX),
            observation
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Drops the attribution staged under `key`, if any: a command that staged it
/// and then did not author the fact (a skipped settlement) leaves nothing behind.
pub(crate) fn discard(tx: &Transaction<'_>, key: &str) -> Result<(), ApiError> {
    tx.execute(
        "DELETE FROM fact_attribution_staging WHERE staging_key=?1",
        [key],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Takes the attribution for `key`: the staged row, deleted, or else the
/// fallback with no binding, generation 0 and the observation
/// `{"provenance":"<provenance>","decided_at":<origin_ms>}`.
///
/// The fallback provenance is the input's own (`replicated_from:<uuid>` for
/// another node's fact). An own-history input with nothing staged (an own old
/// incarnation's entry or a re-issue, `ApplyContext::is_own_history`; local
/// authoring always stages first) is recorded as `local_journal` even when
/// the input says `local`, which TRUST-POLICY does not define as a stored
/// value.
pub(crate) fn take(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    key: &str,
) -> Result<Attribution, ApiError> {
    let staged = tx
        .query_row(
            "DELETE FROM fact_attribution_staging WHERE staging_key=?1 \
             RETURNING binding_id,binding_generation,observation",
            [key],
            |r| {
                Ok(Attribution {
                    binding_id: r.get(0)?,
                    generation: r.get(1)?,
                    observation: r.get(2)?,
                })
            },
        )
        .optional()
        .map_err(store_error)?;
    if let Some(staged) = staged {
        return Ok(staged);
    }
    let provenance = if m.own_history {
        "local_journal"
    } else {
        m.provenance
    };
    Ok(Attribution {
        binding_id: None,
        generation: 0,
        observation: serde_json::json!({"provenance": provenance, "decided_at": m.origin_ms})
            .to_string(),
    })
}

#[cfg(test)]
#[path = "../../tests/store/attribution.rs"]
mod tests;
