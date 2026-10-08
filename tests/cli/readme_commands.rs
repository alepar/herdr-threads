//! Consume the runnable README examples through the production actor/parser.
use herdr_threads::cli::{
    actor_route::InvocationActor,
    commands::{CliAction, MutationSpec, parse_argv},
};

/// Catches person-authored quickstart mutations being routed as Agent.
#[test]
fn readme_person_quickstart_commands_route_as_human() {
    let readme =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
            .unwrap();
    let quickstart = readme
        .split("## Try it yourself")
        .nth(1)
        .unwrap()
        .split("## A role that survives its session")
        .next()
        .unwrap();
    for block in quickstart.split("```sh\n").skip(1) {
        let script = block.split("```").next().unwrap().replace("\\\n", " ");
        for line in script
            .lines()
            .filter(|line| line.starts_with("herdr-threads "))
        {
            let argv = shlex::split(line).expect("documented shell argv");
            let parsed = parse_argv(argv.iter())
                .unwrap_or_else(|error| panic!("documented argv {argv:?}: {error:?}"));
            if matches!(
                parsed.action,
                CliAction::MeInit { .. }
                    | CliAction::Handoff(_)
                    | CliAction::Mutation(MutationSpec::Send { .. })
            ) {
                assert_eq!(
                    parsed.actor,
                    InvocationActor::Human,
                    "person argv must retain Human attribution: {argv:?}"
                );
            }
        }
    }
}
