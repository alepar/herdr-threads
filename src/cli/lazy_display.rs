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

/// Record every independent settlement before the first submission. Recovery
/// errors include all existing exact refs, including a preceding durable intent
/// when writing the second intent fails.
pub fn settle<S>(
    candidates: DisplayCandidates,
    journal: &Journal,
    created_at: i64,
    mut submit: S,
    output: &crate::protocol::output::OutputSpec,
) -> Result<(), RunError>
where
    S: FnMut(
        crate::protocol::commands::Command,
    ) -> Result<CommandResult, crate::protocol::results::ApiError>,
{
    use crate::cli::{
        journal::{IntentScope, SemanticMutation},
        retry,
    };
    let scope = IntentScope::Cooperative {
        instance: candidates.claim.instance.clone(),
        seat: candidates.claim.seat.clone(),
    };
    let human = candidates.claim.harness == crate::protocol::authority::Harness::Human;
    let mut refs = Vec::new();
    let mutations = [
        (!candidates.ordinary.is_empty()).then_some(SemanticMutation::AckDisplayed {
            messages: candidates.ordinary,
        }),
        (!candidates.lazy.is_empty()).then_some(SemanticMutation::CompleteInboxDelivery {
            messages: candidates.lazy,
        }),
    ];
    let frozen_mutations = mutations
        .into_iter()
        .flatten()
        .map(|mutation| SemanticMutation::freeze(mutation, candidates.claim.clone()))
        .collect::<std::io::Result<Vec<_>>>()?;
    for frozen in frozen_mutations {
        match journal.record(scope.clone(), frozen, created_at) {
            Ok(reference) => refs.push(reference),
            Err(error) => {
                return Err(RunError::Io(std::io::Error::new(
                    error.kind(),
                    format!(
                        "inbox page displayed; settlement intent recording failed: {error}{}",
                        recovery_commands(&refs, human, output)
                    ),
                )));
            }
        }
    }
    let mut failures = Vec::new();
    for reference in refs {
        if let Err(error) = retry::run_retry_api_to_writer(
            journal,
            &reference,
            &scope,
            || unreachable!("frozen display claim"),
            &mut submit,
            output,
            &mut std::io::sink(),
        ) {
            failures.push(format!(
                "{error:?}{}",
                recovery_commands(&[reference], human, output)
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(RunError::Io(std::io::Error::other(format!(
            "inbox page displayed; settlement outcomes pending: {}",
            failures.join("; ")
        ))))
    }
}
fn recovery_commands(
    refs: &[crate::cli::journal::IntentRef],
    human: bool,
    output: &OutputSpec,
) -> String {
    refs.iter()
        .map(|r| {
            let mut argv = vec!["herdr-threads".to_owned()];
            if human {
                argv.push("human".into());
            }
            if let Some(state_dir) = &output.context.state_dir {
                argv.extend(["--state-dir".into(), state_dir.clone()]);
            }
            if let Some(host) = &output.context.host {
                argv.extend(["--host-endpoint".into(), host.clone()]);
            }
            argv.extend(["retry".into(), r.recovery_ref()]);
            format!(
                "; retry {}",
                crate::protocol::output::format_command_argv(&argv)
            )
        })
        .collect()
}
