//! Passive body display proofs, separate from ordinary receipt candidates.
use crate::{
    cli::{RunError, journal::Journal, output},
    protocol::{
        authority::CallerClaim, ids::MessageId, output::OutputSpec, results::CommandResult,
    },
};
use std::io::Write;
/// The CLI routing decision, never inferred from a cursor or body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySelection {
    OwnDefaultText,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayCandidates {
    pub claim: CallerClaim,
    pub lazy: Vec<MessageId>,
    pub ordinary: Vec<MessageId>,
}
/// Write and flush one complete selected page before recording any proof.
/// Only the CLI's own default text route opts in; other reads remain passive.
/// Returned lazy IDs carry the exact displayed claim for later frozen intents.
pub fn write_page<W: Write>(
    result: &CommandResult,
    spec: &OutputSpec,
    max_bytes: u32,
    writer: &mut W,
    journal: &Journal,
    claim: &CallerClaim,
    selection: DisplaySelection,
) -> Result<DisplayCandidates, RunError> {
    output::write_selected(result, spec, max_bytes, writer)?;
    let mut candidates = DisplayCandidates {
        claim: claim.clone(),
        lazy: vec![],
        ordinary: vec![],
    };
    if selection != DisplaySelection::OwnDefaultText
        || claim.role != crate::protocol::authority::CallerRole::TopLevel
        || spec.format != crate::protocol::output::OutputFormat::Text
    {
        return Ok(candidates);
    }
    let CommandResult::InboxBatchV2(page) = result else {
        return Ok(candidates);
    };
    for item in &page.items {
        if let crate::protocol::results::InboxBatchV2Item::LazyMessage {
            body,
            body_start,
            body_end,
            body_len,
            ..
        } = item
            && (body_start > body_end
                || body_end > body_len
                || body_end - body_start != body.len() as u64)
        {
            return Err(RunError::Api(
                crate::protocol::results::ApiError::store_corrupt("invalid lazy inbox body bounds"),
            ));
        }
        match item {
            crate::protocol::results::InboxBatchV2Item::LazyMessage {
                message,
                body_start,
                body_end,
                body_len,
                ..
            } => {
                if journal.record_lazy_displayed_chunk(
                    claim,
                    message,
                    *body_start,
                    *body_end,
                    *body_len,
                )? && !candidates.lazy.contains(message)
                {
                    candidates.lazy.push(message.clone());
                }
            }
            crate::protocol::results::InboxBatchV2Item::Message {
                message,
                body_start,
                body_end,
                body_len,
                ack_candidate,
                ..
            } if claim.harness != crate::protocol::authority::Harness::Human => {
                if journal.record_displayed_chunk(
                    claim,
                    message,
                    *body_start,
                    *body_end,
                    *body_len,
                )? && ack_candidate.as_ref() == Some(message)
                    && !candidates.ordinary.contains(message)
                {
                    candidates.ordinary.push(message.clone());
                }
            }
            _ => {}
        }
    }
    Ok(candidates)
}
