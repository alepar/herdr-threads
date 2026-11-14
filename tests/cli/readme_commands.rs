//! Consume the runnable README examples through the production actor/parser.
use herdr_threads::cli::{
    actor_route::InvocationActor,
    commands::{CliAction, MutationSpec, parse_argv},
};

/// Keeps person actions Human and coordinating-agent handoffs Agent.
#[test]
fn readme_quickstart_commands_preserve_actor_routes() {
    // The walkthrough the README links to; its examples are runnable as written.
    let quickstart = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/try-it.md"),
    )
    .unwrap();
    let mut checked = 0;
    for block in quickstart.split("```sh\n").skip(1) {
        let script = block.split("```").next().unwrap().replace("\\\n", " ");
        for line in script
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("herdr-threads "))
        {
            checked += 1;
            let argv = shlex::split(line).expect("documented shell argv");
            let parsed = parse_argv(argv.iter())
                .unwrap_or_else(|error| panic!("documented argv {argv:?}: {error:?}"));
            if matches!(parsed.action, CliAction::Handoff(_)) {
                assert_eq!(parsed.actor, InvocationActor::Agent);
            } else if matches!(
                parsed.action,
                CliAction::MeInit { .. } | CliAction::Mutation(MutationSpec::Send { .. })
            ) {
                assert_eq!(
                    parsed.actor,
                    InvocationActor::Human,
                    "person argv must retain Human attribution: {argv:?}"
                );
            }
        }
    }
    assert!(checked >= 5, "walkthrough commands not found: {checked}");
}
