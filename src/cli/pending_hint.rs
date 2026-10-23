//! One pending-count line on top of a text command for a caller without turn delivery
//! (ht-piggyback).
//!
//! A binding with no turn hooks and no live mod channel (a plain-CLI Codex binding, say) sees
//! its mail only through its own `inbox` or `accept`. Any other text-mode command of such a
//! top-level agent therefore starts with one line naming the pending count and the `inbox`
//! command that shows it:
//!
//! ```text
//! [herdr-threads] 3 pending: herdr-threads inbox
//! ```
//!
//! The daemon decides "no turn delivery" from the binding's harness and canonical provenance
//! (`HarnessAdapter::binding_turn_delivery`, text only like the wake text) in the same
//! read-only attention digest a hook reads: one round trip, nothing displayed, ACKed or
//! advanced. The count is read before the command runs. Any failure, an older daemon, a
//! subagent, a person, machine output or a zero count prints nothing.
use crate::{
    ports::LocalClient,
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        capabilities::{BINDING_DIGEST, CLI_PENDING_HINT, LAZY_HOOK_DELIVERY},
        commands::{AttentionDigestQuery, Command},
        output::{OutputFormat, OutputSpec, format_command_argv},
        results::CommandResult,
        time::CallBudget,
    },
};
use std::io::Write;

use super::output::Presentation;

/// Writes the line when `claim`'s caller qualifies; never fails the command it precedes.
pub fn write<C: LocalClient + ?Sized, W: Write>(
    output: &OutputSpec,
    presentation: Presentation,
    claim: &CallerClaim,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
    writer: &mut W,
) {
    if let Some(line) = line(output, presentation, claim, client, budget) {
        let _ = writeln!(writer, "{line}");
    }
}

fn line<C: LocalClient + ?Sized>(
    output: &OutputSpec,
    presentation: Presentation,
    claim: &CallerClaim,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
) -> Option<String> {
    if claim.role != CallerRole::TopLevel
        || claim.harness == Harness::Human
        || output.format != OutputFormat::Text
        || presentation == Presentation::Machine
    {
        return None;
    }
    let probe = budget();
    if ![CLI_PENDING_HINT, BINDING_DIGEST]
        .into_iter()
        .all(|name| client.supports_capability(name, &probe))
    {
        return None;
    }
    let result = client
        .call(
            Command::AttentionDigest(AttentionDigestQuery {
                seat: claim.seat.clone(),
                lazy: client.supports_capability(LAZY_HOOK_DELIVERY, &probe),
                claim: Some(claim.clone()),
                cli_hint: true,
            }),
            &budget(),
        )
        .ok()?;
    let CommandResult::AttentionDigest(digest) = result else {
        return None;
    };
    if !digest.no_turn_delivery {
        return None;
    }
    let classes = [&digest.invitations, &digest.receipts, &digest.warnings]
        .into_iter()
        .chain(&digest.lazy)
        .chain(&digest.mail);
    let (count, more) = classes.fold((0u64, false), |(count, more), class| {
        (
            count.saturating_add(class.count),
            more || class.count_has_more,
        )
    });
    if count == 0 {
        return None;
    }
    let mut argv = vec!["herdr-threads".to_owned()];
    if let Some(state_dir) = &output.context.state_dir {
        argv.extend(["--state-dir".into(), state_dir.clone()]);
    }
    if let Some(host) = &output.context.host {
        argv.extend(["--host-endpoint".into(), host.clone()]);
    }
    if presentation == Presentation::Human {
        argv.push("--human".into());
    }
    argv.push("inbox".into());
    Some(format!(
        "[herdr-threads] {count}{} pending: {}",
        if more { "+" } else { "" },
        format_command_argv(&argv)
    ))
}
