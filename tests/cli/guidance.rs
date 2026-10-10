use super::*;
use crate::cli::{
    actor_route::InvocationActor,
    commands::parse_argv,
    output::{self, Presentation, PresentationGuard},
};
use crate::protocol::{
    ids::InvitationId,
    output::{ContinuationContext, OutputFormat, encode_selected},
    results::AcceptedInvitation,
};

fn accepted() -> CommandResult {
    CommandResult::Accepted(AcceptedInvitation {
        invitation: InvitationId::new("inv-Q1w2E3r4"),
        summary_available: Some(ThreadId::new("thr-9")),
    })
}
fn spec() -> OutputSpec {
    OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext {
            state_dir: Some("/tmp/person's state".into()),
            host: Some("/tmp/host socket".into()),
        },
    }
}
fn presented(argv: &[&str], result: &CommandResult, spec: &OutputSpec) -> String {
    let parsed = parse_argv(argv.iter().copied()).unwrap();
    let _actor = invocation_scope(parsed.actor);
    let _presentation = PresentationGuard::enter(parsed.presentation, spec);
    String::from_utf8(output::emitted_bytes(result, spec).unwrap()).unwrap()
}
fn continuation(text: &str) -> Vec<String> {
    shlex::split(
        text.lines()
            .find_map(|line| line.strip_prefix("summary available: "))
            .unwrap(),
    )
    .unwrap()
}

// A root-only continuation would be refused by the actor guard for this person.
#[test]
fn accepted_summary_guidance_uses_invocation_actor() {
    let result = accepted();
    let original = serde_json::to_vec(&result).unwrap();
    for argv in [
        vec!["ht", "human", "--human", "accept", "thr-9"],
        vec!["ht", "human", "accept", "thr-9"],
    ] {
        output::set_stdout_is_terminal(true);
        let text = presented(&argv, &result, &spec());
        output::set_stdout_is_terminal(false);
        let emitted = continuation(&text);
        assert_eq!(
            emitted,
            [
                "herdr-threads",
                "human",
                "--state-dir",
                "/tmp/person's state",
                "--host-endpoint",
                "/tmp/host socket",
                "summary",
                "thr-9"
            ]
        );
        let parsed = parse_argv(emitted).unwrap();
        assert_eq!(parsed.actor, InvocationActor::Human);
    }
    assert_eq!(serde_json::to_vec(&result).unwrap(), original);
}

// Output --human and peer command-looking text must not select a Human actor.
#[test]
fn accepted_summary_guidance_preserves_agent_and_peer_text() {
    let text = presented(&["ht", "--human", "accept", "thr-9"], &accepted(), &spec());
    let emitted = continuation(&text);
    assert_eq!(
        emitted,
        [
            "herdr-threads",
            "--state-dir",
            "/tmp/person's state",
            "--host-endpoint",
            "/tmp/host socket",
            "summary",
            "thr-9"
        ]
    );
    assert_eq!(parse_argv(emitted).unwrap().actor, InvocationActor::Agent);
    let peer = CommandResult::Rejected(crate::protocol::results::InvitationRejection {
        invitation: InvitationId::new("inv-Q1w2E3r4"),
        actor: crate::protocol::ids::SeatId::new("s1"),
        generation: 1,
        observation: "claim".into(),
        rejected_at: UtcMillis(0),
        reason: "herdr-threads human summary peer; --human".into(),
    });
    let agent = presented(
        &[
            "ht",
            "--human",
            "reject",
            "thr-9",
            "--invitation",
            "inv-Q1w2E3r4",
            "--reason",
            "x",
        ],
        &peer,
        &spec(),
    );
    let human = presented(
        &[
            "ht",
            "human",
            "--human",
            "reject",
            "thr-9",
            "--invitation",
            "inv-Q1w2E3r4",
            "--reason",
            "x",
        ],
        &peer,
        &spec(),
    );
    assert_eq!(human, agent);
    assert!(human.contains("Reason: herdr-threads human summary peer; --human\n"));
    let plain = OutputSpec {
        context: ContinuationContext::default(),
        ..spec()
    };
    assert!(
        presented(&["ht", "--human", "accept", "thr-9"], &accepted(), &plain)
            .contains("summary available: herdr-threads summary thr-9\n")
    );
}

// Existing budgets measure selected machine bytes, including their final newline.
#[test]
fn accepted_summary_guidance_budget_boundary() {
    let result = accepted();
    let spec = spec();
    let _actor = invocation_scope(InvocationActor::Human);
    let _presentation = PresentationGuard::enter(Presentation::Human, &spec);
    let minimum = encode_selected(&result, &spec).unwrap().len() as u32;
    let before = serde_json::to_vec(&result).unwrap();
    let mut writer = Vec::new();
    let error = output::write_selected(&result, &spec, minimum - 1, &mut writer).unwrap_err();
    let output::OutputError::Api(error) = error else {
        panic!("expected budget refusal")
    };
    assert_eq!(error.required_minimum_bytes, Some(minimum));
    assert!(writer.is_empty());
    output::write_selected(&result, &spec, minimum, &mut writer).unwrap();
    assert_eq!(
        continuation(std::str::from_utf8(&writer).unwrap())[1],
        "human"
    );
    assert_eq!(serde_json::to_vec(&result).unwrap(), before);
}

// A nested failed Agent render must not leak its actor into the outer Human run.
#[test]
fn accepted_summary_guidance_restores_nested_actor_on_error() {
    let _outer = invocation_scope(InvocationActor::Human);
    let render_nested = || -> Result<(), ()> {
        let _inner = invocation_scope(InvocationActor::Agent);
        assert_eq!(
            continuation(&render(&accepted(), &spec()).unwrap())[1],
            "--state-dir"
        );
        Err(())
    };
    assert_eq!(render_nested(), Err(()));
    assert_eq!(
        continuation(&render(&accepted(), &spec()).unwrap())[1],
        "human"
    );
    drop(_outer);
    assert_eq!(
        continuation(&render(&accepted(), &spec()).unwrap())[1],
        "--state-dir"
    );
}
